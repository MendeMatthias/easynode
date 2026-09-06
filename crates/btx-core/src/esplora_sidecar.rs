//! Start and stop the two processes that turn a full node into an Esplora
//! endpoint: electrs (the index and the REST API, on localhost) and the Caddy
//! front (CORS, rate limits, freshness headers, the only thing a wallet talks
//! to). Both run BESIDE btxd as children of the app, the way `NodeController`
//! runs btxd. `deploy/esplora/` carries the same two as systemd units for a
//! server without the app.
//!
//! What this deliberately does not do:
//!   - download binaries. electrs is a fork built from `deploy/esplora/electrs`
//!     and the Caddy needs a plugin (`deploy/esplora/build-caddy.sh`); the app
//!     looks for both on PATH and in the usual prefixes and names the script
//!     that builds a missing one. An app that fetched a binary it cannot verify
//!     would be a bigger trust problem than the one it solved.
//!   - listen on a public name by default. The front starts on
//!     [`DEFAULT_LISTEN`], plain HTTP on localhost, so the operator can run
//!     `scripts/verify-esplora.sh` against it first. A hostname is a setting.
//!   - decide freshness. [`crate::esplora_freshness`] writes the markers; the
//!     front answers `unverified` until it has, and this module writes that
//!     marker before the front exists so there is no window with none.
//!   - check the prune posture. That is [`crate::esplora`]'s job, before
//!     anything here is called.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

/// The front's configuration, verbatim from the deploy directory so the app
/// and the systemd deployment can never drift apart. The app writes it next
/// to the datadir and hands Caddy the two environment variables it reads.
pub const CADDYFILE_TEMPLATE: &str = include_str!("../../../deploy/esplora/Caddyfile.template");

/// Where electrs serves the Esplora REST API (localhost only; Caddy fronts it).
pub const ELECTRS_HTTP: &str = "127.0.0.1:3000";
pub const ELECTRS_HTTP_BASE: &str = "http://127.0.0.1:3000";
/// electrs's Electrum protocol port, also localhost only.
pub const ELECTRS_ELECTRUM: &str = "127.0.0.1:50001";
/// btxd's JSON-RPC, the same endpoint the app uses (`setup::RPC_URL`).
pub const BTXD_RPC: &str = "127.0.0.1:19334";
/// Where the front listens until an operator gives it a name.
pub const DEFAULT_LISTEN: &str = "http://127.0.0.1:3080";
pub const ELECTRS_BIN: &str = "electrs";
pub const CADDY_BIN: &str = "caddy";
/// How long a graceful stop may take before the children are killed. electrs
/// flushes rocksdb on SIGTERM; Caddy finishes in-flight requests.
///
/// TEN, not twenty. This runs inside the app's quit path, which budgets about
/// 95 s for btxd's shielded-state flush — and that flush is the long pole:
/// cutting it short leaves an in-flight mutation marker and costs a multi-minute
/// rebuild on the next start. electrs flushing rocksdb is worth waiting for,
/// but not out of btxd's budget.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

/// How long to watch a freshly spawned child before believing it started.
/// Long enough for a bind failure or a config rejection, short enough that
/// nobody notices it in a Settings toggle.
pub const STARTUP_WATCH: Duration = Duration::from_millis(1200);

pub fn esplora_dir(datadir: &Path) -> PathBuf {
    datadir.join("esplora")
}
/// The freshness markers. Under the datadir because the app does not run as
/// root; the systemd deployment uses /run (`BTX_ESPLORA_RUN`).
pub fn run_dir(datadir: &Path) -> PathBuf {
    esplora_dir(datadir).join("run")
}
pub fn db_dir(datadir: &Path) -> PathBuf {
    esplora_dir(datadir).join("electrs-db")
}
pub fn caddyfile_path(datadir: &Path) -> PathBuf {
    esplora_dir(datadir).join("Caddyfile")
}
pub fn electrs_log(datadir: &Path) -> PathBuf {
    esplora_dir(datadir).join("electrs.log")
}
pub fn caddy_log(datadir: &Path) -> PathBuf {
    esplora_dir(datadir).join("caddy.log")
}

/// Where a binary may live: PATH, then the prefixes the build scripts install
/// into, whether or not they are on the app's PATH (a desktop app launched
/// from a menu often has a shorter one than a shell).
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for fixed in ["/usr/local/bin", "/usr/bin", "/opt/homebrew/bin"] {
        dirs.push(PathBuf::from(fixed));
    }
    if let Some(home) = crate::platform::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
        dirs.push(home.join("go").join("bin"));
    }
    dirs
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The first executable named `name` in `dirs`. Pure over its inputs so the
/// lookup is tested with a temporary directory rather than trusted.
pub fn find_binary_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = crate::platform::exe_name(name);
    dirs.iter()
        .map(|d| d.join(&file))
        .find(|p| is_executable(p))
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    find_binary_in(name, &search_dirs())
}

/// What to tell an operator whose machine lacks one of the two. Names the
/// script rather than a URL: the binaries are built, not downloaded.
pub fn missing_binary_message(name: &str) -> String {
    match name {
        ELECTRS_BIN => "electrs is not installed. Build it with deploy/esplora/build-electrs.sh \
                        (it compiles the BTX fork vendored in this repository) and put it in \
                        /usr/local/bin, ~/.local/bin or on PATH."
            .to_string(),
        CADDY_BIN => {
            "caddy is not installed. This front needs a Caddy with the rate-limit plugin, \
                      which stock builds lack: deploy/esplora/build-caddy.sh builds the right one \
                      into /usr/local/bin or ~/.local/bin."
                .to_string()
        }
        other => format!("{other} is not installed"),
    }
}

/// The Caddy module this front's configuration requires. Stock Caddy does not
/// carry it; `deploy/esplora/build-caddy.sh` builds one that does.
pub const CADDY_REQUIRED_MODULE: &str = "http.handlers.rate_limit";

/// Does this caddy binary carry the rate-limit plugin the Caddyfile needs?
///
/// `find_binary` returns the first executable NAMED caddy on PATH,
/// /usr/local/bin, /usr/bin, /opt/homebrew/bin, ~/.local/bin, ~/.cargo/bin or
/// ~/go/bin, and the name is all it ever checked. A stock Caddy is extremely
/// likely to be sitting in one of those directories, and it refuses this whole
/// configuration on its first directive: `unrecognized directive: rate_limit`.
///
/// What that looked like without this check: Settings reported success and the
/// listen address, Caddy had already exited, and 30–60 s later the guardian
/// killed electrs too and said "the Caddy front exited; the log is in the
/// esplora folder" — naming no cause, on a machine where the operator had done
/// nothing wrong except have Caddy installed.
///
/// `deploy/esplora/test-front.sh` has gated itself on exactly this since it was
/// written. The app did not.
pub fn caddy_has_required_module(caddy: &Path) -> bool {
    let Ok(out) = std::process::Command::new(caddy)
        .arg("list-modules")
        .output()
    else {
        // Cannot ask: do not refuse on that alone. A binary that will not run
        // fails loudly at spawn a moment later, with its own error.
        return true;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.trim() == CADDY_REQUIRED_MODULE)
}

/// What to tell an operator whose caddy is the wrong build.
pub fn wrong_caddy_message(caddy: &Path) -> String {
    format!(
        "the caddy at {} has no {CADDY_REQUIRED_MODULE} module, so it cannot read this front's \
         configuration (it stops at `unrecognized directive: rate_limit`). Stock Caddy builds \
         lack it. Build the right one with deploy/esplora/build-caddy.sh and put it earlier on \
         PATH, or in /usr/local/bin.",
        caddy.display()
    )
}

/// The electrs command line, mirroring `deploy/esplora/electrs.service.template`
/// so the app and the unit start the same indexer.
///
/// `--daemon-dir` is the datadir: electrs reads btxd's `.cookie` there, which
/// is how the app's own conf authenticates (no rpcuser). `--jsonrpc-import`
/// fetches blocks over RPC rather than reading blk*.dat, which is safe while
/// btxd is still appending block files; the blk-file path is faster for a
/// full reindex on a finished node and is the unit's documented alternative.
pub fn electrs_args(datadir: &Path) -> Vec<String> {
    vec![
        "--network".into(),
        "mainnet".into(),
        "--daemon-dir".into(),
        datadir.display().to_string(),
        "--daemon-rpc-addr".into(),
        BTXD_RPC.into(),
        "--db-dir".into(),
        db_dir(datadir).display().to_string(),
        "--http-addr".into(),
        ELECTRS_HTTP.into(),
        "--electrum-rpc-addr".into(),
        ELECTRS_ELECTRUM.into(),
        "--cors".into(),
        "*".into(),
        "--jsonrpc-import".into(),
        "-v".into(),
    ]
}

pub fn caddy_args(caddyfile: &Path) -> Vec<String> {
    vec![
        "run".into(),
        "--config".into(),
        caddyfile.display().to_string(),
        "--adapter".into(),
        "caddyfile".into(),
    ]
}

/// Every placeholder the template reads. The upstream addresses are passed
/// explicitly rather than left to the template's defaults, so the app and the
/// config can never disagree about where electrs is: one constant decides, and
/// `electrs_args` and this function both read it.
pub fn caddy_env(listen: &str, run_dir: &Path) -> Vec<(String, String)> {
    vec![
        ("BTX_ESPLORA_HOST".into(), listen.to_string()),
        ("BTX_ESPLORA_RUN".into(), run_dir.display().to_string()),
        ("BTX_ESPLORA_ELECTRS".into(), ELECTRS_HTTP.to_string()),
        ("BTX_ESPLORA_BTXD_RPC".into(), BTXD_RPC.to_string()),
    ]
}

/// A site address Caddy will accept and that cannot smuggle anything into the
/// configuration: `http://host[:port]`, `https://host[:port]`, or a bare
/// hostname (which gets automatic HTTPS). Returns the trimmed value.
pub fn validate_listen(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("the listen address is empty".into());
    }
    if s.len() > 253 {
        return Err("the listen address is too long".into());
    }
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '"' | '\'' | '\\'))
    {
        return Err("the listen address must not contain spaces, quotes or braces".into());
    }
    let host_port = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    if host_port.contains('/') {
        return Err("the listen address is a host, not a URL with a path".into());
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, Some(p)),
        _ => (host_port, None),
    };
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err(format!("'{host}' is not a hostname or an IPv4 address"));
    }
    if host != "localhost" && !host.contains('.') {
        return Err(format!(
            "'{host}' needs a dot: a full hostname or an address"
        ));
    }
    if let Some(p) = port {
        if p.parse::<u16>().map(|n| n == 0).unwrap_or(true) {
            return Err(format!("'{p}' is not a port"));
        }
    }
    Ok(s.to_string())
}

/// Where each sidecar records its pid, so an orphan left by a force-quit can be
/// found and cleared on the next start.
pub fn sidecar_pidfile(datadir: &Path, name: &str) -> PathBuf {
    esplora_dir(datadir).join(format!("{name}.pid"))
}

/// Stop a sidecar left running by a previous app instance.
///
/// WHY THIS EXISTS. Both children are spawned with `kill_on_drop`, which needs
/// the app's own `Drop` to run. A force-quit, a crash or an OS kill skips it and
/// reparents them, exactly as happens to btxd. The difference was what came
/// next: btxd has a whole recovery apparatus (pidfile reconciliation,
/// `DatadirHolder`, `stop_unmanaged_node`, `force_kill_foreign_btxd`), while
/// electrs and caddy had none at all.
///
/// So Esplora mode was dead from that point on, and the operator was never told
/// why: every node start said "electrs exited; the log is in the esplora
/// folder", the log said the database was locked or the address was in use, and
/// nothing mentioned an orphan or offered to clear it — while the orphaned Caddy
/// carried on serving the public hostname from an index nobody was updating.
///
/// Same discipline as the btxd path: a pid is only signalled once its process
/// name confirms what it is, so a reused pid is never touched.
pub async fn reap_orphan(datadir: &Path, name: &str) {
    let pidfile = sidecar_pidfile(datadir, name);
    let Ok(txt) = std::fs::read_to_string(&pidfile) else {
        return;
    };
    let Ok(pid) = txt.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(&pidfile);
        return;
    };
    if !crate::platform::process_is_alive(pid) {
        let _ = std::fs::remove_file(&pidfile);
        return;
    }
    // Confirm identity before signalling anything.
    let is_ours = crate::platform::process_name(pid)
        .await
        .map(|c| {
            c.trim()
                .rsplit('/')
                .next()
                .map(|b| b == name)
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !is_ours {
        eprintln!(
            "[esplora] {name}.pid names live pid {pid} but that process is not {name}; \
             ignoring the stale file and leaving it alone"
        );
        let _ = std::fs::remove_file(&pidfile);
        return;
    }
    eprintln!("[esplora] a {name} from a previous run (pid {pid}) is still going; stopping it");
    // force_kill is the only signal the platform layer offers, and it is the
    // right one here. Unlike btxd, neither sidecar holds anything a flush would
    // save: electrs replays its own write-ahead log on the next start, and the
    // front is stateless. The alternative to killing it is what shipped —
    // Esplora mode permanently dead behind a locked index.
    crate::platform::force_kill(pid);
    // Wait for the index lock and the listen address to come free before the
    // caller spawns replacements, or the new pair loses the same race.
    for _ in 0..40 {
        if !crate::platform::process_is_alive(pid) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let _ = std::fs::remove_file(&pidfile);
}

fn record_pid(datadir: &Path, name: &str, child: &Child) {
    if let Some(pid) = child.id() {
        let _ =
            crate::fsx::atomic_write(&sidecar_pidfile(datadir, name), pid.to_string().as_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SidecarHealth {
    pub electrs: bool,
    pub caddy: bool,
}

impl SidecarHealth {
    pub fn all_up(&self) -> bool {
        self.electrs && self.caddy
    }
}

/// The two children. Dropping this kills both (`kill_on_drop`), so a panic or
/// an early return never leaves an orphaned index writer.
pub struct EsploraSidecars {
    electrs: Child,
    caddy: Child,
    pub listen: String,
    pub datadir: PathBuf,
}

impl EsploraSidecars {
    /// Spawn both. The caller has already passed the prune gate and found the
    /// binaries; this only arranges the directory, the marker, the config and
    /// the processes.
    pub async fn start(
        datadir: &Path,
        electrs_bin: &Path,
        caddy_bin: &Path,
        listen: &str,
    ) -> AppResult<Self> {
        let listen = validate_listen(listen).map_err(AppError::Config)?;
        for d in [esplora_dir(datadir), run_dir(datadir), db_dir(datadir)] {
            std::fs::create_dir_all(&d)
                .map_err(|e| AppError::Config(format!("cannot create {}: {e}", d.display())))?;
        }
        // Clear our own leftovers FIRST. Without this a force-quit left both
        // children running, and every later start died on a locked index or a
        // held address with no explanation.
        reap_orphan(datadir, ELECTRS_BIN).await;
        reap_orphan(datadir, CADDY_BIN).await;
        // The front answers `unverified` until the guardian says otherwise.
        // Written BEFORE the front exists so there is never a window with no
        // marker at all.
        crate::esplora_freshness::write_marker(
            &run_dir(datadir),
            crate::esplora_freshness::Freshness::Unverified,
        )
        .map_err(|e| AppError::Config(format!("cannot write the freshness marker: {e}")))?;
        let caddyfile = caddyfile_path(datadir);
        crate::fsx::atomic_write(&caddyfile, CADDYFILE_TEMPLATE.as_bytes())
            .map_err(|e| AppError::Config(format!("cannot write {}: {e}", caddyfile.display())))?;

        let mut electrs = spawn(
            electrs_bin,
            &electrs_args(datadir),
            &[],
            &electrs_log(datadir),
        )?;
        record_pid(datadir, ELECTRS_BIN, &electrs);
        let caddy = match spawn(
            caddy_bin,
            &caddy_args(&caddyfile),
            &caddy_env(&listen, &run_dir(datadir)),
            &caddy_log(datadir),
        ) {
            Ok(c) => {
                record_pid(datadir, CADDY_BIN, &c);
                c
            }
            Err(e) => {
                let _ = electrs.start_kill();
                let _ = std::fs::remove_file(sidecar_pidfile(datadir, ELECTRS_BIN));
                return Err(e);
            }
        };
        let mut me = Self {
            electrs,
            caddy,
            listen,
            datadir: datadir.to_path_buf(),
        };
        // A spawn that succeeds says the process STARTED, not that it lived.
        // Both of these die immediately on the ordinary mistakes: a port
        // already bound, a Caddyfile the binary will not adapt, an electrs
        // that refuses a pruned datadir. Without this the caller reported
        // success for a front that was already gone, and the operator learned
        // it from a Settings row minutes later.
        tokio::time::sleep(STARTUP_WATCH).await;
        let health = me.health();
        if !health.all_up() {
            let which = if !health.electrs {
                "electrs"
            } else {
                "the Caddy front"
            };
            let log = if !health.electrs {
                electrs_log(datadir)
            } else {
                caddy_log(datadir)
            };
            me.stop().await;
            return Err(AppError::Process(format!(
                "{which} exited immediately after starting. Its log is {}",
                log.display()
            )));
        }
        Ok(me)
    }

    /// Which of the two is still running. `try_wait` reaps a dead child.
    pub fn health(&mut self) -> SidecarHealth {
        SidecarHealth {
            electrs: alive(&mut self.electrs),
            caddy: alive(&mut self.caddy),
        }
    }

    /// Ask both to stop, wait [`STOP_GRACE`], then kill what is left.
    pub async fn stop(&mut self) {
        terminate(&mut self.caddy);
        terminate(&mut self.electrs);
        let wait = async {
            let _ = self.caddy.wait().await;
            let _ = self.electrs.wait().await;
        };
        if tokio::time::timeout(STOP_GRACE, wait).await.is_err() {
            let _ = self.caddy.start_kill();
            let _ = self.electrs.start_kill();
        }
    }
}

fn alive(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// SIGTERM where there is one, so electrs can flush its database; a plain
/// kill elsewhere.
fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            // SAFETY: kill(2) with a pid we spawned and still own; it only
            // sends a signal.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            return;
        }
    }
    let _ = child.start_kill();
}

fn spawn(bin: &Path, args: &[String], envs: &[(String, String)], log: &Path) -> AppResult<Child> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    // Both children log to a file under <datadir>/esplora, appended across
    // runs: electrs's index progress is the thing an operator will want to
    // read after a long first sync.
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        Ok(f) => {
            let err = f
                .try_clone()
                .map_err(|e| AppError::Process(format!("cannot clone log handle: {e}")))?;
            cmd.stdout(std::process::Stdio::from(f));
            cmd.stderr(std::process::Stdio::from(err));
        }
        Err(e) => eprintln!(
            "[esplora] could not open {}: {e}; inheriting stdio",
            log.display()
        ),
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.kill_on_drop(true);
    cmd.spawn()
        .map_err(|e| AppError::Process(format!("cannot start {}: {e}", bin.display())))
}

#[cfg(test)]
mod tests {

    /// A pidfile naming a process that is NOT the sidecar must never be
    /// signalled — the same rule the btxd path applies, for the same reason: a
    /// pid the OS has reused belongs to somebody else.
    #[tokio::test]
    async fn reap_orphan_ignores_a_pid_that_is_not_ours_and_clears_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(esplora_dir(dir.path())).unwrap();
        // Our own pid, which is certainly alive and certainly not named electrs.
        let pidfile = sidecar_pidfile(dir.path(), ELECTRS_BIN);
        std::fs::write(&pidfile, std::process::id().to_string()).unwrap();
        reap_orphan(dir.path(), ELECTRS_BIN).await;
        assert!(!pidfile.exists(), "the stale pidfile should be cleared");
        // And we are still here, which is the point.
        assert!(crate::platform::process_is_alive(std::process::id()));
    }

    #[tokio::test]
    async fn reap_orphan_tolerates_a_missing_or_junk_pidfile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(esplora_dir(dir.path())).unwrap();
        // Missing: nothing to do, no panic.
        reap_orphan(dir.path(), CADDY_BIN).await;
        // Junk: cleared rather than parsed into a signal.
        let pidfile = sidecar_pidfile(dir.path(), CADDY_BIN);
        std::fs::write(&pidfile, "not-a-pid").unwrap();
        reap_orphan(dir.path(), CADDY_BIN).await;
        assert!(!pidfile.exists());
    }

    #[test]
    fn the_wrong_caddy_message_names_the_binary_and_the_builder() {
        let m = wrong_caddy_message(Path::new("/usr/local/bin/caddy"));
        assert!(
            m.contains("/usr/local/bin/caddy"),
            "must name which caddy: {m}"
        );
        assert!(m.contains(CADDY_REQUIRED_MODULE));
        assert!(
            m.contains("build-caddy.sh"),
            "must say how to get the right one: {m}"
        );
    }
    use super::*;

    #[test]
    fn the_template_carries_the_placeholders_the_app_sets() {
        for p in [
            "{$BTX_ESPLORA_HOST}",
            "{$BTX_ESPLORA_RUN:/run}",
            "{$BTX_ESPLORA_ELECTRS:127.0.0.1:3000}",
            "{$BTX_ESPLORA_BTXD_RPC:127.0.0.1:19334}",
        ] {
            assert!(CADDYFILE_TEMPLATE.contains(p), "the template lost {p}");
        }
        let env = caddy_env("http://127.0.0.1:3080", Path::new("/x/run"));
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "BTX_ESPLORA_HOST",
                "BTX_ESPLORA_RUN",
                "BTX_ESPLORA_ELECTRS",
                "BTX_ESPLORA_BTXD_RPC"
            ]
        );
        // One constant decides where electrs listens; the flag we pass electrs
        // and the address we hand the front must never drift apart.
        let by_name = |k: &str| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(by_name("BTX_ESPLORA_ELECTRS"), ELECTRS_HTTP);
        assert!(electrs_args(Path::new("/d")).contains(&ELECTRS_HTTP.to_string()));
        // The front needs the plugin; the message for a missing caddy says so.
        assert!(CADDYFILE_TEMPLATE.contains("rate_limit"));
        assert!(missing_binary_message(CADDY_BIN).contains("rate-limit plugin"));
        assert!(missing_binary_message(ELECTRS_BIN).contains("build-electrs.sh"));
    }

    #[test]
    fn the_rate_limit_is_ordered_ahead_of_the_directive_that_terminates_routing() {
        // A bare `rate_limit` beside `handle` blocks is ordered AFTER them by
        // `order rate_limit before reverse_proxy`, because `handle` sorts
        // before `reverse_proxy` — and `handle` terminates routing, so the
        // limiter never runs. It fails silently: every request is served, every
        // header is right, and there is no rate limit. Caught by
        // deploy/esplora/test-front.sh; pinned here so a future edit of the
        // ordering line has to argue with a test.
        assert!(
            CADDYFILE_TEMPLATE.contains("order rate_limit before handle"),
            "rate_limit must be ordered before `handle`, or it is dead configuration"
        );
    }

    #[test]
    fn electrs_args_mirror_the_unit() {
        let args = electrs_args(Path::new("/home/u/.easybtx"));
        let joined = args.join(" ");
        for needed in [
            "--network mainnet",
            "--daemon-dir /home/u/.easybtx",
            "--daemon-rpc-addr 127.0.0.1:19334",
            "--db-dir /home/u/.easybtx/esplora/electrs-db",
            "--http-addr 127.0.0.1:3000",
            "--electrum-rpc-addr 127.0.0.1:50001",
            "--cors *",
            "--jsonrpc-import",
        ] {
            assert!(joined.contains(needed), "missing {needed} in {joined}");
        }
        let c = caddy_args(Path::new("/x/Caddyfile"));
        assert_eq!(
            c,
            vec!["run", "--config", "/x/Caddyfile", "--adapter", "caddyfile"]
        );
    }

    #[test]
    fn everything_lives_under_the_datadir() {
        let d = Path::new("/data");
        for p in [
            run_dir(d),
            db_dir(d),
            caddyfile_path(d),
            electrs_log(d),
            caddy_log(d),
        ] {
            assert!(p.starts_with("/data/esplora"), "{p:?}");
        }
    }

    #[test]
    fn find_binary_in_wants_an_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        let name = crate::platform::exe_name("electrs");
        assert_eq!(find_binary_in("electrs", &[dir.path().to_path_buf()]), None);
        let p = dir.path().join(&name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Not executable yet: not found.
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(find_binary_in("electrs", &[dir.path().to_path_buf()]), None);
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            find_binary_in(
                "electrs",
                &[PathBuf::from("/nonexistent"), dir.path().to_path_buf()]
            ),
            Some(p)
        );
        // A directory of that name is not a binary.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir2.path().join(&name)).unwrap();
        assert_eq!(
            find_binary_in("electrs", &[dir2.path().to_path_buf()]),
            None
        );
    }

    #[test]
    fn listen_addresses_are_hosts_not_config() {
        for ok in [
            "http://127.0.0.1:3080",
            "https://esplora-2.easybtx.com",
            "esplora-1.easybtx.com",
            "localhost:3080",
            "  http://localhost:8080  ",
        ] {
            assert!(validate_listen(ok).is_ok(), "{ok} should be accepted");
        }
        assert_eq!(
            validate_listen("  http://localhost:8080  ").unwrap(),
            "http://localhost:8080"
        );
        for bad in [
            "",
            "a b",
            "{$X}",
            "http://",
            "foo\n",
            "esplora",
            "http://x.y/path",
            "\"x.y\"",
            "x.y:0",
            "x.y:port",
        ] {
            assert!(validate_listen(bad).is_err(), "{bad:?} should be refused");
        }
    }
}
