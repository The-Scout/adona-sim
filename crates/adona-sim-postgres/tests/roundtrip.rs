//! Live-database round-trip test. Requires a reachable Postgres instance;
//! set `ADONA_TEST_PG_URL` (a standard libpq connection string) to run it.
//! Skips (rather than fails) when the variable is unset, so `cargo test
//! --workspace` doesn't require a database to pass.

use adona_sim::actors::ActorKind;
use adona_sim::storage::SimStore;
use adona_sim::World;
use adona_sim_postgres::PostgresStore;

#[test]
fn snapshot_and_events_round_trip_through_real_postgres() {
    let Ok(url) = std::env::var("ADONA_TEST_PG_URL") else {
        eprintln!("skipping: set ADONA_TEST_PG_URL to run the Postgres round-trip test");
        return;
    };

    let mut store = PostgresStore::connect(&url).expect("connect to test Postgres instance");

    let mut world = World::new(123);
    world.create_actor("Test Faction", ActorKind::Faction, 10_000);
    world.tick();
    world.tick();

    store.save_snapshot(&world).unwrap();
    store.append_events(world.events()).unwrap();

    let loaded = store.load_latest_snapshot().unwrap().expect("snapshot was saved");
    assert_eq!(loaded.state_digest(), world.state_digest());

    let events = store.load_events_from(0).unwrap();
    assert_eq!(events.len(), world.events().len());
    for (a, b) in events.iter().zip(world.events().iter()) {
        assert_eq!(a, b, "event round-trip mismatch");
    }

    // The loaded world keeps simulating identically, proving the round-trip
    // preserved RNG state and every other piece of strategic state.
    let mut w1 = world;
    let mut w2 = loaded;
    w1.tick();
    w2.tick();
    assert_eq!(w1.state_digest(), w2.state_digest());
}
