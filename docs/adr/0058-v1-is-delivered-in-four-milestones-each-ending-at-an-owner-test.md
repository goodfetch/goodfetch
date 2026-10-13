# V1 is delivered in four milestones, each ending at an owner test

ADR 0037 orders the specs of V1's fourteen areas by the contracts they consume, but it orders nothing about delivery. Building in spec order would leave the owner with nothing to try until the Console, area 12, is built. V1 is therefore delivered in four milestones, each a GitHub milestone carried by every ticket. Each of the first three ends at a test the owner runs by hand before the next starts:

1. **M1 Core: a fully working scrape and crawl framework.** The engine on one machine, the Operation catalog over JSON-RPC, the CLI generated from it, a single-user local Console, crawling over HTTP and with browser rendering, `standard` and `identified` access, the Frontier and extraction, the Dataset, every Export format and the results stream.
2. **M2 Client surfaces:** the TypeScript, Python and Rust SDKs, MCP and agent skills.
3. **M3 Browser automation:** actions and the guard, Firefox and WebKit, stealth and the Challenge solver, the Live view with takeover, the Browser host and Session sources.
4. **M4 Everything else**, ending at V1's release gates.

Every V1 capability stays in V1 (ADR 0037); the milestones order the work, they exclude nothing. Decided with the owner while specifying the Engine runtime ([delivery order](https://github.com/Jbmanllr/automan/issues/213#issuecomment-6145231664)).

## Considered Options

- **Delivery in ADR 0037's spec order**: rejected. It gives the owner nothing to test hands-on until area 12, the last area of the engine's surfaces, is built.
- **The Console specified right after the engine**, amending ADR 0037's order: rejected. The Console would be designed before the accounts, Datasets and Schedules it shows (areas 4, 6 and 10), and reworked when they arrive.
- **Every spec written before any code**: rejected. Nothing would run for months, while areas 1 to 3 are already ticketed.
- **An HTTP-only first milestone**: rejected by the owner. On today's web a crawler that cannot render JavaScript returns empty pages, so M1 renders pages and leaves only browser automation to M3.
- **The SDKs first, before the Console**: rejected by the owner, who wants to test the engine through the CLI and the Console first.

## Consequences

**What each milestone holds.**
- **M1:**
  - the engine on one machine (areas 3 and 4): the Attached engine and the Daemon, run state and resume, leases and epochs, admission and the governor, the Usage ledger, shared Politeness on one machine, settings, the foreground Daemon and the ops listener;
  - the security and egress controls of area 4, built before the fetch tiers because ADR 0017's egress policy wraps every fetch;
  - the Operation catalog over JSON-RPC as the only API, and the CLI generated from it;
  - the single-user local Console of ADR 0031: Runs, a run's drawer, Datasets, Engine & settings and "Needs you";
  - from area 5: the HTTP tiers (reqwest and wreq's TLS impersonation), browser rendering for crawling (the CDP driver's navigation, waits and extraction, the per-route tier choice of ADR 0020, screenshots and PDF, the browser pool), proxies, Sessions and the Session pool, block causes and escalation, and Politeness defaults (ADR 0021);
  - from area 6: the Frontier, extraction in every format, the Dataset, every Export format and the results stream;
  - from areas 1 and 2: the build and test gates, the full platform matrix, V8 built from source, the Size budget checks, the Proving ground and the conformance harness.
- **M2:** the three SDKs, MCP and agent skills.
- **M3:** the rest of ADR 0019's page API, Firefox and WebKit through Playwright's driver, the Browser host (ADR 0022), Session sources and Borrowed browsers (ADR 0056), stealth and the Challenge solver (area 8), and the Live view with takeover.
- **M4:**
  - Sinks, Schedules, Watches and Recipes;
  - AI and agents with Plans, Plugins, Script Hooks and the Script sandbox;
  - framework adapters, Integrations, Notifications and the HTTP listener of ADR 0027;
  - Daemon groups, Partitioned crawls, the Politeness store, deployed hosts, and Exposed Daemons with accounts and passkeys;
  - the release pipeline, signing and publishing, the docs site, the online Proving ground and the benchmark.

  The governor keeps its provisional defaults until M4's concurrency search measures them (ADR 0040).

**Gates.**
- Each of M1, M2 and M3 ends with a `ready-for-human` issue listing how to install from a local build on macOS and Linux and the scenarios to try. No ticket of the next milestone starts until the owner closes it.
- M4 ends at the release gates of ADRs 0030 and 0032.
- Nothing is published to a registry before M4; the owner tests from local builds.
- Between gates, delivery stays autonomous under AGENTS.md.

**Tracking** (adds to ADR 0037).
- Every ticket carries one of the four milestones, and every later spec's `/to-tickets` assigns one to each ticket it creates.
- The frontier takes the earliest open milestone first, then map order (`docs/agents/issue-tracker.md`).
- Specs keep ADR 0037's order, and each area spans the milestones its tickets fall in.
- This ADR decides delivery order, not a capability, so no row of ADR 0037's area table lists it.
