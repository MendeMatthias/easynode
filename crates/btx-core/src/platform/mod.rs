//! Cross-platform OS abstractions.
//!
//! Every OS difference (home/data paths, free-disk query, "open in file
//! manager", private file opens, executable naming) lives behind these
//! functions so call sites stay platform-agnostic. macOS/Linux are served by
//! `unix.rs`; Windows by `windows.rs`. Adding a new platform = add a module +
//! a `#[cfg]` arm here, with no churn at the call sites.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// User home directory. macOS/Linux: `$HOME`. Windows: `%USERPROFILE%`.
pub fn home_dir() -> Option<PathBuf> {
    imp::home_dir()
}

/// EasyBTX data directory. macOS/Linux: `~/.easybtx`. Windows:
/// `%APPDATA%\easyBTX`. This is the root the datadir, pool log, and
/// canonical-names file live under.
pub fn data_dir() -> Option<PathBuf> {
    imp::data_dir()
}

/// Bytes available (to a non-privileged user) on the filesystem holding `path`,
/// via `statvfs` (unix) / `GetDiskFreeSpaceExW` (Windows). Returns `0` on any
/// failure, which callers treat as "not measured". Syscall-based on every
/// platform — no `df` subprocess — so the disk preflight measures on Windows too.
pub fn free_disk_bytes(path: &Path) -> u64 {
    imp::free_disk_bytes(path)
}

/// Megabytes available on the filesystem holding `path`. Derived from
/// [`free_disk_bytes`]; `0` on failure = "not measured".
pub fn free_disk_mb(path: &Path) -> u64 {
    free_disk_bytes(path) / (1024 * 1024)
}

/// Resolve an executable file name for this platform: appends `.exe` on
/// Windows, identity on macOS/Linux.
pub fn exe_name(stem: &str) -> String {
    imp::exe_name(stem)
}

/// Reveal a path in the OS file manager (Finder / Explorer / xdg). `Ok` once the
/// opener process spawns; callers may surface the error to the user.
pub fn open_path(path: &Path) -> io::Result<()> {
    imp::open_path(path)
}

/// Open a URL in the default browser. `Ok` once the opener process spawns.
pub fn open_url(url: &str) -> io::Result<()> {
    imp::open_url(url)
}

/// Reveal a file in the OS file manager, selecting it where the platform
/// supports selection (Finder `open -R`, Explorer `/select,`); Linux opens
/// the containing directory (`xdg-open` — no cross-DE selection standard).
/// `Ok` once the opener process spawns.
pub fn reveal_path(path: &Path) -> io::Result<()> {
    imp::reveal_path(path)
}

/// Open a private file for append, creating it. On unix this is owner-only
/// (`0600`) and refuses to follow a symlink at the path (`O_NOFOLLOW`); on
/// Windows the user-profile ACL provides the equivalent confinement.
pub fn open_private_append(path: &Path) -> io::Result<File> {
    imp::open_private_append(path)
}

/// Open a private file for truncate+write, creating it. Same hardening as
/// [`open_private_append`].
pub fn open_private_write(path: &Path) -> io::Result<File> {
    imp::open_private_write(path)
}

/// Whether `pid` is a currently-live process. unix: `kill(pid, 0)`; Windows:
/// `OpenProcess` + `GetExitCodeProcess`. Used to decide whether the btxd we
/// recorded in our pidfile is still running (else we'd needlessly restart it).
pub fn process_is_alive(pid: u32) -> bool {
    imp::process_is_alive(pid)
}

/// The executable/command name of `pid` (no path, no extension), or `None` on
/// any error. unix: `ps -p <pid> -o comm=`; Windows: `tasklist`. Used to confirm
/// a pid is really btxd before force-killing it, so a reused pid is never killed.
pub async fn process_name(pid: u32) -> Option<String> {
    imp::process_name(pid).await
}

/// Force-terminate `pid` (last resort). unix: `SIGKILL`; Windows:
/// `TerminateProcess`. Best-effort; callers must confirm the pid is the intended
/// target (alive + named btxd) first — this does no safety checking itself.
pub fn force_kill(pid: u32) {
    imp::force_kill(pid)
}

/// The parent pid of `pid`, or `None` if it can't be read (the process is gone).
/// unix: `ps -p <pid> -o ppid=`. Windows: a `CreateToolhelp32Snapshot` walk for
/// `th32ParentProcessID`.
///
/// Used to tell an ORPHANED btxd (parent gone → safe to stop/adopt) from one a
/// live app is supervising; callers must treat `None` as "assume managed".
/// Implemented on both platforms since 0.6.6 — while Windows returned a blanket
/// `None`, `holder_is_orphaned` read every Windows holder as managed, which made
/// `ClearStaleHolder` and `RestartForUpgrade` unreachable there and left the
/// self-update path unable to stop the btxd it orphans. See the long note in
/// `platform/windows.rs`.
pub async fn parent_pid(pid: u32) -> Option<u32> {
    imp::parent_pid(pid).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-process oracle: our own pid's parent, as reported by the ps-based
    /// helper, must match what the kernel tells us directly via getppid().
    #[cfg(unix)]
    #[tokio::test]
    async fn parent_pid_of_our_own_process_matches_getppid() {
        let got = parent_pid(std::process::id()).await;
        assert_eq!(got, Some(std::os::unix::process::parent_id()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parent_pid_of_a_dead_pid_is_none() {
        // Spawn a child, let it exit, and REAP it (an unreaped zombie still
        // shows up in ps with a readable ppid): its pid then names no process.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        let _ = child.wait();
        assert_eq!(parent_pid(pid).await, None);
    }

    /// Real-process oracle for Windows, mirroring the unix getppid() test: a
    /// child we spawn OURSELVES must report our pid as its parent. This is the
    /// test that would have failed against the old `None` stub, and it runs on
    /// the native windows-latest runner via the node CI's btx-core test step.
    #[cfg(windows)]
    #[tokio::test]
    async fn parent_pid_of_a_child_we_spawned_is_our_own_pid() {
        // `ping -n 6 127.0.0.1` sits still for ~5s without needing a console.
        let mut child = std::process::Command::new("ping")
            .args(["-n", "6", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn ping");
        let pid = child.id();
        let got = parent_pid(pid).await;
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            got,
            Some(std::process::id()),
            "a child we spawned must name us as its parent; a None here means the \
             toolhelp walk regressed and every Windows holder reads as managed"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn parent_pid_of_a_dead_pid_is_none() {
        // An exited process leaves the toolhelp snapshot entirely, so its pid
        // names nothing. `None` here is "unknown", which callers treat as
        // managed — the conservative direction.
        //
        // ⚠️ Windows recycles pids aggressively. On a busy runner the pid of the
        // cmd.exe we just reaped can be handed to an unrelated process before we
        // look it up, and then `parent_pid` correctly reports THAT process's
        // parent. Asserting once made this test fail by luck: on 2026-08-21 it
        // failed with `left: Some(3828), right: None` on a PR that changed only
        // version strings, while the identical commit passed in another run.
        //
        // One sample cannot tell reuse from a regression. Several can: reuse is
        // chance, so a healthy `parent_pid` returns None almost immediately,
        // whereas a `parent_pid` that never reports None for a dead pid burns
        // every attempt and still fails. The assertion keeps its teeth; it just
        // stops firing on coincidence.
        const ATTEMPTS: usize = 8;
        for attempt in 1..=ATTEMPTS {
            let mut child = std::process::Command::new("cmd")
                .args(["/c", "exit"])
                .spawn()
                .expect("spawn cmd");
            let pid = child.id();
            let _ = child.wait();
            match parent_pid(pid).await {
                None => return,
                Some(ppid) => assert!(
                    attempt < ATTEMPTS,
                    "parent_pid({pid}) still named a parent ({ppid}) after {ATTEMPTS} \
                     freshly-reaped pids. Pid reuse cannot plausibly explain that many in \
                     a row, so the toolhelp walk is reporting dead pids as live and every \
                     orphaned holder would read as managed."
                ),
            }
        }
    }

    /// The whole point of implementing it: an orphaned holder must now be
    /// RECOGNISED as orphaned rather than defaulting to managed.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_live_child_is_not_orphaned_while_we_are_alive() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "6", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn ping");
        let pid = child.id();
        let ppid = parent_pid(pid).await;
        let parent_alive = ppid.map(process_is_alive).unwrap_or(false);
        let orphaned = crate::node::holder_is_orphaned(ppid, parent_alive);
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            !orphaned,
            "we are its live parent, so it is supervised, not orphaned"
        );
    }

    #[test]
    fn data_dir_is_absolute_and_named_easybtx() {
        let d = data_dir().expect("data_dir resolves on the test host");
        let s = d.to_string_lossy().to_lowercase();
        assert!(
            s.ends_with("easybtx"),
            "data_dir should end in (.)easybtx, got {d:?}"
        );
        assert!(d.is_absolute(), "data_dir must be absolute, got {d:?}");
    }

    #[test]
    fn exe_name_is_identity_or_dot_exe() {
        let n = exe_name("btxd");
        assert!(n == "btxd" || n == "btxd.exe", "unexpected exe name {n}");
    }
}
