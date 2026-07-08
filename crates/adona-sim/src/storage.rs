//! Storage boundary.
//!
//! The simulation core is pure and engine-agnostic; persistence is an
//! adapter behind [`SimStore`]. The model is snapshot + append-only event
//! log, which maps directly onto PostgreSQL (a `world_snapshots` table and
//! an `events` table keyed by `seq`) as well as onto files or memory.
//!
//! PostgreSQL is the intended first-party backend, living in a companion
//! crate (working name `adona-sim-postgres`) so the core crate stays free of
//! database dependencies. TODO(postgres): companion crate with schema
//! migrations implementing `SimStore`; later, normalized relational
//! projections of hot tables (lots, assets, listings) for tooling and
//! modder inspection rather than opaque snapshot blobs.

use crate::events::Event;
use crate::world::World;

#[derive(Debug)]
pub enum StorageError {
    /// Backend-specific failure, stringly wrapped at the boundary.
    Backend(String),
    /// Snapshot or event payload failed to (de)serialize.
    Codec(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Backend(msg) => write!(f, "storage backend error: {msg}"),
            StorageError::Codec(msg) => write!(f, "storage codec error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Persistence adapter for the strategic world.
pub trait SimStore {
    /// Persist a full snapshot of the world.
    fn save_snapshot(&mut self, world: &World) -> Result<(), StorageError>;

    /// Load the most recent snapshot, if any.
    fn load_latest_snapshot(&mut self) -> Result<Option<World>, StorageError>;

    /// Append events. Events are immutable and ordered by `seq`; adapters
    /// may assume monotonically increasing `seq` values.
    fn append_events(&mut self, events: &[Event]) -> Result<(), StorageError>;

    /// Load all events with `seq >= from_seq`, in order.
    fn load_events_from(&mut self, from_seq: u64) -> Result<Vec<Event>, StorageError>;
}

/// In-memory store: the first compilable adapter, and the reference for
/// adapter semantics in tests.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    snapshot: Option<String>,
    events: Vec<Event>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SimStore for InMemoryStore {
    fn save_snapshot(&mut self, world: &World) -> Result<(), StorageError> {
        // Serialize rather than clone so the store round-trip exercises the
        // same codec path a database adapter will use.
        let json = serde_json::to_string(world).map_err(|e| StorageError::Codec(e.to_string()))?;
        self.snapshot = Some(json);
        Ok(())
    }

    fn load_latest_snapshot(&mut self) -> Result<Option<World>, StorageError> {
        match &self.snapshot {
            None => Ok(None),
            Some(json) => serde_json::from_str(json)
                .map(Some)
                .map_err(|e| StorageError::Codec(e.to_string())),
        }
    }

    fn append_events(&mut self, events: &[Event]) -> Result<(), StorageError> {
        self.events.extend_from_slice(events);
        Ok(())
    }

    fn load_events_from(&mut self, from_seq: u64) -> Result<Vec<Event>, StorageError> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.seq >= from_seq)
            .cloned()
            .collect())
    }
}
