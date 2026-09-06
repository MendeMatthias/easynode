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

/// Where the witness server listens.
///
/// The systemd deployment puts Caddy in front, which owns TLS, CORS and rate
/// limiting exactly as it does for electrs. The app's own switch does NOT: it
/// binds this server directly, and on `0.0.0.0` there is no proxy in the path
/// at all. So the limits that protect the node have to live here — see
/// `MAX_INFLIGHT`, the connection deadline and the tip cache below — rather
/// than being assumed of whatever is in front.
pub const WITNESS_ADDR: &str = "127.0.0.1:3081";

/// Whether a bind address accepts connections from outside this machine.
/// The UI needs this to say what a setting actually does, rather than leaving
/// an operator to work out what 0.0.0.0 means.
pub fn is_public_bind(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // Every other spelling of loopback (127.0.0.0/8, ::ffff:127.0.0.1) is
    // reported public. That over-warns, which is the safe direction: this
    // decides whether the UI shows an exposure warning, and a missing warning
    // is worse than one too many. `validate_bind` refuses names outright, and
    // callers pass the address actually bound wherever they have it.
    !(host == "127.0.0.1" || host == "::1")
}

/// A bind address the witness server will accept: `host:port`, where host is
/// an IPv4 literal or a bracketed IPv6 literal.
///
/// Deliberately NOT a hostname, `localhost` included. A name goes through the
/// system resolver at bind time and takes the first address that works, so it
/// can land somewhere other than where it was read as meaning — and
/// `is_public_bind` would still be judging the string. A hosts file or a
/// resolver that answers a bare label from a search domain is enough to make
/// `localhost` a public bind that the UI calls private, which is the one
/// direction this must never be wrong in. 0.6.20 is the first release with
/// this setting, so nothing persisted can be a name. Returns the trimmed
/// value.
pub fn validate_bind(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("the address is empty".into());
    }
    if t.chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        return Err("an address is host:port, with no path".into());
    }
    let (host, port) = t
        .rsplit_once(':')
        .ok_or_else(|| "an address needs a port, like 127.0.0.1:3081".to_string())?;
    match port.parse::<u16>() {
        Ok(0) | Err(_) => return Err(format!("'{port}' is not a port")),
        Ok(p) if p < 1024 => {
            return Err(format!(
                "port {p} needs privileges this app does not have; pick one above 1023"
            ))
        }
        Ok(_) => {}
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let ok =
        bare.parse::<std::net::Ipv4Addr>().is_ok() || bare.parse::<std::net::Ipv6Addr>().is_ok();
    if !ok {
        return Err(format!(
            "'{bare}' must be an address like 127.0.0.1 or 0.0.0.0, not a name"
        ));
    }
    Ok(t.to_string())
}

/// The largest request head this will read. A witness request is about sixty
/// bytes; anything approaching this is not one, and reading unboundedly from a
/// socket is how a trivial server becomes a memory bug.
const MAX_HEAD: usize = 8 * 1024;

/// How long one connection may take, from accept to its request line being
/// read. This is a deadline for the whole read, not a per-read idle timeout: a
/// client that sends one byte every nine seconds must be cut off, not renewed.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How many requests may be in flight at once.
///
/// This bound protects the NODE, not this server. Each request in flight is at
/// most one JSON-RPC call, and btxd's defaults are `rpcthreads=4` with
/// `rpcworkqueue=16`; past that it answers 503, which the app's status poll
/// counts as a failure, and twenty consecutive failures make the app conclude
/// the node is wedged and reap it. An unbounded accept loop therefore hands
/// anyone who can reach this port a way to kill the node it serves from, and
/// have the app blame the node for it. Eight leaves btxd's queue mostly to the
/// app's own polling, and eight concurrent block-hash lookups is still
/// thousands of answers a second.
const MAX_INFLIGHT: usize = 8;

/// How long a tip height is reused instead of asked for again.
///
/// `/blocks/tip/height` is the route a caller polls, and its answer cannot
/// change faster than a block. This is a cache for the node's benefit, not the
/// caller's: it bounds what a flood of tip requests can push onto btxd.
const TIP_TTL: Duration = Duration::from_secs(1);

/// How long, and how much, is drained off a socket before it is closed.
const DRAIN_GRACE: Duration = Duration::from_millis(100);
const MAX_DRAIN: usize = 64 * 1024;

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
/// `deadline` covers the whole read. Timing each read separately would let a
/// client hold a connection open forever by trickling a byte before every
/// expiry, which costs the attacker nothing and costs the node a task, a
/// socket and one of MAX_INFLIGHT slots.
async fn read_request_line(
    stream: &mut TcpStream,
    deadline: tokio::time::Instant,
) -> Result<String, u16> {
    let mut buf = Vec::with_capacity(256);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await {
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

/// Take whatever the client sent after the request line off the socket, then
/// close the write side.
///
/// Nothing reads those bytes — no header can change what is served — but they
/// cannot be left unread. Closing a socket that still holds received data
/// sends RST rather than FIN, and an arriving RST discards what the peer has
/// buffered and not yet read: the client loses the response it was already
/// sent and sees a connection reset instead. Any browser request carries
/// enough header bytes past the first read to trigger it, and the failure
/// looks exactly like "this witness is down" — the one thing a fork check must
/// not report wrongly. Bounded by its own short grace and by MAX_DRAIN, so
/// draining can never be how a connection is held open.
async fn finish(stream: &mut TcpStream) {
    let until = tokio::time::Instant::now() + DRAIN_GRACE;
    let mut sink = [0u8; 1024];
    let mut seen = 0usize;
    while seen < MAX_DRAIN {
        match tokio::time::timeout_at(until, stream.read(&mut sink)).await {
            Ok(Ok(n)) if n > 0 => seen += n,
            _ => break,
        }
    }
    let _ = stream.shutdown().await;
}

/// A one-second memory of the tip height. Only successful answers are stored,
/// so a node that is failing is never remembered as one that answered.
#[derive(Default)]
struct TipCache(std::sync::Mutex<Option<(tokio::time::Instant, String)>>);

impl TipCache {
    fn get(&self) -> Option<String> {
        let guard = self.0.lock().ok()?;
        let (at, body) = guard.as_ref()?;
        (at.elapsed() < TIP_TTL).then(|| body.clone())
    }

    fn put(&self, body: &str) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some((tokio::time::Instant::now(), body.to_string()));
        }
    }
}

async fn serve_one(rpc: Arc<dyn Rpc>, tip: Arc<TipCache>, mut stream: TcpStream) {
    let deadline = tokio::time::Instant::now() + READ_TIMEOUT;
    let (status, body) = match read_request_line(&mut stream, deadline).await {
        Err(code) => (code, String::new()),
        Ok(line) => match parse_request_line(&line).and_then(|(m, p)| route(m, p)) {
            Some(WitnessRoute::TipHeight) => match tip.get() {
                Some(cached) => (200, cached),
                None => {
                    let answered = answer(rpc.as_ref(), WitnessRoute::TipHeight).await;
                    if answered.0 == 200 {
                        tip.put(&answered.1);
                    }
                    answered
                }
            },
            Some(r) => answer(rpc.as_ref(), r).await,
            None => (404, "not found".to_string()),
        },
    };
    let _ = stream.write_all(&http_response(status, &body)).await;
    let _ = stream.flush().await;
    finish(&mut stream).await;
}

/// A running witness server. `stop()` ends it, and so does dropping it.
///
/// The second half of that needs the `Drop` below to be true: dropping a
/// `JoinHandle` DETACHES the task, it does not cancel it. Without an explicit
/// abort, a server replaced in a slot goes on listening on its port, answering
/// from an RPC client belonging to a node that may no longer exist, with no
/// handle left to stop it.
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
            let inflight = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT));
            let tip = Arc::new(TipCache::default());
            loop {
                // The permit is taken BEFORE accept, deliberately. At the bound
                // the connections wait in the kernel's backlog and the callers
                // wait with them, which is backpressure; accepting first and
                // then queueing would move the flood inside the process, where
                // it becomes tasks, sockets and work for btxd.
                let Ok(permit) = inflight.clone().acquire_owned().await else {
                    return;
                };
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let rpc = rpc.clone();
                        let tip = tip.clone();
                        tokio::spawn(async move {
                            serve_one(rpc, tip, stream).await;
                            drop(permit);
                        });
                    }
                    // A transient accept error must not kill the server; a
                    // permanent one would spin, so yield between attempts.
                    Err(e) => {
                        eprintln!("[witness] accept failed: {e}");
                        drop(permit);
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

impl Drop for WitnessServer {
    fn drop(&mut self) {
        self.task.abort();
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

    /// A node that counts what it is asked and how much of it happens at once.
    struct CountingNode {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        live: Arc<std::sync::atomic::AtomicUsize>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
        delay: Duration,
    }

    impl CountingNode {
        fn new(delay: Duration) -> Self {
            Self {
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                live: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                delay,
            }
        }
    }

    #[async_trait]
    impl Rpc for CountingNode {
        async fn call(&self, method: &str, _params: Value) -> AppResult<Value> {
            use std::sync::atomic::Ordering::SeqCst;
            self.calls.fetch_add(1, SeqCst);
            let now = self.live.fetch_add(1, SeqCst) + 1;
            self.peak.fetch_max(now, SeqCst);
            tokio::time::sleep(self.delay).await;
            self.live.fetch_sub(1, SeqCst);
            match method {
                "getblockchaininfo" => Ok(json!({
                    "blocks": 211_500,
                    "headers": 211_500,
                    "verificationprogress": 1.0,
                    "initialblockdownload": false,
                })),
                "getblockhash" => Ok(json!(format!("{:064x}", 7))),
                other => panic!("a witness must never call {other}"),
            }
        }
    }

    async fn ask(addr: SocketAddr, request: &str) -> String {
        let mut c = TcpStream::connect(addr).await.unwrap();
        c.write_all(request.as_bytes()).await.unwrap();
        let mut got = String::new();
        c.read_to_string(&mut got).await.unwrap();
        got
    }

    /// The finding this fixes: an unbounded accept loop lets anyone who can
    /// reach the port drive btxd past `rpcworkqueue`, whereupon the app's own
    /// poll starts failing and, twenty failures later, reaps the node. A
    /// witness must never be the reason its node dies.
    #[tokio::test]
    async fn a_burst_is_answered_in_full_but_never_hits_the_node_all_at_once() {
        let node = Arc::new(CountingNode::new(Duration::from_millis(30)));
        let peak = node.peak.clone();
        let server = WitnessServer::start(node, "127.0.0.1:0").await.unwrap();
        let addr = server.addr;

        let mut asks = Vec::new();
        for _ in 0..40 {
            asks.push(tokio::spawn(async move {
                ask(addr, "GET /block-height/7 HTTP/1.1\r\n\r\n").await
            }));
        }
        let mut answered = 0;
        for a in asks {
            let got = a.await.unwrap();
            assert!(got.starts_with("HTTP/1.1 200"), "burst answer was: {got:?}");
            answered += 1;
        }
        assert_eq!(answered, 40, "every request must still be answered");
        let seen = peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            seen <= MAX_INFLIGHT,
            "{seen} concurrent RPCs reached the node, bound is {MAX_INFLIGHT}"
        );
        server.stop();
    }

    /// The finding this fixes: with the timeout inside the read loop, one byte
    /// every nine seconds renewed it forever. The deadline is for the whole
    /// read, so a trickle is cut off.
    #[tokio::test]
    async fn a_trickling_client_is_cut_off_by_the_whole_connection_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut c = TcpStream::connect(addr).await.unwrap();
            // Never a newline, and never idle long enough to trip a per-read
            // timeout: exactly the shape that used to hold a socket for hours.
            for _ in 0..40 {
                if c.write_all(b"G").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let started = tokio::time::Instant::now();
        assert_eq!(read_request_line(&mut stream, deadline).await, Err(400));
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "the deadline did not hold: {:?}",
            started.elapsed()
        );
        client.abort();
    }

    /// The finding this fixes: replying and closing while header bytes sit
    /// unread sends RST, and an RST can destroy the response the client was
    /// already sent. Every request here carries a head bigger than one read.
    #[tokio::test]
    async fn a_request_with_a_long_head_still_gets_its_whole_response() {
        let node = Arc::new(StubNode { height: 211_500 });
        let server = WitnessServer::start(node, "127.0.0.1:0").await.unwrap();
        let padding = "X-Pad: ".to_string() + &"p".repeat(4000) + "\r\n";
        let request = format!(
            "GET /blocks/tip/height HTTP/1.1\r\nHost: x\r\n{padding}Cookie: {}\r\n\r\n",
            "c".repeat(2000)
        );
        for _ in 0..5 {
            let got = ask(server.addr, &request).await;
            assert!(got.starts_with("HTTP/1.1 200"), "answer was: {got:?}");
            assert!(got.ends_with("211500"), "body was lost: {got:?}");
        }
        server.stop();
    }

    /// The tip height is the polled route, so it is answered from a one-second
    /// memory rather than from the node every time.
    #[tokio::test]
    async fn the_tip_height_is_reused_for_a_second_instead_of_asked_again() {
        let node = Arc::new(CountingNode::new(Duration::ZERO));
        let calls = node.calls.clone();
        let server = WitnessServer::start(node, "127.0.0.1:0").await.unwrap();
        for _ in 0..20 {
            let got = ask(server.addr, "GET /blocks/tip/height HTTP/1.1\r\n\r\n").await;
            assert!(got.ends_with("211500"), "answer was: {got:?}");
        }
        let n = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            n < 20,
            "20 tip requests made {n} calls; the cache did nothing"
        );
        // Block hashes are never cached: a reorg changes them.
        let before = calls.load(std::sync::atomic::Ordering::SeqCst);
        for _ in 0..3 {
            ask(server.addr, "GET /block-height/7 HTTP/1.1\r\n\r\n").await;
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst) - before,
            3,
            "block hashes must be asked for every time"
        );
        server.stop();
    }

    /// The finding this fixes: `WitnessServer`'s doc said dropping it stops it,
    /// and dropping a JoinHandle detaches rather than cancels. A server left in
    /// a slot that is overwritten used to keep the port open with no handle
    /// left to close it.
    #[tokio::test]
    async fn dropping_the_server_really_does_free_the_port() {
        let node = Arc::new(StubNode { height: 1 });
        let server = WitnessServer::start(node.clone(), "127.0.0.1:0")
            .await
            .unwrap();
        let addr = server.addr;
        drop(server);
        // The abort has to be observed by the runtime before the port is free.
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Ok(again) = WitnessServer::start(node.clone(), &addr.to_string()).await {
                again.stop();
                return;
            }
        }
        panic!("{addr} was still held after the server was dropped");
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

    #[test]
    fn a_bind_address_is_an_address_and_a_usable_port() {
        for good in [
            "127.0.0.1:3081",
            "0.0.0.0:3081",
            "[::1]:3081",
            "  127.0.0.1:3081  ",
        ] {
            assert!(validate_bind(good).is_ok(), "{good} should be accepted");
        }
        assert_eq!(
            validate_bind("  127.0.0.1:3081  ").unwrap(),
            "127.0.0.1:3081"
        );
        for bad in [
            "",
            "127.0.0.1", // no port
            "127.0.0.1:0",
            "127.0.0.1:99999",
            "127.0.0.1:http",
            // A name would be resolved at start and could silently bind
            // somewhere else later.
            "esplora-1.easybtx.com:3081",
            "example.com:3081",
            "127.0.0.1:3081/x",
            "127.0.0.1 :3081",
        ] {
            assert!(validate_bind(bad).is_err(), "{bad:?} should be refused");
        }
        // Privileged ports: this app is not running as root and should say so
        // rather than failing at bind time with EACCES.
        let e = validate_bind("0.0.0.0:443").unwrap_err();
        assert!(e.contains("above 1023"), "{e}");
    }

    /// `localhost` is a name, and a name is resolved at bind time. Accepting it
    /// let a resolver decide where this server listened while `is_public_bind`
    /// went on judging the eight letters it was given — a public bind the UI
    /// would call private.
    #[test]
    fn a_name_is_refused_even_when_the_name_is_localhost() {
        for name in ["localhost:8080", "localhost:3081", "[localhost]:3081"] {
            let err = validate_bind(name).unwrap_err();
            assert!(err.contains("not a name"), "{name} said: {err}");
        }
    }

    #[test]
    fn a_public_bind_is_recognised_as_public() {
        for local in ["127.0.0.1:3081", "[::1]:3081"] {
            assert!(!is_public_bind(local), "{local} is loopback");
        }
        for public in ["0.0.0.0:3081", "192.168.1.50:3081", "[::]:3081"] {
            assert!(is_public_bind(public), "{public} accepts from outside");
        }
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
