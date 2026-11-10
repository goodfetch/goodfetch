# V1 is delivered in four milestones, each ending at an owner test

ADR 0037 orders the specs of V1's fourteen areas by the contracts they consume, but it orders nothing about delivery. Building in spec order would leave the owner with nothing to try until the Console, area 12, is built. V1 is therefore delivered in four milestones, each a GitHub milestone carried by every ticket. Each of the first three ends at a test the owner runs by hand before the next starts:

1. **M1 Core: a fully working extraction and crawl framework.** The engine on one machine, the Operation catalog over JSON-RPC, the CLI generated from it, a single-user local Console, crawling over HTTP and with browser rendering, `standard` and `identified` access, the Frontier and extraction, the Dataset, every Export format and the results stream.
2. **M2 Client surfaces:** the TypeScript, Python and Rust SDKs, MCP and agent skills.
3. **M3 Browser automation:** actions and the guard, Firefox and WebKit, stealth and the Challenge solver, the Live view with takeover, the Browser host and Session sources.
4. **M4 Everything else**, ending at V1's release gates.

Every V1 capability stays in V1 (ADR 0037); the milestones order the work, they exclude nothing. Decided with the owner while specifying the Engine runtime ([delivery order](https://github.com/goodfetch/goodfetch/issues/213#issuecomment-6145231664)).

## Considered Options

- **Delivery in ADR 0037's spec order**: rejected. It gives the owner nothing to test hands-on until the Console, area 12, is built.
- **The Console specified right after the engine**, amending ADR 0037's order: rejected. The Console would be designed before the accounts, Datasets and Schedules it shows (areas 4, 6 and 10), and reworked when they arrive.
- **Every spec written before any code**: rejected. Nothing would run until every area is specified.
- **An HTTP-only first milestone**: rejected by the owner, who asked for a fully working crawl framework in M1; M1 therefore renders pages and leaves only browser automation to M3.
- **The SDKs first, before the Console**: rejected by the owner, who wants to test the engine through the CLI and the Console first.
  > Note (owner's decisions of 2026-11-10, in the grilling of decision G-01 of the consolidated review, Q1, Q5, Q9 and Q15, answered "Yes, all four", "Yes, all ten" and "Yes, and confirmed"): this rejection no longer orders the work. The Console it waited for is taken out of both Products (ADR 0137), and a first TypeScript `Page` needs no engine change: the operations it calls are in the catalog, generated and served (`crates/goodfetch-protocol/src/entries.rs:179-218`, `sdks/typescript/collie-client/src/generated/operations.ts:29-81`, `crates/goodfetch-page/src/pages.rs:397-766`, read at `094c96b2d`). A first program is therefore built by relay while the refactor runs, TypeScript leading each concern; Python's and Rust's parts follow as tickets of their own, neither waiting for the other, and are built in the Features step. The crawl with a handler waits for no refactor ticket but [#2167](https://github.com/goodfetch/goodfetch/issues/2167). `docs/agents/rollout-order.md` holds the tickets. What each milestone holds is unchanged.

## Consequences

**What each milestone holds.**
- **M1:**
  - the engine on one machine (areas 3 and 4): the Attached engine and the Daemon, run state and resume, leases, the governor, the Usage ledger, shared Politeness on one machine, settings, and the ops listener with the foreground Daemon (#228) in its chain;
  - the security and egress controls of area 4, built before the fetch tiers because ADR 0017's egress policy wraps every fetch;
  - the Operation catalog over JSON-RPC as the only API, and the CLI generated from it;
  - the single-user local Console of ADR 0031: Runs, a run's drawer, Datasets, Engine & settings and "Needs you";
  - from area 5: the HTTP tiers (reqwest and wreq's TLS impersonation), browser rendering for crawling (the CDP driver's navigation, waits and extraction, the per-route tier choice of ADR 0020, screenshots and PDF, the browser pool), proxies, Sessions and the Session pool, block causes and escalation, and Politeness defaults (ADR 0021);
  - from area 6: the Frontier, extraction in every format, the Dataset, every Export format and the results stream;
  - from areas 1 and 2: the build and test gates (#132–#137), the full platform matrix (#145), `xtask size` (#201), the local Proving ground (#158–#167) and the conformance harness (#169, #170, #172).
    > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): both Products' executables are built and tested from M1. The engine tracer builds both, Foxhound's starting from Collie's composition, and these gates cover both.
- **M2:** the three SDKs, MCP and agent skills.
- **M3:** the guard and the rest of ADR 0019's page API, Firefox and WebKit through Playwright's driver, the Browser host (ADR 0022), Session sources and Borrowed browsers (ADR 0056), stealth and the Challenge solver (area 8), and the Live view with takeover.
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): M3 is Collie's milestone, whose owner test is the first to try Collie's page automation. Collie's AI actions come with M4, and both Products are first published there.
- **M4:**
  - Sinks, Schedules, Watches and Recipes;
  - AI and agents with Plans, Plugins, Script Hooks and the Script sandbox;
  - framework adapters, Integrations, Notifications and the HTTP listener of ADR 0027;
  - Daemon groups, Partitioned crawls, the Politeness store, deployed hosts, and Exposed Daemons with accounts and passkeys;
  - the release pipeline, signing and publishing, the docs site, the online Proving ground and the benchmark (from areas 1 and 2: #138–#144, #146–#150, #168, #173–#189, #202, #203, #205–#209, #211);
  - four area 1–2 tickets first placed in M1 that wait on M4 work: V8 built from source (#210) and the merge queue's size increase check (#204), both behind the release pipeline, and the process Plugin conformance target (#171) with `goodfetch plugins test` (#212), which serve Plugins. M1 needs none of them: V8 serves only the Script sandbox, and the binary links it from M4 on. **Changes ADRs 0032 and 0057:** the size estimate posted on pull requests and the merge queue's size checks (the increase check, the crate's ceiling and the OCI tags' check) start in M4; M1 only builds `xtask size` itself (#201).

  The governor keeps its provisional defaults until #178's concurrency search, in M4, measures them.

**Gates.**
- Each of M1, M2 and M3 ends with a `ready-for-human` testing issue listing how to install from a local build on macOS and Linux. It is ready once it is the only open issue of its milestone; the frontier skips `ready-for-human` issues, so no agent takes it, and no ticket of the next milestone starts until the owner closes it.
  > Amended by [ADR 0063](0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md): it is ready only when every automated variant of its milestone is also `proved` on `main`.
  >
  > Amended by [ADR 0072](0072-every-feature-in-a-generated-inventory-has-a-passing-test-and-code-coverage-and-mutants-are-signals.md): it is also ready only when every generated Feature, and every declared Feature due at its milestone, is tested or excepted on `main`'s latest Release suite run.
  >
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): each testing issue installs both Products' executables from a local build and lists the Journeys of both Products due at its milestone.
  >
  > Amended by [ADR 0108](0108-while-the-repository-is-private-the-release-suite-runs-its-macos-cells-on-the-owners-machine-and-its-hosted-cells-under-a-ceiling.md): while the repository is private, `main`'s latest Release suite run is `main`'s latest commit whose cells have all run, across its dispatches.
  >
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): the testing issue is a checkpoint, not a start gate. Tickets of the next milestone start while it is open; its readiness rules stand.
- M4 ends at the release gates of ADRs 0030 and 0032.
- Nothing is published before M4, on any channel; the owner tests from local builds. **Changes ADRs 0030, 0032 and 0057:** the nightly channel (npm `nightly`, GitHub pre-releases, the nightly Python index, the OCI `nightly` tag) and the `0.y` previews start in M4.
- Between gates, delivery stays autonomous under AGENTS.md.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the autonomous delivery mode is retired ([#1560](https://github.com/goodfetch/goodfetch/pull/1560)); agent work follows the skills as written, and the owner merges.

**Tracking** (adds to ADR 0037).
- Every ticket carries one of the four milestones, and every later spec's `/to-tickets` assigns one to each ticket it creates.
- A ticket's milestone is never later than the earliest milestone of any ticket it blocks, directly or transitively, so a gate never holds back a ticket that an earlier milestone needs. `/to-tickets` checks this when it assigns milestones, and moves a blocker earlier when it finds one later than what it blocks.
  > Amended by [ADR 0131](0131-a-cross-spec-milestone-inversion-moves-the-dependent-later-unless-pulling-the-blocker-earlier-moves-fewer-tickets.md): this holds within one spec. Across specs, the dependent moves later or is split, unless pulling the blocker earlier changes the milestone of fewer tickets, counted transitively; a tie moves the dependent.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): `/to-tickets` has no milestone step. The invariant holds, and `docs/agents/issue-tracker.md` states it.
- A milestone is done when it has no open ticket; the earliest milestone with an open ticket is the current one. Each of M1, M2 and M3 holds its testing issue, created with the milestone.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): no rule picks work by the current milestone. A milestone is done when it has no open ticket, and its testing issue is then ready.
- The tracker's frontier query takes the earliest open milestone first, then map order (`docs/agents/issue-tracker.md`).
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): each `/implement-spec` run is one whole spec, whatever milestones its tickets carry, and its frontier is the spec's open tickets with no open blocker and no assignee. Milestones neither scope nor order a run.
- Specs keep ADR 0037's order, and each area spans the milestones its tickets fall in.
  > Amended on 2026-11-10, in the grilling of decision G-03 of the consolidated review, under the owner's instruction to build « engine sdk » in the standard order, confirmed as "engine, then SDK, then the rest, nothing removed" (« Yes, that's it »): the order of the Features' specs is the list in `docs/agents/rollout-order.md`, whose lines are the session's choices awaiting the owner's reading. Every V1 capability stays in V1.
- This ADR decides delivery order, not a capability, so no row of ADR 0037's area table lists it, and ADR 0037's rule on area rows does not apply.
