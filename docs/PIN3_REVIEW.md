# Pin3 source review

The reviewed range is Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c` to `40432e0f83d3802434f898a4e28687bd977d5cf5`: 44 commits, including two merges. The new commit is the maintainer's `dev-build`, tagged `ferroan-pin3` in [KanikaBreed/OpenSim-Tranquillity](https://github.com/KanikaBreed/OpenSim-Tranquillity/tree/40432e0f83d3802434f898a4e28687bd977d5cf5), under [D21](DECISIONS.md). Its carried fixes are **on dev-build, not yet on develop**. The pin includes upstream `develop` `80a642a681` and the eight PRs listed in [REFERENCES.md](REFERENCES.md).

This review follows the commit diffs and final C# source, including the tests; PR descriptions are not the behavior specification. The shared reference checkout stays at `8583218` until the maintainer moves it after merge. Generation and validation use clean clones under the task's `.scratch/reference`, selected by `FERROAN_REF`; builds use separate scratch copies.

## Every commit in the range

| Commit | Change read in C# | Effect on Ferroan |
| --- | --- | --- |
| `43ec490e2a` | Experience console test double declares its input availability and tests unattended prompts | Test-only; no implemented Rust console behavior changes |
| `80a642a681` | Upstream merge of the preceding experience test fix (#424) | No additional behavior |
| `7daeab2e1b` | Refuse child-agent teleports; track destination arrival and close abandoned transfers; event-assisted bounded root/presence waits | Current region accepts only initial root login. Transfer behavior belongs to P2.0i; C# harness inherits it now |
| `fdf891cc81` | Compare UDP endpoints by value; reopen child connections while retaining authorized circuit and CAPS; refuse a client whose circuit was removed | Applies also to a pre-root ViaLogin connection. Actual merged C# observation exposes TB11: replacement leaves the reopened login offline. Existing Rust endpoint restriction is retained pending maintainer disposition; crossing support remains P2.0i |
| `0e1dd1cf9f` | Reject undersized buffers entering the C# UDP pool | Rust uses bounded safe byte buffers and has no equivalent unsafe pool; capture encoding remains checked |
| `f6cc9597ad` | Queue local teleport on Task behind preceding avatar updates | P2.0h must preserve this ordering; local teleport is not implemented yet |
| `6bb0670cb7` | Increase ray budget in sculpt ray tests | Test-only; Rust physics is planned |
| `9783276432` | Wait for destination removal to finish in logout/teleport regression test | Test-only synchronization fix; no arbitrary Rust delay added |
| `3a30022b43` | Demote a stale root for a same-session non-login arrival instead of logging it out | P2.0i transfer behavior; current Rust rejects non-ViaLogin admission |
| `23896ed0e0` | Watch updates still sent to a departed child and resend CrossedRegion, bounded by time/count | P2.0i; current Rust has no child agents or crossing event producer |
| `5c14ea7ed1` | Move compacting GC off close and require all regions idle | No Rust tracing GC or equivalent collector |
| `8e7f283e6d` | Wake far-teleport release wait immediately on destination callback | P2.0i; no current region-to-region release wait |
| `a90bc6b805` | Bound variable-length zero-encoder writes before unsafe pointer writes | Rust encoder uses checked lengths and safe storage; no matching unsafe write |
| `5efede620f` | Clear stale GridUser Online when Presence has no sessions, then permit login | C# Robust inherits it now; P1.13 must carry the login-service rule |
| `c6e073d0e3` | Document Jolt and required Meshmerizer in example configuration | Config oracle rerun; no physics implementation in this task |
| `59e5165a4b` | Revive circuit tests, use fresh pooled buffers, clean hidden-group fixture | Test-only; both accepted and refused UCC acknowledgements are explicit reference evidence |
| `f7d9ce245e` | Release terrain send/save/edit locks in finally, including early refusal | Later terrain editing must use failure-safe lock ownership; P2.0e notes the source |
| `def3a48cb7` | Parcel flood selection tests each metre's owning 4 m parcel cell | Future land/terraforming behavior; no new edit route in P2.0e |
| `42a120ecf3` | Enforce region BlockTerraform except estate-authorized users | Future land permissions; record with terrain follow-ups |
| `eb0280dd34` | Refuse empty or whole-parcel subdivision | Future parcel editing |
| `1955af6dee` | Enforce AllowLandJoinDivide, with estate/admin exception | Future parcel permissions |
| `3e972ffd41` | UDP parcel updates preserve absent media/avatar settings | Future parcel update port must distinguish UDP from CAPS fields |
| `785b0ead22` | Require finite, in-parcel landing position; sanitize nonfinite look | Future parcel landing settings |
| `9411550322` | Reset displaced parent landing point before saving subdivision | Future parcel persistence |
| `f42717457e` | Advance overlay encoding past missing parcel cells | Future overlay encoder must emit public-land cells rather than loop |
| `1847b9e647` | Use region Y size for parcel query Y bounds | Future non-square parcel queries |
| `b53eaa30b0` | Include every patch cell in terrain min/max range | P2.0e must capture LayerData from this fixed encoder, including a first-row final-cell extreme |
| `99534db0f3` | Missing ModifyBlockExtended uses legacy brush index for 1/2/4 m brush | Future terrain-edit handler; current protocol still preserves observed packet bytes |
| `7cecfa5e6a` | Reject terrain selection wholly outside region before clamping | Future terraforming; prevent accidental edge edits |
| `13b4f645c8` | Reject nonfinite estate terrain settings; clamp water/raise/lower ranges | P2.0e notes fixed terrain-limit source; estate edit route remains planned |
| `d172e97abf` | Ordered, deduplicated, bounded terrain upload; idle/disconnected sender releases slot | Future estate terrain transfer; no upload implementation added |
| `4f8f1903a8` | Download terrain through per-request memory rather than a shared file | Future estate terrain download |
| `aab7de67b2` | Fire land events and finalize counts outside land-list locks | Future parcel load/join concurrency |
| `88851bf1dd` | Authorize parcel-owner enquiry before expensive prim recount | Future land permissions/counting |
| `695c013e6f` | Persist authorized parcel field edits even when no flag changes | Future parcel roles/media/identity settings |
| `30f25a1ff8` | Add LoggedOutRegion through detector, connector, handler and three stores | Port region startup call now; service/store behavior belongs to P1.3 |
| `7c096156b3` | Track accepted login until root/removal; expire unopened circuits with LoginConnectWaitSeconds | Port now with C# lifecycle observations, stored-location cleanup and session isolation |
| `e028cb2475` | Send LogoutReply in Unknown; invoke close handlers independently and continue removal | Rust already sends logout directly; regenerate capture and add observed-category evidence. Make cleanup errors nonfatal and retain independent service attempts |
| `835d6cfe77` | Publish completed column lists in six SQLite/PostgreSQL stores | Existing sqlx row decoding has no shared mutable column-name cache. Carry concurrency checks into P1.1/P1.2/P1.3/P1.6 store ports; schema migrations unchanged |
| `632bffd141` | Parse trusted addresses before URI/host-port forms, combine lists, warn on unusable entries | Port fixed forms now and replace D20's old ambiguous-host rejection proposal |
| `f89c53300c` | Preserve script marker for XML-RPC, require child-position session; refresh trusted DNS and normalize IPv6 zones | Port trust refresh/zones now. Rust already rejects script-marker POSTs; no XML-RPC server or AgentPosition PUT exists to amend |
| `b7994eb5ab` | Use in-process standalone simulation and correct HG/voice connector HTTP-server arguments | C# baseline inherits it; Hypergrid services are P1.14 and voice is planned |
| `ae893baada` | Explicit AllowPrivateGrids permits configured private-grid destinations/callbacks | P1.14 must preserve host/port authorization; ordinary local grid harness needs no Hypergrid exception |
| `40432e0f83` | Merge carried PR branches; resolve shared scene/client changes and update child-chat fixture sessions | Final merged sources reviewed. Chat test helpers now share root/child session IDs required by the new check; no Rust chat implementation added |

## Lifecycle and trust evidence

[The Pin3 oracle](../tools/pin3-oracle/README.md) calls the actual C# Scene, LLClientView, ActivityDetector, GridUser connector and ControlPlaneAccess. [Its fixtures](../fixtures/pin3/README.md) record observations rather than Rust-generated expectations. Lifecycle fixtures use the reference scene helpers, the real Presence service and a recording GridUser backend. Startup request bytes come from the actual HTTP connector against controlled success/failure replies. The close case throws from one callback and observes subsequent callbacks and removal. Trust fixtures inject DNS answers and time into C#'s supported constructor and capture parsing, combined configuration lists and refresh results.

`LoginConnectWaitSeconds` bounds the wait before a valid viewer connection, not the whole journey to root. Once connected, removal/inactivity handles an unfinished login. Zero disables this new watcher. Failed-login cleanup first checks the old Presence session, then logs it out and uses the existing GridUser last region/position/look; it does not overwrite those with a destination the viewer never reached. A later session whose predecessor is gone remains online. A failed startup `loggedoutregion` response is nonfatal, preserving compatibility with older Robust.

No migration resource changes occur in the range. Existing type, packet, schema, configuration, HTTP and asset oracles are regenerated and verified. Fresh live captures replace affected circuit/arrival/login/CAPS evidence. Historical captures whose producers did not change retain their original capture revision and an explicit current-source audit; they are not relabelled as new traffic. Final command output, CI links and durations belong to the Pin3 row in [TASKS.md](../TASKS.md).

## Scope carried forward

The task asks for notes in Phase 3 land rows, but TASKS.md has no such rows: it ends after Phase 1 and states later rows are added near their start. [PLAN.md](PLAN.md) assigns land and estate modules to Phase 5, while Phase 3 is physics. The notes above and below TASKS.md's future-phase paragraph preserve the findings without creating or reordering manager-owned tasks. P2.0e gains the patch/range source note; Phase 1 service/store rows gain the applicable startup-logout and metadata-publication notes.
