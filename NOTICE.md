# Notices

Copyright (c) 2026 Ferroan contributors.

New Ferroan code is distributed under the Mozilla Public License 2.0; see [LICENSE](LICENSE). Code ported from other projects keeps the licence and copyright notices of its source, as listed below and in the header of each ported file. It is provided without warranty, as each licence describes.

## Reference and porting sources

[OpenSim Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) is the behavior reference and the source of ported code. The initial reference revision is `713857f10b8e91a639a967d4354a4682b080793c` on `develop`. Each ported file names its C# source file and revision ([PORTING.md](docs/PORTING.md)). Tranquillity contains code under several licences, and a port keeps the licence of the file it came from:

| Origin | Licence | Text |
| --- | --- | --- |
| OpenSimulator, copyright Contributors, http://opensimulator.org/ | BSD-3-Clause | [licenses/OpenSimulator-BSD-3-Clause.txt](licenses/OpenSimulator-BSD-3-Clause.txt) |
| Tranquillity's own changes and new files, copyright Utopia Skye LLC, Legion Builds and others | MPL-2.0 | [LICENSE](LICENSE) |
| InWorldz Halcyon | BSD-3-Clause | [licenses/Halcyon.txt](licenses/Halcyon.txt) |
| HalcyonGrid/phlox | Apache-2.0 | [licenses/Phlox.txt](licenses/Phlox.txt) |
| Luau | MIT | [licenses/Luau.txt](licenses/Luau.txt) |
| lsl-definitions | MIT | [licenses/lsl-definitions.txt](licenses/lsl-definitions.txt) |

No code has been ported yet. This table lists the sources the plan expects to draw on, so their texts are in place before the first port.

## Native libraries (planned)

- [Jolt Physics](https://github.com/jrouwe/JoltPhysics), MIT, through `oxijolt-sys` and joltc.
- Linden Lab's [SLua engine](https://github.com/secondlife/slua), MIT (copyright Linden Lab), a fork of Luau (MIT, copyright Roblox).

Their licence texts will be added here when they become dependencies.

## Workflow

The repository layout and contributor workflow (AGENTS.md, HUMANS.md, TASKS.md, the check script and CI) follow the structure the maintainers use in Aurora Viewer and dcs-srs-rust. The text here was written for Ferroan; no Aurora or dcs-srs-rust text or code is copied.

## Names

OpenSimulator, Tranquillity, Second Life, SLua and Linden Lab names identify references and compatibility targets. Ferroan is independent and does not claim their endorsement. Second Life is a trademark of Linden Research, Inc.
