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
/// status refresher and the start/stop paths; cheap and best-effort.
pub fn reflect_phase(app: &AppHandle, phase: &NodePhase) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let (status, running) = match phase {
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
        NodePhase::Ready { height, .. } => (format!("Node: running · block {height}"), true),
        NodePhase::Stopped => ("Node: stopped".to_string(), false),
        NodePhase::Error { .. } => ("Node: needs attention".to_string(), false),
    };
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
