//! ADONA strategic observer view.
//!
//! A Bevy GUI over `adona-sim`: a top-down map of sites, convoys, and
//! formations, plus egui side panels for treasuries, contracts, and the
//! event log. Time only advances when you press Step — this is a window
//! onto the headless simulation, not the cockpit/tactical game (that is
//! separate, later work per the docket's staging).

mod seed;

use adona_sim::actors::Actor;
use adona_sim::contracts::ContractState;
use adona_sim::convoys::{Convoy, ConvoyState};
use adona_sim::ids::ActorId;
use adona_sim::locations::LocationKind;
use adona_sim::toe::{Formation, FormationState};
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
        .insert_resource(SimWorld(load_seeded_world()))
        .add_systems(Startup, setup_camera)
        .add_systems(Update, (ui_panels, draw_map, hover_tooltip))
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Load the game's real faction content from `assets/world_seed.json` — a
/// moddable data file, not hardcoded Rust — and build a `World` from it.
fn load_seeded_world() -> World {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/world_seed.json");
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read world seed {path:?}: {e}"));
    let file: seed::WorldSeedFile =
        serde_json::from_str(&data).unwrap_or_else(|e| panic!("failed to parse world seed {path:?}: {e}"));
    seed::build_world(&file).unwrap_or_else(|e| panic!("failed to build seeded world: {e}"))
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

/// A site's drawn (and hoverable) radius: a little larger for cities than
/// outposts, shared by drawing and hit-testing so they can never drift apart.
fn location_radius(kind: LocationKind) -> f32 {
    match kind {
        LocationKind::City => 18.0,
        _ => 12.0,
    }
}

/// Where a formation's marker is drawn relative to its home site: small
/// squares offset from the site so multiple factions holding the same
/// ground are visible as separate marks (the setup for the automatic
/// ant-sim clash).
fn formation_marker_offset(index: usize) -> Vec2 {
    Vec2::new(-20.0 + (index as f32 % 3.0) * 10.0, -20.0 - (index as f32 / 3.0).floor() * 10.0)
}

/// A convoy's current world position, whether sitting at a site or
/// interpolated along its route while en route — real movement, drawn
/// honestly as "on the road" rather than snapped to either endpoint.
/// `None` for a disbanded convoy or one whose site/route no longer resolves.
fn convoy_marker_pos(w: &World, convoy: &Convoy) -> Option<Vec2> {
    match convoy.state {
        ConvoyState::Forming { at } | ConvoyState::Arrived { at } => {
            Some(map_pos(w.location(at)?.position) + Vec2::new(0.0, 20.0))
        }
        ConvoyState::EnRoute { route, departed_day, arrives_day } => {
            let r = w.route(route)?;
            let from = w.location(r.from)?;
            let to = w.location(r.to)?;
            let span = (arrives_day - departed_day).max(1) as f32;
            let progress = ((w.today() - departed_day) as f32 / span).clamp(0.0, 1.0);
            Some(map_pos(from.position).lerp(map_pos(to.position), progress))
        }
        ConvoyState::Disbanded => None,
    }
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
        let color = loc.controller.map(actor_color).unwrap_or(Color::srgb(0.4, 0.4, 0.4));
        gizmos.circle_2d(pos, location_radius(loc.kind), color);
    }

    // Convoys: a small marker at their site, or interpolated along the
    // route while en route.
    for convoy in w.convoys_iter() {
        let Some(pos) = convoy_marker_pos(w, convoy) else { continue };
        gizmos.circle_2d(pos, 5.0, actor_color(convoy.owner));
    }

    // Formations: small squares offset from their home site, one per
    // formation.
    for (i, formation) in w.formations_iter().enumerate() {
        if let Some(at) = formation.current_site() {
            if let Some(loc) = w.location(at) {
                let pos = map_pos(loc.position) + formation_marker_offset(i);
                gizmos.rect_2d(pos, Vec2::splat(8.0), actor_color(formation.owner));
            }
        }
    }
}

/// Cursor -> world-space conversion for the primary camera, used only for
/// hover hit-testing (drawing stays purely in `Gizmos`' own world space).
fn cursor_world_pos(
    windows: &Query<&Window>,
    camera_q: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.iter().next()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = camera_q.iter().next()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

fn actor_name(w: &World, id: ActorId) -> String {
    w.actor(id).map(|a| a.name.clone()).unwrap_or_else(|| format!("actor {id}"))
}

fn formation_tooltip(ui: &mut egui::Ui, w: &World, formation: &Formation) {
    ui.strong(&formation.name);
    ui.label(format!("owner: {}", actor_name(w, formation.owner)));
    if let Some(template) = w.toe_template(formation.template) {
        ui.label(format!("doctrine: {}", template.name));
    }
    ui.label(format!("assets: {}", formation.assets.len()));
    let mut by_design: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for &asset_id in &formation.assets {
        if let Some(asset) = w.asset(asset_id) {
            let name = w.design(asset.design).map(|d| d.name.clone()).unwrap_or_else(|| "unknown design".into());
            *by_design.entry(name).or_insert(0) += 1;
        }
    }
    for (name, count) in by_design {
        ui.label(format!("  {count}x {name}"));
    }
    match formation.state {
        FormationState::Stationed { .. } => {
            ui.label("stationed");
        }
        FormationState::EnRoute { arrives_day, .. } => {
            ui.label(format!("en route, arrives day {arrives_day}"));
        }
    }
}

fn convoy_tooltip(ui: &mut egui::Ui, w: &World, convoy: &Convoy) {
    ui.strong(format!("Convoy {}", convoy.id));
    ui.label(format!("owner: {}", actor_name(w, convoy.owner)));
    match convoy.state {
        ConvoyState::Forming { .. } => {
            ui.label("forming");
        }
        ConvoyState::Arrived { .. } => {
            ui.label("arrived");
        }
        ConvoyState::EnRoute { departed_day, arrives_day, .. } => {
            let span = (arrives_day - departed_day).max(1) as f32;
            let progress = ((w.today() - departed_day) as f32 / span * 100.0).clamp(0.0, 100.0);
            ui.label(format!("en route, {progress:.0}% (arrives day {arrives_day})"));
        }
        ConvoyState::Disbanded => {
            ui.label("disbanded");
        }
    }
    ui.label(format!("vehicles: {}", convoy.vehicles.len()));
    ui.label(format!("guards: {}", convoy.guards.len()));
    ui.label(format!("cargo: {} lots, {} assets", convoy.cargo_lots.len(), convoy.cargo_assets.len()));
}

fn location_tooltip(ui: &mut egui::Ui, w: &World, loc: &adona_sim::locations::Location) {
    ui.strong(&loc.name);
    ui.label(format!("{:?}", loc.kind));
    let owner = loc.controller.map(|id| actor_name(w, id)).unwrap_or_else(|| "unclaimed".into());
    ui.label(format!("controlled by: {owner}"));
    if loc.population > 0 {
        ui.label(format!("population: {} (unrest {}%)", loc.population, loc.unrest_pct));
    }
    if let Some(reserves) = loc.mine_reserves {
        match reserves {
            adona_sim::locations::MineReserves::Infinite => {
                ui.label("mine reserves: infinite");
            }
            adona_sim::locations::MineReserves::Finite { remaining } => {
                ui.label(format!("mine reserves: {remaining} remaining"));
            }
        }
    }
    let mut by_owner: std::collections::BTreeMap<ActorId, u32> = std::collections::BTreeMap::new();
    for formation in w.formations_iter() {
        if formation.current_site() == Some(loc.id) {
            *by_owner.entry(formation.owner).or_insert(0) += formation.assets.len() as u32;
        }
    }
    if !by_owner.is_empty() {
        ui.label("forces present:");
        for (owner, count) in by_owner {
            ui.label(format!("  {}: {count} assets", actor_name(w, owner)));
        }
    }
}

/// Hover hit-testing: convert the cursor to world space and check, in
/// priority order from smallest/most specific to largest, whether it's over
/// a formation marker, a convoy marker, or a site — the first hit wins so a
/// formation square drawn over a city doesn't get shadowed by the city's
/// bigger circle. Shows an egui tooltip with real state, not a static label.
fn hover_tooltip(
    sim: Res<SimWorld>,
    windows: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    let w = &sim.0;
    let Some(cursor) = cursor_world_pos(&windows, &camera_q) else { return };
    let ctx = contexts.ctx_mut();
    let tooltip_layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("adona_hover_layer"));

    for (i, formation) in w.formations_iter().enumerate() {
        let Some(at) = formation.current_site() else { continue };
        let Some(loc) = w.location(at) else { continue };
        let pos = map_pos(loc.position) + formation_marker_offset(i);
        if cursor.distance(pos) <= 8.0 {
            egui::show_tooltip_at_pointer(ctx, tooltip_layer, egui::Id::new("adona_hover"), |ui| {
                formation_tooltip(ui, w, formation);
            });
            return;
        }
    }

    for convoy in w.convoys_iter() {
        let Some(pos) = convoy_marker_pos(w, convoy) else { continue };
        if cursor.distance(pos) <= 6.0 {
            egui::show_tooltip_at_pointer(ctx, tooltip_layer, egui::Id::new("adona_hover"), |ui| {
                convoy_tooltip(ui, w, convoy);
            });
            return;
        }
    }

    for loc in w.locations_iter() {
        let pos = map_pos(loc.position);
        if cursor.distance(pos) <= location_radius(loc.kind) {
            egui::show_tooltip_at_pointer(ctx, tooltip_layer, egui::Id::new("adona_hover"), |ui| {
                location_tooltip(ui, w, loc);
            });
            return;
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
