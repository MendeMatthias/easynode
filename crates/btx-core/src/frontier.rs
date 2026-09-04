//! Whether this node is actually doing the job it advertises.
//!
//! # The problem this exists to solve
//!
//! A keeper advertises service bit 31, `NODE_MATMUL_ATTESTATION_ARCHIVE`, which
//! means "ask me for attestations at any height". That advertisement is set
//! once, at startup, from configuration.
//!
//! What it can actually serve is decided per request, and it is not the same
//! thing. From v0.34.5:
//!
//! ```text
//! net_processing.cpp:18744
//!     catching_up_behind_frontier =
//!         frontier.available && frontier.blocks_behind >= 2;
//!
//! node/matmul_trusted_attestations.h:436
//!     if (!catching_up_behind_frontier) return true;   // serve any height
//!     // otherwise collapse to the narrow live window
//! ```
//!
//! So **two blocks behind the signed frontier and a keeper silently stops
//! serving history, while still advertising that it does.** Nothing logs it,
//! nothing withdraws the bit, and the operator has no way to know.
//!
//! A fleet of those advertises capacity it is not providing. The directory
//! counts the bit, the network dials them, and they answer nothing useful. That
//! is the same failure as counting mirrors as witnesses: a number that looks
//! like a service that exists.
//!
//! # What this changes about the app
//!
//! "Running" is not the status that matters for a keeper. **At the frontier** is
//! the status that matters, and when a node drops out of it the app should say
//! so plainly rather than continue showing a green light.
//!
//! This module is the honest answer to "is my machine currently useful as an
//! archive", and it is deliberately pure so the UI cannot disagree with it.

use serde::Serialize;

/// Blocks behind the signed frontier at which btxd stops serving history.
///
/// Mirrors `frontier.blocks_behind >= 2` at v0.34.5 net_processing.cpp:18744.
/// If upstream moves this, the app starts lying, so it is named and sourced
/// rather than written as a bare `2` at a call site.
pub const FRONTIER_LAG_STOPS_HISTORY: i64 = 2;

/// What this node is really providing to other nodes right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArchiveService {
    /// Advertising the archive bit AND at the frontier, so history is served.
    /// The only state in which a keeper is doing its job.
    ServingHistory,
    /// Advertising the archive bit, but far enough behind the frontier that
    /// btxd has quietly narrowed to the live window. The node looks like an
    /// archive from outside and is not behaving as one.
    DegradedToLiveWindow { blocks_behind: i64 },
    /// Not configured to serve attestations. Honest and fine: a node can be
    /// useful without being an archive.
    NotServing,
    /// Serving, but the frontier is not readable yet, so we genuinely do not
    /// know. Said as "unknown" rather than guessed either way.
    Unknown,
}

impl ArchiveService {
    /// Is this node currently useful to somebody asking for historical
    /// attestations? `false` for the degraded state, which is the whole point.
    pub fn is_serving_history(&self) -> bool {
        matches!(self, ArchiveService::ServingHistory)
    }

    /// Does this deserve the operator's attention? True only for the state
    /// where the node is misrepresenting itself to the network.
    pub fn needs_attention(&self) -> bool {
        matches!(self, ArchiveService::DegradedToLiveWindow { .. })
    }

    /// One sentence for a human, in the app's voice: what is happening, and
    /// what it means for other people. No jargon, no blame, no false alarm.
    pub fn message(&self) -> String {
        match self {
            ArchiveService::ServingHistory => {
                "At the frontier and serving attestation history to other nodes.".to_string()
            }
            ArchiveService::DegradedToLiveWindow { blocks_behind } => format!(
                "Behind the signed frontier by {blocks_behind} blocks, so this node has \
                 stopped serving attestation history even though it still advertises that \
                 it does. It is still following the chain. Leaving it running and connected \
                 is usually all it needs to catch up."
            ),
            ArchiveService::NotServing => {
                "Not serving attestations. This node follows the chain for itself.".to_string()
            }
            ArchiveService::Unknown => {
                "Serving attestations. Waiting to read the signed frontier before reporting \
                 whether history is being served."
                    .to_string()
            }
        }
    }
}

/// Decide what this node is really providing.
///
/// * `advertises_archive` — is attestation serving configured on, i.e. would
///   btxd have set bit 31 at startup.
/// * `blocks_behind` — `signed_frontier.blocks_behind`, `None` when the
///   frontier has not been read yet.
pub fn archive_service(advertises_archive: bool, blocks_behind: Option<i64>) -> ArchiveService {
    if !advertises_archive {
        return ArchiveService::NotServing;
    }
    match blocks_behind {
        None => ArchiveService::Unknown,
        // Negative would mean ahead of the frontier, which is normal and is not
        // a lag. Treat anything below the threshold as healthy.
        Some(n) if n < FRONTIER_LAG_STOPS_HISTORY => ArchiveService::ServingHistory,
        Some(n) => ArchiveService::DegradedToLiveWindow { blocks_behind: n },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_the_frontier_is_the_only_state_that_serves_history() {
        assert_eq!(
            archive_service(true, Some(0)),
            ArchiveService::ServingHistory
        );
        assert_eq!(
            archive_service(true, Some(1)),
            ArchiveService::ServingHistory
        );
        assert!(archive_service(true, Some(1)).is_serving_history());
    }

    #[test]
    fn two_blocks_behind_is_where_btxd_stops_serving_history() {
        // The exact boundary in net_processing.cpp:18744 is `>= 2`. One block
        // behind still serves; two does not. Getting this off by one would make
        // the app report a healthy archive that answers nothing.
        assert_eq!(
            archive_service(true, Some(1)),
            ArchiveService::ServingHistory
        );
        assert_eq!(
            archive_service(true, Some(2)),
            ArchiveService::DegradedToLiveWindow { blocks_behind: 2 }
        );
        assert!(!archive_service(true, Some(2)).is_serving_history());
    }

    #[test]
    fn being_ahead_of_the_frontier_is_not_a_fault() {
        // The frontier is what has been SIGNED. A node can legitimately hold a
        // tip above it, and that must not read as degraded.
        assert_eq!(
            archive_service(true, Some(-3)),
            ArchiveService::ServingHistory
        );
    }

    #[test]
    fn a_node_that_does_not_serve_is_not_reported_as_broken() {
        assert_eq!(
            archive_service(false, Some(999)),
            ArchiveService::NotServing
        );
        assert_eq!(archive_service(false, None), ArchiveService::NotServing);
        assert!(!archive_service(false, Some(999)).needs_attention());
    }

    #[test]
    fn an_unread_frontier_is_unknown_rather_than_guessed() {
        // Reporting "fine" here would be a lie on startup, and reporting
        // "degraded" would cry wolf every launch.
        let s = archive_service(true, None);
        assert_eq!(s, ArchiveService::Unknown);
        assert!(!s.is_serving_history());
        assert!(!s.needs_attention(), "startup must not raise an alarm");
    }

    #[test]
    fn only_the_misrepresenting_state_asks_for_attention() {
        assert!(archive_service(true, Some(50)).needs_attention());
        assert!(!archive_service(true, Some(0)).needs_attention());
        assert!(!archive_service(false, None).needs_attention());
        assert!(!archive_service(true, None).needs_attention());
    }

    #[test]
    fn the_degraded_message_says_what_it_means_for_other_people() {
        let m = archive_service(true, Some(7)).message();
        assert!(
            m.contains('7'),
            "the operator should see how far behind: {m}"
        );
        // The point of the message is that the node is misrepresenting itself.
        assert!(m.contains("advertises"), "{m}");
        // And it must not read as a crash, because nothing has crashed.
        assert!(m.contains("still following the chain"), "{m}");
    }

    #[test]
    fn every_state_has_a_plain_sentence_with_no_jargon_leaking() {
        for s in [
            archive_service(true, Some(0)),
            archive_service(true, Some(9)),
            archive_service(false, None),
            archive_service(true, None),
        ] {
            let m = s.message();
            assert!(!m.is_empty());
            assert!(m.ends_with('.'), "{m}");
            for jargon in [
                "bit 31",
                "GETMMATTEST",
                "NODE_MATMUL",
                "catching_up_behind_frontier",
            ] {
                assert!(!m.contains(jargon), "{jargon} must not reach a user: {m}");
            }
        }
    }

    #[test]
    fn it_serialises_with_a_state_tag_the_ui_can_switch_on() {
        let v = serde_json::to_value(archive_service(true, Some(4))).unwrap();
        assert_eq!(v["state"], "degraded_to_live_window");
        assert_eq!(v["blocks_behind"], 4);
        assert_eq!(
            serde_json::to_value(archive_service(true, Some(0))).unwrap()["state"],
            "serving_history"
        );
    }
}
