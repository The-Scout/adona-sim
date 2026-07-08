//! Markets: local physical inventories under a global discovery layer.
//!
//! A market is attached to a real site. A sell listing reserves a specific
//! real lot that is physically at that site; a buy order escrows real
//! credits. The "global market" is a discovery view over every local market
//! (`global_listings`), EVE-style: you can see and order remotely, but the
//! goods exist in one physical place and still have to be hauled by convoy
//! after purchase. Goods can run out; when the lot is gone the listing is
//! gone.
//!
//! Price formation: buy orders match at listing price (the real price a real
//! seller asked for real goods — never an abstract clearing price). On top of
//! that, [`World::price_index`] derives a smoothed reference price per
//! (market, commodity) purely from `TradeExecuted` history: an exponential
//! moving average, updated on every trade, that civilian demand and future
//! faction AI read to judge whether a market is cheap, normal, or gouging.
//! Nothing sets the index directly — it only ever moves because real trades
//! happened at real prices.

use crate::actors::Credits;
use crate::error::mul_money;
use crate::events::EventKind;
use crate::goods::LotState;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub id: MarketId,
    pub name: String,
    /// The physical site whose goods this market trades.
    pub site: LocationId,
    /// `None` for open public markets; `Some` for faction-private markets /
    /// internal procurement spaces.
    pub owner: Option<ActorId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SellListing {
    pub id: SellListingId,
    pub market: MarketId,
    pub seller: ActorId,
    /// The real, physically-present lot this listing sells. The lot is in
    /// state `Listed(this)` while the listing is open.
    pub lot: LotId,
    pub price_per_unit: Credits,
    pub listed_day: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderScope {
    /// Match only listings at this market.
    Market(MarketId),
    /// Match the global discovery layer: any market, cheapest first.
    /// Purchased goods still sit at the seller's market until hauled.
    Global,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuyOrder {
    pub id: BuyOrderId,
    pub scope: OrderScope,
    pub buyer: ActorId,
    pub commodity: CommodityId,
    /// Remaining unfilled quantity.
    pub quantity: u64,
    pub limit_price_per_unit: Credits,
    /// Real credits held out of the buyer's treasury: always
    /// `quantity * limit_price_per_unit` for the remaining quantity.
    pub escrow: Credits,
    pub placed_day: u64,
}

impl World {
    pub fn create_market(
        &mut self,
        name: &str,
        site: LocationId,
        owner: Option<ActorId>,
    ) -> Result<MarketId, SimError> {
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        if let Some(o) = owner {
            if !self.actors.contains_key(&o) {
                return Err(SimError::UnknownActor(o));
            }
        }
        let id = MarketId(self.alloc());
        self.markets.insert(
            id,
            Market {
                id,
                name: name.to_string(),
                site,
                owner,
            },
        );
        self.push_event(EventKind::MarketCreated { market: id });
        Ok(id)
    }

    /// List a real lot for sale. The lot must be active, owned by the
    /// seller, and physically at the market's site (goods in a sequestered
    /// stockpile cannot be listed at all — release them first).
    pub fn list_lot_for_sale(
        &mut self,
        seller: ActorId,
        market: MarketId,
        lot: LotId,
        price_per_unit: Credits,
    ) -> Result<SellListingId, SimError> {
        if price_per_unit <= 0 {
            return Err(SimError::InvalidQuantity);
        }
        let market_site = self
            .markets
            .get(&market)
            .ok_or(SimError::UnknownMarket(market))?
            .site;
        let (owner, state, location) = {
            let l = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?;
            (l.owner, l.state.clone(), l.location)
        };
        if owner != seller {
            return Err(SimError::NotOwner { actor: seller });
        }
        if state != LotState::Active {
            return Err(SimError::LotNotActive(lot));
        }
        if let LocationRef::Stockpile(sp) = location {
            if self
                .stockpiles
                .get(&sp)
                .ok_or(SimError::UnknownStockpile(sp))?
                .sequestered
            {
                return Err(SimError::SequesteredStockpile(sp));
            }
        }
        if self.resolve_site(location) != Some(market_site) {
            return Err(SimError::NotColocated);
        }
        let id = SellListingId(self.alloc());
        let listed_day = self.clock.day;
        self.sell_listings.insert(
            id,
            SellListing {
                id,
                market,
                seller,
                lot,
                price_per_unit,
                listed_day,
            },
        );
        self.lots.get_mut(&lot).unwrap().state = LotState::Listed(id);
        self.push_event(EventKind::LotListed {
            listing: id,
            market,
            lot,
            price_per_unit,
        });
        Ok(id)
    }

    pub fn cancel_listing(&mut self, listing: SellListingId) -> Result<(), SimError> {
        let lot = self
            .sell_listings
            .get(&listing)
            .ok_or(SimError::UnknownListing(listing))?
            .lot;
        self.sell_listings.remove(&listing);
        if let Some(l) = self.lots.get_mut(&lot) {
            l.state = LotState::Active;
        }
        self.push_event(EventKind::ListingCancelled { listing });
        Ok(())
    }

    /// Direct purchase against a listing, paid from the buyer's treasury.
    /// Returns the delivered lot (the whole lot, or a split child for a
    /// partial buy). The goods stay at the market site — hauling is the
    /// buyer's problem, by convoy.
    pub fn execute_purchase(
        &mut self,
        listing: SellListingId,
        buyer: ActorId,
        quantity: u64,
    ) -> Result<LotId, SimError> {
        let l = self
            .sell_listings
            .get(&listing)
            .ok_or(SimError::UnknownListing(listing))?
            .clone();
        if l.seller == buyer {
            return Err(SimError::InvalidState("cannot buy own listing".into()));
        }
        if !self.actors.contains_key(&buyer) {
            return Err(SimError::UnknownActor(buyer));
        }
        let total = mul_money(quantity, l.price_per_unit)?;
        self.debit(buyer, total)?;
        self.credit(l.seller, total)?;
        let commodity = self.lots.get(&l.lot).map(|lot| lot.commodity);
        let delivered = self.deliver_listed_lot(listing, buyer, quantity)?;
        self.push_event(EventKind::TradeExecuted {
            listing,
            market: l.market,
            seller: l.seller,
            buyer,
            lot_delivered: delivered,
            quantity,
            price_per_unit: l.price_per_unit,
            total,
        });
        if let Some(commodity) = commodity {
            self.update_price_index(l.market, commodity, l.price_per_unit);
        }
        Ok(delivered)
    }

    /// Place a buy order, escrowing `quantity * limit` real credits.
    pub fn place_buy_order(
        &mut self,
        buyer: ActorId,
        scope: OrderScope,
        commodity: CommodityId,
        quantity: u64,
        limit_price_per_unit: Credits,
    ) -> Result<BuyOrderId, SimError> {
        if quantity == 0 || limit_price_per_unit <= 0 {
            return Err(SimError::InvalidQuantity);
        }
        if !self.commodities.contains_key(&commodity) {
            return Err(SimError::UnknownCommodity(commodity));
        }
        if let OrderScope::Market(m) = scope {
            if !self.markets.contains_key(&m) {
                return Err(SimError::UnknownMarket(m));
            }
        }
        let escrow = mul_money(quantity, limit_price_per_unit)?;
        self.debit(buyer, escrow)?;
        let id = BuyOrderId(self.alloc());
        let placed_day = self.clock.day;
        self.buy_orders.insert(
            id,
            BuyOrder {
                id,
                scope,
                buyer,
                commodity,
                quantity,
                limit_price_per_unit,
                escrow,
                placed_day,
            },
        );
        self.push_event(EventKind::BuyOrderPlaced {
            order: id,
            buyer,
            commodity,
            quantity,
            limit_price_per_unit,
            escrow,
        });
        Ok(id)
    }

    /// Cancel a buy order and refund the remaining escrow.
    pub fn cancel_buy_order(&mut self, order: BuyOrderId) -> Result<(), SimError> {
        let o = self
            .buy_orders
            .remove(&order)
            .ok_or(SimError::UnknownOrder(order))?;
        self.credit(o.buyer, o.escrow)?;
        self.push_event(EventKind::BuyOrderClosed {
            order,
            refunded: o.escrow,
        });
        Ok(())
    }

    /// The global market layer: every open listing across every local
    /// market, in deterministic id order. Discovery, not teleportation.
    pub fn global_listings(&self) -> impl Iterator<Item = &SellListing> {
        self.sell_listings.values()
    }

    /// Fold one real trade into the (market, commodity) price index: a
    /// integer exponential moving average (75% history / 25% latest trade)
    /// so a single flooded or scarce trade cannot whipsaw the index —
    /// anti-oscillation from the docket's open markets question, resolved as
    /// "smooth toward real trades, never jump to them."
    fn update_price_index(&mut self, market: MarketId, commodity: CommodityId, trade_price: Credits) {
        let per_commodity = self.price_index.entry(market).or_default();
        let updated = match per_commodity.get(&commodity) {
            Some(prev) => (prev.saturating_mul(3) + trade_price) / 4,
            None => trade_price,
        };
        per_commodity.insert(commodity, updated);
    }

    /// Deliver goods from a listing to a buyer: split the lot for a partial
    /// fill, or transfer the whole lot and close the listing. Payment is the
    /// caller's responsibility (treasury for direct buys, escrow for order
    /// matching).
    fn deliver_listed_lot(
        &mut self,
        listing: SellListingId,
        buyer: ActorId,
        quantity: u64,
    ) -> Result<LotId, SimError> {
        let lot_id = self
            .sell_listings
            .get(&listing)
            .ok_or(SimError::UnknownListing(listing))?
            .lot;
        let lot_qty = {
            let lot = self.lots.get(&lot_id).ok_or(SimError::UnknownLot(lot_id))?;
            lot.quantity
        };
        if quantity == 0 || quantity > lot_qty {
            return Err(SimError::InvalidQuantity);
        }
        if quantity == lot_qty {
            // Whole lot: transfer and close the listing. The goods ran out.
            let from = {
                let lot = self.lots.get_mut(&lot_id).unwrap();
                lot.state = LotState::Active;
                let from = lot.owner;
                lot.owner = buyer;
                from
            };
            self.sell_listings.remove(&listing);
            self.push_event(EventKind::LotOwnerChanged {
                lot: lot_id,
                from,
                to: buyer,
            });
            Ok(lot_id)
        } else {
            let child = self.split_lot_internal(lot_id, quantity)?;
            let from = {
                let c = self.lots.get_mut(&child).unwrap();
                let from = c.owner;
                c.owner = buyer;
                from
            };
            self.push_event(EventKind::LotOwnerChanged {
                lot: child,
                from,
                to: buyer,
            });
            Ok(child)
        }
    }

    /// Fill part of a buy order from a listing, paying from the order's
    /// escrow. Refunds the limit/price difference to the buyer immediately.
    fn fill_buy_order(
        &mut self,
        order_id: BuyOrderId,
        listing_id: SellListingId,
        quantity: u64,
    ) -> Result<(), SimError> {
        let order = self
            .buy_orders
            .get(&order_id)
            .ok_or(SimError::UnknownOrder(order_id))?
            .clone();
        let listing = self
            .sell_listings
            .get(&listing_id)
            .ok_or(SimError::UnknownListing(listing_id))?
            .clone();
        if quantity == 0 || quantity > order.quantity {
            return Err(SimError::InvalidQuantity);
        }
        let total = mul_money(quantity, listing.price_per_unit)?;
        let escrow_release = mul_money(quantity, order.limit_price_per_unit)?;
        let refund = escrow_release - total;
        let commodity = self.lots.get(&listing.lot).map(|lot| lot.commodity);

        let delivered = self.deliver_listed_lot(listing_id, order.buyer, quantity)?;

        self.credit(listing.seller, total)?;
        if refund > 0 {
            self.credit(order.buyer, refund)?;
        }
        let done = {
            let o = self.buy_orders.get_mut(&order_id).unwrap();
            o.quantity -= quantity;
            o.escrow -= escrow_release;
            o.quantity == 0
        };
        self.push_event(EventKind::TradeExecuted {
            listing: listing_id,
            market: listing.market,
            seller: listing.seller,
            buyer: order.buyer,
            lot_delivered: delivered,
            quantity,
            price_per_unit: listing.price_per_unit,
            total,
        });
        if let Some(commodity) = commodity {
            self.update_price_index(listing.market, commodity, listing.price_per_unit);
        }
        if done {
            let o = self.buy_orders.remove(&order_id).unwrap();
            if o.escrow > 0 {
                self.credit(o.buyer, o.escrow)?;
            }
            self.push_event(EventKind::BuyOrderClosed {
                order: order_id,
                refunded: o.escrow.max(0),
            });
        }
        Ok(())
    }

    /// Daily order matching: for each open buy order (in deterministic id
    /// order), repeatedly fill from the cheapest compatible listing whose
    /// price is within the limit. Executes at listing price.
    pub(crate) fn tick_market_matching(&mut self) {
        let order_ids: Vec<BuyOrderId> = self.buy_orders.keys().copied().collect();
        for order_id in order_ids {
            loop {
                let Some(order) = self.buy_orders.get(&order_id).cloned() else {
                    break;
                };
                let mut best: Option<(Credits, SellListingId)> = None;
                for (lid, listing) in &self.sell_listings {
                    if listing.seller == order.buyer {
                        continue;
                    }
                    if listing.price_per_unit > order.limit_price_per_unit {
                        continue;
                    }
                    if let OrderScope::Market(m) = order.scope {
                        if listing.market != m {
                            continue;
                        }
                    }
                    let Some(lot) = self.lots.get(&listing.lot) else {
                        continue;
                    };
                    if lot.commodity != order.commodity {
                        continue;
                    }
                    let candidate = (listing.price_per_unit, *lid);
                    if best.map_or(true, |b| candidate < b) {
                        best = Some(candidate);
                    }
                }
                let Some((_, listing_id)) = best else { break };
                let available = {
                    let l = &self.sell_listings[&listing_id];
                    self.lots[&l.lot].quantity
                };
                let qty = order.quantity.min(available);
                if self.fill_buy_order(order_id, listing_id, qty).is_err() {
                    break;
                }
            }
        }
    }

    /// Civilian demand: each city with population, an authority, and needs
    /// keeps one open buy order per needed commodity at its local market,
    /// budget permitting. Demand is real orders backed by real escrowed
    /// credits — if the authority is broke, demand goes unexpressed and
    /// shortage is visible in the order book.
    /// TODO(population): consumption destroying goods, per-capita scaling,
    /// prices feeding back into need urgency.
    pub(crate) fn tick_civilian_demand(&mut self) {
        let cities: Vec<LocationId> = self
            .locations
            .iter()
            .filter(|(_, l)| l.population > 0 && l.authority.is_some() && !l.civilian_needs.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for city in cities {
            let (authority, needs) = {
                let l = &self.locations[&city];
                (l.authority.unwrap(), l.civilian_needs.clone())
            };
            let Some(market_id) = self
                .markets
                .iter()
                .find(|(_, m)| m.site == city)
                .map(|(id, _)| *id)
            else {
                continue;
            };
            for need in needs {
                let already_open = self.buy_orders.values().any(|o| {
                    o.buyer == authority
                        && o.commodity == need.commodity
                        && o.scope == OrderScope::Market(market_id)
                });
                if already_open {
                    continue;
                }
                let Some(def) = self.commodities.get(&need.commodity) else {
                    continue;
                };
                // Bid slightly above the discovered market price so real
                // demand can clear against real scarcity; fall back to the
                // bootstrap base price until any trade has ever happened
                // here (docket TODO(markets): real price discovery).
                let reference = self.price_index(market_id, need.commodity).unwrap_or(def.base_price);
                let limit = reference.saturating_mul(110) / 100;
                // Errors (usually InsufficientFunds) are real constraints,
                // not bugs: a broke authority simply fails to buy food.
                let _ = self.place_buy_order(
                    authority,
                    OrderScope::Market(market_id),
                    need.commodity,
                    need.quantity_per_day,
                    limit,
                );
            }
        }
    }
}
