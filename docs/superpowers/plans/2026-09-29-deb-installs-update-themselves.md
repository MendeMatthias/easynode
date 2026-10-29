> **Release note (2026-09-29):** this plan was written for "0.6.33". The release is **0.7.0**: wherever this plan says 0.6.33 for the first release that ships `node-deb.json`, read 0.7.0 (for example `DEB_FEED_SINCE = (0, 7, 0)`). The website part (Tasks 13-14) is done on EasyBTX branch `claude/node-deb-feed-check` (PR MendeMatthias/EasyBTX#548). Work happens in the worktree `/Users/m2promende/repos/easynode/.claude/worktrees/easynode-0-7-0-release-65b687` on this branch, not in a separate `deb-self-update` worktree.

# A .deb Install Updates Itself: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A copy of easyNode installed from the `.deb` reads its own updater feed, `node-deb.json`, and installs the new `.deb` after one password prompt; where it cannot, it downloads nothing and shows the command to install by hand.

**Architecture:** The app asks `node-{{bundle_type}}.json` first and `latest-node.json` second (tauri.conf.json). The version comparator in `update_binding.rs` learns the `.deb` platform key, declines an AppImage-only release on a `.deb` copy, and declines (on automatic checks only) a version whose verified download already failed to install here; declines travel through the existing refusal slot and are recorded as a failed check whose detail is a notice. The release scripts sign the `.deb` and write `node-deb.json` beside `latest-node.json`; the website repository serves it and `scripts/check-node-links.py` enforces its rules, each with a sabotage case.

**Tech Stack:** Tauri 2.11.5 (Rust), tauri-plugin-updater 2.11.0, tauri-utils 2.9.3, semver, minisign-verify 0.2.5; TypeScript + vitest 3; Python 3 scripts; bash; Astro site on Vercel (website repo).

**Design:** `docs/decisions/2026-09-28-deb-installs-update-themselves.md` on branch `claude/deb-self-update` (approved by the owner 2026-09-28).

**How this plan was checked:** every code block below was applied, task by task, to a throwaway copy of `origin/claude/deb-self-update` and of the website's `origin/main` (not to either repository), and every "Expected" line below is what that dry run printed: `cargo test` (100 passed, 1 ignored at the end), `cargo fmt --check`, the enforced clippy lints, `vitest` (130 passed), `tsc --noEmit`, `vite build`, `gen-node-feed.py --self-test`, `test-publish-gate.sh`, a full `build-node-feed.sh --linux --deb` run with a throwaway signing key, and `check-node-links.py` with every sabotage case applied to a copied tree.

---

## Decisions the design left open

Each is the simplest option I could find that keeps every install updating. Change any of them before Task 1 if the owner disagrees.

1. **Where the failed-version rule lives: in the comparator, and "Check now" clears it first.** The plugin's comparator (`Fn(Version, RemoteRelease) -> bool`) cannot tell a press from an automatic check. So `decide_here` declines a remembered failed version on every check, and the "Check now" path calls a new command, `forget_failed_update`, before it checks. One pure rule, both automatic paths (the launch check in `main.ts` and the Rust timer) covered by it.
2. **What "its install has failed" means: a verified download that then failed to install.** Read literally, the design would also stop automatic retries after a download that merely broke off (network), on every platform. So both paths now call `download()` then `install()` apart (the plugin's `download_and_install` is exactly those two calls, updater.rs:761-769), and only an `install()` failure is remembered. A broken download or a bad signature is retried at the next check, as today.
3. **Storage:** `NodeAppSettings.update_install_failed: Option<String>`, one version (the latest failure). Never cleared except by "Check now"; a newer release is a different version and is offered as usual.
4. **How a decline is recorded: through the existing refusal slot, as `check-failed`, with the notice as the detail.** No sixth outcome word (the vocabulary is closed and pinned on both sides). Every notice contains the phrase `install it by hand` (`HAND_INSTALL_MARK`, pinned equal in Rust and TypeScript), and the pane finds a notice by it.
5. **Where the notice shows.** Without this, a `.deb` copy would show nothing on its own: the old signal was the failed-install banner, which the guard now prevents. So: the banner (`vX is out. Install it by hand, the steps are in Settings.`) and the sentence beside "Check now" (the full notice with the command), on the launch check (the front end reads the notice through a new `peek_update_refusal` command before `record_update_check` takes it) and on the timer's event; the "Last check" line reads `vX is out, install it by hand from easybtx.com/node`. A pressed check that was declined or refused no longer says "You're on the latest version".
6. **Notice wording** (exact; all fit the 240-character record with a trigger label, tested):
   - `.deb` copy offered only the AppImage: `v0.6.34 is out. This copy came from a .deb, so install it by hand: get the .deb from easybtx.com/node and run sudo apt install ./BTX-Node_0.6.34_amd64.deb`
   - failed before, `.deb` copy: `v0.6.34 failed to install here, so it is not downloaded again. Press Check now to try again, or install it by hand: get the .deb from easybtx.com/node and run sudo apt install ./BTX-Node_0.6.34_amd64.deb`
   - failed before, any other copy: `v0.6.34 failed to install here, so it is not downloaded again. Press Check now to try again, or install it by hand from easybtx.com/node`
7. **`gen-node-feed.py` CLI:** `--deb-sig <file>` writes `node-deb.json` into the directory of `--out`; `latest-node.json` is written only when a mac/linux/win signature is given; both feeds are built (and so checked) before either is written. `write_feed` refuses the `.deb` key in any file not named `node-deb.json`, and anything but exactly that key in `node-deb.json`.
8. **`build-node-feed.sh --deb` may be given with or without `--linux`.** No pairing rule: the site check already fails a Linux release that forgot the `.deb`, because `node-deb.json` must equal `REL_LINUX`.
9. **"The typed endpoint never answers 204" is enforced as "nothing but a static file answers under `/updater`":** no route under `site/src/pages/updater`, and no Vercel redirect or rewrite that can reach `/updater` on easybtx.com. Plus a live probe in `site-links.yml` that fails when any of the six other names answers 2xx or 3xx. See the concern at the end: a 200 page is as fatal as a 204.
10. **The site check does not require `node-deb.json` to exist.** Removing it is the design's rollback, and a required file would block that PR. It prints a NOTE when `REL_LINUX` is 0.6.33 or later and the file is missing.
11. **Sabotage cases live inside `check-node-links.py`** as a module-level self-check, the pattern that file already uses three times (`_self_check`, `_self_check_signed_name`, `_self_check_claims`), so CI runs them with no new harness. The rules are one pure function so each can be sabotaged in isolation.
12. **The site's `.deb` paragraph on `/node` changes in the 0.6.33 site PR, not on this branch** (it is true until 0.6.33 ships). Task 12 puts the replacement copy in the release recipe.

## Global Constraints

Copied from the design; every task includes these.

- The `.deb` feed is `https://easybtx.com/updater/node-deb.json`, the same schema as `latest-node.json`, with exactly one platform key, `linux-x86_64-deb`. Its url is `https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v<V>/BTX-Node_<V>_amd64.deb` and its signature is that file's `.sig`, signed under that name.
- `latest-node.json` keeps its three keys (`darwin-aarch64`, `linux-x86_64`, `windows-x86_64`) and NEVER gains `linux-x86_64-deb`: 0.6.32 refuses the whole release for any key it does not know ("it lists X, which no release of this app has", update_binding.rs:123).
- Updater endpoints, in this order: `https://easybtx.com/updater/node-{{bundle_type}}.json`, then `https://easybtx.com/updater/latest-node.json`.
- `PLATFORM_SUFFIXES` gains `("linux-x86_64-deb", "_amd64.deb")`; `gen-node-feed.py`'s `ASSET` gains `"linux-x86_64-deb": "BTX-Node_{v}_amd64.deb"`; the existing test `the_names_match_the_feed_generator` keeps them equal.
- On a `.deb` install (`bundle_type() == Some(BundleType::Deb)`) the comparator declines a release that lists `linux-x86_64` but not `linux-x86_64-deb`, records the manual command `sudo apt install ./BTX-Node_<V>_amd64.deb` from easybtx.com/node, and nothing is downloaded.
- An automatic check never downloads again a version whose install already failed on this machine; the failed version is kept in the node app settings; "Check now" still tries.
- The typed endpoint never answers 204; no `node-*.json` other than `node-deb.json` is ever published.
- `build-node-feed.sh --deb <BTX-Node_V_amd64.deb>` checks the name, signs, verifies against the app's key, and writes `node-deb.json` beside `latest-node.json`. `gen-node-feed.py --deb-sig` writes the separate file and refuses `linux-x86_64-deb` in `latest-node.json`; its self-test covers both. `publish-node-release.sh`: a `.deb` in the asset folder must carry a `.sig`. `docs/node-release-recipe.md`: the feed step deploys both files.
- Website: `site/public/updater/node-deb.json`; `scripts/check-node-links.py` checks one key, url under `node-v<its version>`, signed under its release name, version equals the Linux pin and the `.deb` download, `latest-node.json` never lists `linux-x86_64-deb`, no other `node-*.json`; each rule has a sabotage case.
- Leaves alone: `latest-node.json`'s shape; the binding rule (#148) that every entry is signed under its release name; the AppImage, Mac and Windows update paths.
- User-facing copy: friendly, simple, no hype, no guarantees, no em-dashes in any new string.
- `cargo fmt --check` is enforced in CI (style.yml:56-62), and clippy with `-D clippy::correctness -D clippy::suspicious` (style.yml:99-101).
- Commit subjects follow the repo's `node: ...` style and end with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not push or open a pull request unless the owner asks.

## File structure

| Repository | File | Change | Responsibility |
|---|---|---|---|
| easynode | `apps/node/src-tauri/src/update_binding.rs` | modify | platform table, `.deb` guard, failed-version memory, notices, `decide_here`, refusal slot peek, endpoint pin test |
| easynode | `apps/node/src-tauri/src/state.rs` | modify | `NodeAppSettings.update_install_failed` |
| easynode | `apps/node/src-tauri/src/update_timer.rs` | modify | download then install; remember a failed install |
| easynode | `apps/node/src-tauri/src/commands.rs` | modify | `remember_failed_update`, `forget_failed_update`, `peek_update_refusal` |
| easynode | `apps/node/src-tauri/src/lib.rs` | modify | register the three commands |
| easynode | `apps/node/src-tauri/tauri.conf.json` | modify | two endpoints |
| easynode | `apps/node/src/update-check.ts` | modify | `HAND_INSTALL_MARK`, `handInstallNotice`, `handInstallBanner`, `noUpdateMessage`, `plainOutcome` case |
| easynode | `apps/node/src/main.ts` | modify | forget before a press, peek on "no update", download then install, remember, `paintHandInstall` |
| easynode | `apps/node/src/update-check.test.ts` | modify | TS tests and source-order pins |
| easynode | `apps/node/scripts/gen-node-feed.py` | modify | `ASSET` row, `build_deb_feed`, `feed_problem`, `write_feed`, `--deb-sig`, self-test |
| easynode | `apps/node/scripts/build-node-feed.sh` | modify | `--deb` |
| easynode | `apps/node/scripts/publish-node-release.sh` | modify | `.deb` needs a `.sig` |
| easynode | `apps/node/scripts/test-publish-gate.sh` | modify | fixture for the unsigned `.deb` |
| easynode | `docs/node-release-recipe.md`, `README.md`, `apps/node/CHANGELOG.md` | modify | release steps, the Linux note, the Unreleased entry |
| EasyBTX | `scripts/check-node-links.py` | modify | `deb_feed_problems`, `_self_check_deb_feed`, wiring, skip the key in the FEED_PIN loop |
| EasyBTX | `.github/workflows/site-links.yml` | modify | live probe of the six other typed names |
| EasyBTX | `site/public/updater/node-deb.json` | created at release time (0.6.33 site PR), not on this branch | the `.deb` feed |

---

## Before you start (Part 1)

- [ ] **Step A: Worktree on the existing branch.** The local branch `claude/deb-self-update` exists at `ddc122a` (the design doc on top of `b008f98`, which is `origin/main`). If your session already runs in a worktree of that branch, use its path wherever this plan says `/Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update`.

```bash
git -C /Users/m2promende/repos/easynode fetch origin
git -C /Users/m2promende/repos/easynode worktree add /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update claude/deb-self-update
git -C /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update merge --ff-only origin/claude/deb-self-update
git -C /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update log --oneline -2
```

Expected: `ddc122a docs: a .deb install updates itself (design, approved 2026-09-28)` then `b008f98 changelog: 0.6.32 shipped on 28 September (#152)`. If `origin/main` has moved past `b008f98`, stop and rebase the branch first; every line number below was read at `b008f98`.

- [ ] **Step B: Satisfy the bundle-resource glob and install the front end.** `resources/node-pkg/` is gitignored (.gitignore:18), exactly as CI does it (ci.yml:132-136).

```bash
mkdir -p /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/src-tauri/resources/node-pkg
echo "Placeholder for tauri-build's resource glob. Not a node package." > /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER
npm --prefix /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node ci
```

Optional, saves a long first build: `cp -cR /Users/m2promende/repos/easynode/apps/node/src-tauri/target /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/src-tauri/target` (an APFS clone, no extra disk until it diverges).

- [ ] **Step C: Baseline, all green before any change.**

```bash
cargo test --manifest-path /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/src-tauri/Cargo.toml --lib
(cd /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node && npx vitest run)
python3 /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/scripts/gen-node-feed.py --self-test
bash /Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update/apps/node/scripts/test-publish-gate.sh
```

Expected: `test result: ok. 85 passed; 0 failed; 1 ignored`; `Tests  119 passed (119)`; `gen-node-feed self-test OK (app trusts key id 5D4392DA73BCC2A2)`; `publish gate: all fixtures behave`.

In the commands below, `$WT` is short for `/Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update`. A shell does not keep variables between tool calls, so either write the path out or start each command with `WT=/Users/m2promende/repos/easynode/.claude/worktrees/deb-self-update;`.

---

# Part 1: the app repository (`claude/deb-self-update`)

### Task 1: The binding knows the .deb

**Files:**
- Modify: `apps/node/src-tauri/src/update_binding.rs:72-79` (PLATFORM_SUFFIXES), tests module (insert before `the_names_match_the_feed_generator`, :482)
- Modify: `apps/node/scripts/gen-node-feed.py:60-74` (ASSET), `:366` (`missing`)

**Interfaces:**
- Produces: `pub const APPIMAGE_KEY: &str = "linux-x86_64";`, `pub const DEB_KEY: &str = "linux-x86_64-deb";`, `PLATFORM_SUFFIXES: [(&str, &str); 4]`; test helpers `sig_named(file: &str) -> String` and `deb_feed(version: &str, signed_as: &str) -> RemoteRelease` (tests module, used by Tasks 3). Python: `DEB_KEY`, `MAIN_KEYS` (used by Task 9).

- [ ] **Step 1: Write the failing tests.** In `update_binding.rs`, inside `mod tests`, insert immediately before the doc comment `/// \`gen-node-feed.py\` mints the names this module expects.`:

```rust
    /// A genuine release signature with its trusted comment rewritten to name
    /// `file`. Its cryptography no longer holds, which does not matter here:
    /// the binding reads the name before anything is downloaded, and the
    /// plugin verifies the bytes and the comment at download.
    fn sig_named(file: &str) -> String {
        let real = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(sig(&feed(), "linux-x86_64"))
                .unwrap(),
        )
        .unwrap();
        let mut lines: Vec<String> = real.lines().map(str::to_string).collect();
        lines[2] = format!("trusted comment: timestamp:1790340476\tfile:{file}");
        base64::engine::general_purpose::STANDARD.encode(lines.join("\n") + "\n")
    }

    /// A `node-deb.json` the way `gen-node-feed.py --deb-sig` writes it: one
    /// entry, `linux-x86_64-deb`, its build signed as `signed_as`.
    fn deb_feed(version: &str, signed_as: &str) -> RemoteRelease {
        release(serde_json::json!({
            "version": version,
            "notes": "n",
            "pub_date": "2026-09-29T00:00:00Z",
            "platforms": {
                "linux-x86_64-deb": {
                    "signature": sig_named(signed_as),
                    "url": format!(
                        "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v{version}/BTX-Node_{version}_amd64.deb"
                    ),
                }
            }
        }))
    }

    /// The .deb has a feed of its own, `node-deb.json`, and its one entry is
    /// bound like every other: signed under `BTX-Node_<version>_amd64.deb`.
    #[test]
    fn a_deb_feed_signed_under_its_name_is_offered() {
        assert_eq!(
            signed_name(&sig_named("BTX-Node_0.6.34_amd64.deb")).as_deref(),
            Some("BTX-Node_0.6.34_amd64.deb")
        );
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        assert_eq!(binding_refusal("0.6.34", &r.data), None);
        assert_eq!(decide(&ver("0.6.33"), None, &r), Decision::Offer);
    }

    /// The AppImage's signature under the .deb's key is another platform's
    /// build, and is refused like the Windows installer under Linux's.
    #[test]
    fn a_deb_feed_signed_under_the_appimage_name_is_refused() {
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.AppImage");
        assert_eq!(
            binding_refusal("0.6.34", &r.data).as_deref(),
            Some(
                "its linux-x86_64-deb build is signed as BTX-Node_0.6.34_amd64.AppImage, \
                 not BTX-Node_0.6.34_amd64.deb"
            )
        );
    }

```

- [ ] **Step 2: Run them, see them fail.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib a_deb_feed`
Expected: `FAILED. 0 passed; 2 failed`, both with `left: Some("it lists linux-x86_64-deb, which no release of this app has")`.

- [ ] **Step 3: Add the row, on both sides.** In `update_binding.rs`, replace:

```rust
/// Each platform key a feed may carry, and the end of the name its artifact is
/// signed under. `ASSET` in `apps/node/scripts/gen-node-feed.py` is the same
/// table, and a test below reads that file to keep the two equal.
pub const PLATFORM_SUFFIXES: [(&str, &str); 3] = [
    ("darwin-aarch64", "_aarch64.app.tar.gz"),
    ("linux-x86_64", "_amd64.AppImage"),
    ("windows-x86_64", "_x64-setup.exe"),
];
```

with:

```rust
/// The AppImage's platform key. Every Linux install read it until the .deb
/// had a feed of its own.
pub const APPIMAGE_KEY: &str = "linux-x86_64";

/// The .deb's platform key. It is listed in `node-deb.json` and NEVER in
/// `latest-node.json`: 0.6.32 refuses a whole release that lists a key it does
/// not know ("it lists X, which no release of this app has", below), so this
/// key there would stop every 0.6.32 install from updating, on every platform.
pub const DEB_KEY: &str = "linux-x86_64-deb";

/// Each platform key a feed may carry, and the end of the name its artifact is
/// signed under. `ASSET` in `apps/node/scripts/gen-node-feed.py` is the same
/// table, and a test below reads that file to keep the two equal.
pub const PLATFORM_SUFFIXES: [(&str, &str); 4] = [
    ("darwin-aarch64", "_aarch64.app.tar.gz"),
    (APPIMAGE_KEY, "_amd64.AppImage"),
    (DEB_KEY, "_amd64.deb"),
    ("windows-x86_64", "_x64-setup.exe"),
];
```

In `apps/node/scripts/gen-node-feed.py`, replace the block from `# Release asset names follow the node convention` through the closing `}` of `ASSET` (lines 60-74, the comment that says "there is no `.deb` key: the updater cannot install one") with:

```python
# Release asset names follow the node convention BTX-Node_<ver>_<arch>.<ext>.
# PLATFORM_SUFFIXES in apps/node/src-tauri/src/update_binding.rs is the same
# table, and a test there reads this file to keep the two equal.
#
# linux-x86_64 is the AppImage. linux-x86_64-deb is the .deb, and it lives in
# its OWN feed, node-deb.json, which only a .deb install reads (the app asks
# for node-{{bundle_type}}.json first; docs/decisions/2026-09-28-deb-installs-
# update-themselves.md). It must never appear in latest-node.json: easyNode
# 0.6.32 refuses a whole release that lists a key it does not know, so one
# stray key there stops every 0.6.32 install from updating, on every platform.
ASSET = {
    "darwin-aarch64": "BTX-Node_{v}_aarch64.app.tar.gz",
    "linux-x86_64": "BTX-Node_{v}_amd64.AppImage",
    "linux-x86_64-deb": "BTX-Node_{v}_amd64.deb",
    "windows-x86_64": "BTX-Node_{v}_x64-setup.exe",
}
DEB_KEY = "linux-x86_64-deb"
# The keys latest-node.json may carry: every one but the .deb's.
MAIN_KEYS = tuple(k for k in ASSET if k != DEB_KEY)
```

and in `main()` replace `    missing = sorted(set(ASSET) - set(feed["platforms"]))` with `    missing = sorted(set(MAIN_KEYS) - set(feed["platforms"]))` (otherwise every run prints "no key for linux-x86_64-deb").

- [ ] **Step 4: Run them, see them pass.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib update_binding` then `python3 $WT/apps/node/scripts/gen-node-feed.py --self-test`
Expected: `test result: ok. 18 passed; 0 failed` (includes `the_names_match_the_feed_generator`, which fails if only one side has the row); `gen-node-feed self-test OK (app trusts key id 5D4392DA73BCC2A2)`.

- [ ] **Step 5: Format.** Run: `cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check`. Expected: no output. (If it prints a diff, run it without `-- --check` and re-run the tests.)

- [ ] **Step 6: Commit.**

```bash
git -C $WT add apps/node/src-tauri/src/update_binding.rs apps/node/scripts/gen-node-feed.py
git -C $WT commit -F - <<'EOF'
node: the update binding knows the .deb's platform key

linux-x86_64-deb joins PLATFORM_SUFFIXES and gen-node-feed.py's ASSET, so a
node-deb.json entry is bound like every other: signed under
BTX-Node_<version>_amd64.deb. Tested with the .deb's name and refused under the
AppImage's.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 2: The failed version is remembered in the settings file

**Files:**
- Modify: `apps/node/src-tauri/src/state.rs:272-277` (field), `:342` (Default)
- Modify: `apps/node/src-tauri/src/update_binding.rs` (after `remember_running_version`, :224-228; tests)

**Interfaces:**
- Consumes: `NodeAppSettings::{load, update}` (state.rs:348, :471), `SETTINGS_FILE_NAME` (state.rs:25).
- Produces: `pub fn failed_install(datadir: &Path) -> Option<Version>`, `pub fn remember_failed_install(datadir: &Path, version: &str) -> Result<(), String>`, `pub fn forget_failed_install(datadir: &Path)`; field `NodeAppSettings.update_install_failed: Option<String>`.

- [ ] **Step 1: Write the failing tests.** In `update_binding.rs` `mod tests`, insert immediately before `    /// A genuine release signature with its trusted comment rewritten to name` (added in Task 1):

```rust
    #[test]
    fn a_failed_install_round_trips_through_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(failed_install(dir.path()), None);
        remember_failed_install(dir.path(), "0.6.34").unwrap();
        assert_eq!(failed_install(dir.path()), Some(ver("0.6.34")));
        // The latest failure replaces an earlier one.
        remember_failed_install(dir.path(), "0.6.35").unwrap();
        assert_eq!(failed_install(dir.path()), Some(ver("0.6.35")));
        forget_failed_install(dir.path());
        assert_eq!(failed_install(dir.path()), None);
    }

    /// The front end passes the version in, so anything else is refused
    /// before the settings file is touched.
    #[test]
    fn only_a_version_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        assert!(remember_failed_install(dir.path(), "not a version").is_err());
        assert!(remember_failed_install(dir.path(), "").is_err());
        assert_eq!(failed_install(dir.path()), None);
        assert!(!dir.path().join(crate::state::SETTINGS_FILE_NAME).exists());
    }

    /// "Check now" clears it before every press; with nothing to clear, the
    /// settings file is not rewritten.
    #[test]
    fn forgetting_nothing_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        forget_failed_install(dir.path());
        assert!(!dir.path().join(crate::state::SETTINGS_FILE_NAME).exists());
    }

    #[test]
    fn remembering_a_failure_leaves_the_other_settings_alone() {
        let dir = tempfile::tempdir().unwrap();
        NodeAppSettings::update(dir.path(), |s| {
            s.node_nickname = "alice".into();
            s.update_high_water = Some("0.6.33".into());
        });
        remember_failed_install(dir.path(), "0.6.34").unwrap();
        let s = NodeAppSettings::load(dir.path());
        assert_eq!(s.node_nickname, "alice");
        assert_eq!(s.update_high_water.as_deref(), Some("0.6.33"));
        assert_eq!(s.update_install_failed.as_deref(), Some("0.6.34"));
    }

```

- [ ] **Step 2: Run, see it fail to compile.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib update_binding`
Expected: `error[E0425]: cannot find function \`failed_install\` in this scope` (and the same for `remember_failed_install`, `forget_failed_install`).

- [ ] **Step 3: Add the field.** In `state.rs`, replace:

```rust
    #[serde(default)]
    pub update_high_water: Option<String>,
}
```

with:

```rust
    #[serde(default)]
    pub update_high_water: Option<String>,
    /// A version whose verified download then failed to install here: a .deb
    /// copy whose password prompt was cancelled or could not be shown, an
    /// install that `dpkg` or the AppImage swap refused. The automatic checks
    /// do not download it again (`update_binding::decide_here`), so a machine
    /// that cannot install it does not fetch it every six hours; "Check now"
    /// clears it first and tries. `None` until an install fails; a newer
    /// release is a different version and is offered as usual.
    #[serde(default)]
    pub update_install_failed: Option<String>,
}
```

and in `impl Default for NodeAppSettings` replace:

```rust
            update_high_water: None,
        }
```

with:

```rust
            update_high_water: None,
            update_install_failed: None,
        }
```

- [ ] **Step 4: Add the three functions.** In `update_binding.rs`, insert immediately before `#[cfg(test)]` (after `remember_running_version`):

```rust
/// The version whose verified download failed to install here, if any.
pub fn failed_install(datadir: &Path) -> Option<Version> {
    NodeAppSettings::load(datadir)
        .update_install_failed
        .and_then(|v| Version::parse(&v).ok())
}

/// Remember that `version` was downloaded and verified here and then failed to
/// install, so the automatic checks leave it alone. Anything that is not a
/// version is refused unwritten: the front end passes it in.
pub fn remember_failed_install(datadir: &Path, version: &str) -> Result<(), String> {
    let v = Version::parse(version).map_err(|e| format!("not a version: {version:?} ({e})"))?;
    NodeAppSettings::update(datadir, |s| s.update_install_failed = Some(v.to_string()));
    Ok(())
}

/// Forget it, before "Check now", so a press always tries. Writes the settings
/// file only when there is something to forget.
pub fn forget_failed_install(datadir: &Path) {
    if NodeAppSettings::load(datadir)
        .update_install_failed
        .is_some()
    {
        NodeAppSettings::update(datadir, |s| s.update_install_failed = None);
    }
}

```

- [ ] **Step 5: Run, see them pass.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib`
Expected: `test result: ok. 91 passed; 0 failed; 1 ignored`. (Until Task 5, `cargo check` warns that `remember_failed_install` and `forget_failed_install` are never used outside tests. That is expected and not fatal.)

- [ ] **Step 6: Format and commit.**

```bash
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
git -C $WT add apps/node/src-tauri/src/state.rs apps/node/src-tauri/src/update_binding.rs
git -C $WT commit -F - <<'EOF'
node: remember a version whose install failed here

update_install_failed in the node app settings holds the version whose
verified download then failed to install. The next task makes the automatic
checks leave it alone; "Check now" will clear it first. Only a version is
accepted, and forgetting nothing writes nothing.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 3: The comparator declines what this copy will not download on its own

**Files:**
- Modify: `apps/node/src-tauri/src/update_binding.rs` (module doc :52, after `expected_signed_name` :88, `Decision` :143-151, `comparator`/`judge`/`take_refusal` :173-200, test `a_refusal_is_kept_for_the_record_and_taken_once` :473-480, new tests)

**Interfaces:**
- Consumes: `DEB_KEY`, `APPIMAGE_KEY`, `expected_signed_name`, `failed_install`, `forget_failed_install`, `remember_failed_install` (Tasks 1-2); test helpers `deb_feed`, `bound_0630_as` (:279), `ver` (:273).
- Produces: `pub const DOWNLOADS_AT: &str = "easybtx.com/node"`, `pub const HAND_INSTALL_MARK: &str = "install it by hand"`, `pub fn deb_hand_install_notice(version: &str) -> String`, `pub fn failed_before_notice(version: &str, deb: bool) -> String`, `pub fn offers_only_the_appimage(data: &RemoteReleaseInner) -> bool`, `Decision::Decline(String)`, `pub struct ThisInstall { pub deb: bool, pub failed: Option<Version> }` (Default), `pub fn decide_here(current: &Version, high_water: Option<&Version>, here: &ThisInstall, release: &RemoteRelease) -> Decision`, `pub fn peek_refusal() -> Option<String>`; `judge` gains a `&ThisInstall` parameter.

- [ ] **Step 1: Write the failing tests.** In `mod tests`, replace the whole test:

```rust
    #[test]
    fn a_refusal_is_kept_for_the_record_and_taken_once() {
        let _ = take_refusal();
        assert!(!judge(&ver("0.6.32"), None, &bound_0630_as("0.9.0")));
        let reason = take_refusal().expect("the refusal is kept");
        assert!(reason.starts_with("refused v0.9.0: "), "{reason}");
        assert_eq!(take_refusal(), None, "taken once");
    }
```

with (the one test that touches the shared slot; a second one would race it, since tests run in parallel):

```rust
    #[test]
    fn a_refusal_is_kept_for_the_record_and_taken_once() {
        // The one test that touches the shared slot: tests run in parallel,
        // and a second one would take this one's record.
        let _ = take_refusal();
        let here = ThisInstall::default();
        assert!(!judge(&ver("0.6.32"), None, &here, &bound_0630_as("0.9.0")));
        let reason = take_refusal().expect("the refusal is kept");
        assert!(reason.starts_with("refused v0.9.0: "), "{reason}");
        assert_eq!(take_refusal(), None, "taken once");

        // A decline is kept as its notice, whole, and peeking leaves it for
        // the record to take.
        assert!(!judge(
            &ver("0.6.29"),
            None,
            &deb_copy(),
            &bound_0630_as("0.6.30")
        ));
        let notice = deb_hand_install_notice("0.6.30");
        assert_eq!(peek_refusal(), Some(notice.clone()));
        assert_eq!(
            peek_refusal(),
            Some(notice.clone()),
            "peeking takes nothing"
        );
        assert_eq!(take_refusal(), Some(notice));
        assert_eq!(peek_refusal(), None);
    }

    fn deb_copy() -> ThisInstall {
        ThisInstall {
            deb: true,
            failed: None,
        }
    }

    /// The AppImage-only feed a .deb copy read until now: it downloaded the
    /// ~467 MB AppImage, `install_deb` refused it, and it did so again at the
    /// next launch and every six hours. Now nothing is downloaded, and the
    /// record gives the command.
    #[test]
    fn a_deb_copy_declines_a_release_that_offers_only_the_appimage() {
        let r = bound_0630_as("0.6.30");
        assert!(offers_only_the_appimage(&r.data));
        match decide_here(&ver("0.6.29"), None, &deb_copy(), &r) {
            Decision::Decline(notice) => {
                assert_eq!(
                    notice,
                    "v0.6.30 is out. This copy came from a .deb, so install it by hand: \
                     get the .deb from easybtx.com/node and run \
                     sudo apt install ./BTX-Node_0.6.30_amd64.deb"
                );
            }
            other => panic!("expected a decline, got {other:?}"),
        }
    }

    #[test]
    fn an_appimage_copy_is_never_declined_by_the_deb_guard() {
        let r = bound_0630_as("0.6.30");
        assert_eq!(
            decide_here(&ver("0.6.29"), None, &ThisInstall::default(), &r),
            Decision::Offer
        );
    }

    #[test]
    fn a_deb_copy_takes_a_release_that_lists_its_deb() {
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        assert!(!offers_only_the_appimage(&r.data));
        assert_eq!(
            decide_here(&ver("0.6.33"), None, &deb_copy(), &r),
            Decision::Offer
        );
    }

    /// The guard comes after the rules that were there first: a tampered feed
    /// is still reported as tampered on a .deb copy, and nothing newer is
    /// still the ordinary "no update".
    #[test]
    fn the_guard_never_hides_a_refusal_or_a_current_version() {
        let tampered = bound_0630_as("0.9.0");
        assert!(matches!(
            decide_here(&ver("0.6.32"), None, &deb_copy(), &tampered),
            Decision::Refuse(_)
        ));
        let current = bound_0630_as("0.6.30");
        assert_eq!(
            decide_here(&ver("0.6.30"), None, &deb_copy(), &current),
            Decision::NotNewer
        );
    }

    #[test]
    fn an_automatic_check_does_not_download_a_version_that_failed_here_again() {
        let r = bound_0630_as("0.6.30");
        let failed = ThisInstall {
            deb: false,
            failed: Some(ver("0.6.30")),
        };
        match decide_here(&ver("0.6.29"), None, &failed, &r) {
            Decision::Decline(notice) => assert_eq!(
                notice,
                "v0.6.30 failed to install here, so it is not downloaded again. \
                 Press Check now to try again, or install it by hand from easybtx.com/node"
            ),
            other => panic!("expected a decline, got {other:?}"),
        }
        // A .deb copy is given the command.
        let failed_deb = ThisInstall {
            deb: true,
            failed: Some(ver("0.6.34")),
        };
        let deb = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        match decide_here(&ver("0.6.33"), None, &failed_deb, &deb) {
            Decision::Decline(notice) => assert!(
                notice.ends_with("sudo apt install ./BTX-Node_0.6.34_amd64.deb"),
                "{notice}"
            ),
            other => panic!("expected a decline, got {other:?}"),
        }
        // Any other version is downloaded as usual.
        let older_failure = ThisInstall {
            deb: false,
            failed: Some(ver("0.6.29")),
        };
        assert_eq!(
            decide_here(&ver("0.6.28"), None, &older_failure, &r),
            Decision::Offer
        );
    }

    /// "Check now" forgets the failed version before it checks
    /// (`forget_failed_update`), so a press always tries.
    #[test]
    fn check_now_clears_the_failed_version_so_it_tries_again() {
        let dir = tempfile::tempdir().unwrap();
        let r = bound_0630_as("0.6.30");
        remember_failed_install(dir.path(), "0.6.30").unwrap();
        let here = ThisInstall {
            deb: false,
            failed: failed_install(dir.path()),
        };
        assert!(matches!(
            decide_here(&ver("0.6.29"), None, &here, &r),
            Decision::Decline(_)
        ));
        forget_failed_install(dir.path());
        let here = ThisInstall {
            deb: false,
            failed: failed_install(dir.path()),
        };
        assert_eq!(
            decide_here(&ver("0.6.29"), None, &here, &r),
            Decision::Offer
        );
    }

    /// A notice is recorded as `<trigger>: <notice>` and a record holds
    /// `update_log::DETAIL_MAX_CHARS`, so the command must fit whole even for
    /// a long version. Each carries the phrase the pane finds it by, and none
    /// uses an em-dash.
    #[test]
    fn every_notice_fits_one_record_whole() {
        let long = "10.100.1000";
        for notice in [
            deb_hand_install_notice(long),
            failed_before_notice(long, true),
            failed_before_notice(long, false),
        ] {
            let line = format!("automatic: {notice}");
            assert!(
                line.chars().count() <= crate::update_log::DETAIL_MAX_CHARS,
                "{} chars: {line}",
                line.chars().count()
            );
            assert!(notice.starts_with("v10.100.1000 "), "{notice}");
            assert!(notice.contains(HAND_INSTALL_MARK), "{notice}");
            assert!(notice.contains(DOWNLOADS_AT), "{notice}");
            assert!(!notice.contains('\u{2014}'), "{notice}");
        }
    }
```

- [ ] **Step 2: Run, see it fail to compile.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib update_binding`
Expected: errors including `cannot find struct, variant or union type \`ThisInstall\``, `cannot find function \`decide_here\``, `cannot find function \`peek_refusal\``, and `this function takes 3 arguments but 4 arguments were supplied` (for `judge`).

- [ ] **Step 3: Module doc.** In the `//!` header, insert immediately before the line `//! # What a mistake here costs`:

```rust
//! # What this copy will not download on its own
//!
//! Two offers are declined before anything is downloaded, with a notice that
//! says what to do instead (docs/decisions/2026-09-28-deb-installs-update-
//! themselves.md). A copy installed from the .deb, offered a release that lists
//! the AppImage and no .deb: the plugin would fetch the ~467 MB AppImage and
//! `install_deb` would refuse it, at every launch and every six hours. And, on
//! an automatic check, a version whose verified download already failed to
//! install on this machine; "Check now" clears that memory first, so a press
//! always tries. Both are kept in the same slot as a refusal, and recorded as a
//! failed check whose detail is the notice.
//!
```

- [ ] **Step 4: The notices and the guard.** Insert immediately after the closing `}` of `pub fn expected_signed_name` (before `/// The file name a Tauri signature was made over`):

```rust
/// Where a person gets any build by hand.
pub const DOWNLOADS_AT: &str = "easybtx.com/node";

/// Every notice for an update this copy will not download on its own carries
/// this phrase, and the Settings pane finds the notice by it. `HAND_INSTALL_MARK`
/// in `src/update-check.ts` is the same text, and `update-check.test.ts` reads
/// this file to keep the two equal.
pub const HAND_INSTALL_MARK: &str = "install it by hand";

/// What a .deb copy does by hand for `version`: fetch the package, then the
/// command.
fn deb_steps(version: &str) -> String {
    let deb = expected_signed_name(DEB_KEY, version).unwrap_or_default();
    format!("get the .deb from {DOWNLOADS_AT} and run sudo apt install ./{deb}")
}

/// The notice for a .deb copy offered a release with no .deb in it.
pub fn deb_hand_install_notice(version: &str) -> String {
    format!(
        "v{version} is out. This copy came from a .deb, so {HAND_INSTALL_MARK}: {}",
        deb_steps(version)
    )
}

/// The notice for a version whose install already failed on this machine.
pub fn failed_before_notice(version: &str, deb: bool) -> String {
    let how = if deb {
        format!("{HAND_INSTALL_MARK}: {}", deb_steps(version))
    } else {
        format!("{HAND_INSTALL_MARK} from {DOWNLOADS_AT}")
    };
    format!(
        "v{version} failed to install here, so it is not downloaded again. \
         Press Check now to try again, or {how}"
    )
}

/// True when a release lists the AppImage and no .deb. A .deb copy handed such
/// a release would get the AppImage (the plugin falls back from
/// `linux-x86_64-deb` to `linux-x86_64`), and `install_deb` refuses it after
/// the whole download: `InvalidUpdaterFormat`.
pub fn offers_only_the_appimage(data: &RemoteReleaseInner) -> bool {
    match data {
        RemoteReleaseInner::Static { platforms } => {
            platforms.contains_key(APPIMAGE_KEY) && !platforms.contains_key(DEB_KEY)
        }
        RemoteReleaseInner::Dynamic(_) => false,
    }
}

```

- [ ] **Step 5: `Decision::Decline` and `ThisInstall`.** Replace:

```rust
    /// Newer, but not bound to its version. Refused, with the reason.
    Refuse(String),
}
```

with:

```rust
    /// Newer, but not bound to its version. Refused, with the reason.
    Refuse(String),
    /// Newer and bound, but not for this copy to download on its own: a .deb
    /// copy offered only the AppImage, or a version whose install already
    /// failed here. The notice says what to do instead.
    Decline(String),
}

/// What this install knows about itself, beyond its version, when it judges
/// an offer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThisInstall {
    /// Installed from the .deb: the bundler stamped `Deb` into this binary.
    pub deb: bool,
    /// A version whose verified download failed to install here.
    pub failed: Option<Version>,
}
```

- [ ] **Step 6: `decide_here`, the comparator, `judge`, `peek_refusal`.** Replace the block from `/// The last refusal, kept for whichever caller records the check's outcome.` through the end of `pub fn take_refusal` (update_binding.rs:173-200, shown here exactly):

```rust
/// The last refusal, kept for whichever caller records the check's outcome.
/// The comparator can only answer yes or no, and a no alone would be recorded
/// as "no update", which is exactly the silence this module must not have.
static REFUSAL: Mutex<Option<String>> = Mutex::new(None);

/// The plugin's version comparator (`lib.rs`). Both update paths use it.
pub fn comparator(current: Version, release: RemoteRelease) -> bool {
    judge(&current, high_water(&node_datadir()).as_ref(), &release)
}

/// [`decide`], with a refusal kept for the record.
fn judge(current: &Version, mark: Option<&Version>, release: &RemoteRelease) -> bool {
    match decide(current, mark, release) {
        Decision::Offer => true,
        Decision::NotNewer => false,
        Decision::Refuse(reason) => {
            let reason = format!("refused v{}: {reason}", release.version);
            eprintln!("[update] {reason}");
            *REFUSAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
            false
        }
    }
}

/// Take the refusal the last check left, if any.
pub fn take_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).take()
}
```

with:

```rust
/// [`decide`], then what this install knows about itself. Pure: `here.deb`
/// comes from the binary and `here.failed` from the settings file, both read
/// by [`comparator`], never here.
pub fn decide_here(
    current: &Version,
    high_water: Option<&Version>,
    here: &ThisInstall,
    release: &RemoteRelease,
) -> Decision {
    let decision = decide(current, high_water, release);
    if decision != Decision::Offer {
        return decision;
    }
    let version = release.version.to_string();
    if here.deb && offers_only_the_appimage(&release.data) {
        return Decision::Decline(deb_hand_install_notice(&version));
    }
    if here.failed.as_ref() == Some(&release.version) {
        return Decision::Decline(failed_before_notice(&version, here.deb));
    }
    Decision::Offer
}

/// The last refusal or notice, kept for whichever caller records the check's
/// outcome. The comparator can only answer yes or no, and a no alone would be
/// recorded as "no update", which is exactly the silence this module must not
/// have.
static REFUSAL: Mutex<Option<String>> = Mutex::new(None);

/// The plugin's version comparator (`lib.rs`). Both update paths use it.
pub fn comparator(current: Version, release: RemoteRelease) -> bool {
    let datadir = node_datadir();
    let here = ThisInstall {
        deb: is_deb_install(),
        failed: failed_install(&datadir),
    };
    judge(&current, high_water(&datadir).as_ref(), &here, &release)
}

/// This binary was packed into a .deb. The bundler stamps the package type
/// into the binary it packs (`usr/bin/easybtx-node` in the 0.6.32 .deb carries
/// `__TAURI_BUNDLE_TYPE_VAR_DEB`); a dev or test build carries none, and a Mac
/// always reads as `App`.
fn is_deb_install() -> bool {
    tauri::utils::platform::bundle_type() == Some(tauri::utils::config::BundleType::Deb)
}

/// [`decide_here`], with a refusal or a notice kept for the record.
fn judge(
    current: &Version,
    mark: Option<&Version>,
    here: &ThisInstall,
    release: &RemoteRelease,
) -> bool {
    let kept = match decide_here(current, mark, here, release) {
        Decision::Offer => return true,
        Decision::NotNewer => return false,
        Decision::Refuse(reason) => format!("refused v{}: {reason}", release.version),
        Decision::Decline(notice) => notice,
    };
    eprintln!("[update] {kept}");
    *REFUSAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(kept);
    false
}

/// Take the refusal or notice the last check left, if any.
pub fn take_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The refusal or notice the last check left, without taking it. The front
/// end reads it to say on screen what `record_update_check`, which takes it
/// right after, writes down.
pub fn peek_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
```

- [ ] **Step 7: Run, see them pass; format; the enforced lints.**

```bash
cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path $WT/apps/node/src-tauri/Cargo.toml --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
```

Expected: `test result: ok. 98 passed; 0 failed; 1 ignored`; fmt prints nothing; clippy ends without `error` (warnings such as "function `peek_refusal` is never used" disappear in Task 5).

- [ ] **Step 8: Commit.**

```bash
git -C $WT add apps/node/src-tauri/src/update_binding.rs
git -C $WT commit -F - <<'EOF'
node: a .deb copy declines an AppImage-only release, and an automatic check leaves a failed version alone

On a .deb install (bundle_type() == Deb) a release that lists linux-x86_64
and no linux-x86_64-deb is declined before anything is downloaded, with a
notice that gives the command: sudo apt install ./BTX-Node_<V>_amd64.deb
from easybtx.com/node. A version whose verified download already failed to
install here is declined too; "Check now" will clear that first. Declines
travel through the refusal slot, so both update paths record them as a
failed check whose detail is the notice. peek_refusal lets the front end
read it before record_update_check takes it.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 4: The six-hourly timer downloads, then installs, and remembers a failed install

**Files:**
- Modify: `apps/node/src-tauri/src/update_timer.rs:129-139` (doc of `check_once`), `:178-186` (download and install), tests module (before `a_missing_platform_is_named_not_quoted`, :324)

**Interfaces:**
- Consumes: `update_binding::remember_failed_install` (Task 2); plugin `Update::download<C: FnMut(usize, Option<u64>), D: FnOnce()>(&self, C, D) -> Result<Vec<u8>>` (updater.rs:680) and `Update::install(&self, bytes: impl AsRef<[u8]>) -> Result<()>` (updater.rs:751).
- Produces: nothing new for other tasks; the record strings are unchanged (`"automatic: v{version}: {}"`, pinned by update-check.test.ts:411).

- [ ] **Step 1: Write the failing test.** In `update_timer.rs` `mod tests`, insert immediately before `    #[test]\n    fn a_missing_platform_is_named_not_quoted() {`:

```rust
    /// A verified download that would not install is remembered, so the next
    /// automatic check leaves that version alone (`update_binding`); a
    /// download that broke off is not, and is retried at the next check as
    /// before. Telling the two apart needs the download and the install apart.
    #[test]
    fn only_an_install_that_failed_after_a_verified_download_is_remembered() {
        let src = include_str!("update_timer.rs");
        let start = src.find("async fn check_once(").unwrap();
        let body = &src[start..src.find("\nfn settle(").unwrap()];
        assert!(
            !body.contains("download_and_install("),
            "the two halves are apart"
        );
        let download = body.find("update.download(").expect("the download");
        let install = body.find("update.install(").expect("the install");
        let remember = body
            .find("update_binding::remember_failed_install(")
            .expect("the failure is remembered");
        assert!(download < install && install < remember);
        assert_eq!(body.matches("remember_failed_install(").count(), 1);
    }

```

- [ ] **Step 2: Run, see it fail.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib only_an_install`
Expected: `panicked ... the two halves are apart`, `FAILED. 0 passed; 1 failed`.

- [ ] **Step 3: Implement.** Replace:

```rust
    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        let detail = format!("automatic: v{version}: {}", error_text(&e));
        settle(app, &datadir, "install-failed", &version, &detail);
        return;
    }
```

with:

```rust
    // Downloaded and verified first, then installed, so a failure knows which
    // half it was. A download that broke off is retried at the next check, as
    // before. A verified download that would not install is remembered, and
    // the next automatic check leaves that version alone (update_binding), so
    // a .deb copy that cannot show the password prompt does not fetch the same
    // package every six hours.
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let detail = format!("automatic: v{version}: {}", error_text(&e));
            settle(app, &datadir, "install-failed", &version, &detail);
            return;
        }
    };
    if let Err(e) = update.install(bytes) {
        if let Err(why) = update_binding::remember_failed_install(&datadir, &version) {
            eprintln!("[update-timer] could not remember v{version} as failed: {why}");
        }
        let detail = format!("automatic: v{version}: {}", error_text(&e));
        settle(app, &datadir, "install-failed", &version, &detail);
        return;
    }
```

and replace the doc comment of `check_once` (the eleven `///` lines starting `/// One automatic check, start to finish,`) with:

```rust
/// One automatic check, start to finish, mirroring `updateCheck()` in
/// `main.ts` branch for branch: the builder with no options is exactly what
/// the JavaScript `check()` with no options asks the plugin for (its `check`
/// command calls `updater_builder()` and sets only what it was passed), so
/// the endpoints, the public key and the target come from `tauri.conf.json`
/// on both paths. The download and the install run apart, on both paths, so
/// a verified download that then fails to install can be remembered
/// (`update_binding::remember_failed_install`). On `found` the record is
/// written BEFORE the download, so a check that found something and died
/// mid-download still left the finding behind. On `installed` the restart is
/// requested the way the front end's `relaunch()` requests it (the process
/// plugin's `restart` command is `app.request_restart()`), which `lib.rs`
/// recognises by `RESTART_EXIT_CODE` and lets through without stopping btxd.
```

- [ ] **Step 4: Run, see it pass (and the vocabulary pins still hold).**

```bash
cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib
(cd $WT/apps/node && npx vitest run src/update-check.test.ts)
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
```

Expected: `test result: ok. 99 passed; 0 failed; 1 ignored` (includes `the_timer_uses_the_whole_vocabulary_and_nothing_else`); `Tests  29 passed (29)` (includes "speaks the same five words, always as automatic, on the Rust side"); fmt silent.

- [ ] **Step 5: Commit.**

```bash
git -C $WT add apps/node/src-tauri/src/update_timer.rs
git -C $WT commit -F - <<'EOF'
node: the six-hourly check remembers an install that failed after a verified download

download_and_install is download then install; the timer now runs them apart.
A download that broke off is retried at the next check, as before. A verified
download that would not install (a .deb copy with no one to answer the
password prompt) is remembered, and the next automatic check does not fetch
it again.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 5: Three small commands for the front end

**Files:**
- Modify: `apps/node/src-tauri/src/commands.rs` (insert before `/// The record for a "no update" that was really a refusal`, :3836)
- Modify: `apps/node/src-tauri/src/lib.rs:68` (handler list)

**Interfaces:**
- Consumes: `update_binding::{remember_failed_install, forget_failed_install, peek_refusal}`, `node_datadir` (already imported in commands.rs).
- Produces (Tauri commands, invoked from `main.ts` in Task 7): `remember_failed_update(version: String) -> Result<(), String>` (JS `invoke("remember_failed_update", { version })`), `forget_failed_update() -> Result<(), String>`, `peek_update_refusal() -> Result<Option<String>, String>` (JS `invoke<string | null>("peek_update_refusal")`).

- [ ] **Step 1: Add the commands.** In `commands.rs`, insert immediately before `/// The record for a "no update" that was really a refusal, or \`None\` to keep`:

```rust
/// Remember a version whose verified download then failed to install, so the
/// automatic checks do not download it again (`update_binding::decide_here`).
/// The front end calls this only after `download()` resolved and `install()`
/// failed; the six-hourly timer calls `update_binding::remember_failed_install`
/// itself. Anything that is not a version is refused unwritten.
#[tauri::command]
pub async fn remember_failed_update(version: String) -> Result<(), String> {
    crate::update_binding::remember_failed_install(&node_datadir(), &version)
}

/// Forget that version before a "Check now", so a press always tries.
#[tauri::command]
pub async fn forget_failed_update() -> Result<(), String> {
    crate::update_binding::forget_failed_install(&node_datadir());
    Ok(())
}

/// Why the last check declined or refused what it was offered, without taking
/// it: the front end shows it, and `record_update_check`, called right after,
/// takes it for the record.
#[tauri::command]
pub async fn peek_update_refusal() -> Result<Option<String>, String> {
    Ok(crate::update_binding::peek_refusal())
}

```

- [ ] **Step 2: Register them.** In `lib.rs`, replace `            commands::record_update_check,` with:

```rust
            commands::record_update_check,
            commands::remember_failed_update,
            commands::forget_failed_update,
            commands::peek_update_refusal,
```

- [ ] **Step 3: Build and test.** The commands are thin wrappers over functions tested in Tasks 2-3; the front-end test in Task 7 pins that each name `main.ts` invokes is registered here.

```bash
cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib
cargo check --manifest-path $WT/apps/node/src-tauri/Cargo.toml --locked 2>&1 | grep -E "^(warning|error)"
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
```

Expected: `test result: ok. 99 passed; 0 failed; 1 ignored`; the only warning is the pre-existing `constant \`NODE_RELEASE_COMMIT\` is never used` (no "never used" for the new functions any more); fmt silent.

- [ ] **Step 4: Commit.**

```bash
git -C $WT add apps/node/src-tauri/src/commands.rs apps/node/src-tauri/src/lib.rs
git -C $WT commit -F - <<'EOF'
node: commands to remember and forget a failed update, and to read a decline

remember_failed_update (after a verified download failed to install),
forget_failed_update (before Check now) and peek_update_refusal (read what
update_binding kept, before record_update_check takes it).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 6: The pane's words for an update to install by hand

**Files:**
- Modify: `apps/node/src/update-check.ts` (insert after :232 `const VERSION_IN_DETAIL`; `plainOutcome` `check-failed` case :242-245)
- Test: `apps/node/src/update-check.test.ts` (imports :21-32; append at end of file)

**Interfaces:**
- Consumes: `HAND_INSTALL_MARK` in update_binding.rs (Task 3), read by the test.
- Produces: `export const HAND_INSTALL_MARK = "install it by hand"`, `export function handInstallNotice(text: string): string | null`, `export function handInstallBanner(notice: string): { head: string; tail: string }`, `export function noUpdateMessage(refusal: string | null, currentVersion: string, downloadsAt: string): string`.

- [ ] **Step 1: Write the failing tests.** In `update-check.test.ts`, replace the import block:

```ts
import {
  classifyCheckFailure,
  checkFailureMessage,
  describeWhen,
  installErrorFromDetail,
  lastCheckLine,
  plainOutcome,
  updateCheckRecord,
  UPDATE_CHECK_OUTCOMES,
  type UpdateCheckBranch,
  type UpdateCheckOutcome,
} from "./update-check";
```

with:

```ts
import {
  classifyCheckFailure,
  checkFailureMessage,
  describeWhen,
  handInstallBanner,
  handInstallNotice,
  HAND_INSTALL_MARK,
  installErrorFromDetail,
  lastCheckLine,
  noUpdateMessage,
  plainOutcome,
  updateCheckRecord,
  UPDATE_CHECK_OUTCOMES,
  type UpdateCheckBranch,
  type UpdateCheckOutcome,
} from "./update-check";
```

and append at the end of the file:

```ts

// ── An update this copy will not download on its own ─────────────────────────
// update_binding.rs declines two offers before anything is downloaded: a .deb
// copy offered only the AppImage, and, on an automatic check, a version whose
// install already failed here. Its notices are the strings below, verbatim
// (update_binding.rs pins the same text in its own tests).

const DEB_NOTICE =
  "v0.6.34 is out. This copy came from a .deb, so install it by hand: " +
  "get the .deb from easybtx.com/node and run sudo apt install ./BTX-Node_0.6.34_amd64.deb";
const FAILED_NOTICE =
  "v0.6.34 failed to install here, so it is not downloaded again. " +
  "Press Check now to try again, or install it by hand from easybtx.com/node";
const REFUSED = "refused v0.9.0: its linux-x86_64 build is signed as X, not Y";

describe("an update this copy will not download on its own", () => {
  it("is found by the same phrase in TypeScript and in Rust", () => {
    const src = read("../src-tauri/src/update_binding.rs");
    const m = /HAND_INSTALL_MARK: &str = "([^"]+)"/.exec(src);
    expect(m, "HAND_INSTALL_MARK in update_binding.rs").not.toBeNull();
    expect(HAND_INSTALL_MARK).toBe(m![1]);
  });

  it("reads a notice from a refusal or from a recorded detail", () => {
    expect(handInstallNotice(DEB_NOTICE)).toBe(DEB_NOTICE);
    expect(handInstallNotice(`automatic: ${DEB_NOTICE}`)).toBe(DEB_NOTICE);
    expect(handInstallNotice(`manual: ${FAILED_NOTICE}`)).toBe(FAILED_NOTICE);
    // A refused feed and an ordinary detail are not notices.
    expect(handInstallNotice(REFUSED)).toBeNull();
    expect(handInstallNotice(`automatic: ${REFUSED}`)).toBeNull();
    expect(handInstallNotice("automatic: v0.6.33 is current")).toBeNull();
    expect(handInstallNotice("")).toBeNull();
  });

  it("puts the version in the banner and sends the reader to Settings", () => {
    expect(handInstallBanner(DEB_NOTICE)).toEqual({
      head: "v0.6.34 is out.",
      tail: "Install it by hand, the steps are in Settings.",
    });
    expect(handInstallBanner("install it by hand").head).toBe("An update is out.");
  });

  it("never says 'latest version' after a pressed check that was declined or refused", () => {
    expect(noUpdateMessage(null, "0.6.33", "easybtx.com/node")).toBe(
      "You're on the latest version (v0.6.33).",
    );
    expect(noUpdateMessage(null, "", "easybtx.com/node")).toBe("You're on the latest version.");
    // A notice is written for people already, and it carries the command.
    expect(noUpdateMessage(DEB_NOTICE, "0.6.33", "easybtx.com/node")).toBe(DEB_NOTICE);
    const refused = noUpdateMessage(REFUSED, "0.6.33", "easybtx.com/node");
    expect(refused).not.toMatch(/latest version/);
    expect(refused).toContain(REFUSED);
    expect(refused).toContain("easybtx.com/node");
    expect(noUpdateMessage("x".repeat(500), "0.6.33", "e.com").length).toBeLessThan(260);
  });

  it("shows the Last check line in plain words, with the version", () => {
    const now = new Date(2026, 8, 29, 16, 30);
    const at = new Date(2026, 8, 29, 14, 3).toISOString();
    for (const notice of [DEB_NOTICE, FAILED_NOTICE]) {
      expect(
        lastCheckLine(
          { at, outcome: "check-failed", detail: `automatic: ${notice}` },
          now,
          "easybtx.com/node",
        ),
      ).toBe("Last check: today 14:03 — v0.6.34 is out, install it by hand from easybtx.com/node");
    }
    // A refusal still reads as a failed check.
    expect(plainOutcome("check-failed", `automatic: ${REFUSED}`, "e")).toBe("couldn't check");
  });

  it("adds no em-dash of its own", () => {
    for (const text of [
      handInstallBanner(DEB_NOTICE).head,
      handInstallBanner(DEB_NOTICE).tail,
      noUpdateMessage(REFUSED, "0.6.33", "easybtx.com/node"),
      plainOutcome("check-failed", DEB_NOTICE, "easybtx.com/node"),
    ]) {
      expect(text).not.toContain("—");
    }
  });
});
```

(The `—` in the Last check expectation is the separator `lastCheckLine` already uses, update-check.ts:265; the new strings themselves have none.)

- [ ] **Step 2: Run, see them fail.**

Run: `(cd $WT/apps/node && npx vitest run src/update-check.test.ts)`
Expected: `Tests  6 failed | 29 passed (35)` (the new functions are not exported yet).

- [ ] **Step 3: Implement.** In `update-check.ts`, insert immediately after the line `const VERSION_IN_DETAIL = /\bv\d+\.\d+\.\d+\b/;`:

```ts

// ── An update this copy will not download on its own ────────────────────────
//
// update_binding.rs declines two offers before anything is downloaded: a copy
// installed from the .deb, offered a release with only the AppImage in it,
// and, on an automatic check, a version whose install already failed on this
// machine. To the plugin both are "no update". The notice update_binding
// keeps says what to do instead, and every such notice carries the phrase
// below; the record is a failed check with the notice as its detail.

/** The phrase every such notice carries. `HAND_INSTALL_MARK` in
 *  src-tauri/src/update_binding.rs is the same text, and
 *  update-check.test.ts reads that file to keep the two equal. */
export const HAND_INSTALL_MARK = "install it by hand";

const TRIGGER = /^(?:manual|automatic): /;

/** The notice in `text`, which is either what update_binding kept or a
 *  recorded detail (the same notice behind its trigger), or null. */
export function handInstallNotice(text: string): string | null {
  const notice = text.replace(TRIGGER, "");
  return notice.includes(HAND_INSTALL_MARK) ? notice : null;
}

/** The banner for such a notice. The steps themselves are in Settings, in
 *  the sentence beside "Check now". */
export function handInstallBanner(notice: string): { head: string; tail: string } {
  const v = VERSION_IN_DETAIL.exec(notice)?.[0];
  return {
    head: v ? `${v} is out.` : "An update is out.",
    tail: "Install it by hand, the steps are in Settings.",
  };
}

/**
 * The sentence beside "Check now" when a pressed check ended with nothing to
 * install. `refusal` is what update_binding kept, if anything: to the plugin
 * a declined or refused offer is "no update", and saying "latest version"
 * then would be untrue.
 */
export function noUpdateMessage(
  refusal: string | null,
  currentVersion: string,
  downloadsAt: string,
): string {
  if (!refusal) {
    return currentVersion
      ? `You're on the latest version (v${currentVersion}).`
      : "You're on the latest version.";
  }
  const notice = handInstallNotice(refusal);
  if (notice) return notice;
  return (
    `This copy did not take the update it was offered (${refusal.slice(0, 160)}). ` +
    `Downloads are at ${downloadsAt}.`
  );
}
```

and in `plainOutcome` replace:

```ts
    case "check-failed":
      return /no build for this platform/.test(detail)
        ? "no build for this platform yet"
        : "couldn't check";
```

with:

```ts
    case "check-failed":
      if (handInstallNotice(detail)) {
        return `${v ?? "an update"} is out, install it by hand from ${downloadsAt}`;
      }
      return /no build for this platform/.test(detail)
        ? "no build for this platform yet"
        : "couldn't check";
```

- [ ] **Step 4: Run, see them pass; typecheck.**

```bash
(cd $WT/apps/node && npx vitest run)
(cd $WT/apps/node && npx tsc --noEmit)
```

Expected: `Test Files  7 passed (7)`, `Tests  125 passed (125)`; tsc prints nothing.

- [ ] **Step 5: Commit.**

```bash
git -C $WT add apps/node/src/update-check.ts apps/node/src/update-check.test.ts
git -C $WT commit -F - <<'EOF'
node: plain words for an update to install by hand

handInstallNotice finds a notice from update_binding by the phrase both
sides pin ("install it by hand"); handInstallBanner and noUpdateMessage say
it, and the Last check line reads "vX is out, install it by hand from
easybtx.com/node". A pressed check that was declined or refused no longer
says "You're on the latest version".

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 7: `main.ts`: Check now forgets, a decline is shown, a failed install is remembered

**Files:**
- Modify: `apps/node/src/main.ts` (imports :22-30; comment :2120-2128; `onUpdateCheckEvent` :2190-2198; before `updateCheck` :2215; `!update` branch :2236-2244; download/install :2251-2257)
- Test: `apps/node/src/update-check.test.ts` (append)

**Interfaces:**
- Consumes: commands from Task 5; `handInstallBanner`, `handInstallNotice`, `noUpdateMessage` from Task 6; existing `showUpdateBanner` (:2107), `setUpdateResult` (:2116), `MANUAL_DOWNLOAD` (:2129), `recordUpdateCheck` (:2207); plugin JS `Update.download()` / `Update.install()` (`@tauri-apps/plugin-updater` 2.11.0, both allowed by `updater:default`).
- Produces: `paintHandInstall(text: string): void`, `peekUpdateRefusal(): Promise<string | null>` (main.ts internal).

- [ ] **Step 1: Write the failing tests.** Append to `update-check.test.ts`:

```ts

// ── main.ts: the failed version, the declined offer, and "Check now" ─────────
// Read from main.ts, the way the tests above read it: updateCheck() is a thin
// Tauri veneer that cannot run under vitest, but its order can be pinned.

describe("updateCheck() keeps a failed install from downloading again, and a press still tries", () => {
  const main = read("./main.ts");
  const lib = read("../src-tauri/src/lib.rs");
  const fn = (name: string) => {
    const s = main.indexOf(`function ${name}(`);
    expect(s, `function ${name} in main.ts`).toBeGreaterThan(-1);
    return main.slice(s, main.indexOf("\n}\n", s));
  };
  const body = fn("updateCheck");

  it("clears the failed version before a pressed check, and only then", () => {
    const forget = body.indexOf('invoke("forget_failed_update")');
    expect(forget).toBeGreaterThan(-1);
    expect(forget).toBeLessThan(body.indexOf("await checkForUpdate()"));
    expect(body.slice(0, forget)).toMatch(/if \(manual\) \{\s*await $/);
  });

  it("downloads, then installs, and remembers a version only when the install failed", () => {
    expect(body).not.toContain("downloadAndInstall(");
    const download = body.indexOf("await update.download()");
    const downloaded = body.indexOf("downloaded = true;");
    const install = body.indexOf("await update.install()");
    const remember = body.indexOf('invoke("remember_failed_update"');
    expect(download).toBeGreaterThan(-1);
    expect(download < downloaded && downloaded < install && install < remember).toBe(true);
    expect(body.slice(install, remember)).toMatch(/if \(downloaded\) \{\s*void $/);
  });

  it("reads why an offer was declined before the record takes it, and shows it", () => {
    const peek = body.indexOf("await peekUpdateRefusal()");
    expect(peek).toBeGreaterThan(-1);
    expect(peek).toBeLessThan(body.indexOf('branch: "no-update"'));
    expect(body).toContain("paintHandInstall(declined");
    expect(body).toContain("noUpdateMessage(declined, appVersion, MANUAL_DOWNLOAD)");
  });

  it("shows a notice from the six-hourly timer the same way", () => {
    expect(fn("onUpdateCheckEvent")).toContain("paintHandInstall(ev.detail)");
    const paint = fn("paintHandInstall");
    expect(paint).toContain("handInstallNotice(");
    expect(paint).toContain("showUpdateBanner(");
    expect(paint).toContain("setUpdateResult(");
  });

  it("calls only commands the backend registers", () => {
    for (const name of [
      "record_update_check",
      "remember_failed_update",
      "forget_failed_update",
      "peek_update_refusal",
    ]) {
      expect(main, name).toMatch(new RegExp(`invoke(<[^>]*>)?\\("${name}"`));
      expect(lib, name).toContain(`commands::${name},`);
    }
  });
});
```

- [ ] **Step 2: Run, see them fail.**

Run: `(cd $WT/apps/node && npx vitest run src/update-check.test.ts)`
Expected: `Tests  5 failed | 35 passed (40)`.

- [ ] **Step 3: Imports.** In `main.ts`, replace:

```ts
import {
  classifyCheckFailure,
  checkFailureMessage,
  installErrorFromDetail,
  lastCheckLine,
  updateCheckRecord,
```

with:

```ts
import {
  classifyCheckFailure,
  checkFailureMessage,
  handInstallBanner,
  handInstallNotice,
  installErrorFromDetail,
  lastCheckLine,
  noUpdateMessage,
  updateCheckRecord,
```

- [ ] **Step 4: The stale comment.** Replace the four lines:

```ts
// Failing to CHECK and failing to INSTALL are different events and must not
// share a catch. A failed check is usually just being offline, and there is
// nothing for the user to do about it. A failed INSTALL is permanent for that
// build — a Linux .deb cannot be replaced by the updater at all — and the old
```

with:

```ts
// Failing to CHECK and failing to INSTALL are different events and must not
// share a catch. A failed check is usually just being offline, and there is
// nothing for the user to do about it. A failed INSTALL is usually permanent
// for that build on that machine (a .deb copy that cannot show the password
// prompt, a folder the app cannot write to), and the old
```

- [ ] **Step 5: `paintHandInstall` and the timer's event.** Replace:

```ts
// A check the Rust timer ran has settled (src-tauri/src/update_timer.rs). The
// backend has already written the record; this paints what updateCheck()
// would have painted had the check run here, through the same two functions,
// and the "Last check" line from the record itself rather than waiting for
// the next status tick to read it back.
function onUpdateCheckEvent(ev: UpdateCheckEvent): void {
  paintUpdateProgress(ev.outcome, ev.version, installErrorFromDetail(ev.detail));
  paintLastUpdateCheck({ at: ev.at, outcome: ev.outcome, detail: ev.detail });
}
```

with:

```ts
/**
 * An update this copy will not download on its own (update_binding.rs): a
 * .deb copy offered only the AppImage, or a version whose install already
 * failed here. The banner says so and the sentence beside "Check now" gives
 * the steps. `text` is what update_binding kept or a recorded detail; for
 * anything else this paints nothing.
 */
function paintHandInstall(text: string): void {
  const notice = handInstallNotice(text);
  if (!notice) return;
  const banner = handInstallBanner(notice);
  showUpdateBanner(banner.head, banner.tail);
  setUpdateResult(notice);
}

// A check the Rust timer ran has settled (src-tauri/src/update_timer.rs). The
// backend has already written the record; this paints what updateCheck()
// would have painted had the check run here, through the same functions,
// and the "Last check" line from the record itself rather than waiting for
// the next status tick to read it back.
function onUpdateCheckEvent(ev: UpdateCheckEvent): void {
  paintUpdateProgress(ev.outcome, ev.version, installErrorFromDetail(ev.detail));
  paintHandInstall(ev.detail);
  paintLastUpdateCheck({ at: ev.at, outcome: ev.outcome, detail: ev.detail });
}
```

- [ ] **Step 6: Forget before a press, and the peek helper.** Replace:

```ts
async function updateCheck(manual = false): Promise<void> {
  let update: Awaited<ReturnType<typeof checkForUpdate>>;
```

with:

```ts
// What update_binding kept about the check that just ran: why it declined or
// refused the offer, or null. Read before recordUpdateCheck, which takes it.
// A failure to read it is a null, never a reason for the check to fail.
function peekUpdateRefusal(): Promise<string | null> {
  return invoke<string | null>("peek_update_refusal").catch(() => null);
}

async function updateCheck(manual = false): Promise<void> {
  // "Check now" always tries. The automatic checks leave alone a version whose
  // install already failed here (update_binding.rs); a press forgets that
  // first. Awaited, so the check below sees it cleared; a failure to clear is
  // a console warning and the check goes ahead.
  if (manual) {
    await invoke("forget_failed_update").catch((e) =>
      console.warn("update-check: could not clear the failed version", e),
    );
  }
  let update: Awaited<ReturnType<typeof checkForUpdate>>;
```

- [ ] **Step 7: Say what "no update" really was.** Replace:

```ts
  if (!update) {
    if (manual) {
      setUpdateResult(
        appVersion ? `You're on the latest version (v${appVersion}).` : "You're on the latest version."
      );
    }
    void recordUpdateCheck({ branch: "no-update", currentVersion: appVersion }, manual);
    return;
  }
```

with:

```ts
  if (!update) {
    // To the plugin a declined or refused offer is "no update". Read why
    // before the record below takes it, so the screen says what the record
    // will: the steps for an update to install by hand, or the refusal.
    const declined = await peekUpdateRefusal();
    paintHandInstall(declined ?? "");
    if (manual) {
      setUpdateResult(noUpdateMessage(declined, appVersion, MANUAL_DOWNLOAD));
    }
    void recordUpdateCheck({ branch: "no-update", currentVersion: appVersion }, manual);
    return;
  }
```

- [ ] **Step 8: Download, then install, and remember.** Replace:

```ts
  try {
    await update.downloadAndInstall();
  } catch (e) {
    paintUpdateProgress("install-failed", update.version, String(e));
```

with:

```ts
  // Downloaded and verified first, then installed, so a failure knows which
  // half it was. A download that broke off is retried at the next check, as
  // before. A verified download that would not install is remembered, and the
  // automatic checks leave that version alone, so a copy that cannot install
  // it does not fetch it again every six hours. "Check now" still tries.
  let downloaded = false;
  try {
    await update.download();
    downloaded = true;
    await update.install();
  } catch (e) {
    if (downloaded) {
      void invoke("remember_failed_update", { version: update.version }).catch((err) =>
        console.warn("update-check: could not remember the failed version", err),
      );
    }
    paintUpdateProgress("install-failed", update.version, String(e));
```

(The two lines after it, `void recordUpdateCheck({ branch: "install-failed", ... }, manual);` and `return;`, stay as they are: the existing test "records right before every return" pins that order.)

- [ ] **Step 9: Run everything on the front end.**

```bash
(cd $WT/apps/node && npx vitest run)
(cd $WT/apps/node && npx tsc --noEmit)
(cd $WT/apps/node && npx vite build)
```

Expected: `Test Files  7 passed (7)`, `Tests  130 passed (130)` (the older pins "records right before every return", "names every branch of the union", "waits, with a bound, for the installed record" and "paints the event through the functions updateCheck() paints with" still pass); tsc silent; vite ends `✓ built in`.

- [ ] **Step 10: Commit.**

```bash
git -C $WT add apps/node/src/main.ts apps/node/src/update-check.test.ts
git -C $WT commit -F - <<'EOF'
node: Check now always tries, a decline is shown, a failed install is remembered

updateCheck() forgets the failed version before a pressed check, reads what
update_binding kept when the plugin says "no update" and shows it (a banner,
and the steps beside Check now), and runs download() then install() so that
only a verified download that failed to install is remembered. The timer's
event paints a notice the same way. Source-order pins in update-check.test.ts,
and every command main.ts invokes is checked against lib.rs's handler list.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 8: The app asks for its install type's feed first

**Files:**
- Modify: `apps/node/src-tauri/tauri.conf.json:45-47`
- Test: `apps/node/src-tauri/src/update_binding.rs` tests (insert before `the_names_match_the_feed_generator`)

**Interfaces:**
- Consumes: plugin substitution of `{{bundle_type}}` and of its escaped form `%7B%7Bbundle_type%7D%7D` (updater.rs:459-487), non-success status moves on (:529-558), 204 returns "no update" (:531-534).
- Produces: the endpoint list in the Global Constraints.

This task comes after the guard on purpose: from here on a `.deb` build reads `node-deb.json` when it exists, and falls back to `latest-node.json`, which the guard now declines without downloading.

- [ ] **Step 1: Write the failing test.** In `update_binding.rs` `mod tests`, insert immediately before `    /// \`gen-node-feed.py\` mints the names this module expects.`:

```rust
    /// The typed feed first, then the one every install has always read. The
    /// plugin puts the install type where `{{bundle_type}}` is (`deb`,
    /// `appimage`, `app`, `nsis`, `msi`, `rpm` or `unknown`); only
    /// `node-deb.json` exists, every other name answers 404, and on a
    /// non-success status the plugin tries the next endpoint
    /// (tauri-plugin-updater 2.11.0, `Updater::check`). The endpoints are
    /// compiled in, so a change here strands the installs already out there,
    /// and the Linux and Windows overrides must not carry endpoints of their
    /// own (they once did, and those installs never updated).
    #[test]
    fn the_app_reads_its_install_types_feed_then_the_common_one() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            conf["plugins"]["updater"]["endpoints"],
            serde_json::json!([
                "https://easybtx.com/updater/node-{{bundle_type}}.json",
                "https://easybtx.com/updater/latest-node.json"
            ])
        );
        // The config parses each endpoint as a URL, which escapes the braces;
        // the plugin substitutes the escaped form (`Updater::check`), so the
        // .deb copy asks for exactly node-deb.json.
        let typed: tauri::Url = "https://easybtx.com/updater/node-{{bundle_type}}.json"
            .parse()
            .unwrap();
        assert_eq!(
            typed.to_string().replace("%7B%7Bbundle_type%7D%7D", "deb"),
            "https://easybtx.com/updater/node-deb.json"
        );
        for overlay in [
            include_str!("../tauri.linux.conf.json"),
            include_str!("../tauri.windows.conf.json"),
        ] {
            let v: serde_json::Value = serde_json::from_str(overlay).unwrap();
            assert!(v.get("plugins").is_none(), "{overlay}");
        }
    }

```

- [ ] **Step 2: Run, see it fail.**

Run: `cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib the_app_reads`
Expected: `left: Array [String("https://easybtx.com/updater/latest-node.json")]` against the two-entry right side; `FAILED. 0 passed; 1 failed`.

- [ ] **Step 3: Implement.** In `tauri.conf.json`, replace:

```json
      "endpoints": [
        "https://easybtx.com/updater/latest-node.json"
      ]
```

with:

```json
      "endpoints": [
        "https://easybtx.com/updater/node-{{bundle_type}}.json",
        "https://easybtx.com/updater/latest-node.json"
      ]
```

- [ ] **Step 4: Run, see it pass.**

```bash
cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --lib
python3 $WT/apps/node/scripts/gen-node-feed.py --self-test
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
```

Expected: `test result: ok. 100 passed; 0 failed; 1 ignored`; `gen-node-feed self-test OK (app trusts key id 5D4392DA73BCC2A2)` (it reads the pubkey from the same file, so this proves the edit left the JSON valid); fmt silent.

- [ ] **Step 5: Commit.**

```bash
git -C $WT add apps/node/src-tauri/tauri.conf.json apps/node/src-tauri/src/update_binding.rs
git -C $WT commit -F - <<'EOF'
node: ask for node-{{bundle_type}}.json first, then latest-node.json

A .deb copy reads node-deb.json when the site serves one. Every other install
type gets a 404 for its name and the plugin moves on to latest-node.json,
exactly as today, for one extra request per check. Pinned by a test, with the
escaped form the plugin substitutes, and the Linux and Windows overrides must
carry no endpoints of their own.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 9: `gen-node-feed.py --deb-sig` writes `node-deb.json`, and never the key into `latest-node.json`

**Files:**
- Modify: `apps/node/scripts/gen-node-feed.py` (docstring usage :45-49, constants after `MAIN_KEYS`, `build_feed` :183-188, new functions before `write_atomic` :211, `self_test` before the "REAL embedded pubkey" block :325, `main` :337-375)

**Interfaces:**
- Consumes: `ASSET`, `DEB_KEY`, `MAIN_KEYS` (Task 1); existing `_check_sig(target, sig, want_key_id, want_name)`, `pubkey_key_id`, `write_atomic`, `REPO`, `_named`, `_fake_pubkey`.
- Produces: `MAIN_FEED_NAME = "latest-node.json"`, `DEB_FEED_NAME = "node-deb.json"`, `build_deb_feed(version, tag, notes, pub_date, deb_sig, pubkey=None) -> dict`, `feed_problem(path, feed) -> str | None`, `write_feed(path, feed)`; CLI flag `--deb-sig` (used by Task 10).

- [ ] **Step 1: Write the failing self-test cases.** In `self_test()`, insert immediately before `    # The REAL embedded pubkey must parse, so a conf change cannot silently`:

```python
    # The .deb has a feed of its own, node-deb.json, with exactly one key.
    deb = build_deb_feed("0.6.33", "node-v0.6.33", "n", "d",
                         _named(DEB_KEY, "0.6.33"), pubkey=pub)
    assert set(deb["platforms"]) == {DEB_KEY}, deb
    assert deb["platforms"][DEB_KEY]["url"] == (
        f"{REPO}/node-v0.6.33/BTX-Node_0.6.33_amd64.deb"), deb
    assert deb["version"] == "0.6.33", deb
    # Its build is bound like every other: the AppImage's signature under the
    # .deb's key, an older .deb, or the wrong key are all refused.
    for bad_sig in (_named("linux-x86_64", "0.6.33"), _named(DEB_KEY, "0.6.32"),
                    _named(DEB_KEY, "0.6.33", b"\x09" * 8)):
        try:
            build_deb_feed("0.6.33", "node-v0.6.33", "", "", bad_sig, pubkey=pub)
            raise AssertionError("accepted an unbound .deb signature")
        except ValueError:
            pass

    # And the .deb key goes into node-deb.json and nowhere else: easyNode
    # 0.6.32 refuses a whole release that lists a key it does not know, so the
    # key in latest-node.json would stop every 0.6.32 install from updating.
    main = build_feed("0.6.33", "node-v0.6.33", "n", "d",
                      linux_sig=_named("linux-x86_64", "0.6.33"), pubkey=pub)
    poisoned = json.loads(json.dumps(main))
    poisoned["platforms"][DEB_KEY] = deb["platforms"][DEB_KEY]
    with tempfile.TemporaryDirectory() as d:
        for name, feed in ((MAIN_FEED_NAME, poisoned), ("feed.json", poisoned),
                           (DEB_FEED_NAME, main), (DEB_FEED_NAME, poisoned)):
            path = os.path.join(d, name)
            try:
                write_feed(path, feed)
                raise AssertionError(f"wrote {sorted(feed['platforms'])} to {name}")
            except ValueError:
                pass
            assert not os.path.exists(path), f"{name} was written anyway"
        write_feed(os.path.join(d, MAIN_FEED_NAME), main)
        write_feed(os.path.join(d, DEB_FEED_NAME), deb)
        with open(os.path.join(d, DEB_FEED_NAME)) as f:
            assert json.load(f) == deb

```

- [ ] **Step 2: Run, see it fail.**

Run: `python3 $WT/apps/node/scripts/gen-node-feed.py --self-test`
Expected: `NameError: name 'build_deb_feed' is not defined`.

- [ ] **Step 3: Constants and the shared version check.** After the line `MAIN_KEYS = tuple(k for k in ASSET if k != DEB_KEY)` add:

```python
MAIN_FEED_NAME = "latest-node.json"
DEB_FEED_NAME = "node-deb.json"
```

Replace:

```python
def build_feed(version, tag, notes, pub_date, mac_sig=None, linux_sig=None,
               win_sig=None, pubkey=None):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"version must be X.Y.Z, got {version!r}")
    if version not in tag:
        raise ValueError(f"tag {tag!r} does not contain version {version!r}")
    want = pubkey_key_id(pubkey) if pubkey else None
```

with:

```python
def _check_version_tag(version, tag):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"version must be X.Y.Z, got {version!r}")
    if version not in tag:
        raise ValueError(f"tag {tag!r} does not contain version {version!r}")


def build_feed(version, tag, notes, pub_date, mac_sig=None, linux_sig=None,
               win_sig=None, pubkey=None):
    _check_version_tag(version, tag)
    want = pubkey_key_id(pubkey) if pubkey else None
```

- [ ] **Step 4: The `.deb` feed and the guarded writer.** Insert immediately before `def write_atomic(path, data):`:

```python
def build_deb_feed(version, tag, notes, pub_date, deb_sig, pubkey=None):
    """node-deb.json: the feed only a .deb install reads, with exactly one key.

    Its entry is checked like every other: the app's key, and signed under the
    release name BTX-Node_<version>_amd64.deb, or easyNode refuses it."""
    _check_version_tag(version, tag)
    want = pubkey_key_id(pubkey) if pubkey else None
    name = ASSET[DEB_KEY].format(v=version)
    _check_sig(DEB_KEY, deb_sig, want, name)
    return {"version": version, "notes": notes, "pub_date": pub_date,
            "platforms": {DEB_KEY: {"signature": deb_sig.strip(),
                                    "url": f"{REPO}/{tag}/{name}"}}}


def feed_problem(path, feed):
    """Why `feed` must not be written to `path`, or None.

    node-deb.json lists exactly the .deb's key. Every other feed never lists
    it: easyNode 0.6.32 refuses a whole release that lists a key it does not
    know, so the key in latest-node.json would stop every 0.6.32 install from
    updating, on every platform."""
    keys = sorted(feed.get("platforms") or {})
    name = os.path.basename(path)
    if name == DEB_FEED_NAME:
        if keys != [DEB_KEY]:
            return f"{DEB_FEED_NAME} must list exactly {DEB_KEY}, not {keys}"
    elif DEB_KEY in keys:
        return (f"{DEB_KEY} must never be written to {name}: easyNode 0.6.32 "
                f"refuses a whole release that lists a key it does not know, so "
                f"every 0.6.32 install would stop updating. It belongs in "
                f"{DEB_FEED_NAME} only.")
    return None


def write_feed(path, feed):
    """write_atomic, after feed_problem has found nothing wrong."""
    problem = feed_problem(path, feed)
    if problem:
        raise ValueError(problem)
    write_atomic(path, feed)


```

- [ ] **Step 5: The CLI.** In the module docstring, replace:

```
  # All three:
  gen-node-feed.py --version 0.5.1 --tag node-v0.5.1 \
     --mac-sig <file> --linux-sig <file> --win-sig <file> ... --out latest-node.json

  gen-node-feed.py --self-test
```

with:

```
  # All three:
  gen-node-feed.py --version 0.5.1 --tag node-v0.5.1 \
     --mac-sig <file> --linux-sig <file> --win-sig <file> ... --out latest-node.json

  # A Linux release also passes the .deb's signature. It goes into its own
  # feed, node-deb.json, written beside --out, and never into latest-node.json:
  gen-node-feed.py --version 0.6.33 --tag node-v0.6.33 \
     --linux-sig <file> --deb-sig <file> ... --out latest-node.json

  gen-node-feed.py --self-test
```

In `main()`, replace `    p.add_argument("--win-sig")` with:

```python
    p.add_argument("--win-sig")
    p.add_argument("--deb-sig",
                   help=f"the .deb's .sig; writes {DEB_FEED_NAME} beside --out")
```

replace:

```python
    if not (a.mac_sig or a.linux_sig or a.win_sig):
        p.error("need at least one of --mac-sig / --linux-sig / --win-sig")
    feed = build_feed(
        a.version, a.tag, a.notes, a.pub_date,
        _read(a.mac_sig) if a.mac_sig else None,
        _read(a.linux_sig) if a.linux_sig else None,
        _read(a.win_sig) if a.win_sig else None,
        pubkey=load_pubkey_from_conf(a.pubkey_conf),
    )
```

with:

```python
    if not (a.mac_sig or a.linux_sig or a.win_sig or a.deb_sig):
        p.error("need at least one of --mac-sig / --linux-sig / --win-sig / --deb-sig")
    pubkey = load_pubkey_from_conf(a.pubkey_conf)
    # Both feeds are built, and so checked, before either is written: a bad
    # signature on one must not leave the other half-published beside it.
    deb = None
    if a.deb_sig:
        deb = build_deb_feed(a.version, a.tag, a.notes, a.pub_date,
                             _read(a.deb_sig), pubkey=pubkey)
    feed = None
    if a.mac_sig or a.linux_sig or a.win_sig:
        feed = build_feed(
            a.version, a.tag, a.notes, a.pub_date,
            _read(a.mac_sig) if a.mac_sig else None,
            _read(a.linux_sig) if a.linux_sig else None,
            _read(a.win_sig) if a.win_sig else None,
            pubkey=pubkey,
        )
    if deb:
        deb_out = os.path.join(os.path.dirname(os.path.abspath(a.out)), DEB_FEED_NAME)
        write_feed(deb_out, deb)
        print(f"wrote {deb_out}: version {deb['version']}, platform {DEB_KEY}")
    if not feed:
        return
```

and replace the last line of `main()`, `    write_atomic(a.out, feed)`, with `    write_feed(a.out, feed)`.

- [ ] **Step 6: Run the self-test, then the CLI with throwaway keys.**

```bash
python3 $WT/apps/node/scripts/gen-node-feed.py --self-test
T=$(mktemp -d) && python3 - "$T" "$WT/apps/node/scripts/gen-node-feed.py" <<'EOF'
import importlib.util, json, sys
T, script = sys.argv[1], sys.argv[2]
spec = importlib.util.spec_from_file_location("g", script)
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
json.dump({"plugins": {"updater": {"pubkey": g._fake_pubkey()}}}, open(f"{T}/conf.json", "w"))
open(f"{T}/linux.sig", "w").write(g._named("linux-x86_64", "0.6.33"))
open(f"{T}/deb.sig", "w").write(g._named(g.DEB_KEY, "0.6.33"))
EOF
python3 $WT/apps/node/scripts/gen-node-feed.py --version 0.6.33 --tag node-v0.6.33 --linux-sig "$T/linux.sig" --deb-sig "$T/deb.sig" --pubkey-conf "$T/conf.json" --out "$T/out/latest-node.json"
python3 -c "import json,sys; print(sorted(json.load(open(sys.argv[1]))['platforms']), sorted(json.load(open(sys.argv[2]))['platforms']))" "$T/out/node-deb.json" "$T/out/latest-node.json"
python3 $WT/apps/node/scripts/gen-node-feed.py --version 0.6.33 --tag node-v0.6.33 --linux-sig "$T/deb.sig" --deb-sig "$T/deb.sig" --pubkey-conf "$T/conf.json" --out "$T/out2/latest-node.json"; ls "$T/out2"
```

Expected, in order: `gen-node-feed self-test OK (app trusts key id 5D4392DA73BCC2A2)`; `wrote .../out/node-deb.json: version 0.6.33, platform linux-x86_64-deb` then `wrote .../out/latest-node.json — version 0.6.33, platforms: linux-x86_64` and the existing NOTE about darwin-aarch64 and windows-x86_64 (not linux-x86_64-deb); `['linux-x86_64-deb'] ['linux-x86_64']`; a `ValueError: linux-x86_64: signed as 'BTX-Node_0.6.33_amd64.deb', but this release must be signed as 'BTX-Node_0.6.33_amd64.AppImage'` and `ls: .../out2: No such file or directory` (nothing half-written).

- [ ] **Step 7: Commit.**

```bash
git -C $WT add apps/node/scripts/gen-node-feed.py
git -C $WT commit -F - <<'EOF'
node: gen-node-feed.py writes node-deb.json, and never the .deb key into latest-node.json

--deb-sig builds the one-key feed a .deb install reads, bound like every other
entry, and writes it beside --out. write_feed refuses linux-x86_64-deb in any
file but node-deb.json and anything else in node-deb.json: 0.6.32 refuses a
whole release that lists a key it does not know. Both feeds are checked before
either is written. The self-test covers both refusals.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 10: `build-node-feed.sh --deb`

**Files:**
- Modify: `apps/node/scripts/build-node-feed.sh` (usage :37-45, args :61-81, existence :144, names :193-195, signing :210-211, ARGS :214-217, summary :222-230)

**Interfaces:**
- Consumes: `gen-node-feed.py --deb-sig` (Task 9), `verify-updater-sig.py` (unchanged: it verifies any artifact under its own name).
- Produces: `--deb <BTX-Node_V_amd64.deb>`; writes `$FEED_OUT_DIR/node-deb.json` and `<deb>.sig`.

- [ ] **Step 1: Show the gap.** With a dummy key file the script gets past the key check and stops at the name check before signing anything:

```bash
T=$(mktemp -d) && printf deb > "$T/BTX-Node_0.6.32_amd64.deb" && printf key > "$T/dummy.key"
OBSERVER_OVERRIDE=1 EASYBTX_UPDATER_KEY="$T/dummy.key" bash $WT/apps/node/scripts/build-node-feed.sh --version 0.6.33 --deb "$T/BTX-Node_0.6.32_amd64.deb"; echo "exit=$?"
```

Expected before the change: `unknown argument: --deb`, `exit=1`.

- [ ] **Step 2: Implement.** Replace the usage lines:

```bash
# Usage:
#   build-node-feed.sh --version <ver> [--mac <app.tar.gz>] [--linux <.AppImage>]
#                      [--win <-setup.exe>] [--notes "..."]
#
#   # Linux-only release (mac + windows stay where they are):
#   build-node-feed.sh --version 0.6.5 --linux dist/BTX-Node_0.6.5_amd64.AppImage
```

with:

```bash
# Usage:
#   build-node-feed.sh --version <ver> [--mac <app.tar.gz>] [--linux <.AppImage>]
#                      [--deb <.deb>] [--win <-setup.exe>] [--notes "..."]
#
#   # Linux-only release (mac + windows stay where they are). A Linux release
#   # passes the .deb too: it is signed like the rest and goes into its OWN feed,
#   # node-deb.json, beside latest-node.json, which only .deb installs read.
#   build-node-feed.sh --version 0.6.33 \
#     --linux dist/BTX-Node_0.6.33_amd64.AppImage --deb dist/BTX-Node_0.6.33_amd64.deb
```

Replace the block from `VERSION="" ; MAC_TGZ=""` through the closing `fi` of the "need at least one of" check with:

```bash
VERSION="" ; MAC_TGZ="" ; LINUX_APPIMAGE="" ; LINUX_DEB="" ; WIN_SETUP="" ; NOTES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?}" ; shift 2 ;;
    --mac)     MAC_TGZ="${2:?}" ; shift 2 ;;
    --linux)   LINUX_APPIMAGE="${2:?}" ; shift 2 ;;
    --deb)     LINUX_DEB="${2:?}" ; shift 2 ;;
    --win)     WIN_SETUP="${2:?}" ; shift 2 ;;
    --notes)   NOTES="${2:?}" ; shift 2 ;;
    -h|--help) sed -n '1,45p' "$0" ; exit 0 ;;
    *) echo "unknown argument: $1" >&2
       echo "usage: $0 --version <ver> [--mac <f>] [--linux <f>] [--deb <f>] [--win <f>] [--notes <s>]" >&2
       exit 1 ;;
  esac
done

[ -n "$VERSION" ] || { echo "--version is required (e.g. --version 0.6.5)" >&2; exit 1; }
if [ -z "$MAC_TGZ" ] && [ -z "$LINUX_APPIMAGE" ] && [ -z "$LINUX_DEB" ] && [ -z "$WIN_SETUP" ]; then
  echo "need at least one of --mac / --linux / --deb / --win: a feed with no" >&2
  echo "platforms offers nothing to anyone." >&2
  exit 1
fi
```

Replace `for f in "$MAC_TGZ" "$LINUX_APPIMAGE" "$WIN_SETUP"; do` with `for f in "$MAC_TGZ" "$LINUX_APPIMAGE" "$LINUX_DEB" "$WIN_SETUP"; do`.

After the line `[ -n "$LINUX_APPIMAGE" ] && expect_name "$LINUX_APPIMAGE" "BTX-Node_${VERSION}_amd64.AppImage"` add:

```bash
[ -n "$LINUX_DEB" ]      && expect_name "$LINUX_DEB"      "BTX-Node_${VERSION}_amd64.deb"
```

After the line `if [ -n "$LINUX_APPIMAGE" ]; then sign "$LINUX_APPIMAGE"; verify "$LINUX_APPIMAGE"; fi` add:

```bash
if [ -n "$LINUX_DEB" ]; then sign "$LINUX_DEB"; verify "$LINUX_DEB"; fi
```

After the line `[ -n "$LINUX_APPIMAGE" ] && ARGS+=(--linux-sig "${LINUX_APPIMAGE}.sig")` add:

```bash
# The .deb's entry goes to node-deb.json beside --out, never into
# latest-node.json: gen-node-feed.py refuses that, because easyNode 0.6.32
# refuses a whole release that lists a key it does not know.
[ -n "$LINUX_DEB" ]      && ARGS+=(--deb-sig   "${LINUX_DEB}.sig")
```

Replace the closing summary, from the `echo` after the `python3 "$HERE/gen-node-feed.py"` call through `echo "Those assets MUST be attached to $TAG before the feed goes live."`, with:

```bash
echo
if [ -n "$MAC_TGZ" ] || [ -n "$LINUX_APPIMAGE" ] || [ -n "$WIN_SETUP" ]; then
  echo "feed written to $OUT_DIR/latest-node.json: every signature in it was"
  echo "verified against the app key."
fi
if [ -n "$LINUX_DEB" ]; then
  echo "feed written to $OUT_DIR/node-deb.json (the .deb only, verified the same"
  echo "way). The site PR deploys it together with latest-node.json."
fi
echo "Release-asset names the feed expects on $TAG:"
[ -n "$MAC_TGZ" ]        && echo "  BTX-Node_${VERSION}_aarch64.app.tar.gz   (= $MAC_TGZ)"
[ -n "$LINUX_APPIMAGE" ] && echo "  BTX-Node_${VERSION}_amd64.AppImage       (= $LINUX_APPIMAGE)"
[ -n "$LINUX_DEB" ]      && echo "  BTX-Node_${VERSION}_amd64.deb            (= $LINUX_DEB)"
[ -n "$WIN_SETUP" ]      && echo "  BTX-Node_${VERSION}_x64-setup.exe        (= $WIN_SETUP)"
echo
echo "Those assets, and every .sig beside them, MUST be attached to $TAG before"
echo "the feed goes live."
```

- [ ] **Step 3: Run it again.**

```bash
bash -n $WT/apps/node/scripts/build-node-feed.sh && echo syntax-ok
T=$(mktemp -d) && printf deb > "$T/BTX-Node_0.6.32_amd64.deb" && printf key > "$T/dummy.key"
OBSERVER_OVERRIDE=1 EASYBTX_UPDATER_KEY="$T/dummy.key" bash $WT/apps/node/scripts/build-node-feed.sh --version 0.6.33 --deb "$T/BTX-Node_0.6.32_amd64.deb"; echo "exit=$?"
```

Expected: `syntax-ok`; then `error: .../BTX-Node_0.6.32_amd64.deb is named 'BTX-Node_0.6.32_amd64.deb', but the feed will publish a URL ending` / `in 'BTX-Node_0.6.33_amd64.deb' (derived from --version 0.6.33).`, `exit=1`, and no `.sig` next to the file.

- [ ] **Step 4 (optional, end to end with a throwaway key; nothing it writes is kept).** In a scratch copy of the tree, swap in a throwaway key so signing and verifying both run:

```bash
T=$(mktemp -d); git -C $WT archive HEAD | tar -x -C "$T" && ln -s $WT/apps/node/node_modules "$T/apps/node/node_modules"
(cd "$T/apps/node" && npx tauri signer generate -w "$T/k.key" -p "" --ci >/dev/null)
PUB=$(cat "$T/k.key.pub"); KEYLINE=$(printf %s "$PUB" | base64 -d | sed -n 2p)
python3 - "$T/apps/node/src-tauri/tauri.conf.json" "$PUB" <<'EOF'
import json, sys
c = json.load(open(sys.argv[1])); c["plugins"]["updater"]["pubkey"] = sys.argv[2]
json.dump(c, open(sys.argv[1], "w"), indent=2)
EOF
sed -i '' "s|^PUBKEY=.*|PUBKEY=\"$KEYLINE\"|" "$T/apps/node/scripts/build-node-feed.sh"
mkdir -p "$T/art" && head -c 2000 /dev/urandom > "$T/art/BTX-Node_0.6.33_amd64.AppImage" && head -c 2000 /dev/urandom > "$T/art/BTX-Node_0.6.33_amd64.deb"
OBSERVER_OVERRIDE=1 EASYBTX_UPDATER_KEY="$T/k.key" FEED_OUT_DIR="$T/feed" bash "$T/apps/node/scripts/build-node-feed.sh" --version 0.6.33 --linux "$T/art/BTX-Node_0.6.33_amd64.AppImage" --deb "$T/art/BTX-Node_0.6.33_amd64.deb"
ls "$T/feed" "$T/art"
```

Expected: `signing BTX-Node_0.6.33_amd64.deb`, `verifying BTX-Node_0.6.33_amd64.deb against the app's public key`, both feed files in `$T/feed`, and `BTX-Node_0.6.33_amd64.deb.sig` beside the `.deb`.

- [ ] **Step 5: Commit.**

```bash
git -C $WT add apps/node/scripts/build-node-feed.sh
git -C $WT commit -F - <<'EOF'
node: build-node-feed.sh --deb signs the .deb and writes node-deb.json

The .deb is checked against its release name before anything is signed,
signed and verified against the app's key like the other builds, and handed
to gen-node-feed.py --deb-sig, which writes node-deb.json beside
latest-node.json.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 11: The publish gate wants a `.sig` beside the `.deb`

**Files:**
- Modify: `apps/node/scripts/publish-node-release.sh:95-100`
- Test: `apps/node/scripts/test-publish-gate.sh` (before the FORK case, :66)

**Interfaces:**
- Consumes: nothing new. Produces: a `.deb` without a `.sig` is refused with `error: <f> has no <f>.sig. The updater cannot serve an unsigned artifact.`

- [ ] **Step 1: Write the failing fixture.** In `test-publish-gate.sh`, replace `echo "== the box's own node on a fork: refused before any asset is read =="` with:

```bash
echo "== a .deb with no .sig: refused, like every build the updater serves =="
# Since 0.6.33 a .deb install updates itself from node-deb.json, so the .deb
# is served by the updater like the AppImage and must carry its signature.
printf 'deb bytes' > "$A/BTX-Node_${VER}_amd64.deb"
run
grep -q "error: BTX-Node_${VER}_amd64.deb has no BTX-Node_${VER}_amd64.deb.sig" "$T/out" || { echo "FAIL: an unsigned .deb was not refused"; cat "$T/out"; exit 1; }
printf 'sig' > "$A/BTX-Node_${VER}_amd64.deb.sig"
run
grep -q "amd64.deb has no" "$T/out" && { echo "FAIL: a signed .deb was refused for a missing .sig"; cat "$T/out"; exit 1; }
echo "   pass"

echo "== the box's own node on a fork: refused before any asset is read =="
```

- [ ] **Step 2: Run, see it fail.**

Run: `bash $WT/apps/node/scripts/test-publish-gate.sh`
Expected: the first three fixtures `pass`, then `FAIL: an unsigned .deb was not refused`, exit 1.

- [ ] **Step 3: Implement.** In `publish-node-release.sh`, replace:

```bash
# Anything the updater serves (.tar.gz, .AppImage, .exe) needs a sibling .sig.
missing=0
for f in "${ALL[@]}"; do
  case "$f" in
    *.sig|SHA256SUMS*|*.json) continue ;;
    *.tar.gz|*.AppImage|*.exe)
```

with:

```bash
# Anything the updater serves (.tar.gz, .AppImage, .exe, and since 0.6.33 the
# .deb, which a .deb install fetches through node-deb.json) needs a sibling .sig.
missing=0
for f in "${ALL[@]}"; do
  case "$f" in
    *.sig|SHA256SUMS*|*.json) continue ;;
    *.tar.gz|*.AppImage|*.exe|*.deb)
```

- [ ] **Step 4: Run, see it pass.**

Run: `bash $WT/apps/node/scripts/test-publish-gate.sh`
Expected: five `   pass` lines, the new one under `== a .deb with no .sig: refused, like every build the updater serves ==`, then `publish gate: all fixtures behave`.

- [ ] **Step 5: Commit.**

```bash
git -C $WT add apps/node/scripts/publish-node-release.sh apps/node/scripts/test-publish-gate.sh
git -C $WT commit -F - <<'EOF'
node: the publish gate refuses a .deb without its .sig

A .deb install now fetches the .deb through node-deb.json, so the .deb is an
updater artifact like the AppImage, the tarball and the installer. Fixture in
test-publish-gate.sh, which CI runs.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 12: The recipe, the README and the changelog; then the whole of Part 1

**Files:**
- Modify: `docs/node-release-recipe.md` (:37-39, table :110-114, step 6 :978-990, step 7 :992-997, step 8 :1061-1064, "Verifying" :1083-1089)
- Modify: `README.md:46-51`
- Modify: `apps/node/CHANGELOG.md:9` (`## [Unreleased]`)

**Interfaces:** documentation only.

- [ ] **Step 1: Recipe, the endpoints.** Replace:

```
**Endpoint, baked into the app at build time:**
`https://easybtx.com/updater/latest-node.json`
(`apps/node/src-tauri/tauri.conf.json` → `plugins.updater.endpoints`)
```

with:

```
**Endpoints, baked into the app at build time, in this order:**
`https://easybtx.com/updater/node-{{bundle_type}}.json`, then
`https://easybtx.com/updater/latest-node.json`
(`apps/node/src-tauri/tauri.conf.json`, `plugins.updater.endpoints`). The
plugin puts the install type in the first (`deb`, `appimage`, `app`, `nsis`,
`msi`, `rpm` or `unknown`). Only `node-deb.json` exists; every other name
answers 404 and the plugin moves on to `latest-node.json`. 0.6.32 and older
read `latest-node.json` only. See "The .deb feed" below.
```

- [ ] **Step 2: Recipe, the table and the `.deb` feed.** Replace the table:

```
| Target | Updater artifact |
|---|---|
| `darwin-aarch64` | `.app.tar.gz` (**not** the .dmg — that is manual-install only) |
| `linux-x86_64` | the `.AppImage` (same file users download) |
| `windows-x86_64` | the `-setup.exe` (v2 reuses it; not a `.nsis.zip`) |
```

with:

```
| Target | Updater artifact |
|---|---|
| `darwin-aarch64` | `.app.tar.gz` (**not** the .dmg — that is manual-install only) |
| `linux-x86_64` | the `.AppImage` (same file users download) |
| `linux-x86_64-deb` | the `.deb`, in `node-deb.json` ONLY, never in `latest-node.json` |
| `windows-x86_64` | the `-setup.exe` (v2 reuses it; not a `.nsis.zip`) |

### The .deb feed, `node-deb.json` (from 0.6.33)

A `.deb` install reads `https://easybtx.com/updater/node-deb.json` first. It is
the same schema as `latest-node.json` with exactly one platform key,
`linux-x86_64-deb`; its url is `.../node-v<V>/BTX-Node_<V>_amd64.deb` and its
signature is that file's `.sig`, signed under that name. The app installs it
with `pkexec dpkg -i`, after one password prompt. The design is
`docs/decisions/2026-09-28-deb-installs-update-themselves.md`.

- **`linux-x86_64-deb` never goes into `latest-node.json`.** easyNode 0.6.32
  refuses a whole release that lists a key it does not know, so that one key
  would stop every 0.6.32 install from updating, on every platform.
  `gen-node-feed.py` refuses to write it there, and the website's
  `scripts/check-node-links.py` fails the site PR.
- **`node-deb.json` moves with the Linux download**: its version is
  `REL_LINUX` and the `.deb` link on `/node`, in the same site PR as
  `latest-node.json`.
- **No other `node-*.json` is ever published, and nothing but a static file
  answers under `/updater`.** An install reads its typed feed instead of
  `latest-node.json`; a 204 there, or a 200 page that is not a feed, stops its
  updates.
- **A missing `node-deb.json` is safe, and removing it is the rollback.** A
  `.deb` install then reads `latest-node.json`, declines the AppImage without
  downloading it, and shows the command to install by hand.
```

- [ ] **Step 3: Recipe, step 6.** Replace:

````
   ```bash
   bash apps/node/scripts/build-node-feed.sh --version <ver> \
     [--mac <mac.app.tar.gz>] [--linux <linux.AppImage>] [--win <win-setup.exe>] \
     [--notes "<notes>"]
   ```
   It **writes** `site/public/updater/latest-node.json`. Never hand-edit that file.
   Every platform key it emits points at an asset at `<ver>`, so those assets
   must be attached to the release in step 7 before this feed is deployed in
   step 8.
````

with:

````
   ```bash
   bash apps/node/scripts/build-node-feed.sh --version <ver> \
     [--mac <mac.app.tar.gz>] [--linux <linux.AppImage>] [--deb <linux.deb>] \
     [--win <win-setup.exe>] [--notes "<notes>"]
   ```
   It **writes** `latest-node.json` into `$FEED_OUT_DIR` (default
   `apps/node/dist-feed`), and with `--deb` also `node-deb.json` beside it. A
   Linux release always passes both `--linux` and `--deb`. Never hand-edit
   either file. Every platform key they emit points at an asset at `<ver>`, so
   those assets and their `.sig` files must be attached to the release in
   step 7 before the feeds are deployed in step 8.
````

- [ ] **Step 4: Recipe, step 7.** Replace `upload -> re-download -> verify -> flip sequence in the right order, refuses` / `an artifact with no signature, refuses one whose signature does not verify` with:

```
   upload -> re-download -> verify -> flip sequence in the right order, refuses
   an artifact with no signature (the `.tar.gz`, the `.AppImage`, the
   `-setup.exe` and, since 0.6.33, the `.deb`), refuses one whose signature does not verify
```

- [ ] **Step 5: Recipe, step 8.** Replace:

```
8. **Merge the site PR** so Vercel deploys the new feed — and only AFTER the
   GitHub release is live with every asset returning 200, or easybtx.com serves
   download buttons that 404.
```

with:

```
8. **Merge the site PR** so Vercel deploys the new feed, and only AFTER the
   GitHub release is live with every asset returning 200, or easybtx.com serves
   download buttons that 404. The PR copies `latest-node.json` into
   `site/public/updater/`, and on a Linux release `node-deb.json` too, in the
   same commit as the `REL_LINUX` bump; `scripts/check-node-links.py` fails
   the PR when they disagree.

   On the 0.6.33 site PR only: the `/node` paragraph that begins "One exception
   on Linux: a .deb install is told, not updated" stops being true. Replace it
   with: "<strong>On Linux, a .deb install updates too, from 0.6.33 on.</strong>
   It asks for your password once, the way installing any system package does.
   A .deb copy older than 0.6.33 is told about the new version, and you install
   it by hand one last time with the command on this page. On a machine with no
   desktop to show the prompt, a .deb copy shows that command every time. The
   AppImage updates itself without asking."
```

- [ ] **Step 6: Recipe, verifying.** Replace:

```
- `curl -s https://easybtx.com/updater/latest-node.json | jq .version`
```

with:

```
- `curl -s https://easybtx.com/updater/latest-node.json | jq .version`
- On a Linux release: `curl -s https://easybtx.com/updater/node-deb.json | jq '.version, (.platforms | keys)'`
  prints the new version and `["linux-x86_64-deb"]`, nothing else.
- `curl -s -o /dev/null -w '%{http_code}\n' https://easybtx.com/updater/node-appimage.json`
  prints `404`. Anything in 2xx or 3xx stops every 0.6.33+ AppImage from updating.
```

- [ ] **Step 7: README.** Replace the block (README.md:46-51):

```
> **On Linux, pick the `.AppImage` if you want automatic updates.** The updater
> can replace an AppImage in place; it cannot replace a `.deb`, because the
> installed files live under `/usr` and belong to root. A `.deb` install is a
> perfectly good node — it just tells you when a new version exists and leaves
> the install to you and `dpkg`. That is a real difference between the two
> downloads, so it is said here rather than discovered later.
```

with:

```
> **On Linux, both downloads can update themselves, in different ways.** The
> `.AppImage` replaces itself in place without asking. A `.deb` install, from
> 0.6.33 on, installs the new `.deb` after asking for your password once,
> because its files live under `/usr` and belong to root. On a machine with no
> desktop to show that prompt, a `.deb` copy shows the command instead and
> leaves the install to you and `apt`. A `.deb` copy older than 0.6.33 moves to
> 0.6.33 by hand once.
```

- [ ] **Step 8: Changelog.** In `apps/node/CHANGELOG.md`, replace:

```
## [Unreleased]

```

with:

```
## [Unreleased]

**A .deb install updates itself, after one password prompt.** Until now only
the AppImage updated itself on Linux. A copy installed from the .deb downloaded
the AppImage on every check, could not install it, and tried again six hours
later. From this version on, a .deb copy reads a feed of its own and installs
the new .deb after asking for your password once, the way any system package
does. Where there is no .deb to install, it downloads nothing and shows the
command to install it by hand. On every platform, an update that downloaded but
failed to install is not downloaded again by the automatic checks; Check now
still tries. A .deb copy on 0.6.32 or older moves to this version by hand one
last time, with the command on easybtx.com/node.

```

- [ ] **Step 9: The whole of Part 1, green.**

```bash
cargo test --manifest-path $WT/apps/node/src-tauri/Cargo.toml --locked
cargo fmt --manifest-path $WT/apps/node/src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path $WT/apps/node/src-tauri/Cargo.toml --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
(cd $WT/apps/node && npx tsc --noEmit && npx vitest run && npx vite build)
python3 $WT/apps/node/scripts/gen-node-feed.py --self-test
bash $WT/apps/node/scripts/test-publish-gate.sh
git -C $WT diff origin/main --stat
```

Expected: `test result: ok. 100 passed; 0 failed; 1 ignored` (plus two empty `ok. 0 passed` results for the bin and doc targets); fmt silent; clippy without `error`; tsc silent, `Tests  130 passed (130)`, `✓ built in`; `gen-node-feed self-test OK (app trusts key id 5D4392DA73BCC2A2)`; `publish gate: all fixtures behave`; the diff stat lists only the files in the File structure table plus the design doc.

- [ ] **Step 10: Commit.**

```bash
git -C $WT add docs/node-release-recipe.md README.md apps/node/CHANGELOG.md
git -C $WT commit -F - <<'EOF'
docs: the .deb feed in the release recipe, the README and the changelog

The recipe names both endpoints, the four rules for node-deb.json, the
--deb flag, the .deb's .sig in the publish gate, deploying both feeds in the
site PR, the 0.6.33 copy for /node, and two curl checks. The README's Linux
note and the Unreleased entry say what a .deb copy does now.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

# Part 2: the website repository (EasyBTX, new branch `claude/node-deb-feed-check`)

## Before you start (Part 2)

- [ ] **Step A: Worktree on a new branch from `origin/main`.** The main checkout at `/Users/m2promende/repos/EasyBTX` sits on an old detached HEAD; do not work there.

```bash
git -C /Users/m2promende/repos/EasyBTX fetch origin main
git -C /Users/m2promende/repos/EasyBTX worktree add -b claude/node-deb-feed-check /Users/m2promende/repos/EasyBTX/.claude/worktrees/node-deb-feed-check origin/main
python3 /Users/m2promende/repos/EasyBTX/.claude/worktrees/node-deb-feed-check/scripts/check-node-links.py
```

Expected (at `origin/main` = `e187d14c`): `OK: BTX Node links split correctly, mac node-v0.6.32, win node-v0.6.32, linux node-v0.6.32, 4 links checked.`, `OK: every latest-node.json platform key points at its own version.`, `OK: every latest-node.json entry is signed under its release name.`

`$SITE` below is `/Users/m2promende/repos/EasyBTX/.claude/worktrees/node-deb-feed-check`.

### Task 13: `check-node-links.py` enforces the `.deb` feed's rules, each with a sabotage case

**Files:**
- Modify: `scripts/check-node-links.py` (paths :16-18; the latest-node.json loop :264-266; new section before `# ── No page may PROMISE an update` :304; final output :434-435)

**Interfaces:**
- Consumes: existing `signed_name(sig_b64)` (:175), `_fake_sig(trusted, first=...)` (:193), `fail(msg)` (:23), `src` (node.astro text, :27), `rel_lnx` (:49).
- Produces: `deb_feed_problems(deb_feed, main_keys, rel_linux, deb_link_versions, node_feed_names, page_routes, vercel_rules) -> list[str]`, `_reaches_updater(rule) -> bool`, `_self_check_deb_feed()`.

- [ ] **Step 1: Paths and constants.** Replace `FEED = ROOT / "site/public/updater/latest-node.json"` with:

```python
FEED = ROOT / "site/public/updater/latest-node.json"
UPDATER_DIR = ROOT / "site/public/updater"
DEB_FEED = UPDATER_DIR / "node-deb.json"
PAGES_UPDATER = ROOT / "site/src/pages/updater"
VERCEL = ROOT / "site/vercel.json"
# The .deb's platform key and feed: see "The .deb's own feed" below.
DEB_KEY = "linux-x86_64-deb"
DEB_FEED_NAME = "node-deb.json"
DEB_FEED_SINCE = (0, 6, 33)
RELEASES = "https://github.com/MendeMatthias/EasyBTX-releases/releases/download"
UPDATER_HOSTS = {"easybtx.com", "www.easybtx.com"}
```

- [ ] **Step 2: Write the self-check first, against a rule function that catches nothing.** Insert immediately before the line `# ── No page may PROMISE an update to a platform the feed cannot serve ──────`:

```python
# ── The .deb's own feed, node-deb.json ─────────────────────────────────────
#
# From 0.6.33, easyNode asks for node-<its install type>.json FIRST and falls
# back to latest-node.json only when that answers a non-success status
# (docs/decisions/2026-09-28-deb-installs-update-themselves.md in
# MendeMatthias/easynode). Only a .deb install has a feed of its own. What
# can go wrong, and the rule that stops each:
#
#   * The .deb key in latest-node.json: easyNode 0.6.32 refuses a whole
#     release that lists a key it does not know, so every 0.6.32 install, on
#     every platform, would stop updating.
#   * node-deb.json with any other key, a URL under another tag, a build
#     signed under another name, or a version that is not the site's Linux
#     download: a .deb install would be offered what it cannot or must not
#     install, or refuse the whole release.
#   * Any other node-*.json: that install type would read it INSTEAD of
#     latest-node.json.
#   * Anything but a static file answering under /updater (a route, a Vercel
#     redirect or rewrite): the plugin gives up on a 204 without trying
#     latest-node.json, and a 200 page that is not a feed fails the whole
#     check, on every 0.6.33+ install whose name it answers.
#
# A missing node-deb.json is allowed: it is the rollback, and .deb installs
# then fall back to latest-node.json and are told to update by hand.


def deb_feed_problems(deb_feed, main_keys, rel_linux, deb_link_versions,
                      node_feed_names, page_routes, vercel_rules):
    return []


def _self_check_deb_feed():
    """Every rule catches its sabotage, and a correct tree passes."""
    v = "0.6.40"
    asset = f"BTX-Node_{v}_amd64.deb"

    def good_feed():
        return {"version": v, "notes": "n", "pub_date": "2026-09-29T00:00:00Z",
                "platforms": {DEB_KEY: {
                    "signature": _fake_sig(f"timestamp:0\tfile:{asset}"),
                    "url": f"{RELEASES}/node-v{v}/{asset}"}}}

    def args(**change):
        a = dict(deb_feed=good_feed(),
                 main_keys={"darwin-aarch64", "linux-x86_64", "windows-x86_64"},
                 rel_linux=v, deb_link_versions=[v], node_feed_names=[DEB_FEED_NAME],
                 page_routes=[], vercel_rules=[
                     {"source": "/", "has": [{"type": "host", "value": "btc2btx.com"}],
                      "destination": "/btc2btx/"},
                     {"source": "/((?!ctl-).+)",
                      "has": [{"type": "host", "value": "hq.easybtx.com"}],
                      "destination": "https://easybtx.com/"},
                     {"source": "/sitemap.xml", "destination": "/sitemap-index.xml"},
                     {"source": "/qid/:path*",
                      "has": [{"type": "host", "value": "btc2btx.com"}],
                      "destination": "https://api.btxscan.io/qid/:path*"}])
        a.update(change)
        return a

    def feed_with(fn):
        f = good_feed()
        fn(f)
        return f

    bad = []
    if deb_feed_problems(**args()):
        bad.append(f"a correct node-deb.json was flagged: {deb_feed_problems(**args())}")
    if deb_feed_problems(**args(deb_feed=None, node_feed_names=[])):
        bad.append("a site with no node-deb.json yet (or rolled back) was flagged")
    sabotage = [
        ("the .deb key in latest-node.json",
         args(main_keys={"linux-x86_64", DEB_KEY}), "must never"),
        ("a second key in node-deb.json",
         args(deb_feed=feed_with(lambda f: f["platforms"].update(
             {"linux-x86_64": f["platforms"][DEB_KEY]}))), "must list exactly"),
        ("the AppImage's key instead of the .deb's",
         args(deb_feed=feed_with(lambda f: f.update(
             platforms={"linux-x86_64": f["platforms"][DEB_KEY]}))), "must list exactly"),
        ("a URL under another tag",
         args(deb_feed=feed_with(lambda f: f["platforms"][DEB_KEY].update(
             url=f"{RELEASES}/node-v0.6.39/{asset}"))), "url is"),
        ("a .deb signed under the AppImage's name",
         args(deb_feed=feed_with(lambda f: f["platforms"][DEB_KEY].update(
             signature=_fake_sig(f"timestamp:0\tfile:BTX-Node_{v}_amd64.AppImage")))),
         "signed as"),
        ("a version off the Linux pin", args(rel_linux="0.6.41"), "REL_LINUX"),
        ("a version off the .deb download", args(deb_link_versions=["0.6.39"]),
         ".deb download"),
        ("another typed feed",
         args(node_feed_names=[DEB_FEED_NAME, "node-appimage.json"]), "only typed feed"),
        ("a route under /updater",
         args(page_routes=["[name].json.ts"]), "Only static files"),
        ("a redirect under /updater", args(vercel_rules=[
            {"source": "/updater/:file", "destination": "/"}]), "vercel.json"),
        ("a catch-all rewrite", args(vercel_rules=[
            {"source": "/(.*)", "destination": "/index.html"}]), "vercel.json"),
    ]
    for label, a, phrase in sabotage:
        found = deb_feed_problems(**a)
        if not any(phrase in p for p in found):
            bad.append(f"{label}: not caught (got {found})")
    if bad:
        print("ERROR: the node-deb.json guard does not work, so it would pass a site "
              "that stops .deb or every 0.6.32 install from updating:", file=sys.stderr)
        for b in bad:
            print(f"  - {b}", file=sys.stderr)
        sys.exit(1)


_self_check_deb_feed()

```

- [ ] **Step 3: Run, see every sabotage go uncaught.**

Run: `python3 $SITE/scripts/check-node-links.py`
Expected: exit 1, `ERROR: the node-deb.json guard does not work, ...` followed by eleven lines, `  - the .deb key in latest-node.json: not caught (got [])` through `  - a catch-all rewrite: not caught (got [])`.

- [ ] **Step 4: Implement the rules.** Replace the stub:

```python
def deb_feed_problems(deb_feed, main_keys, rel_linux, deb_link_versions,
                      node_feed_names, page_routes, vercel_rules):
    return []
```

with:

```python
def _reaches_updater(rule):
    """Can this Vercel redirect or rewrite answer a request under /updater on
    easybtx.com? A rule scoped to another host cannot; a source that starts
    with /updater, or whose first segment is a pattern, can."""
    hosts = [h.get("value") for h in (rule.get("has") or []) if h.get("type") == "host"]
    if hosts and not UPDATER_HOSTS.intersection(hosts):
        return False
    src = rule.get("source") or ""
    if src.startswith("/updater"):
        return True
    first = src[1:].split("/", 1)[0]
    return bool(first) and first[0] in "(:"


def deb_feed_problems(deb_feed, main_keys, rel_linux, deb_link_versions,
                      node_feed_names, page_routes, vercel_rules):
    """Every rule above, as a list of problems. Pure, so that each rule has a
    sabotage case in _self_check_deb_feed.

    deb_feed: node-deb.json parsed, or None when there is none.
    main_keys: the platform keys latest-node.json lists.
    rel_linux: the REL_LINUX pin's version.
    deb_link_versions: the version of every BTX-Node_<v>_amd64.deb on node.astro.
    node_feed_names: the name of every node-*.json in site/public/updater.
    page_routes: every file under site/src/pages/updater.
    vercel_rules: every redirect and rewrite in site/vercel.json."""
    problems = []
    if DEB_KEY in main_keys:
        problems.append(
            f"latest-node.json lists {DEB_KEY}, and it must never: easyNode 0.6.32 "
            f"refuses a whole release that lists a key it does not know, so every "
            f"0.6.32 install would stop updating, on every platform. The .deb entry "
            f"belongs in {DEB_FEED_NAME} only")
    extra = sorted(n for n in node_feed_names if n != DEB_FEED_NAME)
    if extra:
        problems.append(
            f"site/public/updater has {', '.join(extra)}. The only typed feed is "
            f"{DEB_FEED_NAME}: easyNode 0.6.33+ reads node-<its install type>.json "
            f"before latest-node.json, so that install type would read this instead")
    if page_routes:
        problems.append(
            f"site/src/pages/updater has {', '.join(page_routes)}. Only static files "
            f"may answer under /updater: a route could answer node-*.json with a 204, "
            f"and the updater stops there without trying latest-node.json")
    for rule in vercel_rules:
        if _reaches_updater(rule):
            problems.append(
                f"site/vercel.json rule {rule.get('source')!r} can answer under "
                f"/updater on easybtx.com. A redirect or rewrite there turns the 404 "
                f"that sends easyNode 0.6.33+ on to latest-node.json into a page or a "
                f"204, and those installs stop updating")
    if deb_feed is None:
        return problems
    keys = sorted(deb_feed.get("platforms") or {})
    if keys != [DEB_KEY]:
        problems.append(f"{DEB_FEED_NAME} lists {keys}; it must list exactly {DEB_KEY}")
        return problems
    v = deb_feed.get("version") or ""
    entry = deb_feed["platforms"][DEB_KEY]
    asset = f"BTX-Node_{v}_amd64.deb"
    if entry.get("url") != f"{RELEASES}/node-v{v}/{asset}":
        problems.append(
            f"{DEB_FEED_NAME}: url is {entry.get('url')!r}, not "
            f"{RELEASES}/node-v{v}/{asset}")
    named = signed_name(entry.get("signature"))
    if named != asset:
        problems.append(
            f"{DEB_FEED_NAME}: signed as {named!r}, not {asset!r}. easyNode refuses "
            f"a build signed under another name. Sign the .deb as {asset} "
            f"(easynode's build-node-feed.sh --deb does) and rebuild the feed")
    if v != rel_linux:
        problems.append(
            f"{DEB_FEED_NAME} is {v} but REL_LINUX is node-v{rel_linux}. It moves "
            f"with the Linux download, in the same site PR")
    if v not in deb_link_versions:
        problems.append(
            f"{DEB_FEED_NAME} is {v} but node.astro's .deb download is "
            f"{', '.join(deb_link_versions) or 'missing'}")
    return problems
```

- [ ] **Step 5: Wire it to the real files.** Replace the line that CALLS the self-check (column 0, right after the function body; not the `def _self_check_deb_feed():` line) and the blank line after it:

```python
_self_check_deb_feed()

```

with:

```python
_self_check_deb_feed()

_vercel = json.loads(VERCEL.read_text(encoding="utf-8")) if VERCEL.exists() else {}
_deb_feed = json.loads(DEB_FEED.read_text(encoding="utf-8")) if DEB_FEED.exists() else None
for problem in deb_feed_problems(
        deb_feed=_deb_feed,
        main_keys=set((json.loads(FEED.read_text(encoding="utf-8")).get("platforms") or {}))
        if FEED.exists() else set(),
        rel_linux=rel_lnx,
        deb_link_versions=re.findall(r"BTX-Node_([0-9.]+)_amd64\.deb", src),
        node_feed_names=sorted(p.name for p in UPDATER_DIR.glob("node-*.json")),
        page_routes=sorted(str(p.relative_to(PAGES_UPDATER))
                           for p in PAGES_UPDATER.rglob("*") if p.is_file())
        if PAGES_UPDATER.exists() else [],
        vercel_rules=(_vercel.get("redirects") or []) + (_vercel.get("rewrites") or [])):
    fail(problem)

```

In the `latest-node.json` loop, replace:

```python
    for k, p in (d.get("platforms") or {}).items():
        url = p.get("url", "")
```

with (otherwise the key hits "unknown updater target; add it to FEED_PIN", which is exactly the wrong advice):

```python
    for k, p in (d.get("platforms") or {}).items():
        # Reported by deb_feed_problems below, with the reason. Never add it
        # to FEED_PIN: it must not be in this file at all.
        if k == DEB_KEY:
            continue
        url = p.get("url", "")
```

and replace the first `print` of the final output:

```python
print(f"OK: BTX Node links split correctly, mac node-v{rel_mac}, "
      f"win node-v{rel_win}, linux node-v{rel_lnx}, {seen} links checked.")
```

with:

```python
print(f"OK: BTX Node links split correctly, mac node-v{rel_mac}, "
      f"win node-v{rel_win}, linux node-v{rel_lnx}, {seen} links checked.")
if _deb_feed is not None:
    print(f"OK: node-deb.json lists only {DEB_KEY}, at node-v{rel_lnx}, signed under "
          f"its release name, and nothing else answers under /updater.")
elif tuple(int(x) for x in rel_lnx.split(".")) >= DEB_FEED_SINCE:
    print(f"NOTE: no node-deb.json, so .deb installs read latest-node.json and are "
          f"told to update by hand. Right for a rollback; otherwise run easynode's "
          f"build-node-feed.sh with --deb and deploy node-deb.json.")
```

- [ ] **Step 6: Run, see it pass on the real tree.**

```bash
python3 $SITE/scripts/check-node-links.py
(cd $SITE && bash scripts/check-download-links.sh | tail -3)
```

Expected: the same three `OK:` lines as before this task (there is no `node-deb.json` yet and `REL_LINUX` is 0.6.32, so neither the new OK line nor the NOTE prints); `check-download-links.sh` ends with those lines too.

- [ ] **Step 7: The self-check really guards the rules.** Break one rule on purpose and watch the self-check refuse, then put it back:

```bash
cp $SITE/scripts/check-node-links.py /tmp/cnl.py
sed -i '' 's/    if DEB_KEY in main_keys:/    if False and DEB_KEY in main_keys:/' $SITE/scripts/check-node-links.py
python3 $SITE/scripts/check-node-links.py; echo "exit=$?"
cp /tmp/cnl.py $SITE/scripts/check-node-links.py && git -C $SITE diff --stat
```

Expected: `ERROR: the node-deb.json guard does not work, ...`, `  - the .deb key in latest-node.json: not caught (got [])`, `exit=1`; afterwards the diff stat shows only `scripts/check-node-links.py` with this task's changes.

- [ ] **Step 8: The wiring, end to end, on a copy of the tree.** Each sabotage below is applied to the real files in a throwaway copy, so a rule that is correct but never reached would show here:

```bash
C=$(mktemp -d) && git -C $SITE archive HEAD scripts/check-node-links.py site/src/pages/node.astro site/src/pages/node/how-to.astro site/public/updater site/vercel.json | tar -x -C "$C"
cp $SITE/scripts/check-node-links.py "$C/scripts/"
REL=$(sed -n "s|^const REL_LINUX = '.*/node-v\([0-9.]*\)';|\1|p" "$C/site/src/pages/node.astro")
python3 - "$C" "$REL" <<'EOF'
import base64, json, sys
C, v = sys.argv[1], sys.argv[2]; asset = f"BTX-Node_{v}_amd64.deb"
raw = f"untrusted comment: x\nRUQAAAAA\ntrusted comment: timestamp:0\tfile:{asset}\nZg==\n"
json.dump({"version": v, "notes": "n", "pub_date": "2026-09-29T00:00:00Z", "platforms": {"linux-x86_64-deb": {
    "signature": base64.b64encode(raw.encode()).decode(),
    "url": f"https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v{v}/{asset}"}}},
    open(f"{C}/site/public/updater/node-deb.json", "w"), indent=2)
EOF
echo "--- a correct node-deb.json"; python3 "$C/scripts/check-node-links.py" | grep node-deb
echo "--- another typed feed"; cp "$C/site/public/updater/node-deb.json" "$C/site/public/updater/node-appimage.json"; python3 "$C/scripts/check-node-links.py" 2>&1 | tail -1; rm "$C/site/public/updater/node-appimage.json"
echo "--- the .deb key in latest-node.json"; cp "$C/site/public/updater/latest-node.json" "$C/ln.json"; python3 -c "
import json,sys; p=sys.argv[1]+'/site/public/updater/'; d=json.load(open(p+'latest-node.json')); d['platforms']['linux-x86_64-deb']=json.load(open(p+'node-deb.json'))['platforms']['linux-x86_64-deb']; json.dump(d,open(p+'latest-node.json','w'),indent=2)" "$C"; python3 "$C/scripts/check-node-links.py" 2>&1 | grep -c "FEED_PIN"; python3 "$C/scripts/check-node-links.py" 2>&1 | tail -1; cp "$C/ln.json" "$C/site/public/updater/latest-node.json"
echo "--- a route under /updater"; mkdir -p "$C/site/src/pages/updater" && touch "$C/site/src/pages/updater/[name].json.ts"; python3 "$C/scripts/check-node-links.py" 2>&1 | tail -1; rm -rf "$C/site/src/pages/updater"
echo "--- a redirect under /updater"; cp "$C/site/vercel.json" "$C/v.json"; python3 -c "
import json,sys; p=sys.argv[1]+'/site/vercel.json'; d=json.load(open(p)); d['redirects'].append({'source':'/updater/:f','destination':'/'}); json.dump(d,open(p,'w'),indent=2)" "$C"; python3 "$C/scripts/check-node-links.py" 2>&1 | tail -1; cp "$C/v.json" "$C/site/vercel.json"
```

Expected: `OK: node-deb.json lists only linux-x86_64-deb, at node-v0.6.32, signed under its release name, and nothing else answers under /updater.`; then one `  - ...` line per sabotage naming `node-appimage.json`, `latest-node.json lists linux-x86_64-deb, and it must never`, `site/src/pages/updater has [name].json.ts`, and `site/vercel.json rule '/updater/:f'`; the `grep -c "FEED_PIN"` line prints `0` (the misleading advice is gone).

- [ ] **Step 9: Commit.**

```bash
git -C $SITE add scripts/check-node-links.py
git -C $SITE commit -F - <<'EOF'
site: check-node-links.py guards node-deb.json, the .deb's own update feed

easyNode 0.6.33 asks for node-<install type>.json first and falls back to
latest-node.json on a non-success status. The check now fails when
latest-node.json lists linux-x86_64-deb (0.6.32 would refuse every release),
when node-deb.json lists anything but that key, points under another tag, was
signed under another name, or is off REL_LINUX or the .deb link, when any other
node-*.json exists, and when a route or a Vercel rule could answer under
/updater. Each rule has a sabotage case in the script's own self-check, and a
missing node-deb.json stays allowed: it is the rollback.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 14: A live probe: the other typed names must stay 404

**Files:**
- Modify: `.github/workflows/site-links.yml` (append a step after "Is the site offering what the fleet is actually on?")

**Interfaces:** none; CI only.

- [ ] **Step 1: See what the probe will see today.**

```bash
for t in appimage app nsis msi rpm unknown; do printf "node-%s.json -> " $t; curl -sS -o /dev/null -w '%{http_code}\n' --max-time 30 "https://easybtx.com/updater/node-$t.json" || echo 000; done
```

Expected (measured 2026-09-29): six lines ending `-> 404`.

- [ ] **Step 2: Add the step.** Append to the end of `site-links.yml`, at the same indentation as the other `- name:` steps under `steps:`:

```yaml

      # easyNode 0.6.33 and later ask for node-<install type>.json FIRST and
      # fall back to latest-node.json only on a non-success status. For every
      # name but node-deb.json that answer must stay a plain 404: a 204 makes
      # the updater stop with "no update", and a 200 page, or a redirect to
      # one, fails the whole check (tauri-plugin-updater 2.11.0,
      # Updater::check). Either stops every 0.6.33+ install of that type from
      # updating, on the day the host starts answering it. check-node-links.py
      # guards the repository; this guards the host. curl does not follow
      # redirects here, so a 3xx is seen as a 3xx.
      - name: Typed node feeds other than node-deb.json answer 404
        run: |
          set -uo pipefail
          bad=0
          for t in appimage app nsis msi rpm unknown; do
            url="https://easybtx.com/updater/node-$t.json"
            code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 30 "$url" || echo 000)
            echo "node-$t.json -> HTTP $code"
            case "$code" in
              2*|3*)
                echo "::error::$url answered $code. Every easyNode 0.6.33+ install of that type stops updating until it answers 404."
                bad=1 ;;
              404) ;;
              *)
                echo "::warning::$url answered $code, not 404. A non-success status still falls through to latest-node.json, so this is not an outage, but check the host." ;;
            esac
          done
          exit $bad
```

- [ ] **Step 3: Check the YAML and run the step's script locally.**

```bash
python3 -c "import yaml,sys; d=yaml.safe_load(open(sys.argv[1])); print([s.get('name') for s in d['jobs']['check-links']['steps']])" $SITE/.github/workflows/site-links.yml
python3 -c "import yaml,sys; d=yaml.safe_load(open(sys.argv[1])); print(d['jobs']['check-links']['steps'][-1]['run'])" $SITE/.github/workflows/site-links.yml | bash; echo "exit=$?"
```

Expected: the step list ends with `'Typed node feeds other than node-deb.json answer 404'`; six `-> HTTP 404` lines and `exit=0`. (If `yaml` is not installed: `python3 -m pip install --user pyyaml`, or read the file by eye.)

- [ ] **Step 4: Commit.**

```bash
git -C $SITE add .github/workflows/site-links.yml
git -C $SITE commit -F - <<'EOF'
ci: the other typed node feeds must stay 404 on easybtx.com

easyNode 0.6.33+ reads node-<install type>.json before latest-node.json and
moves on only on a non-success status. A 2xx or 3xx for any name but
node-deb.json stops that install type from updating, so site-links fails
when one appears. Other statuses warn.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

## After both parts (release time, not on these branches)

- **Merge order.** Part 2 can merge first: with no `node-deb.json` it changes nothing for anyone. Part 1 ships in 0.6.33.
- **The 0.6.33 release** follows `docs/node-release-recipe.md`: `build-node-feed.sh --version 0.6.33 --linux ... --deb ...`; the asset dir carries `BTX-Node_0.6.33_amd64.deb.sig`; the site PR adds `site/public/updater/node-deb.json` at 0.6.33, updates `latest-node.json`, bumps `REL_LINUX`, and replaces the `.deb` paragraph on `/node` with the copy in recipe step 8. `check-node-links.py` then prints the `OK: node-deb.json ...` line.
- **The design's end-to-end acceptance** (an Ubuntu desktop and a headless Ubuntu machine, two signed test builds pointed at a local server) needs a real machine. Two things from the plugin source (updater.rs:1177-1216) to expect: cancelling the pkexec dialog is followed by a second password dialog (zenity or kdialog) before the install fails; and an app started from a terminal can reach the last fallback, plain `sudo dpkg -i`, which waits for a password in that terminal. After a cancel, `update-check.log` shows `install-failed` for the version, `easybtx-node-app.json` shows `"update_install_failed": "<version>"`, and the next automatic check records `check-failed` with the "failed to install here" notice and downloads nothing; "Check now" downloads it again.

## What was verified, and where

Read at `origin/main` = `b008f98` (identical to the files touched on `origin/claude/deb-self-update`, which adds only the design doc), in the plugin sources in `~/.cargo/registry`, and in EasyBTX `origin/main` = `e187d14c`.

- `apps/node/src-tauri/src/update_binding.rs`: `PLATFORM_SUFFIXES` :75-79; `expected_signed_name` :83-88; `binding_refusal` :106-140 and the unknown-key refusal :122-124; `Decision` :143-151; `decide` :155-171; `REFUSAL` :176; `comparator` :179-181; `judge` :184-195; `take_refusal` :198-200; `high_water` :204-208; `remember_running_version` :224-228; tests helpers `feed` :258, `release` :262, `sig` :266, `ver` :273, `bound_0630_as` :279; `a_refusal_is_kept_for_the_record_and_taken_once` :473-480; `the_names_match_the_feed_generator` :484-491.
- `apps/node/src-tauri/src/state.rs`: `SETTINGS_FILE_NAME` :25; `update_high_water` :272-276; `Default` :299-345; `load` :348; `update` :471-478. `NodeAppSettings` is built literally only in `Default`.
- `apps/node/src-tauri/src/update_timer.rs`: `check_once` :140-194, `download_and_install` call :182, `settle` :200; the vocabulary test :308-322.
- `apps/node/src-tauri/src/update_log.rs`: `UPDATE_CHECK_OUTCOMES` :37-43; `DETAIL_MAX_CHARS` = 240 :56.
- `apps/node/src-tauri/src/commands.rs`: `record_update_check` :3818-3834; `refused_record` :3838-3850.
- `apps/node/src-tauri/src/lib.rs`: comparator wiring :31-33; `record_update_check` registered :68.
- `apps/node/src-tauri/tauri.conf.json`: endpoints :45-47; `tauri.linux.conf.json` and `tauri.windows.conf.json` carry no `plugins`.
- `apps/node/src-tauri/capabilities/default.json`: `updater:default`, which grants `allow-download` and `allow-install` (plugin `permissions/default.toml`).
- `apps/node/src/main.ts`: imports :22-30; `showUpdateBanner` :2107; `setUpdateResult` :2116; comment :2120-2128; `MANUAL_DOWNLOAD` :2129; `paintUpdateProgress` :2168; `onUpdateCheckEvent` :2195; `recordUpdateCheck` :2207; `updateCheck` :2215-2276, `downloadAndInstall` :2252.
- `apps/node/src/update-check.ts`: `VERSION_IN_DETAIL` :232; `plainOutcome` :237-255; `lastCheckLine` :262-266. Its test file: `read` :38, the structure pins :204-268 and :351-429.
- `apps/node/scripts/gen-node-feed.py`: `ASSET` :70-74; `_check_sig` :137; `build_feed` :183-208; `write_atomic` :211; `_named` :238; `self_test` :249-329; `main` :337-375, `missing` :366. `build-node-feed.sh`: args :61-81, existence :144, `expect_name` :182-195, signing :205-211, `ARGS` :214-220, summary :222-230. `publish-node-release.sh`: `.sig` rule :95-107. `test-publish-gate.sh`: fixtures :44-72. `verify-updater-sig.py` needs no change (it checks any artifact under its own name, :153-159).
- `.github/workflows/node-linux-installer.yml`: the CI artifact is already named `BTX-Node_<ver>_amd64.deb` and listed in `SHA256SUMS` (the rename and hash step).
- tauri-plugin-updater 2.11.0 (`Cargo.lock:3916-3917`), `src/updater.rs`: `{{bundle_type}}` and escaped substitution :459-487; 204 returns `Ok(None)` :531-534; a 2xx body that is not JSON returns an error at once, `res.json().await?` :536; a JSON body that is not a release sets `last_error` and tries the next endpoint :538-552; a non-success status logs and tries the next endpoint :554-558; comparator :576-579, then `get_urls` regardless :581-582; `get_urls` tries `{os}-{arch}-{installer}` then `{os}-{arch}` :608-640; `download` calls `on_download_finish` before `verify_signature` :738-740; `install` :751; `download_and_install` is `download` then `install` :761-769; Linux `install_inner` picks `install_deb` for `Deb` :1039-1045; `install_deb` rejects non-.deb bytes with `InvalidUpdaterFormat` :1120-1126; pkexec, then zenity/kdialog, then plain sudo :1177-1216; `Installer::name` :58-68; `installer_for_bundle_type` :1502-1512. `src/config.rs`: endpoints are `Vec<Url>` and must be https :114, :160-179.
- tauri-utils 2.9.3 (`Cargo.lock:4000-4001`), `src/platform.rs`: `bundle_type()` :353-370 (a Mac always `App`, an unstamped Linux binary `None`); `BundleType` derives `PartialEq` (`src/config.rs:129`).
- minisign-verify 0.2.5 `Signature::decode` parses the four lines without verifying them (`src/lib.rs:232-268`), which is why `sig_named` works for the name tests.
- EasyBTX `scripts/check-node-links.py`: paths :15-18; `fail` :23; `src` :27; `rel_lnx` :49; `signed_name` :175-190; `_fake_sig` :193-195; the `latest-node.json` loop :264-302 and the FEED_PIN advice :274-278; final output :428-439. `site/src/pages/node.astro`: `REL_LINUX` :128, `relPageLinux` :144, `dl.deb` :149, the `.deb` paragraph :556-559. `site/vercel.json`: every redirect and rewrite is host-scoped or a literal path. `site/astro.config.mjs`: `output: 'static'`; `site/src/pages/404.astro` exists. `.github/workflows/site-links.yml` runs `scripts/check-download-links.sh`, which runs `check-node-links.py`.
- Live, 2026-09-29: `node-{deb,appimage,app,nsis,msi,rpm,unknown}.json` all answer 404 on easybtx.com; `latest-node.json` answers 200.

## Self-review

**1. Spec coverage.** A second feed with one key: Tasks 9, 10, 13. Endpoints: Task 8. `update_binding.rs` learns the `.deb` suffix: Task 1. The comparator declines an AppImage-only release on a `.deb` copy and records the manual command: Task 3 (record path unchanged: timer via `settle` :158-170, front end via `refused_record`), shown in Tasks 6-7. An automatic check never re-downloads a failed version; kept in the settings; "Check now" still tries: Tasks 2, 3, 4, 7. The release scripts sign the `.deb` and write `node-deb.json`: Tasks 9, 10. `publish-node-release.sh` `.sig`: Task 11. Recipe: Task 12. Website file and checks, with sabotage cases: Task 13 (the file itself is release time, as the design's rollout says). 204 rule and "no other node-*.json": Tasks 13 and 14. Acceptance unit tests: binding offers a signed `.deb` feed and refuses the AppImage name (Task 1); guard declines on `.deb`, never on AppImage (Task 3); failed-version rule skips automatic and allows manual (Task 3, `check_now_clears_the_failed_version_so_it_tries_again`, and Task 7's order pin); `gen-node-feed.py` refuses the `.deb` key in the main feed (Task 9); `check-node-links.py` catches every sabotage (Task 13). The two end-to-end acceptance runs need real Ubuntu machines and are listed under "After both parts".

**2. Placeholder scan.** No "TBD", "TODO" or "similar to Task N"; every code step shows the code. The one stub (`return []` in Task 13 Step 2) is the deliberate red step of TDD and is replaced in Step 4.

**3. Type consistency.** `DEB_KEY`/`APPIMAGE_KEY` (Task 1) are used in Task 3; `failed_install`/`remember_failed_install`/`forget_failed_install` (Task 2) in Tasks 3, 4, 5; `ThisInstall { deb, failed }`, `decide_here`, `Decision::Decline`, `peek_refusal`, `HAND_INSTALL_MARK`, `DOWNLOADS_AT` (Task 3) in Tasks 5, 6; command names `remember_failed_update`, `forget_failed_update`, `peek_update_refusal` are identical in Task 5 (Rust and `lib.rs`) and Task 7 (`invoke` calls and the test); `handInstallNotice`, `handInstallBanner`, `noUpdateMessage` (Task 6) are the names imported in Task 7; `build_deb_feed`, `write_feed`, `feed_problem`, `MAIN_FEED_NAME`, `DEB_FEED_NAME` (Task 9) match their self-test; `--deb-sig` (Task 9) is what Task 10 passes; `deb_feed_problems`'s keyword arguments match between the self-check and the real call in Task 13.

## Concerns about the design, for the owner

1. **A 200 page is as fatal as a 204, and the design names only the 204.** In tauri-plugin-updater 2.11.0 a 2xx answer whose body is not JSON returns an error at once (`res.json().await?`, updater.rs:536) without trying `latest-node.json`, and a redirect is followed to wherever it lands. So from 0.6.33 every Mac, Windows and AppImage install depends on easybtx.com answering `node-app.json`, `node-nsis.json`, `node-appimage.json` and `node-unknown.json` with a real non-2xx, forever: a catch-all redirect, an SPA-style fallback or a CDN page served with 200 would stop all of their updates while 0.6.32 carried on. Tasks 13 and 14 guard the repository and the live host; the dependency itself remains.
2. **"Once its install has failed", read literally, would stop automatic retries after any failed download**, including a network drop, on every platform. Decision 2 narrows it to a verified download that failed to install.
3. **Without the banner in Decision 5, a `.deb` copy would go quiet.** Today the failed install at least shows a banner; with the guard nothing is downloaded, so nothing would be shown unless the owner opens Settings.
4. **"One password dialog"** holds for the pkexec path; a cancel brings a second dialog, and an app started from a terminal can wait on a terminal `sudo` prompt (updater.rs:1177-1216). Only the end-to-end test on a real desktop shows how that feels.
