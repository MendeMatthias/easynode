# Tools and the Command Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One wrench button opens a Tools overlay with four quick actions, Copy diagnostics and a command window whose allowed commands are decided by one pure Rust function.

**Architecture:** Every decision is a pure function in `crates/btx-core` with its own tests: the command policy and confirm token (`console_policy.rs`), the report and its redaction (`diagnostics.rs`), the stuck-block planner (`stuck_blocks.rs`), and the engine notices (`engine_warnings.rs`). A thin Tauri module (`apps/node/src-tauri/src/tools.rs`) gathers RPC answers and calls them. The window (`apps/node/src/tools.ts`) only renders text and sends the typed line; it never names a method the backend trusts.

**Tech Stack:** Rust (tokio, serde_json, rand_core already in btx-core), Tauri 2 commands, TypeScript with vitest, plain HTML/CSS.

**Spec:** `docs/decisions/2026-09-29-tools-and-command-window.md` (approved 2026-09-29).

**Out of scope here:** the Fast-forward button (spec section 3). It needs the loading path from `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` and is built in that plan, in the overlay this plan creates.

## Global Constraints

- The window cannot reach the node; every action is a Rust command. No command passes an arbitrary RPC through.
- A command is refused unless it is in the allowed or confirm rows of the spec's table. Named refusals only improve the sentence.
- Confirm-class calls run only through a one-time token that lives 30 seconds.
- Output reaches the page only through `textContent`, never `innerHTML`.
- Never in any output: the RPC cookie, the signing key file, anything WIF-shaped, wallet contents, peer IPs other than the app's published peers, the home folder path.
- User-facing copy: friendly, simple, no hype, no guarantees, no em-dashes.
- The window stays 560x780; the status screen stays home; no new top-level screens.
- Command history: last 50, memory only.
- Diagnostics log excerpt: last 20 warning or error lines from the last 2 MB of `debug.log`, each cut at 240 characters.
- Fetch a stuck block: at most 16 blocks; one click; never adds, bans or disconnects peers.

## File structure

| File | Responsibility |
|---|---|
| `crates/btx-core/src/console_policy.rs` (create) | Tokenise a typed line, decide Run / Confirm / Local / Refuse, the confirm token book |
| `crates/btx-core/src/console_policy/engine-help-v0.34.9.txt` (create) | The engine's 299 commands, captured from v0.34.9 `help` |
| `crates/btx-core/src/diagnostics.rs` (create) | Warning lines from a log tail, the report text, redaction |
| `crates/btx-core/src/stuck_blocks.rs` (create) | Which blocks to ask for and from whom |
| `crates/btx-core/src/engine_warnings.rs` (modify) | `all_notices`: every warning, hidden ones included, with the reason |
| `crates/btx-core/src/node_api.rs` (modify) | `PeerInfo` gains `synced_headers`, `synced_blocks` |
| `crates/btx-core/src/node.rs` (modify) | `debug_log_tail`, `published_peer_hosts` |
| `crates/btx-core/src/lib.rs` (modify) | Declare the three new modules |
| `crates/btx-core/tests/console_regtest.rs` (create) | Opt-in: every table row through a real regtest btxd |
| `apps/node/src-tauri/src/tools.rs` (create) | Tauri commands for Tools |
| `apps/node/src-tauri/src/commands.rs` (modify) | `node_ownership` becomes `pub(crate)` |
| `apps/node/src-tauri/src/lib.rs` (modify) | `mod tools;` and registration |
| `apps/node/index.html` (modify) | The wrench button and the Tools overlay |
| `apps/node/src/tools.ts` (create) | The overlay's behaviour |
| `apps/node/src/tools-history.ts` (create) | Pure helpers: history ring, display cap |
| `apps/node/src/tools-history.test.ts` (create) | Their tests |
| `apps/node/src/main.ts` (modify) | Call `initTools()` |
| `apps/node/src/styles.css` (modify) | A few `.tools-*` rules |
| `apps/node/CHANGELOG.md` (modify) | The Unreleased entry |

Commands used throughout (run from the worktree root):

- Core tests: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib <filter>`
- App crate: `cargo test --manifest-path apps/node/src-tauri/Cargo.toml`
- UI: `npm --prefix apps/node ci` once, then `npm --prefix apps/node test` and `npm --prefix apps/node run build`

---

### Task 1: The command policy

**Files:**
- Create: `crates/btx-core/src/console_policy.rs`
- Create: `crates/btx-core/src/console_policy/engine-help-v0.34.9.txt`
- Modify: `crates/btx-core/src/lib.rs` (add `pub mod console_policy;` in alphabetical order, after `pub mod checkin;`)

**Interfaces:**
- Produces: `pub struct Call { pub method: String, pub params: Vec<serde_json::Value> }`, `pub enum Decision { Run(Call), Confirm { call: Call, sentence: String }, Local(String), Refuse(String) }`, `pub fn decide(line: &str) -> Decision`, `pub fn class_of(name: &str) -> Option<&'static str>` (returns `"run"` or `"confirm"`), `pub const ENGINE_HELP_V0_34_9: &str`.

- [ ] **Step 1: Capture the fixture**

The file is the output of `btx-cli help` against a v0.34.9 btxd (299 commands under `== Category ==` headings). Capture it from the staged engine:

```bash
S=$(mktemp -d)
BIN=apps/node/src-tauri/resources/node-pkg/bin
[ -x "$BIN/btxd" ] || BIN=/Users/m2promende/repos/easynode/apps/node/src-tauri/resources/node-pkg/bin
"$BIN/btxd" -regtest -datadir="$S" -daemon -server -rpcuser=u -rpcpassword=p -rpcport=29601 -port=29602 -listen=0 -connect=0 -dnsseed=0
sleep 5
mkdir -p crates/btx-core/src/console_policy
"$BIN/btx-cli" -regtest -datadir="$S" -rpcport=29601 -rpcuser=u -rpcpassword=p help > crates/btx-core/src/console_policy/engine-help-v0.34.9.txt
"$BIN/btx-cli" -regtest -datadir="$S" -rpcport=29601 -rpcuser=u -rpcpassword=p stop
grep -c '^[a-z]' crates/btx-core/src/console_policy/engine-help-v0.34.9.txt
```

Expected last line: `299`.

- [ ] **Step 2: Write the failing tests**

Create `crates/btx-core/src/console_policy.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const H: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";

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
            "getblockchaininfo", "getchaintips", "getpeerinfo", "getnetworkinfo",
            "getmempoolinfo", "getmatmulattestedtip", "getmatmultrustedstatus", "uptime",
            "getchainstates", "getblockcount", "getbestblockhash", "getconnectioncount",
        ] {
            assert_eq!(run(name), Call { method: name.into(), params: vec![] });
        }
        assert_eq!(run("getblockhash 228146").params, vec![json!(228146)]);
        assert_eq!(run(&format!("getblockheader {H}")).params, vec![json!(H)]);
        assert_eq!(run(&format!("getblockheader {H} false")).params, vec![json!(H), json!(false)]);
        assert_eq!(run(&format!("getblock {H} 2")).params, vec![json!(H), json!(2)]);
        assert_eq!(run(&format!("getrawtransaction {H} 1 {H}")).params, vec![json!(H), json!(1), json!(H)]);
        assert_eq!(run(&format!("gettxout {H} 0 true")).params, vec![json!(H), json!(0), json!(true)]);
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
                assert_eq!(call, Call { method: "addnode".into(), params: vec![json!("1.2.3.4:19335"), json!("onetry")] });
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
        assert!(matches!(decide("addnode [2001:db8::1]:19335 onetry"), Decision::Confirm { .. }));
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
            ("addmatmulattestationblocklist 02aa", "signatures the node trusts"),
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
        let mut allowed: Vec<&str> = names.iter().copied().filter(|n| class_of(n).is_some()).collect();
        allowed.sort_unstable();
        assert_eq!(allowed, vec![
            "addnode", "getbestblockhash", "getblock", "getblockchaininfo", "getblockcount",
            "getblockfrompeer", "getblockhash", "getblockheader", "getchainstates", "getchaintips",
            "getconnectioncount", "getmatmulattestedtip", "getmatmultrustedstatus", "getmempoolinfo",
            "getnetworkinfo", "getpeerinfo", "getrawtransaction", "gettxout", "help", "uptime",
        ]);
        // Every named refusal is a real command, so a typo cannot hide one.
        for (group, _) in REFUSALS {
            for n in *group {
                assert!(names.contains(n) || HIDDEN_ENGINE_COMMANDS.contains(n), "{n} is not an engine command");
            }
        }
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib console_policy`
Expected: compile errors (`cannot find function decide`, `Call`, `Decision`, `class_of`, `REFUSALS`, `ENGINE_HELP_V0_34_9`).

- [ ] **Step 4: Write the implementation above the test module**

```rust
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
    Shape { name, required: &[], optional: &[], class: Class::Free }
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
    Shape { name: "getblockhash", required: &[(Arg::Int(0, i32::MAX as i64), "height")], optional: &[], class: Class::Free },
    Shape { name: "getblockheader", required: &[(Arg::Hash, "blockhash")], optional: &[(Arg::Bool, "verbose")], class: Class::Free },
    Shape { name: "getblock", required: &[(Arg::Hash, "blockhash")], optional: &[(Arg::Int(0, 3), "verbosity")], class: Class::Free },
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
    Shape { name: "help", required: &[], optional: &[(Arg::CommandName, "command")], class: Class::Free },
    Shape {
        name: "addnode",
        required: &[(Arg::IpLiteral, "ip[:port]"), (Arg::Exactly("onetry"), "onetry")],
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

/// Real engine commands that `help` does not list (Bitcoin Core's hidden
/// category). The app itself calls `invalidateblock` to hold branches.
pub(crate) const HIDDEN_ENGINE_COMMANDS: &[&str] = &["invalidateblock", "reconsiderblock"];

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
    ALLOWED.iter().find(|s| s.name == name).map(|s| match s.class {
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
                let call = Call { method: name, params };
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
        Arg::Int(lo, hi) => a.parse::<i64>().ok().filter(|n| (lo..=hi).contains(n)).map(|n| json!(n)),
        Arg::Bool => match a.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(json!(true)),
            "false" | "0" => Some(json!(false)),
            _ => None,
        },
        Arg::IpLiteral => {
            let ok = a.parse::<std::net::SocketAddr>().is_ok() || a.parse::<std::net::IpAddr>().is_ok();
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
            format!("Ask peer {} for block {}...?", call.params[1], &hash[..16.min(hash.len())])
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
    if name.starts_with("z_") || name.starts_with("bridge_") || engine_category(name) == Some("Wallet") {
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
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib console_policy`
Expected: `test result: ok. 8 passed`.

- [ ] **Step 6: Commit**

```bash
git add crates/btx-core/src/console_policy.rs crates/btx-core/src/console_policy/engine-help-v0.34.9.txt crates/btx-core/src/lib.rs
git commit -m "core: the command window's policy, one pure function that refuses by default"
```

---

### Task 2: The confirm token

**Files:**
- Modify: `crates/btx-core/src/console_policy.rs` (append above the test module; add tests inside it)

**Interfaces:**
- Consumes: `Call` from Task 1.
- Produces: `pub const CONFIRM_TTL: Duration`, `pub struct ConfirmBook` with `pub const fn new() -> Self`, `pub fn issue(&mut self, token: String, call: Call, now: Instant)`, `pub fn redeem(&mut self, token: &str, now: Instant) -> Option<Call>`, and `pub fn new_token() -> String` (32 lowercase hex characters).

- [ ] **Step 1: Write the failing tests** (inside `mod tests`)

```rust
    fn a_call() -> Call {
        Call { method: "addnode".into(), params: vec![json!("1.2.3.4"), json!("onetry")] }
    }

    #[test]
    fn a_token_redeems_once_and_returns_the_stored_call_unchanged() {
        let t0 = std::time::Instant::now();
        let mut book = ConfirmBook::new();
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(book.redeem("abc", t0 + std::time::Duration::from_secs(5)), Some(a_call()));
        assert_eq!(book.redeem("abc", t0 + std::time::Duration::from_secs(6)), None, "used twice");
    }

    #[test]
    fn an_expired_or_wrong_token_is_refused_and_clears_the_book() {
        let t0 = std::time::Instant::now();
        let mut book = ConfirmBook::new();
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(book.redeem("abc", t0 + CONFIRM_TTL + std::time::Duration::from_millis(1)), None);
        book.issue("abc".into(), a_call(), t0);
        assert_eq!(book.redeem("xyz", t0), None);
        assert_eq!(book.redeem("abc", t0), None, "a wrong token cancels the pending call");
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
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib console_policy`
Expected: compile errors for `ConfirmBook`, `CONFIRM_TTL`, `new_token`.

- [ ] **Step 3: Implement**

```rust
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
```

- [ ] **Step 4: Run to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib console_policy`
Expected: `test result: ok. 12 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/btx-core/src/console_policy.rs
git commit -m "core: a one-time, 30-second token behind every confirm click"
```

---

### Task 3: Engine notices, hidden ones included

**Files:**
- Modify: `crates/btx-core/src/engine_warnings.rs` (the `NOT_SHOWN` block inside `classify`, plus new items after `from_node`)

**Interfaces:**
- Produces: `pub struct Notice { pub raw: String, pub message: String, pub needs_attention: bool, pub hidden_because: Option<&'static str> }` (Serialize), `pub fn all_notices(info: &BlockchainInfo) -> Vec<Notice>`.

- [ ] **Step 1: Write the failing tests** (inside the file's existing `mod tests`)

```rust
    fn info_with(warnings: &[&str]) -> BlockchainInfo {
        BlockchainInfo { warnings: warnings.iter().map(|w| w.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn all_notices_keeps_the_hidden_ones_and_says_why() {
        let n = all_notices(&info_with(&[
            "Cadence burst hold active: pacing background validation",
            "Warning: Deep reorg detected (depth 400)",
            "This is a pre-release test build - use at your own risk",
            "Warning: Unrecognised block version being mined",
        ]));
        assert_eq!(n.len(), 4);
        for notice in &n {
            assert!(notice.hidden_because.is_some(), "{notice:?}");
            assert!(!notice.message.is_empty());
            assert!(!notice.needs_attention);
        }
        assert!(n[0].message.contains("pacing"));
        assert_eq!(n[0].raw, "Cadence burst hold active: pacing background validation");
    }

    #[test]
    fn all_notices_shows_what_the_home_screen_shows_unhidden() {
        let n = all_notices(&info_with(&["Warning: Found invalid chain more than 6 blocks longer than our best chain."]));
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].hidden_because, None);
        assert!(n[0].message.contains("longer chain"));
    }

    #[test]
    fn hiding_did_not_change_for_the_home_screen() {
        for raw in [
            "Cadence burst hold active",
            "Deep reorg detected",
            "pre-release test build",
            "attempting to activate unknown new rules",
            "Unrecognised block version",
        ] {
            assert_eq!(classify(raw), None, "{raw}");
        }
    }

    #[test]
    fn notice_sentences_are_plain() {
        for raw in ["Cadence burst hold", "Deep reorg detected", "pre-release test build", "Unrecognised block version"] {
            let n = &all_notices(&info_with(&[raw]))[0];
            assert!(!n.message.contains('\u{2014}') && !n.hidden_because.unwrap().contains('\u{2014}'));
        }
    }
```

If `BlockchainInfo` does not derive `Default`, add `Default` to its derive list in `node_api.rs` in this step (every field is a number, bool, `i64` or `Vec<String>`).

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib engine_warnings`
Expected: compile errors for `all_notices` and `Notice`.

- [ ] **Step 3: Implement**

Replace the `NOT_SHOWN` constant and its check inside `classify` with:

```rust
    if hidden_kind(t).is_some() {
        return None;
    }
```

Add after `from_node`:

```rust
/// A warning as Tools shows it: every one the engine reports, the hidden
/// kinds included, each with the reason the home screen leaves it out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub raw: String,
    pub message: String,
    pub needs_attention: bool,
    /// Why the home screen leaves it out; `None` when it is shown there.
    pub hidden_because: Option<&'static str>,
}

/// The kinds the home screen hides on purpose: (phrase, sentence, reason).
const HIDDEN: [(&str, &str, &str); 5] = [
    (
        "Cadence burst hold",
        "The engine is pacing how fast it adds blocks while it catches up, to leave room for new ones.",
        "Local pacing while catching up. The engine says this is not a problem with the chain.",
    ),
    (
        "Deep reorg detected",
        "Since it started, the node switched branches deeper than usual. On this network that is usually a node rejoining the main chain.",
        "It stays until the next restart, even after what caused it is over.",
    ),
    (
        "pre-release test build",
        "The node engine is a test build.",
        "Which engine ships is this project's choice, not something to act on.",
    ),
    (
        "attempting to activate unknown new rules",
        "Some miners are signalling rules this engine does not know yet.",
        "Signalling is not activation. Nobody needs to act on it.",
    ),
    (
        "Unrecognised block version",
        "Some miners are signalling rules this engine does not know yet.",
        "Signalling is not activation. Nobody needs to act on it.",
    ),
];

fn hidden_kind(t: &str) -> Option<(&'static str, &'static str)> {
    HIDDEN.iter().find(|(phrase, _, _)| t.contains(phrase)).map(|(_, sentence, why)| (*sentence, *why))
}

pub fn all_notices(info: &BlockchainInfo) -> Vec<Notice> {
    info.warnings
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .map(|w| match hidden_kind(w) {
            Some((sentence, why)) => Notice {
                raw: w.to_string(),
                message: sentence.to_string(),
                needs_attention: false,
                hidden_because: Some(why),
            },
            None => {
                let kind = classify(w).unwrap_or(EngineWarning::Other { text: first_sentence(w) });
                Notice {
                    raw: w.to_string(),
                    message: kind.message(),
                    needs_attention: kind.needs_attention(),
                    hidden_because: None,
                }
            }
        })
        .collect()
}
```

- [ ] **Step 4: Run to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib engine_warnings`
Expected: `test result: ok. 23 passed` (19 existing plus 4).

- [ ] **Step 5: Commit**

```bash
git add crates/btx-core/src/engine_warnings.rs crates/btx-core/src/node_api.rs
git commit -m "core: every engine notice, the hidden ones with the reason they are hidden"
```

---

### Task 4: Peer sync heights, the debug.log tail, the published peers

**Files:**
- Modify: `crates/btx-core/src/node_api.rs` (`PeerInfo`)
- Modify: `crates/btx-core/src/node.rs` (next to `node_log_tail`)

**Interfaces:**
- Produces: `PeerInfo::synced_headers: i64`, `PeerInfo::synced_blocks: i64` (both `-1` when absent); `pub fn debug_log_tail(datadir: &Path, max: u64) -> String`; `pub fn published_peer_hosts() -> Vec<String>` (hosts without ports).

- [ ] **Step 1: Write the failing tests**

In `node_api.rs` tests:

```rust
    #[test]
    fn peer_sync_heights_decode_and_default_to_minus_one() {
        let with: PeerInfo = serde_json::from_value(json!({"id": 4, "synced_headers": 233472, "synced_blocks": 225928})).unwrap();
        assert_eq!((with.synced_headers, with.synced_blocks), (233472, 225928));
        let without: PeerInfo = serde_json::from_value(json!({"id": 5})).unwrap();
        assert_eq!((without.synced_headers, without.synced_blocks), (-1, -1));
    }
```

In `node.rs` tests:

```rust
    #[test]
    fn debug_log_tail_reads_only_the_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("debug.log"), "old line\nnew line\n").unwrap();
        assert_eq!(debug_log_tail(dir.path(), 9), "new line\n");
        assert_eq!(debug_log_tail(&dir.path().join("missing"), 100), "");
    }

    #[test]
    fn published_peers_are_the_ones_the_app_ships() {
        let hosts = published_peer_hosts();
        assert!(hosts.contains(&"20.86.181.203".to_string()), "btxscan's mirror");
        assert!(hosts.contains(&"109.199.124.187".to_string()), "an archive peer");
        assert!(hosts.contains(&"node.btx.dev".to_string()));
        assert!(hosts.iter().all(|h| !h.contains(':')), "no ports");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib -- peer_sync_heights debug_log_tail published_peers`
Expected: compile errors for the new fields and functions.

- [ ] **Step 3: Implement**

In `PeerInfo`, after `subver`:

```rust
    /// The last header this peer announced that we also have, `-1` when none.
    /// Which peer to ask for a stuck block is chosen by this.
    #[serde(default = "minus_one")]
    pub synced_headers: i64,
    /// The last block we know this peer has, `-1` when none.
    #[serde(default = "minus_one")]
    pub synced_blocks: i64,
```

and, at module level in `node_api.rs`:

```rust
fn minus_one() -> i64 {
    -1
}
```

In `node.rs`, after `node_log_tail`:

```rust
/// Tail of the engine's own `debug.log`, as text. Empty when missing.
pub fn debug_log_tail(datadir: &Path, max: u64) -> String {
    read_tail(&datadir.join("debug.log"), max)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The hosts of every peer the app ships in its source. These are public, so
/// diagnostics may name them; any other peer address is removed.
pub fn published_peer_hosts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for peer in BTX_BOOTSTRAP_PEERS
        .iter()
        .chain(BTX_ARCHIVE_PEERS.iter())
        .chain(BTX_DISCOVERY_PEERS.iter())
        .chain(crate::signer::BTX_MIRRORS_FED_BY_SIGNERS.iter())
    {
        let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer).to_string();
        if !out.contains(&host) {
            out.push(host);
        }
    }
    out
}
```

`PeerInfo` derives `Default`, which gives `0` for the new fields. Nothing in the codebase builds a `PeerInfo` by `Default` and then reads these fields, so that is harmless; the planner in Task 6 only reads decoded peers.

- [ ] **Step 4: Run to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib`
Expected: every test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/btx-core/src/node_api.rs crates/btx-core/src/node.rs
git commit -m "core: peer sync heights, the debug.log tail and the published peer hosts"
```

---

### Task 5: Diagnostics: warning lines, the report, redaction

**Files:**
- Create: `crates/btx-core/src/diagnostics.rs`
- Modify: `crates/btx-core/src/lib.rs` (add `pub mod diagnostics;` after `pub mod datadir;`)

**Interfaces:**
- Consumes: `engine_warnings::all_notices` (Task 3), `PeerInfo` sync heights (Task 4), `fork::ChainTip`, `node_api::{AttestedTip, BlockchainInfo, ChainStates}`.
- Produces: `pub const LOG_TAIL_BYTES: u64`, `pub fn warning_lines(log_tail: &str) -> Vec<String>`, `pub struct HeldBranchState { pub height: u64, pub root: String, pub state: String }`, `pub struct DiagnosticsInput { .. }` (fields below), `pub fn render(i: &DiagnosticsInput) -> String`, `pub struct RedactionContext { pub home: Option<String>, pub secrets: Vec<String>, pub published_hosts: Vec<String> }`, `pub fn redact(text: &str, ctx: &RedactionContext) -> String`.

- [ ] **Step 1: Write the failing tests** (bottom of the new file)

```rust
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
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib diagnostics`
Expected: compile errors (module items missing).

- [ ] **Step 3: Implement** (top of `diagnostics.rs`)

```rust
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
        if i > 0 && (s.len() - i) % 3 == 0 {
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
```

Note for the implementer: `redact` treats a published host by its bare host. `20.86.181.203:19338` parses as a `SocketAddr` whose IP is in the list, so it stays; `node.btx.dev:19335` is neither an IP nor an onion name, so it is never touched.

- [ ] **Step 4: Run to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib diagnostics`
Expected: `test result: ok. 4 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/btx-core/src/diagnostics.rs crates/btx-core/src/lib.rs
git commit -m "core: the diagnostics report, and redaction tested against every private item"
```

---

### Task 6: The stuck-block planner

**Files:**
- Create: `crates/btx-core/src/stuck_blocks.rs`
- Modify: `crates/btx-core/src/lib.rs` (add `pub mod stuck_blocks;` after `pub mod snapshot_serve;`)

**Interfaces:**
- Consumes: `PeerInfo::synced_headers` (Task 4), `fork::ChainTip`.
- Produces: `pub const MAX_BLOCKS: usize = 16`, `pub const MAX_WALK: u64 = 5_000`, `pub struct FetchRequest { pub height: u64, pub hash: String, pub peer_id: i64 }` (Serialize), `pub enum FetchPlan { Ask(Vec<FetchRequest>), Nothing(String) }`, `pub fn target_tip(tips: &[ChainTip], blocks: u64) -> Option<&ChainTip>`, `pub fn plan(missing: &[(u64, String)], peers: &[PeerInfo]) -> FetchPlan`, `pub fn summary(reqs: &[FetchRequest]) -> String`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: i64, ct: &str, inbound: bool, headers: i64) -> PeerInfo {
        PeerInfo { id, connection_type: ct.into(), inbound, synced_headers: headers, ..Default::default() }
    }
    fn tip(height: u64, status: &str) -> ChainTip {
        ChainTip { height, hash: format!("{height:064x}"), branchlen: 1, status: status.into(), ..Default::default() }
    }
    fn missing(from: u64, n: u64) -> Vec<(u64, String)> {
        (from..from + n).map(|h| (h, format!("{h:064x}"))).collect()
    }

    #[test]
    fn asks_the_app_s_own_peers_first_then_outbound_then_inbound() {
        let peers = [peer(1, "inbound", true, 300), peer(2, "outbound-full-relay", false, 300), peer(3, "manual", false, 300)];
        let FetchPlan::Ask(reqs) = plan(&missing(101, 3), &peers) else { panic!() };
        assert!(reqs.iter().all(|r| r.peer_id == 3));
        let FetchPlan::Ask(reqs) = plan(&missing(101, 1), &peers[..2]) else { panic!() };
        assert_eq!(reqs[0].peer_id, 2);
    }

    #[test]
    fn only_peers_that_announced_the_block_are_asked() {
        let peers = [peer(3, "manual", false, 101), peer(2, "outbound-full-relay", false, 110)];
        let FetchPlan::Ask(reqs) = plan(&missing(101, 3), &peers) else { panic!() };
        assert_eq!(reqs.iter().map(|r| r.peer_id).collect::<Vec<_>>(), vec![3, 2, 2]);
    }

    #[test]
    fn at_most_sixteen_blocks() {
        let FetchPlan::Ask(reqs) = plan(&missing(101, 40), &[peer(3, "manual", false, 1_000)]) else { panic!() };
        assert_eq!(reqs.len(), MAX_BLOCKS);
        assert_eq!(reqs[0].height, 101);
    }

    #[test]
    fn says_so_when_there_is_nothing_to_do() {
        assert!(matches!(plan(&[], &[peer(3, "manual", false, 1)]), FetchPlan::Nothing(s) if s.contains("every block")));
        assert!(matches!(plan(&missing(101, 2), &[peer(3, "manual", false, 100)]), FetchPlan::Nothing(s) if s.contains("No connected peer")));
    }

    #[test]
    fn follows_the_highest_tip_that_is_not_invalid() {
        let tips = [tip(100, "active"), tip(233_453, "headers-only"), tip(233_467, "valid-headers"), tip(240_000, "invalid")];
        assert_eq!(target_tip(&tips, 100).unwrap().height, 233_467);
        assert!(target_tip(&[tip(100, "active")], 100).is_none());
    }

    #[test]
    fn the_summary_counts_peers_and_blocks() {
        let reqs = vec![
            FetchRequest { height: 1, hash: "a".into(), peer_id: 3 },
            FetchRequest { height: 2, hash: "b".into(), peer_id: 2 },
        ];
        assert_eq!(summary(&reqs), "Asked 2 peers for 2 blocks.");
        assert_eq!(summary(&reqs[..1]), "Asked 1 peer for 1 block.");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib stuck_blocks`
Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
//! "Fetch a stuck block": which blocks to ask for, and which peer to ask.
//!
//! For a node that knows of blocks it does not have and is asking nobody for
//! them. The watchdog names the fix (`StallClass::BlockFetchGated`); on
//! 2026-09-29 asking an archive peer by name moved a stuck mirror at about 940
//! blocks a minute. This button is for a stuck tip, so it asks for at most
//! [`MAX_BLOCKS`].

use crate::fork::ChainTip;
use crate::node_api::PeerInfo;

pub const MAX_BLOCKS: usize = 16;
/// Farther than this and the header walk is not what a quick action is for.
pub const MAX_WALK: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FetchRequest {
    pub height: u64,
    pub hash: String,
    pub peer_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchPlan {
    Ask(Vec<FetchRequest>),
    Nothing(String),
}

/// The header tip to follow: the highest one above `blocks` that is not invalid.
pub fn target_tip(tips: &[ChainTip], blocks: u64) -> Option<&ChainTip> {
    tips.iter()
        .filter(|t| t.height > blocks && matches!(t.status.as_str(), "headers-only" | "valid-headers" | "valid-fork"))
        .max_by_key(|t| t.height)
}

/// `missing`: (height, hash) above the tip on the followed chain, lowest first.
pub fn plan(missing: &[(u64, String)], peers: &[PeerInfo]) -> FetchPlan {
    if missing.is_empty() {
        return FetchPlan::Nothing("Your node has every block it knows of. Nothing to fetch.".into());
    }
    let mut ranked: Vec<&PeerInfo> = peers.iter().collect();
    ranked.sort_by_key(|p| rank(p));
    let reqs: Vec<FetchRequest> = missing
        .iter()
        .take(MAX_BLOCKS)
        .filter_map(|(height, hash)| {
            ranked
                .iter()
                .find(|p| p.synced_headers >= *height as i64)
                .map(|p| FetchRequest { height: *height, hash: hash.clone(), peer_id: p.id })
        })
        .collect();
    if reqs.is_empty() {
        FetchPlan::Nothing(
            "No connected peer has announced the next block yet. Give it a few minutes.".into(),
        )
    } else {
        FetchPlan::Ask(reqs)
    }
}

fn rank(p: &PeerInfo) -> u8 {
    if p.connection_type == "manual" {
        0
    } else if !p.inbound {
        1
    } else {
        2
    }
}

pub fn summary(reqs: &[FetchRequest]) -> String {
    let peers: std::collections::BTreeSet<i64> = reqs.iter().map(|r| r.peer_id).collect();
    let (np, nb) = (peers.len(), reqs.len());
    format!(
        "Asked {np} {} for {nb} {}.",
        if np == 1 { "peer" } else { "peers" },
        if nb == 1 { "block" } else { "blocks" }
    )
}
```

- [ ] **Step 4: Run to see them pass**

Run: `cargo test --manifest-path crates/btx-core/Cargo.toml --lib stuck_blocks`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/btx-core/src/stuck_blocks.rs crates/btx-core/src/lib.rs
git commit -m "core: plan a stuck-block fetch, at most 16 blocks from peers that have them"
```

---

### Task 7: The Tauri commands

**Files:**
- Create: `apps/node/src-tauri/src/tools.rs`
- Modify: `apps/node/src-tauri/src/lib.rs` (add `mod tools;` after `mod tray;`; register the commands in `generate_handler!` after `commands::open_data_folder,`)
- Modify: `apps/node/src-tauri/src/commands.rs:4628` (`async fn node_ownership` becomes `pub(crate) async fn node_ownership`)

**Interfaces:**
- Consumes: everything from Tasks 1 to 6; `crate::ask::{Ask, degrade}`; `crate::commands::{node_ownership, destructive_allowed, restart_node_projected, NODE_RELEASE_TAG, NODE_RELEASE_COMMIT}`; `crate::state::{AppState, node_datadir}`.
- Produces (window-facing): `tools_console_run(line: String) -> ConsoleAnswer`, `tools_console_confirm(token: String) -> ConsoleAnswer`, `tools_engine_notices() -> Ask<Vec<Notice>>`, `tools_diagnostics(status_line: String, window_lines: Vec<String>) -> String`, `tools_fetch_stuck_blocks() -> FetchOutcome`, `tools_restart_check() -> Option<String>`, `tools_restart_node() -> ()`. `ConsoleAnswer` serialises as `{kind: "output", text}`, `{kind: "confirm", token, sentence}`, `{kind: "refused", sentence}`, `{kind: "stopped"}`. `FetchOutcome` is `{message: String, tip_before: u64, tip_after: u64}`.

- [ ] **Step 1: Write the failing test**

The commands are glue over tested pure functions. The one pure piece added here is how an RPC answer becomes text. Put this test at the bottom of the new `tools.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::answer_text;
    use serde_json::json;

    #[test]
    fn strings_print_raw_and_objects_print_pretty() {
        assert_eq!(answer_text(&json!("line one\nline two")), "line one\nline two");
        assert_eq!(answer_text(&json!({"a": 1})), "{\n  \"a\": 1\n}");
        assert_eq!(answer_text(&json!(null)), "null");
    }
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test --manifest-path apps/node/src-tauri/Cargo.toml tools::`
Expected: compile error, `answer_text` not found (and `mod tools` missing).

- [ ] **Step 3: Implement `tools.rs`**

```rust
//! Tools: one overlay with quick actions, Copy diagnostics and the command
//! window. Every decision is a pure function in btx-core; this module only
//! gathers answers from the node and hands them over.
//! docs/decisions/2026-09-29-tools-and-command-window.md

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use btx_core::console_policy::{self, ConfirmBook, Decision};
use btx_core::diagnostics::{self, DiagnosticsInput, HeldBranchState, RedactionContext};
use btx_core::engine_warnings::Notice;
use btx_core::node_api as api;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::stuck_blocks::{self, FetchPlan};

use crate::ask::{degrade, Ask};
use crate::commands::{destructive_allowed, node_ownership, restart_node_projected};
use crate::state::{node_datadir, AppState};

static CONFIRM: std::sync::Mutex<ConfirmBook> = std::sync::Mutex::new(ConfirmBook::new());

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConsoleAnswer {
    Output { text: String },
    Confirm { token: String, sentence: String },
    Refused { sentence: String },
    Stopped,
}

pub(crate) fn answer_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

async fn rpc_handle(state: &State<'_, AppState>) -> Option<RpcClient> {
    state.rpc.lock().await.clone()
}

async fn run_call(state: &State<'_, AppState>, call: console_policy::Call) -> ConsoleAnswer {
    let Some(rpc) = rpc_handle(state).await else {
        return ConsoleAnswer::Stopped;
    };
    match rpc.call(&call.method, Value::Array(call.params)).await {
        Ok(v) => ConsoleAnswer::Output { text: answer_text(&v) },
        Err(e) => ConsoleAnswer::Output { text: format!("The node answered: {e}") },
    }
}

#[tauri::command]
pub async fn tools_console_run(line: String, state: State<'_, AppState>) -> Result<ConsoleAnswer, String> {
    Ok(match console_policy::decide(&line) {
        Decision::Run(call) => run_call(&state, call).await,
        Decision::Local(text) => ConsoleAnswer::Output { text },
        Decision::Refuse(sentence) => ConsoleAnswer::Refused { sentence },
        Decision::Confirm { call, sentence } => {
            let token = console_policy::new_token();
            CONFIRM
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .issue(token.clone(), call, std::time::Instant::now());
            ConsoleAnswer::Confirm { token, sentence }
        }
    })
}

#[tauri::command]
pub async fn tools_console_confirm(token: String, state: State<'_, AppState>) -> Result<ConsoleAnswer, String> {
    let call = CONFIRM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .redeem(&token, std::time::Instant::now());
    Ok(match call {
        Some(call) => run_call(&state, call).await,
        None => ConsoleAnswer::Refused {
            sentence: "That confirmation has expired. Run the command again.".into(),
        },
    })
}

#[tauri::command]
pub async fn tools_engine_notices(state: State<'_, AppState>) -> Result<Ask<Vec<Notice>>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    Ok(match api::get_blockchain_info(&rpc).await {
        Ok(info) => Ask::Ready(btx_core::engine_warnings::all_notices(&info)),
        Err(e) => degrade(e),
    })
}

/// `None` when Restart node may run; otherwise the sentence saying why not.
#[tauri::command]
pub async fn tools_restart_check(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let owner = node_ownership(&state, &node_datadir()).await;
    Ok(destructive_allowed(owner).err())
}

#[tauri::command]
pub async fn tools_restart_node(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    destructive_allowed(node_ownership(&state, &node_datadir()).await)?;
    restart_node_projected(&app, &state).await
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchOutcome {
    pub message: String,
    pub tip_before: u64,
    pub tip_after: u64,
}

/// Walk back from `target` to the block above `tip_height`, and check it
/// sits on the node's own tip.
async fn missing_blocks(
    rpc: &RpcClient,
    target: &btx_core::fork::ChainTip,
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<(u64, String)>, String> {
    if target.height.saturating_sub(tip_height) > stuck_blocks::MAX_WALK {
        return Err("Your node is far behind. That is catching up, not a stuck block; leave it running.".into());
    }
    let mut chain = Vec::new();
    let mut hash = target.hash.clone();
    loop {
        let h = rpc.call("getblockheader", json!([hash, true])).await.map_err(|e| e.to_string())?;
        let height = h["height"].as_u64().ok_or("The node sent a header without a height.")?;
        chain.push((height, hash.clone()));
        let prev = h["previousblockhash"].as_str().unwrap_or("").to_string();
        if height <= tip_height + 1 {
            if prev != tip_hash {
                return Err("The newest headers are on another branch than your node's tip. The node decides that on its own.".into());
            }
            break;
        }
        hash = prev;
    }
    chain.reverse();
    Ok(chain)
}

#[tauri::command]
pub async fn tools_fetch_stuck_blocks(state: State<'_, AppState>) -> Result<FetchOutcome, String> {
    let rpc = rpc_handle(&state).await.ok_or("Start your node first.")?;
    let info = api::get_blockchain_info(&rpc).await.map_err(|e| e.to_string())?;
    let tip_hash = rpc.call("getbestblockhash", json!([])).await.map_err(|e| e.to_string())?;
    let tip_hash = tip_hash.as_str().unwrap_or("").to_string();
    let tips = api::get_chain_tips(&rpc).await.map_err(|e| e.to_string())?;
    let done = |message: String| FetchOutcome { message, tip_before: info.blocks, tip_after: info.blocks };
    let Some(target) = stuck_blocks::target_tip(&tips, info.blocks) else {
        return Ok(done("Your node has every block it knows of. Nothing to fetch.".into()));
    };
    let missing = match missing_blocks(&rpc, target, info.blocks, &tip_hash).await {
        Ok(m) => m,
        Err(sentence) => return Ok(done(sentence)),
    };
    let peers = api::get_peer_info(&rpc).await.map_err(|e| e.to_string())?;
    let reqs = match stuck_blocks::plan(&missing, &peers) {
        FetchPlan::Nothing(sentence) => return Ok(done(sentence)),
        FetchPlan::Ask(reqs) => reqs,
    };
    let mut asked = Vec::new();
    for r in &reqs {
        if rpc.call("getblockfrompeer", json!([r.hash, r.peer_id])).await.is_ok() {
            asked.push(r.clone());
        }
    }
    if asked.is_empty() {
        return Ok(done("The peers did not take the request. Give it a few minutes.".into()));
    }
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    let after = api::get_blockchain_info(&rpc).await.map(|i| i.blocks).unwrap_or(info.blocks);
    let moved = if after > info.blocks {
        format!(" The tip moved from {} to {}.", info.blocks, after)
    } else {
        " No block has connected yet; the node may still be checking them.".to_string()
    };
    Ok(FetchOutcome { message: format!("{}{}", stuck_blocks::summary(&asked), moved), tip_before: info.blocks, tip_after: after })
}

#[tauri::command]
pub async fn tools_diagnostics(
    status_line: String,
    window_lines: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let datadir = node_datadir();
    let rpc = rpc_handle(&state).await;
    let mut input = DiagnosticsInput {
        generated_at: format!("{} UTC", chrono_like_now()),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_pinned: format!(
            "{} ({})",
            crate::commands::NODE_RELEASE_TAG,
            &crate::commands::NODE_RELEASE_COMMIT[..8]
        ),
        platform: format!(
            "{} {}, installed as {:?}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            tauri::utils::platform::bundle_type()
        ),
        status_line,
        window_lines,
        phase: format!("{:?}", *state.phase.lock().await),
        signer_pubkey: state.signer_pubkey.lock().await.clone(),
        stall: state.stall_verdict.lock().await.as_ref().map(|v| v.summary.to_string()),
        log_warnings: diagnostics::warning_lines(&btx_core::node::debug_log_tail(&datadir, diagnostics::LOG_TAIL_BYTES)),
        ..Default::default()
    };
    if let Some(rpc) = &rpc {
        input.chain = api::get_blockchain_info(rpc).await.ok();
        input.best_block_hash = rpc.call("getbestblockhash", json!([])).await.ok().and_then(|v| v.as_str().map(str::to_string));
        input.engine_running = rpc.call("getnetworkinfo", json!([])).await.ok().and_then(|v| v["subversion"].as_str().map(str::to_string));
        input.chainstates = api::get_chainstates(rpc).await.ok();
        input.tips = api::get_chain_tips(rpc).await.unwrap_or_default();
        input.peers = api::get_peer_info(rpc).await.unwrap_or_default();
        input.attested_tip = api::get_attested_tip(rpc).await.ok();
        let trusted = api::get_matmul_trusted_status(rpc).await.ok();
        input.role = match trusted {
            Some(t) if t.trusted_mirror => "follows signatures".into(),
            Some(t) => format!("checks blocks itself ({})", t.matmul_validation_mode),
            None => "unknown".into(),
        };
        for h in btx_core::known_invalid::HELD_BRANCHES {
            let on_chain = rpc
                .call("getblockhash", json!([h.height]))
                .await
                .ok()
                .and_then(|v| v.as_str().map(|s| s == h.root));
            let known = rpc.call("getblockheader", json!([h.root, true])).await.is_ok();
            let state = match (known, on_chain) {
                (_, Some(true)) => "ON THIS NODE'S CHAIN",
                (true, _) => "seen, not on this node's chain",
                (false, _) => "not seen by this node",
            };
            input.held.push(HeldBranchState { height: h.height, root: h.root[..16].to_string(), state: state.into() });
        }
    } else {
        input.role = "node not running".into();
    }
    let ctx = RedactionContext {
        home: dirs::home_dir().map(|p| p.display().to_string()),
        secrets: secrets(&datadir),
        published_hosts: btx_core::node::published_peer_hosts(),
    };
    Ok(diagnostics::redact(&diagnostics::render(&input), &ctx))
}

/// The cookie password and the signing key's own text, read only to be removed.
fn secrets(datadir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(cookie) = std::fs::read_to_string(datadir.join(".cookie")) {
        if let Some((_, pass)) = cookie.trim().split_once(':') {
            out.push(pass.to_string());
        }
    }
    if let Ok(wif) = std::fs::read_to_string(btx_core::signer::signer_key_path(datadir)) {
        out.push(wif.trim().to_string());
    }
    out
}

/// "2026-09-29 14:05" in UTC, without a date crate.
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3_600, (rem % 3_600) / 60)
}
```

Before running, check three names against the code and adjust the calls if they differ (they were read on main at b008f98): `state.signer_pubkey` is `Arc<Mutex<Option<String>>>` (state.rs), `StallVerdict::summary` is a `&'static str` (watchdog.rs), `NODE_RELEASE_TAG` and `NODE_RELEASE_COMMIT` are `pub const` in `commands.rs`. `tauri::utils::platform::bundle_type()` returns `Option<BundleType>` in Tauri 2; `dirs` is already a dependency of btx-core; if the app crate lacks it, use `btx_core::datadir` or `std::env::var("HOME")`/`USERPROFILE` instead.

Add a test for the date helper in the same test module:

```rust
    #[test]
    fn the_date_helper_formats_like_utc() {
        let s = super::chrono_like_now();
        assert_eq!(s.len(), 16);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
    }
```

Register in `lib.rs`, after `commands::open_data_folder,`:

```rust
            tools::tools_console_run,
            tools::tools_console_confirm,
            tools::tools_engine_notices,
            tools::tools_diagnostics,
            tools::tools_fetch_stuck_blocks,
            tools::tools_restart_check,
            tools::tools_restart_node,
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test --manifest-path apps/node/src-tauri/Cargo.toml tools::`
Expected: `test result: ok. 2 passed`. Then `cargo test --manifest-path apps/node/src-tauri/Cargo.toml` and `cargo clippy --manifest-path apps/node/src-tauri/Cargo.toml -- -D warnings` both clean.

- [ ] **Step 5: Commit**

```bash
git add apps/node/src-tauri/src/tools.rs apps/node/src-tauri/src/lib.rs apps/node/src-tauri/src/commands.rs
git commit -m "node: Tools commands, the command window, diagnostics, stuck blocks, restart"
```

---

### Task 8: The Tools overlay

**Files:**
- Create: `apps/node/src/tools-history.ts`, `apps/node/src/tools-history.test.ts`, `apps/node/src/tools.ts`
- Modify: `apps/node/index.html` (a button after `#ask-btn`, an overlay after `#ask-overlay`'s closing `</div>`), `apps/node/src/main.ts` (import and call `initTools()` right after `initAsk();`), `apps/node/src/styles.css` (append)

**Interfaces:**
- Consumes: the Task 7 commands, and `open_data_folder`.
- Produces: `initTools(): void`; pure `History` and `capForDisplay` in `tools-history.ts`.

- [ ] **Step 1: Write the failing tests** (`tools-history.test.ts`)

```ts
import { describe, expect, it } from "vitest";
import { History, capForDisplay, DISPLAY_LIMIT } from "./tools-history";

describe("History", () => {
  it("keeps the last 50 and recalls them with up and down", () => {
    const h = new History(50);
    for (let i = 0; i < 60; i++) h.push({ line: `cmd ${i}`, answer: "" });
    expect(h.entries.length).toBe(50);
    expect(h.entries[0].line).toBe("cmd 10");
    expect(h.up()).toBe("cmd 59");
    expect(h.up()).toBe("cmd 58");
    expect(h.down()).toBe("cmd 59");
    expect(h.down()).toBe("");
  });
  it("starts recall again after a new command", () => {
    const h = new History(50);
    h.push({ line: "a", answer: "" });
    h.up();
    h.push({ line: "b", answer: "" });
    expect(h.up()).toBe("b");
  });
  it("copies everything as plain text", () => {
    const h = new History(50);
    h.push({ line: "getblockcount", answer: "233480" });
    h.push({ line: "uptime", answer: "120" });
    expect(h.allText()).toBe("> getblockcount\n233480\n\n> uptime\n120");
  });
});

describe("capForDisplay", () => {
  it("shows up to 256 KB and says Copy takes the rest", () => {
    expect(DISPLAY_LIMIT).toBe(256 * 1024);
    expect(capForDisplay("short")).toBe("short");
    const long = "x".repeat(DISPLAY_LIMIT + 10);
    const shown = capForDisplay(long);
    expect(shown.startsWith("x".repeat(100))).toBe(true);
    expect(shown).toContain("Copy takes the whole answer");
    expect(shown.length).toBeLessThan(long.length);
  });
});
```

- [ ] **Step 2: Run to see them fail**

Run: `npm --prefix apps/node ci && npm --prefix apps/node test -- tools-history`
Expected: FAIL, cannot find module `./tools-history`.

- [ ] **Step 3: Implement `tools-history.ts`**

```ts
// Pure helpers for the Tools command window: the session's history and the
// display cap. The history lives in memory only and is gone when the app closes.

export interface Entry {
  line: string;
  answer: string;
}

export const DISPLAY_LIMIT = 256 * 1024;

export class History {
  entries: Entry[] = [];
  private cursor = -1;
  constructor(private readonly max: number) {}

  push(e: Entry): void {
    this.entries.push(e);
    if (this.entries.length > this.max) this.entries.shift();
    this.cursor = -1;
  }

  /** The previous command, walking back from the newest. */
  up(): string {
    if (this.entries.length === 0) return "";
    this.cursor = this.cursor === -1 ? this.entries.length - 1 : Math.max(0, this.cursor - 1);
    return this.entries[this.cursor].line;
  }

  /** The next command, or an empty line past the newest. */
  down(): string {
    if (this.cursor === -1) return "";
    this.cursor += 1;
    if (this.cursor >= this.entries.length) {
      this.cursor = -1;
      return "";
    }
    return this.entries[this.cursor].line;
  }

  allText(): string {
    return this.entries.map((e) => `> ${e.line}\n${e.answer}`).join("\n\n");
  }
}

export function capForDisplay(text: string): string {
  if (text.length <= DISPLAY_LIMIT) return text;
  return `${text.slice(0, DISPLAY_LIMIT)}\n\n(The answer is longer than this window shows. Copy takes the whole answer.)`;
}
```

- [ ] **Step 4: Run to see them pass**

Run: `npm --prefix apps/node test -- tools-history`
Expected: 4 passed.

- [ ] **Step 5: Add the markup**

In `apps/node/index.html`, after the `#ask-btn` button:

```html
        <button id="tools-btn" class="icon-btn" type="button" aria-label="Tools" aria-haspopup="dialog" title="Tools">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor"
               stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L4 17v3h3l5.3-5.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.1-.6-.6-2.1z"></path>
          </svg>
        </button>
```

After the closing `</div>` of `#ask-overlay`:

```html
      <div class="overlay" id="tools-overlay" hidden>
        <div class="overlay-panel ask-panel" role="dialog" aria-modal="true" aria-labelledby="tools-title">
          <div class="overlay-head">
            <h2 id="tools-title">Tools</h2>
            <button id="tools-close" class="icon-btn" type="button" aria-label="Close">✕</button>
          </div>
          <p class="tools-now" id="tools-now"></p>

          <h3 class="tools-h">Quick actions</h3>
          <div class="tools-actions">
            <button id="tools-restart" class="btn-secondary" type="button">Restart node</button>
            <button id="tools-fetch" class="btn-secondary" type="button">Fetch a stuck block</button>
            <button id="tools-open-folder" class="btn-secondary" type="button">Open data folder</button>
            <button id="tools-notices-btn" class="btn-secondary" type="button" aria-expanded="false">Engine notices</button>
          </div>
          <p class="setting-result" id="tools-action-result" hidden></p>
          <div class="tools-notices" id="tools-notices" hidden></div>

          <h3 class="tools-h">Diagnostics</h3>
          <button id="tools-diag-btn" class="btn-secondary" type="button">Copy diagnostics</button>
          <p class="tools-note">Shows what it copies first. Nothing is uploaded; you choose where to paste it.</p>
          <pre class="tools-pre" id="tools-diag" hidden></pre>

          <details class="tools-console">
            <summary>Command window (advanced)</summary>
            <p class="tools-note">Runs a short list of read-only node commands. Type help to see them.</p>
            <div class="tools-console-row">
              <input id="tools-line" type="text" spellcheck="false" autocomplete="off" aria-label="Command" placeholder="getblockchaininfo" />
              <button id="tools-run" class="btn-secondary" type="button">Run</button>
            </div>
            <div class="tools-confirm" id="tools-confirm" hidden>
              <p id="tools-confirm-text"></p>
              <button id="tools-confirm-yes" class="btn-secondary" type="button">Yes, do it</button>
              <button id="tools-confirm-no" class="btn-secondary" type="button">Cancel</button>
            </div>
            <button id="tools-copy-all" class="link-row" type="button">Copy all</button>
            <div class="tools-history" id="tools-history"></div>
          </details>
        </div>
      </div>
```

- [ ] **Step 6: Write `tools.ts`**

```ts
// Tools: quick actions, Copy diagnostics and the command window. The window
// sends only the typed line; Rust decides what runs. Everything the node says
// is set as text, never HTML.

import { invoke } from "@tauri-apps/api/core";
import { History, capForDisplay } from "./tools-history";

type ConsoleAnswer =
  | { kind: "output"; text: string }
  | { kind: "confirm"; token: string; sentence: string }
  | { kind: "refused"; sentence: string }
  | { kind: "stopped" };

interface Notice {
  raw: string;
  message: string;
  needs_attention: boolean;
  hidden_because: string | null;
}
type Ask<T> =
  | { state: "ready"; data: T }
  | { state: "stopped" }
  | { state: "warming" }
  | { state: "unavailable"; data: { message: string } };

const $ = <T extends HTMLElement = HTMLElement>(id: string): T => document.getElementById(id) as T;

/** The status line exactly as the home screen shows it. */
function statusLine(): string {
  const badge = ($("status-badge").textContent ?? "").trim();
  const sub = ($("status-sub").textContent ?? "").trim();
  return [badge, sub].filter(Boolean).join(" · ");
}

async function copy(text: string, btn: HTMLButtonElement, label: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    btn.textContent = "Copied";
  } catch {
    btn.textContent = "Couldn't copy";
  }
  setTimeout(() => (btn.textContent = label), 1500);
}

function say(text: string): void {
  const r = $("tools-action-result");
  r.textContent = text;
  r.hidden = false;
}

export function initTools(): void {
  const overlay = $("tools-overlay");
  const history = new History(50);
  let pendingToken: string | null = null;
  let restartArmTimer: ReturnType<typeof setTimeout> | undefined;

  const open = async () => {
    overlay.hidden = false;
    $("tools-now").textContent = statusLine();
    const restart = $<HTMLButtonElement>("tools-restart");
    const why = await invoke<string | null>("tools_restart_check").catch(() => null);
    restart.disabled = why !== null;
    restart.title = why ?? "";
  };
  $("tools-btn").addEventListener("click", () => void open());
  $("tools-close").addEventListener("click", () => (overlay.hidden = true));
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) overlay.hidden = true;
  });

  // Restart node: two clicks, disarmed after five seconds.
  $("tools-restart").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-restart");
    if (btn.dataset.armed !== "1") {
      btn.dataset.armed = "1";
      btn.textContent = "Click again to restart the node";
      clearTimeout(restartArmTimer);
      restartArmTimer = setTimeout(() => {
        btn.dataset.armed = "";
        btn.textContent = "Restart node";
      }, 5000);
      return;
    }
    clearTimeout(restartArmTimer);
    btn.dataset.armed = "";
    btn.textContent = "Restarting...";
    btn.disabled = true;
    try {
      await invoke("tools_restart_node");
      say("Your node restarted.");
    } catch (e) {
      say(String(e));
    } finally {
      btn.textContent = "Restart node";
      btn.disabled = false;
    }
  });

  $("tools-fetch").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-fetch");
    btn.disabled = true;
    say("Asking peers for the next blocks...");
    try {
      const out = await invoke<{ message: string }>("tools_fetch_stuck_blocks");
      say(out.message);
    } catch (e) {
      say(String(e));
    } finally {
      btn.disabled = false;
    }
  });

  $("tools-open-folder").addEventListener("click", () => {
    void invoke("open_data_folder").catch((e) => say(String(e)));
  });

  $("tools-notices-btn").addEventListener("click", async () => {
    const box = $("tools-notices");
    const btn = $("tools-notices-btn");
    if (!box.hidden) {
      box.hidden = true;
      btn.setAttribute("aria-expanded", "false");
      return;
    }
    box.replaceChildren();
    const ans = await invoke<Ask<Notice[]>>("tools_engine_notices").catch(() => null);
    const add = (text: string, cls: string) => {
      const p = document.createElement("p");
      p.className = cls;
      p.textContent = text;
      box.appendChild(p);
    };
    if (!ans || ans.state !== "ready") {
      add(ans?.state === "stopped" ? "Start your node to see its notices." : "The node is not answering yet.", "tools-note");
    } else if (ans.data.length === 0) {
      add("The engine reports nothing right now.", "tools-note");
    } else {
      for (const n of ans.data) {
        add(n.message, n.needs_attention ? "tools-notice is-attention" : "tools-notice");
        if (n.hidden_because) add(`Not shown on the home screen: ${n.hidden_because}`, "tools-note");
        add(`Engine: ${n.raw}`, "tools-raw");
      }
    }
    box.hidden = false;
    btn.setAttribute("aria-expanded", "true");
  });

  $("tools-diag-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-diag-btn");
    const pre = $("tools-diag");
    btn.disabled = true;
    try {
      const lines = Array.from(document.querySelectorAll<HTMLElement>("#catchup-line, #fork-msg"))
        .map((el) => (el.textContent ?? "").trim())
        .filter(Boolean);
      const text = await invoke<string>("tools_diagnostics", { statusLine: statusLine(), windowLines: lines });
      pre.textContent = text;
      pre.hidden = false;
      await copy(text, btn, "Copy diagnostics");
    } catch (e) {
      say(String(e));
    } finally {
      btn.disabled = false;
    }
  });

  // The command window.
  const input = $<HTMLInputElement>("tools-line");
  const list = $("tools-history");
  const render = (line: string, answer: string) => {
    history.push({ line, answer });
    const item = document.createElement("div");
    item.className = "tools-entry";
    const head = document.createElement("div");
    head.className = "tools-entry-head";
    const cmd = document.createElement("code");
    cmd.textContent = `> ${line}`;
    const btn = document.createElement("button");
    btn.className = "link-row";
    btn.type = "button";
    btn.textContent = "Copy";
    btn.addEventListener("click", () => void copy(answer, btn, "Copy"));
    head.append(cmd, btn);
    const out = document.createElement("pre");
    out.className = "tools-pre";
    out.textContent = capForDisplay(answer);
    item.append(head, out);
    list.prepend(item);
    while (list.children.length > 50) list.lastElementChild?.remove();
  };
  const show = (line: string, a: ConsoleAnswer) => {
    switch (a.kind) {
      case "output":
        render(line, a.text);
        break;
      case "refused":
        render(line, a.sentence);
        break;
      case "stopped":
        render(line, "Start your node to run commands.");
        break;
      case "confirm":
        pendingToken = a.token;
        $("tools-confirm-text").textContent = a.sentence;
        $("tools-confirm").hidden = false;
        $("tools-confirm").dataset.line = line;
        break;
    }
  };
  const runLine = async () => {
    const line = input.value.trim();
    if (!line) return;
    input.value = "";
    $("tools-confirm").hidden = true;
    pendingToken = null;
    const a = await invoke<ConsoleAnswer>("tools_console_run", { line }).catch(
      (e): ConsoleAnswer => ({ kind: "refused", sentence: String(e) }),
    );
    show(line, a);
  };
  $("tools-run").addEventListener("click", () => void runLine());
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void runLine();
    if (e.key === "ArrowUp") {
      input.value = history.up();
      e.preventDefault();
    }
    if (e.key === "ArrowDown") {
      input.value = history.down();
      e.preventDefault();
    }
  });
  $("tools-confirm-yes").addEventListener("click", async () => {
    const token = pendingToken;
    const line = $("tools-confirm").dataset.line ?? "";
    pendingToken = null;
    $("tools-confirm").hidden = true;
    if (!token) return;
    const a = await invoke<ConsoleAnswer>("tools_console_confirm", { token }).catch(
      (e): ConsoleAnswer => ({ kind: "refused", sentence: String(e) }),
    );
    show(line, a);
  });
  $("tools-confirm-no").addEventListener("click", () => {
    pendingToken = null;
    $("tools-confirm").hidden = true;
  });
  $("tools-copy-all").addEventListener("click", () => {
    void copy(history.allText(), $<HTMLButtonElement>("tools-copy-all"), "Copy all");
  });
}
```

Before relying on `#catchup-line`, check the id the status screen's catch-up sentence uses in `index.html` and use that id; if the sentence has none, pass only `#fork-msg`.

In `main.ts`: `import { initTools } from "./tools";` next to `import { initAsk } from "./ask";`, and `initTools();` on the line after `initAsk();`.

Append to `styles.css`:

```css
.tools-now { color: var(--muted); font-size: 12.5px; margin: 0 0 var(--space-m); }
.tools-h { font-size: 13px; margin: var(--space-m) 0 var(--space-s); }
.tools-actions { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-s); }
.tools-note { color: var(--muted); font-size: 12px; margin: var(--space-s) 0; }
.tools-notice { margin: var(--space-s) 0 0; }
.tools-notice.is-attention { color: var(--warn, #f59e0b); }
.tools-raw { font-family: ui-monospace, monospace; font-size: 11px; color: var(--muted); margin: 2px 0 0; overflow-wrap: anywhere; }
.tools-pre { font-family: ui-monospace, monospace; font-size: 11px; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 260px; overflow: auto; background: var(--surface-2, rgba(255,255,255,0.04)); border-radius: 8px; padding: var(--space-s); margin: var(--space-s) 0 0; user-select: text; }
.tools-console { margin-top: var(--space-m); }
.tools-console-row { display: flex; gap: var(--space-s); }
.tools-console-row input { flex: 1; min-width: 0; font-family: ui-monospace, monospace; }
.tools-entry { margin-top: var(--space-s); }
.tools-entry-head { display: flex; justify-content: space-between; align-items: center; gap: var(--space-s); }
.tools-confirm { margin-top: var(--space-s); }
```

Check that `--muted`, `--space-s`, `--space-m` exist in `styles.css` (they are used by the Ask overlay); where a variable name differs, use the Ask overlay's.

- [ ] **Step 7: Build and test the window**

Run: `npm --prefix apps/node test && npm --prefix apps/node run build`
Expected: every vitest file passes; `tsc` and `vite build` finish without errors.

- [ ] **Step 8: Commit**

```bash
git add apps/node/index.html apps/node/src/tools.ts apps/node/src/tools-history.ts apps/node/src/tools-history.test.ts apps/node/src/main.ts apps/node/src/styles.css
git commit -m "node: the Tools button and overlay"
```

---

### Task 9: Every table row against a real engine (opt-in)

**Files:**
- Create: `crates/btx-core/tests/console_regtest.rs`

**Interfaces:**
- Consumes: `console_policy::decide`, `rpc::{Rpc, RpcClient}`.

- [ ] **Step 1: Write the test**

```rust
//! Every row of the command table through a real v0.34.9 btxd on regtest.
//! Opt-in, like the other shipped-engine tests:
//!   EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test console_regtest -- --ignored

use btx_core::console_policy::{decide, Decision};
use btx_core::rpc::{Rpc, RpcClient};

#[tokio::test]
#[ignore]
async fn every_row_behaves_against_the_shipped_engine() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(&btxd)
        .arg(format!("-datadir={}", dir.path().display()))
        .args(["-regtest", "-listen=0", "-connect=0", "-dnsseed=0", "-rpcport=29444",
               "-server=1", "-printtoconsole=0", "-daemon=0"])
        .spawn()
        .unwrap();
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
    let rpc = rpc.expect("regtest btxd answered");
    let genesis = rpc.call("getblockhash", serde_json::json!([0])).await.unwrap();
    let g = genesis.as_str().unwrap().to_string();
    let block = rpc.call("getblock", serde_json::json!([g, 1])).await.unwrap();
    let coinbase = block["tx"][0].as_str().unwrap().to_string();

    let runs = [
        "getblockchaininfo".to_string(), "getchaintips".into(), "getpeerinfo".into(),
        "getnetworkinfo".into(), "getmempoolinfo".into(), "getmatmulattestedtip".into(),
        "getmatmultrustedstatus".into(), "uptime".into(), "getchainstates".into(),
        "getblockcount".into(), "getbestblockhash".into(), "getconnectioncount".into(),
        "getblockhash 0".into(), format!("getblockheader {g}"), format!("getblock {g} 2"),
        format!("getrawtransaction {coinbase} 1 {g}"), format!("gettxout {coinbase} 0"),
        "help getblock".into(),
    ];
    for line in &runs {
        let Decision::Run(call) = decide(line) else { panic!("{line} did not decide Run") };
        let answer = rpc.call(&call.method, serde_json::Value::Array(call.params)).await;
        // getmatmulattestedtip may answer an RPC error on a keyless regtest node;
        // what matters is that the engine accepted the method and argument shape.
        if let Err(e) = &answer {
            assert!(!e.to_string().contains("Method not found"), "{line}: {e}");
            assert!(!e.to_string().contains("Expected type"), "{line}: {e}");
        }
    }
    for line in ["addnode 127.0.0.1:1 onetry".to_string(), format!("getblockfrompeer {g} 0")] {
        assert!(matches!(decide(&line), Decision::Confirm { .. }), "{line}");
    }
    for line in ["stop", "invalidateblock 00", "dumpprivkey x", "savemempool"] {
        assert!(matches!(decide(line), Decision::Refuse(_)), "{line}");
    }
    let _ = rpc.call("stop", serde_json::json!([])).await;
    let _ = child.wait();
}
```

- [ ] **Step 2: Run it against the staged engine**

Run: `EASYNODE_TEST_BTXD=$PWD/apps/node/src-tauri/resources/node-pkg/bin/btxd cargo test --manifest-path crates/btx-core/Cargo.toml --test console_regtest -- --ignored` (if the worktree has no staged engine, point at `/Users/m2promende/repos/easynode/apps/node/src-tauri/resources/node-pkg/bin/btxd`).
Expected: `test result: ok. 1 passed`. Also run it without the variable and expect it to print "EASYNODE_TEST_BTXD unset" and pass.

- [ ] **Step 3: Commit**

```bash
git add crates/btx-core/tests/console_regtest.rs
git commit -m "core: every command-window row checked against the shipped engine (opt-in)"
```

---

### Task 10: Changelog, full check, and a run on this Mac

**Files:**
- Modify: `apps/node/CHANGELOG.md` (under `## [Unreleased]`)

- [ ] **Step 1: Write the entry**

```markdown
### Added

- **Tools.** A new wrench button at the top opens Tools. From there you can
  restart the node, ask good peers for a block your node is stuck on, open the
  data folder, and see every notice the node engine reports, including the
  ones the home screen leaves out and why. **Copy diagnostics** puts a short
  report on your clipboard to paste into a support chat. It shows you the
  report first and leaves out anything private: keys, wallet details, other
  people's addresses and your home folder. For the curious, a command window
  runs a short list of read-only node commands.
```

- [ ] **Step 2: Run everything**

```bash
cargo test --manifest-path crates/btx-core/Cargo.toml
cargo test --manifest-path apps/node/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/node/src-tauri/Cargo.toml -- -D warnings
npm --prefix apps/node test
npm --prefix apps/node run build
grep -n '—' apps/node/CHANGELOG.md apps/node/src/tools.ts apps/node/index.html crates/btx-core/src/console_policy.rs crates/btx-core/src/diagnostics.rs crates/btx-core/src/engine_warnings.rs | grep -v '^.*://' || echo "no em-dashes in new copy"
```

Expected: all green. The em-dash grep may list pre-existing lines in `index.html` or `CHANGELOG.md`; none may be on lines this plan added.

- [ ] **Step 3: Run the app on this Mac against a throwaway data folder**

```bash
cd apps/node
EASYBTX_NODE_DATADIR=$(mktemp -d) npx tauri dev
```

Check by hand: the wrench opens Tools; Restart node needs two clicks; the command window runs `getblockcount`, refuses `stop` with its sentence, asks before `addnode 127.0.0.1 onetry` and runs it after Yes; Copy diagnostics shows the report and it contains no home path, no cookie, no key; Engine notices opens. Close with Stop node, then quit.

- [ ] **Step 4: Commit**

```bash
git add apps/node/CHANGELOG.md
git commit -m "changelog: Tools"
```

---

## Self-review

- **Spec coverage.** Section 1 (button, overlay order): Task 8; Fast-forward's slot is the confirmed-snapshot plan's, stated at the top. Section 2 (four quick actions): Tasks 3, 6, 7, 8. Section 4 (diagnostics, contents and never-list): Tasks 4, 5, 7. Section 5 (command window, policy, token, help, table): Tasks 1, 2, 7, 8. Tests section: Tasks 1, 2, 5, 6, 9, and the by-hand run in Task 10. Changelog: Task 10.
- **Placeholders.** None. Three items ask the implementer to confirm a name against code before use (state field types, a DOM id, CSS variables); each says exactly what to check and what to do if it differs.
- **Type consistency.** `Call`, `Decision`, `ConfirmBook`, `Notice`, `DiagnosticsInput`, `HeldBranchState`, `RedactionContext`, `FetchRequest`, `FetchPlan`, `ConsoleAnswer` and `FetchOutcome` are defined once and used with the same fields everywhere. Command names match between `tools.rs`, `lib.rs` and `tools.ts` (`tools_console_run`, `tools_console_confirm`, `tools_engine_notices`, `tools_diagnostics`, `tools_fetch_stuck_blocks`, `tools_restart_check`, `tools_restart_node`).
