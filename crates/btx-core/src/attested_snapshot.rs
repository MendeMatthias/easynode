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

/// The cap on the PINNED pair's manifest download: a manifest with one
/// signature is 335 bytes, and the pinned pair's is that. Not the cap on a
/// manifest in general: that is [`cs::MAX_MANIFEST_BYTES`] (64 KiB, the
/// engine's), which a confirmed pointer's `manifest_size` must stay under.
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
/// * HTTP 200 with exactly `{"disputed": [<height>, ...]}` while any dispute
///   stands (section 6a); [`Latest::Disputed`]. The heights are in ascending
///   order and there is at least one; the app refuses an empty list. The app
///   then falls back.
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
/// `manifest_url` and `file_url` are HTTPS on `easybtx.com` or a
/// `<store>.public.blob.vercel-storage.com` host, with no port and no user
/// info ([`confirmed_url_allowed`]); the app downloads from nowhere else.
///
/// The manifest served is the merged one, every signature the website
/// accepted; the app checks it and keeps only what its node pins. Nothing
/// here is trusted: the statement's signatures decide, and every field that
/// repeats the statement must match it or the pair is refused. `operators`
/// is never shown: the app names only the operators it verified.
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

/// What `latest` answered with HTTP 200.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Latest {
    Confirmed(ConfirmedPointer),
    /// The operators disagree about these grid heights (section 6a). Nothing
    /// is confirmed at any height while this stands.
    Disputed(Vec<u64>),
}

impl Latest {
    /// The height the one sentence names (section 6a).
    pub fn newest_disputed(&self) -> Option<u64> {
        match self {
            Latest::Disputed(heights) => heights.iter().copied().max(),
            Latest::Confirmed(_) => None,
        }
    }
}

/// Read `latest`'s answer: the dispute shape (an object whose only key is
/// `disputed`, a non-empty array of heights), else a pointer. A body that
/// carries `disputed` beside anything else is neither.
pub fn parse_latest(body: &[u8]) -> Result<Latest, String> {
    if body.len() > MAX_POINTER_BYTES {
        return Err(format!("{} bytes is not an answer from latest", body.len()));
    }
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unreadable answer: {e}"))?;
    if let Some(obj) = v.as_object().filter(|o| o.contains_key("disputed")) {
        let heights: Option<Vec<u64>> = obj["disputed"]
            .as_array()
            .and_then(|a| a.iter().map(|h| h.as_u64()).collect());
        return match heights {
            Some(h) if obj.len() == 1 && !h.is_empty() => Ok(Latest::Disputed(h)),
            _ => Err("a dispute answer that is not only a list of heights".into()),
        };
    }
    parse_confirmed_pointer(body).map(Latest::Confirmed)
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
        pair.file_size,
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

/// The body of `latest`'s answer, read no further than [`MAX_POINTER_BYTES`]:
/// a server that says it is sending more is refused before the body is
/// read, and one that streams more without saying so is cut off at the cap.
async fn read_pointer_body(mut resp: reqwest::Response) -> Result<Vec<u8>, NotReady> {
    if let Some(len) = resp
        .content_length()
        .filter(|&n| n > MAX_POINTER_BYTES as u64)
    {
        return Err(format!("the pointer says {len} bytes, not a pointer").into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| NotReady::PointerUnread(format!("pointer read: {e}")))?
    {
        if body.len() + chunk.len() > MAX_POINTER_BYTES {
            return Err(format!(
                "the pointer is more than {MAX_POINTER_BYTES} bytes, not a pointer"
            )
            .into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Why no confirmed pair is ready. `latest` could not be read at all (no
/// answer, a server error, a body cut off), or anything else, which is an
/// answer: a 404, a dispute, a refusal, a failed download. The strings are
/// for the log.
enum NotReady {
    PointerUnread(String),
    Answered(String),
}

impl From<String> for NotReady {
    fn from(why: String) -> Self {
        NotReady::Answered(why)
    }
}

impl NotReady {
    fn why(self) -> String {
        match self {
            NotReady::PointerUnread(why) | NotReady::Answered(why) => why,
        }
    }
}

/// Read `latest` at `pointer_url`, then download and check the pair it
/// names (section 7, steps 1 and 2): the manifest first, checked in full by
/// [`cs::check`], then the file, whose size and double SHA-256 must be the
/// statement's. A disputed `latest` stops here, before any download, and the
/// caller takes the fallbacks (section 9). `url_ok` is
/// [`confirmed_url_allowed`] everywhere but tests.
pub async fn prepare_confirmed(
    client: &reqwest::Client,
    pointer_url: &str,
    datadir: &Path,
    view: &NodeView,
    regtest_env: Option<&str>,
    url_ok: fn(&str) -> bool,
) -> Result<ReadyPair, String> {
    confirmed_pair(client, pointer_url, datadir, view, regtest_env, url_ok)
        .await
        .map_err(NotReady::why)
}

/// [`prepare_confirmed`], saying whether `latest` could be read at all.
async fn confirmed_pair(
    client: &reqwest::Client,
    pointer_url: &str,
    datadir: &Path,
    view: &NodeView,
    regtest_env: Option<&str>,
    url_ok: fn(&str) -> bool,
) -> Result<ReadyPair, NotReady> {
    let resp = client
        .get(pointer_url)
        .send()
        .await
        .map_err(|e| NotReady::PointerUnread(format!("pointer unreachable: {e}")))?;
    let status = resp.status();
    if status.is_server_error() {
        return Err(NotReady::PointerUnread(format!(
            "the pointer did not answer (HTTP {})",
            status.as_u16()
        )));
    }
    if status.as_u16() != 200 {
        return Err(format!("no confirmed snapshot (HTTP {})", status.as_u16()).into());
    }
    let body = read_pointer_body(resp).await?;
    let p = match parse_latest(&body)? {
        Latest::Confirmed(p) => p,
        disputed @ Latest::Disputed(_) => {
            return Err(format!(
                "the snapshot operators disagree about block {}",
                crate::snapshot_start::block_number(disputed.newest_disputed().unwrap_or(0))
            )
            .into())
        }
    };
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
            p.manifest_size,
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
    pointer_matches(&p, &confirmed).inspect_err(|_| {
        let _ = std::fs::remove_file(&manifest);
    })?;

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
            st.file_size(),
        )
        .await
        .map_err(|e| format!("snapshot: {e}"))?
    };
    if !cs::file_matches(st, st.file_size(), &double) {
        let _ = std::fs::remove_file(&file);
        return Err(String::from("the file is not the one the statement signs").into());
    }
    Ok(ReadyPair {
        kind: PairKind::Confirmed,
        height: confirmed.height,
        file,
        manifest,
    })
}

/// [`prepare_confirmed`] for a validating node's mirror launch, whose marker
/// names the pair it checked before it launched (`marked`, from
/// `crate::node::MirrorLoad`). When `latest` cannot be read at all, the
/// confirmed pair the marker names is taken from disk rather than lost
/// ([`marked_pair_on_disk`]); an answer decides as always.
#[allow(clippy::too_many_arguments)]
async fn prepare_confirmed_or_marked(
    client: &reqwest::Client,
    pointer_url: &str,
    datadir: &Path,
    view: &NodeView,
    regtest_env: Option<&str>,
    url_ok: fn(&str) -> bool,
    marked: Option<(PairKind, u64)>,
) -> Result<ReadyPair, String> {
    match confirmed_pair(client, pointer_url, datadir, view, regtest_env, url_ok).await {
        Ok(pair) => Ok(pair),
        Err(NotReady::PointerUnread(why)) => match marked_pair_on_disk(datadir, marked) {
            Some(pair) => {
                eprintln!(
                    "[attested] {why}; loading the confirmed pair {} this launch checked before \
                     it started",
                    pair.height
                );
                Ok(pair)
            }
            None => Err(why),
        },
        Err(NotReady::Answered(why)) => Err(why),
    }
}

/// The confirmed pair a mirror-load marker names, when both its files are
/// on disk. Not checked here: `crate::confirmed_load::load` checks it again
/// in full (statement, operators, pins, chain, file hash) before the engine
/// sees it. A marker for the pinned pair needs none of this: that pair on
/// disk is found without the network ([`prepare_start`]).
fn marked_pair_on_disk(datadir: &Path, marked: Option<(PairKind, u64)>) -> Option<ReadyPair> {
    let (PairKind::Confirmed, height) = marked? else {
        return None;
    };
    let (file, manifest) = pair_paths(datadir, height);
    (file.is_file() && manifest.is_file()).then_some(ReadyPair {
        kind: PairKind::Confirmed,
        height,
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
    prepare_start_marked(datadir, view, compiled_anchor, None).await
}

/// [`prepare_start`] on a validating node's mirror launch, whose marker names
/// the pair it checked before it launched (`marked`: kind and height, see
/// [`prepare_confirmed_or_marked`]).
pub async fn prepare_start_marked(
    datadir: &Path,
    view: &NodeView,
    compiled_anchor: u64,
    marked: Option<(PairKind, u64)>,
) -> Option<ReadyPair> {
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[attested] {e}; using the compiled snapshot");
            return None;
        }
    };
    let regtest_env = crate::operators::regtest_env();
    match prepare_confirmed_or_marked(
        &client,
        CONFIRMED_POINTER_URL,
        datadir,
        view,
        regtest_env.as_deref(),
        confirmed_url_allowed,
        marked,
    )
    .await
    {
        Ok(pair) => {
            prune_others(datadir, pair.height);
            eprintln!("[attested] confirmed snapshot {} ready", pair.height);
            return Some(pair);
        }
        // A disputed `latest`, a 404 and every refusal alike (section 9).
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
        assert_eq!(p.height % cs::SNAPSHOT_GRID as u64, 0);
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

    /// Section 6a: while any dispute stands, `latest` answers only the
    /// disputed heights. Its own shape, read strictly: a body that mixes it
    /// with a pointer is neither.
    #[test]
    fn a_dispute_answer_is_its_own_shape() {
        let d = parse_latest(br#"{"disputed": [233700, 233800]}"#).unwrap();
        assert_eq!(d, Latest::Disputed(vec![233_700, 233_800]));
        assert_eq!(d.newest_disputed(), Some(233_800));
        assert!(matches!(
            parse_latest(POINTER.as_bytes()),
            Ok(Latest::Confirmed(_))
        ));
        assert_eq!(
            parse_latest(POINTER.as_bytes()).unwrap().newest_disputed(),
            None
        );
        for bad in [
            &br#"{"disputed": []}"#[..],
            br#"{"disputed": "233800"}"#,
            br#"{"disputed": [233800], "version": 1}"#,
            br#"{"disputed": [-5]}"#,
        ] {
            assert!(
                parse_latest(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let mut mixed: serde_json::Value = serde_json::from_str(POINTER).unwrap();
        mixed["disputed"] = serde_json::json!([232000]);
        assert!(parse_latest(mixed.to_string().as_bytes()).is_err());
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
        assert!(
            !pair_paths(tmp.path(), 100).1.exists(),
            "the checked manifest is not kept for a pointer that misdescribes it"
        );
    }

    /// Final review triage (T3): each download stops at the size the pointer
    /// and the statement give it, not at the largest size any pair may have.
    #[tokio::test]
    async fn a_download_stops_at_the_size_it_was_given() {
        let env = format!("producer={P};confirmer={C}");
        for longer in ["/m", "/f"] {
            let tmp = tempfile::tempdir().unwrap();
            let mut server = mockito::Server::new_async().await;
            let p = regtest_pointer(&server.url(), R_PC, R_DAT);
            server
                .mock("GET", "/latest")
                .with_body(serde_json::to_vec(&p).unwrap())
                .create_async()
                .await;
            for (path, body) in [("/m", R_PC), ("/f", R_DAT)] {
                let mut body = body.to_vec();
                if path == longer {
                    body.extend_from_slice(&[0u8; 4096]);
                }
                server
                    .mock("GET", path)
                    .with_body(body)
                    .create_async()
                    .await;
            }
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
            let (what, size) = if longer == "/m" {
                ("manifest", R_PC.len())
            } else {
                ("snapshot", R_DAT.len())
            };
            assert!(
                err.contains(&format!("{what}: larger than {size} bytes")),
                "{longer}: {err}"
            );
        }
    }

    #[tokio::test]
    async fn a_disputed_latest_downloads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/latest")
            .with_body(r#"{"disputed":[100]}"#)
            .create_async()
            .await;
        let manifest = server.mock("GET", "/m").expect(0).create_async().await;
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
        assert!(err.contains("disagree about block 100"), "{err}");
        manifest.assert_async().await;
        assert!(!pair_dir(tmp.path()).exists(), "nothing written");
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

    /// Final review M6: the pointer body is read up to the cap and no
    /// further. A server that says it is sending more is refused before the
    /// body is read, and one that streams more without saying so is cut off
    /// at the cap.
    #[tokio::test]
    async fn a_pointer_body_is_read_no_further_than_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/long")
            .with_body(vec![b' '; MAX_POINTER_BYTES + 1])
            .create_async()
            .await;
        server
            .mock("GET", "/stream")
            .with_chunked_body(|w| {
                for _ in 0..64 {
                    w.write_all(&[b' '; 1024])?;
                }
                Ok(())
            })
            .create_async()
            .await;
        let fetch = |path: &'static str| {
            let url = format!("{}{path}", server.url());
            let dir = tmp.path().to_path_buf();
            async move {
                prepare_confirmed(&reqwest::Client::new(), &url, &dir, &view(), None, any_url)
                    .await
                    .unwrap_err()
            }
        };
        let said = fetch("/long").await;
        assert!(
            said.contains(&format!("says {} bytes", MAX_POINTER_BYTES + 1)),
            "{said}"
        );
        let streamed = fetch("/stream").await;
        assert!(
            streamed.contains(&format!("more than {MAX_POINTER_BYTES} bytes")),
            "{streamed}"
        );
        assert!(!pair_dir(tmp.path()).exists(), "nothing written");
    }

    /// Final review M4: a validating node's mirror launch checked its pair
    /// before it launched, and its marker names that pair. When `latest`
    /// cannot be read at all (no answer, or a server error), the launch
    /// loads the confirmed pair its marker names from disk instead of losing
    /// its signed start (the loader checks it again in full). An answer (a
    /// 404, a dispute) decides as always, and so does a marker that names
    /// the pinned pair or a pair that is not on disk.
    #[tokio::test]
    async fn an_unreadable_pointer_falls_back_to_the_marked_pair_on_disk() {
        let env = format!("producer={P};confirmer={C}");
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/down")
            .with_status(503)
            .create_async()
            .await;
        server
            .mock("GET", "/none")
            .with_status(404)
            .with_body(r#"{"version":1,"confirmed":null}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/disputed")
            .with_body(r#"{"disputed":[100]}"#)
            .create_async()
            .await;
        let unreachable = "http://127.0.0.1:9/latest".to_string();
        let on_disk = |dir: &Path| {
            let (f, m) = pair_paths(dir, 100);
            std::fs::create_dir_all(pair_dir(dir)).unwrap();
            std::fs::write(&f, R_DAT).unwrap();
            std::fs::write(&m, R_PC).unwrap();
        };
        let marked = Some((PairKind::Confirmed, 100));
        let get = |url: String, dir: std::path::PathBuf, marked: Option<(PairKind, u64)>| {
            let env = env.clone();
            async move {
                prepare_confirmed_or_marked(
                    &reqwest::Client::new(),
                    &url,
                    &dir,
                    &view(),
                    Some(&env),
                    any_url,
                    marked,
                )
                .await
            }
        };
        for url in [unreachable.clone(), format!("{}/down", server.url())] {
            let tmp = tempfile::tempdir().unwrap();
            on_disk(tmp.path());
            let ready = get(url.clone(), tmp.path().to_path_buf(), marked)
                .await
                .unwrap();
            assert_eq!(
                (ready.kind, ready.height),
                (PairKind::Confirmed, 100),
                "{url}"
            );
            assert_eq!(ready.file, pair_paths(tmp.path(), 100).0);
            assert!(
                get(url.clone(), tmp.path().to_path_buf(), None)
                    .await
                    .is_err(),
                "no marker: {url}"
            );
            assert!(
                get(
                    url.clone(),
                    tmp.path().to_path_buf(),
                    Some((PairKind::Pinned, 100))
                )
                .await
                .is_err(),
                "the pinned pair has its own way back: {url}"
            );
            let empty = tempfile::tempdir().unwrap();
            assert!(
                get(url.clone(), empty.path().to_path_buf(), marked)
                    .await
                    .is_err(),
                "not on disk: {url}"
            );
        }
        for answered in ["/none", "/disputed"] {
            let tmp = tempfile::tempdir().unwrap();
            on_disk(tmp.path());
            assert!(
                get(
                    format!("{}{answered}", server.url()),
                    tmp.path().to_path_buf(),
                    marked
                )
                .await
                .is_err(),
                "{answered} is an answer"
            );
        }
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
