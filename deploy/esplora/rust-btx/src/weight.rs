//! Weight / vsize constants and helpers.
//!
//! Authoritative source: `btx-core src/consensus/consensus.h:16-31`.
//!
//! BTX sets `WITNESS_SCALE_FACTOR == 1`, so there is **no witness discount**: a
//! transaction's (and block's) weight equals its vsize equals its full serialized size
//! (with witness *and* shielded bytes included). This is the single most important
//! divergence for any fee/size accounting ported from Bitcoin.

use bitcoin::Weight;

/// `WITNESS_SCALE_FACTOR` (`consensus.h:16-31`). Bitcoin uses 4; BTX uses **1**.
pub const WITNESS_SCALE_FACTOR: u64 = 1;

/// `MAX_BLOCK_WEIGHT` (`consensus.h:16-31`).
pub const MAX_BLOCK_WEIGHT: u64 = 24_000_000;

/// Weight from a serialized size, applying the BTX scale factor of 1 (i.e. identity).
/// Used by [`crate::Transaction::weight`] and [`crate::Block::weight`].
#[inline]
pub fn weight_from_size(size: usize) -> Weight {
    Weight::from_wu(size as u64 * WITNESS_SCALE_FACTOR)
}
