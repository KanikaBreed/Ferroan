# Full-region admission

`full-region.json` is the output of [the access oracle](../../tools/access-oracle/README.md) at pin `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` (Pin8; unchanged from Pin6 apart from that revision). Each case builds a fresh `Scene`, sets `RegionSettings.AgentLimit` to 0, attaches `DefaultPermissionsModule`, and calls `CheckLandPositionAccess` with `NotCrossing` false. The account service returns that case's `user_level` only for that case's agent.

The estate owner is `00000000-0000-4000-8000-000000000001` and the manager is `00000000-0000-4000-8000-000000000002`. A nil user is never staff. `force_grid_gods_only` forces `allow_grid_gods` and leaves the owner and manager god flags off. Those flags do not admit anyone who is not already estate staff. `captured_full_region_bypass_matches_the_staff_gate` requires `StaffGate` to agree with every `success` value.

Regenerate from a clean pin checkout:

```sh
cargo xtask access generate
cargo xtask access verify
```

The build uses `.scratch/oracle-sources` and `.scratch/access-oracle/artifacts`. The reference checkout is not modified.
