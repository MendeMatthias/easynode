//! Keeps the main window painting, and brings it back when it stops.
//!
//! The owner's report of 2026-11-26: under WSL the Linux window "is stuck, I
//! can't click anywhere ... in the background it runs, but in the front end it
//! looks frozen", and it always had been. Measured that day on the live app:
//! the picture stopped at about 12:30 UTC and still showed it at 18:15 while
//! the node signed blocks underneath; the app's main thread sat in poll and
//! WebKitWebProcess kept using about a fifth of a core, so frames were made
//! and never reached the screen. Minimise/restore and resize did not bring it
//! back. WSLg's weston.log had the RDP session go idle about 15 minutes before
//! the last frame.
//!
//! Nothing in the app noticed, and nothing could have: the node, the RPC poll
//! and every timer were fine. So the window now proves it is alive, three ways:
//!
//! * a timer beat from the page every 5 s (`heartbeat.ts`): the page's script
//!   runs at all;
//! * a frame beat, sent from inside a `requestAnimationFrame` callback: the
//!   page is being given frames;
//! * on Linux, a paint probe: the watchdog asks GTK to redraw the web view
//!   every tick and counts the `draw` signals that actually happen. A GTK
//!   window whose frame clock has stopped gets no draws however much the web
//!   process renders, which is what the measurements above look like.
//!
//! When the window is shown and any of these stops for [`STALL_SECS`], the web
//! view is reloaded. If that does not bring every signal back within
//! [`VERIFY_SECS`], the main window is destroyed and built again from the
//! config. It never stops, restarts or otherwise touches the node: the node is
//! owned by `AppState`, which neither path goes near, and destroying the last
//! window would otherwise ask the app to exit, which [`hold_off_exit`] holds
//! off for that one request. An automatic rebuild never takes the focus.
//!
//! It acts on its own under WSL only, where the freeze was seen; elsewhere it
//! waits for [`WATCHDOG_VAR`] (see [`watchdog_mode`]). The tray's "Reload
//! window" and a second launch rebuild the window by hand on every platform
//! that has them.
//!
//! A legitimately unseen window is not a frozen one: rAF pauses in a hidden or
//! minimised window and GTK does not draw an unmapped one. So nothing is judged
//! while the window is hidden, minimised, or its page says it is hidden, and
//! the clock starts again when it is shown. Recoveries are rate-limited
//! ([`MAX_RECOVERIES_PER_HOUR`]) and a recovery that did not help backs off for
//! an hour ([`BACKOFF_SECS`]), so it cannot loop.
//!
//! Every recovery is written to setup.log and stdout.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::state::node_datadir;

/// A shown window with no timer beat, frame beat or paint for this long is
/// frozen. Beats come every 5 s, so this is six missed beats.
pub const STALL_SECS: u64 = 30;
/// How long a reload or a rebuilt window gets to send every signal again.
pub const VERIFY_SECS: u64 = 20;
/// After the window gains focus (someone clicked it), a paint must follow
/// within this long. It is what turns "I can't click anywhere" into a
/// recovery in seconds rather than half a minute.
pub const FOCUS_PROBE_SECS: u64 = 5;
/// How often the watchdog looks.
pub const TICK_SECS: u64 = 5;
/// Reloads and rebuilds together, per rolling hour. Two whole episodes.
pub const MAX_RECOVERIES_PER_HOUR: usize = 4;
/// After a rebuild that did not help, or with the hour's budget spent, the
/// watchdog waits this long before it judges the window again.
pub const BACKOFF_SECS: u64 = 3600;
const HOUR_SECS: u64 = 3600;

/// `EASYBTX_NODE_WINDOW_WATCHDOG`: `0`/`off` off, `1`/`on` on anywhere,
/// `trace` on and printing every tick; unset is on under WSL only.
pub const WATCHDOG_VAR: &str = "EASYBTX_NODE_WINDOW_WATCHDOG";

/// Where the watchdog is in an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Watching,
    /// The web view was reloaded at `at`; waiting for every signal to return.
    Reloaded { at: u64 },
    /// The window was rebuilt at `at`; waiting for every signal to return.
    Recreated { at: u64 },
    /// Nothing more is tried until `until`.
    Backoff { until: u64 },
}

/// What the watchdog does this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Reload,
    Recreate,
    /// Stop trying for [`BACKOFF_SECS`], and say so once.
    GiveUp,
}

/// One tick's view of the window. Times are seconds on the watchdog's own
/// monotonic clock; `None` is "never".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observation {
    pub now: u64,
    /// The window is visible and not minimised.
    pub shown: bool,
    /// The page's own last word on `document.hidden`.
    pub page_hidden: bool,
    /// Last timer beat from the page.
    pub timer: Option<u64>,
    /// Last beat sent from inside a requestAnimationFrame callback.
    pub raf: Option<u64>,
    /// Whether this platform has the paint probe (Linux).
    pub paint_probe: bool,
    /// Last GTK draw of the web view.
    pub paint: Option<u64>,
    /// Nothing before this counts against the window: app start, the moment
    /// the window was last shown, or the last recovery, whichever is latest.
    pub baseline: u64,
    /// When the window last gained focus, while that probe is open.
    pub focus_probe: Option<u64>,
}

/// Whether every signal this window owes has arrived since `since`. A page
/// that says it is hidden owes nothing: no frames, no paint, and not even
/// timer beats, which browsers throttle in hidden pages (macOS WebKit does).
pub fn healthy_since(o: &Observation, since: u64) -> bool {
    if o.page_hidden {
        return true;
    }
    let after = |t: Option<u64>| t.is_some_and(|t| t >= since);
    let painted = !o.paint_probe || after(o.paint);
    after(o.timer) && after(o.raf) && painted
}

/// Whether a shown window has stopped: a signal missing for [`STALL_SECS`]
/// past the baseline, or no paint within [`FOCUS_PROBE_SECS`] of a focus.
pub fn stalled(o: &Observation) -> bool {
    if !o.shown {
        return false;
    }
    if let Some(f) = o.focus_probe {
        let from = f.max(o.baseline);
        if o.paint_probe
            && !o.page_hidden
            && o.now >= from + FOCUS_PROBE_SECS
            && !o.paint.is_some_and(|p| p >= from)
        {
            return true;
        }
    }
    if o.now < o.baseline + STALL_SECS {
        return false;
    }
    !healthy_since(o, o.now - STALL_SECS)
}

/// Whether another recovery fits in the rolling hour.
pub fn budget_left(now: u64, recent: &[u64]) -> bool {
    recent.iter().filter(|&&t| now < t + HOUR_SECS).count() < MAX_RECOVERIES_PER_HOUR
}

/// The watchdog's decision for one tick: what to do and the stage after it.
pub fn decide(o: &Observation, stage: Stage, recent: &[u64]) -> (Action, Stage) {
    match stage {
        Stage::Watching => {
            if !stalled(o) {
                (Action::None, Stage::Watching)
            } else if budget_left(o.now, recent) {
                (Action::Reload, Stage::Reloaded { at: o.now })
            } else {
                (
                    Action::GiveUp,
                    Stage::Backoff {
                        until: o.now + BACKOFF_SECS,
                    },
                )
            }
        }
        Stage::Reloaded { at } | Stage::Recreated { at } => {
            // Back, or hidden by the user: either way the episode is over.
            if !o.shown || healthy_since(o, at + 1) {
                return (Action::None, Stage::Watching);
            }
            if o.now < at + VERIFY_SECS {
                return (Action::None, stage);
            }
            if matches!(stage, Stage::Reloaded { .. }) && budget_left(o.now, recent) {
                (Action::Recreate, Stage::Recreated { at: o.now })
            } else {
                (
                    Action::GiveUp,
                    Stage::Backoff {
                        until: o.now + BACKOFF_SECS,
                    },
                )
            }
        }
        Stage::Backoff { until } => {
            if o.now >= until {
                (Action::None, Stage::Watching)
            } else {
                (Action::None, stage)
            }
        }
    }
}

/// "12 s ago", "never".
fn age(now: u64, t: Option<u64>) -> String {
    match t {
        Some(t) => format!("{} s ago", now.saturating_sub(t)),
        None => "never".to_string(),
    }
}

/// The signals as one readable clause, for setup.log.
pub fn describe(o: &Observation) -> String {
    let mut s = format!(
        "timer beat {}, frame beat {}",
        age(o.now, o.timer),
        age(o.now, o.raf)
    );
    if o.paint_probe {
        s.push_str(&format!(", paint {}", age(o.now, o.paint)));
    }
    if o.page_hidden {
        s.push_str(", page says hidden");
    }
    s
}

/// Seconds on the watchdog's monotonic clock (0 at first use).
fn secs() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
}

#[derive(Default)]
struct Inner {
    timer: Option<u64>,
    raf: Option<u64>,
    page_hidden: bool,
    stage: Stage,
    recent: Vec<u64>,
    shown_since: Option<u64>,
    last_action: u64,
    focus_probe: Option<u64>,
    /// When a rebuild left the app without a window.
    lost_at: Option<u64>,
    /// When building a lost window was last tried.
    last_build_try: Option<u64>,
}

/// The watchdog's state, managed by the app.
pub struct WindowWatch {
    inner: Mutex<Inner>,
    /// Last paint, as `secs() + 1`; 0 is never.
    paint: Arc<AtomicU64>,
    /// Set while the main window is being rebuilt, so the "last window gone"
    /// exit request that its destruction raises is not taken for a quit.
    recreating: AtomicBool,
}

impl Default for WindowWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowWatch {
    pub fn new() -> Self {
        let _ = secs();
        Self {
            inner: Mutex::new(Inner::default()),
            paint: Arc::new(AtomicU64::new(0)),
            recreating: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn last_paint(&self) -> Option<u64> {
        match self.paint.load(Ordering::Relaxed) {
            0 => None,
            v => Some(v - 1),
        }
    }
}

/// Whether the watchdog acts on its own, and how loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    On,
    /// On, and every tick printed to stdout.
    Trace,
}

/// The watchdog's mode from [`WATCHDOG_VAR`] and whether this is WSL.
///
/// On by default under WSL only, where the freeze was seen. Elsewhere a
/// shown window can legitimately stop drawing: on a Linux Wayland desktop a
/// window on another virtual desktop or behind the lock screen gets no frames
/// while GTK3 still calls it visible and not minimised, and GTK3 has no
/// occlusion state to tell the two apart. There an automatic reload or
/// rebuild would be wrong, so it waits for the override: `1`/`on` or `trace`
/// turn it on anywhere, `0`/`off` turn it off anywhere. The tray's "Reload
/// window" and a second launch work in every mode.
pub fn watchdog_mode(value: Option<&str>, wsl: bool) -> Mode {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("0") | Some("off") => Mode::Off,
        Some("1") | Some("on") => Mode::On,
        Some("trace") => Mode::Trace,
        _ if wsl => Mode::On,
        _ => Mode::Off,
    }
}

/// This process's watchdog mode.
fn mode() -> Mode {
    watchdog_mode(
        std::env::var(WATCHDOG_VAR).ok().as_deref(),
        crate::webkit_env::running_under_wsl(),
    )
}

/// Whether an exit request must be held off because the watchdog is
/// rebuilding the window: only a request without an exit code (the runtime's
/// "last window gone"), and only once per rebuild. The flag is taken by that
/// one request, so any later quit, a user's Cmd+Q included, goes through. A
/// quit with an exit code (the tray's Quit) is never held off.
pub fn hold_off_exit(code: Option<i32>, rebuilding: &AtomicBool) -> bool {
    code.is_none() && rebuilding.swap(false, Ordering::SeqCst)
}

/// [`hold_off_exit`] for this app.
pub fn hold_off_for_rebuild(app: &AppHandle, code: Option<i32>) -> bool {
    app.try_state::<WindowWatch>()
        .is_some_and(|w| hold_off_exit(code, &w.recreating))
}

/// The page's heartbeat (`heartbeat.ts`). `kind` is "timer" or "raf".
/// Answers whether to keep beating: with the watchdog off the page stops,
/// so the off switch also measures what the heartbeat itself costs.
#[tauri::command]
pub fn window_heartbeat(kind: String, hidden: bool, watch: tauri::State<'_, WindowWatch>) -> bool {
    let now = secs();
    let mut g = watch.lock();
    g.page_hidden = hidden;
    if kind == "raf" {
        g.raf = Some(now);
    } else {
        g.timer = Some(now);
    }
    mode() != Mode::Off
}

fn log(msg: &str) {
    println!("[window] {msg}");
    crate::commands::setup_log(&node_datadir(), &format!("window: {msg}"));
}

/// Start the watchdog. Called once from setup.
pub fn spawn(app: AppHandle) {
    if mode() == Mode::Off {
        println!(
            "[window] watchdog off (on by default under WSL only; {WATCHDOG_VAR}=1 turns it on)"
        );
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        install_probe(&app, &w);
    }
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(TICK_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            check(&app);
        }
    });
}

/// The window gained focus: someone clicked it. Ask for a paint now and give
/// it [`FOCUS_PROBE_SECS`] to arrive.
pub fn on_focus(app: &AppHandle) {
    let Some(watch) = app.try_state::<WindowWatch>() else {
        return;
    };
    watch.lock().focus_probe = Some(secs());
    if let Some(w) = app.get_webview_window("main") {
        probe::queue(&w);
    }
}

fn check(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        retry_lost_window(app);
        return;
    };
    let watch = app.state::<WindowWatch>();
    let shown = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
    if shown {
        probe::queue(&w);
    }
    let now = secs();
    let paint = watch.last_paint();
    let (action, obs) = {
        let mut g = watch.lock();
        if !shown {
            g.shown_since = None;
        } else if g.shown_since.is_none() {
            g.shown_since = Some(now);
        }
        let obs = Observation {
            now,
            shown,
            page_hidden: g.page_hidden,
            timer: g.timer,
            raf: g.raf,
            paint_probe: probe::SUPPORTED,
            paint,
            baseline: g.shown_since.unwrap_or(now).max(g.last_action),
            focus_probe: g.focus_probe,
        };
        let (action, stage) = decide(&obs, g.stage, &g.recent);
        if g.focus_probe
            .is_some_and(|f| now >= f + FOCUS_PROBE_SECS || !shown)
        {
            g.focus_probe = None;
        }
        g.stage = stage;
        if matches!(action, Action::Reload | Action::Recreate) {
            g.recent.retain(|&t| now < t + HOUR_SECS);
            g.recent.push(now);
            g.last_action = now;
        }
        (action, obs)
    };
    if mode() == Mode::Trace {
        println!(
            "[window] tick shown={} {} -> {:?}",
            obs.shown,
            describe(&obs),
            action
        );
    }
    match action {
        Action::None => {}
        Action::Reload => {
            log(&format!(
                "the window stopped while shown ({}); reloaded the web view. The node was not touched.",
                describe(&obs)
            ));
            if let Err(e) = w.reload() {
                log(&format!("reload failed: {e}"));
            }
        }
        Action::Recreate => {
            log(&format!(
                "the reload did not bring the window back ({}); rebuilding the window. The node was not touched.",
                describe(&obs)
            ));
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                // Automatic: never take the focus, so the window does not
                // jump in front of whatever the person is doing.
                if let Err(e) = recreate(&app, false) {
                    log(&format!("rebuilding the window failed: {e}"));
                }
            });
        }
        Action::GiveUp => log(&format!(
            "the window is still not painting ({}); no more recoveries for {} minutes. \
             The tray's \"Reload window\" or launching the app again reloads it by hand.",
            describe(&obs),
            BACKOFF_SECS / 60
        )),
    }
}

/// Reload the window on request (tray menu, second launch). Someone asking
/// by hand means the window is wrong in a way they can see, and a web view
/// whose page has stopped (a hung web process) cannot be fixed by a reload
/// sent to that same page. So this rebuilds the window, which also starts a
/// fresh page, and falls back to a plain reload only if the rebuild fails.
/// It works with the watchdog switched off too.
pub fn reload_now(app: &AppHandle, reason: &str) {
    let now = secs();
    if let Some(watch) = app.try_state::<WindowWatch>() {
        let mut g = watch.lock();
        g.recent.retain(|&t| now < t + HOUR_SECS);
        g.recent.push(now);
        g.last_action = now;
        g.stage = Stage::Recreated { at: now };
    }
    match recreate(app, true) {
        Ok(()) => log(&format!("window reloaded ({reason}): rebuilt it")),
        Err(e) => {
            log(&format!(
                "rebuilding the window failed ({reason}): {e}; reloading it instead"
            ));
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
                match w.reload() {
                    Ok(()) => log(&format!("window reloaded ({reason})")),
                    Err(e) => log(&format!("window reload failed ({reason}): {e}")),
                }
            }
        }
    }
}

/// How often a window that could not be built is tried again.
const LOST_RETRY_SECS: u64 = 60;
/// Build attempts within one rebuild.
const BUILD_ATTEMPTS: u32 = 3;

/// Build the main window from the config, a few times if it fails.
fn build_main(
    app: &AppHandle,
    position: Option<tauri::PhysicalPosition<i32>>,
    focus: bool,
) -> Result<(), String> {
    let cfg = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or_else(|| "the app config has no window".to_string())?;
    let mut last = String::new();
    for attempt in 1..=BUILD_ATTEMPTS {
        match tauri::WebviewWindowBuilder::from_config(app, &cfg).and_then(|b| b.build()) {
            Ok(w) => {
                if let Some(p) = position {
                    let _ = w.set_position(p);
                }
                if focus {
                    let _ = w.set_focus();
                }
                install_probe(app, &w);
                if let Some(watch) = app.try_state::<WindowWatch>() {
                    watch.lock().lost_at = None;
                }
                return Ok(());
            }
            Err(e) => {
                last = e.to_string();
                log(&format!(
                    "building the window failed (attempt {attempt} of {BUILD_ATTEMPTS}): {last}"
                ));
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
    if let Some(watch) = app.try_state::<WindowWatch>() {
        let mut g = watch.lock();
        if g.lost_at.is_none() {
            g.lost_at = Some(secs());
        }
    }
    log(&format!(
        "THE APP HAS NO WINDOW: building it failed {BUILD_ATTEMPTS} times ({last}). The node keeps running; \
         the watchdog tries again every {LOST_RETRY_SECS} s, and launching the app again tries at once."
    ));
    Err(format!("build: {last}"))
}

/// The window is gone because a rebuild could not build it: try again, at
/// most once a minute.
fn retry_lost_window(app: &AppHandle) {
    let Some(watch) = app.try_state::<WindowWatch>() else {
        return;
    };
    let now = secs();
    {
        let mut g = watch.lock();
        if g.lost_at.is_none() || g.last_build_try.is_some_and(|t| now < t + LOST_RETRY_SECS) {
            return;
        }
        g.last_build_try = Some(now);
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if build_main(&app, None, false).is_ok() {
            log("window built again after it was lost");
        }
    });
}

/// Destroy the main window and build it again from the config, where it was.
/// `focus` only for a rebuild someone asked for by hand.
fn recreate(app: &AppHandle, focus: bool) -> Result<(), String> {
    let watch = app.state::<WindowWatch>();
    let old = app.get_webview_window("main");
    let position = old.as_ref().and_then(|w| w.outer_position().ok());
    if let Some(w) = old {
        // Destroying the last window raises one exit request without a code;
        // this flag holds exactly that one off (`hold_off_exit`). It is
        // cleared again as soon as the new window exists.
        watch.recreating.store(true, Ordering::SeqCst);
        if let Err(e) = w.destroy() {
            watch.recreating.store(false, Ordering::SeqCst);
            return Err(format!("destroy: {e}"));
        }
        // The destruction runs on the event loop; the label is free only once
        // it has. Never called on the main thread (tray clicks hand it off),
        // so waiting here cannot hold up the loop it waits for.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.get_webview_window("main").is_some() {
            if Instant::now() > deadline {
                watch.recreating.store(false, Ordering::SeqCst);
                return Err("the old window was not destroyed within 5 s".to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let built = build_main(app, position, focus);
    watch.recreating.store(false, Ordering::SeqCst);
    built?;
    log("window rebuilt");
    Ok(())
}

/// Count this window's paints (Linux).
fn install_probe(app: &AppHandle, w: &WebviewWindow) {
    if let Some(watch) = app.try_state::<WindowWatch>() {
        probe::install(w, watch.paint.clone());
    }
}

#[cfg(target_os = "linux")]
mod probe {
    use super::*;

    pub const SUPPORTED: bool = true;

    /// Count every GTK draw of the web view. The handler runs before
    /// WebKit's own and lets it proceed.
    pub fn install(w: &WebviewWindow, paint: Arc<AtomicU64>) {
        let _ = w.with_webview(move |pw| {
            use gtk::prelude::*;
            pw.inner().connect_draw(move |_, _| {
                paint.store(secs() + 1, Ordering::Relaxed);
                gtk::glib::Propagation::Proceed
            });
        });
    }

    /// Ask GTK to redraw the web view. A window whose frame clock runs draws
    /// within a frame; a stopped one never does.
    pub fn queue(w: &WebviewWindow) {
        let _ = w.with_webview(|pw| {
            use gtk::prelude::*;
            pw.inner().queue_draw();
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod probe {
    use super::*;

    pub const SUPPORTED: bool = false;

    pub fn install(_w: &WebviewWindow, _paint: Arc<AtomicU64>) {}

    pub fn queue(_w: &WebviewWindow) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shown, healthy Linux window at `now`, every signal 2 s old.
    fn live(now: u64) -> Observation {
        Observation {
            now,
            shown: true,
            page_hidden: false,
            timer: Some(now - 2),
            raf: Some(now - 2),
            paint_probe: true,
            paint: Some(now - 2),
            baseline: 0,
            focus_probe: None,
        }
    }

    #[test]
    fn a_painting_window_is_left_alone() {
        assert_eq!(
            decide(&live(500), Stage::Watching, &[]),
            (Action::None, Stage::Watching)
        );
    }

    // The measured case: script and frames still running, GTK no longer
    // drawing. Only the paint probe sees it.
    #[test]
    fn a_window_that_stopped_painting_is_reloaded() {
        let o = Observation {
            paint: Some(400),
            ..live(500)
        };
        assert_eq!(
            decide(&o, Stage::Watching, &[]),
            (Action::Reload, Stage::Reloaded { at: 500 })
        );
    }

    #[test]
    fn frames_that_stopped_are_a_freeze() {
        let o = Observation {
            raf: Some(400),
            ..live(500)
        };
        assert!(stalled(&o));
    }

    #[test]
    fn a_dead_script_is_a_freeze() {
        let o = Observation {
            timer: Some(400),
            ..live(500)
        };
        assert!(stalled(&o));
        let never = Observation {
            timer: None,
            ..live(500)
        };
        assert!(stalled(&never));
    }

    #[test]
    fn the_stall_threshold_is_exactly_thirty_seconds() {
        let at = |paint| Observation {
            paint: Some(paint),
            ..live(500)
        };
        assert!(!stalled(&at(500 - STALL_SECS)));
        assert!(stalled(&at(500 - STALL_SECS - 1)));
    }

    // rAF pauses and GTK stops drawing when the window is hidden or
    // minimised; that is not a freeze.
    #[test]
    fn a_hidden_or_minimised_window_is_never_judged() {
        let o = Observation {
            shown: false,
            timer: None,
            raf: None,
            paint: None,
            ..live(5_000)
        };
        assert_eq!(
            decide(&o, Stage::Watching, &[]),
            (Action::None, Stage::Watching)
        );
    }

    #[test]
    fn a_page_that_says_hidden_owes_no_frames_or_paint() {
        let o = Observation {
            page_hidden: true,
            raf: Some(10),
            paint: Some(10),
            ..live(5_000)
        };
        assert!(!stalled(&o));
    }

    // macOS WebKit throttles a hidden page's timers; a hidden page owes
    // nothing at all, not even timer beats.
    #[test]
    fn a_hidden_page_owes_not_even_timer_beats() {
        let o = Observation {
            page_hidden: true,
            timer: Some(10),
            raf: None,
            paint: None,
            ..live(5_000)
        };
        assert!(!stalled(&o));
        assert_eq!(
            decide(&o, Stage::Watching, &[]),
            (Action::None, Stage::Watching)
        );
    }

    // A window shown from the tray after hours has hours-old frame beats;
    // the clock starts when it is shown.
    #[test]
    fn the_clock_starts_when_the_window_is_shown() {
        let o = Observation {
            raf: Some(10),
            paint: Some(10),
            baseline: 4_990,
            ..live(5_000)
        };
        assert!(!stalled(&o));
        let later = Observation {
            now: 4_990 + STALL_SECS + 1,
            timer: Some(4_990 + STALL_SECS),
            ..o
        };
        assert!(stalled(&later));
    }

    #[test]
    fn nothing_is_judged_in_the_first_half_minute() {
        let o = Observation {
            timer: None,
            raf: None,
            paint: None,
            baseline: 0,
            ..live(STALL_SECS - 1)
        };
        assert!(!stalled(&o));
    }

    #[test]
    fn a_click_on_a_window_that_does_not_paint_is_a_freeze_within_seconds() {
        let o = Observation {
            paint: Some(495),
            focus_probe: Some(496),
            ..live(496 + FOCUS_PROBE_SECS)
        };
        assert!(stalled(&o));
        let painted = Observation {
            paint: Some(497),
            ..o
        };
        assert!(!stalled(&painted));
        let too_soon = Observation {
            now: 496 + FOCUS_PROBE_SECS - 1,
            ..o
        };
        assert!(!stalled(&too_soon));
    }

    #[test]
    fn the_focus_probe_needs_the_paint_probe() {
        let o = Observation {
            paint_probe: false,
            paint: None,
            focus_probe: Some(496),
            ..live(496 + FOCUS_PROBE_SECS)
        };
        assert!(!stalled(&o));
    }

    #[test]
    fn a_reload_that_brings_every_signal_back_ends_the_episode() {
        let o = live(520);
        assert_eq!(
            decide(&o, Stage::Reloaded { at: 510 }, &[510]),
            (Action::None, Stage::Watching)
        );
    }

    #[test]
    fn a_reload_gets_twenty_seconds_before_the_window_is_rebuilt() {
        let stuck = |now| Observation {
            paint: Some(400),
            ..live(now)
        };
        assert_eq!(
            decide(&stuck(519), Stage::Reloaded { at: 500 }, &[500]),
            (Action::None, Stage::Reloaded { at: 500 })
        );
        assert_eq!(
            decide(&stuck(520), Stage::Reloaded { at: 500 }, &[500]),
            (Action::Recreate, Stage::Recreated { at: 520 })
        );
    }

    #[test]
    fn a_rebuild_that_does_not_help_backs_off_for_an_hour() {
        let o = Observation {
            paint: Some(400),
            ..live(540)
        };
        assert_eq!(
            decide(&o, Stage::Recreated { at: 520 }, &[500, 520]),
            (
                Action::GiveUp,
                Stage::Backoff {
                    until: 540 + BACKOFF_SECS
                }
            )
        );
        let still = Observation {
            now: 540 + BACKOFF_SECS - 1,
            ..o
        };
        assert_eq!(
            decide(
                &still,
                Stage::Backoff {
                    until: 540 + BACKOFF_SECS
                },
                &[500, 520]
            ),
            (
                Action::None,
                Stage::Backoff {
                    until: 540 + BACKOFF_SECS
                }
            )
        );
        let over = Observation {
            now: 540 + BACKOFF_SECS,
            ..o
        };
        assert_eq!(
            decide(
                &over,
                Stage::Backoff {
                    until: 540 + BACKOFF_SECS
                },
                &[500, 520]
            )
            .1,
            Stage::Watching
        );
    }

    #[test]
    fn a_window_hidden_during_an_episode_ends_it() {
        let o = Observation {
            shown: false,
            paint: Some(400),
            ..live(530)
        };
        assert_eq!(
            decide(&o, Stage::Reloaded { at: 500 }, &[500]),
            (Action::None, Stage::Watching)
        );
    }

    #[test]
    fn the_hourly_budget_cannot_be_exceeded() {
        let o = Observation {
            paint: Some(100),
            ..live(3_000)
        };
        let spent = [1_000, 1_100, 2_000, 2_100];
        assert!(!budget_left(3_000, &spent));
        assert_eq!(
            decide(&o, Stage::Watching, &spent),
            (
                Action::GiveUp,
                Stage::Backoff {
                    until: 3_000 + BACKOFF_SECS
                }
            )
        );
        // A reload with no budget left for the rebuild gives up too.
        assert_eq!(
            decide(&o, Stage::Reloaded { at: 2_100 }, &spent).0,
            Action::GiveUp
        );
    }

    #[test]
    fn the_budget_is_a_rolling_hour() {
        let spent = [100, 200, 300, 400];
        assert!(!budget_left(100 + HOUR_SECS - 1, &spent));
        assert!(budget_left(100 + HOUR_SECS, &spent));
    }

    #[test]
    fn without_a_paint_probe_the_beats_decide() {
        let o = Observation {
            paint_probe: false,
            paint: None,
            ..live(500)
        };
        assert!(!stalled(&o));
        assert!(stalled(&Observation {
            raf: Some(100),
            ..o
        }));
    }

    #[test]
    fn the_watchdog_is_on_by_default_under_wsl_only() {
        assert_eq!(watchdog_mode(None, true), Mode::On);
        assert_eq!(watchdog_mode(None, false), Mode::Off);
        assert_eq!(watchdog_mode(Some(""), false), Mode::Off);
    }

    // Ordinary Linux Wayland: another virtual desktop or the lock screen
    // stops a "visible" window's frames. It stays off there unless asked.
    #[test]
    fn the_watchdog_stays_off_outside_wsl_and_the_override_turns_it_on() {
        assert_eq!(watchdog_mode(None, false), Mode::Off);
        assert_eq!(watchdog_mode(Some("1"), false), Mode::On);
        assert_eq!(watchdog_mode(Some(" ON "), false), Mode::On);
        assert_eq!(watchdog_mode(Some("trace"), false), Mode::Trace);
    }

    #[test]
    fn the_override_turns_it_off_anywhere() {
        assert_eq!(watchdog_mode(Some("0"), true), Mode::Off);
        assert_eq!(watchdog_mode(Some(" off "), true), Mode::Off);
        assert_eq!(watchdog_mode(Some("trace"), true), Mode::Trace);
    }

    // The rebuild holds off the one "last window gone" exit request its
    // destroy raises, and nothing else.
    #[test]
    fn a_rebuild_holds_off_only_its_own_exit_request() {
        let flag = AtomicBool::new(true);
        assert!(hold_off_exit(None, &flag));
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn a_user_quit_during_a_rebuild_exits() {
        // Cmd+Q (no exit code) after the rebuild's own request was held off.
        let flag = AtomicBool::new(true);
        assert!(hold_off_exit(None, &flag));
        assert!(!hold_off_exit(None, &flag));
        // A quit with an exit code (the tray's Quit) is never held off, and
        // does not use up the rebuild's one hold-off.
        let flag = AtomicBool::new(true);
        assert!(!hold_off_exit(Some(0), &flag));
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn nothing_is_held_off_without_a_rebuild() {
        let flag = AtomicBool::new(false);
        assert!(!hold_off_exit(None, &flag));
        assert!(!hold_off_exit(Some(0), &flag));
    }

    #[test]
    fn the_log_line_names_every_signal() {
        let o = Observation {
            paint: Some(400),
            page_hidden: false,
            ..live(500)
        };
        assert_eq!(
            describe(&o),
            "timer beat 2 s ago, frame beat 2 s ago, paint 100 s ago"
        );
        let never = Observation {
            timer: None,
            paint_probe: false,
            page_hidden: true,
            ..o
        };
        assert_eq!(
            describe(&never),
            "timer beat never, frame beat 2 s ago, page says hidden"
        );
    }
}
