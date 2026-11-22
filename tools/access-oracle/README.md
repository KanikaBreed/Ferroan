# Access oracle

Calls pinned `Scene.CheckLandPositionAccess` with `DefaultPermissionsModule` attached. `AgentLimit` is 0 and the scene has no presences, so an ordinary account is refused with `The region is full`. An estate owner, an estate manager, and a grid god (`UserLevel` at least 200 when `allow_grid_gods` is set) are admitted first.

The driver compiles `PermissionsModule.cs` from the scratch clone of the pin. It does not copy that file into this tree and it does not start a region.

```sh
cargo xtask access generate
cargo xtask access verify
```

Observations are committed in [fixtures/access](../../fixtures/access/README.md).
