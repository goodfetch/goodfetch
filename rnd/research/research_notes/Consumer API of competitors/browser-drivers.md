# The consumer API of non-AI browser drivers: Playwright, Puppeteer, Selenium/BiDi, Rust crates, and what users like and hate

Research of **October 14, 2026** for Goodfetch's consumer developer API: **Collie** (one page: a Playwright-shaped page API with guarded actions, plus AI primitives) and **Foxhound** (crawling at scale), with TypeScript, Python and Rust SDKs over one Rust engine via JSON-RPC (ADR 0019, ADR 0038). Slice: browser driver and automation libraries without AI. Like everything in `rnd/`, this note informs a decision; it does not make one.

## 1. Sources and method

**Pinned versions (checked 2026-10-14).**

| Library | Version read | Released | Where read |
|---|---|---|---|
| Playwright (JS) | 1.64.1 | 2026-10-14 | source tree and `docs/src` at tag [`v1.64.1`](https://github.com/microsoft/playwright/tree/v1.64.1) (sparse clone) |
| playwright-python | `main` at [`27e324a`](https://github.com/microsoft/playwright-python/commit/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc), driver 1.64.0 | 2026-10-13 | full clone |
| playwright-java / playwright-dotnet | 1.63.0 | 2026-09-14 / 2026-09-23 | GitHub API (generator sources, issues) |
| Puppeteer / puppeteer-core | 25.13.0 | 2026-10-08 | tag [`puppeteer-v25.13.0`](https://github.com/puppeteer/puppeteer/tree/puppeteer-v25.13.0) (sparse clone), `puppeteer-core` CHANGELOG |
| Selenium (all bindings) | 4.51.0 | 2026-10-09 | docs, decision records, tag `selenium-4.51.0` |
| WebDriver BiDi | W3C Working Draft | 2026-10-08 | [w3.org/TR/webdriver-bidi](https://www.w3.org/TR/webdriver-bidi/) |
| chromiumoxide / headless_chrome / fantoccini / thirtyfour | 0.9.1 / 1.0.22 / 0.22.1 / 0.37.8 | 2026-02-25 / 2026-06-11 / 2026-02-28 / 2026-10-12 | crates.io archives at their recorded commits |
| playwright-rs / rustwright / chromey | 0.20.0 / 0.3.0 / 2.58.2 | 2026-10-11 / — / 2026-09-12 | crates.io archives |
| Cypress / WebdriverIO | 16.1.1 / 10.0.2 | 2026-09-29 / 2026-10-08 | docs, release blogs, issues |
| Patchright / rebrowser / nodriver / zendriver | 1.63.0 / 1.0.19 (patches) / 0.50.6 / 0.17.1 | see §2.7 | READMEs, issues, source |

**Method.**
- Primary sources first: official docs (read from the repositories' Markdown sources where they exist, which for Playwright are also the input of the API generators), source at the tags above, release notes and CHANGELOGs, and issue trackers with maintainer replies. Defaults (timeouts, polling intervals, strictness) were confirmed in source, with links to the file.
- GitHub issues: top 50–100 per repository by 👍 (`sort=reactions-+1`), open and closed, plus keyword searches (`ElementHandle`, `waitForNavigation`, `strict mode violation`, `memory leak`, `isVisible`, `cancel`, `thread`, …). Declined requests were opened and the maintainer rationale captured. Counts are 👍 unless "total reactions" is said.
- GitHub Discussions are **disabled** on `microsoft/playwright`, `microsoft/playwright-python` and `puppeteer/puppeteer` (GraphQL `hasDiscussionsEnabled: false`), so there is no Discussions evidence for them.
- Community: Stack Overflow through the Stack Exchange API (375 top-voted questions over `playwright`, `playwright-python`, `playwright-test`, `puppeteer`, clustered; plus Selenium and Cypress top questions), Hacker News through the Algolia API (about 21 threads and 2,000 comments read for Playwright/Puppeteer, plus Selenium, Cypress and Rust-crate threads), users.rust-lang.org, Ministry of Testing Club, discuss.python.org, the Streamlit and CrewAI forums, and nine migration or critical blog posts.
- **Reddit could not be reached**: `reddit.com` and `old.reddit.com` returned HTTP 403 to `curl`, the fetch tool refuses the domain, `site:reddit.com` searches returned nothing, and mirrors sit behind proof-of-work challenges that were not bypassed. Reddit claims appear only second-hand and are marked **unverified**. Exa was not used.
- The work was split into four parallel strands (Playwright and Puppeteer source and issues; forum evidence; Selenium, Cypress, WebdriverIO and stealth forks; Rust crates). Load-bearing claims from each strand were re-checked at the source before inclusion: Selenium ADR 17670's wording, the Selenium #15430 reply, Cypress #1417's counts (429 👍, 492 total reactions), WebdriverIO v10's strict `$` rationale, the Playwright #18266 reply, chromiumoxide's hard-coded `REQUEST_TIMEOUT` and thirtyfour's 20 s / 500 ms poller.
- Code samples are our own, written against the docs and source at the versions above; none was compiled or run.
- No personal data was put in any query.

**Prior notes reused and re-checked**: [browser frameworks](2026-10-07-browser-frameworks.md) and [Reddit follow-up](2026-10-07-reddit-evidence-followup.md) (little driver-API content), [browser-automation products](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) (packaging, install sizes, attach modes, which are not repeated here), [Rust CDP clients](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md) (protocol revisions, OOPIFs, maintenance) and [driver interop](https://github.com/goodfetch/automan/blob/research/driver-interop/rnd/research/2026-10-14-driver-interop.md) (attaching to a lent page).

**Reading guide.** §2 is per library: walkthrough, sample, the requested dimensions, and that library's own evidence. §3 compares them in one table. §4 draws the common patterns, divergences and multi-language generation. §5 and §6 rank likes and pain points **across** libraries; §6 is the section to read for design lessons. Appendix A holds the full Stack Overflow and Hacker News evidence for Playwright and Puppeteer.

## 2. Per library

### 2.1 Playwright (microsoft/playwright 1.64.1; Python 1.64.0; Java and .NET 1.63.0)

**Pinned references.** Source tree at tag [`v1.64.1`](https://github.com/microsoft/playwright/tree/v1.64.1) (released 2026-10-14), playwright-python `main` at [`27e324a`](https://github.com/microsoft/playwright-python/commit/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc) (2026-10-13, `DRIVER_VERSION` = `1.64.0-beta-1791568591000`), playwright-java and playwright-dotnet `main` read through the GitHub API on 2026-10-14. Docs quoted are the Markdown sources in `docs/src` at `v1.64.1`, which are also the input of the API generators (section 4).

#### 2.1.1 Walkthrough

**Object model.** `Playwright` → `BrowserType` (`chromium`, `firefox`, `webkit`) → `Browser` → `BrowserContext` → `Page` → `Frame`, with `Locator`, `FrameLocator`, `Request`/`Response`/`Route`, `APIRequestContext`, `Tracing`, `Download`, `Worker`, `Dialog`, `ConsoleMessage` around them. The context is the isolation unit (cookies, storage, permissions, proxy, viewport, `route()`); a page belongs to one context; `launchPersistentContext(userDataDir)` returns a context that *is* the browser.

**Launch vs connect.**
- `browserType.launch(options)` starts a browser over a pipe; `launchPersistentContext` with a profile directory.
- `browserType.connect(wsEndpoint)` to a `launchServer()` / `npx playwright run-server` / `browser.bind()` endpoint: full fidelity, but "the major and minor version of the Playwright instance that connects needs to match the version of Playwright that launches the browser (1.2.3 → is compatible with 1.2.x)" ([class-browsertype.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md)).
- `browserType.connectOverCDP(url)`: Chromium only, lower fidelity; `isLocal` (1.58), `noDefaults` (1.60), `artifactsDir` (1.61). Already covered in [driver-interop](https://github.com/goodfetch/automan/blob/research/driver-interop/rnd/research/2026-10-14-driver-interop.md) and [browser-automation-products §3.3](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md).
- `browser.bind(title)` (1.59) publishes a launched browser for `playwright-cli`, `@playwright/mcp` and other clients; "Multiple clients at a time are supported" ([release notes 1.59](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)).

**Closing.** Closing a parent closes its children: "closing browser closes all contexts, closing context will close all pages in it… If you close playwright it will automatically close all browsers" (maintainer, [playwright-java#1406](https://github.com/microsoft/playwright-java/issues/1406), 2023-10-12). Since **1.59** (2026-04-01) many APIs return async disposables, so `await using page = await context.newPage()` and `await using route = await page.route(...)` work; `page.route()` and `addInitScript()` return a `Disposable` that removes the handler or script ([release notes 1.59](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)). Python: `Page`, `BrowserContext`, `Browser` and `Disposable` support `with`/`async with` and call `close()` on exit ([python_rules.js](https://github.com/microsoft/playwright-python/blob/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc/scripts/codegen/python_rules.js)). Java implements `AutoCloseable` ([playwright-java#237](https://github.com/microsoft/playwright-java/issues/237), completed 2021). .NET: `IAsyncDisposable` only; a request for `IDisposable` was closed not planned ([playwright-dotnet#2221](https://github.com/microsoft/playwright-dotnet/issues/2221)).

**Locators and auto-waiting.** A `Locator` is a lazy, re-resolved description ("a way to find element(s) on the page at any moment", [locators.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/locators.md)). History:
- **1.14** (2021-08-13): `Locator` introduced, "strict by default", next to `ElementHandle` ([release notes 1.14](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)).
- **1.17** (2021-11-30): `FrameLocator`, also strict.
- **1.27** (2022-10-07): `getByRole`, `getByText`, `getByLabel`, `getByPlaceholder`, `getByAltText`, `getByTitle`, `getByTestId`.
- Later: `filter({ has, hasNot, hasText, hasNotText, visible })`, `and()`, `or()`, `all()` (1.29), `describe()`, `normalize()` and `Page.pickLocator` (1.59), `Locator.visible()` and selector-less `page.frameLocator()` searching every frame (1.63), `Locator.within()` and `Page.getByRef()` (aria-snapshot refs) (1.64).
- `ElementHandle` is not removed but carries a "Discouraged" banner: "The use of ElementHandle is discouraged, use Locator objects and web-first assertions instead." Every `page.click(selector)`-style method is likewise annotated `discouraged: Use locator-based … instead` ([class-elementhandle.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-elementhandle.md), [class-page.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-page.md)).

Actionability ([actionability.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/actionability.md)): before `click` the locator must resolve to exactly one element that is Visible, Stable, Receives Events (hit target) and Enabled; `fill` needs Visible, Enabled, Editable; `focus`, `press`, `setInputFiles`, `dispatchEvent` check nothing. `force: true` skips the non-essential checks; `trial: true` runs the checks without acting. In the server, a failed check is retried with a back-off of `[0, 20, 100, 100, 500]` ms and, for pointer actions, the scroll alignment cycles through four `scrollIntoView` variants to escape sticky headers ([dom.ts `_retryAction`](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/dom.ts)). A `scroll: 'auto' | 'none'` action option arrived in 1.62.

**Strictness.** "Locators are strict… all operations on locators that imply some target DOM element will throw an exception if more than one element matches" ([locators.md §Strictness](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/locators.md)). The violation is thrown at once, not waited out: the error is built in the injected script (`strictModeViolationError`) and a request to wait for the ambiguity to resolve is open ([#36045](https://github.com/microsoft/playwright/issues/36045), +12, 2025-05-22); a 2024 bug asking the same was closed in a stale-issue sweep after the maintainer suggested `await expect(locator).toHaveCount(1)` first ([#30363](https://github.com/microsoft/playwright/issues/30363)). `first()`, `last()`, `nth()` opt out and are documented as "**not recommended**".

**Semantic locators.** The docs rank `getByRole` first ("explicit and implicit accessibility attributes"), then text, label, placeholder, alt text, title, test id; CSS/XPath are "not recommended" for resilience ([locators.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/locators.md)). `getByText` defaults to substring, case-insensitive matching (`exact: false`), unlike Testing Library; a global switch is requested ([#19235](https://github.com/microsoft/playwright/issues/19235), +25, open). A `getById` request was declined with "Can you use `locator('#id')` instead?" ([#19763](https://github.com/microsoft/playwright/issues/19763), +81, closed not planned 2026-05-08).

**Assertions.** Web-first assertions (`expect(locator).toBeVisible()`, `toHaveText`, …) retry until a timeout (5 s default). In JavaScript they live in the test runner (`@playwright/test`): the library docs' comparison table says the Library has "No built-in Web-First Assertions" ([library-js.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/library-js.md)). In Python, Java and .NET the same assertions ship inside the library package (`from playwright.sync_api import expect`, `PlaywrightAssertions.assertThat`). One-shot state reads such as `isVisible()` do not wait: the old `timeout` argument "is ignored", and users are told to use `expect(locator).toBeVisible({ timeout })` instead ([#12224](https://github.com/microsoft/playwright/issues/12224), +20); a request to make that deprecation visible or remove the ignored argument was closed not planned because removing it "will not help" ([#33017](https://github.com/microsoft/playwright/issues/33017)).

**Configuration and defaults.** Plain options objects per call (`launch({ headless, channel, args, proxy, … })`, `newContext({ viewport, locale, storageState, recordVideo, … })`), `devices[...]` descriptors spread into context options, `setDefaultTimeout`/`setDefaultNavigationTimeout` on page and context. Defaults ([params.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/params.md), [test-timeouts-js.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/test-timeouts-js.md)):

| Timeout | Library | Test runner |
|---|---|---|
| Action / navigation | 30,000 ms ("pass `0` to disable") | "no timeout" (actions and navigations), bounded by the test timeout |
| Test | n/a | 30,000 ms |
| `expect` | 5,000 ms (Python/Java/.NET library assertions) | 5,000 ms |

Since **1.62** most operations and assertions also take an `AbortSignal` (`signal` option): "Providing a signal does not disable the default timeout; pass `timeout: 0` to disable it" ([release notes 1.62](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)). On the wire the timeout travels in per-call `metadata`, not in params, and an abort sends an `__abort__` message for the call id ([connection.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/connection.ts)). The `signal` options are annotated `langs: js` ([params.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/params.md)): Python, Java and .NET do not expose this cancellation, and the Python docs still call task cancellation undefined behaviour (below). Python accepts `float` milliseconds or `datetime.timedelta` for every `timeout` ([_helper.py `to_milliseconds`](https://github.com/microsoft/playwright-python/blob/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc/playwright/_impl/_helper.py)). Environment variables are for installation and tooling only (`PLAYWRIGHT_BROWSERS_PATH`, `PLAYWRIGHT_DOWNLOAD_HOST`, `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD`, `PLAYWRIGHT_NODEJS_PATH`, `PWDEBUG`, `DEBUG=pw:api`, `PLAYWRIGHT_DASHBOARD`); no environment variable sets a default timeout (count of `PLAYWRIGHT_*` mentions in `docs/src`).

**Events and network.**
- Event emitters (`page.on('request' | 'response' | 'console' | 'popup' | 'download' | 'dialog' | …)`), plus "wait for event" helpers. The same "arm the waiter before the trigger" idea has four idioms ([events.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/events.md)):
  - JS: `const p = page.waitForResponse(url); await click(); await p` ("Note no await").
  - Python: `with page.expect_response(url) as info: …; info.value`.
  - Java: `page.waitForResponse(url, () -> { … })` (callback scopes the trigger).
  - .NET: `page.RunAndWaitForResponseAsync(async () => …, url)`.
  No async-iterator event API exists in any language.
- Where callbacks run: event predicates (`waitForResponse(r => …)`) and `route` handlers run **in the client process**, on events the server streams over the protocol ([client/page.ts `waitForResponse`](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/page.ts)), so they can close over host variables; only `evaluate(fn, arg)` and friends run in the page, and those take an explicit serialisable `arg`. Declarative filters (`filter({ hasText })`, `getByRole(…, { name })`) are resolved by the injected script without callbacks.
- Interception: `page.route(glob|RegExp|predicate, handler)` / `context.route`, with `route.fulfill`, `route.continue`, `route.abort`, `route.fallback` (chain to the next handler) and `route.fetch` (perform the request, then modify the response). `routeFromHAR` and `tracing.startHar` (1.60), `routeWebSocket`. "Enabling routing disables http cache" ([class-page.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-page.md)); `route` does not see requests served by a Service Worker unless `serviceWorkers: 'block'`.
- Recent pull APIs: `page.requests()`, `page.consoleMessages()`, `page.pageErrors()` (1.56), `Request.existingResponse()` (1.59).

**Errors.** Three classes across languages: `Error`, `TimeoutError`, `TargetClosedError` (Python: [_errors.py](https://github.com/microsoft/playwright-python/blob/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc/playwright/_impl/_errors.py)). The client prefixes the message with the API name (`locator.click: …`), replaces the stack with the user's frames (the server-side stack is appended only when `PWDEBUGIMPL=1`, [stackTrace.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/utils/stackTrace.ts)), and appends the server's **call log**: lines such as `waiting for getByRole('button')`, `attempting click action`, `element is not visible`, `<div …> intercepts pointer events`, `retrying click action`, `waiting 100ms` ([channelOwner.ts `_wrapApiCall`](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/channelOwner.ts), [dom.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/dom.ts), [connection.ts `formatCallLog`](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/connection.ts)). The server knows the client's language (`SDKLanguage` = `javascript | python | java | csharp` in [core.yml](https://github.com/microsoft/playwright/blob/v1.64.1/packages/protocol/spec/core.yml)) and renders locators in that language inside errors (`asLocator(this._sdkLanguage, …)` in [injectedScript.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/injected/src/injectedScript.ts)), so a Python user reads `get_by_role("button")`. Requests for a richer hierarchy were declined: "As a work around… convert the thrown exception to a string and match based on its contents" ([playwright-python#1425](https://github.com/microsoft/playwright-python/issues/1425), +12, not planned); Python's `TimeoutError` shadows the builtin and was not renamed because "Playwright cannot break its API" ([playwright-python#1495](https://github.com/microsoft/playwright-python/issues/1495), +12, not planned 2026-04-30).

**Remote object lifetime.** Every server object has a `guid`; the server creates, adopts and disposes client mirrors with `__create__`, `__adopt__`, `__dispose__` events. To cap memory, each object type is an LRU bucket of 10,000 entries (100,000 for `JSHandle`/`ElementHandle`); past that the oldest 10 % are disposed with reason `gc`, and a later call on one fails with "The object has been collected to prevent unbounded heap growth" ([dispatcher.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/dispatchers/dispatcher.ts), [connection.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/connection.ts)). This was the fix for [#6319](https://github.com/microsoft/playwright/issues/6319) (+60, "Memory increases when same context is used", 2021–2023): "The heap will still saturate to a certain size… but will stop growing afterwards" (pavelfeldman, 2023-09-26).

**Long-lived contexts.** Playwright's stated design centre is testing: "Playwright is focused on testing scenarios, where contexts are constantly closed and re-opened on a per-test basis. We store some useful metadata inside the context to aid with test debugging… If the context is not closed, then all the debugging metadata is piling up" (aslushnikov, [#15400](https://github.com/microsoft/playwright/issues/15400), +24, 2022-07-06). The recommended workaround is to recycle contexts.

**Tracing and debugging.**
- Trace Viewer (`tracing.start({ screenshots, snapshots })` / `tracing.stop({ path })` in the library; `trace: 'on-first-retry' | 'retain-on-failure' | …` in the runner); since 1.63 traces can record aria and screen snapshots per action; `npx playwright trace open|actions|action|snapshot` is a CLI for agents (1.59).
- Inspector via `PWDEBUG=1` or `page.pause()`; codegen (`npx playwright codegen`, emitting JS, Python sync/async/pytest, Java/JUnit, C#/NUnit/MSTest/xUnit); UI Mode and the HTML report are runner-only; `DEBUG=pw:api` prints API logs.
- The client sends each call's user stack frame (`location: {file, line, column}`) and `title` in `metadata`, which is what lets the trace and the inspector point at the user's source line ([connection.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/connection.ts)).
- Language bindings cannot route Playwright logs into `logging`/SLF4J: the request was declined in favour of debug guides and `DEBUG` ([#6465](https://github.com/microsoft/playwright/issues/6465), +82, not planned; [playwright-java#1418](https://github.com/microsoft/playwright-java/issues/1418), not planned).

**Python specifics.** Two generated façades, `playwright.sync_api` and `playwright.async_api`, over one hand-written `_impl`. The sync API runs the asyncio dispatcher in a greenlet ([sync_api/_context_manager.py](https://github.com/microsoft/playwright-python/blob/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc/playwright/sync_api/_context_manager.py)) and refuses to start inside a running loop: "It looks like you are using Playwright Sync API inside the asyncio loop. Please use the Async API instead." Known issues documented in [library-python.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/library-python.md): `time.sleep()` "leads to outdated state" (events are not processed), Windows needs `ProactorEventLoop`, "Playwright's API is not thread-safe… create a playwright instance per thread", and "**Cancelling a task that is running a Playwright call is not supported and results in undefined behavior**" (use `asyncio.shield()`).

**Java and .NET specifics.** Java is synchronous only, not thread-safe ("all its methods… are expected to be called on the same thread where the Playwright object was created", [threading-java.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/threading-java.md)), and renames `goto` to `navigate` because `goto` is a reserved word. .NET is async-only with `Async` suffixes; a synchronous API was declined ([playwright-dotnet#2715](https://github.com/microsoft/playwright-dotnet/issues/2715), +20, not planned). `CancellationToken` parameters were declined in 2022: "Browser interactions like clicks, fills are all not cancelable, so even if we would provide it, it would be a kinda fake cancel" (mxschmitt, [playwright-dotnet#1652](https://github.com/microsoft/playwright-dotnet/issues/1652), +19); users were still asking in 2025–2026 ("one of my first surprises was the lack of support for `CancellationToken`"), and JS got `AbortSignal` in 1.62 while .NET did not. All three non-JS ports ship the Node.js driver: Python wheels 33–46 MiB, Java `driver-bundle` 203.8 MB, .NET package 213.5 MB ([browser-automation-products §3.2](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md)); "We consider Node.js runtime to be an implementation detail of Playwright" ([playwright-dotnet#1850](https://github.com/microsoft/playwright-dotnet/issues/1850), +49, open since 2021).

**Versioning.**
- No major version since 1.0 (2020-05-06); a maintainer: "Playwright version 2 is not gonna happen soon… We'll **definitely not** remove something which is essential" ([#20853](https://github.com/microsoft/playwright/issues/20853), 2023-03-22), and of `waitForNavigation`: "It will never be removed. Its use is discouraged, because it's hard to make it work reliably" (2023-02-16).
- Yet minors do remove long-deprecated API: 1.57 removed `page.accessibility` ("After 3 years of being deprecated"); 1.58 removed `_react`/`_vue` selectors, `:light`, `launch({ devtools })`; 1.60 removed `Locator.ariaRef()`, `exposeBinding({ handle })`, `connect({ logger })`, `videosPath`/`videoSize`; 1.64 changed device descriptors to forward `screen` and elements inside hidden iframes became hidden ([release notes](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)). No written semver or deprecation policy was found in `docs/src` at v1.64.1.
- Each release pins browser builds (1.64: Chromium 156.0.8078.4, Firefox 157.0, WebKit 27.2; also tested on Chrome and Edge 155). Since 1.57 Chromium is Chrome for Testing. `channel: 'chrome' | 'msedge'` uses installed stable browsers; "There is no guarantee it will work with any other version" ([class-browsertype.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md)).
- Monthly-ish cadence: 1.57 (2025-11-25) to 1.64 (2026-10-07) is eight minors in about ten months.
- Firefox and WebKit are patched builds (Juggler for Firefox). On WebDriver BiDi: "Playwright doesn't distinguish between different 'protocols' for driving the same browser today, those are internal details. We'd rather not put that choice on users" (yury-s, [#37277](https://github.com/microsoft/playwright/issues/37277), 2025-09-16); the blocker list is [#32577](https://github.com/microsoft/playwright/issues/32577) (+32, open).

**Issue-tracker policy.** Many old requests were closed in sweeps with the text "We are prioritizing the features based on the upvotes, recency and activity in the issue thread" (e.g. [#8622](https://github.com/microsoft/playwright/issues/8622) network throttling, +94, closed not planned 2026-05-08). Several API-design requests closed on 2026-05-07/16 carry no new rationale; counts of reactions on closed items therefore measure demand, not rejection on merit. GitHub Discussions are disabled on `microsoft/playwright`, `microsoft/playwright-python` and `puppeteer/puppeteer` (GraphQL `hasDiscussionsEnabled: false`, 2026-10-14).

#### 2.1.2 Minimal samples (own code, matching the v1.64 docs)

TypeScript (library):

```ts
import { chromium } from 'playwright';

await using browser = await chromium.launch();          // 1.59+: async disposable
const context = await browser.newContext({ locale: 'en-US' });
const page = await context.newPage();
await page.goto('https://example.com/');
const responsePromise = page.waitForResponse('**/api/search*'); // arm before trigger
await page.getByRole('searchbox', { name: 'Search' }).fill('collie');
await page.getByRole('button', { name: 'Search' }).click();     // auto-wait + strict
const data = await (await responsePromise).json();
const titles = await page.getByRole('listitem').allTextContents();
```

Python, sync and async:

```py
from playwright.sync_api import sync_playwright, expect

with sync_playwright() as p:
    with p.chromium.launch() as browser:
        page = browser.new_page()
        page.goto("https://example.com/")
        with page.expect_response("**/api/search*") as resp:
            page.get_by_role("button", name="Search").click()
        data = resp.value.json()
        expect(page.get_by_text("Results")).to_be_visible()
```

```py
import asyncio
from playwright.async_api import async_playwright

async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://example.com/")
        await page.get_by_role("link", name="More information").click()
        print(await page.title())
        await browser.close()

asyncio.run(main())
```

Java and C#:

```java
try (Playwright pw = Playwright.create();
     Browser browser = pw.chromium().launch()) {
  Page page = browser.newPage();
  page.navigate("https://example.com/");                 // goto is reserved in Java
  Response r = page.waitForResponse("**/api/search*", () ->
      page.getByRole(AriaRole.BUTTON, new Page.GetByRoleOptions().setName("Search")).click());
}
```

```csharp
using var pw = await Playwright.CreateAsync();
await using var browser = await pw.Chromium.LaunchAsync();
var page = await browser.NewPageAsync();
await page.GotoAsync("https://example.com/");
await page.GetByRole(AriaRole.Button, new() { Name = "Search" }).ClickAsync();
```

### 2.2 Puppeteer (puppeteer / puppeteer-core 25.13.0, 2026-10-08)

**Pinned references.** Tag [`puppeteer-v25.13.0`](https://github.com/puppeteer/puppeteer/tree/puppeteer-v25.13.0); `puppeteer-core` [CHANGELOG](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/CHANGELOG.md).

#### 2.2.1 Walkthrough

**Object model.** `puppeteer` → `Browser` → `BrowserContext` (`browser.createBrowserContext()`, renamed from `createIncognitoBrowserContext` in 22.0.0) → `Page` → `Frame`, plus `ElementHandle`/`JSHandle`, `HTTPRequest`/`HTTPResponse`, `CDPSession`, `Target`, `WebWorker`. JavaScript/TypeScript only; ESM-only and Node ≥ 22 since 25.0.0 (2026-05-12). Two protocols: CDP for Chrome and WebDriver BiDi for Firefox (BiDi default for Firefox since 23.0.0, 2024-08-07; Firefox over CDP removed in 24.0.0, 2025-01-09). Ports are community and hand-written: PuppeteerSharp tracks the version number (v25.13.1, 2026-10-10), pyppeteer is "unmaintained… Please consider playwright-python" ([README](https://github.com/pyppeteer/pyppeteer); last PyPI release 2.0.0, 2024-02-18).

**Launch vs connect, closing.** `puppeteer.launch(opts)`; `puppeteer.connect({ browserWSEndpoint | browserURL | transport })`. `browser.close()` kills the browser even when connected; `browser.disconnect()` leaves it running ([browser-management.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/browser-management.md)). Explicit resource management (`await using browser = …`) was exposed in **25.4.0** (2026-07-27) ([changelog](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/CHANGELOG.md)); `Browser` implements `[Symbol.asyncDispose]` ([Browser.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/api/Browser.ts)). Zombie Chrome processes were the classic leak ([#1825](https://github.com/puppeteer/puppeteer/issues/1825), +89, 2018–2020).

**Locators (since 20.6.0, 2023-06-09; filter/map/wait/waitHandle/function locators in 21.0.0, 2023-08-02).** "Locators is the recommended way to select an element and interact with it" ([page-interactions.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/page-interactions.md)). Before `click` a locator "Ensures the element is in the viewport", waits for it to be visible, enabled, and to "have a stable bounding box over two consecutive animation frames". Differences from Playwright, from the guide and source:
- **No hit-target ("receives events") check** in the documented list.
- **Not strict**: a CSS locator acts on the first match (it is built on `waitForSelector`).
- **Configured by chaining immutable setters**: `.setTimeout(3000)`, `.setVisibility(null)`, `.setWaitForEnabled(false)`, `.setEnsureElementIsInTheViewport(false)`, `.setWaitForStableBoundingBox(false)`.
- `.filter(fn)` and `.map(fn)` take a function **serialised into the page** ("doesn't have access to variables from the Node scope"); `.wait()` returns a value, `.waitHandle()` an `ElementHandle`; `Locator.race([...])`; one event, `LocatorEvent.Action`, "might fire multiple times if the locator retries the action".
- Implemented with RxJS operators and `retry({ delay: RETRY_DELAY })` ([locators.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/api/locators/locators.ts)). An `ElementHandle` can be converted to a locator since 24.18.0 (2025-09-01).
- The serialised predicate trips users: a `filter(el => el.textContent === dynamicValue)` times out because "the predicate is evaluated in the page context and dynamicValue is not defined there" (OrKoN, [#11893](https://github.com/puppeteer/puppeteer/issues/11893), open since 2024-02-12; maintainer asked internally whether to "add support for arguments to the filter locator").
- Selector syntax instead of `getBy*` methods (a user "transitioning from Playwright" asked for `getByTitle`/`getByLabel` and was pointed to `[title=…]` and `aria/…` selectors, [#12508](https://github.com/puppeteer/puppeteer/issues/12508), open): `::-p-text(…)`, `::-p-aria(…)`, `::-p-xpath(…)`, `pierce/`, `>>>` and `>>>>` shadow combinators.

`page.waitForSelector()` remains: it "does not automatically retry the action if it fails and requires manually disposing the resulting ElementHandle to prevent memory leaks" (same guide). `page.$`, `$$`, `$eval`, `$$eval` return handles or evaluate immediately with no waiting.

**Configuration.** Options objects per call; `.puppeteerrc.*`/`package.json` configuration files and `PUPPETEER_*` environment variables control downloads and defaults, but "Puppeteer's configuration files and environment variables are ignored by `puppeteer-core`" ([configuration.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/configuration.md)). Default timeout 30,000 ms ([TimeoutSettings.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/TimeoutSettings.ts)); `page.setDefaultTimeout`, `setDefaultNavigationTimeout`. `goto({ waitUntil: 'load' | 'domcontentloaded' | 'networkidle0' | 'networkidle2' })`.

**Events and network.** `page.on(...)` emitters (`addEventListener` aliases removed in 22.0.0). Interception is a mode switch plus a shared event: `await page.setRequestInterception(true)`, then every `request` event handler must call `continue()`, `abort()` or `respond()`, otherwise the request stalls; multiple handlers race and need `isInterceptResolutionHandled()` checks, "Always execute… synchronously together", or the opt-in "Cooperative Intercept Mode" with priorities ([network-interception.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/network-interception.md)). Response interception is the most-voted open request ([#1191](https://github.com/puppeteer/puppeteer/issues/1191), +139, open since 2017). Proxy per page was declined: "we will never support proxy per page given that it is possible to set it per browser context which is cheap to create" (OrKoN, [#678](https://github.com/puppeteer/puppeteer/issues/678), +118, 2024-07-29). The click-then-navigate race is handled by hand: `await Promise.all([page.waitForNavigation(), page.click('a')])`.

**Errors.** `PuppeteerError` → `TimeoutError`, `ProtocolError` → `TargetCloseError`, `ConnectionClosedError`, plus `TouchError`, `UnsupportedOperation` ([Errors.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/Errors.ts)). Messages are short and carry no call log: "Waiting for selector \`x\` failed", "Waiting failed: 30000ms exceeded", "Navigation timeout of 30000 ms exceeded" ([QueryHandler.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/QueryHandler.ts), [WaitTask.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/WaitTask.ts), [LifecycleWatcher.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/cdp/LifecycleWatcher.ts)). Classic CDP-level messages leak to users: "Execution context was destroyed, most likely because of a navigation" ([#3323](https://github.com/puppeteer/puppeteer/issues/3323), +38), "Cannot find context with specified id" ([#1325](https://github.com/puppeteer/puppeteer/issues/1325), +104).

**Debugging.** `headless: false`, `slowMo`, `devtools`-style flags, `node --inspect-brk`, `NODE_DEBUG="puppeteer:*"` to log protocol traffic, `browser.debugInfo` for pending calls ([debugging.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/debugging.md)). No trace viewer; Chrome DevTools Recorder exports Puppeteer scripts and `@puppeteer/replay` replays them.

**Versioning.** Semver majors with breaking changes listed: 22.0.0 (2024-02-05: removed `waitForTimeout`, `$x`, `waitForXPath`, `add/removeEventListener`, new headless by default), 23.0.0 (2024-08-07: `product`→`browser`, `ignoreHTTPSErrors`→`acceptInsecureCerts`, BiDi default for Firefox, `Uint8Array` instead of `Buffer`), 24.0.0 (2025-01-09), 25.0.0 (2026-05-12: ESM only, Node 22, `executablePath()` returns a Promise). "Every Puppeteer release is tightly bundled with a specific browser release" ([faq.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/faq.md)); 25.13.0 maps to Chrome for Testing 155.0.8059.39.

#### 2.2.2 Minimal sample (own code, matching the 25.13 guides)

```ts
import puppeteer from 'puppeteer';

await using browser = await puppeteer.launch();          // 25.4.0+
const page = await browser.newPage();
await page.goto('https://example.com/', { waitUntil: 'domcontentloaded' });
await page.locator('::-p-aria(Search[role="searchbox"])').fill('collie');
const [response] = await Promise.all([
  page.waitForResponse(r => r.url().includes('/api/search')),
  page.locator('button[type=submit]').click(),          // first match, not strict
]);
const titles = await page.$$eval('li', els => els.map(e => e.textContent));
```

### 2.3 Selenium, WebDriver classic and WebDriver BiDi (Selenium 4.51.0; BiDi Working Draft 2026-10-08)

#### Walkthrough

**Object model (classic).**
- A `WebDriver` object is one remote session. Commands are synchronous HTTP calls to a driver binary (chromedriver, geckodriver, …) that implements W3C WebDriver classic.
- `driver.quit()` ends the session and the driver process. `driver.close()` only closes the current window, and the session ends with the last window (docs, `InvalidSessionIdException`). The quit/close/dispose distinction is a perennial question: [SO 15067107](https://stackoverflow.com/q/15067107), 361 votes, 381k views. So is leaking driver processes when `quit()` is skipped: ["release chromedriver.exe from memory"](https://stackoverflow.com/q/21320837), 133 votes.
- `find_element` returns a `WebElement`: an opaque reference to one DOM node, resolved immediately, never re-resolved. If the node is replaced (re-render, navigation, frame switch), any use throws `StaleElementReferenceException`. The docs' remedy is "relocate the element before use" ([errors page](https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/)).
- The session has one implicit "current" browsing context, moved by `switch_to.window()` and `switch_to.frame()`. This is the same hidden-pointer model WebdriverIO dropped in v10 (§2.6).

**Waits.**
- Implicit wait is a session-global element-lookup timeout. Its default is `0`, so a missing element errors immediately.
- Explicit waits are `WebDriverWait(driver, t).until(<condition>)`, with `expected_conditions` (`EC.element_to_be_clickable`, `EC.visibility_of_element_located`, …). `FluentWait` (Java) adds a polling interval and ignored exceptions.
- The docs warn: "Do not mix implicit and explicit waits." Mixing causes unpredictable timeouts, for example implicit 10 s plus explicit 15 s can time out after 20 s ([waits page](https://www.selenium.dev/documentation/webdriver/waits/)).
- Nothing waits automatically before `click()` or `send_keys()`. A maintainer declined that in 2025: "Selenium automates browsers, that's it." ([#15430](https://github.com/SeleniumHQ/selenium/issues/15430), closed 2025-03-19). Auto-wait is left to wrapper libraries in the ecosystem.

**Errors** (27 documented on the [errors page](https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/)):

| Error | Documented cause | Documented fix |
|---|---|---|
| `NoSuchElementException` | element not found "at the exact moment you attempted to locate it" | wait strategy |
| `StaleElementReferenceException` | node detached after DOM change, navigation or frame switch | relocate |
| `ElementClickInterceptedException` | another element (overlay, modal, animation) receives the click | wait for clickable, or scroll |
| `TimeoutException` | explicit wait condition not met, or page load over 300 s | — |
| `ScriptTimeoutException` | async script over 30 s | — |

Messages name the condition, not the cause. `TimeoutException` from `WebDriverWait` carries only the message you pass in, and the last-seen element state is not recorded.

**Selenium Manager** ([docs](https://www.selenium.dev/documentation/selenium_manager/)):
- A Rust CLI bundled since 4.6. It discovers the browser version, downloads the matching driver and caches it in `~/.cache/selenium`.
- Browser download (Chrome for Testing) arrived in 4.11.0, Firefox in 4.12.0 and Edge in 4.14.0.
- It sends usage statistics to Plausible by default. Opt out with `SE_AVOID_STATS=true` or `avoid-stats = true`.
- Open complaint: opt-out does not work and telemetry should be opt-in ([#14643](https://github.com/SeleniumHQ/selenium/issues/14643), 2024-10-24, open). A maintainer replied 2025-04-22 that a GDPR lawyer was being hired.
- Other open issues: ARM64 binaries ([#15801](https://github.com/SeleniumHQ/selenium/issues/15801), 48 comments) and PATH behaviour ([#17554](https://github.com/SeleniumHQ/selenium/issues/17554)).

**How the language bindings are kept in sync.**
- Java, Python, C#, Ruby and JavaScript are hand-written per language in one monorepo (Bazel). They share the JavaScript "atoms", now also published as `@seleniumhq/atoms` (4.51.0 notes) and Selenium Manager.
- CDP bindings are generated per Chrome milestone from the protocol JSON (`CdpClientGenerator.java`). At 4.51.0 the tree holds `v153`, `v154` and `v155`, and the docs say Selenium "supports the 3 most recent versions of Chrome". Java users add `selenium-devtools-v1xx` artifacts matching the browser ([CDP page](https://www.selenium.dev/documentation/webdriver/bidi/cdp/)).
- New in 2026: the internal BiDi protocol layer is now generated from the CDDL, via one shared binding-neutral schema ([#17700](https://github.com/SeleniumHQ/selenium/pull/17700), merged 2026-06-26).
  - JS/TS generator: [#17574](https://github.com/SeleniumHQ/selenium/pull/17574), merged 2026-06-08.
  - Python, generated at build time and not checked in: [#17761](https://github.com/SeleniumHQ/selenium/pull/17761), merged 2026-08-21.
  - Java: [#17777](https://github.com/SeleniumHQ/selenium/pull/17777), merged 2026-10-06, shipped in 4.51.0.
- The decision record [17786](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17786-bidi-low-level-behavioral-contract.md) fixes the wire behaviour every binding must follow:
  - typed objects, not maps
  - spec names
  - full numeric range ("BiDi integers exceed a 32-bit integer's range")
  - reject invalid outbound values
  - tolerate unknown inbound fields
  - reject malformed inbound values or missing required fields
- Binding drift is documented in Selenium's own records. Before ADR [17685](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17685-network-handler-behavior.md):
  - Java ran one matching handler "chosen in no defined order" (a `ConcurrentHashMap`).
  - Ruby ran handlers in parallel threads, so dispositions raced.
  - .NET and JS had no handler API at all.
- Other cross-binding gaps: [#12368](https://github.com/SeleniumHQ/selenium/issues/12368) (HTTP client config, label "needs new owner"), [#10942](https://github.com/SeleniumHQ/selenium/issues/10942) (JS bindings not in TypeScript; types come from DefinitelyTyped, and BiDi types are missing there, [#11670](https://github.com/SeleniumHQ/selenium/issues/11670)), [#14067](https://github.com/SeleniumHQ/selenium/issues/14067) and [#17086](https://github.com/SeleniumHQ/selenium/issues/17086) (.NET sync API, but BiDi is `AsBiDiAsync()`).

**Relative locators (Selenium 4).**
- `locate_with(By.TAG_NAME, "input").above({By.ID: "password"})`. Also `below`, `to_left_of`, `to_right_of`, `near`, and they can be chained.
- Implemented with `getBoundingClientRect()` geometry ([locators page source](https://github.com/SeleniumHQ/seleniumhq.github.io/blob/trunk/website_and_docs/content/documentation/webdriver/elements/locators.en.md)).
- Selenium has no role or accessible-name locator. A community proposal ([#16135](https://github.com/SeleniumHQ/selenium/issues/16135), 2025-08, open, "needs decision") was pointed at BiDi `browsingContext.locateNodes`.

**BiDi in Selenium.**
- Enabled with the `webSocketUrl: true` capability (`options.enable_bidi = True` in Python, `enableBiDi()` in Java and JS).
- High-level surfaces:
  - `driver.script.add_console_message_handler` and `add_javascript_error_handler` (BiDi `log.entryAdded`)
  - `driver.network.add_request_handler` / `remove_` / `clear_`, and `add_authentication_handler` (built on `network.addIntercept` and `continueRequest` / `failRequest` / `provideResponse` / `continueWithAuth`)
  - `driver.browsing_context.*`, and `installWebExtension` per ADR [17817](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17817-driver-extension-install.md)
- Status: the high-level network API has been "beta" and incomplete since 2024 ([#13993](https://github.com/SeleniumHQ/selenium/issues/13993), open; the table shows Java, JS and .NET empty for request handlers at filing). Response bodies need the newer BiDi data collectors ([#16514](https://github.com/SeleniumHQ/selenium/issues/16514), open).
- The official Python example still uses private `driver.network._add_intercept()`, "until a public API is stabilized" ([examples repo](https://github.com/SeleniumHQ/seleniumhq.github.io/blob/trunk/examples/python/tests/bidi/test_network_commands.py)).
- ADR [17670](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17670-bidi-implementation-boundaries.md) (accepted 2026) says: "BiDi is an implementation mechanism, not a public API." The supported API is protocol-neutral. Raw BiDi is internal, unsupported and reachable only by composition (`BiDi::Protocol::Network.new(driver)`), never as `driver.bidi`. Their reason for rejecting a public mid-level BiDi API is protocol coupling, "the same trap CDP created". **No binding conformed when it was written** (the ADR's own table).
- Network-handler semantics from ADR 17685 (directly reusable design):
  - add/remove/clear per family (request, response, auth), with handles
  - URL patterns declared at registration and evaluated by the remote end; globs are forwarded, not expanded
  - first handler to settle (`fail` / `respond` / `submit`) wins; staging without settling falls through
  - LIFO order "matches Playwright"
  - an uncaught exception fails the event
  - return values are ignored
  - handlers see the original event
  - bodies are collected only on opt-in
  - scoped to one window by default
  - Note: the docs example passes `"**/blank.html"`, a glob the ADR says is forwarded as-is. That inconsistency is unresolved in the docs.

**CDP stance and timeline.**
- The CDP page says "CDP support is temporary until WebDriver BiDi has been implemented" and that CDP is "not designed for testing, nor to have a stable API". It never says "deprecated".
- [#13990](https://github.com/SeleniumHQ/selenium/issues/13990) ("Mark CDP usage as deprecated", open since 2024-05-20): the Selenium Dev Summit agreed to keep CDP in Selenium 5 but mark it deprecated. The maintainers want BiDi parity first (the Selenium 5 C-devtools milestone).
- Firefox:
  - CDP was deprecated in Firefox 129 (August 2024).
  - Selenium deprecated Firefox CDP in 4.27–4.28 and removed it in 4.29.0 ([blog 2025-02-03](https://www.selenium.dev/blog/2025/remove-cdp-firefox/): "Selenium's CDP implementation for Firefox was always partial").
  - Mozilla removed CDP and the `remote.active-protocols` pref in the Firefox 141 cycle ([Firefox WebDriver newsletter 141, 2025-07-22](https://fxdx.dev/firefox-webdriver-newsletter-141/)). Firefox 140 ESR keeps CDP behind the pref as a transition ([fxdx.dev](https://fxdx.dev/cdp-retirement-in-firefox/)).
- Python CDP access requires an async `trio` context inside the sync API (`async with driver.bidi_connection()`, [example](https://github.com/SeleniumHQ/seleniumhq.github.io/blob/trunk/examples/python/tests/bidi/cdp/test_network.py)). That is a dual sync/async seam.

#### WebDriver BiDi: spec and support status

- **Spec:** [W3C Working Draft, 8 October 2026](https://www.w3.org/TR/webdriver-bidi/) (the Editor's Draft has the same date).
  - Modules: `session`, `browser`, `browsingContext`, `emulation`, `input`, `log`, `network`, `script`, `storage`, `webExtension`.
  - I counted 68 commands and 24 events from the spec anchors (excluding meta anchors such as `command.name` and `event.type`). They include `network.addIntercept`, `network.addDataCollector` / `getData` (bodies), `script.addPreloadScript`, `browsingContext.locateNodes` (CSS, XPath, inner text and **accessibility** locators), `browsingContext.startScreencast`, `input.setFiles`, and `emulation.*` overrides.
  - Events include `network.beforeRequestSent`, `responseStarted`, `responseCompleted`, `fetchError`, `authRequired` and `log.entryAdded`.
- **Puppeteer "Is Puppeteer WebDriver BiDi ready?"** ([dashboard](https://puppeteer.github.io/ispuppeteerwebdriverbidiready/), data pulled 2026-10-14 from `data.json`; the production-required subset has 1,258 tests):

  | Configuration | Passing |
  |---|---|
  | Chrome via BiDi+ (BiDi with CDP extras) | 94.12% |
  | Chrome via BiDi only | 87.20% |
  | Firefox via BiDi | 78.54% |
  | Chrome via CDP (reference, 1,379 tests) | 97.53% |

  Across all tests (1,346): Chrome BiDi+ 89.90%, Chrome BiDi-only 82.84%, Firefox 73.77%.
- **Puppeteer** v23 made Firefox support stable over BiDi, the default for Firefox. Chrome stays on CDP by default "to not break existing automations" ([Chrome blog](https://developer.chrome.com/blog/firefox-support-in-puppeteer-with-webdriver-bidi)).
- **Chrome** implements BiDi through the `chromium-bidi` mapper over CDP. Safari/WebKit BiDi status: **unverified** (not checked).
- **Context:** Vibium, by Selenium's creator ([HN 46377597](https://news.ycombinator.com/item?id=46377597), 443 points, 2025-12-24), is an AI/human browser-automation tool. A March 2026 HN post connects it with BiDi for cloud browsers ([HN 47335050](https://news.ycombinator.com/item?id=47335050)). Its protocol choice is **partly verified** only. Another part covers it.

#### Minimal representative sample (Python, Selenium 4.51; my own code following current docs)

```python
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.relative_locator import locate_with

options = webdriver.ChromeOptions()
options.enable_bidi = True                      # sets capability webSocketUrl: true
driver = webdriver.Chrome(options=options)      # Selenium Manager resolves chromedriver (and CfT if absent)
try:
    console = []
    driver.script.add_console_message_handler(console.append)              # BiDi log.entryAdded
    driver.network.add_request_handler(["**/*.png"], lambda req: req.fail())  # BiDi addIntercept (beta API)

    driver.get("https://example.com/login")
    wait = WebDriverWait(driver, 10)            # explicit wait; implicit wait left at 0 (never mix)
    email = wait.until(EC.element_to_be_clickable((By.ID, "email")))
    email.send_keys("qa@example.com")
    password = driver.find_element(locate_with(By.TAG_NAME, "input").below({By.ID: "email"}))
    password.send_keys("not-a-real-secret")
    wait.until(EC.element_to_be_clickable((By.CSS_SELECTOR, "button[type=submit]"))).click()
    wait.until(EC.url_contains("/dashboard"))   # `email` is now likely stale: re-find, don't reuse
finally:
    driver.quit()                               # forgetting this leaks driver and browser processes
```

#### Dimensions

| Dimension | Selenium 4.51 |
|---|---|
| Protocols | W3C WebDriver classic (HTTP); BiDi (WebSocket) opt-in; CDP (Chromium only, version-pinned, "temporary") |
| Element handle | eager `WebElement` (one node, goes stale); no lazy locator object |
| Waiting | none built in; implicit (global) or explicit `WebDriverWait` + `expected_conditions` |
| Context model | one implicit current window/frame pointer, `switch_to` |
| Multi-language | 5 hand-written bindings plus shared atoms; BiDi layer now CDDL-generated (2026) |
| Driver/browser provisioning | Selenium Manager (Rust, since 4.6; browsers since 4.11); telemetry on by default |
| Network | BiDi handlers (beta, uneven across bindings); response bodies pending |
| Selectors | CSS, XPath, id, name, link text, tag, class; relative (geometry); no role/text engine |
| Errors | 27 typed exceptions; messages name the symptom; no action log or trace |
| Governance | Software Freedom Conservancy project, ADR process since 2026 |

#### What users like (short quotes)

- A W3C standard with real-browser, multi-vendor drivers; Grid; many languages. "Playwright's main advantage is it has official support for multiple languages" was said *against* Playwright-vs-Selenium parity ([HN 46378403](https://news.ycombinator.com/item?id=46378403)). Selenium has more official languages (5) than Playwright (4).
- Community governance as a durability argument: "Community driven projects … always are more trustworthy" ([HN 33543011](https://news.ycombinator.com/item?id=33543011), 2022-11-10).
- Some testers argue flakiness is a test-authoring problem and auto-waiting is "just marketing" (single-author opinion, [agileway.substack.com](https://agileway.substack.com/p/auto-waiting-in-web-test-automation)).
- Career and ecosystem gratitude: "it was a game changer" ([HN 46378680](https://news.ycombinator.com/item?id=46378680), 2025-12-24).

#### Pain points, ranked (evidence → status → lesson)

1. **Timing, with no auto-wait.**
   - Evidence: SO [26566799](https://stackoverflow.com/q/26566799) (356 votes, 849k views), [5868439](https://stackoverflow.com/q/5868439) (304), [10404160](https://stackoverflow.com/q/10404160) (64), [15164742](https://stackoverflow.com/q/15164742) and [29474296](https://stackoverflow.com/q/29474296) (mixing implicit and explicit, 17 and 21). HN [33539710](https://news.ycombinator.com/item?id=33539710): "With Playwright you don't need to put manual waits in your program". HN [33543274](https://news.ycombinator.com/item?id=33543274): "Auto-waiting - it is a BRILLIANT feature!". Declined [#15430](https://github.com/SeleniumHQ/selenium/issues/15430) (2025). About 8 independent reports, 2009–2025.
   - Status: by design. Wrappers provide waiting.
   - **Lesson:** actionability waits belong inside every action, and the timeout is per action. Never offer a global implicit wait that composes badly with explicit ones. The single timeout model must be documented as additive or non-additive.
2. **Stale element references.**
   - Evidence: SO [18225997](https://stackoverflow.com/q/18225997) (182 votes, 743k views), [16166261](https://stackoverflow.com/q/16166261) (51, 266k views), [43543599](https://stackoverflow.com/q/43543599), [45002008](https://stackoverflow.com/q/45002008), [29928856](https://stackoverflow.com/q/29928856) (Protractor). Selenium issues [#3859](https://github.com/SeleniumHQ/selenium/issues/3859), [#6152](https://github.com/SeleniumHQ/selenium/issues/6152), [#8376](https://github.com/SeleniumHQ/selenium/issues/8376). The same failure class in Cypress: [#7306](https://github.com/cypress-io/cypress/issues/7306), 541 reactions.
   - Status: inherent to eager handles.
   - **Lesson:** the default handle should be a lazy, re-resolving locator (Playwright `Locator`, WDIO lazy chains, Cypress 12 queries). Explicit "pinned" handles are opt-in, and their errors name the re-render that detached them.
3. **Click interception / "not clickable at point".**
   - Evidence: SO [11908249](https://stackoverflow.com/q/11908249) (486 votes, 719k views), [48665001](https://stackoverflow.com/q/48665001) (94, 205k views), [56779520](https://stackoverflow.com/q/56779520), [57632563](https://stackoverflow.com/q/57632563), [62903056](https://stackoverflow.com/q/62903056). WDIO [#3106](https://github.com/webdriverio/webdriverio/issues/3106) (wait for animations, 48 comments). About 7 reports.
   - Status: the exception exists, but there is no hit-test wait.
   - **Lesson:** a guarded click must check hit-target, stability (no animation) and enabled state. On failure it should report *which* element would receive the click.
4. **Driver binaries and version mismatch** (historically the biggest; now mitigated).
   - Evidence: SO [40208051](https://stackoverflow.com/q/40208051) (643 votes, 1.0M views), [29858752](https://stackoverflow.com/q/29858752) (383), [60296873](https://stackoverflow.com/q/60296873) (206), [55201226](https://stackoverflow.com/q/55201226) (200), [60362018](https://stackoverflow.com/q/60362018) (336, macOS Gatekeeper), [50642308](https://stackoverflow.com/q/50642308) (390, DevToolsActivePort). WDIO [#12251](https://github.com/webdriverio/webdriverio/issues/12251) (chromedriver host moved, 2024-02) and [#15608](https://github.com/webdriverio/webdriverio/issues/15608) (open 2026-09). About 9 reports.
   - Status: Selenium Manager (4.6/4.11) fixed most of it. Open: telemetry consent [#14643](https://github.com/SeleniumHQ/selenium/issues/14643), ARM64 [#15801](https://github.com/SeleniumHQ/selenium/issues/15801).
   - **Lesson:** the engine owns provisioning (browser plus driver, pinned and cached), and setup failures need actionable errors. Telemetry is opt-in.
5. **Automation is detectable.**
   - Evidence: SO [33225947](https://stackoverflow.com/q/33225947) (645 votes, 518k views). undetected-chromedriver (12.9k stars, 1,141 open issues, issues now disabled). The whole §2.7 ecosystem exists because of this.
   - Status: out of Selenium's scope.
   - **Lesson:** see §2.7. The protocol-level footprint (CDP domains enabled, isolated worlds, flags) is a design dimension, not an afterthought.
6. **API ergonomics and breaking removals.**
   - Evidence: "that API has NOT aged well … Selenium feels very Java-ish" ([HN 33538602](https://news.ycombinator.com/item?id=33538602), 2022-11-09). Removal of `find_element_by_*` in Python 4.3.0 (June 2022) produced SO [72773206](https://stackoverflow.com/q/72773206) (136 votes) and [72754651](https://stackoverflow.com/q/72754651) (89).
   - **Lesson:** one short, consistent verb set per SDK. Remove nothing tutorials depend on without a long, loud deprecation (Goodfetch is pre-release now, but this applies from first publication).
7. **CDP version coupling.**
   - Evidence: version-specific `selenium-devtools-v1xx` packages (Chrome N, N-1, N-2 only). The CDP page itself calls the API unstable. Python `driver.get_log()` regression closed not-planned ([#15772](https://github.com/SeleniumHQ/selenium/issues/15772), 2025–2026). Firefox console errors missing for years ([#1161](https://github.com/SeleniumHQ/selenium/issues/1161)).
   - Status: being replaced by BiDi; CDP to be deprecated before Selenium 5 ([#13990](https://github.com/SeleniumHQ/selenium/issues/13990)).
   - **Lesson:** never expose raw protocol types in the public SDK (Selenium reached the same conclusion in ADR 17670). Generate the protocol layer from a pinned schema, inside the engine.
8. **BiDi high-level APIs incomplete and divergent across bindings.**
   - Evidence: [#13993](https://github.com/SeleniumHQ/selenium/issues/13993) (open since 2024-05), [#16514](https://github.com/SeleniumHQ/selenium/issues/16514), the ADR 17685 and 17670 divergence tables, and the docs' private `_add_intercept`.
   - **Lesson:** Goodfetch's one-engine design is the antidote. Semantics (handler order, fallthrough, errors, scoping) are decided once in the engine and surfaced identically. Specify the semantics in writing, as ADR 17685 does.
9. **Session and process lifecycle.**
   - Evidence: SO [15067107](https://stackoverflow.com/q/15067107) (361), [21320837](https://stackoverflow.com/q/21320837) (133). "User data directory is already in use" ([#15327](https://github.com/SeleniumHQ/selenium/issues/15327), 54 comments, 2025; same in WDIO [#14501](https://github.com/webdriverio/webdriverio/issues/14501), 31 comments).
   - **Lesson:** scoped lifetimes (context managers, `using`, `Drop`), an engine-side reaper for orphans, and a unique temp profile by default.
10. **Hidden control flow.**
    - Evidence: WebDriverJS's promise manager, a Cypress-like queue, was deprecated and removed after the JS team cited "reduced debuggability" ([#2969](https://github.com/SeleniumHQ/selenium/issues/2969), 92 reactions, 2016–2017). WDIO's fibers sync mode was removed in v8, 2022 (§2.6).
    - **Lesson:** plain `async`/`await` (or blocking calls in Python's sync API) with no implicit scheduler.

#### Declined or deferred requests (maintainer rationale)

- Auto-wait in `findElement`: declined. "Selenium automates browsers, that's it." Leave it to frameworks ([#15430](https://github.com/SeleniumHQ/selenium/issues/15430), pujagani, 2025-03-19).
- Public mid-level BiDi API: rejected in ADR 17670 ("protocol coupling … the same trap CDP created").
- Predicates in URL filters: rejected in ADR 17685. Patterns are forwarded to the remote end, and predicates go inside the callable.
- Marking CDP deprecated: deferred until BiDi parity ([#13990](https://github.com/SeleniumHQ/selenium/issues/13990), titusfortner, 2024-05-28).

---

### 2.4 Rust crates: chromiumoxide, headless_chrome, fantoccini, thirtyfour (plus playwright-rs, rustwright, chromey)

Research of **October 14, 2026**, read-only, for the Goodfetch consumer page API (Rust SDK over one Rust engine, Playwright-shaped). This part covers the API as a developer sees it, and user pain. Protocol revisions, OOPIF handling, maintenance metrics and dependency counts are in the prior note `rnd/research/2026-10-10-rust-cdp-clients.md` on branch `origin/research/rust-cdp-clients`, and are not repeated here.

**Sources and method.**
- Crate archives downloaded from crates.io and read at the commit each one records in `.cargo_vcs_info.json`:
  - chromiumoxide 0.9.1 (`mattsse/chromiumoxide@a7e2bb8`)
  - headless_chrome 1.0.22 (`rust-headless-chrome/rust-headless-chrome@0a5c307`)
  - fantoccini 0.22.1 (`jonhoo/fantoccini@ed1d694`)
  - thirtyfour 0.37.8 (`stevepryde/thirtyfour@17017c8`, path `thirtyfour/`)
  - playwright-rs 0.20.0 (`padamson/playwright-rust@255c8bf`, path `crates/playwright`)
  - rustwright 0.3.0 (`Skyvern-AI/rustwright@769674b`, path `rust-native`, marked "dirty")
  - chromey 2.58.2 (`spider-rs/chromey@623c0df`)
- GitHub search API, for the top 30 issues per repository sorted by reactions and by comments. Each issue cited below was opened and read, with its comments, through the API.
- Hacker News, through the Algolia API.
- users.rust-lang.org, through the Discourse JSON API.
- Two dev.to posts, fetched.
- **Reddit could not be reached.** reddit.com blocks both the search tool and `curl` to its JSON API. Reddit claims below come second-hand and are marked **unverified**.
- Nothing was compiled or run. "From source" means read in the published archive. The code samples are written against the source at those versions; they were not compiled.

**The right repositories.** thirtyfour lives at `stevepryde/thirtyfour`. Vrtgs is listed as co-author in `Cargo.toml`, and that repository has 1,435 stars, 3 open issues and a last push on 2026-10-12. There is no `Vrtgs/thirtyfour` main repository. fantoccini is `jonhoo/fantoccini`. headless_chrome is `rust-headless-chrome/rust-headless-chrome`, formerly `atroche/`.

---

#### Summary

1. **Lifecycle is the top user pain in every Rust crate, because Rust has no async `Drop`.** Each crate picked a different, imperfect answer:
   - chromiumoxide makes you spawn and poll a `Handler` stream, and its `Drop` only logs a warning and relies on `kill_on_drop`.
   - headless_chrome is blocking, and kills Chrome in `Drop`.
   - fantoccini sends `DELETE /session` from a spawned task when the last `Client` clone drops. It is lost if the runtime stops first.
   - thirtyfour blocks the dropping thread on a fresh runtime and logs a warning.
   - playwright-rs blocks in `Drop` up to a grace period, and offers `shutdown().await`.

   Users report leaked Chrome processes, leaked WebDriver sessions on test failure, hangs, and panics inside `Drop`. That is about 16 independent reports from 2018 to 2026, across 5 crates plus 3 forum threads.
2. **Hangs and timeouts that ignore configuration** are the second cluster, about 17 reports, mostly chromiumoxide. One is verified in source: page commands hard-code the 30 s `REQUEST_TIMEOUT` and ignore `BrowserConfig::request_timeout` (issue #333, open).
3. **Waiting is mostly the user's job.**
   - chromiumoxide `find_element` queries once.
   - headless_chrome has `wait_for_element`, with a 20 s default and 100 ms polling.
   - fantoccini has `wait().for_element`, with a 30 s default and 250 ms polling.
   - thirtyfour has the richest model: `query()` plus `ElementQuery` with filters, `first`, `single` and `exists`, a 20 s default and 500 ms polling. Its `single()` semantics still confused users (#275, 17 comments).
   - Only playwright-rs, which delegates to the Playwright driver, and rustwright have Playwright-style actionability auto-wait.
   - Long-standing requests: `networkidle` (chromiumoxide #36, open since 2021), stable DOM (#328), `wait_for_network_idle` (headless_chrome #456).
4. **Errors.** Users hit String errors, the deprecated `failure` crate, private error types, panics inside the library, and timeouts that do not say what timed out. thirtyfour and playwright-rs have since added selector descriptions to errors. playwright-rs does it by appending text to a `String`, not with structured fields.
5. **Network interception and events are hard to discover in every crate.** In chromiumoxide you enable interception in the config, subscribe to the raw CDP event, and answer it with raw `Fetch` commands from your own task. fantoccini declines interception as out of WebDriver scope (#222). thirtyfour only gained typed CDP events and BiDi `network` in 0.37.0 (May 2026). It took 5 years to close #55, "Can I listen CDP events in thirtyfour?".
6. **People keep asking for Playwright in Rust.**
   - microsoft/playwright [#18266](https://github.com/microsoft/playwright/issues/18266) (15 reactions) was closed as not planned in 2022: "community driven project". #30675 and #34216 met the same fate.
   - octaltree's `playwright` crate (0.0.20, August 2022) stalled. Its "still maintained?" issue has 18 comments, and a 2025 fork was archived within months.
   - padamson's `playwright-rs` (0.20.0, October 11, 2026, released roughly every two weeks) now claims full Python API parity, through a 130 MB Node driver.
   - Skyvern's rustwright reimplements Playwright natively over CDP, with a sync Rust API, and treats Python and Node as its primary users.
7. **Design lessons for Goodfetch**, detailed per pain point in the "Pain points" list below:
   - cheap `Clone`, `Send + 'static` handles, with no lifetimes;
   - a sync, lazy `Locator`;
   - auto-wait in actions, surviving navigation;
   - one deadline source with typed timeout errors that carry the operation, selector, elapsed time and configured limit;
   - an explicit `close().await` plus a scoped helper like thirtyfour's `run_browser_test`, with `Drop` as a best-effort fallback that warns;
   - engine-side reaping of browsers when the SDK disconnects;
   - typed, `#[non_exhaustive]` error and option types;
   - first-class route and request events;
   - a browser version pinned to the SDK version, as playwright-rs does through `Cargo.lock`.

---

#### chromiumoxide 0.9.1

Docs: <https://docs.rs/chromiumoxide/0.9.1>. Source: <https://github.com/mattsse/chromiumoxide/tree/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4>. MSRV 1.85; edition 2024 since 0.9.0. Tokio only, since async-std was removed in 0.9.0.

##### Object model and lifecycle

- **Launch.** `Browser::launch(BrowserConfig) -> Result<(Browser, Handler)>` ([browser/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/browser/mod.rs)).
- **Connect.** `Browser::connect(url)` and `connect_with_config(url, HandlerConfig)` accept a `ws://` URL. Given an `http` URL, they first fetch `/json/version`, and replace `127.0.0.1` with the remote IP.
  - From source: `connect_with_config` calls `req.remote_addr().unwrap()`, a possible panic in library code.
- **The `Handler` is a `Stream` that the user must poll.** The README's canonical loop is `tokio::spawn(async move { while let Some(h) = handler.next().await { if h.is_err() { break; } } })`. If it is not polled, nothing progresses: commands are sent through a bounded channel (`channel(1)`) to the handler.
  - Issue [#28](https://github.com/mattsse/chromiumoxide/issues/28) (2021) was "`event_listener()` doesn't receive any events", a library bug at the time. The maintainer's reply defends streams over callbacks: callbacks make the "caller lose control over where the control flow goes".
- **Pages.** `browser.new_page(url)` returns `Page`, a cheap `Arc<PageInner>` handle. `Page::close(self)` consumes it.
  - `Element` holds an `Arc<PageInner>`, a `NodeId`, a `BackendNodeId` and a `RemoteObjectId`. It has no lifetime parameter.
  - `Browser` is not `Clone`. Issue [#51](https://github.com/mattsse/chromiumoxide/issues/51), "How to implement the 'Copy' trait for `chromiumoxide::Browser`?" (7 comments), was answered with "wrap it in an Arc".
  - `close`, `wait`, `kill`, `fetch_targets` and incognito contexts take `&mut self`.
- **Close.**
  - `browser.close().await` sends `Browser.close`.
  - The docs then recommend `browser.wait().await` to reap the child "to avoid zombie processes". `kill()` also exists.
  - `Drop for Browser` does not close. It logs `tracing::warn!("Browser was not closed manually, it will be killed automatically in the background")` and relies on Tokio's `kill_on_drop`, because "it can't really be done anyway since the method is async".
  - History: PR #128 had made `Drop` call `block_on`, which panicked under Tokio ("Cannot start a runtime from within a runtime", issue [#137](https://github.com/mattsse/chromiumoxide/issues/137), 2023, closed).
  - On a launch failure, `Browser::launch` kills and waits for the child, but with `.expect(...)`, which panics if cleanup fails.

##### Element finding and waiting

- **No auto-wait.** `page.find_element(css)` makes four round trips: `DOM.getDocument`, then `DOM.querySelector`, then `DOM.describeNode` and `DOM.resolveNode` (in `Element::new`). It returns the first match once.
  - When nothing matches, `querySelector` returns node id 0. The failure then surfaces from the follow-up `describeNode` as a generic CDP error, not a "not found for selector X" error (from source; exact message **unverified**).
  - `find_elements` uses `querySelectorAll`. `find_xpath` uses `DOM.performSearch` and indexes `[0]` into the result, so an empty success would panic (from source). Whether Chrome errors first on an empty range is **unverified**.
  - There is no locator, strictness or retry.
- **Waiting.**
  - `page.wait_for_navigation()` waits for the frame's load lifecycle, not network idle. In [#36](https://github.com/mattsse/chromiumoxide/issues/36) the maintainer said it is the "same as `load` in puppeteer".
  - `goto()` resolves "after the requested URL is fully loaded".
  - `Element::click()` scrolls into view, takes the content-quad centre and dispatches the mouse events. It runs no actionability checks.
  - The `click()` docs warn that a click triggering navigation needs an extra `wait_for_navigation()`.

##### Configuration

- `BrowserConfig::builder()` offers:
  - `with_head()`, `new_headless_mode()`, `headless_mode()`, `no_sandbox()`, `incognito()`, `window_size()`, `viewport()`;
  - `port()`, `user_data_dir()`, `chrome_executable()`, `chrome_detection()`;
  - `arg()`, `args()`, `env()`, `extension()`, `disable_default_args()`;
  - `enable_request_intercept()`, `enable_cache()` and `disable_cache()`;
  - `respect_https_errors()`, `surface_invalid_messages()`, `hide()`;
  - `launch_timeout()` and `request_timeout()`.
- `build()` returns **`Result<BrowserConfig, String>`**. Issue [#259](https://github.com/mattsse/chromiumoxide/issues/259) (open, 2025) notes that `String` does not implement `std::error::Error`. A maintainer replied it is "worth a breaking change".
- **Defaults** ([config.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/browser/config.rs), [handler/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/mod.rs)):
  - `LAUNCH_TIMEOUT` = 20,000 ms;
  - `REQUEST_TIMEOUT` = 30,000 ms;
  - headless; viewport 800×600 (prior note);
  - `ignore_https_errors` true; `ignore_invalid_messages` true.
- **Timeout bugs, verified in source.**
  - `CommandFuture::new` builds its timeout from the constant: `Delay::new(Duration::from_millis(crate::handler::REQUEST_TIMEOUT))` ([commandfuture.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/commandfuture.rs)). So `request_timeout` has no effect on page commands such as `goto`, as issue [#333](https://github.com/mattsse/chromiumoxide/issues/333) reports (open, August 2026). PRs #332 and #336 are unmerged (prior note).
  - Separately, `TargetConfig::default()` uses `Duration::from_secs(REQUEST_TIMEOUT)`, which is 30,000 **seconds**, a unit slip ([target.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/target.rs)). Whether that default is reached in practice is **unverified**; the handler builds `TargetConfig` explicitly.
- **Environment variables.** None for configuration, apart from detection paths and the `CDP_NO_EXPERIMENTAL` build flag mentioned in the README.
- **README troubleshooting.** The debugging port is parsed from Chrome's stderr, "limited to english". Users hit this in [#113](https://github.com/mattsse/chromiumoxide/issues/113) and [#216](https://github.com/mattsse/chromiumoxide/issues/216).

##### Naming and idioms

- `Result<T, CdpError>`. `CdpError` is a `thiserror` enum ([error.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/error.rs)):
  - transport: `Ws`, `Io`, `Serde`, `ChannelSendError`, `UnexpectedWsMessage`, `InvalidMessage`;
  - browser: `Chrome` (protocol error), `ChromeMessage(String)`, `NoResponse`;
  - launch: `LaunchExit`, `LaunchTimeout`, `LaunchIo`, each with stderr;
  - other: `Timeout` (displayed as "Request timed out.", with no operation), `FrameNotFound`, `NotFound`, `JavascriptException`, `ScrollingFailed`, `DecodeError`, `Url`.
- Methods return `Result<&Self>` for chaining: `.click().await?.type_str(..).await?.press_key("Enter")`.
- The raw escape hatch is `Page::execute<T: Command>`, `Browser::execute`, and `event_listener::<T>()` returning `EventStream<T>`.
- The typed protocol is generated: every `XParams::builder()` comes with an `XReturns`.

##### Events and network

- `page.event_listener::<EventRequestPaused>().await?` returns a `Stream` of `Arc<Event>`.
- **Interception.** Set `BrowserConfig::enable_request_intercept()`, then subscribe to `fetch::EventRequestPaused` and answer each event with `FulfillRequestParams` or `ContinueRequestParams` from your own spawned task. That is the official example, [`examples/interception.rs`](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/examples/interception.rs).
- Users mixed up `fetch::GetResponseBodyParams` and `network::GetResponseBodyParams` and got "Fetch domain is not enabled" ([#256](https://github.com/mattsse/chromiumoxide/issues/256)).

##### Versioning

- Releases: 0.7.0 (August 2024), 0.8.0 (November 2025, protocol r1519099, Tokio by default, fetcher API breaking), 0.9.0 (February 2026, async-std removed), 0.9.1 (February 25, 2026) ([CHANGELOG](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/CHANGELOG.md)).
- Each minor release is breaking (0.x). It is coupled to Chrome through strict generated types; see the prior note.

##### Sample (0.9.1, not compiled)

```rust
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // build() returns Result<_, String>; `?` works only because Box<dyn Error>: From<String>
    let (mut browser, mut handler) = Browser::launch(BrowserConfig::builder().build()?).await?;
    // Mandatory: nothing progresses unless the handler stream is polled.
    let driver = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() { break; }
        }
    });

    let page = browser.new_page("https://example.com").await?;
    // No auto-wait: queries once; a missing node surfaces as a CDP error.
    page.find_element("a").await?.click().await?;
    page.wait_for_navigation().await?; // click() does not wait for the navigation it triggers
    let title = page.find_element("h1").await?.inner_text().await?; // Option<String>
    println!("{title:?}");

    browser.close().await?;   // Browser.close
    browser.wait().await?;    // reap the child process
    driver.await?;            // handler ends when the socket closes
    Ok(())
}
```

---

#### headless_chrome 1.0.22

Docs: <https://docs.rs/headless_chrome/1.0.22>. Source: <https://github.com/rust-headless-chrome/rust-headless-chrome/tree/0a5c307a85debc450378a1f19e4dac1838d7b22d>. MSRV 1.85. **Blocking API** on `std::thread` workers. Result type `anyhow::Result`.

##### Object model and lifecycle

- `Browser::new(LaunchOptions)` launches; `Browser::default()` launches with defaults.
  - `Browser::connect(ws_url)` connects, with a 30 s idle timeout. `connect_with_timeout(ws_url, d)` and `connect_with_root_cert` are variants.
  - `Browser` is `Clone` (an `Arc<BrowserInner>`).
  - `browser.new_tab()` returns `Arc<Tab>`; tabs are listed in `Arc<Mutex<Vec<Arc<Tab>>>>`.
  - **`Element<'a>` borrows its `Tab`.** Users who store elements in structs hit `'static` errors. A users.rust-lang.org thread from June 2025, [131088](https://users.rust-lang.org/t/how-to-deal-with-borrowed-value-does-not-live-long-enough-for-arc-in-for-loops/131088) (8 posts), was answered "'static is the only lifetime that can be used on a struct field without providing a lifetime parameter".
- **Close.** `Drop for BrowserInner` sends `Browser.close` if the browser was launched (`close_on_drop`), stops the event loop, and shuts the transport down. `Drop for TemporaryProcess` kills and waits for Chrome, then removes the temporary profile ([process.rs](https://github.com/rust-headless-chrome/rust-headless-chrome/blob/0a5c307a85debc450378a1f19e4dac1838d7b22d/src/browser/process.rs)).
  - `tab.close(fire_unload)` and `close_with_unload()` close a tab.
- **Idle disconnect.** `LaunchOptions::idle_browser_timeout` defaults to 30 s: "How long to keep the WebSocket to the browser for after not receiving any events from it".
  - Issue [#142](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/142), "RPC connection times out after a while" (open since 2019, 5 reactions, 14 comments). The original author suggested a periodic no-op DOM query as a workaround. It is still the default.

##### Element finding and waiting

- `tab.find_element(css)` makes no wait.
- `tab.wait_for_element(css)` polls `find_element` through `util::Wait::strict_until` until the default timeout. The Tab's `default_timeout` is **20 s** (`Duration::from_secs(20)` in `Tab::new`). `util::Wait` sleeps **100 ms** between polls; its own default timeout is 10 s ([util.rs](https://github.com/rust-headless-chrome/rust-headless-chrome/blob/0a5c307a85debc450378a1f19e4dac1838d7b22d/src/util.rs)).
- Other waits:
  - `wait_for_element_with_custom_timeout`, `wait_for_xpath`, `wait_for_elements`;
  - `wait_until_visible`, which polls `checkVisibility()`;
  - `wait_until_navigated`, which waits until a lifecycle `networkAlmostIdle` clears a `navigating` flag set by `navigate_to`.
- On timeout you get `util::Timeout`, displayed as "The event waited for never came", with no selector.
- "Not found" is a dedicated `NoElementFound` ("No element found"). It is mapped from Chrome's "Could not find node with given id".
- `Element::click()` scrolls into view, takes the midpoint and clicks: no actionability.
  - `get_midpoint()` falls back to a 20 s JS polling loop with **`.unwrap()` on `call_js_fn`**, a panic path in library code (from source, [element/mod.rs](https://github.com/rust-headless-chrome/rust-headless-chrome/blob/0a5c307a85debc450378a1f19e4dac1838d7b22d/src/browser/tab/element/mod.rs)).
  - `type_into` clicks then types.
- No locator, no strictness. `set_slow_motion_multiplier` imitates Puppeteer's `slowMo`.

##### Configuration

- `LaunchOptions`, via `derive_builder`, with `LaunchOptions::default_builder()`. Defaults:
  - `headless: true`, `sandbox: true`, `devtools: false`, `enable_gpu: false`, `enable_logging: false`;
  - `window_size: None`, `port: None`;
  - **`ignore_certificate_errors: true`**;
  - `path: None`, which means auto-detect, or fetch with the `fetch` feature;
  - `user_data_dir: None`, which means a temporary directory;
  - `extensions`, `args`, `ignore_default_args`, `disable_default_args`;
  - `idle_browser_timeout: 30 s`, `process_envs`, `proxy_server`.
- Lifetimes leak into the options: `LaunchOptions<'a>` holds `Vec<&'a OsStr>`.
- **Defaults have flipped in a patch release.** Issue [#500](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/500) (11 reactions): the builder default for `devtools` was `true` while `Default` said `false`. That made the "headless" crate headful by default. A user wrote they were "hit with this in production". Reported 2024-07-27, the day after 1.0.12. Closed 2024-08-20, the day 1.0.13 shipped (version linkage inferred from dates).
- Environment variable: `CHROME`, the executable path consulted by `default_executable()`.
- The default debugging port is searched between 8000 and 9000. Issue [#300](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/300), "There are no available ports between 8000 and 9000", has 12 reactions and is open (WSL2). [#267](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/267) reports a `NoAvailablePorts` panic in Alpine.

##### Errors

- `anyhow` throughout. In 2022 the crate moved from the deprecated `failure`; [#212](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/212) and [#330](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/330) describe the `.compat()` pain.
- Typed error structs exist (`NoElementFound`, `ElementNotVisible`, `NavigationFailed`, `Timeout`), but callers must `downcast` them. [#379](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/379), "Errors should be public" (open), says some, such as the builder error, cannot be named easily.

##### Events and network

- `tab.add_event_listener(Arc<SyncSendEvent>)` takes a callback, unlike chromiumoxide's streams.
- `tab.enable_request_interception(Arc<RequestIntercept>)` takes a closure `Fn(Arc<Transport>, SessionId, RequestPausedEvent) -> RequestPausedDecision`, which returns `Continue`, `Fulfill` or `Fail`, plus `enable_fetch(patterns, handle_auth)`.
- `register_response_handling(name, closure)` is also available.
- [#280](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/280), "Request interception is not working" (open, 12 comments), arose because interception needs `enable_fetch` as well; the API changed between 0.9 and 1.0.

##### Versioning

- 1.0.0 shipped December 2022, after [#323](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/323), "Why hasn't v1.0.0 been published to crates.io?" (8 reactions, 23 comments). It has had 22 patch releases since.
- The in-repo CHANGELOG stops at 0.9.0 (2019), so it is unmaintained.
- The protocol is generated at build time, with GPL-3 tooling (prior note).
- Chrome updates broke it in [#367](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/367) (Chrome 110, February 2023).
- [#489](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/489), "The `offline` cargo feature is backwards", has 19 reactions, the most of any issue, and is open.

##### Sample (1.0.22, not compiled)

```rust
use headless_chrome::{Browser, LaunchOptions};

fn main() -> anyhow::Result<()> {
    let browser = Browser::new(LaunchOptions::default_builder().headless(true).build()?)?;
    let tab = browser.new_tab()?;                       // Arc<Tab>
    tab.navigate_to("https://example.com")?.wait_until_navigated()?;
    tab.wait_for_element("a")?.click()?;               // polls up to 20 s, 100 ms interval
    tab.wait_until_navigated()?;
    let text = tab.wait_for_element("h1")?.get_inner_text()?; // Element<'_> borrows `tab`
    println!("{text}");
    Ok(()) // Drop: Browser.close, then kill + wait Chrome, delete temp profile
}
```

---

#### fantoccini 0.22.1

Docs: <https://docs.rs/fantoccini/0.22.1>. Source: <https://github.com/jonhoo/fantoccini/tree/ed1d6944e100cf36f8a00c03c7a9a5f4424e71f3>. MSRV 1.67. Tokio. A W3C WebDriver client; it needs a running geckodriver or chromedriver.

##### Object model and lifecycle

- `ClientBuilder::native()` or `::rustls()`, then `.capabilities(map)` and `.connect("http://localhost:4444")`, returns `Result<Client, NewSessionError>`. `Client::with_existing_session` reattaches.
- `Client` is `Clone`, a handle to a spawned `Session` task that owns the HTTP client ([session.rs](https://github.com/jonhoo/fantoccini/blob/ed1d6944e100cf36f8a00c03c7a9a5f4424e71f3/src/session.rs)). Methods take `&self`.
  - Before about 0.15, methods took `&mut self`. [#86](https://github.com/jonhoo/fantoccini/issues/86), "Why do most methods use a mutable reference to Self?", has 8 comments.
  - `Element` owns a `Client` clone, with no lifetime.
- **Close.** `client.close(self)` sends `DELETE /session`. `persist()` keeps the session alive.
  - When the last clone drops, the session task issues `DELETE /session` on the Tokio runtime and drops any error. The doc comment on `close` says so ([client.rs](https://github.com/jonhoo/fantoccini/blob/ed1d6944e100cf36f8a00c03c7a9a5f4424e71f3/src/client.rs)).
  - If the runtime shuts down first, for example in a test harness, the request never runs. The browser window stays open: see [#250](https://github.com/jonhoo/fantoccini/issues/250) (open, 3 reactions) and the 2026 forum thread [142477](https://users.rust-lang.org/t/close-fantoccini-client-when-an-assertion-fails-inside-an-integration-test/142477), where Rocket's test macro ended the runtime after 500 ms.
- The crate docs warn that geckodriver does not support multiple simultaneous sessions.

##### Element finding and waiting

- `client.find(Locator::Css(..))` and `find_all` are single-shot WebDriver Find Element calls. `Locator` has `Css`, `Id`, `LinkText` and `XPath`.
- `client.wait()` returns a `Wait<'c>` builder with `.at_most(d)`, `.every(d)`, `.forever()`, `.for_element(locator)` and `.for_url(url)`.
  - Defaults: `DEFAULT_TIMEOUT` = **30 s**, `DEFAULT_PERIOD` = **250 ms** ([wait.rs](https://github.com/jonhoo/fantoccini/blob/ed1d6944e100cf36f8a00c03c7a9a5f4424e71f3/src/wait.rs)).
  - A timeout returns `CmdError::WaitTimeout`, whose `Display` is the bare description "timeout waiting on condition: ", with no locator.
  - The older `wait_for_find` (which waited forever), `wait_for` and `wait_for_navigation` were deprecated in 0.17.5. [#81](https://github.com/jonhoo/fantoccini/issues/81) (2020): "if the element never appears, my automated tests spin forever".
- WebDriver Classic's click does scroll into view and interception checks server-side, but fantoccini does not retry them.
- Forms get special support: `client.form(locator)` returns a `Form` with `set_by_name`, `submit`, `submit_with`, `submit_sneaky`.
- `raw_client_for` makes HTTP requests carrying the session's cookies.

##### Configuration

Capabilities are a JSON map: standard W3C keys plus vendor keys such as `goog:chromeOptions` and `moz:firefoxOptions`. There are no crate-level defaults beyond those of WebDriver itself. The features are `native-tls` (default) and `rustls-tls`.

##### Errors

- `CmdError` has `Standard(WebDriver)` for spec errors, with `is_no_such_element()`-style predicates, plus `BadUrl`, `Failed`, `Lost`, `NotJson`, `Json`, `NotW3C`, `InvalidArgument`, `WaitTimeout` and decode errors. `NewSessionError` is separate.
- stevepryde rated fantoccini's error handling as better than thirtyfour's ([#145](https://github.com/jonhoo/fantoccini/issues/145)).
- A 2022 thirtyfour discussion ([thirtyfour #117](https://github.com/stevepryde/thirtyfour/issues/117)) complains that fantoccini "intentionally" keeps the WebDriver error code private.
- Library panics reported:
  - [#120](https://github.com/jonhoo/fantoccini/issues/120): "entered unreachable code: received unknown error (timeout)" (2021);
  - [#263](https://github.com/jonhoo/fantoccini/issues/263): a panic when the WebDriver is unreachable, a regression in 0.21.0 (2024);
  - [#281](https://github.com/jonhoo/fantoccini/issues/281): `Option::unwrap()` on `None` in the `basic` example (2024, 2 reactions);
  - [#135](https://github.com/jonhoo/fantoccini/issues/135): a panic on "DevToolsActivePort file doesn't exist".

##### Events and network

There are none: WebDriver Classic has no events. Network interception was declined: "isn't part of the WebDriver spec … not something fantoccini will support" ([#222](https://github.com/jonhoo/fantoccini/issues/222), 4 reactions, 2023).

##### Versioning

- crates.io: 0.19.0 (April 2022) → 0.20.0-rc.1…rc.7 (October 2022 to September 2023) → 0.20.0 and 0.21.0 (both July 14, 2024) → 0.22.0 (June 2025) → 0.22.1 (February 2026). Slow and stable, with long release-candidate cycles.
- There is no CHANGELOG file in the crate and there are no GitHub releases.

##### Sample (0.22.1, not compiled)

```rust
use fantoccini::{ClientBuilder, Locator};

#[tokio::main]
async fn main() -> Result<(), fantoccini::error::CmdError> {
    // Requires chromedriver/geckodriver already listening; connect() has its own error type.
    let c = ClientBuilder::native().connect("http://localhost:4444").await
        .expect("failed to connect to WebDriver");
    c.goto("https://example.com").await?;
    c.wait().for_element(Locator::Css("a")).await?.click().await?; // 30 s / 250 ms
    let text = c.wait().for_element(Locator::Css("h1")).await?.text().await?;
    println!("{text}");
    c.close().await // otherwise DELETE /session is fire-and-forget on drop
}
```

---

#### thirtyfour 0.37.8 (with `bidi`)

Docs: <https://docs.rs/thirtyfour/0.37.8>. Source: <https://github.com/stevepryde/thirtyfour/tree/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour>. MSRV 1.88, enforced in CI since 0.37.0. Tokio only. Built on fantoccini in 0.29–0.31 (2022, per crates.io dependencies); on its own `reqwest` stack since 0.32.0 (April 2024).

##### Object model and lifecycle

- **Construction.**
  - `WebDriver::new(server_url, caps)` connects to a running driver or Grid.
  - **`WebDriver::managed(caps)`** returns a `WebDriverManagerBuilder` that implements `IntoFuture`, so `WebDriver::managed(caps).await?` works. It **auto-downloads the matching chromedriver or geckodriver**, starts it, and stops it when the session ends (the `manager` feature, on by default).
  - `WebDriverManager::builder().latest().build()` can share one driver process across sessions.
- `WebDriver` is `Clone` and derefs to `Arc<SessionHandle>`. `WebElement` holds a handle, with no lifetime.
  - Early versions used lifetimes so that "no WebElement can outlive the session". The author discussed that design on the forum in 2020 ([49499](https://users.rust-lang.org/t/async-lifetimes-and-references-owned-by-the-current-function/49499)) and later dropped it.
- **Close and Drop.** `driver.quit(self).await` sends `DELETE /session` and releases the managed driver guard. If you forget `quit()`, `Drop for SessionHandle`:
  - logs `tracing::warn!("WebDriver was not quit properly — falling back to synchronous teardown, which blocks the async executor. Call WebDriver::quit().await instead.")`;
  - spawns an OS thread with a fresh current-thread Tokio runtime and a rebuilt HTTP client;
  - **joins it synchronously** ([session/handle.rs](https://github.com/stevepryde/thirtyfour/blob/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour/src/session/handle.rs)). A long comment explains why a shared global runtime caused chromedriver readiness failures under parallel tests.
  - The crate docs say it bluntly: "If you do not call `WebDriver::quit` your async executor will be blocked".
  - History: [#7](https://github.com/stevepryde/thirtyfour/issues/7) (2020, 16 comments; `block_on` deadlocked on Tokio's basic scheduler, `quit()` added as the workaround); [#19](https://github.com/stevepryde/thirtyfour/issues/19) (2020, endless blocking inside `tokio::spawn`); [#268](https://github.com/stevepryde/thirtyfour/issues/268) (2025, 19 comments; a new drop logic panicked with "A Tokio 1.x context was found, but it is being shutdown").
- **Scoped test helper.** `testing::run_browser_test(session_future, |driver| async move { … })` always attempts `quit()` after the body returns, errors or **unwinds**. The closure receives `WebDriver` **by value**, which avoids the higher-ranked lifetime problem users hit with `FnOnce(&WebDriver) -> Fut` (forum [72402](https://users.rust-lang.org/t/async-closure-with-borrowed-arg/72402), 2022; fantoccini equivalent [79745](https://users.rust-lang.org/t/impaled-on-the-horns-of-a-lifetime/79745)).

##### Element finding and waiting

- `driver.find(By)` is single-shot.
- **`driver.query(By)` returns an `ElementQuery`**, "the recommended way to find elements".
  - Selectors: `By::Id`, `Css`, `XPath`, `Tag`, `ClassName`, `Name`, `LinkText`, `Testid`, …
  - Branches: `.or(By)`.
  - Filters: `.with_text`, `.with_class(StringMatch::new(..).word())`, `.and_enabled()`, `.and_displayed()`, custom predicates. Each filter costs one extra WebDriver request per poll.
  - `.desc("…")` names the target for error messages.
  - `.wait(timeout, interval)`, `.nowait()`, `.with_poller(..)`.
- **Terminals** ([element_query.rs](https://github.com/stevepryde/thirtyfour/blob/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour/src/extensions/query/element_query.rs)):
  - `first()` and `first_opt()`;
  - **`single()`**: polls until **at least one** element matches, then fails if the count is not 1 ("too many elements received; found {n} … using selectors: …"). It does not wait until exactly one matches;
  - `any()` and `any_required()`, `all_from_selector()` and `all_from_selector_required()`;
  - `exists()` and `not_exists()`.
- Default poller: `ElementPollerWithTimeout::default()` = **20 s timeout, 500 ms interval** ([poller.rs](https://github.com/stevepryde/thirtyfour/blob/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour/src/extensions/query/poller.rs)). It can be configured globally through `WebDriverConfig::builder().poller(..)`. The HTTP request timeout defaults to 120 s.
  - Note: the 2022 maintainer reply in #137 says "30 seconds (the default timeout)", and the 2020 dev.to post says "30 second implicit wait". The source today says 20 s, so the default changed or was misremembered.
- **`ElementWaiter`**: `elem.wait_until().displayed() / not_displayed() / enabled() / clickable() / conditions(vec![..])`, with an optional `.error("…")` message.
- **Components**: `#[derive(Component)]` with `ElementResolver<WebElement>` fields, `#[by(css = "...", first)]`, which "automatically re-query stale elements". A page-object model.
- There is no auto-wait inside `click()`. Server-side WebDriver checks return `ElementClickIntercepted` or `ElementNotInteractable` as typed errors.

##### Configuration

- `DesiredCapabilities::chrome()` or `firefox()`, with generated setters such as `set_headless()` and `add_arg`. In 0.37, `Capabilities` became a newtype and `insert_browser_option` was renamed `set_browser_option` ([MIGRATION.md](https://github.com/stevepryde/thirtyfour/blob/main/MIGRATION.md)).
- `WebDriverConfig::builder()` offers `keep_alive`, `poller`, `user_agent` and `request_timeout` (120 s).
- Features: `rustls-tls` (default), `native-tls`, `component` (default), `cdp` (default), `cdp-events`, `bidi`, `manager` (default).

##### Errors

- `WebDriverResult<T> = Result<T, WebDriverError>`. `WebDriverError` is a `Box<WebDriverErrorInner>` with one variant per W3C error: `NoSuchElement`, `StaleElementReference`, `ElementClickIntercepted`, `ElementNotInteractable`, `Timeout(String)`, and others. Each carries `WebDriverErrorInfo` (status, error, message, stacktrace).
- History: [#50](https://github.com/stevepryde/thirtyfour/issues/50) (2021), "Unusable error message when anything but 200 OK is returned"; fixed by adding the status and body.
- A beginner's 2025 forum question, "how to find StaleElementReference?" ([124943](https://users.rust-lang.org/t/thirtyfour-how-to-find-staleelementreference/124943)), shows a raw Chrome stack trace in the error, with no Rust context.

##### Events and network (CDP and BiDi)

- `driver.cdp()` gives typed CDP commands through chromedriver's `goog/cdp/execute`. The `cdp-events` feature adds `CdpSession` event streams over a WebSocket. Both are rewritten in 0.37.0 (PR #308).
- **BiDi**, behind the `bidi` feature ([bidi/mod.rs](https://github.com/stevepryde/thirtyfour/blob/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour/src/bidi/mod.rs)):
  - It needs `caps.enable_bidi()?`, which sets the `webSocketUrl: true` capability.
  - `driver.bidi().await?` lazily dials the WebSocket and caches it. It returns a `BiDi` that is `Clone` and `Arc`-backed.
  - Facades exist for every W3C module (`session`, `browsingContext`, `network`, `script`, `input`, `storage`, `log`, `emulation`, `webExtension`, `permissions`).
  - `bidi.subscribe::<E>()` returns a typed `Stream` and sends `session.subscribe` automatically; dropping the last stream sends `unsubscribe` in the background. `send_raw` and `subscribe_raw` are untyped escape hatches.
  - **Network**: observe `BeforeRequestSent`, `ResponseStarted`, `ResponseCompleted` and `FetchError`; intercept with `AddIntercept`, then `ContinueRequest`, `FailRequest`, `ProvideResponse`, `ContinueResponse` or `ContinueWithAuth`; capture bodies with `AddDataCollector` and `GetData`; and use `SetExtraHeaders` and `SetCacheBehavior`.
  - The `InterceptGuard` returned by an intercept removes it on `Drop`, but only "best-effort: spawn the removal on the current tokio runtime. If no runtime is current, the intercept stays registered" ([network.rs](https://github.com/stevepryde/thirtyfour/blob/17017c844f1bf9c912b8b100a8e47821530ec2d7/thirtyfour/src/bidi/modules/network.rs)). The same async-drop compromise again.
- On Chrome, BiDi runs through chromedriver's chromium-bidi mapper over CDP (prior note).

##### Versioning

- Release history: 0.32.0 had ten release candidates (October 2022 to April 2024); then 0.33 (July 2024), 0.34 (August 2024), 0.35 (November 2024), 0.36 (June 2025), 0.37.0 (May 3, 2026), and 0.37.1–0.37.8 (to October 12, 2026).
- 0.37.0 was a large breaking release with a migration guide: a builder construction API, newtype `Capabilities`, removed deprecated names, privatised fields, deprecated `extensions::cdp`.
- Maintenance wobbled: [#215](https://github.com/stevepryde/thirtyfour/issues/215), "Maintenance Status" (8 reactions, April 2024): the author was "stepping back". Vrtgs joined. In 2026 stevepryde is shipping again.

##### Sample (0.37.8, not compiled)

```rust
use thirtyfour::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let driver = WebDriver::managed(DesiredCapabilities::chrome()).await?; // downloads + runs chromedriver
    driver.goto("https://example.com").await?;
    driver.query(By::Css("a")).desc("first link").first().await?.click().await?; // polls 20 s / 500 ms
    let h1 = driver.query(By::Tag("h1")).desc("page heading").single().await?;   // errors if >1 once found
    println!("{}", h1.text().await?);
    driver.quit().await?; // without this, Drop blocks the executor on a side runtime
    Ok(())
}
```

---

#### Brief notes: playwright-rs, rustwright, chromey, octaltree `playwright`

##### playwright-rs 0.20.0 (`padamson/playwright-rust`)

Docs: <https://docs.rs/playwright-rs/0.20.0>. MSRV 1.88. Apache-2.0. Created November 2025; 160 stars; 26 published versions; about 151k downloads over 90 days.

- **Architecture.** A JSON-RPC client over stdio to Microsoft's Node Playwright driver, version 1.64.0. The driver, about 130 MB, is assembled by the build script on first compile, so that step needs network access.
  - `PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD=1` and `PLAYWRIGHT_DRIVER_CACHE_DIR` are the knobs.
  - `install_browsers()` and `install_browsers_with_deps()` pin browser builds to the crate version: "the browser version rides `Cargo.lock`" (README).
- **API.** It mirrors playwright-python:
  - `Playwright::launch()`, then `pw.chromium().launch()`, then `browser.new_page()`, then `page.goto(url, None)`.
  - **`page.locator(sel)` is synchronous and lazy since 0.15.0** (CHANGELOG: it removed "the `page.locator("x").await.click(None).await` double-await that consumers flagged while dogfooding"). The crate README's quick-comparison block still shows `page.locator("h1").await`, so the README is stale.
  - Option-heavy methods take `impl Into<Option<XOptions>>`.
  - A `locator!()` macro validates selectors at compile time.
  - Since 0.14.0, option structs and `Error` are `#[non_exhaustive]` (ADR 0005 in that repo).
- **Auto-wait and strictness come from the driver.** `Locator` actions use `strict=true`. 0.20.0 made `Locator::screenshot` wait and enforce strictness like the other actions. `DEFAULT_TIMEOUT_MS` = 30,000.
- **Errors.** `playwright_rs::Error` has `Timeout(String)`, `NavigationTimeout { url, duration_ms }`, `TargetClosed`, `ElementNotFound`, `AssertionTimeout`, `ProtocolError(String)`, `Context(String, Box<Error>)`, among others.
  - Locator actions append `[selector: …]` to the message string (`wrap_error_with_selector`), not to a field.
  - Until a recent release, timeouts arrived as `ProtocolError`, "so callers had to string-match `"TimeoutError"`" (CHANGELOG).
- **Lifecycle.** `Drop for Playwright` closes the driver's stdin and **blocks** up to the exit grace period, because "a non-blocking `Drop` is what leaked browsers before". Prefer `playwright.shutdown().await`.
  - It also restores the terminal after [#59](https://github.com/padamson/playwright-rust/issues/59): Ctrl-C left the shell broken because Node's stderr was inherited from the tty (fixed April 2026).
- **Cadence.** 0.x with frequent breaking changes: five sections are flagged breaking in the CHANGELOG, and releases come roughly every two weeks.

##### rustwright 0.3.0 (`Skyvern-AI/rustwright`)

Docs: <https://docs.rs/rustwright/0.3.0>.

- **Positioning.** "A Rust rewrite of Playwright", on an in-process Rust CDP engine. Alpha; Chromium only; 918 stars, repository created July 2026.
- **The primary consumers are Python and Node**, as a drop-in import swap. The Rust crate has 216 downloads over 90 days.
- **Rust API.** "Idiomatic, **synchronous** Rust API … callers do not need Tokio": the core owns its own async runtime.
  - `chromium().launch(LaunchOptions)` or `connect_over_cdp(ConnectOptions::new(endpoint).timeout(..))` (60 s default attach timeout), then `browser.new_page()`, then `page.goto(url, GotoOptions)`, then `page.click(selector, ActionOptions)`.
  - Actions take **selector strings on `Page`**; this native crate exposes no `Locator` type.
  - Every operation has a `*_with_cancel(.., Option<&CancelToken>)` twin. Cancellation is explicit because there are no futures to drop, so the API surface doubles.
  - Events come as polling receivers (`events().recv_timeout(..)`, `dropped_count()`).
  - Errors are structured: `ActionTimeoutError`, `ActionabilityError`, `FailureKind`, `FailurePhase`, `FailureTargetKind`.
- **User issues** show the engine relearning Playwright semantics:
  - [#79](https://github.com/Skyvern-AI/rustwright/issues/79): `wait_for` fails with "Inspected target navigated or closed" when a navigation commits mid-wait ("Playwright survives it"), fixed in a day;
  - [#195](https://github.com/Skyvern-AI/rustwright/issues/195): a raw CDP error surfaced by the actionability probe;
  - [#187](https://github.com/Skyvern-AI/rustwright/issues/187): `fill()` invisible to React controlled inputs;
  - [#186](https://github.com/Skyvern-AI/rustwright/issues/186): inconsistent shadow-DOM piercing;
  - [#211](https://github.com/Skyvern-AI/rustwright/issues/211): about 30% CI flakes on navigation timeouts;
  - [#212](https://github.com/Skyvern-AI/rustwright/issues/212): crates.io lags the other registries.
- **HN launch** ([48923788](https://news.ycombinator.com/item?id=48923788), July 15, 2026, 12 points). The author describes the rewrite as heavily agent-written, with about 10 billion tokens.

##### chromey 2.58.2 (`spider-rs/chromey`)

- Same library name, `chromiumoxide`, and the same `(Browser, Handler)` model with the handler-spawn loop (README).
- Adds conveniences missing upstream: `wait_for_selector(sel, Option<Duration>)` (30 s default; backoff 0/20/50/100/100/500 ms), `wait_for_network_idle`, `wait_for_network_almost_idle`, `wait_for_dom_content_loaded`, `wait_for_load`.
- **`wait_for_selector` retries on any `find_element` error and returns a bare `CdpError::Timeout`** with no selector (from source, [page.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/page.rs)).
- `Page` has 198 `pub async fn`s, including many `goto_with_cache_*` variants: the surface grows with Spider's needs.
- A `bg_cleanup` module routes `Drop`-time async work to one background task, to avoid `tokio::spawn` panics in `Drop`. It is a documented pattern worth borrowing ([bg_cleanup.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/bg_cleanup.rs)).

##### octaltree `playwright` 0.0.20 (August 2022): abandoned

- [#30](https://github.com/octaltree/playwright-rust/issues/30), "Is this project still maintained?" (18 comments). In May 2022 the author said it "works fine as long as the test passes, but I do not recommend it for production".
- A 2025 fork under omnidotdev was archived within months: "it ended up not being worth the effort".
- [#11](https://github.com/octaltree/playwright-rust/issues/11) (13 comments): `expect_event(Load)` hung forever.
- [#38](https://github.com/octaltree/playwright-rust/issues/38): the dev branch was "no longer Send".

---

#### Cross-crate comparison of API dimensions

| Dimension | chromiumoxide 0.9.1 | headless_chrome 1.0.22 | fantoccini 0.22.1 | thirtyfour 0.37.8 | playwright-rs 0.20.0 | rustwright 0.3.0 |
|---|---|---|---|---|---|---|
| Protocol | CDP | CDP (non-flat sessions) | WebDriver Classic | WebDriver Classic + CDP + BiDi | Playwright JSON-RPC to Node driver | CDP, in-process |
| Sync/async | async, Tokio | blocking | async, Tokio | async, Tokio | async, Tokio | **sync** (own runtime) |
| Driver to run | none (spawns Chrome) | none | geckodriver/chromedriver | auto via `WebDriver::managed` | bundled Node driver (130 MB) | none |
| Background task the user must drive | **yes**: poll `Handler` | no (threads) | no (crate spawns one) | no | no | no |
| Handles | `Page`/`Element` via Arc; `Browser` not Clone | `Browser` Clone; `Arc<Tab>`; **`Element<'a>`** | `Client` Clone; `Element` owns a Client | `WebDriver` Clone (Arc); `WebElement` | Arc handles | handles |
| Auto-wait in actions | no | no | no | no (typed WebDriver errors) | **yes** (driver) | **yes** |
| Explicit wait primitive | none (chromey adds `wait_for_selector`) | `wait_for_element` 20 s / 100 ms | `wait().for_element` 30 s / 250 ms | `query()` 20 s / 500 ms; `wait_until()` | locator actions 30 s | action timeouts |
| Locator object | no | no | `Locator` enum = selector only | `ElementQuery` (filters, `or`, `desc`) + Components | lazy `Locator`, strict | selector strings |
| Strictness | none (first match) | none | none | `single()` checks after the first match | strict on actions | Playwright-like |
| Default command timeout | 30 s, **hard-coded for page commands** | 30 s idle disconnect; 20 s waits | WebDriver-side | 120 s HTTP | 30 s | per action |
| Errors | `CdpError` (thiserror); `String` from config builder | `anyhow` + downcastable structs | `CmdError`, `NewSessionError` | `WebDriverError` (W3C codes) | `#[non_exhaustive] Error` | structured failure kinds |
| Close | `close()` + `wait()`; Drop warns, kill_on_drop | Drop closes + kills | `close()`; Drop = fire-and-forget DELETE | `quit()`; Drop **blocks** on a side runtime; `run_browser_test` | `shutdown()`; Drop blocks ≤ grace | `close()` |
| Network interception | config flag + raw `Fetch` events/commands | closure returning `RequestPausedDecision` | **not supported** | CDP events or BiDi `network` intercepts | Playwright `route` | network records |
| MSRV | 1.85 | 1.85 | 1.67 | 1.88 | 1.88 | n/a |

---

#### Likes (short quotes, verified at the link)

- **thirtyfour's query builder.** "I've grown quite fond of the query builder" (bcpeinhardt, [fantoccini #145](https://github.com/jonhoo/fantoccini/issues/145), December 29, 2021). He also liked that it "keeps locators simpler/reusable".
- **fantoccini's design**, rated by the competing author: "Error handling seems much nicer in `fantoccini`" (stevepryde, same thread). He also praised the "Clean / clear API" and the many links to the W3C spec.
- **fantoccini's ease.** "I've found fantoccini easy as heckfire and delightful to use." ([#250](https://github.com/jonhoo/fantoccini/issues/250), May 2024). The same post is the top complaint about cleanup.
- **fantoccini in production.** "I've been using Fantoccini … to great success!" (roblabla, [HN 25417405](https://news.ycombinator.com/item?id=25417405), December 2020). The reason given: WebDriver works across Chrome, Firefox, Edge and Safari.
- **chromiumoxide's generated protocol.** "Amazing work. I was writing CDP requests by hand the other day." (fleetfox, [HN 25417700](https://news.ycombinator.com/item?id=25417700)). The Show HN drew 127 points ([25416686](https://news.ycombinator.com/item?id=25416686)).
- **chromiumoxide versus Go.** "a better experience than I had with go and chromedp" ([dev.to, June 27, 2024](https://dev.to/adaschevici/rustify-some-puppeteer-code-3n33); quote as returned by a summarising fetch, **wording unverified**). The same post chose chromiumoxide because it was "most active in terms of development". It rejected fantoccini because the author wanted CDP, and headless_chrome as less maintained.
- **thirtyfour for crawling and testing.** "I made a basic crawler using Firefox, thirtyfour … and squid" ([HN 39811263](https://news.ycombinator.com/item?id=39811263), 2024). It is also used for Rust web integration tests ([HN 35653049](https://news.ycombinator.com/item?id=35653049), 2023).
- **headless_chrome for form filling.** "I'm using this to fill forms interactively and emulate a user." ([HN 39845008](https://news.ycombinator.com/item?id=39845008), 2024). It was also recommended for SPA scraping on the Rust forum ([47462](https://users.rust-lang.org/t/scraping-ajax-spa-with-rust/47462), 2020).
- **playwright-rs responsiveness.** "The issue is now fixed! Thanks a lot!" ([#59](https://github.com/padamson/playwright-rust/issues/59), April 2026; fixed within 3 days).

##### "Why I chose X over Y"

- **Async and generated types.** "chromiumoxide uses async rust by design and … all the devtools protocol types are generated" (mattsse on headless_chrome, [HN 25419173](https://news.ycombinator.com/item?id=25419173), 2020). He also called headless_chrome "abandoned by its author" at the time.
- **Cross-browser reach versus depth.** In [HN 25417405](https://news.ycombinator.com/item?id=25417405), WebDriver (fantoccini) is preferred for cross-browser reach and CDP (chromiumoxide) for "deep introspection".
- **Maintenance signals.** headless_chrome was dropped on the forum in 2020 ([47462](https://users.rust-lang.org/t/scraping-ajax-spa-with-rust/47462)): "isn't maintained … still uses failure for error handling".
- **Playwright's driver model.** playwright-rs [WHY.md](https://github.com/padamson/playwright-rust/blob/main/WHY.md) says the alternatives are "outdated protocols (WebDriver)", "Chrome-only (CDP-based tools)" or "not designed for modern web testing". It chose the driver model for "Feature Parity" and "Low Maintenance".
- **Giving up on Rust bindings.** The 2025 octaltree fork was dropped because one "can easily write tests alongside your Rust code in a Microsoft-supported language or use chromiu[moxide]" ([#30](https://github.com/octaltree/playwright-rust/issues/30)).
- **Rustwright's pitch.** No Node driver, "70% less memory", "2.55× faster", and "no Playwright automation fingerprint" (repository README; vendor claims, not checked).
- **Demand for Playwright in Rust.** [microsoft/playwright #18266](https://github.com/microsoft/playwright/issues/18266) (15 reactions, October 2022), "Create bindings for Rust", was closed: "out of scope for Playwright … community driven project". [#30675](https://github.com/microsoft/playwright/issues/30675) (4 reactions, 2024) was folded into it. [#34216](https://github.com/microsoft/playwright/issues/34216) (January 2025) said the "unofficial playwright crates in rust … not working properly"; the reply: "There's no plan to extend to Rust, no." padamson later advertised playwright-rs on #18266 and #30675 (January 2026).

---

#### Pain points, ranked by weight of evidence

Weight means independent reports, reactions and comments, recency, and whether the problem is still open. Status is as of October 14, 2026.

##### P1. Teardown, leaks and async `Drop`: about 16 reports, 5 crates plus forum threads, 2018–2026

**Reports:**
- headless_chrome:
  - [#89](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/89), "Chrome instances are left running" (open since 2019, 5 reactions, 14 comments). Dozens of Chrome processes pile up after failed tests. A commenter notes that `Drop` cannot run on `panic=abort`, SIGKILL, OOM or Ctrl-C, and suggests a pipe or parent-death signal.
  - [#74](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/74), a tab outliving its browser (open).
  - [#142](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/142), using a `Tab` after dropping the `Browser` returns `ConnectionClosed`.
- chromiumoxide:
  - [#158](https://github.com/mattsse/chromiumoxide/issues/158) (2023): users closing the app leave headless Chrome running. The maintainer: "not something the library can handle on its own"; use a watchdog process.
  - [#251](https://github.com/mattsse/chromiumoxide/issues/251) (open): SIGINT kills the browser during graceful shutdown, since the child shares the process group; spawning it in its own process group was suggested.
  - [#137](https://github.com/mattsse/chromiumoxide/issues/137): `block_on` in `Drop` panicked under Tokio.
  - [#287](https://github.com/mattsse/chromiumoxide/issues/287) (open): an error log on a clean `close()`.
  - The `Drop` warning in source.
- thirtyfour:
  - [#7](https://github.com/stevepryde/thirtyfour/issues/7) (2020–2024, 16 comments);
  - [#19](https://github.com/stevepryde/thirtyfour/issues/19) (2020);
  - [#268](https://github.com/stevepryde/thirtyfour/issues/268) (2025, 19 comments).
- fantoccini:
  - [#5](https://github.com/jonhoo/fantoccini/issues/5), "Are explicit `drop`s necessary?" (2018);
  - [#250](https://github.com/jonhoo/fantoccini/issues/250), closing the client on test failure (open, 3 reactions). jonhoo: "plagues the async ecosystem … no current good solutions".
- Forum threads:
  - [142477](https://users.rust-lang.org/t/close-fantoccini-client-when-an-assertion-fails-inside-an-integration-test/142477) (September 2026): the reply offers "two bad choices", `catch_unwind` or `block_on` in `Drop`;
  - [79745](https://users.rust-lang.org/t/impaled-on-the-horns-of-a-lifetime/79745) (2022): `close(self)` cannot be called through an `Arc`;
  - [72402](https://users.rust-lang.org/t/async-closure-with-borrowed-arg/72402) (2022): an HRTB closure is needed to guarantee `quit()`.
- playwright-rs [#59](https://github.com/padamson/playwright-rust/issues/59) (2026): Ctrl-C left the terminal broken.
- Blog ([dev.to 2024](https://dev.to/adaschevici/rustify-some-puppeteer-code-3n33)): cleanup "needs to be done correctly".

**Status.** Mitigated, not solved:
- thirtyfour `run_browser_test` plus a blocking fallback;
- playwright-rs `shutdown()` plus a blocking `Drop`;
- chromey's background cleanup worker;
- everyone else: documentation.

**Lessons for Goodfetch:**
1. Expose `close().await`, consuming or idempotent, through `&self` so it works through `Arc`.
2. Provide a scoped helper that passes the handle **by value** to an async closure and always closes, even on unwind.
3. Make `Drop` a best-effort fallback that warns, never panics, and never needs a current runtime. Use a background cleanup channel like chromey's.
4. Because Goodfetch's engine is a separate process, put the guarantee **engine-side**: when the SDK connection drops, the engine closes that SDK's sessions and reaps the browsers. Spawn browsers in their own process group with parent-death or pipe-close detection.
5. In tests, document `#[tokio::test(flavor = "multi_thread")]` caveats.

##### P2. Hangs, and timeouts that do not apply: about 17 reports, mostly chromiumoxide

**Reports:**
- chromiumoxide:
  - [#333](https://github.com/mattsse/chromiumoxide/issues/333), `request_timeout` has no effect on `goto` (open; **verified in source**);
  - [#228](https://github.com/mattsse/chromiumoxide/issues/228), configured timeouts ignored (open);
  - [#52](https://github.com/mattsse/chromiumoxide/issues/52), "Never get timed out" (2021);
  - [#64](https://github.com/mattsse/chromiumoxide/issues/64), the browser hangs forever after a `new_page` failure (2021);
  - [#240](https://github.com/mattsse/chromiumoxide/issues/240), `new_page` blocks (6 reactions, 2024; the cause was cached cookies);
  - [#178](https://github.com/mattsse/chromiumoxide/issues/178), "Times Out On Any Website Using `new_page()`" (9 comments, 2023; a `sleep` "fixes" it);
  - [#191](https://github.com/mattsse/chromiumoxide/issues/191), `goto` hangs (open);
  - [#293](https://github.com/mattsse/chromiumoxide/issues/293), hangs when clearing cookies (open);
  - [#320](https://github.com/mattsse/chromiumoxide/issues/320), "Click Hangs Indefinitely While Js Click Does Not" (open, April 2026);
  - [#321](https://github.com/mattsse/chromiumoxide/issues/321), `save_screenshot` hangs (open);
  - [#292](https://github.com/mattsse/chromiumoxide/issues/292), `find_element` hangs on an invisible element (open; the reporter guesses it waits for visibility, which the source does not show: **unverified** cause);
  - [#49](https://github.com/mattsse/chromiumoxide/issues/49), Chrome idle forever;
  - [#50](https://github.com/mattsse/chromiumoxide/issues/50), a deadlock after hours of load;
  - [#163](https://github.com/mattsse/chromiumoxide/issues/163), iframes stop pages loading (28 comments; the auto-attach workaround);
  - prior note: on a clean socket close, in-flight commands wait for the 30 s eviction.
- headless_chrome:
  - [#142](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/142), a 30 s idle disconnect (open since 2019);
  - [#264](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/264), a multi-tab timeout (13 comments).
- rustwright [#211](https://github.com/Skyvern-AI/rustwright/issues/211): navigation timeouts flake about 30% of CI runs (open).

**Status.** Mostly open in chromiumoxide; fixes are unmerged since April 2026 (prior note).

**Lessons for Goodfetch:**
1. Every call has a deadline, from one hierarchy: call, then page, then context, then client.
2. No hidden constant, and no idle disconnect by default.
3. Fail fast on target crash or detach.
4. A timeout error names the operation, the target or selector, the elapsed time and the effective limit, and says where the limit came from.
5. A user's `tokio::time::timeout` around a call must cancel the engine-side operation (the JSON-RPC cancel).

##### P3. Waiting and readiness semantics: about 13 reports, 6 crates

**Reports:**
- chromiumoxide:
  - [#36](https://github.com/mattsse/chromiumoxide/issues/36), `waitUntil: networkidle` (open since 2021, 10 comments);
  - [#184](https://github.com/mattsse/chromiumoxide/issues/184), `wait_for_navigation` returns before assets load; users hand-roll last-request timers or `readyState` polling;
  - [#328](https://github.com/mattsse/chromiumoxide/issues/328), wait for a stable DOM (open, 2026);
  - the 2024 blog: polling "was not exactly part of the chromiumoxide api".
- headless_chrome [#456](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/456), `wait_for_network_idle` (open, 4 reactions).
- thirtyfour:
  - [#275](https://github.com/stevepryde/thirtyfour/issues/275) (17 comments, 2025): "poller appears to be ignored". The cause was `single()`'s semantics. The maintainer: "The expected behaviour is also a little ambiguous". The error message was "a little confusing".
  - [#122](https://github.com/stevepryde/thirtyfour/issues/122) (15 comments): the query timeout was not discoverable.
  - [#137](https://github.com/stevepryde/thirtyfour/issues/137) (9 comments): `exists()` and `not_exists()` had a bug on timeout.
- fantoccini:
  - [#27](https://github.com/jonhoo/fantoccini/issues/27), improve waits (2018);
  - [#81](https://github.com/jonhoo/fantoccini/issues/81), "my automated tests spin forever" (2020).
- octaltree [#11](https://github.com/octaltree/playwright-rust/issues/11): waiting for `load` hung (13 comments).
- rustwright [#79](https://github.com/Skyvern-AI/rustwright/issues/79): waits die on a mid-wait navigation.

**Status.** Only the Playwright-family crates auto-wait. chromey added `wait_for_network_idle`.

**Lessons for Goodfetch:**
1. Actions auto-wait for actionability, and waits survive navigation and context destruction.
2. Name the strictness contract explicitly. Playwright fails an action when more than one element matches. thirtyfour's `single()` polls for "≥1, then require ==1", which surprised users.
3. Offer `wait_for_load_state` with `load`, `domcontentloaded` and `networkidle` as first-class options.
4. Make the default timeout and polling visible in docs and in errors.

##### P4. Error ergonomics: about 16 reports, 6 crates

**Reports:**
- chromiumoxide:
  - [#259](https://github.com/mattsse/chromiumoxide/issues/259), a `String` error from the builder (open);
  - [#203](https://github.com/mattsse/chromiumoxide/issues/203) and [#218](https://github.com/mattsse/chromiumoxide/issues/218), "Node is either not visible or not an HTMLElement": a raw CDP message for a stale README selector;
  - `CdpError::Timeout` shown as "Request timed out." (source);
  - chromey's `wait_for_selector` returns a bare `Timeout` (source).
- headless_chrome:
  - [#212](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/212) and [#330](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/330), `failure` errors not `std::error::Error`;
  - [#379](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/379), private errors (open);
  - "The event waited for never came" (source).
- thirtyfour:
  - [#50](https://github.com/stevepryde/thirtyfour/issues/50), an unusable error on non-200 responses (2021);
  - [#117](https://github.com/stevepryde/thirtyfour/issues/117), the error model lost detail after rebasing on fantoccini (2022);
  - forum [124943](https://users.rust-lang.org/t/thirtyfour-how-to-find-staleelementreference/124943), a raw stack trace with no Rust context (2025).
- fantoccini:
  - panics in [#120](https://github.com/jonhoo/fantoccini/issues/120), [#263](https://github.com/jonhoo/fantoccini/issues/263), [#281](https://github.com/jonhoo/fantoccini/issues/281) and [#135](https://github.com/jonhoo/fantoccini/issues/135);
  - `WaitTimeout` with an empty detail (source).
- playwright-rs: timeouts arrived as `ProtocolError` and needed string matching (CHANGELOG 0.19/0.20); selector context is a string suffix.
- rustwright [#195](https://github.com/Skyvern-AI/rustwright/issues/195): raw CDP errors leaked through actionability.
- Panics in library paths found in source: chromiumoxide `connect_with_config` `unwrap()` and launch-cleanup `expect`; headless_chrome `get_midpoint` `unwrap()`.

**Status.** Improving: thirtyfour `.desc()`, playwright-rs typed `Timeout` and `#[non_exhaustive]`.

**Lessons for Goodfetch:**
1. One `#[non_exhaustive]` error enum implementing `std::error::Error`.
2. Stable machine codes shared with the TS and Python SDKs.
3. Structured fields: operation, selector or locator description, URL, timeout, elapsed time, a retry hint.
4. Never surface raw protocol text as the primary message; keep it as `source()`.
5. No panics in the SDK, enforced with `clippy::unwrap_used` and `expect_used`, which ADR 0038 and `STYLE.md` already push toward.

##### P5. Network interception and events are hard to find: about 12 reports

**Reports:**
- chromiumoxide:
  - [#83](https://github.com/mattsse/chromiumoxide/issues/83), "Request interception" (7 comments; open 2022–2025);
  - [#157](https://github.com/mattsse/chromiumoxide/issues/157), "Is there an example of Network Request Interception?" (open);
  - [#256](https://github.com/mattsse/chromiumoxide/issues/256), the `Fetch` versus `Network` domain mix-up;
  - [#28](https://github.com/mattsse/chromiumoxide/issues/28) and [#22](https://github.com/mattsse/chromiumoxide/issues/22), events;
  - [#254](https://github.com/mattsse/chromiumoxide/issues/254), HAR.
- headless_chrome [#280](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/280): interception not working (open, 12 comments).
- fantoccini [#222](https://github.com/jonhoo/fantoccini/issues/222): not supported (4 reactions).
- thirtyfour:
  - [#55](https://github.com/stevepryde/thirtyfour/issues/55), listening to CDP events (19 comments; open 2021 to May 2026);
  - [#212](https://github.com/stevepryde/thirtyfour/issues/212), capture traffic (8 comments);
  - [#87](https://github.com/stevepryde/thirtyfour/issues/87), capture `console.log` (5 reactions).

**Lessons for Goodfetch:**
1. A typed `route(pattern, handler)` and `on_request` / `on_response` / `on_console` as typed streams.
2. Response-body access with no protocol domain knowledge.
3. An interception guard whose removal does not depend on an ambient runtime.
4. A raw escape hatch kept clearly secondary.

##### P6. Sharing and concurrency ergonomics (`Clone`, `&mut self`, lifetimes, `Send`): about 11 reports

**Reports:**
- chromiumoxide [#51](https://github.com/mattsse/chromiumoxide/issues/51) (Copy/Arc), and [#50](https://github.com/mattsse/chromiumoxide/issues/50).
- headless_chrome:
  - `Element<'a>` (forum [131088](https://users.rust-lang.org/t/how-to-deal-with-borrowed-value-does-not-live-long-enough-for-arc-in-for-loops/131088), 2025);
  - [#376](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/376), parallel tabs run sequentially (7 comments);
  - [#264](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/264).
- fantoccini:
  - [#86](https://github.com/jonhoo/fantoccini/issues/86) and [#153](https://github.com/jonhoo/fantoccini/issues/153), `&mut self`;
  - [#69](https://github.com/jonhoo/fantoccini/issues/69), no parallel execution (10 comments);
  - [#68](https://github.com/jonhoo/fantoccini/issues/68), pooling causes BrokenPipe.
- thirtyfour lifetime design (forum [49499](https://users.rust-lang.org/t/async-lifetimes-and-references-owned-by-the-current-function/49499)) and the HRTB closure ([72402](https://users.rust-lang.org/t/async-closure-with-borrowed-arg/72402)).
- octaltree [#38](https://github.com/octaltree/playwright-rust/issues/38): not `Send`.

**Lessons for Goodfetch:**
1. All handles (`Browser`, `Context`, `Page`, `Locator`, `ElementHandle`) are cheap `Clone`, `Send + Sync + 'static`.
2. Methods take `&self`, with no lifetime parameters in public types.
3. Locator construction is sync and infallible, as playwright-rs learned in 0.15.0.

##### P7. Launch and environment friction: about 20 reports

**Reports:**
- headless_chrome:
  - [#300](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/300), no free port in 8000–9000 (12 reactions, open);
  - [#267](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/267), the same in Alpine;
  - [#226](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/226), `Browser::default()` panics instead of fetching (5 reactions);
  - [#500](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/500), the devtools default flip (11 reactions);
  - [#489](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/489), the `offline` feature (19 reactions);
  - [#535](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/535) (7 reactions);
  - [#344](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/344), no WebSocket URL before timing out;
  - [#303](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/303), auto-install undocumented;
  - [#427](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/427) and [#488](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/488), the fetch feature broken.
- chromiumoxide:
  - stderr port parsing that needs an English locale ([#113](https://github.com/mattsse/chromiumoxide/issues/113), [#216](https://github.com/mattsse/chromiumoxide/issues/216));
  - [#295](https://github.com/mattsse/chromiumoxide/issues/295), `LaunchExit` with empty stderr;
  - [#219](https://github.com/mattsse/chromiumoxide/issues/219), Docker;
  - [#186](https://github.com/mattsse/chromiumoxide/issues/186), unknown flag;
  - [#238](https://github.com/mattsse/chromiumoxide/issues/238), linux aarch64;
  - [#294](https://github.com/mattsse/chromiumoxide/issues/294), a request for `--remote-debugging-pipe`.
- fantoccini [#128](https://github.com/jonhoo/fantoccini/issues/128): DevToolsActivePort.
- octaltree:
  - [#27](https://github.com/octaltree/playwright-rust/issues/27), Apple silicon (5 reactions, open);
  - [#42](https://github.com/octaltree/playwright-rust/issues/42), browsers not downloading on M1.
- playwright-rs:
  - [#117](https://github.com/padamson/playwright-rust/issues/117), "Can't build the crate";
  - [#93](https://github.com/padamson/playwright-rust/issues/93), a request for bun;
  - the 130 MB build-time driver download.

**Lessons for Goodfetch:**
1. Managed browser install pinned to the SDK version, as in playwright-rs and thirtyfour `managed`.
2. A pipe transport, or `--remote-debugging-port=0` with `DevToolsActivePort` file reading, instead of stderr parsing or port scans.
3. Never flip a default in a patch release.
4. No network access at build time.
5. Launch errors that carry the browser path, arguments and stderr tail.

##### P8. Protocol drift breaking users on Chrome updates: about 7 reports

**Reports:**
- chromiumoxide:
  - [#243](https://github.com/mattsse/chromiumoxide/issues/243) (5 reactions; the fix took 13 months);
  - [#167](https://github.com/mattsse/chromiumoxide/issues/167) (18 comments);
  - [#266](https://github.com/mattsse/chromiumoxide/issues/266) (5 reactions, open);
  - #335 (open; prior note).
- headless_chrome [#367](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/367): Chrome 110.
- chromey [#5](https://github.com/spider-rs/chromey/issues/5): missing enum variant; the advice was to update Chromium.

**Lesson for Goodfetch.** This is an engine concern (prior note: lenient parsing, CI on stable and beta). For the SDK: pin and report the browser version; return a clear "incompatible browser" error rather than a hang.

##### P9. Maintenance anxiety and release gaps: about 10 reports

**Reports:**
- headless_chrome:
  - [#215](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/215), "Has the library given up updating?" (12 comments);
  - [#323](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/323) (8 reactions, 23 comments);
  - [#296](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/296) (12 reactions);
  - [#272](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/272) and [#455](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/455).
- thirtyfour [#215](https://github.com/stevepryde/thirtyfour/issues/215) (8 reactions).
- octaltree [#30](https://github.com/octaltree/playwright-rust/issues/30).
- chromiumoxide [#220](https://github.com/mattsse/chromiumoxide/issues/220), "0.5.8 release?"; no merges since April 2026 (prior note).
- microsoft/playwright [#34216](https://github.com/microsoft/playwright/issues/34216): Rust crates "not working properly".

**Lesson for Goodfetch.** Release cadence, version pinning and a published compatibility matrix are part of the API promise.

##### P10. Element-query performance (round trips): 2 reports plus source

- headless_chrome [#460](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/460), "`find_elements()` performance is 150x worse than JS" (4 reactions, open). The profiler pointed at event de/serialization.
- headless_chrome [#281](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/281) (7 comments).
- chromiumoxide `find_element` costs four round trips (source).

**Lesson for Goodfetch.** Resolve locators engine-side in one evaluate. Batch "query plus extract" operations, such as `all_text_contents` and `evaluate_all`, so extraction from N elements is not N×4 round trips.

##### P11. Sync versus async demand: 3 signals

- fantoccini [#43](https://github.com/jonhoo/fantoccini/issues/43), "Add synchronous API wrapper" (9 reactions, the most-reacted fantoccini issue). It links a Reddit comment arguing for a sync interface (**unverified**, Reddit unreachable).
- headless_chrome [#181](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/181), "Future-based API" (6 reactions, open).
- rustwright chose a sync Rust API on purpose, with explicit `CancelToken` twins.

**Lesson for Goodfetch.** Async-first is the Rust norm for Tokio users. A thin blocking facade, like reqwest's `blocking`, would serve scripts. Avoid doubling every method for cancellation; dropping the future should cancel.

##### P12. Iframes: 4 reports (detail in the prior note)

chromiumoxide [#163](https://github.com/mattsse/chromiumoxide/issues/163) (28 comments), [#228](https://github.com/mattsse/chromiumoxide/issues/228) (open), [#255](https://github.com/mattsse/chromiumoxide/issues/255) (xpath inside an iframe), [#280](https://github.com/mattsse/chromiumoxide/issues/280) (cross-origin iframe, open).

**Lesson for Goodfetch.** A `frame_locator` that transparently crosses OOPIFs.

---

#### Unverified or partly verified items

- **All Reddit content.** reddit.com is blocked to both WebSearch and `curl`. One search summary paraphrased an r/rust comment that a team "uses [thirtyfour] with Selenium Grid for scraping and it has worked as intended"; it is not checked. The Reddit comment linked from fantoccini #43, arguing for a sync API, was not read. The 2020 r/rust thread where stevepryde and jonhoo discussed merging is referenced in fantoccini #145 but was not read.
- **The dev.to 2024 quote** "a better experience than I had with go and chromedp" came through a summarising fetch; the exact wording was not seen raw.
- **The headless_chrome #500 version linkage** (introduced in 1.0.12, fixed in 1.0.13) is inferred from issue and release dates.
- **The chromiumoxide `find_element` miss message.** That a miss surfaces as "Could not find node with given id" is inferred from headless_chrome's mapping of the same CDP call; not run.
- **The chromiumoxide `find_xpath` `[0]` panic** is real in source only if Chrome returns an empty success for `getSearchResults(0, 0)`. Chrome may return an error first.
- **Whether `TargetConfig::default()`'s 30,000 s timeout is ever used.**
- **The cause of chromiumoxide #292** ("find_element waits for visibility") is the reporter's guess, not supported by the 0.9.1 source.
- **Rustwright's performance and fingerprint claims** ("2.55× faster", "70% less memory") are vendor claims, not checked.
- **The thirtyfour default wait.** The source says 20 s; a 2022 maintainer comment and a 2020 blog post say 30 s. When it changed was not traced.
- **None of the code samples were compiled.** The method names and signatures were checked against the published sources at the cited versions.


### 2.5 Cypress (16.1.1): API ideas only

#### Walkthrough

- **Command queue.** `cy.*` calls enqueue commands and return a chainer immediately. Execution starts after the test function returns. "Cypress commands are not Promises and cannot be awaited" ([intro](https://docs.cypress.io/app/core-concepts/introduction-to-cypress)). Values are *yielded* to `.then()` callbacks, never returned. `const el = cy.get('.x')` "will not work". Synchronous code between commands runs *before* them.
- **Retry-ability** ([docs](https://docs.cypress.io/app/core-concepts/retry-ability)):
  - Queries (`get`, `find`, `contains`, …) and assertions link up: "Queries link up, retrying the entire chain together", up to `defaultCommandTimeout` (4 s), overridable per command with `{ timeout }`.
  - Actions (`click`, `type`) are not retried. They wait for actionability, then run once.
  - Guidance: put actions at chain ends, split chains after actions, prefer `.should(cb)` over `.then()` (which is not retried and can hold stale elements).
  - Cypress 12.0.0 (2022-12-05/06) made queries re-query detached elements ([#7306](https://github.com/cypress-io/cypress/issues/7306), 541 reactions, maintainer comment 2022-12-06). It also introduced `Cypress.Commands.addQuery`, made `cy.origin`/`cy.session` GA and turned on test isolation by default ([release](https://www.cypress.io/releases/12-0-0), [blog](https://www.cypress.io/blog/announcing-cypress-12)).
- **`cy.intercept`:** route matching by method/URL/glob, static stubs or `req.reply` / `req.continue` handlers, aliases with `cy.wait('@alias')` as a synchronisation primitive.
  - Gaps: request-body matching ([#8560](https://github.com/cypress-io/cypress/issues/8560), 72 reactions, open), WebSockets ([#2492](https://github.com/cypress-io/cypress/issues/2492), 144, open), not allowed inside `cy.origin` ([#20720](https://github.com/cypress-io/cypress/issues/20720), 77, open).
  - Overriding semantics changed in 6.x and confused users ([#9302](https://github.com/cypress-io/cypress/issues/9302), 184).
- **Architecture trade-offs** ([docs](https://docs.cypress.io/app/references/trade-offs)). Test code runs inside the browser, JavaScript only, one browser at a time, one superdomain per test (`cy.origin` to cross). It is "not a general purpose" automation tool. Temporary trade-offs: no `cy.hover`, no native events, limited iframes.
- **`cy.origin` limits** ([docs](https://docs.cypress.io/api/commands/origin)):
  - the callback "is not a closure"; data passes through serializable `args`
  - no `require`/`import` without an experimental flag
  - no nested `cy.origin`
  - `cy.intercept` and `cy.session` throw inside it
  - custom commands are not shared
  - cannot reach other tabs or iframes

#### Sample (my own, matching current docs)

```js
describe('checkout', () => {
  it('places an order', () => {
    cy.intercept('POST', '/api/orders', { statusCode: 201, body: { id: 42 } }).as('createOrder')
    cy.visit('/cart')
    cy.get('[data-cy=line-item]').should('have.length', 2)   // query + assertion retried (≤4 s)
    cy.contains('button', 'Checkout').click()                 // action: actionability wait, then once
    cy.wait('@createOrder').its('response.statusCode').should('eq', 201)
    cy.get('h1').invoke('text').then((title) => {             // value only inside a callback
      expect(title).to.match(/Thank you/)
    })
    cy.origin('https://pay.example.com', { args: { amount: 10 } }, ({ amount }) => {
      cy.get('#amount').should('have.value', String(amount))  // no closure over outer scope
    })
  })
})
```

#### Praised ideas

- Retrying assertions: "it'll continuously retry until the timeout" ([HN 25387416](https://news.ycombinator.com/item?id=25387416), 2020-12-11).
- Network stubbing: "I really really like the cy.intercept()" ([HN 33543799](https://news.ycombinator.com/item?id=33543799), 2022-11-10).
- Time-travel debugging and the interactive runner, acknowledged by WDIO's v10 post as setting the standard ([WDIO blog 2026-10-05](https://webdriver.io/blog/2026/10/05/webdriverio-v10-release)).
- Aliases with `cy.wait('@alias')` make network completion a declarative wait point.
- Fast feedback on first adoption: "started catching bugs that justified the time investment" ([HN 25381679](https://news.ycombinator.com/item?id=25381679)).

#### Pain points, ranked

1. **Not-a-promise / no `async`/`await` / closure nesting.**
   - Evidence: [#1417](https://github.com/cypress-io/cypress/issues/1417) "Await-ing Cypress Chains" (492 reactions, 123 comments, open since 2018-03-05). SO [51793521](https://stackoverflow.com/q/51793521) (86), [49980311](https://stackoverflow.com/q/49980311) (53), [55847619](https://stackoverflow.com/q/55847619) (50), [56790303](https://stackoverflow.com/q/56790303) (33), [59184571](https://stackoverflow.com/q/59184571) (30). HN [25381646](https://news.ycombinator.com/item?id=25381646) ("command chaining and assertion queuing … very unintuitive"), [25381822](https://news.ycombinator.com/item?id=25381822), [25382099](https://news.ycombinator.com/item?id=25382099), [33538530](https://news.ycombinator.com/item?id=33538530), [33539054](https://news.ycombinator.com/item?id=33539054), [33545117](https://news.ycombinator.com/item?id=33545117). Page-object pitfall: [HN 25385889](https://news.ycombinator.com/item?id=25385889). Migration posts: [mtlynch, 2022-10-25](https://mtlynch.io/notes/cypress-vs-playwright/) and [BigBinary, 2024-09-18](https://www.bigbinary.com/blog/why-we-switched-from-cypress-to-playwright). About 14 independent reports, 2018–2024.
   - Maintainer rationale (brian-mann, 2018-03-06): "Cypress commands are not 1:1 Promises". `try/catch` equals `.catch()`, "which Cypress does not and will never have". `await` would introduce microtask gaps where the DOM changes, so it "will lead to nondeterminism". Status in 2022 (jennifer-shehane): no active work.
   - **Lesson:** keep Cypress's *retry semantics* (lazy queries plus retrying assertions) but deliver them through native `async`/`await` (Playwright's `expect(locator).toHaveText()` model). Values come back as return values. Retrying lives inside one awaited call, not in a hidden queue.
2. **Single origin / tabs / iframes / multi-user.**
   - Evidence: [#944](https://github.com/cypress-io/cypress/issues/944) multiple superdomains (426 reactions; closed by `cy.origin` in 12.0.0, 2022), [#136](https://github.com/cypress-io/cypress/issues/136) iframe support (1,045 reactions, 477 comments, **open since 2016-05**), [#20718](https://github.com/cypress-io/cypress/issues/20718) (nested origin, 67), [#20720](https://github.com/cypress-io/cypress/issues/20720). HN [33539340](https://news.ycombinator.com/item?id=33539340) and [37846744](https://news.ycombinator.com/item?id=37846744) (multi-user collaboration impossible), [25384297](https://news.ycombinator.com/item?id=25384297). BigBinary (chat between two users, SameSite cookie errors from running inside an iframe). About 9 reports.
   - **Lesson:** out-of-process control with explicit page, context and frame *values* (not a global pointer). Multiple contexts per session is table stakes for multi-user flows.
3. **Native events / hover / keyboard.**
   - Evidence: [#299](https://github.com/cypress-io/cypress/issues/299) tab key (1,700 reactions, 2016 → closed 2025-04), [#10](https://github.com/cypress-io/cypress/issues/10) `cy.hover()` (216, **open since 2015**), [#311](https://github.com/cypress-io/cypress/issues/311) native events (125, open). mtlynch: hover and file upload missing for years.
   - **Lesson:** use trusted input (CDP/BiDi `input.performActions`), not synthetic JS events.
4. **Detached DOM before v12.** [#7306](https://github.com/cypress-io/cypress/issues/7306) (541). Fixed by re-querying. Same lesson as Selenium #2.
5. **Commercial lock-in.**
   - Evidence: v13 blocked third-party dashboards (Sorry Cypress, Currents) ([HN 37842778](https://news.ycombinator.com/item?id=37842778), 2023-10-11). "Cypress version 13 blocked all third-party reporters" (BigBinary). Parallelism needs Cloud ([#2520](https://github.com/cypress-io/cypress/issues/2520), 84 reactions).
   - **Lesson:** open reporter and trace formats, and free parallelism (also matches Goodfetch's no-paid-resources stance).
6. **Logging to stdout.** [#3199](https://github.com/cypress-io/cypress/issues/3199) (89), [#448](https://github.com/cypress-io/cypress/issues/448) (316), mtlynch. **Lesson:** structured logs to the host process, never only to an in-browser UI.
7. **Conditional testing is discouraged and hard.** SO [48915773](https://stackoverflow.com/q/48915773) "test if element does not exist" (416), [56145926](https://stackoverflow.com/q/56145926) (101), [47773525](https://stackoverflow.com/q/47773525) (63). **Lesson:** scraping and automation (Goodfetch's use case) *needs* non-throwing probes (`count()`, `isVisible()`, `waitFor({state:'detached'})`) next to retrying assertions.
8. **Crashes and memory.** "Recover from renderer / browser crashes" ([#349](https://github.com/cypress-io/cypress/issues/349), 107 reactions, closed not-planned 2023). Notion removed Cypress over memory and crashes ([HN 25382762](https://news.ycombinator.com/item?id=25382762)). **Lesson:** the engine detects crashes and surfaces a typed `TargetCrashed` error, and the session can be recreated.

---

### 2.6 WebdriverIO (10.0.2): API ideas only

#### Walkthrough

- **Multi-protocol by design.**
  - WebDriver classic, WebDriver BiDi (default since v9, 2024-08-15; opt out with `wdio:enforceWebDriverClassic`, check with `browser.isBidi`), and Puppeteer/CDP access (`getPuppeteer`).
  - Appium for mobile. Electron, Tauri and Dioxus services for desktop.
  - v10 (2026-10-05) removed the last JSON Wire Protocol code ("Every session is a W3C session"), requires Appium 3 and Node 22.19 ([v10 blog](https://webdriver.io/blog/2026/10/05/webdriverio-v10-release)).
  - The typed BiDi client is generated by their own [`cddl`](https://github.com/webdriverio/cddl) parser/generator. That parallels Selenium's 2026 generators.
- **`$` / `$$` chaining.** `$` returns a lazy chainable element, so `await $('#cart').$('li=Blue shirt').click()` needs one `await`; lazy chains landed via [#4765](https://github.com/webdriverio/webdriverio/issues/4765), closed 2024-05.
  - Selector engines: CSS, XPath, `=text` / `*=partial`, `aria/` accessible name, `role/` (new in v10), shadow piercing (v9 tracks open *and* closed shadow roots).
  - v10: **`$` is strict**. It throws `StrictSelectorError` when the selector matches more than one element. Zero matches still return a lazy element, and auto-waiting is unchanged. Opt out per call with `{ strict: false }` or globally with `strictSelectors: false` ([migration skill](https://github.com/webdriverio/webdriverio/blob/main/.agents/skills/wdio-v10-migration/SKILL.md)). Rationale: "An ambiguous selector is a common source of flaky tests" (v10 blog).
  - `$$` now returns a real (awaitable) array.
- **Auto-wait.** Interaction commands wait for the element to be interactable (`isClickable` conditions) ([docs](https://webdriver.io/docs/autowait)). Implicit timeouts are "not recommended". `expect-webdriverio` matchers retry.
- **Contexts as values (v10).** `browser.url()` returns the page, `browser.newWindow()` returns a tab without switching, and `context.frame()` returns a frame. In BiDi sessions `switchWindow` and `switchFrame` **throw**. Reason given: "forgetting a single switch sent the next command to the wrong place."
- **Network.** `browser.mock(url, filter)` with `respond`, `abort`, `calls` and spies works in all browsers since v9 via BiDi. Response bodies come from BiDi data collectors (v10 `extract()`). Websocket mocking is open ([#5911](https://github.com/webdriverio/webdriverio/issues/5911)).
- **History.** The sync mode (node-fibers) was deprecated in 2021 because fibers broke on Node 16 ([blog 2021-07-28](https://webdriver.io/blog/2021/07/28/sync-api-deprecation)) and removed in v8 (2022-12-01). Suites had to be rewritten to async (e.g. [Wikimedia T324318](https://phabricator.wikimedia.org/T324318)).

#### Sample (v10, TypeScript; my own, matching v10 docs and blog)

```ts
const checkout = await browser.url('https://example.com/checkout')   // returns a BrowsingContext (BiDi)
const orders = await browser.mock('**/api/orders', { method: 'post' })
orders.respond({ id: 42 }, { statusCode: 201 })

await $('aria/Checkout').click()               // auto-waits; strict: throws if >1 match
await $('#cart').$('li=Blue shirt').click()    // lazy chain, single await
const items = await $$('li.item')              // real array in v10
await expect($('h1')).toHaveText('Thank you')  // retrying matcher

const payment = await checkout.frame('#payment')   // frame as a value; no switchFrame
await payment.$('#card').setValue('4242')
expect(orders.calls.length).toBe(1)
```

#### Praised ideas

Owner-reported design motives are in the v10 blog. Independent user praise for WDIO is thin in sources I could reach: HN has few WDIO threads, with the top one a benchmark ([HN 25288541](https://news.ycombinator.com/item?id=25288541), 118 points, 2020). The ideas worth keeping:
- strict single-element selector
- contexts as values
- lazy chain with one `await`
- `aria/` and `role/` selectors
- BiDi-first with a classic fallback and an `isBidi` capability probe
- one API across web, mobile and desktop

#### Pain points, ranked

1. **BiDi-by-default migration regressions (v9)**, on clouds, grids and in features.
   - Evidence: BrowserStack [#13722](https://github.com/webdriverio/webdriverio/issues/13722) (33 comments), Selenium Grid after v8→v9 [#14946](https://github.com/webdriverio/webdriverio/issues/14946) (18), `fetch failed` [#13553](https://github.com/webdriverio/webdriverio/issues/13553) (19), `waitForDisplayed` during navigation [#13558](https://github.com/webdriverio/webdriverio/issues/13558) (20), mock spies without request body [#13765](https://github.com/webdriverio/webdriverio/issues/13765) (19), mock slow with long URLs [#13548](https://github.com/webdriverio/webdriverio/issues/13548) (open), `aria/` slowness [#14662](https://github.com/webdriverio/webdriverio/issues/14662) (20), v9 10–20% slower than v8 ([#14494](https://github.com/webdriverio/webdriverio/issues/14494), open since 2025-05). About 8 reports, 2024–2025.
   - **Lesson:** protocol switching must be invisible and benchmarked. Remote and cloud endpoints may lack BiDi, so negotiate capabilities and degrade explicitly.
2. **Hidden current-context pointer.** Fixed in v10 by contexts as values. Same lesson as Cypress #2.
3. **Forced async rewrite** (fibers removal, v7→v8). **Lesson:** do not build on runtime hacks for a sync facade. Python's sync API in Goodfetch should be a real blocking client over the same JSON-RPC, not coroutine tricks.
4. **Clicking during animations.** [#3106](https://github.com/webdriverio/webdriverio/issues/3106) (48 comments, 2018–2023). Maintainer: BiDi could disable animations, "a separate task". Same lesson as Selenium #3.
5. **Driver downloads.** [#12251](https://github.com/webdriverio/webdriverio/issues/12251) (23 reactions, chromedriver URL change), [#15608](https://github.com/webdriverio/webdriverio/issues/15608) (open 2026-09). Same lesson as Selenium #4.

---

### 2.7 Stealth forks: nodriver / zendriver, Patchright, rebrowser (why users switch)

#### Why users switch (what they claim to fix)

| Leak | Mechanism | Who patches |
|---|---|---|
| `navigator.webdriver` / `--enable-automation` | flag set by drivers and the automation switch | Patchright adds `--disable-blink-features=AutomationControlled`, removes `--enable-automation`; nodriver launches Chrome itself without chromedriver |
| chromedriver artefacts | `cdc_` globals, WebDriver HTTP layer | nodriver: "No more webdriver, no more selenium" ([README](https://github.com/ultrafunkamsterdam/nodriver)); successor of undetected-chromedriver |
| `Runtime.enable` | Puppeteer/Playwright enable the Runtime domain to learn execution-context ids, which makes `Runtime.consoleAPICalled` serialisation observable from page JS (DataDome write-up 2024-06-13, cited by [rebrowser](https://rebrowser.net/blog/how-to-fix-runtime-enable-cdp-detection-of-puppeteer-playwright-and-other-automation-libraries)) | rebrowser: `addBinding` (default), `alwaysIsolated` or `enableDisable` modes via `REBROWSER_PATCHES_RUNTIME_FIX_MODE`; Patchright: never call `Runtime.enable`, evaluate in isolated `ExecutionContext`s |
| `Console.enable` | console domain observable | Patchright disables console APIs entirely |
| `sourceURL=pptr:` / utility-world name | identifiers visible to page scripts | rebrowser renames to `app.js` / `util` |
| Closed shadow roots | not reachable | Patchright pierces closed shadow roots, including XPath |

API kept by each:
- **Patchright** is a drop-in Playwright: change the import, Chromium only. It adds `isolated_context` (default `True`) to `evaluate`, `evaluate_handle` and `evaluate_all` ([patchright-python README](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright-python)).
- **rebrowser** is a patch tool plus drop-in packages. Use the package.json alias `"puppeteer": "npm:rebrowser-puppeteer@^24.8.1"`.
- **nodriver/zendriver** have their own async-only CDP API: `start()`, `browser.get()`, `tab.find(text)` (best text match, retries until timeout, searches iframes), `tab.select(css)`, `tab.xpath()`, `tab.add_handler(cdp.Event, cb)`, raw `tab.send(cdp.domain.command())` with generated CDP types. Expert mode disables web security and forces shadow roots open.

#### Samples (my own)

```python
# nodriver 0.50.x (zendriver is the same shape, with `await browser.stop()` and asyncio.run)
import nodriver as uc

async def main():
    browser = await uc.start()                       # local Chrome, fresh temp profile, no chromedriver
    tab = await browser.get("https://example.com")
    await tab.send(uc.cdp.network.enable())
    tab.add_handler(uc.cdp.network.ResponseReceived, lambda ev: print(ev.response.url))
    btn = await tab.find("accept all", best_match=True, timeout=10)  # text search, retried
    if btn:                                          # returns None on timeout (see pain #2)
        await btn.click()
    h1 = await tab.select("h1")
    print(h1.text)
    browser.stop()

if __name__ == "__main__":
    uc.loop().run_until_complete(main())             # README: "asyncio.run never worked (for me)"
```

```python
# Patchright (Python) — drop-in Playwright, "best practice" setup per README
from patchright.sync_api import sync_playwright

with sync_playwright() as p:
    ctx = p.chromium.launch_persistent_context(
        user_data_dir="/tmp/pr-profile", channel="chrome", headless=False, no_viewport=True)
    page = ctx.new_page()
    page.goto("https://example.com")
    page.get_by_role("button", name="Accept").click()
    title = page.evaluate("document.title")                               # isolated world by default
    state = page.evaluate("window.__APP_STATE__", isolated_context=False)  # main world on request
    ctx.close()
```

```sh
# rebrowser: patch an installed library in place (must be re-run after npm install)
npx rebrowser-patches@latest patch --packageName puppeteer-core
REBROWSER_PATCHES_RUNTIME_FIX_MODE=alwaysIsolated node app.js
```

#### Likes

- nodriver's "best practice defaults" and one-line start. Text-first `find()` that "will not naively return the first matching element", with repr-as-HTML elements ([README](https://github.com/ultrafunkamsterdam/nodriver)).
- Independent benchmark ([ianlpaterson.com](https://ianlpaterson.com/blog/anti-detect-browser-benchmark-patchright-nodriver-curl-cffi/), 2026-05-13, updated 2026-08-15; 31 targets, 3 sweeps, one residential IP, headed). Blocked / gated / OK:

  | Tool | Blocked | Gated | OK |
  |---|---|---|---|
  | nodriver | 0 | 3 | 28 |
  | Patchright | 3 | 3 | 25 |
  | Vanilla Playwright | 5 | 2 | 24 |
  | rebrowser-playwright | 5 | 2 | 24 |

  rebrowser-playwright's block set matches vanilla Playwright's exactly. Caveats from the author: one IP, one night, vendor churn.
- Patchright keeps the whole Playwright API and auto-tests against Playwright's suite after every release.

#### Pain points, ranked

1. **Maintenance lag and bus factor.**
   - Patchright:
     - maintainer "currently a bit inactive" around exams, recommending users stay on 1.56.0 after 1.57.0 issues ([#161](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/161), 2026-01-05, 12 reactions)
     - automated "Release Failed: v1.64.0 / v1.64.1 – Tests Failed" with label `upstream-break` ([#249](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/249), [#251](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/251), 2026-10-07 and 10-14)
     - Patchright stands at 1.63.0 while Playwright is at 1.64.1
   - rebrowser: last release 2025-05-09. Latest tested Puppeteer is 24.8.1 against 25.13.0 today, and Playwright 1.52.0 against 1.64.1. The benchmark author says no rebrowser-playwright code commits since Sept 2024 (**unverified**; npm shows 1.52.0 published 2025-05-09).
   - nodriver: contributions "heavily restricted", with critical bug-fix PRs unaddressed "for many months". That is the stated reason for the zendriver fork ([zendriver README](https://github.com/cdpdriver/zendriver)). nodriver's tracker has only ~25 issues ever.
   - **Lesson:** patching an upstream driver is a treadmill. Goodfetch owning its engine (and its CDP/BiDi usage) avoids it, but must then own the footprint itself.
2. **Generated CDP types break on browser updates.**
   - Evidence: Chrome 146 renamed `privateNetworkRequestPolicy`, which raised a `KeyError` on nearly every request ([zendriver #249](https://github.com/cdpdriver/zendriver/issues/249), 2026-03-30). `sameParty` became optional and `cookies.get_all()` crashed ([zendriver #244](https://github.com/cdpdriver/zendriver/issues/244), 7 reactions, 2026-03-11). Same cookie breakage in nodriver ([#33](https://github.com/ultrafunkamsterdam/nodriver/issues/33), open). Launch failures after Chrome 135 and 138 ([#105](https://github.com/cdpdriver/zendriver/issues/105), [#142](https://github.com/cdpdriver/zendriver/issues/142)). About 5 reports in 2 repos.
   - **Lesson:** match Selenium ADR 17786. Tolerate unknown inbound fields, treat deprecated required fields leniently, pin the protocol version per supported browser range, and test against Chrome Beta/Canary in CI.
3. **Error contract mismatches.**
   - nodriver `find()`'s docstring says "raise timeout exception", but the code returns `None` on timeout (source `nodriver/core/tab.py`, `find`/`select`). Users notice: [SO 79830778](https://stackoverflow.com/q/79830778) (2025-11-26).
   - zendriver users ask how to catch `ProtocolException` ([#63](https://github.com/cdpdriver/zendriver/issues/63)) and request Selenium-style `ExpectedConditions` ([#236](https://github.com/cdpdriver/zendriver/issues/236)).
   - **Lesson:** one documented, typed timeout error across SDKs, with an explicit non-throwing probe alternative.
4. **Stealth costs features.**
   - Patchright's known-bugs list ([#30](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/30)):
     - 34 failing Playwright tests from disabling the Console domain (`console`, `pageerror`, `weberror` events, some tracing)
     - 6 WebSocket-routing failures
     - init scripts injected through routing, so they don't run on `about:blank` or data URLs and have timing differences
     - `page.pause()` / Inspector issues ([#132](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/132))
   - rebrowser: `page.pause()` broken with the fix on. Isolated mode loses main-world globals, and workers can't get isolated worlds ([#13](https://github.com/rebrowser/rebrowser-patches/issues/13)).
   - "console functionality will not work in Patchright."
   - **Lesson:** make the main-world/isolated-world choice an explicit, typed option. Expose a "stealth profile" that lists which features it disables, so users aren't surprised when `console` events are silent.
5. **Detection is an arms race.** Patchright detected by Kasada ([#96](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/96), closed not-planned), DataDome vs zendriver ([#20](https://github.com/cdpdriver/zendriver/issues/20), open), fingerprint-spoofing requests ([zendriver #241](https://github.com/cdpdriver/zendriver/issues/241), [#108](https://github.com/cdpdriver/zendriver/issues/108)). rebrowser: "this fix alone won't make your browser bulletproof."
6. **Docs, types, ergonomics.**
   - nodriver README: "since asyncio.run never worked (for me)". zendriver had to remove dynamic `__getattr__` because mypy missed non-existent methods ([#148](https://github.com/cdpdriver/zendriver/issues/148), [#170](https://github.com/cdpdriver/zendriver/issues/170) "Road to v1.0.0").
   - rebrowser patches are wiped by `npm install` and need `patch.exe` on Windows.
   - nodriver and zendriver are **AGPL-3.0** (a constraint for embedding). Patchright is Apache-2.0. rebrowser-patches has no SPDX license detected by GitHub.
   - The Patchright README is dominated by proxy-vendor sponsorships (observation, not a user report).

---

#### Lessons from §2.3 and §2.5–2.7

1. **Lazy locators, not eager handles.** This removes the stale-element class of failures (Selenium #2, Cypress #7306). Pinned handles are explicit, and their stale errors explain the detach.
2. **Retrying assertions through native `await`.** Take Cypress's retry semantics without its queue. No promise manager, no fibers, no hidden scheduler (Selenium #2969, WDIO v8, Cypress #1417).
3. **Per-action actionability guards.** Attached, visible, stable (animation), enabled, receives-the-hit. The failure message names the interceptor (Selenium #3, WDIO #3106). No global implicit wait.
4. **Strict single-target actions by default.** Ambiguous matches throw, with a per-call opt-out (WDIO v10 `$`). Non-throwing probes exist for scraping (Cypress conditional-testing pain).
5. **Pages, contexts and frames as values.** No `switchTo` pointer (WDIO v10, Cypress origin/tab/iframe pain). Multiple contexts per session for multi-user flows.
6. **Protocol-neutral public API.** BiDi and CDP stay internal to the Rust engine (Selenium ADR 17670). The engine's protocol layer is generated from pinned schemas, with tolerant inbound parsing (ADR 17786; zendriver Chrome 146 breakages).
7. **Specify network-interception semantics once** (Selenium ADR 17685): LIFO, fallthrough, first-settle-wins, opt-in body collection, per-context scope, exceptions fail the event. Enforce them identically in TS, Python and Rust through the engine, which avoids Selenium's per-binding drift.
8. **The engine owns provisioning.** Browser plus driver, pinned and cached, with actionable setup errors (Selenium Manager). Telemetry off unless opted in (#14643).
9. **Lifecycle safety.** Scoped sessions in each SDK, orphan reaping, unique temp profiles (Selenium/WDIO "user data directory in use").
10. **Stealth as an explicit, documented profile.** Avoid `Runtime.enable`/`Console.enable` leaks, offer isolated-vs-main world as a typed option, and list the features each profile disables (Patchright #30). Don't depend on patching someone else's driver.
11. **Benchmark protocol switches before making them default** (WDIO v9 regressions, #14494). Negotiate BiDi availability against remote endpoints.
12. **Open trace and report formats, free parallelism** (Cypress 13 lock-in). WDIO v10 also chose a portable `trace.zip`.

#### Unverified or missing (§2.3, §2.5–2.7)

- **Reddit:** inaccessible (HTTP 403 and no search hits). r/QualityAssurance, r/softwaretesting, r/webscraping, r/Python and r/node evidence is absent.
- **Safari/WebKit WebDriver BiDi support:** not checked.
- **wpt.fyi BiDi pass counts:** the page is client-rendered and was not extracted. The Puppeteer dashboard numbers stand in.
- **Firefox 141 release date:** inferred from the 2025-07-22 newsletter, not from Mozilla release notes.
- **Cypress 12 "queries" wording:** from the maintainer comment on #7306 and the retry-ability docs. The 12.0.0 changelog text itself was not read (page too large).
- **WebdriverIO auto-wait default `waitforTimeout` / `waitforInterval` values:** not stated on the page read. The v10 blog says the default wait interval was made faster; the exact values are unverified.
- **rebrowser-playwright "no code commits since Sept 2024":** a third-party claim that conflicts with the npm 1.52.0 publish date (2025-05-09).
- **Vibium's use of BiDi:** inferred from an HN title only.
- **The `Runtime.enable` detection mechanism:** taken from rebrowser's article. The precise page-side trick (a serialised getter on a console argument) is **unverified** here.

#### Key sources (§2.3, §2.5–2.7)

- Selenium docs:
  - [WebDriver BiDi](https://www.selenium.dev/documentation/webdriver/bidi/)
  - [CDP](https://www.selenium.dev/documentation/webdriver/bidi/cdp/)
  - [Waits](https://www.selenium.dev/documentation/webdriver/waits/)
  - [Errors](https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/)
  - [Selenium Manager](https://www.selenium.dev/documentation/selenium_manager/)
  - [Logging (BiDi)](https://www.selenium.dev/documentation/webdriver/bidi/logging/)
  - [Remove Firefox CDP blog](https://www.selenium.dev/blog/2025/remove-cdp-firefox/)
  - [4.51.0 release](https://github.com/SeleniumHQ/selenium/releases/tag/selenium-4.51.0)
- Selenium decision records: [17670](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17670-bidi-implementation-boundaries.md), [17685](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17685-network-handler-behavior.md), [17786](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17786-bidi-low-level-behavioral-contract.md), [17817](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17817-driver-extension-install.md).
- BiDi status:
  - [Spec (TR, 2026-10-08)](https://www.w3.org/TR/webdriver-bidi/), [Editor's Draft](https://w3c.github.io/webdriver-bidi/)
  - [Is Puppeteer WebDriver BiDi ready](https://puppeteer.github.io/ispuppeteerwebdriverbidiready/)
  - [Chrome blog: BiDi production-ready](https://developer.chrome.com/blog/firefox-support-in-puppeteer-with-webdriver-bidi)
  - [Firefox newsletter 141](https://fxdx.dev/firefox-webdriver-newsletter-141/), [CDP retirement in Firefox](https://fxdx.dev/cdp-retirement-in-firefox/)
- Cypress:
  - [Retry-ability](https://docs.cypress.io/app/core-concepts/retry-ability), [Introduction](https://docs.cypress.io/app/core-concepts/introduction-to-cypress)
  - [cy.origin](https://docs.cypress.io/api/commands/origin), [Trade-offs](https://docs.cypress.io/app/references/trade-offs)
  - [Cypress 12 announcement](https://www.cypress.io/blog/announcing-cypress-12)
  - [#1417](https://github.com/cypress-io/cypress/issues/1417), [#7306](https://github.com/cypress-io/cypress/issues/7306), [#136](https://github.com/cypress-io/cypress/issues/136)
- WebdriverIO:
  - [v10 release blog (source)](https://github.com/webdriverio/webdriverio/blob/main/website/blog/2026-10-05-webdriverio-v10-release.md)
  - [v9 release](https://webdriver.io/blog/2024/08/15/webdriverio-v9-release), [Auto-wait](https://webdriver.io/docs/autowait), [Sync API deprecation](https://webdriver.io/blog/2021/07/28/sync-api-deprecation)
  - [v10 migration skill](https://github.com/webdriverio/webdriverio/blob/main/.agents/skills/wdio-v10-migration/SKILL.md)
- Forks:
  - [nodriver](https://github.com/ultrafunkamsterdam/nodriver), [zendriver](https://github.com/cdpdriver/zendriver)
  - [Patchright](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright) and [known bugs #30](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright/issues/30)
  - [rebrowser-patches](https://github.com/rebrowser/rebrowser-patches), [Runtime.enable article](https://rebrowser.net/blog/how-to-fix-runtime-enable-cdp-detection-of-puppeteer-playwright-and-other-automation-libraries)
  - [Anti-detect benchmark 2026](https://ianlpaterson.com/blog/anti-detect-browser-benchmark-patchright-nodriver-curl-cffi/)
- Hacker News:
  - ["Don't use Selenium, use Playwright" (408 pts, 2022)](https://news.ycombinator.com/item?id=33537772)
  - [Cypress launch thread (316 pts, 2020)](https://news.ycombinator.com/item?id=25381242)
  - [Cypress blocking Sorry Cypress (2023)](https://news.ycombinator.com/item?id=37842778)
  - [Vibium (443 pts, 2025)](https://news.ycombinator.com/item?id=46377597)
- Migration posts:
  - [mtlynch 2022](https://mtlynch.io/notes/cypress-vs-playwright/)
  - [BigBinary 2024](https://www.bigbinary.com/blog/why-we-switched-from-cypress-to-playwright)
  - [pythonforengineers 2022](https://pythonforengineers.com/blog/web-automation-dont-use-selenium-use-playwright/)

## 3. Cross-library comparison

State on 2026-10-14. "From source" facts are linked in §2.

| Dimension | Playwright 1.64 | Puppeteer 25.13 | Selenium 4.51 | Cypress 16 | WebdriverIO 10 | chromiumoxide 0.9.1 | thirtyfour 0.37.8 | playwright-rs 0.20 | nodriver / Patchright |
|---|---|---|---|---|---|---|---|---|---|
| Official languages | JS/TS, Python (sync + async), Java (sync), .NET (async) | JS/TS (community: PuppeteerSharp; pyppeteer unmaintained) | Java, Python, C#, Ruby, JS | JS only, runs in the browser | JS/TS | Rust (Tokio) | Rust (Tokio) | Rust (Tokio) | Python async / Playwright's languages |
| Process model | client → Node driver over stdio pipe (non-JS ports bundle Node) | in-process CDP/BiDi client | client → driver binary over HTTP (+ BiDi WebSocket) | test code inside the browser + Node server | client → driver/BiDi | in-process CDP; user must poll a `Handler` stream | client → chromedriver/geckodriver (auto-managed) | client → Node driver (130 MB) | in-process CDP / Playwright driver |
| Protocols | CDP (Chromium), Juggler (patched Firefox), patched WebKit; BiDi internal and not public | CDP (Chrome), BiDi (Firefox, default since 23.0) | WebDriver classic, BiDi opt-in, CDP "temporary" | own | BiDi default (v9), classic fallback | CDP | classic + CDP + BiDi feature | Playwright protocol | CDP |
| Default handle | lazy `Locator`; `ElementHandle` "Discouraged" | `Locator` recommended (20.6, 2023); `ElementHandle` still common | eager `WebElement` (goes stale) | lazy query chain (re-queries since 12.0) | lazy `$` chain | eager `Element` | `ElementQuery` (lazy query builder) + eager `WebElement` | lazy `Locator` (sync constructor since 0.15) | eager element / Playwright `Locator` |
| Auto-wait in actions | yes: visible, stable, receives events, enabled, editable; back-off 0/20/100/100/500 ms | locators: viewport, visible, enabled, stable box (no hit-target check) | no (declined, #15430) | actionability wait, then run once | yes (`isClickable`) | no | no (server-side typed errors) | yes (driver) | nodriver `find()` retries; Patchright = Playwright |
| Multiple matches | **strict**: throws at once (since 1.14, 2021) | first match | first match | actions need a single element unless `{ multiple: true }` | **strict** since v10 (2026-10) | first | `single()` checks after ≥1 found | strict | first / strict |
| Default timeout | library 30 s per action/navigation; runner: none per action, 30 s per test, 5 s per `expect` | 30 s | implicit 0; page load 300 s; script 30 s | 4 s per command | (not verified) | 30 s, hard-coded for page commands | 20 s / 500 ms poll; HTTP 120 s | 30 s | nodriver `find` 10 s |
| Cancellation | `AbortSignal` (JS only, 1.62); Python: "undefined behavior"; .NET declined | `AbortSignal` on locator actions, `waitForSelector` and other waits | none | none | none | drop the future | drop the future | drop the future | — |
| Pages/frames | values (`Page`, `Frame`, `FrameLocator`, selector-less `frameLocator()` 1.63) | values | hidden "current" pointer (`switch_to`) | single tab; `cy.origin` | values since v10; `switchFrame` throws on BiDi | values; no OOPIF commands | pointer (WebDriver) | values | values |
| Events | emitters + "arm-then-trigger" waiters (four idioms per language); no async iterators | emitters | BiDi handlers (beta) | aliases + `cy.wait('@a')` | `browser.mock` spies | typed `Stream`s per CDP event | typed `Stream`s (BiDi, CDP) | Playwright events | handler callbacks |
| Interception | `route(glob, handler)` chain with `fallback`, `fetch`, HAR, WebSocket; disables HTTP cache | `setRequestInterception(true)` + racing `request` handlers, or cooperative priorities; no response interception (#1191) | BiDi `add_request_handler` (beta) | `cy.intercept` | `browser.mock` | raw `Fetch` events and commands | BiDi `AddIntercept` | Playwright `route` | raw CDP / Playwright |
| Errors | `Error`, `TimeoutError`, `TargetClosedError`; message prefixed with API name, **call log** appended, locator rendered in the caller's language | `TimeoutError`, `ProtocolError`, `TargetCloseError`…; short messages, no log | 27 typed exceptions naming the symptom | — | `StrictSelectorError` etc. | `CdpError` enum; "Request timed out." | one variant per W3C error | `#[non_exhaustive] Error`, selector as text suffix | nodriver returns `None` on timeout |
| Debug artefacts | trace viewer (DOM, network, console, aria + screen snapshots), inspector, codegen in 4 languages, UI mode, `PWDEBUG`, `DEBUG=pw:api` | `NODE_DEBUG=puppeteer:*`, DevTools Recorder → `@puppeteer/replay` | logs | time-travel runner | `trace.zip` (v10) | `tracing` logs | — | Playwright trace | — |
| Scoped close | `await using` (1.59); Python `with`; Java `AutoCloseable`; .NET `IAsyncDisposable` | `await using` (exposed 25.4.0) | `quit()`; Python `with` | runner-owned | runner-owned | `close()` + `wait()`; `Drop` only warns | `quit()`; `Drop` blocks a side runtime; `run_browser_test` | `shutdown()`; `Drop` blocks | — |
| Versioning | no major since 1.0 (2020); minors remove long-deprecated API; browsers pinned per release; `connect` needs same major.minor | semver majors (22–25 since 2024); one Chrome per release | semver; CDP per Chrome N, N−1, N−2 | semver | semver; v9 BiDi default caused regressions | 0.x, each minor breaking | 0.x; 0.37 big migration | 0.x, every ~2 weeks | lag upstream |

## 4. Common patterns and divergences

### 4.1 Where the field has converged

1. **Lazy, re-resolving locators replace eager handles.** Playwright introduced `Locator` in 1.14 (2021-08-13) and now marks every `ElementHandle` and selector-taking `page.*` action "Discouraged"; Puppeteer added locators in 20.6.0 (2023-06-09); Cypress 12.0 (2022-12) re-queries detached elements; WebdriverIO chains `$` lazily; thirtyfour pushes `query()` and `#[derive(Component)]` resolvers. Selenium is the holdout, and stale references are its second-ranked pain (§2.3). Nobody removed the eager handle: it stays as an escape hatch.
2. **Ambiguity is an error.** Playwright made locators strict in 2021; Cypress actions require a single element unless `{ multiple: true }`; WebdriverIO made `$` strict in v10 (2026-10-05) because "An ambiguous selector is a common source of flaky tests, and agents write a lot of them." Puppeteer and the Rust crates still take the first match. Playwright fails a strict-mode violation at once instead of waiting for it to resolve; the request to wait is open ([#36045](https://github.com/microsoft/playwright/issues/36045)).
3. **Retrying assertions, delivered through plain `await`.** Cypress proved retry-ability; Playwright's `expect(locator).toHaveText()` delivers it through native async; the hidden-queue models (Cypress, WebDriverJS's promise manager, WebdriverIO's fibers) lost (Selenium [#2969](https://github.com/SeleniumHQ/selenium/issues/2969), WDIO v8, Cypress [#1417](https://github.com/cypress-io/cypress/issues/1417) still open).
4. **Pages, contexts and frames are values, not a switch pointer.** Playwright and Puppeteer always; WebdriverIO v10 now throws on `switchFrame` in BiDi sessions because "forgetting a single switch sent the next command to the wrong place."
5. **The protocol is an implementation detail.** Playwright: "We'd rather not put that choice on users" ([#37277](https://github.com/microsoft/playwright/issues/37277)); Selenium ADR 17670: "BiDi is an implementation mechanism, not a public API", to avoid "the same trap CDP created". Both still offer a raw escape hatch (`CDPSession`, composition), and the users who left Playwright for raw CDP (browser-use, 2025-08) did so because the hatch was too narrow (§6 P12).
6. **Scoped resource management.** `await using` arrived in Playwright 1.59 (2026-04-01) and was exposed in Puppeteer 25.4.0 (2026-07-27); Python context managers, Java `AutoCloseable`, .NET `IAsyncDisposable`. In Rust the absence of async `Drop` makes every crate compromise (§2.4, §6 P3).
7. **Network handler semantics are being written down.** Playwright's `route` chain (`fallback()` passes to the previous handler), Selenium ADR 17685 (LIFO "matches Playwright", first to settle wins, unsettled falls through, bodies on opt-in, per-window scope), Puppeteer's cooperative mode with priorities as a retrofit over its racing handlers.
8. **Generated protocol layers that tolerate drift.** Selenium's 2026 BiDi layer is generated from CDDL with a behavioural contract (typed objects, reject bad outbound values, tolerate unknown inbound fields, ADR 17786); WebdriverIO has its own CDDL generator; strict generated CDP types broke chromiumoxide, headless_chrome, nodriver and zendriver on Chrome updates (§6 P11).

### 4.2 Where they still diverge

| Question | Answers in the field |
|---|---|
| Where does waiting live? | Inside every action (Playwright, Puppeteer locators, WDIO, playwright-rs, rustwright) vs explicit waits only (Selenium, chromiumoxide, headless_chrome, fantoccini, thirtyfour). Selenium declined auto-wait: "Selenium automates browsers, that's it." |
| What counts as actionable? | Playwright: visible, stable, receives events (hit target), enabled, editable. Puppeteer: in viewport, visible, enabled, stable bounding box over two frames, **no hit-target check**. Cypress: actionability then a single attempt. Goodfetch's guard (ADR 0019) adds uniqueness and replays a cancelled gesture. |
| Default timeouts | 30 s (Playwright library, Puppeteer, chromiumoxide, fantoccini waits, playwright-rs); 20 s (thirtyfour, headless_chrome waits); 4 s (Cypress); 5 s (Playwright `expect`); no per-action limit (Playwright test runner); 0 (Selenium implicit). Playwright library and runner disagree on the same call. |
| Polling intervals | Playwright back-off 0/20/100/100/500 ms; chromey 0/20/50/100/100/500 ms; headless_chrome 100 ms; fantoccini 250 ms; thirtyfour 500 ms. |
| Where do callbacks run? | Playwright event predicates and `route` handlers run in the **client** on events streamed over the protocol, so they can close over host variables; `evaluate(fn, arg)` runs in the page with an explicit serialisable argument. Puppeteer's `locator.filter(fn)` and `$$eval` run in the page, so closures silently break ([#11893](https://github.com/puppeteer/puppeteer/issues/11893); SO "pass variable into evaluate", 293 votes). Cypress `cy.origin` callbacks are "not a closure". |
| Cancellation | JS `AbortSignal` in Playwright 1.62 (`langs: js` only) and Puppeteer; Python "undefined behavior"; .NET `CancellationToken` declined in 2022 as "a kinda fake cancel"; Rust: drop the future (crates differ on whether the browser-side work stops); rustwright doubles every method with `*_with_cancel`. |
| Error model | Playwright: three classes, rich text (API name prefix, call log, locator in the caller's language), users told to string-match; Selenium and thirtyfour: one type per W3C error; Rust crates: enums with thin messages; nodriver: `None`. |
| Versioning | Playwright never bumps the major and removes long-deprecated API in minors; Puppeteer and Selenium use semver majors; Rust crates are 0.x with breaking minors. |
| Sync vs async | Playwright Python ships both (sync via greenlets, refused inside a running loop); Java sync-only; .NET async-only (sync declined); Rust crates async except headless_chrome and rustwright (sync with own runtime); fantoccini's top request is a sync wrapper. |

### 4.3 How one API is ported across languages

**Playwright uses two interface definitions, not one.**
1. **The wire protocol**: `packages/protocol/spec/*.yml`, 19 YAML files and 5,324 lines at v1.64.1, split from the single `protocol.yml` on 2026-05-05 ([#40645](https://github.com/microsoft/playwright/pull/40645)). Each interface lists commands with typed `parameters`, `returns`, events and `flags` (`slowMo`, `snapshot`, `pause`, `input`, `isAutoWaiting`, counted 46/97/94/35 uses), plus `title` and `subtitle` templates used by traces and reports. `utils/generate_channels.js` emits TypeScript channel types, runtime validators used on both sides (params on the server, results and events on the client) and per-method metadata; `utils/generate_dotnet_channels.js` emits .NET channel types. Rules enforced by the generator include "Use `int` or `float` instead of `number`" ([generate_channels.js](https://github.com/microsoft/playwright/blob/v1.64.1/utils/generate_channels.js)). The timeout is not a parameter: each call carries `metadata { location, title, internal, timeout }` ([core.yml](https://github.com/microsoft/playwright/blob/v1.64.1/packages/protocol/spec/core.yml)), and the server sends `__create__`, `__adopt__` and `__dispose__` to manage client mirrors of server objects.
2. **The public API**: `docs/src/api/class-*.md` (54 class files). Each member carries `* since:`, optional `* discouraged:` / `* deprecated:`, and `* langs:` annotations (461 of them) with per-language aliases (`alias-python: expect_response`, `alias-csharp: RunAndWaitForResponse`) or language-only overloads (`langs: python` gives `waitForResponse` an `EventContextManager` return type). `utils/doclint/generateApiJson.js` turns this into `api.json`.
   - **TypeScript**: `utils/generate_types` builds `types.d.ts` from the docs plus hand-written `overrides.d.ts`.
   - **Java**: `scripts/generate_api.sh` runs upstream `generateApiJson.js` and `tools/api-generator/ApiGenerator.java` generates the public **interfaces**; the `*Impl` classes are hand-written.
   - **.NET**: `utils/doclint/generateDotnetApi.js` (in the main repo) generates interfaces and XML docs; the implementation is hand-written.
   - **Python**: since 2026-10-12 ([playwright-python#3240](https://github.com/microsoft/playwright-python/pull/3240)) `scripts/codegen/generate_api.js` reads the upstream docs with the upstream parser and emits `async_api/_generated.py` and `sync_api/_generated.py` (about 24,000 lines each) plus the public TypedDicts; before that, the generator **introspected the hand-written `_impl` classes** and pulled docstrings from `api.json`. Python-only facts live in [`python_rules.js`](https://github.com/microsoft/playwright-python/blob/27e324a9a1ccb7525ebd1b3f187fc259d0513cfc/scripts/codegen/python_rules.js) under the policy "the shipped Python API is the reference… The preferred fix is a docs change upstream", each entry with a one-line reason: hand-written classes (`Error`, `TimeoutError`, `FormData`), context-manager classes, traceback-hiding classes, extra members (`playwright["chromium"]`), hidden options, skipped arguments. The generated methods are keyword-only, snake_case, accept `timedelta` for timeouts and call `self._impl_obj` (async) or `self._sync(...)` (sync).
3. **Hand-written per-language implementations** on top: `client/*.ts`, Python `_impl/*.py`, Java `impl/*Impl.java`, .NET `Core/*.cs`. All four speak the same protocol to the same Node server, so behaviour (auto-wait, strictness, retries, call logs) lives once, in the server.
4. **The server knows the caller's language.** `SDKLanguage` is a protocol enum; the injected script renders locators with `asLocator(sdkLanguage, …)` in errors, and `packages/isomorphic/codegen/` has one generator per target (JS test/library, Python pytest/sync/async, C# MSTest/NUnit/xUnit/library, Java JUnit/library, JSONL) shared by codegen, the inspector and traces.
5. **What does not port evenly**: `AbortSignal` is JS-only; web-first assertions are a test-runner feature in JS but part of the library in Python, Java and .NET; Java renames `goto` to `navigate` (reserved word); "arm the waiter, then trigger" has four idioms (promise, context manager, lambda, `RunAndWait…Async`); no port exposes events as async iterators; language bindings cannot route logs into the host logging framework ([#6465](https://github.com/microsoft/playwright/issues/6465), +82, not planned).

**Selenium**: five hand-written bindings in one Bazel monorepo sharing JavaScript "atoms" and Selenium Manager; CDP bindings generated per Chrome milestone (`v153`–`v155` at 4.51.0, "the 3 most recent versions of Chrome"); since June–October 2026 the internal BiDi layer is generated from the CDDL through one binding-neutral schema for JS, Python and Java, with a written behavioural contract because the bindings had drifted (Java ran one handler "chosen in no defined order", Ruby raced handlers in threads, .NET and JS had none) (§2.3).

**WebdriverIO**: one language; its typed BiDi client is generated by its own CDDL parser. **Puppeteer**: one language; ports are community and hand-written (PuppeteerSharp tracks the version number; pyppeteer is unmaintained). **Rust**: chromiumoxide generates 111k lines of CDP types from PDL; playwright-rs mirrors playwright-python's API over the Node driver; rustwright reimplements Playwright natively and treats Python and Node as its main consumers. **Stagehand v4** (prior note): Zod → JSON Schema → Python and Go models.

**Bearing on Goodfetch's design (ADR 0038: Rust types → JSON Schema bundle and catalog → SDKs).** Playwright separates the wire contract (YAML) from the public, documented API (Markdown with language annotations) and hand-writes each client, so the public API can be idiomatic per language while behaviour stays in one server; the cost is two sources of truth kept aligned by linting, and a hand-written client per language. Goodfetch's catalog is one source for both, which removes that drift, but then needs somewhere to put what Playwright keeps in `* langs:` and `python_rules.js`: per-language aliases and reserved words (Rust `type`, `move`, `match`; Python `async`, `from`; Java `goto`), language-only overloads (context-manager waiters in Python, `Stream`s in Rust, `AbortSignal` in TS), defaults expressed in each language's types (`timedelta`, `Duration`), and which classes are hand-written. Playwright's language-aware server (errors and traces that print the caller's own syntax) is a cheap, high-value idea for the same architecture.

## 5. What users like, ranked

Ranked by independent reports across libraries (distinct non-vendor authors), then corroboration by migration posts and Stack Overflow. Quotes are under 15 words; sources in Appendix A and §2.

| Rank | Like | Evidence weight | Representative evidence | Lesson for Goodfetch |
|---|---|---|---|---|
| 1 | **Auto-waiting / actionability before every action** | 8+ distinct HN authors (2021–2026), Selenium's creator among them; 4 migration posts (WordPress, Nutrient, BigBinary, NocoDB); Selenium's #1 pain is its absence | "Auto-waiting - it is a BRILLIANT feature!" ([HN 33543274](https://news.ycombinator.com/item?id=33543274)); "waiting until an element was actually ready" (hugs, [HN 44964169](https://news.ycombinator.com/item?id=44964169)) | Guarded actions by default (ADR 0019 already goes further than Playwright: uniqueness, hit target, cancel-and-replay); list the checks and the last observed state in every timeout error. |
| 2 | **Plain async/await (or plain blocking calls), no hidden command queue** | 7 HN authors; NocoDB and BigBinary posts; the queue designs were removed (WebDriverJS, WDIO fibers) or are the top complaint (Cypress #1417, 492 reactions) | "Meanwhile Playwright's API is just promises." ([HN 30109346](https://news.ycombinator.com/item?id=30109346)) | Each SDK uses its language's native model; results are return values; retrying lives inside one awaited call. |
| 3 | **Trace viewer and debugging artefacts** | 6 HN/forum authors plus 4 migration posts; one counter (rrweb preferred by browser-use) | "The tracing tools are incredible" ([HN 30106518](https://news.ycombinator.com/item?id=30106518)) | Engine-level traces (actions, DOM/aria snapshots, network, console, call metadata with the user's source line) for every SDK and for crawls, not tied to a runner. |
| 4 | **One official API in several languages** | 8 HN authors; Python SO tags growing; pyppeteer's abandonment pushed users to Playwright | "official support for languages outside of JavaScript" ([HN 30106763](https://news.ycombinator.com/item?id=30106763)) | Validates the three-SDK plan; keep shapes identical and behaviour in the engine. |
| 5 | **Cleaner, smaller API than Selenium / WebDriver** | 7 HN authors | Selenium "API has NOT aged well" ([HN 33538602](https://news.ycombinator.com/item?id=33538602)) | Page-centric verbs, no driver object threaded through calls, interception and uploads as plain calls. |
| 6 | **Isolated contexts, multiple pages/users/origins as values** | 5 HN/forum authors; Cypress #136 (1,045 reactions) and #944 (426) on the other side | "context switching made it a breeze" ([HN 30107370](https://news.ycombinator.com/item?id=30107370)) | Cheap contexts; pages, popups and frames (including cross-origin) as values. |
| 7 | **Locators that read like the user's view** (role, text, label) | 6 authors; counterweights in §6 P10 | "You can query by text, label, html semantics, position." ([HN 33554766](https://news.ycombinator.com/item?id=33554766)) | Offer `get_by_role` and friends next to CSS, id and test-id, with equal standing. |
| 8 | **One command installs matching browsers / drivers** | 4 HN authors; Selenium Manager (4.6/4.11) fixed Selenium's most-voted pain; thirtyfour's `WebDriver::managed` | "within 30 seconds Playwright will download it's needed browsers" ([HN 46378927](https://news.ycombinator.com/item?id=46378927)) | Managed, pinned, inspectable provisioning; never at first launch in production (§6 P2). |
| 9 | **Network interception and an API client in the same tool** | 4 HN authors; Cypress `cy.intercept` praised; aliases as wait points | "I really really like the cy.intercept()" ([HN 33543799](https://news.ycombinator.com/item?id=33543799)) | `route` with written semantics; request/response events usable as wait conditions. |
| 10 | **Query builders and chaining** (thirtyfour `query()`, WDIO `$` chains, Playwright `filter`/`and`/`or`/`within`) | thirtyfour author and users; WDIO v10 motives | "I've grown quite fond of the query builder" ([fantoccini#145](https://github.com/jonhoo/fantoccini/issues/145)) | Composable, declarative filters evaluated engine-side (no host callbacks in the page). |
| 11 | **Stock browsers and easy raw protocol access** (Puppeteer `CDPSession`, chromiumoxide's generated types) | 4 HN authors plus Google's Puppeteer lead | Puppeteer wins on "not shipping patched binaries" ([HN 30119396](https://news.ycombinator.com/item?id=30119396)) | Drive stock Chromium; expose a raw session per target as a documented, secondary escape hatch. |

## 6. Pain points, ranked by evidence weight

Weight = independent reports across libraries, votes and reactions, recency, and whether the problem is still open. Per-library detail is in §2; Appendix A has the Stack Overflow clusters.

### P1. Timing and synchronization, even with auto-wait
- **Evidence.** Stack Overflow: 53 of the 375 top Playwright/Puppeteer questions, 1,686 votes (waits, existence, navigation races, sleeps); Selenium's top wait questions have 356 and 304 votes and 849k views ([26566799](https://stackoverflow.com/q/26566799), [5868439](https://stackoverflow.com/q/5868439)); 7 independent HN complaints that auto-wait is "great, until they don't work the way you want them to" ([HN 44628233](https://news.ycombinator.com/item?id=44628233)); strictness that fails at once instead of waiting ([#36045](https://github.com/microsoft/playwright/issues/36045), [#30363](https://github.com/microsoft/playwright/issues/30363)); Rust: waiting is entirely the user's job in four crates (about 13 reports), `networkidle` requested since 2021 (chromiumoxide [#36](https://github.com/mattsse/chromiumoxide/issues/36)); WDIO animation clicks ([#3106](https://github.com/webdriverio/webdriverio/issues/3106), 48 comments).
- **Status.** Open everywhere; Playwright's answer is web-first assertions and `waitFor*` helpers; Selenium declined auto-wait.
- **Lesson.** Auto-wait covers "element ready", not "app ready". Ship first-class, composable waits for app state (URL, response, request finished, count, state, load state including network idle, custom predicate) under one deadline model, decide whether a strict-mode ambiguity waits within the deadline (Goodfetch's guard can wait for uniqueness), and put "what we waited for, the last observed state and where the limit came from" in every timeout error.

### P2. Provisioning: installing, finding and launching browsers and drivers
- **Evidence.** SO: 34 questions / 734 votes in the Playwright/Puppeteer sample; Selenium's driver-mismatch questions reach 643 votes and 1.0M views ([40208051](https://stackoverflow.com/q/40208051)); Puppeteer arm64 [#7740](https://github.com/puppeteer/puppeteer/issues/7740) (+140) and Apple Silicon [#6622](https://github.com/puppeteer/puppeteer/issues/6622) (+138); Playwright distro requests Arch [#8100](https://github.com/microsoft/playwright/issues/8100) (+174), CentOS [#9199](https://github.com/microsoft/playwright/issues/9199) (+98), Red Hat/Oracle [#37812](https://github.com/microsoft/playwright/issues/37812) (+63, open), Ubuntu 26.04 [#40117](https://github.com/microsoft/playwright/issues/40117) (+103); the Node runtime inside .NET/Java/Python packages ([playwright-dotnet#1850](https://github.com/microsoft/playwright-dotnet/issues/1850), +49, open; Simon Willison's "a full copy of the Node.js binary"); Rust: about 20 launch reports (port scans, English-only stderr parsing, a `devtools` default flipped in a patch release); Streamlit users switching to Selenium.
- **Status.** Selenium Manager and Chrome for Testing fixed much of it; patched Firefox/WebKit keep Playwright distribution-bound; telemetry consent open ([Selenium #14643](https://github.com/SeleniumHQ/selenium/issues/14643)).
- **Lesson.** The engine owns provisioning: pinned to the SDK version, explicit and inspectable (`which browser, where, which version`), prefer an installed stock Chromium when compatible, a doctor command that names missing OS libraries, pipe or `DevToolsActivePort` instead of stderr parsing and port scans, no network at build time, no telemetry unless opted in. The Rust engine removes the Node runtime that Playwright's ports carry, a real differentiator.

### P3. Lifecycle, teardown and long-running leaks
- **Evidence.** Rust: about 16 reports across five crates, all rooted in the lack of async `Drop` (chromiumoxide `Drop` only warns; thirtyfour blocks the executor on a side runtime; fantoccini's fire-and-forget `DELETE` is lost when the runtime stops; jonhoo: "plagues the async ecosystem"); Playwright memory growth in long-lived contexts [#6319](https://github.com/microsoft/playwright/issues/6319) (+60), [#15400](https://github.com/microsoft/playwright/issues/15400) (+24) with the maintainer's "Playwright is focused on testing scenarios, where contexts are constantly closed"; Java long-running leaks ([playwright-java#1816](https://github.com/microsoft/playwright-java/issues/1816), [#1841](https://github.com/microsoft/playwright-java/issues/1841)); Puppeteer zombies [#1825](https://github.com/puppeteer/puppeteer/issues/1825) (+89) and memory [#5893](https://github.com/puppeteer/puppeteer/issues/5893) (+56); Selenium quit/close confusion (361 votes) and leaked drivers (133); "user data directory is already in use" (Selenium [#15327](https://github.com/SeleniumHQ/selenium/issues/15327), WDIO [#14501](https://github.com/webdriverio/webdriverio/issues/14501)); SO memory cluster for Puppeteer scraping.
- **Status.** Mitigated by scoped helpers (`await using`, thirtyfour `run_browser_test`) and by Playwright's LRU of 10,000 server objects per type ("The object has been collected to prevent unbounded heap growth").
- **Lesson.** For Foxhound especially: recycling contexts and browsers is a policy the engine applies, not advice; the engine reaps an SDK's sessions when its connection drops; per-SDK scoped close (`async with`, `await using`, a Rust helper that passes handles by value and closes on unwind) with `Drop` as a warning-only fallback; a documented handle-collection policy so a crawler's millionth handle does not fail mysteriously.

### P4. Getting data out of the page: text, attributes, `evaluate` and its serialization boundary
- **Evidence.** SO: 53 questions / 1,472 votes (reads, selection, `evaluate`); "How can I pass variable into an evaluate function?" is the second most-voted Puppeteer question (293 votes, [46088351](https://stackoverflow.com/q/46088351)); Puppeteer locator `filter` closures fail silently ([#11893](https://github.com/puppeteer/puppeteer/issues/11893)); `cy.origin` "is not a closure"; HN "the biggest annoyance with `page.evaluate` is transpiling javascript"; Rust: `find_elements` 150× slower than JS from per-element round trips ([headless_chrome #460](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/460)).
- **Status.** Playwright's typed getters and `evaluate(fn, arg)` reduced it; Playwright 1.62 accepts functions as `evaluate` arguments.
- **Lesson.** Typed getters on the locator for common reads (text, inner text, value, attribute, HTML, count, all texts, bounding box) resolved engine-side in one round trip; for page scripts, an explicit argument parameter and a documented serialization contract per SDK (the page script is JavaScript source even from Python and Rust), and a loud error on references to host variables or non-serialisable returns. Host-language predicates must run in the host, on streamed events, never be shipped to the page.

### P5. Eager handles, stale references and handle-model confusion
- **Evidence.** Selenium `StaleElementReferenceException` (182 votes, 743k views, [18225997](https://stackoverflow.com/q/18225997)) and click interception (486 votes, [11908249](https://stackoverflow.com/q/11908249)); Cypress detached-DOM [#7306](https://github.com/cypress-io/cypress/issues/7306) (541 reactions, fixed in 12.0); Playwright `ElementHandle` vs `Locator` and strict-mode questions (SO full-text 19, three migration posts); Puppeteer "Execution context was destroyed" (53 full-text SO hits, [#3323](https://github.com/puppeteer/puppeteer/issues/3323) +38) and "Cannot find context with specified id" ([#1325](https://github.com/puppeteer/puppeteer/issues/1325), +104).
- **Status.** Solved by lazy locators where adopted; Selenium unchanged.
- **Lesson.** One lazy, re-resolving locator type as the default; pinned handles only on request, with errors that explain why they detached; strict by default with `first`/`nth`/`all` as visible opt-outs, and strict-mode errors that list the matches and suggest a disambiguating locator (Playwright already prints matches; Goodfetch can print them in the caller's language).

### P6. Navigation races and "arm the waiter before the trigger"
- **Evidence.** Puppeteer's `Promise.all([page.waitForNavigation(), page.click()])` idiom and its hangs (SO [46948489](https://stackoverflow.com/q/46948489), 131 votes; [49422819](https://stackoverflow.com/q/49422819); [#1412](https://github.com/puppeteer/puppeteer/issues/1412)); Playwright discourages `waitForNavigation` because "it's hard to make it work reliably" ([#20853](https://github.com/microsoft/playwright/issues/20853)); Playwright's four per-language idioms for the same pattern (§4.3); chromiumoxide's `click()` docs require a separate `wait_for_navigation()`; rustwright [#79](https://github.com/Skyvern-AI/rustwright/issues/79) waits dying mid-navigation.
- **Lesson.** Actions that may navigate should be able to return or await the navigation they caused; waits survive navigations and context destruction; typed errors distinguish "target crashed", "page closed by your code" and "navigated away". Where an "expect X while doing Y" API is needed, give each SDK one idiomatic shape (TS promise, Python context manager, Rust future joined with the action) generated from the same catalog entry.

### P7. Concurrency model friction: hidden queues, sync vs async, threads, cancellation
- **Evidence.** Cypress "Await-ing Cypress Chains" [#1417](https://github.com/cypress-io/cypress/issues/1417) (492 reactions, open since 2018, about 14 independent reports); WebDriverJS promise manager removed for "reduced debuggability" ([Selenium #2969](https://github.com/SeleniumHQ/selenium/issues/2969), 92); WDIO fibers removal forced rewrites; Playwright Python: "Sync API inside the asyncio loop" across SO, Streamlit (7,861 views over two threads), CrewAI and discuss.python.org; not thread-safe in Python and Java; task cancellation "undefined behavior"; .NET `CancellationToken` declined but still requested in 2025–2026; Rust: requests for a sync wrapper (fantoccini [#43](https://github.com/jonhoo/fantoccini/issues/43), the crate's most-voted) and for async (headless_chrome [#181](https://github.com/rust-headless-chrome/rust-headless-chrome/issues/181)).
- **Lesson.** Native async in every SDK plus a real blocking client for Python (a separate synchronous JSON-RPC client, not greenlets) that works when a loop is already running (Jupyter, Streamlit, agent frameworks) or fails with the fix; handles safe to share across threads/tasks; cancellation that actually stops the engine-side operation in every SDK (ADR 0038's `$/cancel`), surfaced as `AbortSignal`, `asyncio` cancellation and dropped Rust futures. This is a gap Playwright has at the protocol level only for JS.

### P8. Weak, untyped or stringly errors
- **Evidence.** Playwright Python: "convert the thrown exception to a string and match based on its contents" ([#1425](https://github.com/microsoft/playwright-python/issues/1425), +12, not planned) and `TimeoutError` shadowing the builtin ([#1495](https://github.com/microsoft/playwright-python/issues/1495), +12, not planned); Rust: about 16 reports (String errors, panics in library code, "Request timed out." with no operation, private error types); Puppeteer messages without context; Selenium exceptions naming the symptom, not the cause; nodriver returning `None` where the docs promise an exception; playwright-rs needing string matches on `"TimeoutError"` until recently.
- **Positive counter-evidence.** Playwright's call log and API-name prefix are the most informative messages in the field; thirtyfour's `.desc()` and WebdriverIO's `StrictSelectorError` show the move toward named targets.
- **Lesson.** ADR 0038's typed codes already answer the hierarchy request; add structured fields (operation, locator rendered in the caller's language, URL, timeout and elapsed, retryable) plus a Playwright-style action log, never raw protocol text as the primary message, and names that do not shadow builtins (`TimeoutError` in Python).

### P9. Network interception and body access
- **Evidence.** Puppeteer response interception [#1191](https://github.com/puppeteer/puppeteer/issues/1191) (+139, open since 2017) and racing handlers needing `isInterceptResolutionHandled()`; Playwright `Response.bodyAsStream()` [#17199](https://github.com/microsoft/playwright/issues/17199) (+60, open), "Enabling routing disables http cache" ([#7220](https://github.com/microsoft/playwright/issues/7220), +78) and route-not-seeing-Service-Workers; "Request content was evicted from inspector cache" in both ([Playwright #13449](https://github.com/microsoft/playwright/issues/13449), [Puppeteer #6647](https://github.com/puppeteer/puppeteer/issues/6647)); Rust: about 12 reports (raw `Fetch` events, interception declined by fantoccini); Selenium's BiDi network API beta since 2024 ([#13993](https://github.com/SeleniumHQ/selenium/issues/13993)); Cypress body matching [#8560](https://github.com/cypress-io/cypress/issues/8560) and WebSockets [#2492](https://github.com/cypress-io/cypress/issues/2492).
- **Lesson.** For a crawler this is core: written handler semantics (Selenium ADR 17685 is a ready template), response bodies on opt-in without eviction surprises, streaming bodies, interception that does not silently disable the HTTP cache (or says so loudly), Service-Worker behaviour stated, and an interception guard whose removal does not depend on an ambient runtime.

### P10. Test-runner coupling and philosophy-driven defaults
- **Evidence.** 58 of 276 Playwright SO questions (21%) are about the runner, not the browser; the most-voted question in the Playwright/Puppeteer sample is Puppeteer users hitting Jest's 5 s timeout (506 votes); Playwright's library and runner differ on timeouts and on whether assertions exist; Playwright's own maintainers close agent and scraping requests as outside "the core QA testing use-case" (browser-use, [HN 44964448](https://news.ycombinator.com/item?id=44964448)); locator philosophy pushback (`getById` declined, [#19763](https://github.com/microsoft/playwright/issues/19763), +81; `getByText` substring default, [#19235](https://github.com/microsoft/playwright/issues/19235); role locators hard to find in DevTools, Ministry of Testing).
- **Lesson.** Collie and Foxhound are not test runners: retrying assertions, tracing, isolation and auth reuse are library features with one set of defaults; CSS, id and test-id locators sit next to role locators without a lecture.

### P11. Browser and protocol coupling, drift and patched builds
- **Evidence.** 5 HN reports on Playwright's patched Firefox/WebKit and "not real Safari"; Playwright `connect` requires matching major.minor; Selenium's `selenium-devtools-v1xx` per Chrome milestone; Rust: about 7 reports of Chrome updates breaking strict generated types (chromiumoxide [#243](https://github.com/mattsse/chromiumoxide/issues/243), fix after 13 months); zendriver and nodriver broken by Chrome 146 field renames; WDIO v9 BiDi default regressions and a 10–20% slowdown ([#14494](https://github.com/webdriverio/webdriverio/issues/14494)); Chrome now ships every two weeks (prior note).
- **Lesson.** Engine-side, generated, lenient protocol types; CI against stable and beta; publish the compatible browser range; a clear "incompatible browser" error instead of a hang; benchmark any protocol switch before making it default.

### P12. Abstraction without an escape hatch
- **Evidence.** browser-use left Playwright for raw CDP (2025-08-20) citing the Node relay hop, hangs and hidden targets and frames; 5 independent HN reports; Playwright's CDPSession wrapper "unnecessarily difficult" for worker profiling; language bindings cannot plug into host logging ([#6465](https://github.com/microsoft/playwright/issues/6465), +82, not planned).
- **Lesson.** A documented raw-protocol session per page, frame and worker from every SDK, events included, kept clearly secondary to the guarded API (and outside the stability promise); logs routed into `tracing`, Python `logging` and Node loggers.

### P13. Multi-tab, multi-origin and frame models
- **Evidence.** Cypress iframes [#136](https://github.com/cypress-io/cypress/issues/136) (1,045 reactions, open since 2016) and superdomains [#944](https://github.com/cypress-io/cypress/issues/944) (426); WDIO's switch pointer replaced in v10; chromiumoxide cannot send into cross-origin iframes ([#280](https://github.com/mattsse/chromiumoxide/issues/280)); Playwright needed a selector-less `frameLocator()` (1.63) to stop users locating iframes first.
- **Lesson.** Frames, including OOPIFs, as values reachable from a locator that crosses them transparently; multiple contexts per session for multi-user flows.

### P14. Detectability (crawler-relevant)
- **Evidence.** Selenium's most-voted detection question (645 votes); Playwright "Avoiding detection is out of the scope" ([#13838](https://github.com/microsoft/playwright/issues/13838)); stealth forks with real adoption (Patchright about 966k PyPI downloads a week) and real costs (Patchright disables the console domain: 34 failing upstream tests; lags upstream; rebrowser silent since 2025-05).
- **Lesson.** The protocol footprint (`Runtime.enable`, `Console.enable`, isolated worlds, flags) is an API design dimension: make main-world versus isolated-world evaluation a typed option and list what any stealth profile turns off. Policy is out of scope here.

### P15. Lower-weight items
- **Codegen output not production-grade** (7 HN reports); the locator picker is valued more than whole-script generation.
- **Fixed sleeps removed** (Puppeteer `waitForTimeout` in 22.0.0 still confuses users in 2024): keep a plainly named sleep, discouraged in docs, and never remove an API without a pointer in the error.
- **Ignored, deprecated options** (`isVisible({ timeout })` silently ignored; [#33017](https://github.com/microsoft/playwright/issues/33017)): an ignored option should warn or fail.
- **Governance and longevity** (HN, Ministry of Testing "Future of Playwright", 2025-12).

## 7. Open questions and unverified items

**Unverified or partial.**
- All Reddit content (blocked for every strand); second-hand quotes from vendor pages are not used as evidence.
- Stack Overflow tag sizes returned by the API (`playwright` 3,501) look low; treat as indicative.
- Playwright's immediate strict-mode failure is from source reading and issue history, not from a run at 1.64.1.
- Puppeteer: whether `[Symbol.asyncDispose]` was usable before 25.4.0 publicly "exposed" it.
- Python `signal`: absence inferred from the generated 1.64 API and the `langs: js` annotations; not run.
- WebdriverIO default `waitforTimeout`/`waitforInterval` values; Safari/WebKit BiDi status; the Runtime.enable detection mechanism detail; rebrowser-playwright's last code commit.
- Rust: chromiumoxide `find_element` miss message and a possible `find_xpath` panic path; the thirtyfour default wait (source 20 s, older maintainer statements 30 s); headless_chrome #500 version linkage (inferred from dates).
- No sample in this note was compiled or run.

**Design questions the evidence raises but does not settle (for the owner).**
1. Should a strict-mode ambiguity wait within the action deadline (Goodfetch's guard can) or fail at once (Playwright)? Users ask for waiting ([#36045](https://github.com/microsoft/playwright/issues/36045)); Playwright's maintainers argue it can hide the wrong page.
2. Library defaults: one action timeout for scripts, crawls and agents, or per-product defaults (Collie vs Foxhound)? The field ranges from 4 s to 30 s, and Playwright's own library and runner disagree.
3. Where do host-language predicates run? Streaming every request/response event to every SDK (Playwright's model) costs JSON-RPC traffic in a crawler; declarative filters evaluated engine-side are cheaper but less expressive.
4. Events: emitters, async iterators/streams, or both per language? No mainstream Node-era driver offers async iterators; Rust crates converged on `Stream`s.
5. Handle collection policy for long crawls: Playwright's LRU (10,000 per type) with a "collected" error, or explicit scopes?
6. A Python sync API: a separate blocking client over the same JSON-RPC (what WDIO's fibers removal and the asyncio errors argue for) or none?
7. How the catalog (ADR 0038) carries per-language aliases, reserved words and language-only overloads that Playwright keeps in `* langs:` annotations and `python_rules.js`.

## Appendix A. Community evidence for Playwright and Puppeteer (Stack Overflow, Hacker News, forums, migration posts)

Collected 2026-10-14 for Goodfetch (consumer page API in TS/Python/Rust over one Rust engine via JSON-RPC, plus a crawler). Read-only research; GitHub issues deliberately excluded (covered by another agent). Quotes are under 15 words, attributed, with links. "Vendor" marks content from a company selling a related product or service.

---

### Method

| Source | How | Coverage | Status |
|---|---|---|---|
| Stack Overflow | Stack Exchange API 2.3, `questions?sort=votes`, pages 1–2 (100 each) for `playwright`, `playwright-python`, `playwright-test`, `puppeteer`; plus `search/advanced` title and full-text counts across whole tags, and targeted error-string searches | 375 unique top-voted questions (276 Playwright-tagged, 99 Puppeteer-only); clustered with a keyword classifier plus 40 manual overrides; every assignment reviewed by hand | Done |
| Hacker News | Algolia API: story search (`points>50` and `num_comments>15`), then full comment trees via `/api/v1/items/<id>` for 21 threads (~2,000 comments) and comment-level keyword searches | Threads 2017–2026 | Done |
| Reddit | `reddit.com` and `old.reddit.com` return 403 to curl; WebFetch refuses reddit.com; WebSearch refuses `reddit.com` as a domain; redlib/safereddit mirrors sit behind Anubis proof-of-work challenges (not bypassed); Arctic Shift API timed out; PullPush returned 429 and asks for payment | — | **Blocked.** Reddit statements appear only second-hand (vendor pages quoting Reddit) and are all marked **unverified** in the Unverified list below |
| Substitute community forums | Discourse JSON APIs: Ministry of Testing Club (QA community), discuss.python.org, Streamlit forum, CrewAI community | 12 threads | Done (MoT rate-limited after ~8 threads) |
| Lobsters | Behind Anubis challenge | — | Blocked, not bypassed |
| Migration and critical blog posts | WebSearch + WebFetch | 8 posts read in full | Done |
| Package downloads | `api.npmjs.org/downloads/point/last-week/<pkg>`, `pypistats.org/api/packages/<pkg>/recent` | 7 npm, 7 PyPI packages | Done |

Caveats:
- SO vote totals are biased toward Puppeteer: its top questions are from 2017–2019 (88 of 99) and have had years to accumulate votes. Playwright's top-100 per tag has a minimum score of 10 (`playwright`) and 3 (`playwright-python`, `playwright-test`), against 26 for `puppeteer`. Compare question *counts* within each tool more than votes across tools.
- SO tag sizes reported by the API on 2026-10-14: `puppeteer` 8,001; `playwright` 3,501; `playwright-python` 543; `playwright-test` 538; `playwright-typescript` 215; `playwright-java` 122; `playwright-sharp` 53; `puppeteer-sharp` 251; `pyppeteer` 178. These counts are as the API returned them; they look low for `playwright`, so treat them as indicative.
- "Independent report" means a distinct author on a distinct forum post. Vendor authors and tool authors are counted separately.

---

### Stack Overflow themes

#### Clustered top-voted questions (375 questions, 4 tags, fetched 2026-10-14)

Votes are question scores. PW = Playwright-tagged, Pptr = Puppeteer-only.

| # | Theme | Qs | Votes | PW / Pptr | Top 3 overall (score, year) | Top Playwright question |
|---|---|---|---|---|---|---|
| 1 | **Waiting** for load / element / network / animation / "until" | 27 | 858 | 19 / 8 | [Wait until page is completely loaded](https://stackoverflow.com/q/52497252) (143, 2018); [Wait page load after form submit](https://stackoverflow.com/q/46948489) (131, 2017); [Scroll down until you can't anymore](https://stackoverflow.com/q/51529332) (111, 2018) | [Wait until there is no animation](https://stackoverflow.com/q/71937343) (40, 2022); [Wait for element to be clickable](https://stackoverflow.com/q/73079690) (22, 2022); [Globally change timeout](https://stackoverflow.com/q/67787470) (20, 2021) |
| 2 | **Install / launch / browsers / Docker / CI** | 34 | 734 | 24 / 10 | [Failed to launch the browser process](https://stackoverflow.com/q/59979188) (116, 2020); [Chromium revision is not downloaded](https://stackoverflow.com/q/53997175) (77, 2019); [Failed to set up Chromium r782078](https://stackoverflow.com/q/63187371) (69, 2020) | [Use installed version of Chrome](https://stackoverflow.com/q/62281859) (29, 2020); [Missing dependencies in Docker](https://stackoverflow.com/q/65802328) (19, 2021); [How to update Playwright?](https://stackoverflow.com/q/75163236) (13, 2023) |
| 3 | **Input actions** (fill, select, press, click, scroll, drag) | 21 | 674 | 13 / 8 | [Fill an input field](https://stackoverflow.com/q/47966510) (119, 2017); [Select option from dropdown](https://stackoverflow.com/q/45864516) (113, 2017); [Pressing Enter](https://stackoverflow.com/q/46442253) (79, 2017) | [Auto-scroll infinite-scroll page](https://stackoverflow.com/q/69183922) (23, 2021); [Select dropdown option](https://stackoverflow.com/q/66702908) (14, 2021); [Drag and drop](https://stackoverflow.com/q/64718915) (10, 2020) |
| 4 | **Browser configuration** (viewport, window, language, date, JS off, headless quirks) | 27 | 608 | 19 / 8 | [Opening local HTML file](https://stackoverflow.com/q/47587352) (96, 2017); [Set max viewport](https://stackoverflow.com/q/52553311) (70, 2018); [Window size when not headless](https://stackoverflow.com/q/48681145) (61, 2018) | [Safari vs WebKit difference](https://stackoverflow.com/q/62184117) (35, 2020); [Resize browser at runtime](https://stackoverflow.com/q/69942054) (24, 2021); [Set the date](https://stackoverflow.com/q/63960549) (20, 2020) |
| 5 | **`evaluate` bridge** (passing variables/functions, serialization, logging) | 10 | 586 | 3 / 7 | [Pass variable into evaluate](https://stackoverflow.com/q/46088351) (293, 2017); [Log inside page.evaluate](https://stackoverflow.com/q/46198527) (73, 2017); [Pass a function to evaluate](https://stackoverflow.com/q/47304665) (53, 2017) | [Run a custom JS function](https://stackoverflow.com/q/63220757) (10, 2020); [Outer-scope variables in addInitScript](https://stackoverflow.com/q/78903490) (4, 2024); [evaluateAll ReferenceError](https://stackoverflow.com/q/69249230) (3, 2021) |
| 6 | **Network** (intercept, capture responses, headers, proxy, console) | 26 | 497 | 20 / 6 | [All console messages](https://stackoverflow.com/q/47539043) (102, 2017); [Capture all requests and responses](https://stackoverflow.com/q/52969381) (60, 2018); [Use a proxy](https://stackoverflow.com/q/52777757) (45, 2018) | [Wait for and validate responses](https://stackoverflow.com/q/66789642) (33, 2021); [console.log in Playwright](https://stackoverflow.com/q/75065382) (19, 2023); [Custom headers](https://stackoverflow.com/q/62605422) (16, 2020) |
| 7 | **Auth / session / cookies / storage / attach to existing browser** | 25 | 480 | 17 / 8 | [Manage login session](https://stackoverflow.com/q/48608971) (86, 2018); [Save cookies, load in another session](https://stackoverflow.com/q/56514877) (53, 2019); [How to login](https://stackoverflow.com/q/50074799) (48, 2018) | [Connect to existing browser session](https://stackoverflow.com/q/71362982) (29, 2022); [Add cookies in a test](https://stackoverflow.com/q/64211501) (18, 2020); [Get cookie](https://stackoverflow.com/q/64664883) (13, 2020) |
| 8 | **Selecting elements / locator semantics** (text, id, XPath, collections, children, parent, strict mode) | 21 | 469 | 17 / 4 | [Click element with text](https://stackoverflow.com/q/47407791) (150, 2017); [Collect by class, click each](https://stackoverflow.com/q/48673906) (55, 2018); [Select element by id](https://stackoverflow.com/q/75151754) (41, 2023) | [Select element by id](https://stackoverflow.com/q/75151754) (41, 2023); [Get a collection of elements](https://stackoverflow.com/q/61453673) (34, 2020); [Iterate a list and click each](https://stackoverflow.com/q/70138360) (18, 2021) |
| 9 | **Test-runner coupling and config** (no tests found, baseURL, .env, hooks, fixtures, runner-only features) | 58 | 450 | 58 / 0 | [VS Code "No tests found"](https://stackoverflow.com/q/72100106) (35, 2022); [Variables from .env](https://stackoverflow.com/q/70861292) (23, 2022); [Run grouped tests sequentially](https://stackoverflow.com/q/68000771) (21, 2021) | same |
| 10 | **Reading text / value / attribute / HTML** | 22 | 417 | 18 / 4 | [Get text inside div](https://stackoverflow.com/q/55237748) (62, 2019); [Value of input element](https://stackoverflow.com/q/62002041) (51, 2020); [Get innerHTML](https://stackoverflow.com/q/46431288) (43, 2017) | [Value of input element](https://stackoverflow.com/q/62002041) (51, 2020); [Current page URL](https://stackoverflow.com/q/64308751) (40, 2020); [Element attribute in a test](https://stackoverflow.com/q/71325935) (28, 2022) |
| 11 | **Existence / visibility checks** | 13 | 361 | 10 / 3 | [Wait until element visible](https://stackoverflow.com/q/46135853) (95, 2017); [Check element exists](https://stackoverflow.com/q/64784781) (58, 2020); [Assert element NOT on page](https://stackoverflow.com/q/71183099) (58, 2022) | [Check element exists](https://stackoverflow.com/q/64784781) (58, 2020); [Assert NOT on page](https://stackoverflow.com/q/71183099) (58, 2022); [Check element visible](https://stackoverflow.com/q/70179441) (25, 2021) |
| 12 | **Files: download / upload / PDF / screenshot** | 17 | 358 | 10 / 7 | [Wait for images, then screenshot](https://stackoverflow.com/q/46160929) (64, 2017); [Download file headless](https://stackoverflow.com/q/49245080) (48, 2018); [PDF missing background colors](https://stackoverflow.com/q/60736354) (45, 2020) | [Upload from non-input element](https://stackoverflow.com/q/66132097) (15, 2021); [Clipboard contents](https://stackoverflow.com/q/72265518) (13, 2022); [Download PDF (Python)](https://stackoverflow.com/q/68409249) (12, 2021) |
| 13 | **Navigation races / "Target closed" / "Execution context destroyed"** | 9 | 244 | 6 / 3 | [Protocol error: Target closed](https://stackoverflow.com/q/51629151) (78, 2018); [Execution context was destroyed](https://stackoverflow.com/q/55877263) (62, 2019); [Navigation failed, browser disconnected](https://stackoverflow.com/q/54527982) (35, 2019) | [Target closed after navigation](https://stackoverflow.com/q/61933492) (31, 2020); [ERR_ABORTED; frame was detached?](https://stackoverflow.com/q/73359803) (16, 2022); [page.url() returns previous URL](https://stackoverflow.com/q/68488352) (12, 2021) |
| 14 | **Fixed sleeps** | 4 | 223 | 3 / 1 | [Wait N seconds](https://stackoverflow.com/q/46919013) (166, 2017); [Pause 3 seconds](https://stackoverflow.com/q/69806337) (38, 2021); [Fixed wait like cy.wait](https://stackoverflow.com/q/74920905) (13, 2022) | same (PW) |
| 15 | **Tabs / popups / iframes** | 12 | 205 | 9 / 3 | [Handle multiple tabs](https://stackoverflow.com/q/45806684) (56, 2017); [Fill form inside iframe](https://stackoverflow.com/q/46529201) (47, 2017); [Empty tab in headful mode](https://stackoverflow.com/q/47744369) (41, 2017) | [Open link in new tab](https://stackoverflow.com/q/64277178) (18, 2020); [Switch tabs in a test](https://stackoverflow.com/q/64348468) (12, 2020); [Button in iframe by index (Python)](https://stackoverflow.com/q/73807634) (7, 2022) |
| 16 | **Bot detection / stealth / CAPTCHA** | 9 | 178 | 6 / 3 | [Bypassing CAPTCHAs](https://stackoverflow.com/q/55678095) (65, 2019); [Avoid being detected as bot](https://stackoverflow.com/q/51731848) (35, 2018); [Fingerprint simulation](https://stackoverflow.com/q/57737610) (27, 2019) | [Login to Google account](https://stackoverflow.com/q/65139098) (18, 2020); [Playwright stealth](https://stackoverflow.com/q/67500031) (12, 2021); [Bright Data: password typing not allowed](https://stackoverflow.com/q/76131573) (9, 2023) |
| 17 | **Memory / concurrency / resources** | 4 | 130 | 0 / 4 | [Managing memory and performance](https://stackoverflow.com/q/51971760) (40, 2018); [Multiple instances safely](https://stackoverflow.com/q/48317725) (33, 2018); [Crawling URLs in a loop](https://stackoverflow.com/q/46293216) (29, 2017) | — |
| 18 | **Python sync vs async / event loops / notebooks** | 10 | 64 | 10 / 0 | [Use in Jupyter on Windows](https://stackoverflow.com/q/71228742) (12, 2022); [Sync vs async API differences](https://stackoverflow.com/q/65439214) (12, 2020); [Start outside `with` context manager](https://stackoverflow.com/q/73043207) (8, 2022) | also [Sync API inside the asyncio loop](https://stackoverflow.com/q/79344524) (5, 2025); [Future belongs to a different loop](https://stackoverflow.com/q/78617556) (4, 2024) |
| 19 | Off-topic (Jest, VS Code, Node, frameworks) | 21 | 1,011 | 11 / 10 | [Jest 5000 ms timeout](https://stackoverflow.com/q/49603939) (506, 2018) — note: Puppeteer users hitting a test-runner default timeout | — |
| 20 | Other | 5 | 69 | 3 / 2 | [Reload page](https://stackoverflow.com/q/55236975) (33, 2019) | — |
| | **Total** | **375** | | 276 / 99 | | |

Grouped families:
- **Synchronization** (themes 1, 11, 13, 14): 53 questions, 1,686 votes. Largest family for both tools.
- **Getting data out of the page** (themes 5, 8, 10): 53 questions, 1,472 votes.
- **Environment** (themes 2, 4): 61 questions, 1,342 votes.
- **Playwright Test runner** (theme 9): 58 of 276 Playwright questions (21%) concern the runner, not the browser API.

#### Whole-tag counts by title keyword (Stack Exchange `search/advanced?title=`, 2026-10-14)

| Title keyword | `playwright` | `puppeteer` |
|---|---|---|
| wait | 77 | 159 |
| text | 92 | 193 |
| locator | 92 | — |
| docker | 74 | 148 |
| timeout | 54 | 118 |
| install | 42 | 68 |
| login | 41 | 75 |
| iframe | 41 | 84 |
| download | 39 | 104 |
| visible | 38 | 36 |
| attribute | 27 | 53 |
| popup / new tab | 26 / 20 | — / 39 |
| upload | 25 | 32 |
| proxy | 23 | 69 |
| fixture | 21 | — |
| evaluate / `$eval` | 8 | 91 / 69 |
| exist | 12 | 16 |
| detect / stealth | 10 / 4 | 25 / 7 |
| Target closed | 7 | 29 |
| Execution context | — | 25 |
| waitForNavigation | — | 16 |
| Failed to launch | — | 69 |
| memory | 4 | 44 |
| strict mode / ElementHandle / asyncio / sync | 5 / 5 / 5 / 9 | — |

Full-text matches (title and body, so noisier): Playwright "strict mode violation" 19, "Executable doesn't exist" 34, "Timeout exceeded" 121, "storageState" 100, "Target closed" 62, "flaky" 24; Puppeteer "Failed to launch" 277, "waitForNavigation" 55, "Execution context was destroyed" 53, "waitForTimeout" 15.

#### Targeted error-string questions

| Error / topic | Representative questions |
|---|---|
| Playwright strict mode | [Locator error: strict mode violation](https://stackoverflow.com/q/76043713) (11, 2023); [getByLabel resolved to 2 elements](https://stackoverflow.com/q/75777710) (7, 2023); [Or operator may match both](https://stackoverflow.com/q/77124207) (3, 2023); [`'Locator' object is not callable` using `.first()`](https://stackoverflow.com/q/78150913) (3, 2024) |
| Python sync API inside asyncio | [Sync API inside the asyncio loop](https://stackoverflow.com/q/79344524) (5, 2025); [Call async method from greenlet](https://stackoverflow.com/q/69888850) (4, 2021); [Colab: asyncio.run() inside running loop](https://stackoverflow.com/q/73084593) (3, 2022); [pytest-playwright same error](https://stackoverflow.com/q/78628634) (2, 2024) |
| Puppeteer `waitForTimeout` removal | [waitForTimeout is not a function](https://stackoverflow.com/q/64805395) (19, 2020); [same, 2024](https://stackoverflow.com/q/78563408) (0, 2024) |
| Puppeteer `waitForNavigation` | [waitForNavigation hanging though page loaded](https://stackoverflow.com/q/49422819) (16, 2018); [waitForNavigation after click](https://stackoverflow.com/q/52211871) (9, 2018) |
| Puppeteer ElementHandle vs evaluate | [page.evaluate vs Puppeteer $ methods](https://stackoverflow.com/q/55664420) (43, 2019); [Convert ElementHandle to DOM element](https://stackoverflow.com/q/53072849) (21, 2018) |
| Playwright ElementHandle relatives | [Elements relative to ElementHandle (Python)](https://stackoverflow.com/q/65400809) (7, 2020); [Equivalent to Selenium FindElements](https://stackoverflow.com/q/74641044) (1, 2022) |
| Browser missing at runtime | [Executable doesn't exist in Cloud Function](https://stackoverflow.com/q/73171905) (11, 2022); [Can't run inside Docker](https://stackoverflow.com/q/69472876) (8, 2021); [Azure Pipeline](https://stackoverflow.com/q/71508646) (4, 2022); [Vercel](https://stackoverflow.com/q/76647611) (4, 2023) |
| Runner-only coupling | [Component test without the Playwright test runner](https://stackoverflow.com/q/79319117) (5, 2024); [Jest: "needs to be invoked via npx playwright test"](https://stackoverflow.com/q/76040003) (4, 2023); [`test()` not expected to be called here](https://stackoverflow.com/q/78016704) (5, 2024); [fixture 'page' not found (pytest)](https://stackoverflow.com/q/74135518) (8, 2022) |

---

### Hacker News findings

Threads read in full or keyword-filtered (story id, points, comments, date):
[33537772](https://news.ycombinator.com/item?id=33537772) "Don't use Selenium, use Playwright" (408, 171, 2022-11);
[30083042](https://news.ycombinator.com/item?id=30083042) "Playwright: Automate Chromium, WebKit and Firefox" (383, 140, 2022-01);
[44962869](https://news.ycombinator.com/item?id=44962869) "Leaving Playwright for CDP" (182, 118, 2025-08);
[41182847](https://news.ycombinator.com/item?id=41182847) "Puppeteer Support for Firefox" (655, 90, 2024-08);
[15028329](https://news.ycombinator.com/item?id=15028329) "Puppeteer: Headless Chrome Node API" (406, 100, 2017-08);
[20504942](https://news.ycombinator.com/item?id=20504942) "Puppeteer for Firefox" (335, 127, 2019-07);
[24209073](https://news.ycombinator.com/item?id=24209073) theheadless.dev (255, 99, 2020-08);
[27458735](https://news.ycombinator.com/item?id=27458735) "Playwright for .NET is now stable" (155, 64, 2021-06);
[26696363](https://news.ycombinator.com/item?id=26696363) playwright-test (101, 26, 2021-04);
[39702386](https://news.ycombinator.com/item?id=39702386) "Playwright Test Generator" (74, 25, 2024-03);
[25288541](https://news.ycombinator.com/item?id=25288541) benchmark (118, 29, 2020-12);
[17233371](https://news.ycombinator.com/item?id=17233371) "Observations running 2M headless sessions" (315, 84, 2018-06);
[44626796](https://news.ycombinator.com/item?id=44626796) "Ask HN: worst part of web E2E testing?" (2025-07);
[41173335](https://news.ycombinator.com/item?id=41173335) supermarket prices with Playwright (467, 210, 2024-08; mostly off-topic);
[46377597](https://news.ycombinator.com/item?id=46377597) Vibium (443, 124, 2025-12);
[48445171](https://news.ycombinator.com/item?id=48445171) Intuned (117, 58, 2026-06);
[49756671](https://news.ycombinator.com/item?id=49756671) Stagehand "Playwright 2x faster" (155, 41, 2026-09); plus comment searches.

#### Likes (with quotes)

**Auto-waiting / actionability** (strongest single like)
- EugeneOZ, 2022-11-10: "Auto-waiting - it is a BRILLIANT feature!" — [33543274](https://news.ycombinator.com/item?id=33543274); also one-word answer "Auto-waiting." to "what does Playwright do that Selenium can't" — [33544522](https://news.ycombinator.com/item?id=33544522).
- simonw, 2022-01-27: auto-wait is designed in "from the very start" unlike Selenium — [30105154](https://news.ycombinator.com/item?id=30105154).
- vthommeret, 2022-01-27: lists actionability checks (attached, visible, stable, enabled) as reason #1 — [30106518](https://news.ycombinator.com/item?id=30106518).
- wickchuck, 2022-11-09: "you don't need to put manual waits in your program" — [33539710](https://news.ycombinator.com/item?id=33539710).
- hugs (Selenium creator), 2025-08-20: "playwright was a huge improvement there -- waiting until an element was actually ready" — [44964169](https://news.ycombinator.com/item?id=44964169).
- CharlieDigital, 2024-03-14: "good heuristics for auto-waiting" — [39705685](https://news.ycombinator.com/item?id=39705685).
- jonperl, 2021-03-12: Playwright is "a fork of Puppeteer that adds auto-waiting" — [26431715](https://news.ycombinator.com/item?id=26431715).
- frankthedog, 2023-03-28: misses "the auto waiting and other niceties" outside Playwright — [35349669](https://news.ycombinator.com/item?id=35349669).
- Vendor: arjun27 (Playwright team), 2020-08-19: "auto-waits for the right conditions on every action" — [24211893](https://news.ycombinator.com/item?id=24211893); fkilaiwi (Intuned), 2026-06-08 — [48446417](https://news.ycombinator.com/item?id=48446417).

**Plain promises / async-await instead of a command queue**
- lewisl9029, 2022-01-28: "Meanwhile Playwright's API is just promises." / "Everybody knows how to compose promises." — [30109346](https://news.ycombinator.com/item?id=30109346).
- lyu07282, 2022-11-09: Cypress "wasn't just using async/await syntax" — [33538530](https://news.ycombinator.com/item?id=33538530).
- btown, 2022-11-09: explains Cypress's enqueue model and the need for explicit yield points — [33539054](https://news.ycombinator.com/item?id=33539054).
- jonkoops, 2025-08-20: API "much more intuitive", "yes you must 'await' a lot" — [44966752](https://news.ycombinator.com/item?id=44966752).
- i_like_apis, 2022-01-27: await "makes the timeout of the action seem more explicitly declared" — [30105278](https://news.ycombinator.com/item?id=30105278).
- nimbix, 2022-01-28: switching from Cypress "allowed me to delete 3/4 of the helper code" — [30109013](https://news.ycombinator.com/item?id=30109013).

**API ergonomics vs Selenium / WebDriver**
- simonw, 2022-11-09: Selenium "API has NOT aged well"; "Playwright is just nicer to use." — [33538602](https://news.ycombinator.com/item?id=33538602); "API design is much more aligned with the kind of things I want" — [33539775](https://news.ycombinator.com/item?id=33539775).
- pjerem, 2021-06-10: "a cleaner and more idiomatic API" — [27459819](https://news.ycombinator.com/item?id=27459819).
- nkozyra, 2022-01-28: "I dislike passing around a driver to tests" (Selenium) — [30109675](https://news.ycombinator.com/item?id=30109675).
- hugs, 2025-12-24: Playwright "raised the bar on usability and developer experience" — [46378769](https://news.ycombinator.com/item?id=46378769).
- Congeec, 2019-07-23 (about Selenium, motivating Puppeteer): "I have to write these workarounds everywhere." — [20506211](https://news.ycombinator.com/item?id=20506211).
- danShumway, 2019-07-23: Selenium "makes it all needlessly difficult" (request interception, uploads) — [20507066](https://news.ycombinator.com/item?id=20507066).

**Trace viewer and debugging**
- vthommeret, 2022-01-27: "The tracing tools are incredible" — [30106518](https://news.ycombinator.com/item?id=30106518).
- benoau, 2025-08-20: "Playwright's "trace" viewer is also fantastic" — [44967585](https://news.ycombinator.com/item?id=44967585).
- arm32, 2025-08-20: "HTML test reports and trace viewing" — [44963301](https://news.ycombinator.com/item?id=44963301).
- wdb, 2021-06-10: considering moving everything to Playwright for the Trace Viewer — [27459286](https://news.ycombinator.com/item?id=27459286).
- Counterpoint: gregpr07 (browser-use), 2025-08-20: rrweb recordings "MUCH better than Playwright's replay traces" — [44963312](https://news.ycombinator.com/item?id=44963312).

**Multiple contexts, tabs, users and origins**
- cebert, 2022-11-09: two windows simulating two users editing, impossible in Cypress — [33539340](https://news.ycombinator.com/item?id=33539340).
- Karupan, 2022-01-27: "context switching made it a breeze" across two apps and six users — [30107370](https://news.ycombinator.com/item?id=30107370).
- mirekrusin, 2022-01-28: Cypress "can work only on single open window/tab" — [30111577](https://news.ycombinator.com/item?id=30111577).

**Official multi-language bindings**
- forgotmyoldacc, 2022-01-27: "official support for languages outside of JavaScript" — [30106763](https://news.ycombinator.com/item?id=30106763).
- nopenopenopeno, 2022-01-28: "Playwright Python is excellent" — [30111579](https://news.ycombinator.com/item?id=30111579).
- ithrow, 2021-06-10: no need for "their own ad-hoc http api in nodejs" — [27461720](https://news.ycombinator.com/item?id=27461720).
- 999900000999, 2025-12-24: "Playwright's main advantage is it has official support for multiple languages." — [46378403](https://news.ycombinator.com/item?id=46378403).
- LordKeren, 2023-10-19: Python Selenium → Playwright "such a significant improvement" — [37948226](https://news.ycombinator.com/item?id=37948226).
- JackC, 2020-12-03: pytest-playwright makes Django browser tests a few lines — [25290595](https://news.ycombinator.com/item?id=25290595).
- nikisweeting (browser-use), 2025-08-20: chose Playwright only because "pyppeteer was abandoned" — [44964394](https://news.ycombinator.com/item?id=44964394).

**Setup speed**
- 999900000999, 2025-12-24: "within 30 seconds Playwright will download it's needed browsers along with a test runner" — [46378927](https://news.ycombinator.com/item?id=46378927).
- XCSme, 2019-07-23: "I just did `npm i puppeteer` and it worked as expected." — [20506125](https://news.ycombinator.com/item?id=20506125).
- wickchuck, 2022-11-09: no more Selenium driver/browser version syncing — [33539710](https://news.ycombinator.com/item?id=33539710).

**Locators that read like the user's view**
- CharlieDigital, 2022-11-10: "Playwright's selectors are very resilient." — [33544379](https://news.ycombinator.com/item?id=33544379).
- epolanski, 2022-11-10: "You can query by text, label, html semantics, position." — [33554766](https://news.ycombinator.com/item?id=33554766).
- seanwilson, 2025-01-06: role/text/label targeting is robust to presentation changes — [42616168](https://news.ycombinator.com/item?id=42616168).
- tmerse, 2023-10-19: locators "pretty powerful", including layout-based — [37950365](https://news.ycombinator.com/item?id=37950365).
- mitemte, 2022-06-12: prefers Playwright's locator abstractions to Cypress aliases — [31713222](https://news.ycombinator.com/item?id=31713222).

**Puppeteer-specific likes**
- nikisweeting, 2025-08-21: Puppeteer "alive and well and arguably moving faster than Playwright" (lockstep with DevTools) — [44968568](https://news.ycombinator.com/item?id=44968568).
- tbrockman, 2025-08-21: worker profiling via CDPSession was "pretty painless in Puppeteer" — [44969198](https://news.ycombinator.com/item?id=44969198).
- throwthrow564, 2022-01-28: Puppeteer wins on "not shipping patched binaries" — [30119396](https://news.ycombinator.com/item?id=30119396).
- nkozyra, 2018-06-05: Puppeteer "an absolute joy to use" — [17233899](https://news.ycombinator.com/item?id=17233899).
- devjab, 2024-08-08: Playwright "only really gets better than puppeteer" for testing your own site — [41190970](https://news.ycombinator.com/item?id=41190970).

#### Complaints (with quotes)

**Auto-wait is not enough; timing is still the main failure source**
- benoau, 2026-02-10: "Playwright improves a lot on this over Puppeteer but it's imperfect" — combines selector, load-state and function waits — [46968103](https://news.ycombinator.com/item?id=46968103).
- testl33t, 2025-07-20: auto-wait is "great, until they don't work the way you want them to"; prefers WebDriver's explicit waits — [44628233](https://news.ycombinator.com/item?id=44628233).
- Klaster_1, 2025-01-09: "Most of my test failures come down to timing issues" (CPU load) — [42642694](https://news.ycombinator.com/item?id=42642694).
- worldsayshi, 2021-06-10: "very surprised to get almost the exact same issues with Playwright" (React app) — [27459326](https://news.ycombinator.com/item?id=27459326).
- steveklabnik, 2025-08-20: "I myself am fighting with Playwright these days" — [44964313](https://news.ycombinator.com/item?id=44964313).
- gmokki, 2022-01-27 (mechanism): "your ui framework actually replaces the element in the dom" — stale references — [30107206](https://news.ycombinator.com/item?id=30107206).

**Patched browsers, version pinning, not real Safari**
- hashseed (manager of Google's Puppeteer team), 2022-01-28: "This differs from the Playwright team's approach of shipping patched browser binaries." — [30111822](https://news.ycombinator.com/item?id=30111822).
- extra88, 2022-01-30: each Playwright version needs specific browser builds; WebKit builds never equal Safari — [30140630](https://news.ycombinator.com/item?id=30140630).
- natorion, 2024-08-14: "Playwright does not work with a "normal" Safari for example." — [41247342](https://news.ycombinator.com/item?id=41247342).
- twic, 2024-08-08: "for Firefox, i apparently need a custom patched build" — [41192146](https://news.ycombinator.com/item?id=41192146).
- Contradiction: imiric, 2022-11-10, says Playwright uses official builds, not custom ones — [33540044](https://news.ycombinator.com/item?id=33540044). Chromium is stock; Firefox and WebKit are patched (other comments above). Treat the "patched" claim as true for Firefox/WebKit.

**Browser downloads and environment friction**
- Bilal_io, 2022-01-27: left Playwright (.NET) possibly because "it had to redownload chromium everytime we deployed"; the author hedges the reason — [30105364](https://news.ycombinator.com/item?id=30105364).
- mceachen, 2019-07-23: Puppeteer CI runs occasionally "fail deep in chromium" (<5%) — [20507797](https://news.ycombinator.com/item?id=20507797).
- jamesgeck0, 2019-07-23: skipping the bundled download requires a compatible Chrome version; Chrome start/quit hangs in CI — [20508671](https://news.ycombinator.com/item?id=20508671).
- oefrha, 2025-05-10: Chrome via Playwright on macOS left 1 GB code-sign clones per unclean exit; "it ate up ~50GB in two days" — [43944642](https://news.ycombinator.com/item?id=43944642).
- starik36, 2022-11-09: Playwright .NET did not support ARM (Raspberry Pi) then; PuppeteerSharp did — [33538682](https://news.ycombinator.com/item?id=33538682) (dated).
- felixhummel, 2022-11-10: TLS client certificates "still not possible", kept Selenium — [33543589](https://news.ycombinator.com/item?id=33543589) (dated; later Playwright releases added client certificates, **unverified here**).

**Abstraction hides the protocol; Node relay; not built for agents/scrapers**
- gregpr07 (browser-use), 2025-08-20: can't contribute because "it runs inside a JS subprocess"; CDP APIs for cross-origin iframes not exposed — [44964322](https://news.ycombinator.com/item?id=44964322).
- nikisweeting (browser-use), 2025-08-20: issues closed as "not part of the core "QA testing" use-case" — [44964448](https://news.ycombinator.com/item?id=44964448).
- keepamovin (BrowserBox), 2025-08-21: Puppeteer/Playwright abstractions "insufficiently expressive or flexible"; uses raw CDP — [44970320](https://news.ycombinator.com/item?id=44970320).
- dataviz1000, 2025-08-20: "The most difficult part is managing the lifecycle of Windows, Pages, and Frames" — [44963425](https://news.ycombinator.com/item?id=44963425).
- tbrockman, 2025-08-21: worker profiling through Playwright's CDPSession wrapper "unnecessarily difficult" — [44969198](https://news.ycombinator.com/item?id=44969198).

**`evaluate` serialization (Puppeteer)**
- brod, 2018-06-05: "the biggest annoyance with `page.evaluate` is transpiling javascript" (helpers outside the closure are not serialized) — [17235701](https://news.ycombinator.com/item?id=17235701).

**Codegen / recorder has limited value**
- giobox, 2022-11-09: "none of the suggested code is production ready" — [33540009](https://news.ycombinator.com/item?id=33540009).
- baal80spam, 2024-03-14: "I find Codegen to be OK-ish for only the simplest/worthless test paths." — [39705567](https://news.ycombinator.com/item?id=39705567); also 2022 — [33538387](https://news.ycombinator.com/item?id=33538387).
- 999900000999, 2022-11-09: "stop thinking auto generated code is worthwhile" — [33538915](https://news.ycombinator.com/item?id=33538915).
- space_ghost, 2024-03-14 and willio58, 2024-03-14: SDETs don't use the generator; the locator picker is the useful part — [39705698](https://news.ycombinator.com/item?id=39705698), [39706802](https://news.ycombinator.com/item?id=39706802).
- Positive nuance: fwlr, 2024-03-14: "about 30% of the generated code informs how the tests are written" — [39709114](https://news.ycombinator.com/item?id=39709114).

**Selector API learning curve**
- nkozyra, 2022-01-28: "I haven't fully made peace with the selector API in playwright" (custom DSL) — [30109675](https://news.ycombinator.com/item?id=30109675).
- ilyazub (SerpApi blog), 2023-11-14: "Playwright's getByRole is 1.5x slower than CSS selectors" — [38268753](https://news.ycombinator.com/item?id=38268753).
- robgibbons, 2022-11-10: "If you're selecting elements by their content the tests are much more brittle." — [33555476](https://news.ycombinator.com/item?id=33555476).

**Runner preferences**
- dvngnt_, 2022-11-09: "i miss the cypress test runner" — [33538440](https://news.ycombinator.com/item?id=33538440).
- Positive about separation: Karupan, 2022-01-27: "a nice separation of browser automation and test runner API" — [30107370](https://news.ycombinator.com/item?id=30107370); ncrmro, 2021-04-06: fixtures "pass POM, db connection, JWT token getter directly into your test" — [26716602](https://news.ycombinator.com/item?id=26716602).

**Detection and memory (scraping)**
- asar, 2025-01-09: "They can detect a playwright instance by how it interacts with the DOM." — [42645490](https://news.ycombinator.com/item?id=42645490).
- oefrha, 2022-11-10: "I've seen sites defeating puppeteer-extra-plugin-stealth before." — [33542446](https://news.ycombinator.com/item?id=33542446).
- Jommi, 2025-07-18 (counter): "Playwright in non headless mode is nearly impossible to detect." — [44602134](https://news.ycombinator.com/item?id=44602134).
- dschnurr, 2018-06-05: launching/closing Puppeteer browsers "does seem to have some sort of memory leak" — [17235519](https://news.ycombinator.com/item?id=17235519); squeaky-clean: zombie processes, kill containers per job — [17234250](https://news.ycombinator.com/item?id=17234250).

**Puppeteer-specific complaints**
- kjkjadksj, 2021-10-11: "Their click function just did nothing at all." — [28830482](https://news.ycombinator.com/item?id=28830482).
- simlevesque, 2020-08-19 (early Playwright vs Puppeteer): "many features were lacking or less polished" — [24209666](https://news.ycombinator.com/item?id=24209666).
- creesch, 2024-08-08: impression "puppeteer was pretty much abandoned" — [41188255](https://news.ycombinator.com/item?id=41188255) (contradicted by [44968568](https://news.ycombinator.com/item?id=44968568)).
- mikeshi42, 2022-02-16: Chrome Recorder output for Puppeteer adds "a _ton!_ of boilerplate" — [30366723](https://news.ycombinator.com/item?id=30366723).

**Governance**
- meitham, 2022-11-10: community projects "more trustworthy and likely to last longer" than Microsoft-controlled — [33543011](https://news.ycombinator.com/item?id=33543011); solarkraft and exyi echo Azure-lock-in fears — [33544457](https://news.ycombinator.com/item?id=33544457), [33544930](https://news.ycombinator.com/item?id=33544930).

---

### Reddit findings

**Status: blocked.** No Reddit thread could be opened (see Method above). What exists is second-hand and is listed under Unverified below as **unverified**:
- aqua cloud (vendor, test-management, page dated 2026-02-27) quotes r/QualityAssurance thread `126yach`: Derpcock "Playwright doesn't seem to automatically wait for actionability" and "you have to await every async call"; arakinas "Our Java tests were pretty flaky with Playwright."; d0rf47 ".NET implementation" issues and "async click event not actually waiting"; ProfCrumpets "The async chainable design is weird to get around" (Cypress); Fissherin "I always hated UI automation frameworks, until I met Playwright." Source: [aqua-cloud.io/playwright-review-what-users-say](https://aqua-cloud.io/playwright-review-what-users-say/).
- DoesQA (vendor selling a Playwright alternative) lists Reddit post titles such as "Is it just me or is Playwright maddeningly flaky at times?" and "Playwright tests are solid locally but flaky in CI" via search links only — [does.qa](https://does.qa/blog/playwright-is-flaky-but-dont-take-our-word-for-it).
- A search-engine summary attributed to r/webscraping user hasdata_com (handle of a scraping-API vendor): Puppeteer for large-scale scraping, Playwright for complex flows.

Substitute community forums (verified, read directly):

**Ministry of Testing Club** (QA practitioners' Discourse)
- [What problems/limitations have you faced with Playwright?](https://club.ministryoftesting.com/t/69895) (2023-08): utchbe "Auto waiting for Playwright can be helpful but it doesn't solve all your problems"; default report "limiting" but trace viewer "really really nice"; lufis "It seems pw doesn't support global hooks"; hitchdev headed-in-container over VNC "a bit flaky"; clittlefair "Reluctantly went to Playwright and was sold from Day-1".
- [Locator strategy: built-in locators or IDs?](https://club.ministryoftesting.com/t/83373) (2025-03): hananurrehman prefers IDs; role locators are hard to find "in the inspect tab of a browser"; daluu: tool-specific locators reduce portability across tools.
- [Playwright vs Cypress](https://club.ministryoftesting.com/t/85581) (2025-06): fullsnacktester contrasts Cypress `data-cy` with Playwright's accessibility-first locators; kristof notes a "higher learning curve".
- [The Future of Playwright](https://club.ministryoftesting.com/t/86980) (2025-12): concern after Microsoft layoffs; ujjwal.singh "companies have doubts regarding the playwright being open-sourced in the future".

**Python async friction** (forums other than SO)
- Streamlit forum, [Using Playwright with Streamlit?](https://discuss.streamlit.io/t/28380) (2022–2024, 5,234 views): sync API fails inside Streamlit; Windows `NotImplementedError` needs a Proactor loop; deploying needs `playwright install` plus ~20 OS packages; goldengrape: "In the end, I used Selenium to achieve my program's goals."
- Streamlit forum, [NotImplementedError (asyncio, playwright)](https://discuss.streamlit.io/t/66756) (2024, 2,627 views): gabrieldagostim "I managed to get around the error by using the Selenium library".
- CrewAI community, [should I use sync or async version?](https://community.crewai.com/t/2163) (2024-12, 970 views): sync API raises "Sync API inside the asyncio loop"; async version returns an un-awaited coroutine; workaround `nest_asyncio`.
- discuss.python.org, [Two sync_apis (playwright and procrastinate)](https://discuss.python.org/t/81521) (2025-02, 682 views): sync Django app breaks because Playwright's sync API runs its own loop; voidus "I've been struggling with this situation for a while".

---

### Migration and critical posts

| Post | Type | Direction | Why moved | What they missed / caveats | Numbers |
|---|---|---|---|---|---|
| WordPress/Gutenberg, Bart Kalisz, [Proposal: Migrate e2e to Playwright](https://make.wordpress.org/test/2021/10/04/proposal-migrate-e2e-to-playwright/) (2021-10-04); [core uses Playwright for all browser tests](https://make.wordpress.org/core/2023/10/16/wordpress-core-is-now-using-playwright-for-all-browser-based-tests/) (2023-10-16) | Independent OSS | Puppeteer → Playwright | Puppeteer tests needed explicit `waitForSelector` before actions; auto-wait and stability checks; "easier to write stable tests"; near-identical API; refactors removed more lines than added | Reviewers: missing elements are not the only flake source; running with animations is slower (≈37 s on 21 suites); auto-wait might hide bugs; breaking change for third-party users of WP e2e utils | 21 suites refactored in pilot |
| Nutrient (Ritesh Kumar), [Migrating from Puppeteer to Playwright](https://nutrient.io/blog/migrating-from-puppeteer-to-playwright) (2023-01-13) | Company, own test suite | Puppeteer → Playwright | ElementHandle limited to CSS and needs manual visibility waits; locators auto-wait and retry; trace viewer; runner close to Jest | Ran both stacks side by side during migration | "Removed all flaky tests"; no counts |
| BigBinary (S Varun), [Why we switched from Cypress to Playwright](https://bigbinary.com/blog/why-we-switched-from-cypress-to-playwright) (2024-09-18) | Consultancy, own products | Cypress → Playwright | Cypress 13 blocked third-party reporters; paid parallelism; chained-command retry flakiness; closures/aliases for variables; single tab; OOM crashes | No Playwright drawbacks mentioned | Suite 2 h 27 min → 16 min; auth flow ~2 min → <20 s; weekly text-change test updates 22 → 2; reporting cost $405 → $90/month |
| NocoDB (Muhammed Mustafa), [Moving to Playwright from Cypress, part 1](https://nocodb.com/blog/moving-to-playwright-from-cypress-part-1) (2022-11-29) | Company, OSS product | Cypress → Playwright | CI flakiness; no async/await ("callback hell of nodejs"); paid parallelism; DOM re-render errors | Codegen less useful than expected; IDE debug setup manual; admits comparison not fair (also restructured tests) | CI 196 min on 15 instances → 70 min on 6; flakiness "close to zero" |
| PayPay Card (Taishun Kusaka), [Modernizing E2E testing](https://blog.paypay.ne.jp/en/modernising-e2e-testing-by-migrating-from-cypress-to-playwright/) (2026-04-07) | Company | Cypress → Playwright | Long runs ("bottleneck phase"), flaky false positives | Needed retries=5 and 2-min timeouts; custom BDD layer on `test.step` | Web 30 → 11 min; Miniapp 44 → 18 min; false positives 6.29% → 4.32% and 8.22% → 4.47% |
| Browser Use (Nick Sweeting), [Closer to the Metal: Leaving Playwright for CDP](https://browser-use.com/posts/playwright-to-cdp) (2025-08-20) | Company (AI agent product) | Playwright → raw CDP | Node.js relay adds a hop and latency for thousands of CDP calls per step; state drift and hangs between browser, Node and Python; `Page`/`BrowserContext` hide targets, frames, sessions (cross-origin iframes); about half of 10+ tab-crash modes handled; edge cases (16k px full-page screenshots, dialogs, remote uploads/downloads) | Gave up typed RPC, cross-language consistency, cross-browser; now owns crash handling | No benchmark figures |
| Vitaliy Potapov, [Playwright in pictures: how fixtures work](https://dev.to/vitalets/playwright-in-pictures-how-fixtures-work-3oe3) | Independent (author of playwright-bdd) | — | — | Fixtures "make test execution less explicit"; code runs without being named in the test | — |
| Simon Willison, [Bundling binary tools in Python wheels](https://simonwillison.net/2022/May/23/bundling-binary-tools-in-python-wheels/) (2022-05-23) | Independent | — | — | Playwright Python is a layer over a bundled Node process: "That's a full copy of the Node.js binary!" (~77 MB) | 77 MB Node binary |
| Zhimin Zhan, [annotations on BigBinary](https://agileway.substack.com/p/annotations-on-the-article-why-we) (2025-01-13) and "Correcting wrong Playwright's advantage" series | Opinionated; sells Selenium-oriented books/services | Pro-Selenium | Argues built-in runner is a con (paywalled, **unverified**) | — | — |

Vendor and SEO content seen but not used as evidence: aqua cloud, DoesQA, Testleaf, Testdino, RoyalCyber, BrowserStack, Currents, QASkills, Autonoma, Scrapfly, Apify, BrowserCat, Skyvern, Firecrawl, Browserless comparison posts. Their numeric claims (Testleaf "20% → <2% flaky", Testdino "Zenjob 35 → 7 min") are anonymous or unsourced, so they are in the Unverified list below.

---

### Likes, ranked

Ranked by independent reports (distinct authors, non-vendor), then by migration-post corroboration.

| Rank | Like | Independent reports | Vendor/tool-author reports | Key evidence | Lesson for Goodfetch |
|---|---|---|---|---|---|
| 1 | **Auto-waiting / actionability checks before actions** 8 distinct authors (EugeneOZ ×2, simonw, vthommeret, wickchuck, hugs ×2, CharlieDigital, jonperl, frankthedog) + WordPress, Nutrient, BigBinary, NocoDB posts; 2 more report low flakiness without naming auto-wait (irjustin [41188806](https://news.ycombinator.com/item?id=41188806), lbriner [27459402](https://news.ycombinator.com/item?id=27459402)) | arjun27, fkilaiwi | Hacker News findings; WordPress, Nutrient | Make actionability waiting the default for every action on a Locator, with per-action timeouts and checks listed in errors. Keep it, but see pain #1: it is not sufficient alone. |
| 2 | **Plain promises/async-await and normal control flow** (vs Cypress queue, vs Selenium explicit waits) | 7 (lewisl9029, lyu07282, btown, jonkoops, i_like_apis, nimbix, mitemte) + NocoDB, BigBinary | — | [30109346](https://news.ycombinator.com/item?id=30109346) | Each SDK uses its language's native concurrency (TS promises, Python async *and* sync, Rust futures); no hidden command queue; results are values you can branch on. |
| 3 | **Trace viewer / debugging artifacts** | 6 (vthommeret, benoau, arm32, wdb, pjerem, MoT utchbe) + NocoDB, Nutrient, WordPress, PayPay | — | [30106518](https://news.ycombinator.com/item?id=30106518) | Engine-level trace recording (actions, DOM snapshots, network, console) available to every SDK and the crawler, not only a test runner. One counter (rrweb better). |
| 4 | **Official bindings in several languages with one API** | 8 (forgotmyoldacc, nopenopenopeno, ithrow, niklasjansson, 999900000999, LordKeren, JackC, nikisweeting) | — | [30106763](https://news.ycombinator.com/item?id=30106763) | Directly validates Goodfetch's TS/Python/Rust-over-one-engine plan; keep the API shape identical across SDKs. |
| 5 | **Isolated contexts, multiple tabs/users/origins** | 5 (cebert, Karupan, mirekrusin, MoT lufis, BigBinary/NocoDB authors) | — | [33539340](https://news.ycombinator.com/item?id=33539340) | Cheap isolated contexts; popups and new pages as first-class events; cross-origin frames handled transparently. |
| 6 | **Cleaner, smaller API than Selenium** | 7 (simonw ×2, pjerem, nkozyra, hugs, Congeec, danShumway) | — | [33538602](https://news.ycombinator.com/item?id=33538602) | No driver object threaded through calls; page-centric verbs; request interception and file upload as plain API calls. |
| 7 | **Locators that target user-visible role/text/label** | 6 (CharlieDigital, epolanski, seanwilson, tmerse, mitemte, MoT fullsnacktester) | — | [33554766](https://news.ycombinator.com/item?id=33554766) | Offer role/text/label locators *and* plain CSS/XPath/test-id with equal standing (see pain #10). |
| 8 | **Fast setup (one install fetches matching browsers)** | 4 (999900000999, XCSme, wickchuck, EugeneOZ) | — | [46378927](https://news.ycombinator.com/item?id=46378927) | Keep a one-command setup, but make it explicit and inspectable (pain #5). |
| 9 | **Built-in runner: parallelism, fixtures, retries, reports** | 4 (ncrmro, holler, jonkoops, MoT utchbe) + BigBinary, NocoDB, PayPay | — | [26716602](https://news.ycombinator.com/item?id=26716602) | Valuable, but users also hit runner coupling (pain #8). |
| 10 | **Network interception and API testing in the same tool** | 4 (EugeneOZ, danShumway, kbumsik, MoT utchbe) | — | [33543274](https://news.ycombinator.com/item?id=33543274) | Routing, response capture and an HTTP client sharing the context's cookies are core, not plugins. |
| 11 | **Puppeteer: stock browsers, lockstep with CDP, easy raw CDP sessions** | 4 (throwthrow564, nikisweeting, tbrockman, XCSme) | hashseed (Google) | [44969198](https://news.ycombinator.com/item?id=44969198) | Drive stock Chrome/Chromium; expose a raw protocol session per target. |

---

### Pain points, ranked (with API-design lesson)

Ranking weighs SO question counts (Stack Overflow themes above), independent forum reports, and migration-post mentions.

#### P1. Synchronization is still the top problem, even with auto-wait
- **Evidence count:** SO 53 questions / 1,686 votes in the synchronization family; title "wait" 77 (PW) and 159 (Pptr); 7 independent HN reports (benoau, testl33t, Klaster_1, worldsayshi, steveklabnik, naasking, gmokki); MoT utchbe; second-hand Reddit (Derpcock, d0rf47).
- **Shapes:** wait for page "completely loaded" ([52497252](https://stackoverflow.com/q/52497252), 143, 2018), network idle after click ([74586859](https://stackoverflow.com/q/74586859), 16, 2022), no animation ([71937343](https://stackoverflow.com/q/71937343), 40, 2022), element to disappear ([58833640](https://stackoverflow.com/q/58833640), 32, 2019), all spinners gone ([76622008](https://stackoverflow.com/q/76622008), 7, 2023), timeout seemingly ignored ([71031986](https://stackoverflow.com/q/71031986), 11, 2022), flaky click ([74301319](https://stackoverflow.com/q/74301319), 17, 2022).
- **Lesson:** Auto-wait covers "element ready", not "app ready". Provide composable, first-class waits for app state (URL, response, request finished, element count/state, custom predicate) that share one timeout model; include "what we were waiting for and the last observed state" in every timeout error.

#### P2. Navigation races and lifecycle errors (Puppeteer especially)
- **Evidence count:** SO 9 questions / 244 votes plus [46948489](https://stackoverflow.com/q/46948489) (131, 2017) and waitForNavigation hangs ([49422819](https://stackoverflow.com/q/49422819), 16; [52211871](https://stackoverflow.com/q/52211871), 9); full-text "Execution context was destroyed" 53 (Pptr); "Target closed" 62 (PW) / title 29 (Pptr); HN dataviz1000, nikisweeting (crash modes), browser-use post.
- **Lesson:** Never require users to start a wait *before* the action that triggers it (Puppeteer's `Promise.all([waitForNavigation(), click()])` idiom). Actions that may navigate should optionally return or await the navigation they caused; evaluation should survive or clearly report navigation; typed errors distinguish "target crashed", "page closed by user code", "navigated away".

#### P3. Getting data out: text/attributes/values and the `evaluate` serialization boundary
- **Evidence count:** SO 53 questions / 1,472 votes (themes 5, 8, 10); [46088351](https://stackoverflow.com/q/46088351) is the second-highest-voted Puppeteer question (293); title "evaluate" 91 + "$eval" 69 (Pptr); HN brod; empty objects from `querySelectorAll` ([46377955](https://stackoverflow.com/q/46377955), 41); outer variables in `addInitScript` ([78903490](https://stackoverflow.com/q/78903490), 2024).
- **Lesson:** Typed getters on Locator for the common reads (text, inner text, value, attribute, HTML, checked, bounding box, count, all-texts). For `evaluate`, take an explicit argument parameter, document the serialization contract per SDK (the page script is always JavaScript source, even from Python/Rust), and fail loudly on non-serializable returns or references to closure variables.

#### P4. Environment: installing, finding and launching browsers
- **Evidence count:** SO 34 questions / 734 votes (theme 2); titles "docker" 74/148, "install" 42/68, "Failed to launch" 69 (Pptr); full-text "Executable doesn't exist" 34 (PW); HN Bilal_io, mceachen, jamesgeck0, oefrha, starik36; Streamlit forum (two users switched to Selenium).
- **Lesson:** Explicit, inspectable browser provisioning (a command and an API that say which browser, where, which version); prefer system or branded browsers when present; a "doctor" check that names missing OS libraries; never download at first launch in production; errors state the exact path searched and the fix.

#### P5. Patched browsers and version lock-step
- **Evidence count:** 5 independent HN reports (hashseed, throwthrow564, extra88, natorion, twic); SO [62184117](https://stackoverflow.com/q/62184117) Safari vs WebKit (35); [69119268](https://stackoverflow.com/q/69119268) roll to specific browser version; [62281859](https://stackoverflow.com/q/62281859) use installed Chrome (29).
- **Lesson:** Target stock browsers over standard protocols (CDP now, WebDriver BiDi where it suffices); state honestly which engines are "real" browsers; decouple engine release from exact browser builds where the protocol allows.

#### P6. Python: sync vs async, event loops, and the Node driver
- **Evidence count:** SO 10 questions in cluster plus 5 targeted error questions (low votes, rising after 2024); forums: Streamlit ×2 (7,861 views combined), CrewAI, discuss.python.org; HN gregpr07, nikisweeting (Node relay hangs), Simon Willison (77 MB Node binary).
- **Lesson:** Goodfetch's Rust engine removes the Node dependency, a real differentiator. The Python SDK should offer async and sync APIs where the sync API works when an event loop is already running (Jupyter, Streamlit, Django async views, agent frameworks), or fails with a specific fix; no greenlet tricks; work on Windows without loop-policy changes.

#### P7. Element existence, visibility and "assert absent"
- **Evidence count:** SO 13 questions / 361 votes; [64784781](https://stackoverflow.com/q/64784781) and [71183099](https://stackoverflow.com/q/71183099) are the two highest-voted Playwright-tagged questions (58 each); [76029417](https://stackoverflow.com/q/76029417) `is_visible()` always false.
- **Lesson:** Separate clearly: immediate non-waiting queries (`count`, `exists`, `isVisible`) that never throw, and waiting expectations (`toBeVisible`, `toBeHidden`, `toHaveCount`) with retries. Name them so the waiting behavior is obvious, and make them available outside any runner.

#### P8. Test-runner coupling
- **Evidence count:** 58 of 276 Playwright SO questions (21%) are runner/config questions; runner-only failures ([79319117](https://stackoverflow.com/q/79319117), [76040003](https://stackoverflow.com/q/76040003), [78016704](https://stackoverflow.com/q/78016704), [74135518](https://stackoverflow.com/q/74135518)); Puppeteer users hit Jest's 5 s timeout ([49603939](https://stackoverflow.com/q/49603939), 506 votes, the single highest-voted question in the sample); HN Karupan (likes the separation), dvngnt_; MoT lufis (global hooks); vitalets (implicit fixtures); nikisweeting (issues outside "QA testing" rejected).
- **Lesson:** Keep the page API fully usable from any runner and from plain scripts, crawlers and agents. Assertions with retry, tracing, isolation and auth reuse should be library features; a runner integration is optional sugar. Defaults (timeouts) must not depend on the host runner.

#### P9. Locator/handle model confusion: ElementHandle vs Locator, strictness
- **Evidence count:** SO strict-mode questions (title 5, full-text 19; [76043713](https://stackoverflow.com/q/76043713), [75777710](https://stackoverflow.com/q/75777710), [77124207](https://stackoverflow.com/q/77124207)); ElementHandle questions ([65400809](https://stackoverflow.com/q/65400809), [55664420](https://stackoverflow.com/q/55664420), [53072849](https://stackoverflow.com/q/53072849)); collections/iteration ([61453673](https://stackoverflow.com/q/61453673), 34; [70138360](https://stackoverflow.com/q/70138360), 18); Nutrient post (ElementHandle CSS-only, manual waits); gmokki (stale elements).
- **Lesson:** One lazy, re-resolving handle type; strict by default with explicit `first`/`nth`/`all`; strict-mode errors list the matched elements and suggest the disambiguating locator; iteration API that snapshots a list safely.

#### P10. Locator strategy debates and selector API learning curve
- **Evidence count:** HN nkozyra, robgibbons, ilyazub (perf); MoT hananurrehman, daluu (portability); SO [75151754](https://stackoverflow.com/q/75151754) "select by Id" (41, 2023) and [68641789](https://stackoverflow.com/q/68641789) by.name.
- **Lesson:** Do not force a philosophy. Make CSS/id/test-id as first-class as role/text; show the resolved DOM node in errors and traces so role locators are debuggable in DevTools; document performance trade-offs.

#### P11. Codegen / recorder output not production-grade
- **Evidence count:** 7 independent HN reports (giobox, baal80spam ×2, 999900000999, MetaWhirledPeas, space_ghost, willio58) + NocoDB post; partial positives (fwlr, px1999, abdusco).
- **Lesson:** Lower priority. The locator picker is valued more than whole-script generation.

#### P12. Long-running scraping: memory, zombies, hangs
- **Evidence count:** SO perf cluster 4 Puppeteer questions; title "memory" 44 (Pptr) / 4 (PW); HN dschnurr, squeaky-clean, oefrha (evaluate hang leaves Chrome copies), bluepnume; Apify Discord reports (**unverified**).
- **Lesson:** The crawler should recycle contexts and browsers by policy, guarantee teardown on cancellation and crash, expose per-context resource metrics, and time out `evaluate` calls.

#### P13. Abstraction hides the protocol; no escape hatch for advanced or agent use
- **Evidence count:** 5 independent reports (gregpr07, nikisweeting, keepamovin, tbrockman, dataviz1000) + browser-use post.
- **Lesson:** Expose a raw protocol session per target/frame/worker from every SDK, with events; keep frame/target identity visible when needed; avoid an extra relay process (Goodfetch's single Rust engine already avoids the Node hop).

#### P14. Bot detection
- **Evidence count:** SO 9 questions / 178 votes; HN asar, oefrha, tzs, OldfieldFund, Intuned (custom Chromium fork); PyPI `patchright` ~966k/week, `undetected-chromedriver` ~538k/week, `nodriver` ~76k/week.
- **Lesson:** For a crawler, avoid adding automation signals the API does not need, and support attaching to a real user profile or stock browser. (Policy decisions on evasion are out of scope here.)

#### P15. Fixed sleeps removed or discouraged
- **Evidence count:** SO 4 questions / 223 votes ([46919013](https://stackoverflow.com/q/46919013), 166); Puppeteer `waitForTimeout` removal confusion ([64805395](https://stackoverflow.com/q/64805395), 19; repeat in 2024).
- **Lesson:** Keep a plainly named sleep for scripts and debugging (discouraged in docs), and never remove an API without a pointer to its replacement in the error.

#### P16. Governance and longevity
- **Evidence count:** HN meitham, solarkraft, exyi; MoT "Future of Playwright" thread (2025-12).
- **Lesson:** Not API design, but adoption: neutral governance and stable published versions matter to buyers.

---

### Unverified

- All Reddit statements (blocked): aqua cloud's quotes from r/QualityAssurance (Derpcock, arakinas, d0rf47, ProfCrumpets, Fissherin, Ultimas134, mourasman; thread `126yach`); DoesQA's Reddit titles; the hasdata_com r/webscraping remark from a search summary.
- Apify Discord reports of Playwright memory rising ~400% to ~800 MB and stalling after ~2,000 URLs (pages returned HTTP 429; known only from a search summary).
- Habr comment that paused Playwright promises can't be inspected (search summary only).
- Testleaf "20% → under 2% flaky after migrating 500+ tests" (anonymous company); Testdino "Zenjob 35 → 7 min" (no primary source).
- Zhimin Zhan's claim that Playwright's built-in runner is a con (paywalled).
- Whether Playwright later added TLS client certificates (felixhummel's 2022 complaint).
- Bilal_io's reason for leaving Playwright .NET (the author hedges it).
- SO tag sizes from the API (`playwright` 3,501 looks low).

---

### Package download context (dated)

npm, week 2026-10-06 to 2026-10-12 (`api.npmjs.org`):

| Package | Weekly downloads |
|---|---|
| playwright-core | 125,588,893 |
| playwright | 113,562,399 |
| @playwright/test | 76,227,910 |
| puppeteer-core | 23,205,163 |
| puppeteer | 11,547,242 |
| cypress | 5,949,802 |
| selenium-webdriver | 1,807,038 |

PyPI, `pypistats.org/api/packages/<pkg>/recent`, fetched 2026-10-14:

| Package | Last day | Last week | Last month |
|---|---|---|---|
| playwright | 4,370,010 | 26,386,194 | 97,001,677 |
| selenium | 1,139,535 | 6,884,385 | 28,029,613 |
| pytest-playwright | 440,415 | 2,564,889 | 9,353,269 |
| patchright | 168,254 | 966,036 | 3,797,565 |
| undetected-chromedriver | 84,890 | 538,416 | 1,972,753 |
| pyppeteer | 27,431 | 156,431 | 597,832 |
| nodriver | 11,940 | 75,911 | 287,266 |

Downloads include CI, mirrors and agent tooling installs, so they measure install volume, not users. `playwright-core` is a dependency of `playwright` and `@playwright/test`, so the npm rows overlap. Ratios that still hold: on npm `playwright` is about 10× `puppeteer`; on PyPI `playwright` is about 4× `selenium` and about 170× the abandoned `pyppeteer`.

