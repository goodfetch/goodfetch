# Browser-automation products and where they draw their boundary

Research of **October 14, 2026** for ticket [#448](https://github.com/goodfetch/automan/issues/448). Question: what do browser-automation SDKs, Collie's direct competitors, ship as a product, and where do they draw their boundary? For each product: what the box contains (driver, AI primitives, caching or replay, agent loop, MCP server, CLI, stealth, proxies, session persistence, storage, recording, UI), install size per platform, languages, how it attaches to a browser or page it did not launch, how crawlers use it, what users complain about, and which Goodfetch capabilities have an equivalent. It feeds the tickets that decide Collie's capability set and the product boundary between Collie and Foxhound. Like everything in `rnd/`, this note informs a decision; it does not make one.

**Sources.** Registry metadata read on 2026-10-14: npm (`npm view`, `npm pack`, downloads API for the week 2026-10-06 to 2026-10-12), PyPI JSON and pypistats, crates.io, the Go module proxy, Maven Central and NuGet; GitHub release assets, tagged source trees and issue trackers through `gh api`; official documentation, read from the repositories' own docs sources where they exist. Browser archive sizes are HTTP `Content-Length` on the vendors' CDN URLs. Earlier notes reused and re-checked: [AI browser frameworks](https://github.com/goodfetch/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md) (2026-10-10), [Rust CDP clients](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md), the [parity list](https://github.com/goodfetch/automan/blob/research/parity-list/rnd/research/2026-10-10-parity-list.md), [browser frameworks](2026-10-07-browser-frameworks.md), [Stagehand alternatives](2026-10-07-stagehand-alternatives-followup.md), [Stagehand CDP latency](2026-10-07-stagehand-cdp-latency.md) and [agent capability matrix](2026-10-07-agent-capability-matrix.md).

**Measured versus cited.** Facts marked **M** were measured for this note: registry sizes, tarball and wheel listings, release-asset sizes, CDN `Content-Length`. Everything else is cited from a primary source, linked inline. No package was run and no browser was started.

**Versions inspected.** Stagehand 4.2.1 (TypeScript, Python and Go, 2026-10-10/11); Playwright 1.64.1 (2026-10-14), Playwright Python 1.64.0, Java and .NET 1.63.0, `@playwright/mcp` 0.0.83, `@playwright/cli` 0.1.22; Puppeteer 25.13.0 (2026-10-08), `chrome-devtools-mcp` 1.10.1; Browser Use 0.13.11 (2026-10-07); Skyvern 1.0.56 (2026-10-14); Midscene 1.15.4 (2026-10-14); agent-browser 0.39.1 (2026-10-13); Steel `steel-browser` v0.5.4-beta, `steel-sdk` 0.18.0/0.19.0; Notte 1.9.7 (2026-10-14); Magnitude `magnitude-core` 0.3.1 (2026-02-08); Lightpanda 1.0.0 (2026-10-02); chromiumoxide 0.9.1, chromey 2.58.2.

---

## 1. Summary

_Draft: the summary is completed once every section is in._

---

## 2. Stagehand v4

**What it is.** "Browserbase's SDK for building browser agents" ([package.json](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/package.json)), MIT, 25,664 stars. npm `latest` is **4.2.1** (2026-10-11); 4.0.0 shipped 2026-08-10, and the v3 line is still patched under the `v3-latest` tag (3.7.3, 2026-08-28) ([npm](https://registry.npmjs.org/@browserbasehq/stagehand)). PyPI `stagehand` 4.2.1 (2026-10-10) and the Go module `github.com/browserbase/stagehand/packages/sdk-go/v4` v4.2.1 ship from the same monorepo. There is no GitHub Release for v4; it is announced in the [CHANGELOG](https://github.com/browserbase/stagehand/blob/main/CHANGELOG.md) as "a protocol-first monorepo with TypeScript, Python, and Go SDKs over a shared core". Downloads: 1,640,981 npm and 469,656 PyPI in the week to 2026-10-12.

### 2.1 What is in the extension, the protocol package and each SDK

The monorepo's `packages/` holds `extension`, `protocol`, `sdk-ts`, `sdk-python`, `sdk-go`, `cli`, `integrations`, `docs`, `evals` and `examples` ([tree](https://github.com/browserbase/stagehand/tree/main/packages)). SDKs, extension and protocol are versioned separately: SDKs 4.2.1, Extension Runtime 1.1.1, Protocol 2.1.1.

| Package | Published as | Contents |
|---|---|---|
| `extension` | private (`@browserbasehq/stagehand-extension` 1.1.1), never published alone | An MV3 extension, "Stagehand Runtime", `minimum_chrome_version` 116, permissions `debugger`, `offscreen`, `scripting`, `tabs` on `<all_urls>`, with a content script in every frame at `document_start` in the isolated world ([manifest.json](https://github.com/browserbase/stagehand/blob/main/packages/extension/manifest.json)). It holds **the whole runtime**: the CDP driver ("understudy": frames, locators, deep locators through OOPIFs and closed shadow roots, network manager, domain policy, cookies, clipboard, PDF), the `act`/`observe`/`extract` services, the LLM clients (Vercel AI SDK `ai` with `@ai-sdk/{openai,anthropic,google,groq,cerebras}`), the cache client and OpenTelemetry ([package.json](https://github.com/browserbase/stagehand/blob/main/packages/extension/package.json)). **Model calls run inside the browser's extension service worker**, not in the caller's process. |
| `protocol` | private (`@browserbasehq/stagehand-protocol` 2.1.1) | Bidirectional JSON-RPC defined in Zod, exported as `stagehand.v4.json` (195 KB), from which Python models and Go structs are generated. The protocol has its own SemVer: majors must match and the server minor must be at least the client minor ([protocol README](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md)). |
| `sdk-ts` | `@browserbasehq/stagehand` | Node ≥22.18. Four dependencies: `zod` 4.4.3, `@browserbasehq/sdk`, `@opentelemetry/api`, `@opentelemetry/core`. Ships the built extension twice: unpacked under `dist/extension/` and as `dist/assets/stagehand-extension.zip` **M**. |
| `sdk-python` | `stagehand` | Python ≥3.11, async only, pure Python: `browserbase`, `httpx`, `opentelemetry-api`, `pydantic`, `typing_extensions`, `websockets`. The wheel embeds the extension under `stagehand/_extension/` ([pyproject](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/pyproject.toml)). |
| `sdk-go` | Go module `…/sdk-go/v4` | Go 1.26; embeds the extension zip with `//go:embed` and unpacks it to a temp directory for a local Chrome ([assets.go](https://github.com/browserbase/stagehand/blob/main/packages/sdk-go/internal/extensionassets/assets.go)). |
| `cli` | `browse` 0.11.1 | The "Unified Browserbase CLI" (oclif): a per-session daemon started by the first command, about 30 DOM commands over snapshot refs, `network on` capture to disk, a skills catalog backed by browse.sh, `cloud …` for Browserbase sessions, contexts, fetch and search, and `functions init/dev/publish` for Browserbase Functions. It still pins `@browserbasehq/stagehand` 4.1.0 ([README](https://github.com/browserbase/stagehand/blob/main/packages/cli/README.md)). 13,836 npm downloads a week. |
| `integrations` | private, not on npm | A three-tool facade (`run`, `snapshot`, `screenshot`) behind a stdio MCP server or in-process tools, with examples for Claude Agent SDK, Codex, Cursor, CrewAI, Deep Agents, Eve, fx, Mastra, Pi and the Vercel AI SDK. `run` executes model-written JavaScript against a Playwright-shaped facade ([README](https://github.com/browserbase/stagehand/blob/main/packages/integrations/README.md)). |

**Transport.** Each SDK attaches a CDP session to the extension's service-worker target and sends JSON-RPC messages through it (`Runtime.evaluate` on that target in the 4.2.1 `dist/index.mjs`). This is why every local launch adds `--enable-unsafe-extension-debugging`: it is "required so the SDK can attach a CDP session to the service worker hosting Stagehand's runtime" ([browser.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/browser.mdx)). The RPC is bidirectional, which carries the "bring your own LLM" callback: anything other than the five bundled providers (Azure, Bedrock, self-hosted, any base URL) is called back on the client ([models.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/models.mdx)). Each SDK also keeps its own deterministic CDP client for page and locator calls.

**What v4 removed.** "`agent()` is gone. Nothing in v4 replaces it one-for-one." The replacements are code mode (a coding assistant writes a Stagehand script once) and tool calling over the full API ([v3 migration](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/v3.mdx)). Also gone: the local cache (`enableCaching`, `cacheDir`) and every Playwright, Puppeteer and Patchright peer dependency.

### 2.2 Install footprint

| Artifact | Size | Notes |
|---|---|---|
| npm `@browserbasehq/stagehand` 4.2.1 | **3.51 MB** unpacked, 16 files **M** | `dist/extension/service-worker.js` alone is 2.08 MB (the runtime with AI SDK and OTel); the zip copy is 0.45 MB; `index.mjs` 219 KB **M** |
| npm 4.2.1 with dependencies | about 10–11 MB | Sum of direct dependencies' `unpackedSize` (zod 4.56 MB, OTel 1.58 MB, Browserbase SDK 0.80 MB); not a measured `node_modules` |
| npm 3.7.3 (v3, for contrast) | 10.19 MB unpacked, 1,174 files **M** | Plus `ai`, `openai`, `@anthropic-ai/sdk`, `@google/genai`, MCP SDK, pino, 16 optional `@ai-sdk/*` providers and peers on `playwright-core`, `puppeteer-core`, `patchright-core` ([npm 3.7.3](https://registry.npmjs.org/@browserbasehq/stagehand/3.7.3)) |
| PyPI `stagehand` 4.2.1 | **0.48 MB**, one universal wheel **M** | No per-platform wheels, no bundled Node |
| PyPI `stagehand` 3.23.0 (v3) | 30.9–37.4 MB per-platform wheels | Each embedded a ~103 MB Node single-executable server, because v3 Python was a client of the Stagehand API ([PyPI 3.23.0](https://pypi.org/pypi/stagehand/3.23.0/json)) |
| Go module v4.2.1 | 0.68 MB zip, of which 0.45 MB is the embedded extension | [proxy.golang.org](https://proxy.golang.org/github.com/browserbase/stagehand/packages/sdk-go/v4/@v/v4.2.1.zip) |
| Browser | **none downloaded** | "Local runs need Chrome installed" ([README](https://github.com/browserbase/stagehand/blob/main/README.md)); the Go launcher searches Chrome Canary, Stable and Beta paths or `CHROME_PATH` ([chrome_launcher.go](https://github.com/browserbase/stagehand/blob/main/packages/sdk-go/chrome_launcher.go)) |

The move to an extension runtime cut the Python install by about 70× and the npm package by about 3×: the runtime is 2 MB of JavaScript that any Chrome executes, so no SDK needs Node.

### 2.3 Local, Browserbase, and a browser someone else launched

Three browser factories feed `Stagehand.create({ browser })` ([browser.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/browser.mdx)):

1. **`localBrowser.launch()`** starts the installed Chrome with a debugging port, a temporary profile unless `userDataDir` is set, about 25 automation flags plus `--enable-unsafe-extension-debugging`, `--remote-allow-origins=*` and `--enable-features=WebMCPTesting,DevToolsWebMCPSupport`. The extension is loaded over CDP with `Extensions.loadUnpacked`, not `--load-extension`. Authenticated local proxies "are not supported yet".
2. **`localBrowser.connect({ cdpUrl })`** attaches to "any Chromium browser you are already running". It "discovers an installed Stagehand extension first, then loads the bundled extension if none is found… Loading requires a shared filesystem." "Stagehand never closes a browser it did not launch." So a borrowed browser works only if it accepts `Extensions.loadUnpacked` and extension-target debugging, or already carries the extension.
3. **`browserbase.launch({ apiKey })`** uploads the extension zip to Browserbase's Extensions API and starts a session with it; "Extensions cannot be added after a browser session starts". `browserbase.connect()` needs a session created with the extension.

**No page interop.** "You can't hand a Playwright `Page` to `act()`. There is no interop; moving a flow to Stagehand means porting it." Firefox and WebKit: "Not supported. Chromium only" ([Playwright migration](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx)).

**What is Browserbase-only.** The docs' own comparison table lists, for Browserbase against local: server-side caching "Supported / Not available", Model Gateway "Supported / Not available", stealth "Advanced fingerprinting / Basic stealth", "Built-in residential proxies / Manual configuration", "Cloud context storage / File-based user data", and "Session recordings and logs / Direct DevTools access" ([browser.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/browser.mdx)). "With a local browser… the `cache` option has no effect and every call runs inference" ([caching.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/best-practices/caching.mdx)). Ad blocking, CAPTCHA solving, fingerprints and Verified are Browserbase session settings in the SDK's schema, gated by plan: the free plan has "No captcha solving or stealth mode" ([pricing](https://www.browserbase.com/pricing)). The deployment guide recommends deploying Stagehand "as a Browserbase Function" ([deployments.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/best-practices/deployments.mdx)). The hosted MCP at `mcp.browserbase.com` is what the README promotes; the self-hostable `mcp-server-browserbase` is archived (2026-07-20) and `@browserbasehq/mcp` 3.0.0 still depends on Stagehand v3.

### 2.4 What it leaves to Crawlee or the user

- **No crawler, queue, dataset, key-value store, scheduler or retry policy.** The cookbooks' "paginated catalog" and "files to bucket" are hand-written loops ([cookbooks](https://github.com/browserbase/stagehand/tree/main/packages/docs/v4/cookbooks)).
- **No test runner:** "no fixtures, no `expect()`, no retries, no HTML reporter, no `codegen`, and no trace viewer" ([Playwright migration](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx)).
- **No Playwright-grade driver guarantees:** no auto-waiting, no `getBy*`, no route interception, no `waitForResponse`, `page.on()` for `"console"` only, one context per browser (same page). `page.pdf()` arrived in 4.2.0.
- **Crawlee's integration is on v3.** `@crawlee/stagehand` 3.18.2 peer-depends on `@browserbasehq/stagehand ^3.0.0` and `playwright`; it calls `new Stagehand()`, `init()`, `connectURL()` and then `chromium.connectOverCDP` "to get a compatible handle" ([stagehand-plugin.ts](https://github.com/apify/crawlee/blob/master/packages/stagehand-crawler/src/internals/stagehand-plugin.ts)). Each of those calls was removed in v4, and no v4 port was found ([apify/crawlee#4196](https://github.com/apify/crawlee/issues/4196), open).

### 2.5 Complaints

Issue counts are low because the project uses Discord, and v4-era issues carry almost no reactions (402 issues, 104 open, 2026-10-14).

- **Attach to existing Playwright code:** [#507](https://github.com/browserbase/stagehand/issues/507) (18 reactions), now ruled out by "no interop".
- **Model breadth:** Ollama [#183](https://github.com/browserbase/stagehand/issues/183) (19), AI SDK LanguageModelV3 [#1645](https://github.com/browserbase/stagehand/issues/1645) (16, open). v4 still has "no base URL option".
- **Replay without a model:** "Return plan for extraction" [#389](https://github.com/browserbase/stagehand/issues/389) (16, open), XPath caching [#404](https://github.com/browserbase/stagehand/issues/404).
- **Open core:** [#492](https://github.com/browserbase/stagehand/issues/492) asks to make the local version complete or disclose it as an on-ramp to Browserbase.
- **Leaving Playwright:** [#2744](https://github.com/browserbase/stagehand/issues/2744): 13 of 15 production scripts blocked by missing `pdf()`, network events and emulation.
- **Egress holes:** `setDomainPolicy` is bypassed by service-worker fetches and WebSockets [#2945](https://github.com/browserbase/stagehand/issues/2945) and fails open by default [#3002](https://github.com/browserbase/stagehand/issues/3002).
- **v4 reliability:** `create` times out on Browserbase with the extension preloaded [#2782](https://github.com/browserbase/stagehand/issues/2782); `locator.click()` resolves without committing a selection [#2486](https://github.com/browserbase/stagehand/issues/2486); Windows close [#3166](https://github.com/browserbase/stagehand/issues/3166); Deno extension upload [#3152](https://github.com/browserbase/stagehand/issues/3152); a leaked uploaded extension in Go [#2725](https://github.com/browserbase/stagehand/issues/2725).
- **Logging:** `onLog` cannot be used without every log also going to stderr [#3035](https://github.com/browserbase/stagehand/issues/3035) (9, the most-reacted v4 issue).
- **Not found:** no issue complains about the extension architecture itself, the v4 install size or the removal of `agent()`.

---

## 3. Playwright

**What it is.** Microsoft's browser library and test runner, Apache-2.0, **1.64.1** (2026-10-14) ([release](https://github.com/microsoft/playwright/releases/tag/v1.64.1)). It is the volume leader: 125.6M `playwright-core` and 113.6M `playwright` npm downloads a week, 26.4M PyPI.

### 3.1 Packaging

| Package | Version | Unpacked | What it adds |
|---|---|---|---|
| `playwright-core` | 1.64.1 | 13.66 MB **M**, no dependencies, no install script | The library and CLI. Since 1.62 it also contains the MCP server, the `playwright-cli` daemon and client, a dashboard and agent skills: "Playwright now bundles the Playwright MCP server and `playwright-cli`, runnable via `npx playwright mcp` and `npx playwright cli`" ([release notes](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md)) |
| `playwright` | 1.64.1 | 5.03 MB **M** | Test-runner plumbing; no postinstall, so installing downloads no browser |
| `@playwright/test` | 1.64.1 | 0.03 MB **M** | Re-export of `playwright` |
| `@playwright/mcp` | 0.0.83 (2026-09-28) | 0.09 MB **M** | Thin wrapper pinning `playwright`/`playwright-core` 1.64 alpha; 6.40M downloads a week |
| `@playwright/cli` | 0.1.22 | 0.11 MB **M** | `playwright-cli` with skills (`install --skills`); 1.06M a week. Microsoft steers coding agents here because CLI plus skills is "more token-efficient" ([playwright-mcp README](https://github.com/microsoft/playwright-mcp/blob/main/README.md)) |

**AI.** Playwright makes no model call itself. AI enters through MCP and CLI tools driven by an external model, and through **Playwright Test Agents** (since 1.56): planner, generator and healer definitions generated by `npx playwright init-agents --loop=claude|vscode|opencode`; the healer "executes the test suite and automatically repairs failing tests" ([test-agents](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/test-agents-js.md)).

**Everything else in the box:** `storageState` and `launchPersistentContext` for sessions; Trace Viewer, UI Mode, codegen, video with action overlays; `page.clock`; `route()`, HAR record and replay; per-context `proxy` (no rotation); WebMCP (`page.webmcp`, 1.64); `playwright-cli show`, a dashboard of every bound browser with live screencast and takeover (1.59). **No stealth:** "Avoiding detection is out of the scope for Playwright" ([#13838](https://github.com/microsoft/playwright/issues/13838)). The only cloud is the optional Azure Playwright service (`@azure/playwright`).

### 3.2 Install size per platform

The npm packages are small; the browsers are not. `npx playwright install` downloads every browser pinned in [`browsers.json` @v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/browsers.json): Chrome for Testing 156.0.8078.4 (Playwright stopped building its own Chromium in 1.57), its headless shell, patched Firefox 157.0 and patched WebKit 27.2, plus ffmpeg. Compressed archives, MiB **M**:

| Archive | linux x64 | linux arm64 | macOS arm64 | win64 |
|---|---:|---:|---:|---:|
| Chrome for Testing | 191.1 | 190.5 | 185.8 | 197.5 |
| chromium-headless-shell | 120.0 | 119.9 | 99.0 | 118.1 |
| Firefox | 111.0 | 102.7 | 105.7 | 123.8 |
| WebKit | 103.1 | 97.1 | 78.4 | 60.2 |
| **Default `playwright install`** | **≈527** | ≈510 | **≈470** | **≈501** |

`--only-shell` cuts that to the ~120 MiB headless shell. The Docker image `mcr.microsoft.com/playwright:v1.64.1-noble` is 940.6 MiB compressed on amd64, and the MCP image (headless Chromium only) about 394 MiB **M**.

**The ports bundle a Node driver.** Python, Java and .NET are clients of the same Node.js driver over the Playwright protocol on a stdio pipe, so each ships Node plus the `playwright-core` JavaScript:

| Port | Version | Artifact size **M** |
|---|---|---|
| Python `playwright` | 1.64.0 (2026-10-13) | wheels of 46.0 MiB (manylinux x86_64), 45.7 (aarch64), 41.0 (macOS arm64), 42.3 (macOS x86_64), 36.9 (win_amd64), 33.0 (win_arm64), each with `driver/node` ([setup.py](https://github.com/microsoft/playwright-python/blob/main/setup.py)) |
| Java `driver-bundle` | 1.63.0 (2026-09-14) | **203.8 MB** jar with the drivers of every platform; a split has been requested since 2023 ([playwright-java#1196](https://github.com/microsoft/playwright-java/issues/1196), +21). Java downloads browsers on first run unless told not to ([intro-java.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/intro-java.md)) |
| .NET `Microsoft.Playwright` | 1.63.0 (2026-09-23) | **213.5 MB** `.nupkg`, packing `node/<platform>/node` ([Playwright.csproj](https://github.com/microsoft/playwright-dotnet/blob/main/src/Playwright/Playwright.csproj)) |

There is no Go port ([#6856](https://github.com/microsoft/playwright/issues/6856), +229, closed). The patched stealth fork Patchright mirrors the same layout (PyPI wheels 32.9–46.0 MiB **M**, 966,036 downloads a week).

### 3.3 Attach and remote

- **`BrowserType.connect(wsEndpoint)`** to a `launchServer()` or `npx playwright run-server`: full fidelity, but client and server major.minor must match ([class-browsertype.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md)).
- **`connectOverCDP(url)`**, Chromium only, "significantly lower fidelity than the Playwright protocol connection". Recent options: `isLocal` (1.58), `noDefaults` (1.60) for "attaching to a user's daily-driver browser" without overriding its downloads, focus or media settings, and `artifactsDir` (1.61).
- **`Browser.bind(title)`** (1.59) publishes a launched browser on a named pipe or WebSocket so that `playwright-cli attach <name>` or `@playwright/mcp --endpoint` can reach it ([class-browser.md](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browser.md)).
- **MCP modes:** a persistent profile per workspace by default (one browser per profile, so concurrent clients conflict), `--isolated`, `--cdp-endpoint`, and `--extension`, which reaches the user's running Chrome or Edge through the "Playwright Extension" from the Chrome Web Store with per-connection approval ([extension README](https://github.com/microsoft/playwright/blob/v1.64.1/packages/extension/README.md)). With no browser named, MCP uses the installed Chrome channel, so it needs no download when Chrome is present ([config.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/tools/mcp/config.ts)).

### 3.4 Inside crawlers

Playwright has no queue, storage or crawler. Crawlee's `PlaywrightCrawler` (`@crawlee/playwright` 3.18.2) peer-depends on `playwright` and on `idcac-playwright` for cookie-consent dismissal; Scrapy uses [scrapy-playwright](https://github.com/scrapy-plugins/scrapy-playwright) 0.0.48 as a download handler with `PLAYWRIGHT_CDP_URL`, `PLAYWRIGHT_ABORT_REQUEST` and `PLAYWRIGHT_MAX_CONTEXTS` settings. Crawl4AI and Firecrawl also drive Playwright (prior note §4).

### 3.5 Complaints

- **Platforms and size:** Ubuntu 26.04 support [#40117](https://github.com/microsoft/playwright/issues/40117) (+103), Arch [#8100](https://github.com/microsoft/playwright/issues/8100) (+186), CentOS [#9199](https://github.com/microsoft/playwright/issues/9199) (+98): the patched Firefox and WebKit builds are distribution-pinned. Single-browser Docker images [#31729](https://github.com/microsoft/playwright/issues/31729) (+34; "reach 2.5Gb"); a shared resource cache between runs [#22865](https://github.com/microsoft/playwright/issues/22865) (+102).
- **Chrome for Testing switch:** no open-source Chromium any more and 20 GB+ memory [#38489](https://github.com/microsoft/playwright/issues/38489); a maintainer: "Playwright no longer provides a Chromium build".
- **The Node driver in other languages:** the Java bundle size [playwright-java#1196](https://github.com/microsoft/playwright-java/issues/1196); a frozen driver never surfaces a timeout [playwright-java#1990](https://github.com/microsoft/playwright-java/issues/1990); "Connection closed while reading from the driver" [playwright-python#2530](https://github.com/microsoft/playwright-python/issues/2530); PyInstaller packaging [#2357](https://github.com/microsoft/playwright-python/issues/2357).
- **`connectOverCDP` fidelity:** no video [#29065](https://github.com/microsoft/playwright/issues/29065), context management unsupported [#15370](https://github.com/microsoft/playwright/issues/15370), context options not customisable [#37627](https://github.com/microsoft/playwright/issues/37627).
- **MCP:** snapshot and tool-output size [playwright-mcp#395](https://github.com/microsoft/playwright-mcp/issues/395) (+14), [#889](https://github.com/microsoft/playwright-mcp/issues/889) (+13); "Browser is already in use" [#891](https://github.com/microsoft/playwright-mcp/issues/891) (+11); "controlled by automated test software" [#148](https://github.com/microsoft/playwright-mcp/issues/148) (+16); the bridge extension blocked by enterprise policy [#1035](https://github.com/microsoft/playwright-mcp/issues/1035) (+10).
- **Test Agents:** Python support [#38610](https://github.com/microsoft/playwright/issues/38610) (52), unusable in Cursor [#37739](https://github.com/microsoft/playwright/issues/37739) (41).

---

## 4. Puppeteer and Chrome DevTools MCP

**Puppeteer** 25.13.0 (2026-10-08), Apache-2.0, JavaScript only ([release](https://github.com/puppeteer/puppeteer/releases/tag/puppeteer-v25.13.0)); 23.2M `puppeteer-core` and 11.5M `puppeteer` downloads a week.

- **`puppeteer-core`** (6.01 MB **M**) is the library only: it "will not download Chrome when installed" ([installation.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/installation.md)). It speaks CDP to Chrome and WebDriver BiDi to stock Firefox (`stable_157.0.1`), unlike Playwright's patched Firefox ([faq.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/faq.md)).
- **`puppeteer`** (0.04 MB on top of core **M**) is "a product": a postinstall downloads Chrome for Testing 155.0.8059.39 and `chrome-headless-shell` to `~/.cache/puppeteer`, plus config-file support and the `npx puppeteer browsers` CLI. Firefox is not downloaded by default. The docs now warn that npm, pnpm, Yarn Berry, Bun and Deno block install scripts by default, so launch then fails with "Could not find Chrome". Download per platform, compressed **M**: ≈307 MiB on linux64, ≈281 on macOS arm64, ≈317 on win64 (Chrome plus headless shell). `ghcr.io/puppeteer/puppeteer:25.13.0` is 762.2 MiB compressed **M**.
- **`@puppeteer/browsers`** 3.2.4 is a standalone installer for Chrome, the headless shell, ChromeDriver, Chromium and Firefox; **`@puppeteer/replay`** 4.0.2 replays Chrome DevTools Recorder JSON files without a model.
- **No AI primitives, MCP, stealth, proxy rotation, persistence or cloud.** The reference `@modelcontextprotocol/server-puppeteer` is deprecated. `puppeteer-extra` and its stealth plugin were last published in 2023, the repository last pushed 2024-07-18, and an open issue asks for maintainers ([#932](https://github.com/berstend/puppeteer-extra/issues/932)).
- **Attach:** `puppeteer.connect({ browserWSEndpoint | browserURL | transport, protocol })`; `channel` finds the `DevToolsActivePort` of the user's own Chrome profile ([ConnectOptions.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/ConnectOptions.ts)). Any CDP or BiDi endpoint works, which is why hosted-browser vendors document it. Crawlee's `PuppeteerCrawler` peer-depends on `puppeteer`.
- **Complaints:** binaries for arm64 [#7740](https://github.com/puppeteer/puppeteer/issues/7740) (+154) and Apple Silicon [#6622](https://github.com/puppeteer/puppeteer/issues/6622) (+139); download failures [#2173](https://github.com/puppeteer/puppeteer/issues/2173); Lambda's 250 MB limit [#603](https://github.com/puppeteer/puppeteer/issues/603); zombie processes [#1825](https://github.com/puppeteer/puppeteer/issues/1825) (+98); response interception [#1191](https://github.com/puppeteer/puppeteer/issues/1191) (+161, open since 2017); BiDi gaps on Firefox ([#13925](https://github.com/puppeteer/puppeteer/issues/13925)).

**Chrome DevTools MCP** (`chrome-devtools-mcp` 1.10.1, 2026-09-23, Apache-2.0, 1.31M downloads a week), the Chrome DevTools team's agent server built on Puppeteer, now branded "Chrome DevTools for agents" ([README](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/README.md)):

- **14.25 MB with zero runtime dependencies** **M**: Puppeteer, the MCP SDK and DevTools frontend code are bundled.
- An MCP server and an experimental `chrome-devtools` CLI that talks to a **background daemon** over a Unix socket or named pipe ([cli.md](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/cli.md)). Tools cover input, navigation, emulation, performance traces with CrUX data, network, debugging, heap snapshots, extensions, WebMCP and PWA; it ships agent skills.
- **Attach:** `--browser-url`, `--ws-endpoint`, or **`--autoConnect`** (Chrome 144+, after the user enables remote debugging at `chrome://inspect/#remote-debugging`) to the user's default profile; otherwise it launches the installed Chrome with a persistent profile ([configuration.md](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/configuration.md)).
- **Egress:** first-class `--allowedUrlPattern` (Chrome 149+) and `--blockedUrlPattern` with URLPattern, blocking navigations and subresources and detaching from disallowed targets; patterns with regexp groups are refused "because it is not enforced on redirects or subresources".
- **Usage statistics go to Google by default** (`--no-usage-statistics`; off when `CI` is set).
- Complaints: macOS focus stealing on every command [#1254](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1254) (+26), parallel sessions [#926](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/926) (+24), hangs with many tabs [#1921](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1921).

---

## 5. Browser Use

_In progress._

---

## 6. Medium-depth products

### 6.1 Vercel agent-browser

**What it is.** A browser CLI for AI agents, Apache-2.0, Rust, **0.39.1** (2026-10-13), 43,955 stars, **1,726,779 npm downloads a week**, slightly above Stagehand. One binary is both the CLI and an auto-started daemon "using direct CDP, no Node.js required", which idles out after an hour ([README, Architecture](https://github.com/vercel-labs/agent-browser/blob/main/README.md#architecture)).

- **Box contents:** accessibility snapshot with refs (`@e1`) and semantic locators, batch, network routes and HAR, cookies and storage, tabs, frames, dialogs, snapshot and pixel diffs, Chrome trace and profiler, video (needs ffmpeg), session state save and load (optionally AES-GCM encrypted), an auth vault the model never sees, a domain allowlist, action policies and confirmations, an out-of-process plugin protocol, skills, an MCP server (`core`, `network`, `react`, `all` profiles), a **local dashboard on port 4848** and an AI `chat` command through the Vercel AI Gateway ([README](https://github.com/vercel-labs/agent-browser/blob/main/README.md)). No crawler, queue or dataset; network response bodies for scraping are an open request ([#111](https://github.com/vercel-labs/agent-browser/issues/111)).
- **History that matters for size:** up to 0.16 the daemon was Node.js plus Playwright; 0.20.0 (2026-03-14) "Removed the entire Node.js/Playwright daemon", claiming "Install size reduced from 710 MB to 7 MB" ([CHANGELOG](https://github.com/vercel-labs/agent-browser/blob/main/CHANGELOG.md)).
- **Install size:** v0.39.1 release binaries are 15.8 MB (darwin-arm64), 17.4 (darwin-x64), 15.8 (linux-arm64), 18.3 (linux-x64), 18.1 (linux-musl-x64) and 17.9 (win32-x64) **M** ([release](https://github.com/vercel-labs/agent-browser/releases/tag/v0.39.1)). The npm package ships every platform's binary in one tarball, so `unpackedSize` is **119.4 MB** **M**. Chrome is not downloaded on install; `agent-browser install` fetches Chrome for Testing, and existing Chrome, Brave, Playwright and Puppeteer browsers are detected. Also on Homebrew; crates.io lags (0.19.0).
- **Languages:** none; agents call the CLI or MCP.
- **Attach:** `--cdp <port|ws-url>` or `connect 9222` to any CDP endpoint (Chrome, Electron, WebView2), `--auto-connect` through Chrome's `DevToolsActivePort`, cloud providers (Browserbase, Browserless, Browser Use, Kernel, AgentCore), engines `chrome`, `lightpanda` and `obscura`, and iOS Safari through Appium. Idle shutdown never closes a user-attached or headed browser.
- **Complaints:** stealth mode [#120](https://github.com/vercel-labs/agent-browser/issues/120) (15, open); Windows install and daemon failures, the most-commented class ([#56](https://github.com/vercel-labs/agent-browser/issues/56), [#390](https://github.com/vercel-labs/agent-browser/issues/390), [#148](https://github.com/vercel-labs/agent-browser/issues/148)); profile reuse [#42](https://github.com/vercel-labs/agent-browser/issues/42), [#207](https://github.com/vercel-labs/agent-browser/issues/207); cross-origin iframes [#279](https://github.com/vercel-labs/agent-browser/issues/279) (12, open); orphaned Chrome helpers at high CPU [#1371](https://github.com/vercel-labs/agent-browser/issues/1371); a click reported done that missed under smooth scroll [#2083](https://github.com/vercel-labs/agent-browser/issues/2083).

### 6.2 Lightpanda

**What it is.** A headless browser "written in Zig", not a Chromium fork, built on V8, html5ever and libcurl, **AGPL-3.0**, 1.0.0 (2026-10-02), 36,471 stars ([README](https://github.com/lightpanda-io/browser/blob/main/README.md), [release 1.0.0](https://github.com/lightpanda-io/browser/releases/tag/1.0.0)). It is a browser, not an SDK, but it now ships SDK-like parts: CDP and WebDriver BiDi servers, `fetch` with HTML or Markdown output, proxies, network interception, a built-in ad blocker, `--obey-robots`, a native MCP server (stdio or HTTP), an in-process `agent`, and PandaScript recordings replayed by `lightpanda run`, described as "deterministic and token-free".

- **Size:** raw binaries 188 MB (linux x86_64), 192 MB (linux aarch64), 94 MB (macOS x86_64), 90 MB (macOS aarch64); `.deb` 35 MB and 33 MB **M**. No native Windows build (WSL only). A PyPI `lightpanda` 1.0.0.post1 bundles the binary in 26–50 MiB wheels **M**.
- **Attach:** it is the browser; Puppeteer and Playwright connect over CDP, and agent-browser and Stagehand list it as an engine. Telemetry is on by default (`LIGHTPANDA_DISABLE_TELEMETRY=true`).
- **Complaints:** site operators reporting abuse by Lightpanda clients behind residential proxies [#3156](https://github.com/lightpanda-io/browser/issues/3156) (22, the most-reacted, open); robots.txt compliance by default [#3234](https://github.com/lightpanda-io/browser/issues/3234); CDP coverage gaps for Playwright and Puppeteer ([#2177](https://github.com/lightpanda-io/browser/issues/2177), [#2043](https://github.com/lightpanda-io/browser/issues/2043)); bot detection ([#990](https://github.com/lightpanda-io/browser/issues/990)).
- The vendor's benchmark (100 pages in 5 s against 46 s, 123 MB against 2 GB peak memory compared with headless Chrome) was not reproduced.

### 6.3 chromiumoxide and other Rust drivers

None of these is a product with AI primitives, record and replay, MCP or a UI; they are libraries (crates.io API, 2026-10-14):

| Crate | Latest (date) | Downloads total / 90 days | Notes |
|---|---|---|---|
| [chromiumoxide](https://crates.io/crates/chromiumoxide) | 0.9.1 (2026-02-25) | 4.98M / 2.11M | Last merge 2026-04-03; protocol types at Chrome 145; no per-session send into cross-origin iframes ([Rust CDP note](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md)) |
| [chromey](https://crates.io/crates/chromey) | 2.58.2 (2026-09-12) | 334k / 43k | Spider's fork; one maintainer; 203 crates including mandatory stealth crates |
| [headless_chrome](https://crates.io/crates/headless_chrome) | 1.0.22 (2026-06-11) | 3.45M / 779k | Blocking, non-flat sessions |
| [thirtyfour](https://crates.io/crates/thirtyfour) | 0.37.8 (2026-10-12) | 2.14M / 596k | WebDriver, BiDi feature since 0.37 |
| [fantoccini](https://crates.io/crates/fantoccini) | 0.22.1 (2026-02-28) | 4.02M / 520k | WebDriver |
| [playwright-rs](https://crates.io/crates/playwright-rs) | 0.20.0 (2026-10-11) | 188k / 151k | "A thin JSON-RPC client" to the Playwright driver, which its build script fetches with its own Node.js, "about 130 MB" ([README](https://github.com/padamson/playwright-rust/blob/main/README.md)) |

Rust also sits under other products: agent-browser's daemon, Browser Use's beta runtime (`browser-use-core`, §5), Skyvern's optional [rustwright](https://github.com/Skyvern-AI/rustwright) ("Playwright's API on a Rust CDP engine… no driver subprocess", PyPI 0.3.0, 3.2–3.9 MiB wheels **M**) and Spider.

### 6.4 Skyvern, Midscene, Steel, Notte and Magnitude

_In progress._

---

## 7. Other products with real adoption (brief)

| Product | What ships | Open boundary | Adoption (week to 2026-10-12) |
|---|---|---|---|
| [Kernel](https://github.com/kernel/kernel-images) | Hosted sandboxed Chrome; open `kernel-images` (Docker or unikernel, live view, video replays, standby snapshots), CLI, SDKs | Images Apache-2.0, service commercial | `@onkernel/sdk` 660,219 npm; `kernel` 159,306 PyPI |
| [Browserless](https://github.com/browserless/browserless) | Docker image of headless browsers with REST and CDP endpoints | SSPL-1.0 or commercial ([LICENSE](https://github.com/browserless/browserless/blob/main/LICENSE)) | 13,811 stars |
| [dev-browser](https://github.com/SawyerHood/dev-browser) | CLI and skill that lets coding agents run short scripts against a persistent Chrome; its README warns `node:vm` "is not a security sandbox" | MIT | 11,994 npm |
| [Hyperbrowser](https://github.com/hyperbrowserai/HyperAgent) | Cloud browsers plus HyperAgent (Playwright or Patchright with AI actions and an XPath `actionCache`) | AGPL-3.0 per the prior note | `@hyperbrowser/sdk` 83,959 npm; `hyperbrowser` 122,931 PyPI |
| [AgentQL](https://github.com/tinyfish-io/agentql) | Query language over Playwright, resolved by a hosted engine | SDK MIT, inference hosted | 3,934 npm; 11,407 PyPI |
| [Laminar](https://github.com/lmnr-ai/lmnr) | Agent observability with browser session recordings | Apache-2.0 | `lmnr` 1.20M PyPI |
| Stealth drivers | [Patchright](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright) (patched Playwright), [Camoufox](https://github.com/daijro/camoufox) (Firefox anti-detect), [nodriver](https://github.com/ultrafunkamsterdam/nodriver) (CDP, Python) | Apache-2.0, MPL-2.0, AGPL-3.0 | 966,036, 273,423 and 75,911 PyPI |
| Selenium and WebdriverIO | WebDriver clients with Selenium Manager and BiDi | Apache-2.0, MIT | `selenium-webdriver` 4.51.0 is 23.3 MB unpacked **M** |

Skipped: Bytebot (archived), LaVague (unmaintained since January 2025), BrowserOS (a browser fork, covered in the 2026-10-10 note), and the OpenAI, Anthropic and Google computer-use tools (model APIs with reference loops, not browser SDKs).

---

## 8. Capability equivalence with Goodfetch

_In progress: the matrix is built once every product section is in._

---

## 9. Implications for Goodfetch

_In progress._

---

## 10. Unverified / open

- Exact transitive install size of `@browserbasehq/stagehand` 4.2.1: about 10–11 MB is a sum of direct dependencies, not a measured `node_modules`.
- Whether stock Chrome Stable accepts Stagehand's `Extensions.loadUnpacked` on a borrowed browser; Stagehand's docs imply yes; not tested.
- The "Basic stealth" Stagehand's docs list for local mode: no stealth code was found in the 4.2.1 bundles.
- Whether the hosted Browserbase MCP runs Stagehand v4: npm `@browserbasehq/mcp` 3.0.0 depends on v3.
- That the .NET `Microsoft.Playwright` package holds Node for every platform: inferred from its csproj and size, not opened.
- Puppeteer's documented Chrome download sizes ("~170MB macOS, ~282MB Linux") do not match the measured archives.
- agent-browser's "7 MB" install claim is the changelog's; the measured binaries are 15.8–18.3 MB.
- Lightpanda's vendor benchmark was not reproduced.

---

## 11. Sources

Listed inline. Registries: [npm registry](https://registry.npmjs.org/), [npm downloads API](https://api.npmjs.org/downloads/), [PyPI JSON](https://pypi.org/pypi/), [pypistats](https://pypistats.org/api/), [crates.io API](https://crates.io/api/v1/crates/), [Go module proxy](https://proxy.golang.org/), [Maven Central](https://repo1.maven.org/maven2/com/microsoft/playwright/driver-bundle/), [NuGet](https://api.nuget.org/v3-flatcontainer/microsoft.playwright/), all read 2026-10-14.
