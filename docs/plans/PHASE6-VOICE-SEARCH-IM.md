# Phase 6 plan: voice, search, offline IM and the mute list

Planning note for the manager. This file is the map. It does not add rows to [TASKS.md](../../TASKS.md) or [WORK_QUEUE.md](../../WORK_QUEUE.md). The manager allocates queue ids after review.

**Gate** ([PLAN.md](../PLAN.md)): voice, search and offline IM work in Rust regions. Phases 5 and 6 run side by side after Phase 2. The estimate in PLAN is 7 to 14 person-months. The rows below are the gate slice, 39 coder-days. What drives that split is the last section before the questions.

**Scope here:** WebRTC voice through the external Janus server, search, offline IM, and the mute list. The same sentence is the Phase 6 line in [PLAN.md](../PLAN.md).

**Standing decisions**

- [D2](../DECISIONS.md) and [COMPATIBILITY.md](../COMPATIBILITY.md). The C# server is the specification. A viewer, a script, another simulator and a grid service see the same packets, CAPS, LLSD, Robust forms, store rows, console text and INI keys.
- [D6](../DECISIONS.md). tokio for networking. Janus stays an external process. The region speaks the HTTP API the pin's client speaks. Ferroan does not embed Janus.
- [D8](../DECISIONS.md). This phase adds no native library.
- [ARCHITECTURE.md](../../ARCHITECTURE.md). Region modules live in `ferroan-region`. CAP LLSD uses `ferroan-http`. Viewer packets use the client stack. Robust handlers that duplicate P1.8, P1.9 or P1.11 stay on those rows. This phase does not host a second copy.
- [PORTING.md](../PORTING.md). `ProvisionResponseBuilder.cs` and `ProvisionParcelAccess.cs` are MPL-2.0 (Legion Builds, 2026). `InstantMessageMuteCheck.cs` carries a 2026 Legion Builds line and the OpenSimulator BSD header. The WebRTC region module, the search modules, the offline-IM modules and `MuteListModule` are the OpenSimulator BSD header. This plan does not edit NOTICE.md. A row that ports an MPL file follows PORTING.md in that row's PR.

The rows below are about a day each, in the order section 5 lists. An id is a label. A row that overruns a day is split before the next one starts, as Phase 2 did. Ids in this file run from P6.1 through P6.39. Work that does not fit is a question in section 6.

Line numbers are the file as it is at Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`. Pin9 differs from Pin8 in the Jolt module. Every file this plan cites is outside that module. Paths are under `Source/` unless they start with `Addons/` or `Tests/`.

Ferroan, at the commit this branch was cut from, has no voice module, no search module, no offline-IM region module and no mute-list module. Estate flags that hide a region from search on the handshake are not directory search. [PROTOCOL.md](../PROTOCOL.md) lists `DirFindQueryBackend`, `DirPlacesQueryBackend`, `DirClassifiedQueryBackend`, `DirLandQueryBackend` and `DirPopularQueryBackend` among messages with no generated class. Those backend names are grid-to-grid. The viewer packets this phase answers (`DirPlacesQuery`, `DirFindQuery`, `DirPopularQuery`, `DirLandQuery`, `DirClassifiedQuery`, `MapNameRequest`, `MuteListRequest`, `UpdateMuteListEntry`, `RemoveMuteListEntry`, `RetrieveInstantMessages`) are region handlers.

## 1. Voice

Janus is external. The region is a client of its HTTP API. The addon is `Addons/os-webrtc-janus`. The module name the region returns is `RegionVoiceModule` (`WebRtcVoiceRegionModule.cs` 145–148).

### What a viewer can observe

`[WebRtcVoice] Enabled` defaults to false (`WebRtcVoiceServiceModule.cs` 75, and `os-webrtc-janus.ini` line 2). While it is false, `RegionLoaded` does not register CAPS and does not set a voice feature (`WebRtcVoiceRegionModule.cs` 116–133). While it is true:

- The simulator feature `VoiceServerType` is the string `webrtc` (`WebRtcVoiceRegionModule.cs` 128).
- `StunServers` defaults to the empty string (`WebRtcVoiceRegionModule.Stun.cs` 57). A whitespace value is omitted. A non-empty value is one feature, `stun-servers`, whose LLSD value is that string (`OSD.FromString`, `WebRtcVoiceRegionModule.Stun.cs` 46–50). It is not an array. The comment on that file says a viewer that reads `SimulatorFeatures["stun-servers"]` needs a non-empty valid entry or peer-connection setup fails with an empty ICE URI.
- Login and a region crossing register three CAPS (`WebRtcVoiceRegionModule.cs` 175–196): `ProvisionVoiceAccountRequest`, `VoiceSignalingRequest`, `ChatSessionRequest`. `RegisterModerationCapability` runs on the same path (198). Call moderation defaults off (`CallModerationEnabled`, default false).

`ProvisionVoiceAccountRequest` (`ProvisionVoiceAccountRequestCore`, 211–423):

| Request | Answer |
| --- | --- |
| No `IWebRtcVoiceService` | HTTP 404, no body (215–220) |
| Method other than POST | HTTP 404 (223–227) |
| Body that is not an OSD map | HTTP 204 (231–236) |
| `voice_server_type` present and not `webrtc`, case-insensitive | HTTP 200 and the bytes `<llsd><undef /></llsd>` (65, 240–248). The comment names Firestorm's Vivox probe |
| `channel_type` `local`, estate `AllowVoice` false | HTTP 501 and the same undef bytes (270–275). `AllowVoice` defaults to true (`EstateSettings.cs` 141) |
| `local`, no land channel, or a resolved parcel with no land data | HTTP 501 (277–281) or HTTP 404 (340–347) |
| Avatar missing | HTTP 404 and undef (285–290) |
| Parcel voice off, or the avatar barred | HTTP 403 and undef (365–377) |

The parcel for a root agent comes from the avatar's position, not from the viewer's `parcel_local_id` (293–309). A mismatch is logged and the server parcel wins (350–352). A child agent's client id is a hint, accepted only when `ChildParcelProximity.IsNear`; otherwise the id is dropped and the service uses the estate room (311–323). `ParcelFlags.UseEstateVoiceChan` removes `parcel_local_id` (380–382). The server-position path writes the server local id into the map before the service call (384–388).

`ProvisionParcelAccess.Check` (`ProvisionParcelAccess.cs` 38–52, MPL-2.0) returns `VoiceOff` when `ParcelFlags.AllowVoiceChat` is unset, `Barred` when the simulator bars the avatar or `LandBan.IsBanned`, and `Allowed` otherwise. The check runs on every estate, including TaxFree (354–356). Calls, conferences, group calls and teardown are admitted by `TryAdmitProvision` and skip those parcel checks (263–265). Those features default off, so the stock path is the local channel.

The service map is serialised as LLSD XML (404–406). `error_code` equal to `JANUS_ROOM_FULL_ERROR_CODE` (495, `WebRtcJanusService.Hardening.cs` 49) becomes HTTP 409 (407–412). Every other failure map stays HTTP 200. A null service map is HTTP 200 with no body (418–421).

The three maps are pure (`ProvisionResponseBuilder.cs`, MPL-2.0, 23–59). Success keys, in order: `jsep`, `viewer_session`, `room`. Failure keys: `response` = `failed`, `error`, and `error_code` only when non-zero. Logout: `response` = `closed`. The comment says `ProvisionResponseShapeTests` pins the shape.

`VoiceSignalingRequest` (426–471) uses the same 404, 204 and non-webrtc gates. It then calls the service and discards the map. The CAP always writes the undef bytes and HTTP 200 (467–470). The comment at 467 is a TODO to check errors and package the response. The service itself builds a map (`WebRtcJanusService.cs` 380–466): no Janus session yields `response` = `error` and `error` = `no voice session`; a completed candidate, one candidate line, or a `candidates` array become trickle operations; anything else is an error map. The CAP does not return that map. Reproducing the CAP bytes is question 10. It is not one of TB229–TB232.

`ChatSessionRequest` (483–573). Non-POST is 404. A missing or deleted presence is 404. `RefuseChildChatSession` can refuse a child. A body that is not a map is 204. `TryHandleNonSpatialChatSession` returns false when A2A voice is off (`WebRtcVoiceRegionModule.NonSpatial.cs` 410–411), so the stock switch answers. A missing `method` or `session-id` is 404. `decline p2p voice`, `decline invitation`, `start conference` and `fetch history` are HTTP 200 with an empty body (533–538). `start p2p voice` builds a session id as the XOR of the other agent's UUID and the caller when `params` is a UUID, otherwise a random UUID (541–546), posts `ChatterBoxSessionStartReply`, and returns HTTP 200. No event queue is HTTP 500 (548–552). Any other method is HTTP 400 (570–571). The event (`EventQueueGetHandlers.cs` 272–295) is named `ChatterBoxSessionStartReply`, with `session_id`, `temp_session_id`, `success`, and on success a `session_info` map holding `moderated_mode.voice`, `session_name`, `type` and `voice_enabled`. The C# parameter is spelled `sucess`. The call site passes `newSessionID`, the presence name, type 2, voice-moderated false, voice-enabled true, the viewer's session id, success true, an empty error, and the agent id (556–565).

### What the region sends to Janus

The 3-argument `ProvisionVoiceAccountRequest(OSDMap, UUID, UUID)` throws `NotImplementedException` (`WebRtcJanusService.cs` 470–472). The session overload does the work (227–377). A connect failure returns `BuildFailure("janus unavailable", 0)` (243–250). `logout` true leaves the room and returns `BuildClosed` (255–268). A missing `parcel_local_id` becomes `JanusAudioBridge.REGION_ROOM_ID`, which is −999 (`JanusAudioBridge.cs` 134, used at `WebRtcJanusService.cs` 276). A JSEP whose type is not `offer` fails with `jsep type not offer` (353). No JSEP fails with `no jsep` (359). Room selection failure is `room selection failed` (303–304). A successful join is `BuildSuccess` (320). Room full sets `error_code` 495 (322). Error 485, the room gone between create and join, is `JoinRoom failed` with no `error_code`.

`CalcRoomNumber` (`JanusAudioBridge.cs` 142–172) uses `BHasherMdjb2`. A `local` room hashes the region id, the type and the parcel local id, and ignores the grid id. A `multiagent` room folds a non-empty grid id, then the channel id, then the type. An empty grid id adds nothing. Any other type throws `Unknown channel type`. `FoldHashToRoom` (`JanusAudioBridge.RoomGate.cs` 130–131) maps `int.MinValue` to `int.MaxValue` and takes the absolute value of every other code, because `Math.Abs(int.MinValue)` throws and a Janus room number is positive. `SelectRoom` (177–185) always asks the mixer to create the room. "Already exists" (486) is success (`JanusAudioBridge.RoomGate.cs` 55–77). The room description is `region/type/parcel/channel`.

`[JanusWebRtcVoice]` supplies `JanusGatewayURI`, `APIToken`, `JanusGatewayAdminURI`, `AdminAPIToken` and an optional CA file. A token shorter than 32 characters logs a warning (`WebRtcJanusService.Hardening.cs` 61–71). `RequestTimeoutMs` defaults to 5000 (`JanusSession.RequestTimeouts.cs` 43–44). The plugin name defaults to `janus.plugin.audiobridge` (`WebRtcJanusService.PluginName.cs` 38). `RemoveEmptyRooms` defaults to true and `EmptyRoomGraceSeconds` to 60 (`EmptyRoomReaper.cs` 31–33).

`[WebRtcVoice] SpatialVoiceService` defaults, in the example ini, to `WebRtcJanusService.dll:WebRtcJanusService` (in-process). The alternative `WebRtcVoice.dll:WebRtcVoiceServiceConnector` talks to a grid URI. The Robust module registers JSON-RPC methods `provision_voice_account_request` and `voice_signaling_request` (`WebRtcVoiceServerConnector.cs` 89–90). The connector calls the 3-argument overload (`WebRtcVoiceServiceConnector.cs` 108), which throws. Hosting that service is question 8. P6.16 records the form and does not host it.

`janus info` is a console command (`WebRtcJanusService.cs` 497–499). P6.15 compares its text from fixed state, with wall-clock fields masked, the same rule as the physics console rows.

### Features that stay off, and the legacy CAP

These defaults are false, and they are most of the addon: `A2AVoiceEnabled`, `GroupVoiceEnabled`, `AdhocVoiceEnabled`, `CallModerationEnabled`, `VisibilityFeederEnabled`, `VisibilityEmitEnabled`, `VisibilityArmingEnabled`, `JoinCapabilityEnabled`, `VoiceModerationEnabled`, `PositionFeedEnabled`, `ConnectorsEnabled`. `RefusalCacheSeconds` defaults to 5 (`os-webrtc-janus.ini` 204). `MaxNonSpatialSessions` is 5000 and `MaxSessionsPerAgent` is 20 in that same example. Question 1 leaves them for a later id block. P6.14 proves that, with the defaults, those paths emit nothing, and that a repeated refusal inside five seconds is the cached answer.

`ParcelVoiceInfoRequest` is not registered by the WebRTC module. Vivox and FreeSwitch register it. FreeSwitch also registers `ProvisionVoiceAccountRequest` and, when `voice_server_type` is not the value that module expects, answers `<llsd>undef</llsd>` (`FreeSwitchVoiceModule.cs` 354 and 400), which is different bytes from WebRTC's `<llsd><undef /></llsd>`. A FreeSwitch parcel-voice body carries `parcel_local_id`, `region_name` and a nested `voice_credentials.channel_uri` (482–494). A missing land or an exception is `<llsd>undef</llsd>` (431, 450, 503). A FreeSwitch provision success carries `username`, `password`, `voice_sip_uri_hostname` and `voice_account_server_name` (388–390). The password field is a fixed literal in that module. This plan does not publish the value. Question 2 leaves Vivox and FreeSwitch unported. P6.13 records that a WebRTC region does not register `ParcelVoiceInfoRequest`, and records the two undef byte strings.

### What Phase 1 and Phase 2 already cover

Nothing in Phase 1 speaks Janus. The event queue that carries `ChatterBoxSessionStartReply` is the Phase 2 CAPS queue. Parcel flags, `AllowVoice`, and the ban list are Phase 5. Until those rows exist, P6.4's test supplies the land record `ProvisionParcelAccess.Check` reads. Script instant message is [P4.F3.5](PHASE4-FUNCTIONS-COMMS.md) and the region sender is P4.30. Neither is a voice row.

### How a row is proved

A pure builder is a unit test of the LLSD bytes against `ProvisionResponseBuilder`. Admission is a table of HTTP status and body, with a scripted voice service and a scripted land record. The Janus client is recorded by pointing the pin's client at a stub gateway and keeping the HTTP it sends: create, attach, the token, the timeout, and 486 as success. Room numbers are a table of `CalcRoomNumber` and `FoldHashToRoom`, including `int.MinValue`. A live spatial provision, under the shared live lock with a free port base and `UseSharedCompilation=false`, compares the Janus request the region sends and the LLSD keys the viewer receives. The SDP bytes Janus returns are not the compare. [COMPATIBILITY.md](../COMPATIBILITY.md) is the capture rule. A Ferroan-to-Ferroan round trip is not the pass.

### Risks

The addon is about 30,081 lines. Eighteen rows cover the spatial gate. The rest waits on question 1. A viewer that probes with a non-webrtc `voice_server_type` must receive HTTP 200 and `<llsd><undef /></llsd>`, or the probe is a hard failure. The signaling CAP's discarded map means a trickle error is invisible to the viewer until question 10 says otherwise. `ProvisionResponseBuilder.cs` and `ProvisionParcelAccess.cs` are MPL-2.0.

## 2. Search

Two region modules exist. The shipped includes set `[Modules] SearchModule = "BasicSearchModule"` in `config-include/Standalone.ini` (24), `Grid.ini` (27), `StandaloneHypergrid.ini` (29) and `GridHypergrid.ini` (33). There is no Robust search server in the tree. OpenSearch is an XML-RPC client of an external URL. UserProfiles (P1.11) stores profiles, picks, classifieds and notes, and speaks JSON-RPC. Its `classifieds_info_query` is a different service from OpenSearch's XML-RPC method of the same name (question 9).

### BasicSearchModule

The module enables when `[Modules] SearchModule` equals its name. The `GetString` default is that name, `BasicSearchModule` (`BasicSearchModule.cs` 58–63 and 71–73), so a missing key enables it. It subscribes to `DirFindQuery` only while the agent is root (131–140).

People (`DirFindFlags.People`, 150–197). The text is trimmed and lower-cased when it is non-empty (144–148). An empty text sends an empty `DirPeopleReply` and does not return (152–153), so the lookup still runs and a second reply can be sent. That is TB229. The lookup is `m_Scenes[0].UserAccountService.GetUserAccounts` with the first scene's `ScopeID` (157). The cache lifetime is 30 seconds (159). Page size is 101, with the comment that a viewer page is 100 (184–194). `online` is hard-coded false (175). Mature and sort flags are ignored (196). Zero accounts send one empty reply and return (161–164), which is the second reply when the text was empty.

Groups (`DirFindFlags.Groups`, 199–251). No `IGroupsModule` sends the alert `Groups search is not enabled` and returns (201–206). An empty text sends an empty `DirGroupsReply` and does not return (208–209), the same TB229 fall-through. `FindGroups` drops groups whose `members` count is 0 (226–229). The same 101-page and the same 30-second cache apply, again on the first scene's module. Groups V2 itself is P1.10 and the Phase 5 region groups module. This row calls `FindGroups`. It does not implement groups.

Account search words are TB18, already filed. This plan does not refile them.

### OpenSearch

`OpenSearchModule` (`Addons/OpenSimSearch/SearchModule/OpenSearch.cs`) disables when `[Search]` is missing (38–41), when `Module` is not `OpenSimSearch` (43–46), or when `SearchURL` is empty (49–54). The `Module` default string, when the section exists and the key is absent, is `OpenSimSearch`. It registers `ISearchModule` and subscribes on every new client, root or child (117–128): `DirPlacesQuery`, `DirFindQuery`, `DirPopularQuery`, `DirLandQuery`, `DirClassifiedQuery`, `EventInfoRequest`, `ClassifiedInfoRequest`, `MapItemRequest`.

Transport is XML-RPC, a 30-second legacy HTTP client (`GenericXMLRPCRequest`, 134–196). A connect failure, a socket failure, an XML failure or a fault sets `success` false and `errorMessage` to `Unable to search at this time. ` with the trailing space (154, 167, 180, 189). The viewer alert is that string. A success with no `data` member is zero rows (`ResultRows`, 202–205). The comment cites the external `query.php`. `Tests/OpenSim.Region.CoreModules.Tests/Avatar/Search/Tests/OpenSearchNoResultsTests.cs` is that case.

| Viewer query | XML-RPC method | Reply |
| --- | --- | --- |
| `DirPlacesQuery` (207–253) | `dir_places_query` | `DirPlacesReply`: `parcel_id`, `name`, `for_sale`, `auction`, `dwell`. Count capped at 101 |
| `DirPopularQuery` (256–294) | `dir_popular_query` | `DirPopularReply`, cap 101 |
| `DirLandQuery` (308–357) | `dir_land_query` | `DirLandReply`: `parcel_id`, `name`, `auction`, `for_sale`, `sale_price`, `area`. A row with a null `name` is skipped |
| `DirFindQuery` (360–368) | `dir_events_query` only when `DirFindFlags.DateEvents` is set | `DirEventsReply` (371–415). Any other DirFind flag produces no reply from this module |
| `DirClassifiedQuery` (418–464) | `dir_classified_query` | `DirClassifiedReply` |
| `EventInfoRequest` (467–510) | `event_info_query` | `EventInfoReply`, or the alert `Couldn't find this event.` when the row array is empty (489–490) |
| `ClassifiedInfoRequest` (513–562) | `classifieds_info_query` | `ClassifiedInfoReply`. An empty row array returns with no alert (528–539). The comment says the viewer asks while creating a classified, before the row exists |
| `MapItemRequest` land for sale (565–646) | `dir_land_query` with flags `163840`, type `4294967295`, price `0`, area `0`, start `0` | `MapItemReply`. A parcel is placed only when its region id is one of this process's scenes and its `landing_point` parses as `x/y` |
| `MapItemRequest` events (649–722) | `dir_events_query` with text `0\|0` and maturity bits | `MapItemReply`. The event id is packed into a UUID whose last eight hex digits are the id (708–712) |

Both modules subscribe to `DirFindQuery` when both are enabled. The shipped ini enables only BasicSearch. OpenSearch's DirFind ignores People and Groups. BasicSearch's DirFind ignores DateEvents. Question 4 keeps that split.

### Map name, the avatar picker, and the login URL

`MapSearchModule` (`MapSearchModule.cs`) has an empty `Initialise` (47–49) and hooks every new client (91–94). It is on even when search modules are off. `MapNameRequest` (`LLClientView.cs` 8620, handler 10701–10707) carries the name and flags. A name shorter than 3 characters, or a name ending in `#` and shorter than 4, sends one closing `MapBlock` (`Access` = `NonExistent`, X = Y = 0, the name echoed) and the alert `Use a search string with at least 3 characters` (`MapSearchModule.cs` 104–113). Otherwise `IGridService.GetRegionsByName` is called with the first scene's scope, the name, and a limit of 20 (122). The scope is stored from the first `RegionLoaded` (44, 61). `Agents` is 0 (135). Flag 0 uses `TerrainImage`, flag 2 uses `ParcelImage`, and any other flag uses a zero image id (175–186). A name containing `. ! + | : %`, with flags equal to 2 and exactly one region, keeps the original name (118–141). A final `NonExistent` block is always appended (`AddFinalBlock`, 194–208). Flags equal to 2 and zero regions send `No regions found with that name.` (156–159). The handler swallows every exception (162). Map tiles are Phase 5. The grid service is Phase 1. People, group and map-name search answer from the first loaded scene's scope.

`AvatarPickerSearchModule` (`AvatarPickerSearchModule.cs`) reads `[ClientStack.LindenCaps] Cap_AvatarPickerSearch`. The code default is empty (61), which disables the module. `RegionServerDefaults.ini` line 993 sets it to `localhost`. On `localhost` the module registers the CAP `AvatarPickerSearch` only when `IPeople` is present (90, 114). Otherwise the configured URL is a foreign CAP. GET only; other methods are 404. Query keys are `names`, `page_size` (default 500) and `page` (default 1) (137–151). `names` missing or shorter than 3 is HTTP 400 (141). The reply is LLSD `LLSDAvatarPicker`: `next_page_url` is the raw URL, and each person has `legacy_first_name`, `legacy_last_name`, `display_name`, `username`, `id` and `is_display_name_default` (176). This CAP is not `DirFindQuery`. `IPeople` is the profiles surface P1.11 owns.

Login reads `SearchURL` (default empty) into the login response (`LLLoginService.cs` 81, 130, 676; `LLLoginResponse.cs` 267 and 1098). `GridInfo` reads `SearchServerURI` and throws `SearchServerURI configuration error` when the host does not resolve (`GridInfo.cs` 478–485). The region adds the OpenSim extra feature `search-server-url` when `SceneGridInfo.SearchURL` is non-empty and the feature is not already set (`Scene.cs` 1361–1364). `SimulatorFeaturesModule` can set the same feature from config (417–418). Three keys, three readers. P6.28 records all three.

`Source/OpenSim.Data.Model/Search/` is an EF Core design-time context. No region caller was found. The optional DataSnapshot publisher lives under `Source/OpenSim.Region.OptionalModules/DataSnapshot/`. This plan does not read its output. Question 3 leaves a Rust search service and that publisher outside P6.1–P6.39.

### What Phase 1 already covers

P1.1 is `GetUserAccounts` for people search. P1.11 is profiles, picks, classifieds storage, notes, and the confirmation of which queries the grid serves. This plan is that confirmation for the region side: directory packets are BasicSearch and OpenSearch; the avatar picker is `IPeople`; UserProfiles JSON-RPC stays P1.11. The grid `GetRegionsByName` used by map-name search is the Phase 1 grid service.

### How a row is proved

BasicSearch is a scripted account service and a scripted groups module, compared with the pin's reply packets for the same accounts. OpenSearch is a stub XML-RPC server that returns the captured replies, including a success with no `data` member and a fault. The alert string includes the trailing space. Map-name search is captured `MapBlock` bytes, including the closing `NonExistent` block. The avatar picker is captured LLSD. A Ferroan-to-Ferroan round trip is not the pass.

### Risks

Enabling both modules subscribes both to `DirFindQuery`. A query that sets both People and DateEvents then draws a people reply from BasicSearch and an events reply from OpenSearch. The shipped ini does not do that. An empty people or groups query can produce two replies (TB229). Directory results and map-name results follow the first loaded scene.

## 3. Offline IM

P1.9 owns `Addons/OpenSim.Addons.OfflineIM`: the Robust connector, and the MySQL and PostgreSQL stores. Its acceptance is an IM to a logged-out agent delivered at the next login. Phase 6 owns the region module that calls that service. The region rows prove against the pin's C# Robust, or against P1.9 once it exists. They do not reimplement the handler.

### The service P1.9 ports, cited so the region rows know the bytes

`OfflineIMService` (`Service/OfflineIMService.cs`). Realm default `im_offline` (55). `[Messaging]` overrides `[DatabaseService]` for `StorageProvider`, `ConnectionString`, `Realm` and `MaxOfflineIMs` (default 25, lines 47 and 75–78). The constructor throws when `StorageProvider` is missing (85). The first init calls `DeleteOld` (94). `StoreMessage` counts by the recipient `im.toAgentID`; at the cap the reason is `Number of offline IMs has maxed out` and the call returns false (129–131). The stored body is XML of the whole `GridInstantMessage` under the key `Message`, UTF-8 with no BOM. `FromID` is `im.fromAgentID`. `GetMessages` loads that XML and then deletes every row for that principal. `DeleteMessages` deletes by principal and by `FromID`.

`OfflineIMData` (`Source/OpenSim.Data/IOfflineIMData.cs` 32–37) is `PrincipalID`, `FromID`, and a data dictionary. The MySQL handler (`MySQLOfflineIMData.cs`) is a generic table on the realm, migration name `IM_Store`, plus `DeleteOld`: `delete from {realm} where TMStamp < NOW() - INTERVAL 2 WEEK` (39–43). The MySQL migration (`Source/OpenSim.Data.MySQL/Resources/IM_Store.migrations`, version 5) creates `im_offline` with `ID` mediumint auto-increment, `PrincipalID` char(36), `FromID` char(36), `Message` text, `TMStamp` timestamp default `CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP`, keys on `PrincipalID` and `FromID`, InnoDB, utf8. The PostgreSQL migration version 1 creates a sequence named `im_offiline_id`. Version 3 drops and recreates the table with `FromID`. The sequence name is on a path version 3 replaces. It is not a TB row. No SQLite offline store was found.

The Robust connector is POST `/offlineim` (`OfflineIMServiceRobustConnector.cs` 73, 93–107). `METHOD=GET` needs `PrincipalID`; a missing key returns `RESULT` null and `REASON` `Bad network data` (142–143). `METHOD=STORE` builds a `GridInstantMessage` from the form and returns `RESULT` true or false, plus `REASON` on failure (118–131). `METHOD=DELETE` needs `UserID` (165). Service auth comes from the Messaging config.

### The region module this phase ports

`OfflineIMRegionModule`, name `Offline Message Module V2` (`OfflineIMRegionModule.cs` 119–121). It enables only when `[Messaging] OfflineMessageModule` equals that name. An empty `OfflineMessageURL` constructs a local `OfflineIMService`. A non-empty URL constructs `OfflineIMServiceRemoteConnector`. `ForwardOfflineGroupMessages` defaults to true (46, 66). `RegionServer.ini.example` 996–1001 leaves the key commented and documents both `OfflineMessageModule` and `Offline Message Module V2`.

`RegionLoaded` requires `IMessageTransferModule`. When it is missing, the module unsubscribes `OnNewClient`, clears the scene list, logs an error, and then still executes `m_TransferModule.OnUndeliveredMessage += ...` (85–96). That is TB230. The core module has the same shape (`OfflineMessageModule.cs` 102–114), including the log text `Diabling offline messages` (112).

`RetrieveInstantMessages` is the packet of that name (`LLClientView.cs` 8722, handler 12366–12369). An inventory-offered dialog is `SendInstantMessage` direct, with the comment that otherwise the item is given twice (172–174). Every other stored IM is marked `offline = 1` and passed to `TriggerIncomingInstantMessage` (175–188). A null list is logged (167–168) and the following `foreach` would throw. The local service returns an empty list.

`UndeliveredMessage` (193–227) stores only dialogs `MessageFromObject`, `MessageFromAgent`, `GroupNotice`, `GroupInvitation` and `InventoryOffered`. Group notice and group invitation are skipped when `ForwardOfflineGroupMessages` is false (204–208). For `MessageFromAgent`, if the sender is a root in this process, they receive an IM from `System`: `User is not logged in. Message saved.` or `Message not saved: ` plus the reason (214–226). The sender IM uses dialog `MessageFromAgent`.

The core `OfflineMessageModule` (`Source/OpenSim.Region.CoreModules/Avatar/InstantMessage/OfflineMessageModule.cs`) is named `OfflineMessageModule`. It enables only when the config string equals that name. The `GetString` default is `None`, so an unset key stays off. It requires a non-empty `OfflineMessageURL`. An empty URL logs an error and disables the module. There is no local service. Retrieve POSTs `{url}/RetrieveMessages/` with the agent UUID. Question 5 ports V2. P6.33 records the core module.

### What a script sees

`llInstantMessage` is P4.F3.5. The grid send is P4.30 (`IMessageTransferModule`). Offline delivery stays P1.9 and this phase's hook on the undelivered event. This plan does not implement the sender.

### How a row is proved

The region module is tested with a scripted `IOfflineIMService` that records `StoreMessage` and returns the list `GetMessages` was given. The sender notice is a captured `ImprovedInstantMessage`. The Robust form, when a row needs it, is a capture of the pin's POST `/offlineim` body. A live delivery, under the shared live lock with a free port base and `UseSharedCompilation=false`, sends an IM to a logged-out account and compares the message the next login delivers with the message the pin delivers. The service on that run is the pin's C# Robust, or P1.9 once it exists. A Ferroan-to-Ferroan round trip is not the pass.

### Risks

TB230 fires when the transfer module is absent, which is the state before P4.30. The region rows that store and retrieve wait on that sender. A stored inventory offer that is also delivered through the scene event gives the item twice; the direct send is the pin's guard. SQLite has no offline store, so a standalone that asks for one has nothing to open until a later decision adds it. This plan does not add it.

## 4. Mute list

P1.8 owns `MuteListServiceConnector`, the mute service and the stores, together with friends. Phase 6 owns `MuteListModule` and `InstantMessageMuteCheck`. TB34, the PostgreSQL mute-list primary key, is already fixed. This plan does not refile it. TB46, TB47 and TB48 are friends, and they stay on P1.8.

### The region module

`MuteListModule` (`Source/OpenSim.Region.CoreModules/Avatar/InstantMessage/MuteListModule.cs`) enables only when `[Messaging] MuteListModule` is `MuteListModule`. The `GetString` default is `None` (55), so an unset key stays off. `RegionServer.ini.example` 1013–1015 leaves it commented. The module disables itself when `IXfer` or `IMuteListService` is missing (70–80).

A foreign user (`IUserManagement.IsLocalGridUser` false) gets an empty list when the CRC is 0 and a use-cached answer otherwise (131–137, 148–154). A null user-management module treats the user as local. Updates and removes from a foreign user are ignored (200–201, 229–230).

`MuteListRequest` (`LLClientView.cs` 8539, handler 11591–11596) carries `MuteCRC`. The region (`MuteListModule.cs` 146–196):

| Service bytes | Region answer |
| --- | --- |
| Null, and CRC 0 | `SendEmpytMuteList` |
| Null, and CRC non-zero | `SendUseCachedMuteList` |
| Length 0 | `SendEmpytMuteList` (178–181) |
| Length 1, CRC 0 | `SendEmpytMuteList` (184–187) |
| Length 1, CRC non-zero | `SendUseCachedMuteList` |
| Longer | Xfer file name `mutes` plus the agent id, then `SendMuteListUpdate` (193–195) |

`SendEmpytMuteList` (`LLClientView.cs` 4492–4504) is a `GenericMessage` whose method is `emptymutelist` and whose one parameter is empty. The typo is the method name. `SendUseCachedMuteList` (4483–4490) is `UseCachedMuteList` with the agent id. `SendMuteListUpdate` (4507–4517) is `MuteListUpdate` with the agent id and the file name.

`UpdateMuteListEntry` (198–225): mute id, name, type, flags. Mute type 1 is an agent. A self-mute returns without storing (206–207). Muting an administrator resends the list with CRC 0 and does not store (208–212). Any other update stores `MuteData` (`AgentID`, `MuteID`, `MuteName`, `MuteType`, `MuteFlags`, `Stamp` = Unix time) and calls `InstantMessageMuteCheck.Forget` (215–224). `RemoveMuteListEntry` (227–232) stores the delete and calls `Forget`.

A second class, also named `MuteListModule`, lives in `Addons/OpenSimMutelist/MutelistModule/OpenMutelist.cs`. It enables when the config string is `OpenSimMutelist` and speaks XML-RPC. Question 6 ports the core module. P6.35 records the addon and does not port it.

### The service bytes the region must accept

P1.8 writes them. Phase 6 reads them. `MuteListService.MuteListRequest` (`MuteListService.cs` 74–104). An empty database result is an empty byte array (80–81). Each line is `{MuteType} {MuteID} {MuteName}|{MuteFlags}\n`, UTF-8 (83–89). The CRC32 of the whole file is compared with the client's CRC (93–95). When they are equal and the CRC is 0, the service returns an empty array (97–98). The region then sends an empty mute list. A list whose CRC happens to be 0 is therefore replaced by an empty list when the client sends CRC 0. That is TB231. An equal non-zero CRC returns a one-byte array `{1}` (100–101), which the region treats as use-cached.

The HTTP handler is POST `/mutelist`. Methods are `get` (agent id and CRC; the result is base64 of the bytes), `update` and `delete`. The connector section is `MuteListService`, key `MuteListServerURI` (`MuteListServicesConnector.cs`). The URL is `{uri}/mutelist`. POST fields are `METHOD` `get`, `update` or `delete`, with `agentid`, `mutecrc`, `muteid`, `mutetype` (omitted when 0), `muteflags` (omitted when 0), `mutestamp` and `mutename` (omitted when empty).

MySQL `MuteListStore.migrations` version 1: table `MuteList`, `AgentID` char(36), `MuteID` char(36) default nil, `MuteName` varchar(64) default empty, `MuteType` int default 1, `MuteFlags` int default 0, `Stamp` int not null, unique `(AgentID, MuteID, MuteName)`, a non-unique key on `AgentID`, InnoDB, latin1. PostgreSQL version 2 drops the agent primary key (TB34, fixed). SQLite is still version 1 (`Source/OpenSim.Data.SQLite/Resources/MuteListStore.migrations`): `PRIMARY KEY(AgentID)` plus `UNIQUE (AgentID, MuteID, MuteName)`. The primary key admits one mute row per agent. That is TB232.

A mute name is written into the list file as stored, and a newline or a vertical bar changes how the line splits.

### Whether an IM is delivered

`InstantMessageMuteCheck.IsMutedByRecipient` is called from `MessageTransferModule.cs` 147 and `HGMessageTransferModule.cs` 132, so it waits on P4.30. Only `MessageFromAgent` and `MessageFromObject` are considered (`InstantMessageMuteCheck.cs` 162–166). A zero recipient, or a sender equal to the recipient, is not muted (168–171). An object message uses `imSessionID` as the prim; when a part is found, the id becomes the root and the object flag is cleared (175–191). A row matches the sender id or the object id (212). Flag `0x1` (`MuteFlagTextChat`, 68) skips that row (215–217). A name is split on the first two spaces, then the last `|` in the remainder is the flags (209–215). Length 0 or 1 mutes nothing (204–205). No mute service mutes nothing (193–194). The cache lifetime is 60,000 ms (71). The read timeout is 3 seconds (74). A failed or slow read lets the message through and caches the miss, with at most one warning per cache lifetime (59–63, 126–134). `Remember` ignores null and the one-byte token. `Forget` runs on a local mute change. A change on another simulator appears after the cache expires.

### How a row is proved

The region logic is a scripted `IMuteListService` returning the byte arrays the pin's service returns: empty, the one-byte token, and a multi-line file. The xfer bytes and the three reply packets are compared with captures. The CRC 0 case is reproduced (TB231). The SQLite primary key is reproduced as a store constraint (TB232) and recorded against the MySQL and PostgreSQL shapes. The IM check is a table of dialogs, ids, the `0x1` flag, an empty body and the one-byte token. A live run, under the shared live lock with a free port base and `UseSharedCompilation=false`, stores a mute through the pin's service or through P1.8, compares the list xfer, and checks that an IM from the muted agent is not delivered and an IM from anyone else is. A Ferroan-to-Ferroan round trip is not the pass.

## 5. Queue items

Each row is about a day. The dependency cell is the order. Evidence is against the pin. A Ferroan-to-Ferroan round trip does not close a row.

| Id | Title | C# source | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| P6.1 | Voice feature flags | `WebRtcVoiceServiceModule.cs` 75. `WebRtcVoiceRegionModule.cs` 116–133. `WebRtcVoiceRegionModule.Stun.cs` 46–57. `os-webrtc-janus.ini` line 2. | About a day. Closes on `Enabled` false registering nothing, and `Enabled` true setting `VoiceServerType` to the string `webrtc`. An empty `StunServers` omits `stun-servers`. A non-empty value is that string, not an array. No Janus process. | None. |
| P6.2 | The three voice CAPS | `WebRtcVoiceRegionModule.cs` 175–198. | About a day. Closes on login and a crossing registering `ProvisionVoiceAccountRequest`, `VoiceSignalingRequest` and `ChatSessionRequest` only while P6.1 enabled the module. Disabled registers none of them. Moderation registration follows the default-off path and adds no viewer behaviour on this row. | P6.1. The Phase 2 seed CAP list. |
| P6.3 | Provision admission | `WebRtcVoiceRegionModule.cs` 65 and 211–248. | About a day. Closes on HTTP 404 with no service, HTTP 404 for a non-POST, HTTP 204 for a body that is not a map, and HTTP 200 with the bytes `<llsd><undef /></llsd>` when `voice_server_type` is present and not `webrtc`. A scripted service. No Janus. | P6.2. |
| P6.4 | Estate and parcel gate | `WebRtcVoiceRegionModule.cs` 263–392. `ProvisionParcelAccess.cs` 38–52. `EstateSettings.cs` 141. | About a day. Closes on the local channel: estate `AllowVoice` false is HTTP 501 and undef; voice off is HTTP 403; barred is HTTP 403; a root uses the server parcel; a child hint is kept only when it is near; an estate channel removes `parcel_local_id`. The test supplies the land record until Phase 5 exposes `AllowVoice`, parcel flags and the ban list. MPL-2.0: the PR follows [PORTING.md](../PORTING.md). | P6.3. Phase 5 parcel and estate rows for a live parcel. The scripted land record closes this row before those rows exist. |
| P6.5 | Provision response shapes | `ProvisionResponseBuilder.cs` 23–59. `WebRtcVoiceRegionModule.cs` 399–421. `WebRtcJanusService.Hardening.cs` 49. | About a day. Closes on the success key order `jsep`, `viewer_session`, `room`; the failure keys `response`, `error`, and `error_code` only when non-zero; logout `response` = `closed`; HTTP 409 when `error_code` is 495; HTTP 200 for every other failure. No Janus. MPL-2.0, same rule as P6.4. | P6.3. |
| P6.6 | Janus session client | `JanusSession.cs` 77–88. `JanusPlugin.cs` 89–92. `JanusMessages.cs` 92 and 364–371. `JanusSession.RequestTimeouts.cs` 43–44. `JanusAudioBridge.RoomGate.cs` 55–77. `WebRtcJanusService.Hardening.cs` 61–71 and 186. `WebRtcJanusService.PluginName.cs` 38. | About a day. Closes on the HTTP the pin's client sends to a stub gateway: session create, plugin attach of `janus.plugin.audiobridge`, the API token, the 5000 ms request timeout, and create-already-exists (486) as success. A token shorter than 32 characters is the pin's warning, not a refusal. | P6.1. |
| P6.7 | Room numbers | `JanusAudioBridge.cs` 134 and 142–185. `JanusAudioBridge.RoomGate.cs` 124–131. | About a day. Closes on `REGION_ROOM_ID` −999, a local room ignoring the grid id, a multiagent room folding a non-empty grid id, an unknown channel type throwing, and `FoldHashToRoom(int.MinValue)` equal to `int.MaxValue`. A table of hashes. No live mixer. | P6.6. |
| P6.8 | Join and leave | `WebRtcJanusService.cs` 227–377. `ProvisionResponseBuilder.cs` 27–59. | About a day. Closes on an offer joined against the stub: success map, `logout` answered `closed`, missing JSEP answered `no jsep`, a non-offer answered `jsep type not offer`, room-select failure, 485 with no `error_code`, and room full with `error_code` 495. The 3-argument overload still throws. The compare is the request and the response map. | P6.7, P6.5. |
| P6.9 | Voice signaling CAP | `WebRtcVoiceRegionModule.cs` 426–471. `WebRtcJanusService.cs` 380–466. `JanusSession.cs` 202–209. | About a day. Closes on the CAP bytes: the same 404 and 204 gates as provision, non-webrtc HTTP 200 undef, and every accepted POST answered HTTP 200 undef, including when the service built an error map. The service's trickle request to the stub is recorded on the same row. Question 10. | P6.8. |
| P6.10 | Viewer session lifetime | `WebRtcJanusService.cs` 237–250 and 320–347. | About a day. Closes on a session created for a provision, removed when the provision fails before a join, and a later logout sending no leave for a session that never joined. Fixed ids. No wall clock. | P6.8. |
| P6.11 | In-process service selection | `WebRtcVoiceServiceModule.cs` 84–113. `os-webrtc-janus.ini` lines 5–11. | About a day. Closes on `SpatialVoiceService` empty disabling the module, the in-process Janus service loading when named, and the grid connector named and not hosted. The connector's request form is P6.16. | P6.1. |
| P6.12 | Chat session, stock answers | `WebRtcVoiceRegionModule.cs` 483–573. `WebRtcVoiceRegionModule.NonSpatial.cs` 373–411. `EventQueueGetHandlers.cs` 272–295. | About a day. Closes on non-POST 404, missing presence 404, a non-map 204, the four default methods answering HTTP 200 with an empty body, `start p2p voice` posting `ChatterBoxSessionStartReply` with the keys and the XOR session id, no event queue answering 500, and any other method answering 400. A2A, group and conference stay off, so `TryHandleNonSpatialChatSession` returns false. | P6.2. The Phase 2 event queue. |
| P6.13 | Parcel voice info, recorded | `FreeSwitchVoiceModule.cs` 287–290, 354, 388–400, 414–503. WebRTC registration is `WebRtcVoiceRegionModule.cs` 180–196, which does not name this CAP. | About a day. Closes on a WebRTC region leaving `ParcelVoiceInfoRequest` unregistered. The FreeSwitch and Vivox shapes are recorded, including `<llsd>undef</llsd>` and `<llsd><undef /></llsd>` as different bytes. The FreeSwitch password value is not copied into a fixture. Question 2. | P6.2. |
| P6.14 | Refusal cache, features off | `os-webrtc-janus.ini` 204 (`RefusalCacheSeconds` 5). `WebRtcVoiceRegionModule.cs` 257–261. The default-off keys in that ini: A2A, group, ad-hoc, call moderation, visibility, position feed, connectors. | About a day. Closes on a repeated local refusal inside five seconds answering from the cache, and on each default-off feature emitting no CAP, no event and no Janus request. | P6.4. |
| P6.15 | `janus info` | `WebRtcJanusService.cs` 497–527. | About a day. Closes on command text from a fixed session and a fixed room list. Wall-clock fields are masked. `janus list rooms` is the same rule if it fits the day; otherwise it is split before P6.16. | P6.6. |
| P6.16 | Grid voice connector form | `WebRtcVoiceServerConnector.cs` 89–90. `WebRtcVoiceServiceConnector.cs` 108 and 136. `WebRtcJanusService.cs` 470–472. | About a day. Closes on the JSON-RPC bodies `provision_voice_account_request` and `voice_signaling_request`, parameter keys `request`, `userID` and `scene`, compared with a capture of the pin's connector. The 3-argument service method throws, and this row records that answer. It does not host the service. Question 8. | P6.11. |
| P6.17 | Live spatial provision | The live harness. P6.4's gate, P6.8's join, P6.9's signaling. | About a day. One local offer. The Ferroan region and the pin region send the same Janus request shape to the stub, and the viewer receives the same LLSD keys. The SDP from a live gateway is not the compare. Under the shared live lock with a free port base and `UseSharedCompilation=false`. | P6.8, P6.9, P6.4. |
| P6.18 | Voice limits | The caps exercised above: refusal cache 5 s, room-full 495, body-not-a-map 204. [LIMITS.md](../LIMITS.md). | About a day. Each new cap is a LIMITS.md id added on this row, with a call site outside `ferroan-diag`. Existing ids are not renumbered. | P6.14, P6.9. |
| P6.19 | People directory | `BasicSearchModule.cs` 58–63, 131–197. Account service is P1.1. | About a day. Closes on a people `DirFindQuery`: trim and lower-case, page 101, cache 30 seconds, `online` false, flags other than People ignored. An empty text sends the empty reply and the lookup still runs (TB229), including a second reply. Both use `m_Scenes[0]`. The pass is the pin's `DirPeopleReply` for the same accounts. | P1.1. |
| P6.20 | Groups directory | `BasicSearchModule.cs` 103–116 and 199–251. | About a day. Closes on a groups `DirFindQuery`: the alert `Groups search is not enabled` when no groups module is loaded, members equal to 0 dropped, page 101, cache 30 seconds, and the empty-text fall-through (TB229). `FindGroups` is the Phase 5 region groups module over P1.10. | P6.19. Phase 5 `IGroupsModule.FindGroups` and P1.10. The alert case closes without them. |
| P6.21 | OpenSearch client | `OpenSearch.cs` 34–59 and 134–205. `OpenSearchNoResultsTests.cs`. | About a day. Closes on the module staying disabled when `[Search]` is missing, `Module` is not `OpenSimSearch`, or `SearchURL` is empty. A fault and a connect failure alert `Unable to search at this time. ` with the trailing space. A success with no `data` member is zero rows and still produces the reply packet the caller asked for. A stub XML-RPC server. | None of the voice rows. |
| P6.22 | Places | `OpenSearch.cs` 207–253. | About a day. Closes on `dir_places_query` and `DirPlacesReply` fields `parcel_id`, `name`, `for_sale`, `auction`, `dwell`, count capped at 101. The request hashtable matches the pin's keys. | P6.21. |
| P6.23 | Popular, land, and map items | `OpenSearch.cs` 256–357 and 565–722. | About a day. Closes on `dir_popular_query`, `dir_land_query` (including a null `name` skipped), and `MapItemRequest` for land for sale and for the three event maturities, including the event id packed into the UUID and a landing point that fails to parse left out. | P6.21. |
| P6.24 | Events and classifieds queries | `OpenSearch.cs` 360–464. | About a day. Closes on `DirFindQuery` with `DateEvents` calling `dir_events_query` and sending `DirEventsReply`, and on every other DirFind flag producing no reply from this module. `DirClassifiedQuery` calls `dir_classified_query`. People and groups stay on BasicSearch. | P6.21. |
| P6.25 | Event info and classified info | `OpenSearch.cs` 467–562. | About a day. Closes on `event_info_query` and `EventInfoReply`, the alert `Couldn't find this event.` when the row array is empty, `classifieds_info_query` and `ClassifiedInfoReply`, and an empty classified array returning with no alert. This is the directory packet. UserProfiles JSON-RPC of the same method name stays P1.11 (question 9). | P6.24. |
| P6.26 | Map name search | `MapSearchModule.cs` 44–49, 91–208. `LLClientView.cs` 8620 and 10701–10707. | About a day. Closes on the length gate and its alert, `GetRegionsByName` of the first scene's scope with limit 20, image id by flags, `Agents` 0, the original name kept for one region when flags are 2 and the name contains `. ! + \| : %`, the closing `NonExistent` block, the flags-2 empty alert, and a swallowed exception producing no packet. Map tiles stay Phase 5. | The Phase 1 grid service. No OpenSearch row. |
| P6.27 | Avatar picker CAP | `AvatarPickerSearchModule.cs` 61, 90, 114, 137–176. `RegionServerDefaults.ini` 993. | About a day. Closes on an empty `Cap_AvatarPickerSearch` registering nothing, `localhost` registering the CAP only when `IPeople` is loaded, GET, `names` shorter than 3 answering 400, page size default 500, page default 1, and the LLSD person keys. A foreign URL is registered as a foreign CAP and is not fetched by this row. | P1.11 for `IPeople`. It does not wait on P6.19. |
| P6.28 | Search URLs | `LLLoginService.cs` 81, 130, 676. `LLLoginResponse.cs` 267 and 1098. `GridInfo.cs` 478–485. `Scene.cs` 1361–1364. `SimulatorFeaturesModule.cs` 417–418. | About a day. Closes on an empty login `SearchURL` omitted, a set URL present in the login response, an unresolvable `SearchServerURI` throwing `SearchServerURI configuration error`, and `search-server-url` set from the scene grid info only when that feature is absent. | P6.21 for the OpenSearch URL. The login field is the Phase 1 login service. |
| P6.29 | Offline module V2 | `OfflineIMRegionModule.cs` 46, 66, 119–121. `OfflineIMService.cs` 47–94 and 129–131. `IM_Store.migrations` version 5. `MySQLOfflineIMData.cs` 39–43. | About a day. Closes on the config name `Offline Message Module V2` enabling the module, any other name leaving it off, an empty `OfflineMessageURL` selecting the local service, and a set URL selecting the remote connector. `ForwardOfflineGroupMessages` defaults true. The Robust handler is P1.9. This row cites the `im_offline` columns and the two-week delete so later rows know the store. It does not open a SQLite offline store. | P1.9 for a real store. A scripted service closes the selection. |
| P6.30 | Store an undelivered IM | `OfflineIMRegionModule.cs` 193–227. | About a day. Closes on the five dialogs being stored and every other dialog ignored, and on the sender's IM `User is not logged in. Message saved.` or `Message not saved: ` plus the reason. The cap text `Number of offline IMs has maxed out` is the service's, default 25, returned through the scripted service. | P6.29, P1.9, P4.30. |
| P6.31 | Deliver on login | `OfflineIMRegionModule.cs` 156–190. `LLClientView.cs` 8722 and 12366–12369. | About a day. Closes on `RetrieveInstantMessages` sending an inventory offer direct and every other stored IM with `offline = 1` through the scene. A null list is the log line, then the throw the pin hits. The local service returns an empty list, and that empty list sends nothing. | P6.29, P1.9. |
| P6.32 | Group notice forward flag | `OfflineIMRegionModule.cs` 204–208. | About a day. Closes on both values of `ForwardOfflineGroupMessages`: true stores a group notice and a group invitation, false drops both, and agent and object IMs are stored either way. | P6.30. |
| P6.33 | Core offline module, recorded | `OfflineMessageModule.cs` 45–114 and 196 (`POST` `{url}/RetrieveMessages/`). | About a day. Closes on a record of the core module: name `OfflineMessageModule`, default `None`, a required URL, no local service, retrieve POST `{url}/RetrieveMessages/`, and the same null-module path as TB230. The row does not port it. Question 5. | P6.29. |
| P6.34 | Live offline delivery | The live harness. P1.9's acceptance sentence. | About a day. An IM to a logged-out account is delivered at the next login as the same message the pin delivers. Service is the pin's Robust, or P1.9 once it exists. Under the shared live lock with a free port base and `UseSharedCompilation=false`. | P6.31. |
| P6.35 | Mute list request | `MuteListModule.cs` 49–80 and 131–196. `LLClientView.cs` 4483–4517, 8539, 11591–11596. | About a day. Closes on the config value `MuteListModule` enabling and `None` leaving the module off; `emptymutelist`, `UseCachedMuteList` and `MuteListUpdate` with the file name `mutes` plus the agent id; a foreign user receiving empty or use-cached and no update. A scripted mute service supplies the bytes. The OpenSimMutelist addon is recorded and not ported. Question 6. | P1.8 for the service bytes. The scripted service closes the region logic. |
| P6.36 | Mute update and remove | `MuteListModule.cs` 198–232. | About a day. Closes on a self-mute stored nowhere, an administrator mute resending the list at CRC 0 and storing nothing, any other update storing `MuteData` with a Unix stamp, a remove storing the delete, and both calling `Forget`. | P6.35. |
| P6.37 | Mute file and CRC | `MuteListService.cs` 74–104. `Source/OpenSim.Data.SQLite/Resources/MuteListStore.migrations` version 1. MySQL `MuteListStore.migrations` version 1. | About a day. Closes on the line format, an equal non-zero CRC returning the one-byte token, and CRC 0 of a non-empty file returning an empty array which the region sends as an empty list (TB231). The SQLite primary key admits one row per agent (TB232), recorded against the MySQL non-unique agent key and the PostgreSQL shape after TB34. The writer is P1.8. This row is the region's reading of those bytes and the store constraint. | P6.35, P1.8. |
| P6.38 | IM mute check | `InstantMessageMuteCheck.cs` 68–74 and 162–222. `MessageTransferModule.cs` 147. `HGMessageTransferModule.cs` 132. | About a day. Closes on agent and object dialogs only, a zero recipient and a self-IM delivered, an object IM matched on the root, flag `0x1` not muting, an empty body and the one-byte token muting nothing, the 60-second cache, and a read that fails inside 3 seconds delivering the IM. | P6.37, P4.30. |
| P6.39 | Live mute | The live harness. | About a day. A mute stored through the pin's service, or through P1.8, produces the same list xfer. An IM from the muted agent is not delivered. An IM from anyone else is. Under the shared live lock with a free port base and `UseSharedCompilation=false`. | P6.38, P6.35. |

## 6. Estimate

39 coder-days, one per row above. The voice CAP and the Janus client are P6.1 through P6.18, about 18 of those days. Search is 10, offline IM is 6, and the mute list is 5.

[PLAN.md](../PLAN.md) still says 7 to 14 person-months, at 3,000 to 6,000 lines of C# a person-month. The WebRTC addon is about 30,081 lines, which is about 5 to 10 person-months before search, offline IM and the mute list. The 39 days are the gate slice: a spatial parcel channel, the region search modules against an external search server, the V2 offline region module, and the core mute list. They do not replace the PLAN figure. Question 1 is the overflow.

## 7. Questions

Each item is open. The recommendation is a recommendation. The maintainer decides.

1. **How much of the WebRTC addon is inside P6.1–P6.39.** Recommendation: the spatial channel. That is provision, signaling, the Janus AudioBridge local and estate rooms, and the stock `ChatSessionRequest` answers. Non-spatial voice, A2A across processes, group voice, moderation, visibility, the position feed, NPC connectors, and hosting the Robust voice-session service wait for a later id block. They are most of the 30,081 lines. The gate sentence "voice works" is a spatial parcel channel. Rows: P6.1–P6.18. P6.14 records that the deferred features emit nothing at their defaults.

2. **Vivox and FreeSwitch, including `ParcelVoiceInfoRequest`.** Recommendation: leave them unported. A WebRTC region does not register `ParcelVoiceInfoRequest`. Record the legacy shapes, including the two undef byte strings. Do not publish the FreeSwitch password value. Row: P6.13.

3. **Where search is served.** Recommendation: port the region modules and treat the XML-RPC search server as external. Proof is a captured XML-RPC reply, including a success with no `data` member. A Rust search service, and the DataSnapshot publisher under `Source/OpenSim.Region.OptionalModules/DataSnapshot/`, wait for a later block. P1.11 keeps profiles, picks, classifieds storage and notes. Rows: P6.19–P6.28.

4. **Which search module a Ferroan region enables.** Recommendation: `BasicSearchModule`, matching the four shipped config-include files and the `GetString` default. OpenSearch runs when `[Search] Module` is `OpenSimSearch` and `SearchURL` is set. When both are on, both subscriptions stay: OpenSearch answers places, popular, land, events, classifieds and map items, and BasicSearch answers people and groups. Rows: P6.19 and P6.21.

5. **Which offline module.** Recommendation: `Offline Message Module V2`, because P1.9 names that addon and an empty `OfflineMessageURL` constructs a local service. The core `OfflineMessageModule` is recorded on P6.33. Rows: P6.29 and P6.33.

6. **Which mute module.** Recommendation: the core `MuteListModule`, config value `MuteListModule`. The OpenSimMutelist addon, config value `OpenSimMutelist`, is recorded and waits. Row: P6.35.

7. **Whether TB229, TB230, TB231 and TB232 are reproduced.** Recommendation: reproduce the pin's observable bytes, status codes and store constraints. A later decision can change that. The same recommendation covers map-name search answering from the first loaded scene's scope (`MapSearchModule.cs` 44 and 122). That scope is the same class of behaviour as TB229's `m_Scenes[0]`. It is not a fifth TB id. Rows: P6.19, P6.20, P6.26, P6.30, P6.33, P6.37.

8. **Whether Ferroan hosts the Robust voice JSON-RPC service.** Recommendation: leave it unhosted. In-process Janus is the gate. P6.16 records the request form, including the 3-argument method throwing. Row: P6.16.

9. **Directory classified info and UserProfiles `classifieds_info_query`.** Recommendation: the directory packet stays on OpenSearch XML-RPC. The UserProfiles JSON-RPC method of the same name stays on P1.11. The two are not merged. Row: P6.25.

10. **`VoiceSignalingRequest` answering HTTP 200 and the undef bytes, and discarding the service map.** Recommendation: reproduce that CAP. It is the answer a viewer receives today. The service map is recorded on P6.9 and is not returned. This is not one of TB229–TB232. Row: P6.9.

## 8. Bugs

These four are proposed here. They are not yet rows in [TRANQUILLITY_BUGS.md](../TRANQUILLITY_BUGS.md). The manager files them. Each is at problem level. Question 7 is whether Ferroan reproduces them.

**TB229.** `BasicSearchModule.OnDirFindQuery` (`Source/OpenSim.Region.CoreModules/Framework/Search/BasicSearchModule.cs` 152–157 and 208–213) at `41f18f983b`. An empty query text sends an empty people or groups reply and does not return, so the lookup still runs and a second reply can be sent. Both lookups use `m_Scenes[0]`, the first scene's account service, scope and groups module. People, group and map-name search answer from the first loaded scene's scope.

**TB230.** `Addons/OpenSim.Addons.OfflineIM/OfflineIMRegionModule.cs` 85–96 and `Source/OpenSim.Region.CoreModules/Avatar/InstantMessage/OfflineMessageModule.cs` 102–114, at `41f18f983b`. When no `IMessageTransferModule` is loaded, each module logs, drops its client subscription, and then dereferences the null transfer module.

**TB231.** `MuteListService.MuteListRequest` (`Source/OpenSim.Services.MuteListService/MuteListService.cs` 93–101) at `41f18f983b`. When the stored file's CRC equals the requested CRC and that CRC is 0, the service returns an empty array. The region treats an empty array as an empty mute list (`MuteListModule.cs` 178–181, `SendEmpytMuteList`). A list whose CRC is 0 is therefore replaced by an empty list when the client sends CRC 0.

**TB232.** `Source/OpenSim.Data.SQLite/Resources/MuteListStore.migrations` version 1, at `41f18f983b`, declares `PRIMARY KEY(AgentID)` on `MuteList`, with `UNIQUE (AgentID, MuteID, MuteName)` as well. The primary key admits one mute row per agent. MySQL uses a non-unique key on `AgentID` plus the triple unique key. PostgreSQL version 2 drops the agent primary key (TB34, already fixed). SQLite is still version 1.

A mute name is written into the list file as stored, and a newline or a vertical bar changes how the line splits.
