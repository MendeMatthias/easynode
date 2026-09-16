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
    /// True once the contribution migration has run on this install.
    ///
    /// 0.6.24 turned the three cheap services on for NEW installs only, which
    /// was the cautious choice and the wrong outcome: the census on
    /// 2026-09-16 saw 59 nodes and only 21 serving attestations. An update
    /// that leaves an existing node contributing nothing is most of the fleet.
    ///
    /// So this runs once per install and turns on the services whose key is
    /// ABSENT from the settings file, which is the honest test for "never
    /// chose". A key that is present and `false` was set by somebody on
    /// purpose and is left alone: `#[serde(default)]` maps both to `false`, so
    /// the struct cannot tell them apart and the raw JSON has to be read.
    ///
    /// ⚠ The two defaults are the OPPOSITE pairing from `welcome_shown`, and
    /// getting it backwards silently does nothing: the serde default is
    /// `false`, so a settings file written before this field existed reads as
    /// "not yet migrated" and gets the services. The struct default is `true`,
    /// because a brand new install already has them from `Default` and must
    /// not be migrated on top. A test pins both directions; the first draft of
    /// this had `default_true` on the serde side, which would have skipped
    /// every existing install, which is the entire population this is for.
    #[serde(default)]
    pub contribution_migrated: bool,
    /// True once the signer migration has run on this install. Same pairing
    /// as `contribution_migrated` and for the same reason: serde `false` so a
    /// file from before the field is migrated, struct `true` so a new install
    /// (which already signs from `Default`) is not migrated on top of it.
    ///
    /// A second flag rather than a reuse of the first: every 0.6.25 install
    /// has `contribution_migrated: true` already, and the whole population
    /// this is for is exactly those installs.
    #[serde(default)]
    pub signer_migrated: bool,
    /// True once the one-time welcome panel has been shown.
    ///
    /// The two defaults differ ON PURPOSE and it is the whole trick. The
    /// struct's `Default` is `false`, and `load` reaches it only when there is
    /// no settings file, so a brand new install sees the panel. The serde
    /// default is `true`, which fills the field in for a file that predates
    /// it, so nobody who already runs a node is shown a welcome for a node
    /// they set up weeks ago.
    #[serde(default = "default_true")]
    pub welcome_shown: bool,
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
    /// protocol.
    ///
    /// ON by default since 2026-09-16, which is the flip this comment used to
    /// promise "once the fleet's serve path is field-proven". It is: 18
    /// archives have served attestations since 0.6.21 and the signer link
    /// stopped being a single point because of them. A node that keeps this
    /// to itself costs the network the one thing it is short of.
    ///
    /// The default reaches NEW installs only. `NodeAppSettings::load` falls
    /// back to `Default` just when no settings file exists, and every field is
    /// `#[serde(default)]`, so an existing user's saved choice is read back
    /// unchanged and a field they never had stays `false`. Nobody's node
    /// starts serving because they updated.
    #[serde(default)]
    pub attestation_serve_enabled: bool,
    /// Sign confirmations for mirrors (`btx_core::signer`): on a node that
    /// validates, keep a signing key and hand it to the engine, which then
    /// signs an attestation for every block it validates. The mirrors that
    /// pin the key (btxscan.io's explorer, the wallets behind it, every
    /// GPU-less easyNode) follow those signatures instead of the proof.
    ///
    /// ON by default, and the one default here that the 2026-09-16 outage was
    /// about: the whole network's mirrors were following ONE key on one home
    /// computer, and when it was switched off at 15:23Z the explorer froze
    /// while the chain went on. Every node with a card the engine accepts is
    /// asked to do what that machine does. A node that mirrors (no CUDA
    /// driver, a refused Mac) cannot sign, and the setting does nothing there;
    /// the UI says so rather than showing a switch that lies.
    ///
    /// The migration (`migrate_signer`) turns it on for existing installs
    /// that were never asked, the same rule as `contribution_migrated`, and
    /// the welcome panel says so on the next launch. An explicit `false` is a
    /// choice and stays.
    ///
    /// What is being asked of the operator is trust, not bandwidth: at
    /// threshold one a pinned key is a full authority, and the Settings copy
    /// says so next to the public key.
    #[serde(default)]
    pub signer_enabled: bool,
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
    /// Serve the Esplora REST API to wallets: electrs and the Caddy front run
    /// beside btxd (`btx_core::esplora_sidecar`). Off by default. Refused on a
    /// pruned datadir by `btx_core::esplora`, and the refusal is shown where
    /// the switch is, never logged away.
    #[serde(default)]
    pub esplora_enabled: bool,
    /// Where the front listens. Plain HTTP on localhost until an operator
    /// gives it a hostname, which gets automatic HTTPS.
    #[serde(default = "default_esplora_listen")]
    pub esplora_listen: String,
    /// Serve the two routes a wallet needs to settle a fork
    /// (`btx_core::witness`). Unlike Esplora mode it needs no second binary
    /// and no particular prune posture: the server is compiled into this app
    /// and reads block hashes from the node's index, which every node has.
    ///
    /// ON by default for new installs since 2026-09-16. It binds
    /// `WITNESS_ADDR`, `127.0.0.1:3081`, so it opens NO port to the network
    /// and reaches only this machine: a wallet running here can settle a fork
    /// against a node its owner runs instead of trusting someone's server.
    /// Serving other machines still means changing the bind to `0.0.0.0`,
    /// which stays an explicit choice because there is no proxy in front of it
    /// (see the header of `btx_core::witness`).
    #[serde(default)]
    pub witness_enabled: bool,
    /// Where the witness binds. Loopback by default: accepting connections
    /// from outside is a deliberate choice, not something a toggle does behind
    /// somebody's back.
    #[serde(default = "default_witness_listen")]
    pub witness_listen: String,
    /// When the last self-update check finished (RFC 3339, UTC), how it ended
    /// (one of `update_log::UPDATE_CHECK_OUTCOMES`), and the short detail the
    /// front end recorded with it. `None`/empty until the first check has
    /// finished. Written by `record_update_check` at every exit of the front
    /// end's `updateCheck()` and by the six-hourly timer in `update_timer`, so
    /// the Settings pane can say what the automatic path did, and can still
    /// say it after the relaunch that an install causes. The per-check history is `<datadir>/update-check.log`; this is
    /// only the last line of it, kept where the pane can read it without
    /// parsing a file. See `update_log` for the eight silent hours behind it.
    #[serde(default)]
    pub last_update_check_at: Option<String>,
    #[serde(default)]
    pub last_update_check_outcome: Option<String>,
    #[serde(default)]
    pub last_update_check_detail: String,
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

fn default_esplora_listen() -> String {
    btx_core::esplora_sidecar::DEFAULT_LISTEN.to_string()
}

fn default_witness_listen() -> String {
    btx_core::witness::WITNESS_ADDR.to_string()
}

impl Default for NodeAppSettings {
    fn default() -> Self {
        Self {
            setup_complete: false,
            // A fresh machine gets the services from these defaults, so it
            // needs no migration; it only needs the panel.
            contribution_migrated: true,
            signer_migrated: true,
            // A fresh machine has not seen it. See the field's docs for why
            // this disagrees with the serde default.
            welcome_shown: false,
            snapshot_loaded: false,
            btx_release_tag: None,
            keep_awake: true,
            txindex_enabled: false,
            wallet_enabled: false,
            wallet_name: None,
            wallet_address: None,
            on_close: default_on_close(),
            // The three below are ON for a new install. See each field's
            // docs. All are cheap, none opens a port, and first run shows
            // them so this is opt-out rather than something done quietly.
            attestation_serve_enabled: true,
            // The signer role, on. See the field: this is the one the
            // explorer's freeze on 2026-09-16 was about.
            signer_enabled: true,
            service_report_enabled: true,
            node_profile: default_profile(),
            // No nickname. Anything else would publish an identifier the user
            // never chose to publish.
            node_nickname: String::new(),
            esplora_enabled: false,
            esplora_listen: default_esplora_listen(),
            witness_enabled: true,
            witness_listen: default_witness_listen(),
            // No check has finished yet, and the pane says so in those words.
            last_update_check_at: None,
            last_update_check_outcome: None,
            last_update_check_detail: String::new(),
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

    /// Which of the three cheap services this install has never been asked
    /// about, read from the RAW json.
    ///
    /// This is the whole reason the migration is safe. `#[serde(default)]`
    /// turns both a missing key and an explicit `false` into `false`, so the
    /// struct cannot distinguish "we never offered it" from "they said no".
    /// The file can: a key that is not there was never chosen.
    ///
    /// Returns `(attestations, witness, report)`, true meaning "absent, so
    /// free to turn on". Anything unreadable returns all false: when in doubt,
    /// change nothing on somebody's machine.
    pub fn services_never_chosen(raw: &str) -> (bool, bool, bool) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            return (false, false, false);
        };
        let Some(o) = v.as_object() else {
            return (false, false, false);
        };
        (
            !o.contains_key("attestation_serve_enabled"),
            !o.contains_key("witness_enabled"),
            !o.contains_key("service_report_enabled"),
        )
    }

    /// Turn on the services this install was never asked about, once.
    ///
    /// Returns true when something was turned on, which is the caller's cue to
    /// show the panel: the user is told what changed rather than discovering
    /// it later. A no-op install (already migrated, or every key already set
    /// by hand) returns false and shows nothing.
    pub fn migrate_contributions(datadir: &std::path::Path) -> bool {
        let path = datadir.join(SETTINGS_FILE_NAME);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return false; // no file at all is a new install; Default covers it
        };
        if Self::load(datadir).contribution_migrated {
            return false;
        }
        let (attest, witness, report) = Self::services_never_chosen(&raw);
        let changed = attest || witness || report;
        Self::update(datadir, |s| {
            if attest {
                s.attestation_serve_enabled = true;
            }
            if witness {
                s.witness_enabled = true;
            }
            if report {
                s.service_report_enabled = true;
            }
            s.contribution_migrated = true;
            // Only interrupt somebody if something actually changed.
            if changed {
                s.welcome_shown = false;
            }
        });
        changed
    }

    /// Has this install never been asked about signing? Read from the RAW
    /// json for the reason `services_never_chosen` gives: the struct cannot
    /// tell an absent key from a deliberate `false`, and only the absent one
    /// is ours to fill in. Unreadable means "touch nothing".
    pub fn signer_never_chosen(raw: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.as_object().map(|o| !o.contains_key("signer_enabled")))
            .unwrap_or(false)
    }

    /// Turn signing on for an install that was never asked, once.
    ///
    /// `applies_here` is whether this host will launch as a validator (the
    /// only place a key signs anything, `btx_core::node::launches_as_mirror`
    /// negated). The setting is recorded either way, so a machine that later
    /// gains a card signs without being asked again; the welcome panel is
    /// armed only where the change means something today, because a panel
    /// announcing a role the machine cannot fill would be noise on exactly the
    /// machines that already got the 0.6.25 panel.
    ///
    /// Returns true when the setting was turned on AND the panel was armed.
    pub fn migrate_signer(datadir: &std::path::Path, applies_here: bool) -> bool {
        let path = datadir.join(SETTINGS_FILE_NAME);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return false; // no file at all is a new install; Default covers it
        };
        if Self::load(datadir).signer_migrated {
            return false;
        }
        let never_chosen = Self::signer_never_chosen(&raw);
        let announce = never_chosen && applies_here;
        Self::update(datadir, |s| {
            if never_chosen {
                s.signer_enabled = true;
            }
            s.signer_migrated = true;
            if announce {
                s.welcome_shown = false;
            }
        });
        announce
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
    /// The engine's own answer to `getmatmultrustedstatus`, from the refresher
    /// tick that already makes the call. Until now only `local_signer` was
    /// kept, and only long enough to feed the frontier verdict; the validation
    /// mode and the mirror flag were thrown away, which is why no screen
    /// could tell an operator that a key pinned on a trusted mirror signs
    /// nothing (2026-09-03, eleven days). `None` when the engine does not
    /// know the method, and absence is reported as unknown, never as "no".
    /// Cleared on every stop/start like the others. See `btx_core::role`.
    pub matmul_trusted: Arc<Mutex<Option<btx_core::node_api::MatmulTrustedStatus>>>,
    /// Who signed the newest hundred blocks, from the node's own attestation
    /// store, kept current by the refresher only while the engine reports a
    /// local signer (`btx_core::signer::RecentSigners`). This is how the role
    /// card says "your key was on N of the last 100 blocks" instead of
    /// "a key is configured", which is the sentence that hid the 2026-09-03
    /// key that signed nothing. Cleared on every stop/start like the others.
    pub recent_signers: Arc<Mutex<Option<btx_core::signer::RecentSigners>>>,
    /// The public key of the signing key on disk, read once at start (and
    /// when the switch is flipped) so the status poll does not derive a curve
    /// point from a file every second. `None` when there is no readable key.
    pub signer_pubkey: Arc<Mutex<Option<String>>>,
    /// Whether the last start decided this host validates (a key can sign)
    /// or mirrors (it cannot), from `btx_core::node::launches_as_mirror`.
    /// `None` before the first start of this app run.
    pub signer_applies_here: Arc<Mutex<Option<bool>>>,
    /// The fork detector's verdict — a longer chain this node cannot obtain
    /// blocks for — computed by the refresher from `getchaintips` and the
    /// headers/blocks gap. Cleared on every stop/start like the others, so a
    /// dead run's fork never renders as current. See `btx_core::fork`.
    pub fork: Arc<Mutex<Option<btx_core::fork::ForkAlarm>>>,
    /// `getblockchaininfo.mediantime` from the last successful poll, or `None`
    /// when the node is stopped or has not answered yet.
    ///
    /// WHY THIS IS SEPARATE FROM `fork`. Every other chain signal this app
    /// shows is derived from the node's peers: `blocks`, `headers`,
    /// `getchaintips`, and therefore `fork` too. When a node and all of its
    /// peers are stuck together, every one of those agrees that nothing is
    /// wrong — which is exactly what happened to the explorer on 2026-09-13,
    /// where its own health check printed GREEN for twenty-one hours.
    ///
    /// A block timestamp cannot be fooled that way: it comes from the chain,
    /// not from agreement among the peers we happen to be talking to. So this
    /// is the one staleness signal in the app that is not self-referential,
    /// and `node_api::tip_is_stale` has always been the right check for it —
    /// it was simply only ever wired into the wallet panel. See
    /// `node_api.rs:36`: "it can be hours dead without `is_stale` if it
    /// believes its own tip."
    ///
    /// Stored raw rather than pre-judged so the verdict is computed against
    /// the clock at render time, never against the clock at poll time.
    pub tip_median_time: Arc<Mutex<Option<i64>>>,
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
    /// electrs and the Caddy front, while Esplora mode is on and the node
    /// runs; `None` otherwise. Dropping the value kills both children.
    pub esplora: Arc<Mutex<Option<btx_core::esplora_sidecar::EsploraSidecars>>>,
    /// The freshness guardian's last verdict for the served endpoint. Cleared
    /// with the sidecars, so a dead front never renders as fresh.
    pub esplora_verdict: Arc<Mutex<Option<btx_core::esplora_freshness::Verdict>>>,
    /// Why the front is not running although the setting is on: the prune
    /// gate's sentence, a missing binary, a child that exited. Shown beside
    /// the switch. `None` when running or off.
    pub esplora_error: Arc<Mutex<Option<String>>>,
    /// Generation counter for the guardian loop, bumped on every sidecar
    /// start; a loop whose generation is superseded exits.
    pub esplora_gen: Arc<AtomicU64>,
    /// The fork-witness server, while it is on and the node runs.
    pub witness: Arc<Mutex<Option<btx_core::witness::WitnessServer>>>,
    /// Why the witness is not running although the setting is on. Shown beside
    /// the switch; `None` when it is running or off.
    pub witness_error: Arc<Mutex<Option<String>>>,
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
            matmul_trusted: Arc::new(Mutex::new(None)),
            recent_signers: Arc::new(Mutex::new(None)),
            signer_pubkey: Arc::new(Mutex::new(None)),
            signer_applies_here: Arc::new(Mutex::new(None)),
            fork: Arc::new(Mutex::new(None)),
            tip_median_time: Arc::new(Mutex::new(None)),
            archive_peers_cache: Arc::new(Mutex::new(None)),
            peer_nicknames_cache: Arc::new(Mutex::new(Vec::new())),
            esplora: Arc::new(Mutex::new(None)),
            esplora_verdict: Arc::new(Mutex::new(None)),
            esplora_error: Arc::new(Mutex::new(None)),
            esplora_gen: Arc::new(AtomicU64::new(0)),
            witness: Arc::new(Mutex::new(None)),
            witness_error: Arc::new(Mutex::new(None)),
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
        assert!(!s.esplora_enabled, "Esplora mode is opt-in, never default");
        // Changed 2026-09-16, deliberately. A new install now contributes the
        // three things that are cheap and expose nothing, and first run shows
        // them so it is opt-out. Esplora stays opt-in: it needs a second
        // binary, the full chain and an index.
        assert!(
            s.attestation_serve_enabled,
            "a new node serves attestations: the scarcest service on the network, ~208 bytes/block"
        );
        assert!(
            s.service_report_enabled,
            "the service report is written locally and uploaded nowhere"
        );
        assert!(
            s.witness_enabled,
            "a new node answers block-hash questions so a wallet here can settle a fork against it"
        );
        assert_eq!(
            s.witness_listen,
            btx_core::witness::WITNESS_ADDR,
            "the witness binds loopback until somebody chooses otherwise"
        );
        assert!(
            !btx_core::witness::is_public_bind(&s.witness_listen),
            "ENABLED is not EXPOSED: the witness runs by default but must never \
             accept connections from outside until someone chooses that"
        );
        assert_eq!(
            s.esplora_listen,
            btx_core::esplora_sidecar::DEFAULT_LISTEN,
            "the front starts on localhost until an operator names it"
        );
    }

    /// The defaults above reach NEW installs only. This is the half that
    /// protects everyone who already runs one: updating the app must never
    /// start a service on somebody's machine because we changed our minds.
    ///
    /// `load` uses `Default` only when there is no settings file. Every field
    /// is `#[serde(default)]`, which fills a MISSING field with the field
    /// type's default, `false` for a bool, and not with the value in our
    /// `Default` impl. So an old file, and a file that predates these fields
    /// entirely, both read back with the services off.
    #[test]
    fn an_existing_install_is_never_switched_on_by_an_update() {
        // A settings file from before these fields existed.
        let old: NodeAppSettings =
            serde_json::from_str(r#"{"setup_complete":true,"keep_awake":true}"#).unwrap();
        assert!(old.setup_complete, "this is an existing user");
        assert!(
            !old.attestation_serve_enabled,
            "an update must not start serving attestations for them"
        );
        assert!(
            !old.witness_enabled,
            "an update must not start the witness for them"
        );
        assert!(
            !old.service_report_enabled,
            "an update must not start writing reports"
        );

        // And a user who said no explicitly still gets no.
        let refused: NodeAppSettings = serde_json::from_str(
            r#"{"setup_complete":true,"attestation_serve_enabled":false,"witness_enabled":false}"#,
        )
        .unwrap();
        assert!(
            !refused.attestation_serve_enabled,
            "their choice is read back, not overridden"
        );
        assert!(
            !refused.witness_enabled,
            "their choice is read back, not overridden"
        );

        // A fresh machine, by contrast, contributes.
        let fresh = NodeAppSettings::default();
        assert!(fresh.attestation_serve_enabled && fresh.witness_enabled);
    }

    /// The migration turns on what was never chosen, and nothing else.
    ///
    /// This is the half that decides whether the change is trustworthy: an
    /// explicit `false` in the file is a person saying no, and no update may
    /// overturn it.
    #[test]
    fn the_migration_respects_a_deliberate_no() {
        // Never asked: all three keys absent.
        let (a, w, r) =
            NodeAppSettings::services_never_chosen(r#"{"setup_complete":true,"keep_awake":true}"#);
        assert!(a && w && r, "absent keys are free to turn on");

        // Said no to serving, never asked about the other two.
        let (a, w, r) = NodeAppSettings::services_never_chosen(
            r#"{"setup_complete":true,"attestation_serve_enabled":false}"#,
        );
        assert!(
            !a,
            "an explicit false is a choice and must survive the update"
        );
        assert!(
            w && r,
            "the keys they were never asked about are still free"
        );

        // Already serving: present and true, so not ours to touch either.
        let (a, _, _) =
            NodeAppSettings::services_never_chosen(r#"{"attestation_serve_enabled":true}"#);
        assert!(!a, "present means chosen, whatever the value");

        // Unreadable: change nothing.
        let (a, w, r) = NodeAppSettings::services_never_chosen("{ not json");
        assert!(!a && !w && !r, "when in doubt, touch nothing");
    }

    /// It runs once. A second launch must not re-enable what somebody turned
    /// off in between, which is the obvious way a migration becomes a bug.
    #[test]
    fn the_migration_runs_once_and_then_leaves_people_alone() {
        let fresh = NodeAppSettings::default();
        assert!(
            fresh.contribution_migrated,
            "a new install gets the services from Default and needs no migration"
        );
        let old: NodeAppSettings = serde_json::from_str(r#"{"setup_complete":true}"#).unwrap();
        assert!(
            !old.contribution_migrated,
            "a file predating the flag has not run it"
        );
        let done: NodeAppSettings =
            serde_json::from_str(r#"{"setup_complete":true,"contribution_migrated":true}"#)
                .unwrap();
        assert!(done.contribution_migrated, "and it does not run twice");
    }

    /// The signer migration: on for everyone who was never asked, a deliberate
    /// `false` kept, once only, and the panel armed only where a key can sign.
    #[test]
    fn the_signer_migration_turns_signing_on_once_and_respects_a_no() {
        assert!(
            NodeAppSettings::default().signer_enabled,
            "a new install signs"
        );
        assert!(
            NodeAppSettings::default().signer_migrated,
            "and needs no migration on top of that"
        );
        let old: NodeAppSettings = serde_json::from_str(r#"{"setup_complete":true}"#).unwrap();
        assert!(
            !old.signer_migrated,
            "a file predating the flag has not run it"
        );
        assert!(
            !old.signer_enabled,
            "and reads as off until the migration runs"
        );

        assert!(NodeAppSettings::signer_never_chosen(
            r#"{"setup_complete":true,"contribution_migrated":true}"#
        ));
        assert!(!NodeAppSettings::signer_never_chosen(
            r#"{"signer_enabled":false}"#
        ));
        assert!(!NodeAppSettings::signer_never_chosen(
            r#"{"signer_enabled":true}"#
        ));
        assert!(!NodeAppSettings::signer_never_chosen("{ nope"));

        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join(SETTINGS_FILE_NAME);

        // A 0.6.25 install on a validating host: turned on, told.
        std::fs::write(
            &settings_path,
            r#"{"setup_complete":true,"contribution_migrated":true,"welcome_shown":true}"#,
        )
        .unwrap();
        assert!(NodeAppSettings::migrate_signer(dir.path(), true));
        let s = NodeAppSettings::load(dir.path());
        assert!(s.signer_enabled && s.signer_migrated && !s.welcome_shown);
        // Once: switching it off afterwards is a choice the next launch keeps.
        NodeAppSettings::update(dir.path(), |s| {
            s.signer_enabled = false;
            s.welcome_shown = true;
        });
        assert!(!NodeAppSettings::migrate_signer(dir.path(), true));
        let s = NodeAppSettings::load(dir.path());
        assert!(!s.signer_enabled && s.welcome_shown);

        // The same install on a host that mirrors: the setting is recorded
        // for the day it gains a card, but nobody is interrupted about it.
        std::fs::write(
            &settings_path,
            r#"{"setup_complete":true,"contribution_migrated":true,"welcome_shown":true}"#,
        )
        .unwrap();
        assert!(!NodeAppSettings::migrate_signer(dir.path(), false));
        let s = NodeAppSettings::load(dir.path());
        assert!(s.signer_enabled && s.signer_migrated && s.welcome_shown);

        // A deliberate no survives.
        std::fs::write(
            &settings_path,
            r#"{"setup_complete":true,"signer_enabled":false,"welcome_shown":true}"#,
        )
        .unwrap();
        assert!(!NodeAppSettings::migrate_signer(dir.path(), true));
        let s = NodeAppSettings::load(dir.path());
        assert!(!s.signer_enabled && s.signer_migrated && s.welcome_shown);

        // No settings file: a new install, nothing to migrate.
        let empty = tempfile::tempdir().unwrap();
        assert!(!NodeAppSettings::migrate_signer(empty.path(), true));
        assert!(!empty.path().join(SETTINGS_FILE_NAME).exists());
    }

    /// The welcome panel shows once, to a new install, and never to somebody
    /// who set their node up before it existed.
    #[test]
    fn the_welcome_panel_is_for_new_installs_only() {
        assert!(
            !NodeAppSettings::default().welcome_shown,
            "no settings file means a brand new install: show it"
        );
        let existing: NodeAppSettings =
            serde_json::from_str(r#"{"setup_complete":true,"keep_awake":true}"#).unwrap();
        assert!(
            existing.welcome_shown,
            "a file that predates the field belongs to someone already running a node"
        );
        let seen: NodeAppSettings =
            serde_json::from_str(r#"{"setup_complete":true,"welcome_shown":true}"#).unwrap();
        assert!(seen.welcome_shown, "and it does not come back");
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
