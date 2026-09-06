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
pub const STOP_GRACE: Duration = Duration::from_secs(20);

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

/// The two placeholders the template reads (`{$BTX_ESPLORA_HOST}`,
/// `{$BTX_ESPLORA_RUN:/run}`).
pub fn caddy_env(listen: &str, run_dir: &Path) -> Vec<(String, String)> {
    vec![
        ("BTX_ESPLORA_HOST".into(), listen.to_string()),
        ("BTX_ESPLORA_RUN".into(), run_dir.display().to_string()),
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
        let caddy = match spawn(
            caddy_bin,
            &caddy_args(&caddyfile),
            &caddy_env(&listen, &run_dir(datadir)),
            &caddy_log(datadir),
        ) {
            Ok(c) => c,
            Err(e) => {
                let _ = electrs.start_kill();
                return Err(e);
            }
        };
        Ok(Self {
            electrs,
            caddy,
            listen,
            datadir: datadir.to_path_buf(),
        })
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
    use super::*;

    #[test]
    fn the_template_carries_the_placeholders_the_app_sets() {
        assert!(CADDYFILE_TEMPLATE.contains("{$BTX_ESPLORA_HOST}"));
        assert!(CADDYFILE_TEMPLATE.contains("{$BTX_ESPLORA_RUN:/run}"));
        let env = caddy_env("http://127.0.0.1:3080", Path::new("/x/run"));
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["BTX_ESPLORA_HOST", "BTX_ESPLORA_RUN"]);
        // The front needs the plugin; the message for a missing caddy says so.
        assert!(CADDYFILE_TEMPLATE.contains("rate_limit"));
        assert!(missing_binary_message(CADDY_BIN).contains("rate-limit plugin"));
        assert!(missing_binary_message(ELECTRS_BIN).contains("build-electrs.sh"));
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
