# Browser-automation products and where they draw their boundary

Research of **October 14, 2026** for ticket [#448](https://github.com/goodfetch/automan/issues/448). Question: what do browser-automation SDKs, Collie's direct competitors, ship as a product, and where do they draw their boundary? For each product: what the box contains (driver, AI primitives, caching or replay, agent loop, MCP server, CLI, stealth, proxies, session persistence, storage, recording, UI), install size per platform, languages, how it attaches to a browser or page it did not launch, how crawlers use it, what users complain about, and which Goodfetch capabilities have an equivalent. It feeds the tickets that decide Collie's capability set and the product boundary between Collie and Foxhound. Like everything in `rnd/`, this note informs a decision; it does not make one.

**Sources.** Registry metadata read on 2026-10-14: npm (`npm view`, `npm pack`, downloads API for the week 2026-10-06 to 2026-10-12), PyPI JSON and pypistats, crates.io, the Go module proxy, Maven Central and NuGet; GitHub release assets, tagged source trees and issue trackers through `gh api`; official documentation, read from the repositories' own docs sources where they exist. Browser archive sizes are HTTP `Content-Length` on the vendors' CDN URLs. Earlier notes reused and re-checked: [AI browser frameworks](https://github.com/goodfetch/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md) (2026-10-10), [Rust CDP clients](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md), the [parity list](https://github.com/goodfetch/automan/blob/research/parity-list/rnd/research/2026-10-10-parity-list.md), [browser frameworks](2026-10-07-browser-frameworks.md), [Stagehand alternatives](2026-10-07-stagehand-alternatives-followup.md), [Stagehand CDP latency](2026-10-07-stagehand-cdp-latency.md) and [agent capability matrix](2026-10-07-agent-capability-matrix.md).

**Measured versus cited.** Facts marked **M** were measured for this note: registry sizes, tarball and wheel listings, release-asset sizes, image manifests, CDN `Content-Length`, and install sizes from `npm install --ignore-scripts` and `pip download` into scratch directories on macOS arm64. Everything else is cited from a primary source, linked inline. No package code was run and no browser was started.

**Versions inspected.** Stagehand 4.2.1 (TypeScript, Python and Go, 2026-10-10/11); Playwright 1.64.1 (2026-10-14), Playwright Python 1.64.0, Java and .NET 1.63.0, `@playwright/mcp` 0.0.83, `@playwright/cli` 0.1.22; Puppeteer 25.13.0 (2026-10-08), `chrome-devtools-mcp` 1.10.1; Browser Use 0.13.11 (2026-10-07); Skyvern 1.0.56 (2026-10-14); Midscene 1.15.4 (2026-10-14); agent-browser 0.39.1 (2026-10-13); Steel `steel-browser` v0.5.4-beta, `steel-sdk` 0.18.0/0.19.0; Notte 1.9.7 (2026-10-14); Magnitude `magnitude-core` 0.3.1 (2026-02-08); Lightpanda 1.0.0 (2026-10-02); chromiumoxide 0.9.1, chromey 2.58.2.

---

## 1. Summary

- **Every product draws its boundary at the browser session.** None ships a crawler, request queue, dataset or key-value store. Repeated work goes to the user's own loop, to the vendor's cloud (rerunnable scripts, Functions, schedules) or to a crawler that takes the driver's page, as Crawlee does with Playwright, Puppeteer and Stagehand v3 (§8.1, §8.2).
- **Stagehand v4 is three thin SDKs over one runtime inside a Chrome extension.** 4.0.0 shipped 2026-08-10; 4.2.1 is current (2026-10-11).
  - The MV3 extension holds the CDP driver, `act`/`observe`/`extract` and the model calls. Each SDK embeds it, so the npm install is 33.0 MB, the Python wheel 0.48 MB (v3 wheels were 31–37 MB with a bundled Node server) and the Go module 0.68 MB **M**.
  - It downloads no browser. Attaching to a borrowed browser means loading the extension over CDP, which needs a shared filesystem and `--enable-unsafe-extension-debugging`.
  - There is no Playwright page interop, and `agent()` is gone.
  - Caching, the Model Gateway, stealth, proxies, CAPTCHA solving, ad blocking and recordings are Browserbase-only.
  - Crawlee's integration still targets v3 (§2).
- **Playwright 1.64.1 bundles the MCP server, the CLI with skills and a dashboard into `playwright-core` since 1.62.**
  - It installs 18.3 MB with no third-party dependency. Browsers are a separate ≈470–527 MiB download.
  - The Python, Java and .NET ports bundle a Node driver: 33–46 MiB wheels, a 204 MB jar, a 214 MB NuGet package **M**.
  - It is the only guard-grade driver: strictness, actionability and a hit-target interceptor that cancels mis-targeted events.
  - It offers no stealth by policy, and nothing of it is cloud-gated (§3).
- **Puppeteer 25.13.0 splits library from "product".** `puppeteer` downloads ≈281–317 MiB of Chrome at postinstall, which package managers now block by default. The Chrome team's DevTools MCP (14.25 MB, zero dependencies) adds a daemon, `--autoConnect` to the person's Chrome and URL-pattern egress rules, and sends usage statistics to Google by default (§4).
- **Browser Use 0.13.11 is four products split by who runs the agent.** They are a library, a code-mode CLI, a Rust Terminal and a Cloud.
  - The library unpacks to **262 MB across 107 wheels**, with 37 mostly exact-pinned dependencies **M**; dependency conflicts are its most-reacted recent complaint.
  - Some of it is closer to Goodfetch than anything else found:
    - default local ad and cookie-banner extensions;
    - `rerun_history`, which re-finds elements through a five-level match;
    - the Terminal's SQLite event log with resumable tasks.
  - It also clicks through JavaScript when it detects a covered target. Its telemetry, on by default, may include task instructions and URLs (§5).
- **agent-browser 0.39.1 is one 15.8–18.3 MB Rust binary.** That binary is CLI, daemon, MCP server and local dashboard, replacing a 710 MB Node-and-Playwright install. It now out-downloads Stagehand on npm (1.73M against 1.64M a week), has the strictest domain allowlist found, ships no stealth, and is the runtime under Steel's CLI (§6.1).
- **Skyvern is the only AI SDK that ships a local server, web UI and database** (SQLite at `~/.skyvern/data.db`, or Postgres), under AGPL-3.0. Its Custom Code block "sandbox" was an `exec` escaped through `object.__subclasses__()` (§6.4). Midscene adds vision AI to someone else's Playwright or Puppeteer page and keeps its plan and XPath cache in local YAML files (§6.5).
- **Against Goodfetch's capabilities (§9):**
  - **Absent everywhere:** an Effect journal, a replay that verifies its target before acting, and a real Script sandbox.
  - **Playwright only:** a guard-grade driver.
  - **Only with a server:** durable run state (Skyvern, Browser Use Terminal).
  - **Cloud-only almost everywhere:** stealth and Clean-up rules.
  - **Table stakes for agent-facing products:** a CLI with skills, an MCP server, a daemon and a local dashboard.
- **Attaching to the person's own Chrome has converged on Chrome 144's remote-debugging toggle** at `chrome://inspect/#remote-debugging`, used by DevTools MCP, agent-browser, Skyvern and browser-harness. Extension bridges are the alternative (Playwright Extension, Midscene, Stagehand), and they draw flag and enterprise-policy complaints (§10).

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

**Transport.** Each SDK attaches a CDP session to the extension's service-worker target and sends JSON-RPC messages through it with `Runtime.evaluate` on that target; the 4.2.1 `dist/index.mjs` also calls `Runtime.addBinding`, presumably for the return path **M**. This is why every local launch adds `--enable-unsafe-extension-debugging`: it is "required so the SDK can attach a CDP session to the service worker hosting Stagehand's runtime" ([browser.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/browser.mdx)). The RPC is bidirectional, which carries the "bring your own LLM" callback: anything other than the five bundled providers (Azure, Bedrock, self-hosted, any base URL) is called back on the client ([models.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/configuration/models.mdx)). Each SDK also keeps its own deterministic CDP client for page and locator calls.

**What v4 removed.** "`agent()` is gone. Nothing in v4 replaces it one-for-one." The replacements are code mode (a coding assistant writes a Stagehand script once) and tool calling over the full API ([v3 migration](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/v3.mdx)). Also gone: the local cache (`enableCaching`, `cacheDir`) and every Playwright, Puppeteer and Patchright peer dependency.

### 2.2 Install footprint

| Artifact | Size | Notes |
|---|---|---|
| npm `@browserbasehq/stagehand` 4.2.1 | **3.51 MB** unpacked, 16 files **M** | `dist/extension/service-worker.js` alone is 2.08 MB (the runtime with AI SDK and OTel); the zip copy is 0.45 MB; `index.mjs` 219 KB **M** |
| npm 4.2.1 installed with dependencies | **33.0 MB**, 42 packages on macOS arm64 **M** | OpenTelemetry is the largest part (15.9 MB, of which `@opentelemetry/semantic-conventions` is 11.8 MB), then zod 6.3 MB and the Browserbase SDK; the SDK package itself is 3.4 MB **M** |
| npm 3.7.3 (v3, for contrast) | 10.19 MB unpacked, 1,174 files **M** | Plus `ai`, `openai`, `@anthropic-ai/sdk`, `@google/genai`, MCP SDK, pino, 16 optional `@ai-sdk/*` providers and peers on `playwright-core`, `puppeteer-core`, `patchright-core` ([npm 3.7.3](https://registry.npmjs.org/@browserbasehq/stagehand/3.7.3)) |
| PyPI `stagehand` 4.2.1 | **0.48 MB**, one universal wheel **M** | No per-platform wheels, no bundled Node |
| PyPI `stagehand` 3.23.0 (v3) | 30.9–37.4 MB per-platform wheels | The macOS arm64 wheel embedded a 102.9 MB Node single-executable server, because v3 Python was a client of the Stagehand API ([PyPI 3.23.0](https://pypi.org/pypi/stagehand/3.23.0/json)) |
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
- **No Playwright-grade driver guarantees:** no auto-waiting, no `getBy*`, no route interception, no `waitForResponse`, `page.on()` for `"console"` only, one context per browser (same migration guide). `page.pdf()` arrived in 4.2.0.
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

Installed with its dependencies, `@playwright/test` (or `@playwright/mcp`, or `@playwright/cli`) is **18.3 MB in 3 packages** on macOS arm64 **M**: Playwright has no third-party runtime dependency.

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

- **`puppeteer-core`** (6.01 MB; installed with dependencies 31.8 MB in 25 packages, of which `chromium-bidi` is 9.7 MB and `zod` 8.1 MB **M**) is the library only: it "will not download Chrome when installed" ([installation.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/installation.md)). It speaks CDP to Chrome and WebDriver BiDi to stock Firefox (`stable_157.0.1`), unlike Playwright's patched Firefox ([faq.md](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/faq.md)).
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

**What it is.** "Make websites accessible for AI agents": an MIT Python agent library, **0.13.11** (2026-10-07), 117,951 stars, 1,666,124 PyPI downloads a week ([release](https://github.com/browser-use/browser-use/releases/tag/0.13.11), [pyproject](https://github.com/browser-use/browser-use/blob/0.13.11/pyproject.toml)). Browser Use is now a family of products sold around one cloud, and its own docs ask the reader to "Choose who runs the agent. Then choose a local or cloud browser" ([which product](https://docs.browser-use.com/cloud/which-product.md)). There is no official TypeScript library; npm `browser-use` is an "Unofficial TypeScript port" by another author.

### 5.1 What the products contain

| Product | License, version | What it is |
|---|---|---|
| **`browser-use` library** | MIT, 0.13.11 | The agent loop (`Agent.run()` returning an `AgentHistoryList`), a CDP driver through `cdp-use`, an event bus and watchdogs (crash, downloads, popups, HAR, video, storage state, security, CAPTCHA), DOM-plus-accessibility perception with optional vision, built-in tools plus custom `Tools().action`, an LLM judge, cost tracking, `sensitive_data` placeholders with TOTP, `allowed_domains`/`prohibited_domains`, HAR, video and trace recording, and `rerun_history` (§5.4). Its own thin model clients (OpenAI, Anthropic, Google, Groq, Ollama, Azure, Bedrock, DeepSeek, OpenRouter, Vercel AI Gateway and others) and the BU2 model through the Browser Use gateway. |
| **`browser-use` CLI** (same package) | MIT | "Direct browser control for coding agents… backed by Browser Harness": the agent pipes **Python** into `browser-use`, which runs helpers such as `new_tab()` and `page_info()` against the browser ([CLI docs](https://docs.browser-use.com/open-source/browser-use-cli.md)). `browser-use skill install` registers a skill; `browser-use --mcp` runs a local stdio MCP server ([MCP docs](https://docs.browser-use.com/open-source/customize/integrations/mcp-server.md)). Cloud browsers are named daemons (`start_remote_daemon("work")`) that "bill until they stop or time out". |
| **[browser-harness](https://github.com/browser-use/browser-harness)** | MIT, 0.1.13, 18,431 stars | "One editable CDP websocket" to the user's real Chrome; "the agent writes missing helpers as it works" into a local workspace, while the package source "stays protected". Ships `browser-harness-mcp`. `browser-use` pins it as a hard dependency. |
| **[Browser Use Terminal](https://github.com/browser-use/terminal)** | MIT, Rust, v0.1.8 (2026-06-12) | A Ratatui TUI (`browser`), a CLI (`browser-use-terminal run-openai …`), a skill for coding assistants, and the runtime behind `browser_use.beta.Agent`. "Rust owns the agent loop and durable state", with a **"SQLite event log: history · screenshots · artifacts · traces"** and tasks you can "watch, steer, stop, retry, and resume" ([README](https://github.com/browser-use/terminal/blob/main/README.md)). It also holds a secret store with 1Password import and TOTP seeds, a domain allow and deny list, MCP client config and layered config profiles ([Terminal docs](https://docs.browser-use.com/open-source/browser-use-terminal.md)). Its crates include `browser-use-store` (with `rusqlite`), `browser-use-secrets` and `browser-use-python-worker`. Release archives are 23–26 MiB per platform (macOS and Linux musl only) **M**; the repository was last pushed 2026-08-16. |
| **Browser Use Cloud** | commercial | The hosted agent (API V4 with sessions, workspaces and profiles), a hosted browser API reached over CDP, rerunnable scripts, a hosted MCP server, 1Password and secret bindings, live view and recordings. Cloud browsers are "a hardened Chromium fork with stealth enabled by default", with "Ad and cookie banner blocking", CAPTCHA solving and residential proxies in 195+ countries ([stealth](https://docs.browser-use.com/cloud/browser/stealth.md)). |
| `workflow-use` | AGPL-3.0, 0.2.11 (2025-11) | Record-and-replay of "semantic workflows" with Browser Use as fallback; its README warns against production use (prior note). |

### 5.2 Install footprint

- **The library is light; its dependencies are not.** The wheel is 0.71 MB, but 0.13.11 declares 37 required dependencies, almost all exact pins (`==`), including five provider SDKs, the Google API client, the MCP SDK, PostHog, `pypdf`, `reportlab`, `python-docx` and `browser-harness`. Resolved for CPython 3.12 on macOS arm64 it is **107 wheels, 59.7 MB to download and 262 MB unpacked** **M**. The largest single item is `googleapiclient` at 92.8 MB unpacked, then `lxml` 19.3 MB and the PyObjC test suite 15.6 MB **M**.
- **The Rust runtime is an extra.** `browser-use[core]` adds `browser-use-core` 0.13.3 (2026-08-16), per-platform wheels of 21.3–25.6 MiB **M**.
- **The browser is a separate step.** `uvx browser-use install` runs `uvx playwright install chromium` ([cli.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/cli.py)), so it fetches Playwright's Chrome for Testing and headless shell (≈311 MiB on linux x64, §3.2). `Browser.from_system_chrome()` uses the installed Chrome instead.
- **Extensions download at first launch.** `enable_default_extensions` is on by default and fetches uBlock Origin Lite, "I still don't care about cookies" and Force Background Tab from Google's extension update server into a cache, unless `BROWSER_USE_DISABLE_EXTENSIONS=1` ([profile.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py)). The field's description says "URL cleaning (ClearURLs)", which the code's list no longer contains.

### 5.3 Attach, local and cloud

- **Real browser:** `Browser.from_system_chrome(profile_directory=…)` launches the installed Chrome on the user's own profile; "You may need to fully close Chrome before running these examples" ([real browser](https://docs.browser-use.com/open-source/customize/browser/real-browser.md)). Chrome 136 stopped allowing CDP on the default profile, which drew [#1520](https://github.com/browser-use/browser-use/issues/1520) (+10, 36 comments).
- **Borrowed browser:** `Browser(cdp_url=…)` attaches to "any cloud browser provider" or remote Chrome ([remote](https://docs.browser-use.com/open-source/customize/browser/remote.md)); the CLI reads `BU_CDP_URL` or `BU_CDP_WS` and by default attaches to the running Chrome after the user approves Chrome's remote-debugging prompt.
- **Cloud:** `Browser(use_cloud=True, cloud_profile_id=…, cloud_proxy_country_code=…)`. Profile Sync copies selected local cookies into a cloud profile, on request only ([profile sync](https://docs.browser-use.com/cloud/guides/profile-sync.md)).
- **Lock-in shape:** the open library runs fully locally with any model, but stealth, CAPTCHA solving, proxies, recordings, live view, rerunnable scripts and profiles that outlive a machine are Cloud features; `captcha_solver` is a watchdog that "Only active when the browser emits BrowserUse CDP events (e.g. Browser Use cloud browsers)" ([profile.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py)).
- **Telemetry is on by default** and "may include… task instructions, URLs visited, action traces, errors, and final results"; `ANONYMIZED_TELEMETRY=false` turns it off ([telemetry](https://docs.browser-use.com/open-source/development/monitoring/telemetry.md)). Observability beyond logs goes to Laminar, cloud or self-hosted ([observability](https://docs.browser-use.com/open-source/development/monitoring/observability.md)).

### 5.4 What Browser Use does that is close to Goodfetch's design

- **Replay with a cascading target match.** `Agent.rerun_history(history, …)` replays a saved `AgentHistoryList` without the model for its actions (an LLM is used only for extract steps and the summary). It re-finds each element through five levels: full `element_hash` (attributes plus accessible name), a "stable" hash without dynamic CSS classes, XPath, accessible name, then a unique attribute ([agent/service.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py), `_update_action_indices`). It is the closest open-source analogue of a Plan step's Target fingerprint found, but nothing stops a lower level (XPath or accessible name) from matching a look-alike, and there is no versioned plan format beyond the history JSON.
- **An occlusion check that does not stop the action.** Before a CDP click the default action watchdog runs `document.elementFromPoint` to see whether the target is covered, and if it is, "falling back to JavaScript click" ([default_action_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/default_action_watchdog.py)). It detects the condition the Goodfetch guard refuses on, then acts anyway through a path that bypasses the hit test.
- **A domain policy at navigation level only.** The security watchdog checks `allowed_domains` before navigation, after redirects and on new tabs, and sends a blocked page to `about:blank` ([security_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/security_watchdog.py)). Subresources are not covered, and open reports show `javascript:` and `chrome://` bypasses ([#5099](https://github.com/browser-use/browser-use/issues/5099)) and a default profile that "allows all hosts and leaves IP URLs unblocked" ([#5879](https://github.com/browser-use/browser-use/issues/5879)).
- **Durable state, in the Rust Terminal only.** The library keeps an in-memory history that can be saved to JSON and a conversation log (`save_conversation_path`); the Terminal runtime is the one with a SQLite event log and resumable tasks.

### 5.5 Inside crawlers

No crawler, queue or dataset. Repeated extraction is sold as Cloud "rerunnable scripts": the agent saves and tests a `script.py`, and "Every later run still starts a model and uses tokens" ([scripts](https://docs.browser-use.com/cloud/agent/scripts.md)). No first-party Crawlee or Scrapy integration was found.

### 5.6 Complaints

- **Exact-pin dependency conflicts** in projects that embed the library: "Too strict dependency version pinning (`==`)" [#4824](https://github.com/browser-use/browser-use/issues/4824) (+5, open), [#4877](https://github.com/browser-use/browser-use/issues/4877), [#4868](https://github.com/browser-use/browser-use/issues/4868) (anthropic), [#4285](https://github.com/browser-use/browser-use/issues/4285) (openai). #4824 is the most-reacted issue opened since May 2026.
- **Supply chain:** the CLI install script pulled a malicious `litellm` 1.82.8 [#4505](https://github.com/browser-use/browser-use/issues/4505) (+9).
- **Telemetry:** PostHog upload errors were among the first top issues ([#282](https://github.com/browser-use/browser-use/issues/282) +19, [#265](https://github.com/browser-use/browser-use/issues/265) +10).
- **Browser lifecycle:** "Chromium stuck, need manual intervention" [#839](https://github.com/browser-use/browser-use/issues/839) (+14, 41 comments), blank page at step 1 [#1020](https://github.com/browser-use/browser-use/issues/1020) (+14), no page-crash recovery [#5067](https://github.com/browser-use/browser-use/issues/5067).
- **False success:** actions that report "element not found" as success [#5438](https://github.com/browser-use/browser-use/issues/5438); a known-answer regression set [#6031](https://github.com/browser-use/browser-use/issues/6031). The docs themselves now say `is_successful` "is the agent-reported outcome; verify important external actions independently" ([llms.txt](https://docs.browser-use.com/llms.txt)).
- **Replay and cost:** "ability to rerun same thing faster" [#763](https://github.com/browser-use/browser-use/issues/763) (+11), "Export a generated script" [#943](https://github.com/browser-use/browser-use/issues/943) (+11); a governance layer with URL allowlists and action budgets [#5293](https://github.com/browser-use/browser-use/issues/5293).
- **Human-like behaviour and human-in-the-loop:** [#947](https://github.com/browser-use/browser-use/issues/947) (+20), [#221](https://github.com/browser-use/browser-use/issues/221) (+18).

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

- **Size:** raw binaries 188 MB (linux x86_64), 192 MB (linux aarch64), 94 MB (macOS x86_64), 90 MB (macOS aarch64); `.deb` 35 MB and 33 MB **M**. No native Windows build (WSL only). The official Python package `lightpanda` 1.0.0.post1 ([lightpanda-python](https://github.com/lightpanda-io/lightpanda-python)) bundles the binary in 26–50 MiB wheels **M**.
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

### 6.4 Skyvern

**What it is.** "A Playwright extension that adds AI-powered browser automation" plus a no-code workflow builder, **AGPL-3.0**, Python, **1.0.56** (2026-10-14), 23,195 stars ([README](https://github.com/Skyvern-AI/skyvern/blob/main/README.md)). Of the products here it is the one that ships as a **local server with a database and a UI**.

- **Box contents:** a Playwright-compatible SDK where `page.click(prompt=…)`, `page.fill(prompt=…)` and `page.select_option(prompt=…)` sit beside plain selectors, plus `page.act`, `page.extract`, `page.validate` (returns a bool), `page.prompt` and `page.agent.run_task/login/download_files/run_workflow`; tasks and DAG workflows with loops, validation, file parsing, email, HTTP and **Custom Code** blocks; a FastAPI server on :8000 and a UI on :8080 with live streaming of the viewport; credentials through Bitwarden, 1Password, Azure Key Vault or an HTTP credential service, with TOTP, email and SMS 2FA; an MCP server; Zapier, Make and n8n connectors; and a TypeScript client (`@skyvern/client`).
- **Run state in a database:** "`skyvern quickstart` and `skyvern run server` default to a SQLite database at `~/.skyvern/data.db`"; Docker Compose always uses a bundled Postgres ([README](https://github.com/Skyvern-AI/skyvern/blob/main/README.md)). The schema is migrated with Alembic.
- **Code caching:** after a successful run Skyvern "can generate code from the actions it took"; later runs use `run_with="code"`, and with "AI Fallback" on by default a failing cached script falls back to the agent and regenerates the code ([code caching](https://www.skyvern.com/docs/developers/features/code-caching)). The check on replay is whether the code runs, not whether it hit the right element; a user asked "what ensures it's the correct one in that context?" ([#5637](https://github.com/Skyvern-AI/skyvern/issues/5637)).
- **Install size:** the base wheel is 7.41 MB and `pip install skyvern` (the SDK and cloud client) resolves to **26 wheels, 12.5 MB** on macOS arm64 **M**. The `local` extra, which pulls Playwright, LiteLLM, SQLAlchemy, FastAPI, pandas, pyarrow, Google Cloud AI Platform and the 1Password SDK, could not be resolved to wheels only (an sdist-only dependency), so it was not measured. Its own `pyproject.toml` calls the embedded local mode "a Forge-backed compatibility bridge" that "is not a minimal browser-only runtime" ([pyproject](https://github.com/Skyvern-AI/skyvern/blob/main/pyproject.toml)). The browser comes from Playwright.
- **Attach:** `Skyvern(browser_address="http://127.0.0.1:9222")` or `BROWSER_TYPE=cdp-connect` attaches to the user's Chrome after enabling `chrome://inspect/#remote-debugging`; `skyvern browser serve --tunnel` exposes a local Chrome to Skyvern Cloud through a tunnel, with a warning that without `--api-key` "anyone with the URL has full control of your browser". The optional `skyvern[rustwright]` extra swaps Playwright's Node driver for a Rust CDP engine.
- **Cloud-only:** "anti-bot detection mechanisms, proxy network, and CAPTCHA solvers" and parallel instances.
- **Script sandbox in name only:** Custom Code blocks ran through `exec()` with emptied builtins, and a critical escape through `object.__subclasses__()` was reported and closed ([#6868](https://github.com/Skyvern-AI/skyvern/issues/6868), 2026-06-27).
- **Complaints:** self-hosting friction dominates (Docker with Claude models [#312](https://github.com/Skyvern-AI/skyvern/issues/312), 24 comments; a circular import breaking `skyvern run server` [#7762](https://github.com/Skyvern-AI/skyvern/issues/7762); "Local quickstart and repo install not working" [#5756](https://github.com/Skyvern-AI/skyvern/issues/5756), open); the blocked-host gate "only covers agent-initiated routes" [#8577](https://github.com/Skyvern-AI/skyvern/issues/8577) (open); "Is anyone currently monitoring the security advisory queue?" [#8482](https://github.com/Skyvern-AI/skyvern/issues/8482) (open). Reaction counts are very low (the top issue has 8).

### 6.5 Midscene.js

**What it is.** ByteDance web-infra's vision-driven automation SDK, MIT, TypeScript, **1.15.4** (2026-10-14), 15,256 stars, 66,120 `@midscene/web` downloads a week. Code calls `aiAct`, `aiTap`, `aiInput`, `aiQuery`, `aiAssert` and `aiWaitFor`; perception is screenshots sent to a vision model.

- **Borrowed pages are its main mode.** It does not own a driver: `new PuppeteerAgent(page)` or the Playwright fixture wraps a page the user's own Puppeteer or Playwright code launched ([Puppeteer integration](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/integrate-with-puppeteer.mdx)). **Bridge mode** drives the user's desktop Chrome through the Midscene extension from the Chrome Web Store, on a new tab or "the currently active tab", reusing "all cookies, plugins, page status" ([bridge mode](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/bridge-mode.mdx)). Android, iOS, HarmonyOS and desktop agents use the same API.
- **Local, file-based cache.** Plan steps of `aiAct` and element XPaths (web only) are cached under `./midscene_run/cache/*.cache.yaml`, keyed by prompt, with `read-write`, `read-only` and `write-only` strategies; an XPath is checked for validity on reuse, a failing cached plan falls back to live planning, and query results "will never be cached" ([caching](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/caching.mdx)). Unlike Stagehand's server-side cache it is local and can be committed with the code, but it keys on the prompt and checks only that the XPath still resolves.
- **Reports, YAML, skills; no MCP.** An HTML replay report per run, a YAML script runner and CLI, and Skills. "Midscene no longer ships MCP servers"; 1.9.8 was the last version with them ([mcp.mdx](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/mcp.mdx)).
- **Install size:** `@midscene/web` is 2.05 MB unpacked, but an install resolves **276 packages, 166.4 MB** on macOS arm64, including the playground, `sharp`'s libvips and `playwright-core` **M**. No browser is downloaded by `@midscene/web`; Puppeteer or Playwright bring their own.
- **No stealth, proxies, egress control, sandbox, database or UI beyond the report and playground** were found.
- **Complaints:** invalid JSON from the model [#2831](https://github.com/web-infra-dev/midscene/issues/2831) (open), schema errors [#339](https://github.com/web-infra-dev/midscene/issues/339); "Export AI execution steps as reusable Playwright scripts" [#2240](https://github.com/web-infra-dev/midscene/issues/2240) (+4, open); elements outside the viewport [#179](https://github.com/web-infra-dev/midscene/issues/179).

### 6.6 Steel

**What it is.** "An open-source browser API" for agents: `steel-browser`, Apache-2.0, TypeScript, **v0.5.4-beta** (2026-08-25), 7,801 stars, plus Steel Cloud ([README](https://github.com/steel-dev/steel-browser/blob/main/README.md)). Steel runs no agent and makes no model call; it is the browser host that agents and drivers connect to.

- **Box contents:** a Fastify API that manages sessions and Chrome processes through Puppeteer, a CDP WebSocket for Puppeteer, Playwright or Selenium clients, session state across requests, a proxy chain, Chrome extensions, "stealth plugins and fingerprint management" (`fingerprint-generator` and `fingerprint-injector`), page-to-Markdown, readability, screenshot and PDF tools, and a **session viewer UI** at `/ui`. CDP instrumentation events go to a DuckDB `browser_events` table, in memory unless a path is set ([duckdb-storage.ts](https://github.com/steel-dev/steel-browser/blob/main/api/src/services/cdp/instrumentation/storage/duckdb-storage.ts)); LevelDB reads Chrome's own localStorage and sessionStorage for session persistence ([api/package.json](https://github.com/steel-dev/steel-browser/blob/main/api/package.json)).
- **Install size:** the combined API and UI image `ghcr.io/steel-dev/steel-browser:latest` is **1,338 MiB compressed** on linux/amd64 **M**. The Node path needs an installed Chrome. `steel-sdk` is 0.82 MB on npm and 0.14 MB on PyPI.
- **CLI built on agent-browser.** The `steel` CLI drives sessions with `open`, `snapshot`, `fill` and `click` in cloud, self-hosted or local mode, and "`steel browser` is directly integrated with the `agent-browser` runtime"; it installs coding-agent skills ([Steel CLI](https://docs.steel.dev/overview/steel-cli)).
- **Cloud-only:** the Credentials API, CAPTCHA solving, residential proxies, profiles, files, dedicated IPs and multi-region; free credit is $30 ([pricing](https://docs.steel.dev/overview/pricinglimits), [docs index](https://docs.steel.dev/llms.txt)).
- **Complaints:** self-hosted fingerprint generation fails ([#295](https://github.com/steel-dev/steel-browser/issues/295), [#302](https://github.com/steel-dev/steel-browser/issues/302) for Chrome 146), Docker launch failures ([#322](https://github.com/steel-dev/steel-browser/issues/322)), no cluster session affinity ([#144](https://github.com/steel-dev/steel-browser/issues/144)), the self-hosted API lagging the SDK's session timeout ([#350](https://github.com/steel-dev/steel-browser/issues/350)), missing docs ([#123](https://github.com/steel-dev/steel-browser/issues/123)).

### 6.7 Notte

**What it is.** "The full-stack web AI agent framework", Python with a Node SDK, **SSPL-1.0** (GitHub reports NOASSERTION), **1.9.7** (2026-10-14), 2,035 stars ([pyproject](https://github.com/nottelabs/notte/blob/main/pyproject.toml), [LICENSE](https://github.com/nottelabs/notte/blob/main/LICENSE)). The README now opens with "Notte gives AI agents and developers cloud browsers" ([README](https://github.com/nottelabs/notte/blob/main/README.md)).

- **Packages:** `notte` is a 0.01 MB meta-package over `notte-core` (0.56 MB wheel), `notte-browser` (Patchright locally, or any CDP URL), `notte-agent`, `notte-llm` (LiteLLM) and `notte-sdk` **M**. The cloud adds a hosted MCP at `api.notte.cc/mcp`, a CLI via Homebrew, Browser Functions (scripts deployed as APIs or cron jobs), Vaults, Browser Profiles and Personas; every cloud session has "proxies, CAPTCHA solving, anti-detection, live view and recordings".
- **Replay:** an agent run can be turned into a function, which its own docs say "Breaks if page changes", with an agent fallback (prior note).
- **Adoption is small:** 466 PyPI downloads a week; the license is the main barrier to embedding it.

### 6.8 Magnitude

**What it is.** A vision-first browser agent and test runner, Apache-2.0, TypeScript, `magnitude-core` **0.3.1** and `magnitude-test` 0.3.13, both 2026-02-08, the date of the repository's last push ([repo](https://github.com/magnitudedev/browser-agent)). The organisation now ships an unrelated inference engine ([magnitudedev/magnitude](https://github.com/magnitudedev/magnitude)), so the browser agent is **dormant**. It still has 11,270 npm downloads a week.

- **Box contents:** `agent.act()` and `agent.extract(zod)` over Playwright aliased to Patchright, coordinates from a visually grounded model through BAML, a test runner with AI `check()` assertions, and an MCP server. Its deterministic cache was listed as "in progress" and never shipped (prior note).
- **Install size:** `magnitude-core` is 0.84 MB, but an install resolves **84 packages, 102.0 MB** on macOS arm64: the BAML native library alone is 39.1 MB, `sharp`'s libvips 14.9 MB and `patchright-core` 13.2 MB **M**.

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
| Selenium and WebdriverIO | WebDriver clients, no AI layer | Apache-2.0, MIT | `selenium-webdriver` 4.51.0 is 23.3 MB unpacked, `webdriverio` 10.0.2 is 2.0 MB **M** |

Skipped: Bytebot (archived), LaVague (unmaintained since January 2025), BrowserOS (a browser fork, covered in the 2026-10-10 note), and the OpenAI, Anthropic and Google computer-use tools (model APIs with reference loops, not browser SDKs).

---

## 8. Boundaries compared

### 8.1 At a glance

Sizes are measured (**M**) on macOS arm64 for installs, per platform for binaries and wheels; sections 2 to 6 cite them.

| Product | Ships as | Languages | Install | Browser at install | Borrowed browser | Only in the vendor's cloud |
|---|---|---|---|---|---|---|
| Stagehand 4.2.1 | Three SDKs embedding one extension runtime; `browse` CLI | TS, Python, Go | npm 33.0 MB installed; wheel 0.48 MB; Go 0.68 MB | none; uses the installed Chrome | `localBrowser.connect(cdpUrl)`, then loads its extension (shared filesystem, debugging flag) | cache, Model Gateway, stealth, proxies, CAPTCHA, ad blocking, recordings, contexts, hosted MCP |
| Playwright 1.64.1 | Library, test runner, MCP server and CLI with skills, the last two inside `playwright-core` since 1.62 | JS/TS; Python, Java, .NET over a bundled Node driver | npm 18.3 MB; Python wheels 33–46 MiB; Java jar 204 MB; .NET 214 MB | separate `playwright install`, ≈470–527 MiB | `connectOverCDP` (lower fidelity), `Browser.bind`, MCP `--extension` | nothing (optional Azure test service) |
| Puppeteer 25.13.0 | `puppeteer-core` library; `puppeteer` adds a browser download; Chrome DevTools MCP apart | JS/TS | core 31.8 MB installed; DevTools MCP 14.7 MB | `puppeteer` postinstall, ≈281–317 MiB | `connect()` by endpoint or `channel`; DevTools MCP `--autoConnect` | nothing |
| Browser Use 0.13.11 | Agent library, code-mode CLI, harness, Rust Terminal, Cloud | Python (Cloud SDK also TS) | 262 MB unpacked, 107 wheels | `browser-use install` fetches Playwright's Chromium | `cdp_url`, `from_system_chrome()`, CLI attach to the running Chrome | stealth Chromium fork, CAPTCHA, proxies, recordings, rerunnable scripts, cloud profiles |
| Skyvern 1.0.56 | SDK, local server, UI, database, workflows, MCP | Python (TS client) | SDK 12.5 MB in 26 wheels; server extra not measured | through Playwright | `browser_address` or `cdp-connect`; tunnel to its cloud | anti-bot, proxies, CAPTCHA |
| Midscene 1.15.4 | AI layer over someone else's page; extension bridge; YAML runner; skills | TS | 166.4 MB installed | none (the driver's) | wraps a Puppeteer or Playwright page; bridge to desktop Chrome | no cloud |
| agent-browser 0.39.1 | One Rust binary: CLI, daemon, MCP, dashboard | none (CLI, MCP) | 15.8–18.3 MB binary; npm 119 MB for all platforms | `agent-browser install` (Chrome for Testing) | `--cdp`, `--auto-connect`, cloud providers | no cloud of its own |
| Steel v0.5.4-beta | Browser API server with UI; SDKs; CLI on agent-browser | server TS; SDKs TS, Python | Docker image 1,338 MiB | inside the image | it is the host others connect to | credentials, CAPTCHA, proxies, profiles |
| Notte 1.9.7 | Agent framework, cloud browsers, functions | Python, Node SDK | wheels under 1 MB each, plus dependencies | Patchright's | `cdp_url` | most of the product |
| Magnitude 0.3.1 (dormant) | Agent, test runner, MCP | TS | 102.0 MB installed | Patchright's | Playwright page access | no cloud |
| Lightpanda 1.0.0 | A browser with CDP, BiDi, MCP and an agent | Zig binary | 90–192 MB binary | it is the browser | n/a | Lightpanda Cloud |
| chromiumoxide 0.9.1 | Rust CDP library | Rust | crate | optional fetcher | `Browser::connect(ws)` | n/a |

**What no browser-automation product here ships:** a crawler, a request queue, a dataset or a key-value store. The boundary every one of them draws is the session: one browser, its pages and its state. Repeated work is left to the user's loop (Stagehand's cookbooks), to the vendor's cloud (Browser Use rerunnable scripts, Browserbase and Notte Functions, Skyvern schedules) or to a crawler that takes the page (Crawlee).

### 8.2 Inside crawlers

- **Crawlee hands the driver's page to user code.** `PlaywrightCrawler` and `PuppeteerCrawler` peer-depend on the driver; `@crawlee/stagehand` creates a Stagehand v3 instance and reattaches Playwright through `connectOverCDP` "to get a compatible handle" (§2.4). Crawlee supplies the queue, sessions, proxy rotation, autoscaling and fingerprints; the driver supplies the page.
- **Stagehand v4 broke that integration** by removing the Playwright handle and the v3 entry points, and no v4 port exists yet ([apify/crawlee#4196](https://github.com/apify/crawlee/issues/4196)).
- **Scrapy uses a download handler** ([scrapy-playwright](https://github.com/scrapy-plugins/scrapy-playwright)); Crawl4AI and Firecrawl drive Playwright themselves.
- **Spider uses its own Rust driver** (chromey), with stealth crates compiled in ([Rust CDP note](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md)).
- **Runtimes now reuse each other:** Steel's CLI runs on agent-browser (§6.6), agent-browser and Stagehand accept Lightpanda as an engine, and Midscene adds AI to a Playwright or Puppeteer page it never owns.

### 8.3 Complaint themes

| Theme | Where it shows | Examples |
|---|---|---|
| Size and native downloads | Playwright browsers and ports, Puppeteer's postinstall, Browser Use's dependency tree | [playwright#31729](https://github.com/microsoft/playwright/issues/31729), [playwright-java#1196](https://github.com/microsoft/playwright-java/issues/1196), [puppeteer#7740](https://github.com/puppeteer/puppeteer/issues/7740), [puppeteer#603](https://github.com/puppeteer/puppeteer/issues/603), [browser-use#4824](https://github.com/browser-use/browser-use/issues/4824) |
| Platform support | Playwright's pinned Linux distributions, Windows for agent-browser and Stagehand, no Windows Lightpanda | [playwright#40117](https://github.com/microsoft/playwright/issues/40117), [agent-browser#56](https://github.com/vercel-labs/agent-browser/issues/56), [stagehand#3166](https://github.com/browserbase/stagehand/issues/3166), [lightpanda#369](https://github.com/lightpanda-io/browser/issues/369) |
| Attaching to a real or existing browser | All of them | [stagehand#507](https://github.com/browserbase/stagehand/issues/507), [playwright#15370](https://github.com/microsoft/playwright/issues/15370), [browser-use#1520](https://github.com/browser-use/browser-use/issues/1520), [agent-browser#42](https://github.com/vercel-labs/agent-browser/issues/42), [playwright-mcp#891](https://github.com/microsoft/playwright-mcp/issues/891) |
| Replay without a model | Stagehand, Browser Use, Midscene, Magnitude | [stagehand#389](https://github.com/browserbase/stagehand/issues/389), [browser-use#943](https://github.com/browser-use/browser-use/issues/943), [midscene#2240](https://github.com/web-infra-dev/midscene/issues/2240) |
| False success | Browser Use, agent-browser, Stagehand | [browser-use#5438](https://github.com/browser-use/browser-use/issues/5438), [agent-browser#2083](https://github.com/vercel-labs/agent-browser/issues/2083), [stagehand#2486](https://github.com/browserbase/stagehand/issues/2486) |
| Egress controls that leak | Stagehand, Browser Use, Skyvern; Playwright MCP disclaims it | [stagehand#2945](https://github.com/browserbase/stagehand/issues/2945), [browser-use#5099](https://github.com/browser-use/browser-use/issues/5099), [skyvern#8577](https://github.com/Skyvern-AI/skyvern/issues/8577) |
| Stealth missing locally | agent-browser, Playwright, Puppeteer (stale plugins), Steel self-hosted | [agent-browser#120](https://github.com/vercel-labs/agent-browser/issues/120), [playwright#13838](https://github.com/microsoft/playwright/issues/13838), [puppeteer-extra#932](https://github.com/berstend/puppeteer-extra/issues/932), [steel#302](https://github.com/steel-dev/steel-browser/issues/302) |
| Open core and cloud lock-in | Stagehand most directly | [stagehand#492](https://github.com/browserbase/stagehand/issues/492) |
| Token and output size | Playwright MCP | [playwright-mcp#395](https://github.com/microsoft/playwright-mcp/issues/395), [#889](https://github.com/microsoft/playwright-mcp/issues/889) |
| Browser lifecycle | Browser Use, Puppeteer, agent-browser | [browser-use#839](https://github.com/browser-use/browser-use/issues/839), [puppeteer#1825](https://github.com/puppeteer/puppeteer/issues/1825), [agent-browser#1371](https://github.com/vercel-labs/agent-browser/issues/1371) |

---

## 9. Capability equivalence with Goodfetch

The Goodfetch capabilities, as the [glossary](../../GLOSSARY.md) and ADRs define them: **run state** in SQLite that a run resumes from (ADR 0002); the **Effect journal**, so a retried or resumed step never repeats an Effect (ADR 0010); **Plans**, steps with a Target fingerprint replayed without a model, with Drift and bounded Repair (ADR 0009); the **guard**, which requires a unique, visible, enabled, stable, uncovered target and cancels events that land elsewhere (ADR 0019); the **Goodfetch Chromium** and stealth catalog (ADR 0035); one **egress policy** (ADR 0017); **Clean-up rules** for ads, trackers, consent prompts and walls (ADRs 0047, 0048); the **Script sandbox** (ADR 0051); the **Console** (ADR 0031); the **Daemon** (ADR 0036).

**Y** built in and local; **part** partial (the cell says how); **cloud** only in the vendor's paid cloud; **—** none found in the sources of the product's section.

| Product | Run state | Effect journal | Plans | Guard | Stealth build | Egress policy | Clean-up rules | Script sandbox | Console | Daemon |
|---|---|---|---|---|---|---|---|---|---|---|
| **Stagehand 4.2** | — (keep-alive browser only) | — (docs: retry `observe()`, never `act()`) | cloud: server-side action cache; locally the user keeps `observe()` selectors | — (one resolve, centre click) | cloud | part: `setDomainPolicy`, fails open, misses workers and WebSockets | cloud: `blockAds` | — (`experimentalBatch` runs in the extension, which holds `debugger`) | cloud | part: `browse` per-session daemon |
| **Playwright 1.64** | — | — | part: codegen and recordings emit code; Test Agents' healer edits tests | **Y**: strictness, visible, stable, receives events, enabled, plus the hit-target interceptor | — ("out of the scope") | part: `route()` by hand; MCP origins "not a security boundary" | — | — (`browser_run_code_unsafe` is "RCE-equivalent") | **Y**: UI Mode, Trace Viewer, `playwright-cli show` | **Y**: `run-server`, `Browser.bind`, CLI daemon |
| **Puppeteer 25 + DevTools MCP** | — | — | part: `@puppeteer/replay` replays Recorder JSON | part: visible, enabled, stable, in viewport; no hit-target or uniqueness check | — | part: DevTools MCP URL patterns block navigations and subresources | — | — | — | part: DevTools MCP CLI daemon |
| **Browser Use 0.13** | part: SQLite event log and resume in the Rust Terminal; JSON history in the library | — | part: `rerun_history` with a five-level element match; Cloud scripts still start a model | part: detects occlusion, then clicks through JavaScript | cloud: hardened Chromium fork | part: navigation-level allow and deny lists, known bypasses | **Y**: uBlock Origin Lite and a cookie-banner extension on by default; cloud banner blocking | — (CLI runs agent-written Python directly) | part: Terminal TUI; cloud live view | part: named browser daemons |
| **Skyvern 1.0.56** | **Y**: SQLite at `~/.skyvern/data.db` or Postgres | — | part: generated code per successful run, agent fallback regenerates it | part: Playwright's checks apply to selector actions; the path of prompt-located actions was not traced | cloud | part: blocked-host gate on agent routes only | — | part: `exec` with emptied builtins; escape fixed (#6868) | **Y**: web UI with live stream | **Y**: API server for UI and SDK |
| **Midscene 1.15** | — | — | part: local YAML plan and XPath cache keyed by prompt | — (the driver's; vision locates) | — | — | — | — | part: HTML report, playground | — |
| **agent-browser 0.39** | — (session state files) | — (action policy and confirmations) | — | — (refs and waits) | — (requested, #120) | **Y**: subresources, WebSocket, EventSource, beacons, WebRTC, workers fail closed | — | — (`eval` in the page) | **Y**: dashboard on :4848 | **Y**: per-session Rust daemon |
| **Steel 0.5** | part: DuckDB event log, in memory by default | — | — | — (the client's driver) | part: fingerprint injection; more in cloud | — | — (custom extensions only) | — | **Y**: session viewer | **Y**: it is a browser server |
| **Notte 1.9** | cloud | — | part: agent run turned into a function, "Breaks if page changes" | — | part: Patchright locally; cloud anti-detection | — | — | unverified (`restrictedpython` dependency) | cloud | cloud |
| **Magnitude 0.3** | — | — | — (cache never shipped) | — (vision coordinates) | part: Patchright | — | — | — | — | — |
| **Lightpanda 1.0** | — | — | part: PandaScript replay, no target check | — | — (its own engine) | — | part: built-in ad blocker | — | — | part: CDP and MCP server |
| **chromiumoxide 0.9** | — | — | — | — | — | — | — | — | — | — |

**Reading the matrix.**

- **No product has an Effect journal.** The nearest things are advice: Stagehand's docs say a failed `act()` "may already have clicked, submitted, or paid", and Browser Use's say to "reconcile external actions before starting duplicate work" after a client timeout ([llms.txt](https://docs.browser-use.com/llms.txt)).
- **No product verifies a replayed step's target before it acts.** Browser Use's cascade, Midscene's XPath check and Stagehand's cached selector all ask "does it still resolve?", not "is it the same element?".
- **Playwright is the only guard-grade driver**, and the only one that cancels mis-targeted events. Browser Use detects a covered target and clicks anyway; Stagehand and agent-browser check nothing beyond a box model or a ref.
- **Durable run state appears only where there is a server:** Skyvern's SQLite or Postgres and Browser Use Terminal's SQLite event log. Both are Python or Rust applications with a UI, not SDKs.
- **Stealth and Clean-up rules are cloud features almost everywhere.** The local exceptions are Browser Use's default extensions, Lightpanda's ad blocker, Steel's fingerprint injection and the Patchright and Camoufox forks.
- **A real Script sandbox exists nowhere.** Every code mode found runs model-written code with the host's or the browser's full rights, and two products say so in writing (Playwright MCP, dev-browser).
- **CLI with skills, MCP, a daemon and a local dashboard are now table stakes** for products aimed at coding agents: Playwright, agent-browser, Chrome DevTools MCP, Browser Use and Stagehand's `browse` all ship at least three of the four.

---

## 10. Implications for Goodfetch

Considerations for the Collie capability-set and product-boundary tickets; none of them is a decision.

- **The market's boundary is the session, and no competitor bundles a crawler.** A standalone Collie that stops at Sessions, pages, Plans and the AI operations would match what users expect from Stagehand, Playwright or Browser Use; Frontier, Datasets and Sinks would be Foxhound's, as Crawlee is to Playwright today.
- **Thin clients over one runtime is where the leaders converged.**
  - Stagehand v4 moved to three SDKs over one embedded runtime and cut its Python install from 31–37 MB wheels to 0.48 MB.
  - Browser Use's beta wraps a Rust runtime over stdio JSON-RPC.
  - Playwright has always wrapped a Node driver, and pays for it in 33–46 MiB wheels and ~200 MB Java and .NET packages that carry every platform.
  - This supports ADR 0024's shape. It also says that how the runtime is packaged per platform, not the client, is what users measure.
- **Size expectations are set by two ends:**
  - agent-browser's 16–18 MB Rust binary, which replaced a 710 MB Node and Playwright install, is the benchmark for a native automation tool.
  - Browsers dominate every real install (Playwright ≈470–527 MiB, Puppeteer ≈281–317 MiB).
  - ADR 0057 projects 95–120 MB raw and 35–45 MB compressed per platform, before the engine's own code. That is five to seven times agent-browser's raw binary, while the compressed figure sits in the band of Playwright Python's wheels (33–46 MiB).
  - V8 (Script sandbox, Code mode) is Collie-relevant. DuckDB serves search, the SQL tab and the DuckDB Sink, so it is less clearly a Collie need, which the boundary ticket may want to weigh.
- **What users expect from a standalone automation product**, by how many products ship it:
  - **Nearly universal:** a CLI with agent skills, an MCP server (Midscene retired its own), attaching to the person's running Chrome, session state save and restore, and traces or recordings.
  - **Common:** a daemon that keeps the browser between calls, and a local dashboard.
  - **Rare:** durable run state, a domain policy enforced at the network layer, and local Clean-up rules.
  - **Absent:** an Effect journal, a verified replay, a real sandbox, and a local stealth build.
  - The rare and absent items are where Collie would differ. They are also the parts a "Collie without the engine" would lose if the split cut them out.
- **Competitors keep in the cloud what Goodfetch puts locally.**
  - Stagehand, Browser Use, Skyvern, Steel and Notte all gate stealth, CAPTCHA solving, proxies, recordings and caches behind their clouds. The free Browserbase plan has "No captcha solving or stealth mode".
  - ADRs 0035 and 0047 make the stealth catalog and Clean-up rules local defaults.
  - That is a product difference worth naming in Collie's positioning. It also means Collie's install carries data (Clean-up rules) and a browser download (the Goodfetch Chromium) that no competitor SDK carries.
- **The borrowed-browser path has standardised since Chrome 144.**
  - Chrome DevTools MCP `--autoConnect`, agent-browser `--auto-connect`, Skyvern and browser-harness all attach after the person enables remote debugging at `chrome://inspect/#remote-debugging`.
  - Extension routes (the Playwright Extension, Midscene's bridge, Stagehand's runtime) are the alternative, and draw enterprise-policy and flag complaints.
  - ADR 0056's Borrowed browser could be checked against this flow.
- **Crawlers integrate a browser product by taking its page.**
  - Crawlee's three integrations do exactly that, and Stagehand v4 broke its own by removing the page handle.
  - If Foxhound carries Collie as its browser layer, the Collie page API inside a Foxhound handler is the contract that matters.
  - ADR 0019's raw CDP WebSocket endpoint is the escape hatch Crawlee-style users would reach for.
- **Dependency hygiene is a complaint in its own right.** Browser Use's 37 exact pins and 262 MB tree draw conflict reports from projects that embed it. Stagehand v4's four-dependency SDK, and Playwright with no third-party runtime dependency at all, show the alternative.
- **Default telemetry is common, and sometimes includes task data:**
  - Browser Use's PostHog telemetry "may include… task instructions, URLs visited, action traces".
  - Chrome DevTools MCP sends usage statistics to Google.
  - Lightpanda's telemetry is on.
  - ADR 0043's "nothing leaves the machine unless configured" is a differentiator users can check.

---

## 11. Unverified / open

- **Installs measured on one platform only.** Every `node_modules` and unpacked-wheel size is for macOS arm64; Linux and Windows installs were not measured. Resolving Browser Use for Linux from macOS failed on platform markers.
- **Skyvern's server install** (`skyvern[local]`) was not measured: one dependency ships only as an sdist. Where Skyvern stores generated code (local database or cloud) is not documented on the code-caching page.
- **Browser Use's default extensions:** the field's description names ClearURLs, while the 0.13.11 code fetches Force Background Tab instead. Whether the Terminal's install script serves builds newer than its GitHub releases (last v0.1.8, 2026-06-12) is unknown.
- **Stagehand:**
  - Whether stock Chrome Stable accepts `Extensions.loadUnpacked` on a borrowed browser; the docs imply yes; not tested.
  - The "Basic stealth" its docs list for local mode: no stealth code was found in the 4.2.1 bundles.
  - Whether the hosted Browserbase MCP runs v4: npm `@browserbasehq/mcp` 3.0.0 depends on v3.
  - How Director relates to v4.
- **Midscene's click path** (coordinate input after vision localization) is inferred from its "pure vision" design, not traced in code.
- **Notte's `restrictedpython` dependency** suggests a confined code path; its use was not traced.
- **Steel's egress controls** were looked for in its README and package manifest only.
- **Playwright .NET:** that `Microsoft.Playwright` holds Node for every platform is inferred from its csproj and size; the package was not opened.
- **Puppeteer's documented Chrome download sizes** ("~170MB macOS, ~282MB Linux") do not match the measured archives; their basis is unknown.
- **agent-browser's "7 MB" install claim** is the changelog's; the measured binaries are 15.8–18.3 MB.
- **Lightpanda's vendor benchmark** was not reproduced.
- **Issue reactions are a weak signal** for Stagehand, Skyvern, Steel and Notte, whose communities sit on Discord; counts there are low.

---

## 12. Sources

Each claim is linked where it is made. The main primary sources, all read on 2026-10-14:

- **Registries:** [npm registry](https://registry.npmjs.org/) and [downloads API](https://api.npmjs.org/downloads/) (week 2026-10-06 to 2026-10-12), [PyPI JSON](https://pypi.org/pypi/) and [pypistats](https://pypistats.org/api/), [crates.io API](https://crates.io/api/v1/crates/), [Go module proxy](https://proxy.golang.org/), [Maven Central](https://repo1.maven.org/maven2/com/microsoft/playwright/driver-bundle/), [NuGet](https://api.nuget.org/v3-flatcontainer/microsoft.playwright/), GHCR and MCR image manifests.
- **Stagehand:** [repository](https://github.com/browserbase/stagehand) at `main` (packages, CHANGELOG, extension manifest and source, protocol README), the v4 docs sources under [packages/docs/v4](https://github.com/browserbase/stagehand/tree/main/packages/docs/v4) (configuration/browser, configuration/models, best-practices/caching and deployments, migrations/v3 and playwright), the 4.2.1 npm tarball, [Browserbase pricing](https://www.browserbase.com/pricing).
- **Playwright:** [release notes @v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/release-notes-js.md), [browsers.json](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/browsers.json), [actionability](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/actionability.md), [injectedScript.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/injected/src/injectedScript.ts), [class-browsertype](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-browsertype.md), [playwright-mcp README](https://github.com/microsoft/playwright-mcp/blob/main/README.md), [playwright-cli README](https://github.com/microsoft/playwright-cli/blob/main/README.md), [playwright-python setup.py](https://github.com/microsoft/playwright-python/blob/main/setup.py), [Playwright.csproj](https://github.com/microsoft/playwright-dotnet/blob/main/src/Playwright/Playwright.csproj).
- **Puppeteer and DevTools MCP:** [installation guide @v25.13.0](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/installation.md), [ConnectOptions.ts](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/packages/puppeteer-core/src/common/ConnectOptions.ts), [page interactions](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.13.0/docs/guides/page-interactions.md), [chrome-devtools-mcp README and docs](https://github.com/ChromeDevTools/chrome-devtools-mcp).
- **Browser Use:** [pyproject @0.13.11](https://github.com/browser-use/browser-use/blob/0.13.11/pyproject.toml), [agent/service.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py), [browser/profile.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py), [default_action_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/default_action_watchdog.py), [security_watchdog.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/security_watchdog.py), [cli.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/cli.py), the [docs index](https://docs.browser-use.com/llms.txt) and its pages linked in §5, [browser-harness](https://github.com/browser-use/browser-harness), [Browser Use Terminal](https://github.com/browser-use/terminal).
- **Others:** [Skyvern README and pyproject](https://github.com/Skyvern-AI/skyvern), [Midscene docs sources](https://github.com/web-infra-dev/midscene/tree/main/apps/site/docs/en), [agent-browser README and CHANGELOG](https://github.com/vercel-labs/agent-browser), [steel-browser](https://github.com/steel-dev/steel-browser) and [Steel docs](https://docs.steel.dev/llms.txt), [Notte](https://github.com/nottelabs/notte), [Magnitude browser-agent](https://github.com/magnitudedev/browser-agent), [Lightpanda](https://github.com/lightpanda-io/browser), [Crawlee stagehand-crawler](https://github.com/apify/crawlee/tree/master/packages/stagehand-crawler).
- **Issue trackers** of each product, searched through the GitHub REST search API sorted by reactions; each issue is linked where cited.
