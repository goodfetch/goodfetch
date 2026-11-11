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
- Firefox's, WebKit's and Camoufox's source, for their service-worker preferences;
- the Chrome DevTools Protocol's domain files;
- the WebDriver and WebDriver BiDi specifications;
- WAI-ARIA 1.2;
- Chrome's back/forward cache documentation.

Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Service workers

- **On Chromium, a site's service workers run, as in Playwright, whose `serviceWorkers` defaults to `'allow'`** (S3).
  - **The engine attaches to each service worker before it runs.** It uses the browser protocol's auto-attach, which pauses a new service worker target until it is set up. So no request leaves a service worker before interception covers it, and ADR 0014's domain policy still fails closed.
  - **What a service worker requests itself passes the same interception as a page's subresources:** precaching, background sync, and the fetches it makes to pass a page's request on to the network. That covers the domain policy, Filter lists (ADR 0047 already covers "service workers included"), the Response cache and the Session's routes.
  - **A service worker's own requests never pass a page's routes, and a page's events do not report them, since a service worker belongs to no page.** Playwright's `context.route` likewise sees them, and `page.route` does not. The Run trace records them.
  - **The navigation gate (ADR 0087) counts what reaches the site.** A service worker's request that carries a page's navigation to the network, as a network-first service worker passes it on, takes the permit that navigation would take, so a service worker never lets a crawl press a site past Politeness (G5). The engine recognises that request by its `navigate` mode, and holds the permit as ADR 0087 holds a browser-tier page's, until the page's load event. A navigation the service worker answers from its own cache sends nothing to the site and takes none.
    > Note (agent decision, as built), 2026-10-23, [#720](https://github.com/goodfetch/goodfetch/issues/720): CDP's paused request (`Fetch.requestPaused`) names no fetch mode, so the engine recognises it by its URL, fragment removed, against the main-frame navigations pending on the pages of the worker's Session, as spec #577's second rule does for any other mode. That covers the `navigate`-mode request too, which carries the pending navigation's URL. Each pending navigation is taken once; its permit is released at the page's `load` event.
  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-24, [#632](https://github.com/goodfetch/goodfetch/issues/632): a site's shared worker, which belongs to no page either, is attached the same way (browser-level auto-attach now also takes `shared_worker` targets) and held by its Session's interception before it runs, whatever `browser.service_workers` says; its refused requests are recorded in the Run trace as `page.request_failed`. Each page also attaches its out-of-process frames and dedicated workers before they run (`Target.setAutoAttach` on the page's and each frame's target): the Consent prompt runner's earlier auto-attach of frames alone left every dedicated worker paused, never run.
- **A page request that a service worker's fetch handler answers never passes the page's interception.**
  - The fetch handler either answers from the service worker's own Cache Storage, which reaches no network, or passes the request on with its own fetch, the request above.
  - Neither a page's route nor the Response cache can answer or serve the page's request. Playwright's routes miss it too ("will not intercept requests intercepted by Service Worker"). What reaches the network was already judged as the service worker's own request, so nothing escapes the domain policy or Filter lists.
  - Its `Response` answers `fromServiceWorker()` with `true` in both cases, Playwright's accessor (S3), which is true for a response "fulfilled by a Service Worker's Fetch Handler".
  - The page's `response` event, its waits and a crawl's kept responses (ADR 0110) see it. They match the responses the engine observes through the browser protocol's network events, a service worker's answer among them, not only those its interception pauses.
  - Its body follows ADR 0085's rules. It never raises `body_unavailable` after `waitForRequestFinished` or once kept. Read later, from a `response` event, it may.
  - So the docs of `route` and the Response cache state the rule: when a site's service worker answers what they must act on, set `block`.
- **`page.request` never goes through a service worker.** It is the engine's own request (ADR 0020's browser-fetch tier), sent to the network as Playwright's `APIRequestContext` sends its requests.
- **On Firefox and WebKit, no service worker registers.** Playwright's driver, through which ADR 0019 runs them, intercepts none of a service worker's requests. So the domain policy could not judge them, and ADR 0014 fails closed.
  - **This has a cost on Firefox.** Firefox stealth is Camoufox, through that same driver (ADR 0035). A `stealth` Camoufox Session therefore offers no service worker where real Firefox does, a difference a site can see.
  - ADR 0014's guarantee outranks it (G5), and Chromium is the engine that leads stealth (ADR 0035).
  - Area 8's spec measures how often a site's challenge or page needs a service worker on Camoufox. WebKit runs without a stealth mode (ADR 0035).
- **The setting `browser.service_workers`,** with Playwright's values `allow` and `block` (S3, ADR 0083), is a Session setting. It is `allow` by default on Chromium and `block` on Firefox and WebKit, and `explainSettings` shows which applies and why (G6).
  - An explicit `allow` set where a Firefox or WebKit Session opens raises ADR 0083's `capability_unavailable`, whose fix names `block` (G3).
  - An `allow` inherited from the Run or the environment, set there for its Chromium Sessions, yields to `block` on a Firefox or WebKit Session, as ADR 0083's inherited values yield, and `explainSettings` says so.
  - On a Borrowed browser, where `browser.*` settings are refused (ADR 0083), the person's own service workers run as they are. On Chromium the engine attaches to them as it does on its own browser. With no network layer under that browser, their requests are held by interception alone, as its pages' are (ADRs 0014, 0056).
  - On a page whose interception a raw CDP client can switch off, a service worker's requests are held at the network layer as its page's are (ADRs 0017, 0089).
- **`block` refuses registration with the browser's own switch, never a page-visible stub, wherever the browser has one** (G5, ADR 0021).
  - **On Chromium, `block` changes nothing the page can read.** Registration fails as a browser's own refusal fails, with `register()`'s promise rejected, and `register` stays the browser's own native function. Area 5's spec picks the mechanism inside the browser.
    > Note (agent decision, on a guarantee, not read by the owner), 2026-10-23, [#707](https://github.com/goodfetch/goodfetch/issues/707): Chrome for Testing 156 has no per-context switch that makes `register()` reject (no permission, content setting or browser-context option does), so `block` is spec #577's floor. Chromium fetches a worker's main script outside every page's and worker's own interception, so the browser-level `Fetch` domain, turned on once a Session of the browser chooses `block` and pausing only the resource kind a worker's script is (`Other`), fails each request carrying `Service-Worker: script` whose worker target is in a `block` Session's browser context with `net::ERR_FAILED`: `register()` rejects with a `TypeError` ("An unknown error occurred when fetching the script"), and every other request it paused goes on unchanged. The engine sets browser-level auto-attach (`Target.setAutoAttach` with `waitForDebuggerOnStart` and a filter of `service_worker` targets alone) on every browser it launches; a site's worker (an `http` or `https` script) of an `allow` Session runs only once its own target's `Fetch` domain pauses its requests, which pass the page pipeline under the Session's routes alone (`route.handle`'s `page_id` is absent for them), and a worker the interception could not hold is closed, never run unheld. A browser extension's worker, and one of a context no Session chose for, runs untouched. The Run trace records each of a worker's own requests in `trace/requests.jsonl` as `service_worker.request` `{session_id, url, method, resource_type, failure?}`. A Session's own `settings` may set `browser.service_workers`. The worker's main script itself is no request of a page or of the worker, so neither a route nor the domain policy sees it; it is same-origin with the page that registered it.
  - **On Firefox, Camoufox included, the engine sets Firefox's preference `dom.serviceWorkers.enabled` to `false`** through the driver's Firefox preferences: Playwright's `firefoxUserPrefs`, and Camoufox's `firefox_user_prefs`, which its own configuration leaves unset. Firefox then exposes no `navigator.serviceWorker`, as any Firefox with that preference off does, and injects nothing.
  - **On WebKit, Playwright's driver reaches no such switch.** WebKit's `ServiceWorkersEnabled` preference is not among the settings its inspector protocol's `Page.overrideSetting` takes, and the driver takes no WebKit preferences. There `block` is Playwright's stub of `register`, which logs "Service Worker registration blocked by Playwright" and registers nothing.
    - The Session is marked in Provenance as ADR 0019 marks a main-world action on WebKit. Like a user init script, the stub departs from `standard`'s no-injection identity (ADR 0021, SA-16), and the mark says so.
    - A WebKit switch the engine can reach replaces the stub.
  - **`block` also keeps a service worker that is already registered, such as one in a Profile directory used in place (ADR 0056), from controlling a page.** Failing `register()` alone would not, and a page's requests must all reach its routes. Area 5's spec proves it in each browser.
    - Where a browser's mechanism cannot, as Playwright's WebKit stub cannot, a Session on a Profile directory used in place that holds a service worker registration is refused when it opens, with `capability_unavailable`. Its fix names a run-only import of the directory's state with `state_file` (ADR 0056), which carries no service worker (G3, ADR 0014).

### The back/forward cache

- **The back/forward cache is off in every browser, with no setting,** as Playwright does in each one. In Chromium it is the switch `--disable-back-forward-cache`, commented "Avoids surprises like main request not being intercepted".
- **`goBack` and `goForward` always load their document again.** A load passes interception, the domain policy and the navigation gate (ADR 0087), or, when a service worker controls the page, its fetch handler under the rules above. It fires `commit`, `domcontentloaded` and `load`. It returns its `Response`, so `waitForLoadState`, `waitForURL` (ADR 0080) and a Page check (ADR 0006) hold after it (G1).
- **A restore would skip all of that.** It sends no request and fires only `pageshow`. Playwright's docs call restores unsupported because they desynchronize the page object.
- **Turning the cache off shows a site nothing it could single out.** Chromium reports the switch to the page as the `notRestoredReasons` reason `masked`, which other browser-internal causes also produce.
- **Prerendering by Speculation Rules** also activates a document without a load of its own, and Playwright leaves it on. Area 5's spec tests an activation against `waitForURL`, a Page check and the navigation gate. If an activation skips them, prerendering is switched off under the same rule as the back/forward cache.
  > Note (agent decision, on a default, not read by the owner), 2026-10-25, fix-queue 83: a page left on a network error page is not loaded again by the browser on its own either. Chromium reloads such a page a second after its error page, then with backoff, unless it runs with `--enable-automation`, as Playwright and Puppeteer launch it and the engine does not; so every Chromium launch passes `--disable-auto-reload`. The reload is a request no call made, and the egress relay's refusal of it was raised by the page's next call as that call's own: under load, the H03 certificate test saw the earlier host's `certificate_invalid` for the next host's navigation.

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
  > Note (agent decision, on a default, not read by the owner), 2026-10-25, [#784](https://github.com/goodfetch/goodfetch/issues/784): `page.formats` reads closed and open roots alike. The document is serialized once with every shadow root (`DOM.getOuterHTML`'s `includeShadowDOM`, each root a `<template shadowrootmode>`), and the extractor reads its composed tree: each root's content in its host's place, each `<slot>` filled with the host's children assigned to it (by `slot` attribute, the default slot taking those with none and text), else its fallback content; a host's child no slot takes is left out, as it is not rendered. A `<template>` without `shadowrootmode` stays inert.
- **Why.** A closed root hides its content from page scripts, not from a person or a screen reader: its content is rendered and in the accessibility tree. A `getByRole` that missed a button a person sees would break the meaning its name promises (S3, G1).
  - The W3C automation standards already reach these roots. WebDriver's Get Element Shadow Root reads "element's shadow root", the internal slot, with no mode check, and WebDriver BiDi's `includeShadowTree: "all"` serializes closed roots.
  - Playwright lacks the feature because its injected script cannot traverse them. Its maintainer said there is "no support for closed shadow DOM, because it is not available to JavaScript traversal", on [#23047](https://github.com/microsoft/playwright/issues/23047), open with 30 votes. A pull request forcing roots open was closed unmerged.
- **The engine never changes the page's roots.** It resolves each closed root into its own isolated world through the browser protocol, where CDP's `DOM.getDocument` with `pierce` and `DOM.resolveNode` reach any node.
  - It never patches `attachShadow` to force roots open. Pages can detect that, and Playwright's maintainer warned that it "may affect the behavior of some pages".
  - So the guard's checks hold in full, nothing is marked unguarded, and the page sees no change (G5).
- **`locator.evaluate` hands such an element to a page function in either world.** The main world's own scripts still cannot reach the root.
- **On Firefox and WebKit,** which run through Playwright's driver (ADR 0019), closed roots are not reached. When one may hide a target, ADR 0085's deadline error's `observed` names the closed root and says that Chromium reaches it (G6).
  - So a locator that is unique on Firefox or WebKit can match more elements on Chromium, inside a closed root, and raise `target_not_unique` there. The docs of `locator` warn programs that run on several browsers.

## Considered Options

- **Service workers blocked by default, with a `browser.*` setting that allows them,** as Playwright's docs advise under `route`: rejected. Every site would see a browser no person uses, to protect only the programs that mock what a service worker answers, and those set `block`.
- **Bypassed by default** (Puppeteer's `setBypassServiceWorker`, CDP's `Network.setBypassServiceWorker`): rejected. No leader defaults to it. It leaves a registered service worker that never sees its page's fetches, a state no real browser is in, and it would add a third value for one use (S2).
- **Playwright's `'block'` as it is,** an init script that stubs `register`, anywhere a browser switch exists: rejected by G5 and ADR 0021. A page can read the stub and its console warning. It remains only on WebKit, where the driver reaches no switch, and is marked there.
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
- **Adds to ADR 0014.** On Chromium, a service worker's own requests are judged as a page's subresources are. On Firefox and WebKit, where they cannot be judged, service workers never register.
- **Amends ADR 0035.** A `stealth` Camoufox Session never registers a service worker, a difference from real Firefox that a site can see. Firefox's own `dom.serviceWorkers.enabled` preference refuses it, never a page-visible stub. A note sits under its Firefox engine bullet.
- **Amends ADR 0056.** A Profile directory used in place that holds a service worker registration `block` cannot keep from running, as on WebKit, is refused when its Session opens. A note sits under its WebKit bullet.
- **Amends ADRs 0019 and 0021.** On WebKit, `block` is Playwright's `register` stub, a main-world injection marked in Provenance as ADR 0019's are, so a `standard` WebKit Session carries it. Notes sit under ADR 0019's main-world bullet and among ADR 0021's notes.
- **Adds to ADR 0047.** Filter lists cover a service worker's own requests on Chromium. A request the service worker answers from its own cache reaches no network and needs no blocking.
- **Adds to ADR 0046.** The Response cache acts at interception, so a page request a site's service worker answers is outside it. Its docs say to set `browser.service_workers: block` when that matters.
- **Amends ADR 0110.** Kept responses match the responses the engine observes through the browser protocol's network events, not only those its interception pauses, so they keep a response a service worker's fetch handler supplied, marked by `fromServiceWorker()`. A note sits under `fetch.kept_responses.urls`.
- **Amends ADRs 0087 and 0086.** The navigation gate counts what reaches the site. A service worker's request that carries a page's navigation to the network takes that navigation's permit, though the engine sees it on the service worker's target, not the page's. A navigation the service worker answers from its own cache sends nothing to the site and takes none. `goBack` and `goForward` always reach the gate as navigations. Notes sit under ADR 0087's navigation gate and beside ADR 0086's note from ADR 0087.
- **Error codes.** None is new. `capability_unavailable` (ADR 0083) refuses an explicit `allow` on Firefox and WebKit.
- **Who builds what.** Each area lists this ADR.
  - **Area 5 (Site access)** builds:
    - the service-worker auto-attach and its interception, and the observed responses kept responses match;
    - `block`'s refusal: the mechanism inside Chromium, Firefox's preference, WebKit's stub and its Provenance mark, and the proof that an already registered service worker controls no page;
    - the navigation gate's permit for a service worker's `navigate` request, held until the page's load event;
    - `page.request` sent so that no service worker answers it, never as a `fetch()` from a page a service worker controls;
    - the refusal of a Profile directory used in place whose service worker registration `block` cannot keep from running;
    - the back/forward cache switches, and the prerendering test;
    - closed-root resolution in the isolated world, for locators, the guard and `ariaSnapshot`;
    - the `observed` text on Firefox and WebKit.
  - **Area 4 (Security and egress)** checks that the domain policy holds over service workers, attached before they run on Chromium and never registered on Firefox and WebKit.
  - **Area 6 (Collection)**, if it adds CO-17's infinite-scroll helper, has it collect rows by a key at each step.
  - **Area 8 (Stealth and Challenge solving)** measures what a `stealth` Camoufox Session loses without service workers, refused there by Firefox's preference.
  - **Area 9 (AI and agent)** builds the `ai` snapshot's count line.
  - **Area 11 (Client surfaces)** builds `fromServiceWorker()` and the setting per language, and the doc text of `route`, `extract`, `locator`'s cross-browser warning and the virtualized-list recipe.
  - **Area 2 (Proving ground and conformance)** builds the four pages.
- **ADR 0037's matrix.**
  - SA-15 becomes `decided`, since its last `spec` part, closed shadow roots, is decided here.
  - SA-14, SA-16, SA-17 and CO-17 cite this ADR.
  - The status counts follow.
- **`TESTING.md`** gains the page state conformance cases. The virtualized-list recipe runs as a docs code sample.
- **The `prototype/sdk-api` sketch** holds `browser.service_workers` and `fromServiceWorker()` in each language. Its pitfall rows B10, B13, B20 and B21 close as Handled, and its pain row P34 names this answer.
