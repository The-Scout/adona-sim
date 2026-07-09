//! Locations: physical places and the holding hierarchy.
//!
//! Hard invariant: every physical lot, asset, and loose component has exactly
//! one current holder, expressed as a [`LocationRef`]. Holders can nest
//! (a lot in a container, the container on a convoy), but everything resolves
//! to at most one physical site at a time — or to none, when the holder is a
//! convoy on the road.

use crate::actors::Credits;
use crate::events::EventKind;
use crate::goods::{LegalStatus, Lineage, LotOrigin, LotState, QualityGrade};
use crate::ids::*;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocationKind {
    City,
    Mine,
    Refinery,
    FactorySite,
    Warehouse,
    Depot,
    Port,
    MilitaryBase,
    Battlefield,
    Ruin,
    /// Explicit admin holding area — admin placement is real and auditable,
    /// never silent mutation.
    AdminHolding,
}

/// Mine depletion modes per the docket: infinite, finite, finite-but-huge
/// (which is just a large finite reserve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MineReserves {
    Infinite,
    Finite { remaining: u64 },
}

/// One independent automatic-production line at a location: a commodity, a
/// daily quantity, and its own real depletion tracking (separate from the
/// location's `mine_reserves`, which is a distinct, on-demand-only pool).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationYield {
    pub commodity: CommodityId,
    pub quantity_per_day: u64,
    pub reserves: MineReserves,
}

/// A daily civilian demand line for a city. Cities with population and needs
/// generate real budget-constrained buy orders at their local market each
/// tick — demand is real orders, not an abstract modifier. [`World::tick_population`]
/// then actually consumes (destroys) what was bought, tracks unrest from
/// shortfall, grows or shrinks population, and credits per-capita taxation.
/// TODO(population): recruitment hooks into pilot/crew generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CivilianNeed {
    pub commodity: CommodityId,
    pub quantity_per_day: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub id: LocationId,
    pub name: String,
    pub kind: LocationKind,
    /// Abstract map grid for now. Routes carry real travel time; positions
    /// exist so later systems (detection ranges, route generation) have a
    /// physical anchor. TODO(map): real geography.
    pub position: (i64, i64),
    pub population: u64,
    /// The actor that buys on behalf of the civilian population.
    pub authority: Option<ActorId>,
    pub civilian_needs: Vec<CivilianNeed>,
    /// Per-capita daily tax revenue credited to `authority` (docket
    /// candidate money source: taxation). Zero means no taxation is
    /// configured for this city.
    pub tax_rate_per_capita: Credits,
    /// 0-100. Climbs when civilian needs go unmet, decays when they are
    /// satisfied. Suppresses population growth above a threshold — the
    /// urgency signal the docket's price-feedback question was asking for.
    pub unrest_pct: u32,
    /// `Some` only for kind == Mine. Gates `World::produce_from_mine`
    /// (on-demand extraction), independent of `yields` below.
    pub mine_reserves: Option<MineReserves>,
    /// Automatic daily production (docket TODO(production): "mines as
    /// scheduled producers in tick with labor, equipment, and output rates
    /// instead of on-demand extraction"). A location can hold any number of
    /// independent yields — e.g. a faction's capital producing several raw
    /// materials for its own industry at once — each with its own real
    /// depletion tracking. Not restricted to `LocationKind::Mine`: what a
    /// site is *labeled* and what it can produce are independent questions.
    pub yields: Vec<LocationYield>,
    /// The faction currently holding this site, if any. Combat is the only
    /// thing that changes this once set (docket: faction-war territory
    /// control); it starts unclaimed.
    pub controller: Option<ActorId>,
    /// The day `controller` last changed hands. Feeds the entrenchment bonus
    /// in `combat::resolve_battle` — ground held longer is dug in deeper.
    pub controlled_since: u64,
}

/// Where a physical thing currently is. Exactly one of these per lot/asset/
/// loose component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocationRef {
    /// At a site, in the open local economy of that site.
    Site(LocationId),
    /// Inside a specific stockpile warehouse (which is itself at a site).
    Stockpile(StockpileId),
    /// Aboard a convoy (directly, e.g. bulk cargo strapped to a vehicle).
    Convoy(ConvoyId),
    /// Inside a serial cargo container (which itself is somewhere).
    Container(AssetId),
    /// Assigned to a military formation.
    Formation(FormationId),
}

impl World {
    pub fn create_location(&mut self, name: &str, kind: LocationKind, position: (i64, i64)) -> LocationId {
        let id = LocationId(self.alloc());
        let mine_reserves = if kind == LocationKind::Mine {
            Some(MineReserves::Infinite)
        } else {
            None
        };
        self.locations.insert(
            id,
            Location {
                id,
                name: name.to_string(),
                kind,
                position,
                population: 0,
                authority: None,
                civilian_needs: Vec::new(),
                tax_rate_per_capita: 0,
                unrest_pct: 0,
                mine_reserves,
                yields: Vec::new(),
                controller: None,
                controlled_since: 0,
            },
        );
        self.push_event(EventKind::LocationCreated { location: id });
        id
    }

    /// Add an automatic daily production line to a location (see
    /// `tick_location_yields`): every day it produces `quantity_per_day` of
    /// `commodity` for its controller, depleting `reserves` independently of
    /// any other yield at the same site or the location's own
    /// `mine_reserves`. Not restricted to `LocationKind::Mine` — a
    /// location's real productive capacity is data, not implied by its
    /// label. A location can hold any number of these (e.g. a capital city
    /// producing several raw materials for its own industry at once).
    pub fn add_location_yield(
        &mut self,
        location: LocationId,
        commodity: CommodityId,
        quantity_per_day: u64,
        reserves: MineReserves,
    ) -> Result<(), SimError> {
        if !self.commodities.contains_key(&commodity) {
            return Err(SimError::UnknownCommodity(commodity));
        }
        let loc = self.locations.get_mut(&location).ok_or(SimError::UnknownLocation(location))?;
        loc.yields.push(LocationYield { commodity, quantity_per_day, reserves });
        self.push_event(EventKind::AdminEdit {
            operator: None,
            description: format!("added a {quantity_per_day}/day yield to {location}"),
        });
        Ok(())
    }

    /// Automatic daily production: every location with a real controller
    /// and at least one configured yield produces real commodity for that
    /// controller from each yield line, respecting that line's own `Finite`
    /// depletion. Unclaimed locations and locations with no configured
    /// yield produce nothing on their own.
    pub(crate) fn tick_location_yields(&mut self) {
        let due: Vec<(LocationId, ActorId, usize, CommodityId, u64)> = self
            .locations
            .values()
            .filter_map(|l| {
                let controller = l.controller?;
                Some(l.yields.iter().enumerate().filter_map(move |(i, y)| {
                    let has_capacity = match y.reserves {
                        MineReserves::Infinite => true,
                        MineReserves::Finite { remaining } => remaining >= y.quantity_per_day,
                    };
                    has_capacity.then_some((l.id, controller, i, y.commodity, y.quantity_per_day))
                }))
            })
            .flatten()
            .collect();

        for (location, controller, index, commodity, quantity) in due {
            let lot = self.create_lot_raw(
                controller,
                commodity,
                quantity,
                QualityGrade::Standard,
                LegalStatus::Legitimate,
                LocationRef::Site(location),
                Lineage::Root(LotOrigin::Mined { mine: location }),
            );
            let Ok(lot) = lot else { continue };
            if let MineReserves::Finite { remaining } = self.locations[&location].yields[index].reserves {
                self.locations.get_mut(&location).unwrap().yields[index].reserves =
                    MineReserves::Finite { remaining: remaining - quantity };
            }
            self.push_event(EventKind::MineYield { mine: location, lot, quantity });
        }
    }

    /// Set population, purchasing authority, and daily civilian needs for a
    /// city. Recorded as an explicit admin edit.
    pub fn configure_city(
        &mut self,
        city: LocationId,
        population: u64,
        authority: Option<ActorId>,
        needs: Vec<CivilianNeed>,
    ) -> Result<(), crate::SimError> {
        if let Some(a) = authority {
            if !self.actors.contains_key(&a) {
                return Err(crate::SimError::UnknownActor(a));
            }
        }
        let loc = self
            .locations
            .get_mut(&city)
            .ok_or(crate::SimError::UnknownLocation(city))?;
        loc.population = population;
        loc.authority = authority;
        loc.civilian_needs = needs;
        self.push_event(EventKind::AdminEdit {
            operator: None,
            description: format!("configured city {city}: population {population}"),
        });
        Ok(())
    }

    /// Set a city's per-capita tax rate credited to its authority each day.
    pub fn set_tax_rate(&mut self, city: LocationId, rate_per_capita: Credits) -> Result<(), SimError> {
        let loc = self.locations.get_mut(&city).ok_or(SimError::UnknownLocation(city))?;
        loc.tax_rate_per_capita = rate_per_capita;
        self.push_event(EventKind::AdminEdit {
            operator: None,
            description: format!("set {city} tax rate to {rate_per_capita}/capita"),
        });
        Ok(())
    }

    /// Population phase: real consumption that destroys real stock, unrest
    /// from unmet need, population growth/decline, and per-capita taxation.
    /// Runs after civilian demand and market matching so today's purchases
    /// are on hand to actually eat (docket TODO(population)).
    pub(crate) fn tick_population(&mut self) {
        let cities: Vec<LocationId> = self
            .locations
            .iter()
            .filter(|(_, l)| l.population > 0)
            .map(|(id, _)| *id)
            .collect();

        for city in cities {
            let (authority, needs, population, tax_rate, unrest) = {
                let l = &self.locations[&city];
                (l.authority, l.civilian_needs.clone(), l.population, l.tax_rate_per_capita, l.unrest_pct)
            };

            let mut any_shortfall = !needs.is_empty() && authority.is_none();
            if let Some(authority) = authority {
                for need in &needs {
                    let mut remaining = need.quantity_per_day;
                    let candidate_lots: Vec<LotId> = self
                        .lots
                        .iter()
                        .filter(|(_, l)| {
                            l.owner == authority
                                && l.commodity == need.commodity
                                && l.state == LotState::Active
                                && self.resolve_site(l.location) == Some(city)
                        })
                        .map(|(id, _)| *id)
                        .collect();
                    for lid in candidate_lots {
                        if remaining == 0 {
                            break;
                        }
                        let have = self.lots[&lid].quantity;
                        let take = remaining.min(have);
                        if take > 0 {
                            self.consume_lot_quantity(lid, take).expect("checked colocated active lot");
                            remaining -= take;
                        }
                    }
                    if remaining > 0 {
                        any_shortfall = true;
                    }
                }
            }

            let new_unrest = if any_shortfall { (unrest + 10).min(100) } else { unrest.saturating_sub(5) };

            // Modest daily growth when content; decline under sustained
            // unrest. Basis-point integer math, no floats anywhere in state.
            let growth_bp: i64 = if new_unrest > 30 { -20 } else { 5 };
            let delta = (population as i64 * growth_bp) / 10_000;
            let new_population = (population as i64 + delta).max(0) as u64;

            if let Some(authority) = authority {
                if tax_rate > 0 {
                    let revenue = (population as i64).saturating_mul(tax_rate);
                    if revenue > 0 {
                        // A broke tax base is not a bug; there is nothing to
                        // fail here since this issues money rather than
                        // moving it, but keep the call fallible-safe anyway.
                        let _ = self.issue_money(authority, revenue);
                    }
                }
            }

            let loc = self.locations.get_mut(&city).unwrap();
            loc.unrest_pct = new_unrest;
            loc.population = new_population;
        }
    }

    /// Seed a site's initial controlling faction (front lines at world
    /// start). After this, only combat changes control.
    pub fn set_territory_controller(
        &mut self,
        site: LocationId,
        controller: Option<ActorId>,
    ) -> Result<(), SimError> {
        if let Some(owner) = controller {
            if !self.actors.contains_key(&owner) {
                return Err(SimError::UnknownActor(owner));
            }
        }
        let day = self.clock.day;
        let loc = self.locations.get_mut(&site).ok_or(SimError::UnknownLocation(site))?;
        loc.controller = controller;
        loc.controlled_since = day;
        self.push_event(EventKind::AdminEdit {
            operator: None,
            description: format!("set {site} controller to {controller:?}"),
        });
        Ok(())
    }

    /// Set a mine's depletion mode. Recorded as an explicit admin edit.
    pub fn configure_mine(
        &mut self,
        mine: LocationId,
        reserves: MineReserves,
    ) -> Result<(), crate::SimError> {
        let loc = self
            .locations
            .get_mut(&mine)
            .ok_or(crate::SimError::UnknownLocation(mine))?;
        if loc.kind != LocationKind::Mine {
            return Err(crate::SimError::InvalidLocationKind(mine));
        }
        loc.mine_reserves = Some(reserves);
        self.push_event(EventKind::AdminEdit {
            operator: None,
            description: format!("configured mine {mine} reserves"),
        });
        Ok(())
    }

    /// Resolve a holder chain to the physical site it is currently at, if it
    /// is at one. A convoy en route resolves to `None`: the thing is real but
    /// on the road, not at any site.
    pub fn resolve_site(&self, loc: LocationRef) -> Option<LocationId> {
        let mut current = loc;
        // Holder chains are short (lot -> container -> convoy); the depth
        // bound only guards against a corrupted cyclic chain.
        for _ in 0..16 {
            match current {
                LocationRef::Site(site) => return Some(site),
                LocationRef::Stockpile(s) => {
                    return self.stockpiles.get(&s).map(|sp| sp.site);
                }
                LocationRef::Convoy(c) => {
                    return self.convoys.get(&c).and_then(|cv| cv.current_site());
                }
                LocationRef::Container(a) => {
                    current = self.assets.get(&a)?.location;
                }
                LocationRef::Formation(fm) => {
                    return self.formations.get(&fm).and_then(|f| f.current_site());
                }
            }
        }
        None
    }

    /// True if the holder referenced actually exists.
    pub(crate) fn location_ref_valid(&self, loc: LocationRef) -> bool {
        match loc {
            LocationRef::Site(id) => self.locations.contains_key(&id),
            LocationRef::Stockpile(id) => self.stockpiles.contains_key(&id),
            LocationRef::Convoy(id) => self.convoys.contains_key(&id),
            LocationRef::Container(id) => self.assets.contains_key(&id),
            LocationRef::Formation(id) => self.formations.contains_key(&id),
        }
    }

    /// World-generation phase: a small daily chance that prospectors turn up
    /// a new, unclaimed mine somewhere near the existing map, wired into the
    /// route network so it is real ground factions can actually reach and
    /// fight over — never a mine that only exists as a name in a list.
    /// TODO(worldgen): terrain-aware placement once the map is more than an
    /// abstract grid (see `Location::position`'s own TODO).
    pub(crate) fn tick_mine_discovery(&mut self) {
        const DISCOVERY_CHANCE_PCT: u8 = 3;
        const INFINITE_CHANCE_PCT: u8 = 10;
        if self.locations.is_empty() {
            return;
        }
        if self.rng.roll_percent() >= DISCOVERY_CHANCE_PCT {
            return;
        }

        let (min_x, max_x, min_y, max_y) = self.locations.values().fold(
            (i64::MAX, i64::MIN, i64::MAX, i64::MIN),
            |(min_x, max_x, min_y, max_y), l| {
                let (x, y) = l.position;
                (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
            },
        );
        // Sample within the existing map's bounding box plus a margin so
        // discoveries land near the known world instead of arbitrarily far
        // from anything reachable.
        let margin = 10;
        let span_x = (max_x - min_x + 2 * margin).max(1) as u64;
        let span_y = (max_y - min_y + 2 * margin).max(1) as u64;
        let x = min_x - margin + self.rng.roll_range(span_x) as i64;
        let y = min_y - margin + self.rng.roll_range(span_y) as i64;

        let nearest = self
            .locations
            .values()
            .min_by_key(|l| chebyshev_distance(l.position, (x, y)))
            .map(|l| (l.id, l.position))
            .expect("checked locations is non-empty above");
        let distance_days = (chebyshev_distance(nearest.1, (x, y)) / 4).max(1);

        let name = format!("Prospect Site {}", self.next_id);
        let mine = self.create_location(&name, LocationKind::Mine, (x, y));

        let reserves = if self.rng.roll_percent() < INFINITE_CHANCE_PCT {
            MineReserves::Infinite
        } else {
            MineReserves::Finite { remaining: 5_000 + self.rng.roll_range(45_000) }
        };
        self.configure_mine(mine, reserves).expect("mine was just created");

        // Connect both directions: convoys and marching formations only
        // travel along a route from their current site, so a one-way link
        // would strand traffic in one direction.
        let _ = self.create_route(mine, nearest.0, distance_days);
        let _ = self.create_route(nearest.0, mine, distance_days);

        self.push_event(EventKind::MineDiscovered { mine });
    }
}

fn chebyshev_distance(a: (i64, i64), b: (i64, i64)) -> u64 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}
