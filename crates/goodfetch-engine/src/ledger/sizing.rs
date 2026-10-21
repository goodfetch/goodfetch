//! How big a Slice a run takes (ADR 0041): the smaller of a share of the
//! Limit and what is left over twice the live runs holding Slices of it, so
//! Slices shrink as the Limit runs out and no run holds back what another
//! could use; never below a minimum share, so the number of ledger
//! transactions stays bounded, and a remainder below that minimum goes
//! whole to the run that asks.

/// The shares a Slice is sized by, in thousandths of the Limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceSizing {
    /// The largest Slice: 10, 1 % of the Limit.
    pub share: u64,
    /// The smallest Slice: 1, 0.1 % of the Limit.
    pub minimum: u64,
}

impl Default for SliceSizing {
    fn default() -> Self {
        Self {
            share: 10,
            minimum: 1,
        }
    }
}

/// The size of the Slice a run takes of a Limit of `amount` with `left`
/// remaining, `live_runs` holding Slices of it, the asker included: never
/// more than is left, and none only when nothing is.
pub fn slice_size(sizing: SliceSizing, amount: u64, left: u64, live_runs: u64) -> u64 {
    let share = amount.saturating_mul(sizing.share) / 1000;
    let minimum = amount.saturating_mul(sizing.minimum).div_ceil(1000).max(1);
    let even = left / live_runs.max(1).saturating_mul(2);
    share.min(even).max(minimum).min(left)
}

#[cfg(test)]
mod tests;
