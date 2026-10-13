//! easyNode inside WSL2 on Windows, and how it gets its graphics card back.
//!
//! Measured on this project's RTX 3060 signer on 2026-10-13
//! (docs/incident-2026-10-11-3060-verifier-quarantine.md): a graphics driver
//! installed on the Windows host while the node runs takes the engine's GPU
//! verifier away one second after the swap. Restarting the engine does NOT
//! bring it back: the new btxd failed its startup qualification ("CUDA driver
//! version is insufficient for CUDA runtime version"), because the WSL VM keeps
//! its view of the old driver. `wsl --shutdown` and a fresh start did, in
//! seconds.
//!
//! Nothing inside the VM can restart the VM and outlive it, so the restart is a
//! short PowerShell script on the Windows side ([`restart_wsl_script`]). The app
//! launches it through WSL interop, detached, so it is no child of the VM it
//! shuts down. It then starts the distro again and relaunches easyNode, unless
//! something on the distro (a boot hook, as on the 3060) already did.

/// Is this a WSL kernel? There `/proc/sys/kernel/osrelease` reads
/// `6.18.33.2-microsoft-standard-WSL2` (the 3060, 2026-10-13); WSL1 kernels
/// say `Microsoft`.
pub fn running_in_wsl(osrelease: &str) -> bool {
    let s = osrelease.to_ascii_lowercase();
    s.contains("microsoft") || s.contains("wsl")
}

/// Does this process run inside WSL? Always false off Linux.
pub fn this_host_runs_in_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| running_in_wsl(&s))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Windows PowerShell as seen from inside WSL with the default automount.
/// Interop reached it from every context the app runs in on the 3060 (a
/// shell, a systemd --user unit, and the app's own environment; 2026-10-13).
pub const POWERSHELL_FROM_WSL: &str =
    "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";

/// The interpreter WSL registers for Windows programs (binfmt `WSLInterop`,
/// `:WSLInterop:M::MZ::/init:PF`). Called directly as
/// `/init <windows program> <args>` it starts the program the same way,
/// which is the way left when the handler is gone. Another distro booting
/// with `systemd-binfmt` wipes it for the whole VM: on the 3060 on
/// 2026-10-13 two new distros did, and the WSL restart failed with "Exec
/// format error". Reproduced that day by wiping the entry as root: a plain
/// exec failed and `/init` worked, with arguments passed unchanged
/// (`cmd.exe /c echo` printed).
pub const INTEROP_INTERPRETER: &str = "/init";

/// A distro or user name that can go into the script as it is.
fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A path that needs no quoting in sh, in a PowerShell single-quoted string,
/// or on a wsl.exe command line.
fn plain_path(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 512
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+'))
}

/// The Windows-side script that restarts WSL and brings easyNode back.
///
/// `distro` is `WSL_DISTRO_NAME` when the app has it (`None` restarts the
/// default distro: the 3060's app is started by a boot hook and has no such
/// variable). `relaunch` is the AppImage or binary to start again.
///
/// After `wsl --shutdown` it starts the distro with a process that:
/// - waits 40 s, so a boot hook on the distro starts easyNode first;
/// - starts easyNode only if neither `easybtx-node` nor the AppImage itself
///   runs. The AppImage unpacks for several seconds before `easybtx-node`
///   exists, so checking for that alone could start a second copy. The
///   AppImage check is an exact full-command-line match (`pgrep -fx`), so it
///   never matches this process, whose command line contains the same path.
///   (Live test, 2026-10-13: the 3060's boot hook checks with a plain
///   `pgrep -f`, matched this process and stood aside, and this started the
///   one instance at +40 s.)
/// - then holds the distro open with `exec sleep infinity`. WSL stops a
///   distro about 15 s after its last wsl.exe client leaves, even with
///   `vmIdleTimeout=-1`, which killed the 3060's node on 2026-10-13.
///
/// `None` when a name or the path is not plain enough to embed safely. The
/// caller then restarts the engine instead, and the message tells the person
/// what to do.
pub fn restart_wsl_script(
    distro: Option<&str>,
    linux_user: &str,
    relaunch: &str,
) -> Option<String> {
    if !plain_name(linux_user) || !plain_path(relaunch) {
        return None;
    }
    if distro.is_some_and(|d| !plain_name(d)) {
        return None;
    }
    let target = distro.map(|d| format!("-d {d} ")).unwrap_or_default();
    let sh = format!(
        "sleep 40; pgrep -x easybtx-node >/dev/null || pgrep -fx {relaunch} >/dev/null || \
         (APPIMAGE_EXTRACT_AND_RUN=1 setsid {relaunch} >/dev/null 2>&1 </dev/null &); \
         exec sleep infinity"
    );
    Some(format!(
        "Start-Sleep -Seconds 3\r\n\
         wsl.exe --shutdown\r\n\
         Start-Sleep -Seconds 8\r\n\
         Start-Process -WindowStyle Hidden -FilePath wsl.exe -ArgumentList \
         '{target}-u {linux_user} --cd ~ -e sh -c \"{sh}\"'\r\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_3060s_kernel_is_recognised_as_wsl() {
        assert!(running_in_wsl("6.18.33.2-microsoft-standard-WSL2\n"));
        assert!(running_in_wsl("4.4.0-19041-Microsoft"));
        assert!(!running_in_wsl("6.8.0-45-generic"));
        assert!(!running_in_wsl("24.6.0"));
    }

    #[test]
    fn the_script_shuts_wsl_down_then_starts_the_distro_and_holds_it() {
        let s = restart_wsl_script(
            Some("Ubuntu"),
            "bonuz",
            "/home/bonuz/BTX-Node_0.6.15_amd64.AppImage",
        )
        .unwrap();
        let shutdown = s.find("wsl.exe --shutdown").unwrap();
        let start = s.find("Start-Process").unwrap();
        assert!(shutdown < start, "{s}");
        assert!(s.contains("'-d Ubuntu -u bonuz --cd ~ -e sh -c \""), "{s}");
        // Never a second instance, and the distro stays up afterwards.
        assert!(s.contains("pgrep -x easybtx-node >/dev/null ||"), "{s}");
        assert!(
            s.contains("setsid /home/bonuz/BTX-Node_0.6.15_amd64.AppImage"),
            "{s}"
        );
        assert!(s.trim_end().ends_with("exec sleep infinity\"'"), "{s}");
        // Inside the PowerShell single-quoted argument: no single quote may
        // appear, or it would end the string early.
        let arg = s.split("-ArgumentList '").nth(1).unwrap();
        let arg = arg.trim_end().strip_suffix('\'').unwrap();
        assert!(!arg.contains('\''), "{arg}");
    }

    #[test]
    fn without_a_distro_name_the_default_distro_is_restarted() {
        let s = restart_wsl_script(None, "bonuz", "/usr/bin/easybtx-node").unwrap();
        assert!(s.contains("-ArgumentList '-u bonuz --cd ~"), "{s}");
        assert!(!s.contains("-d "), "{s}");
    }

    #[test]
    fn anything_that_would_need_quoting_is_refused() {
        let app = "/home/a/node.AppImage";
        assert!(restart_wsl_script(Some("Ubuntu"), "bonuz", "/home/a b/node.AppImage").is_none());
        assert!(restart_wsl_script(Some("Ubuntu"), "bonuz", "/home/a/x';rm -rf ~;'").is_none());
        assert!(restart_wsl_script(Some("Ubuntu"), "bonuz", "relative/path").is_none());
        assert!(restart_wsl_script(Some("Ubu ntu"), "bonuz", app).is_none());
        assert!(restart_wsl_script(Some("Ubuntu"), "bo\"nuz", app).is_none());
        assert!(restart_wsl_script(Some("Ubuntu"), "", app).is_none());
        assert!(restart_wsl_script(Some("Ubuntu-24.04"), "bonuz", app).is_some());
    }
}
