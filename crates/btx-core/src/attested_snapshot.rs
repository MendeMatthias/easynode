//! Signed ("attested") UTXO snapshots: how a node that follows signatures
//! starts near the tip instead of at the snapshot compiled into the engine.
//!
//! WHY. A node that follows signatures (a native Windows PC, an M5, a PC
//! without an NVIDIA driver) starts from the compiled 219,000 snapshot, more
//! than 10,000 blocks below the tip on 2026-09-26, and then needs a signature
//! for every one of those blocks. After the 23 September split one reachable
//! peer held them, btxscan's node, and while it was frozen a new install
//! crawled: 4 blocks in 30 minutes, reported that day. This project's signer,
//! `02d5efca`, already exports the chain state and signs it every 500 blocks
//! (docs/snapshot-serve.md). A node that follows `02d5efca` can load that pair
//! with `loadtxoutsetattested` and start a few hundred blocks from the tip.
//!
//! WHAT IT TRUSTS. The engine loads the pair only when a key this node pins
//! signed its manifest (`VerifyUtxoSnapshotManifest`, v0.34.9), so neither the
//! host nor the pointer below can forge one; the checks here only keep a bad
//! download away from it. Loading takes the signer's word for the balances at
//! the base, which is more than a mirror takes today (the signer's word for
//! each block, balances checked here). Not for long: the background
//! chainstate re-checks every block below the base and compares the result
//! with the signed hash (`MaybeCompleteSnapshotValidation` reads
//! `m_attested_assumeutxo`), and a mismatch invalidates the snapshot
//! chainstate. Owner's decision, 2026-09-26.
//!
//! WHO. Only a node that follows signatures. The engine refuses the RPC on a
//! node that checks blocks itself ("strict consensus nodes must not
//! attested-fast-forward"), so those keep the compiled snapshot.
//!
//! WHERE FROM. `scripts/publish-attested-snapshot.sh`, run beside the keeper,
//! publishes each pair to MendeMatthias/EasyBTX-releases as a pre-release
//! `utxo-snapshot-<height>` with the files named as the keeper names them, and
//! replaces `utxo-snapshot-latest/attested-snapshot.json`, the keeper's own
//! offer record, as the pointer to the newest. When the pointer is missing or
//! fails its checks, the pair published before it existed ([`pinned_pair`])
//! is used. When both fail, the compiled snapshot loads exactly as before.

use crate::snapshot::verify_file_sha256;
use crate::snapshot_serve::{manifest_file_name, snapshot_file_name};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Where the pairs and the pointer are published.
pub const RELEASES_DOWNLOAD: &str =
    "https://github.com/MendeMatthias/EasyBTX-releases/releases/download";

/// The one tag whose asset is replaced with every new pair. A pre-release, like
/// the pairs, so none of them can become the repository's "latest" release,
/// which the app releases use.
pub const POINTER_TAG: &str = "utxo-snapshot-latest";

/// Not `latest.json`: that name belongs to the app's update feeds.
pub const POINTER_ASSET: &str = "attested-snapshot.json";

/// A pointer is a few hundred bytes. Anything far larger is not one.
pub const MAX_POINTER_BYTES: usize = 16 * 1024;

/// The pairs so far are about 9 MB. A pointer claiming more than this is
/// refused before anything is downloaded.
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// A manifest with one signature is 335 bytes.
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024;

/// One published pair. The field names are the keeper's own
/// (`snapshot_serve::OfferRecord`), so its `current-offer.json` is the
/// pointer as it stands; fields this side does not use are ignored.
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

pub fn pointer_url() -> String {
    format!("{RELEASES_DOWNLOAD}/{POINTER_TAG}/{POINTER_ASSET}")
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

/// Whether a pair is worth downloading. It does not decide trust (the engine
/// does, from the signature), only that the pair is shaped like one and would
/// start this node higher than the snapshot compiled into its engine.
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

pub fn parse_pointer(body: &str) -> Result<AttestedPair, String> {
    serde_json::from_str(body).map_err(|e| format!("unreadable pointer: {e}"))
}

/// The pairs to try, best first: the published one when it checks out and is
/// newer than the pin, then the pin. Empty when neither would start this node
/// above the compiled snapshot.
pub fn candidates(
    published: Option<&AttestedPair>,
    pinned: &AttestedPair,
    compiled_anchor: u64,
) -> Vec<AttestedPair> {
    let mut out = Vec::new();
    if let Some(p) = published {
        if check(p, compiled_anchor).is_ok() && p.height > pinned.height {
            out.push(p.clone());
        }
    }
    if check(pinned, compiled_anchor).is_ok() {
        out.push(pinned.clone());
    }
    out
}

/// Where a pair is kept until the snapshot sweep removes it with the compiled
/// one: beside `faststart/snapshot.dat`.
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
/// is not in the pointer, so its SHA-256 alone decides it.
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

fn http_client() -> Result<reqwest::Client, String> {
    // The same timeouts as the compiled snapshot's download: a stalled
    // connection fails, a slow one is never cut off.
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

pub async fn fetch_pointer(client: &reqwest::Client) -> Result<AttestedPair, String> {
    let resp = client
        .get(pointer_url())
        .send()
        .await
        .map_err(|e| format!("pointer unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("pointer HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("pointer read: {e}"))?;
    if bytes.len() > MAX_POINTER_BYTES {
        return Err(format!("pointer is {} bytes, not a pointer", bytes.len()));
    }
    parse_pointer(&String::from_utf8_lossy(&bytes))
}

/// Stream `url` into `dest` through a `.partial`, refusing more than `cap`
/// bytes and anything whose size or SHA-256 differs from what the pair says.
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    expected_sha256: &str,
    cap: u64,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
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
    let mut hasher = Sha256::new();
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
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if expected_size.is_some_and(|s| s != written) || !got.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!(
            "{written} bytes with SHA-256 {got}, not the published pair's"
        ));
    }
    tokio::fs::rename(&tmp, dest)
        .await
        .map_err(|e| format!("rename: {e}"))
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
    .map_err(|e| format!("snapshot: {e}"))
}

/// The newest signed pair this node can use, verified on disk, or `None`.
/// Best effort and quiet about it: every failure is logged and leaves the
/// caller on the compiled snapshot, which is where it was before this existed.
pub async fn prepare(
    datadir: &Path,
    compiled_anchor: u64,
) -> Option<(AttestedPair, PathBuf, PathBuf)> {
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[attested] {e}; using the compiled snapshot");
            return None;
        }
    };
    let published = match fetch_pointer(&client).await {
        Ok(p) => {
            if let Err(e) = check(&p, compiled_anchor) {
                eprintln!("[attested] published pair {} refused: {e}", p.height);
            }
            Some(p)
        }
        Err(e) => {
            eprintln!("[attested] no published pair ({e}); trying the pinned one");
            None
        }
    };
    for pair in candidates(published.as_ref(), &pinned_pair(), compiled_anchor) {
        let (file, manifest) = pair_paths(datadir, pair.height);
        if !on_disk(datadir, &pair) {
            if let Err(e) = download_pair(&client, datadir, &pair).await {
                eprintln!("[attested] pair {} not downloaded: {e}", pair.height);
                continue;
            }
        }
        prune_others(datadir, pair.height);
        eprintln!("[attested] signed pair {} ready", pair.height);
        return Some((pair, file, manifest));
    }
    None
}

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
    fn the_keepers_own_offer_record_is_a_pointer() {
        // snapshot_serve::save_record's output, as the publisher uploads it.
        let record = crate::snapshot_serve::OfferRecord {
            height: 229_500,
            block_hash: "11".repeat(32),
            txoutset_hash: "22".repeat(32),
            file_size: 9_100_000,
            sha256: "33".repeat(32),
            manifest_sha256: "44".repeat(32),
            file_hash: "55".repeat(32),
            chunk_count: 9,
            signatures: 1,
            offered_at: 1_790_000_000,
        };
        let json = serde_json::to_string_pretty(&record).unwrap();
        let p = parse_pointer(&json).unwrap();
        assert_eq!(p.height, 229_500);
        assert_eq!(p.file_size, 9_100_000);
        assert_eq!(p.sha256, "33".repeat(32));
        assert_eq!(p.manifest_sha256, "44".repeat(32));
        assert!(check(&p, COMPILED).is_ok());
    }

    #[test]
    fn a_pair_that_would_not_start_the_node_higher_is_refused() {
        assert!(check(&pair(COMPILED), COMPILED).is_err());
        assert!(check(&pair(COMPILED - 1), COMPILED).is_err());
        assert!(check(&pair(COMPILED + 1), COMPILED).is_ok());
    }

    #[test]
    fn a_pointer_that_is_not_shaped_like_a_pair_is_refused_before_any_download() {
        let mut p = pair(229_500);
        p.file_size = MAX_SNAPSHOT_BYTES + 1;
        assert!(check(&p, COMPILED).is_err(), "oversized");
        let mut p = pair(229_500);
        p.file_size = 0;
        assert!(check(&p, COMPILED).is_err(), "empty");
        let mut p = pair(229_500);
        p.sha256 = "zz".repeat(32);
        assert!(check(&p, COMPILED).is_err(), "not hex");
        let mut p = pair(229_500);
        p.manifest_sha256.pop();
        assert!(check(&p, COMPILED).is_err(), "short");
        assert!(parse_pointer("<html>Not Found</html>").is_err());
        assert!(
            parse_pointer(r#"{"height": 229500}"#).is_err(),
            "fields missing"
        );
    }

    #[test]
    fn a_newer_published_pair_goes_first_and_the_pin_stays_behind_it() {
        let pinned = pinned_pair();
        let got = candidates(Some(&pair(229_500)), &pinned, COMPILED);
        assert_eq!(
            got.iter().map(|p| p.height).collect::<Vec<_>>(),
            vec![229_500, pinned.height]
        );
    }

    #[test]
    fn an_older_or_broken_published_pair_leaves_only_the_pin() {
        let pinned = pinned_pair();
        for published in [pair(pinned.height), pair(pinned.height - 1), {
            let mut p = pair(229_500);
            p.sha256 = String::new();
            p
        }] {
            let got = candidates(Some(&published), &pinned, COMPILED);
            assert_eq!(got, vec![pinned.clone()], "{published:?}");
        }
        assert_eq!(candidates(None, &pinned, COMPILED), vec![pinned.clone()]);
    }

    #[test]
    fn nothing_is_tried_when_even_the_pin_is_not_above_the_compiled_snapshot() {
        // An engine that one day compiles a base above 225,927 makes the pin
        // pointless, and it must drop out rather than be loaded lower.
        let pinned = pinned_pair();
        assert!(candidates(None, &pinned, pinned.height).is_empty());
        assert_eq!(
            candidates(Some(&pair(pinned.height + 500)), &pinned, pinned.height)
                .iter()
                .map(|p| p.height)
                .collect::<Vec<_>>(),
            vec![pinned.height + 500]
        );
    }

    #[test]
    fn the_pin_is_a_real_pair_above_the_compiled_snapshot() {
        let p = pinned_pair();
        assert!(check(&p, COMPILED).is_ok());
        // Below the 23 September split, on the chain both sides share.
        assert!(p.height < 227_313);
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
            pointer_url(),
            "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/utxo-snapshot-latest/attested-snapshot.json"
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
