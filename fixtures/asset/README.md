# Asset XML fixtures

`cargo xtask asset generate` runs `tools/asset-oracle`, which calls Tranquillity `ServerUtils.SerializeResult` for `AssetBase`, `AssetMetadata`, and the bool and string values the asset handlers return. `cargo xtask asset verify` checks these files against that program. Both need the .NET SDK and `FERROAN_REF` (default `~/Git/reference`). CI does not run them.

`manifest.json` records the Tranquillity revision and the fields that produced each document. `content-types.json` is `SLUtil.SLAssetTypeToContentType` for the types the oracle asks about. `ferroan-services` encodes to these bytes and decodes the asset documents back to the same fields.

The XML files are marked binary in `.gitattributes`. `controls.xml` contains a raw CR, and the repository's LF conversion would delete it.
