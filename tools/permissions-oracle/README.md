# Object permission fixtures

Run `cargo xtask permissions generate` to write [objects.json](../../fixtures/permissions/objects.json). Run `cargo xtask permissions verify` to repeat the observations and report the first differing case and field. The runner builds the reference framework and links `Source/OpenSim.Region.CoreModules/World/Permissions/PermissionsModule.cs` from a clean scratch checkout at the revision in [REFERENCES.md](../../docs/REFERENCES.md).

Every case registers a child presence and a box in a fresh in-memory scene. The real `DefaultPermissionsModule` handles permission events. The viewer supplies its identity and cached membership. Test services supply the parcel, account level, group powers and, in the friend case, the owner's grant to the actor.

Each case's `settings` object supplies the values installed in `[Startup]`: `permissionmodules`, `serverside_object_permissions`, `allow_grid_gods`, `region_owner_is_god`, `region_manager_is_god`, `force_grid_gods_only`, `automatic_gods`, `implicit_gods` and `propagate_permissions`. The estate owner and manager cases vary administrative eligibility. The god cases record both the presence's effective status and its viewer god mode.

The input records the prim's masks and flags, membership, ownership and parcel permissions. In each owner or shared-group lock pair, clearing only Move from the owner's mask locks the prim while retaining Modify. One notecard narrows the owner's effective permissions. Another case clears Transfer from the base mask. The four effective mask fields record the linkset's aggregate permissions. Masks and flag words are unsigned integers.

`clientFlagInput` is the flag word on entry to the module, `moduleClientFlags` is the module result, and `generateClientFlags` is the dispatcher's result sent to LLClientView. Separate prim cases exercise Scripted, Touch, Money, AllowInventoryDrop, temporary and physics flags.

The permission decisions come from the scene dispatcher. Edit and delete are recorded for both the client and UUID overloads. Link and delink use object and actor IDs. `canSelectObject` calls `CanChangeSelectedState`, the entry used for viewer selection at Pin8. Take-copy checks copying to inventory. Duplicate also checks whether the actor can rez on the parcel.
