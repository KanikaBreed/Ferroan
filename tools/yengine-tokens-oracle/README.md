# YEngine token oracle

`cargo xtask yengine-tokens verify` prepares a clean checkout of the pin, builds this project once, and runs it twice. The first run compares `fixtures/yengine-tokens/digest.json`, `manifest.json` and every `objects/*.yobj`. `canonical.txt` is an oracle scratch file and is not published. The second run passes `--slots` and compares `fixtures/yengine-slots/digest.json` and its object files. That digest is the case name and each global's name, type and index, not the token-header digest. The command is not part of `cargo xtask check`.

The build compiles the pinned YEngine (`TokenBegin.Construct`, `ScriptReduce.Reduce`, `ScriptCodeGen.CodeGen`) and the oracle writes a small object file plus the token texts. Before `CodeGen`, the oracle defines the empty `XMRInstance` subclass that `XMREngine.OneTimeLateInitialization` would define, and registers the short name `xmrsuper`. The fixture scripts do not call API methods, so that subclass has no API fields. The runtime must report `10.0.14`. The harness sets `FERROAN_REF` to the prepared checkout's parent. A direct run uses `FERROAN_REF`, or `~/Git/reference` when that variable is unset. The checkout must be the pin in [REFERENCES.md](../../docs/REFERENCES.md), and `git status --porcelain` must be empty. A dirty tree is reported on its own, rather than as an empty revision.

YEngine calls `Microsoft.Extensions.Logging` without a package reference of its own. `Logging.props` adds that package for the YEngine project only. The harness copies it to `$(MSBuildUserExtensionsPath)/<MSBuildToolsVersion>/Imports/Microsoft.Common.props/ImportBefore/Logging.props` and passes `ImportUserLocationsByWildcardBeforeMicrosoftCommonProps=true`. It points `MSBuildUserExtensionsPath` and `ArtifactsPath` at a scratch directory. `UseArtifactsOutput=true` keeps the pin free of `bin` and `obj`. The same build also passes:

```
dotnet build tools/yengine-tokens-oracle/YEngineTokensOracle.csproj -c Release -m:1 \
  -p:UseArtifactsOutput=true -p:ArtifactsPath=$SCRATCH/artifacts/ \
  -p:GeneratePackageOnBuild=false -p:ContinuousIntegrationBuild=true \
  -p:UseSharedCompilation=false -p:TreatWarningsAsErrors=false \
  -p:NBGV_SetCloudBuildVersionVars=false \
  -p:ImportUserLocationsByWildcardBeforeMicrosoftCommonProps=true \
  -p:MSBuildUserExtensionsPath=$SCRATCH/msbuild
```

The built executable takes the cases file and an output directory. It writes `manifest.json`, `digest.json`, `canonical.txt` and `objects/*.yobj`.
