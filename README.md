# Ferroan

A ground-up Rust rebuild of [OpenSim Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity), the OpenSimulator-based virtual world server.

Ferroan aims to run the same grids, viewers, content and scripts as Tranquillity, with everything in Rust except two native C++ libraries: [Jolt](https://github.com/jrouwe/JoltPhysics) for physics and Linden Lab's [SLua engine](https://github.com/secondlife/slua) for LSL and SLua scripts.

**Status: foundations and first region login.** The workspace implements packet and LLSD encodings, shared types, Tranquillity configuration and schema checks, Robust HTTP formats, and an asset service. The capture tools and `ferroan-bot` check behavior against local C# servers. `ferroan-region` registers one region with C# Robust, accepts its login messages, and serves one circuit and seed capability. Physics and the rest of the scene are still planned. Terrain layers and the no-bake avatar appearance packets are sent. The redacted C# session and oracle fixtures are committed. See [ARCHITECTURE.md](ARCHITECTURE.md) for the crate inventory and [TASKS.md](TASKS.md) for validation evidence and remaining work.

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

Install [Rust through rustup](https://rustup.rs). The checked-in toolchain file pins the exact stable Rust that CI uses, with rustfmt and clippy. [Testing](docs/TESTING.md) says how to move that pin. Do not change the machine's default toolchain. `cargo xtask check` uses the committed fixtures and needs no .NET SDK or reference checkouts. The bundled SQLite dependency needs a C compiler. The Jolt bind fetches a SHA-checked prebuilt for x86_64 Linux (glibc 2.35), x86_64 Windows (MSVC 14.44) and aarch64 macOS 11.0. A default build and CI use that archive (`JOLTC_PREBUILT=require`). `JOLTC_PREBUILT=off` builds it from source, for a host without a prebuilt (ARM Linux, Intel macOS, glibc older than 2.35, a linker override, an offline build, or a vendored tree). `JOLTC_LIB_DIR` is refused. The C# oracle (`cargo xtask oracle`), the schema fixtures (`cargo xtask schema`), the INI fixtures (`cargo xtask config`), the HTTP fixtures (`cargo xtask http`) and the asset fixtures (`cargo xtask asset`) also need the .NET SDK 10 and the reference checkouts. The [Linux live workflow](.github/workflows/live.yml) runs all five `verify` commands; the ordinary Linux, Windows and macOS jobs run `cargo xtask check`.

```sh
git clone https://github.com/KanikaBreed/Ferroan.git
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
- [Compatibility](docs/COMPATIBILITY.md), [LLUDP messages](docs/PROTOCOL.md) and [testing](docs/TESTING.md)
- [Bug reports](docs/BUG_REPORTING.md) and [security](SECURITY.md)

## Licence and attribution

Ferroan is licensed under the GNU General Public License, version 3 or later; see [LICENSE](LICENSE). Code ported from Tranquillity, OpenMetaverse or Aurora Viewer keeps the copyright and licence notices of the file it came from. [NOTICE.md](NOTICE.md) lists the sources and their licences.

This is an independent project. It is not an official OpenSimulator, Tranquillity or Linden Lab release. Second Life is a trademark of Linden Research, Inc.
