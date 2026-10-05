//! The snapshot network end to end on regtest, through the app's own
//! functions, against real engines. Opt-in:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd \
//!   cargo test --test snapshot_network_regtest -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Without `EASYNODE_TEST_BTXD`, or with a path that is not there, the test
//! says so and passes: CI never runs an engine.
//!
//! THE CAST. P, the producer; C, a confirmer on P's chain; D, a confirmer
//! that mined its own chain from genesis, so its diary at 100 names another
//! block. All three validate (`-matmulvalidation=consensus`) and sign with a
//! local key. The list, in `EASYNODE_REGTEST_OPERATORS`'s format, is
//! `producer=P;confirmer1=C;confirmer2=D`. The keys are fixed test keys
//! (secrets 0x01.., 0x02.., 0x03.. repeated), regtest only, so a website
//! stand-in can be started with the same list before the test runs:
//!
//! ```text
//! producer=031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f;confirmer1=024d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766;confirmer2=02531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe337
//! ```
//!
//! OFF THE GPU. Every engine starts with `CUDA_VISIBLE_DEVICES=-1` (the CUDA
//! driver lists no device), `BTX_MATMUL_BACKEND=cpu` and
//! `-matmulrcexecution=cpu-diagnostic`, plus
//! `-allowunverifiablematmulconsensus=1` so a validating node connects
//! blocks above 100 without a qualified device. The env alone is not
//! enough: a btxd with `BTX_MATMUL_BACKEND=cpu` still took the card
//! (src/node.rs, 2026-09-15); hiding the device is what keeps it off. Each
//! start reads the engine's own log and fails unless it says
//! `no_supported_device` and `cpu-diagnostic provider=toy-rc`. Ports
//! 29631-29633 (RPC) and 29731-29733 (P2P), on 127.0.0.1, `-connect=0`: the
//! engines meet only each other, by `addnode`.
//!
//! WHAT RUNS WITHOUT A WEBSITE, in order:
//!
//! 1. P mines to 100 (its own block 100 connects after a restart), records
//!    its diary and exports on the grid through the keeper's own
//!    `export_on_grid` with `ProducerChecks`, i.e. `dumptxoutsetattested`.
//!    C syncs from P and records its diary at 100: the same block. D mines
//!    its own 100 and records it: another block.
//! 2. All three record again at 200. At 200 the export is 101 deep:
//!    `mature` keeps it waiting and C's check says `TooShallow`.
//! 3. At 243 the base is exactly 144 deep. `mature` re-verifies, runs
//!    `check_before_send` through the hook, and offers it over P2P;
//!    `check_before_send` on its own matches the export to P's diary.
//! 4. `check_against_node` on real nodes: C passes; C with no diary is
//!    `NoDiaryEntry`; C with its diary one coin up `Differs` in coins only;
//!    C with a hold on its own block 50 is `HeldRootOnActiveChain`; D
//!    `Differs` in the block hash, which is a dissent.
//! 5. The confirmer's own `confirm_one` on P's statement, listed as the
//!    website would list it. C signs a COPY with the real
//!    `signutxosnapshotmanifest` and keeps exactly one signature, its own,
//!    valid. D builds its dissent from its diary, an unsigned manifest the
//!    real engine signs, and never signs P's statement. With no website the
//!    send fails after the signing (`Verdict::Failed`), so the signed bytes
//!    are read back from the confirmer's log, where they wait unsent.
//! 6. The real engine refuses to sign a manifest it signed already, as
//!    `fake_node` assumes.
//!
//! WITH A WEBSITE. `EASYNODE_SNAPSHOT_SITE=http://127.0.0.1:<port>` names a
//! running stand-in (the website plan's `site/scripts/
//! snapshot-rendezvous-local.mjs`, started with the list above as its
//! `SNAPSHOT_REGTEST_OPERATORS` and a fresh store folder for each run). Then
//! step 5 goes through the website instead: a file with one bit flipped is
//! refused, P's pair is sent with `submit`, `latest` stays 404 at one
//! operator, C's round co-signs and `latest` names 100, D's round dissents.
//! Only `http://127.0.0.1:` is taken; anything else fails the test rather
//! than reach a real website.

use btx_core::confirmed_load::Holds;
use btx_core::confirmed_snapshot as cs;
use btx_core::diary::{self, DiaryEntry, DiaryOutcome};
use btx_core::known_invalid::HeldBranch;
use btx_core::operators::{self, Chain};
use btx_core::role::ValidationMode;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::snapshot_confirmer::{self as confirmer, Confirmer, LogKind, Verdict, WORK_DIR};
use btx_core::snapshot_producer::{self as producer, ProducerChecks};
use btx_core::snapshot_serve::{self as serve, MatureEvent};
use btx_core::snapshot_site::{self as site, PendingStatement, Site, SiteError};
use btx_core::statement_check::{check_against_node, Field, Mismatch, Next, DEPTH};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const GRID: u64 = serve::EXPORT_GRID;
/// Where the base at 100 is exactly [`DEPTH`] deep.
const DEEP: u64 = GRID + DEPTH - 1;
/// Nothing listens here: a confirmer's send fails after the engine signed.
const NO_SITE: &str = "http://127.0.0.1:29659";

/// A fixed regtest test key: (WIF, compressed public key hex, the key).
fn test_key(seed: u8) -> (String, String, [u8; 33]) {
    let sk = k256::SecretKey::from_slice(&[seed; 32]).unwrap();
    let mut payload = vec![0xef];
    payload.extend_from_slice(&sk.to_bytes());
    payload.push(0x01);
    let wif = bs58::encode(payload).with_check().into_string();
    let point = sk.public_key().to_encoded_point(true);
    let key: [u8; 33] = point.as_bytes().try_into().unwrap();
    (wif, operators::hex(&key), key)
}

/// One regtest btxd in its own folder, off the GPU, killed and reaped on
/// drop.
struct Node {
    name: &'static str,
    btxd: PathBuf,
    dir: PathBuf,
    rpc_port: u16,
    p2p_port: u16,
    args: Vec<String>,
    child: Option<std::process::Child>,
}

impl Node {
    fn new(
        name: &'static str,
        btxd: &Path,
        root: &Path,
        rpc_port: u16,
        wif: &str,
        pubkey: &str,
    ) -> Self {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("regtest")).unwrap();
        std::fs::write(dir.join("regtest").join("signer.wif"), format!("{wif}\n")).unwrap();
        Self {
            name,
            btxd: btxd.to_path_buf(),
            dir,
            rpc_port,
            p2p_port: rpc_port + 100,
            args: vec![
                "-matmulvalidation=consensus".into(),
                "-matmulattestationsignerkeyfile=signer.wif".into(),
                format!("-matmultrustedpubkey={pubkey}"),
                "-matmultrustedthreshold=1".into(),
                "-matmulattestationserve=0".into(),
                "-allowunverifiablematmulconsensus=1".into(),
                "-matmulrcexecution=cpu-diagnostic".into(),
            ],
            child: None,
        }
    }

    /// The network folder: the engine's regtest chain, and here also the
    /// app's diary, confirmer log and snapshot pairs.
    fn net(&self) -> PathBuf {
        self.dir.join("regtest")
    }

    fn snapshots(&self) -> PathBuf {
        serve::snapshot_dir(&self.net())
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.p2p_port)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.net().join("debug.log")).unwrap_or_default()
    }

    fn log_tail(&self) -> String {
        let log = self.log();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(40)..].join("\n")
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    async fn start(&mut self) -> RpcClient {
        self.kill();
        let _ = std::fs::remove_file(self.net().join(".cookie"));
        let child = std::process::Command::new(&self.btxd)
            .arg("-regtest")
            .arg(format!("-datadir={}", self.dir.display()))
            .arg(format!("-rpcport={}", self.rpc_port))
            .arg(format!("-port={}", self.p2p_port))
            .args([
                "-server=1",
                "-bind=127.0.0.1",
                "-rpcbind=127.0.0.1",
                "-rpcallowip=127.0.0.1",
                "-listen=1",
                "-connect=0",
                "-discover=0",
                "-dnsseed=0",
                "-fixedseeds=0",
                "-upnp=0",
                "-natpmp=0",
                "-printtoconsole=0",
                "-daemon=0",
            ])
            .args(&self.args)
            // The GPU belongs to whoever else runs on this machine.
            .env("CUDA_VISIBLE_DEVICES", "-1")
            .env("BTX_MATMUL_BACKEND", "cpu")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn btxd");
        eprintln!("[rehearsal] {} started, pid {}", self.name, child.id());
        self.child = Some(child);
        let url = format!("http://127.0.0.1:{}", self.rpc_port);
        for _ in 0..240 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child = None;
                panic!("{} exited with {status}:\n{}", self.name, self.log_tail());
            }
            if let Ok(c) = RpcClient::from_cookie(url.clone(), &self.net().join(".cookie")) {
                if c.call("getblockcount", json!([])).await.is_ok() {
                    self.assert_off_gpu();
                    return c;
                }
            }
        }
        self.kill();
        panic!("{}: no RPC within 120 s:\n{}", self.name, self.log_tail());
    }

    /// The engine's own word that it found no CUDA device and replays on
    /// the CPU. Killed at once otherwise.
    fn assert_off_gpu(&mut self) {
        let log = self.log();
        let policy = log
            .lines()
            .rev()
            .find(|l| l.contains("MatMul RC execution policy:"))
            .unwrap_or_default()
            .to_string();
        let ok = log.contains("no_supported_device")
            && policy.contains("cpu-diagnostic provider=toy-rc")
            && !log.contains("provider=cuda");
        if !ok {
            self.kill();
            panic!(
                "{} did not stay off the GPU; policy line: {policy:?}\n{}",
                self.name,
                self.log_tail()
            );
        }
    }

    async fn stop(&mut self, rpc: &RpcClient) {
        let _ = rpc.call("stop", json!([])).await;
        if let Some(mut child) = self.child.take() {
            for _ in 0..120 {
                if child.try_wait().unwrap().is_some() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn call(rpc: &RpcClient, method: &str, params: Value) -> Value {
    rpc.call(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

async fn height(rpc: &RpcClient) -> u64 {
    call(rpc, "getblockcount", json!([]))
        .await
        .as_u64()
        .unwrap()
}

/// Mine until the node has a header at `to`, at most ten a call and never
/// past `to`. Headers, not blocks: the miner's own block 100 stays
/// unconnected until a restart, and asking for more there mines sibling
/// after sibling at 100.
async fn mine_to(rpc: &RpcClient, to: u64) {
    for _ in 0..200 {
        let headers = call(rpc, "getblockchaininfo", json!([])).await["headers"]
            .as_u64()
            .unwrap_or(0);
        if headers >= to {
            return;
        }
        let _ = rpc
            .call(
                "generatetodescriptor",
                json!([(to - headers).min(10), "raw(51)"]),
            )
            .await;
    }
    panic!("did not reach {to}");
}

/// Mine a fresh chain to 100 and restart, so the miner's own block 100
/// connects (measured with v0.34.12 on 2026-10-05: 99 blocks and a header
/// at 100 until the restart, 100 after it).
async fn mine_first_100(node: &mut Node, rpc: RpcClient) -> RpcClient {
    mine_to(&rpc, GRID).await;
    let rpc = if height(&rpc).await < GRID {
        node.stop(&rpc).await;
        node.start().await
    } else {
        rpc
    };
    wait_for(&rpc, GRID).await;
    rpc
}

async fn wait_for(rpc: &RpcClient, to: u64) {
    for _ in 0..360 {
        if height(rpc).await >= to {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the node did not reach {to}");
}

async fn record(rpc: &RpcClient, node: &Node) -> DiaryEntry {
    match diary::record_at_tip(rpc, &node.net(), ValidationMode::Consensus)
        .await
        .unwrap()
    {
        DiaryOutcome::Recorded(e) => e,
        other => panic!("{}: {other:?}", node.name),
    }
}

/// The statement as `GET /api/snapshots/pending` lists it.
fn listed(manifest: &[u8], operators: &[&str]) -> PendingStatement {
    let m = cs::parse(manifest).unwrap();
    let st = &m.statement;
    PendingStatement {
        statement_hash: st.hash().display_hex(),
        height: st.height() as u64,
        block_hash: st.block_hash().display_hex(),
        manifest_hex: operators::hex(manifest),
        signers: m
            .signatures
            .iter()
            .map(|s| operators::hex(&s.key))
            .collect(),
        operators: operators.iter().map(|s| s.to_string()).collect(),
        file: "stored".into(),
        first_seen: String::new(),
        confirmed: false,
        disputed: false,
        dissent: false,
    }
}

/// The one-signature manifest a confirmer's log holds unsent.
fn unsent(e: &confirmer::LogEntry) -> cs::Manifest {
    let hex = e.unsent.as_ref().expect("signed and kept unsent");
    cs::parse(&operators::hex_decode(hex).unwrap()).unwrap()
}

/// `EASYNODE_SNAPSHOT_SITE`, only as a loopback stand-in.
fn stand_in() -> Option<Site> {
    let raw = std::env::var(site::SITE_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    assert!(
        raw.starts_with("http://127.0.0.1:"),
        "{} must be a stand-in on http://127.0.0.1:<port>, not {raw:?}",
        site::SITE_ENV
    );
    Some(Site::parse(Some(raw)).unwrap())
}

/// A confirmer for one node, as the app builds it, for one round.
fn confirmer_at<'a>(
    rpc: &'a RpcClient,
    client: &'a reqwest::Client,
    site: &'a Site,
    state_dir: &'a Path,
    work_dir: &'a Path,
    our_key: [u8; 33],
    env: &'a str,
) -> Confirmer<'a> {
    Confirmer {
        rpc,
        client,
        site,
        state_dir,
        work_dir,
        our_key,
        holds: Holds::none(),
        regtest_env: Some(env),
    }
}

fn verdict_at(v: &[Verdict], height: u64) -> &Verdict {
    v.iter()
        .find(|x| x.height() == height)
        .unwrap_or_else(|| panic!("no verdict at {height}: {v:?}"))
}

#[tokio::test]
#[ignore]
async fn producer_and_confirmers_on_regtest() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("EASYNODE_TEST_BTXD unset or not a file; nothing to test against");
        return;
    };
    let web = stand_in();
    if web.is_none() {
        eprintln!(
            "{} unset: the website's part is skipped, everything else runs",
            site::SITE_ENV
        );
    }
    let root = tempfile::tempdir().unwrap();
    let (p_wif, p_pub, _) = test_key(1);
    let (c_wif, c_pub, c_key) = test_key(2);
    let (d_wif, d_pub, d_key) = test_key(3);
    let env = format!("producer={p_pub};confirmer1={c_pub};confirmer2={d_pub}");
    eprintln!("[rehearsal] operators: {env}");
    let chain = site::chain_name(Chain::Regtest);

    // ── 1. P to 100, its diary and its export; C follows; D on its own ──
    let mut p = Node::new("p", &btxd, root.path(), 29631, &p_wif, &p_pub);
    let mut c = Node::new("c", &btxd, root.path(), 29632, &c_wif, &c_pub);
    let mut d = Node::new("d", &btxd, root.path(), 29633, &d_wif, &d_pub);
    let rp = p.start().await;
    let rp = mine_first_100(&mut p, rp).await;
    let checks = ProducerChecks {
        diary_dir: p.net(),
        holds: Holds::none(),
        regtest_env: Some(env.clone()),
    };
    let pair = serve::export_on_grid(&rp, &p.snapshots(), GRID, &checks, &|_| {})
        .await
        .expect("the export at 100");
    assert_eq!(pair.height, GRID);
    let p_100 = diary::load(&p.net(), operators::REGTEST_GENESIS)
        .at(GRID)
        .cloned()
        .expect("the keeper's hook wrote the diary before the export");
    assert_eq!(p_100.block_hash, pair.block_hash);

    let rc = c.start().await;
    call(&rc, "addnode", json!([p.addr(), "onetry"])).await;
    wait_for(&rc, GRID).await;
    let c_100 = record(&rc, &c).await;
    assert_eq!(
        c_100,
        DiaryEntry {
            recorded_at: c_100.recorded_at,
            ..p_100.clone()
        }
    );

    let rd = d.start().await;
    let rd = mine_first_100(&mut d, rd).await;
    let d_100 = record(&rd, &d).await;
    assert_ne!(d_100.block_hash, p_100.block_hash, "D's chain is its own");
    eprintln!(
        "[rehearsal] diaries at 100: P/C {} , D {}",
        p_100.block_hash, d_100.block_hash
    );

    // ── 2. 200 on every chain: diaries again; the export is too shallow ──
    mine_to(&rp, 2 * GRID).await;
    wait_for(&rp, 2 * GRID).await;
    wait_for(&rc, 2 * GRID).await;
    mine_to(&rd, 2 * GRID).await;
    wait_for(&rd, 2 * GRID).await;
    let p_200 = record(&rp, &p).await;
    assert_eq!(record(&rc, &c).await.block_hash, p_200.block_hash);
    record(&rd, &d).await;
    for n in [&p, &c, &d] {
        let kept: Vec<u64> = diary::load(&n.net(), operators::REGTEST_GENESIS)
            .entries
            .iter()
            .map(|e| e.height)
            .collect();
        assert_eq!(kept, vec![GRID, 2 * GRID], "{}'s diary", n.name);
    }
    let events = serve::mature(
        &rp,
        &p.snapshots(),
        serve::MATURE_DEADLINE,
        &checks,
        &|_| {},
    )
    .await;
    assert_eq!(
        events,
        vec![MatureEvent::Waiting {
            base: GRID,
            confirmations: GRID + 1,
            tip: 2 * GRID
        }]
    );
    let manifest = std::fs::read(p.snapshots().join(serve::manifest_file_name(GRID))).unwrap();
    let st = cs::parse(&manifest).unwrap().statement;
    let rules = cs::ChainRules::for_statement(&st, Some(&env)).unwrap();
    let c_diary = diary::load(&c.net(), operators::REGTEST_GENESIS);
    let shallow = check_against_node(&rc, &st, &rules, &c_diary, &Holds::none()).await;
    assert_eq!(
        shallow,
        Err(Mismatch::TooShallow {
            depth: GRID + 1,
            need: DEPTH
        })
    );

    // ── 3. 144 deep: matured, checked by the hook, offered over P2P ─────
    mine_to(&rp, DEEP).await;
    wait_for(&rp, DEEP).await;
    wait_for(&rc, DEEP).await;
    mine_to(&rd, DEEP).await;
    wait_for(&rd, DEEP).await;
    let events = serve::mature(
        &rp,
        &p.snapshots(),
        serve::MATURE_DEADLINE,
        &checks,
        &|_| {},
    )
    .await;
    match events.as_slice() {
        [MatureEvent::Offered(r)] => {
            assert_eq!(
                (r.height, r.block_hash.as_str()),
                (GRID, p_100.block_hash.as_str())
            )
        }
        other => panic!("{other:?}"),
    }
    let checked = producer::check_before_send(&rp, &manifest, &p.net(), &Holds::none(), Some(&env))
        .await
        .expect("the producer's export matches its own diary");
    assert_eq!(checked.operators, vec!["producer".to_string()]);
    assert_eq!(checked.statement_hash, st.hash().display_hex());
    assert_eq!(
        (
            st.block_hash().display_hex(),
            st.hash_serialized().display_hex(),
            st.coins(),
            st.chain_tx()
        ),
        (
            p_100.block_hash.clone(),
            p_100.hash_serialized.clone(),
            p_100.coins,
            p_100.chain_tx
        ),
        "the export and the diary agree field by field"
    );
    eprintln!(
        "[rehearsal] statement {} at 100: coins {}, transactions {}",
        checked.statement_hash, p_100.coins, p_100.chain_tx
    );

    // ── 4. The statement against real nodes ─────────────────────────────
    let c_diary = diary::load(&c.net(), operators::REGTEST_GENESIS);
    let ok = check_against_node(&rc, &st, &rules, &c_diary, &Holds::none()).await;
    assert_eq!(ok.map(|e| e.block_hash), Ok(p_100.block_hash.clone()));

    let empty = diary::Diary::new(operators::REGTEST_GENESIS);
    let none = check_against_node(&rc, &st, &rules, &empty, &Holds::none()).await;
    assert_eq!(none, Err(Mismatch::NoDiaryEntry { height: GRID }));
    assert_eq!(none.unwrap_err().next(), Next::Skip);

    let mut wrong = c_diary.clone();
    let mut e = wrong.at(GRID).unwrap().clone();
    e.coins += 1;
    wrong.record(e);
    match check_against_node(&rc, &st, &rules, &wrong, &Holds::none()).await {
        Err(m @ Mismatch::Differs { .. }) => {
            assert_eq!(m.next(), Next::Dissent);
            let Mismatch::Differs { fields, .. } = m else {
                unreachable!()
            };
            assert_eq!(fields, vec![Field::Coins]);
        }
        other => panic!("{other:?}"),
    }

    let c_50 = call(&rc, "getblockhash", json!([50])).await;
    let root_50: &'static str = Box::leak(c_50.as_str().unwrap().to_string().into_boxed_str());
    let held: &'static [HeldBranch] = Box::leak(Box::new([HeldBranch {
        height: 50,
        root: root_50,
        why: "the rehearsal's hold",
    }]));
    let holds = Holds { invalid: &[], held };
    assert_eq!(
        check_against_node(&rc, &st, &rules, &c_diary, &holds).await,
        Err(Mismatch::HeldRootOnActiveChain {
            height: 50,
            root: root_50.into()
        })
    );

    let d_diary = diary::load(&d.net(), operators::REGTEST_GENESIS);
    match check_against_node(&rd, &st, &rules, &d_diary, &Holds::none()).await {
        Err(m @ Mismatch::Differs { .. }) => {
            assert_eq!(m.next(), Next::Dissent);
            eprintln!("[rehearsal] D: {m}");
            let Mismatch::Differs { fields, .. } = m else {
                unreachable!()
            };
            assert!(fields.contains(&Field::BlockHash), "{fields:?}");
        }
        other => panic!("{other:?}"),
    }

    // ── 5. The confirmers ───────────────────────────────────────────────
    let client = site::client().unwrap();
    let (c_net, c_work) = (c.net(), c.snapshots().join(WORK_DIR));
    let (d_net, d_work) = (d.net(), d.snapshots().join(WORK_DIR));
    let statement_hash = st.hash().display_hex();
    let dat = p.snapshots().join(serve::snapshot_file_name(GRID));

    if let Some(web) = &web {
        // A file with one bit flipped is refused, and kept nowhere.
        let first = site::post_statement(&client, web, &manifest).await.unwrap();
        assert_eq!(first.operators, vec!["producer".to_string()]);
        let tampered = root.path().join("tampered.dat");
        let mut bytes = std::fs::read(&dat).unwrap();
        bytes[1000] ^= 1;
        std::fs::write(&tampered, &bytes).unwrap();
        match site::upload_file(&client, web, &st, &tampered).await {
            Err(SiteError::Refused { status: 422, .. }) => {}
            other => panic!("a tampered file: {other:?}"),
        }
        let sent = producer::submit(&client, web, &manifest, &dat)
            .await
            .unwrap();
        assert_eq!(sent.reply.statement_hash, statement_hash);
        assert!(
            site::get_latest(&client, web, Chain::Regtest)
                .await
                .unwrap()
                .is_none(),
            "one operator: latest names nothing"
        );
        let v = confirmer_at(&rc, &client, web, &c_net, &c_work, c_key, &env)
            .round(Chain::Regtest)
            .await
            .unwrap();
        assert!(
            matches!(verdict_at(&v, GRID), Verdict::Signed { .. }),
            "{v:?}"
        );
        let latest = site::get_latest(&client, web, Chain::Regtest)
            .await
            .unwrap();
        eprintln!("[rehearsal] latest after C: {latest:?}");
        assert!(latest.is_some(), "two operators: latest names 100");
        let v = confirmer_at(&rd, &client, web, &d_net, &d_work, d_key, &env)
            .round(Chain::Regtest)
            .await
            .unwrap();
        assert!(
            v.iter()
                .any(|x| matches!(x, Verdict::Dissented { height, .. } if *height == GRID)),
            "{v:?}"
        );
        let v = confirmer_at(&rc, &client, web, &c_net, &c_work, c_key, &env)
            .round(Chain::Regtest)
            .await
            .unwrap();
        assert!(v.iter().any(|x| matches!(x, Verdict::AlreadySigned { statement_hash: h, .. } if *h == statement_hash)), "{v:?}");
        eprintln!(
            "[rehearsal] latest after D: {:?}",
            site::get_latest(&client, web, Chain::Regtest).await
        );
    } else {
        let no_site = Site::parse(Some(NO_SITE)).unwrap();
        let listing = listed(&manifest, &["producer"]);

        // C signs a copy; the one signature it keeps is its own and valid.
        let v = confirmer_at(&rc, &client, &no_site, &c_net, &c_work, c_key, &env)
            .confirm_one(&listing)
            .await;
        match &v {
            Verdict::Failed { error, .. } => assert!(
                !error.contains("engine") && !error.contains("signature"),
                "the send failed, not the signing: {error}"
            ),
            other => panic!("{other:?}"),
        }
        let log = confirmer::load_log(&c.net());
        let entry = log
            .signature(chain, &statement_hash)
            .expect("C signed P's statement");
        assert!(entry.kind == LogKind::Signed && entry.height == GRID);
        let signed = unsent(entry);
        assert_eq!(signed.statement, st);
        assert_eq!(signed.signatures.len(), 1);
        assert_eq!(signed.signatures[0].key, c_key);
        assert!(cs::signature_is_valid(
            &st.hash(),
            &c_key,
            &signed.signatures[0].der
        ));
        assert!(
            std::fs::read_dir(c.snapshots().join(WORK_DIR))
                .unwrap()
                .next()
                .is_none(),
            "the copy is gone"
        );

        // The real engine will not sign the same manifest twice.
        let again = c.snapshots().join(WORK_DIR).join("again.manifest");
        std::fs::write(&again, signed.to_bytes()).unwrap();
        let twice = rc
            .call("signutxosnapshotmanifest", json!([again.to_string_lossy()]))
            .await;
        eprintln!("[rehearsal] signing a manifest C signed already: {twice:?}");
        assert!(twice.is_err(), "{twice:?}");
        let after = cs::parse(&std::fs::read(&again).unwrap()).unwrap();
        assert_eq!(after.signatures.len(), 1, "the refused file is unchanged");
        std::fs::remove_file(&again).unwrap();

        // D dissents: the engine signs a manifest that carried no signature.
        let v = confirmer_at(&rd, &client, &no_site, &d_net, &d_work, d_key, &env)
            .confirm_one(&listing)
            .await;
        match &v {
            Verdict::Failed { error, .. } => assert!(
                !error.contains("engine") && !error.contains("dissent"),
                "the send failed, not the signing: {error}"
            ),
            other => panic!("{other:?}"),
        }
        let log = confirmer::load_log(&d.net());
        assert!(
            log.signature(chain, &statement_hash).is_none(),
            "D never signs P's statement"
        );
        assert_eq!(log.entries.len(), 1, "{:?}", log.entries);
        let entry = log.dissent_at(chain, GRID).expect("D's dissent");
        assert_eq!(entry.against.as_deref(), Some(statement_hash.as_str()));
        let dissent = unsent(entry);
        assert!(cs::is_dissent(&dissent.statement));
        assert_eq!(dissent.statement.hash().display_hex(), entry.statement_hash);
        assert_eq!(
            (
                dissent.statement.height() as u64,
                dissent.statement.block_hash().display_hex(),
                dissent.statement.hash_serialized().display_hex(),
                dissent.statement.coins(),
                dissent.statement.chain_tx()
            ),
            (
                GRID,
                d_100.block_hash.clone(),
                d_100.hash_serialized.clone(),
                d_100.coins,
                d_100.chain_tx
            ),
            "the dissent is D's diary"
        );
        assert_eq!(dissent.signatures.len(), 1);
        assert_eq!(dissent.signatures[0].key, d_key);
        assert!(cs::signature_is_valid(
            &dissent.statement.hash(),
            &d_key,
            &dissent.signatures[0].der
        ));
        let d_listing = listed(&dissent.to_bytes(), &["confirmer2"]);
        assert!(matches!(
            confirmer_at(&rc, &client, &no_site, &c_net, &c_work, c_key, &env)
                .confirm_one(&d_listing)
                .await,
            Verdict::ListedDissent { .. }
        ));
        eprintln!(
            "[rehearsal] D's dissent {} at 100, signed by the engine",
            entry.statement_hash
        );
    }

    d.stop(&rd).await;
    c.stop(&rc).await;
    p.stop(&rp).await;
}
