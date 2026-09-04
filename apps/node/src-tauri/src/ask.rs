//! "Ask your node" — calm, cited answers straight from the user's own node.
//!
//! Every command resolves (never rejects) to an [`Ask`] state: the panel
//! renders `stopped` / `warming` / `unavailable` as plain sentences, so no
//! error ever throws to the webview. Numbers come from the local node's RPC
//! or pure consensus math (`btx_core::supply`) — no external service.

use serde::Serialize;
use tauri::{AppHandle, State};

use btx_core::error::AppError;
use btx_core::node_api as api;
use btx_core::rpc::RpcClient;
use btx_core::supply;

use crate::commands::{snapshot_spec, start_node_inner, stop_node_inner};
use crate::state::{node_datadir, AppState, NodeAppSettings};

/// Wire shape: `{state: "ready", data: T} | {state: "stopped"} | …`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", content = "data", rename_all = "snake_case")]
pub enum Ask<T: Serialize> {
    Ready(T),
    /// Node not running — the panel shows "Start your node to ask it questions."
    Stopped,
    /// RPC_IN_WARMUP (-28): alive but catching up — "ask again in a moment".
    Warming,
    /// Anything else, as one plain sentence.
    Unavailable {
        message: String,
    },
}

/// Map an RPC failure to its calm state.
pub(crate) fn degrade<T: Serialize>(e: AppError) -> Ask<T> {
    match e {
        AppError::Rpc { code: -28, .. } => Ask::Warming,
        other => Ask::Unavailable {
            message: other.to_string(),
        },
    }
}

/// The armed RPC client, or `None` when the node is stopped.
async fn rpc_handle(state: &State<'_, AppState>) -> Option<RpcClient> {
    state.rpc.lock().await.clone()
}

/// The height the user experiences: snapshot-chainstate-aware (assumeutxo).
async fn best_height(rpc: &RpcClient) -> Result<u64, AppError> {
    let chainstates = api::get_chainstates(rpc).await?;
    let h = chainstates.best_height();
    if h > 0 {
        return Ok(h);
    }
    // Degenerate/older shape: fall back to getblockchaininfo.
    Ok(api::get_blockchain_info(rpc).await?.blocks)
}

// ── 1. How far along is the chain? ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChainProgress {
    pub height: u64,
    pub headers: u64,
    pub progress: f64,
    pub near_tip: bool,
    pub peers: i64,
}

#[tauri::command]
pub async fn ask_chain_progress(state: State<'_, AppState>) -> Result<Ask<ChainProgress>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    let chain = match api::get_blockchain_info(&rpc).await {
        Ok(c) => c,
        Err(e) => return Ok(degrade(e)),
    };
    let (chainstates, peers) =
        tokio::join!(api::get_chainstates(&rpc), api::get_connection_count(&rpc));
    let chainstates = chainstates.unwrap_or_default();
    let readiness =
        btx_core::health::sync_readiness(&chainstates, &chain, snapshot_spec().anchor_height);
    Ok(Ask::Ready(ChainProgress {
        height: readiness.height(),
        headers: chain.headers,
        progress: readiness.progress(),
        near_tip: readiness.is_near_tip(),
        peers: peers.unwrap_or(0),
    }))
}

// ── 2. How much BTX exists so far? (pure math from height) ──────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SupplyAnswer {
    pub mined_btx: f64,
    pub cap_btx: f64,
    pub pct: f64,
    pub height: u64,
}

#[tauri::command]
pub async fn ask_supply(state: State<'_, AppState>) -> Result<Ask<SupplyAnswer>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    match best_height(&rpc).await {
        Ok(height) => {
            let mined_btx = supply::sats_to_btx(supply::mined_supply_sats(height));
            Ok(Ask::Ready(SupplyAnswer {
                mined_btx,
                cap_btx: supply::SUPPLY_CAP_BTX,
                pct: mined_btx / supply::SUPPLY_CAP_BTX,
                height,
            }))
        }
        Err(e) => Ok(degrade(e)),
    }
}

// ── 3. When's the next halving? ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct HalvingAnswer {
    pub blocks_remaining: u64,
    pub at_height: u64,
    pub est_secs: u64,
    pub from_reward_btx: f64,
    pub to_reward_btx: f64,
    pub height: u64,
}

#[tauri::command]
pub async fn ask_next_halving(state: State<'_, AppState>) -> Result<Ask<HalvingAnswer>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    match best_height(&rpc).await {
        Ok(height) => {
            let h = supply::next_halving(height);
            Ok(Ask::Ready(HalvingAnswer {
                blocks_remaining: h.blocks_remaining,
                at_height: h.at_height,
                est_secs: h.est_secs,
                from_reward_btx: supply::sats_to_btx(h.from_subsidy_sats),
                to_reward_btx: supply::sats_to_btx(h.to_subsidy_sats),
                height,
            }))
        }
        Err(e) => Ok(degrade(e)),
    }
}

// ── 4. What are fees like right now? ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FeesAnswer {
    /// BTX per kvB for ~6-block confirmation; `null` = the node has no
    /// estimate (quiet network) — the panel falls back to the mempool line.
    pub feerate_btx_kvb: Option<f64>,
    pub mempool_txs: u64,
    pub mempool_vsize: u64,
}

#[tauri::command]
pub async fn ask_fees(state: State<'_, AppState>) -> Result<Ask<FeesAnswer>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    let mempool = match api::get_mempool_info(&rpc).await {
        Ok(m) => m,
        Err(e) => return Ok(degrade(e)),
    };
    // No estimate is a normal quiet-network answer, never an error.
    let feerate = api::estimate_smart_fee(&rpc, 6).await.unwrap_or(None);
    Ok(Ask::Ready(FeesAnswer {
        feerate_btx_kvb: feerate,
        mempool_txs: mempool.size,
        mempool_vsize: mempool.bytes,
    }))
}

// ── 5. How hard is mining right now? ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct MiningAnswer {
    pub difficulty: f64,
    pub network_hashps: f64,
    pub height: u64,
}

#[tauri::command]
pub async fn ask_mining(state: State<'_, AppState>) -> Result<Ask<MiningAnswer>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    match api::get_mining_info(&rpc).await {
        Ok(mi) => Ok(Ask::Ready(MiningAnswer {
            difficulty: mi.difficulty,
            network_hashps: mi.network_hashps,
            height: mi.blocks,
        })),
        Err(e) => Ok(degrade(e)),
    }
}

// ── 6. Show me a block ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BlockAnswer {
    pub height: u64,
    pub hash: String,
    pub time: u64,
    pub n_tx: u64,
    pub size: u64,
}

/// `query`: empty → the tip block; digits → that height; 64-hex → that hash.
#[tauri::command]
pub async fn ask_block(
    state: State<'_, AppState>,
    query: String,
) -> Result<Ask<BlockAnswer>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    let q = query.trim().replace(',', "");
    let fetched = if q.is_empty() {
        match best_height(&rpc).await {
            Ok(h) => api::get_block_by_height(&rpc, h).await,
            Err(e) => Err(e),
        }
    } else if q.chars().all(|c| c.is_ascii_digit()) {
        match q.parse::<u64>() {
            Ok(h) => api::get_block_by_height(&rpc, h).await,
            Err(_) => {
                return Ok(Ask::Unavailable {
                    message: "That block height is out of range.".into(),
                })
            }
        }
    } else if q.len() == 64 && q.chars().all(|c| c.is_ascii_hexdigit()) {
        api::get_block_by_hash(&rpc, &q).await
    } else {
        return Ok(Ask::Unavailable {
            message: "Enter a block height (a number) or a 64-character block hash.".into(),
        });
    };
    match fetched {
        Ok(b) => Ok(Ask::Ready(BlockAnswer {
            height: b.height,
            hash: b.hash,
            time: b.time,
            n_tx: b.n_tx,
            size: b.size,
        })),
        Err(e) => Ok(block_error_to_ask(e)),
    }
}

/// Map a block-lookup failure to its calm sentence. Split out for testing:
/// besides -5/-8 (unknown height/hash), a FRESHLY fast-started node answers
/// -1 "Block not available (not fully downloaded)" for most historical
/// heights — assumeutxo loads the UTXO set instantly but the raw blocks only
/// arrive as the background backfill progresses (observed live in the v0.2
/// e2e). That's a normal state, not an error.
fn block_error_to_ask(e: AppError) -> Ask<BlockAnswer> {
    match e {
        AppError::Rpc { code: -5, .. } | AppError::Rpc { code: -8, .. } => Ask::Unavailable {
            message: "No block with that height or hash on your node.".into(),
        },
        AppError::Rpc {
            code: -1,
            ref message,
        } if message.contains("Block not available") => Ask::Unavailable {
            message: "Your node hasn't downloaded that block yet — it's still \
                          backfilling older history in the background. Ask for a newer \
                          block, or try again later."
                .into(),
        },
        other => degrade(other),
    }
}

// ── Gated: look up a transaction (Explorer mode) ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TxLookup {
    Found {
        txid: String,
        confirmations: u64,
        block_height: Option<u64>,
        block_time: Option<u64>,
        vsize: u64,
        vin_count: usize,
        vout_count: usize,
        total_out_btx: f64,
    },
    /// Historical lookup needs Explorer mode (txindex) — the just-in-time
    /// prompt renders from this.
    NeedsIndex,
    /// txindex is on but still building — pct of the chain indexed so far.
    Building {
        pct: f64,
    },
    NotFound,
}

#[tauri::command]
pub async fn ask_transaction(
    state: State<'_, AppState>,
    txid: String,
) -> Result<Ask<TxLookup>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    let t = txid.trim().to_lowercase();
    if t.len() != 64 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Ask::Unavailable {
            message: "A transaction id is 64 hex characters.".into(),
        });
    }
    match api::get_raw_transaction(&rpc, &t).await {
        Ok(v) => {
            let s = api::tx_summary(&v);
            // Enrich with the block height when the tx is confirmed (verbose
            // getrawtransaction carries the hash but not the height).
            let block_height = match &s.block_hash {
                Some(h) => api::get_block_by_hash(&rpc, h).await.ok().map(|b| b.height),
                None => None,
            };
            Ok(Ask::Ready(TxLookup::Found {
                txid: s.txid,
                confirmations: s.confirmations,
                block_height,
                block_time: s.block_time,
                vsize: s.vsize,
                vin_count: s.vin_count,
                vout_count: s.vout_count,
                total_out_btx: s.total_out_btx,
            }))
        }
        Err(AppError::Rpc { code: -5, .. }) => {
            // Not in the mempool. Without txindex a historical tx is
            // unreachable → the gate. With txindex: building vs truly absent.
            if !NodeAppSettings::load(&node_datadir()).txindex_enabled {
                return Ok(Ask::Ready(TxLookup::NeedsIndex));
            }
            match api::get_tx_index_info(&rpc).await {
                Ok(Some(st)) if !st.synced => {
                    let tip = best_height(&rpc).await.unwrap_or(0).max(1);
                    Ok(Ask::Ready(TxLookup::Building {
                        pct: st.best_block_height as f64 / tip as f64,
                    }))
                }
                _ => Ok(Ask::Ready(TxLookup::NotFound)),
            }
        }
        Err(e) => Ok(degrade(e)),
    }
}

// ── Explorer mode (txindex) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TxIndexAnswer {
    /// The persisted user choice (settings).
    pub enabled: bool,
    /// Whether the RUNNING node has the index configured (getindexinfo).
    pub configured: bool,
    pub synced: bool,
    pub pct: f64,
}

#[tauri::command]
pub async fn ask_tx_index_status(state: State<'_, AppState>) -> Result<Ask<TxIndexAnswer>, String> {
    let enabled = NodeAppSettings::load(&node_datadir()).txindex_enabled;
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(Ask::Stopped);
    };
    match api::get_tx_index_info(&rpc).await {
        Ok(Some(st)) => {
            let tip = best_height(&rpc).await.unwrap_or(0).max(1);
            Ok(Ask::Ready(TxIndexAnswer {
                enabled,
                configured: true,
                synced: st.synced,
                pct: if st.synced {
                    1.0
                } else {
                    st.best_block_height as f64 / tip as f64
                },
            }))
        }
        Ok(None) => Ok(Ask::Ready(TxIndexAnswer {
            enabled,
            configured: false,
            synced: false,
            pct: 0.0,
        })),
        Err(e) => Ok(degrade(e)),
    }
}

/// Turn Explorer mode on/off: persist the choice, write/remove `txindex=1` in
/// the faststart conf, and (when the node is running) restart it gracefully so
/// btxd picks the flag up. The node stays fully usable while the index builds.
#[tauri::command]
pub async fn set_explorer_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    on: bool,
) -> Result<(), String> {
    let datadir = node_datadir();
    let conf = datadir.join("faststart").join("faststart.conf");
    btx_core::setup::set_conf_kv(&conf, "txindex", if on { Some("1") } else { None })
        .map_err(|e| e.to_string())?;
    NodeAppSettings::update(&datadir, |s| s.txindex_enabled = on);
    if state.rpc.lock().await.is_some() {
        stop_node_inner(&state).await;
        start_node_inner(&app, &state).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_serializes_tagged_states() {
        let ready = Ask::Ready(SupplyAnswer {
            mined_btx: 3_114_020.0,
            cap_btx: 21_000_000.0,
            pct: 0.1483,
            height: 155_700,
        });
        let v = serde_json::to_value(&ready).unwrap();
        assert_eq!(v["state"], "ready");
        assert_eq!(v["data"]["height"], 155_700);

        let stopped: Ask<SupplyAnswer> = Ask::Stopped;
        assert_eq!(serde_json::to_value(&stopped).unwrap()["state"], "stopped");

        let warm: Ask<SupplyAnswer> = degrade(AppError::Rpc {
            code: -28,
            message: "warming".into(),
        });
        assert_eq!(serde_json::to_value(&warm).unwrap()["state"], "warming");

        let un: Ask<SupplyAnswer> = degrade(AppError::Http("boom".into()));
        let v = serde_json::to_value(&un).unwrap();
        assert_eq!(v["state"], "unavailable");
        assert!(v["data"]["message"].as_str().unwrap().contains("boom"));
    }

    #[test]
    fn block_errors_map_to_calm_sentences() {
        // Unknown height/hash.
        let a = block_error_to_ask(AppError::Rpc {
            code: -5,
            message: "Block not found".into(),
        });
        match a {
            Ask::Unavailable { message } => assert!(message.contains("No block")),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        // Fresh fast-started node: block data not backfilled yet.
        let a = block_error_to_ask(AppError::Rpc {
            code: -1,
            message: "Block not available (not fully downloaded)".into(),
        });
        match a {
            Ask::Unavailable { message } => {
                assert!(message.contains("backfilling"), "got: {message}")
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        // Warmup still degrades to Warming, not a block message.
        let a = block_error_to_ask(AppError::Rpc {
            code: -28,
            message: "Verifying blocks".into(),
        });
        assert!(matches!(a, Ask::Warming));
    }

    #[test]
    fn tx_lookup_serializes_kinds() {
        let v = serde_json::to_value(&TxLookup::NeedsIndex).unwrap();
        assert_eq!(v["kind"], "needs_index");
        let v = serde_json::to_value(&TxLookup::Building { pct: 0.42 }).unwrap();
        assert_eq!(v["kind"], "building");
        assert_eq!(v["pct"], 0.42);
    }
}
