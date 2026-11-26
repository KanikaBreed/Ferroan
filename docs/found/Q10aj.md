# Found while working: Q10aj

- 25 November 2026, Q10aj: The local console-client build on Pin8 failed with CS0579. MSBuild included a generated `OpenSim.Framework.Version.cs` under `Source/OpenSim.Framework/tools/walk-oracle/obj/` in the shared reference tree. Git reported the checkout clean. The same console check passed from a fresh scratch clone of that commit, as the hosted jobs use.
- 25 November 2026, Q10aj: Pin8 GridServer exceeded the account-creation deadline in local loopback-only namespaces. Login and limits both passed on the same base with routed networking. Neither the failed rehearsals nor their setup time is in the wall-clock comparison.
