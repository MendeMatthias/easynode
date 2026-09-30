//! Confirmed snapshots against a real engine: a statement two operators
//! signed, loaded on a regtest mirror through the app's own loading path
//! (`confirmed_load::load`), then the node restarted as a validating node
//! on it under the pin rule (`node::validating_snapshot_pin_args`), with the
//! pin on the command line and with the pin already in the node's
//! btx_rw.conf. Also the engine-bump check of section 12: the replay contexts
//! the app compiles are the engine's. The app's own launches are not run
//! here: that the launch after a mirror launch carries nothing left over is
//! node.rs's `a_mirror_load_marker_makes_only_the_load_launch_a_mirror`.
//! Opt-in, like the other shipped-engine tests:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test confirmed_snapshot_regtest -- --ignored --test-threads=1
//! ```
//!
//! btx-cli is taken from beside btxd, or from `EASYNODE_TEST_BTX_CLI`. The
//! keys are made fresh for each run and never written outside the scratch
//! folders. Regtest's ExactReplay starts at 101, so the chain stops at 100,
//! which is on the grid (`confirmed_snapshot::SNAPSHOT_GRID`).

use btx_core::attested_snapshot::{PairKind, ReadyPair};
use btx_core::confirmed_load::{self, CliRunner, Holds, LoadError};
use btx_core::confirmed_snapshot as cs;
use btx_core::known_invalid::HeldBranch;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::snapshot_start::{self, StartRecord, StartSource};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A fresh key: (regtest WIF, compressed public key hex).
fn regtest_key() -> (String, String) {
    let sk = k256::SecretKey::random(&mut rand_core::OsRng);
    let mut payload = vec![0xef];
    payload.extend_from_slice(&sk.to_bytes());
    payload.push(0x01);
    let wif = bs58::encode(payload).with_check().into_string();
    let pubkey = sk.public_key().to_encoded_point(true);
    (wif, btx_core::operators::hex(pubkey.as_bytes()))
}

/// One regtest btxd in its own folder, killed and reaped on drop.
struct Node {
    btxd: PathBuf,
    dir: PathBuf,
    rpc_port: u16,
    p2p_port: u16,
    child: Option<std::process::Child>,
}

impl Node {
    fn new(btxd: &Path, dir: PathBuf, rpc_port: u16) -> Self {
        std::fs::create_dir_all(dir.join("regtest")).unwrap();
        Self {
            btxd: btxd.to_path_buf(),
            dir,
            rpc_port,
            p2p_port: rpc_port + 100,
            child: None,
        }
    }

    /// The network folder: where the engine keeps regtest's chain.
    fn net(&self) -> PathBuf {
        self.dir.join("regtest")
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.net().join("debug.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(40)..].join("\n")
    }

    /// Kill and reap the engine, if one runs.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Start with `extra`, and wait for RPC. `Err` carries the log's tail
    /// when the engine exits instead, or does not answer in time (then it
    /// is killed first). An engine still running from before is killed, never
    /// left behind unreaped.
    async fn start(&mut self, extra: &[String]) -> Result<RpcClient, String> {
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
                "-listen=1",
                "-discover=0",
                "-dnsseed=0",
                "-fixedseeds=0",
                "-upnp=0",
                "-natpmp=0",
                "-printtoconsole=0",
                "-daemon=0",
            ])
            .args(extra)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;
        self.child = Some(child);
        let url = format!("http://127.0.0.1:{}", self.rpc_port);
        for _ in 0..240 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child = None;
                return Err(format!("btxd exited with {status}:\n{}", self.log_tail()));
            }
            if let Ok(c) = RpcClient::from_cookie(url.clone(), &self.net().join(".cookie")) {
                if c.call("getblockcount", json!([])).await.is_ok() {
                    return Ok(c);
                }
            }
        }
        self.kill();
        Err(format!("no RPC within 120 s:\n{}", self.log_tail()))
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

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn signer_args(pubkey: &str) -> Vec<String> {
    vec![
        "-matmulvalidation=consensus".into(),
        "-matmulattestationsignerkeyfile=signer.wif".into(),
        format!("-matmultrustedpubkey={pubkey}"),
        "-matmultrustedthreshold=1".into(),
    ]
}

fn mirror_args(pubkey: &str) -> Vec<String> {
    vec![
        "-matmulvalidation=trusted".into(),
        "-connect=0".into(),
        format!("-matmultrustedpubkey={pubkey}"),
        "-matmultrustedthreshold=1".into(),
    ]
}

async fn call(rpc: &RpcClient, method: &str, params: Value) -> Value {
    rpc.call(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

/// Mine until the node has a header at `height`, ten blocks a call, so a
/// call that outlasts the RPC client's 60 s on a busy machine only costs a
/// retry (the node keeps mining what it was asked for).
async fn mine_to(rpc: &RpcClient, height: u64) {
    for _ in 0..120 {
        let headers = call(rpc, "getblockchaininfo", json!([])).await["headers"]
            .as_u64()
            .unwrap_or(0);
        if headers >= height {
            return;
        }
        let batch = (height - headers).min(10);
        let _ = rpc
            .call("generatetodescriptor", json!([batch, "raw(51)"]))
            .await;
    }
    panic!("the node did not reach {height}");
}

/// Blocks to 99 and the header at 100 from the producer, as the spike fed
/// its mirrors: the producer serves no attestations, so a mirror cannot
/// connect block 100 itself and the snapshot at 100 is ahead of it.
async fn feed_headers(mirror: &RpcClient, producer: &Node) {
    call(
        mirror,
        "addnode",
        json!([format!("127.0.0.1:{}", producer.p2p_port), "onetry"]),
    )
    .await;
    for _ in 0..60 {
        let info = call(mirror, "getblockchaininfo", json!([])).await;
        if info["headers"].as_u64() >= Some(100) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the mirror never heard the producer's headers");
}

fn snapshot_base(chainstates: &Value) -> Option<String> {
    chainstates["chainstates"]
        .as_array()?
        .iter()
        .find_map(|c| c["snapshot_blockhash"].as_str().map(str::to_string))
}

fn copy_pair(to: &Path, file: &Path, manifest: &[u8]) -> ReadyPair {
    std::fs::create_dir_all(to).unwrap();
    let f = to.join("utxo-100.dat");
    let m = to.join("snapshot-manifest-100.json");
    std::fs::copy(file, &f).unwrap();
    std::fs::write(&m, manifest).unwrap();
    ReadyPair {
        kind: PairKind::Confirmed,
        height: 100,
        file: f,
        manifest: m,
    }
}

fn cli_path(btxd: &Path) -> PathBuf {
    std::env::var_os("EASYNODE_TEST_BTX_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| btxd.with_file_name("btx-cli"))
}

fn runner(cli: &Path, node: &Node) -> CliRunner {
    CliRunner {
        btx_cli: cli.to_path_buf(),
        args: vec![
            "-regtest".into(),
            format!("-datadir={}", node.dir.display()),
            format!("-rpcport={}", node.rpc_port),
        ],
    }
}

#[tokio::test]
#[ignore]
async fn a_two_operator_snapshot_loads_on_a_mirror_and_a_validating_node_restarts_on_it() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let cli = cli_path(&btxd);
    let root = tempfile::tempdir().unwrap();
    let (p_wif, p_pub) = regtest_key();
    let (c_wif, c_pub) = regtest_key();
    let env = format!("producer={p_pub};confirmer={c_pub}");
    let pins = [p_pub.as_str()];

    // The producer checks blocks, signs with P, and serves no attestations.
    let mut a = Node::new(&btxd, root.path().join("a"), 29471);
    std::fs::write(a.net().join("signer.wif"), format!("{p_wif}\n")).unwrap();
    let mut a_args = signer_args(&p_pub);
    a_args.push("-matmulattestationserve=0".into());
    let ra = a.start(&a_args).await.unwrap();
    // Block 100 is regtest's MatMul v4 height: the node mines it but connects
    // it only at its next start (as in the spike), so mine, then restart.
    mine_to(&ra, 100).await;
    a.stop(&ra).await;
    let ra = a.start(&a_args).await.unwrap();
    assert_eq!(call(&ra, "getblockcount", json!([])).await, json!(100));
    let dump = call(
        &ra,
        "dumptxoutsetattested",
        json!(["snap.dat", "snap.manifest"]),
    )
    .await;
    assert_eq!(dump["base_height"], json!(100), "{dump}");
    let base = dump["base_hash"].as_str().unwrap().to_string();
    let produced = std::fs::read(a.net().join("snap.manifest")).unwrap();
    let produced_st = cs::parse(&produced).unwrap().statement;
    // Section 12 on regtest: a fresh chain carries the shielded commitment
    // the app compiles.
    assert_eq!(
        produced_st.shielded().display_hex(),
        cs::REGTEST_SHIELDED_COMMITMENT
    );

    // The confirmer co-signs a copy with C.
    let mut c = Node::new(&btxd, root.path().join("c"), 29472);
    std::fs::write(c.net().join("signer.wif"), format!("{c_wif}\n")).unwrap();
    let rc = c.start(&signer_args(&c_pub)).await.unwrap();
    std::fs::write(c.net().join("work.manifest"), &produced).unwrap();
    call(&rc, "signutxosnapshotmanifest", json!(["work.manifest"])).await;
    let cosigned = std::fs::read(c.net().join("work.manifest")).unwrap();

    // A dissent (section 6a): the producer's chain facts, all four file
    // fields zero. Both nodes sign it with signutxosnapshotmanifest, which
    // checks the chain id and replay context and never the file fields.
    let unsigned = cs::Manifest {
        statement: produced_st.chain_facts().dissent(),
        signatures: vec![],
    }
    .to_bytes();
    std::fs::write(a.net().join("dissent.manifest"), &unsigned).unwrap();
    call(&ra, "signutxosnapshotmanifest", json!(["dissent.manifest"])).await;
    std::fs::copy(
        a.net().join("dissent.manifest"),
        c.net().join("dissent.manifest"),
    )
    .unwrap();
    call(&rc, "signutxosnapshotmanifest", json!(["dissent.manifest"])).await;
    let dissent = std::fs::read(c.net().join("dissent.manifest")).unwrap();
    c.stop(&rc).await;
    let m = cs::parse(&cosigned).unwrap();
    assert_eq!(m.signatures.len(), 2);
    assert_eq!(cs::parse(&produced).unwrap().signatures.len(), 1);

    // A mirror pinning only P, fed the producer's headers.
    let mut mirror = Node::new(&btxd, root.path().join("m"), 29473);
    let rm = mirror.start(&mirror_args(&p_pub)).await.unwrap();
    feed_headers(&rm, &a).await;

    // Section 12: the engine's replay context is the one the app compiles.
    let view = confirmed_load::node_view(&rm, &pins, 0).await;
    assert_eq!(
        view.genesis.as_deref(),
        Some(btx_core::operators::REGTEST_GENESIS)
    );
    assert_eq!(
        view.replay_context.as_deref(),
        Some(cs::REGTEST_REPLAY_CONTEXT)
    );

    // Refused before the engine sees anything (section 7, step 1), each on
    // its own, on the mirror that pins only P.
    // 1. A statement signed by one operator.
    let one = copy_pair(
        &mirror.net().join("one"),
        &a.net().join("snap.dat"),
        &produced,
    );
    let err = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &one,
        &view,
        &Holds::none(),
        Some(&env),
        &mirror.net(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, LoadError::NotConfirmed(e) if e.contains("two are needed")),
        "{err}"
    );

    // 2. A wrong file hash: the two-operator manifest, one byte of the file
    //    changed.
    let mut tampered = std::fs::read(a.net().join("snap.dat")).unwrap();
    tampered[100] ^= 1;
    std::fs::write(root.path().join("tampered.dat"), &tampered).unwrap();
    let wrong_file = copy_pair(
        &mirror.net().join("wrong-file"),
        &root.path().join("tampered.dat"),
        &cosigned,
    );
    let err = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &wrong_file,
        &view,
        &Holds::none(),
        Some(&env),
        &mirror.net(),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("not the one the statement signs"),
        "{err}"
    );

    // 3. A dissent: signed by both operators, one of them the pinned key,
    //    and its signatures verify. It is never loaded.
    let dm = cs::parse(&dissent).unwrap();
    assert!(cs::is_dissent(&dm.statement));
    let list = btx_core::operators::parse_env_list(&env).unwrap();
    assert_eq!(
        cs::confirming_operators(&dm, &list),
        Ok(vec!["producer".to_string(), "confirmer".to_string()])
    );
    let dissent_pair = copy_pair(
        &mirror.net().join("dissent"),
        &a.net().join("snap.dat"),
        &dissent,
    );
    let err = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &dissent_pair,
        &view,
        &Holds::none(),
        Some(&env),
        &mirror.net(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, LoadError::NotConfirmed(e) if e.contains("dissent")),
        "{err}"
    );

    // 4. Two operators, neither with a key the mirror pins: C's signature
    //    and one by a third key made here.
    let third = k256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
    let third_pub: [u8; 33] = third
        .verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .try_into()
        .unwrap();
    let cm = cs::parse(&cosigned).unwrap();
    let third_sig: k256::ecdsa::Signature = third.sign_prehash(&cm.statement.hash().0).unwrap();
    let unpinned = cs::Manifest {
        statement: cm.statement.clone(),
        signatures: vec![
            cm.signatures[1].clone(),
            cs::Signed {
                key: third_pub,
                der: third_sig.to_der().as_bytes().to_vec(),
            },
        ],
    };
    let env3 = format!("{env};third={}", btx_core::operators::hex(&third_pub));
    // Both signatures verify and name two operators under the list the load
    // reads, so the missing pin is the only thing wrong with it.
    let list3 = btx_core::operators::parse_env_list(&env3).unwrap();
    assert_eq!(
        cs::confirming_operators(&unpinned, &list3),
        Ok(vec!["confirmer".to_string(), "third".to_string()])
    );
    let unpinned_pair = copy_pair(
        &mirror.net().join("unpinned"),
        &a.net().join("snap.dat"),
        &unpinned.to_bytes(),
    );
    let err = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &unpinned_pair,
        &view,
        &Holds::none(),
        Some(&env3),
        &mirror.net(),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("no signature is from a key this node pins"),
        "{err}"
    );

    // 5. A disputed `latest` (section 6a): nothing is downloaded, and the
    //    start path takes the fallbacks.
    let mut site = mockito::Server::new_async().await;
    site.mock("GET", "/latest")
        .with_body(r#"{"disputed":[100]}"#)
        .create_async()
        .await;
    let fetched = site
        .mock("GET", mockito::Matcher::Regex("^/(m|f)".into()))
        .expect(0)
        .create_async()
        .await;
    let err = btx_core::attested_snapshot::prepare_confirmed(
        &reqwest::Client::new(),
        &format!("{}/latest", site.url()),
        &mirror.net(),
        &view,
        Some(&env),
        |_| true,
    )
    .await
    .unwrap_err();
    assert!(err.contains("disagree about block 100"), "{err}");
    fetched.assert_async().await;

    // None of them reached the engine or left a start record.
    assert_eq!(
        snapshot_base(&call(&rm, "getchainstates", json!([])).await),
        None
    );
    assert_eq!(snapshot_start::read(&mirror.net()), None);

    // Two operators load, trimmed to the one key the mirror pins.
    let pair = copy_pair(
        &mirror.net().join("pair"),
        &a.net().join("snap.dat"),
        &cosigned,
    );
    let loaded = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &pair,
        &view,
        &Holds::none(),
        Some(&env),
        &mirror.net(),
    )
    .await
    .unwrap();
    assert_eq!(
        (loaded.height, loaded.signatures, loaded.superseded),
        (100, 1, false)
    );
    // Section 7, step 4: the confirmers' names survive the trim.
    assert_eq!(
        snapshot_start::read(&mirror.net()),
        Some(StartRecord {
            height: 100,
            block_hash: base.clone(),
            source: StartSource::Confirmed,
            operators: vec!["producer".into(), "confirmer".into()],
        })
    );
    assert_eq!(
        std::fs::read(confirmed_load::trimmed_manifest_path(&pair)).unwrap(),
        produced,
        "trimming gave the producer's file back byte for byte"
    );
    assert_eq!(
        snapshot_base(&call(&rm, "getchainstates", json!([])).await),
        Some(base.clone())
    );
    mirror.stop(&rm).await;

    // Section 8: without the mirrors' pins a validating restart refuses to start ...
    let validating = args(&["-matmulvalidation=consensus", "-connect=0"]);
    let Err(err) = mirror.start(&validating).await else {
        panic!("a validating node on a signed snapshot started without the pins");
    };
    assert!(
        err.contains("failed verification under the current authority configuration"),
        "{err}"
    );
    // ... and with the pin rule's arguments it starts, validating, on the snapshot.
    let pin_args = btx_core::node::validating_snapshot_pin_args(&mirror.net(), &pins, &[]);
    assert_eq!(
        pin_args,
        vec![
            format!("-matmultrustedpubkey={p_pub}"),
            "-matmultrustedthreshold=1".into()
        ]
    );
    let mut with_pins = validating.clone();
    with_pins.extend(pin_args);
    let rv = mirror.start(&with_pins).await.unwrap();
    let status = call(&rv, "getmatmultrustedstatus", json!([])).await;
    assert_eq!(
        status["matmul_validation_mode"],
        json!("consensus"),
        "{status}"
    );
    assert_eq!(status["trusted_mirror"], json!(false), "{status}");
    assert_eq!(
        snapshot_base(&call(&rv, "getchainstates", json!([])).await),
        Some(base.clone())
    );
    mirror.stop(&rv).await;
    // A tripwire, not a proof: v0.34.9's daemon never writes btx_rw.conf
    // (only the Qt options model calls ModifyRWConfigFile, and never for
    // matmulvalidation), so a mirror launch cannot leave its mode there. If
    // an engine bump starts writing it, this fails first. The app-side
    // property, that the launch after the mirror launch carries nothing left
    // over, is node.rs's
    // `a_mirror_load_marker_makes_only_the_load_launch_a_mirror`.
    let rw_conf = mirror.net().join("btx_rw.conf");
    assert!(
        !rw_conf.exists(),
        "the engine wrote {} on its own",
        rw_conf.display()
    );

    // Section 8 once more, with the pin already in the node's btx_rw.conf:
    // the rule adds no key (the engine refuses a duplicate pin), only the
    // threshold, and the engine counts the btx_rw.conf pin toward its check
    // of the stored manifest.
    std::fs::write(&rw_conf, format!("matmultrustedpubkey={p_pub}\n")).unwrap();
    let already = btx_core::node::rw_conf_pins(&rw_conf);
    assert_eq!(already, vec![p_pub.clone()]);
    let pin_args = btx_core::node::validating_snapshot_pin_args(&mirror.net(), &pins, &already);
    assert_eq!(pin_args, vec!["-matmultrustedthreshold=1".to_string()]);
    let mut with_conf_pin = validating.clone();
    with_conf_pin.extend(pin_args);
    let rv = mirror.start(&with_conf_pin).await.unwrap();
    let status = call(&rv, "getmatmultrustedstatus", json!([])).await;
    assert_eq!(
        status["matmul_validation_mode"],
        json!("consensus"),
        "{status}"
    );
    assert_eq!(status["trusted_mirror"], json!(false), "{status}");
    assert_eq!(status["trusted_signer_pubkeys"], json!([p_pub]), "{status}");
    assert_eq!(
        snapshot_base(&call(&rv, "getchainstates", json!([])).await),
        Some(base.clone())
    );
    mirror.stop(&rv).await;
    std::fs::remove_file(&rw_conf).unwrap();

    // A base above a held block: the engine refuses it once the app has
    // refused the block (section 7, step 3).
    let mut held = Node::new(&btxd, root.path().join("h"), 29474);
    let rh = held.start(&mirror_args(&p_pub)).await.unwrap();
    feed_headers(&rh, &a).await;
    let root50: &'static str = Box::leak(
        call(&ra, "getblockhash", json!([50]))
            .await
            .as_str()
            .unwrap()
            .to_string()
            .into_boxed_str(),
    );
    let branch = [HeldBranch {
        height: 50,
        root: root50,
        why: "rehearsal",
    }];
    let view_h = confirmed_load::node_view(&rh, &pins, 0).await;
    let pair_h = copy_pair(
        &held.net().join("pair"),
        &a.net().join("snap.dat"),
        &cosigned,
    );
    let err = confirmed_load::load(
        &rh,
        &runner(&cli, &held),
        &pair_h,
        &view_h,
        &Holds {
            invalid: &[],
            held: &branch,
        },
        Some(&env),
        &held.net(),
    )
    .await
    .unwrap_err();
    // v0.34.9 answers "Attested snapshot base is incompatible with the
    // current best-header chain" here (measured 2026-09-29 and 2026-09-30),
    // and "part of an invalid chain" only for a base whose own header is
    // marked failed, which this one is not.
    assert!(
        matches!(&err, LoadError::Engine(e)
            if e.contains("incompatible with the current best-header chain")),
        "{err}"
    );
    assert_eq!(
        snapshot_base(&call(&rh, "getchainstates", json!([])).await),
        None
    );
    assert_eq!(
        snapshot_start::read(&held.net()),
        None,
        "a refused load leaves no start record"
    );
    // The hold was the reason: the mirror above loaded this same pair from
    // the same producer's headers. (Lifting the hold with reconsiderblock on
    // this node and loading again was tried: 2 runs of 6, v0.34.9 stopped
    // answering RPC after the refused load, so the control is the mirror.)
    held.stop(&rh).await;
    a.stop(&ra).await;
}

/// Kills and reaps the mainnet-parameters btxd on drop, including on panic,
/// so a failed assertion never leaves it running.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Section 12, mainnet: an engine bump that moves the replay context would
/// strand every node on a signed snapshot, so it fails here first. Mainnet
/// parameters, a scratch folder, no peers, the mirror arm's own flags.
#[tokio::test]
#[ignore]
async fn the_engine_reports_the_compiled_mainnet_replay_context() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(&btxd);
    cmd.arg(format!("-datadir={}", dir.path().display()))
        .args([
            // No peers: nothing in, nothing out, no seeds of any kind.
            "-listen=0",
            "-connect=0",
            "-dnsseed=0",
            "-fixedseeds=0",
            "-discover=0",
            "-upnp=0",
            "-natpmp=0",
            "-rpcport=29479",
            "-server=1",
            "-prune=550",
            "-printtoconsole=0",
            "-daemon=0",
            "-matmulvalidation=trusted",
            "-matmultrustedthreshold=1",
            "-allowsinglekeytrustedmirror=1",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for k in btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS {
        cmd.arg(format!("-matmultrustedpubkey={k}"));
    }
    let mut child = KillOnDrop(cmd.spawn().unwrap());
    let cookie = dir.path().join(".cookie");
    let mut status = None;
    for _ in 0..360 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(code) = child.0.try_wait().unwrap() {
            panic!("btxd exited with {code}");
        }
        if let Ok(c) = RpcClient::from_cookie("http://127.0.0.1:29479", &cookie) {
            if let Ok(v) = c.call("getmatmultrustedstatus", json!([])).await {
                status = Some((c, v));
                break;
            }
        }
    }
    let (rpc, v) = status.expect("btxd answered within 180 s");
    let peers = rpc.call("getconnectioncount", json!([])).await;
    let _ = rpc.call("stop", json!([])).await;
    for _ in 0..120 {
        if child.0.try_wait().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(peers.ok(), Some(json!(0)), "no peer, ever, on mainnet");
    assert_eq!(
        v["replay_authority_context"].as_str(),
        Some(cs::MAINNET_REPLAY_CONTEXT),
        "{v}"
    );
}

/// Fast-forward's roll-back on a real engine's folder: set the chain aside,
/// start on nothing, put it back, and the node is where it was.
#[tokio::test]
#[ignore]
async fn fast_forward_puts_a_real_chain_back() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let mut n = Node::new(&btxd, root.path().join("f"), 29475);
    let validating = args(&["-matmulvalidation=consensus", "-connect=0"]);
    let r = n.start(&validating).await.unwrap();
    mine_to(&r, 50).await;
    let best = call(&r, "getbestblockhash", json!([])).await;
    n.stop(&r).await;

    let record = btx_core::fast_forward::set_aside(
        &n.net(),
        50,
        btx_core::fast_forward::Before::default(),
        1,
    )
    .unwrap();
    assert!(record.moved.contains(&"blocks".to_string()), "{record:?}");
    let r = n.start(&validating).await.unwrap();
    assert_eq!(
        call(&r, "getblockcount", json!([])).await,
        json!(0),
        "a fresh chain"
    );
    n.stop(&r).await;

    btx_core::fast_forward::restore(&n.net()).unwrap();
    let r = n.start(&validating).await.unwrap();
    assert_eq!(call(&r, "getblockcount", json!([])).await, json!(50));
    assert_eq!(call(&r, "getbestblockhash", json!([])).await, best);
    n.stop(&r).await;
}

/// Fast-forward's other end on a real engine's folder: set the old chain
/// aside, start fresh on a new one, and `finish` keeps the new chain and
/// leaves no dated folder or record behind.
#[tokio::test]
#[ignore]
async fn fast_forward_finish_keeps_a_real_new_chain() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let mut n = Node::new(&btxd, root.path().join("g"), 29476);
    let validating = args(&["-matmulvalidation=consensus", "-connect=0"]);
    let r = n.start(&validating).await.unwrap();
    mine_to(&r, 20).await;
    n.stop(&r).await;

    let record = btx_core::fast_forward::set_aside(
        &n.net(),
        20,
        btx_core::fast_forward::Before::default(),
        2,
    )
    .unwrap();
    let aside_dir = n.net().join(&record.aside);
    assert!(aside_dir.is_dir(), "{}", aside_dir.display());

    let r = n.start(&validating).await.unwrap();
    assert_eq!(
        call(&r, "getblockcount", json!([])).await,
        json!(0),
        "a fresh chain"
    );
    mine_to(&r, 5).await;
    let new_best = call(&r, "getbestblockhash", json!([])).await;
    n.stop(&r).await;

    btx_core::fast_forward::finish(&n.net()).unwrap();
    assert!(
        !aside_dir.exists(),
        "the dated folder should be gone: {}",
        aside_dir.display()
    );
    // finish renames the folder to *.discard before it deletes it, and only
    // warns if that last delete fails: check the renamed folder is gone too.
    let discard = aside_dir.with_file_name(format!(
        "{}.discard",
        aside_dir.file_name().unwrap().to_string_lossy()
    ));
    assert!(
        !discard.exists(),
        "the renamed folder should be gone: {}",
        discard.display()
    );
    assert_eq!(
        btx_core::fast_forward::read_record(&n.net()).unwrap(),
        None,
        "no run should be recorded once finish is done"
    );

    let r = n.start(&validating).await.unwrap();
    assert_eq!(call(&r, "getblockcount", json!([])).await, json!(5));
    assert_eq!(call(&r, "getbestblockhash", json!([])).await, new_best);
    n.stop(&r).await;
}
