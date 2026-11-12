# Collie carries every capability except those only many pages need, the Script sandbox and the Daemon included

ADR 0084 made Collie the Product that drives one browser page at a time and Foxhound the one that discovers and collects many pages, with Foxhound's executable containing Collie's code, and left Collie's exact contents to this decision. Collie carries the whole engine except what exists only because a run discovers, visits and keeps many pages. That is run state and the Effect journal, Plans, the guard, Sessions, stealth, the egress policy, Clean-up rules, the Script sandbox in V8, Plugins, the Browser host, every agent surface, and the Daemon, Run index, Console, Usage ledger and Schedules. Foxhound alone adds the Frontier and crawl scope, Politeness toward sites, the HTTP fetch tiers, the Session pool, Paid fetches, Datasets, Records, Sinks, DuckDB and Partitioned crawls. The map's research found:

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no executable embeds a Console build, no Size budget counts one, Foxhound's screens are no workspace package of their own, no release test walks a Console's routes, and no Daemon serves a Console. The Stub gateway stays for the cloud-readiness conformance, without a Console build.

- **What sets Goodfetch apart is what Collie's market lacks.** No browser-automation product has an Effect journal, a replay that verifies its target, or a real Script sandbox. Durable run state appears only where there is a server (Skyvern, Browser Use's Terminal), and stealth and Clean-up rules are cloud features almost everywhere ([browser-automation products](https://github.com/goodfetch/goodfetch/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §9).
- **Agent-facing tools ship the shared side.** A CLI with skills, an MCP server, a daemon and a local dashboard are table stakes: Playwright, agent-browser, Chrome DevTools MCP, Browser Use and Stagehand's `browse` each ship at least three of the four (same note, §9).
- **The many-pages part is what makes the executables differ.** DuckDB (+33.0 MB), the in-binary Sinks (about +11 MB: the seven sink libraries' 21.5 MB less MongoDB's and SQL Server's, which ADR 0026 runs as sink processes) and `wreq` (+5.5 MB) come to about 50 MB. The Collie set measured 18.6 MB with the filter-list engine, about 22.7 MB with its embedded data and about 65 MB with a prebuilt V8 ([multi-product builds](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §7.1–§7.2), against ADR 0057's 95–120 MB projected for the whole set, now Foxhound's.
  > Amended by [ADR 0124](0124-collies-tls-relay-links-wreqs-boringssl-but-not-wreq-and-forwards-the-browsers-own-http2.md): the BoringSSL inside `wreq`'s +5.5 MB, measured at +1.43 MB raw, is in both executables, since Collie's TLS relay links it. Only the rest of `wreq` is Foxhound's.

Decided in [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/goodfetch/issues/447), by the agent on the owner's delegation of 2026-10-14 ([grilling record](https://github.com/goodfetch/goodfetch/issues/455#issuecomment-6179818426), [round 4](https://github.com/goodfetch/goodfetch/issues/455#issuecomment-6181411312), [corrections](https://github.com/goodfetch/goodfetch/issues/455#issuecomment-6181950280)), and confirmed by the owner, who also decided Q18, which the fourth review raised ([confirmation](https://github.com/goodfetch/goodfetch/issues/455#issuecomment-6187800565)). The evidence is also in the research on [crawl frameworks](https://github.com/goodfetch/goodfetch/blob/research/crawl-framework-products/rnd/research/2026-10-14-crawl-framework-products.md).

## Decision

### The rule

- **Collie carries everything that acts within a Run's Sessions and pages, or that guards, records, limits or reports that work, and every piece ADR 0084 put on the shared side.** Foxhound adds only what exists because a run discovers, visits and keeps many pages.
- **A capability Collie carries behaves the same in both Products.** A Foxhound handler's page runs Collie's code, and ADR 0083 already gives every setting one default in both.

### What Collie carries

| Group | Collie carries | ADRs |
|---|---|---|
| Runs | Run state in SQLite, Checkpoints and resume, the run epoch, a run's Key-value store, the run's report and Reconciliation | 0002, 0003, 0029, 0051 |
| AI | The Effect journal; Plans with Target fingerprints, Drift, Repair and Decision providers; the recorder; Success checks, Grounding and Verdicts; the built-in Agent; the engine's AI safety; model calls through gateways, local servers and the SDK callback | 0009, 0010, 0012–0014 |
| Extraction | Extraction from the page it is on: the default extractor, AI extraction with Grounding, compiled selectors, Extractor Plugins, and Clean-up rules applied to content | 0007, 0013, 0047 |
| Driver | The CDP driver and the guard, the browser and browser-fetch tiers, the raw CDP endpoint, Lent pages (`page.lend()`), Firefox and WebKit through Playwright's driver | 0019, 0020, 0089 |
| Sessions | Access modes and Identity profiles; proxies with their pools, provider adapters and per-proxy limits; local addresses; Session sources, Saved identities, Profile directories and Borrowed browsers; the Page check, login recovery, MFA and the Human pause | 0006, 0021, 0035, 0044, 0053, 0056 |
| Site access | The stealth catalog and the Goodfetch Chromium (a download, never part of the executable, ADR 0084), block detection and the Challenge solver; Filter lists, Consent prompts, Walls and the Wall allowance; the Response cache; third-party CDP browsers, Browser hosts and the Browser host's own work | 0022, 0035, 0046–0048, 0052 |
| Security | The egress policy and the endpoint rule; certificate exceptions and the TLS relay; Secret references, Secret providers and field encryption; the Audit log; the switchable controls and their reporting; Purpose and Site preferences read and recorded; nothing sent unless configured | 0014–0017, 0023, 0042, 0043 |
| Scripts and Plugins | The Script sandbox in V8, Code mode and Script Hooks; the plugin protocol, confinement, the Plugin index and the conformance suites of its kinds, with every Plugin kind but Fetch backend and Sink | 0011, 0033, 0051 |
| Surfaces | The SDKs, the CLI with `--json`, the MCP server over stdio and Streamable HTTP, agent skills, Framework adapters, the web search operation, sites' WebMCP tools, Notifications and the HTTP listener | 0011, 0027 |
| Shared side | The Attached engine and the Daemon, the Exposed Daemon with its accounts, the ops listener, the Daemon group, the resource governor with its learned ceiling per cloud-browser provider, the Run index, the Console with the Live view, Run traces and "Needs you", the Usage ledger and its Limits, Schedules and Watches, Recipes | 0024, 0031, 0036, 0040, 0041, 0049, 0050, 0055 |

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

### What only Foxhound carries

| Foxhound alone carries | ADRs | Why not Collie |
|---|---|---|
| The Frontier, crawl scope and learned Frontier rules, with the work-item leases that fence them | 0003, 0005 | Collie's pages are the ones its program directs |
| Politeness toward sites: the per-domain limits and backoff, the robots.txt gate and Crawl-delay | 0004, 0021, 0053, 0086 | A program's own navigation is outside them (ADR 0086) |
| The HTTP and discovered-API fetch tiers, impersonated HTTP (`wreq`) and the choice of tier per route; hedged requests | 0020, 0040 | Below |
| The Session pool: scoring, rotation, retirement, account rotation, and escalation by opening a new Session (ADR 0021's proxy tiers and the ladder's step 3), learned per domain | 0006, 0021, 0035 | Below |
| Paid fetches | 0045 | Below |
| Datasets, Records, Exports and Sinks; DuckDB, with search across runs and the SQL tab; change tracking against a Baseline | 0008, 0026, 0031, 0036, 0057 | Collie's results go to its caller (below) |
| Partitioned crawls | 0054 | They split a crawl's domains |
| The Fetch backend and Sink Plugin kinds; the Hook stages before request, after response and Record before commit | 0033 | They act on a Frontier's requests and a Dataset's Records |

> Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): the new observe-only Hook stage `request_ended`, which receives a crawl request's outcome, is Foxhound's too.

### Sessions a program or an Agent opened are never replaced

- **The Session pool is Foxhound's.** The engine retires and replaces only Sessions that a crawl's Session pool opened.
- **A Session a program or an Agent opened itself is never retired, rotated or replaced**, in either Product and whatever its Session source. The program holds that Session and its pages (ADR 0077's G2), and a replacement would carry other cookies, another identity and none of the page's state. Crawlee supplies "the queue, sessions, proxy rotation, autoscaling and fingerprints; the driver supplies the page" ([browser-automation products](https://github.com/goodfetch/goodfetch/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §8.2), and the research found no browser-automation product that rotates a program's session under it.
- **A failure that would retire or replace such a Session leaves it in place.**
  - A Challenge, a CAPTCHA included, goes on down ADR 0035's ladder without step 3, the new `stealth` Session: the Challenge solver when `solve_challenges` is on, a Human pause when a human channel is configured, otherwise `challenge_unsolved`. The ladder's other steps replace no Session, so they stay in Collie, and their start is learned per domain as ADR 0035 says.
  - Any other Block cause is reported on the call that met it, on a Borrowed browser and a Profile directory used in place too. The program decides what happens to its own Session (ADR 0077's G1 and G2), and can ask for a Human pause itself ([Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/goodfetch/issues/476)). ADR 0056's Human pause for such a failure stays for a crawl's Session on a Borrowed browser or a Profile directory. A `goto()` that meets a Block cause it cannot get past raises a `PageError` naming it ([ADR 0079](0079-failures-raise-grouped-typed-errors-and-expected-outcomes-are-values.md)); the rest of how a reported Block cause reads in the SDK API is [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/goodfetch/issues/476)'s.
    > Amended by [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): such a Block cause raises `blocked`, a `PageError` with the cause, the URL and the status, from the call waiting for the navigation that met it, or else the page's next call; a Challenge the ladder cannot pass raises `challenge_unsolved`, and `session.recovery.human_pause: false` skips the ladder's Human pause.
- **Opening another Session**, in another Access mode or behind another proxy, is the program's or the Agent's choice, under ADR 0021's rule that such a change opens a new Session. Proxy pools and provider adapters stay in Collie: they give each Session its proxy when it opens.

### Proxy and provider limits

- **Collie carries ADR 0053's per-proxy limits** (`politeness.proxy.concurrency` and `politeness.proxy.rate`, unset by default), the ceiling the engine learns from a proxy's own refusals, and the ceiling ADR 0040's governor learns per cloud-browser provider, shared as ADR 0053 shares them (`politeness.sharing`).
- **They protect the user's own accounts, not the sites.** A proxy plan or a cloud-browser plan caps the whole account, and the vendor refuses past it. Without them, an Agent that opens Sessions through one proxy, or several Collie runs sharing a plan, would turn the vendor's refusals into failed calls instead of waiting for a slot. So they hold a program's own navigation through that proxy or provider, which ADR 0086 otherwise keeps outside Politeness.
- **A proxy's own refusal is never a Block cause** in Collie either: a 429 or 502 from the proxy is a proxy error that lowers its ceiling, and a 407 fails the call with `proxy_auth_failed` (ADR 0053).
- Whether a Collie page's 429 from a site slows a Foxhound crawl on the same machine stays [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486)'s (ADR 0098; ADR 0086).

### The Script sandbox stays in Collie

- **V8 without JIT, Code mode and Script Hooks are in Collie's executable**, as in Foxhound's. With a prebuilt V8 (+42.8 MB), Collie comes to about 65 MB raw and 27 MB compressed before the engine's own code. V8 built from source with its sandbox may cost up to about 60 MB, which brings Collie to about 80 MB raw ([multi-product builds](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §7.2).
- **Code mode serves Agents that drive pages, who are Collie's users, and no competitor runs it in a real sandbox.** Stagehand v4 removed `agent()`, pointing to code mode and tool calling instead, and its integrations' `run` tool executes model-written JavaScript; Playwright MCP's `browser_run_code_unsafe` is "RCE-equivalent"; Browser Use's CLI runs agent-written Python; dev-browser warns that `node:vm` "is not a security sandbox" ([browser-automation products](https://github.com/goodfetch/goodfetch/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §2.1, §7, §9). In the lab, raw code mode gave 17 false successes out of 45, and none through the journal (ADR 0010).
- **A Recipe that carries a Script Hook runs on either Product**, and an MCP profile can offer Code mode in either (ADR 0011).
- **The size stays in its market's band.** Collie's compressed file sits beside Playwright Python's 33–46 MiB wheels and Stagehand's 33.0 MB npm install (same note, §2.2, §3.2), and the browser it downloads, 124–198 MB compressed, is still most of a Collie user's download (ADR 0057).

### Fetching beyond the browser

- **The HTTP and discovered-API fetch tiers, impersonated HTTP (`wreq`) and the choice of tier per route are Foxhound's.** Collie keeps the browser and browser-fetch tiers (ADR 0020): it sends a Session's HTTP only from inside the Session's browser, through `page.request` ([ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md)), so the Session's identity never changes.
- **Requests that are not a Session's go through the engine's own HTTP client,** such as those to the endpoints an operator configures (Model providers, Secret providers, search providers, Notification channels) and Plugins' `net.fetch`, which may reach the run's sites but never carries a Session's cookies (ADR 0033).
- **Paid fetches are Foxhound's.** ADR 0045 answers "a site that asks a crawler to pay": its `max_price_per_page`, its Completeness rule for pages left in scope and its Provenance mark on Records are crawl concepts, and it brings `x402-rs` and `alloy`, which nothing else in Collie needs.
  - A Collie page that asks for payment gets the Block cause `payment_required`, reported as any other Block cause, and Collie never pays.
  - `pay_domains` given to a Run on a Collie engine is refused like any other setting Collie does not carry (below).
  - Adding Paid fetches to Collie later, for Agents paying through x402, would be additive.
- **The Response cache is Collie's.** It serves browsers through the engine's interception (ADR 0046), and its `replay` mode is the offline loop for developing an automation, as Playwright's HAR replay is ([browser-automation products](https://github.com/goodfetch/goodfetch/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §3.1). Whether the SDK API's testing story uses it is [Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482)'s.
  > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): a program's tests do not use the cache; they replay a Response recording, a WARC file the Run's start names, which both Products read and write. `replay` stays the development loop.

### Remote browsers and the Browser host

- **Collie drives remote browsers:** third-party CDP browsers and Browser hosts alike (ADR 0022).
- **Everything a Browser host does is Collie's work:** the guard over a pipe, egress enforcement, the certificate relay, Filter-list matching, live hiding, the Consent prompt runner, memory reporting and the endpoint through which a hosted page is lent (ADRs 0022, 0040, 0042, 0047, 0089). So either Product's executable can serve as a Browser host with no Foxhound code.
- Which executable and image Goodfetch ships for the Browser host and for Goodfetch Cloud's engines, and how a Browser host is versioned against its engine, are [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/goodfetch/issues/498)'s (ADR 0096).

### The shared side

- **Collie carries every piece ADR 0084 put on the shared side, because Collie's own capabilities rest on them.**
  - A Human pause, a Notification's link and "Needs you" need a Console: a script's own, or the Daemon's, which an Attached engine starts on demand (ADR 0031).
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - Reconciliation, raising a Limit and widening `consent_domains` or `wall_domains` are never done through an agent surface: a person answers in the Console or an interactive CLI prompt, or edits configuration (ADRs 0029, 0041, 0048, 0052).
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - A CLI or MCP session that outlives one command needs a long-lived engine, as agent-browser's and Playwright's daemons are ([browser-automation products](https://github.com/goodfetch/goodfetch/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §3.1, §3.3, §6.1).
- **The Console** shows Collie's runs, the Live view, Run traces, the recorder and "Needs you" (Human pauses, confirmations, uncertain Effects, Plan diffs, runs stopped by a Limit, results to review). Foxhound adds Datasets with their SQL tab, Sinks, crawl progress and search across runs (ADR 0084).
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **The Usage ledger** holds every Limit scope but a Partitioned crawl's, so an Agent that starts runs through MCP is capped across them (ADR 0041).
- **Schedules** keep their three triggers in Collie. Repeated automations would otherwise go to cron or to a vendor's cloud, as Browserbase and Notte Functions, Browser Use's rerunnable scripts and Skyvern's schedules do (same note, §8.1), which ADR 0001's self-host parity rejects.
  - A Watch checks its pages through a browser Session, and its comparison point follows ADR 0036's rule in both Products.
  - Change tracking of Records against a Baseline, the "only new or changed" Sink mode and the "records changed" Notification need Records, so they are Foxhound's.
- **The Daemon group** is Collie's too, without Partitioned crawls, so a team serving agents from several pods shares one Usage ledger, Run index and set of Schedules (ADR 0055).
- **Recipes** run on Collie when they need nothing Foxhound alone carries.

### Collie's results

- Collie has no Datasets, Records, Exports, Sinks or DuckDB (ADR 0084). An extraction or an AI task returns its result to its caller, with its Provenance and the run's Verdict, as Stagehand's `extract` returns a value and leaves storage to its caller (same note, §2.4).
  > Amended by [ADR 0091](0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md): a single AI verb call (`act`, `observe`, `extract`) returns a `StepResult` with its output, how its step was resolved and its Usage, and no Verdict: the run's Verdict comes with its report once it finishes (ADR 0029), and an Agent's task returns its own (ADR 0079's `AiWorkResult`). The rest of a value's Provenance is kept in the Run trace.
- A run's Key-value store stays in Collie: Code mode programs and Script Hooks keep their lasting state there (ADR 0051), and a program keeps its own state there across a resume.

### Collie inside Foxhound

- **A Foxhound handler's browser page is a Collie page:** the same object, with the same AI verbs, journaled and fenced by the handler's lease (ADR 0084). ADR 0037's row V3-01 (`act`, `extract` and `observe` on a crawler's page) is therefore decided. A Crawlee or Scrapy handler reaches the same verbs on its crawler's page through `run.borrow(page)` ([ADR 0089](0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md)), as a page of a Borrowed browser without a handler's lease (ADR 0056).
- The verbs' shape is [Collie's AI verbs and Plans](https://github.com/goodfetch/goodfetch/issues/473)'s (ADR 0091), and how a handler receives its page [Crawl definition and handler context](https://github.com/goodfetch/goodfetch/issues/477)'s (ADR 0093).

### What the build and the surfaces take from this

- **Collie's `cargo-deny` ban list** keeps out at least DuckDB (`duckdb`, `libduckdb-sys`), `wreq`, and the crates only Sinks or Paid fetches need, such as `x402-rs` and `alloy`. The crate graph, the checks and each Size budget are [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md)'s.
  > Amended by [ADR 0124](0124-collies-tls-relay-links-wreqs-boringssl-but-not-wreq-and-forwards-the-browsers-own-http2.md): the list keeps out `wreq` and the crates that exist for it, but not its BoringSSL. Collie's TLS relay links `btls`, `btls-sys` and `tokio-btls`, and no crate outside them but the relay's may depend on them.
- **What Collie refuses.** An operation, a command or a Recipe that needs a capability only Foxhound carries, and such a setting given to a Run or connection on a Collie engine, are refused with `not_in_product`, as [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md) decides; it also lets a configuration file hold Foxhound's keys.
  > Amended by [ADR 0096](0096-the-browser-host-runs-from-either-products-executable-and-goodfetch-cloud-runs-each-run-on-the-product-it-needs.md): a Recipe needs the Product its manifest names, which `init` writes and Collie refuses with `not_in_product` when it names Foxhound; whichever engine runs a Recipe holds its code to that Product, so on Foxhound's engine a Foxhound-only operation, Plugin kind or Hook stage on the run of a Recipe that names Collie is refused, whether the Recipe's code or a Client calls or enables it.
- **Collie's Operation catalog** holds the operations of everything Collie carries. How the catalog is split or marked per Product, what `hello` announces, which Integrations and MCP profiles each Product gets, and how surfaces are packaged are [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458)'s.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): one catalog marks each operation with the smallest Product that carries it, `hello` carries the Product a Client needs, MCP profiles carry the same mark, workflow-tool Integrations exist once per Product, and the Airbyte source and the Dataset loaders are Foxhound's; each Product's surfaces are generated from its own catalog.

## Considered Options

- **A library-sized Collie,** with the driver, the guard, the page API, the AI verbs, Extraction and model calls in an Attached engine only, as Stagehand and Playwright are libraries: rejected.
  - Earlier ADRs tie the rest to those verbs. Every AI action goes through the Effect journal, which lives in run state (ADRs 0002, 0010). A Human pause needs a Console's Live view (ADR 0031). Reconciliation and raising a Limit are never done through an agent surface (ADRs 0029, 0041).
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - It would make Collie's `act` the unjournaled `act` the research faults, and a user who outgrew Collie would install the crawler to get a Console.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Whatever fits a size target,** such as agent-browser's 18 MB: rejected. It sorts capabilities by their bytes rather than their purpose, and only V8 is large enough to matter.
- **Plans and AI checks without run state,** as a file cache like Midscene's: rejected. A journal that does not survive the process cannot stop a resumed `act` from paying twice (ADR 0010).
- **Collie without V8** (about 23 MB raw, 12 MB compressed, near agent-browser's 15.8–18.3 MB): rejected. Code mode would sit only in the crawler, though it serves agents that drive pages; a Recipe with a Script Hook would fail on Collie; and MCP profiles would differ by Product.
- **V8 as a download or a companion executable for Collie only:** rejected for ADR 0057's reasons. Codex's split drew missing, unnotarized, stale and mismatched helpers within three months, and both its packages still ship the helper ([multi-product builds](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §1).
- **The Session pool in Collie,** so a blocked Collie page gets a new Session by itself: rejected. It breaks ADR 0077's G2: the program's page would move to other cookies and another identity without its state.
- **A Human pause for any Block cause on a program's Borrowed browser or Profile directory,** as ADR 0056 rules for a crawl: rejected. It would make the same 403 pause one program and fail another by its Session source, and a person can do little about a rate limit; the program can ask for a Human pause itself.
- **No proxy or provider limits in Collie,** as Playwright and Stagehand have none: rejected. The vendors' refusals would reach the program as failures, and runs sharing one plan could not wait for each other.
- **Only classifying a proxy's refusals as proxy errors,** without limits or learned ceilings: rejected. It keeps the diagnosis but not the remedy.
- **Paid fetches in Collie,** so an Agent can pay through x402: rejected for now. ADR 0045 addresses crawlers paying per page, its rules are written in terms of a crawl, and its crates serve nothing else in Collie.
- **The Response cache in Foxhound only:** rejected. It works in browsers, and its `replay` mode is the offline development loop an automation needs as much as a crawl.
- **Remote browsers in Foxhound only, or Collie as a Browser host's client only:** rejected. A hosted browser is a page a program drives, and a Browser host's work is the guard and a Session's egress, which Collie already carries.
- **The Attached engine only, with a trace viewer:** rejected. A Human pause, a Notification's link and "Needs you" need a Console (ADR 0031), and a CLI or MCP session that outlives one command needs a long-lived engine.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **The Daemon and the Console without Schedules or the Daemon group:** rejected. Collie users' repeated automations, such as a monthly download or a page watched for a change, would go to cron or a vendor's cloud, and a team running Collie on several pods would hold a Limit per pod; once the Daemon is there, both cost little.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

## Consequences

- **Amends ADR 0057.** Collie's executable carries V8 and the Script sandbox; DuckDB, the in-binary Sinks and `wreq` are in Foxhound's alone. A note sits under "What stays in the binary".
- **Amends ADR 0024.** A shorter note sits under its restatement of ADR 0057.
- **Amends ADR 0006.** The Session pool is Foxhound's, and a Session a program or an Agent opened is never rotated or retired. A note sits under the Session pool.
- **Amends ADR 0021.** Escalation retires and replaces only a Session pool's Session. On a program's Session on a Profile directory or a Borrowed browser, a Challenge goes on down ADR 0035's ladder without its step 3 and any other Block cause is reported on the call. Notes sit under its opening and its escalation rule.
- **Amends ADR 0035.** The ladder's step 3, and retirement by age and use count, apply only to a Session pool's Session. Notes sit under the step and under Saved identities.
- **Amends ADR 0040.** Retiring a failing Session is the Session pool's. A note sits under Session retirement.
- **Amends ADR 0045.** A Collie run never pays. A note sits under the opening.
- **Amends ADR 0053.** Collie carries the per-proxy limits. A note sits under them.
- **Amends ADR 0056.** Its Human pause for a failure that would rotate a Session on a Profile directory or a Borrowed browser holds for a crawl's Session. On a program's, a Challenge goes on down ADR 0035's ladder without its step 3 and any other Block cause is reported on the call. Notes sit under "One Session" and the Borrowed browser's Session pool.
- **Amends ADR 0086.** A program's own navigation through a proxy or a cloud-browser provider is held to their limits, and a Collie Run takes their settings. Notes sit under its bullet on a program's own navigation, under its settings table and under Collie Runs.
- **Amends ADR 0037's matrix.** Modular install (SDK-03), the Session pool (CO-12) and block detection with retry or rotation (ST-08) cite this ADR, and V3-01 is `decided`, which moves one Stagehand v3 row from `spec` to `decided` in the status counts.
- **Areas.** Area 1 (Repository, CI and release) builds Collie's executable without Foxhound's crates; area 3 (Engine runtime) builds the engine inside it; area 5 (Site access) builds the rule for Sessions a program opened; area 6 (Collection) builds the Session pool as a crawl's; area 7 (Plugin protocol) builds the Plugin kinds per Product; area 8 (Stealth and Challenge solving) builds the ladder without its step 3 for a program's Session; area 9 (AI and agent) builds V3-01; area 10 (Scheduling) builds Schedules in Collie; area 11 (Client surfaces) builds Collie's surfaces; area 12 (Console) builds Collie's Console; area 13 (Self-hosted deployment and operations) deploys Collie's Daemon, Exposed Daemon and Daemon group. Each lists this ADR.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Glossary.** A Session pool is a crawl's, not every run's; Provenance covers any result of a run, not only a Record, since Collie's results are not Records; and Foxhound's entry names Politeness toward sites and the HTTP fetch tiers, since Collie carries the per-proxy limits and the browser tiers.
- **Tests.** `TESTING.md` adds Session ownership and proxy limit conformance cases. Which suites run against each Product's executable is [Release, signing and testing of two products](https://github.com/goodfetch/goodfetch/issues/459)'s (ADR 0094).
- **Positioning.** Collie's docs can name what its competitors lack (the Effect journal, verified Plans, a Script sandbox) or keep in their clouds (stealth, Clean-up rules, Schedules), which Collie runs locally. How the site presents the two Products is [Docs site and positioning of two Products](https://github.com/goodfetch/goodfetch/issues/499)'s, and what two installed Products share is [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486)'s (ADR 0098).
  > Amended by [ADR 0097](0097-one-docs-site-renders-each-products-section-from-one-source-and-neither-product-leads-with-access.md): such claims about competitors appear only in migration guides and benchmark results, each with the competitor's version, the date and a source; Collie's lead names what runs on the user's machine without comparing.
