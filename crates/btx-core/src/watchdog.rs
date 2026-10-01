//! The trusted-mirror stall discriminator — "why is my tip not moving?" as a
//! pure, testable function.
//!
//! Grounded in the api.btxscan.io incident work (2026-08-14..17; PR
//! btxchain/btx#105 issuecomment-5309830791 + -5309870607, and Papers 1/3 of
//! that series). A post-#331 trusted mirror has exactly four ways to stop
//! advancing while every ordinary health signal stays green:
//!
//! - **A: body missing** — height frozen, headers ahead, no retryable-failure
//!   marker. The node is not being served block bodies (preferred-download
//!   starvation — the other face of the authority gate).
//! - **B: attestation missing** — the `retryable MatMul failure` marker is in
//!   the log: the body is banked and the signed confirmation is not.
//! - **C: no qualifying peer** — zero peers pass the authority gate (archive
//!   service bit AND manual-or-noban). Root cause of A and B; the ONE class
//!   with a cheap, proven remediation (attach an archive peer: 21 seconds from
//!   handshake to unstuck in production).
//! - **D: msghand spin** — the no-backoff retry loop burning a core while the
//!   tip is frozen (b-msghand measured at 99.5% of a core for 4+ hours). The
//!   node also degrades its own ability to fix A–C: saturated message handling
//!   completes no new handshakes.
//!
//! A fifth, added for 0.7.2 from an operator's report: **signatures arrive
//! and the node accepts none** (`PinsRejectEverySignature`). Its pinned keys
//! are out of date or have stopped signing, so the frontier it knows is its
//! own tip and the at-the-frontier guard used to keep it silent. Only the
//! engine's signature counters show it ([`SignatureWindow`]).
//!
//! HARD RULES, learned expensively: no verdict during header presync or
//! snapshot load (blocks==0 — loadtxoutset is legitimately hot with a frozen
//! height, and header pre-sync laps for the better part of an hour reporting
//! nothing); progress is never a high-water mark (EasyBTX already shipped a
//! detector that misread the pre-sync lap once); and the watchdog NEVER
//! restarts the node — a restart discards the peer set that is usually the
//! only attestation source, and unclean shutdowns have bricked snapshot
//! datadirs. Automation earns the boring jobs: dialling peers, reading logs,
//! counting, telling the truth on screen.
//!
//! THE PROGRESS RULE, refined (2026-08-17 review): while a connectable gap
//! exists (`headers > blocks` on an active chain), only BLOCK movement is
//! progress. BTX mints a header every ~90 s, so treating header arrival as
//! progress re-arms the 15-minute freeze window forever and the watchdog can
//! never fire on a live network — exactly while the mirror is starving. At
//! the frontier and during presync, ANY change still counts (pre-sync laps
//! and paused networks must never accumulate freeze). The freeze-window
//! bookkeeping lives with the caller; `discriminate` just receives
//! `frozen_secs` measured under that rule.

use std::borrow::Cow;
use std::collections::VecDeque;

use serde::Serialize;

/// The exact log marker that splits the world in two (Paper 1 §4.1): present
/// means the BODY is banked and the ATTESTATION is missing; absent with a
/// frozen tip means the body itself is missing.
pub const RETRYABLE_MARKER: &str = "retryable MatMul failure connecting";

/// Facts one refresher tick hands the discriminator. Everything here is a
/// measurement, not a guess — callers that cannot measure a field say so
/// (`None`) instead of defaulting it.
#[derive(Debug, Clone, PartialEq)]
pub struct StallFacts {
    /// Active-chain height and best-header height this tick.
    pub blocks: u64,
    pub headers: u64,
    /// How long the node has been failing to make progress, under the
    /// GAP-AWARE rule this module's header mandates: headers moving while
    /// blocks do not is a node falling behind, not a healthy node, so it does
    /// not reset the clock. The superseded any-change rule (reset whenever
    /// EITHER number moved) could not see that case at all, which is why the
    /// fixture below holds a 1000-block gap for 900 s on a chain minting a
    /// header every ~90 s and still expects a verdict.
    pub frozen_secs: u64,
    /// The retryable-failure marker was seen in the bounded log tail.
    pub retryable_marker: bool,
    /// Peers passing the trusted-mirror authority gate (manual/noban archive),
    /// `None` when getpeerinfo did not answer.
    pub archive_authority: Option<usize>,
    /// btxd's CPU as a percentage of ONE core over the last sample window,
    /// `None` where unmeasured (e.g. Windows in v1).
    pub cpu_pct_one_core: Option<u32>,
    /// True while the node runs as a trusted mirror (the discriminator is
    /// mirror-specific; strict-device stalls are a different, existing path).
    pub trusted_mirror: bool,
    /// How far the active tip trails the SIGNED frontier
    /// (`getmatmulattestedtip.signed_frontier.blocks_behind`), `None` when the
    /// node did not answer or predates the field.
    ///
    /// THE FACT THAT SEPARATES WAITING FROM STALLING. A frozen tip with a
    /// header above it looks the same in both cases, and they want opposite
    /// responses:
    ///   * `Some(0)` — we are AT the frontier. The attestor has signed nothing
    ///     newer, so there is nothing to fetch and nothing to fix. btxd still
    ///     logs `matmul trusted mirror stall` every minute; that is noise here.
    ///   * `Some(n > 0)` — signed work exists that we have not consumed. This
    ///     is our stall, and archive redial is the remedy.
    /// Measured live 2026-08-19: the network's attestor was offline ~100
    /// minutes while GPU consensus nodes ran 43 blocks ahead on unattested
    /// work. Every node app in the field classified that as a stall and
    /// redialled archives on a loop, because this fact did not exist.
    ///
    /// READ THE SIGN CAREFULLY. This is what the node KNOWS of the frontier,
    /// not what the network has actually signed, and a node whose attestation
    /// supply has been cut knows nothing newer *for the same reason a healthy
    /// waiting node does*. Zero therefore means "nothing to fetch" ONLY when
    /// there is independent evidence the node can still hear. See
    /// `discriminate`.
    pub frontier_lag: Option<i64>,
    /// Whether the signed frontier sits on our active chain. `Some(false)`
    /// means the frontier we can see is on a fork, so `frontier_lag` is a fork
    /// artefact and must never be read as "nothing to fetch".
    pub frontier_on_active_chain: Option<bool>,
    /// Blocks in flight across every peer this tick, `None` when unmeasured.
    ///
    /// THE FACT THAT SEPARATES "NOBODY WILL SERVE ME" FROM "I AM ASKING NOBODY".
    /// Both look identical from the height fields alone: headers above the tip,
    /// nothing connecting. They need opposite responses.
    ///   * `Some(n > 0)` — requests are outstanding; the node is trying and the
    ///     peers are slow or absent. Redialling archives is the right remedy.
    ///   * `Some(0)` with headers above the tip — the node knows the blocks
    ///     exist and is requesting NONE of them. The block scheduler's gate has
    ///     stopped asking (upstream btxchain/btx#112). Redialling cannot fix
    ///     this and never will: peer availability was never the input.
    /// Measured on api.btxscan.io 2026-08-20: 75 minutes wedged at 195,422 with
    /// 51 headers above it, 7 peers through the authority gate, 11 archives,
    /// and `in_flight=0`. Our watchdog redialled six archives four times and
    /// the lag grew 21 -> 48. Asking named peers for named blocks with
    /// `getblockfrompeer` moved the tip in 20 seconds and recovered all 51.
    pub blocks_in_flight: Option<usize>,
    /// What the engine did with the signatures that reached it over the
    /// last [`SignatureWindow`], `None` until the window spans two samples.
    ///
    /// THE FACT THAT SEPARATES "NOTHING ARRIVES" FROM "NOTHING IS ACCEPTED".
    /// A node whose pinned keys have all stopped signing, or whose list is out
    /// of date, hears signatures and refuses every one. It never gets a
    /// quorum, so its known frontier is its own tip and `frontier_lag` reads
    /// 0: the same reading as a node waiting on a quiet attestor. Reported
    /// 2026-09-30: two old keys pinned, 0 accepted, 5,750 rejected, about 230
    /// blocks behind, and nothing told the operator. See `discriminate`.
    pub signatures: Option<SignatureEvidence>,
}

/// The signature counters' movement over a span, plus what the shell knows
/// about why every signature might be refused. Built by the caller from a
/// [`SignatureWindow`] and its own settings; nothing here is guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureEvidence {
    pub accepted_delta: u64,
    pub rejected_delta: u64,
    pub span_secs: u64,
    /// This app's own shipped keys (`node::BTX_TRUSTED_ATTESTATION_PUBKEYS`)
    /// that the engine's LIVE pin does not hold. 0 when it holds them all,
    /// and 0 when the engine did not list its pin (an older engine).
    pub shipped_keys_missing: usize,
    /// When some are missing: the file in the node folder the app can see the
    /// cause in (the app left that key to the file, and the engine does not
    /// hold it), `None` when no file it reads explains it.
    pub missing_explained_by: Option<&'static str>,
    /// The last update check found a newer easyNode (`found`, or
    /// `install-failed` after one), so this build's key list is probably the
    /// stale part. Without that the app cannot know its own list is stale.
    pub update_known: bool,
}

/// How many samples a [`SignatureWindow`] keeps. The refresher samples every
/// 30 s, so 45 samples span 22 minutes: past [`FROZEN_VERDICT_SECS`] with
/// room to spare, and short enough that the rule speaks about now.
pub const SIGNATURE_WINDOW_SAMPLES: usize = 45;

/// Fewest rejected signatures across the span, with none accepted, before the
/// rule may speak. BTX mints about one block every 90 s, so a 15-minute span
/// holds about ten blocks, and every live signer's statement for each of them
/// arrives at least once from every peer that relays it. A node that refuses
/// them all counts tens per block (the report: 5,750 over about 230 blocks,
/// 25 a block). 20 is two per block over ten blocks: a handful of stray
/// relays (a peer re-sending its store after a reconnect, one late signature
/// for an old height) cannot reach it, while the reported case clears it by
/// more than ten times over the shortest span the rule reads.
pub const REJECTED_FLOOR: u64 = 20;

/// The engine's signature counters over the last few minutes, as deltas.
///
/// `accepted` and `rejected` (`getmatmultrustedstatus`) live in the engine's
/// memory: they start again at every engine start (`accepted` from the
/// archive it loads, not from 0) and only rise. So a single reading means
/// nothing; the movement across a span does. A counter that goes DOWN is an
/// engine restart, and nothing before it compares with anything after.
///
/// Pure: the caller passes the time and the engine generation, and samples it
/// on the refresher's slow tick from the status it already fetched.
#[derive(Debug, Clone, Default)]
pub struct SignatureWindow {
    generation: u64,
    /// (seconds on the caller's monotonic clock, accepted, rejected).
    samples: VecDeque<(u64, u64, u64)>,
}

/// What a [`SignatureWindow`] holds, first sample to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureDeltas {
    pub accepted: u64,
    pub rejected: u64,
    pub span_secs: u64,
}

impl SignatureWindow {
    /// Add one reading. A new `generation` (another engine run), a counter
    /// that went down or a clock that went back starts the window over.
    pub fn push(&mut self, generation: u64, at_secs: u64, accepted: u64, rejected: u64) {
        let restarted = self.generation != generation
            || self
                .samples
                .back()
                .is_some_and(|&(t, a, r)| at_secs < t || accepted < a || rejected < r);
        if restarted {
            self.samples.clear();
            self.generation = generation;
        }
        self.samples.push_back((at_secs, accepted, rejected));
        while self.samples.len() > SIGNATURE_WINDOW_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// The movement from the oldest sample held to the newest, `None` until
    /// there are two.
    pub fn deltas(&self) -> Option<SignatureDeltas> {
        let (&(t0, a0, r0), &(t1, a1, r1)) = (self.samples.front()?, self.samples.back()?);
        (self.samples.len() >= 2).then_some(SignatureDeltas {
            accepted: a1 - a0,
            rejected: r1 - r0,
            span_secs: t1 - t0,
        })
    }
}

/// How long the tip must be frozen before we classify at all. Catch-up on a
/// healthy mirror was measured at 22–31 blocks/min but single blocks can
/// legitimately take minutes; a 15-minute freeze is far outside normal and
/// still early enough to beat the user to the question.
pub const FROZEN_VERDICT_SECS: u64 = 15 * 60;

/// The spin signature's CPU floor (percent of one core, sustained).
pub const SPIN_CPU_PCT: u32 = 85;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StallClass {
    BodyMissing,
    /// Bodies are available and the node is asking for none of them: the
    /// scheduler's gate, not the peer set. Distinct from `BodyMissing` because
    /// the remedy is disjoint — nudging beats redialling, and redialling is a
    /// guaranteed no-op.
    BlockFetchGated,
    /// The catch-up help concluded that no archive peer connected serves old
    /// blocks to this node (`crate::catchup_assist::CatchUp::
    /// no_archive_serves_old_blocks`). Not issued by [`discriminate`]: the
    /// shell shows [`old_blocks_refused_verdict`] while that holds.
    OldBlocksRefused,
    AttestationMissing,
    NoQualifyingPeer,
    MsghandSpin,
    /// Signatures reach the node and it accepts none of them: every one comes
    /// from a key outside its pin. Either this build's key list is out of
    /// date or the keys it trusts have stopped signing; the counters cannot
    /// tell which, so the copy says what is true of both. Redialling peers
    /// cannot fix it, and the shell does not.
    PinsRejectEverySignature,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StallVerdict {
    pub class: StallClass,
    /// One plain-language sentence for the UI/log — what is wrong, in terms a
    /// person who has never read net_processing.cpp can act on.
    pub summary: Cow<'static, str>,
}

/// Classify a frozen trusted mirror. `None` = no verdict (healthy, not frozen
/// long enough, not a mirror, or in a phase where verdicts are forbidden).
pub fn discriminate(f: &StallFacts) -> Option<StallVerdict> {
    if !f.trusted_mirror {
        return None;
    }
    // Presync / snapshot-load guard: blocks==0 means the node has no active
    // chain yet — loadtxoutset and header sync are legitimately hot + frozen.
    if f.blocks == 0 {
        return None;
    }
    if f.frozen_secs < FROZEN_VERDICT_SECS {
        return None;
    }
    // SIGNATURES ARRIVE AND NONE IS ACCEPTED. Ahead of the frontier guard
    // below, because this is exactly the node that guard cannot see: with no
    // accepted signature it has no quorum, so the frontier it knows is its own
    // tip and the lag reads 0 while an authority peer is connected. Ahead of
    // C, B and A too, because rising rejections PROVE supply: someone is
    // handing this node signatures, it is refusing them, and neither a redial
    // nor a retry changes which keys it accepts.
    //
    // Every condition is required. Over a span at least as long as the
    // freeze verdict: not one signature accepted (one is proof a pinned key
    // still reaches the node), and at least REJECTED_FLOOR rejected
    // (rejections alone are normal: every signature from a key outside the
    // pin counts, and the healthy RTX 3060 signer showed 1,735 next to 3,073
    // accepted). No evidence never fires.
    if let Some(s) = &f.signatures {
        if s.span_secs >= FROZEN_VERDICT_SECS
            && s.accepted_delta == 0
            && s.rejected_delta >= REJECTED_FLOOR
        {
            return Some(pins_reject_every_signature_verdict(s));
        }
    }
    // AT THE SIGNED FRONTIER, AND ABLE TO HEAR: the attestor has signed nothing
    // newer, so a frozen tip is the network waiting, not this node failing.
    //
    // THE TRAP THIS GUARD MUST NOT FALL INTO (found in review, 2026-08-19, after
    // an earlier version of it shipped): `blocks_behind` is what this node KNOWS
    // of the frontier. A node that has lost every authority peer hears no
    // attestations at all, so its known frontier is its own tip and it reports
    // exactly 0 — the same reading as a healthy node waiting on a quiet attestor.
    // Suppressing on that value alone silences class C, which is the one class
    // with an automated remedy, using the very channel the failure has cut. The
    // same applies to class B: a node holding a banked body it cannot get an
    // attestation for cannot know the next height is signed, so it too reads 0.
    //
    // So suppress only with POSITIVE evidence that the silence is the network's
    // and not this node's:
    //   * a live authority peer exists (someone would have told us), and
    //   * no banked-body marker (we are not sitting on an unattested block), and
    //   * the frontier is not known to be on a fork.
    // A negative lag (ahead of the frontier) has even less to fetch than zero,
    // so it suppresses on the same evidence. An unmeasured frontier (`None`)
    // never suppresses: it degrades to the height-only rules below.
    if matches!(f.frontier_lag, Some(n) if n <= 0)
        && matches!(f.archive_authority, Some(n) if n > 0)
        && !f.retryable_marker
        && f.frontier_on_active_chain != Some(false)
    {
        return None;
    }

    // Frozen at the frontier is normally not a stall — nothing to connect.
    // But TOTAL ISOLATION looks exactly like this too: with no authority
    // peer the mirror also stops HEARING about new work, so headers freeze
    // alongside blocks and an unconditional early return leaves the watchdog
    // blind to the one class it can actually fix. Classify C when the census
    // affirmatively says zero qualifying peers; an unknown census (None) or a
    // healthy one at a paused network stays verdict-free.
    if f.headers <= f.blocks {
        if f.archive_authority == Some(0) {
            return Some(no_qualifying_peer_verdict());
        }
        return None;
    }

    // D first: the spin degrades everything else, including the remediations.
    if let Some(cpu) = f.cpu_pct_one_core {
        if cpu >= SPIN_CPU_PCT {
            return Some(StallVerdict {
                class: StallClass::MsghandSpin,
                summary: Cow::Borrowed(
                    "the node is stuck retrying one block flat-out (a known upstream bug); \
                          it may also be too busy to accept the peer that would fix it — do NOT \
                          restart, keep the archive peers dialling",
                ),
            });
        }
    }
    // C: root cause of A and B, and the one with a proven cheap fix.
    if f.archive_authority == Some(0) {
        return Some(no_qualifying_peer_verdict());
    }
    // B: body banked, confirmation missing.
    if f.retryable_marker {
        return Some(StallVerdict {
            class: StallClass::AttestationMissing,
            summary: Cow::Borrowed(
                "the next block is downloaded but its signed confirmation has not arrived; \
                      the node needs a working archive peer and will retry on its own",
            ),
        });
    }
    // A2, BEFORE A: is this node even asking? A qualifying peer set plus zero
    // blocks in flight is not "no one will serve me", it is "I am requesting
    // nothing". Ordered ahead of A because the height fields are identical in
    // both and A's remedy is provably useless here. Only an affirmative zero
    // qualifies; `None` (unmeasured) falls through to A as before.
    if f.blocks_in_flight == Some(0) {
        return Some(StallVerdict {
            class: StallClass::BlockFetchGated,
            summary: Cow::Borrowed(
                "this node can see the next blocks and is not asking any peer for them \
                      (a known upstream scheduler bug), so adding or redialling peers will not \
                      help. While it is 20 or more blocks behind, this app asks its own \
                      archive peers for the next blocks by name; closer than that, Tools > \
                      Fetch a stuck block asks for them once. If the tip still does not move, \
                      those peers may not be connected, or may not serve old blocks to this \
                      node. Copy diagnostics in Tools shows what the app tried",
            ),
        });
    }
    // A: body missing. Requests ARE outstanding (or we could not measure), so
    // the peer set genuinely is the suspect and redialling is worth doing.
    Some(StallVerdict {
        class: StallClass::BodyMissing,
        summary: Cow::Borrowed(
            "this node has asked for the next blocks and no peer has served them yet; \
                  this is peer selection, not corruption — archive/noban peers usually fix it",
        ),
    })
}

/// The status card's sentence while the catch-up help concludes that no
/// archive peer serves old blocks to this node (the owner's decision 1, the
/// night of 2026-09-29). On any node, not only mirrors, and without waiting
/// for a freeze: the engine's own rescue still connects a block every few
/// minutes in that state.
pub fn old_blocks_refused_verdict() -> StallVerdict {
    StallVerdict {
        class: StallClass::OldBlocksRefused,
        summary: Cow::Borrowed(
            "this node is far behind and none of the archive peers connected now serves \
                  old blocks to it: each dropped the connection twice when asked for them. \
                  This app has stopped asking them, and the node keeps asking on its own, \
                  which is slow. If Tools offers Fast-forward, it can take the node closer \
                  to the tip; Copy diagnostics in Tools names the peers",
        ),
    }
}

/// The datadir's own settings file, which the engine loads on every start.
pub const RW_CONF: &str = "btx_rw.conf";
/// The conf this app starts the engine with, relative to the node folder.
pub const FASTSTART_CONF: &str = "faststart/faststart.conf";

/// The keys this app ships (`node::BTX_TRUSTED_ATTESTATION_PUBKEYS`) that the
/// engine's live pin (`trusted_signer_pubkeys`) does not hold. Empty when the
/// pin is empty: an engine that does not list it has told us nothing, and a
/// mirror with no pin at all does not start.
pub fn shipped_keys_missing(live_pin: &[String]) -> Vec<&'static str> {
    if live_pin.is_empty() {
        return Vec::new();
    }
    crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS
        .into_iter()
        .filter(|k| !live_pin.iter().any(|p| p.eq_ignore_ascii_case(k)))
        .collect()
}

/// Which file in the node folder explains a shipped key the engine does not
/// hold, from the app's own reading of them (`node::rw_conf_pins`,
/// `node::conf_pins`). The launch leaves a key to a file that already pins it
/// rather than repeat it on the command line, so a key the app reads in a
/// file and the engine does not hold was dropped by the engine's reading of
/// that file. `None` when the app put every missing key on the command line
/// itself (for example, the engine still runs from before an update).
pub fn where_missing_keys_are_set(
    missing: &[&str],
    rw_conf_pins: &[String],
    conf_pins: &[String],
) -> Option<&'static str> {
    let in_file = |pins: &[String]| {
        missing
            .iter()
            .any(|k| pins.iter().any(|p| p.eq_ignore_ascii_case(k)))
    };
    if in_file(rw_conf_pins) {
        Some(RW_CONF)
    } else if in_file(conf_pins) {
        Some(FASTSTART_CONF)
    } else {
        None
    }
}

/// The sentence for [`StallClass::PinsRejectEverySignature`]. The counters
/// cannot tell an out-of-date key list from trusted keys that stopped
/// signing, so the copy is chosen by what the shell does know: whether an
/// update is waiting, and whether the engine dropped any key this app ships.
fn pins_reject_every_signature_verdict(s: &SignatureEvidence) -> StallVerdict {
    let mut summary = String::from(if s.update_known {
        "signatures are reaching this node, but it does not accept any of them, so it \
         cannot move. Its list of signer keys is probably out of date. Updating easyNode \
         brings the current list"
    } else {
        "signatures are reaching this node, but none come from a key it trusts, so it \
         cannot move. Updating easyNode brings the current key list; if it is already up \
         to date, the keys it trusts may have stopped signing for now"
    });
    if s.shipped_keys_missing > 0 {
        // Read-only: the app never edits these files on its own.
        match s.missing_explained_by {
            Some(file) => summary.push_str(&format!(
                ". The node is not using every signer key this app ships, and {file} in \
                 the node folder is where that is set. Tools > Copy diagnostics lists the \
                 missing keys"
            )),
            None => summary.push_str(
                ". The node is not using every signer key this app ships. Tools > Copy \
                 diagnostics lists the missing keys",
            ),
        }
    }
    StallVerdict {
        class: StallClass::PinsRejectEverySignature,
        summary: Cow::Owned(summary),
    }
}

/// The class-C verdict — issued both on a frozen gap and on a frontier
/// freeze with an affirmatively empty authority census (total isolation).
fn no_qualifying_peer_verdict() -> StallVerdict {
    StallVerdict {
        class: StallClass::NoQualifyingPeer,
        summary: Cow::Borrowed(
            "no connected peer is allowed to hand this node signed confirmations — \
                  redialling the known archive peers (an RPC addnode counts as manual and \
                  passes the gate with no restart)",
        ),
    }
}

/// Does a bounded debug.log tail carry the class-B marker?
pub fn log_tail_has_retryable_marker(tail: &str) -> bool {
    tail.contains(RETRYABLE_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> StallFacts {
        StallFacts {
            blocks: 190_500,
            headers: 191_500,
            frozen_secs: FROZEN_VERDICT_SECS,
            retryable_marker: false,
            archive_authority: Some(3),
            cpu_pct_one_core: Some(5),
            trusted_mirror: true,
            frontier_lag: Some(1000),
            frontier_on_active_chain: Some(true),
            // Requests ARE outstanding by default, so the existing cases keep
            // landing on the classes they were written to pin.
            blocks_in_flight: Some(4),
            signatures: None,
        }
    }

    #[test]
    fn healthy_and_guarded_states_get_no_verdict() {
        // Not a mirror.
        assert_eq!(
            discriminate(&StallFacts {
                trusted_mirror: false,
                ..base()
            }),
            None
        );
        // Presync / snapshot load (blocks==0) — the guard that keeps the
        // watchdog from shooting a node mid-loadtxoutset.
        assert_eq!(
            discriminate(&StallFacts {
                blocks: 0,
                ..base()
            }),
            None
        );
        // Not frozen long enough.
        assert_eq!(
            discriminate(&StallFacts {
                frozen_secs: FROZEN_VERDICT_SECS - 1,
                ..base()
            }),
            None
        );
        // At the frontier with a healthy census: nothing to connect, freeze
        // is normal block cadence (or a paused network).
        assert_eq!(
            discriminate(&StallFacts {
                headers: 190_500,
                ..base()
            }),
            None
        );
    }

    /// The 2026-08-19 false positive: the attestor goes quiet, a header appears
    /// above our tip, the tip sits still for hours, and the node is otherwise
    /// healthy. That must stay silent.
    #[test]
    fn at_the_frontier_and_hearing_the_network_is_waiting_not_stalling() {
        let f = StallFacts {
            blocks: 194_160,
            headers: 194_161,           // an unattested header above us
            frozen_secs: 6_000,         // 100 minutes
            frontier_lag: Some(0),      // nothing newer signed...
            archive_authority: Some(3), // ...and peers who would have told us
            retryable_marker: false,
            ..base()
        };
        assert_eq!(
            discriminate(&f),
            None,
            "at the frontier there is nothing to fetch"
        );
        // Ahead of the frontier has even less to fetch.
        assert_eq!(
            discriminate(&StallFacts {
                frontier_lag: Some(-3),
                ..f.clone()
            }),
            None
        );
    }

    /// THE REGRESSION THIS FILE EXISTS FOR. An isolated node reports
    /// `blocks_behind == 0` for the same reason a healthy one does: it hears
    /// nothing. Suppressing on that value alone silenced class C, the only
    /// class with an automated remedy, and did it using the very channel the
    /// failure had cut. Shipped once (#368) and caught in review.
    #[test]
    fn zero_lag_never_silences_a_node_that_cannot_hear() {
        // Total isolation: zero authority peers, and the frontier reads 0
        // precisely BECAUSE we are cut off.
        let isolated = StallFacts {
            headers: 190_500, // == blocks: no gap either, the isolation signature
            archive_authority: Some(0),
            frontier_lag: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&isolated).unwrap().class,
            StallClass::NoQualifyingPeer,
            "a node with no authority peers must classify, not be suppressed by its own blindness"
        );
        // Class B: a banked body whose attestation never arrived also cannot
        // know the next height is signed, so it too reports 0.
        let banked = StallFacts {
            retryable_marker: true,
            frontier_lag: Some(0),
            archive_authority: Some(2),
            ..base()
        };
        assert_eq!(
            discriminate(&banked).unwrap().class,
            StallClass::AttestationMissing
        );
        // Class D outranks both and must survive the guard as well.
        let spinning = StallFacts {
            cpu_pct_one_core: Some(99),
            frontier_lag: Some(0),
            archive_authority: Some(2),
            retryable_marker: true,
            ..base()
        };
        assert_eq!(
            discriminate(&spinning).unwrap().class,
            StallClass::MsghandSpin
        );
        // A frontier known to be on a fork is not evidence of anything.
        let forked = StallFacts {
            frontier_lag: Some(0),
            frontier_on_active_chain: Some(false),
            archive_authority: Some(3),
            ..base()
        };
        assert_eq!(
            discriminate(&forked).unwrap().class,
            StallClass::BodyMissing
        );
    }

    /// An unmeasured frontier must change nothing: older engines and failed
    /// RPCs fall back to the height-only rules rather than going silent.
    #[test]
    fn an_unmeasured_frontier_degrades_to_the_old_behaviour() {
        let f = StallFacts {
            frontier_lag: None,
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
        let f = StallFacts {
            frontier_lag: None,
            archive_authority: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::NoQualifyingPeer
        );
    }

    /// Total isolation: at the frontier the node stops hearing about new
    /// work, so headers freeze WITH blocks. An affirmatively empty authority
    /// census must still classify C (the remediable class); an unknown
    /// census must not.
    #[test]
    fn frontier_freeze_with_zero_authority_is_class_c_not_blindness() {
        let f = StallFacts {
            headers: 190_500, // == blocks: frontier
            archive_authority: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::NoQualifyingPeer
        );
        // Unknown census at the frontier: not proof of isolation — no verdict.
        let f = StallFacts {
            headers: 190_500,
            archive_authority: None,
            ..base()
        };
        assert_eq!(discriminate(&f), None);
    }

    #[test]
    fn class_c_no_qualifying_peer_beats_class_b_marker() {
        // C is the root cause: with zero authority peers, the retryable marker
        // is a symptom, and the remediation (dial archives) targets C.
        let f = StallFacts {
            archive_authority: Some(0),
            retryable_marker: true,
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::NoQualifyingPeer
        );
    }

    #[test]
    fn class_b_attestation_missing_on_marker_with_authority_present() {
        let f = StallFacts {
            retryable_marker: true,
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::AttestationMissing
        );
    }

    #[test]
    fn class_a_body_missing_is_the_quiet_default() {
        assert_eq!(
            discriminate(&base()).unwrap().class,
            StallClass::BodyMissing
        );
    }

    #[test]
    fn class_d_spin_wins_over_everything() {
        let f = StallFacts {
            cpu_pct_one_core: Some(99),
            archive_authority: Some(0),
            retryable_marker: true,
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::MsghandSpin);
    }

    #[test]
    fn unknown_cpu_or_peers_degrade_gracefully() {
        // No CPU sample: spin undetectable, falls through to the peer facts.
        let f = StallFacts {
            cpu_pct_one_core: None,
            archive_authority: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::NoQualifyingPeer
        );
        // No peer info either: an unknown census is NOT "zero peers" — class B/A
        // still classify from the log marker alone.
        let f = StallFacts {
            cpu_pct_one_core: None,
            archive_authority: None,
            retryable_marker: true,
            ..base()
        };
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::AttestationMissing
        );
    }

    #[test]
    fn marker_helper_matches_the_live_log_line() {
        // Verbatim shape from the incident logs.
        let line = "2026-08-16T20:15:01Z ActivateBestChainStep: retryable MatMul failure connecting 3ab1… (leaving candidate)";
        assert!(log_tail_has_retryable_marker(line));
        assert!(!log_tail_has_retryable_marker(
            "UpdateTip: new best=… height=1"
        ));
    }

    #[test]
    fn zero_in_flight_with_headers_ahead_is_the_scheduler_gate_not_the_peer_set() {
        // The api.btxscan.io wedge of 2026-08-20 in facts: a HEALTHY peer set
        // (7 through the authority gate), headers well above the tip, and the
        // node asking nobody for anything.
        let f = StallFacts {
            blocks: 195_422,
            headers: 195_474,
            archive_authority: Some(7),
            frontier_lag: Some(51),
            blocks_in_flight: Some(0),
            ..base()
        };
        let v = discriminate(&f).expect("a wedged mirror must get a verdict");
        assert_eq!(v.class, StallClass::BlockFetchGated);
        // The operator-facing half of the lesson: this must not send anyone
        // back to the peer list, because that is where 75 minutes went.
        assert!(v.summary.contains("will not help"));
        assert!(!v.summary.contains("NOT"), "no shouting");
        // It names the help that now exists (crate::catchup_assist), and only
        // what that help does: it asks, and peers may not answer. The copy once
        // ended "which the guardian does automatically" while nothing did.
        assert!(v.summary.contains("asks its own archive peers"));
        assert!(!v.summary.contains("nothing in this app"));
        assert!(
            !v.summary.contains("automatically") && !v.summary.contains("will fix"),
            "say what the app does, never promise the outcome"
        );
        assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
        assert!(v
            .summary
            .contains(&crate::catchup_assist::MIN_BEHIND.to_string()));
        // Closer than that, the Tools button on main does it once; and when
        // the tip still does not move, the report says what was tried.
        assert!(v.summary.contains("Fetch a stuck block"));
        assert!(v.summary.contains("Copy diagnostics"));
    }

    #[test]
    fn the_catch_up_help_s_conclusion_has_its_own_sentence() {
        let v = old_blocks_refused_verdict();
        assert_eq!(v.class, StallClass::OldBlocksRefused);
        assert_eq!(serde_json::to_value(v.class).unwrap(), "old_blocks_refused");
        for part in [
            "none of the archive peers",
            "twice",
            "Fast-forward",
            // Offered only with a confirmed snapshot above the node, so the
            // sentence says "if", never that it is there.
            "If Tools offers Fast-forward",
            "Copy diagnostics",
        ] {
            assert!(v.summary.contains(part), "{part}");
        }
        assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
        assert!(
            !v.summary.contains("automatically") && !v.summary.contains("will fix"),
            "say what the app does, never promise the outcome"
        );
    }

    #[test]
    fn outstanding_requests_stay_body_missing_so_redial_is_still_offered() {
        // Same shape, one request in flight. The node IS asking, so the peers
        // are the suspect again and the old remedy is the right one.
        let f = StallFacts {
            blocks: 195_422,
            headers: 195_474,
            archive_authority: Some(7),
            frontier_lag: Some(51),
            blocks_in_flight: Some(1),
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
    }

    #[test]
    fn unmeasured_in_flight_degrades_to_the_old_behaviour() {
        // An old node app, or a getpeerinfo that did not answer, must not be
        // told the scheduler is gated on the strength of a missing number.
        let f = StallFacts {
            blocks_in_flight: None,
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
    }

    #[test]
    fn the_gate_never_outranks_isolation_or_a_banked_body() {
        // Zero in flight is a SYMPTOM of class C too: a node nobody will serve
        // ends up asking for nothing. C keeps priority, because C has the
        // cheaper and more certain fix.
        let isolated = StallFacts {
            archive_authority: Some(0),
            blocks_in_flight: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&isolated).unwrap().class,
            StallClass::NoQualifyingPeer
        );
        // And a banked body waiting on its signature is class B regardless.
        let banked = StallFacts {
            retryable_marker: true,
            blocks_in_flight: Some(0),
            ..base()
        };
        assert_eq!(
            discriminate(&banked).unwrap().class,
            StallClass::AttestationMissing
        );
    }

    /// A window sampled every 30 s (the refresher's slow tick) for `n`
    /// samples, accepted and rejected each moving by a fixed step per sample.
    fn window(n: u64, accepted: (u64, u64), rejected: (u64, u64)) -> SignatureWindow {
        let mut w = SignatureWindow::default();
        for i in 0..n {
            w.push(
                1,
                i * 30,
                accepted.0 + i * accepted.1,
                rejected.0 + i * rejected.1,
            );
        }
        w
    }

    fn evidence(w: &SignatureWindow) -> SignatureEvidence {
        let d = w.deltas().expect("two samples or more");
        SignatureEvidence {
            accepted_delta: d.accepted,
            rejected_delta: d.rejected,
            span_secs: d.span_secs,
            shipped_keys_missing: 0,
            missing_explained_by: None,
            update_known: false,
        }
    }

    /// The operator's report in facts: two old keys pinned, 0 signatures
    /// accepted, 5,750 rejected, about 230 blocks behind, and nothing said
    /// so. Accepted sits flat at what the archive loaded at start.
    fn the_reported_mirror() -> StallFacts {
        // 31 samples, 30 s apart: 900 s, rejected +5,750 across it.
        let mut w = SignatureWindow::default();
        for i in 0..31u64 {
            w.push(1, i * 30, 412, 1_000 + (i * 5_750) / 30);
        }
        StallFacts {
            blocks: 228_900,
            headers: 229_130,
            frozen_secs: FROZEN_VERDICT_SECS,
            signatures: Some(evidence(&w)),
            ..base()
        }
    }

    #[test]
    fn the_reported_case_is_named_even_on_the_silent_path() {
        let f = the_reported_mirror();
        let ev = f.signatures.as_ref().unwrap();
        assert_eq!((ev.accepted_delta, ev.rejected_delta), (0, 5_750));
        assert_eq!(ev.span_secs, FROZEN_VERDICT_SECS);
        assert_eq!(
            discriminate(&f).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
        // THE SILENT PATH: with no accepted signature the node's known
        // frontier is its own tip, so the lag reads 0 while an authority peer
        // is connected. That guard used to return None here.
        let silent = StallFacts {
            frontier_lag: Some(0),
            archive_authority: Some(3),
            retryable_marker: false,
            ..f.clone()
        };
        assert_eq!(
            discriminate(&silent).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
        // At the frontier too (headers frozen with blocks): the other early
        // return must not swallow it either.
        let level = StallFacts {
            headers: f.blocks,
            frontier_lag: Some(0),
            ..f.clone()
        };
        assert_eq!(
            discriminate(&level).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
        // With the banked-body marker it would have said "will retry on its
        // own", which is not true of a key mismatch.
        let banked = StallFacts {
            retryable_marker: true,
            frontier_lag: Some(0),
            ..f.clone()
        };
        assert_eq!(
            discriminate(&banked).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
        // Rising rejections prove supply, so it outranks "nobody may hand
        // this node signatures" as well.
        let isolated = StallFacts {
            archive_authority: Some(0),
            ..f
        };
        assert_eq!(
            discriminate(&isolated).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
    }

    /// The RTX 3060 signer, healthy (docs/gpu-qualification-rtx3060.md):
    /// accepted 3008 -> 3073 while rejected 1700 -> 1735. Rejections on a
    /// node that accepts anything at all are other keys' signatures, normal.
    #[test]
    fn the_3060_s_real_numbers_never_fire() {
        // 31 samples: accepted +65 and rejected +35 over 900 s, spread out.
        let mut w = SignatureWindow::default();
        for i in 0..31u64 {
            w.push(1, i * 30, 3_008 + (i * 65) / 30, 1_700 + (i * 35) / 30);
        }
        let ev = evidence(&w);
        assert_eq!((ev.accepted_delta, ev.rejected_delta), (65, 35));
        // Even on a mirror that is frozen for another reason, it is not this.
        let f = StallFacts {
            signatures: Some(ev.clone()),
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
        // And with many more rejections than accepts, still not this: one
        // accepted signature is proof a pinned key reaches the node.
        let f = StallFacts {
            signatures: Some(SignatureEvidence {
                accepted_delta: 1,
                rejected_delta: 5_000,
                ..ev
            }),
            ..base()
        };
        assert_ne!(
            discriminate(&f).unwrap().class,
            StallClass::PinsRejectEverySignature
        );
    }

    /// The attestor offline: nothing accepted and nothing rejected either.
    /// That is the network waiting, and what was said before is said now.
    #[test]
    fn a_quiet_network_is_not_a_key_problem() {
        let quiet = SignatureEvidence {
            accepted_delta: 0,
            rejected_delta: 0,
            span_secs: FROZEN_VERDICT_SECS * 2,
            shipped_keys_missing: 0,
            missing_explained_by: None,
            update_known: true,
        };
        let waiting = StallFacts {
            frontier_lag: Some(0),
            headers: 190_501,
            signatures: Some(quiet.clone()),
            ..base()
        };
        assert_eq!(discriminate(&waiting), None);
        assert_eq!(
            discriminate(&StallFacts {
                signatures: Some(quiet),
                ..base()
            })
            .unwrap()
            .class,
            StallClass::BodyMissing
        );
        // A trickle below the floor is a few stray relays, not this.
        let trickle = SignatureEvidence {
            accepted_delta: 0,
            rejected_delta: REJECTED_FLOOR - 1,
            span_secs: FROZEN_VERDICT_SECS,
            shipped_keys_missing: 0,
            missing_explained_by: None,
            update_known: false,
        };
        assert_eq!(
            discriminate(&StallFacts {
                signatures: Some(trickle),
                ..base()
            })
            .unwrap()
            .class,
            StallClass::BodyMissing
        );
    }

    #[test]
    fn a_window_shorter_than_the_verdict_never_fires() {
        let mut f = the_reported_mirror();
        f.signatures.as_mut().unwrap().span_secs = FROZEN_VERDICT_SECS - 1;
        assert_ne!(
            discriminate(&f).map(|v| v.class),
            Some(StallClass::PinsRejectEverySignature)
        );
        // And a tip that is not frozen long enough says nothing at all.
        let f = StallFacts {
            frozen_secs: FROZEN_VERDICT_SECS - 1,
            ..the_reported_mirror()
        };
        assert_eq!(discriminate(&f), None);
    }

    #[test]
    fn no_evidence_and_non_mirrors_change_nothing() {
        let f = StallFacts {
            signatures: None,
            ..the_reported_mirror()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
        let f = StallFacts {
            trusted_mirror: false,
            ..the_reported_mirror()
        };
        assert_eq!(discriminate(&f), None);
        let f = StallFacts {
            blocks: 0,
            ..the_reported_mirror()
        };
        assert_eq!(discriminate(&f), None);
    }

    #[test]
    fn the_window_reads_deltas_and_starts_over_when_the_engine_does() {
        let w = window(31, (412, 0), (1_000, 10));
        let d = w.deltas().unwrap();
        assert_eq!((d.accepted, d.rejected, d.span_secs), (0, 300, 900));
        // One sample is no span.
        assert_eq!(window(1, (5, 0), (5, 0)).deltas(), None);
        assert_eq!(SignatureWindow::default().deltas(), None);

        // A counter that goes DOWN is an engine restart: the in-memory
        // counters began again at 0 (plus the archive on `accepted`), so
        // nothing before it is comparable.
        let mut w = window(31, (412, 0), (1_000, 10));
        w.push(1, 31 * 30, 380, 0);
        assert_eq!(w.deltas(), None);
        w.push(1, 32 * 30, 380, 25);
        let d = w.deltas().unwrap();
        assert_eq!((d.accepted, d.rejected, d.span_secs), (0, 25, 30));
        // Only rejected going down is a restart too.
        let mut w = window(5, (412, 1), (1_000, 10));
        w.push(1, 5 * 30, 500, 3);
        assert_eq!(w.deltas(), None);

        // A new engine generation starts over even when the numbers climb.
        let mut w = window(31, (412, 0), (1_000, 10));
        w.push(2, 31 * 30, 9_000, 9_000);
        assert_eq!(w.deltas(), None);
    }

    #[test]
    fn the_window_is_bounded_and_keeps_the_newest_samples() {
        let w = window(200, (0, 1), (0, 2));
        let d = w.deltas().unwrap();
        let held = SIGNATURE_WINDOW_SAMPLES as u64;
        assert_eq!(d.span_secs, (held - 1) * 30);
        assert_eq!((d.accepted, d.rejected), (held - 1, 2 * (held - 1)));
        // Bounded, and still longer than the verdict at the refresher's
        // 30-second sampling, or the rule could never fire.
        assert!(d.span_secs >= FROZEN_VERDICT_SECS);
    }

    #[test]
    fn the_key_verdict_says_what_is_true_in_both_cases_it_cannot_tell_apart() {
        let f = the_reported_mirror();
        let no_update = discriminate(&f).unwrap();
        let with_update = discriminate(&StallFacts {
            signatures: Some(SignatureEvidence {
                update_known: true,
                ..f.signatures.clone().unwrap()
            }),
            ..f.clone()
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(no_update.class).unwrap(),
            "pins_reject_every_signature"
        );
        // An update is known: the list is probably what is out of date.
        assert!(with_update.summary.contains("probably out of date"));
        assert!(with_update.summary.contains("Updating easyNode"));
        // No update known: the app may be current, so the sentence must also
        // hold when the trusted keys simply stopped signing.
        assert!(no_update.summary.contains("Updating easyNode"));
        assert!(no_update.summary.contains("may have stopped signing"));
        assert!(!no_update.summary.contains("probably out of date"));
        for v in [&no_update, &with_update] {
            assert!(v.summary.contains("cannot move"));
            assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
            assert!(
                !v.summary.contains("automatically") && !v.summary.contains("will fix"),
                "say what is known, never promise the outcome"
            );
            // Nothing in the node folder is named when the pin holds every
            // key this app ships.
            assert!(!v.summary.contains("btx_rw.conf"));
            assert!(!v.summary.contains("Copy diagnostics"));
        }
    }

    #[test]
    fn a_pin_missing_shipped_keys_names_the_file_and_the_diagnostics() {
        let f = the_reported_mirror();
        let missing = |file: Option<&'static str>| {
            discriminate(&StallFacts {
                signatures: Some(SignatureEvidence {
                    shipped_keys_missing: 2,
                    missing_explained_by: file,
                    ..f.signatures.clone().unwrap()
                }),
                ..f.clone()
            })
            .unwrap()
        };
        let rw = missing(Some("btx_rw.conf"));
        assert_eq!(rw.class, StallClass::PinsRejectEverySignature);
        assert!(rw.summary.contains("btx_rw.conf in the node folder"));
        assert!(rw.summary.contains("Tools > Copy diagnostics"));
        let unseen = missing(None);
        assert!(!unseen.summary.contains(".conf"));
        assert!(unseen.summary.contains("Tools > Copy diagnostics"));
        for v in [&rw, &unseen] {
            assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
        }
    }

    #[test]
    fn missing_shipped_keys_are_read_against_the_live_pin() {
        use crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS as SHIPPED;
        let all: Vec<String> = SHIPPED.iter().map(|k| k.to_ascii_uppercase()).collect();
        assert!(
            shipped_keys_missing(&all).is_empty(),
            "case does not matter"
        );
        // An engine that does not list its pin says nothing about it.
        assert!(shipped_keys_missing(&[]).is_empty());
        let two_old = vec![
            "03aaaa".repeat(11)[..66].to_string(),
            SHIPPED[0].to_string(),
        ];
        assert_eq!(shipped_keys_missing(&two_old), SHIPPED[1..].to_vec());

        // A missing key the app left to btx_rw.conf (it pins it there, so the
        // command line did not repeat it) is explained by that file.
        let rw = vec![SHIPPED[3].to_string()];
        let missing = vec![SHIPPED[3]];
        assert_eq!(
            where_missing_keys_are_set(&missing, &rw, &[]),
            Some(RW_CONF)
        );
        assert_eq!(
            where_missing_keys_are_set(&missing, &[], &rw),
            Some(FASTSTART_CONF)
        );
        // A key the app put on the command line itself: no file explains it.
        assert_eq!(where_missing_keys_are_set(&missing, &[], &[]), None);
        assert_eq!(where_missing_keys_are_set(&[], &rw, &rw), None);
    }

    #[test]
    fn the_gate_verdict_still_respects_the_at_the_frontier_suppressor() {
        // Zero in flight is CORRECT and expected when there is nothing signed
        // to fetch. Waiting at the frontier must stay silent.
        let quiet = StallFacts {
            frontier_lag: Some(0),
            archive_authority: Some(3),
            blocks_in_flight: Some(0),
            headers: 190_500,
            blocks: 190_500,
            ..base()
        };
        assert_eq!(discriminate(&quiet), None);
    }
}
