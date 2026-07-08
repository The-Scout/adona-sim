# Contributing

Thanks for taking a look. This project is early — see the note at the
bottom of the [README](README.md) for honest context on where it's at and
how it's being built.

## Ground rules

- **Everything important is real.** This is the core design axiom of
  `adona-sim`: no fake inventories, no spawned units, no abstract "market
  level" standing in for actual goods. If you add a system, every item,
  unit, or credit it touches needs an owner, a location, and (where
  applicable) provenance. `World::check_invariants()` exists to catch
  violations of this — new features should extend it, not work around it.
- **Determinism matters.** Same seed + same ordered inputs must produce the
  same [`World::state_digest`]. No floats in strategic state, no
  non-deterministic iteration (hence `BTreeMap` everywhere instead of
  `HashMap`), no system randomness outside the seeded RNG.
- **The simulation core stays engine-agnostic.** `adona-sim` must not gain
  a Bevy (or any engine) dependency. Engine/game-layer code belongs in a
  separate crate (like `adona-game`) that consumes `adona-sim`, not the
  other way around.

## Before opening a PR

1. `cargo test --workspace` — all tests should pass. If you add behavior,
   add a test that would fail without it (this codebase leans heavily on
   integration tests in `crates/adona-sim/tests/invariants.rs` that build a
   real scenario and assert on real outcomes, not mocks).
2. `cargo run --example demo -p adona-sim` and skim the output — a good
   sanity check that nothing subtly broke the day-by-day narration.
3. If you touched anything storage-related, note whether you tested it
   against a real Postgres instance (`ADONA_TEST_PG_URL`) or only compiled it.

## Known gaps / good first areas

See [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) — in particular, faction formation
movement/deployment AI and long-run (100+ day) tick verification are open
and would be genuinely useful contributions.

## License

By contributing, you agree your contribution is licensed under the same
terms as the project (see [`LICENSE-NOTICE.md`](LICENSE-NOTICE.md)).
