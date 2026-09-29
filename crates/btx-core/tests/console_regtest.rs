//! Every row of the command table through a real v0.34.9 btxd on regtest.
//! Opt-in, like the other shipped-engine test (`signer::tests::
//! the_shipped_engine_signs_with_a_generated_key`):
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test console_regtest -- --ignored
//! ```
//!
//! This does not just spot-check a few commands: it runs `console_policy::
//! decide` over every allowed row (the exact strings the command window
//! accepts) and sends whatever `Call` it produces straight to the engine, so
//! a method name typo'd in the policy or an argument shape the engine no
//! longer likes shows up here instead of in the window. The two confirm-class
//! rows are checked to still decide `Confirm` (never run — a real `addnode`
//! or `getblockfrompeer` needs a peer this offline node doesn't have), and a
//! handful of refused rows are checked to never reach `decide` as anything
//! but `Refuse`, so the engine never even sees them.

use btx_core::console_policy::{decide, Decision};
use btx_core::error::AppError;
use btx_core::rpc::{Rpc, RpcClient};

/// True when `e` is the shape `RpcClient::call` produces for a call the
/// engine actually rejected — an unknown method or a wrong-typed argument —
/// as opposed to a call it accepted but answered with a data-level RPC
/// error (e.g. the genesis coinbase being unspendable/pruned).
///
/// Measured against the real v0.34.9 engine on regtest (see
/// `task-9-report.md`'s "Fix round" section for the exact values):
///   - unknown method: btxd answers a bare HTTP 404 with no JSON-RPC body.
///     `RpcClient::call` only parses the JSON-RPC error object on a 5xx
///     (crates/btx-core/src/rpc.rs, the `is_server_error()` branch), so a
///     404 carries no error code at all and surfaces as a generic
///     `AppError::Http("node RPC error (HTTP 404 Not Found)")`.
///   - wrong-typed argument: this rides an HTTP 500, so its JSON-RPC error
///     object IS preserved: `AppError::Rpc { code: -3, message: "Wrong
///     type passed: ..." }` (-3 is Bitcoin Core's RPC_TYPE_ERROR).
///
/// `-32601` (the standard JSON-RPC "Method not found" code) is accepted
/// too, in case a future engine build answers an unknown method over a 5xx
/// instead of the 404 this one uses — the point is the *shape* of an engine
/// rejection, not one specific status code.
fn engine_rejected_the_call(e: &AppError) -> bool {
    match e {
        AppError::Http(msg) => msg.contains("404"),
        AppError::Rpc { code, .. } => *code == -3 || *code == -32601,
        _ => false,
    }
}

/// Kills and reaps the spawned btxd on drop, including on panic: an
/// assertion failing partway through the row loop must not leave a regtest
/// node listening on 29444 for the next run of this test (or anything else
/// on that port).
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore]
async fn every_row_behaves_against_the_shipped_engine() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(&btxd)
        .arg(format!("-datadir={}", dir.path().display()))
        .args([
            "-regtest",
            "-listen=0",
            "-connect=0",
            "-dnsseed=0",
            "-rpcport=29444",
            "-server=1",
            "-printtoconsole=0",
            "-daemon=0",
        ])
        .spawn()
        .unwrap();
    // From here on, every early return (a panicking `unwrap`/`expect`/assert)
    // still runs `KillOnDrop::drop`, so btxd is always killed and reaped.
    let _guard = KillOnDrop(child);

    let cookie = dir.path().join("regtest").join(".cookie");
    let mut rpc = None;
    for _ in 0..60 {
        if let Ok(c) = RpcClient::from_cookie("http://127.0.0.1:29444", &cookie) {
            if c.call("getblockcount", serde_json::json!([])).await.is_ok() {
                rpc = Some(c);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let rpc = rpc.expect("regtest btxd answered within 30s");

    let genesis = rpc
        .call("getblockhash", serde_json::json!([0]))
        .await
        .unwrap();
    let g = genesis.as_str().unwrap().to_string();
    let block = rpc
        .call("getblock", serde_json::json!([g, 1]))
        .await
        .unwrap();
    let coinbase = block["tx"][0].as_str().unwrap().to_string();

    // Negative control: prove `engine_rejected_the_call` can actually say
    // "true" against this same engine and client, before relying on it below
    // to say "false" for every allowed row. Without this, a helper that
    // always returned `false` would make the row check vacuous.
    let unknown_method = rpc.call("getblockcounts", serde_json::json!([])).await;
    assert!(
        matches!(&unknown_method, Err(e) if engine_rejected_the_call(e)),
        "a misspelled method must be recognized as rejected: {unknown_method:?}"
    );
    let wrong_type = rpc.call("getblockhash", serde_json::json!(["x"])).await;
    assert!(
        matches!(&wrong_type, Err(e) if engine_rejected_the_call(e)),
        "a wrong-typed argument must be recognized as rejected: {wrong_type:?}"
    );

    // Every read-only row the command window can produce, in the exact text
    // a person would type. `decide` must turn each into `Run`, and the
    // engine must accept the method name and argument shape `decide` chose.
    let runs = [
        "getblockchaininfo".to_string(),
        "getchaintips".into(),
        "getpeerinfo".into(),
        "getnetworkinfo".into(),
        "getmempoolinfo".into(),
        "getmatmulattestedtip".into(),
        "getmatmultrustedstatus".into(),
        "uptime".into(),
        "getchainstates".into(),
        "getblockcount".into(),
        "getbestblockhash".into(),
        "getconnectioncount".into(),
        "getblockhash 0".into(),
        format!("getblockheader {g}"),
        format!("getblock {g} 2"),
        format!("getrawtransaction {coinbase} 1 {g}"),
        format!("gettxout {coinbase} 0"),
        "help getblock".into(),
    ];
    for line in &runs {
        let Decision::Run(call) = decide(line) else {
            panic!("{line} did not decide Run");
        };
        let answer = rpc
            .call(&call.method, serde_json::Value::Array(call.params))
            .await;
        // getmatmulattestedtip may answer an RPC error on a keyless regtest
        // node, and the genesis coinbase cannot be fetched with
        // getrawtransaction/gettxout on a Bitcoin-derived engine either
        // (spent-and-pruned by construction). What matters here is only that
        // the engine accepted the method name and the argument shape decide
        // produced — i.e. that this was not a rejection of the kind the
        // negative control above just proved `engine_rejected_the_call`
        // catches — not that every row returns data.
        if let Err(e) = &answer {
            assert!(!engine_rejected_the_call(e), "{line}: {e}");
        }
    }

    // Confirm-class rows decide Confirm, and are never sent to the engine:
    // an offline node with -connect=0 has no peer to try or to ask.
    for line in [
        "addnode 127.0.0.1:1 onetry".to_string(),
        format!("getblockfrompeer {g} 0"),
    ] {
        assert!(matches!(decide(&line), Decision::Confirm { .. }), "{line}");
    }

    // Refused rows never even reach the engine.
    for line in ["stop", "invalidateblock 00", "dumpprivkey x", "savemempool"] {
        assert!(matches!(decide(line), Decision::Refuse(_)), "{line}");
    }

    let _ = rpc.call("stop", serde_json::json!([])).await;
    // `_guard` drops here (or earlier, on panic) and kills/reaps btxd.
}
