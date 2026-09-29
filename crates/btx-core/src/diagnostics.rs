//! Copy diagnostics: one plain-text report a person can paste into a support
//! chat, and the redaction that keeps anything private out of it. Pure: the
//! Tauri side gathers the inputs and calls `redact(&render(..), ..)`.

use crate::fork::ChainTip;
use crate::node_api::{AttestedTip, BlockchainInfo, ChainStates, PeerInfo};

pub const LOG_TAIL_BYTES: u64 = 2 * 1024 * 1024;
pub const LOG_WARNING_LINES: usize = 20;
pub const LOG_LINE_CHARS: usize = 240;

/// The last warning or error lines of a log tail, oldest first.
pub fn warning_lines(log_tail: &str) -> Vec<String> {
    let mut lines: Vec<String> = log_tail
        .lines()
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            low.contains("[warning]") || low.contains("[error]") || low.contains("warning:")
                || low.contains("error:") || l.contains("Cadence burst hold")
        })
        .map(|l| cut(l.trim_end(), LOG_LINE_CHARS))
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
    o.push(format!("Signing key: {}", i.signer_pubkey.as_deref().unwrap_or("off")));
    o.push(String::new());
    o.push(format!("Status: {}", i.status_line));
    o.extend(i.window_lines.iter().cloned());
    o.push(format!("Phase: {}", i.phase));
    o.push(String::new());
    o.push("Chain".into());
    match &i.chain {
        Some(c) => o.push(format!(
            "  blocks {} · headers {} · progress {:.4} · first sync {}",
            group(c.blocks), group(c.headers), c.verification_progress,
            if c.initial_block_download { "yes" } else { "no" }
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
                    group(c.blocks), &base[..16.min(base.len())], if c.validated { "yes" } else { "not yet" }
                )),
                None => o.push(format!("  history check at {}", group(c.blocks))),
            }
        }
    }
    let others: Vec<&ChainTip> = i.tips.iter().filter(|t| t.status != "active" && t.branchlen > 1).collect();
    o.push(format!("Chain tips ({}), branches longer than one block:", i.tips.len()));
    for t in &others {
        o.push(format!("  {} {} length {} {}", group(t.height), &t.hash[..16.min(t.hash.len())], t.branchlen, t.status));
    }
    o.push("Held branches".into());
    for h in &i.held {
        o.push(format!("  {} {}: {}", group(h.height), h.root, h.state));
    }
    let inbound = i.peers.iter().filter(|p| p.inbound).count();
    o.push(format!("Peers ({} in, {} out)", inbound, i.peers.len() - inbound));
    for p in &i.peers {
        o.push(format!(
            "  peer {}: {} {} headers {} blocks {} · {} · {}",
            p.id, p.addr, p.subver, p.synced_headers, p.synced_blocks, history(p), p.connection_type
        ));
    }
    if let Some(a) = &i.attested_tip {
        o.push(format!(
            "Signed frontier: height {} · {} behind · on this chain: {}",
            a.height.map(group).unwrap_or_else(|| "unknown".into()),
            a.blocks_behind.map(|b| b.to_string()).unwrap_or_else(|| "unknown".into()),
            match a.on_active_chain { Some(true) => "yes", Some(false) => "no", None => "unknown" }
        ));
    }
    let notices = i.chain.as_ref().map(crate::engine_warnings::all_notices).unwrap_or_default();
    o.push(format!("Engine notices ({})", notices.len()));
    for n in &notices {
        let hidden = if n.hidden_because.is_some() { " (not shown on the home screen)" } else { "" };
        o.push(format!("  - {}{}", n.message, hidden));
        o.push(format!("    engine: {}", n.raw));
    }
    o.push(format!("Watchdog: {}", i.stall.as_deref().unwrap_or("nothing to report")));
    o.push(format!("Last warning lines of debug.log ({})", i.log_warnings.len()));
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

pub fn redact(text: &str, ctx: &RedactionContext) -> String {
    let mut out = text.to_string();
    for s in ctx.secrets.iter().map(|s| s.trim()).filter(|s| s.len() >= 8) {
        out = out.replace(s, "[removed]");
    }
    if let Some(home) = ctx.home.as_deref().filter(|h| h.len() > 1) {
        out = out.replace(home, "~");
    }
    out.lines().map(|l| redact_line(l, ctx)).collect::<Vec<_>>().join("\n")
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '=' | '"' | '\'' | '<' | '>' | '{' | '}')
}

fn redact_line(line: &str, ctx: &RedactionContext) -> String {
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

fn redact_token(tok: &str, ctx: &RedactionContext) -> String {
    let core = tok.trim_end_matches('.');
    let tail = &tok[core.len()..];
    if core.is_empty() {
        return tok.to_string();
    }
    if let Some(host) = address_host(core) {
        if ctx.published_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return tok.to_string();
        }
        return format!("[peer address]{tail}");
    }
    if looks_like_key(core) {
        return format!("[key removed]{tail}");
    }
    if looks_like_address(core) {
        return format!("[address removed]{tail}");
    }
    tok.to_string()
}

fn address_host(t: &str) -> Option<String> {
    use std::net::{IpAddr, SocketAddr};
    if let Ok(sa) = t.parse::<SocketAddr>() {
        return Some(sa.ip().to_string());
    }
    if let Ok(ip) = t.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    let host = match t.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => t,
    };
    let low = host.to_ascii_lowercase();
    (low.ends_with(".onion") || low.ends_with(".i2p")).then_some(low)
}

const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// WIF, extended keys and the like: long base58, not plain hex (hashes and
/// public keys are hex and stay).
fn looks_like_key(t: &str) -> bool {
    t.len() >= 50 && t.chars().all(|c| BASE58.contains(c)) && !t.chars().all(|c| c.is_ascii_hexdigit())
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

    #[test]
    fn warning_lines_keep_the_last_twenty_warnings_cut_to_length() {
        let mut log = String::new();
        for i in 0..30 {
            log.push_str(&format!("2026-09-29T10:00:{i:02}Z [warning] thing {i}\n"));
            log.push_str("2026-09-29T10:00:00Z ordinary line\n");
        }
        log.push_str(&format!("2026-09-29T10:01:00Z [error] {}\n", "x".repeat(400)));
        let lines = warning_lines(&log);
        assert_eq!(lines.len(), 20);
        assert!(lines[0].contains("thing 11"));
        assert!(lines[19].chars().count() <= 241);
        assert!(lines.iter().all(|l| !l.contains("ordinary")));
        assert!(warning_lines("Cadence burst hold at 82000\n")[0].contains("Cadence"));
    }

    #[test]
    fn redaction_removes_every_item_on_the_never_list() {
        let wif = crate::signer::generate_wif();
        let shaped: String = "P".chars().chain("abcdefghijkmnopqrstuvwxyz123456789".chars().cycle().take(51)).collect();
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
        for gone in ["cookiepassword123", wif.as_str(), shaped.as_str(), "/Users/alice", "84.32.49.226", "2001:db8::7", "abcdefghijklmnop.onion", "btx1qxyz"] {
            assert!(!out.contains(gone), "{gone} survived:\n{out}");
        }
        for kept in ["~/.easybtx/debug.log", "109.199.124.187:19335", "20.86.181.203:19338", "node.btx.dev:19335",
                     "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c",
                     "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675", "peer 12"] {
            assert!(out.contains(kept), "{kept} was removed:\n{out}");
        }
        assert!(out.contains("[peer address]") && out.contains("[key removed]") && out.contains("[address removed]"));
    }

    #[test]
    fn version_numbers_and_times_are_not_mistaken_for_addresses() {
        let out = redact("engine v0.34.9 at 2026-09-29T10:11:20Z height 233,470", &ctx("unusedsecret"));
        assert_eq!(out, "engine v0.34.9 at 2026-09-29T10:11:20Z height 233,470");
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
            chain: Some(BlockchainInfo { blocks: 233480, headers: 233481, warnings: vec!["Cadence burst hold".into()], ..Default::default() }),
            best_block_hash: Some("11bd18812b6afcd1".into()),
            chainstates: None,
            tips: vec![],
            held: vec![HeldBranchState { height: 228146, root: "8240c62e62b47fc6".into(), state: "not on this node's chain".into() }],
            peers: vec![PeerInfo { id: 4, addr: "109.199.124.187:19335".into(), subver: "/BTX:0.34.11/".into(), synced_headers: 233481, synced_blocks: 233480, servicesnames: vec!["NETWORK_LIMITED".into()], connection_type: "manual".into(), ..Default::default() }],
            attested_tip: None,
            stall: None,
            log_warnings: vec!["[warning] something".into()],
        };
        let r = render(&input);
        for part in ["easyNode diagnostics", "App 0.7.0", "Role: follows signatures", "Status: LIVE", "blocks 233,480", "Held branches", "228,146", "Peers (0 in, 1 out)", "peer 4: 109.199.124.187:19335", "recent history", "Engine notices (1)", "not shown on the home screen", "Last warning lines of debug.log (1)"] {
            assert!(r.contains(part), "missing {part}:\n{r}");
        }
        assert!(!r.contains('\u{2014}'));
    }
}
