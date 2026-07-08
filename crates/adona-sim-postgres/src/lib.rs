//! PostgreSQL storage adapter for `adona-sim`.
//!
//! Implements [`SimStore`] against the model the docket asks for: a
//! `world_snapshots` table and an `events` table keyed by `seq`. The crate
//! stays a thin adapter — the simulation core in `adona-sim` has no
//! knowledge of Postgres, and this crate has no knowledge of simulation
//! rules, only of how to persist and load [`World`] and [`Event`].
//!
//! Schema is created on connect (`CREATE TABLE IF NOT EXISTS`), so a fresh
//! database just works. World state and event payloads are stored as
//! `jsonb` rather than normalized columns for now — normalized projections
//! of hot tables (lots, assets, listings) for modder tooling are future
//! work, matching the docket's own "opaque snapshot blobs first, normalized
//! projections later" ordering.

use adona_sim::events::Event;
use adona_sim::storage::{SimStore, StorageError};
use adona_sim::World;
use postgres::{Client, NoTls};

pub struct PostgresStore {
    client: Client,
}

impl PostgresStore {
    /// Connect and ensure the schema exists. `conn_str` is a standard
    /// libpq connection string, e.g.
    /// `"host=localhost user=postgres password=... dbname=adona"`.
    pub fn connect(conn_str: &str) -> Result<Self, StorageError> {
        let mut client =
            Client::connect(conn_str, NoTls).map_err(|e| StorageError::Backend(e.to_string()))?;
        client
            .batch_execute(
                "
                CREATE TABLE IF NOT EXISTS world_snapshots (
                    id BIGSERIAL PRIMARY KEY,
                    seed BIGINT NOT NULL,
                    day BIGINT NOT NULL,
                    state JSONB NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
                );
                CREATE TABLE IF NOT EXISTS events (
                    seq BIGINT PRIMARY KEY,
                    event_id BIGINT NOT NULL,
                    day BIGINT NOT NULL,
                    quarter TEXT NOT NULL,
                    kind JSONB NOT NULL
                );
                ",
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(PostgresStore { client })
    }
}

impl SimStore for PostgresStore {
    fn save_snapshot(&mut self, world: &World) -> Result<(), StorageError> {
        let state = serde_json::to_value(world).map_err(|e| StorageError::Codec(e.to_string()))?;
        self.client
            .execute(
                "INSERT INTO world_snapshots (seed, day, state) VALUES ($1, $2, $3)",
                &[&(world.seed as i64), &(world.today() as i64), &state],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    fn load_latest_snapshot(&mut self) -> Result<Option<World>, StorageError> {
        let row = self
            .client
            .query_opt("SELECT state FROM world_snapshots ORDER BY id DESC LIMIT 1", &[])
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        match row {
            None => Ok(None),
            Some(row) => {
                let state: serde_json::Value = row.get(0);
                serde_json::from_value(state)
                    .map(Some)
                    .map_err(|e| StorageError::Codec(e.to_string()))
            }
        }
    }

    fn append_events(&mut self, events: &[Event]) -> Result<(), StorageError> {
        // A transaction so a batch of events lands atomically — a crash
        // mid-append must never leave a gap in the seq sequence.
        let mut tx = self.client.transaction().map_err(|e| StorageError::Backend(e.to_string()))?;
        for event in events {
            let quarter = format!("{:?}", event.quarter);
            let kind = serde_json::to_value(&event.kind).map_err(|e| StorageError::Codec(e.to_string()))?;
            tx.execute(
                "INSERT INTO events (seq, event_id, day, quarter, kind) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (seq) DO NOTHING",
                &[&(event.seq as i64), &(event.id.0 as i64), &(event.day as i64), &quarter, &kind],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        }
        tx.commit().map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    fn load_events_from(&mut self, from_seq: u64) -> Result<Vec<Event>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT seq, event_id, day, quarter, kind FROM events WHERE seq >= $1 ORDER BY seq",
                &[&(from_seq as i64)],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        rows.into_iter()
            .map(|row| {
                let seq: i64 = row.get(0);
                let event_id: i64 = row.get(1);
                let day: i64 = row.get(2);
                let quarter_text: String = row.get(3);
                let kind_json: serde_json::Value = row.get(4);
                let quarter = parse_quarter(&quarter_text)?;
                let kind = serde_json::from_value(kind_json).map_err(|e| StorageError::Codec(e.to_string()))?;
                Ok(Event {
                    id: adona_sim::ids::EventId(event_id as u64),
                    seq: seq as u64,
                    day: day as u64,
                    quarter,
                    kind,
                })
            })
            .collect()
    }
}

/// `DayQuarter` derives `Debug` (used to write it out as text) but not
/// `FromStr`; this is the one place that needs to read it back.
fn parse_quarter(s: &str) -> Result<adona_sim::time::DayQuarter, StorageError> {
    use adona_sim::time::DayQuarter::*;
    match s {
        "Q1" => Ok(Q1),
        "Q2" => Ok(Q2),
        "Q3" => Ok(Q3),
        "Q4" => Ok(Q4),
        other => Err(StorageError::Codec(format!("unknown quarter {other:?}"))),
    }
}
