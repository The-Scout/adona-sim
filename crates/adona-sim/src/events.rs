//! The append-only event / provenance log.
//!
//! Every important change in the world is recorded as an event: creation,
//! split, merge, movement, ownership change, consumption, production, trade,
//! convoy activity, intel, contracts, TO&E assembly, and explicit admin
//! edits. Provenance questions ("where did this armor plate come from?") are
//! answered by lot lineage plus this log, never by trusting a mutable field.
//!
//! TODO(compaction): snapshotting/compaction for very long histories without
//! losing the ability to explain important history (docket: Admin tooling).

use crate::actors::Credits;
use crate::assets::ComponentCategory;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::time::DayQuarter;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    /// Monotonic sequence number, dense from 0. Storage adapters key on this.
    pub seq: u64,
    pub day: u64,
    pub quarter: DayQuarter,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    WorldCreated { seed: u64 },
    DayAdvanced { day: u64 },

    ActorCreated { actor: ActorId, issued: Credits },
    LocationCreated { location: LocationId },
    RouteCreated { route: RouteId },
    CommodityDefined { commodity: CommodityId },
    DesignDefined { design: DesignId },
    ComponentDefDefined { def: ComponentDefId },

    LotCreated { lot: LotId, commodity: CommodityId, quantity: u64, owner: ActorId },
    LotSplit { parent: LotId, child: LotId, quantity: u64 },
    LotsMerged { sources: Vec<LotId>, result: LotId },
    LotMoved { lot: LotId, from: LocationRef, to: LocationRef },
    LotOwnerChanged { lot: LotId, from: ActorId, to: ActorId },
    LotConsumed { lot: LotId, job: ProductionJobId },
    /// Real goods destroyed by population consumption (not production) —
    /// docket TODO(population): "consumption that actually destroys
    /// purchased goods."
    GoodsConsumed { lot: LotId, commodity: CommodityId, owner: ActorId, quantity: u64 },
    MineYield { mine: LocationId, lot: LotId, quantity: u64 },

    AssetCreated { asset: AssetId, design: DesignId, owner: ActorId },
    AssetMoved { asset: AssetId, from: LocationRef, to: LocationRef },
    AssetOwnerChanged { asset: AssetId, from: ActorId, to: ActorId },
    ComponentCreated { component: ComponentId, def: ComponentDefId, owner: ActorId },
    ComponentFitted { component: ComponentId, asset: AssetId, slot: u32 },
    ComponentFittedToFactory { component: ComponentId, factory: FactoryId, category: ComponentCategory },

    MarketCreated { market: MarketId },
    LotListed { listing: SellListingId, market: MarketId, lot: LotId, price_per_unit: Credits },
    ListingCancelled { listing: SellListingId },
    BuyOrderPlaced {
        order: BuyOrderId,
        buyer: ActorId,
        commodity: CommodityId,
        quantity: u64,
        limit_price_per_unit: Credits,
        escrow: Credits,
    },
    BuyOrderClosed { order: BuyOrderId, refunded: Credits },
    /// A real trade: real goods changed owner, real credits changed hands.
    /// This is also the raw price history for future price discovery.
    TradeExecuted {
        listing: SellListingId,
        market: MarketId,
        seller: ActorId,
        buyer: ActorId,
        lot_delivered: LotId,
        quantity: u64,
        price_per_unit: Credits,
        total: Credits,
    },

    StockpileCreated { stockpile: StockpileId },

    FactoryCreated { factory: FactoryId },
    ToolingInstalled { factory: FactoryId, tooling: AssetId, cost: Credits, retooling_until: u64 },
    RecipeDefined { recipe: RecipeId },
    ProductionStarted {
        job: ProductionJobId,
        factory: FactoryId,
        recipe: RecipeId,
        consumed_lots: Vec<LotId>,
    },
    ProductionCompleted {
        job: ProductionJobId,
        output_lots: Vec<LotId>,
        output_assets: Vec<AssetId>,
        output_components: Vec<ComponentId>,
    },

    ConvoyFormed { convoy: ConvoyId, at: LocationId },
    GuardAssigned { convoy: ConvoyId, asset: AssetId },
    CargoLotLoaded { convoy: ConvoyId, lot: LotId },
    CargoLotUnloaded { convoy: ConvoyId, lot: LotId, at: LocationId },
    CargoAssetLoaded { convoy: ConvoyId, asset: AssetId },
    CargoAssetUnloaded { convoy: ConvoyId, asset: AssetId, at: LocationId },
    ConvoyDeparted { convoy: ConvoyId, route: RouteId, arrives_day: u64 },
    ConvoyArrived { convoy: ConvoyId, at: LocationId },
    ConvoyDisbanded { convoy: ConvoyId, at: LocationId },

    IntelRecorded { intel: IntelId },
    IntelRelayed { source: IntelId, relayed: IntelId, via: ActorId },
    MisinformationPlanted { intel: IntelId, planter: Option<ActorId> },

    ContractIssued { contract: ContractId, employer: ActorId, escrow: Credits },
    ContractAccepted { contract: ContractId, contractor: ActorId },
    ContractCompleted { contract: ContractId, payout: Credits },
    ContractCancelled { contract: ContractId, refunded: Credits },
    SalvageSettled { contract: ContractId, contractor: ActorId, assets: Vec<AssetId>, lots: Vec<LotId> },
    IllegalSalvageFlagged { contract: ContractId, lot: LotId, legal_status: crate::goods::LegalStatus },

    ToeTemplateDefined { template: ToeTemplateId },
    FormationAssembled { formation: FormationId, template: ToeTemplateId, assets: Vec<AssetId> },

    /// A real battle resolved between two forces sharing a site (docket:
    /// the ant-life-sim faction war). See `combat::BattleOutcome` for the
    /// full result including which specific units were lost or captured.
    BattleResolved {
        site: LocationId,
        attacker: ActorId,
        defender: ActorId,
        attacker_won: bool,
        attacker_power: u64,
        defender_power: u64,
    },

    FactionGoalSet { goal: FactionGoalId, owner: ActorId, template: ToeTemplateId, site: LocationId },
    /// Faction procurement AI started real production toward a TO&E
    /// shortage — the shortage is the demand signal, this is the AI acting
    /// on it (docket TODO(faction-ai)).
    FactionProcurementStarted { goal: FactionGoalId, job: ProductionJobId },
    /// Faction procurement AI placed a real buy order for a recipe input it
    /// is short of, toward filling a TO&E shortage.
    FactionProcurementOrdered { goal: FactionGoalId, order: BuyOrderId, commodity: CommodityId },

    /// Explicit, auditable operator intervention. Admin edits are events,
    /// never silent mutation.
    AdminEdit { operator: Option<ActorId>, description: String },
}
