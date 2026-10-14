# Collie's page API keeps Playwright's names, strictness waits, values cross as JSON, and unanswered requests fall through

Collie's page API is Playwright's (ADR 0019) wherever the meaning is the same, with the guard's promises kept everywhere and Playwright's traps closed where its behaviour hurts users. Pages, Sessions and frames are values, and a locator is resolved again at each use. Uniqueness is one more condition the guard waits for. A page's gestures run one at a time. A page function takes one explicit argument, and values cross as JSON. Routes and dialog handlers answer requests under written rules, so nothing stalls. A raw CDP session is the way down. Leaving Playwright is where AI tools lose users: "partial shape plus missing guarantees is worse than a different shape" (AI pain 12, Stagehand's migration guide). Playwright's own traps are documented: strictness fails at once while every other condition is waited out ([#36045](https://github.com/microsoft/playwright/issues/36045), open). A non-serializable `evaluate` result silently becomes `undefined`. A route that settles nothing stalls its request. A `dialog` listener that does not answer stalls the action. Decided in [Collie's page API](https://github.com/goodfetch/automan/issues/472), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), every answer by the agent on the owner's delegation, on the evidence of the [research on competitors' APIs](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) ([browser drivers](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/browser-drivers.md) §2.1, §4 and P1, P4, P5, P6, P9, P12, P13; [AI browser automation](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) §6 rows 9 and 12), of Selenium's [ADR 17685](https://github.com/SeleniumHQ/selenium/blob/trunk/docs/decisions/17685-network-handler-behavior.md), and of Playwright 1.64.1's own documentation. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Pages, Sessions and frames

- **The Session is Playwright's context.** Playwright's `BrowserContext` holds cookies, storage, proxy and routes; the glossary's Session already is that unit, so there is no second type (S2). Pages open with `session.newPage()` or arrive as popups. How a Session is opened, and whether `run.newPage()` uses a default Session, are [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476)'s.
- **Tabs are values; there is no active page** (G2). A popup is a Page of its opener's Session (`popup` event); `session.pages()` lists open pages and the Session's `page` event announces new ones. `close()` closes a popup as any page Goodfetch opened, and on a Borrowed browser only releases it (ADR 0078). There is no `bringToFront()` or `opener()`: the Live view follows popups (ADR 0031), and area 5's spec keeps a driven page from being throttled as a background tab.
- **Frames are reached through frame locators only.** `frameLocator(selector?)` on a page, a locator or a frame locator, with Playwright's selector-less form that searches every frame, plus `locator.contentFrame()` and `frameLocator.owner()`. They cross same-origin and out-of-process frames alike (ADR 0019 tracks OOPIFs). There is no eager `Frame` handle and no frame pointer (S2, G2). Code that must run in a frame's own window goes through a locator inside it.
- **Shadow DOM:** CSS and `getBy*` pierce open shadow roots and XPath does not, as in Playwright (S3). Whether closed shadow roots are reached on Chromium is area 5's spec; when one hides a target, the deadline error's `observed` says so (G6).

### Locators

- **Playwright's constructors and matching rules** (S3): `locator(selector)`, `getByRole`, `getByText`, `getByLabel`, `getByPlaceholder`, `getByAltText`, `getByTitle`, `getByTestId`. Text matches as a case-insensitive substring after whitespace is normalized, or exactly with `exact`, because a different default under the same name is what models get wrong most (ADR 0077's RustEvo² figure). Regexes run in the engine under ADR 0081's rules.
- **`locator()` takes CSS, or XPath** when the string starts with `//`, `..` or `xpath=`, as Playwright detects it. Playwright's other selector engines (`text=`, `role=`, `internal:`) and `>>` chains fail with `selector_invalid`, whose fix names the method that replaces them: each has a method, and a string mini-language would be a second way (S2, G3). There is no `getById` (Playwright declined it, +81).
- **The test-id attribute is the setting `locator.test_id_attribute`**, `data-testid` by default, set at any scope (ADR 0028), never Playwright's process-wide `selectors.setTestIdAttribute` (G2).
- **Composition is declarative and resolved by the engine:** `filter({ hasText, hasNotText, has, hasNot, visible })`, `and`, `or`, `first()`, `last()`, `nth()`, `all()`, and constructors on a locator. No host callback ever runs in the page (ADR 0081).
- **No handles.** A locator is a lazy value, built without I/O and resolved again at each use. No `ElementHandle` or `JSHandle` exists; `locator.evaluate` hands its element to a page function, and raw CDP is below that (G8). Selenium's stale references and Puppeteer's destroyed contexts are the handle model's pains (browser drivers P5).

### Strictness

- **Uniqueness is one of the conditions the guard waits for.** Every action, and every read of one element, waits until exactly one element matches and the guard's other conditions hold, until its deadline (ADR 0080). If the last observation was ambiguous, it raises `target_not_unique` (a `PageError`, ADR 0079's case) listing every match rendered in the caller's language, with the timeout's setting; otherwise a missing target raises `DeadlineExceededError`.
- **A question that never waits** (`isVisible()`) raises `target_not_unique` at once. `first()`, `last()` and `nth()` are the visible opt-outs.
- Playwright fails at once on ambiguity while it waits out every other condition, so two copies during a re-render fail where a moving element would be waited for. A persistent ambiguity now costs the timeout a missing target already costs, and `timeout` shortens both. The promise of an action is unchanged; only when an unkept promise is reported changes, as the guard's replay already changes it (ADR 0019).

### Actions

- **Playwright's locator actions under their names** (S3): `click`, `dblclick`, `tap`, `hover`, `check`, `uncheck`, `setChecked`, `dragTo`, `fill`, `clear`, `press`, `pressSequentially`, `selectOption`, `selectText`, `setInputFiles`, `focus`, `blur`, `scrollIntoViewIfNeeded`.
- **Every action goes through the guard** (ADR 0019) and needs a unique target. Pointer actions also need it visible, stable, enabled and receiving the event, and a gesture that lands elsewhere is cancelled and replayed. Text actions need it visible and enabled, and `fill` and `clear` need it editable. Input is trusted browser input, so controlled inputs see real events (P52).
- **A target of the wrong kind fails at once** with `target_unsupported` (a `PageError`), naming the action, what it accepts and what it found: `fill` on a `<div>`, `selectOption` on anything but a `<select>`, `setInputFiles` on anything but a file input. An element's kind does not change while waiting.
- **`force` is ADR 0023's per-call switch:** it skips the actionability checks and the replay, keeps uniqueness, and marks the action unguarded in Provenance. It is the setting `action.force` given for one call; a run can set it for all its actions. Playwright's `force` has that meaning (S3).
- An action that starts a navigation waits for it to commit, as Playwright's do.
- **Input with no target** keeps Playwright's `page.keyboard`, `page.mouse` and `page.touchscreen` (a canvas, a map, a key for whatever has focus). Nothing can be guarded, so each call is marked unguarded.

### Gestures on one page

- **The engine runs a page's gestures one at a time, in arrival order.** A gesture is a call that sends input or changes what the page shows: every locator action, `keyboard`, `mouse` and `touchscreen` calls, `goto`, `reload`, `goBack`, `goForward`, `setContent` and `fileChooser.setFiles`. The next starts when the previous one has returned.
- **Reads, waits, `evaluate` and answers to routes and dialogs run concurrently**, so ADR 0080's waiters stay armed during a gesture and `Promise.all` over reads costs one round trip's latency. Pages of one Session do not wait for each other.
- A gesture's timeout counts from its call (ADR 0080), so time spent behind another counts. This is the engine's per-page input order, as a browser has one input stream, not a queue in the SDK (ADR 0081, K04). Overlapping gestures interleave keystrokes and pointer moves silently, as concurrent Playwright `fill`s race for focus (G1).

### Reads

- **Playwright's getters, each one round trip whatever the number of matches** (P28; headless_chrome's per-element round trips made `find_elements` 150 times slower than JavaScript): `textContent`, `innerText`, `innerHTML`, `inputValue`, `getAttribute`, `boundingBox`, `isEnabled`, `isDisabled`, `isChecked`, `isEditable`, `isVisible`, `isHidden`, `count`, `all`, `allTextContents`, `allInnerTexts`.
- Reads of one element's content, and states that need one element, wait for a unique match under the action timeout, as Playwright's do. `isVisible`, `isHidden`, `count`, `all` and the `all*` reads answer at once, take no timeout and return values (ADR 0079); Playwright's `isVisible({ timeout })` ignored its timeout (#12224, #33017). An absent attribute is `null`: an answer (G1).
- No row-wise structured read is added (S2): reads run concurrently, `evaluateAll` reads rows in one call, and a crawl's selector reads and `extract` belong to [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477) and [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473).

### Page functions

- **A page function is JavaScript source run in the page with one explicit argument** (ADR 0081). TypeScript takes a function, whose source is sent, or a string; Python and Rust take a string, as Playwright for Python does. A string is a function if it parses as one, otherwise an expression; an argument given with an expression is a `ProgramError`.
- **Values cross as JSON both ways,** ADR 0082's `JsonValue` (S1, S2). The argument is `JsonValue` in TypeScript and Python and `impl Serialize` in Rust, and TypeScript constrains the result to JSON, so returning a `Date` or an element is a type error (G3).
- A value outside JSON (a non-finite number, a DOM node, a function, a `Map`, a cycle, a `BigInt`) raises `page_value_invalid` (a `ProgramError`) naming its path, its kind and the fix, in either direction; an argument is refused before it is sent. Playwright resolves such a result to `undefined`, silently (G1).
- `undefined` follows `JSON.stringify`: dropped from objects, `null` in arrays. A function that returns nothing gives `undefined` in TypeScript, `None` in Python and JSON `null` in Rust, and `-0` arrives as `0`.
- **An exception thrown in the page** raises `page_function_failed` (a `ProgramError`, the program's own code, as `handler_failed` is) with the page's error name, message and stack. A `ReferenceError`'s fix names the argument: a page function sees only its argument ("pass variable into evaluate" is Puppeteer's second most-voted question).
- **The calls:** `page.evaluate(fn, arg)`, `locator.evaluate(fn, arg)` with the element first, `locator.evaluateAll(fn, arg)`, `waitForFunction(fn, arg, { polling })` with Playwright's `raf` or an interval in the language's duration type (ADR 0080), and `addInitScript(fn, arg)`, which returns a `Subscription` and is registered when called.
- **Its trust level is fixed, so the name stays** (S3, ADR 0019; AI pain 9 asks for a trust level in the name of code-execution verbs). It runs only in the page, with the page's privileges, never in the program's or the engine's process; code a model writes runs in the Script sandbox (ADR 0051).
- **A `world` option** takes `main` (the default, as Playwright and Puppeteer, so page globals are visible) or `isolated` (the page can neither see nor hook it). Main-world calls and init scripts are marked in Provenance (ADR 0019).
- The argument is plain JSON, so a Secret reference cannot be passed in (G5); where Secret references go is Sessions, logins, Human pause and secrets in code's.

### Routes and responses

Selenium's ADR 17685 under Playwright's names, the same in every SDK because the engine applies it (S1; Selenium's bindings had drifted until it was written: Java ran one handler "chosen in no defined order", Ruby raced threads):

- **`page.route(pattern, handler)` and `session.route(pattern, handler)`,** the Session's catching a popup's first request, return a `Subscription`. Patterns are globs or regexes matched in the engine (ADR 0081); routes take no predicate, since filtering in the handler and calling `fallback()` is equivalent (S2, ADR 17685 decision 2).
- **Later routes run first. The first to settle wins:** `continue(overrides)`, `fulfill(response)`, `abort(reason)`, or `fetch(overrides)` followed by `fulfill`. `fallback(overrides)` passes the request, with its overrides, to the route registered before.
- **A handler that ends without settling falls back too,** and past the last route the request goes to the network. Playwright stalls such a request (G1, G6). Settling twice, or after the handler ended, raises `route_already_handled`; in Rust each answer takes the route, so it cannot happen. Return values are ignored, and handlers read the original request.
- **A handler that throws:** its request is aborted, never sent half-changed (ADR 17685 decision 7). The route is removed, and the exception reaches the program as `page_handler_failed` on the next call or close of the value it was registered on, under ADR 0081's listener rule.
- **Routes run before the network.** What a route sends on still passes the run's domain policy (ADR 0014) and Filter lists (ADR 0047); `fulfill` and `abort` reach no network. A route sees what the engine's own interception sees, so adding one changes neither caching nor Service Worker coverage, which Playwright's routing does ("Enabling routing disables http cache"); area 5's spec states both.
- **Bodies.** `body()`, `text()` and `json()` read on demand. They never fail for a response obtained through `waitForRequestFinished` (ADR 0080), `route.fetch()` or `page.request`, whose bodies the engine keeps. Elsewhere, a body the browser has dropped raises `body_unavailable` (an `UnavailableError`) whose fix names `waitForRequestFinished`, where Playwright and Puppeteer report "Request content was evicted from inspector cache". `json()` returns `JsonValue`, and a body that is not JSON raises `response_not_json` (a `PageError`). No streaming body: large files are downloads.
- **`ok` is offered** with Playwright's spelling per language (`ok()` in TypeScript and Rust, the `ok` property in Python): true for a status from 200 to 299. Its doc says a login page that answers 200 is `ok`, since the Page check judges pages (ADR 0006). Omitting it would not stop `status === 200`, and it is the name `fetch`, Playwright and requests users write (S3, T).
- **`page.request`** is Playwright's request API on the page's Session, sent through the browser-fetch tier so the Session's identity does not change (ADR 0020), and returns the same `Response`.

### Events and their data

ADR 0081 fixed their shapes; under Playwright's names (S3):

| Event | Carries |
|---|---|
| `console` | `ConsoleMessage`: type, text, location |
| `pageerror` | `WebError`, Playwright's name for an exception the page did not catch: name, message, stack; never `PageError`, ADR 0079's group |
| `request`, `requestfinished`, `requestfailed` | `Request`: URL, method, headers, post data, resource type, navigation flag, redirect chain, failure |
| `response` | `Response`: URL, status, status text, `ok`, headers, request, bodies |
| `popup` | `Page` |
| `download` | `Download` |
| `filechooser` | `FileChooser` |
| `websocket` | `WebSocket`, with its own `framesent`, `framereceived` and `close` |
| `load`, `domcontentloaded`, `close`, `crash` | nothing |
| The Session's `page` | `Page` |

`Request` and `Response` keep Playwright's accessors per language: methods in TypeScript and Rust, properties in Python. Workers and frame events are left to raw CDP, and `waitForURL` covers navigations.

### Dialogs and overlays

- **A dialog with no handler is dismissed,** since accepting a `confirm()` may confirm an Effect (ADR 0014, ADR 0037's SA-26).
- **`page.on('dialog', handler)` keeps Playwright's spelling,** but the handler answers a request (ADR 0081) under the routes' rules: later handlers first, the first `accept(promptText?)` or `dismiss()` wins, and a handler that does neither falls through. Past the last one the dialog is dismissed. Playwright's docs warn that an unanswered `dialog` listener stalls the action (G1).
- A handler that throws: the dialog is dismissed, and `page_handler_failed` follows the routes' rule. In Rust a dialog is an `EventStream` item, and dropping one unanswered passes it on.
- `beforeunload` dialogs appear only through `close({ runBeforeUnload: true })`, as in Playwright.
- **`addLocatorHandler(locator, handler, { times, noWaitAfter })`,** Playwright's handler for overlays that interrupt an action (P01, Midscene #1551), returns a `Subscription`. Before each attempt of a guarded action, when its locator is visible, the guard calls the handler in the program, then waits for the locator to be hidden unless `noWaitAfter`. Handlers run one at a time, their own actions trigger none, and their time counts toward the action's timeout. One that throws fails that action with `page_handler_failed`. It sits beside ADR 0048's Consent prompt refusal and ADR 0047's hiding, which already clear known overlays.

### Navigation

- `goto`, `reload`, `goBack` and `goForward` return `Response | null`, with a 404 or a 500 as a value (ADR 0079). `url` is read without a round trip, kept current by the engine's navigation events. `title()` and `content()` are reads, and `setContent(html)` is a gesture.
- **An unreachable site** raises `navigation_failed`, an `UnavailableError` carrying the browser's network error, retryable by kind: a reset connection is, an unresolved name is not. This reads ADR 0079's "something Goodfetch depends on is unreachable" as covering the site a navigation needs.
- A navigation superseded by another before it committed raises `navigation_interrupted` (a `PageError`). A renderer crash raises `page_crashed` (an `UnavailableError`) and emits `crash`. A call on a page the program closed raises ADR 0078's `closed`, so "target crashed", "closed by your code" and "navigated away" are told apart (browser drivers P6); a locator simply resolves again on the new document.

### Files

- **Uploads:** `setInputFiles` and `fileChooser.setFiles` take paths or files in memory. The SDK reads paths on the program's machine and streams them over the connection, so uploads work with a Daemon or Goodfetch Cloud (ADR 0001).
- **Downloads** are kept by the engine with their Run (ADR 0037's CO-22). `Download` gives `url`, `suggestedFilename`, `failure()` (an answer: the reason or `null`), `saveAs(path)`, streamed to the program's machine, and `cancel()`. `saveAs` on a failed download raises `download_failed` (an `UnavailableError`). There is no `path()`, which means nothing with a remote engine.
- **Screenshots and PDFs:** `screenshot()` on a page or a locator returns bytes, with Playwright's options (`fullPage`, `clip`, `mask`, `path`, …); the engine masks Secrets anyway (ADR 0014). `pdf()` works on Chromium only.
- **The clipboard has no API** (ADR 0037's SA-30): the page's own clipboard is reached through `keyboard.press('ControlOrMeta+C')` and `evaluate`, with the permission a Session setting, never the host's clipboard.

### The raw CDP way down

- **`page.newCdpSession()` returns a `CdpSession`:** `send(method, params?)` returning `JsonValue`, events through `on` (an `EventStream` in Rust), and `close()` (G8). Puppeteer's `createCDPSession()` and Playwright's `context.newCDPSession(page)` are the leaders' forms. browser-use left Playwright because its way down was too narrow (browser drivers P12).
- **It travels over the program's own connection,** so no port or token is opened. It is a raw CDP attach under ADR 0017: the Session's traffic goes through the egress proxy, no Secret is typed, results are marked unguarded, the attach is in the Audit log, and an attach from a handler is bound to its work item's lease (ADR 0003).
- **It reaches only its page's target.** The engine refuses with `cdp_command_refused` (a `RefusedError`) commands that close or attach other targets, and on a Borrowed browser anything that would close the browser (G4).
- On Firefox and WebKit, which run through Playwright's driver (ADR 0019), it raises `capability_unavailable` ([Settings in code](https://github.com/goodfetch/automan/issues/470)'s code for a capability a platform cannot provide).
- **Semver covers its shape, never the protocol:** methods, parameters and events are Chromium's and typed `JsonValue`. No generated CDP types enter the SDK API (G7), as Selenium's ADR 17670 keeps raw BiDi out of its public API.

### What ADR 0019's mapping leaves out

ADR 0019 asks each Playwright call to map or be excluded with a reason. Mapped calls keep their names; these are excluded:

- **Selector-taking page shortcuts** (`page.click(selector)`, `page.fill`, `$`, `$$`, `$eval`, `waitForSelector`) and `dragAndDrop`: S2, ADR 0077's own example. `locator.type` and `waitForNavigation`, which Playwright itself replaced. No tombstones are added: each has its replacement one dot away, the type checker flags the call at its line, and tombstoning thirty discouraged names would freeze them per language at 1.0 (ADR 0030). ADR 0080's sleep tombstone stays the only one.
- **`ElementHandle`, `JSHandle` and `Frame`:** the handle model.
- **`trial`, `noWaitAfter` on actions and `dispatchEvent`.** Synthetic, untrusted events are `evaluate`'s.
- **`exposeFunction` and `exposeBinding`:** they let page scripts call into the program (G5); events and `waitForFunction` cover their uses.
- **Setters:** `setDefaultTimeout` (ADR 0080), `setViewportSize`, `emulateMedia` and `setExtraHTTPHeaders`. Session settings (ADR 0028), Hooks (ADR 0033) and `route.continue({ headers })` cover them.
- **Elsewhere:** `page.pause()` is a Human pause ([Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476)). Web-first assertions (`expect`) map to the deterministic verb that runs a Success check, and `ariaSnapshot()` and aria references are observations, both [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473)'s. `page.clock` is a testing feature ([Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482)). Video and coverage belong to the Run trace (ADR 0031) and raw CDP, and workers to raw CDP.

### Error codes

Each code has one class under ADR 0079's naming rule, and none shadows a built-in:

| Group | Codes |
|---|---|
| `ProgramError` | `selector_invalid`, `page_function_failed`, `page_value_invalid`, `page_handler_failed`, `route_already_handled` |
| `PageError` | `target_not_unique`, `target_unsupported`, `navigation_interrupted`, `response_not_json` |
| `UnavailableError` | `navigation_failed`, `page_crashed`, `body_unavailable`, `download_failed` |
| `RefusedError` | `cdp_command_refused` |

`page_handler_failed` and `route_already_handled` are raised by the TypeScript and Python SDKs only: Rust answers those requests in the program's own code, and its answers take the route.

## Considered Options

- **A `BrowserContext` beside the Session:** rejected by S2 and the glossary, whose Session avoids "context".
- **Playwright's `Frame` handles beside frame locators:** rejected. An eager handle detaches on navigation, and two ways reach a frame. **A frame pointer**, as Selenium and WebdriverIO before v10: rejected by G2.
- **Playwright's whole selector language** (`text=`, `>>`, `internal:`): rejected by S2. Every engine has a method, and the mini-language would be frozen at 1.0.
- **Exact text matching by default,** as Testing Library and Playwright's open request #19235: rejected by S3. The same name with a different default is the case models get wrong most.
- **Failing at once on an ambiguous target,** as Playwright: rejected. It is the only condition not waited out, and transient duplicates during a re-render fail. **Failing once the same matches have held for a settle window:** rejected. It is a heuristic and, under ADR 0028, one more setting.
- **Overlapping gestures,** as Playwright allows: rejected; keystrokes and pointer moves interleave silently. **Every call on a page one at a time:** rejected; it deadlocks ADR 0080's waiters, which must be armed while their gesture runs.
- **A structured row-wise read** beside the getters: rejected by S2. Concurrent reads, `evaluateAll`, the crawl's selector reads and `extract` already cover it.
- **Playwright's serializer for page functions** (JSON plus `NaN`, `±Infinity`, `-0`, and `undefined` for anything else): rejected. Python's and Rust's JSON idioms cannot hold the extras, and the silent `undefined` breaks G1.
- **Renaming `evaluate` to carry a trust level,** as AI pain 9 suggests for code-execution verbs: rejected. Its trust level is fixed by where it runs, the page, and a new name breaks S3 and ADR 0019.
- **Routes that stall when no handler settles,** as Playwright: rejected by G1 and G6. **Predicates in route patterns,** as Playwright: rejected by S2 and ADR 17685; they would also make the engine pause every request for the program.
- **A route that throws staying registered,** as a crawl handler stays: rejected. ADR 0081's listener rule is one rule for every function a value calls, and the program hears of it at its next call.
- **No `ok()`:** rejected. It would not stop `status === 200`, and agents and users write `ok` everywhere (S3, T).
- **A `dialog` listener that must answer,** as Playwright: rejected; it stalls the action. **A declarative overlay dismissal** (click this when that appears): rejected; it cannot express a close that takes two steps.
- **Generated CDP types in the SDK API,** as chromiumoxide's: rejected by G7. They break on Chrome updates (browser drivers P11) and would put a third party's protocol under Goodfetch's semver.
- **Tombstones for every discouraged Playwright call:** rejected by S2 and ADR 0030.

## Consequences

- **Adds to ADR 0019.** This ADR is the mapping its Consequences ask for, in all three SDKs; each Playwright call maps here or is excluded with a reason.
- **Adds to ADR 0023.** The guard's per-call switch is `force`, the setting `action.force` given for one call.
- **Adds to ADR 0017.** A CDP session opened from an SDK is a raw CDP attach under its rules, relayed over the program's connection, so it needs no endpoint or token of its own.
- **Adds to ADR 0047.** User routes run before Filter lists, and what a route sends on is blocked or allowed by them like any request.
- **Adds to ADR 0081.** Route, dialog and locator handlers follow its listener rule for exceptions, under `page_handler_failed`. Routes take no predicate. In Rust, dropping a route, dialog or locator-handler item unanswered gives its default.
- **Error codes.** The table's codes join `goodfetch-protocol`'s list through `xtask codegen`'s naming checks; `target_not_unique` gains its details (the matches, the wait).
- **Area 5 (Site access)** builds strictness in the guard, gestures in arrival order, route and dialog semantics, kept bodies, events and their data, the CDP session's scope and refusals, and the closed shadow root and background-tab questions. **Area 11 (Client surfaces)** builds the page API's shapes per language, the JSON value checks and the handler rules. Each lists this ADR.
- **ADR 0037's matrix:** SA-04, SA-12, SA-13, SA-16, SA-17, SA-18, SA-19, SA-26, SA-29 and SA-30 are decided here, and SA-15's frames and open shadow roots.
- **Other tickets of the map.** Sessions, logins, Human pause and secrets in code decides the Session's own API, a default Session, cookies (SA-21), HTTP auth challenges (SA-27), the clipboard permission and where Secret references go. Crawl definition and handler context hands its handlers this page and `page.request`. Its crawl routes (ADR 0020's "per route") share a word with `page.route`, so that ticket names them to avoid the clash, or `GLOSSARY.md` splits the term. [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) names the per-language forms (`continue_`, `ok` as a property, `newCdpSession`). Collie's AI verbs and Plans builds on locators, frame locators and observations. A new ticket, [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500), decides how a program declares that its own action or request is an Effect (ADR 0010).
- **TESTING.md** gains page API conformance cases.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in its `page` files for each language.
