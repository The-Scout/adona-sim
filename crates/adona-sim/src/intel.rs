//! Intel: immutable observations.
//!
//! "Convoy seen at X on day 12" stays true forever; it goes stale because
//! the world moves, not because the record decays. There are no mutators for
//! recorded intel. [`World::relay_intel`] models rumor spread and broker
//! networks: relaying never edits the source record, it produces a new,
//! separately-stored observation with decayed confidence, increased
//! corruption, and the relayer appended to the chain of custody — a broker
//! only ever knows what reached them, not the original truth.
//! [`World::plant_misinformation`] gives false-intel systems (corrupt
//! reports, black-market leakage) a typed, auditable home distinct from
//! stale-but-true observation.

use crate::events::EventKind;
use crate::ids::*;
use crate::world::World;
use crate::SimError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntelSubject {
    Convoy(ConvoyId),
    Asset(AssetId),
    Formation(FormationId),
    StockpileSite(LocationId),
    Battle { site: LocationId },
    Market(MarketId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntelObservation {
    pub id: IntelId,
    /// Day the observation was made (world time, not report arrival).
    pub observed_day: u64,
    /// Event-log sequence number at the moment of observation. Staleness
    /// checks that need finer granularity than a day (multiple things can
    /// happen on the same strategic day) compare against this instead.
    pub observed_seq: u64,
    /// Observer, if known. `None` covers anonymous rumor and leakage.
    pub observer: Option<ActorId>,
    pub subject: IntelSubject,
    /// Where the subject was observed.
    pub observed_at: LocationId,
    /// Freeform detail ("twelve trucks, light escort, heading east").
    pub detail: String,
    /// 0–100. How sure the source was.
    pub confidence_pct: u8,
    /// 0–100. How mangled the report got in transit; feeds future false
    /// intel systems.
    pub corruption_pct: u8,
    /// Chain of custody, oldest first, if useful.
    pub chain: Vec<ActorId>,
    /// The record this one was relayed from, if it is a rumor rather than a
    /// direct observation. Rumor chains are walkable back to the original
    /// sighting; nothing in the chain is ever mutated.
    pub derived_from: Option<IntelId>,
    /// Deliberately false intel (corrupt report, black-market leak, planted
    /// disinformation) rather than a genuine sighting or an honest relay of
    /// one. The subject still had to be real when planted — misinformation
    /// lies about real things, it does not invent targets from nothing.
    pub fabricated: bool,
}

impl World {
    /// Record an observation. The subject must actually exist: intel is
    /// created by observation of real things (or explicit admin action),
    /// never conjured. The record is immutable once stored.
    pub fn record_observation(
        &mut self,
        observer: Option<ActorId>,
        subject: IntelSubject,
        observed_at: LocationId,
        detail: &str,
        confidence_pct: u8,
        corruption_pct: u8,
    ) -> Result<IntelId, SimError> {
        if !self.locations.contains_key(&observed_at) {
            return Err(SimError::UnknownLocation(observed_at));
        }
        if let Some(o) = observer {
            if !self.actors.contains_key(&o) {
                return Err(SimError::UnknownActor(o));
            }
        }
        match subject {
            IntelSubject::Convoy(c) => {
                if !self.convoys.contains_key(&c) {
                    return Err(SimError::UnknownConvoy(c));
                }
            }
            IntelSubject::Asset(a) => {
                if !self.assets.contains_key(&a) {
                    return Err(SimError::UnknownAsset(a));
                }
            }
            IntelSubject::Formation(f) => {
                if !self.formations.contains_key(&f) {
                    return Err(SimError::UnknownFormation(f));
                }
            }
            IntelSubject::StockpileSite(s) | IntelSubject::Battle { site: s } => {
                if !self.locations.contains_key(&s) {
                    return Err(SimError::UnknownLocation(s));
                }
            }
            IntelSubject::Market(m) => {
                if !self.markets.contains_key(&m) {
                    return Err(SimError::UnknownMarket(m));
                }
            }
        }
        let id = IntelId(self.alloc());
        let observed_day = self.clock.day;
        let observed_seq = self.next_event_seq;
        self.intel.insert(
            id,
            IntelObservation {
                id,
                observed_day,
                observed_seq,
                observer,
                subject,
                observed_at,
                detail: detail.to_string(),
                confidence_pct: confidence_pct.min(100),
                corruption_pct: corruption_pct.min(100),
                chain: observer.into_iter().collect(),
                derived_from: None,
                fabricated: false,
            },
        );
        self.push_event(EventKind::IntelRecorded { intel: id });
        Ok(id)
    }

    /// Relay an existing record through a broker or the rumor mill. This is
    /// how "a scout saw it" becomes "word reached three cities over": the
    /// source record is untouched (intel is never mutated), but the relay is
    /// a *new*, separately-stored observation with confidence decayed by
    /// `confidence_decay_pct`, corruption increased, and `via` appended to
    /// the chain of custody. A broker only ever knows what reached them —
    /// this is why the relay's `observer` is `None` rather than the original
    /// witness.
    pub fn relay_intel(
        &mut self,
        source: IntelId,
        via: ActorId,
        confidence_decay_pct: u8,
    ) -> Result<IntelId, SimError> {
        if !self.actors.contains_key(&via) {
            return Err(SimError::UnknownActor(via));
        }
        let src = self.intel.get(&source).ok_or(SimError::UnknownIntel(source))?.clone();
        let decay = confidence_decay_pct.min(100) as u32;
        let new_confidence = (src.confidence_pct as u32 * (100 - decay) / 100) as u8;
        let new_corruption = src.corruption_pct.saturating_add((decay / 2) as u8).min(100);
        let mut chain = src.chain.clone();
        chain.push(via);
        let id = IntelId(self.alloc());
        self.intel.insert(
            id,
            IntelObservation {
                id,
                observed_day: src.observed_day,
                observed_seq: src.observed_seq,
                observer: None,
                subject: src.subject,
                observed_at: src.observed_at,
                detail: src.detail.clone(),
                confidence_pct: new_confidence,
                corruption_pct: new_corruption,
                chain,
                derived_from: Some(source),
                fabricated: src.fabricated,
            },
        );
        self.push_event(EventKind::IntelRelayed { source, relayed: id, via });
        Ok(id)
    }

    /// Plant deliberate misinformation about a real subject (docket: corrupt
    /// or compromised reports, black-market leakage). The subject must still
    /// physically exist — the axiom "everything important is real" applies
    /// to what misinformation is *about*, even though the detail attached to
    /// it is false. The resulting record is flagged `fabricated` so
    /// downstream systems (faction AI, player-facing intel screens) can
    /// weigh it differently from a genuine sighting or honest rumor.
    pub fn plant_misinformation(
        &mut self,
        planter: Option<ActorId>,
        subject: IntelSubject,
        observed_at: LocationId,
        false_detail: &str,
        claimed_confidence_pct: u8,
    ) -> Result<IntelId, SimError> {
        let id =
            self.record_observation(planter, subject, observed_at, false_detail, claimed_confidence_pct, 100)?;
        self.intel.get_mut(&id).unwrap().fabricated = true;
        self.push_event(EventKind::MisinformationPlanted { intel: id, planter });
        Ok(id)
    }

    /// Is this observation stale — true when made, but the world has moved?
    /// Returns `Some(true/false)` when staleness is decidable for this
    /// subject kind given current state, `None` otherwise. Staleness is
    /// computed against the live world; the observation itself is never
    /// touched.
    pub fn intel_is_stale(&self, intel: IntelId) -> Result<Option<bool>, SimError> {
        let obs = self.intel.get(&intel).ok_or(SimError::UnknownIntel(intel))?;
        match obs.subject {
            IntelSubject::Convoy(c) => {
                let convoy = self.convoys.get(&c).ok_or(SimError::UnknownConvoy(c))?;
                Ok(Some(convoy.current_site() != Some(obs.observed_at)))
            }
            IntelSubject::Asset(a) => {
                let asset = self.assets.get(&a).ok_or(SimError::UnknownAsset(a))?;
                Ok(Some(self.resolve_site(asset.location) != Some(obs.observed_at)))
            }
            IntelSubject::Formation(f) => {
                let formation = self.formations.get(&f).ok_or(SimError::UnknownFormation(f))?;
                Ok(Some(formation.current_site() != Some(obs.observed_at)))
            }
            IntelSubject::StockpileSite(site) => {
                // Stale once any real cargo has moved into or out of a
                // stockpile at this site since the observation. Compared by
                // event sequence, not day, since several things can happen
                // on the same strategic day.
                let moved_since = self.events.iter().any(|e| {
                    e.seq > obs.observed_seq
                        && match &e.kind {
                            EventKind::LotMoved { from, to, .. } => {
                                self.resolve_site(*from) == Some(site)
                                    || self.resolve_site(*to) == Some(site)
                            }
                            _ => false,
                        }
                });
                Ok(Some(moved_since))
            }
            // TODO(intel): Market staleness needs a real price/inventory
            // snapshot to compare against; Battle staleness needs the
            // combat-resolution system (docket: RISK-style faction combat).
            IntelSubject::Market(_) | IntelSubject::Battle { .. } => Ok(None),
        }
    }
}
