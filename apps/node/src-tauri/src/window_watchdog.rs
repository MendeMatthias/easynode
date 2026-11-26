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
//! window would otherwise ask the app to exit, which [`recreating`] holds off.
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

/// `EASYBTX_NODE_WINDOW_WATCHDOG=0` turns the watchdog off; `=trace` prints
/// every tick's signals to stdout.
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
/// that says it is hidden owes no frames and no paint.
pub fn healthy_since(o: &Observation, since: u64) -> bool {
    let after = |t: Option<u64>| t.is_some_and(|t| t >= since);
    let framed = o.page_hidden || after(o.raf);
    let painted = !o.paint_probe || o.page_hidden || after(o.paint);
    after(o.timer) && framed && painted
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

/// The page's heartbeat (`heartbeat.ts`). `kind` is "timer" or "raf".
#[tauri::command]
pub fn window_heartbeat(kind: String, hidden: bool, watch: tauri::State<'_, WindowWatch>) {
    let now = secs();
    let mut g = watch.lock();
    g.page_hidden = hidden;
    if kind == "raf" {
        g.raf = Some(now);
    } else {
        g.timer = Some(now);
    }
}

/// Whether the main window is being rebuilt right now (see
/// [`WindowWatch::recreating`]).
pub fn recreating(app: &AppHandle) -> bool {
    app.try_state::<WindowWatch>()
        .is_some_and(|w| w.recreating.load(Ordering::SeqCst))
}

fn log(msg: &str) {
    println!("[window] {msg}");
    crate::commands::setup_log(&node_datadir(), &format!("window: {msg}"));
}

fn trace_on() -> bool {
    std::env::var(WATCHDOG_VAR).as_deref() == Ok("trace")
}

/// Start the watchdog. Called once from setup.
pub fn spawn(app: AppHandle) {
    if std::env::var(WATCHDOG_VAR).as_deref() == Ok("0") {
        println!("[window] watchdog off ({WATCHDOG_VAR}=0)");
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
    if trace_on() {
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
                if let Err(e) = recreate(&app) {
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
    match recreate(app) {
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

/// Destroy the main window and build it again from the config, where it was.
fn recreate(app: &AppHandle) -> Result<(), String> {
    let watch = app.state::<WindowWatch>();
    let cfg = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or_else(|| "the app config has no window".to_string())?;
    let old = app.get_webview_window("main");
    let position = old.as_ref().and_then(|w| w.outer_position().ok());
    watch.recreating.store(true, Ordering::SeqCst);
    let clear = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        if let Some(w) = clear.try_state::<WindowWatch>() {
            w.recreating.store(false, Ordering::SeqCst);
        }
    });
    if let Some(w) = old {
        w.destroy().map_err(|e| format!("destroy: {e}"))?;
        // The destruction runs on the event loop; the label is free only once
        // it has. Never called on the main thread (tray clicks hand it off),
        // so waiting here cannot hold up the loop it waits for.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.get_webview_window("main").is_some() {
            if Instant::now() > deadline {
                return Err("the old window was not destroyed within 5 s".to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let w = tauri::WebviewWindowBuilder::from_config(app, &cfg)
        .and_then(|b| b.build())
        .map_err(|e| format!("build: {e}"))?;
    if let Some(p) = position {
        let _ = w.set_position(p);
    }
    let _ = w.set_focus();
    install_probe(app, &w);
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
        // But its script must still run.
        let dead = Observation {
            timer: Some(10),
            ..o
        };
        assert!(stalled(&dead));
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
