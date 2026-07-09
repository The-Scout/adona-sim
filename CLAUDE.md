# Working rules for this repository

These rules apply to every session, not just the one that wrote them. They
override the general instinct to take the fastest path to today's ticket
when that path would hardcode something game-specific into generic engine
code.

## Build everything modular, generic, and expandable

Every system added to this repo should be designed to grow into both
ADONA's full game and other people's games — not just to satisfy the
request in front of you. Before writing a new mechanism, ask: **could a
different game reuse this with different data?** If yes, it belongs in
`crates/adona-sim` as a parameterized mechanism (config struct in, real
handles out), never as ADONA-specific names/numbers baked into logic.

This is not a new aspiration — `docs/design/ECONOMIC_SIMULATION_DOCKET.md`
already says the economy crate should be "clean enough to socket into other
projects." That principle is now a hard rule enforced for *all* new work,
not just the systems that happened to be built with it in mind.

### Where things belong

- **`crates/adona-sim`** — generic, reusable simulation machinery. No
  ADONA-specific content: no hardcoded faction names, material names, tier
  counts, or magic numbers tied to Adona's lore. Every mechanism here should
  be usable by a project that has never heard of ADONA.
- **`crates/adona-game`** — thin ADONA-specific glue and content. Game
  flavor (faction names, doctrine, material names, tier names, map layout)
  lives in data files (e.g. `assets/world_seed.json`), loaded through a
  schema (`src/seed.rs`), not hardcoded in Rust logic. If you're tempted to
  write a Rust `for` loop over "the 5 materials" or "the 7 factions," that
  count and those names should come from data, not a literal in the loop.

### How to build it

- Prefer **config-struct-in, handles-out generator functions** over one-off
  content scripts. A function that builds a repeated pattern (a tiered
  production chain, a formation template, a market) should take a spec
  struct and return the real typed IDs it created, so a caller can wire
  them up without the generator knowing anything about who's calling it or
  why.
- Prefer **generic tick phases and mechanisms** over feature-specific
  special-casing. If a new behavior only fires for one particular faction,
  material, or mission, it's very likely the wrong shape — find the general
  rule it's an instance of instead.
- When a new feature needs the same *shape* of thing an existing feature
  already built (e.g. equipment/weapon components will need the same
  raw-material -> tiered-component pipeline mechs use), reuse and extend the
  existing generic mechanism rather than duplicating it with different
  hardcoded numbers.

### Worked example

`crates/adona-sim/src/content.rs`'s `generate_tiered_material_chain` (added
alongside this rule) is the pattern to match: it takes a `TieredChainSpec`
(material name, tier names, quantities, durations — all caller-supplied,
none defaulted to anything ADONA-specific) and returns a
`TieredChainHandles` of real `CommodityId`/`ComponentDefId`/`RecipeId`
values. `crates/adona-game/src/seed.rs` calls it once per material listed
in `assets/world_seed.json`, supplying Adona's actual 5 materials and its
13-tier naming from `docs/design/Old/Tier System.md`. A different game
would ship a different JSON and a different tier count through the exact
same function.

## Hardening: validate values, avoid unnecessary writes

This runs on a real machine against a real SSD, and `crates/adona-sim`'s
event log and lot/component history are append-only by design (provenance is
never deleted — see `world.rs`'s module docs). Both facts mean the same
discipline applies to every new tick phase, storage adapter, or content
generator:

- **All arithmetic on accumulated quantities (money, stock, percentages, day
  counts, tiers) must be validated against overflow/underflow.** Use
  `checked_*`/`saturating_*`, or an explicit bounds check immediately before
  the operation in the same scope (not "checked earlier in a different
  function and trusted to still hold"). Raw `-`/`+`/`*` on `u64`/`i64`
  derived from world state is a bug waiting for the one input path that
  doesn't go through the guard. `crates/adona-sim/src/combat.rs` and
  `crates/adona-sim/src/actors.rs` are the reference examples to match.
- **Never write a zero-effect record.** Don't create a zero-quantity lot,
  push an event for something that didn't actually change, or start a
  production job whose output is empty — every write should correspond to a
  real state change, both because fake writes violate the "everything
  important is real" axiom and because this is genuinely disk I/O once
  persistence (`storage.rs`) is in the loop.
- **Don't rescan a world-size collection inside a nested loop.** If a tick
  phase loops over formations/factories/cities/goals and, for each one,
  iterates all of `self.lots`/`self.assets`/`self.components`/
  `self.buy_orders`/`self.sell_listings` again, that's an O(n²)-shaped bug
  that gets worse every simulated day as the world grows. Build the index
  you need once per tick (a `BTreeMap` keyed by whatever you're filtering
  on) before the outer loop, the same way `production.rs`'s
  `tick_factory_auto_production`, `faction_ai.rs`'s `tick_faction_ai`, and
  `markets.rs`'s `tick_market_matching`/`tick_civilian_demand` do. When in
  doubt: name what grows without bound as the game runs (days, factions,
  lots, formations) and make sure no phase's cost multiplies two of those
  together.

## Process notes

- **Verify by actually driving the simulation, not just checking it
  doesn't crash.** A GUI that launches without panicking is not evidence
  the simulation is doing anything meaningful. Step days forward and watch
  the actual numbers (population, treasury, production events) before
  declaring an economy/simulation feature done.
- Large or architecturally significant changes should go through plan mode
  before code is written — see the plan file convention already in use in
  this project's sessions.
