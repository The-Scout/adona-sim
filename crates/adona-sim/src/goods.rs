//! Commodities and lots.
//!
//! A lot is a provenance-bearing physical batch — never an originless
//! commodity pool. A lot of 42 tons of armor plate exists somewhere, belongs
//! to someone, came from a specific mine/factory/seed source, and can run
//! out. Lots split and merge without losing provenance: lineage links are
//! permanent and old lot records are never deleted, so origin is always
//! resolvable through [`World::root_origins`].

use crate::actors::Credits;
use crate::events::EventKind;
use crate::ids::*;
use crate::locations::{LocationRef, MineReserves};
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitOfMeasure {
    Kilograms,
    Liters,
    Units,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommodityDef {
    pub id: CommodityId,
    pub name: String,
    pub unit: UnitOfMeasure,
    pub tier: u8,
    /// Bootstrap reference price used by civilian demand until real price
    /// discovery exists. TODO(markets): prices from actual supply/demand
    /// history (TradeExecuted events), smoothing, anti-oscillation.
    pub base_price: Credits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum QualityGrade {
    Salvage,
    Poor,
    Standard,
    Fine,
    Exceptional,
}

/// Legal status travels with goods; illegal salvage risk depends on what
/// kind of illegal it is (docket: Contracts / salvage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalStatus {
    Legitimate,
    Stolen,
    RestrictedMilitary,
    FactionProtected,
    Contraband,
    TabooTech,
}

/// Where a root lot physically came from. Ownership changes are events, not
/// origin changes — a purchased lot still originates at its mine or factory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LotOrigin {
    Mined { mine: LocationId },
    Produced { factory: FactoryId, job: ProductionJobId },
    /// Pre-sim historical stock placed by world seeding.
    SeededHistorical { note: String },
    /// Populated by an external simulator or import.
    Imported { source: String },
    /// Explicit admin placement (auditable).
    AdminPlaced { operator: ActorId },
}

/// How this lot record relates to its ancestors. Root lots carry an origin;
/// split/merge children point at their parents, which stay in the table
/// forever, so provenance is never lost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Lineage {
    Root(LotOrigin),
    SplitFrom(LotId),
    MergedFrom(Vec<LotId>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LotState {
    /// In play: owned, located, tradable, consumable.
    Active,
    /// Reserved by a market listing (still physical, still at the market).
    Listed(SellListingId),
    /// Consumed as real production input. Record kept for provenance.
    ConsumedByProduction(ProductionJobId),
    /// Merged into another lot. Record kept for provenance.
    MergedInto(LotId),
    /// Emptied by splits. Record kept for provenance.
    Depleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub id: LotId,
    pub commodity: CommodityId,
    /// For historical states (consumed/merged), this is the quantity at the
    /// time the lot left play.
    pub quantity: u64,
    pub quality: QualityGrade,
    pub legal_status: LegalStatus,
    pub owner: ActorId,
    pub location: LocationRef,
    pub lineage: Lineage,
    pub state: LotState,
    pub created_day: u64,
}

impl World {
    pub fn define_commodity(
        &mut self,
        name: &str,
        unit: UnitOfMeasure,
        tier: u8,
        base_price: Credits,
    ) -> CommodityId {
        let id = CommodityId(self.alloc());
        self.commodities.insert(
            id,
            CommodityDef {
                id,
                name: name.to_string(),
                unit,
                tier,
                base_price,
            },
        );
        self.push_event(EventKind::CommodityDefined { commodity: id });
        id
    }

    /// Internal lot creation — every path that makes goods real goes through
    /// here so the event log always has a `LotCreated`.
    pub(crate) fn create_lot_raw(
        &mut self,
        owner: ActorId,
        commodity: CommodityId,
        quantity: u64,
        quality: QualityGrade,
        legal_status: LegalStatus,
        location: LocationRef,
        lineage: Lineage,
    ) -> Result<LotId, SimError> {
        if quantity == 0 {
            return Err(SimError::InvalidQuantity);
        }
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.commodities.contains_key(&commodity) {
            return Err(SimError::UnknownCommodity(commodity));
        }
        if !self.location_ref_valid(location) {
            return Err(SimError::NotColocated);
        }
        let id = LotId(self.alloc());
        let created_day = self.clock.day;
        self.lots.insert(
            id,
            Lot {
                id,
                commodity,
                quantity,
                quality,
                legal_status,
                owner,
                location,
                lineage,
                state: LotState::Active,
                created_day,
            },
        );
        self.push_event(EventKind::LotCreated {
            lot: id,
            commodity,
            quantity,
            owner,
        });
        Ok(id)
    }

    /// Seed a lot into the world. Only seed-class origins are accepted here;
    /// mined and produced goods must come from `produce_from_mine` and
    /// production jobs — no back door into fake stock.
    pub fn seed_lot(
        &mut self,
        owner: ActorId,
        commodity: CommodityId,
        quantity: u64,
        quality: QualityGrade,
        legal_status: LegalStatus,
        site: LocationId,
        origin: LotOrigin,
    ) -> Result<LotId, SimError> {
        match origin {
            LotOrigin::SeededHistorical { .. }
            | LotOrigin::Imported { .. }
            | LotOrigin::AdminPlaced { .. } => {}
            _ => {
                return Err(SimError::InvalidOrigin(
                    "seed_lot only accepts SeededHistorical/Imported/AdminPlaced origins".into(),
                ))
            }
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        self.create_lot_raw(
            owner,
            commodity,
            quantity,
            quality,
            legal_status,
            LocationRef::Site(site),
            Lineage::Root(origin),
        )
    }

    /// Extract real goods from a mine, respecting its depletion mode.
    /// TODO(production): mines as scheduled producers in `tick` with labor,
    /// equipment, and output rates instead of on-demand extraction.
    pub fn produce_from_mine(
        &mut self,
        mine: LocationId,
        owner: ActorId,
        commodity: CommodityId,
        quantity: u64,
        quality: QualityGrade,
    ) -> Result<LotId, SimError> {
        if quantity == 0 {
            return Err(SimError::InvalidQuantity);
        }
        {
            let loc = self
                .locations
                .get(&mine)
                .ok_or(SimError::UnknownLocation(mine))?;
            match loc.mine_reserves {
                None => return Err(SimError::InvalidLocationKind(mine)),
                Some(MineReserves::Infinite) => {}
                Some(MineReserves::Finite { remaining }) => {
                    if remaining < quantity {
                        return Err(SimError::InsufficientQuantity {
                            commodity,
                            missing: quantity - remaining,
                        });
                    }
                }
            }
        }
        let lot = self.create_lot_raw(
            owner,
            commodity,
            quantity,
            quality,
            LegalStatus::Legitimate,
            LocationRef::Site(mine),
            Lineage::Root(LotOrigin::Mined { mine }),
        )?;
        if let Some(MineReserves::Finite { remaining }) =
            self.locations.get(&mine).and_then(|l| l.mine_reserves)
        {
            self.locations.get_mut(&mine).unwrap().mine_reserves =
                Some(MineReserves::Finite {
                    remaining: remaining.saturating_sub(quantity),
                });
        }
        self.push_event(EventKind::MineYield {
            mine,
            lot,
            quantity,
        });
        Ok(lot)
    }

    /// Real consumption that destroys goods with no production output — a
    /// city's population eating food, not a factory transforming inputs.
    /// Reduces the lot in place and marks it `Depleted` if this exhausts it;
    /// the lineage chain is untouched, so provenance still answers "where
    /// did this come from," this event just records how it left play.
    pub(crate) fn consume_lot_quantity(&mut self, lot: LotId, quantity: u64) -> Result<(), SimError> {
        let (commodity, owner) = {
            let l = self.lots.get_mut(&lot).ok_or(SimError::UnknownLot(lot))?;
            if l.state != LotState::Active {
                return Err(SimError::LotNotActive(lot));
            }
            if quantity == 0 || quantity > l.quantity {
                return Err(SimError::InvalidQuantity);
            }
            l.quantity -= quantity;
            if l.quantity == 0 {
                l.state = LotState::Depleted;
            }
            (l.commodity, l.owner)
        };
        self.push_event(EventKind::GoodsConsumed { lot, commodity, owner, quantity });
        Ok(())
    }

    /// Split `quantity` off an active lot into a new lot. Provenance is
    /// preserved through the lineage link.
    pub fn split_lot(&mut self, lot: LotId, quantity: u64) -> Result<LotId, SimError> {
        let state = &self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?.state;
        if *state != LotState::Active {
            return Err(SimError::LotNotActive(lot));
        }
        self.split_lot_internal(lot, quantity)
    }

    /// Split that also works on listed lots (used by partial market fills;
    /// the child is always Active).
    pub(crate) fn split_lot_internal(&mut self, lot: LotId, quantity: u64) -> Result<LotId, SimError> {
        let (commodity, quality, legal_status, owner, location, parent_qty) = {
            let l = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?;
            (l.commodity, l.quality, l.legal_status, l.owner, l.location, l.quantity)
        };
        if quantity == 0 || quantity >= parent_qty {
            return Err(SimError::InvalidQuantity);
        }
        let child = self.create_lot_raw(
            owner,
            commodity,
            quantity,
            quality,
            legal_status,
            location,
            Lineage::SplitFrom(lot),
        )?;
        let parent = self.lots.get_mut(&lot).unwrap();
        parent.quantity -= quantity;
        self.push_event(EventKind::LotSplit {
            parent: lot,
            child,
            quantity,
        });
        Ok(child)
    }

    /// Merge active lots into one. Commodity, quality, legal status, owner
    /// and location must all match — merging must never launder provenance
    /// or blur quality. The merged lot's lineage records every source.
    pub fn merge_lots(&mut self, sources: &[LotId]) -> Result<LotId, SimError> {
        if sources.len() < 2 {
            return Err(SimError::MergeMismatch("need at least two lots".into()));
        }
        let first = {
            let l = self
                .lots
                .get(&sources[0])
                .ok_or(SimError::UnknownLot(sources[0]))?;
            l.clone()
        };
        let mut total: u64 = 0;
        for id in sources {
            let l = self.lots.get(id).ok_or(SimError::UnknownLot(*id))?;
            if l.state != LotState::Active {
                return Err(SimError::LotNotActive(*id));
            }
            if l.commodity != first.commodity {
                return Err(SimError::MergeMismatch("commodity differs".into()));
            }
            if l.quality != first.quality {
                return Err(SimError::MergeMismatch("quality differs".into()));
            }
            if l.legal_status != first.legal_status {
                return Err(SimError::MergeMismatch("legal status differs".into()));
            }
            if l.owner != first.owner {
                return Err(SimError::MergeMismatch("owner differs".into()));
            }
            if l.location != first.location {
                return Err(SimError::MergeMismatch("location differs".into()));
            }
            total = total.checked_add(l.quantity).ok_or(SimError::Overflow)?;
        }
        // Reject duplicates: merging a lot with itself would double-count.
        let mut sorted = sources.to_vec();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != sources.len() {
            return Err(SimError::MergeMismatch("duplicate source lot".into()));
        }
        let result = self.create_lot_raw(
            first.owner,
            first.commodity,
            total,
            first.quality,
            first.legal_status,
            first.location,
            Lineage::MergedFrom(sources.to_vec()),
        )?;
        for id in sources {
            self.lots.get_mut(id).unwrap().state = LotState::MergedInto(result);
        }
        self.push_event(EventKind::LotsMerged {
            sources: sources.to_vec(),
            result,
        });
        Ok(result)
    }

    /// Resolve a lot's root origins by walking lineage through retired
    /// records. A merged lot reports every root origin it descends from,
    /// in first-encountered order, deduplicated.
    pub fn root_origins(&self, lot: LotId) -> Result<Vec<LotOrigin>, SimError> {
        let mut origins: Vec<LotOrigin> = Vec::new();
        let mut stack = vec![lot];
        let mut visited: Vec<LotId> = Vec::new();
        while let Some(id) = stack.pop() {
            if visited.contains(&id) {
                continue;
            }
            visited.push(id);
            let l = self.lots.get(&id).ok_or(SimError::UnknownLot(id))?;
            match &l.lineage {
                Lineage::Root(origin) => {
                    if !origins.contains(origin) {
                        origins.push(origin.clone());
                    }
                }
                Lineage::SplitFrom(parent) => stack.push(*parent),
                Lineage::MergedFrom(parents) => stack.extend(parents.iter().copied()),
            }
        }
        Ok(origins)
    }

    /// Transfer ownership of an active lot. Physical location is unchanged —
    /// buying goods does not teleport them.
    pub fn transfer_lot(&mut self, lot: LotId, to: ActorId) -> Result<(), SimError> {
        if !self.actors.contains_key(&to) {
            return Err(SimError::UnknownActor(to));
        }
        let from = {
            let l = self.lots.get_mut(&lot).ok_or(SimError::UnknownLot(lot))?;
            if l.state != LotState::Active {
                return Err(SimError::LotNotActive(lot));
            }
            let from = l.owner;
            l.owner = to;
            from
        };
        self.push_event(EventKind::LotOwnerChanged { lot, from, to });
        Ok(())
    }

    /// Explicit admin teleport, recorded as an admin edit. Normal goods move
    /// by convoy; this exists for operators, debugging, and world repair.
    pub fn admin_move_lot(
        &mut self,
        lot: LotId,
        to: LocationRef,
        operator: Option<ActorId>,
    ) -> Result<(), SimError> {
        if !self.location_ref_valid(to) {
            return Err(SimError::NotColocated);
        }
        let from = {
            let l = self.lots.get_mut(&lot).ok_or(SimError::UnknownLot(lot))?;
            let from = l.location;
            l.location = to;
            from
        };
        self.push_event(EventKind::LotMoved { lot, from, to });
        self.push_event(EventKind::AdminEdit {
            operator,
            description: format!("admin moved {lot}"),
        });
        Ok(())
    }

    /// Internal physical move (convoy load/unload, stockpile deposit…).
    pub(crate) fn move_lot_raw(&mut self, lot: LotId, to: LocationRef) -> Result<(), SimError> {
        let from = {
            let l = self.lots.get_mut(&lot).ok_or(SimError::UnknownLot(lot))?;
            let from = l.location;
            l.location = to;
            from
        };
        self.push_event(EventKind::LotMoved { lot, from, to });
        Ok(())
    }
}
