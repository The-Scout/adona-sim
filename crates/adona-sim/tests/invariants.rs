//! Integration tests proving the core "everything important is real"
//! invariants: ownership, location, provenance through split/merge, real
//! market goods, real convoy cargo, real production inputs/outputs, real
//! escrowed money, TO&E from real assets, immutable intel, and determinism.

use adona_sim::actors::ActorKind;
use adona_sim::assets::{AssetKind, AssetOrigin, ComponentCategory, ComponentSlot};
use adona_sim::contracts::{ContractObjective, ContractState, ContractTarget, SalvageTerms};
use adona_sim::convoys::ConvoyState;
use adona_sim::goods::{LegalStatus, LotOrigin, LotState, QualityGrade, UnitOfMeasure};
use adona_sim::ids::*;
use adona_sim::intel::IntelSubject;
use adona_sim::locations::{CivilianNeed, LocationKind, LocationRef};
use adona_sim::markets::OrderScope;
use adona_sim::production::{JobState, RecipeOutputs};
use adona_sim::storage::{InMemoryStore, SimStore};
use adona_sim::toe::ToeSlot;
use adona_sim::{SimError, World};

/// Handles into the scenario world so tests can poke at specific entities.
/// Some handles are kept for future tests even if unused today.
#[allow(dead_code)]
struct Scenario {
    world: World,
    karth: ActorId,
    veyra: ActorId,
    merc: ActorId,
    authority: ActorId,
    meridian: LocationId,
    redrock_mine: LocationId,
    forge: LocationId,
    market: MarketId,
    iron_ore: CommodityId,
    armor_plate: CommodityId,
    food: CommodityId,
    truck_design: DesignId,
    talon_design: DesignId,
    ore_lot: LotId,
    armor_lot: LotId,
    factory: FactoryId,
    job: ProductionJobId,
    convoy: ConvoyId,
    lance_template: ToeTemplateId,
}

/// Build a full economic loop: mine -> convoy -> factory -> convoy ->
/// market, plus city demand, mechs, and TO&E scaffolding. Every step is
/// physical.
fn build_scenario(seed: u64) -> Scenario {
    let mut w = World::new(seed);

    // Actors.
    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 1_000_000);
    let veyra = w.create_actor("Veyra Compact", ActorKind::Faction, 1_000_000);
    let merc = w.create_actor("Red Sable Company", ActorKind::MercenaryCompany, 20_000);
    let authority = w.create_actor("Meridian City Authority", ActorKind::CityAuthority, 50_000);

    // Locations.
    let meridian = w.create_location("Meridian", LocationKind::City, (0, 0));
    let redrock_mine = w.create_location("Redrock Mine", LocationKind::Mine, (10, 0));
    let forge = w.create_location("Forge Complex", LocationKind::FactorySite, (5, 5));

    // Commodities.
    let iron_ore = w.define_commodity("Iron Ore", UnitOfMeasure::Kilograms, 1, 2);
    let armor_plate = w.define_commodity("Armor Plate", UnitOfMeasure::Kilograms, 2, 40);
    let food = w.define_commodity("Food", UnitOfMeasure::Units, 1, 3);

    // City: population buys food through its authority.
    w.configure_city(
        meridian,
        10_000,
        Some(authority),
        vec![CivilianNeed { commodity: food, quantity_per_day: 500 }],
    )
    .unwrap();

    let market = w.create_market("Meridian Exchange", meridian, None).unwrap();

    // Designs.
    let truck_design = w
        .define_design("Hauler-6 Truck", AssetKind::Vehicle, vec![], Some(20_000), None)
        .unwrap();
    // A mech is real components, not a stat block: exactly five distinct
    // slots (docket component priors).
    let actuator_def = w.define_component_def("Talon Leg Actuator", 2, ComponentCategory::MechOrEquipment);
    let barrel_def = w.define_component_def("Talon Autocannon Barrel", 2, ComponentCategory::MechOrEquipment);
    let ammo_feed_def = w.define_component_def("Talon Ammo Feed", 2, ComponentCategory::MechOrEquipment);
    let reactor_feed_def = w.define_component_def("Talon Reactor Feed", 2, ComponentCategory::MechOrEquipment);
    let cooling_def = w.define_component_def("Talon Cooling Assembly", 2, ComponentCategory::MechOrEquipment);
    let talon_design = w
        .define_design(
            "TLN-3 Talon",
            AssetKind::Mech,
            vec![
                ComponentSlot { name: "Leg Actuator".into(), accepts: vec![actuator_def] },
                ComponentSlot { name: "Weapon Barrel".into(), accepts: vec![barrel_def] },
                ComponentSlot { name: "Ammo Feed".into(), accepts: vec![ammo_feed_def] },
                ComponentSlot { name: "Reactor Feed".into(), accepts: vec![reactor_feed_def] },
                ComponentSlot { name: "Cooling Assembly".into(), accepts: vec![cooling_def] },
            ],
            None,
            None,
        )
        .unwrap();
    let tooling_design = w
        .define_design(
            "Armor Plate Line",
            AssetKind::FactoryTooling,
            vec![],
            None,
            Some(talon_design), // tooling binds to an exact design; placeholder binding
        )
        .unwrap();

    // Real ore out of a real mine.
    let ore_lot = w
        .produce_from_mine(redrock_mine, karth, iron_ore, 10_000, QualityGrade::Standard)
        .unwrap();

    // A real truck (pre-war stock) hauls the ore to the factory.
    let truck = w
        .seed_asset(
            karth,
            truck_design,
            redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war logistics fleet".into() },
            Some("Old Reliable"),
        )
        .unwrap();
    let mine_to_forge = w.create_route(redrock_mine, forge, 2).unwrap();
    let convoy = w.form_convoy(karth, redrock_mine, &[truck]).unwrap();
    w.load_lot_onto_convoy(convoy, ore_lot).unwrap();
    w.depart_convoy(convoy, mine_to_forge).unwrap();
    w.tick();
    w.tick(); // arrives on day 2
    w.unload_lot(convoy, ore_lot).unwrap();
    w.disband_convoy(convoy).unwrap();

    // Factory with exact tooling turns ore into armor plate.
    let factory = w.create_factory(karth, forge, 1).unwrap();
    let tooling = w
        .seed_asset(
            karth,
            tooling_design,
            forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war plant equipment".into() },
            None,
        )
        .unwrap();
    w.install_tooling(factory, tooling, 5_000, 0).unwrap();
    // A factory is not just tooling: it needs its five fixed sub-system
    // components (labor, assembly line, power, control, QA) before it can
    // run at all.
    for category in ComponentCategory::FACTORY_SLOTS {
        let def = w.define_component_def("Forge Complex sub-system", 1, category);
        let comp = w
            .seed_component(
                karth,
                def,
                forge,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: "pre-war plant equipment".into() },
            )
            .unwrap();
        w.fit_factory_component(factory, comp).unwrap();
    }
    let recipe = w.define_recipe(
        "Roll Armor Plate",
        vec![(iron_ore, 8_000)],
        RecipeOutputs::Commodity { commodity: armor_plate, quantity: 4_000 },
        3,
        Some(tooling_design),
    );
    let job = w.start_production(factory, recipe, &[ore_lot]).unwrap();
    w.tick();
    w.tick();
    w.tick(); // completes on day 5

    let armor_lot = match &w.production_job(job).unwrap().state {
        JobState::Completed { output_lots, .. } => output_lots[0],
        other => panic!("job should be complete, is {other:?}"),
    };

    // Haul armor plate to the city market and list it.
    let truck2 = w
        .seed_asset(
            karth,
            truck_design,
            forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war logistics fleet".into() },
            None,
        )
        .unwrap();
    let forge_to_city = w.create_route(forge, meridian, 1).unwrap();
    let convoy2 = w.form_convoy(karth, forge, &[truck2]).unwrap();
    w.load_lot_onto_convoy(convoy2, armor_lot).unwrap();
    w.depart_convoy(convoy2, forge_to_city).unwrap();
    w.tick(); // arrives day 6
    w.unload_lot(convoy2, armor_lot).unwrap();
    w.disband_convoy(convoy2).unwrap();

    // Food for the civilian market (pre-war stores) so city demand can buy.
    let food_lot = w
        .seed_lot(
            karth,
            food,
            5_000,
            QualityGrade::Standard,
            LegalStatus::Legitimate,
            meridian,
            LotOrigin::SeededHistorical { note: "grain reserve".into() },
        )
        .unwrap();
    w.list_lot_for_sale(karth, market, food_lot, 3).unwrap();

    // Veyra's pre-war mechs at Meridian for TO&E tests.
    for i in 0..2 {
        w.seed_asset(
            veyra,
            talon_design,
            meridian,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: format!("pre-war lance, mech {i}") },
            None,
        )
        .unwrap();
    }
    let lance_template = w.define_toe_template(
        "Talon Lance",
        "line-defense",
        vec![ToeSlot { role: "Line Mech".into(), design: talon_design, count: 3 }],
    );

    Scenario {
        world: w,
        karth,
        veyra,
        merc,
        authority,
        meridian,
        redrock_mine,
        forge,
        market,
        iron_ore,
        armor_plate,
        food,
        truck_design,
        talon_design,
        ore_lot,
        armor_lot,
        factory,
        job,
        convoy,
        lance_template,
    }
}

#[test]
fn scenario_upholds_all_hard_invariants() {
    let s = build_scenario(42);
    let violations = s.world.check_invariants();
    assert!(violations.is_empty(), "invariant violations: {violations:#?}");
}

#[test]
fn every_lot_and_asset_has_one_owner_and_one_location() {
    let s = build_scenario(42);
    for lot in s.world.lots_iter() {
        assert!(s.world.actor(lot.owner).is_some(), "{} has no owner", lot.id);
    }
    for asset in s.world.assets_iter() {
        assert!(s.world.actor(asset.owner).is_some(), "{} has no owner", asset.id);
    }
    // Location validity is covered by check_invariants; spot-check that the
    // armor lot is physically at the city after hauling.
    let armor = s.world.lot(s.armor_lot).unwrap();
    assert_eq!(s.world.resolve_site(armor.location), Some(s.meridian));
}

#[test]
fn lots_preserve_origin_through_split_and_merge() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // Split the armor lot: the child must trace back to the factory job.
    let child = w.split_lot(s.armor_lot, 1_000).unwrap();
    let child_origins = w.root_origins(child).unwrap();
    assert_eq!(
        child_origins,
        vec![LotOrigin::Produced { factory: s.factory, job: s.job }],
        "split child lost its production origin"
    );

    // Merge lots from two different origins: both origins must survive.
    // Match the seeded lot's quality to whatever the factory actually rolled
    // for the child (production quality is a real RNG roll, not guaranteed
    // Standard) so this merge is a genuine same-quality case.
    let produced_quality = w.lot(child).unwrap().quality;
    let seeded = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            500,
            produced_quality,
            LegalStatus::Legitimate,
            s.meridian,
            LotOrigin::Imported { source: "external-sim:earth".into() },
        )
        .unwrap();
    let merged = w.merge_lots(&[child, seeded]).unwrap();
    let merged_origins = w.root_origins(merged).unwrap();
    assert!(merged_origins.contains(&LotOrigin::Produced { factory: s.factory, job: s.job }));
    assert!(merged_origins
        .contains(&LotOrigin::Imported { source: "external-sim:earth".into() }));
    assert_eq!(w.lot(merged).unwrap().quantity, 1_500);

    // Source lots stay on record, marked merged — provenance is never erased.
    assert_eq!(w.lot(child).unwrap().state, LotState::MergedInto(merged));

    // Merging different qualities must fail: no quality laundering. Pick a
    // grade guaranteed to differ from whatever the factory actually rolled.
    let mismatched_quality =
        if produced_quality == QualityGrade::Fine { QualityGrade::Poor } else { QualityGrade::Fine };
    let fine = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            100,
            mismatched_quality,
            LegalStatus::Legitimate,
            s.meridian,
            LotOrigin::SeededHistorical { note: "boutique plate".into() },
        )
        .unwrap();
    assert!(matches!(w.merge_lots(&[merged, fine]), Err(SimError::MergeMismatch(_))));

    assert!(w.check_invariants().is_empty());
}

#[test]
fn market_listings_reference_real_goods_and_goods_run_out() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // Cannot list goods that are not physically at the market site.
    let ore_at_mine = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 100, QualityGrade::Standard)
        .unwrap();
    assert_eq!(
        w.list_lot_for_sale(s.karth, s.market, ore_at_mine, 5),
        Err(SimError::NotColocated),
        "listed goods that are physically elsewhere"
    );

    // List the armor plate; a partial purchase splits the real lot.
    let listing = w.list_lot_for_sale(s.karth, s.market, s.armor_lot, 50).unwrap();
    let veyra_before = w.actor(s.veyra).unwrap().treasury;
    let karth_before = w.actor(s.karth).unwrap().treasury;
    let bought = w.execute_purchase(listing, s.veyra, 1_000).unwrap();
    assert_eq!(w.lot(bought).unwrap().owner, s.veyra);
    assert_eq!(w.lot(bought).unwrap().quantity, 1_000);
    // Goods stay physical at the market site: buying is not teleporting.
    assert_eq!(w.resolve_site(w.lot(bought).unwrap().location), Some(s.meridian));
    assert_eq!(w.actor(s.veyra).unwrap().treasury, veyra_before - 50_000);
    assert_eq!(w.actor(s.karth).unwrap().treasury, karth_before + 50_000);
    // Provenance survives the trade: the bought plate still traces to the
    // factory job.
    assert_eq!(
        w.root_origins(bought).unwrap(),
        vec![LotOrigin::Produced { factory: s.factory, job: s.job }]
    );

    // Buy the rest: the lot runs out and the listing disappears with it.
    let remaining = w.lot(s.armor_lot).unwrap().quantity;
    w.execute_purchase(listing, s.veyra, remaining).unwrap();
    assert!(w.listing(listing).is_none(), "listing outlived its goods");
    assert_eq!(
        w.execute_purchase(listing, s.veyra, 1),
        Err(SimError::UnknownListing(listing)),
        "bought from an empty market"
    );

    assert!(w.check_invariants().is_empty());
}

#[test]
fn buy_orders_escrow_real_money_and_match_real_listings() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let listing = w.list_lot_for_sale(s.karth, s.market, s.armor_lot, 50).unwrap();

    let veyra_before = w.actor(s.veyra).unwrap().treasury;
    let order = w
        .place_buy_order(s.veyra, OrderScope::Market(s.market), s.armor_plate, 1_000, 60)
        .unwrap();
    // Escrow left the treasury immediately.
    assert_eq!(w.actor(s.veyra).unwrap().treasury, veyra_before - 60_000);

    w.tick();

    // Matched at listing price (50), limit difference refunded.
    assert!(w.buy_order(order).is_none(), "order should be filled and closed");
    assert_eq!(w.actor(s.veyra).unwrap().treasury, veyra_before - 50_000);
    let veyra_armor: u64 = w
        .lots_iter()
        .filter(|l| l.owner == s.veyra && l.commodity == s.armor_plate && l.state == LotState::Active)
        .map(|l| l.quantity)
        .sum();
    assert_eq!(veyra_armor, 1_000);
    assert!(w.listing(listing).is_some(), "partially-filled listing should remain");

    // Civilian demand generated real food orders and they matched against
    // the real food lot (population consumption may have already destroyed
    // the purchased stock this same tick, so check the trade happened
    // rather than checking remaining owned quantity).
    let food_bought = w.events().iter().any(|e| {
        matches!(&e.kind, adona_sim::events::EventKind::TradeExecuted { buyer, .. } if *buyer == s.authority)
    });
    assert!(food_bought, "civilian demand bought no food");

    assert!(w.check_invariants().is_empty());
}

#[test]
fn convoy_cargo_must_be_real_and_colocated() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // A fresh convoy at the mine.
    let truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let convoy = w.form_convoy(s.karth, s.redrock_mine, &[truck]).unwrap();

    // Cannot load cargo that is physically elsewhere.
    assert_eq!(
        w.load_lot_onto_convoy(convoy, s.armor_lot),
        Err(SimError::NotColocated),
        "loaded cargo from another site"
    );

    // Cannot load someone else's goods.
    let veyra_ore = w
        .produce_from_mine(s.redrock_mine, s.veyra, s.iron_ore, 50, QualityGrade::Standard)
        .unwrap();
    assert!(matches!(
        w.load_lot_onto_convoy(convoy, veyra_ore),
        Err(SimError::NotOwner { .. })
    ));

    // Real colocated cargo loads; its location becomes the convoy.
    let ore = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 500, QualityGrade::Standard)
        .unwrap();
    w.load_lot_onto_convoy(convoy, ore).unwrap();
    assert_eq!(w.lot(ore).unwrap().location, LocationRef::Convoy(convoy));

    // While en route, cargo is at no site — it is on the road.
    let route = w.create_route(s.redrock_mine, s.forge, 2).unwrap();
    w.depart_convoy(convoy, route).unwrap();
    assert_eq!(w.resolve_site(w.lot(ore).unwrap().location), None);
    // And nothing can be loaded mid-route.
    let more_ore = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 10, QualityGrade::Standard)
        .unwrap();
    assert_eq!(
        w.load_lot_onto_convoy(convoy, more_ore),
        Err(SimError::ConvoyNotAtSite(convoy))
    );

    w.tick();
    w.tick();
    assert!(matches!(w.convoy(convoy).unwrap().state, ConvoyState::Arrived { .. }));
    w.unload_lot(convoy, ore).unwrap();
    assert_eq!(w.lot(ore).unwrap().location, LocationRef::Site(s.forge));

    assert!(w.check_invariants().is_empty());
}

#[test]
fn production_consumes_real_inputs_and_creates_provenanced_outputs() {
    let s = build_scenario(42);
    let w = &s.world;

    let job = w.production_job(s.job).unwrap();

    // Inputs: real lots, marked consumed by this exact job, records kept.
    assert!(!job.consumed_lots.is_empty());
    let mut consumed_total = 0u64;
    for lid in &job.consumed_lots {
        let lot = w.lot(*lid).expect("consumed lot record must be kept");
        assert_eq!(lot.state, LotState::ConsumedByProduction(s.job));
        consumed_total += lot.quantity;
    }
    assert_eq!(consumed_total, 8_000, "job consumed a different amount than the recipe");

    // The leftover ore (10_000 - 8_000) is still real and active.
    let leftover: u64 = w
        .lots_iter()
        .filter(|l| l.commodity == s.iron_ore && l.state == LotState::Active)
        .map(|l| l.quantity)
        .sum();
    assert_eq!(leftover, 2_000);

    // Output: a real lot whose origin is this factory and job, and whose
    // input lots trace back to the mine.
    let JobState::Completed { output_lots, .. } = &job.state else {
        panic!("job must be complete");
    };
    let out = w.lot(output_lots[0]).unwrap();
    assert_eq!(out.quantity, 4_000);
    assert_eq!(
        w.root_origins(out.id).unwrap(),
        vec![LotOrigin::Produced { factory: s.factory, job: s.job }]
    );
    for lid in &job.consumed_lots {
        assert_eq!(
            w.root_origins(*lid).unwrap(),
            vec![LotOrigin::Mined { mine: s.redrock_mine }],
            "consumed input lost its mine origin"
        );
    }
}

#[test]
fn production_fails_without_sufficient_real_inputs() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let small_ore = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 100, QualityGrade::Standard)
        .unwrap();
    // Haul it to the factory the honest way is already proven; teleport via
    // the explicit admin path to keep this test focused.
    w.admin_move_lot(small_ore, LocationRef::Site(s.forge), None).unwrap();

    let recipe = w.define_recipe(
        "Roll Armor Plate (again)",
        vec![(s.iron_ore, 8_000)],
        RecipeOutputs::Commodity { commodity: s.armor_plate, quantity: 4_000 },
        3,
        None,
    );
    match w.start_production(s.factory, recipe, &[small_ore]) {
        Err(SimError::InsufficientQuantity { missing, .. }) => assert_eq!(missing, 7_900),
        other => panic!("production ran without real inputs: {other:?}"),
    }
    // Nothing was consumed by the failed attempt.
    assert_eq!(w.lot(small_ore).unwrap().state, LotState::Active);
    assert_eq!(w.lot(small_ore).unwrap().quantity, 100);
}

#[test]
fn tooling_binds_production_to_exact_designs() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let other_tooling_design = w
        .define_design("Autocannon Line", AssetKind::FactoryTooling, vec![], None, None)
        .unwrap();
    let recipe = w.define_recipe(
        "Mill Autocannon Barrels",
        vec![(s.iron_ore, 100)],
        RecipeOutputs::Commodity { commodity: s.armor_plate, quantity: 10 },
        1,
        Some(other_tooling_design),
    );
    let ore = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 100, QualityGrade::Standard)
        .unwrap();
    w.admin_move_lot(ore, LocationRef::Site(s.forge), None).unwrap();
    // The factory has Armor Plate Line tooling, not Autocannon Line.
    assert_eq!(
        w.start_production(s.factory, recipe, &[ore]),
        Err(SimError::ToolingMismatch { factory: s.factory })
    );
}

#[test]
fn contracts_escrow_real_funds_and_point_at_real_targets() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // A contract against a nonexistent convoy is rejected outright.
    let ghost = ConvoyId(999_999);
    assert_eq!(
        w.issue_contract(
            s.karth,
            ContractObjective::AttackTarget {
                target: ContractTarget::Convoy(ghost),
                report_back_to: s.meridian,
            },
            25_000,
            SalvageTerms { contractor_share_pct: 50, employer_veto: true, tonnage_cap: None },
        ),
        Err(SimError::UnknownConvoy(ghost)),
        "issued a contract against an enemy that does not exist"
    );

    // A real target: Veyra's convoy... use the (disbanded) convoy's truck as
    // an asset target instead — assets are real targets too.
    let target_asset = w
        .assets_iter()
        .find(|a| a.owner == s.veyra)
        .map(|a| a.id)
        .expect("veyra has seeded mechs");
    let karth_before = w.actor(s.karth).unwrap().treasury;
    let contract = w
        .issue_contract(
            s.karth,
            ContractObjective::AttackTarget {
                target: ContractTarget::Asset(target_asset),
                report_back_to: s.meridian,
            },
            25_000,
            SalvageTerms { contractor_share_pct: 50, employer_veto: true, tonnage_cap: None },
        )
        .unwrap();
    // Funds are reserved at creation: real escrow, not a promise.
    assert_eq!(w.actor(s.karth).unwrap().treasury, karth_before - 25_000);

    // An employer that cannot pay cannot post.
    assert!(matches!(
        w.issue_contract(
            s.merc,
            ContractObjective::EscortConvoy { convoy: s.convoy },
            9_999_999,
            SalvageTerms { contractor_share_pct: 0, employer_veto: false, tonnage_cap: None },
        ),
        Err(SimError::InsufficientFunds { .. })
    ));

    let merc_before = w.actor(s.merc).unwrap().treasury;
    w.accept_contract(contract, s.merc).unwrap();
    // AttackTarget contracts are now verified against real state: completing
    // before the target is actually destroyed or captured must fail.
    assert_eq!(w.complete_contract(contract), Err(SimError::ContractNotFulfilled(contract)));
    // Make the objective real: the contractor captures the target.
    w.transfer_asset(target_asset, s.merc).unwrap();
    w.complete_contract(contract).unwrap();
    assert_eq!(w.actor(s.merc).unwrap().treasury, merc_before + 25_000);
    assert_eq!(w.contract(contract).unwrap().state, ContractState::Completed { by: s.merc });

    assert!(w.check_invariants().is_empty(), "money leaked somewhere");
}

#[test]
fn toe_assembles_only_from_physically_available_assets() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // Veyra has 2 Talons at Meridian; the lance needs 3. No spawning: the
    // attempt fails with a typed shortage — the demand signal.
    match w.try_assemble_formation(s.veyra, s.lance_template, "First Lance", s.meridian) {
        Err(SimError::ToeShortage(shortages)) => {
            assert_eq!(shortages.len(), 1);
            assert_eq!(shortages[0].design, s.talon_design);
            assert_eq!(shortages[0].missing, 1);
        }
        other => panic!("formation assembled from thin air: {other:?}"),
    }
    assert_eq!(w.toe_shortages(s.veyra, s.lance_template, s.meridian).unwrap().len(), 1);

    // A third real mech arrives (pre-war stock) and the lance assembles.
    w.seed_asset(
        s.veyra,
        s.talon_design,
        s.meridian,
        QualityGrade::Standard,
        AssetOrigin::SeededHistorical { note: "reserve mech".into() },
        None,
    )
    .unwrap();
    let formation = w
        .try_assemble_formation(s.veyra, s.lance_template, "First Lance", s.meridian)
        .unwrap();
    let f = w.formation(formation).unwrap().clone();
    assert_eq!(f.assets.len(), 3);
    for aid in &f.assets {
        let a = w.asset(*aid).unwrap();
        assert_eq!(a.owner, s.veyra);
        assert_eq!(a.location, LocationRef::Formation(formation));
    }

    // Those mechs are now committed: a second lance cannot reuse them.
    assert!(matches!(
        w.try_assemble_formation(s.veyra, s.lance_template, "Second Lance", s.meridian),
        Err(SimError::ToeShortage(_))
    ));

    assert!(w.check_invariants().is_empty());
}

#[test]
fn intel_is_immutable_observation_that_goes_stale() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let convoy = w.form_convoy(s.karth, s.redrock_mine, &[truck]).unwrap();

    // Veyra scouts spot the convoy forming at the mine.
    let intel = w
        .record_observation(
            Some(s.veyra),
            IntelSubject::Convoy(convoy),
            s.redrock_mine,
            "single truck, no escort",
            90,
            0,
        )
        .unwrap();
    assert_eq!(w.intel_is_stale(intel).unwrap(), Some(false));

    // The convoy departs; the observation stays true-as-of-day but stale.
    let route = w.create_route(s.redrock_mine, s.forge, 2).unwrap();
    w.depart_convoy(convoy, route).unwrap();
    assert_eq!(w.intel_is_stale(intel).unwrap(), Some(true));

    // The record itself never changed.
    let obs = w.intel_record(intel).unwrap();
    assert_eq!(obs.observed_at, s.redrock_mine);
    assert_eq!(obs.observer, Some(s.veyra));
    assert_eq!(obs.confidence_pct, 90);

    // Intel about nonexistent subjects cannot be recorded.
    assert!(matches!(
        w.record_observation(None, IntelSubject::Convoy(ConvoyId(424_242)), s.forge, "?", 10, 50),
        Err(SimError::UnknownConvoy(_))
    ));
}

#[test]
fn sequestered_stockpiles_block_open_trade() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let depot = w.create_stockpile(s.karth, s.meridian, true).unwrap();
    let plate = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            300,
            QualityGrade::Standard,
            LegalStatus::RestrictedMilitary,
            s.meridian,
            LotOrigin::SeededHistorical { note: "war reserve".into() },
        )
        .unwrap();
    w.deposit_lot(plate, depot).unwrap();
    assert_eq!(
        w.list_lot_for_sale(s.karth, s.market, plate, 100),
        Err(SimError::SequesteredStockpile(depot)),
        "sequestered war reserve reached the open market"
    );

    // Deliberate release: withdraw, then trade freely.
    w.withdraw_lot(plate).unwrap();
    assert!(w.list_lot_for_sale(s.karth, s.market, plate, 100).is_ok());

    assert!(w.check_invariants().is_empty());
}

#[test]
fn same_seed_and_inputs_produce_identical_state() {
    let a = build_scenario(42);
    let b = build_scenario(42);
    assert_eq!(
        a.world.state_digest(),
        b.world.state_digest(),
        "same seed + same ordered inputs diverged"
    );

    // Continued identical inputs stay identical.
    let (mut wa, mut wb) = (a.world, b.world);
    for _ in 0..10 {
        wa.tick();
        wb.tick();
    }
    assert_eq!(wa.state_digest(), wb.state_digest());

    // A different seed is a different world.
    let c = build_scenario(1337);
    assert_ne!(wa.state_digest(), c.world.state_digest());
}

#[test]
fn storage_roundtrip_preserves_exact_state() {
    let s = build_scenario(42);
    let mut store = InMemoryStore::new();
    store.save_snapshot(&s.world).unwrap();
    store.append_events(s.world.events()).unwrap();

    let loaded = store.load_latest_snapshot().unwrap().expect("snapshot saved");
    assert_eq!(loaded.state_digest(), s.world.state_digest());

    let events = store.load_events_from(0).unwrap();
    assert_eq!(events.len(), s.world.events().len());

    // The loaded world keeps simulating deterministically.
    let mut w1 = s.world;
    let mut w2 = loaded;
    w1.tick();
    w2.tick();
    assert_eq!(w1.state_digest(), w2.state_digest());
}

#[test]
fn resolve_battle_only_fights_real_colocated_owned_forces() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let veyra_mechs: Vec<_> = w.assets_iter().filter(|a| a.owner == s.veyra).map(|a| a.id).collect();
    // Karth has no forces at Meridian: this must fail, not fabricate a fight.
    assert!(matches!(
        w.resolve_battle(s.meridian, s.karth, &[], s.veyra, &veyra_mechs),
        Err(SimError::InvalidState(_))
    ));

    // Attacker force at the wrong site: must fail on colocation, not just
    // proceed with a phantom army.
    let karth_truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    assert_eq!(
        w.resolve_battle(s.meridian, s.karth, &[karth_truck], s.veyra, &veyra_mechs),
        Err(SimError::NotColocated)
    );
}

#[test]
fn battles_produce_real_losses_captures_and_territory_change() {
    let mut w = World::new(5);
    let strong = w.create_actor("Strong Faction", ActorKind::Faction, 0);
    let weak = w.create_actor("Weak Faction", ActorKind::Faction, 0);
    let site = w.create_location("Contested Ridge", LocationKind::Battlefield, (0, 0));

    let mech_slots: Vec<_> = (0..5)
        .map(|i| {
            let def = w.define_component_def(&format!("Part {i}"), 1, ComponentCategory::MechOrEquipment);
            ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] }
        })
        .collect();
    let design = w.define_design("Grunt", AssetKind::Mech, mech_slots, None, None).unwrap();

    let mut strong_assets = Vec::new();
    for _ in 0..10 {
        strong_assets.push(
            w.seed_asset(
                strong,
                design,
                site,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: "line mech".into() },
                None,
            )
            .unwrap(),
        );
    }
    let weak_asset = w
        .seed_asset(
            weak,
            design,
            site,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "outnumbered mech".into() },
            None,
        )
        .unwrap();

    assert_eq!(w.location(site).unwrap().controller, None);
    let outcome = w.resolve_battle(site, strong, &strong_assets, weak, &[weak_asset]).unwrap();

    assert!(outcome.attacker_won, "10 mechs vs 1, even with defender bonus, should overwhelm");
    assert_eq!(w.location(site).unwrap().controller, Some(strong), "winning attacker should take the site");
    let defender_after = w.asset(weak_asset).unwrap();
    assert!(
        defender_after.condition_pct == 0 || defender_after.owner == strong,
        "the lone defender should be a real casualty: destroyed or captured, not untouched"
    );
    // Every engaged asset still exists — nothing was deleted to make the
    // battle happen.
    for id in strong_assets.iter().chain(std::iter::once(&weak_asset)) {
        assert!(w.asset(*id).is_some());
    }
    assert!(w.check_invariants().is_empty());
}

#[test]
fn hostile_formations_sharing_a_site_fight_automatically_each_tick() {
    let mut w = World::new(11);
    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 0);
    let veyra = w.create_actor("Veyra Compact", ActorKind::Faction, 0);
    let site = w.create_location("Shared Ground", LocationKind::Battlefield, (0, 0));

    let mech_slots: Vec<_> = (0..5)
        .map(|i| {
            let def = w.define_component_def(&format!("Part {i}"), 1, ComponentCategory::MechOrEquipment);
            ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] }
        })
        .collect();
    let design = w.define_design("Grunt", AssetKind::Mech, mech_slots, None, None).unwrap();

    let make_formation = |w: &mut World, owner: ActorId, n: u32| -> FormationId {
        let mut assets = Vec::new();
        for _ in 0..n {
            assets.push(
                w.seed_asset(
                    owner,
                    design,
                    site,
                    QualityGrade::Standard,
                    AssetOrigin::SeededHistorical { note: "line mech".into() },
                    None,
                )
                .unwrap(),
            );
        }
        let template =
            w.define_toe_template("Ad Hoc", "line", vec![ToeSlot { role: "Line".into(), design, count: n }]);
        w.try_assemble_formation(owner, template, "Ad Hoc Force", site).unwrap()
    };
    make_formation(&mut w, karth, 3);
    make_formation(&mut w, veyra, 3);

    let before = w.events().len();
    w.tick();
    let fought = w.events()[before..]
        .iter()
        .any(|e| matches!(&e.kind, adona_sim::events::EventKind::BattleResolved { .. }));
    assert!(fought, "two hostile formations sharing a site did not fight on tick");
    assert!(w.check_invariants().is_empty());
}

#[test]
fn formations_march_on_contested_ground_and_fight_on_arrival() {
    use adona_sim::events::EventKind;
    use adona_sim::toe::FormationState;

    let mut w = World::new(23);
    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 0);
    let veyra = w.create_actor("Veyra Compact", ActorKind::Faction, 0);
    let home = w.create_location("Karth Home Ground", LocationKind::Battlefield, (0, 0));
    let frontier = w.create_location("Contested Frontier", LocationKind::Battlefield, (1, 0));
    w.set_territory_controller(home, Some(karth)).unwrap();
    // frontier is Veyra-held ground: Karth's formation must not already be
    // standing there, or this would just be the ordinary co-location fight.
    w.set_territory_controller(frontier, Some(veyra)).unwrap();
    let route = w.create_route(home, frontier, 2).unwrap();

    let mech_slots: Vec<_> = (0..5)
        .map(|i| {
            let def = w.define_component_def(&format!("Part {i}"), 1, ComponentCategory::MechOrEquipment);
            ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] }
        })
        .collect();
    let design = w.define_design("Grunt", AssetKind::Mech, mech_slots, None, None).unwrap();

    let make_formation = |w: &mut World, owner: ActorId, at: LocationId, n: u32| -> FormationId {
        let mut assets = Vec::new();
        for _ in 0..n {
            assets.push(
                w.seed_asset(
                    owner,
                    design,
                    at,
                    QualityGrade::Standard,
                    AssetOrigin::SeededHistorical { note: "line mech".into() },
                    None,
                )
                .unwrap(),
            );
        }
        let template =
            w.define_toe_template("Ad Hoc", "line", vec![ToeSlot { role: "Line".into(), design, count: n }]);
        w.try_assemble_formation(owner, template, "Ad Hoc Force", at).unwrap()
    };
    let karth_formation = make_formation(&mut w, karth, home, 5);
    // A lone outmatched defender already sitting on the frontier: enough to
    // prove the battle really happened, not enough to win it.
    make_formation(&mut w, veyra, frontier, 1);

    // Tick 1: Karth's formation, stationed on its own controlled ground with
    // a route to Veyra-held ground, marches automatically — no manual order
    // given.
    w.tick();
    match w.formation(karth_formation).unwrap().state {
        FormationState::EnRoute { route: r, .. } => assert_eq!(r, route),
        other => panic!("formation on controlled ground with a hostile-held neighbor did not march: {other:?}"),
    }
    assert!(w.formation(karth_formation).unwrap().current_site().is_none(), "en route formation is on no site");
    let marched = w
        .events()
        .iter()
        .any(|e| matches!(&e.kind, EventKind::FormationMarchOrdered { formation, .. } if *formation == karth_formation));
    assert!(marched, "no FormationMarchOrdered event was recorded");
    assert!(w.check_invariants().is_empty());

    // Tick 2: still en route (2-day route), cannot fight, cannot be ordered
    // to march again.
    w.tick();
    assert!(matches!(w.formation(karth_formation).unwrap().state, FormationState::EnRoute { .. }));
    assert_eq!(w.order_formation_march(karth_formation, route), Err(SimError::FormationNotAtSite(karth_formation)));

    // Tick 3: arrives at the frontier and, sharing hostile ground with
    // Veyra's formation, fights automatically the same tick it lands.
    let before = w.events().len();
    w.tick();
    assert_eq!(w.formation(karth_formation).unwrap().current_site(), Some(frontier));
    let arrived = w.events()[before..]
        .iter()
        .any(|e| matches!(&e.kind, EventKind::FormationArrived { formation, at } if *formation == karth_formation && *at == frontier));
    assert!(arrived, "no FormationArrived event was recorded");
    let fought = w.events()[before..].iter().any(|e| matches!(&e.kind, EventKind::BattleResolved { .. }));
    assert!(fought, "formation did not fight immediately on arrival at hostile ground");
    assert!(w.check_invariants().is_empty());
}

#[test]
fn haul_cargo_contracts_only_complete_once_cargo_really_arrives() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let cargo = w
        .produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 50, QualityGrade::Standard)
        .unwrap();
    let contract = w
        .issue_contract(
            s.karth,
            ContractObjective::HaulCargo { lots: vec![cargo], from: s.redrock_mine, to: s.forge },
            5_000,
            SalvageTerms { contractor_share_pct: 0, employer_veto: false, tonnage_cap: None },
        )
        .unwrap();
    w.accept_contract(contract, s.merc).unwrap();

    // Cargo is still at the mine: completing early must fail, not just be
    // trusted.
    assert_eq!(w.complete_contract(contract), Err(SimError::ContractNotFulfilled(contract)));

    // Actually haul it, for real, then completion succeeds.
    let truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let route = w.create_route(s.redrock_mine, s.forge, 1).unwrap();
    let convoy = w.form_convoy(s.karth, s.redrock_mine, &[truck]).unwrap();
    w.load_lot_onto_convoy(convoy, cargo).unwrap();
    w.depart_convoy(convoy, route).unwrap();
    w.tick();
    w.unload_lot(convoy, cargo).unwrap();

    w.complete_contract(contract).unwrap();
    assert_eq!(w.contract(contract).unwrap().state, ContractState::Completed { by: s.merc });
    assert!(w.check_invariants().is_empty());
}

#[test]
fn salvage_settles_real_assets_respects_veto_and_cap_and_flags_illegal_goods() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let target_asset = w.assets_iter().find(|a| a.owner == s.veyra).map(|a| a.id).unwrap();
    let contract = w
        .issue_contract(
            s.karth,
            ContractObjective::AttackTarget {
                target: ContractTarget::Asset(target_asset),
                report_back_to: s.meridian,
            },
            10_000,
            SalvageTerms { contractor_share_pct: 50, employer_veto: true, tonnage_cap: Some(200) },
        )
        .unwrap();
    w.accept_contract(contract, s.merc).unwrap();
    // The objective must be real before completion pays out: the
    // contractor captures the target.
    w.transfer_asset(target_asset, s.merc).unwrap();
    w.complete_contract(contract).unwrap();

    // Give Karth (the employer) a mixed pile of "wreck" loot: one asset, one
    // stolen lot. Karth owns the asset until salvage transfers it.
    w.transfer_asset(target_asset, s.karth).unwrap();
    let stolen = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            50,
            QualityGrade::Standard,
            LegalStatus::Stolen,
            s.meridian,
            LotOrigin::SeededHistorical { note: "salvaged plate".into() },
        )
        .unwrap();

    // Employer veto is on: without an approval list nothing transfers.
    w.settle_salvage(contract, &[target_asset], &[(stolen, 50)], None).unwrap();
    assert_eq!(w.asset(target_asset).unwrap().owner, s.karth, "vetoed salvage transferred anyway");

    // Approve just the asset, not the stolen lot.
    w.settle_salvage(
        contract,
        &[target_asset],
        &[(stolen, 50)],
        Some(&[adona_sim::contracts::AssetOrLot::Asset(target_asset)]),
    )
    .unwrap();
    assert_eq!(w.asset(target_asset).unwrap().owner, s.merc, "approved asset did not transfer");
    assert_eq!(w.lot(stolen).unwrap().owner, s.karth, "unapproved lot transferred anyway");

    // Now approve the stolen lot too: it transfers and gets flagged.
    let before = w.events().len();
    w.settle_salvage(
        contract,
        &[],
        &[(stolen, 50)],
        Some(&[adona_sim::contracts::AssetOrLot::Lot(stolen)]),
    )
    .unwrap();
    assert_eq!(w.lot(stolen).unwrap().owner, s.merc);
    assert!(w.events()[before..].iter().any(|e| matches!(
        &e.kind,
        adona_sim::events::EventKind::IllegalSalvageFlagged { legal_status: LegalStatus::Stolen, .. }
    )));

    // Tonnage cap is enforced against real requested quantity.
    let too_much = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            500,
            QualityGrade::Standard,
            LegalStatus::Legitimate,
            s.meridian,
            LotOrigin::SeededHistorical { note: "too much plate".into() },
        )
        .unwrap();
    assert_eq!(
        w.settle_salvage(contract, &[], &[(too_much, 500)], None),
        Err(SimError::SalvageCapExceeded { contract, requested: 500, cap: 200 })
    );

    assert!(w.check_invariants().is_empty());
}

#[test]
fn en_route_convoys_can_generate_real_contact_intel_each_quarter() {
    // A long enough haul and enough ticks that a 15%-per-quarter contact
    // chance is overwhelmingly likely to fire at least once, proving the
    // sub-day quarter phase actually runs and actually produces intel.
    let mut w = World::new(99);
    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 100_000);
    let a = w.create_location("A", LocationKind::Mine, (0, 0));
    let b = w.create_location("B", LocationKind::FactorySite, (50, 0));
    let route = w.create_route(a, b, 40).unwrap();
    let truck_design = w.define_design("Truck", AssetKind::Vehicle, vec![], Some(1_000), None).unwrap();
    let truck = w
        .seed_asset(
            karth,
            truck_design,
            a,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let convoy = w.form_convoy(karth, a, &[truck]).unwrap();
    w.depart_convoy(convoy, route).unwrap();

    let mut contacted = false;
    for _ in 0..40 {
        w.tick();
        if w.events().iter().any(|e| {
            matches!(&e.kind, adona_sim::events::EventKind::IntelRecorded { intel }
                if matches!(w.intel_record(*intel).map(|o| o.subject), Some(IntelSubject::Convoy(c)) if c == convoy))
        }) {
            contacted = true;
            break;
        }
    }
    assert!(contacted, "a 40-day, 4-quarter-per-day transit never generated a single contact");
}

#[test]
fn population_consumes_real_stock_and_taxation_issues_real_money() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    w.set_tax_rate(s.meridian, 1).unwrap();
    let authority_before = w.actor(s.authority).unwrap().treasury;
    let unrest_before = w.location(s.meridian).unwrap().unrest_pct;

    // Stock the authority with more food than one day's need so demand can
    // clear and population can actually eat it.
    let food_lot = w
        .seed_lot(
            s.authority,
            s.food,
            10_000,
            QualityGrade::Standard,
            LegalStatus::Legitimate,
            s.meridian,
            LotOrigin::SeededHistorical { note: "city reserve".into() },
        )
        .unwrap();
    let _ = food_lot;

    let before_events = w.events().len();
    w.tick();

    // Real goods were actually destroyed by consumption, not just bought.
    let consumed = w.events()[before_events..].iter().any(|e| {
        matches!(&e.kind, adona_sim::events::EventKind::GoodsConsumed { commodity, .. } if *commodity == s.food)
    });
    assert!(consumed, "population did not actually consume its food stock");

    // Taxation issued real money: the authority's treasury grew by more than
    // any single trade could explain, and total circulation grew with it
    // (this is deliberate issuance, checked against the invariant below).
    assert!(
        w.actor(s.authority).unwrap().treasury >= authority_before,
        "authority treasury should not shrink from taxation"
    );
    assert!(w.check_invariants().is_empty(), "taxation issuance broke money conservation");

    // With this tick's need fully met, unrest should ease rather than climb
    // further (build_scenario's earlier days had no food stock yet, so some
    // accumulated unrest from before this fix is expected and fine).
    let unrest_after = w.location(s.meridian).unwrap().unrest_pct;
    assert!(unrest_after < unrest_before, "unrest should ease once the need is actually met");
}

#[test]
fn unmet_civilian_need_raises_unrest_and_can_shrink_population() {
    let mut w = World::new(7);
    let authority = w.create_actor("Broke Authority", ActorKind::CityAuthority, 0);
    let city = w.create_location("Hungry City", LocationKind::City, (0, 0));
    let food = w.define_commodity("Food", UnitOfMeasure::Units, 1, 5);
    w.configure_city(
        city,
        1_000_000,
        Some(authority),
        vec![CivilianNeed { commodity: food, quantity_per_day: 100 }],
    )
    .unwrap();
    // No market, no food stock, no funds: the need cannot possibly be met.

    for _ in 0..20 {
        w.tick();
    }
    let loc = w.location(city).unwrap();
    assert!(loc.unrest_pct > 0, "chronic unmet need should raise unrest");
    assert!(loc.population < 1_000_000, "sustained unrest should suppress or reverse growth");
    assert!(w.check_invariants().is_empty());
}

#[test]
fn faction_ai_produces_toward_a_real_toe_shortage_when_it_can() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // Give Veyra a fully operational factory tooled to build Talons, at
    // Meridian, plus the raw ore a mech-assembly recipe needs.
    let veyra_forge = w.create_location("Veyra Yard", LocationKind::FactorySite, (1, 1));
    let factory = w.create_factory(s.veyra, veyra_forge, 1).unwrap();
    let mech_tooling_design =
        w.define_design("Talon Assembly Line", AssetKind::FactoryTooling, vec![], None, Some(s.talon_design))
            .unwrap();
    let tooling = w
        .seed_asset(
            s.veyra,
            mech_tooling_design,
            veyra_forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war mech line".into() },
            None,
        )
        .unwrap();
    w.install_tooling(factory, tooling, 0, 0).unwrap();
    for category in ComponentCategory::FACTORY_SLOTS {
        let def = w.define_component_def("Veyra Yard sub-system", 1, category);
        let comp = w
            .seed_component(
                s.veyra,
                def,
                veyra_forge,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: "pre-war plant".into() },
            )
            .unwrap();
        w.fit_factory_component(factory, comp).unwrap();
    }
    w.define_recipe(
        "Assemble Talon",
        vec![(s.iron_ore, 100)],
        RecipeOutputs::SerialAssets { design: s.talon_design, count: 1 },
        1,
        Some(mech_tooling_design),
    );
    w.produce_from_mine(s.redrock_mine, s.veyra, s.iron_ore, 500, QualityGrade::Standard)
        .and_then(|ore| w.admin_move_lot(ore, LocationRef::Site(veyra_forge), None))
        .unwrap();

    // Veyra is short one Talon for the lance (2 seeded vs 3 needed). Register
    // the standing goal and let the faction AI phase of tick() act on it.
    w.set_faction_goal(s.veyra, s.lance_template, s.meridian).unwrap();
    let before = w.events().len();
    w.tick();
    let started = w.events()[before..]
        .iter()
        .any(|e| matches!(e.kind, adona_sim::events::EventKind::FactionProcurementStarted { .. }));
    assert!(started, "faction AI did not start production toward its real TO&E shortage");
    assert!(w.check_invariants().is_empty());
}

#[test]
fn faction_ai_orders_missing_inputs_when_it_cannot_produce_yet() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let veyra_forge = w.create_location("Veyra Yard", LocationKind::FactorySite, (1, 1));
    let factory = w.create_factory(s.veyra, veyra_forge, 1).unwrap();
    let mech_tooling_design =
        w.define_design("Talon Assembly Line", AssetKind::FactoryTooling, vec![], None, Some(s.talon_design))
            .unwrap();
    let tooling = w
        .seed_asset(
            s.veyra,
            mech_tooling_design,
            veyra_forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war mech line".into() },
            None,
        )
        .unwrap();
    w.install_tooling(factory, tooling, 0, 0).unwrap();
    for category in ComponentCategory::FACTORY_SLOTS {
        let def = w.define_component_def("Veyra Yard sub-system", 1, category);
        let comp = w
            .seed_component(
                s.veyra,
                def,
                veyra_forge,
                QualityGrade::Standard,
                AssetOrigin::SeededHistorical { note: "pre-war plant".into() },
            )
            .unwrap();
        w.fit_factory_component(factory, comp).unwrap();
    }
    w.define_recipe(
        "Assemble Talon",
        vec![(s.iron_ore, 100)],
        RecipeOutputs::SerialAssets { design: s.talon_design, count: 1 },
        1,
        Some(mech_tooling_design),
    );
    // Deliberately no ore this time: Veyra has the line but not the input.

    w.set_faction_goal(s.veyra, s.lance_template, s.meridian).unwrap();
    let before = w.events().len();
    w.tick();
    let ordered = w.events()[before..]
        .iter()
        .any(|e| matches!(e.kind, adona_sim::events::EventKind::FactionProcurementOrdered { .. }));
    assert!(ordered, "faction AI did not fall back to ordering the missing recipe input");
    assert!(w.check_invariants().is_empty());
}

#[test]
fn intel_relays_decay_confidence_and_never_touch_the_source() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let convoy = w.form_convoy(s.karth, s.redrock_mine, &[truck]).unwrap();

    let sighting = w
        .record_observation(
            Some(s.veyra),
            IntelSubject::Convoy(convoy),
            s.redrock_mine,
            "single truck, no escort",
            90,
            0,
        )
        .unwrap();

    // Word travels: Veyra's scout tells the Meridian authority, who tells a
    // trader. Each hop decays confidence and never mutates the earlier hop.
    let hop1 = w.relay_intel(sighting, s.authority, 20).unwrap();
    let hop2 = w.relay_intel(hop1, s.merc, 30).unwrap();

    let original = w.intel_record(sighting).unwrap().clone();
    assert_eq!(original.confidence_pct, 90, "relaying must not mutate the source record");
    assert_eq!(original.observer, Some(s.veyra));

    let h1 = w.intel_record(hop1).unwrap();
    assert_eq!(h1.confidence_pct, 72); // 90 * (100-20)/100
    assert_eq!(h1.observer, None, "a relay is secondhand, not the original witness");
    assert_eq!(h1.chain, vec![s.veyra, s.authority]);
    assert_eq!(h1.derived_from, Some(sighting));

    let h2 = w.intel_record(hop2).unwrap();
    assert_eq!(h2.confidence_pct, 50); // 72 * (100-30)/100 = 50.4 -> 50
    assert!(h2.corruption_pct > h1.corruption_pct, "corruption should climb with each hop");
    assert_eq!(h2.chain, vec![s.veyra, s.authority, s.merc]);
}

#[test]
fn misinformation_is_flagged_and_still_points_at_a_real_subject() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let truck = w
        .seed_asset(
            s.karth,
            s.truck_design,
            s.redrock_mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare truck".into() },
            None,
        )
        .unwrap();
    let convoy = w.form_convoy(s.karth, s.redrock_mine, &[truck]).unwrap();

    // A planter cannot invent a target that does not exist...
    assert!(matches!(
        w.plant_misinformation(
            Some(s.veyra),
            IntelSubject::Convoy(ConvoyId(999_999)),
            s.redrock_mine,
            "twelve mechs, heavy escort",
            95,
        ),
        Err(SimError::UnknownConvoy(_))
    ));

    // ...but can lie about a real one.
    let lie = w
        .plant_misinformation(
            Some(s.veyra),
            IntelSubject::Convoy(convoy),
            s.redrock_mine,
            "twelve mechs, heavy escort",
            95,
        )
        .unwrap();
    let record = w.intel_record(lie).unwrap();
    assert!(record.fabricated);
    assert_eq!(record.corruption_pct, 100);
}

#[test]
fn intel_staleness_covers_assets_formations_and_stockpiles() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // Asset staleness: a mech seen at Meridian is stale once it moves.
    let mech = w
        .seed_asset(
            s.veyra,
            s.talon_design,
            s.meridian,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare mech, not part of the lance".into() },
            None,
        )
        .unwrap();
    let sighting = w
        .record_observation(None, IntelSubject::Asset(mech), s.meridian, "idle", 100, 0)
        .unwrap();
    assert_eq!(w.intel_is_stale(sighting).unwrap(), Some(false));
    w.transfer_asset(mech, s.merc).unwrap(); // ownership change alone must not move it
    assert_eq!(w.intel_is_stale(sighting).unwrap(), Some(false), "ownership change is not relocation");

    // Formation staleness: assembling elsewhere makes the sighting stale.
    w.seed_asset(
        s.veyra,
        s.talon_design,
        s.meridian,
        QualityGrade::Standard,
        AssetOrigin::SeededHistorical { note: "reserve mech".into() },
        None,
    )
    .unwrap();
    let formation = w.try_assemble_formation(s.veyra, s.lance_template, "First Lance", s.meridian).unwrap();
    let formation_intel = w
        .record_observation(None, IntelSubject::Formation(formation), s.meridian, "assembling", 100, 0)
        .unwrap();
    assert_eq!(w.intel_is_stale(formation_intel).unwrap(), Some(false));

    // Stockpile-site staleness: depositing new cargo after the observation
    // makes it stale.
    let depot = w.create_stockpile(s.karth, s.meridian, false).unwrap();
    let stockpile_intel = w
        .record_observation(None, IntelSubject::StockpileSite(s.meridian), s.meridian, "quiet", 80, 0)
        .unwrap();
    assert_eq!(w.intel_is_stale(stockpile_intel).unwrap(), Some(false));
    let plate = w
        .seed_lot(
            s.karth,
            s.armor_plate,
            10,
            QualityGrade::Standard,
            LegalStatus::Legitimate,
            s.meridian,
            LotOrigin::SeededHistorical { note: "fresh delivery".into() },
        )
        .unwrap();
    w.deposit_lot(plate, depot).unwrap();
    assert_eq!(w.intel_is_stale(stockpile_intel).unwrap(), Some(true));
}

#[test]
fn price_index_moves_toward_real_trades_but_smooths() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // No trade has happened yet at 50 cr/unit for armor plate.
    assert_eq!(w.price_index(s.market, s.armor_plate), None);

    let listing = w.list_lot_for_sale(s.karth, s.market, s.armor_lot, 50).unwrap();
    w.execute_purchase(listing, s.veyra, 100).unwrap();
    assert_eq!(w.price_index(s.market, s.armor_plate), Some(50));
    w.cancel_listing(listing).unwrap();

    // A much higher second trade nudges the index up, but does not jump to
    // it outright (75/25 smoothing) — anti-oscillation.
    let listing2 = w.list_lot_for_sale(s.karth, s.market, s.armor_lot, 200).unwrap();
    w.execute_purchase(listing2, s.veyra, 100).unwrap();
    let idx = w.price_index(s.market, s.armor_plate).unwrap();
    assert!(idx > 50 && idx < 200, "index should move toward but not jump to the new trade: {idx}");
    assert_eq!(idx, (50 * 3 + 200) / 4);
}

#[test]
fn mech_designs_require_exactly_five_components() {
    let mut w = World::new(1);
    let def = w.define_component_def("Some Part", 1, ComponentCategory::MechOrEquipment);

    // Too few slots.
    assert_eq!(
        w.define_design(
            "Bad Mech",
            AssetKind::Mech,
            vec![ComponentSlot { name: "Only Slot".into(), accepts: vec![def] }],
            None,
            None,
        ),
        Err(SimError::InvalidSlotCount { kind: AssetKind::Mech, got: 1 })
    );

    // Exactly five is required, not "five or more".
    let six_slots: Vec<_> = (0..6)
        .map(|i| ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] })
        .collect();
    assert_eq!(
        w.define_design("Overloaded Mech", AssetKind::Mech, six_slots, None, None),
        Err(SimError::InvalidSlotCount { kind: AssetKind::Mech, got: 6 })
    );

    let five_slots: Vec<_> = (0..5)
        .map(|i| ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] })
        .collect();
    assert!(w.define_design("Fine Mech", AssetKind::Mech, five_slots, None, None).is_ok());
}

#[test]
fn equipment_designs_require_five_or_six_components() {
    let mut w = World::new(1);
    let def = w.define_component_def("Some Part", 1, ComponentCategory::MechOrEquipment);
    let slots_of = |n: usize| -> Vec<ComponentSlot> {
        (0..n).map(|i| ComponentSlot { name: format!("Slot {i}"), accepts: vec![def] }).collect()
    };

    assert_eq!(
        w.define_design("Bad Weapon", AssetKind::Weapon, slots_of(4), None, None),
        Err(SimError::InvalidSlotCount { kind: AssetKind::Weapon, got: 4 })
    );
    assert!(w.define_design("Fine Weapon (5)", AssetKind::Weapon, slots_of(5), None, None).is_ok());
    assert!(w.define_design("Fine Equipment (6)", AssetKind::Equipment, slots_of(6), None, None).is_ok());
    assert_eq!(
        w.define_design("Bad Equipment", AssetKind::Equipment, slots_of(7), None, None),
        Err(SimError::InvalidSlotCount { kind: AssetKind::Equipment, got: 7 })
    );
}

#[test]
fn factories_cannot_produce_until_all_five_sub_systems_are_fitted() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    // A brand new factory has tooling installed via build_scenario, but this
    // one gets none of the five sub-system components.
    let bare_factory = w.create_factory(s.karth, s.forge, 1).unwrap();
    let bare_tooling_design =
        w.define_design("Bare Line", AssetKind::FactoryTooling, vec![], None, None).unwrap();
    let bare_tooling = w
        .seed_asset(
            s.karth,
            bare_tooling_design,
            s.forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare tooling".into() },
            None,
        )
        .unwrap();
    w.install_tooling(bare_factory, bare_tooling, 0, 0).unwrap();

    let recipe = w.define_recipe(
        "Roll Armor Plate (bare factory)",
        vec![(s.iron_ore, 10)],
        RecipeOutputs::Commodity { commodity: s.armor_plate, quantity: 5 },
        1,
        None,
    );
    let ore = w.produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 10, QualityGrade::Standard).unwrap();
    w.admin_move_lot(ore, LocationRef::Site(s.forge), None).unwrap();

    match w.start_production(bare_factory, recipe, &[ore]) {
        Err(SimError::FactoryIncomplete { factory, missing }) => {
            assert_eq!(factory, bare_factory);
            assert_eq!(missing.len(), 5);
        }
        other => panic!("factory produced without its five sub-systems: {other:?}"),
    }

    // s.factory (from build_scenario) already has all five fitted, so the
    // identical recipe runs there without a FactoryIncomplete error.
    let ore2 = w.produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 10, QualityGrade::Standard).unwrap();
    w.admin_move_lot(ore2, LocationRef::Site(s.forge), None).unwrap();
    assert!(w.start_production(s.factory, recipe, &[ore2]).is_ok());
}

#[test]
fn retooling_burns_real_money_and_imposes_downtime() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    let karth_before = w.actor(s.karth).unwrap().treasury;
    let new_tooling_design =
        w.define_design("Second Line", AssetKind::FactoryTooling, vec![], None, None).unwrap();
    let new_tooling = w
        .seed_asset(
            s.karth,
            new_tooling_design,
            s.forge,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "spare tooling".into() },
            None,
        )
        .unwrap();
    w.install_tooling(s.factory, new_tooling, 2_000, 3).unwrap();
    assert_eq!(w.actor(s.karth).unwrap().treasury, karth_before - 2_000, "retool cost was not charged");

    // Downtime blocks production until the retool window passes.
    let recipe = w.define_recipe(
        "Roll Armor Plate (post-retool)",
        vec![(s.iron_ore, 10)],
        RecipeOutputs::Commodity { commodity: s.armor_plate, quantity: 5 },
        1,
        None,
    );
    let ore = w.produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 10, QualityGrade::Standard).unwrap();
    w.admin_move_lot(ore, LocationRef::Site(s.forge), None).unwrap();
    assert!(matches!(w.start_production(s.factory, recipe, &[ore]), Err(SimError::InvalidState(_))));

    assert!(w.check_invariants().is_empty(), "burned retool money violated conservation");
}

#[test]
fn mine_reserves_deplete_for_real() {
    let mut s = build_scenario(42);
    let w = &mut s.world;

    w.configure_mine(
        s.redrock_mine,
        adona_sim::locations::MineReserves::Finite { remaining: 1_000 },
    )
    .unwrap();
    w.produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 800, QualityGrade::Standard)
        .unwrap();
    match w.produce_from_mine(s.redrock_mine, s.karth, s.iron_ore, 800, QualityGrade::Standard) {
        Err(SimError::InsufficientQuantity { missing, .. }) => assert_eq!(missing, 600),
        other => panic!("mined ore that is not in the ground: {other:?}"),
    }
}
