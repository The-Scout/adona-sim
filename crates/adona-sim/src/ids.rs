//! Typed identifiers for every domain entity.
//!
//! Ids are plain `u64`s allocated deterministically by the [`crate::World`]
//! from a single monotonic counter, so the same seed plus the same ordered
//! inputs allocate the same ids. They are newtyped so a `LotId` can never be
//! used where an `AssetId` is expected.

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            serde::Serialize, serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

define_id!(
    /// Decision-making entity: faction, city authority, merc company, trader,
    /// pirate polity, admin operator, player-class operator.
    ActorId
);
define_id!(
    /// Physical site: city, mine, refinery, factory site, warehouse, ruin…
    LocationId
);
define_id!(
    /// Definition of a fungible commodity (ore, armor plate, fuel, food…).
    CommodityId
);
define_id!(
    /// A provenance-bearing physical batch of a commodity. Never an
    /// originless pool.
    LotId
);
define_id!(
    /// A design (blueprint) for a serial asset: a specific mech model,
    /// vehicle model, weapon design, tooling design, container design.
    DesignId
);
define_id!(
    /// An individual serial object with identity and history: a mech, a
    /// vehicle, a weapon, factory tooling, a cargo container.
    AssetId
);
define_id!(
    /// Definition of a component type (actuator for a given mech model,
    /// weapon barrel, control board…). Compatibility is data, not vibes.
    ComponentDefId
);
define_id!(
    /// An individual component instance with provenance.
    ComponentId
);
define_id!(
    /// A local physical market attached to a site. The global market layer is
    /// a discovery view over all of these.
    MarketId
);
define_id!(
    /// An offer to sell a specific real lot at a market.
    SellListingId
);
define_id!(
    /// An order to buy a quantity of a commodity, with funds escrowed.
    BuyOrderId
);
define_id!(
    /// A physical faction warehouse; possibly sequestered from open trade.
    StockpileId
);
define_id!(
    /// A production facility at a site.
    FactoryId
);
define_id!(
    /// A production recipe: exact inputs, outputs, duration, tooling.
    RecipeId
);
define_id!(
    /// A running or completed production job at a factory.
    ProductionJobId
);
define_id!(
    /// A travel route between two sites, measured in days.
    RouteId
);
define_id!(
    /// A convoy: real vehicles, real guards, real cargo.
    ConvoyId
);
define_id!(
    /// An immutable intel observation.
    IntelId
);
define_id!(
    /// A contract generated from the economy/faction war, with escrowed funds.
    ContractId
);
define_id!(
    /// A Table of Organization & Equipment template (doctrine target).
    ToeTemplateId
);
define_id!(
    /// A formation assembled from real assets against a TO&E template.
    FormationId
);
define_id!(
    /// An entry in the append-only event/provenance log.
    EventId
);
define_id!(
    /// A standing TO&E doctrine goal a faction pursues: fill this template
    /// at this site, forever, by procuring real production and trade.
    FactionGoalId
);
