//! easyBTX Node — a standalone macOS app that runs a BTX full node in one
//! click, for people who want to support the network. Built on the shared
//! `btx-core` engine (the same code the easyBTX miner runs its node with).
//!
//! Lifecycle model: this is a BACKGROUND SUPPORT app. Closing the window hides
//! it to the menu-bar tray and the node keeps running; only Quit (tray menu or
//! Cmd+Q) stops the node — gracefully, so btxd's shielded-state flush completes
//! and the next start is instant instead of an 8-minute rebuild.

mod ask;
mod commands;
mod state;
mod tray;
mod wallet;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use crate::state::{node_datadir, AppState, NodeAppSettings, NodePhase};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_node_status,
            commands::begin_setup,
            commands::start_node,
            commands::stop_node,
            commands::open_data_folder,
            commands::set_keep_awake,
            commands::set_node_profile,
            commands::set_attestation_serve,
            commands::set_node_nickname,
            commands::set_service_report,
            commands::reclaim_disk_now,
            commands::node_footprint,
            commands::remove_node_data_now,
            commands::open_global_stats,
            commands::close_choice,
            commands::set_on_close,
            commands::esplora_preflight,
            commands::set_esplora,
            commands::set_esplora_listen,
            commands::set_witness,
            commands::set_witness_listen,
            ask::ask_chain_progress,
            ask::ask_supply,
            ask::ask_next_halving,
            ask::ask_fees,
            ask::ask_mining,
            ask::ask_block,
            ask::ask_transaction,
            ask::ask_tx_index_status,
            ask::set_explorer_mode,
            wallet::set_wallet_enabled,
            wallet::wallet_status,
            wallet::wallet_import,
            wallet::wallet_create,
            wallet::wallet_forget,
            wallet::wallet_receive_address,
            wallet::wallet_send,
            wallet::wallet_open_explorer,
        ])
        .setup(|app| {
            tray::build_tray(app.handle())?;

            // E2E/QA seam: EASYBTX_NODE_E2E_AUTOSETUP=1 runs the full setup
            // pipeline at launch without a wizard click. HARD-GATED on
            // EASYBTX_NODE_DATADIR also being set: an autosetup against the
            // real shared ~/.easybtx (e.g. a QA shell that exported the var
            // persistently) must be impossible — unattended downloads +
            // provisioning belong to throwaway datadirs only.
            if std::env::var("EASYBTX_NODE_E2E_AUTOSETUP").as_deref() == Ok("1")
                && std::env::var("EASYBTX_NODE_DATADIR").map(|v| !v.trim().is_empty()) == Ok(true)
                && !NodeAppSettings::load(&node_datadir()).setup_complete
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    if let Err(message) = commands::guarded_setup(&handle, &state).await {
                        eprintln!("[e2e] setup pipeline failed: {message}");
                    }
                });
            }

            // Returning user: the app's promise is "open it and your node
            // runs" — auto-start without a click. First run shows the wizard
            // (phase stays Welcome until the user begins setup).
            let datadir = node_datadir();
            if NodeAppSettings::load(&datadir).setup_complete {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    if let Err(message) = commands::start_node_inner(&handle, &state).await {
                        let p = NodePhase::Error {
                            message: message.clone(),
                        };
                        *state.phase.lock().await = p.clone();
                        tray::reflect_phase(&handle, &p);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Red X: behavior is the user's choice, remembered in settings.
            //   "tray"  → hide; the node keeps supporting the network (old default)
            //   "quit"  → graceful stop + exit
            //   "ask"   → prevent close and let the webview show the close dialog
            // An unknown/hand-edited value falls back to "ask" so nothing wedges.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle();
                match NodeAppSettings::load(&node_datadir()).on_close.as_str() {
                    "tray" => {
                        let _ = window.hide();
                    }
                    "quit" => commands::spawn_graceful_quit(app),
                    _ => {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = app.emit("close-requested", ());
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building easyBTX Node")
        .run(|app, event| {
            match event {
                // Quit (tray menu or Cmd+Q). The graceful node stop must run OFF
                // the main thread or the UI freezes for up to the ~90s shielded
                // flush — the exact hang that drove users to force-quit. So on
                // the first request we hold the exit, run the async quit flow
                // (which shows "stopping…" then exits), and let the *second*
                // ExitRequested — the one our flow triggers — proceed.
                RunEvent::ExitRequested { api, .. } => {
                    let state = app.state::<AppState>();
                    if !state.quitting.load(std::sync::atomic::Ordering::SeqCst) {
                        api.prevent_exit();
                        commands::spawn_graceful_quit(app);
                    }
                }
                // macOS dock-icon click while the window is hidden.
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => {
                    tray::show_main_window(app);
                }
                _ => {}
            }
        });
}
