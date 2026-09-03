//! Windows platform implementation. Only compiled under `#[cfg(windows)]`.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;

/// `CREATE_NO_WINDOW` — suppress the console window Windows would otherwise pop
/// for a console subprocess (`cmd`, `tasklist`, …). Without it these helpers
/// flash a black command window. GUI processes like `explorer` ignore it.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir() // %USERPROFILE%
}

pub fn data_dir() -> Option<PathBuf> {
    // %APPDATA%\easyBTX (Roaming). Falls back to %USERPROFILE%\easyBTX.
    dirs::data_dir().or_else(home_dir).map(|d| d.join("easyBTX"))
}

pub fn free_disk_bytes(path: &std::path::Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_avail: u64 = 0;
    // SAFETY: GetDiskFreeSpaceExW writes the caller-available free byte count
    // into `free_avail`; `wide` is null-terminated. The other out-params are
    // null (we don't need total/total-free). On failure (0) we return 0.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return 0;
    }
    free_avail
}

pub fn exe_name(stem: &str) -> String {
    if stem.ends_with(".exe") {
        stem.to_string()
    } else {
        format!("{stem}.exe")
    }
}

pub fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    // explorer.exe sometimes returns a nonzero exit even on success, but a
    // successful spawn is all we assert here. explorer is a GUI process so it
    // wouldn't flash a console, but we set the flag uniformly for consistency.
    std::process::Command::new("explorer")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    // `cmd /C start "" <url>` hands the URL to the default browser. The empty
    // "" is the window-title arg that `start` consumes positionally, so a URL
    // with spaces/quotes isn't mistaken for the title.
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

pub fn reveal_path(path: &std::path::Path) -> std::io::Result<()> {
    // `explorer /select,<path>` opens the parent folder with the file
    // selected. The comma is part of explorer's argument syntax, so the
    // switch and the path form ONE argument.
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(path.as_os_str());
    std::process::Command::new("explorer")
        .arg(arg)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

pub fn open_private_append(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    // NTFS ACLs confine %APPDATA% to the user; there is no O_NOFOLLOW analogue
    // we need here. Plain create+append.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

pub fn open_private_write(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
}

pub fn process_is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259; // STATUS_PENDING — the process has not exited.
    // SAFETY: OpenProcess returns a null handle on failure (process gone / no
    // access). On success we read its exit code and always close the handle.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    unsafe { CloseHandle(handle) };
    ok != 0 && code == STILL_ACTIVE
}

pub async fn process_name(pid: u32) -> Option<String> {
    // `tasklist /FI "PID eq <pid>" /FO CSV /NH` → a CSV row whose first field is
    // the image name, e.g. `"btxd.exe","1234",...`. Parse the first quoted field
    // and strip the .exe so the caller's bare-name `btxd` comparison matches.
    let out = tokio::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim();
    // "btxd.exe","1234",... → btxd.exe
    let image = first.trim_start_matches('"').split('"').next()?.trim();
    if image.is_empty() || image.eq_ignore_ascii_case("INFO:") {
        return None; // tasklist prints "INFO: No tasks..." when the pid is gone.
    }
    let stem = image.strip_suffix(".exe").unwrap_or(image);
    (!stem.is_empty()).then(|| stem.to_string())
}

pub async fn parent_pid(pid: u32) -> Option<u32> {
    // Walk the process table and read `th32ParentProcessID`. There is no
    // `getppid`-for-another-process on Win32; the toolhelp snapshot is the
    // supported way, and it is the same call `tasklist`/WMI use underneath.
    //
    // WHY THIS HAD TO STOP BEING A STUB. It returned `None` while the node app
    // shipped mac-only, and `holder_is_orphaned(None, _)` is `false`, so
    // `holder_managed` was UNCONDITIONALLY true on Windows whenever a live btxd
    // held the datadir. That made two branches of `pre_launch_plan`
    // unreachable, and both of them are on the self-update path this release
    // opens up:
    //
    //   * `ClearStaleHolder` — the updater exits via `std::process::exit(0)`,
    //     which runs no destructors, so tokio's `kill_on_drop` never fires and
    //     btxd.exe SURVIVES the update as an orphan. If it is not answering RPC
    //     when the new app launches (warmup, mid-flush), the plan fell through
    //     to `ManagedElsewhereNoRpc`: a dead end with a wrong message and no
    //     self-recovery. With a real ppid the orphan is recognised and stopped.
    //
    //   * `RestartForUpgrade` — reachable only when `!holder_managed`. A
    //     Windows client coming from 0.6.0 moves NODE_RELEASE_TAG
    //     v0.33.2 → v0.33.3-pr105b, so it must restart the daemon to pick the
    //     new binaries up. Without this it answered `Attach` instead: the app
    //     updated, reported the new version, and went on running the old
    //     fork-blind btxd parked at 184,999 — the exact failure the upgrade
    //     exists to end.
    //
    // Windows never reparents orphans (no init to adopt them), so the orphan
    // signature is "the recorded parent pid is dead", which `holder_is_orphaned`
    // already encodes via `parent_alive`.
    //
    // ⚠ PID REUSE. Windows recycles pids, so a dead parent's pid can name an
    // unrelated live process, and the holder then reads as managed. That is the
    // CONSERVATIVE direction — we decline to stop a btxd we cannot prove is
    // orphaned, which is the same answer the stub gave — so the failure mode is
    // no worse than before, just far rarer. Do not "fix" it by assuming a
    // missing parent means orphaned.
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // Synchronous, but a process-table snapshot is a few milliseconds and this
    // runs once per launch decision, so it does not earn a spawn_blocking hop.
    // SAFETY: the snapshot handle is checked against INVALID_HANDLE_VALUE and
    // closed on every path. `entry` is fully initialised (zeroed, then dwSize
    // set) before Process32FirstW, which REQUIRES dwSize and fails otherwise.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut found = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            if entry.th32ProcessID == pid {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    found
}

pub fn force_kill(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: open with terminate rights; null handle on failure. Always close.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if handle.is_null() {
        return;
    }
    unsafe {
        TerminateProcess(handle, 1);
        CloseHandle(handle);
    }
}
