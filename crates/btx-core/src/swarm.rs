//! The swarm (BTX's model network), as this app runs it: off unless the owner
//! switches it on, and observing only when it is on.
//!
//! WHAT THE SWARM IS, read from upstream at 074dac16 (v0.34.16):
//!
//! * It has no switch of its own. An engine built with the model network
//!   registers `-modelnet` (default 1, init.cpp:653) and, while that is on,
//!   starts a `btx-modeld` helper and a thread inside btxd
//!   (`StartSwarmObserver`, init.cpp:4638) that owns a `btx-guardd` child.
//!   `-modelnet=0` stops all three (init.cpp:4446, :4640).
//! * It reports through `getmodelnetworkinfo`, in `native_participation`
//!   (schema `BTX/NativeParticipation/1`, helper.cpp:5605): the peers that
//!   passed the PQ1 handshake, the models each one says it has, and the
//!   "shared model families", which are the models this node holds verified
//!   that a peer says it holds too. Every peer row carries
//!   `compute_authorized: false` and the report `automatic_spend_atoms: "0"`.
//! * The models the node holds are `listmodels`.
//! * The guard's health is in NO command. btxd writes it to
//!   `<modeldir>/swarm/guard-process.json` (schema `BTX/GuardProcess/1`,
//!   swarm_observer.cpp:402) with a `valid_until_ms`, and that file is what
//!   [`read_guard`] reads.
//!
//! Nothing here pays, earns, touches the wallet or the chain. The card says so
//! in those words, and [`Report::observing_only`] is true only while the
//! engine's own report says the same.
//!
//! WHY THE COMMAND LINE: the engine reads a `modelnet=1` from btx.conf and
//! from the folder's own btx_rw.conf, and its default is on. Stated on the
//! command line, which outranks both, the app's switch is the only thing that
//! turns the swarm on under this app, and nothing in a conf can widen what it
//! does once on.

use crate::error::AppError;
use crate::rpc::Rpc;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The file in the node folder that says "the owner switched the swarm on".
/// The app writes or removes it from its setting before every launch
/// ([`set_wanted`]); the launch reads it ([`launch_args`]). A marker in the
/// folder, like the header bootstrap's, because the launch is built from the
/// folder and the engine path alone.
const WANTED_MARKER: &str = "easynode-swarm-on";

/// An option only an engine built with the model network knows. It sits inside
/// `#ifdef ENABLE_MODELNET` (init.cpp:652-673), so its name is in the binary
/// exactly when `-modelnet` is a known argument.
const MODEL_NETWORK_OPTION: &[u8] = b"-modelnetrequired";

fn marker_path(datadir: &Path) -> PathBuf {
    datadir.join(WANTED_MARKER)
}

/// Whether the owner switched the swarm on for this node folder.
pub fn wanted(datadir: &Path) -> bool {
    marker_path(datadir).exists()
}

/// Bring the marker in line with the setting. A folder that cannot take the
/// marker launches with the swarm off, which is the default; a marker that
/// cannot be removed is reported as an error, because then the next launch
/// would start a swarm the owner switched off.
pub fn set_wanted(datadir: &Path, on: bool) -> std::io::Result<()> {
    let path = marker_path(datadir);
    if on {
        std::fs::write(
            &path,
            "The owner switched the swarm (BTX's model network) on in easyNode's\n\
             settings. While this file is here, easyNode starts btxd with the model\n\
             network on, observing only. easyNode writes and removes this file from\n\
             its own setting at every start; deleting it by hand lasts one start.\n",
        )
    } else if path.exists() {
        std::fs::remove_file(&path)
    } else {
        Ok(())
    }
}

/// Whether the btxd at `path` was built with the model network, read from the
/// engine itself and not from its version: a release engine is built with
/// `-DWITH_MODELNET=OFF` at the same version at which upstream's own archive
/// is built with it on, and the two differ in exactly this.
///
/// It looks for the name of an option that only a model-network build
/// registers. An option name is a plain string in the binary: `-swapnet` is
/// found six times in the 0.34.16 engine and never in 0.34.15 (measured
/// 2026-11-25 on the installed engines), the same way.
///
/// Fails to `false` for a file that cannot be read. `false` passes no model
/// option at all, which is what every launch did before this existed and the
/// only thing an engine without the model network accepts: it refuses an
/// unknown argument at init.
pub fn engine_has_model_network(btxd: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(btxd) else {
        return false;
    };
    let needle = MODEL_NETWORK_OPTION;
    let mut buf = vec![0u8; 1 << 20];
    // Bytes kept from the end of the last read, so a name across two reads is
    // still found.
    let mut kept = 0usize;
    loop {
        let n = match file.read(&mut buf[kept..]) {
            Ok(0) => return false,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        let filled = kept + n;
        if buf[..filled].windows(needle.len()).any(|w| w == needle) {
            return true;
        }
        let keep = filled.min(needle.len() - 1);
        buf.copy_within(filled - keep..filled, 0);
        kept = keep;
    }
}

/// What a launch says about the model network.
///
/// * An engine without it: nothing. It does not know the options.
/// * An engine with it, swarm not switched on: `-modelnet=0`. No helper, no
///   swarm thread, no guard, no listener, no LAN beacon (measured: no child
///   process and no listening port; the engine still writes
///   `<datadir>/modelnet/governor-permit.json`). Without the flag such an
///   engine listens on 0.0.0.0:29447 and binds UDP 29448 by default.
/// * Switched on: [`OBSERVING`].
pub fn launch_args(btxd: &Path, datadir: &Path) -> Vec<String> {
    args_for(engine_has_model_network(btxd), wanted(datadir))
}

/// The switched-on launch: the model network on, and every way it could do
/// more than look switched off by name.
///
/// * `-modelhost=0`: this node serves no model files and never advertises
///   itself as a model host.
/// * `-modelrelay=0`: it does not advertise itself as a discovery relay.
/// * `-modelfollowpeers=0`: the engine's default is 1, which downloads the
///   free models a peer announces into spare disk. Nothing is downloaded
///   that the owner did not ask for.
/// * `-modelpreserverare=0`: it does not fetch rare models to keep them alive.
/// * `-modelseed=off`: it does not upload a model it holds.
///
/// Measured on the real engine with exactly this set (two regtest nodes,
/// 2026-11-25): `follow_configured_peers` false, `seed_mode` "off",
/// `relay_status` "off", the helper `READY`, and a peer named with
/// `-modelpeer` authenticated five seconds after start.
///
/// NOT stated, on purpose, so the engine's own rules and the owner's conf
/// decide:
///
/// * `-modelbind`. The engine's default is 0.0.0.0:29447, and off when the
///   node runs with `-listen=0`. The listener is what the swarm needs: LAN
///   discovery only runs while it is open (helper.cpp:12967), and there are no
///   built-in model peers, so without it this node sees nobody and nobody sees
///   it. It answers the PQ1 handshake and, with hosting and seeding off,
///   serves nothing. An owner who wants it narrower sets `modelbind=` in the
///   conf.
/// * `-modellandiscovery`. On by default, and off by default on a node that
///   runs behind `-proxy` or `-connect` (init.cpp:4462). Forcing it on would
///   announce a node on the LAN whose owner routed it through Tor.
/// * `-modelstorage`. A budget and not a use; with the three downloads above
///   off, nothing fills it.
pub const OBSERVING: &[&str] = &[
    "-modelnet=1",
    "-modelhost=0",
    "-modelrelay=0",
    "-modelfollowpeers=0",
    "-modelpreserverare=0",
    "-modelseed=off",
];

fn args_for(engine_has_it: bool, wanted: bool) -> Vec<String> {
    match (engine_has_it, wanted) {
        (false, _) => Vec::new(),
        (true, false) => vec!["-modelnet=0".to_string()],
        (true, true) => OBSERVING.iter().map(|a| a.to_string()).collect(),
    }
}

/// The guard child's health, from the file btxd writes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Guard {
    /// Running and answering, and the file is current.
    Ready,
    /// The file is current and says the guard is not ready.
    NotReady { why: String },
    /// The file is older than btxd says it may be: nothing is known now.
    Stale,
    /// No file, or one this app cannot read.
    Unknown,
}

/// Read `<datadir>/modelnet/swarm/guard-process.json` against `now_ms`.
pub fn read_guard(datadir: &Path, now_ms: u64) -> Guard {
    let path = datadir
        .join("modelnet")
        .join("swarm")
        .join("guard-process.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Guard::Unknown;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return Guard::Unknown;
    };
    guard_from(&v, now_ms)
}

fn guard_from(v: &Value, now_ms: u64) -> Guard {
    if v["schema"].as_str() != Some("BTX/GuardProcess/1") {
        return Guard::Unknown;
    }
    let Some(valid_until) = v["valid_until_ms"].as_u64() else {
        return Guard::Unknown;
    };
    if valid_until < now_ms {
        return Guard::Stale;
    }
    if v["enabled"].as_bool() == Some(true) && v["ready"].as_bool() == Some(true) {
        return Guard::Ready;
    }
    let error = v["error"].as_str().unwrap_or_default().trim();
    let state = v["state"].as_str().unwrap_or_default().trim();
    Guard::NotReady {
        why: if !error.is_empty() {
            error.to_string()
        } else if !state.is_empty() {
            state.to_string()
        } else {
            "not ready".to_string()
        },
    }
}

/// A model this node holds that a peer says it holds too.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SharedFamily {
    pub model_id: String,
    pub peers: u64,
}

/// What a running swarm reports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Models on this node's disk, complete and verified.
    pub models_held: u64,
    /// Peers that passed the PQ1 handshake and are still fresh.
    pub peers_seen: u64,
    pub shared_families: Vec<SharedFamily>,
    /// Whether LAN discovery is running, and the engine's reason if not.
    pub lan_discovery: bool,
    pub lan_error: String,
    pub guard: Guard,
    /// True while the engine's own report says what the card says: no peer is
    /// authorized to compute, nothing is spent, nothing is release-qualified.
    /// False means the engine reports something this app does not describe,
    /// and the card stops saying "observing only".
    pub observing_only: bool,
}

/// What the node says about its model network, asked once.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Seen {
    /// The engine has no model network: it does not know the command.
    NoModelNetwork,
    /// The engine has one and it is off.
    Off,
    /// On, and the helper that runs it is not answering. `why` is the
    /// engine's sentence (a missing `btx-modeld` reads like this).
    HelperDown { why: String },
    /// On and reporting.
    Observing(Report),
    /// On, and the answer is not in a shape this app knows.
    Unreadable { why: String },
    /// The node did not answer at all.
    Unreachable { why: String },
}

/// Ask the node. Two read-only commands, `getmodelnetworkinfo` and, only when
/// the swarm is reporting, `listmodels`; plus the guard's file.
pub async fn read(rpc: &dyn Rpc, datadir: &Path, now_ms: u64) -> Seen {
    let info = match rpc.call("getmodelnetworkinfo", json!([])).await {
        Ok(v) => v,
        // JSON-RPC "method not found": the build has no model network.
        Err(AppError::Rpc { code: -32601, .. }) => return Seen::NoModelNetwork,
        Err(e) => return Seen::Unreachable { why: e.to_string() },
    };
    if info["enabled"].as_bool() == Some(false) {
        return Seen::Off;
    }
    if info["helper_ready"].as_bool() != Some(true) {
        let why = ["helper_error", "error"]
            .iter()
            .filter_map(|k| info[*k].as_str())
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("the model helper is not answering")
            .to_string();
        return Seen::HelperDown { why };
    }
    let native = &info["native_participation"];
    if native["schema"].as_str() != Some("BTX/NativeParticipation/1") {
        return Seen::Unreadable {
            why: match native["schema"].as_str() {
                Some(other) => format!("the engine reports its swarm as {other}"),
                None => "the engine's answer has no swarm report".to_string(),
            },
        };
    }
    let models_held = match rpc.call("listmodels", json!([])).await {
        Ok(v) => v["models"]
            .as_array()
            .map(|a| a.iter().filter(|m| holds(m)).count() as u64)
            .unwrap_or(0),
        Err(e) => return Seen::Unreachable { why: e.to_string() },
    };
    let peers = native["peer_observations"].as_array();
    let shared_families = native["shared_model_families"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(SharedFamily {
                        model_id: f["model_id"].as_str()?.to_string(),
                        peers: f["peer_transport_pins"].as_array()?.len() as u64,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let nobody_computes = peers
        .map(|a| {
            a.iter()
                .all(|p| p["compute_authorized"].as_bool() == Some(false))
        })
        .unwrap_or(true);
    Seen::Observing(Report {
        models_held,
        peers_seen: native["authenticated_peers"]
            .as_u64()
            .unwrap_or_else(|| peers.map(|a| a.len() as u64).unwrap_or(0)),
        shared_families,
        lan_discovery: native["lan_enabled"].as_bool() == Some(true),
        lan_error: native["lan_error"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string(),
        guard: read_guard(datadir, now_ms),
        observing_only: nobody_computes
            && native["automatic_spend_atoms"].as_str() == Some("0")
            && native["release_qualified"].as_bool() != Some(true),
    })
}

/// The same test the engine applies before it counts a model as held
/// (helper.cpp:5611).
fn holds(model: &Value) -> bool {
    model["model_id"].is_string()
        && model["bytes_verified"].as_bool() == Some(true)
        && model["incomplete"].as_bool() != Some(true)
}

/// The file in the node folder that says "the swarm engine failed here, and
/// this node runs on its usual engine until its owner asks for the swarm
/// engine again". A file and not a variable, so it holds across a restart of
/// the app: without it every start would try the engine that just failed,
/// fall back, and try it again at the next one.
const HELD_BACK_MARKER: &str = "easynode-swarm-engine-held-back";

/// The longest reason kept. A launch error quotes the engine's own log line.
const HELD_BACK_WHY_MAX: usize = 600;

/// Why the swarm engine is held back, and which version of the app held it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBack {
    pub app_version: String,
    pub why: String,
}

fn held_back_path(datadir: &Path) -> PathBuf {
    datadir.join(HELD_BACK_MARKER)
}

/// Record that the swarm engine failed, with the reason on one line. Two
/// lines: `app <version>`, then the reason.
pub fn hold_engine_back(datadir: &Path, app_version: &str, why: &str) -> std::io::Result<()> {
    let one_line = why.split_whitespace().collect::<Vec<_>>().join(" ");
    let why: String = one_line.chars().take(HELD_BACK_WHY_MAX).collect();
    std::fs::write(
        held_back_path(datadir),
        format!("app {}\n{why}\n", app_version.trim()),
    )
}

/// Whether the swarm engine is held back in this node folder, and why. The
/// file being there is the fact: one that does not read still holds, with no
/// reason, because the safe side is the engine every install has.
pub fn engine_held_back(datadir: &Path) -> Option<HeldBack> {
    let path = held_back_path(datadir);
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines = text.lines();
    let app_version = lines
        .next()
        .and_then(|l| l.strip_prefix("app "))
        .unwrap_or_default()
        .trim()
        .to_string();
    let why = lines.next().unwrap_or_default().trim().to_string();
    Some(HeldBack { app_version, why })
}

/// Let the swarm engine be tried again. Only the owner does this: the switch
/// in Settings, and a fresh install of the swarm engine.
pub fn clear_engine_hold(datadir: &Path) -> std::io::Result<()> {
    let path = held_back_path(datadir);
    if path.exists() {
        std::fs::remove_file(&path)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_node::FakeNode;

    /// The hold is a file in the node folder: it is there for the next start
    /// and for the next run of the app, until it is cleared.
    #[test]
    fn a_held_back_engine_stays_held_until_it_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(engine_held_back(dir.path()), None);
        // Nothing to clear is not an error.
        clear_engine_hold(dir.path()).unwrap();

        hold_engine_back(
            dir.path(),
            " 0.8.1 ",
            "the node kept exiting right after launch:\n  Error: no such option.  See the log.",
        )
        .unwrap();
        let held = HeldBack {
            app_version: "0.8.1".into(),
            why: "the node kept exiting right after launch: Error: no such option. See the log."
                .into(),
        };
        // Read twice: reading does not use it up.
        assert_eq!(engine_held_back(dir.path()), Some(held.clone()));
        assert_eq!(engine_held_back(dir.path()), Some(held));
        // It is its own file, not the switch's marker.
        assert!(!wanted(dir.path()));
        set_wanted(dir.path(), true).unwrap();
        set_wanted(dir.path(), false).unwrap();
        assert!(engine_held_back(dir.path()).is_some());

        clear_engine_hold(dir.path()).unwrap();
        assert_eq!(engine_held_back(dir.path()), None);
    }

    #[test]
    fn a_hold_that_does_not_read_still_holds_and_a_long_reason_is_cut() {
        let dir = tempfile::tempdir().unwrap();
        let nothing = Some(HeldBack {
            app_version: String::new(),
            why: String::new(),
        });
        std::fs::write(held_back_path(dir.path()), b"").unwrap();
        assert_eq!(engine_held_back(dir.path()), nothing);
        std::fs::write(held_back_path(dir.path()), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(engine_held_back(dir.path()), nothing);

        hold_engine_back(dir.path(), "0.8.1", &"x".repeat(5000)).unwrap();
        let held = engine_held_back(dir.path()).unwrap();
        assert_eq!(held.why.chars().count(), HELD_BACK_WHY_MAX);
        assert_eq!(held.app_version, "0.8.1");
    }

    const NOW: u64 = 1_800_000_000_000;

    fn node() -> FakeNode {
        FakeNode::new(&"00".repeat(32), 10)
    }

    /// `getmodelnetworkinfo` as helper.cpp:6881 and rpc/modelnet.cpp:213
    /// write it on a node whose swarm is on, with `native` as the report.
    fn info(native: Value) -> Value {
        json!({
            "schema_version": 2,
            "enabled": true,
            "helper_ready": true,
            "pq1_ready": true,
            "error": "",
            "transport": "pq1",
            "automatic_spend_atoms": 0,
            "follow_configured_peers": false,
            "peers_connected": 0,
            "native_participation": native,
            "helper_managed_by_btxd": true,
            "helper_restart_count": 0,
            "helper_state": "ready"
        })
    }

    /// `native_participation` as helper.cpp:5605 writes it.
    fn native(peers: Value, families: Value) -> Value {
        let n = peers.as_array().map(|a| a.len()).unwrap_or(0);
        json!({
            "schema": "BTX/NativeParticipation/1",
            "network": "main",
            "lan_enabled": true,
            "lan_error": "",
            "hint_count": n,
            "authenticated_peers": n,
            "peer_observations": peers,
            "shared_model_families": families,
            "acquired_models": 0,
            "automatic_spend_atoms": "0",
            "release_qualified": false
        })
    }

    fn peer(pin: &str, models: &[&str]) -> Value {
        json!({
            "endpoint": "192.168.1.20:29447",
            "transport_pin": pin,
            "observed_at_ms": NOW - 1000,
            "expires_at_ms": NOW + 60_000,
            "advertised_models": models,
            "compute_authorized": false
        })
    }

    fn guard_file(dir: &Path, body: Value) {
        let swarm = dir.join("modelnet").join("swarm");
        std::fs::create_dir_all(&swarm).unwrap();
        std::fs::write(swarm.join("guard-process.json"), body.to_string()).unwrap();
    }

    fn guard_ready() -> Value {
        json!({
            "schema": "BTX/GuardProcess/1", "enabled": true, "ready": true,
            "state": "READY", "pid": 4242, "restart_count": 0, "error": "",
            "automatic_spend_atoms": "0", "execution_authorized": false,
            "observed_at_ms": NOW - 2000, "valid_until_ms": NOW + 28_000
        })
    }

    #[tokio::test]
    async fn an_engine_without_the_model_network_says_so_and_is_asked_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let n = node(); // `modelnet: None` is an engine that lacks the command
        assert_eq!(read(&n, dir.path(), NOW).await, Seen::NoModelNetwork);
        assert_eq!(n.methods(), vec!["getmodelnetworkinfo"]);
    }

    /// The setting off on an engine that has the model network: the launch
    /// said `-modelnet=0`, the helper never started, and btxd answers from
    /// `LocalNetworkInfo` (rpc/modelnet.cpp:92) with `enabled: false`.
    #[tokio::test]
    async fn an_engine_with_it_and_the_setting_off_reads_as_off() {
        let dir = tempfile::tempdir().unwrap();
        let n = node();
        n.with(|s| {
            s.modelnet = Some(json!({
                "schema_version": 2, "enabled": false, "helper_ready": false,
                "helper_managed_by_btxd": false, "helper_state": "disabled",
                "helper_pid": -1, "helper_restart_count": 0, "pq1_ready": false,
                "error": "btx-modeld not connected", "automatic_spend_atoms": 0,
                "helper_error": "model helper disabled"
            }))
        });
        assert_eq!(read(&n, dir.path(), NOW).await, Seen::Off);
        assert_eq!(n.count("listmodels"), 0);
        assert_eq!(
            args_for(true, false),
            vec!["-modelnet=0"],
            "and that launch names nothing else"
        );
    }

    /// The ordinary first day: no model, no peer. It is a report, not an error.
    #[tokio::test]
    async fn on_and_empty_is_a_plain_report_of_nothing() {
        let dir = tempfile::tempdir().unwrap();
        guard_file(dir.path(), guard_ready());
        let n = node();
        n.with(|s| s.modelnet = Some(info(native(json!([]), json!([])))));
        assert_eq!(
            read(&n, dir.path(), NOW).await,
            Seen::Observing(Report {
                models_held: 0,
                peers_seen: 0,
                shared_families: vec![],
                lan_discovery: true,
                lan_error: String::new(),
                guard: Guard::Ready,
                observing_only: true,
            })
        );
    }

    #[tokio::test]
    async fn on_with_one_peer_counts_it_and_the_model_both_hold() {
        let dir = tempfile::tempdir().unwrap();
        guard_file(dir.path(), guard_ready());
        let n = node();
        n.with(|s| {
            s.modelnet = Some(info(native(
                json!([peer("pin-a", &["qwen3-4b", "other"])]),
                json!([{
                    "model_id": "qwen3-4b",
                    "peer_transport_pins": ["pin-a"],
                    "role": "observer",
                    "assurance": "LOCAL_BYTES_VERIFIED_REMOTE_AVAILABILITY_ADVERTISED"
                }]),
            )));
            s.models = json!([
                {"model_id": "qwen3-4b", "bytes_verified": true, "incomplete": false},
                // Half-fetched and unverified files are not held.
                {"model_id": "half", "bytes_verified": true, "incomplete": true},
                {"model_id": "unchecked", "bytes_verified": false, "incomplete": false}
            ]);
        });
        let Seen::Observing(r) = read(&n, dir.path(), NOW).await else {
            panic!("expected a report");
        };
        assert_eq!(r.models_held, 1);
        assert_eq!(r.peers_seen, 1);
        assert_eq!(
            r.shared_families,
            vec![SharedFamily {
                model_id: "qwen3-4b".into(),
                peers: 1
            }]
        );
        assert!(r.observing_only);
        assert_eq!(n.methods(), vec!["getmodelnetworkinfo", "listmodels"]);
    }

    /// The card's "observing only, nothing earns" rests on the engine's own
    /// words. If a later engine authorizes a peer to compute or reports a
    /// spend, the card must stop saying it.
    #[tokio::test]
    async fn observing_only_is_withdrawn_when_the_engine_reports_more() {
        let dir = tempfile::tempdir().unwrap();
        for change in ["compute", "spend", "qualified"] {
            let mut p = peer("pin-a", &[]);
            let mut nat = native(json!([]), json!([]));
            match change {
                "compute" => p["compute_authorized"] = json!(true),
                "spend" => nat["automatic_spend_atoms"] = json!("1"),
                _ => nat["release_qualified"] = json!(true),
            }
            nat["peer_observations"] = json!([p]);
            let n = node();
            n.with(|s| s.modelnet = Some(info(nat)));
            let Seen::Observing(r) = read(&n, dir.path(), NOW).await else {
                panic!("expected a report");
            };
            assert!(!r.observing_only, "{change}");
        }
    }

    /// Upstream's own archive with its helpers taken out, which is how the
    /// download staging script stages it: the model network is compiled in,
    /// the helper is not there.
    #[tokio::test]
    async fn a_missing_helper_is_named_in_the_engines_words() {
        let dir = tempfile::tempdir().unwrap();
        let n = node();
        n.with(|s| {
            s.modelnet = Some(json!({
                "schema_version": 2, "enabled": true, "helper_ready": false,
                "helper_state": "missing", "error": "btx-modeld not found next to btxd",
                "helper_error": "connect: No such file or directory"
            }))
        });
        assert_eq!(
            read(&n, dir.path(), NOW).await,
            Seen::HelperDown {
                why: "connect: No such file or directory".into()
            }
        );
    }

    #[tokio::test]
    async fn an_unknown_report_shape_and_a_silent_node_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let n = node();
        n.with(|s| {
            s.modelnet = Some(info(json!({"schema": "BTX/NativeParticipation/2"})));
        });
        assert_eq!(
            read(&n, dir.path(), NOW).await,
            Seen::Unreadable {
                why: "the engine reports its swarm as BTX/NativeParticipation/2".into()
            }
        );
        n.with(|s| {
            s.silent.insert("getmodelnetworkinfo");
        });
        assert!(matches!(
            read(&n, dir.path(), NOW).await,
            Seen::Unreachable { .. }
        ));
    }

    #[test]
    fn the_guard_file_is_believed_only_while_it_is_current() {
        assert_eq!(guard_from(&guard_ready(), NOW), Guard::Ready);
        assert_eq!(guard_from(&guard_ready(), NOW + 28_001), Guard::Stale);
        let mut down = guard_ready();
        down["ready"] = json!(false);
        down["state"] = json!("BACKOFF");
        assert_eq!(
            guard_from(&down, NOW),
            Guard::NotReady {
                why: "BACKOFF".into()
            }
        );
        down["error"] = json!("btx-guardd not found");
        assert_eq!(
            guard_from(&down, NOW),
            Guard::NotReady {
                why: "btx-guardd not found".into()
            }
        );
        assert_eq!(guard_from(&json!({"schema": "other"}), NOW), Guard::Unknown);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_guard(dir.path(), NOW), Guard::Unknown);
        guard_file(dir.path(), guard_ready());
        assert_eq!(read_guard(dir.path(), NOW), Guard::Ready);
    }

    #[test]
    fn the_marker_follows_the_setting_and_is_off_in_a_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!wanted(dir.path()));
        set_wanted(dir.path(), false).unwrap();
        assert!(!wanted(dir.path()));
        set_wanted(dir.path(), true).unwrap();
        assert!(wanted(dir.path()));
        set_wanted(dir.path(), true).unwrap();
        set_wanted(dir.path(), false).unwrap();
        assert!(!wanted(dir.path()));
    }

    /// The engine is read, not its version. Two files that differ only in the
    /// one option name, with the name placed across the read boundary too.
    #[test]
    fn the_model_network_is_read_from_the_engine_itself() {
        let dir = tempfile::tempdir().unwrap();
        let without = dir.path().join("btxd-without");
        std::fs::write(&without, vec![b'x'; (1 << 20) + 4096]).unwrap();
        assert!(!engine_has_model_network(&without));
        // A shorter or cut-off name is not the option.
        let near = dir.path().join("btxd-near");
        std::fs::write(&near, b"....-modelnet....-modelnetrequire").unwrap();
        assert!(!engine_has_model_network(&near));
        for at in [
            0usize,
            77,
            (1 << 20) - 5,
            (1 << 20) - 1,
            1 << 20,
            (1 << 20) + 3000,
        ] {
            let mut bytes = vec![b'x'; (1 << 20) + 4096];
            bytes[at..at + MODEL_NETWORK_OPTION.len()].copy_from_slice(MODEL_NETWORK_OPTION);
            let with = dir.path().join(format!("btxd-with-{at}"));
            std::fs::write(&with, bytes).unwrap();
            assert!(engine_has_model_network(&with), "at {at}");
        }
        assert!(!engine_has_model_network(&dir.path().join("absent")));
    }

    /// A node that answers with what the REAL engine answered
    /// (`tests/fixtures/swarm/README.md`): the ModelNet build of 074dac16,
    /// two nodes on regtest, started with [`OBSERVING`].
    fn real(info: &str, models: Option<&str>) -> FakeNode {
        let n = node();
        let info: Value = serde_json::from_str(info).unwrap();
        let models = models
            .map(|m| serde_json::from_str::<Value>(m).unwrap()["models"].clone())
            .unwrap_or_else(|| json!([]));
        n.with(|s| {
            s.modelnet = Some(info);
            s.models = models;
        });
        n
    }

    const REAL_EMPTY: &str = include_str!("../tests/fixtures/swarm/empty.getmodelnetworkinfo.json");
    const REAL_EMPTY_MODELS: &str = include_str!("../tests/fixtures/swarm/empty.listmodels.json");
    const REAL_GUARD: &str = include_str!("../tests/fixtures/swarm/empty.guard-process.json");
    const REAL_ONE_PEER: &str =
        include_str!("../tests/fixtures/swarm/onepeer.getmodelnetworkinfo.json");
    const REAL_ONE_PEER_MODELS: &str =
        include_str!("../tests/fixtures/swarm/onepeer.listmodels.json");
    const REAL_OFF: &str = include_str!("../tests/fixtures/swarm/off.getmodelnetworkinfo.json");

    /// The real guard file, read at the moment btxd wrote it and after it
    /// ran out. btxd gives it thirty seconds.
    #[test]
    fn the_real_guard_file_reads_as_healthy_until_it_runs_out() {
        let v: Value = serde_json::from_str(REAL_GUARD).unwrap();
        let written = v["observed_at_ms"].as_u64().unwrap();
        assert_eq!(v["valid_until_ms"].as_u64().unwrap() - written, 30_000);
        assert_eq!(guard_from(&v, written), Guard::Ready);
        assert_eq!(guard_from(&v, written + 30_001), Guard::Stale);
    }

    #[tokio::test]
    async fn the_real_engine_on_and_empty_reads_as_an_empty_report() {
        let dir = tempfile::tempdir().unwrap();
        let guard: Value = serde_json::from_str(REAL_GUARD).unwrap();
        guard_file(dir.path(), guard.clone());
        let n = real(REAL_EMPTY, Some(REAL_EMPTY_MODELS));
        let now = guard["observed_at_ms"].as_u64().unwrap();
        assert_eq!(
            read(&n, dir.path(), now).await,
            Seen::Observing(Report {
                models_held: 0,
                peers_seen: 0,
                shared_families: vec![],
                // The test nodes ran with -modellandiscovery=0.
                lan_discovery: false,
                lan_error: String::new(),
                guard: Guard::Ready,
                observing_only: true,
            })
        );
        // And what the launch asked for is what the engine says it does.
        let info: Value = serde_json::from_str(REAL_EMPTY).unwrap();
        assert_eq!(info["follow_configured_peers"], false);
        assert_eq!(info["seed_mode"], "off");
        assert_eq!(info["relay_status"], "off");
        assert_eq!(info["helper_state"], "READY");
    }

    #[tokio::test]
    async fn the_real_engine_with_one_peer_reads_as_one_peer_seen() {
        let dir = tempfile::tempdir().unwrap();
        let n = real(REAL_ONE_PEER, Some(REAL_ONE_PEER_MODELS));
        let Seen::Observing(r) = read(&n, dir.path(), NOW).await else {
            panic!("expected a report");
        };
        assert_eq!(r.peers_seen, 1);
        assert_eq!(r.models_held, 0);
        assert!(r.shared_families.is_empty(), "neither node held a model");
        assert!(r.observing_only);
        assert_eq!(r.guard, Guard::Unknown, "no guard file in this folder");
    }

    #[tokio::test]
    async fn the_real_engine_with_modelnet_off_reads_as_off() {
        let dir = tempfile::tempdir().unwrap();
        let n = real(REAL_OFF, None);
        assert_eq!(read(&n, dir.path(), NOW).await, Seen::Off);
        assert_eq!(n.methods(), vec!["getmodelnetworkinfo"]);
    }

    /// The same through the launch the app really builds, on every backend.
    #[test]
    fn the_node_launch_carries_the_swarm_flags_and_no_others() {
        use crate::backend::Backend;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir
            .path()
            .join("btx")
            .join("v0.34.16")
            .join("plat")
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let btxd = bin.join("btxd");
        std::fs::write(&btxd, MODEL_NETWORK_OPTION).unwrap();
        let dd = dir.path().join("dd");
        std::fs::create_dir_all(&dd).unwrap();
        let conf = dd.join("btx.conf");
        // A conf that asks for more than the app allows does not reach the
        // launch: the command line names each of these and outranks it.
        std::fs::write(
            &conf,
            "modelnet=1
modelhost=1
modelfollowpeers=1
",
        )
        .unwrap();
        let model = |args: &[String]| -> Vec<String> {
            args.iter()
                .filter(|a| a.starts_with("-model"))
                .cloned()
                .collect()
        };
        for backend in [Backend::Metal, Backend::Cuda, Backend::Cpu] {
            let (_, args, _) = crate::node::build_node_command(&btxd, &dd, &conf, backend);
            assert_eq!(model(&args), vec!["-modelnet=0"], "{backend:?}");
            assert!(args.iter().any(|a| a == "-swapnet=0"));
        }
        set_wanted(&dd, true).unwrap();
        for backend in [Backend::Metal, Backend::Cuda, Backend::Cpu] {
            let (_, args, _) = crate::node::build_node_command(&btxd, &dd, &conf, backend);
            assert_eq!(model(&args), OBSERVING, "{backend:?}");
        }
    }

    #[test]
    fn the_launch_says_nothing_off_or_observing_and_never_more() {
        let dir = tempfile::tempdir().unwrap();
        let with = dir.path().join("with");
        std::fs::write(&with, MODEL_NETWORK_OPTION).unwrap();
        let without = dir.path().join("without");
        std::fs::write(&without, b"btxd").unwrap();
        let dd = dir.path().join("dd");
        std::fs::create_dir_all(&dd).unwrap();

        assert!(launch_args(&without, &dd).is_empty());
        assert_eq!(launch_args(&with, &dd), vec!["-modelnet=0"]);
        set_wanted(&dd, true).unwrap();
        // An engine without the model network refuses the option, whatever the
        // setting says.
        assert!(launch_args(&without, &dd).is_empty());
        let on = launch_args(&with, &dd);
        assert_eq!(on[0], "-modelnet=1");
        for off in [
            "-modelhost=0",
            "-modelrelay=0",
            "-modelfollowpeers=0",
            "-modelpreserverare=0",
            "-modelseed=off",
        ] {
            assert!(on.iter().any(|a| a == off), "{off}: {on:?}");
        }
        // The listener, LAN discovery and the storage budget are the engine's
        // and the owner's conf to decide; see OBSERVING.
        for unstated in [
            "-modelbind",
            "-modellandiscovery",
            "-modelstorage",
            "-modelpeer",
        ] {
            assert!(!on.iter().any(|a| a.starts_with(unstated)), "{unstated}");
        }
        assert_eq!(on.len(), OBSERVING.len());
    }
}
