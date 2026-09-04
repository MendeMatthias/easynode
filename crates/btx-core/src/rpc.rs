use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use crate::error::{AppError, AppResult};

/// Minimal RFC 3986 percent-encoder for the URL path segment used by
/// `RpcClient::for_wallet`. We avoid pulling in a new crate (`percent-encoding`,
/// `url`) for this one call site — the unreserved set is small and stable.
///
/// Unreserved (passed through): `A-Z`, `a-z`, `0-9`, `-`, `.`, `_`, `~`.
/// Everything else is emitted as `%XX` (uppercase hex, byte-by-byte, so any
/// multi-byte UTF-8 character is encoded by its individual bytes — matching
/// what every web browser does).
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let unreserved = matches!(b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        );
        if unreserved {
            out.push(b as char);
        } else {
            // Uppercase hex per RFC 3986 §2.1 ("For consistency, URI producers
            // and normalizers should use uppercase hexadecimal digits").
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

#[async_trait]
pub trait Rpc: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value>;
}

#[derive(Clone)]
pub struct RpcClient {
    client: reqwest::Client,
    url: String,
    user: String,
    pass: String,
}

impl RpcClient {
    pub fn new(
        base_url: impl Into<String>,
        user: impl Into<String>,
        pass: impl Into<String>,
    ) -> Self {
        // 60-second request timeout: safety net against a hung connection.
        // Short enough to keep recovery responsive; long enough not to abort a
        // legitimate generatetoaddress call (256 maxtries returns well under 60s).
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            url: base_url.into(),
            user: user.into(),
            pass: pass.into(),
        }
    }

    pub fn for_wallet(&self, name: &str) -> Self {
        let base = self.url.trim_end_matches('/');
        Self {
            client: self.client.clone(),
            // Percent-encode the wallet name before interpolating it into the URL
            // path. Defence-in-depth: callers today only pass `"miner"`, but any
            // future call site (or a future user-named wallet) that includes `/`,
            // `?`, `#`, whitespace, or other URL-reserved bytes would otherwise
            // silently corrupt the URL (e.g. a name `../foo` could traverse off
            // `/wallet/`). pct_encode keeps the same unreserved set RFC 3986
            // defines (`A-Z a-z 0-9 - . _ ~`) and percent-encodes the rest.
            url: format!("{}/wallet/{}", base, pct_encode(name)),
            user: self.user.clone(),
            pass: self.pass.clone(),
        }
    }

    /// Build a client from a node datadir `.cookie` file (format `__cookie__:<password>`).
    pub fn from_cookie(
        base_url: impl Into<String>,
        cookie_path: &std::path::Path,
    ) -> AppResult<Self> {
        let raw = std::fs::read_to_string(cookie_path)
            .map_err(|_| AppError::Config("cannot read .cookie file".into()))?;
        let (user, pass) = raw
            .trim()
            .split_once(':')
            .ok_or_else(|| AppError::Config("malformed .cookie (expected user:pass)".into()))?;
        Ok(Self::new(base_url, user.to_string(), pass.to_string()))
    }
}

#[async_trait]
impl Rpc for RpcClient {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        let body = json!({
            "jsonrpc": "1.0",
            "id": "easybtx",
            "method": method,
            "params": params,
        });

        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Http(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // btxd reports genuine JSON-RPC errors — most importantly RPC_IN_WARMUP
            // (-28), returned for minutes while it verifies blocks / rebuilds
            // shielded state — as an HTTP 500 with a structured error body. Surface
            // those as AppError::Rpc so the startup wait can tell a node that is
            // ALIVE and warming up apart from one that is truly unreachable; without
            // this the body was discarded and a healthy multi-minute warmup looked
            // identical to a dead node (and got timed out into the error/repair UI).
            //
            // We do this ONLY for 5xx server errors carrying a parseable JSON-RPC
            // error object. A 4xx (e.g. 401 on a bad cookie) keeps the generic
            // message so verbose node internals — and any credential the node might
            // echo on an auth failure — never reach the webview.
            if status.is_server_error() {
                if let Ok(json) = serde_json::from_str::<Value>(&body) {
                    if let Some(err) = json.get("error").filter(|e| !e.is_null()) {
                        let code = err.get("code").and_then(Value::as_i64).unwrap_or(-1);
                        let message = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                            .to_string();
                        return Err(AppError::Rpc { code, message });
                    }
                }
            }
            // Log the full body to stderr for debugging, but return only a generic
            // message to the webview (see above).
            eprintln!("[rpc] HTTP {status} from node: {body}");
            return Err(AppError::Http(format!("node RPC error (HTTP {status})")));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| AppError::Decode(e.to_string()))?;

        if let Some(err) = json.get("error") {
            if !err.is_null() {
                let code = err.get("code").and_then(Value::as_i64).unwrap_or(-1);
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(AppError::Rpc { code, message });
            }
        }

        Ok(json.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[tokio::test]
    async fn call_returns_result_field() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"blocks":42},"error":null,"id":"easybtx"}"#)
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p");
        let result = client.call("getblockchaininfo", json!([])).await.unwrap();

        assert_eq!(result["blocks"], 42);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn call_surfaces_rpc_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":null,"error":{"code":-8,"message":"bad"},"id":"x"}"#)
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p");
        let result = client.call("somemethod", json!([])).await;

        match result {
            Err(AppError::Rpc { code, message }) => {
                assert_eq!(code, -8);
                assert_eq!(message, "bad");
            }
            other => panic!("expected AppError::Rpc, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn warmup_500_surfaces_the_rpc_in_warmup_code() {
        // btxd answers RPC during block verification / shielded-state rebuild with
        // HTTP 500 + a JSON-RPC error carrying code -28 (RPC_IN_WARMUP). The client
        // MUST surface that as AppError::Rpc{-28} so the startup wait can tell
        // "alive and warming" apart from "unreachable" — instead of collapsing it
        // into a generic Http error (the bug that timed a healthy node into Error).
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"result":null,"error":{"code":-28,"message":"Verifying blocks…"},"id":"easybtx"}"#,
            )
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p");
        let result = client.call("getblockchaininfo", json!([])).await;

        match result {
            Err(AppError::Rpc { code, message }) => {
                assert_eq!(code, -28);
                assert_eq!(message, "Verifying blocks…");
            }
            other => panic!("expected AppError::Rpc{{-28}}, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn auth_failure_401_stays_generic_and_leaks_nothing() {
        // SECURITY REGRESSION GUARD: a 401 (bad cookie) must NOT be parsed into an
        // Rpc error — it keeps the generic Http message so verbose node internals
        // and any echoed credential never reach the webview. The 5xx body-parse
        // above must not weaken this: only 5xx server errors carrying a JSON-RPC
        // error body are surfaced; a 4xx stays opaque.
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body("user:supersecretcookietoken")
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p");
        let result = client.call("getblockchaininfo", json!([])).await;

        match result {
            Err(AppError::Http(msg)) => {
                assert!(
                    msg.contains("401"),
                    "should name the HTTP status, got {msg:?}"
                );
                assert!(
                    !msg.contains("supersecret"),
                    "must NOT leak the response body / credential, got {msg:?}"
                );
            }
            other => panic!("expected AppError::Http, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn for_wallet_targets_wallet_path() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/wallet/miner")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"balance":1.0},"error":null,"id":"easybtx"}"#)
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p").for_wallet("miner");
        let result = client.call("getwalletinfo", json!([])).await.unwrap();

        assert_eq!(result["balance"], 1.0);
        mock.assert_async().await;
    }

    #[test]
    fn pct_encode_passes_unreserved_and_escapes_reserved() {
        // Unreserved set (RFC 3986 §2.3) — every byte goes through unchanged.
        assert_eq!(
            pct_encode("ABCxyz012-._~"),
            "ABCxyz012-._~",
            "unreserved chars must not be touched"
        );
        // Reserved / unsafe bytes — every one becomes %XX (uppercase hex).
        // These are the ones that would actually break a /wallet/<name> URL.
        assert_eq!(pct_encode("/"), "%2F");
        assert_eq!(pct_encode("?"), "%3F");
        assert_eq!(pct_encode("#"), "%23");
        assert_eq!(pct_encode(" "), "%20");
        assert_eq!(pct_encode(".."), ".."); // dots ARE unreserved — path safety
                                            // is handled by the receiving HTTP server, not by encoding.
        assert_eq!(pct_encode("../etc"), "..%2Fetc");
        // Multi-byte UTF-8 → byte-by-byte percent encoding (matches browsers).
        // "é" is 0xC3 0xA9.
        assert_eq!(pct_encode("é"), "%C3%A9");
        // Empty input → empty output, no panic.
        assert_eq!(pct_encode(""), "");
    }

    #[tokio::test]
    async fn for_wallet_percent_encodes_unsafe_chars() {
        // A wallet name with a `/` must NOT smash through the /wallet/ path —
        // it has to be encoded so the server sees a single path segment.
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/wallet/odd%2Fname")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"ok":true},"error":null,"id":"easybtx"}"#)
            .create_async()
            .await;

        let client = RpcClient::new(server.url(), "u", "p").for_wallet("odd/name");
        let result = client.call("getwalletinfo", json!([])).await.unwrap();

        assert_eq!(result["ok"], true);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn from_cookie_parses_and_authenticates() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                Matcher::Exact("Basic X19jb29raWVfXzpzZWNyZXRwYXNz".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"ok":true},"error":null,"id":"easybtx"}"#)
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join(".cookie");
        std::fs::write(&cookie_path, "__cookie__:secretpass").unwrap();

        let client = RpcClient::from_cookie(server.url(), &cookie_path).unwrap();
        let result = client.call("getinfo", json!([])).await.unwrap();

        assert_eq!(result["ok"], true);
        mock.assert_async().await;
    }
}
