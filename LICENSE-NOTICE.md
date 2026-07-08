# Licensing notice

This project is currently dual-licensed under [MIT](LICENSE-MIT) or
[Apache License 2.0](LICENSE-APACHE), at your option — the Rust ecosystem
standard, chosen so anyone can build engines, tools, or games on top of the
simulation crates freely, including commercially.

**This may change in the future.** As the project grows, the license may
change toward a copyleft model (e.g. something in the MPL/LGPL/AGPL family)
for parts of the project, to keep improvements to the shared simulation
flowing back to the community rather than being closed off downstream.

Two things are intended to stay true even if that happens:

- **Any code already released under MIT/Apache-2.0 stays available under
  those terms.** A future license change would apply to new releases going
  forward, not retroactively revoke permissions already granted.
- **The engine-agnostic simulation crates (`adona-sim` and its storage
  adapters) are intended to keep open permissions** — the goal is for
  anyone to be able to build a game engine integration on top of this work
  without friction, even if other parts of the broader ADONA project (game
  content, art, the eventual full game) end up under different terms.
- **Commercial users are welcome, especially game developers.** If a future release changes license terms in a way that creates friction for a commercial or downstream project, the maintainers are open to discussing alternative licensing options where appropriate.
If you're building on this crate, check the license headers/files in the
version you depend on — this notice describes intent, not a guarantee that
overrides whatever license actually ships with a given release.
