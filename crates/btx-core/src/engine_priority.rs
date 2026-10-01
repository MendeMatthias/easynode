//! How hard the node engine is asked to step back for the person using the
//! computer.
//!
//! ── WHY THIS EXISTS ─────────────────────────────────────────────────────────
//! Owners on an M4 MacBook Air and an M4 MacBook Pro, with easyNode and the
//! easyBTX miner open, found the Mac almost unusable. The miner is its own
//! product; this module is about easyNode never being part of that problem.
//! Until 0.7.2 the app started btxd at the same priority as the window the
//! person is typing in, and on Apple Silicon btxd validates MatMul proofs on
//! the GPU (Metal), which is also the chip that draws the screen.
//!
//! ── WHAT WAS MEASURED ───────────────────────────────────────────────────────
//! `docs/mac-engine-priority.md` has the numbers (M2 Pro, engine v0.34.12,
//! the start-up GPU check that runs on every start, a fixed Metal job and a
//! fixed CPU job in the foreground). In short: at normal priority the
//! foreground Metal job took about 3.2 times its idle time on average while
//! the engine ran its GPU check, and most samples were slow; under macOS's
//! background policy 2 to 2.7 times on average, the median sample was back at
//! the idle time, and the slow samples came in bursts (the engine's long GPU
//! jobs still run once started). `nice` did not help the GPU at all, because
//! the GPU does not look at `nice`. The cost: the check took 1.5 to 2.1 times
//! as long (148 to 219 s against 84 to 110 s), still far inside the app's
//! ten-minute start budget, and plain CPU work in the background policy runs
//! on the efficiency cores only (a fixed single-thread job took 3.1 times as
//! long there).
//!
//! ── WHAT THIS DOES ──────────────────────────────────────────────────────────
//! On macOS the app puts btxd into the Darwin background policy right after
//! spawning it: `setpriority(PRIO_DARWIN_PROCESS, pid, PRIO_DARWIN_BG)`, the
//! same call `taskpolicy -b` makes. macOS then runs its threads on the
//! efficiency cores, throttles its disk I/O behind other programs' I/O, marks
//! its sockets as background traffic, and gives its GPU work low priority.
//!
//! One exception, because of the efficiency-core cost above: while the node
//! is far behind the chain ([`FAR_BEHIND_BLOCKS`]), the app takes it back out
//! of the background policy so catching up is not slowed down several times
//! over, and puts it back once it is near the tip ([`NEAR_TIP_BLOCKS`]). The
//! gap between the two keeps it from switching back and forth. A node at the
//! tip validates one block about every 90 seconds, which is where the
//! background policy costs least and helps most.
//!
//! The engine's own worker counts stay the engine's (`node.rs`, the comment
//! above `BTX_MATMUL_BACKEND`); this changes who goes first, not how much.
//!
//! Linux and Windows are unchanged in this release: nothing was measured
//! there, and the reports are all from Macs.
//!
//! ── SIGNERS ─────────────────────────────────────────────────────────────────
//! A node that signs (`btx_core::signer`, on by default on every validating
//! Mac) gets the same policy, on purpose. An exception would leave exactly the
//! reported Macs unchanged. The policy delays the engine while something in
//! the foreground wants the same chip; it does not stop it. Under a
//! continuous foreground GPU load (a stand-in for a miner at normal priority)
//! the start-up GPU check still finished, in 139 and 174 s against 126 and
//! 153 s at normal priority. A block arrives about every 90 seconds, so a
//! signature made somewhat later still lands well before the next block. That
//! is reasoning from the start-up check, not a measurement of signing itself;
//! the doc says so.

/// Behind the best known header by more than this many blocks, the node is
/// catching up, and it runs at normal priority so the catch-up is not slowed
/// to the efficiency cores. About 12 hours of chain at 90 seconds a block: a
/// Mac that slept overnight stays in the background policy.
pub const FAR_BEHIND_BLOCKS: u64 = 500;

/// Within this many blocks of the best known header, the node is at the tip
/// and goes (back) into the background policy.
pub const NEAR_TIP_BLOCKS: u64 = 10;

/// Which scheduling policy the app gives the engine process it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePriority {
    /// Leave the engine at the priority it inherits from the app.
    Normal,
    /// macOS background policy (`PRIO_DARWIN_BG`): efficiency cores,
    /// throttled disk I/O, background network traffic, low GPU priority.
    Background,
}

impl EnginePriority {
    /// The words the start log uses for this policy.
    pub fn describe(self) -> &'static str {
        match self {
            EnginePriority::Normal => "normal priority",
            EnginePriority::Background => {
                "macOS background policy (the Mac goes first: efficiency cores, \
                 throttled disk, low GPU priority)"
            }
        }
    }
}

/// The policy for the engine on operating system `os` (the value of
/// `std::env::consts::OS`), `blocks_behind` its best known header (`None` at
/// spawn, before anything is known), while it currently runs under
/// `current`. Pure, so the choice is tested on every platform.
///
/// At spawn it is the background policy, so the start-up GPU check, which
/// runs on every start, already gives way. Between [`NEAR_TIP_BLOCKS`] and
/// [`FAR_BEHIND_BLOCKS`] the current policy stays.
///
/// The role does not enter into it: see SIGNERS in the module docs for why a
/// signing node gets the same policy.
pub fn engine_priority_for(
    os: &str,
    blocks_behind: Option<u64>,
    current: EnginePriority,
) -> EnginePriority {
    if os != "macos" {
        return EnginePriority::Normal;
    }
    match blocks_behind {
        None => EnginePriority::Background,
        Some(b) if b > FAR_BEHIND_BLOCKS => EnginePriority::Normal,
        Some(b) if b <= NEAR_TIP_BLOCKS => EnginePriority::Background,
        Some(_) => current,
    }
}

/// Apply `priority` to the running process `pid`. An error is returned as
/// text for the log; the caller keeps the node running either way, because a
/// node at the wrong priority is still a working node.
///
/// Off macOS, [`EnginePriority::Normal`] is `Ok(())` and changes nothing;
/// on macOS it takes the process out of the background policy, which a
/// process may do to its own user's processes without extra rights
/// (`taskpolicy -B`).
pub fn apply_engine_priority(pid: u32, priority: EnginePriority) -> Result<(), String> {
    match priority {
        EnginePriority::Normal if !cfg!(target_os = "macos") => Ok(()),
        EnginePriority::Normal => set_background(pid, false),
        EnginePriority::Background => set_background(pid, true),
    }
}

/// The policy a freshly spawned engine gets on this operating system,
/// before anything about the chain is known.
pub fn engine_priority_at_spawn() -> EnginePriority {
    engine_priority_for(std::env::consts::OS, None, EnginePriority::Normal)
}

/// Re-decide the policy of the running engine `pid` from how far behind it
/// is, and apply it when it changes. `current` is the policy the caller last
/// set on it, or `None` when the caller does not know (a node adopted from a
/// previous app instance): then the chosen policy is applied regardless, and
/// between the two lines that is the spawn policy. Returns the policy in
/// force afterwards; on an error, the error text, and the caller should
/// assume nothing changed.
///
/// The caller remembers `current`, because macOS does not report another
/// process's background state through `getpriority` (it reads 0 for any pid
/// but the caller's own, measured on macOS 26.6).
pub fn retune_engine_priority(
    pid: u32,
    blocks_behind: u64,
    current: Option<EnginePriority>,
) -> Result<EnginePriority, String> {
    let assumed = current.unwrap_or_else(engine_priority_at_spawn);
    let wanted = engine_priority_for(std::env::consts::OS, Some(blocks_behind), assumed);
    if current == Some(wanted) {
        return Ok(wanted);
    }
    apply_engine_priority(pid, wanted).map(|()| wanted)
}

/// Whether process `pid` runs in the background policy, read the way
/// `ps -o pri` reads it: the task's base priority drops to 4 there (measured
/// on an M2 Pro, macOS 26.6: 26 or 31 before, 4 after `setpriority`, back
/// after clearing it). `None` when the process is gone or off macOS. For
/// tests and diagnostics; the policy decisions never read it back.
#[cfg(target_os = "macos")]
pub fn is_background(pid: u32) -> Option<bool> {
    // SAFETY: zeroed is a valid proc_taskinfo (plain integers).
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: `info` is a properly sized, writable proc_taskinfo.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (n == size).then_some(info.pti_priority <= 4)
}

/// Off macOS there is no background policy to read.
#[cfg(not(target_os = "macos"))]
pub fn is_background(_pid: u32) -> Option<bool> {
    None
}

#[cfg(target_os = "macos")]
fn set_background(pid: u32, on: bool) -> Result<(), String> {
    let prio = if on { libc::PRIO_DARWIN_BG } else { 0 };
    // SAFETY: setpriority takes plain integers and touches no memory of ours.
    let rc = unsafe { libc::setpriority(libc::PRIO_DARWIN_PROCESS, pid as libc::id_t, prio) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn set_background(_pid: u32, _on: bool) -> Result<(), String> {
    Err("the background policy exists only on macOS".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use EnginePriority::{Background, Normal};

    #[test]
    fn a_mac_starts_the_engine_in_the_background_policy() {
        assert_eq!(engine_priority_for("macos", None, Normal), Background);
        assert_eq!(engine_priority_for("macos", None, Background), Background);
    }

    #[test]
    fn linux_and_windows_are_unchanged_in_this_release() {
        for os in ["linux", "windows", ""] {
            assert_eq!(engine_priority_for(os, None, Normal), Normal);
            assert_eq!(engine_priority_for(os, Some(0), Background), Normal);
            assert_eq!(engine_priority_for(os, Some(100_000), Normal), Normal);
        }
    }

    #[test]
    fn a_mac_far_behind_catches_up_at_normal_priority() {
        let far = FAR_BEHIND_BLOCKS + 1;
        assert_eq!(engine_priority_for("macos", Some(far), Background), Normal);
        assert_eq!(engine_priority_for("macos", Some(far), Normal), Normal);
        // Exactly at the line is not yet far behind.
        assert_eq!(
            engine_priority_for("macos", Some(FAR_BEHIND_BLOCKS), Background),
            Background
        );
    }

    #[test]
    fn a_mac_at_the_tip_gives_way() {
        for b in [0, 1, NEAR_TIP_BLOCKS] {
            assert_eq!(engine_priority_for("macos", Some(b), Normal), Background);
            assert_eq!(
                engine_priority_for("macos", Some(b), Background),
                Background
            );
        }
    }

    #[test]
    fn between_the_lines_the_current_policy_stays() {
        // Catching up from far behind stays at normal priority until the tip;
        // a node at the tip that falls a little behind stays in the background.
        for b in [NEAR_TIP_BLOCKS + 1, 200, FAR_BEHIND_BLOCKS] {
            assert_eq!(engine_priority_for("macos", Some(b), Normal), Normal);
            assert_eq!(
                engine_priority_for("macos", Some(b), Background),
                Background
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_normal_priority_changes_nothing() {
        assert_eq!(apply_engine_priority(0, Normal), Ok(()));
        assert_eq!(retune_engine_priority(1, 0, Some(Normal)), Ok(Normal));
        assert_eq!(retune_engine_priority(1, 0, None), Ok(Normal));
    }

    #[test]
    fn the_log_line_names_the_policy() {
        assert!(EnginePriority::Background
            .describe()
            .contains("background policy"));
        assert_eq!(EnginePriority::Normal.describe(), "normal priority");
    }

    /// The real calls on a real child: a `sleep` spawned here goes into the
    /// background policy, comes out of it when far behind and goes back at
    /// the tip, and this test process is never touched.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_policy_lands_on_the_child_and_only_there() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep spawns");
        let pid = child.id();
        let me_before = is_background(std::process::id());
        let mut seen = vec![is_background(pid)];
        let applied = apply_engine_priority(pid, Background);
        seen.push(is_background(pid));
        let still_tip = retune_engine_priority(pid, 0, Some(Background));
        let far = retune_engine_priority(pid, FAR_BEHIND_BLOCKS + 1, Some(Background));
        seen.push(is_background(pid));
        let between = retune_engine_priority(pid, 200, Some(Normal));
        let tip = retune_engine_priority(pid, 0, Some(Normal));
        seen.push(is_background(pid));
        let me_after = is_background(std::process::id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(applied, Ok(()));
        assert_eq!(still_tip, Ok(Background));
        assert_eq!(far, Ok(Normal));
        assert_eq!(between, Ok(Normal));
        assert_eq!(tip, Ok(Background));
        assert_eq!(seen, vec![Some(false), Some(true), Some(false), Some(true)]);
        assert_eq!(me_before, Some(false));
        assert_eq!(me_after, Some(false));
    }

    /// A node whose policy the caller does not know (adopted from a previous
    /// app instance) gets the chosen policy applied, not assumed: far behind
    /// it is lifted, at the tip and between the lines it goes into the
    /// background, whatever it ran at before.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unknown_policy_is_applied_not_assumed() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep spawns");
        let pid = child.id();
        apply_engine_priority(pid, Background).unwrap();
        let far = retune_engine_priority(pid, FAR_BEHIND_BLOCKS + 1, None);
        let far_seen = is_background(pid);
        let between = retune_engine_priority(pid, 200, None);
        let between_seen = is_background(pid);
        apply_engine_priority(pid, Normal).unwrap();
        let tip = retune_engine_priority(pid, 0, None);
        let tip_seen = is_background(pid);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!((far, far_seen), (Ok(Normal), Some(false)));
        assert_eq!((between, between_seen), (Ok(Background), Some(true)));
        assert_eq!((tip, tip_seen), (Ok(Background), Some(true)));
    }

    #[test]
    fn the_spawn_policy_is_the_unknown_chain_policy() {
        assert_eq!(
            engine_priority_at_spawn(),
            engine_priority_for(std::env::consts::OS, None, Normal)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_process_that_is_gone_is_an_error_not_a_panic() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(apply_engine_priority(pid, Background).is_err());
        // And a retune on it reports the error rather than a change.
        assert!(retune_engine_priority(pid, 0, Some(Normal)).is_err());
    }
}
