//! The catch-up help against real v0.34.9 engines on regtest. Opt-in:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test catchup_regtest -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The mainnet stall of 2026-09-29 in miniature. Node A plays the archive
//! peer: pruned (`-prune=1`, so it advertises NETWORK_LIMITED without
//! NODE_NETWORK, like 109.199.124.187) with 400 blocks. Node B holds all 400
//! headers and block 1, and its only peer is A. Blocks 2 to 101 are more than
//! 288 below A's tip, so B's engine asks A for none of them: its tip stays at
//! 1. The help is ticked the way the refresher ticks it.
//!
//! - When A grants the test's loopback address `noban`, which is what lets a
//!   limited node serve a block deeper than 290 when asked for it by name,
//!   the help must ask A for blocks 2 to 101 by name and move B's tip.
//! - When A grants nothing, A drops B on the first of them (engine
//!   `src/net_processing.cpp:9384-9391` at 84b998b4), each time it is asked.
//!   The help must say the first drop and ask A again, then on the second
//!   drop say once that A does not serve old blocks to us, never ask it for
//!   them again, and conclude that no archive peer serves old blocks to B.
//!
//! Either way it must send no command but the ones it is allowed.
//!
//! Heights up to 99 connect without ExactReplay on regtest, so the first test
//! asks for 99, which takes seconds on any machine. On a Mac whose device
//! fails the engine's self-test, A's own block 100 connects only after a
//! restart; the setup does that when it has to (measured 2026-09-29 on the
//! owner's M2 Pro).

use std::ffi::OsStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use btx_core::catchup_assist::{self, CatchUp, Tick};
use btx_core::error::AppResult;
use btx_core::node_api::{get_attested_tip, get_blockchain_info, get_chain_tips, get_peer_info};
use btx_core::rpc::{Rpc, RpcClient};
use serde_json::{json, Value};

/// Every command the help may send. Anything else fails the test. The signed
/// frontier reaches the help through the Tick, as the refresher's slot, so
/// `getmatmulattestedtip` is not among them.
const ALLOWED: &[&str] = &[
    "getbestblockhash",
    "getblockheader",
    "getblockhash",
    "getblockfrompeer",
];

struct Btxd {
    child: std::process::Child,
    rpc: RpcClient,
}

impl Drop for Btxd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start(bin: &OsStr, dir: &std::path::Path, rpc_port: u16, args: &[String]) -> Btxd {
    let mut child = std::process::Command::new(bin)
        .arg(format!("-datadir={}", dir.display()))
        .arg(format!("-rpcport={rpc_port}"))
        .args([
            "-regtest",
            "-server=1",
            "-printtoconsole=0",
            "-daemon=0",
            "-dnsseed=0",
            "-fixedseeds=0",
            "-discover=0",
            "-allowunverifiablematmulconsensus=1",
        ])
        .args(args)
        .spawn()
        .unwrap();
    let cookie = dir.join("regtest").join(".cookie");
    let url = format!("http://127.0.0.1:{rpc_port}");
    for _ in 0..120 {
        if let Ok(c) = RpcClient::from_cookie(&url, &cookie) {
            if c.call("getblockcount", json!([])).await.is_ok() {
                return Btxd { child, rpc: c };
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("btxd on {rpc_port} did not answer within 60 s");
}

async fn stop(mut node: Btxd) {
    let _ = node.rpc.call("stop", json!([])).await;
    for _ in 0..120 {
        if node.child.try_wait().unwrap().is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn blocks(rpc: &RpcClient) -> u64 {
    get_blockchain_info(rpc).await.unwrap().blocks
}

/// The node under test, recording every command sent to it.
struct Watched<'a> {
    inner: &'a RpcClient,
    methods: Mutex<Vec<String>>,
}

impl Watched<'_> {
    fn count(&self, method: &str) -> usize {
        self.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == method)
            .count()
    }
}

#[async_trait]
impl Rpc for Watched<'_> {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.methods.lock().unwrap().push(method.to_string());
        self.inner.call(method, params).await
    }
}

#[derive(Clone, Copy)]
struct Ports {
    a_rpc: u16,
    a_p2p: u16,
    b_rpc: u16,
}

/// The two nodes and their folders. Fields drop in order, so the nodes are
/// killed before their folders go.
struct Pair {
    a: Btxd,
    b: Btxd,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// The stall: A with 400 blocks, pruned, granting loopback `noban` only when
/// `noban`; B with every header and block 1, whose only peer is A, and whose
/// engine is shown to ask A for nothing.
async fn stalled_pair(bin: &OsStr, ports: Ports, noban: bool) -> Pair {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a_args: Vec<String> = [
        "-listen=1",
        "-bind=127.0.0.1",
        "-listenonion=0",
        "-connect=0",
        "-prune=1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("-port={}", ports.a_p2p)])
    .collect();
    if noban {
        a_args.push("-whitelist=noban@127.0.0.1".into());
    }

    // A: 400 blocks.
    let mut a = start(bin, dir_a.path(), ports.a_rpc, &a_args).await;
    a.rpc.call("createwallet", json!(["w"])).await.unwrap();
    let addr = a.rpc.call("getnewaddress", json!([])).await.unwrap();
    a.rpc
        .call("generatetoaddress", json!([99, addr]))
        .await
        .unwrap();
    let _ = tokio::time::timeout(
        Duration::from_secs(20),
        a.rpc.call("generatetoaddress", json!([1, addr])),
    )
    .await;
    if blocks(&a.rpc).await < 100 {
        stop(a).await;
        a = start(bin, dir_a.path(), ports.a_rpc, &a_args).await;
        let _ = a.rpc.call("loadwallet", json!(["w"])).await;
        for _ in 0..120 {
            if blocks(&a.rpc).await >= 100 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    while blocks(&a.rpc).await < 400 {
        let n = (400 - blocks(&a.rpc).await).min(50);
        a.rpc
            .call("generatetoaddress", json!([n, addr]))
            .await
            .unwrap();
    }

    // B: every header, then block 1, with A as its only peer. Block 1 goes in
    // after the headers: a node whose tip is recent fetches the blocks of the
    // next headers it hears straight away (up to 16), which on mainnet a node
    // days behind never does.
    let b = start(
        bin,
        dir_b.path(),
        ports.b_rpc,
        &[
            "-listen=0".to_string(),
            format!("-connect=127.0.0.1:{}", ports.a_p2p),
        ],
    )
    .await;
    for _ in 0..120 {
        if get_blockchain_info(&b.rpc).await.unwrap().headers >= 400 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let one = a.rpc.call("getblockhash", json!([1])).await.unwrap();
    let raw = a.rpc.call("getblock", json!([one, 0])).await.unwrap();
    b.rpc.call("submitblock", json!([raw])).await.unwrap();
    let info = get_blockchain_info(&b.rpc).await.unwrap();
    assert_eq!((info.blocks, info.headers), (1, 400));

    // Left alone, B's engine asks A for none of blocks 2 to 101.
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(blocks(&b.rpc).await, 1, "the engine fetched on its own");
    Pair {
        a,
        b,
        _dirs: (dir_a, dir_b),
    }
}

/// One tick the way the refresher runs it: its own reads, the signed
/// frontier among them as its `signed_frontier` slot would hold it, then the
/// help on the watched client.
async fn tick_once(
    node: &RpcClient,
    watched: &Watched<'_>,
    cu: &mut CatchUp,
    now: Instant,
) -> Vec<String> {
    let info = get_blockchain_info(node).await.unwrap();
    let tips = get_chain_tips(node).await.unwrap();
    let peers = get_peer_info(node).await.unwrap();
    let slot = get_attested_tip(node).await.ok();
    let t = Tick {
        blocks: info.blocks,
        headers: info.headers,
        tips: &tips,
        peers: &peers,
        frontier: slot.as_ref(),
    };
    let lines = catchup_assist::tick(watched, cu, &t, now).await;
    for line in &lines {
        eprintln!("catch-up help: {line}");
    }
    lines
}

#[tokio::test]
#[ignore]
async fn the_help_moves_a_node_its_engine_leaves_standing() {
    let Some(bin) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let ports = Ports {
        a_rpc: 29451,
        a_p2p: 29461,
        b_rpc: 29452,
    };
    let pair = stalled_pair(&bin, ports, true).await;

    // The help, ticked like the refresher: 3 s of its clock per tick.
    let watched = Watched {
        inner: &pair.b.rpc,
        methods: Mutex::new(Vec::new()),
    };
    let archive = format!("127.0.0.1:{}", ports.a_p2p);
    let mut cu = CatchUp::new(vec![archive.clone()]);
    let clock = Instant::now();
    let mut lines = Vec::new();
    for i in 0..200u32 {
        if blocks(&pair.b.rpc).await >= 99 {
            break;
        }
        let now = clock + Duration::from_secs(3 * u64::from(i));
        lines.extend(tick_once(&pair.b.rpc, &watched, &mut cu, now).await);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let reached = blocks(&pair.b.rpc).await;
    assert!(reached >= 99, "tip {reached}; the help said {lines:?}");
    // The first request names exactly the next hundred. How many the node
    // took depends on the connection: in the dry run of 2026-09-29 the peer
    // dropped after 11 of the first 100, the help paused, and asked again.
    assert!(
        lines[0].starts_with(&format!("asked {archive} for blocks 2 to 101 (")),
        "{lines:?}"
    );
    for m in watched.methods.lock().unwrap().iter() {
        assert!(ALLOWED.contains(&m.as_str()), "the help sent {m}");
    }

    stop(pair.b).await;
    stop(pair.a).await;
}

#[tokio::test]
#[ignore]
async fn a_peer_that_drops_us_twice_over_old_blocks_is_not_asked_for_them_again() {
    let Some(bin) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let ports = Ports {
        a_rpc: 29453,
        a_p2p: 29463,
        b_rpc: 29454,
    };
    let pair = stalled_pair(&bin, ports, false).await;

    let watched = Watched {
        inner: &pair.b.rpc,
        methods: Mutex::new(Vec::new()),
    };
    let archive = format!("127.0.0.1:{}", ports.a_p2p);
    let once = catchup_assist::dropped_once_line(&archive);
    let mark = catchup_assist::refuses_old_line(&archive);
    let mut cu = CatchUp::new(vec![archive.clone()]);
    let clock = Instant::now();
    let mut lines = Vec::new();
    // (tick of the mark, requests sent by then)
    let mut marked: Option<(u32, usize)> = None;
    for i in 0..120u32 {
        let now = clock + Duration::from_secs(3 * u64::from(i));
        lines.extend(tick_once(&pair.b.rpc, &watched, &mut cu, now).await);
        match marked {
            None if lines.contains(&mark) => {
                marked = Some((i, watched.count("getblockfrompeer")));
            }
            // Twenty ticks, a minute of the help's clock, after the mark.
            Some((at, _)) if i >= at + 20 => break,
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let Some((_, sent_by_the_mark)) = marked else {
        panic!("the help never said A dropped us; it said {lines:?}");
    };
    assert!(
        lines[0].starts_with(&format!("asked {archive} for blocks 2 to 101 (")),
        "{lines:?}"
    );
    // Asked twice: once at first, once more after the first drop.
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("asked ")).count(),
        2,
        "asked A for old blocks a third time: {lines:?}"
    );
    assert_eq!(lines.iter().filter(|l| **l == once).count(), 1, "{lines:?}");
    assert_eq!(
        watched.count("getblockfrompeer"),
        sent_by_the_mark,
        "requests after the mark"
    );
    assert_eq!(lines.iter().filter(|l| **l == mark).count(), 1, "said once");
    // With A back and marked, the help concludes that no archive peer serves
    // old blocks to B, and says so once.
    let concluded = catchup_assist::NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string();
    assert_eq!(
        lines.iter().filter(|l| **l == concluded).count(),
        1,
        "{lines:?}"
    );
    assert!(cu.no_archive_serves_old_blocks());
    assert_eq!(blocks(&pair.b.rpc).await, 1, "A served B an old block");
    for m in watched.methods.lock().unwrap().iter() {
        assert!(ALLOWED.contains(&m.as_str()), "the help sent {m}");
    }

    stop(pair.b).await;
    stop(pair.a).await;
}
