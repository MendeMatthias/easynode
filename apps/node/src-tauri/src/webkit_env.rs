//! How WebKitGTK renders under WSL, chosen by the app before any web view
//! exists.
//!
//! On WSLg the window goes through Xwayland and an RDP session to Windows,
//! with the GPU behind WSLg's d3d12 bridge. With WebKit's accelerated
//! compositing on, the live window froze after the RDP session went idle
//! (2026-11-26; see `window_watchdog`), and the web process drew about a fifth
//! of a core around the clock. The app never set a `WEBKIT_*` variable, so
//! every WSL install ran that way.
//!
//! Under WSL the app now sets [`WSL_DEFAULT`] itself, unless the person (or
//! a launcher script) already exported a `WEBKIT_DISABLE_*` variable, which is
//! respected as it stands. [`OVERRIDE_VAR`] chooses explicitly:
//!
//! * `gpu`: WebKit's own defaults (what every release before did). Both
//!   `WEBKIT_DISABLE_*` variables are removed, because an update relaunch
//!   inherits the environment the previous run set;
//! * `software`: `WEBKIT_DISABLE_COMPOSITING_MODE=1` (and the other removed);
//! * `no-dmabuf`: `WEBKIT_DISABLE_DMABUF_RENDERER=1` (and the other removed);
//! * `auto`, empty or unset: [`WSL_DEFAULT`] under WSL, nothing elsewhere.
//!
//! Nothing changes outside WSL unless the override asks for it.

pub const OVERRIDE_VAR: &str = "EASYBTX_NODE_WEBKIT";
pub const DISABLE_COMPOSITING: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";
pub const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// The variable set under WSL when nothing says otherwise. On the live WSL
/// box on 2026-11-26 the window kept repainting 66 minutes into a display
/// idle with it, and froze about 15 minutes into one without it (PR #246).
pub const WSL_DEFAULT: &str = DISABLE_COMPOSITING;

/// What the app does to the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Set this variable to 1 and remove the other `WEBKIT_DISABLE_*`.
    Set(&'static str),
    /// Remove both `WEBKIT_DISABLE_*` variables: WebKit's defaults.
    Clear,
    /// Leave the environment alone, for this reason.
    Leave(&'static str),
}

/// Whether this is WSL: the kernel says "microsoft" in its release string
/// (`/proc/sys/kernel/osrelease`), or WSL's own `WSL_DISTRO_NAME` is set.
pub fn is_wsl(osrelease: &str, distro: Option<&str>) -> bool {
    osrelease.to_ascii_lowercase().contains("microsoft")
        || distro.is_some_and(|d| !d.trim().is_empty())
}

/// The choice for this host. `already_set` is whether either
/// `WEBKIT_DISABLE_*` variable is in the environment already.
pub fn choose(wsl: bool, override_value: Option<&str>, already_set: bool) -> Choice {
    let wanted = override_value
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match wanted.as_str() {
        "gpu" => Choice::Clear,
        "software" => Choice::Set(DISABLE_COMPOSITING),
        "no-dmabuf" => Choice::Set(DISABLE_DMABUF),
        _ if already_set => Choice::Leave("a WEBKIT_DISABLE_* variable is already set"),
        _ if wsl => Choice::Set(WSL_DEFAULT),
        _ => Choice::Leave("not WSL"),
    }
}

/// Whether this process runs under WSL (Linux only; false elsewhere).
pub fn running_under_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        is_wsl(&osrelease, std::env::var("WSL_DISTRO_NAME").ok().as_deref())
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Apply the choice. Runs first thing in `run()`, while the process has one
/// thread and before GTK or WebKit read the environment. Returns the line it
/// printed, for setup.log.
#[cfg(target_os = "linux")]
pub fn apply() -> String {
    let wsl = running_under_wsl();
    let override_value = std::env::var(OVERRIDE_VAR).ok();
    let already_set = [DISABLE_COMPOSITING, DISABLE_DMABUF]
        .iter()
        .any(|v| std::env::var_os(v).is_some());
    let line = match choose(wsl, override_value.as_deref(), already_set) {
        Choice::Set(var) => {
            for other in [DISABLE_COMPOSITING, DISABLE_DMABUF] {
                if other != var {
                    std::env::remove_var(other);
                }
            }
            std::env::set_var(var, "1");
            format!(
                "web view rendering: {var}=1 ({}; {OVERRIDE_VAR}=gpu turns it off)",
                if wsl { "WSL" } else { "asked by the override" }
            )
        }
        Choice::Clear => {
            std::env::remove_var(DISABLE_COMPOSITING);
            std::env::remove_var(DISABLE_DMABUF);
            format!("web view rendering: WebKit defaults ({OVERRIDE_VAR}=gpu; any inherited WEBKIT_DISABLE_* removed)")
        }
        Choice::Leave(why) => format!("web view rendering: as the environment says ({why})"),
    };
    println!("[node-app] {line}");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_is_read_from_the_kernel_release() {
        assert!(is_wsl("6.18.33.2-microsoft-standard-WSL2\n", None));
        assert!(is_wsl("4.4.0-19041-Microsoft", None));
        assert!(!is_wsl("6.8.0-49-generic", None));
    }

    #[test]
    fn wsl_is_read_from_the_distro_variable() {
        assert!(is_wsl("6.8.0-49-generic", Some("Ubuntu")));
        assert!(!is_wsl("6.8.0-49-generic", Some("  ")));
    }

    #[test]
    fn wsl_gets_software_compositing_by_default() {
        assert_eq!(choose(true, None, false), Choice::Set(DISABLE_COMPOSITING));
        assert_eq!(
            choose(true, Some("auto"), false),
            Choice::Set(DISABLE_COMPOSITING)
        );
        assert_eq!(
            choose(true, Some(""), false),
            Choice::Set(DISABLE_COMPOSITING)
        );
    }

    #[test]
    fn nothing_changes_outside_wsl() {
        assert!(matches!(choose(false, None, false), Choice::Leave(_)));
    }

    // The live box's start-node.sh exports WEBKIT_DISABLE_COMPOSITING_MODE=1
    // itself: the app adds nothing on top.
    #[test]
    fn a_variable_someone_already_set_is_respected() {
        assert!(matches!(choose(true, None, true), Choice::Leave(_)));
    }

    #[test]
    fn the_override_chooses_explicitly() {
        assert_eq!(choose(true, Some("gpu"), false), Choice::Clear);
        assert_eq!(
            choose(false, Some("software"), false),
            Choice::Set(DISABLE_COMPOSITING)
        );
        assert_eq!(
            choose(true, Some("no-dmabuf"), false),
            Choice::Set(DISABLE_DMABUF)
        );
        assert_eq!(
            choose(true, Some("No-DMABuf"), true),
            Choice::Set(DISABLE_DMABUF)
        );
    }

    // An update relaunch inherits the WEBKIT_DISABLE_COMPOSITING_MODE=1 the
    // previous run set; `gpu` must take it away, not just skip setting it.
    #[test]
    fn gpu_removes_an_inherited_variable() {
        assert_eq!(choose(true, Some("gpu"), true), Choice::Clear);
        assert_eq!(choose(false, Some(" GPU "), true), Choice::Clear);
    }

    #[test]
    fn an_unknown_override_value_is_automatic() {
        assert_eq!(choose(true, Some("fast"), false), Choice::Set(WSL_DEFAULT));
        assert!(matches!(
            choose(false, Some("fast"), false),
            Choice::Leave(_)
        ));
    }
}
