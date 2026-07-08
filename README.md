# ADONA: Warsim

A headless, engine-agnostic Rust workspace for simulating a strategic
economy and factional war — the foundation layer for ADONA, a mercenary
mech-combat game currently in preproduction. The war/economy simulation is
being built and shared first because it's the piece that doesn't need a
game engine, an art pipeline, or gameplay code to be useful on its own.

## What's here

- **[`crates/adona-sim`](crates/adona-sim)** — the core simulation crate.
  No Bevy dependency, no engine assumptions. Everything important (mines,
  convoys, factories, markets, mechs, contracts) is a real, owned,
  located, provenance-tracked object — never a fake number pretending to be
  inventory. A hard invariant checker enforces this and every test proves it.
- **[`crates/adona-sim-postgres`](crates/adona-sim-postgres)** — a
  `SimStore` adapter backed by real PostgreSQL (snapshot + append-only
  event-log tables). Verified against a live instance.
- **[`crates/adona-game`](crates/adona-game)** — a small Bevy app that
  renders a strategic observer view over the simulation: a map of sites,
  convoys, and formations, plus panels for treasuries, territory,
  contracts, and the event log. This is a debug/observer tool, not the
  eventual cockpit/tactical combat game.

## Core capabilities

1. **A turn-based economy with provenance tracking per item.** Every lot of
   ore, every mech, every component traces back to where it actually came
   from (a mine, a factory job, a seeded historical stock, an import) —
   split, merge, trade, and salvage all preserve that lineage instead of
   laundering it into an anonymous inventory count.
2. **A factional war simulation** where factions compete to capture each
   other's territory. Combat resolves from real assembled forces (never
   spawned units), defenders get a mechanical home-ground edge, and
   territory control changes hands on the outcome — automatically, every
   tick, whenever hostile forces occupy the same ground. Formations also
   march on their own: a formation stationed on ground its faction controls
   automatically routes toward the nearest contested or enemy-held site and
   fights on arrival — a first-cut greedy rule, not full pathfinding or
   campaign-level coordination (see [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)).
3. **A database-backed living economy** — currently PostgreSQL, with the
   storage boundary (`SimStore`) designed so other backends (SQLite, etc.)
   are a matter of writing an adapter, not restructuring the simulation —
   where supply, demand, production capacity, and consumption drive prices
   and scarcity instead of a fixed price list.

## Getting started

```
cargo test --workspace          # run the full test suite
cargo run --example demo -p adona-sim   # headless day-by-day narration
cargo run -p adona-game          # the Bevy strategic observer GUI
```

Known gaps and open questions are tracked in [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)
rather than left silent.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. See [`LICENSE-NOTICE.md`](LICENSE-NOTICE.md) for intent around
how this may evolve (short version: existing releases keep their
permissions either way, and the engine-agnostic simulation crates are meant
to stay open).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Design docs

The broader game design material (world, factions, narrative, full
gameplay-loop design) lives in `docs/design/` locally and is intentionally
`.gitignore`d — this repo is the public, engine-agnostic simulation crate;
the game built on top of it is a separate, private effort for now.

---

Yes I know I'm using AI to work on this, I'm going to be doing my best to make sure this project isn't slop, but I have LITTERALLY just started learning coding, and rust yesturday. this project has been in preproduction for a few months, its the start of a combined arms mercenary simulator, but the Warsimulation part is what I'm working on first, because its what AI can currently tackle, this is meant to be an engine agnostic rust crate I'm giving to the community, so anyone can make a game that has a backing of:

1. a turn based economy, with provenance tracking per item.
2. A factional war simulation where factions compete to form frontlines, and try to capture eachother's territory.
3. a postgres or Sqlite backend that helps simulate a living economy, with supply and demand, and other factors leading to a realistic production

This part was hand typed.
