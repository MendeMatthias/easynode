//! `btx-witness` — serve the two routes a wallet needs to settle a fork.
//!
//! A wallet settles a fork by comparing the block HASH at a height two sources
//! both hold, which is `GET /blocks/tip/height` and `GET /block-height/<h>`
//! and nothing else. Esplora mode answers those too, and needs electrs, a full
//! 124 GiB archival chain and a `prune=0` datadir to do it — all of which
//! exist for the ADDRESS index, which a witness is never asked about.
//!
//! A pruned node knows every block hash: pruning discards block data, not the
//! block index. So this runs anywhere a node runs, including the pruned ones
//! that could never serve Esplora.
//!
//!     btx-witness --datadir ~/.easybtx
//!     btx-witness --datadir /var/lib/btx --rpc 127.0.0.1:19334 --listen 127.0.0.1:3081
//!
//! It binds loopback by default and expects a reverse proxy in front for TLS,
//! CORS and rate limiting — `deploy/esplora/Caddyfile.template` is that proxy,
//! and it is the single source of CORS, so this emits none.
//!
//! It reads nothing but the node's `.cookie` and answers nothing but those two
//! routes. Every `/address`, `/tx`, `/mempool` and broadcast route is a 404 by
//! construction: a node serving witness data has made no promise about an
//! address index, and the defect that retired the last independent witness was
//! an address index that answered every route confidently while not recording
//! spends.

use btx_core::rpc::RpcClient;
use btx_core::witness::{WitnessServer, WITNESS_ADDR};
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = "\
btx-witness — serve /blocks/tip/height and /block-height/<h> from a BTX node

USAGE:
    btx-witness --datadir <path> [--rpc <addr:port>] [--listen <addr:port>]

OPTIONS:
    --datadir <path>     the node's data directory, holding its .cookie
    --rpc <addr:port>    the node's JSON-RPC (default 127.0.0.1:19334)
    --listen <addr:port> where to serve (default 127.0.0.1:3081, loopback only)
    -h, --help           this

A pruned node can serve this: pruning discards block data, not the block index.
";

#[tokio::main]
async fn main() {
    let mut datadir: Option<PathBuf> = None;
    let mut rpc = "127.0.0.1:19334".to_string();
    let mut listen = WITNESS_ADDR.to_string();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--datadir" => datadir = args.next().map(PathBuf::from),
            "--rpc" => rpc = args.next().unwrap_or(rpc),
            "--listen" => listen = args.next().unwrap_or(listen),
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

    let server = match WitnessServer::start(Arc::new(client), &listen).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "[witness] serving /blocks/tip/height and /block-height/<h> on {}",
        server.addr
    );
    eprintln!("[witness] nothing else is served: this node makes no claim about an address index");

    // Serve until the supervisor stops us. A witness has no state to flush.
    std::future::pending::<()>().await;
}
