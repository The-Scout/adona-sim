# Known Issues / Follow-ups

Tracked here in lieu of a GitHub remote. Move these to GitHub Issues once a
remote exists.

## Open

### Factions do not move, and simulation behavior beyond ~day 100 is unverified

- **Factions do not move.** `war.rs`'s automatic combat phase only fights
  formations that already physically share a site — there is no
  deployment/movement AI that marches a formation toward contested or enemy
  territory. Faction "war" today is limited to whatever co-location happens
  to already exist (e.g. from manual/admin setup). This is the same gap
  called out as `TODO(war)` in `crates/adona-sim/src/war.rs` and
  `crates/adona-sim/src/toe.rs`.
- **Long-run tick behavior (~100+ days) is unverified.** No test currently
  runs the simulation for an extended number of ticks (existing tests run at
  most tens of days). It's unconfirmed whether the tick phases (production,
  convoys, civilian demand, faction AI, faction war, market matching,
  population) remain well-behaved, performant, or bug-free over long runs —
  e.g. unbounded growth in event log size, formations/factories accumulating
  in ways that degrade the O(sites × formations) war-phase scan, population
  or unrest drifting to a degenerate state, etc. Needs a dedicated long-run
  soak test before this is trusted for real campaign lengths.

## Other known gaps (not bugs, staged intentionally)

- Convoy interception and stockpile raiding are not implemented as combat
  shapes; `AttackTarget` contracts against those target kinds still complete
  on caller assertion rather than verified outcome (`contracts.rs`).
- Market price discovery is a simple EMA over trade history; no explicit
  anti-manipulation or liquidity modeling.
- Intel rumor spread exists (`relay_intel`) but nothing yet automates rumor
  propagation on a schedule — it's caller-driven today.
- The Bevy GUI (`adona-game`) is an observer view only; no click-to-command
  interaction layer yet.
- The full cockpit/tactical mech-combat game described in
  `GAME_DESIGN_DOCUMENT.md` has no code yet — `combat.rs`'s RISK-style
  resolution is an explicit collapsed stand-in for it.
