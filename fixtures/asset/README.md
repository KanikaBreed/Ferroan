# Asset XML fixtures

`cargo xtask asset generate` runs `tools/asset-oracle`, which calls Tranquillity `ServerUtils.SerializeResult` for `AssetBase`, `AssetMetadata`, and the bool and string values the asset handlers return. `cargo xtask asset verify` checks these files against that program. Both need the .NET SDK and `FERROAN_REF` (default `~/Git/reference`). The [Linux live workflow](../../.github/workflows/live.yml) runs `verify`; `generate` is the explicit fixture-update step.

`manifest.json` records the Tranquillity revision and the fields that produced each document. `content-types.json` is `SLUtil.SLAssetTypeToContentType` for the types the oracle asks about. `ferroan-services` encodes to these bytes and decodes the asset documents back to the same fields.

`unnamed-flags.json` records `StoredAssetFlags.FromStored` for `AssetFlags` value 8 (TB1, Pin5). The known bits are 0 and `threw` is false. `asset/unnamed-flags.xml` is the `AssetServerGetHandler` body (`<Flags>Normal</Flags>`). `mixed-flags.json` records value 9, Maptile mixed with bit 8: the known bits are 1 and `threw` is false. `asset/mixed-flags.xml` is that GET body (`<Flags>Maptile</Flags>`). Neither JSON file is a `manifest.json` case. `csharp_unnamed_asset_flags_are_masked` names `crates/ferroan-services/src/asset/xml.rs` `flags_text`, and `stored_unnamed_and_mixed_flags_match_the_csharp_get` serves both documents.

`loader-sample.json`, `seed.json`, `commands.json` and `library-manifest.json` are the same oracle. The sample is a small asset-set tree. The seed file is the loader callback against a copy of the SQLite fixture: a new id is inserted, matching bytes are left alone, different bytes replace the row, and a null payload stops the load. The command file is the connector text for `show asset`, `dump asset` and `delete asset`. The manifest is id, name, type and SHA1 for the pin's library; the library bytes stay in the reference checkout.

The XML files are marked binary in `.gitattributes`. `controls.xml` contains a raw CR, and the repository's LF conversion would delete it.
