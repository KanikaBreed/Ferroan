# OAR control fixtures

`load.json` is a Pin8 `load oar` of each archive in this directory: the control file, the region each path maps to, and the loaded-scene list from `DearchiveRegion(false)`. The archives are bytes `TarArchiveWriter` wrote. Rust reads those bytes and compares its report with the record. Expected ids are the record's, not values typed into the test.

`cargo xtask oar generate` rebuilds [the oracle](../../tools/oar-oracle/README.md) and writes this directory again. `cargo xtask oar verify` checks the committed files. The commands do not start a region. Ordinary tests do not start the oracle.

A path load that fails reports an empty scene list and the same error the control phase threw. A stream load keeps the control phase beside the event the stream constructor fired.
