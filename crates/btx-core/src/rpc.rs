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

/// One `.cookie` line, `user:password`, as btxd writes it
/// (`__cookie__:<random>`). Shared by construction and by the reload after a
/// 401, so the two can never disagree about what the file says.
fn parse_cookie(raw: &str) -> Option<(String, String)> {
    let (user, pass) = raw.trim().split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

/// The cookie file's mtime for the reload log line: when btxd wrote it, as
/// unix seconds, and how long ago. The contents are never logged — the file
/// is the node's RPC secret.
fn cookie_mtime(path: &std::path::Path) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return "mtime unknown".to_string();
    };
    let unix = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ago = modified.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    format!("mtime {unix} (written {ago}s ago)")
}

/// JSON-RPC `RPC_IN_WARMUP`: btxd is alive and not ready to answer yet.
pub const RPC_IN_WARMUP: i64 = -28;

/// What an HTTP 503 from the node becomes: `AppError::Http` with this text.
///
/// btxd answers 503 with an HTML page (`<HTML><HEAD><TITLE>503 Service
/// Unavailable</TITLE>...`) while its HTTP server is up and its RPC is not:
/// during a restart, on the way down and on the way up, and for as long as
/// it hangs in shutdown or its RPC queue stays full.
///
/// It is a FAILURE, not `RPC_IN_WARMUP`. The status refresher counts
/// failures to "The node stopped responding" and a graceful stop, and the
/// start's RPC wait counts them against its budget; a node that answers 503
/// for ever must reach both. Only screens that merely ask the person to try
/// again in a moment read it as warming ([`is_not_ready`]).
pub const NODE_NOT_READY: &str = "node not ready (HTTP 503)";

/// The error a 503 became ([`NODE_NOT_READY`]).
pub fn is_not_ready(e: &AppError) -> bool {
    matches!(e, AppError::Http(m) if m == NODE_NOT_READY)
}

/// The last reply state a line was logged for, process-wide: 0 for an
/// ordinary answer, else the HTTP status of a 503 or of an HTML page. Shared
/// by every client, because the app builds a fresh client for many probes
/// (the warmup watcher makes one every two seconds), and "once per state
/// change" has to hold across them.
static LOGGED_REPLY_STATE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// A reply body that is an HTML page rather than JSON. A JSON-RPC body starts
/// with `{`; one that starts with `<` is markup, whatever its status.
fn is_html(body: &str) -> bool {
    body.trim_start().starts_with('<')
}

/// What to log for a failed reply with `status` and `body`, given the state
/// last logged in `logged`.
///
/// `None`: not a 503 and not an HTML page; the caller handles it as before,
/// and the state goes back to "answering", so the next 503 is a new one.
/// `Some(line)`: a 503 or an HTML page. `line` is the one short line to log
/// when that state begins, and `None` while it lasts. The page itself is
/// never in it.
fn unready_log_line(
    logged: &std::sync::atomic::AtomicU16,
    status: reqwest::StatusCode,
    body: &str,
) -> Option<Option<String>> {
    use std::sync::atomic::Ordering;
    let unready = status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
    if !unready && !is_html(body) {
        logged.store(0, Ordering::SeqCst);
        return None;
    }
    let code = status.as_u16();
    let first = logged.swap(code, Ordering::SeqCst) != code;
    Some(first.then(|| {
        if unready {
            format!("[rpc] {NODE_NOT_READY}; said once until the node answers again")
        } else {
            format!("[rpc] HTTP {status} from node (an HTML page, not logged)")
        }
    }))
}

#[async_trait]
pub trait Rpc: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value>;
}

#[derive(Clone)]
pub struct RpcClient {
    client: reqwest::Client,
    url: String,
    /// Shared so a refresh reaches every clone — `for_wallet` hands out a second
    /// handle to the SAME node, and a cookie that rotated rotated for both.
    creds: std::sync::Arc<std::sync::RwLock<(String, String)>>,
    /// Where the credentials came from, when they came from a `.cookie`.
    ///
    /// btxd regenerates `.cookie` on every start, so credentials read once at
    /// construction are only valid for the btxd that was running then. A client
    /// that outlives a node restart — every long-lived one in this app does —
    /// then authenticates with a dead password and gets HTTP 401 forever, which
    /// `call` reports as a generic HTTP error indistinguishable from a node
    /// that is down. Keeping the PATH is what lets a 401 be answered by
    /// re-reading the file instead of by giving up.
    cookie_path: Option<std::path::PathBuf>,
    /// This handle's own request timeout, in place of the client's 60 s
    /// ([`RpcClient::with_timeout`]). `None` keeps the 60 s.
    timeout: Option<Duration>,
    /// Where the "logged once" state of unready replies is kept: the
    /// process-wide [`LOGGED_REPLY_STATE`], except in tests.
    logged: &'static std::sync::atomic::AtomicU16,
}

impl RpcClient {
    /// The same node and credentials, with requests allowed `timeout` instead
    /// of 60 s. For the few calls that can take longer on mainnet
    /// (`gettxoutsetinfo` hashes the whole UTXO set), always off the status
    /// refresher's tick.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            ..self.clone()
        }
    }

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
            creds: std::sync::Arc::new(std::sync::RwLock::new((user.into(), pass.into()))),
            cookie_path: None,
            timeout: None,
            logged: &LOGGED_REPLY_STATE,
        }
    }

    /// This handle with its own "logged once" state, so a test can read it
    /// without the process-wide one other tests share.
    #[cfg(test)]
    fn with_logged_state(mut self, logged: &'static std::sync::atomic::AtomicU16) -> Self {
        self.logged = logged;
        self
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
            creds: self.creds.clone(),
            cookie_path: self.cookie_path.clone(),
            timeout: self.timeout,
            logged: self.logged,
        }
    }

    /// Build a client from a node datadir `.cookie` file (format `__cookie__:<password>`).
    pub fn from_cookie(
        base_url: impl Into<String>,
        cookie_path: &std::path::Path,
    ) -> AppResult<Self> {
        let raw = std::fs::read_to_string(cookie_path)
            .map_err(|_| AppError::Config("cannot read .cookie file".into()))?;
        let (user, pass) = parse_cookie(&raw)
            .ok_or_else(|| AppError::Config("malformed .cookie (expected user:pass)".into()))?;
        let mut client = Self::new(base_url, user, pass);
        client.cookie_path = Some(cookie_path.to_path_buf());
        Ok(client)
    }

    /// The credentials to sign the next request with.
    ///
    /// Cloned out under the lock rather than held across the `await`: a
    /// `std::sync::RwLock` guard is not `Send`, and holding one over a network
    /// round trip would serialise every concurrent RPC behind the slowest.
    fn creds(&self) -> (String, String) {
        let guard = self.creds.read().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    /// Re-read the `.cookie` after a 401 answered to `sent`.
    ///
    /// Returns the credentials now on disk when they differ from what was
    /// sent, which is the one case a replay can succeed, and adopts them for
    /// every clone. `None` when the file is gone, malformed, or still says
    /// exactly what was sent: then the 401 is a real refusal and a replay
    /// would be a second identical failure. A client built from an explicit
    /// user/pass has no path and never replays.
    ///
    /// Compared against what THIS request sent, not against the shared
    /// credentials. Eight witness requests can be in flight when btxd comes
    /// back, all refused with the dead cookie; the first one here adopts the
    /// new file, and comparing the other seven against the shared state would
    /// call the file "unchanged" and fail them, though a replay with the fresh
    /// cookie is exactly what they need. The file is the truth, and each
    /// request asks whether it was behind it.
    fn reload_cookie_after_401(&self, sent: &(String, String)) -> Option<(String, String)> {
        let path = self.cookie_path.as_ref()?;
        let fresh = parse_cookie(&std::fs::read_to_string(path).ok()?)?;
        if fresh == *sent {
            return None;
        }
        let mtime = cookie_mtime(path);
        let mut guard = self.creds.write().unwrap_or_else(|e| e.into_inner());
        if *guard != fresh {
            *guard = fresh.clone();
            // One line per adoption, not per replay, and never the cookie.
            eprintln!(
                "[rpc] node RPC cookie reloaded after 401 (btxd restarted?); .cookie {mtime}"
            );
        }
        Some(fresh)
    }

    async fn send(&self, creds: &(String, String), body: &Value) -> AppResult<reqwest::Response> {
        let mut req = self
            .client
            .post(&self.url)
            .basic_auth(&creds.0, Some(&creds.1))
            .json(body);
        if let Some(t) = self.timeout {
            // Overrides the client's 60 s for this request only.
            req = req.timeout(t);
        }
        req.send().await.map_err(|e| AppError::Http(e.to_string()))
    }
}

/// The JSON-RPC "method not found" error in a 404 body, and nothing else:
/// any other 4xx body keeps the generic message (it may echo credentials).
fn method_not_found(body: &str) -> Option<AppError> {
    let json = serde_json::from_str::<Value>(body).ok()?;
    let err = json.get("error")?;
    if err.get("code").and_then(Value::as_i64) != Some(-32601) {
        return None;
    }
    Some(AppError::Rpc {
        code: -32601,
        message: err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Method not found")
            .to_string(),
    })
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

        let sent = self.creds();
        let mut response = self.send(&sent, &body).await?;

        // A 401 from btxd means one thing in practice: it restarted and wrote a
        // new `.cookie` under us. Re-read it and replay ONCE — once, because a
        // second 401 on freshly-read credentials is a real authentication
        // failure and retrying it would spin. The reload hands back nothing
        // when the file is unchanged or absent, so a client built from an
        // explicit user/pass never replays at all.
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(fresh) = self.reload_cookie_after_401(&sent) {
                response = self.send(&fresh, &body).await?;
            }
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // A 503 (btxd restarting: its HTTP server answers and its RPC
            // does not, with an HTML page) and any other HTML page: one short
            // line when it begins, never the page. Still a failure: see
            // `NODE_NOT_READY`.
            if let Some(line) = unready_log_line(self.logged, status, &body) {
                if let Some(line) = line {
                    eprintln!("{line}");
                }
                if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                    return Err(AppError::Http(NODE_NOT_READY.to_string()));
                }
                return Err(AppError::Http(format!("node RPC error (HTTP {status})")));
            }
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
            // btxd answers a command it does not have with HTTP 404 and a
            // JSON-RPC "method not found" (-32601) body. That is an answer,
            // not a fault: it comes back as that error, so a caller asking
            // whether this engine has a command (the swarm card, the snapshot
            // offer) reads "no", and nothing is logged for it.
            if status == reqwest::StatusCode::NOT_FOUND {
                if let Some(e) = method_not_found(&body) {
                    return Err(e);
                }
            }
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

        // An answer: a 503 or an HTML page after this is a new one to log.
        self.logged.store(0, std::sync::atomic::Ordering::SeqCst);
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

    /// A node that takes the request and never answers. `with_timeout` is
    /// the only thing that ends the wait (the shared default is 60 s), and it
    /// leaves the client it came from, and its credentials, as they were.
    #[tokio::test]
    async fn with_timeout_bounds_one_handle_and_shares_the_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hold = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                open.push(s);
            }
        });
        let base = RpcClient::new(format!("http://{addr}"), "u", "p");
        let slow = base.with_timeout(Duration::from_millis(300));
        assert_eq!(slow.timeout, Some(Duration::from_millis(300)));
        assert_eq!(base.timeout, None);
        assert!(std::sync::Arc::ptr_eq(&slow.creds, &base.creds));
        let started = std::time::Instant::now();
        let r = slow.call("gettxoutsetinfo", json!([])).await;
        assert!(r.is_err(), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
        hold.abort();
    }

    /// What btxd v0.34.16 sends for a command it lacks (measured on the
    /// default engine, `getmodelnetworkinfo`): HTTP 404 with a -32601 body.
    /// It is the RPC error "method not found"; another 404 stays generic.
    #[tokio::test]
    async fn a_404_method_not_found_is_the_rpc_error_it_carries() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .match_body(Matcher::PartialJson(json!({"method": "getmodelnetworkinfo"})))
            .with_status(404)
            .with_body(
                r#"{"result":null,"error":{"code":-32601,"message":"Method not found"},"id":"easybtx"}"#,
            )
            .create_async()
            .await;
        let _other = server
            .mock("POST", "/")
            .match_body(Matcher::PartialJson(json!({"method": "x"})))
            .with_status(404)
            .with_body("not here")
            .create_async()
            .await;
        let client = RpcClient::new(server.url(), "u", "p");
        match client.call("getmodelnetworkinfo", json!([])).await {
            Err(AppError::Rpc {
                code: -32601,
                message,
            }) => assert_eq!(message, "Method not found"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            client.call("x", json!([])).await,
            Err(AppError::Http(_))
        ));
    }

    /// What btxd sends while it restarts, as the app logged it on 2026-11-26:
    /// HTTP 503 with an HTML page. It is the warmup error with a short
    /// message, never the page, so every caller that waits out a warmup
    /// (`RPC_IN_WARMUP`) waits this out the same way.
    #[tokio::test]
    async fn a_503_html_page_is_a_short_failure_and_never_its_body() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_status(503)
            .with_header("content-type", "text/html")
            .with_body(
                "<HTML><HEAD><TITLE>503 Service Unavailable</TITLE></HEAD>\
                 <BODY><H1>503 Service Unavailable</H1></BODY></HTML>",
            )
            .create_async()
            .await;
        let logged: &'static std::sync::atomic::AtomicU16 =
            Box::leak(Box::new(std::sync::atomic::AtomicU16::new(0)));
        let client = RpcClient::new(server.url(), "u", "p").with_logged_state(logged);
        match client.call("getblockchaininfo", json!([])).await {
            Err(e) => {
                assert!(is_not_ready(&e), "{e:?}");
                // A failure, never the warmup code a watchdog forgives.
                assert!(!matches!(
                    e,
                    AppError::Rpc {
                        code: RPC_IN_WARMUP,
                        ..
                    }
                ));
                assert!(!e.to_string().contains('<'), "{e}");
            }
            other => panic!("expected the not-ready error, got {other:?}"),
        }
        assert_eq!(logged.load(std::sync::atomic::Ordering::SeqCst), 503);
    }

    /// An HTML page with another status is a plain HTTP error that names the
    /// status and carries none of the page.
    #[tokio::test]
    async fn an_html_page_with_another_status_is_a_short_http_error() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_status(502)
            .with_body("<html><body>Bad Gateway: secret-internal-host</body></html>")
            .create_async()
            .await;
        let logged: &'static std::sync::atomic::AtomicU16 =
            Box::leak(Box::new(std::sync::atomic::AtomicU16::new(0)));
        let client = RpcClient::new(server.url(), "u", "p").with_logged_state(logged);
        match client.call("getblockchaininfo", json!([])).await {
            Err(AppError::Http(msg)) => {
                assert!(msg.contains("502"), "{msg}");
                assert!(!msg.contains('<') && !msg.contains("secret"), "{msg}");
            }
            other => panic!("expected AppError::Http, got {other:?}"),
        }
    }

    /// One line when the node stops being ready, none while it stays so, and
    /// one again after it has answered in between. The line never carries
    /// the page.
    #[test]
    fn an_unready_node_is_logged_once_per_state_change() {
        use reqwest::StatusCode;
        let logged = std::sync::atomic::AtomicU16::new(0);
        let page = "<HTML><HEAD><TITLE>503 Service Unavailable</TITLE></HEAD></HTML>";
        let first = unready_log_line(&logged, StatusCode::SERVICE_UNAVAILABLE, page)
            .expect("a 503 is an unready reply")
            .expect("logged when it begins");
        assert!(first.contains(NODE_NOT_READY), "{first}");
        assert!(!first.contains('<'), "{first}");
        for _ in 0..5 {
            assert_eq!(
                unready_log_line(&logged, StatusCode::SERVICE_UNAVAILABLE, page),
                Some(None),
                "not again while it lasts"
            );
        }
        // A JSON-RPC error (the -28 warmup body) is an answer, not a page.
        let warm = r#"{"result":null,"error":{"code":-28,"message":"Loading"},"id":"x"}"#;
        assert_eq!(
            unready_log_line(&logged, StatusCode::INTERNAL_SERVER_ERROR, warm),
            None
        );
        assert!(
            unready_log_line(&logged, StatusCode::SERVICE_UNAVAILABLE, page)
                .unwrap()
                .is_some()
        );
        // An HTML page with another status: its own state, once, no page.
        let html = unready_log_line(&logged, StatusCode::BAD_GATEWAY, "  <html>x</html>")
            .unwrap()
            .unwrap();
        assert!(html.contains("502") && !html.contains("<html"), "{html}");
        assert_eq!(
            unready_log_line(&logged, StatusCode::BAD_GATEWAY, "<html>x</html>"),
            Some(None)
        );
        // A 503 without a page is still the unready state.
        assert!(
            unready_log_line(&logged, StatusCode::SERVICE_UNAVAILABLE, "")
                .unwrap()
                .is_some()
        );
    }

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

    /// The outage on witness-1.easybtx.com, 2026-09-15: btxd restarted at
    /// 11:45:44Z for the 0.6.23 engine and wrote a new `.cookie`; the witness
    /// running there was built before any reload existed, so from 11:46:06Z
    /// every request was `HTTP 401 Unauthorized` and the public endpoint said
    /// `the node did not answer` until the unit was restarted at 11:47:37Z.
    /// This is that sequence against the client: one call, the file changes,
    /// one more call, and the second recovers by itself, replaying exactly once.
    #[tokio::test]
    async fn a_cookie_that_changes_between_two_calls_is_reloaded_after_the_401() {
        let mut server = Server::new_async().await;
        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join(".cookie");
        std::fs::write(&cookie_path, "__cookie__:first\n").unwrap();
        let client = RpcClient::from_cookie(server.url(), &cookie_path).unwrap();

        // Before the restart: the first cookie is what the node accepts.
        let before = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                Matcher::Exact("Basic X19jb29raWVfXzpmaXJzdA==".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"blocks":1},"error":null,"id":"easybtx"}"#)
            .expect(1)
            .create_async()
            .await;
        let first = client.call("getblockchaininfo", json!([])).await.unwrap();
        assert_eq!(first["blocks"], 1);
        before.assert_async().await;
        before.remove_async().await;

        // btxd restarts: a new cookie on disk, and the old one is refused.
        std::fs::write(&cookie_path, "__cookie__:second\n").unwrap();
        let refused = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                Matcher::Exact("Basic X19jb29raWVfXzpmaXJzdA==".to_string()),
            )
            .with_status(401)
            .expect(1)
            .create_async()
            .await;
        let accepted = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                Matcher::Exact("Basic X19jb29raWVfXzpzZWNvbmQ=".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"blocks":2},"error":null,"id":"easybtx"}"#)
            .expect(1)
            .create_async()
            .await;

        let second = client.call("getblockchaininfo", json!([])).await.unwrap();
        assert_eq!(
            second["blocks"], 2,
            "the replay must carry the reloaded cookie"
        );
        // Exactly one refusal and exactly one replay: the reload ran once.
        refused.assert_async().await;
        accepted.assert_async().await;
        // And the client now holds the file's cookie for every later call.
        assert_eq!(
            client.creds(),
            ("__cookie__".to_string(), "second".to_string())
        );
    }

    /// A 401 with the cookie on disk unchanged is a real refusal, not
    /// staleness: no replay, because it would be a second identical failure.
    #[tokio::test]
    async fn a_401_with_an_unchanged_cookie_is_reported_and_not_replayed() {
        let mut server = Server::new_async().await;
        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join(".cookie");
        std::fs::write(&cookie_path, "__cookie__:first").unwrap();
        let client = RpcClient::from_cookie(server.url(), &cookie_path).unwrap();
        let refused = server
            .mock("POST", "/")
            .with_status(401)
            .expect(1)
            .create_async()
            .await;

        match client.call("getblockchaininfo", json!([])).await {
            Err(AppError::Http(msg)) => assert!(msg.contains("401"), "{msg}"),
            other => panic!("expected AppError::Http, got {other:?}"),
        }
        refused.assert_async().await;
        assert_eq!(
            client.creds().1,
            "first",
            "an unchanged file changes nothing"
        );
    }

    /// The cookie changed and the node refuses the new one too: replay once,
    /// then report. Never a loop — a client that replayed every 401 would spin
    /// against a node it is not allowed to talk to.
    #[tokio::test]
    async fn a_second_401_on_the_fresh_cookie_is_the_error_and_ends_the_retry() {
        let mut server = Server::new_async().await;
        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join(".cookie");
        std::fs::write(&cookie_path, "__cookie__:first").unwrap();
        let client = RpcClient::from_cookie(server.url(), &cookie_path).unwrap();
        std::fs::write(&cookie_path, "__cookie__:second").unwrap();
        let refused = server
            .mock("POST", "/")
            .with_status(401)
            .expect(2)
            .create_async()
            .await;

        match client.call("getblockchaininfo", json!([])).await {
            Err(AppError::Http(msg)) => assert!(msg.contains("401"), "{msg}"),
            other => panic!("expected AppError::Http, got {other:?}"),
        }
        // One original, one replay with the reloaded cookie, and no third.
        refused.assert_async().await;
        // The reload still took effect: the next call sends the file's cookie.
        assert_eq!(client.creds().1, "second");
    }

    /// Eight requests can be in flight when btxd comes back, all refused with
    /// the dead cookie. The first to reload adopts the new file; the other
    /// seven must still replay, because what THEY sent is stale even though
    /// the shared credentials are already fresh. Comparing against the shared
    /// state, which is what this did before, failed all seven with a 401 that
    /// a replay would have cleared.
    #[tokio::test]
    async fn a_request_that_sent_the_dead_cookie_replays_even_after_another_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let cookie_path = dir.path().join(".cookie");
        std::fs::write(&cookie_path, "__cookie__:first").unwrap();
        let client = RpcClient::from_cookie("http://127.0.0.1:1", &cookie_path).unwrap();
        let wallet = client.for_wallet("miner");
        let dead = ("__cookie__".to_string(), "first".to_string());
        let fresh = ("__cookie__".to_string(), "second".to_string());
        std::fs::write(&cookie_path, "__cookie__:second").unwrap();

        // The first 401 adopts the file, for every clone of the client.
        assert_eq!(client.reload_cookie_after_401(&dead), Some(fresh.clone()));
        assert_eq!(wallet.creds(), fresh);
        // A second request that had sent the dead cookie still gets to replay.
        assert_eq!(client.reload_cookie_after_401(&dead), Some(fresh.clone()));
        assert_eq!(wallet.reload_cookie_after_401(&dead), Some(fresh.clone()));
        // A request that sent the fresh cookie and was still refused: real.
        assert_eq!(client.reload_cookie_after_401(&fresh), None);
        // A cookie file that vanished (btxd shutting down) is not a reload.
        std::fs::remove_file(&cookie_path).unwrap();
        assert_eq!(client.reload_cookie_after_401(&dead), None);
        // And a client built from an explicit user/pass has nothing to reload.
        let explicit = RpcClient::new("http://127.0.0.1:1", "u", "p");
        assert_eq!(explicit.reload_cookie_after_401(&dead), None);
    }

    #[test]
    fn a_cookie_line_is_user_colon_password_and_nothing_else_parses() {
        assert_eq!(
            parse_cookie("__cookie__:abc123\n"),
            Some(("__cookie__".to_string(), "abc123".to_string()))
        );
        // The first colon splits; a password may carry one.
        assert_eq!(
            parse_cookie("  u:p:q  "),
            Some(("u".to_string(), "p:q".to_string()))
        );
        assert_eq!(parse_cookie(""), None);
        assert_eq!(parse_cookie("nocolon"), None);
    }
}
