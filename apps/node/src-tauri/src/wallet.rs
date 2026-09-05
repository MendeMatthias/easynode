//! Optional wallet — balance, history, receive and send, all answered by the
//! user's OWN node, never a public explorer (the official btx.dev/wallet reads
//! explorers; "your node answers" is the whole point). OFF from factory
//! settings: the Settings toggle is the only way in.
//!
//! v2 adds spending. That is not new capability, only newly reachable: the
//! bundle `restorewalletbundle` installs carries the PQ master seed, so btxd
//! has held full spending keys since v1 and the watch-only framing was a UI
//! choice, not a cryptographic one.
//!
//! The one outward link is `wallet_open_explorer`, and it is deliberately not
//! an "open this URL" command: the webview passes a *kind* and an *id*, both
//! re-validated here, and the host builds the URL against a hardcoded prefix.
//! Same rule as `open_global_stats` — the webview never chooses a destination.

use serde::Serialize;
use tauri::State;

use btx_core::error::AppError;
use btx_core::node_api as api;
use btx_core::rpc::{Rpc, RpcClient};

use crate::ask::{degrade, Ask};
use crate::state::{node_datadir, AppState, NodeAppSettings};

/// The one wallet this app manages. A fixed name keeps the flow one-button
/// simple; the underlying btxd wallet dir carries it.
pub const WALLET_NAME: &str = "btxnode";

/// How many history rows the panel asks for. v1 showed 8, which was a preview,
/// not a history — a wallet that has been mining for a week fills that in an hour.
const HISTORY_COUNT: u32 = 50;

#[derive(Debug, Clone, Serialize)]
pub struct WalletTx {
    /// Needed for the explorer deep-link. v1 dropped it, so no row was clickable.
    pub txid: String,
    pub category: String,
    pub amount: f64,
    pub confirmations: i64,
    pub time: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalletView {
    pub enabled: bool,
    pub imported: bool,
    /// The wallet's receive address (display; recorded at create/import).
    pub address: Option<String>,
    pub trusted: f64,
    pub pending: f64,
    pub immature: f64,
    /// True while the node's background chainstate is still backfilling
    /// history — balances/history may still be filling in.
    pub backfilling: bool,
    /// True when the node has not accepted a block in hours, so every figure in
    /// this view is as of whenever it stopped.
    ///
    /// Separate from `backfilling` on purpose, and the distinction is the whole
    /// point: `backfilling` means the node is still filling in the PAST, which
    /// is normal and temporary. This means it stopped following the PRESENT,
    /// which is what a withdrawn consensus rule does and which no existing flag
    /// could see. `verification_progress` is computed against the tip the node
    /// itself believes in, so a stalled node reports near 1.0 and looks healthy.
    pub tip_stale: bool,
    /// How far the node still has to climb: `headers - blocks`.
    ///
    /// This is not cosmetic. An operator who ran a real catch-up measured 58
    /// blocks per hour against a per-block validation cost that alone would
    /// allow roughly 775, so catch-up is limited by how fast peers serve BODIES,
    /// not by the machine. And a LOADED wallet updates on every connected block,
    /// which on their node slowed `ConnectBlock` enough that it fell behind the
    /// tip and began self-forking, while the wallet RPCs hung on `cs_main` so
    /// the balance they had opened it for was unreadable anyway.
    ///
    /// So a wallet held open through a long catch-up costs the user the thing
    /// they actually want. The panel uses this to say so and to point at the
    /// existing close control, rather than unloading anything behind their back.
    pub blocks_behind: u64,
    pub txs: Vec<WalletTx>,
}

fn empty_view(enabled: bool) -> WalletView {
    WalletView {
        enabled,
        imported: false,
        address: None,
        trusted: 0.0,
        pending: 0.0,
        immature: 0.0,
        backfilling: false,
        tip_stale: false,
        blocks_behind: 0,
        txs: Vec::new(),
    }
}

/// Map `listtransactions` JSON into display rows (most recent first).
fn map_txs(v: &serde_json::Value) -> Vec<WalletTx> {
    let mut txs: Vec<WalletTx> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| WalletTx {
                    txid: t["txid"].as_str().unwrap_or("").to_string(),
                    category: t["category"].as_str().unwrap_or("").to_string(),
                    amount: t["amount"].as_f64().unwrap_or(0.0),
                    confirmations: t["confirmations"].as_i64().unwrap_or(0),
                    time: t["time"].as_u64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    txs.reverse(); // listtransactions is oldest-first within the window
    txs
}

/// A wallet-scoped RPC handle that survives a node restart, plus the balances
/// the probe already paid for.
///
/// btxd may have the wallet on disk but not loaded (-18 not found / -35 not
/// loaded) after a restart, or after an import made before `load_on_startup`.
/// Probing with `getbalances`, loading once, and retrying is the only way to be
/// sure. EVERY wallet-scoped command must come through here — a command that
/// calls `rpc.for_wallet()` directly works until the first node restart and then
/// fails in the user's hands.
async fn ensure_loaded(
    rpc: &RpcClient,
    name: &str,
) -> Result<(RpcClient, api::Balances), AppError> {
    let wallet_rpc = rpc.for_wallet(name);
    match api::get_balances(&wallet_rpc).await {
        Ok(b) => Ok((wallet_rpc, b)),
        Err(AppError::Rpc { code, .. }) if code == -18 || code == -35 => {
            let _ = api::load_wallet(rpc, name).await;
            let b = api::get_balances(&wallet_rpc).await?;
            Ok((wallet_rpc, b))
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn set_wallet_enabled(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    let _ = &state; // settings-only toggle; no node interaction needed
    NodeAppSettings::update(&node_datadir(), |s| s.wallet_enabled = on);
    Ok(())
}

/// The wallet panel's single data call: settings + (when imported and the
/// node runs) node-verified balances and recent history.
#[tauri::command]
pub async fn wallet_status(state: State<'_, AppState>) -> Result<Ask<WalletView>, String> {
    let settings = NodeAppSettings::load(&node_datadir());
    let Some(name) = settings.wallet_name.clone() else {
        // Nothing imported yet — no node needed to say so.
        return Ok(Ask::Ready(empty_view(settings.wallet_enabled)));
    };
    let Some(rpc) = state.rpc.lock().await.clone() else {
        return Ok(Ask::Stopped);
    };
    let (wallet_rpc, balances) = match ensure_loaded(&rpc, &name).await {
        Ok(pair) => pair,
        Err(e) => return Ok(degrade(e)),
    };
    let txs = api::list_transactions(&wallet_rpc, HISTORY_COUNT)
        .await
        .map(|v| map_txs(&v))
        .unwrap_or_default();
    let backfilling = api::get_chainstates(&rpc)
        .await
        .map(|cs| cs.snapshot().map(|c| !c.validated).unwrap_or(false))
        .unwrap_or(false);
    // A node frozen on a withdrawn consensus rule still answers every RPC
    // confidently with figures from whenever it stopped. Tip age is the only
    // signal that separates that from a healthy node, so read it here rather
    // than letting the panel imply the balance is current.
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // One call answers both questions: is the figure old, and is the node still
    // climbing toward the tip. They are different states and the panel says
    // different things about each.
    let chain = api::get_blockchain_info(&rpc).await.ok();
    let tip_stale = chain
        .as_ref()
        .map(|bi| api::tip_is_stale(bi.median_time, now_unix))
        .unwrap_or(false);
    let blocks_behind = chain
        .as_ref()
        .map(|bi| bi.headers.saturating_sub(bi.blocks))
        .unwrap_or(0);
    Ok(Ask::Ready(WalletView {
        enabled: settings.wallet_enabled,
        imported: true,
        address: settings.wallet_address.clone(),
        trusted: balances.trusted,
        pending: balances.untrusted_pending,
        immature: balances.immature,
        backfilling,
        tip_stale,
        blocks_behind,
        txs,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub rescanned: bool,
    pub warning: Option<String>,
}

/// Largest file we will stage. A `.btxwallet` bundle is kilobytes, but a real
/// `wallet.dat` from a node that has been receiving for months is megabytes, and
/// the old 1 MB ceiling was sized for the bundle alone. 64 MB is far above any
/// legitimate wallet and still bounds what a webview can make us write to disk.
const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

/// Does btxd already have a wallet dir by this name? Read-only: `listwalletdir`
/// answers from disk and never creates or loads anything, which is exactly what
/// a pre-flight check must not do. An RPC failure answers `false`, and every
/// caller treats `false` as "refuse", so the unknown case fails closed.
async fn wallet_dir_has(rpc: &dyn Rpc, name: &str) -> bool {
    api::list_wallet_dir(rpc)
        .await
        .map(|names| names.iter().any(|n| n == name))
        .unwrap_or(false)
}

/// What we tell a user when the default wallet name is taken and we could not
/// find a free one to import beside it. Never a bare failure: it says what did
/// not happen, so nobody reads a refusal as "my coins are gone".
const COLLISION_ADVICE: &str = "This node already holds wallets under every name easyNode uses, \
     so your file was NOT imported and nothing was changed. The balance on \
     screen belongs to the wallet already loaded here, not to the file you \
     just chose.";

/// Did the import actually land, despite the call reporting an error?
///
/// `restorewallet` rescans synchronously and the RPC client gives up after 60
/// seconds. A rescan of ~199k blocks does not finish in 60 seconds, so the
/// NORMAL path for a real wallet.dat is: btxd creates the wallet, loads it and
/// keeps scanning, while our call times out and we tell the user it failed.
/// That is the worst lie available here, because their wallet DID import and
/// they have just been told it did not, on the screen where they are moving
/// their money.
///
/// So a transport failure is never final on its own. We ask btxd what is
/// actually on disk, and if the wallet is there we treat the import as having
/// happened and say the scan is still running.
async fn import_landed_anyway(rpc: &dyn Rpc, name: &str, err: &AppError) -> bool {
    // Only for transport failures. An RPC-level refusal is btxd telling us
    // something real and must not be second-guessed.
    if !matches!(err, AppError::Http(_)) {
        return false;
    }
    wallet_dir_has(rpc, name).await
}

/// First wallet name not already taken on disk, or `None` if we cannot find one.
///
/// Bounded on purpose. If someone genuinely has sixteen imported wallets we stop
/// and say so rather than looping, and returning `None` makes the caller refuse
/// rather than overwrite.
///
/// Note the failure direction, because it is the opposite of the dumpwallet
/// pre-check that shares this helper. `wallet_dir_has` answers `false` on an RPC
/// error, so here an unreachable node makes the FIRST candidate look free and we
/// return it optimistically. That is safe, but only because `restorewallet` is
/// the real gate: if the name is genuinely taken btxd refuses and the error
/// reaches the user. It is never turned into a success. Do not "harden" this
/// into returning `None` on error without checking the dump path, which relies
/// on the same `false` meaning "refuse".
async fn next_free_wallet_name(rpc: &dyn Rpc) -> Option<String> {
    for n in 2..=16 {
        let candidate = format!("{WALLET_NAME}-{n}");
        if !wallet_dir_has(rpc, &candidate).await {
            return Some(candidate);
        }
    }
    None
}

/// Is this btxd error "that wallet already exists", or is it a DIFFERENT error
/// that merely contains the substring "exist"?
///
/// This matters more than it looks. `restorewallet` answers "Backup file does
/// not exist" when the path it was handed is gone — which is exactly what a
/// second import racing the first one, or a staging failure, produces. The old
/// bare `contains("exist")` read that hard failure as "already imported", loaded
/// whatever wallet happened to be there and reported SUCCESS. A user would be
/// shown someone else's balance (usually zero) and told their import worked.
fn is_already_exists(message: &str) -> bool {
    let m = message.to_lowercase();
    if m.contains("not exist") || m.contains("doesn't exist") || m.contains("does not exist") {
        return false;
    }
    m.contains("exist")
}

/// Create `dir` (0700 on unix) and return a file handle at `path` opened 0600.
///
/// `std::fs::write` creates 0644 under the usual 0022 umask, so the staged file
/// would be world-readable for the whole import. That file is a wallet.dat, a
/// PQ master seed bundle, or a dumpwallet text of plaintext private keys. On a
/// shared Mac that is a window in which any other local account can copy
/// spending keys. Nothing else in this app sweeps the datadir for these names,
/// so a crash mid-import would leave the window open forever.
fn stage_private(
    dir: &std::path::Path,
    path: &std::path::Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Import a wallet the user already has, whatever shape it arrives in.
///
/// The maintainer's line when BTX.dev's hosted wallet was retired was "your
/// wallet file works everywhere BTX does". To someone who ran a node that
/// sentence means `wallet.dat`. This used to accept ONLY the browser bundle
/// JSON, so a person holding the file the sentence was about got "that doesn't
/// look like a .btxwallet file" and stopped. Now the bytes decide the route:
///
///   browser bundle JSON  -> `restorewalletbundle`, the PQ seed path
///   wallet.dat           -> `restorewallet`, sqlite or berkeley, btxd decides
///   dumpwallet text      -> `importwallet` into a wallet that already exists
///   anything else        -> advice naming every format we DO take
///
/// Arrives base64 because a `wallet.dat` is binary and the previous signature
/// took a `String`, which silently mangled every non-UTF-8 byte before Rust ever
/// saw it. That alone made a real wallet file impossible to import.
#[tauri::command]
pub async fn wallet_import(
    state: State<'_, AppState>,
    content_b64: String,
) -> Result<Ask<ImportResult>, String> {
    use base64::Engine as _;
    use btx_core::wallet_format::{detect, unknown_file_advice, WalletFileKind};

    // Base64 inflates by 4/3, so bound the encoded form before decoding rather
    // than after, or a hostile webview could make us allocate first and refuse
    // second.
    if content_b64.len() > MAX_IMPORT_BYTES / 3 * 4 + 4 {
        return Ok(Ask::Unavailable {
            message: "That file is too large to be a wallet.".into(),
        });
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(content_b64.as_bytes()) else {
        return Ok(Ask::Unavailable {
            message: "The file could not be read.".into(),
        });
    };
    if bytes.is_empty() {
        return Ok(Ask::Unavailable {
            message: "That file is empty.".into(),
        });
    }

    let kind = detect(&bytes);
    if kind == WalletFileKind::Unknown {
        return Ok(Ask::Unavailable {
            message: unknown_file_advice().into(),
        });
    }

    let Some(rpc) = state.rpc.lock().await.clone() else {
        return Ok(Ask::Stopped);
    };

    // Only one import may be in flight. The Import button disables itself, but
    // that is a webview guard: a reload while an import is mid-rescan re-enables
    // it, and two imports sharing one staged path make the first one's cleanup
    // delete the file the second one's btxd is about to open.
    static IMPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _import_guard = IMPORT_LOCK.lock().await;

    // A dumpwallet file can only add keys to a wallet that ALREADY exists, and
    // creating one here is a one-way door: once `btxnode` has a wallet dir, both
    // `restorewallet` and `restorewalletbundle` fail forever with "already
    // exists". The previous version called `load_or_create_wallet` here, so a
    // dump import that btxd then refused (it refuses `importwallet` on the
    // descriptor wallet `createwallet` makes) left an empty wallet behind that
    // permanently blocked the user's real wallet.dat and .btxwallet. Decide this
    // BEFORE any key material reaches the disk.
    if kind == WalletFileKind::WalletDump && !wallet_dir_has(&rpc, WALLET_NAME).await {
        return Ok(Ask::Unavailable {
            message: "A dumpwallet text file can only add keys to a wallet that already \
                      exists, and this node doesn't have one yet. Import your wallet.dat \
                      or your .btxwallet file instead — either of those brings the whole \
                      wallet across on its own."
                .into(),
        });
    }

    let datadir = node_datadir();
    // Key material is staged in its OWN directory, 0700, with the file 0600, and
    // the directory is cleared before and after. Name the staged file after what
    // it actually is, so anything a killed process leaves behind is identifiable.
    let stage = datadir.join("wallet-import");
    let _ = std::fs::remove_dir_all(&stage); // sweep a crashed import's leftovers
    let tmp = stage.join(match kind {
        WalletFileKind::BrowserBundle => "wallet-import.btxwallet.json",
        WalletFileKind::WalletDump => "wallet-import.dump.txt",
        _ => "wallet-import.wallet.dat",
    });
    if let Err(e) = stage_private(&stage, &tmp, &bytes) {
        let _ = std::fs::remove_dir_all(&stage);
        return Ok(Ask::Unavailable {
            message: format!("couldn't stage the wallet file: {e}"),
        });
    }
    let path = tmp.display().to_string();

    // The bundle carries the address it expects to own. That is the only marker
    // we get, and it is what makes a safe re-import possible below.
    let bundle_addr: Option<String> = if kind == WalletFileKind::BrowserBundle {
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|b| b["first_receive_address"].as_str().map(String::from))
    } else {
        None
    };

    // The name this import is ATTEMPTED under. Normally WALLET_NAME; if that is
    // taken by a DIFFERENT wallet we import beside it rather than pretending,
    // because the alternative is telling someone their import worked while
    // showing them a stranger's balance.
    //
    // "Attempted" is the load-bearing word, and it is not the same as "succeeded".
    // Any path that retargets the import MUST set this before it calls btxd: the
    // failure handling below asks btxd what landed under this name, and on the
    // path where that question matters most the call has already failed.
    let mut target_name = WALLET_NAME.to_string();

    let (result, rescanned) = match kind {
        WalletFileKind::BrowserBundle => {
            let first = api::restore_wallet_bundle(&rpc, WALLET_NAME, &path, true).await;
            match first {
                Ok(v) => (Ok(v), true),
                Err(AppError::Rpc { ref message, .. }) if is_already_exists(message) => {
                    // The name is taken. Before claiming success, find out whether
                    // it is taken by THIS wallet (the user clicked Import twice,
                    // which must keep working) or by a different one (in which
                    // case reporting success would show them the wrong balance).
                    let _ = api::load_wallet(&rpc, WALLET_NAME).await;
                    let same_wallet = match &bundle_addr {
                        Some(a) => {
                            let w = rpc.for_wallet(WALLET_NAME);
                            // Unknown answers false, so doubt never becomes a
                            // claim that the wallet is theirs.
                            api::address_is_mine(&w, a).await.unwrap_or(false)
                        }
                        None => false,
                    };
                    if same_wallet {
                        (Ok(serde_json::json!({"name": WALLET_NAME})), false)
                    } else {
                        match next_free_wallet_name(&rpc).await {
                            Some(alt) => {
                                // Adopt the name BEFORE the call, not on success.
                                // See the wallet.dat branch below for why: on the
                                // failing return this name is what decides which
                                // wallet we ask btxd about, and asking about the
                                // wrong one answers true by definition.
                                target_name = alt.clone();
                                let r = api::restore_wallet_bundle(&rpc, &alt, &path, true).await;
                                (r, true)
                            }
                            None => (
                                Err(AppError::Rpc {
                                    code: 0,
                                    message: COLLISION_ADVICE.into(),
                                }),
                                false,
                            ),
                        }
                    }
                }
                Err(AppError::Rpc { ref message, .. })
                    if message.to_lowercase().contains("block")
                        || message.to_lowercase().contains("rescan") =>
                {
                    // Backfill hasn't reached the bundle birthday — import without
                    // the scan; balances appear as the node backfills history.
                    (
                        api::restore_wallet_bundle(&rpc, WALLET_NAME, &path, false).await,
                        false,
                    )
                }
                Err(e) => (Err(e), false),
            }
        }
        WalletFileKind::WalletDatSqlite | WalletFileKind::WalletDatBerkeley => {
            // btxd always rescans on restore, so on a node still backfilling
            // this can fail on the SCAN while the wallet itself is fine. Treat
            // an existing wallet as "already imported" rather than an error, the
            // same way the bundle path does.
            match api::restore_wallet(&rpc, WALLET_NAME, &path).await {
                Ok(v) => (Ok(v), true),
                Err(AppError::Rpc { ref message, .. }) if is_already_exists(message) => {
                    // A wallet.dat carries no marker we can compare, so we cannot
                    // tell "the same wallet again" from "a different wallet".
                    // Import beside it rather than guessing. Silently loading the
                    // existing one and calling it success is how a person ends up
                    // staring at 0.00 and concluding their coins are gone.
                    match next_free_wallet_name(&rpc).await {
                        Some(alt) => {
                            // Adopt the name BEFORE the call. `if r.is_ok()` looks
                            // careful and is the bug: a wallet.dat rescan is
                            // multi-hour and the transport gives up at 60 s, so
                            // the FAILING return is the normal outcome here. With
                            // the name dropped on that path, `import_landed_anyway`
                            // below asked btxd about WALLET_NAME — which exists by
                            // definition, since that is what made this a collision
                            // — got true, and reported success pointing the panel
                            // at the pre-existing wallet while the user's keys were
                            // in this one. That is the "staring at 0.00 and
                            // concluding their coins are gone" failure the whole
                            // import-beside path exists to prevent, reintroduced by
                            // the guard that runs after it.
                            target_name = alt.clone();
                            let r = api::restore_wallet(&rpc, &alt, &path).await;
                            (r, true)
                        }
                        None => (
                            Err(AppError::Rpc {
                                code: 0,
                                message: COLLISION_ADVICE.into(),
                            }),
                            false,
                        ),
                    }
                }
                Err(e) => (Err(e), false),
            }
        }
        WalletFileKind::WalletDump => {
            // The wallet is known to exist (checked before staging), so load it
            // and let btxd's own refusal — it rejects `importwallet` on a
            // descriptor wallet — reach the user rather than inventing one.
            let _ = api::load_wallet(&rpc, WALLET_NAME).await;
            let wallet_rpc = rpc.for_wallet(WALLET_NAME);
            (
                api::import_wallet_dump(&wallet_rpc, &path)
                    .await
                    .map(|_| serde_json::json!({"name": WALLET_NAME})),
                true,
            )
        }
        WalletFileKind::Unknown => unreachable!("returned above"),
    };

    // A 60 second transport timeout during a multi-hour rescan is the NORMAL
    // outcome for a real wallet.dat, not an edge case. Ask btxd what actually
    // landed before telling anyone their import failed. `rescanned` stays false
    // so the panel says the history is still filling in, which is true.
    let (result, rescanned) = match result {
        Err(ref e) if import_landed_anyway(&rpc, &target_name, e).await => {
            (Ok(serde_json::json!({ "name": target_name })), false)
        }
        other => (other, rescanned),
    };

    // Never leave key material staged. The whole 0700 directory goes, so a file
    // btxd may have renamed or a second name from an earlier kind goes with it.
    let _ = std::fs::remove_dir_all(&stage);

    match result {
        Ok(v) => {
            // Only the browser bundle carries a verified first receive address.
            // A restored wallet.dat has its own addresses already and the panel
            // asks the node for one, so leaving this None is correct there.
            let addr = if kind == WalletFileKind::BrowserBundle {
                std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                    .and_then(|b| b["first_receive_address"].as_str().map(String::from))
            } else {
                None
            };
            // `target_name`, not WALLET_NAME. When the default name was taken by
            // a different wallet the import landed beside it, and pointing the
            // panel at the wrong one is exactly the bug this avoids.
            NodeAppSettings::update(&datadir, |s| {
                s.wallet_name = Some(target_name.clone());
                s.wallet_address = addr;
            });
            let warning = v["warnings"]
                .as_array()
                .and_then(|w| w.first())
                .and_then(|x| x.as_str())
                .map(String::from);
            Ok(Ask::Ready(ImportResult { rescanned, warning }))
        }
        Err(e) => Ok(degrade(e)),
    }
}

/// Stop watching: unload the wallet (and drop it from btxd's startup list)
/// and clear the setting. The wallet files stay in the node's data folder —
/// keys are never deleted.
#[tauri::command]
pub async fn wallet_forget(state: State<'_, AppState>) -> Result<(), String> {
    let datadir = node_datadir();
    let name = NodeAppSettings::load(&datadir).wallet_name;
    if let (Some(rpc), Some(name)) = (state.rpc.lock().await.clone(), name) {
        let _ = rpc
            .call("unloadwallet", serde_json::json!([name, false]))
            .await;
    }
    NodeAppSettings::update(&datadir, |s| {
        s.wallet_name = None;
        s.wallet_address = None;
    });
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateResult {
    pub address: String,
    /// Where the .btxwallet file was written (the user's Desktop).
    pub file_path: String,
}

/// Create a NEW wallet inside the user's own node — the same self-custody
/// `.btxwallet` file the official BTX browser wallet produces, but generated
/// by btxd on this Mac with no website involved: `createwallet` (native PQ
/// master seed) → first receive address → `exportwalletbundle` to the
/// Desktop. Same format both ways, so btx.dev and the CLI can open it too.
#[tauri::command]
pub async fn wallet_create(state: State<'_, AppState>) -> Result<Ask<CreateResult>, String> {
    let datadir = node_datadir();
    if NodeAppSettings::load(&datadir).wallet_name.is_some() {
        return Ok(Ask::Unavailable {
            message: "A wallet is already set up — stop watching it first.".into(),
        });
    }
    let Some(rpc) = state.rpc.lock().await.clone() else {
        return Ok(Ask::Stopped);
    };

    // Create (or, if a previous wallet dir survives, load) the app's wallet.
    if let Err(e) = api::create_wallet(&rpc, WALLET_NAME).await {
        let msg = e.to_string().to_lowercase();
        if msg.contains("exist") || msg.contains("already") {
            let _ = api::load_wallet(&rpc, WALLET_NAME).await;
        } else {
            return Ok(degrade(e));
        }
    }
    let wallet_rpc = rpc.for_wallet(WALLET_NAME);
    let address = match api::get_new_address(&wallet_rpc).await {
        Ok(a) => a,
        Err(e) => return Ok(degrade(e)),
    };

    // Export the browser-compatible bundle to the Desktop — the one place the
    // user actually finds files. The filename carries the address prefix so
    // repeated wallets never overwrite each other.
    let Some(home) = btx_core::platform::home_dir() else {
        return Ok(Ask::Unavailable {
            message: "couldn't resolve your home folder".into(),
        });
    };
    let short = address.chars().take(12).collect::<String>();
    let file = home
        .join("Desktop")
        .join(format!("btx-wallet-{short}.btxwallet.json"));
    if let Err(e) = api::export_wallet_bundle(&wallet_rpc, &file.display().to_string()).await {
        return Ok(degrade(e));
    }

    NodeAppSettings::update(&datadir, |s| {
        s.wallet_name = Some(WALLET_NAME.into());
        s.wallet_address = Some(address.clone());
    });

    // Reveal the file in the OS file manager so "save it somewhere safe"
    // starts right now (Finder/Explorer select the file; Linux opens the dir).
    let _ = btx_core::platform::reveal_path(&file);

    Ok(Ask::Ready(CreateResult {
        address,
        file_path: file.display().to_string(),
    }))
}

/// A fresh receive address from the wallet's own descriptor range.
///
/// Handing out a new address per payment is the whole reason the bundle carries
/// an HD descriptor (`.../0/*`) rather than one key. Reusing a single address
/// links every payment you ever receive to the same public identity.
#[tauri::command]
pub async fn wallet_receive_address(state: State<'_, AppState>) -> Result<Ask<String>, String> {
    let settings = NodeAppSettings::load(&node_datadir());
    let Some(name) = settings.wallet_name.clone() else {
        return Err("No wallet yet — create or import one first.".into());
    };
    let Some(rpc) = state.rpc.lock().await.clone() else {
        return Ok(Ask::Stopped);
    };
    let (wallet_rpc, _) = match ensure_loaded(&rpc, &name).await {
        Ok(pair) => pair,
        Err(e) => return Ok(degrade(e)),
    };
    match api::get_new_address(&wallet_rpc).await {
        Ok(addr) => Ok(Ask::Ready(addr)),
        Err(e) => Ok(degrade(e)),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SendResult {
    pub txid: String,
    /// What btxd actually paid in fees, positive BTX. `None` if `gettransaction`
    /// didn't answer — the send still happened, so never fail the call over this.
    pub fee: Option<f64>,
}

/// Spend from the wallet.
///
/// Order matters and each guard exists because its absence loses money:
/// 1. `amount` finite and > 0 — NaN/inf reach btxd as garbage.
/// 2. the destination is valid *according to the node*, never a UI regex — a
///    typo that still parses would send coins into an unspendable void.
/// 3. `amount` fits the spendable balance, checked against the same probe that
///    loaded the wallet, so a stale UI number can't authorise an overspend.
///
/// `subtract_fee` is what makes "send Max" work: without it btxd needs
/// `amount + fee` and rejects a whole-balance send as insufficient funds.
#[tauri::command]
pub async fn wallet_send(
    state: State<'_, AppState>,
    address: String,
    amount: f64,
    subtract_fee: bool,
) -> Result<Ask<SendResult>, String> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err("Enter an amount greater than zero.".into());
    }
    let address = address.trim().to_string();
    if address.is_empty() {
        return Err("Enter the address you're sending to.".into());
    }

    let settings = NodeAppSettings::load(&node_datadir());
    let Some(name) = settings.wallet_name.clone() else {
        return Err("No wallet yet — create or import one first.".into());
    };
    let Some(rpc) = state.rpc.lock().await.clone() else {
        return Ok(Ask::Stopped);
    };
    let (wallet_rpc, balances) = match ensure_loaded(&rpc, &name).await {
        Ok(pair) => pair,
        Err(e) => return Ok(degrade(e)),
    };

    match api::address_is_valid(&rpc, &address).await {
        Ok(true) => {}
        Ok(false) => {
            return Err("That doesn't look like a BTX address. Check it and try again.".into())
        }
        Err(e) => return Ok(degrade(e)),
    }

    // The float epsilon mirrors the miner's send guard: an exact `>` on f64 sums
    // rejects a legitimate "send everything" by a rounding crumb.
    if amount > balances.trusted + 1e-12 {
        return Err(format!(
            "You can spend {:.8} BTX right now — that's less than {:.8}.",
            balances.trusted, amount
        ));
    }

    let txid = match api::send_to_address(&wallet_rpc, &address, amount, subtract_fee).await {
        Ok(t) => t,
        Err(e) => return Ok(degrade(e)),
    };
    // Best-effort: btxd reports `fee` negative (money leaving). Show it positive.
    let fee = api::get_transaction(&wallet_rpc, &txid)
        .await
        .ok()
        .and_then(|v| v["fee"].as_f64())
        .map(f64::abs);
    Ok(Ask::Ready(SendResult { txid, fee }))
}

/// Open a transaction or address on the public block explorer.
///
/// NOT an "open this URL" command. The webview may pass only a kind and an id,
/// both re-validated here, and the host composes the URL against a hardcoded
/// prefix — so a compromised webview cannot turn this into an arbitrary-URL
/// (or `file://`) opener. Same rule as `open_global_stats`.
///
/// This is the only wallet feature that touches the network beyond your node,
/// it is never automatic, and it sends nothing but the id you clicked.
#[tauri::command]
pub async fn wallet_open_explorer(kind: String, id: String) -> Result<(), String> {
    let url = explorer_url(&kind, &id).ok_or("Nothing to look up.")?;
    btx_core::platform::open_url(&url).map_err(|e| format!("couldn't open the explorer: {e}"))
}

/// The whole trust boundary, kept pure so it can be tested without a webview.
/// Returns `None` for anything that isn't demonstrably a txid or a BTX address —
/// which is what stops this from becoming an arbitrary-URL opener.
fn explorer_url(kind: &str, id: &str) -> Option<String> {
    const EXPLORER: &str = "https://btxscan.io";

    let path = match kind {
        // A txid is exactly 64 hex chars. Anything else isn't one.
        "tx" if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) => "tx",
        // Mainnet bech32m: `btx1` + the lowercase bech32 charset, length-capped.
        // Rejecting uppercase and every separator (`/`, `.`, `?`, `#`, `:`) is
        // what keeps the id inside its path segment.
        "address"
            if id.starts_with("btx1")
                && (14..=128).contains(&id.len())
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) =>
        {
            "address"
        }
        _ => return None,
    };
    Some(format!("{EXPLORER}/{path}/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Minimal `Rpc` that answers `listwalletdir` from a fixed list, and can be
    /// told to fail instead, so the collision logic is testable with no node.
    struct DirRpc {
        names: Vec<String>,
        fail: bool,
        calls: Mutex<Vec<String>>,
    }
    impl DirRpc {
        fn with(names: &[&str]) -> Self {
            Self {
                names: names.iter().map(|s| s.to_string()).collect(),
                fail: false,
                calls: Mutex::new(Vec::new()),
            }
        }
        fn failing() -> Self {
            Self {
                names: Vec::new(),
                fail: true,
                calls: Mutex::new(Vec::new()),
            }
        }
    }
    #[async_trait]
    impl Rpc for DirRpc {
        async fn call(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, AppError> {
            self.calls.lock().unwrap().push(method.to_string());
            if self.fail {
                return Err(AppError::Rpc {
                    code: -1,
                    message: "node down".into(),
                });
            }
            Ok(
                serde_json::json!({ "wallets": self.names.iter().map(|n| serde_json::json!({"name": n})).collect::<Vec<_>>() }),
            )
        }
    }

    #[tokio::test]
    async fn a_second_wallet_lands_beside_the_first_rather_than_on_top_of_it() {
        // The whole point: the default name is taken, so we must import NEXT to
        // it. Silently loading the existing one and calling that success is how
        // a user is shown 0.00 and concludes their coins are gone.
        let rpc = DirRpc::with(&["btxnode"]);
        assert_eq!(
            next_free_wallet_name(&rpc).await.as_deref(),
            Some("btxnode-2")
        );
    }

    #[tokio::test]
    async fn already_used_alternates_are_skipped_not_reused() {
        let rpc = DirRpc::with(&["btxnode", "btxnode-2", "btxnode-3"]);
        assert_eq!(
            next_free_wallet_name(&rpc).await.as_deref(),
            Some("btxnode-4")
        );
    }

    #[tokio::test]
    async fn we_refuse_rather_than_loop_when_every_name_is_taken() {
        let mut all = vec!["btxnode".to_string()];
        all.extend((2..=16).map(|n| format!("btxnode-{n}")));
        let refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        let rpc = DirRpc::with(&refs);
        assert_eq!(
            next_free_wallet_name(&rpc).await,
            None,
            "refuse, never overwrite"
        );
    }

    #[tokio::test]
    async fn wallet_dir_has_answers_false_when_the_node_is_unreachable() {
        // Pinned deliberately. The dumpwallet pre-check reads this `false` as
        // "refuse", while next_free_wallet_name reads it as "free". Both are
        // safe today only because restorewallet is the real gate. If this
        // default ever changes, BOTH callers have to be revisited.
        let rpc = DirRpc::failing();
        assert!(!wallet_dir_has(&rpc, "btxnode").await);
    }

    #[tokio::test]
    async fn the_landed_anyway_guard_answers_about_the_name_it_is_given() {
        // This guard turns a transport failure into reported success, so the
        // name it is handed decides which wallet the panel is pointed at.
        //
        // The import-beside path used to hand it a STALE name. `btxnode` is
        // taken (that is what made the import a collision), the retry goes to
        // `btxnode-2`, the rescan outruns the 60 s transport limit — the normal
        // outcome — and the old code kept `target_name` at `btxnode` because the
        // result was Err. Asking "did btxnode land?" then answers true by
        // definition and the user is shown the wallet they already had.
        let timeout = AppError::Http("operation timed out".into());

        // The attempted name did NOT land: say so, do not claim success.
        let rpc = DirRpc::with(&["btxnode"]);
        assert!(
            !import_landed_anyway(&rpc, "btxnode-2", &timeout).await,
            "btxnode-2 does not exist; the guard must not answer for btxnode"
        );

        // The attempted name DID land and is still rescanning: report it.
        let rpc = DirRpc::with(&["btxnode", "btxnode-2"]);
        assert!(import_landed_anyway(&rpc, "btxnode-2", &timeout).await);

        // The stale name is exactly the question that always answers yes, which
        // is why passing it was undetectable without this asymmetry.
        let rpc = DirRpc::with(&["btxnode"]);
        assert!(import_landed_anyway(&rpc, "btxnode", &timeout).await);
    }

    #[tokio::test]
    async fn an_rpc_refusal_is_never_second_guessed() {
        // Only transport failures are ambiguous. btxd saying no is btxd telling
        // us something real, and the wallet existing does not make it a success.
        let rpc = DirRpc::with(&["btxnode", "btxnode-2"]);
        let refusal = AppError::Rpc {
            code: -4,
            message: "Wallet file verification failed".into(),
        };
        assert!(!import_landed_anyway(&rpc, "btxnode-2", &refusal).await);
    }

    #[test]
    fn the_collision_message_says_nothing_was_imported() {
        // A refusal must never read as "your coins are gone".
        assert!(COLLISION_ADVICE.contains("NOT imported"));
        assert!(COLLISION_ADVICE.contains("nothing was changed"));
    }

    #[test]
    fn maps_listtransactions_newest_first() {
        let v = serde_json::json!([
            {"category": "receive", "amount": 1.5, "confirmations": 900, "time": 1000u64},
            {"category": "receive", "amount": 0.25, "confirmations": 12, "time": 2000u64}
        ]);
        let txs = map_txs(&v);
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].time, 2000, "newest first for display");
        assert_eq!(txs[0].confirmations, 12);
        assert_eq!(txs[1].amount, 1.5);
        // Wrong shapes never panic.
        assert!(map_txs(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn wallet_view_serializes_for_the_panel() {
        let v = serde_json::to_value(empty_view(true)).unwrap();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["imported"], false);
        assert_eq!(v["txs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn history_rows_carry_the_txid_so_they_can_deep_link() {
        let v = serde_json::json!([
            {"txid": "a".repeat(64), "category": "receive", "amount": 1.0, "confirmations": 3, "time": 1u64}
        ]);
        assert_eq!(map_txs(&v)[0].txid, "a".repeat(64));
        // A row btxd sent without a txid must not panic, just render unclickable.
        let no_txid = serde_json::json!([{"category": "receive", "amount": 1.0}]);
        assert_eq!(map_txs(&no_txid)[0].txid, "");
    }

    #[test]
    fn a_missing_backup_file_is_never_read_as_an_existing_wallet() {
        // The only message that may be swallowed into "already imported".
        assert!(is_already_exists("Wallet name already exists."));
        assert!(is_already_exists("Wallet btxnode already exists"));
        // btxd's answer when the staged path is gone — a HARD failure. Reading
        // this as success shows the user a different wallet's balance and tells
        // them the import worked.
        assert!(!is_already_exists("Backup file does not exist"));
        assert!(!is_already_exists("Bundle file doesn't exist"));
        assert!(!is_already_exists("wallet.dat does not exist at that path"));
        // Unrelated failures stay failures.
        assert!(!is_already_exists("Insufficient funds"));
    }

    #[test]
    fn the_staged_wallet_file_is_not_world_readable() {
        // A wallet.dat, a PQ seed bundle and a dumpwallet text all carry
        // spending keys. std::fs::write would create these 0644.
        let dir = std::env::temp_dir().join(format!("ebtx-stage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("wallet-import.wallet.dat");
        stage_private(&dir, &f, b"SYNTHETIC-FIXTURE-NOT-A-WALLET").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let fm = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
            let dm = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(fm, 0o600, "staged key file must be owner-only, got {fm:o}");
            assert_eq!(dm, 0o700, "staging dir must be owner-only, got {dm:o}");
        }
        assert_eq!(
            std::fs::read(&f).unwrap(),
            b"SYNTHETIC-FIXTURE-NOT-A-WALLET"
        );
        // Re-staging over a leftover file must truncate, not append.
        stage_private(&dir, &f, b"SHORTER").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"SHORTER");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explorer_url_accepts_only_real_txids_and_addresses() {
        let txid = "e".repeat(64);
        assert_eq!(
            explorer_url("tx", &txid).unwrap(),
            format!("https://btxscan.io/tx/{txid}")
        );
        let addr = "btx1zlhp2j9h5fhhntt0cqaprglvafxxy0w8qknmkmglwhc60tyet3vsstyt3pj";
        assert_eq!(
            explorer_url("address", addr).unwrap(),
            format!("https://btxscan.io/address/{addr}")
        );
    }

    #[test]
    fn explorer_url_refuses_to_become_an_arbitrary_url_opener() {
        // Wrong length / non-hex txid.
        assert!(explorer_url("tx", "abc").is_none());
        assert!(explorer_url("tx", &"z".repeat(64)).is_none());
        // Unknown kind.
        assert!(explorer_url("block", &"a".repeat(64)).is_none());
        // Path traversal and scheme smuggling through the id.
        assert!(explorer_url("address", "btx1../../evil").is_none());
        assert!(explorer_url("tx", "../../../etc/passwd").is_none());
        assert!(explorer_url("address", "btx1z@evil.com/x").is_none());
        assert!(explorer_url("address", "https://evil.com").is_none());
        // Right shape, wrong chain prefix.
        assert!(explorer_url("address", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7k").is_none());
        // Absurd length.
        assert!(explorer_url("address", &format!("btx1{}", "q".repeat(200))).is_none());
    }
}
