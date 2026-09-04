//! First-run / launch plumbing shared by both apps: the RPC endpoint, disk
//! preflight, bootstrap-peer conf injection, and the cookie-based "wait until
//! the node's RPC answers" helper.

use crate::error::{AppError, AppResult};
use crate::rpc::RpcClient;
use std::path::Path;

/// BTX mainnet JSON-RPC endpoint. The port is 19334 (btx-main
/// chainparamsbase.cpp: main chain = CBaseChainParams("", 19334)) — NOT
/// Bitcoin's 8332. Using 8332 meant the attach-if-running probe could never
/// reach the live btxd, so the app spawned a SECOND daemon that collided on the
/// datadir lock and the cookie wait timed out.
pub const RPC_URL: &str = "http://127.0.0.1:19334";

/// The un-pruned chain's BLOCK PAYLOAD, measured 2026-09-04 and stated in GiB.
///
/// Method, because this number has been wrong in four places at once: BTX block
/// sizes are bimodal — a block is either ~367 bytes or ~1,049,000 bytes, the
/// large mode being the MatMul PoW payload rather than transaction traffic
/// (height 120000 is 1,048,948 bytes and carries one transaction). So the figure
/// comes from a stratified sample of real block sizes across the whole height
/// range, taken from an archival peer that was first cross-checked hash-for-hash
/// and byte-for-byte against this node for the heights this node still holds.
/// Three independent runs gave 123.5, 123.8 and 125.2 GiB; the large-block
/// region carried 245 samples with no exceptions. `docs/archival-capacity.md`
/// has the write-up and `scripts/measure-chain-size.py` re-runs it in minutes.
///
/// Re-measure it rather than adjusting it by feel. `disk_gate_covers_the_chain`
/// fails if the install gate is ever set below it.
pub const MEASURED_CHAIN_PAYLOAD_GIB: u64 = 124;

/// Free-disk thresholds for the first-run preflight. A FRESH install must
/// download + unpack the snapshot and write the chain + headers/index/overhead.
/// We run UN-PRUNED (prune=0 — see the faststart conf) so btxd keeps every
/// block (required for a restart-safe shielded-state rebuild).
///
/// 120 GiB was set from a ~105 GB reading taken 2026-07-12, and the chain has
/// since grown past it: the gate was BELOW the chain it exists to gate, so a
/// fresh install could pass the preflight and then run out of disk — the exact
/// failure the 18 GiB → 120 GiB change was made to prevent. 140 GiB covers the
/// measured 124 GiB plus the snapshot unpack, `debug.log`, and working room,
/// and it matches the "plan for 150 to 160 GB" this project already tells
/// people in `docs/always-on.md`.
///
/// The growth term that used to justify the headroom is gone: blocks left the
/// large mode at the fork around height 185,000, and since 2026-08-10 the
/// measured mean is 8.4 kB/block — about 8 MB/day, not the 1 GB/day this
/// comment claimed. The headroom is for the chain that exists, not for growth.
///
/// A RESUME only needs operating headroom.
pub const DISK_REQUIRED_FRESH: u64 = 140 * 1024 * 1024 * 1024; // measured chain (124 GiB) + working room
pub const DISK_REQUIRED_RESUME: u64 = 2 * 1024 * 1024 * 1024; // ~2 GiB

/// Whether `available` bytes meets `required`. Pure → unit-testable.
pub fn enough_free_disk(available: u64, required: u64) -> bool {
    available >= required
}

/// Free bytes on the volume holding `path`, via the platform syscall
/// (`statvfs` / `GetDiskFreeSpaceExW` — no `df` subprocess, so it measures on
/// Windows too, where the old `df -Pk` silently returned `None` and let the
/// 18 GiB preflight pass unchecked). `None` = "couldn't measure, don't block"
/// (the preflight is best-effort; a truly full disk fails the actual write with
/// a clear error anyway). Mirrors the `0 = not measured` convention of
/// [`crate::platform::free_disk_mb`].
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    match crate::platform::free_disk_bytes(path) {
        0 => None,
        n => Some(n),
    }
}

/// Idempotently append `addnode=<peer>` lines to `conf_path` for any peer not
/// already present. Creates the file if it doesn't exist.
///
/// This ensures a faststart-generated conf always contains bootstrap peers so
/// installer-started (non-NodeController) nodes also reach the network when DNS
/// seeds return no results.
pub fn ensure_addnodes_in_conf(conf_path: &Path, peers: &[&str]) -> AppResult<()> {
    // Read existing content (empty string if file doesn't exist yet).
    let existing = match std::fs::read_to_string(conf_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(AppError::Config(format!(
                "cannot read conf {}: {e}",
                conf_path.display()
            )))
        }
    };

    // Collect lines that are not already present.
    let mut to_append = String::new();
    for peer in peers {
        let line = format!("addnode={peer}");
        // Check for an exact line match (trim each line to be robust against
        // trailing whitespace / CRLF line endings).
        let already_present = existing.lines().any(|l| l.trim() == line.as_str());
        if !already_present {
            to_append.push_str(&line);
            to_append.push('\n');
        }
    }

    if to_append.is_empty() {
        return Ok(()); // Nothing to do — all peers already present.
    }

    // Append-only open: creates the file if absent, never truncates.
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(conf_path)
        .map_err(|e| AppError::Config(format!("cannot open conf {}: {e}", conf_path.display())))?;

    // Ensure we start on a fresh line if the existing file doesn't end with one.
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n").map_err(|e| {
            AppError::Config(format!("cannot write conf {}: {e}", conf_path.display()))
        })?;
    }
    file.write_all(to_append.as_bytes())
        .map_err(|e| AppError::Config(format!("cannot write conf {}: {e}", conf_path.display())))?;

    Ok(())
}

/// Remove `addnode=` lines whose peer is not in `keep`. Pure → testable.
///
/// WHY THIS EXISTS. `ensure_addnodes_in_conf` only ever APPENDS. The conf is
/// written once at setup and never reconciled, so the peer set btxd actually
/// dials is the UNION of every census we have ever shipped. Adding a seed
/// reaches existing installs; RETIRING one does not, and it never has.
///
/// Measured on a real install 2026-09-01, on 0.6.17, the release whose whole
/// point was the corrected census: `faststart/faststart.conf` still carried
/// `addnode=139.59.106.83:19335`, the seed three independent confirmations
/// caught serving stale branch headers and which we removed in 0.6.14 because
/// it can wedge a fresh header sync. It also still carried
/// `addnode=185.204.25.227:19335`, dropped in the same release. The app passed
/// the corrected census on the command line at the same time, and btxd honoured
/// both, so the bad seed was still dialled on every start.
///
/// This is the same shape as the bug that killed 0.6.15: a file persisted by an
/// older version silently outranking what the new one intends. The answer there
/// was to be explicit rather than to rely on a default, and it is the same here.
/// After this runs the conf's `addnode` set is exactly `keep`, so a seat we give
/// up is actually given up.
///
/// Only `addnode=` lines are touched. Comments, `# addnode=` lines, and every
/// other key are preserved byte for byte, as is the file's trailing newline.
pub fn prune_retired_addnodes_str(conf: &str, keep: &[&str]) -> String {
    let kept: Vec<&str> = conf
        .lines()
        .filter(|line| {
            let t = line.trim();
            match t.strip_prefix("addnode=") {
                // Not an addnode line: never our business.
                None => true,
                // An addnode line survives only if it names a peer we still ship.
                Some(peer) => keep.iter().any(|k| *k == peer.trim()),
            }
        })
        .collect();
    let mut out = kept.join("\n");
    if conf.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Rewrite `conf_path` so its `addnode` set is exactly `keep`. Returns how many
/// lines were removed, 0 when the file was already correct or unreadable.
///
/// Call it AFTER `ensure_addnodes_in_conf` has added the current census, so the
/// two together converge the file on exactly what this build ships. A failure to
/// write is not fatal: a stale seed is a degraded peer set, not a broken node,
/// and refusing to start over it would be worse than the problem.
pub fn prune_retired_addnodes_in_conf(conf_path: &Path, keep: &[&str]) -> usize {
    let Ok(original) = std::fs::read_to_string(conf_path) else {
        return 0;
    };
    let rewritten = prune_retired_addnodes_str(&original, keep);
    if rewritten == original {
        return 0;
    }
    let removed = original
        .lines()
        .count()
        .saturating_sub(rewritten.lines().count());
    if std::fs::write(conf_path, rewritten).is_ok() {
        removed
    } else {
        0
    }
}

/// Managed-block markers for the archive noban whitelist. Everything between
/// them is OWNED by the app and rewritten wholesale on every start.
pub const WHITELIST_BLOCK_BEGIN: &str =
    "# BEGIN easybtx-managed archive whitelist (rewritten on every start; edits inside are lost)";
pub const WHITELIST_BLOCK_END: &str = "# END easybtx-managed archive whitelist";

/// Rewrite the managed archive-whitelist block in `conf_path` to exactly
/// `whitelist=in,out,noban@<ip>` for `ips`, preserving every line outside the
/// block.
///
/// On a trusted mirror, `noban` is one arm of the attestation/download
/// authority gate (the other is `addnode`); the `in,out` direction flags are
/// REQUIRED because bare `-whitelist` applies to incoming connections only
/// and the connection addnode creates is outgoing (PR #105 runbook,
/// issuecomment-5309870607).
///
/// This replaces the 0.6.7-candidate `ensure_whitelist_in_conf`, which was
/// append-only: `noban` is a SECURITY GRANT (ban-immunity + preferred-download
/// + attestation authority), and append-only made it an irrevocable ratchet
/// on frozen IPs — an address that left the archive census (DNS rotation,
/// provider churn, a compromised host) kept its blessing forever on every
/// install. With the managed block, the grant tracks the shipped constants +
/// live DNS exactly; removing an IP upstream revokes it on the next start.
///
/// Migration/dedupe: bare `whitelist=in,out,noban@<ip>` lines OUTSIDE the
/// block are claimed into it when their ip is one we are asserting anyway
/// (what the append-only version wrote); every other whitelist line is the
/// operator's and survives untouched. An unterminated block (BEGIN without
/// END — a truncated write) is treated as ours to the end of file.
pub fn set_managed_whitelist_block(conf_path: &Path, ips: &[String]) -> AppResult<()> {
    let existing = match std::fs::read_to_string(conf_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(AppError::Config(format!(
                "cannot read conf {}: {e}",
                conf_path.display()
            )))
        }
    };

    let mut out: Vec<String> = Vec::new();
    let mut in_block = false;
    for l in existing.lines() {
        let t = l.trim();
        if t == WHITELIST_BLOCK_BEGIN {
            in_block = true;
            continue;
        }
        if t == WHITELIST_BLOCK_END {
            in_block = false;
            continue;
        }
        if in_block {
            continue; // owned content: dropped here, rewritten below
        }
        if let Some(ip) = t.strip_prefix("whitelist=in,out,noban@") {
            if ips.iter().any(|i| i == ip) {
                continue; // claimed into the managed block (dedupe)
            }
        }
        out.push(l.to_string());
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if !out.is_empty() {
        out.push(String::new()); // one blank line before the block
    }
    out.push(WHITELIST_BLOCK_BEGIN.to_string());
    for ip in ips {
        out.push(format!("whitelist=in,out,noban@{ip}"));
    }
    out.push(WHITELIST_BLOCK_END.to_string());

    let mut content = out.join("\n");
    content.push('\n');
    std::fs::write(conf_path, content)
        .map_err(|e| AppError::Config(format!("cannot write conf {}: {e}", conf_path.display())))
}

/// Read the value of the first `key=value` line in a conf file (`None` when
/// the file or the key is absent). Companion to [`set_conf_kv`] — lets the
/// start path ADOPT a hand-set flag instead of overwriting it.
pub fn conf_kv(conf_path: &Path, key: &str) -> Option<String> {
    let existing = std::fs::read_to_string(conf_path).ok()?;
    let prefix = format!("{key}=");
    existing
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

/// Set or clear ONE `key=value` line in a conf file, preserving every other
/// line (comments included). `Some(value)` upserts `key=value`; `None` removes
/// all `key=…` lines. Unlike `ensure_addnodes_in_conf` (append-only), this
/// rewrites the file — removal needs it. Explorer mode uses it for `txindex=1`.
pub fn set_conf_kv(conf_path: &Path, key: &str, value: Option<&str>) -> AppResult<()> {
    let existing = match std::fs::read_to_string(conf_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if value.is_none() {
                return Ok(()); // clearing a key from a missing file is a no-op
            }
            String::new()
        }
        Err(e) => {
            return Err(AppError::Config(format!(
                "cannot read conf {}: {e}",
                conf_path.display()
            )))
        }
    };

    let prefix = format!("{key}=");
    let mut out: Vec<&str> = existing
        .lines()
        .filter(|l| !l.trim().starts_with(&prefix))
        .collect();
    let line;
    if let Some(v) = value {
        line = format!("{key}={v}");
        out.push(&line);
    }
    let mut content = out.join("\n");
    content.push('\n');
    std::fs::write(conf_path, content)
        .map_err(|e| AppError::Config(format!("cannot write conf {}: {e}", conf_path.display())))
}

/// Wait for a freshly-spawned btxd's RPC to become usable: poll for the
/// `.cookie` file (btxd writes it only once its RPC server is ready), build a
/// client from it, and confirm `getblockchaininfo` answers.
///
/// TWO separate budgets, mirroring the miner's startup wait: a node that never
/// answers at all is bounded by `max_polls` (a dead node should fail fast),
/// but a node answering RPC_IN_WARMUP (-28: verifying blocks / rebuilding
/// shielded state — an unclean shutdown's rebuild can run ~8+ minutes) is
/// ALIVE and gets the much larger `warmup_max_polls` budget. Timing a healthy
/// warming node out (and then respawning over it) recreates the exact
/// unclean-shutdown corruption the graceful-stop path exists to avoid.
///
/// The cookie is re-read on every attempt: btxd regenerates it per start, so a
/// client built from a stale cookie would 401 forever.
pub async fn wait_for_node_rpc(
    datadir: &Path,
    url: &str,
    max_polls: u32,
    poll_ms: u64,
    warmup_max_polls: u32,
) -> Result<RpcClient, String> {
    let cookie = datadir.join(".cookie");
    let mut last = String::from("the node's RPC never became reachable (no .cookie yet)");
    let mut unreachable_polls: u32 = 0;
    let mut warmup_polls: u32 = 0;
    while unreachable_polls < max_polls && warmup_polls < warmup_max_polls {
        if let Ok(client) = RpcClient::from_cookie(url, &cookie) {
            match crate::node_api::get_blockchain_info(&client).await {
                Ok(_) => return Ok(client),
                Err(AppError::Rpc { code: -28, message }) => {
                    last = format!("node is warming up: {message}");
                    warmup_polls += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
                    continue;
                }
                Err(e) => {
                    last = e.to_string();
                }
            }
        }
        unreachable_polls += 1;
        tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
    }
    Err(last)
}

#[cfg(test)]
mod tests {

    // ---- retired addnode pruning -------------------------------------------

    #[test]
    fn prune_drops_a_retired_seed_and_keeps_the_shipped_ones() {
        // The real 2026-09-01 case: 0.6.17 shipped a corrected census while the
        // conf still carried the stale branch seed 0.6.14 removed.
        let conf = "server=1\nlisten=1\naddnode=139.59.106.83:19335\naddnode=207.56.229.99:19335\nprune=0\n";
        let keep = ["207.56.229.99:19335"];
        let out = super::prune_retired_addnodes_str(conf, &keep);
        assert!(
            !out.contains("139.59.106.83"),
            "the retired seed must be gone"
        );
        assert!(
            out.contains("addnode=207.56.229.99:19335"),
            "a shipped seed stays"
        );
        assert!(
            out.contains("server=1") && out.contains("listen=1") && out.contains("prune=0"),
            "every non-addnode line is preserved"
        );
    }

    #[test]
    fn prune_is_a_no_op_when_the_conf_is_already_correct() {
        let conf = "server=1\naddnode=a:1\naddnode=b:2\n";
        let keep = ["a:1", "b:2"];
        assert_eq!(
            super::prune_retired_addnodes_str(conf, &keep),
            conf,
            "an already-correct conf must come back byte identical"
        );
    }

    #[test]
    fn prune_never_touches_a_commented_out_addnode() {
        // A commented seed is a human's note. Removing it silently rewrites
        // somebody's reasoning out of their own file.
        let conf = "# addnode=1.2.3.4:19335 banned by jpp, kept for the record\naddnode=a:1\n";
        let out = super::prune_retired_addnodes_str(conf, &["a:1"]);
        assert!(
            out.contains("# addnode=1.2.3.4:19335 banned by jpp"),
            "a comment is not configuration"
        );
    }

    #[test]
    fn prune_preserves_the_trailing_newline_either_way() {
        assert!(
            super::prune_retired_addnodes_str("addnode=a:1\naddnode=b:2\n", &["a:1"])
                .ends_with('\n')
        );
        assert!(
            !super::prune_retired_addnodes_str("addnode=a:1\naddnode=b:2", &["a:1"])
                .ends_with('\n')
        );
    }

    #[test]
    fn prune_tolerates_whitespace_and_crlf_the_way_ensure_does() {
        let conf = "  addnode=a:1  \r\naddnode=b:2\r\n";
        let out = super::prune_retired_addnodes_str(conf, &["a:1"]);
        assert!(
            out.contains("addnode=a:1"),
            "a padded line still matches its peer"
        );
        assert!(
            !out.contains("b:2"),
            "and a padded retired line is still dropped"
        );
    }

    #[test]
    fn prune_with_an_empty_keep_list_removes_every_addnode() {
        let out = super::prune_retired_addnodes_str("server=1\naddnode=a:1\naddnode=b:2\n", &[]);
        assert_eq!(out, "server=1\n");
    }

    #[test]
    fn ensure_then_prune_converges_the_conf_on_exactly_the_shipped_census() {
        // The pair is the contract: append what is missing, drop what is retired.
        let dir = std::env::temp_dir().join(format!("ebtx-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("faststart.conf");
        std::fs::write(&conf, "server=1\naddnode=retired:19335\n").unwrap();
        let census = ["kept-a:19335", "kept-b:19335"];
        super::ensure_addnodes_in_conf(&conf, &census).unwrap();
        let removed = super::prune_retired_addnodes_in_conf(&conf, &census);
        let out = std::fs::read_to_string(&conf).unwrap();
        assert_eq!(removed, 1);
        assert!(!out.contains("retired:19335"));
        assert!(out.contains("addnode=kept-a:19335") && out.contains("addnode=kept-b:19335"));
        assert!(out.contains("server=1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_on_a_missing_file_reports_nothing_removed_rather_than_failing() {
        // Startup must never be blocked by this. A stale seed is a degraded peer
        // set, not a broken node.
        let missing = std::env::temp_dir().join("ebtx-prune-does-not-exist.conf");
        std::fs::remove_file(&missing).ok();
        assert_eq!(super::prune_retired_addnodes_in_conf(&missing, &["a:1"]), 0);
    }

    use super::*;

    fn ips(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn managed_whitelist_block_writes_idempotently_and_preserves_content() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("faststart.conf");
        std::fs::write(&conf, "# preset: node\nserver=1\naddnode=1.2.3.4:19335\n").unwrap();
        set_managed_whitelist_block(&conf, &ips(&["207.56.229.99", "185.204.25.227"])).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert!(s
            .lines()
            .any(|l| l == "whitelist=in,out,noban@207.56.229.99"));
        assert!(s
            .lines()
            .any(|l| l == "whitelist=in,out,noban@185.204.25.227"));
        assert!(s.contains("# preset: node"), "comments survive");
        assert!(s.contains("addnode=1.2.3.4:19335"), "other lines survive");
        // Idempotent: second call produces identical bytes.
        set_managed_whitelist_block(&conf, &ips(&["207.56.229.99", "185.204.25.227"])).unwrap();
        let s2 = std::fs::read_to_string(&conf).unwrap();
        assert_eq!(s2.matches("207.56.229.99").count(), 1);
        assert_eq!(s2, s, "second call is a no-op");
        // Creates a missing file.
        let fresh = tmp.path().join("fresh.conf");
        set_managed_whitelist_block(&fresh, &ips(&["9.9.9.9"])).unwrap();
        let f = std::fs::read_to_string(&fresh).unwrap();
        assert!(f.lines().any(|l| l == "whitelist=in,out,noban@9.9.9.9"));
        assert!(f.starts_with(WHITELIST_BLOCK_BEGIN));
    }

    /// THE POINT of the managed block: dropping an IP from the list revokes
    /// its noban grant on the next write — append-only could never do this.
    #[test]
    fn managed_whitelist_block_revokes_removed_ips() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("faststart.conf");
        set_managed_whitelist_block(&conf, &ips(&["1.1.1.1", "2.2.2.2"])).unwrap();
        set_managed_whitelist_block(&conf, &ips(&["1.1.1.1"])).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert!(s.contains("noban@1.1.1.1"));
        assert!(
            !s.contains("2.2.2.2"),
            "an IP removed from the census must lose its grant: {s}"
        );
    }

    /// Migration from the append-only 0.6.7 candidate: its bare lines are
    /// claimed into the block when we assert the same ip (no duplicates), and
    /// the OPERATOR's own whitelist lines survive untouched.
    #[test]
    fn managed_whitelist_block_claims_our_legacy_lines_but_not_operator_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("faststart.conf");
        std::fs::write(
            &conf,
            "server=1\n\
             whitelist=in,out,noban@207.56.229.99\n\
             whitelist=noban@10.0.0.7\n\
             whitelist=in,out,noban@192.168.1.50\n",
        )
        .unwrap();
        set_managed_whitelist_block(&conf, &ips(&["207.56.229.99"])).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert_eq!(
            s.matches("207.56.229.99").count(),
            1,
            "legacy line for an asserted ip is deduped into the block: {s}"
        );
        assert!(
            s.contains("whitelist=noban@10.0.0.7"),
            "operator's own (different-format) line survives"
        );
        assert!(
            s.contains("whitelist=in,out,noban@192.168.1.50"),
            "same-format line for an ip we are NOT asserting is the operator's"
        );
    }

    #[test]
    fn conf_kv_reads_first_value_or_none() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("faststart.conf");
        std::fs::write(&conf, "server=1\nmatmulattestationserve=1\n").unwrap();
        assert_eq!(
            conf_kv(&conf, "matmulattestationserve").as_deref(),
            Some("1")
        );
        assert_eq!(conf_kv(&conf, "server").as_deref(), Some("1"));
        assert_eq!(conf_kv(&conf, "txindex"), None);
        assert_eq!(conf_kv(&tmp.path().join("absent.conf"), "server"), None);
    }

    #[test]
    fn set_conf_kv_upserts_removes_and_preserves_other_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("faststart.conf");
        std::fs::write(&conf, "# preset: node\nserver=1\nprune=0\n").unwrap();
        // Insert.
        set_conf_kv(&conf, "txindex", Some("1")).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert!(s.lines().any(|l| l == "txindex=1"));
        assert!(s.contains("# preset: node"), "comments survive");
        assert!(s.contains("server=1"));
        // Idempotent upsert (no duplicates).
        set_conf_kv(&conf, "txindex", Some("1")).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert_eq!(s.matches("txindex=1").count(), 1);
        // Remove.
        set_conf_kv(&conf, "txindex", None).unwrap();
        let s = std::fs::read_to_string(&conf).unwrap();
        assert!(!s.contains("txindex"));
        assert!(s.contains("prune=0"), "unrelated keys survive removal");
        // Removing from a missing file is a no-op, not an error.
        set_conf_kv(&tmp.path().join("absent.conf"), "txindex", None).unwrap();
        // Setting into a missing file creates it.
        let fresh = tmp.path().join("fresh.conf");
        set_conf_kv(&fresh, "txindex", Some("1")).unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "txindex=1\n");
    }

    /// The gate exists to refuse an install that could never finish. If it is
    /// ever set below the chain it is gating, it does the opposite: it waves
    /// through the install that runs out of disk halfway. That is what happened
    /// between 2026-07-12 and 2026-09-04, silently, because nothing checked.
    #[test]
    fn disk_gate_covers_the_chain() {
        let gib = 1024 * 1024 * 1024;
        assert!(
            DISK_REQUIRED_FRESH >= MEASURED_CHAIN_PAYLOAD_GIB * gib,
            "fresh-install gate is {} GiB but the measured chain is {} GiB: re-measure with scripts/measure-chain-size.py, then raise the gate",
            DISK_REQUIRED_FRESH / gib,
            MEASURED_CHAIN_PAYLOAD_GIB
        );
    }

    #[test]
    fn disk_preflight_gates_on_threshold() {
        let gb = 1024 * 1024 * 1024;
        // A comfortable disk passes the fresh-install gate.
        assert!(enough_free_disk(200 * gb, DISK_REQUIRED_FRESH));
        // A disk that could not hold the full chain fails the gate.
        assert!(!enough_free_disk(40 * gb, DISK_REQUIRED_FRESH));
        // Exactly the requirement is acceptable.
        assert!(enough_free_disk(DISK_REQUIRED_FRESH, DISK_REQUIRED_FRESH));
        // Resume needs only headroom.
        assert!(enough_free_disk(3 * gb, DISK_REQUIRED_RESUME));
        assert!(!enough_free_disk(gb, DISK_REQUIRED_RESUME));
    }

    #[test]
    fn free_disk_bytes_reports_on_a_real_path() {
        // Environment-dependent value, but on any healthy dev/CI box the temp
        // dir's volume has SOME free space and df parses.
        let free = free_disk_bytes(&std::env::temp_dir());
        assert!(free.unwrap_or(0) > 0, "df -Pk should report free space");
    }

    // ── ensure_addnodes_in_conf tests ───────────────────────────────────────

    #[test]
    fn addnodes_appended_to_empty_conf() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join("btx.conf");

        let peers = &["1.2.3.4:19335", "5.6.7.8:19335"];
        ensure_addnodes_in_conf(&conf, peers).expect("should succeed");

        let content = std::fs::read_to_string(&conf).expect("file must exist after call");
        assert!(content.contains("addnode=1.2.3.4:19335\n"));
        assert!(content.contains("addnode=5.6.7.8:19335\n"));
    }

    #[test]
    fn addnodes_not_duplicated_when_already_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join("btx.conf");

        // Pre-populate the conf with one peer.
        std::fs::write(&conf, "addnode=1.2.3.4:19335\n").expect("write");

        let peers = &["1.2.3.4:19335", "5.6.7.8:19335"];
        ensure_addnodes_in_conf(&conf, peers).expect("should succeed");

        let content = std::fs::read_to_string(&conf).expect("file must exist");
        // The first peer must appear exactly once.
        let count = content.matches("addnode=1.2.3.4:19335").count();
        assert_eq!(
            count, 1,
            "peer must not be duplicated; content: {content:?}"
        );
        // The second peer must have been appended.
        assert!(content.contains("addnode=5.6.7.8:19335"));
    }

    #[test]
    fn addnodes_idempotent_on_second_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join("btx.conf");

        let peers = &["1.2.3.4:19335"];
        ensure_addnodes_in_conf(&conf, peers).expect("first call");
        ensure_addnodes_in_conf(&conf, peers).expect("second call");

        let content = std::fs::read_to_string(&conf).expect("file must exist");
        let count = content.matches("addnode=1.2.3.4:19335").count();
        assert_eq!(
            count, 1,
            "idempotent second call must not duplicate; content: {content:?}"
        );
    }

    #[test]
    fn addnodes_appended_after_existing_non_addnode_conf() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf = dir.path().join("btx.conf");

        std::fs::write(&conf, "server=1\nrpcbind=127.0.0.1\n").expect("write");

        let peers = &["1.2.3.4:19335"];
        ensure_addnodes_in_conf(&conf, peers).expect("should succeed");

        let content = std::fs::read_to_string(&conf).expect("file must exist");
        assert!(content.starts_with("server=1\n"));
        assert!(content.contains("addnode=1.2.3.4:19335"));
    }

    // ── wait_for_node_rpc ───────────────────────────────────────────────────

    #[tokio::test]
    async fn wait_for_node_rpc_succeeds_once_cookie_and_rpc_answer() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"result":{"blocks":10,"headers":10,"verificationprogress":1.0,"initialblockdownload":false},"error":null,"id":"easybtx"}"#,
            )
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".cookie"), "__cookie__:secret").unwrap();

        let client = wait_for_node_rpc(dir.path(), &server.url(), 5, 10, 100).await;
        assert!(client.is_ok(), "expected Ok, got {:?}", client.err());
    }

    #[tokio::test]
    async fn wait_for_node_rpc_times_out_without_cookie() {
        let dir = tempfile::tempdir().unwrap();
        // No .cookie ever appears → bounded failure with an actionable message.
        // No unwrap_err(): RpcClient deliberately has no Debug impl (it would
        // print the RPC credentials), so destructure by hand.
        let err = match wait_for_node_rpc(dir.path(), "http://127.0.0.1:1", 3, 10, 100).await {
            Err(e) => e,
            Ok(_) => panic!("expected a timeout error"),
        };
        assert!(
            err.contains("cookie") || err.contains("reachable"),
            "timeout error should explain the cookie never appeared, got: {err}"
        );
    }

    #[tokio::test]
    async fn wait_for_node_rpc_keeps_waiting_through_warmup_then_gives_last_error() {
        // A node stuck in RPC_IN_WARMUP for the whole budget must time out with
        // the WARMUP message (proving -28 was recognized as "alive"), not a
        // generic unreachable error.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"result":null,"error":{"code":-28,"message":"Verifying blocks…"},"id":"easybtx"}"#,
            )
            .expect_at_least(2)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".cookie"), "__cookie__:secret").unwrap();

        // Warmup gets its OWN budget: give a tiny unreachable budget (1) but a
        // slightly larger warmup budget (4); the -28 responses must consume the
        // warmup budget (proving they were recognized as "alive"), and the
        // timeout must surface the warmup message.
        let err = match wait_for_node_rpc(dir.path(), &server.url(), 1, 10, 4).await {
            Err(e) => e,
            Ok(_) => panic!("expected the warmup budget to expire"),
        };
        assert!(
            err.contains("warming up"),
            "expected the warmup message to surface, got: {err}"
        );
    }
}
