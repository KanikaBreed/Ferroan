# Asset XML fixtures

`cargo xtask asset generate` runs `tools/asset-oracle`, which calls Tranquillity `ServerUtils.SerializeResult` for `AssetBase`, `AssetMetadata`, and the bool and string values the asset handlers return. `cargo xtask asset verify` checks these files against that program. Both need the .NET SDK and `FERROAN_REF` (default `~/Git/reference`). The [Linux live workflow](../../.github/workflows/live.yml) runs `verify`; `generate` is the explicit fixture-update step.

`manifest.json` records the Tranquillity revision and the fields that produced each document. `content-types.json` is `SLUtil.SLAssetTypeToContentType` for the types the oracle asks about. `ferroan-services` encodes to these bytes and decodes the asset documents back to the same fields.

`unnamed-flags.json` records `StoredAssetFlags.FromStored` for `AssetFlags` value 8 (TB1, Pin5). The known bits are 0 and `threw` is false. It is not a `manifest.json` case. `csharp_unnamed_asset_flags_are_masked` names `crates/ferroan-services/src/asset/xml.rs` `flags_text`.

The XML files are marked binary in `.gitattributes`. `controls.xml` contains a raw CR, and the repository's LF conversion would delete it.
