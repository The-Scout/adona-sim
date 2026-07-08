//! Data-driven world seeding: the game's actual faction content (names,
//! doctrine, TO&E) lives in `assets/world_seed.json`, not hardcoded Rust, so
//! it can be modded without recompiling. `adona-sim` stays generic — this
//! module is purely `adona-game` glue that reads a schema-validated file and
//! drives the same public `World` builder API any other consumer would use.
//!
//! TODO(economy): this seed gives every faction a real starting garrison and
//! a standing `FactionGoal`, but no factories/recipes/mines of their own
//! (besides the shared Cradle of Conflict), so losses currently cannot be
//! replenished by production — the goal will just place buy orders that
//! never fill. Real per-faction production chains are future work.

use adona_sim::actors::{ActorKind, Credits};
use adona_sim::assets::{AssetKind, AssetOrigin, ComponentCategory, ComponentSlot};
use adona_sim::goods::QualityGrade;
use adona_sim::ids::{ActorId, ComponentDefId, DesignId, LocationId, ToeTemplateId};
use adona_sim::locations::{LocationKind, MineReserves};
use adona_sim::toe::ToeSlot;
use adona_sim::World;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct WorldSeedFile {
    pub seed: u64,
    pub component_defs: Vec<ComponentDefSeed>,
    pub designs: Vec<DesignSeed>,
    pub locations: Vec<LocationSeed>,
    pub routes: Vec<RouteSeed>,
    pub factions: Vec<FactionSeed>,
}

#[derive(Debug, Deserialize)]
pub struct ComponentDefSeed {
    pub key: String,
    pub name: String,
    pub tier: u8,
}

#[derive(Debug, Deserialize)]
pub struct SlotSeed {
    pub name: String,
    pub component_def_key: String,
}

#[derive(Debug, Deserialize)]
pub struct DesignSeed {
    pub key: String,
    pub name: String,
    pub kind: AssetKind,
    pub cargo_capacity_kg: Option<u64>,
    #[serde(default)]
    pub slots: Vec<SlotSeed>,
}

#[derive(Debug, Deserialize)]
pub struct LocationSeed {
    pub key: String,
    pub name: String,
    pub kind: LocationKind,
    pub position: (i64, i64),
    #[serde(default)]
    pub controller_key: Option<String>,
    #[serde(default)]
    pub mine_reserves: Option<MineReserves>,
}

#[derive(Debug, Deserialize)]
pub struct RouteSeed {
    pub from_key: String,
    pub to_key: String,
    pub distance_days: u64,
}

#[derive(Debug, Deserialize)]
pub struct ToeSlotSeed {
    pub role: String,
    pub design_key: String,
    pub count: u32,
}

#[derive(Debug, Deserialize)]
pub struct FactionSeed {
    pub key: String,
    pub name: String,
    pub kind: ActorKind,
    pub starting_treasury: Credits,
    pub doctrine: String,
    pub home_location_key: String,
    pub toe: Vec<ToeSlotSeed>,
}

/// Build a real `World` from a parsed seed file: every actor, location,
/// design, component, formation, and TO&E goal created here goes through
/// the same public builder API a hand-written scenario would use — nothing
/// is spawned outside it.
pub fn build_world(seed: &WorldSeedFile) -> Result<World, String> {
    let mut w = World::new(seed.seed);

    let mut component_defs: HashMap<&str, ComponentDefId> = HashMap::new();
    for def in &seed.component_defs {
        let id = w.define_component_def(&def.name, def.tier, ComponentCategory::MechOrEquipment);
        component_defs.insert(&def.key, id);
    }

    let mut designs: HashMap<&str, DesignId> = HashMap::new();
    for d in &seed.designs {
        let mut slots = Vec::with_capacity(d.slots.len());
        for slot in &d.slots {
            let def_id = *component_defs
                .get(slot.component_def_key.as_str())
                .ok_or_else(|| format!("design {:?} slot {:?}: unknown component_def_key {:?}", d.key, slot.name, slot.component_def_key))?;
            slots.push(ComponentSlot { name: slot.name.clone(), accepts: vec![def_id] });
        }
        let id = w
            .define_design(&d.name, d.kind, slots, d.cargo_capacity_kg, None)
            .map_err(|e| format!("design {:?}: {e}", d.key))?;
        designs.insert(&d.key, id);
    }

    let mut locations: HashMap<&str, LocationId> = HashMap::new();
    for l in &seed.locations {
        let id = w.create_location(&l.name, l.kind, l.position);
        if let Some(reserves) = l.mine_reserves {
            w.configure_mine(id, reserves).map_err(|e| format!("location {:?}: {e}", l.key))?;
        }
        locations.insert(&l.key, id);
    }

    for r in &seed.routes {
        let from = *locations
            .get(r.from_key.as_str())
            .ok_or_else(|| format!("route: unknown from_key {:?}", r.from_key))?;
        let to = *locations
            .get(r.to_key.as_str())
            .ok_or_else(|| format!("route: unknown to_key {:?}", r.to_key))?;
        w.create_route(from, to, r.distance_days).map_err(|e| format!("route {:?}->{:?}: {e}", r.from_key, r.to_key))?;
    }

    let mut faction_ids: HashMap<&str, ActorId> = HashMap::new();
    for f in &seed.factions {
        let id = w.create_actor(&f.name, f.kind, f.starting_treasury);
        faction_ids.insert(&f.key, id);
    }

    // Controllers are applied only now that faction ids exist.
    for l in &seed.locations {
        let Some(controller_key) = &l.controller_key else { continue };
        let site = locations[l.key.as_str()];
        let controller = *faction_ids
            .get(controller_key.as_str())
            .ok_or_else(|| format!("location {:?}: unknown controller_key {:?}", l.key, controller_key))?;
        w.set_territory_controller(site, Some(controller))
            .map_err(|e| format!("location {:?}: {e}", l.key))?;
    }

    for f in &seed.factions {
        let owner = faction_ids[f.key.as_str()];
        let home = *locations
            .get(f.home_location_key.as_str())
            .ok_or_else(|| format!("faction {:?}: unknown home_location_key {:?}", f.key, f.home_location_key))?;

        let mut toe_slots = Vec::with_capacity(f.toe.len());
        for slot in &f.toe {
            let design = *designs
                .get(slot.design_key.as_str())
                .ok_or_else(|| format!("faction {:?} TO&E: unknown design_key {:?}", f.key, slot.design_key))?;
            toe_slots.push(ToeSlot { role: slot.role.clone(), design, count: slot.count });
        }
        let template: ToeTemplateId = w.define_toe_template(&format!("{} TO&E", f.name), &f.doctrine, toe_slots);

        for slot in &f.toe {
            let design = designs[slot.design_key.as_str()];
            for _ in 0..slot.count {
                w.seed_asset(
                    owner,
                    design,
                    home,
                    QualityGrade::Standard,
                    AssetOrigin::SeededHistorical { note: format!("{} pre-war garrison", f.name) },
                    None,
                )
                .map_err(|e| format!("faction {:?}: seeding {:?}: {e}", f.key, slot.design_key))?;
            }
        }

        w.try_assemble_formation(owner, template, &format!("{} Garrison", f.name), home)
            .map_err(|e| format!("faction {:?}: assembling garrison: {e:?}", f.key))?;
        w.set_faction_goal(owner, template, home).map_err(|e| format!("faction {:?}: {e}", f.key))?;
    }

    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_real_seed() -> WorldSeedFile {
        let data = include_str!("../assets/world_seed.json");
        serde_json::from_str(data).expect("assets/world_seed.json must parse against WorldSeedFile")
    }

    const EXPECTED_FACTIONS: &[&str] = &[
        "The Drone Remnant",
        "Axiom Consortium",
        "Athenian Republic",
        "Hanseatic League",
        "Pirate Baronies",
        "Techno-Primitives",
        "Independent Scavengers",
    ];

    #[test]
    fn real_world_seed_builds_a_consistent_seven_faction_world() {
        let file = load_real_seed();
        assert_eq!(file.factions.len(), 7, "expected exactly 7 factions in world_seed.json");

        let w = build_world(&file).expect("world_seed.json must build a valid World");

        let actor_names: Vec<String> = w.actors_iter().map(|a| a.name.clone()).collect();
        for expected in EXPECTED_FACTIONS {
            assert!(actor_names.iter().any(|n| n == expected), "missing faction actor {expected:?}");
        }

        // Every faction has a real, stationed garrison formation at its home
        // site, and that site's controller is that same faction.
        for f in &file.factions {
            let owner = w.actors_iter().find(|a| a.name == f.name).map(|a| a.id).expect("faction actor must exist");
            let formation = w
                .formations_iter()
                .find(|form| form.owner == owner)
                .unwrap_or_else(|| panic!("faction {:?} has no assembled formation", f.key));
            assert!(!formation.assets.is_empty(), "faction {:?} garrison has no real assets", f.key);

            let home = w.locations_iter().find(|l| l.name == homes_by_key(&file, &f.home_location_key)).unwrap();
            assert_eq!(home.controller, Some(owner), "faction {:?} does not control its own home site", f.key);
        }

        assert!(w.check_invariants().is_empty(), "seeded world must satisfy every hard invariant");
    }

    fn homes_by_key<'a>(file: &'a WorldSeedFile, key: &str) -> &'a str {
        &file.locations.iter().find(|l| l.key == key).expect("home_location_key must resolve").name
    }
}
