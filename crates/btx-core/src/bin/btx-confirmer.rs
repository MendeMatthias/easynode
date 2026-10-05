//! `btx-confirmer`: the snapshot network's confirmer, beside a plain btxd.
//!
//! An operator whose node the easyNode app does not run still counts as a
//! confirmer (section 5a of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md). This runs the app's own code for it: the diary
//! (`btx_core::diary`, read every few seconds, written at every multiple of
//! 100), and every ten minutes a round of `btx_core::snapshot_confirmer`
//! (the `getchainstates` rule, the four checks, a signature from the node's
//! own `signutxosnapshotmanifest`, and the upload of that signature or of a
//! dissent). It adds argument parsing, the startup check and the loop.
//!
//!     btx-confirmer --datadir /var/lib/btx --state /var/lib/btx-confirmer
//!
//! It never exports, loads, pins or restarts anything, and listens on
//! nothing.
//!
//! WHAT IT TOUCHES. It reads the node's `.cookie` (and re-reads it after a
//! 401, so a btxd restart does not stop it) and the PUBLIC half of the node's
//! signing key: the file the node's conf names in
//! `matmulattestationsignerkeyfile=`, or `<datadir>/attestation-signer.key`,
//! or `--signer-key`. The private key never leaves that file; the engine
//! signs with it. It writes the diary and its log of what it signed in
//! `--state`, and the copy the engine signs in `<datadir>/snapshot-confirmer/`,
//! because `signutxosnapshotmanifest` reads and rewrites a file on the node's
//! own machine.
//!
//! IT REFUSES TO START (exit 3, which the unit does not restart) on a node
//! whose chain is not one the app knows, whose replay context is not the
//! one compiled for that chain, that follows signatures instead of checking
//! blocks, that has no signing key, or whose key is not on the operator list.
//! A node that does not answer yet is exit 1, which the unit restarts.

use btx_core::confirmed_load::Holds;
use btx_core::confirmed_snapshot as cs;
use btx_core::diary::{self, DiaryOutcome};
use btx_core::node_api::{self, MatmulTrustedStatus};
use btx_core::operators::{self, Chain};
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::snapshot_confirmer::{self as confirmer, Confirmer, Tally, CONFIRM_EVERY_SECS};
use btx_core::snapshot_site::{self, Site};
use btx_core::{role, setup, signer};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "\
btx-confirmer: co-sign the snapshot network's statements, or dissent, by this node's own diary

USAGE:
    btx-confirmer --datadir <path> --state <dir> [--rpc <addr:port>] [--signer-key <file>]

OPTIONS:
    --datadir <path>     the node's data directory, holding its .cookie
    --state <dir>        where the diary and the log of what was signed are kept
    --rpc <addr:port>    the node's JSON-RPC (default 127.0.0.1:19334)
    --signer-key <file>  the node's signing key file, read for its public key only
                         (default: the file btx.conf names, else attestation-signer.key)
    -h, --help           this

The node must check blocks itself and sign with a key on the operator list.
The copy the node signs is written to <datadir>/snapshot-confirmer/.
";

/// Under the datadir: where the engine signs a copy (section 5a).
const WORK_DIR: &str = "snapshot-confirmer";
/// How often the tip is read for the diary.
const TIP_EVERY_SECS: u64 = 5;
/// `gettxoutsetinfo` hashes the whole UTXO set and a round can outlast the
/// default: the app gives these calls the same.
const SLOW_RPC_TIMEOUT: Duration = Duration::from_secs(600);
/// A summary line once an hour.
const SUMMARY_EVERY_ROUNDS: u64 = 3600 / CONFIRM_EVERY_SECS;
/// Exit status for a refusal: the unit's `RestartPreventExitStatus`.
const EXIT_REFUSED: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    datadir: PathBuf,
    state: PathBuf,
    rpc: String,
    signer_key: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    Run(Args),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Parsed, String> {
    let mut datadir = None;
    let mut state = None;
    let mut rpc = "127.0.0.1:19334".to_string();
    let mut signer_key = None;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--datadir" => datadir = Some(PathBuf::from(value("--datadir")?)),
            "--state" => state = Some(PathBuf::from(value("--state")?)),
            "--rpc" => rpc = value("--rpc")?,
            "--signer-key" => signer_key = Some(PathBuf::from(value("--signer-key")?)),
            "-h" | "--help" => return Ok(Parsed::Help),
            other => return Err(format!("unknown option: {other}")),
        }
    }
    let datadir = datadir.ok_or("--datadir is required")?;
    let state = state.ok_or("--state is required")?;
    Ok(Parsed::Run(Args {
        datadir,
        state,
        rpc,
        signer_key,
    }))
}

/// The signing key file: `--signer-key`, else the one the datadir's
/// `btx.conf` names (relative to the datadir, as the engine resolves it),
/// else the app's default name.
fn signer_key_path(args: &Args) -> PathBuf {
    if let Some(p) = &args.signer_key {
        return p.clone();
    }
    let conf = args.datadir.join("btx.conf");
    match setup::conf_kv(&conf, signer::SIGNER_KEY_CONF_KEY) {
        Some(named) if !named.trim().is_empty() => {
            let named = Path::new(named.trim());
            if named.is_absolute() {
                named.to_path_buf()
            } else {
                args.datadir.join(named)
            }
        }
        _ => signer::signer_key_path(&args.datadir),
    }
}

/// The public key of the WIF in `path`, or `None`.
fn read_pubkey(path: &Path) -> Option<String> {
    let wif = std::fs::read_to_string(path).ok()?;
    signer::wif_to_pubkey_hex(&wif).ok()
}

/// The replay context compiled for `chain`, display order.
fn compiled_replay_context(chain: Chain) -> &'static str {
    match chain {
        Chain::Main => cs::MAINNET_REPLAY_CONTEXT,
        Chain::Regtest => cs::REGTEST_REPLAY_CONTEXT,
    }
}

/// The startup decision, from what the node said: its genesis
/// (`getblockhash 0`), its `getmatmultrustedstatus`, and the public key of
/// its signing key file. The chain and this node's key, or why it may not
/// confirm.
fn startup_check(
    genesis: &str,
    status: &MatmulTrustedStatus,
    our_key_hex: Option<&str>,
    regtest_env: Option<&str>,
) -> Result<(Chain, [u8; 33]), String> {
    let Some(chain) = Chain::from_genesis_hex(genesis) else {
        return Err(format!(
            "the node is on chain {genesis}, which has no snapshot network"
        ));
    };
    let compiled = compiled_replay_context(chain);
    match status.replay_authority_context.as_deref() {
        None => return Err("the node did not say which replay context it runs with".into()),
        Some(c) if !c.trim().eq_ignore_ascii_case(compiled) => {
            return Err(format!(
                "the node's replay context is {}, not {compiled}, the one this build knows",
                c.trim()
            ))
        }
        Some(_) => {}
    }
    if let Some(why) = confirmer::why_not(status, our_key_hex, genesis, regtest_env) {
        return Err(format!("this node does not confirm: {why}"));
    }
    our_key_hex
        .and_then(operators::parse_key)
        .map(|k| (chain, k))
        .ok_or_else(|| "its signing key could not be read".into())
}

/// What a diary step said, for the log. `None` when it is not news.
fn diary_note(outcome: &Result<DiaryOutcome, String>) -> Option<String> {
    match outcome {
        Ok(DiaryOutcome::Recorded(e)) => Some(format!(
            "wrote {} (block {}, {} coins)",
            e.height,
            e.block_hash.get(..16).unwrap_or(&e.block_hash),
            e.coins
        )),
        Ok(DiaryOutcome::TipMoved) => Some(
            "the tip moved while the chain state was read; the next chance is 100 blocks later"
                .into(),
        ),
        Ok(DiaryOutcome::NotValidating) => {
            Some("not kept: this node follows signatures instead of checking blocks itself".into())
        }
        Ok(DiaryOutcome::Unvalidated) => {
            Some("not kept: this node is still checking older history in the background".into())
        }
        Ok(DiaryOutcome::AlreadyRecorded(_) | DiaryOutcome::NotOnGrid) => None,
        Err(e) => Some(e.clone()),
    }
}

fn refuse(msg: impl std::fmt::Display) -> ! {
    eprintln!("[confirmer] refusing to run: {msg}");
    std::process::exit(EXIT_REFUSED);
}

#[tokio::main]
async fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(a)) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // Cookie auth, as btx-witness and the app do: no password on a command
    // line. The client re-reads the cookie after a 401.
    let cookie = args.datadir.join(".cookie");
    let client = match RpcClient::from_cookie(format!("http://{}", args.rpc), &cookie) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "cannot read {}: {e}\nIs the node running, and is this its datadir?",
                cookie.display()
            );
            std::process::exit(1);
        }
    };
    let slow = client.with_timeout(SLOW_RPC_TIMEOUT);

    // What the node says about itself, asked once here and refused on any
    // mismatch with what this build compiles.
    let genesis = match client.call("getblockhash", json!([0])).await {
        Ok(v) => v.as_str().unwrap_or_default().to_ascii_lowercase(),
        Err(e) => {
            eprintln!("the node did not answer getblockhash 0: {e}");
            std::process::exit(1);
        }
    };
    let status = match node_api::get_matmul_trusted_status(&client).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("the node did not answer getmatmultrustedstatus: {e}");
            std::process::exit(1);
        }
    };
    let key_path = signer_key_path(&args);
    let key_hex = read_pubkey(&key_path);
    let env = operators::regtest_env();
    let (chain, our_key) =
        match startup_check(&genesis, &status, key_hex.as_deref(), env.as_deref()) {
            Ok(v) => v,
            Err(e) if key_hex.is_none() => refuse(format!("{e} ({})", key_path.display())),
            Err(e) => refuse(e),
        };
    let site = Site::from_env().unwrap_or_else(|e| refuse(e));
    let http = snapshot_site::client().unwrap_or_else(|e| refuse(e));
    if let Err(e) = std::fs::create_dir_all(&args.state) {
        refuse(format!("cannot create {}: {e}", args.state.display()));
    }
    let work = args.datadir.join(WORK_DIR);

    eprintln!(
        "[confirmer] {} node, key {}, diary in {}, copies signed in {}, statements from {}",
        match chain {
            Chain::Main => "mainnet",
            Chain::Regtest => "regtest",
        },
        key_hex.as_deref().unwrap_or_default(),
        args.state.display(),
        work.display(),
        site.as_str()
    );
    if let Some(s) = diary::summary(&args.state) {
        eprintln!("[confirmer] diary: {s}");
    }

    let diary_loop = async {
        let mut last: Option<String> = None;
        loop {
            let tip = client
                .call("getblockcount", json!([]))
                .await
                .ok()
                .and_then(|v| v.as_u64());
            if tip.is_some_and(diary::on_grid) {
                let status = node_api::get_matmul_trusted_status(&client).await.ok();
                let mode = role::validation_mode(status.as_ref());
                let outcome = diary::record_at_tip(&slow, &args.state, mode).await;
                if let Some(note) = diary_note(&outcome) {
                    if last.as_deref() != Some(note.as_str()) {
                        eprintln!("[confirmer] diary: {note}");
                        last = Some(note);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(TIP_EVERY_SECS)).await;
        }
    };

    let confirm_loop = async {
        let mut tally = Tally::default();
        let mut seen = HashSet::new();
        let mut off: Option<String> = None;
        let c = Confirmer {
            rpc: &slow,
            client: &http,
            site: &site,
            state_dir: &args.state,
            work_dir: &work,
            our_key,
            holds: Holds::compiled(),
            regtest_env: env.as_deref(),
        };
        // The first round after one minute, so the diary step has run.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            // The standing gate again: a btxd restarted with another conf
            // must not be signed for.
            let status = node_api::get_matmul_trusted_status(&client).await.ok();
            let why = match &status {
                None => Some("the node did not answer getmatmultrustedstatus".to_string()),
                Some(s) => confirmer::why_not(s, key_hex.as_deref(), &genesis, env.as_deref())
                    .map(str::to_string),
            };
            if let Some(why) = why {
                if off.as_deref() != Some(why.as_str()) {
                    eprintln!("[confirmer] not confirming this round: {why}");
                    off = Some(why);
                }
            } else {
                off = None;
                let lines = match c.round(chain).await {
                    Ok(verdicts) => tally.add(&verdicts, &mut seen),
                    Err(why) => tally.stopped(&why, &mut seen).into_iter().collect(),
                };
                for line in lines {
                    eprintln!("[confirmer] {line}");
                }
                if tally.rounds % SUMMARY_EVERY_ROUNDS == 0 {
                    for line in tally.report() {
                        eprintln!("[confirmer] {line}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(CONFIRM_EVERY_SECS)).await;
        }
    };

    // Both run until the supervisor stops the process. Every file is
    // written atomically, so there is nothing to flush.
    tokio::join!(diary_loop, confirm_loop);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Parsed, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    fn status(mode: &str, signer: bool, context: Option<&str>) -> MatmulTrustedStatus {
        MatmulTrustedStatus {
            matmul_validation_mode: mode.into(),
            local_signer: signer,
            replay_authority_context: context.map(str::to_string),
            ..Default::default()
        }
    }

    /// A fresh key, and a regtest operator list that names it.
    fn listed_key() -> (String, String) {
        let key = signer::wif_to_pubkey_hex(&signer::generate_wif()).unwrap();
        let env = format!("me={key}");
        (key, env)
    }

    #[test]
    fn the_datadir_and_the_state_are_required_and_the_rest_defaults() {
        assert_eq!(
            args(&["--datadir", "/d", "--state", "/s"]),
            Ok(Parsed::Run(Args {
                datadir: "/d".into(),
                state: "/s".into(),
                rpc: "127.0.0.1:19334".into(),
                signer_key: None,
            }))
        );
        assert_eq!(
            args(&["--state", "/s"]),
            Err("--datadir is required".into())
        );
        assert_eq!(
            args(&["--datadir", "/d"]),
            Err("--state is required".into())
        );
    }

    #[test]
    fn every_option_reads_and_help_wins() {
        assert_eq!(
            args(&[
                "--datadir",
                "/d",
                "--state",
                "/s",
                "--rpc",
                "127.0.0.1:19434",
                "--signer-key",
                "/k"
            ]),
            Ok(Parsed::Run(Args {
                datadir: "/d".into(),
                state: "/s".into(),
                rpc: "127.0.0.1:19434".into(),
                signer_key: Some("/k".into()),
            }))
        );
        assert_eq!(args(&["--help"]), Ok(Parsed::Help));
        assert_eq!(args(&["--datadir", "/d", "-h"]), Ok(Parsed::Help));
    }

    #[test]
    fn an_unknown_option_or_a_missing_value_is_refused() {
        assert_eq!(
            args(&["--datadir", "/d", "--listen", "x"]),
            Err("unknown option: --listen".into())
        );
        assert_eq!(args(&["--datadir"]), Err("--datadir needs a value".into()));
        assert_eq!(
            args(&["--datadir", "--state", "/s"]),
            Err("--datadir needs a value".into())
        );
    }

    #[test]
    fn the_key_file_is_the_flag_then_the_conf_then_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Args {
            datadir: dir.path().into(),
            state: dir.path().join("state"),
            rpc: "127.0.0.1:19334".into(),
            signer_key: None,
        };
        assert_eq!(
            signer_key_path(&a),
            dir.path().join(signer::SIGNER_KEY_FILE)
        );
        std::fs::write(
            dir.path().join("btx.conf"),
            "server=1\nmatmulattestationsignerkeyfile=keys/signer.wif\n",
        )
        .unwrap();
        assert_eq!(signer_key_path(&a), dir.path().join("keys/signer.wif"));
        std::fs::write(
            dir.path().join("btx.conf"),
            "matmulattestationsignerkeyfile=/etc/btx/signer.wif\n",
        )
        .unwrap();
        assert_eq!(signer_key_path(&a), PathBuf::from("/etc/btx/signer.wif"));
        a.signer_key = Some("/elsewhere/k".into());
        assert_eq!(signer_key_path(&a), PathBuf::from("/elsewhere/k"));
    }

    #[test]
    fn a_listed_validating_signer_with_its_chains_replay_context_starts() {
        let (key, env) = listed_key();
        let s = status("consensus", true, Some(cs::REGTEST_REPLAY_CONTEXT));
        let (chain, k) =
            startup_check(operators::REGTEST_GENESIS, &s, Some(&key), Some(&env)).unwrap();
        assert_eq!(chain, Chain::Regtest);
        assert_eq!(operators::hex(&k), key);
        // Case does not matter.
        let upper = status(
            "consensus",
            true,
            Some(&cs::REGTEST_REPLAY_CONTEXT.to_ascii_uppercase()),
        );
        assert!(startup_check(operators::REGTEST_GENESIS, &upper, Some(&key), Some(&env)).is_ok());
    }

    #[test]
    fn another_chain_refuses_to_start() {
        let (key, env) = listed_key();
        let s = status("consensus", true, Some(cs::REGTEST_REPLAY_CONTEXT));
        let other = "11".repeat(32);
        let e = startup_check(&other, &s, Some(&key), Some(&env)).unwrap_err();
        assert!(e.contains("has no snapshot network"), "{e}");
    }

    #[test]
    fn another_replay_context_or_none_refuses_to_start() {
        let (key, env) = listed_key();
        // Mainnet's context on a regtest node, regtest's on a mainnet node.
        let s = status("consensus", true, Some(cs::MAINNET_REPLAY_CONTEXT));
        let e = startup_check(operators::REGTEST_GENESIS, &s, Some(&key), Some(&env)).unwrap_err();
        assert!(e.contains("replay context is"), "{e}");
        let s = status("consensus", true, Some(cs::REGTEST_REPLAY_CONTEXT));
        let e = startup_check(operators::MAINNET_GENESIS, &s, Some(&key), None).unwrap_err();
        assert!(e.contains("replay context is"), "{e}");
        let s = status("consensus", true, None);
        let e = startup_check(operators::REGTEST_GENESIS, &s, Some(&key), Some(&env)).unwrap_err();
        assert!(e.contains("did not say which replay context"), "{e}");
    }

    #[test]
    fn a_mirror_a_node_without_a_key_or_an_unlisted_key_refuses_to_start() {
        let (key, env) = listed_key();
        let env = Some(env.as_str());
        let ctx = Some(cs::REGTEST_REPLAY_CONTEXT);
        let g = operators::REGTEST_GENESIS;
        let e = startup_check(g, &status("trusted", true, ctx), Some(&key), env).unwrap_err();
        assert!(e.contains("follows signatures"), "{e}");
        let e = startup_check(g, &status("consensus", false, ctx), Some(&key), env).unwrap_err();
        assert!(e.contains("does not sign"), "{e}");
        let e = startup_check(g, &status("consensus", true, ctx), None, env).unwrap_err();
        assert!(e.contains("could not be read"), "{e}");
        let (unlisted, _) = listed_key();
        let e =
            startup_check(g, &status("consensus", true, ctx), Some(&unlisted), env).unwrap_err();
        assert!(e.contains("not on the operator list"), "{e}");
    }

    #[test]
    fn the_public_key_is_read_from_a_wif_file_and_nothing_else_reads() {
        let dir = tempfile::tempdir().unwrap();
        let wif = signer::generate_wif();
        let p = dir.path().join("k");
        std::fs::write(&p, format!("{wif}\n")).unwrap();
        assert_eq!(
            read_pubkey(&p),
            Some(signer::wif_to_pubkey_hex(&wif).unwrap())
        );
        std::fs::write(&p, "not a key").unwrap();
        assert_eq!(read_pubkey(&p), None);
        assert_eq!(read_pubkey(&dir.path().join("missing")), None);
    }

    #[test]
    fn the_diary_says_only_news() {
        assert_eq!(diary_note(&Ok(DiaryOutcome::NotOnGrid)), None);
        assert_eq!(diary_note(&Ok(DiaryOutcome::AlreadyRecorded(100))), None);
        assert!(diary_note(&Ok(DiaryOutcome::Unvalidated))
            .unwrap()
            .contains("older history"));
        assert_eq!(diary_note(&Err("x".into())), Some("x".into()));
    }
}
