# Service workers run under the engine's interception, the back/forward cache is off, locators reach closed shadow roots, and a read never scrolls a list

[ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md) left two of its page's states to area 5's spec: Service Worker coverage, and whether closed shadow roots are reached. No ADR mentions two others: the back/forward cache, and virtualized lists.

Each changes what a program can rely on:
- the egress policy (ADR 0017), Filter lists (ADR 0047), the Response cache (ADR 0046), `route` and kept responses (ADR 0110) all assume the engine sees every request;
- `waitForLoadState` (ADR 0080) and the navigation gate (ADR 0087) assume a navigation fires its events;
- `extract` (ADR 0091) copies values from the elements it finds.

The [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547) found all four, rows B10, B13, B20 and B21 of its [`pitfalls.md`](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md), from items 3.2, 3.11, 3.13 and 3.21 of the [common pitfalls research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md).

The decisions were made in [Page states the page API leaves open](https://github.com/goodfetch/goodfetch/issues/552), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), every answer by the agent on the owner's delegation. The evidence is primary sources read on 2026-10-16:
- Playwright 1.64.1's docs and source;
- Puppeteer's, Crawlee's and chromedriver's source;
- the Chrome DevTools Protocol's domain files;
- the WebDriver and WebDriver BiDi specifications;
- WAI-ARIA 1.2;
- Chrome's back/forward cache documentation.

Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Service workers

- **On Chromium, a site's service workers run, as in Playwright, whose `serviceWorkers` defaults to `'allow'`** (S3).
  - **The engine attaches to each one before it runs.** It uses the browser protocol's auto-attach, which pauses a new worker until it is set up. So no request leaves a worker before interception covers it, and ADR 0014's domain policy still fails closed.
  - **What a worker requests itself passes the same interception as a page's subresources:** precaching, background sync, and fetches it passes through to the network. That covers the domain policy, Filter lists (ADR 0047 already covers "service workers included"), the Response cache and the Session's routes.
  - **Its requests never pass a page's routes, and a page's events do not report them, since a worker belongs to no page.** Playwright's `context.route` likewise sees a worker's requests, and `page.route` does not. The Run trace records them.
  - **The navigation gate (ADR 0087) counts what reaches the site.** A worker's request that carries a page's navigation to the network, as a network-first worker passes it on, takes the permit that navigation would take, so a worker never lets a crawl press a site past Politeness (G5). A navigation the worker answers from its own cache sends nothing to the site and takes none.
- **A page request that the worker answers itself reaches no network.** Typically it answers from its own Cache Storage.
  - There is nothing for the domain policy or a Filter list to judge.
  - A route and the Response cache act only at interception, so they cannot answer it or serve it. Playwright's routes miss it too ("will not intercept requests intercepted by Service Worker").
  - The page's `response` event, its waits and a crawl's kept responses (ADR 0110) see it, as they see any response the engine observes. Its `Response` answers `fromServiceWorker()` with `true`, Playwright's accessor (S3).
  - Its body follows ADR 0085's rules. It never raises `body_unavailable` after `waitForRequestFinished` or once kept. Read later, from a `response` event, it may.
  - So the docs of `route` and the Response cache state the rule: when a site's worker answers what they must act on, set `block`.
- **`page.request` never goes through a service worker.** It is the engine's own request (ADR 0020's browser-fetch tier), sent to the network as Playwright's `APIRequestContext` sends its requests.
- **On Firefox and WebKit, no service worker registers.** Playwright's driver, through which ADR 0019 runs them, intercepts none of a worker's requests. So the domain policy could not judge them, and ADR 0014 fails closed. Neither browser is a stealth path (ADR 0035).
- **The setting `browser.service_workers`,** with Playwright's values `allow` and `block` (S3, ADR 0083), is a Session setting. It is `allow` by default on Chromium and `block` on Firefox and WebKit, and `explainSettings` shows which applies and why (G6).
  - An explicit `allow` on Firefox or WebKit raises ADR 0083's `capability_unavailable`, whose fix names `block` (G3).
  - On a Borrowed browser, where `browser.*` settings are refused (ADR 0083), the person's own workers run as they are. On Chromium the engine attaches to them as it does on its own browser.
  - On a page whose interception a raw CDP client can switch off, a worker's requests are held at the network layer as its page's are (ADRs 0017, 0089).
- **`block` changes nothing the page can read.**
  - Registration fails as a browser's own refusal fails, with `register()`'s promise rejected.
  - `register` stays the browser's own native function. Playwright's `'block'` replaces it with a page-visible stub that logs "Service Worker registration blocked by Playwright", which would break the `standard` Access mode's no-injection identity (ADR 0021) (G5).
  - Area 5's spec picks the mechanism inside the browser.

### The back/forward cache

- **The back/forward cache is off in every browser, with no setting,** as Playwright does in each one. In Chromium it is the switch `--disable-back-forward-cache`, commented "Avoids surprises like main request not being intercepted".
- **`goBack` and `goForward` always load their document again.** A load passes interception, the domain policy and the navigation gate (ADR 0087). It fires `commit`, `domcontentloaded` and `load`. It returns its `Response`, so `waitForLoadState`, `waitForURL` (ADR 0080) and a Page check (ADR 0006) hold after it (G1).
- **A restore would skip all of that.** It sends no request and fires only `pageshow`. Playwright's docs call restores unsupported because they desynchronize the page object.
- **Turning the cache off shows a site nothing it could single out.** Chromium reports the switch to the page as the `notRestoredReasons` reason `masked`, which other browser-internal causes also produce.
- **Prerendering by Speculation Rules** also activates a document without a load of its own, and Playwright leaves it on. Area 5's spec tests an activation against `waitForURL`, a Page check and the navigation gate. If an activation skips them, prerendering is switched off under the same rule as the back/forward cache.

### Virtualized lists

- **A read never scrolls.** `extract`, `observe`, `ariaSnapshot` and ADR 0085's getters read the DOM as it is and move nothing.
  - A virtualized list, such as one built with TanStack Virtual, which renders "only the visible items", yields only its rendered rows.
  - A read that scrolled would be a Gesture (ADR 0085). Two concurrent reads would fight over one scroll position, and a Plan step would replay against a position the program never set (G1, G2).
- **No virtualized-list helper is added** (S2). No automation leader has one (Playwright, Puppeteer, Selenium, Crawlee). ADR 0037's CO-17 infinite-scroll helper, area 6's spec, is the one place a scroll helper may be added. If it is added, it collects rows by a key at each step, so it covers windowed lists too.
- **The answers, data first:**
  - a list fed by JSON calls is read from them: kept responses (ADR 0110), `waitForResponse` (ADR 0080), or the discovered-API tier (ADR 0020);
  - data already in the page is read with `evaluate`;
  - otherwise, the docs show a recipe in each SDK: scroll the last rendered row into view, read the rows' keys and values with `evaluateAll`, and stop when no new key appears.
  - The recipe holds on recycled nodes because a locator is resolved again at each use and no handle goes stale (ADR 0085).
- **A list that declares more rows than it renders says so in the `ai` aria snapshot** (G1, G6).
  - WAI-ARIA 1.2 has a page declare it: `aria-rowcount`, or its items' `aria-setsize`, applies "if only a portion of the rows is present in the DOM".
  - In `ai` mode, a grid, table, list or listbox whose declared count exceeds the rows present carries one line giving both counts, as ADR 0091 marks a collapsed subtree. An Agent or a model-mode `extract` then knows that the rows it sees are not all of them.
  - An unknown total (`-1`) is marked as unknown.
  - The default mode keeps Playwright's text, which carries none of these attributes (S3).

### Closed shadow roots

- **On Chromium, the page API reaches author shadow roots, open or closed alike.** That covers CSS and `getBy*` locators, the guard's checks (ADR 0019), strictness (ADR 0085) and `ariaSnapshot`, and so the AI verbs too. It also covers a closed root that a `<template shadowrootmode="closed">` declares.
  - The browser's own user-agent roots, such as an `<input>`'s internals, are never reached.
  - `<template>` content stays inert.
  - XPath pierces neither kind, as in Playwright.
- **Why.** A closed root hides its content from page scripts, not from a person or a screen reader: its content is rendered and in the accessibility tree. A `getByRole` that missed a button a person sees would break the meaning its name promises (S3, G1).
  - The W3C automation standards already reach these roots. WebDriver's Get Element Shadow Root reads "element's shadow root", the internal slot, with no mode check, and WebDriver BiDi's `includeShadowTree: "all"` serializes closed roots.
  - Playwright lacks the feature because its injected script cannot traverse them. Its maintainer said there is "no support for closed shadow DOM, because it is not available to JavaScript traversal", on [#23047](https://github.com/microsoft/playwright/issues/23047), open with 30 votes. A pull request forcing roots open was closed unmerged.
- **The engine never changes the page's roots.** It resolves each closed root into its own isolated world through the browser protocol, where CDP's `DOM.getDocument` with `pierce` and `DOM.resolveNode` reach any node.
  - It never patches `attachShadow` to force roots open. Pages can detect that, and Playwright's maintainer warned that it "may affect the behavior of some pages".
  - So the guard's checks hold in full, nothing is marked unguarded, and the page sees no change (G5).
- **`locator.evaluate` hands such an element to a page function in either world.** The main world's own scripts still cannot reach the root.
- **On Firefox and WebKit,** which run through Playwright's driver (ADR 0019), closed roots are not reached. When one may hide a target, ADR 0085's deadline error's `observed` names the closed root and says that Chromium reaches it (G6).

## Considered Options

- **Service workers blocked by default, with a `browser.*` setting that allows them,** as Playwright's docs advise under `route`: rejected. Every site would see a browser no person uses, to protect only the programs that mock what a worker answers, and those set `block`.
- **Bypassed by default** (Puppeteer's `setBypassServiceWorker`, CDP's `Network.setBypassServiceWorker`): rejected. No leader defaults to it. It leaves a registered worker that never sees its page's fetches, a state no real browser is in, and it would add a third value for one use (S2).
- **Playwright's `'block'` as it is,** an init script that stubs `register`: rejected by G5 and ADR 0021. A page can read the stub and its console warning.
- **Service workers allowed on Firefox and WebKit too,** their requests unseen: rejected by ADR 0014, which fails closed on a request the policy cannot judge.
- **The back/forward cache left on, with a restore reported as its own navigation:** rejected. A restore passes neither interception nor the gate, has no `Response` for `goBack` to return, and is a navigation kind no leader supports. Puppeteer and chromedriver leave the cache on; Playwright, which switches it off, is the leader whose names the page keeps (ADR 0019).
- **`extract` and `observe` scrolling a list and collecting its rows by key:** rejected. A read would become a Gesture, concurrent reads would contend for one scroll position, and a Plan step would depend on a position the program never set.
- **A virtualized-list helper on the page:** rejected by S2. It would be a second scroll helper beside CO-17's, with no leader's to copy (S3).
- **The ARIA counts in the default-mode aria snapshot:** rejected by S3. That mode's text is Playwright's, compared by `toMatchAriaSnapshot`.
- **Closed shadow roots declined, as in Playwright:** rejected. `getBy*` would miss what a person and the accessibility tree see, and the W3C automation standards reach them.
- **Forcing closed roots open with an init script,** the recipe Playwright users share: rejected by G5. Pages detect a patched `attachShadow`, and an open root changes what the page's own scripts can do.
- **Reaching them, but marking such results unguarded in Provenance:** rejected. The guard's checks run on them in full, so a mark would report a safeguard that never lapsed.

## Consequences

- **Amends ADR 0085.** Notes sit under:
  - its "Shadow DOM" bullet: closed roots are reached on Chromium;
  - "Routes run before the network": what a page's routes see under a service worker;
  - the events table: `Response` gains `fromServiceWorker()`;
  - "Navigation": `goBack` and `goForward` always load their document;
  - its Consequences' area 5 bullet, whose closed shadow root question is answered here.
- **Amends ADR 0083.** `browser.service_workers` joins the `browser.*` Session settings. A note sits under them.
- **Amends ADR 0091.** A read never scrolls, and the `ai` aria snapshot marks a list that declares more rows than it renders. A note sits under "Observations".
- **Amends ADR 0025.** The Proving ground's hand-made pages gain:
  - a page whose service worker answers one JSON call from its cache and passes another to the network;
  - a page reached by Back after leaving it;
  - a virtualized list of 1,000 rows that declares its count;
  - a target inside a closed shadow root, attached by script and declared in HTML.

  A note sits under its page list.
- **Adds to ADR 0014.** On Chromium, a service worker's own requests are judged as a page's subresources are. On Firefox and WebKit, where they cannot be judged, workers never register.
- **Adds to ADR 0047.** Filter lists cover a worker's own requests on Chromium. A request the worker answers from its own cache reaches no network and needs no blocking.
- **Adds to ADR 0046.** The Response cache acts at interception, so a request a site's worker answers itself is outside it. Its docs say to set `browser.service_workers: block` when that matters.
- **Adds to ADR 0110.** Kept responses keep a response a site's worker answers, as waits see it, marked by `fromServiceWorker()`.
- **Adds to ADR 0087.** `goBack` and `goForward` always reach the navigation gate as navigations. A service worker's request that carries a page's navigation to the network takes that navigation's permit, and a navigation the worker answers from its own cache takes none.
- **Error codes.** None is new. `capability_unavailable` (ADR 0083) refuses an explicit `allow` on Firefox and WebKit.
- **Who builds what.** Each area lists this ADR.
  - **Area 5 (Site access)** builds:
    - the worker auto-attach and its interception;
    - `block`'s browser-side refusal;
    - the back/forward cache switches, and the prerendering test;
    - closed-root resolution in the isolated world, for locators, the guard and `ariaSnapshot`;
    - the `observed` text on Firefox and WebKit.
  - **Area 6 (Collection)**, if it adds CO-17's infinite-scroll helper, has it collect rows by a key at each step.
  - **Area 9 (AI and agent)** builds the `ai` snapshot's count line.
  - **Area 11 (Client surfaces)** builds `fromServiceWorker()` and the setting per language, and the doc text of `route`, `extract` and the virtualized-list recipe.
  - **Area 2 (Proving ground and conformance)** builds the four pages.
- **ADR 0037's matrix.**
  - SA-15 becomes `decided`, since its last `spec` part, closed shadow roots, is decided here.
  - SA-14, SA-17 and CO-17 cite this ADR.
  - The status counts follow.
- **`TESTING.md`** gains the page state conformance cases. The virtualized-list recipe runs as a docs code sample.
- **The `prototype/sdk-api` sketch** holds `browser.service_workers` and `fromServiceWorker()` in each language. Its pitfall rows B10, B13, B20 and B21 close as Handled, and its pain row P34 names this answer.
