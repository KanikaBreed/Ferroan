# Object permission fixtures

Run `cargo xtask permissions generate` to write [objects.json](../../fixtures/permissions/objects.json). Run `cargo xtask permissions verify` to repeat the observations and report the first differing case and field. The runner builds the reference framework and links `Source/OpenSim.Region.CoreModules/World/Permissions/PermissionsModule.cs` from a clean scratch checkout at the revision in [REFERENCES.md](../../docs/REFERENCES.md).

Every case registers a child presence and a one- or two-part object in a fresh in-memory scene. The real `DefaultPermissionsModule` handles permission events. The viewer supplies its identity and cached membership. Test services supply the parcel, account level, group powers and any grant from the object's owner to the actor.

Each case's `settings` object supplies the values installed in `[Startup]`: `permissionmodules`, `serverside_object_permissions`, `allow_grid_gods`, `region_owner_is_god`, `region_manager_is_god`, `force_grid_gods_only`, `automatic_gods`, `implicit_gods` and `propagate_permissions`. The estate owner and manager cases vary administrative eligibility. The god cases record both the presence's effective status and its viewer god mode.

The 83 cases record prim masks and flags, membership, ownership and parcel permissions. The lock pairs clear only Move from the owner's mask. Additional pairs separate friend and group rights on attachments and deeded prims, including when the owner cannot transfer. The nil-group case supplies a membership response for the zero UUID to exercise the module's guard. One viewer belongs to a different group from the object. Bypass probes include moving the owner's own attachment and forced denial on a deleted target. The unowned prim exercises `ObjectAnyOwner`.

A case may specify a `child` with its own owner mask and inventory. The flag target is the root unless `child.observe` selects the child. The four effective masks always describe the whole linkset. The two child-inventory cases differ only by the root's creation date, one second apart at the legacy cutoff. The bypass cases distinguish the target part's owner mask from the linkset's effective mask. Masks and flag words are unsigned integers.

`clientFlagInput` is the flag word on entry to the module, `moduleClientFlags` is the module result, and `generateClientFlags` is the dispatcher's result sent to LLClientView. Separate prim cases exercise Scripted, Touch, Money, AllowInventoryDrop, temporary and physics flags.

The permission decisions come from the scene dispatcher. Edit and delete are recorded for both the client and UUID overloads. Link and delink use object and actor IDs. `canSelectObject` calls `CanChangeSelectedState`, the entry used for viewer selection at Pin8. Take-copy checks copying to inventory. Duplicate also checks whether the actor can rez on the parcel.

The optional `actions` inputs apply to the action probes. `rootGrant` and `partGrant` give the linked object's root or child UUID a synthetic friendship grant, which distinguishes the client and UUID delete overloads (TB96). `serviceMember` can differ from the client's membership. `rezElsewhere` requests rez at a point without a parcel while the object's own parcel permits duplication.

The oracle records the initialized module and viewer flags before changing the scene for missing/deleted or bypass probes. The action calls then receive the indicated null arguments, unresolvable IDs or deleted entities. `forceDeny` uses the permissions console handler; `noHandler` unregisters the module to exercise the scene dispatcher's defaults. Selection goes through the scene's viewer flags in that state.
