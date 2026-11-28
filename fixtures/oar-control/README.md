# OAR control fixtures

`load.json` is a Pin8 `load oar` of each archive in this directory: the control file, the region each path maps to, and the loaded-scene list from `DearchiveRegion(false)`. The archives are bytes `TarArchiveWriter` wrote. Rust reads those bytes and compares its report with the record. Expected ids are the record's, not values typed into the test. Where D132 or D133 says Ferroan does not follow Pin8, the test expects Ferroan's result and the JSON stays Pin8's record.

`cargo xtask oar generate` rebuilds [the oracle](../../tools/oar-oracle/README.md) and writes this directory again. `cargo xtask oar verify` checks the committed files. The commands do not start a region. Ordinary tests do not start the oracle.

Any throw reports an empty scene list. The JSON can still name the scenes the control phase parsed. Ferroan's report leaves that list empty and keeps those regions when the throw happens after the control file.
