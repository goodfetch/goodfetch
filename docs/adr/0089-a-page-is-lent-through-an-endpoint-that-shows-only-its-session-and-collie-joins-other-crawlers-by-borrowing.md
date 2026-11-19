# A page is lent through an endpoint that shows only its Session, and Collie joins other crawlers by borrowing their pages

Collie and Foxhound meet third-party automation in two directions. **Out:** a Foxhound handler, or a Collie program, lends its page to Playwright, Puppeteer or Browser Use. The page becomes a **Lent page**, reached through a CDP endpoint that plays a browser holding only that page's Session. **In:** a Collie program borrows a page that Crawlee, Scrapy or another crawler opened, through `run.borrow()` (ADRs 0056 and 0078). No third-party library ever replaces the engine's driver: Foxhound offers no Crawlee-style driver adapter.

The research found that neither direction works as the earlier ADRs assumed ([driver interop](https://github.com/goodfetch/goodfetch/blob/research/driver-interop/rnd/research/2026-10-14-driver-interop.md), cited DI; [crawl frameworks](https://github.com/goodfetch/goodfetch/blob/research/crawl-framework-products/rnd/research/2026-10-14-crawl-framework-products.md), cited CF):
- **The libraries cannot be lent one page.** Given a page endpoint, Playwright's `connectOverCDP` connects and sees no pages, and Puppeteer's `connect` fails with `Target.getBrowserContexts: Not allowed`. On a browser endpoint, Playwright adopts every page of every browser context (DI §1, measured). A run's Sessions share one browser process as contexts unless their Fingerprint method is per process (ADR 0037's SA-09, ADR 0040), so a browser endpoint would lend every Session in it, cookies included.
- **A second client can break the owner.** Puppeteer's `close()` sends `Browser.close`, and Chrome exited; Stagehand v4's and chromiumoxide's send it too, by their source. A context created with its own `proxyServer` bypassed the browser's launch proxy. `Extensions.loadUnpacked` installed an extension whose tabs and traffic no relay sees. Detaching cleared the owner's device emulation, and Playwright's defaults replaced its download folder (DI §1, §6, measured). Chrome offers no reduced-trust mode for a WebSocket client (DI §3.1).
- **No crawler hands its handler an endpoint bound to its page,** and the one first-party package that pairs a crawler with an automation library, Crawlee's Stagehand crawler, inverts the relationship: Stagehand launches the browser and Crawlee attaches, losing launch options and fingerprints. Both Crawlee languages are still held at Stagehand v3 (`@crawlee/stagehand` 4.0.0-rc.1 peers `^3.0.0`; `crawlee[stagehand]` 1.10.4 requires `stagehand<4.0.0`), while Stagehand v4 has "no interop" with Playwright pages (CF §1, §4).

Decided in [Third-party automation on Foxhound pages, and Collie inside other crawlers](https://github.com/goodfetch/goodfetch/issues/456) ([grilling record](https://github.com/goodfetch/goodfetch/issues/456#issuecomment-6179774145)), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/goodfetch/issues/447), with the agent answering on the owner's delegation.

## Decision

### A Lent page

- **The endpoint shows a browser with one context, the lent page's Session.** In it, the library sees the lent page, the pages the library itself creates, and the targets opened from either (popups, frames, workers). It never sees another Session or the Session's other pages, including pages the program opens during the attach. In a handler, the Session is already reserved for the attach (ADR 0003); in a Collie program, a second `page.lend()` on a Session while one attach is open is refused with `session_reserved` too, so one attach's end never restores state under another. This extends the scoping that [Collie's page API](https://github.com/goodfetch/goodfetch/issues/472) gives `page.newCdpSession()`, which reaches only its page's target, to a library that needs a whole browser's endpoint.
- **Every command is classed:**
  - **Passed:** commands on the pages and targets the library sees.
  - **Rescoped:** a command with an optional browser context (permissions, cookies, `Target.createTarget`) gets the Session's context when it names none, and naming another is refused. Target discovery and auto-attach show and attach only the pages and targets the library sees.
  - **Ending the attach:** `Browser.close` is answered and closes that library's connection, never the browser, as Playwright's own `close()` does on a connected browser.
  - **Answered without effect:** a download-behaviour command. Downloads on a Lent page stay the Run's (ADR 0037's CO-22), so a library never chooses where the browser writes files.
  - **Refused,** as a CDP error naming `cdp_command_refused`, the code Collie's page API gives its CDP session:
    - creating or disposing a browser context, which could carry its own proxy;
    - attaching to the browser target or to a target outside the Session, and `Target.exposeDevToolsProtocol`;
    - the `Extensions` domain, which installs code that drives tabs out of the endpoint's sight;
    - crashing the browser;
    - ignoring certificate errors (TLS verification is never switchable, ADRs 0016 and 0042).
  - **Refused on a Daemon with accounts (an Exposed Daemon or a Daemon group included) and in Goodfetch Cloud,** where the browser's machine also serves people other than the program's own: commands that read a file on the browser's machine by path, such as an upload from a path or a `file:` load, as ADR 0056 refuses a server path on a Daemon with accounts.
  - **Fail closed:** a browser-level command the spec's table does not class is refused. A Live check on each supported library's newest release catches a library that starts sending a new one.
- **Closing the lent page itself is allowed,** as it is for a CDP session. The program's Page then reports `closed` (ADR 0078).
- **The endpoint is a WebSocket for every library and engine,** under ADR 0017's rules: loopback by default, `Host` and `Origin` checked, a token for each attach, and TLS when exposed. A token for each attach narrows ADR 0017's per-Session token for every lend, as ADR 0003 already does for a handler's attach. The token travels in a header, never in the address, as ADR 0067's endpoint table requires; Playwright, Puppeteer and Browser Use all send headers when they connect. Python's sync client needs no relay thread, because the library connects from the program's own thread.
- **When the attach ends,** by `close()`, the library's `Browser.close`, its last connection closing, the lease (ADR 0003), the page closing or the Run ending:
  - the engine closes the pages the library created and the popups opened from them; popups opened from the lent page stay Pages of the Session, as any popup does;
  - it re-applies the Session's and the page's own state (device emulation, permissions, the Identity profile's overrides) before any other use, because when a CDP session detaches, Chromium clears device emulation, whoever set it, and the permission overrides that CDP session made (DI §3.2);
  - the token is revoked, so Browser Use's automatic reconnects after a dropped connection (three retries with backoff, while its actions wait up to 54 s) are refused, and the program lends the page again.

### What a Lent page keeps and loses

| | While a library is attached |
|---|---|
| Guard | The library's actions never pass it. The program's own page calls still work; their results are marked unguarded and no Secret is typed (ADR 0017). |
| Fencing | In a handler, the attach is bound to the work item's lease and cut at its end with `attach_ended` (ADR 0003). The cut also releases requests and child targets the library paused, so the lease bounds a stall. Commands sent before the cut are unfenced. |
| Session | In a handler, reserved for the attach (ADR 0003); out of the Session pool afterwards unless saved explicitly (ADR 0017); its state re-applied when the attach ends. |
| Stealth | The Fingerprint method stays, but no Access mode keeps `standard`'s promise that `Runtime` is never enabled (ADR 0021). Playwright, Puppeteer, chromiumoxide and chromium-bidi enable `Runtime` on the pages they wrap, and the first three add named utility worlds (DI §3.3). Human-like input applies only to the engine's own actions. Records taken during the attach say so in Provenance, and a Saved identity saved from a Session that was lent records it. |
| Egress | On a browser the engine or a Browser host launched, ADR 0017's egress proxy stays the Session's only way out: refusing new contexts and extensions closes both bypasses the research measured. On any other browser (a Borrowed browser, a third-party cloud browser), egress stays interception only, which the library can switch off, with the rebinding gap recorded (ADRs 0014 and 0056). A library's own HTTP client, such as Playwright's `request`, sends from the program's process with the Session's cookies, outside the navigation gate, robots.txt, the domain policy and the egress proxy, and from another exit than the Session's; the guides send such calls through Collie's `page.request` instead. |
| AI safety | ADR 0014's enforcement now covers the engine's own AI only. A library's own model, such as Browser Use's, gets no Effect confirmation and no untrusted-content wrapping, and the library can read the Session's cookies, as any raw CDP client can (ADR 0017). Only the domain policy still holds below it, as the Egress row says: below the browser on a browser the engine or a Browser host launched, by interception alone elsewhere. |
| Politeness | In a crawl handler, the library's navigation is the crawl's, as the handler's own is (ADR 0086). ADR 0087's navigation gate, consulted at the engine's interception for each main-frame navigation whoever starts it, keeps gating the lent page and the pages opened from it, and the pages the library creates carry it too. So each navigation is admitted once, and robots.txt refuses a disallowed one there, as for the handler's own navigation. Where interception no longer sees the library's requests (ADR 0047's case), they go ungated, and the Run trace says so. The library's own HTTP client is never gated (Egress row). In a Collie program, outside any crawl handler, the library's navigation is outside the navigation gate, as the program's own is (ADRs 0086 and 0087). |
| Ad blocking, consent | As ADRs 0047 and 0048 rule for any raw CDP client. |
| Effect journal, Plans | The library's actions are not journaled and never become Plans. |
| Usage, Limits | The library's own model calls, such as Browser Use's, are outside Usage and Limits (ADR 0041); the docs send AI work that must be counted to Collie's own verbs. |
| Run trace | Each attach appears with its start, its end and reason, and every refused command (G6). |

> Amended by [ADR 0114](0114-a-program-and-collie-take-turns-on-a-page-other-code-drives-and-an-ai-verb-fails-when-a-navigation-it-did-not-start-replaces-its-document.md): while a library is attached, the program awaits each of its own Collie calls before the library acts, and an AI verb of the program's whose document a navigation it did not start replaces fails with `page_navigated_during_call`.

> Note (owner's decision, 2026-11-11, grilling of decision G-15 of the consolidated review: Q2 answered "Yes, all four" on the grilling's recommended option as put; the wording is the grilling session's, and the owner did not read it): the Stealth row above is amended: a `stealth` Session's page is lent only when its setting allows `Runtime`, as [ADR 0021](0021-standard-access-by-default-and-escalation-by-new-session.md)'s note of that day says.

### Supported libraries

- **Supported, each with a helper, a guide and conformance cases:** Playwright in TypeScript and Python, Puppeteer in TypeScript, Browser Use in Python.
- **Not supported:**
  - Stagehand v4, which needs a whole browser and installs its extension with a shared file system (DI §2.3); its load is refused, and its users have Collie's `act`, `observe` and `extract` and its built-in Agent (ADR 0013);
  - Stagehand v3, as ADR 0037 offers no v3 layer;
  - Selenium, ChromeDriver and WebDriver BiDi: ChromeDriver attached to a running browser leaves "some commands" unsupported, BiDi on Chrome runs chromium-bidi, which enables `Runtime`, and Firefox allows one BiDi session (DI §2.6);
  - Rust CDP crates: chromiumoxide's `close()` kills the browser, and a Rust program has Collie's page API and its CDP session.
- **Any other CDP library** reaches the same endpoint under the same rules, untested and outside support.
- **Firefox and WebKit pages,** which run through Playwright's driver and expose no CDP (ADR 0019), cannot be lent: `page.lend()` raises `capability_unavailable` (ADR 0083) there, as a CDP session does in Collie's page API.
- **The Playwright guides send a program's HTTP calls through Collie's `page.request`,** which the navigation gate covers in a handler (ADR 0087), since Playwright's own `request` sends from the program's process.
- **Each supported range is stated in the release notes,** as ADR 0077's G7 does for Pydantic. Its floor is the oldest release with the options the helper sets: Playwright 1.60 for `noDefaults`.
- **Conformance cases** run each supported library through its helper against the binary of the same commit (ADR 0069): its newest pinned release in the Pre-merge suite when touched, both ends of its range in the Release suite, and its newest release as a Live check ([`TESTING.md`](../testing/sdk-and-cli-tests.md#sdks)).

### In code

- **`page.lend()` returns a `CdpEndpoint`:** its `url`, the `headers` that carry its token, and `close()`, with ADR 0078's scoped forms. It is the same in TypeScript, Python and Rust (S1).
- **One helper per supported library:**
  - It takes the program's own library object (`chromium`, `puppeteer`, Browser Use's session class), so no SDK depends on Playwright, Puppeteer or Browser Use (G7).
  - It lends the page and connects with the safe options: Playwright `noDefaults: true`, Puppeteer `defaultViewport: null`, Browser Use an empty `permissions` list, which skips its grant in 0.13.11 (`permissions_watchdog.py`); the endpoint's rescoping stays the guarantee for other releases.
  - It returns that library's object for the lent page, ends the lending when that object is closed, and raises `attach_ended` as a typed error when the attach is cut (ADR 0003).
  - Its per-language names are [Mapping the catalog to each language and surface](https://github.com/goodfetch/goodfetch/issues/471)'s (ADR 0099).
- **Only the SDKs offer lending.** An endpoint is useful only to a library in the same program, and there the program's own code chooses the library, as running the code is the approval (ADR 0056). On an MCP profile, a Framework adapter, an Integration, a skill or in Code mode, a model would choose, and get a browser outside ADR 0014's enforcement; the CLI holds no page to lend.
- **Lending belongs to the page,** so Collie carries it and Foxhound's handlers inherit it.
  - In a handler, the attach is bound to the work item's lease (ADR 0003).
  - In a Collie program, outside any work item, it lasts until it is closed, its page closes or its Run ends, under its token's lifetime (ADR 0017).
  - A hosted page lends through the Browser host's endpoint, with the same scoping, under ADR 0022's opt-in for a raw CDP endpoint on a hosted browser (ADR 0003). A page of a Borrowed browser lends under the same rules, and nothing closes a Borrowed browser (ADR 0056).

### Collie inside other crawlers

- **`run.borrow(page)` is the one seam.** Inside a crawler's handler, it turns the crawler's Playwright or Puppeteer page into a Goodfetch Page with Collie's page API, AI verbs and Extraction. A crawler that holds only a CDP address uses `cdp_url` (ADR 0056).
- **The pattern** is one Collie Run per crawl, opened beside the crawler, borrowing each handler's page in the scoped form so the borrow ends with the handler.
- **Each browser context a borrowed page belongs to is one Session of the Run,** joined by later pages of that context. Crawlee's pool runs several browsers and, with incognito pages, a context per page; scrapy-playwright opens named contexts in one browser. A second run borrowing the same browser is still refused with `browser_borrowed`. When the crawler closes a page or retires its browser, the borrow ends and the Page reports `closed`.
- **What Collie keeps:** the Effect journal and Run trace for its own actions, Plans and their replay, Success checks and Verdicts (ADR 0013), the engine's AI safety on its own actions as far as interception reaches (ADR 0014), Usage and Limits on its model calls, and Extraction.
- **What it loses** is ADR 0056's: the guard's guarantee (results unguarded, since the crawler's driver holds the page too), typing Secrets, egress enforced below the browser (interception only, with the rebinding gap recorded), stealth (the Session is `standard`, with the crawler's browser's identity), the Session pool, and resuming on another machine.
  > Amended by [ADR 0114](0114-a-program-and-collie-take-turns-on-a-page-other-code-drives-and-an-ai-verb-fails-when-a-navigation-it-did-not-start-replaces-its-document.md): the program and Collie take turns on a borrowed page, and an AI verb whose document a navigation it did not start replaces, such as the crawler's own navigation during an `extract`, fails with `page_navigated_during_call`.
- **Retries stay the crawler's:** Collie sees a retried handler as new calls in the same Run. How a program names its own Effects is [Effects a program declares in code](https://github.com/goodfetch/goodfetch/issues/500)'s.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): on a Run whose browser is borrowed, a key declared again after its answer returns that answer (`journal` or `reconciled`) instead of raising `effect_key_reused`, so a retried handler finds the Effect it already performed; the docs tell such a program to put the crawler's request key in the Effect key.
- **Guides with tested examples:**
  - Crawlee for JavaScript (`PlaywrightCrawler` and `PuppeteerCrawler`), Crawlee for Python (`PlaywrightCrawler`), and Scrapy with scrapy-playwright (`playwright_include_page`), each run at a pinned crawler version.
  - All three run handlers asynchronously, so they use the async clients, and ADR 0056's open question on Python's sync relay does not block them.
  - They cover Chromium pages only: `run.borrow()` refuses a Firefox or WebKit page with `borrow_requires_cdp` (ADR 0056).
  - Crawl4AI's page hooks and any other program holding a Playwright or Puppeteer page reach Collie the same way, untested.

## Considered Options

- **A Crawlee-style driver adapter,** where a library launches the browser and Foxhound's handlers drive through that library's page: rejected.
  - The crawl would run without the guard, the Effect journal, the Goodfetch Chromium or the egress proxy, in the SDK's process, where a Daemon or Goodfetch Cloud cannot reach it.
  - It reopens ADR 0019's rejection of Playwright as the driver and ADR 0033's rejection of a browser backend that launches what it names. Every browser source a user has already comes in as a Borrowed browser or through a browser backend Plugin.
  - Crawlee's Stagehand crawler shows the cost: "low-level browser properties cannot be modified", five launch options pass through, and in Python it applies no canvas, WebGL or screen fingerprint (CF §1, §4).
- **A raw browser endpoint, as ADR 0019 first read:** rejected. It lends every Session in the browser process.
- **A dedicated browser process for each lend:** rejected. It costs a browser launch and its memory per lend, and `Browser.close` and the proxy bypass still need the same handling.
- **Passing every command and only recording it:** rejected. Puppeteer's `close()` would end the browser, and a new browser context or an extension would bypass the egress proxy, before any record is read (DI §6, P7–P9).
- **The bare endpoint for every library, with no helpers:** rejected. Each user would have to find the safe options alone (`noDefaults`, `defaultViewport: null`, an empty `permissions` list), which every library gets wrong by default (DI §2.8).
- **Leaving a Lent page's navigation outside Politeness:** rejected. Eight handlers lending their pages to Playwright would press a site eight times over its limit, the case ADR 0086 rejected for a handler's own navigation.
- **Gating only `Page.navigate` at the endpoint:** rejected. A click or a script navigates without it; ADR 0087's navigation gate at the engine's interception sees every main-frame navigation.
- **Refusing every browser-level command:** rejected. Playwright's own connect sends several (`Browser.getVersion`, `Target.setAutoAttach`, `Browser.setDownloadBehavior`), so it could not connect.
- **CDP carried over the SDK's own connection,** through the transport Playwright's Node `connectOverCDP` accepts since 1.61: rejected.
  - It is a second path to test, and the overload is typed only in `overrides.d.ts`, not in the API docs, and was once removed ([PR #40972](https://github.com/microsoft/playwright/pull/40972) restored it after #40195).
  - It works only where the engine shares the program's machine: it forces `isLocal`, which assumes a shared file system, and a Daemon elsewhere or Goodfetch Cloud has none.
  - Python's `connect_over_cdp` and Browser Use take only an address.
- **Stagehand v4 on a dedicated browser with its extension allowed:** rejected. It would install a third-party extension with the `debugger` permission into the engine's browser, out of the endpoint's sight, for verbs Collie already has.
- **Lending on every generated surface:** rejected. A CDP address handed to a model lets it drive a browser outside the engine's AI safety (ADR 0014).
- **Crawler-specific packages** (a `CollieCrawler` class, a Scrapy package): rejected.
  - Each would couple a Goodfetch package to a crawler's releases. Crawlee's own Stagehand crawlers show the cost: held at Stagehand v3 in both languages, and Crawlee for Python postponed the port "until v2".
  - ADR 0027 rejects hand-written per-tool Integrations, and `run.borrow()` is one call.
- **Collie as the crawler's browser provider** (Crawlee v4's `RemoteBrowserPool`, scrapy-playwright's `PLAYWRIGHT_CDP_URL` or `PLAYWRIGHT_BROWSER_PROVIDER`): rejected.
  - `RemoteBrowserPool` and `PLAYWRIGHT_CDP_URL` create browser contexts on the endpoint, which a Lent page refuses.
  - `PLAYWRIGHT_BROWSER_PROVIDER` wants Playwright-compatible `Browser`, `BrowserContext` and `Page` objects in the crawler's process, which Collie's Playwright-shaped page API does not claim to be (ADR 0019).
  - Crawlee skips its own fingerprints on a remote browser anyway.
  - Foxhound is the Product for a crawl in the engine's own browser.
- **One Session per borrowed browser, refusing a second context:** rejected. It refuses Crawlee's incognito pages and scrapy-playwright's named contexts, the crawlers' ordinary setups.
- **A new Run per borrowed context:** rejected. It splits one crawl's AI work, Usage and Run trace across Runs.

## Consequences

- **Amends ADR 0003.** Its rejected per-message lease check does not cover scoping: the endpoint of a Lent page parses each message to keep it inside the Session, while the lease is still enforced by cutting the attach. A note sits under that option.
- **Amends ADR 0014.** Its enforcement covers the engine's own AI: a library attached to a Lent page, a model-driven one included, acts outside it except for the domain policy. A note sits under its opening.
- **Amends ADR 0017.** A Lent page's endpoint refuses new browser contexts, extensions and the commands above, keeps downloads the Run's, and gives every lend its own token, narrowing the per-Session token; the engine re-applies the Session's state when the attach ends. A note sits under its raw CDP consequence.
- **Amends ADR 0019.** Pages are lent to Playwright, Puppeteer or Browser Use. The endpoint lends one page, shows only its Session, and stays a WebSocket by choice: Playwright's Node `connectOverCDP` can now take a transport, but Python and engines on another machine need an address. A note sits under its consequence on Playwright and Puppeteer code.
- **Amends ADR 0021.** While a library is attached to a Lent page, `standard` no longer keeps `Runtime` off on the pages it wraps; Provenance records the attach. A note sits under its opening.
- **Amends ADR 0037's matrix.** SA-07, SA-08, RT-21 and pain 12 cite this ADR. Areas 4 (Security and egress), 5 (Site access) and 11 (Client surfaces) list it.
- **Amends ADR 0040.** A Borrowed browser still serves one run, but through `run.borrow()` it can serve several of its Sessions, one per borrowed browser context. A note sits under its "Never across runs" item.
- **Amends ADR 0056.** Through `run.borrow()`, each browser context a borrowed page belongs to is one Session of the run. A note sits under its Borrowed browsers section's opening.
- **Amends ADR 0070.** Its Quick check over generated surfaces also refuses `page.lend` on every MCP profile, Framework adapter, Integration and skill, and Code mode never offers it. A note sits under that item.
- **Amends ADR 0084.** What a Lent page keeps and loses is decided here. A note sits under its Foxhound item.
- **Amends ADR 0086.** It left to this ticket whether a Lent page's navigation stays counted: in a crawl handler it does, through ADR 0087's navigation gate, admitted once; a library's own HTTP client, sending from the program's process, is the one fetch it cannot count. A note sits under its "Left to other tickets" item.
- **Amends ADR 0087.** It left to this ticket what the navigation gate does on a lent page: it keeps gating, the pages the library creates carry it too, and where interception no longer sees the library's requests they go ungated and the Run trace says so; a library's own HTTP client never reaches the gate. A note sits under that item.
- **Adds to ADR 0064:** two Live checks owned by area 11, each blocking nothing directly: `lend-libraries-newest`, each supported library's newest release on a Lent page, and `crawler-examples-newest`, the crawler guides' examples on each crawler's newest release. Both run nightly, or before each release candidate while the nightly tier is off. `TESTING.md`'s list of Live checks names them.
- **Adds to ADR 0067:** a Lent page layer in its SSRF table, whose client also tries a browser context with its own proxy and an extension.
- **`GLOSSARY.md`** gains **Lent page**.
- **`TESTING.md`** gains the lending conformance cases, the crawler guides' examples, the two Live checks, the Lent page's layer of the SSRF table and the Quick check's refusal of `page.lend` on generated surfaces.
- **Areas.**
  - **Area 4** builds the refusals that keep files, contexts and extensions on the engine's side, and the Lent page's layer of the SSRF table, whose client also tries a browser context with its own proxy and an extension (ADR 0067).
  - **Area 5** builds the endpoint's scoping, its command table, the attach's end, the state re-applied, and ADR 0087's navigation gate on the pages a library creates.
  - **Area 11** builds `page.lend()`, the helpers, their conformance cases, the guides and the crawler examples.
- **Other tickets.**
  - [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455) lists lending as part of the page, and settles V3-01, AI verbs on a crawler's page.
  - [Crawl definition and handler context](https://github.com/goodfetch/goodfetch/issues/477) (ADR 0093) hands its handlers a page that offers `lend()`.
  - [Docs site and positioning of two Products](https://github.com/goodfetch/goodfetch/issues/499) (ADR 0097) places the lending and crawler guides beside the migration guides.
- **To prove in the spec:**
  - the endpoint's command table, including which page-level commands reach other Sessions (a browser-wide cache or cookie clear);
  - that auto-attach can follow the lent page's tree alone, through Chromium's experimental `Target.autoAttachRelated` (DI §3.1) or the endpoint's own filtering;
  - which commands read files on the browser's machine;
  - what each library's own download API reports when its download-behaviour command has no effect, so the guides point to the Run's downloads;
  - lending a page borrowed through `run.borrow(page)`, which needs a browser-level relay session over the program's connection;
  - the error a library sees when the navigation gate refuses its navigation, and how a wait for a permit meets the library's own navigation timeout;
  - that no passed command removes the engine's interception;
  - each supported library's connect, close and reconnect against the endpoint, at its floor and its newest release, and how each helper sends the token (Puppeteer 25.13 deprecates its top-level `headers` for `wsOptions.headers`).
