//! The 13-tier refinement ladder.
//!
//! Tier is an *instance* property (MW5-style): the same commodity, component,
//! or asset design exists at tiers 1..=13, and a thing's tier comes from how
//! it was made — refined goods can be refined again to climb the ladder, and
//! assembled goods inherit the minimum tier of their real inputs. Higher-tier
//! combat assets hit harder and take more of a beating; the scaling lives
//! here so every consumer (stats, UI, future combat) agrees on the curve.

use crate::SimError;

/// Bottom of the ladder: raw extraction and unremarkable pre-war stock.
pub const MIN_TIER: u8 = 1;
/// Top of the ladder. Refining a tier-13 input is refused, not wasted.
pub const MAX_TIER: u8 = 13;

/// Validate a tier value at every boundary where one enters the world.
pub fn check_tier(tier: u8) -> Result<u8, SimError> {
    if (MIN_TIER..=MAX_TIER).contains(&tier) {
        Ok(tier)
    } else {
        Err(SimError::InvalidTier(tier))
    }
}

/// Stat multiplier in percent for a tier. Integer math only — no floats in
/// strategic state. Tier 1 = 100%, each tier adds 15 points, tier 13 = 280%.
pub fn tier_multiplier_pct(tier: u8) -> u64 {
    let t = tier.clamp(MIN_TIER, MAX_TIER) as u64;
    100 + 15 * (t - 1)
}

/// Apply the tier curve to a base stat.
pub fn scale_by_tier(base: u64, tier: u8) -> u64 {
    base.saturating_mul(tier_multiplier_pct(tier)) / 100
}
