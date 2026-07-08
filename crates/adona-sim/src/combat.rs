//! RISK-style battle resolution: the collapsed stand-in for full
//! mech-vs-mech tactical combat.
//!
//! This is deliberately not the real cockpit/tactical fight — that is a
//! separate, much later system the docket describes as first-person,
//! real-time, and cockpit-driven. What this module gives the strategic
//! layer *now* is a real, deterministic, provenance-respecting resolution
//! between two forces that are physically at the same site: power is
//! computed from real asset kind, quality, and condition; defenders get a
//! mechanical home-ground edge; losses are real (destroyed units go to zero
//! condition rather than disappearing, some captured units change owner);
//! and every engaged asset still exists and is still traceable afterward.
//! No mech is invented or deleted to make a battle happen.
//!
//! TODO(combat): replace the flat kind/quality power formula with real
//! stats once components carry combat-relevant numbers (armor, weapons,
//! heat); traits and gear bonuses; multi-faction battles instead of
//! strictly two-sided; convoy interception as its own combat shape.

use crate::assets::AssetKind;
use crate::events::EventKind;
use crate::goods::QualityGrade;
use crate::ids::*;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

/// Defenders fighting from their own ground get a real, mechanical edge —
/// the RISK-style bias requested for this first collapsed model.
pub const DEFENDER_BONUS_PCT: u64 = 120;
/// Fraction of engaged units the losing side loses.
pub const LOSER_ATTRITION_PCT: u64 = 35;
/// Fraction of engaged units the winning side loses even so — war is not free.
pub const WINNER_ATTRITION_PCT: u64 = 10;
/// Of the losing side's lost units, this fraction is captured (ownership
/// transfers to the winner) rather than destroyed outright.
pub const CAPTURE_FRACTION_PCT: u64 = 30;

fn base_power(kind: AssetKind) -> u64 {
    match kind {
        AssetKind::Mech => 100,
        AssetKind::Vehicle => 40,
        AssetKind::Weapon => 20,
        AssetKind::Equipment => 10,
        AssetKind::FactoryTooling | AssetKind::CargoContainer => 0,
    }
}

fn quality_multiplier_pct(q: QualityGrade) -> u64 {
    match q {
        QualityGrade::Salvage => 70,
        QualityGrade::Poor => 85,
        QualityGrade::Standard => 100,
        QualityGrade::Fine => 115,
        QualityGrade::Exceptional => 130,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleOutcome {
    pub site: LocationId,
    pub attacker: ActorId,
    pub defender: ActorId,
    pub attacker_power: u64,
    pub defender_power: u64,
    pub attacker_won: bool,
    pub attacker_losses: Vec<AssetId>,
    pub defender_losses: Vec<AssetId>,
    /// Losing-side units the winner captured (ownership transferred) rather
    /// than destroyed.
    pub captured: Vec<AssetId>,
}

impl World {
    /// Real combat power of one asset: base power for its kind, scaled by
    /// its actual quality grade and current condition. Destroyed
    /// (`condition_pct == 0`) assets contribute nothing.
    pub fn asset_combat_power(&self, asset: AssetId) -> Result<u64, SimError> {
        let a = self.assets.get(&asset).ok_or(SimError::UnknownAsset(asset))?;
        let design = self.designs.get(&a.design).ok_or(SimError::UnknownDesign(a.design))?;
        let base = base_power(design.kind);
        let power = base.saturating_mul(quality_multiplier_pct(a.quality)) / 100;
        Ok(power.saturating_mul(a.condition_pct as u64) / 100)
    }

    /// Resolve a real battle between two forces physically at the same
    /// site, each entirely owned by its side. Nothing here spawns or erases
    /// an asset: losses reduce `condition_pct` to zero (wreckage, still a
    /// real record) and a portion of the loser's losses change owner
    /// (capture) instead of being destroyed. Territory control changes
    /// hands only on an attacker win. Automatically records a real intel
    /// observation of the battle.
    pub fn resolve_battle(
        &mut self,
        site: LocationId,
        attacker: ActorId,
        attacker_assets: &[AssetId],
        defender: ActorId,
        defender_assets: &[AssetId],
    ) -> Result<BattleOutcome, SimError> {
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        if attacker_assets.is_empty() || defender_assets.is_empty() {
            return Err(SimError::InvalidState("a battle needs real forces on both sides".into()));
        }
        for &a in attacker_assets.iter().chain(defender_assets) {
            let asset = self.assets.get(&a).ok_or(SimError::UnknownAsset(a))?;
            if self.resolve_site(asset.location) != Some(site) {
                return Err(SimError::NotColocated);
            }
        }
        for &a in attacker_assets {
            let owner = self.assets[&a].owner;
            if owner != attacker {
                return Err(SimError::NotOwner { actor: owner });
            }
        }
        for &a in defender_assets {
            let owner = self.assets[&a].owner;
            if owner != defender {
                return Err(SimError::NotOwner { actor: owner });
            }
        }

        let mut attacker_power = 0u64;
        for &a in attacker_assets {
            attacker_power = attacker_power.saturating_add(self.asset_combat_power(a)?);
        }
        let mut defender_power = 0u64;
        for &a in defender_assets {
            defender_power = defender_power.saturating_add(self.asset_combat_power(a)?);
        }
        let defended_power = defender_power.saturating_mul(DEFENDER_BONUS_PCT) / 100;

        let total = (attacker_power as u128 + defended_power as u128).max(1);
        let attacker_win_pct = ((attacker_power as u128 * 100) / total).min(100) as u8;
        let roll = self.rng.roll_percent();
        let attacker_won = roll < attacker_win_pct;

        let (winner_assets, loser_assets, winner) = if attacker_won {
            (attacker_assets, defender_assets, attacker)
        } else {
            (defender_assets, attacker_assets, defender)
        };

        // Weakest units lost first — deterministic given the win/loss roll;
        // no further hidden randomness in *which* units fall.
        let mut loser_sorted = loser_assets.to_vec();
        loser_sorted.sort_by_key(|a| (self.asset_combat_power(*a).unwrap_or(0), a.0));
        let mut winner_sorted = winner_assets.to_vec();
        winner_sorted.sort_by_key(|a| (self.asset_combat_power(*a).unwrap_or(0), a.0));

        // A side that loses a battle loses at least one real unit — a 35%
        // attrition rate rounding down to zero against a lone defender
        // would make losing free, which is not the point of fighting.
        let loser_losses_n = ((loser_sorted.len() as u64 * LOSER_ATTRITION_PCT / 100).max(1) as usize)
            .min(loser_sorted.len());
        let winner_losses_n = (winner_sorted.len() as u64 * WINNER_ATTRITION_PCT / 100) as usize;

        let mut captured = Vec::new();
        let mut loser_losses = Vec::new();
        for (i, &asset) in loser_sorted.iter().take(loser_losses_n).enumerate() {
            if (i as u64 * 100 / loser_losses_n.max(1) as u64) < CAPTURE_FRACTION_PCT {
                self.transfer_asset(asset, winner)?;
                captured.push(asset);
            } else {
                self.assets.get_mut(&asset).unwrap().condition_pct = 0;
            }
            self.remove_asset_from_formations(asset);
            self.move_asset_raw(asset, crate::locations::LocationRef::Site(site))?;
            loser_losses.push(asset);
        }
        let winner_losses: Vec<AssetId> = winner_sorted.iter().take(winner_losses_n).copied().collect();
        for &asset in &winner_losses {
            self.assets.get_mut(&asset).unwrap().condition_pct = 0;
            self.remove_asset_from_formations(asset);
            self.move_asset_raw(asset, crate::locations::LocationRef::Site(site))?;
        }

        if attacker_won {
            self.locations.get_mut(&site).unwrap().controller = Some(attacker);
        } else if self.locations[&site].controller.is_none() {
            self.locations.get_mut(&site).unwrap().controller = Some(defender);
        }

        self.push_event(EventKind::BattleResolved {
            site,
            attacker,
            defender,
            attacker_won,
            attacker_power,
            defender_power,
        });
        let _ = self.record_observation(
            None,
            crate::intel::IntelSubject::Battle { site },
            site,
            "battle resolved",
            100,
            0,
        );

        let (attacker_losses, defender_losses) = if attacker_won {
            (winner_losses, loser_losses)
        } else {
            (loser_losses, winner_losses)
        };
        Ok(BattleOutcome {
            site,
            attacker,
            defender,
            attacker_power,
            defender_power,
            attacker_won,
            attacker_losses,
            defender_losses,
            captured,
        })
    }
}
