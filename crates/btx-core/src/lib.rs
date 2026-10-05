//! btx-core — the shared BTX node engine.
//!
//! Extracted verbatim from the easyBTX miner (see the design spec at
//! `docs/superpowers/specs/2026-07-10-easybtx-node-design.md`): the miner's
//! proven node lifecycle (spawn/supervise/stop btxd with pidfile + foreign-node
//! reconciliation), JSON-RPC client + typed node API, assumeutxo-aware sync
//! readiness, disk maintenance, and the self-contained faststart provisioning
//! (bundled binaries + snapshot download + `loadtxoutset`).
//!
//! Consumed by two apps via `path` dependencies:
//!   * the easyBTX miner (`src-tauri/`), which re-exports these modules through
//!     facade modules so its historical `crate::node::…` paths keep working;
//!   * easyBTX Node (`apps/node/`), the standalone one-click full-node app.
//!
//! Module-visibility note: everything here is `pub` (this is a library crate);
//! items that were `pub(crate)` inside the miner were widened mechanically
//! during the extraction — no behavior changed.

pub mod aside;
pub mod attested_snapshot;
pub mod backend;
pub mod catchup_assist;
pub mod chain_agreement;
pub mod checkin;
pub mod confirmed_load;
pub mod confirmed_snapshot;
pub mod console_policy;
pub mod datadir;
pub mod diagnostics;
pub mod diary;
pub mod disk;
pub mod engine_priority;
pub mod engine_warnings;
pub mod error;
pub mod esplora;
pub mod esplora_freshness;
pub mod esplora_sidecar;
#[cfg(test)]
pub(crate) mod fake_node;
pub mod fast_forward;
pub mod fork;
pub mod frontier;
pub mod fsx;
pub mod header_path;
pub mod health;
pub mod installer;
pub mod known_invalid;
pub mod nickname;
pub mod node;
pub mod node_api;
pub mod operators;
pub mod platform;
pub mod power;
pub mod read_block_recovery;
pub mod role;
pub mod rpc;
pub mod sentinel;
pub mod service_report;
pub mod setup;
pub mod signer;
pub mod snapshot;
pub mod snapshot_serve;
pub mod snapshot_start;
pub mod statement_check;
pub mod stuck_blocks;
pub mod supply;
pub mod wallet_format;
pub mod watchdog;
pub mod witness;
