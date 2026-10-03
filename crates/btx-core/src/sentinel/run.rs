//! The sentinel's I/O, behind traits, and the tick that ties it together.
//!
//! [`Sources`] is everything it reads, [`Notifier`] everything it sends. The
//! live implementations are below; tests use fakes, so no test touches the
//! network. Every read is a GET or a read-only RPC (`getblockchaininfo`,
//! `getblockhash`, `getmatmulattestations`), plus the version handshake in
//! `p2p`. Nothing here can change a node.

use super::channels::{self, AlarmConf, Channel};
use super::decide::{
    self, Census, CensusNode, Dispute, Latest, Observations, Outgoing, SignersObs, SnapshotsObs,
    State, Statement, Thresholds,
};
use super::p2p::{self, Handshake};
use crate::rpc::{Rpc, RpcClient};
use crate::signer::RecentSigners;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// How often each check runs, in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intervals {
    pub signers: u64,
    pub witnesses: u64,
    pub snapshots: u64,
    pub census: u64,
    pub seeds: u64,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            signers: 60,
            witnesses: 120,
            snapshots: 300,
            census: 600,
            seeds: 1800,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub thresholds: Thresholds,
    pub intervals: Intervals,
    /// "host:port", read from the shipped lists in `node.rs`.
    pub seeds: Vec<String>,
    /// (name, base URL); the tip is `<base>/blocks/tip/height`.
    pub witnesses: Vec<(String, String)>,
}

/// The seeds every easyNode ships, from the code, deduplicated, in order.
pub fn shipped_seeds() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in crate::node::BTX_BOOTSTRAP_PEERS
        .iter()
        .chain(crate::node::BTX_ARCHIVE_PEERS.iter())
    {
        if !out.iter().any(|q| q == p) {
            out.push(p.to_string());
        }
    }
    out
}

// ---- traits -------------------------------------------------------------

#[async_trait]
pub trait Sources: Send + Sync {
    async fn snapshots(&self) -> SnapshotsObs;
    /// btxd2's tip and the signer window, brought up to date.
    async fn signers(&self, window: &mut RecentSigners) -> Result<SignersObs, String>;
    async fn handshake(&self, peer: &str) -> Result<Handshake, String>;
    async fn census(&self) -> Result<Census, String>;
    async fn witness_tip(&self, base: &str) -> Result<u64, String>;
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, o: &Outgoing, now: u64) -> Result<(), String>;
}

// ---- parsing the site's answers (pure) ------------------------------------

/// /api/snapshots/latest: 200 with a pointer, 200 with `disputed`, or 404
/// with `confirmed: null`. Anything else is a failed read.
pub fn parse_latest(status: u16, body: &str) -> Result<Latest, String> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| format!("latest: HTTP {status}, not JSON"))?;
    if status == 404 && v.get("confirmed").is_some_and(Value::is_null) {
        return Ok(Latest::None);
    }
    if status != 200 {
        return Err(format!("latest: HTTP {status}"));
    }
    if let Some(d) = v.get("disputed").and_then(Value::as_array) {
        return Ok(Latest::Disputed(
            d.iter().filter_map(Value::as_u64).collect(),
        ));
    }
    let height = v.get("height").and_then(Value::as_u64);
    let at = v
        .get("confirmed_at")
        .and_then(Value::as_str)
        .and_then(decide::parse_iso_utc);
    match (height, at) {
        (Some(height), Some(confirmed_at)) => Ok(Latest::Confirmed {
            height,
            confirmed_at,
        }),
        _ => Err("latest: a pointer without height or confirmed_at".to_string()),
    }
}

pub fn parse_pending(body: &str) -> Result<Vec<Statement>, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "pending: not JSON".to_string())?;
    let list = v
        .get("statements")
        .and_then(Value::as_array)
        .ok_or("pending: no statements list")?;
    Ok(list
        .iter()
        .filter_map(|s| {
            Some(Statement {
                height: s.get("height")?.as_u64()?,
                first_seen: decide::parse_iso_utc(s.get("first_seen")?.as_str()?)?,
                operators: s
                    .get("operators")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
                dissent: s.get("dissent").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect())
}

pub fn parse_disputes(body: &str) -> Result<Vec<Dispute>, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "disputes: not JSON".to_string())?;
    let list = v
        .get("disputes")
        .and_then(Value::as_array)
        .ok_or("disputes: no disputes list")?;
    Ok(list
        .iter()
        .filter_map(|d| {
            Some(Dispute {
                height: d.get("height")?.as_u64()?,
                differ: d
                    .get("differ")
                    .and_then(Value::as_array)
                    .map(|xs| {
                        xs.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect())
}

/// easybtx.com/api/nodes: `checkedAt` (unix seconds, the checker's run) and
/// `nodes[]` with `tag`, `version`, `behind`.
pub fn parse_census(body: &str) -> Result<Census, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "census: not JSON".to_string())?;
    let checked_at = v
        .get("checkedAt")
        .and_then(Value::as_u64)
        .ok_or("census: no checkedAt")?;
    let nodes = v
        .get("nodes")
        .and_then(Value::as_array)
        .map(|xs| {
            xs.iter()
                .filter_map(|n| {
                    Some(CensusNode {
                        tag: n.get("tag")?.as_str()?.to_string(),
                        version: n.get("version").and_then(Value::as_str).map(str::to_string),
                        behind: n.get("behind").and_then(Value::as_u64),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Census { checked_at, nodes })
}

/// A witness tip is digits and nothing else; an error page full of numbers
/// must not read as a height.
pub fn parse_tip(body: &str) -> Result<u64, String> {
    let t = body.trim();
    if t.is_empty() || t.len() > 12 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err("not a height".to_string());
    }
    t.parse().map_err(|_| "not a height".to_string())
}

// ---- live sources ---------------------------------------------------------

pub struct LiveSources {
    pub http: reqwest::Client,
    pub site: String,
    pub rpc_url: String,
    pub cookie: PathBuf,
    rpc: tokio::sync::Mutex<Option<RpcClient>>,
}

impl LiveSources {
    pub fn new(site: &str, rpc_addr: &str, cookie: &Path) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!("btx-sentinel/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            http,
            site: site.trim_end_matches('/').to_string(),
            rpc_url: format!("http://{rpc_addr}"),
            cookie: cookie.to_path_buf(),
            rpc: tokio::sync::Mutex::new(None),
        }
    }

    async fn get(&self, url: &str) -> Result<(u16, String), String> {
        let r = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        let status = r.status().as_u16();
        let body = r.text().await.map_err(|e| e.without_url().to_string())?;
        Ok((status, body))
    }

    async fn get_ok(&self, url: &str) -> Result<String, String> {
        match self.get(url).await? {
            (200, b) => Ok(b),
            (s, _) => Err(format!("HTTP {s}")),
        }
    }
}

#[async_trait]
impl Sources for LiveSources {
    async fn snapshots(&self) -> SnapshotsObs {
        let base = format!("{}/api/snapshots", self.site);
        let latest = match self.get(&format!("{base}/latest?chain=main")).await {
            Ok((s, b)) => parse_latest(s, &b),
            Err(e) => Err(e),
        };
        let pending = self
            .get_ok(&format!("{base}/pending?chain=main"))
            .await
            .and_then(|b| parse_pending(&b));
        let disputes = self
            .get_ok(&format!("{base}/disputes?chain=main"))
            .await
            .and_then(|b| parse_disputes(&b));
        SnapshotsObs {
            latest,
            pending,
            disputes,
        }
    }

    async fn signers(&self, window: &mut RecentSigners) -> Result<SignersObs, String> {
        let mut guard = self.rpc.lock().await;
        if guard.is_none() {
            // Built lazily, so a sentinel started before btxd2 wrote its
            // cookie catches up by itself. The client re-reads the cookie
            // on a 401, which covers every later btxd2 restart.
            *guard = Some(
                RpcClient::from_cookie(&self.rpc_url, &self.cookie)
                    .map_err(|e| format!("{}: {e}", self.cookie.display()))?,
            );
        }
        let rpc: &dyn Rpc = guard.as_ref().ok_or("no RPC client")?;
        observe_signers(rpc, window).await
    }

    async fn handshake(&self, peer: &str) -> Result<Handshake, String> {
        p2p::probe(peer, HANDSHAKE_TIMEOUT, unix_now() as i64).await
    }

    async fn census(&self) -> Result<Census, String> {
        let b = self.get_ok(&format!("{}/api/nodes", self.site)).await?;
        parse_census(&b)
    }

    async fn witness_tip(&self, base: &str) -> Result<u64, String> {
        let b = self
            .get_ok(&format!("{}/blocks/tip/height", base.trim_end_matches('/')))
            .await?;
        parse_tip(&b)
    }
}

/// The tip and the signer window from any node RPC: read-only calls only.
pub async fn observe_signers(
    rpc: &dyn Rpc,
    window: &mut RecentSigners,
) -> Result<SignersObs, String> {
    let info = crate::node_api::get_blockchain_info(rpc)
        .await
        .map_err(|e| e.to_string())?;
    let budget = crate::signer::SIGNED_WINDOW_BLOCKS as usize;
    crate::signer::refresh_recent_signers(rpc, window, info.blocks, budget).await;
    let s = window.summary();
    Ok(SignersObs {
        tip: info.blocks,
        seen: s.seen,
        distinct: s.distinct_keys,
        newest_signed: window.newest_signed_height(),
    })
}

pub struct LiveNotifier {
    pub http: reqwest::Client,
    pub conf: AlarmConf,
}

#[async_trait]
impl Notifier for LiveNotifier {
    async fn send(&self, o: &Outgoing, now: u64) -> Result<(), String> {
        channels::deliver(&self.http, &self.conf, o.channel, o.prio, &o.text, now).await
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---- the sentinel -----------------------------------------------------------

pub struct Sentinel {
    pub cfg: Config,
    pub state: State,
    /// In memory only: refilled in one pass after a restart.
    pub window: RecentSigners,
    pub state_path: Option<PathBuf>,
}

impl Sentinel {
    /// Start from the state file, or fresh when it is missing or unreadable
    /// (a fresh start forgets which alarms were sent, so each that is still
    /// true is said once more; nothing is lost that matters).
    pub fn load(cfg: Config, state_path: Option<PathBuf>) -> Self {
        let state = state_path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            cfg,
            state,
            window: RecentSigners::new(),
            state_path,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = &self.state_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(std::io::Error::other)?;
        crate::fsx::atomic_write(p, &bytes)
    }

    fn due(&mut self, name: &str, every: u64, now: u64, force: bool) -> bool {
        let due = force
            || self
                .state
                .last_run
                .get(name)
                .is_none_or(|at| now.saturating_sub(*at) >= every);
        if due {
            self.state.last_run.insert(name.to_string(), now);
        }
        due
    }

    /// Read what is due (everything with `force`).
    pub async fn observe(&mut self, src: &dyn Sources, now: u64, force: bool) -> Observations {
        let iv = self.cfg.intervals.clone();
        let mut obs = Observations::default();
        // Signers first: the witness check compares against btxd2's tip.
        if self.due("signers", iv.signers, now, force) {
            obs.signers = Some(src.signers(&mut self.window).await);
        }
        if self.due("snapshots", iv.snapshots, now, force) {
            obs.snapshots = Some(src.snapshots().await);
        }
        if self.due("census", iv.census, now, force) {
            obs.census = Some(src.census().await);
        }
        if self.due("witnesses", iv.witnesses, now, force) {
            let mut w = Vec::new();
            for (name, base) in &self.cfg.witnesses {
                w.push((name.clone(), src.witness_tip(base).await));
            }
            obs.witnesses = Some(w);
        }
        if self.due("seeds", iv.seeds, now, force) {
            let mut s = Vec::new();
            for peer in &self.cfg.seeds {
                s.push((peer.clone(), src.handshake(peer).await));
            }
            obs.seeds = Some(s);
        }
        obs
    }

    /// One tick: observe, decide, deliver, remember. Returns each message
    /// with whether its channel took it.
    pub async fn tick(
        &mut self,
        src: &dyn Sources,
        notify: &dyn Notifier,
        channels: &[Channel],
        now: u64,
    ) -> Vec<(Outgoing, Result<(), String>)> {
        let obs = self.observe(src, now, false).await;
        // The signer check runs first, so the witness check this same tick
        // compares against the fresh tip.
        let findings = decide::evaluate(&obs, &self.cfg.thresholds, &mut self.state, now);
        let out = decide::plan(&mut self.state, &findings, channels, now);
        let mut done = Vec::with_capacity(out.len());
        for o in out {
            let r = notify.send(&o, now).await;
            decide::record_delivery(&mut self.state, &o, r.is_ok(), now);
            done.push((o, r));
        }
        if let Err(e) = self.save() {
            eprintln!("[sentinel] could not save state: {e}");
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::channels::Prio;
    use std::sync::Mutex;

    const NOW: u64 = 1_791_000_000;

    #[test]
    fn the_site_answers_parse() {
        assert_eq!(
            parse_latest(404, r#"{"version":1,"confirmed":null}"#),
            Ok(Latest::None)
        );
        assert_eq!(
            parse_latest(200, r#"{"disputed":[237200]}"#),
            Ok(Latest::Disputed(vec![237_200]))
        );
        assert_eq!(
            parse_latest(
                200,
                r#"{"version":1,"chain":"main","height":237100,"confirmed_at":"2026-10-02T21:26:28.000Z"}"#
            ),
            Ok(Latest::Confirmed {
                height: 237_100,
                confirmed_at: 1_790_976_388
            })
        );
        assert!(parse_latest(503, "").is_err());
        assert!(parse_latest(404, "<html>").is_err());

        let p = parse_pending(r#"{"version":1,"chain":"main","statements":[
            {"statement_hash":"aa","height":237200,"first_seen":"2026-10-02T21:26:28.000Z","operators":["mende"],"dissent":false},
            {"statement_hash":"bb","height":237100,"first_seen":"2026-10-02T20:00:00.000Z","operators":["mende","numair"],"dissent":false}]}"#).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].operators, 1);
        assert_eq!(p[1].operators, 2);
        assert_eq!(
            parse_pending(r#"{"version":1,"chain":"main","statements":[]}"#),
            Ok(vec![])
        );

        let d = parse_disputes(
            r#"{"disputes":[{"height":237200,"differ":["block_hash","coins"],"opened_at":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(
            d,
            vec![Dispute {
                height: 237_200,
                differ: vec!["block_hash".into(), "coins".into()]
            }]
        );
    }

    /// The live census, trimmed (easybtx.com/api/nodes, 2026-10-03).
    #[test]
    fn the_census_parses() {
        let c = parse_census(r#"{"schema":2,"checkedAt":1791004034,"tipHeight":237194,
            "nodes":[{"name":null,"tag":"seed d942","kind":"listed","up":true,"height":237194,"behind":0,"version":"/BTX:0.34.12/"},
                     {"name":null,"tag":"node 9e47","kind":"community","up":true,"height":237194,"behind":0,"version":"/BTX:0.34.11/"},
                     {"tag":"node x","version":null,"behind":null}]}"#).unwrap();
        assert_eq!(c.checked_at, 1_791_004_034);
        assert_eq!(c.nodes.len(), 3);
        assert_eq!(c.nodes[0].version.as_deref(), Some("/BTX:0.34.12/"));
        assert_eq!(c.nodes[2].behind, None);
    }

    #[test]
    fn a_tip_is_digits_only() {
        assert_eq!(parse_tip("237194\n"), Ok(237_194));
        assert!(parse_tip("error 502").is_err());
        assert!(parse_tip("").is_err());
    }

    #[test]
    fn the_seeds_come_from_the_shipped_lists() {
        let s = shipped_seeds();
        for p in crate::node::BTX_BOOTSTRAP_PEERS
            .iter()
            .chain(crate::node::BTX_ARCHIVE_PEERS)
        {
            assert!(s.contains(&p.to_string()));
        }
        let mut d = s.clone();
        d.dedup();
        assert_eq!(d.len(), s.len());
    }

    /// Fake sources: today's world, with a seed that stops answering.
    struct Fake {
        seed_up: Mutex<bool>,
        /// Each signer read sees one more signed block, as on a live chain.
        tip: Mutex<u64>,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Sources for Fake {
        async fn snapshots(&self) -> SnapshotsObs {
            self.calls.lock().unwrap().push("snapshots".into());
            SnapshotsObs {
                latest: Ok(Latest::None),
                pending: Ok(vec![]),
                disputes: Ok(vec![]),
            }
        }
        async fn signers(&self, _w: &mut RecentSigners) -> Result<SignersObs, String> {
            self.calls.lock().unwrap().push("signers".into());
            let mut tip = self.tip.lock().unwrap();
            *tip += 1;
            Ok(SignersObs {
                tip: *tip,
                seen: 100,
                distinct: 2,
                newest_signed: Some(*tip),
            })
        }
        async fn handshake(&self, peer: &str) -> Result<Handshake, String> {
            self.calls.lock().unwrap().push(format!("seed {peer}"));
            if *self.seed_up.lock().unwrap() {
                Ok(Handshake {
                    version: 800_002,
                    services: 9,
                    user_agent: "/BTX:0.34.12/".into(),
                    start_height: 1000,
                })
            } else {
                Err("TCP connect failed".into())
            }
        }
        async fn census(&self) -> Result<Census, String> {
            self.calls.lock().unwrap().push("census".into());
            Ok(Census {
                checked_at: NOW,
                nodes: vec![],
            })
        }
        async fn witness_tip(&self, _base: &str) -> Result<u64, String> {
            self.calls.lock().unwrap().push("witness".into());
            Ok(*self.tip.lock().unwrap())
        }
    }

    #[derive(Default)]
    struct Inbox(Mutex<Vec<(Channel, Prio, String)>>);

    #[async_trait]
    impl Notifier for Inbox {
        async fn send(&self, o: &Outgoing, _now: u64) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push((o.channel, o.prio, o.text.clone()));
            Ok(())
        }
    }

    fn cfg() -> Config {
        Config {
            thresholds: Thresholds::default(),
            intervals: Intervals::default(),
            seeds: vec!["seed.example:19335".into()],
            witnesses: vec![("witness-1".into(), "https://w1.example".into())],
        }
    }

    #[tokio::test]
    async fn ticks_run_each_check_on_its_own_schedule_and_persist_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let src = Fake {
            seed_up: Mutex::new(true),
            tip: Mutex::new(1000),
            calls: Mutex::new(vec![]),
        };
        let inbox = Inbox::default();
        let mut s = Sentinel::load(cfg(), Some(path.clone()));

        let out = s.tick(&src, &inbox, &[Channel::Orca], NOW).await;
        // First tick: everything ran; the one thing true today is said.
        assert_eq!(src.calls.lock().unwrap().len(), 5);
        assert_eq!(out.len(), 1);
        assert!(out[0].0.text.starts_with("No snapshot is confirmed yet"));

        // A minute later only the signers are due.
        src.calls.lock().unwrap().clear();
        s.tick(&src, &inbox, &[Channel::Orca], NOW + 60).await;
        assert_eq!(*src.calls.lock().unwrap(), vec!["signers".to_string()]);

        // The seed stops answering: two seed runs (an hour) later, one alarm.
        *src.seed_up.lock().unwrap() = false;
        let mut now = NOW;
        let mut alarms = Vec::new();
        for _ in 0..61 {
            now += 60;
            for (o, _) in s.tick(&src, &inbox, &[Channel::Orca], now).await {
                alarms.push(o.text);
            }
        }
        assert_eq!(alarms.len(), 1, "{alarms:?}");
        assert!(alarms[0].starts_with("Seed seed.example:19335 failed a version handshake 2 times"));

        // A restart from the state file does not repeat it inside the hour.
        let mut s2 = Sentinel::load(cfg(), Some(path));
        assert_eq!(s2.state, s.state);
        let out = s2.tick(&src, &inbox, &[Channel::Orca], now + 60).await;
        assert!(out.is_empty(), "{out:?}");
    }

    /// Against a stub node: the signer observation uses only read-only calls.
    #[tokio::test]
    async fn signers_are_read_with_read_only_calls() {
        struct Node(Mutex<Vec<String>>);
        #[async_trait]
        impl Rpc for Node {
            async fn call(&self, method: &str, params: Value) -> crate::error::AppResult<Value> {
                self.0.lock().unwrap().push(method.to_string());
                match method {
                    "getblockchaininfo" => Ok(serde_json::json!({
                        "blocks": 50, "headers": 50, "verificationprogress": 1.0, "initialblockdownload": false
                    })),
                    "getblockhash" => Ok(serde_json::json!(format!(
                        "{:064x}",
                        params[0].as_u64().unwrap()
                    ))),
                    "getmatmulattestations" => Ok(serde_json::json!([format!(
                        "00ff21{}463044deadbeef",
                        "02".to_string() + &"a".repeat(64)
                    )])),
                    other => panic!("the sentinel must never call {other}"),
                }
            }
        }
        let node = Node(Mutex::new(vec![]));
        let mut w = RecentSigners::new();
        let o = observe_signers(&node, &mut w).await.unwrap();
        assert_eq!(o.tip, 50);
        assert_eq!(o.distinct, 1);
        assert_eq!(o.newest_signed, Some(50));
        let calls = node.0.lock().unwrap();
        assert!(calls.iter().all(|m| [
            "getblockchaininfo",
            "getblockhash",
            "getmatmulattestations"
        ]
        .contains(&m.as_str())));
    }
}
