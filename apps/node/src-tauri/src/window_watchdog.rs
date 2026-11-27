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

use std::sync::atomic::{AtomicU64, Ordering};
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
/// How long the page's word that it is hidden is trusted without another
/// beat. A hidden page still beats (throttled: macOS WebKit slows a hidden
/// page's timers to about one a minute); a page whose web process hung right
/// after a beat that said hidden never beats again, and must not keep the
/// window out of judgement for good (#250).
pub const HIDDEN_TRUST_SECS: u64 = 300;
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
    /// The window does not have the focus. Under WSLg a minimised window
    /// still counts as shown to GTK (Wayland tells a client nothing about
    /// minimising) and gets no frames, so it looks frozen; building it again
    /// would put it back on the desktop. A window nobody is using is reloaded
    /// but not rebuilt until it has the focus again (#250).
    pub in_background: bool,
}

/// Whether the page's word that it is hidden still counts: it was said in a
/// beat within [`HIDDEN_TRUST_SECS`].
pub fn page_says_hidden(o: &Observation) -> bool {
    let last_beat = o.timer.max(o.raf);
    o.page_hidden && last_beat.is_some_and(|t| o.now < t + HIDDEN_TRUST_SECS)
}

/// Whether every signal this window owes has arrived since `since`. A page
/// that says it is hidden owes nothing: no frames, no paint, and not even
/// timer beats, which browsers throttle in hidden pages (macOS WebKit does).
pub fn healthy_since(o: &Observation, since: u64) -> bool {
    if page_says_hidden(o) {
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
            && !page_says_hidden(o)
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
            // A rebuild puts the window on the desktop; one nobody is using
            // (minimised under WSLg looks just like this) waits for the focus.
            if matches!(stage, Stage::Reloaded { .. }) && o.in_background {
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
    if page_says_hidden(o) {
        s.push_str(", page says hidden");
    } else if o.page_hidden {
        s.push_str(", page said hidden but has not beaten since");
    }
    if o.in_background {
        s.push_str(", not focused");
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
    /// When a rebuild destroyed the main window, as `secs() + 1` (0: no
    /// rebuild waiting). The "last window gone" exit request that the
    /// destruction raises is held off while this is set (`hold_off_exit`).
    destroy_armed: AtomicU64,
    /// One rebuild at a time: the tray, a second launch, "Open BTX Node"
    /// and the watchdog all go through it.
    rebuild: Mutex<()>,
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
            destroy_armed: AtomicU64::new(0),
            rebuild: Mutex::new(()),
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

/// How long after a rebuild destroyed the window its exit request is still
/// expected. A destroy that the event loop runs late (a busy main thread)
/// is still held off; past this, a request is a real quit.
pub const HOLD_OFF_SECS: u64 = 120;

/// Whether an exit request must be held off because a rebuild destroyed the
/// main window. `armed` is when it did, as `secs() + 1` (0: no rebuild). Only
/// a request without an exit code (the runtime's "last window gone"), only
/// once per rebuild (the request takes the arm, so any later quit, a user's
/// Cmd+Q included, goes through), and only within [`HOLD_OFF_SECS`]. A quit
/// with an exit code (the tray's Quit) is never held off.
pub fn hold_off_exit(code: Option<i32>, armed: &AtomicU64, now: u64) -> bool {
    if code.is_some() {
        return false;
    }
    match armed.swap(0, Ordering::SeqCst) {
        0 => false,
        v => now < (v - 1) + HOLD_OFF_SECS,
    }
}

/// Arm the hold-off, then destroy the window. A destroy that fails at once
/// raises no exit request, so the arm is taken back: left armed, it would
/// swallow the next real quit for up to [`HOLD_OFF_SECS`], and a Cmd+Q would
/// need pressing twice.
pub fn destroy_armed<E>(
    armed: &AtomicU64,
    now: u64,
    destroy: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    armed.store(now + 1, Ordering::SeqCst);
    let result = destroy();
    if result.is_err() {
        armed.store(0, Ordering::SeqCst);
    }
    result
}

/// [`hold_off_exit`] for this app. A held-off request whose window is gone
/// (a destroy that ran after the rebuild had given up waiting for it) gets
/// its window built now, so the app is never left without one.
pub fn hold_off_for_rebuild(app: &AppHandle, code: Option<i32>) -> bool {
    let held = app
        .try_state::<WindowWatch>()
        .is_some_and(|w| hold_off_exit(code, &w.destroy_armed, secs()));
    if held {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _ = ensure_window(&app, false);
        });
    }
    held
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
    let in_background = !w.is_focused().unwrap_or(true);
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
            in_background,
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

/// How often the watchdog tries again to build a window it lost.
const LOST_RETRY_SECS: u64 = 60;
/// Build attempts within one rebuild.
const BUILD_ATTEMPTS: u32 = 3;
/// How long a rebuild waits for the old window's destruction.
const DESTROY_WAIT: Duration = Duration::from_secs(5);

/// Where the new window goes: where the old one was, unless it was
/// minimised (a minimised window on Windows sits at -32000,-32000).
pub fn keep_position(
    position: Option<tauri::PhysicalPosition<i32>>,
    minimized: bool,
) -> Option<tauri::PhysicalPosition<i32>> {
    if minimized {
        None
    } else {
        position
    }
}

/// What the log says will bring a lost window back.
pub fn lost_window_hint(watchdog: Mode) -> String {
    let by_hand = "\"Open BTX Node\" in the tray menu, \"Reload window\", or launching the app again builds it";
    if watchdog == Mode::Off {
        format!("{by_hand}.")
    } else {
        format!("the watchdog tries again every {LOST_RETRY_SECS} s, and {by_hand} at once.")
    }
}

/// Build the main window from the config, a few times if it fails. Callers
/// hold `WindowWatch::rebuild`.
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
    mark_lost(app);
    log(&format!(
        "THE APP HAS NO WINDOW: building it failed {BUILD_ATTEMPTS} times ({last}). The node keeps running; {}",
        lost_window_hint(mode())
    ));
    Err(format!("build: {last}"))
}

fn mark_lost(app: &AppHandle) {
    if let Some(watch) = app.try_state::<WindowWatch>() {
        let mut g = watch.lock();
        if g.lost_at.is_none() {
            g.lost_at = Some(secs());
        }
    }
}

/// Build the main window if there is none ("Open BTX Node" on an app whose
/// rebuild failed, or a late destroy). Nothing happens when it exists.
pub fn ensure_window(app: &AppHandle, focus: bool) -> Result<(), String> {
    let watch = app.state::<WindowWatch>();
    let _one = watch.rebuild.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(w) = app.get_webview_window("main") {
        if focus {
            let _ = w.show();
            let _ = w.set_focus();
        }
        return Ok(());
    }
    build_main(app, None, focus)?;
    log("window built");
    Ok(())
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
        let _ = ensure_window(&app, false);
    });
}

/// Destroy the main window and build it again from the config, where it was.
/// `focus` only for a rebuild someone asked for by hand. One at a time.
fn recreate(app: &AppHandle, focus: bool) -> Result<(), String> {
    let watch = app.state::<WindowWatch>();
    let _one = watch.rebuild.lock().unwrap_or_else(|e| e.into_inner());
    let old = app.get_webview_window("main");
    let position = old
        .as_ref()
        .and_then(|w| keep_position(w.outer_position().ok(), w.is_minimized().unwrap_or(false)));
    if let Some(w) = old {
        // Destroying the last window raises one exit request without a code;
        // `destroy_armed` holds exactly that one off. It stays armed until
        // that request takes it, even if the destroy runs late, so a late
        // destroy can never stop the node. It is cleared once the new window
        // exists.
        if let Err(e) = destroy_armed(&watch.destroy_armed, secs(), || w.destroy()) {
            return Err(format!("destroy: {e}"));
        }
        // The destruction runs on the event loop; the label is free only once
        // it has. Never called on the main thread (tray clicks hand it off),
        // so waiting here cannot hold up the loop it waits for.
        let deadline = Instant::now() + DESTROY_WAIT;
        while app.get_webview_window("main").is_some() {
            if Instant::now() > deadline {
                // Still armed: when the destroy lands, its exit request is
                // held off and the window is built then (hold_off_for_rebuild).
                mark_lost(app);
                return Err(format!(
                    "the old window was not destroyed within {} s; it is built when the destroy lands",
                    DESTROY_WAIT.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let built = build_main(app, position, focus);
    if built.is_ok() {
        watch.destroy_armed.store(0, Ordering::SeqCst);
    }
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
            in_background: false,
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
    // no frames, no paint and no regular timer beats, only one beat within
    // HIDDEN_TRUST_SECS to say it is still hidden.
    #[test]
    fn a_hidden_page_owes_not_even_timer_beats() {
        let o = Observation {
            page_hidden: true,
            timer: Some(5_000 - HIDDEN_TRUST_SECS + 1),
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

    // #250: the web process hung right after a beat that said hidden. The
    // page never speaks again, so its last word must not keep the window out
    // of judgement for good: once it is HIDDEN_TRUST_SECS old the window is
    // judged like any other, and reloaded.
    #[test]
    fn a_hidden_beat_from_a_page_that_hung_expires() {
        let hung = |now| Observation {
            page_hidden: true,
            timer: Some(1_000),
            raf: Some(990),
            paint: Some(990),
            ..live(now)
        };
        assert!(!stalled(&hung(1_000 + HIDDEN_TRUST_SECS - 1)));
        let o = hung(1_000 + HIDDEN_TRUST_SECS);
        assert!(stalled(&o));
        assert_eq!(decide(&o, Stage::Watching, &[]).0, Action::Reload);
        assert!(describe(&o).contains("page said hidden but has not beaten since"));
        // A focus on it is judged too.
        let clicked = Observation {
            focus_probe: Some(1_000 + HIDDEN_TRUST_SECS),
            paint: Some(900),
            ..hung(1_000 + HIDDEN_TRUST_SECS + FOCUS_PROBE_SECS)
        };
        assert!(stalled(&clicked));
    }

    // #250: under WSLg a minimised window counts as shown and gets no frames.
    // It is reloaded (which leaves it minimised), but not rebuilt onto the
    // desktop while it does not have the focus; once it is focused again and
    // still frozen, it is.
    #[test]
    fn a_window_without_the_focus_is_reloaded_but_not_rebuilt_until_focused() {
        let stuck = |now, in_background| Observation {
            paint: Some(400),
            in_background,
            ..live(now)
        };
        assert_eq!(
            decide(&stuck(500, true), Stage::Watching, &[]),
            (Action::Reload, Stage::Reloaded { at: 500 })
        );
        for now in [520, 600, 4_000] {
            assert_eq!(
                decide(&stuck(now, true), Stage::Reloaded { at: 500 }, &[500]),
                (Action::None, Stage::Reloaded { at: 500 }),
                "{now}"
            );
        }
        assert_eq!(
            decide(&stuck(4_005, false), Stage::Reloaded { at: 500 }, &[500]),
            (Action::Recreate, Stage::Recreated { at: 4_005 })
        );
        assert!(describe(&stuck(500, true)).contains("not focused"));
    }

    // A destroy that failed at once raises no exit request: the next quit
    // (a Cmd+Q, without a code) must go through the first time.
    #[test]
    fn a_destroy_that_fails_does_not_hold_off_the_next_quit() {
        let armed = AtomicU64::new(0);
        assert_eq!(
            destroy_armed(&armed, 100, || Err::<(), _>("gone")),
            Err("gone")
        );
        assert!(!hold_off_exit(None, &armed, 101));
        // One that succeeded still holds off its own exit request, once.
        assert_eq!(destroy_armed(&armed, 200, || Ok::<(), &str>(())), Ok(()));
        assert!(hold_off_exit(None, &armed, 201));
        assert!(!hold_off_exit(None, &armed, 202));
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

    fn armed_at(t: u64) -> AtomicU64 {
        AtomicU64::new(t + 1)
    }

    // The rebuild holds off the one "last window gone" exit request its
    // destroy raises, and nothing else.
    #[test]
    fn a_rebuild_holds_off_only_its_own_exit_request() {
        let armed = armed_at(100);
        assert!(hold_off_exit(None, &armed, 100));
        assert_eq!(armed.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_user_quit_during_a_rebuild_exits() {
        // Cmd+Q (no exit code) after the rebuild's own request was held off.
        let armed = armed_at(100);
        assert!(hold_off_exit(None, &armed, 101));
        assert!(!hold_off_exit(None, &armed, 102));
        // A quit with an exit code (the tray's Quit) is never held off, and
        // does not use up the rebuild's one hold-off.
        let armed = armed_at(100);
        assert!(!hold_off_exit(Some(0), &armed, 101));
        assert_ne!(armed.load(Ordering::SeqCst), 0);
    }

    // The review's case A: the main thread was busy, the rebuild gave up
    // waiting after 5 s and left the hold-off armed; the destroy lands later.
    // Its exit request is held off (no exit, no node stop).
    #[test]
    fn a_destroy_that_lands_after_the_timeout_does_not_exit() {
        let armed = armed_at(100);
        let late = 100 + DESTROY_WAIT.as_secs() + 30;
        assert!(hold_off_exit(None, &armed, late));
        // And the request after it is a real quit again.
        assert!(!hold_off_exit(None, &armed, late + 1));
    }

    #[test]
    fn an_arm_nobody_used_expires() {
        let armed = armed_at(100);
        assert!(!hold_off_exit(None, &armed, 100 + HOLD_OFF_SECS));
    }

    #[test]
    fn nothing_is_held_off_without_a_rebuild() {
        let armed = AtomicU64::new(0);
        assert!(!hold_off_exit(None, &armed, 5));
        assert!(!hold_off_exit(Some(0), &armed, 5));
    }

    #[test]
    fn a_minimised_window_does_not_pass_its_off_screen_position_on() {
        let p = tauri::PhysicalPosition::new(-32000, -32000);
        assert_eq!(keep_position(Some(p), true), None);
        let q = tauri::PhysicalPosition::new(120, 80);
        assert_eq!(keep_position(Some(q), false), Some(q));
    }

    #[test]
    fn the_lost_window_log_only_promises_retries_the_watchdog_makes() {
        assert!(!lost_window_hint(Mode::Off).contains("tries again"));
        assert!(lost_window_hint(Mode::Off).contains("Open BTX Node"));
        assert!(lost_window_hint(Mode::On).contains("tries again every 60 s"));
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
