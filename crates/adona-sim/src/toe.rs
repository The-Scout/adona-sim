//! Table of Organization & Equipment: the core faction-AI target.
//!
//! Doctrine defines desired formations; the economy fills them with real
//! assets. `try_assemble_formation` pulls only from assets the faction
//! physically has at the assembly site (open ground or its stockpiles
//! there). When assets are missing it returns a typed shortage list — that
//! shortage IS the demand signal the faction AI will convert into
//! production orders, market buys, salvage priorities, and refits.
//!
//! Strict rule enforced here: a formation cannot contain a single asset the
//! faction does not own and physically hold. No spawned faction mechs.
//!
//! [`crate::faction_ai`] is the planner that consumes [`ToeShortage`] and
//! drives procurement. TODO(faction-ai): pulling formations back to refit;
//! deployment/territory orders (see the RISK-style combat resolution and war
//! AI docket items); personnel/pilot assignment (pilot generation is the one
//! accepted spawn exception per the docket).

use crate::events::EventKind;
use crate::ids::*;
use crate::locations::LocationRef;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

/// One line of a TO&E template: this many assets of this exact design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToeSlot {
    /// Role label ("line mech", "scout", "supply truck").
    pub role: String,
    pub design: DesignId,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToeTemplate {
    pub id: ToeTemplateId,
    pub name: String,
    /// Doctrine tag this template belongs to.
    pub doctrine: String,
    pub slots: Vec<ToeSlot>,
}

/// A concrete shortage: real demand for real gear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToeShortage {
    pub role: String,
    pub design: DesignId,
    pub missing: u32,
}

/// Where a formation physically is: stationed at a site (able to fight,
/// assemble, and receive march orders) or en route along a real route (not
/// at any site, cannot fight until it arrives — mirrors [`crate::convoys::ConvoyState`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FormationState {
    Stationed { at: LocationId },
    EnRoute {
        route: RouteId,
        departed_day: u64,
        arrives_day: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Formation {
    pub id: FormationId,
    pub owner: ActorId,
    pub template: ToeTemplateId,
    pub name: String,
    /// The real serial assets filling this formation.
    pub assets: Vec<AssetId>,
    pub state: FormationState,
    pub formed_day: u64,
}

impl Formation {
    /// The site this formation is currently at, if it is at one (`None`
    /// while en route — the same "on the road" fog convoys observe).
    pub fn current_site(&self) -> Option<LocationId> {
        match self.state {
            FormationState::Stationed { at } => Some(at),
            FormationState::EnRoute { .. } => None,
        }
    }
}

impl World {
    pub fn define_toe_template(
        &mut self,
        name: &str,
        doctrine: &str,
        slots: Vec<ToeSlot>,
    ) -> ToeTemplateId {
        let id = ToeTemplateId(self.alloc());
        self.toe_templates.insert(
            id,
            ToeTemplate {
                id,
                name: name.to_string(),
                doctrine: doctrine.to_string(),
                slots,
            },
        );
        self.push_event(EventKind::ToeTemplateDefined { template: id });
        id
    }

    /// Assemble a formation from real, physically-present assets the owner
    /// holds at `site` (in the open or in stockpiles there). Fails with a
    /// typed shortage list if the template cannot be fully filled — nothing
    /// is spawned, nothing is partially locked.
    pub fn try_assemble_formation(
        &mut self,
        owner: ActorId,
        template_id: ToeTemplateId,
        name: &str,
        site: LocationId,
    ) -> Result<FormationId, SimError> {
        if !self.actors.contains_key(&owner) {
            return Err(SimError::UnknownActor(owner));
        }
        if !self.locations.contains_key(&site) {
            return Err(SimError::UnknownLocation(site));
        }
        let template = self
            .toe_templates
            .get(&template_id)
            .ok_or(SimError::UnknownToeTemplate(template_id))?
            .clone();

        let mut chosen: Vec<AssetId> = Vec::new();
        let mut shortages: Vec<ToeShortage> = Vec::new();
        for slot in &template.slots {
            let mut found: u32 = 0;
            for (aid, asset) in &self.assets {
                if found == slot.count {
                    break;
                }
                if asset.owner != owner || asset.design != slot.design {
                    continue;
                }
                if chosen.contains(aid) {
                    continue;
                }
                // Only free assets: not already in a formation, not aboard a
                // convoy.
                if matches!(asset.location, LocationRef::Formation(_) | LocationRef::Convoy(_)) {
                    continue;
                }
                if self.resolve_site(asset.location) != Some(site) {
                    continue;
                }
                chosen.push(*aid);
                found += 1;
            }
            if found < slot.count {
                shortages.push(ToeShortage {
                    role: slot.role.clone(),
                    design: slot.design,
                    missing: slot.count - found,
                });
            }
        }
        if !shortages.is_empty() {
            return Err(SimError::ToeShortage(shortages));
        }

        let id = FormationId(self.alloc());
        let formed_day = self.clock.day;
        self.formations.insert(
            id,
            Formation {
                id,
                owner,
                template: template_id,
                name: name.to_string(),
                assets: chosen.clone(),
                state: FormationState::Stationed { at: site },
                formed_day,
            },
        );
        for asset in &chosen {
            self.move_asset_raw(*asset, LocationRef::Formation(id))?;
        }
        self.push_event(EventKind::FormationAssembled {
            formation: id,
            template: template_id,
            assets: chosen,
        });
        Ok(id)
    }

    /// Strike a destroyed or captured asset from whatever formation roster
    /// it was on. Combat can leave a formation's ownership/membership
    /// invariant broken otherwise: a captured unit now belongs to the
    /// winner, not the formation's original owner.
    pub(crate) fn remove_asset_from_formations(&mut self, asset: AssetId) {
        for formation in self.formations.values_mut() {
            formation.assets.retain(|a| *a != asset);
        }
    }

    /// Order a stationed formation to march along a real route. The
    /// formation must be at the route's origin; it becomes untargetable by
    /// combat and unable to receive march/assemble orders until it arrives
    /// (mirrors [`World::depart_convoy`]).
    pub fn order_formation_march(&mut self, formation: FormationId, route: RouteId) -> Result<(), SimError> {
        let at = self
            .formations
            .get(&formation)
            .ok_or(SimError::UnknownFormation(formation))?
            .current_site()
            .ok_or(SimError::FormationNotAtSite(formation))?;
        let r = self
            .routes
            .get(&route)
            .ok_or(SimError::UnknownRoute(route))?
            .clone();
        if r.from != at {
            return Err(SimError::NotColocated);
        }
        let departed_day = self.clock.day;
        let arrives_day = departed_day + r.distance_days;
        self.formations.get_mut(&formation).unwrap().state = FormationState::EnRoute {
            route,
            departed_day,
            arrives_day,
        };
        self.push_event(EventKind::FormationMarchOrdered { formation, route, arrives_day });
        Ok(())
    }

    /// Advance formations: arrivals fire when the clock reaches the arrival
    /// day (mirrors [`World::tick_convoys`]).
    pub(crate) fn tick_formations(&mut self) {
        let today = self.clock.day;
        let arriving: Vec<(FormationId, LocationId)> = self
            .formations
            .iter()
            .filter_map(|(id, f)| match f.state {
                FormationState::EnRoute { route, arrives_day, .. } if arrives_day <= today => {
                    self.routes.get(&route).map(|r| (*id, r.to))
                }
                _ => None,
            })
            .collect();
        for (formation, at) in arriving {
            self.formations.get_mut(&formation).unwrap().state = FormationState::Stationed { at };
            self.push_event(EventKind::FormationArrived { formation, at });
        }
    }

    /// What a faction is short for a template at a site, without assembling.
    /// This is the raw demand query for faction procurement AI.
    pub fn toe_shortages(
        &self,
        owner: ActorId,
        template_id: ToeTemplateId,
        site: LocationId,
    ) -> Result<Vec<ToeShortage>, SimError> {
        let template = self
            .toe_templates
            .get(&template_id)
            .ok_or(SimError::UnknownToeTemplate(template_id))?;
        let mut counted: Vec<AssetId> = Vec::new();
        let mut shortages = Vec::new();
        for slot in &template.slots {
            let mut found: u32 = 0;
            for (aid, asset) in &self.assets {
                if found == slot.count {
                    break;
                }
                if asset.owner != owner || asset.design != slot.design {
                    continue;
                }
                if counted.contains(aid) {
                    continue;
                }
                if matches!(asset.location, LocationRef::Formation(_) | LocationRef::Convoy(_)) {
                    continue;
                }
                if self.resolve_site(asset.location) != Some(site) {
                    continue;
                }
                counted.push(*aid);
                found += 1;
            }
            if found < slot.count {
                shortages.push(ToeShortage {
                    role: slot.role.clone(),
                    design: slot.design,
                    missing: slot.count - found,
                });
            }
        }
        Ok(shortages)
    }
}
