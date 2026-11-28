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
//! It serves the routes listed here and 404s everything else, including every
//! `/address` route, `GET /tx/<txid>`, the mempool and every broadcast. That
//! is not an oversight to be filled in later: a node serving witness data has
//! made no promise about an address index, and the defect that retired Byron
//! Bay was an address index that answered every route confidently while not
//! recording spends. A witness that also served balances would be exactly that
//! machine. The wallet enforces the same split from its side
//! (`WITNESS_ONLY_ORIGINS`), and this is the server half of it.
//!
//! It also reports what the node actually sees, always, including while the
//! node is behind or on a branch. A witness that withheld its answer when it
//! looked wrong would disappear at the exact moment a fork made it useful.
//! Judging is the caller's job.
//!
//! ── THE ONE ADDITION SINCE, AND WHY IT IS THE SAME KIND OF THING ────────────
//! `GET /signers/recent` (2026-09-16): which keys signed the last hundred
//! blocks, as JSON, read from the node's own stored attestations
//! (`btx_core::signer::RecentSigners`). It exists because the census on
//! easybtx.com has no other way to count live signers: the checker on Vercel
//! speaks P2P to probe nodes and has no RPC anywhere, and the number it needs
//! to publish, honestly, was ONE on the day btxscan froze for want of that one.
//! It is a chain fact from the block index and the attestation store, exactly
//! the class the two hash routes are; it is not an address, a balance, a
//! transaction or a mempool, and those stay 404. The PQ wallet's egress gate
//! does not list this route and is unaffected by its existence.
//!
//! ── THE FIVE SWAP ROUTES (2026-11-28) ───────────────────────────────────────
//! An HTLC atomic swap client checks one transaction across a quorum of
//! independent Esplora operators, with five GETs:
//!
//! ```text
//!   /tx/<txid>/status          where it is confirmed            JSON
//!   /tx/<txid>/hex             its bytes                        text
//!   /tx/<txid>/merkle-proof    how it hashes into its block     JSON
//!   /tx/<txid>/outspend/<vout> what spent this output, if any   JSON
//!   /block/<hash>/header       the block's header               text
//! ```
//!
//! They are served here, byte for byte in electrs' formats (the reference is
//! `btx-esplora/electrs/src/rest.rs` and the live `api.btxscan.io`), so any
//! easyNode node, pruned ones included, can be one more independent operator
//! without the 124 GiB archive. They are the same kind of thing as the hash
//! routes, which is why they belong here and the address routes do not: every
//! answer is one the client checks itself. The hex hashes to the txid, the
//! proof folds into the header's merkle root, the header hashes to a block hash
//! other operators confirm, and "spent" names a spending transaction the client
//! fetches and verifies. "Unspent" is never proof of anything, and this server
//! says it only when the answer is complete up to the node's tip (see
//! `answer_outspend`).
//!
//! A pruned node without `-txindex` cannot find a txid, so the first four are
//! answered from an in-memory index of the blocks the node still holds
//! (`crate::swap_index`). Outside that window the answer is 404, "this witness
//! does not know", and never a guess: a 404 is an abstention a quorum client
//! can count, a confident wrong answer is not. While the index is being built,
//! or is catching up with a reorg, the answer is 503.
//!
//! ── THE SERVER ──────────────────────────────────────────────────────────────
//! Hand-written HTTP over tokio rather than a web framework. Two read-only GET
//! routes on loopback do not justify a new dependency tree in a crate that
//! ships in a signing wallet's sibling product, and the request handling is
//! small enough to read in one sitting: a bounded read, one request line, a
//! path allow-list, a fixed response, close.

use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use crate::swap_index::{id_hex, parse_id, BlockRef, Id, IndexState, SwapIndex};
use serde_json::{json, Value};
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

/// How long the signer window is reused before the node is asked what changed.
///
/// Refreshing is incremental (two RPCs per new block, see
/// `signer::refresh_recent_signers`), so this bounds the poll rate rather than
/// the cost; a cold window costs two hundred calls once. Half a block time is
/// plenty for a census that runs every thirty minutes.
const SIGNERS_TTL: Duration = Duration::from_secs(30);

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
    /// `GET /signers/recent` — which keys signed the newest blocks, as JSON
    /// (`signer::RecentSignersSummary`). Served from a window the server keeps
    /// current, never computed from scratch per request.
    RecentSigners,
    /// `GET /tx/<txid>/status` — electrs' `TransactionStatus` JSON.
    TxStatus(Id),
    /// `GET /tx/<txid>/hex` — the transaction's bytes, lowercase hex.
    TxHex(Id),
    /// `GET /tx/<txid>/merkle-proof` — `{block_height, merkle, pos}`.
    TxMerkleProof(Id),
    /// `GET /tx/<txid>/outspend/<vout>` — electrs' `SpendingValue` JSON.
    TxOutspend(Id, u32),
    /// `GET /block/<hash>/header` — the serialized header, lowercase hex.
    BlockHeader(Id),
}

impl WitnessRoute {
    /// The five swap routes, which exist only when the server was started
    /// with a swap index.
    pub fn is_swap(&self) -> bool {
        matches!(
            self,
            WitnessRoute::TxStatus(_)
                | WitnessRoute::TxHex(_)
                | WitnessRoute::TxMerkleProof(_)
                | WitnessRoute::TxOutspend(..)
                | WitnessRoute::BlockHeader(_)
        )
    }
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

/// The largest output index the outspend route accepts.
///
/// A small cap, like the height's, so the segment is a number and not a free
/// slot a compromised client could smuggle bytes through. Four digits covers
/// every output a swap names (an HTLC is one output of a small transaction)
/// and the largest transactions on the chain today (a pool payout with 74
/// outputs); a vout above it is answered 404, an abstention, never a guess.
pub const MAX_VOUT: u32 = 9_999;

/// An output index path segment: plain decimal, one to four digits, no
/// leading zero, so one vout has exactly one spelling.
fn parse_vout(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse().ok().filter(|v| *v <= MAX_VOUT)
}

/// Which route, if any. `None` is a 404 and covers every address, mempool and
/// broadcast route and `GET /tx/<txid>` itself — a witness is not asked those,
/// and answering them would be a promise about an index this node has not
/// made. A txid or block hash is exactly 64 lowercase hex characters: electrs
/// also accepts uppercase, and this does not, so one id has one spelling here
/// as it does in the wallet's egress gate.
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
        ["", "signers", "recent"] => Some(WitnessRoute::RecentSigners),
        ["", "tx", t, "status"] => parse_id(t).map(WitnessRoute::TxStatus),
        ["", "tx", t, "hex"] => parse_id(t).map(WitnessRoute::TxHex),
        ["", "tx", t, "merkle-proof"] => parse_id(t).map(WitnessRoute::TxMerkleProof),
        ["", "tx", t, "outspend", v] => {
            Some(WitnessRoute::TxOutspend(parse_id(t)?, parse_vout(v)?))
        }
        ["", "block", h, "header"] => parse_id(h).map(WitnessRoute::BlockHeader),
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
    http_response_typed(status, "text/plain; charset=utf-8", body)
}

/// The same response with a stated content type; the signer window is JSON.
pub fn http_response_typed(status: u16, content_type: &str, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
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
        // Answered from the server's window, not from here: a per-request
        // computation would be two hundred RPCs per caller, which is the
        // flood this module's whole bound exists to prevent.
        WitnessRoute::RecentSigners => {
            (500, "the signer window is served by the server".to_string())
        }
        // The swap routes need the server's index; see `answer_swap`.
        _ => (500, "the swap routes are served by the server".to_string()),
    }
}

/// One answer: status, content type and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

/// electrs' content types, exactly: `application/json` for the JSON routes,
/// bare `text/plain` (no charset) for hex, headers and its error messages.
pub const JSON: &str = "application/json";
pub const TEXT: &str = "text/plain";

fn reply(status: u16, content_type: &'static str, body: impl Into<String>) -> Reply {
    Reply {
        status,
        content_type,
        body: body.into(),
    }
}

/// The bodies electrs answers a miss with, word for word, so a client that
/// matches on them reads this witness the same way.
pub const TX_NOT_FOUND: &str = "Transaction not found";
pub const TX_NOT_CONFIRMED: &str = "Transaction not found or is unconfirmed";
pub const BLOCK_NOT_FOUND: &str = "Block not found";
/// An outpoint this witness cannot place. electrs never says this (it answers
/// `{"spent":false}` for any outpoint it has not seen spent, existing or not);
/// a pruned witness says it whenever the output was created below its window
/// and has already left the UTXO set, because "unspent" there would be a guess.
pub const OUTPOINT_UNKNOWN: &str = "Output not known to this witness";

fn not_ready() -> Reply {
    reply(503, TEXT, "the swap index is still being built")
}

fn catching_up(index: &SwapIndex) -> Reply {
    index.wake.notify_one();
    reply(503, TEXT, "the swap index is catching up with the node")
}

fn node_failed(what: &str, e: &AppError) -> Reply {
    eprintln!("[witness] {what} failed: {e}");
    reply(500, TEXT, "the node did not answer")
}

/// electrs' `TransactionStatus`, field for field and in its order.
pub fn status_json(b: &BlockRef) -> String {
    format!(
        r#"{{"confirmed":true,"block_height":{},"block_hash":"{}","block_time":{}}}"#,
        b.height,
        id_hex(&b.hash),
        b.time
    )
}

/// electrs' `SpendingValue` for a spent output. `block` is `None` for a spend
/// still in the mempool, which electrs reports as `{"confirmed":false}`.
pub fn spent_json(txid: &Id, vin: u32, block: Option<&BlockRef>) -> String {
    let status = match block {
        Some(b) => status_json(b),
        None => r#"{"confirmed":false}"#.to_string(),
    };
    format!(
        r#"{{"spent":true,"txid":"{}","vin":{vin},"status":{status}}}"#,
        id_hex(txid)
    )
}

/// electrs' merkle-proof object: `{block_height, merkle, pos}`, in that order.
pub fn proof_json(p: &crate::swap_index::Proof) -> String {
    let merkle: Vec<String> = p.branch.iter().map(id_hex).collect();
    json!({
        "block_height": p.block.height,
        "merkle": merkle,
        "pos": p.pos,
    })
    .to_string()
}

const MERKLE_MISMATCH: &str =
    "this block's transactions do not hash to its merkle root; no proof is served";

pub const UNSPENT_JSON: &str = r#"{"spent":false}"#;
pub const UNCONFIRMED_JSON: &str = r#"{"confirmed":false}"#;

/// Is this block still where the index says it is, on the node's active chain?
/// `Err` only when the node did not answer at all.
async fn still_active(rpc: &dyn Rpc, b: &BlockRef) -> Result<bool, AppError> {
    match rpc.call("getblockhash", json!([b.height])).await {
        Ok(v) => Ok(v.as_str() == Some(id_hex(&b.hash).as_str())),
        // A height above the node's tip: the block is not active any more.
        Err(AppError::Rpc { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// What a second look at the index found, after a miss.
enum Recheck<T> {
    /// Connected since the first look; the index is at the node's tip, so the
    /// block it names is on the active chain.
    Found(T),
    /// Not there, and the index is at the node's tip: a real miss.
    Missing,
    /// The index is not at the node's tip: no miss can be reported yet.
    Behind,
}

/// A miss is only reported as a miss when it is one. A txid in a block the
/// index has not read yet must read as "catching up", not as "not found"; and
/// one in a block connected between the first look and the tip check must be
/// found, so the look is repeated under the same lock as the tip comparison.
async fn recheck<T>(
    rpc: &dyn Rpc,
    index: &SwapIndex,
    look: impl Fn(&IndexState) -> Option<T>,
) -> Result<Recheck<T>, AppError> {
    let best = rpc.call("getbestblockhash", json!([])).await?;
    let best = best.as_str().and_then(parse_id);
    let s = index.state.read().await;
    if best.is_none() || s.tip().map(|t| t.1) != best {
        return Ok(Recheck::Behind);
    }
    Ok(match look(&s) {
        Some(x) => Recheck::Found(x),
        None => Recheck::Missing,
    })
}

/// A string of lowercase hex, even length, non-empty: the only shape a
/// transaction or header body may leave this server in.
fn is_lower_hex(s: &str) -> bool {
    !s.is_empty()
        && s.len().is_multiple_of(2)
        && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Answer one of the five swap routes. Any other route is a caller error.
pub async fn answer_swap(rpc: &dyn Rpc, index: &SwapIndex, r: WitnessRoute) -> Reply {
    match r {
        WitnessRoute::TxStatus(t) => answer_status(rpc, index, &t).await,
        WitnessRoute::TxHex(t) => answer_hex(rpc, index, &t).await,
        WitnessRoute::TxMerkleProof(t) => answer_merkle_proof(rpc, index, &t).await,
        WitnessRoute::TxOutspend(t, v) => answer_outspend(rpc, index, &t, v).await,
        WitnessRoute::BlockHeader(h) => answer_header(rpc, &h).await,
        _ => reply(500, TEXT, "not a swap route"),
    }
}

/// `/tx/<txid>/status`: confirmed (from the index, re-checked against the
/// node), unconfirmed (in the mempool), or 404.
///
/// electrs answers an unknown txid `200 {"confirmed":false}`. This answers
/// 404, as Byron Bay does: a pruned witness cannot tell "never existed" from
/// "confirmed below my window", and `{"confirmed":false}` would be a claim
/// about the second. Neither answer proves anything to a quorum client; both
/// read as "not confirmed".
pub async fn answer_status(rpc: &dyn Rpc, index: &SwapIndex, txid: &Id) -> Reply {
    let found = {
        let s = index.state.read().await;
        if !s.ready {
            return not_ready();
        }
        s.tx(txid)
    };
    if let Some(b) = found {
        return match still_active(rpc, &b).await {
            Ok(true) => reply(200, JSON, status_json(&b)),
            Ok(false) => catching_up(index),
            Err(e) => node_failed("getblockhash", &e),
        };
    }
    match rpc.call("getmempoolentry", json!([id_hex(txid)])).await {
        Ok(_) => return reply(200, JSON, UNCONFIRMED_JSON),
        Err(AppError::Rpc { .. }) => {}
        Err(e) => return node_failed("getmempoolentry", &e),
    }
    match recheck(rpc, index, |s| s.tx(txid)).await {
        Ok(Recheck::Found(b)) => reply(200, JSON, status_json(&b)),
        Ok(Recheck::Missing) => reply(404, TEXT, TX_NOT_FOUND),
        Ok(Recheck::Behind) => catching_up(index),
        Err(e) => node_failed("getbestblockhash", &e),
    }
}

/// `/tx/<txid>/hex`: the bytes, from the block the index places it in (works
/// pruned while that block is on disk), else from the mempool or a txindex.
pub async fn answer_hex(rpc: &dyn Rpc, index: &SwapIndex, txid: &Id) -> Reply {
    let found = {
        let s = index.state.read().await;
        if !s.ready {
            return not_ready();
        }
        s.tx(txid)
    };
    let t = id_hex(txid);
    if let Some(b) = found {
        match rpc
            .call("getrawtransaction", json!([t, false, id_hex(&b.hash)]))
            .await
        {
            Ok(v) => {
                if let Some(hex) = v.as_str().filter(|h| is_lower_hex(h)) {
                    return reply(200, TEXT, hex);
                }
                return reply(500, TEXT, "the node returned a malformed transaction");
            }
            // Pruned since it was indexed: fall through to the mempool/txindex.
            Err(AppError::Rpc { .. }) => {}
            Err(e) => return node_failed("getrawtransaction", &e),
        }
    }
    match rpc.call("getrawtransaction", json!([t, false])).await {
        Ok(v) => match v.as_str().filter(|h| is_lower_hex(h)) {
            Some(hex) => reply(200, TEXT, hex),
            None => reply(500, TEXT, "the node returned a malformed transaction"),
        },
        Err(AppError::Rpc { .. }) => reply(404, TEXT, TX_NOT_FOUND),
        Err(e) => node_failed("getrawtransaction", &e),
    }
}

/// `/tx/<txid>/merkle-proof`: the electrum-style branch, computed from the
/// block's txids, which the index has checked hash to the header's root.
pub async fn answer_merkle_proof(rpc: &dyn Rpc, index: &SwapIndex, txid: &Id) -> Reply {
    let found = {
        let s = index.state.read().await;
        if !s.ready {
            return not_ready();
        }
        s.proof(txid)
    };
    match found {
        Some(Ok(p)) => match still_active(rpc, &p.block).await {
            Ok(true) => reply(200, JSON, proof_json(&p)),
            Ok(false) => catching_up(index),
            Err(e) => node_failed("getblockhash", &e),
        },
        Some(Err(())) => reply(500, TEXT, MERKLE_MISMATCH),
        None => match recheck(rpc, index, |s| s.proof(txid)).await {
            Ok(Recheck::Found(Ok(p))) => reply(200, JSON, proof_json(&p)),
            Ok(Recheck::Found(Err(()))) => reply(500, TEXT, MERKLE_MISMATCH),
            Ok(Recheck::Missing) => reply(404, TEXT, TX_NOT_CONFIRMED),
            Ok(Recheck::Behind) => catching_up(index),
            Err(e) => node_failed("getbestblockhash", &e),
        },
    }
}

/// `/tx/<txid>/outspend/<vout>`.
///
/// "Spent" is said when the spend is seen: confirmed in the index (and its
/// block re-checked against the node) or waiting in the mempool. Either way
/// the client fetches the named spender and verifies it.
///
/// "Unspent" is said only when it is complete up to the node's tip, which is
/// one of two cases:
///
/// 1. the transaction that created the output is in the index's window or in
///    the mempool, the index is at the node's tip, and neither the index nor
///    the mempool holds a spend. The window is contiguous to the tip, so every
///    block that could hold a spend was read;
/// 2. the node's UTXO set holds the output (`gettxout` with the mempool).
///
/// Anything else is 404: an output created below the window that is no longer
/// in the UTXO set was spent somewhere this witness cannot see, and saying
/// either word about it would be a guess. The order of the checks matters: the
/// mempool is read BEFORE the index is checked against the node's tip, so a
/// spend that leaves the mempool for a block in between is still seen.
///
/// electrs answers `{"spent":false}` for an output index past the end of a
/// transaction it knows; so does this (case 1), byte for byte.
pub async fn answer_outspend(rpc: &dyn Rpc, index: &SwapIndex, txid: &Id, vout: u32) -> Reply {
    let confirmed = {
        let s = index.state.read().await;
        if !s.ready {
            return not_ready();
        }
        s.spend(txid, vout)
    };
    if let Some(sp) = confirmed {
        return match still_active(rpc, &sp.block).await {
            Ok(true) => reply(200, JSON, spent_json(&sp.txid, sp.vin, Some(&sp.block))),
            Ok(false) => catching_up(index),
            Err(e) => node_failed("getblockhash", &e),
        };
    }

    // The mempool, first (see above).
    let t = id_hex(txid);
    let q = json!([[{"txid": t, "vout": vout}]]);
    let spender = match rpc.call("gettxspendingprevout", q).await {
        Ok(v) => v
            .get(0)
            .and_then(|e| e.get("spendingtxid"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Err(e) => {
            // Without the mempool this cannot say "unspent", and a node
            // without the call cannot be asked another way.
            eprintln!("[witness] gettxspendingprevout failed: {e}");
            return reply(503, TEXT, "the mempool cannot be read");
        }
    };
    if let Some(sp) = spender {
        let Some(sp_id) = parse_id(&sp) else {
            return reply(500, TEXT, "the node named a malformed spender");
        };
        return match rpc.call("getrawtransaction", json!([sp, 1])).await {
            Ok(v) => {
                let vin = v.get("vin").and_then(Value::as_array).and_then(|ins| {
                    ins.iter().position(|i| {
                        i.get("txid").and_then(Value::as_str) == Some(t.as_str())
                            && i.get("vout").and_then(Value::as_u64) == Some(vout as u64)
                    })
                });
                match vin.and_then(|i| u32::try_from(i).ok()) {
                    Some(i) => reply(200, JSON, spent_json(&sp_id, i, None)),
                    // Mined or evicted between the two calls: ask again.
                    None => reply(503, TEXT, "the mempool changed; ask again"),
                }
            }
            Err(AppError::Rpc { .. }) => reply(503, TEXT, "the mempool changed; ask again"),
            Err(e) => node_failed("getrawtransaction", &e),
        };
    }

    // Now the index, which must be at the node's tip to say anything more.
    let best = match rpc.call("getbestblockhash", json!([])).await {
        Ok(v) => v.as_str().and_then(parse_id),
        Err(e) => return node_failed("getbestblockhash", &e),
    };
    let funding_in_window = {
        let s = index.state.read().await;
        if best.is_none() || s.tip().map(|x| x.1) != best {
            drop(s);
            return catching_up(index);
        }
        // A block connected since the first look may hold the spend.
        if let Some(sp) = s.spend(txid, vout) {
            return reply(200, JSON, spent_json(&sp.txid, sp.vin, Some(&sp.block)));
        }
        s.tx(txid).is_some()
    };
    let mut known_unspent = funding_in_window;
    if !known_unspent {
        known_unspent = match rpc.call("getmempoolentry", json!([t])).await {
            Ok(_) => true,
            Err(AppError::Rpc { .. }) => false,
            Err(e) => return node_failed("getmempoolentry", &e),
        };
    }
    if !known_unspent {
        known_unspent = match rpc.call("gettxout", json!([t, vout, true])).await {
            Ok(Value::Null) | Err(AppError::Rpc { .. }) => false,
            Ok(_) => true,
            Err(e) => return node_failed("gettxout", &e),
        };
    }
    if !known_unspent {
        return reply(404, TEXT, OUTPOINT_UNKNOWN);
    }
    // Complete up to the node's tip is only "unspent" if that tip is the
    // network's: a node still syncing, or one that stopped following the
    // chain, has not seen the blocks a spend may be in.
    match node_is_caught_up(rpc).await {
        Ok(true) => reply(200, JSON, UNSPENT_JSON),
        Ok(false) => reply(503, TEXT, NODE_BEHIND),
        Err(e) => node_failed("getblockchaininfo", &e),
    }
}

/// Said instead of "unspent" while the node is behind the network.
pub const NODE_BEHIND: &str = "this node is behind the network; it cannot say unspent";

/// Is the node at the network's tip, as far as it can tell? Every signal the
/// crate has for "behind", because a "no" here costs one 503 and a wrong "yes"
/// costs an "unspent" that is not:
///
/// - not in initial block download (which latches off once caught up, so
///   it is the weakest of these);
/// - not `tip_unsafe_to_act_on`: btxd's own `is_stale`, or its tip behind its
///   best header at all. It flaps by design while a block arrives; flapping
///   into a 503 is the safe direction;
/// - `network_height` (blocks, headers and the signed frontier when it is on
///   this chain) no more than two above `blocks`: after mainnet 244,000
///   `headers` alone stays near `blocks` while a node catches up;
/// - its tip not older than `node_api::TIP_STALE_AFTER_SECS`.
///
/// It still cannot see a branch the node holds only as headers it has not
/// accepted; nothing a node reports about itself can. That is why "unspent"
/// is never proof to a swap client.
async fn node_is_caught_up(rpc: &dyn Rpc) -> Result<bool, AppError> {
    let info = crate::node_api::get_blockchain_info(rpc).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(!info.initial_block_download
        && !info.tip_unsafe_to_act_on()
        && info.network_height() <= info.blocks + 2
        && !crate::node_api::tip_is_stale(info.median_time, now))
}

/// `/block/<hash>/header`: the header of a block on the node's active chain.
/// Needs no index, and works on any node, pruned or not. A block the node
/// knows only as a side branch is a 404, as electrs answers it: electrs
/// serves headers from its best chain only.
pub async fn answer_header(rpc: &dyn Rpc, hash: &Id) -> Reply {
    let h = id_hex(hash);
    match rpc.call("getblockheader", json!([h, true])).await {
        Ok(v) => {
            if v.get("confirmations").and_then(Value::as_i64).unwrap_or(-1) < 1 {
                return reply(404, TEXT, BLOCK_NOT_FOUND);
            }
        }
        Err(AppError::Rpc { .. }) => return reply(404, TEXT, BLOCK_NOT_FOUND),
        Err(e) => return node_failed("getblockheader", &e),
    }
    match rpc.call("getblockheader", json!([h, false])).await {
        Ok(v) => match v.as_str().filter(|x| is_lower_hex(x)) {
            Some(hex) => reply(200, TEXT, hex),
            None => reply(500, TEXT, "the node returned a malformed header"),
        },
        Err(AppError::Rpc { .. }) => reply(404, TEXT, BLOCK_NOT_FOUND),
        Err(e) => node_failed("getblockheader", &e),
    }
}

/// The signer window the server keeps: brought up to the tip at most once per
/// `SIGNERS_TTL`, otherwise reused. Only a successful tip read advances it.
#[derive(Default)]
pub struct SignersCache(
    tokio::sync::Mutex<(Option<tokio::time::Instant>, crate::signer::RecentSigners)>,
);

impl SignersCache {
    /// The window's summary, refreshed first if it is older than the TTL.
    /// Never fails: a node that does not answer leaves the last window in
    /// place, which is still true about the blocks it holds, and an empty
    /// window says `seen: 0` rather than inventing a count.
    pub async fn summary(&self, rpc: &dyn Rpc) -> crate::signer::RecentSignersSummary {
        let mut guard = self.0.lock().await;
        let fresh = guard.0.is_some_and(|at| at.elapsed() < SIGNERS_TTL);
        if !fresh {
            if let Ok(info) = crate::node_api::get_blockchain_info(rpc).await {
                // A whole window at once when cold, then two calls per block.
                let budget = crate::signer::SIGNED_WINDOW_BLOCKS as usize;
                crate::signer::refresh_recent_signers(rpc, &mut guard.1, info.blocks, budget).await;
                guard.0 = Some(tokio::time::Instant::now());
            }
        }
        guard.1.summary()
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

async fn serve_one(
    rpc: Arc<dyn Rpc>,
    tip: Arc<TipCache>,
    signers: Arc<SignersCache>,
    index: Option<Arc<SwapIndex>>,
    mut stream: TcpStream,
) {
    let deadline = tokio::time::Instant::now() + READ_TIMEOUT;
    let mut content_type = "text/plain; charset=utf-8";
    let (status, body) = match read_request_line(&mut stream, deadline).await {
        Err(code) => (code, String::new()),
        Ok(line) => match parse_request_line(&line).and_then(|(m, p)| route(m, p)) {
            Some(r) if r.is_swap() => match &index {
                Some(index) => {
                    let got = answer_swap(rpc.as_ref(), index, r).await;
                    content_type = got.content_type;
                    (got.status, got.body)
                }
                // Started without a swap index: these routes do not exist.
                None => (404, "not found".to_string()),
            },
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
            Some(WitnessRoute::RecentSigners) => {
                content_type = "application/json";
                let summary = signers.summary(rpc.as_ref()).await;
                match serde_json::to_string(&summary) {
                    Ok(json) => (200, json),
                    Err(_) => (500, "{}".to_string()),
                }
            }
            Some(r) => answer(rpc.as_ref(), r).await,
            None => (404, "not found".to_string()),
        },
    };
    let _ = stream
        .write_all(&http_response_typed(status, content_type, &body))
        .await;
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
    /// The task that keeps the swap index on the node's tip, when there is one.
    index_task: Option<tokio::task::JoinHandle<()>>,
    index: Option<Arc<SwapIndex>>,
    pub addr: SocketAddr,
}

/// How a witness server is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WitnessOptions {
    /// Serve the five swap routes from an index of this many blocks below the
    /// tip (capped by what a pruned node still holds). `None` leaves the
    /// routes out entirely: they answer 404 and the node is asked nothing
    /// extra.
    pub swap_window: Option<u32>,
}

impl Default for WitnessOptions {
    /// The swap routes on, over the default window: what `btx-witness` runs.
    fn default() -> Self {
        Self {
            swap_window: Some(crate::swap_index::DEFAULT_WINDOW),
        }
    }
}

impl WitnessOptions {
    /// What the app serves on `addr`: the swap routes only when the witness
    /// accepts connections from other machines. On loopback (the default
    /// install) no swap client can reach them, and the index would cost the
    /// owner's node a full read of the window on every start for nobody.
    pub fn for_bind(addr: &str) -> Self {
        if is_public_bind(addr) {
            Self::default()
        } else {
            Self::without_swap()
        }
    }

    /// The witness as it was before the swap routes: hash routes and signers.
    pub fn without_swap() -> Self {
        Self { swap_window: None }
    }
}

impl WitnessServer {
    /// Bind and start serving the hash routes and the signer census, WITHOUT
    /// the swap routes: exactly the witness every caller had before them.
    /// Binding is done here, not in the task, so a port already in use is an
    /// error the caller sees immediately rather than a server that silently
    /// never came up.
    ///
    /// The swap index is opt-in through `start_with` on purpose. The app's
    /// witness is on by default on every install, bound to loopback where no
    /// swap client can reach it; turning the index on there would make every
    /// app start read the last 100,000 blocks from its owner's node for
    /// nobody. `btx-witness`, which operators run behind a public front, turns
    /// it on; the app does when its witness is bound publicly.
    pub async fn start(rpc: Arc<dyn Rpc>, addr: &str) -> AppResult<Self> {
        Self::start_with(rpc, addr, WitnessOptions::without_swap()).await
    }

    /// Bind and start serving with explicit options.
    pub async fn start_with(
        rpc: Arc<dyn Rpc>,
        addr: &str,
        opts: WitnessOptions,
    ) -> AppResult<Self> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            crate::error::AppError::Config(format!("witness server cannot bind {addr}: {e}"))
        })?;
        let bound = listener.local_addr().map_err(|e| {
            crate::error::AppError::Config(format!("witness server has no address: {e}"))
        })?;
        // One bound for everything that reaches the node: requests take a
        // permit per connection, the index takes one per RPC.
        let inflight = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT));
        let index = opts.swap_window.map(|w| Arc::new(SwapIndex::new(w)));
        let index_task = index.clone().map(|index| {
            let rpc = rpc.clone();
            let gate = inflight.clone();
            tokio::spawn(async move { index.run(rpc.as_ref(), &gate).await })
        });
        let served_index = index.clone();
        let task = tokio::spawn(async move {
            let index = served_index;
            let tip = Arc::new(TipCache::default());
            let signers = Arc::new(SignersCache::default());
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
                        let signers = signers.clone();
                        let index = index.clone();
                        tokio::spawn(async move {
                            serve_one(rpc, tip, signers, index, stream).await;
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
        Ok(Self {
            task,
            index_task,
            index,
            addr: bound,
        })
    }

    pub fn stop(self) {
        // Drop does the aborting, for both tasks.
        drop(self);
    }

    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }

    /// The swap index's counts, and whether it has reached the tip; `None`
    /// when the server was started without one.
    pub async fn swap_stats(&self) -> Option<(bool, crate::swap_index::IndexStats)> {
        let index = self.index.as_ref()?;
        let s = index.state.read().await;
        Some((s.ready, s.stats()))
    }
}

impl Drop for WitnessServer {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(t) = &self.index_task {
            t.abort();
        }
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
                // Even heights carry one signature from a fixed key, odd
                // heights none, so a window's counts are predictable.
                "getmatmulattestations" => {
                    let hash = params[0].as_str().unwrap_or("");
                    let h = u64::from_str_radix(hash, 16).unwrap_or(1);
                    if h.is_multiple_of(2) {
                        Ok(json!([format!("00ff21{STUB_SIGNER}463044deadbeef")]))
                    } else {
                        Ok(json!([]))
                    }
                }
                other => panic!("a witness must never call {other}"),
            }
        }
    }

    const STUB_SIGNER: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";

    /// The signer census route: JSON, from the node's attestations, served
    /// from a window rather than recomputed, and never a promise about
    /// anything but who signed.
    #[tokio::test]
    async fn recent_signers_is_json_from_a_window_the_server_keeps() {
        assert_eq!(
            route("GET", "/signers/recent"),
            Some(WitnessRoute::RecentSigners)
        );
        assert_eq!(route("GET", "/signers/recent?x=1"), None);
        assert_eq!(route("POST", "/signers/recent"), None);
        assert_eq!(route("GET", "/signers"), None);

        let node = Arc::new(StubNode { height: 211_500 });
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
        let got = ask(server.addr, "GET /signers/recent HTTP/1.1\r\n\r\n").await;
        assert!(got.starts_with("HTTP/1.1 200"), "{got:?}");
        assert!(
            got.contains("Content-Type: application/json\r\n"),
            "{got:?}"
        );
        let body = got.split("\r\n\r\n").nth(1).unwrap();
        let v: Value = serde_json::from_str(body).unwrap();
        assert_eq!(v["window"], 100);
        assert_eq!(v["seen"], 100);
        assert_eq!(v["tip"], 211_500);
        assert_eq!(v["blocks_with_signature"], 50);
        assert_eq!(v["distinct_keys"], 1);
        assert_eq!(v["keys"][STUB_SIGNER], 50);
        server.stop();
    }

    /// A cold window is two hundred calls once; a warm one is reused for the
    /// TTL and then costs two calls per new block, never two hundred per
    /// request. This is what lets the route exist under MAX_INFLIGHT.
    #[tokio::test]
    async fn the_signer_window_is_reused_not_recomputed_per_request() {
        let node = Arc::new(CountingNode::new(Duration::ZERO));
        let calls = node.calls.clone();
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
        let first = ask(server.addr, "GET /signers/recent HTTP/1.1\r\n\r\n").await;
        assert!(first.starts_with("HTTP/1.1 200"), "{first:?}");
        let cold = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!((100..=201).contains(&cold), "cold fill made {cold} calls");
        for _ in 0..10 {
            ask(server.addr, "GET /signers/recent HTTP/1.1\r\n\r\n").await;
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            cold,
            "ten warm requests must cost the node nothing"
        );
        server.stop();
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
                "getmatmulattestations" => Ok(json!([])),
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
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
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
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
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
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
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
        let server = start_plain(node.clone(), "127.0.0.1:0").await.unwrap();
        let addr = server.addr;
        drop(server);
        // The abort has to be observed by the runtime before the port is free.
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Ok(again) = start_plain(node.clone(), &addr.to_string()).await {
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
        let server = start_plain(node, "127.0.0.1:0").await.unwrap();
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
        let first = start_plain(node.clone(), "127.0.0.1:0").await.unwrap();
        let taken = first.addr.to_string();
        // Binding happens in start(), not in the spawned task, so this is an
        // error rather than a server that silently never came up.
        let second = start_plain(node, &taken).await;
        assert!(second.is_err(), "a taken port must fail loudly");
        let msg = second.err().unwrap().to_string();
        assert!(msg.contains("cannot bind"), "{msg}");
        first.stop();
    }

    /// The witness as it was before the swap routes: the tests above count
    /// RPC calls and must not see an index's.
    async fn start_plain(node: Arc<dyn Rpc>, addr: &str) -> AppResult<WitnessServer> {
        WitnessServer::start_with(node, addr, WitnessOptions::without_swap()).await
    }

    // ── the five swap routes ────────────────────────────────────────────────

    const FIXTURE: &str = include_str!("../tests/fixtures/witness_swap/mainnet-288858.json");

    fn rpc_err(code: i64, message: &str) -> AppError {
        AppError::Rpc {
            code,
            message: message.into(),
        }
    }

    /// A node backed by real mainnet blocks, with an active chain, a mempool
    /// and a UTXO set the test can change under the server.
    #[derive(Default, Clone)]
    struct ChainState {
        blocks: std::collections::HashMap<String, Value>,
        headers: std::collections::HashMap<String, String>,
        raw: std::collections::HashMap<String, (String, String)>,
        active: Vec<String>,
        base: u64,
        mempool: std::collections::HashMap<String, Value>,
        utxos: std::collections::HashSet<(String, u64)>,
        pruned_below: u64,
        ibd: bool,
        behind: u64,
        calls: Vec<String>,
    }

    struct ChainNode(std::sync::Mutex<ChainState>);

    impl ChainNode {
        /// The fixture's blocks at `heights`, as one active chain.
        fn from_fixture(heights: &[u64]) -> Self {
            let f: Value = serde_json::from_str(FIXTURE).unwrap();
            let mut st = ChainState::default();
            for b in f["blocks"].as_array().unwrap() {
                let h = b["height"].as_u64().unwrap();
                if !heights.contains(&h) {
                    continue;
                }
                let hash = b["hash"].as_str().unwrap().to_string();
                let mut b = b.clone();
                b.as_object_mut().unwrap().remove("nextblockhash");
                st.blocks.insert(hash.clone(), b);
                st.active.push(hash);
            }
            for (k, v) in f["headers"].as_object().unwrap() {
                st.headers
                    .insert(k.clone(), v.as_str().unwrap().to_string());
            }
            for (k, v) in f["raw"].as_object().unwrap() {
                st.raw.insert(
                    k.clone(),
                    (
                        v["block"].as_str().unwrap().to_string(),
                        v["hex"].as_str().unwrap().to_string(),
                    ),
                );
            }
            st.base = heights[0];
            st.pruned_below = heights[0];
            Self(std::sync::Mutex::new(st))
        }

        fn with<R>(&self, f: impl FnOnce(&mut ChainState) -> R) -> R {
            f(&mut self.0.lock().unwrap())
        }
    }

    impl ChainState {
        fn height_of(&self, hash: &str) -> Option<u64> {
            self.blocks.get(hash).and_then(|b| b["height"].as_u64())
        }
        fn active_at(&self, h: u64) -> Option<&String> {
            self.active.get(h.checked_sub(self.base)? as usize)
        }
        fn is_active(&self, hash: &str) -> bool {
            self.height_of(hash)
                .is_some_and(|h| self.active_at(h).map(String::as_str) == Some(hash))
        }
        fn tip(&self) -> u64 {
            self.base + self.active.len() as u64 - 1
        }
    }

    #[async_trait]
    impl Rpc for ChainNode {
        async fn call(&self, method: &str, p: Value) -> AppResult<Value> {
            let mut st = self.0.lock().unwrap();
            st.calls.push(method.to_string());
            let s = |i: usize| p[i].as_str().unwrap_or("").to_string();
            match method {
                "getblockchaininfo" => Ok(json!({
                    "blocks": st.tip(), "headers": st.tip(),
                    "pruned": true, "pruneheight": st.pruned_below,
                    "initialblockdownload": st.ibd,
                    "behind_best_header": st.behind,
                })),
                "getbestblockhash" => Ok(json!(st.active.last().unwrap())),
                "getblockhash" => {
                    let h = p[0].as_u64().unwrap();
                    st.active_at(h)
                        .map(|x| json!(x))
                        .ok_or_else(|| rpc_err(-8, "Block height out of range"))
                }
                "getblock" => {
                    let hash = s(0);
                    let Some(mut b) = st.blocks.get(&hash).cloned() else {
                        return Err(rpc_err(-5, "Block not found"));
                    };
                    let h = b["height"].as_u64().unwrap();
                    if h < st.pruned_below {
                        return Err(rpc_err(-1, "Block not available (pruned data)"));
                    }
                    if st.is_active(&hash) {
                        if let Some(n) = st.active_at(h + 1) {
                            b["nextblockhash"] = json!(n);
                        }
                    }
                    Ok(b)
                }
                "getblockheader" => {
                    let hash = s(0);
                    let Some(h) = st.height_of(&hash) else {
                        return Err(rpc_err(-5, "Block not found"));
                    };
                    if p[1] == json!(false) {
                        return Ok(json!(st
                            .headers
                            .get(&hash)
                            .cloned()
                            .unwrap_or_else(|| "00".repeat(182))));
                    }
                    let active = st.is_active(&hash);
                    let mut v = json!({
                        "hash": hash, "height": h,
                        "confirmations": if active { (st.tip() - h + 1) as i64 } else { -1 },
                    });
                    if active {
                        if let Some(n) = st.active_at(h + 1) {
                            v["nextblockhash"] = json!(n);
                        }
                    }
                    Ok(v)
                }
                "getrawtransaction" => {
                    let t = s(0);
                    if let Some(blockhash) = p.get(2).and_then(Value::as_str) {
                        return match st.raw.get(&t) {
                            Some((b, hex)) if b == blockhash => Ok(json!(hex)),
                            _ => Err(rpc_err(
                                -5,
                                "No such transaction found in the provided block",
                            )),
                        };
                    }
                    match st.mempool.get(&t) {
                        Some(tx) if p[1] == json!(1) => Ok(tx.clone()),
                        Some(tx) => Ok(tx["hex"].clone()),
                        None => Err(rpc_err(-5, "No such mempool transaction")),
                    }
                }
                "getmempoolentry" => match st.mempool.contains_key(&s(0)) {
                    true => Ok(json!({"vsize": 1})),
                    false => Err(rpc_err(-5, "Transaction not in mempool")),
                },
                "gettxspendingprevout" => {
                    let q = &p[0][0];
                    let mut out = json!({"txid": q["txid"], "vout": q["vout"]});
                    for (txid, tx) in &st.mempool {
                        for i in tx["vin"].as_array().unwrap() {
                            if i["txid"] == q["txid"] && i["vout"] == q["vout"] {
                                out["spendingtxid"] = json!(txid);
                            }
                        }
                    }
                    Ok(json!([out]))
                }
                "gettxout" => {
                    let key = (s(0), p[1].as_u64().unwrap());
                    Ok(if st.utxos.contains(&key) {
                        json!({"value": 1.0})
                    } else {
                        Value::Null
                    })
                }
                other => panic!("a witness must never call {other}"),
            }
        }
    }

    /// A full HTTP exchange: (status, content type, body).
    async fn fetch(addr: SocketAddr, path: &str) -> (u16, String, String) {
        let raw = ask(addr, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n")).await;
        let (head, body) = raw.split_once("\r\n\r\n").expect("a head");
        let status = head[9..12].parse().unwrap();
        let ctype = head
            .lines()
            .find_map(|l| l.strip_prefix("Content-Type: "))
            .unwrap_or("")
            .to_string();
        (status, ctype, body.to_string())
    }

    async fn wait_ready(server: &WitnessServer, tip: u32) {
        for _ in 0..500 {
            if let Some((true, st)) = server.swap_stats().await {
                if st.tip == tip {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "the swap index never reached {tip}: {:?}",
            server.swap_stats().await
        );
    }

    async fn serve(node: Arc<ChainNode>) -> WitnessServer {
        WitnessServer::start_with(node, "127.0.0.1:0", WitnessOptions::default())
            .await
            .unwrap()
    }

    /// The acceptance test in miniature: every swap-route answer api.btxscan.io
    /// (electrs) gave for these mainnet blocks, served again by a witness on a
    /// pruned node that holds only those blocks, compared byte for byte —
    /// status, content type and body.
    #[tokio::test]
    async fn mainnet_fixture_answers_match_electrs_byte_for_byte() {
        let f: Value = serde_json::from_str(FIXTURE).unwrap();
        let chain = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let lone = Arc::new(ChainNode::from_fixture(&[289982]));
        let a = serve(chain.clone()).await;
        let b = serve(lone.clone()).await;
        wait_ready(&a, 288860).await;
        wait_ready(&b, 289982).await;
        let in_lone = |p: &str| {
            let blk = &f["blocks"].as_array().unwrap()[3];
            p.contains(blk["hash"].as_str().unwrap())
                || blk["tx"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| p.contains(t["txid"].as_str().unwrap()))
        };
        let mut compared = 0;
        for (path, want) in f["expect"].as_object().unwrap() {
            if path.starts_with("/block-height/") {
                continue;
            }
            let at = if in_lone(path) { b.addr } else { a.addr };
            let (code, ctype, body) = fetch(at, path).await;
            assert_eq!(code as u64, want["status"].as_u64().unwrap(), "{path}");
            assert_eq!(ctype, want["type"].as_str().unwrap(), "{path}");
            assert_eq!(body, want["body"].as_str().unwrap(), "{path}");
            compared += 1;
        }
        // 2 outspends, 1 hex, 4 headers, and status + proof for 23 txs.
        assert_eq!(compared, 53);
    }

    #[tokio::test]
    async fn outspend_says_unspent_only_when_it_is_complete_to_the_tip() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let s = serve(node.clone()).await;
        wait_ready(&s, 288860).await;
        let t0 = "37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5";
        // Created in the window and not spent in it: unspent, from the index
        // alone, whatever gettxout says. vout 7 does not exist; electrs says
        // the same thing about it.
        for v in [1, 7] {
            let r = fetch(s.addr, &format!("/tx/{t0}/outspend/{v}")).await;
            assert_eq!(r, (200, JSON.into(), UNSPENT_JSON.into()), "vout {v}");
        }
        // Created below the window: only the UTXO set can say unspent ...
        let below = "1111111111111111111111111111111111111111111111111111111111111111";
        node.with(|st| st.utxos.insert((below.into(), 3)));
        let r = fetch(s.addr, &format!("/tx/{below}/outspend/3")).await;
        assert_eq!(r, (200, JSON.into(), UNSPENT_JSON.into()));
        // ... and when it is not there either, this witness does not know.
        let r = fetch(s.addr, &format!("/tx/{below}/outspend/4")).await;
        assert_eq!(r, (404, TEXT.into(), OUTPOINT_UNKNOWN.into()));
        // A spend waiting in the mempool is a spend, with the input it uses.
        let spender = "2222222222222222222222222222222222222222222222222222222222222222";
        node.with(|st| {
            st.mempool.insert(
                spender.into(),
                json!({"hex": "00", "vin": [
                    {"txid": below, "vout": 9},
                    {"txid": t0, "vout": 1},
                ]}),
            )
        });
        let r = fetch(s.addr, &format!("/tx/{t0}/outspend/1")).await;
        assert_eq!(
            r.2,
            format!(
                r#"{{"spent":true,"txid":"{spender}","vin":1,"status":{{"confirmed":false}}}}"#
            )
        );
        // Funding still in the mempool, spend not: unspent.
        node.with(|st| {
            st.mempool
                .insert(below.into(), json!({"hex": "00", "vin": []}))
        });
        let r = fetch(s.addr, &format!("/tx/{below}/outspend/5")).await;
        assert_eq!(r, (200, JSON.into(), UNSPENT_JSON.into()));
        // A node still syncing has not seen the blocks a spend may be in:
        // every "unspent" becomes an abstention, and "spent" stays spent.
        node.with(|st| st.ibd = true);
        for p in [
            format!("/tx/{t0}/outspend/7"),
            format!("/tx/{below}/outspend/3"),
            format!("/tx/{below}/outspend/5"),
        ] {
            assert_eq!(
                fetch(s.addr, &p).await,
                (503, TEXT.into(), NODE_BEHIND.into()),
                "{p}"
            );
        }
        let r = fetch(s.addr, &format!("/tx/{t0}/outspend/0")).await;
        assert!(r.2.starts_with(r#"{"spent":true"#), "{r:?}");
        // IBD latches off once a node has caught up, so a node that falls
        // behind later says so only through its other signals.
        node.with(|st| {
            st.ibd = false;
            st.behind = 3;
        });
        let r = fetch(s.addr, &format!("/tx/{t0}/outspend/7")).await;
        assert_eq!(r, (503, TEXT.into(), NODE_BEHIND.into()));
        node.with(|st| st.behind = 0);
        let r = fetch(s.addr, &format!("/tx/{t0}/outspend/7")).await;
        assert_eq!(r, (200, JSON.into(), UNSPENT_JSON.into()));
    }

    /// The node stops knowing the index's tip at all (a -reindex, or a
    /// different datadir behind the same port). The index must take it off
    /// and carry on, not answer 503 until someone restarts it.
    #[tokio::test]
    async fn an_index_tip_the_node_no_longer_knows_is_disconnected() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let s = serve(node.clone()).await;
        wait_ready(&s, 288860).await;
        node.with(|st| {
            let gone = st.active.pop().unwrap();
            st.blocks.remove(&gone);
        });
        let spender = "1f01798f15c704e13e31c478f75f68cb795acee603f4b2503ebf61ad36ecac46";
        for _ in 0..500 {
            let r = fetch(s.addr, &format!("/tx/{spender}/status")).await;
            if r == (404, TEXT.into(), TX_NOT_FOUND.into()) {
                assert_eq!(s.swap_stats().await.unwrap().1.tip, 288859);
                return;
            }
            assert_eq!(r.0, 503, "{r:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the index never let go of a tip the node does not know");
    }

    /// A reorg that replaces the block holding a spend must turn "spent" into
    /// "unspent" — through a 503 while the index catches up, never through a
    /// stale answer — and back again when the chain returns.
    #[tokio::test]
    async fn a_reorg_is_followed_and_never_answered_stale() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let s = serve(node.clone()).await;
        wait_ready(&s, 288860).await;
        let funding = "37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5";
        let spender = "1f01798f15c704e13e31c478f75f68cb795acee603f4b2503ebf61ad36ecac46";
        let spent = fetch(s.addr, &format!("/tx/{funding}/outspend/0")).await;
        assert!(spent.2.starts_with(r#"{"spent":true"#), "{spent:?}");

        // Replace 288860 with a sibling that spends nothing.
        let real = node.with(|st| st.active[2].clone());
        let fake = "3333333333333333333333333333333333333333333333333333333333333333";
        let fake_cb = "4444444444444444444444444444444444444444444444444444444444444444";
        node.with(|st| {
            let prev = st.active[1].clone();
            st.blocks.insert(
                fake.into(),
                json!({
                    "hash": fake, "height": 288860, "time": 1, "previousblockhash": prev,
                    "merkleroot": fake_cb,
                    "tx": [{"txid": fake_cb, "vin": [{"coinbase": "00"}]}],
                }),
            );
            st.active[2] = fake.into();
        });
        // Every answer about the replaced block is either a 503 (not caught
        // up) or the new truth. Never the old "spent", never the old status.
        let mut saw_new = false;
        for _ in 0..300 {
            let st = fetch(s.addr, &format!("/tx/{spender}/status")).await;
            let os = fetch(s.addr, &format!("/tx/{funding}/outspend/0")).await;
            assert!(
                st.0 == 503 || st == (404, TEXT.into(), TX_NOT_FOUND.into()),
                "{st:?}"
            );
            assert!(os.0 == 503 || os.2 == UNSPENT_JSON, "{os:?}");
            if st.0 == 404 && os.2 == UNSPENT_JSON {
                saw_new = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(saw_new, "the index never followed the reorg");
        let cb = fetch(s.addr, &format!("/tx/{fake_cb}/status")).await;
        assert_eq!(cb.0, 200, "{cb:?}");
        // The proof for the new block's only tx: the tx is its own root.
        let pf = fetch(s.addr, &format!("/tx/{fake_cb}/merkle-proof")).await;
        assert_eq!(pf.2, r#"{"block_height":288860,"merkle":[],"pos":0}"#);
        // The replaced block's header is a side branch now: 404, as electrs.
        let h = fetch(s.addr, &format!("/block/{real}/header")).await;
        assert_eq!(h, (404, TEXT.into(), BLOCK_NOT_FOUND.into()));

        // And back.
        node.with(|st| st.active[2] = real.clone());
        for _ in 0..300 {
            let os = fetch(s.addr, &format!("/tx/{funding}/outspend/0")).await;
            if os.2.starts_with(r#"{"spent":true"#) {
                return;
            }
            assert!(os.0 == 503 || os.2 == UNSPENT_JSON, "{os:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the index never followed the chain back");
    }

    /// While the first scan runs nothing is answered from the index: a half
    /// built index would call an output unspent while its spend sits in a
    /// block not read yet.
    #[tokio::test]
    async fn nothing_is_answered_from_an_index_that_is_not_built() {
        let node = ChainNode::from_fixture(&[288858, 288859, 288860]);
        let index = SwapIndex::new(1000);
        let t =
            parse_id("37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5").unwrap();
        for r in [
            answer_status(&node, &index, &t).await,
            answer_hex(&node, &index, &t).await,
            answer_merkle_proof(&node, &index, &t).await,
            answer_outspend(&node, &index, &t, 0).await,
        ] {
            assert_eq!(r.status, 503, "{r:?}");
        }
        // The header needs no index.
        let h =
            parse_id("429bfa29bdb12edbb5491c6a66e9ebccd5cb34cb25584118d1acd4c6cdc4ddc2").unwrap();
        assert_eq!(answer_header(&node, &h).await.status, 200);
    }

    /// The node prunes its bottom block while the index starts: the scan steps
    /// past it rather than failing.
    #[tokio::test]
    async fn the_first_scan_steps_past_a_block_pruned_under_it() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        // getblockchaininfo still reports 288858, but its data is gone.
        node.with(|st| st.pruned_below = 288859);
        let index = SwapIndex::new(1000);
        let gate = tokio::sync::Semaphore::new(MAX_INFLIGHT);
        index.sync_once(node.as_ref(), &gate).await.unwrap();
        let st = index.state.read().await.stats();
        assert_eq!((st.bottom, st.tip, st.blocks), (288859, 288860, 2));
    }

    /// The app's witness is on by default, on loopback. There it must not
    /// build an index nobody can reach; bound publicly it serves the routes.
    #[test]
    fn the_app_builds_the_index_only_for_a_public_bind() {
        for local in ["127.0.0.1:3081", "[::1]:3081"] {
            assert_eq!(
                WitnessOptions::for_bind(local),
                WitnessOptions::without_swap()
            );
        }
        for public in ["0.0.0.0:3081", "192.168.1.50:3081", "[::]:3081"] {
            assert_eq!(
                WitnessOptions::for_bind(public).swap_window,
                Some(crate::swap_index::DEFAULT_WINDOW)
            );
        }
    }

    /// `start`, which the app used before the swap routes, still starts the
    /// witness without them.
    #[tokio::test]
    async fn start_alone_is_still_the_witness_without_swap() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let s = WitnessServer::start(node.clone(), "127.0.0.1:0")
            .await
            .unwrap();
        assert!(s.swap_stats().await.is_none());
        let t = "37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5";
        assert_eq!(fetch(s.addr, &format!("/tx/{t}/status")).await.0, 404);
    }

    #[tokio::test]
    async fn a_witness_started_without_swap_serves_none_of_the_five() {
        let node = Arc::new(ChainNode::from_fixture(&[288858, 288859, 288860]));
        let s =
            WitnessServer::start_with(node.clone(), "127.0.0.1:0", WitnessOptions::without_swap())
                .await
                .unwrap();
        let t = "37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5";
        let h = "429bfa29bdb12edbb5491c6a66e9ebccd5cb34cb25584118d1acd4c6cdc4ddc2";
        for p in [
            format!("/tx/{t}/status"),
            format!("/tx/{t}/hex"),
            format!("/tx/{t}/merkle-proof"),
            format!("/tx/{t}/outspend/0"),
            format!("/block/{h}/header"),
        ] {
            assert_eq!(fetch(s.addr, &p).await.0, 404, "{p}");
        }
        assert!(s.swap_stats().await.is_none());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            node.with(|st| st.calls.is_empty()),
            "the node was asked something"
        );
    }

    #[test]
    fn the_swap_routes_have_exactly_one_spelling() {
        let t = "37f03c45f1ecf44b91b52c9b5ff7debc5067cf1669e0ab714b35ad3c35354fb5";
        let id = parse_id(t).unwrap();
        assert_eq!(
            route("GET", &format!("/tx/{t}/status")),
            Some(WitnessRoute::TxStatus(id))
        );
        assert_eq!(
            route("GET", &format!("/tx/{t}/hex")),
            Some(WitnessRoute::TxHex(id))
        );
        assert_eq!(
            route("GET", &format!("/tx/{t}/merkle-proof")),
            Some(WitnessRoute::TxMerkleProof(id))
        );
        assert_eq!(
            route("GET", &format!("/tx/{t}/outspend/0")),
            Some(WitnessRoute::TxOutspend(id, 0))
        );
        assert_eq!(
            route("GET", &format!("/tx/{t}/outspend/9999")),
            Some(WitnessRoute::TxOutspend(id, MAX_VOUT))
        );
        assert_eq!(
            route("GET", &format!("/block/{t}/header")),
            Some(WitnessRoute::BlockHeader(id))
        );
        let upper = t.to_uppercase();
        let short = &t[..63];
        for p in [
            // ids: case, length, characters
            format!("/tx/{upper}/status"),
            format!("/tx/{short}/status"),
            format!("/tx/{t}0/status"),
            format!("/tx/{}g/hex", &t[..63]),
            format!("/block/{upper}/header"),
            format!("/block/{short}/header"),
            // vouts: one spelling, a small cap
            format!("/tx/{t}/outspend/00"),
            format!("/tx/{t}/outspend/01"),
            format!("/tx/{t}/outspend/10000"),
            format!("/tx/{t}/outspend/99999"),
            format!("/tx/{t}/outspend/-1"),
            format!("/tx/{t}/outspend/+1"),
            format!("/tx/{t}/outspend/1e3"),
            format!("/tx/{t}/outspend/0x1"),
            format!("/tx/{t}/outspend/ 1"),
            format!("/tx/{t}/outspend/١"),
            format!("/tx/{t}/outspend/"),
            format!("/tx/{t}/outspend"),
            format!("/tx/{t}/outspend/1/2"),
            // shape: no trailing slash, query, fragment, extra segment
            format!("/tx/{t}/status/"),
            format!("/tx/{t}/hex?x=1"),
            format!("/tx/{t}/merkle-proof#f"),
            format!("/block/{t}/header/"),
            format!("/block/{t}/header?seed=1"),
            format!("//tx/{t}/status"),
            format!("/tx//{t}/status"),
            // electrs routes a witness does not serve
            format!("/tx/{t}"),
            format!("/tx/{t}/raw"),
            format!("/tx/{t}/outspends"),
            format!("/tx/{t}/merkleblock-proof"),
            format!("/block/{t}"),
            format!("/block/{t}/status"),
            format!("/block/{t}/txids"),
            format!("/block/{t}/raw"),
            format!("/block/{t}/txid/0"),
        ] {
            assert_eq!(route("GET", &p), None, "{p} must be refused");
        }
        for m in ["POST", "PUT", "HEAD", "OPTIONS", "get"] {
            assert_eq!(route(m, &format!("/tx/{t}/status")), None, "{m}");
            assert_eq!(route(m, &format!("/block/{t}/header")), None, "{m}");
        }
    }
}
