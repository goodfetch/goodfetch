# A third-party automation library driving a page another program owns

Research of **October 14, 2026** for ticket [#452](https://github.com/goodfetch/automan/issues/452). Question: what works, and what breaks, when a browser-automation library drives a page that another program (a crawler or an engine) launched and owns, over CDP or WebDriver BiDi? Four parts: third-party libraries on a lent page (Foxhound lending to others), two CDP clients on one target, the reverse (Collie inside another crawler), and leading practice for lending a session to untrusted code.

**Sources.** Source code read through the GitHub API and Chromium's Gitiles at pinned versions:
- Playwright **v1.64.1** (2026-10-14) and Playwright Python **v1.64.0**;
- Puppeteer **puppeteer-core v25.13.0** (2026-10-08);
- Stagehand `main` at **`1da7359`** (2026-10-13, SDK 4.2.1), plus the lab's installed **4.1.0** bundle;
- Browser Use **0.13.11** (2026-10-07);
- chromiumoxide at **`a7e2bb8`** (the commit the Rust CDP note pinned);
- chromium-bidi **v17.0.2** (2026-07-06);
- Chromium `main` at **`3ea9312`** (2026-10-14), the DevTools protocol at **r1718299** (`dfad770`, 2026-10-14), the WebDriver BiDi editor's draft at **`f922e6a`** (2026-10-08) and Firefox `main` at **`3934f78`** (2026-10-14).

Also official docs (Playwright, Puppeteer, Stagehand, ChromeDriver, Selenium, Browserbase, Browserless, Steel, Bright Data), release notes and issue trackers, read on 2026-10-14. The Stagehand v4 packaging facts reuse the salvaged ticket #448 report; its sources are cited inline. Prior notes reused and re-checked: [Rust CDP clients](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md), [Stagehand CDP latency](2026-10-07-stagehand-cdp-latency.md), [Crawlee and Stagehand contracts](2026-10-07-crawlee-spider-contracts-followup.md), [stealth state of the art](https://github.com/goodfetch/automan/blob/research/stealth-state-of-the-art/rnd/research/2026-10-10-stealth-state-of-the-art.md) and [AI browser frameworks](https://github.com/goodfetch/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md).

**Measured versus cited.** One localhost experiment was run (§6): headless Chrome 155.0.8059.40, Playwright 1.63.0, puppeteer-core 25.12.0 and the Stagehand 4.1.0 extension, all already installed, and a raw CDP "owner" client that stands in for the engine. Measured facts are marked **M**; everything else is cited and marked **C**. The script and its raw output are in [`2026-10-14-driver-interop/`](2026-10-14-driver-interop/). Like everything in `rnd/`, this note informs a decision; it does not make one.

---

## 1. Summary

- **None of the mainstream libraries can be lent one page; they all want the browser.**
  - Given a page-level endpoint (`/devtools/page/<id>`), Playwright `connectOverCDP` connects but sees no pages, and Puppeteer `connect` fails with `Target.getBrowserContexts: Not allowed` **M**.
  - On the browser endpoint, Playwright auto-attaches every page of every browser context and puts pages from contexts it did not create into its default context ("TODO: auto attach only to pages from our contexts") **C** ([crBrowser.ts v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crBrowser.ts)). A page in a separate context created by the owner showed up in Playwright's default context **M**.
  - So lending "one page" needs a relay that plays a browser endpoint and filters targets.
- **Attaching rewrites the owner's browser state, and detaching does not put it back.**
  - Playwright's defaults send downloads from the default context to its own temp folder under GUID names. After Playwright disconnected, the owner's own download setting did not come back: files went to Chrome's default folder **M**. Chromium keeps one download slot per browser context, and resets it to `default` when the last writer's session ends **C** ([devtools_download_manager_delegate.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/devtools_download_manager_delegate.cc)).
  - Playwright's `noDefaults: true` (since **v1.60**) left the owner's download folder alone **M**.
  - Puppeteer's default `defaultViewport` replaced the owner's 1111×777 emulation with 800×600. After `disconnect()` the page was 756×469, the bare window, because the detaching session cleared device emulation for everyone **M**.
  - Browser Use 0.13.11, on any browser it connects to, sets `Browser.setDownloadBehavior` to its own temp folder, grants `clipboardReadWrite` and `notifications` to every origin, and navigates `chrome://newtab` tabs to `about:blank` **C**.
- **"Close" kills the owner's browser in three of the libraries.**
  - Puppeteer's `browser.close()` on a connected browser sends `Browser.close`; Chrome exited with code 0 **M**.
  - Stagehand v4's `browser.close()` on a `localBrowser.connect()` handle sends `Browser.close` **C** ([factories.ts @1da7359](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/browser/factories.ts)), and so does chromiumoxide's `Browser::close` **C**.
  - Playwright's `close()` only disconnects; the browser stayed alive **M**.
- **Stagehand v4 cannot use a lent page at all.** It has "no interop" with Playwright or Puppeteer pages **C** ([playwright.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx)). Its runtime is an extension it installs in the owner's browser with `Extensions.loadUnpacked`, which "requires a shared filesystem" **C** ([browser.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/browser.mdx)). The extension's `debugger` permission lets it drive every tab from inside the browser, where a relay cannot see it. On Chrome 155 headless, a plain `--remote-debugging-port` client could load that extension and evaluate in its service worker, **with or without** `--enable-unsafe-extension-debugging` **M**, although Stagehand's docs call the flag required **C**.
- **A second client can stall, rewrite or bypass the owner.**
  - **Stalling.** A second client with `Fetch` on that never continues a request stops the owner's navigation until it disconnects **M**. A second client's page-level `waitForDebuggerOnStart` holds the page's OOPIF and worker scripts until it disconnects **M**.
  - **Rewriting.** A second client rewrote a request URL in Fetch. In three orderings the owner's earlier session saw the rewritten URL, so the owner's check ran on what was actually sent **M**.
  - **Bypassing.** With the browser launched behind `--proxy-server`, a second client created a context with `proxyServer: "direct://"`, and its page reached the server directly **M**. A Chrome-level proxy flag is therefore not an egress boundary against a browser-level client.
- **Runtime goes on.** Playwright, Puppeteer, chromiumoxide and chromium-bidi (behind ChromeDriver BiDi) all send `Runtime.enable` on every page they wrap **C**. Browser Use's session code does not **C**. A Runtime-free owner (ADR 0019) loses that property while one of them is attached.
- **Two Playwright clients on one browser are unsupported upstream.** Playwright "is not designed to work with multiple Playwright sessions connected to the same browser" **C** ([#32821](https://github.com/microsoft/playwright/issues/32821), 2024-09-27).
- **Playwright can now attach without a port.** Since v1.61 (2026-06-15), Node `connectOverCDP` accepts a `ConnectionTransport` object, local only **C**. That contradicts ADR 0019's premise that the raw CDP endpoint "must be a WebSocket, because Playwright's `connectOverCDP` cannot attach over a pipe". Python's `connect_over_cdp` still takes only a URL **C**.
- **The reverse direction exists only as Crawlee + Stagehand v3.**
  - **TS** (`@crawlee/stagehand` 3.18.2): Stagehand launches the browser, and Crawlee attaches Playwright to it over CDP **C**.
  - **Python:** the integration is capped below Stagehand 4. The port is "postponed until v2" because v4 dropped Python 3.10 **C** ([crawlee-python#2154](https://github.com/apify/crawlee-python/issues/2154)).
  - **Known problems:** a race between two clients registering the same page, cookies saved before the handler runs, context options ignored, fingerprints only partly applied, and `cdpUrl` dropped ([crawlee#4196](https://github.com/apify/crawlee/issues/4196), open) **C**.
- **Cloud vendors scope per session, not per command.**
  - **Credentials:** a bearer credential in the connect URL (Browserbase, Steel, Browserless, Bright Data) **C**.
  - **Connections:** one client per persisted session (Browserless, HTTP 429), or several (Browserbase with keep-alive) **C**.
  - **Navigation:** limited to one domain per session (Bright Data) or checked on top frames only (Browserbase's experimental `allowedDomains`) **C**.
  - **Idle cut:** after 5 to 10 minutes without a CDP command **C**.
  - **No published command allowlist:** no vendor publishes one for user CDP (**Unverified** that none exists). Puppeteer's own `allowlist` says it is "not a complete network sandbox" **C**.

---

## 2. Third-party libraries on a lent page

### 2.1 Playwright `connectOverCDP` (v1.64.1)

**What the docs promise** **C** ([class-browsertype.md v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md)):
- The method "attaches Playwright to an existing browser instance using the Chrome DevTools Protocol", Chromium only.
- "This connection is significantly lower fidelity than the Playwright protocol connection".
- "If you launch the browser without Playwright and do not pass the exact same arguments, some of Playwright functionality may be broken upon connecting to the browser."

**Options, with the version that added them** **C** (same file):
- `headers` (v1.11), `slowMo`, `timeout` (30 s default).
- `isLocal` (v1.58): Playwright "runs on the same host as the CDP server", enabling file-system optimizations.
- `noDefaults` (v1.60): "Playwright will not apply its default overrides to the existing default browser context". Downloads stay at the browser's setting, focus emulation stays off, and media emulation (`colorScheme`, `reducedMotion`, `forcedColors`, `contrast`) is not applied. New contexts are not affected.
- `artifactsDir` (v1.61): where traces and downloads go.
- **A channel name** such as `'chrome'` instead of a URL: Playwright reads the profile's `DevToolsActivePort`, which is how it reaches Chrome 144's consented debugging ([chromium.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts) `resolveChannelEndpoint`; [#40177](https://github.com/microsoft/playwright/pull/40177)).
- **A transport object** instead of a URL (Node only). The overload `connectOverCDP(transport: ConnectOverCDPTransport)` takes `{send, close, onmessage, onclose}` and is typed in [overrides.d.ts](https://github.com/microsoft/playwright/blob/v1.64.1/utils/generate_types/overrides.d.ts). It is not in the API docs: the PR says the shape "is defined only in overrides.d.ts, not in the docs" ([#40972](https://github.com/microsoft/playwright/pull/40972), merged 2026-05-22, first in v1.61.0, 2026-06-15).
  - It throws "Passing a ConnectionTransport to connectOverCDP is not supported when connecting remotely" ([client browserType.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/browserType.ts)), and forces `isLocal`.
  - Playwright Python 1.64.0's `connect_over_cdp` takes only `endpointURL` ([_browser_type.py](https://github.com/microsoft/playwright-python/blob/v1.64.0/playwright/_impl/_browser_type.py)).

**What it does on connect** **C** (from source: [crBrowser.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crBrowser.ts), [crPage.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crPage.ts), [crNetworkManager.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crNetworkManager.ts)):
1. `Browser.getVersion`. Playwright reads "headful" from the user agent, because launch options "may lie".
2. `Target.setAutoAttach {autoAttach: true, waitForDebuggerOnStart: true, flatten: true}` on the browser session. Every existing and future page in every browser context is attached.
   - A page whose context Playwright did not create goes into the default context ("TODO: auto attach only to pages from our contexts").
   - Service workers are kept; other target types are detached "to avoid side-effects".
3. The default context is initialized. Unless `noDefaults` is set, Playwright sends `Browser.setDownloadBehavior {behavior: 'allowAndName', downloadPath: <artifacts dir>, eventsEnabled: true}` with no `browserContextId`, which targets the browser's default context. The artifacts dir is a fresh temp folder that Playwright removes on disconnect ([chromium.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts) `_connectOverCDPImpl`).
4. For each page, Playwright sends:
   - `Page.enable`, `Page.getFrameTree`, `Log.enable`, `Page.setLifecycleEventsEnabled` and **`Runtime.enable`**;
   - `Page.addScriptToEvaluateOnNewDocument` and `Page.createIsolatedWorld` for a world named `__playwright_utility_world_<guid>`, with `grantUniveralAccess: true`;
   - `Network.enable`, and a page-level `Target.setAutoAttach` for OOPIFs and workers;
   - `Emulation.setFocusEmulationEnabled` and media emulation, unless `noDefaults` is set;
   - file-chooser interception (`Page.setInterceptFileChooserDialog`), default font families in headless, and `Inspector.enable` for pages attached during connect;
   - then `Runtime.runIfWaitingForDebugger`.

   The default context gets `noDefaultViewport: true`, so the window size is untouched, which the experiment confirmed (1111×777 kept) **M**.
5. Connect waits until every pre-existing page answers its init commands.
   - A frozen or discarded tab used to hang connect forever ([#42882](https://github.com/microsoft/playwright/issues/42882), 2026-09-23: "noDefaults does not help").
   - [PR #42936](https://github.com/microsoft/playwright/pull/42936), merged 2026-09-30, sends `Inspector.enable`, so such a page reports `Inspector.targetCrashed` and is dropped. The fix is in v1.64.1's `crPage.ts`.

**Contexts and close** **C**:
- New contexts are `Target.createBrowserContext {disposeOnDetach: true}`, so they disappear when Playwright disconnects.
- On a connected browser, `browser.close()` "clears all created contexts belonging to this browser and disconnects" ([class-browser.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browser.md)). In code it closes the socket and removes the temp folders; it never sends `Browser.close`. The browser stayed alive **M**.
- There is no `detach()`. A maintainer's answer to [#39011](https://github.com/microsoft/playwright/issues/39011) (2026-01-28) was "Call browser.close()".

**What users hit** **C**:
- **Downloads.** Files land in Playwright's temp folder, unnamed, even for tabs the person opened by hand, and are lost when the folder is removed ([#40158](https://github.com/microsoft/playwright/issues/40158), 2026-04-10). `noDefaults` fixed it ([PR #40185](https://github.com/microsoft/playwright/pull/40185)). With a remote browser, `download.saveAs()` throws `ENOENT` because the file is on the browser's host ([#41060](https://github.com/microsoft/playwright/issues/41060), 1.60).
- **Forced light colour scheme** on attach ([#37627](https://github.com/microsoft/playwright/issues/37627)), also fixed by `noDefaults`.
- **Two Playwright clients on one browser.** A maintainer explained that `Runtime.bindingCalled` "is broadcast to all connections and only one of them is aware of the binding, the other one throws". The issue was closed as out of scope: Playwright "is not designed to work with multiple Playwright sessions connected to the same browser, they will conflict in more places even if we fix it here" ([#32821](https://github.com/microsoft/playwright/issues/32821), 1.47.2).
- **A headed browser the person also uses** "responds to real mouse events", a collaborator answered, recommending Playwright's own launch arguments ([#41286](https://github.com/microsoft/playwright/issues/41286), 2026-06-15).
- **Video, tracing and context options** were long missing on connected browsers ([#29065](https://github.com/microsoft/playwright/issues/29065), [#42776](https://github.com/microsoft/playwright/issues/42776)).

### 2.2 Puppeteer `connect` (puppeteer-core v25.13.0)

**Options** **C** ([ConnectOptions.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/common/ConnectOptions.ts)):
- `browserWSEndpoint`, `browserURL` or a `transport`; `channel` (experimental) reads `DevToolsActivePort`.
- `defaultViewport`, **default `{width: 800, height: 600}`**, which "sets the viewport for each page".
- `downloadBehavior`, unset by default.
- `targetFilter`, a callback that decides which targets Puppeteer attaches to.
- `networkEnabled` and `issuesEnabled` (experimental, both `true` by default).
- `protocolTimeout`, 180 s by default.
- `blocklist` and `allowlist` (experimental) of URLPatterns; `allowlist` needs Chrome 149.
  - The docs say they are "meant as an additional guardrails to LLM-based usage under Puppeteer control and not a complete network sandbox", and that Puppeteer "will silently detach from any already open targets that violate the patterns".
  - They are enforced with `Network.emulateNetworkConditionsByRule`, which makes blocked requests go offline. While they are on, Puppeteer refuses the user's own `emulateNetworkConditions` calls ([TargetManager.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/TargetManager.ts), [Browser.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/Browser.ts)).

**What it does on connect** (from source) **C**:
- `Target.getBrowserContexts`, then `Target.setDiscoverTargets {discover: true}`, then a browser-level `Target.setAutoAttach {waitForDebuggerOnStart: true, flatten: true}` that excludes `page` targets. Pages are reached through their `tab` targets, on which Puppeteer sets auto-attach again ([TargetManager.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/TargetManager.ts)).
- A `Page` object is created when code asks for it (`browser.pages()`, `target.page()`). Its init sends `Page.enable`, `Page.getFrameTree`, `Page.setLifecycleEventsEnabled`, **`Runtime.enable`**, `Performance.enable`, `Log.enable` and `Network.enable`. It creates the utility world with `Page.addScriptToEvaluateOnNewDocument` and `Page.createIsolatedWorld {grantUniveralAccess: true}`, then calls `setViewport(defaultViewport)` ([Page.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/Page.ts), [FrameManager.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/FrameManager.ts)). The owner's page became 800×600 **M**.
- **`browser.close()` sends `Browser.close`**, even on a connected browser: the connector passes `() => connection.send('Browser.close')` as the close callback. `browser.disconnect()` only disposes the connection ([BrowserConnector.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/BrowserConnector.ts)). Chrome exited **M**.
- On a page-level endpoint, connect fails with `Target.getBrowserContexts: Not allowed` **M**.

### 2.3 Stagehand v4 (SDK 4.2.1, `main` at `1da7359`)

**Attach model** **C**:
- **The runtime is an extension.** v4's driver, `act`/`observe`/`extract` and LLM clients live in an MV3 extension, "Stagehand Runtime". It holds the `debugger`, `offscreen`, `scripting` and `tabs` permissions, `<all_urls>` host permissions, and a content script in all frames at `document_start` ([manifest.json](https://github.com/browserbase/stagehand/blob/main/packages/extension/manifest.json), via the #448 report; the 4.1.0 bundle's manifest says the same).
- **The SDK speaks JSON-RPC over CDP** ([cdpClient.ts @1da7359](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/cdpClient.ts)):
  - it finds the extension's service worker with `Target.getTargets`, waking it if needed with `Target.createTarget` on `wake-service-worker.html`;
  - it attaches with `Target.attachToTarget {flatten: true}`, then sends `Runtime.enable` and `Runtime.addBinding`;
  - it sends each message with `Runtime.evaluate`.

**Three browser factories** **C** ([browser.mdx @1da7359](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/browser.mdx)):
- **`localBrowser.launch()`** starts system Chrome with `--remote-debugging-port`, `--enable-unsafe-extension-debugging`, `--remote-allow-origins=*` and about 25 automation flags. It loads the extension with `Extensions.loadUnpacked` (#448 report, from the Go launcher and the 4.2.1 wheel). Note that `--remote-allow-origins=*` switches off Chrome's Origin check on the debugging port (§5.1).
- **`localBrowser.connect({cdpUrl})`** "attaches to a running Chromium browser through its DevTools endpoint. It discovers an installed Stagehand extension first, then loads the bundled extension if none is found or discovery is unsupported. Loading requires a shared filesystem."
  - Without `Extensions.loadUnpacked` it fails with "This Chrome build does not support Extensions.loadUnpacked. Launch with --load-extension and connect using extensionId instead." (cdpClient.ts). Per the #448 report, `extensionId` on connect is deprecated and ignored since 4.2.0.
- **`browserbase.connect({sessionId})`** needs a session created with the extension: "Extensions cannot be added after a browser session starts."

The docs say `--enable-unsafe-extension-debugging` is "required so the SDK can attach a CDP session to the service worker hosting Stagehand's runtime. Chrome blocks debugger access to extension targets without it" **C** (#448 report).
- **Measured on Chrome 155 headless**, a client on a plain `--remote-debugging-port` browser could:
  - load the 4.1.0 extension with `Extensions.loadUnpacked`;
  - attach to its service worker;
  - evaluate `typeof chrome.debugger` (`"object"`) there.

  This held with and without that flag **M**.
- **In the protocol and the source:** the DevTools protocol at r1718299 no longer says `Extensions.loadUnpacked` needs `--remote-debugging-pipe` and the flag. In Chromium `main`, the Extensions handler is created for trusted clients and allows loading on the browser target ([Extensions.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Extensions.pdl), [chrome_devtools_session.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/devtools/chrome_devtools_session.cc), [extensions_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/devtools/protocol/extensions_handler.cc)) **C**.
- **Not tested:** headed Chrome, and whether stable Chrome still gates the service-worker attach somewhere.

**On a lent page** **C**:
- **No interop.** v4 has "no interop": "you can't hand a Playwright `Page` to `act()`", and the `page` option "only accepts a page from `browser.context`", with one context per browser ([playwright.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx)).
- **It needs the whole browser.** A Foxhound handler cannot give Stagehand v4 one page: Stagehand needs the browser endpoint and installs an extension with the `debugger` permission into the owner's profile.
- **Out of the relay's sight.** The extension drives its own pages from inside the browser through `chrome.debugger`, which never crosses the WebSocket a relay could filter.

**Close** **C**:
- The docs: "Stagehand closes only the browsers it launched, so `browser.close()` is yours to call" (browser.mdx).
- In code, `browser.close()` on a `localBrowser.connect()` handle sends `Browser.close` ([factories.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/browser/factories.ts)). `stagehand.close()` only releases its claim (4.1.0 bundle, `Stagehand.close`).

### 2.4 Stagehand v3 (`v3-latest` 3.7.3)

**C**:
- v3 ran its own CDP driver in Node and accepted `localBrowserLaunchOptions.cdpUrl`.
- It had peer dependencies on `playwright-core`, `puppeteer-core` and `patchright-core`, so a page from those libraries could be passed to `act` (#448 report, [npm 3.7.3](https://www.npmjs.com/package/@browserbasehq/stagehand/v/3.7.3)).
- Crawlee's integrations are built on it (§4). As a guest on a lent browser, v3 is closer to Playwright than to v4: no extension, a Node-side CDP client.

### 2.5 Browser Use (0.13.11)

**Connect** **C** ([session.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session.py)):
- `BrowserSession(cdp_url=...)` (with `is_local=False`) connects to any CDP browser.
- It sets `Target.setAutoAttach {autoAttach: true, waitForDebuggerOnStart: false, flatten: true}` on the browser session and attaches to every page.
- It navigates any `chrome://newtab` page to `about:blank`, and opens a blank page if there is none.
- Per target it enables `Page`, lifecycle events and `Network` ([session_manager.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session_manager.py)). Its session and watchdog code never sends `Runtime.enable`; a code search finds it only in `actor/page.py` and the Anthropic integration.

**Browser-wide changes** **C**:
- `DownloadsWatchdog` sends `Browser.setDownloadBehavior {behavior: 'allow', downloadPath, eventsEnabled: true}` with no context. The default `downloads_path` is a fresh `browser-use-downloads-<id>` temp folder, so this happens on every browser it connects to ([downloads_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/downloads_watchdog.py), [profile.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py)).
- `PermissionsWatchdog` sends `Browser.grantPermissions` with `['clipboardReadWrite', 'notifications']` and "origin=None means grant to all origins" ([permissions_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/permissions_watchdog.py)).

**Lifecycle** **C**:
- `stop()` "stops the browser session without killing the browser process". For a non-local browser, `BrowserStopEvent` resets state and only kills what Browser Use launched. If the CDP URL looks like a Browser Use Cloud session, it asks the cloud to stop it.
- After an unintended WebSocket drop, `_auto_reconnect` retries 3 times with exponential backoff, and actions wait up to `RECONNECT_WAIT_TIMEOUT = 54.0` s. A cut attach is therefore retried before the agent fails.

### 2.6 Selenium 4, ChromeDriver and WebDriver BiDi

- **ChromeDriver can attach to a running Chrome** through `goog:chromeOptions.debuggerAddress`, but "there are some commands that ChromeDriver does not support when working with existing sessions through remote debugging". Some commands "require a Chrome extension to be loaded into the browser", which ChromeDriver loads only when it launches the browser ([ChromeDriver help](https://developer.chrome.com/docs/chromedriver/help/operation-not-supported-when-using-remote-debugging), updated 2024-12-17) **C**.
- **Selenium's CDP access** "is temporary until WebDriver BiDi has been implemented", "is not designed for testing", and Selenium supports "the 3 most recent versions of Chrome" ([Selenium CDP docs](https://www.selenium.dev/documentation/webdriver/bidi/cdp/), modified 2024-07-10) **C**.
- **Playwright through a Selenium Grid** is the one official case of two drivers sharing a browser: with `SELENIUM_REMOTE_URL`, Playwright opens a Grid session, reads `se:cdp` (Selenium 4) or `goog:chromeOptions.debuggerAddress` (Selenium 3), then calls `connectOverCDP` ([chromium.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts)) **C**. ChromeDriver and Playwright then both hold CDP sessions on the same browser.
- **WebDriver BiDi on Chrome is CDP underneath.** chromium-bidi v17.0.2 sends `Page.enable`, **`Runtime.enable`**, `Network.enable`, `Target.setAutoAttach` and `Runtime.runIfWaitingForDebugger` for each target, and `Fetch.enable` for interception ([CdpTarget.ts](https://github.com/GoogleChromeLabs/chromium-bidi/blob/chromium-bidi-v17.0.2/src/bidiMapper/modules/cdp/CdpTarget.ts)) **C**. A BiDi client on a lent Chrome page adds the same signals as Playwright.
- **The BiDi spec on several sessions** **C** ([index.bs @f922e6a](https://github.com/w3c/webdriver-bidi/blob/f922e6af669ead51496db85f37813f39438ef171/index.bs)):
  - A session can have several WebSocket connections ("session WebSocket connections").
  - `session.new` fails with "session not created" "if the implementation is unable to start a new session for any reason".
  - For `browser.close` with several sessions: "The behaviour in cases where the browser has multiple automation sessions is currently unspecified."
- **Firefox runs one BiDi session at a time:** "We currently only support one session, see Bug 1720707", and creation fails with "Maximum number of active sessions" ([WebDriverBiDi.sys.mjs @3934f78](https://github.com/mozilla-firefox/firefox/blob/3934f7874d/remote/webdriver-bidi/WebDriverBiDi.sys.mjs)) **C**. A Firefox page cannot be lent to a second BiDi session; at most the session's connection can be shared.

### 2.7 chromiumoxide (and Chromey)

**C** ([handler/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/mod.rs), [handler/target.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/target.rs), [handler/network.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/network.rs), [browser/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/browser/mod.rs)):
- `Browser::connect(url)` sends `Target.setDiscoverTargets` and attaches to every target from `Target.getTargets`.
- Page init sends `Runtime.enable` and creates `__chromiumoxide_utility_world__` with universal access (Rust CDP note).
- `HandlerConfig::default()` has `ignore_https_errors: true`, so every attached page gets `Security.setIgnoreCertificateErrors {ignore: true}`. Its `viewport` is `None` by default on connect.
- `Browser::close()` sends `Browser.close` whether the browser was launched or connected.
- Chromey shares this shape, except that its frame init has `Runtime.enable` commented out (Rust CDP note).

### 2.8 What the lent page keeps and loses, per library

Owner-visible effects of attaching with default options, from the source above and the experiment. "All pages" means every page in every browser context, including the owner's.

| Library (version) | Endpoint it needs | Attaches to | Runtime on | Changes the owner's state on attach | Detach / close | Opt-outs |
|---|---|---|---|---|---|---|
| Playwright `connectOverCDP` (1.64.1) | browser (page endpoint: 0 pages **M**) | all pages, put in its default context **M** | yes | download folder → temp, GUID names **M**; focus emulation; media emulation; file-chooser interception; fonts (headless) | `close()` disconnects, browser lives **M**; owner's download setting lost **M** | `noDefaults: true` (downloads kept **M**, no focus/media); `isLocal`; transport (Node, local) |
| Puppeteer `connect` (25.13.0) | browser (page endpoint fails **M**) | all tabs; Page objects on demand | yes | 800×600 viewport per wrapped page **M** | `close()` → `Browser.close`, browser exits **M**; `disconnect()` clears the owner's emulation **M** | `defaultViewport: null`, `targetFilter`, `networkEnabled: false`; use `disconnect()` |
| Stagehand v4 `localBrowser.connect` (4.2.1) | browser + shared filesystem | its own pages, through its extension | yes (on its service worker) | installs an extension with `debugger` + `<all_urls>` **M** (load) | `browser.close()` → `Browser.close`; `stagehand.close()` releases | none for a lent page; no page interop |
| Stagehand v3 (3.7.3) | browser (`cdpUrl`) or a Playwright/Puppeteer page | its own pages | **Unverified** | **Unverified** | **Unverified** | — |
| Browser Use `cdp_url` (0.13.11) | browser | all pages; redirects new-tab pages | no (session code) | download folder → temp; grants clipboard and notifications to all origins | `stop()` keeps the browser; auto-reconnects ×3 | `downloads_path`, `permissions=[]` (**Unverified** that an empty list skips the grant) |
| Selenium / ChromeDriver `debuggerAddress` | browser port | its window handles | through chromium-bidi when BiDi is on | **Unverified** | `quit()` on an attached session **Unverified** | some commands unavailable when attached |
| chromiumoxide `Browser::connect` (a7e2bb8) | browser | every target | yes | ignores certificate errors on each page | `close()` → `Browser.close` | `HandlerConfig { ignore_https_errors: false }` |

---

## 3. Two CDP clients on one target

### 3.1 What Chrome supports

**C**:
- **Several clients per target since Chrome 63** (2017): "two separate WebSocket protocol clients" can connect to the same tab, and several `chrome.debugger` extensions can share a tab ([DevTools 63](https://developer.chrome.com/blog/new-in-devtools-63)).
- **Flat sessions are the intended mode.** `setAutoAttach`'s `flatten` "enables flat access to the session". Chrome plans to "make this the default, deprecate non-flattened mode, and eventually retire it" (crbug.com/991325) ([Target.pdl r1718299](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Target.pdl)).
- **Scoped auto-attach exists.** `Target.autoAttachRelated {targetId}` is experimental and browser-target only. It "adds the specified target to the list of targets that will be monitored for any related target creation" and auto-attaches it, so a client can follow one page and its children without the rest (Target.pdl).
- **Other browser-wide commands any browser-level client can send** (Target.pdl, [Browser.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Browser.pdl)):
  - `Browser.close` ("Close browser gracefully");
  - `Browser.setDownloadBehavior`, whose omitted `browserContextId` means the default context;
  - `Target.createBrowserContext {proxyServer, proxyBypassList, disposeOnDetach}`;
  - `Target.exposeDevToolsProtocol` (gives a page a `cdp` binding);
  - `Target.attachToBrowserTarget`.
- **Chromium has a permission model for clients, used for `chrome.debugger`.**
  - `DevToolsAgentHostClient` asks each client `MayAttachToURL`, `IsTrusted` ("allowed to attach to the browser agent host and perform other privileged operations"), `MayReadLocalFiles`, `MayWriteLocalFiles` ("manipulating a default downloads path") and `AllowUnsafeOperations` ([devtools_agent_host_client.h](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/public/browser/devtools_agent_host_client.h)).
  - An extension client is untrusted (except Perfetto's) and may never write local files. Its navigations count as renderer-initiated from the extension's origin. It raises an "is debugging" infobar unless `--silent-debugger-extension-api` is set ([debugger_api.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/extensions/api/debugger/debugger_api.cc)).
  - Chrome creates most domain handlers (Target, Browser, Emulation, Extensions…) for an untrusted client only when the domain is marked available to untrusted ones ([chrome_devtools_session.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/devtools/chrome_devtools_session.cc)).
  - A WebSocket or pipe client is trusted, and the protocol offers no way to hand one a reduced set.

### 3.2 State the clients share, and who wins

| State | Scope in Chromium | When two clients set it | When one detaches | Source |
|---|---|---|---|---|
| Download behaviour (`Browser.setDownloadBehavior`) | one slot per browser context | last writer wins **M** | if the detaching client was the last writer, the behaviour resets to `default`, not to the earlier client's value **M** | [devtools_download_manager_delegate.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/devtools_download_manager_delegate.cc), [browser_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/browser_handler.cc) |
| Device metrics, touch, geolocation, sensors (`Emulation.*`) | per session handler, applied to the one WebContents | last writer wins **M** | the detaching session's `Disable` clears device emulation and restores the view size (`ClearDeviceEmulationSize`), whoever set it **M** | [emulation_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/emulation_handler.cc) |
| Request interception (`Fetch.enable`) | one interceptor per session, daisy-chained "from outermost (client-facing) to innermost (network-facing)" | each must continue the request. The later-attached session saw the original URL and the owner's earlier session saw the rewritten one, in all three orderings tried **M**. A stalled interceptor stops the request **M** | its interceptor leaves the chain and the request proceeds **M** | [devtools_url_loader_interceptor.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/devtools_url_loader_interceptor.cc) |
| New targets with `waitForDebuggerOnStart` | one navigation throttle per auto-attaching session | a browser-level wait by a second client did **not** hold a new tab the owner created (its script ran) **M**; a page-level wait held the page's OOPIF and worker scripts **M** | the held targets run **M** | [target_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/target_handler.cc) |
| Proxy (`Target.createBrowserContext {proxyServer}`) | per browser context | a second client's context with `proxyServer: "direct://"` bypassed the launch `--proxy-server` **M** | the context is disposed if created with `disposeOnDetach` | Target.pdl |
| Bindings (`Runtime.addBinding`) | page | `Runtime.bindingCalled` reaches every connection with Runtime on, per a Playwright maintainer | — | [#32821](https://github.com/microsoft/playwright/issues/32821) |
| Permissions (`Browser.grantPermissions`) | per browser context | last writer wins | the detaching session resets the overrides it made | browser_handler.cc `Disable` |

### 3.3 What a second client adds to detection

- **`Runtime.enable`.** Playwright, Puppeteer, chromiumoxide and chromium-bidi send it on every page they wrap (§2) **C**. The stealth note records two published signals that need the Runtime domain on: the 2024 console-serialization getter and the March 2026 Proxy-prototype preview trap ([stealth note](https://github.com/goodfetch/automan/blob/research/stealth-state-of-the-art/rnd/research/2026-10-10-stealth-state-of-the-art.md)). A guarded, Runtime-free owner (ADR 0019) loses that property for as long as a lent library is attached.
  - Our own version of the Proxy-prototype probe (a `console.debug` of an object whose prototype is a counting Proxy) never fired on Chrome 155 headless, with or without Playwright or Puppeteer attached **M**. We cannot tell whether V8 changed or our probe differs from the published one (**Unverified**).
- **Named isolated worlds with universal access:** `__playwright_utility_world_<guid>`, Puppeteer's utility world with its `//# sourceURL` marker, and `__chromiumoxide_utility_world__` **C**.
- **Viewport and focus changes.** Puppeteer's 800×600 default and Playwright's focus emulation change `innerWidth` and `document.hasFocus()` unless switched off **M**/**C**.
- **Certificate and permission changes.** chromiumoxide ignores certificate errors, and Browser Use grants notifications and clipboard to every origin **C**. Both change what a page can observe about the browser.
- **Stagehand v4** adds an extension with content scripts at `document_start` in every frame **C**.

### 3.4 Relays and cloud connect URLs

| Provider | How a client connects | Several clients on one browser | Limits it enforces | Source |
|---|---|---|---|---|
| Browserbase | `connectUrl` WebSocket from session creation; `signingKey` "to use when connecting to the Session via HTTP"; docs say to use `browser.contexts()[0]` | without keep-alive the session ends when the connection closes; with `keepAlive`, "Different scripts or tools can connect to the same session" | CDP connections "closed after 10 minutes without any CDP commands"; `allowedDomains` (experimental) checks "top-frame navigations" only, not iframes or subresources | [create a session](https://docs.browserbase.com/reference/api/create-a-session.md), [using a session](https://docs.browserbase.com/platform/browser/getting-started/using-browser-session.md), [keep alive](https://docs.browserbase.com/platform/browser/long-sessions/keep-alive.md), [manage](https://docs.browserbase.com/platform/browser/getting-started/manage-browser-session.md), [allowed domains](https://docs.browserbase.com/platform/browser/security/allowed-domains.md) |
| Browserless | `wss://…/chromium?token=…`; custom CDP methods (`Browserless.liveURL`, `Browserless.reconnect`, `Browserless.pageId`, captcha, recording) | persisted sessions: "Only one client can connect to a session at a time", HTTP 429; `Browserless.reconnect` returns a new endpoint and takes an optional `auth` token "required on the reconnect handshake" | session `ttl`; per-connection `timeout` | [persisting state](https://docs.browserless.io/baas/session-management/persisting-state), [llms-full.txt](https://docs.browserless.io/llms-full.txt) (custom CDP section) |
| Steel | `wss://connect.steel.dev?apiKey=<key>&sessionId=<id>`; reuse `browser.contexts()[0]` | **Unverified** | sessions must be released; 5-minute default timeout | [Steel llms-full.txt](https://docs.steel.dev/llms-full.txt) |
| Bright Data Browser API | `wss://<user>:<pass>@brd.superproxy.io:9222`; "all puppeteer functions/features work"; custom CDP events | **Unverified** | `navigate_domains_limit` "Session limited to one domain"; 60-minute session; 5 minutes without a CDP command or without network activity ends it; robots.txt honoured before KYC (`brob`); "Password entry is disabled by default" | [error codes](https://docs.brightdata.com/products/scraping-browser/error-codes.md), [standard CDP functions](https://docs.brightdata.com/products/scraping-browser/cdp-functions/standard.md), [KYC and access](https://docs.brightdata.com/scraping-automation/scraping-browser/kyc-and-access) |

Each of these is reached with a long-lived account credential in the URL. None documents a per-attach token, a command allowlist or target filtering for the user's own CDP (**Unverified** that none exists).

---

## 4. The reverse: Collie inside another crawler

**Crawlee TS + Stagehand v3** (`@crawlee/stagehand` 3.18.2) **C**, from the [contracts note](2026-10-07-crawlee-spider-contracts-followup.md), re-checked against the issues below:
- **Who launches.** Stagehand launches the browser; Crawlee reads `connectURL()` and attaches Playwright `chromium.connectOverCDP()` to it. Two CDP clients then share every page: Stagehand's and Playwright's.
- **Pages.** They are created in `browser.contexts()[0]`, and `_contextOptions` is ignored. Crawlee polls every 25 ms, for up to 10 s, until Stagehand registers each page, because `newPage()` on the Playwright side races `Target.attachedToTarget` on Stagehand's side ([fix #3865](https://github.com/apify/crawlee/pull/3865), 2026-07-16).
- **The agent can land on another handler's page.** `page.agent()` does not pass the current page to Stagehand.
- **Cookies** are read and written best-effort, and saved after navigation but before the handler.
- **Fingerprints** are only partly applied, because Stagehand controls the launch.
- **Remote browsers are not supported:** `cdpUrl` is dropped by a fixed option allowlist, and "`remoteBrowser` never worked for `StagehandCrawler`… since the refactor it throws" ([crawlee#4196](https://github.com/apify/crawlee/issues/4196), 2026-10-05, open).
- **No v4 port:** none was found (#448 report).

**Crawlee Python + Stagehand** **C**:
- The plugin creates a REST `AsyncSession`, takes `session.data.cdp_url` and calls `connect_over_cdp` (contracts note).
- Stagehand 4.0.0 broke the import (`AsyncStagehand` removed), so the extra was capped at `<4.0.0` ([#2153](https://github.com/apify/crawlee-python/pull/2153), 2026-08-12).
- A collaborator then wrote: "Postponed until v2… v4 does not support Python 3.10, so we cannot make the transition at this time" ([#2154](https://github.com/apify/crawlee-python/issues/2154), 2026-09-01).
- Related lifecycle fix: "do not close a browser as inactive while a page opening is in flight" ([#2156](https://github.com/apify/crawlee-python/pull/2156)).

**Scrapy** (scrapy-playwright v0.0.48, 2026-07-10) **C** ([README](https://github.com/scrapy-plugins/scrapy-playwright/blob/main/README.md)):
- **`PLAYWRIGHT_CDP_URL`** connects to a remote Chromium with `connect_over_cdp`. With it, "all non-persistent contexts will be created on the connected remote browser" and `PLAYWRIGHT_LAUNCH_OPTIONS` "is ignored".
- **`PLAYWRIGHT_BROWSER_PROVIDER`** is "an extension point for integrating third-party drivers that expose Playwright-compatible `Browser`/`BrowserContext`/`Page` objects (e.g. patchright, camoufox) without changing the handler".
- **Remote downloads failed** with "Path is not available when using browser_type.connect()" until `save_as()` replaced `path()` ([#388](https://github.com/scrapy-plugins/scrapy-playwright/issues/388), [#389](https://github.com/scrapy-plugins/scrapy-playwright/pull/389), 2026-09-01).

**Browser Use inside a crawler** **C**:
- No Crawlee or Scrapy integration was found: an issue search of `apify/crawlee-python` for Browser Use returned none.
- A 2025 pull request to let Browser Use take over an existing Playwright page "for single AI actions" was closed unmerged ([#1076](https://github.com/browser-use/browser-use/pull/1076), 2025-03-19). Since dropping Playwright, Browser Use takes a `cdp_url` only (§2.5), so a crawler would lend it the browser, with the browser-wide changes listed there.

**What goes wrong in practice**, grouped as the ticket asks:
- **Sessions and cookies:** the crawler's session store and the tool's view of cookies diverge, because Crawlee saves cookies before the handler runs **C**.
- **Proxies:** the tool's launch or context settings win; Crawlee ignores context and proxy options for later pages **C**.
- **Stealth:** fingerprints are partly applied (Crawlee TS), and Crawlee Python supports "consistent HTTP header generation only" **C**.
- **Lifecycle:** page-registration races between two clients, browsers closed while a page was opening, and a version lockstep that broke Python users on 4.0.0 **C**.
- **Double counting:** nothing published was found on pages, tokens or cost counted twice when a tool runs inside a crawler (**Unverified**).

---

## 5. Leading practice for lending a session to third-party code

### 5.1 What Chrome itself does

**C**:
- **The debugging port has no authentication.** Its only defences are request checks: the `Host` header must be an IP address or `localhost`, and a WebSocket whose `Origin` is not same-origin is rejected unless `--remote-allow-origins` lists it, or `*` allows all ([devtools_http_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/devtools_http_handler.cc)). Stagehand's launcher passes `--remote-allow-origins=*` (§2.3).
- **Chrome 136 refuses the port on the default data directory.** Chrome 144 adds consented remote debugging with an "Allow remote debugging?" dialog per connection. These are already cited in ADRs 0017 and 0056 ([Chrome blog](https://developer.chrome.com/blog/remote-debugging-port), [chrome-devtools-mcp#1794](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1794)).
- **The only reduced-privilege client is the extension client** (§3.1): it is untrusted, cannot attach to the browser target, cannot write local files, has URL checks, and shows a visible infobar.

### 5.2 What libraries and vendors do

**C**:
- **Library guardrails say they are not boundaries.**
  - Puppeteer's `allowlist`/`blocklist`: "not a complete network sandbox… we recommend using container/OS-level sandbox mechanism" (§2.2).
  - Playwright MCP calls its own origin filter and secrets handling "not a security boundary", and Stagehand's docs say "Use Browserbase as the isolation boundary for untrusted tasks" ([AI browser frameworks note](https://github.com/goodfetch/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md)).
  - Stagehand's `setDomainPolicy` misses service-worker fetches and WebSockets ([stagehand#2945](https://github.com/browserbase/stagehand/issues/2945)).
- **Vendors scope by session and by time, not by command** (§3.4): a bearer URL, one client per session (Browserless) or keep-alive sharing (Browserbase), one domain per session (Bright Data), top-frame allowlists (Browserbase), idle cuts after 5 to 10 minutes, and a reconnect token (Browserless `auth`).
- **Action-level policy** is the richest option shipped in an open-source agent tool: Vercel agent-browser's vault, allowlist covering workers and WebRTC, action policies and confirmations (AI browser frameworks note). It works above CDP, not on raw CDP.

### 5.3 Against Goodfetch's ADRs

| Practice | Where it stands in Goodfetch | What this research adds |
|---|---|---|
| Scoped, short-lived token per attach | ADR 0003: one token per attach, scoped to the work item's Session and bound to its lease; ADR 0017: per-Session, short-lived, revocable | Stricter than any vendor documents (§3.4). Browser Use retries a cut attach 3 times over up to 54 s (§2.5), so a revoked token must refuse those retries |
| Cut the attach when the work item ends | ADR 0003: answers in-flight commands with `attach_ended`, closes the connections, revokes the token; the Browser host cuts on its own clock | Cutting also releases what the lent client held: paused requests and paused OOPIFs and workers resumed when it disconnected **M**. It does not undo state it changed: downloads stayed reset and emulation stayed cleared **M** |
| Egress through a proxy | ADR 0017: an engine-owned egress proxy under a Session with raw CDP, enforcing the policy "at the network layer" | A browser-level client can create a context with its own `proxyServer` and bypass a proxy set by launch flag **M**. The proxy holds only if it is the browser's only route out. It can also install an extension whose traffic page-level interception never sees **M** |
| Command allowlist | ADR 0003 rejected checking each raw CDP message against the lease, because it "stops nothing more than cutting the attach does" | Chrome offers no reduced-trust mode for a WebSocket client (§3.1). Target scoping, blocking `Browser.close` and stripping `proxyServer` all need a relay that parses messages, for scoping rather than fencing. Page-level endpoints are no substitute: Playwright and Puppeteer cannot use them **M** |
| Mark the run | ADRs 0017 and 0056: unguarded results, the run marked as sharing its browser, no Session back to the pool by itself | Playwright, Puppeteer, chromiumoxide and BiDi on Chrome enable `Runtime` on every page they wrap **C**, so the Session's stealth posture changes too, not only its guard |

---

## 6. Experiment (localhost, measured)

**Setup.** macOS, Node v24.19.0, Google Chrome **155.0.8059.40** headless (`--headless=new --remote-debugging-port=0`), each run on a fresh temporary profile whose `Preferences` point default downloads into that profile. Playwright **1.63.0** and puppeteer-core **25.12.0** came from the lab's installed packages, and the Stagehand **4.1.0** extension from its installed bundle; nothing was installed. A local HTTP server on `127.0.0.1` served every page.

The "owner" is a raw CDP client over the browser WebSocket. It creates and attaches its own page, never enables `Runtime`, reads the page through an isolated world, sets downloads to its own folder and emulates 1111×777. Command:
```
LAB_NODE_MODULES=<rnd/lab/adapters/node/node_modules> node rnd/research/2026-10-14-driver-interop/experiment.mjs
```
Raw output: [`raw-output.jsonl`](2026-10-14-driver-interop/raw-output.jsonl). The last run took about 45 s. Playwright 1.63.0 rather than 1.64.1 was measured; the code paths involved (download behaviour, `noDefaults`, close) are the same in both, from source.

| Case | Result |
|---|---|
| P1 owner alone | download in the owner's folder; page 1111×777 |
| P2 Playwright `connectOverCDP`, defaults | one context holding the owner's page **and** a page from the owner's separate context; size kept; download went to `playwright-artifacts-*/<guid>` |
| P2 after Playwright `close()` | browser alive; the owner's download setting gone (file in Chrome's default folder) |
| P3 Playwright with `noDefaults: true` | download in the owner's folder |
| P4 Puppeteer `connect`, defaults | owner's page 800×600 |
| P4 after `disconnect()` | page 756×469 (owner's emulation cleared); downloads unaffected |
| P5 Fetch on both, second client rewrites `/fetch-a` | second client saw `/fetch-a`, the owner saw `/fetch-b-rewritten`, the server got `/fetch-b-rewritten`. Same with the owner re-sending `Fetch.enable` (P5d) and with the second client enabling first (P5e) |
| P5b second client never continues | owner saw nothing, the server got nothing |
| P5c second client disconnects | request reached the server, and the owner saw it |
| P6 second client: browser-level `waitForDebuggerOnStart`, never resumes | the owner's new tab loaded and ran its script anyway |
| P6b second client: page-level `waitForDebuggerOnStart`, never resumes | iframe and worker attached "paused"; their scripts ran only after it disconnected |
| P7 Puppeteer `close()` on the connected browser | owner's socket closed; Chrome exited with code 0 |
| P8 `Extensions.loadUnpacked` (Stagehand 4.1.0 extension) | loaded, service worker attached, `typeof chrome.debugger` = `"object"`, with and without `--enable-unsafe-extension-debugging` |
| P9 browser launched with `--proxy-server=http://127.0.0.1:9 --proxy-bypass-list=<-loopback>` | default-context page never reached the server; a second client's context with `proxyServer: "direct://"` did |
| P10 page-level endpoint `/devtools/page/<id>` | Playwright: connected, 0 pages; Puppeteer: `Target.getBrowserContexts: Not allowed` |
| Proxy-prototype console probe (all phases) | 0 trap hits, with or without a Runtime-enabled client |

**Side effect.** The first run, before the profile's default download folder was redirected, left a 5-byte `~/Downloads/report.txt` containing `hello`. Deleting it was blocked by permissions, so it is still there. Two temporary folders from an interrupted run also remain in the system temp directory (`interop-profile-*`, `owner-dl-*`).

---

## 7. Implications for Goodfetch

These are considerations for the planning map, not decisions.

1. **Lending "a page" means a relay that plays a browser.** Playwright and Puppeteer cannot use a page-level endpoint (P10). On a browser endpoint they adopt every page, the owner's included (P2). Lending one handler's page while other work items run in the same browser therefore needs the endpoint of ADRs 0003 and 0017 to:
   - answer browser-level commands;
   - filter `Target.*` events and commands to the lent page's tree (its OOPIFs, workers and popups);
   - hide other contexts.

   That is message parsing for scoping, which ADR 0003's rejection of per-message lease checks did not weigh. Without it, lending a page lends the browser and every Session in it.
2. **A small set of commands decides whether a lent handler can break the crawl:**
   - `Browser.close` (Puppeteer, Stagehand v4 and chromiumoxide send it on `close()`);
   - `Target.createBrowserContext` with `proxyServer` (bypasses a proxy flag, P9);
   - `Extensions.loadUnpacked` (installs a privileged agent inside the browser, P8);
   - `Browser.setDownloadBehavior` and `Browser.grantPermissions` (browser-wide, outlive the attach);
   - `Security.setIgnoreCertificateErrors`, `Target.exposeDevToolsProtocol` and `Target.attachToBrowserTarget`.

   Whether the endpoint refuses, rewrites or only records these is the decision; the evidence is that Chrome offers no reduced-trust mode for such a client (§3.1).
3. **The owner has to re-assert its own state after every lent attach ends.** Chromium resets downloads to `default` and clears device emulation when the guest's session goes, whoever set them (P2, P4). Engine-owned overrides need to be re-applied on detach, or held where a guest cannot write them.
4. **Interception order favours the owner, but a guest can stall it.** The owner's earlier session saw the request a guest had rewritten, in every order tried (P5). A guest that pauses requests (P5b) or child targets (P6b) blocks the page until it disconnects. ADR 0003's cut at lease end releases both (P5c, P6b), so the lease timeout bounds how long a guest can hold a crawl's page.
5. **The egress proxy has to be the only way out, not a Chrome setting.** P9 shows a browser-level guest can open a direct context. ADR 0017's "network layer" wording fits a proxy enforced outside the browser: OS sandbox, network namespace or firewall. It does not fit `--proxy-server` alone. On Borrowed browsers (ADR 0056), interception is all there is, and a guest that installs an extension (P8) gets traffic interception never sees.
6. **Goodfetch's attach helpers (ADR 0003) can set the safe options:**
   - Playwright `noDefaults: true`;
   - Puppeteer `defaultViewport: null`;
   - mapping `close()` to disconnect;
   - Browser Use `downloads_path` and an empty `permissions` list (**Unverified** that it skips the grant).

   These cover cooperative code; they do not bind a guest that ignores the helper.
7. **ADR 0019's WebSocket premise is outdated for Node.** Since Playwright 1.61, Node `connectOverCDP(transport)` attaches over any message channel, locally (§2.1). The TS SDK could hand Playwright a CDP channel over its existing JSON-RPC connection with no listening port, as `goodfetch.borrow()` already does in the other direction (ADR 0056). Python Playwright still needs a URL.
8. **Stagehand v4 can only be given a whole browser.** It refuses foreign pages, installs its extension and drives tabs from inside (§2.3). In Foxhound terms, that is a Borrowed or hosted browser per work item, not a lent page. Stagehand v3 (Node driver, takes Playwright pages) is the version that fits a lent page, and Crawlee itself is stuck on v3.
9. **Most guests enable Runtime on the pages they touch.** Playwright, Puppeteer, chromiumoxide and BiDi on Chrome do; Browser Use's session code does not; what Stagehand v4's extension enables on pages was not checked. Provenance already marks the results unguarded, and the Session stays out of the pool (ADR 0017). The stealth side, a Runtime signal on a Session that was clean, is a separate fact worth recording, since a later run reusing a saved identity inherits a site's view of it.
10. **For Collie inside other crawlers, the working pattern is Crawlee's**: the tool launches, the crawler attaches Playwright. Its known costs are a page-registration race between two clients, ignored context options, partial fingerprints and version lockstep. Two ways in:
    - scrapy-playwright's `PLAYWRIGHT_BROWSER_PROVIDER` takes any Playwright-compatible driver, which Goodfetch's Playwright-shaped page API (ADR 0019) might satisfy;
    - Crawlee's `BrowserPlugin` is the TS seam.

    The reverse, Collie borrowing the crawler's page, is ADR 0056's `client_browser` and inherits everything in §3.

---

## 8. Unverified / open

- Whether the per-session Fetch chain is ordered by session attach time; three orderings matched that, but the Chromium comment only says creation order.
- Whether stable headed Chrome (not headless) also lets a port client load an unpacked extension and debug its service worker without `--enable-unsafe-extension-debugging` (P8 ran headless).
- Whether Chrome 155 still has a separate gate on debugging extension targets that Stagehand's docs describe; none was hit.
- Why our Proxy-prototype console probe never fired: a V8 change or a probe unlike the published one.
- Which CDP domains Stagehand v4's extension enables on the pages it drives (Runtime included), through `chrome.debugger`.
- Stagehand v3's exact attach-time side effects and close semantics on a `cdpUrl` browser.
- ChromeDriver's attach-time side effects with `debuggerAddress`, and what `quit()` does to a browser it attached to.
- Steel's and Bright Data's rules for several clients on one session.
- Whether any cloud vendor filters or allowlists user CDP commands; none documents it.
- Whether Browser Use with `permissions=[]` skips `Browser.grantPermissions` entirely.
- Published reports of double counting (pages, tokens, cost) when an AI tool runs inside a crawler; none found.
- Firefox: whether a lent page can share a BiDi session's WebSocket in practice (the spec allows several connections per session; Firefox allows one session).

---

## 9. Sources

All read on 2026-10-14 unless stated.

**Playwright** (v1.64.1 unless stated): [chromium.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts), [crBrowser.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crBrowser.ts), [crPage.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crPage.ts), [crNetworkManager.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/crNetworkManager.ts), [browser.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/browser.ts), [client browserType.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/browserType.ts), [overrides.d.ts](https://github.com/microsoft/playwright/blob/v1.64.1/utils/generate_types/overrides.d.ts), [class-browsertype.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md), [class-browser.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browser.md), [v1.61.0 release](https://github.com/microsoft/playwright/releases/tag/v1.61.0), [Playwright Python _browser_type.py v1.64.0](https://github.com/microsoft/playwright-python/blob/v1.64.0/playwright/_impl/_browser_type.py). Issues and PRs: [#32821](https://github.com/microsoft/playwright/issues/32821), [#37627](https://github.com/microsoft/playwright/issues/37627), [#39011](https://github.com/microsoft/playwright/issues/39011), [#40158](https://github.com/microsoft/playwright/issues/40158), [#40177](https://github.com/microsoft/playwright/pull/40177), [#40185](https://github.com/microsoft/playwright/pull/40185), [#40972](https://github.com/microsoft/playwright/pull/40972), [#41060](https://github.com/microsoft/playwright/issues/41060), [#41286](https://github.com/microsoft/playwright/issues/41286), [#42882](https://github.com/microsoft/playwright/issues/42882), [#42936](https://github.com/microsoft/playwright/pull/42936), [#29065](https://github.com/microsoft/playwright/issues/29065), [#42776](https://github.com/microsoft/playwright/issues/42776).

**Puppeteer** (puppeteer-core v25.13.0): [ConnectOptions.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/common/ConnectOptions.ts), [BrowserConnector.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/BrowserConnector.ts), [Browser.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/Browser.ts), [TargetManager.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/TargetManager.ts), [Page.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/Page.ts), [FrameManager.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.13.0/packages/puppeteer-core/src/cdp/FrameManager.ts).

**Stagehand** (`main` at `1da7359`, 2026-10-13): [cdpClient.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/cdpClient.ts), [factories.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/browser/factories.ts), [browser.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/browser.mdx), [playwright.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx), [extension manifest](https://github.com/browserbase/stagehand/blob/main/packages/extension/manifest.json), [npm 3.7.3](https://www.npmjs.com/package/@browserbasehq/stagehand/v/3.7.3), [#2945](https://github.com/browserbase/stagehand/issues/2945); the 4.1.0 npm bundle (`dist/index.mjs`, `dist/extension/manifest.json`) installed in `rnd/lab`; the salvaged ticket #448 report.

**Browser Use** (0.13.11): [session.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session.py), [session_manager.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session_manager.py), [profile.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py), [downloads_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/downloads_watchdog.py), [permissions_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/permissions_watchdog.py), [#1076](https://github.com/browser-use/browser-use/pull/1076).

**Selenium, ChromeDriver, BiDi:** [ChromeDriver: operation not supported when using remote debugging](https://developer.chrome.com/docs/chromedriver/help/operation-not-supported-when-using-remote-debugging), [Selenium CDP docs](https://www.selenium.dev/documentation/webdriver/bidi/cdp/), [chromium-bidi CdpTarget.ts v17.0.2](https://github.com/GoogleChromeLabs/chromium-bidi/blob/chromium-bidi-v17.0.2/src/bidiMapper/modules/cdp/CdpTarget.ts), [WebDriver BiDi index.bs @f922e6a](https://github.com/w3c/webdriver-bidi/blob/f922e6af669ead51496db85f37813f39438ef171/index.bs), [Firefox WebDriverBiDi.sys.mjs @3934f78](https://github.com/mozilla-firefox/firefox/blob/3934f7874d/remote/webdriver-bidi/WebDriverBiDi.sys.mjs).

**chromiumoxide** (`a7e2bb8`): [handler/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/mod.rs), [handler/target.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/target.rs), [handler/network.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/network.rs), [browser/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/browser/mod.rs).

**Chromium and the protocol** (Chromium `main` at `3ea9312`, protocol r1718299): [devtools_agent_host_client.h](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/public/browser/devtools_agent_host_client.h), [target_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/target_handler.cc), [emulation_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/emulation_handler.cc), [browser_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/browser_handler.cc), [devtools_download_manager_delegate.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/protocol/devtools_download_manager_delegate.cc), [devtools_url_loader_interceptor.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/devtools_url_loader_interceptor.cc), [devtools_http_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/content/browser/devtools/devtools_http_handler.cc), [debugger_api.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/extensions/api/debugger/debugger_api.cc), [chrome_devtools_session.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/devtools/chrome_devtools_session.cc), [extensions_handler.cc](https://chromium.googlesource.com/chromium/src/+/3ea93124568e116f89d7493360b8e7257c78e186/chrome/browser/devtools/protocol/extensions_handler.cc), [Target.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Target.pdl), [Browser.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Browser.pdl), [Extensions.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/dfad770693/pdl/domains/Extensions.pdl), [DevTools 63 (multi-client)](https://developer.chrome.com/blog/new-in-devtools-63), [Chrome remote-debugging changes](https://developer.chrome.com/blog/remote-debugging-port).

**Cloud browsers:** Browserbase [create a session](https://docs.browserbase.com/reference/api/create-a-session.md), [using a session](https://docs.browserbase.com/platform/browser/getting-started/using-browser-session.md), [keep alive](https://docs.browserbase.com/platform/browser/long-sessions/keep-alive.md), [manage a session](https://docs.browserbase.com/platform/browser/getting-started/manage-browser-session.md), [allowed domains](https://docs.browserbase.com/platform/browser/security/allowed-domains.md); Browserless [persisting state](https://docs.browserless.io/baas/session-management/persisting-state), [llms-full.txt](https://docs.browserless.io/llms-full.txt); Steel [llms-full.txt](https://docs.steel.dev/llms-full.txt); Bright Data [error codes](https://docs.brightdata.com/products/scraping-browser/error-codes.md), [standard CDP functions](https://docs.brightdata.com/products/scraping-browser/cdp-functions/standard.md), [configuration](https://docs.brightdata.com/products/scraping-browser/configuration.md), [KYC and access](https://docs.brightdata.com/scraping-automation/scraping-browser/kyc-and-access).

**Crawlers:** [crawlee#4196](https://github.com/apify/crawlee/issues/4196), [crawlee#3865](https://github.com/apify/crawlee/pull/3865), [crawlee-python#2153](https://github.com/apify/crawlee-python/pull/2153), [crawlee-python#2154](https://github.com/apify/crawlee-python/issues/2154), [crawlee-python#2156](https://github.com/apify/crawlee-python/pull/2156), [scrapy-playwright README](https://github.com/scrapy-plugins/scrapy-playwright/blob/main/README.md) (v0.0.48), [scrapy-playwright#388](https://github.com/scrapy-plugins/scrapy-playwright/issues/388), [#389](https://github.com/scrapy-plugins/scrapy-playwright/pull/389).

**Goodfetch:** ADRs [0003](../../docs/adr/0003-two-level-fencing-at-least-once-delivery.md), [0017](../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md), [0019](../../docs/adr/0019-direct-cdp-driver-with-the-guard-as-reference.md), [0022](../../docs/adr/0022-remote-browsers-driven-through-a-browser-host.md), [0035](../../docs/adr/0035-stealth-is-a-settable-catalog-led-by-a-goodfetch-built-chromium.md), [0056](../../docs/adr/0056-a-login-is-brought-in-through-one-session-source-read-by-the-browser-that-owns-it.md); [GLOSSARY.md](../../GLOSSARY.md).
