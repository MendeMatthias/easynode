//! Which role this node actually fills on the network, said plainly.
//!
//! # The problem this exists to solve
//!
//! The app can already say whether the node is running, whether it is at the
//! frontier, and whether the archive bit means what it appears to mean
//! ([`crate::frontier`]). It has never said which ROLE the machine fills:
//! whether it validates blocks itself or follows other people's signatures,
//! whether the key it holds produces anything, whether anyone can reach it.
//! Nobody could read that off any screen, and three things went wrong because
//! of it.
//!
//! On 2026-09-03 an operator offered a signing key. It was pinned, and it
//! produced zero signatures for eleven days, because his node runs as a
//! trusted mirror and a mirror CONSUMES attestations rather than producing
//! them. Nothing was broken and nothing was logged; there was simply no
//! surface on which "your key signs nothing here" could have appeared.
//!
//! This project's own signer advertises `MATMUL_ATTESTATION_ARCHIVE` while
//! holding a key. `src/node/matmul_trusted_attestations.h:317` clamps a node
//! with a local signer to `SIGNER_GETMMATTEST_SERVE_WINDOW = 16` blocks and
//! refuses everything older; a keyless node serves the full range. On
//! 2026-09-13 the explorer's mirror needed one signature for a block ~800
//! back, every peer it asked refused, and it sat frozen for twenty-one hours.
//!
//! A non-Mac host without a qualifying GPU is launched in `consensus` mode
//! (see the degraded-start block in `node::build_node_command`). It starts,
//! follows headers, gets neither `NODE_MATMUL_CONSENSUS` nor the archive bit,
//! and stalls below the Epoch A height. Its operator currently believes it is
//! helping.
//!
//! # What this changes about the app
//!
//! One small card, one line per fact, each line ending in a sentence about
//! whether that fact helps the network. Every fact comes from the engine's own
//! answers — `getmatmultrustedstatus` and `getnetworkinfo` — never from the
//! platform or from a setting, because a setting is what WILL be true at the
//! next start and the wire is what IS. Absence of an answer is reported as
//! unknown and is never read as evidence.
//!
//! Deliberately pure, like [`crate::frontier`], so the UI cannot disagree with
//! it and every sentence has a test.

use crate::frontier::ArchiveService;
use crate::node_api::{
    MatmulTrustedStatus, NODE_MATMUL_ATTESTATION_ARCHIVE_BIT, NODE_MATMUL_CONSENSUS_BIT,
};
use serde::Serialize;

/// Blocks of history a node HOLDING A KEY will answer for, from
/// `SIGNER_GETMMATTEST_SERVE_WINDOW` at `matmul_trusted_attestations.h:317`.
/// Named here so the sentence that quotes it and the test that checks the
/// sentence cannot drift apart.
pub const SIGNER_SERVE_WINDOW_BLOCKS: u64 = 16;

/// Headers ahead of the tip at which this node is called "behind" rather than
/// "at the tip". One header ahead is a block in flight — BTX mints one every
/// ~90 s and a body follows its header by seconds — so calling that "behind"
/// would flicker on every block. Two matches the tolerance
/// [`crate::frontier::FRONTIER_LAG_STOPS_HISTORY`] applies to the signed
/// frontier, so the two cards never disagree about what "behind" means.
pub const HEADERS_AHEAD_IS_BEHIND: u64 = 2;

/// How long a fresh run gets before "no inbound connections" is called a
/// reachability problem. A reachable node still needs a while to be found by
/// anyone; a false alarm in the first minutes trains people to ignore the
/// line. Mirrors `REACHABILITY_GRACE_SECS` in `apps/node/src/contribution.ts`,
/// which applies the same rule to the "Helping the network" card, so the two
/// cards flip on the same tick.
pub const REACHABILITY_GRACE_SECS: u64 = 30 * 60;

/// How many recent blocks the signed-block window must have read before "on
/// none of them" is a verdict rather than a window that has barely started.
/// Ten blocks is about fifteen minutes of chain; a node that validated ten
/// blocks and signed none of them is telling us something.
pub const SIGNED_WINDOW_MIN_SEEN: u64 = 10;

/// The P2P port a router has to forward for this node to be reachable. Only
/// ever quoted to a person, never dialled from here.
const P2P_PORT: u16 = 19335;

/// How the engine says it validates MatMul, from
/// `getmatmultrustedstatus.matmul_validation_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationMode {
    /// `consensus`: validates the proof itself. Whether it CAN is a separate
    /// fact — see [`NodeRole::advertises_consensus`].
    Consensus,
    /// `trusted`: follows a quorum of signed attestations instead of
    /// replaying the proof. Consumes attestations; produces none.
    Trusted,
    /// `relay`: the 0.34 introducer role. Serves no blocks at all. The app
    /// never configures this itself (it refuses to start beside a wallet).
    Relay,
    /// The engine did not answer, or answered with a mode this app does not
    /// know. Never guessed from the platform.
    Unknown,
}

/// One fact about this node and one sentence about whether it helps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleLine {
    /// The fact's name, e.g. "Validation".
    pub label: &'static str,
    /// The fact itself, short, e.g. "Checking every block itself".
    pub value: String,
    /// `Some(true)` helps other nodes; `Some(false)` does not; `None` when the
    /// engine did not say, or when the fact is simply neutral (most nodes hold
    /// no key, and that is not a shortfall).
    pub helps: Option<bool>,
    /// One or two plain sentences in the app's voice: what is happening and
    /// what it means for other people. No jargon, no blame.
    pub note: String,
}

/// The role this node fills right now, decided from the engine's own answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeRole {
    pub validation_mode: ValidationMode,
    /// `None` when `getmatmultrustedstatus` did not answer. Absence of an
    /// answer is never evidence of absence of a key.
    pub holds_signing_key: Option<bool>,
    /// `NODE_MATMUL_CONSENSUS` (bit 27) is on the wire. In `consensus` mode
    /// this is the difference between a validating node and a degraded one.
    pub advertises_consensus: bool,
    /// `NODE_MATMUL_ATTESTATION_ARCHIVE` (bit 31) is on the wire.
    pub advertises_archive: bool,
    /// At least one peer has connected TO this node.
    pub reachable_inbound: bool,
    pub inbound: u64,
    /// Seconds this run has been up, for the reachability grace period.
    pub uptime_secs: u64,
    /// Headers this node knows about beyond the blocks it holds, or `None`
    /// when the phase carries no height.
    pub blocks_behind: Option<u64>,
    /// The archive verdict this role reports, reconciled against the wire (see
    /// [`node_role`]). Not serialised: the status payload already carries the
    /// refresher's own `archive_service`, and two fields with the same name
    /// that can differ is a trap for whoever reads the wire next.
    #[serde(skip)]
    archive: ArchiveService,
    /// The wire and the app's serving switch disagree: the bit is on but the
    /// switch is off, or the reverse. Both mean "changes at the next start",
    /// and the line says so instead of judging a state that is about to end.
    #[serde(skip)]
    archive_pending_restart: bool,
    /// How many of the newest blocks carry this node's own signature, read
    /// from the node's stored attestations (`btx_core::signer`). `None` until
    /// the window has been read, or when the node holds no key. This is the
    /// difference between "a key is configured" and "the key is doing
    /// something", which no surface could show on 2026-09-03.
    pub signed_recent: Option<crate::signer::SignedRecent>,
}

/// Parse `getnetworkinfo.localservices` (16 hex chars, `0x` tolerated).
fn service_bits(hex: &str) -> Option<u64> {
    let t = hex.trim().trim_start_matches("0x");
    if t.is_empty() {
        return None;
    }
    u64::from_str_radix(t, 16).ok()
}

/// Does this node advertise the service? Decided from the BITS; the names are
/// only a fallback for an engine that omitted `localservices`, and then only
/// on the exact name. Same rule as `node_api::advertises_archive` and for the
/// same reason: a btxd that predates a bit's name renders it `UNKNOWN[2^31]`,
/// which a name match misreads, and a substring match false-positives on any
/// future name that happens to contain the word.
fn advertises(hex: &str, names: &[String], bit: u64, name: &str) -> bool {
    match service_bits(hex) {
        Some(bits) => bits & bit != 0,
        None => names.iter().any(|n| n == name),
    }
}

fn validation_mode(status: Option<&MatmulTrustedStatus>) -> ValidationMode {
    let Some(s) = status else {
        return ValidationMode::Unknown;
    };
    match s
        .matmul_validation_mode
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "consensus" => ValidationMode::Consensus,
        "trusted" => ValidationMode::Trusted,
        "relay" => ValidationMode::Relay,
        // An engine that reports the flag but not the string.
        "" if s.trusted_mirror => ValidationMode::Trusted,
        _ => ValidationMode::Unknown,
    }
}

/// Decide which role this node fills.
///
/// * `status` — `getmatmultrustedstatus`, or `None` when the engine did not
///   answer (older engines do not know the method).
/// * `localservices_hex` / `localservices_names` — from `getnetworkinfo`.
/// * `connections_in` — `getnetworkinfo.connections_in`.
/// * `uptime_secs` — this run's age, for the reachability grace period.
/// * `blocks_behind` — headers minus blocks from the phase, `None` when the
///   phase carries no height.
/// * `archive` — the refresher's [`ArchiveService`] verdict, `None` before its
///   first tick. Passed in rather than recomputed so the two cards can never
///   disagree about the frontier; reconciled here only against facts the
///   refresher did not have (the live bit, and the key).
pub fn node_role(
    status: Option<&MatmulTrustedStatus>,
    localservices_hex: &str,
    localservices_names: &[String],
    connections_in: u64,
    uptime_secs: u64,
    blocks_behind: Option<u64>,
    archive: Option<&ArchiveService>,
) -> NodeRole {
    let holds_signing_key = status.map(|s| s.local_signer);
    let advertises_consensus = advertises(
        localservices_hex,
        localservices_names,
        NODE_MATMUL_CONSENSUS_BIT,
        "MATMUL_CONSENSUS",
    );
    let advertises_archive = advertises(
        localservices_hex,
        localservices_names,
        NODE_MATMUL_ATTESTATION_ARCHIVE_BIT,
        "MATMUL_ATTESTATION_ARCHIVE",
    );

    // The refresher judges the frontier from the app's serving SWITCH; this
    // judges the wire. They disagree only across a restart boundary (the
    // switch was flipped while the node ran), and then the wire is what other
    // nodes see, so the wire wins and the line says a restart is pending.
    let refresher_says_serving = archive.is_some_and(|a| *a != ArchiveService::NotServing);
    let archive_pending_restart = advertises_archive != refresher_says_serving && archive.is_some();
    let archive = match (advertises_archive, holds_signing_key, archive) {
        (false, _, _) => ArchiveService::NotServing,
        // Checked BEFORE the passed verdict, and independent of chain
        // position, for the reason frontier.rs gives: a signer perfectly at
        // the frontier still answers only the last 16 blocks. Trusting a
        // passed `ServingHistory` here would reintroduce the exact sentence
        // that was wrong on 2026-09-13.
        (true, Some(true), _) => ArchiveService::SignerLiveWindowOnly,
        (true, _, None) => ArchiveService::Unknown,
        (true, _, Some(a)) => a.clone(),
    };

    NodeRole {
        validation_mode: validation_mode(status),
        holds_signing_key,
        advertises_consensus,
        advertises_archive,
        reachable_inbound: connections_in > 0,
        inbound: connections_in,
        uptime_secs,
        blocks_behind,
        archive,
        archive_pending_restart,
        signed_recent: None,
    }
}

impl NodeRole {
    /// Attach what the signed-block window says. Separate from [`node_role`]
    /// because the window is read on a different cadence from the facts above
    /// and is simply absent on the many nodes that hold no key.
    pub fn with_signed_recent(
        mut self,
        signed_recent: Option<crate::signer::SignedRecent>,
    ) -> Self {
        self.signed_recent = signed_recent;
        self
    }

    /// Holds a key AND validates: the only state in which the key produces
    /// signatures that mirrors can follow. The close dialog and the welcome
    /// panel ask this.
    pub fn signs_for_mirrors(&self) -> bool {
        self.holds_signing_key == Some(true) && self.validates_independently()
    }

    /// The archive verdict this role reports, after reconciliation.
    pub fn archive_service(&self) -> &ArchiveService {
        &self.archive
    }

    /// Validates the proof itself AND is allowed to: `consensus` mode with the
    /// consensus bit on the wire. The only state in which a key signs
    /// anything, and the only one in which "full node" means what people
    /// think it means.
    pub fn validates_independently(&self) -> bool {
        self.validation_mode == ValidationMode::Consensus && self.advertises_consensus
    }

    /// `consensus` mode without the consensus bit: the degraded start. Follows
    /// headers, cannot validate or serve new blocks, stalls below Epoch A.
    pub fn started_degraded(&self) -> bool {
        self.validation_mode == ValidationMode::Consensus && !self.advertises_consensus
    }

    /// One line per fact, in the order a person should read them: what the
    /// node does, what its key does, what it serves, whether anyone can reach
    /// it, and where it stands on the chain.
    pub fn lines(&self) -> Vec<RoleLine> {
        vec![
            self.validation_line(),
            self.key_line(),
            self.archive_line(),
            self.reach_line(),
            self.position_line(),
        ]
    }

    fn validation_line(&self) -> RoleLine {
        let (value, helps, note) = match self.validation_mode {
            ValidationMode::Consensus if self.advertises_consensus => (
                "Checking every block itself",
                Some(true),
                "Validates the proof of work on its own hardware and takes nobody's word for \
                 the chain. This is the job a full node exists for."
                    .to_string(),
            ),
            ValidationMode::Consensus => (
                "Started degraded",
                Some(false),
                "Started without a graphics card the engine accepts, so it follows headers \
                 but cannot validate new blocks or serve them, and it stalls below the Epoch \
                 A height. In this state it does not help the network."
                    .to_string(),
            ),
            ValidationMode::Trusted => (
                "Following signed attestations",
                Some(false),
                "Follows the chain through attestations signed by other nodes instead of \
                 checking the proof itself, and produces none of its own. It can still pass \
                 blocks on; the lines below say whether it does."
                    .to_string(),
            ),
            ValidationMode::Relay => (
                "Relay only",
                Some(false),
                "Introduces peers to each other and serves no blocks.".to_string(),
            ),
            ValidationMode::Unknown => (
                "Unknown",
                None,
                "The engine did not say how it validates, so nothing here is evidence either \
                 way."
                    .to_string(),
            ),
        };
        RoleLine {
            label: "Validation",
            value: value.to_string(),
            helps,
            note,
        }
    }

    fn key_line(&self) -> RoleLine {
        let (value, helps, note) = match (self.holds_signing_key, self.validation_mode) {
            (None, _) => (
                "Unknown".to_string(),
                None,
                "The engine did not answer. No answer is not evidence of no key.".to_string(),
            ),
            (Some(false), _) => (
                "None".to_string(),
                None,
                "Ordinary. Most nodes hold none, and a node without a key can serve the full \
                 range of history, which is what the network is short of."
                    .to_string(),
            ),
            // The 2026-09-03 case. The key was pinned in good faith and it
            // never had a chance: a mirror is on the receiving end of
            // signatures by construction.
            (Some(true), ValidationMode::Trusted) => (
                "Present, signing nothing".to_string(),
                Some(false),
                "This node follows attestations rather than producing them, so the key signs \
                 nothing here. A key only produces signatures on a node that validates blocks \
                 itself."
                    .to_string(),
            ),
            // The signer. What the window says outranks what the config says:
            // a configured key that is on none of the recent blocks is not
            // helping anyone yet, and the line must not read "helps" on the
            // strength of a setting.
            (Some(true), ValidationMode::Consensus) if self.advertises_consensus => {
                match self.signed_recent {
                    Some(w) if w.seen >= SIGNED_WINDOW_MIN_SEEN && w.signed == 0 => (
                        format!("Present, on none of the last {} blocks", w.seen),
                        Some(false),
                        "This node validates and holds a key, but none of the recent blocks \
                         carries its signature. A key signs only blocks this node fully \
                         validated itself; if it stays at none, the node is still catching up \
                         or the engine is not accepting the card."
                            .to_string(),
                    ),
                    Some(w) if w.seen > 0 => (
                        format!("Signing, on {} of the last {} blocks", w.signed, w.seen),
                        Some(true),
                        "Signs a confirmation for every block it validates. Mirrors that pin \
                         this key follow it instead of checking the proof themselves, so this \
                         is what keeps them moving. At threshold one a pinned key is a full \
                         authority: they take this node's word for it."
                            .to_string(),
                    ),
                    _ => (
                        "Present, signing".to_string(),
                        Some(true),
                        "Signs a confirmation for every block it validates, which the mirrors \
                         that cannot validate follow. How many recent blocks carry it shows \
                         here once the node has read them."
                            .to_string(),
                    ),
                }
            }
            (Some(true), ValidationMode::Consensus) => (
                "Present, signing nothing".to_string(),
                Some(false),
                "A key only signs blocks this node has validated, and in its degraded state it \
                 validates none."
                    .to_string(),
            ),
            (Some(true), ValidationMode::Relay) => (
                "Present, signing nothing".to_string(),
                Some(false),
                "A relay validates nothing, so the key signs nothing.".to_string(),
            ),
            (Some(true), ValidationMode::Unknown) => (
                "Present".to_string(),
                None,
                "Whether it produces signatures depends on how this node validates, which the \
                 engine did not say."
                    .to_string(),
            ),
        };
        RoleLine {
            label: "Signing key",
            value,
            helps,
            note,
        }
    }

    fn archive_line(&self) -> RoleLine {
        // `NotServing` after reconciliation means one of three things, told
        // apart by the wire: the bit is off and the switch agrees (Off); the
        // bit is off and the switch was turned on since the start; the bit is
        // still on and the switch was turned off since the start.
        let pending = self.archive_pending_restart;
        let (value, helps, note) = match &self.archive {
            ArchiveService::NotServing if pending && !self.advertises_archive => (
                "Off until restart".to_string(),
                Some(false),
                "Not serving attestations yet. Serving was switched on in Settings and takes \
                 effect at the next node start."
                    .to_string(),
            ),
            ArchiveService::NotServing if pending => (
                "On until restart".to_string(),
                None,
                "Serving attestations until the next node start, when the switch-off in \
                 Settings takes effect."
                    .to_string(),
            ),
            a @ ArchiveService::NotServing => ("Off".to_string(), Some(false), a.message()),
            a @ ArchiveService::SignerLiveWindowOnly => (
                format!("Last {SIGNER_SERVE_WINDOW_BLOCKS} blocks only"),
                Some(false),
                a.message(),
            ),
            a @ ArchiveService::ServingHistory => {
                ("Full range".to_string(), Some(true), a.message())
            }
            a @ ArchiveService::DegradedToLiveWindow { blocks_behind } => (
                format!("Paused, {blocks_behind} blocks behind"),
                Some(false),
                a.message(),
            ),
            a @ ArchiveService::Unknown => ("On".to_string(), None, a.message()),
        };
        RoleLine {
            label: "Serving history",
            value,
            helps,
            note,
        }
    }

    fn reach_line(&self) -> RoleLine {
        let (value, helps, note) = if self.inbound > 0 {
            (
                format!(
                    "{} {} connected in",
                    self.inbound,
                    if self.inbound == 1 { "node" } else { "nodes" }
                ),
                Some(true),
                "Other nodes can reach this one and take blocks from it.".to_string(),
            )
        } else if self.uptime_secs < REACHABILITY_GRACE_SECS {
            (
                "No inbound connections yet".to_string(),
                None,
                "A reachable node still needs a while to be found. If this line has not \
                 changed after half an hour, it is the router or a firewall, not the node."
                    .to_string(),
            )
        } else {
            (
                "No inbound connections".to_string(),
                Some(false),
                format!(
                    "Nothing this node holds is on offer until another node can reach it. That \
                     is almost always the router or a firewall rather than the node: forwarding \
                     port {P2P_PORT} to this machine is usually the whole fix."
                ),
            )
        };
        RoleLine {
            label: "Reachable",
            value,
            helps,
            note,
        }
    }

    fn position_line(&self) -> RoleLine {
        let (value, helps, note) = match self.blocks_behind {
            None => (
                "Unknown".to_string(),
                None,
                "Waiting for the node to report its height.".to_string(),
            ),
            Some(0) => (
                "At the tip".to_string(),
                Some(true),
                "Holds every block it knows about, so what it serves is current.".to_string(),
            ),
            Some(n) if n < HEADERS_AHEAD_IS_BEHIND => (
                "At the tip, one block arriving".to_string(),
                Some(true),
                "One block is on its way, which is normal between blocks.".to_string(),
            ),
            Some(n) => (
                format!("{n} blocks behind"),
                Some(false),
                "Knows about blocks it does not have yet, so what it serves is not current. \
                 Leaving it running and connected is usually all it needs."
                    .to_string(),
            ),
        };
        RoleLine {
            label: "Chain position",
            value,
            helps,
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Service words as `getnetworkinfo` prints them: 16 hex chars.
    /// NETWORK | WITNESS | NETWORK_LIMITED | P2P_V2 | MATMUL_CONSENSUS.
    const CONSENSUS_BITS: &str = "0000000008000c09";
    /// The same with MATMUL_ATTESTATION_ARCHIVE (bit 31) as well.
    const CONSENSUS_ARCHIVE_BITS: &str = "0000000088000c09";
    /// NETWORK | WITNESS | NETWORK_LIMITED | P2P_V2 | ARCHIVE, no consensus:
    /// what a trusted mirror that serves attestations advertises.
    const ARCHIVE_ONLY_BITS: &str = "0000000080000c09";
    /// NETWORK | WITNESS | NETWORK_LIMITED | P2P_V2 and nothing MatMul: the
    /// degraded consensus start on a host without a qualifying card.
    const PLAIN_BITS: &str = "0000000000000c09";

    fn status(mode: &str, key: bool) -> MatmulTrustedStatus {
        MatmulTrustedStatus {
            local_signer: key,
            serves_attestations: true,
            matmul_validation_mode: mode.to_string(),
            trusted_mirror: mode == "trusted",
        }
    }

    fn line<'a>(lines: &'a [RoleLine], label: &str) -> &'a RoleLine {
        lines
            .iter()
            .find(|l| l.label == label)
            .unwrap_or_else(|| panic!("no line labelled {label}"))
    }

    /// (a) The healthy full node: consensus mode, the consensus bit on the
    /// wire, somebody connected in, at the tip. Every line helps.
    #[test]
    fn a_validating_reachable_node_helps_on_every_line() {
        let r = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_ARCHIVE_BITS,
            &[],
            3,
            3600,
            Some(0),
            Some(&ArchiveService::ServingHistory),
        );
        assert_eq!(r.validation_mode, ValidationMode::Consensus);
        assert!(r.validates_independently());
        assert!(!r.started_degraded());
        assert_eq!(r.holds_signing_key, Some(false));
        assert!(r.advertises_consensus);
        assert!(r.advertises_archive);
        assert!(r.reachable_inbound);

        let lines = r.lines();
        assert_eq!(lines.len(), 5);
        assert_eq!(line(&lines, "Validation").helps, Some(true));
        assert_eq!(line(&lines, "Serving history").helps, Some(true));
        assert_eq!(line(&lines, "Reachable").helps, Some(true));
        assert_eq!(line(&lines, "Chain position").helps, Some(true));
        // No key is ordinary, not a shortfall: neutral, never "does not help".
        assert_eq!(line(&lines, "Signing key").helps, None);
        assert!(
            !lines.iter().any(|l| l.helps == Some(false)),
            "nothing on a healthy node may read as not helping: {lines:#?}"
        );
    }

    /// (b) The 2026-09-03 case. A key pinned on a trusted mirror signed
    /// nothing for eleven days, and no surface said so. This is the surface.
    #[test]
    fn a_key_on_a_trusted_mirror_is_reported_as_signing_nothing() {
        let r = node_role(
            Some(&status("trusted", true)),
            ARCHIVE_ONLY_BITS,
            &[],
            2,
            86_400,
            Some(0),
            Some(&ArchiveService::ServingHistory),
        );
        assert_eq!(r.validation_mode, ValidationMode::Trusted);
        assert_eq!(r.holds_signing_key, Some(true));
        assert!(!r.validates_independently());

        let lines = r.lines();
        let v = line(&lines, "Validation");
        assert_eq!(v.helps, Some(false));
        assert!(v.note.contains("produces none of its own"), "{}", v.note);

        let k = line(&lines, "Signing key");
        assert_eq!(k.helps, Some(false));
        assert!(k.value.contains("signing nothing"), "{}", k.value);
        assert!(k.note.contains("signs nothing here"), "{}", k.note);
    }

    /// A trusted mirror WITHOUT a key is described the same way on the
    /// validation line and is neutral on the key line: the mirror is an honest
    /// role, the key is what was misunderstood.
    #[test]
    fn a_keyless_mirror_is_not_blamed_for_its_key() {
        let lines = node_role(
            Some(&status("trusted", false)),
            ARCHIVE_ONLY_BITS,
            &[],
            1,
            86_400,
            Some(0),
            Some(&ArchiveService::ServingHistory),
        )
        .lines();
        assert_eq!(line(&lines, "Validation").helps, Some(false));
        assert_eq!(line(&lines, "Signing key").helps, None);
        assert_eq!(line(&lines, "Signing key").value, "None");
        assert_eq!(line(&lines, "Serving history").helps, Some(true));
    }

    /// (c) The signer. Holds a key, validates, advertises the archive bit: the
    /// engine clamps what it serves to the last 16 blocks, and the network is
    /// short of keyless archives. The 2026-09-13 refusal, in one line.
    #[test]
    fn a_signer_serves_only_the_last_sixteen_blocks() {
        let r = node_role(
            Some(&status("consensus", true)),
            CONSENSUS_ARCHIVE_BITS,
            &[],
            5,
            86_400,
            Some(0),
            // The refresher's own verdict already says so; the role agrees.
            Some(&ArchiveService::SignerLiveWindowOnly),
        );
        assert!(r.validates_independently());
        assert_eq!(r.archive_service(), &ArchiveService::SignerLiveWindowOnly);

        let lines = r.lines();
        let k = line(&lines, "Signing key");
        assert_eq!(k.helps, Some(true), "a signer that validates does sign");
        let a = line(&lines, "Serving history");
        assert_eq!(a.helps, Some(false));
        assert!(a.value.contains("16"), "{}", a.value);
        assert!(a.note.contains("last 16 blocks"), "{}", a.note);
        assert!(a.note.contains("KEYLESS"), "{}", a.note);
    }

    /// The ordering rule, mirrored from frontier.rs: holding a key outranks
    /// chain position AND outranks whatever the refresher passed in. A signer
    /// at the frontier with a `ServingHistory` verdict in hand must still read
    /// "last 16 blocks", because the clamp does not depend on either.
    #[test]
    fn holding_a_key_outranks_chain_position_and_the_passed_verdict() {
        for behind in [None, Some(0), Some(1), Some(10_000)] {
            for passed in [
                None,
                Some(ArchiveService::ServingHistory),
                Some(ArchiveService::DegradedToLiveWindow { blocks_behind: 7 }),
                Some(ArchiveService::Unknown),
            ] {
                let r = node_role(
                    Some(&status("consensus", true)),
                    CONSENSUS_ARCHIVE_BITS,
                    &[],
                    5,
                    86_400,
                    behind,
                    passed.as_ref(),
                );
                assert_eq!(
                    r.archive_service(),
                    &ArchiveService::SignerLiveWindowOnly,
                    "signer verdict must not depend on blocks_behind ({behind:?}) or on the \
                     passed verdict ({passed:?})"
                );
                let lines = r.lines();
                let a = line(&lines, "Serving history");
                assert_eq!(a.helps, Some(false));
                assert!(a.value.contains("16"), "{}", a.value);
            }
        }
    }

    /// Not advertising the bit outranks the key, exactly as frontier.rs has it:
    /// a signer that was never asked to serve promises nothing.
    #[test]
    fn a_signer_that_does_not_advertise_the_bit_is_just_not_serving() {
        let r = node_role(
            Some(&status("consensus", true)),
            CONSENSUS_BITS,
            &[],
            5,
            86_400,
            Some(0),
            Some(&ArchiveService::NotServing),
        );
        assert_eq!(r.archive_service(), &ArchiveService::NotServing);
        let lines = r.lines();
        let a = line(&lines, "Serving history");
        assert_eq!(a.value, "Off");
        assert!(!a.note.contains("16"), "{}", a.note);
    }

    /// (d) The degraded consensus start: mode `consensus`, no consensus bit.
    /// Its operator currently believes it helps. It does not, and the line
    /// says what it actually does: follows headers, stalls below Epoch A.
    #[test]
    fn consensus_mode_without_the_bit_is_a_degraded_start() {
        let r = node_role(
            Some(&status("consensus", false)),
            PLAIN_BITS,
            &[],
            4,
            86_400,
            Some(0),
            Some(&ArchiveService::NotServing),
        );
        assert_eq!(r.validation_mode, ValidationMode::Consensus);
        assert!(!r.advertises_consensus);
        assert!(r.started_degraded());
        assert!(!r.validates_independently());

        let lines = r.lines();
        let v = line(&lines, "Validation");
        assert_eq!(v.value, "Started degraded");
        assert_eq!(v.helps, Some(false));
        for fact in ["follows headers", "cannot validate", "Epoch A"] {
            assert!(v.note.contains(fact), "{fact} missing: {}", v.note);
        }
    }

    /// A key on a degraded node signs nothing either: it can only sign what it
    /// has validated, and it validates nothing past the stall.
    #[test]
    fn a_key_on_a_degraded_node_signs_nothing() {
        let k_line = node_role(
            Some(&status("consensus", true)),
            PLAIN_BITS,
            &[],
            4,
            86_400,
            Some(0),
            None,
        )
        .lines();
        let k = line(&k_line, "Signing key");
        assert_eq!(k.helps, Some(false));
        assert!(k.value.contains("signing nothing"), "{}", k.value);
    }

    /// The signer role's own line: what the window says outranks what the
    /// config says. This is the surface that was missing on 2026-09-03 (a key
    /// pinned in good faith, zero signatures, eleven days, nobody told) and
    /// the one the 2026-09-16 outage asks for: "signing: your key was on N of
    /// the last 100 blocks", so an operator knows they are actually helping.
    #[test]
    fn a_validating_signer_says_how_many_recent_blocks_carry_its_key() {
        use crate::signer::SignedRecent;
        let signer = || {
            node_role(
                Some(&status("consensus", true)),
                CONSENSUS_BITS,
                &[],
                5,
                86_400,
                Some(0),
                Some(&ArchiveService::NotServing),
            )
        };
        assert!(signer().signs_for_mirrors());
        assert!(!node_role(
            Some(&status("trusted", true)),
            ARCHIVE_ONLY_BITS,
            &[],
            5,
            86_400,
            Some(0),
            None
        )
        .signs_for_mirrors());

        // Window not read yet: the key is present and the line promises the
        // count rather than inventing one.
        let k = line(&signer().lines(), "Signing key").clone();
        assert_eq!(k.value, "Present, signing");
        assert_eq!(k.helps, Some(true));

        // The good case, with the trust statement in it.
        let r = signer().with_signed_recent(Some(SignedRecent {
            signed: 97,
            seen: 100,
            window: 100,
        }));
        let k = line(&r.lines(), "Signing key").clone();
        assert_eq!(k.value, "Signing, on 97 of the last 100 blocks");
        assert_eq!(k.helps, Some(true));
        assert!(k.note.contains("full authority"), "{}", k.note);
        assert!(k.note.contains("keeps them moving"), "{}", k.note);

        // Just after a start: a handful of blocks read, none signed yet, is
        // not a verdict.
        let r = signer().with_signed_recent(Some(SignedRecent {
            signed: 0,
            seen: SIGNED_WINDOW_MIN_SEEN - 1,
            window: 100,
        }));
        let k = line(&r.lines(), "Signing key").clone();
        assert_eq!(k.helps, Some(true), "{}", k.value);
        assert!(
            k.value.starts_with("Signing, on 0 of the last"),
            "{}",
            k.value
        );

        // Enough blocks read and none carry the key: says so, and does not
        // claim to help on the strength of a setting.
        let r = signer().with_signed_recent(Some(SignedRecent {
            signed: 0,
            seen: 40,
            window: 100,
        }));
        let k = line(&r.lines(), "Signing key").clone();
        assert_eq!(k.value, "Present, on none of the last 40 blocks");
        assert_eq!(k.helps, Some(false));
        assert!(k.note.contains("catching up"), "{}", k.note);

        // The window never changes what a non-validating key reads as.
        let mirror = node_role(
            Some(&status("trusted", true)),
            ARCHIVE_ONLY_BITS,
            &[],
            5,
            86_400,
            Some(0),
            None,
        )
        .with_signed_recent(Some(SignedRecent {
            signed: 50,
            seen: 100,
            window: 100,
        }));
        let k = line(&mirror.lines(), "Signing key").clone();
        assert_eq!(k.value, "Present, signing nothing");
        assert_eq!(k.helps, Some(false));
    }

    /// (e) Not reachable inbound, after the grace period: the line names the
    /// router or firewall and the port, and never the node.
    #[test]
    fn no_inbound_after_the_grace_period_names_the_router_not_the_node() {
        let r = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_BITS,
            &[],
            0,
            REACHABILITY_GRACE_SECS,
            Some(0),
            Some(&ArchiveService::NotServing),
        );
        assert!(!r.reachable_inbound);
        let lines = r.lines();
        let l = line(&lines, "Reachable");
        assert_eq!(l.value, "No inbound connections");
        assert_eq!(l.helps, Some(false));
        assert!(l.note.contains("router"), "{}", l.note);
        assert!(l.note.contains("firewall"), "{}", l.note);
        assert!(l.note.contains("19335"), "{}", l.note);
        assert!(!l.note.contains("broken"), "{}", l.note);
    }

    /// In the first half hour, no inbound is not yet a verdict. Same grace as
    /// the contribution card, so the two never contradict each other.
    #[test]
    fn no_inbound_in_the_first_half_hour_is_not_yet_a_verdict() {
        let l_lines = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_BITS,
            &[],
            0,
            REACHABILITY_GRACE_SECS - 1,
            Some(0),
            None,
        )
        .lines();
        let l = line(&l_lines, "Reachable");
        assert_eq!(l.helps, None, "a fresh run must not raise an alarm");
        assert!(l.value.contains("yet"), "{}", l.value);
    }

    #[test]
    fn one_inbound_peer_is_reachable_and_the_grammar_holds() {
        let one = node_role(None, PLAIN_BITS, &[], 1, 0, None, None);
        assert!(one.reachable_inbound);
        assert_eq!(line(&one.lines(), "Reachable").value, "1 node connected in");
        let many = node_role(None, PLAIN_BITS, &[], 7, 0, None, None);
        assert_eq!(
            line(&many.lines(), "Reachable").value,
            "7 nodes connected in"
        );
        assert_eq!(line(&many.lines(), "Reachable").helps, Some(true));
    }

    /// (f) The engine did not answer `getmatmultrustedstatus`. Everything that
    /// depends on it is unknown and neutral; absence is never evidence. The
    /// facts that come from `getnetworkinfo` are still reported.
    #[test]
    fn an_unanswered_rpc_is_unknown_not_a_verdict() {
        let r = node_role(
            None,
            CONSENSUS_ARCHIVE_BITS,
            &[],
            2,
            86_400,
            Some(0),
            Some(&ArchiveService::ServingHistory),
        );
        assert_eq!(r.validation_mode, ValidationMode::Unknown);
        assert_eq!(r.holds_signing_key, None);
        // The wire still speaks for itself.
        assert!(r.advertises_consensus);
        assert!(r.advertises_archive);

        let lines = r.lines();
        assert_eq!(line(&lines, "Validation").value, "Unknown");
        assert_eq!(line(&lines, "Validation").helps, None);
        assert_eq!(line(&lines, "Signing key").value, "Unknown");
        assert_eq!(line(&lines, "Signing key").helps, None);
        assert!(
            line(&lines, "Signing key")
                .note
                .contains("not evidence of no key"),
            "{}",
            line(&lines, "Signing key").note
        );
        // Without a key answer the passed frontier verdict stands.
        assert_eq!(line(&lines, "Serving history").helps, Some(true));
    }

    /// A mode string this app does not know is unknown too, never guessed.
    #[test]
    fn an_unrecognised_mode_is_unknown() {
        let r = node_role(
            Some(&status("economic", false)),
            PLAIN_BITS,
            &[],
            0,
            0,
            None,
            None,
        );
        assert_eq!(r.validation_mode, ValidationMode::Unknown);
        // An engine reporting the flag without the string is still a mirror.
        let mut s = status("", false);
        s.trusted_mirror = true;
        assert_eq!(
            node_role(Some(&s), PLAIN_BITS, &[], 0, 0, None, None).validation_mode,
            ValidationMode::Trusted
        );
        assert_eq!(
            node_role(
                Some(&status("Consensus ", false)),
                PLAIN_BITS,
                &[],
                0,
                0,
                None,
                None
            )
            .validation_mode,
            ValidationMode::Consensus
        );
    }

    #[test]
    fn relay_mode_serves_no_blocks() {
        let lines = node_role(
            Some(&status("relay", true)),
            "0000000200000808",
            &[],
            9,
            86_400,
            Some(0),
            None,
        )
        .lines();
        assert_eq!(line(&lines, "Validation").value, "Relay only");
        assert_eq!(line(&lines, "Validation").helps, Some(false));
        assert_eq!(line(&lines, "Signing key").helps, Some(false));
    }

    /// Bits decide; names are only an exact-match fallback when the engine
    /// omitted `localservices`. Same rule as node_api::advertises_archive.
    #[test]
    fn service_bits_outrank_names_and_names_are_only_a_fallback() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // A btxd that predates the names renders both bits UNKNOWN[…]; the
        // bits still say consensus + archive.
        let r = node_role(
            None,
            CONSENSUS_ARCHIVE_BITS,
            &names(&["NETWORK", "UNKNOWN[2^27]", "UNKNOWN[2^31]"]),
            0,
            0,
            None,
            None,
        );
        assert!(r.advertises_consensus);
        assert!(r.advertises_archive);
        // Names contradicting present bits lose.
        let r = node_role(
            None,
            PLAIN_BITS,
            &names(&["MATMUL_CONSENSUS", "MATMUL_ATTESTATION_ARCHIVE"]),
            0,
            0,
            None,
            None,
        );
        assert!(!r.advertises_consensus);
        assert!(!r.advertises_archive);
        // No hex at all: the exact names carry it.
        let r = node_role(
            None,
            "",
            &names(&["NETWORK", "MATMUL_CONSENSUS"]),
            0,
            0,
            None,
            None,
        );
        assert!(r.advertises_consensus);
        assert!(!r.advertises_archive);
        // A substring is not a name.
        let r = node_role(
            None,
            "",
            &names(&["LEGACY_ARCHIVED_INDEX"]),
            0,
            0,
            None,
            None,
        );
        assert!(!r.advertises_archive);
        // `0x` is tolerated, as it is everywhere else in this crate.
        assert!(node_role(None, "0x08000000", &[], 0, 0, None, None).advertises_consensus);
    }

    /// The archive bit on the wire is what other nodes see. When the app's
    /// serving switch has been flipped since the node started, the two
    /// disagree until the next start, and the line says so rather than
    /// judging a state that is about to end.
    #[test]
    fn a_serving_switch_flipped_since_start_is_reported_as_pending_restart() {
        // Switched ON while running: the refresher judges the frontier, the
        // wire still has no bit. Nobody is asking this node for history yet.
        let on_pending = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_BITS,
            &[],
            2,
            86_400,
            Some(0),
            Some(&ArchiveService::ServingHistory),
        );
        assert_eq!(on_pending.archive_service(), &ArchiveService::NotServing);
        let on_lines = on_pending.lines();
        let a = line(&on_lines, "Serving history");
        assert_eq!(a.value, "Off until restart");
        assert_eq!(a.helps, Some(false));
        assert!(a.note.contains("next node start"), "{}", a.note);

        // Switched OFF while running: the bit is still on the wire, the
        // refresher stopped reading the frontier. Neutral, and named.
        let off_pending = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_ARCHIVE_BITS,
            &[],
            2,
            86_400,
            Some(0),
            Some(&ArchiveService::NotServing),
        );
        let off_lines = off_pending.lines();
        let a = line(&off_lines, "Serving history");
        assert_eq!(a.value, "On until restart");
        assert_eq!(a.helps, None);

        // Before the refresher's first tick nothing is pending: the bit is on
        // and the frontier is simply not read yet.
        let first_tick = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_ARCHIVE_BITS,
            &[],
            2,
            10,
            Some(0),
            None,
        );
        assert_eq!(first_tick.archive_service(), &ArchiveService::Unknown);
        assert_eq!(line(&first_tick.lines(), "Serving history").value, "On");
        assert_eq!(line(&first_tick.lines(), "Serving history").helps, None);
    }

    /// The degraded-to-live-window verdict passes through with its number, so
    /// the operator sees how far behind, and the sentence is frontier.rs's own.
    #[test]
    fn a_degraded_archive_passes_through_with_its_lag() {
        let lines = node_role(
            Some(&status("consensus", false)),
            CONSENSUS_ARCHIVE_BITS,
            &[],
            2,
            86_400,
            Some(9),
            Some(&ArchiveService::DegradedToLiveWindow { blocks_behind: 9 }),
        )
        .lines();
        let a = line(&lines, "Serving history");
        assert_eq!(a.value, "Paused, 9 blocks behind");
        assert_eq!(a.helps, Some(false));
        assert_eq!(
            a.note,
            ArchiveService::DegradedToLiveWindow { blocks_behind: 9 }.message()
        );
    }

    #[test]
    fn chain_position_tolerates_one_block_in_flight_and_not_two() {
        let pos = |behind: Option<u64>| {
            let lines = node_role(None, PLAIN_BITS, &[], 0, 0, behind, None).lines();
            line(&lines, "Chain position").clone()
        };
        assert_eq!(pos(Some(0)).value, "At the tip");
        assert_eq!(pos(Some(0)).helps, Some(true));
        assert_eq!(
            pos(Some(1)).helps,
            Some(true),
            "one header ahead is a block in flight"
        );
        assert_eq!(pos(Some(2)).helps, Some(false));
        assert_eq!(pos(Some(2)).value, "2 blocks behind");
        assert_eq!(pos(Some(12_345)).value, "12345 blocks behind");
        assert!(pos(Some(12_345)).note.contains("Leaving it running"));
        assert_eq!(pos(None).value, "Unknown");
        assert_eq!(pos(None).helps, None);
    }

    /// Every sentence is for a person. No wire names, no engine identifiers,
    /// a full stop at the end, and no blame.
    #[test]
    fn every_line_is_a_plain_sentence_with_no_jargon_leaking() {
        let statuses = [
            None,
            Some(status("consensus", false)),
            Some(status("consensus", true)),
            Some(status("trusted", false)),
            Some(status("trusted", true)),
            Some(status("relay", true)),
            Some(status("economic", false)),
        ];
        let bits = [
            CONSENSUS_BITS,
            CONSENSUS_ARCHIVE_BITS,
            ARCHIVE_ONLY_BITS,
            PLAIN_BITS,
        ];
        let verdicts = [
            None,
            Some(ArchiveService::ServingHistory),
            Some(ArchiveService::NotServing),
            Some(ArchiveService::Unknown),
            Some(ArchiveService::DegradedToLiveWindow { blocks_behind: 3 }),
        ];
        for s in &statuses {
            for b in bits {
                for v in &verdicts {
                    for (inbound, up) in [(0, 0), (0, 86_400), (3, 86_400)] {
                        for behind in [None, Some(0), Some(1), Some(40)] {
                            let lines =
                                node_role(s.as_ref(), b, &[], inbound, up, behind, v.as_ref())
                                    .lines();
                            assert_eq!(lines.len(), 5);
                            for l in &lines {
                                assert!(!l.value.is_empty());
                                assert!(l.note.ends_with('.'), "{}: {}", l.label, l.note);
                                for jargon in [
                                    "bit 27",
                                    "bit 31",
                                    "NODE_MATMUL",
                                    "GETMMATTEST",
                                    "localservices",
                                    "getmatmultrustedstatus",
                                    "RPC",
                                    "broken",
                                    "fault",
                                ] {
                                    assert!(
                                        !l.note.contains(jargon) && !l.value.contains(jargon),
                                        "{jargon} must not reach a user: {l:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The wire shape the UI declares in apps/node/src/main.ts:
    ///
    /// ```text
    /// role: { validation_mode: string; holds_signing_key: boolean | null;
    ///         advertises_consensus: boolean; advertises_archive: boolean;
    ///         reachable_inbound: boolean; inbound: number; uptime_secs: number;
    ///         blocks_behind: number | null;
    ///         signed_recent: { signed: number; seen: number; window: number } | null } | null;
    /// role_lines: { label: string; value: string; helps: boolean | null; note: string }[];
    /// ```
    ///
    /// Asserted as WHOLE objects, as frontier.rs does, so a field quietly added
    /// or a private field leaking onto the wire fails here rather than in a
    /// language no compiler checks against this one.
    #[test]
    fn the_wire_shape_is_what_the_ui_declares() {
        let r = node_role(
            Some(&status("trusted", true)),
            ARCHIVE_ONLY_BITS,
            &[],
            2,
            600,
            Some(1),
            Some(&ArchiveService::ServingHistory),
        );
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({
                "validation_mode": "trusted",
                "holds_signing_key": true,
                "advertises_consensus": false,
                "advertises_archive": true,
                "reachable_inbound": true,
                "inbound": 2,
                "uptime_secs": 600,
                "blocks_behind": 1,
                "signed_recent": null,
            })
        );
        let unknown = node_role(None, "", &[], 0, 0, None, None);
        let v = serde_json::to_value(&unknown).unwrap();
        assert_eq!(v["validation_mode"], "unknown");
        assert!(v["holds_signing_key"].is_null());
        assert!(v["blocks_behind"].is_null());
        let signing = r
            .clone()
            .with_signed_recent(Some(crate::signer::SignedRecent {
                signed: 97,
                seen: 100,
                window: 100,
            }));
        assert_eq!(
            serde_json::to_value(&signing).unwrap()["signed_recent"],
            serde_json::json!({ "signed": 97, "seen": 100, "window": 100 })
        );

        let lines = r.lines();
        let l = &lines[1];
        assert_eq!(
            serde_json::to_value(l).unwrap(),
            serde_json::json!({
                "label": "Signing key",
                "value": "Present, signing nothing",
                "helps": false,
                "note": l.note.clone(),
            })
        );
        assert!(serde_json::to_value(&lines[4]).unwrap()["helps"].is_boolean());
        assert!(serde_json::to_value(&unknown.lines()[0]).unwrap()["helps"].is_null());
    }
}
