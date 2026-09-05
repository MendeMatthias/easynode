//! A longer chain exists that this node cannot obtain blocks for.
//!
//! # The problem this exists to solve
//!
//! On 2026-09-05 the project's own validator sat on a minority branch for
//! hours with a green status. `blocks` advanced slowly, `headers` were 300
//! ahead, and `getchaintips` showed a `headers-only` branch of 671 blocks
//! forking at height 210496 that no connected peer served bodies for. The
//! engine takes blocks from inbound peers only when they are GPU attestors,
//! and skips any peer that has never served it a body unless that peer is
//! manual or noban (`net_processing.cpp`, `no_body_availability`), so outbound
//! peer selection decides which chain a node follows — and every outbound peer
//! was on the same minority branch. Nothing in the app said so. A node in that
//! state looks exactly like a healthy node on a quiet network.
//! `docs/incident-2026-09-05-fork.md` has the numbers.
//!
//! # What this module decides
//!
//! Two signals, both read from the node itself, both pure so they are tested
//! rather than trusted:
//!
//! 1. **A longer branch.** Every `getchaintips` entry whose status is
//!    `headers-only` or `valid-headers` is a branch the node knows the headers
//!    of and has never validated bodies for. Its fork point is
//!    `height - branchlen`. When that point lies BELOW the active tip (a real
//!    divergence: we hold blocks the branch does not) and the branch carries
//!    more than [`FORK_LEAD_ALARM`] blocks beyond what our chain has since the
//!    same point, a longer chain exists that this node cannot obtain. A branch
//!    whose fork point IS our tip merely extends our chain: that is ordinary
//!    lag, not a fork, and is left to signal 2.
//! 2. **Headers outrunning blocks.** `headers - blocks` above
//!    [`HEADERS_AHEAD_ALARM`] for at least [`HEADERS_AHEAD_ALARM_SECS`] while
//!    the gap is NOT closing. A node catching up after a night offline is
//!    hundreds of headers ahead of its blocks for an hour and is fine, because
//!    the gap shrinks every tick; a node nobody serves bodies to sits at the
//!    same gap or watches it grow. The gap at the window's start is compared
//!    with the current one, so the second case alarms and the first does not.
//!
//! No guess is made about which chain is right. The message says what is
//! known — a longer chain exists that this node cannot get blocks for — and
//! that the user's view of the chain may be behind.

use serde::{Deserialize, Serialize};

/// One entry of `getchaintips`. Every field defaults, so an entry with a
/// shape this code did not anticipate decodes to something harmless instead
/// of failing the whole call and hiding every other tip with it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct ChainTip {
    #[serde(default)]
    pub height: u64,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub branchlen: u64,
    #[serde(default)]
    pub status: String,
}

impl ChainTip {
    /// Headers known, bodies never validated: the only tips that can mean
    /// "a chain we cannot obtain". `valid-fork` has bodies (it simply has
    /// less work) and `invalid` has been judged; neither is unobtainable.
    pub fn is_headers_only(&self) -> bool {
        matches!(self.status.as_str(), "headers-only" | "valid-headers")
    }

    /// The last block this branch shares with the active chain.
    pub fn fork_height(&self) -> u64 {
        self.height.saturating_sub(self.branchlen)
    }
}

/// Blocks a headers-only branch must lead our chain by, counted from their
/// common ancestor, before it is called a longer chain. Six is the engine's
/// own emergency park depth (`maxreorgdepthpark=6` in the shipped conf): the
/// depth at which btxd itself stops treating a rewrite as routine.
pub const FORK_LEAD_ALARM: u64 = 6;

/// `headers - blocks` above which the gap is worth timing.
pub const HEADERS_AHEAD_ALARM: u64 = 20;

/// How long that gap must persist without closing before it is an alarm.
pub const HEADERS_AHEAD_ALARM_SECS: u64 = 10 * 60;

/// The `headers - blocks` gap as the caller has watched it: when it first
/// crossed [`HEADERS_AHEAD_ALARM`], and how wide it was then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapWindow {
    /// Seconds since the gap first exceeded the threshold.
    pub since_secs: u64,
    /// The gap at that moment, to tell "closing" from "stuck".
    pub behind_at_start: u64,
}

/// What the fork detector found. `None` is healthy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ForkAlarm {
    /// A headers-only branch is longer than the active chain since their
    /// common ancestor, by more than [`FORK_LEAD_ALARM`].
    LongerBranch {
        /// Height of the branch's tip.
        branch_height: u64,
        /// Blocks on the branch since the fork point.
        branch_len: u64,
        /// The last block both chains share.
        fork_height: u64,
        /// Blocks on OUR chain since that point.
        our_len: u64,
        /// `branch_len - our_len`.
        lead: u64,
        /// Seconds this has been the verdict; 0 on the tick that first saw it.
        since_secs: u64,
    },
    /// Headers have outrun blocks past [`HEADERS_AHEAD_ALARM`] for at least
    /// [`HEADERS_AHEAD_ALARM_SECS`] and the gap is not closing.
    HeadersAhead {
        headers: u64,
        blocks: u64,
        behind: u64,
        since_secs: u64,
    },
}

impl ForkAlarm {
    /// Every alarm deserves attention; there is no benign variant. Named so
    /// the payload reads like `archive_service_needs_attention`.
    pub fn needs_attention(&self) -> bool {
        true
    }

    /// The same verdict with the "how long" field set by the caller, which
    /// is the only party that knows when it first saw it.
    pub fn with_since(self, secs: u64) -> Self {
        match self {
            ForkAlarm::LongerBranch {
                branch_height,
                branch_len,
                fork_height,
                our_len,
                lead,
                ..
            } => ForkAlarm::LongerBranch {
                branch_height,
                branch_len,
                fork_height,
                our_len,
                lead,
                since_secs: secs,
            },
            ForkAlarm::HeadersAhead {
                headers,
                blocks,
                behind,
                ..
            } => ForkAlarm::HeadersAhead {
                headers,
                blocks,
                behind,
                since_secs: secs,
            },
        }
    }

    /// One sentence for a human. It states what is known and nothing more:
    /// no verdict on which chain is right, no blame, no false comfort.
    pub fn message(&self) -> String {
        match self {
            ForkAlarm::LongerBranch {
                branch_height,
                lead,
                fork_height,
                ..
            } => format!(
                "A longer chain exists (height {branch_height}, {lead} blocks ahead of this node \
                 since the two split at height {fork_height}) that your node cannot obtain \
                 blocks for. Your view of the chain may be behind."
            ),
            ForkAlarm::HeadersAhead {
                behind, since_secs, ..
            } => format!(
                "Your node has known of {behind} block headers beyond its own blocks for {} \
                 minutes and the gap is not closing: no connected peer is serving them. Your \
                 view of the chain may be behind.",
                since_secs / 60
            ),
        }
    }
}

/// The longest headers-only branch that beats the active chain by more than
/// [`FORK_LEAD_ALARM`] since their common ancestor, if there is one.
///
/// `None` when the tips carry no `active` entry: with no tip of our own there
/// is nothing to compare against, and unknown must never alarm.
pub fn longer_branch(tips: &[ChainTip]) -> Option<ForkAlarm> {
    let active = tips.iter().find(|t| t.status == "active")?.height;
    tips.iter()
        .filter(|t| t.is_headers_only())
        .filter_map(|t| {
            let fork_height = t.fork_height();
            // Fork point at or past our tip: the branch extends our chain.
            // That is lag, which `headers_ahead` judges, not a fork.
            if fork_height >= active {
                return None;
            }
            let our_len = active - fork_height;
            // Shorter than ours since the split: a stale sibling, not a rival.
            let lead = t.branchlen.checked_sub(our_len)?;
            (lead > FORK_LEAD_ALARM).then_some(ForkAlarm::LongerBranch {
                branch_height: t.height,
                branch_len: t.branchlen,
                fork_height,
                our_len,
                lead,
                since_secs: 0,
            })
        })
        .max_by_key(|a| match a {
            ForkAlarm::LongerBranch { lead, .. } => *lead,
            ForkAlarm::HeadersAhead { .. } => 0,
        })
}

/// Headers more than [`HEADERS_AHEAD_ALARM`] beyond blocks, for at least
/// [`HEADERS_AHEAD_ALARM_SECS`], with the gap not closing. `gap` is `None`
/// when the caller has not seen the gap cross the threshold, which is the
/// healthy case and never alarms.
pub fn headers_ahead(blocks: u64, headers: u64, gap: Option<GapWindow>) -> Option<ForkAlarm> {
    let behind = headers.saturating_sub(blocks);
    let gap = gap?;
    if behind <= HEADERS_AHEAD_ALARM
        || gap.since_secs < HEADERS_AHEAD_ALARM_SECS
        || behind < gap.behind_at_start
    {
        return None;
    }
    Some(ForkAlarm::HeadersAhead {
        headers,
        blocks,
        behind,
        since_secs: gap.since_secs,
    })
}

/// The verdict. A longer branch wins over a bare gap because it says more:
/// where the chains split and by how much.
pub fn fork_alarm(
    tips: &[ChainTip],
    blocks: u64,
    headers: u64,
    gap: Option<GapWindow>,
) -> Option<ForkAlarm> {
    longer_branch(tips).or_else(|| headers_ahead(blocks, headers, gap))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(height: u64, branchlen: u64, status: &str) -> ChainTip {
        ChainTip {
            height,
            hash: format!("{height:064x}"),
            branchlen,
            status: status.to_string(),
        }
    }

    /// `getchaintips` on the release box at 18:41Z on 2026-09-05
    /// (docs/incident-2026-09-05-fork.md), trimmed to the entries that matter
    /// plus the routine stubs a BTX node always carries.
    fn incident_tips() -> Vec<ChainTip> {
        vec![
            tip(211167, 671, "headers-only"),
            tip(211077, 581, "headers-only"),
            tip(210865, 0, "active"),
            tip(210771, 1, "valid-fork"),
            tip(210681, 1, "headers-only"),
            tip(210566, 1, "valid-fork"),
            tip(210492, 1, "headers-only"),
            tip(210483, 1, "headers-only"),
            tip(210435, 13, "valid-fork"),
            tip(210401, 8, "headers-only"),
            tip(191803, 112, "invalid"),
        ]
    }

    #[test]
    fn the_wire_shape_is_what_the_ui_declares() {
        // apps/node/src/main.ts types this as
        //     fork: { kind: string; since_secs?: number } | null
        // and renders `fork_message`, never the fields. The tag name and the
        // snake_case variants still cross a language boundary and are pinned.
        let v = serde_json::to_value(ForkAlarm::LongerBranch {
            branch_height: 211167,
            branch_len: 671,
            fork_height: 210496,
            our_len: 369,
            lead: 302,
            since_secs: 5,
        })
        .unwrap();
        assert_eq!(v["kind"], "longer_branch");
        assert_eq!(v["branch_height"], 211167);
        assert_eq!(v["fork_height"], 210496);
        assert_eq!(v["since_secs"], 5);
        let v = serde_json::to_value(ForkAlarm::HeadersAhead {
            headers: 211167,
            blocks: 210865,
            behind: 302,
            since_secs: 900,
        })
        .unwrap();
        assert_eq!(v["kind"], "headers_ahead");
        assert_eq!(v["behind"], 302);
    }

    #[test]
    fn healthy_tips_do_not_alarm() {
        let tips = vec![
            tip(210865, 0, "active"),
            tip(210771, 1, "valid-fork"),
            tip(210435, 13, "valid-fork"),
            tip(191803, 112, "invalid"),
        ];
        assert_eq!(longer_branch(&tips), None);
        assert_eq!(fork_alarm(&tips, 210865, 210865, None), None);
    }

    #[test]
    fn a_single_block_stale_tip_must_not_alarm() {
        // The commonest shape on a BTX node: a sibling header that lost. Its
        // fork point is one below our tip and its lead is negative.
        let tips = vec![tip(210865, 0, "active"), tip(210681, 1, "headers-only")];
        assert_eq!(longer_branch(&tips), None);
        // Even a longer stale sibling that is still SHORTER than our chain
        // since the split is not a rival.
        let tips = vec![tip(210865, 0, "active"), tip(210401, 8, "headers-only")];
        assert_eq!(longer_branch(&tips), None);
    }

    #[test]
    fn a_branch_that_merely_extends_our_tip_is_lag_not_a_fork() {
        // A node catching up: 500 headers beyond our tip on OUR chain. Their
        // fork point is our tip, so nothing has diverged.
        let tips = vec![tip(210865, 0, "active"), tip(211365, 500, "headers-only")];
        assert_eq!(longer_branch(&tips), None);
    }

    #[test]
    fn the_2026_09_05_shape_alarms_with_the_real_numbers() {
        let alarm = longer_branch(&incident_tips()).expect("the incident must alarm");
        assert_eq!(
            alarm,
            ForkAlarm::LongerBranch {
                branch_height: 211167,
                branch_len: 671,
                fork_height: 210496,
                our_len: 369,
                lead: 302,
                since_secs: 0,
            }
        );
        // The longest branch wins over the 211077 one.
        assert!(alarm.needs_attention());
        let msg = alarm.with_since(120).message();
        assert!(msg.contains("211167"), "{msg}");
        assert!(msg.contains("302"), "{msg}");
        assert!(msg.contains("210496"), "{msg}");
        assert!(msg.contains("cannot obtain blocks"), "{msg}");
    }

    #[test]
    fn a_lead_at_the_threshold_does_not_alarm_and_one_past_it_does() {
        // Fork point 990, ours 10 blocks since, theirs 16: lead 6, the threshold.
        let at = vec![tip(1000, 0, "active"), tip(1006, 16, "headers-only")];
        assert_eq!(longer_branch(&at), None);
        // Theirs 17: lead 7, one past it.
        let past = vec![tip(1000, 0, "active"), tip(1007, 17, "headers-only")];
        assert!(matches!(
            longer_branch(&past),
            Some(ForkAlarm::LongerBranch {
                lead: 7,
                fork_height: 990,
                ..
            })
        ));
    }

    #[test]
    fn no_active_tip_means_unknown_and_unknown_never_alarms() {
        let tips = vec![tip(211167, 671, "headers-only")];
        assert_eq!(longer_branch(&tips), None);
    }

    #[test]
    fn headers_ahead_needs_the_gap_the_time_and_no_progress() {
        // No window yet: nothing to say.
        assert_eq!(headers_ahead(210865, 211167, None), None);
        // Window too young.
        let young = GapWindow {
            since_secs: 60,
            behind_at_start: 302,
        };
        assert_eq!(headers_ahead(210865, 211167, Some(young)), None);
        // Old enough, gap unchanged: alarm.
        let stuck = GapWindow {
            since_secs: 700,
            behind_at_start: 302,
        };
        assert_eq!(
            headers_ahead(210865, 211167, Some(stuck)),
            Some(ForkAlarm::HeadersAhead {
                headers: 211167,
                blocks: 210865,
                behind: 302,
                since_secs: 700,
            })
        );
        // Old enough, gap GROWN: alarm.
        let grown = GapWindow {
            since_secs: 700,
            behind_at_start: 175,
        };
        assert!(headers_ahead(210865, 211167, Some(grown)).is_some());
        // Old enough but the gap is CLOSING: a node catching up. No alarm.
        let closing = GapWindow {
            since_secs: 700,
            behind_at_start: 800,
        };
        assert_eq!(headers_ahead(210865, 211167, Some(closing)), None);
        // Gap back under the threshold: no alarm whatever the window says.
        assert_eq!(headers_ahead(211150, 211167, Some(stuck)), None);
    }

    #[test]
    fn a_longer_branch_outranks_a_bare_gap() {
        let stuck = GapWindow {
            since_secs: 700,
            behind_at_start: 302,
        };
        let alarm = fork_alarm(&incident_tips(), 210865, 211167, Some(stuck)).unwrap();
        assert!(matches!(alarm, ForkAlarm::LongerBranch { .. }));
        // With no rival branch the gap speaks.
        let tips = vec![tip(210865, 0, "active"), tip(211167, 302, "headers-only")];
        let alarm = fork_alarm(&tips, 210865, 211167, Some(stuck)).unwrap();
        assert!(matches!(alarm, ForkAlarm::HeadersAhead { behind: 302, .. }));
        let msg = alarm.message();
        assert!(msg.contains("302"), "{msg}");
        assert!(msg.contains("11 minutes"), "{msg}");
    }
}
