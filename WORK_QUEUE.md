# Work queue

The project manager orders Ferroan's work here. [TASKS.md](TASKS.md) records what each task is and the evidence that it is done. This file says what to do next and in what order.

## How the coder uses it

The coder works unattended, through the queue, as far as it can get. The maintainer pauses or restarts it.

1. Take the first item marked `READY` whose `after` item is `DONE`, and whose TASKS.md dependencies are all `DONE`. Do not start anything else: not other plan items, not B04.
2. Work it the way [AGENTS.md](AGENTS.md) describes: a task worktree, the C# reference at the pinned revision, the checks, and a PR using the template.
3. Prove it by automation. Use unit and fixture tests against bytes captured from C#, `ferroan-bot` scenarios, and Ferrobot checks once Q10 is done, all in the live harness. A step that only a person can check is recorded as pending, never as passed.
4. Set the item to `IN PROGRESS` with the branch when you start. In the same PR, set it to `DONE` with the PR number and fill the TASKS.md row with the evidence.
5. Merge with `gh pr merge --auto --squash`. `main` requires the Linux, Windows, macOS and live checks, so the PR lands only when they pass. Never use `--admin` and never bypass the protection. If a check fails, fix the cause. After three attempts at the same failure, set the item to `BLOCKED`.
6. If an item is wrong, too big or blocked, set it to `BLOCKED`, add the reason and a proposed fix or split under it, and open that change as its own PR. Do not change an item's scope yourself. Then go on to the next item that can be taken, skipping items that depend on the blocked one.
7. Gates (`G0`, `G1`, `G2` …) are the maintainer's Firestorm checks. Never mark one. Items that depend on a gate wait, and the rest go on.
8. Write anything you find that is not your item (a flaky test, a C# bug, a gap in a row) under "Found while working" below, with the date. A C# bug also gets a row in [docs/TRANQUILLITY_BUGS.md](docs/TRANQUILLITY_BUGS.md). The manager turns it into items. A flaky check is never retried silently: record it there.
9. Stop only when no item can be taken, and say why.

Only the manager adds, removes, reorders or rewrites items. The coder changes only the Status column, the notes under an item, and "Found while working".

Status values: `READY`, `IN PROGRESS`, `BLOCKED`, `DONE`.

## Queue

| # | Item | Status |
| --- | --- | --- |
| Q1 | P2.0a: the LLUDP circuit in `ferroan-client` | DONE (#20) |
| Q2 | P2.0b: seed capability and event queue | DONE (#21) |
| Q3 | P2.0c: harness grid mode | DONE (#22) |
| Q4 | Pin2: move the Tranquillity pin to `develop` `8583218` | DONE (#24) |
| Q5 | P0.10f: fix `live assets` and add `live up --assets ferroan` | DONE (#25) |
| Q6 | P2.0d: login through C# Robust | DONE (#26) |
| Q7 | P0.10g: harness fixes from the review of #22 | DONE (#29) |
| Q8 | P2.0a2: circuit fixes from the review of #20 | DONE (#31) |
| Q9 | P2.0b2: the event queue as Tranquillity answers it, with the review of #21 | DONE (#32) |
| Q10a | P2.0d2: login arrival (`AgentMovementComplete`) | DONE (#35) |
| Q10c | P0.10j: trailing blocks Tranquillity sends beyond libopenmetaverse's template (`ImprovedInstantMessage` and seven more) | DONE (#40) |
| Q10p | Pin3: move the Tranquillity pin to bring in the C# login, logout and crossing fixes | BLOCKED (waits for Tranquillity PRs #418, #419 and #428 to merge) |
| Q10 | P0.10h: Ferrobot as the second test client in the live harness | BLOCKED (after Q10p) |
| Q10b | P0.10i: follow-ups from Q7 to Q9 (LLSD binary, dropped-ID queue, documentation) | DONE (#37) |
| Q10d | P0.10k: harness and test-evidence fixes from the review of #25 to #37 | DONE (#44) |
| Q10e | P2.0a3: circuit timing as C# does it (resends, inactivity, re-login) | DONE (#45) |
| Q10f | P2.0d3: region login: presence, trust and C# parity | DONE (#46) |
| Q11 | P0.11: container images and the example compose grid | READY after Q10f |
| Q10g | P2.0d4: root-arrival Presence and GridUser reporting | DONE (#48) |
| Q10h | P2.0d5: the shared HTTP form client checks the status as C# does | READY after Q10g |
| Q12 | P0.12: monitoring: Tranquillity's status endpoints, plus metrics and health checks | READY after Q11 |
| Q13 | P0.13: logging hooks: stdout, `LOGDIR` files, JSON, OpenTelemetry export with a collector adapter, and Tranquillity's remote console | READY after Q12 |
| Q14 | P2.0e: terrain from the C# region store | READY after Q10h (Q11 to Q13 wait for Q10) |
| Q15 | P2.0f: appearance from the C# avatar service | READY after Q14 |
| Q16 | P2.0g: local chat | READY after Q15 |
| Q17 | P2.0h: teleport inside the Ferroan region | READY after Q16 |
| Q18 | P2.0i: teleport to the C# neighbour and back | READY after Q17 |
| Q19 | P2.0j: grid-mode report comparison and the first-login checklist | READY after Q18 |
| Q20 | P1.A1: `AssetService` on MySQL and PostgreSQL, and the pilot's gaps | READY after Q19 |
| Q21 | P1.0: harness for Phase 1 (`--rust`, `live compare`, `--db`) | READY after Q20 |
| Q22 | P1.A2: `XAssetService` | READY after Q21 |
| Q23 | P1.A3: FSAssets | READY after Q22 |
| Q24 | P1.1: user accounts | READY after Q23 |
| Q25 | P1.2: authentication and authorization | READY after Q24 |
| Q26 | P1.3: grid user, presence and agent preferences | READY after Q25 |
| Q27 | P1.4: avatars and baked textures | READY after Q26 |
| Q28 | P1.5: inventory and the library | READY after Q27 |
| Q29 | P1.6: grid service | READY after Q28 |
| Q30 | P1.7: map tiles | READY after Q29 |
| Q31 | P1.8: friends and the mute list | READY after Q30 |
| Q32 | P1.9: offline IM | READY after Q31 |
| Q33 | P1.10: Groups V2 | READY after Q32 |
| Q34 | P1.11: profiles and search | READY after Q33 |
| Q35 | P1.12: estates and Experiences over Robust | READY after Q34 |
| Q36 | P1.13: login | READY after Q35 |
| Q37 | P1.14: hypergrid (split first) | READY after Q36 |
| Q38 | P1.15: money server | READY after Q37 |
| Q39 | P1.16: all-Rust Robust in the harness | READY after Q38 |

Q1: [PR #20](https://github.com/KanikaBreed/Ferroan/pull/20).

Q2: [PR #21](https://github.com/KanikaBreed/Ferroan/pull/21).

Q3: [PR #22](https://github.com/KanikaBreed/Ferroan/pull/22).

Q4: [PR #24](https://github.com/KanikaBreed/Ferroan/pull/24). Q5: [PR #25](https://github.com/KanikaBreed/Ferroan/pull/25). Q6: [PR #26](https://github.com/KanikaBreed/Ferroan/pull/26).

Q7 to Q9 were added on 18 November 2026 from a review of #20, #21 and #22. They come first because grid-mode results are not reliable until the proxy timeout is fixed, and later rows build on the circuit and the event queue. The findings were not verified. Confirm each one, and record any that are wrong in the row.

Q10 was added on 18 November 2026 ([D17](docs/DECISIONS.md)). Q11 to Q13 ([D18](docs/DECISIONS.md)) were added the same day, so the remaining P2.0 work and Phase 1 run with containers, monitoring and logs in place. It comes after the event-queue fix, so that Ferrobot does not meet the immediate empty-poll answer.

Q9 review at `8583218`: the hold, ordinary empty-poll id behavior, seed parse errors and URI name conversion are confirmed. Two review statements need qualification: seed `OPTIONS` is intercepted with 200/CORS before the non-POST guard, and C# itself uses unchecked signed id arithmetic with negative-id handling. Its crossing re-registration markers can change the id even when their batch produces no events; ordinary empty polls do not. Seed success uses the HTTP default `text/html;charset=UTF-8`, successful event batches use `application/xml`, and empty 502 replies use `text/html`. Crossing queue reuse remains with its later item.

Q10 historical blocker (#33, 18 November 2026, `feat/ferrobot-harness`; resolved by Q10a in #35): P2.0d does not yet send `AgentMovementComplete`. At pinned Ferrobot `9cc7ab284c78b7f17ae209039e8a3405a12620b6`, `crates/client/src/network/mod.rs::on_arrival` marks the client online only after that packet; `crates/ferrobot/src/api/mod.rs` refuses signed commands before then. Ferroan `860c704` validates the viewer's `CompleteAgentMovement` but never emits the arrival reply, and its bot's `run_until_ready` currently accepts the handshake without arrival. A local diagnostic with an unmerged Q10 candidate built the pinned client successfully, then failed the 45-second login check with `Ferrobot API reports no bot connected yet`. UDP capture showed one circuit open, one movement request, one handshake and reply, eight ping exchanges, and zero arrival replies. This is a missing server prerequisite, not a Ferrobot bug; no client workaround or external issue was made. Partial harness work is preserved locally, with diagnostic logs/captures under the task worktree's `.scratch/`; it is not shipped by this status-only change.

The Q10 fix proposed in #33 (implemented as Q10a in #35) was for the manager to add a prerequisite for the Rust region to send the reliable arrival packet from Tranquillity `Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs::MoveAgentIntoRegion` and its login call path at `858321883f5487a5d192d36ea9d7b90fa468310c`, carrying the actual agent/session, position/look, region handle, timestamp and channel/version. Require captured C# byte evidence and fresh `AgentMovementComplete` in `ferroan-bot` readiness. Requeue Q10 afterward. At that attempt Q11–Q39 were ineligible through their ordered dependency on Q10; no item was skipped into implementation.

Q10a and Q10b were added on 18 November 2026 from the Q10 blocker and "Found while working". Q10a is the prerequisite the blocker proposed. Q10 then resumes from its preserved partial work. Q10b waits only on Q10a, so it still runs if Q10 blocks again. Q11 also needs Q10 through its TASKS.md dependencies.

Q10g and Q10h (added 18 November 2026, from Q10f's Found while working) sit below Q11 in the table, but come first: Q11 depends on P0.10h, which is BLOCKED, so Q11 is not eligible until Q10 resumes. After Q10h, the next eligible items are Q14 onward (P2.0e to P2.0j), which do not depend on Q10.

When Q10 resumes from #41: Q10f's PR uses TB5 for the C# trusted-host parsing bug. #41's logout race takes the next free TB number when its conflict with `main` is resolved.

Q10p and Q10 (manager, 19 November 2026): the maintainer's C# fix for the lost `LogoutReply` is [Tranquillity PR #428](https://github.com/OpenSim-NGC/OpenSim-Tranquillity/pull/428), stacked on #419 and #418. It also fixes users left online and simultaneous logins (TB6 to TB9). Q10p (Pin3) stays BLOCKED until all three are merged into `develop`, and the manager then sets it READY. Q10 follows it and resumes from commit `85ee0f1`, which is in the history of `feat/ferrobot-harness-complete`. #41 was closed, and its TB5 logout entry is now TB6. The strict logout-reply check stays as it is.

Q10 BLOCKED by the manager (18 November 2026): the live CI failure on #41 (C# standalone gets no `LogoutReply`; runs 44405800874 and 44408547716) is a confirmed Tranquillity login issue, and the maintainer has a C# fix coming. Do not investigate it further, and do not retry #41's live job. Leave #41 open as it is and continue with Q10d. When the fix is on Tranquillity `develop`, the manager queues a pin move. After that, Q10 resumes from #41 with the strict logout-reply check unchanged. When resuming, merge `main` into the branch and keep `main`'s versions of the Q10 and P0.10h status lines.

Q10 BLOCKED again (18 November 2026, #36, `feat/ferrobot-harness-resume`): the resumed harness reaches the C# standalone IM check, then the mandatory capture check fails: `record 442 (udp out) does not re-encode: the encoded bytes differ`. Raw UDP record 427 is `ImprovedInstantMessage`, 191 bytes; re-encoding yields 187 identical prefix bytes and drops four trailing zero bytes. At the pin, `LLClientView.SendInstantMessage` appends that undocumented `EstateBlock` after `BinaryBucket`, while the pinned message template ends at `BinaryBucket` (TB4). This is a shared protocol/capture prerequisite, not a Ferrobot bug. No packet was filtered or canonicalized to hide the discrepancy; no retry is claimed. Proposed fix: the manager adds a prerequisite deciding and implementing bounded preservation of this observed suffix in lossless capture/replay (with a C# fixture), or takes an upstream fix through a pin move and regenerates affected fixtures. Then resume Q10's implementation preserved at `789fe59` / local branch `wip/ferrobot-harness-arrival-ready`, with `.scratch/q10-arrival-ready.patch` in the existing worktree. The initial Rust-region diagnostic passed arrival, 120 seconds and SIGTERM logout after Q10a; full Q10 acceptance remains pending. Q10b can proceed independently; Q11 waits on Q10.

Q10c was added on 18 November 2026 from the second Q10 blocker (#36). The four bytes are `ImprovedInstantMessage`'s `EstateBlock`, which Linden's viewer template defines and libopenmetaverse's lacks, so TB4 is a gap in Ferroan's template, not a Tranquillity bug. Q10 then resumes from `789fe59`.

Q10d to Q10f were added on 18 November 2026 from a review of #25 to #37 at `ebda8a9` (four reviewers: region, client stack, harness, test evidence). No high-severity defect was found, and every fixture checked came from C#. They come before Q11 because the harness fixes keep later CI honest, the resend and inactivity fixes matter for G2 on a real network, and P0.11's containers need the configurable control-plane trust in Q10f. Q10d waits for Q10 so it does not collide with the Ferrobot harness work. Confirm each finding first, as with Q7 to Q9.

Q10d: [PR #44](https://github.com/KanikaBreed/Ferroan/pull/44), taken independently after the manager's Q10 hold in #43. All thirteen findings were confirmed and addressed. [CI](https://github.com/KanikaBreed/Ferroan/actions/runs/44421899638) passed Linux 2m37s, Windows 4m35s and macOS 1m26s; [live](https://github.com/KanikaBreed/Ferroan/actions/runs/44421899822) passed in 9m44s. Recovery passed 200 consecutive default-parallel runs in 218 seconds. TASKS.md records test names, captures, startup interruption, database isolation and asset checks. Firestorm remains pending. Q10 and #41 remain held for the upstream fix.

Q10e review qualifications at `8583218`: finding 4's `KickUser` is a simulator-shutdown artifact, confirmed by the fresh C# capture's reason `The simulator is going down.` and `Scene.Close`; ordinary `CloseAgent(false)` does not call `Kick`. No ordinary-logout kick is added. `KillObject` needs the avatar local ID and scene broadcast path in P2.0f. Finding 5 is incorrect: `UnackedPacketCollection` uses a `SortedDictionary<uint, OutgoingPacket>` and selects its first numeric key. Ferroan already matches that ordering, which is retained with wraparound coverage. Findings 1–3 and 6 are confirmed.

Q20 to Q39 were added on 18 November 2026: all of Phase 1, starting with Tranquillity's three asset backends. They come after P2.0j, so they run while the maintainer does G2 (rule 7). G1 follows Q39.

Q18: size it from the C# first, and set it to `BLOCKED` with a proposed split if it is more than two weeks of work.

Each item's scope and acceptance is its row in TASKS.md. The rows were revised on 18 November 2026: the first Ferroan region logs in through C# Robust and is not a login server, and the harness gains a grid mode first. Work from the rows as they stand on `main`, not from the earlier split in #18.

## Found while working

- 2026-11-18, Q10d first CI: [run 44421488081](https://github.com/KanikaBreed/Ferroan/actions/runs/44421488081) rejected the new proxy counter's `AtomicU64::fetch_update` under newer Rust's deprecation warning (Linux 1m18s, macOS 42s, Windows 2m18s; local stable is 1.97.1). The counter now uses an equivalent saturating compare-and-swap loop supported by both compilers. No lint is suppressed; this is a code repair before a fresh CI run, not a flaky retry.

- 2026-11-18, Q10d local validation: the first repeated `grid_state` run exposed a readiness race in the new oversized-environment regression fixture: immediately after spawning a child, `/proc/<pid>/environ` could still be empty. The fixture now waits for an explicit child-ready handshake before checking the environment. This does not serialize tests or change production recovery. The failed run is retained in `.scratch/q10d-grid-state-stress/run-1.log`; the 200-run count restarts after the correction.

- 2026-11-18, Q10d integration validation: the first standalone run passed, then grid mode failed with one newly counted HTTP proxy exchange failure. The previous wrapper discarded the exchange error; diagnostics now retain its message alongside the count before another run. No failure is ignored. A preliminary recovery stress run also encountered `ENOENT` while spawning its own test executable during a concurrent Cargo rebuild (run 64, `.scratch/q10d-grid-state-stress-fixed/run-64.log`). Final stress validation uses a private copy of the exact compiled test binary with the default parallel runner; tests remain enabled and parallel.

- 2026-11-18, Q10d recovery stress fixture: copied-binary run 77 exposed the same post-spawn `/proc` visibility window in the new injected-permission-error case. All recovery test children now signal readiness before tests inspect their identity, still before recording the launch; interrupted-spawn recovery remains tested. Production classification is unchanged by this fixture correction. The log remains in `.scratch/q10d-grid-state-stress-final/run-77.log`, and final validation restarts the 200-run count.

- 2026-11-18, Q10d proxy validation, second grid run: the home proxy reported `connection closed` before recording a request; all 40 captured requests had complete responses. Stopping that listener then caused the return crossing to fail with connection refused. The bot cancels its outstanding poll when the destination becomes ready. The proxy must distinguish a clean TCP close before any HTTP request bytes from a truncated HTTP exchange; tests cover both, while partial headers/bodies and failed responses still fail the run. The second capture is retained in `.scratch/q10d-grid-second-capture/`; the next full grid run validates that distinction.

Q10d validation follow-up: after distinguishing unused zero-byte connections from HTTP exchanges, standalone followed by full grid passed (717/852 UDP records, 32/34 HTTP records; check/redact/replay passed), including neighbour and return teleports. The three region databases are regular files with distinct inodes. The final recovery stress run passed 200/200 in 218 seconds (2,200 checks, default parallel runner); earlier failed test setups remain recorded above. No failed exchange or partial request is suppressed.

- 2026-11-18, Q10c final CI: Linux [run 44402778694](https://github.com/KanikaBreed/Ferroan/actions/runs/44402778694) failed after 2m8s in the unchanged `grid_state::tests::interrupted_spawn_before_record_is_recovered_without_touching_identical_unmarked_process`: `/proc` recovery returned `could not inspect a process environment during grid recovery; state preserved`. The same implementation passed Linux 3m5s and the live job 9m49s in the preceding run; only task evidence and README text changed. `scan_pending_bounded` aborts on an environment-open error for any still-live same-UID process newer than its controller, before checking launch identity. The CI error omits the OS error, so its precise cause is not proven. This is separate from Q10c's protocol change; Q10d already covers recovery scan limits, and should also investigate unrelated unreadable process environments. One explicit CI retry is requested after recording this intermittent failure; no test is disabled and no recovery policy is changed here. Three occurrences of the same failure block the item.

- 2026-11-18, Q10b CI (no item: fixed in #37): the first Windows check on `e77dde1` failed Clippy 1.100's `needless_range_loop` in the unchanged `ferroan-types::texture::paint` helper ([run 44329492971](https://github.com/KanikaBreed/Ferroan/actions/runs/44329492971), Windows 2m30s). Local stable was 1.97.1. The minimal iterator conversion preserves the same selected faces and values; existing texture fixtures validate it. This required-check repair is recorded explicitly; no flaky retry or lint suppression was used.

- 2026-11-18, Q10b documentation audit (added to P1.A1, Q20): TB1 said Ferroan refuses unnamed asset flags, but `crates/ferroan-services/src/asset/xml.rs::flags_text` currently emits decimal text when bits outside 1/2/4 are set. The C# oracle records `XmlSerializer` rejection for those values. TB1's Ferroan column is corrected; no asset behavior changed in Q10b. Proposed follow-up: verify the full asset route against a C# capture and resolve this discrepancy in the asset pilot gaps task (P1.A1), with the required compatibility decision.

- 2026-11-18, resumed Q10 (queued as Q10c, P0.10j; not a C# bug, see Q10c): Tranquillity `LLClientView.SendInstantMessage` at `8583218` appends four zero bytes beyond the pinned `ImprovedInstantMessage` template. Ferrobot receives the IM, but Ferroan's generated re-encoder drops the suffix and the mandatory lossless capture check fails deterministically on the preserved packet. Recorded as TB4 in [TRANQUILLITY_BUGS.md](docs/TRANQUILLITY_BUGS.md); proposed protocol/capture prerequisite is under Q10 above. No external issue was filed and no C# or Ferrobot checkout was changed.

- 2026-11-18, Q10 (queued as Q10a, P2.0d2): P2.0d's passing bot login check proves handshake and seed completion but not viewer arrival. The second client's stricter readiness exposed the missing `AgentMovementComplete`; see the Q10 blocker and proposed prerequisite above. Existing viewer gates remain pending.

- 2026-11-18, revisiting #26 during Q7 (no item: Q7 fixed the proxy; record a recurrence here): the final documentation revision's live CI failed twice in the unchanged C# grid baseline, first on presence verification and then on a texture upload (HTTP 500). [Attempt 1](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/1), [attempt 2](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/2), and the unchanged [passing attempt 3](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/3) are recorded in #26. Q7 confirms that the capture proxy advertised persistent HTTP connections but closed after one exchange. Q7 corrects that proxy behavior along with its timeout; the historical failures' precise cause is unproven.

- 2026-11-18, Q7 documentation review (queued as Q10b, P0.10i part 3): the last paragraph of `crates/ferroan-capture/README.md` still says no session capture is committed, although `fixtures/captures/session.cap` exists. Several oracle/session paragraphs in `docs/TESTING.md` describe CI before the live job was added. Proposed fix: a documentation consistency pass against the current workflows; these historical claims were not rewritten as part of Q7.

- 2026-11-18, Q8 reliability extraction (queued as Q10b, P0.10i part 2): the bot's existing `Circuit::take_dropped` queue accumulates exhausted sequence IDs until drained, and `Session` currently does not drain it. Q8 shares retry accounting while preserving this caller behavior. Proposed follow-up: bound or consume the bot's dropped-ID notification queue and test a long loss-heavy session.

- 2026-11-18, Q9 source review (queued as Q10b, P0.10i part 1): the shared LLSD XML decoder predates this item and accepts `base16`, unpadded base64, and malformed binary text (invalid base64 becomes empty bytes). Pinned libopenmetaverse `OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs` rejects unsupported encodings and malformed base64. For example, `<llsd><array><binary>!</binary></array></llsd>` reaches the Rust seed as an array but C# returns 400. Proposed follow-up: capture malformed LLSD cases in the oracle and align `crates/ferroan-llsd/src/xml_llsd.rs::decode_binary`, checking all callers. Q9 changes seed error mapping and name conversion without rewriting the shared parser.

- 2026-11-18, Q10f: `ControlPlaneAccess.AddTrustedHost` at `8583218` treats some host:port and bare IPv6 strings as opaque absolute URIs, then resolves their empty Host to machine-local addresses. A .NET 10 oracle confirmed the parser and empty-host DNS behavior. Recorded as TB5 in `docs/TRANQUILLITY_BUGS.md`; D20 records Ferroan's explicit startup rejection. Proposed upstream fix: reject empty URI hosts or parse IP/host-port forms before URI interpretation. No upstream issue was filed.

- 2026-11-18, Q10f: shared `ferroan_http::post_form` parses XML without checking HTTP success, whereas pinned `WebUtil.SynchronousRestFormsRequester` calls `EnsureSuccessStatusCode`. The new region Presence/GridUser clients and their offline-state probe check status locally, so a 500 response containing plausible XML cannot authorize admission or prove logout. Proposed separate HTTP follow-up: audit shared-client callers, capture failed-status behavior, and add status validation there. Existing unrelated callers were not changed.

- 2026-11-18, Q10f detector review (**resolved by Q10g, #48**): the Rust region still lacks the root-arrival service updates `PresenceDetector.DoOnMakeRootAgent` -> `ReportAgent(session, region)` and `ActivityDetector.DoOnMakeRootAgent` -> `SetLastPosition` at `8583218`. Q10f adds the specified admission verification and termination notifications, not these arrival hooks. Proposed follow-up: report the accepted root arrival to both services and verify the live Presence RegionID and GridUser last location while the bot is online. The current offline/second-login proof does not cover active location reporting.

## Waiting on the maintainer

- Nothing. G0 passed on 18 November 2026 at `c85783e` (see its row in TASKS.md).
