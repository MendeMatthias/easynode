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
/// or on a wsl.exe command line. No `+`: it is a regex character to the
/// `pgrep -fx` that keeps a second instance from starting, so such a path
/// would never match itself there.
fn plain_path(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 512
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
}

/// The distro's name from what `wslpath -w /` prints, `\\wsl.localhost\Ubuntu\`
/// (or `\\wsl$\Ubuntu\` on older WSL). For an app started without
/// `WSL_DISTRO_NAME`, which a boot hook or a systemd unit does not set.
pub fn distro_from_unc_root(unc: &str) -> Option<String> {
    let mut parts = unc.trim().split('\\').filter(|p| !p.is_empty());
    let host = parts.next()?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && host != "wsl$" {
        return None;
    }
    let name = parts.next()?;
    plain_name(name).then(|| name.to_string())
}

/// Where the Windows-side script writes what it did, for anyone asking why
/// a node did or did not come back (`%TEMP%` of the Windows user).
pub const RESTART_LOG_NAME: &str = "easynode-wsl-restart.log";

/// The Windows-side script that restarts WSL and brings easyNode back.
///
/// `distro` is the distro's name (`WSL_DISTRO_NAME`, or
/// [`distro_from_unc_root`]); `None` restarts the default distro. `relaunch`
/// is the AppImage or binary to start again. Every step goes to
/// `%TEMP%\easynode-wsl-restart.log`, and the relaunch is tried three times.
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
        "$log = Join-Path $env:TEMP '{RESTART_LOG_NAME}'\r\n\
         function Note($m) {{ Add-Content -Path $log -Value ((Get-Date).ToUniversalTime().ToString('s') + 'Z ' + $m) }}\r\n\
         Note 'easyNode lost its GPU block verifier; restarting WSL'\r\n\
         Start-Sleep -Seconds 3\r\n\
         wsl.exe --shutdown\r\n\
         Note ('wsl --shutdown exited ' + $LASTEXITCODE)\r\n\
         Start-Sleep -Seconds 8\r\n\
         for ($i = 1; $i -le 3; $i++) {{\r\n\
         try {{\r\n\
         Start-Process -WindowStyle Hidden -FilePath wsl.exe -ArgumentList \
         '{target}-u {linux_user} --cd ~ -e sh -c \"{sh}\"' -ErrorAction Stop\r\n\
         Note ('relaunch started, attempt ' + $i); break\r\n\
         }} catch {{\r\n\
         Note ('relaunch attempt ' + $i + ' failed: ' + $_.Exception.Message); Start-Sleep -Seconds 10\r\n\
         }}\r\n\
         }}\r\n"
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
        assert!(
            s.contains("exec sleep infinity\"' -ErrorAction Stop"),
            "{s}"
        );
        // Inside the PowerShell single-quoted argument: no single quote may
        // appear, or it would end the string early.
        let arg = s.split("-ArgumentList '").nth(1).unwrap();
        let arg = arg.split("' -ErrorAction Stop").next().unwrap();
        assert!(!arg.contains('\''), "{arg}");
    }

    #[test]
    fn the_script_logs_every_step_and_retries_the_relaunch() {
        let s = restart_wsl_script(Some("Ubuntu"), "bonuz", "/usr/bin/easybtx-node").unwrap();
        assert!(
            s.contains("Join-Path $env:TEMP 'easynode-wsl-restart.log'"),
            "{s}"
        );
        assert!(
            s.contains("Note ('wsl --shutdown exited ' + $LASTEXITCODE)"),
            "{s}"
        );
        assert!(s.contains("for ($i = 1; $i -le 3; $i++) {"), "{s}");
        assert!(s.contains("} catch {"), "{s}");
        // Balanced braces: a format! slip would leave one doubled or missing.
        assert_eq!(s.matches('{').count(), s.matches('}').count(), "{s}");
        assert!(!s.contains("{{") && !s.contains("}}"), "{s}");
    }

    #[test]
    fn the_distro_name_comes_from_wslpath_when_the_variable_is_missing() {
        assert_eq!(
            distro_from_unc_root("\\\\wsl.localhost\\Ubuntu\\\n").as_deref(),
            Some("Ubuntu")
        );
        assert_eq!(
            distro_from_unc_root("\\\\wsl$\\Ubuntu-24.04\\").as_deref(),
            Some("Ubuntu-24.04")
        );
        assert_eq!(distro_from_unc_root("C:\\Users\\x"), None);
        assert_eq!(distro_from_unc_root("\\\\wsl.localhost\\"), None);
        assert_eq!(distro_from_unc_root("\\\\wsl.localhost\\Ub untu\\"), None);
    }

    /// The script through Windows PowerShell's own parser, where it can be
    /// reached (a WSL host); skipped elsewhere. A string test cannot catch a
    /// brace or quote that only PowerShell would refuse.
    #[test]
    fn the_script_parses_in_real_powershell_where_one_is_reachable() {
        if !std::path::Path::new(POWERSHELL_FROM_WSL).exists() {
            return;
        }
        let s = restart_wsl_script(
            Some("Ubuntu"),
            "bonuz",
            "/home/bonuz/BTX-Node_0.7.13_amd64.AppImage",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("restart.ps1");
        std::fs::write(&file, &s).unwrap();
        // For running the exact script by hand: EASYNODE_WSL_SCRIPT_OUT=<path>.
        if let Ok(out) = std::env::var("EASYNODE_WSL_SCRIPT_OUT") {
            std::fs::write(out, &s).unwrap();
        }
        let win = std::process::Command::new("wslpath")
            .arg("-w")
            .arg(&file)
            .output()
            .unwrap();
        let win = String::from_utf8(win.stdout).unwrap();
        let probe = format!(
            "$t=$null; $e=$null; [void][System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$t, [ref]$e); $e.Count",
            win.trim()
        );
        let out = std::process::Command::new(POWERSHELL_FROM_WSL)
            .args(["-NoProfile", "-NonInteractive", "-Command", &probe])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(errors, "0", "PowerShell parse errors in:\n{s}");
    }

    #[test]
    fn a_path_with_a_plus_is_refused_since_pgrep_reads_it_as_a_regex() {
        assert!(restart_wsl_script(Some("Ubuntu"), "bonuz", "/opt/node+1/easybtx-node").is_none());
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
