//! Runnable demo of the strategic sim: seeds a small two-faction world and
//! narrates the economy day by day.
//!
//!     cargo run --example demo
//!     cargo run --example demo -- <seed> <days>

use adona_sim::actors::ActorKind;
use adona_sim::assets::{AssetKind, AssetOrigin, ComponentCategory};
use adona_sim::events::EventKind;
use adona_sim::goods::{LegalStatus, LotOrigin, QualityGrade, UnitOfMeasure};
use adona_sim::locations::{CivilianNeed, LocationKind};
use adona_sim::production::{JobState, RecipeOutputs};
use adona_sim::toe::ToeSlot;
use adona_sim::{SimError, World};

fn main() -> Result<(), SimError> {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(42);
    let days: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(12);

    println!("=== ADONA strategic sim demo (seed {seed}, {days} days) ===\n");
    let mut w = World::new(seed);

    // --- Seed the world -------------------------------------------------
    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 1_000_000);
    let veyra = w.create_actor("Veyra Compact", ActorKind::Faction, 1_000_000);
    let authority = w.create_actor("Meridian City Authority", ActorKind::CityAuthority, 60_000);

    let meridian = w.create_location("Meridian", LocationKind::City, (0, 0));
    let mine = w.create_location("Redrock Mine", LocationKind::Mine, (10, 0));
    let forge = w.create_location("Forge Complex", LocationKind::FactorySite, (5, 5));

    let iron_ore = w.define_commodity("Iron Ore", UnitOfMeasure::Kilograms, 1, 2);
    let armor_plate = w.define_commodity("Armor Plate", UnitOfMeasure::Kilograms, 2, 40);
    let food = w.define_commodity("Food", UnitOfMeasure::Units, 1, 3);

    w.configure_city(
        meridian,
        10_000,
        Some(authority),
        vec![CivilianNeed { commodity: food, quantity_per_day: 500 }],
    )?;
    let market = w.create_market("Meridian Exchange", meridian, None)?;

    let truck_design =
        w.define_design("Hauler-6 Truck", AssetKind::Vehicle, vec![], Some(20_000), None)?;
    let mech_slots: Vec<_> = ["Leg Actuator", "Weapon Barrel", "Ammo Feed", "Reactor Feed", "Cooling Assembly"]
        .into_iter()
        .map(|name| {
            let def = w.define_component_def(name, 2, ComponentCategory::MechOrEquipment);
            adona_sim::assets::ComponentSlot { name: name.to_string(), accepts: vec![def] }
        })
        .collect();
    let talon_design = w.define_design("TLN-3 Talon", AssetKind::Mech, mech_slots, None, None)?;
    let tooling_design =
        w.define_design("Armor Plate Line", AssetKind::FactoryTooling, vec![], None, None)?;

    // Karth's industry: mine ore, haul it, roll armor plate.
    let ore = w.produce_from_mine(mine, karth, iron_ore, 10_000, QualityGrade::Standard)?;
    let truck = w.seed_asset(
        karth,
        truck_design,
        mine,
        QualityGrade::Standard,
        AssetOrigin::SeededHistorical { note: "pre-war logistics fleet".into() },
        Some("Old Reliable"),
    )?;
    let route = w.create_route(mine, forge, 2)?;
    let convoy = w.form_convoy(karth, mine, &[truck])?;
    w.load_lot_onto_convoy(convoy, ore)?;
    w.depart_convoy(convoy, route)?;

    let factory = w.create_factory(karth, forge, 1)?;
    let tooling = w.seed_asset(
        karth,
        tooling_design,
        forge,
        QualityGrade::Standard,
        AssetOrigin::SeededHistorical { note: "pre-war plant equipment".into() },
        None,
    )?;
    w.install_tooling(factory, tooling, 5_000, 0)?;
    for category in ComponentCategory::FACTORY_SLOTS {
        let def = w.define_component_def("Forge Complex sub-system", 1, category);
        let comp = w.seed_component(
            karth,
            def,
            forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war plant equipment".into() },
        )?;
        w.fit_factory_component(factory, comp)?;
    }
    let recipe = w.define_recipe(
        "Roll Armor Plate",
        vec![(iron_ore, 8_000)],
        RecipeOutputs::Commodity { commodity: armor_plate, quantity: 4_000 },
        3,
        Some(tooling_design),
    );

    // Food stores so the city has something to buy.
    let grain = w.seed_lot(
        karth,
        food,
        6_000,
        QualityGrade::Standard,
        LegalStatus::Legitimate,
        meridian,
        LotOrigin::SeededHistorical { note: "grain reserve".into() },
    )?;
    w.list_lot_for_sale(karth, market, grain, 3)?;

    // Veyra's military: two mechs, a lance that needs three.
    for i in 0..2 {
        w.seed_asset(
            veyra,
            talon_design,
            meridian,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: format!("pre-war mech {i}") },
            None,
        )?;
    }
    let lance = w.define_toe_template(
        "Talon Lance",
        "line-defense",
        vec![ToeSlot { role: "Line Mech".into(), design: talon_design, count: 3 }],
    );

    // --- Run ------------------------------------------------------------
    // Simple scripted "faction behavior" until real faction AI exists:
    // start production when the ore arrives, list output when made, have
    // Veyra bid for plate once it is on the market.
    let mut job = None;
    let mut listed_plate = false;
    let mut ordered_plate = false;

    for _ in 0..days {
        let before = w.events().len();
        w.tick();

        if job.is_none() {
            job = w.start_production(factory, recipe, &[ore]).ok();
        }
        if let (Some(j), false) = (job, listed_plate) {
            if let Some(JobState::Completed { output_lots, .. }) =
                w.production_job(j).map(|pj| pj.state.clone())
            {
                // Real hauling matters, but to keep the demo short the
                // factory sells at a factory-gate market instead of
                // convoying the plate to Meridian.
                let plate = output_lots[0];
                let gate = w.create_market("Forge Gate Market", forge, Some(karth))?;
                w.list_lot_for_sale(karth, gate, plate, 50)?;
                listed_plate = true;
            }
        }
        if listed_plate && !ordered_plate {
            w.place_buy_order(
                veyra,
                adona_sim::markets::OrderScope::Global,
                armor_plate,
                2_000,
                60,
            )?;
            ordered_plate = true;
        }

        // Narrate the day's important events.
        println!("--- Day {} ---", w.today());
        for e in &w.events()[before..] {
            match &e.kind {
                EventKind::DayAdvanced { .. } => {}
                EventKind::ConvoyArrived { convoy, at } => {
                    println!("  convoy {convoy} arrived at {}", name_of(&w, *at));
                }
                EventKind::ProductionStarted { job, .. } => {
                    println!("  production {job} started (consuming real ore)");
                }
                EventKind::ProductionCompleted { job, output_lots, .. } => {
                    println!("  production {job} completed: {} output lot(s)", output_lots.len());
                }
                EventKind::TradeExecuted { seller, buyer, quantity, price_per_unit, total, .. } => {
                    println!(
                        "  trade: {} -> {} | {} units @ {} = {} cr",
                        actor_name(&w, *seller),
                        actor_name(&w, *buyer),
                        quantity,
                        price_per_unit,
                        total
                    );
                }
                EventKind::BuyOrderPlaced { buyer, quantity, limit_price_per_unit, .. } => {
                    println!(
                        "  buy order: {} wants {} units (limit {})",
                        actor_name(&w, *buyer),
                        quantity,
                        limit_price_per_unit
                    );
                }
                _ => {}
            }
        }

        // Convoy management once arrived.
        if let Some(site) = w.convoy(convoy).and_then(|c| c.current_site()) {
            if !w.convoy(convoy).unwrap().cargo_lots.is_empty() {
                w.unload_lot(convoy, ore)?;
                println!("  cargo unloaded at {}", name_of(&w, site));
            }
        }
    }

    // --- Wrap up ----------------------------------------------------------
    println!("\n=== Day {} summary ===", w.today());
    for id in [karth, veyra, authority] {
        let a = w.actor(id).unwrap();
        println!("  {} treasury: {} cr", a.name, a.treasury);
    }
    match w.try_assemble_formation(veyra, lance, "First Lance", meridian) {
        Ok(f) => println!("  Veyra assembled formation {f}"),
        Err(SimError::ToeShortage(missing)) => {
            for m in missing {
                println!(
                    "  Veyra TO&E shortage: {} x{} — this is the demand signal for faction AI",
                    m.role, m.missing
                );
            }
        }
        Err(e) => println!("  TO&E error: {e}"),
    }
    let violations = w.check_invariants();
    println!(
        "  invariants: {}",
        if violations.is_empty() { "all hold".to_string() } else { format!("{violations:?}") }
    );
    println!("  state digest: {:#018x}", w.state_digest());
    println!("  events recorded: {}", w.events().len());
    Ok(())
}

fn name_of(w: &World, id: adona_sim::ids::LocationId) -> String {
    w.location(id).map(|l| l.name.clone()).unwrap_or_else(|| id.to_string())
}

fn actor_name(w: &World, id: adona_sim::ids::ActorId) -> String {
    w.actor(id).map(|a| a.name.clone()).unwrap_or_else(|| id.to_string())
}
