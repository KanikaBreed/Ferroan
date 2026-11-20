# Root-arrival service reporting

These requests, replies and online observations were captured from Tranquillity
`f67db54399c027fd74e5aefcb2193905b4dc1646` on the local test grid.

```sh
cargo xtask live build
cargo xtask live run login grid
cargo xtask live root-arrival-fixtures .scratch/live/grid
cargo xtask live run login grid --region ferroan
```

`PresenceDetector.DoOnMakeRootAgent` calls `ReportAgent(session, region)`.
`ActivityDetector.DoOnMakeRootAgent` calls `SetLastPosition(user, region,
position, look)`; its connector does not transmit the unused session argument.
Both run synchronously for this fresh login before
`ScenePresence.CompleteMovement` sends `AgentMovementComplete`. The capture
records Presence first (record 32), then GridUser (record 34). Form field order,
percent escaping, response status, content type and XML bytes are retained.
The test agent/region and generated session UUIDs use the same placeholders as
[the earlier region login fixtures](../region-login/README.md).

The online probe queries Robust before logging out and while continuing to
drive the bot's circuit. It requires the current user/session, Presence RegionID,
GridUser Online/LastRegionID, and an exact parsed `f32` LastPosition match with
the received arrival. No tolerance is used. The C# form's `25.960001` therefore
corresponds to the captured arrival's widened value `25.96000099182129`.
Stored LookAt remains zero; the viewer arrival's fallback direction is distinct.
The fixture extractor verifies the pinned C# channel marker and matching
position/handle/look in the UDP capture. It rejects ambiguous HTTP pairings.

Rust runs the same online assertion and compares the captured C# region,
handle, XY and look values. Terrain-adjusted Z remains P2.0e: each service's
stored Z must exactly match its own actual arrival, but Rust Z is not claimed
equal to C# terrain height. The fixture form tests separately use the captured
position to prove exact C# serialization. No passwords, live session identities
or raw captures are committed. Firestorm remains pending.
