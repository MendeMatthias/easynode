//! What the engine itself is warning about, in words a person can act on.
//!
//! # The problem this exists to solve
//!
//! btxd keeps a list of standing warnings and returns it in
//! `getblockchaininfo.warnings`. Until 0.6.31 the app did not read it, so the
//! engine could be saying, in so many words, that this machine can no longer
//! verify blocks, and the status screen would carry on showing a green LIVE.
//!
//! v0.34.9 puts three warnings there that matter since the MatMul fork, and
//! each is the only place its fact is reported while it is true:
//!
//! * `MATMUL_RC_NEXT_BLOCK_UNVERIFIABLE` (init.cpp:3961). The graphics chip is
//!   not qualified to check the new proof of work, so the node cannot check
//!   the next block. The app already reads btxd's startup verdict from the log,
//!   but that line is written ONCE, at startup. A chip that qualified at
//!   startup and was quarantined an hour later is reported here, and in
//!   nothing the app read before.
//!   This is also how a signer whose graphics card stops qualifying (a new
//!   engine on an older card, btxchain/btx#205) would show up.
//! * `MATMUL_BEHIND_SIGNED_FRONTIER` (validation.cpp:12345). The network's
//!   signers have confirmed at least six blocks this node does not have. On a
//!   node that has not even received the headers, every other signal in this
//!   app reads healthy: blocks equal headers, no longer branch is known, and
//!   the tip is not old enough to be stale yet. It is the one early sign of a
//!   node that sits on a branch the network has left.
//! * `MATMUL_DIVERGENT_POW_FORK` (validation.cpp:7032). The node refused a
//!   header with the wrong difficulty. Nothing to fix: it explains why an
//!   explorer on that refused chain shows different numbers.
//!
//! # What is deliberately not shown
//!
//! * **Cadence hold** (validation.cpp:10651). Local pacing while catching up,
//!   in the engine's own words "not a consensus invalidity", and never cleared
//!   while the node runs. The catch-up line already says when a gap is
//!   closing slowly, from the gap's own trend.
//! * **Deep reorg** (validation.cpp:10564). Never cleared while the node runs
//!   either, and on this network the reorg it reports is usually a recovery: a
//!   node stranded on one of September's dead branches that rejoins the main
//!   chain reorganises hundreds of blocks. It would then carry "this may
//!   indicate a 51% attack -- raise required confirmations and investigate"
//!   until its next restart, for the event that fixed it, with nothing for the
//!   person to do.
//! * **Pre-release build**. Which engine ships is this project's choice, not
//!   something the person running it can change.
//! * **Unknown rules being signalled** (as opposed to activated). Miners
//!   trying something is not a reason for anybody to act; the activated form
//!   is, and it is shown.
//!
//! The heavier invalid chain (`LARGE_WORK_INVALID_CHAIN`) is shown, calmly. A
//! node that refuses a heavier chain is usually right, and an alarm would fire
//! exactly when it is.
//!
//! Anything this version does not recognise is shown in the engine's own
//! words, calmly, rather than hidden: an engine update can add a warning, and
//! a hidden one helps nobody.
//!
//! Everything here is pure, so the sentences are tested rather than trusted.

use crate::node_api::BlockchainInfo;
use serde::Serialize;

/// Blocks behind the signed frontier at which btxd raises its warning.
///
/// Mirrors `st.blocks_behind < 6` in `NotifySignedFrontierStatus`
/// (validation.cpp:12340 at v0.34.9). Used here to recognise the case where
/// the node already knows it is at least that far behind.
pub const FRONTIER_WARNING_BLOCKS: u64 = 6;

/// One warning from the engine, recognised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineWarning {
    /// `MATMUL_RC_NEXT_BLOCK_UNVERIFIABLE`: no qualified device, so the node
    /// cannot check the next block. `reason` is btxd's own token.
    CannotVerifyBlocks { reason: Option<String> },
    /// `MATMUL_BEHIND_SIGNED_FRONTIER`: the signers have confirmed blocks up to
    /// `signed_height`, `blocks_behind` beyond this node.
    BehindSigners {
        blocks_behind: Option<u64>,
        signed_height: Option<u64>,
    },
    /// `CLOCK_OUT_OF_SYNC`: this computer's clock disagrees with its peers by
    /// more than `minutes`.
    ClockOff { minutes: Option<u64> },
    /// `UNKNOWN_NEW_RULES_ACTIVATED` or `SOFTWARE_EXPIRY`: the engine says it
    /// is out of date.
    OutOfDate,
    /// `MATMUL_DIVERGENT_POW_FORK`: a header at `height` was refused for the
    /// wrong difficulty.
    RefusedWrongDifficulty { height: Option<u64> },
    /// `LARGE_WORK_INVALID_CHAIN`: a longer chain exists that the node holds
    /// invalid.
    RefusingInvalidChain,
    /// A warning this version does not recognise, first sentence only.
    Other { text: String },
}

/// A warning ready for the status screen: the sentence, and whether it asks
/// the person for attention or only explains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineNote {
    pub message: String,
    pub needs_attention: bool,
}

impl EngineWarning {
    /// Does this ask something of the person running the node, or is it only
    /// an explanation? Amber for the first, quiet for the second.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self,
            EngineWarning::CannotVerifyBlocks { .. }
                | EngineWarning::BehindSigners { .. }
                | EngineWarning::ClockOff { .. }
                | EngineWarning::OutOfDate
        )
    }

    /// Whether this belongs in the list of engine notes. The first two have a
    /// home of their own on the status screen: not verifying blocks is what the
    /// Block checking card is about, and being behind the signers is a fact
    /// about where the node is on the chain, which the chain card says.
    pub fn is_note(&self) -> bool {
        !matches!(
            self,
            EngineWarning::CannotVerifyBlocks { .. } | EngineWarning::BehindSigners { .. }
        )
    }

    /// One or two sentences for a person, in the app's voice: what is true,
    /// what it means, and what to do when there is something to do.
    pub fn message(&self) -> String {
        match self {
            EngineWarning::CannotVerifyBlocks { reason } => {
                let mut m = String::from(
                    "This machine's graphics chip did not pass the node engine's own check, so \
                     your node cannot check the new proof of work and has stopped following new \
                     blocks. Everything already downloaded is safe. Restarting easyNode runs the \
                     check again.",
                );
                if let Some(r) = reason {
                    m.push_str(&format!(" The engine's reason: {}.", r.replace('_', " ")));
                }
                m
            }
            EngineWarning::BehindSigners {
                blocks_behind,
                signed_height,
            } => {
                let how_far = match (blocks_behind, signed_height) {
                    (Some(n), Some(h)) => format!(
                        "The network's signers have confirmed blocks up to height {}, {} beyond \
                         this node,",
                        group(*h),
                        group(*n)
                    ),
                    (Some(n), None) => format!(
                        "The network's signers have confirmed {} blocks beyond this node,",
                        group(*n)
                    ),
                    _ => {
                        "The network's signers have confirmed blocks beyond this node,".to_string()
                    }
                };
                format!(
                    "{how_far} and your node has not received them. Your view of the chain may \
                     be behind."
                )
            }
            EngineWarning::ClockOff { minutes } => {
                let off = match minutes {
                    Some(m) => format!("more than {m} minutes off"),
                    None => "off".to_string(),
                };
                format!(
                    "Your computer's clock is {off} from the rest of the network, and a wrong \
                     clock can stop your node from following the chain. Set the date and time \
                     to update automatically in your system settings, then restart easyNode."
                )
            }
            // easyNode installs updates by itself, and a failed install has
            // its own banner with the download address, so the sentence does
            // not send anybody looking for a button.
            EngineWarning::OutOfDate => "The node engine says it is out of date and may stop \
                 agreeing with the rest of the network. easyNode installs a newer version by \
                 itself as soon as one is out."
                .to_string(),
            EngineWarning::RefusedWrongDifficulty { height } => {
                let at = match height {
                    Some(h) => format!(" at height {}", group(*h)),
                    None => String::new(),
                };
                format!(
                    "Your node refused a block{at} that was built with the wrong difficulty. A \
                     block explorer showing a chain whose difficulty keeps falling is showing \
                     that refused chain, not the one your node follows."
                )
            }
            EngineWarning::RefusingInvalidChain => "Your node knows of a longer chain that it \
                 considers invalid, and it is not following it. That happens when other \
                 computers stay on a chain that breaks the rules, as after the split at height \
                 227,313."
                .to_string(),
            EngineWarning::Other { text } => format!("Your node's engine reports: {text}"),
        }
    }

    pub fn note(&self) -> EngineNote {
        EngineNote {
            message: self.message(),
            needs_attention: self.needs_attention(),
        }
    }
}

/// Recognise one warning. `None` for the kinds this app deliberately does not
/// show; see the module docs for each one and why.
///
/// Matched on a phrase from the middle of each text rather than its start, so
/// a changed "Warning:" prefix in a later engine does not turn a known warning
/// into an unknown one.
pub fn classify(raw: &str) -> Option<EngineWarning> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    const NOT_SHOWN: [&str; 5] = [
        "Cadence burst hold",
        "Deep reorg detected",
        "pre-release test build",
        "attempting to activate unknown new rules",
        "Unrecognised block version",
    ];
    if NOT_SHOWN.iter().any(|p| t.contains(p)) {
        return None;
    }
    if t.contains("strict-device validation is unavailable") {
        return Some(EngineWarning::CannotVerifyBlocks {
            reason: token_after(t, "reason="),
        });
    }
    if t.contains("blocks behind the signed MatMul frontier") {
        return Some(EngineWarning::BehindSigners {
            blocks_behind: number_before(t, " blocks behind the signed MatMul frontier"),
            signed_height: number_after(t, "frontier (height "),
        });
    }
    if t.contains("out of sync with the network") {
        return Some(EngineWarning::ClockOff {
            minutes: number_after(t, "more than "),
        });
    }
    if t.contains("Unknown new rules activated")
        || t.contains("This software expires")
        || t.contains("This software is expired")
    {
        return Some(EngineWarning::OutOfDate);
    }
    if t.contains("for incorrect proof-of-work bits") {
        return Some(EngineWarning::RefusedWrongDifficulty {
            height: number_after(t, "rejected a header at height "),
        });
    }
    if t.contains("Found invalid chain more than") {
        return Some(EngineWarning::RefusingInvalidChain);
    }
    Some(EngineWarning::Other {
        text: first_sentence(t),
    })
}

/// Everything worth showing from one `getblockchaininfo` answer, those that
/// ask for attention first.
///
/// Being behind the signers is dropped while the node already knows it is
/// behind: during the first sync, or while it holds headers at least as far
/// ahead as the engine's own threshold. The catch-up line and the chain card
/// already say so there, from better numbers. The warning is kept for the one
/// case they cannot see, a node that has not even heard of the blocks.
pub fn from_node(info: &BlockchainInfo) -> Vec<EngineWarning> {
    let knows_it_is_behind = info.initial_block_download
        || info.headers.saturating_sub(info.blocks) >= FRONTIER_WARNING_BLOCKS;
    let mut out: Vec<EngineWarning> = info
        .warnings
        .iter()
        .filter_map(|w| classify(w))
        .filter(|w| !(knows_it_is_behind && matches!(w, EngineWarning::BehindSigners { .. })))
        .collect();
    // Stable, so the engine's own order holds within each group.
    out.sort_by_key(|w| !w.needs_attention());
    out
}

/// The integer that directly follows `marker`, e.g. "height " in "at height
/// 229400 for".
fn number_after(t: &str, marker: &str) -> Option<u64> {
    let rest = &t[t.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// The integer that directly precedes `marker`, e.g. "12" in "is 12 blocks
/// behind".
fn number_before(t: &str, marker: &str) -> Option<u64> {
    let head = &t[..t.find(marker)?];
    let digits: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

/// btxd's `key=value` token after `marker`, up to the end of its sentence.
fn token_after(t: &str, marker: &str) -> Option<String> {
    let rest = &t[t.find(marker)? + marker.len()..];
    let end = rest.find(". ").unwrap_or(rest.len());
    let token = rest[..end].trim().trim_end_matches('.');
    (!token.is_empty()).then(|| token.to_string())
}

/// The first sentence of an unrecognised warning, without the engine's own
/// "Warning:" prefix, capped so one runaway string cannot take over the screen.
fn first_sentence(t: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut s = t;
    for prefix in ["Warning: ", "WARNING: ", "Warning:", "WARNING:"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start();
            break;
        }
    }
    let s = match s.find(". ") {
        Some(i) => &s[..=i],
        None => s,
    };
    if s.chars().count() <= MAX_CHARS {
        return s.to_string();
    }
    let cut: String = s.chars().take(MAX_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// 229400 → "229,400", the way the rest of the app writes heights.
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every string below is what v0.34.9 actually puts in the array, with its
    // format arguments filled in. The source lines are named at each one. If an
    // engine bump changes a text, these are the tests that should break.

    /// init.cpp:3961, as published at startup on a machine whose chip did not
    /// qualify.
    const RC_UNVERIFIABLE: &str = "CRITICAL: MatMul RC strict-device validation is \
        unavailable. The next MatMul RC block at or after activation height 185000 cannot be \
        independently verified by this node; MatMul consensus-validator and \
        attestation-archive services are withheld. provider=cuda \
        reason=strict_device_not_ready. Runtime presence is re-probed on a bounded cooldown, \
        but presence alone is not qualification. Restart with a provider that passes the full \
        self-qualification and production canary, or run as a trusted archive / discovery \
        relay if this process is not a consensus validator.";

    /// validation.cpp:12345, with a known frontier hash.
    const BEHIND_FRONTIER: &str = "Warning: this node is 969 blocks behind the signed MatMul \
        frontier (height 230663, hash 00000000000000000000000000000000000000000000000000000000000c0ffe). \
        getmatmulattestedtip.hash only reflects HAVE_DATA on this chain; a stranded fork reports \
        on_active_chain=true there. See getblockchaininfo.matmul_signed_frontier.";

    /// validation.cpp:7032.
    const DIVERGENT_POW: &str = "This node rejected a header at height 229400 for incorrect \
        proof-of-work bits (bad-diffbits, hash 00ab). EncDr stall-recovery nBits start at \
        height 228000. Explorers listing a falling-difficulty chain above that height are on \
        a rejected fork, not this node's live chain.";

    /// validation.cpp:7007.
    const LARGE_WORK: &str = "Warning: Found invalid chain more than 6 blocks longer than our \
        best chain. This could be due to database corruption or consensus incompatibility with \
        peers.";

    /// node/timeoffsets.cpp:56.
    const CLOCK: &str = "Your computer's date and time appear to be more than 10 minutes out \
        of sync with the network, this may lead to consensus failure. After you've confirmed \
        your computer's clock, this message should no longer appear when you restart your \
        node.";

    /// validation.cpp:10651.
    const CADENCE: &str = "Cadence burst hold: candidate height 230700 would jump the live tip \
        (230650) faster than the 90-second block cadence (burst_max=3, allowed_height=230653). \
        Holding ConnectTip/GETDATA until wall-clock catches up. Local policy, not a consensus \
        invalidity.";

    /// validation.cpp:10564, as it reads when a node stranded at the tip of the
    /// 229,399 phantom branch rejoins the main chain.
    const DEEP_REORG: &str = "Deep reorg detected: a branch would reorganize 296 blocks \
        (profile=default; warn=6; park=0; tip=229694, fork=229398, candidate=230663). Following \
        the most-work chain (warn-only). This may indicate a 51% attack -- raise required \
        confirmations and investigate.";

    fn info(warnings: &[&str], blocks: u64, headers: u64, ibd: bool) -> BlockchainInfo {
        BlockchainInfo {
            blocks,
            headers,
            verification_progress: 1.0,
            initial_block_download: ibd,
            median_time: 0,
            is_stale: false,
            behind_best_header: 0,
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    #[test]
    fn a_chip_that_cannot_verify_is_recognised_with_its_reason() {
        assert_eq!(
            classify(RC_UNVERIFIABLE),
            Some(EngineWarning::CannotVerifyBlocks {
                reason: Some("strict_device_not_ready".into())
            })
        );
        let m = classify(RC_UNVERIFIABLE).unwrap().message();
        assert!(m.contains("strict device not ready"), "{m}");
        assert!(m.contains("Restarting easyNode"), "{m}");
    }

    /// The reason the engine re-publishes every half hour while a chip stays
    /// unqualified (init.cpp, the runtime re-probe). It says a restart is what
    /// clears it, which is why the sentence offers one.
    #[test]
    fn the_re_probe_reason_survives_intact() {
        let text = RC_UNVERIFIABLE.replace(
            "reason=strict_device_not_ready",
            "reason=runtime_identity_present_but_full_qualification_requires_restart",
        );
        assert_eq!(
            classify(&text),
            Some(EngineWarning::CannotVerifyBlocks {
                reason: Some(
                    "runtime_identity_present_but_full_qualification_requires_restart".into()
                )
            })
        );
    }

    #[test]
    fn being_behind_the_signers_reads_both_numbers() {
        let w = classify(BEHIND_FRONTIER).unwrap();
        assert_eq!(
            w,
            EngineWarning::BehindSigners {
                blocks_behind: Some(969),
                signed_height: Some(230_663)
            }
        );
        let m = w.message();
        assert!(m.contains("230,663") && m.contains("969"), "{m}");
        // Same wording as the chain card's own alarms, on purpose: it is the
        // same kind of fact, and it may replace one of them on screen.
        assert!(m.ends_with("Your view of the chain may be behind."), "{m}");
    }

    #[test]
    fn a_frontier_without_a_known_hash_still_reads() {
        // "%s" is empty when the frontier hash is not known.
        let w = classify(
            "Warning: this node is 12 blocks behind the signed MatMul frontier (height 230663). \
             getmatmulattestedtip.hash only reflects HAVE_DATA on this chain.",
        )
        .unwrap();
        assert_eq!(
            w,
            EngineWarning::BehindSigners {
                blocks_behind: Some(12),
                signed_height: Some(230_663)
            }
        );
    }

    #[test]
    fn the_refused_difficulty_fork_explains_rather_than_alarms() {
        let w = classify(DIVERGENT_POW).unwrap();
        assert_eq!(
            w,
            EngineWarning::RefusedWrongDifficulty {
                height: Some(229_400)
            }
        );
        assert!(!w.needs_attention(), "the node did its job");
        assert!(w.message().contains("229,400"));
    }

    /// Told to jpp on 2026-09-26: a node refusing the heavier invalid chain is
    /// the normal state for a correct node, so no alarm, just a calm sentence.
    #[test]
    fn a_heavier_invalid_chain_is_calm() {
        let w = classify(LARGE_WORK).unwrap();
        assert_eq!(w, EngineWarning::RefusingInvalidChain);
        assert!(!w.needs_attention());
    }

    #[test]
    fn a_wrong_clock_says_what_to_do() {
        let w = classify(CLOCK).unwrap();
        assert_eq!(w, EngineWarning::ClockOff { minutes: Some(10) });
        assert!(w.needs_attention());
        let m = w.message();
        assert!(m.contains("more than 10 minutes"), "{m}");
        assert!(m.contains("restart easyNode"), "{m}");
    }

    #[test]
    fn an_out_of_date_engine_is_recognised_in_all_three_wordings() {
        for text in [
            "WARNING: Unknown new rules activated (versionbit 5) - this software is not secure",
            "This software expires soon, and may fall out of consensus. Before 2028-01-01, you \
             must choose to upgrade or override this expiration.",
            "This software is expired, and may be out of consensus. You must choose to upgrade \
             or override this expiration.",
        ] {
            assert_eq!(classify(text), Some(EngineWarning::OutOfDate), "{text}");
        }
        assert!(EngineWarning::OutOfDate.needs_attention());
    }

    /// Each of these is explained in the module docs. The deep reorg one is the
    /// sharpest: a node that rejoins the main chain after being stranded on a
    /// dead branch would carry "this may indicate a 51% attack" until its next
    /// restart, for the very event that fixed it.
    #[test]
    fn the_kinds_nobody_can_act_on_are_not_shown() {
        for text in [
            CADENCE,
            DEEP_REORG,
            "This is a pre-release test build - use at your own risk - do not use for mining or \
             merchant applications",
            "Warning: Miners are attempting to activate unknown new rules (bit 5)! You may or \
             may not need to act to remain secure",
            "Warning: Unrecognised block versions are being mined! Unknown rules may or may not \
             be in effect",
            "Warning: Unrecognised block version (0x20000000) is being mined! Unknown rules may \
             or may not be in effect",
            "",
            "   ",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }

    #[test]
    fn an_unknown_warning_is_shown_in_the_engines_words_not_hidden() {
        let w = classify(
            "Warning: Something new happened. Here is a second sentence with details nobody needs.",
        )
        .unwrap();
        assert_eq!(
            w,
            EngineWarning::Other {
                text: "Something new happened.".into()
            }
        );
        assert_eq!(
            w.message(),
            "Your node's engine reports: Something new happened."
        );
        assert!(!w.needs_attention());
    }

    #[test]
    fn a_runaway_unknown_warning_is_capped() {
        let long = "x".repeat(5_000);
        let EngineWarning::Other { text } = classify(&long).unwrap() else {
            panic!("an unrecognised text is Other");
        };
        assert!(text.chars().count() <= 241, "{}", text.len());
        assert!(text.ends_with('…'));
    }

    /// The case the warning exists for: the node believes it is at the tip,
    /// because it has not even received the headers the signers have signed.
    #[test]
    fn behind_the_signers_is_kept_when_the_node_cannot_see_it_otherwise() {
        let stranded = info(&[BEHIND_FRONTIER], 229_694, 229_694, false);
        assert_eq!(
            from_node(&stranded),
            vec![EngineWarning::BehindSigners {
                blocks_behind: Some(969),
                signed_height: Some(230_663)
            }]
        );
    }

    #[test]
    fn behind_the_signers_is_dropped_while_the_node_knows_it_is_behind() {
        // First sync: the syncing screen already says so.
        let syncing = info(&[BEHIND_FRONTIER], 228_000, 230_663, true);
        assert!(from_node(&syncing).is_empty());
        // Headers far ahead of blocks: the catch-up line and the chain card
        // say it, with the node's own numbers.
        let catching_up = info(&[BEHIND_FRONTIER], 229_694, 230_663, false);
        assert!(from_node(&catching_up).is_empty());
        // Just under the engine's own threshold still counts as not knowing.
        let blind = info(
            &[BEHIND_FRONTIER],
            229_694,
            229_694 + FRONTIER_WARNING_BLOCKS - 1,
            false,
        );
        assert_eq!(from_node(&blind).len(), 1);
    }

    #[test]
    fn attention_comes_first_and_the_engines_order_holds_otherwise() {
        let node = info(
            &[DIVERGENT_POW, CADENCE, LARGE_WORK, CLOCK, RC_UNVERIFIABLE],
            230_000,
            230_000,
            false,
        );
        let kinds: Vec<_> = from_node(&node)
            .into_iter()
            .map(|w| (w.needs_attention(), w))
            .collect();
        assert_eq!(kinds.len(), 4, "cadence hold is not shown: {kinds:?}");
        assert!(matches!(kinds[0].1, EngineWarning::ClockOff { .. }));
        assert!(matches!(
            kinds[1].1,
            EngineWarning::CannotVerifyBlocks { .. }
        ));
        assert!(matches!(
            kinds[2].1,
            EngineWarning::RefusedWrongDifficulty { .. }
        ));
        assert_eq!(kinds[3].1, EngineWarning::RefusingInvalidChain);
    }

    #[test]
    fn a_node_with_nothing_to_say_says_nothing() {
        assert!(from_node(&info(&[], 230_000, 230_000, false)).is_empty());
    }

    #[test]
    fn the_two_warnings_with_their_own_card_are_not_notes() {
        assert!(!classify(RC_UNVERIFIABLE).unwrap().is_note());
        assert!(!classify(BEHIND_FRONTIER).unwrap().is_note());
        for text in [DIVERGENT_POW, LARGE_WORK, CLOCK] {
            assert!(classify(text).unwrap().is_note(), "{text}");
        }
    }

    #[test]
    fn every_sentence_is_plain_and_finished() {
        let all = [
            EngineWarning::CannotVerifyBlocks { reason: None },
            EngineWarning::CannotVerifyBlocks {
                reason: Some("strict_device_not_ready".into()),
            },
            EngineWarning::BehindSigners {
                blocks_behind: Some(7),
                signed_height: Some(230_663),
            },
            EngineWarning::BehindSigners {
                blocks_behind: Some(7),
                signed_height: None,
            },
            EngineWarning::BehindSigners {
                blocks_behind: None,
                signed_height: None,
            },
            EngineWarning::ClockOff { minutes: Some(10) },
            EngineWarning::ClockOff { minutes: None },
            EngineWarning::OutOfDate,
            EngineWarning::RefusedWrongDifficulty { height: Some(1) },
            EngineWarning::RefusedWrongDifficulty { height: None },
            EngineWarning::RefusingInvalidChain,
        ];
        for w in all {
            let m = w.message();
            assert!(m.ends_with('.'), "{m}");
            assert!(!m.contains("  "), "double space: {m}");
            for jargon in [
                "RC",
                "ExactReplay",
                "strict-device",
                "frontier",
                "bad-diffbits",
                "nBits",
                "HAVE_DATA",
                "provider=",
            ] {
                assert!(!m.contains(jargon), "{jargon} must not reach a person: {m}");
            }
        }
    }

    #[test]
    fn the_wire_shape_is_tagged_by_kind() {
        let j = |w: EngineWarning| serde_json::to_value(w).unwrap();
        assert_eq!(
            j(EngineWarning::OutOfDate),
            serde_json::json!({ "kind": "out_of_date" })
        );
        assert_eq!(
            j(EngineWarning::BehindSigners {
                blocks_behind: Some(7),
                signed_height: Some(9)
            }),
            serde_json::json!({ "kind": "behind_signers", "blocks_behind": 7, "signed_height": 9 })
        );
        assert_eq!(
            serde_json::to_value(EngineWarning::ClockOff { minutes: Some(10) }.note()).unwrap()
                ["needs_attention"],
            true
        );
    }

    #[test]
    fn heights_are_grouped_like_the_rest_of_the_app() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(227_313), "227,313");
        assert_eq!(group(1_234_567), "1,234,567");
    }
}
