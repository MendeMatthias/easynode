//! `btx-witness` — serve the routes a wallet needs to settle a fork, and the
//! five a swap client needs to check a transaction, from any BTX node.
//!
//! A wallet settles a fork by comparing the block HASH at a height two sources
//! both hold, which is `GET /blocks/tip/height` and `GET /block-height/<h>`.
//! Esplora mode answers those too, and needs electrs, a full 124 GiB archival
//! chain and a `prune=0` datadir to do it — all of which exist for the ADDRESS
//! index, which a witness is never asked about.
//!
//! A pruned node knows every block hash: pruning discards block data, not the
//! block index. So this runs anywhere a node runs, including the pruned ones
//! that could never serve Esplora.
//!
//! Since 2026-11-28 it also serves the five routes an HTLC swap client checks
//! across a quorum of independent operators: `/tx/<txid>/status`, `/hex`,
//! `/merkle-proof`, `/outspend/<vout>` and `/block/<hash>/header`, in electrs'
//! exact formats. A pruned node has no txindex, so the first four come from an
//! in-memory index of the blocks the node still holds (`--swap-window`, built
//! at start, followed through reorgs); outside it the answer is 404, never a
//! guess. See `btx_core::witness` and `docs/esplora-mode.md`.
//!
//!     btx-witness --datadir ~/.easybtx
//!     btx-witness --datadir /var/lib/btx --rpc 127.0.0.1:19334 --listen 127.0.0.1:3081
//!
//! It binds loopback by default and expects a reverse proxy in front for TLS,
//! CORS and rate limiting — `deploy/esplora/Caddyfile.template` is that proxy,
//! and it is the single source of CORS, so this emits none.
//!
//! It reads nothing but the node's `.cookie` and answers nothing but those
//! routes, plus `GET /signers/recent` since 2026-09-16: which keys signed the
//! last hundred blocks, for the census (see `btx_core::witness`). Every
//! `/address`, `/mempool` and broadcast route is a 404 by construction: a node
//! serving witness data has made no promise about an address index, and the
//! defect that retired the last independent witness was an address index that
//! answered every route confidently while not recording spends.

use btx_core::rpc::RpcClient;
use btx_core::swap_index::{DEFAULT_WINDOW, MIN_WINDOW};
use btx_core::witness::{WitnessOptions, WitnessServer, WITNESS_ADDR};
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = "\
btx-witness — serve the fork-witness and swap routes from a BTX node

USAGE:
    btx-witness --datadir <path> [--rpc <addr:port>] [--listen <addr:port>]
                [--swap-window <blocks> | --no-swap]

ROUTES:
    GET /blocks/tip/height            GET /tx/<txid>/status
    GET /block-height/<h>             GET /tx/<txid>/hex
    GET /signers/recent               GET /tx/<txid>/merkle-proof
                                      GET /tx/<txid>/outspend/<vout>
                                      GET /block/<hash>/header

OPTIONS:
    --datadir <path>       the node's data directory, holding its .cookie
    --rpc <addr:port>      the node's JSON-RPC (default 127.0.0.1:19334)
    --listen <addr:port>   where to serve (default 127.0.0.1:3081, loopback only)
    --swap-window <blocks> how many blocks below the tip the swap index covers
                           (default 100000, at least 144; a pruned node covers
                           what it still holds, if that is less)
    --no-swap              serve only the first three routes; no index is built
    -h, --help             this

A pruned node can serve all of these: pruning discards block data, not the
block index, and the swap index is built from the blocks the node still holds.
";

#[tokio::main]
async fn main() {
    let mut datadir: Option<PathBuf> = None;
    let mut rpc = "127.0.0.1:19334".to_string();
    let mut listen = WITNESS_ADDR.to_string();
    let mut opts = WitnessOptions::default();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--datadir" => datadir = args.next().map(PathBuf::from),
            "--rpc" => rpc = args.next().unwrap_or(rpc),
            "--listen" => listen = args.next().unwrap_or(listen),
            "--no-swap" => opts = WitnessOptions::without_swap(),
            "--swap-window" => {
                // One spelling, like every number this server reads: plain
                // decimal, in range. A typo must not silently become the
                // default.
                let v = args.next().unwrap_or_default();
                match v.parse::<u32>() {
                    Ok(n) if n >= MIN_WINDOW && !v.starts_with('0') => {
                        opts.swap_window = Some(n);
                    }
                    _ => {
                        eprintln!(
                            "--swap-window needs a number of blocks, at least {MIN_WINDOW} \
                             (default {DEFAULT_WINDOW}); got '{v}'"
                        );
                        std::process::exit(2);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            other => {
                eprintln!("unknown option: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let Some(datadir) = datadir else {
        eprintln!("--datadir is required\n\n{USAGE}");
        std::process::exit(2);
    };

    // Cookie auth, the same way electrs and the app authenticate: no password
    // is passed on a command line, where it would be visible to every process
    // on the machine.
    let cookie = datadir.join(".cookie");
    let client = match RpcClient::from_cookie(format!("http://{rpc}"), &cookie) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "cannot read {}: {e}\nIs the node running, and is this its datadir?",
                cookie.display()
            );
            std::process::exit(1);
        }
    };

    // Fail here rather than after binding: an operator should learn that the
    // node is unreachable from the first line of output, not from a witness
    // that serves 500s.
    match btx_core::node_api::get_blockchain_info(&client).await {
        Ok(info) => eprintln!(
            "[witness] node at height {} ({} headers){}",
            info.blocks,
            info.headers,
            if info.blocks < info.headers {
                ", still catching up"
            } else {
                ""
            }
        ),
        Err(e) => {
            eprintln!("the node did not answer getblockchaininfo: {e}");
            std::process::exit(1);
        }
    }

    let server = match WitnessServer::start_with(Arc::new(client), &listen, opts).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "[witness] serving /blocks/tip/height, /block-height/<h> and /signers/recent on {}",
        server.addr
    );
    match opts.swap_window {
        Some(w) => eprintln!(
            "[witness] and the five swap routes, from an index of up to {w} blocks below the tip \
             (503 until it is built)"
        ),
        None => eprintln!("[witness] swap routes off (--no-swap)"),
    }
    eprintln!("[witness] nothing else is served: this node makes no claim about an address index");

    // Serve until the supervisor stops us. A witness has no state to flush.
    std::future::pending::<()>().await;
}
