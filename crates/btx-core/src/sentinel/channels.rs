//! Where the sentinel's alarms go: the box's existing Orca route and ntfy
//! topic, read from the conf the box's selfcheck already uses.
//!
//! Orca is matched to `orca_post` in btx-apps (`.claude/skills/btx-ops/lib.sh`
//! and `box/btxscan-selfcheck.sh`, byte-identical there) byte for byte:
//!
//!   body = python3 `json.dumps({"text": text[:3500]})`
//!   sig  = hex(HMAC-SHA256(secret, "<unix seconds>.<body>"))
//!   POST ORCA_URL, Content-Type: application/json, X-Webhook-Timestamp,
//!        X-Webhook-Signature-V2, X-Request-ID (a v4 UUID); 2xx = delivered.
//!
//! The body has to be Python's, not serde's: serde writes `{"text":"…"}` with
//! raw UTF-8, Python writes `{"text": "…"}` with every non-ASCII character as a
//! `\uXXXX` escape. The route checks the signature over the bytes it received,
//! so either would verify, but the one format the route has ever seen is
//! Python's, and a vector computed by the shell function pins it here.
//!
//! The secret never leaves this module except as the HMAC key: no log line,
//! no error, no `Debug` prints it.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// What Orca's route accepts; `orca_post` cuts at the same 3,500 characters.
pub const ORCA_MAX_CHARS: usize = 3500;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// The alarm channels, as the box's conf names them. `Debug` is written by
/// hand so a stray `{:?}` can never print the secret.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AlarmConf {
    pub orca_url: String,
    pub orca_secret: String,
    pub ntfy_url: String,
    pub ntfy_topic: String,
}

impl std::fmt::Debug for AlarmConf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlarmConf")
            .field("orca", &self.orca_configured())
            .field("ntfy", &self.ntfy_configured())
            .finish()
    }
}

impl AlarmConf {
    pub fn orca_configured(&self) -> bool {
        !self.orca_url.is_empty() && !self.orca_secret.is_empty()
    }

    pub fn ntfy_configured(&self) -> bool {
        !self.ntfy_topic.is_empty()
    }

    /// The channels this conf reaches, in the order they are tried.
    pub fn channels(&self) -> Vec<Channel> {
        let mut out = Vec::new();
        if self.ntfy_configured() {
            out.push(Channel::Ntfy);
        }
        if self.orca_configured() {
            out.push(Channel::Orca);
        }
        out
    }

    /// Read the shell-style conf (`KEY=value`, optional quotes, `#` comments,
    /// optional `export`). A missing or unreadable file is an empty conf:
    /// the sentinel then runs, logs its alarms, and says so once.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut c = AlarmConf {
            ntfy_url: "https://ntfy.sh".to_string(),
            ..Default::default()
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = unquote(v.trim());
            match k.trim() {
                "ORCA_URL" => c.orca_url = v,
                "ORCA_SECRET" => c.orca_secret = v,
                "NTFY_TOPIC" => c.ntfy_topic = v,
                "NTFY_URL" if !v.is_empty() => c.ntfy_url = v,
                _ => {}
            }
        }
        c.ntfy_url = c.ntfy_url.trim_end_matches('/').to_string();
        c
    }
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// One alarm channel. `Log` is always there: journald, so an alarm that no
/// configured channel took is still on the box.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Channel {
    Log,
    Ntfy,
    Orca,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Log => "log",
            Channel::Ntfy => "ntfy",
            Channel::Orca => "orca",
        }
    }
}

/// How loud. The same three the selfcheck uses, and the same ntfy priority
/// words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Prio {
    High,
    Default,
    Recovery,
}

impl Prio {
    fn ntfy_priority(self) -> &'static str {
        match self {
            Prio::High => "high",
            Prio::Default | Prio::Recovery => "default",
        }
    }

    fn ntfy_tags(self) -> &'static str {
        match self {
            Prio::High => "rotating_light",
            Prio::Default => "wrench",
            Prio::Recovery => "white_check_mark",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Prio::High => "\u{1F6A8}",
            Prio::Default => "\u{1F527}",
            Prio::Recovery => "\u{2705}",
        }
    }
}

/// The Orca text for one message: the selfcheck's shape, "<icon> <who>: <msg>".
pub fn orca_text(prio: Prio, msg: &str) -> String {
    format!("{} btx sentinel: {msg}", prio.icon())
}

/// `json.dumps({"text": text[:3500]})`, byte for byte as CPython writes it
/// with its defaults (`ensure_ascii=True`, `", "` and `": "` separators).
pub fn python_json_text_body(text: &str) -> String {
    let mut out = String::from("{\"text\": \"");
    for ch in text.chars().take(ORCA_MAX_CHARS) {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // Python escapes everything outside printable ASCII, 0x7f too.
            ' '..='~' => out.push(ch),
            _ => {
                let mut units = [0u16; 2];
                for u in ch.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out.push_str("\"}");
    out
}

/// HMAC-SHA256, RFC 2104, over sha2 (already a dependency for the snapshot
/// hash), so the signer adds no crate.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new()
        .chain_update(ipad)
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(opad)
        .chain_update(inner)
        .finalize()
        .into()
}

/// The signed request, everything but the request id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedOrca {
    pub timestamp: String,
    pub signature: String,
    pub body: String,
}

pub fn sign_orca(secret: &str, text: &str, unix_secs: u64) -> SignedOrca {
    let body = python_json_text_body(text);
    let timestamp = unix_secs.to_string();
    let mac = hmac_sha256(secret.as_bytes(), format!("{timestamp}.{body}").as_bytes());
    SignedOrca {
        timestamp,
        signature: crate::operators::hex(&mac),
        body,
    }
}

/// A random v4 UUID, as Python's `uuid.uuid4()` prints it.
fn uuid4() -> String {
    use rand_core::RngCore;
    let mut b = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::operators::hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Deliver one message on one channel. `Ok` only when the channel took it:
/// a 2xx from Orca (which answers 2xx only after Telegram took the message)
/// or from ntfy. The error is safe to log: it never carries the secret.
pub async fn deliver(
    http: &reqwest::Client,
    conf: &AlarmConf,
    channel: Channel,
    prio: Prio,
    msg: &str,
    unix_secs: u64,
) -> Result<(), String> {
    match channel {
        Channel::Log => {
            eprintln!("[sentinel] {} {msg}", prio.ntfy_priority());
            Ok(())
        }
        Channel::Ntfy => {
            let url = format!("{}/{}", conf.ntfy_url, conf.ntfy_topic);
            let resp = http
                .post(url)
                .timeout(SEND_TIMEOUT)
                .header("Title", "btx sentinel")
                .header("Priority", prio.ntfy_priority())
                .header("Tags", prio.ntfy_tags())
                .body(msg.to_string())
                .send()
                .await
                .map_err(|e| format!("ntfy: {}", e.without_url()))?;
            status_ok("ntfy", resp.status().as_u16())
        }
        Channel::Orca => {
            let s = sign_orca(&conf.orca_secret, &orca_text(prio, msg), unix_secs);
            let resp = http
                .post(&conf.orca_url)
                .timeout(SEND_TIMEOUT)
                .header("Content-Type", "application/json")
                .header("X-Webhook-Timestamp", &s.timestamp)
                .header("X-Webhook-Signature-V2", &s.signature)
                .header("X-Request-ID", uuid4())
                .body(s.body)
                .send()
                .await
                .map_err(|e| format!("orca: {}", e.without_url()))?;
            status_ok("orca", resp.status().as_u16())
        }
    }
}

fn status_ok(who: &str, code: u16) -> Result<(), String> {
    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(format!("{who}: HTTP {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector: CPython's `orca_post` code with the timestamp fixed at
    /// 1791000000, and `openssl dgst -sha256 -hmac` over the same bytes,
    /// computed 2026-10-03; both printed the signature below. The text has
    /// what a JSON encoder can disagree on: an astral emoji (a surrogate
    /// pair), a newline, quotes, a backslash, a tab, Latin-1 letters and DEL.
    #[test]
    fn orca_signature_matches_the_shell_function_byte_for_byte() {
        let text = "\u{1F6A8} btx sentinel: No snapshot is confirmed yet.\n\"quoted\" back\\slash tab\tend \u{e9} \u{fc}\u{7f}";
        let s = sign_orca("test-secret-not-real", text, 1_791_000_000);
        assert_eq!(
            s.body,
            concat!(
                r#"{"text": "\ud83d\udea8 btx sentinel: No snapshot is confirmed yet.\n\"quoted\" back\\slash tab\tend "#,
                r#"\u00e9 \u00fc\u007f"}"#
            )
        );
        assert_eq!(s.timestamp, "1791000000");
        assert_eq!(
            s.signature,
            "c0a58a4e2590be38315a7de5458c27b06d709d9e6726832c94e3b9d87152f8bf"
        );
    }

    /// RFC 4231 test case 2 and the long-key case 6, so the HMAC is HMAC.
    #[test]
    fn hmac_matches_rfc_4231() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            crate::operators::hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let mac = hmac_sha256(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            crate::operators::hex(&mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn the_body_is_cut_at_3500_characters_like_orca_post() {
        let long = "\u{e9}".repeat(4000);
        let body = python_json_text_body(&long);
        assert_eq!(body.matches("\\u00e9").count(), 3500);
    }

    #[test]
    fn the_conf_is_read_like_the_shell_sources_it() {
        let c = AlarmConf::parse(
            "# box alarms\nORCA_URL=https://orca.example/webhooks/btx-alarm-1\nORCA_SECRET='s3cret'\nexport NTFY_TOPIC=\"btxscan-alarms-x\"\nNTFY_URL=https://ntfy.example/\n",
        );
        assert_eq!(c.orca_url, "https://orca.example/webhooks/btx-alarm-1");
        assert_eq!(c.orca_secret, "s3cret");
        assert_eq!(c.ntfy_topic, "btxscan-alarms-x");
        assert_eq!(c.ntfy_url, "https://ntfy.example");
        assert_eq!(c.channels(), vec![Channel::Ntfy, Channel::Orca]);
        // Debug never shows the secret.
        assert!(!format!("{c:?}").contains("s3cret"));
    }

    #[test]
    fn a_missing_conf_is_no_channel_not_an_error() {
        let c = AlarmConf::load(Path::new("/nonexistent/btxscan-selfcheck.conf"));
        assert!(c.channels().is_empty());
        assert_eq!(c.ntfy_url, "");
        // An ORCA_URL without its secret is not a channel, as in orca_configured.
        let c = AlarmConf::parse("ORCA_URL=https://x\n");
        assert!(c.channels().is_empty());
    }

    #[test]
    fn a_request_id_is_a_v4_uuid() {
        let u = uuid4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }
}
