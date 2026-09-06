//! Can this node serve the Esplora REST API?
//!
//! ── WHY THIS IS A GATE AND NOT A SETTING ────────────────────────────────────
//! The BTX PQ wallet reads all chain data over Esplora, and as of 2026-09-04 it
//! has exactly one working source: `api.btxscan.io`. `explorer.minebtx.com`
//! answers 503, and `esplora.btxbyronbay.com` is being retired — measured the
//! same day it was 486 blocks behind and its `/blocks` route 404s.
//!
//! The witness half matters as much as the data half. A wallet settles a fork by
//! comparing the block HASH at a height two sources both hold; a height alone
//! proves nothing, because on 2026-08-24 two mirrors agreed on 199,296 and both
//! were wrong. When Byron goes, nothing else publishes hashes.
//!
//! So easyNode operators are the fix. But an Esplora endpoint that is WRONG is
//! far worse than one that is missing: Byron's address index does not record
//! spends, and on one live address it reported 664.40757255 BTX against a true
//! 157.34199443 — 507 BTX of phantom unspent outputs across 116 entries. A
//! wallet reading that builds transactions spending outputs that no longer
//! exist. The build succeeds locally and every broadcast fails.
//!
//! This module answers the cheapest question standing between an operator and
//! that outcome, and answers it BEFORE anything is installed.
//!
//! ── THE ONE HARD PRECONDITION ───────────────────────────────────────────────
//! electrs builds its index by reading btxd's block files off disk. A pruned
//! datadir deletes those files as they age, so the index can never be built and
//! can never be completed later without a full resync. A node running
//! `prune=5000` can validate, sign, and serve a tip — it can never serve
//! Esplora. Refusing here, with the reason, is the whole point: the alternative
//! is electrs failing obscurely hours into an index it was never going to
//! finish.
//!
//! Facts in, verdict out. Callers that cannot measure a field say `None` rather
//! than defaulting it, the same contract `watchdog.rs` uses — a default here
//! would mean guessing an operator's disk posture, and this gate exists
//! precisely because guessing is what hurts.

use serde::Serialize;

/// What the app knows about a datadir when the operator asks for Esplora mode.
#[derive(Debug, Clone, Default)]
pub struct EsploraFacts {
    /// The `prune=` value the app's own conf asks for, if it states one.
    pub conf_prune: Option<u64>,
    /// `getblockchaininfo.pruned`, if the node was reachable.
    pub node_pruned: Option<bool>,
    /// `getblockchaininfo.pruneheight`, if pruned.
    pub prune_height: Option<u64>,
    /// The persisted node profile: "full" or "keeper".
    pub profile: Option<String>,
    /// Free space on the datadir's filesystem, if it could be read.
    pub free_disk_mb: Option<u64>,
}

/// Why Esplora mode cannot be enabled. Each variant carries what it measured,
/// so the message shown to an operator is never vaguer than the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EsploraBlocker {
    /// The conf asks for pruning. Nothing has been deleted yet if the node has
    /// not run, so this is the cheap, recoverable case.
    ConfIsPruned { prune_mb: u64 },
    /// The node has already pruned. History is gone; only a resync fixes it.
    DatadirAlreadyPruned { prune_height: Option<u64> },
    /// The keeper profile is pruned BY DESIGN (`prune=10000`). This is not a
    /// misconfiguration to correct, it is a different product choice.
    KeeperProfile,
    /// Nothing could be measured. Refusing is the safe answer.
    Unmeasured,
}

/// Non-blocking things an operator should be told before they commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EsploraWarning {
    /// Disk is the cost nobody expects, and it has two parts with very
    /// different provenance.
    ///
    /// The chain is now MEASURED: `setup.rs::MEASURED_CHAIN_PAYLOAD_GIB`, 124
    /// GiB on 2026-09-04, method in docs/archival-capacity.md. That figure is
    /// quoted, because refusing to say a number we have is its own kind of
    /// unhelpful.
    ///
    /// electrs' index sits on top of it and is NOT measured by anybody here, so
    /// no total is quoted. Naming which half is known and which is not beats
    /// both a confident total and a blanket "we don't know" — a confident total
    /// from an unsettled base is how the install gate got a number that later
    /// fell below the chain it was gating.
    DiskUnsettled { free_mb: Option<u64> },
}

/// The verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EsploraVerdict {
    pub blocker: Option<EsploraBlocker>,
    pub warnings: Vec<EsploraWarning>,
}

impl EsploraVerdict {
    pub fn is_allowed(&self) -> bool {
        self.blocker.is_none()
    }
}

/// Decide whether Esplora mode may be enabled.
///
/// The order matters. A keeper is reported as a keeper rather than as "pruned",
/// because those are different conversations: one is a setting to change, the
/// other is a profile the operator chose on purpose.
pub fn check(f: &EsploraFacts) -> EsploraVerdict {
    let mut warnings = Vec::new();
    warnings.push(EsploraWarning::DiskUnsettled {
        free_mb: f.free_disk_mb,
    });

    let blocker = if f.profile.as_deref() == Some("keeper") {
        Some(EsploraBlocker::KeeperProfile)
    } else if f.node_pruned == Some(true) {
        Some(EsploraBlocker::DatadirAlreadyPruned {
            prune_height: f.prune_height,
        })
    } else if let Some(mb) = f.conf_prune.filter(|mb| *mb > 0) {
        Some(EsploraBlocker::ConfIsPruned { prune_mb: mb })
    } else if f.conf_prune.is_none() && f.node_pruned.is_none() {
        // Nothing measured at all. Do not let an operator start a multi-hour
        // index on a datadir whose posture we never established.
        Some(EsploraBlocker::Unmeasured)
    } else {
        None
    };

    EsploraVerdict { blocker, warnings }
}

/// The operator-facing reason. Written to be actionable: what is true, why it
/// blocks, and what would change it.
pub fn explain(b: &EsploraBlocker) -> String {
    match b {
        EsploraBlocker::ConfIsPruned { prune_mb } => format!(
            "This node is configured to prune ({prune_mb} MB). electrs builds its \
             index by reading block files off disk, and a pruned node deletes \
             those files as they age, so the index can never be completed. \
             Set prune=0 and restart before enabling Esplora mode."
        ),
        EsploraBlocker::DatadirAlreadyPruned { prune_height } => {
            let where_ = match prune_height {
                Some(h) => format!(" History below block {h} has already been deleted."),
                None => String::new(),
            };
            format!(
                "This datadir has already been pruned.{where_} electrs indexes from \
                 the block files on disk, and the ones it needs are gone — changing \
                 prune=0 now is not enough, because nothing re-downloads history \
                 that was discarded. Enabling Esplora mode here requires a full \
                 resync with prune=0 from the start."
            )
        }
        EsploraBlocker::KeeperProfile => {
            "The keeper profile keeps about 10 GB of recent blocks on purpose, not \
             the whole chain, so it can never serve historical addresses or \
             transactions. Esplora mode needs the full profile and a complete \
             unpruned chain. Switching profiles means a resync, so this is a \
             deliberate choice rather than a toggle."
                .to_string()
        }
        EsploraBlocker::Unmeasured => {
            "easyNode could not read this node's prune posture, from either its \
             config or a running node. Esplora mode is refused rather than \
             guessed: starting an index on a pruned datadir wastes hours and \
             then fails. Start the node once and try again."
                .to_string()
        }
    }
}

/// The disk warning, kept separate from the blockers because it is honest about
/// not knowing rather than pretending to a threshold.
pub fn explain_warning(w: &EsploraWarning) -> String {
    match w {
        EsploraWarning::DiskUnsettled { free_mb } => {
            let have = match free_mb {
                Some(mb) => format!(" You have about {} GiB free.", mb / 1024),
                None => String::new(),
            };
            let chain = crate::setup::MEASURED_CHAIN_PAYLOAD_GIB;
            format!(
                "Esplora mode needs the FULL unpruned chain plus an electrs index \
                 on top of it, and it grows.{have} The chain measured {chain} GiB on \
                 2026-09-04. We will not quote you a total, because nobody has \
                 measured the electrs index on top of it — so budget above that \
                 figure, not at it. Treat this as a server-class commitment, not a \
                 laptop one, and see docs/archival-capacity.md."
            )
        }
    }
}

/// The `prune=` value the app's own conf asks for: `None` when the conf or the
/// key is absent, which the gate treats as unmeasured rather than as zero.
pub fn conf_prune(conf: &std::path::Path) -> Option<u64> {
    crate::setup::conf_kv(conf, "prune").and_then(|v| v.trim().parse().ok())
}

/// `pruned` and `pruneheight` from a running node's `getblockchaininfo`. Both
/// `None` when the node did not answer. Read straight from the JSON rather
/// than through `node_api::BlockchainInfo`, so the gate can see a field that
/// struct does not carry without widening a type the whole app decodes.
pub async fn node_prune_posture(rpc: &dyn crate::rpc::Rpc) -> (Option<bool>, Option<u64>) {
    match rpc.call("getblockchaininfo", serde_json::json!([])).await {
        Ok(v) => (
            v.get("pruned").and_then(|p| p.as_bool()),
            v.get("pruneheight").and_then(|h| h.as_u64()),
        ),
        Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_prune_reads_the_setting_and_admits_when_there_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("faststart.conf");
        assert_eq!(conf_prune(&conf), None, "no conf is unmeasured, not zero");
        std::fs::write(&conf, "server=1\nprune=0\n").unwrap();
        assert_eq!(conf_prune(&conf), Some(0));
        std::fs::write(&conf, "server=1\nprune=4096\n").unwrap();
        assert_eq!(conf_prune(&conf), Some(4096));
        std::fs::write(&conf, "server=1\n").unwrap();
        assert_eq!(conf_prune(&conf), None);
        std::fs::write(&conf, "prune=yes\n").unwrap();
        assert_eq!(conf_prune(&conf), None, "a non-number is not a posture");
    }

    fn facts() -> EsploraFacts {
        EsploraFacts {
            conf_prune: Some(0),
            node_pruned: Some(false),
            prune_height: None,
            profile: Some("full".into()),
            free_disk_mb: Some(200 * 1024),
        }
    }

    #[test]
    fn a_full_unpruned_node_is_allowed() {
        let v = check(&facts());
        assert!(
            v.is_allowed(),
            "an unpruned full node must be allowed: {v:?}"
        );
    }

    #[test]
    fn a_pruned_conf_is_refused_with_the_number_it_read() {
        // The cheap case: the conf asks for pruning but nothing is deleted yet.
        let f = EsploraFacts {
            conf_prune: Some(5000),
            node_pruned: Some(false),
            ..facts()
        };
        let v = check(&f);
        assert_eq!(
            v.blocker,
            Some(EsploraBlocker::ConfIsPruned { prune_mb: 5000 })
        );
        let msg = explain(v.blocker.as_ref().unwrap());
        assert!(
            msg.contains("5000"),
            "the message must name what it measured: {msg}"
        );
        assert!(
            msg.contains("prune=0"),
            "the message must say what would fix it: {msg}"
        );
    }

    #[test]
    fn an_already_pruned_datadir_says_a_resync_is_required() {
        // The expensive case. This is the distinction that matters to an
        // operator: changing the setting does NOT bring the history back.
        let f = EsploraFacts {
            conf_prune: Some(0),
            node_pruned: Some(true),
            prune_height: Some(184942),
            ..facts()
        };
        let v = check(&f);
        assert_eq!(
            v.blocker,
            Some(EsploraBlocker::DatadirAlreadyPruned {
                prune_height: Some(184942)
            })
        );
        let msg = explain(v.blocker.as_ref().unwrap());
        assert!(
            msg.contains("184942"),
            "name the height that was lost: {msg}"
        );
        assert!(msg.contains("resync"), "say a resync is required: {msg}");
    }

    #[test]
    fn a_live_node_outranks_a_clean_looking_conf() {
        // The exact shape measured on the release box 2026-09-04: the conf says
        // prune=0 and the datadir's btx_rw.conf overrode it, so the node is
        // pruned regardless of what the conf claims. Trust the node.
        let f = EsploraFacts {
            conf_prune: Some(0),
            node_pruned: Some(true),
            prune_height: Some(184942),
            ..facts()
        };
        assert!(
            !check(&f).is_allowed(),
            "a pruned NODE must refuse even when the conf looks clean"
        );
    }

    #[test]
    fn a_keeper_is_told_it_is_a_keeper_not_that_it_is_misconfigured() {
        let f = EsploraFacts {
            profile: Some("keeper".into()),
            conf_prune: Some(10000),
            node_pruned: Some(true),
            ..facts()
        };
        let v = check(&f);
        assert_eq!(v.blocker, Some(EsploraBlocker::KeeperProfile));
        let msg = explain(v.blocker.as_ref().unwrap());
        assert!(msg.contains("keeper"), "{msg}");
        assert!(
            !msg.contains("Set prune=0"),
            "a keeper is a choice, not an error: {msg}"
        );
    }

    #[test]
    fn nothing_measured_refuses_rather_than_assuming_the_best() {
        let f = EsploraFacts {
            conf_prune: None,
            node_pruned: None,
            prune_height: None,
            profile: None,
            free_disk_mb: None,
        };
        assert_eq!(check(&f).blocker, Some(EsploraBlocker::Unmeasured));
    }

    #[test]
    fn the_disk_warning_never_invents_a_total() {
        let v = check(&facts());
        let w = v
            .warnings
            .first()
            .expect("a disk warning is always attached");
        let msg = explain_warning(w);
        // Two halves, and the test pins both. We DO have a measured chain size
        // now and it would be unhelpful to withhold it; we do NOT have a
        // measured electrs index, so no total is quoted. If somebody later
        // hardcodes a total, or drops the figure we actually have, this should
        // make them think.
        assert!(msg.contains("will not quote you a total"), "{msg}");
        assert!(
            msg.contains(&crate::setup::MEASURED_CHAIN_PAYLOAD_GIB.to_string()),
            "quote the half that IS measured: {msg}"
        );
        assert!(
            msg.contains("archival-capacity"),
            "point at the evidence: {msg}"
        );
    }

    #[test]
    fn the_disk_warning_is_attached_even_when_allowed() {
        let v = check(&facts());
        assert!(v.is_allowed());
        assert!(
            !v.warnings.is_empty(),
            "an allowed node still needs the cost stated"
        );
    }
}
