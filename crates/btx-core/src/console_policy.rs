//! What the Tools command window may run, decided in one place.
//!
//! The window sends the line the person typed and this module decides. A
//! command runs only if it is in [`ALLOWED`]; everything else is refused. The
//! named refusals in [`REFUSALS`] exist only to say why in better words: the
//! safety is the default. See docs/decisions/2026-09-29-tools-and-command-window.md.

use serde_json::{json, Value};

/// The engine's command list, captured from `btx-cli help` on v0.34.9. Read
/// for the refusal sentence only; nothing is allowed because it is listed here.
pub const ENGINE_HELP_V0_34_9: &str = include_str!("console_policy/engine-help-v0.34.9.txt");

/// One RPC the node will be asked to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: String,
    pub params: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Read-only: run it.
    Run(Call),
    /// Changes something for a moment: run it only after a second click.
    Confirm { call: Call, sentence: String },
    /// Answered by the app itself (`help` on its own).
    Local(String),
    /// Not in this window, and why.
    Refuse(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Arg {
    Hash,
    Int(i64, i64),
    Bool,
    IpLiteral,
    Exactly(&'static str),
    CommandName,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    Free,
    Confirm,
}

struct Shape {
    name: &'static str,
    required: &'static [(Arg, &'static str)],
    optional: &'static [(Arg, &'static str)],
    class: Class,
}

const fn free(name: &'static str) -> Shape {
    Shape {
        name,
        required: &[],
        optional: &[],
        class: Class::Free,
    }
}

const ALLOWED: &[Shape] = &[
    free("getblockchaininfo"),
    free("getchaintips"),
    free("getpeerinfo"),
    free("getnetworkinfo"),
    free("getmempoolinfo"),
    free("getmatmulattestedtip"),
    free("getmatmultrustedstatus"),
    free("uptime"),
    free("getchainstates"),
    free("getblockcount"),
    free("getbestblockhash"),
    free("getconnectioncount"),
    Shape {
        name: "getblockhash",
        required: &[(Arg::Int(0, i32::MAX as i64), "height")],
        optional: &[],
        class: Class::Free,
    },
    Shape {
        name: "getblockheader",
        required: &[(Arg::Hash, "blockhash")],
        optional: &[(Arg::Bool, "verbose")],
        class: Class::Free,
    },
    Shape {
        name: "getblock",
        required: &[(Arg::Hash, "blockhash")],
        optional: &[(Arg::Int(0, 3), "verbosity")],
        class: Class::Free,
    },
    Shape {
        name: "getrawtransaction",
        required: &[(Arg::Hash, "txid")],
        optional: &[(Arg::Int(0, 2), "verbosity"), (Arg::Hash, "blockhash")],
        class: Class::Free,
    },
    Shape {
        name: "gettxout",
        required: &[(Arg::Hash, "txid"), (Arg::Int(0, u32::MAX as i64), "n")],
        optional: &[(Arg::Bool, "include_mempool")],
        class: Class::Free,
    },
    Shape {
        name: "help",
        required: &[],
        optional: &[(Arg::CommandName, "command")],
        class: Class::Free,
    },
    Shape {
        name: "addnode",
        required: &[
            (Arg::IpLiteral, "ip[:port]"),
            (Arg::Exactly("onetry"), "onetry"),
        ],
        optional: &[],
        class: Class::Confirm,
    },
    Shape {
        name: "getblockfrompeer",
        required: &[(Arg::Hash, "blockhash"), (Arg::Int(0, i64::MAX), "peer_id")],
        optional: &[],
        class: Class::Confirm,
    },
];

const PEERS: &str = "The app manages this node's peers.";
const FILES: &str = "This window does not read or write files.";
const WALLET: &str = "Wallet keys and payments stay in the Wallet panel.";

pub(crate) const REFUSALS: &[(&[&str], &str)] = &[
    (&["stop"], "Use Stop node, which shuts down in the right order."),
    (
        &["invalidateblock", "reconsiderblock", "preciousblock"],
        "The app decides which branches this node holds. Changing that by hand fights the app and can strand the node.",
    ),
    (&["setban", "clearbanned", "disconnectnode", "setnetworkactive"], PEERS),
    (&["pruneblockchain", "setprunelock"], "Pruning is set in Settings."),
    (
        &[
            "loadtxoutset", "loadtxoutsetattested", "dumptxoutset", "dumptxoutsetattested",
            "signutxosnapshotmanifest", "offerattestedutxosnapshot",
            "withdrawattestedutxosnapshot", "fetchattestedutxosnapshot",
        ],
        "Snapshots go through Fast-forward and Serve a chain snapshot, which check them first.",
    ),
    (
        &["addmatmulattestationblocklist", "clearmintedattestation", "submitmatmulattestations", "submitmatmulrefutation"],
        "This changes which signatures the node trusts. That only changes with an app update.",
    ),
    (&["savemempool", "importmempool", "savefeeestimates", "importwallet", "restorewallet"], FILES),
];

/// `"run"` or `"confirm"` for a command this window can run, else `None`.
pub fn class_of(name: &str) -> Option<&'static str> {
    ALLOWED
        .iter()
        .find(|s| s.name == name)
        .map(|s| match s.class {
            Class::Free => "run",
            Class::Confirm => "confirm",
        })
}

pub fn decide(line: &str) -> Decision {
    let tokens = match tokenize(line) {
        Ok(t) => t,
        Err(e) => return Decision::Refuse(e),
    };
    let Some((first, args)) = tokens.split_first() else {
        return Decision::Refuse("Type a command, or help to see what this window runs.".into());
    };
    let name = first.to_lowercase();
    if !is_command_name(&name) {
        return Decision::Refuse(format!(
            "\"{}\" is not a command this window knows. Type help to see them.",
            first.chars().take(40).collect::<String>()
        ));
    }
    if name == "help" && args.is_empty() {
        return Decision::Local(help_text());
    }
    if let Some(shape) = ALLOWED.iter().find(|s| s.name == name) {
        if name == "addnode" && args.len() >= 2 && !args[1].eq_ignore_ascii_case("onetry") {
            return Decision::Refuse(PEERS.into());
        }
        return match parse_args(shape, args) {
            Ok(params) => {
                let call = Call {
                    method: name,
                    params,
                };
                match shape.class {
                    Class::Free => Decision::Run(call),
                    Class::Confirm => {
                        let sentence = confirm_sentence(&call);
                        Decision::Confirm { call, sentence }
                    }
                }
            }
            Err(usage) => Decision::Refuse(usage),
        };
    }
    Decision::Refuse(refusal_for(&name))
}

fn is_command_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut started = false;
    for c in line.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if in_quotes {
        return Err("A quote is not closed.".into());
    }
    if started {
        out.push(cur);
    }
    Ok(out)
}

fn usage(shape: &Shape) -> String {
    let mut s = format!("Use: {}", shape.name);
    for (kind, label) in shape.required {
        match kind {
            Arg::Exactly(word) => s.push_str(&format!(" {word}")),
            _ => s.push_str(&format!(" <{label}>")),
        }
    }
    for (_, label) in shape.optional {
        s.push_str(&format!(" [{label}]"));
    }
    s
}

fn parse_args(shape: &Shape, args: &[String]) -> Result<Vec<Value>, String> {
    let max = shape.required.len() + shape.optional.len();
    if args.len() < shape.required.len() || args.len() > max {
        return Err(usage(shape));
    }
    args.iter()
        .zip(shape.required.iter().chain(shape.optional.iter()))
        .map(|(a, (kind, _))| parse_one(*kind, a).ok_or_else(|| usage(shape)))
        .collect()
}

fn parse_one(kind: Arg, a: &str) -> Option<Value> {
    match kind {
        Arg::Hash => (a.len() == 64 && a.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| json!(a.to_ascii_lowercase())),
        Arg::Int(lo, hi) => a
            .parse::<i64>()
            .ok()
            .filter(|n| (lo..=hi).contains(n))
            .map(|n| json!(n)),
        Arg::Bool => match a.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(json!(true)),
            "false" | "0" => Some(json!(false)),
            _ => None,
        },
        Arg::IpLiteral => {
            let ok =
                a.parse::<std::net::SocketAddr>().is_ok() || a.parse::<std::net::IpAddr>().is_ok();
            ok.then(|| json!(a))
        }
        Arg::Exactly(word) => a.eq_ignore_ascii_case(word).then(|| json!(word)),
        Arg::CommandName => {
            let n = a.to_lowercase();
            is_command_name(&n).then(|| json!(n))
        }
    }
}

fn confirm_sentence(call: &Call) -> String {
    match call.method.as_str() {
        "addnode" => format!(
            "Try one connection to {}? Nothing is saved; the node forgets it at its next restart.",
            call.params[0].as_str().unwrap_or("")
        ),
        "getblockfrompeer" => {
            let hash = call.params[0].as_str().unwrap_or("");
            format!(
                "Ask peer {} for block {}...?",
                call.params[1],
                &hash[..16.min(hash.len())]
            )
        }
        other => format!("Run {other}?"),
    }
}

fn help_text() -> String {
    let mut runs: Vec<String> = Vec::new();
    let mut asks: Vec<String> = Vec::new();
    for shape in ALLOWED {
        let line = usage(shape).trim_start_matches("Use: ").to_string();
        match shape.class {
            Class::Free => runs.push(line),
            Class::Confirm => asks.push(line),
        }
    }
    runs.sort();
    asks.sort();
    format!(
        "This window runs these commands:\n  {}\n\nThese ask you first:\n  {}\n\nhelp <command> shows the engine's own help for any command.",
        runs.join("\n  "),
        asks.join("\n  ")
    )
}

fn refusal_for(name: &str) -> String {
    if let Some((_, why)) = REFUSALS.iter().find(|(names, _)| names.contains(&name)) {
        return (*why).to_string();
    }
    if name == "addnode" {
        return PEERS.into();
    }
    if name.starts_with("z_")
        || name.starts_with("bridge_")
        || engine_category(name) == Some("Wallet")
    {
        return WALLET.into();
    }
    format!("This window runs a short list of read-only commands. {name} isn't one of them. Type help to see them.")
}

/// The `== Category ==` a command is listed under in the captured help.
fn engine_category(name: &str) -> Option<&'static str> {
    let mut category = None;
    for line in ENGINE_HELP_V0_34_9.lines() {
        if let Some(c) = line.strip_prefix("== ").and_then(|l| l.strip_suffix(" ==")) {
            category = Some(c);
        } else if line.split_whitespace().next() == Some(name) {
            return category;
        }
    }
    None
}

/// How long a confirm click stays valid.
pub const CONFIRM_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The one pending confirm-class call. The window gets only the token; the
/// second click sends it back, and the call it runs is the one kept here, so
/// no bug in the window can turn a confirm-class call into a free one.
#[derive(Debug, Default)]
pub struct ConfirmBook {
    pending: Option<(String, Call, std::time::Instant)>,
}

impl ConfirmBook {
    pub const fn new() -> Self {
        Self { pending: None }
    }

    /// Keep `call` behind `token`. A newer call replaces an older one.
    pub fn issue(&mut self, token: String, call: Call, now: std::time::Instant) {
        self.pending = Some((token, call, now));
    }

    /// The call behind `token`, once, while it is fresh. Any redeem empties
    /// the book, so a wrong or stale token cancels the pending call.
    pub fn redeem(&mut self, token: &str, now: std::time::Instant) -> Option<Call> {
        let (t, call, at) = self.pending.take()?;
        (t == token && now.saturating_duration_since(at) <= CONFIRM_TTL).then_some(call)
    }
}

/// 16 random bytes as lowercase hex.
pub fn new_token() -> String {
    use rand_core::{OsRng, RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const H: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";

    /// Real engine commands that `help` does not list (Bitcoin Core's hidden
    /// category). The app itself calls `invalidateblock` to hold branches.
    const HIDDEN_ENGINE_COMMANDS: &[&str] = &["invalidateblock", "reconsiderblock"];

    fn run(line: &str) -> Call {
        match decide(line) {
            Decision::Run(c) => c,
            other => panic!("{line}: expected Run, got {other:?}"),
        }
    }
    fn refused(line: &str) -> String {
        match decide(line) {
            Decision::Refuse(s) => s,
            other => panic!("{line}: expected Refuse, got {other:?}"),
        }
    }

    #[test]
    fn every_read_only_command_runs() {
        for name in [
            "getblockchaininfo",
            "getchaintips",
            "getpeerinfo",
            "getnetworkinfo",
            "getmempoolinfo",
            "getmatmulattestedtip",
            "getmatmultrustedstatus",
            "uptime",
            "getchainstates",
            "getblockcount",
            "getbestblockhash",
            "getconnectioncount",
        ] {
            assert_eq!(
                run(name),
                Call {
                    method: name.into(),
                    params: vec![]
                }
            );
        }
        assert_eq!(run("getblockhash 228146").params, vec![json!(228146)]);
        assert_eq!(run(&format!("getblockheader {H}")).params, vec![json!(H)]);
        assert_eq!(
            run(&format!("getblockheader {H} false")).params,
            vec![json!(H), json!(false)]
        );
        assert_eq!(
            run(&format!("getblock {H} 2")).params,
            vec![json!(H), json!(2)]
        );
        assert_eq!(
            run(&format!("getrawtransaction {H} 1 {H}")).params,
            vec![json!(H), json!(1), json!(H)]
        );
        assert_eq!(
            run(&format!("gettxout {H} 0 true")).params,
            vec![json!(H), json!(0), json!(true)]
        );
        assert_eq!(run("help stop").params, vec![json!("stop")]);
    }

    #[test]
    fn help_alone_is_answered_by_the_app() {
        match decide("help") {
            Decision::Local(text) => {
                assert!(text.contains("getblock <blockhash> [verbosity]"));
                assert!(text.contains("addnode <ip[:port]> onetry"));
                assert!(!text.contains('\u{2014}'), "no em-dashes in copy");
            }
            other => panic!("expected Local, got {other:?}"),
        }
    }

    #[test]
    fn the_two_actions_ask_first_and_never_run_directly() {
        match decide("addnode 1.2.3.4:19335 onetry") {
            Decision::Confirm { call, sentence } => {
                assert_eq!(
                    call,
                    Call {
                        method: "addnode".into(),
                        params: vec![json!("1.2.3.4:19335"), json!("onetry")]
                    }
                );
                assert!(sentence.contains("1.2.3.4:19335"));
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
        match decide(&format!("getblockfrompeer {H} 7")) {
            Decision::Confirm { call, sentence } => {
                assert_eq!(call.params, vec![json!(H), json!(7)]);
                assert!(sentence.contains("peer 7") && sentence.contains("8240c62e62b47fc6"));
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
        assert!(matches!(
            decide("addnode [2001:db8::1]:19335 onetry"),
            Decision::Confirm { .. }
        ));
    }

    #[test]
    fn every_named_refusal_says_why() {
        let cases: &[(&str, &str)] = &[
            ("stop", "Stop node"),
            (&format!("invalidateblock {H}"), "which branches"),
            (&format!("reconsiderblock {H}"), "which branches"),
            (&format!("preciousblock {H}"), "which branches"),
            ("setban 1.2.3.4 add", "peers"),
            ("clearbanned", "peers"),
            ("disconnectnode 1.2.3.4", "peers"),
            ("setnetworkactive false", "peers"),
            ("pruneblockchain 1000", "Settings"),
            ("setprunelock", "Settings"),
            ("loadtxoutset a", "Snapshots"),
            ("loadtxoutsetattested a b", "Snapshots"),
            ("dumptxoutset a", "Snapshots"),
            ("dumptxoutsetattested a b", "Snapshots"),
            ("signutxosnapshotmanifest a", "Snapshots"),
            ("offerattestedutxosnapshot a b", "Snapshots"),
            ("withdrawattestedutxosnapshot", "Snapshots"),
            ("fetchattestedutxosnapshot", "Snapshots"),
            (
                "addmatmulattestationblocklist 02aa",
                "signatures the node trusts",
            ),
            ("clearmintedattestation", "signatures the node trusts"),
            ("submitmatmulattestations x", "signatures the node trusts"),
            ("submitmatmulrefutation x", "signatures the node trusts"),
            ("dumpprivkey x", "Wallet panel"),
            ("dumpwallet x", "Wallet panel"),
            ("backupwallet x", "Wallet panel"),
            ("exportpqkey x", "Wallet panel"),
            ("dumpmasterprivkey", "Wallet panel"),
            ("sendtoaddress x 1", "Wallet panel"),
            ("send x", "Wallet panel"),
            ("sendall x", "Wallet panel"),
            ("z_sendmany x", "Wallet panel"),
            ("bridge_listarchive", "Wallet panel"),
            ("savemempool", "read or write files"),
            ("importmempool x", "read or write files"),
            ("savefeeestimates", "read or write files"),
            ("importwallet x", "read or write files"),
            ("restorewallet x y", "read or write files"),
        ];
        for (line, why) in cases {
            let s = refused(line);
            assert!(s.contains(why), "{line}: {s}");
        }
    }

    #[test]
    fn anything_else_is_refused_by_default() {
        let s = refused("gettxoutsetinfo");
        assert!(s.contains("gettxoutsetinfo isn't one of them"), "{s}");
        assert!(refused("somethingnew").contains("Type help"));
    }

    #[test]
    fn sabotage_cases_never_run() {
        assert!(refused(&format!("InvalidateBlock {H}")).contains("which branches"));
        assert!(refused("STOP").contains("Stop node"));
        assert!(refused("st\u{200b}op").contains("not a command this window knows"));
        assert!(refused("stop;getblockcount").contains("not a command this window knows"));
        assert!(refused(&format!("getblock {H} 2 extra")).contains("Use: getblock"));
        assert!(refused("addnode 1.2.3.4 add").contains("peers"));
        assert!(refused("addnode evil.example onetry").contains("Use: addnode"));
        assert!(refused("addnode 1.2.3.4 onetry true archive").contains("Use: addnode"));
        assert!(refused("getblockhash -1").contains("Use: getblockhash"));
        assert!(refused(r#"getblockhash {"height":1}"#).contains("Use: getblockhash"));
        assert!(refused(&format!("getblock {H} 4")).contains("Use: getblock"));
        assert!(refused("getblock nothex").contains("Use: getblock"));
        assert!(refused("getblockcount \"unclosed").contains("quote"));
        assert!(refused("").contains("Type a command"));
        assert!(refused("   ").contains("Type a command"));
    }

    #[test]
    fn quotes_hold_an_argument_together() {
        assert_eq!(run("help \"getblock\"").params, vec![json!("getblock")]);
    }

    #[test]
    fn the_fixture_is_the_v0_34_9_command_list() {
        let names: Vec<&str> = ENGINE_HELP_V0_34_9
            .lines()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert_eq!(names.len(), 299);
        // Only these come out as anything but a refusal. An engine bump that
        // adds a command changes the fixture, and this list shows the change.
        let mut allowed: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| class_of(n).is_some())
            .collect();
        allowed.sort_unstable();
        assert_eq!(
            allowed,
            vec![
                "addnode",
                "getbestblockhash",
                "getblock",
                "getblockchaininfo",
                "getblockcount",
                "getblockfrompeer",
                "getblockhash",
                "getblockheader",
                "getchainstates",
                "getchaintips",
                "getconnectioncount",
                "getmatmulattestedtip",
                "getmatmultrustedstatus",
                "getmempoolinfo",
                "getnetworkinfo",
                "getpeerinfo",
                "getrawtransaction",
                "gettxout",
                "help",
                "uptime",
            ]
        );
        // Every named refusal is a real command, so a typo cannot hide one.
        for (group, _) in REFUSALS {
            for n in *group {
                assert!(
                    names.contains(n) || HIDDEN_ENGINE_COMMANDS.contains(n),
                    "{n} is not an engine command"
                );
            }
        }
    }

    fn a_call() -> Call {
        Call {
            method: "addnode".into(),
            params: vec![json!("1.2.3.4"), json!("onetry")],
        }
    }

    #[test]
    fn a_token_redeems_once_and_returns_the_stored_call_unchanged() {
        let t0 = std::time::Instant::now();
        let mut book = ConfirmBook::new();
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(
            book.redeem("abc", t0 + std::time::Duration::from_secs(5)),
            Some(a_call())
        );
        assert_eq!(
            book.redeem("abc", t0 + std::time::Duration::from_secs(6)),
            None,
            "used twice"
        );
    }

    #[test]
    fn an_expired_or_wrong_token_is_refused_and_clears_the_book() {
        let t0 = std::time::Instant::now();
        let mut book = ConfirmBook::new();
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(
            book.redeem(
                "abc",
                t0 + CONFIRM_TTL + std::time::Duration::from_millis(1)
            ),
            None
        );
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(book.redeem("xyz", t0), None);
        assert_eq!(
            book.redeem("abc", t0),
            None,
            "a wrong token cancels the pending call"
        );
    }

    #[test]
    fn a_newer_confirm_replaces_the_older_one() {
        let t0 = std::time::Instant::now();
        let mut book = ConfirmBook::new();
        book.issue("one".into(), a_call(), t0);
        book.issue("two".into(), a_call(), t0);
        assert_eq!(book.redeem("one", t0), None);
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
