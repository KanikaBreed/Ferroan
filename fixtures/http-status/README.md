# Connector HTTP status fault injection

These 36 cases execute unmodified C# connector methods against a local controlled HTTP responder. They were captured at `858321883f5487a5d192d36ea9d7b90fa468310c`. Pin8 regenerated them at `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. The outcomes are unchanged from Pin5 apart from that revision. They are **fault-injection observations**, not claims that a production Robust server emitted these statuses or bodies. The oracle implementation is [StatusCases.cs](../../tools/http-oracle/StatusCases.cs).

Every caller receives the same valid-looking success XML at 200, 201, 302 without `Location`, and 500. Requests are captured from the real C# network client; outcomes are the actual connector return values. Each case is executed twice to check determinism.

| Caller | 200 / 201 | 302 / 500 |
| --- | --- | --- |
| Grid RegisterRegion | empty success string | empty success string |
| Grid DeregisterRegion | `true` | `true` |
| Presence GetAgent | matching presence object | `null` |
| Presence GetAgents | array containing the injected presence | empty array |
| Presence LogoutAgent / ReportAgent | `true` | `false` |
| GridUser LoggedOut / SetLastPosition | `true` | `false` |
| GridUser GetGridUserInfo | matching user object | `null` |

`Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs` calls `SynchronousRestFormsRequester.MakePostRequest`; `Source/OpenSim.Framework/WebUtil.cs` reads that response without calling `EnsureSuccessStatusCode`. The Presence and GridUser connectors call `MakeRequest`, which checks the status first. Ferroan keeps both policies. `GetAgents` catches that failure and converts it to an empty array, so callers cannot distinguish this failure from an empty result by inspecting its return alone. The harness probes still require 2xx. A failed Presence or GridUser status is not proof that an agent is offline.

Both requester methods use `GetNewGlobalHttpClient`. Its shared handler follows redirects, with a limit of ten. The separately configured no-redirect handler is not the handler these form methods use. The 302 cases intentionally omit `Location` so the final response remains 302; this corpus does not claim to capture redirect-chain behavior.

The fixtures use fixed synthetic test UUIDs, a documentation-only advertised endpoint, and a test region name. They contain no credentials or live-grid identities. The injected XML is written by the oracle; it is identical across the four statuses for each caller. Request method, path, content type and form body are retained. Ephemeral loopback transport ports and timing are omitted. A Grid registration error endpoint, if returned, is replaced with `http://192.0.2.1:8003` for determinism. `outcome.accepted` means the injected success result was accepted; it does not mean C# threw an exception on every false value. `GetAgents` therefore records both `accepted: false` and its actual empty-array value.

Generate or verify on the pinned .NET SDK using:

```sh
cargo xtask http status-generate
cargo xtask http status-verify
```

The driver validates the clean reference pin and builds a scratch clone. The manifest records the revision and all case IDs. Source methods are `GridServicesConnector.RegisterRegion/DeregisterRegion`, `PresenceServicesConnector.GetAgent/GetAgents/LogoutAgent/ReportAgent`, `GridUserServicesConnector.LoggedOut/SetLastPosition/GetGridUserInfo`, and their shared `WebUtil` requesters. No reference source is copied into this repository.

Pin5 regenerated these cases. `MakePostRequest` now checks the status (TB10, #438). Grid register and deregister 302 and 500 have `outcome.accepted` false: register's value starts with `Error communicating with the grid service at `, and deregister's value is false. `csharp_make_post_request_rejects_grid_failure_status` names `crates/ferroan-region/src/grid.rs` `grid_form` and `FormStatusPolicy::Checked`. Presence and GridUser stay checked. The harness probes still require 2xx. The default `post_form` parser still accepts a non-2xx body when a caller asks for that.
