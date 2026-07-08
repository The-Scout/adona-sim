//! The world aggregate: all strategic state, the deterministic tick, the
//! event log, and the invariant checker.
//!
//! Determinism contract: same seed + same ordered API calls = same strategic
//! state, byte for byte. Everything that feeds state is deterministic —
//! id allocation is a single monotonic counter, all collections are
//! `BTreeMap`s iterated in key order, the RNG is seeded SplitMix64 carried
//! in state, and there are no floats anywhere.

use crate::actors::Actor;
use crate::assets::{ComponentDef, ComponentInstance, ComponentPlacement, ItemDesign, SerialAsset};
use crate::contracts::{Contract, ContractState};
use crate::convoys::{Convoy, ConvoyState, Route};
use crate::events::{Event, EventKind};
use crate::faction_ai::FactionGoal;
use crate::goods::{CommodityDef, Lot, LotState};
use crate::ids::*;
use crate::intel::IntelObservation;
use crate::locations::{Location, LocationRef};
use crate::markets::{BuyOrder, Market, SellListing};
use crate::production::{Factory, JobState, ProductionJob, Recipe};
use crate::rng::SimRng;
use crate::stockpiles::Stockpile;
use crate::time::SimClock;
use crate::toe::{Formation, ToeTemplate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub clock: SimClock,
    pub(crate) rng: SimRng,
    pub(crate) next_id: u64,
    pub(crate) next_event_seq: u64,
    /// Total credits ever issued (actor seed treasuries). The
    /// money-conservation invariant checks all treasuries + escrows against
    /// this.
    pub(crate) money_issued: i128,

    pub(crate) actors: BTreeMap<ActorId, Actor>,
    pub(crate) locations: BTreeMap<LocationId, Location>,
    pub(crate) commodities: BTreeMap<CommodityId, CommodityDef>,
    pub(crate) lots: BTreeMap<LotId, Lot>,
    pub(crate) designs: BTreeMap<DesignId, ItemDesign>,
    pub(crate) component_defs: BTreeMap<ComponentDefId, ComponentDef>,
    pub(crate) components: BTreeMap<ComponentId, ComponentInstance>,
    pub(crate) assets: BTreeMap<AssetId, SerialAsset>,
    pub(crate) markets: BTreeMap<MarketId, Market>,
    pub(crate) sell_listings: BTreeMap<SellListingId, SellListing>,
    pub(crate) buy_orders: BTreeMap<BuyOrderId, BuyOrder>,
    pub(crate) stockpiles: BTreeMap<StockpileId, Stockpile>,
    pub(crate) factories: BTreeMap<FactoryId, Factory>,
    pub(crate) recipes: BTreeMap<RecipeId, Recipe>,
    pub(crate) production_jobs: BTreeMap<ProductionJobId, ProductionJob>,
    pub(crate) routes: BTreeMap<RouteId, Route>,
    pub(crate) convoys: BTreeMap<ConvoyId, Convoy>,
    pub(crate) intel: BTreeMap<IntelId, IntelObservation>,
    pub(crate) contracts: BTreeMap<ContractId, Contract>,
    pub(crate) toe_templates: BTreeMap<ToeTemplateId, ToeTemplate>,
    pub(crate) formations: BTreeMap<FormationId, Formation>,
    pub(crate) faction_goals: BTreeMap<FactionGoalId, FactionGoal>,

    /// Real price discovery: an exponential moving average per (market,
    /// commodity) over `TradeExecuted` history, updated on every trade. This
    /// is the crate's answer to the docket's "prices emerge from actual
    /// supply and demand" — it is derived purely from real trades, never set
    /// directly. Nested (rather than tuple-keyed) so it round-trips through
    /// serde_json, whose map keys must be strings/numbers, not tuples.
    pub(crate) price_index: BTreeMap<MarketId, BTreeMap<CommodityId, i64>>,

    /// Append-only event/provenance log.
    pub(crate) events: Vec<Event>,
}

impl World {
    pub fn new(seed: u64) -> Self {
        let mut w = World {
            seed,
            clock: SimClock::start(),
            rng: SimRng::new(seed),
            next_id: 0,
            next_event_seq: 0,
            money_issued: 0,
            actors: BTreeMap::new(),
            locations: BTreeMap::new(),
            commodities: BTreeMap::new(),
            lots: BTreeMap::new(),
            designs: BTreeMap::new(),
            component_defs: BTreeMap::new(),
            components: BTreeMap::new(),
            assets: BTreeMap::new(),
            markets: BTreeMap::new(),
            sell_listings: BTreeMap::new(),
            buy_orders: BTreeMap::new(),
            stockpiles: BTreeMap::new(),
            factories: BTreeMap::new(),
            recipes: BTreeMap::new(),
            production_jobs: BTreeMap::new(),
            routes: BTreeMap::new(),
            convoys: BTreeMap::new(),
            intel: BTreeMap::new(),
            contracts: BTreeMap::new(),
            toe_templates: BTreeMap::new(),
            formations: BTreeMap::new(),
            faction_goals: BTreeMap::new(),
            price_index: BTreeMap::new(),
            events: Vec::new(),
        };
        w.push_event(EventKind::WorldCreated { seed });
        w
    }

    /// Advance one strategic day, in fixed deterministic phase order:
    /// sub-day quarter events (contact/interception windows), production
    /// completions, convoy movement, formation movement, civilian demand,
    /// faction planning, faction deployment (marching on contested/enemy
    /// ground), faction war, market matching, population.
    pub fn tick(&mut self) {
        self.clock.advance_day();
        let day = self.clock.day;
        self.push_event(EventKind::DayAdvanced { day });
        for quarter in [crate::time::DayQuarter::Q1, crate::time::DayQuarter::Q2, crate::time::DayQuarter::Q3, crate::time::DayQuarter::Q4] {
            self.clock.quarter = quarter;
            self.tick_quarter_convoy_contacts();
        }
        self.tick_production();
        self.tick_convoys();
        self.tick_formations();
        self.tick_civilian_demand();
        self.tick_faction_ai();
        self.tick_faction_deployment();
        self.tick_faction_war();
        self.tick_market_matching();
        self.tick_population();
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    /// Deterministic id allocation: one monotonic counter across all types.
    pub(crate) fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn push_event(&mut self, kind: EventKind) -> EventId {
        let id = EventId(self.alloc());
        let seq = self.next_event_seq;
        self.next_event_seq += 1;
        self.events.push(Event {
            id,
            seq,
            day: self.clock.day,
            quarter: self.clock.quarter,
            kind,
        });
        id
    }

    // ------------------------------------------------------------------
    // Read API
    // ------------------------------------------------------------------

    pub fn today(&self) -> u64 {
        self.clock.day
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn actor(&self, id: ActorId) -> Option<&Actor> {
        self.actors.get(&id)
    }
    pub fn location(&self, id: LocationId) -> Option<&Location> {
        self.locations.get(&id)
    }
    pub fn commodity(&self, id: CommodityId) -> Option<&CommodityDef> {
        self.commodities.get(&id)
    }
    pub fn lot(&self, id: LotId) -> Option<&Lot> {
        self.lots.get(&id)
    }
    pub fn design(&self, id: DesignId) -> Option<&ItemDesign> {
        self.designs.get(&id)
    }
    pub fn component_def(&self, id: ComponentDefId) -> Option<&ComponentDef> {
        self.component_defs.get(&id)
    }
    pub fn component(&self, id: ComponentId) -> Option<&ComponentInstance> {
        self.components.get(&id)
    }
    pub fn asset(&self, id: AssetId) -> Option<&SerialAsset> {
        self.assets.get(&id)
    }
    pub fn market(&self, id: MarketId) -> Option<&Market> {
        self.markets.get(&id)
    }
    pub fn listing(&self, id: SellListingId) -> Option<&SellListing> {
        self.sell_listings.get(&id)
    }
    pub fn buy_order(&self, id: BuyOrderId) -> Option<&BuyOrder> {
        self.buy_orders.get(&id)
    }
    pub fn stockpile(&self, id: StockpileId) -> Option<&Stockpile> {
        self.stockpiles.get(&id)
    }
    pub fn factory(&self, id: FactoryId) -> Option<&Factory> {
        self.factories.get(&id)
    }
    pub fn recipe(&self, id: RecipeId) -> Option<&Recipe> {
        self.recipes.get(&id)
    }
    pub fn production_job(&self, id: ProductionJobId) -> Option<&ProductionJob> {
        self.production_jobs.get(&id)
    }
    pub fn route(&self, id: RouteId) -> Option<&Route> {
        self.routes.get(&id)
    }
    pub fn convoy(&self, id: ConvoyId) -> Option<&Convoy> {
        self.convoys.get(&id)
    }
    pub fn intel_record(&self, id: IntelId) -> Option<&IntelObservation> {
        self.intel.get(&id)
    }
    pub fn contract(&self, id: ContractId) -> Option<&Contract> {
        self.contracts.get(&id)
    }
    pub fn toe_template(&self, id: ToeTemplateId) -> Option<&ToeTemplate> {
        self.toe_templates.get(&id)
    }
    pub fn formation(&self, id: FormationId) -> Option<&Formation> {
        self.formations.get(&id)
    }

    /// The current price-discovery index for a commodity at a market: an
    /// exponential moving average over its real trade history. `None` means
    /// no trade has ever executed there yet — callers should fall back to
    /// the commodity's bootstrap `base_price`.
    pub fn price_index(&self, market: MarketId, commodity: CommodityId) -> Option<i64> {
        self.price_index.get(&market)?.get(&commodity).copied()
    }

    pub fn lots_iter(&self) -> impl Iterator<Item = &Lot> {
        self.lots.values()
    }
    pub fn assets_iter(&self) -> impl Iterator<Item = &SerialAsset> {
        self.assets.values()
    }
    pub fn buy_orders_iter(&self) -> impl Iterator<Item = &BuyOrder> {
        self.buy_orders.values()
    }
    pub fn convoys_iter(&self) -> impl Iterator<Item = &Convoy> {
        self.convoys.values()
    }
    pub fn actors_iter(&self) -> impl Iterator<Item = &Actor> {
        self.actors.values()
    }
    pub fn locations_iter(&self) -> impl Iterator<Item = &Location> {
        self.locations.values()
    }
    pub fn contracts_iter(&self) -> impl Iterator<Item = &Contract> {
        self.contracts.values()
    }
    pub fn formations_iter(&self) -> impl Iterator<Item = &Formation> {
        self.formations.values()
    }
    pub fn routes_iter(&self) -> impl Iterator<Item = &Route> {
        self.routes.values()
    }

    // ------------------------------------------------------------------
    // Determinism
    // ------------------------------------------------------------------

    /// Stable digest of the entire strategic state (including the event
    /// log and RNG state). Two worlds with the same seed and same ordered
    /// inputs must produce the same digest.
    pub fn state_digest(&self) -> u64 {
        let json = serde_json::to_string(self).expect("world state must be serializable");
        fnv1a(json.as_bytes())
    }

    // ------------------------------------------------------------------
    // Invariants
    // ------------------------------------------------------------------

    /// Check the hard "everything important is real" invariants. Returns a
    /// list of violations; an empty list means the world is consistent.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();

        // Actors: treasuries never negative.
        for (id, a) in &self.actors {
            if a.treasury < 0 {
                v.push(format!("{id} has negative treasury {}", a.treasury));
            }
        }

        // Lots: one owner, one location, nonzero quantity while in play,
        // consistent listing back-references.
        for (id, lot) in &self.lots {
            if !self.actors.contains_key(&lot.owner) {
                v.push(format!("{id} owner {} does not exist", lot.owner));
            }
            if !self.location_ref_valid(lot.location) {
                v.push(format!("{id} has a dangling location"));
            }
            match &lot.state {
                LotState::Active => {
                    if lot.quantity == 0 {
                        v.push(format!("{id} is active with zero quantity"));
                    }
                }
                LotState::Listed(listing) => {
                    if lot.quantity == 0 {
                        v.push(format!("{id} is listed with zero quantity"));
                    }
                    match self.sell_listings.get(listing) {
                        None => v.push(format!("{id} is listed on nonexistent {listing}")),
                        Some(l) if l.lot != *id => {
                            v.push(format!("{id} listing back-reference mismatch"))
                        }
                        _ => {}
                    }
                }
                LotState::ConsumedByProduction(job) => {
                    if !self.production_jobs.contains_key(job) {
                        v.push(format!("{id} consumed by nonexistent {job}"));
                    }
                }
                LotState::MergedInto(result) => {
                    if !self.lots.contains_key(result) {
                        v.push(format!("{id} merged into nonexistent {result}"));
                    }
                }
                LotState::Depleted => {}
            }
        }

        // Serial assets: one owner, one location, component back-references.
        for (id, a) in &self.assets {
            if !self.actors.contains_key(&a.owner) {
                v.push(format!("{id} owner {} does not exist", a.owner));
            }
            if !self.location_ref_valid(a.location) {
                v.push(format!("{id} has a dangling location"));
            }
            if !self.designs.contains_key(&a.design) {
                v.push(format!("{id} has unknown design {}", a.design));
            }
            for (slot, comp) in &a.fitted {
                match self.components.get(comp) {
                    None => v.push(format!("{id} slot {slot} holds nonexistent {comp}")),
                    Some(c) => {
                        if c.placement != ComponentPlacement::Fitted(*id) {
                            v.push(format!("{comp} placement does not match {id} slot {slot}"));
                        }
                    }
                }
            }
        }

        // Components: owner exists, placement valid both ways.
        for (id, c) in &self.components {
            if !self.actors.contains_key(&c.owner) {
                v.push(format!("{id} owner {} does not exist", c.owner));
            }
            match &c.placement {
                ComponentPlacement::Fitted(asset) => match self.assets.get(asset) {
                    None => v.push(format!("{id} fitted to nonexistent {asset}")),
                    Some(a) => {
                        if !a.fitted.values().any(|fc| fc == id) {
                            v.push(format!("{id} claims to be fitted to {asset} which disagrees"));
                        }
                    }
                },
                ComponentPlacement::FittedFactory(factory) => match self.factories.get(factory) {
                    None => v.push(format!("{id} fitted to nonexistent {factory}")),
                    Some(f) => {
                        if !f.components.values().any(|fc| fc == id) {
                            v.push(format!("{id} claims to be fitted to {factory} which disagrees"));
                        }
                    }
                },
                ComponentPlacement::Loose(loc) => {
                    if !self.location_ref_valid(*loc) {
                        v.push(format!("{id} has a dangling loose location"));
                    }
                }
            }
        }

        // Factories: every fixed sub-system slot points at a real component
        // whose def actually matches that category and whose placement
        // agrees it is fitted here.
        for (id, f) in &self.factories {
            for (category, comp_id) in &f.components {
                match self.components.get(comp_id) {
                    None => v.push(format!("{id} slot {category:?} holds nonexistent {comp_id}")),
                    Some(c) => {
                        if c.placement != ComponentPlacement::FittedFactory(*id) {
                            v.push(format!(
                                "{comp_id} placement does not match {id} slot {category:?}"
                            ));
                        }
                        match self.component_defs.get(&c.def) {
                            None => v.push(format!("{comp_id} has unknown def {}", c.def)),
                            Some(def) if def.category != *category => v.push(format!(
                                "{comp_id} is in {id}'s {category:?} slot but its def category is {:?}",
                                def.category
                            )),
                            _ => {}
                        }
                    }
                }
            }
        }

        // Market listings reference real goods physically at the market.
        for (id, l) in &self.sell_listings {
            match self.lots.get(&l.lot) {
                None => v.push(format!("{id} lists nonexistent {}", l.lot)),
                Some(lot) => {
                    if lot.state != LotState::Listed(*id) {
                        v.push(format!("{id} lists {} which is not in Listed state", l.lot));
                    }
                    match self.markets.get(&l.market) {
                        None => v.push(format!("{id} on nonexistent {}", l.market)),
                        Some(m) => {
                            if self.resolve_site(lot.location) != Some(m.site) {
                                v.push(format!(
                                    "{id} lists {} which is not physically at the market site",
                                    l.lot
                                ));
                            }
                        }
                    }
                    if lot.owner != l.seller {
                        v.push(format!("{id} seller does not own {}", l.lot));
                    }
                }
            }
        }

        // Buy orders: buyer exists, escrow matches remaining quantity.
        for (id, o) in &self.buy_orders {
            if !self.actors.contains_key(&o.buyer) {
                v.push(format!("{id} buyer {} does not exist", o.buyer));
            }
            if o.escrow < 0 {
                v.push(format!("{id} has negative escrow"));
            }
            match crate::error::mul_money(o.quantity, o.limit_price_per_unit) {
                Ok(expected) => {
                    if o.escrow != expected {
                        v.push(format!(
                            "{id} escrow {} does not match remaining quantity ({expected})",
                            o.escrow
                        ));
                    }
                }
                Err(_) => v.push(format!("{id} escrow arithmetic overflows")),
            }
        }

        // Convoys: everything aboard exists and is located aboard.
        for (id, c) in &self.convoys {
            if matches!(c.state, ConvoyState::Disbanded) {
                continue;
            }
            if !self.actors.contains_key(&c.owner) {
                v.push(format!("{id} owner {} does not exist", c.owner));
            }
            for aid in c.vehicles.iter().chain(&c.guards).chain(&c.cargo_assets) {
                match self.assets.get(aid) {
                    None => v.push(format!("{id} references nonexistent {aid}")),
                    Some(a) => {
                        if a.location != LocationRef::Convoy(*id) {
                            v.push(format!("{aid} is on {id}'s manifest but not located aboard"));
                        }
                    }
                }
            }
            for lid in &c.cargo_lots {
                match self.lots.get(lid) {
                    None => v.push(format!("{id} carries nonexistent {lid}")),
                    Some(lot) => {
                        let aboard = match lot.location {
                            LocationRef::Convoy(cv) => cv == *id,
                            LocationRef::Container(a) => self
                                .assets
                                .get(&a)
                                .map(|asset| asset.location == LocationRef::Convoy(*id))
                                .unwrap_or(false),
                            _ => false,
                        };
                        if !aboard {
                            v.push(format!("{lid} is on {id}'s manifest but not located aboard"));
                        }
                        if lot.state != LotState::Active {
                            v.push(format!("{lid} aboard {id} is not active"));
                        }
                    }
                }
            }
        }

        // Contracts: employers exist; live contracts hold their escrow and
        // point at real targets.
        for (id, c) in &self.contracts {
            if !self.actors.contains_key(&c.employer) {
                v.push(format!("{id} employer {} does not exist", c.employer));
            }
            match c.state {
                ContractState::Open | ContractState::Accepted { .. } => {
                    if c.escrowed_payment <= 0 {
                        v.push(format!("{id} is live without escrowed payment"));
                    }
                }
                _ => {}
            }
        }

        // Production jobs: consumed inputs really exist and are marked
        // consumed by this job.
        for (id, j) in &self.production_jobs {
            if !self.factories.contains_key(&j.factory) {
                v.push(format!("{id} at nonexistent {}", j.factory));
            }
            for lid in &j.consumed_lots {
                match self.lots.get(lid) {
                    None => v.push(format!("{id} consumed nonexistent {lid}")),
                    Some(lot) => {
                        if lot.state != LotState::ConsumedByProduction(*id) {
                            v.push(format!("{lid} not marked consumed by {id}"));
                        }
                    }
                }
            }
            if let JobState::Completed { output_lots, output_assets, output_components } = &j.state {
                for ol in output_lots {
                    if !self.lots.contains_key(ol) {
                        v.push(format!("{id} output lot {ol} does not exist"));
                    }
                }
                for oa in output_assets {
                    if !self.assets.contains_key(oa) {
                        v.push(format!("{id} output asset {oa} does not exist"));
                    }
                }
                for oc in output_components {
                    if !self.components.contains_key(oc) {
                        v.push(format!("{id} output component {oc} does not exist"));
                    }
                }
            }
        }

        // Formations: assets exist, are owned by the formation owner, and
        // are located in the formation.
        for (id, f) in &self.formations {
            for aid in &f.assets {
                match self.assets.get(aid) {
                    None => v.push(format!("{id} contains nonexistent {aid}")),
                    Some(a) => {
                        if a.owner != f.owner {
                            v.push(format!("{aid} in {id} is not owned by the formation owner"));
                        }
                        if a.location != LocationRef::Formation(*id) {
                            v.push(format!("{aid} on {id}'s roster but not located in it"));
                        }
                    }
                }
            }
        }

        // Money conservation: treasuries + order escrows + live contract
        // escrows == everything ever issued. No money from nowhere.
        let mut total: i128 = 0;
        for a in self.actors.values() {
            total += a.treasury as i128;
        }
        for o in self.buy_orders.values() {
            total += o.escrow as i128;
        }
        for c in self.contracts.values() {
            total += c.escrowed_payment as i128;
        }
        if total != self.money_issued {
            v.push(format!(
                "money conservation violated: {total} in circulation vs {} issued",
                self.money_issued
            ));
        }

        v
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
