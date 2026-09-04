//! macOS / Linux platform implementation. Only compiled under `#[cfg(unix)]`.

use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn data_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".easybtx"))
}

pub fn free_disk_bytes(path: &std::path::Path) -> u64 {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
        return 0;
    };
    // SAFETY: statvfs writes into a zeroed stack buffer and only reads the
    // null-terminated `cpath`. On any error we return 0.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    // f_bavail = blocks available to a non-superuser; f_frsize = their size.
    // u128 multiply guards against overflow on very large volumes; clamp back
    // into u64 (a real volume's free bytes never exceeds u64).
    let avail_bytes = (st.f_bavail as u128) * (st.f_frsize as u128);
    avail_bytes.min(u64::MAX as u128) as u64
}

pub fn exe_name(stem: &str) -> String {
    stem.to_string()
}

pub fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    let program = if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        "open"
    };
    std::process::Command::new(program).arg(path).spawn().map(|_| ())
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        "open"
    };
    std::process::Command::new(program).arg(url).spawn().map(|_| ())
}

/// (program, args) to reveal `path` in the file manager — pure, for tests.
fn reveal_command(path: &std::path::Path) -> (&'static str, Vec<std::ffi::OsString>) {
    if cfg!(target_os = "linux") {
        // No cross-desktop selection standard: open the containing dir.
        let dir = path.parent().unwrap_or(path);
        ("xdg-open", vec![dir.as_os_str().to_os_string()])
    } else {
        ("open", vec!["-R".into(), path.as_os_str().to_os_string()])
    }
}

pub fn reveal_path(path: &std::path::Path) -> std::io::Result<()> {
    let (program, args) = reveal_command(path);
    std::process::Command::new(program).args(args).spawn().map(|_| ())
}

pub fn open_private_append(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

pub fn open_private_write(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

pub fn process_is_alive(pid: u32) -> bool {
    // Signal 0 = existence/permission probe; no signal is delivered.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

pub fn boot_time() -> Option<std::time::SystemTime> {
    boot_epoch_secs().map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s))
}

/// Seconds since the epoch at which this boot started.
///
/// Linux states it outright: `/proc/stat` carries a `btime <seconds>` line
/// written by the kernel. Deriving it from `/proc/uptime` instead would be a
/// subtraction against a clock that NTP moves after boot, which is exactly the
/// error this check must not make.
#[cfg(target_os = "linux")]
fn boot_epoch_secs() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    stat.lines()
        .find_map(|l| l.strip_prefix("btime "))
        .and_then(|v| v.trim().parse().ok())
}

/// macOS has no `/proc`. `sysctl -n kern.boottime` prints a struct timeval:
/// `{ sec = 1756900000, usec = 123456 } Fri Sep  4 02:46:07 2026`. We take the
/// `sec` field. Any surprise in that format parses to `None`, which turns the
/// pidfile-age check off rather than feeding it a wrong number — see
/// `platform::boot_time` for why that is the safe direction.
#[cfg(not(target_os = "linux"))]
fn boot_epoch_secs() -> Option<u64> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", "kern.boottime"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let after_sec = text.split("sec =").nth(1)?;
    let digits: String = after_sec
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

pub async fn process_name(pid: u32) -> Option<String> {
    // `ps -p <pid> -o comm=` → command name (Linux) / exe path (macOS). The
    // caller compares the basename, so a path is fine.
    let out = tokio::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("comm=")
        .output()
        .await
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

pub async fn parent_pid(pid: u32) -> Option<u32> {
    // `ps -p <pid> -o ppid=` → the parent pid, space-padded. Empty output when
    // the pid names no process. Same subprocess pattern as `process_name`.
    let out = tokio::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("ppid=")
        .output()
        .await
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

pub fn force_kill(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reveal_command_selects_file_or_opens_parent() {
        let p = std::path::Path::new("/tmp/dir/file.txt");
        let (prog, args) = super::reveal_command(p);
        if cfg!(target_os = "linux") {
            // No cross-desktop "select in file manager" standard on Linux:
            // we open the containing directory instead.
            assert_eq!(prog, "xdg-open");
            assert_eq!(args, vec![std::ffi::OsString::from("/tmp/dir")]);
        } else {
            assert_eq!(prog, "open");
            assert_eq!(args[0], std::ffi::OsString::from("-R"));
            assert_eq!(args[1], std::ffi::OsString::from("/tmp/dir/file.txt"));
        }
    }
}
