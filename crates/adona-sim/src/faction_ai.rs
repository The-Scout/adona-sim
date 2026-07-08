//! Faction procurement AI: the planner that consumes [`crate::toe::ToeShortage`].
//!
//! A [`FactionGoal`] is a standing order: "keep this TO&E template filled at
//! this site." Each tick, for every open goal, the world computes real
//! shortages (never a wishlist against fake inventory) and tries two things,
//! in order:
//!
//! 1. If the owner already has an operational factory tooled for the
//!    missing design, and enough real input lots on hand at that factory's
//!    site, start real production toward the shortfall.
//! 2. Otherwise, place a real escrowed buy order for whatever recipe inputs
//!    are short, so the next tick's production attempt has a chance to
//!    succeed — demand becomes a real market order, not a note to self.
//!
//! Nothing here ever spawns an asset or lot directly. Every unit the AI
//! fields still has to come from production or trade, exactly like a player.

use crate::actors::Credits;
use crate::convoys::Route;
use crate::events::EventKind;
use crate::goods::LotState;
use crate::ids::*;
use crate::markets::OrderScope;
use crate::production::RecipeOutputs;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactionGoal {
    pub id: FactionGoalId,
    pub owner: ActorId,
    pub template: ToeTemplateId,
    pub site: LocationId,
}

impl World {
    /// Register a standing TO&E goal for a faction. The faction AI phase of
    /// `tick` will try to fill it every day from real production and trade
    /// for as long as the goal exists.
    pub fn set_faction_goal(
        &mut self,
        owner: ActorId,
        template: ToeTemplateId,
        site: LocationId,
    ) -> Result<FactionGoalId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.toe_templates.contains_key(&template) {
            return Err(SimError::UnknownToeTemplate(template));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        let id = FactionGoalId(self.alloc());
        self.faction_goals.insert(id, FactionGoal { id, owner, template, site });
        self.push_event(EventKind::FactionGoalSet { goal: id, owner, template, site });
        Ok(id)
    }

    pub fn faction_goal(&self, id: FactionGoalId) -> Option<&FactionGoal> {
        self.faction_goals.get(&id)
    }

    /// Faction planning phase: convert each goal's real TO&E shortage into
    /// real production or real procurement orders. This is the planner the
    /// docket's `ToeShortage` demand signal was always meant to feed.
    pub(crate) fn tick_faction_ai(&mut self) {
        let goals: Vec<FactionGoal> = self.faction_goals.values().cloned().collect();
        for goal in goals {
            let Ok(shortages) = self.toe_shortages(goal.owner, goal.template, goal.site) else {
                continue;
            };
            for shortage in shortages {
                if self.try_produce_toward_shortage(&goal, shortage.design) {
                    continue;
                }
                self.order_inputs_toward_shortage(&goal, shortage.design);
            }
        }
    }

    /// Attempt (1): a real, already-tooled, already-stocked factory can run
    /// the recipe right now. Returns true if a job was started.
    fn try_produce_toward_shortage(&mut self, goal: &FactionGoal, design: DesignId) -> bool {
        let candidate_factories: Vec<FactoryId> = self
            .factories
            .iter()
            .filter(|(_, f)| f.owner == goal.owner && f.is_operational() && self.clock.day >= f.retooling_until)
            .map(|(id, _)| *id)
            .collect();

        for factory_id in candidate_factories {
            let factory_site = self.factories[&factory_id].site;
            let Some(tooling) = self.factories[&factory_id].tooling else { continue };
            let Some(tooling_design) = self.assets.get(&tooling).map(|a| a.design) else { continue };

            let Some((recipe_id, recipe)) = self.recipes.iter().find(|(_, r)| {
                matches!(&r.outputs, RecipeOutputs::SerialAssets { design: d, .. } if *d == design)
                    && r.required_tooling_design == Some(tooling_design)
            }) else {
                continue;
            };

            // Gather real, active, owner-held lots at the factory's site
            // that could cover the recipe's inputs.
            let mut offered: Vec<LotId> = Vec::new();
            let mut covered: BTreeMap<CommodityId, u64> = BTreeMap::new();
            for (lid, lot) in &self.lots {
                if lot.owner != goal.owner || lot.state != LotState::Active {
                    continue;
                }
                if self.resolve_site(lot.location) != Some(factory_site) {
                    continue;
                }
                if !recipe.inputs.iter().any(|(c, _)| *c == lot.commodity) {
                    continue;
                }
                *covered.entry(lot.commodity).or_insert(0) += lot.quantity;
                offered.push(*lid);
            }
            let has_enough = recipe.inputs.iter().all(|(c, qty)| covered.get(c).copied().unwrap_or(0) >= *qty);
            if !has_enough {
                continue;
            }

            let recipe_id = *recipe_id;
            if let Ok(job) = self.start_production(factory_id, recipe_id, &offered) {
                self.push_event(EventKind::FactionProcurementStarted { goal: goal.id, job });
                return true;
            }
        }
        false
    }

    /// Attempt (2): no factory can run yet — place real buy orders for
    /// whatever recipe inputs are short, so a future tick can produce.
    /// Skips commodities the faction already has an open global order for,
    /// mirroring civilian demand's one-order-at-a-time discipline.
    fn order_inputs_toward_shortage(&mut self, goal: &FactionGoal, design: DesignId) {
        let Some((_, recipe)) = self
            .recipes
            .iter()
            .find(|(_, r)| matches!(&r.outputs, RecipeOutputs::SerialAssets { design: d, .. } if *d == design))
            .map(|(id, r)| (*id, r.clone()))
        else {
            return;
        };

        for (commodity, required_qty) in recipe.inputs.clone() {
            let owned: u64 = self
                .lots
                .values()
                .filter(|l| l.owner == goal.owner && l.commodity == commodity && l.state == LotState::Active)
                .map(|l| l.quantity)
                .sum();
            if owned >= required_qty {
                continue;
            }
            let already_ordering = self.buy_orders.values().any(|o| {
                o.buyer == goal.owner && o.commodity == commodity && o.scope == OrderScope::Global
            });
            if already_ordering {
                continue;
            }
            let Some(def) = self.commodities.get(&commodity) else { continue };
            let reference = self.price_index_anywhere(commodity).unwrap_or(def.base_price);
            let limit: Credits = reference.saturating_mul(115) / 100;
            let missing = required_qty - owned;
            if let Ok(order) = self.place_buy_order(goal.owner, OrderScope::Global, commodity, missing, limit) {
                self.push_event(EventKind::FactionProcurementOrdered { goal: goal.id, order, commodity });
            }
        }
    }

    /// Best-effort price reference for a commodity across every market that
    /// has ever traded it (used when the faction has no home market yet).
    fn price_index_anywhere(&self, commodity: CommodityId) -> Option<Credits> {
        self.price_index
            .values()
            .filter_map(|by_commodity| by_commodity.get(&commodity))
            .copied()
            .max()
    }

    /// Automatic deployment phase: a formation stationed on ground its own
    /// owner controls marches toward the first (by route id, for
    /// determinism) adjacent site it does *not* control — contested or
    /// enemy-held territory — instead of sitting still forever. This is the
    /// war AI's answer to docket TODO(war): factions actively press toward
    /// contested/enemy ground rather than only fighting where formations
    /// already happen to be. A formation already standing on contested or
    /// enemy ground is left alone; that's what `tick_faction_war` is for.
    pub(crate) fn tick_faction_deployment(&mut self) {
        let formations: Vec<FormationId> = self.formations.keys().copied().collect();
        for fid in formations {
            let Some(formation) = self.formations.get(&fid) else { continue };
            let Some(at) = formation.current_site() else { continue };
            let owner = formation.owner;
            let home_controller = self.locations.get(&at).and_then(|l| l.controller);
            if home_controller != Some(owner) {
                continue;
            }
            let mut routes: Vec<&Route> = self.routes.values().filter(|r| r.from == at).collect();
            routes.sort_by_key(|r| r.id);
            for route in routes {
                let dest_controller = self.locations.get(&route.to).and_then(|l| l.controller);
                if dest_controller != Some(owner) {
                    let route_id = route.id;
                    let _ = self.order_formation_march(fid, route_id);
                    break;
                }
            }
        }
    }
}
