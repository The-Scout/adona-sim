//! Simulation errors.
//!
//! Errors are how the "everything important is real" axiom bites: you cannot
//! list goods that are not physically at the market, load cargo that is not
//! at the convoy's site, consume inputs that do not exist, or spend money a
//! treasury does not hold.

use crate::assets::ComponentCategory;
use crate::assets::AssetKind;
use crate::ids::*;
use crate::toe::ToeShortage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SimError {
    UnknownActor(ActorId),
    UnknownLocation(LocationId),
    UnknownCommodity(CommodityId),
    UnknownLot(LotId),
    UnknownDesign(DesignId),
    UnknownAsset(AssetId),
    UnknownComponentDef(ComponentDefId),
    UnknownComponent(ComponentId),
    UnknownMarket(MarketId),
    UnknownListing(SellListingId),
    UnknownOrder(BuyOrderId),
    UnknownStockpile(StockpileId),
    UnknownFactory(FactoryId),
    UnknownRecipe(RecipeId),
    UnknownJob(ProductionJobId),
    UnknownRoute(RouteId),
    UnknownConvoy(ConvoyId),
    UnknownIntel(IntelId),
    UnknownContract(ContractId),
    UnknownToeTemplate(ToeTemplateId),
    UnknownFormation(FormationId),

    /// The acting actor does not own the thing it is trying to use.
    NotOwner { actor: ActorId },
    /// Two things that must be physically together are not.
    NotColocated,
    /// The lot is not in the `Active` state (it is listed, consumed, merged
    /// away, or depleted).
    LotNotActive(LotId),
    /// Zero or otherwise nonsensical quantity.
    InvalidQuantity,
    /// Real goods ran out: the offered lots do not cover the requirement.
    InsufficientQuantity { commodity: CommodityId, missing: u64 },
    /// Real components ran out: not enough loose components matching an
    /// accepted def were physically on hand to cover a recipe's requirement.
    InsufficientComponents { accepts: Vec<ComponentDefId>, missing: u32 },
    /// The treasury cannot cover the cost. Money is real too.
    InsufficientFunds { actor: ActorId, needed: i64, available: i64 },
    /// Arithmetic overflow in money or quantity math.
    Overflow,
    /// Component is not compatible with the target slot (compatibility is
    /// data-driven, not assumed).
    IncompatibleComponent { component: ComponentId, asset: AssetId, slot: u32 },
    /// The slot already holds a component.
    SlotOccupied { asset: AssetId, slot: u32 },
    /// The location exists but is the wrong kind for this operation
    /// (e.g. mining a city).
    InvalidLocationKind(LocationId),
    /// The asset exists but is the wrong kind (e.g. using a mech as factory
    /// tooling).
    InvalidAssetKind(AssetId),
    /// Convoy is not at a site (it is en route or disbanded).
    ConvoyNotAtSite(ConvoyId),
    /// Formation is not at a site (it is already en route).
    FormationNotAtSite(FormationId),
    /// Factory tooling missing or bound to a different exact design.
    ToolingMismatch { factory: FactoryId },
    /// Goods in a sequestered stockpile cannot be freely traded.
    SequesteredStockpile(StockpileId),
    /// Lots cannot merge unless commodity, quality, legal status, owner and
    /// location all match; merging must never launder provenance.
    MergeMismatch(String),
    /// Seeding APIs only accept seed-class origins; produced goods must come
    /// from real production.
    InvalidOrigin(String),
    /// TO&E assembly failed: real assets are missing. This is the demand
    /// signal faction AI will consume.
    ToeShortage(Vec<ToeShortage>),
    /// A design's declared component slots do not meet the docket's per-kind
    /// requirement (mechs: exactly 5; weapons/equipment: 5-6).
    InvalidSlotCount { kind: AssetKind, got: usize },
    /// A component's category does not fit the slot kind being targeted
    /// (e.g. a factory sub-system component offered to a mech slot).
    WrongComponentCategory { component: ComponentId },
    /// A factory's fixed sub-system slot is already filled.
    FactorySlotOccupied { factory: FactoryId, category: ComponentCategory },
    /// A factory is missing one or more of its five required sub-system
    /// components and cannot run production yet.
    FactoryIncomplete { factory: FactoryId, missing: Vec<ComponentCategory> },
    /// The contract's objective is not actually true against live state yet
    /// (cargo not at destination, convoy not arrived).
    ContractNotFulfilled(ContractId),
    /// Salvage settlement would exceed the contract's negotiated tonnage cap.
    SalvageCapExceeded { contract: ContractId, requested: u64, cap: u64 },
    /// Catch-all for state-machine violations (departing a disbanded convoy,
    /// completing a cancelled contract…).
    InvalidState(String),
}

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use SimError::*;
        match self {
            UnknownActor(id) => write!(f, "unknown actor {id}"),
            UnknownLocation(id) => write!(f, "unknown location {id}"),
            UnknownCommodity(id) => write!(f, "unknown commodity {id}"),
            UnknownLot(id) => write!(f, "unknown lot {id}"),
            UnknownDesign(id) => write!(f, "unknown design {id}"),
            UnknownAsset(id) => write!(f, "unknown asset {id}"),
            UnknownComponentDef(id) => write!(f, "unknown component def {id}"),
            UnknownComponent(id) => write!(f, "unknown component {id}"),
            UnknownMarket(id) => write!(f, "unknown market {id}"),
            UnknownListing(id) => write!(f, "unknown listing {id}"),
            UnknownOrder(id) => write!(f, "unknown buy order {id}"),
            UnknownStockpile(id) => write!(f, "unknown stockpile {id}"),
            UnknownFactory(id) => write!(f, "unknown factory {id}"),
            UnknownRecipe(id) => write!(f, "unknown recipe {id}"),
            UnknownJob(id) => write!(f, "unknown production job {id}"),
            UnknownRoute(id) => write!(f, "unknown route {id}"),
            UnknownConvoy(id) => write!(f, "unknown convoy {id}"),
            UnknownIntel(id) => write!(f, "unknown intel {id}"),
            UnknownContract(id) => write!(f, "unknown contract {id}"),
            UnknownToeTemplate(id) => write!(f, "unknown TO&E template {id}"),
            UnknownFormation(id) => write!(f, "unknown formation {id}"),
            NotOwner { actor } => write!(f, "{actor} does not own this"),
            NotColocated => write!(f, "objects are not physically colocated"),
            LotNotActive(id) => write!(f, "{id} is not active"),
            InvalidQuantity => write!(f, "invalid quantity"),
            InsufficientQuantity { commodity, missing } => {
                write!(f, "insufficient real stock of {commodity}: {missing} short")
            }
            InsufficientComponents { accepts, missing } => {
                write!(f, "insufficient real components matching {accepts:?}: {missing} short")
            }
            InsufficientFunds { actor, needed, available } => {
                write!(f, "{actor} needs {needed} credits but has {available}")
            }
            Overflow => write!(f, "arithmetic overflow"),
            IncompatibleComponent { component, asset, slot } => {
                write!(f, "{component} is not compatible with slot {slot} of {asset}")
            }
            SlotOccupied { asset, slot } => write!(f, "slot {slot} of {asset} is occupied"),
            InvalidLocationKind(id) => write!(f, "{id} is the wrong kind of location"),
            InvalidAssetKind(id) => write!(f, "{id} is the wrong kind of asset"),
            ConvoyNotAtSite(id) => write!(f, "{id} is not at a site"),
            FormationNotAtSite(id) => write!(f, "{id} is not at a site"),
            ToolingMismatch { factory } => {
                write!(f, "{factory} lacks the exact tooling this recipe requires")
            }
            SequesteredStockpile(id) => {
                write!(f, "goods in sequestered {id} cannot be freely traded")
            }
            MergeMismatch(msg) => write!(f, "cannot merge lots: {msg}"),
            InvalidOrigin(msg) => write!(f, "invalid origin: {msg}"),
            ToeShortage(missing) => {
                write!(f, "TO&E shortage: {} slot requirement(s) unfilled", missing.len())
            }
            InvalidSlotCount { kind, got } => {
                write!(f, "{kind:?} design declares {got} component slot(s), which violates the docket's slot-count rule for that kind")
            }
            WrongComponentCategory { component } => {
                write!(f, "{component} has a category that does not fit this slot")
            }
            FactorySlotOccupied { factory, category } => {
                write!(f, "{factory} already has a component in its {category:?} slot")
            }
            FactoryIncomplete { factory, missing } => {
                write!(f, "{factory} is missing sub-system component(s): {missing:?}")
            }
            ContractNotFulfilled(id) => write!(f, "{id}'s objective is not yet true"),
            SalvageCapExceeded { contract, requested, cap } => {
                write!(f, "{contract} salvage of {requested} units exceeds the {cap}-unit cap")
            }
            InvalidState(msg) => write!(f, "invalid state: {msg}"),
        }
    }
}

impl std::error::Error for SimError {}

/// Checked money math: `quantity * price_per_unit`.
pub(crate) fn mul_money(quantity: u64, price_per_unit: i64) -> Result<i64, SimError> {
    let q: i64 = i64::try_from(quantity).map_err(|_| SimError::Overflow)?;
    q.checked_mul(price_per_unit).ok_or(SimError::Overflow)
}
