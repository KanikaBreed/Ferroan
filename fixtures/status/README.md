# Region and Robust status fixtures

Captured from the Tranquillity pin `f67db54399c027fd74e5aefcb2193905b4dc1646`
on 20 November 2026. The region was a standalone `OpenSim.Server.RegionServer`
with `[Startup] ManagedStatsRemoteFetchURI = ManagedStats` and
`[WebStats] enabled = true`. Robust was `OpenSim.Server.GridServer` with
`[GridInfoService]` and `[HeloService]`. No grid credentials are in these files.
The grid-info port is normalized to `8002`.

`simstatus.body`, `managed-denied.body`, `managed-cont.body` and `http404.body`
are exact response bodies. `monitor.xml` is the idle `/monitorstats/<region id>`
document. Its element order is the fixture. GC, process memory, thread count
and frame counters are live, so those numbers are not a byte oracle.
`scene-stats.txt` is the `scene` container's name order from
`StatsManager.HandleStatsRequest` (`SortedDictionary`), including `SlowFrames`.

`/ManagedStats/` itself answers with `http404.body`. `BaseHttpServer.CleanSearchPath`
trims a trailing slash, and the registrar stores `/ManagedStats/`, so the
registered path does not match. The handler bytes were taken from
`/ManagedStats/fetch`. `managed-cont.body` is `cat=scene&cont=not-the-region`:
the handler reads `cat` again when `cont` is present, the container name does
not match the region, and the category stays empty. A `cont` with no `cat`
throws and the server returns an empty 500. Other captured categories
(`clientstack`, `entitytransfer`, `httpserver`, `inventory`, `server`) are
registered by the C# process. Ferroan does not register them, so they are
omitted the way an unregistered category is omitted.

`sstats.html` and `sstats-json.json` are the default report after one all-zero
`stats_session_data` row. An empty table makes `AVG` return DBNull and the
request fails with an empty 500. The seeded JSON has `totalUsers` and
`totalSessions` of `"1"`. Ferroan has no viewer-stats database and serves the
same keys with `"0"`. The HTML fixture is the marker source (the `defaultr`
table and its column headers), not a byte copy of Ferroan's idle page.

`grid-info.xml` is `application/xml` with no extra whitespace.
`grid-json.json` is `OSDParser.SerializeJsonToBytes` (compact, defaults
omitted). `/get_grid_stats` is `405` with an empty body when GridUser data is
not loaded. `/helo` is `200` with an empty body, `X-Handlers-Provided:
opensim-robust`, and keep-alive. POST `/helo` is `405`.
