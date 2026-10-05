//! The confirmer (section 5 of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md): a node that validates, signs, and whose key is on the
//! operator list reads the statements waiting on easybtx.com about every ten
//! minutes and answers each one with its OWN node's account of that height.
//!
//! ONE ROUND, IN ORDER:
//!
//! 1. `getchainstates`. While any chainstate shows `"validated": false` (a
//!    signed snapshot or upstream's plain assumeutxo one, history still being
//!    checked), the round stops there and signs nothing ([`NoRound::Unvalidated`]):
//!    otherwise one snapshot could vouch for the next (section 3).
//! 2. `GET /api/snapshots/pending`. Nothing in it is trusted: every statement
//!    is read from its own manifest bytes, its listed hash and height must be
//!    the bytes' own, and every signature on it must verify.
//! 3. Per statement that is not a dissent, `crate::statement_check`: the
//!    chain id, replay context and shielded commitment (check 3), 144 deep
//!    (check 1), a diary entry whose block is still on the active chain
//!    (check 2), no refused block under it, then the five chain fields
//!    against that entry (check 4). [`Mismatch::next`] decides: wait, say
//!    nothing, or dissent.
//! 4. All passed: `signutxosnapshotmanifest` on a COPY of the listed manifest
//!    in the work folder, then exactly one new signature is taken from it,
//!    checked to be this node's key and valid over the statement, and a
//!    manifest carrying the statement and that one signature goes back
//!    through `POST /api/snapshots/statement`, where the website merges it
//!    (`mergeSignatures` in snapshotRendezvous.mjs: one per key, stored ones
//!    first). Only check 4 failed: a dissent instead (below).
//!
//! WHY A COPY, AND WHY ONLY ONE SIGNATURE BACK. `signutxosnapshotmanifest`
//! signs blindly (it checks only the chain id, the replay context and its own
//! key, `trusted_exact_replay_attestation.cpp:1387-1403`), so it only ever
//! sees a statement that already passed, and it rewrites the file it is
//! given, so it never sees one the app still needs. Sending back the
//! website's whole merged manifest would re-send signatures this node did
//! not make; the one new signature is all this node can vouch for.
//!
//! THE DISSENT (section 6a). Built from the diary entry, not the statement:
//! height, block hash, `hash_serialized_3`, coin count and chain transaction
//! count from the diary, the chain id and replay context the node just
//! confirmed (check 3), the compiled shielded commitment, and all four file
//! fields zero (`cs::dissent_statement`). The engine signs it as an unsigned
//! manifest; the app checks the one signature and posts it as any statement.
//! The website takes it because every field but the file is in the chain's
//! rules and the file fields are all zero (`acceptManifest`, `isDissent`).
//! Two confirmers whose diaries agree build the very same dissent, which the
//! website merges like co-signatures.
//!
//! NEVER TWICE. [`ConfirmerLog`] (`snapshot-confirmer.json` beside the diary,
//! newest [`LOG_KEEP`] entries) is written BEFORE anything is sent, holding
//! the signed manifest until the website took it. A send that failed is
//! retried with those same bytes, after the checks pass again, so the engine
//! signs each statement once, and each height gets at most one dissent from
//! this node. The website's own listing counts too: a statement that already
//! carries this operator's signature (this key or another of the operator's
//! keys), or a height where a listed dissent carries it, is left alone.
//!
//! COUNTED. Every outcome is a [`Verdict`]; [`Tally`] counts them and says
//! each one once per run for the log, and [`NetworkReport`] is the
//! "Snapshots" section of Copy diagnostics.
//!
//! WHO. [`why_not`] is the standing gate the app reads before it builds a
//! [`Confirmer`]: consensus mode, a local signer, its key readable and on its
//! chain's list. The `getchainstates` gate is per round, since a node leaves
//! it by itself when its background check finishes.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Hash32, Manifest, Signed, Statement};
use crate::diary::{self, DiaryEntry};
use crate::operators::{self, Chain};
use crate::rpc::Rpc;
use crate::snapshot_site::{self as site, Pending, PendingStatement, Site};
use crate::statement_check::{check_against_node, Field, Mismatch, Next};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How often a confirmer reads what waits.
pub const CONFIRM_EVERY_SECS: u64 = 600;
/// Under `<datadir>/snapshots/`: where a copy is signed and removed again.
pub const WORK_DIR: &str = "confirm";
/// Beside the diary: what this node signed and sent.
pub const LOG_FILE: &str = "snapshot-confirmer.json";
pub const LOG_VERSION: u32 = 1;
/// Entries kept: a co-signature and perhaps a dissent per grid height, for
/// far longer than the seven days `pending` lists a statement.
pub const LOG_KEEP: usize = 200;

// ── What one statement came to ──────────────────────────────────────────────

/// What one listed statement came to. `height` and `statement_hash` are the
/// listing's until the bytes are read, the bytes' own after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// This node's one signature went to the website. `operators` is the
    /// website's count after the merge: its word, for the log only.
    Signed {
        height: u64,
        statement_hash: String,
        operators: Vec<String>,
    },
    /// This node's diary disagrees in `fields`: its signed dissent went to
    /// the website (filed there under `dissent_hash`). The statement itself
    /// was never signed.
    Dissented {
        height: u64,
        statement_hash: String,
        dissent_hash: String,
        fields: Vec<Field>,
        operators: Vec<String>,
    },
    /// This operator signed it already, with this key or another of its
    /// keys, or this node sent its signature before.
    AlreadySigned { height: u64, statement_hash: String },
    /// This node disagrees, and its dissent at that height is out already.
    AlreadyDissented { height: u64, statement_hash: String },
    /// The listing is a dissent (all four file fields zero): never signed.
    ListedDissent { height: u64, statement_hash: String },
    /// Not yet: fewer than 144 blocks deep, or the node did not answer.
    Waiting {
        height: u64,
        statement_hash: String,
        why: Mismatch,
    },
    /// Nothing this node can vouch for either way (another chain or replay
    /// context, no diary entry on its chain, a refused block under it):
    /// neither signed nor dissented.
    Skipped {
        height: u64,
        statement_hash: String,
        why: Mismatch,
    },
    /// The listing does not read: not hex, not a manifest, not the hash or
    /// height it is listed as, a signature that does not verify, a chain
    /// this app does not know. Asked the node nothing.
    Unreadable {
        height: u64,
        statement_hash: String,
        why: String,
    },
    /// Something failed on the way (the engine, the work folder, the
    /// website). Whatever was signed is kept and sent next round.
    Failed {
        height: u64,
        statement_hash: String,
        error: String,
    },
}

impl Verdict {
    pub fn height(&self) -> u64 {
        match self {
            Verdict::Signed { height, .. }
            | Verdict::Dissented { height, .. }
            | Verdict::AlreadySigned { height, .. }
            | Verdict::AlreadyDissented { height, .. }
            | Verdict::ListedDissent { height, .. }
            | Verdict::Waiting { height, .. }
            | Verdict::Skipped { height, .. }
            | Verdict::Unreadable { height, .. }
            | Verdict::Failed { height, .. } => *height,
        }
    }

    pub fn statement_hash(&self) -> &str {
        match self {
            Verdict::Signed { statement_hash, .. }
            | Verdict::Dissented { statement_hash, .. }
            | Verdict::AlreadySigned { statement_hash, .. }
            | Verdict::AlreadyDissented { statement_hash, .. }
            | Verdict::ListedDissent { statement_hash, .. }
            | Verdict::Waiting { statement_hash, .. }
            | Verdict::Skipped { statement_hash, .. }
            | Verdict::Unreadable { statement_hash, .. }
            | Verdict::Failed { statement_hash, .. } => statement_hash,
        }
    }

    /// One plain line for the node log.
    pub fn line(&self) -> String {
        let h = self.height();
        match self {
            Verdict::Signed { operators, .. } if operators.is_empty() => {
                format!("co-signed the snapshot at {h}")
            }
            Verdict::Signed { operators, .. } => format!(
                "co-signed the snapshot at {h}; signed by {} now",
                operators.join(", ")
            ),
            Verdict::Dissented { fields, .. } => format!(
                "sent a dissent about the snapshot at {h}: this node's diary differs in {}",
                fields
                    .iter()
                    .map(|f| f.words())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Verdict::AlreadySigned { .. } => {
                format!("the snapshot at {h} carries this operator's signature already")
            }
            Verdict::AlreadyDissented { .. } => {
                format!("this node's dissent at {h} is out already")
            }
            Verdict::ListedDissent { .. } => {
                format!("the statement at {h} is a dissent, which nobody co-signs")
            }
            Verdict::Waiting { why, .. } => {
                format!("not yet for the snapshot at {h}: {why}")
            }
            Verdict::Skipped { why, .. } => format!("did not sign the snapshot at {h}: {why}"),
            Verdict::Unreadable { why, .. } => {
                format!("did not sign the snapshot at {h}: {why}")
            }
            Verdict::Failed { error, .. } => {
                format!("could not answer the snapshot at {h} yet: {error}")
            }
        }
    }
}

/// Why a round did not look at any statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoRound {
    /// `getchainstates` shows a chainstate at `"validated": false`, or did
    /// not answer. Nothing was asked of the website.
    Unvalidated,
    /// `pending` could not be read.
    Site(String),
}

impl std::fmt::Display for NoRound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoRound::Unvalidated => write!(
                f,
                "this node is still checking older history in the background, so it signs nothing"
            ),
            NoRound::Site(e) => write!(f, "{e}"),
        }
    }
}

/// Why this node does not confirm, or `None` when it may: it validates,
/// signs, its key reads, and the key is on the list of `genesis`'s chain.
/// The `getchainstates` gate is not here: [`Confirmer::round`] reads it every
/// round, since a node passes it by itself once its history is checked.
pub fn why_not(
    status: &crate::node_api::MatmulTrustedStatus,
    our_key_hex: Option<&str>,
    genesis: &str,
    regtest_env: Option<&str>,
) -> Option<&'static str> {
    if !status
        .matmul_validation_mode
        .trim()
        .eq_ignore_ascii_case("consensus")
    {
        return Some("it follows signatures instead of checking blocks");
    }
    if !status.local_signer {
        return Some("it does not sign");
    }
    let Some(key) = our_key_hex.and_then(operators::parse_key) else {
        return Some("its signing key could not be read");
    };
    let Some(chain) = Chain::from_genesis_hex(genesis) else {
        return Some("its chain has no snapshot network");
    };
    if operators::for_chain(chain, regtest_env)
        .operator_of(&key)
        .is_none()
    {
        return Some("its key is not on the operator list");
    }
    None
}

// ── What this node signed ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    /// A co-signature on someone's statement.
    Signed,
    /// This node's own dissent.
    Dissent,
}

/// One signature this node's engine made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// `main` or `regtest`.
    pub chain: String,
    pub height: u64,
    pub kind: LogKind,
    /// What was signed: the statement, or for a dissent the dissent itself.
    pub statement_hash: String,
    /// For a dissent: the statement it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub against: Option<String>,
    /// The manifest to send (the statement and this node's one signature),
    /// hex, kept only until the website took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsent: Option<String>,
    /// Unix seconds.
    pub signed_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<u64>,
}

/// `<state>/snapshot-confirmer.json`, ascending by height.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmerLog {
    pub version: u32,
    pub entries: Vec<LogEntry>,
}

impl Default for ConfirmerLog {
    fn default() -> Self {
        Self {
            version: LOG_VERSION,
            entries: Vec::new(),
        }
    }
}

impl ConfirmerLog {
    /// This node's co-signature on `statement_hash`.
    pub fn signature(&self, chain: &str, statement_hash: &str) -> Option<&LogEntry> {
        self.entries.iter().find(|e| {
            e.kind == LogKind::Signed && e.chain == chain && e.statement_hash == statement_hash
        })
    }

    /// This node's dissent at `height`.
    pub fn dissent_at(&self, chain: &str, height: u64) -> Option<&LogEntry> {
        self.entries
            .iter()
            .find(|e| e.kind == LogKind::Dissent && e.chain == chain && e.height == height)
    }

    /// Keep `e`, replacing the co-signature on the same statement or the
    /// dissent at the same height, and only the newest [`LOG_KEEP`].
    pub fn record(&mut self, e: LogEntry) {
        self.entries.retain(|x| {
            x.chain != e.chain
                || x.kind != e.kind
                || match e.kind {
                    LogKind::Signed => x.statement_hash != e.statement_hash,
                    LogKind::Dissent => x.height != e.height,
                }
        });
        self.entries.push(e);
        self.entries.sort_by_key(|x| (x.height, x.signed_at));
        let excess = self.entries.len().saturating_sub(LOG_KEEP);
        self.entries.drain(..excess);
    }

    /// The website took `statement_hash`: drop the bytes, keep the fact.
    fn mark_sent(&mut self, chain: &str, kind: LogKind, statement_hash: &str, now: u64) {
        for e in &mut self.entries {
            if e.chain == chain && e.kind == kind && e.statement_hash == statement_hash {
                e.unsent = None;
                e.sent_at.get_or_insert(now);
            }
        }
    }
}

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join(LOG_FILE)
}

/// Missing, unreadable or another version reads as empty: at worst the
/// engine signs a statement again, and the website keeps one per key.
pub fn load_log(dir: &Path) -> ConfirmerLog {
    std::fs::read(log_path(dir))
        .ok()
        .and_then(|raw| serde_json::from_slice::<ConfirmerLog>(&raw).ok())
        .filter(|l| l.version == LOG_VERSION)
        .unwrap_or_default()
}

/// Written atomically.
pub fn save_log(dir: &Path, log: &ConfirmerLog) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(log).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&log_path(dir), &json)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The dissent for a diary entry that disagrees with `st` (section 6a): the
/// diary's chain facts, `st`'s chain id and replay context (check 3 found
/// them the node's own), the compiled shielded commitment, file fields zero.
pub fn dissent_for(
    entry: &DiaryEntry,
    st: &Statement,
    rules: &ChainRules,
) -> Result<Statement, String> {
    let hash = |what: &str, hex: &str| {
        Hash32::from_display_hex(hex).ok_or_else(|| format!("the diary's {what} is not a hash"))
    };
    let raw = cs::dissent_statement(
        entry.height,
        &hash("block hash", &entry.block_hash)?,
        &hash("UTXO hash", &entry.hash_serialized)?,
        entry.coins,
        entry.chain_tx,
        &st.chain_id(),
        &rules.replay_context,
        &rules.shielded,
    )
    .ok_or_else(|| format!("height {} does not fit a statement", entry.height))?;
    let dissent = Statement::from_raw(raw);
    cs::check_dissent_shape(&dissent, rules).map_err(|e| e.to_string())?;
    Ok(dissent)
}

// ── The confirmer ───────────────────────────────────────────────────────────

/// One confirmer, for one round.
pub struct Confirmer<'a> {
    pub rpc: &'a dyn Rpc,
    pub client: &'a reqwest::Client,
    pub site: &'a Site,
    /// Holds the diary and [`ConfirmerLog`]: the datadir, or `btx-confirmer`'s
    /// `--state`.
    pub state_dir: &'a Path,
    /// Where copies are signed. The engine reads and rewrites the file, so
    /// it must be on the node's machine and inside its datadir
    /// (`<datadir>/snapshots/confirm`).
    pub work_dir: &'a Path,
    /// This node's signing key, compressed.
    pub our_key: [u8; 33],
    /// `Holds::compiled()` in the app.
    pub holds: Holds<'a>,
    /// `operators::regtest_env()` in the app; only a regtest statement reads it.
    pub regtest_env: Option<&'a str>,
}

/// What a listing says, read from its own bytes.
struct Read {
    manifest: Manifest,
    rules: ChainRules,
    signed_by: Vec<String>,
}

impl Confirmer<'_> {
    /// Section 5's round on `chain`: the `getchainstates` gate, then every
    /// statement waiting, newest height first as the website lists them.
    pub async fn round(&self, chain: Chain) -> Result<Vec<Verdict>, NoRound> {
        if !crate::node_api::read_chainstates_validated(self.rpc).await {
            return Err(NoRound::Unvalidated);
        }
        let pending = site::get_pending(self.client, self.site, chain)
            .await
            .map_err(|e| NoRound::Site(e.to_string()))?;
        let dissented = self.listed_dissents_of_ours(&pending);
        let mut out = Vec::with_capacity(pending.statements.len());
        for p in &pending.statements {
            out.push(self.confirm(p, &dissented).await);
        }
        Ok(out)
    }

    /// One statement, with nothing known from the rest of the listing.
    pub async fn confirm_one(&self, p: &PendingStatement) -> Verdict {
        self.confirm(p, &HashSet::new()).await
    }

    /// The listing read from its bytes: the hash and height it is listed
    /// under are its own, its chain is known, every signature verifies.
    fn read(&self, p: &PendingStatement) -> Result<Read, String> {
        let bytes = operators::hex_decode(&p.manifest_hex).ok_or("the manifest is not hex")?;
        let manifest = cs::parse(&bytes).map_err(|e| e.to_string())?;
        let st = &manifest.statement;
        if !st
            .hash()
            .display_hex()
            .eq_ignore_ascii_case(&p.statement_hash)
        {
            return Err("the manifest is not the statement it is listed as".into());
        }
        if i64::from(st.height()) != p.height as i64 {
            return Err(format!(
                "the statement is at {}, listed at {}",
                st.height(),
                p.height
            ));
        }
        let rules = ChainRules::for_statement(st, self.regtest_env).map_err(|e| e.to_string())?;
        let signed_by =
            cs::confirming_operators(&manifest, &rules.operators).map_err(|e| e.to_string())?;
        Ok(Read {
            manifest,
            rules,
            signed_by,
        })
    }

    /// Whether `key` is this node's, or another key of its operator.
    fn ours(&self, rules: &ChainRules, key: &[u8; 33]) -> bool {
        *key == self.our_key
            || rules
                .operators
                .operator_of(&self.our_key)
                .is_some_and(|o| rules.operators.operator_of(key) == Some(o))
    }

    /// Heights where a listed dissent carries a valid signature of this
    /// operator: one dissent per height is enough, wherever it was written
    /// down.
    fn listed_dissents_of_ours(&self, pending: &Pending) -> HashSet<u64> {
        pending
            .statements
            .iter()
            .filter_map(|p| self.read(p).ok())
            .filter(|r| cs::is_dissent(&r.manifest.statement))
            .filter(|r| {
                r.manifest
                    .signatures
                    .iter()
                    .any(|s| self.ours(&r.rules, &s.key))
            })
            .map(|r| r.manifest.statement.height() as u64)
            .collect()
    }

    async fn confirm(&self, p: &PendingStatement, dissented: &HashSet<u64>) -> Verdict {
        let height = p.height;
        let statement_hash = p.statement_hash.to_ascii_lowercase();
        let r = match self.read(p) {
            Ok(r) => r,
            Err(why) => {
                return Verdict::Unreadable {
                    height,
                    statement_hash,
                    why,
                }
            }
        };
        if cs::is_dissent(&r.manifest.statement) {
            return Verdict::ListedDissent {
                height,
                statement_hash,
            };
        }
        let Some(ours) = r.rules.operators.operator_of(&self.our_key) else {
            return Verdict::Failed {
                height,
                statement_hash,
                error: "this node's key is not on the operator list".into(),
            };
        };
        let chain = site::chain_name(r.rules.chain);
        let mut log = load_log(self.state_dir);
        let on_site = r.signed_by.iter().any(|o| o == ours)
            || r.manifest.signatures.iter().any(|s| s.key == self.our_key);
        let sent_before = log
            .signature(chain, &statement_hash)
            .is_some_and(|e| e.unsent.is_none());
        if on_site || sent_before {
            if on_site && log.signature(chain, &statement_hash).is_some() {
                // The website has it, so whatever is still marked unsent
                // went through after all.
                log.mark_sent(chain, LogKind::Signed, &statement_hash, now_unix());
                self.save(&log);
            }
            return Verdict::AlreadySigned {
                height,
                statement_hash,
            };
        }

        let st = &r.manifest.statement;
        let diary = diary::load(self.state_dir, &st.chain_id().display_hex());
        match check_against_node(self.rpc, st, &r.rules, &diary, &self.holds).await {
            Ok(_) => {
                self.cosign(&r.manifest, chain, &statement_hash, height, &mut log)
                    .await
            }
            Err(why) => match why.next() {
                Next::Wait => Verdict::Waiting {
                    height,
                    statement_hash,
                    why,
                },
                Next::Skip => Verdict::Skipped {
                    height,
                    statement_hash,
                    why,
                },
                Next::Dissent => {
                    let sent = log
                        .dissent_at(chain, height)
                        .is_some_and(|e| e.unsent.is_none());
                    if sent || dissented.contains(&height) {
                        return Verdict::AlreadyDissented {
                            height,
                            statement_hash,
                        };
                    }
                    self.dissent(why, &r, chain, &statement_hash, &mut log)
                        .await
                }
            },
        }
    }

    fn save(&self, log: &ConfirmerLog) {
        if let Err(e) = save_log(self.state_dir, log) {
            eprintln!("[confirmer] could not write {LOG_FILE}: {e}");
        }
    }

    /// The manifest kept unsent for `statement`, when it still reads as the
    /// statement with this node's one valid signature.
    fn kept(&self, unsent: Option<&String>, statement: &Statement) -> Option<Vec<u8>> {
        let bytes = operators::hex_decode(unsent?)?;
        let m = cs::parse(&bytes).ok()?;
        let ok = m.statement == *statement
            && m.signatures.len() == 1
            && m.signatures[0].key == self.our_key
            && cs::signature_is_valid(&statement.hash(), &self.our_key, &m.signatures[0].der);
        ok.then_some(bytes)
    }

    /// Every check passed: sign a copy (or take the signature kept from a
    /// send that failed), write it down, send it.
    async fn cosign(
        &self,
        m: &Manifest,
        chain: &str,
        statement_hash: &str,
        height: u64,
        log: &mut ConfirmerLog,
    ) -> Verdict {
        let failed = |error: String| Verdict::Failed {
            height,
            statement_hash: statement_hash.to_string(),
            error,
        };
        let kept = log
            .signature(chain, statement_hash)
            .and_then(|e| self.kept(e.unsent.as_ref(), &m.statement));
        let one = match kept {
            Some(bytes) => bytes,
            None => {
                let sig = match self
                    .sign_copy(m, &format!("{statement_hash}.manifest"))
                    .await
                {
                    Ok(s) => s,
                    Err(e) => return failed(e),
                };
                let one = Manifest {
                    statement: m.statement.clone(),
                    signatures: vec![sig],
                }
                .to_bytes();
                log.record(LogEntry {
                    chain: chain.into(),
                    height,
                    kind: LogKind::Signed,
                    statement_hash: statement_hash.into(),
                    against: None,
                    unsent: Some(operators::hex(&one)),
                    signed_at: now_unix(),
                    sent_at: None,
                });
                self.save(log);
                one
            }
        };
        match self.send(&one, statement_hash).await {
            Ok(operators) => {
                log.mark_sent(chain, LogKind::Signed, statement_hash, now_unix());
                self.save(log);
                Verdict::Signed {
                    height,
                    statement_hash: statement_hash.into(),
                    operators,
                }
            }
            Err(e) => failed(e),
        }
    }

    /// Only check 4 failed: sign and send this node's own account of the
    /// height, once per height.
    async fn dissent(
        &self,
        why: Mismatch,
        r: &Read,
        chain: &str,
        statement_hash: &str,
        log: &mut ConfirmerLog,
    ) -> Verdict {
        let height = r.manifest.statement.height() as u64;
        let failed = |error: String| Verdict::Failed {
            height,
            statement_hash: statement_hash.to_string(),
            error,
        };
        let Mismatch::Differs { entry, fields, .. } = why else {
            return failed(format!("not a disagreement: {why}"));
        };
        let dissent = match dissent_for(&entry, &r.manifest.statement, &r.rules) {
            Ok(d) => d,
            Err(e) => return failed(format!("the dissent: {e}")),
        };
        let dissent_hash = dissent.hash().display_hex();
        // A dissent signed before and not sent goes now, if it says the
        // same; one built from a diary entry since replaced is signed anew.
        let kept = log
            .dissent_at(chain, height)
            .filter(|e| e.statement_hash == dissent_hash)
            .and_then(|e| self.kept(e.unsent.as_ref(), &dissent));
        let one = match kept {
            Some(bytes) => bytes,
            None => {
                let unsigned = Manifest {
                    statement: dissent.clone(),
                    signatures: Vec::new(),
                };
                let sig = match self
                    .sign_copy(&unsigned, &format!("dissent-{dissent_hash}.manifest"))
                    .await
                {
                    Ok(s) => s,
                    Err(e) => return failed(e),
                };
                let one = Manifest {
                    statement: dissent,
                    signatures: vec![sig],
                }
                .to_bytes();
                log.record(LogEntry {
                    chain: chain.into(),
                    height,
                    kind: LogKind::Dissent,
                    statement_hash: dissent_hash.clone(),
                    against: Some(statement_hash.into()),
                    unsent: Some(operators::hex(&one)),
                    signed_at: now_unix(),
                    sent_at: None,
                });
                self.save(log);
                one
            }
        };
        match self.send(&one, &dissent_hash).await {
            Ok(operators) => {
                log.mark_sent(chain, LogKind::Dissent, &dissent_hash, now_unix());
                self.save(log);
                Verdict::Dissented {
                    height,
                    statement_hash: statement_hash.into(),
                    dissent_hash,
                    fields,
                    operators,
                }
            }
            Err(e) => failed(e),
        }
    }

    /// Post a one-signature manifest; the website must file it under
    /// `statement_hash`. Its operator count comes back, for the log.
    async fn send(&self, manifest: &[u8], statement_hash: &str) -> Result<Vec<String>, String> {
        let reply = site::post_statement(self.client, self.site, manifest)
            .await
            .map_err(|e| e.to_string())?;
        if !reply.statement_hash.eq_ignore_ascii_case(statement_hash) {
            return Err(format!(
                "the snapshot site filed it as {}",
                reply.statement_hash
            ));
        }
        Ok(reply.operators)
    }

    /// Sign a copy of `m` with the engine, and take exactly the one
    /// signature it added: this node's key, valid over the statement. The
    /// copy is removed whatever happens.
    async fn sign_copy(&self, m: &Manifest, name: &str) -> Result<Signed, String> {
        std::fs::create_dir_all(self.work_dir).map_err(|e| format!("work folder: {e}"))?;
        let path = self.work_dir.join(name);
        std::fs::write(&path, m.to_bytes()).map_err(|e| format!("writing the copy: {e}"))?;
        let signed = self
            .rpc
            .call("signutxosnapshotmanifest", json!([path.to_string_lossy()]))
            .await;
        let after = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        signed.map_err(|e| format!("the engine did not sign: {e}"))?;
        let mut after = cs::parse(&after.map_err(|e| format!("reading the copy: {e}"))?)
            .map_err(|e| format!("the signed copy does not read: {e}"))?;
        let n = m.signatures.len();
        if after.statement != m.statement
            || after.signatures.len() != n + 1
            || after.signatures[..n] != m.signatures[..]
        {
            return Err("the engine did not add exactly one signature".into());
        }
        let new = after.signatures.pop().expect("n + 1 signatures");
        if new.key != self.our_key {
            return Err("the engine did not sign with this node's key".into());
        }
        if !cs::signature_is_valid(&m.statement.hash(), &new.key, &new.der) {
            return Err("the engine's signature does not check out".into());
        }
        Ok(new)
    }
}

// ── Counted, for Copy diagnostics ───────────────────────────────────────────

/// What the confirmer did this run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// Every round started, including the ones that stopped.
    pub rounds: u64,
    /// Rounds that looked at nothing ([`NoRound`]).
    pub stopped: u64,
    pub signed: u64,
    pub dissented: u64,
    /// The rest count each statement once per run, whatever the rounds.
    pub waiting: u64,
    pub skipped: u64,
    pub unreadable: u64,
    pub failed: u64,
    /// Signed or dissented already, by this node or its operator.
    pub already: u64,
    pub listed_dissents: u64,
    /// The last statement this node did not sign, and why.
    pub last_refusal: Option<String>,
    /// Why the last round stopped, while it does.
    pub last_error: Option<String>,
}

fn kind(v: &Verdict) -> &'static str {
    match v {
        Verdict::Signed { .. } => "signed",
        Verdict::Dissented { .. } => "dissented",
        Verdict::AlreadySigned { .. } | Verdict::AlreadyDissented { .. } => "already",
        Verdict::ListedDissent { .. } => "dissent",
        Verdict::Waiting { .. } => "waiting",
        Verdict::Skipped { .. } => "skipped",
        Verdict::Unreadable { .. } => "unreadable",
        Verdict::Failed { .. } => "failed",
    }
}

impl Tally {
    /// Count a round. Returns the log lines this run has not said yet
    /// (`seen` lives as long as the run), so a statement that waits or is
    /// refused is said once, not every ten minutes. Statements signed or
    /// dissented already, and listed dissents, are the quiet normal state:
    /// counted, not said.
    pub fn add(&mut self, verdicts: &[Verdict], seen: &mut HashSet<String>) -> Vec<String> {
        self.rounds += 1;
        self.last_error = None;
        let mut lines = Vec::new();
        for v in verdicts {
            let first = seen.insert(format!("{}:{}", kind(v), v.statement_hash()));
            match v {
                Verdict::Signed { .. } => self.signed += 1,
                Verdict::Dissented { .. } => {
                    self.dissented += 1;
                    self.last_refusal = Some(v.line());
                }
                Verdict::Skipped { .. } | Verdict::Unreadable { .. } => {
                    self.last_refusal = Some(v.line());
                    if first {
                        if matches!(v, Verdict::Skipped { .. }) {
                            self.skipped += 1;
                        } else {
                            self.unreadable += 1;
                        }
                    }
                }
                _ if !first => {}
                Verdict::Waiting { .. } => self.waiting += 1,
                Verdict::Failed { .. } => self.failed += 1,
                Verdict::AlreadySigned { .. } | Verdict::AlreadyDissented { .. } => {
                    self.already += 1
                }
                Verdict::ListedDissent { .. } => self.listed_dissents += 1,
            }
            let quiet = matches!(
                v,
                Verdict::AlreadySigned { .. }
                    | Verdict::AlreadyDissented { .. }
                    | Verdict::ListedDissent { .. }
            );
            if first && !quiet {
                lines.push(v.line());
            }
        }
        lines
    }

    /// Count a round that stopped. The line, when this run has not said it.
    pub fn stopped(&mut self, why: &NoRound, seen: &mut HashSet<String>) -> Option<String> {
        self.rounds += 1;
        self.stopped += 1;
        let line = format!("confirmer round stopped: {why}");
        self.last_error = Some(why.to_string());
        seen.insert(line.clone()).then_some(line)
    }

    /// The lines Copy diagnostics shows.
    pub fn report(&self) -> Vec<String> {
        let mut out = vec![format!(
            "confirmer: {} rounds ({} stopped), co-signed {}, dissented {}, waiting {}, \
             skipped {}, unreadable {}, failed {}, done before {}, dissents listed {}",
            self.rounds,
            self.stopped,
            self.signed,
            self.dissented,
            self.waiting,
            self.skipped,
            self.unreadable,
            self.failed,
            self.already,
            self.listed_dissents
        )];
        if let Some(r) = &self.last_refusal {
            out.push(format!("confirmer, last refusal: {r}"));
        }
        if let Some(e) = &self.last_error {
            out.push(format!("confirmer, last round: {e}"));
        }
        out
    }
}

/// What the snapshot network did in this run of the app, for Copy
/// diagnostics. The status refresher writes the diary and confirmer parts,
/// the snapshot keeper the producer part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkReport {
    /// The last diary step worth saying.
    pub diary: Option<String>,
    pub confirmer: Tally,
    /// Why this node does not confirm ([`why_not`]).
    pub confirmer_off: Option<String>,
    /// What this run already logged (see [`Tally::add`]).
    pub seen: HashSet<String>,
    /// The producer's last word.
    pub producer: Option<String>,
}

impl NetworkReport {
    /// The "Snapshots" section of Copy diagnostics. `diary_summary` is
    /// `diary::summary` of the datadir.
    pub fn lines(&self, diary_summary: Option<String>) -> Vec<String> {
        let mut out = vec![format!(
            "diary: {}",
            diary_summary.unwrap_or_else(|| "empty".into())
        )];
        if let Some(d) = &self.diary {
            out.push(format!("diary, last step: {d}"));
        }
        match &self.confirmer_off {
            Some(why) => out.push(format!("confirmer: off, {why}")),
            None if self.confirmer.rounds == 0 => {
                out.push("confirmer: no round yet in this run".into())
            }
            None => out.extend(self.confirmer.report()),
        }
        if let Some(p) = &self.producer {
            out.push(format!("producer: {p}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::Diary;
    use crate::fake_node::{synthetic_hash, FakeNode};
    use crate::operators::REGTEST_GENESIS;
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    /// The spike's statement at 100, its producer's key and its confirmer's.
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    /// 144 deep for the statement at 100.
    const DEEP: u64 = 243;
    const PENDING: &str = "/api/snapshots/pending?chain=regtest";
    const POST: &str = "/api/snapshots/statement";

    fn key(n: u8) -> SigningKey {
        SigningKey::from_slice(&[n; 32]).unwrap()
    }

    fn pubkey(sk: &SigningKey) -> [u8; 33] {
        sk.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap()
    }

    fn statement() -> Statement {
        cs::parse(R_P).unwrap().statement
    }

    fn entry(coins: u64) -> DiaryEntry {
        DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins,
            chain_tx: 101,
            recorded_at: 0,
        }
    }

    /// Our node: validating, `tip` high, key 7 in its engine, a diary entry
    /// at 100 with `coins` (101 is the statement's), and a regtest list on
    /// which key 7 is the operator "confirmer".
    fn setup_at(tip: u64, coins: u64) -> (FakeNode, tempfile::TempDir, String) {
        let n = FakeNode::new(REGTEST_GENESIS, tip);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
            s.signer = Some(key(7));
        });
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(entry(coins));
        crate::diary::save(dir.path(), &d).unwrap();
        let env = format!(
            "producer={P};confirmer={}",
            operators::hex(&pubkey(&key(7)))
        );
        (n, dir, env)
    }

    fn setup(coins: u64) -> (FakeNode, tempfile::TempDir, String) {
        setup_at(DEEP, coins)
    }

    /// `pending` as the website writes it, one record per manifest, its
    /// hash and dissent flag from the bytes unless `hash` overrides.
    fn pending_json(items: &[(&[u8], Option<&str>)]) -> String {
        let records: Vec<_> = items
            .iter()
            .map(|(m, hash)| {
                let st = cs::parse(m).unwrap().statement;
                json!({
                    "statement_hash": hash.map(str::to_string).unwrap_or(st.hash().display_hex()),
                    "height": st.height(),
                    "block_hash": st.block_hash().display_hex(),
                    "manifest_hex": operators::hex(m),
                    "signers": [], "operators": [], "file": "stored",
                    "first_seen": "2026-10-01T12:00:00.000Z",
                    "confirmed": false, "disputed": false,
                    "dissent": cs::is_dissent(&st),
                })
            })
            .collect();
        json!({"version": 1, "chain": "regtest", "statements": records}).to_string()
    }

    /// The manifest a node with `sk` should send for `st`: the statement and
    /// its one signature, as the engine makes it (RFC 6979, so the same).
    fn one_signature(st: &Statement, sk: &SigningKey) -> Vec<u8> {
        let sig: Signature = sk.sign_prehash(&st.hash().0).unwrap();
        Manifest {
            statement: st.clone(),
            signatures: vec![Signed {
                key: pubkey(sk),
                der: sig.to_der().as_bytes().to_vec(),
            }],
        }
        .to_bytes()
    }

    fn reply(hash: &str) -> String {
        format!(
            r#"{{"statement_hash":"{hash}","chain":"regtest","height":100,"signers":[],"operators":["producer","confirmer"],"added":1,"file":"stored"}}"#
        )
    }

    async fn listing(server: &mut mockito::ServerGuard, body: String) -> mockito::Mock {
        server
            .mock("GET", PENDING)
            .with_body(body)
            .create_async()
            .await
    }

    async fn no_post(server: &mut mockito::ServerGuard) -> mockito::Mock {
        server.mock("POST", POST).expect(0).create_async().await
    }

    async fn round_with(
        n: &FakeNode,
        dir: &Path,
        env: &str,
        our_key: [u8; 33],
        server: &mockito::ServerGuard,
    ) -> Result<Vec<Verdict>, NoRound> {
        let client = site::client().unwrap();
        let site = Site::parse(Some(&server.url())).unwrap();
        let work = dir.join("snapshots").join(WORK_DIR);
        let c = Confirmer {
            rpc: n,
            client: &client,
            site: &site,
            state_dir: dir,
            work_dir: &work,
            our_key,
            holds: Holds::none(),
            regtest_env: Some(env),
        };
        c.round(Chain::Regtest).await
    }

    async fn round(
        n: &FakeNode,
        dir: &Path,
        env: &str,
        server: &mockito::ServerGuard,
    ) -> Vec<Verdict> {
        round_with(n, dir, env, pubkey(&key(7)), server)
            .await
            .unwrap()
    }

    fn work_is_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir.join("snapshots").join(WORK_DIR))
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    }

    // ── Section 5: sign when the diary agrees ───────────────────────────────

    /// All four checks pass: the engine signs a copy, and exactly the
    /// statement with this node's one signature goes to the website, byte
    /// for byte. The copy is gone and the send is written down.
    #[tokio::test]
    async fn a_statement_the_diary_agrees_with_gets_this_nodes_one_signature() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = server
            .mock("POST", POST)
            .match_header("x-ebtx-node", "ebtx-snapshot-v1")
            .match_body(one_signature(&statement(), &key(7)))
            .with_body(reply(H))
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Signed {
                height: 100,
                statement_hash: H.into(),
                operators: vec!["producer".into(), "confirmer".into()]
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);
        assert!(work_is_empty(dir.path()), "the copy is gone");
        let log = load_log(dir.path());
        let e = log.signature("regtest", H).unwrap();
        assert!(e.unsent.is_none() && e.sent_at.is_some(), "{e:?}");
    }

    /// Section 3's gate, per round: while `getchainstates` shows an
    /// unvalidated chainstate (or does not answer), the website is not even
    /// asked and nothing is signed.
    #[tokio::test]
    async fn an_unvalidated_chainstate_stops_the_round_before_anything() {
        let (n, dir, env) = setup(101);
        n.with(|s| s.on_unchecked_snapshot(&synthetic_hash(50)));
        let mut server = mockito::Server::new_async().await;
        let get = server.mock("GET", PENDING).expect(0).create_async().await;
        let post = no_post(&mut server).await;
        let got = round_with(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        assert_eq!(got, Err(NoRound::Unvalidated));
        n.with(|s| {
            s.chainstates = None;
            s.silent.insert("getchainstates");
        });
        let got = round_with(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        assert_eq!(got, Err(NoRound::Unvalidated));
        get.assert_async().await;
        post.assert_async().await;
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// Check 1: under 144 deep it waits; nothing signed, nothing sent.
    #[tokio::test]
    async fn a_statement_under_144_deep_waits() {
        let (n, dir, env) = setup_at(DEEP - 1, 101);
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Waiting {
                height: 100,
                statement_hash: H.into(),
                why: Mismatch::TooShallow {
                    depth: 143,
                    need: 144
                }
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// Check 2: no diary entry for the block on the chain now: neither a
    /// signature nor a dissent.
    #[tokio::test]
    async fn without_a_diary_entry_nothing_is_signed_or_dissented() {
        let (n, dir, env) = setup(101);
        crate::diary::save(dir.path(), &Diary::new(REGTEST_GENESIS)).unwrap();
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Skipped {
                height: 100,
                statement_hash: H.into(),
                why: Mismatch::NoDiaryEntry { height: 100 }
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// Check 3: a node whose replay context is not the statement's says
    /// nothing either way.
    #[tokio::test]
    async fn a_node_with_another_replay_context_says_nothing() {
        let (n, dir, env) = setup(101);
        n.with(|s| s.replay_context = Some("33".repeat(32)));
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert!(
            matches!(
                &v[..],
                [Verdict::Skipped {
                    why: Mismatch::ReplayContext { .. },
                    ..
                }]
            ),
            "{v:?}"
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    // ── Section 6a: the dissent ─────────────────────────────────────────────

    /// Check 4 alone fails: the statement is never signed; a dissent built
    /// from the diary (its coin count, the statement's chain, the compiled
    /// commitment, all four file fields zero) is signed by the engine and
    /// sent, byte for byte.
    #[tokio::test]
    async fn a_diary_that_disagrees_sends_a_signed_dissent_and_never_signs() {
        let (n, dir, env) = setup(102);
        let rules = ChainRules::for_statement(&statement(), Some(&env)).unwrap();
        let dissent = dissent_for(&entry(102), &statement(), &rules).unwrap();
        assert!(cs::is_dissent(&dissent));
        assert_eq!(
            (
                dissent.height(),
                dissent.coins(),
                dissent.chain_tx(),
                dissent.block_hash().display_hex(),
                dissent.hash_serialized().display_hex()
            ),
            (100, 102, 101, R_BLOCK.to_string(), R_UTXO.to_string())
        );
        assert_eq!(
            (
                dissent.chain_id(),
                dissent.replay_context(),
                dissent.shielded()
            ),
            (
                statement().chain_id(),
                statement().replay_context(),
                statement().shielded()
            )
        );
        let dh = dissent.hash().display_hex();
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = server
            .mock("POST", POST)
            .match_body(one_signature(&dissent, &key(7)))
            .with_body(reply(&dh))
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Dissented {
                height: 100,
                statement_hash: H.into(),
                dissent_hash: dh.clone(),
                fields: vec![Field::Coins],
                operators: vec!["producer".into(), "confirmer".into()]
            }]
        );
        // The engine signed once, and only the dissent.
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);
        assert!(work_is_empty(dir.path()));
        let log = load_log(dir.path());
        assert!(log.signature("regtest", H).is_none(), "never signed");
        let e = log.dissent_at("regtest", 100).unwrap();
        assert_eq!(
            (e.statement_hash.as_str(), e.against.as_deref()),
            (dh.as_str(), Some(H))
        );
    }

    /// Once per height: a second round sends nothing more. A listed dissent
    /// at that height carrying this node's signature counts as sent, log or
    /// no log.
    #[tokio::test]
    async fn a_dissent_goes_once_per_height() {
        let (n, dir, env) = setup(102);
        let rules = ChainRules::for_statement(&statement(), Some(&env)).unwrap();
        let dissent = dissent_for(&entry(102), &statement(), &rules).unwrap();
        let dh = dissent.hash().display_hex();
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = server
            .mock("POST", POST)
            .with_body(reply(&dh))
            .expect(1)
            .create_async()
            .await;
        round(&n, dir.path(), &env, &server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::AlreadyDissented {
                height: 100,
                statement_hash: H.into()
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);

        // A fresh state folder, but the website lists our dissent.
        let (n, dir, env) = setup(102);
        let ours = one_signature(&dissent, &key(7));
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None), (&ours, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![
                Verdict::AlreadyDissented {
                    height: 100,
                    statement_hash: H.into()
                },
                Verdict::ListedDissent {
                    height: 100,
                    statement_hash: dh
                }
            ]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// A listed dissent is never signed, whoever sent it: it is skipped from
    /// its bytes before this node's checks run.
    #[tokio::test]
    async fn a_listed_dissent_is_never_signed() {
        let (n, dir, env) = setup(101);
        let rules = ChainRules::for_statement(&statement(), Some(&env)).unwrap();
        let theirs = dissent_for(&entry(999), &statement(), &rules).unwrap();
        // Key 9 is on no list: it verifies and counts for nobody.
        let m = one_signature(&theirs, &key(9));
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(&m, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::ListedDissent {
                height: 100,
                statement_hash: theirs.hash().display_hex()
            }]
        );
        assert_eq!(n.methods(), vec!["getchainstates"]);
    }

    // ── Never twice ─────────────────────────────────────────────────────────

    /// A statement already carrying this operator's signature is left alone:
    /// this very key, or another key of the same operator.
    #[tokio::test]
    async fn a_statement_this_operator_signed_is_left_alone() {
        let (n, dir, _) = setup(101);
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_PC, None)])).await;
        let post = no_post(&mut server).await;
        let env = format!("producer={P};confirmer={C}");
        let v = round_with(
            &n,
            dir.path(),
            &env,
            operators::parse_key(C).unwrap(),
            &server,
        )
        .await
        .unwrap();
        let already = vec![Verdict::AlreadySigned {
            height: 100,
            statement_hash: H.into(),
        }];
        assert_eq!(v, already);
        // Key 7 is the same operator's second key.
        let env = format!(
            "producer={P};confirmer={C},{}",
            operators::hex(&pubkey(&key(7)))
        );
        let v = round(&n, dir.path(), &env, &server).await;
        assert_eq!(v, already);
        post.assert_async().await;
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// The website still lists a statement without this node's signature
    /// (a stale read, a lost merge): the log says it went, so the engine is
    /// not asked again and nothing is sent again.
    #[tokio::test]
    async fn a_statement_is_signed_and_sent_once() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = server
            .mock("POST", POST)
            .with_body(reply(H))
            .expect(1)
            .create_async()
            .await;
        round(&n, dir.path(), &env, &server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::AlreadySigned {
                height: 100,
                statement_hash: H.into()
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);
    }

    /// A send that failed keeps its signature; the next round sends those
    /// same bytes, after the checks pass again, without a second signature.
    #[tokio::test]
    async fn a_failed_send_is_retried_with_the_same_signature() {
        let (n, dir, env) = setup(101);
        let body = one_signature(&statement(), &key(7));
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let down = server
            .mock("POST", POST)
            .with_status(503)
            .expect(1)
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, &server).await;
        down.assert_async().await;
        assert!(
            matches!(&v[..], [Verdict::Failed { error, .. }] if error.contains("503")),
            "{v:?}"
        );
        let kept = load_log(dir.path());
        assert_eq!(
            kept.signature("regtest", H).unwrap().unsent.as_deref(),
            Some(operators::hex(&body).as_str())
        );
        down.remove_async().await;
        let up = server
            .mock("POST", POST)
            .match_body(body)
            .with_body(reply(H))
            .expect(1)
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, &server).await;
        up.assert_async().await;
        assert!(matches!(&v[..], [Verdict::Signed { .. }]), "{v:?}");
        assert_eq!(n.count("signutxosnapshotmanifest"), 1, "signed once");
        assert_eq!(n.count("getblockcount"), 2, "checked again before sending");
    }

    // ── What the website says is not trusted ────────────────────────────────

    #[tokio::test]
    async fn a_listing_that_lies_or_a_bad_signature_is_refused_unasked() {
        let (n, dir, env) = setup(101);
        let mut bad = R_P.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let cases: [(Vec<u8>, Option<String>, &str); 2] = [
            (R_P.to_vec(), Some("00".repeat(32)), "not the statement"),
            (bad, None, "is not valid"),
        ];
        for (m, hash, want) in cases {
            let mut server = mockito::Server::new_async().await;
            listing(&mut server, pending_json(&[(&m, hash.as_deref())])).await;
            let post = no_post(&mut server).await;
            let v = round(&n, dir.path(), &env, &server).await;
            post.assert_async().await;
            assert!(
                matches!(&v[..], [Verdict::Unreadable { why, .. }] if why.contains(want)),
                "{want}: {v:?}"
            );
        }
        assert!(
            n.methods().iter().all(|m| m == "getchainstates"),
            "{:?}",
            n.methods()
        );
    }

    /// The engine signed, but not with this node's key: nothing is sent and
    /// nothing is written down as signed.
    #[tokio::test]
    async fn an_engine_that_signs_with_another_key_sends_nothing() {
        let (n, dir, env) = setup(101);
        n.with(|s| s.signer = Some(key(8)));
        let mut server = mockito::Server::new_async().await;
        listing(&mut server, pending_json(&[(R_P, None)])).await;
        let post = no_post(&mut server).await;
        let v = round(&n, dir.path(), &env, &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Failed {
                height: 100,
                statement_hash: H.into(),
                error: "the engine did not sign with this node's key".into()
            }]
        );
        assert!(load_log(dir.path()).entries.is_empty());
        assert!(work_is_empty(dir.path()));
    }

    // ── Counted ─────────────────────────────────────────────────────────────

    #[test]
    fn every_outcome_is_counted_and_said_once_per_run() {
        let skipped = Verdict::Skipped {
            height: 100,
            statement_hash: H.into(),
            why: Mismatch::NoDiaryEntry { height: 100 },
        };
        let already = Verdict::AlreadySigned {
            height: 100,
            statement_hash: H.into(),
        };
        let waiting = Verdict::Waiting {
            height: 200,
            statement_hash: "ab".repeat(32),
            why: Mismatch::TooShallow {
                depth: 10,
                need: 144,
            },
        };
        let mut t = Tally::default();
        let mut seen = HashSet::new();
        assert_eq!(
            t.add(&[skipped.clone(), waiting.clone()], &mut seen),
            vec![
                "did not sign the snapshot at 100: this node's diary has nothing at 100 \
                 for the block on its chain now"
                    .to_string(),
                "not yet for the snapshot at 200: the block is 10 blocks deep on this \
                 node's chain, 144 are needed"
                    .to_string()
            ]
        );
        assert!(t.add(&[skipped, waiting], &mut seen).is_empty());
        assert!(t.add(&[already], &mut seen).is_empty(), "quiet");
        assert_eq!(
            t.stopped(&NoRound::Unvalidated, &mut seen).as_deref(),
            Some(
                "confirmer round stopped: this node is still checking older history in \
                 the background, so it signs nothing"
            )
        );
        assert!(t.stopped(&NoRound::Unvalidated, &mut seen).is_none());
        assert_eq!(
            (t.rounds, t.stopped, t.skipped, t.waiting, t.already, t.signed),
            (5, 2, 1, 1, 1, 0)
        );
        let report = t.report();
        assert_eq!(
            report[0],
            "confirmer: 5 rounds (2 stopped), co-signed 0, dissented 0, waiting 1, skipped 1, \
             unreadable 0, failed 0, done before 1, dissents listed 0"
        );
        assert!(report[1].starts_with("confirmer, last refusal: did not sign"));
        assert!(report[2].starts_with("confirmer, last round: this node is still"));
        assert!(report.iter().all(|l| !l.contains('\u{2014}')));
    }

    #[test]
    fn the_diagnostics_lines_say_what_ran_and_what_did_not() {
        let mut r = NetworkReport::default();
        assert_eq!(
            r.lines(None),
            vec![
                "diary: empty".to_string(),
                "confirmer: no round yet in this run".to_string()
            ]
        );
        r.confirmer_off = Some("its key is not on the operator list".into());
        r.producer = Some("sent block 226200 to easybtx.com".into());
        r.diary = Some("wrote 226200".into());
        assert_eq!(
            r.lines(Some(
                "3 heights, newest 226200 (block 0123456789abcdef)".into()
            )),
            vec![
                "diary: 3 heights, newest 226200 (block 0123456789abcdef)".to_string(),
                "diary, last step: wrote 226200".to_string(),
                "confirmer: off, its key is not on the operator list".to_string(),
                "producer: sent block 226200 to easybtx.com".to_string(),
            ]
        );
    }

    #[test]
    fn only_a_validating_signing_listed_node_confirms() {
        let env = format!("confirmer={C}");
        let status = |mode: &str, signer: bool| crate::node_api::MatmulTrustedStatus {
            local_signer: signer,
            matmul_validation_mode: mode.into(),
            ..Default::default()
        };
        let ok = status("consensus", true);
        let mirror = status("trusted", true);
        let keyless = status("consensus", false);
        let run = |s, k: Option<&str>| why_not(s, k, REGTEST_GENESIS, Some(&env));
        assert_eq!(run(&ok, Some(C)), None);
        assert_eq!(
            run(&mirror, Some(C)),
            Some("it follows signatures instead of checking blocks")
        );
        assert_eq!(run(&keyless, Some(C)), Some("it does not sign"));
        assert_eq!(
            run(&ok, Some(P)),
            Some("its key is not on the operator list")
        );
        assert_eq!(run(&ok, None), Some("its signing key could not be read"));
        assert_eq!(
            run(&ok, Some("02zz")),
            Some("its signing key could not be read")
        );
        assert_eq!(
            why_not(&ok, Some(C), &"00".repeat(32), Some(&env)),
            Some("its chain has no snapshot network")
        );
        // Mainnet reads the compiled list, never the test one.
        assert_eq!(
            why_not(&ok, Some(C), operators::MAINNET_GENESIS, Some(&env)),
            Some("its key is not on the operator list")
        );
    }

    /// The log keeps one co-signature per statement and one dissent per
    /// height, per chain, and only the newest [`LOG_KEEP`].
    #[test]
    fn the_log_is_small_and_keyed_per_chain() {
        let e = |chain: &str, height: u64, kind: LogKind, hash: &str| LogEntry {
            chain: chain.into(),
            height,
            kind,
            statement_hash: hash.into(),
            against: None,
            unsent: None,
            signed_at: height,
            sent_at: Some(height),
        };
        let mut log = ConfirmerLog::default();
        for h in 1..=(LOG_KEEP as u64 + 50) {
            log.record(e("main", h * 100, LogKind::Signed, &format!("{h:064x}")));
        }
        assert_eq!(log.entries.len(), LOG_KEEP);
        assert_eq!(log.entries[0].height, 5_100);
        log.record(e("main", 30_000, LogKind::Dissent, "aa"));
        log.record(e("main", 30_000, LogKind::Dissent, "bb"));
        assert_eq!(log.dissent_at("main", 30_000).unwrap().statement_hash, "bb");
        assert!(log.dissent_at("regtest", 30_000).is_none());
        assert!(log.signature("regtest", &format!("{:064x}", 250)).is_none());
        assert!(log.signature("main", &format!("{:064x}", 250)).is_some());
        let dir = tempfile::tempdir().unwrap();
        save_log(dir.path(), &log).unwrap();
        assert_eq!(load_log(dir.path()), log);
        std::fs::write(log_path(dir.path()), b"{ broken").unwrap();
        assert_eq!(load_log(dir.path()), ConfirmerLog::default());
    }
}
