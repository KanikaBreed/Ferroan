# Pin3 C# behavior oracle

This driver executes the production Tranquillity types at the revision and
repository pinned in [REFERENCES.md](../../docs/REFERENCES.md). It observes login
arrival tracking, independent close callbacks, the startup GridUser connector,
and trusted-host parsing and DNS refresh. It does not reimplement those behaviors.

```sh
cargo xtask pin3 generate
cargo xtask pin3 verify
```

`FERROAN_REF` selects a directory containing clean `OpenSim-Tranquillity` and
`libopenmetaverse` checkouts. The task validates their pins and clones writable
build copies under `.scratch/oracle-sources`. Build artifacts and private failure
diagnostics stay under `.scratch/pin3-oracle`; the reference checkouts are only
read. The driver requires .NET SDK 10 and the real scene helpers from Tranquillity's
test project, including its copied animation data.

[The fixture guide](../../fixtures/pin3/README.md) lists the observations and
injected inputs. `LifecycleCases.cs` retains the original BSD notice for its
adapted test setup; `TrustCases.cs` invokes the actual MPL-2.0 trust implementation
through reflection. `Program.cs` writes deterministic JSON and verifies it against
the committed fixtures. Transient C# logs and session identities are not emitted
as public evidence. An oracle failure reports its type and keeps details in the
ignored scratch directory for local diagnosis.
