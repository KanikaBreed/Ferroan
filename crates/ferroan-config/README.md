# ferroan-config

Reads Tranquillity's INI files the way Tranquillity does: Nini 0.9.2.16 for the text, then the region loader in `Source/OpenSim.Server.RegionServer/ConfigurationLoader.cs` or the grid loader in `Source/OpenSim.Server.GridServer/GridServerConfigSource.cs`. Both loaders are taken from OpenSim-Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`.

Nini's source is not in this repository. `tools/config-oracle` calls the `nini-core` 0.9.2.16 package (X11 licence) and `OpenSim.Framework.Util.Glob`, and writes the resolved key/value set under `fixtures/config/`. The Rust tests compare against that set.

## What matches

- `;` starts a comment. `#` does not: a line that begins with `#` is a parse error, and `#` after a value stays in the value.
- The first copy of a key inside one file wins. A later file merged on top overwrites that key in place and appends new keys.
- A repeated section drops the earlier section and appends a new one, so the section moves to the end.
- Keys written before the first section are dropped. Names and values are trimmed. A section written `[ Startup ]` keeps the leading space.
- Quotes do not use backslash escapes. A quote that is not the first character ends the value.
- `${Section|key}` and same-section `${key}` are substituted, including forward references. Only the first two `|` pieces are used. An unclosed `${` is left as text. A missing key, a missing section, or a key that refers to itself is an error.
- `Include-*` values are loaded in the order the loaders load them. A missing file is a warning, reported once, using the include text from the INI. A wildcard with no match is silent. Several matches are merged in ordinal path order. The shipped includes match one file or none.
- The region loader merges its `inidirectory` after includes, so the directory wins. The grid loader expands includes after every base file, so an include wins over the directory. `region-directory` and `grid-include` lock that difference.
- The region loader treats a missing master as fatal and `inimaster = none` as "no master". The grid loader skips a missing master.
- `[Environment]` keys are replaced from the map passed to the loader. A name that is not in the map becomes `""`. The process environment is not read.
- `unknown_keys` lists keys in a section that the caller did not name. Loading still succeeds. Tranquillity has no matching "unknown key" warning.

## Differences that are deliberate

- An `http` or `https` include is `remote-include`. It is not fetched.
- `OpenSim.xml` is `xml-config`. It is not parsed.
- Substitution stops after 4096 steps on one value (`limit`). Nini has no cap.
- An INI over 16 MiB, more than 8192 sections, more than 8192 keys in one section, more than 10000 sources, or more than 10000 glob matches is `limit`.
- The region directory filter uses an ASCII case-insensitive `.ini` suffix. Tranquillity uses the culture's `ToLower()`. For the ASCII names in the shipped files the two agree.
- Glob walks with `read_dir`. `*` matches a dotfile. The prefix before the first wildcard is canonicalized, so a Windows 8.3 name (`RUNNER~1`) or a symlink such as macOS `/var` becomes the directory name the walk will see. A symlink or junction found while walking counts as a directory or a file according to its target, which is what `Directory.GetDirectories` and `GetFiles` do. A broken link is skipped.
- The grid directory pattern is case-sensitive `.ini` except on Windows, where both loaders compare ASCII case-insensitively. Tranquillity's grid `Directory.GetFiles(dir, "*.ini")` is case-sensitive on Linux.

## Shipped files

`region-shipped` and `grid-shipped` resolve the example INI files in the pinned checkout. CI has no checkout, so those two cases are skipped unless `FERROAN_REF` (default `~/Git/reference`) points at one. The committed dumps are what a machine with the checkout checks. The example files contain published placeholder passwords (`ConsolePass` is `secret`, and similar). They are not live credentials.

Regenerate with `cargo xtask config generate`. That needs the .NET SDK 10 and does not write into the reference tree.
