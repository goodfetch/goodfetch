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

Only a real trade-off still unmade is `open`, and it becomes a grilling ticket on the map. A decision recorded only in a ticket's resolution stays there and is cited from the matrix, unless it contradicts an ADR's text, which is then amended. The matrix is [`0037-v1-scope-matrix.md`](0037-v1-scope-matrix.md), with one status per row (`decided`, `spec`, `excluded`, `open → #N`). A pull request that resolves a ticket changes that ticket's rows, and its fresh review checks it. The map is done when no row reads `open` and the map has no open ticket, since a ticket such as platform code signing holds no matrix row. Decided in [V1 scope sign-off: parity and pain coverage](https://github.com/goodfetch/goodfetch/issues/81).

> Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; this check is a review standard the `code-review` skill applies, and the owner sees its result before merging.

> Amended on 2026-11-11 by the owner, in the grilling of decision G-19 of the consolidated review (Q1, Q2 and Q3 answered « Yes, all three »; Q28 and Q33 answered « Yes, all six »; each on the grilling's recommended option, quoted as put; the owner did not read this note's other words):
> - **The compared tools.** Q2: "one list of 21, which the table, the benchmark and `just compare` all point to; a tool joins or leaves by the owner's word", shown with "the scope's reference stops being "Crawlee, Spider and Stagehand" and becomes "the compared tools"". Q28 made it "22, chromiumoxide included". The list is the matrix's [Compared tools](0037-v1-scope-matrix.md#compared-tools) table, and parity is held against it, no longer against Crawlee, Spider-rs and Stagehand v4 alone.
> - **Every capability gets a row.** Q1: "every capability a compared tool offers gets a row with one of three states: held (a decision or a ticket already covers it), to decide (it comes to the owner), refused (the owner's word, with the reason kept on the row)", with the four details shown: "A capability a tool sells only in its paid cloud still gets a row; the row says so." "A capability already decided that has no row gets its row too." "When an existing row already covers the need, no new row is added; that row's note names the tool." "A session never refuses a capability on its own, and "it is a lot to build" is never a reason."
> - **The re-read.** Q3: "each time a line of Features is next, before its spec is written, every listed tool is re-read from the version last read to its latest release, and one tool is re-read when its pinned version is moved for a comparison run; new capabilities arrive as rows "to decide" and come to the owner in one batch; the list keeps, per tool, the version and date last read". Q33 put its mechanism: "How a line is cut" gains a first step, the re-read, whose new capabilities are rows marked open under one grilling ticket, and the spec waits for the owner's answers on it; "A pull request that marks a row `excluded` quotes the owner's words and links where they were said; the code-review standard checks it." The step is in "How a line is cut" of [`docs/agents/rollout-order.md`](../agents/rollout-order.md).
>
> Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading): held is `decided` or `spec`, to decide is `open → #N`, and refused is `excluded`, so the batch of a re-read is one grilling ticket, as the paragraph above already makes of an `open` row. Which existing row covers a need, and so gets a note rather than a row, is the grilling session's reading. The capabilities an earlier decision refuses by name are `excluded` citing it, and five that rested on a reading went back to the owner, who answered them (Q30, Q31). ADR 0025's own list of tools (:51) does not point to the table yet.

## Order of `/to-spec`

One spec per area. An area goes to `/to-spec` only after every area whose contracts it consumes, except between areas that use each other's contracts (below), and after every ticket that holds one of its `open` rows in the matrix; the "Waits on" column is derived from those rows, and a pull request that changes them updates it:

| # | Area | ADRs | Waits on |
|---:|---|---|---|
| 1 | Repository, CI and release | 0018, 0030, 0032, 0034, 0039, 0049, 0057, 0060, 0061, 0062, 0064, 0065, 0066, 0067, 0068, 0069, 0071, 0072, 0073, 0074, 0075, 0076, 0084, 0087, 0088, 0090, 0094, 0095, 0097, 0099, 0108, 0124, 0125, 0126 |  |
| 2 | Proving ground and conformance | 0025, 0063, 0065, 0067, 0069, 0071, 0072, 0073, 0074, 0075, 0094, 0119, 0121, 0123 |  |
| 3 | Engine runtime | 0002–0004, 0024, 0028, 0040, 0041, 0049–0051, 0053–0057, 0060, 0066, 0071, 0073, 0075, 0080, 0082, 0083, 0084, 0086, 0087, 0088, 0093, 0094, 0095, 0096, 0098, 0099, 0101, 0102, 0103, 0104, 0106, 0107, 0113, 0115, 0116, 0117, 0118, 0122, 0127, 0128, 0129 |  |
| 4 | Security and egress | 0015–0017, 0042–0044, 0049–0051, 0055, 0056, 0066, 0067, 0086, 0089, 0092, 0093, 0120, 0121, 0124 |  |
| 5 | Site access | 0019–0023, 0045, 0047–0049, 0052, 0053, 0056, 0066, 0080, 0081, 0083, 0085, 0086, 0087, 0088, 0089, 0090, 0092, 0093, 0096, 0100, 0103, 0104, 0110, 0113, 0114, 0115, 0117, 0118, 0119, 0121, 0127, 0128, 0129 |  |
| 6 | Collection | 0005–0008, 0026, 0040, 0046, 0054, 0056, 0066, 0073, 0082, 0086, 0087, 0088, 0093, 0102, 0103, 0104, 0105, 0106, 0107, 0110, 0116, 0119, 0120, 0121, 0122, 0123, 0127, 0129 |  |
| 7 | Plugin protocol | 0033, 0049, 0056, 0066, 0088, 0101, 0104, 0129 |  |
| 8 | Stealth and Challenge solving | 0035, 0056, 0074, 0083, 0088, 0092, 0121 |  |
| 9 | AI and agent | 0009–0014, 0051, 0056, 0065, 0066, 0082, 0088, 0091, 0092, 0100, 0101, 0106, 0107, 0114, 0117, 0121, 0123, 0125, 0127, 0129 |  |
| 10 | Scheduling | 0036, 0041, 0055, 0066, 0084, 0088, 0098, 0103 |  |
| 11 | Client surfaces (SDKs, CLI, MCP, skills, adapters, Integrations, Notifications) | 0011, 0024, 0027, 0049, 0055, 0056, 0065, 0069, 0070, 0071, 0073, 0077, 0078, 0079, 0080, 0081, 0082, 0083, 0085, 0086, 0087, 0088, 0089, 0090, 0091, 0092, 0093, 0094, 0095, 0097, 0098, 0099, 0100, 0101, 0102, 0103, 0104, 0105, 0106, 0107, 0109, 0110, 0111, 0112, 0113, 0114, 0115, 0116, 0117, 0118, 0119, 0120, 0121 |  |
| 12 | Console | 0029, 0031, 0049, 0055–0057, 0068, 0071, 0073, 0084, 0087, 0088, 0092, 0094, 0096, 0098, 0099, 0102, 0106, 0113, 0122, 0130 |  |
| 13 | Self-hosted deployment and operations | 0024, 0049, 0050, 0053, 0055, 0056, 0066, 0088, 0090, 0094, 0096, 0122 |  |
| 14 | Cloud-readiness contracts | 0001, 0049, 0056, 0068, 0096 |  |

> Amended on 2026-11-10, in the grilling of decision G-03 of the consolidated review, by the session's choice under the owner's instruction to « build the thing as best pratice would build it, engine sdk » (awaiting the owner's reading): a spec of Features is cut by what can be built and proved alone, when its line of `docs/agents/rollout-order.md` is next, and no longer by area. On that day 46 open specs blocked one another in one loop holding 1,137 open tickets, and 1,804 of 3,284 open blocker edges crossed a spec boundary, so no order of the area specs let one run whole. The matrix's "Spec area" column keeps naming the area a row belongs to; no row changes status.

Every area also follows ADR 0038, which sets the contracts and code conventions, ADR 0126, which sets how a crate's modules divide their concerns, ADR 0127, which sets where a failure from a party Goodfetch did not write becomes a typed error, and ADR 0129, which sets where the behaviour that several implementations of a contract share lives. Each area's `/to-spec` also reads every ADR that amends ADR 0038, 0126, 0127 or 0129 or one of the ADRs listed in its row, however the amendment is worded ("Changes ADR", "adds to ADR", "amends", "extends", "narrows", "replaces", "an exception to ADR"), so a later decision reaches every area whose ADR it amends without being listed there. This applies to ADRs 0038, 0126, 0127 and 0129 and the ADRs listed in the row, not again to the amending ADRs. A row lists the ADRs whose main decisions that area builds: the pull request that records a new ADR adds it to the rows of the areas that build its main decisions, and its fresh review checks it; the amendment rule brings it to the areas whose listed ADRs it amends, and to every area when it amends ADR 0038, 0126, 0127 or 0129.

> Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; this check is a review standard the `code-review` skill applies, and the owner sees its result before merging.

The Proving ground comes second because it is the oracle every later area's conformance suite runs against. Security precedes Site access because ADR 0017's egress policy wraps every fetch. Scheduling precedes Client surfaces and the Console, which expose its `schedules.*` operations (ADR 0036). Where two areas use each other's contracts, as Scheduling uses the Console's Run index and the Notification events of Client surfaces (ADRs 0031 and 0027), the ADRs already fix the shared contract, and the earlier area's spec builds against it while the later area's spec implements it. Across areas, `/to-issues` blocks each issue on the issues that build the contracts it consumes, whichever area they belong to; when those issues do not exist yet, the later area's `/to-issues` adds the links to the issues already created.

> Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the skill is now `/to-tickets`, and it blocks each ticket.

## Considered Options

- **Cloud-only rows excluded from parity:** rejected. Goodfetch would look weaker than Stagehand on what users pay Browserbase for, while every such row but the keyless gateway already had a self-hosted equivalent.
- **Managed versions of cloud-only rows in V1:** rejected. They are Goodfetch Cloud operations, a separate map.
- **Parity on Stagehand v4 only:** rejected. It would drop the capabilities that Crawlee+Stagehand users, stuck on v3, look for elsewhere.
- **A Stagehand v3 compatibility layer:** rejected as over-engineering. It freezes a third-party API that fights the Operation catalog (ADR 0011), for an integration with about 3,300 weekly npm downloads against about 1.7 million for Stagehand.
  > Note, 2026-11-11 (the owner, grilling of decision G-19 of the consolidated review, Q10: « Yes to all », answered after being shown this line): the owner reversed this reason for answering Firecrawl's and Jina Reader's HTTP APIs, which Q10 takes (ADR 0027's note of that day); it is not taken to apply to them.
  >
  > Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading; shown to the owner before the answer as a table of differences): those are current APIs that other tools copy, translated at the HTTP edge onto Goodfetch's operations and adding none, and the version they follow is the current one, kept current by the re-read in the note of that day above.
- **Every API detail as its own decision:** rejected. It would add about 40 tickets to settle what `/to-spec` settles better with the whole area in view.
- **A final re-audit ticket after the new tickets:** rejected. A matrix kept current by each ticket makes the end condition checkable without a second pass.
- **Ordering specs by user value** (AI first) **or one spec for all:** rejected. `/to-issues` needs blockers built before what they block.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the skill is now `/to-tickets`.

## Consequences

- The parity list is the research of 2026-10-10 (Crawlee JS 3.18.2 and 4.0.0-rc.1, Crawlee Python 1.10.4, Spider 2.53.9, Stagehand 4.2.0), kept on the unmerged `research/parity-list` branch. The matrix carries its row IDs, so specs cite rows as `ADR 0037 / AI-12`.
- Specs carry the notes the matrix attaches to rows, for example that clicks made to discover links are not Effects, and that the page API dismisses dialogs by default because accepting one can confirm an Effect.
- The Engine runtime spec says how a reader tests a run's lock (ADR 0031) without making a writer that tries at the same moment think the owner is alive, either with a lock kind that can be tested without taking it or with writers that retry when the owner they forward to is gone; some lock kinds, such as `flock`, are usually tested by briefly taking them.
