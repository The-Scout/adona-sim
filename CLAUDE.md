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

## Everything should be data-driven

This generalizes the modularity rule above from "content" (faction names,
materials, tiers) to *any* tunable number or behavior parameter. If you
catch yourself writing a bare numeric literal into engine or game logic —
a quantity, a duration, a percentage, a threshold, a chance-per-day — stop
and ask whether it's a genuine invariant of the simulation or an actual
tunable that should vary by scenario.

- **Genuine invariants** (rules the simulation's design depends on, like
  `MECH_COMPONENT_SLOTS == 5`) can stay as named `const`s in `adona-sim`,
  documented as deliberate product rules.
- **Everything else is data.** Economy tuning (mine yields, refine/convert
  quantities, durations), world-seed numbers (population, tax rates,
  starting treasuries), doctrine parameters — these belong in the data file
  that already exists for the purpose (`world_seed.json`'s `economy` block
  is the template to extend, not to work around) or a new one, never a
  literal buried in a function body.
- Rule of thumb: **if changing the number requires a recompile, ask whether
  it should have only required editing a JSON file instead.**

## Follow Rust best practices

Match the patterns already established in this codebase — don't regress
them when adding new code:

- Prefer `Result<T, SimError>` over panicking for anything that can fail
  from real-world input (unknown ids, insufficient stock, invalid state
  transitions). Reserve `.unwrap()`/`.expect()` for cases that are true
  programmer errors or already validated earlier in the same call — and say
  so in a comment when it isn't obvious why the unwrap is safe.
- Use the newtype id pattern (`ActorId`, `LocationId`, etc., via the
  `define_id!` macro in `ids.rs`) for anything that could otherwise be
  confused with a bare `u64`. Never pass raw integers between domains.
- Keep state in `BTreeMap`/`BTreeSet`, never `HashMap`/`HashSet`, for
  anything that feeds `World::tick()` or `state_digest()` — iteration order
  must stay deterministic (see `world.rs`'s determinism contract). This is
  not a style preference; using a `HashMap` here is a correctness bug.
- Run `cargo clippy --workspace --all-targets` and `cargo fmt --check`
  before considering a change done. Fix warnings rather than silence them;
  an `#[allow(...)]` needs a comment explaining why the lint doesn't apply.
- Write doc comments (`///`) that explain *why*, not *what* — the existing
  modules (`production.rs`, `combat.rs`, `war.rs`) are full of good
  examples. A comment that just restates the function's name in prose isn't
  worth writing.
- Avoid needless `.clone()` in tick-path code. When a clone is genuinely
  required (e.g. to end a borrow before a mutating call), a short comment
  saying why saves the next person from "optimizing" it back into a
  borrow-checker error.
- Match the module-per-domain layout already established (`combat.rs`,
  `markets.rs`, `production.rs`, `locations.rs`, etc.) — a new system gets
  its own module rather than growing inside an unrelated one.

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
