//! The producer (section 4 of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md): a node with "Serve a chain snapshot" on, that validates,
//! signs, and whose `getchainstates` shows every chainstate validated,
//! exports where its tip is exactly on the grid (`crate::snapshot_serve`),
//! and sends nothing, neither the P2P offer nor anything to easybtx.com,
//! until that block is at least 144 blocks deep on its own active chain. At
//! that depth it checks, again ([`check_before_send`]):
//!
//! * `getchainstates` still shows no unvalidated chainstate;
//! * every held root it knows is refused on this node (`invalidateblock`
//!   through `crate::known_invalid`, as the fork check does every 30
//!   seconds anyway), so the base cannot sit on a held branch;
//! * the base is still the block at that height on its active chain;
//! * the statement, against the node and its own diary entry for that
//!   height, field by field (`crate::statement_check`).
//!
//! The keeper asks the same through [`ProducerChecks`] (its
//! `snapshot_serve::BeforeOffer` hook) before it offers a pair on P2P, and
//! [`send_offered`] asks again right before the offered pair goes to
//! easybtx.com: the website may be minutes or hours later than the offer
//! (a failed upload is retried every [`RESUBMIT_EVERY`]), and a producer
//! never sends a pair that failed any check. The hook also writes the diary
//! right before each export, so the producer never misses its own height.
//!
//! WHO SENDS TO THE WEBSITE. Only a producer whose signature on its own export
//! comes from a key on the operator list (`crate::operators`): the export's
//! manifest carries exactly the node's signature, so the list is read from
//! the very bytes that would be sent, not from a setting. Any other node that
//! validates and signs keeps offering over P2P, as before the design.
//!
//! WHAT IS WRITTEN DOWN. `<datadir>/snapshots/submitted.json` ([`Submissions`]):
//! per pair, what was sent, how often it was tried, and what the website
//! answered (the file's state, the operators it counted), so a restart
//! neither sends a pair twice nor forgets one that failed.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Refusal};
use crate::diary::{self, DiaryEntry};
use crate::known_invalid::{self, Refusal as Refused};
use crate::role::ValidationMode;
use crate::rpc::Rpc;
use crate::snapshot_serve::{
    manifest_file_name, snapshot_file_name, BeforeOffer, NotOffered, OfferRecord,
};
use crate::snapshot_site::{self as site, FileUpload, Site, SiteError, StatementReply};
use crate::statement_check::{self, Mismatch, Next};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Beside the pairs: what went to the website.
pub const SUBMITTED_FILE: &str = "submitted.json";
pub const SUBMITTED_VERSION: u32 = 1;
/// Pairs remembered: a week of grid heights is about 67, but only the
/// offered pair is ever due, so a few days of history is plenty for Copy
/// diagnostics.
pub const SUBMISSIONS_KEEP: usize = 20;
/// How long a failed submission waits before it is tried again.
pub const RESUBMIT_EVERY: Duration = Duration::from_secs(600);

// ── The checks ──────────────────────────────────────────────────────────────

/// Why a pair is not sent. `Display` is one plain line for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotSent {
    /// The manifest does not read, a signature on it does not verify, or the
    /// statement breaks its chain's rules on its own. Asked nothing.
    Unreadable(Refusal),
    /// `getchainstates` shows a chainstate at `"validated": false`, or did
    /// not answer.
    Unvalidated,
    /// A held root this node knows could not be refused (or a branch before
    /// it failed, and the order is the point).
    HeldNotRefused(String),
    /// The base is no longer the block at its height on the active chain
    /// (after the holds were refused).
    BaseLeftChain { height: u64 },
    /// The statement against the node and its diary.
    Check(Mismatch),
}

impl NotSent {
    /// Whether asking again later can change the answer: the node did not
    /// answer, is still checking its history, or is still short of 144.
    /// Everything else is a pair that is never sent.
    pub fn retry(&self) -> bool {
        match self {
            NotSent::Unvalidated | NotSent::HeldNotRefused(_) => true,
            NotSent::Check(m) => m.next() == Next::Wait,
            NotSent::Unreadable(_) | NotSent::BaseLeftChain { .. } => false,
        }
    }
}

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotSent::Unreadable(r) => write!(f, "the export does not check out: {r}"),
            NotSent::Unvalidated => write!(
                f,
                "this node is still checking older history in the background"
            ),
            NotSent::HeldNotRefused(e) => {
                write!(f, "a held block could not be refused on this node: {e}")
            }
            NotSent::BaseLeftChain { height } => {
                write!(f, "block {height} is no longer on this node's chain")
            }
            NotSent::Check(m) => write!(f, "{m}"),
        }
    }
}

impl From<Refusal> for NotSent {
    fn from(r: Refusal) -> Self {
        NotSent::Unreadable(r)
    }
}

/// What a pair that passed looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The diary entry the statement matched.
    pub entry: DiaryEntry,
    /// The listed operators behind the manifest's signatures (its own one,
    /// or none for a key off the list).
    pub operators: Vec<String>,
    /// The statement's hash, display hex: what the website files it under.
    pub statement_hash: String,
}

/// The manifest read, every signature verified, the statement's own rules
/// checked, and the operators behind it. No RPC.
fn read_manifest(
    manifest: &[u8],
    regtest_env: Option<&str>,
) -> Result<(cs::Manifest, ChainRules, Vec<String>), Refusal> {
    let m = cs::parse(manifest)?;
    let rules = ChainRules::for_statement(&m.statement, regtest_env)?;
    let operators = cs::confirming_operators(&m, &rules.operators)?;
    cs::check_shape(&m.statement, &rules)?;
    Ok((m, rules, operators))
}

/// A refusal that leaves nothing open: refused now, or a block the node has
/// never heard of (nothing to follow).
fn refused_ok(r: &Refused) -> bool {
    matches!(r, Refused::Refused | Refused::NotKnownYet)
}

/// Section 4's checks on this node's own export, at 144 deep, in this
/// order: the manifest alone (no RPC); every chainstate validated (before
/// anything changes the node); every held root refused; the base still on
/// the active chain; the statement against the node and its diary in
/// `diary_dir` (the datadir), which includes 144 deep, the chain and replay
/// context, and no refused block below the base. Refusing held roots changes
/// the node, as the fork check does anyway; everything else reads.
pub async fn check_before_send(
    rpc: &dyn Rpc,
    manifest: &[u8],
    diary_dir: &Path,
    holds: &Holds<'_>,
    regtest_env: Option<&str>,
) -> Result<Checked, NotSent> {
    let (m, rules, operators) = read_manifest(manifest, regtest_env)?;
    let st = &m.statement;

    if !crate::node_api::read_chainstates_validated(rpc).await {
        return Err(NotSent::Unvalidated);
    }

    for block in holds.invalid {
        let r = known_invalid::refuse(rpc, block).await;
        if !refused_ok(&r) {
            return Err(NotSent::HeldNotRefused(format!(
                "{} at {}: {r:?}",
                block.hash, block.height
            )));
        }
    }
    for (branch, r) in known_invalid::refuse_held_in_order(rpc, holds.held).await {
        if !refused_ok(&r) {
            return Err(NotSent::HeldNotRefused(format!(
                "{} at {}: {r:?}",
                branch.root, branch.height
            )));
        }
    }

    let height = st.height() as u64;
    let at = diary::block_at(rpc, height)
        .await
        .map_err(|e| NotSent::Check(Mismatch::NodeUnanswered(e)))?;
    if !at.is_some_and(|b| b.eq_ignore_ascii_case(&st.block_hash().display_hex())) {
        return Err(NotSent::BaseLeftChain { height });
    }

    let diary = diary::load(diary_dir, &st.chain_id().display_hex());
    let entry = statement_check::check_against_node(rpc, st, &rules, &diary, holds)
        .await
        .map_err(NotSent::Check)?;
    Ok(Checked {
        entry,
        operators,
        statement_hash: st.hash().display_hex(),
    })
}

/// The keeper's hook (`snapshot_serve::BeforeOffer`): the diary right before
/// each export, [`check_before_send`] before each offer and re-offer.
pub struct ProducerChecks {
    /// The datadir, where the diary lives.
    pub diary_dir: PathBuf,
    /// `Holds::compiled()` in the app.
    pub holds: Holds<'static>,
    /// `operators::regtest_env()` in the app; only a regtest statement reads it.
    pub regtest_env: Option<String>,
}

#[async_trait::async_trait]
impl BeforeOffer for ProducerChecks {
    async fn before_export(&self, rpc: &dyn Rpc) {
        // The keeper exports only on a node its gate found validating.
        match diary::record_at_tip(rpc, &self.diary_dir, ValidationMode::Consensus).await {
            Ok(diary::DiaryOutcome::Recorded(e)) => {
                eprintln!(
                    "[snapshot] diary: wrote {} (block {})",
                    e.height, e.block_hash
                )
            }
            Ok(_) => {}
            Err(e) => eprintln!("[snapshot] diary: {e}"),
        }
    }

    async fn check(
        &self,
        rpc: &dyn Rpc,
        _base_height: u64,
        manifest: &[u8],
    ) -> Result<(), NotOffered> {
        check_before_send(
            rpc,
            manifest,
            &self.diary_dir,
            &self.holds,
            self.regtest_env.as_deref(),
        )
        .await
        .map(|_| ())
        .map_err(|n| NotOffered {
            why: n.to_string(),
            retry: n.retry(),
        })
    }
}

// ── What went to the website ────────────────────────────────────────────────

/// Where one pair's submission stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SubmissionOutcome {
    /// The website took the statement and has the file.
    Sent {
        /// Unix seconds.
        at: u64,
        /// What became of the file: `stored` (uploaded now, or the website
        /// had it), or `dropped` (the website let it go, being below its
        /// newest confirmed one).
        file: String,
        /// The operators the website counted on the statement, by its copy of
        /// the list. Its word, kept for Copy diagnostics, never acted on.
        operators: Vec<String>,
        signers: Vec<String>,
        /// The website called it confirmed when the file went in.
        confirmed: bool,
    },
    /// Not sent this time; tried again after [`RESUBMIT_EVERY`].
    Failed { error: String },
    /// Never sent and never tried again: a check that cannot change, a key
    /// off the list, a height the owner closed.
    GivenUp { reason: String },
}

/// One pair's submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub height: u64,
    /// The pair's base, as the offer record names it.
    pub block_hash: String,
    /// Computed here from the manifest; empty when it did not read.
    pub statement_hash: String,
    pub attempts: u32,
    /// Unix seconds.
    pub first_tried_at: u64,
    pub last_tried_at: u64,
    pub outcome: SubmissionOutcome,
}

/// `<datadir>/snapshots/submitted.json`, ascending by height, one per pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submissions {
    pub version: u32,
    pub entries: Vec<Submission>,
}

impl Default for Submissions {
    fn default() -> Self {
        Self {
            version: SUBMITTED_VERSION,
            entries: Vec::new(),
        }
    }
}

impl Submissions {
    /// The entry for the pair at `height` with base `block_hash`.
    pub fn of(&self, height: u64, block_hash: &str) -> Option<&Submission> {
        self.entries
            .iter()
            .find(|e| e.height == height && e.block_hash.eq_ignore_ascii_case(block_hash))
    }

    /// Keep `s`, replacing what was there for its height, and only the newest
    /// [`SUBMISSIONS_KEEP`] heights.
    pub fn record(&mut self, s: Submission) {
        self.entries.retain(|e| e.height != s.height);
        self.entries.push(s);
        self.entries.sort_by_key(|e| e.height);
        let excess = self.entries.len().saturating_sub(SUBMISSIONS_KEEP);
        self.entries.drain(..excess);
    }
}

/// The record in the pairs' folder. Missing, unreadable or another version
/// reads as empty: at worst the offered pair is sent again, which the
/// website merges.
pub fn load_submissions(dir: &Path) -> Submissions {
    std::fs::read(dir.join(SUBMITTED_FILE))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Submissions>(&raw).ok())
        .filter(|s| s.version == SUBMITTED_VERSION)
        .unwrap_or_default()
}

/// Write the record atomically.
pub fn save_submissions(dir: &Path, s: &Submissions) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&dir.join(SUBMITTED_FILE), &json)
}

/// Whether the offered pair still has to go to the website: it is on the
/// grid (a pair from before the design is not), and it was never tried, or
/// its last try failed at least [`RESUBMIT_EVERY`] ago. A sibling at a
/// height already tried is a new pair.
pub fn submission_due(log: &Submissions, record: &OfferRecord, now: u64) -> bool {
    if record.height == 0
        || !record
            .height
            .is_multiple_of(crate::snapshot_serve::EXPORT_GRID)
    {
        return false;
    }
    match log.of(record.height, &record.block_hash) {
        None => true,
        Some(s) => match s.outcome {
            SubmissionOutcome::Failed { .. } => {
                now.saturating_sub(s.last_tried_at) >= RESUBMIT_EVERY.as_secs()
            }
            SubmissionOutcome::Sent { .. } | SubmissionOutcome::GivenUp { .. } => false,
        },
    }
}

// ── Sending ─────────────────────────────────────────────────────────────────

/// What the website said to one submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub reply: StatementReply,
    /// `stored` or `dropped`, as [`SubmissionOutcome::Sent`] keeps it.
    pub file: String,
    pub confirmed: bool,
}

/// Send one pair: the statement, then the file unless the website has it or
/// let it go. The caller has checked the pair ([`check_before_send`]);
/// [`send_offered`] is the whole step.
pub async fn submit(
    client: &reqwest::Client,
    site: &Site,
    manifest: &[u8],
    file: &Path,
) -> Result<Sent, SiteError> {
    let statement = cs::parse(manifest)
        .map_err(|e| SiteError::NotSent(format!("the manifest: {e}")))?
        .statement;
    let reply = site::post_statement(client, site, manifest).await?;
    let (file, confirmed) = match reply.file.as_str() {
        "stored" | "dropped" => (reply.file.clone(), false),
        _ => match site::upload_file(client, site, &statement, file).await? {
            FileUpload::Stored(s) => ("stored".to_string(), s.confirmed),
            FileUpload::AlreadyStored => ("stored".to_string(), false),
        },
    };
    Ok(Sent {
        reply,
        file,
        confirmed,
    })
}

/// What [`send_offered`] came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Sent already, given up, waiting out [`RESUBMIT_EVERY`], or off the grid.
    NotDue,
    /// This node's key is not on the operator list: nothing goes to the
    /// website. Written down, so it is said once per pair.
    NotListed,
    /// The pair failed a check: nothing was sent. Tried again only when the
    /// check can change ([`NotSent::retry`]).
    NotSent(NotSent),
    Sent(Submission),
    /// The website did not take it; tried again after [`RESUBMIT_EVERY`].
    Failed {
        error: String,
    },
    /// The website will never take it (the owner closed the height), or the
    /// local file cannot be the statement's.
    GivenUp {
        reason: String,
    },
}

/// Section 4's last step for the offered pair `record`, whose files are in
/// `dir` (`<datadir>/snapshots`): when due, and when its key is on the list,
/// check it again and send it, and write down what happened. `now` is unix
/// seconds. The keeper calls it after every round; it does nothing most of
/// the time.
pub async fn send_offered(
    rpc: &dyn Rpc,
    checks: &ProducerChecks,
    client: &reqwest::Client,
    site: &Site,
    dir: &Path,
    record: &OfferRecord,
    now: u64,
) -> SendOutcome {
    let mut log = load_submissions(dir);
    if !submission_due(&log, record, now) {
        return SendOutcome::NotDue;
    }
    let before = log.of(record.height, &record.block_hash).cloned();
    let mut note = |statement_hash: String, outcome: SubmissionOutcome| {
        let s = Submission {
            height: record.height,
            block_hash: record.block_hash.clone(),
            statement_hash,
            attempts: before.as_ref().map_or(0, |b| b.attempts) + 1,
            first_tried_at: before.as_ref().map_or(now, |b| b.first_tried_at),
            last_tried_at: now,
            outcome,
        };
        log.record(s.clone());
        if let Err(e) = save_submissions(dir, &log) {
            eprintln!("[snapshot] could not write {SUBMITTED_FILE}: {e}");
        }
        s
    };

    let manifest = match std::fs::read(dir.join(manifest_file_name(record.height))) {
        Ok(m) => m,
        Err(e) => {
            let error = format!("the manifest could not be read: {e}");
            note(
                String::new(),
                SubmissionOutcome::Failed {
                    error: error.clone(),
                },
            );
            return SendOutcome::Failed { error };
        }
    };
    // The list first, from the bytes that would go out: no RPC for a node
    // that would send nothing anyway.
    let env = checks.regtest_env.as_deref();
    let listed = match read_manifest(&manifest, env) {
        Ok((m, _, operators)) => Ok((m.statement.hash().display_hex(), operators)),
        Err(r) => Err(NotSent::from(r)),
    };
    let (hash, operators) = match listed {
        Ok(x) => x,
        Err(n) => {
            note(
                String::new(),
                SubmissionOutcome::GivenUp {
                    reason: n.to_string(),
                },
            );
            return SendOutcome::NotSent(n);
        }
    };
    if operators.is_empty() {
        note(
            hash,
            SubmissionOutcome::GivenUp {
                reason: "this node's key is not on the operator list".into(),
            },
        );
        return SendOutcome::NotListed;
    }

    if let Err(n) = check_before_send(rpc, &manifest, &checks.diary_dir, &checks.holds, env).await {
        let outcome = if n.retry() {
            SubmissionOutcome::Failed {
                error: n.to_string(),
            }
        } else {
            SubmissionOutcome::GivenUp {
                reason: n.to_string(),
            }
        };
        note(hash, outcome);
        return SendOutcome::NotSent(n);
    }

    let file = dir.join(snapshot_file_name(record.height));
    match submit(client, site, &manifest, &file).await {
        Ok(sent) => SendOutcome::Sent(note(
            hash,
            SubmissionOutcome::Sent {
                at: now,
                file: sent.file,
                operators: sent.reply.operators,
                signers: sent.reply.signers,
                confirmed: sent.confirmed,
            },
        )),
        Err(e) if e.is_closed_height() || matches!(e, SiteError::NotSent(_)) => {
            let reason = e.to_string();
            note(
                hash,
                SubmissionOutcome::GivenUp {
                    reason: reason.clone(),
                },
            );
            SendOutcome::GivenUp { reason }
        }
        Err(e) => {
            let error = e.to_string();
            note(
                hash,
                SubmissionOutcome::Failed {
                    error: error.clone(),
                },
            );
            SendOutcome::Failed { error }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Diary, DiaryEntry};
    use crate::fake_node::{synthetic_hash, FakeNode};
    use crate::known_invalid::HeldBranch;
    use crate::operators::REGTEST_GENESIS;
    use crate::snapshot_serve::{self as serve, MatureEvent};
    use crate::snapshot_site::Site;
    use mockito::Matcher;
    use serde_json::json;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    /// The spike's statement hash and its producer's key.
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    /// 144 deep for the statement at 100.
    const DEEP: u64 = 243;

    fn env() -> Option<String> {
        Some(format!("producer={P}"))
    }

    fn hold_at(height: u64) -> &'static [HeldBranch] {
        Box::leak(Box::new([HeldBranch {
            height,
            root: ROOT,
            why: "a test hold",
        }]))
    }

    fn spike_entry() -> DiaryEntry {
        DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins: 101,
            chain_tx: 101,
            recorded_at: 0,
        }
    }

    /// The node that exported the spike's statement at 100, `tip` high, its
    /// diary written at 100 in `dir` (the datadir).
    fn producer_at(tip: u64) -> (FakeNode, tempfile::TempDir) {
        let n = FakeNode::new(REGTEST_GENESIS, tip);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
        });
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(spike_entry());
        crate::diary::save(dir.path(), &d).unwrap();
        (n, dir)
    }

    fn producer() -> (FakeNode, tempfile::TempDir) {
        producer_at(DEEP)
    }

    async fn check(n: &FakeNode, dir: &Path, holds: &Holds<'_>) -> Result<Checked, NotSent> {
        check_before_send(n, R_P, dir, holds, env().as_deref()).await
    }

    // ── The checks at 144 deep ──────────────────────────────────────────────

    #[tokio::test]
    async fn the_spikes_export_passes_on_the_node_that_made_it() {
        let (n, dir) = producer();
        let got = check(&n, dir.path(), &Holds::none()).await.unwrap();
        assert_eq!(got.entry, spike_entry());
        assert_eq!(got.operators, vec!["producer".to_string()]);
        assert_eq!(got.statement_hash, H);
        // No list for the chain: it still passes, and counts for nobody.
        let got = check_before_send(&n, R_P, dir.path(), &Holds::none(), None)
            .await
            .unwrap();
        assert!(got.operators.is_empty());
    }

    /// Fewer than 144 blocks deep on its own chain: nothing yet, ask again.
    #[tokio::test]
    async fn nothing_is_sent_before_144_deep() {
        let (n, dir) = producer_at(DEEP - 1);
        let got = check(&n, dir.path(), &Holds::none()).await.unwrap_err();
        assert_eq!(
            got,
            NotSent::Check(Mismatch::TooShallow {
                depth: 143,
                need: 144
            })
        );
        assert!(got.retry());
    }

    /// Its own diary entry for that height must match the statement field by
    /// field (`statement_check` has one case per field); one is enough here
    /// to show the producer does not send, for good.
    #[tokio::test]
    async fn a_producer_whose_diary_disagrees_sends_nothing() {
        let (n, dir) = producer();
        let mut d = crate::diary::load(dir.path(), REGTEST_GENESIS);
        d.record(DiaryEntry {
            hash_serialized: "55".repeat(32),
            ..spike_entry()
        });
        crate::diary::save(dir.path(), &d).unwrap();
        let got = check(&n, dir.path(), &Holds::none()).await.unwrap_err();
        assert!(
            matches!(&got, NotSent::Check(Mismatch::Differs { fields, .. }) if fields == &vec![crate::statement_check::Field::HashSerialized]),
            "{got:?}"
        );
        assert!(!got.retry());
        // No diary entry at all: nothing to vouch with.
        crate::diary::save(dir.path(), &Diary::new(REGTEST_GENESIS)).unwrap();
        let got = check(&n, dir.path(), &Holds::none()).await.unwrap_err();
        assert_eq!(got, NotSent::Check(Mismatch::NoDiaryEntry { height: 100 }));
        assert!(!got.retry());
    }

    /// Every held root the node knows is refused on it, before anything is
    /// compared. Refusing a root below the base takes the base off the
    /// chain: not sent, for good. A root the node cannot refuse stops
    /// everything, and is asked again. A root above the base on the chain is
    /// refused too, which takes the tip below 144 deep.
    #[tokio::test]
    async fn every_held_root_is_refused_first() {
        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: hold_at(50),
        };
        let got = check(&n, dir.path(), &holds).await.unwrap_err();
        assert_eq!(got, NotSent::BaseLeftChain { height: 100 });
        assert!(!got.retry());
        assert_eq!(n.count("invalidateblock"), 1);

        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
            s.invalidate_fails = true;
        });
        let got = check(&n, dir.path(), &holds).await.unwrap_err();
        assert!(matches!(got, NotSent::HeldNotRefused(_)), "{got:?}");
        assert!(got.retry());

        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(150, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: hold_at(150),
        };
        let got = check(&n, dir.path(), &holds).await.unwrap_err();
        assert_eq!(n.count("invalidateblock"), 1);
        assert!(
            matches!(got, NotSent::Check(Mismatch::TooShallow { depth: 50, .. })),
            "{got:?}"
        );

        // A hold on a branch the node has only seen: refused, and it does
        // not concern this statement.
        let (n, dir) = producer();
        n.with(|s| {
            s.side.insert(ROOT.into());
        });
        assert!(check(&n, dir.path(), &holds).await.is_ok());
        assert_eq!(n.count("invalidateblock"), 1);
        // One it never heard of is nothing to refuse.
        let (n, dir) = producer();
        assert!(check(&n, dir.path(), &holds).await.is_ok());
        assert_eq!(n.count("invalidateblock"), 0);
    }

    /// The base must still be the block at that height on the active chain.
    #[tokio::test]
    async fn a_base_that_left_the_chain_is_not_sent() {
        let (n, dir) = producer();
        n.with(|s| s.reorg_from(90, DEEP + 10));
        let got = check(&n, dir.path(), &Holds::none()).await.unwrap_err();
        assert_eq!(got, NotSent::BaseLeftChain { height: 100 });
    }

    /// `getchainstates` must still show every chainstate validated at 144
    /// deep, and a node that does not answer it is not an open gate. Read
    /// before anything changes the node.
    #[tokio::test]
    async fn an_unvalidated_chainstate_sends_nothing() {
        let (n, dir) = producer();
        n.with(|s| s.on_unchecked_snapshot(&synthetic_hash(50)));
        let holds = Holds {
            invalid: &[],
            held: hold_at(50),
        };
        let got = check(&n, dir.path(), &holds).await.unwrap_err();
        assert_eq!(got, NotSent::Unvalidated);
        assert!(got.retry());
        assert_eq!(n.count("invalidateblock"), 0);
        let (n, dir) = producer();
        n.with(|s| {
            s.silent.insert("getchainstates");
        });
        assert_eq!(
            check(&n, dir.path(), &Holds::none()).await.unwrap_err(),
            NotSent::Unvalidated
        );
    }

    /// A manifest that does not read, or whose signature does not verify, is
    /// refused before the node is asked anything.
    #[tokio::test]
    async fn a_manifest_with_a_bad_signature_is_not_sent() {
        let (n, dir) = producer();
        let mut bad = R_P.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let got = check_before_send(&n, &bad, dir.path(), &Holds::none(), None)
            .await
            .unwrap_err();
        assert!(matches!(got, NotSent::Unreadable(_)), "{got:?}");
        assert!(!got.retry());
        assert!(n.methods().is_empty());
        let got = check_before_send(&n, &R_P[..100], dir.path(), &Holds::none(), None)
            .await
            .unwrap_err();
        assert!(matches!(got, NotSent::Unreadable(_)), "{got:?}");
    }

    /// The keeper's hook: the diary right before the export, the checks
    /// before the offer, a refusal's `retry` passed through.
    #[tokio::test]
    async fn the_keepers_hook_writes_the_diary_and_runs_the_checks() {
        let (n, dir) = producer_at(100);
        n.with(|s| {
            s.utxo = Some(json!({
                "height": 100, "bestblock": R_BLOCK, "txouts": 101,
                "hash_serialized_3": R_UTXO, "transactions": 101
            }));
            s.chain_tx = 101;
        });
        crate::diary::save(dir.path(), &Diary::new(REGTEST_GENESIS)).unwrap();
        let hook = ProducerChecks {
            diary_dir: dir.path().to_path_buf(),
            holds: Holds::none(),
            regtest_env: env(),
        };
        hook.before_export(&n).await;
        let d = crate::diary::load(dir.path(), REGTEST_GENESIS);
        assert_eq!(d.at(100).map(|e| e.coins), Some(101), "written at the tip");
        let got = hook.check(&n, 100, R_P).await.unwrap_err();
        assert!(got.retry, "143 short: {got:?}");
        assert!(got.why.contains("blocks deep"), "{}", got.why);
        n.with(|s| s.extend_to(DEEP));
        assert_eq!(hook.check(&n, 100, R_P).await, Ok(()));
    }

    // ── What was sent, kept on disk ─────────────────────────────────────────

    fn offered(height: u64, block_hash: &str) -> OfferRecord {
        OfferRecord {
            height,
            block_hash: block_hash.into(),
            txoutset_hash: String::new(),
            file_size: 0,
            sha256: String::new(),
            manifest_sha256: String::new(),
            file_hash: String::new(),
            chunk_count: 0,
            signatures: 1,
            offered_at: 0,
        }
    }

    fn tried(height: u64, at: u64, outcome: SubmissionOutcome) -> Submission {
        Submission {
            height,
            block_hash: R_BLOCK.into(),
            statement_hash: H.into(),
            attempts: 1,
            first_tried_at: at,
            last_tried_at: at,
            outcome,
        }
    }

    /// Once per offered pair: a sent or abandoned one never again, a failed
    /// one after ten minutes, a pair off the grid (from before the design)
    /// never, and a sibling at a height already sent is a new pair.
    #[test]
    fn a_submission_is_due_once_per_pair_and_retried_after_ten_minutes() {
        let now = 1_000_000;
        let r = offered(100, R_BLOCK);
        let mut log = Submissions::default();
        assert!(submission_due(&log, &r, now));
        assert!(
            !submission_due(&log, &offered(226_140, R_BLOCK), now),
            "off the grid"
        );
        log.record(tried(
            100,
            now,
            SubmissionOutcome::Sent {
                at: now,
                file: "stored".into(),
                operators: vec!["producer".into()],
                signers: vec![P.into()],
                confirmed: false,
            },
        ));
        assert!(!submission_due(&log, &r, now + 86_400));
        assert!(submission_due(&log, &offered(100, "ab"), now), "a sibling");
        assert!(submission_due(&log, &offered(200, R_BLOCK), now));
        log.record(tried(
            100,
            now,
            SubmissionOutcome::Failed {
                error: "503".into(),
            },
        ));
        assert!(!submission_due(&log, &r, now + 599));
        assert!(submission_due(&log, &r, now + 600));
        log.record(tried(
            100,
            now,
            SubmissionOutcome::GivenUp {
                reason: "closed".into(),
            },
        ));
        assert!(!submission_due(&log, &r, now + 86_400));
        assert_eq!(RESUBMIT_EVERY, Duration::from_secs(600));
    }

    /// The record survives a restart and keeps the newest heights only.
    #[test]
    fn the_submission_record_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_submissions(dir.path()), Submissions::default());
        let mut log = Submissions::default();
        for i in 1..=(SUBMISSIONS_KEEP as u64 + 5) {
            log.record(tried(
                i * 100,
                i,
                SubmissionOutcome::Failed { error: "x".into() },
            ));
        }
        save_submissions(dir.path(), &log).unwrap();
        let back = load_submissions(dir.path());
        assert_eq!(back, log);
        assert_eq!(back.entries.len(), SUBMISSIONS_KEEP);
        assert_eq!(back.entries.first().unwrap().height, 600);
        assert!(dir.path().join(SUBMITTED_FILE).is_file());
        std::fs::write(dir.path().join(SUBMITTED_FILE), b"{ broken").unwrap();
        assert_eq!(load_submissions(dir.path()), Submissions::default());
    }

    // ── Sending ─────────────────────────────────────────────────────────────

    /// The producer's datadir with the offered pair at 100 in `snapshots/`.
    fn offered_pair(dir: &Path) -> PathBuf {
        let snaps = serve::snapshot_dir(dir);
        std::fs::create_dir_all(&snaps).unwrap();
        std::fs::write(snaps.join(serve::manifest_file_name(100)), R_P).unwrap();
        std::fs::write(snaps.join(serve::snapshot_file_name(100)), R_DAT).unwrap();
        snaps
    }

    fn checks(dir: &Path, regtest_env: Option<String>) -> ProducerChecks {
        ProducerChecks {
            diary_dir: dir.to_path_buf(),
            holds: Holds::none(),
            regtest_env,
        }
    }

    async fn statement_mock(server: &mut mockito::Server, file: &str) -> mockito::Mock {
        server
            .mock("POST", "/api/snapshots/statement")
            .match_header("x-ebtx-node", "ebtx-snapshot-v1")
            .match_body(R_P.to_vec())
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":["{P}"],"operators":["producer"],"added":1,"file":"{file}"}}"#
            ))
            .create_async()
            .await
    }

    async fn file_mocks(server: &mut mockito::Server) -> mockito::Mock {
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::UrlEncoded("action".into(), "start".into()))
            .with_body(
                r#"{"upload":"0123456789abcdef0123456789abcdef","part_bytes":4194304,"parts":1}"#,
            )
            .create_async()
            .await;
        let part = server
            .mock("PUT", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .match_body(R_DAT.to_vec())
            .with_body(r#"{"part":1,"bytes":8055}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::UrlEncoded("action".into(), "complete".into()))
            .with_body(r#"{"stored":true,"file_url":"u","file_sha256":"s","confirmed":false}"#)
            .create_async()
            .await;
        part
    }

    fn site(server: &mockito::Server) -> Site {
        Site::parse(Some(&server.url())).unwrap()
    }

    /// The statement, then the file, then a record of what the website said;
    /// asked again, nothing goes out twice.
    #[tokio::test]
    async fn a_checked_pair_is_sent_once_and_written_down() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        let mut server = mockito::Server::new_async().await;
        let statement = statement_mock(&mut server, "missing").await;
        let part = file_mocks(&mut server).await;
        let client = crate::snapshot_site::client().unwrap();
        let c = checks(dir.path(), env());
        let r = offered(100, R_BLOCK);
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_000).await;
        let SendOutcome::Sent(s) = &out else {
            panic!("{out:?}")
        };
        assert_eq!((s.height, s.statement_hash.as_str()), (100, H));
        assert!(matches!(
            &s.outcome,
            SubmissionOutcome::Sent { file, operators, .. } if file == "stored" && operators == &vec!["producer".to_string()]
        ));
        statement.assert_async().await;
        part.assert_async().await;
        // After a restart: on disk, not due, nothing sent again.
        let log = load_submissions(&snaps);
        assert_eq!(log.entries, vec![s.clone()]);
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 99_000).await;
        assert_eq!(out, SendOutcome::NotDue);
        statement.assert_async().await;
    }

    /// A website that already has the file gets the statement only.
    #[tokio::test]
    async fn a_file_the_website_has_is_not_sent_again() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        let mut server = mockito::Server::new_async().await;
        let statement = statement_mock(&mut server, "stored").await;
        let start = server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let out = send_offered(
            &n,
            &checks(dir.path(), env()),
            &client,
            &site(&server),
            &snaps,
            &offered(100, R_BLOCK),
            5_000,
        )
        .await;
        assert!(matches!(out, SendOutcome::Sent(_)), "{out:?}");
        statement.assert_async().await;
        start.assert_async().await;
    }

    /// A key that is not on the operator list sends nothing to the website
    /// (the P2P offer is the keeper's business), and says so once.
    #[tokio::test]
    async fn a_key_off_the_list_sends_nothing_to_the_website() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        let mut server = mockito::Server::new_async().await;
        let any = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let r = offered(100, R_BLOCK);
        let c = checks(
            dir.path(),
            Some(
                "someone=02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675".into(),
            ),
        );
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_000).await;
        assert_eq!(out, SendOutcome::NotListed);
        assert!(n.methods().is_empty(), "not even checked");
        assert!(matches!(
            load_submissions(&snaps).entries[0].outcome,
            SubmissionOutcome::GivenUp { .. }
        ));
        assert_eq!(
            send_offered(&n, &c, &client, &site(&server), &snaps, &r, 99_000).await,
            SendOutcome::NotDue
        );
        any.assert_async().await;
    }

    /// A pair that fails a check is never sent: nothing reaches the website,
    /// and a refusal that cannot change is not asked again.
    #[tokio::test]
    async fn a_pair_that_fails_a_check_is_never_sent() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        n.with(|s| s.reorg_from(90, DEEP + 10));
        let mut server = mockito::Server::new_async().await;
        let any = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let c = checks(dir.path(), env());
        let r = offered(100, R_BLOCK);
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_000).await;
        assert_eq!(
            out,
            SendOutcome::NotSent(NotSent::BaseLeftChain { height: 100 })
        );
        assert!(!submission_due(&load_submissions(&snaps), &r, 99_000));
        any.assert_async().await;
    }

    /// The website away: written down as failed, asked again after ten
    /// minutes and not before, then sent.
    #[tokio::test]
    async fn a_failed_send_is_tried_again_after_ten_minutes() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        let mut server = mockito::Server::new_async().await;
        let down = server
            .mock("POST", "/api/snapshots/statement")
            .with_status(503)
            .expect(1)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let c = checks(dir.path(), env());
        let r = offered(100, R_BLOCK);
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_000).await;
        assert!(
            matches!(&out, SendOutcome::Failed { error } if error.contains("503")),
            "{out:?}"
        );
        assert_eq!(
            send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_599).await,
            SendOutcome::NotDue
        );
        down.assert_async().await;
        down.remove_async().await;
        statement_mock(&mut server, "stored").await;
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_600).await;
        let SendOutcome::Sent(s) = out else {
            panic!("{out:?}")
        };
        assert_eq!(
            (s.attempts, s.first_tried_at, s.last_tried_at),
            (2, 5_000, 5_600)
        );
    }

    /// A height the owner closed (section 6a) takes nothing again: given up,
    /// never retried.
    #[tokio::test]
    async fn a_closed_height_is_given_up() {
        let (n, dir) = producer();
        let snaps = offered_pair(dir.path());
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"closed-height"}"#)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let c = checks(dir.path(), env());
        let r = offered(100, R_BLOCK);
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, &r, 5_000).await;
        assert!(matches!(out, SendOutcome::GivenUp { .. }), "{out:?}");
        assert!(!submission_due(&load_submissions(&snaps), &r, 1_000_000));
    }

    /// The whole producer on one node: the export at 100 with the diary
    /// written first, nothing at 143 deep, the checks and the P2P offer at
    /// 144, then the website.
    #[tokio::test]
    async fn the_producer_exports_on_the_grid_offers_at_144_and_sends() {
        let n = FakeNode::new(REGTEST_GENESIS, 100);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
            s.utxo = Some(json!({
                "height": 100, "bestblock": R_BLOCK, "txouts": 101,
                "hash_serialized_3": R_UTXO, "transactions": 101
            }));
            s.chain_tx = 101;
            s.dump = Some((R_DAT.to_vec(), R_P.to_vec()));
        });
        let dir = tempfile::tempdir().unwrap();
        let snaps = serve::snapshot_dir(dir.path());
        let c = checks(dir.path(), env());
        serve::export_on_grid(&n, &snaps, serve::EXPORT_GRID, &c, &|_| {})
            .await
            .unwrap();
        assert!(crate::diary::load(dir.path(), REGTEST_GENESIS)
            .at(100)
            .is_some());

        n.with(|s| s.extend_to(DEEP - 1));
        let events = serve::mature(&n, &snaps, serve::MATURE_DEADLINE, &c, &|_| {}, &|| true).await;
        assert!(
            matches!(
                &events[..],
                [MatureEvent::Waiting {
                    confirmations: 143,
                    ..
                }]
            ),
            "{events:?}"
        );
        assert_eq!(n.count("offerattestedutxosnapshot"), 0);

        n.with(|s| s.extend_to(DEEP));
        let events = serve::mature(&n, &snaps, serve::MATURE_DEADLINE, &c, &|_| {}, &|| true).await;
        let [MatureEvent::Offered(record)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(record.height, 100);

        let mut server = mockito::Server::new_async().await;
        let statement = statement_mock(&mut server, "missing").await;
        let part = file_mocks(&mut server).await;
        let client = crate::snapshot_site::client().unwrap();
        let out = send_offered(&n, &c, &client, &site(&server), &snaps, record, 5_000).await;
        assert!(matches!(out, SendOutcome::Sent(_)), "{out:?}");
        statement.assert_async().await;
        part.assert_async().await;
    }
}
