//! Assumeutxo snapshot handling: pinned download + SHA-256 verification, and the
//! background `loadtxoutset` guarantee.
//!
//! Extracted from the miner's `commands.rs` (behavior unchanged), with two
//! seams so BOTH apps can drive it:
//!   * progress is reported through a plain callback instead of the miner's
//!     `AppPhase::Installing` writes;
//!   * the persisted "the snapshot has genuinely been loaded" flag goes through
//!     the [`SnapshotFlags`] trait (the miner backs it with `EasyBtxState`;
//!     easyBTX Node backs it with its own settings file). C3 still holds: the
//!     flag must ONLY flip after a confirmed successful `loadtxoutset`, because
//!     `disk::reclaim_disk` deletes `snapshot.dat` once it is true.

use crate::node_api::{get_blockchain_info, get_chainstates};
use crate::rpc::RpcClient;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Everything that pins one release's assumeutxo snapshot asset. Values mirror
/// the release's `snapshot.manifest.json` (height, sha, size).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotSpec {
    /// Release asset URL of `snapshot.dat`.
    pub url: String,
    /// Hex SHA-256 of the asset, verified before the (expensive) loadtxoutset.
    pub sha256: String,
    /// Exact asset size in bytes (drives the skip-download idempotency check).
    pub size_bytes: u64,
    /// Snapshot base height (`m_assumeutxo_data`): the node's HEADERS must reach
    /// this before `loadtxoutset` accepts the snapshot.
    pub anchor_height: u64,
}

/// The v0.33.2 release snapshot (height 179000, snapshot_file_version 9),
/// pinned from that release's own `snapshot.manifest.json` (fetched + verified
/// 2026-08-10, sha cross-checked against the release `SHA256SUMS`). **Superseded
/// by [`v0_34_5_spec`] as the node app pin in 0.6.13**; kept as a pinned
/// fallback. v0.33.2 is the MatMul v4.7 release, so the pin
/// had to move with it (upstream regenerated `snapshot.dat`: different sha,
/// height and size from v0.33.1, so the old pin's SHA gate would refuse this
/// asset and vice versa).
///
/// The anchor jumping 155700 → 179000 is a straight win for faststart: 23300
/// fewer blocks to backfill after `loadtxoutset`.
///
/// SHA-256 verified at download AND btxd's assumeutxo commitment verifies it
/// again at load time.
pub fn v0_33_2_spec() -> SnapshotSpec {
    SnapshotSpec {
        url: "https://github.com/btxchain/btx/releases/download/v0.33.2/snapshot.dat".into(),
        sha256: "2bc308713cb8c083698ca228de1ecf7a8c2800dd09894cac6c0f9dff22d7f494".into(),
        size_bytes: 452_282_113,
        anchor_height: 179_000,
    }
}

/// The v0.34.5 release snapshot (height 203000, snapshot_file_version 9),
/// pinned from that release's own `snapshot-manifest-203000.json`.
///
/// **This is the node app's pin from 0.6.13.** It replaces [`v0_33_2_spec`] and
/// it is the single biggest change to how long a new user waits. The 179000 pin
/// left about 26860 blocks to grind after the load; this leaves about 2860.
///
/// ⚠ That is a smaller GAP, not a promise of convergence. Measured 2026-08-31 on
/// this Mac after a real load: the network produced about 61.8 blocks/hour over
/// the preceding 47 hours while the Mac validated the recent chain at about 60
/// blocks/hour. Those are close enough that the last stretch can take days and on
/// a poorly peered host may not close at all. Do not derive a duration from this
/// pin and do not put one in user copy.
///
/// The asset is 9.3 MB rather than 452 MB because upstream publishes this one as
/// a `"snapshot_type": "rollback"` UTXO set rather than a full one.
///
/// Verified 2026-08-31 by downloading the asset: the size matched the manifest,
/// the SHA-256 matched the manifest AND the release `SHA256SUMS`, and a real
/// `loadtxoutset` on a v0.34.5 node returned `base_height` 203000 with
/// `tip_hash` 89cfe990...f87999ef, matching the manifest `blockhash` exactly.
///
/// ⚠ This pin REQUIRES an engine of v0.34.5 or newer. Height 203000 is in
/// v0.34.5's compiled `m_assumeutxo_data`; it is NOT in v0.33.4.x, so pairing
/// this spec with an older engine makes `loadtxoutset` refuse and turns a fast
/// first run into a sync from genesis. `NODE_RELEASE_TAG` and this spec move
/// together or not at all.
pub fn v0_34_5_spec() -> SnapshotSpec {
    SnapshotSpec {
        url: "https://github.com/btxchain/btx/releases/download/v0.34.5/utxo-btx-main-203000.dat"
            .into(),
        sha256: "ae86589a2516fbceffe959c068b4c8aa2d25749f75198231b8b90abc8a683679".into(),
        size_bytes: 9_312_020,
        anchor_height: 203_000,
    }
}

/// The v0.33.1 release snapshot (height 155700, snapshot_file_version 9),
/// pinned from that release's own `snapshot.manifest.json` (fetched + verified
/// 2026-07-11). Superseded by [`v0_33_2_spec`] as the node app's pin when the
/// MatMul v4.7 fork forced the btxd bump; kept for reference / a pinned
/// fallback. v0.33.1 ships the SAME snapshot bytes as v0.33.0 (identical
/// sha/height/size — it's a wallet-interop point release); the URL points at
/// v0.33.1's own asset for lifecycle safety. SHA-256 verified at download AND
/// btxd's assumeutxo commitment verifies it again at load time.
pub fn v0_33_1_spec() -> SnapshotSpec {
    SnapshotSpec {
        url: "https://github.com/btxchain/btx/releases/download/v0.33.1/snapshot.dat".into(),
        sha256: "e0fb6d34852a7f0ac649dfaa9e4a50a1fa5bcde7ba97475ef3bf62f4175fc69e".into(),
        size_bytes: 448_392_435,
        anchor_height: 155_700,
    }
}

/// The v0.32.12 release snapshot (height 132209, snapshot_file_version 9),
/// pinned from that release's own `snapshot.manifest.json` (fetched + verified
/// 2026-07-11). Superseded by [`v0_33_0_spec`] as the node app's pin; kept for
/// reference / a pinned fallback. SHA-256 verified at download AND btxd's
/// built-in assumeutxo commitment verifies it again at load time.
pub fn v0_32_12_spec() -> SnapshotSpec {
    SnapshotSpec {
        url: "https://github.com/btxchain/btx/releases/download/v0.32.12/snapshot.dat".into(),
        sha256: "4555b41740e9c3ea8a4dc79a14b545cd210c076a36361e06d2f5858ae50bcc4e".into(),
        size_bytes: 446_967_831,
        anchor_height: 132_209,
    }
}

/// The miner's historical pin: the v0.32.11 release snapshot as it existed
/// when the miner shipped (height 130656). ⚠ KNOWN-STALE as of 2026-07-11: a
/// live download check showed upstream REGENERATED the v0.32.11 asset (its
/// manifest now reports sha 28edf8af…, height 130501), so this pin's SHA gate
/// would refuse today's asset. Kept verbatim because the miner's Solo path is
/// retired (should_launch_node() is always false) and this PR must not change
/// miner behavior; any Solo revival must re-pin — prefer [`v0_32_12_spec`].
pub fn v0_32_11_spec() -> SnapshotSpec {
    SnapshotSpec {
        url: "https://github.com/btxchain/btx/releases/download/v0.32.11/snapshot.dat".into(),
        sha256: "eae763b45075b6a525fe65cd107364da4545aa25837755eeeafdb067152fc7dc".into(),
        size_bytes: 446_222_643,
        anchor_height: 130_656,
    }
}

/// Whether the `snapshot.dat` currently on disk is the one `spec` pins.
///
/// Exists because an UPGRADE moves [`SnapshotSpec`] but never re-enters the
/// first-run setup wizard, which is the only caller of [`download_snapshot`]. So
/// after a release that repins the snapshot, an existing install still has the
/// PREVIOUS release's file sitting in `faststart/`, while the anchor height the
/// loader waits for has already moved to the new one.
///
/// Measured 2026-08-31 upgrading a live 0.6.12 install to 0.6.13: the 452 MB
/// height-179000 asset was still on disk and untouched after the upgrade, while
/// the anchor had moved to 203000. Left alone that is STRICTLY WORSE than not
/// repinning at all, because the user waits for headers to reach 203000 and then
/// loads a snapshot that only takes them to 179000.
///
/// Same size-then-SHA order as the download idempotency check: cheap gate first,
/// and a file of the right size is still verified by content.
pub fn snapshot_file_matches_spec(spec: &SnapshotSpec, datadir: &Path) -> bool {
    let path = datadir.join("faststart").join("snapshot.dat");
    match std::fs::metadata(&path) {
        Ok(m) if m.len() == spec.size_bytes => {
            matches!(verify_file_sha256(&path, &spec.sha256), Ok(true))
        }
        _ => false,
    }
}

/// True when a `snapshot.dat` exists but is NOT the one `spec` pins, i.e. the
/// pin moved under an existing install and the file must be refreshed before
/// [`ensure_snapshot_loaded`] reads it.
pub fn snapshot_file_is_stale_for_spec(spec: &SnapshotSpec, datadir: &Path) -> bool {
    datadir.join("faststart").join("snapshot.dat").exists()
        && !snapshot_file_matches_spec(spec, datadir)
}

/// Whether the node's header chain has reached the snapshot anchor height.
/// assumeutxo refuses `loadtxoutset` until headers reach the snapshot base, so
/// this is the precondition the faststart installer enforces. Pure + testable.
pub fn snapshot_anchor_reached(headers: u64, anchor_height: u64) -> bool {
    headers >= anchor_height
}

/// A SHARED, cross-process "the snapshot.dat currently on disk has been loaded"
/// marker: `<datadir>/faststart/.snapshot-loaded`.
///
/// C3 background: `disk::reclaim_disk` may delete `faststart/snapshot.dat`, but
/// the miner and easyBTX Node each carry their OWN per-app `snapshot_loaded`
/// boolean (EasyBtxState vs NodeAppSettings) over the SAME shared datadir. A
/// stale per-app flag (e.g. the miner's from a historical load) could let one
/// app's reclaim delete a snapshot the OTHER app just downloaded and is still
/// loading. This marker is ONE fact both apps write and read, so the delete
/// decision is authoritative across processes:
///   * written when EITHER app confirms a load (see `ensure_snapshot_loaded`),
///   * CLEARED whenever a fresh `snapshot.dat` is downloaded (the new file is
///     not yet loaded — see `download_snapshot`),
///   * required (in addition to the per-app flag) before reclaim deletes
///     `snapshot.dat` (see `disk::reclaim_disk`).
pub fn snapshot_marker_path(datadir: &Path) -> PathBuf {
    datadir.join("faststart").join(".snapshot-loaded")
}

/// True if the shared cross-process "loaded" marker is present.
pub fn snapshot_marker_present(datadir: &Path) -> bool {
    snapshot_marker_path(datadir).exists()
}

/// Write the shared "loaded" marker (best-effort; a failure only means reclaim
/// keeps `snapshot.dat` one session longer, never data loss). Creates the
/// `faststart/` dir if needed.
pub fn mark_snapshot_marker(datadir: &Path) {
    let path = snapshot_marker_path(datadir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, b"loaded\n") {
        eprintln!("[snapshot] could not write shared loaded-marker (non-fatal): {e}");
    }
}

/// Remove the shared "loaded" marker (best-effort). Called when a FRESH
/// snapshot.dat lands, since the new file has not been loaded yet.
pub fn clear_snapshot_marker(datadir: &Path) {
    let _ = std::fs::remove_file(snapshot_marker_path(datadir));
}

/// SHA-256 verify a file on disk against a hex-encoded expected digest.
/// Returns `Ok(true)` if the file's SHA-256 matches `expected_hex`, `Ok(false)`
/// on mismatch, or `Err(io_msg)` if the file can't be read. Used by the snapshot
/// idempotency check (M13) so a wrong-content file of the expected size cannot
/// satisfy the skip-download fast path.
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(hex.eq_ignore_ascii_case(expected_hex))
}

/// Download the pinned assumeutxo snapshot to `<datadir>/faststart/snapshot.dat`,
/// reporting download progress (0.0..=1.0) through `progress`, verifying its
/// SHA-256 before accepting it. Skips the download when a correctly-sized,
/// correct-SHA file is already present. Network failures surface a plain-language,
/// actionable message (shown verbatim to the user).
pub async fn download_snapshot(
    spec: &SnapshotSpec,
    datadir: &Path,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let faststart_dir = datadir.join("faststart");
    let dest = faststart_dir.join("snapshot.dat");
    // M13: size-only idempotency is a false-equivalence. A truncated half-mirror
    // or a tampered file can land on disk at exactly the expected size and the
    // old check would accept it without ever verifying the contents — passing a
    // bogus snapshot.dat straight to `loadtxoutset`. When the size matches, also
    // verify SHA-256. Match → genuine skip. Mismatch → log, delete, re-download.
    if std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == spec.size_bytes {
        match verify_file_sha256(&dest, &spec.sha256) {
            Ok(true) => {
                eprintln!(
                    "[snapshot] snapshot.dat already present at expected size + SHA; skipping download"
                );
                return Ok(());
            }
            Ok(false) => {
                eprintln!(
                    "[snapshot] existing snapshot.dat has the right size but WRONG SHA-256 \
                     (expected {}); deleting and re-downloading",
                    spec.sha256
                );
                let _ = std::fs::remove_file(&dest);
            }
            Err(e) => {
                eprintln!(
                    "[snapshot] could not SHA-verify existing snapshot.dat ({e}); deleting and re-downloading"
                );
                let _ = std::fs::remove_file(&dest);
            }
        }
    }
    std::fs::create_dir_all(&faststart_dir).map_err(|e| format!("create faststart dir: {e}"))?;

    // A real download is starting (we didn't hit the verified-skip return above),
    // so the snapshot.dat about to land has NOT been loaded — clear the shared
    // cross-process marker so no app's reclaim deletes this fresh file until a
    // load re-confirms it (C3, cross-process).
    clear_snapshot_marker(datadir);

    let tmp = faststart_dir.join("snapshot.dat.partial");
    // Timeouts so a STALLED connection (TCP open, no bytes flowing) surfaces the
    // "interrupted, press Retry" message instead of hanging forever on "Working…".
    // read_timeout is per-read inactivity (NOT a total cap — the download
    // legitimately takes minutes), so a healthy slow link is never cut off.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut resp = match client.get(&spec.url).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(if e.is_connect() || e.is_timeout() {
                "Couldn't reach the network to download the blockchain. Check your \
                 internet connection, then press Retry."
                    .to_string()
            } else {
                format!("Couldn't start the blockchain download: {e}")
            });
        }
    };
    if !resp.status().is_success() {
        return Err(format!(
            "Blockchain download failed (HTTP {}). Please try again.",
            resp.status().as_u16()
        ));
    }
    let total = resp.content_length().unwrap_or(spec.size_bytes).max(1);

    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| format!("create snapshot tmp: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_reported = 0.0_f64;

    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(e) = file.write_all(&chunk).await {
                    // Disk likely full mid-download — clean up the large partial so
                    // it doesn't waste space at the worst time, and say so plainly.
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(format!(
                        "Couldn't save the blockchain to disk — this machine may be low \
                         on space ({e}). Free up space, then press Retry."
                    ));
                }
                hasher.update(&chunk);
                downloaded += chunk.len() as u64;
                let ratio = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                // Throttle progress callbacks to ~1% steps to avoid lock thrash on a
                // multi-hundred-MB download (thousands of chunks).
                if ratio - last_reported >= 0.01 || ratio >= 1.0 {
                    last_reported = ratio;
                    progress(ratio);
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(format!(
                    "The blockchain download was interrupted ({e}). Check your \
                     connection and press Retry."
                ));
            }
        }
    }
    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!(
            "Couldn't finish saving the blockchain ({e}). Free up disk space and press Retry."
        ));
    }
    drop(file);

    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !hex.eq_ignore_ascii_case(&spec.sha256) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!(
            "The downloaded blockchain snapshot failed its integrity check \
             (expected {}, got {hex}). Press Retry to download it again.",
            spec.sha256
        ));
    }
    tokio::fs::rename(&tmp, &dest)
        .await
        .map_err(|e| format!("finalize snapshot: {e}"))?;
    eprintln!("[snapshot] downloaded + verified snapshot.dat ({downloaded} bytes)");
    Ok(())
}

/// Persistence seam for the authoritative "the snapshot has been loaded" flag.
/// The miner backs this with `EasyBtxState.snapshot_loaded` (under its state
/// file lock); easyBTX Node backs it with its own settings file. C3: reclaim
/// deletes `snapshot.dat` when `loaded()` is true, so `mark_loaded` must only
/// ever be called after a CONFIRMED successful `loadtxoutset` (this module is
/// the only caller and honors that).
/// Stall accounting for the header-anchor wait: given the latest observed
/// progress (headers or presync height), return the new `(best_seen,
/// stalled_polls)`. Any forward movement resets the stall counter. Pure →
/// unit-tested.
pub fn track_header_progress(progress: u64, last_seen: u64, stalled_polls: u32) -> (u64, u32) {
    // Liveness is any CHANGE in the observed height, not a new high-water mark.
    //
    // Header PRE-sync restarts from a low height every time it switches peer:
    // btxd climbs to ~184000, abandons that candidate chain, and begins again
    // from a few thousand. Against a high-water-mark rule the whole of that next
    // climb scores as "stalled", because nothing in it beats the previous peak.
    // One lap takes about three minutes, so three laps exhaust a ten-minute
    // zero-progress budget while the node is working perfectly.
    //
    // Measured 2026-08-12 on a GPU-less machine: the watcher gave up after
    // exactly that pattern (184000 -> 4076 -> climbing), the app logged "header
    // sync stalled short of the snapshot anchor" and left the node to sync from
    // genesis. Headers then reached 187,489 on their own about half an hour
    // later, with snapshot.dat sitting unused beside a node stuck at block 22.
    // Loading it by hand at that point fast-started the node to 179,000
    // immediately, which is what this watcher exists to do automatically.
    //
    // A genuinely hung btxd reports the SAME number every poll, so it still
    // trips the counter and we still give up. Only movement counts as alive.
    if progress != last_seen {
        (progress, 0)
    } else {
        (last_seen, stalled_polls + 1)
    }
}

pub trait SnapshotFlags: Send + Sync + 'static {
    fn loaded(&self) -> bool;
    fn mark_loaded(&self);
}

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
    tokio::spawn(async move {
        // FAST PATH (returning node): if a prior run already loaded the snapshot,
        // the persisted flag says so and the snapshot chainstate is on disk —
        // there is nothing to load. Return BEFORE the up-to-30-min header-anchor
        // wait below. Without this, the cold-start race where `getchainstates`
        // transiently doesn't yet report the snapshot chainstate would sink an
        // already-synced node into that long wait, pinning the UI on an early
        // setup phase for many minutes after a relaunch/heal (2026-05-29 invest.).
        if flags.loaded() {
            // Backfill the shared cross-process marker for installs that loaded
            // BEFORE the marker existed, so reclaim's marker gate can proceed.
            if !snapshot_marker_present(&datadir) {
                mark_snapshot_marker(&datadir);
            }
            return;
        }
        // Already have a snapshot chainstate? Nothing to do — but persist the
        // loaded flag so later reclaim runs can safely drop snapshot.dat.
        match get_chainstates(&rpc).await {
            Ok(cs) if cs.snapshot().is_some() => {
                flags.mark_loaded();
                mark_snapshot_marker(&datadir);
                return;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[snapshot] getchainstates unavailable ({e}); skipping snapshot load");
                return;
            }
        }

        let snapshot_path = datadir.join("faststart").join("snapshot.dat");
        if !snapshot_path.exists() {
            // No snapshot file to load (e.g. a clean full-sync install) — fine.
            return;
        }

        // Wait for headers to reach the snapshot anchor before loading —
        // `loadtxoutset` is rejected until then. Patience is free in a
        // background task, so the ONLY reason to stop is a genuinely STALLED
        // header sync, never a wall clock: the old fixed budgets (180 s, then
        // ~30 min) each abandoned a WORKING sync short of the anchor — a
        // from-genesis header sync took ~65 min in the 2026-07-12 live run,
        // the 30-min budget expired ~15 min early, and the node fell into a
        // full sync-from-genesis with snapshot.dat sitting unused. Headers
        // PRE-sync keeps `getblockchaininfo.headers` at 0, so btxd's own log
        // line (read_header_presync) counts as forward progress too.
        const STALL_GIVE_UP_POLLS: u32 = 300; // × 2 s = 10 min of ZERO progress
        const HEADER_WAIT_POLL_MS: u64 = 2000;
        let mut headers_ready = false;
        // LAST seen, not BEST seen: header pre-sync restarts from a low height
        // on every peer switch, and treating that as a stall abandons a working
        // sync. See track_header_progress.
        let mut last_seen: u64 = 0;
        let mut stalled_polls: u32 = 0;
        let mut poll_n: u64 = 0;
        loop {
            match get_blockchain_info(&rpc).await {
                Ok(info) if snapshot_anchor_reached(info.headers, anchor_height) => {
                    headers_ready = true;
                    break;
                }
                Ok(info) => {
                    let presync = crate::node::read_header_presync(&datadir)
                        .map(|(h, _)| h)
                        .unwrap_or(0);
                    let progress = info.headers.max(presync);
                    (last_seen, stalled_polls) =
                        track_header_progress(progress, last_seen, stalled_polls);
                    if poll_n % 10 == 0 {
                        eprintln!(
                            "[snapshot] waiting for headers to reach snapshot anchor: {}/{}",
                            progress, anchor_height
                        );
                    }
                }
                Err(_) => {
                    stalled_polls += 1;
                }
            }
            if stalled_polls >= STALL_GIVE_UP_POLLS {
                break;
            }
            poll_n += 1;
            tokio::time::sleep(std::time::Duration::from_millis(HEADER_WAIT_POLL_MS)).await;
        }
        if !headers_ready {
            eprintln!(
                "[snapshot] header sync stalled short of the snapshot anchor; \
                 leaving the node to sync the slow way (non-fatal)"
            );
            return;
        }

        // A peer may have advanced past / loaded the snapshot during the wait.
        if matches!(get_chainstates(&rpc).await, Ok(cs) if cs.snapshot().is_some()) {
            flags.mark_loaded();
            mark_snapshot_marker(&datadir);
            return;
        }

        eprintln!("[snapshot] headers at anchor, no snapshot chainstate yet; running loadtxoutset");
        let cli = btx_cli.clone();
        let dd = datadir.clone();
        let snap = snapshot_path.clone();
        // loadtxoutset can take a while to read+validate the snapshot file; run it
        // on a blocking thread with rpcclienttimeout=0 (no client-side timeout),
        // mirroring the faststart wrapper's documented invocation.
        let result = tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&cli);
            cmd.arg(format!("-datadir={}", dd.display()))
                .arg("-rpcclienttimeout=0")
                .arg("loadtxoutset")
                .arg(&snap);
            // Don't flash a console window on Windows. Compiled out on macOS.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            cmd.output()
        })
        .await;

        match result {
            Ok(Ok(out)) if out.status.success() => {
                eprintln!("[snapshot] loadtxoutset succeeded; snapshot chainstate activating");
                // C3: persist loaded=true ONLY here — on a confirmed successful
                // loadtxoutset. `disk::reclaim_disk` gates deleting snapshot.dat
                // on this flag AND the shared cross-process marker.
                flags.mark_loaded();
                mark_snapshot_marker(&datadir);
            }
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                // "Work does not exceed active chainstate" means a peer already
                // advanced the chain past the snapshot — that's success, not error.
                if stderr.contains("Work does not exceed active chainstate") {
                    eprintln!("[snapshot] snapshot already superseded by active chain; continuing");
                    // Active chain already past the snapshot — snapshot.dat is
                    // safe to drop on the next reclaim, exactly as if it had
                    // been loaded into the snapshot chainstate.
                    flags.mark_loaded();
                    mark_snapshot_marker(&datadir);
                } else {
                    eprintln!("[snapshot] loadtxoutset failed (non-fatal): {stderr}");
                }
            }
            Ok(Err(e)) => eprintln!("[snapshot] could not spawn loadtxoutset (non-fatal): {e}"),
            Err(e) => eprintln!("[snapshot] loadtxoutset task panicked (non-fatal): {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    fn sha_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn a_repinned_release_sees_the_previous_snapshot_as_stale() {
        // The 0.6.12 -> 0.6.13 upgrade in one test. Old asset on disk, new spec
        // pinned, and the loader must be told to refresh it rather than load it.
        let dir = tempfile::tempdir().unwrap();
        let fs_dir = dir.path().join("faststart");
        std::fs::create_dir_all(&fs_dir).unwrap();
        let old = fs_dir.join("snapshot.dat");
        std::fs::write(&old, b"pretend this is the 179000 asset").unwrap();

        let spec = v0_34_5_spec();
        assert!(!snapshot_file_matches_spec(&spec, dir.path()));
        assert!(snapshot_file_is_stale_for_spec(&spec, dir.path()));
    }

    #[test]
    fn no_snapshot_on_disk_is_not_stale_it_is_absent() {
        // Absent must NOT report stale: a fresh install has no file and there is
        // nothing to refresh. Only an EXISTING wrong file is stale.
        let dir = tempfile::tempdir().unwrap();
        assert!(!snapshot_file_is_stale_for_spec(
            &v0_34_5_spec(),
            dir.path()
        ));
        assert!(!snapshot_file_matches_spec(&v0_34_5_spec(), dir.path()));
    }

    #[test]
    fn snapshot_anchor_gate_matches_assumeutxo_precondition() {
        let anchor = v0_32_11_spec().anchor_height;
        // loadtxoutset is rejected until headers reach the snapshot anchor — the
        // fresh node (headers ~0) must wait, not attempt the load.
        assert!(!snapshot_anchor_reached(0, anchor));
        assert!(!snapshot_anchor_reached(anchor - 1, anchor));
        // At or beyond the anchor → safe to load.
        assert!(snapshot_anchor_reached(anchor, anchor));
        assert!(snapshot_anchor_reached(anchor + 5000, anchor));
    }

    #[test]
    fn header_progress_tracker_resets_stall_on_any_movement() {
        // Forward movement resets the stall counter.
        assert_eq!(track_header_progress(100, 50, 7), (100, 0));
        // No movement increments it.
        assert_eq!(track_header_progress(100, 100, 0), (100, 1));
        // A pre-sync RESTART is movement, not a stall. btxd drops from ~184000
        // back to a few thousand every time it switches peer, and the previous
        // rule counted that whole working climb as stalled — which is what made
        // the watcher abandon a healthy sync and leave snapshot.dat unused.
        assert_eq!(track_header_progress(4_076, 184_000, 3), (4_076, 0));
    }

    #[test]
    fn header_progress_tracker_survives_repeated_presync_laps() {
        // Three full pre-sync laps, exactly the pattern measured on a GPU-less
        // machine. The counter must never approach the give-up threshold while
        // the height is still moving.
        let (mut last, mut stalled) = (0u64, 0u32);
        for _lap in 0..3 {
            for h in (4_000..=184_000).step_by(2_000) {
                (last, stalled) = track_header_progress(h, last, stalled);
                assert_eq!(stalled, 0, "a moving header height is never a stall");
            }
        }
        // A genuinely hung node repeats one value, and still trips the counter.
        for expected in 1..=5 {
            (last, stalled) = track_header_progress(184_000, last, stalled);
            assert_eq!(stalled, expected);
        }
    }

    #[test]
    fn verify_file_sha256_matches_and_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("blob");
        std::fs::write(&p, b"hello world").unwrap();
        let good = sha_hex(b"hello world");
        assert_eq!(verify_file_sha256(&p, &good), Ok(true));
        // Case-insensitive hex comparison.
        assert_eq!(verify_file_sha256(&p, &good.to_uppercase()), Ok(true));
        assert_eq!(verify_file_sha256(&p, &sha_hex(b"other")), Ok(false));
        // Unreadable path → Err, never a silent false-positive.
        assert!(verify_file_sha256(&dir.path().join("missing"), &good).is_err());
    }

    fn test_spec(server_url: &str, body: &[u8]) -> SnapshotSpec {
        SnapshotSpec {
            url: format!("{server_url}/snapshot.dat"),
            sha256: sha_hex(body),
            size_bytes: body.len() as u64,
            anchor_height: 100,
        }
    }

    #[tokio::test]
    async fn a_right_size_wrong_sha_file_is_deleted_and_refetched() {
        // The M13 branch, previously untested. The skip is keyed on SIZE first,
        // so a corrupt or truncated-then-padded snapshot.dat of exactly the
        // right length would be skipped as "already present" if the SHA check
        // were ever dropped or inverted - and the node would loadtxoutset a file
        // that is not the pinned snapshot. Size alone must never be enough.
        let mut server = mockito::Server::new_async().await;
        let body = b"snapshot-bytes-snapshot-bytes".to_vec();
        let _m = server
            .mock("GET", "/snapshot.dat")
            .with_status(200)
            .with_body(body.clone())
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let spec = test_spec(&server.url(), &body);

        // Same length, different bytes: passes the size gate, fails the SHA.
        let faststart = dir.path().join("faststart");
        std::fs::create_dir_all(&faststart).unwrap();
        let dest = faststart.join("snapshot.dat");
        let impostor = vec![b'x'; body.len()];
        std::fs::write(&dest, &impostor).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().len(),
            spec.size_bytes,
            "the fixture must hit the size gate, or this tests nothing"
        );

        download_snapshot(&spec, dir.path(), &|_| {}).await.unwrap();

        assert_eq!(
            std::fs::read(&dest).unwrap(),
            body,
            "the impostor must be replaced by the served bytes"
        );
        assert!(!dir
            .path()
            .join("faststart")
            .join("snapshot.dat.partial")
            .exists());
    }

    #[tokio::test]
    async fn a_matching_file_is_skipped_without_touching_the_network() {
        // The other side of the same gate: a file that is byte-identical to the
        // pin must not be re-downloaded. No mockito server at all, so any HTTP
        // attempt fails the test by failing the call.
        let dir = tempfile::tempdir().unwrap();
        let body = b"snapshot-bytes-snapshot-bytes".to_vec();
        let spec = test_spec("http://127.0.0.1:1/unreachable", &body);

        let faststart = dir.path().join("faststart");
        std::fs::create_dir_all(&faststart).unwrap();
        std::fs::write(faststart.join("snapshot.dat"), &body).unwrap();

        download_snapshot(&spec, dir.path(), &|_| {})
            .await
            .expect("a verified file must skip the download, not attempt it");
    }

    #[tokio::test]
    async fn download_snapshot_writes_verified_file_and_reports_progress() {
        let mut server = mockito::Server::new_async().await;
        let body = b"snapshot-bytes-snapshot-bytes".to_vec();
        let _m = server
            .mock("GET", "/snapshot.dat")
            .with_status(200)
            .with_body(body.clone())
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let spec = test_spec(&server.url(), &body);

        let max_seen = Mutex::new(0.0_f64);
        let progress = |r: f64| {
            let mut g = max_seen.lock().unwrap();
            if r > *g {
                *g = r;
            }
        };
        download_snapshot(&spec, dir.path(), &progress)
            .await
            .unwrap();

        let dest = dir.path().join("faststart").join("snapshot.dat");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(
            (*max_seen.lock().unwrap() - 1.0).abs() < f64::EPSILON,
            "progress must reach 1.0"
        );
        // No .partial left behind.
        assert!(!dir
            .path()
            .join("faststart")
            .join("snapshot.dat.partial")
            .exists());
    }

    #[tokio::test]
    async fn download_snapshot_rejects_wrong_sha_and_cleans_up() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/snapshot.dat")
            .with_status(200)
            .with_body(b"tampered-content".to_vec())
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        // Spec expects DIFFERENT content → integrity failure.
        let mut spec = test_spec(&server.url(), b"expected-content");
        spec.size_bytes = 16; // match tampered length so only the SHA gate fires

        let err = download_snapshot(&spec, dir.path(), &|_| {})
            .await
            .unwrap_err();
        assert!(
            err.contains("integrity"),
            "error should mention the integrity check, got: {err}"
        );
        // Neither the final file nor the partial may survive.
        assert!(!dir.path().join("faststart").join("snapshot.dat").exists());
        assert!(!dir
            .path()
            .join("faststart")
            .join("snapshot.dat.partial")
            .exists());
    }

    #[tokio::test]
    async fn download_snapshot_skips_when_verified_file_present() {
        // A correctly-sized + correct-SHA file short-circuits: no network call.
        // mockito would panic the test on an unexpected hit only if we asserted
        // the mock; instead we point the spec at an unroutable URL — a network
        // attempt would error, so Ok(()) proves the skip path ran.
        let dir = tempfile::tempdir().unwrap();
        let body = b"already-downloaded".to_vec();
        let fs_dir = dir.path().join("faststart");
        std::fs::create_dir_all(&fs_dir).unwrap();
        std::fs::write(fs_dir.join("snapshot.dat"), &body).unwrap();
        let spec = SnapshotSpec {
            url: "http://127.0.0.1:1/unreachable".into(),
            sha256: sha_hex(&body),
            size_bytes: body.len() as u64,
            anchor_height: 100,
        };
        download_snapshot(&spec, dir.path(), &|_| {}).await.unwrap();
    }

    #[test]
    fn snapshot_flags_trait_is_object_safe_and_arc_shareable() {
        struct MemFlags(AtomicBool);
        impl SnapshotFlags for MemFlags {
            fn loaded(&self) -> bool {
                self.0.load(Ordering::Relaxed)
            }
            fn mark_loaded(&self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let flags: Arc<dyn SnapshotFlags> = Arc::new(MemFlags(AtomicBool::new(false)));
        assert!(!flags.loaded());
        flags.mark_loaded();
        assert!(flags.loaded());
    }
}
