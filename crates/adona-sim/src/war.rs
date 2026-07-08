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
    /// in id order, and when more than two factions share a site only the
    /// two lowest-id owners fight this tick (a full multi-way melee is
    /// TODO(combat)).
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
            let (a, b) = (owners[0], owners[1]);
            let attacker_assets = by_owner[&a].clone();
            let defender_assets = by_owner[&b].clone();
            let _ = self.resolve_battle(site, a, &attacker_assets, b, &defender_assets);
        }
    }
}
