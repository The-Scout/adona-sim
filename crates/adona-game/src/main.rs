//! ADONA strategic observer view.
//!
//! A Bevy GUI over `adona-sim`: a top-down map of sites, convoys, and
//! formations, plus egui side panels for treasuries, contracts, and the
//! event log. Time only advances when you press Step — this is a window
//! onto the headless simulation, not the cockpit/tactical game (that is
//! separate, later work per the docket's staging).

use adona_sim::actors::{Actor, ActorKind};
use adona_sim::assets::{AssetKind, AssetOrigin, ComponentCategory, ComponentSlot};
use adona_sim::contracts::ContractState;
use adona_sim::convoys::ConvoyState;
use adona_sim::goods::{LegalStatus, LotOrigin, QualityGrade, UnitOfMeasure};
use adona_sim::ids::ActorId;
use adona_sim::locations::{CivilianNeed, LocationKind};
use adona_sim::production::RecipeOutputs;
use adona_sim::toe::ToeSlot;
use adona_sim::World;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

#[derive(Resource)]
struct SimWorld(World);

/// World-space pixels per one unit of a `Location`'s abstract map position.
const MAP_SCALE: f32 = 8.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "ADONA — Strategic Observer".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin)
        .insert_resource(SimWorld(build_demo_world()))
        .add_systems(Startup, setup_camera)
        .add_systems(Update, (ui_panels, draw_map))
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Seed a small two-faction economy so there is something to watch
/// immediately, mirroring `adona-sim`'s own `examples/demo.rs` scenario.
fn build_demo_world() -> World {
    let mut w = World::new(42);

    let karth = w.create_actor("Karth Directorate", ActorKind::Faction, 1_000_000);
    let veyra = w.create_actor("Veyra Compact", ActorKind::Faction, 1_000_000);
    let authority = w.create_actor("Meridian City Authority", ActorKind::CityAuthority, 60_000);

    let meridian = w.create_location("Meridian", LocationKind::City, (0, 0));
    let mine = w.create_location("Redrock Mine", LocationKind::Mine, (10, 4));
    let forge = w.create_location("Forge Complex", LocationKind::FactorySite, (5, -6));

    let iron_ore = w.define_commodity("Iron Ore", UnitOfMeasure::Kilograms, 1, 2);
    let armor_plate = w.define_commodity("Armor Plate", UnitOfMeasure::Kilograms, 2, 40);
    let food = w.define_commodity("Food", UnitOfMeasure::Units, 1, 3);

    w.configure_city(
        meridian,
        10_000,
        Some(authority),
        vec![CivilianNeed { commodity: food, quantity_per_day: 500 }],
    )
    .unwrap();
    w.set_tax_rate(meridian, 1).unwrap();
    let market = w.create_market("Meridian Exchange", meridian, None).unwrap();

    let truck_design =
        w.define_design("Hauler-6 Truck", AssetKind::Vehicle, vec![], Some(20_000), None).unwrap();
    let mech_slots: Vec<_> = ["Leg Actuator", "Weapon Barrel", "Ammo Feed", "Reactor Feed", "Cooling Assembly"]
        .into_iter()
        .map(|name| {
            let def = w.define_component_def(name, 2, ComponentCategory::MechOrEquipment);
            ComponentSlot { name: name.to_string(), accepts: vec![def] }
        })
        .collect();
    let talon_design = w.define_design("TLN-3 Talon", AssetKind::Mech, mech_slots, None, None).unwrap();
    let tooling_design =
        w.define_design("Armor Plate Line", AssetKind::FactoryTooling, vec![], None, None).unwrap();

    let ore = w.produce_from_mine(mine, karth, iron_ore, 10_000, QualityGrade::Standard).unwrap();
    let truck = w
        .seed_asset(
            karth,
            truck_design,
            mine,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: "pre-war logistics fleet".into() },
            Some("Old Reliable"),
        )
        .unwrap();
    let route = w.create_route(mine, forge, 2).unwrap();
    let convoy = w.form_convoy(karth, mine, &[truck]).unwrap();
    w.load_lot_onto_convoy(convoy, ore).unwrap();
    w.depart_convoy(convoy, route).unwrap();

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
    w.install_tooling(factory, tooling, 0, 0).unwrap();
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
    w.define_recipe(
        "Roll Armor Plate",
        vec![(iron_ore, 8_000)],
        RecipeOutputs::Commodity { commodity: armor_plate, quantity: 4_000 },
        3,
        Some(tooling_design),
    );

    let grain = w
        .seed_lot(
            karth,
            food,
            6_000,
            QualityGrade::Standard,
            LegalStatus::Legitimate,
            meridian,
            LotOrigin::SeededHistorical { note: "grain reserve".into() },
        )
        .unwrap();
    w.list_lot_for_sale(karth, market, grain, 3).unwrap();

    for i in 0..2 {
        w.seed_asset(
            veyra,
            talon_design,
            meridian,
            QualityGrade::Standard,
            AssetOrigin::SeededHistorical { note: format!("pre-war mech {i}") },
            None,
        )
        .unwrap();
    }
    let lance = w.define_toe_template(
        "Talon Lance",
        "line-defense",
        vec![ToeSlot { role: "Line Mech".into(), design: talon_design, count: 3 }],
    );
    w.set_faction_goal(veyra, lance, meridian).unwrap();
    w.set_territory_controller(meridian, Some(veyra)).unwrap();
    w.set_territory_controller(forge, Some(karth)).unwrap();
    w.set_territory_controller(mine, Some(karth)).unwrap();

    w
}

/// Stable, deterministic color per actor so the same faction reads as the
/// same color across every panel and every frame.
fn actor_color(id: ActorId) -> Color {
    let h = (id.0.wrapping_mul(2654435761) % 360) as f32;
    Color::hsl(h, 0.65, 0.55)
}

fn map_pos(position: (i64, i64)) -> Vec2 {
    Vec2::new(position.0 as f32, position.1 as f32) * MAP_SCALE
}

fn draw_map(sim: Res<SimWorld>, mut gizmos: Gizmos) {
    let w = &sim.0;

    // Routes as faint lines between the sites they connect.
    for route in w.routes_iter() {
        let (Some(from), Some(to)) = (w.location(route.from), w.location(route.to)) else { continue };
        gizmos.line_2d(map_pos(from.position), map_pos(to.position), Color::srgba(0.5, 0.5, 0.5, 0.4));
    }

    // Sites: a circle colored by controller, sized a little larger for
    // cities than outposts.
    for loc in w.locations_iter() {
        let pos = map_pos(loc.position);
        let radius = match loc.kind {
            adona_sim::locations::LocationKind::City => 18.0,
            _ => 12.0,
        };
        let color = loc.controller.map(actor_color).unwrap_or(Color::srgb(0.4, 0.4, 0.4));
        gizmos.circle_2d(pos, radius, color);
    }

    // Convoys: a small marker at their site, or interpolated along the
    // route while en route — real movement, drawn honestly as "on the
    // road" rather than snapped to either endpoint.
    for convoy in w.convoys_iter() {
        let color = actor_color(convoy.owner);
        match convoy.state {
            ConvoyState::Forming { at } | ConvoyState::Arrived { at } => {
                if let Some(loc) = w.location(at) {
                    gizmos.circle_2d(map_pos(loc.position) + Vec2::new(0.0, 20.0), 4.0, color);
                }
            }
            ConvoyState::EnRoute { route, departed_day, arrives_day } => {
                let Some(r) = w.route(route) else { continue };
                let (Some(from), Some(to)) = (w.location(r.from), w.location(r.to)) else { continue };
                let span = (arrives_day - departed_day).max(1) as f32;
                let progress = ((w.today() - departed_day) as f32 / span).clamp(0.0, 1.0);
                let pos = map_pos(from.position).lerp(map_pos(to.position), progress);
                gizmos.circle_2d(pos, 5.0, color);
            }
            ConvoyState::Disbanded => {}
        }
    }

    // Formations: small squares offset from their home site, one per
    // formation, so multiple factions holding the same ground are visible
    // as separate marks (the setup for the automatic ant-sim clash).
    for (i, formation) in w.formations_iter().enumerate() {
        if let Some(at) = formation.current_site() {
            if let Some(loc) = w.location(at) {
                let offset = Vec2::new(-20.0 + (i as f32 % 3.0) * 10.0, -20.0 - (i as f32 / 3.0).floor() * 10.0);
                let pos = map_pos(loc.position) + offset;
                let color = actor_color(formation.owner);
                gizmos.rect_2d(pos, Vec2::splat(8.0), color);
            }
        }
    }
}

fn ui_panels(mut contexts: EguiContexts, mut sim: ResMut<SimWorld>) {
    let ctx = contexts.ctx_mut();

    egui::TopBottomPanel::top("top").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading(format!("Day {}", sim.0.today()));
            if ui.button("Step Day").clicked() {
                sim.0.tick();
            }
            if ui.button("Step 10 Days").clicked() {
                for _ in 0..10 {
                    sim.0.tick();
                }
            }
            let violations = sim.0.check_invariants();
            ui.label(if violations.is_empty() {
                "invariants: OK".to_string()
            } else {
                format!("invariants: {} VIOLATION(S)", violations.len())
            });
        });
    });

    egui::SidePanel::right("panel").min_width(360.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Actors");
            let actors: Vec<Actor> = sim.0.actors_iter().cloned().collect();
            for actor in &actors {
                ui.label(format!("{}: {} cr", actor.name, actor.treasury));
            }

            ui.separator();
            ui.heading("Territory");
            for loc in sim.0.locations_iter() {
                let owner = loc
                    .controller
                    .and_then(|id| sim.0.actor(id))
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "unclaimed".to_string());
                ui.label(format!("{}: {} (pop {}, unrest {}%)", loc.name, owner, loc.population, loc.unrest_pct));
            }

            ui.separator();
            ui.heading("Contracts");
            for c in sim.0.contracts_iter() {
                let state = match &c.state {
                    ContractState::Open => "open".to_string(),
                    ContractState::Accepted { by } => format!("accepted by {by}"),
                    ContractState::Completed { by } => format!("completed by {by}"),
                    ContractState::Failed => "failed".to_string(),
                    ContractState::Cancelled => "cancelled".to_string(),
                };
                ui.label(format!("{} — {} — {} cr", c.id, state, c.escrowed_payment));
            }

            ui.separator();
            ui.heading("Recent Events");
            for event in sim.0.events().iter().rev().take(40) {
                ui.label(format!("day {}: {:?}", event.day, event.kind));
            }
        });
    });
}
