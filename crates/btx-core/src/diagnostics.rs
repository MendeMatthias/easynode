//! Copy diagnostics: one plain-text report a person can paste into a support
//! chat, and the redaction that keeps anything private out of it. Pure: the
//! Tauri side gathers the inputs and calls `report(..)`.

use crate::fork::ChainTip;
use crate::node_api::{AttestedTip, BlockchainInfo, ChainStates, PeerInfo};

pub const LOG_TAIL_BYTES: u64 = 2 * 1024 * 1024;
pub const LOG_WARNING_LINES: usize = 20;
pub const LOG_LINE_CHARS: usize = 240;

/// The last warning or error lines of a log tail, oldest first, whole.
///
/// Not cut here: a cut that lands inside an address or a key leaves a piece
/// the redaction can no longer recognise (`84.32.49.226:193…`, the first 30
/// characters of a WIF). [`report`] cuts each line to [`LOG_LINE_CHARS`]
/// only after it has been redacted.
pub fn warning_lines(log_tail: &str) -> Vec<String> {
    let mut lines: Vec<String> = log_tail
        .lines()
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            low.contains("[warning]")
                || low.contains("[error]")
                || low.contains("warning:")
                || low.contains("error:")
                || l.contains("Cadence burst hold")
        })
        .map(|l| l.trim_end().to_string())
        .collect();
    let skip = lines.len().saturating_sub(LOG_WARNING_LINES);
    lines.drain(..skip);
    lines
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[derive(Debug, Clone, Default)]
pub struct HeldBranchState {
    pub height: u64,
    pub root: String,
    pub state: String,
}

#[derive(Debug, Clone, Default)]
pub struct DiagnosticsInput {
    pub generated_at: String,
    pub app_version: String,
    pub engine_pinned: String,
    pub engine_running: Option<String>,
    pub platform: String,
    pub role: String,
    pub signer_pubkey: Option<String>,
    pub status_line: String,
    pub window_lines: Vec<String>,
    pub phase: String,
    pub chain: Option<BlockchainInfo>,
    pub best_block_hash: Option<String>,
    pub chainstates: Option<ChainStates>,
    /// Where the running chain started and who confirmed it
    /// (`snapshot_start::started_from_current`), or `None`.
    pub started_from: Option<String>,
    pub tips: Vec<ChainTip>,
    pub held: Vec<HeldBranchState>,
    pub peers: Vec<PeerInfo>,
    pub attested_tip: Option<AttestedTip>,
    pub stall: Option<String>,
    pub log_warnings: Vec<String>,
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn history(p: &PeerInfo) -> &'static str {
    if p.servicesnames.iter().any(|s| s == "NETWORK") {
        "full history"
    } else if p.servicesnames.iter().any(|s| s == "NETWORK_LIMITED") {
        "recent history"
    } else {
        "no history"
    }
}

pub fn render(i: &DiagnosticsInput) -> String {
    let mut o: Vec<String> = Vec::new();
    o.push(format!("easyNode diagnostics, {}", i.generated_at));
    o.push(format!(
        "App {} · engine {} (running: {}) · {}",
        i.app_version,
        i.engine_pinned,
        i.engine_running.as_deref().unwrap_or("not running"),
        i.platform
    ));
    o.push(format!("Role: {}", i.role));
    o.push(format!(
        "Signing key: {}",
        i.signer_pubkey.as_deref().unwrap_or("off")
    ));
    o.push(String::new());
    o.push(format!("Status: {}", i.status_line));
    o.extend(i.window_lines.iter().cloned());
    o.push(format!("Phase: {}", i.phase));
    o.push(String::new());
    o.push("Chain".into());
    match &i.chain {
        Some(c) => o.push(format!(
            "  blocks {} · headers {} · progress {:.4} · first sync {}",
            group(c.blocks),
            group(c.headers),
            c.verification_progress,
            if c.initial_block_download {
                "yes"
            } else {
                "no"
            }
        )),
        None => o.push("  not answering".into()),
    }
    if let Some(h) = &i.best_block_hash {
        o.push(format!("  tip {h}"));
    }
    if let Some(cs) = &i.chainstates {
        for c in &cs.chainstates {
            match &c.snapshot_blockhash {
                Some(base) => o.push(format!(
                    "  snapshot chain state at {} (base {}), checked: {}",
                    group(c.blocks),
                    &base[..16.min(base.len())],
                    if c.validated { "yes" } else { "not yet" }
                )),
                None => o.push(format!("  history check at {}", group(c.blocks))),
            }
        }
    }
    if let Some(from) = &i.started_from {
        o.push(format!("  {from}"));
    }
    let others: Vec<&ChainTip> = i
        .tips
        .iter()
        .filter(|t| t.status != "active" && t.branchlen > 1)
        .collect();
    o.push(format!(
        "Chain tips ({}), branches longer than one block:",
        i.tips.len()
    ));
    for t in &others {
        o.push(format!(
            "  {} {} length {} {}",
            group(t.height),
            &t.hash[..16.min(t.hash.len())],
            t.branchlen,
            t.status
        ));
    }
    o.push("Held branches".into());
    for h in &i.held {
        o.push(format!("  {} {}: {}", group(h.height), h.root, h.state));
    }
    let inbound = i.peers.iter().filter(|p| p.inbound).count();
    o.push(format!(
        "Peers ({} in, {} out)",
        inbound,
        i.peers.len() - inbound
    ));
    for p in &i.peers {
        o.push(format!(
            "  peer {}: {} {} headers {} blocks {} · {} · {}",
            p.id,
            p.addr,
            p.subver,
            p.synced_headers,
            p.synced_blocks,
            history(p),
            p.connection_type
        ));
    }
    if let Some(a) = &i.attested_tip {
        o.push(format!(
            "Signed frontier: height {} · {} behind · on this chain: {}",
            a.height.map(group).unwrap_or_else(|| "unknown".into()),
            a.blocks_behind
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unknown".into()),
            match a.on_active_chain {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            }
        ));
    }
    let notices = i
        .chain
        .as_ref()
        .map(crate::engine_warnings::all_notices)
        .unwrap_or_default();
    o.push(format!("Engine notices ({})", notices.len()));
    for n in &notices {
        let hidden = if n.hidden_because.is_some() {
            " (not shown on the home screen)"
        } else {
            ""
        };
        o.push(format!("  - {}{}", n.message, hidden));
        o.push(format!("    engine: {}", n.raw));
    }
    o.push(format!(
        "Watchdog: {}",
        i.stall.as_deref().unwrap_or("nothing to report")
    ));
    o.push(format!(
        "Last warning lines of debug.log ({})",
        i.log_warnings.len()
    ));
    for l in &i.log_warnings {
        o.push(format!("  {l}"));
    }
    o.join("\n")
}

pub struct RedactionContext {
    /// The person's home folder, written as `~`.
    pub home: Option<String>,
    /// Exact strings that must never appear: the cookie password, the key file.
    pub secrets: Vec<String>,
    /// Peer hosts the app ships in its source; every other address is removed.
    pub published_hosts: Vec<String>,
}

/// The report exactly as it may leave the app, in the only safe order: each
/// debug.log line is redacted whole and cut to [`LOG_LINE_CHARS`] after, so
/// a cut can never split an address or a key into a piece that no longer
/// looks like one. Then the whole report is redacted once more.
pub fn report(input: &DiagnosticsInput, ctx: &RedactionContext) -> String {
    let mut shown = input.clone();
    shown.log_warnings = input
        .log_warnings
        .iter()
        .map(|l| cut(&redact(l, ctx), LOG_LINE_CHARS))
        .collect();
    redact(&render(&shown), ctx)
}

pub fn redact(text: &str, ctx: &RedactionContext) -> String {
    let mut out = text.to_string();
    // 4 is the floor, not 1: an empty or 1-3 character "secret" would match
    // all over an ordinary report and shred it.
    for s in ctx
        .secrets
        .iter()
        .map(|s| s.trim())
        .filter(|s| s.len() >= 4)
    {
        out = out.replace(s, "[removed]");
    }
    if let Some(home) = ctx
        .home
        .as_deref()
        .map(|h| h.trim_end_matches(['/', '\\']))
        .filter(|h| h.len() > 1)
    {
        out = out.replace(home, "~");
    }
    out.lines()
        .map(|l| redact_line(l, ctx))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ',' | ';' | '(' | ')' | '=' | '"' | '\'' | '<' | '>' | '{' | '}' | '/' | '@' | '|'
        )
}

/// One line of the report. IP addresses, keys and payment addresses are
/// found wherever they sit in the line, whatever punctuation is glued to
/// them, so no log format can hide one by where it puts a bracket. Host
/// names come last, token by token: only the token around a name tells a
/// peer (`node.example.com:19335`) from prose and source lines.
fn redact_line(line: &str, ctx: &RedactionContext) -> String {
    let line = splice(line, ipv6_spans(line, ctx));
    let line = splice(&line, ipv4_spans(&line, ctx));
    let line = splice(&line, key_spans(&line));
    let line = splice(&line, payment_address_spans(&line));
    redact_names(&line, ctx)
}

/// A part of a line to replace: its byte span and what goes in its place.
type Span = (usize, usize, &'static str);

/// `line` with each span replaced. Spans come in order; one that overlaps
/// the span before it is skipped, since that text is already gone.
fn splice(line: &str, spans: Vec<Span>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    for (s, e, with) in spans {
        if s < at {
            continue;
        }
        out.push_str(&line[at..s]);
        out.push_str(with);
        at = e;
    }
    out.push_str(&line[at..]);
    out
}

/// Every maximal run of bytes in `line` that `class` accepts, as byte spans.
/// `class` only ever accepts ASCII, so a span always starts and ends on a
/// character boundary.
fn runs(line: &str, class: impl Fn(u8) -> bool) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !class(b[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < b.len() && class(b[i]) {
            i += 1;
        }
        out.push((s, i));
    }
    out
}

/// Whether `ip` is one of the peers this app publishes, which stay.
fn is_published_ip(ip: std::net::IpAddr, ctx: &RedactionContext) -> bool {
    ctx.published_hosts.iter().any(|h| {
        h.trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|p| p == ip)
    })
}

fn is_port(t: &str) -> bool {
    (1..=5).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit())
}

/// `[ip]:port` around the address at `s..e`: the brackets and the port
/// belong to it and go with it, as a socket address always has. Without a
/// port the brackets are the log's own and stay: `[[peer address]]`.
fn with_brackets_and_port(line: &str, s: usize, e: usize) -> (usize, usize) {
    let b = line.as_bytes();
    if s == 0 || b[s - 1] != b'[' || !line[e..].starts_with("]:") {
        return (s, e);
    }
    let digits = line[e + 2..].bytes().take_while(u8::is_ascii_digit).count();
    if is_port(&line[e + 2..e + 2 + digits]) {
        (s - 1, e + 2 + digits)
    } else {
        (s, e)
    }
}

/// IPv6 addresses anywhere in a line: maximal runs of hex digits, `:` and
/// `.` that parse as `Ipv6Addr`, bracketed or not, with a `:port` when they
/// carry one. A run glued to a word is a C++ name
/// (`Chainstate::ActivateBestChain` holds the run `e::Ac`), never an address.
fn ipv6_spans(line: &str, ctx: &RedactionContext) -> Vec<Span> {
    let b = line.as_bytes();
    let word = |x: u8| x.is_ascii_alphanumeric() || x == b'_';
    let mut out = Vec::new();
    for (s, e) in runs(line, |x| x.is_ascii_hexdigit() || x == b':' || x == b'.') {
        let mut cs = s;
        let mut ce = e;
        // `peer:fe80::1`: one leading colon belongs to the word before it.
        if line[cs..ce].starts_with(':') && !line[cs..ce].starts_with("::") {
            cs += 1;
        }
        // End of a sentence, or a stray colon; never the `::` of `fe80::`.
        while ce > cs && b[ce - 1] == b'.' {
            ce -= 1;
        }
        if line[cs..ce].ends_with(':') && !line[cs..ce].ends_with("::") {
            ce -= 1;
        }
        let core = &line[cs..ce];
        if !core.bytes().any(|x| x.is_ascii_hexdigit()) {
            continue; // `::` on its own is punctuation, not a peer
        }
        if (cs > 0 && word(b[cs - 1])) || (ce < b.len() && word(b[ce])) {
            continue;
        }
        // Unbracketed with a port, `2001:db8::7:19335`, when the whole run
        // is not an address by itself.
        let ip = core.parse::<std::net::Ipv6Addr>().ok().or_else(|| {
            core.rsplit_once(':')
                .filter(|(_, port)| is_port(port))
                .and_then(|(host, _)| host.parse().ok())
        });
        let Some(ip) = ip else { continue };
        if is_published_ip(std::net::IpAddr::V6(ip), ctx) {
            continue;
        }
        let (s, e) = with_brackets_and_port(line, cs, ce);
        out.push((s, e, "[peer address]"));
    }
    out
}

/// Four dot-separated decimal octets, each 0-255 and at most three digits.
fn parse_ipv4(t: &str) -> Option<std::net::Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut parts = t.split('.');
    for o in &mut octets {
        let p = parts.next()?;
        if !(1..=3).contains(&p.len()) || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    parts.next().is_none().then_some(octets.into())
}

/// IPv4 addresses anywhere in a line: maximal runs of digits, `.` and `:`,
/// cut at each `:` into parts, and every part that is four octets is an
/// address, with the part after it as its port when that is a port. Times
/// (`10:11:20`), versions (`0.34.9`), heights and byte counts are never four
/// octets.
fn ipv4_spans(line: &str, ctx: &RedactionContext) -> Vec<Span> {
    let mut out = Vec::new();
    for (s, e) in runs(line, |x| x.is_ascii_digit() || x == b'.' || x == b':') {
        let mut parts = Vec::new();
        let mut at = s;
        for p in line[s..e].split(':') {
            parts.push((at, at + p.len()));
            at += p.len() + 1;
        }
        let mut i = 0;
        while i < parts.len() {
            let (ps, pe) = parts[i];
            i += 1;
            let part = &line[ps..pe];
            let lead = part.len() - part.trim_start_matches('.').len();
            let cs = ps + lead;
            let ce = cs + line[cs..pe].trim_end_matches('.').len();
            let Some(ip) = parse_ipv4(&line[cs..ce]) else {
                continue;
            };
            let mut ce2 = ce;
            if ce == pe && i < parts.len() {
                let (qs, qe) = parts[i];
                let port = line[qs..qe].trim_end_matches('.');
                if is_port(port) {
                    ce2 = qs + port.len();
                    i += 1;
                }
            }
            if is_published_ip(std::net::IpAddr::V4(ip), ctx) {
                continue;
            }
            let (s, e) = if ce2 == ce {
                with_brackets_and_port(line, cs, ce)
            } else {
                (cs, ce2)
            };
            out.push((s, e, "[peer address]"));
        }
    }
    out
}

/// Keys anywhere in a line: a run of base58 long enough to be a WIF or an
/// extended key, whatever is glued to it.
fn key_spans(line: &str) -> Vec<Span> {
    runs(line, |x| BASE58.as_bytes().contains(&x))
        .into_iter()
        .filter(|&(s, e)| looks_like_key(&line[s..e]))
        .map(|(s, e)| (s, e, "[key removed]"))
        .collect()
}

/// Payment addresses anywhere in a line: a run of letters and digits that
/// reads as a `btx1` address.
fn payment_address_spans(line: &str) -> Vec<Span> {
    runs(line, |x| x.is_ascii_alphanumeric())
        .into_iter()
        .filter(|&(s, e)| looks_like_address(&line[s..e]))
        .map(|(s, e)| (s, e, "[address removed]"))
        .collect()
}

/// Host names, token by token (IP addresses in tokens are caught here too,
/// as a second net under the line-wide scans above).
fn redact_names(line: &str, ctx: &RedactionContext) -> String {
    let mut out = String::with_capacity(line.len());
    let mut token = String::new();
    for c in line.chars() {
        if is_separator(c) {
            out.push_str(&redact_token(&token, ctx));
            token.clear();
            out.push(c);
        } else {
            token.push(c);
        }
    }
    out.push_str(&redact_token(&token, ctx));
    out
}

/// Trailing sentence/URL punctuation, never part of the value before it:
/// "84.32.49.226:19335." (end of a sentence), "...:19335:" (a stray colon)
/// and "...:19335…" (a cut line) must all still be recognised.
const TRAILING: [char; 3] = ['.', ':', '…'];

/// How many places in one piece of a token an address may start: the piece
/// itself and after its first `:` or `[` characters. Bounds the work on a
/// pathological log line; real ones have a handful.
const MAX_STARTS: usize = 16;

/// A token of a line. Each address in it becomes a marker and the
/// punctuation around it stays: `addr:[peer address]`. A published peer is
/// left as it is.
fn redact_token(piece: &str, ctx: &RedactionContext) -> String {
    let mut out = String::with_capacity(piece.len());
    let mut rest = piece;
    while let Some((s, e, found)) = find_private(rest) {
        out.push_str(&rest[..s]);
        match found {
            Found::Peer(host)
                if ctx
                    .published_hosts
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(&host)) =>
            {
                out.push_str(&rest[s..e])
            }
            Found::Peer(_) => out.push_str("[peer address]"),
            Found::Address => out.push_str("[address removed]"),
        }
        rest = &rest[e..];
    }
    out.push_str(rest);
    out
}

enum Found {
    Peer(String),
    Address,
}

/// The first peer address or payment address in `piece`, as a byte span:
/// the whole piece (`84.32.49.226:19335`, `[2001:db8::7]:19335`), or a part
/// that starts after a `:` or a `[` (`addr:1.2.3.4`, `[1.2.3.4]`). Trailing
/// punctuation and closing brackets stay outside the span.
fn find_private(piece: &str) -> Option<(usize, usize, Found)> {
    let starts = std::iter::once(0)
        .chain(piece.match_indices([':', '[']).map(|(i, _)| i + 1))
        .take(MAX_STARTS);
    for s in starts {
        let whole = piece[s..].trim_end_matches(TRAILING);
        let unbracketed = whole.trim_end_matches(']').trim_end_matches(TRAILING);
        for c in [whole, unbracketed] {
            if c.is_empty() {
                continue;
            }
            if let Some(host) = address_host(c) {
                return Some((s, s + c.len(), Found::Peer(host)));
            }
            if looks_like_address(c) {
                return Some((s, s + c.len(), Found::Address));
            }
        }
    }
    None
}

fn address_host(t: &str) -> Option<String> {
    use std::net::{IpAddr, SocketAddr};
    if let Ok(sa) = t.parse::<SocketAddr>() {
        return Some(sa.ip().to_string());
    }
    if let Ok(ip) = t.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    match t.rsplit_once(':') {
        // A numeric port after the colon: this is `host:port`. Engine source
        // references (`validation.cpp:17539`) have exactly this shape and
        // must be left alone; everything else with a dotted, lettered host
        // is a DNS-style peer address, published or not.
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            // `[1.2.3.4]:19335`: an IPv4 host in the brackets IPv6 uses.
            let host = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            if let Ok(ip) = host.parse::<IpAddr>() {
                return Some(ip.to_string());
            }
            if is_source_reference(host) || !could_be_host_name(host) {
                return None;
            }
            let low = host.to_ascii_lowercase();
            (low.ends_with(".onion")
                || low.ends_with(".i2p")
                || low.ends_with(".local")
                || looks_like_dns_host(&low))
            .then_some(low)
        }
        // No numeric port: only a bare onion/i2p address or a bare ".local"
        // machine name counts as an address. A bare version number like
        // `v0.34.9` must not.
        _ => {
            if !could_be_host_name(t) {
                return None;
            }
            let low = t.to_ascii_lowercase();
            (low.ends_with(".onion") || low.ends_with(".i2p") || low.ends_with(".local"))
                .then_some(low)
        }
    }
}

/// A host name never holds `:`, `[` or `]`. When one of them is there, the
/// name starts after it, and `find_private` tries that start on its own:
/// `addr:node.example.com:19335` is the host `node.example.com`, not
/// `addr:node.example.com`, so a published peer written that way stays.
fn could_be_host_name(host: &str) -> bool {
    !host.contains([':', '[', ']'])
}

/// Whether `host` is the engine's own way of citing a line in its source,
/// e.g. `validation.cpp` in `validation.cpp:17539` or its `-logsourcelocations`
/// prefix `[validation.cpp:17539]`. Only the two extensions the engine cites
/// count: `.rs`, `.py` and most other file extensions are also country or
/// generic domains, and `mynode.example.rs:19335` is a peer.
fn is_source_reference(host: &str) -> bool {
    const EXTENSIONS: [&str; 2] = ["cpp", "h"];
    match host.rsplit_once('.') {
        Some((_, ext)) => EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)),
        None => false,
    }
}

/// A DNS-style peer host: a label separator and at least one letter, so a
/// bare version number's dotted form (`0.34.9`) is never mistaken for one
/// (it never carries a `:port` either, which the caller already requires).
fn looks_like_dns_host(host: &str) -> bool {
    host.contains('.') && host.chars().any(|c| c.is_ascii_alphabetic())
}

const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// WIF, extended keys and the like: long base58, not plain hex (hashes and
/// public keys are hex and stay).
fn looks_like_key(t: &str) -> bool {
    t.len() >= 50
        && t.chars().all(|c| BASE58.contains(c))
        && !t.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_address(t: &str) -> bool {
    (t.starts_with("btx1") || t.starts_with("btxrt1") || t.starts_with("tbtx1"))
        && t.len() >= 40
        && t.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_api::PeerInfo;

    fn ctx(wif: &str) -> RedactionContext {
        RedactionContext {
            home: Some("/Users/alice".into()),
            secrets: vec!["cookiepassword123".into(), wif.into()],
            published_hosts: crate::node::published_peer_hosts(),
        }
    }

    /// The last twenty, whole: the cut to 240 characters happens in
    /// `report`, after redaction, never here.
    #[test]
    fn warning_lines_keep_the_last_twenty_warnings_and_report_cuts_them() {
        let mut log = String::new();
        for i in 0..30 {
            log.push_str(&format!("2026-09-29T10:00:{i:02}Z [warning] thing {i}\n"));
            log.push_str("2026-09-29T10:00:00Z ordinary line\n");
        }
        log.push_str(&format!(
            "2026-09-29T10:01:00Z [error] {}\n",
            "a long line ".repeat(40)
        ));
        let lines = warning_lines(&log);
        assert_eq!(lines.len(), 20);
        assert!(lines[0].contains("thing 11"));
        assert!(lines[19].chars().count() > 400, "cut too early");
        assert!(lines.iter().all(|l| !l.contains("ordinary")));
        assert!(warning_lines("Cadence burst hold at 82000\n")[0].contains("Cadence"));
        let input = DiagnosticsInput {
            log_warnings: lines,
            ..Default::default()
        };
        let out = report(&input, &ctx("unusedsecret"));
        let long = out
            .lines()
            .find(|l| l.contains("[error]"))
            .expect("the long line is in the report");
        assert_eq!(long.trim_start().chars().count(), LOG_LINE_CHARS + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn redaction_removes_every_item_on_the_never_list() {
        let wif = crate::signer::generate_wif();
        let shaped: String = "P"
            .chars()
            .chain(
                "abcdefghijkmnopqrstuvwxyz123456789"
                    .chars()
                    .cycle()
                    .take(51),
            )
            .collect();
        let text = format!(
            "cookie __cookie__:cookiepassword123\n\
             key {wif}\n\
             another key-shaped token {shaped}.\n\
             path /Users/alice/.easybtx/debug.log\n\
             peer 12 addr=84.32.49.226:19335 and [2001:db8::7]:19335 and abcdefghijklmnop.onion:19335\n\
             pay btx1qxyzxyzxyzxyzxyzxyzxyzxyzxyzxyzxyzxyzxyzxyz\n\
             archive 109.199.124.187:19335 mirror 20.86.181.203:19338 relay node.btx.dev:19335\n\
             hash 8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c\n\
             pubkey 02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
        );
        let out = redact(&text, &ctx(&wif));
        for gone in [
            "cookiepassword123",
            wif.as_str(),
            shaped.as_str(),
            "/Users/alice",
            "84.32.49.226",
            "2001:db8::7",
            "abcdefghijklmnop.onion",
            "btx1qxyz",
        ] {
            assert!(!out.contains(gone), "{gone} survived:\n{out}");
        }
        for kept in [
            "~/.easybtx/debug.log",
            "109.199.124.187:19335",
            "20.86.181.203:19338",
            "node.btx.dev:19335",
            "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c",
            "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675",
            "peer 12",
        ] {
            assert!(out.contains(kept), "{kept} was removed:\n{out}");
        }
        assert!(
            out.contains("[peer address]")
                && out.contains("[key removed]")
                && out.contains("[address removed]")
        );
    }

    #[test]
    fn version_numbers_and_times_are_not_mistaken_for_addresses() {
        let out = redact(
            "engine v0.34.9 at 2026-09-29T10:11:20Z height 233,470",
            &ctx("unusedsecret"),
        );
        assert_eq!(out, "engine v0.34.9 at 2026-09-29T10:11:20Z height 233,470");
    }

    /// An unpublished DNS-style peer host must not survive, and a published
    /// one must, exactly as the equivalent IP-shaped hosts already do.
    #[test]
    fn unpublished_dns_hostnames_are_redacted_but_published_ones_stay() {
        let out = redact(
            "mirror node.btx.dev:19335 home myhome-node.duckdns.org:19335",
            &ctx("unusedsecret"),
        );
        assert!(
            out.contains("node.btx.dev:19335"),
            "published host removed:\n{out}"
        );
        assert!(
            !out.contains("myhome-node.duckdns.org"),
            "unpublished host survived:\n{out}"
        );
        assert!(
            out.contains("[peer address]"),
            "no redaction marker:\n{out}"
        );
    }

    /// A machine's own `.local` name is never a published peer host, so it
    /// must go whether or not it carries a port, and regardless of case.
    #[test]
    fn bare_machine_names_ending_in_dot_local_are_redacted() {
        let out = redact(
            "host Alices-MacBook-Pro.local and Bobs-PC.LOCAL:19335",
            &ctx("unusedsecret"),
        );
        assert!(
            !out.contains("Alices-MacBook-Pro"),
            "bare .local name survived:\n{out}"
        );
        assert!(
            !out.contains("Bobs-PC"),
            ".local name with a port survived:\n{out}"
        );
        assert!(
            out.contains("[peer address]"),
            "no redaction marker:\n{out}"
        );
    }

    /// `file.cpp:line` is how the engine cites its own source, never a peer;
    /// the exact shape that could be confused with `host:port` must survive.
    #[test]
    fn engine_source_references_are_not_mistaken_for_addresses() {
        let out = redact(
            "(validation.cpp:17539) net_processing.cpp:1234 init.cpp:3961",
            &ctx("unusedsecret"),
        );
        assert_eq!(
            out,
            "(validation.cpp:17539) net_processing.cpp:1234 init.cpp:3961"
        );
    }

    /// A peer IP wrapped in a URL, an `@`-prefixed userinfo, a stray trailing
    /// colon, or `|`-delimited log formatting must all still be caught; an
    /// ordinary update URL and the already-redacted home path must not be
    /// touched by the wider tokenizing this needs.
    #[test]
    fn url_wrapped_and_delimited_peer_addresses_are_redacted() {
        let context = ctx("unusedsecret");
        for wrapped in [
            "http://84.32.49.226:19335/",
            "user@84.32.49.226:19335",
            "84.32.49.226:19335:",
            "|84.32.49.226|",
        ] {
            let out = redact(wrapped, &context);
            assert!(
                !out.contains("84.32.49.226"),
                "IP survived in {wrapped:?}:\n{out}"
            );
        }
        for safe in [
            "https://easybtx.com/updater/latest-node.json",
            "~/.easybtx/debug.log",
        ] {
            let out = redact(safe, &context);
            assert_eq!(out, safe, "survivor mangled: {safe:?} -> {out:?}");
        }
    }

    /// Redact `text` with no secret known, so only the shape of a value can
    /// give it away, and return what comes out.
    fn shape_only(text: &str) -> String {
        redact(text, &ctx("unusedsecret"))
    }

    // Each shape below came out unchanged before: `[`, `]` and a leading
    // `word:` glued the value to its token, and the token as a whole was
    // neither an address nor base58. Engine forks bring their own log
    // formats, so none of them can be ruled out.

    #[test]
    fn an_ip_in_square_brackets_is_redacted() {
        assert_eq!(shape_only("from [1.2.3.4]"), "from [[peer address]]");
    }

    #[test]
    fn an_ip_and_port_in_square_brackets_is_redacted() {
        assert_eq!(shape_only("from [1.2.3.4:19335]"), "from [[peer address]]");
    }

    #[test]
    fn a_bracketed_ip_with_the_port_outside_is_redacted() {
        let out = shape_only("from [1.2.3.4]:19335");
        assert!(!out.contains("1.2.3.4"), "IP survived:\n{out}");
    }

    #[test]
    fn an_ip_before_a_closing_bracket_is_redacted() {
        assert_eq!(shape_only("from 1.2.3.4]"), "from [peer address]]");
    }

    #[test]
    fn an_ip_after_a_word_and_a_colon_is_redacted() {
        assert_eq!(shape_only("addr:1.2.3.4"), "addr:[peer address]");
    }

    #[test]
    fn a_key_after_a_word_and_a_colon_is_redacted() {
        let wif = crate::signer::generate_wif();
        assert_eq!(shape_only(&format!("key:{wif}")), "key:[key removed]");
    }

    #[test]
    fn a_key_in_square_brackets_is_redacted() {
        let wif = crate::signer::generate_wif();
        assert_eq!(shape_only(&format!("[{wif}]")), "[[key removed]]");
    }

    #[test]
    fn a_key_before_a_closing_bracket_is_redacted() {
        let wif = crate::signer::generate_wif();
        assert_eq!(shape_only(&format!("{wif}]")), "[key removed]]");
    }

    /// Unchanged by the bracket handling: the whole `[v6]:port` is one
    /// address, as it always was.
    #[test]
    fn a_bracketed_ipv6_with_a_port_is_still_redacted_whole() {
        assert_eq!(
            shape_only("peer [2001:db8::7]:19335 and addr:[2001:db8::7]:19335"),
            "peer [peer address] and addr:[peer address]"
        );
    }

    /// `.rs` is Serbia's country domain, not only a Rust file.
    #[test]
    fn a_host_under_the_rs_domain_is_redacted() {
        let out = shape_only("peer mynode.example.rs:19335");
        assert!(!out.contains("mynode"), "host survived:\n{out}");
    }

    /// `.py` is Paraguay's country domain, not only a Python file.
    #[test]
    fn a_host_under_the_py_domain_is_redacted() {
        let out = shape_only("peer mynode.example.py:19335");
        assert!(!out.contains("mynode"), "host survived:\n{out}");
    }

    /// The engine's own `-logsourcelocations` prefix is bracketed, so the
    /// bracket handling must still see `validation.cpp:17539` as a source
    /// line; a published peer in brackets stays too.
    #[test]
    fn bracketed_engine_source_lines_and_published_peers_stay() {
        let text = "[validation.cpp:17539] [ProcessNewBlock] [logging.h:88] \
                    via [109.199.124.187:19335]";
        assert_eq!(shape_only(text), text);
    }

    // N1: an address is found wherever it sits in a line, whatever is glued
    // to it. Each shape below came out unchanged after the I1 fix, because
    // an address was only found where it ran to the end of its token.

    #[test]
    fn an_ip_before_an_exclamation_mark_is_redacted() {
        assert_eq!(shape_only("from 1.2.3.4!"), "from [peer address]!");
    }

    #[test]
    fn an_ip_before_a_question_mark_is_redacted() {
        assert_eq!(shape_only("from 1.2.3.4?"), "from [peer address]?");
    }

    #[test]
    fn an_ip_and_port_before_punctuation_is_redacted() {
        assert_eq!(shape_only("from 1.2.3.4:19335!"), "from [peer address]!");
    }

    #[test]
    fn an_ip_and_port_after_a_hash_sign_is_redacted() {
        assert_eq!(shape_only("from #1.2.3.4:19335"), "from #[peer address]");
    }

    #[test]
    fn an_ip_in_backticks_is_redacted() {
        assert_eq!(shape_only("from `1.2.3.4`"), "from `[peer address]`");
    }

    #[test]
    fn an_ip_after_a_hyphen_is_redacted() {
        assert_eq!(shape_only("from peer-1.2.3.4"), "from peer-[peer address]");
    }

    #[test]
    fn two_bracketed_ips_that_touch_are_both_redacted() {
        assert_eq!(
            shape_only("from [84.32.49.226][5.6.7.8]"),
            "from [[peer address]][[peer address]]"
        );
    }

    #[test]
    fn a_bracketed_ip_touching_a_published_one_is_redacted_and_the_published_one_stays() {
        assert_eq!(
            shape_only("from [84.32.49.226][109.199.124.187]"),
            "from [[peer address]][109.199.124.187]"
        );
    }

    #[test]
    fn two_ips_with_ports_joined_by_a_colon_are_both_redacted() {
        assert_eq!(
            shape_only("from 84.32.49.226:19335:5.6.7.8:19335"),
            "from [peer address]:[peer address]"
        );
    }

    #[test]
    fn an_ipv6_before_punctuation_is_redacted() {
        assert_eq!(shape_only("from 2001:db8::7!"), "from [peer address]!");
    }

    #[test]
    fn two_ips_joined_by_punctuation_are_both_redacted() {
        assert_eq!(
            shape_only("from 1.2.3.4!5.6.7.8"),
            "from [peer address]![peer address]"
        );
    }

    #[test]
    fn an_ip_and_port_in_parentheses_is_redacted() {
        assert_eq!(
            shape_only("from (84.32.49.226:19335)"),
            "from ([peer address])"
        );
    }

    #[test]
    fn a_key_glued_between_punctuation_is_redacted() {
        let wif = crate::signer::generate_wif();
        for (text, want) in [
            (format!("!{wif}?"), "![key removed]?"),
            (format!("#{wif}#"), "#[key removed]#"),
            (format!("key:{wif}!"), "key:[key removed]!"),
            (format!("`{wif}`"), "`[key removed]`"),
        ] {
            assert_eq!(shape_only(&text), want);
        }
    }

    /// The published-peer exemption holds however the peer is wrapped.
    #[test]
    fn a_published_ip_glued_to_punctuation_stays() {
        let text = "via 109.199.124.187! #109.199.124.187:19335 `109.199.124.187` \
                    (109.199.124.187:19335)? [20.86.181.203][109.199.124.187]";
        assert_eq!(shape_only(text), text);
    }

    /// Finding addresses anywhere in a line must not find them where there
    /// are none: times, versions, heights, byte counts, hashes, outpoints,
    /// C++ names and the engine's source lines, glued to punctuation too.
    #[test]
    fn times_versions_numbers_hashes_and_names_are_not_addresses_anywhere() {
        let text = "2026-09-29T10:11:20.123456Z [msghand] (10:11:20)! v0.34.9, \
                    /BTX:0.34.11/ #233,470 height=233470 1048576 bytes 14:05? \
                    8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c:0 \
                    [validation.cpp:17539] [Chainstate::ActivateBestChain] \
                    CConnman::ThreadSocketHandler DB::Read progress=0.999871";
        assert_eq!(shape_only(text), text);
    }

    /// The `secrets` floor is 4 characters, not 8: a short passphrase or
    /// token must still be removed, while the floor itself keeps a 1-3
    /// character string from being treated as a secret and shredding the
    /// whole report.
    #[test]
    fn secrets_as_short_as_four_characters_are_removed() {
        let context = RedactionContext {
            home: None,
            secrets: vec!["ab12cd".into(), "no".into()],
            published_hosts: crate::node::published_peer_hosts(),
        };
        let out = redact(
            "token ab12cd appears here, unlike no which is too short",
            &context,
        );
        assert!(
            !out.contains("ab12cd"),
            "6-character secret survived:\n{out}"
        );
        assert!(
            out.contains(" no "),
            "a 2-character string must not be treated as a secret:\n{out}"
        );
    }

    /// The real pipeline: `report(&input, &ctx)`. An unpublished
    /// peer, a WIF and a URL carrying an unpublished IP inside a log line,
    /// and an engine warning string must all disappear, while the published
    /// peer, its `peer N` label and the report's section headings survive.
    #[test]
    fn report_hides_every_private_value_end_to_end() {
        let wif = crate::signer::generate_wif();
        let input = DiagnosticsInput {
            generated_at: "2026-09-29 14:05 UTC".into(),
            app_version: "0.7.0".into(),
            engine_pinned: "v0.34.9 (84b998b4)".into(),
            engine_running: Some("/BTX:0.34.9/".into()),
            platform: "macos aarch64, installed from a dmg".into(),
            role: "follows signatures".into(),
            signer_pubkey: None,
            status_line: "LIVE · Up to date".into(),
            window_lines: vec!["All caught up.".into()],
            phase: "ready".into(),
            chain: Some(BlockchainInfo {
                blocks: 233_480,
                headers: 233_481,
                warnings: vec!["Cadence burst hold".into()],
                ..Default::default()
            }),
            best_block_hash: Some("11bd18812b6afcd1".into()),
            chainstates: None,
            started_from: None,
            tips: vec![],
            held: vec![],
            peers: vec![
                PeerInfo {
                    id: 4,
                    addr: "109.199.124.187:19335".into(),
                    subver: "/BTX:0.34.11/".into(),
                    synced_headers: 233_481,
                    synced_blocks: 233_480,
                    servicesnames: vec!["NETWORK_LIMITED".into()],
                    connection_type: "manual".into(),
                    ..Default::default()
                },
                PeerInfo {
                    id: 7,
                    addr: "84.32.49.226:19335".into(),
                    inbound: true,
                    subver: "/BTX:0.34.9/".into(),
                    synced_headers: 200_000,
                    synced_blocks: 200_000,
                    connection_type: "inbound".into(),
                    ..Default::default()
                },
            ],
            attested_tip: None,
            stall: None,
            log_warnings: vec![format!(
                "[warning] signing key backup contains {wif}, update check via \
                 http://84.32.49.226:19335/status failed"
            )],
        };
        let context = RedactionContext {
            home: Some("/Users/alice".into()),
            secrets: vec![wif.clone()],
            published_hosts: crate::node::published_peer_hosts(),
        };
        let out = report(&input, &context);
        for gone in [wif.as_str(), "84.32.49.226"] {
            assert!(!out.contains(gone), "{gone} survived end to end:\n{out}");
        }
        for kept in [
            "easyNode diagnostics",
            "Chain",
            "Held branches",
            "Peers (1 in, 1 out)",
            "peer 4: 109.199.124.187:19335",
            "Engine notices",
            "Last warning lines of debug.log",
        ] {
            assert!(out.contains(kept), "{kept} missing end to end:\n{out}");
        }
    }

    /// A debug.log line is redacted first and cut to LOG_LINE_CHARS after,
    /// so the cut can never leave a piece of an address or a key that the
    /// redaction no longer recognises. An unpublished peer address and a
    /// WIF (once as the app's own known secret, once as a stranger's key
    /// only its shape gives away) are placed across character 240 at every
    /// offset that touches the cut, and go through the real pipeline:
    /// `warning_lines`, then `report`.
    #[test]
    fn a_log_line_cut_at_240_characters_keeps_no_piece_of_an_address_or_a_key() {
        let wif = crate::signer::generate_wif();
        let prefix = "2026-09-29T10:00:00Z [warning] ";
        let rest = "and the rest of the line, long enough that it is always cut";
        let mut leaks = Vec::new();
        let mut uncut = Vec::new();
        for known_secret in [true, false] {
            let context = RedactionContext {
                home: None,
                secrets: if known_secret {
                    vec![wif.clone()]
                } else {
                    vec![]
                },
                published_hosts: crate::node::published_peer_hosts(),
            };
            for (name, private, fragment) in [
                ("peer address", "84.32.49.226:19335", "84.32"),
                ("WIF", wif.as_str(), &wif[..8]),
            ] {
                for start in (LOG_LINE_CHARS - private.len())..=LOG_LINE_CHARS {
                    // Words, not one long run: a run of 50+ letters is itself
                    // key-shaped and would be removed, and the cut with it.
                    let filler: String = "log words "
                        .chars()
                        .cycle()
                        .take(start - prefix.len() - 1)
                        .collect();
                    let log = format!("{prefix}{filler} {private} {rest}\n");
                    let input = DiagnosticsInput {
                        log_warnings: warning_lines(&log),
                        ..Default::default()
                    };
                    let out = report(&input, &context);
                    let case = format!("{name} at {start} (known secret: {known_secret})");
                    if out.contains(fragment) {
                        leaks.push(case.clone());
                    }
                    let shown = out
                        .lines()
                        .find(|l| l.contains("[warning]"))
                        .expect("the log line is in the report");
                    if shown.trim_start().chars().count() != LOG_LINE_CHARS + 1
                        || !shown.ends_with('…')
                    {
                        uncut.push(case);
                    }
                }
            }
        }
        assert!(leaks.is_empty(), "a piece survived the cut: {leaks:#?}");
        assert!(uncut.is_empty(), "not cut at {LOG_LINE_CHARS}: {uncut:#?}");
    }

    /// Belt and braces for the cut: the `…` it appends is trailing
    /// punctuation, never part of the address it follows.
    #[test]
    fn a_trailing_ellipsis_does_not_hide_an_address() {
        let out = redact("peer 84.32.49.226:19335…", &ctx("unusedsecret"));
        assert!(!out.contains("84.32.49.226"), "IP survived:\n{out}");
        assert_eq!(out, "peer [peer address]…");
    }

    #[test]
    fn the_report_has_every_section_and_no_em_dash() {
        let input = DiagnosticsInput {
            generated_at: "2026-09-29 14:05 UTC".into(),
            app_version: "0.7.0".into(),
            engine_pinned: "v0.34.9 (84b998b4)".into(),
            engine_running: Some("/BTX:0.34.9/".into()),
            platform: "macos aarch64, installed from a dmg".into(),
            role: "follows signatures".into(),
            signer_pubkey: None,
            status_line: "LIVE · Up to date".into(),
            window_lines: vec!["All caught up.".into()],
            phase: "ready".into(),
            chain: Some(BlockchainInfo {
                blocks: 233480,
                headers: 233481,
                warnings: vec!["Cadence burst hold".into()],
                ..Default::default()
            }),
            best_block_hash: Some("11bd18812b6afcd1".into()),
            chainstates: None,
            started_from: None,
            tips: vec![],
            held: vec![HeldBranchState {
                height: 228146,
                root: "8240c62e62b47fc6".into(),
                state: "not on this node's chain".into(),
            }],
            peers: vec![PeerInfo {
                id: 4,
                addr: "109.199.124.187:19335".into(),
                subver: "/BTX:0.34.11/".into(),
                synced_headers: 233481,
                synced_blocks: 233480,
                servicesnames: vec!["NETWORK_LIMITED".into()],
                connection_type: "manual".into(),
                ..Default::default()
            }],
            attested_tip: None,
            stall: None,
            log_warnings: vec!["[warning] something".into()],
        };
        let r = render(&input);
        for part in [
            "easyNode diagnostics",
            "App 0.7.0",
            "Role: follows signatures",
            "Status: LIVE",
            "blocks 233,480",
            "Held branches",
            "228,146",
            "Peers (0 in, 1 out)",
            "peer 4: 109.199.124.187:19335",
            "recent history",
            "Engine notices (1)",
            "not shown on the home screen",
            "Last warning lines of debug.log (1)",
        ] {
            assert!(r.contains(part), "missing {part}:\n{r}");
        }
        assert!(!r.contains('\u{2014}'));
    }

    /// Integration review M2: the report says where the chain started and
    /// who confirmed it, as the status screen does, names intact after the
    /// redaction; and says nothing when there is no current start record.
    #[test]
    fn the_report_says_where_the_chain_started() {
        let base = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
        let input = DiagnosticsInput {
            chainstates: Some(ChainStates {
                headers: 233_900,
                chainstates: vec![crate::node_api::ChainstateEntry {
                    blocks: 233_900,
                    snapshot_blockhash: Some(base.into()),
                    ..Default::default()
                }],
            }),
            started_from: Some("Started from block 233,800, confirmed by Mende and jpp.".into()),
            ..Default::default()
        };
        let out = report(&input, &ctx("unusedsecret"));
        assert!(
            out.lines()
                .any(|l| l == "  Started from block 233,800, confirmed by Mende and jpp."),
            "{out}"
        );
        let none = render(&DiagnosticsInput::default());
        assert!(!none.contains("Started from"), "{none}");
    }
}
