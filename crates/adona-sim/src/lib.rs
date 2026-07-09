//! # adona-sim
//!
//! ADONA's strategic economic and faction-war simulation: a headless,
//! engine-agnostic Rust crate. The game consumes this crate; this crate
//! depends on no engine.
//!
//! ## Core axiom: everything important is real
//!
//! No fake faction inventories, no spawned faction mechs, no arbitrary
//! contract enemies, no abstract "market availability". If a faction fields
//! a mech, it came from pre-war inventory, production, purchase, salvage, or
//! capture. If a factory produces armor plate, that output is a
//! provenance-bearing physical lot. If a convoy moves goods, they are real
//! cargo aboard real vehicles. Money is conserved. Intel is immutable
//! observation of real things. Contracts escrow real funds against real
//! targets. [`World::check_invariants`] enforces all of this and the test
//! suite proves it.
//!
//! ## Shape
//!
//! - [`World`] is the aggregate: all state, the deterministic day [`World::tick`],
//!   the append-only event log, and the invariant checker. Domain operations
//!   are `impl World` blocks in their domain modules.
//! - Determinism: same seed + same ordered inputs = same
//!   [`World::state_digest`]. No floats, ordered maps, seeded RNG in state.
//! - Persistence is behind [`storage::SimStore`] (snapshot + event log).
//!   PostgreSQL is the intended first-party backend via a companion crate;
//!   [`storage::InMemoryStore`] is the reference adapter.
//!
//! See `ECONOMIC_SIMULATION_DOCKET.md` at the workspace root for the full
//! design docket this crate implements.

pub mod actors;
pub mod assets;
pub mod combat;
pub mod content;
pub mod contracts;
pub mod convoys;
pub mod error;
pub mod events;
pub mod faction_ai;
pub mod goods;
pub mod ids;
pub mod intel;
pub mod locations;
pub mod markets;
pub mod production;
pub mod rng;
pub mod stockpiles;
pub mod storage;
pub mod time;
pub mod toe;
pub mod war;
pub mod world;

pub use error::SimError;
pub use world::World;
