# OAR RAW32 terrain

Pin8 `c401bceecd` ran `load oar` on [region.oar](region.oar). The scene name is `Raw32`. The archive is a gzip tar. `archive.xml` is a single-region control file (`major_version` 0, `minor_version` 3). The terrain member is `terrains/Raw32.r32`, byte for byte [member.r32](member.r32), including seven trailing bytes the reader ignores.

[initial.r32](initial.r32) is `SaveToStream` of the scene heightmap before any load: the generated pinhead island, 256 by 256 little-endian floats, Y outer and X inner. Each later load writes that map back onto the scene first, so the merges do not pile up.

[loaded.r32](loaded.r32) is `TerrainModule.SaveToStream("loaded.r32")` after the default console load: the same 256 by 256 layout. The same process decoded the member with `RAW32.LoadStream` and `SaveStream`. The run required the two saves to be equal. `load_raw32` matches `loaded.r32`. A default console load passes bounding origin (0, 0) and the region size, with no rotation and no displacement, and this member has no −65535 sample, so the merge copies every cell.

The member's first row holds NaN, positive infinity, negative infinity, a zero with the sign bit set, 1.0001, -12.5, the subnormal with bits 1, and 100. Later cells are `((x * 17 + y * 3) % 10007) * 0.01`. NaN, both infinities, and the signed zero are stored as +0. The finite values keep their bits. None is at or above 4095, so the `.r32` save is accepted. "load oar equals LoadStream" is a statement about this 256-square. The decoder does not crop a larger square to that corner (D142).

[displacement.r32](displacement.r32) is the same archive with `--displacement=<8.9,-1.2,1.5>`. The merge floors X to 8 and Y to −2, then adds 1.5 to each written cell. [rotation90.r32](rotation90.r32) is `--rotation=90`. [rotation45.r32](rotation45.r32) is `--rotation=45`, which takes the half-to-even path. [rotation270.r32](rotation270.r32) is `--rotation=270` and [rotation-neg90.r32](rotation-neg90.r32) is `--rotation=-90`. The negative angle gains 360 before the orthogonal tests, so the two saves are equal.

[wide.r32](wide.r32) is a 512-square, one float per cell, `((x * 17 + y * 3) % 10007) * 0.01`, packed as [wide.oar](wide.oar). [bounding.r32](bounding.r32) is that archive with `--bounding-origin=<256,0,-100>`. The window reads source columns 256 through 511 into the 256-square region. [mask-member.r32](mask-member.r32) is the 256 member with cell (10, 10) set to −65535, packed as [mask.oar](mask.oar). [mask.r32](mask.r32) keeps the island's cell there and writes the other cells. [f32.oar](f32.oar) packs the original member as `terrains/Raw32.f32`. [f32.r32](f32.r32) is that default load and matches [loaded.r32](loaded.r32). [clip.r32](clip.r32) is `--bounding-size=<0,512,0>` on the original archive. Both axes clip to the region, and the save matches the default load.

[trim-in.r32](trim-in.r32) is 300 by 300 floats, value `i * 0.001` in file order. [trim-out.r32](trim-out.r32) is `LoadStream` then `SaveStream`: the first 256 by 256 samples, read straight through the file.

[section-in.r32](section-in.r32) is the file for `LoadFile(path, 1, 1, 2, 99, 16, 16)`. `fileHeight` 99 is unused. [section-out.r32](section-out.r32) is that channel saved. The first output sample is NaN in the file and +0 in the channel. The next is a signed zero in the file and +0 in the channel.

[tiny-in.r32](tiny-in.r32) is 16 zero floats. `LoadFile` of a 4 by 4 section returns them ([tiny-out.r32](tiny-out.r32)). [taint-in.r32](taint-in.r32) is the same length with the first sample set to 1. That load throws `IndexOutOfRangeException` ([taint.txt](taint.txt)). The channel is 4 by 4, so the taint array has length 0, and the changed sample calls `TerrainTaintsArray.Set`. [word-in.r32](word-in.r32) is a 17 by 16 section with 1 at x=16. The taint array is one bit, which still occupies one int, so that sample is stored ([word-out.r32](word-out.r32)).

[short.r32](short.r32) is 100 zero bytes. `LoadStream` throws `EndOfStreamException` ([short.txt](short.txt)).

## Reproduce

`cargo xtask raw32 verify` rebuilds [tools/raw32-oracle](../../tools/raw32-oracle/README.md) against a task-owned clone of the pin and diffs a fresh run with this directory.

To run it by hand, build that project against a scratch clone of the pin. Run the built program from `tools/raw32-oracle/bin/Debug/net10.0`, so `OpenSim.Tests.Common.dll` loads, under the shared live lock with a free port base and `UseSharedCompilation=false`. Pass an empty output directory. The run prints `load oar matches LoadStream`.
