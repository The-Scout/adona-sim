//! Faction stockpiles: physical warehouses, possibly sequestered.
//!
//! A stockpile is a real warehouse at a real site. Goods deposited into a
//! sequestered stockpile cannot be listed on markets until withdrawn —
//! sequestration is a physical/legal state, not a UI flag. Warehouses are
//! real targets: raiding, capture, and destruction are TODO(war) but the
//! physical model already supports "everything in stockpile X changes hands".

use crate::events::EventKind;
use crate::goods::LotState;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stockpile {
    pub id: StockpileId,
    pub owner: ActorId,
    pub site: LocationId,
    /// Sequestered gear is out of the open economy until deliberately
    /// released, sold, transferred, lost, or captured.
    pub sequestered: bool,
}

impl World {
    pub fn create_stockpile(
        &mut self,
        owner: ActorId,
        site: LocationId,
        sequestered: bool,
    ) -> Result<StockpileId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        let id = StockpileId(self.alloc());
        self.stockpiles.insert(
            id,
            Stockpile {
                id,
                owner,
                site,
                sequestered,
            },
        );
        self.push_event(EventKind::StockpileCreated { stockpile: id });
        Ok(id)
    }

    /// Deposit an active lot into a stockpile. Must be owned by the
    /// stockpile owner and physically at the stockpile's site.
    pub fn deposit_lot(&mut self, lot: LotId, stockpile: StockpileId) -> Result<(), SimError> {
        let sp = self
            .stockpiles
            .get(&stockpile)
            .ok_or(SimError::UnknownStockpile(stockpile))?
            .clone();
        let (owner, state, location) = {
            let l = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?;
            (l.owner, l.state.clone(), l.location)
        };
        if state != LotState::Active {
            return Err(SimError::LotNotActive(lot));
        }
        if owner != sp.owner {
            return Err(SimError::NotOwner { actor: owner });
        }
        if self.resolve_site(location) != Some(sp.site) {
            return Err(SimError::NotColocated);
        }
        self.move_lot_raw(lot, LocationRef::Stockpile(stockpile))
    }

    /// Withdraw a lot from a stockpile back to the open site.
    pub fn withdraw_lot(&mut self, lot: LotId) -> Result<(), SimError> {
        let location = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?.location;
        let LocationRef::Stockpile(sp) = location else {
            return Err(SimError::InvalidState("lot is not in a stockpile".into()));
        };
        let site = self
            .stockpiles
            .get(&sp)
            .ok_or(SimError::UnknownStockpile(sp))?
            .site;
        self.move_lot_raw(lot, LocationRef::Site(site))
    }

    /// Deposit a serial asset into a stockpile.
    pub fn deposit_asset(&mut self, asset: AssetId, stockpile: StockpileId) -> Result<(), SimError> {
        let sp = self
            .stockpiles
            .get(&stockpile)
            .ok_or(SimError::UnknownStockpile(stockpile))?
            .clone();
        let (owner, location) = {
            let a = self.assets.get(&asset).ok_or(SimError::UnknownAsset(asset))?;
            (a.owner, a.location)
        };
        if owner != sp.owner {
            return Err(SimError::NotOwner { actor: owner });
        }
        if self.resolve_site(location) != Some(sp.site) {
            return Err(SimError::NotColocated);
        }
        self.move_asset_raw(asset, LocationRef::Stockpile(stockpile))
    }

    /// Withdraw a serial asset from a stockpile back to the open site.
    pub fn withdraw_asset(&mut self, asset: AssetId) -> Result<(), SimError> {
        let location = self
            .assets
            .get(&asset)
            .ok_or(SimError::UnknownAsset(asset))?
            .location;
        let LocationRef::Stockpile(sp) = location else {
            return Err(SimError::InvalidState("asset is not in a stockpile".into()));
        };
        let site = self
            .stockpiles
            .get(&sp)
            .ok_or(SimError::UnknownStockpile(sp))?
            .site;
        self.move_asset_raw(asset, LocationRef::Site(site))
    }
}
