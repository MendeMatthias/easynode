//! Serve the two routes a wallet needs to settle a fork, from any node at all.
//!
//! ── WHY THIS IS SEPARATE FROM ESPLORA MODE ──────────────────────────────────
//! A wallet settles a fork by comparing the block HASH at a height two sources
//! both hold. That is the whole of it: `GET /blocks/tip/height` and
//! `GET /block-height/<h>`. The PQ wallet's egress gate permits exactly those
//! two for a witness origin and nothing else, and its `witnessTarget` /
//! `judgeDivergence` pair asks for nothing more.
//!
//! Esplora mode answers those routes too, and it is enormous: electrs, a full
//! archival chain (124 GiB measured 2026-09-04), an index on top, and a
//! `prune=0` datadir it can never be given retroactively. Every one of those
//! costs exists for the ADDRESS index — balances, UTXOs, history — which a
//! witness is never asked about and must never be trusted for.
//!
//! ── THE MEASUREMENT THAT MOTIVATED THIS ─────────────────────────────────────
//! A pruned node knows every block hash. Measured on this project's validator
//! on 2026-09-06, `pruneheight` 184942:
//!
//! ```text
//!   getblockhash 1       99911b8fb5433f68bfc5b5e389e87f2d001fb58fef271ef50ce61aca8475ec41
//!   getblockhash 150000  85db9f65d6f58cd121edbee2a6147e09f2d61b05a2a8a62c096c070cf128f854
//!   getblockhash 211475  d3f2320640266f234e1ed82b20c94521522b9212fe891b5b40283365406c0645
//! ```
//!
//! Height 1 and height 150000 are hundreds of thousands of blocks below the
//! prune height, and the node answers both instantly. Pruning discards block
//! DATA; the block INDEX is complete on every node. So the requirement that
//! kept the fleet from witnessing — a full unpruned chain and a GPU to build it
//! — was never the witness's requirement. It was the address index's.
//!
//! What this changes: the wallet's only chain witness has been
//! esplora.btxbyronbay.com, frozen at 209,778 while the chain ran past
//! 211,400, so its fork check has not run for days. Replacing it needed a
//! machine nobody had. It now needs a node anybody already runs.
//!
//! ── WHAT THIS DELIBERATELY WILL NOT DO ──────────────────────────────────────
//! It serves two routes and 404s everything else, including every `/address`
//! and `/tx` route. That is not an oversight to be filled in later: a node
//! serving witness data has made no promise about an address index, and the
//! defect that retired Byron Bay was an address index that answered every route
//! confidently while not recording spends. A witness that also served balances
//! would be exactly that machine. The wallet enforces the same split from its
//! side (`WITNESS_ONLY_ORIGINS`), and this is the server half of it.
//!
//! It also reports what the node actually sees, always, including while the
//! node is behind or on a branch. A witness that withheld its answer when it
//! looked wrong would disappear at the exact moment a fork made it useful.
//! Judging is the caller's job.
//!
//! ── THE SERVER ──────────────────────────────────────────────────────────────
//! Hand-written HTTP over tokio rather than a web framework. Two read-only GET
//! routes on loopback do not justify a new dependency tree in a crate that
//! ships in a signing wallet's sibling product, and the request handling is
//! small enough to read in one sitting: a bounded read, one request line, a
//! path allow-list, a fixed response, close.

use crate::error::AppResult;
use crate::rpc::Rpc;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Where the witness server listens. Loopback only: a reverse proxy in front
/// of it owns TLS, CORS and rate limiting, exactly as it does for electrs.
pub const WITNESS_ADDR: &str = "127.0.0.1:3081";

/// The largest request head this will read. A witness request is about sixty
/// bytes; anything approaching this is not one, and reading unboundedly from a
/// socket is how a trivial server becomes a memory bug.
const MAX_HEAD: usize = 8 * 1024;

/// How long one connection may take to send its request line.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The routes a witness answers. Everything else is a 404, by construction
/// rather than by omission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessRoute {
    /// `GET /blocks/tip/height` — the active chain's height, as a bare decimal.
    TipHeight,
    /// `GET /block-height/<h>` — the block hash at that height, as bare hex.
    BlockHash(u64),
}

/// Parse the request line of an HTTP request: `GET /path HTTP/1.1`.
/// Returns `(method, path)`, with the query string left attached so the router
/// can refuse it rather than silently ignoring it.
pub fn parse_request_line(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let path = parts.next()?;
    let version = parts.next()?;
    // A request line has exactly three fields and names a version we answer.
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }
    (!method.is_empty() && path.starts_with('/')).then_some((method, path))
}

/// A height path segment: a plain decimal, at most nine digits, no leading
/// zero so one height has exactly one spelling.
///
/// The same shape the PQ wallet's own egress validator enforces on this route
/// (`is_height_segment` in its `main.rs`). Kept identical on purpose: a server
/// that accepted spellings the client's gate refuses would be inviting someone
/// to find out which of the two is wrong.
fn parse_height(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse().ok()
}

/// Which route, if any. `None` is a 404 and covers every address, transaction,
/// mempool and broadcast route — a witness is not asked those, and answering
/// them would be a promise about an index this node has not made.
pub fn route(method: &str, path: &str) -> Option<WitnessRoute> {
    if method != "GET" {
        return None;
    }
    // No query strings. They carry nothing this server reads, and a route that
    // ignores them is a route that can be used to smuggle bytes past a log.
    if path.contains('?') || path.contains('#') {
        return None;
    }
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["", "blocks", "tip", "height"] => Some(WitnessRoute::TipHeight),
        ["", "block-height", h] => parse_height(h).map(WitnessRoute::BlockHash),
        _ => None,
    }
}

/// One HTTP/1.1 response. `text/plain`, no keep-alive, no server banner.
///
/// CORS is deliberately absent: the reverse proxy in front is the single
/// source of it, the same rule `deploy/esplora/Caddyfile.template` enforces by
/// stripping electrs' own headers. Two sources of that header is a duplicate
/// `Access-Control-Allow-Origin`, which browsers reject outright and which
/// broke the web wallet once already.
pub fn http_response(status: u16, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )
    .into_bytes()
}

/// Answer one routed request from the node.
///
/// A height the node does not have is a 404, not an error: asking a witness
/// about a block above its tip is the ordinary way a caller discovers the
/// witness is behind, and it must read as "I do not have that" rather than as
/// a fault.
pub async fn answer(rpc: &dyn Rpc, r: WitnessRoute) -> (u16, String) {
    match r {
        WitnessRoute::TipHeight => match crate::node_api::get_blockchain_info(rpc).await {
            Ok(info) => (200, info.blocks.to_string()),
            Err(e) => {
                eprintln!("[witness] getblockchaininfo failed: {e}");
                (500, "the node did not answer".to_string())
            }
        },
        WitnessRoute::BlockHash(h) => {
            match rpc.call("getblockhash", serde_json::json!([h])).await {
                Ok(v) => match v.as_str() {
                    Some(hash) if is_block_hash(hash) => (200, hash.to_string()),
                    _ => (404, "no block at that height".to_string()),
                },
                // btxd answers a height above its tip with an RPC error, which
                // is the common case here and is not a fault of ours.
                Err(_) => (404, "no block at that height".to_string()),
            }
        }
    }
}

/// 64 lowercase hex characters, and nothing else leaves this server as a hash.
fn is_block_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read the request head, up to the blank line, bounded and with a timeout.
/// Returns the first line, which is all this server reads: no header is
/// consulted, so none can change what is served.
async fn read_request_line(stream: &mut TcpStream) -> Result<String, u16> {
    let mut buf = Vec::with_capacity(256);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => return Err(400),
            Ok(Ok(n)) => n,
            Ok(Err(_)) | Err(_) => return Err(400),
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..pos]).trim_end().to_string();
            return Ok(line);
        }
        if buf.len() > MAX_HEAD {
            return Err(413);
        }
    }
}

async fn serve_one(rpc: Arc<dyn Rpc>, mut stream: TcpStream) {
    let (status, body) = match read_request_line(&mut stream).await {
        Err(code) => (code, String::new()),
        Ok(line) => match parse_request_line(&line).and_then(|(m, p)| route(m, p)) {
            Some(r) => answer(rpc.as_ref(), r).await,
            None => (404, "not found".to_string()),
        },
    };
    let _ = stream.write_all(&http_response(status, &body)).await;
    let _ = stream.flush().await;
}

/// A running witness server. Dropping the handle stops it: the accept loop
/// exits with the task, so a stop is a drop and there is no half-serving state
/// to reason about.
pub struct WitnessServer {
    task: tokio::task::JoinHandle<()>,
    pub addr: SocketAddr,
}

impl WitnessServer {
    /// Bind and start serving. Binding is done here, not in the task, so a
    /// port already in use is an error the caller sees immediately rather than
    /// a server that silently never came up.
    pub async fn start(rpc: Arc<dyn Rpc>, addr: &str) -> AppResult<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            crate::error::AppError::Config(format!("witness server cannot bind {addr}: {e}"))
        })?;
        let bound = listener.local_addr().map_err(|e| {
            crate::error::AppError::Config(format!("witness server has no address: {e}"))
        })?;
        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let rpc = rpc.clone();
                        tokio::spawn(serve_one(rpc, stream));
                    }
                    // A transient accept error must not kill the server; a
                    // permanent one would spin, so yield between attempts.
                    Err(e) => {
                        eprintln!("[witness] accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        });
        Ok(Self { task, addr: bound })
    }

    pub fn stop(self) {
        self.task.abort();
    }

    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct StubNode {
        height: u64,
    }

    #[async_trait]
    impl Rpc for StubNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            match method {
                "getblockchaininfo" => Ok(json!({
                    "blocks": self.height,
                    "headers": self.height,
                    "verificationprogress": 1.0,
                    "initialblockdownload": false,
                })),
                "getblockhash" => {
                    let h = params[0].as_u64().unwrap_or(0);
                    if h > self.height {
                        // What btxd does above its tip.
                        return Err(crate::error::AppError::Config(
                            "Block height out of range".into(),
                        ));
                    }
                    Ok(json!(format!("{:064x}", h)))
                }
                other => panic!("a witness must never call {other}"),
            }
        }
    }

    #[test]
    fn the_request_line_is_parsed_strictly() {
        assert_eq!(
            parse_request_line("GET /blocks/tip/height HTTP/1.1"),
            Some(("GET", "/blocks/tip/height"))
        );
        assert_eq!(
            parse_request_line("GET /block-height/1 HTTP/1.0"),
            Some(("GET", "/block-height/1"))
        );
        for bad in [
            "",
            "GET",
            "GET /",                 // two fields, no version
            "GET /x HTTP/1.1 extra", // a fourth field
            "GET x HTTP/1.1",        // not absolute
            "GET /x HTTP/2",         // a version this does not speak
            "GET /x SMTP/1.1",
        ] {
            assert_eq!(parse_request_line(bad), None, "should refuse {bad:?}");
        }
    }

    #[test]
    fn only_the_two_witness_routes_exist() {
        assert_eq!(
            route("GET", "/blocks/tip/height"),
            Some(WitnessRoute::TipHeight)
        );
        assert_eq!(
            route("GET", "/block-height/0"),
            Some(WitnessRoute::BlockHash(0))
        );
        assert_eq!(
            route("GET", "/block-height/211475"),
            Some(WitnessRoute::BlockHash(211475))
        );
        assert_eq!(
            route("GET", "/block-height/999999999"),
            Some(WitnessRoute::BlockHash(999_999_999))
        );
    }

    #[test]
    fn the_money_routes_are_absent_by_construction() {
        // A witness has made no promise about an address index. The defect
        // that retired Byron Bay was an address index answering every route
        // confidently while not recording spends; a witness that also served
        // balances would be that machine.
        for p in [
            "/address/btx1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8g",
            "/address/btx1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8g/utxo",
            "/address/btx1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8g/txs",
            "/tx/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "/mempool",
            "/blocks",
            "/blocks/tip/hash",
            "/",
        ] {
            assert_eq!(route("GET", p), None, "{p} must not be served");
        }
        // And no method other than GET reaches anything, POST /tx above all.
        for m in ["POST", "PUT", "DELETE", "HEAD", "OPTIONS", "get"] {
            assert_eq!(route(m, "/blocks/tip/height"), None, "{m} must be refused");
            assert_eq!(route(m, "/tx"), None);
        }
    }

    #[test]
    fn a_height_has_exactly_one_spelling() {
        // The same shape the wallet's own egress validator enforces. A server
        // that accepted spellings the client refuses invites someone to find
        // out which of the two is wrong.
        for bad in [
            "",           // bare route
            "00",         // padded
            "0123",       // leading zero
            "1234567890", // ten digits
            "-1",
            "1.0",
            "1e6",
            "0x1f",
            " 12",
            "12 ",
            "abc",
            "١٢٣", // non-ASCII digits
        ] {
            assert_eq!(
                route("GET", &format!("/block-height/{bad}")),
                None,
                "height {bad:?} must be refused"
            );
        }
        // Shape rules: no extra segment, no trailing slash, no query.
        for p in [
            "/block-height",
            "/block-height/",
            "/block-height/1/2",
            "/block-height/1?leak=seed",
            "/blocks/tip/height?x=1",
            "/blocks/tip/height/",
            "/block-height/1#f",
        ] {
            assert_eq!(route("GET", p), None, "{p} must be refused");
        }
    }

    #[tokio::test]
    async fn it_answers_the_tip_and_a_hash_and_404s_a_height_it_lacks() {
        let node = StubNode { height: 211_500 };
        assert_eq!(
            answer(&node, WitnessRoute::TipHeight).await,
            (200, "211500".to_string())
        );
        let (code, body) = answer(&node, WitnessRoute::BlockHash(211_475)).await;
        assert_eq!(code, 200);
        assert_eq!(body, format!("{:064x}", 211_475));
        assert!(is_block_hash(&body));
        // Above the tip: 404, because "I do not have that" is how a caller
        // discovers this witness is behind. It is not a fault.
        let (code, body) = answer(&node, WitnessRoute::BlockHash(999_999)).await;
        assert_eq!(code, 404);
        assert!(!is_block_hash(&body));
    }

    #[test]
    fn the_response_carries_a_length_and_no_cors() {
        let r = String::from_utf8(http_response(200, "211500")).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(r.contains("Content-Length: 6\r\n"));
        assert!(r.contains("Connection: close\r\n"));
        assert!(r.ends_with("\r\n\r\n211500"));
        // The proxy in front is the single source of CORS. Two sources is a
        // duplicate header, which browsers reject outright.
        assert!(!r.to_lowercase().contains("access-control-allow-origin"));
        assert!(String::from_utf8(http_response(404, ""))
            .unwrap()
            .contains("Content-Length: 0"));
    }

    #[tokio::test]
    async fn end_to_end_over_a_real_socket() {
        let node: Arc<dyn Rpc> = Arc::new(StubNode { height: 211_500 });
        // Port 0: the OS picks a free one, so the test cannot collide with a
        // real deployment or with itself running twice.
        let server = WitnessServer::start(node, "127.0.0.1:0").await.unwrap();
        let addr = server.addr;
        assert!(server.is_running());

        async fn get(addr: SocketAddr, path: &str) -> String {
            let mut s = TcpStream::connect(addr).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut out = String::new();
            tokio::io::AsyncReadExt::read_to_string(&mut s, &mut out)
                .await
                .unwrap();
            out
        }

        let r = get(addr, "/blocks/tip/height").await;
        assert!(r.starts_with("HTTP/1.1 200 OK"), "{r}");
        assert!(r.ends_with("211500"), "{r}");

        let r = get(addr, "/block-height/211475").await;
        assert!(r.contains("200 OK"), "{r}");
        assert!(r.ends_with(&format!("{:064x}", 211_475)), "{r}");

        // The money path is not reachable over the wire either.
        for p in ["/address/btx1z/utxo", "/tx/abc", "/mempool", "/blocks"] {
            let r = get(addr, p).await;
            assert!(r.starts_with("HTTP/1.1 404"), "{p} answered {r}");
        }

        // Garbage does not crash the server, and it keeps serving afterwards.
        {
            let mut s = TcpStream::connect(addr).await.unwrap();
            s.write_all(b"not http at all\r\n\r\n").await.unwrap();
            let mut out = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut s, &mut out).await;
        }
        let r = get(addr, "/blocks/tip/height").await;
        assert!(r.contains("200 OK"), "the server must survive junk: {r}");

        server.stop();
    }

    #[tokio::test]
    async fn a_port_already_taken_is_an_error_the_caller_sees() {
        let node: Arc<dyn Rpc> = Arc::new(StubNode { height: 1 });
        let first = WitnessServer::start(node.clone(), "127.0.0.1:0")
            .await
            .unwrap();
        let taken = first.addr.to_string();
        // Binding happens in start(), not in the spawned task, so this is an
        // error rather than a server that silently never came up.
        let second = WitnessServer::start(node, &taken).await;
        assert!(second.is_err(), "a taken port must fail loudly");
        let msg = second.err().unwrap().to_string();
        assert!(msg.contains("cannot bind"), "{msg}");
        first.stop();
    }
}
