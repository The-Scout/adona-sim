//! Data-driven world seeding: the game's actual faction content (names,
//! doctrine, materials, TO&E) lives in `assets/world_seed.json`, not
//! hardcoded Rust, so it can be modded without recompiling. `adona-sim`
//! stays generic (see `CLAUDE.md`'s modularity rule) — this module is
//! `adona-game` glue that reads a schema-validated file and drives the
//! generic `World`/`generate_tiered_material_chain` builder API any other
//! consumer could also call with its own data.
//!
//! TODO(economy): factions can produce and replenish real mechs from their
//! own mines now, but there are still no markets/convoys/contracts wired
//! into the live simulation, so factions can't yet trade what they lack or
//! haul goods between territories. Tracked separately (GitHub issue #10).

use adona_sim::actors::{ActorKind, Credits};
use adona_sim::assets::{AssetKind, AssetOrigin, ComponentCategory, ComponentSlot};
use adona_sim::content::{TieredChainHandles, TieredChainSpec};
use adona_sim::goods::{QualityGrade, UnitOfMeasure};
use adona_sim::ids::{ActorId, CommodityId, DesignId, FactoryId, LocationId, ToeTemplateId};
use adona_sim::locations::{LocationKind, MineReserves};
use adona_sim::production::RecipeOutputs;
use adona_sim::toe::ToeSlot;
use adona_sim::World;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct WorldSeedFile {
    pub seed: u64,
    /// Shared tier-name ladder every material chain is built against (see
    /// `docs/design/Old/Tier System.md` for ADONA's own 13; a different
    /// project would supply its own list of any length here).
    pub tier_names: Vec<String>,
    /// The raw materials factions mine and refine. Must be exactly
    /// `adona_sim::assets::MECH_COMPONENT_SLOTS` long for the generated
    /// Universal Mech design below to validate — a pre-existing, deliberate
    /// engine rule about mechs specifically, not something this file works
    /// around.
    pub materials: Vec<MaterialSeed>,
    #[serde(default)]
    pub designs: Vec<DesignSeed>,
    pub locations: Vec<LocationSeed>,
    pub routes: Vec<RouteSeed>,
    pub factions: Vec<FactionSeed>,
    pub economy: EconomySeed,
}

#[derive(Debug, Deserialize)]
pub struct MaterialSeed {
    pub key: String,
    pub material_name: String,
    /// The Universal Mech slot this material's components fill (e.g. "Leg
    /// Actuator").
    pub component_role: String,
}

#[derive(Debug, Deserialize)]
pub struct SlotSeed {
    pub name: String,
    pub component_def_key: String,
}

/// A hand-authored, non-tiered design (e.g. a vehicle) that doesn't need
/// generated component slots.
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
    /// If set, this mine also auto-yields the named material's raw (tier 1)
    /// commodity every day — used for the Cradle of Conflict's bonus
    /// Ruthenium supply on top of every faction's own small mine of it.
    #[serde(default)]
    pub yields_material_key: Option<String>,
    #[serde(default)]
    pub yield_quantity_per_day: Option<u64>,
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
    /// Either a key from `designs`, or the literal `"universal_mech"` for
    /// the generated, shared mech design every faction assembles from its
    /// own mined-and-refined materials.
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
    pub population: u64,
    pub tax_rate_per_capita: Credits,
    pub toe: Vec<ToeSlotSeed>,
}

/// Tuning shared by every faction's generated economy — the *shape* (5
/// small mines, a couple of refineries, one assembly line) is generic code
/// below; these are just the numbers, still data so a modder can retune
/// them without recompiling.
#[derive(Debug, Deserialize)]
pub struct EconomySeed {
    pub mine_starting_reserve: u64,
    pub mine_yield_per_day: u64,
    pub refineries_per_faction: u32,
    pub refine_input_qty: u64,
    pub refine_output_qty: u64,
    pub refine_duration_days: u64,
    pub convert_input_qty: u64,
    pub convert_duration_days: u64,
    pub assembly_duration_days: u64,
}

/// One material's generated chain paired with the data that spawned it.
struct MaterialChain<'a> {
    seed: &'a MaterialSeed,
    handles: TieredChainHandles,
}

/// Build a real `World` from a parsed seed file: every actor, location,
/// design, component, factory, mine, and TO&E goal created here goes
/// through the same public builder API (`adona_sim::World` and
/// `generate_tiered_material_chain`) a hand-written scenario or a different
/// project entirely would use — nothing is spawned outside it.
pub fn build_world(seed: &WorldSeedFile) -> Result<World, String> {
    let mut w = World::new(seed.seed);

    // --- Material chains: generic engine machinery, ADONA-specific data ---
    let material_chains: Vec<MaterialChain> = seed
        .materials
        .iter()
        .map(|m| {
            let handles = w.generate_tiered_material_chain(&TieredChainSpec {
                material_name: m.material_name.clone(),
                tier_names: seed.tier_names.clone(),
                unit: UnitOfMeasure::Kilograms,
                base_price: 10,
                component_category: ComponentCategory::MechOrEquipment,
                refine_input_qty: seed.economy.refine_input_qty,
                refine_output_qty: seed.economy.refine_output_qty,
                refine_duration_days: seed.economy.refine_duration_days,
                convert_input_qty: seed.economy.convert_input_qty,
                convert_duration_days: seed.economy.convert_duration_days,
            });
            MaterialChain { seed: m, handles }
        })
        .collect();
    let material_by_key: HashMap<&str, &MaterialChain> =
        material_chains.iter().map(|c| (c.seed.key.as_str(), c)).collect();

    // --- The Universal Mech: one design, assembled from one component per
    // material, any tier accepted --------------------------------------
    let mech_slots: Vec<ComponentSlot> = material_chains
        .iter()
        .map(|c| ComponentSlot { name: c.seed.component_role.clone(), accepts: c.handles.component_defs.clone() })
        .collect();
    let universal_mech_design = w
        .define_design("Universal Mech", AssetKind::Mech, mech_slots, None, None)
        .map_err(|e| format!("universal mech design: {e}"))?;
    let assembly_tooling_design = w
        .define_design("Universal Mech Assembly Line", AssetKind::FactoryTooling, vec![], None, Some(universal_mech_design))
        .map_err(|e| format!("universal mech assembly tooling design: {e}"))?;
    let assembly_requirements = material_chains.iter().map(|c| c.handles.any_tier_requirement(1)).collect();
    let assembly_recipe = w.define_recipe(
        "Assemble Universal Mech",
        vec![],
        assembly_requirements,
        RecipeOutputs::SerialAssets { design: universal_mech_design, count: 1 },
        seed.economy.assembly_duration_days,
        Some(assembly_tooling_design),
    );

    // --- Hand-authored simple designs (vehicles etc.) -------------------
    let mut designs: HashMap<&str, DesignId> = HashMap::new();
    designs.insert("universal_mech", universal_mech_design);
    for d in &seed.designs {
        let mut slots = Vec::with_capacity(d.slots.len());
        for slot in &d.slots {
            // Non-tiered designs may still reference a material's full tier
            // range by material key, for parts that come in any grade.
            let def_id = material_by_key
                .get(slot.component_def_key.as_str())
                .and_then(|c| c.handles.component_defs.first().copied())
                .ok_or_else(|| format!("design {:?} slot {:?}: unknown component_def_key {:?}", d.key, slot.name, slot.component_def_key))?;
            slots.push(ComponentSlot { name: slot.name.clone(), accepts: vec![def_id] });
        }
        let id = w.define_design(&d.name, d.kind, slots, d.cargo_capacity_kg, None).map_err(|e| format!("design {:?}: {e}", d.key))?;
        designs.insert(&d.key, id);
    }

    // --- Locations, routes -----------------------------------------------
    let mut locations: HashMap<&str, LocationId> = HashMap::new();
    for l in &seed.locations {
        let id = w.create_location(&l.name, l.kind, l.position);
        if let Some(reserves) = l.mine_reserves {
            w.configure_mine(id, reserves).map_err(|e| format!("location {:?}: {e}", l.key))?;
        }
        if let Some(material_key) = &l.yields_material_key {
            let commodity = raw_commodity_for(&material_by_key, material_key, &l.key)?;
            let qty = l.yield_quantity_per_day.unwrap_or(0);
            w.add_location_yield(id, commodity, qty, MineReserves::Infinite)
                .map_err(|e| format!("location {:?}: {e}", l.key))?;
        }
        locations.insert(&l.key, id);
    }
    for r in &seed.routes {
        let from = *locations.get(r.from_key.as_str()).ok_or_else(|| format!("route: unknown from_key {:?}", r.from_key))?;
        let to = *locations.get(r.to_key.as_str()).ok_or_else(|| format!("route: unknown to_key {:?}", r.to_key))?;
        w.create_route(from, to, r.distance_days).map_err(|e| format!("route {:?}->{:?}: {e}", r.from_key, r.to_key))?;
    }

    // --- Factions ----------------------------------------------------------
    let mut faction_ids: HashMap<&str, ActorId> = HashMap::new();
    for f in &seed.factions {
        faction_ids.insert(&f.key, w.create_actor(&f.name, f.kind, f.starting_treasury));
    }

    // Controllers deferred until faction ids exist.
    for l in &seed.locations {
        let Some(controller_key) = &l.controller_key else { continue };
        let site = locations[l.key.as_str()];
        let controller = *faction_ids
            .get(controller_key.as_str())
            .ok_or_else(|| format!("location {:?}: unknown controller_key {:?}", l.key, controller_key))?;
        w.set_territory_controller(site, Some(controller)).map_err(|e| format!("location {:?}: {e}", l.key))?;
    }

    for f in &seed.factions {
        let owner = faction_ids[f.key.as_str()];
        let home = *locations
            .get(f.home_location_key.as_str())
            .ok_or_else(|| format!("faction {:?}: unknown home_location_key {:?}", f.key, f.home_location_key))?;

        // Population + tax (issue #7): a real, if simple, city economy so
        // treasuries and population actually move tick over tick. A real
        // civilian-goods economy is future work (see module doc TODO).
        w.configure_city(home, f.population, Some(owner), vec![]).map_err(|e| format!("faction {:?}: {e}", f.key))?;
        w.set_tax_rate(home, f.tax_rate_per_capita).map_err(|e| format!("faction {:?}: {e}", f.key))?;

        // Self-sufficient economy: one independent yield line per material,
        // right at home, so the faction's own factories (also at home) can
        // actually draw on it — no convoy/logistics system exists yet to
        // haul goods in from a separate mine site (tracked separately,
        // GitHub issue #10's remaining scope).
        for chain in &material_chains {
            w.add_location_yield(
                home,
                chain.handles.commodities[0],
                seed.economy.mine_yield_per_day,
                MineReserves::Finite { remaining: seed.economy.mine_starting_reserve },
            )
            .map_err(|e| format!("faction {:?} yield: {e}", f.key))?;
        }

        // A couple of untooled refineries (any material, whatever's ready)
        // plus one assembly line tooled for the Universal Mech.
        for _ in 0..seed.economy.refineries_per_faction {
            seed_operational_factory(&mut w, owner, home, &f.name)?;
        }
        let assembly_factory = seed_operational_factory(&mut w, owner, home, &f.name)?;
        let assembly_tooling = w
            .seed_asset(
                owner,
                assembly_tooling_design,
                home,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: format!("{} pre-war assembly line", f.name) },
                None,
            )
            .map_err(|e| format!("faction {:?} assembly tooling: {e}", f.key))?;
        w.install_tooling(assembly_factory, assembly_tooling, 0, 0)
            .map_err(|e| format!("faction {:?} assembly tooling: {e}", f.key))?;
        let _ = assembly_recipe; // available to every faction's factories generically via tick_factory_auto_production

        // TO&E: a standing goal against the combined template (so
        // production replenishes toward the faction's full desired
        // strength), but one *separate* starting formation per role rather
        // than one formation combining everything. A single combined
        // formation gives the deployment AI's "always keep one garrison
        // behind" rule (see `tick_faction_deployment` in
        // `crates/adona-sim/src/faction_ai.rs`) nothing to ever send
        // anywhere — every faction would sit at home forever, never
        // marching and never fighting.
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

            let role_template = w.define_toe_template(
                &format!("{} {} Squad", f.name, slot.role),
                &f.doctrine,
                vec![ToeSlot { role: slot.role.clone(), design, count: slot.count }],
            );
            w.try_assemble_formation(owner, role_template, &format!("{} {}", f.name, slot.role), home)
                .map_err(|e| format!("faction {:?}: assembling {:?}: {e:?}", f.key, slot.role))?;
        }

        w.set_faction_goal(owner, template, home).map_err(|e| format!("faction {:?}: {e}", f.key))?;
    }

    Ok(w)
}


fn raw_commodity_for(
    material_by_key: &HashMap<&str, &MaterialChain>,
    material_key: &str,
    context_key: &str,
) -> Result<CommodityId, String> {
    material_by_key
        .get(material_key)
        .and_then(|c| c.handles.commodities.first().copied())
        .ok_or_else(|| format!("location {context_key:?}: unknown yields_material_key {material_key:?}"))
}

/// A factory at `site` for `owner` with all five fixed sub-system
/// components fitted (labor, assembly line, power, control, QA) — the same
/// "operational from day one" pattern the engine's own tests use, generic
/// over any owner/site.
fn seed_operational_factory(w: &mut World, owner: ActorId, site: LocationId, label: &str) -> Result<FactoryId, String> {
    let factory = w.create_factory(owner, site, 1).map_err(|e| format!("{label} factory: {e}"))?;
    for category in ComponentCategory::FACTORY_SLOTS {
        let def = w.define_component_def(&format!("{label} Factory Sub-System"), 1, category);
        let comp = w
            .seed_component(
                owner,
                def,
                site,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: format!("{label} pre-war plant equipment") },
            )
            .map_err(|e| format!("{label} factory: {e}"))?;
        w.fit_factory_component(factory, comp).map_err(|e| format!("{label} factory: {e}"))?;
    }
    Ok(factory)
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
    fn real_world_seed_builds_a_consistent_seven_faction_economy() {
        let file = load_real_seed();
        assert_eq!(file.factions.len(), 7, "expected exactly 7 factions in world_seed.json");
        assert_eq!(file.materials.len(), 5, "the Universal Mech needs exactly 5 materials/slots");

        let w = build_world(&file).expect("world_seed.json must build a valid World");

        let actor_names: Vec<String> = w.actors_iter().map(|a| a.name.clone()).collect();
        for expected in EXPECTED_FACTIONS {
            assert!(actor_names.iter().any(|n| n == expected), "missing faction actor {expected:?}");
        }

        for f in &file.factions {
            let owner = w.actors_iter().find(|a| a.name == f.name).map(|a| a.id).expect("faction actor must exist");

            // One formation per TO&E role, never one combined formation —
            // a single combined formation leaves the deployment AI's
            // "always keep one garrison behind" rule nothing to ever send
            // out (see CLAUDE.md and crates/adona-sim/src/faction_ai.rs).
            let faction_formations: Vec<_> = w.formations_iter().filter(|form| form.owner == owner).collect();
            assert_eq!(
                faction_formations.len(),
                f.toe.len(),
                "faction {:?} must start with one formation per TO&E role, not one combined formation",
                f.key
            );
            for formation in &faction_formations {
                assert!(!formation.assets.is_empty(), "faction {:?} formation {:?} has no real assets", f.key, formation.id);
            }

            let home = w.locations_iter().find(|l| l.name == homes_by_key(&file, &f.home_location_key)).unwrap();
            assert_eq!(home.controller, Some(owner), "faction {:?} does not control its own home site", f.key);
            assert!(home.population > 0, "faction {:?} home site has no population", f.key);
            assert_eq!(
                home.tax_rate_per_capita, f.tax_rate_per_capita,
                "faction {:?} home site tax rate must match the seed data (0 is valid for factions like the Drone Remnant with no economy)",
                f.key
            );
            // At least one self-sufficient yield per material; the Drone
            // Remnant's home is the Cradle of Conflict itself, which also
            // carries its own bonus Ruthenium yield on top of the standard
            // per-material set, so it legitimately has one more.
            assert!(
                home.yields.len() >= file.materials.len(),
                "faction {:?} home site must have at least one self-sufficient yield per material",
                f.key
            );

            let owned_factories = w.factories_iter().filter(|fac| fac.owner == owner).count();
            assert_eq!(owned_factories, 3, "faction {:?} must have 2 refineries + 1 assembly factory", f.key);
            for factory in w.factories_iter().filter(|fac| fac.owner == owner) {
                assert!(factory.is_operational(), "faction {:?} factory {:?} is not operational", f.key, factory.id);
            }
        }

        assert!(w.check_invariants().is_empty(), "seeded world must satisfy every hard invariant");
    }

    /// End-to-end proof the bottom-up chain actually produces a mech with
    /// no manual intervention: mines auto-yield, factories auto-refine and
    /// auto-assemble, over enough simulated days.
    #[test]
    fn seeded_economy_eventually_manufactures_a_real_mech() {
        let file = load_real_seed();
        let mut w = build_world(&file).expect("world_seed.json must build a valid World");

        for _ in 0..400 {
            w.tick();
        }

        let manufactured = w
            .assets_iter()
            .any(|a| matches!(a.origin, adona_sim::assets::AssetOrigin::Manufactured { .. }));
        assert!(manufactured, "no faction manufactured a real asset from its own economy within 400 days");
        assert!(w.check_invariants().is_empty());
    }

    /// Regression test for the reported bug: garrisons must actually march
    /// off their home site and fight, not sit forever. Splitting the
    /// starting assembly into one formation per TO&E role (rather than one
    /// combined formation) is what gives the deployment AI's "keep one
    /// garrison behind" rule real surplus to send toward the contested
    /// Cradle of Conflict.
    #[test]
    fn seeded_factions_eventually_march_and_fight_each_other() {
        let file = load_real_seed();
        let mut w = build_world(&file).expect("world_seed.json must build a valid World");

        let mut marched = false;
        let mut fought = false;
        for _ in 0..120 {
            let before = w.events().len();
            w.tick();
            for e in &w.events()[before..] {
                match &e.kind {
                    adona_sim::events::EventKind::FormationMarchOrdered { .. } => marched = true,
                    adona_sim::events::EventKind::BattleResolved { .. } => fought = true,
                    _ => {}
                }
            }
            if marched && fought {
                break;
            }
        }

        assert!(marched, "no formation ever marched off its home site within 120 days");
        assert!(fought, "no battle was ever resolved within 120 days");
        assert!(w.check_invariants().is_empty());
    }

    fn homes_by_key<'a>(file: &'a WorldSeedFile, key: &str) -> &'a str {
        &file.locations.iter().find(|l| l.key == key).expect("home_location_key must resolve").name
    }
}
