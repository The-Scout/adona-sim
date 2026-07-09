//! Reusable content-authoring helpers.
//!
//! Everything here is generic simulation machinery, not ADONA content: no
//! material names, tier counts, or faction numbers are hardcoded. A
//! consumer (`adona-game`, or an entirely different project) supplies its
//! own names, tier count, and quantities through a spec struct and gets back
//! the real typed ids the generator created — see `CLAUDE.md`'s modularity
//! rule, which this module is the worked example for.
//!
//! [`World::generate_tiered_material_chain`] builds one raw-material ->
//! tiered-refining -> tiered-component pipeline per call. Call it once per
//! material a game wants (ADONA calls it 5 times, for 5 materials, each with
//! Adona's own 13 tier names — a different game could call it 3 times with
//! 6 tiers and completely different names and get a working tech tree from
//! the same code).

use crate::actors::Credits;
use crate::assets::ComponentCategory;
use crate::goods::UnitOfMeasure;
use crate::ids::{ComponentDefId, CommodityId, RecipeId};
use crate::production::{ComponentRequirement, RecipeOutputs};
use crate::world::World;

/// Caller-supplied description of one tiered material chain. Nothing here
/// defaults to an ADONA-specific value — every field is provided by the
/// content calling this, whether that's `adona-game` or another project.
#[derive(Debug, Clone)]
pub struct TieredChainSpec {
    /// e.g. "Ferrite" — used to name the generated commodities/components.
    pub material_name: String,
    /// One name per tier, lowest tier first. Length decides how many tiers
    /// this chain has; there is no fixed tier count in the engine.
    pub tier_names: Vec<String>,
    pub unit: UnitOfMeasure,
    pub base_price: Credits,
    pub component_category: ComponentCategory,
    /// Raw/refined commodity consumed per refining step.
    pub refine_input_qty: u64,
    /// Refined commodity produced per refining step (real yield loss when
    /// less than the input quantity).
    pub refine_output_qty: u64,
    pub refine_duration_days: u64,
    /// Commodity consumed to convert one tier's material into one component
    /// of that same tier.
    pub convert_input_qty: u64,
    pub convert_duration_days: u64,
}

/// The real ids `generate_tiered_material_chain` created, one entry per tier
/// (`refine_recipes` has one fewer, since it links tier `i` to tier `i+1`).
#[derive(Debug, Clone)]
pub struct TieredChainHandles {
    pub commodities: Vec<CommodityId>,
    pub refine_recipes: Vec<RecipeId>,
    pub component_defs: Vec<ComponentDefId>,
    pub convert_recipes: Vec<RecipeId>,
}

impl World {
    /// Build a full N-tier raw-material-to-component chain from a spec:
    /// one commodity, one component def, and one "convert to component"
    /// recipe per tier, plus a "refine to the next tier" recipe between
    /// each consecutive pair. Generic over any material identity and any
    /// tier count — see the module docs.
    pub fn generate_tiered_material_chain(&mut self, spec: &TieredChainSpec) -> TieredChainHandles {
        let tiers = spec.tier_names.len();

        let commodities: Vec<CommodityId> = spec
            .tier_names
            .iter()
            .enumerate()
            .map(|(i, tier_name)| {
                self.define_commodity(
                    &format!("{tier_name} {}", spec.material_name),
                    spec.unit,
                    (i + 1) as u8,
                    spec.base_price,
                )
            })
            .collect();

        let component_defs: Vec<ComponentDefId> = spec
            .tier_names
            .iter()
            .enumerate()
            .map(|(i, tier_name)| {
                self.define_component_def(
                    &format!("{tier_name} {} Component", spec.material_name),
                    (i + 1) as u8,
                    spec.component_category,
                )
            })
            .collect();

        let convert_recipes: Vec<RecipeId> = (0..tiers)
            .map(|i| {
                self.define_recipe(
                    &format!("Convert {} {} to Component", spec.tier_names[i], spec.material_name),
                    vec![(commodities[i], spec.convert_input_qty)],
                    vec![],
                    RecipeOutputs::Components { def: component_defs[i], count: 1 },
                    spec.convert_duration_days,
                    None,
                )
            })
            .collect();

        let refine_recipes: Vec<RecipeId> = (0..tiers.saturating_sub(1))
            .map(|i| {
                self.define_recipe(
                    &format!(
                        "Refine {} {} to {} {}",
                        spec.tier_names[i], spec.material_name, spec.tier_names[i + 1], spec.material_name
                    ),
                    vec![(commodities[i], spec.refine_input_qty)],
                    vec![],
                    RecipeOutputs::Commodity { commodity: commodities[i + 1], quantity: spec.refine_output_qty },
                    spec.refine_duration_days,
                    None,
                )
            })
            .collect();

        TieredChainHandles { commodities, refine_recipes, component_defs, convert_recipes }
    }
}

impl TieredChainHandles {
    /// A `ComponentRequirement` accepting any tier of this chain's
    /// components — the shape a final assembly recipe uses so it works
    /// regardless of which tier the input material actually reached.
    pub fn any_tier_requirement(&self, count: u32) -> ComponentRequirement {
        ComponentRequirement { accepts: self.component_defs.clone(), count }
    }
}
