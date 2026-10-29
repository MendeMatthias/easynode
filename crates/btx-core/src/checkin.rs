//! Node check-in: telling the directory this node exists, over HTTPS.
//!
//! [`service_report`](crate::service_report) writes what this node has been
//! doing to a local file and says, in its own module doc, that publishing it
//! anywhere is "a separate, explicit, future feature with its own consent".
//! This is that feature, and the consent is the point: it is **off unless the
//! operator turns it on**, and nothing here runs otherwise.
//!
//! # What actually turns it on, since 0.6.26
//!
//! One thing: **choosing to sign confirmations for mirrors**
//! ([`crate::signer`]). A node that does not sign never sends a check-in, and
//! the app has no other caller for this module.
//!
//! That is not a coincidence of scheduling, it is the reason the feature
//! shipped. A signing key that nobody pins signs into the void, so a volunteer
//! has to get 66 public characters to whoever runs a mirror. Until 0.6.26 that
//! meant copying them out of a settings panel and pasting them into a chat with
//! Mende, once per volunteer, forever — which does not scale past the people
//! who already know him, and the network needs the opposite of that. On
//! 2026-09-16 every mirror on BTX was following ONE key on ONE home computer;
//! at 15:23Z it was switched off and the explorer froze at 221,448 while the
//! chain went on.
//!
//! So a signing node offers its public key here, `easybtx.com` collects them,
//! and `/api/signer-offers` is the list a mirror operator pins from. Offering
//! is not pinning: the trust decision stays a human one, taken on the mirror's
//! own machine, and nothing in this file can make it.
//!
//! # Why a node needs to be able to say "I am here"
//!
//! The node directory finds nodes by dialling them over P2P. That only ever
//! sees machines with an inbound-reachable port, which excludes the large
//! majority of home machines behind NAT. Somebody can run this app every day
//! for a month and appear nowhere, which is both discouraging and a measurement
//! problem: the fleet is invisible to the people counting it.
//!
//! A check-in is one small HTTPS POST that closes that gap.
//!
//! # What a check-in is NOT
//!
//! It is a **claim**, not evidence. Anybody can POST one, so the receiving end
//! counts check-ins separately and never folds them into the measured
//! "live" / "at tip" figures. That is not a limitation to work around later; it
//! is the honest design, and the same distinction as service bit 27: a node
//! saying it validates is not the same as a node having been observed to.
//!
//! Where a check-in genuinely helps beyond visibility: if the node reports an
//! inbound port, the directory's existing prober can dial it and turn the claim
//! into a measurement. Discovery from the check-in, verification from the probe.
//!
//! # What is sent, exactly
//!
//! The struct below is the whole payload and the whole of it is operational:
//! counters, a version string, service bits, and a self-generated random id.
//!
//! There is **no wallet, no address, no balance, no SECRET key, no username,
//! and no machine fingerprint**. The id is random bytes generated once and kept
//! in the datadir; it identifies the same node across restarts and nothing
//! else, and deleting the file gives the node a new identity with no
//! consequence.
//!
//! The one key it carries is `signer_pubkey`, the PUBLIC half of the signing
//! key, and only when the operator has turned signing on. It is public by
//! construction: every confirmation this node signs already carries it to every
//! peer on the network. The private key never leaves the datadir and nothing in
//! this crate reads it except the engine's own key file loader.
//!
//! The server sees the source IP, as it must for any HTTP request. It stores
//! only a salted hash of it plus a coarse location, the same model the peer map
//! already uses.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Schema of the payload below. Must match the receiving endpoint.
///
/// 2 adds `signer_pubkey`. The endpoint accepts exactly this value and refuses
/// 1, which costs nothing: this client existed from 0.6.17 with no caller, so
/// no released app ever sent a schema 1 check-in.
pub const CHECKIN_SCHEMA: u32 = 2;

/// Where a check-in goes. `EASYBTX_CHECKIN_ENDPOINT` overrides it, which is how
/// the end-to-end test on the release box points a real node at a local
/// receiver instead of the live site.
pub const CHECKIN_ENDPOINT: &str = "https://easybtx.com/api/node-checkin";

/// The endpoint this run will use.
pub fn checkin_endpoint() -> String {
    match std::env::var("EASYBTX_CHECKIN_ENDPOINT") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => CHECKIN_ENDPOINT.to_string(),
    }
}

/// Where the node's self-generated id lives, inside the datadir.
pub const NODE_ID_FILE: &str = "node-id";

/// Don't check in more often than this. The endpoint rejects anything faster,
/// and hammering a service that is doing us a favour is not how this project
/// behaves. Fifteen minutes gives a useful liveness signal at negligible cost.
pub const CHECKIN_INTERVAL_SECS: u64 = 900;

/// The exact body the endpoint accepts. Field names are snake_case on the wire
/// and the receiver rejects any key it does not know, so this struct and the
/// server's validator have to agree exactly. That strictness is deliberate on
/// both sides: an intake that silently accepts unknown fields is one that will
/// eventually store something it should not.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Checkin {
    pub schema: u32,
    /// 32 lowercase hex chars. Random, local, rotatable.
    pub node_id: String,
    /// e.g. "easynode/0.6.17".
    pub agent: String,
    /// The engine this node runs, e.g. "v0.34.5".
    pub btxd_version: Option<String>,
    pub uptime_secs: u64,
    pub blocks: u64,
    pub headers: u64,
    pub peers: i64,
    pub bytes_sent: Option<u64>,
    /// `localservices` exactly as `getnetworkinfo` prints it: 16 lowercase hex.
    /// This is what carries bit 27, so it is the most useful field here.
    pub services: String,
    pub trusted_mirror: bool,
    pub serving_attestations: bool,
    /// Set only when this node accepts inbound connections. Telling the
    /// directory where to dial is what lets it verify the claim rather than
    /// take it, so a reachable node should populate it.
    pub listening_port: Option<u16>,
    /// The PUBLIC signing key this node's confirmations carry, 66 lowercase
    /// hex, or `None` on a node that does not sign — which is every node until
    /// its operator turns signing on.
    ///
    /// This is the whole reason the app calls this module. See the header, and
    /// [`crate::signer`] for where the key comes from.
    pub signer_pubkey: Option<String>,
}

/// The engine version in the shape the endpoint accepts (`v0.34.6`), from the
/// release tag the app pins (`v0.34.6-3013c2c2`).
///
/// The receiver takes `^v?[0-9][0-9.]{0,15}$` and silently nulls anything else,
/// so sending the raw tag would quietly publish a fleet with no engine version
/// at all. Everything from the first `-` is a commit pin, which is ours and not
/// upstream's version, so it is dropped rather than mangled.
pub fn engine_version_for_checkin(tag: &str) -> Option<String> {
    let head = tag.trim().split('-').next()?.trim();
    let digits = head.strip_prefix('v').unwrap_or(head);
    let ok = !digits.is_empty()
        && digits.len() <= 16
        && digits.starts_with(|c: char| c.is_ascii_digit())
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    ok.then(|| head.to_string())
}

/// Format 16 random bytes as the 32-hex id the endpoint requires.
fn format_id(bytes: [u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// True for a well-formed id: 32 lowercase hex characters.
pub fn is_valid_node_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn node_id_path(datadir: &Path) -> PathBuf {
    datadir.join(NODE_ID_FILE)
}

/// Read the node's id, generating and persisting one on first call.
///
/// A malformed or truncated file is replaced rather than trusted: a half-written
/// id would be rejected by the endpoint on every single check-in forever, which
/// is a silent permanent failure and the worst outcome available here.
pub fn load_or_create_node_id(datadir: &Path, random: [u8; 16]) -> AppResult<String> {
    let path = node_id_path(datadir);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if is_valid_node_id(&trimmed) {
            return Ok(trimmed);
        }
    }
    let id = format_id(random);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Process(format!("create datadir for node id: {e}")))?;
    }
    // tmp + rename so a crash mid-write cannot leave a torn id behind, matching
    // how service_report writes.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &id).map_err(|e| AppError::Process(format!("write node id: {e}")))?;
    std::fs::rename(&tmp, &path).map_err(|e| AppError::Process(format!("rename node id: {e}")))?;
    Ok(id)
}

/// [`load_or_create_node_id`] with the randomness taken from the operating
/// system, which is what every caller outside a test wants. Kept separate so
/// the id logic stays deterministic and testable.
pub fn load_or_create_node_id_os(datadir: &Path) -> AppResult<String> {
    use rand_core::RngCore as _;
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    load_or_create_node_id(datadir, bytes)
}

/// Forget this node's id. The next check-in generates a fresh one, so the
/// directory sees an unrelated node. This is the operator's off-ramp and it
/// must stay trivially available.
pub fn reset_node_id(datadir: &Path) -> AppResult<()> {
    let path = node_id_path(datadir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Process(format!("remove node id: {e}"))),
    }
}

/// Normalise `getnetworkinfo`'s `localservices` into what the endpoint accepts:
/// 16 lowercase hex characters. Returns `None` for anything else, so a
/// malformed value is dropped locally instead of producing a 422 every time.
pub fn normalize_services(raw: &str) -> Option<String> {
    let t = raw.trim().trim_start_matches("0x").to_ascii_lowercase();
    if t.len() > 16 || t.is_empty() || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{t:0>16}"))
}

/// Does this services value have bit 27, NODE_MATMUL_CONSENSUS, set?
///
/// Local convenience so the app can tell the operator whether their machine is
/// an independent validator without a round trip. The directory does not take
/// our word for this.
pub fn claims_matmul_consensus(services_hex: &str) -> bool {
    u64::from_str_radix(services_hex.trim_start_matches("0x"), 16)
        .map(|v| v & crate::node_api::NODE_MATMUL_CONSENSUS_BIT != 0)
        .unwrap_or(false)
}

/// The outcome of one check-in attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckinOutcome {
    /// Stored. 204.
    Accepted,
    /// Too soon after the last one. 429. Not an error; back off and carry on.
    TooSoon,
    /// The endpoint is not provisioned right now. 503. Transient, retry later.
    Unavailable,
    /// The payload was refused. 4xx. This is a bug in this client, not a
    /// network condition, so it is surfaced with the reason rather than retried.
    Rejected { status: u16, reason: String },
}

/// POST one check-in.
///
/// Never panics and never retries internally: the caller owns the schedule, so
/// a retry loop here would silently multiply the request rate.
pub async fn send_checkin(
    client: &reqwest::Client,
    endpoint: &str,
    checkin: &Checkin,
) -> AppResult<CheckinOutcome> {
    let res = client
        .post(endpoint)
        .header("x-ebtx-node", "ebtx-node-checkin-v1")
        .json(checkin)
        .send()
        .await
        .map_err(|e| AppError::Process(format!("check-in request failed: {e}")))?;

    let status = res.status().as_u16();
    Ok(match status {
        204 | 200 => CheckinOutcome::Accepted,
        429 => CheckinOutcome::TooSoon,
        503 => CheckinOutcome::Unavailable,
        _ => {
            let reason = res.text().await.unwrap_or_default();
            CheckinOutcome::Rejected {
                status,
                reason: reason.chars().take(200).collect(),
            }
        }
    })
}

/// Build a client and POST one check-in.
///
/// The whole HTTPS surface of this feature is here rather than in the app,
/// which has no HTTP dependency at all and should not grow one to send four
/// hundred bytes every fifteen minutes. The timeout is short on purpose: this
/// runs inside the status refresher's tick, and a stalled connection must not
/// hold up the tick that keeps the screen honest.
pub async fn offer_signing_key(endpoint: &str, checkin: &Checkin) -> AppResult<CheckinOutcome> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Process(format!("check-in client: {e}")))?;
    send_checkin(&client, endpoint, checkin).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Schema 2 carries the key, and a node that does not sign sends it as
    /// null rather than omitting the field: the site accepts a null as absent,
    /// and an omitted-or-renamed field is how a contract quietly breaks.
    #[test]
    fn schema_two_carries_the_key_and_a_non_signer_sends_null() {
        let v = serde_json::to_value(sample()).unwrap();
        assert_eq!(v["schema"], 2);
        assert_eq!(
            v["signer_pubkey"],
            "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
        );
        let keyless = Checkin {
            signer_pubkey: None,
            ..sample()
        };
        let v = serde_json::to_value(keyless).unwrap();
        assert!(v["signer_pubkey"].is_null());
    }

    #[test]
    fn the_engine_tag_becomes_a_version_the_endpoint_keeps() {
        // Our pin carries a commit; the endpoint's regex refuses the hyphen and
        // would store nothing at all.
        assert_eq!(
            engine_version_for_checkin("v0.34.6-3013c2c2").as_deref(),
            Some("v0.34.6")
        );
        assert_eq!(
            engine_version_for_checkin("v0.34.5").as_deref(),
            Some("v0.34.5")
        );
        assert_eq!(
            engine_version_for_checkin("0.34.5").as_deref(),
            Some("0.34.5")
        );
        assert_eq!(engine_version_for_checkin(""), None);
        assert_eq!(engine_version_for_checkin("nightly"), None);
        assert_eq!(engine_version_for_checkin("-3013c2c2"), None);
    }

    #[test]
    fn the_endpoint_is_the_live_site_unless_a_test_says_otherwise() {
        assert_eq!(CHECKIN_ENDPOINT, "https://easybtx.com/api/node-checkin");
        assert!(CHECKIN_ENDPOINT.starts_with("https://"));
    }

    #[test]
    fn a_generated_id_is_the_shape_the_endpoint_requires() {
        let id = format_id([0xab; 16]);
        assert_eq!(id.len(), 32);
        assert!(is_valid_node_id(&id));
        assert_eq!(id, "ab".repeat(16));
    }

    #[test]
    fn id_validation_rejects_the_shapes_the_server_rejects() {
        assert!(!is_valid_node_id(""));
        assert!(!is_valid_node_id(&"a".repeat(31)));
        assert!(!is_valid_node_id(&"a".repeat(33)));
        assert!(
            !is_valid_node_id(&"A".repeat(32)),
            "uppercase is refused server-side"
        );
        assert!(!is_valid_node_id(&"g".repeat(32)), "not hex");
    }

    #[test]
    fn the_id_persists_across_calls() {
        let d = TempDir::new().unwrap();
        let first = load_or_create_node_id(d.path(), [1; 16]).unwrap();
        // Different randomness the second time: the stored id must win, or the
        // directory would see a brand-new node on every restart.
        let second = load_or_create_node_id(d.path(), [2; 16]).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_corrupt_id_file_is_replaced_rather_than_sent_forever() {
        // A truncated id would be rejected by the endpoint on every check-in,
        // for good, and nothing would ever say why.
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join(NODE_ID_FILE), "not-a-valid-id\n").unwrap();
        let id = load_or_create_node_id(d.path(), [3; 16]).unwrap();
        assert!(is_valid_node_id(&id));
        assert_eq!(id, "03".repeat(16));
    }

    #[test]
    fn resetting_the_id_gives_the_node_a_new_identity_and_is_idempotent() {
        let d = TempDir::new().unwrap();
        let first = load_or_create_node_id(d.path(), [4; 16]).unwrap();
        reset_node_id(d.path()).unwrap();
        reset_node_id(d.path()).unwrap(); // already gone: still Ok
        let second = load_or_create_node_id(d.path(), [5; 16]).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn services_are_normalised_to_the_wire_shape() {
        assert_eq!(normalize_services("8000000").unwrap(), "0000000008000000");
        assert_eq!(
            normalize_services("0x08000000").unwrap(),
            "0000000008000000"
        );
        assert_eq!(
            normalize_services("0000000008000000").unwrap(),
            "0000000008000000"
        );
        assert_eq!(
            normalize_services("  08000000  ").unwrap(),
            "0000000008000000"
        );
        assert_eq!(normalize_services("08000000").unwrap().len(), 16);
    }

    #[test]
    fn a_malformed_services_value_is_dropped_here_not_sent() {
        assert!(normalize_services("").is_none());
        assert!(normalize_services("zzzz").is_none());
        assert!(
            normalize_services("00000000000000000").is_none(),
            "17 chars is too long"
        );
    }

    #[test]
    fn bit_27_is_read_the_same_way_the_directory_reads_it() {
        // 0x08000000 is 1<<27, the mask an operator checks in getnetworkinfo.
        assert!(claims_matmul_consensus("0000000008000000"));
        assert!(claims_matmul_consensus("0x08000000"));
        // bit 25 is the trusted mirror, and must NOT read as consensus.
        assert!(!claims_matmul_consensus("0000000002000000"));
        assert!(!claims_matmul_consensus("0000000000000009"));
        assert!(!claims_matmul_consensus("nonsense"));
    }

    fn sample() -> Checkin {
        Checkin {
            schema: CHECKIN_SCHEMA,
            node_id: "a".repeat(32),
            agent: "easynode/test".into(),
            btxd_version: Some("v0.34.5".into()),
            uptime_secs: 3600,
            blocks: 209_274,
            headers: 209_274,
            peers: 12,
            bytes_sent: Some(1234),
            services: "0000000008000000".into(),
            trusted_mirror: false,
            serving_attestations: true,
            listening_port: Some(19335),
            // A real public key: this project's own signer, which every
            // attestation on the network already carries.
            signer_pubkey: Some(
                "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675".into(),
            ),
        }
    }

    /// Emit the exact wire payload so the receiving validator can be run
    /// against it. The two live in different repositories and the endpoint
    /// rejects unknown keys, so "they agree" has to be checked, not assumed.
    /// `cargo test -- --nocapture wire_payload` prints it.
    #[test]
    fn wire_payload_sample_for_cross_checking_the_endpoint() {
        println!(
            "WIRE_PAYLOAD_JSON {}",
            serde_json::to_string(&sample()).unwrap()
        );
        let no_optional = Checkin {
            btxd_version: None,
            bytes_sent: None,
            listening_port: None,
            ..sample()
        };
        println!(
            "WIRE_PAYLOAD_MIN {}",
            serde_json::to_string(&no_optional).unwrap()
        );
    }

    #[test]
    fn the_payload_carries_no_wallet_key_or_identity_field() {
        // The server refuses these outright. This asserts we never grow one.
        let json = serde_json::to_string(&sample()).unwrap();
        for banned in [
            "address",
            "wallet",
            "balance",
            "privkey",
            "private_key",
            "seed",
            "mnemonic",
            "xprv",
            "secret",
            "passphrase",
            "user",
            "email",
        ] {
            assert!(
                !json.contains(banned),
                "payload must not contain {banned}: {json}"
            );
        }
    }

    #[test]
    fn the_payload_serialises_to_exactly_the_keys_the_endpoint_allows() {
        // The endpoint rejects unknown keys, so an extra field here would fail
        // every check-in in the field while passing every test that only
        // round-trips the struct.
        let v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "agent",
                "blocks",
                "btxd_version",
                "bytes_sent",
                "headers",
                "listening_port",
                "node_id",
                "peers",
                "schema",
                "services",
                "serving_attestations",
                // Added 2026-09-17 with schema 2, and added to the site's
                // ALLOWED set in the same change. These two lists are the
                // contract; a field in one and not the other is a 422 on
                // every node in the fleet.
                "signer_pubkey",
                "trusted_mirror",
                "uptime_secs",
            ]
        );
    }

    #[tokio::test]
    async fn a_204_is_accepted() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("POST", "/api/node-checkin")
            .match_header("x-ebtx-node", "ebtx-node-checkin-v1")
            .with_status(204)
            .create_async()
            .await;
        let out = send_checkin(
            &reqwest::Client::new(),
            &format!("{}/api/node-checkin", server.url()),
            &sample(),
        )
        .await
        .unwrap();
        assert_eq!(out, CheckinOutcome::Accepted);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn a_429_is_too_soon_and_a_503_is_transient() {
        let mut server = mockito::Server::new_async().await;
        let _a = server
            .mock("POST", "/a")
            .with_status(429)
            .create_async()
            .await;
        let _b = server
            .mock("POST", "/b")
            .with_status(503)
            .create_async()
            .await;
        let c = reqwest::Client::new();
        assert_eq!(
            send_checkin(&c, &format!("{}/a", server.url()), &sample())
                .await
                .unwrap(),
            CheckinOutcome::TooSoon
        );
        assert_eq!(
            send_checkin(&c, &format!("{}/b", server.url()), &sample())
                .await
                .unwrap(),
            CheckinOutcome::Unavailable
        );
    }

    #[tokio::test]
    async fn a_422_reports_the_reason_instead_of_retrying_blindly() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/node-checkin")
            .with_status(422)
            .with_body(r#"{"error":"unknown field: lol"}"#)
            .create_async()
            .await;
        let out = send_checkin(
            &reqwest::Client::new(),
            &format!("{}/api/node-checkin", server.url()),
            &sample(),
        )
        .await
        .unwrap();
        match out {
            CheckinOutcome::Rejected { status, reason } => {
                assert_eq!(status, 422);
                assert!(reason.contains("unknown field"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dead_endpoint_is_an_error_not_a_panic() {
        // Port 1 on localhost refuses immediately.
        let out = send_checkin(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/api/node-checkin",
            &sample(),
        )
        .await;
        assert!(out.is_err());
    }
}
