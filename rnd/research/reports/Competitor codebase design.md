# Competitors are weakest exactly where no tool checks them

We read the source of seven projects in Goodfetch's space at their heads of 15 October 2026 (Spider at 1 October): **Crawlee** for JavaScript and for Python, **Scrapy**, **Playwright**, **Stagehand**, **Browser Use** and **Spider**, the one competitor written in Rust. Each got a note covering its paradigm, organization, design patterns, separation of concerns and duplication, contracts, quality machinery, pain points, a scorecard and lessons for Goodfetch; the notes sit under [`research_notes/Competitor codebase design/`](../research_notes/Competitor%20codebase%20design/) and cite every claim with a permalink at the commit read. The method was static: reading, `tokei`, import-graph and line-similarity scripts. No code from the clones was built or run.

Three findings stand out. **First, the field is converging on Goodfetch's shape**, one protocol with SDKs generated from it, and increasingly an engine written in Rust: Playwright has driven every language from one Node engine for years, Stagehand v4 (August 2026) moved its runtime behind a JSON-RPC protocol with generated Python and Go models, Browser Use 0.13 (June 2026) shipped a Rust core in beta, and Apify started a Rust port of Crawlee on 30 September 2026. Most of them stumble at the seams of that shape: two hand-mapped contracts, a thick client re-written in each language, a 6,810-line hand-written bridge, runtime key-casing. **Second, the same four weaknesses recur in every codebase**: a small median file next to one hub class that absorbs every feature, copies along the backend axis that drift apart, an error model that ties with duplication as the lowest-scored dimension, and ambient state with implicit lifecycles that the crawlers fix late: Crawlee JS in its v4 major, Scrapy on its unreleased main branch after years of shutdown hangs, Crawlee Python still only in plans. None of the projects checks these with a tool. Where a project did write a check (Playwright's import rules and cancellation lint, Stagehand's cross-language parity tests, Crawlee's committed API reports), the property holds. **Third, Goodfetch's `STYLE.md`, `TESTING.md` and ADRs already go further than any of them on most of these points**; the gaps left are few, and the last section lists them.

## The scorecard

Each note scored its project from 1 to 5 on ten fixed dimensions, with a justification tied to evidence. The scores are one reader's judgement per project, so a one-point difference between projects means little; the pattern across rows means more.

| Dimension | Crawlee JS | Crawlee Py | Scrapy | Playwright | Stagehand | Browser Use | Spider | Mean |
| --- | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: |
| Layering and dependency direction | 3 | 3 | 2 | 4 | 4 | 2 | 2 | 2.9 |
| Module and file size discipline | 2 | 3 | 3 | 3 | 3 | 1 | 1 | 2.3 |
| DRY | 2 | 2 | 2 | 3 | 3 | 2 | 1 | **2.1** |
| Separation of concerns | 3 | 2 | 3 | 4 | 3 | 2 | 2 | 2.7 |
| Extension model | 4 | 4 | 4 | 3 | 3 | 3 | 3 | 3.4 |
| Contracts and types | 4 | 3 | 3 | 4 | 4 | 2 | 1 | 3.0 |
| Error model | 3 | 2 | 2 | 3 | 2 | 1 | 2 | **2.1** |
| Test architecture | 3 | 4 | 4 | 4 | 4 | 3 | 2 | 3.4 |
| Tooling and CI enforcement | 4 | 4 | 4 | 4 | 4 | 2 | 2 | 3.4 |
| In-code documentation | 4 | 4 | 4 | 3 | 3 | 3 | 4 | 3.6 |
| **Total (of 50)** | 32 | 31 | 31 | **35** | 33 | 21 | 20 | |

Read down the rows rather than across the columns. **Documentation, tooling, tests and extension models score highest**: these are what maintainers chose to build and turn on. **Duplication, errors and file size score lowest**: these are what nothing checks. Playwright scores best overall because it enforces its boundaries with tools. Browser Use and Spider score worst. Browser Use writes rules it does not enforce (split files past about 500 lines, never mock) and has 14 files over 1,000 lines ([note](../research_notes/Competitor%20codebase%20design/browser_use.md)). Spider shows that Rust's compiler does not prevent any of the four weaknesses on its own ([note](../research_notes/Competitor%20codebase%20design/spider_rs.md); see its own section below).

## The field converges on one protocol and generated SDKs, and stumbles at the seams

| Project | How other languages reach the core | Where it breaks |
| --- | --- | --- |
| Playwright | One Node engine, run as a "driver" over stdio; 19 YAML files define 34 interfaces and about 327 commands, from which typed client and server channels, runtime validators on both sides and .NET channels are generated; Python generates its protocol from the same spec and its public surface from the Markdown API docs | Two contracts mapped by hand (Markdown API, YAML protocol): one new method takes six manual steps. Timeouts, option normalization and waiting live in the client, so each port re-implements them; Python's hand-written page class is 56 KB. The protocol has no version: client and server must match major.minor, checked only from the User-Agent |
| Stagehand v4 | The runtime is a Chrome extension; SDKs speak JSON-RPC tunnelled through CDP. A Zod registry emits one JSON Schema, from which Python and Go models are generated; protocol SemVer is separate from package versions, with a typed handshake | The registry drives codegen but not dispatch: a 76-case hand-written router, and 70 of the runtime's 93 methods are pass-throughs. camelCase-to-snake_case conversion happens at run time, with lists of opaque keys and an acronym table. Every domain error crosses the wire as JSON-RPC `-32603` |
| Browser Use 0.13 | A Rust core in beta, reached through a stdio JSON-RPC client written by hand in Python | That client is 6,810 lines of plain dicts, excluded from type checking, with a 7,920-line test file; the old Python agent still ships beside it. MCP repeats 15 action schemas by hand |
| Crawlee (JS, Python) | Two codebases synced by hand, issue by issue; 14 Python files carry "Inspiration" links to old JS tags. Shared code: one Rust storage crate bound through napi-rs and maturin. A Rust port, [`apify/crawlee-rs`](https://github.com/apify/crawlee-rs), created 30 September 2026, replays golden files recorded by running Crawlee-JS as an oracle | Each feature is ported twice; the Python port lacks JS's retry-classifying error hierarchy (#1739); the wire's camelCase aliases are written by hand 94 times |
| Scrapy | Python only. Its new agent and MCP integration is the `RemoteControl` extension, on by default since 2.19, whose `/execute` endpoint runs any Python sent to it (localhost, bearer token) | An eval channel, not a typed protocol |

Sources: [Playwright](../research_notes/Competitor%20codebase%20design/playwright.md) §5–6, [Stagehand](../research_notes/Competitor%20codebase%20design/stagehand.md) §4–6, [Browser Use](../research_notes/Competitor%20codebase%20design/browser_use.md) §5 and §8, [Crawlee JS](../research_notes/Competitor%20codebase%20design/crawlee_js.md) §5, [Crawlee Python](../research_notes/Competitor%20codebase%20design/crawlee_python.md) §5, [Scrapy](../research_notes/Competitor%20codebase%20design/scrapy.md) §6. We checked the most consequential claims against the source ourselves: Stagehand's `packages/extension/manifest.json` and `packages/protocol/` with [PR #2671](https://github.com/browserbase/stagehand/pull/2671) (1,522 files, merged 10 August 2026); Browser Use's release "0.13.0 - Rebuilt in Rust [beta]" (8 June 2026) and its 6,810-line `browser_use/beta/service.py`; `crawlee-rs`'s `conformance/golden` and `conformance/oracle` folders; Scrapy's `REMOTE_CONTROL_ENABLED = True`.

The lesson is consistent: a single source for the contract only pays off when **everything** derives from it. Playwright's split between docs and protocol, Stagehand's hand-written router, Crawlee's options declared two or three times and Browser Use's hand-written bridge are four versions of the same failure. Goodfetch's design already makes the Rust types in `goodfetch-protocol` the one source (ADR 0038), keeps defaults and timeouts in the engine rather than in each SDK (ADR 0080), checks that each executable has exactly one handler per catalog operation (ADR 0087) and that every hand-written SDK member names its catalog entry (ADR 0099). Stagehand's other good ideas, protocol versioning in the handshake and drift checks on every committed artifact, are already ADRs 0030 and 0038 here.

## Every codebase has one hub class that absorbs each new feature

Median files are small: 114 to 184 lines in Crawlee Python, Playwright, Scrapy and Browser Use, with a mean of 168 in Stagehand; Spider's median, 335, is the exception. The trouble is the tail: one class per project that every feature passes through, and nothing that caps it.

| Project | Hub | Size |
| --- | --- | --- |
| Spider | `impl Website`; the files `website.rs`, `page.rs`, `utils/mod.rs` | one `impl` block of 13,435 lines with 361 functions, 310 of them public; 18,576, 14,520 and 12,147 lines, 55% of the core crate's source; functions of up to about 1,000 lines in the core crate (about 1,500 in its agent crate), nesting 26 levels deep |
| Browser Use | `beta/service.py`; `Agent`; `BrowserSession`; `DefaultActionWatchdog` | 6,810 lines; 98 methods and a 62-parameter constructor whose `**kwargs` swallows typos; 108 methods; 3,758 lines |
| Crawlee JS | `BasicCrawler` | 3,180 lines, a 400-line constructor, 5 to 8 generic parameters per crawler class |
| Stagehand | `understudy/Page`; `StagehandRuntime` | 2,770 lines and about 100 members, about 550 of the lines for one feature (WebMCP); 93 methods |
| Playwright | `Frame`; `InjectedScript` | about 1,350 lines and about 100 methods; about 1,740 lines |
| Crawlee Python | `BasicCrawler` | 1,898 lines, 63 methods, 30 constructor parameters, one 160-line method with six `except` branches |
| Scrapy | `crawler.py`; `engine.py` | 1,345 and 1,046 lines |

The hub is never designed as one. Crawlee's crawler owns retries, sessions, robots.txt, rate limits, statistics, root-logger setup, signal handling and CSV export because each was the easiest place to add the next feature; Stagehand's god object did not disappear in its v4 rewrite, it moved one layer down from the 2,381-line v3 `V3` class to the CDP `Page`. No project caps file or type size with a tool: Crawlee Python selects every ruff rule and then disables every size and complexity rule, and Browser Use writes a 500-line limit into its contributor guide without a check.

## Duplication runs along the backend axis, and the copies drift

Every project has an axis where several implementations sit side by side, and every project copies along it. The copies then diverge, so the duplication is not only cost but a source of bugs.

- **Browser drivers.** Crawlee JS's Playwright and Puppeteer helpers are 54% to 82% identical, and `infiniteScroll` already stops at `>=` in one and `>` in the other ([note](../research_notes/Competitor%20codebase%20design/crawlee_js.md) §5). Playwright's WebView backend copies its WebKit backend file by file, 50% to 98% identical once prefixes are normalized ([note](../research_notes/Competitor%20codebase%20design/playwright.md) §5).
- **Fetch versus browser.** Both Crawlee ports implement the navigation phase twice, once for HTTP and once for browsers; in JS the validation has drifted (only the browser checks `navigationTimeoutSecs > 0`), in Python the two share 72 identical lines.
- **Storage and HTTP clients.** Crawlee Python copies one warning block seven times across four storage backends, because the storage interface mirrors the Apify API's parameters, and its three HTTP clients each re-implement redirects and cookies (maintainers plan a shared base, #2307).
- **Model providers.** Browser Use's 17 LLM providers each re-map vendor errors; output-truncation detection exists in 3 of them.
- **Languages.** Stagehand keeps four CDP clients and four JSON-RPC peers across TypeScript, Python, Go and the extension, two of them in TypeScript alone, with an error helper that has already diverged.
- **Feature flags.** Spider holds 61 feature-gated variants of methods inside one `impl`, and at least ten crawl loops that are 47% to 70% identical (HTTP, Chrome, WebDriver, "smart" and `_send` copies made only to satisfy `Send`); users have reported the Chrome and HTTP paths behaving differently (#282, #201). Its four search providers are copied between two crates at about 90% similarity.

The fix the maintainers themselves move towards is the same each time: one shared layer above the implementations, with each implementation reduced to override points (Crawlee's template-method HTTP clients are 48 to 83 lines each), and **one conformance suite run against every implementation**, as Scrapy does for its download handlers and Crawlee Python for its four storage backends.

## Errors tie with duplication as the lowest-scored dimension

Crawlee JS 3, Playwright 3, Crawlee Python 2, Scrapy 2, Stagehand 2, Spider 2, Browser Use 1. The failures repeat:

- **No common base and no codes.** Crawlee Python has 14 exception classes, Scrapy about 19, Stagehand at least 19; none has a shared root or a stable code.
- **Classification by message text.** Crawlee JS detects proxy errors by substring, Stagehand recognizes CDP errors by message ("Cannot find context with specified id"), Browser Use classifies by substring and flattens errors into `RuntimeError` strings, with 526 broad `except Exception` handlers, 46 of them a silent `pass`.
- **Reconstruction by name across a wire.** Playwright's client rebuilds known error classes by matching the name string, with copies of each class on both sides; Stagehand sends every domain error as `-32603` with a name, and its TypeScript, Python and Go SDKs expose three different error types.
- **Failures disguised as data.** Spider returns crawl failures as invented HTTP statuses (525 for a DNS failure, 599, 310, 503) that collide with real server codes, and its `Website::build()` returns `Result<Self, Self>`, so the error type is the crawler itself.
- **Regression.** Stagehand v3 had a 55-class hierarchy; v4 lost it.

Three projects show good parts. Crawlee JS encodes retry semantics in its class hierarchy (`NonRetryableError`, `CriticalError`, `SessionError`, `RetryRequestError`). Playwright declares typed error details per command in its protocol and turns every "target closed" protocol error into one `TargetClosedError` carrying the close reason. Scrapy's download handlers (HTTP/1.1, HTTP/2, aiohttp, httpx, FTP) all raise one transport-neutral family (`DownloadTimeoutError`, `CannotResolveHostError` and the rest). Goodfetch's model (one Error code per remedy, a retryable flag and a class per code generated into every SDK, ADRs 0038 and 0079) is already what all of them lack on the outbound side. The **inbound** side is where most of them fail and what Goodfetch's rules do not yet state: CDP and proxy failures arrive as text, and Crawlee, Stagehand and Browser Use match that text wherever they happen to need it, where Scrapy and Playwright normalize part of it once.

## Ambient state and implicit lifecycles get fixed late

- **Service locators.** Crawlee JS reaches configuration, storage, events and the logger through a global `Proxy` over `AsyncLocalStorage`, from 75 call sites in 36 files; to give each crawler its own scope it rewrites every method of the instance. The impit HTTP adapter depends on the `core` package only to log, and `browser-pool` depends on it for the locator and two error classes. The locator's semantics took four iterations across the two ports (#3710). Crawlee Python has a global locator and a per-crawler one, which produced duplicate event managers (#2242) and a second crawler silently reusing the first one's queue (#1653).
- **Lifecycle by flag.** Crawlee Python's crawler enters any collaborator not yet `active`, so shared instances break; the fix is planned, not shipped (#2301). Crawlee JS retrofitted owned-versus-injected collaborators (`OwnedOrInjected`) and a per-run `CrawlerRun` object for its v4 major, now in release candidate, after a teardown bug that made multi-run tests "process nothing" (#3087, closed in September 2026). Scrapy added an engine state machine to its main branch, not yet released, after years of hangs on early shutdown (#6916), and an invalid transition still only logs a warning.
- **Import-time side effects.** Importing Browser Use sets up global logging, monkeypatches asyncio and loads `.env` from six modules; building a profile creates temporary directories, which leaked 7,741 of them (14 GB) for one user (#5422). Its three dozen fire-and-forget tasks are cancelled by a `__del__` that matches attribute names.
- **Hidden runtimes and orphan tasks.** Spider's library starts a second Tokio runtime with one thread per CPU and never joins it, keeps 17 bare `tokio::spawn` calls in the core, and reads about 40 environment variables deep inside the library.
- **Observability by patching private code.** Both Crawlee ports instrument OpenTelemetry by monkeypatching private methods by name, so the core must keep those methods private but stable, with lint suppressions saying so.

Goodfetch's rules close most of this: every dependency is injected, never reached through a global; time and randomness come from handles; environment reads are banned outside `main`; every task starts under its owner's tracker; state with one owner is an actor; an SDK program closes only what it started (ADR 0078). The lesson not yet fully written down is to enforce lifecycle **transitions** in code, not only owners: a Run's states are decided (ADR 0029), but not how an invalid transition is refused, nor the states of a Session, a page and the Engine.

## Boundaries hold only where a tool checks them

| Check | Project | Effect |
| --- | --- | --- |
| 51 `DEPS.list` files enforced by `check_deps.js` in `npm run lint`; only `server/playwright.ts` may import the browser backends | Playwright | The client/server split and the composition root hold. Gaps: type-only imports are exempt, and five rules name files that no longer exist without the checker noticing |
| `progress/await-must-use-progress`, a type-aware lint: in a function that takes a `Progress`, every awaited call must pass it on or race it | Playwright | Cancellation is threaded through the whole server by construction |
| Nine ast-grep suites: every method called by every SDK and routed once, every field constructed and consumed, docs in sync, every protocol object strict | Stagehand | Four languages stay in parity despite hand-written runtimes |
| Committed API Extractor reports, diffed in CI, with "exported means promised unless `@internal`" | Crawlee JS | Public-surface drift becomes a reviewable diff |
| No import-boundary check (Scrapy has only one ruff ban on module-level imports) | Scrapy, Crawlee JS, Browser Use, Spider | Scrapy's `utils` sits in one import cycle spanning 17 packages and modules, and 12 of its public `utils` modules are now deprecated shims; Crawlee JS has four cycles inside packages and its abstraction package imports a concrete storage backend; Browser Use's `agent` and `tools` import each other; Spider's crawler depends on its agent crate while satellite crates depend back on the crawler, and 32 of its 33 `utils` modules are public |

## Spider shows what each of Goodfetch's Rust rules prevents

Spider is the one competitor in Goodfetch's language, with about 110,000 lines of Rust code in a ten-member workspace, eight of whose crates are published. Almost every hard rule in `STYLE.md` has a counterpart failure in it, which makes it a useful answer to "is this rule worth its cost?" ([note](../research_notes/Competitor%20codebase%20design/spider_rs.md) §10; the counts below were checked against the clone).

| Goodfetch rule | Spider at `fb0f270` |
| --- | --- |
| A capability is a setting, never a Cargo feature (ADR 0038) | About 128 features in the core crate and about 2,470 feature conditions. Features change public field types, method signatures and derived traits: `Page::error_status` is a `String` or an `Arc<reqwest::Error>` depending on a feature, and turning on `regex` removes `PartialEq` from the configuration. Without `chrome`, `with_stealth(true)` silently does nothing. Builds that fail under some feature combination are a recurring class of bug in its tracker (#349, #348, #260, #245) |
| `cargo-hack` checks every feature combination | Clippy checks two feature configurations out of 128 features |
| No `unwrap()`; `expect("invariant: …")` only, and anything input can trigger returns an error; `unsafe` denied, each block with a `// SAFETY:` comment | 25 plain `unwrap`s in the core, but `unwrap_unchecked()` on a fallible HTTP client build inside `unsafe` seven times in `website.rs`, including the public `configure_http_client`: undefined behaviour if proxy or TLS settings make the build fail. `website.rs` holds 9 `unsafe` uses and no `SAFETY` comment |
| `#[expect(lint, reason)]`, never `#[allow]` | 25 crate-wide allows, starting with `#![allow(clippy::perf)]`, 80 local ones, no `#[expect]` |
| No task outside its owner's tracker; only `main` builds a runtime | 17 bare `tokio::spawn` in the core; the library starts its own second runtime |
| Typed `thiserror` errors per crate, mapped to Error codes | `thiserror` is declared and never used; failures become fake HTTP statuses |
| cargo-semver-checks; an unannounced break blocks (ADR 0030) The main branch carries breaking changes as 2.54.0, a minor version; `spider` has 2,227 versions on crates.io |
| A build script downloads nothing | The `benches` crate's `build.rs` runs `apt-get`, `go get` and `npm install` |
| MSRV declared and tested | No `rust-version` in any manifest |
| Only three writers touch stdout and stderr | **Spider complies**: library code never prints, and its MCP server sends logs to stderr |

Spider also has good ideas worth taking: small strategy traits behind `Arc<dyn Trait>` for retry, proxy and fetch engine; a data-only types crate with no heavy dependencies, which mirrors `goodfetch-protocol`; private modules with a curated `pub use` façade in its agent crate; and a dedicated-thread actor that lets code which cannot be sent between threads be used without locks, a template for CDP state that is not `Send`. Its CI enforces `missing_docs` through Clippy with `-D warnings`, which is why documentation is its best dimension.

## Patterns worth adopting

| Pattern | Seen in | Goodfetch status |
| --- | --- | --- |
| Deferred commit: a handler's writes are staged and committed only when it succeeds, so a retry never duplicates output | Crawlee JS and Python | Decided: records staged with their attempt (ADR 0103), effect journal (ADR 0106) |
| One deadline object per call, shared by every step, with late-result cleanup | Playwright, Stagehand | Partly: nested timeouts and `deadline_exceeded` naming its setting (ADR 0080) and child cancellation tokens (ADR 0038) are decided; late-result cleanup only for a model handler's late answer (ADR 0101) |
| Protocol version independent of package version, negotiated in the handshake | Stagehand | Decided: `hello` and the version check (ADRs 0030, 0038) |
| Drift checks on every generated or committed artifact | Playwright, Stagehand | Decided: `xtask codegen --check` in the Quick suite |
| Cross-language parity tests | Stagehand | Decided: completeness tests and the hand-written member manifest (TESTING.md, ADR 0099) |
| Committed public-API reports diffed in CI | Crawlee JS | Decided: cargo-semver-checks, API Extractor, `griffe check` and a catalog diff (ADR 0030) |
| Golden-file conformance across implementations | crawlee-rs | Decided: the Scripted engine and `goodfetch-conformance` (ADR 0069) |
| Composition root: one place names the concrete backends | Playwright | Decided: each Product's executable crate is its composition root (ADR 0087) |
| Per-command metadata (snapshot, pause, title) read by the dispatcher and tracing, so handlers hold no tracing code | Playwright | Partly: each catalog entry already declares its class, whether it can be cancelled and its markers (ADR 0038); behaviour flags read by dispatch and tracing are not decided |
| Typed middleware pipeline building each request's context, cleanups in reverse order, setup failures told apart from handler failures | Crawlee JS and Python | Partly: hooks declared per stage, each returning its change (ADR 0104) |
| CDP target and session state owned by one component fed by `Target.attachedToTarget` and `detachedFromTarget` | Browser Use | Not decided; a Collie design input |
| Fail-open intercept: an optional subsystem (cache, self-heal) wraps the call and can never fail it | Stagehand | Decided the other way for Collie's AI calls: a call replays from its Plan, gets at most one Repair and otherwise fails with `plan_drift`, on purpose (ADR 0091). Whether an optional store such as the Response cache (ADR 0046) may ever fail a call is not decided |
| Versioned persisted formats with frozen per-version types and a chain of upgraders | Playwright (trace V3 to V10) | Decided: an engine reads every older minor of a format within its major, and `goodfetch upgrade-files` upgrades every contract format at a major (ADR 0030) |
| One construction contract for every component, with a typed "disabled" outcome | Scrapy (`from_crawler`, `NotConfigured`) | Partly: each domain crate exports one value listing what it adds (ADR 0087) |
| Trace context (`traceparent`) carried in the protocol envelope, one server span per request | Stagehand | Not found in any ADR |

## What this means for Goodfetch's quality bar

**Where the rules already go further than every competitor.** Contracts come from one source and everything derives from it, including the check that each operation has exactly one handler; errors carry codes and generated classes in every SDK; one async runtime with structured cancellation and no task outside a tracker; only three writers touch stdout and stderr; no globals, no environment reads outside `main`, time and randomness injected; capabilities are settings, never Cargo features; suppressions use `#[expect]` with a reason; API breaks are diffed per language. Each of these rules closes a failure documented above.

**Already implied by `STYLE.md`.** `STYLE.md` turns on Clippy's `pedantic` group with warnings as errors. We checked with the installed Clippy that `pedantic` enables `too_many_lines`, which flags functions over 100 lines, and that `too_many_arguments` (more than seven) is on by default. Once the workspace exists, function size is capped unless the `[workspace.lints]` allow-list exempts these two lints.

**Gaps worth deciding**, each tied to a failure the field shows:

1. **File and type size.** Nothing caps a module's length or a type's number of methods, and that is exactly where every competitor's hub grew (the table above). Clippy has no lint for either. Options: an `xtask` check with budgets per file and per `impl`, a review rule, or both.
2. **Module boundaries inside a crate.** Crates enforce direction between crates, and several rules already pin one concern to one module: sockets to `transport`, spawning to `tasks`, clocks and randomness to `Clock` and `Entropy`, environment reads to `main`, the std sync types to `sync`. Nothing states in general which modules of a crate may import which. Playwright's `DEPS.list` is the model, including its two gaps to avoid: exempting type-only imports and keeping rules for files that no longer exist.
3. **Inbound errors classified once, where they arrive.** CDP, HTTP and proxy failures arrive as codes and text; Crawlee, Stagehand and Browser Use match that text wherever it is needed, while Scrapy's download handlers and Playwright's closed-target errors show the fix in part. A rule could require the CDP client and each Fetch backend to map upstream failures to typed variants in one place, with a test per known message, and forbid matching on error text anywhere else.
4. **One shared layer above parallel implementations.** Half of this is decided: every contract with several implementations has a conformance suite as its interface test (`TESTING.md`; one suite for Sinks, ADR 0026, extended to every Plugin kind, Fetch backends included, by ADR 0033). For the fetch-versus-browser axis, ADR 0093 already gives a crawl handler one context and Collie's page on every fetch tier. The other half is not decided in general: that high-level operations sit once above the implementations, and each implementation only fills override points. The competitors' 50% to 98% copies, and their drift, came from that half, since Crawlee and Scrapy both run shared suites over their backends.
5. **Lifecycle transitions enforced in code.** Ownership is decided, and so are a Run's Lifecycle states (`running`, `stopped`, `finished`, `failed`, ADR 0029) and how a Run resumes or is refused while another writer holds it (ADR 0102). Not decided: how code enforces transitions, and the states of a Session, a page and the Engine. Scrapy and Crawlee both added a state machine or a per-run object only after shipping hang and teardown bugs, and Scrapy's still only warns on an invalid transition. Typestates, or a transition table where an invalid transition is an error, would close it.
6. **Trace context in the envelope.** If spans should follow a call from an SDK into the engine and on to a Browser host or Goodfetch Cloud, the protocol needs a place for `traceparent`; we found no ADR that decides it.
7. **Stable span names.** Most of this is decided: log text, like error message text, is outside semver (ADR 0079); logger names and `tracing` targets are SDK API under semver, under `goodfetch::…` (ADR 0078); metric names are generated from `goodfetch-protocol`, each Stable or Experimental under semver (ADR 0050). Spans per request, run, page and CDP command are decided with glossary IDs as fields (`STYLE.md`). Not decided: whether span names and span fields, and the Engine's own targets in an OpenTelemetry export, are a contract, although an exported trace is what an OpenTelemetry user builds dashboards on.

Spider's `unsafe` `unwrap_unchecked()` is not a gap: `STYLE.md` denies `unsafe_code` workspace-wide, admits it only in modules that name their reason, and requires that anything input can trigger returns an error.

## Conclusion

The competitors' architectures are mostly sound, and several have converged on the shape Goodfetch chose. Their code quality falls short in the same four places in every repository, and in each case the cause is that nothing checks the property: size, duplication, inbound errors and lifecycle state. The properties they did check, through custom lints, import rules, parity tests and API diffs, held even in large and old codebases. For Goodfetch, the practical conclusion is that the quality bar should be written as checks before the first crate exists: most of it already is, and the seven gaps above are the candidates for the next decisions.
