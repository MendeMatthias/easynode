//! BTX issuance math — pure, deterministic, no RPC.
//!
//! "How much BTX exists" is answered from the block height and the subsidy
//! schedule, NOT `gettxoutsetinfo`: that call is slow and, on an assumeutxo
//! node, reflects the loaded snapshot chainstate rather than the tip. The
//! schedule is exact: 20 BTX per block, halving every 525,000 blocks, 90 s
//! target spacing → 21,000,000 BTX cap (2 × 525_000 × 20).

/// Blocks between subsidy halvings (BTX consensus).
pub const HALVING_INTERVAL: u64 = 525_000;
/// Genesis-era block subsidy: 20 BTX, in sats.
pub const INITIAL_SUBSIDY_SATS: u64 = 2_000_000_000;
/// Target block spacing.
pub const BLOCK_TIME_SECS: u64 = 90;
/// The nominal supply cap in BTX (the asymptote of the halving series).
pub const SUPPLY_CAP_BTX: f64 = 21_000_000.0;

/// The block subsidy at `height`, in sats.
pub fn subsidy_sats_at(height: u64) -> u64 {
    let epoch = height / HALVING_INTERVAL;
    if epoch >= 63 {
        return 0; // shifted to nothing long before this
    }
    INITIAL_SUBSIDY_SATS >> epoch
}

/// Total sats issued by all blocks 0..=height (closed form over full epochs).
/// Includes the genesis subsidy — like Bitcoin's, BTX's genesis coinbase is
/// not spendable, but at ±20 BTX on a 10.5M+ display number the simpler
/// convention wins.
pub fn mined_supply_sats(height: u64) -> u64 {
    let mut total: u64 = 0;
    let mut subsidy = INITIAL_SUBSIDY_SATS;
    let mut remaining = height + 1; // blocks 0..=height
    while remaining > 0 && subsidy > 0 {
        let n = remaining.min(HALVING_INTERVAL);
        total += n * subsidy;
        remaining -= n;
        subsidy >>= 1;
    }
    total
}

#[derive(Debug, Clone, PartialEq)]
pub struct NextHalving {
    pub at_height: u64,
    pub blocks_remaining: u64,
    /// blocks_remaining × 90 s — an estimate, cite it as one.
    pub est_secs: u64,
    pub from_subsidy_sats: u64,
    pub to_subsidy_sats: u64,
}

/// The next halving after `height` (a tip AT a halving height already pays
/// the halved subsidy, so "next" is the following boundary).
pub fn next_halving(height: u64) -> NextHalving {
    let at_height = (height / HALVING_INTERVAL + 1) * HALVING_INTERVAL;
    let blocks_remaining = at_height - height;
    NextHalving {
        at_height,
        blocks_remaining,
        est_secs: blocks_remaining * BLOCK_TIME_SECS,
        from_subsidy_sats: subsidy_sats_at(height),
        to_subsidy_sats: subsidy_sats_at(at_height),
    }
}

pub fn sats_to_btx(sats: u64) -> f64 {
    sats as f64 / 100_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsidy_schedule_halves_at_boundaries() {
        assert_eq!(subsidy_sats_at(0), 2_000_000_000);
        assert_eq!(subsidy_sats_at(524_999), 2_000_000_000);
        assert_eq!(subsidy_sats_at(525_000), 1_000_000_000);
        assert_eq!(subsidy_sats_at(1_049_999), 1_000_000_000);
        assert_eq!(subsidy_sats_at(1_050_000), 500_000_000);
        assert_eq!(subsidy_sats_at(63 * HALVING_INTERVAL), 0);
    }

    #[test]
    fn mined_supply_matches_closed_form() {
        // Genesis alone.
        assert_eq!(mined_supply_sats(0), 2_000_000_000);
        // Whole first epoch: 525,000 blocks × 20 BTX = 10.5M BTX.
        assert_eq!(mined_supply_sats(524_999), 525_000 * 2_000_000_000);
        // One block into epoch 1 adds a halved subsidy.
        assert_eq!(
            mined_supply_sats(525_000),
            525_000 * 2_000_000_000 + 1_000_000_000
        );
        // Far future: approaches (never exceeds) the 21M cap.
        let deep = mined_supply_sats(100 * HALVING_INTERVAL);
        assert!(sats_to_btx(deep) < SUPPLY_CAP_BTX);
        assert!(sats_to_btx(deep) > SUPPLY_CAP_BTX - 1.0);
    }

    #[test]
    fn next_halving_from_todays_range() {
        // v0.33.1 snapshot anchor height — the realistic live neighborhood.
        let h = next_halving(155_700);
        assert_eq!(h.at_height, 525_000);
        assert_eq!(h.blocks_remaining, 369_300);
        assert_eq!(h.est_secs, 369_300 * 90);
        assert_eq!(h.from_subsidy_sats, 2_000_000_000);
        assert_eq!(h.to_subsidy_sats, 1_000_000_000);
    }

    #[test]
    fn next_halving_at_a_boundary_points_to_the_following_one() {
        let h = next_halving(525_000);
        assert_eq!(h.at_height, 1_050_000);
        assert_eq!(h.blocks_remaining, 525_000);
        assert_eq!(h.from_subsidy_sats, 1_000_000_000);
        assert_eq!(h.to_subsidy_sats, 500_000_000);
    }
}
