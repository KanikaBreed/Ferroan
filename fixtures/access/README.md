# Full-region admission

`full-region.json` is the output of [the access oracle](../../tools/access-oracle/README.md) at pin `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` (Pin8). Each case builds a fresh `Scene`, sets `RegionSettings.AgentLimit` to 0, initializes the module with the recorded `permissionmodules` and `serverside_object_permissions` values, and calls `CheckLandPositionAccess` with `NotCrossing` false. The account service returns that case's `user_level` only for that case's agent.

Four cases are module-level only: `default-module-absent` (`Other`), `default-module-empty` (`""`), `default-module-case-sensitive` (`defaultpermissionsmodule`) and `default-module-in-list` (`First, DefaultPermissionsModule , Last`). A Pin8 region using secure loading stops at the first missing name in each list, before admission. The Rust fixture test disables secure loading for these four cases to exercise the captured callbacks.

The estate owner is `00000000-0000-4000-8000-000000000001` and the manager is `00000000-0000-4000-8000-000000000002`. With the default module attached, a nil user is refused even when server-side permissions are bypassed. Omitting the module leaves the Pin8 permission callbacks empty and admits the ordinary account. `force_grid_gods_only` forces `allow_grid_gods` and leaves the owner and manager god flags off. Those flags do not admit anyone who is not already estate staff. `captured_full_region_bypass_matches_the_staff_gate` requires `StaffGate` to agree with every `success` value.

Regenerate from a clean pin checkout:

```sh
cargo xtask access generate
cargo xtask access verify
```

The build uses `.scratch/oracle-sources` and `.scratch/access-oracle/artifacts`. The reference checkout is not modified.
