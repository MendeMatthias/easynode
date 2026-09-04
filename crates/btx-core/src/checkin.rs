//! Node check-in: telling the directory this node exists, over HTTPS.
//!
//! [`service_report`](crate::service_report) writes what this node has been
//! doing to a local file and says, in its own module doc, that publishing it
//! anywhere is "a separate, explicit, future feature with its own consent".
//! This is that feature, and the consent is the point: it is **off unless the
//! operator turns it on**, and nothing here runs otherwise.
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
//! There is **no wallet, no address, no balance, no key, no username, and no
//! machine fingerprint**. The id is random bytes generated once and kept in the
//! datadir; it identifies the same node across restarts and nothing else, and
//! deleting the file gives the node a new identity with no consequence.
//!
//! The server sees the source IP, as it must for any HTTP request. It stores
//! only a salted hash of it plus a coarse location, the same model the peer map
//! already uses.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Schema of the payload below. Must match the receiving endpoint.
pub const CHECKIN_SCHEMA: u32 = 1;

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
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
        .map(|v| v & (1u64 << 27) != 0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        assert!(!is_valid_node_id(&"A".repeat(32)), "uppercase is refused server-side");
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
        assert_eq!(normalize_services("0x08000000").unwrap(), "0000000008000000");
        assert_eq!(normalize_services("0000000008000000").unwrap(), "0000000008000000");
        assert_eq!(normalize_services("  08000000  ").unwrap(), "0000000008000000");
        assert_eq!(normalize_services("08000000").unwrap().len(), 16);
    }

    #[test]
    fn a_malformed_services_value_is_dropped_here_not_sent() {
        assert!(normalize_services("").is_none());
        assert!(normalize_services("zzzz").is_none());
        assert!(normalize_services("00000000000000000").is_none(), "17 chars is too long");
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
        }
    }

    /// Emit the exact wire payload so the receiving validator can be run
    /// against it. The two live in different repositories and the endpoint
    /// rejects unknown keys, so "they agree" has to be checked, not assumed.
    /// `cargo test -- --nocapture wire_payload` prints it.
    #[test]
    fn wire_payload_sample_for_cross_checking_the_endpoint() {
        println!("WIRE_PAYLOAD_JSON {}", serde_json::to_string(&sample()).unwrap());
        let no_optional = Checkin { btxd_version: None, bytes_sent: None, listening_port: None, ..sample() };
        println!("WIRE_PAYLOAD_MIN {}", serde_json::to_string(&no_optional).unwrap());
    }

    #[test]
    fn the_payload_carries_no_wallet_key_or_identity_field() {
        // The server refuses these outright. This asserts we never grow one.
        let json = serde_json::to_string(&sample()).unwrap();
        for banned in [
            "address", "wallet", "balance", "privkey", "private_key", "seed",
            "mnemonic", "xprv", "secret", "passphrase", "user", "email",
        ] {
            assert!(!json.contains(banned), "payload must not contain {banned}: {json}");
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
                "agent", "blocks", "btxd_version", "bytes_sent", "headers",
                "listening_port", "node_id", "peers", "schema", "services",
                "serving_attestations", "trusted_mirror", "uptime_secs",
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
        let out = send_checkin(&reqwest::Client::new(), &format!("{}/api/node-checkin", server.url()), &sample())
            .await
            .unwrap();
        assert_eq!(out, CheckinOutcome::Accepted);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn a_429_is_too_soon_and_a_503_is_transient() {
        let mut server = mockito::Server::new_async().await;
        let _a = server.mock("POST", "/a").with_status(429).create_async().await;
        let _b = server.mock("POST", "/b").with_status(503).create_async().await;
        let c = reqwest::Client::new();
        assert_eq!(
            send_checkin(&c, &format!("{}/a", server.url()), &sample()).await.unwrap(),
            CheckinOutcome::TooSoon
        );
        assert_eq!(
            send_checkin(&c, &format!("{}/b", server.url()), &sample()).await.unwrap(),
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
        let out = send_checkin(&reqwest::Client::new(), &format!("{}/api/node-checkin", server.url()), &sample())
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
        let out = send_checkin(&reqwest::Client::new(), "http://127.0.0.1:1/api/node-checkin", &sample()).await;
        assert!(out.is_err());
    }
}
