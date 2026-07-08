//! Convoys: first-class physical transport.
//!
//! Every physical movement step needs transport — mine to market, market to
//! refinery, factory to stockpile, stockpile to front. A convoy is real
//! vehicles (serial assets) carrying real cargo (lots and serial assets),
//! guarded by real guards. Nothing in a convoy is decorative. Cargo can only
//! be loaded where the convoy physically is; while the convoy is en route
//! the cargo is at no site — it is on the road and can be observed, raided,
//! or lost.
//!
//! TODO(war): interception, convoy battles, loot/capture/abandon outcomes
//! rippling into markets and production (docket: Convoys).
//! TODO(logistics): fuel and supplies consumed per travel day.
//!
//! [`World::tick_quarter_convoy_contacts`] is the sub-day interception
//! window the docket's Time Model calls for: every quarter a convoy spends
//! en route, there is a real chance it gets spotted, generating a genuine
//! (if approximate) intel observation rather than perfect fog of war.

use crate::assets::AssetKind;
use crate::events::EventKind;
use crate::goods::LotState;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub id: RouteId,
    pub from: LocationId,
    pub to: LocationId,
    pub distance_days: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConvoyState {
    /// Assembling at a site; cargo can be loaded and unloaded.
    Forming { at: LocationId },
    /// On the road. Cargo is physically aboard and at no site.
    EnRoute {
        route: RouteId,
        departed_day: u64,
        arrives_day: u64,
    },
    /// At the destination; cargo can be unloaded, convoy can re-depart.
    Arrived { at: LocationId },
    /// Dissolved; vehicles and cargo were returned to the site.
    Disbanded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Convoy {
    pub id: ConvoyId,
    pub owner: ActorId,
    /// Transport vehicles (serial assets, kind Vehicle).
    pub vehicles: Vec<AssetId>,
    /// Guard assets (mechs/vehicles). Real equipment from real inventories.
    pub guards: Vec<AssetId>,
    /// Bulk cargo aboard.
    pub cargo_lots: Vec<LotId>,
    /// Serial cargo aboard (containers, crated mechs, weapons…).
    pub cargo_assets: Vec<AssetId>,
    pub state: ConvoyState,
    pub formed_day: u64,
}

impl Convoy {
    /// The site the convoy is currently at, if it is at one.
    pub fn current_site(&self) -> Option<LocationId> {
        match self.state {
            ConvoyState::Forming { at } | ConvoyState::Arrived { at } => Some(at),
            ConvoyState::EnRoute { .. } | ConvoyState::Disbanded => None,
        }
    }
}

impl World {
    pub fn create_route(
        &mut self,
        from: LocationId,
        to: LocationId,
        distance_days: u64,
    ) -> Result<RouteId, SimError> {
        if !self.locations.contains_key(&from) {
            return Err(SimError::UnknownLocation(from));
        }
        if !self.locations.contains_key(&to) {
            return Err(SimError::UnknownLocation(to));
        }
        if distance_days == 0 || from == to {
            return Err(SimError::InvalidQuantity);
        }
        let id = RouteId(self.alloc());
        self.routes.insert(
            id,
            Route {
                id,
                from,
                to,
                distance_days,
            },
        );
        self.push_event(EventKind::RouteCreated { route: id });
        Ok(id)
    }

    /// Form a convoy at a site from real vehicles the owner has there.
    pub fn form_convoy(
        &mut self,
        owner: ActorId,
        at: LocationId,
        vehicles: &[AssetId],
    ) -> Result<ConvoyId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.locations.contains_key(&at) {
            return Err(SimError::UnknownLocation(at));
        }
        if vehicles.is_empty() {
            return Err(SimError::InvalidState("a convoy needs at least one vehicle".into()));
        }
        for &v in vehicles {
            let kind = self.asset_kind(v)?;
            if kind != AssetKind::Vehicle && kind != AssetKind::Mech {
                return Err(SimError::InvalidAssetKind(v));
            }
            let a = &self.assets[&v];
            if a.owner != owner {
                return Err(SimError::NotOwner { actor: a.owner });
            }
            if self.resolve_site(a.location) != Some(at) {
                return Err(SimError::NotColocated);
            }
        }
        let id = ConvoyId(self.alloc());
        let formed_day = self.clock.day;
        self.convoys.insert(
            id,
            Convoy {
                id,
                owner,
                vehicles: vehicles.to_vec(),
                guards: Vec::new(),
                cargo_lots: Vec::new(),
                cargo_assets: Vec::new(),
                state: ConvoyState::Forming { at },
                formed_day,
            },
        );
        for &v in vehicles {
            self.move_asset_raw(v, LocationRef::Convoy(id))?;
        }
        self.push_event(EventKind::ConvoyFormed { convoy: id, at });
        Ok(id)
    }

    /// Assign a real guard asset (mech or vehicle) to a convoy at a site.
    pub fn assign_guard(&mut self, convoy: ConvoyId, asset: AssetId) -> Result<(), SimError> {
        let (owner, at) = self.convoy_site(convoy)?;
        let kind = self.asset_kind(asset)?;
        if kind != AssetKind::Vehicle && kind != AssetKind::Mech {
            return Err(SimError::InvalidAssetKind(asset));
        }
        let a = &self.assets[&asset];
        if a.owner != owner {
            return Err(SimError::NotOwner { actor: a.owner });
        }
        if self.resolve_site(a.location) != Some(at) {
            return Err(SimError::NotColocated);
        }
        self.move_asset_raw(asset, LocationRef::Convoy(convoy))?;
        self.convoys.get_mut(&convoy).unwrap().guards.push(asset);
        self.push_event(EventKind::GuardAssigned { convoy, asset });
        Ok(())
    }

    /// Load a real lot aboard. The lot must exist, be active (not reserved
    /// by a listing or job), be owned by the convoy owner, and be physically
    /// at the convoy's current site.
    /// TODO(contracts): hauling other actors' cargo under a haul contract.
    pub fn load_lot_onto_convoy(&mut self, convoy: ConvoyId, lot: LotId) -> Result<(), SimError> {
        let (owner, at) = self.convoy_site(convoy)?;
        let (lot_owner, state, location) = {
            let l = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?;
            (l.owner, l.state.clone(), l.location)
        };
        if state != LotState::Active {
            return Err(SimError::LotNotActive(lot));
        }
        if lot_owner != owner {
            return Err(SimError::NotOwner { actor: lot_owner });
        }
        if self.resolve_site(location) != Some(at) {
            return Err(SimError::NotColocated);
        }
        self.move_lot_raw(lot, LocationRef::Convoy(convoy))?;
        self.convoys.get_mut(&convoy).unwrap().cargo_lots.push(lot);
        self.push_event(EventKind::CargoLotLoaded { convoy, lot });
        Ok(())
    }

    /// Load a serial asset aboard as cargo.
    pub fn load_asset_onto_convoy(&mut self, convoy: ConvoyId, asset: AssetId) -> Result<(), SimError> {
        let (owner, at) = self.convoy_site(convoy)?;
        let a = self.assets.get(&asset).ok_or(SimError::UnknownAsset(asset))?;
        if a.owner != owner {
            return Err(SimError::NotOwner { actor: a.owner });
        }
        if self.resolve_site(a.location) != Some(at) {
            return Err(SimError::NotColocated);
        }
        if matches!(a.location, LocationRef::Convoy(_)) {
            return Err(SimError::InvalidState("asset is already aboard a convoy".into()));
        }
        self.move_asset_raw(asset, LocationRef::Convoy(convoy))?;
        self.convoys.get_mut(&convoy).unwrap().cargo_assets.push(asset);
        self.push_event(EventKind::CargoAssetLoaded { convoy, asset });
        Ok(())
    }

    /// Depart along a route. The convoy must be at the route's origin.
    pub fn depart_convoy(&mut self, convoy: ConvoyId, route: RouteId) -> Result<(), SimError> {
        let (_, at) = self.convoy_site(convoy)?;
        let r = self
            .routes
            .get(&route)
            .ok_or(SimError::UnknownRoute(route))?
            .clone();
        if r.from != at {
            return Err(SimError::NotColocated);
        }
        let departed_day = self.clock.day;
        let arrives_day = departed_day + r.distance_days;
        self.convoys.get_mut(&convoy).unwrap().state = ConvoyState::EnRoute {
            route,
            departed_day,
            arrives_day,
        };
        self.push_event(EventKind::ConvoyDeparted {
            convoy,
            route,
            arrives_day,
        });
        Ok(())
    }

    /// Unload a lot at the convoy's current site.
    pub fn unload_lot(&mut self, convoy: ConvoyId, lot: LotId) -> Result<(), SimError> {
        let (_, at) = self.convoy_site(convoy)?;
        let c = self.convoys.get_mut(&convoy).unwrap();
        let Some(pos) = c.cargo_lots.iter().position(|l| *l == lot) else {
            return Err(SimError::InvalidState("lot is not aboard this convoy".into()));
        };
        c.cargo_lots.remove(pos);
        self.move_lot_raw(lot, LocationRef::Site(at))?;
        self.push_event(EventKind::CargoLotUnloaded { convoy, lot, at });
        Ok(())
    }

    /// Unload a serial asset at the convoy's current site.
    pub fn unload_asset(&mut self, convoy: ConvoyId, asset: AssetId) -> Result<(), SimError> {
        let (_, at) = self.convoy_site(convoy)?;
        let c = self.convoys.get_mut(&convoy).unwrap();
        let Some(pos) = c.cargo_assets.iter().position(|a| *a == asset) else {
            return Err(SimError::InvalidState("asset is not aboard this convoy".into()));
        };
        c.cargo_assets.remove(pos);
        self.move_asset_raw(asset, LocationRef::Site(at))?;
        self.push_event(EventKind::CargoAssetUnloaded { convoy, asset, at });
        Ok(())
    }

    /// Disband a convoy at a site: all cargo, guards, and vehicles return to
    /// the open site.
    pub fn disband_convoy(&mut self, convoy: ConvoyId) -> Result<(), SimError> {
        let (_, at) = self.convoy_site(convoy)?;
        let c = self.convoys.get(&convoy).unwrap().clone();
        for lot in c.cargo_lots {
            self.move_lot_raw(lot, LocationRef::Site(at))?;
            self.push_event(EventKind::CargoLotUnloaded { convoy, lot, at });
        }
        for asset in c.cargo_assets.iter().chain(&c.guards).chain(&c.vehicles) {
            self.move_asset_raw(*asset, LocationRef::Site(at))?;
        }
        let c = self.convoys.get_mut(&convoy).unwrap();
        c.cargo_lots.clear();
        c.cargo_assets.clear();
        c.guards.clear();
        c.vehicles.clear();
        c.state = ConvoyState::Disbanded;
        self.push_event(EventKind::ConvoyDisbanded { convoy, at });
        Ok(())
    }

    /// Sub-day interception window: for every convoy currently en route,
    /// roll a real chance it is spotted this quarter. A contact records a
    /// genuine intel observation (anonymous — nobody in particular is
    /// credited as the observer) at the convoy's last known site, not a
    /// perfect read of its live position; that is the "on the road" fog the
    /// docket wants preserved even when contact happens.
    pub(crate) fn tick_quarter_convoy_contacts(&mut self) {
        const CONTACT_CHANCE_PCT: u8 = 15;
        let en_route: Vec<(ConvoyId, LocationId)> = self
            .convoys
            .iter()
            .filter_map(|(id, c)| match c.state {
                ConvoyState::EnRoute { route, .. } => self.routes.get(&route).map(|r| (*id, r.from)),
                _ => None,
            })
            .collect();
        for (convoy, last_known_site) in en_route {
            if self.rng.roll_percent() < CONTACT_CHANCE_PCT {
                let _ = self.record_observation(
                    None,
                    crate::intel::IntelSubject::Convoy(convoy),
                    last_known_site,
                    "contact: spotted en route",
                    60,
                    20,
                );
            }
        }
    }

    /// Advance convoys: arrivals fire when the clock reaches the arrival day.
    pub(crate) fn tick_convoys(&mut self) {
        let today = self.clock.day;
        let arriving: Vec<(ConvoyId, LocationId)> = self
            .convoys
            .iter()
            .filter_map(|(id, c)| match c.state {
                ConvoyState::EnRoute { route, arrives_day, .. } if arrives_day <= today => {
                    self.routes.get(&route).map(|r| (*id, r.to))
                }
                _ => None,
            })
            .collect();
        for (convoy, at) in arriving {
            self.convoys.get_mut(&convoy).unwrap().state = ConvoyState::Arrived { at };
            self.push_event(EventKind::ConvoyArrived { convoy, at });
        }
    }

    /// Owner and current site of a convoy that is at a site (Forming or
    /// Arrived); error if en route or disbanded.
    fn convoy_site(&self, convoy: ConvoyId) -> Result<(ActorId, LocationId), SimError> {
        let c = self
            .convoys
            .get(&convoy)
            .ok_or(SimError::UnknownConvoy(convoy))?;
        match c.current_site() {
            Some(at) => Ok((c.owner, at)),
            None => Err(SimError::ConvoyNotAtSite(convoy)),
        }
    }
}
