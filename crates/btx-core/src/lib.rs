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

pub mod backend;
pub mod checkin;
pub mod datadir;
pub mod disk;
pub mod error;
pub mod frontier;
pub mod health;
pub mod installer;
pub mod node;
pub mod node_api;
pub mod platform;
pub mod power;
pub mod rpc;
pub mod service_report;
pub mod setup;
pub mod snapshot;
pub mod supply;
pub mod wallet_format;
pub mod watchdog;
