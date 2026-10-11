# V1 scope counts capabilities, is tracked in one matrix, and goes to `/to-spec` in dependency order

V1 is complete when it covers the core capabilities of Crawlee, Spider-rs and Stagehand v4, and each of the 24 ranked pains of `rnd/research/market/pains-ranking.md`, unless an item is excluded with a reason. Parity counts **capabilities**, not call signatures or who operates them. A row that a reference offers only through its vendor's paid cloud counts when the public framework delivers it self-hosted:
- a gateway or local model server by URL (ADR 0012);
- committed Plans as the AI cache (ADR 0009);
- the Challenge solver (ADR 0035);
- the user's own proxies (ADR 0021);
- Run traces and the local Console (ADR 0031);
- a Browser host (ADR 0022).

The managed operation behind such a row (a pooled residential IP fleet, keyless model billing, multi-tenant hosting) is Goodfetch Cloud's, as ADR 0018 sells operations, not capabilities. Crawlee's Stagehand integrations are pinned to Stagehand v3, and neither can move to v4 without a rewrite (v4 refuses Playwright pages). The v3 capabilities their users reach therefore count as rows V3-01 to V3-05:
- AI calls on the crawler's page;
- a local AI cache;
- a custom model client;
- an agent loop;
- local or remote by configuration.

They are covered by Goodfetch's own mechanisms and a Crawlee+Stagehand migration guide, with no v3 compatibility layer.

An item is **covered** when either:
- an ADR or a closed ticket's resolution decides it (`decided`); or
- a cited decision puts it in V1 and only API shape, naming or a default remain for `/to-spec` (`spec`).

Only a real trade-off still unmade is `open`, and it becomes a grilling ticket on the map. A decision recorded only in a ticket's resolution stays there and is cited from the matrix, unless it contradicts an ADR's text, which is then amended. The matrix is [`0037-v1-scope-matrix.md`](0037-v1-scope-matrix.md), with one status per row (`decided`, `spec`, `excluded`, `open → #N`). A pull request that resolves a ticket changes that ticket's rows, and its fresh review checks it. The map is done when no row reads `open` and the map has no open ticket, since a ticket such as platform code signing holds no matrix row. Decided in [V1 scope sign-off: parity and pain coverage](https://github.com/Jbmanllr/automan/issues/81).

## Order of `/to-spec`

One spec per area. An area goes to `/to-spec` only after every area whose contracts it consumes, and after every ticket that holds one of its `open` rows in the matrix; the "Waits on" column is derived from those rows, and a pull request that changes them updates it:

| # | Area | ADRs | Waits on |
|---:|---|---|---|
| 1 | Repository, CI and release | 0018, 0030, 0032, 0034 | [Platform code signing for binaries and the Goodfetch Chromium](https://github.com/Jbmanllr/automan/issues/121) |
| 2 | Proving ground and conformance | 0025 |  |
| 3 | Engine runtime | 0002–0004, 0024, 0028 | [Engine resource governance](https://github.com/Jbmanllr/automan/issues/115), [Scaling past one engine](https://github.com/Jbmanllr/automan/issues/116), [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |
| 4 | Security and egress | 0015–0017 | [Network defaults](https://github.com/Jbmanllr/automan/issues/118) |
| 5 | Site access | 0019–0023 | [Bringing an existing login or browser](https://github.com/Jbmanllr/automan/issues/117), [Network defaults](https://github.com/Jbmanllr/automan/issues/118), [Built-in page clean-up](https://github.com/Jbmanllr/automan/issues/119), [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |
| 6 | Collection | 0005–0008, 0026 | [Engine resource governance](https://github.com/Jbmanllr/automan/issues/115), [Network defaults](https://github.com/Jbmanllr/automan/issues/118) |
| 7 | Plugin protocol | 0033 |  |
| 8 | Stealth and Challenge solving | 0035 |  |
| 9 | AI and agent | 0009–0014 |  |
| 10 | Scheduling | 0036 | [Engine resource governance](https://github.com/Jbmanllr/automan/issues/115), [Scaling past one engine](https://github.com/Jbmanllr/automan/issues/116), [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |
| 11 | Client surfaces (SDKs, CLI, MCP, skills, adapters, Integrations, Notifications) | 0011, 0024, 0027 | [Bringing an existing login or browser](https://github.com/Jbmanllr/automan/issues/117), [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |
| 12 | Console | 0029, 0031 |  |
| 13 | Self-hosted deployment and operations | 0024 | [Scaling past one engine](https://github.com/Jbmanllr/automan/issues/116), [Bringing an existing login or browser](https://github.com/Jbmanllr/automan/issues/117), [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |
| 14 | Cloud-readiness contracts | 0001 | [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120) |

The Proving ground comes second because it is the oracle every later area's conformance suite runs against. Security precedes Site access because ADR 0017's egress policy wraps every fetch. Scheduling precedes Client surfaces and the Console, which expose its `schedules.*` operations (ADR 0036).

## Considered Options

- **Cloud-only rows excluded from parity:** rejected. Goodfetch would look weaker than Stagehand on what users pay Browserbase for, while every such row but the keyless gateway already had a self-hosted equivalent.
- **Managed versions of cloud-only rows in V1:** rejected. They are Goodfetch Cloud operations, a separate map.
- **Parity on Stagehand v4 only:** rejected. It would drop the capabilities that Crawlee+Stagehand users, stuck on v3, look for elsewhere.
- **A Stagehand v3 compatibility layer:** rejected as over-engineering. It freezes a third-party API that fights the Operation catalog (ADR 0011), for an integration with about 3,300 weekly npm downloads against about 1.7 million for Stagehand.
- **Every API detail as its own decision:** rejected. It would add about 40 tickets to settle what `/to-spec` settles better with the whole area in view.
- **A final re-audit ticket after the new tickets:** rejected. A matrix kept current by each ticket makes the end condition checkable without a second pass.
- **Ordering specs by user value** (AI first) **or one spec for all:** rejected. `/to-issues` needs blockers built before what they block.

## Consequences

- The parity list is the research of 2026-10-10 (Crawlee JS 3.18.2 and 4.0.0-rc.1, Crawlee Python 1.10.4, Spider 2.53.9, Stagehand 4.2.0), kept on the unmerged `research/parity-list` branch. The matrix carries its row IDs, so specs cite rows as `ADR 0037 / AI-12`.
- Specs carry the notes the matrix attaches to rows, for example that clicks made to discover links are not Effects, and that the page API dismisses dialogs by default because accepting one can confirm an Effect.
