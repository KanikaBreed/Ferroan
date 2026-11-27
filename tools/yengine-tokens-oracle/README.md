# YEngine token oracle

Compiles the pinned YEngine (`TokenBegin.Construct`, `ScriptReduce.Reduce`, `ScriptCodeGen.CodeGen`) and writes a small object file plus the token texts. Before `CodeGen`, it defines the empty `XMRInstance` subclass that `XMREngine.OneTimeLateInitialization` would define, and registers the short name `xmrsuper`. The fixture scripts do not call API methods, so that subclass has no API fields. The runtime must report `10.0.14`. `FERROAN_REF` is the parent of the OpenSim-Tranquillity checkout, or `~/Git/reference` when that variable is unset. The checkout must be the pin in [REFERENCES.md](../../docs/REFERENCES.md).

YEngine calls `Microsoft.Extensions.Logging` without a package reference of its own. `Logging.props` adds that package for the YEngine project only. Copy it to `$(MSBuildUserExtensionsPath)/Current/Imports/Microsoft.Common.props/ImportBefore/Logging.props` and pass `ImportUserLocationsByWildcardBeforeMicrosoftCommonProps=true`. Point `MSBuildUserExtensionsPath` and `ArtifactsPath` at a scratch directory. `UseArtifactsOutput=true` keeps the pin free of `bin` and `obj`.

```
dotnet build tools/yengine-tokens-oracle/YEngineTokensOracle.csproj -c Release -m:1 \
  -p:UseArtifactsOutput=true -p:ArtifactsPath=$SCRATCH/artifacts/ \
  -p:GeneratePackageOnBuild=false -p:ContinuousIntegrationBuild=true \
  -p:UseSharedCompilation=false -p:TreatWarningsAsErrors=false \
  -p:NBGV_SetCloudBuildVersionVars=false \
  -p:ImportUserLocationsByWildcardBeforeMicrosoftCommonProps=true \
  -p:MSBuildUserExtensionsPath=$SCRATCH/msbuild
```

Run the built executable with the cases file and an output directory. It writes `manifest.json`, `digest.json` and `objects/*.yobj`.
