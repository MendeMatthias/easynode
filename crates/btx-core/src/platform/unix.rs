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
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        "open"
    };
    std::process::Command::new(program)
        .arg(url)
        .spawn()
        .map(|_| ())
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
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
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

/// Linux: utime plus stime from `/proc/<pid>/stat`, in clock ticks.
#[cfg(target_os = "linux")]
pub async fn process_cpu_time(pid: u32) -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = proc_stat_cpu_ticks(&stat)?;
    // SAFETY: sysconf only reads a system constant.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (hz > 0).then(|| std::time::Duration::from_secs_f64(ticks as f64 / hz as f64))
}

/// macOS has no `/proc`. `ps -p <pid> -o time=` prints the CPU time the
/// process has used, user plus system, as `mm:ss.cc`. Same subprocess pattern
/// as `process_name`.
#[cfg(not(target_os = "linux"))]
pub async fn process_cpu_time(pid: u32) -> Option<std::time::Duration> {
    let out = tokio::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("time=")
        .output()
        .await
        .ok()?;
    ps_cpu_time(String::from_utf8_lossy(&out.stdout).trim())
}

/// utime plus stime, fields 14 and 15 of a `/proc/<pid>/stat` line. Field 2 is
/// the command name in parentheses and may itself contain spaces or `)`, so
/// the fields are counted from the LAST `)`: what follows it starts at field 3.
#[cfg(any(target_os = "linux", test))]
fn proc_stat_cpu_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

/// ps's `time` column, `[[dd-]hh:]mm:ss[.cc]`. macOS prints `mm:ss.cc`
/// (`12:34.56`); procps prints `[dd-]hh:mm:ss` (`01:02:03`, `2-01:02:03`).
#[cfg(any(not(target_os = "linux"), test))]
fn ps_cpu_time(s: &str) -> Option<std::time::Duration> {
    let (days, clock) = match s.split_once('-') {
        Some((d, rest)) => (d.parse::<u64>().ok()?, rest),
        None => (0, s),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let (last, whole) = parts.split_last()?;
    let secs: f64 = last.parse().ok()?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    // Minutes, or hours then minutes, before the seconds.
    let mut minutes: u64 = 0;
    for p in whole {
        minutes = minutes * 60 + p.parse::<u64>().ok()?;
    }
    Some(
        std::time::Duration::from_secs(days * 86_400 + minutes * 60)
            + std::time::Duration::from_secs_f64(secs),
    )
}

pub fn force_kill(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn proc_stat_cpu_ticks_counts_from_the_last_paren() {
        // A real line from a btxd, then one whose command name carries a
        // space and a `)`, which would shift every field if counted from the
        // first `)`.
        let real = "4242 (btxd) S 1 4242 4242 0 -1 4194560 91232 0 12 0 1500 250 0 0 20 0 30 0 \
                    1234 5678 910 18446744073709551615";
        assert_eq!(super::proc_stat_cpu_ticks(real), Some(1750));
        let odd = "7 (b) x) R 1 7 7 0 -1 0 0 0 0 0 40 2 0 0 20 0 1 0 1 1 1";
        assert_eq!(super::proc_stat_cpu_ticks(odd), Some(42));
        assert_eq!(super::proc_stat_cpu_ticks("garbage"), None);
        assert_eq!(super::proc_stat_cpu_ticks("1 (btxd) S 1 2 3"), None);
    }

    #[test]
    fn ps_cpu_time_reads_both_formats() {
        let s = Duration::from_secs;
        // macOS
        assert_eq!(
            super::ps_cpu_time("0:00.50"),
            Some(Duration::from_millis(500))
        );
        assert_eq!(super::ps_cpu_time("12:34.00"), Some(s(12 * 60 + 34)));
        // procps
        assert_eq!(super::ps_cpu_time("01:02:03"), Some(s(3600 + 2 * 60 + 3)));
        assert_eq!(super::ps_cpu_time("2-01:02:03"), Some(s(2 * 86_400 + 3723)));
        // An empty column is a process that is gone, never zero CPU.
        assert_eq!(super::ps_cpu_time(""), None);
        assert_eq!(super::ps_cpu_time("1:2:3:4"), None);
        assert_eq!(super::ps_cpu_time("ab:cd"), None);
    }

    /// The live reader agrees with a process that just burned CPU: this test's
    /// own process, after a busy loop.
    #[tokio::test]
    async fn process_cpu_time_reads_a_live_process() {
        let pid = std::process::id();
        let before = super::process_cpu_time(pid).await.expect("own CPU time");
        let t = std::time::Instant::now();
        let mut x: u64 = 0;
        while t.elapsed() < Duration::from_millis(300) {
            x = std::hint::black_box(x.wrapping_add(1));
        }
        let after = super::process_cpu_time(pid).await.expect("own CPU time");
        assert!(after >= before, "{after:?} < {before:?}");
        assert!(
            after - before >= Duration::from_millis(100),
            "300 ms of busy loop read as {:?} of CPU",
            after - before
        );
    }

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
