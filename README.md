# Ferroan

A ground-up Rust rebuild of [OpenSim Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity), the OpenSimulator-based virtual world server.

Ferroan aims to run the same grids, viewers, content and scripts as Tranquillity, with everything in Rust except two native C++ libraries: [Jolt](https://github.com/jrouwe/JoltPhysics) for physics and Linden Lab's [SLua engine](https://github.com/secondlife/slua) for LSL and SLua scripts.

**Status: foundations.** The workspace holds the plan, `ferroan-types` (OpenMetaverse packet bytes for UUIDs, vectors, quaternions and colours), `ferroan-db` (schema version checks on Tranquillity's databases), `ferroan-config` (Tranquillity's INI files) and `ferroan-llsd` (LLSD in XML, binary and notation, checked against libopenmetaverse's encoder). There is no server behaviour yet: nothing here can host a region or talk to a viewer. See [TASKS.md](TASKS.md) for what is next.

## How it gets built

Ferroan replaces Tranquillity one piece at a time. Every Rust piece speaks the same protocols as the C# piece it replaces, so a grid can run both side by side and switch back if something fails. The C# server stays in use, and stays the reference for how things behave, until Ferroan is stable.

1. **Foundations:** protocol, LLSD, types, configuration, databases, HTTP, and a traffic-capture rig.
2. **Grid services** (Robust) running with C# regions.
3. **Region simulator core:** a Rust region next to C# regions.
4. **Physics** (Jolt) and **scripts** (Linden's SLua engine), side by side.
5. **Region modules and content**, and **add-ons**, side by side.
6. **Cut-over:** no C# process left in production.

Details, effort and the gate that ends each phase are in the [plan](docs/PLAN.md). Settled project decisions are in [DECISIONS.md](docs/DECISIONS.md).

## Development

Install [Rust through rustup](https://rustup.rs). The checked-in toolchain file selects stable Rust with rustfmt and clippy. `cargo xtask check` needs nothing else. The C# oracle (`cargo xtask oracle`), the schema fixtures (`cargo xtask schema`) and the INI fixtures (`cargo xtask config`) also need the .NET SDK 10 and the reference checkouts; CI does not run them.

```sh
git clone https://github.com/KanikaBreed/ferroan.git
cd ferroan
cargo xtask check
```

`cargo xtask check` works the same on every platform. It checks local Markdown links, formatting, clippy with warnings denied, and the workspace tests. The same Cargo checks can be run directly:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Project guides

- [Plan](docs/PLAN.md) and [decisions](docs/DECISIONS.md)
- [Architecture](ARCHITECTURE.md) and [current tasks](TASKS.md)
- [Agent instructions](AGENTS.md) and [human contributor guide](HUMANS.md)
- [Porting rules](docs/PORTING.md), including licence headers and attribution
- [Action plan](docs/ACTION_PLAN.md) and [reference checkouts](docs/REFERENCES.md)
- [Compatibility](docs/COMPATIBILITY.md) and [testing](docs/TESTING.md)
- [Bug reports](docs/BUG_REPORTING.md) and [security](SECURITY.md)

## Licence and attribution

Ferroan is licensed under the GNU General Public License, version 3 or later; see [LICENSE](LICENSE). Code ported from Tranquillity, OpenMetaverse or Aurora Viewer keeps the copyright and licence notices of the file it came from. [NOTICE.md](NOTICE.md) lists the sources and their licences.

This is an independent project. It is not an official OpenSimulator, Tranquillity or Linden Lab release. Second Life is a trademark of Linden Research, Inc.
