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
    /// How long blocks AND headers have both been unchanged (any-change rule).
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
    AttestationMissing,
    NoQualifyingPeer,
    MsghandSpin,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StallVerdict {
    pub class: StallClass,
    /// One plain-language sentence for the UI/log — what is wrong, in terms a
    /// person who has never read net_processing.cpp can act on.
    pub summary: &'static str,
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
                summary: "the node is stuck retrying one block flat-out (a known upstream bug); \
                          it may also be too busy to accept the peer that would fix it — do NOT \
                          restart, keep the archive peers dialling",
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
            summary: "the next block is downloaded but its signed confirmation has not arrived; \
                      the node needs a working archive peer and will retry on its own",
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
            summary: "this node can see the next blocks and is not asking any peer for them \
                      (a known upstream scheduler bug); adding or redialling peers will NOT \
                      help — the fix is to request the next blocks by name, which the \
                      guardian does automatically",
        });
    }
    // A: body missing. Requests ARE outstanding (or we could not measure), so
    // the peer set genuinely is the suspect and redialling is worth doing.
    Some(StallVerdict {
        class: StallClass::BodyMissing,
        summary: "this node has asked for the next blocks and no peer has served them yet; \
                  this is peer selection, not corruption — archive/noban peers usually fix it",
    })
}

/// The class-C verdict — issued both on a frozen gap and on a frontier
/// freeze with an affirmatively empty authority census (total isolation).
fn no_qualifying_peer_verdict() -> StallVerdict {
    StallVerdict {
        class: StallClass::NoQualifyingPeer,
        summary: "no connected peer is allowed to hand this node signed confirmations — \
                  redialling the known archive peers (an RPC addnode counts as manual and \
                  passes the gate with no restart)",
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
        }
    }

    #[test]
    fn healthy_and_guarded_states_get_no_verdict() {
        // Not a mirror.
        assert_eq!(discriminate(&StallFacts { trusted_mirror: false, ..base() }), None);
        // Presync / snapshot load (blocks==0) — the guard that keeps the
        // watchdog from shooting a node mid-loadtxoutset.
        assert_eq!(discriminate(&StallFacts { blocks: 0, ..base() }), None);
        // Not frozen long enough.
        assert_eq!(
            discriminate(&StallFacts { frozen_secs: FROZEN_VERDICT_SECS - 1, ..base() }),
            None
        );
        // At the frontier with a healthy census: nothing to connect, freeze
        // is normal block cadence (or a paused network).
        assert_eq!(
            discriminate(&StallFacts { headers: 190_500, ..base() }),
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
        assert_eq!(discriminate(&f), None, "at the frontier there is nothing to fetch");
        // Ahead of the frontier has even less to fetch.
        assert_eq!(discriminate(&StallFacts { frontier_lag: Some(-3), ..f.clone() }), None);
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
        assert_eq!(discriminate(&banked).unwrap().class, StallClass::AttestationMissing);
        // Class D outranks both and must survive the guard as well.
        let spinning = StallFacts {
            cpu_pct_one_core: Some(99),
            frontier_lag: Some(0),
            archive_authority: Some(2),
            retryable_marker: true,
            ..base()
        };
        assert_eq!(discriminate(&spinning).unwrap().class, StallClass::MsghandSpin);
        // A frontier known to be on a fork is not evidence of anything.
        let forked = StallFacts {
            frontier_lag: Some(0),
            frontier_on_active_chain: Some(false),
            archive_authority: Some(3),
            ..base()
        };
        assert_eq!(discriminate(&forked).unwrap().class, StallClass::BodyMissing);
    }

    /// An unmeasured frontier must change nothing: older engines and failed
    /// RPCs fall back to the height-only rules rather than going silent.
    #[test]
    fn an_unmeasured_frontier_degrades_to_the_old_behaviour() {
        let f = StallFacts { frontier_lag: None, ..base() };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::BodyMissing);
        let f = StallFacts { frontier_lag: None, archive_authority: Some(0), ..base() };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::NoQualifyingPeer);
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
        assert_eq!(discriminate(&f).unwrap().class, StallClass::NoQualifyingPeer);
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
        assert_eq!(discriminate(&f).unwrap().class, StallClass::NoQualifyingPeer);
    }

    #[test]
    fn class_b_attestation_missing_on_marker_with_authority_present() {
        let f = StallFacts { retryable_marker: true, ..base() };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::AttestationMissing);
    }

    #[test]
    fn class_a_body_missing_is_the_quiet_default() {
        assert_eq!(discriminate(&base()).unwrap().class, StallClass::BodyMissing);
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
        let f = StallFacts { cpu_pct_one_core: None, archive_authority: Some(0), ..base() };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::NoQualifyingPeer);
        // No peer info either: an unknown census is NOT "zero peers" — class B/A
        // still classify from the log marker alone.
        let f = StallFacts {
            cpu_pct_one_core: None,
            archive_authority: None,
            retryable_marker: true,
            ..base()
        };
        assert_eq!(discriminate(&f).unwrap().class, StallClass::AttestationMissing);
    }

    #[test]
    fn marker_helper_matches_the_live_log_line() {
        // Verbatim shape from the incident logs.
        let line = "2026-08-16T20:15:01Z ActivateBestChainStep: retryable MatMul failure connecting 3ab1… (leaving candidate)";
        assert!(log_tail_has_retryable_marker(line));
        assert!(!log_tail_has_retryable_marker("UpdateTip: new best=… height=1"));
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
        assert!(v.summary.contains("will NOT"));
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
        let f = StallFacts { blocks_in_flight: None, ..base() };
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
        assert_eq!(discriminate(&isolated).unwrap().class, StallClass::NoQualifyingPeer);
        // And a banked body waiting on its signature is class B regardless.
        let banked = StallFacts {
            retryable_marker: true,
            blocks_in_flight: Some(0),
            ..base()
        };
        assert_eq!(discriminate(&banked).unwrap().class, StallClass::AttestationMissing);
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
