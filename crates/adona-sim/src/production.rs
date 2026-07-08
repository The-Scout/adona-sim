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
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecipeOutputs {
    /// Bulk output as a provenance-bearing lot.
    Commodity { commodity: CommodityId, quantity: u64 },
    /// Serial output: individual assets of an exact design.
    SerialAssets { design: DesignId, count: u32 },
    /// Component output: loose component instances.
    Components { def: ComponentDefId, count: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: RecipeId,
    pub name: String,
    /// Exact inputs: commodity and quantity. TODO(production): component
    /// inputs (mechs are assembled from real components, not just bulk).
    pub inputs: Vec<(CommodityId, u64)>,
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
    pub state: JobState,
}

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
}
