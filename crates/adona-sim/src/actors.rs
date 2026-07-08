//! Actors: decision-making entities.
//!
//! Actors own assets, hold money, place market orders, issue contracts, run
//! convoys, request production, and pursue TO&E goals. Money is conserved:
//! it only enters the world through explicit issuance (currently actor
//! creation seed treasuries; mints/budgets/taxation are open design work in
//! the docket) and moves between treasuries and escrows after that.

use crate::events::EventKind;
use crate::ids::ActorId;
use crate::world::World;
use serde::{Deserialize, Serialize};

/// Money. Integer credits — no floats anywhere in strategic state, for
/// determinism and clean hashing.
pub type Credits = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorKind {
    Faction,
    CityAuthority,
    MercenaryCompany,
    PiratePolity,
    IndependentTrader,
    FactoryOwner,
    AdminOperator,
    PlayerOperator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: ActorId,
    pub name: String,
    pub kind: ActorKind,
    pub treasury: Credits,
}

impl World {
    /// Create an actor with a seed treasury. The seed treasury is explicit
    /// money issuance and is counted toward the money-conservation invariant.
    pub fn create_actor(&mut self, name: &str, kind: ActorKind, treasury: Credits) -> ActorId {
        let id = ActorId(self.alloc());
        self.actors.insert(
            id,
            Actor {
                id,
                name: name.to_string(),
                kind,
                treasury,
            },
        );
        self.money_issued = self.money_issued.saturating_add(treasury as i128);
        self.push_event(EventKind::ActorCreated {
            actor: id,
            issued: treasury,
        });
        id
    }

    /// Move credits out of a treasury. Fails rather than going negative.
    pub(crate) fn debit(&mut self, actor: ActorId, amount: Credits) -> Result<(), crate::SimError> {
        let a = self
            .actors
            .get_mut(&actor)
            .ok_or(crate::SimError::UnknownActor(actor))?;
        if a.treasury < amount {
            return Err(crate::SimError::InsufficientFunds {
                actor,
                needed: amount,
                available: a.treasury,
            });
        }
        a.treasury -= amount;
        Ok(())
    }

    /// Move credits into a treasury.
    pub(crate) fn credit(&mut self, actor: ActorId, amount: Credits) -> Result<(), crate::SimError> {
        let a = self
            .actors
            .get_mut(&actor)
            .ok_or(crate::SimError::UnknownActor(actor))?;
        a.treasury = a.treasury.checked_add(amount).ok_or(crate::SimError::Overflow)?;
        Ok(())
    }

    /// Explicit money issuance outside of actor creation — currently used
    /// for per-capita taxation revenue (docket: candidate money source
    /// "Taxation, if population is tracked"). Like a seed treasury, this
    /// grows `money_issued` because real credits are entering circulation
    /// from outside any actor's treasury, not moving between actors.
    pub(crate) fn issue_money(&mut self, actor: ActorId, amount: Credits) -> Result<(), crate::SimError> {
        self.credit(actor, amount)?;
        self.money_issued = self.money_issued.saturating_add(amount as i128);
        Ok(())
    }

    /// Debit a treasury for a real cost with no receiving actor in the
    /// simulation (retooling labor/materials, taxation to an untracked
    /// state budget, etc.). The credits leave circulation entirely, so
    /// `money_issued` shrinks with them — otherwise the money-conservation
    /// invariant would treat this as money vanishing into nowhere.
    pub(crate) fn burn(&mut self, actor: ActorId, amount: Credits) -> Result<(), crate::SimError> {
        self.debit(actor, amount)?;
        self.money_issued = self.money_issued.saturating_sub(amount as i128);
        Ok(())
    }
}
