# Confirmed Snapshots, Part 1: Verify and Load (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every node the app starts begins from a snapshot that two different operators confirmed, checked by the app before the engine sees anything, with the pinned pair (225,927) and then the engine's compiled start point as fallbacks; a validating node loads it in one mirror launch and restarts as a validating node under the pin rule.

**Architecture:** `operators.rs` holds the per-chain operator list and groups keys by operator. `confirmed_snapshot.rs` reads a v2 manifest strictly, hashes the statement, verifies strict-DER low-S ECDSA with k256, counts distinct operators, checks the pinned signature and trims to pinned signatures. `attested_snapshot.rs` reads the website pointer (contract defined here) and downloads a confirmed pair, else the pinned pair. `confirmed_load.rs` is the one loading path (re-check against the live node, refuse held blocks, trimmed manifest, `loadtxoutsetattested`, post-load held check). `node.rs` gains the pin rule of section 8, the one-time mirror-load marker and the "pins only grow" guard. The app's start path chooses the load and restarts a validating node after its mirror launch. An opt-in regtest test runs all of it against the real engine, and `scripts/check-engine-tag.sh` runs that test before any engine bump.

**Tech Stack:** Rust (btx-core, Tauri 2 shell), k256 0.13.4 with `ecdsa`, sha2, reqwest, mockito (tests), serde_json; btxd v0.34.9 (regtest) for the opt-in test.

**This is plan 1 of 2.** Plan 2, `2026-09-29-confirmed-snapshots-fast-forward.md` (same folder), builds Fast-forward (section 10) on the interfaces this plan produces. Sections 3 to 6 (diary, producers, confirmers, the website) and 11 (catch-up help) are separate plans; the interfaces they build on are listed under "Interfaces for the other plans" at the end.

## Global Constraints

- Design: `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` on `origin/claude/cosigned-snapshots` (read it with `git -C /Users/m2promende/repos/easynode show origin/claude/cosigned-snapshots:docs/decisions/2026-09-29-every-node-starts-near-the-tip.md`), approved 2026-09-29 with its choices as proposed. Sections implemented here: 1, 7, 8, 9, 12. Section 10 is plan 2.
- Branch: `claude/confirmed-snapshots`, created from `claude/tools-command-window` at `72304d9` ("changelog: Tools") or any later commit of that branch. The Tools overlay (`apps/node/src/tools.ts`, `apps/node/src-tauri/src/tools.rs`, `#tools-overlay` in `apps/node/index.html`) exists only on that branch; `origin/claude/tools-command-window` may lag the local branch, so base on the local branch if it is ahead.
- Operator list, mainnet, compiled: **Mende only**, key `02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675`. Aleksander (`03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24`, `02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0`, one operator) and jpp are NOT added until the owner confirms they agreed. Adding one is one line in `MAINNET_OPERATORS`, guarded by `every_mainnet_line_is_well_formed`. With one operator nothing is confirmed and every node uses the fallbacks.
- Test chains: the list comes from `EASYNODE_REGTEST_OPERATORS` (format `name=key[,key];name=key`) and only for a statement whose chain id is regtest's genesis hash `521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360`. A mainnet statement (chain id `75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601`) never reads it.
- Statement: 229 bytes, version 2; hash = double SHA-256 of `0x28 || "BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2" || statement`; 32-byte fields little-endian, shown reversed. File hash = double SHA-256 of the file. Manifest cap 64 KB (65,536 bytes). Signatures: strict DER (the engine's `IsStrictDERSignature`, at most 72 bytes), low S, compressed key on the curve.
- Compiled replay contexts (v0.34.9): mainnet `32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188`, regtest `9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577` (both measured from the running engine by the regtest test during this plan's dry run).
- Grid: mainnet multiples of **200**; regtest multiples of **100** (regtest's ExactReplay starts at 101, so a test chain stops at 100).
- Confirmed = every signature valid; no key twice; at least **2 different operators**; at least one signature from a key the node pins; version 2; chain and replay context as compiled and as the running node reports; height on the grid and above the node's fallback start (`max(compiled anchor, 225,927)`); file size 1..=64 MiB (67,108,864 bytes) with the engine's chunk geometry.
- Fallback order for every node: confirmed snapshot, else `attested_snapshot::pinned_pair()` (225,927), else the engine's compiled start point (219,000).
- Loading on a validating node: one launch in mirror mode via the marker `<datadir>/.load-snapshot-as-mirror` (JSON `{"height":u64,"written_at":u64}`, honoured for 6 hours), then the marker is cleared and the node restarts validating. Same order as the header bootstrap: stop, clear marker, start.
- Pin rule (section 8): while `<datadir>/chainstate_snapshot/attested_assumeutxo` exists, the validating arm of `build_node_command` also passes every key in `BTX_TRUSTED_ATTESTATION_PUBKEYS` not already pinned (by the conf or the node's own self-pin) and `-matmultrustedthreshold=1`. The mirror arm is unchanged.
- Pins only grow; the mirrors' threshold never rises above 1. Checked-in list: `crates/btx-core/src/pins_ever_shipped.txt`.
- Website pointer: `GET https://easybtx.com/api/snapshots/latest`, contract in `attested_snapshot::ConfirmedPointer` (Task 3). Download hosts: `easybtx.com` and `*.public.blob.vercel-storage.com`, HTTPS, no port, no user info.
- Dependencies: enable only the `ecdsa` feature of `k256` (adds `rfc6979 0.4.0` and `hmac 0.12.1`). Update each lock with `cargo update -p k256 --precise 0.13.4` in `crates/btx-core` and in `apps/node/src-tauri`, never a bare `cargo update`; then `cargo metadata --locked --format-version 1 >/dev/null` must succeed in both.
- CI gates, all must pass before each commit that touches them: in `crates/btx-core` and `apps/node/src-tauri`: `cargo fmt --all --check` and `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, `cargo test --locked`; in `apps/node`: `npx tsc --noEmit`, `npm test`, `npx vite build`.
- Commits end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- User-facing copy: friendly, simple, no hype, no guarantees, no em-dashes.
- Known flake, not caused by this plan: `node::tests::launch_watch_detects_an_immediate_child_death` fails now and then on macOS when the suite runs in parallel (seen at `72304d9` on a clean tree). If it fails, rerun it alone: `cargo test --locked --lib -- --exact node::tests::launch_watch_detects_an_immediate_child_death`.
- The Tauri crate needs `apps/node/src-tauri/resources/node-pkg/` to hold at least one file for `cargo check`/`test` (CI writes `CI-PLACEHOLDER`). If it is empty locally: `mkdir -p apps/node/src-tauri/resources/node-pkg && echo placeholder > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER` (the folder is gitignored).

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `crates/btx-core/Cargo.toml`, `crates/btx-core/Cargo.lock`, `apps/node/src-tauri/Cargo.lock` | modify | `k256` gains `ecdsa` |
| `crates/btx-core/src/operators.rs` | create | The per-chain operator list, key parsing, grouping keys by operator |
| `crates/btx-core/src/confirmed_snapshot.rs` | create | Strict manifest parse, statement hash, file hash, strict-DER low-S verify, operator count, pinned check, trim |
| `crates/btx-core/tests/fixtures/confirmed_snapshot/*` | create | The real 225,927 manifest, the spike's regtest manifests and file, the pointer contract fixture |
| `crates/btx-core/src/attested_snapshot.rs` | modify | Pointer contract, confirmed download, pinned fallback, `prepare_start`; the GitHub single-signed pointer is no longer read |
| `crates/btx-core/src/node_api.rs`, `crates/btx-core/src/role.rs` | modify | `replay_authority_context` on `MatmulTrustedStatus` |
| `crates/btx-core/src/confirmed_load.rs` | create | The one loading path (section 7, steps 3 to 6) |
| `crates/btx-core/src/snapshot.rs` | modify | `LoadOutcome` public, `run_cli_load`, `SignedLoad`, `SnapshotOutcome`, the start-path loader |
| `crates/btx-core/src/node.rs` | modify | Pin rule, mirror-load marker, `host_follows_signatures`, threshold const, pins-only-grow test |
| `crates/btx-core/src/pins_ever_shipped.txt` | create | Every pinned key ever shipped, and the highest threshold |
| `crates/btx-core/src/lib.rs` | modify | Register the new modules |
| `apps/node/src-tauri/src/commands.rs` | modify | Start path: the mirror launch, which load a launch makes, the restart after it |
| `crates/btx-core/tests/confirmed_snapshot_regtest.rs` | create | Opt-in real-engine rehearsal (`EASYNODE_TEST_BTXD`) |
| `scripts/check-engine-tag.sh`, `docs/node-release-recipe.md` | modify | The regtest check before an engine bump |
| `apps/node/CHANGELOG.md` | modify | One entry |

## Tasks

### Task 1: The operator list (`operators.rs`)

**Files:**
- Create: `crates/btx-core/src/operators.rs`
- Modify: `crates/btx-core/src/lib.rs` (module list)

**Interfaces:**
- Consumes: `crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS` (test only).
- Produces (used by Tasks 2, 3, 4, 10 and by the diary/producer/confirmer/website plans):
  - `pub struct Operator { pub name: String, pub keys: Vec<[u8; 33]> }`
  - `pub struct OperatorList` with `new(Vec<Operator>) -> Result<Self, String>`, `operator_of(&self, &[u8; 33]) -> Option<&str>`, `distinct_operators(&self, &[[u8; 33]]) -> Vec<String>`, `len`, `is_empty`, `names`
  - `pub enum Chain { Main, Regtest }`, `Chain::from_genesis_hex(&str) -> Option<Chain>`, `MAINNET_GENESIS`, `REGTEST_GENESIS`
  - `pub const MAINNET_OPERATORS: &[(&str, &[&str])]`, `pub fn mainnet() -> OperatorList`
  - `pub const REGTEST_OPERATORS_ENV: &str = "EASYNODE_REGTEST_OPERATORS"`, `parse_env_list(&str) -> Result<OperatorList, String>`, `for_chain(Chain, Option<&str>) -> OperatorList`, `regtest_env() -> Option<String>`
  - helpers `parse_key(&str) -> Option<[u8; 33]>`, `hex(&[u8]) -> String`, `hex_decode(&str) -> Option<Vec<u8>>`

- [ ] **Step 1: Create the branch**

````bash
cd /Users/m2promende/repos/easynode
git fetch origin
git worktree add ../easynode-confirmed-snapshots -b claude/confirmed-snapshots claude/tools-command-window
cd ../easynode-confirmed-snapshots
git log --oneline -1   # 72304d9 changelog: Tools, or later
````

All paths below are relative to this worktree.

- [ ] **Step 2: Write the failing tests**

Create `crates/btx-core/src/operators.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;

    const MENDE: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";
    const ALEKS_2: &str = "03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24";
    const ALEKS_3: &str = "02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0";
    // Throwaway regtest keys from the spike (P, C, D). Never used anywhere else.
    pub(crate) const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    pub(crate) const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    pub(crate) const D: &str = "034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e";

    fn key(h: &str) -> [u8; 33] {
        parse_key(h).unwrap()
    }

    /// The guard on adding an operator: every line parses into a checked list
    /// with as many operators as lines, and none is lost to a typo.
    #[test]
    fn every_mainnet_line_is_well_formed() {
        let list = mainnet();
        assert_eq!(list.len(), MAINNET_OPERATORS.len(), "a line was refused");
        for (name, keys) in MAINNET_OPERATORS {
            for k in *keys {
                assert_eq!(k.len(), 66, "{name}: {k}");
                assert_eq!(*k, k.to_ascii_lowercase(), "{name}: lowercase hex");
                assert_eq!(list.operator_of(&key(k)), Some(*name), "{name}: {k}");
            }
        }
    }

    /// The first list: Mende alone, with the 3060's key, which every mirror
    /// pins. Aleksander and jpp are not on it until the owner says they agreed.
    #[test]
    fn the_first_mainnet_list_is_mende_with_the_3060() {
        let list = mainnet();
        assert_eq!(list.operator_of(&key(MENDE)), Some("Mende"));
        assert!(crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS.contains(&MENDE));
        assert_eq!(list.operator_of(&key(ALEKS_2)), None);
        assert_eq!(list.operator_of(&key(ALEKS_3)), None);
    }

    /// Aleksander's waiting line is ready to paste: both keys are valid, and
    /// with it the list checks and counts him once.
    #[test]
    fn aleksanders_waiting_line_is_ready() {
        let list = OperatorList::new(vec![
            Operator {
                name: "Mende".into(),
                keys: vec![key(MENDE)],
            },
            Operator {
                name: "Aleksander".into(),
                keys: vec![key(ALEKS_2), key(ALEKS_3)],
            },
        ])
        .unwrap();
        assert_eq!(
            list.distinct_operators(&[key(ALEKS_2), key(ALEKS_3)]),
            vec!["Aleksander".to_string()]
        );
        assert_eq!(
            list.distinct_operators(&[key(ALEKS_3), key(MENDE)]),
            vec!["Mende".to_string(), "Aleksander".to_string()]
        );
    }

    #[test]
    fn a_malformed_list_is_refused() {
        let two = |a: &str, b: &str| {
            OperatorList::new(vec![
                Operator {
                    name: a.into(),
                    keys: vec![key(P)],
                },
                Operator {
                    name: b.into(),
                    keys: vec![key(C)],
                },
            ])
        };
        assert!(two("a", "b").is_ok());
        assert!(two("a", "a").is_err(), "a name twice");
        assert!(two("a", " ").is_err(), "an empty name");
        let shared = OperatorList::new(vec![
            Operator {
                name: "a".into(),
                keys: vec![key(P)],
            },
            Operator {
                name: "b".into(),
                keys: vec![key(P)],
            },
        ]);
        assert!(shared.is_err(), "one key on two operators");
        let keyless = OperatorList::new(vec![Operator {
            name: "a".into(),
            keys: vec![],
        }]);
        assert!(keyless.is_err());
        // 0x02 followed by x = 0 is not on the curve.
        let mut off_curve = [0u8; 33];
        off_curve[0] = 0x02;
        assert!(OperatorList::new(vec![Operator {
            name: "a".into(),
            keys: vec![off_curve],
        }])
        .is_err());
    }

    #[test]
    fn keys_parse_only_as_66_hex_characters_of_a_compressed_point() {
        assert!(parse_key(MENDE).is_some());
        assert!(parse_key(&MENDE.to_ascii_uppercase()).is_some());
        assert!(parse_key(&MENDE[..64]).is_none(), "short");
        assert!(
            parse_key(&format!("04{}", &MENDE[2..])).is_none(),
            "not compressed"
        );
        assert!(
            parse_key(&format!("{}zz", &MENDE[..64])).is_none(),
            "not hex"
        );
    }

    #[test]
    fn the_test_list_format_parses_and_refuses() {
        let list = parse_env_list(&format!("producer={P};confirmer={C},{D}")).unwrap();
        assert_eq!(list.names(), vec!["producer", "confirmer"]);
        assert_eq!(list.operator_of(&key(D)), Some("confirmer"));
        assert_eq!(
            list.distinct_operators(&[key(C), key(D)]),
            vec!["confirmer".to_string()],
            "two keys of one operator count once"
        );
        assert!(parse_env_list("producer").is_err());
        assert!(parse_env_list(&format!("producer={}", &P[..10])).is_err());
        assert!(parse_env_list(&format!("a={P};b={P}")).is_err());
        assert!(parse_env_list("").unwrap().is_empty());
    }

    /// A mainnet statement never reads the environment: a test list naming
    /// any keys at all leaves the mainnet list exactly the compiled one.
    #[test]
    fn the_environment_never_reaches_mainnet() {
        let env = format!("a={P};b={C};c={MENDE}");
        assert_eq!(for_chain(Chain::Main, Some(&env)), mainnet());
        assert_eq!(for_chain(Chain::Main, None), mainnet());
        assert_eq!(for_chain(Chain::Regtest, Some(&env)).len(), 3);
        assert!(for_chain(Chain::Regtest, None).is_empty());
        assert!(for_chain(Chain::Regtest, Some("broken")).is_empty());
    }

    #[test]
    fn chains_are_named_by_their_genesis_hash() {
        assert_eq!(Chain::from_genesis_hex(MAINNET_GENESIS), Some(Chain::Main));
        assert_eq!(
            Chain::from_genesis_hex(&MAINNET_GENESIS.to_ascii_uppercase()),
            Some(Chain::Main)
        );
        assert_eq!(
            Chain::from_genesis_hex(REGTEST_GENESIS),
            Some(Chain::Regtest)
        );
        assert_eq!(Chain::from_genesis_hex(&"00".repeat(32)), None);
    }
}
````

In `crates/btx-core/src/lib.rs`, add the module in alphabetical order, after `pub mod node_api;`:

````rust
pub mod node_api;
pub mod operators;
````

- [ ] **Step 3: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- operators`
Expected: compile errors such as `cannot find function \`mainnet\` in this scope` and `cannot find type \`OperatorList\``.

- [ ] **Step 4: Write the implementation**

Insert above `#[cfg(test)]` in `crates/btx-core/src/operators.rs`:

````rust
//! Who may confirm a chain snapshot, and how their keys group.
//!
//! A snapshot counts as confirmed only when two different operators on this
//! list signed its statement (`crate::confirmed_snapshot`). An operator is a
//! person, and all of one person's keys together count once, so one machine
//! with two keys can never confirm a snapshot alone.
//!
//! The mainnet list is compiled in and changes only with a signed app update.
//! Test chains get their list from the environment, and only test chains: a
//! statement names its chain by the chain's genesis hash, and a mainnet
//! statement is always checked against the compiled list, whatever the
//! environment says. docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! section 1.

/// One operator: a name for the log and every key they sign with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    pub name: String,
    pub keys: Vec<[u8; 33]>,
}

/// A checked list: names unique and not empty, every key a valid compressed
/// secp256k1 point, no key on two operators or twice on one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorList {
    operators: Vec<Operator>,
}

impl OperatorList {
    pub fn new(operators: Vec<Operator>) -> Result<Self, String> {
        let mut seen_names: Vec<&str> = Vec::new();
        let mut seen_keys: Vec<&[u8; 33]> = Vec::new();
        for op in &operators {
            let name = op.name.trim();
            if name.is_empty() {
                return Err("an operator has no name".into());
            }
            if seen_names.contains(&name) {
                return Err(format!("operator {name} is listed twice"));
            }
            seen_names.push(name);
            if op.keys.is_empty() {
                return Err(format!("operator {name} has no key"));
            }
            for key in &op.keys {
                if !is_compressed_point(key) {
                    return Err(format!(
                        "operator {name} has a key that is not a compressed secp256k1 point"
                    ));
                }
                if seen_keys.contains(&key) {
                    return Err(format!("key {} is listed twice", hex(key)));
                }
                seen_keys.push(key);
            }
        }
        Ok(Self { operators })
    }

    /// The operator a key belongs to, if any.
    pub fn operator_of(&self, key: &[u8; 33]) -> Option<&str> {
        self.operators
            .iter()
            .find(|op| op.keys.contains(key))
            .map(|op| op.name.as_str())
    }

    /// The operators behind `keys`, each named once, in list order. Keys not
    /// on the list are ignored. Two keys of one operator give one name.
    pub fn distinct_operators(&self, keys: &[[u8; 33]]) -> Vec<String> {
        self.operators
            .iter()
            .filter(|op| op.keys.iter().any(|k| keys.contains(k)))
            .map(|op| op.name.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.operators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.operators.iter().map(|op| op.name.as_str()).collect()
    }
}

/// The chains a statement can name, by the chain's genesis hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    Main,
    Regtest,
}

/// Mainnet's genesis hash, display order. Read from the published 225,927
/// statement's `chain_id` on 2026-09-29.
pub const MAINNET_GENESIS: &str =
    "75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601";

/// Regtest's genesis hash, display order. Read from the regtest statements the
/// spike of 2026-09-29 exported with v0.34.9.
pub const REGTEST_GENESIS: &str =
    "521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360";

impl Chain {
    /// The chain a genesis hash (display order, any case) names, if either.
    pub fn from_genesis_hex(display_hex: &str) -> Option<Chain> {
        let h = display_hex.to_ascii_lowercase();
        if h == MAINNET_GENESIS {
            Some(Chain::Main)
        } else if h == REGTEST_GENESIS {
            Some(Chain::Regtest)
        } else {
            None
        }
    }
}

/// The mainnet list, one line per operator. Adding an operator is adding one
/// line; `every_mainnet_line_is_well_formed` guards it.
///
/// Waiting for the owner's word that they agreed (2026-09-29), and not to be
/// added before it: Aleksander, one operator with two keys,
/// `("Aleksander", &["03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24", "02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0"]),`
/// and jpp, once he has sent a key. While Mende is alone here nothing counts
/// as confirmed and every node uses the fallbacks.
pub const MAINNET_OPERATORS: &[(&str, &[&str])] = &[(
    "Mende",
    &["02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"],
)];

/// Where a test chain's list comes from. Format, operators separated by `;`,
/// each `name=key[,key...]` with keys as 66 hex characters:
///
/// ```text
/// EASYNODE_REGTEST_OPERATORS="producer=0343fa...e464;confirmer=02c05d...6c0f"
/// ```
///
/// Read only for a regtest statement. A mainnet statement never consults it.
pub const REGTEST_OPERATORS_ENV: &str = "EASYNODE_REGTEST_OPERATORS";

/// The compiled mainnet list. Empty, which confirms nothing, if a line is
/// malformed; the test above makes that impossible to ship.
pub fn mainnet() -> OperatorList {
    let mut ops = Vec::new();
    for (name, keys) in MAINNET_OPERATORS {
        let mut parsed = Vec::new();
        for k in *keys {
            match parse_key(k) {
                Some(key) => parsed.push(key),
                None => {
                    eprintln!("[operators] mainnet key {k} is malformed; no operator counts");
                    return OperatorList::default();
                }
            }
        }
        ops.push(Operator {
            name: name.to_string(),
            keys: parsed,
        });
    }
    OperatorList::new(ops).unwrap_or_else(|e| {
        eprintln!("[operators] the mainnet list is malformed ({e}); no operator counts");
        OperatorList::default()
    })
}

/// Parse [`REGTEST_OPERATORS_ENV`]'s value.
pub fn parse_env_list(raw: &str) -> Result<OperatorList, String> {
    let mut ops = Vec::new();
    for part in raw.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, keys) = part
            .split_once('=')
            .ok_or_else(|| format!("{part:?} is not name=key[,key]"))?;
        let mut parsed = Vec::new();
        for k in keys.split(',').map(str::trim) {
            parsed.push(parse_key(k).ok_or_else(|| format!("{k:?} is not a 66-hex key"))?);
        }
        ops.push(Operator {
            name: name.trim().to_string(),
            keys: parsed,
        });
    }
    OperatorList::new(ops)
}

/// The list a statement on `chain` is checked against. `regtest_env` is the
/// value of [`REGTEST_OPERATORS_ENV`] as the caller read it; for
/// [`Chain::Main`] it is never looked at. A malformed test list is empty,
/// which confirms nothing.
pub fn for_chain(chain: Chain, regtest_env: Option<&str>) -> OperatorList {
    match chain {
        Chain::Main => mainnet(),
        Chain::Regtest => match regtest_env.map(parse_env_list) {
            Some(Ok(list)) => list,
            Some(Err(e)) => {
                eprintln!("[operators] {REGTEST_OPERATORS_ENV} refused: {e}");
                OperatorList::default()
            }
            None => OperatorList::default(),
        },
    }
}

/// [`REGTEST_OPERATORS_ENV`] as this process sees it.
pub fn regtest_env() -> Option<String> {
    std::env::var(REGTEST_OPERATORS_ENV).ok()
}

/// A 66-hex compressed key, or `None`.
pub fn parse_key(hex_key: &str) -> Option<[u8; 33]> {
    let bytes = hex_decode(hex_key.trim())?;
    let key: [u8; 33] = bytes.try_into().ok()?;
    is_compressed_point(&key).then_some(key)
}

fn is_compressed_point(key: &[u8; 33]) -> bool {
    matches!(key[0], 0x02 | 0x03) && k256::PublicKey::from_sec1_bytes(key).is_ok()
}

/// Lowercase hex of any bytes.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex (either case) to bytes; `None` on an odd length or a non-hex character.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
````

- [ ] **Step 5: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- operators`
Expected: `test result: ok. 8 passed; 0 failed`.

- [ ] **Step 6: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````

````bash
git add crates/btx-core/src/operators.rs crates/btx-core/src/lib.rs
git commit -m "core: the operator list, Mende alone until another operator agrees" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: Read, verify, count and trim a manifest (`confirmed_snapshot.rs`)

**Files:**
- Modify: `crates/btx-core/Cargo.toml` (k256 features), `crates/btx-core/Cargo.lock`, `apps/node/src-tauri/Cargo.lock`
- Create: `crates/btx-core/src/confirmed_snapshot.rs`
- Create: `crates/btx-core/tests/fixtures/confirmed_snapshot/` (7 binary vectors, 1 data file)
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 (`operators::{self, Chain, OperatorList, Operator}`), `crate::attested_snapshot::MAX_SNAPSHOT_BYTES` (exists), `crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS` (test only).
- Produces (Tasks 3, 4, 10; the producer, confirmer and website plans):
  - consts `STATEMENT_VERSION`, `STATEMENT_LEN` (229), `MAX_MANIFEST_BYTES` (65,536), `MAX_SIGNATURES` (64), `MAX_DER_LEN` (72), `MAINNET_REPLAY_CONTEXT`, `REGTEST_REPLAY_CONTEXT`, `MAINNET_GRID` (200), `REGTEST_GRID` (100)
  - `pub struct Hash32(pub [u8; 32])` with `display_hex`, `from_display_hex`, `is_null`
  - `pub enum Refusal` (every reason, `Display` = one log line)
  - `pub struct Statement` (`from_raw`, `raw`, `version`, `chain_id`, `block_hash`, `height`, `hash_serialized`, `coins`, `chain_tx`, `shielded`, `replay_context`, `file_size`, `file_hash`, `chunk_size`, `chunk_count`, `hash`)
  - `pub struct Signed { key: [u8; 33], der: Vec<u8> }`, `pub struct Manifest { statement, signatures }` with `to_bytes()`
  - `pub fn parse(&[u8]) -> Result<Manifest, Refusal>`
  - `pub fn is_strict_der(&[u8]) -> bool`, `pub fn signature_is_valid(&Hash32, &[u8; 33], &[u8]) -> bool`
  - `pub fn confirming_operators(&Manifest, &OperatorList) -> Result<Vec<String>, Refusal>`
  - `pub struct ChainRules` + `ChainRules::for_statement(&Statement, Option<&str>)`; `pub struct NodeView { genesis: Option<String>, replay_context: Option<String>, start_height: u64, pinned: Vec<[u8; 33]> }` (`Default`)
  - `pub struct Confirmed { manifest, chain, height: u64, statement_hash: Hash32, operators: Vec<String> }`
  - `pub fn check(&Manifest, &NodeView, Option<&str>) -> Result<Confirmed, Refusal>`, `pub fn check_with(&Manifest, &NodeView, &ChainRules)`, `pub fn node_agrees(&Statement, &NodeView) -> Result<(), Refusal>`
  - `pub fn trim_to_pinned(&Manifest, &[[u8; 33]]) -> Manifest`
  - `pub struct FileHasher` (`update`, `finish() -> (u64, String plain_sha256_hex, Hash32 double)`), `pub fn file_matches(&Statement, u64, &Hash32) -> bool`, `pub fn pinned_keys(&[&str]) -> Vec<[u8; 33]>`

- [ ] **Step 1: Enable k256's `ecdsa` feature, with a targeted lock update**

In `crates/btx-core/Cargo.toml` change the k256 line to:

````toml
k256 = { version = "0.13", default-features = false, features = ["arithmetic", "ecdsa", "std"] }
````

Then:

````bash
(cd crates/btx-core && cargo update -p k256 --precise 0.13.4)
(cd apps/node/src-tauri && cargo update -p k256 --precise 0.13.4)
(cd crates/btx-core && cargo metadata --locked --format-version 1 >/dev/null && echo core-locked-ok)
(cd apps/node/src-tauri && cargo metadata --locked --format-version 1 >/dev/null && echo app-locked-ok)
git diff --stat -- '*Cargo.lock'
````

Expected: each `cargo update` prints `Adding hmac v0.12.1` and `Adding rfc6979 v0.4.0` and nothing else; both `*-locked-ok` lines print; each lock file gains about 25 lines. If any other crate moves, stop and `git checkout -- '*Cargo.lock'`.

- [ ] **Step 2: Add the test vectors**

````bash
D=crates/btx-core/tests/fixtures/confirmed_snapshot
S=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad
mkdir -p $D
cp $S/mainnet/pair/snapshot-manifest-225927.json $D/mainnet-225927.manifest
for m in P PC PCD CD C CP; do cp $S/spike/m/$m.manifest $D/regtest-$m.manifest; done
cp $S/spike/m/snap.dat $D/regtest-100.dat
(cd $D && shasum -a 256 *)
````

If the scratchpad copy of the mainnet manifest is gone, fetch the published one: `gh release download utxo-snapshot-225927 --repo MendeMatthias/EasyBTX-releases --pattern snapshot-manifest-225927.json -O $D/mainnet-225927.manifest`. The regtest files exist only in the scratchpad; if they are gone, stop and ask (they carry the spike's signatures). Expected sums, exactly:

````text
8adc90c2b4514334d0bc0e1dafa5f3bc85ed0cfcc55d051a586e117794e332ed  mainnet-225927.manifest
b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85  regtest-100.dat
691e427d899ed18c0701a20422e908d35a4e29fdf9fa17f8eb484f256525cb6f  regtest-C.manifest
3ce5d526dfe971d736b3d9ce3efc27ad7111c60e3ee42e9952bc61e1b89ac8fd  regtest-CD.manifest
2a77b16882ea2ee988882e55b19cb20305f3202dd85ef10157bade072387fbc2  regtest-CP.manifest
eac1e450d1aa22cdadb8472f7b1dec80d6af62fbb04387b6d49e726abdcc2d4a  regtest-P.manifest
e8fa06d2f700feb488d7f494a532cd0e44799d20613b80532b2eca5cffaa746e  regtest-PC.manifest
649ce13ffb22f57358253530d187a6d3dc3679d4bc5649acb77dc6a6d22e5b09  regtest-PCD.manifest
````

What they are: the manifest published with the 225,927 pair (signed by `02d5efca`, the 3060); and the spike's regtest export at height 100 (producer P, then confirmers C and D, throwaway keys P `0343faeb…e464`, C `02c05d68…6c0f`, D `034694ab…6d0e`), with its 8,055-byte snapshot file. `regtest-P.manifest` is also what trimming PCD to P gives, byte for byte.

- [ ] **Step 3: Write the failing tests**

Create `crates/btx-core/src/confirmed_snapshot.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::operators::Operator;
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};

    const MAINNET: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/mainnet-225927.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_PCD: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PCD.manifest");
    const R_CD: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-CD.manifest");
    const R_C: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-C.manifest");
    const R_CP: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-CP.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");

    const MENDE: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";
    // The spike's throwaway regtest keys. Never used anywhere else.
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    const D: &str = "034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e";

    fn key(h: &str) -> [u8; 33] {
        operators::parse_key(h).unwrap()
    }

    fn regtest_env() -> String {
        format!("producer={P};confirmer={C};third={D}")
    }

    fn regtest_view(pinned: &[&str]) -> NodeView {
        NodeView {
            genesis: Some(operators::REGTEST_GENESIS.into()),
            replay_context: Some(REGTEST_REPLAY_CONTEXT.into()),
            start_height: 0,
            pinned: pinned.iter().map(|h| key(h)).collect(),
        }
    }

    // ── the real vectors ────────────────────────────────────────────────

    #[test]
    fn the_published_225927_statement_reads_as_the_independent_check_read_it() {
        let m = parse(MAINNET).unwrap();
        let st = &m.statement;
        assert_eq!(st.version(), 2);
        assert_eq!(st.chain_id().display_hex(), operators::MAINNET_GENESIS);
        assert_eq!(
            st.block_hash().display_hex(),
            "06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932"
        );
        assert_eq!(st.height(), 225_927);
        assert_eq!(
            st.hash_serialized().display_hex(),
            "79435348a3ff8bc8c07bd58603d18439fe29f0b629f9d38695d0c824be9439a8"
        );
        assert_eq!((st.coins(), st.chain_tx()), (140_731, 328_195));
        assert_eq!(st.replay_context().display_hex(), MAINNET_REPLAY_CONTEXT);
        assert_eq!(
            (st.file_size(), st.chunk_size(), st.chunk_count()),
            (9_045_522, 1_048_576, 9)
        );
        assert_eq!(
            st.file_hash().display_hex(),
            "f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c"
        );
        assert_eq!(
            st.hash().display_hex(),
            "d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482"
        );
        assert_eq!(m.signatures.len(), 1);
        assert_eq!(m.signatures[0].key, key(MENDE));
        assert!(signature_is_valid(
            &st.hash(),
            &m.signatures[0].key,
            &m.signatures[0].der
        ));
        assert_eq!(m.to_bytes(), MAINNET, "reserialized byte for byte");
    }

    /// Signed by the 3060 alone: one operator, so not confirmed, and off the
    /// grid besides. It stays the pinned fallback, trusted by its compiled
    /// hashes, not by this check.
    #[test]
    fn the_published_pair_is_one_operator_and_is_not_confirmed() {
        let m = parse(MAINNET).unwrap();
        assert_eq!(
            confirming_operators(&m, &operators::mainnet()).unwrap(),
            vec!["Mende".to_string()]
        );
        let view = NodeView {
            pinned: pinned_keys(&crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS),
            ..NodeView::default()
        };
        assert_eq!(
            check(&m, &view, None).unwrap_err(),
            Refusal::OffGrid {
                height: 225_927,
                grid: 200
            }
        );
    }

    #[test]
    fn the_regtest_vectors_verify_and_the_file_matches() {
        for bytes in [R_P, R_PC, R_PCD, R_CD, R_C, R_CP] {
            let m = parse(bytes).unwrap();
            assert_eq!(m.to_bytes(), bytes);
            assert_eq!(
                m.statement.hash().display_hex(),
                "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194"
            );
            assert_eq!(
                m.statement.replay_context().display_hex(),
                REGTEST_REPLAY_CONTEXT
            );
            for s in &m.signatures {
                assert!(signature_is_valid(&m.statement.hash(), &s.key, &s.der));
            }
        }
        let st = parse(R_P).unwrap().statement;
        let mut h = FileHasher::default();
        h.update(&R_DAT[..1000]);
        h.update(&R_DAT[1000..]);
        let (len, plain, double) = h.finish();
        assert_eq!(
            plain,
            "b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85"
        );
        assert!(file_matches(&st, len, &double));
        let mut h = FileHasher::default();
        h.update(&R_DAT[..R_DAT.len() - 1]);
        let (len, _, double) = h.finish();
        assert!(!file_matches(&st, len, &double), "one byte short");
        let mut changed = R_DAT.to_vec();
        changed[100] ^= 1;
        let mut h = FileHasher::default();
        h.update(&changed);
        let (len, _, double) = h.finish();
        assert!(!file_matches(&st, len, &double), "same size, one bit off");
    }

    // ── the parser ──────────────────────────────────────────────────────

    #[test]
    fn a_manifest_is_read_strictly() {
        assert_eq!(parse(&R_PC[..R_PC.len() - 1]), Err(Refusal::Truncated));
        assert_eq!(parse(&R_PC[..200]), Err(Refusal::Truncated));
        assert_eq!(parse(&[]), Err(Refusal::Truncated));
        let mut extra = R_PC.to_vec();
        extra.push(0);
        assert_eq!(parse(&extra), Err(Refusal::TrailingBytes(1)));
        let mut v1 = R_P.to_vec();
        v1[0] = 1;
        assert_eq!(parse(&v1), Err(Refusal::UnsupportedVersion(1)));
        let big = vec![2u8; MAX_MANIFEST_BYTES + 1];
        assert_eq!(parse(&big), Err(Refusal::TooLarge(MAX_MANIFEST_BYTES + 1)));
    }

    #[test]
    fn an_oversized_or_oddly_written_signature_count_is_refused() {
        let st = &R_P[..STATEMENT_LEN];
        // 10,000 signatures, written canonically.
        let mut many = st.to_vec();
        many.extend_from_slice(&[253, 0x10, 0x27]);
        assert_eq!(parse(&many), Err(Refusal::TooManySignatures(10_000)));
        // One signature, written the long way.
        let mut odd = st.to_vec();
        odd.extend_from_slice(&[253, 1, 0]);
        odd.extend_from_slice(&R_P[STATEMENT_LEN + 1..]);
        assert_eq!(parse(&odd), Err(Refusal::NonCanonicalSize));
        // A count the bytes cannot hold.
        let mut short = st.to_vec();
        short.push(5);
        short.extend_from_slice(&R_P[STATEMENT_LEN + 1..]);
        assert_eq!(parse(&short), Err(Refusal::Truncated));
    }

    #[test]
    fn keys_are_33_bytes_and_signatures_at_most_72() {
        let mut key65 = R_P.to_vec();
        key65[STATEMENT_LEN + 1] = 65;
        assert!(matches!(parse(&key65), Err(Refusal::Malformed(_))));
        let mut long_sig = R_P[..STATEMENT_LEN + 1 + 1 + 33].to_vec();
        long_sig.push(73);
        long_sig.extend_from_slice(&[0x30; 73]);
        assert!(matches!(parse(&long_sig), Err(Refusal::Malformed(_))));
    }

    // ── signatures ──────────────────────────────────────────────────────

    fn signer(n: u8) -> SigningKey {
        SigningKey::from_slice(&[n; 32]).unwrap()
    }

    fn pubkey(sk: &SigningKey) -> [u8; 33] {
        sk.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap()
    }

    fn sign(sk: &SigningKey, st: &Statement) -> Signed {
        let sig: Signature = sk.sign_prehash(&st.hash().0).unwrap();
        Signed {
            key: pubkey(sk),
            der: sig.to_der().as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_high_s_signature_is_refused() {
        let m = parse(R_P).unwrap();
        let hash = m.statement.hash();
        let low = Signature::from_der(&m.signatures[0].der).unwrap();
        let high = Signature::from_scalars(low.r().to_bytes(), (-*low.s()).to_bytes()).unwrap();
        let der = high.to_der().as_bytes().to_vec();
        assert!(is_strict_der(&der), "still canonical DER, only S is high");
        assert!(!signature_is_valid(&hash, &m.signatures[0].key, &der));
        let mut bad = m.clone();
        bad.signatures[0].der = der;
        assert_eq!(
            confirming_operators(&bad, &OperatorList::default()),
            Err(Refusal::InvalidSignature(P.into()))
        );
    }

    #[test]
    fn a_signature_that_is_not_strict_der_is_refused() {
        let m = parse(R_P).unwrap();
        let hash = m.statement.hash();
        let good = m.signatures[0].der.clone();
        assert!(is_strict_der(&good));
        // A needless leading zero on R, lengths adjusted: BER, not DER.
        let len_r = good[3] as usize;
        let mut padded = vec![0x30, good[1] + 1, 0x02, good[3] + 1, 0x00];
        padded.extend_from_slice(&good[4..4 + len_r]);
        padded.extend_from_slice(&good[4 + len_r..]);
        assert!(!is_strict_der(&padded));
        assert!(!signature_is_valid(&hash, &m.signatures[0].key, &padded));
        // A wrong outer length.
        let mut wrong_len = good.clone();
        wrong_len[1] += 1;
        assert!(!is_strict_der(&wrong_len));
        // A sighash byte on the end.
        let mut sighash = good.clone();
        sighash.push(0x01);
        assert!(!is_strict_der(&sighash));
    }

    #[test]
    fn the_right_signature_under_the_wrong_key_is_refused() {
        let m = parse(R_PC).unwrap();
        let hash = m.statement.hash();
        assert!(!signature_is_valid(
            &hash,
            &m.signatures[1].key,
            &m.signatures[0].der
        ));
        let mut swapped = m.clone();
        swapped.signatures[0].key = m.signatures[1].key;
        swapped.signatures[1].key = m.signatures[0].key;
        assert!(matches!(
            confirming_operators(&swapped, &OperatorList::default()),
            Err(Refusal::InvalidSignature(_))
        ));
        // And a signature over another statement.
        let mainnet = parse(MAINNET).unwrap();
        assert!(!signature_is_valid(
            &mainnet.statement.hash(),
            &m.signatures[0].key,
            &m.signatures[0].der
        ));
    }

    #[test]
    fn a_key_that_signed_twice_is_refused() {
        let mut m = parse(R_PC).unwrap();
        m.signatures.push(m.signatures[0].clone());
        assert_eq!(
            confirming_operators(&m, &OperatorList::default()),
            Err(Refusal::DuplicateSigner(P.into()))
        );
    }

    // ── counting operators and the whole check ─────────────────────────

    #[test]
    fn two_regtest_operators_confirm_and_one_does_not() {
        let env = regtest_env();
        let view = regtest_view(&[P]);
        let c = check(&parse(R_PC).unwrap(), &view, Some(&env)).unwrap();
        assert_eq!(c.height, 100);
        assert_eq!(c.chain, Chain::Regtest);
        assert_eq!(
            c.operators,
            vec!["producer".to_string(), "confirmer".to_string()]
        );
        assert_eq!(
            check(&parse(R_P).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["producer".into()]))
        );
        // Signed by two operators, but none this node pins.
        assert_eq!(
            check(&parse(R_CD).unwrap(), &view, Some(&env)),
            Err(Refusal::NoPinnedSigner)
        );
    }

    #[test]
    fn two_keys_of_one_operator_count_once() {
        let env = format!("producer={P};confirmer={C},{D}");
        let view = regtest_view(&[C]);
        assert_eq!(
            check(&parse(R_CD).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["confirmer".into()]))
        );
    }

    #[test]
    fn an_unknown_key_counts_for_nobody() {
        let env = format!("producer={P};confirmer={C}");
        let view = regtest_view(&[P]);
        // D is on no list: P and C still confirm, D adds nothing.
        let c = check(&parse(R_PCD).unwrap(), &view, Some(&env)).unwrap();
        assert_eq!(c.operators.len(), 2);
        // With only P on the list, C and D count for nobody: one operator.
        let env = format!("producer={P}");
        assert_eq!(
            check(&parse(R_PCD).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["producer".into()]))
        );
    }

    /// The test list can never validate a mainnet statement: a statement on
    /// mainnet's chain id, signed by two keys that the environment names as
    /// two operators, is checked against the compiled list and has none.
    #[test]
    fn a_regtest_list_never_validates_a_mainnet_statement() {
        let (a, b) = (signer(7), signer(8));
        let mut raw = *parse(MAINNET).unwrap().statement.raw();
        raw[65..69].copy_from_slice(&232_000i32.to_le_bytes());
        let st = Statement::from_raw(raw);
        let m = Manifest {
            signatures: vec![sign(&a, &st), sign(&b, &st)],
            statement: st,
        };
        let env = format!(
            "a={};b={}",
            operators::hex(&pubkey(&a)),
            operators::hex(&pubkey(&b))
        );
        let view = NodeView {
            pinned: vec![pubkey(&a), pubkey(&b)],
            ..NodeView::default()
        };
        assert_eq!(
            check(&m, &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec![]))
        );
        let rules = ChainRules::for_statement(&m.statement, Some(&env)).unwrap();
        assert_eq!(rules.chain, Chain::Main);
        assert_eq!(rules.operators, operators::mainnet());
    }

    /// Mainnet-shaped rules with two test operators, so every rule can be
    /// broken one at a time on a statement that otherwise passes.
    struct Mainnetish {
        a: SigningKey,
        b: SigningKey,
        rules: ChainRules,
        view: NodeView,
    }

    impl Mainnetish {
        fn new() -> Self {
            let (a, b) = (signer(7), signer(8));
            let list = OperatorList::new(vec![
                Operator {
                    name: "a".into(),
                    keys: vec![pubkey(&a)],
                },
                Operator {
                    name: "b".into(),
                    keys: vec![pubkey(&b)],
                },
            ])
            .unwrap();
            let mut rules =
                ChainRules::for_statement(&parse(MAINNET).unwrap().statement, None).unwrap();
            rules.operators = list;
            let view = NodeView {
                genesis: Some(operators::MAINNET_GENESIS.into()),
                replay_context: Some(MAINNET_REPLAY_CONTEXT.into()),
                start_height: 225_927,
                pinned: vec![pubkey(&a)],
            };
            Self { a, b, rules, view }
        }

        /// The published statement at `height`, changed by `edit`, signed by both.
        fn manifest(&self, height: i32, edit: impl Fn(&mut [u8; STATEMENT_LEN])) -> Manifest {
            let mut raw = *parse(MAINNET).unwrap().statement.raw();
            raw[65..69].copy_from_slice(&height.to_le_bytes());
            edit(&mut raw);
            let st = Statement::from_raw(raw);
            Manifest {
                signatures: vec![sign(&self.a, &st), sign(&self.b, &st)],
                statement: st,
            }
        }

        fn check(&self, m: &Manifest) -> Result<Confirmed, Refusal> {
            check_with(m, &self.view, &self.rules)
        }
    }

    #[test]
    fn a_statement_that_breaks_no_rule_is_confirmed() {
        let t = Mainnetish::new();
        let c = t.check(&t.manifest(232_000, |_| {})).unwrap();
        assert_eq!(c.height, 232_000);
        assert_eq!(c.operators, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn each_rule_refuses_on_its_own() {
        let t = Mainnetish::new();
        let off = t.manifest(232_100, |_| {});
        assert_eq!(
            t.check(&off),
            Err(Refusal::OffGrid {
                height: 232_100,
                grid: 200
            })
        );
        let low = t.manifest(225_800, |_| {});
        assert_eq!(
            t.check(&low),
            Err(Refusal::NotAboveStart {
                height: 225_800,
                start: 225_927
            })
        );
        let negative = t.manifest(-200, |_| {});
        assert!(matches!(t.check(&negative), Err(Refusal::OffGrid { .. })));
        let chain = t.manifest(232_000, |r| r[1] ^= 1);
        assert!(matches!(t.check(&chain), Err(Refusal::UnknownChain(_))));
        let replay = t.manifest(232_000, |r| r[149] ^= 1);
        assert_eq!(t.check(&replay), Err(Refusal::WrongReplayContext));
        let shielded = t.manifest(232_000, |r| r[117..149].fill(0));
        assert_eq!(t.check(&shielded), Err(Refusal::MissingShieldedCommitment));
        let chunks = t.manifest(232_000, |r| {
            r[225..229].copy_from_slice(&8u32.to_le_bytes())
        });
        assert_eq!(t.check(&chunks), Err(Refusal::BadGeometry));
        let huge = t.manifest(232_000, |r| {
            let size: u64 = 65 * 1024 * 1024;
            r[181..189].copy_from_slice(&size.to_le_bytes());
            r[225..229].copy_from_slice(&65u32.to_le_bytes());
        });
        assert_eq!(t.check(&huge), Err(Refusal::FileTooLarge(65 * 1024 * 1024)));
        let one = {
            let mut m = t.manifest(232_000, |_| {});
            m.signatures.pop();
            m
        };
        assert_eq!(
            t.check(&one),
            Err(Refusal::TooFewOperators(vec!["a".into()]))
        );
        let unpinned = {
            let mut t2 = Mainnetish::new();
            t2.view.pinned = vec![key(MENDE)];
            t2.check(&t2.manifest(232_000, |_| {}))
        };
        assert_eq!(unpinned, Err(Refusal::NoPinnedSigner));
    }

    #[test]
    fn the_running_node_must_agree_on_chain_and_replay_context() {
        let mut t = Mainnetish::new();
        let m = t.manifest(232_000, |_| {});
        t.view.genesis = Some(operators::REGTEST_GENESIS.into());
        assert_eq!(t.check(&m), Err(Refusal::NodeOnAnotherChain));
        t.view.genesis = Some(operators::MAINNET_GENESIS.into());
        t.view.replay_context = Some(REGTEST_REPLAY_CONTEXT.into());
        assert_eq!(t.check(&m), Err(Refusal::NodeReplayContextDiffers));
        // A node with no pin and no key reports no context: the compiled one decides.
        t.view.replay_context = None;
        assert!(t.check(&m).is_ok());
    }

    #[test]
    fn an_invalid_signature_from_an_unlisted_key_still_refuses_the_whole_manifest() {
        let t = Mainnetish::new();
        let mut m = t.manifest(232_000, |_| {});
        let stranger = signer(9);
        let mut bad = sign(&stranger, &m.statement);
        bad.der = m.signatures[0].der.clone();
        m.signatures.push(bad);
        assert!(matches!(t.check(&m), Err(Refusal::InvalidSignature(_))));
    }

    // ── trimming ────────────────────────────────────────────────────────

    #[test]
    fn trimming_keeps_every_pinned_signature_in_order_and_gives_the_producers_file_back() {
        let pcd = parse(R_PCD).unwrap();
        assert_eq!(
            trim_to_pinned(&pcd, &[key(P)]).to_bytes(),
            R_P,
            "byte for byte"
        );
        let pc_only = trim_to_pinned(&pcd, &[key(C), key(P)]);
        assert_eq!(
            pc_only.to_bytes(),
            R_PC,
            "order is the manifest's, not the pins'"
        );
        let cp = parse(R_CP).unwrap();
        assert_eq!(trim_to_pinned(&cp, &[key(P), key(C)]).to_bytes(), R_CP);
        assert!(trim_to_pinned(&pcd, &[key(MENDE)]).signatures.is_empty());
    }

    #[test]
    fn hashes_round_trip_through_display_order() {
        let h = Hash32::from_display_hex(MAINNET_REPLAY_CONTEXT).unwrap();
        assert_eq!(h.display_hex(), MAINNET_REPLAY_CONTEXT);
        assert!(Hash32::from_display_hex("00").is_none());
        assert!(Hash32([0; 32]).is_null());
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod checkin;`:

````rust
pub mod checkin;
pub mod confirmed_snapshot;
````

- [ ] **Step 4: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- confirmed_snapshot`
Expected: compile errors, among them `cannot find function `parse` in this scope` and `cannot find type `Statement``.

- [ ] **Step 5: Write the implementation**

Insert above `#[cfg(test)]` in `crates/btx-core/src/confirmed_snapshot.rs`:

````rust
//! Read, check and trim a signed snapshot manifest before the engine sees it.
//!
//! WHAT A MANIFEST IS. The engine's `UtxoSnapshotManifest` (v0.34.9,
//! `src/matmul/trusted_utxo_snapshot_attestation.h`): a 229-byte version-2
//! statement, then a compact-size count of signatures, each a compact-size
//! length and a 33-byte compressed key, then a compact-size length and a
//! strict-DER signature. Every 32-byte field is stored little-endian and shown
//! reversed, as the engine's `GetHex` shows it.
//!
//! WHAT CONFIRMED MEANS. Every signature is strict DER, low-S and valid over
//! the statement; signers grouped by operator (`crate::operators`) number at
//! least two; at least one signature is from a key this node pins; and the
//! statement names this chain, this engine's replay context, a height on the
//! grid and above where the node would start anyway, and a file this app will
//! download. docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! sections 1, 7 and 8.
//!
//! WHY TRIM. The engine refuses the whole manifest when any signature is from
//! a key it does not pin (`untrusted-signer`, measured 2026-09-29), and a
//! signature covers only the statement, so [`trim_to_pinned`] drops the rest.
//! Dropping them gave back the producer's own file byte for byte.

use crate::operators::{self, Chain, OperatorList};
use sha2::{Digest, Sha256};

pub const STATEMENT_VERSION: u8 = 2;
pub const STATEMENT_LEN: usize = 229;
/// The engine's cap on a manifest.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Far more than any list will hold; a count above it is not a manifest.
pub const MAX_SIGNATURES: u64 = 64;
/// `CPubKey::SIGNATURE_SIZE`.
pub const MAX_DER_LEN: usize = 72;
const HASH_DOMAIN: &[u8] = b"BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2";
const MIN_CHUNK: u32 = 64 * 1024;
const MAX_CHUNK: u32 = 4 * 1024 * 1024;

/// Mainnet's replay authority context on v0.34.9, display order. Read from
/// the published 225,927 statement on 2026-09-29. An engine bump that moves it
/// fails `confirmed_snapshot_regtest`'s mainnet check before it ships.
pub const MAINNET_REPLAY_CONTEXT: &str =
    "32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188";
/// Regtest's on v0.34.9, from the spike's statements of 2026-09-29.
pub const REGTEST_REPLAY_CONTEXT: &str =
    "9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577";
/// Snapshots are taken at multiples of this (section 2).
pub const MAINNET_GRID: u32 = 200;
/// Regtest's ExactReplay starts at 101, so a test chain stops at 100.
pub const REGTEST_GRID: u32 = 100;

/// A 32-byte value in the engine's serialized order.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash32(pub [u8; 32]);

impl Hash32 {
    /// Reversed, as the engine and every explorer print it.
    pub fn display_hex(&self) -> String {
        let mut b = self.0;
        b.reverse();
        operators::hex(&b)
    }

    pub fn from_display_hex(s: &str) -> Option<Self> {
        let v = operators::hex_decode(s.trim())?;
        let mut a: [u8; 32] = v.try_into().ok()?;
        a.reverse();
        Some(Self(a))
    }

    pub fn is_null(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl std::fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_hex())
    }
}

/// Why a manifest is not used. `Display` is one plain line for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TooLarge(usize),
    Truncated,
    TrailingBytes(usize),
    NonCanonicalSize,
    TooManySignatures(u64),
    UnsupportedVersion(u8),
    Malformed(&'static str),
    UnknownChain(String),
    NodeOnAnotherChain,
    WrongReplayContext,
    NodeReplayContextDiffers,
    MissingShieldedCommitment,
    BadGeometry,
    FileTooLarge(u64),
    OffGrid { height: i64, grid: u32 },
    NotAboveStart { height: i64, start: u64 },
    NoSignatures,
    InvalidSignature(String),
    DuplicateSigner(String),
    TooFewOperators(Vec<String>),
    NoPinnedSigner,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::TooLarge(n) => write!(f, "{n} bytes is more than a manifest may be"),
            Refusal::Truncated => write!(f, "the manifest ends early"),
            Refusal::TrailingBytes(n) => write!(f, "{n} bytes after the last signature"),
            Refusal::NonCanonicalSize => write!(f, "a length is not written the one allowed way"),
            Refusal::TooManySignatures(n) => write!(f, "{n} signatures is not a manifest"),
            Refusal::UnsupportedVersion(v) => write!(f, "statement version {v}, not 2"),
            Refusal::Malformed(what) => write!(f, "{what}"),
            Refusal::UnknownChain(id) => write!(f, "chain {id} is neither mainnet nor regtest"),
            Refusal::NodeOnAnotherChain => {
                write!(f, "the statement is for another chain than this node's")
            }
            Refusal::WrongReplayContext => write!(f, "the replay context is not this engine's"),
            Refusal::NodeReplayContextDiffers => {
                write!(f, "the replay context is not the running node's")
            }
            Refusal::MissingShieldedCommitment => write!(f, "the shielded commitment is empty"),
            Refusal::BadGeometry => write!(f, "the file size and chunks do not add up"),
            Refusal::FileTooLarge(n) => {
                write!(f, "a {n}-byte file is more than this app downloads")
            }
            Refusal::OffGrid { height, grid } => {
                write!(f, "height {height} is not a multiple of {grid}")
            }
            Refusal::NotAboveStart { height, start } => {
                write!(
                    f,
                    "height {height} is not above {start}, where the node starts anyway"
                )
            }
            Refusal::NoSignatures => write!(f, "no signatures"),
            Refusal::InvalidSignature(k) => write!(f, "the signature by {k} is not valid"),
            Refusal::DuplicateSigner(k) => write!(f, "{k} signed twice"),
            Refusal::TooFewOperators(names) => write!(
                f,
                "signed by {} operator(s) ({}), two are needed",
                names.len(),
                names.join(", ")
            ),
            Refusal::NoPinnedSigner => write!(f, "no signature is from a key this node pins"),
        }
    }
}

/// The 229 statement bytes, kept as read so a trimmed manifest reproduces
/// them exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    raw: [u8; STATEMENT_LEN],
}

fn h32(raw: &[u8], at: usize) -> Hash32 {
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw[at..at + 32]);
    Hash32(a)
}

fn le_u64(raw: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(raw[at..at + 8].try_into().expect("8 bytes"))
}

fn le_u32(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().expect("4 bytes"))
}

impl Statement {
    pub fn from_raw(raw: [u8; STATEMENT_LEN]) -> Self {
        Self { raw }
    }
    pub fn raw(&self) -> &[u8; STATEMENT_LEN] {
        &self.raw
    }
    pub fn version(&self) -> u8 {
        self.raw[0]
    }
    pub fn chain_id(&self) -> Hash32 {
        h32(&self.raw, 1)
    }
    pub fn block_hash(&self) -> Hash32 {
        h32(&self.raw, 33)
    }
    pub fn height(&self) -> i32 {
        i32::from_le_bytes(self.raw[65..69].try_into().expect("4 bytes"))
    }
    pub fn hash_serialized(&self) -> Hash32 {
        h32(&self.raw, 69)
    }
    pub fn coins(&self) -> u64 {
        le_u64(&self.raw, 101)
    }
    pub fn chain_tx(&self) -> u64 {
        le_u64(&self.raw, 109)
    }
    pub fn shielded(&self) -> Hash32 {
        h32(&self.raw, 117)
    }
    pub fn replay_context(&self) -> Hash32 {
        h32(&self.raw, 149)
    }
    pub fn file_size(&self) -> u64 {
        le_u64(&self.raw, 181)
    }
    pub fn file_hash(&self) -> Hash32 {
        h32(&self.raw, 189)
    }
    pub fn chunk_size(&self) -> u32 {
        le_u32(&self.raw, 221)
    }
    pub fn chunk_count(&self) -> u32 {
        le_u32(&self.raw, 225)
    }
    /// Double SHA-256 of the length-prefixed domain and the statement: what
    /// every signature signs.
    pub fn hash(&self) -> Hash32 {
        let mut first = Sha256::new();
        first.update([HASH_DOMAIN.len() as u8]);
        first.update(HASH_DOMAIN);
        first.update(self.raw);
        Hash32(Sha256::digest(first.finalize()).into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub key: [u8; 33],
    pub der: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub statement: Statement,
    pub signatures: Vec<Signed>,
}

fn read_compact(bytes: &[u8], pos: &mut usize) -> Result<u64, Refusal> {
    let first = *bytes.get(*pos).ok_or(Refusal::Truncated)?;
    *pos += 1;
    let width = match first {
        0..=252 => return Ok(first as u64),
        253 => 2,
        254 => 4,
        255 => 8,
    };
    let body = bytes.get(*pos..*pos + width).ok_or(Refusal::Truncated)?;
    *pos += width;
    let mut buf = [0u8; 8];
    buf[..width].copy_from_slice(body);
    let n = u64::from_le_bytes(buf);
    let min = match width {
        2 => 253,
        4 => 0x1_0000,
        _ => 0x1_0000_0000,
    };
    if n < min {
        return Err(Refusal::NonCanonicalSize);
    }
    Ok(n)
}

fn write_compact(out: &mut Vec<u8>, n: usize) {
    if n < 253 {
        out.push(n as u8);
    } else {
        out.push(253);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    }
}

/// Read a manifest strictly: version 2, canonical lengths, 33-byte keys,
/// signatures of at most 72 bytes, nothing after the last one.
pub fn parse(bytes: &[u8]) -> Result<Manifest, Refusal> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(Refusal::TooLarge(bytes.len()));
    }
    let first = *bytes.first().ok_or(Refusal::Truncated)?;
    if first != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(first));
    }
    let raw: [u8; STATEMENT_LEN] = bytes
        .get(..STATEMENT_LEN)
        .ok_or(Refusal::Truncated)?
        .try_into()
        .expect("229 bytes");
    let mut pos = STATEMENT_LEN;
    let count = read_compact(bytes, &mut pos)?;
    if count > MAX_SIGNATURES {
        return Err(Refusal::TooManySignatures(count));
    }
    let mut signatures = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if read_compact(bytes, &mut pos)? != 33 {
            return Err(Refusal::Malformed("a signer key is not 33 bytes"));
        }
        let key: [u8; 33] = bytes
            .get(pos..pos + 33)
            .ok_or(Refusal::Truncated)?
            .try_into()
            .expect("33 bytes");
        pos += 33;
        let len = read_compact(bytes, &mut pos)? as usize;
        if len > MAX_DER_LEN {
            return Err(Refusal::Malformed("a signature is longer than 72 bytes"));
        }
        let der = bytes
            .get(pos..pos + len)
            .ok_or(Refusal::Truncated)?
            .to_vec();
        pos += len;
        signatures.push(Signed { key, der });
    }
    if pos != bytes.len() {
        return Err(Refusal::TrailingBytes(bytes.len() - pos));
    }
    Ok(Manifest {
        statement: Statement { raw },
        signatures,
    })
}

impl Manifest {
    /// The engine's serialization, byte for byte.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.statement.raw.to_vec();
        write_compact(&mut out, self.signatures.len());
        for s in &self.signatures {
            write_compact(&mut out, s.key.len());
            out.extend_from_slice(&s.key);
            write_compact(&mut out, s.der.len());
            out.extend_from_slice(&s.der);
        }
        out
    }
}

/// The engine's `IsStrictDERSignature`, without a sighash byte.
pub fn is_strict_der(sig: &[u8]) -> bool {
    let n = sig.len();
    if !(8..=MAX_DER_LEN).contains(&n) {
        return false;
    }
    if sig[0] != 0x30 || sig[1] as usize != n - 2 || sig[2] != 0x02 {
        return false;
    }
    let len_r = sig[3] as usize;
    if len_r == 0 || 5 + len_r >= n {
        return false;
    }
    if sig[4] & 0x80 != 0 {
        return false;
    }
    if len_r > 1 && sig[4] == 0 && sig[5] & 0x80 == 0 {
        return false;
    }
    let s_tag = 4 + len_r;
    if sig[s_tag] != 0x02 {
        return false;
    }
    let len_s = sig[s_tag + 1] as usize;
    let s_value = s_tag + 2;
    if len_s == 0 || s_value + len_s != n {
        return false;
    }
    if sig[s_value] & 0x80 != 0 {
        return false;
    }
    if len_s > 1 && sig[s_value] == 0 && sig[s_value + 1] & 0x80 == 0 {
        return false;
    }
    len_r + len_s + 6 == n
}

/// Strict DER, low S, a compressed key on the curve, and valid over `hash`.
pub fn signature_is_valid(hash: &Hash32, key: &[u8; 33], der: &[u8]) -> bool {
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    use k256::ecdsa::{Signature, VerifyingKey};
    if !matches!(key[0], 0x02 | 0x03) || !is_strict_der(der) {
        return false;
    }
    let Ok(sig) = Signature::from_der(der) else {
        return false;
    };
    if sig.normalize_s().is_some() {
        return false; // high S
    }
    let Ok(vk) = VerifyingKey::from_sec1_bytes(key) else {
        return false;
    };
    vk.verify_prehash(&hash.0, &sig).is_ok()
}

/// Every signature valid and no key twice, then the operators behind them,
/// each named once. A valid signature from a key on no operator is allowed
/// and counts for nobody.
pub fn confirming_operators(
    manifest: &Manifest,
    operators: &OperatorList,
) -> Result<Vec<String>, Refusal> {
    if manifest.signatures.is_empty() {
        return Err(Refusal::NoSignatures);
    }
    let hash = manifest.statement.hash();
    let mut keys: Vec<[u8; 33]> = Vec::new();
    for s in &manifest.signatures {
        if keys.contains(&s.key) {
            return Err(Refusal::DuplicateSigner(operators::hex(&s.key)));
        }
        if !signature_is_valid(&hash, &s.key, &s.der) {
            return Err(Refusal::InvalidSignature(operators::hex(&s.key)));
        }
        keys.push(s.key);
    }
    Ok(operators.distinct_operators(&keys))
}

/// What a chain's statements must say.
#[derive(Debug, Clone)]
pub struct ChainRules {
    pub chain: Chain,
    pub genesis: Hash32,
    pub replay_context: Hash32,
    pub grid: u32,
    pub operators: OperatorList,
}

impl ChainRules {
    /// The rules for the chain the statement names. Mainnet's list is the
    /// compiled one whatever `regtest_env` holds; only a regtest statement
    /// reads it.
    pub fn for_statement(
        statement: &Statement,
        regtest_env: Option<&str>,
    ) -> Result<Self, Refusal> {
        let id = statement.chain_id().display_hex();
        let chain = Chain::from_genesis_hex(&id).ok_or(Refusal::UnknownChain(id))?;
        let (genesis, context, grid) = match chain {
            Chain::Main => (
                operators::MAINNET_GENESIS,
                MAINNET_REPLAY_CONTEXT,
                MAINNET_GRID,
            ),
            Chain::Regtest => (
                operators::REGTEST_GENESIS,
                REGTEST_REPLAY_CONTEXT,
                REGTEST_GRID,
            ),
        };
        Ok(Self {
            chain,
            genesis: Hash32::from_display_hex(genesis).expect("compiled genesis"),
            replay_context: Hash32::from_display_hex(context).expect("compiled context"),
            grid,
            operators: operators::for_chain(chain, regtest_env),
        })
    }
}

/// What the running node says, and where it would start without this pair.
#[derive(Debug, Clone, Default)]
pub struct NodeView {
    /// `getblockhash 0`, display order, when the node answered.
    pub genesis: Option<String>,
    /// `getmatmultrustedstatus.replay_authority_context`, display order. A
    /// node with no pin and no key reports none.
    pub replay_context: Option<String>,
    /// The highest start the node has without this pair: the pinned pair's
    /// height or the compiled snapshot's.
    pub start_height: u64,
    /// The keys this node pins.
    pub pinned: Vec<[u8; 33]>,
}

/// A manifest that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    pub manifest: Manifest,
    pub chain: Chain,
    pub height: u64,
    pub statement_hash: Hash32,
    pub operators: Vec<String>,
}

/// [`check_with`] under the rules of the chain the statement names.
pub fn check(
    manifest: &Manifest,
    node: &NodeView,
    regtest_env: Option<&str>,
) -> Result<Confirmed, Refusal> {
    let rules = ChainRules::for_statement(&manifest.statement, regtest_env)?;
    check_with(manifest, node, &rules)
}

/// The running node is on the statement's chain and, when it reports one,
/// has the statement's replay context. A node with no pin and no key
/// reports no context; the compiled one then decides alone.
pub fn node_agrees(st: &Statement, node: &NodeView) -> Result<(), Refusal> {
    if let Some(g) = &node.genesis {
        if Hash32::from_display_hex(g) != Some(st.chain_id()) {
            return Err(Refusal::NodeOnAnotherChain);
        }
    }
    if let Some(c) = &node.replay_context {
        if Hash32::from_display_hex(c) != Some(st.replay_context()) {
            return Err(Refusal::NodeReplayContextDiffers);
        }
    }
    Ok(())
}

/// Every rule of section 7, step 1.
pub fn check_with(
    manifest: &Manifest,
    node: &NodeView,
    rules: &ChainRules,
) -> Result<Confirmed, Refusal> {
    let st = &manifest.statement;
    if st.version() != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(st.version()));
    }
    if st.chain_id() != rules.genesis {
        return Err(Refusal::UnknownChain(st.chain_id().display_hex()));
    }
    if st.replay_context() != rules.replay_context {
        return Err(Refusal::WrongReplayContext);
    }
    node_agrees(st, node)?;
    if st.shielded().is_null() {
        return Err(Refusal::MissingShieldedCommitment);
    }
    let size = st.file_size();
    let chunk = st.chunk_size();
    if size == 0
        || st.file_hash().is_null()
        || !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk)
        || st.chunk_count() as u64 != 1 + (size - 1) / chunk as u64
    {
        return Err(Refusal::BadGeometry);
    }
    if size > crate::attested_snapshot::MAX_SNAPSHOT_BYTES {
        return Err(Refusal::FileTooLarge(size));
    }
    let height = st.height() as i64;
    if height <= 0 || height % rules.grid as i64 != 0 {
        return Err(Refusal::OffGrid {
            height,
            grid: rules.grid,
        });
    }
    if height as u64 <= node.start_height {
        return Err(Refusal::NotAboveStart {
            height,
            start: node.start_height,
        });
    }
    let operators = confirming_operators(manifest, &rules.operators)?;
    if operators.len() < 2 {
        return Err(Refusal::TooFewOperators(operators));
    }
    if !manifest
        .signatures
        .iter()
        .any(|s| node.pinned.contains(&s.key))
    {
        return Err(Refusal::NoPinnedSigner);
    }
    Ok(Confirmed {
        manifest: manifest.clone(),
        chain: rules.chain,
        height: height as u64,
        statement_hash: st.hash(),
        operators,
    })
}

/// Keep every signature from a key this node pins, in order, and drop the
/// rest. The engine refuses a manifest carrying any other key.
pub fn trim_to_pinned(manifest: &Manifest, pinned: &[[u8; 33]]) -> Manifest {
    Manifest {
        statement: manifest.statement.clone(),
        signatures: manifest
            .signatures
            .iter()
            .filter(|s| pinned.contains(&s.key))
            .cloned()
            .collect(),
    }
}

/// SHA-256 while streaming, giving both the plain digest (what the pointer
/// names) and the double one (what the statement names).
#[derive(Default)]
pub struct FileHasher {
    sha: Sha256,
    len: u64,
}

impl FileHasher {
    pub fn update(&mut self, chunk: &[u8]) {
        self.sha.update(chunk);
        self.len += chunk.len() as u64;
    }

    /// (bytes, plain SHA-256 lowercase hex, double SHA-256).
    pub fn finish(self) -> (u64, String, Hash32) {
        let plain = self.sha.finalize();
        let double: [u8; 32] = Sha256::digest(plain).into();
        (self.len, operators::hex(&plain), Hash32(double))
    }
}

/// Whether a file of `len` bytes with double SHA-256 `sha256d` is the one the
/// statement signs.
pub fn file_matches(statement: &Statement, len: u64, sha256d: &Hash32) -> bool {
    len == statement.file_size() && *sha256d == statement.file_hash()
}

/// The pinned keys, parsed.
pub fn pinned_keys(hexes: &[&str]) -> Vec<[u8; 33]> {
    hexes
        .iter()
        .filter_map(|h| operators::parse_key(h))
        .collect()
}
````

- [ ] **Step 6: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- confirmed_snapshot operators`
Expected: `test result: ok. 28 passed; 0 failed` (20 here, 8 from Task 1). These reproduce the independent Python check: statement hash `d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482`, file hash `f234192d…eb2c`, the 3060's signature valid, and the regtest statement hash `11c5406e…f194`.

- [ ] **Step 7: Format, lint, commit**

````bash
for c in crates/btx-core apps/node/src-tauri; do (cd $c && cargo fmt --all --check); done
(cd crates/btx-core && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious)
````

````bash
git add crates/btx-core/Cargo.toml crates/btx-core/Cargo.lock apps/node/src-tauri/Cargo.lock crates/btx-core/src/confirmed_snapshot.rs crates/btx-core/src/lib.rs crates/btx-core/tests/fixtures/confirmed_snapshot
git commit -m "core: read, verify, count and trim a signed snapshot manifest" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: The pointer contract and the start pair (`attested_snapshot.rs`)

**Files:**
- Modify: `crates/btx-core/src/attested_snapshot.rs` (rewritten: the GitHub single-signed pointer is no longer read; the confirmed pointer, the confirmed download and `prepare_start` are added)
- Create: `crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json` (the pointer contract, shared with the website plan)
- Modify: `crates/btx-core/src/snapshot.rs:647-651` (interim call, replaced in Task 5)

**Interfaces:**
- Consumes: Task 2 (`confirmed_snapshot as cs`: `parse`, `check`, `Confirmed`, `NodeView`, `FileHasher`, `file_matches`, `Hash32`, `STATEMENT_LEN`, `MAX_MANIFEST_BYTES`, `REGTEST_REPLAY_CONTEXT`, `pinned_keys`), Task 1 (`operators::{Chain, REGTEST_GENESIS, regtest_env, hex}`).
- Produces (Tasks 4, 5, 9; plan 2; the website plan):
  - `pub const CONFIRMED_POINTER_URL: &str = "https://easybtx.com/api/snapshots/latest"`
  - `pub struct ConfirmedPointer { version: u32, chain: String, height: u64, block_hash: String, statement_hash: String, manifest_url: String, manifest_size: u64, manifest_sha256: String, file_url: String, file_size: u64, file_sha256: String, file_hash: String, operators: Vec<String>, confirmed_at: String }` (Serialize + Deserialize; the doc comment is the website contract)
  - `pub fn confirmed_url_allowed(&str) -> bool`, `pub fn check_pointer(&ConfirmedPointer, fn(&str) -> bool) -> Result<(), String>`, `pub fn parse_confirmed_pointer(&[u8]) -> Result<ConfirmedPointer, String>`, `pub fn pointer_matches(&ConfirmedPointer, &cs::Confirmed) -> Result<(), String>`
  - `pub enum PairKind { Confirmed, Pinned }`, `pub struct ReadyPair { kind: PairKind, height: u64, file: PathBuf, manifest: PathBuf }`
  - `pub async fn prepare_confirmed(&reqwest::Client, pointer_url: &str, datadir: &Path, &NodeView, regtest_env: Option<&str>, url_ok: fn(&str) -> bool) -> Result<ReadyPair, String>`
  - `pub async fn prepare_start(datadir: &Path, &NodeView, compiled_anchor: u64) -> Option<ReadyPair>` (confirmed, else pinned, else `None`)
  - `pub fn fallback_start(compiled_anchor: u64) -> u64` (= `max(compiled_anchor, 225_927)`), `pub fn http_client() -> Result<reqwest::Client, String>`
  - kept: `pinned_pair`, `check`, `file_url`, `manifest_url`, `release_tag`, `pair_dir`, `pair_paths`, `on_disk`, `pair_height_on_disk`, `prune_others`, `MAX_SNAPSHOT_BYTES`, `MAX_MANIFEST_BYTES`, `MAX_POINTER_BYTES`
  - removed: `POINTER_TAG`, `POINTER_ASSET`, `pointer_url`, `parse_pointer`, `fetch_pointer`, `candidates`, `prepare` (nothing else in the repository used them except `snapshot.rs`, changed below; `scripts/publish-attested-snapshot.sh` keeps serving 0.6.32 mirrors, and section 13 of the design is the owner's call).

**The pointer contract, for the website plan** (also in the `ConfirmedPointer` doc comment): `GET https://easybtx.com/api/snapshots/latest` answers HTTP 200 with `Content-Type: application/json`, at most 16 KB, the object below; HTTP 404 with `{"version":1,"confirmed":null}` when nothing is confirmed; `Cache-Control: public, max-age=60`. `manifest_url` and `file_url` are HTTPS on `easybtx.com` or `<store>.public.blob.vercel-storage.com`, no port. The manifest served is the merged one (every accepted signature). Every field that repeats the statement must match it.

- [ ] **Step 1: Add the contract fixture**

Create `crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json` (illustrative values, the shape is the contract):

````json
{
  "version": 1,
  "chain": "main",
  "height": 232000,
  "block_hash": "5c1d3f8a9b2e4c6d7e8f901a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e",
  "statement_hash": "9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a291807f6e5d4c3b2a1908f7e6d5c4b",
  "manifest_url": "https://ebtxsnap.public.blob.vercel-storage.com/snapshots/232000/9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a291807f6e5d4c3b2a1908f7e6d5c4b.manifest",
  "manifest_size": 440,
  "manifest_sha256": "1f2e3d4c5b6a79881726354453627180f9e8d7c6b5a4938271605f4e3d2c1b0a",
  "file_url": "https://ebtxsnap.public.blob.vercel-storage.com/snapshots/232000/utxo-btx-main-232000.dat",
  "file_size": 9112345,
  "file_sha256": "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
  "file_hash": "f9e8d7c6b5a4039281706f5e4d3c2b1a0f9e8d7c6b5a4039281706f5e4d3c2b1",
  "operators": ["Mende", "Aleksander"],
  "confirmed_at": "2026-10-01T12:00:00Z"
}
````

- [ ] **Step 2: Write the failing tests**

In `crates/btx-core/src/attested_snapshot.rs`, replace everything from the line `#[cfg(test)]` to the end of the file with:

````rust
#[cfg(test)]
mod tests {
    use super::*;

    const COMPILED: u64 = 219_000;

    fn pair(height: u64) -> AttestedPair {
        AttestedPair {
            height,
            block_hash: "ab".repeat(32),
            file_size: 9_000_000,
            sha256: "cd".repeat(32),
            manifest_sha256: "ef".repeat(32),
        }
    }

    #[test]
    fn a_pair_that_would_not_start_the_node_higher_is_refused() {
        assert!(check(&pair(COMPILED), COMPILED).is_err());
        assert!(check(&pair(COMPILED - 1), COMPILED).is_err());
        assert!(check(&pair(COMPILED + 1), COMPILED).is_ok());
    }

    #[test]
    fn the_pin_is_a_real_pair_above_the_compiled_snapshot() {
        let p = pinned_pair();
        assert!(check(&p, COMPILED).is_ok());
        // Below the 23 September split, on the chain both sides share.
        assert!(p.height < 227_313);
        let mut broken = p.clone();
        broken.sha256.pop();
        assert!(check(&broken, COMPILED).is_err());
    }

    /// Section 9: a confirmed snapshot must beat the pinned pair, and an
    /// engine that one day compiles a higher base beats both.
    #[test]
    fn the_fallback_start_is_the_higher_of_the_pin_and_the_compiled_base() {
        assert_eq!(fallback_start(COMPILED), 225_927);
        assert_eq!(fallback_start(228_000), 228_000);
    }

    #[test]
    fn urls_follow_the_names_the_first_published_pair_used() {
        assert_eq!(
            file_url(225_927),
            "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/utxo-snapshot-225927/utxo-btx-main-225927.dat"
        );
        assert_eq!(
            manifest_url(225_927),
            "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/utxo-snapshot-225927/snapshot-manifest-225927.json"
        );
        assert_eq!(
            CONFIRMED_POINTER_URL,
            "https://easybtx.com/api/snapshots/latest"
        );
    }

    #[test]
    fn a_pair_on_disk_counts_only_when_both_files_match() {
        use sha2::{Digest, Sha256};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let hex = |b: &[u8]| -> String {
            Sha256::digest(b)
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect()
        };
        let body = b"snapshot bytes".to_vec();
        let man = b"manifest bytes".to_vec();
        let mut p = pair(229_500);
        p.file_size = body.len() as u64;
        p.sha256 = hex(&body);
        p.manifest_sha256 = hex(&man);
        assert!(!on_disk(&dir, &p), "nothing there yet");

        let (file, manifest) = pair_paths(&dir, p.height);
        std::fs::create_dir_all(pair_dir(&dir)).unwrap();
        std::fs::write(&file, &body).unwrap();
        assert!(!on_disk(&dir, &p), "manifest missing");
        std::fs::write(&manifest, b"another manifest").unwrap();
        assert!(!on_disk(&dir, &p), "wrong manifest");
        std::fs::write(&manifest, &man).unwrap();
        assert!(on_disk(&dir, &p));
        std::fs::write(&file, b"snapshot bytez").unwrap();
        assert!(!on_disk(&dir, &p), "same size, different bytes");

        // Pruning keeps exactly the chosen pair.
        std::fs::write(&file, &body).unwrap();
        let (old_file, old_manifest) = pair_paths(&dir, 225_927);
        std::fs::write(&old_file, b"old").unwrap();
        std::fs::write(&old_manifest, b"old").unwrap();
        assert_eq!(
            pair_height_on_disk(&dir),
            Some(229_500),
            "the higher of the two"
        );
        prune_others(&dir, p.height);
        assert!(file.exists() && manifest.exists());
        assert!(!old_file.exists() && !old_manifest.exists());
        assert_eq!(pair_height_on_disk(&dir), Some(229_500));
        assert_eq!(
            pair_height_on_disk(tmp.path().join("nowhere").as_path()),
            None
        );
    }

    // ── the confirmed pointer ───────────────────────────────────────────

    const POINTER: &str = include_str!("../tests/fixtures/confirmed_snapshot/latest.json");

    #[test]
    fn the_contract_fixture_is_a_pointer() {
        let p = parse_confirmed_pointer(POINTER.as_bytes()).unwrap();
        assert_eq!(p.version, 1);
        assert_eq!(p.chain, "main");
        assert_eq!(p.height % 200, 0);
        assert!(check_pointer(&p, confirmed_url_allowed).is_ok(), "{p:?}");
    }

    #[test]
    fn a_pointer_that_is_not_shaped_like_one_is_refused_before_any_download() {
        let good = parse_confirmed_pointer(POINTER.as_bytes()).unwrap();
        let refused = |edit: &dyn Fn(&mut ConfirmedPointer)| {
            let mut p = good.clone();
            edit(&mut p);
            check_pointer(&p, confirmed_url_allowed).is_err()
        };
        assert!(refused(&|p| p.version = 2));
        assert!(refused(&|p| p.chain = "test".into()));
        assert!(refused(&|p| p
            .statement_hash
            .pop()
            .map(|_| ())
            .unwrap_or(())));
        assert!(refused(&|p| p.file_hash = "zz".repeat(32)));
        assert!(refused(&|p| p.manifest_size = 65 * 1024));
        assert!(refused(&|p| p.manifest_size = 100));
        assert!(refused(&|p| p.file_size = 0));
        assert!(refused(&|p| p.file_size = MAX_SNAPSHOT_BYTES + 1));
        assert!(refused(&|p| p.file_url = "http://easybtx.com/x.dat".into()));
        assert!(refused(&|p| p.manifest_url = "https://127.0.0.1/x".into()));
        assert!(parse_confirmed_pointer(b"<html>Not Found</html>").is_err());
        assert!(parse_confirmed_pointer(br#"{"version":1,"confirmed":null}"#).is_err());
        assert!(parse_confirmed_pointer(&vec![b' '; MAX_POINTER_BYTES + 1]).is_err());
    }

    #[test]
    fn only_the_website_and_its_blob_store_are_download_hosts() {
        for ok in [
            "https://easybtx.com/api/snapshots/file/232000",
            "https://abc123.public.blob.vercel-storage.com/snapshots/232000/x.manifest",
        ] {
            assert!(confirmed_url_allowed(ok), "{ok}");
        }
        for bad in [
            "http://easybtx.com/x",
            "https://easybtx.com:8443/x",
            "https://user@easybtx.com/x",
            "https://evil.com/x",
            "https://easybtx.com.evil.com/x",
            "https://public.blob.vercel-storage.com/x",
            "https://.public.blob.vercel-storage.com/x",
            "https://127.0.0.1:8332/",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(!confirmed_url_allowed(bad), "{bad}");
        }
    }

    // ── prepare_confirmed against a local server ────────────────────────

    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        crate::operators::hex(&Sha256::digest(b))
    }

    fn regtest_pointer(base: &str, manifest: &[u8], file: &[u8]) -> ConfirmedPointer {
        let m = cs::parse(manifest).unwrap();
        let st = &m.statement;
        ConfirmedPointer {
            version: 1,
            chain: "regtest".into(),
            height: st.height() as u64,
            block_hash: st.block_hash().display_hex(),
            statement_hash: st.hash().display_hex(),
            manifest_url: format!("{base}/m"),
            manifest_size: manifest.len() as u64,
            manifest_sha256: sha(manifest),
            file_url: format!("{base}/f"),
            file_size: file.len() as u64,
            file_sha256: sha(file),
            file_hash: st.file_hash().display_hex(),
            operators: vec!["producer".into(), "confirmer".into()],
            confirmed_at: "2026-09-29T12:00:00Z".into(),
        }
    }

    fn any_url(_: &str) -> bool {
        true
    }

    fn view() -> NodeView {
        NodeView {
            genesis: Some(crate::operators::REGTEST_GENESIS.into()),
            replay_context: Some(cs::REGTEST_REPLAY_CONTEXT.into()),
            start_height: 0,
            pinned: cs::pinned_keys(&[P]),
        }
    }

    #[tokio::test]
    async fn a_confirmed_pair_is_downloaded_and_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        let p = regtest_pointer(&server.url(), R_PC, R_DAT);
        server
            .mock("GET", "/latest")
            .with_body(serde_json::to_vec(&p).unwrap())
            .create_async()
            .await;
        server
            .mock("GET", "/m")
            .with_body(R_PC)
            .create_async()
            .await;
        server
            .mock("GET", "/f")
            .with_body(R_DAT)
            .create_async()
            .await;
        let env = format!("producer={P};confirmer={C}");
        let ready = prepare_confirmed(
            &reqwest::Client::new(),
            &format!("{}/latest", server.url()),
            tmp.path(),
            &view(),
            Some(&env),
            any_url,
        )
        .await
        .unwrap();
        assert_eq!(ready.kind, PairKind::Confirmed);
        assert_eq!(ready.height, 100);
        assert_eq!(std::fs::read(&ready.file).unwrap(), R_DAT);
        assert_eq!(std::fs::read(&ready.manifest).unwrap(), R_PC);
    }

    #[tokio::test]
    async fn one_operator_is_refused_after_the_manifest_and_before_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        let p = regtest_pointer(&server.url(), R_P, R_DAT);
        server
            .mock("GET", "/latest")
            .with_body(serde_json::to_vec(&p).unwrap())
            .create_async()
            .await;
        server.mock("GET", "/m").with_body(R_P).create_async().await;
        let file = server
            .mock("GET", "/f")
            .with_body(R_DAT)
            .expect(0)
            .create_async()
            .await;
        let env = format!("producer={P};confirmer={C}");
        let err = prepare_confirmed(
            &reqwest::Client::new(),
            &format!("{}/latest", server.url()),
            tmp.path(),
            &view(),
            Some(&env),
            any_url,
        )
        .await
        .unwrap_err();
        assert!(err.contains("two are needed"), "{err}");
        file.assert_async().await;
        assert!(
            !pair_paths(tmp.path(), 100).1.exists(),
            "a refused manifest is not kept"
        );
    }

    #[tokio::test]
    async fn a_file_whose_hash_does_not_match_the_statement_is_refused_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut tampered = R_DAT.to_vec();
        tampered[500] ^= 1;
        let mut server = mockito::Server::new_async().await;
        // The pointer lies consistently: its plain SHA-256 is the tampered file's.
        let p = regtest_pointer(&server.url(), R_PC, &tampered);
        server
            .mock("GET", "/latest")
            .with_body(serde_json::to_vec(&p).unwrap())
            .create_async()
            .await;
        server
            .mock("GET", "/m")
            .with_body(R_PC)
            .create_async()
            .await;
        server
            .mock("GET", "/f")
            .with_body(tampered.clone())
            .create_async()
            .await;
        let env = format!("producer={P};confirmer={C}");
        let err = prepare_confirmed(
            &reqwest::Client::new(),
            &format!("{}/latest", server.url()),
            tmp.path(),
            &view(),
            Some(&env),
            any_url,
        )
        .await
        .unwrap_err();
        assert!(err.contains("not the one the statement signs"), "{err}");
        assert!(!pair_paths(tmp.path(), 100).0.exists());
    }

    #[tokio::test]
    async fn a_pointer_that_misdescribes_its_manifest_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        let mut p = regtest_pointer(&server.url(), R_PC, R_DAT);
        p.block_hash = "00".repeat(32);
        server
            .mock("GET", "/latest")
            .with_body(serde_json::to_vec(&p).unwrap())
            .create_async()
            .await;
        server
            .mock("GET", "/m")
            .with_body(R_PC)
            .create_async()
            .await;
        let env = format!("producer={P};confirmer={C}");
        let err = prepare_confirmed(
            &reqwest::Client::new(),
            &format!("{}/latest", server.url()),
            tmp.path(),
            &view(),
            Some(&env),
            any_url,
        )
        .await
        .unwrap_err();
        assert!(err.contains("does not describe"), "{err}");
    }

    #[tokio::test]
    async fn no_confirmed_snapshot_is_a_plain_404() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/latest")
            .with_status(404)
            .with_body(r#"{"version":1,"confirmed":null}"#)
            .create_async()
            .await;
        let err = prepare_confirmed(
            &reqwest::Client::new(),
            &format!("{}/latest", server.url()),
            tmp.path(),
            &view(),
            None,
            any_url,
        )
        .await
        .unwrap_err();
        assert!(err.contains("HTTP 404"), "{err}");
    }

    /// Before a release: the pinned pair is still published byte for byte.
    /// `cargo test -p btx-core -- --ignored the_pinned_pair_is_still_published`
    #[tokio::test]
    #[ignore = "network: downloads 9 MB from GitHub"]
    async fn the_pinned_pair_is_still_published() {
        let tmp = tempfile::tempdir().unwrap();
        let client = http_client().unwrap();
        let p = pinned_pair();
        download_pair(&client, tmp.path(), &p).await.unwrap();
        assert!(on_disk(tmp.path(), &p));
    }
}
````

- [ ] **Step 3: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- attested_snapshot`
Expected: compile errors, among them `cannot find function `fallback_start``, `cannot find type `ConfirmedPointer``, `cannot find function `prepare_confirmed``.

- [ ] **Step 4: Write the implementation**

In `crates/btx-core/src/attested_snapshot.rs`, replace everything above `#[cfg(test)]` (the module doc, the imports and every item) with:

````rust
//! Signed ("attested") UTXO snapshots: how every node starts near the tip
//! instead of at the snapshot compiled into the engine.
//!
//! WHERE A NODE STARTS, best first (docs/decisions/2026-09-29-every-node-
//! starts-near-the-tip.md, section 9):
//!
//! 1. The newest CONFIRMED snapshot: a pair easybtx.com points at
//!    ([`CONFIRMED_POINTER_URL`]) whose statement two different operators
//!    signed, checked here by [`crate::confirmed_snapshot`] before anything
//!    is downloaded past the manifest. Nothing the website says is trusted;
//!    the pointer only says where to look.
//! 2. The pair published before any of this existed ([`pinned_pair`], base
//!    225,927). One operator signed it, but its sizes and hashes are compiled
//!    into the app, so it is trusted as the app is.
//! 3. The snapshot compiled into the engine (`crate::snapshot`).
//!
//! The engine loads a signed pair only when a key this node pins signed its
//! manifest, and refuses a manifest carrying any other key, so the loader
//! (`crate::confirmed_load`) hands it only the pinned signatures. The
//! single-signed pointer of 0.6.31 (`utxo-snapshot-latest` on GitHub) is no
//! longer read: it was never published, and one signature is not a
//! confirmation.

use crate::confirmed_snapshot::{self as cs, Hash32, NodeView};
use crate::snapshot::verify_file_sha256;
use crate::snapshot_serve::{manifest_file_name, snapshot_file_name};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the pinned pair is published.
pub const RELEASES_DOWNLOAD: &str =
    "https://github.com/MendeMatthias/EasyBTX-releases/releases/download";

/// A pointer is a few hundred bytes. Anything far larger is not one.
pub const MAX_POINTER_BYTES: usize = 16 * 1024;

/// The pairs so far are about 9 MB. A pointer or statement claiming more than
/// this is refused before anything is downloaded.
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// A manifest with one signature is 335 bytes. The pinned pair's is that.
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024;

/// Where the website serves the newest confirmed snapshot. The contract is
/// [`ConfirmedPointer`].
pub const CONFIRMED_POINTER_URL: &str = "https://easybtx.com/api/snapshots/latest";

/// One published pair, as the app pins it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AttestedPair {
    pub height: u64,
    pub block_hash: String,
    pub file_size: u64,
    /// Plain SHA-256 of the snapshot file.
    pub sha256: String,
    pub manifest_sha256: String,
}

/// The pair published before the pointer existed: base 225,927, on the chain
/// both sides of the 227,313 split share. Read from the published files on
/// 2026-09-26: the manifest (statement version 2, 140,731 coins) carries one
/// signature, by `02d5efca`, the key every mirror pins since 0.6.30, and its
/// `snapshot_file_hash` equals the file's byte-reversed double SHA-256
/// (`f234192d…`). The sizes and SHA-256 values below are those files'.
pub fn pinned_pair() -> AttestedPair {
    AttestedPair {
        height: 225_927,
        block_hash: "06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932".into(),
        file_size: 9_045_522,
        sha256: "5f386c9c8be5a6c28bc5b63352903325cfe6a64a68c68b38f49ea4ac85f9ca05".into(),
        manifest_sha256: "8adc90c2b4514334d0bc0e1dafa5f3bc85ed0cfcc55d051a586e117794e332ed".into(),
    }
}

/// The highest start a node has without a confirmed snapshot: the pinned
/// pair's base when it is above the compiled one, else the compiled one. A
/// confirmed snapshot must be above this to be worth loading.
pub fn fallback_start(compiled_anchor: u64) -> u64 {
    compiled_anchor.max(pinned_pair().height)
}

/// The pre-release a pair is published under, the name the 225,927 one set.
pub fn release_tag(height: u64) -> String {
    format!("utxo-snapshot-{height}")
}

pub fn file_url(height: u64) -> String {
    format!(
        "{RELEASES_DOWNLOAD}/{}/{}",
        release_tag(height),
        snapshot_file_name(height)
    )
}

pub fn manifest_url(height: u64) -> String {
    format!(
        "{RELEASES_DOWNLOAD}/{}/{}",
        release_tag(height),
        manifest_file_name(height)
    )
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether the pinned pair is worth downloading: shaped like one, and above
/// the snapshot compiled into this engine.
pub fn check(pair: &AttestedPair, compiled_anchor: u64) -> Result<(), String> {
    if pair.height <= compiled_anchor {
        return Err(format!(
            "base {} is not above the compiled snapshot's {compiled_anchor}",
            pair.height
        ));
    }
    if pair.file_size == 0 || pair.file_size > MAX_SNAPSHOT_BYTES {
        return Err(format!("file size {} is out of range", pair.file_size));
    }
    for (name, value) in [
        ("block_hash", &pair.block_hash),
        ("sha256", &pair.sha256),
        ("manifest_sha256", &pair.manifest_sha256),
    ] {
        if !is_hex64(value) {
            return Err(format!("{name} is not 64 hex characters"));
        }
    }
    Ok(())
}

// ── The confirmed pointer ───────────────────────────────────────────────────

/// What `GET https://easybtx.com/api/snapshots/latest` answers. The website
/// plan implements it; this is the contract.
///
/// * HTTP 200, `Content-Type: application/json`, at most 16 KB, this object.
/// * HTTP 404 when no snapshot is confirmed, body `{"version":1,"confirmed":null}`.
///   The app treats every status other than 200 as "none" and falls back.
/// * `Cache-Control: public, max-age=60`.
///
/// ```json
/// {
///   "version": 1,
///   "chain": "main",
///   "height": 232000,
///   "block_hash": "<64 hex, display order>",
///   "statement_hash": "<64 hex, display order>",
///   "manifest_url": "https://<store>.public.blob.vercel-storage.com/snapshots/232000/<statement_hash>.manifest",
///   "manifest_size": 440,
///   "manifest_sha256": "<64 hex, plain SHA-256 of the manifest bytes>",
///   "file_url": "https://<store>.public.blob.vercel-storage.com/snapshots/232000/utxo-btx-main-232000.dat",
///   "file_size": 9112345,
///   "file_sha256": "<64 hex, plain SHA-256 of the file>",
///   "file_hash": "<64 hex, the statement's double SHA-256, display order>",
///   "operators": ["Mende", "Aleksander"],
///   "confirmed_at": "2026-10-01T12:00:00Z"
/// }
/// ```
///
/// The manifest served is the merged one, every signature the website
/// accepted; the app checks it and keeps only what its node pins. Nothing
/// here is trusted: the statement's signatures decide, and every field that
/// repeats the statement must match it or the pair is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedPointer {
    pub version: u32,
    pub chain: String,
    pub height: u64,
    pub block_hash: String,
    pub statement_hash: String,
    pub manifest_url: String,
    pub manifest_size: u64,
    pub manifest_sha256: String,
    pub file_url: String,
    pub file_size: u64,
    pub file_sha256: String,
    pub file_hash: String,
    pub operators: Vec<String>,
    pub confirmed_at: String,
}

/// The only hosts the app downloads a confirmed pair from: the website and
/// its public blob store. HTTPS, no port, no user info.
pub fn confirmed_url_allowed(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let host_ok = match u.host_str() {
        Some("easybtx.com") => true,
        Some(h) => {
            h.ends_with(".public.blob.vercel-storage.com")
                && h.len() > ".public.blob.vercel-storage.com".len()
        }
        None => false,
    };
    u.scheme() == "https"
        && host_ok
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
}

/// The pointer's shape, before anything it names is downloaded.
pub fn check_pointer(p: &ConfirmedPointer, url_ok: fn(&str) -> bool) -> Result<(), String> {
    if p.version != 1 {
        return Err(format!("pointer version {}, not 1", p.version));
    }
    if p.chain != "main" && p.chain != "regtest" {
        return Err(format!("pointer names chain {:?}", p.chain));
    }
    for (name, value) in [
        ("block_hash", &p.block_hash),
        ("statement_hash", &p.statement_hash),
        ("manifest_sha256", &p.manifest_sha256),
        ("file_sha256", &p.file_sha256),
        ("file_hash", &p.file_hash),
    ] {
        if !is_hex64(value) {
            return Err(format!("{name} is not 64 hex characters"));
        }
    }
    if p.manifest_size <= cs::STATEMENT_LEN as u64
        || p.manifest_size > cs::MAX_MANIFEST_BYTES as u64
    {
        return Err(format!("manifest size {} is out of range", p.manifest_size));
    }
    if p.file_size == 0 || p.file_size > MAX_SNAPSHOT_BYTES {
        return Err(format!("file size {} is out of range", p.file_size));
    }
    for url in [&p.manifest_url, &p.file_url] {
        if !url_ok(url) {
            return Err(format!("{url} is not a place this app downloads from"));
        }
    }
    Ok(())
}

pub fn parse_confirmed_pointer(body: &[u8]) -> Result<ConfirmedPointer, String> {
    if body.len() > MAX_POINTER_BYTES {
        return Err(format!("pointer is {} bytes, not a pointer", body.len()));
    }
    serde_json::from_slice(body).map_err(|e| format!("unreadable pointer: {e}"))
}

/// Every field the pointer repeats from the statement must be the
/// statement's.
pub fn pointer_matches(p: &ConfirmedPointer, confirmed: &cs::Confirmed) -> Result<(), String> {
    let st = &confirmed.manifest.statement;
    let chain = match confirmed.chain {
        crate::operators::Chain::Main => "main",
        crate::operators::Chain::Regtest => "regtest",
    };
    let same = p.chain == chain
        && p.height == confirmed.height
        && p.block_hash
            .eq_ignore_ascii_case(&st.block_hash().display_hex())
        && p.statement_hash
            .eq_ignore_ascii_case(&confirmed.statement_hash.display_hex())
        && p.file_size == st.file_size()
        && p.file_hash
            .eq_ignore_ascii_case(&st.file_hash().display_hex());
    if same {
        Ok(())
    } else {
        Err("the pointer does not describe the manifest it points at".into())
    }
}

// ── On disk ─────────────────────────────────────────────────────────────────

/// Where a pair is kept until the snapshot sweep removes it with the compiled
/// one: beside `faststart/snapshot.dat`. Confirmed and pinned pairs share it,
/// under the keeper's file names.
pub fn pair_dir(datadir: &Path) -> PathBuf {
    datadir.join("faststart").join("attested")
}

/// (snapshot file, manifest) for a pair.
pub fn pair_paths(datadir: &Path, height: u64) -> (PathBuf, PathBuf) {
    let dir = pair_dir(datadir);
    (
        dir.join(snapshot_file_name(height)),
        dir.join(manifest_file_name(height)),
    )
}

fn file_matches(path: &Path, size: u64, sha256: &str) -> bool {
    std::fs::metadata(path).map(|m| m.len()).ok() == Some(size)
        && matches!(verify_file_sha256(path, sha256), Ok(true))
}

/// Both files present with the pair's sizes and SHA-256. The manifest's size
/// is not in the pin, so its SHA-256 alone decides it.
pub fn on_disk(datadir: &Path, pair: &AttestedPair) -> bool {
    let (file, manifest) = pair_paths(datadir, pair.height);
    file_matches(&file, pair.file_size, &pair.sha256)
        && manifest.is_file()
        && matches!(
            verify_file_sha256(&manifest, &pair.manifest_sha256),
            Ok(true)
        )
}

/// The base of the signed pair on disk, if any. The snapshot sweep measures
/// "the node has built on it" from this base when there is one, not from the
/// compiled snapshot's lower one.
pub fn pair_height_on_disk(datadir: &Path) -> Option<u64> {
    std::fs::read_dir(pair_dir(datadir))
        .ok()?
        .flatten()
        .filter_map(|e| {
            crate::snapshot_serve::height_from_file_name(&e.file_name().to_string_lossy())
        })
        .max()
}

/// Remove every file in the pair folder that is not `keep`'s, so a node that
/// was offered several pairs over its life holds one.
pub fn prune_others(datadir: &Path, keep: u64) {
    let (file, manifest) = pair_paths(datadir, keep);
    let Ok(rd) = std::fs::read_dir(pair_dir(datadir)) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path != file && path != manifest {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// ── Downloading ─────────────────────────────────────────────────────────────

pub fn http_client() -> Result<reqwest::Client, String> {
    // The same timeouts as the compiled snapshot's download: a stalled
    // connection fails, a slow one is never cut off.
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// Stream `url` into `dest` through a `.partial`, refusing more than `cap`
/// bytes and anything whose size or SHA-256 differs from what was expected.
/// Returns the file's double SHA-256, what a statement names.
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    expected_sha256: &str,
    cap: u64,
) -> Result<Hash32, String> {
    use tokio::io::AsyncWriteExt;

    let tmp = dest.with_extension("partial");
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| format!("create {}: {e}", tmp.display()))?;
    let mut hasher = cs::FileHasher::default();
    let mut written: u64 = 0;
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(format!("interrupted: {e}"));
            }
        };
        written += chunk.len() as u64;
        if written > cap {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!("larger than {cap} bytes"));
        }
        hasher.update(&chunk);
        if let Err(e) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!("write: {e}"));
        }
    }
    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    drop(file);
    let (len, got, double) = hasher.finish();
    if expected_size.is_some_and(|s| s != len) || !got.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!(
            "{len} bytes with SHA-256 {got}, not the published pair's"
        ));
    }
    tokio::fs::rename(&tmp, dest)
        .await
        .map_err(|e| format!("rename: {e}"))?;
    Ok(double)
}

async fn download_pair(
    client: &reqwest::Client,
    datadir: &Path,
    pair: &AttestedPair,
) -> Result<(), String> {
    let (file, manifest) = pair_paths(datadir, pair.height);
    std::fs::create_dir_all(pair_dir(datadir)).map_err(|e| format!("create pair dir: {e}"))?;
    // The small file first: a pair whose manifest is gone is not worth 9 MB.
    download_verified(
        client,
        &manifest_url(pair.height),
        &manifest,
        None,
        &pair.manifest_sha256,
        MAX_MANIFEST_BYTES,
    )
    .await
    .map_err(|e| format!("manifest: {e}"))?;
    download_verified(
        client,
        &file_url(pair.height),
        &file,
        Some(pair.file_size),
        &pair.sha256,
        MAX_SNAPSHOT_BYTES,
    )
    .await
    .map_err(|e| format!("snapshot: {e}"))?;
    Ok(())
}

/// Which kind of pair a node is about to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    /// Two operators signed it (section 1).
    Confirmed,
    /// [`pinned_pair`].
    Pinned,
}

/// A pair on disk, checked, ready for `crate::confirmed_load::load`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyPair {
    pub kind: PairKind,
    pub height: u64,
    pub file: PathBuf,
    pub manifest: PathBuf,
}

/// Read the pointer at `pointer_url`, then download and check the pair it
/// names (section 7, steps 1 and 2): the manifest first, checked in full by
/// [`cs::check`], then the file, whose size and double SHA-256 must be the
/// statement's. `url_ok` is [`confirmed_url_allowed`] everywhere but tests.
pub async fn prepare_confirmed(
    client: &reqwest::Client,
    pointer_url: &str,
    datadir: &Path,
    view: &NodeView,
    regtest_env: Option<&str>,
    url_ok: fn(&str) -> bool,
) -> Result<ReadyPair, String> {
    let resp = client
        .get(pointer_url)
        .send()
        .await
        .map_err(|e| format!("pointer unreachable: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!(
            "no confirmed snapshot (HTTP {})",
            resp.status().as_u16()
        ));
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("pointer read: {e}"))?;
    let p = parse_confirmed_pointer(&body)?;
    check_pointer(&p, url_ok)?;

    let (file, manifest) = pair_paths(datadir, p.height);
    std::fs::create_dir_all(pair_dir(datadir)).map_err(|e| format!("create pair dir: {e}"))?;
    let manifest_there =
        manifest.is_file() && matches!(verify_file_sha256(&manifest, &p.manifest_sha256), Ok(true));
    if !manifest_there {
        download_verified(
            client,
            &p.manifest_url,
            &manifest,
            Some(p.manifest_size),
            &p.manifest_sha256,
            cs::MAX_MANIFEST_BYTES as u64,
        )
        .await
        .map_err(|e| format!("manifest: {e}"))?;
    }
    let bytes = std::fs::read(&manifest).map_err(|e| format!("manifest: {e}"))?;
    let confirmed = cs::parse(&bytes)
        .and_then(|m| cs::check(&m, view, regtest_env))
        .map_err(|e| {
            let _ = std::fs::remove_file(&manifest);
            format!("not confirmed: {e}")
        })?;
    pointer_matches(&p, &confirmed)?;

    let st = &confirmed.manifest.statement;
    let double = if file_matches(&file, st.file_size(), &p.file_sha256) {
        let mut h = cs::FileHasher::default();
        h.update(&std::fs::read(&file).map_err(|e| format!("snapshot: {e}"))?);
        h.finish().2
    } else {
        download_verified(
            client,
            &p.file_url,
            &file,
            Some(st.file_size()),
            &p.file_sha256,
            MAX_SNAPSHOT_BYTES,
        )
        .await
        .map_err(|e| format!("snapshot: {e}"))?
    };
    if !cs::file_matches(st, st.file_size(), &double) {
        let _ = std::fs::remove_file(&file);
        return Err("the file is not the one the statement signs".into());
    }
    Ok(ReadyPair {
        kind: PairKind::Confirmed,
        height: confirmed.height,
        file,
        manifest,
    })
}

/// The pair this node should start from, verified on disk, or `None` for the
/// compiled snapshot: a confirmed pair, else the pinned one (section 9). Best
/// effort and quiet about it: every failure is logged and falls through.
pub async fn prepare_start(
    datadir: &Path,
    view: &NodeView,
    compiled_anchor: u64,
) -> Option<ReadyPair> {
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[attested] {e}; using the compiled snapshot");
            return None;
        }
    };
    let regtest_env = crate::operators::regtest_env();
    match prepare_confirmed(
        &client,
        CONFIRMED_POINTER_URL,
        datadir,
        view,
        regtest_env.as_deref(),
        confirmed_url_allowed,
    )
    .await
    {
        Ok(pair) => {
            prune_others(datadir, pair.height);
            eprintln!("[attested] confirmed snapshot {} ready", pair.height);
            return Some(pair);
        }
        Err(e) => eprintln!("[attested] {e}; trying the pinned pair"),
    }
    let pinned = pinned_pair();
    if let Err(e) = check(&pinned, compiled_anchor) {
        eprintln!("[attested] pinned pair not used: {e}");
        return None;
    }
    if !on_disk(datadir, &pinned) {
        if let Err(e) = download_pair(&client, datadir, &pinned).await {
            eprintln!(
                "[attested] pinned pair {} not downloaded: {e}",
                pinned.height
            );
            return None;
        }
    }
    prune_others(datadir, pinned.height);
    let (file, manifest) = pair_paths(datadir, pinned.height);
    eprintln!("[attested] pinned pair {} ready", pinned.height);
    Some(ReadyPair {
        kind: PairKind::Pinned,
        height: pinned.height,
        file,
        manifest,
    })
}
````

In `crates/btx-core/src/snapshot.rs`, `ensure_snapshot_loaded_with` still calls the removed `prepare`. Replace:

````rust
        if prefer_attested {
            if let Some((pair, file, manifest)) =
                crate::attested_snapshot::prepare(&datadir, anchor_height).await
            {
````

with:

````rust
        if prefer_attested {
            // Interim until Task 5 replaces this block with the one loading
            // path: the pair `prepare_start` checked, loaded as it is.
            let view = crate::confirmed_snapshot::NodeView {
                start_height: crate::attested_snapshot::fallback_start(anchor_height),
                pinned: crate::confirmed_snapshot::pinned_keys(
                    &crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS,
                ),
                ..Default::default()
            };
            if let Some(pair) =
                crate::attested_snapshot::prepare_start(&datadir, &view, anchor_height).await
            {
                let (file, manifest) = (pair.file.clone(), pair.manifest.clone());
````

The rest of that block is unchanged (it reads `pair.height`, `file` and `manifest`).

- [ ] **Step 5: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- attested_snapshot snapshot::`
Expected: every test passes; `attested_snapshot` shows `13 passed; 0 failed; 1 ignored` within the total (the ignored one downloads the pinned pair from GitHub).

Optional, before a release: `cargo test --locked --lib -- --ignored the_pinned_pair_is_still_published` (downloads 9 MB).

- [ ] **Step 6: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cargo test --locked
cd ../..
````
````bash
git add crates/btx-core/src/attested_snapshot.rs crates/btx-core/src/snapshot.rs crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json
git commit -m "core: the confirmed pointer, the confirmed download, and the pinned pair as the fallback" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: The one loading path (`confirmed_load.rs`)

**Files:**
- Create: `crates/btx-core/src/confirmed_load.rs`
- Modify: `crates/btx-core/src/snapshot.rs` (`LoadOutcome` public and `Clone`; `run_cli_load`)
- Modify: `crates/btx-core/src/node_api.rs:417-429` (`replay_authority_context`), `crates/btx-core/src/role.rs:655-660` (test literal)
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 (`cs::{parse, check, node_agrees, trim_to_pinned, FileHasher, file_matches, pinned_keys, NodeView, REGTEST_REPLAY_CONTEXT}`), Task 3 (`attested_snapshot::{ReadyPair, PairKind, pinned_pair, pair_paths, pair_dir}`), `known_invalid::{refuse, refuse_held_in_order, HeldBranch, KnownInvalidBlock, Refusal, KNOWN_INVALID_BLOCKS, HELD_BRANCHES, refusal_enabled}` (exist).
- Produces (Tasks 5, 9, 10; plan 2):
  - `pub trait LoadRunner { async fn load_attested(&self, file: &Path, manifest: &Path) -> LoadOutcome; }` and `pub struct CliRunner { btx_cli: PathBuf, args: Vec<String> }` with `CliRunner::for_datadir(&Path, &Path)`
  - `pub struct Holds<'a> { invalid: &'a [KnownInvalidBlock], held: &'a [HeldBranch] }`, `Holds::compiled()`, `Holds::none()`
  - `pub enum LoadError { NotConfirmed(String), HeldNotRefused(String), Engine(String), HeldRootOnChain { height: u64, root: String }, Io(String) }` (`Display`)
  - `pub struct Loaded { height: u64, signatures: usize, superseded: bool }`
  - `pub async fn node_view(&dyn Rpc, pinned: &[&str], start_height: u64) -> NodeView`
  - `pub fn trimmed_manifest_path(&ReadyPair) -> PathBuf`
  - `pub async fn load(&dyn Rpc, &dyn LoadRunner, &ReadyPair, &NodeView, &Holds<'_>, regtest_env: Option<&str>) -> Result<Loaded, LoadError>`
  - `pub fn set_aside_snapshot_chainstate(network_dir: &Path, now_unix: u64) -> std::io::Result<Option<PathBuf>>`
  - `snapshot::LoadOutcome` (now `pub`, `Clone`, `Eq`), `pub async fn snapshot::run_cli_load(btx_cli: &Path, args: &[String], method: &str, files: &[&Path]) -> LoadOutcome`
  - `node_api::MatmulTrustedStatus::replay_authority_context: Option<String>`

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/confirmed_load.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, AppResult};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_PCD: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PCD.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const HELD: &[HeldBranch] = &[HeldBranch {
        height: 50,
        root: ROOT,
        why: "a test hold",
    }];

    /// A node scripted per method. `chain` answers getblockhash by height.
    struct Node {
        genesis: &'static str,
        chain: HashMap<u64, String>,
        knows_root: bool,
        invalidate_fails: bool,
        calls: Mutex<Vec<String>>,
    }

    impl Node {
        fn regtest() -> Self {
            Self {
                genesis: crate::operators::REGTEST_GENESIS,
                chain: HashMap::new(),
                knows_root: true,
                invalidate_fails: false,
                calls: Mutex::new(Vec::new()),
            }
        }
        fn methods(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Rpc for Node {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls.lock().unwrap().push(method.to_string());
            let not_found = || AppError::Rpc {
                code: -5,
                message: "Block not found".into(),
            };
            match method {
                "getblockhash" => {
                    let h = params[0].as_u64().unwrap_or(u64::MAX);
                    if h == 0 {
                        return Ok(json!(self.genesis));
                    }
                    self.chain
                        .get(&h)
                        .map(|s| json!(s))
                        .ok_or_else(|| AppError::Rpc {
                            code: -8,
                            message: "Block height out of range".into(),
                        })
                }
                "getmatmultrustedstatus" => Ok(json!({
                    "replay_authority_context": cs::REGTEST_REPLAY_CONTEXT,
                    "matmul_validation_mode": "trusted",
                    "trusted_mirror": true
                })),
                "getblockheader" if self.knows_root => Ok(json!({"height": 50})),
                "getblockheader" => Err(not_found()),
                "invalidateblock" if self.invalidate_fails => Err(AppError::Rpc {
                    code: -1,
                    message: "request timed out".into(),
                }),
                "invalidateblock" => Ok(Value::Null),
                other => panic!("the loader must never call {other}"),
            }
        }
    }

    /// Records the manifest it was handed and answers as told.
    struct Runner {
        answer: LoadOutcome,
        seen: Mutex<Option<Vec<u8>>>,
    }

    impl Runner {
        fn new(answer: LoadOutcome) -> Self {
            Self {
                answer,
                seen: Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl LoadRunner for Runner {
        async fn load_attested(&self, _file: &Path, manifest: &Path) -> LoadOutcome {
            *self.seen.lock().unwrap() = Some(std::fs::read(manifest).unwrap());
            self.answer.clone()
        }
    }

    fn pair_on_disk(dir: &Path, manifest: &[u8], file: &[u8]) -> ReadyPair {
        let (f, m) = attested_snapshot::pair_paths(dir, 100);
        std::fs::create_dir_all(attested_snapshot::pair_dir(dir)).unwrap();
        std::fs::write(&f, file).unwrap();
        std::fs::write(&m, manifest).unwrap();
        ReadyPair {
            kind: PairKind::Confirmed,
            height: 100,
            file: f,
            manifest: m,
        }
    }

    fn env() -> String {
        format!("producer={P};confirmer={C}")
    }

    async fn view(node: &Node) -> NodeView {
        node_view(node, &[P], 0).await
    }

    #[tokio::test]
    async fn a_confirmed_pair_is_trimmed_to_the_pins_and_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PCD, R_DAT);
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let got = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
        )
        .await
        .unwrap();
        assert_eq!(
            got,
            Loaded {
                height: 100,
                signatures: 1,
                superseded: false
            }
        );
        assert_eq!(
            runner.seen.lock().unwrap().as_deref(),
            Some(R_P),
            "the engine saw the producer's own file, byte for byte"
        );
        let calls = node.methods();
        let invalidate = calls.iter().position(|m| m == "invalidateblock").unwrap();
        assert!(
            invalidate < calls.len() - 1,
            "refused before the load: {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_hold_that_cannot_be_refused_stops_the_load() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.invalidate_fails = true;
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoadError::HeldNotRefused(_)), "{err}");
        assert!(
            runner.seen.lock().unwrap().is_none(),
            "the engine was never asked"
        );
    }

    /// A node that never heard of the root cannot follow it: the load goes on.
    #[tokio::test]
    async fn an_unknown_root_does_not_stop_the_load() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.knows_root = false;
        let runner = Runner::new(LoadOutcome::Loaded);
        assert!(load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD
            },
            Some(&env())
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn a_refused_block_on_the_chain_after_the_load_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.chain.insert(50, ROOT.to_string());
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            LoadError::HeldRootOnChain {
                height: 50,
                root: ROOT.into()
            }
        );
    }

    #[tokio::test]
    async fn the_engines_refusal_is_passed_on() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let node = Node::regtest();
        let why = "The base block header (x) is part of an invalid chain";
        let runner = Runner::new(LoadOutcome::Failed(why.into()));
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds::none(),
            Some(&env()),
        )
        .await
        .unwrap_err();
        assert_eq!(err, LoadError::Engine(why.into()));
    }

    #[tokio::test]
    async fn a_pair_that_no_longer_checks_out_is_not_loaded() {
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let v = view(&node).await;
        // One operator.
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_P, R_DAT);
        let err = load(&node, &runner, &pair, &v, &Holds::none(), Some(&env()))
            .await
            .unwrap_err();
        assert!(matches!(err, LoadError::NotConfirmed(_)), "{err}");
        // A file changed on disk since it was downloaded.
        let tmp = tempfile::tempdir().unwrap();
        let mut changed = R_DAT.to_vec();
        changed[7] ^= 1;
        let pair = pair_on_disk(tmp.path(), R_PC, &changed);
        let err = load(&node, &runner, &pair, &v, &Holds::none(), Some(&env()))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("not the one the statement signs"),
            "{err}"
        );
        // A node on another chain.
        let mut mainnet = Node::regtest();
        mainnet.genesis = crate::operators::MAINNET_GENESIS;
        let v = view(&mainnet).await;
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let err = load(&mainnet, &runner, &pair, &v, &Holds::none(), Some(&env()))
            .await
            .unwrap_err();
        assert!(matches!(err, LoadError::NotConfirmed(_)), "{err}");
        assert!(runner.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_pinned_pair_is_held_to_its_compiled_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut pair = pair_on_disk(tmp.path(), R_P, R_DAT);
        pair.kind = PairKind::Pinned;
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds::none(),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("compiled into the app"), "{err}");
    }

    #[test]
    fn a_refused_snapshot_chainstate_is_moved_aside_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(set_aside_snapshot_chainstate(tmp.path(), 1).unwrap(), None);
        let cs_dir = tmp.path().join("chainstate_snapshot");
        std::fs::create_dir_all(&cs_dir).unwrap();
        std::fs::write(cs_dir.join("attested_assumeutxo"), b"x").unwrap();
        let moved = set_aside_snapshot_chainstate(tmp.path(), 1_790_000_000)
            .unwrap()
            .unwrap();
        assert!(!cs_dir.exists());
        assert!(moved.join("attested_assumeutxo").exists());
        assert!(moved.ends_with("chainstate_snapshot.refused-1790000000"));
    }
}
````

In `crates/btx-core/src/lib.rs`, before `pub mod confirmed_snapshot;`:

````rust
pub mod confirmed_load;
pub mod confirmed_snapshot;
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- confirmed_load`
Expected: compile errors, among them `cannot find trait `LoadRunner``, `cannot find function `load``, and `enum `LoadOutcome` is private`.

- [ ] **Step 3: Open up the engine call in `snapshot.rs`**

Replace:

````rust
/// What a `loadtxoutset` / `loadtxoutsetattested` call came to.
#[derive(Debug, PartialEq)]
enum LoadOutcome {
````

with:

````rust
/// What a `loadtxoutset` / `loadtxoutsetattested` call came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
````

Then replace the head of `run_load`:

````rust
/// Run one snapshot-load RPC through btx-cli. The load can take a while to
/// read+validate the snapshot file; run it on a blocking thread with
/// rpcclienttimeout=0 (no client-side timeout), mirroring the faststart
/// wrapper's documented invocation.
async fn run_load(btx_cli: &Path, datadir: &Path, method: &str, files: &[&Path]) -> LoadOutcome {
    let cli = btx_cli.to_path_buf();
    let dd = datadir.to_path_buf();
    let method = method.to_string();
    let files: Vec<PathBuf> = files.iter().map(|f| f.to_path_buf()).collect();
    let result = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cli);
        cmd.arg(format!("-datadir={}", dd.display()))
            .arg("-rpcclienttimeout=0")
            .arg(&method)
            .args(&files);
````

with:

````rust
/// Run one snapshot-load RPC through btx-cli on `datadir`.
async fn run_load(btx_cli: &Path, datadir: &Path, method: &str, files: &[&Path]) -> LoadOutcome {
    run_cli_load(
        btx_cli,
        &[format!("-datadir={}", datadir.display())],
        method,
        files,
    )
    .await
}

/// Run one snapshot-load RPC through btx-cli. The load can take a while to
/// read+validate the snapshot file; run it on a blocking thread with
/// rpcclienttimeout=0 (no client-side timeout), mirroring the faststart
/// wrapper's documented invocation. `args` come before the method: the
/// datadir, and on regtest the network and port.
pub async fn run_cli_load(
    btx_cli: &Path,
    args: &[String],
    method: &str,
    files: &[&Path],
) -> LoadOutcome {
    let cli = btx_cli.to_path_buf();
    let args = args.to_vec();
    let method = method.to_string();
    let files: Vec<PathBuf> = files.iter().map(|f| f.to_path_buf()).collect();
    let result = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cli);
        cmd.args(&args)
            .arg("-rpcclienttimeout=0")
            .arg(&method)
            .args(&files);
````

(The `match result { ... }` tail of the function is unchanged.)

- [ ] **Step 4: Report the replay context**

In `crates/btx-core/src/node_api.rs`, in `pub struct MatmulTrustedStatus`, after the `trusted_mirror` field:

````rust
    #[serde(default)]
    pub trusted_mirror: bool,
    /// Display order. Absent on a node with no pin and no key.
    #[serde(default)]
    pub replay_authority_context: Option<String>,
}
````

In `crates/btx-core/src/role.rs`, the test helper `fn status(mode: &str, key: bool)` builds the struct literally; add the field:

````rust
            trusted_mirror: mode == "trusted",
            replay_authority_context: None,
        }
````

- [ ] **Step 5: Write the implementation**

Insert above `#[cfg(test)]` in `crates/btx-core/src/confirmed_load.rs`:

````rust
//! The one loading path for a signed snapshot, the same for setup,
//! Fast-forward and every kind of node (docs/decisions/2026-09-29-every-node-
//! starts-near-the-tip.md, section 7, steps 3 to 6).
//!
//! By the time a pair gets here, `crate::attested_snapshot` has downloaded
//! and checked it. This checks it again against the node that is about to
//! load it (its chain, its replay context, its pins), because the engine
//! checks only what it pins and nothing else:
//!
//! 1. The pair on disk passes every check again: a confirmed pair
//!    [`crate::confirmed_snapshot::check`] and its file's double SHA-256,
//!    the pinned pair its compiled SHA-256s.
//! 2. Every block the app refuses (`crate::known_invalid`) is refused now,
//!    in order, as the fork check does every 30 seconds. The engine then
//!    refuses a snapshot whose base sits above any of them ("part of an
//!    invalid chain", proven on regtest by `tests/confirmed_snapshot_regtest.rs`).
//!    If one cannot be refused, nothing is loaded.
//! 3. A trimmed manifest with every signature from a key this node pins, in
//!    order, and no other: the engine refuses a manifest carrying any key it
//!    does not pin.
//! 4. `loadtxoutsetattested`, which the engine allows only in mirror mode.
//! 5. After the load, the block at each refused height must not be the
//!    refused block. If it ever were, the caller stops the node and discards
//!    the snapshot ([`set_aside_snapshot_chainstate`]).

use crate::attested_snapshot::{self, PairKind, ReadyPair};
use crate::confirmed_snapshot::{self as cs, NodeView};
use crate::known_invalid::{self, HeldBranch, KnownInvalidBlock, Refusal};
use crate::rpc::Rpc;
use crate::snapshot::LoadOutcome;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Runs `loadtxoutsetattested`. The app's is [`CliRunner`]; tests script one.
#[async_trait::async_trait]
pub trait LoadRunner: Send + Sync {
    async fn load_attested(&self, file: &Path, manifest: &Path) -> LoadOutcome;
}

/// `btx-cli` with no client timeout, the way `crate::snapshot` has always
/// loaded: a load reads the whole file and can take minutes.
pub struct CliRunner {
    pub btx_cli: PathBuf,
    /// Everything before the method: `-datadir=...` for the app, plus
    /// `-regtest` and `-rpcport=...` in the regtest test.
    pub args: Vec<String>,
}

impl CliRunner {
    pub fn for_datadir(btx_cli: &Path, datadir: &Path) -> Self {
        Self {
            btx_cli: btx_cli.to_path_buf(),
            args: vec![format!("-datadir={}", datadir.display())],
        }
    }
}

#[async_trait::async_trait]
impl LoadRunner for CliRunner {
    async fn load_attested(&self, file: &Path, manifest: &Path) -> LoadOutcome {
        crate::snapshot::run_cli_load(
            &self.btx_cli,
            &self.args,
            "loadtxoutsetattested",
            &[file, manifest],
        )
        .await
    }
}

/// The blocks the app refuses, as this load sees them.
#[derive(Debug, Clone, Copy)]
pub struct Holds<'a> {
    pub invalid: &'a [KnownInvalidBlock],
    pub held: &'a [HeldBranch],
}

impl Holds<'static> {
    /// The compiled lists, or none when the operator switched refusal off
    /// (`EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`).
    pub fn compiled() -> Self {
        if known_invalid::refusal_enabled() {
            Self {
                invalid: known_invalid::KNOWN_INVALID_BLOCKS,
                held: known_invalid::HELD_BRANCHES,
            }
        } else {
            Self::none()
        }
    }

    pub fn none() -> Self {
        Self {
            invalid: &[],
            held: &[],
        }
    }
}

impl Holds<'_> {
    /// (height, block) of everything refused, invalid blocks first.
    fn roots(&self) -> Vec<(u64, &'static str)> {
        self.invalid
            .iter()
            .map(|b| (b.height, b.hash))
            .chain(self.held.iter().map(|h| (h.height, h.root)))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The pair no longer passes the checks against the live node.
    NotConfirmed(String),
    /// A refused block could not be refused, so nothing was loaded.
    HeldNotRefused(String),
    /// The engine refused the load. The chainstate is untouched.
    Engine(String),
    /// After the load a refused block is on this node's chain. The caller
    /// stops the node and discards the snapshot.
    HeldRootOnChain {
        height: u64,
        root: String,
    },
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotConfirmed(e) => write!(f, "the snapshot no longer checks out: {e}"),
            LoadError::HeldNotRefused(e) => write!(f, "a refused block could not be refused yet: {e}"),
            LoadError::Engine(e) => write!(f, "the engine did not load it: {e}"),
            LoadError::HeldRootOnChain { height, root } => write!(
                f,
                "after the load, block {root} at {height} is on this node's chain, which the app refuses"
            ),
            LoadError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A finished load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub height: u64,
    /// Signatures in the trimmed manifest the engine read.
    pub signatures: usize,
    /// The node's chain was already past the base ("Work does not exceed
    /// active chainstate"). Nothing changed; the caller counts it as loaded.
    pub superseded: bool,
}

/// What the running node says about itself, for [`cs::check`].
pub async fn node_view(rpc: &dyn Rpc, pinned: &[&str], start_height: u64) -> NodeView {
    let genesis = rpc
        .call("getblockhash", json!([0]))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string));
    let replay_context = crate::node_api::get_matmul_trusted_status(rpc)
        .await
        .ok()
        .and_then(|s| s.replay_authority_context);
    NodeView {
        genesis,
        replay_context,
        start_height,
        pinned: cs::pinned_keys(pinned),
    }
}

/// Where the trimmed manifest goes: beside the pair, under a name the sweep
/// removes with it.
pub fn trimmed_manifest_path(pair: &ReadyPair) -> PathBuf {
    pair.manifest
        .with_file_name(format!("loaded-{}.manifest", pair.height))
}

fn recheck(
    pair: &ReadyPair,
    view: &NodeView,
    regtest_env: Option<&str>,
) -> Result<cs::Manifest, LoadError> {
    let bytes =
        std::fs::read(&pair.manifest).map_err(|e| LoadError::Io(format!("manifest: {e}")))?;
    let m = cs::parse(&bytes).map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
    if view.genesis.is_none() {
        return Err(LoadError::NotConfirmed(
            "the node did not say which chain it is on".into(),
        ));
    }
    match pair.kind {
        PairKind::Confirmed => {
            cs::check(&m, view, regtest_env).map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
            let file =
                std::fs::read(&pair.file).map_err(|e| LoadError::Io(format!("snapshot: {e}")))?;
            let mut h = cs::FileHasher::default();
            h.update(&file);
            let (len, _, double) = h.finish();
            if !cs::file_matches(&m.statement, len, &double) {
                return Err(LoadError::NotConfirmed(
                    "the file is not the one the statement signs".into(),
                ));
            }
        }
        PairKind::Pinned => {
            let pin = attested_snapshot::pinned_pair();
            let same = |p: &Path, sha: &str| {
                matches!(crate::snapshot::verify_file_sha256(p, sha), Ok(true))
            };
            if pair.height != pin.height
                || !same(&pair.manifest, &pin.manifest_sha256)
                || !same(&pair.file, &pin.sha256)
            {
                return Err(LoadError::NotConfirmed(
                    "the pinned pair on disk is not the one compiled into the app".into(),
                ));
            }
            cs::node_agrees(&m.statement, view)
                .map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
        }
    }
    Ok(m)
}

/// Section 7, steps 3 to 6. See the module doc.
pub async fn load(
    rpc: &dyn Rpc,
    runner: &dyn LoadRunner,
    pair: &ReadyPair,
    view: &NodeView,
    holds: &Holds<'_>,
    regtest_env: Option<&str>,
) -> Result<Loaded, LoadError> {
    let m = recheck(pair, view, regtest_env)?;

    for block in holds.invalid {
        match known_invalid::refuse(rpc, block).await {
            Refusal::Refused | Refusal::NotKnownYet => {}
            other => {
                return Err(LoadError::HeldNotRefused(format!(
                    "{} at {}: {other:?}",
                    block.hash, block.height
                )))
            }
        }
    }
    for (branch, outcome) in known_invalid::refuse_held_in_order(rpc, holds.held).await {
        if !matches!(outcome, Refusal::Refused | Refusal::NotKnownYet) {
            return Err(LoadError::HeldNotRefused(format!(
                "{} at {}: {outcome:?}",
                branch.root, branch.height
            )));
        }
    }

    let trimmed = cs::trim_to_pinned(&m, &view.pinned);
    if trimmed.signatures.is_empty() {
        return Err(LoadError::NotConfirmed(
            "no signature is from a key this node pins".into(),
        ));
    }
    let path = trimmed_manifest_path(pair);
    let tmp = path.with_extension("partial");
    std::fs::write(&tmp, trimmed.to_bytes())
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| LoadError::Io(format!("write {}: {e}", path.display())))?;

    let superseded = match runner.load_attested(&pair.file, &path).await {
        LoadOutcome::Loaded => false,
        LoadOutcome::Superseded => true,
        LoadOutcome::Failed(e) => return Err(LoadError::Engine(e)),
    };

    for (height, root) in holds.roots() {
        let at = rpc
            .call("getblockhash", json!([height]))
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string));
        if at.as_deref() == Some(root) {
            return Err(LoadError::HeldRootOnChain {
                height,
                root: root.to_string(),
            });
        }
    }
    Ok(Loaded {
        height: pair.height,
        signatures: trimmed.signatures.len(),
        superseded,
    })
}

/// Move a snapshot chainstate the app refuses out of the engine's way, as the
/// engine itself does with one that fails its background check. `network_dir`
/// is the datadir on mainnet. Call only with the node stopped. `Ok(None)`
/// when there was none.
pub fn set_aside_snapshot_chainstate(
    network_dir: &Path,
    now_unix: u64,
) -> std::io::Result<Option<PathBuf>> {
    let from = network_dir.join("chainstate_snapshot");
    if !from.exists() {
        return Ok(None);
    }
    let to = network_dir.join(format!("chainstate_snapshot.refused-{now_unix}"));
    std::fs::rename(&from, &to)?;
    Ok(Some(to))
}
````

- [ ] **Step 6: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- confirmed_load role:: snapshot::`
Expected: all pass; `confirmed_load` shows `8 passed`. `a_confirmed_pair_is_trimmed_to_the_pins_and_loaded` proves the engine is handed the producer's own 335-byte file after trimming PCD to P.

- [ ] **Step 7: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cargo test --locked
cd ../..
````
````bash
git add crates/btx-core/src/confirmed_load.rs crates/btx-core/src/snapshot.rs crates/btx-core/src/node_api.rs crates/btx-core/src/role.rs crates/btx-core/src/lib.rs
git commit -m "core: one loading path for a signed snapshot, refused blocks first, pinned signatures only" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 5: The start-path loader chooses and reports (`snapshot.rs`)

**Files:**
- Modify: `crates/btx-core/src/snapshot.rs` (`ensure_snapshot_loaded`, `ensure_snapshot_loaded_with`, new `SignedLoad`, `SnapshotOutcome`, private `Signed`, `load_signed`; tests)
- Modify: `apps/node/src-tauri/src/commands.rs:1247-1256` (interim call site, replaced in Task 9)

**Interfaces:**
- Consumes: Task 3 (`attested_snapshot::{prepare_start, fallback_start}`), Task 4 (`confirmed_load::{node_view, load, CliRunner, Holds, LoadError}`), `operators::regtest_env`, `node::BTX_TRUSTED_ATTESTATION_PUBKEYS`.
- Produces (Task 9, plan 2):
  - `pub enum SignedLoad { None, Mirror, SignedOnly }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub enum SnapshotOutcome { AlreadyLoaded, SignedLoaded { height: u64 }, CompiledLoaded, NotLoaded(String), HeldRootOnChain(String) }`
  - `pub fn ensure_snapshot_loaded_with(rpc: RpcClient, btx_cli: PathBuf, datadir: PathBuf, anchor_height: u64, flags: Arc<dyn SnapshotFlags>, signed: SignedLoad) -> tokio::task::JoinHandle<SnapshotOutcome>` (was `prefer_attested: bool` and `()`)
  - `ensure_snapshot_loaded(...)` keeps its signature (drops the handle).

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/snapshot.rs`, inside `mod tests`, immediately before `#[tokio::test] async fn a_right_size_wrong_sha_file_is_deleted_and_refetched()`, add:

````rust
    /// A flag the tests can read back.
    struct Flag(AtomicBool);
    impl SnapshotFlags for Flag {
        fn loaded(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn mark_loaded(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    const CHAINSTATES_WITH_SNAPSHOT: &str = r#"{"result":{"headers":100,"chainstates":[{"blocks":99,"validated":true},{"blocks":100,"snapshot_blockhash":"ab"}]},"error":null,"id":"easybtx"}"#;

    /// A signed-only load (a validating node's mirror launch, Fast-forward)
    /// does not take the flag's word that a snapshot is loaded: Fast-forward
    /// sets the chain aside under a flag that still says so. It asks the node.
    #[tokio::test]
    async fn a_signed_only_load_asks_the_node_whatever_the_flag_says() {
        let mut server = mockito::Server::new_async().await;
        let asked = server
            .mock("POST", mockito::Matcher::Any)
            .with_body(CHAINSTATES_WITH_SNAPSHOT)
            .expect(1)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let outcome = ensure_snapshot_loaded_with(
            RpcClient::new(server.url(), "u", "p"),
            PathBuf::from("/nonexistent/btx-cli"),
            dir.path().to_path_buf(),
            219_000,
            Arc::new(Flag(AtomicBool::new(true))),
            SignedLoad::SignedOnly,
        )
        .await
        .unwrap();
        assert_eq!(outcome, SnapshotOutcome::AlreadyLoaded);
        asked.assert_async().await;
    }

    #[tokio::test]
    async fn an_ordinary_launch_on_a_loaded_flag_asks_nothing_and_says_so() {
        let mut server = mockito::Server::new_async().await;
        let never = server
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        for signed in [SignedLoad::None, SignedLoad::Mirror] {
            let outcome = ensure_snapshot_loaded_with(
                RpcClient::new(server.url(), "u", "p"),
                PathBuf::from("/nonexistent/btx-cli"),
                dir.path().to_path_buf(),
                219_000,
                Arc::new(Flag(AtomicBool::new(true))),
                signed,
            )
            .await
            .unwrap();
            assert_eq!(outcome, SnapshotOutcome::AlreadyLoaded, "{signed:?}");
        }
        never.assert_async().await;
    }

    #[tokio::test]
    async fn a_snapshot_chainstate_found_marks_the_flag() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_body(CHAINSTATES_WITH_SNAPSHOT)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let outcome = ensure_snapshot_loaded_with(
            RpcClient::new(server.url(), "u", "p"),
            PathBuf::from("/nonexistent/btx-cli"),
            dir.path().to_path_buf(),
            219_000,
            flag.clone(),
            SignedLoad::Mirror,
        )
        .await
        .unwrap();
        assert_eq!(outcome, SnapshotOutcome::AlreadyLoaded);
        assert!(flag.loaded());
        assert!(snapshot_marker_present(dir.path()));
    }
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot::`
Expected: compile errors `cannot find type `SignedLoad`` / `SnapshotOutcome` and `no method named `unwrap` found for unit type `()``.

- [ ] **Step 3: Write the implementation**

In `crates/btx-core/src/snapshot.rs`, replace everything from the line `/// Guarantee the assumeutxo snapshot actually gets loaded — on ANY startup path.` down to, not including, `/// Wait for the node's headers to reach `target`` (that is: `ensure_snapshot_loaded`, the old `ensure_snapshot_loaded_with` and Task 3's interim block) with:

````rust
/// Guarantee the assumeutxo snapshot actually gets loaded — on ANY startup path.
///
/// A partial first run can leave `<datadir>/faststart/snapshot.dat` on disk
/// WITHOUT ever calling `loadtxoutset` (e.g. the installer crashed before the
/// snapshot-load step), leaving the node in genuine IBD at height 0. This spawns
/// a BACKGROUND task (never blocks setup) that, when a snapshot.dat is present and
/// `getchainstates` reports no snapshot chainstate yet:
///   1. WAITS for the node's headers to reach the snapshot anchor —
///      `loadtxoutset` is rejected until then. The wait is PROGRESS-based:
///      it only gives up after ~10 min with zero header movement, never on a
///      wall clock (a from-genesis header sync can run an hour+).
///   2. Runs `btx-cli loadtxoutset` so the node fast-syncs to the snapshot height.
///
/// Idempotent + best-effort: no-ops when a snapshot chainstate already exists or
/// no snapshot.dat is present, and logs+ignores every failure (the node still
/// syncs the slow way).
pub fn ensure_snapshot_loaded(
    rpc: RpcClient,
    btx_cli: PathBuf,
    datadir: PathBuf,
    anchor_height: u64,
    flags: Arc<dyn SnapshotFlags>,
) {
    // The task runs on; nothing here waits for it.
    drop(ensure_snapshot_loaded_with(
        rpc,
        btx_cli,
        datadir,
        anchor_height,
        flags,
        SignedLoad::None,
    ));
}

/// Which signed snapshot a launch loads before the compiled one
/// (docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedLoad {
    /// A validating node's ordinary launch: the compiled snapshot only. The
    /// engine refuses a signed load outside mirror mode.
    None,
    /// A node that follows signatures: a confirmed pair, else the pinned
    /// pair, else the compiled snapshot.
    Mirror,
    /// A signed pair and nothing else: a validating node's one mirror launch
    /// (`crate::node::begin_mirror_load`), whose caller then restarts it as a
    /// validating node that loads the compiled snapshot if this came to
    /// nothing; and any node during Fast-forward, whose caller rolls back.
    SignedOnly,
}

/// What a background load came to, for a caller that has to act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOutcome {
    /// A snapshot chainstate was there already, or the flag said so.
    AlreadyLoaded,
    SignedLoaded {
        height: u64,
    },
    CompiledLoaded,
    /// Nothing was loaded, and why.
    NotLoaded(String),
    /// A signed load put a block the app refuses on the chain. The caller
    /// stops the node and sets the snapshot chainstate aside.
    HeldRootOnChain(String),
}

enum Signed {
    Loaded(u64),
    NotLoaded(String),
    HeldRootOnChain(String),
}

/// A confirmed or pinned pair, checked against this node and loaded through
/// `crate::confirmed_load`.
async fn load_signed(
    rpc: &RpcClient,
    btx_cli: &Path,
    datadir: &Path,
    anchor_height: u64,
) -> Signed {
    use crate::confirmed_load::{self, CliRunner, Holds, LoadError};
    let start = crate::attested_snapshot::fallback_start(anchor_height);
    let pins = crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS;
    let view = confirmed_load::node_view(rpc, &pins, start).await;
    let Some(pair) = crate::attested_snapshot::prepare_start(datadir, &view, anchor_height).await
    else {
        return Signed::NotLoaded("no signed snapshot is available".into());
    };
    if !wait_for_headers(rpc, datadir, pair.height).await {
        return Signed::NotLoaded(format!(
            "headers stalled short of the signed snapshot's base {}",
            pair.height
        ));
    }
    // A peer may have advanced past / loaded a snapshot during the wait.
    if matches!(get_chainstates(rpc).await, Ok(cs) if cs.snapshot().is_some()) {
        return Signed::Loaded(pair.height);
    }
    eprintln!(
        "[snapshot] headers at {}; loading the signed snapshot (loadtxoutsetattested)",
        pair.height
    );
    let runner = CliRunner::for_datadir(btx_cli, datadir);
    let env = crate::operators::regtest_env();
    match confirmed_load::load(
        rpc,
        &runner,
        &pair,
        &view,
        &Holds::compiled(),
        env.as_deref(),
    )
    .await
    {
        Ok(done) => {
            eprintln!(
                "[snapshot] signed snapshot {} loaded ({} signature(s) kept)",
                done.height, done.signatures
            );
            Signed::Loaded(done.height)
        }
        Err(e @ LoadError::HeldRootOnChain { .. }) => Signed::HeldRootOnChain(e.to_string()),
        Err(e) => Signed::NotLoaded(e.to_string()),
    }
}

/// [`ensure_snapshot_loaded`], with a signed snapshot first as `signed`
/// says. Anything short of a loaded signed pair (none published or pinned
/// above the anchor, a failed download, headers that stall below its base, a
/// refused manifest) falls through to the compiled snapshot exactly as
/// [`ensure_snapshot_loaded`] loads it, except on a validating node's mirror
/// launch, which loads nothing else; a failed signed load leaves the
/// chainstate untouched, so that path is still open.
pub fn ensure_snapshot_loaded_with(
    rpc: RpcClient,
    btx_cli: PathBuf,
    datadir: PathBuf,
    anchor_height: u64,
    flags: Arc<dyn SnapshotFlags>,
    signed: SignedLoad,
) -> tokio::task::JoinHandle<SnapshotOutcome> {
    tokio::spawn(async move {
        // FAST PATH (returning node): if a prior run already loaded the snapshot,
        // the persisted flag says so and the snapshot chainstate is on disk —
        // there is nothing to load. Return BEFORE the up-to-30-min header-anchor
        // wait below. Without this, the cold-start race where `getchainstates`
        // transiently doesn't yet report the snapshot chainstate would sink an
        // already-synced node into that long wait, pinning the UI on an early
        // setup phase for many minutes after a relaunch/heal (2026-05-29 invest.).
        // Not for a signed-only load: a validating node's mirror launch and
        // Fast-forward exist to load, and `getchainstates` below answers for
        // them (Fast-forward sets the chain aside under a flag that still
        // says "loaded" until it resets it).
        if signed != SignedLoad::SignedOnly && flags.loaded() {
            // Backfill the shared cross-process marker for installs that loaded
            // BEFORE the marker existed, so reclaim's marker gate can proceed.
            if !snapshot_marker_present(&datadir) {
                mark_snapshot_marker(&datadir);
            }
            return SnapshotOutcome::AlreadyLoaded;
        }
        // Already have a snapshot chainstate? Nothing to do — but persist the
        // loaded flag so later reclaim runs can safely drop snapshot.dat.
        match get_chainstates(&rpc).await {
            Ok(cs) if cs.snapshot().is_some() => {
                flags.mark_loaded();
                mark_snapshot_marker(&datadir);
                return SnapshotOutcome::AlreadyLoaded;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[snapshot] getchainstates unavailable ({e}); skipping snapshot load");
                return SnapshotOutcome::NotLoaded(format!("getchainstates unavailable: {e}"));
            }
        }

        if signed != SignedLoad::None {
            match load_signed(&rpc, &btx_cli, &datadir, anchor_height).await {
                Signed::Loaded(height) => {
                    flags.mark_loaded();
                    mark_snapshot_marker(&datadir);
                    return SnapshotOutcome::SignedLoaded { height };
                }
                Signed::HeldRootOnChain(why) => {
                    eprintln!("[snapshot] {why}");
                    return SnapshotOutcome::HeldRootOnChain(why);
                }
                Signed::NotLoaded(why) if signed == SignedLoad::SignedOnly => {
                    eprintln!("[snapshot] no signed snapshot loaded ({why})");
                    return SnapshotOutcome::NotLoaded(why);
                }
                Signed::NotLoaded(why) => {
                    eprintln!(
                        "[snapshot] no signed snapshot loaded ({why}); loading the compiled one"
                    );
                }
            }
        }

        let snapshot_path = datadir.join("faststart").join("snapshot.dat");
        if !snapshot_path.exists() {
            // No snapshot file to load (e.g. a clean full-sync install) — fine.
            return SnapshotOutcome::NotLoaded("no snapshot.dat".into());
        }

        if !wait_for_headers(&rpc, &datadir, anchor_height).await {
            eprintln!(
                "[snapshot] header sync stalled short of the snapshot anchor; \
                 leaving the node to sync the slow way (non-fatal)"
            );
            return SnapshotOutcome::NotLoaded("headers stalled short of the anchor".into());
        }

        // A peer may have advanced past / loaded the snapshot during the wait.
        if matches!(get_chainstates(&rpc).await, Ok(cs) if cs.snapshot().is_some()) {
            flags.mark_loaded();
            mark_snapshot_marker(&datadir);
            return SnapshotOutcome::AlreadyLoaded;
        }

        eprintln!("[snapshot] headers at anchor, no snapshot chainstate yet; running loadtxoutset");
        match run_load(&btx_cli, &datadir, "loadtxoutset", &[&snapshot_path]).await {
            LoadOutcome::Loaded => {
                eprintln!("[snapshot] loadtxoutset succeeded; snapshot chainstate activating");
                // C3: persist loaded=true ONLY here — on a confirmed successful
                // loadtxoutset. `disk::reclaim_disk` gates deleting snapshot.dat
                // on this flag AND the shared cross-process marker.
                flags.mark_loaded();
                mark_snapshot_marker(&datadir);
                SnapshotOutcome::CompiledLoaded
            }
            LoadOutcome::Superseded => {
                eprintln!("[snapshot] snapshot already superseded by active chain; continuing");
                // Active chain already past the snapshot — snapshot.dat is
                // safe to drop on the next reclaim, exactly as if it had
                // been loaded into the snapshot chainstate.
                flags.mark_loaded();
                mark_snapshot_marker(&datadir);
                SnapshotOutcome::AlreadyLoaded
            }
            LoadOutcome::Failed(e) => {
                eprintln!("[snapshot] loadtxoutset failed (non-fatal): {e}");
                SnapshotOutcome::NotLoaded(e)
            }
        }
    })
}
````

In `apps/node/src-tauri/src/commands.rs` (inside `start_node_inner`, the `else` branch after the header-bootstrap check) the call still passes a bool. Replace:

````rust
        btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            !signer_applies_here,
        );
````

with (interim, Task 9 replaces it):

````rust
        let signed = if signer_applies_here {
            btx_core::snapshot::SignedLoad::None
        } else {
            btx_core::snapshot::SignedLoad::Mirror
        };
        drop(btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            signed,
        ));
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot::` then `cd ../../apps/node/src-tauri && cargo check --locked --all-targets`
Expected: `snapshot::` all pass, including the three new ones; the app compiles.

- [ ] **Step 5: Format, lint, commit**

````bash
for c in crates/btx-core apps/node/src-tauri; do (cd $c && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious); done
(cd crates/btx-core && cargo test --locked)
````
````bash
git add crates/btx-core/src/snapshot.rs apps/node/src-tauri/src/commands.rs
git commit -m "core: the start-path loader tries a confirmed pair, then the pinned one, and says what it did" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 6: The pin rule for a validating node on a signed snapshot (`node.rs`, section 8)

**Files:**
- Modify: `crates/btx-core/src/node.rs` (`build_node_command` mirror and validating arms; new `BTX_TRUSTED_ATTESTATION_THRESHOLD`, `conf_pins`, `attested_snapshot_record`, `validating_snapshot_pin_args`; tests)

**Interfaces:**
- Consumes: `signing_key_self_pin`, `BTX_TRUSTED_ATTESTATION_PUBKEYS` (exist).
- Produces (Tasks 7, 8, 10; plan 2; the diary plan, which records nothing while `attested_snapshot_record` exists):
  - `pub const BTX_TRUSTED_ATTESTATION_THRESHOLD: u32 = 1`
  - `pub fn conf_pins(conf: &Path) -> Vec<String>`
  - `pub fn attested_snapshot_record(network_dir: &Path) -> PathBuf` (`<network_dir>/chainstate_snapshot/attested_assumeutxo`)
  - `pub fn validating_snapshot_pin_args(network_dir: &Path, mirror_pins: &[&str], already: &[String]) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

At the end of `mod tests` in `crates/btx-core/src/node.rs` (after `fn published_peers_are_the_ones_the_app_ships`), add:

````rust
    // ── Confirmed snapshots: the pin rule, the mirror launch, pins only grow ──

    fn signed_snapshot_datadir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easynode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("chainstate_snapshot")).unwrap();
        dir
    }

    fn pin_args(args: &[String]) -> Vec<String> {
        args.iter()
            .filter(|a| {
                a.starts_with("-matmultrustedpubkey=") || a.starts_with("-matmultrustedthreshold=")
            })
            .cloned()
            .collect()
    }

    /// Section 8: while the engine's attested record exists, a validating
    /// node pins every mirror key and threshold 1, in consensus mode still.
    /// Without the record it pins nothing it did not pin before.
    #[test]
    fn a_validating_node_on_a_signed_snapshot_pins_the_mirrors_keys() {
        let dir = signed_snapshot_datadir("pin-rule");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");

        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert!(pin_args(&args).is_empty(), "no record, no pins: {args:?}");

        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let mut want: Vec<String> = BTX_TRUSTED_ATTESTATION_PUBKEYS
            .iter()
            .map(|k| format!("-matmultrustedpubkey={k}"))
            .collect();
        want.push("-matmultrustedthreshold=1".into());
        for backend in [Backend::Cuda, Backend::Metal] {
            let (_, args, _) = build_node_command(btxd, &dir, &conf, backend);
            assert_eq!(pin_args(&args), want, "{backend:?}: {args:?}");
            assert!(!validation_modes(&args).contains(&"trusted"), "{args:?}");
        }
        // The engine retires the snapshot, the record goes, and so do the pins.
        std::fs::remove_file(attested_snapshot_record(&dir)).unwrap();
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert!(pin_args(&args).is_empty(), "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine refuses a duplicate pin: a key the conf pins, or the
    /// node's own, is never passed twice.
    #[test]
    fn the_pin_rule_never_repeats_a_pin() {
        let dir = signed_snapshot_datadir("pin-dupes");
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let wif = crate::signer::generate_wif();
        let own = crate::signer::wif_to_pubkey_hex(&wif).unwrap();
        std::fs::write(dir.join(crate::signer::SIGNER_KEY_FILE), format!("{wif}\n")).unwrap();
        let the_3060 = BTX_TRUSTED_ATTESTATION_PUBKEYS[3];
        let conf = dir.join("signing.conf");
        std::fs::write(
            &conf,
            format!(
                "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n\
                 matmultrustedpubkey={}\n",
                the_3060.to_ascii_uppercase()
            ),
        )
        .unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.9/lin/btxd"),
            &dir,
            &conf,
            Backend::Cuda,
        );
        let pins: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("-matmultrustedpubkey="))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for p in &pins {
            assert!(
                seen.insert(p.to_ascii_lowercase()),
                "pinned twice: {args:?}"
            );
        }
        assert!(
            pins.iter().any(|p| p.ends_with(&own)),
            "its own key: {args:?}"
        );
        assert!(
            !pins.iter().any(|p| p.ends_with(the_3060)),
            "the conf already pins the 3060: {args:?}"
        );
        assert_eq!(pins.len(), 1 + BTX_TRUSTED_ATTESTATION_PUBKEYS.len() - 1);
        // The pure rule, case-blind.
        let got = validating_snapshot_pin_args(&dir, &[the_3060], &[the_3060.to_ascii_uppercase()]);
        assert_eq!(got, vec!["-matmultrustedthreshold=1".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mirror_arm_is_the_same_on_a_signed_snapshot() {
        let dir = signed_snapshot_datadir("pin-mirror");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        let (_, before, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let (_, after, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(before, after);
        assert_eq!(validation_modes(&after), vec!["trusted"]);
        assert_eq!(
            after
                .iter()
                .filter(|a| a.starts_with("-matmultrustedthreshold="))
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::tests::a_validating_node_on node::tests::the_pin_rule node::tests::the_mirror_arm_is_the_same`
Expected: compile errors `cannot find function `attested_snapshot_record`` and `cannot find function `validating_snapshot_pin_args``.

- [ ] **Step 3: Write the implementation**

After `pub const BTX_TRUSTED_ATTESTATION_PUBKEYS: [&str; 4] = [ ... ];` (the closing `];` after `"02d5efca…4675",`), add:

````rust

/// The mirrors' threshold. It never rises: a mirror on a snapshot stored
/// with fewer pinned signatures than this does not start
/// (`pins_only_grow_and_the_threshold_stays` holds it).
pub const BTX_TRUSTED_ATTESTATION_THRESHOLD: u32 = 1;
````

In the mirror arm of `build_node_command`, replace `args.push("-matmultrustedthreshold=1".to_string());` with:

````rust
            args.push(format!(
                "-matmultrustedthreshold={BTX_TRUSTED_ATTESTATION_THRESHOLD}"
            ));
````

In the same function, the signer block `if !mirror_here && signs_here(conf) { ... }` ends with the self-pin (`if let Some(pubkey) = signing_key_self_pin(conf, datadir) { ... }` and two closing braces). Directly after that block's closing `}` and before the closing `}` of `if node_supports_matmul_rc_flags(btxd) {`, add:

````rust
        // A validating node on a signed snapshot pins the mirrors' keys too,
        // or the engine's start-up check of the stored manifest refuses to
        // start it (section 8 of the confirmed-snapshot decision). In
        // consensus mode they are telemetry: they skip no check.
        if !mirror_here {
            let mut already = conf_pins(conf);
            already.extend(signing_key_self_pin(conf, datadir));
            args.extend(validating_snapshot_pin_args(
                datadir,
                &BTX_TRUSTED_ATTESTATION_PUBKEYS,
                &already,
            ));
        }
````

After `pub fn signing_key_self_pin(...) { ... }`, add:

````rust
/// Every key the conf already pins (`matmultrustedpubkey=`), lowercase. The
/// engine refuses a duplicate pin, so the command line never repeats one.
pub fn conf_pins(conf: &Path) -> Vec<String> {
    std::fs::read_to_string(conf)
        .map(|text| {
            text.lines()
                .filter_map(|l| l.trim().strip_prefix("matmultrustedpubkey="))
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The engine's record that this node runs on a signed snapshot whose
/// background check has not finished (`SNAPSHOT_ATTESTED_ASSUMEUTXO_FILENAME`,
/// v0.34.9 `src/node/utxo_snapshot.h:254`). It goes away when the engine
/// retires the snapshot. `network_dir` is the datadir on mainnet.
pub fn attested_snapshot_record(network_dir: &Path) -> PathBuf {
    network_dir
        .join("chainstate_snapshot")
        .join("attested_assumeutxo")
}

/// The pins a validating node must carry while it runs on a signed snapshot
/// (the confirmed-snapshot decision, section 8): every key in `mirror_pins`
/// not already in `already` (the conf's pins and the node's own), and the
/// threshold the stored manifest was loaded under. Empty otherwise.
///
/// WHY. The engine re-checks the stored manifest at every start against the
/// pins and threshold of that moment, and refuses to start when the check
/// fails ("Attested snapshot manifest is present but failed verification
/// under the current authority configuration", `validation.cpp:22330`).
/// Measured on 2026-09-29, regtest and mainnet: a validating restart on a
/// signed snapshot starts with the signer pinned and not without it. In
/// consensus mode the pins are telemetry, never a reason to skip a check.
pub fn validating_snapshot_pin_args(
    network_dir: &Path,
    mirror_pins: &[&str],
    already: &[String],
) -> Vec<String> {
    if !attested_snapshot_record(network_dir).exists() {
        return Vec::new();
    }
    let mut args: Vec<String> = mirror_pins
        .iter()
        .filter(|k| !already.iter().any(|a| a.eq_ignore_ascii_case(k)))
        .map(|k| format!("-matmultrustedpubkey={k}"))
        .collect();
    args.push(format!(
        "-matmultrustedthreshold={BTX_TRUSTED_ATTESTATION_THRESHOLD}"
    ));
    args
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::`
Expected: all `node::` tests pass (see Global Constraints for the one known flake), including `a_validating_signer_pins_its_own_key_and_a_mirror_never_does` unchanged, which proves nothing changes without the engine's record.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/src/node.rs
git commit -m "node: a validating node on a signed snapshot pins the mirrors keys" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 7: One mirror launch for a validating node (`node.rs`, section 7 step 5)

**Files:**
- Modify: `crates/btx-core/src/node.rs` (`launches_as_mirror` split; new `host_follows_signatures`, `MirrorLoad`, `MIRROR_LOAD_MAX_AGE_SECS`, `mirror_load_is_fresh`, `begin_mirror_load`, `mirror_load_pending`, `mirror_load_marker_exists`, `end_mirror_load`, `mirror_load_wanted`; tests)

**Interfaces:**
- Consumes: Task 6's test helpers `signed_snapshot_datadir`, `pin_args`; `trusted_mirror_override`, `follows_signatures_by_choice` (exist).
- Produces (Tasks 9, 10; plan 2):
  - `pub fn launches_as_mirror(btxd, datadir, backend) -> bool` now also true while a fresh marker is pending (unless `EASYBTX_NODE_TRUSTED_MIRROR=0`); `pub fn host_follows_signatures(btxd, datadir, backend) -> bool` is the old rule
  - `pub struct MirrorLoad { height: u64, written_at: u64 }` (serde), `pub const MIRROR_LOAD_MAX_AGE_SECS: u64 = 21_600`
  - `pub fn mirror_load_is_fresh(&MirrorLoad, now: u64) -> bool`, `pub fn begin_mirror_load(&Path, height: u64) -> std::io::Result<()>`, `pub fn mirror_load_pending(&Path) -> Option<MirrorLoad>`, `pub fn mirror_load_marker_exists(&Path) -> bool`, `pub fn end_mirror_load(&Path)`
  - `pub fn mirror_load_wanted(host_validates, header_bootstrap_pending, snapshot_loaded, has_snapshot_chainstate, operator_forbids_mirror: bool) -> bool`

- [ ] **Step 1: Write the failing tests**

At the end of `mod tests` in `crates/btx-core/src/node.rs` (after Task 6's tests), add:

````rust
    /// Section 7, step 5: the marker makes exactly the load launch a mirror
    /// (and takes the signing key out of it), and clearing it gives the
    /// validating launch back.
    #[test]
    fn a_mirror_load_marker_makes_only_the_load_launch_a_mirror() {
        let dir = signed_snapshot_datadir("mirror-load");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        assert!(!launches_as_mirror(btxd, &dir, Backend::Cuda));
        assert_eq!(mirror_load_pending(&dir), None);

        begin_mirror_load(&dir, 232_000).unwrap();
        assert_eq!(mirror_load_pending(&dir).map(|m| m.height), Some(232_000));
        assert!(launches_as_mirror(btxd, &dir, Backend::Cuda));
        assert!(!host_follows_signatures(btxd, &dir, Backend::Cuda));
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");

        end_mirror_load(&dir);
        end_mirror_load(&dir); // idempotent
        assert!(!mirror_load_marker_exists(&dir));
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert_eq!(validation_modes(&args), vec!["consensus"], "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_or_broken_mirror_load_marker_is_ignored() {
        let now = 1_790_000_000;
        let m = |written_at| MirrorLoad {
            height: 232_000,
            written_at,
        };
        assert!(mirror_load_is_fresh(&m(now), now));
        assert!(mirror_load_is_fresh(
            &m(now - MIRROR_LOAD_MAX_AGE_SECS + 1),
            now
        ));
        assert!(!mirror_load_is_fresh(
            &m(now - MIRROR_LOAD_MAX_AGE_SECS),
            now
        ));
        assert!(
            !mirror_load_is_fresh(&m(now + 3_600), now),
            "from the future"
        );

        let dir = signed_snapshot_datadir("mirror-load-stale");
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        std::fs::write(
            dir.join(".load-snapshot-as-mirror"),
            r#"{"height":232000,"written_at":1}"#,
        )
        .unwrap();
        assert!(mirror_load_marker_exists(&dir));
        assert_eq!(mirror_load_pending(&dir), None);
        assert!(!launches_as_mirror(btxd, &dir, Backend::Cuda));
        std::fs::write(dir.join(".load-snapshot-as-mirror"), "not json").unwrap();
        assert_eq!(mirror_load_pending(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_fresh_validating_node_gets_a_mirror_launch() {
        assert!(mirror_load_wanted(true, false, false, false, false));
        assert!(
            !mirror_load_wanted(false, false, false, false, false),
            "a mirror loads in place"
        );
        assert!(
            !mirror_load_wanted(true, true, false, false, false),
            "header bootstrap first"
        );
        assert!(
            !mirror_load_wanted(true, false, true, false, false),
            "already loaded one"
        );
        assert!(
            !mirror_load_wanted(true, false, false, true, false),
            "a snapshot chainstate"
        );
        assert!(
            !mirror_load_wanted(true, false, false, false, true),
            "the operator said =0"
        );
    }
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::tests::a_mirror_load node::tests::a_stale node::tests::only_a_fresh`
Expected: compile errors `cannot find function `begin_mirror_load``, `cannot find struct `MirrorLoad``, `cannot find function `host_follows_signatures``.

- [ ] **Step 3: Write the implementation**

Replace the start of `launches_as_mirror`:

````rust
pub fn launches_as_mirror(btxd: &Path, datadir: &Path, backend: Backend) -> bool {
    // The owner's choice outranks the backend split, a Cuda host included,
    // and yields only to the operator's explicit =0.
    if follows_signatures_by_choice(datadir) && trusted_mirror_override() != Some(false) {
        return true;
    }
````

with:

````rust
pub fn launches_as_mirror(btxd: &Path, datadir: &Path, backend: Backend) -> bool {
    // A validating node's one mirror launch, to load a signed snapshot.
    if mirror_load_pending(datadir).is_some() && trusted_mirror_override() != Some(false) {
        return true;
    }
    host_follows_signatures(btxd, datadir, backend)
}

/// [`launches_as_mirror`] without the one-time mirror launch: does this host
/// follow signatures, or does it check blocks itself?
pub fn host_follows_signatures(btxd: &Path, datadir: &Path, backend: Backend) -> bool {
    // The owner's choice outranks the backend split, a Cuda host included,
    // and yields only to the operator's explicit =0.
    if follows_signatures_by_choice(datadir) && trusted_mirror_override() != Some(false) {
        return true;
    }
````

(The rest of the old body, from `let degraded_start = ...` on, is now the body of `host_follows_signatures`, unchanged.)

After Task 6's `validating_snapshot_pin_args`, add:

````rust
/// The one-time marker that makes the next launch of a validating node a
/// mirror launch, so it can load a signed snapshot (section 7, step 5): the
/// engine allows `loadtxoutsetattested` only in mirror mode. The same
/// pattern as the header bootstrap's `.header-bootstrap`: written before the
/// launch, read by [`launches_as_mirror`], cleared by the app after the load
/// with a restart as a validating node.
fn mirror_load_path(datadir: &Path) -> PathBuf {
    datadir.join(".load-snapshot-as-mirror")
}

/// A marker older than this is left from a run that died, and is ignored:
/// a load takes minutes, and a validating node must never stay a mirror for
/// longer than one.
pub const MIRROR_LOAD_MAX_AGE_SECS: u64 = 6 * 60 * 60;

/// What the marker holds: the base being loaded, and when it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorLoad {
    pub height: u64,
    pub written_at: u64,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pure half of [`mirror_load_pending`]: young enough, and not from the
/// future by more than a clock step.
pub fn mirror_load_is_fresh(m: &MirrorLoad, now: u64) -> bool {
    m.written_at <= now + 300 && now.saturating_sub(m.written_at) < MIRROR_LOAD_MAX_AGE_SECS
}

/// Mark the next launch of this datadir as the mirror launch that loads the
/// snapshot at `height`.
pub fn begin_mirror_load(datadir: &Path, height: u64) -> std::io::Result<()> {
    let m = MirrorLoad {
        height,
        written_at: unix_now(),
    };
    std::fs::write(
        mirror_load_path(datadir),
        serde_json::to_string(&m).map_err(std::io::Error::other)?,
    )
}

/// The pending mirror launch, if a fresh marker says so.
pub fn mirror_load_pending(datadir: &Path) -> Option<MirrorLoad> {
    let raw = std::fs::read_to_string(mirror_load_path(datadir)).ok()?;
    let m: MirrorLoad = serde_json::from_str(&raw).ok()?;
    mirror_load_is_fresh(&m, unix_now()).then_some(m)
}

/// A marker file is there, fresh or not.
pub fn mirror_load_marker_exists(datadir: &Path) -> bool {
    mirror_load_path(datadir).exists()
}

/// Clear the marker, so the next launch is the node's ordinary one.
pub fn end_mirror_load(datadir: &Path) {
    let path = mirror_load_path(datadir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[node] could not clear {}: {e}", path.display());
        }
    }
}

/// Should this start launch a validating node once as a mirror, to load a
/// signed snapshot? Only for a node that checks blocks itself, has never
/// loaded a snapshot, holds no snapshot chainstate, is not in its header
/// bootstrap (the load waits for the launch after it), and whose operator has
/// not said "never a mirror" (`EASYBTX_NODE_TRUSTED_MIRROR=0`).
pub fn mirror_load_wanted(
    host_validates: bool,
    header_bootstrap_pending: bool,
    snapshot_loaded: bool,
    has_snapshot_chainstate: bool,
    operator_forbids_mirror: bool,
) -> bool {
    host_validates
        && !header_bootstrap_pending
        && !snapshot_loaded
        && !has_snapshot_chainstate
        && !operator_forbids_mirror
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::`
Expected: all pass (see Global Constraints for the one known flake).

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/src/node.rs
git commit -m "node: a one-time mirror launch lets a validating node load a signed snapshot" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 8: Pins only grow, the threshold stays (section 12, first rule)

**Files:**
- Create: `crates/btx-core/src/pins_ever_shipped.txt`
- Modify: `crates/btx-core/src/node.rs` (tests only)

**Interfaces:**
- Consumes: `BTX_TRUSTED_ATTESTATION_PUBKEYS`, Task 6's `BTX_TRUSTED_ATTESTATION_THRESHOLD`.
- Produces: the checked-in list every later key addition appends to.

History checked while writing this plan: the list shipped with three keys in the first release (`4f3e63b`) and gained `02d5efca` in #134 (`5aa8887`, 0.6.30); nothing was ever removed.

- [ ] **Step 1: Write the failing test**

At the end of `mod tests` in `crates/btx-core/src/node.rs`, add:

````rust
    /// Section 12. `Err` names what broke the rule.
    fn pins_only_grow(ever: &[&str], now: &[&str]) -> Result<(), String> {
        let has = |list: &[&str], k: &str| list.iter().any(|x| x.eq_ignore_ascii_case(k));
        let removed: Vec<&str> = ever.iter().copied().filter(|k| !has(now, k)).collect();
        if !removed.is_empty() {
            return Err(format!("removed from the pins: {}", removed.join(", ")));
        }
        let unrecorded: Vec<&str> = now.iter().copied().filter(|k| !has(ever, k)).collect();
        if !unrecorded.is_empty() {
            return Err(format!(
                "pinned but not in pins_ever_shipped.txt: {}",
                unrecorded.join(", ")
            ));
        }
        Ok(())
    }

    fn pins_file() -> (Vec<&'static str>, u32) {
        let text = include_str!("pins_ever_shipped.txt");
        let mut keys = Vec::new();
        let mut threshold = 0;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            if let Some(t) = line.strip_prefix("threshold ") {
                threshold = t.trim().parse().unwrap();
            } else {
                keys.push(line.split_whitespace().next().unwrap());
            }
        }
        (keys, threshold)
    }

    #[test]
    fn pins_only_grow_and_the_threshold_stays() {
        let (ever, threshold) = pins_file();
        assert_eq!(ever.len(), 4, "the file lost a line");
        pins_only_grow(&ever, &BTX_TRUSTED_ATTESTATION_PUBKEYS).unwrap();
        assert!(
            BTX_TRUSTED_ATTESTATION_THRESHOLD <= threshold,
            "the mirrors' threshold rose to {BTX_TRUSTED_ATTESTATION_THRESHOLD}"
        );
        // Sabotage: a release that drops the 3060's key.
        let dropped: Vec<&str> = BTX_TRUSTED_ATTESTATION_PUBKEYS[..3].to_vec();
        let err = pins_only_grow(&ever, &dropped).unwrap_err();
        assert!(err.contains("02d5efca"), "{err}");
        // A key added without its line in the file.
        let mut added = BTX_TRUSTED_ATTESTATION_PUBKEYS.to_vec();
        added.push("0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464");
        assert!(pins_only_grow(&ever, &added).is_err());
    }
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::tests::pins_only_grow`
Expected: compile error `couldn't read `src/pins_ever_shipped.txt``.

- [ ] **Step 3: Add the list**

Create `crates/btx-core/src/pins_ever_shipped.txt`:

````text
# Every key BTX_TRUSTED_ATTESTATION_PUBKEYS (crates/btx-core/src/node.rs) has
# ever shipped, one per line, with the release that added it, and the highest
# threshold the mirrors have ever used.
#
# A key is never removed from the pins. The engine re-checks a stored
# snapshot manifest at every start against the pins of that moment, so a
# mirror on a snapshot signed by a removed key stops starting (measured on
# regtest, 2026-09-29). Retire a key with -matmulattestationblocklist, which
# the engine tolerates at restart. The same holds for the threshold: raise it
# above the pinned signatures a stored manifest carries and the node stops.
#
# Adding a key: add it to the list in node.rs and append a line here.
# The test `pins_only_grow_and_the_threshold_stays` compares the two.
03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa first release
0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c first release
028995b25c887ee03eb53a41312d33c8eccf48f261ecf9e91fe2b1e8e50373258a first release
02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675 0.6.30
threshold 1
````

- [ ] **Step 4: Run it to see it pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- node::tests::pins_only_grow`
Expected: `1 passed`. The test carries its own sabotage: a list without the 3060's key must fail naming `02d5efca`, and a key added without its line must fail too.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/src/node.rs crates/btx-core/src/pins_ever_shipped.txt
git commit -m "node: a test fails if a release removes a pinned key or raises the threshold" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 9: The start path: which load, one mirror launch, the restart after it (`commands.rs`)

**Files:**
- Modify: `apps/node/src-tauri/src/commands.rs` (`start_node_inner`; new `SIGNED_LOAD_FAILED`, `signed_load_for`, `prepare_mirror_load`, `mirror_load_end_message`, `set_aside_refused_snapshot`, `spawn_load_watch`, `after_snapshot_load`; tests)

**Interfaces:**
- Consumes: Task 3 (`prepare_start`, `fallback_start`), Task 2 (`NodeView`, `pinned_keys`), Task 4 (`set_aside_snapshot_chainstate`), Task 5 (`SignedLoad`, `SnapshotOutcome`, `ensure_snapshot_loaded_with` returning a handle), Task 7 (`mirror_load_pending`, `mirror_load_marker_exists`, `begin_mirror_load`, `end_mirror_load`, `mirror_load_wanted`, `host_follows_signatures`), and existing `attached_node_is_ours_to_stop`, `stop_node_inner`, `start_node_projected`, `set_phase`, `setup_log`, `snapshot_spec`, `node_backend`, `header_bootstrap_pending`, `header_bootstrap_wanted`, `trusted_mirror_override`.
- Produces (plan 2 extends them): `static SIGNED_LOAD_FAILED: AtomicBool`; `fn signed_load_for(mirror_load_launch: bool, follows_signatures: bool, failed_this_run: bool) -> SignedLoad`; `async fn prepare_mirror_load(&AppHandle, &AppState, btxd: &Path, datadir: &Path)`; `fn mirror_load_end_message(&SnapshotOutcome) -> String`; `fn spawn_load_watch(AppHandle, JoinHandle<SnapshotOutcome>, gen: u64, mirror_load_launch: bool)`; `async fn after_snapshot_load(&AppHandle, &State<AppState>, gen: u64, mirror_load_launch: bool, SnapshotOutcome) -> Result<(), String>`.

How the pieces meet, for a fresh validating install: setup, then the header bootstrap launch (unchanged), then the restart that ends it calls `start_node_inner`, where `prepare_mirror_load` finds a pair (confirmed, else pinned) and writes the marker; that launch is a mirror (the key line leaves the conf, `build_node_command` takes the mirror arm); the loader loads with `SignedOnly`; `after_snapshot_load` stops the node, clears the marker and starts it again as a validating node, which the pin rule (Task 6) lets start. A load that fails sets `SIGNED_LOAD_FAILED`, so the validating restart loads the compiled snapshot and nothing retries until the app restarts. A node that follows signatures loads a pair in place (`Mirror`), and a compiled one if that fails, as before.

- [ ] **Step 1: Write the failing tests**

At the end of `apps/node/src-tauri/src/commands.rs`, add:

````rust
#[cfg(test)]
mod signed_start_tests {
    use super::{mirror_load_end_message, signed_load_for};
    use btx_core::snapshot::{SignedLoad, SnapshotOutcome};

    #[test]
    fn each_launch_makes_the_signed_load_its_node_can_make() {
        assert_eq!(signed_load_for(true, false, false), SignedLoad::SignedOnly);
        assert_eq!(signed_load_for(true, true, true), SignedLoad::SignedOnly);
        assert_eq!(signed_load_for(false, true, false), SignedLoad::Mirror);
        assert_eq!(
            signed_load_for(false, true, true),
            SignedLoad::None,
            "a mirror whose signed load failed this run takes the compiled one"
        );
        assert_eq!(signed_load_for(false, false, false), SignedLoad::None);
    }
    #[test]
    fn the_end_of_a_mirror_launch_says_what_happens_next() {
        let loaded = mirror_load_end_message(&SnapshotOutcome::SignedLoaded { height: 232_000 });
        assert!(
            loaded.contains("232000") && loaded.contains("check new blocks itself"),
            "{loaded}"
        );
        let none = mirror_load_end_message(&SnapshotOutcome::NotLoaded("HTTP 404".into()));
        assert!(
            none.contains("HTTP 404") && none.contains("compiled"),
            "{none}"
        );
        let held = mirror_load_end_message(&SnapshotOutcome::HeldRootOnChain("block x".into()));
        assert!(held.contains("aside"), "{held}");
        for m in [loaded, none, held] {
            assert!(!m.contains('\u{2014}'), "no em-dash: {m}");
        }
    }
}
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd apps/node/src-tauri && cargo test --locked -- signed_start_tests`
Expected: compile errors `unresolved imports `super::mirror_load_end_message`, `super::signed_load_for``.

- [ ] **Step 3: Write the helpers**

In `apps/node/src-tauri/src/commands.rs`, directly before:

````rust
#[tauri::command]
pub async fn start_node(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
````

add:

````rust
/// Set when a signed load failed on this node in this run of the app: no
/// further mirror launch and no further signed load until the app restarts,
/// so a pair that fails cannot restart the node in a loop.
static SIGNED_LOAD_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Which signed load a launch makes. Pure, so every case has a test.
fn signed_load_for(
    mirror_load_launch: bool,
    follows_signatures: bool,
    failed_this_run: bool,
) -> btx_core::snapshot::SignedLoad {
    use btx_core::snapshot::SignedLoad;
    if mirror_load_launch {
        SignedLoad::SignedOnly
    } else if follows_signatures && !failed_this_run {
        SignedLoad::Mirror
    } else {
        SignedLoad::None
    }
}

/// Before a launch: begin a validating node's one mirror launch when it has
/// never loaded a snapshot and a signed pair is ready (confirmed, else the
/// pinned one). Resumes a load a stopped run began; drops a stale marker.
async fn prepare_mirror_load(app: &AppHandle, state: &AppState, btxd: &Path, datadir: &Path) {
    use btx_core::node;
    if SIGNED_LOAD_FAILED.load(Ordering::SeqCst) {
        node::end_mirror_load(datadir);
        return;
    }
    if node::mirror_load_marker_exists(datadir) {
        if node::mirror_load_pending(datadir).is_some() {
            return;
        }
        node::end_mirror_load(datadir);
    }
    let settings = NodeAppSettings::load(datadir);
    let wanted = node::mirror_load_wanted(
        !node::host_follows_signatures(btxd, datadir, node_backend()),
        node::header_bootstrap_pending(datadir) || node::header_bootstrap_wanted(datadir),
        settings.snapshot_loaded,
        datadir.join("chainstate_snapshot").exists(),
        node::trusted_mirror_override() == Some(false),
    );
    if !wanted {
        return;
    }
    set_phase(
        app,
        state,
        NodePhase::Warming {
            message: "Looking for a recent snapshot to start from…".to_string(),
        },
    )
    .await;
    let anchor = snapshot_spec().anchor_height;
    let view = btx_core::confirmed_snapshot::NodeView {
        start_height: btx_core::attested_snapshot::fallback_start(anchor),
        pinned: btx_core::confirmed_snapshot::pinned_keys(&node::BTX_TRUSTED_ATTESTATION_PUBKEYS),
        ..Default::default()
    };
    match btx_core::attested_snapshot::prepare_start(datadir, &view, anchor).await {
        Some(pair) => match node::begin_mirror_load(datadir, pair.height) {
            Ok(()) => setup_log(
                datadir,
                &format!(
                    "signed snapshot {} ready: this launch runs as a mirror once to load it",
                    pair.height
                ),
            ),
            Err(e) => eprintln!("[node-app] could not mark the mirror launch: {e}"),
        },
        None => setup_log(
            datadir,
            "no signed snapshot to start from; the node starts from the one compiled into its engine",
        ),
    }
}

/// What the log says when a validating node's mirror launch ends.
fn mirror_load_end_message(outcome: &btx_core::snapshot::SnapshotOutcome) -> String {
    use btx_core::snapshot::SnapshotOutcome as O;
    match outcome {
        O::SignedLoaded { height } => format!(
            "signed snapshot {height} loaded; restarting the node to check new blocks itself"
        ),
        O::AlreadyLoaded => {
            "a snapshot was already loaded; restarting the node to check new blocks itself".into()
        }
        O::HeldRootOnChain(why) => format!(
            "{why}; setting the snapshot aside and restarting the node from the compiled snapshot"
        ),
        O::NotLoaded(why) => format!(
            "no signed snapshot loaded ({why}); restarting the node to start from the compiled one"
        ),
        O::CompiledLoaded => "restarting the node to check new blocks itself".into(),
    }
}

/// Move a snapshot chainstate the app refuses aside, and forget it was loaded.
fn set_aside_refused_snapshot(datadir: &Path) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match btx_core::confirmed_load::set_aside_snapshot_chainstate(datadir, now) {
        Ok(Some(to)) => setup_log(
            datadir,
            &format!("refused snapshot chainstate moved to {}", to.display()),
        ),
        Ok(None) => {}
        Err(e) => eprintln!("[node-app] could not set the refused snapshot aside: {e}"),
    }
    NodeAppSettings::update(datadir, |s| s.snapshot_loaded = false);
    btx_core::snapshot::clear_snapshot_marker(datadir);
}

/// Wait for a background load and act on it ([`after_snapshot_load`]). A
/// plain function, as `spawn_status_refresher` is, so the task it spawns can
/// restart the node without the start path's future containing itself.
fn spawn_load_watch(
    app: AppHandle,
    handle: tokio::task::JoinHandle<btx_core::snapshot::SnapshotOutcome>,
    gen: u64,
    mirror_load_launch: bool,
) {
    tauri::async_runtime::spawn(async move {
        let outcome = handle.await.unwrap_or_else(|e| {
            btx_core::snapshot::SnapshotOutcome::NotLoaded(format!("the load task ended: {e}"))
        });
        let state = app.state::<AppState>();
        if let Err(e) = after_snapshot_load(&app, &state, gen, mirror_load_launch, outcome).await {
            eprintln!("[node-app] the restart after a snapshot load failed: {e}");
        }
    });
}

/// After a background load: end a validating node's mirror launch with a
/// restart as a validating node, or discard a signed snapshot that put a
/// refused block on the chain. Nothing, when a stop or another restart got
/// here first: the marker then waits for the next start.
///
/// The ORDER is the header bootstrap's: stop, then clear the marker, then
/// start. An app that dies in between leaves the marker, and the next start
/// resumes the load rather than running a mirror with nothing to say so.
async fn after_snapshot_load(
    app: &AppHandle,
    state: &State<'_, AppState>,
    gen: u64,
    mirror_load_launch: bool,
    outcome: btx_core::snapshot::SnapshotOutcome,
) -> Result<(), String> {
    use btx_core::snapshot::SnapshotOutcome as O;
    let held = matches!(outcome, O::HeldRootOnChain(_));
    if !mirror_load_launch && !held {
        return Ok(());
    }
    let datadir = node_datadir();
    let msg = mirror_load_end_message(&outcome);
    eprintln!("[node-app] {msg}");
    setup_log(&datadir, &msg);
    if held || !matches!(outcome, O::SignedLoaded { .. } | O::AlreadyLoaded) {
        SIGNED_LOAD_FAILED.store(true, Ordering::SeqCst);
    }
    if state.refresher_gen.load(Ordering::SeqCst) != gen
        || state.rpc.lock().await.is_none()
        || state.quitting.load(Ordering::SeqCst)
    {
        return Ok(());
    }
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    if held {
        set_aside_refused_snapshot(&datadir);
    }
    btx_core::node::end_mirror_load(&datadir);
    start_node_projected(app, state).await
}
````

- [ ] **Step 4: Decide the mirror launch before the conf is written**

In `start_node_inner`, replace the line:

````rust
    // ── The signer role (btx_core::signer) ──────────────────────────────────
````

with:

````rust
    // A validating node that has never loaded a snapshot loads a signed one
    // in one mirror launch (the confirmed-snapshot decision, section 7 step
    // 5). Decided here, before the signing key goes in or out of the conf,
    // because that launch is a mirror and a mirror holds no key.
    prepare_mirror_load(app, state, &paths.btxd, &datadir).await;

    // ── The signer role (btx_core::signer) ──────────────────────────────────
````

- [ ] **Step 5: Choose the load and act on its outcome**

In `start_node_inner`, replace (this is Task 5's interim version):

````rust
    if bootstrap_launch {
        eprintln!(
            "[snapshot] header bootstrap launch: the snapshot loads after the restart that ends it"
        );
    } else {
        // A node that follows signatures starts from the newest snapshot this
        // project's signer has signed, a few hundred blocks from the tip,
        // instead of the compiled one thousands below it
        // (btx_core::attested_snapshot; the owner's decision of 2026-09-26).
        // One that checks blocks itself cannot load a signed snapshot, the
        // engine refuses it, so it keeps the compiled one. Same rule as the
        // -matmulvalidation arm, through `signer_applies_here`.
        let signed = if signer_applies_here {
            btx_core::snapshot::SignedLoad::None
        } else {
            btx_core::snapshot::SignedLoad::Mirror
        };
        drop(btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            signed,
        ));
    }

    set_phase(app, state, NodePhase::LoadingSnapshot).await;
    spawn_status_refresher(app.clone(), state, bootstrap_launch);
````

with:

````rust
    let mut load_watch = None;
    if bootstrap_launch {
        eprintln!(
            "[snapshot] header bootstrap launch: the snapshot loads after the restart that ends it"
        );
    } else {
        // Where this node starts (the confirmed-snapshot decision, sections 7
        // and 9): a validating node's one mirror launch loads a confirmed or
        // the pinned pair and is then restarted as a validating node; a node
        // that follows signatures loads one in place, else the compiled one;
        // an ordinary validating launch loads the compiled one.
        let mut mirror_load_launch = btx_core::node::mirror_load_pending(&datadir).is_some();
        if mirror_load_launch && !attached_node_is_ours_to_stop(*state.attached_to.lock().await) {
            // Another app's node never read the marker, and is not ours to restart.
            btx_core::node::end_mirror_load(&datadir);
            mirror_load_launch = false;
        }
        let signed = signed_load_for(
            mirror_load_launch,
            btx_core::node::host_follows_signatures(&paths.btxd, &datadir, node_backend()),
            SIGNED_LOAD_FAILED.load(Ordering::SeqCst),
        );
        let handle = btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            signed,
        );
        load_watch = Some((handle, mirror_load_launch));
    }

    set_phase(app, state, NodePhase::LoadingSnapshot).await;
    spawn_status_refresher(app.clone(), state, bootstrap_launch);
    if let Some((handle, mirror_load_launch)) = load_watch {
        // This run's generation: a stop or restart moves it, and then the
        // outcome is no longer this run's to act on.
        let gen = state.refresher_gen.load(Ordering::SeqCst);
        spawn_load_watch(app.clone(), handle, gen, mirror_load_launch);
    }
````

`spawn_load_watch` is a plain function on purpose: spawning `after_snapshot_load` directly inside `start_node_inner` makes the compiler try to prove the start path's own future `Send` through itself, and it cannot (measured while writing this plan: "future cannot be sent between threads safely"). `spawn_status_refresher` is plain for the same reason.

- [ ] **Step 6: Run the tests to see them pass**

Run: `cd apps/node/src-tauri && cargo test --locked`
Expected: all pass, including `signed_start_tests` (2).

- [ ] **Step 7: Format, lint, commit**

````bash
cd apps/node/src-tauri
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../../..
````
````bash
git add apps/node/src-tauri/src/commands.rs
git commit -m "node: a validating node loads a signed snapshot in one mirror launch, then restarts validating" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 10: The rehearsal against a real engine (opt-in regtest test)

**Files:**
- Create: `crates/btx-core/tests/confirmed_snapshot_regtest.rs`

**Interfaces:**
- Consumes: everything above through the public API: `confirmed_snapshot::{parse, REGTEST_REPLAY_CONTEXT, MAINNET_REPLAY_CONTEXT}`, `confirmed_load::{node_view, load, CliRunner, Holds, LoadError, trimmed_manifest_path}`, `attested_snapshot::{ReadyPair, PairKind}`, `known_invalid::HeldBranch`, `node::{validating_snapshot_pin_args, BTX_TRUSTED_ATTESTATION_PUBKEYS}`, `operators::REGTEST_GENESIS`.
- Produces: `a_two_operator_snapshot_loads_on_a_mirror_and_a_validating_node_restarts_on_it` and `the_engine_reports_the_compiled_mainnet_replay_context`, both `#[ignore]`, run by Task 11's engine check.

What it proves, each step against btxd v0.34.9 (it passed 5 runs in a row while this plan was written, about 25 s each): a producer (validating, key P) exports at 100; a confirmer (key C) co-signs; the operator list comes from the regtest variable format (`producer=P;confirmer=C`); the node reports the compiled regtest genesis and replay context; a one-operator statement is refused before the engine sees it; the two-operator statement loads on a mirror that pins only P, through `confirmed_load::load`, and the trimmed manifest is the producer's file byte for byte; the node restarted validating WITHOUT the pins refuses to start ("failed verification under the current authority configuration"); WITH `validating_snapshot_pin_args` it starts in consensus mode on the snapshot; the mirror launch leaves no `matmulvalidation` in the node's settings file; a base above a held block is refused by the engine after the loader refuses the block. The second test starts btxd with mainnet parameters, no peers, and checks `replay_authority_context` equals `MAINNET_REPLAY_CONTEXT`.

Two engine facts this test works around, both measured while writing it:
- `submitheader` is refused on MatMul chains, so mirrors get headers (and blocks to 99) from the producer over P2P, and the producer serves no attestations (`-matmulattestationserve=0`) so a mirror cannot connect block 100 itself.
- Regtest block 100 (the MatMul v4 height) is mined but connects only when the producer restarts, so the test restarts it. It mines ten blocks per call (`mine_to`): on a busy machine one call for 100 blocks outlasted the RPC client's 60 s once.
- After the engine refused the held-branch load, lifting the hold with `reconsiderblock` on the same node and loading again was tried as a control: in 2 of 6 runs btxd stopped answering RPC. So the control is the mirror that loaded the same pair. See "Risks" at the end.

- [ ] **Step 1: Write the test**

Create `crates/btx-core/tests/confirmed_snapshot_regtest.rs`:

````rust
//! Confirmed snapshots against a real engine: a statement two operators
//! signed, loaded on a regtest mirror through the app's own loading path
//! (`confirmed_load::load`), then the node restarted as a validating node
//! on it under the pin rule (`node::validating_snapshot_pin_args`). Also the
//! engine-bump check of section 12: the replay contexts the app compiles are
//! the engine's. Opt-in, like the other shipped-engine tests:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test confirmed_snapshot_regtest -- --ignored --test-threads=1
//! ```
//!
//! btx-cli is taken from beside btxd, or from `EASYNODE_TEST_BTX_CLI`. The
//! keys are made fresh for each run and never written outside the scratch
//! folders. Regtest's ExactReplay starts at 101, so the chain stops at 100,
//! which is regtest's grid (`confirmed_snapshot::REGTEST_GRID`).

use btx_core::attested_snapshot::{PairKind, ReadyPair};
use btx_core::confirmed_load::{self, CliRunner, Holds, LoadError};
use btx_core::confirmed_snapshot as cs;
use btx_core::known_invalid::HeldBranch;
use btx_core::rpc::{Rpc, RpcClient};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A fresh key: (regtest WIF, compressed public key hex).
fn regtest_key() -> (String, String) {
    let sk = k256::SecretKey::random(&mut rand_core::OsRng);
    let mut payload = vec![0xef];
    payload.extend_from_slice(&sk.to_bytes());
    payload.push(0x01);
    let wif = bs58::encode(payload).with_check().into_string();
    let pubkey = sk.public_key().to_encoded_point(true);
    let hex: String = pubkey
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (wif, hex)
}

/// One regtest btxd in its own folder, killed and reaped on drop.
struct Node {
    btxd: PathBuf,
    dir: PathBuf,
    rpc_port: u16,
    p2p_port: u16,
    child: Option<std::process::Child>,
}

impl Node {
    fn new(btxd: &Path, dir: PathBuf, rpc_port: u16) -> Self {
        std::fs::create_dir_all(dir.join("regtest")).unwrap();
        Self {
            btxd: btxd.to_path_buf(),
            dir,
            rpc_port,
            p2p_port: rpc_port + 100,
            child: None,
        }
    }

    /// The network folder: where the engine keeps regtest's chain.
    fn net(&self) -> PathBuf {
        self.dir.join("regtest")
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.net().join("debug.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(40)..].join("\n")
    }

    /// Start with `extra`, and wait for RPC. `Err` carries the log's tail
    /// when the engine exits instead.
    async fn start(&mut self, extra: &[String]) -> Result<RpcClient, String> {
        let _ = std::fs::remove_file(self.net().join(".cookie"));
        let child = std::process::Command::new(&self.btxd)
            .arg("-regtest")
            .arg(format!("-datadir={}", self.dir.display()))
            .arg(format!("-rpcport={}", self.rpc_port))
            .arg(format!("-port={}", self.p2p_port))
            .args([
                "-server=1",
                "-bind=127.0.0.1",
                "-listen=1",
                "-discover=0",
                "-dnsseed=0",
                "-fixedseeds=0",
                "-upnp=0",
                "-natpmp=0",
                "-printtoconsole=0",
                "-daemon=0",
            ])
            .args(extra)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;
        self.child = Some(child);
        let url = format!("http://127.0.0.1:{}", self.rpc_port);
        for _ in 0..240 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child = None;
                return Err(format!("btxd exited with {status}:\n{}", self.log_tail()));
            }
            if let Ok(c) = RpcClient::from_cookie(url.clone(), &self.net().join(".cookie")) {
                if c.call("getblockcount", json!([])).await.is_ok() {
                    return Ok(c);
                }
            }
        }
        Err(format!("no RPC within 120 s:\n{}", self.log_tail()))
    }

    async fn stop(&mut self, rpc: &RpcClient) {
        let _ = rpc.call("stop", json!([])).await;
        if let Some(mut child) = self.child.take() {
            for _ in 0..120 {
                if child.try_wait().unwrap().is_some() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn signer_args(pubkey: &str) -> Vec<String> {
    vec![
        "-matmulvalidation=consensus".into(),
        "-matmulattestationsignerkeyfile=signer.wif".into(),
        format!("-matmultrustedpubkey={pubkey}"),
        "-matmultrustedthreshold=1".into(),
    ]
}

fn mirror_args(pubkey: &str) -> Vec<String> {
    vec![
        "-matmulvalidation=trusted".into(),
        "-connect=0".into(),
        format!("-matmultrustedpubkey={pubkey}"),
        "-matmultrustedthreshold=1".into(),
    ]
}

async fn call(rpc: &RpcClient, method: &str, params: Value) -> Value {
    rpc.call(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

/// Mine until the node has a header at `height`, ten blocks a call, so a
/// call that outlasts the RPC client's 60 s on a busy machine only costs a
/// retry (the node keeps mining what it was asked for).
async fn mine_to(rpc: &RpcClient, height: u64) {
    for _ in 0..120 {
        let headers = call(rpc, "getblockchaininfo", json!([])).await["headers"]
            .as_u64()
            .unwrap_or(0);
        if headers >= height {
            return;
        }
        let batch = (height - headers).min(10);
        let _ = rpc
            .call("generatetodescriptor", json!([batch, "raw(51)"]))
            .await;
    }
    panic!("the node did not reach {height}");
}

/// Blocks to 99 and the header at 100 from the producer, as the spike fed
/// its mirrors: the producer serves no attestations, so a mirror cannot
/// connect block 100 itself and the snapshot at 100 is ahead of it.
async fn feed_headers(mirror: &RpcClient, producer: &Node) {
    call(
        mirror,
        "addnode",
        json!([format!("127.0.0.1:{}", producer.p2p_port), "onetry"]),
    )
    .await;
    for _ in 0..60 {
        let info = call(mirror, "getblockchaininfo", json!([])).await;
        if info["headers"].as_u64() >= Some(100) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the mirror never heard the producer's headers");
}

fn snapshot_base(chainstates: &Value) -> Option<String> {
    chainstates["chainstates"]
        .as_array()?
        .iter()
        .find_map(|c| c["snapshot_blockhash"].as_str().map(str::to_string))
}

fn copy_pair(to: &Path, file: &Path, manifest: &[u8]) -> ReadyPair {
    std::fs::create_dir_all(to).unwrap();
    let f = to.join("utxo-100.dat");
    let m = to.join("snapshot-manifest-100.json");
    std::fs::copy(file, &f).unwrap();
    std::fs::write(&m, manifest).unwrap();
    ReadyPair {
        kind: PairKind::Confirmed,
        height: 100,
        file: f,
        manifest: m,
    }
}

fn cli_path(btxd: &Path) -> PathBuf {
    std::env::var_os("EASYNODE_TEST_BTX_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| btxd.with_file_name("btx-cli"))
}

fn runner(cli: &Path, node: &Node) -> CliRunner {
    CliRunner {
        btx_cli: cli.to_path_buf(),
        args: vec![
            "-regtest".into(),
            format!("-datadir={}", node.dir.display()),
            format!("-rpcport={}", node.rpc_port),
        ],
    }
}

#[tokio::test]
#[ignore]
async fn a_two_operator_snapshot_loads_on_a_mirror_and_a_validating_node_restarts_on_it() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let cli = cli_path(&btxd);
    let root = tempfile::tempdir().unwrap();
    let (p_wif, p_pub) = regtest_key();
    let (c_wif, c_pub) = regtest_key();
    let env = format!("producer={p_pub};confirmer={c_pub}");
    let pins = [p_pub.as_str()];

    // The producer checks blocks, signs with P, and serves no attestations.
    let mut a = Node::new(&btxd, root.path().join("a"), 29471);
    std::fs::write(a.net().join("signer.wif"), format!("{p_wif}\n")).unwrap();
    let mut a_args = signer_args(&p_pub);
    a_args.push("-matmulattestationserve=0".into());
    let ra = a.start(&a_args).await.unwrap();
    // Block 100 is regtest's MatMul v4 height: the node mines it but connects
    // it only at its next start (as in the spike), so mine, then restart.
    mine_to(&ra, 100).await;
    a.stop(&ra).await;
    let ra = a.start(&a_args).await.unwrap();
    assert_eq!(call(&ra, "getblockcount", json!([])).await, json!(100));
    let dump = call(
        &ra,
        "dumptxoutsetattested",
        json!(["snap.dat", "snap.manifest"]),
    )
    .await;
    assert_eq!(dump["base_height"], json!(100), "{dump}");
    let base = dump["base_hash"].as_str().unwrap().to_string();
    let produced = std::fs::read(a.net().join("snap.manifest")).unwrap();

    // The confirmer co-signs a copy with C.
    let mut c = Node::new(&btxd, root.path().join("c"), 29472);
    std::fs::write(c.net().join("signer.wif"), format!("{c_wif}\n")).unwrap();
    let rc = c.start(&signer_args(&c_pub)).await.unwrap();
    std::fs::write(c.net().join("work.manifest"), &produced).unwrap();
    call(&rc, "signutxosnapshotmanifest", json!(["work.manifest"])).await;
    let cosigned = std::fs::read(c.net().join("work.manifest")).unwrap();
    c.stop(&rc).await;
    let m = cs::parse(&cosigned).unwrap();
    assert_eq!(m.signatures.len(), 2);
    assert_eq!(cs::parse(&produced).unwrap().signatures.len(), 1);

    // A mirror pinning only P, fed the producer's headers.
    let mut mirror = Node::new(&btxd, root.path().join("m"), 29473);
    let rm = mirror.start(&mirror_args(&p_pub)).await.unwrap();
    feed_headers(&rm, &a).await;

    // Section 12: the engine's replay context is the one the app compiles.
    let view = confirmed_load::node_view(&rm, &pins, 0).await;
    assert_eq!(
        view.genesis.as_deref(),
        Some(btx_core::operators::REGTEST_GENESIS)
    );
    assert_eq!(
        view.replay_context.as_deref(),
        Some(cs::REGTEST_REPLAY_CONTEXT)
    );

    // One operator is refused before the engine sees anything.
    let one = copy_pair(
        &mirror.net().join("one"),
        &a.net().join("snap.dat"),
        &produced,
    );
    let err = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &one,
        &view,
        &Holds::none(),
        Some(&env),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, LoadError::NotConfirmed(_)), "{err}");
    assert_eq!(
        snapshot_base(&call(&rm, "getchainstates", json!([])).await),
        None
    );

    // Two operators load, trimmed to the one key the mirror pins.
    let pair = copy_pair(
        &mirror.net().join("pair"),
        &a.net().join("snap.dat"),
        &cosigned,
    );
    let loaded = confirmed_load::load(
        &rm,
        &runner(&cli, &mirror),
        &pair,
        &view,
        &Holds::none(),
        Some(&env),
    )
    .await
    .unwrap();
    assert_eq!(
        (loaded.height, loaded.signatures, loaded.superseded),
        (100, 1, false)
    );
    assert_eq!(
        std::fs::read(confirmed_load::trimmed_manifest_path(&pair)).unwrap(),
        produced,
        "trimming gave the producer's file back byte for byte"
    );
    assert_eq!(
        snapshot_base(&call(&rm, "getchainstates", json!([])).await),
        Some(base.clone())
    );
    mirror.stop(&rm).await;

    // Section 8: without the mirrors' pins a validating restart refuses to start ...
    let validating = args(&["-matmulvalidation=consensus", "-connect=0"]);
    let Err(err) = mirror.start(&validating).await else {
        panic!("a validating node on a signed snapshot started without the pins");
    };
    assert!(
        err.contains("failed verification under the current authority configuration"),
        "{err}"
    );
    // ... and with the pin rule's arguments it starts, validating, on the snapshot.
    let pin_args = btx_core::node::validating_snapshot_pin_args(&mirror.net(), &pins, &[]);
    assert_eq!(
        pin_args,
        vec![
            format!("-matmultrustedpubkey={p_pub}"),
            "-matmultrustedthreshold=1".into()
        ]
    );
    let mut with_pins = validating.clone();
    with_pins.extend(pin_args);
    let rv = mirror.start(&with_pins).await.unwrap();
    let status = call(&rv, "getmatmultrustedstatus", json!([])).await;
    assert_eq!(
        status["matmul_validation_mode"],
        json!("consensus"),
        "{status}"
    );
    assert_eq!(status["trusted_mirror"], json!(false), "{status}");
    assert_eq!(
        snapshot_base(&call(&rv, "getchainstates", json!([])).await),
        Some(base.clone())
    );
    mirror.stop(&rv).await;
    let rw = std::fs::read_to_string(mirror.net().join("btx_rw.conf")).unwrap_or_default();
    assert!(
        !rw.contains("matmulvalidation"),
        "the mirror launch's mode must not outlive it: {rw}"
    );

    // A base above a held block: the engine refuses it once the app has
    // refused the block (section 7, step 3).
    let mut held = Node::new(&btxd, root.path().join("h"), 29474);
    let rh = held.start(&mirror_args(&p_pub)).await.unwrap();
    feed_headers(&rh, &a).await;
    let root50: &'static str = Box::leak(
        call(&ra, "getblockhash", json!([50]))
            .await
            .as_str()
            .unwrap()
            .to_string()
            .into_boxed_str(),
    );
    let branch = [HeldBranch {
        height: 50,
        root: root50,
        why: "rehearsal",
    }];
    let view_h = confirmed_load::node_view(&rh, &pins, 0).await;
    let pair_h = copy_pair(
        &held.net().join("pair"),
        &a.net().join("snap.dat"),
        &cosigned,
    );
    let err = confirmed_load::load(
        &rh,
        &runner(&cli, &held),
        &pair_h,
        &view_h,
        &Holds {
            invalid: &[],
            held: &branch,
        },
        Some(&env),
    )
    .await
    .unwrap_err();
    // v0.34.9 answers "Attested snapshot base is incompatible with the
    // current best-header chain" here (measured 2026-09-29), and "part of an
    // invalid chain" for a base whose own header is marked failed.
    assert!(matches!(&err, LoadError::Engine(_)), "{err}");
    assert_eq!(
        snapshot_base(&call(&rh, "getchainstates", json!([])).await),
        None
    );
    // The hold was the reason: the mirror above loaded this same pair from
    // the same producer's headers. (Lifting the hold with reconsiderblock on
    // this node and loading again was tried: 2 runs of 6, v0.34.9 stopped
    // answering RPC after the refused load, so the control is the mirror.)
    held.stop(&rh).await;
    a.stop(&ra).await;
}

/// Section 12, mainnet: an engine bump that moves the replay context would
/// strand every node on a signed snapshot, so it fails here first. Mainnet
/// parameters, a scratch folder, no peers, the mirror arm's own flags.
#[tokio::test]
#[ignore]
async fn the_engine_reports_the_compiled_mainnet_replay_context() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(&btxd);
    cmd.arg(format!("-datadir={}", dir.path().display()))
        .args([
            "-listen=0",
            "-connect=0",
            "-dnsseed=0",
            "-rpcport=29479",
            "-server=1",
            "-prune=550",
            "-printtoconsole=0",
            "-daemon=0",
            "-matmulvalidation=trusted",
            "-matmultrustedthreshold=1",
            "-allowsinglekeytrustedmirror=1",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for k in btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS {
        cmd.arg(format!("-matmultrustedpubkey={k}"));
    }
    let mut child = cmd.spawn().unwrap();
    let cookie = dir.path().join(".cookie");
    let mut status = None;
    for _ in 0..360 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(code) = child.try_wait().unwrap() {
            panic!("btxd exited with {code}");
        }
        if let Ok(c) = RpcClient::from_cookie("http://127.0.0.1:29479", &cookie) {
            if let Ok(v) = c.call("getmatmultrustedstatus", json!([])).await {
                status = Some((c, v));
                break;
            }
        }
    }
    let (rpc, v) = status.expect("btxd answered within 180 s");
    let _ = rpc.call("stop", json!([])).await;
    let _ = child.wait();
    assert_eq!(
        v["replay_authority_context"].as_str(),
        Some(cs::MAINNET_REPLAY_CONTEXT),
        "{v}"
    );
}
````

- [ ] **Step 2: Run it without an engine (it must skip, not fail)**

Run: `cd crates/btx-core && cargo test --locked --test confirmed_snapshot_regtest -- --ignored`
Expected: `2 passed` with `EASYNODE_TEST_BTXD unset; nothing to test against` printed (use `--nocapture` to see it).

- [ ] **Step 3: Run it against the shipped engine**

The v0.34.9 btxd and btx-cli used for this plan are at `/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/spike/bin/`; otherwise use the staged package's (`apps/node/src-tauri/resources/node-pkg/.../btxd`, after `apps/node/scripts/stage-node-pkg.sh`). Ports 29471 to 29479 and 29571 to 29574 must be free.

Run: `cd crates/btx-core && EASYNODE_TEST_BTXD=/path/to/btxd cargo test --locked --test confirmed_snapshot_regtest -- --ignored --test-threads=1`
Expected: `test result: ok. 2 passed` in about 30 s. No btxd is left running afterwards (`pgrep -fl btxd` shows none started by the test).

- [ ] **Step 4: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/tests/confirmed_snapshot_regtest.rs
git commit -m "core: rehearse confirmed snapshots against the real engine on regtest (opt-in)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 11: The engine-bump check, the recipe and the changelog (section 12, second rule)

**Files:**
- Modify: `scripts/check-engine-tag.sh` (a `regtest_check` function, called in the OK branch)
- Modify: `docs/node-release-recipe.md` (one paragraph)
- Modify: `apps/node/CHANGELOG.md` (one entry under `[Unreleased]`)

**Interfaces:**
- Consumes: Task 10's test. Environment: `EASYNODE_TEST_BTXD` (path to the candidate btxd, btx-cli beside it), `ENGINE_TAG_GUARD_REGTEST=1` (makes the check required).
- Produces: `scripts/check-engine-tag.sh <tag>` runs the rehearsal when it can, says how when it cannot, and fails when told it must run and cannot.

- [ ] **Step 1: See the current script pass without the new check**

Run: `BTX_CLONE=/Users/m2promende/repos/btx bash scripts/check-engine-tag.sh`
Expected: exit 0, ending with `No mainnet stall-recovery height. This engine follows the majority chain.`

- [ ] **Step 2: Add the check**

In `scripts/check-engine-tag.sh`, directly before the line `# --- 1. which tag ----------------------------------------------------------`, add:

````bash
# --- 5, defined here: the confirmed-snapshot rehearsal on regtest -----------
# Section 12 of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
# before any engine bump: the replay contexts the engine reports are the ones
# the app compiles (confirmed_snapshot.rs), and a node on a signed snapshot
# still restarts as a validating node. A context that moves strands every
# node on a signed snapshot: the engine re-checks the stored manifest at each
# start and refuses to start when it no longer verifies.
#
# It needs the candidate engine's btxd, which CI does not have:
#   EASYNODE_TEST_BTXD=/path/to/btxd scripts/check-engine-tag.sh <tag>
# Without it the check is skipped with a note. ENGINE_TAG_GUARD_REGTEST=1
# makes it required, so the release recipe cannot skip it by accident.
regtest_check() {
  if [ -z "${EASYNODE_TEST_BTXD:-}" ]; then
    if [ -n "${ENGINE_TAG_GUARD_REGTEST:-}" ]; then
      die "ENGINE_TAG_GUARD_REGTEST is set but EASYNODE_TEST_BTXD is not" \
        "Point EASYNODE_TEST_BTXD at the candidate engine's btxd (btx-cli beside it)."
    fi
    echo "regtest check: skipped. Before an engine bump run it with"
    echo "  EASYNODE_TEST_BTXD=/path/to/btxd ENGINE_TAG_GUARD_REGTEST=1 $0 <tag>"
    return 0
  fi
  echo "regtest check: confirmed snapshots against $EASYNODE_TEST_BTXD"
  if ! (cd "$ROOT/crates/btx-core" \
        && cargo test --locked --test confirmed_snapshot_regtest -- --ignored --test-threads=1); then
    die "the confirmed-snapshot rehearsal failed against $EASYNODE_TEST_BTXD" \
      "Either the engine's replay context moved, which would stop every node on a" \
      "signed snapshot from starting, or a validating node on a signed snapshot no" \
      "longer restarts. Do not ship this engine until confirmed_snapshot.rs agrees."
  fi
  echo "OK: the engine's replay contexts are the compiled ones, and a validating"
  echo "    node restarts on a signed snapshot."
}
````

In the OK branch of step 4, after the line `  echo "    No mainnet stall-recovery height. This engine follows the majority chain."`, add the line:

````bash
  regtest_check
````

- [ ] **Step 3: Run it three ways**

````bash
bash -n scripts/check-engine-tag.sh && echo syntax-ok
BTX_CLONE=/Users/m2promende/repos/btx bash scripts/check-engine-tag.sh; echo "exit=$?"
BTX_CLONE=/Users/m2promende/repos/btx ENGINE_TAG_GUARD_REGTEST=1 bash scripts/check-engine-tag.sh; echo "exit=$?"
BTX_CLONE=/Users/m2promende/repos/btx EASYNODE_TEST_BTXD=/path/to/btxd ENGINE_TAG_GUARD_REGTEST=1 bash scripts/check-engine-tag.sh; echo "exit=$?"
````
Expected, in order: `syntax-ok`; `regtest check: skipped. Before an engine bump run it with` and `exit=0`; `FAIL: ENGINE_TAG_GUARD_REGTEST is set but EASYNODE_TEST_BTXD is not` and `exit=1`; `test result: ok. 2 passed`, `OK: the engine's replay contexts are the compiled ones, and a validating` and `exit=0`. CI (`engine-tag-guard.yml`) has no btxd and sees the "skipped" line, so it stays green.

- [ ] **Step 3b: Sabotage the check once**

Change one hex digit of `REGTEST_REPLAY_CONTEXT` in `crates/btx-core/src/confirmed_snapshot.rs`, run the third command again, expect `FAIL: the confirmed-snapshot rehearsal failed` and `exit=1`, and undo the change (`git diff crates/btx-core/src/confirmed_snapshot.rs` must be empty afterwards).

- [ ] **Step 4: The recipe**

In `docs/node-release-recipe.md`, after the paragraph ending "Run it rather than retyping the grep. It does **not** run the assumeutxo check below. Run that one by hand.", add:

````markdown
Before any engine bump, also run it with the candidate engine's btxd:
`EASYNODE_TEST_BTXD=/path/to/btxd ENGINE_TAG_GUARD_REGTEST=1 scripts/check-engine-tag.sh <tag>`.
That runs `crates/btx-core/tests/confirmed_snapshot_regtest.rs`: the replay
contexts the engine reports must be the ones `confirmed_snapshot.rs` compiles,
and a validating node on a signed snapshot must still restart. A context that
moves would stop every node on a signed snapshot from starting, because the
engine re-checks the stored manifest at every start
(docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 12).
````

- [ ] **Step 5: The changelog**

In `apps/node/CHANGELOG.md`, under `## [Unreleased]`, after the Tools entry, add:

````markdown
**New nodes start closer to the tip, from a snapshot two operators confirmed.**
When you set up a node, easyNode looks for the newest chain snapshot that two
different people running this network have both signed, checks every signature
itself, and starts your node there instead of thousands of blocks back. A node
that checks blocks itself loads it in one short extra start, then goes back to
checking every new block. If there is no such snapshot yet, your node starts
from the one easyNode already ships, as before. Today only one operator is on
the list, so for now every node takes that second path.
````

- [ ] **Step 6: Commit**

````bash
git add scripts/check-engine-tag.sh docs/node-release-recipe.md apps/node/CHANGELOG.md
git commit -m "engine check: rehearse confirmed snapshots on regtest before an engine bump; changelog" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

## Interfaces for the other plans

- **Diary, producers, confirmers (sections 3 to 5):** `operators::{mainnet, for_chain, regtest_env, OperatorList::operator_of}` tell a node whether its key is on the list; `confirmed_snapshot::{parse, Statement accessors, check_with, ChainRules, trim_to_pinned, Manifest::to_bytes}` read and compare a statement field by field against a diary entry (`block_hash`, `hash_serialized`, `coins`, `chain_tx`, `chain_id`, `replay_context`); `node::attested_snapshot_record(datadir).exists()` is the "runs on a signed snapshot whose background check has not finished" test after which the diary records nothing and a node confirms nothing; the grid is `confirmed_snapshot::MAINNET_GRID` (200).
- **Website (section 6):** the pointer contract is `attested_snapshot::ConfirmedPointer` (fields, statuses and hosts in Task 3), and `crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json` is its fixture. The verification the website mirrors in TypeScript is `confirmed_snapshot::{parse, is_strict_der, signature_is_valid, confirming_operators, check_with}`; the shared vectors are the fixture files of Task 2 (statement hash `d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482`, file hash `f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c`, regtest statement hash `11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194`). The website's copy of the operator list must equal `operators::MAINNET_OPERATORS`.
- **Catch-up help (section 11):** nothing here; it reads `getmatmulattestedtip` and the watchdog as before.
- **Fast-forward (plan 2):** `attested_snapshot::{prepare_confirmed, confirmed_url_allowed, CONFIRMED_POINTER_URL, http_client, prune_others, fallback_start}`, `confirmed_load::node_view`, `snapshot::{SignedLoad::SignedOnly, SnapshotOutcome}`, `node::{mirror_load_marker_exists, header_bootstrap_pending, end_mirror_load, end_header_bootstrap}`, and the app's `SIGNED_LOAD_FAILED`, `signed_load_for`, `spawn_load_watch`, `after_snapshot_load`, `set_phase` (plan 2 widens the last to `pub(crate)`).

## Risks and open points (for the owner)

1. **Nothing is confirmed until a second operator is on the list.** With Mende alone every new node takes the pinned pair (225,927), which today is about 7,500 blocks back; a fresh validating install now takes it too (the owner's choice 3), in one mirror launch. That is a real change for validating nodes before anyone is added: a new Mac or NVIDIA node that used to start at 219,000 starts at 225,927 on the 3060's single signature, with the pins the rule adds. Nodes that already hold a chain are not touched (only Fast-forward moves them).
2. **`shielded_state` is chain data** the design's Fast-forward list leaves out; plan 2 sets it aside too.
3. **A refused load can leave v0.34.9 unresponsive.** In the rehearsal, after the engine refused a base above a held block, a `reconsiderblock` on that node went unanswered in 2 of 6 runs. The loader never calls `reconsiderblock`, but a node whose load the engine refused may stop answering; the start path's restart (validating node) and the refresher's error phase (mirror) are what recover it. Worth an upstream note beside the others the design lists.
4. **The engine's refusal text differs from the design's.** A base above a held block is refused with "Attested snapshot base is incompatible with the current best-header chain", not "part of an invalid chain"; both are engine refusals, and the loader treats any refusal the same.
5. **Extra restarts.** A fresh validating install now goes bootstrap launch, mirror launch, validating launch: one more MatMul canary on a Mac (80 to 125 s). A failed signed load does not retry until the app restarts (`SIGNED_LOAD_FAILED`).
6. **Pins after the background check.** The design leaves open whether the pins may go once `attested_assumeutxo` disappears. This plan keys the pins on the file, so they go with it, which is what the design's measurement plan asks to confirm on a real data folder before 0.7.0 ships.
7. **Two measurements this plan did not repeat:** a validating node checking new blocks on a signed snapshot (needs a card the engine qualifies) and the mainnet validating restart with the pins (measured by the spike on this Mac on 2026-09-29, not repeated here).
8. **Threshold on the validating arm:** the rule passes `-matmultrustedthreshold=1` with the pins, as the measured mainnet run did (`phase2-args.txt`); the design names only the keys.

## Self-review

- Spec coverage: section 1 (Task 1: list per chain, Mende only, one-line addition guarded, env list for regtest only, proven never to reach mainnet); section 7 steps 1 and 2 (Tasks 2, 3), steps 3 to 6 (Task 4), step 5's mirror launch and step 7's restart (Tasks 7, 9); section 8 (Task 6, and Task 10 proves it against the engine); section 9 (Tasks 3, 5, 9: confirmed, pinned, compiled, for every node); section 12 first rule (Task 8) and second rule (Tasks 10, 11). Section 10 is plan 2. Sections 3 to 6 and 11 are other plans; their interfaces are listed above.
- Sabotage tests, one per rule: one operator only (`two_regtest_operators_confirm_and_one_does_not`, `each_rule_refuses_on_its_own`, `the_published_pair_is_one_operator_and_is_not_confirmed`, regtest rehearsal); two keys of one operator (`two_keys_of_one_operator_count_once`); an unknown key (`an_unknown_key_counts_for_nobody`, `an_invalid_signature_from_an_unlisted_key_still_refuses_the_whole_manifest`); high S (`a_high_s_signature_is_refused`); non-strict DER (`a_signature_that_is_not_strict_der_is_refused`); wrong chain id (`each_rule_refuses_on_its_own`, `the_running_node_must_agree_on_chain_and_replay_context`, `a_regtest_list_never_validates_a_mainnet_statement`); wrong replay context (same two, plus Task 11 step 3b); off-grid height (`each_rule_refuses_on_its_own`, `the_published_pair_is_one_operator_and_is_not_confirmed`); trailing bytes (`a_manifest_is_read_strictly`); a file whose hash does not match (`the_regtest_vectors_verify_and_the_file_matches`, `a_file_whose_hash_does_not_match_the_statement_is_refused_and_removed`, `a_pair_that_no_longer_checks_out_is_not_loaded`); a held-branch base (`a_hold_that_cannot_be_refused_stops_the_load`, `a_refused_block_on_the_chain_after_the_load_is_reported`, regtest rehearsal); a removed pinned key (`pins_only_grow_and_the_threshold_stays`).
- Placeholders: none; every code step carries the code, every run step the command and the expected result.
- Consistency: names checked across tasks (`SignedLoad::SignedOnly`, `ReadyPair`, `PairKind`, `NodeView`, `Holds`, `mirror_load_pending`, `validating_snapshot_pin_args`, `BTX_TRUSTED_ATTESTATION_THRESHOLD`). The whole plan was replayed on a clean worktree of `72304d9` while it was written: after Tasks 2, 3, 4, 5 and 11 every gate passed (btx-core 633 tests, app 90 tests, fmt, clippy correctness and suspicious), and `scripts/check-engine-tag.sh` with the rehearsal passed against v0.34.9.
