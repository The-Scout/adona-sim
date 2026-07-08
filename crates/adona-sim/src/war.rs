//! Faction war AI: the "ant simulator" behavior.
//!
//! Every tick, at every site, any two different factions whose formations
//! physically occupy the same ground fight — automatically, deterministically,
//! using only their real assembled formations. Nobody scripts the fight;
//! the world makes it happen because two hostile factions are standing on
//! each other. This is the acceptance feeling the docket names directly:
//! "the factions are fighting among themselves because the simulated world
//! made them do it."
//!
//! Formations now march toward contested/enemy territory on their own via
//! [`World::tick_faction_deployment`] instead of only clashing where they
//! already happen to be. Still TODO(war): pulling damaged formations back to
//! refit; faction diplomacy (currently every distinct owner is treated as
//! hostile — there is no alliance system yet).

use crate::ids::*;
use crate::world::World;
use std::collections::BTreeMap;

impl World {
    /// Automatic territorial combat phase. Deterministic: sites are visited
    /// in id order. When three or more factions share a site, battles
    /// cascade — the winner of each pairwise fight (with its real surviving
    /// assets) immediately faces the next owner in id order — so a three-way
    /// standoff actually resolves in one tick instead of leaving everyone
    /// past the first two idle next to a live enemy.
    pub(crate) fn tick_faction_war(&mut self) {
        let sites: Vec<LocationId> = self.locations.keys().copied().collect();
        for site in sites {
            let mut by_owner: BTreeMap<ActorId, Vec<AssetId>> = BTreeMap::new();
            for f in self.formations.values() {
                if f.current_site() != Some(site) {
                    continue;
                }
                by_owner.entry(f.owner).or_default().extend(f.assets.iter().copied());
            }
            // Only units that are still combat-capable (condition > 0) can
            // fight; wrecks from an earlier battle this tick don't refight.
            for assets in by_owner.values_mut() {
                assets.retain(|a| self.assets.get(a).map(|x| x.condition_pct > 0).unwrap_or(false));
            }
            by_owner.retain(|_, assets| !assets.is_empty());
            if by_owner.len() < 2 {
                continue;
            }
            let mut owners: Vec<ActorId> = by_owner.keys().copied().collect();
            owners.sort();

            let mut champion = owners[0];
            let mut champion_assets = by_owner[&champion].clone();
            for &challenger in &owners[1..] {
                if champion_assets.is_empty() {
                    break;
                }
                let mut challenger_assets = by_owner[&challenger].clone();
                challenger_assets.retain(|a| self.assets.get(a).map(|x| x.condition_pct > 0).unwrap_or(false));
                if challenger_assets.is_empty() {
                    continue;
                }
                let Ok(outcome) =
                    self.resolve_battle(site, champion, &champion_assets, challenger, &challenger_assets)
                else {
                    continue;
                };
                let (winner, mut winner_assets) = if outcome.attacker_won {
                    (champion, champion_assets.clone())
                } else {
                    (challenger, challenger_assets.clone())
                };
                let losses = if outcome.attacker_won { &outcome.attacker_losses } else { &outcome.defender_losses };
                winner_assets.retain(|a| !losses.contains(a));
                champion = winner;
                champion_assets = winner_assets;
            }
        }

        // A formation that lost every real asset in combat is no longer a
        // real formation — drop the empty roster rather than leaving a
        // ghost entry behind.
        self.formations.retain(|_, f| !f.assets.is_empty());
    }
}
