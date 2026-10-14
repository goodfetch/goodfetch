# A page is lent through an endpoint that shows only its Session, and Collie joins other crawlers by borrowing their pages

Goodfetch meets third-party automation in two directions. **Out:** a Foxhound handler, or a Collie program, lends its page to Playwright, Puppeteer or Browser Use. The page becomes a **Lent page**, reached through a CDP endpoint that plays a browser holding only that page's Session. **In:** a Collie program borrows a page that Crawlee, Scrapy or another crawler opened, through `run.borrow()` (ADRs 0056 and 0078). No third-party library ever replaces the engine's driver: Foxhound offers no Crawlee-style driver adapter.

The research found that neither direction works as the earlier ADRs assumed ([driver interop](https://github.com/goodfetch/automan/blob/research/driver-interop/rnd/research/2026-10-14-driver-interop.md), cited DI; [crawl frameworks](https://github.com/goodfetch/automan/blob/research/crawl-framework-products/rnd/research/2026-10-14-crawl-framework-products.md), cited CF):
- **The libraries cannot be lent one page.** Given a page endpoint, Playwright's `connectOverCDP` connects and sees no pages, and Puppeteer's `connect` fails with `Target.getBrowserContexts: Not allowed`. On a browser endpoint, Playwright adopts every page of every browser context (DI §1, measured). A run's Sessions share one browser process as contexts (ADR 0037's SA-09), so a browser endpoint would lend every Session in it, cookies included.
- **A second client can break the owner.** Puppeteer's, Stagehand v4's and chromiumoxide's `close()` sends `Browser.close`, and Chrome exited. A context created with its own `proxyServer` bypassed the browser's launch proxy. `Extensions.loadUnpacked` installed an extension whose tabs and traffic no relay sees. Detaching cleared the owner's device emulation, and Playwright's defaults replaced its download folder (DI §1, §6, measured). Chrome offers no reduced-trust mode for a WebSocket client (DI §3.1).
- **No crawler hands its handler an endpoint bound to its page,** and the one first-party package that pairs a crawler with an automation library, Crawlee's Stagehand crawler, inverts the relationship: Stagehand launches the browser and Crawlee attaches, losing launch options and fingerprints. Both Crawlee languages are still held at Stagehand v3 (`@crawlee/stagehand` 4.0.0-rc.1 peers `^3.0.0`; `crawlee[stagehand]` 1.10.4 requires `stagehand<4.0.0`), while Stagehand v4 has "no interop" with Playwright pages (CF §1, §4).

Decided in [Third-party automation on Foxhound pages, and Collie inside other crawlers](https://github.com/goodfetch/automan/issues/456) ([grilling record](https://github.com/goodfetch/automan/issues/456#issuecomment-6179774145)), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447), with the agent answering on the owner's delegation.

## Decision

### A Lent page

- **The endpoint shows a browser with one context, the lent page's Session.** In it, the library sees the lent page and the pages opened while it is attached, never another Session or the Session's earlier pages. The Session is already reserved for the attach (ADR 0003). This is the scoping [Collie's page API](https://github.com/goodfetch/automan/issues/472) gives `page.newCdpSession()`, served to a library that needs a whole browser's endpoint.
- **Every command is classed:**
  - **Passed:** commands on the lent page's targets and the targets it opens.
  - **Rescoped:** a command with an optional browser context (permissions, cookies, `Target.createTarget`) gets the Session's context when it names none, and naming another is refused. Target discovery and auto-attach show only the Session's context, and attach the library only to the lent page and the targets opened from it.
  - **Ending the attach:** `Browser.close` is answered and closes that library's connection, never the browser, as Playwright's own `close()` does on a connected browser.
  - **Answered without effect:** a download-behaviour command. Downloads on a Lent page stay the Run's (ADR 0037's CO-22), so a library never chooses where the browser writes files.
  - **Refused,** as a CDP error naming `cdp_command_refused`, the code Collie's page API gives its CDP session:
    - creating or disposing a browser context, which could carry its own proxy;
    - attaching to the browser target or to a target outside the Session, and `Target.exposeDevToolsProtocol`;
    - the `Extensions` domain, which installs code that drives tabs out of the endpoint's sight;
    - crashing the browser;
    - ignoring certificate errors (TLS verification is never switchable, ADRs 0016 and 0042).
  - **Refused where the program does not share the browser's machine** (a Daemon with accounts, an Exposed Daemon, Goodfetch Cloud): commands that read a file there by path, such as an upload from a path or a `file:` load, as ADR 0056 refuses a server path on a Daemon with accounts.
  - **Fail closed:** a browser-level command the spec's table does not class is refused. A Live check on each supported library's newest release catches a library that starts sending a new one.
- **Closing the lent page itself is allowed,** as it is for a CDP session. The program's Page then reports `closed` (ADR 0078).
- **The endpoint is a WebSocket for every library and engine,** under ADR 0017's rules: loopback by default, `Host` and `Origin` checked, a token for each attach, and TLS when exposed. The token travels in a header, never in the address, as ADR 0067's endpoint table requires; Playwright, Puppeteer and Browser Use all send headers when they connect. Python's sync client needs no relay thread, because the library connects from the program's own thread.
- **When the attach ends,** by `close()`, the lease (ADR 0003) or the page closing:
  - the engine closes the pages the library opened;
  - it re-applies the Session's and the page's own state (device emulation, permissions, the Identity profile's overrides) before any other use, because when a CDP session detaches, Chromium clears device emulation, whoever set it, and the permission overrides that CDP session made (DI §3.2);
  - the token is revoked, so Browser Use's automatic reconnects (three, over up to 54 s) are refused.

### What a Lent page keeps and loses

| | While a library is attached |
|---|---|
| Guard | The library's actions never pass it. The program's own page calls still work; their results are marked unguarded and no Secret is typed (ADR 0017). |
| Fencing | The attach is bound to the work item's lease and cut at its end with `attach_ended` (ADR 0003). The cut also releases requests and child targets the library paused, so the lease bounds a stall. Commands sent before the cut are unfenced. |
| Session | Reserved for the attach (ADR 0003); out of the Session pool afterwards unless saved explicitly (ADR 0017); its state re-applied when the attach ends. |
| Stealth | The Fingerprint method stays. Playwright, Puppeteer, chromiumoxide and chromium-bidi enable `Runtime` and add named utility worlds on the pages they wrap (DI §3.3), and human-like input applies only to the engine's own actions. Records taken during the attach say so in Provenance, and a Saved identity saved from a lent Session records that it was lent. |
| Egress | ADR 0017's egress proxy stays the Session's only way out: refusing new contexts and extensions closes both bypasses the research measured. |
| Ad blocking, consent | As ADRs 0047 and 0048 rule for any raw CDP client. |
| Effect journal, Plans | The library's actions are not journaled and never become Plans. |
| Usage, Limits | The library's own model calls, such as Browser Use's, are outside Usage and Limits (ADR 0041); the docs send AI work that must be counted to Collie's own verbs. |
| Run trace | Each attach appears with its start, its end and reason, and every refused command (G6). |

### Supported libraries

- **Supported, each with a helper, a guide and conformance cases:** Playwright in TypeScript and Python, Puppeteer in TypeScript, Browser Use in Python.
- **Not supported:**
  - Stagehand v4, which needs a whole browser and installs its extension with a shared file system (DI §2.3); its load is refused, and its users have Collie's `act`, `observe`, `extract` and `agent`;
  - Stagehand v3, as ADR 0037 offers no v3 layer;
  - Selenium, ChromeDriver and WebDriver BiDi: ChromeDriver attached to a running browser leaves "some commands" unsupported, BiDi on Chrome runs chromium-bidi, which enables `Runtime`, and Firefox allows one BiDi session (DI §2.6);
  - Rust CDP crates: chromiumoxide's `close()` kills the browser, and a Rust program has Collie's page API and its CDP session.
- **Any other CDP library** reaches the same endpoint under the same rules, untested and outside support.
- **Firefox and WebKit pages,** which run through Playwright's driver and expose no CDP (ADR 0019), cannot be lent: `page.lend()` raises `capability_unavailable` (ADR 0083) there, as a CDP session does in Collie's page API.
- **Each supported range is stated in the release notes,** as ADR 0077's G7 does for Pydantic. Its floor is the oldest release with the options the helper sets: Playwright 1.60 for `noDefaults`.

### In code

- **`page.lend()` returns a `CdpEndpoint`:** its `url`, the `headers` that carry its token, and `close()`, with ADR 0078's scoped forms. It is the same in TypeScript, Python and Rust (S1).
- **One helper per supported library:**
  - It takes the program's own library object (`chromium`, `puppeteer`, Browser Use's session class), so no SDK depends on Playwright, Puppeteer or Browser Use (G7).
  - It lends the page and connects with the safe options: Playwright `noDefaults: true`, Puppeteer `defaultViewport: null`, Browser Use an empty `permissions` list. The research could not confirm that Browser Use's empty list skips its grant, so the endpoint's rescoping stays the guarantee.
  - It returns that library's object for the lent page, ends the lending when that object is closed, and raises `attach_ended` as a typed error when the attach is cut (ADR 0003).
  - Its per-language names are [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)'s.
- **Only the SDKs offer lending.** An endpoint is useful only to a library in the same program. On an MCP profile, a Framework adapter, an Integration, a skill or in Code mode it would hand a model a browser outside ADR 0014's enforcement, and the CLI holds no page to lend.
- **Lending belongs to the page,** so Collie carries it and Foxhound's handlers inherit it.
  - In a handler, the attach is bound to the work item's lease (ADR 0003).
  - In a Collie program, outside any work item, it lasts until it is closed, its page closes or its Run ends, under its token's lifetime (ADR 0017).
  - A hosted page lends through the Browser host's endpoint (ADRs 0003 and 0022), and a page of a Borrowed browser under the same rules; nothing closes a Borrowed browser (ADR 0056).

### Collie inside other crawlers

- **`run.borrow(page)` is the one seam.** Inside a crawler's handler, it turns the crawler's Playwright or Puppeteer page into a Goodfetch Page with Collie's page API, AI verbs and Extraction. A crawler that holds only a CDP address uses `cdp_url` (ADR 0056).
- **The pattern** is one Collie Run per crawl, opened beside the crawler, borrowing each handler's page in the scoped form so the borrow ends with the handler.
- **Each browser context a borrowed page belongs to is one Session of the Run,** joined by later pages of that context. Crawlee's pool runs several browsers and, with incognito pages, a context per page; scrapy-playwright opens named contexts in one browser. A second run borrowing the same browser is still refused with `browser_borrowed`. When the crawler closes a page or retires its browser, the borrow ends and the Page reports `closed`.
- **What Collie keeps:** the Effect journal and Run trace for its own actions, Plans and their replay, Success checks and Verdicts (ADR 0013), the engine's AI safety on its own actions as far as interception reaches (ADR 0014), Usage and Limits on its model calls, and Extraction.
- **What it loses** is ADR 0056's: the guard's guarantee (results unguarded, since the crawler's driver holds the page too), typing Secrets, egress enforced below the browser (interception only, with the rebinding gap recorded), stealth (the Session is `standard`, with the crawler's browser's identity), the Session pool, and resuming on another machine.
- **Retries stay the crawler's:** Collie sees a retried handler as new calls in the same Run. How a program names its own Effects is [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500)'s.
- **Guides with tested examples:**
  - Crawlee for JavaScript (`PlaywrightCrawler` and `PuppeteerCrawler`), Crawlee for Python (`PlaywrightCrawler`), and Scrapy with scrapy-playwright (`playwright_include_page`).
  - All three run handlers asynchronously, so they use the async clients, and ADR 0056's open question on Python's sync relay does not block them.
  - Crawl4AI's page hooks and any other program holding a Playwright or Puppeteer page reach Collie the same way, untested.

## Considered Options

- **A Crawlee-style driver adapter,** where a library launches the browser and Foxhound's handlers drive through that library's page: rejected.
  - The crawl would run without the guard, the Effect journal, the Goodfetch Chromium or the egress proxy, in the SDK's process, where a Daemon or Goodfetch Cloud cannot reach it.
  - It reopens ADR 0019's rejection of Playwright as the driver and ADR 0033's rejection of a browser backend that launches what it names. Every browser source a user has already comes in as a Borrowed browser or through a browser backend Plugin.
  - Crawlee's Stagehand crawler shows the cost: "low-level browser properties cannot be modified", five launch options pass through, and Crawlee for Python applies no canvas, WebGL or screen fingerprint (CF §1, §4).
- **A raw browser endpoint, as ADR 0019 first read:** rejected. It lends every Session in the browser process.
- **A dedicated browser process for each lend:** rejected. It costs a browser launch and its memory per lend, and `Browser.close` and the proxy bypass still need the same handling.
- **Refusing every browser-level command:** rejected. Playwright's own connect sends several (`Browser.getVersion`, `Target.setAutoAttach`, `Browser.setDownloadBehavior`), so it could not connect.
- **CDP carried over the SDK's own connection,** through the transport Playwright's Node `connectOverCDP` accepts since 1.61: rejected.
  - It is a second path to test, and the overload is documented only in `overrides.d.ts` and was once removed ([PR #40972](https://github.com/microsoft/playwright/pull/40972) restored it after #40195).
  - It works only where the engine shares the program's machine: it forces `isLocal`, which assumes a shared file system, and a Daemon elsewhere or Goodfetch Cloud has none.
  - Python's `connect_over_cdp` and Browser Use take only an address.
- **Stagehand v4 on a dedicated browser with its extension allowed:** rejected. It would install a third-party extension with the `debugger` permission into Foxhound's browser, out of the endpoint's sight, for verbs Collie already has.
- **Lending on every generated surface:** rejected. A CDP address handed to a model lets it drive a browser outside the engine's AI safety (ADR 0014).
- **Crawler-specific packages** (a `CollieCrawler` class, a Scrapy package): rejected.
  - Each would couple a Goodfetch package to a crawler's releases. Crawlee's own Stagehand crawlers show the cost: held at Stagehand v3 in both languages, and Crawlee for Python postponed the port "until v2".
  - ADR 0027 rejects hand-written per-tool Integrations, and `run.borrow()` is one call.
- **Collie as the crawler's browser provider** (Crawlee v4's `RemoteBrowserPool`, scrapy-playwright's `PLAYWRIGHT_CDP_URL` or `PLAYWRIGHT_BROWSER_PROVIDER`): rejected.
  - Those create browser contexts on the endpoint, which a Lent page refuses.
  - Crawlee skips its own fingerprints on a remote browser anyway.
  - Foxhound is the Product for a crawl in Goodfetch's browser.
- **One Session per borrowed browser, refusing a second context:** rejected. It refuses Crawlee's incognito pages and scrapy-playwright's named contexts, the crawlers' ordinary setups.
- **A new Run per borrowed context:** rejected. It splits one crawl's AI work, Usage and Run trace across Runs.

## Consequences

- **Amends ADR 0003.** Its rejected per-message lease check does not cover scoping: the endpoint of a Lent page parses each message to keep it inside the Session, while the lease is still enforced by cutting the attach. A note sits under that option.
- **Amends ADR 0017.** A Lent page's endpoint refuses new browser contexts, extensions and the commands above, keeps downloads the Run's, and the engine re-applies the Session's state when the attach ends. A note sits under its raw CDP consequence.
- **Amends ADR 0019.** The endpoint lends one page, shows only its Session, and stays a WebSocket by choice: Playwright's Node `connectOverCDP` can now take a transport, but Python and remote engines need an address. A note sits under its consequence on Playwright and Puppeteer code.
- **Amends ADR 0056.** Through `run.borrow()`, each browser context a borrowed page belongs to is one Session of the run. A note sits under its Borrowed browsers section's opening.
- **Amends ADR 0070.** Its Quick check over generated surfaces also refuses `page.lend` on every MCP profile, Framework adapter, Integration and skill. A note sits under that item.
- **Adds to ADR 0064:** two Live checks owned by area 11, each blocking nothing directly: `lend-libraries-newest`, each supported library's newest release on a Lent page, and `crawler-examples-newest`, the crawler guides' examples on each crawler's newest release. Both run nightly, or before each release candidate while the nightly tier is off. `TESTING.md`'s list of Live checks names them.
- **Amends ADR 0037's matrix.** SA-07, SA-08, RT-21 and pain 12 cite this ADR. Areas 4 (Security and egress), 5 (Site access) and 11 (Client surfaces) list it.
- **`GLOSSARY.md`** gains **Lent page**.
- **Areas.**
  - **Area 4** builds the refusals that keep files, contexts and extensions on the engine's side, and the egress canary through a Lent page (ADR 0067).
  - **Area 5** builds the endpoint's scoping, its command table, the attach's end and the state re-applied.
  - **Area 11** builds `page.lend()`, the helpers, their conformance cases, the guides and the crawler examples.
- **Other tickets.**
  - [Collie's exact capability set](https://github.com/goodfetch/automan/issues/455) lists lending as part of the page, and settles V3-01, AI verbs on a crawler's page.
  - [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477) hands its handlers a page that offers `lend()`.
  - [Docs site and positioning of two Products](https://github.com/goodfetch/automan/issues/499) places the lending and crawler guides beside the migration guides.
- **To prove in the spec:**
  - the endpoint's command table, including which page-level commands reach other Sessions (a browser-wide cache or cookie clear);
  - that auto-attach can follow the lent page's tree alone, through Chromium's experimental `Target.autoAttachRelated` (DI §3.1) or the endpoint's own filtering;
  - which commands read files on the browser's machine;
  - each supported library's connect, close and reconnect against the endpoint, at its floor and its newest release.
