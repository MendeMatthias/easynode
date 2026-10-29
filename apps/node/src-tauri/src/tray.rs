//! Menu-bar tray: the always-there control surface of a background support
//! app. Closing the window hides it (the node keeps running); the tray shows
//! live status and carries Start/Stop + Quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::state::{AppState, NodePhase};

/// Clonable handles to the dynamic menu items so the status refresher can
/// update their text as the phase changes.
pub struct TrayHandles {
    pub status: MenuItem<Wry>,
    pub toggle: MenuItem<Wry>,
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_i = MenuItem::with_id(app, "open", "Open BTX Node", true, None::<&str>)?;
    let status_i = MenuItem::with_id(app, "status", "Node: stopped", false, None::<&str>)?;
    let toggle_i = MenuItem::with_id(app, "toggle", "Start node", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit BTX Node", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_i,
            &PredefinedMenuItem::separator(app)?,
            &status_i,
            &toggle_i,
            &PredefinedMenuItem::separator(app)?,
            &quit_i,
        ],
    )?;

    app.manage(TrayHandles {
        status: status_i,
        toggle: toggle_i,
    });

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled app icon");
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("BTX Node")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "toggle" => toggle_node(app),
            "quit" => {
                // The RunEvent::ExitRequested handler performs the graceful
                // node stop — one shutdown path for menu-quit and Cmd+Q.
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_node(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        // Decide by the PHASE — the same signal the menu label reflects — so
        // the action can never be the opposite of what the item says. (An
        // rpc.is_some() check disagreed with the label during Starting and in
        // the wedged-error state.)
        let phase = state.phase.lock().await.clone();
        match phase {
            NodePhase::Starting => {
                // A start is in flight; reflect_phase disables the item, but a
                // click can still race in — do nothing rather than fight it.
            }
            NodePhase::Ready { .. }
            | NodePhase::Syncing { .. }
            | NodePhase::LoadingSnapshot
            | NodePhase::Warming { .. } => {
                crate::commands::stop_node_inner(&state).await;
                *state.phase.lock().await = NodePhase::Stopped;
                reflect_phase(&app, &NodePhase::Stopped);
            }
            _ => match crate::commands::start_node_inner(&app, &state).await {
                Ok(()) => {}
                Err(message) => {
                    let p = NodePhase::Error {
                        message: message.clone(),
                    };
                    *state.phase.lock().await = p.clone();
                    reflect_phase(&app, &p);
                }
            },
        }
    });
}

/// Project the phase onto the tray's status + toggle items. Called from the
/// start/stop paths; cheap and best-effort. The status refresher calls
/// [`reflect_status`], which also knows whether the node is stuck.
pub fn reflect_phase(app: &AppHandle, phase: &NodePhase) {
    reflect_status(app, phase, false);
}

/// Whether the tray calls a running node stuck: the watchdog's verdict
/// (frozen 15 minutes, `btx_core::watchdog`), or a tip older than
/// `TIP_STALE_AFTER_SECS` with no newer block known. A node that knows of
/// newer blocks and has an old tip is usually closing a gap from an older
/// snapshot; only the window holds the trend that tells a closing gap from a
/// pinned one (catchup-trend.ts), so the tray leaves that case to it.
pub fn stuck_for_tray(stall_verdict: bool, tip_stale: bool, blocks_behind: u64) -> bool {
    stall_verdict || (tip_stale && blocks_behind < 2)
}

/// "245,498".
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The tray's status line and whether the node run is active. Pure, so it
/// is tested. `stuck` turns "running" into "not following", the word the
/// window's headline uses (headline.ts): on 2026-10-12 every node following
/// signatures stood at block 245,498 for hours while this line said
/// "running".
pub fn tray_status(phase: &NodePhase, stuck: bool) -> (String, bool) {
    match phase {
        NodePhase::Welcome => ("Node: not set up yet".to_string(), false),
        NodePhase::Downloading { progress } => {
            (format!("Setting up… {:.0}%", progress * 100.0), false)
        }
        NodePhase::Preparing => ("Setting up…".to_string(), false),
        NodePhase::Starting => ("Node: starting…".to_string(), true),
        NodePhase::Warming { .. } => ("Node: getting ready…".to_string(), true),
        NodePhase::LoadingSnapshot => ("Node: loading snapshot…".to_string(), true),
        NodePhase::Syncing { progress, .. } => {
            (format!("Node: syncing {:.0}%", progress * 100.0), true)
        }
        NodePhase::Ready { height, .. } if stuck => (
            format!("Node: not following · block {}", group(*height)),
            true,
        ),
        NodePhase::Ready { height, .. } => {
            (format!("Node: running · block {}", group(*height)), true)
        }
        NodePhase::Stopped => ("Node: stopped".to_string(), false),
        NodePhase::Error { .. } => ("Node: needs attention".to_string(), false),
    }
}

/// [`reflect_phase`], with whether the node is stuck ([`stuck_for_tray`]).
pub fn reflect_status(app: &AppHandle, phase: &NodePhase, stuck: bool) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let (status, running) = tray_status(phase, stuck);
    let _ = handles.status.set_text(status);
    let _ = handles
        .toggle
        .set_text(if running { "Stop node" } else { "Start node" });
    // No sensible toggle action mid-transition: disable during Starting and
    // the setup pipeline phases so the item can't misfire.
    let actionable = !matches!(
        phase,
        NodePhase::Starting
            | NodePhase::Downloading { .. }
            | NodePhase::Preparing
            | NodePhase::Welcome
    );
    let _ = handles.toggle.set_enabled(actionable);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_at_its_tip_reads_running_with_the_block_grouped() {
        let p = NodePhase::Ready {
            height: 245_760,
            peers: 9,
            blocks_behind: 0,
        };
        assert_eq!(
            tray_status(&p, false),
            ("Node: running · block 245,760".to_string(), true)
        );
    }

    // 2026-10-12: every node following signatures stood at 245,498 for
    // hours while this line said "running".
    #[test]
    fn a_stuck_node_reads_not_following() {
        let p = NodePhase::Ready {
            height: 245_498,
            peers: 9,
            blocks_behind: 0,
        };
        assert_eq!(
            tray_status(&p, true),
            ("Node: not following · block 245,498".to_string(), true)
        );
    }

    #[test]
    fn stuck_only_changes_a_running_node() {
        assert_eq!(
            tray_status(&NodePhase::Stopped, true),
            ("Node: stopped".to_string(), false)
        );
        let s = NodePhase::Syncing {
            height: 200_000,
            headers: 245_760,
            progress: 0.62,
            peers: 8,
        };
        assert_eq!(
            tray_status(&s, false),
            ("Node: syncing 62%".to_string(), true)
        );
    }

    #[test]
    fn the_watchdog_verdict_is_stuck() {
        assert!(stuck_for_tray(true, false, 0));
    }

    #[test]
    fn an_old_tip_with_nothing_newer_known_is_stuck() {
        assert!(stuck_for_tray(false, true, 0));
        assert!(stuck_for_tray(false, true, 1));
    }

    #[test]
    fn an_old_tip_while_catching_up_is_not_called_stuck_here() {
        // A node closing a gap from an older snapshot has an old tip by
        // construction; only the window has the trend that tells a closing
        // gap from a pinned one (catchup-trend.ts), so the tray stays quiet.
        assert!(!stuck_for_tray(false, true, 2));
        assert!(!stuck_for_tray(false, true, 460));
    }

    #[test]
    fn nothing_wrong_is_not_stuck() {
        assert!(!stuck_for_tray(false, false, 0));
    }

    #[test]
    fn grouping_puts_a_comma_every_three_digits() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(245_498), "245,498");
        assert_eq!(group(1_234_567), "1,234,567");
    }
}
