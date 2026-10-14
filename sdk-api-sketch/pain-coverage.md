# Pain coverage matrix

Every developer pain the [consumer API research](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) found, and how the SDK API answers it. Its rows were set by [Golden scenarios and the pain coverage matrix](https://github.com/goodfetch/automan/issues/463). It is the map's own working list, distinct from the pain rows of the V1 scope matrix ([ADR 0037](https://github.com/goodfetch/automan/blob/main/docs/adr/0037-v1-scope-parity-and-pain-coverage.md)), which cover the market's pains rather than the SDK API's.

## How a row closes

Each row closes in one of three ways, and the map's Destination is reached when no row is blank ([Close the pain coverage matrix](https://github.com/goodfetch/automan/issues/485)):

- **API**: answered by a named SDK API feature (a type, method or setting) that a [golden scenario](scenarios/README.md) shows.
- **Elsewhere**: answered by an adopted decision outside the SDK API, which the row links (provisioning, versioning, release cadence, the Goodfetch Chromium).
- **Declined**: not answered, with the reason.

## Who fills it

This ticket wrote every row's ID, pain, sources, deciding ticket and the scenarios expected to show it. Each deciding ticket fills its own rows (closes as, feature, scenarios, principles) in the same commit as its sketch changes, and links them from its resolution. Close the pain coverage matrix closes whatever is left. A row's sources list every research item it answers. Every item of the merged list and of the three family lists appears in at least one row. A per-library pain that no family item covers has its own row (P52 to P57), found by a sweep of every per-library list in the notes; per-library items that sharpen an existing row are listed in its sources.

**Sources.** `M1`–`M19`: the merged pains of [synthesis §4](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md#4-pain-points-merged-ranked-by-evidence-weight). `AI-n`: [AI browser automation §6](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md#6-pain-points-ranked-by-evidence-weight). `DRV-Pn`: [browser drivers §6](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/browser-drivers.md#6-pain-points-ranked-by-evidence-weight). `CR-6.n`: [crawl frameworks §6](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md#6-pain-points-ranked-by-evidence-weight). `L1`–`L12`: the merged likes of [synthesis §3](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md#3-what-users-like-merged-ranking). Per-library items are named with their library.

**Deciding tickets**, by short name:

| Short name | Ticket |
|---|---|
| Lifecycle | [Entry points and lifecycle](https://github.com/goodfetch/automan/issues/465) |
| Results | [Results, outcomes and errors](https://github.com/goodfetch/automan/issues/466) |
| Time | [Time, waits, deadlines, cancellation and budgets](https://github.com/goodfetch/automan/issues/467) |
| Concurrency | [Concurrency model per language](https://github.com/goodfetch/automan/issues/468) |
| Typed data | [Typed data: schemas, Records and payloads](https://github.com/goodfetch/automan/issues/469) |
| Settings | [Settings in code](https://github.com/goodfetch/automan/issues/470) |
| Catalog | [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) |
| Page API | [Collie's page API](https://github.com/goodfetch/automan/issues/472) |
| AI verbs | [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473) |
| Agent | [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) |
| Models | [Models, usage and cost in the SDK API](https://github.com/goodfetch/automan/issues/475) |
| Sessions | [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476) |
| Crawl | [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477) |
| Run | [The Run in code](https://github.com/goodfetch/automan/issues/478) |
| Output | [Crawl output in code](https://github.com/goodfetch/automan/issues/479) |
| Politeness | [Crawl politeness defaults](https://github.com/goodfetch/automan/issues/480) |
| Hooks | [Hooks and injectable components in the SDK API](https://github.com/goodfetch/automan/issues/481) |
| Testing | [Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482) |
| Close | [Close the pain coverage matrix](https://github.com/goodfetch/automan/issues/485) |

## Pains

"Scenarios" holds the golden scenarios expected to show the answer; "—" means none is chosen yet, and the deciding ticket or Close places it. "Notes" names the adopted decisions a row starts from.

| ID | Pain | Sources | Deciding | Closes as | Answering feature | Scenarios | Principles | Notes |
|---|---|---|---|---|---|---|---|---|
| P01 | "Element ready" is not "app ready": no first-class wait for app state (URL, response, count, load state, predicate) under one deadline | M1, DRV-P1; Midscene #1551 (popups interrupting an action) | Time; Page API (strictness, interrupt handlers) | | | 01 | | The guard covers element readiness (ADR 0019) |
| P02 | Navigation races: a waiter must be armed before the action that triggers it | DRV-P6 | Time | | | 01 | | |
| P03 | AI steps are non-deterministic and cost on every run: no replay without the model | M2, AI-1; Midscene #443, Skyvern #1271 (latency per AI step) | AI verbs | | | 02, 03, 08, 10, 11 | | Plans replayed without a model (ADR 0009) |
| P04 | False success: failures and empty results that look like results | M3, AI-2, CR-6.15; Stagehand #651, #912 (values retyped by the model, incomplete results) | Results; AI verbs (a deterministic verb that runs a Success check); Agent (stopped, reported or verified) | | | 02, 03, 05, 06, 11 | | AI success is never judged by a model alone (ADRs 0013, 0014) |
| P05 | Embedding: the library owns the process, the event loop or signals; one instance per process; not notebook-safe | M4, CR-6.1 | Lifecycle | | | 09, 12 | | Attached engine and Daemon (ADR 0024) |
| P06 | Teardown and ownership: leaks, zombie browsers, no async `Drop`, a library closing a browser it did not launch | M4, DRV-P3, AI-5; Browser Use #839, agent-browser #42, Playwright MCP #891, Crawlee #1822 (attaching to the user's own browser, locked profiles) | Lifecycle | | | 01, 09, 12 | | Borrowed browser (ADR 0056) |
| P07 | Breaking churn and stale examples | M5, AI-3, CR-6.8; Notte #551 (SDK and engine versions drifting apart) | Catalog (type-level deprecation markers, T) | | | — | | Versioning and deprecation (ADRs 0030, 0041, 0047); docs samples run in CI (ADRs 0030, 0068) |
| P08 | Silent hangs: no deadline on navigation, hooks, handler, sink write or shutdown, and errors that name no phase | M6, AI-8, CR-6.4; Cypress #349 (recovering from a browser crash) | Time | | | 04, 07 | | Leases and timeouts per work item (spec area 3) |
| P09 | No live status of what is in flight, what waits, and why | M6, CR-6.4 | Run | | | 07 | | |
| P10 | Weak, stringly errors: matching on message text, no codes, no structured fields | M7, DRV-P8, AI-8 | Results | | | all | | Typed Error codes (ADR 0038) |
| P11 | Switching fetch tier is a rewrite: another crawler class, context or build | M8, CR-6.5 | Crawl | | | 06 | | Fetch tiers chosen by sample comparison (ADR 0020) |
| P12 | Unsafe crawl politeness defaults: robots.txt ignored, high per-host concurrency, no delay, anonymous user agent | M9, CR-6.2 | Politeness | | | 05 | | ADRs 0016 and 0021, reopened by amendment only; shared Politeness (ADR 0053) |
| P13 | Telemetry on by default, carrying task text and addresses | M9, AI-9 | Close | | | 11 | | Nothing leaves the machine unless configured (ADR 0043): an Elsewhere candidate |
| P14 | Secrets reach prompts, caches, logs and snapshots | M9, AI-9 | Sessions | | | 04 | | Secret references (ADR 0015) |
| P15 | AI actions bypass the domain policy | M9, AI-9; LaVague #650 (prompt injection through page content) | AI verbs | | | 11 | | One egress policy (ADR 0017); AI safety in the engine (ADR 0014) |
| P16 | Code execution: model output or user code run with no trust level in the call's name | AI-9 | Page API (`evaluate`); AI verbs | | | — | | Scripts in jitless V8 (ADR 0051) |
| P17 | Run state surprises: purge on start, a second run that does nothing, re-running failures | M10, CR-6.3 | Run | | | 07 | | Portable run state and Checkpoints (ADR 0002) |
| P18 | Duplicate or partial results on retry | M11, CR-6.14 | Output | | | 07 | | Fencing and at-least-once delivery (ADR 0003); staged outputs (spec area 3) |
| P19 | Model breadth and fragile structured output | M12, AI-4; Stagehand #1029, #938, #1098 (coupling to one schema library) | Models; Typed data (schema input) | | | 02, 08, 11 | | Gateways and local servers (ADR 0012) |
| P20 | Interop: AI on a page another driver owns, and AI inside crawlers | M13, AI-7 | Lifecycle (`borrow`); Crawl (Collie inside a handler) | | | 08, 09 | | ADR 0056; [Third-party automation on Foxhound pages, and Collie inside other crawlers](https://github.com/goodfetch/automan/issues/456) |
| P21 | Passing data between handlers: untyped, framework-owned or silently stringified payloads | M14, CR-6.10 | Typed data | | | 05 | | |
| P22 | Concurrency model friction: a blocking call inside a running loop, thread safety, sharing handles across tasks | M15, DRV-P7; Rust crates P6, P11 (`Clone`, `Send`, `&mut self`); Skyvern #2, scrapy-playwright #343 (Windows event loops) | Concurrency | | | 12 | | ADR 0038's concurrency model |
| P23 | Cancellation that does not stop the engine-side work | M15, DRV-P7 | Time | | | 12 | | `$/cancel` (ADR 0038) |
| P24 | Provisioning browsers and packages, runtime portability and dependency weight | M16, DRV-P2, CR-6.7, AI-14; Rust crates P7 (port scans, stderr parsing) | Lifecycle (the missing-browser error) | | | setup of all | | One binary, no Node runtime (ADRs 0024, 0030, 0057): mostly Elsewhere |
| P25 | Token bloat in agent-facing observations | M17, AI-6 | AI verbs | | | 11 | | MCP profiles (ADR 0011) |
| P26 | Cloud lock-in: features that only work in the vendor's cloud | M18, AI-10, CR-6.19 | Close | | | 03 | | Cloud as a protocol client (ADR 0001): an Elsewhere candidate |
| P27 | Compile-time capability gating | M19, CR-6.12 | Settings | | | 06 | | A capability is a setting (`AGENTS.md`, ADRs 0038, 0057) |
| P28 | Getting data out of the page: text, attributes, `evaluate` serialization, one round trip per query | DRV-P4; Rust crates P10 (one round trip per element query) | Page API | | | 01 | | |
| P29 | Eager handles, stale references, handle-model confusion | DRV-P5 | Page API | | | 01 | | |
| P30 | Network interception, response bodies, console messages and page errors | DRV-P9; Stack Overflow "all console messages" (102 votes), thirtyfour #87 | Page API | | | 06 | | |
| P31 | Coupling to one test runner, and defaults driven by a testing philosophy | DRV-P10 | Testing | | | 10 | | Public testing kits (ADR 0069) |
| P32 | Coupling to a browser and protocol version, drift, patched builds | DRV-P11 | Close | | | — | | ADRs 0019, 0035, 0074: an Elsewhere candidate |
| P33 | No way down below the abstraction | DRV-P12, synthesis pattern 3 | Page API (raw CDP); Catalog (raw catalog call) | | | — | G8 | G8: no golden scenario needs a way down; it is shown by a snippet |
| P34 | Multi-tab, multi-origin and frame models; shadow DOM, custom dropdowns and off-screen content the engine cannot reach | DRV-P13; Browser Use #1700, #871; Midscene #179; rustwright #186 | Page API | | | — | | |
| P35 | Detectability by anti-bot systems | DRV-P14 | Close | | | — | | Stealth catalog and the Goodfetch Chromium (ADR 0035): an Elsewhere candidate |
| P36 | Generated code is not production-grade | DRV-P15 | AI verbs | | | 03 | | Plans are reviewable, versioned files (ADR 0009) |
| P37 | Fixed sleeps removed with no pointer to the replacement | DRV-P15 | Time | | | — | | |
| P38 | Configuration footguns: unknown or ignored options accepted, surprising precedence, mixed time units, import-time side effects | DRV-P15, AI-16, synthesis pattern 1; Crawlee v3 globs replacing the strategy, Spider #293 (crawl scope patterns that combine silently) | Settings; Time (one duration type) | | | all | G3 | Layered settings (ADR 0028) |
| P39 | Governance, maintenance and release cadence | DRV-P15, CR-6.20 | Close | | | — | | Monthly minors (ADR 0030): Elsewhere or Declined |
| P40 | Login, two-factor authentication and a person in the loop | AI-11 | Sessions | | | 04 | | Session sources (ADR 0056); Human pause |
| P41 | Driver guarantees lost behind a "Playwright-shaped" page | AI-12 | Page API | | | 09 | | The guard (ADR 0019) |
| P42 | Budgets and cost visibility | AI-13; Browser Use #4103, #3726 (a judge call and a paid model on by default) | Time (budgets); Models (usage and cost) | | | 02, 08, 11 | | Limits (ADR 0041); costly controls reported (ADR 0023) |
| P43 | Language ports and bindings drift from the core | AI-15, CR-6.11, synthesis pattern 6 | Catalog | | | all | S1 | Generated SDKs over one catalog (ADRs 0011, 0024, 0038) |
| P44 | Logs written to stdout, or out of the host's control | AI-17, CR-6.18, synthesis pattern 8; Magnitude #11, HyperAgent #88 (run reports and artifacts) | Lifecycle | | | 12 | G4 | Only the transport, the CLI's writer and `tracing` touch stdout (`AGENTS.md`) |
| P45 | Documentation gaps, undocumented options, no real-world examples | CR-6.6 | Close | | | all | | The golden scenarios become docs samples; the docs shape is in the map's fog |
| P46 | Extensibility: no middleware, no injectable components, fetch tied to parse | CR-6.9 | Hooks | | | — | | Plugins (ADR 0033) |
| P47 | Autoscaling grows concurrency until memory runs out, with no reason shown | CR-6.13 | Run (why concurrency is what it is) | | | — | | Fair per-domain admission (ADR 0004); standard access (ADR 0021) |
| P48 | Failure, retry and skip semantics in handlers | CR-6.15 | Crawl; Results (outcome kinds) | | | 05 | | |
| P49 | Handler-context access and typing: final URL, headers, cookies; one return type per method | CR-6.16; Crawl4AI #855, #1509 (features failing when combined) | Crawl | | | 05, 06 | S2 | |
| P50 | Cookies and proxies of the Session in code: reading and writing cookies, a proxy per page or Session, evicting blocked proxies | CR-6.17; Puppeteer #678 (proxy per page, +118, declined), Firecrawl #1129, crawl forum CP7 | Sessions | | | 04 | | ADR 0056; Session pool (ADR 0021) |
| P51 | Supply chain: heavy, exactly pinned or compromised dependencies | AI-9 | Close | | | — | G7 | Thin SDKs (ADR 0024); pinned ranges for idiom types (G7): an Elsewhere candidate |
| P52 | Input actions unlike a real user: Tab, hover, native events, choosing an option, drag, Enter, `fill` ignored by controlled inputs | Cypress #299 (1,700 r), #10, #311; Stack Overflow input-actions cluster (21 questions, 674 votes); rustwright #187 | Page API | | | 01 | | The guard's actions (ADR 0019) |
| P53 | Files: downloads, uploads, PDFs, screenshots, the clipboard | Browser Use #499, #729, #1951; Stack Overflow files cluster (17 questions, 358 votes); agent-browser #587 | Page API | | | — | | |
| P54 | Browser and context setup: viewport, window size, locale, time, JavaScript off, network throttling | Stack Overflow browser-configuration cluster (27 questions, 608 votes); Playwright #8622 (+94, not planned) | Page API; Settings | | | — | | |
| P55 | Keeping raw responses (an HTTP cache, WARC) so parsing can be rerun offline without fetching again | Crawl forum evidence, ranked theme 17 (4 independent reports) | Testing; Output | | | 10 | | WARC is in V1's scope |
| P56 | Crawling across several machines with one shared Frontier | Crawl forum evidence S23, G14, K1 (low weight) | Close | | | — | | Partitioned crawl (ADR 0054); Daemon group (ADR 0055): an Elsewhere candidate |
| P57 | Electron and desktop apps as targets | Playwright MCP #994 (+12), playwright-cli #229 (+10) (low weight) | Close | | | — | | A Borrowed browser reached by its address (ADR 0056) may cover it |

## Likes to keep

What users praise in the leaders, and the feature that keeps it.

| ID | Liked quality | Sources | Deciding | Closes as | Keeping feature | Scenarios | Principles | Notes |
|---|---|---|---|---|---|---|---|---|
| K01 | Auto-waiting, guarded actions | L1 | Page API | | | 01 | | The guard (ADR 0019) |
| K02 | Generate with AI once, then own a deterministic, reviewable artifact | L2 | AI verbs | | | 03, 08 | | Plans (ADR 0009) |
| K03 | One API across HTTP and browser fetching | L3 | Crawl | | | 05, 06 | | |
| K04 | Plain async code with no hidden queue; small primitives mixed with ordinary code | L4 | Concurrency | | | 12 | | |
| K05 | Seeing what happened: traces, replay, the code of each action | L5 | Run | | | 07 | | Run trace; the Console (ADR 0031) |
| K06 | One official API in several languages | L6 | Catalog | | | all | S1 | ADRs 0011, 0024, 0038 |
| K07 | Testable callbacks and offline replay of saved responses | L7 | Testing | | | 10 | | ADR 0069 |
| K08 | Any model, including local ones | L8 | Models | | | 02, 11 | | ADR 0012 |
| K09 | Time to first result: one install, browsers fetched for you | L9 | Lifecycle | | | setup of all | | ADRs 0024, 0030 |
| K10 | Token-efficient observations | L10 | AI verbs | | | 11 | | ADR 0011 |
| K11 | Locators that read like the user's view, next to CSS and test IDs | L11 | Page API | | | 01 | S3 | |
| K12 | Middleware and fine-grained control off the happy path | L12 | Hooks | | | — | | ADR 0033 |
