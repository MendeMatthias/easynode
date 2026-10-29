//! btx-sentinel: the always-on watcher on the Azure box, beside btxd and
//! btxd2, for the watching a 60-second Vercel function cannot do.
//!
//! It ONLY reads: GETs to easybtx.com and the witnesses, read-only RPC on
//! btxd2 (`getblockchaininfo`, `getblockhash`, `getmatmulattestations`, the
//! same calls `btx-witness` serves `/signers/recent` from), and a P2P version
//! handshake to each shipped seed. It listens on nothing. Its alarms go
//! through the box's existing Orca route and ntfy topic, read from
//! `/etc/btxscan-selfcheck.conf`.
//!
//! What it alarms on (thresholds are flags, see `decide::Thresholds`):
//!   1. snapshots: the newest confirmed snapshot older than 6 h; the producer
//!      quiet (no new statement for 12 h); a confirmer quiet (the newest
//!      statement still short of a second operator after 6 h); a dispute
//!      opened. "No snapshot is confirmed yet" is a once-a-day reminder.
//!   2. signers: fewer than 2 distinct keys over the last 100 blocks (a daily
//!      warning); no new signature for 20 min (an alarm).
//!   3. seeds: a failed handshake twice in a row, probed every 30 min.
//!   4. the census stale, a witness down or more than 6 blocks behind btxd2
//!      (a witness that has never answered is not deployed yet: skipped).
//!   5. fleet: a node on the current engine further behind over two census
//!      runs in a row.
//!
//! Layout: `decide` decides (pure, fake clock in tests), `run` reads and sends
//! behind traits, `channels` is Orca and ntfy, `p2p` is the handshake.

pub mod channels;
pub mod decide;
pub mod p2p;
pub mod run;
