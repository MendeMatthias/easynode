//! easyBTX Node app state: the persisted settings file, the phase model the
//! UI polls, and the shared runtime state.
//!
//! DATADIR SHARING: the node app uses the SAME datadir as the miner
//! (`~/.easybtx`, override-aware via `~/.easybtx-location` — see
//! `btx_core::datadir`). Its own persisted settings therefore live in a
//! SEPARATE file, `easybtx-node-app.json`, so it never touches the miner's
//! `easybtx-state.json` (which carries wallet/payout state).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tokio::sync::Mutex;

use btx_core::node::NodeController;
use btx_core::power::SleepAssertion;
use btx_core::rpc::RpcClient;

/// Serialize writes to the settings file (load-modify-save races between async
/// commands would otherwise lose updates). Same pattern as the miner's
/// STATE_FILE_LOCK.
pub static SETTINGS_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub const SETTINGS_FILE_NAME: &str = "easybtx-node-app.json";

/// The active datadir for the node app. `EASYBTX_NODE_DATADIR` overrides for
/// tests/e2e runs (a throwaway dir keeps the real shared `~/.easybtx` — which
/// may hold a live chain and the miner's wallets — untouched); otherwise the
/// shared override-aware resolution from btx-core.
pub fn node_datadir() -> PathBuf {
    if let Ok(p) = std::env::var("EASYBTX_NODE_DATADIR") {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    btx_core::datadir::easybtx_datadir()
}

/// Persisted app settings. Every field `#[serde(default)]` so any older/missing
/// file loads cleanly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeAppSettings {
    /// True once first-run setup completed (binaries provisioned + node started
    /// once). A returning user skips the wizard and auto-starts the node.
    #[serde(default)]
    pub setup_complete: bool,
    /// True once `loadtxoutset` has SUCCEEDED against this datadir (or the
    /// chain advanced past the snapshot). Gates snapshot.dat reclaim — see
    /// btx_core::snapshot (C3).
    #[serde(default)]
    pub snapshot_loaded: bool,
    /// The BTX release tag whose binaries we launch (install dir key).
    #[serde(default)]
    pub btx_release_tag: Option<String>,
    /// Hold a "don't idle-sleep" power assertion while the node runs, so the
    /// node keeps supporting the network when the user walks away. Display
    /// sleep is never blocked. Default ON (that's the app's whole purpose);
    /// visible toggle in Settings for laptop users who prefer sleep.
    #[serde(default = "default_true")]
    pub keep_awake: bool,
    /// Explorer mode: the node maintains a full transaction index
    /// (`txindex=1`) so historical txid lookups answer locally. Off by
    /// default — enabling is a deliberate, reversible user choice.
    #[serde(default)]
    pub txindex_enabled: bool,
    /// Wallet view: OFF from factory settings — no wallet surface exists
    /// until the user flips the Settings toggle. Not a default surface.
    #[serde(default)]
    pub wallet_enabled: bool,
    /// The btxd wallet name created by a `.btxwallet` import (None = nothing
    /// imported yet). The wallet itself lives in the node's wallet dir.
    #[serde(default)]
    pub wallet_name: Option<String>,
    /// The wallet's first receive address (display only; set at create/import).
    #[serde(default)]
    pub wallet_address: Option<String>,
    /// What the window's red X does: "ask" (prompt each time — default), "tray"
    /// (hide, node keeps running), or "quit" (stop the node and quit). Set by
    /// the close dialog's "remember my choice". Any unknown value is treated as
    /// "ask" by the close handler, so a hand-edited file can't wedge the app.
    #[serde(default = "default_on_close")]
    pub on_close: String,
    /// Serve historical attestations to the network
    /// (`matmulattestationserve=1`). The single scarcest service on today's
    /// network (census 2026-08-17: one reachable full-history archive
    /// network-wide) and cheap to give: ~208 bytes/block, rate-limited by
    /// protocol. Opt-in at first per Paper 3 patch 7; a later release can
    /// flip the default once the fleet's serve path is field-proven.
    #[serde(default)]
    pub attestation_serve_enabled: bool,
    /// Write a local `service-report.json` next to the datadir every few
    /// minutes: uptime, heights, peers, bytes served, archive-peer summary,
    /// stall verdict. LOCAL FILE ONLY — nothing phones home; this is the
    /// opt-in seed for a future Keepers dashboard that READS it. Off by
    /// default.
    #[serde(default)]
    pub service_report_enabled: bool,
    /// Which node this app runs: "full" (whole chain, ~124 GiB measured
    /// 2026-09-04) or "keeper"
    /// (pruned ~10 GB, serves signed confirmations). The CHOICE persists here;
    /// whether the keeper conf actually activates is the engine gate
    /// (`installer::conf_for_profile`) — an old bundled btxd provisions the
    /// safe full conf and the UI says the choice arrives with the next engine
    /// update. Default "full": existing installs keep exactly their behavior.
    #[serde(default = "default_profile")]
    pub node_profile: String,
    /// Optional public nickname, broadcast to every peer as the user agent
    /// comment: `/BTX:0.34.6(yourname)/`. Empty = no nickname, which is the
    /// default and must stay the default.
    ///
    /// This is the one setting in this struct that OTHER PEOPLE can see. It is
    /// a persistent public identifier that follows the node across restarts and
    /// IP changes, so it is opt-in, easy to clear, and the UI says what it does
    /// before it is set rather than after. Validation lives in
    /// `btx_core::nickname`, deliberately stricter than btxd's, because btxd
    /// refuses to START on a comment it does not like.
    #[serde(default)]
    pub node_nickname: String,
}

fn default_on_close() -> String {
    "ask".to_string()
}

fn default_profile() -> String {
    "full".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for NodeAppSettings {
    fn default() -> Self {
        Self {
            setup_complete: false,
            snapshot_loaded: false,
            btx_release_tag: None,
            keep_awake: true,
            txindex_enabled: false,
            wallet_enabled: false,
            wallet_name: None,
            wallet_address: None,
            on_close: default_on_close(),
            attestation_serve_enabled: false,
            service_report_enabled: false,
            node_profile: default_profile(),
            // No nickname. Anything else would publish an identifier the user
            // never chose to publish.
            node_nickname: String::new(),
        }
    }
}

impl NodeAppSettings {
    pub fn load(datadir: &std::path::Path) -> Self {
        match std::fs::read_to_string(datadir.join(SETTINGS_FILE_NAME)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, datadir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(datadir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        // Atomic. `load` maps any unreadable or unparseable file to defaults,
        // so a torn settings file does not fail loudly — it silently resets
        // every choice the user has made, including which wallet the panel
        // points at and whether the node serves at all.
        btx_core::fsx::atomic_write(&datadir.join(SETTINGS_FILE_NAME), json.as_bytes())
    }

    /// Load-modify-save under the settings lock.
    pub fn update(datadir: &std::path::Path, f: impl FnOnce(&mut NodeAppSettings)) {
        let _g = SETTINGS_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Self::load(datadir);
        f(&mut s);
        if let Err(e) = s.save(datadir) {
            eprintln!("[settings] could not persist {SETTINGS_FILE_NAME}: {e}");
        }
    }
}

/// `btx_core::snapshot::SnapshotFlags` backed by this app's settings file.
pub struct NodeAppSnapshotFlags {
    pub datadir: PathBuf,
}

impl btx_core::snapshot::SnapshotFlags for NodeAppSnapshotFlags {
    fn loaded(&self) -> bool {
        NodeAppSettings::load(&self.datadir).snapshot_loaded
    }
    fn mark_loaded(&self) {
        NodeAppSettings::update(&self.datadir, |s| s.snapshot_loaded = true);
    }
}

/// The high-level phase the UI renders, serialized as an internally-tagged
/// enum (`{"phase":"syncing", ...}`) — same convention as the miner's AppPhase.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum NodePhase {
    /// Fresh install: show the wizard.
    Welcome,
    /// Downloading the assumeutxo snapshot (progress 0.0..=1.0).
    Downloading { progress: f64 },
    /// Copying + signing the bundled node binaries, writing the conf.
    Preparing,
    /// btxd spawned; waiting for its RPC to come up.
    Starting,
    /// btxd is alive but answering RPC_IN_WARMUP (-28): verifying blocks /
    /// rebuilding shielded state. A WORKING state, never an error — a long
    /// rebuild used to time into a red "needs attention" card, which reads
    /// as broken while the node is actually busy getting ready.
    Warming { message: String },
    /// Waiting for headers / loading the snapshot into a chainstate.
    LoadingSnapshot,
    /// Node running, chain catching up (progress of the best chainstate).
    ///
    /// `peers` is carried here and not only on [`NodePhase::Ready`] because this
    /// is the LONGEST phase of a first run, roughly two hours of header sync on a
    /// Mac, and it used to render the peer count as an em dash for all of it. A
    /// user watching a working node saw no height and no peers and reasonably
    /// concluded nothing was connected. Measured 2026-08-31 on a live 0.6.12:
    /// the UI showed "PEERS —" while the daemon reported 15 connections.
    Syncing {
        height: u64,
        headers: u64,
        progress: f64,
        peers: i64,
    },
    /// Node running at/near the tip — helping the network.
    ///
    /// `blocks_behind` is how far the active chain trails the best header we
    /// know about. It is carried because "near tip" is a BOOLEAN with no lag
    /// term in it: `sync_readiness` returns NearTip the moment a snapshot
    /// chainstate loads at the anchor, and the anchor is a fixed height in a
    /// shipped release. On a fresh install the badge therefore flips to LIVE
    /// while the node is still thousands of blocks short, and stays there while
    /// it grinds. The verdict is not changed here — that is a product decision
    /// about what "ready" means — but the number is no longer withheld from the
    /// screen that claims it.
    Ready {
        height: u64,
        peers: i64,
        blocks_behind: u64,
    },
    /// Node deliberately stopped by the user.
    Stopped,
    /// Something failed; message is plain-language and actionable.
    Error { message: String },
}

impl Default for NodePhase {
    fn default() -> Self {
        NodePhase::Welcome
    }
}

/// Shared, thread-safe app state managed by Tauri.
pub struct AppState {
    pub rpc: Arc<Mutex<Option<RpcClient>>>,
    pub node: Arc<Mutex<Option<NodeController>>>,
    pub phase: Arc<Mutex<NodePhase>>,
    /// (btx_cli, datadir) recorded on start — what a graceful stop needs.
    pub launch: Arc<Mutex<Option<(PathBuf, PathBuf)>>>,
    /// Wall-clock start of the current node run (uptime display).
    pub started_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// Held while the node runs && keep_awake is on.
    pub sleep_guard: std::sync::Mutex<Option<SleepAssertion>>,
    /// Cached datadir size (MB, last measured) — a recursive walk of a ~124 GiB
    /// tree is too heavy for the status poll, so it refreshes at most once a
    /// minute, off-thread (see get_node_status). Arc so the walk task can
    /// write the result back without borrowing AppState.
    pub datadir_size_cache: Arc<std::sync::Mutex<(u64, Option<std::time::Instant>)>>,
    /// True while a background size walk is in flight (never start two).
    pub size_walk_running: Arc<AtomicBool>,
    /// Guards against two concurrent setup pipelines (double-click).
    pub setup_running: Arc<AtomicBool>,
    /// True while `start_node_inner` is running. The double-spawn guard keys
    /// on a live child in `state.node`, and there is none for the whole window
    /// between a stop and the next spawn, during which `Stopped` is an
    /// actionable phase on both surfaces: a second Start (tray, button,
    /// explorer toggle) could enter the same start sequence and race the first
    /// for `btxd.pid`. Held through a drop guard, so a panicking start releases
    /// it instead of wedging every later one.
    pub start_in_flight: Arc<AtomicBool>,
    /// Who the node we ATTACHED to belongs to, when we attached rather than
    /// spawned. `None` whenever the node in `state.rpc` is our own child or
    /// there is none. Set on the Attach plan, cleared on every spawn and on
    /// stop. The destructive commands read this: `state.rpc.is_some()` was the
    /// wrong proxy for "ours", because in attach mode that slot holds the
    /// OTHER app's client, which is the exact case the gate exists for.
    pub attached_to: Arc<Mutex<Option<AttachedTo>>>,
    /// Generation counter for the status refresher: each (re)start bumps it and
    /// stale refresher loops exit when their generation is superseded.
    pub refresher_gen: Arc<AtomicU64>,
    /// Set once a graceful quit is under way, so the ExitRequested handler knows
    /// the async shutdown already ran and can let the exit proceed instead of
    /// blocking the main thread a second time (the old force-quit-inducing hang).
    pub quitting: Arc<AtomicBool>,
    /// btxd's MatMul RC execution verdict for the CURRENT node run, remembered
    /// once observed: `(policy, stalled)`.
    ///
    /// btxd logs that verdict ONCE, at startup, after a production canary that
    /// takes minutes. The status poll can only read a bounded tail of the log,
    /// so on a node that has been up for hours the line has scrolled far out of
    /// that window — without this cache the "Block checking" card would simply
    /// vanish on exactly the long-running nodes it exists to describe. Caching
    /// is sound because the verdict is a property of the run: btxd does not
    /// re-qualify mid-run. Cleared on every start/attach so a restarted node is
    /// re-read rather than inheriting a stale answer.
    pub rc_status_cache: Arc<Mutex<Option<(Option<btx_core::node::RcExecutionPolicy>, bool)>>>,
    /// The stall discriminator's current verdict (None = healthy / no verdict).
    /// Written by the refresher's watchdog tick, read by get_node_status.
    /// Cleared whenever progress resumes AND on every stop/start, so the UI
    /// never shows a stale stall (a previous run's verdict used to survive a
    /// manual stop/start and render as current on a freshly booted node).
    pub stall_verdict: Arc<Mutex<Option<btx_core::watchdog::StallVerdict>>>,
    /// What this node is really providing to other nodes: computed once per
    /// refresher tick from the signed frontier, and ONLY when attestation
    /// serving is on — a node that does not serve has no frontier question to
    /// answer. Cleared on every stop/start alongside the stall verdict, so a
    /// previous run's answer never renders as current. See `btx_core::frontier`.
    pub archive_service: Arc<Mutex<Option<btx_core::frontier::ArchiveService>>>,
    /// The archive-peer census, computed ONCE per refresher tick from a single
    /// getpeerinfo and shared by the status snapshot, the watchdog and the
    /// service report. The UI poll used to run its own full getpeerinfo every
    /// ~1.5 s on top of the refresher's — three duplicate pipelines for the
    /// same numbers. None when stopped or the node did not answer.
    pub archive_peers_cache: Arc<Mutex<Option<btx_core::node_api::ArchivePeerSummary>>>,
    /// Nicknames of connected peers, from the SAME per-tick getpeerinfo as the
    /// census above. Cached for the same reason: the UI polls ~1.5 s and a
    /// second full getpeerinfo for a decorative list would be indefensible.
    pub peer_nicknames_cache: Arc<Mutex<Vec<String>>>,
}

/// Whose node did we attach to? Derived from the `DatadirHolder` seen at the
/// moment of attaching, and kept as its own small type so the destructive
/// commands can reason about it without re-probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachedTo {
    /// A live btxd with a live parent app: the miner, or another window of
    /// this app. Never ours to stop, and never ours to delete under.
    AnotherApp,
    /// A btxd whose parent is gone: our own previous instance, which this app
    /// adopts after a self-update relaunch. Ours.
    OurOrphan,
    /// RPC answered but the pidfile named nothing we could classify. We are
    /// using a node we did not start and cannot vouch for.
    Unknown,
}

impl AppState {
    /// Clone of the size-cache handle for the background walk task.
    pub fn datadir_size_cache_handle(
        &self,
    ) -> Arc<std::sync::Mutex<(u64, Option<std::time::Instant>)>> {
        self.datadir_size_cache.clone()
    }

    pub fn new() -> Self {
        Self {
            rpc: Arc::new(Mutex::new(None)),
            node: Arc::new(Mutex::new(None)),
            phase: Arc::new(Mutex::new(NodePhase::default())),
            launch: Arc::new(Mutex::new(None)),
            started_at: std::sync::Mutex::new(None),
            sleep_guard: std::sync::Mutex::new(None),
            datadir_size_cache: Arc::new(std::sync::Mutex::new((0, None))),
            size_walk_running: Arc::new(AtomicBool::new(false)),
            setup_running: Arc::new(AtomicBool::new(false)),
            start_in_flight: Arc::new(AtomicBool::new(false)),
            attached_to: Arc::new(Mutex::new(None)),
            refresher_gen: Arc::new(AtomicU64::new(0)),
            quitting: Arc::new(AtomicBool::new(false)),
            rc_status_cache: Arc::new(Mutex::new(None)),
            stall_verdict: Arc::new(Mutex::new(None)),
            archive_service: Arc::new(Mutex::new(None)),
            archive_peers_cache: Arc::new(Mutex::new(None)),
            peer_nicknames_cache: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_keeps_awake_and_needs_setup() {
        let s = NodeAppSettings::default();
        assert!(s.keep_awake, "keep-awake defaults ON (the app's purpose)");
        assert!(!s.setup_complete);
        assert!(!s.snapshot_loaded);
        assert!(s.btx_release_tag.is_none());
        assert!(!s.txindex_enabled, "explorer mode is opt-in, never default");
        assert!(!s.wallet_enabled, "wallet is OFF from factory settings");
        assert!(s.wallet_name.is_none());
        assert_eq!(s.on_close, "ask", "the red X asks until the user decides");
    }

    #[test]
    fn on_close_default_survives_a_legacy_settings_file() {
        // A pre-0.5 settings JSON has no on_close key; serde's default must fill
        // it with "ask" rather than an empty string the close handler can't read.
        let legacy: NodeAppSettings =
            serde_json::from_str(r#"{"setup_complete":true,"keep_awake":true}"#).unwrap();
        assert_eq!(legacy.on_close, "ask");
    }

    #[test]
    fn settings_roundtrip_and_legacy_load() {
        let dir = tempfile::tempdir().unwrap();
        // Missing file → defaults, never an error.
        assert_eq!(
            NodeAppSettings::load(dir.path()),
            NodeAppSettings::default()
        );
        // Roundtrip.
        let mut s = NodeAppSettings::default();
        s.setup_complete = true;
        s.btx_release_tag = Some("v0.32.12".into());
        s.keep_awake = false;
        s.save(dir.path()).unwrap();
        assert_eq!(NodeAppSettings::load(dir.path()), s);
        // A legacy/partial file (missing fields) loads with defaults filled in.
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            r#"{"setup_complete":true}"#,
        )
        .unwrap();
        let legacy = NodeAppSettings::load(dir.path());
        assert!(legacy.setup_complete);
        assert!(legacy.keep_awake, "missing keep_awake defaults true");
        // Corrupt file → defaults, never a panic.
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), b"not-json").unwrap();
        assert_eq!(
            NodeAppSettings::load(dir.path()),
            NodeAppSettings::default()
        );
    }

    #[test]
    fn update_persists_through_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        NodeAppSettings::update(dir.path(), |s| s.snapshot_loaded = true);
        assert!(NodeAppSettings::load(dir.path()).snapshot_loaded);
    }

    #[test]
    fn snapshot_flags_are_backed_by_the_settings_file() {
        use btx_core::snapshot::SnapshotFlags;
        let dir = tempfile::tempdir().unwrap();
        let flags = NodeAppSnapshotFlags {
            datadir: dir.path().to_path_buf(),
        };
        assert!(!flags.loaded());
        flags.mark_loaded();
        assert!(flags.loaded());
        // And it landed in THIS app's file, not the miner's easybtx-state.json.
        assert!(dir.path().join(SETTINGS_FILE_NAME).exists());
        assert!(!dir.path().join("easybtx-state.json").exists());
    }

    #[test]
    fn warming_phase_serializes_with_message() {
        let p = NodePhase::Warming {
            message: "Verifying blocks…".into(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["phase"], "warming");
        assert_eq!(v["message"], "Verifying blocks…");
    }

    #[test]
    fn phase_serializes_tagged_snake_case() {
        let p = NodePhase::Syncing {
            height: 130000,
            headers: 155000,
            progress: 0.97,
            peers: 12,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["phase"], "syncing");
        assert_eq!(v["height"], 130000);
        let r = NodePhase::Ready {
            height: 155052,
            peers: 8,
            blocks_behind: 0,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["phase"], "ready");
        assert_eq!(v["peers"], 8);
    }

    #[test]
    fn node_datadir_honors_env_override() {
        // Serialize env mutation: this is the only test touching this var.
        std::env::set_var("EASYBTX_NODE_DATADIR", "/tmp/ebtx-node-e2e-test");
        assert_eq!(node_datadir(), PathBuf::from("/tmp/ebtx-node-e2e-test"));
        std::env::remove_var("EASYBTX_NODE_DATADIR");
        // Without the override we resolve the shared datadir (ends in easybtx).
        let d = node_datadir();
        assert!(d.to_string_lossy().to_lowercase().contains("easybtx"));
    }
}
