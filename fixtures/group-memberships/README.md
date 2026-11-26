# Remote parcel group memberships

The three XML files are original HTTP response bodies from Pin8 Robust (`c401bceecd1684832ae8ba86a31b5ee01a4ded7b`). The live comparison opens a disposable SQLite store with [these group memberships and roles](seed.sql). All identifiers are synthetic, and all access tokens are empty.

| Viewer | Robust's complete list |
| --- | --- |
| Member | East grants 33,554,440 from two roles. West grants zero. East is active and West still appears in the list. |
| Non-member | `RESULT=NULL` and an empty list on the region. |
| Owner | East grants 18,446,744,073,709,551,615 from its owners role, whose stored mask is zero. |

Run `cargo xtask live build` and `cargo xtask live group-memberships` under the shared live lock with a free port base and `UseSharedCompilation=false`. The region reader calls the pinned GroupsServiceRobustConnector through the capture proxy. The harness checks all three returned maps and the method, path and form bytes on every request. It saves the original response bodies in the reported scratch directory. To refresh the fixtures, copy its three XML files here. Replay them with `cargo test -p ferroan-region groups::tests`.
