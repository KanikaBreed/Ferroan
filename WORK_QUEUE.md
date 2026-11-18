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
| Q10c | P0.10j: trailing blocks Tranquillity sends beyond libopenmetaverse's template (`ImprovedInstantMessage` and seven more) | READY |
| Q10 | P0.10h: Ferrobot as the second test client in the live harness | READY after Q10c |
| Q10b | P0.10i: follow-ups from Q7 to Q9 (LLSD binary, dropped-ID queue, documentation) | READY after Q10a |
| Q11 | P0.11: container images and the example compose grid | READY after Q10b |
| Q12 | P0.12: monitoring: Tranquillity's status endpoints, plus metrics and health checks | READY after Q11 |
| Q13 | P0.13: logging hooks: stdout, `LOGDIR` files, JSON, OpenTelemetry export with a collector adapter, and Tranquillity's remote console | READY after Q12 |
| Q14 | P2.0e: terrain from the C# region store | READY after Q13 |
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

Q10 BLOCKED again (18 November 2026, #36, `feat/ferrobot-harness-resume`): the resumed harness reaches the C# standalone IM check, then the mandatory capture check fails: `record 442 (udp out) does not re-encode: the encoded bytes differ`. Raw UDP record 427 is `ImprovedInstantMessage`, 191 bytes; re-encoding yields 187 identical prefix bytes and drops four trailing zero bytes. At the pin, `LLClientView.SendInstantMessage` appends that undocumented `EstateBlock` after `BinaryBucket`, while the pinned message template ends at `BinaryBucket` (TB4). This is a shared protocol/capture prerequisite, not a Ferrobot bug. No packet was filtered or canonicalized to hide the discrepancy; no retry is claimed. Proposed fix: the manager adds a prerequisite deciding and implementing bounded preservation of this observed suffix in lossless capture/replay (with a C# fixture), or takes an upstream fix through a pin move and regenerates affected fixtures. Then resume Q10's implementation preserved at `789fe59` / local branch `wip/ferrobot-harness-arrival-ready`, with `.scratch/q10-arrival-ready.patch` in the existing worktree. The initial Rust-region diagnostic passed arrival, 120 seconds and SIGTERM logout after Q10a; full Q10 acceptance remains pending. Q10b can proceed independently; Q11 waits on Q10.

Q10c was added on 18 November 2026 from the second Q10 blocker (#36). The four bytes are `ImprovedInstantMessage`'s `EstateBlock`, which Linden's viewer template defines and libopenmetaverse's lacks, so TB4 is a gap in Ferroan's template, not a Tranquillity bug. Q10 then resumes from `789fe59`.

Q20 to Q39 were added on 18 November 2026: all of Phase 1, starting with Tranquillity's three asset backends. They come after P2.0j, so they run while the maintainer does G2 (rule 7). G1 follows Q39.

Q18: size it from the C# first, and set it to `BLOCKED` with a proposed split if it is more than two weeks of work.

Each item's scope and acceptance is its row in TASKS.md. The rows were revised on 18 November 2026: the first Ferroan region logs in through C# Robust and is not a login server, and the harness gains a grid mode first. Work from the rows as they stand on `main`, not from the earlier split in #18.

## Found while working

- 2026-11-18, resumed Q10 (queued as Q10c, P0.10j; not a C# bug, see Q10c): Tranquillity `LLClientView.SendInstantMessage` at `8583218` appends four zero bytes beyond the pinned `ImprovedInstantMessage` template. Ferrobot receives the IM, but Ferroan's generated re-encoder drops the suffix and the mandatory lossless capture check fails deterministically on the preserved packet. Recorded as TB4 in [TRANQUILLITY_BUGS.md](docs/TRANQUILLITY_BUGS.md); proposed protocol/capture prerequisite is under Q10 above. No external issue was filed and no C# or Ferrobot checkout was changed.

- 2026-11-18, Q10 (queued as Q10a, P2.0d2): P2.0d's passing bot login check proves handshake and seed completion but not viewer arrival. The second client's stricter readiness exposed the missing `AgentMovementComplete`; see the Q10 blocker and proposed prerequisite above. Existing viewer gates remain pending.

- 2026-11-18, revisiting #26 during Q7 (no item: Q7 fixed the proxy; record a recurrence here): the final documentation revision's live CI failed twice in the unchanged C# grid baseline, first on presence verification and then on a texture upload (HTTP 500). [Attempt 1](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/1), [attempt 2](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/2), and the unchanged [passing attempt 3](https://github.com/KanikaBreed/Ferroan/actions/runs/44263666135/attempts/3) are recorded in #26. Q7 confirms that the capture proxy advertised persistent HTTP connections but closed after one exchange. Q7 corrects that proxy behavior along with its timeout; the historical failures' precise cause is unproven.

- 2026-11-18, Q7 documentation review (queued as Q10b, P0.10i part 3): the last paragraph of `crates/ferroan-capture/README.md` still says no session capture is committed, although `fixtures/captures/session.cap` exists. Several oracle/session paragraphs in `docs/TESTING.md` describe CI before the live job was added. Proposed fix: a documentation consistency pass against the current workflows; these historical claims were not rewritten as part of Q7.

- 2026-11-18, Q8 reliability extraction (queued as Q10b, P0.10i part 2): the bot's existing `Circuit::take_dropped` queue accumulates exhausted sequence IDs until drained, and `Session` currently does not drain it. Q8 shares retry accounting while preserving this caller behavior. Proposed follow-up: bound or consume the bot's dropped-ID notification queue and test a long loss-heavy session.

- 2026-11-18, Q9 source review (queued as Q10b, P0.10i part 1): the shared LLSD XML decoder predates this item and accepts `base16`, unpadded base64, and malformed binary text (invalid base64 becomes empty bytes). Pinned libopenmetaverse `OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs` rejects unsupported encodings and malformed base64. For example, `<llsd><array><binary>!</binary></array></llsd>` reaches the Rust seed as an array but C# returns 400. Proposed follow-up: capture malformed LLSD cases in the oracle and align `crates/ferroan-llsd/src/xml_llsd.rs::decode_binary`, checking all callers. Q9 changes seed error mapping and name conversion without rewriting the shared parser.

## Waiting on the maintainer

- Nothing. G0 passed on 18 November 2026 at `c85783e` (see its row in TASKS.md).
