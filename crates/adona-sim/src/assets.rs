//! Serial assets, designs, and components.
//!
//! Player-facing weapons, equipment, mechs, vehicles, major components,
//! factory tooling, and meaningful cargo containers are serial entities with
//! identity and history. Components are their own primitive: named, typed,
//! and compatibility-bound by data ("an actuator for one mech model is an
//! actuator for that model"). Generic parts are not ADONA's default.
//!
//! TODO(modes): a looser "generic mode" (Starsector-like categories) for
//! other projects; ADONA targets exact mode and exact mode is what is built.

use crate::events::EventKind;
use crate::goods::QualityGrade;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Mech,
    Vehicle,
    Weapon,
    Equipment,
    /// Binds a factory to an exact item design (docket: Tooling).
    FactoryTooling,
    CargoContainer,
}

/// Minimum distinct component slots a mech design must declare (docket:
/// component priors — a mech is assembled from real, named components, not a
/// monolithic stat block).
pub const MECH_COMPONENT_SLOTS: usize = 5;
/// Weapons and equipment are built from 5-6 sub-components per the docket.
pub const EQUIPMENT_COMPONENT_SLOTS_MIN: usize = 5;
pub const EQUIPMENT_COMPONENT_SLOTS_MAX: usize = 6;

/// A component slot on a design. Compatibility is an explicit accept-list —
/// checked by data, never assumed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSlot {
    pub name: String,
    pub accepts: Vec<ComponentDefId>,
}

/// What kind of thing a component definition can be fitted into. Mech and
/// equipment components share one general-purpose category (compatibility is
/// still enforced per-slot by `accepts`); the five Factory categories are
/// fixed, named factory sub-systems (docket: Production And Factories) and
/// each factory needs exactly one component in each to operate at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ComponentCategory {
    MechOrEquipment,
    FactoryLabor,
    FactoryAssemblyLine,
    FactoryPower,
    FactoryControl,
    FactoryQualityAssurance,
}

impl ComponentCategory {
    /// The five fixed factory sub-system slots every factory must fill.
    pub const FACTORY_SLOTS: [ComponentCategory; 5] = [
        ComponentCategory::FactoryLabor,
        ComponentCategory::FactoryAssemblyLine,
        ComponentCategory::FactoryPower,
        ComponentCategory::FactoryControl,
        ComponentCategory::FactoryQualityAssurance,
    ];

    pub fn is_factory_slot(self) -> bool {
        self != ComponentCategory::MechOrEquipment
    }
}

/// A design (blueprint) for serial assets of one exact model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemDesign {
    pub id: DesignId,
    pub name: String,
    pub kind: AssetKind,
    pub slots: Vec<ComponentSlot>,
    /// Cargo capacity for vehicles and containers, in kilograms.
    /// TODO(logistics): enforce capacity against real cargo mass once
    /// commodities carry per-unit mass.
    pub cargo_capacity_kg: Option<u64>,
    /// For FactoryTooling designs: the exact item design this tooling
    /// produces. A tool for an AC-5 is a tool for that specific AC-5 design.
    pub tooling_for: Option<DesignId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDef {
    pub id: ComponentDefId,
    pub name: String,
    pub tier: u8,
    pub category: ComponentCategory,
}

/// Where a real thing came from at creation. Later capture, salvage, and
/// purchase are ownership-change events in the log, not origin rewrites.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssetOrigin {
    Manufactured { factory: FactoryId, job: ProductionJobId },
    SeededHistorical { note: String },
    Imported { source: String },
    AdminPlaced { operator: ActorId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComponentPlacement {
    /// Installed in a serial asset; physical position follows the asset.
    Fitted(AssetId),
    /// Installed in a factory's fixed sub-system slot.
    FittedFactory(FactoryId),
    /// Loose stock with its own location.
    Loose(LocationRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentInstance {
    pub id: ComponentId,
    pub def: ComponentDefId,
    pub owner: ActorId,
    pub placement: ComponentPlacement,
    pub origin: AssetOrigin,
    pub quality: QualityGrade,
    pub created_day: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerialAsset {
    pub id: AssetId,
    pub design: DesignId,
    /// Optional individual name ("Old Reliable").
    pub name: Option<String>,
    pub owner: ActorId,
    pub location: LocationRef,
    pub origin: AssetOrigin,
    pub quality: QualityGrade,
    /// 0–100. TODO(combat/repair): real damage model, repairs consuming real
    /// components and materials.
    pub condition_pct: u8,
    /// Slot index -> fitted component.
    pub fitted: BTreeMap<u32, ComponentId>,
    pub created_day: u64,
}

impl World {
    /// Define an item design. Mechs must declare exactly
    /// [`MECH_COMPONENT_SLOTS`] distinct component slots; weapons and
    /// equipment must declare between [`EQUIPMENT_COMPONENT_SLOTS_MIN`] and
    /// [`EQUIPMENT_COMPONENT_SLOTS_MAX`] (docket: "most weapons and equipment
    /// are built from 5-6 sub-components"). A design is not a monolithic stat
    /// block; it is real components, checked here at definition time so bad
    /// data never enters the world.
    pub fn define_design(
        &mut self,
        name: &str,
        kind: AssetKind,
        slots: Vec<ComponentSlot>,
        cargo_capacity_kg: Option<u64>,
        tooling_for: Option<DesignId>,
    ) -> Result<DesignId, SimError> {
        match kind {
            AssetKind::Mech if slots.len() != MECH_COMPONENT_SLOTS => {
                return Err(SimError::InvalidSlotCount { kind, got: slots.len() });
            }
            AssetKind::Weapon | AssetKind::Equipment
                if !(EQUIPMENT_COMPONENT_SLOTS_MIN..=EQUIPMENT_COMPONENT_SLOTS_MAX)
                    .contains(&slots.len()) =>
            {
                return Err(SimError::InvalidSlotCount { kind, got: slots.len() });
            }
            _ => {}
        }
        let id = DesignId(self.alloc());
        self.designs.insert(
            id,
            ItemDesign {
                id,
                name: name.to_string(),
                kind,
                slots,
                cargo_capacity_kg,
                tooling_for,
            },
        );
        self.push_event(EventKind::DesignDefined { design: id });
        Ok(id)
    }

    pub fn define_component_def(
        &mut self,
        name: &str,
        tier: u8,
        category: ComponentCategory,
    ) -> ComponentDefId {
        let id = ComponentDefId(self.alloc());
        self.component_defs.insert(
            id,
            ComponentDef {
                id,
                name: name.to_string(),
                tier,
                category,
            },
        );
        self.push_event(EventKind::ComponentDefDefined { def: id });
        id
    }

    /// Internal asset creation — all real assets pass through here.
    pub(crate) fn create_asset_raw(
        &mut self,
        owner: ActorId,
        design: DesignId,
        location: LocationRef,
        origin: AssetOrigin,
        quality: QualityGrade,
        name: Option<&str>,
    ) -> Result<AssetId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.designs.contains_key(&design) {
            return Err(SimError::UnknownDesign(design));
        }
        if !self.location_ref_valid(location) {
            return Err(SimError::NotColocated);
        }
        let id = AssetId(self.alloc());
        let created_day = self.clock.day;
        self.assets.insert(
            id,
            SerialAsset {
                id,
                design,
                name: name.map(str::to_string),
                owner,
                location,
                origin,
                quality,
                condition_pct: 100,
                fitted: BTreeMap::new(),
                created_day,
            },
        );
        self.push_event(EventKind::AssetCreated {
            asset: id,
            design,
            owner,
        });
        Ok(id)
    }

    /// Seed a serial asset (pre-war inventory, import, admin placement).
    /// Manufactured origins are rejected: manufactured assets come from
    /// production jobs only.
    pub fn seed_asset(
        &mut self,
        owner: ActorId,
        design: DesignId,
        site: LocationId,
        quality: QualityGrade,
        origin: AssetOrigin,
        name: Option<&str>,
    ) -> Result<AssetId, SimError> {
        if matches!(origin, AssetOrigin::Manufactured { .. }) {
            return Err(SimError::InvalidOrigin(
                "seed_asset does not accept Manufactured origins".into(),
            ));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        self.create_asset_raw(owner, design, LocationRef::Site(site), origin, quality, name)
    }

    /// Seed a loose component instance.
    pub fn seed_component(
        &mut self,
        owner: ActorId,
        def: ComponentDefId,
        site: LocationId,
        quality: QualityGrade,
        origin: AssetOrigin,
    ) -> Result<ComponentId, SimError> {
        if matches!(origin, AssetOrigin::Manufactured { .. }) {
            return Err(SimError::InvalidOrigin(
                "seed_component does not accept Manufactured origins".into(),
            ));
        }
        if !self.component_defs.contains_key(&def) {
            return Err(SimError::UnknownComponentDef(def));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        self.create_component_raw(owner, def, ComponentPlacement::Loose(LocationRef::Site(site)), origin, quality)
    }

    pub(crate) fn create_component_raw(
        &mut self,
        owner: ActorId,
        def: ComponentDefId,
        placement: ComponentPlacement,
        origin: AssetOrigin,
        quality: QualityGrade,
    ) -> Result<ComponentId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        let id = ComponentId(self.alloc());
        let created_day = self.clock.day;
        self.components.insert(
            id,
            ComponentInstance {
                id,
                def,
                owner,
                placement,
                origin,
                quality,
                created_day,
            },
        );
        self.push_event(EventKind::ComponentCreated {
            component: id,
            def,
            owner,
        });
        Ok(id)
    }

    /// Fit a loose component into a design slot. Compatibility is checked
    /// against the slot's accept list; the component must be colocated with
    /// the asset and share its owner.
    /// TODO(refit): unfitting, swap, and refit downtime/labor costs.
    pub fn fit_component(
        &mut self,
        component: ComponentId,
        asset: AssetId,
        slot: u32,
    ) -> Result<(), SimError> {
        let (comp_def, comp_owner, comp_placement) = {
            let c = self
                .components
                .get(&component)
                .ok_or(SimError::UnknownComponent(component))?;
            (c.def, c.owner, c.placement.clone())
        };
        let (asset_design, asset_owner, asset_location) = {
            let a = self.assets.get(&asset).ok_or(SimError::UnknownAsset(asset))?;
            (a.design, a.owner, a.location)
        };
        let comp_category = self
            .component_defs
            .get(&comp_def)
            .ok_or(SimError::UnknownComponentDef(comp_def))?
            .category;
        if comp_category != ComponentCategory::MechOrEquipment {
            return Err(SimError::WrongComponentCategory { component });
        }
        let design = self
            .designs
            .get(&asset_design)
            .ok_or(SimError::UnknownDesign(asset_design))?;
        let slot_def = design
            .slots
            .get(slot as usize)
            .ok_or(SimError::IncompatibleComponent { component, asset, slot })?;
        if !slot_def.accepts.contains(&comp_def) {
            return Err(SimError::IncompatibleComponent { component, asset, slot });
        }
        if comp_owner != asset_owner {
            return Err(SimError::NotOwner { actor: comp_owner });
        }
        match comp_placement {
            ComponentPlacement::Fitted(_) | ComponentPlacement::FittedFactory(_) => {
                return Err(SimError::InvalidState("component is already fitted".into()))
            }
            ComponentPlacement::Loose(loc) => {
                if self.resolve_site(loc).is_none()
                    || self.resolve_site(loc) != self.resolve_site(asset_location)
                {
                    return Err(SimError::NotColocated);
                }
            }
        }
        if self.assets[&asset].fitted.contains_key(&slot) {
            return Err(SimError::SlotOccupied { asset, slot });
        }
        self.assets.get_mut(&asset).unwrap().fitted.insert(slot, component);
        self.components.get_mut(&component).unwrap().placement =
            ComponentPlacement::Fitted(asset);
        self.push_event(EventKind::ComponentFitted {
            component,
            asset,
            slot,
        });
        Ok(())
    }

    /// Fit a loose component into one of a factory's five fixed sub-system
    /// slots (docket: Production And Factories — labor, assembly line,
    /// power, control, quality-assurance). The component's category decides
    /// which slot it fills; each slot takes exactly one component, and a
    /// factory cannot run production until all five are filled.
    pub fn fit_factory_component(
        &mut self,
        factory: FactoryId,
        component: ComponentId,
    ) -> Result<(), SimError> {
        let f = self
            .factories
            .get(&factory)
            .ok_or(SimError::UnknownFactory(factory))?
            .clone();
        let (comp_def, comp_owner, comp_placement) = {
            let c = self
                .components
                .get(&component)
                .ok_or(SimError::UnknownComponent(component))?;
            (c.def, c.owner, c.placement.clone())
        };
        let category = self
            .component_defs
            .get(&comp_def)
            .ok_or(SimError::UnknownComponentDef(comp_def))?
            .category;
        if !category.is_factory_slot() {
            return Err(SimError::WrongComponentCategory { component });
        }
        if comp_owner != f.owner {
            return Err(SimError::NotOwner { actor: comp_owner });
        }
        match comp_placement {
            ComponentPlacement::Fitted(_) | ComponentPlacement::FittedFactory(_) => {
                return Err(SimError::InvalidState("component is already fitted".into()))
            }
            ComponentPlacement::Loose(loc) => {
                if self.resolve_site(loc) != Some(f.site) {
                    return Err(SimError::NotColocated);
                }
            }
        }
        if f.components.contains_key(&category) {
            return Err(SimError::FactorySlotOccupied { factory, category });
        }
        self.factories.get_mut(&factory).unwrap().components.insert(category, component);
        self.components.get_mut(&component).unwrap().placement =
            ComponentPlacement::FittedFactory(factory);
        self.push_event(EventKind::ComponentFittedToFactory {
            component,
            factory,
            category,
        });
        Ok(())
    }

    /// Transfer ownership of a serial asset (trade, capture resolution,
    /// contract salvage settlement). Fitted components transfer with it.
    pub fn transfer_asset(&mut self, asset: AssetId, to: ActorId) -> Result<(), SimError> {
        if !self.actors.contains_key(&to) {
            return Err(SimError::UnknownActor(to));
        }
        let (from, fitted) = {
            let a = self.assets.get_mut(&asset).ok_or(SimError::UnknownAsset(asset))?;
            let from = a.owner;
            a.owner = to;
            (from, a.fitted.values().copied().collect::<Vec<_>>())
        };
        for comp in fitted {
            if let Some(c) = self.components.get_mut(&comp) {
                c.owner = to;
            }
        }
        self.push_event(EventKind::AssetOwnerChanged { asset, from, to });
        Ok(())
    }

    /// Stow a lot inside a serial cargo container. The container must be a
    /// CargoContainer and physically colocated with the lot.
    pub fn stow_lot_in_container(&mut self, lot: LotId, container: AssetId) -> Result<(), SimError> {
        let container_asset = self
            .assets
            .get(&container)
            .ok_or(SimError::UnknownAsset(container))?;
        let design = self
            .designs
            .get(&container_asset.design)
            .ok_or(SimError::UnknownDesign(container_asset.design))?;
        if design.kind != AssetKind::CargoContainer {
            return Err(SimError::InvalidAssetKind(container));
        }
        let container_site = self.resolve_site(container_asset.location);
        let lot_loc = self.lots.get(&lot).ok_or(SimError::UnknownLot(lot))?.location;
        if container_site.is_none() || self.resolve_site(lot_loc) != container_site {
            return Err(SimError::NotColocated);
        }
        self.move_lot_raw(lot, LocationRef::Container(container))
    }

    /// Internal asset move.
    pub(crate) fn move_asset_raw(&mut self, asset: AssetId, to: LocationRef) -> Result<(), SimError> {
        let from = {
            let a = self.assets.get_mut(&asset).ok_or(SimError::UnknownAsset(asset))?;
            let from = a.location;
            a.location = to;
            from
        };
        self.push_event(EventKind::AssetMoved { asset, from, to });
        Ok(())
    }

    pub(crate) fn asset_kind(&self, asset: AssetId) -> Result<AssetKind, SimError> {
        let a = self.assets.get(&asset).ok_or(SimError::UnknownAsset(asset))?;
        Ok(self
            .designs
            .get(&a.design)
            .ok_or(SimError::UnknownDesign(a.design))?
            .kind)
    }
}
