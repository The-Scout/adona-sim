//! Factories and production jobs.
//!
//! Factories consume exact real inputs and produce real outputs, over real
//! days. Inputs are specific lots physically at the factory's site; they are
//! consumed (state `ConsumedByProduction`, record kept) when the job starts.
//! Outputs are created at completion with `Produced`/`Manufactured` origins
//! pointing at the factory and job — full provenance both directions.
//!
//! Tooling binds a factory to an exact item design (docket: a tool for an
//! AC-5 is a tool for that specific AC-5 design). Recipes that require
//! tooling refuse to run without the matching tooling asset installed.
//!
//! TODO(factories): labor/power/QA as factory components, retooling downtime
//! and cost, commissioned production for non-owners, factory leasing.

use crate::assets::{AssetKind, AssetOrigin, ComponentCategory, ComponentPlacement};
use crate::events::EventKind;
use crate::goods::{LegalStatus, Lineage, LotOrigin, LotState, QualityGrade};
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecipeOutputs {
    /// Bulk output as a provenance-bearing lot.
    Commodity { commodity: CommodityId, quantity: u64 },
    /// Serial output: individual assets of an exact design.
    SerialAssets { design: DesignId, count: u32 },
    /// Component output: loose component instances.
    Components { def: ComponentDefId, count: u32 },
}

/// A real component requirement on a recipe: accept any of these defs (the
/// same "accept-list" shape `ComponentSlot` already uses for fitting), need
/// this many. Letting a recipe accept a family of interchangeable defs
/// (e.g. any tier of one component role) is what makes a single assembly
/// recipe work regardless of which tier its inputs were refined to, instead
/// of needing one recipe per exact tier combination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentRequirement {
    pub accepts: Vec<ComponentDefId>,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: RecipeId,
    pub name: String,
    /// Exact bulk inputs: commodity and quantity.
    pub inputs: Vec<(CommodityId, u64)>,
    /// Real component inputs consumed by the recipe (docket: "mechs are
    /// assembled from real components, not just bulk").
    pub component_inputs: Vec<ComponentRequirement>,
    pub outputs: RecipeOutputs,
    pub duration_days: u64,
    /// If set, the factory must have tooling of this exact design installed.
    pub required_tooling_design: Option<DesignId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factory {
    pub id: FactoryId,
    pub owner: ActorId,
    pub site: LocationId,
    /// Installed tooling asset (kind FactoryTooling), if any. Tooling decides
    /// *what exact item* the line makes; the five sub-system components
    /// below decide whether the factory can run at all.
    pub tooling: Option<AssetId>,
    /// Parallel line count. TODO(factories): enforce concurrent job limit.
    pub lines: u32,
    /// The five fixed sub-system components (labor, assembly line, power,
    /// control, quality-assurance). A factory cannot start production until
    /// all five are fitted (docket: Production And Factories — "factories
    /// themselves must be allowed to be componentized").
    pub components: BTreeMap<ComponentCategory, ComponentId>,
    /// Hard money + downtime cost paid the last time this factory's tooling
    /// changed. TODO(factories): quality rolls from retooling.
    pub last_retool_cost: Option<i64>,
    /// Day the factory becomes available again after a retool. Production
    /// cannot start while `today < retooling_until`.
    pub retooling_until: u64,
}

impl Factory {
    /// Which of the five required sub-system slots are still empty.
    pub fn missing_components(&self) -> Vec<ComponentCategory> {
        ComponentCategory::FACTORY_SLOTS
            .into_iter()
            .filter(|c| !self.components.contains_key(c))
            .collect()
    }

    pub fn is_operational(&self) -> bool {
        self.missing_components().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JobState {
    InProgress,
    Completed {
        output_lots: Vec<LotId>,
        output_assets: Vec<AssetId>,
        output_components: Vec<ComponentId>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionJob {
    pub id: ProductionJobId,
    pub factory: FactoryId,
    pub recipe: RecipeId,
    pub owner: ActorId,
    pub started_day: u64,
    pub completes_day: u64,
    /// The real input lots this job consumed, kept for provenance.
    pub consumed_lots: Vec<LotId>,
    /// The real input components this job consumed, kept for provenance.
    pub consumed_components: Vec<ComponentId>,
    pub state: JobState,
}

/// A candidate recipe for `tick_factory_auto_production`'s priority ranking:
/// (is tooled, output tier rank, real backlog, id, lots to offer, components
/// to offer) — see that function for what each field means.
type AutoProductionCandidate = (bool, u8, u64, RecipeId, Vec<LotId>, Vec<ComponentId>);

impl World {
    pub fn create_factory(
        &mut self,
        owner: ActorId,
        site: LocationId,
        lines: u32,
    ) -> Result<FactoryId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        let id = FactoryId(self.alloc());
        self.factories.insert(
            id,
            Factory {
                id,
                owner,
                site,
                tooling: None,
                lines,
                components: BTreeMap::new(),
                last_retool_cost: None,
                retooling_until: 0,
            },
        );
        self.push_event(EventKind::FactoryCreated { factory: id });
        Ok(id)
    }

    /// Install (or swap) a tooling asset into a factory. The tooling must be
    /// a real FactoryTooling serial asset, owned by the factory owner,
    /// physically at the factory's site. Retooling is a hard money cost paid
    /// immediately from the owner's treasury plus real downtime: the factory
    /// cannot start production again until `downtime_days` have passed
    /// (docket: "editing the components of an item is cheaper than changing
    /// the entire factory over to a different item" — this is the generic
    /// changeover cost that quote is comparing against).
    pub fn install_tooling(
        &mut self,
        factory: FactoryId,
        tooling: AssetId,
        cost: crate::actors::Credits,
        downtime_days: u64,
    ) -> Result<(), SimError> {
        let f = self
            .factories
            .get(&factory)
            .ok_or(SimError::UnknownFactory(factory))?
            .clone();
        if self.asset_kind(tooling)? != AssetKind::FactoryTooling {
            return Err(SimError::InvalidAssetKind(tooling));
        }
        let (owner, location) = {
            let a = self.assets.get(&tooling).ok_or(SimError::UnknownAsset(tooling))?;
            (a.owner, a.location)
        };
        if owner != f.owner {
            return Err(SimError::NotOwner { actor: owner });
        }
        if self.resolve_site(location) != Some(f.site) {
            return Err(SimError::NotColocated);
        }
        if cost > 0 {
            self.burn(f.owner, cost)?;
        }
        let retooling_until = self.clock.day + downtime_days;
        let entry = self.factories.get_mut(&factory).unwrap();
        entry.tooling = Some(tooling);
        entry.last_retool_cost = Some(cost);
        entry.retooling_until = retooling_until;
        self.push_event(EventKind::ToolingInstalled { factory, tooling, cost, retooling_until });
        Ok(())
    }

    pub fn define_recipe(
        &mut self,
        name: &str,
        inputs: Vec<(CommodityId, u64)>,
        component_inputs: Vec<ComponentRequirement>,
        outputs: RecipeOutputs,
        duration_days: u64,
        required_tooling_design: Option<DesignId>,
    ) -> RecipeId {
        let id = RecipeId(self.alloc());
        self.recipes.insert(
            id,
            Recipe {
                id,
                name: name.to_string(),
                inputs,
                component_inputs,
                outputs,
                duration_days: duration_days.max(1),
                required_tooling_design,
            },
        );
        self.push_event(EventKind::RecipeDefined { recipe: id });
        id
    }

    /// Start a production job. The offered lots must physically cover the
    /// recipe's exact inputs at the factory's site; they are consumed now
    /// (partially-needed lots are split so exactly the required quantity is
    /// consumed). Outputs appear at completion via `tick`.
    pub fn start_production(
        &mut self,
        factory_id: FactoryId,
        recipe_id: RecipeId,
        offered: &[LotId],
        offered_components: &[ComponentId],
    ) -> Result<ProductionJobId, SimError> {
        let factory = self
            .factories
            .get(&factory_id)
            .ok_or(SimError::UnknownFactory(factory_id))?
            .clone();
        let recipe = self
            .recipes
            .get(&recipe_id)
            .ok_or(SimError::UnknownRecipe(recipe_id))?
            .clone();

        if !factory.is_operational() {
            return Err(SimError::FactoryIncomplete {
                factory: factory_id,
                missing: factory.missing_components(),
            });
        }
        if self.clock.day < factory.retooling_until {
            return Err(SimError::InvalidState(format!(
                "{factory_id} is retooling until day {}",
                factory.retooling_until
            )));
        }

        if let Some(required) = recipe.required_tooling_design {
            let tooling = factory
                .tooling
                .ok_or(SimError::ToolingMismatch { factory: factory_id })?;
            let tooling_design = self
                .assets
                .get(&tooling)
                .ok_or(SimError::UnknownAsset(tooling))?
                .design;
            if tooling_design != required {
                return Err(SimError::ToolingMismatch { factory: factory_id });
            }
        }

        // Plan the takes first so a failed start consumes nothing.
        let mut takes: Vec<(LotId, u64)> = Vec::new();
        let mut remaining_by_lot: std::collections::BTreeMap<LotId, u64> = std::collections::BTreeMap::new();
        for &lot_id in offered {
            let l = self.lots.get(&lot_id).ok_or(SimError::UnknownLot(lot_id))?;
            if l.state != LotState::Active {
                return Err(SimError::LotNotActive(lot_id));
            }
            if l.owner != factory.owner {
                return Err(SimError::NotOwner { actor: l.owner });
            }
            if self.resolve_site(l.location) != Some(factory.site) {
                return Err(SimError::NotColocated);
            }
            remaining_by_lot.entry(lot_id).or_insert(l.quantity);
        }
        for (commodity, required) in &recipe.inputs {
            let mut needed = *required;
            for &lot_id in offered {
                if needed == 0 {
                    break;
                }
                let l = &self.lots[&lot_id];
                if l.commodity != *commodity {
                    continue;
                }
                let available = remaining_by_lot.get_mut(&lot_id).unwrap();
                if *available == 0 {
                    continue;
                }
                let take = needed.min(*available);
                *available -= take;
                needed -= take;
                takes.push((lot_id, take));
            }
            if needed > 0 {
                return Err(SimError::InsufficientQuantity {
                    commodity: *commodity,
                    missing: needed,
                });
            }
        }

        // Plan component takes the same way: real, physically-present, not
        // already spoken for, cheapest (lowest tier) first so a factory
        // doesn't burn its best stock when cheaper stock already satisfies
        // the requirement.
        let mut used_components: std::collections::BTreeSet<ComponentId> = std::collections::BTreeSet::new();
        let mut component_takes: Vec<ComponentId> = Vec::new();
        for requirement in &recipe.component_inputs {
            let mut candidates: Vec<ComponentId> = Vec::new();
            for &comp_id in offered_components {
                if used_components.contains(&comp_id) {
                    continue;
                }
                let Some(c) = self.components.get(&comp_id) else {
                    return Err(SimError::UnknownComponent(comp_id));
                };
                if c.owner != factory.owner {
                    return Err(SimError::NotOwner { actor: c.owner });
                }
                if !requirement.accepts.contains(&c.def) {
                    continue;
                }
                match c.placement {
                    ComponentPlacement::Loose(loc) if self.resolve_site(loc) == Some(factory.site) => {}
                    _ => continue,
                }
                candidates.push(comp_id);
            }
            candidates.sort_by_key(|id| {
                let tier = self.components.get(id).and_then(|c| self.component_defs.get(&c.def)).map(|d| d.tier);
                (tier, *id)
            });
            let take_n = requirement.count as usize;
            if candidates.len() < take_n {
                return Err(SimError::InsufficientComponents {
                    accepts: requirement.accepts.clone(),
                    missing: (take_n - candidates.len()) as u32,
                });
            }
            for &comp_id in candidates.iter().take(take_n) {
                used_components.insert(comp_id);
                component_takes.push(comp_id);
            }
        }

        // Commit: consume exactly the planned quantities.
        let job_id = ProductionJobId(self.alloc());
        let mut consumed: Vec<LotId> = Vec::new();
        for (lot_id, take) in takes {
            let full = self.lots[&lot_id].quantity;
            let victim = if take == full {
                lot_id
            } else {
                self.split_lot_internal(lot_id, take)?
            };
            self.lots.get_mut(&victim).unwrap().state = LotState::ConsumedByProduction(job_id);
            self.push_event(EventKind::LotConsumed {
                lot: victim,
                job: job_id,
            });
            consumed.push(victim);
        }
        for &comp_id in &component_takes {
            self.components.get_mut(&comp_id).unwrap().placement = ComponentPlacement::Consumed(job_id);
        }

        let started_day = self.clock.day;
        let completes_day = started_day + recipe.duration_days;
        self.production_jobs.insert(
            job_id,
            ProductionJob {
                id: job_id,
                factory: factory_id,
                recipe: recipe_id,
                owner: factory.owner,
                started_day,
                completes_day,
                consumed_components: component_takes,
                consumed_lots: consumed.clone(),
                state: JobState::InProgress,
            },
        );
        self.push_event(EventKind::ProductionStarted {
            job: job_id,
            factory: factory_id,
            recipe: recipe_id,
            consumed_lots: consumed,
        });
        Ok(job_id)
    }

    /// Complete due jobs: create real outputs with full provenance.
    pub(crate) fn tick_production(&mut self) {
        let today = self.clock.day;
        let due: Vec<ProductionJobId> = self
            .production_jobs
            .iter()
            .filter(|(_, j)| j.state == JobState::InProgress && j.completes_day <= today)
            .map(|(id, _)| *id)
            .collect();
        for job_id in due {
            let job = self.production_jobs[&job_id].clone();
            let recipe = self.recipes[&job.recipe].clone();
            let factory = self.factories[&job.factory].clone();
            let location = LocationRef::Site(factory.site);

            let mut output_lots = Vec::new();
            let mut output_assets = Vec::new();
            let mut output_components = Vec::new();

            match recipe.outputs {
                RecipeOutputs::Commodity { commodity, quantity } => {
                    let quality = self.roll_output_quality();
                    // Failure to create an output here would be a corrupted
                    // world (inputs were already validated); surface loudly.
                    let lot = self
                        .create_lot_raw(
                            job.owner,
                            commodity,
                            quantity,
                            quality,
                            LegalStatus::Legitimate,
                            location,
                            Lineage::Root(LotOrigin::Produced {
                                factory: job.factory,
                                job: job_id,
                            }),
                        )
                        .expect("production output creation must succeed");
                    output_lots.push(lot);
                }
                RecipeOutputs::SerialAssets { design, count } => {
                    for _ in 0..count {
                        let quality = self.roll_output_quality();
                        let asset = self
                            .create_asset_raw(
                                job.owner,
                                design,
                                location,
                                AssetOrigin::Manufactured {
                                    factory: job.factory,
                                    job: job_id,
                                },
                                quality,
                                None,
                            )
                            .expect("production output creation must succeed");
                        output_assets.push(asset);
                    }
                }
                RecipeOutputs::Components { def, count } => {
                    for _ in 0..count {
                        let quality = self.roll_output_quality();
                        let comp = self
                            .create_component_raw(
                                job.owner,
                                def,
                                ComponentPlacement::Loose(location),
                                AssetOrigin::Manufactured {
                                    factory: job.factory,
                                    job: job_id,
                                },
                                quality,
                            )
                            .expect("production output creation must succeed");
                        output_components.push(comp);
                    }
                }
            }

            self.production_jobs.get_mut(&job_id).unwrap().state = JobState::Completed {
                output_lots: output_lots.clone(),
                output_assets: output_assets.clone(),
                output_components: output_components.clone(),
            };
            self.push_event(EventKind::ProductionCompleted {
                job: job_id,
                output_lots,
                output_assets,
                output_components,
            });
        }
    }

    /// Quality roll for production outputs. Deterministic from the world
    /// seed. TODO(factories): quality from tooling tier, QA components,
    /// labor quality — not just a flat roll.
    fn roll_output_quality(&mut self) -> QualityGrade {
        let roll = self.rng.roll_percent();
        if roll < 10 {
            QualityGrade::Poor
        } else if roll >= 90 {
            QualityGrade::Fine
        } else {
            QualityGrade::Standard
        }
    }

    /// Read-only check: does `owner` physically hold enough real stock at
    /// `site` to cover `recipe`'s commodity and component requirements right
    /// now? Returns the exact lot/component ids to offer `start_production`
    /// if so — reads from an already-gathered local inventory snapshot
    /// rather than rescanning every lot/component in the world, so checking
    /// many recipe candidates against one factory stays cheap regardless of
    /// how large the rest of the world's inventory is.
    fn recipe_offer_from_inventory(
        &self,
        recipe: &Recipe,
        lots_by_commodity: &BTreeMap<CommodityId, Vec<(LotId, u64)>>,
        components_by_def: &BTreeMap<ComponentDefId, Vec<ComponentId>>,
    ) -> Option<(Vec<LotId>, Vec<ComponentId>)> {
        let mut lot_ids: Vec<LotId> = Vec::new();
        for (commodity, required) in &recipe.inputs {
            let matching = lots_by_commodity.get(commodity).map(Vec::as_slice).unwrap_or(&[]);
            let total: u64 = matching.iter().map(|(_, qty)| qty).sum();
            if total < *required {
                return None;
            }
            lot_ids.extend(matching.iter().map(|(id, _)| *id));
        }

        let mut used: BTreeSet<ComponentId> = BTreeSet::new();
        let mut comp_ids: Vec<ComponentId> = Vec::new();
        for requirement in &recipe.component_inputs {
            let mut candidates: Vec<ComponentId> = requirement
                .accepts
                .iter()
                .flat_map(|def| components_by_def.get(def).map(Vec::as_slice).unwrap_or(&[]).iter().copied())
                .filter(|id| !used.contains(id))
                .collect();
            if candidates.len() < requirement.count as usize {
                return None;
            }
            candidates.truncate(requirement.count as usize);
            for id in candidates {
                used.insert(id);
                comp_ids.push(id);
            }
        }
        Some((lot_ids, comp_ids))
    }

    /// Generic auto-production phase: every idle, operational, non-retooling
    /// factory tries to start whatever it can. Priority order: a recipe
    /// matching its installed tooling when one is ready; else the runnable
    /// untooled recipe with the highest output tier; ties broken by which
    /// candidate has the largest real backlog of its own input (so
    /// attention rotates across every material a factory could work on
    /// instead of always favoring whichever was defined first); a final
    /// `RecipeId` tie-break for full determinism. This is deliberately not a
    /// planner — it never looks more than one recipe ahead — but running it
    /// every day is enough to bootstrap a whole multi-step production chain
    /// bottom-up from raw mined material, with no per-material or per-chain
    /// special-casing.
    pub(crate) fn tick_factory_auto_production(&mut self) {
        let factory_ids: Vec<FactoryId> = self.factories.keys().copied().collect();
        for factory_id in factory_ids {
            let Some(factory) = self.factories.get(&factory_id).cloned() else { continue };
            if !factory.is_operational() || self.clock.day < factory.retooling_until {
                continue;
            }
            let already_running = self
                .production_jobs
                .values()
                .any(|j| j.factory == factory_id && j.state == JobState::InProgress);
            if already_running {
                continue;
            }
            let tooling_design = factory.tooling.and_then(|t| self.assets.get(&t)).map(|a| a.design);

            // Gather this factory's real local inventory once, not once per
            // candidate recipe — a mature economy can have thousands of
            // lots and hundreds of recipes, and rescanning everything per
            // recipe would make this phase scale with total world size
            // instead of with what's actually sitting at this one site.
            let mut lots_by_commodity: BTreeMap<CommodityId, Vec<(LotId, u64)>> = BTreeMap::new();
            for (id, l) in &self.lots {
                if l.owner == factory.owner && l.state == LotState::Active && self.resolve_site(l.location) == Some(factory.site) {
                    lots_by_commodity.entry(l.commodity).or_default().push((*id, l.quantity));
                }
            }
            let mut components_by_def: BTreeMap<ComponentDefId, Vec<ComponentId>> = BTreeMap::new();
            for (id, c) in &self.components {
                if c.owner == factory.owner
                    && matches!(c.placement, ComponentPlacement::Loose(loc) if self.resolve_site(loc) == Some(factory.site))
                {
                    components_by_def.entry(c.def).or_default().push(*id);
                }
            }

            let mut best: Option<AutoProductionCandidate> = None;
            for (recipe_id, recipe) in &self.recipes {
                if recipe.required_tooling_design.is_some() && recipe.required_tooling_design != tooling_design {
                    continue;
                }
                let Some((lots, comps)) = self.recipe_offer_from_inventory(recipe, &lots_by_commodity, &components_by_def) else {
                    continue;
                };
                let is_tooled_recipe = recipe.required_tooling_design.is_some();
                // A bulk refining step (`Commodity` output) always reports a
                // tier one higher than the component-conversion step for the
                // exact same input stock (refining tier N produces tier
                // N+1; converting tier N produces a tier-N component) — so
                // its raw tier is discounted by one here. That makes
                // "refine this stock further" and "cash this stock in as a
                // real component now" tie for the same input, with the tie
                // broken by backlog below (in practice: conversion, since a
                // material chain's convert recipes are generated before its
                // refine recipes and so is checked as the tie-break last
                // resort). Without this, refining would always numerically
                // outrank cashing in, and factories would refine forever,
                // never actually producing a component or a finished asset.
                let tier_rank: u8 = match &recipe.outputs {
                    RecipeOutputs::SerialAssets { .. } => u8::MAX,
                    RecipeOutputs::Commodity { commodity, .. } => {
                        self.commodities.get(commodity).map(|c| c.tier.saturating_sub(1)).unwrap_or(0)
                    }
                    RecipeOutputs::Components { def, .. } => {
                        self.component_defs.get(def).map(|d| d.tier).unwrap_or(0)
                    }
                };
                // Backlog (real available input quantity) is the next
                // tie-break, ahead of `RecipeId` — without it, a fixed
                // recipe id ordering would let a factory's earliest-defined
                // material win every single tie forever, starving whichever
                // materials happen to sort last even as their own real stock
                // piles up unboundedly. Preferring the largest real backlog
                // makes attention rotate toward whichever material has
                // actually been neglected longest.
                let backlog: u64 = recipe
                    .inputs
                    .iter()
                    .map(|(commodity, _)| lots_by_commodity.get(commodity).map(|v| v.iter().map(|(_, qty)| qty).sum()).unwrap_or(0))
                    .sum();
                let is_better = match &best {
                    None => true,
                    Some((b_tooled, b_tier, b_backlog, b_id, ..)) => {
                        (is_tooled_recipe, tier_rank, backlog) > (*b_tooled, *b_tier, *b_backlog)
                            || ((is_tooled_recipe, tier_rank, backlog) == (*b_tooled, *b_tier, *b_backlog) && recipe_id < b_id)
                    }
                };
                if is_better {
                    best = Some((is_tooled_recipe, tier_rank, backlog, *recipe_id, lots, comps));
                }
            }

            if let Some((_, _, _, recipe_id, lots, comps)) = best {
                let _ = self.start_production(factory_id, recipe_id, &lots, &comps);
            }
        }
    }
}
