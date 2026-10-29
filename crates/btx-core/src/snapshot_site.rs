//! The app's side of the snapshot meeting point on easybtx.com (sections 6
//! and 6a of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md): a
//! producer sends its statement and file, a confirmer reads what waits and
//! sends back one signature or a dissent, and anyone can read `latest` and
//! the open disputes. The routes live in the EasyBTX repository
//! (`site/src/lib/snapshotRoutes.mjs`, the rules in `snapshotRendezvous.mjs`);
//! this is their client, matched to that code as it stands on 2026-10-05:
//!
//! ```text
//! POST /api/snapshots/statement                          a manifest, at most 64 KiB
//! POST /api/snapshots/file?statement=H&action=start
//! PUT  /api/snapshots/file?statement=H&upload=U&part=N   one part, at most 4 MiB
//! POST /api/snapshots/file?statement=H&upload=U&action=complete
//! GET  /api/snapshots/pending?chain=main
//! GET  /api/snapshots/latest?chain=main
//! GET  /api/snapshots/disputes?chain=main
//! ```
//!
//! Every write carries `x-ebtx-node: ebtx-snapshot-v1` (the routes answer 403
//! with no body without it) and an `application/octet-stream` body. A refusal
//! is `{"error": "<reason>"}` with 400, 404, 409, 413 or 422; 503 and 403
//! come with no body. A GET takes exactly `?chain=main` or `?chain=regtest`.
//!
//! NOTHING the website answers is trusted. A producer only learns whether its
//! upload was kept; a confirmer re-reads every statement from its bytes and
//! checks it against its own node (`crate::statement_check`) before it signs;
//! loading reads `latest` through `crate::attested_snapshot`, which checks
//! every signature and hash again. So every answer here is size-capped
//! before it is parsed, and parsed into plain data.
//!
//! NOTHING SECRET IS SENT. The website needs no credential: what authenticates
//! a statement is its signatures. The client keeps no cookies, follows no
//! redirect (a 307 would carry the body to another host), and sends only the
//! junk-filter header and the bytes it is given.
//!
//! WHERE. [`SITE`], or `EASYNODE_SNAPSHOT_SITE` when that is another `https://`
//! origin or `http://127.0.0.1:<port>` (the regtest rehearsal's stand-in,
//! `site/scripts/snapshot-rendezvous-local.mjs`). Any other value is an error,
//! never a silent fall back to the real website: a rehearsal with a typo in
//! its override must not post regtest statements to easybtx.com.
//!
//! WHY PARTS. A Vercel function takes a request body of about 4.5 MB, so the
//! file goes up in parts of at most 4 MiB, each through the same functions:
//! start, one PUT per part, complete. The website hashes the whole file at
//! complete and keeps it only if size and double SHA-256 are the statement's.

use crate::attested_snapshot::{self, Latest};
use crate::confirmed_snapshot::{self as cs, Statement};
use crate::operators::Chain;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// The website.
pub const SITE: &str = "https://easybtx.com";
/// The override, see the module doc.
pub const SITE_ENV: &str = "EASYNODE_SNAPSHOT_SITE";
/// A junk filter the routes require on every write. Not authentication.
pub const NODE_HEADER: &str = "x-ebtx-node";
pub const NODE_HEADER_VALUE: &str = "ebtx-snapshot-v1";
/// The website's part size (`PART_BYTES`), and the largest part it takes.
pub const MAX_PART_BYTES: u64 = 4 * 1024 * 1024;
/// The largest file the website stores (`MAX_FILE_BYTES`), the same as the
/// largest this app downloads.
pub const MAX_FILE_BYTES: u64 = attested_snapshot::MAX_SNAPSHOT_BYTES;
/// Per request. A 4 MiB part on a slow home uplink needs the room.
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// The website's refusal for a height the owner cleared (`CLOSED_HEIGHT`):
/// a code to match, not a sentence.
pub const CLOSED_HEIGHT: &str = "closed-height";
/// The website's refusal when another upload already stored the file. The
/// routes have no code for it, so this matches their sentence exactly
/// (`ALREADY_STORED` in snapshotRoutes.mjs).
pub const ALREADY_STORED: &str = "the file is already stored";

/// The cap on a reply to a write, and on any refusal's body. The website's
/// replies to writes are a few hundred bytes.
pub const MAX_REPLY_BYTES: usize = 64 * 1024;
/// The cap on `pending`: seven days of statements, about 70 grid heights,
/// each record carrying its merged manifest (at most 64 KiB, so 128 KiB as
/// hex; about 1 KiB with a few signatures).
pub const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;
/// The cap on `disputes`: a few hundred bytes per statement, plus the alert.
pub const MAX_DISPUTES_BYTES: usize = 1024 * 1024;

/// Where the routes are: an origin, no path. Only [`Site::parse`] makes one,
/// so every request goes to easybtx.com or an override that passed its rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site(String);

impl Default for Site {
    fn default() -> Self {
        Site(SITE.to_string())
    }
}

impl Site {
    /// [`SITE`] for `None` or a blank value; else `raw` when it is
    /// `https://<host>[:port]` or `http://127.0.0.1:<port>` with no user
    /// info, path, query or fragment (a trailing `/` is dropped). Anything
    /// else is an error naming the rule. `localhost` is refused: a name can
    /// resolve somewhere else, an address cannot.
    pub fn parse(raw: Option<&str>) -> Result<Site, String> {
        let Some(v) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(Site::default());
        };
        let refused = |why: &str| {
            Err(format!(
                "{SITE_ENV}={v:?} is not used: {why}. It takes an https:// origin \
                 or http://127.0.0.1:<port>"
            ))
        };
        let Ok(u) = reqwest::Url::parse(v) else {
            return refused("it is not a URL");
        };
        let loopback = u.host_str() == Some("127.0.0.1");
        match u.scheme() {
            "https" => {}
            "http" if loopback && u.port().is_some() => {}
            "http" => return refused("plain http only to 127.0.0.1 with a port"),
            _ => return refused("not https"),
        }
        if !u.username().is_empty() || u.password().is_some() {
            return refused("it carries user info");
        }
        if u.path() != "/" || u.query().is_some() || u.fragment().is_some() {
            return refused("it has a path, a query or a fragment");
        }
        // The origin as the URL parser wrote it (lowercase host, no default
        // port), never the raw text: what is checked is what is used.
        Ok(Site(u.origin().ascii_serialization()))
    }

    /// [`Site::parse`] of this process's `EASYNODE_SNAPSHOT_SITE`.
    pub fn from_env() -> Result<Site, String> {
        Site::parse(std::env::var(SITE_ENV).ok().as_deref())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The client for every route: [`TIMEOUT`], no redirects, no cookies.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("easynode-snapshot-site")
        .build()
        .map_err(|e| format!("snapshot site client: {e}"))
}

/// 32 lowercase hex characters, the routes' `upload` id. Checked before it
/// goes into a query, so an answer cannot add a parameter of its own.
fn is_upload_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The status and body of a sent request, the body refused past `cap`: by
/// its declared length at once, else as soon as the running total passes
/// it. A refusal's body is read under [`MAX_REPLY_BYTES`] whatever `cap` is,
/// and one past that is still a refusal, with no reason.
async fn read_capped(
    sent: Result<reqwest::Response, reqwest::Error>,
    cap: usize,
) -> Result<(u16, Vec<u8>), SiteError> {
    let mut resp = sent.map_err(|e| SiteError::Unreachable(e.to_string()))?;
    let status = resp.status().as_u16();
    let cap = if status == 200 {
        cap
    } else {
        cap.min(MAX_REPLY_BYTES)
    };
    let too_large = || {
        if status == 200 {
            Err(SiteError::Unreadable(format!(
                "an answer of more than {cap} bytes"
            )))
        } else {
            Ok((status, Vec::new()))
        }
    };
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return too_large();
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| SiteError::Unreachable(e.to_string()))?
    {
        if body.len() + chunk.len() > cap {
            return too_large();
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

/// A refusal: the status and the website's `error`, when the body is the
/// routes' `{"error": "..."}`.
fn refused(status: u16, body: &[u8]) -> SiteError {
    let reason = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_default();
    SiteError::Refused { status, reason }
}

/// A 200 parsed as `T`; any other status a [`SiteError::Refused`].
fn parse_200<T: serde::de::DeserializeOwned>(
    (status, body): (u16, Vec<u8>),
) -> Result<T, SiteError> {
    if status != 200 {
        return Err(refused(status, &body));
    }
    serde_json::from_slice(&body).map_err(|e| SiteError::Unreadable(e.to_string()))
}

/// [`read_capped`] under [`MAX_REPLY_BYTES`], then [`parse_200`]: every write.
async fn answer<T: serde::de::DeserializeOwned>(
    sent: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, SiteError> {
    parse_200(read_capped(sent, MAX_REPLY_BYTES).await?)
}

/// `main` or `regtest`, as the routes name chains.
pub fn chain_name(chain: Chain) -> &'static str {
    match chain {
        Chain::Main => "main",
        Chain::Regtest => "regtest",
    }
}

/// `POST /api/snapshots/statement`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatementReply {
    pub statement_hash: String,
    pub chain: String,
    pub height: u64,
    /// Every key on the stored statement, compressed hex.
    pub signers: Vec<String>,
    /// The operators behind them, under the website's copy of the list.
    pub operators: Vec<String>,
    /// Signatures this request added.
    pub added: u64,
    /// `missing`, `uploading`, `stored` or `dropped`.
    pub file: String,
}

/// One statement or dissent in `GET /api/snapshots/pending`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PendingStatement {
    pub statement_hash: String,
    pub height: u64,
    pub block_hash: String,
    /// The merged manifest, every signature the website took. The only
    /// field a confirmer acts on, after reading it with `cs::parse`.
    pub manifest_hex: String,
    pub signers: Vec<String>,
    pub operators: Vec<String>,
    pub file: String,
    pub first_seen: String,
    pub confirmed: bool,
    pub disputed: bool,
    /// The website's word for a statement with its four file fields zero.
    /// A confirmer reads the bytes and decides for itself.
    pub dissent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pending {
    pub version: u32,
    pub chain: String,
    /// Newest height first.
    pub statements: Vec<PendingStatement>,
}

/// `complete`'s answer: the website kept the file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileStored {
    pub stored: bool,
    pub file_url: String,
    /// Plain SHA-256 of the file, as the website computed it.
    pub file_sha256: String,
    /// The statement is confirmed now (two operators, a pinned key, the file).
    pub confirmed: bool,
}

/// What [`upload_file`] came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileUpload {
    Stored(FileStored),
    /// The website had the file already, from this or another upload.
    AlreadyStored,
}

/// One account of a disputed height, as `GET /api/snapshots/disputes` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DisputeStatement {
    pub statement_hash: String,
    pub block_hash: String,
    pub hash_serialized_3: String,
    pub coins: u64,
    pub chain_tx: u64,
    pub file_size: u64,
    pub file_hash: String,
    /// Operator NAMES here, not keys (the website's `recordOperators`).
    pub signers: Vec<String>,
    pub dissent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Dispute {
    pub height: u64,
    pub statements: Vec<DisputeStatement>,
    /// `block_hash`, `hash_serialized_3`, `coins`, `chain_tx`: the fields
    /// that differ.
    pub differ: Vec<String>,
    /// The owner's alert, as the website wrote it when the height was first
    /// disputed.
    pub alert: String,
    pub opened_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Disputes {
    pub disputes: Vec<Dispute>,
}

#[derive(Debug, Deserialize)]
struct UploadStart {
    upload: String,
    part_bytes: u64,
    parts: u64,
}

#[derive(Debug, Deserialize)]
struct PartTaken {
    part: u64,
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteError {
    /// Refused here, before anything was sent.
    NotSent(String),
    Unreachable(String),
    /// The website answered, and not with 200. `reason` is its `error`, or
    /// empty when it sent none (403, 503).
    Refused {
        status: u16,
        reason: String,
    },
    /// An answer too large, or not the shape the routes give.
    Unreadable(String),
}

impl SiteError {
    /// The owner cleared this height (section 6a): nothing at it is taken
    /// again, so there is no point retrying.
    pub fn is_closed_height(&self) -> bool {
        matches!(self, SiteError::Refused { status: 422, reason } if reason == CLOSED_HEIGHT)
    }
}

impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteError::NotSent(e) => write!(f, "not sent: {e}"),
            SiteError::Unreachable(e) => write!(f, "the snapshot site could not be reached ({e})"),
            SiteError::Refused { status, reason } if reason.is_empty() => {
                write!(f, "the snapshot site answered {status}")
            }
            SiteError::Refused { status, reason } => {
                write!(f, "the snapshot site answered {status}: {reason}")
            }
            SiteError::Unreadable(e) => write!(f, "the snapshot site's answer did not read: {e}"),
        }
    }
}

/// Send a manifest: a new statement or dissent, or co-signatures for one the
/// website has. Refused here when empty or over [`cs::MAX_MANIFEST_BYTES`].
pub async fn post_statement(
    client: &reqwest::Client,
    site: &Site,
    manifest: &[u8],
) -> Result<StatementReply, SiteError> {
    if manifest.is_empty() || manifest.len() > cs::MAX_MANIFEST_BYTES {
        return Err(SiteError::NotSent(format!(
            "a manifest of {} bytes; the website takes 1 to {}",
            manifest.len(),
            cs::MAX_MANIFEST_BYTES
        )));
    }
    let sent = client
        .post(format!("{}/api/snapshots/statement", site.as_str()))
        .header(NODE_HEADER, NODE_HEADER_VALUE)
        .header("content-type", "application/octet-stream")
        .body(manifest.to_vec())
        .send()
        .await;
    answer(sent).await
}

/// The website already has the file: [`ALREADY_STORED`], at any step.
fn already_stored(e: &SiteError) -> bool {
    matches!(e, SiteError::Refused { status: 409, reason } if reason == ALREADY_STORED)
}

/// A POST to the file route with no body: `start` or `complete`.
async fn file_action<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: String,
) -> Result<T, SiteError> {
    let sent = client
        .post(url)
        .header(NODE_HEADER, NODE_HEADER_VALUE)
        .header("content-type", "application/octet-stream")
        .send()
        .await;
    answer(sent).await
}

/// Upload the file of `statement`, which the website holds, in the parts it
/// asks for. Refused here, before anything is sent, when the file's size is
/// not the statement's or is over [`MAX_FILE_BYTES`], and before any part is
/// sent when the website's part plan is not one for this file.
pub async fn upload_file(
    client: &reqwest::Client,
    site: &Site,
    statement: &Statement,
    file: &Path,
) -> Result<FileUpload, SiteError> {
    match upload_parts(client, site, statement, file).await {
        Err(e) if already_stored(&e) => Ok(FileUpload::AlreadyStored),
        other => other,
    }
}

async fn upload_parts(
    client: &reqwest::Client,
    site: &Site,
    statement: &Statement,
    file: &Path,
) -> Result<FileUpload, SiteError> {
    let hash = statement.hash().display_hex();
    let not_sent = |e: String| SiteError::NotSent(format!("{}: {e}", file.display()));
    let len = std::fs::metadata(file)
        .map_err(|e| not_sent(e.to_string()))?
        .len();
    if len != statement.file_size() {
        return Err(not_sent(format!(
            "{len} bytes, the statement says {}",
            statement.file_size()
        )));
    }
    if len == 0 || len > MAX_FILE_BYTES {
        return Err(not_sent(format!(
            "{len} bytes; the website stores 1 to {MAX_FILE_BYTES}"
        )));
    }
    let url = format!("{}/api/snapshots/file", site.as_str());

    let start: UploadStart =
        file_action(client, format!("{url}?statement={hash}&action=start")).await?;
    // The website plans parts from the statement's file size; this file has
    // that size, so its plan must be this one exactly.
    let plan_ok = start.part_bytes > 0
        && start.part_bytes <= MAX_PART_BYTES
        && start.parts == len.div_ceil(start.part_bytes).max(1);
    if !is_upload_id(&start.upload) || !plan_ok {
        return Err(SiteError::Unreadable(format!(
            "upload {:?} of {} parts of {} bytes, for a {len}-byte file",
            start.upload, start.parts, start.part_bytes
        )));
    }

    let mut f = tokio::fs::File::open(file)
        .await
        .map_err(|e| not_sent(e.to_string()))?;
    for n in 1..=start.parts {
        let size = start.part_bytes.min(len - (n - 1) * start.part_bytes);
        let mut part = vec![0u8; size as usize];
        f.read_exact(&mut part)
            .await
            .map_err(|e| not_sent(e.to_string()))?;
        let sent = client
            .put(format!(
                "{url}?statement={hash}&upload={}&part={n}",
                start.upload
            ))
            .header(NODE_HEADER, NODE_HEADER_VALUE)
            .header("content-type", "application/octet-stream")
            .body(part)
            .send()
            .await;
        let taken: PartTaken = answer(sent).await?;
        if taken.part != n || taken.bytes != size {
            return Err(SiteError::Unreadable(format!(
                "part {n} of {size} bytes was taken as part {} of {} bytes",
                taken.part, taken.bytes
            )));
        }
    }

    let stored: FileStored = file_action(
        client,
        format!(
            "{url}?statement={hash}&upload={}&action=complete",
            start.upload
        ),
    )
    .await?;
    if !stored.stored {
        return Err(SiteError::Unreadable(
            "complete answered without storing the file".into(),
        ));
    }
    Ok(FileUpload::Stored(stored))
}

/// A GET of one of the read routes for `chain`, with its exact query.
async fn get_route(
    client: &reqwest::Client,
    site: &Site,
    route: &str,
    chain: Chain,
    cap: usize,
) -> Result<(u16, Vec<u8>), SiteError> {
    let sent = client
        .get(format!(
            "{}/api/snapshots/{route}?chain={}",
            site.as_str(),
            chain_name(chain)
        ))
        .send()
        .await;
    read_capped(sent, cap).await
}

/// What waits for confirmers on `chain`.
pub async fn get_pending(
    client: &reqwest::Client,
    site: &Site,
    chain: Chain,
) -> Result<Pending, SiteError> {
    parse_200(get_route(client, site, "pending", chain, MAX_PENDING_BYTES).await?)
}

/// `latest` on `chain`, read as `crate::attested_snapshot::parse_latest`
/// reads it. `Ok(None)` for the website's 404: nothing is confirmed.
pub async fn get_latest(
    client: &reqwest::Client,
    site: &Site,
    chain: Chain,
) -> Result<Option<Latest>, SiteError> {
    let (status, body) = get_route(
        client,
        site,
        "latest",
        chain,
        attested_snapshot::MAX_POINTER_BYTES,
    )
    .await?;
    match status {
        200 => attested_snapshot::parse_latest(&body)
            .map(Some)
            .map_err(SiteError::Unreadable),
        404 => Ok(None),
        _ => Err(refused(status, &body)),
    }
}

/// The open disputes on `chain`, oldest height first.
pub async fn get_disputes(
    client: &reqwest::Client,
    site: &Site,
    chain: Chain,
) -> Result<Disputes, SiteError> {
    parse_200(get_route(client, site, "disputes", chain, MAX_DISPUTES_BYTES).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const LATEST: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/latest.json");
    /// The spike's regtest statement at 100.
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";
    const UPLOAD: &str = "0123456789abcdef0123456789abcdef";

    fn statement() -> Statement {
        cs::parse(R_P).unwrap().statement
    }

    fn site(server: &mockito::Server) -> Site {
        Site::parse(Some(&server.url())).unwrap()
    }

    fn q(pairs: &[(&str, &str)]) -> Matcher {
        Matcher::AllOf(
            pairs
                .iter()
                .map(|(k, v)| Matcher::UrlEncoded(k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// Nothing that could be a credential goes with a request.
    fn bare(m: mockito::Mock) -> mockito::Mock {
        m.match_header("authorization", Matcher::Missing)
            .match_header("cookie", Matcher::Missing)
            .match_header("proxy-authorization", Matcher::Missing)
    }

    /// The website's own numbers and words (snapshotRoutes.mjs,
    /// snapshotManifest.mjs, snapshotRendezvous.mjs on 2026-10-05), written
    /// out so a changed constant here cannot pass against itself.
    #[test]
    fn the_limits_and_words_are_the_websites() {
        assert_eq!(
            (NODE_HEADER, NODE_HEADER_VALUE),
            ("x-ebtx-node", "ebtx-snapshot-v1")
        );
        assert_eq!(MAX_PART_BYTES, 4_194_304);
        assert_eq!(MAX_FILE_BYTES, 67_108_864);
        assert_eq!(cs::MAX_MANIFEST_BYTES, 65_536);
        assert_eq!(CLOSED_HEIGHT, "closed-height");
        assert_eq!(ALREADY_STORED, "the file is already stored");
        assert_eq!(chain_name(Chain::Main), "main");
        assert_eq!(chain_name(Chain::Regtest), "regtest");
    }

    #[test]
    fn the_site_is_easybtx_unless_a_safe_override_says_otherwise() {
        assert_eq!(Site::parse(None).unwrap().as_str(), SITE);
        assert_eq!(Site::parse(Some("  ")).unwrap().as_str(), SITE);
        assert_eq!(Site::default().as_str(), "https://easybtx.com");
        assert_eq!(SITE_ENV, "EASYNODE_SNAPSHOT_SITE");
        for (raw, want) in [
            ("http://127.0.0.1:29650/", "http://127.0.0.1:29650"),
            ("http://127.0.0.1:29650", "http://127.0.0.1:29650"),
            ("https://preview.vercel.app", "https://preview.vercel.app"),
            (
                "https://preview.vercel.app:8443/",
                "https://preview.vercel.app:8443",
            ),
        ] {
            assert_eq!(Site::parse(Some(raw)).unwrap().as_str(), want, "{raw}");
        }
        for refused in [
            "http://example.com",
            "http://127.0.0.2:29650",
            "http://localhost:29650",
            "http://127.0.0.1",
            "http://[::1]:29650",
            "ftp://127.0.0.1:21",
            "https://easybtx.com/api",
            "https://easybtx.com/?x=1",
            "https://easybtx.com/#f",
            "https://user:pw@easybtx.com",
            "https://user@easybtx.com",
            "not a url",
        ] {
            assert!(Site::parse(Some(refused)).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn a_statement_goes_as_bytes_with_the_node_header_and_nothing_else() {
        let mut server = mockito::Server::new_async().await;
        let m = bare(server.mock("POST", "/api/snapshots/statement"))
            .match_header("x-ebtx-node", "ebtx-snapshot-v1")
            .match_header("content-type", "application/octet-stream")
            .match_body(R_P.to_vec())
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":["03ab"],"operators":["producer"],"added":1,"file":"missing"}}"#
            ))
            .create_async()
            .await;
        let reply = post_statement(&client().unwrap(), &site(&server), R_P)
            .await
            .unwrap();
        m.assert_async().await;
        assert_eq!(
            reply,
            StatementReply {
                statement_hash: H.into(),
                chain: "regtest".into(),
                height: 100,
                signers: vec!["03ab".into()],
                operators: vec!["producer".into()],
                added: 1,
                file: "missing".into(),
            }
        );
    }

    #[tokio::test]
    async fn a_manifest_the_website_would_refuse_for_its_size_is_not_sent() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let c = client().unwrap();
        for body in [vec![], vec![0u8; cs::MAX_MANIFEST_BYTES + 1]] {
            let err = post_statement(&c, &site(&server), &body).await.unwrap_err();
            assert!(matches!(err, SiteError::NotSent(_)), "{err}");
        }
        m.assert_async().await;
        // Exactly the cap goes (and the website decides).
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"the manifest ends early"}"#)
            .create_async()
            .await;
        let at_cap = vec![0u8; cs::MAX_MANIFEST_BYTES];
        assert!(matches!(
            post_statement(&c, &site(&server), &at_cap).await,
            Err(SiteError::Refused { status: 422, .. })
        ));
    }

    /// A refusal carries the website's own reason; one with no body (403
    /// without the header, 503 when its store is down) carries none; the
    /// owner's clear is recognised by its code.
    #[tokio::test]
    async fn a_refusal_carries_the_websites_reason() {
        let c = client().unwrap();
        for (status, body, reason) in [
            (
                422,
                r#"{"error":"height 150 is not a positive multiple of 100"}"#,
                "height 150 is not a positive multiple of 100",
            ),
            (
                413,
                r#"{"error":"at most 65536 bytes"}"#,
                "at most 65536 bytes",
            ),
            (403, "", ""),
            (503, "", ""),
            (500, "<html>oops</html>", ""),
        ] {
            let mut server = mockito::Server::new_async().await;
            server
                .mock("POST", "/api/snapshots/statement")
                .with_status(status)
                .with_body(body)
                .create_async()
                .await;
            let err = post_statement(&c, &site(&server), R_P).await.unwrap_err();
            assert_eq!(
                err,
                SiteError::Refused {
                    status: status as u16,
                    reason: reason.into()
                }
            );
            assert!(!err.is_closed_height());
        }
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"closed-height"}"#)
            .create_async()
            .await;
        let err = post_statement(&c, &site(&server), R_P).await.unwrap_err();
        assert!(err.is_closed_height(), "{err}");
        let nobody = Site::parse(Some("http://127.0.0.1:9")).unwrap();
        let err = post_statement(&c, &nobody, R_P).await.unwrap_err();
        assert!(matches!(err, SiteError::Unreachable(_)), "{err}");
    }

    /// A 307 would carry the manifest to another host: it is an answer, not
    /// an address to follow.
    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let mut elsewhere = mockito::Server::new_async().await;
        let never = elsewhere
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(307)
            .with_header(
                "location",
                &format!("{}/api/snapshots/statement", elsewhere.url()),
            )
            .create_async()
            .await;
        let err = post_statement(&client().unwrap(), &site(&server), R_P)
            .await
            .unwrap_err();
        assert!(
            matches!(err, SiteError::Refused { status: 307, .. }),
            "{err}"
        );
        never.assert_async().await;
    }

    /// Every answer is capped before it is parsed.
    #[tokio::test]
    async fn an_oversized_answer_is_not_read() {
        let c = client().unwrap();
        let mut server = mockito::Server::new_async().await;
        let padding = " ".repeat(MAX_REPLY_BYTES);
        server
            .mock("POST", "/api/snapshots/statement")
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":[],"operators":[],"added":0,"file":"missing"}}{padding}"#
            ))
            .create_async()
            .await;
        let err = post_statement(&c, &site(&server), R_P).await.unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(format!(
                r#"{{"version":1,"chain":"regtest","statements":[]}}{}"#,
                " ".repeat(MAX_PENDING_BYTES)
            ))
            .create_async()
            .await;
        let err = get_pending(&c, &site(&server), Chain::Regtest)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
        server
            .mock("GET", "/api/snapshots/latest?chain=main")
            .with_body(format!(
                "{}{}",
                String::from_utf8_lossy(LATEST),
                " ".repeat(attested_snapshot::MAX_POINTER_BYTES)
            ))
            .create_async()
            .await;
        let err = get_latest(&c, &site(&server), Chain::Main)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
        // A refusal's body is capped too, and then carries no reason.
        server
            .mock("GET", "/api/snapshots/disputes?chain=main")
            .with_status(400)
            .with_body(format!(r#"{{"error":"x"}}{padding}"#))
            .create_async()
            .await;
        let err = get_disputes(&c, &site(&server), Chain::Main)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            SiteError::Refused {
                status: 400,
                reason: String::new()
            }
        );
    }

    /// An answer with no declared length (chunked) is capped while it
    /// streams, not after.
    #[tokio::test]
    async fn an_oversized_answer_with_no_declared_length_is_not_read() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_chunked_body(|w| {
                w.write_all(br#"{"statement_hash":"x","chain":"regtest","height":100,"signers":[],"operators":[],"added":0,"file":"missing"}"#)?;
                for _ in 0..(MAX_REPLY_BYTES / 1024 + 1) {
                    w.write_all(&[b' '; 1024])?;
                }
                Ok(())
            })
            .create_async()
            .await;
        let err = post_statement(&client().unwrap(), &site(&server), R_P)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
    }

    /// `complete` that did not store the file is not a stored file.
    #[tokio::test]
    async fn a_complete_that_stored_nothing_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "start")]))
            .with_body(format!(
                r#"{{"upload":"{UPLOAD}","part_bytes":8055,"parts":1}}"#
            ))
            .create_async()
            .await;
        server
            .mock("PUT", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_body(r#"{"part":1,"bytes":8055}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "complete")]))
            .with_body(r#"{"stored":false,"file_url":"u","file_sha256":"s","confirmed":false}"#)
            .create_async()
            .await;
        let err = upload_file(&client().unwrap(), &site(&server), &statement(), &file)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
    }

    #[tokio::test]
    async fn the_file_goes_up_in_the_parts_the_website_asks_for() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        assert_eq!(statement().file_size(), R_DAT.len() as u64);
        let mut server = mockito::Server::new_async().await;
        let start = bare(server.mock("POST", "/api/snapshots/file"))
            .match_query(q(&[("statement", H), ("action", "start")]))
            .match_header("x-ebtx-node", "ebtx-snapshot-v1")
            .with_body(format!(
                r#"{{"upload":"{UPLOAD}","part_bytes":3000,"parts":3}}"#
            ))
            .create_async()
            .await;
        let mut parts = Vec::new();
        for (n, range) in [(1, 0..3000), (2, 3000..6000), (3, 6000..8055)] {
            parts.push(
                bare(server.mock("PUT", "/api/snapshots/file"))
                    .match_query(q(&[
                        ("statement", H),
                        ("upload", UPLOAD),
                        ("part", &n.to_string()),
                    ]))
                    .match_header("x-ebtx-node", "ebtx-snapshot-v1")
                    .match_header("content-type", "application/octet-stream")
                    .match_body(R_DAT[range.clone()].to_vec())
                    .with_body(format!(r#"{{"part":{n},"bytes":{}}}"#, range.len()))
                    .create_async()
                    .await,
            );
        }
        let complete = bare(server.mock("POST", "/api/snapshots/file"))
            .match_query(q(&[
                ("statement", H),
                ("upload", UPLOAD),
                ("action", "complete"),
            ]))
            .match_header("x-ebtx-node", "ebtx-snapshot-v1")
            .with_body(r#"{"stored":true,"file_url":"https://x.public.blob.vercel-storage.com/f.dat","file_sha256":"b2c5","confirmed":false}"#)
            .create_async()
            .await;
        let got = upload_file(&client().unwrap(), &site(&server), &statement(), &file)
            .await
            .unwrap();
        start.assert_async().await;
        for p in parts {
            p.assert_async().await;
        }
        complete.assert_async().await;
        assert_eq!(
            got,
            FileUpload::Stored(FileStored {
                stored: true,
                file_url: "https://x.public.blob.vercel-storage.com/f.dat".into(),
                file_sha256: "b2c5".into(),
                confirmed: false,
            })
        );
    }

    /// One part of the website's own 4 MiB: the whole file in one PUT.
    #[tokio::test]
    async fn a_small_file_goes_in_one_part() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "start")]))
            .with_body(format!(
                r#"{{"upload":"{UPLOAD}","part_bytes":4194304,"parts":1}}"#
            ))
            .create_async()
            .await;
        let put = server
            .mock("PUT", "/api/snapshots/file")
            .match_query(q(&[("part", "1")]))
            .match_body(R_DAT.to_vec())
            .with_body(r#"{"part":1,"bytes":8055}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "complete")]))
            .with_body(r#"{"stored":true,"file_url":"u","file_sha256":"s","confirmed":true}"#)
            .create_async()
            .await;
        let got = upload_file(&client().unwrap(), &site(&server), &statement(), &file)
            .await
            .unwrap();
        put.assert_async().await;
        assert!(matches!(
            got,
            FileUpload::Stored(FileStored {
                confirmed: true,
                ..
            })
        ));
    }

    /// The website had the file: at start, or because another upload won
    /// while this one ran. A file it let go (also a 409) is a refusal.
    #[tokio::test]
    async fn a_file_already_stored_is_not_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let c = client().unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_status(409)
            .with_body(r#"{"error":"the file is already stored"}"#)
            .create_async()
            .await;
        let no_put = server
            .mock("PUT", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        assert_eq!(
            upload_file(&c, &site(&server), &statement(), &file)
                .await
                .unwrap(),
            FileUpload::AlreadyStored
        );
        no_put.assert_async().await;

        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "start")]))
            .with_body(format!(
                r#"{{"upload":"{UPLOAD}","part_bytes":8055,"parts":1}}"#
            ))
            .create_async()
            .await;
        server
            .mock("PUT", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_status(409)
            .with_body(r#"{"error":"the file is already stored"}"#)
            .create_async()
            .await;
        assert_eq!(
            upload_file(&c, &site(&server), &statement(), &file)
                .await
                .unwrap(),
            FileUpload::AlreadyStored
        );

        let mut server = mockito::Server::new_async().await;
        let dropped = "this file was let go to keep storage small: the website keeps files only for the newest statements";
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_status(409)
            .with_body(format!(r#"{{"error":"{dropped}"}}"#))
            .create_async()
            .await;
        assert_eq!(
            upload_file(&c, &site(&server), &statement(), &file).await,
            Err(SiteError::Refused {
                status: 409,
                reason: dropped.into()
            })
        );
    }

    /// A plan that is not one for this file, or an upload id that is not the
    /// website's shape, stops before a single part goes.
    #[tokio::test]
    async fn an_odd_part_plan_is_refused_before_any_part_goes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let c = client().unwrap();
        let big = 4 * 1024 * 1024 + 1;
        for plan in [
            format!(r#"{{"upload":"{UPLOAD}","part_bytes":{big},"parts":1}}"#),
            format!(r#"{{"upload":"{UPLOAD}","part_bytes":0,"parts":1}}"#),
            format!(r#"{{"upload":"{UPLOAD}","part_bytes":3000,"parts":2}}"#),
            format!(r#"{{"upload":"{UPLOAD}","part_bytes":3000,"parts":4}}"#),
            r#"{"upload":"u&part=9","part_bytes":3000,"parts":3}"#.to_string(),
            r#"{"upload":"0123456789ABCDEF0123456789ABCDEF","part_bytes":3000,"parts":3}"#
                .to_string(),
            r#"{"part_bytes":3000,"parts":3}"#.to_string(),
        ] {
            let mut server = mockito::Server::new_async().await;
            server
                .mock("POST", "/api/snapshots/file")
                .match_query(Matcher::Any)
                .with_body(&plan)
                .create_async()
                .await;
            let no_put = server
                .mock("PUT", Matcher::Any)
                .expect(0)
                .create_async()
                .await;
            let err = upload_file(&c, &site(&server), &statement(), &file)
                .await
                .unwrap_err();
            assert!(matches!(err, SiteError::Unreadable(_)), "{plan}: {err}");
            no_put.assert_async().await;
        }
    }

    /// A part the website did not take as sent stops the upload.
    #[tokio::test]
    async fn a_part_answer_for_another_part_or_size_stops_the_upload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "start")]))
            .with_body(format!(
                r#"{{"upload":"{UPLOAD}","part_bytes":8055,"parts":1}}"#
            ))
            .create_async()
            .await;
        server
            .mock("PUT", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_body(r#"{"part":1,"bytes":8000}"#)
            .create_async()
            .await;
        let complete = server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("action", "complete")]))
            .expect(0)
            .create_async()
            .await;
        let err = upload_file(&client().unwrap(), &site(&server), &statement(), &file)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
        complete.assert_async().await;
    }

    /// A file that is not the statement's size, or is larger than the website
    /// stores, is not offered at all.
    #[tokio::test]
    async fn a_file_that_is_not_the_statements_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, &R_DAT[..8000]).unwrap();
        let mut server = mockito::Server::new_async().await;
        let no_post = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let no_put = server
            .mock("PUT", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let c = client().unwrap();
        let err = upload_file(&c, &site(&server), &statement(), &file)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::NotSent(_)), "{err}");
        let missing = dir.path().join("gone.dat");
        let err = upload_file(&c, &site(&server), &statement(), &missing)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::NotSent(_)), "{err}");
        // A file of the statement's size, but over the website's 64 MiB
        // (sparse, so the test writes nothing).
        let huge = dir.path().join("huge.dat");
        let over = 64 * 1024 * 1024 + 1;
        std::fs::File::create(&huge).unwrap().set_len(over).unwrap();
        let mut raw = *statement().raw();
        raw[181..189].copy_from_slice(&over.to_le_bytes());
        let err = upload_file(&c, &site(&server), &Statement::from_raw(raw), &huge)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::NotSent(_)), "{err}");
        no_post.assert_async().await;
        no_put.assert_async().await;
    }

    #[tokio::test]
    async fn pending_reads_the_websites_list() {
        let mut server = mockito::Server::new_async().await;
        let m = bare(server.mock("GET", "/api/snapshots/pending?chain=regtest"))
            .with_header("cache-control", "public, max-age=30, s-maxage=30")
            .with_body(format!(
                r#"{{"version":1,"chain":"regtest","statements":[{{"statement_hash":"{H}","height":100,"block_hash":"bd23","manifest_hex":"02","signers":["03ab"],"operators":["producer"],"file":"stored","first_seen":"2026-10-01T12:00:00.000Z","confirmed":false,"disputed":false,"dissent":true}}]}}"#
            ))
            .create_async()
            .await;
        let p = get_pending(&client().unwrap(), &site(&server), Chain::Regtest)
            .await
            .unwrap();
        m.assert_async().await;
        assert_eq!((p.version, p.chain.as_str()), (1, "regtest"));
        let s = &p.statements[0];
        assert_eq!(
            (s.height, s.operators.clone(), s.dissent, s.file.as_str()),
            (100, vec!["producer".to_string()], true, "stored")
        );
    }

    /// `latest`: a pointer, the dispute shape, or the website's 404.
    #[tokio::test]
    async fn latest_reads_a_pointer_a_dispute_or_nothing() {
        let c = client().unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/latest?chain=main")
            .with_body(LATEST)
            .create_async()
            .await;
        let got = get_latest(&c, &site(&server), Chain::Main).await.unwrap();
        assert!(
            matches!(&got, Some(Latest::Confirmed(p)) if p.height == 232_000),
            "{got:?}"
        );
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/latest?chain=main")
            .with_body(r#"{"disputed":[233800]}"#)
            .create_async()
            .await;
        assert_eq!(
            get_latest(&c, &site(&server), Chain::Main).await.unwrap(),
            Some(Latest::Disputed(vec![233_800]))
        );
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/latest?chain=regtest")
            .with_status(404)
            .with_body(r#"{"version":1,"confirmed":null}"#)
            .create_async()
            .await;
        assert_eq!(
            get_latest(&c, &site(&server), Chain::Regtest)
                .await
                .unwrap(),
            None
        );
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/latest?chain=main")
            .with_body(r#"{"disputed":[]}"#)
            .create_async()
            .await;
        assert!(matches!(
            get_latest(&c, &site(&server), Chain::Main).await,
            Err(SiteError::Unreadable(_))
        ));
    }

    #[tokio::test]
    async fn disputes_read_both_accounts_and_the_alert() {
        let mut server = mockito::Server::new_async().await;
        let account = |hash: &str, utxo: &str, who: &str, dissent: bool| {
            format!(
                r#"{{"statement_hash":"{hash}","block_hash":"bd23","hash_serialized_3":"{utxo}","coins":101,"chain_tx":101,"file_size":0,"file_hash":"00","signers":["{who}"],"dissent":{dissent}}}"#
            )
        };
        server
            .mock("GET", "/api/snapshots/disputes?chain=regtest")
            .with_body(format!(
                r#"{{"disputes":[{{"height":100,"statements":[{},{}],"differ":["hash_serialized_3"],"alert":"easyNode snapshots: block 100 is disputed.","opened_at":"2026-10-01T12:00:00.000Z"}}]}}"#,
                account(H, "e611", "producer", false),
                account(&"22".repeat(32), "2222", "confirmer", true)
            ))
            .create_async()
            .await;
        let d = get_disputes(&client().unwrap(), &site(&server), Chain::Regtest)
            .await
            .unwrap();
        let one = &d.disputes[0];
        assert_eq!(
            (one.height, one.differ.clone(), one.statements.len()),
            (100, vec!["hash_serialized_3".to_string()], 2)
        );
        assert!(one.statements[1].dissent);
        assert_eq!(one.statements[1].signers, vec!["confirmer".to_string()]);
        assert!(one.alert.starts_with("easyNode snapshots: block 100"));
    }
}
