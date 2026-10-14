# Consumer API of AI browser-automation libraries: what developers get, like and hate

Research of **2026-10-14** for Goodfetch's consumer-API decision for **Collie** (one page: Playwright-shaped page API plus `act`/`observe`/`extract`/agent) and, by reflection, **Foxhound**. Slice: AI browser-automation libraries. Priority: Stagehand (very deep), Browser Use (very deep), then Midscene.js, Vercel agent-browser, Skyvern, Playwright MCP / playwright-cli (plus Playwright core's own AI experiments), Notte, LaVague, with HyperAgent and Magnitude as Stagehand-shaped comparators. Like everything in `rnd/`, this informs a decision and adopts nothing.

**How to read it.** §1 sources and method. §2 one section per library: a walkthrough with a short code sample per supported language (written for this note against the docs, not copied, not executed), then the dimensions (lifecycle, configuration, verbs and types, determinism, page-API mixing and tabs, agent API, errors, models and cost, observability, sync/async and parity, breaking changes), then that library's pains and likes. §3 the cross-library comparison table. §4 what nearly everyone converged on and where they diverge. §5 likes, ranked. §6 pains, ranked by evidence weight, each with the API lesson it implies. §7 open questions and unverified items. Appendix A holds the forum evidence tables.

## 1. Sources and method

**Versions inspected (all read 2026-10-14).**

| Library | Version (date) | License | Languages | Primary sources read |
|---|---|---|---|---|
| Stagehand | 4.2.1 npm (2026-10-11) and PyPI (2026-10-10); docs and SDK source at `main` [`1da7359`](https://github.com/browserbase/stagehand/tree/1da7359172f1c057487b8918af118e389fe76615) (2026-10-13); v3 docs in the same tree; `v3-latest` 3.7.3 | MIT | TS, Python, Go | v4 and v3 docs MDX, `sdk-ts/src`, `sdk-python/src`, `extension/services`, npm registry and per-version downloads API, 402 issues (top 60 by reactions and by comments, all v4-era), 34 Discussions, archived `stagehand-python` tracker, [v4 blog](https://www.browserbase.com/blog/stagehand-v4) |
| Browser Use | 0.13.11 (2026-10-07), tag commit `2d6f9d3` | MIT | Python (no official TS) | source tarball at the tag (agent, tools, browser, actor, beta, llm, telemetry), 138 release notes, ~70 issue threads, 321 Discussions, 25 doc pages, npm/PyPI |
| Midscene.js | 1.15.4 (2026-10-14) | MIT | TS only | midscenejs.com `.md` pages (reference, caching, model config), changelog, repo tree, top issues |
| Vercel agent-browser | 0.39.1 (2026-10-13) | Apache-2.0 | CLI (Rust); no library | README, docs MDX, CHANGELOG, `cli/src/connection.rs`, `main.rs`, top issues |
| Skyvern | 1.0.56 (2026-10-14), tag `0bfb831` | AGPL-3.0 | Python, TS | `skyvern/library/*`, `skyvern-ts/client`, docs `.md` (sdk-reference, production, configuration), changelog, top issues |
| Playwright MCP / playwright-cli / core | `@playwright/mcp` 0.0.83 (2026-09-28), `@playwright/cli` 0.1.22, `playwright` 1.64.1 (2026-10-14) | Apache-2.0 | MCP, CLI; core in TS/Py/Java/.NET | READMEs at all 73 MCP tags (diffed), `packages/playwright-core/src/tools/*` at v1.64.1, release notes 1.56–1.64, hidden `PageAgent` docs at v1.58.0 |
| Notte | 1.9.7 (2026-10-14) | SSPL-1.0 | Python (local + cloud), TS (cloud only) | source at tag, docs.notte.cc, PyPI/npm |
| LaVague | last commit 2025-01-21, last PyPI 2024-08 | Apache-2.0 | Python | GitHub API, PyPI, open issues |
| HyperAgent / Magnitude | 1.1.2 (2026-01-07) / 0.3.1 (2026-02-08) | AGPL-3.0 / Apache-2.0 | TS | README, types, issues |

**Method.**
- Facts about APIs come from source at a pinned tag or commit, or from the official docs at the date above; each claim links its source. Defaults were checked in code where docs and code could disagree (they did several times: Stagehand's cache default, Browser Use's `max_steps`, Browser Use actor methods).
- User evidence: GitHub issues ranked by reactions and comments (`gh api search/issues … sort=reactions|comments`), the threads read, Discussions via GraphQL; Hacker News through the Algolia API (about 1,100 comments scanned, 27+ threads read in full); Stack Overflow API; 13+ blogs and engineering posts opened. Each item is classified independent / vendor / competitor / adjacent (Appendix A), and only independent reports are counted.
- **Reddit was unreachable** from every route tried (WebSearch domain filter, WebFetch on old.reddit.com, the JSON endpoints, the PullPush archive). No Reddit evidence is in this note beyond what earlier notes already cite. Exa was not used (no credits).
- Prior notes reused and re-checked: [2026-10-07 browser frameworks](https://github.com/goodfetch/automan/blob/main/rnd/research/2026-10-07-browser-frameworks.md), [Stagehand usage](https://github.com/goodfetch/automan/blob/main/rnd/research/stagehand-usage.md), [Stagehand alternatives](https://github.com/goodfetch/automan/blob/main/rnd/research/2026-10-07-stagehand-alternatives-followup.md), [Reddit evidence](https://github.com/goodfetch/automan/blob/main/rnd/research/2026-10-07-reddit-evidence-followup.md), [2026-10-10 AI browser frameworks](https://github.com/goodfetch/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md), [2026-10-14 browser-automation products](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md). Two corrections to them: Stagehand's cache **is** on by default since PR [#2964](https://github.com/browserbase/stagehand/pull/2964) (2026-09-22), so the 4.1.0 docs/code divergence is closed in 4.2.x; and Browser Use's default `max_steps` is 500 in code, not the documented 100.
- Reaction counts are small for most of these trackers, and support often lives on Discord or Slack. Counts are signal direction, not prevalence. Quotes are under 15 words.

## 2. Libraries

One section per library. Each starts with a walkthrough and code sample, then the API dimensions, then pains and likes. Stagehand (§2.1) and Browser Use (§2.2) are the deep ones.

## 2.1 Stagehand (Browserbase), v1 to v4, TypeScript, Python and Go

**Versions read.** npm `@browserbasehq/stagehand` `latest` = **4.2.1** (2026-10-11), `v3-latest` = 3.7.3; majors shipped 1.0.0 (2024-10-29), 2.0.0 (2025-03-31), 3.0.0 (2025-10-29), 4.0.0 (2026-08-10) ([npm registry](https://registry.npmjs.org/@browserbasehq/stagehand)). PyPI `stagehand` 4.2.1 (2026-10-10). Docs and SDK source read from the monorepo at `main` commit [`1da7359`](https://github.com/browserbase/stagehand/tree/1da7359172f1c057487b8918af118e389fe76615) (2026-10-13), two days after 4.2.1; the v3 docs are still in the same tree under `packages/docs/v3`. The design rationale for v4 is the vendor post [Stagehand v4](https://www.browserbase.com/blog/stagehand-v4) (2026-08-10).

### Walkthrough (v4)

A browser comes from a **factory** (`localBrowser.launch()`, `localBrowser.connect({ cdpUrl })`, `browserbase.launch({ apiKey })`, `browserbase.connect(...)`); `Stagehand.create({ browser, ... })` attaches the AI runtime to it. AI verbs live on the `Stagehand` instance; the Playwright-shaped `Page`, `Locator` and `BrowserContext` live on the browser handle. Both objects are closed separately, in reverse order.

TypeScript (minimal, written against [reference/stagehand.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/stagehand.mdx)):

```ts
import { localBrowser, Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod/v4";

const browser = await localBrowser.launch({ headless: true });
try {
  const sh = await Stagehand.create({
    browser,
    model: { modelName: "openai/gpt-5.4-mini", apiKey: process.env.OPENAI_API_KEY! },
  });
  try {
    const page = await browser.context.activePage();
    await page!.goto("https://example.com/catalog");
    await page!.locator("#q").fill("lamp");                    // deterministic, no model
    const { data: candidates } = await sh.observe("the search submit button");
    const r = await sh.act(candidates[0]);                       // replay, no inference
    if (!r.data.success) throw new Error(r.data.message);
    const { data, metadata } = await sh.extract(
      "first product name and price",
      z.object({ name: z.string(), price: z.number() }),
    );
    console.log(data.price, metadata.usage.inputTokens, metadata.cache.status);
  } finally { await sh.close(); }
} finally { await browser.close(); }
```

Python (async only; keyword-only options; Pydantic class as schema):

```python
import asyncio, os
from pydantic import BaseModel
from stagehand import Stagehand, local_browser

class Product(BaseModel):
    name: str
    price: float

async def main() -> None:
    browser = await local_browser.launch()
    try:
        sh = await Stagehand.create(browser=browser, model="openai/gpt-5.4-mini",
                                    model_api_key=os.environ["OPENAI_API_KEY"])
        try:
            page = await browser.context.active_page()
            await page.goto("https://example.com/catalog")
            res = await sh.act("type %q% into the search box", variables={"q": "lamp"})
            out = await sh.extract("first product name and price", Product, page=page)
            print(out.data.price, res.data.success)
        finally:
            await sh.close()
    finally:
        await browser.close()

asyncio.run(main())
```

### Entry point and lifecycle

- **Construction.** `Stagehand.create()` is "the only way to build one; there is no public constructor", and the instance comes back initialized ([reference](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/stagehand.mdx)). In code the constructor is `private`, and `create()` parses options with a Zod schema, claims the browser, and on an *ambiguous* init failure invalidates the browser handle (`AggregateError` if that cleanup also fails) ([stagehand.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/stagehand.ts)).
- **Ownership.** "Each handle can back only one Stagehand instance." `close()` releases the runtime and leaves the browser open; "Stagehand never closes a browser it did not launch" ([browser.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/browser.mdx)). After `close()`, every call throws `Stagehand is unavailable. Create a new instance…`.
- **No scoped disposal.** Neither SDK implements `Symbol.asyncDispose` (`await using`) or `__aenter__` (`async with`); a grep of `sdk-ts/src` and `sdk-python/src` at `1da7359` finds none. Every doc example nests two `try/finally` blocks, and the repo's own AI-rules file tells agents to "Wrap every workflow in `try`/`finally`".
- **Implicit state.** The AI verbs target "the active page", which is "whichever tab Chrome has focused in its last-focused window", so a newly opened tab silently becomes the target ([using-multiple-tabs.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/best-practices/using-multiple-tabs.mdx)). Passing `{ page }` (TS) / `page=` (Python) pins the target. Page and context getters are async because they query the browser.
- **Environment.** "Stagehand never reads environment variables on your behalf" ([quickstart](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/first-steps/quickstart.mdx)). This reverses v3, whose string model form "auto-loads model API key from env" ([v2→v3 migration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/migrations/v2.mdx)).
- **Local launch requirements.** Local runs need an installed Chrome; the SDK loads its MV3 extension over CDP and adds `--enable-unsafe-extension-debugging` (prior note [2026-10-14 browser-automation products](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §2.3, re-read in browser.mdx).

### Configuration

One options object on `create()` (TS: `browser, apiKey, apiUrl, model, telemetry, systemPrompt, selfHeal, domSettleTimeoutMs, cache, logging`; Python: the same as snake_case keyword-only args, except that the model is **flattened** to `model=`, `model_api_key=`, `model_headers=`) ([stagehand.py](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-python/src/stagehand/stagehand.py)). Per-call options mirror the instance ones (`model`, `cache`, `timeout`, `variables`, `locator`, `ignoreLocators`, `page`, and `screenshot` on extract). No config file, no env vars.

Defaults that bite:

| Default | Evidence | Why it bites |
|---|---|---|
| **Server cache on** for Browserbase browsers | `defaultCaching: initParams.cache ?? true` in [cacheService.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/extension/services/cacheService.ts), since PR [#2964](https://github.com/browserbase/stagehand/pull/2964) (2026-09-22, so in 4.2.0); 4.1.0 shipped `?? false` while the docs already said "on by default" (prior note 2026-10-07). | `%var%` secret values are sent to the cache service; the docs say to pass `cache: false` on credential calls ([act.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/basics/act.mdx)). A secure-by-default API would invert this. |
| Cache has **no effect locally** | "the `cache` option has no effect and every call runs inference" ([caching.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/best-practices/caching.mdx)) | Same code, different cost profile depending on where the browser runs. |
| **No `model` → Model Gateway** | Omitting `model` routes through Browserbase and it "selects a model automatically" per call ([models.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/models.mdx)) | Model can change between calls without a code change; local browsers error instead. |
| `selfHeal` off | `selfHeal = false` in [actService.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/extension/services/actService.ts) | The headline "self-healing" is opt-in in v4 (v2 Python had `self_heal=True` in examples). |
| `goto` waits for `domcontentloaded`; no auto-wait | "NOTHING AUTO-WAITS. Stagehand resolves a selector once and throws" ([playwright.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/migrations/playwright.mdx)) | Ported Playwright scripts flake; the docs ship a hand-written retry loop. |
| Zod pinned | "Keep Zod on the `4.4.x` minor… Newer Zod minor versions can cause TypeScript errors" (quickstart) | The schema library's minor version is part of Stagehand's public type contract (see pains). |
| Logs always to stderr | `process.stderr.write` is unconditional before `onLog` ([#3035](https://github.com/browserbase/stagehand/issues/3035), 9 reactions, open) | No way to route logs only to the app's logger. |
| Domain policy `null` → fail-open | [#3002](https://github.com/browserbase/stagehand/issues/3002) (open) | Safety control is opt-in. |

### Core verbs and signatures (v4)

| Verb | TS signature | Python | Returns |
|---|---|---|---|
| `act` | `act(instruction: string \| Action, options?)` (overloads) | `act(instruction: str \| Action, *, page, model, variables, timeout, locator, ignore_locators, cache)` | `ActResult = { data: { success, message, actionDescription, actions: Action[] }, metadata: { actionId?, cache: { status: "HIT"\|"MISS"\|"DISABLED", missReason?, tokensSaved? }, usage: { inputTokens, outputTokens, reasoningTokens, cachedInputTokens, inferenceTimeMs } } }` |
| `observe` | `observe(instruction?: string, options?)` | `observe(instruction=None, *, …)` | `{ data: Action[], metadata }`; `Action = { selector (XPath), description, method?, arguments? }` |
| `extract` | `extract(instruction)`, `extract(instruction, zodSchema, options?)` | `extract(instruction, schema: type[BaseModel] = DefaultExtract, *, …)` | `{ data: z.output<Schema> \| Model, metadata }`; no schema → `{ extraction: string }` |
| `metrics` | `metrics(): Promise<StagehandMetrics>` | same | per-verb prompt/completion/reasoning/cached tokens and inference ms, cumulative per instance |
| `experimentalBatch` | `(callback, input?, { page, timeout })` | `experimental_batch(source: str, input, timeout, page)` | JSON value; callback runs in the extension service worker; "can change in patch releases" |

Sources: [reference/stagehand.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/stagehand.mdx), [stagehand.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/stagehand.ts), [stagehand.py](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-python/src/stagehand/stagehand.py).

- **Schemas.** TS converts the Zod schema with `z.toJSONSchema()` before sending it over the protocol and re-validates the reply with `schema.parse()`; Python sends `model_json_schema()` and re-validates with `model_validate()`. The wire contract is JSON Schema; the typed language layer is a thin wrapper. Python's `experimental_batch` takes **JavaScript source as a string**, since the callback runs in the browser: the batch API cannot be idiomatic in non-JS languages.
- **Variables.** `%name%` placeholders in `act`/`observe`, with values substituted in the runtime and not sent to the model; cache keys use the variable *keys* (v3 caching doc).
- **Raw page API.** `Page`: `goto, reload, goBack, goForward, click(x,y), hover, scroll, dragAndDrop, type, keyPress, evaluate, on("console") only, onToolsAdded/Removed (WebMCP), addInitScript, setExtraHTTPHeaders, setViewportSize, waitForLoadState, waitForTimeout, waitForSelector (returns boolean), screenshot, pdf (4.2.0), snapshot, tools, url, title, close, locator`. `Locator`: `click, hover, fill, count, isChecked, inputValue, isVisible, innerText, innerHtml, textContent, scrollTo, centroid, highlight, sendClickEvent, type, selectOption, setInputFiles, first, nth`. `BrowserContext`: `pages, newPage, activePage, setActivePage, close, addInitScript, setExtraHTTPHeaders, get/setDomainPolicy, cookies, addCookies, clearCookies`. Locators pierce iframes (`>>`) and closed shadow roots. No `getBy*`, `filter`, `route`, `waitForResponse`, `expect`, multiple contexts, Firefox or WebKit ([page.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/page.mdx), [locator.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/locator.mdx), [context.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/reference/context.mdx), [playwright.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/migrations/playwright.mdx)).
- **Mixing AI and code.** AI verbs accept a `locator` (scope the snapshot to one subtree) and `ignoreLocators` (exclude subtrees), both built with `page.locator()`. Either one makes the call bypass the cache. `page.snapshot()` returns `{ formattedTree, xpathMap, urlMap }` without a model, which is what tool-calling agents read. **No interop with Playwright objects**: "You can't hand a Playwright `Page` to `act()`" (playwright.mdx). v3 accepted Playwright, Puppeteer and Patchright pages through `{ page }` ([v3 playwright integration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/integrations/playwright.mdx)).

### Determinism, cache and self-heal

- `observe()` → `act(action)` is the documented deterministic path: replaying an `Action` "skips model inference, the page snapshot, and the DOM-settle wait, and it does not consult the server-side cache"; with `selfHeal` on, a broken selector triggers one re-inference ([act.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/basics/act.mdx)). The selector is an **absolute XPath** (`xpath=/html[1]/body[1]/div[1]/span[1]` in the doc example), which users call brittle: "the brittle xpath selectors are a dealbreaker" ([#1555](https://github.com/browserbase/stagehand/issues/1555), closed without a merge after the author got no response).
- The cache is server-side only and keyed on instruction + page content + options; model config is excluded on purpose ([caching.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/best-practices/caching.mdx)). v4 removed the v3 local file cache (`cacheDir`) and the v3 `history` accessor ("There is no history accessor"; rebuild it from `onLog`, [observability.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/observability.mdx)).
- `metadata.cache` (`status`, `missReason`, `tokensSaved`) is a good per-call observability shape: the caller can see *why* a call was or was not served from cache.
- Open, unmerged: PRs [#2952–#2955](https://github.com/browserbase/stagehand/pull/2954) and [#3103](https://github.com/browserbase/stagehand/pull/3103) (`experimentalDecisions` namespace with abstention and semantic cache checks); prior note 2026-10-07.

### Agent API

v4 has none. "`agent()` is gone. Nothing in v4 replaces it one-for-one." The stated reason: `agent()` "was built for models that couldn't reliably drive a browser on their own"; now "v4 exposes discrete tools and leaves the control flow to you" ([v3 migration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/migrations/v3.mdx)). Replacements: **code mode** (a coding assistant writes a Stagehand script once; "free of per-step inference") or **tool calling** over the full page API (a table maps capabilities to tools). The migration table is explicit about what disappeared without an equivalent: "Agent streaming, callbacks, abort signal, message continuation: No equivalent. Your loop sits between steps", `agent({ mode: "cua" })` and `highlightCursor`. The integrations package exposes three tools (`run`, `snapshot`, `screenshot`) behind MCP or native framework bindings, all marked experimental. `browserbase.com/blog/stagehand-v4` lists `agent()` among removed APIs "but gives no explicit reason" beyond the docs' rationale.

For contrast, v3's agent was `stagehand.agent({ mode: "cua" | "dom" | "hybrid", model, systemPrompt, tools, integrations })` then `agent.execute({ instruction, maxSteps, highlightCursor })` → `AgentResult { success, message, actions, completed, usage }`, with a per-mode feature matrix (CUA mode had no variables, streaming, callbacks, abort signal or structured output; "Non CUA agents are currently only supported in TypeScript") ([v3 agent.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/basics/agent.mdx)).

### Errors and failure surfaces

- **Two channels for failure.** `act` *returns* `success: false` with a message when the model finds no element, the method is unsupported, or the deterministic action throws (without self-heal); it *throws* on `TimeoutError` and on model, RPC or validation errors ([actService.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/extension/services/actService.ts)). Callers must check both.
- **Untyped errors.** RPC errors surface in TS as a plain `Error(message, { cause: jsonRpcError })`, renamed `TimeoutError` only for timeouts ([rpcClient.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/rpcClient.ts)); Python raises `RPCError(RuntimeError)` with `code` and `data`. Exported error classes are transport-level only: `CDPConnectionClosedError`, `RPCResponseTimeoutError`, `StagehandRuntimeIncompatibleError`, `StagehandBatchTimeoutError`, `BrowserbaseSessionError` ([index.ts](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/sdk-ts/src/index.ts)). The docs' own troubleshooting recipe branches on `message.includes("method not supported")` in TS and `strings.Contains(err.Error(), "method not supported")` in Go (act.mdx), i.e. error handling by string matching.
- **Side effects before failure.** The docs warn that a failed `act()` may already have acted, and recommend escalating through `observe()` rather than blind retries (prior note, v3 migration). The v4 driver has no actionability checks, so `locator.click()` can resolve without committing ([#2486](https://github.com/browserbase/stagehand/issues/2486), open).
- **Cancellation.** Per-call `timeout` (ms) in both SDKs; no `AbortSignal` parameter on `act/observe/extract` (the SDK uses internal signals only for init). Batch timeout "is not an atomic transaction".
- **v2/v3 history.** v2 wrapped failures in `StagehandDefaultError` with a generic "Hey! We're sorry you ran into an error" banner ([#704](https://github.com/browserbase/stagehand/issues/704)); schema failures reported only "Invalid response schema" until users asked for `safeParse` detail ([#689](https://github.com/browserbase/stagehand/issues/689), fixed 2025-05).

### Model configuration and cost

- `model: { modelName: "provider/model", apiKey, headers }` (TS) with a provider prefix; omit for the Model Gateway on Browserbase; or pass a **client-side generate callback** for anything else (Bedrock, Azure, self-hosted). The callback receives provider-neutral `{ messages, systemPrompt, temperature, responseFormat }` and runs in the caller's process over the bidirectional RPC ([models.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/models.mdx)). The TS SDK "does not re-export `LLMGenerateParams` yet", so users retype it by hand (same page).
- Cost: `metadata.usage` per call and cumulative `metrics()`; cache hits and deterministic actions report zero usage. No budget cap (a community discussion shows how to build one: [#3097](https://github.com/browserbase/stagehand/discussions/3097)).
- History of churn: v2 `modelName` + `modelClientOptions` → v3 `model: string | { modelName, apiKey, baseURL }` → v4 `model: { modelName, apiKey, headers } | callback` (no `baseURL`; "no base URL option" per prior note) and Python flattened to `model=`/`model_api_key=`.

### Observability

`logging: { level: debug|info|warn|error|off, format: pretty|json, onLog }` (stderr always written, #3035); OpenTelemetry spans for every operation and log record with W3C trace-context propagation across SDK and runtime, exported to an OTLP endpoint passed in `telemetry.traces` ([observability.mdx](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/configuration/observability.mdx)); file-based JSONL session logs and LLM inference debugging (logging.mdx). Session recordings, live view and session analytics are Browserbase features. A v4 bug sent default traces to a placeholder `example.com` endpoint ([#2732](https://github.com/browserbase/stagehand/issues/2732), closed).

### Sync/async, parity, typing

- TS and Python are async-only; Python ≥3.11, ships `py.typed`, generated Pydantic wire models; Go uses `context.Context` and generics (`stagehand.Extract[T]`). The three SDKs are generated from one Zod protocol (`stagehand.v4.json`), so v4 is the first version where all three expose the same verbs (v4 migration: "v4 gives all three languages the same SDK").
- Residual asymmetries: Python model config is flat on `create()` but an object per call; TS takes an options object, Python keyword-only args; `experimentalBatch` takes a closure in TS and a JS string in Python; Zod minor pin in TS only.
- **Python went through three unrelated API shapes in 16 months**: v2 `stagehand-py` (`StagehandConfig`, `stagehand.page.act`), v3 a Stainless-generated REST client (`AsyncStagehand().sessions.create(model_name=…)`, `session.act(input=…)`, `response.data.result`, embedding a ~103 MB Node server for local mode), then v4 `Stagehand.create(browser=…)` ([v3 Python migration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/migrations/python.mdx), [v3 Python SDK page](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/sdk/python.mdx)). The v3 Python client "does **not yet support**" custom LLM client classes. The Python SDK waitlist thread drew 28 comments over five months ([#393](https://github.com/browserbase/stagehand/issues/393)).

### Breaking-change timeline

| Version (date) | What broke for the consumer | Source |
|---|---|---|
| 1.x → 2.0 (2025-03-31) | `extract` pipeline trimmed content sent to the model; link URLs stopped being extracted ("Cannot extract link targets (anymore)", 18 comments) until `z.string().url()` special-casing; ability to init from an existing Playwright page removed in favour of Browserbase session IDs (per a user reading PR #296) | [#651](https://github.com/browserbase/stagehand/issues/651), [#507](https://github.com/browserbase/stagehand/issues/507) |
| 2.0 → 3.0 (2025-10-29) | Playwright dependency removed (own CDP "understudy"); `stagehand.page` → `stagehand.context.pages()[0]`; `page.act({action, modelName, timeoutMs, iframes})` → `stagehand.act(str, { model, timeout, page })`; `page.extract({instruction, schema})` → `stagehand.extract(instruction, schema)`; `modelName`/`modelClientOptions` → `model`; `domSettleTimeoutMs` → `domSettleTimeout`; `enableCaching` → `cacheDir`; `init()` no longer returns session URLs. Python became a REST client. Local storage-state persistence broke (`userDataDir`, no `storageState()`), open 11 months | [v2→v3 migration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v3/migrations/v2.mdx), [#1250](https://github.com/browserbase/stagehand/issues/1250) |
| 3.x → 4.0 (2026-08-10) | Private constructor + `Stagehand.create({ browser })`; browser factories; async page getters; results wrapped in `{ data, metadata }`; `agent()`, local cache, `history`, `deepLocator`, Playwright/Puppeteer interop, `browserbaseSessionID` removed; `domSettleTimeout` back to `domSettleTimeoutMs`; `verbose`/`logger` → `logging`; `metrics` property → method | [v3→v4 migration](https://github.com/browserbase/stagehand/blob/1da7359172f1c057487b8918af118e389fe76615/packages/docs/v4/migrations/v3.mdx) |

The v4 migration's quick-reference table maps from `page.act(...)` and `extract({ instruction, schema })`, which are **v2** shapes the v3 docs had already replaced; the guide is partly a v2→v4 table (docs inaccuracy, minor).

**Adoption of v4, measured.** npm per-version downloads for the week to 2026-10-13 (two months after 4.0.0): v4 **28.8%**, v3 **40.4%**, v2 3.9%, v1 **26.0%** (almost all `1.14.0`), alphas 0.9%, of 1,640,981 ([npm versions API](https://api.npmjs.org/versions/@browserbasehq%2Fstagehand/last-week), measured 2026-10-14). Downloads count CI and transitive installs, and which dependent pins 1.14.0 was not identified (**unverified**). The v3 line is still patched (`v3-latest` 3.7.3, 3.7.4 alphas on 2026-10-14), and Crawlee's `@crawlee/stagehand` still depends on v3 ([apify/crawlee#4196](https://github.com/apify/crawlee/issues/4196), open). No GitHub issue or discussion objecting to the removal of `agent()` was found (searches for "agent removed", "v4 agent", "bring back agent", 2026-10-14); the tracker is low-traffic because support runs on Slack/Discord, so absence of complaints is weak evidence.

### Stagehand pains from the tracker (ranked by evidence weight)

GitHub, read 2026-10-14: 402 issues; top 60 by reactions and by comments, every v4-era issue (28 created since 2026-08-01), and 34 Discussions. Reaction counts are low because support runs on Slack and Discord; Discussions are dominated by third-party promotion. Keyword issue counts (title or body, not deduplicated): `playwright` 64, `local model` 47, `zod` 42, `custom llm` 25, `cache` 21, `ollama` 15, `iframe` 12.

| # | Pain | Evidence (reactions r / comments c, dates) | Status 2026-10-14 | API lesson |
|---|---|---|---|---|
| 1 | **Rerun without the model; export the plan** | "Return plan for extraction" [#389](https://github.com/browserbase/stagehand/issues/389) (16r/14c, 2025-01, users propose `extract.compile(task) -> plan` with a success condition); XPath caching [#404](https://github.com/browserbase/stagehand/issues/404) (6r); "rerun… without more OpenAI calls?" [#255](https://github.com/browserbase/stagehand/issues/255) (7c); stable selectors instead of absolute XPath [#1555](https://github.com/browserbase/stagehand/issues/1555) (3r/5c, closed unmerged, "a dealbreaker to us"); JSON/DB workflow definitions [#793](https://github.com/browserbase/stagehand/issues/793) | #389 open 21 months; v3 local cache removed in v4; only server cache | Make the plan a first-class, local, versioned value (`observe` → `Action` is the seed), with stable selectors and a success predicate. |
| 2 | **Model and provider breadth** | Ollama [#183](https://github.com/browserbase/stagehand/issues/183) (19r), AI SDK `LanguageModelV3` [#1645](https://github.com/browserbase/stagehand/issues/1645) (16r, open, draft PR unreviewed), "llm Options" [#307](https://github.com/browserbase/stagehand/issues/307) (14r), provider key instead of model→provider mapping [#790](https://github.com/browserbase/stagehand/issues/790) (7r), Ollama docs [#1164](https://github.com/browserbase/stagehand/issues/1164), DeepSeek structured output [#901](https://github.com/browserbase/stagehand/issues/901), [#1204](https://github.com/browserbase/stagehand/issues/1204), ZhiPu [#1510](https://github.com/browserbase/stagehand/issues/1510) (9c), Bedrock structured mode [#2565](https://github.com/browserbase/stagehand/issues/2565) | v4 answer is the BYO `generate` callback; native set is 5 providers | Accept an OpenAI-compatible base URL natively plus a callback; key provider capabilities (structured output mode) by declaration, not by parsing model IDs. |
| 3 | **Interop with existing Playwright/browser** | Attach to a Playwright page for video in tests [#507](https://github.com/browserbase/stagehand/issues/507) (18r/13c, incl. Crawlee's maintainer asking for page reuse to integrate natively), bring your own browser [#391](https://github.com/browserbase/stagehand/issues/391) (7r/15c: custom `newContext` options "a major blocker"), CDP connect [#565](https://github.com/browserbase/stagehand/issues/565) (6r), 13 of 15 scripts blocked from leaving Playwright [#2744](https://github.com/browserbase/stagehand/issues/2744) | v3 added interop, **v4 removed it**; CDP attach exists | Let AI verbs accept a page owned by another driver, or expose the engine as an attachable sidecar. |
| 4 | **Schema-library coupling (Zod)** | Zod 4 broke Stagehand 2.4–2.5: [#1029](https://github.com/browserbase/stagehand/issues/1029) (9c), [#938](https://github.com/browserbase/stagehand/issues/938) (`schema._def.shape is not a function`), [#1098](https://github.com/browserbase/stagehand/issues/1098) (7r), Mastra integration blocked; fixed 2025-11-21 after ~3 months; v4 quickstart still pins Zod `~4.4.3`. Python: JSON Schema not accepted [stagehand-python#193](https://github.com/browserbase/stagehand-python/issues/193) | fixed; pin remains | Wire contract in JSON Schema; accept Standard Schema / JSON Schema / Pydantic without depending on private internals. |
| 5 | **Runtime portability** | Cloudflare Workers [#564](https://github.com/browserbase/stagehand/issues/564) (12r), [#164](https://github.com/browserbase/stagehand/issues/164) (11r), [#1595](https://github.com/browserbase/stagehand/issues/1595) (open); Lambda bundling [#845](https://github.com/browserbase/stagehand/issues/845) (open); Node 24/Bun [#1552](https://github.com/browserbase/stagehand/issues/1552); `pino-pretty` in Next.js [#1464](https://github.com/browserbase/stagehand/issues/1464); `dotenv` peer dep [#1109](https://github.com/browserbase/stagehand/issues/1109); Deno upload [#3152](https://github.com/browserbase/stagehand/issues/3152) | v4 cut deps to 4; Workers still open | Thin SDK with no Node-only APIs and no logging/env peer deps. |
| 6 | **Extraction correctness** | Links lost after 2.0 [#651](https://github.com/browserbase/stagehand/issues/651) (18c), "extract() totally broken for links" [#912](https://github.com/browserbase/stagehand/issues/912) (user read internal ids in logs as output), list completed early [#239](https://github.com/browserbase/stagehand/issues/239), wrong number [#771](https://github.com/browserbase/stagehand/issues/771) (open) | partially fixed via `z.string().url()` id mapping | Copy values from the page deterministically (ids → hrefs) rather than letting the model write them; expose completeness, not just parse success. |
| 7 | **Local-mode parity and session persistence** | "disclose project properly as an onramp to Browserbase" [#492](https://github.com/browserbase/stagehand/issues/492); v3 `userDataDir`/`storageState()` broken [#1250](https://github.com/browserbase/stagehand/issues/1250) (9c, open 11 months; "maybe not release v3 till the gaps are addressed"); v4 cache and Gateway Browserbase-only | open | Every SDK feature works locally; cloud adds capacity, not API surface. |
| 8 | **Observability of what the AI did** | "Make executed Playwright steps visible" [#186](https://github.com/browserbase/stagehand/issues/186) (8r), token usage per call [#268](https://github.com/browserbase/stagehand/issues/268) (6r, now `metadata.usage`), `onLog` forces stderr [#3035](https://github.com/browserbase/stagehand/issues/3035) (9r, the top v4 issue), Python v3 lost metrics [stagehand-python#299](https://github.com/browserbase/stagehand-python/issues/299) | partly fixed in v4 (`metadata`, OTel) | Return the resolved action and usage on every result; logging sinks fully caller-controlled. |
| 9 | **Driver guarantees after leaving Playwright** | no auto-wait (docs); `locator.click()` returns without committing [#2486](https://github.com/browserbase/stagehand/issues/2486); `waitForSelector` navigation race [#2982](https://github.com/browserbase/stagehand/issues/2982); `page.evaluate` "Promise was collected" [#2745](https://github.com/browserbase/stagehand/issues/2745); date/time inputs [#1249](https://github.com/browserbase/stagehand/issues/1249) | open | A Playwright-shaped API is judged against Playwright's actionability and auto-waiting. |
| 10 | **Safety controls fail open** | `setDomainPolicy` bypass [#2945](https://github.com/browserbase/stagehand/issues/2945), default `null` fail-open [#3002](https://github.com/browserbase/stagehand/issues/3002); secrets sent to cache by default (docs); Windows Python wheel flagged as malware [stagehand-python#348](https://github.com/browserbase/stagehand-python/issues/348) (3.21–3.22 embedded Node `.exe`) | open | Secure defaults; no embedded native executables in language packages. |

Likes for Stagehand (forum evidence, see Appendix A for the full table): "The act, extract, and observe methods are super clean." ([fbouvier, HN](https://news.ycombinator.com/item?id=42644964)); "actually a useful abstraction here" ([sparab18, HN](https://news.ycombinator.com/item?id=42638393)); "you can do pretty much anything while being provider agnostic" ([dimitri-vs, HN](https://news.ycombinator.com/item?id=43639238)); "Stagehand seems more token efficient." ([vishalanton, HN](https://news.ycombinator.com/item?id=49757217)); "I like the caching idea" while asking why it can't "just be a local file" ([ulrikrasmussen, HN](https://news.ycombinator.com/item?id=49763632)). On the v4 removal: a user asked "why not keep agent() alongside the new primatives?" and the vendor replied "We removed agent because so many great harnesses are available" ([HN 49757863](https://news.ycombinator.com/item?id=49757863), [49757944](https://news.ycombinator.com/item?id=49757944), 2026-09-18). A downstream Effect wrapper deferred v4 support, describing v4 as a rewrite rather than a version bump ([ADR in effect-libs-browser PR 19](https://github.com/LordCoughmann/effect-libs-browser/pull/19); paraphrase).

## 2.2 Browser Use (browser-use/browser-use, Python)

_Numbered subsections and "§n" references inside this library section are local to it._

Research date: 2026-10-14. Subject: `browser-use` 0.13.11 (GitHub release and PyPI upload 2026-10-07, tag `0.13.11` = commit `2d6f9d3`), MIT.
Purpose: input to an SDK API decision for a product with a Playwright-shaped page API plus AI primitives (act/observe/extract/agent) over one engine, with TS/Python/Rust bindings.

---

### 0. Sources for this section

| Source | What was read | Version/date |
|---|---|---|
| Source tarball `gh api repos/browser-use/browser-use/tarball/0.13.11` | `browser_use/__init__.py`, `agent/service.py` (4163 lines), `agent/views.py`, `tools/service.py`, `tools/registry/{service,views}.py`, `browser/{profile,session,views}.py`, `actor/{page,element,mouse,README.md}`, `beta/service.py` (6810 lines), `llm/{base,exceptions,views}.py`, `exceptions.py`, `config.py`, `telemetry/{service,views}.py`, `observability.py`, `BETA_AGENT_INTEGRATION_FEATURES.md` | 0.13.11 / `2d6f9d3` |
| PyPI JSON API | `requires_dist`, Python floor, upload time | 0.13.11, uploaded 2026-10-07T04:24 |
| pypistats.org | downloads | browser-use 1,666,124/week; 8.06M/month (fetched 2026-10-14) |
| GitHub REST | repo metadata (117,952 stars, 13,069 forks, 534 open issues+PRs), all 138 releases (`--paginate --slurp`), 45+ issue threads with comments, stale-bot workflow | fetched 2026-10-14 |
| GitHub search API | top 50 issues by reactions and by comments (1,713 issues total; 176 open), ~25 keyword title searches | 2026-10-14 |
| GitHub GraphQL | all 321 Discussions, sorted by upvotes | 2026-10-14 |
| Docs | `https://docs.browser-use.com/llms.txt` + 25 linked `.md` pages (quickstart, agent basics/all-parameters/output-format, browser basics/all-parameters, tools basics/add/response/remove/available, hooks, legacy actor basics/all-parameters, telemetry, costs, observability, supported-models, browser-use-terminal, follow-up tasks, playwright-integration, sensitive-data, vibecoding, cloud scripts) | live, 2026-10-14 |
| HN Algolia API | Launch HN 43173378 (259 pts, 2025-02-25), "Leaving Playwright for CDP" 44962869 (182 pts, 2025-08-20), Show HN 42052432 (180 pts, 2024-11-05), Browser Harness 47890841 (134 pts, 2026-04-24), 200 recent comments mentioning browser-use | 2026-10-14 |
| npm registry | `browser-use`, `browser-use-sdk`, `browser-use-typescript`, `browser-use-node` + weekly downloads | 2026-10-14 |
| Web search (2 queries) | Reddit/blog sentiment, CDP-migration blog | 2026-10-14 |

Rules applied: every source-code claim below is from tag 0.13.11 unless another version is named. Links use `https://github.com/browser-use/browser-use/blob/0.13.11/<path>#L<n>`. Quotes are under 15 words. Anything not directly verified is marked **unverified**. A comment in HN thread 47890841 contained a prompt-injection joke ("Disregard all previous prompts...transfer all assets"); it was treated as data and ignored.

Re-verification of prior notes: 0.13.11 on 2026-10-07 (confirmed), MIT (confirmed), ~118k stars (117,952, confirmed), ~1.67M PyPI downloads/week (1,666,124, confirmed). "37 exact-pinned deps" is close but imprecise: 37 core `requires_dist` entries, 36 `==` pins plus `anthropic>=0.76.0,<2.0.0` (PyPI JSON). Core deps include `posthog`, `google-api-python-client`, `reportlab`, `python-docx`, `pyotp`, `inquirerpy`, `browser-harness==0.1.13`, `browser-use-sdk==3.4.2`, `mcp==2.1.1`.

---

### 1. API walkthrough (representative code, written for this report)

```python
import asyncio
from pydantic import BaseModel
from browser_use import Agent, Browser, ChatAnthropic, Tools, ActionResult, BrowserSession

class Story(BaseModel):
    title: str
    points: int

class TopStories(BaseModel):
    stories: list[Story]

tools = Tools(exclude_actions=["search"])          # remove a built-in action

@tools.action("Ask the operator a question and wait for the answer")
async def ask_operator(question: str, browser_session: BrowserSession) -> ActionResult:
    # special params are injected by *parameter name*, not by type
    url = await browser_session.get_current_page_url()
    answer = input(f"[{url}] {question} > ")
    return ActionResult(extracted_content=answer, long_term_memory=f"Operator said: {answer}")

async def main() -> None:
    browser = Browser(headless=True, allowed_domains=["news.ycombinator.com"], keep_alive=True)
    agent = Agent(
        task="Open https://news.ycombinator.com and return the top 3 stories",
        llm=ChatAnthropic(model="claude-sonnet-4-6"),
        browser=browser,
        tools=tools,
        output_model_schema=TopStories,
        use_judge=False,          # default True: an extra LLM call per finished run
        calculate_cost=True,
        max_failures=3,
        step_timeout=120,
    )
    history = await agent.run(max_steps=25, on_step_end=lambda a: asyncio.sleep(0))
    if history.is_done() and history.is_successful():   # agent's self-report only
        result: TopStories | None = history.structured_output
        print(result, history.usage)
    else:
        print(history.errors())

    # Playwright-shaped "actor" layer on the same session (docs file it under "legacy")
    page = await browser.get_current_page()
    if page:
        el = await page.must_get_element_by_prompt("the 'More' link", llm=agent.llm)
        await el.click()
    await browser.kill()   # stop()/close() leave the browser process alive

asyncio.run(main())
```

Shape in one line: one big constructor (`Agent(task, llm, browser, tools, **~60 kwargs)`), one coroutine verb (`run(max_steps)`), a fat result object (`AgentHistoryList`), extension by decorated Python functions (`@tools.action`), and a separate, thinner CDP "actor" API reachable from the session.

---

### 2. Entry point and lifecycle

**Constructor.** `Agent.__init__(self, task: str, llm: BaseChatModel | None = None, ...)` ([agent/service.py#L136](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L136)). `task` is required at construction; `run()` takes no task. The class is `Agent(Generic[Context, AgentStructuredOutput])`.

**Run verbs.**
- `async run(max_steps: int = 500, on_step_start=None, on_step_end=None) -> AgentHistoryList[AgentStructuredOutput]` ([#L2503](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L2503)). Docs say the default is `100` ([agent basics](https://docs.browser-use.com/open-source/customize/agent/basics.md)); source says 500; the beta agent says 100. Doc/code drift.
- `run_sync(...)` = `asyncio.run(self.run(...))` ([#L4061](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L4061)). It cannot be called from inside a running loop (Jupyter, FastAPI), since `asyncio.run` refuses that.
- `step()`, `take_step()`, `multi_act()` are public but low-level.

**Browser/session names (current vs alias).**
- `BrowserSession` is the real class; `Browser` is an alias for it in `browser_use/__init__.py` (`'Browser': ('browser_use.browser', 'BrowserSession')  # Alias`). 0.7.0 release notes said "BrowserSession class has been renamed to Browser", but the code keeps `BrowserSession` as the class and `Browser` as alias; docs present `Browser` as primary ("Browser (alias: BrowserSession)").
- `BrowserProfile` = static config template (pydantic, **`extra='ignore'`**, [browser/profile.py#L585](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/profile.py#L585)); `BrowserSession` = live session (pydantic, **`extra='forbid'`**, [browser/session.py#L156](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session.py#L156)). Session accepts most profile fields flat via two `@overload`s (cloud vs local). Typos in a `BrowserProfile(...)` are silently dropped; typos in `BrowserSession(...)` raise.
- `Agent(browser=...)` and `Agent(browser_session=...)` are aliases; passing both raises `ValueError`. Same pattern for `tools`/`controller`, `skills`/`skill_ids`, `domains`/`allowed_domains` on `@action`.

**Implicit behaviour (what happens without asking).**
- Importing `browser_use` sets up logging handlers (unless `BROWSER_USE_SETUP_LOGGING=false`) and **monkeypatches** `asyncio.base_subprocess.BaseSubprocessTransport.__del__` globally (`browser_use/__init__.py`).
- `llm=None` resolves to `DEFAULT_LLM` env or `ChatBrowserUse()` (the vendor's paid cloud model; raises `ValueError` if `BROWSER_USE_API_KEY` missing) ([#L216-226](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L136)).
- If the LLM provider is `browser-use`, `flash_mode` is forced on, which disables planning.
- Model-name sniffing: LLM timeout chosen from the name (gemini 75/90 s, groq 30 s, claude/o3/deepseek 90 s, else 75 s); `llm_screenshot_size=(1400,850)` auto-set for `claude-sonnet*`; coordinate clicking auto-enabled for `claude-sonnet-4`, `claude-opus-4`, `claude-fable-5`, `gemini-3-pro`, `browser-use/`; vision forced off for `deepseek` and `grok-3`/`grok-code`.
- If no browser is given, the Agent creates `BrowserSession(browser_profile=DEFAULT_BROWSER_PROFILE copy)`; `run()` calls `browser_session.start()`. (In 0.2.1 users had to call `start()` manually: #1762, fixed in PR #1818.)
- If no Chromium is found, the local watchdog shells out to `uvx playwright install chromium` at runtime ([watchdogs/local_browser_watchdog.py#L360](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/watchdogs/local_browser_watchdog.py#L360)). Docs instead tell users to run `uvx browser-use install`.
- `enable_default_extensions=True` by default: downloads uBlock Origin, "I still don't care about cookies", ClearURLs `.crx` from `clients2.google.com` into `~/.config/browseruse/extensions` (profile.py#L648, #L1034). Opt-out env `BROWSER_USE_DISABLE_EXTENSIONS`. Chromium 139+ showed "extensions turned off" warnings (#2724, 2025-08-19).
- A URL found in the task text is auto-navigated as an initial action (`directly_open_url=True`, [#L462](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L462)).
- `enable_signal_handler=True`: `run()` installs SIGINT handlers. First Ctrl+C **pauses** (prints to stdout), second exits.
- `captcha_solver=True` watchdog (only active with vendor cloud browsers).
- Creates `~/.config/browseruse/{profiles,extensions}`, a `device_id`, and a temp dir `browser_use_agent_<id>_<ts>`.

**Closing and keep_alive.**
- `run()` always calls `agent.close()` in `finally` ([#L2700ff](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L2503)). `close()` calls `browser_session.kill()` unless `browser_profile.keep_alive` is truthy ([#L3978](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L3978)). `keep_alive` defaults to `None` (falsy), so **passing your own browser does not keep it alive** unless you set `keep_alive=True`.
- `BrowserSession.stop()` "without killing the browser process"; `close()` is an alias for `stop()` (added 0.12.7, PR #4665); `kill()` kills. So `await browser.close()` does not terminate Chrome. ([session.py#L727-784](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session.py#L727))
- `rerun_history()` also closes resources in `finally`.
- **No context managers**: no `__aenter__/__aexit__` on `Agent` or `BrowserSession` (grep of `browser_use/`: only `mcp/client.py`, `dom/service.py`, an Anthropic integration).
- Demo mode: on success `run()` sleeps 30 s before closing.

---

### 3. Configuration style

#### 3.1 Agent kwargs (0.13.11 signature, defaults)
From [agent/service.py#L136-214](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L136) and `AgentSettings` ([agent/views.py#L59](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/views.py#L59)):

| Group | Kwargs (default) |
|---|---|
| Core | `task` (required), `llm` (None→`DEFAULT_LLM`/`ChatBrowserUse`), `browser`/`browser_session`, `browser_profile`, `tools`/`controller`, `skills`/`skill_ids`, `skill_service` |
| Output | `output_model_schema` (None), `extraction_schema` (None, dict, per-page extract only), `display_files_in_done_text` (True) |
| Vision | `use_vision` (True; `'auto'` = screenshot tool only), `vision_detail_level` ('auto'), `llm_screenshot_size` (None; auto for Sonnet), `sample_images` |
| Prompting | `override_system_message`, `extend_system_message`, `use_thinking` (True), `flash_mode` (False), `include_attributes`, `include_tool_call_examples` (False), `include_recent_events` (False), `max_clickable_elements_length` (40000) |
| Limits/resilience | `max_failures` (5), `final_response_after_failure` (True; one extra attempt), `max_actions_per_step` (5), `step_timeout` (180 s), `llm_timeout` (None→name-based), `fallback_llm` (None), `loop_detection_enabled` (True), `loop_detection_window` (20), `enable_planning` (True), `planning_replan_on_stall` (3), `planning_exploration_limit` (5), `max_history_items` (None), `message_compaction` (True → `MessageCompactionSettings(compact_every_n_steps=25, keep_last_items=6, ...)`) |
| Judge | `use_judge` (**True**), `judge_llm` (None→same `llm`), `ground_truth` (None) |
| Other LLMs | `page_extraction_llm` (None→same `llm`) |
| Data | `sensitive_data` (None; `{name: value}` or `{domain_pattern: {name: value}}`), `initial_actions` (None; list of `{action_name: params}`), `available_file_paths`, `file_system_path` |
| Observability | `save_conversation_path`, `save_conversation_path_encoding` ('utf-8'), `generate_gif` (False or path), `calculate_cost` (False), `pricing_url`, `demo_mode`, `source`, `task_id` |
| Callbacks | `register_new_step_callback(state, model_output, step)`, `register_done_callback(history)`, `register_should_stop_callback() -> Awaitable[bool]`, `register_external_agent_status_raise_error_callback` |
| Internal-ish | `injected_agent_state`, `directly_open_url` (True), `_url_shortening_limit` (25), `enable_signal_handler` (True) |
| Catch-all | **`**kwargs` accepted and silently ignored** (only occurrence of `kwargs` in `__init__` is the signature, L213). E.g. `Agent(..., context={...})` is swallowed; `agent.context` then raises `AttributeError` (#3535, 0.9.5). |

Docs "All Parameters" page omits `use_judge`, `judge_llm`, `enable_planning`, `loop_detection_*`, `message_compaction`, `extraction_schema`, `enable_signal_handler` ([docs](https://docs.browser-use.com/open-source/customize/agent/all-parameters.md)).

#### 3.2 BrowserProfile/Browser kwargs (main ones)
`headless` (env `BROWSER_USE_HEADLESS`), `cdp_url`, `headers` (for CDP websocket), `executable_path` (aliases `browser_binary_path`, `chrome_binary_path`), `user_data_dir`, `profile_directory` ('Default'), `storage_state`, `allowed_domains`, `prohibited_domains`, `block_ip_addresses`, `keep_alive` (None), `proxy: ProxySettings`, `enable_default_extensions` (True), `captcha_solver` (True), `demo_mode`, `window_size`, `viewport`, `user_agent`, `downloads_path` (aliases `downloads_dir`, `save_downloads_path`), `accept_downloads` (True), `auto_download_pdfs` (True), `cross_origin_iframes` (True), `max_iframes` (100), `minimum_wait_page_load_time` (0.25), `wait_for_network_idle_page_load_time` (0.5), `wait_between_actions` (0.1), `highlight_elements` (True), `disable_security` (False), `deterministic_rendering`, `use_cloud`/`cloud_browser`, `cloud_profile_id`, `cloud_proxy_country_code`, `record_har_path`/`record_har_mode`/`record_har_content`, `record_video_dir`/`record_video_size`/`record_video_framerate`, `traces_dir`. Many fields still carry Playwright docstrings ("kwargs that can be passed to BrowserType.launch...") although Playwright was removed in 0.6.0.

#### 3.3 Environment variables
`config.py` ([#L54-232](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/config.py#L54)) and code greps:
- `ANONYMIZED_TELEMETRY` (default **true**), `BROWSER_USE_CLOUD_SYNC` (defaults to telemetry value), `BROWSER_USE_LOGGING_LEVEL` (info), `CDP_LOGGING_LEVEL`, `BROWSER_USE_SETUP_LOGGING`, `BROWSER_USE_DEBUG_LOG_FILE`, `BROWSER_USE_INFO_LOG_FILE`
- `BROWSER_USE_HEADLESS`, `BROWSER_USE_DISABLE_EXTENSIONS`, `BROWSER_USE_CONFIG_DIR` (~/.config/browseruse), `BROWSER_USE_CONFIG_PATH`, `XDG_*`, `IN_DOCKER` (disables Chromium sandbox), `BROWSER_USE_VERSION_CHECK` (true: network version check)
- `DEFAULT_LLM`, `SKIP_LLM_API_KEY_VERIFICATION`, provider keys (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `BROWSER_USE_API_KEY`, `AI_GATEWAY_API_KEY`, `OPENROUTER_API_KEY`, ...)
- `BROWSER_USE_ACTION_TIMEOUT_S` (180 s per action), `BROWSER_USE_CDP_TIMEOUT_S`, `BROWSER_USE_CALCULATE_COST`, `BROWSER_USE_MODEL_PRICING_URL`
- **77 distinct `TIMEOUT_<EventName>` variables** (e.g. `TIMEOUT_NavigateToUrlEvent`=30, `TIMEOUT_TypeTextEvent`=60, `TIMEOUT_AgentEventBusStop`=3); 37 are documented in a table. Per-event timeouts are configurable only by env var, not by constructor.
- Beta: `BROWSER_USE_TERMINAL_BINARY`, `BUT_HOME`, `BUT_INSTALL_DIR`, `BROWSER_USE_RUST_DIRECT_INITIAL_NAVIGATION`.
- Bug: an empty boolean env var (`ANONYMIZED_TELEMETRY=`) switches the flag ON (#5837, open, 2026-09-18), because parsing is `value.lower()[:1] in 'ty1'`.
- Env read at import time: setting `os.environ["ANONYMIZED_TELEMETRY"]="false"` after `import browser_use` had no effect in early versions (#939 0.1.40, #1912 0.2.5; users' workaround was to set it before importing).

#### 3.4 Config file
`~/.config/browseruse/config.json` (DB-style, auto-created/migrated; [config.py#L292-355](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/config.py#L292)). Mostly used by CLI; old formats are replaced with a fresh file.

#### 3.5 Defaults that bite (ranked by consequence)
1. **Telemetry on, and not "anonymized" in content**: `AgentTelemetryEvent` carries `task`, `action_history`, `urls_visited`, `final_result_response`, `error_message`, judge verdict, token counts, model, CDP host; sent to PostHog EU with a persisted device id (hash of MAC node + hostname) ([telemetry/views.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/telemetry/views.py), [telemetry/service.py#L17-33](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/telemetry/service.py#L17)). The docs telemetry page now says task instructions, URLs, action traces and results may be collected. Issue #5216 (open, 2026-07-13) objects to the "anonymous usage data" wording.
2. **Judge on**: `use_judge=True` runs an extra LLM call with the same model after every `done` ([#L2277](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L2254), [#L1593](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L1593)). Past bugs: judge corrupted structured output JSON (#4103, 0.11.9, fixed by 0.12.x); judge sent screenshots even with `use_vision=False`, breaking text-only models (#3726, 0.10.1, fixed).
3. Default LLM is the vendor's paid model when `llm` is omitted.
4. Extensions downloaded from the Chrome Web Store at first launch.
5. Browser killed after `run()` unless `keep_alive=True`, even for a user-supplied browser.
6. `max_steps=500` (very high) with no token, cost or wall-clock budget. Only steps are capped; a prompt "BUDGET WARNING" is injected at 75% of steps ([#L1542](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L1542)). Requests: #230 (+8, 2025-01), #5293 (2026-07), discussion #5983 ("dollar cap").
7. Signal handlers installed and stdout `print()` in `pause()`/`resume()`, which pollutes stdout for embedders (stdio MCP breakage #2748, 0.6.0).
8. Exact-pinned dependency set (36 `==` pins) incl. ones with CVEs; #4824 (+5, open, 2026-05-12) "deal-breaker" for security compliance.

---

### 4. Core verbs and signatures

#### 4.1 Agent verbs
| Verb | Signature / behaviour | Source |
|---|---|---|
| run | `async run(max_steps=500, on_step_start=None, on_step_end=None) -> AgentHistoryList` | service.py#L2503 |
| run_sync | sync wrapper via `asyncio.run` | #L4061 |
| follow-up | `add_new_task(new_task: str) -> None` then `await run()` again; same browser if `keep_alive=True` ([docs follow-up-tasks](https://docs.browser-use.com/open-source/examples/templates/follow-up-tasks.md)). Bug: `max_steps` compared against the cumulative `state.n_steps`, so a follow-up `run(max_steps=N)` may take zero steps (#5881, open, 0.13.10). | #L997 |
| pause / resume | sync `pause()`, `resume()`; pause takes effect before the next step; prints to stdout | #L3917, #L3923 |
| stop | sync `stop()`; sets flag, loop exits before next step; `run()` returns history with "Agent stopped programmatically" | #L3931 |
| step | `async step(step_info=None)` (one LLM call + actions) | #L1035 |
| save/rerun | `save_history(path='AgentHistory.json')`, `rerun_history(...)`, `load_and_rerun(...)`, `detect_variables()` | see §5 |
| close | `async close()` | #L3978 |
| authenticate_cloud_sync | returns False, logs "Cloud sync has been removed"; docs still describe it ([observability docs](https://docs.browser-use.com/open-source/development/monitoring/observability.md)) | #L4045 |

#### 4.2 Structured output
- `Agent(output_model_schema=MyModel)`: registers a structured `done` action whose param is `MyModel`, and also **appends the JSON schema to the task text** (`_enhance_task_with_schema`).
- Retrieval: `history.structured_output` (property; `model_validate_json(final_result())`; raises pydantic `ValidationError` if the final text isn't valid JSON) or `history.get_structured_output(MyModel)` (needed after deserialization because `_output_model_schema` is a private attr not persisted) ([views.py#L915-944](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/views.py#L915)).
- Typing: `AgentHistoryList[AgentStructuredOutput]` is generic, so `history.structured_output` is typed as `MyModel | None` if the Agent was parametrized.
- Success semantics: `is_successful()` = the agent's own `success` flag on its final `done`. Bug: with `output_model_schema`, the structured `done` hid `success` and parsing defaulted it to `True`, so blocked tasks reported success (#5955, open, fix PR #5946). Issue #6031 (open, 2026-10-10) lists 8 "known-answer false-success" regression cases (403 block, disabled click, ...). Docs and `llms.txt` now warn: "is_successful is the agent-reported outcome; verify important external actions independently."
- `extract` action: built-in action using `page_extraction_llm`; per-page schema only via `extraction_schema` (0.13.7 PR #5278 stopped auto-inheriting `output_model_schema`, which "breaks it" on the vendor gateway).

#### 4.3 Page-level "actor" API (Playwright-shaped, mixes AI + raw)
Introduced in **0.7.9 (2025-09-19)** ("Actor Use" release note); docs now file it under **"Legacy > Actor"** ([legacy/actor/basics](https://docs.browser-use.com/open-source/legacy/actor/basics.md)). CDP-based, documented as "a subset" resembling Playwright.

Entry: `await browser.new_page(url=None) -> Page`, `await browser.get_current_page() -> Page | None`, `await browser.must_get_current_page() -> Page`, `await browser.get_pages()`, `await browser.close_page(page | target_id)` ([session.py#L1346-1412](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/browser/session.py#L1346)).

`Page` ([actor/page.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/actor/page.py)), constructor `Page(browser_session, target_id, session_id=None, llm=None)`:
- Navigation: `goto(url)`, `navigate(url)`, `go_back()`, `go_forward()`, `reload()`, `get_url()`, `get_title()`, `get_target_info()`
- Query: `get_elements_by_css_selector(selector) -> list[Element]` (no auto-wait), `get_element(backend_node_id) -> Element`
- AI: `get_element_by_prompt(prompt, llm=None) -> Element | None`, `must_get_element_by_prompt(prompt, llm=None) -> Element` (raises `ValueError`), `extract_content(prompt, structured_output: type[T], llm=None) -> T` (markdown of page → LLM structured output, hard-coded 120 s timeout, wraps any failure in `RuntimeError(str(e))`)
- Misc: `evaluate(page_function: str, *args) -> str` (must be arrow-function source; result always stringified), `press("Control+A")`, `hold_key(key, duration)`, `set_viewport_size(w,h)`, `screenshot(format='png', quality=None, clip=None) -> base64 str`
- `mouse` and `session_id` are **async properties** (`mouse = await page.mouse`).

`Element` ([actor/element.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/actor/element.py), 1097 lines): `click(button='left', click_count=1, modifiers=None)`, `fill(value, clear=True)`, `hover()`, `focus()`, `check()`, `select_option(values)`, `drag_to(target, ...)`, `scroll_into_view()`, `set_input_files(paths)`, `get_attribute(name)`, `get_bounding_box()`, `screenshot()`, `evaluate(fn, *args)`, `get_basic_info() -> ElementInfo`. **No `type()` or `get_text()`**, although the Add-Tools docs list them. Handles are `backend_node_id`-based (no locator re-resolution, no auto-wait, no strictness).

`Mouse`: `click(x,y,...)`, `down()`, `up()`, `move(x,y,steps=1)`, `scroll(x=0,y=0,delta_x,delta_y)`.

Docs example `Agent(browser=browser, llm=llm)` then `agent.run("Complete login...")` (also in 0.7.9 release notes) would fail in code: `task` is required by the constructor and `run()`'s first positional is `max_steps`.

Errors in actor: plain `ValueError`/`RuntimeError`; no typed exceptions.

---

### 5. Determinism and replay

- `save_history(file_path='AgentHistory.json')` → `AgentHistoryList.save_to_file(path, sensitive_data=...)` (redacts secrets; nested-list redaction bug fixed in 0.13.11 PR #5662). Loading needs the dynamic output type: `AgentHistoryList.load_from_file(path, agent.AgentOutput)`. `usage` is dropped on round-trip (#5767, open).
- `async rerun_history(history, max_retries=3, skip_failures=False, delay_between_actions=2.0, max_step_interval=45.0, summary_llm=None, ai_step_llm=None, wait_for_elements=False) -> list[ActionResult]` ([#L3103](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L3103)). Behaviour:
  - replays each recorded action, using recorded `step_interval` (capped) as delay;
  - re-resolves element indices with a 5-level matcher: `EXACT` (`element_hash`, all attributes + ax_name) → `STABLE` (`stable_hash`) → `XPATH` → `AX_NAME` (tag + accessible name) → `ATTRIBUTE` (unique attribute) ([#L3529-3660](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L3529));
  - skips "redundant retry" steps; re-opens dropdown menus if a menu item isn't found;
  - exponential backoff 5→30 s; with `skip_failures=False` raises `RuntimeError` after `max_retries`;
  - `extract` steps re-run with `ai_step_llm` (0.11.0, PR #3696), so a rerun is **not LLM-free**;
  - always ends with an LLM-generated summary (`RerunSummaryAction{summary, success, completion_status}`), so rerun needs an LLM;
  - closes the browser in `finally`.
- `load_and_rerun(history_file=None, variables: dict[str,str] | None = None, **kwargs)` ([#L3881](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L3881)). `variables` substitutes values found by `detect_variables()` (heuristic detection of emails, names... added 0.10.0, PRs #3628/#3631). Docstring says `skip_failures` defaults True, while `rerun_history` defaults False (internal inconsistency).
- No action cache (no "first run with LLM, then cached selector" mode like Stagehand). Maintainer in #763 (+11, 2025-02, "ability to rerun same thing faster", $200 bounty) described the idea: rerun with "fallback to LLM if hash of the DOM tree changes". #943 (+11) "Export a generated script". Both closed as duplicates. History dates back to 0.1.12 (2024-11-28: "re-run it without LLM").
- **Beta agent** `rerun_history` ignores the history (`_ = history`) and **re-runs the task through the LLM** with retries ([beta/service.py#L6206](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/beta/service.py#L6206)). Deterministic replay is not available on the new runtime.
- Sibling products: `workflow-use` (separate repo, 4,225 stars, "Deterministic, Self Healing Workflows (RPA 2.0)", README says "very early development"; records or generates a workflow and falls back to browser-use per failed step). Cloud "Rerunnable scripts" ([docs](https://docs.browser-use.com/cloud/agent/scripts.md)): the Cloud agent writes a Python script into a workspace, later runs reuse and self-repair it. The vendor is moving determinism toward agent-written code, not recorded selectors.

---

### 6. Extension API (Tools)

- Rename history: `Controller` → `Tools` and `controller=` → `tools=` in **0.7.0 (2025-08-27)**; `Controller = Tools` alias kept ([tools/service.py#L2327](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/service.py#L2327)); `browser_use.controller` package re-exports it.
- `Tools(exclude_actions: list[str] | None = None, output_model=None, display_files_in_done_text=True)` ([#L441](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/service.py#L441)); `tools.exclude_action(name)`.
- Decorator: `@tools.action(description: str, param_model: type[BaseModel] | None = None, domains | allowed_domains: list[str] | None = None, terminates_sequence: bool = False)` ([registry/service.py#L291](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/registry/service.py#L291)). Action name = function `__name__`. Params inferred from signature/type hints or from `param_model`.
- Injected special params, **matched by parameter name** ([registry/views.py#L157](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/registry/views.py#L157), [registry/service.py#L367-395](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/registry/service.py#L367)): `browser_session`, `cdp_client`, `page_url`, `page_extraction_llm`, `file_system`, `available_file_paths`, `has_sensitive_data` (true only for `input`), `sensitive_data` (only the built-in `input` action), `extraction_schema`, `context` (declared, but no code path supplies it in 0.13.11; always `None`). Docs warn that a wrong name like `browser: Browser` makes the tool "fail silently". Type checks on these names produce errors: `page: Page` conflict (#2475, 0.5.5); PEP 604 `BrowserSession | None` rejected (#5905, open); keyword-only params passed positionally (#5998, open).
- Built-in actions (0.13.11): `search`, `navigate`, `go_back`, `wait`, `click` (index, plus coordinates for some models), `input`, `upload_file`, `switch`, `close`, `extract`, `search_page`, `find_elements`, `find_text`, `scroll`, `send_keys`, `dropdown_options`, `select_dropdown`, `evaluate`, `screenshot` (only with `use_vision='auto'`), `save_as_pdf`, `read_file`, `write_file`, `replace_file`, `done`. Names changed over time (`go_to_url`→`navigate`, `click_element_by_index`→`click`, `input_text`→`input`, `extract_content`/`extract_structured_data`→`extract`, `search_google`→`search`); `SearchGoogleAction` alias was restored in 0.13.8 (PR #5318) after breakage. Initial actions and saved histories reference these names.
- Direct call sugar: `await tools.navigate(url=..., browser_session=...)` via `__getattr__`.
- `ActionResult` fields ([views.py#L307](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/views.py#L307)): `is_done` (False), `success` (None; validator: `success=True` requires `is_done=True`), `judgement`, `error`, `attachments`, `images`, `long_term_memory`, `extracted_content`, `include_extracted_content_only_once` (False), `metadata`, `include_in_memory` (**deprecated**). A tool may return `str` (→ `extracted_content`), `ActionResult`, or `None`. Exceptions inside a tool are caught and turned into `ActionResult(error=str(e))`. Per-action cap `BROWSER_USE_ACTION_TIMEOUT_S` (180 s).
- Failure reporting in built-ins: several built-ins returned "element not found" as a **non-error** result, so `multi_act` kept going and `max_failures` never counted it (#5361 scroll/input/find_text; #5438 click/dropdown; #5964 disabled button; all open Aug–Oct 2026).
- MCP tools: `MCPClient(...).register_to_tools(tools, tool_filter=[...], prefix='x_')` (stdio only, ~10 s connect budget, first `tools/list` page only, per docs).
- Hooks: `run(on_step_start=, on_step_end=)` receive the `Agent` (async). Constructor callbacks listed in §3.1. Event bus (`bubus`): `agent.eventbus` emits `CreateAgentSessionEvent`, `CreateAgentTaskEvent`, `CreateAgentStepEvent`, `UpdateAgentTaskEvent`, `CreateAgentOutputFileEvent` (cloud-sync shaped); browser events (`NavigateToUrlEvent`, `ScreenshotEvent`, ...) on `browser_session.event_bus`. The hooks docs show dispatching `ScreenshotEvent` directly as the supported way to screenshot from a hook. Requested: pause from inside a tool without tripping `step_timeout` (#4798, open, 16 comments).
- `AgentHistoryList` API ([views.py#L590-944](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/views.py#L590)): `final_result()`, `is_done()`, `is_successful()`, `has_errors()`, `errors()`, `judgement()`, `is_judged()`, `is_validated()`, `urls()`, `screenshot_paths()`, `screenshots()`, `action_names()`, `model_thoughts()`, `model_outputs()`, `model_actions()`, `model_actions_filtered()`, `action_history()`, `action_results()`, `extracted_content()`, `last_action()`, `number_of_steps()`, `agent_steps()`, `total_duration_seconds()`, `structured_output`, `get_structured_output()`, `save_to_file()`, `load_from_file()`, `load_from_dict()`, field `usage: UsageSummary | None`.

---

### 7. Errors and failure surfaces

- Exception classes: `browser_use.exceptions.LLMException(status_code, message)`; `llm.exceptions.ModelError` → `ModelProviderError(message, status_code=502, model)` → `ModelRateLimitError` (429), `ModelOutputTruncatedError` (400); `browser.views.BrowserError(message, short_term_memory, long_term_memory, details, event)` → `URLNotAllowedError`; `FileSystemError`, `CloudBrowserError`/`CloudBrowserAuthError`, `SandboxError`, `MissingCookieException`, beta `BetaAgentError(RuntimeError)` → `RustSdkJsonRpcError`. No common base class.
- **Most failures don't raise.** `_handle_step_error` ([#L1258](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L1258)) converts exceptions into `ActionResult(error=...)` and increments `consecutive_failures`. A closed or disconnected browser sets `stopped=True` and returns. `run()` breaks after `max_failures + final_response_after_failure` consecutive failures and **returns** history; the error is in `history.errors()`, not an exception. Max steps reached → a synthetic history item "Failed to complete task in maximum steps". `run()` re-raises only unexpected exceptions outside step handling.
- Timeouts: `llm_timeout` wraps the LLM call (with retries inside `_get_model_output_with_retry`); `step_timeout` (180 s) wraps each step; on timeout it records an error and continues. Per-action 180 s; per-event `TIMEOUT_*` env vars. No run-level deadline.
- Fallback: `fallback_llm` switches once, for the rest of the run, on 401/402/429/5xx or truncation ([#L1983](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py#L1983)).
- Cancellation: `agent.stop()` is cooperative (between steps). `KeyboardInterrupt` → returns current history. `asyncio.CancelledError` from internal bubus parent timeouts can escape `run()` in tasks nobody cancelled (#5977, open, 2026-10-02). Beta `stop()` closes the Rust SDK client process.
- Historic "silent hang" class: the top two issues by comments (#839, 41 comments; #1020, 37 comments; +14 each) were the agent looping on "Step 1" on a blank page. Root causes reported by users included exhausted OpenAI credits and model errors not surfaced ("Maybe lets add some sort of error messaging", #839). Same class: #1137, #1322, #2555, #986 (35 comments), #1286.
- Parse failures with weaker models: "Could not parse response" (#442, 37 comments; #349 19), Ollama/gpt-oss JSON (#2605, +19), Gemini custom actions (#733, +7).

---

### 8. Model configuration

- Own wrappers in `browser_use.llm` (top-level re-exports): `ChatOpenAI`, `ChatAnthropic`, `ChatGoogle`, `ChatBrowserUse`, `ChatAzureOpenAI`, `ChatAWSBedrock`, `ChatAnthropicBedrock`, `ChatGroq`, `ChatOllama`, `ChatDeepSeek`, `ChatMistral`, `ChatCerebras`, `ChatOpenRouter`, `ChatOrcaRouter`, `ChatVercel` (AI Gateway), `ChatLiteLLM`, `ChatOCIRaw`; plus `browser_use.llm.models.get_llm_by_name`.
- LangChain dropped in **0.3.3 (2025-06-26)** (PRs #2031, #2081 by gregpr07). The release note warned that Bedrock, DeepSeek and Ollama were unsupported in 0.3.3 and told those users to stay on 0.3.2; 0.4.1 was labelled "BETA ... wait for 0.4.5 ish". **No rationale is stated** in the release note, the PRs (bot summaries only) or the docs. The maintainer's reply to the breakage (#2134, +9) was "We should definitely make a migration guide"; migration-guide issue #2137 was opened the same day. Inferred motives, **unverified**: own token/cost accounting ("we have actually built this ourselves now (`token_service`)", #2134), structured-output control, dependency weight. Issue volume rose about 52% in the two weeks after 0.3.3 (67 vs 44 opened in the prior two weeks; GitHub search counts). A LangChain example adapter remains in `examples/models/langchain`; an auto-wrap PR #4069 (2026-02) was closed.
- Injection protocol ([llm/base.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/llm/base.py)): `@runtime_checkable class BaseChatModel(Protocol)` with `model: str`, `provider`, `name`, `model_name`, and `async ainvoke(messages: list[BaseMessage], output_format: type[T] | None = None, **kwargs) -> ChatInvokeCompletion[T] | ChatInvokeCompletion[str]`. `ChatInvokeCompletion{completion, thinking, redacted_thinking, usage: ChatInvokeUsage, stop_reason, stop_details}`. `ChatInvokeUsage{prompt_tokens, prompt_cached_tokens, prompt_cache_creation_tokens(_5m/_1h), prompt_image_tokens, completion_tokens, total_tokens, pricing_multiplier}`. Structural typing makes custom models easy to inject; the agent then sniffs `model` strings for behaviour (§2).
- Cost: `calculate_cost=True` → `history.usage` (`UsageSummary`), `agent.token_cost_service.get_usage_summary()`; prices fetched from a pricing URL (override `pricing_url`/env). Gaps: Ollama/DeepSeek report no usage (#5683, open); cache-creation double count (#5305, fixed); 0.7.9 users couldn't find cost on history (#3194).

---

### 9. Observability

- Logging: `logging` with a custom setup on import, emoji/ANSI-colored INFO lines per step (Eval/Memory/Next goal), handlers to stderr by default; file logs via `BROWSER_USE_DEBUG_LOG_FILE`/`BROWSER_USE_INFO_LOG_FILE`. `pause()`/`resume()` use `print()` to stdout.
- `generate_gif=True|path` (GIF of screenshots, default `agent_history.gif`); `save_conversation_path` (per-step prompt/response dumps); `save_history()` JSON.
- Laminar: optional `lmnr` integration via `@observe` decorators (no-op if absent); docs make Laminar the recommended tracing route; OpenLIT page also exists.
- BrowserProfile recording: `record_har_path` (+mode/content; CDP-based HAR watchdog; HAR capture was broken after the CDP switch, #3063, +6, fixed), `record_video_dir` (needs `[video]` extra), `traces_dir` (accepted, but no consumer found in 0.13.11 source; a Playwright leftover, **inferred**).
- Demo mode: in-browser side panel streaming logs.
- Telemetry: PostHog (§3.5).

---

### 10. Sync vs async, typing, other languages

- Async-first; only `run_sync` is sync. Hooks must be async; `register_*` callbacks accept sync or async.
- Typing: `py.typed` shipped since 0.5.6 (PR #2509, 2025-07-26). Pydantic v2 models everywhere (`AgentSettings`, `BrowserProfile`, `ActionResult`, `AgentHistoryList`); generics on `Agent[Context, Output]` and `AgentHistoryList[Output]`; `BrowserSession.__init__` typed via overloads; `BaseChatModel` is a Protocol. Weak spots: `**kwargs` on Agent, `extra='ignore'` on BrowserProfile, `Any` skill_service, name-based injection, stringly action dicts for `initial_actions`.
- Python >= 3.11 (PyPI `requires_python`).
- TypeScript: **no official TS library for the OSS agent.** npm `browser-use` is an unofficial port by `unadlib` (repo `webllm/browser-use`), latest 0.9.0 published 2026-10-08, 3,051 downloads/week. `browser-use-typescript` (jaskirat055, 1.0.15, 2025-04-02, 87/week; announced in #1263, +17, where a user hoped it "can be officially be supported"). `browser-use-node` (0.1.11, 2024-12-30, 58/week). Official `browser-use-sdk` (npm 3.11.3, 2026-09-04, maintainers sauravpanda/gregpr07, 122,895/week; PyPI 3.11.3) is a **Cloud REST client** with a different API (`client.runs.create(...)`, `browser_use_sdk.v4`). `llms.txt` states the OSS library and hosted SDK "have different APIs". PyPI downloads of `browser-use-sdk` (1.5M/week) are inflated by the OSS package pinning it as a core dependency. Go SDK was requested (#991, +9).

---

### 11. Beta Agent (`browser_use.beta`, since 0.13.0, 2026-06-08)

- Install: `uv add "browser-use[core]"` (pulls platform wheel `browser-use-core==0.13.3` with the Rust `browser-use-terminal` binary); discovery order `BROWSER_USE_TERMINAL_BINARY` → packaged → `~/.browser-use-terminal/...` → `PATH`, else `BetaAgentError` ([beta/service.py#L187](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/beta/service.py#L187)).
- Transport: spawns `browser-use-terminal sdk-server --transport stdio`, newline-delimited JSON-RPC (`agent.run_task`, `agent.run`, `agent.event`/`agent.projected_event` notifications). The Python side reconstructs an `AgentHistoryList` from the event stream (`BETA_AGENT_INTEGRATION_FEATURES.md`).
- Consumer-facing differences from the classic Agent (source reading of [beta/service.py#L4235ff](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/beta/service.py#L4235)):
  - **Same constructor signature** (a deliberate drop-in), but `use_vision` is overwritten to `True` regardless of the argument.
  - `run(max_steps=100, ...)` (classic: 500).
  - New `follow_up(task, max_steps=None, *, step_timeout=None, enqueue_timeout=None)` reuses the Rust session (`BetaAgentError` if no prior `run()`).
  - `step()`/`take_step()` = `run(max_steps=1)`; `multi_act()` serializes non-`done` actions as a follow-up instruction.
  - Callbacks fire **after** the Rust run returns, replayed from history; `register_new_step_callback` gets `model_output=None`.
  - Custom `Tools` actions: accepted (`_resolve_tools`) but `_sdk_run_params` sends no tool definitions; the Rust server "owns its tool registry". Custom Python actions therefore do not reach the Rust agent. **Inferred from source, not run.**
  - `output_model_schema`: appended to the task prompt and parsed from final text (`_task_with_schema`, `_structured_result_text`), not enforced by a `done` tool schema. A failed model turn ended as a fake-successful prose `done` (#5129, 0.13.3, auto-closed stale/not_planned).
  - `rerun_history` re-runs the task with the LLM (no deterministic replay).
  - `stop()` closes the SDK process (hard cancel) instead of a cooperative flag only.
- The ledger documents many transport workarounds: oversize JSON-RPC lines (manual chunked reader), lost final results recovered from notifications, duplicate events deduped, usage reconstruction from summed per-turn counters. Lesson: a cross-process engine boundary needs a first-class streaming event protocol with bounded frames, idempotent final result and explicit cancel. Bolting it onto an in-process API surface forced lossy reconstruction.
- Docs present it as the Python surface of "Browser Use Terminal" (TUI/CLI/Python/coding-assistant skill) ([docs](https://docs.browser-use.com/open-source/browser-use-terminal.md)). Supported Python model classes listed there: ChatOpenAI, ChatAnthropic, ChatLiteLLM, ChatDeepSeek, ChatOpenRouter.

---

### 12. Breaking-change timeline

| Version (date) | Break | Evidence / user reaction |
|---|---|---|
| 0.1.7 (2024-11-22) | Switched to Playwright (from Selenium) | release title |
| 0.1.16 (2024-12-03) | "detached controller from browser state; browser service split into browser and context" | release "Breaking changes" |
| 0.1.45 (2025-05-03) | `BrowserWindowContextSize` removed → flat `window_width/height` ("💔 BREAKING CHANGE ... sorry for the trouble!") | release note |
| **0.2.1 (2025-05-23)** | `Browser`, `BrowserConfig`, `BrowserContext`, `BrowserContextConfig` → `BrowserProfile` + `BrowserSession`; `cookies_file` → `storage_state` | #1815 (21 comments, validation errors; maintainer: "please migrate your code"), #1762 (browser not started), #1793 "breaking changes in 0.2.*", #702 (+8). Issues +8% in next 2 weeks (62 vs 57) |
| **0.3.3 (2025-06-26)** | LangChain removed → `browser_use.llm.*`; Bedrock/DeepSeek/Ollama temporarily unsupported; 0.4.1 "BETA" until ~0.4.5 | #2134 (+9), #2242 (18 comments, `"ChatOllama" object has no field "ainvoke"`), #2215 (17), #2137 migration guide. Issues +52% (67 vs 44) |
| 0.4.x–0.5.x (Jun–Jul 2025) | Action renames/cleanup ("Clean Controller" #2187); `ActionResult.include_in_memory` deprecated in favour of `long_term_memory`; mem0/`memory_config` removed ("Remove mem0" #2150) | #2475 special-param conflicts |
| **0.6.0 (2025-08-19)** | Playwright removed → `cdp-use` + `bubus` event bus/watchdogs; passing Playwright `Page/Browser/Context` objects no longer supported; `browser_pid` dropped; header forwarding to CDP websocket lost | #2730 ("none of which seem to work"), #3111 (+4, AWS AgentCore auth headers), #2819 (21 comments CDP errors), #3063 HAR broken, #2748 stdout pollution. 0.7.1 note: "such a large migration introduced lots of bugs". Issues +26% (34 vs 27). 0.5.4 note gave rationale: Node relay subprocess "frequently conflicts with long-running Python asyncio" |
| **0.7.0 (2025-08-27)** | `controller=` → `tools=`, `Controller` → `Tools`; `Browser` promoted as name; removed `planner_llm`, `memory_config`, `timezone`, `viewport_expansion`, `use_touch`, others. Class renames kept back-compat aliases; removed params did not | release note |
| 0.7.9 (2025-09-19) | Actor API added (not breaking) | release |
| 0.8.1 (2025-10-14) | "remove-default-llm-fallback"; `ChatBrowserUse` pushed in examples | release |
| 0.9.0 (2025-10-22) | `CodeAgent` ("APIs are compatible", requires vendor LLM) | release |
| 0.9.6 (2025-11-15) | `use_judge` judge added (default on); `use_vision` default True | release ("Library judge" PR #3496, "vision defaults to True") |
| 0.9.x | `Agent(context=...)` documentation removed (PR #3561) | #3535 |
| 0.11.0 (2025-12-10) | Skills (`skills=[...]`, cloud) | release |
| 0.12.0 (2026-02-26) | All deps exact-pinned (PR #4215) | #4824 (+5, open) |
| 0.12.3 (2026-03-23) | CLI 2.0 (daemon); later "CLI 3.0" | release |
| 0.12.7 (2026-05-19) | `BrowserSession.close` added as alias of `stop` | PR #4665 |
| **0.13.0 (2026-06-08)** | Rust-backed `browser_use.beta.Agent` (additive; "The existing Python agent remains unchanged") | release |
| 0.13.7 (2026-07-27) | `extract` stops inheriting `output_model_schema` (behaviour change) | PR #5278 |
| 0.13.8 (2026-08-16) | restored `SearchGoogleAction` back-compat alias | PR #5318 |
| 0.13.11 (2026-10-07) | Actor input semantics fixed + CDP primitives (PR #5889); `toolsets_for_claude` integration | release |

Pattern: big breaks shipped as minor 0.x bumps, sometimes with rc and "BETA" labels, with migration notes only in release bodies. Aliases kept for renamed classes; removed kwargs were dropped without deprecation. The stale bot (60 days stale, then 14 days to close; `.github/workflows/stale-bot.yml`) closes most threads, so only 176 of 1,713 issues are open. "closed/completed" often means auto-closed or "should be fixed now", not verified fixed. Docs lagged releases: #902 "Proposal to Synchronize Documentation with Code Releases" (+7); #2730 asked for a prominent "new users use version x" banner.

---

### 13. Pain points (ranked by evidence weight)

Weight = reactions on primary issue + number of independent reports found + comment volume. Status as of 2026-10-14.

| # | Theme | Evidence (issues, reactions/comments, dates, versions) | Status | API-design lesson |
|---|---|---|---|---|
| 1 | **Silent hangs / non-progress with no surfaced error** (blank page, "Step 1" loop, provider error swallowed) | #839 (+14, 41c, 2025-02), #1020 (+14, 37c, 2025-03), #986 (35c), #1137 (+13, 13c), #1322 (+7, 12c), #2555 (20c), #1286 (19c), #192 (29c "All connection attempts failed"); discussion #964 "Stuck in Step 1" (14 upvotes, top discussion) | closed (mostly auto/old); class persists as swallowed errors (§7) | Fail loudly: typed, early errors for auth/credit/model errors; a run must end with a terminal status and reason, never loop silently. Expose a "why stopped" enum on the result. |
| 2 | **Browser connection/profile/launch fragility** (real Chrome profile, CDP port, Chrome 136 user-data-dir policy, headless Linux) | #1520 (+10, 36c), #291 (+9, 29c), #51 (+9, 15c), #2799 (23c cookies via storage_state), #2819 (21c), #4471, #4694 (19c), #1104 (14c browser not closing) | partially addressed (profiles, `from_system_chrome`, cloud) | Make browser acquisition an explicit, typed step (launch / connect(cdpUrl, headers) / cloud) with clear errors. Own lifecycle with `async with`/`using`. Never kill a browser the caller supplied unless asked. |
| 3 | **Breaking churn across 0.x** (config classes, LLM layer, Playwright removal, renames, removed kwargs) | §12; #1815 (21c), #2134 (+9), #2242 (18c), #2215 (17c), #2730, #3111 (+4), #1793, #3535; issue spikes +52% after 0.3.3, +26% after 0.6.0 | ongoing (beta Rust runtime is the next shift) | Freeze a small public surface early; keep engine swaps (Playwright→CDP→Rust) behind it. Deprecate with warnings and aliases for one cycle. Version the history/trace format. |
| 4 | **Local/open model compatibility & output parsing** | #442 (37c), #349 (19c), #2605 (+19, 12c), #104 (+7, 18c), #733 (+7 custom actions + Gemini), #584 (+8), #238, #486, #5687 (open: endpoints ignoring `response_format`) | improving per-provider | The engine owns structured output: a strategy ladder (native JSON schema → tool-call → prompt+repair) per model, with a clear error naming the model and strategy. Don't sniff model names in core logic; use declared capabilities. |
| 5 | **False success / untrustworthy outcome** | #6031 (8 cases, open, 2026-10-10), #5955 (open), #5361, #5438, #5964 (open), #5129 (beta fake success), #4103; docs now warn about `is_successful` | open, active | Separate `done` (agent stopped), `agentReportedSuccess`, `verified` (assertion/judge) in the result type. Actions must return typed failure, never success-with-"not found" text. Offer first-class assertions (#3709 "QA Assertion Tool" +9). |
| 6 | **Telemetry & privacy defaults** | #282 (+19), #265 (+10, 15c) posthog SSL errors breaking runs; #939, #1912 can't disable (import-time env); #5216 (content not anonymous, open); #5837 empty env var turns it on (open); #2748 stdout pollution | partly fixed; default still on | Telemetry opt-in (or at minimum content-free), runtime-configurable, never on the hot path, never writing to stdout. |
| 7 | **Human-in-the-loop / pause / 2FA** | #221 (+18, 24c), #404 (+11), #4798 (16c, open: pause from tool without step_timeout), discussions #408, #412 | partial (pause/resume, 2FA example, `sensitive_data` TOTP) | First-class "await external input" primitive that suspends timeouts; pause/resume as async API without console I/O. |
| 8 | **Determinism / replay / cheaper reruns** | #763 (+11, 13c, bounty), #943 (+11), #123 (+5 "convert actions into Playwright code"), discussion #494 (7), #954, #3856 (stable selectors), #3044 (initial actions missing from history), #431 | rerun exists with hashing; beta drops it; vendor moved to Cloud scripts and workflow-use | Make "record once, replay with fallback to AI" a core contract: action cache keyed on stable selectors and a11y, versioned trace format, typed replay result. Keep it working across engine rewrites. |
| 9 | **Cost/limits control** | #804 (+4, 15c), #230 (+8 "max steps, tokens, runtime"), #878 discussion (6), #3194, #5683, #5293, discussion #5983 | steps only; cost reporting opt-in | Budgets (tokens, cost, wall-clock, steps) as first-class run options, enforced by the engine, with typed "budget exceeded" outcome. Usage on by default. |
| 10 | **Anti-bot/stealth** | #947 (+20), #81 discussion (8), #652 (+9 captcha), #356 (15c Cloudflare), HN 35mm, 2026 HN harness comments | delegated to paid cloud | Out of API scope, but keep proxy/fingerprint/humanized input pluggable. |
| 11 | **Dependency weight & exact pins / supply chain** | #4824 (+5, 10c, open), #4505 (+9, malicious litellm via CLI install script, 2026-03-24), #1066, #3731 | open | Small core, optional extras; ranges not pins; no network installs at runtime. |
| 12 | **Custom tool ergonomics** (name-based injection, type quirks) | #2475 (+2), #1244 (18c "custom functions not recognized"), #3322 (9c), #1985, #5998, #5905, #3293 | partly fixed | Typed context object passed to tools (one `ctx` parameter) instead of magic param names; validate at registration with clear errors. |
| 13 | **Downloads/files/PDF** | #499 (+8, 13c), #729 (25c), #1951 (17c), #358 (16c), #578 (15c) | fixed over time (`auto_download_pdfs`) | Downloads as a typed event/result on the page API. |
| 14 | **Element detection gaps** (iframes, dropdowns, shadow DOM) | #1700 (23c), #871 (24c), #817, #370 (+3), #4306 | improved (cross-origin iframes 0.6+) | Engine concern; expose `observe()` results with frame info. |
| 15 | **Other languages** | #1263 (+17 TS port), #991 (+9 Go), discussion #961 | no official OSS TS | Demand for TS is real; an official, API-identical TS binding is a differentiator. |

Also notable: security posture (HN launch: "The agent could access a website that takes control of your machine."; 2026 HN: an RCE report GHSA-r2x7-6hq9-qp7v unanswered for about 40 days, per the reporter, **unverified**); `sensitive_data` warns when `allowed_domains` is not set (code L540ff).

---

### 14. What users like (quotes < 15 words)

| Quote | Who / where | Date |
|---|---|---|
| "These guys are goated" | anyekwest, [HN 43174557](https://news.ycombinator.com/item?id=43174557) (Launch HN) | 2025-02-25 |
| "this is the best thing to ever happen to the RPA world" | orliesaurus, [HN 43180642](https://news.ycombinator.com/item?id=43180642) | 2025-02-26 |
| "browser-use is compatible with more models, and has (much) more github stars" | baal80spam, [HN 43174017](https://news.ycombinator.com/item?id=43174017) (vs Stagehand) | 2025-02-25 |
| "Browser-use is incredible." | joemazerino, [HN 47897927](https://news.ycombinator.com/item?id=47897927) | 2026-04-25 |
| "has been great so far" | nateb2022, [HN 49345856](https://news.ycombinator.com/item?id=49345856) | 2026-08-18 |
| "I had a 'Wow' moment" (followed by "it feels very geek to me") | upwindchange, [HN 48012946](https://news.ycombinator.com/item?id=48012946) | 2026-05-04 |
| "like how thin this harness is" | vaporaviatorlab, [HN 47927379](https://news.ycombinator.com/item?id=47927379) (Browser Harness) | 2026-04-27 |
| "This is really awesome, I was looking for it" (TS port) | bhupesh-sf, [#1263](https://github.com/browser-use/browser-use/issues/1263) | 2025-04-22 |
| "Nice thorough write up" (CDP migration post) | ipsum2, [HN 44964883](https://news.ycombinator.com/item?id=44964883) | 2025-08-20 |
| "extremely fast onboarding (3 lines of code to working browser agent)" | rhumb.dev review, via web search summary (**unverified** wording) | 2026 |

Liked attributes (synthesized): any-LLM support (incl. local), minimal hello-world (`Agent(task, llm).run()`), open source MIT, connecting to a real logged-in browser, `sensitive_data` placeholders, custom actions as plain Python functions, speed claims vs Operator.

Counter-voices: "the saddest development experience" (gitgud, [HN 42057007](https://news.ycombinator.com/item?id=42057007), 2024-11-06, on prompt-as-code); "flaky reliability on multi-step flows" (isoldex, [HN 48196527](https://news.ycombinator.com/item?id=48196527), 2026-05-19, about browser-use and Stagehand); "Talk about 'not built here' mentality." (saberience, [HN 44964070](https://news.ycombinator.com/item?id=44964070), on dropping Playwright); "none of which seem to work" (docs examples after 0.6.0, [#2730](https://github.com/browser-use/browser-use/issues/2730)); "All because of one `uvx browser-use --help` command" ([#4505](https://github.com/browser-use/browser-use/issues/4505)).

Third-party comparisons (vendor-affiliated, low weight): 2026 reviews (Skyvern, Morph, PkgPulse, Scrapfly) consistently frame Browser Use as an autonomous agent loop and Stagehand as code-first primitives (act/extract/observe) with action caching. Reddit: no substantive thread found (one r/AI_Agents snapshot, thin).

---

### 15. Design lessons for a Playwright-shaped + AI-primitive SDK (synthesis)

1. **Two layers, one session, both first-class.** Browser Use's agent is first-class and its page API ("actor") is a late, thin, now "legacy" add-on with doc drift. Users repeatedly asked to mix deterministic steps and AI (#123, HN xnx, #3856). Design `page.act/observe/extract` and `agent.run` on the same `Page` with the same element handles and the same error types.
2. **Strict config.** Reject unknown kwargs (no `**kwargs` swallow, no `extra='ignore'`). Per-call options over env-var sprawl (77 `TIMEOUT_*` vars). Env only for secrets and deployment knobs.
3. **Typed outcome, not log lines.** Result = `{status: done|max_steps|budget|cancelled|error, agentSuccess, verified, output: T, errors[], usage, trace}`. `is_done` vs `is_successful` vs `judge` confusion, and false successes, are top current issues.
4. **Errors**: one base error with subclasses (ModelError, BrowserConnectionError, TimeoutError, BudgetExceeded, Cancelled, ActionFailed); actions return typed failures; surface provider auth/credit errors immediately (the #839/#1020 class).
5. **Lifecycle**: `await using`/`async with`; an explicit owner for each browser (launched vs attached); never kill attached browsers; `close()` means close.
6. **Budgets and cancellation**: AbortSignal/CancellationToken, wall-clock, token and cost budgets enforced by the engine; pause/resume and "await human input" that suspend timeouts.
7. **Determinism as a contract**: a versioned trace format, replay with layered element matching (Browser Use's EXACT→STABLE→XPATH→AX_NAME→ATTRIBUTE ladder is a good reference) and AI fallback. It must survive engine rewrites: the Rust beta lost replay.
8. **Engine boundary**: the beta's stdio JSON-RPC needed many recovery hacks. Design the engine protocol for streaming events, bounded frames, idempotent final result and explicit cancel from day one, since TS/Python/Rust bindings all sit on it.
9. **Model layer**: a protocol-style injection (`BaseChatModel.ainvoke(messages, output_format)`) is liked and portable. Avoid hidden name-sniffing; use declared capability flags. Keep a gateway option (Browser Use added `ChatVercel`).
10. **Privacy defaults**: telemetry opt-in or content-free; no import-time side effects (logging setup, asyncio monkeypatch, network version checks, extension downloads, runtime browser install).
11. **Stability policy**: Browser Use shipped six large breaks in about 15 months under 0.x minors. An explicit deprecation window and a migration guide published with the release would have absorbed most of the issue spikes.

---

### 16. Unverified / open items
- Official rationale for removing LangChain: not found in release notes, PRs or docs (**unverified** motives in §8).
- Behaviour of beta Agent with custom `Tools`: inferred from source (no tool payload sent), not executed.
- `traces_dir` having no effect: inferred from absence of consumers in 0.13.11 source.
- RCE advisory GHSA-r2x7-6hq9-qp7v response time: single HN reporter claim.
- Third-party review wording (rhumb.dev) came through a search summary; not opened directly.
- Download counts are point-in-time (2026-10-14) and include CI and mirrors.

## 2.3 Midscene.js (web-infra-dev/midscene)

**Status:** v1.15.4 published 2026-10-14 (GitHub release), MIT, 15.3k stars, pushed daily, 119 open issues. Docs: https://midscenejs.com (LLM-readable `.md` per page, index at https://midscenejs.com/llms.txt). API reference: https://midscenejs.com/reference/index.md (fetched 2026-10-14). TypeScript only (JS SDK packages `@midscene/web`, `@midscene/android`, `@midscene/ios`, `@midscene/harmony`, `@midscene/computer`, `@midscene/test`, `@midscene/cli`; repo tree at tag v1.15.4).

### Entry point and lifecycle
- You bring your own Playwright/Puppeteer page; Midscene wraps it: `new PlaywrightAgent(page, opts)` (alias of `PlaywrightPageAgent`), `new PuppeteerAgent(page, opts)` (alias of `PuppeteerPageAgent`), `AgentOverChromeBridge` (drives your desktop Chrome through the extension), plus device agents. Source: reference §Web.
- Multi-page: `PlaywrightBrowserAgent(context, page, { autoFollowNewPage })` / `PlaywrightBrowserAgent.create(context, opts)` with `activePage`, `pages()`, `newPage()`, `setActivePage(page)`, `waitForNewPage(action?, {timeout})`; the model also gets `ListBrowserPages`, `GetBrowserPageInfo`, `SetActivePage` actions. Page agents default to `forceSameTabNavigation: true` (new tabs are forced into the current tab). Source: reference §PlaywrightBrowserAgent.
- Teardown: `await agent.destroy()` finalizes the report and releases Midscene-owned resources; it "does not close the Page, Browser, or BrowserContext"; idempotent; `.reportFile` is only final after `destroy()`. No `await using`/`Symbol.asyncDispose` documented. Source: reference §destroy, §.reportFile.
- Playwright Test integration: `base.extend<PlayWrightAiFixtureType>(PlaywrightAiFixture({...agentOpts}))` gives fixtures `ai`, `aiAct`, `aiTap`, `aiQuery`, `aiAssert`, `aiWaitFor`, ... and `agentForPage(page)`; reporter `@midscene/web/playwright-reporter` (`type: 'merged'|'separate'`). Source: https://midscenejs.com/integrate-with-playwright.md.
- Other entry points: YAML scripts (`midscene file.yaml`, legacy runner), new `@midscene/test` (`midscene-test`, YAML cases + TS "Nodes", `midscene.config.ts`, beta since v1.12), Gherkin (`agent.runGherkinScenario()`, beta since v1.10), CLIs per platform (`npx @midscene/web act --prompt ...`) used by Agent Skills. MCP servers retired in v1.10 (last version with MCP: 1.9.8). Sources: changelog §v1.10, §v1.12; https://midscenejs.com/mcp.md; https://midscenejs.com/skills.md.

### Configuration
- Model config is **environment variables first**: `MIDSCENE_MODEL_API_KEY`, `MIDSCENE_MODEL_BASE_URL`, `MIDSCENE_MODEL_NAME`, `MIDSCENE_MODEL_FAMILY` (required, decides coordinate handling), plus `_PROTOCOL` (`openai-chat`|`openai-responses`), `_TIMEOUT` (default 180000 ms), `_RETRY_COUNT` (default 1), `_RETRY_INTERVAL` (2000), `_REASONING_*`, `_RESPONSE_FORMAT`, `_HTTP_PROXY`, `_INIT_CONFIG_JSON`, `_EXTRA_BODY_JSON`; separate `MIDSCENE_INSIGHT_MODEL_*` and `MIDSCENE_PLANNING_MODEL_*` role overrides. Source: https://midscenejs.com/model-config.md.
- Code alternative: `modelConfig: Record<string, string|number>` on the agent, using the same keys; **if present, all model env vars are ignored** (no merge). Source: reference §Custom model configuration.
- Agent options object (shared): `generateReport` (default true), `persistExecutionDump`, `reportFileName`, `outputFormat` (`single-html` default | `html-and-external-assets`), `autoPrintReportMsg`, `cache`, `aiContexts` (`{ default, [apiName] }`, resolution `options.context ?? aiContexts[api] ?? aiContexts.default`, no merging), `replanningCycleLimit` (20 default; 40 UI-TARS; 100 AutoGLM), `waitForActionReady(ctx)`, `waitAfterAction` (300 ms), `onTaskStartTip`, `createOpenAIClient`, `onLLMUsage`, `screenshotShrinkFactor`, `customActions`. Source: reference §Agent construction.
- Defaults that bite: reports are on by default and written to `./midscene_run/report/` (single HTML with base64 screenshots, which grows large; #619, #871 report-merge/generation threads); `.env` in CWD is auto-loaded by the CLIs/skills (skills doc); `forceSameTabNavigation: true` silently rewrites new-tab behaviour; cache is **off** unless `cache` is set; `modelConfig` replaces rather than merges env; `MIDSCENE_PREFERRED_LANGUAGE` defaults to Chinese when the time zone is `Asia/Shanghai`, else English (reference §Runtime configuration); all run artifacts go to `MIDSCENE_RUN_DIR` (default `midscene_run`) relative to CWD.
- Renamed config keeps aliases: `OPENAI_API_KEY`→`MIDSCENE_MODEL_API_KEY`, `OPENAI_BASE_URL`→`MIDSCENE_MODEL_BASE_URL`, `MIDSCENE_USE_*` family flags → `MIDSCENE_MODEL_FAMILY` (model-config §Still-compatible configs; changelog §v1.0).

### Core verbs (exact signatures, reference/index.md at v1.15.4 docs)
| Verb | Signature | Returns |
|---|---|---|
| `aiAct` / `ai` (was `aiAction`) | `aiAct(prompt: string \| object, opts?: { cacheable?, deepThink?: 'unset'\|true\|false, deepLocate?, fileChooserAccept?, fileChooserAllowedDir?, abortSignal?, context? })` | `Promise<string \| undefined>` (plan's output text) |
| `aiTap` / `aiHover` / `aiDoubleClick` / `aiRightClick` / `aiLongPress` | `aiTap(locate: string \| object, opts?: { deepLocate?, context?, xpath?, cacheable?, fileChooserAccept? })` | `Promise<void>` |
| `aiInput` | `aiInput(locate, { value, mode?: 'replace'\|'clear'\|'typeOnly', inputStrategy?, xpath?, ... })`; legacy `aiInput(value, locate, opts)` still accepted | `Promise<void>` |
| `aiScroll`, `aiKeyboardPress`, `aiClearInput`, `aiPinch` | locate + options | `Promise<void>` |
| `aiQuery<T>` | `aiQuery<T>(dataDemand: string \| object, opts?: { domIncluded?: boolean\|'visible-only', screenshotIncluded?, context? })` | `Promise<T>` (T is unchecked) |
| `aiBoolean` / `aiNumber` / `aiString` / `aiAsk` | `(prompt, opts?)` | `Promise<boolean \| number \| string>` |
| `aiLocate` | `(locate, opts?)` | `Promise<{ rect: {left,top,width,height}; center: [x,y]; dpr? }>` |
| `aiAssert` | `(assertion, errorMsg?, opts?)` | `Promise<void>`, throws with AI reason |
| `aiWaitFor` | `(assertion, { timeoutMs = 15000, checkIntervalMs = 3000, context? })` | `Promise<void>`, throws on timeout |
| `startObserving` | `({ intervalMs=1000, maxFrames=30, watchdogMs=300000 })` → `UIObserver.stop()` → `UIObservation` with `aiQuery/aiAssert/...` over recorded frames | |
| `runYaml` | `(yaml: string)` | `Promise<{ result }>` (all `aiQuery` results) |
| `evaluateJavaScript`, `freezePageContext`/`unfreezePageContext`, `setAIContext`, `recordToReport(title?, {content?, screenshots?})`, `flushCache({cleanUnused?})`, `metrics`, `_unstableLogContent()` | | |

- **Schema passing is natural-language only.** `aiQuery` takes a string or an object whose values are prose type descriptions, e.g. `{ items: '{name: string, price: number}[]' }` or `'string[], list of task names'`. No zod/JSON Schema/pydantic input; the TS generic `T` is a cast, not validation. Source: reference §aiQuery. The docs themselves recommend `aiQuery` + plain `expect` over `aiAssert` for deterministic checks (reference §aiAssert info box).
- Prompts can carry images (`prompt: { prompt, images: [{name,url}] }`), reference §Prompt input with images.

### Determinism
- Cache (`cache: { id, strategy: 'read-write'|'read-only'|'write-only', cacheDir? }`, or `cache: false`; YAML `cache: true` uses file name; fixture `cache: true` derives id from test metadata). Files: `./midscene_run/cache/*.cache.yaml`. Two kinds: `aiAct` **plan** cache keyed by prompt text, and **locate** cache (XPath, web only) keyed by locate prompt and re-validated before use. Query/assert results are never cached. `read-only` requires manual `agent.flushCache()`; `flushCache({ cleanUnused: true })` prunes. Stale plan → falls back to live planning and does not rewrite the cache record. Source: https://midscenejs.com/caching.md.
- `xpath` option on `aiTap`/`aiInput`/`aiLocate` is tried before cache/AI (added after #760 "add xpath selector like stagehand", closed by #844, 2025-06).
- No export to deterministic Playwright code: #2240 "Export AI execution steps as reusable Playwright scripts" open since 2026-03-30 (4 reactions; users ask for "run without consuming tokens").
- v1.0 (2025-12-18) went **pure vision**: no DOM sent by default ("Removing DOM extraction cuts token usage by about 80%"), `domIncluded` is an opt-in for queries only. Source: changelog §v1.0.

### Mixing AI with the raw page
- The agent never hides Playwright: you keep `page` and call `page.locator(...)` yourself; maintainers answer selector requests with "use playwright or puppeteer page directly" (#760, 2025-05/06). `agentForPage(page)` in fixtures; `agent.evaluateJavaScript()`.
- `customActions: DeviceAction[]` extends the model's action space (web options); `integrate-with-any-interface` lets you implement `AbstractInterface` for any device.

### Agent API
- `aiAct` is the agent loop (plan→locate→act→replan). Step cap = `replanningCycleLimit`. `deepThink: true` splits planning and localization into separate calls. Cancellation by `abortSignal`. Return value is a string (no history object). File upload sandbox: `fileChooserAllowedDir` required for prompt-driven uploads (prompt-injection guard). Hooks: `onTaskStartTip`, `waitForActionReady({action:{id,name,param}, signal})`, `onLLMUsage`. Popups mid-flow: maintainers say instant actions "don't support this kind of thinking", use `aiAct` (#1551, 2025-12-05).

### Errors and failure surfaces
- Plain `Error` subclasses: `TaskExecutionError` (`code: 'TASK_EXECUTION_FAILED'`, `cause: SerializedError`, `task?`, `toJSON()`), `ServiceError` (with `dump`), internal `NonRetryableModelCallError`. Files: `packages/core/src/task-runner.ts` L569, `packages/core/src/errors.ts`, `packages/core/src/ai-model/service-caller/call-ai.ts` L152 at v1.15.4.
- Model call retry: `MIDSCENE_MODEL_RETRY_COUNT` (default 1) on HTTP error or unparsable structured response. Per-request hard timeout via `MIDSCENE_MODEL_TIMEOUT` (180 s). No per-call `timeoutMs` on instant actions; #873 asked for it (2025-06-30, closed 2026-06-05), solution is `abortSignal` on `aiAct` and env timeout.
- `waitForActionReady` warns after 5 s and never times out; throwing stops the workflow without replay.
- `aiWaitFor`/`aiAssert` throw; no partial-result object.

### Model configuration
- OpenAI-compatible HTTP only (OpenAI SDK inside), plus `codex://app-server` / "Sign in with ChatGPT" subscription mode (https://midscenejs.com/chatgpt-subscription.md). Model *family* string is mandatory because grounding coordinates differ by family (`qwen3`, `doubao-seed`, `gemini`, `gpt-5`, UI-TARS, AutoGLM...). Three intents: default, Planning, Insight, each separately configurable. Client injection: `createOpenAIClient(openai, options)` returns a wrapped OpenAI client (LangSmith/Langfuse). Source: reference, model-config.
- Cost: `agent.metrics` → `{ totalPromptTokens, totalCompletionTokens, totalTokens, totalCachedInput, totalTimeCostMs, calls, byIntent, byModel }`; `onLLMUsage(usage)` per call. Reports show token usage by model (changelog §v1.0).

### Observability
- HTML replay report per agent (timeline, screenshots, parameter markers, token summary); `persistExecutionDump` JSON; report consumption tooling since v1.7 (extract screenshots/JSON, convert to Markdown); `ReportMergingTool`; Playwright reporter; Chrome-extension Playground and Midscene Studio desktop app (v1.8); `DEBUG=midscene:ai:profile:stats|midscene:ai:call|midscene:*` debug namespaces, and logs always written under `<MIDSCENE_RUN_DIR>/log` (reference §Runtime configuration, §Debug logs); https://midscenejs.com/model-debugging-observability.md for traces and recording model calls.

### Sync/async, language parity, typing
- TS/JS only, all async. **No official Python SDK**: community ports only, listed on https://midscenejs.com/awesome-midscene.md: `Python51888/Midscene-Python` (PyPI `midscene-python` 0.1.1, single release 2025-09-03; announced in #1150, maintainers asked for English README + credit and listed it), `zypdominate/midscene-python` (PyPI `midscene` 0.1.0–0.1.5, 2026-06-28 → 2026-07-12, "bridged into Python"), `AIPythoner/pymidscene` (PyPI `pymidscene` 0.1.0–0.3.0, 2026-02-05 → 2026-04-21). Also two community Java ports. (PyPI JSON fetched 2026-10-14.)
- Typing: options are typed, but results of `aiQuery<T>` are unchecked casts; many option bags typed as `object` in docs.

### Breaking changes across majors
- v1.0 (2025-12-18): pure-vision path; renames kept as aliases with deprecation warnings: `aiAction()`→`aiAct()`, `logScreenshot()`→`recordToReport()`, `OPENAI_API_KEY`→`MIDSCENE_MODEL_API_KEY`, `OPENAI_BASE_URL`→`MIDSCENE_MODEL_BASE_URL`; MCP redesigned (changelog §v1.0). Alias chains persist: `aiActionContext`→`aiActContext`→`aiContexts.aiAct`; `cacheId`→`cache.id`; `PlaywrightAgent`→`PlaywrightPageAgent`; `deepThink` on locate APIs → `deepLocate`; `aiInput(value, locate)` → `aiInput(locate, {value})`; `Input` mode `append`→`typeOnly` (reference).
- v1.10: MCP packages removed (pin 1.9.8), replaced by Skills + CLIs.
- Pre-1.0 churn with user impact: exports-map change in 0.12.5 broke ESM Playwright users (#461, fixed 0.12.7 same day); same script passing on 0.17.3 failing on 0.19.0 with a 7B model, maintainers blamed model size (#847, 2025-06); 1.10.5 JSON-shape failures (#2831, below).

### Sample (TypeScript; only supported language)
```ts
import { chromium } from 'playwright';
import { PlaywrightAgent } from '@midscene/web/playwright';

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto('https://shop.example.com');

const agent = new PlaywrightAgent(page, {
  cache: { id: 'checkout-smoke', strategy: 'read-write' },
  aiContexts: { default: 'Prices are in EUR.' },
  onLLMUsage: (u) => console.log(u.intent, u.total_tokens),
});
try {
  await agent.aiAct('search for "headphones" and open the first result');
  await agent.aiTap('Add to cart button', { xpath: '//button[@data-test="add"]' });
  const items = await agent.aiQuery<{ name: string; price: number }[]>(
    '{name: string, price: number}[], items listed in the cart',
  );
  if (items.length !== 1) throw new Error('cart should hold one item');
  await page.getByRole('link', { name: 'Checkout' }).click(); // raw Playwright
  await agent.aiWaitFor('the checkout form is visible', { timeoutMs: 20_000 });
} finally {
  await agent.destroy();          // finalizes report; does not close the page
  console.log(agent.reportFile, agent.metrics.totalTokens);
  await browser.close();
}
```

### User evidence: pains (top issues by reactions and comments, fetched 2026-10-14)
| Rank | Pain | Reports / links | Dates / versions | Status | API-design lesson |
|---|---|---|---|---|---|
| 1 | Model connection/config confusion (OpenAI-compatible env vars, Azure, Ollama, Qwen without vision, `/v1` suffix, CORS) | ~12 threads in the top-30 by comments: #221 (25 comments), #309 (24), #186 (13), #209 (13), #268 (11), #196 (11), #211 (11), #299, #319, #340, #1278, #1060 | 2024-12 → 2025-10; 0.8–1.x | Closed by docs/renames (`MIDSCENE_MODEL_*`, `MODEL_FAMILY`, connectivity test repo) | Typed provider config with validation at construction and a `doctor`/connectivity check; fail fast with "model lacks vision" style errors rather than HTTP 4xx from the provider |
| 2 | Structured-output fragility (`response_format` schema 400s; model returns `[]` instead of `{}`) | #339 (4 reactions, 12 comments, "same" from 3+ users), #196 (11 comments), #2831 (3 reactions, open) | 2025-01 (0.10.3); 2026-07 (1.10.5) | #339 closed (model pin advice); #2831 open, workaround = upgrade + more retries | Validate model output against a real schema and repair/retry inside the engine; expose retry policy per call; let callers pass a real schema (zod/JSON Schema) instead of prose |
| 3 | Speed vs plain Playwright | #443 (11 comments: 5 s plain vs 24 s with 4 `ai()` calls, 16 s with cache); NearForm (2025-01-02): 45.8 s vs 1.9 s login test | 2025-01 → 2025-06 | Closed; cache + vision-only path | Make the deterministic path (cached selector/plan, no screenshot) the fast default and report cache hits; keep AI per-step overhead measurable |
| 4 | No deterministic export / replay without LLM | #2240 open (4 reactions, 3 comments incl. "use the AI once ... run repeatedly without tokens"), #760 (xpath like Stagehand), #633 (batch coordinates) | 2025-05 → 2026-08 | #760 fixed via `xpath` option; #2240 open | Return replayable artifacts (selectors/code) from AI calls; first-class "compile to script" |
| 5 | Viewport-only vision; elements below the fold | #179 (5 reactions; user switched to Stagehand for full-page), #957 | 2024-12 → 2025-12 | Closed (scroll actions, deepLocate) | A vision-only engine must own scrolling/search; DOM-aware locate remains needed |
| 6 | Unexpected popups break instant actions | #1551 (8 comments), #962 | 2025-12 | Closed ("use aiAct") | Offer page-level interrupt handlers (Playwright `addLocatorHandler`-style) usable with single-step verbs |
| 7 | Reports: size, merging, generation in runners | #619 (12 comments), #871 (8), #418 (10, CJK garbling) | 2025-02 → 2025-07 | Closed; `html-and-external-assets`, `ReportMergingTool` | Separate artifacts (screenshots) from the HTML index; make report finalization explicit (`destroy`) |
| 8 | Regressions across minor versions | #847 (0.17.3 ok, 0.19.0 fails), #461 (exports map), #1569/#1534 (`aiActionContext` in YAML) | 2025-03 → 2025-12 | Closed | Pin prompts/model behaviour per version; contract tests on packaging; avoid alias proliferation |

### Likes (quotes < 15 words)
- "you can set the model and UI-TARS works" (HN, 2026-05-06, https://news.ycombinator.com/item?id=48030492)
- "Write it as you think it should execute" (NearForm, 2025-01-02, https://nearform.com/digital-community/midscene-js-assessing-a-natural-language-ai-testing-tool)
- Cache "approaching native speed for repeated test executions" (aicoolies comparison, 2026-04-02, https://aicoolies.com/comparisons/midscene-js-vs-playwright; via search summary)
- Community demand signal: three independent Python ports and two Java ports exist (awesome-midscene page).

---

## 2.4 Vercel agent-browser (vercel-labs/agent-browser)

**Status:** v0.39.1 (2026-10-13), Apache-2.0, 44k stars, 867 open issues, pushed daily. Docs https://agent-browser.dev (sources `docs/content/docs/*.mdx` at tag v0.39.1); README at v0.39.1; CHANGELOG.md at v0.39.1.

### Is there a programmatic SDK?
- **No library API.** npm `agent-browser@0.39.1` has only `bin` (no `main`/`exports`/`types`) (registry JSON, 2026-10-14). The Rust crate on crates.io is the CLI (`categories: command-line-utilities`; only one version published, 0.19.0 on 2026-03-14). The former TypeScript `BrowserManager` programmatic API was **removed** from docs and public surface in 0.20.8 (2026-03-15, #821), and README now shows `execSync('agent-browser ... --json')` for programmatic use (README §Serverless).
- 0.20.0 (2026-03-14): "100% native Rust — Removed the entire Node.js/Playwright daemon" (install 710 MB → 7 MB) (CHANGELOG §0.20.0, #754).
- Wrappers that exist: `@agent-browser/sandbox` (`runAgentBrowserCommand(sandbox, ['open', url], {json})`, `withAgentBrowserSandbox`) and `@agent-browser/eve` (mounts ~20 namespaced tools like `browser__click` into an eve agent). Both shell out to the CLI. Sources: `packages/@agent-browser/sandbox/README.md`, `docs/content/docs/eve.mdx`.
- So the "API" is (a) the command grammar, (b) the `--json` envelope, (c) the MCP server (`agent-browser mcp`, added 0.28.0, 2026-06-16, #1454), (d) a stdio plugin protocol `agent-browser.plugin.v1` (0.28.0, #1452).

### Lifecycle
- Client/daemon: each CLI call talks over a socket to a per-session daemon that owns Chrome; `--session <name>` / `AGENT_BROWSER_SESSION` isolates browser, cookies, history; `session list|info --json`; `session id --scope worktree` for stable ids; `close`, `close --all`; daemon idle timeout `--idle-timeout` default 1 h. `--cdp <port>`/`--auto-connect` attach to an existing Chrome; `--pin-tab` binds a session to a tab (else commands follow the active tab); `--profile <name|path>`, `--state`, `--restore` for auth persistence; encrypted state with `AGENT_BROWSER_ENCRYPTION_KEY`. Sources: README §Sessions, §Session Persistence; sessions.mdx.
- `batch` runs many commands per process: `agent-browser batch "open https://x" "snapshot -i"` or JSON array on stdin, `--bail` to stop on first error (README §Batch Execution).

### Configuration
- `agent-browser.json` (project) over `~/.agent-browser/config.json` (user); env vars override files; flags override env; `--config`/`AGENT_BROWSER_CONFIG`; published JSON Schema `https://agent-browser.dev/schema.json` (configuration.mdx). Default action timeout 25 s (below the 30 s IPC read timeout), `AGENT_BROWSER_DEFAULT_TIMEOUT` (README §Default Timeout).
- Bites: Playwright-revision mismatch between `install` and `open` (pre-0.20), Windows npm shims calling `/bin/sh`, daemon socket/port collisions with Hyper-V excluded ports (pain table).

### Command grammar as API
- Verbs: `open|goto|navigate <url>`, `snapshot [-i -c -d N -s sel -u]`, `click|dblclick|hover|focus <sel>`, `fill <sel> <text>` (clear + fill), `type <sel> <text>`, `press <key>`, `select`, `check`, `scroll`, `drag`, `upload`, `get text|url|...`, `is visible`, `find role|label|placeholder|testid <value> <action> [--name]`, `wait ...`, `eval <js> [-b|--stdin]`, `screenshot [--annotate --full --if-changed]`, `pdf`, `tab list|new|close`, `frame <ref>|main`, `dialog`, `network ...`, `record start|stop`, `read [url]` (agent-readable text/llms.txt), `diff`, `webmcp list|invoke`, `chat "<instruction>"` (built-in NL agent via Vercel AI Gateway, `--model openai/gpt-4o`). Source: README §Commands.
- Selectors: refs `@e2` from the last snapshot (recommended), CSS, `text=`, `xpath=`, semantic `find`. Refs survive for surviving DOM nodes; navigation invalidates them "without recycling IDs"; iframe content inlined one level; cross-origin iframes "silently omitted" (snapshots.mdx). Doc drift: snapshots.mdx shows `@e1 [heading] "..."` lines, selectors.mdx shows `- heading "..." [ref=e1]`.
- Determinism: refs are deterministic within a snapshot; no AI in the action path except `chat`. No action cache.

### JSON output contract and errors
- `--json` prints the daemon `Response` struct: `{ "success": bool, "data"?: any, "error"?: string, "code"?: string, "warning"?: string }` (`cli/src/connection.rs` L34-44 at v0.39.1); e.g. `{"success":true,"data":{"snapshot":"...","refs":{"e1":{"role":"heading","name":"Title"}}}}` (snapshots.mdx). `code` is set only "for select failures (e.g. `tab_gone`)". Content boundaries add `_boundary: {nonce, origin}` for prompt-injection provenance (`cli/src/output.rs` L507ff).
- Exit codes: `exit(1)` for every failure path in `cli/src/main.rs` (40 call sites, no other non-zero code); `doctor` documents 0/1. Text mode errors go to stderr with a `✗` prefix.
- Silent-success failures reported: `click` prints `✓ Done` while the press lands elsewhere (#2083, open), `press` silently swaps the browser and "the CLI still exits 0" (#2079, open).

### Model configuration / observability
- No model in the core path (the calling agent is the brain); `chat` uses `AI_GATEWAY_API_KEY`. Observability: dashboard with live session view and chat, `record start|stop` video, tracing/profiling, `console --json`, HAR, `doctor --json`. Sources: README §Debug, CHANGELOG 0.25.0.

### Language parity
- None needed for the CLI; MCP gives typed tools (`agent_browser_open`, `agent_browser_click`, ... with `session`, `allowedDomains`, `extraArgs`) in profiles `core` (default), `network`, `state`, `debug`, `tabs`, `react`, `mobile`, `all` (README §MCP Server).

### Breaking changes
- 0.20.0 Node/Playwright daemon removed; 0.20.8 `BrowserManager` API removed; 0.22.0 `snapshot -C/--cursor` deprecated (cursor-interactive elements now default); 0.26 skill `agent-browser` renamed to `core`; `--state` fixed in 0.26 after being silently ignored "since the native Rust rewrite" (CHANGELOG lines 545-550, 824-839).
- User reaction: rewrite fixed many Windows/install issues (#148, #56 closed by #582), but regressions such as #1157 (tab becomes `about:blank`; users downgraded to 0.24.0; fixed 0.25.2 within a day).

### Sample (shell, the supported "language"; plus Node wrapper)
```bash
export AGENT_BROWSER_SESSION=checkout
agent-browser open https://shop.example.com
agent-browser snapshot -i --json > snap.json     # {"success":true,"data":{"snapshot":...,"refs":{...}}}
agent-browser fill @e3 "headphones" && agent-browser press Enter
agent-browser find role button click --name "Add to cart"
agent-browser batch --bail "wait --text 'Cart (1)'" "screenshot cart.png"
agent-browser close
```
```ts
import { execFileSync } from 'node:child_process';
type AbResponse<T> = { success: boolean; data?: T; error?: string; code?: string; warning?: string };
const ab = <T>(...args: string[]): T => {
  const r = JSON.parse(execFileSync('agent-browser', [...args, '--json'], { encoding: 'utf8' })) as AbResponse<T>;
  if (!r.success) throw new Error(`${r.code ?? 'error'}: ${r.error}`);
  return r.data as T;
};
ab('open', 'https://example.com');
const { refs } = ab<{ snapshot: string; refs: Record<string, { role: string; name: string }> }>('snapshot', '-i');
```
(Note: with non-zero exit `execFileSync` throws before parsing; a real wrapper must read stdout from the thrown error.)

### User evidence: pains (fetched 2026-10-14)
| Rank | Pain | Reports / links | Dates / versions | Status | API-design lesson |
|---|---|---|---|---|---|
| 1 | Install/daemon start failures, esp. Windows (shims call `/bin/sh`, daemon socket/port, Defender quarantine) | #148 (15 reactions), #262 (10), #108 (11 comments), #56 (22 comments), #390 (22), #25 (19), #37, #132, #382, #1407 (hangs under PowerShell capture, still on 0.37.1) | 2026-01 → 2026-09; 0.5–0.37 | Mostly closed by #582/#614; #1407, #382, #132 open | A daemon architecture multiplies platform failure modes; ship an in-process library path and a `doctor` from day one |
| 2 | Browser binary/revision mismatch | #107 (15 reactions), #244 (13, open), #483 (12 comments), #39 | 2026-01 → 2026-09 | #107 fixed; #244 open | Own the browser resolution contract; report the exact path tried |
| 3 | Login-state reuse (profiles, user-data-dir, extension attach, focus stealing) | #42 (13), #207 (9), #297 (12 comments), #1319 (6/9), #1247 (7/6) | 2026-01 → 2026-10 | #42/#207/#1247/#1319 open; community proxies built | Make "attach to my real browser without stealing focus" first-class (cf. Playwright MCP extension) |
| 4 | Silent wrong success / lost browser | #2083 (click `✓ Done` but misses), #2079 (browser swapped, exit 0), #1157 (tab becomes about:blank, downgrade), #386 (`Browser not launched`) | 2026-02 → 2026-10; 0.24–0.37 | #1157 fixed 0.25.2; others open | Actions must verify effects (hit-target check, navigation tracking) and return typed errors, never success on uncertainty |
| 5 | Iframes / cross-origin frames | #279 (12, open), #50, #318 | 2026-01 → 2026-09 | Partially (frame refs one level; #834) | Frame-aware refs and locators are table stakes |
| 6 | Bot detection / stealth | #120 (15 reactions, 14 comments), #145 (Camoufox) | 2026-01 → 2026-09 | Open; users route via CDP to other browsers | Pluggable browser/provider seam |
| 7 | Orphaned Chrome processes, CPU burn | #1371 (5/10), #1113 | 2026-05 → 2026-09 | Partially fixed (#1527 stderr drain) | Process ownership and cleanup guarantees in the engine |
| 8 | Input semantics (`fill` on contenteditable/React, `type` prepends) | #2081 | 2026-10 | Open | Match Playwright `fill` semantics exactly; test against frameworks |
| 9 | Missing features: video (#66, 12 reactions; now `record`), element screenshot (#587, 8), network bodies (#111) | | 2026-01 → 2026-03 | Mixed | |

### Likes
- "agent-browser works fine, and it can use browser use as a provider" (HN 2026-08-18, https://news.ycombinator.com/item?id=49347126)
- "Pi with ... plus agent-browser CLI. I don't miss anything" (HN 2026-10-01, https://news.ycombinator.com/item?id=49928094)
- Persistent session use in a QA product: "agents have a persistent Chromium session they use via the agent-browser CLI" (HN 2026-08-03, https://news.ycombinator.com/item?id=49160592)
- Counterpoint: "watch it execute commands one by one, it's really inefficient" (HN 2026-05-30, https://news.ycombinator.com/item?id=48337954); "agent-browser is CLI-only" (HN 2026-09-21, https://news.ycombinator.com/item?id=49784241).
- Token efficiency: CLI tools generally beat MCP on tokens; one benchmark cites ~114k vs ~27k tokens for Playwright MCP vs Playwright CLI (bug0 blog, https://bug0.com/blog/playwright-cli-vs-playwright-mcp-ai-browser-testing-2026, via search summary); a neutral harness exists (https://github.com/nsosio/web-agent-bench) comparing Playwright MCP, agent-browser, browser-use, Stagehand with one model.

## 2.5 Skyvern (Skyvern-AI/skyvern)

_Numbered subsections and "§n" references inside this library section are local to it._

Researched 2026-10-14. Subject: Skyvern-AI/skyvern **v1.0.56** (GitHub release 2026-10-14T04:03Z; PyPI `skyvern` 1.0.56 uploaded 2026-10-14; npm `@skyvern/client` 1.0.56, `latest`), AGPL-3.0, 23,195 stars (`gh api repos/Skyvern-AI/skyvern`, 2026-10-14). Tag commit `0bfb8317bc4b4a5913d1aabcdb73f66b11c538fa`.

Citation shorthand:
- `src:<path>#L` = `https://github.com/Skyvern-AI/skyvern/blob/0bfb8317bc4b4a5913d1aabcdb73f66b11c538fa/<path>#L<n>` (tag v1.0.56).
- `doc:<slug>` = `https://www.skyvern.com/docs/<slug>` (the `.md` variant was fetched 2026-10-14; index at `doc:llms.txt`).
- `#N` = `https://github.com/Skyvern-AI/skyvern/issues/N`; `PR#N` = `.../pull/N`; `HN:<id>` = `https://news.ycombinator.com/item?id=<id>`.

### 0. Shape in one paragraph

Two layers in one package. (1) A **Fern-generated REST client** (`skyvern.client.AsyncSkyvern` / sync `skyvern.client.Skyvern`; npm `SkyvernClient`) covering runs, agents (= workflows), sessions, profiles, credentials, artifacts, schedules (`src:fern/generators.yml` pins `fernapi/fern-python-sdk` 4.31.1 and `fernapi/fern-typescript-sdk` 3.11.1). (2) A hand-written **"library" layer** `skyvern.Skyvern` (subclass of `AsyncSkyvern`, `src:skyvern/library/skyvern.py#L86`) that adds `run_task(wait_for_completion=...)`, browser launch/connect, and returns a `SkyvernBrowser` (wraps a Playwright `BrowserContext`) whose pages are `SkyvernBrowserPage` = Playwright `Page` + AI methods (`act/extract/validate/prompt/classify/locator`) + `page.agent.*` (full agent runs on the page you hold). Every AI page call is a **server round-trip** (`run_sdk_action`), not in-process inference, in cloud mode (`src:skyvern/library/skyvern_browser_page_ai.py#L30-L77`). Code caching ("run with code") is a **server-side, workflow/agent-level** feature, not an SDK page feature (`doc:developers/browser-automation` "Replay agents as cached code"; `src:skyvern/core/script_generations/CLAUDE.md`).

### 1. Entry point and lifecycle

| Step | Python | TypeScript | Source |
|---|---|---|---|
| Construct (cloud/self-host) | `Skyvern(api_key=..., environment=SkyvernEnvironment.CLOUD, base_url=None, headers=None, timeout=None, follow_redirects=True, httpx_client=None)`; `api_key` is **required keyword, no env fallback** | `new Skyvern({ apiKey, environment?, baseUrl?, timeoutInSeconds? (60), maxRetries? (2), headers? })` | `src:skyvern/library/skyvern.py#L144-L184`; `doc:sdk-reference/overview` |
| Construct (embedded local) | `Skyvern.local(llm_config=None, settings=None, use_in_memory_db=None)`: reads `.env`, boots the server **in-process**, wires an httpx transport to it | none ("TypeScript requires a running Skyvern server") | `src:skyvern/library/skyvern.py#L186-L335`; `doc:sdk-reference/overview` "Local mode" |
| Browser | `launch_cloud_browser(timeout=<minutes 5-240, default 60>, proxy_location, extensions, browser_profile_id, browser_type)`, `use_cloud_browser()` (reuses most recent idle session or creates), `connect_to_cloud_browser_session(id)`, `connect_to_browser_over_cdp(url)`, `launch_local_browser(headless=False, port, args, user_data_dir)` (Python only) | `launchCloudBrowser({timeout, proxyLocation})`, `useCloudBrowser`, `connectToCloudBrowserSession`, `connectToBrowserOverCdp` | `doc:sdk-reference/browser-automation/browsers`; `src:skyvern-ts/client/src/library/Skyvern.ts#L196-L300` |
| Page | `browser.get_working_page()` (last page or new), `browser.new_page()`, `browser.get_page_for(playwright_page)` (Py only) | `getWorkingPage()`, `newPage()`, `browser.pages()` | `src:skyvern/library/skyvern_browser.py#L120-L158` |
| Close | `browser.close(**kwargs)` (also closes the cloud session via API), `skyvern.aclose()` (stops Playwright + embedded server) | `browser.close()`, `skyvern.close()` | `src:skyvern/library/skyvern_browser.py#L159`; `src:skyvern/library/skyvern.py#L802-L816` |

- **No async context managers.** No `__aenter__/__aexit__` in `skyvern/library/*.py` at v1.0.56 (grep). Docs teach `try/finally: await browser.close()` (`doc:developers/browser-automation` "Launch a browser": "Always call it in `finally`"). TS has no `Symbol.asyncDispose` (grep of `skyvern-ts/client/src/library`).
- **Async only** in the library layer: "All methods are async" (`doc:sdk-reference/overview`). A sync Fern `skyvern.client.Skyvern` exists (`src:skyvern/client/client.py#L84`) but it is the REST client without browser helpers. **Name collision**: `skyvern.Skyvern` (async, library) vs `skyvern.client.Skyvern` (sync, generated) (`src:skyvern/__init__.py#L20-L25`, `src:skyvern/client/__init__.py`).
- **Extras gate what works.** `pip install skyvern` = REST-only; browser/page methods need `skyvern[local]` "even when the browser runs in Skyvern Cloud" (adds Playwright); local server/MCP needs `skyvern[server]` (`doc:developers/browser-automation` Note; `doc:sdk-reference/overview` Note; `src:pyproject.toml` `[project.optional-dependencies] local`, comment: local "is currently a Forge-backed compatibility bridge"). Server-only names (`SkyvernPage`, `cached`, `workflow`, ...) are lazily exported and raise "server extra required" (`src:skyvern/__init__.py#L27-L79`). TS bundles Playwright (`playwright ^1.48.0`, `src:skyvern-ts/client/package.json`).
- **Implicit infrastructure in local mode.** `Skyvern.local()` auto-detects: persistent Postgres if `DATABASE_STRING` is set anywhere, else in-memory SQLite (added 2026-03-26, PR#5260). In-memory mode lacks scheduling, cloud sessions, rate limiting, multi-worker (docstring `src:skyvern/library/skyvern.py#L250-L265`). Persistent mode raises `ValueError` without `SKYVERN_API_KEY` (`#L300-L308`). Self-host server path: `skyvern quickstart` installs Chromium, sets up DB (Postgres container needs Docker, or `--no-postgres` SQLite at `~/.skyvern/data.db`), writes a local API key to `.env` (`doc:developers/self-hosted/overview`).
- **Environment guard.** Cloud browser methods raise `ValueError("Cloud browser sessions are supported only in the cloud environment")` outside CLOUD/STAGING (`src:skyvern/library/skyvern.py#L761-L763`). Session-connect sends the API key as a CDP handshake header (`#L770-L772`).
- Windows: package import calls `setup_windows_event_loop_policy()` (`src:skyvern/__init__.py#L17`), the fix for #6494 (PR#6559, 2026-06).

### 2. Configuration style and defaults that bite

- Remote client: constructor args only; docs show `api_key=os.environ["SKYVERN_API_KEY"]` explicitly (`doc:developers/getting-started/quickstart`). Per-call `request_options=RequestOptions(timeout_in_seconds, max_retries, additional_headers, additional_query_parameters, additional_body_parameters)`; TS second arg `{timeoutInSeconds, maxRetries, headers, abortSignal, apiKey}` (`doc:sdk-reference/overview` "Request options").
- Self-host/local: **everything is an env var in `.env`** (`doc:reference/configuration` intro); `Skyvern.local(settings={...})` overrides at runtime. CLI: `skyvern quickstart` (setup + start), `skyvern init` (interactive, no start), `skyvern init browser`, `skyvern run server|mcp`, `skyvern setup claude-code|cursor|...` (`doc:reference/cli`).
- Defaults that bite (all cited):
  - `MAX_STEPS_PER_RUN` default **10**, but `.env.example` sets 50 (`doc:reference/configuration` Browser settings). A run that hits the cap ends `failed` with "Reached the maximum steps" (`doc:developers/production` Cost control).
  - `BROWSER_ACTION_TIMEOUT_MS` **5000** (vs Playwright's 30 s), used as the Python `page.click/fill` default timeout (`src:skyvern/core/script_generations/skyvern_page.py#L489-L490`). TS click passes `timeout` through as `undefined`, i.e. Playwright's default (`src:skyvern-ts/client/src/library/SkyvernBrowserPage.ts#L130-L154`): **cross-language behavioral divergence**.
  - **Unit mismatch on `timeout`**: browser `timeout` is **minutes** (5-240, default 60); `run_task`/`page.agent.*` `timeout` is **seconds** (default 1800); `hover(timeout=5000)` is **ms** (`doc:sdk-reference/browser-automation/browsers`, `.../agent`, `.../page`).
  - `wait_for_completion=False` by default: `run_task` returns immediately with a `run_id` (`doc:sdk-reference/overview` "Waiting for completion"). `page.agent.*` always waits (`src:skyvern/library/skyvern_browser_page_agent.py#L103`).
  - `use_cloud_browser()` silently **reuses the most recent idle session** (`doc:sdk-reference/browser-automation/browsers`).
  - `engine` default `skyvern_v1` (`"skyvern-1.0"`); `skyvern_v3` newest but "the published TypeScript client's type does not list it yet" (`doc:sdk-reference/runs` run_task table).
  - `LLM_KEY` defaults to `OPENAI_GPT5_5`; "Without `LLM_KEY`, Skyvern keeps calling the default OpenAI model" (`doc:developers/self-hosted/overview`).
  - `webhook_url` (on a run) vs `webhook_callback_url` (in an agent definition): wrong one "returns no error, and no webhook is ever sent" (`doc:developers/production` Warning).
  - Docker: `localhost` in `.env.example` `DATABASE_STRING` is the container itself; frontend `VITE_SKYVERN_API_KEY=YOUR_API_KEY` placeholder makes "every request fail" (`doc:developers/self-hosted/overview`).
  - Self-hosted: no CAPTCHA solving; agent waits 30 s for a human (`doc:developers/production` Note).

### 3. Core verbs (exact signatures at v1.0.56)

All on `SkyvernBrowserPage` (Python) unless noted. Signatures from `doc:sdk-reference/browser-automation/page` and checked against `src:skyvern/core/script_generations/skyvern_page.py` / `src:skyvern/library/skyvern_browser_page.py`.

| Verb | Python | TS | Returns | Notes |
|---|---|---|---|---|
| act | `act(prompt, skip_refresh=False, use_economy_tree=False)` | `act(prompt)` | `None` | "one action per call" (`doc:developers/browser-automation`). `skip_refresh`/`use_economy_tree` are **accepted but not forwarded** on the SDK HTTP path (`src:skyvern/library/skyvern_browser_page_ai.py#L239-L250` docstring). |
| extract | `extract(prompt, schema: dict\|list\|str\|None=None, error_code_mapping=None, intention=None)` | `extract({prompt, schema?, errorCodeMapping?, intention?, data?})` | `dict \| list \| str \| None` ("None if extraction fails") | Schema = **JSON Schema dict** (or a string); no pydantic/zod. Array root returns a list. TS result must be cast (`as Post[]`). |
| validate | `validate(prompt, model: dict\|str\|None=None) -> bool` | `validate(prompt, model?)` | `bool` | Server `None` result is coerced to `False` (`src:skyvern/library/skyvern_browser_page_ai.py#L211-L237`). |
| prompt | `prompt(prompt, schema=None, model=None)` | `prompt(prompt, schema?, model?)` | `dict\|list\|str\|None`; without schema `{"llm_response": "..."}` | Does not see the page (`doc:developers/browser-automation`). |
| classify (Py only) | `classify(options: dict[str,str], url_patterns=None, text_patterns=None) -> str` | - | option key or `"UNKNOWN"` | Tiered: URL regex, then text substring (free), then LLM (~$0.001). |
| click | `click(selector=None, *, prompt=None, ai="fallback", mode=None, desired_state=None, **pw_kwargs) -> str \| None` | `click(selector, opts?)`, `click({prompt,...})`, `click(selector, {prompt,...})` → `void` | selector used, or None | See Determinism. |
| fill | `fill(selector=None, value=None, *, prompt=None, ai="fallback", mode=None, totp_identifier=None, totp_url=None) -> str` | 3 overloads → `void` | value filled | |
| select_option | `select_option(selector=None, value=None, *, prompt=None, ai="fallback") -> str \| None` | `selectOption(...)` | | |
| type / hover / upload_file / keypress / scroll / drag / move / wait / download_file / reload_page | Python only | - | | `upload_file(files=<URL>)` "One file per call" (`doc:.../page`). |
| locator (Py only) | `locator(selector=None, *, prompt=None, ai="fallback", **pw_kwargs) -> Locator` | - | Playwright `Locator` or lazy `AILocator` | AILocator resolves XPath via `ai_locate_element` on first use, then **caches** it (`src:skyvern/library/ai_locator.py#L59-L100`). |
| element_fallback (Py only) | `element_fallback(navigation_goal, max_steps=5, validate_first=False)` | - | `None` | Mini-agent from current page; designed for cached-code recovery. |
| frames (Py only) | `frame_list()`, `frame_switch(selector=|name=|index=)`, `frame_main()` | use Playwright `frameLocator` | | Stateful working frame (`src:skyvern/library/skyvern_browser_page.py#L71-L132`). |
| forms (Py) | `fill_form`, `fill_multipage_form`, `fill_from_mapping`, `extract_form_fields` ... | | | `doc:sdk-reference/browser-automation/forms`; `skyvern_page.py#L1773-L2610`. |

Typed results: `TaskRunResponse` / `WorkflowRunResponse` are pydantic (Fern) with `extra="allow", frozen=True`; but `output` is `dict|list|str|None` and `errors` is `list[dict[str, Any]]` (`src:skyvern/client/types/task_run_response.py#L16-L122`). Extraction results are untyped dicts; the docs' own TS example casts (`doc:developers/browser-automation` "Extract structured data").

### 4. Determinism: selectors, fallback, caching, replay

- **Selector-first with AI fallback is the default** (`ai="fallback"`). Python `click`: wait-for-selector with retry and element prep, skip if `desired_state` already holds, click; on failure (not a post-dispatch navigation timeout) press **Escape** and retry once (to dismiss overlays), then call `ai_click` with the failed selector as context; if no `prompt`, re-raise the selector error (`src:skyvern/core/script_generations/skyvern_page.py#L494-L578`). `ai="proactive"` = AI first (`#L580-L589`). `mode="direct"` = raw Playwright, no fallback, still recorded in the timeline (`#L472-L478`). A run context can override `ai` globally (`ai_mode_override`, `#L492-L493`).
- Selector resolution uses `.first` (`#L476`, `#L511`): Playwright strict-mode errors on multiple matches are **not** surfaced.
- TS fallback is simpler: one `page.click(selector)` try, then `aiClick` (`src:skyvern-ts/client/src/library/SkyvernBrowserPage.ts#L130-L154`). No Escape retry, no `desired_state`, no `ai` mode switch.
- Docs guidance: "Both: Skyvern tries the selector first and uses the prompt only if the selector fails. This is the safest choice for production" (`doc:developers/browser-automation` "Mix selectors and AI").
- **Code caching = workflow-level** ("Adaptive Caching", changelog week of 2026-02-23, PR#4908 etc. in `doc:changelog`): after a successful agent run Skyvern generates Python code (the same `SkyvernPage` API plus `@cached`/`@workflow` decorators, `src:skyvern/__init__.py#L27-L33`) per block; later runs replay it. Per-run override `run_with="code"|"agent"`; `ai_fallback` on by default regenerates code when replay fails (`doc:developers/browser-automation` "Replay agents as cached code"; `doc:developers/production` Cost control). Conditional blocks are never cached; branches are cached progressively; `run_with: code` requires every top-level block to have a cached `script_block` + `run_signature`, else silently falls back to agent (`src:skyvern/core/script_generations/CLAUDE.md`). Clear-cache API added (changelog week of 2026-02-16, PR#4809). Cross-run extraction cache in Redis (PR#5562, 2026-04).
- `run_task` (TS type) has `run_with`, but the Python `run_task` signature does not expose it (`doc:sdk-reference/runs`; `src:skyvern/library/skyvern.py#L341-L364`): **parity gap**.
- Replay without LLM: yes for cached agent blocks; SDK page calls with a prompt always hit the server/LLM. No user-facing "record my SDK script and replay it" API found (Unverified).

### 5. Mixing AI with raw Playwright; multi-tab

- Python `SkyvernPage(Page)` subclasses Playwright `Page` and overrides `__getattribute__`: any attribute the wrapped page has and the Skyvern class hierarchy does not define is forwarded to the real page (`src:skyvern/core/script_generations/skyvern_page.py#L92-L125`). So `goto`, `wait_for_selector`, `screenshot`, etc. are real Playwright; `click/fill/select_option/type/locator` are **shadowed** with Skyvern signatures (e.g. Python `fill` loses Playwright kwargs like `force`/`no_wait_after`; it takes only `selector, value, prompt, ai, mode, totp_*`; `doc:.../page` fill table). Raw page via `page.page`.
- TS: `SkyvernBrowserPageCore` returned through a JS `Proxy`; unknown props forward to the Playwright page with `bind` (`src:skyvern-ts/client/src/library/SkyvernBrowserPage.ts#L38-L50`); type is `SkyvernBrowserPageCore & Page` (`#L400`).
- `page.agent.*` runs a full server agent **on the same browser** (passes `browser_session_id`/`browser_address`) and returns control "on the same page" (`doc:sdk-reference/browser-automation/agent`; `src:skyvern/library/skyvern_browser_page_agent.py#L80-L103`).
- Tabs share cookies/storage; `browser.new_page()`, `get_working_page()` (most recent tab), Python `get_page_for(browser.pages[-1])` to wrap a site-opened popup (`doc:developers/browser-automation` "Tabs and frames"). `BROWSER_MAX_PAGES_NUMBER=10` (`doc:reference/configuration`). Agent-side "Multi-tab agent control" shipped week of 2026-06-08 (`doc:changelog`).

### 6. Agent API

- `skyvern.run_task(prompt, engine=RunEngine.skyvern_v1, model=None, url=None, webhook_url=None, totp_identifier=None, totp_url=None, title=None, error_code_mapping=None, data_extraction_schema: dict|str|None=None, proxy_location=None, max_steps=None, wait_for_completion=False, timeout=1800, browser_session_id=None, user_agent=None, extra_http_headers=None, publish_workflow=False (deprecated), include_action_history_in_verification=None, max_screenshot_scrolls=None, browser_address=None, request_options=None) -> TaskRunResponse` (`src:skyvern/library/skyvern.py#L341-L396`; `doc:sdk-reference/runs`). TS: `runTask(request: RunTaskOptions)` single object; also `cdp_connect_headers`, `run_with`.
- `page.agent.run_task(prompt, engine, model, url (defaults to current page URL unless about:blank), webhook_url, totp_identifier, totp_url, title, error_code_mapping, data_extraction_schema, max_steps, timeout=1800, user_agent)`; TS `page.agent.runTask(prompt, options?)` (`doc:sdk-reference/browser-automation/agent`; `src:skyvern/library/skyvern_browser_page_agent.py#L42-L103,#L412`).
- `page.agent.login(credential_type="skyvern"|bitwarden|1password|azure_vault, credential_id / vault ids, url, prompt, webhook_url, totp_*, extra_http_headers, timeout) -> WorkflowRunResponse`; TS discriminated overloads `login("skyvern", {credentialId})` etc. `page.agent.download_files(prompt, url, download_suffix, download_timeout, max_steps_per_run, ...)` → `downloaded_files: [FileInfo{url (presigned), filename, checksum, file_size, modified_at}]`. `page.agent.run_workflow(workflow_id, parameters, template, title, webhook_url, totp_*, max_elapsed_time_minutes, run_metadata, timeout)` (same doc).
- `TaskRunResponse` fields: `run_id` (`tsk_`/`wr_`), `status`, `output`, `downloaded_files`, `recording_url`, `recording_archived`, `screenshot_urls` (latest first), `failure_reason`, `created_at/modified_at/queued_at/started_at/finished_at`, `app_url`, `browser_session_id`, `browser_profile_id`, `max_screenshot_scrolls`, `script_run`, `errors`, `step_count`, `run_request`; `WorkflowRunResponse` adds `run_with`, `ai_fallback`, `script_id` (`doc:sdk-reference/browser-automation/agent` accordions; `src:skyvern/client/types/task_run_response.py`).
- Status enum: `created, queued, running, paused, timed_out, failed, terminated, completed, canceled`; `failed` includes step-cap exhaustion; `terminated` = agent decided goal unreachable (`doc:sdk-reference/error-handling` "Run statuses").
- Step/time limits: `max_steps` (task), `max_steps_override` / TS `"x-max-steps-override"` (agent run), `max_elapsed_time_minutes` (default 240, max 480) (`doc:developers/production` Cost control).
- Hooks/callbacks: **no in-process step callbacks/hooks in the SDK** found (Unverified negative); integration points are `webhook_url` (one POST at terminal status, HMAC-SHA256 `x-skyvern-signature` + `x-skyvern-timestamp`, 3 attempts, 30 s), `retry_run_webhook`, agent `retry_policy`, `get_run_timeline` (`doc:developers/production` Webhooks; `doc:sdk-reference/runs`).
- Workflows renamed "Agents" (API `/v1/run/agents`, `agent_id` alias of `workflow_id`, "Fully backwards-compatible") week of 2026-06-01 (`doc:changelog`). SDK still names methods `run_workflow`, TS takes `agent_id` inside the request (`doc:developers/browser-automation` "Run tasks and agents from code").

### 7. Errors, timeouts, partial success, cancellation

- HTTP errors: Python `ApiError` base (`skyvern.client.core`) + `BadRequestError 400, ForbiddenError 403, NotFoundError 404, ConflictError 409, RangeNotSatisfiableError 416, UnprocessableEntityError 422, InternalServerError 500` (`skyvern.client.errors`); TS `SkyvernError`, `SkyvernTimeoutError`, `SkyvernApi.*Error`. Props `status_code/body/headers` (TS `statusCode/body/rawResponse/message`) (`doc:sdk-reference/error-handling`).
- **Run failure is not an exception**: `run_task(... wait_for_completion=True)` returns normally; check `status` (`doc:sdk-reference/error-handling` "Run failure vs API errors"). Docs: "Check `status` before you use `output`" (`doc:developers/browser-automation`).
- Completion timeout raises **built-in `TimeoutError`** via `asyncio.timeout` (TS: generic `Error` with "Timeout" in message, docs suggest `e.message.includes("Timeout")`) (`doc:sdk-reference/error-handling`). The wait loop polls `get_run` every 10 s and **does not cancel the server run on timeout** (`src:skyvern/library/skyvern.py#L389-L395`; `src:skyvern/library/skyvern_browser_page_agent.py#L400-L407`; `DEFAULT_AGENT_HEARTBEAT_INTERVAL = 10`, `src:skyvern/library/constants.py`). Cancellation is a separate call: `cancel_run`, `bulk_cancel_runs` (`doc:api-reference/runs/cancel-a-run-by-id`); TS `abortSignal` aborts only the HTTP request.
- Retries: HTTP-level only (TS default 2); "They do not retry the entire task" (`doc:sdk-reference/error-handling`). Agent-level `retry_policy {max_retries 1-5, delay_seconds, retry_on: [{status, error_codes}]}`; idempotency key must be `run_id`+`attempt`+`retry_pending` (`doc:developers/production`).
- Branchable failure info: user `error_code_mapping {code: description}` → `errors[].error_code` with `reasoning`, `confidence_float`; system codes `REACH_MAX_STEPS, REACH_MAX_RETRIES, OTP_ERROR, OTP_TIMEOUT, OTP_EXPIRED, MISSING_TOTP_SOURCE, CREDENTIAL_LOGIN_UNCONFIRMED, CREDENTIAL_ACCOUNT_LOCKED`; agent runs get `failure_category` list (`ANTI_BOT_DETECTION, PROXY_ERROR, AUTH_FAILURE, MAX_STEPS_EXCEEDED, BUDGET_EXHAUSTED, NAVIGATION_FAILURE, LLM_ERROR, BROWSER_SESSION_EXPIRED`, ...). `failure_reason` is "free text written by the AI. For logs, not for branching" (`doc:developers/production` Error handling).
- Page-level AI calls: `extract` returns `None` on failure; `validate` returns `False` when the server returns nothing (`src:skyvern/library/skyvern_browser_page_ai.py#L196-L237`): failure and negative answer are conflated.
- Partial success: `output` is "Still present when webhook delivery fails"; `downloaded_files` per run; `continue_on_failure` on loop blocks (`doc:developers/production`).

### 8. Model configuration and cost

- Self-host: `ENABLE_<PROVIDER>=true` + provider key + `LLM_KEY=<registry key>` e.g. `OPENAI_GPT5_5`, `ANTHROPIC_CLAUDE4.7_OPUS`, `GEMINI_3_PRO`, `BEDROCK_...`, `OLLAMA`, `OPENROUTER` (+`OPENROUTER_MODEL`), `OPENAI_COMPATIBLE` (+`OPENAI_COMPATIBLE_MODEL_NAME/_API_BASE/_SUPPORTS_VISION`, covers LiteLLM proxy/vLLM/LM Studio); `SECONDARY_LLM_KEY` for cheaper calls (some features error without it: "To enable svg shape conversion, please set the Secondary LLM key") (`doc:reference/configuration`). Not free-form `provider/model` strings: an enumerated registry of keys.
- `litellm>=1.85.0` is a dependency of the `local` extra (`src:pyproject.toml`); docstring: in-memory mode supports "Any LLM provider supported by litellm" (`src:skyvern/library/skyvern.py#L258`). `Skyvern.local(llm_config=LLMConfig(model_name="gpt-4o", required_env_vars=[...], supports_vision=True, add_assistant_prefix=False))` registers `CUSTOM_LLM` (`#L230-L244`).
- Per-call: `model: dict` on `run_task`/`page.agent.run_task`; `page.validate/prompt(model={"model_name": "gemini-2.5-flash-lite", "max_tokens": 2048} | "gpt-4")` (`doc:sdk-reference/browser-automation/page`).
- Cost: ~"30+ LLM calls" per 10-step task (`doc:reference/configuration` "How Skyvern uses LLMs"). Cloud bills per action credit (Billing v2, changelog week of 2026-01-05, PR#4398). Server stores `llm_cost, proxy_cost, captcha_cost` on task runs (PR#6219 merged 2026-05-28; changelog says "trackable via the API") but the typed `TaskRunResponse` at v1.0.56 has **no cost or token fields** (`src:skyvern/client/types/task_run_response.py`), and balance/usage API is an open request (#6226, 2026-05-28, open; billing endpoints `x-fern-ignore`d from the SDK per issue body). `step_count` is the documented cost proxy (`doc:developers/production`).

### 9. Observability

- Every run: `app_url` (live view + timeline in the web app, "Take control" for humans), `recording_url`, `screenshot_urls` (signed, 1 h) (`doc:developers/production` Payload, Anti-bot).
- `get_run_artifacts(run_id, artifact_type=[...])`: ~40 types incl. `recording, screenshot_llm/action/final, llm_prompt, llm_request, llm_response(_parsed/_rendered), visible_elements_tree(_in_prompt), html, har, trace, skyvern_log, browser_console_log, script_file, download` (`doc:sdk-reference/artifacts`; `doc:developers/troubleshooting` "Use artifacts" maps question→artifact).
- `get_run_timeline(run_id)` → thoughts/blocks (`doc:sdk-reference/runs`). Page AI calls in one SDK browser are grouped under one `workflow_run_id` that the browser carries between calls (`src:skyvern/library/skyvern_browser_page_ai.py#L64-L77`), so ad-hoc scripts show up as one run in the UI (inference from code).
- Tracing: Laminar (`Laminar.initialize()` before the client; server traces every LLM call with tokens) and OTEL (changelog week of 2026-02-09) (`doc:developers/production` Observability).
- Self-host artifacts on local disk or S3/Azure/GCS (`doc:developers/self-hosted/overview` Storage).

### 10. Sync/async, Python vs TS parity, typing

- Library layer async-only in both languages (`doc:sdk-reference/overview`).
- Parity gaps (all from `doc:sdk-reference/browser-automation/*` "Python only" notes unless cited): TS lacks `Skyvern.local`, `launch_local_browser`, `get_page_for`, `locator`/AILocator, `classify`, `element_fallback`, `type/hover/upload_file/keypress/scroll/drag/move/wait/download_file/reload_page`, frame switching, form helpers, `browser_profile_id`/`extensions`/`browser_type` on `launchCloudBrowser`, and `skyvern_v3` in its `RunEngine` type. TS click fallback logic is thinner (section 4). In the other direction, Python lacks `run_with` on `run_task` and a constructor-level `max_retries` (TS default 2) (`doc:sdk-reference/overview` table).
- **Casing inconsistency in TS**: in one object, `runTask({ prompt, url, webhook_url, error_code_mapping, waitForCompletion })`, `runWorkflow({ agent_id, run_with, "x-max-steps-override": 30, waitForCompletion })`; results are snake_case (`result.app_url`, `failure_reason`, `downloaded_files`) while `page.agent.runTask(prompt, { webhookUrl, dataExtractionSchema })` uses camelCase (`doc:developers/production` code blocks; `doc:developers/browser-automation` "In TypeScript the fields are snake_case too").
- Fern generation gives typed requests/responses, `py.typed`, `RequestOptions`, raw-response clients (`src:skyvern/client/py.typed`, `src:skyvern/client/client.py` `with_raw_response`); payload-ish fields stay `Any`. The generated client is published from a mirror (`Skyvern-AI/skyvern-python`, `Skyvern-AI/skyvern-typescript`) per `src:fern/generators.yml`, while the library layer is hand-written in the monorepo.

### 11. Breaking changes and versioning history

| Date | Change | Evidence |
|---|---|---|
| 2025-05-28 | Removed sub-clients (`client.agent.run_task` → `client.run_task`, `AgentGetRunResponse` → `GetRunResponse`) | PR#2490 |
| 2025-10-17 | Browser/page SDK prototype introduced (0.2.x) | first commit of `skyvern/library/skyvern_browser.py`, PR#3624 |
| 2025-10-31 | `SkyvernEnvironment` renamed | PR#3870 |
| 2025-11-12 | `SkyvernSdk` class removed, merged into `Skyvern` (extends Fern client) | PR#3987 |
| 2025-11-15 | **1.0.0** = version bump only ("Bumped version from 0.2.23 to 1.0.0"), no API change | PR#4003; PyPI upload 2025-11-15 |
| 2025-11-22 | `AILocator` added | PR#4027 |
| 2025-12-05 | Overloaded constructor split: remote `Skyvern(api_key=...)` required, local via `Skyvern.local()` (in 1.0.x) | PR#4206 |
| 2025-12-06 | `page.run.task()/workflow()` → `page.agent.run_task()/run_workflow()` (in 1.0.x) | PR#4215 |
| 2025-12-10 | TS library skeleton (browser/page in TS) | PR#4236 |
| 2026-06-01 | Workflows → Agents rename, backwards-compatible aliases | `doc:changelog` week of 2026-06-01 |

- Semver discipline: breaking renames shipped as 1.0.x patches within three weeks of 1.0.0 (above). A founder said in 2025-01: "We at Skyvern are still doing patch versions only" (HN:42815170). PyPI history: 84 releases, 0.1.53 (2025-02-06) → 1.0.56 (2026-10-14), roughly weekly (PyPI JSON).
- User reaction: no issue thread found that complains specifically about the renames (search for `SDK` in titles returned 5 issues, none about renames). Indirect evidence of stale guidance: a maintainer recommended `Skyvern(cdp_url=...)` in #3476 (2025-10-13), a constructor argument that does not exist at v1.0.56; #6494 (2026-06-10, v1.0.33) shows `Skyvern()` with no `api_key`, which the post-PR#4206 constructor rejects (inference: snippet may be paraphrased).

### 12. Representative code (my own, matching the documented API)

Python (`pip install "skyvern[local]"`, Python 3.11-3.14):

```python
import asyncio, os
from skyvern import Skyvern

STORIES = {
    "type": "array",
    "items": {
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "Story title as shown"},
            "points": {"type": "integer", "description": "Points, digits only"},
        },
        "required": ["title"],
    },
}

async def main() -> None:
    skyvern = Skyvern(api_key=os.environ["SKYVERN_API_KEY"])  # no env fallback
    browser = await skyvern.launch_cloud_browser(timeout=30)    # minutes
    try:
        page = await browser.get_working_page()
        await page.goto("https://news.ycombinator.com")         # plain Playwright
        if not await page.validate("The page lists Hacker News stories"):
            raise RuntimeError("unexpected page")
        top = await page.extract("The first 3 stories", schema=STORIES)  # list | None
        await page.click("a.morelink", prompt="the 'More' link at the bottom")  # selector, AI fallback
        run = await page.agent.run_task(
            "Open the first story's comments and report the top commenter",
            data_extraction_schema={"type": "object", "properties": {"author": {"type": "string"}}},
            max_steps=10,
            timeout=600,                                        # seconds
        )
        if run.status != "completed":
            print(run.status, run.failure_reason, run.app_url)
        else:
            print(top, run.output)
    finally:
        await browser.close()
        await skyvern.aclose()

asyncio.run(main())
```

TypeScript (`npm i @skyvern/client`, Node 18+):

```ts
import { Skyvern } from "@skyvern/client";

type Story = { title: string; points?: number };

const skyvern = new Skyvern({ apiKey: process.env.SKYVERN_API_KEY! });
const browser = await skyvern.launchCloudBrowser({ timeout: 30 }); // minutes
try {
  const page = await browser.getWorkingPage();
  await page.goto("https://news.ycombinator.com");
  const top = (await page.extract({
    prompt: "The first 3 stories",
    schema: { type: "array", items: { type: "object",
      properties: { title: { type: "string" }, points: { type: "integer" } }, required: ["title"] } },
  })) as Story[] | null;                                             // untyped result, cast
  await page.click("a.morelink", { prompt: "the 'More' link at the bottom" });
  const run = await page.agent.runTask("Open the first story's comments and report the top commenter", {
    dataExtractionSchema: { type: "object", properties: { author: { type: "string" } } },
    maxSteps: 10,
    timeout: 600,                                                    // seconds
  });
  console.log(run.status, run.status === "completed" ? run.output : run.failure_reason, top);
} finally {
  await browser.close();
  await skyvern.close();
}

// Fire-and-forget task on a fresh cloud browser (note mixed casing in one object):
const task = await skyvern.runTask({
  prompt: "Find the support email on the Contact page",
  url: "https://example.com",
  error_code_mapping: { not_found: "No contact email exists on the site" },
  waitForCompletion: true,
});
```

### 13. User evidence: pain points (ranked by evidence weight)

Weight = independent reporters + reactions + comment volume + recency. Sources: top issues by reactions and by comments (GitHub search API), threads read via `gh api` on 2026-10-14, plus targeted searches.

| # | Pain | Reports / links | Dates / versions | Status | API-design lesson |
|---|---|---|---|---|---|
| 1 | Self-host setup friction: Docker/DB/ports/env vars, UI "Network error", 400/404s | #312 (24 comments, "after 30 minutes of messing with docker configs"), #318 (14), #733 (17), #113 (11), #319 (10), #1061 (11), #688 (13), #1679 (10), #1063 (13), #1277 (2 reactions, 6), #1653 (10), #2992 (2 r), #4220 (5, open), #2150 (7), #147 | 2024-03 → 2025-12; v0.1.x-1.0.x | Mostly closed; #4220 open. Skyvern responded with `skyvern quickstart`, `Skyvern.local()` + in-memory SQLite (PR#5260, 2026-03) | Make the SDK usable with zero infrastructure (in-process engine, no DB/server/UI required); config errors must fail fast with the exact variable name. |
| 2 | Windows support (event loop, psycopg, Playwright subprocess, WSL Chrome) | #733, #3546 (13, 2 r), #3891 (6), #2993, #3012, #3476, #6494 (SDK direct), #5439, #4666 (open), #8638 (open), #68 (6 r) | 2024-03 → 2026-09; up to v1.0.33 | Partially fixed (PR#6559 sets loop policy on import, 2026-06); CLI gaps open | Treat Windows as tier-1 in CI; never require users to set event-loop policy; avoid async-runtime assumptions leaking into the public API. |
| 3 | LLM provider choice/config (OpenRouter, Ollama, Claude in Docker, local vision models, Azure needing OpenAI key) | #991 (8 r, top-reacted issue), #239 (3 r), #92, #180 (2 r, 5), #242 (20), #72, #76, #312, #2165, #1177, #278 | 2024-03 → 2025-04 | Closed: OpenRouter, Ollama, OpenAI-compatible, many providers added (`doc:reference/configuration`) | Model choice is the #1 requested knob: accept provider/model strings + OpenAI-compatible base URL from day one, validate at startup, allow per-call override. |
| 4 | Speed / latency of agent runs | #1271 (3 reporters: "4 minutes for the fill the form", "1 min to simply connect"), #4375 (2 r; ~5 min login+form on v1.0.2, asks for "fast mode"), #105 (maintainer: "very slow if compared with other auto-test frameworks") | 2024-03 → 2025-12; v1.0.2 | Closed by bot/staleness; answered product-wise by caching (2026-02) | Ship a deterministic fast path (selectors + cached actions) as first-class; expose per-step timing so users can see LLM vs browser time. |
| 5 | Cost visibility / spend control | #6226 (open; "no way to programmatically read an account's credit balance or token usage"), #242 ("GPT4 60 USD in 3 days"), HN:41947199 (LLM-generated script beats $0.10/page) | 2024-05 → 2026-06 | Open; cost columns in DB (PR#6219) but not in typed SDK result | Return token usage and cost on every result object and expose budget caps per call; do not hide billing data behind the UI. |
| 6 | Session continuity / concurrency / cancellation | #2779 (consecutive tasks in same tab; sessions were cloud-only), #545 (API "hangs" while a task runs), #1246 (need a stop signal that still emits webhook/ids) | 2024-07 → 2026-07 | Closed (sessions, cancel API, Temporal workers) | Make browser sessions a local first-class object; every long call needs an id up front and a cancel that propagates; client-side timeout should be able to cancel server work (Skyvern's still does not, section 7). |
| 7 | Credential store required Bitwarden org (paid) | #2604 (2 r, 12 comments; env names wrong in compose; "Bitwarden is still the only option") | 2025-06 → 2026-02 | Closed (native credentials now) | Built-in secret handling must work with no third-party vault; env var names in samples must match code. |
| 8 | False completion / wrong action with confidence | HN:42362929 ("say a task is completed, but it actually has not done the action"), #5637 (what ensures the right click; maintainer: internal confidence), HN:42365427 (restaurant booking never completed) | 2024-12 → 2026-05 | Ongoing; mitigations: `validate`, Validation blocks, `error_code_mapping`, `failure_category` | Separate "agent says done" from verified done: typed validate/assert primitives with evidence (screenshot id, element) and explicit completion criteria. |
| 9 | New windows/tabs invisible to agent | #402 (1 r; user patched handler) | 2024-06 | Closed; multi-tab agent control 2026-06 | Popups/new tabs must be observable/adoptable in the page API. |
| 10 | Navigation safety (SSRF gate bypass via page-initiated nav, history, window.open) | #8577 (open, "Severity: serious", OSS self-host) | 2026-09 | Open, community PR#8623 | Enforce navigation policy at the browser/CDP layer, not only on agent-issued actions. |
| 11 | API churn in SDK surface | PR#2490, PR#3870, PR#3987, PR#4206, PR#4215 (5 breaking renames in 7 months, two after 1.0.0); stale guidance in #3476 | 2025-05 → 2025-12 | Settled since 2025-12 | Freeze the public surface before 1.0; deprecate with aliases rather than renaming in patches. |

### 14. Likes (quotes under 15 words, with links)

- "one of the applications of LLM I find most compelling" (HN:41937245, 2024-10-24).
- "Great work! Big fan of Skyvern." (HN:44131345, 2025-05-29).
- "I'd be all over Skyvern if only they had enterprise compliance agreements" (HN:45622188, 2025-10-17; maintainer replied they have SOC 2 Type 2/HIPAA, HN:45622359).
- Code-gen direction resonated: "We had a similar realization here at Thoughtful and pivoted towards code generation" (HN:45621406, 2025-10-17).
- Mixed/negative for balance: "I tried skyvern like 6 mo ago and it didn't work" (HN:45625571, 2025-10-18); "feels too much like an ad for Skyvern" (HN:47650028, 2026-04-05).
- Users value responsive maintainers: "this is really good feedback" (maintainer, #2604, 2025-06-15) followed by native credentials shipping; user in #2604 built a Vaultwarden workaround (shows willingness to self-host).
- Docs tone that is consumer-friendly and worth copying: explicit "Run failure vs API errors" section and a question-to-artifact table (`doc:sdk-reference/error-handling`; `doc:developers/troubleshooting`).

### 15. Lessons for our SDK (synthesis)

1. Selector-first + prompt fallback on the same verb (`click("#id", prompt=...)`) is the pattern Skyvern recommends for production; specify fallback semantics identically across languages (Skyvern's TS and Python differ in timeouts and retries).
2. Do not shadow Playwright verbs with incompatible signatures/defaults (5 s vs 30 s timeouts, `.first` instead of strict, dropped kwargs); offer AI verbs as additive names or an explicit `ai` namespace.
3. One unit for `timeout` everywhere; one casing convention per language for both inputs and outputs.
4. Typed extraction: accept JSON Schema and native schema types (pydantic/zod/serde) and return typed values; distinguish "failed" from `None`/`False`.
5. Run results: typed `status` enum, typed `errors` with machine codes, token/cost fields, artifact links; failures as values for agent runs, exceptions for transport, and a cancel that the client timeout triggers.
6. Caching/replay should be reachable from the SDK level (scripts users author), not only from server-side agents.
7. Async context managers / `await using` for browser and client lifecycle.
8. Version discipline: stabilize names before 1.0.

### Unverified

- Whether any SDK-level (non-workflow) code caching or "record and replay my script" exists; only workflow/agent `run_with="code"` was found in docs and source.
- Absence of in-process step hooks/callbacks in the SDK (searched docs pages listed above; did not grep the whole Fern client for event streams).
- Whether cost fields (`llm_cost`, `proxy_cost`, `captcha_cost`) appear in the raw `get_run` JSON (they would pass through `extra="allow"`); not observed live.
- Reddit sentiment: WebSearch returned only aggregators (tooliverse.ai, everydev.ai) attributing quotes to Reddit; no reddit.com thread verified, so none quoted.
- Exact behavior of `Skyvern()` with no `api_key` reported in #6494 at v1.0.33 (likely paraphrased snippet).
- The npm README content for `@skyvern/client` 1.0.56 was not read (version and Playwright dependency verified from `skyvern-ts/client/package.json` and the registry).
- TS `runTask` `timeout` param semantics (seconds) inferred from `timeout ?? 1800` in `src:skyvern-ts/client/src/library/Skyvern.ts` and docs; not executed.
- Prior note "`skyvern init`" as the main setup command: verified it exists (`doc:reference/cli`) but docs now lead with `skyvern quickstart`.

## 2.6 Playwright MCP, playwright-cli and Playwright core AI APIs (Microsoft)

_Numbered subsections and "§n" references inside this library section are local to it._

Researched 2026-10-14. Primary sources only, except the "Likes" section and the token-cost blogs (marked). Versions: `@playwright/mcp` 0.0.83 (2026-09-28, tag commit `f183dad4`), `@playwright/cli` 0.1.22 (2026-09-28), `playwright` 1.64.1 (2026-10-14, tag commit `cfe490d9`), 1.64.0 (2026-10-07), 1.63.0 (2026-09-04).

Citation keys used below:
- **[MCP-RM]** = https://github.com/microsoft/playwright-mcp/blob/v0.0.83/README.md
- **[CLI-RM]** = https://github.com/microsoft/playwright-cli/blob/v0.1.22/README.md
- **[SRC:path]** = https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/tools/path. This monorepo source is what `@playwright/mcp` 0.0.83 runs: its `package.json` pins `playwright-core 1.64.0-alpha-1790635538000`.
- **[MCP-REL vX]** = https://github.com/microsoft/playwright-mcp/releases/tag/vX; **[CLI-REL vX]** = https://github.com/microsoft/playwright-cli/releases/tag/vX; **[PW-REL vX]** = https://github.com/microsoft/playwright/releases/tag/vX
- **mcp#N** = https://github.com/microsoft/playwright-mcp/issues/N; **cli#N** = https://github.com/microsoft/playwright-cli/issues/N; **pw#N** = https://github.com/microsoft/playwright/pull/N
- **README diff vA→vB**: I downloaded the README at every one of the 73 release tags and diffed the generated tool and option lists, so a tool rename is dated by the first tag where it appears.

---

### 0. Where the code lives (packaging)

- **The MCP source moved into the monorepo.** The playwright-mcp repo is now a thin shell. `cli.js` calls `require('playwright-core/lib/coreBundle').tools.decorateMCPCommand(...)` and `index.js` re-exports `tools.createConnection` (https://github.com/microsoft/playwright-mcp/blob/v0.0.83/cli.js, `/index.js`). `src/README.md` says the source is in `packages/playwright-core/src/tools/mcp`. CONTRIBUTING.md warns that "The core of the Playwright MCP was moved to the Playwright monorepo" and lists `src/tools/mcp` and `src/tools/backend` (https://github.com/microsoft/playwright-mcp/blob/v0.0.83/CONTRIBUTING.md).
- **There is no `packages/playwright-mcp` in the monorepo at v1.64.1.** The code is `packages/playwright-core/src/tools/{backend,mcp,cli-client,cli-daemon,dashboard,skills,trace,utils}` (tree listing at the v1.64.1 tag). The Chrome extension lives in `packages/extension` ([MCP-RM] "Browser Extension").
- **One backend serves both surfaces.** MCP tools and CLI commands share `BrowserBackend` and the `browserTools` list ([SRC:index.ts], [SRC:backend/browserBackend.ts], [SRC:cli-daemon/daemon.ts] `new BrowserBackend(contextConfig, browserContext, browserTools, …)`). The CLI is a socket client of a per-session daemon that calls the same tools.
- **Core bundles both since 1.62.** [PW-REL v1.62.0], "Command line & MCP": "Playwright now bundles the Playwright MCP server and `playwright-cli`, runnable via `npx playwright mcp` and `npx playwright cli`." I confirmed `program.command('cli')` and `program.command('mcp')` in `packages/playwright-core/src/cli/program.ts` at v1.62.0, v1.63.0 and v1.64.0. They are absent at v1.58 to v1.61.
- **Why the MCP repo shows 0 open issues.** Issues are still filed there but closed fast; 20 of the last 20 were closed within about 0 to 7 days (`gh api repos/microsoft/playwright-mcp/issues?state=all`). Release notes link fixes as microsoft/playwright PRs. The repo's CONTRIBUTING closes unsolicited PRs and "Low-Quality AI Contributions".
- **Adoption, npm last-week downloads on 2026-10-14:** `@playwright/mcp` 6.40M, `@playwright/cli` 1.06M, `playwright` 113.6M (api.npmjs.org). The old unscoped `playwright-cli` npm package is deprecated with the message "use @playwright/cli instead" (registry.npmjs.org).

### 1. Entry and lifecycle

- **MCP launch.** Start with `npx @playwright/mcp@latest` (stdio), or `--port 8931` for HTTP. HTTP clients use `"url": "http://localhost:8931/mcp"`; streamable HTTP became the default in v0.0.32 ([MCP-RM] "Standalone MCP server"; [MCP-REL v0.0.32]). Docker image: `mcr.microsoft.com/playwright/mcp`, headless Chromium only ([MCP-RM]).
- **Programmatic MCP:** `createConnection({ browser: { launchOptions: { headless: true } } })` then `connection.connect(transport)` ([MCP-RM] "Programmatic usage").
- **Lazy browser.** The browser starts on the first tool call; tab tools call `context.ensureTab()` ([SRC:backend/tool.ts] `defineTabTool`).
- **Headed by default** for MCP (`--headless` to opt out). The CLI is headless by default (`open --headed`) ([MCP-RM] options table; [CLI-RM] "Headed operation").
- **Idle shutdown, since v0.0.81.** Headless browsers close after 1 h without a tool call and relaunch on the next call. Headed and attached browsers are never closed unless `--idle-timeout` is set. The CLI shuts the whole session down instead of relaunching ([MCP-REL v0.0.81]; [MCP-RM] `timeouts.idle`).
- **Close semantics.** `browser_close` sets `isClose`. The backend disposes on close or on browser disconnect ([SRC:backend/browserBackend.ts] `callTool`). With `--shared-browser-context`, `browser_close` returns an error instead ([MCP-REL v0.0.81]).
- **Recovery.** v0.0.74 added auto-recover on remote browser disconnect and on page renderer crash. v0.0.79 added "Reconnect to the browser after a disconnect" ([MCP-REL v0.0.74], [MCP-REL v0.0.79]).
- **Interop since 1.59.** `browser.bind('name', { workspaceDir })` makes a launched browser attachable by `playwright-cli attach name` or `@playwright/mcp --endpoint=name` ([PW-REL v1.59.0] "Interoperability"). `--endpoint` appears in the MCP options from v0.0.70 (README diff).

### 2. Configuration

- **Precedence.** Options come from CLI flags, `PLAYWRIGHT_MCP_*` env vars (mirrored since v0.0.31) and a JSON `--config` file. `browser_get_config` (`--caps=config`) returns the "final resolved config after merging CLI options, environment variables and config file" ([MCP-RM]; [MCP-REL v0.0.31]). Since v0.0.79, malformed JSON config is reported instead of "silently falling back to INI parsing" ([MCP-REL v0.0.79]); `configIni.ts` exists ([SRC:mcp/configIni.ts]).
- **Config schema, top-level keys** ([MCP-RM] "Configuration file schema"):
  - `browser{browserName, isolated, userDataDir, launchOptions, contextOptions, cdpEndpoint, cdpHeaders, cdpTimeout, remoteEndpoint (string | ConnectOptions & {endpoint}), initPage[], initScript[]}`
  - `extension`, `server{port, host, allowedHosts}`, `capabilities[]`, `saveSession`, `webmcp`, `sharedBrowserContext`, `secrets{}`, `outputDir`, `outputMaxSize`, `console{level}`, `network{allowedOrigins, blockedOrigins}`, `testIdAttribute`
  - `timeouts{action=5000, navigation=60000, expect=5000, settle=500, idle}`
  - `imageResponses: allow|omit|only`, `filePaths: relative|absolute`, `snapshot{mode: full|none, boxes}`, `allowUnrestrictedFileAccess`, `codegen: typescript|python|java|csharp|none`
- **Flags that matter (v0.0.83)** ([MCP-RM] options table):
  - **Profile and auth:** `--isolated` keeps the profile in memory. `--user-data-dir`. `--storage-state` applies to isolated sessions. `--extension` connects to a running Chrome or Edge through the "Playwright Extension"; `--profile-dir-name` picks which profile (v0.0.81). `--cdp-endpoint`, `--cdp-header`, `--cdp-timeout`, `--endpoint`.
  - **Output and artifacts:** `--output-dir` holds auto-named files. `--output-max-size <bytes>` evicts old output files. `--save-session` writes a `session-<ts>/session.md` log of every tool call, with args and parsed results ([SRC:backend/sessionLog.ts]). `--file-paths absolute|relative`. `--image-responses allow|omit|only`. `--snapshot-mode full|none`. `--snapshot-boxes`. `--console-level`.
  - **Codegen and secrets:** `--codegen <lang>`. `--secrets <dotenv file>` replaces matching plaintext with `<secret>NAME</secret>` in responses and written files ([SRC:backend/response.ts] `redactSecrets` and `_writeFile`). [MCP-RM] calls it "a convenience and not a security feature".
  - **Timeouts:** `--timeout-action` (5000), `--timeout-navigation` (60000), `--timeout-settle` (500), `--idle-timeout`.
  - **Guardrails:** `--allowed-origins`/`--blocked-origins` ("*does not* serve as a security boundary"), `--allowed-hosts` (DNS-rebinding protection), `--allow-unrestricted-file-access`. File access is restricted to workspace roots by default since v0.0.55 ([MCP-REL v0.0.55]).
  - **Emulation and browser:** `--mobile` ("Mobile pages are usually lighter, which saves tokens"), `--device`, `--viewport-size`, `--grant-permissions`, `--init-page` (a TS file run against `page`), `--init-script`, `--no-webmcp`, `--shared-browser-context`.
- **Gone from the flag list:** `--save-trace` and `--save-video` disappear from the README at v0.0.69, replaced by `--caps=devtools` tools. `--vision` was removed from the README at v0.0.31 and survives only as a hidden legacy flag ([SRC:mcp/program.ts] `.option('--vision', 'Legacy option, use --caps=vision instead').hideHelp()`). `--output-mode` was added in v0.0.57 and removed in v0.0.79.
- **CLI config** is read from `.playwright/cli.config.json` by default, or from `--config` ([CLI-RM] "Configuration file"). **Doc drift:** the schema block in [CLI-RM] still lists `outputMode`, `saveVideo` and `codegen: 'typescript'|'none'`. Those are older than the shared implementation ([SRC:mcp/config.d.ts]).

### 3. Tools, verbs and shapes (MCP v0.0.83)

**Capability groups.** Core tools are always on; everything else needs `--caps`. Internal capabilities are `config | core | core-navigation | core-tabs | core-input | core-install | network | pdf | storage | testing | vision | devtools` ([SRC:backend/tool.ts] `ToolCapability`). `filteredTools` keeps `capability.startsWith('core')`, any capability in `config.capabilities`, and drops `skillOnly` tools ([SRC:backend/tools.ts]). The `--caps` help string still lists only "vision, pdf, devtools", but the README documents `--caps=config|network|storage|testing` groups too ([MCP-RM] Tools section).

Every tool below is quoted from [MCP-RM] "Tools".

- **core, 25 tools, all prefixed `browser_`:**
  - `click(element?, target, doubleClick?, button?, modifiers?)`
  - `close()`
  - `console_messages(level, all?, filename?)`
  - `drag(startElement?, startTarget, endElement?, endTarget)`
  - `drop(element?, target, paths?, data?)`
  - `emulate_media(colorScheme?, reducedMotion?, forcedColors?, contrast?, media?)`
  - `evaluate(element?, target?, function, filename?)`
  - `file_upload(paths?)`
  - `fill_form(fields[{name, type: textbox|checkbox|radio|combobox|slider, target, value}])`
  - `find(text? | regex?, filename?)`
  - `handle_dialog(accept, promptText?)`
  - `hover(element?, target)`
  - `navigate(url)`, `navigate_back()`
  - `network_request(index, part?, filename?)`, `network_requests(static, filter?, filename?)`
  - `press_key(key)`
  - `resize(width, height)`
  - `run_code_unsafe(code? | filename?)`
  - `select_option(element?, target, values[])`
  - `snapshot(target?, filename?, depth?, boxes?)`
  - `take_screenshot(element?, target?, type?, filename?, fullPage?, scale)`
  - `type(element?, target, text, submit?, slowly?)`
  - `wait_for(time? ≤30 s, text?, textGone?)`
- **core-tabs:** `browser_tabs(action: list|new|close|select, index?, url?)`. The v1.64.1 source adds `isolatedContext?` for `new` ([SRC:backend/tabs.ts]; pw#43087, 2026-10-03). It is not yet in the 0.0.83 README.
- **config:** `browser_get_config`.
- **network:** `browser_network_state_set`, `browser_route(pattern, status?, body?, contentType?, headers?, removeHeaders?)`, `browser_route_list`, `browser_unroute`.
- **storage:** `browser_cookie_{clear,delete,get,list,set}`, `browser_localstorage_{clear,delete,get,list,set}`, `browser_sessionstorage_{…}`, `browser_set_storage_state(filename)`, `browser_storage_state(filename?)`.
- **devtools:**
  - `browser_annotate` opens a dashboard where the human draws annotations.
  - `browser_highlight`, `browser_hide_highlight`.
  - `browser_resume(step?, location?)` is the debugger.
  - `browser_start_recording`, `browser_stop_recording` capture human actions as Playwright code.
  - `browser_start_tracing`, `browser_stop_tracing`.
  - `browser_start_video(filename?, size?, fps?, cursor?)`, `browser_stop_video`, `browser_video_chapter`, `browser_video_show_actions`, `browser_video_hide_actions`.
- **vision:** `browser_mouse_click_xy(x, y, button?, clickCount?, delay?)`, `browser_mouse_move_xy`, `browser_mouse_drag_xy`, `browser_mouse_down`, `browser_mouse_up`, `browser_mouse_wheel`.
- **pdf:** `browser_pdf_save(filename?)`.
- **testing:** `browser_generate_locator(element?, target)`, `browser_verify_element_visible(role, accessibleName)`, `browser_verify_list_visible`, `browser_verify_text_visible(text)`, `browser_verify_value(type, element, target, value)`.
- **WebMCP passthrough (v0.0.82).** Tools a page registers through WebMCP appear as `webmcp_<tool>` with the page's own schema. The list follows the current tab and the server sends `tools/list_changed`. Opt out with `--no-webmcp` ([MCP-REL v0.0.82]; [SRC:utils/mcp/server.ts] `tools: { listChanged: true }`). This replaced v0.0.81's `browser_webmcp_list` and `browser_webmcp_call`.

**Shape conventions**
- **`element` is an optional human description.** Its schema text is "Human-readable element description used to obtain permission to interact with the element". **`target` is a ref (`e12`, or `f1e3` inside iframes) "or a unique element selector"** ([SRC:backend/snapshot.ts] `elementSchema`).
  - Ref detection uses `/^(f\d+)?e\d+$/`. Anything else is parsed with `locatorOrSelectorAsSelector`, which accepts CSS or a Playwright locator string such as `getByRole('button', {name:'Submit'})` ([SRC:backend/tab.ts] `targetLocators`).
  - Plain selectors in addition to aria-refs arrived in v0.0.69 ([MCP-REL v0.0.69]). The param rename `ref`→`target` shows up in the README at v0.0.71 (README diff).
- **Every action is followed by a settle step** ([SRC:backend/utils.ts] `waitForCompletion`):
  1. Run the action, then wait `settle` ms (500 by default).
  2. If a navigation request fired, wait for `load` (10 s cap).
  3. Otherwise wait up to 5 s for the triggered document, stylesheet, script, XHR and fetch requests to finish.
- **Modal states are first-class.** A dialog or file chooser blocks other tools with `Error: Tool "X" does not handle the modal state.` The response includes a `### Modal state` line such as `- ["alert" dialog with message "Alert"]: can be handled by browser_handle_dialog` ([SRC:backend/tool.ts]; `tests/mcp/dialogs.spec.ts` at v1.64.1).
- **Response format.** One text block of Markdown sections, then optional image blocks ([SRC:backend/response.ts] `serialize/_build`). Sections, in order:
  - `### Error`, `### Result`
  - `### Ran Playwright code` (fenced in the codegen language)
  - `### Open tabs` (only when there is more than one tab), `### Page` (URL, title, non-2xx HTTP status, console error and warning counts, WebMCP tool count)
  - `### Modal state`
  - `### Snapshot`
  - `### Events` (downloads, `New console entries: <link>`)
  - `### Paused`
- **Hidden request metadata.** A hidden `_meta.json: true` returns the same sections as a JSON object, and `_meta.raw` keeps only Error, Result, WebMCP tools and Snapshot ([SRC:backend/browserBackend.ts]). The CLI's `--json` and `--raw` use these.

### 4. Determinism: snapshots, refs and codegen

- **Snapshot format.** Snapshots come from core `page.ariaSnapshot({ mode: 'ai', depth, boxes })`, or `ariaSnapshotJSON` for JSON responses ([SRC:backend/tab.ts] `captureSnapshot`).
  - Example lines: `- textbox "What needs to be done?" [ref=e5]` and `- checkbox "Toggle Todo" [ref=e10]` (https://playwright.dev/docs/getting-started-mcp).
  - After a click: `button "Submit" [active] [ref=e2]`; a page body renders as `- generic [active] [ref=e1]: Hello, world!` (`tests/mcp/click.spec.ts`, `core.spec.ts` at v1.64.1).
  - The AI mode also emits `[cursor=pointer]`, and with boxes on, `[box=x,y,w,h]` (types.d.ts `ariaSnapshotJSON` docs; [MCP-RM] `--snapshot-boxes`).
- **Ref lifetime.** Refs resolve "against the latest snapshot taken in the element's frame" (types.d.ts `getByRef`, v1.64.1). A stale ref fails with `Ref e12 not found in the current page snapshot. Try capturing new snapshot.` ([SRC:backend/tab.ts]).
- **Snapshot delivery history. This is the key design churn:**
  - **≤ v0.0.42:** full YAML snapshot inline after every action.
  - **v0.0.43 (2025-10-16), incremental diffs:** "Unchanged subtrees are collapsed as `- ref=<id> [unchanged]`" ([MCP-REL v0.0.43]; pw#37795).
  - **v0.0.51 (2025-12-08):** `--snapshot=incremental|full|none`, defaulting to incremental ([MCP-REL v0.0.51]; pw#38381).
  - **v0.0.52:** `browser_snapshot(filename)` saves the snapshot to disk ([MCP-REL v0.0.52]).
  - **2026-03-19/20, shipped in v0.0.69 (2026-03-30):** incremental diffs were removed from the API (pw#39777). `_snapshotForAI()` was merged into `ariaSnapshot({mode})` (pw#39755, pw#39759). `outputMode` was removed, and **implicit post-action snapshots now go to a file (`page-<ts>.yml`) with only a link `- [Snapshot](…)` in the response. Only an explicit `browser_snapshot` without `filename` inlines YAML** (pw#39768 body: "snapshots go to files for non-explicit mode, inline for explicit (browser_snapshot)"). The current code confirms it: `snapshotToFile = this._includeSnapshot !== 'explicit' || !!this._includeSnapshotFileName` ([SRC:backend/response.ts]). The same PR stopped inlining console messages; responses now carry a `New console entries: <link>` line.
  - **v0.0.78:** "Less verbose snapshots … distilled" (pw#41604), plus `browser_find`, a grep over the snapshot with 3 context lines ([MCP-REL v0.0.78]; [SRC:backend/find.ts] `contextLines = 3`).
  - **v0.0.79:** snapshot capture is skipped when the response discards it ([MCP-REL v0.0.79]).
- **Codegen in every response.** Each action calls `response.addAction(...)`, and the response renders `### Ran Playwright code` using the `--codegen` language ([SRC:backend/response.ts], [SRC:backend/codegen.ts]).
  - Ref targets go through `locator.describe(element).normalize()`, so the emitted code uses resilient locators such as `getByRole('button', { name: 'Submit' })` rather than refs ([SRC:backend/tab.ts]).
  - Languages: TypeScript, plus Python, Java and C# since v0.0.79. Example: `page.get_by_role("button", name="Submit").click()` (`tests/mcp/codegen.spec.ts` v1.64.1; [MCP-REL v0.0.79]).
  - The default language is inferred from `PW_LANG_NAME` since v0.0.80 ([SRC:mcp/config.ts] `defaultCodegenLanguage`; [MCP-REL v0.0.80]).
  - Secret values are substituted in the generated code (`substituteSecrets`).
- **Test-generation history:** `browser_generate_playwright_test` was added in v0.0.19 and removed in v0.0.31: "We received mixed feedback on the tool, manually prompting to generate tests after actions is still the best practice" ([MCP-REL v0.0.31]). Test Agents (§9) replaced it.

### 5. Mixing with raw Playwright

- **`browser_run_code_unsafe(code | filename)`.** `code` is "A JavaScript function … invoked with a single argument, page", for example `async (page) => { await page.getByRole('button', { name: 'Submit' }).click(); return await page.title(); }`.
  - It runs in a Node `vm` context in the **server process**. It exposes only `page` and a whitelist of globals: timers, URL, TextEncoder, AbortController, fetch, Headers, Request, Response, FormData, Blob, Buffer, crypto, atob, btoa and structuredClone ([SRC:backend/runCode.ts]).
  - The return value is `JSON.stringify`'d into `### Result`. The response's `### Ran Playwright code` is `await (<code>)(page);`.
  - Unhandled rejections from callbacks like `page.route` are reported on the response; before v0.0.72 they crashed the server ([MCP-REL v0.0.72]).
- **History:** `browser_run_code` was added in v0.0.47 ("LLM can speed up repetitive operations and save on context tokens") ([MCP-REL v0.0.47]). It started taking a function with `return` in v0.0.52 and was **renamed `browser_run_code_unsafe` in v0.0.72**, "to make the sandbox-escape implication explicit" ([MCP-REL v0.0.72]). The description now says "RCE-equivalent".
- **`browser_evaluate(function, target?, filename?)`** runs in the **page**. It accepts `() => …`, `(element) => …` or, since v0.0.71, a plain expression ([MCP-REL v0.0.71]). The result is JSON-stringified, or written to a file when `filename` is set ([SRC:backend/evaluate.ts]). Errors become `### Error` with the error message. It was added in v0.0.31 after mcp#424, the most-upvoted issue (+1=25).
- **`--init-page file.ts`** runs `export default async ({ page }) => {...}` before the session starts ([MCP-RM] "Initial state").
- **Same browser, many clients.** Since 1.59, `browser.bind()` and `--endpoint` let user code, the CLI and MCP share one browser, with "Multiple clients at a time" ([PW-REL v1.59.0]).

### 6. Errors (how the model sees failures)

- **Tool errors are content, not protocol errors.** The result is `isError: true` with text `### Error\n<message>`, or `{"isError":true,"error":…}` in JSON mode ([SRC:backend/browserBackend.ts] `formatError`; [SRC:utils/mcp/server.ts]).
  - Invalid args (since v0.0.76): `Invalid arguments for tool "X":\n<z.prettifyError>`. Before that this "fail[ed] opaquely" ([MCP-REL v0.0.76]).
  - An unknown tool returns `Tool "X" not found`.
  - A thrown handler error returns `String(error)` plus any drained unhandled rejections.
  - Errors set with `response.addError` still carry the Page and Snapshot sections, because `_build` always runs.
- **Typed messages the model sees** ([SRC:backend/tab.ts], [SRC:backend/tool.ts], [SRC:mcp/browserFactory.ts]):
  - `Ref eN not found in the current page snapshot. Try capturing new snapshot.`
  - `"<selector>" does not match any elements.`
  - Modal-state errors.
  - `Browser is already in use for <dir>, use --isolated to run multiple instances of the same browser`. This one was historically also thrown when Chrome failed to launch; see the §Pain table.
- **Navigation surfaces non-2xx** as `- HTTP status: 402 Payment Required` (v0.0.78). A dialog during load is reported instead of timing out (v0.0.83). `browser_wait_for` now reports when `time` was capped at 30 s; it used to cap silently ([MCP-REL v0.0.78], [MCP-REL v0.0.83]; mcp#1767).
- **CLI errors.** The text output exits non-zero for unknown commands and options. JSON mode emits `{isError:true, error}` ([SRC:cli-client/output.ts] `JsonOutput`). A version-skewed daemon fails with an instruction to re-`open` ([SRC:cli-client/session.ts]).

### 7. Model configuration

Not applicable. MCP and the CLI are model-agnostic, and the host brings the model.
- **Exception, removed: `page.agent()` in core 1.58.** It was hidden and undocumented publicly. Options were `provider{api: openai|openai-compatible|anthropic|google, apiKey, apiEndpoint, model}`, `cache{cacheFile, cacheOutFile}`, `limits{maxTokens, maxActions=10, maxActionRetries=3}`, `secrets` and `systemPrompt`. It returned a `PageAgent` with `perform(task)`, `expect(expectation)`, `extract(query, zodSchema)`, `usage()`, `dispose()` and a `turn` event carrying token usage (https://github.com/microsoft/playwright/blob/v1.58.0/docs/src/api/class-pageagent.md, `class-page.md` §Page.agent; client `page.ts` at v1.58.0).
  - It was hidden with "Not ready for primetime!" (pw#38882, 2026-01-22) and then deleted with −6,571 lines in "chore: rip perform" (pw#39626, merged 2026-03-11). It is absent from 1.59 onward.
  - **Lesson:** Microsoft prototyped exactly act/extract/expect with a code cache, then withdrew it and pushed the intelligence back to external agents (MCP, CLI, Test Agents).

### 8. Observability

- **Traces:** `browser_start_tracing` and `browser_stop_tracing` (`--caps=devtools`) link the trace and resources dirs ([SRC:backend/tracing.ts]). `--save-trace` existed from v0.0.26 to v0.0.68.
- **Agent-facing trace CLI (1.59):** `npx playwright trace open|actions|action N|snapshot N --name after` lets agents read traces from the terminal ([PW-REL v1.59.0]). A `playwright-trace` skill ships in `tools/skills/`.
- **Video:** `--save-video=WxH` existed from v0.0.40 to v0.0.68 (added for mcp#695). It became the devtools tools `browser_start_video(fps, cursor)`, `browser_video_chapter` and `browser_video_show_actions(style)` ([MCP-REL v0.0.69], [MCP-REL v0.0.76], [MCP-REL v0.0.82]). These are backed by core `page.screencast` (1.59), marketed as "Agentic video receipts" ([PW-REL v1.59.0]).
- **Console:** recent console logs were inlined in every result from v0.0.31 ([MCP-REL v0.0.31]). That caused the ×6 token regression reported in mcp#889. Since v0.0.69 console output is a linked log file plus error and warning counts in `### Page`; full output comes from `browser_console_messages(level, all?)`.
- **Network:** `browser_network_requests` gives a numbered list, and `browser_network_request(index, part)` returns details (v0.0.72). This pattern is "stable indexes" + "pipe-friendly part extractors" ([CLI-REL v0.1.10]).
- **Session log:** `--save-session` writes `session.md` with every call's args and parsed result ([SRC:backend/sessionLog.ts]).
- **Human dashboard (1.59, CLI v0.1.1):** `playwright-cli show` tiles every bound browser with a live screencast, and a human can click in to take over ([PW-REL v1.59.0]; [CLI-RM] "Monitoring").
- **Human in the loop:** `browser_annotate`, and `browser_start_recording`/`browser_stop_recording`, which return human-demonstrated actions as code (v0.0.80).

### 9. Playwright core AI-ish APIs (1.56 to 1.64)

Status: everything below is public and documented in types.d.ts at v1.64.1 unless noted. "Since" versions come from `docs/src/api/*.md` at v1.64.1.

| API | Since | Notes |
|---|---|---|
| `locator.describe(desc)` | 1.53 | Labels the locator for traces and reports. MCP uses it to attach the model's `element` text to the ref locator. `locator.description()` getter since 1.57 ([PW-REL v1.57.0]). |
| `page.consoleMessages()`, `page.pageErrors()`, `page.requests()` | 1.56 | "most recent" messages and requests ([PW-REL v1.56.0]). `filter` option and `page.clearConsoleMessages()`/`clearPageErrors()` in 1.59. |
| Test Agents: planner, generator, healer; `npx playwright init-agents --loop=vscode\|claude\|codex\|opencode\|copilot\|vscode-legacy` | 1.56 | Writes agent definitions plus an MCP config for the hidden `npx playwright run-test-mcp-server` ("playwright-test"). Agent tools include `planner_setup_page`, `planner_save_plan`, `generator_setup_page`, `generator_read_log`, `generator_write_test`, `test_run`, `test_list`, `test_debug` and the `browser_*` tools. Artifacts: `specs/*.md` and `tests/*.spec.ts` seeded by `tests/seed.spec.ts` ([PW-REL v1.56.0]; https://playwright.dev/docs/test-agents; `packages/playwright/src/program.ts` and `src/agents/*.agent.md` at v1.64.1). Renamed to "test agents" in 1.56.1 (pw#37759). Agent files pin `model: sonnet`. Docs don't label it experimental. |
| `npx playwright init-skills --loop=claude\|agents` | ≤1.64 | Copies `playwright-cli`, `playwright-component-testing` and `playwright-trace` skills into `.claude/skills` or `.agents/skills` ([SRC:utils/installSkills.ts]; `packages/playwright/src/program.ts`). |
| `page._snapshotForAI()` (private) | ≤1.55 to 1.58 | Returned `{full, incremental?}` with `track`. Removed in 1.59 and merged into `ariaSnapshot({mode:'ai'})` (pw#39759). |
| `page.agent()` / `PageAgent` (hidden) | 1.58 only | See §7. Ripped out 2026-03-11 (pw#39626). |
| `page.ariaSnapshot()` | 1.59 | Page-level shortcut. `locator.ariaSnapshot` itself dates from 1.49. |
| `ariaSnapshot({ mode: 'ai'\|'default', depth })` | 1.59 | AI mode "Includes element references `[ref=e2]`", doesn't wait for a match and throws when nothing matches, and includes iframe content (types.d.ts L14560+). |
| `ariaSnapshot({ boxes })` | 1.60 | `[box=x,y,width,height]`, "useful for AI consumption" ([PW-REL v1.60.0]). |
| `locator.normalize()` | 1.59 | Rewrites a locator to best-practice form, e.g. ref→`getByRole(...)`. This is how MCP codegen turns refs into stable code. |
| `page.pickLocator()` | 1.59 | Human picks an element interactively. |
| `browser.bind()`, `--debug=cli`, `npx playwright trace …`, `page.screencast`, `browserContext.debugger` | 1.59 | Agent interop and observability ([PW-REL v1.59.0]). |
| `locator.ariaSnapshotJSON()`, `page.ariaSnapshotJSON()` (JS only) | 1.63 | JSON nodes `{role, name, text, children, checked, disabled, …, ref, cursor:'pointer', box}`. |
| `page.getByRef('e2' \| 'f1e3')` | 1.64 | Resolves against the latest AI-mode snapshot in that frame ([PW-REL v1.64.0]). |
| `page.webmcp` / `frame.webmcp`: `tools()`, `callTool(name, input)` | 1.64 | Experimental browser feature, enabled with `--enable-features=WebMCP`. "treat them as untrusted input". |
| Removed: `Locator.ariaRef()` | removed 1.60 | "use the standard locator.ariaSnapshot() pipeline" ([PW-REL v1.60.0]). |

Checked across v1.55.0 to v1.64.0 (types.d.ts and client `page.ts`): there is no public `page.agent`, `page.ai`, `act`, `observe` or `extract` in any release. The only one was the hidden 1.58 `agent()`.

### 10. playwright-cli

- **Install and run:** `npm install -g @playwright/cli@latest`, then `playwright-cli --help`. Without a global install, use `npx playwright cli` (core ≥1.62) ([CLI-RM] "Installation", "Local installation").
- **Skills:** `playwright-cli install --skills` installs into `.claude/skills/playwright-cli` (or `.agents`). The skill's frontmatter is `allowed-tools: Bash(playwright-cli:*) Bash(npx playwright:*)` (https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/tools/skills/playwright-cli/SKILL.md).
  - "Skills-less" use is supported: the agent reads `--help` ([CLI-RM]).
  - The client warns about stale skills and checks npm for updates once a day; `NO_UPDATE_NOTIFIER`/`CI` turn this off (https://github.com/microsoft/playwright-cli/blob/v0.1.22/playwright-cli.js).
  - A plugin marketplace for the skill was declined: "plugin won't resolve the freshness problem" (cli#335, 2026-06-25).
- **Grammar:** `playwright-cli [-s=<session>] [--json|--raw] <command> [args] [--options]`. About 100 command names are registered in [SRC:cli-daemon/commands.ts] (`name:` entries, v1.64.1); [CLI-RM] "Commands" documents them. Groups:
  - **Navigation:** `open [url]`, `goto`, `go-back`, `go-forward`, `reload`, `close`.
  - **Input:** `click <ref> [button]`, `dblclick`, `fill <ref> <text> [--submit]`, `type`, `drag`, `drop --path|--data`, `hover`, `select`, `upload`, `check`, `uncheck`, `press`, `keydown`, `keyup`, `mouse*`.
  - **Inspection:** `snapshot [ref|selector] [--filename] [--depth] [--boxes]`, `find <text>|--regex`, `eval <func> [ref]`, `run-code <code>|--filename`, `generate-locator`, `highlight`.
  - **Tabs:** `tab-list`, `tab-new`, `tab-close`, `tab-select`.
  - **Storage:** `state-save`, `state-load`, `cookie-*`, `localstorage-*`, `sessionstorage-*`.
  - **Network:** `route`, `route-list`, `unroute`, `network-state-set`, `requests`, `request <n>`, `request-headers`, `request-body`, `response-headers`, `response-body`.
  - **Debug and observability:** `console`, `tracing-*`, `video-*`, `recording-*`, `show [--annotate]`, `resume`, `step-over`, `pause-at`.
  - **Emulation:** `set-color-scheme` / `set-reduced-motion` / `set-forced-colors` / `set-contrast` / `set-media` and their `clear-*` pairs.
  - **WebMCP:** `webmcp-list`, `webmcp-call <name> --params=JSON`.
  - **Sessions:** `attach --extension[=chrome|msedge] | --cdp=<channel|url> | <name>`, `detach`, `list`, `close-all`, `kill-all`, `delete-data`, `install [--skills]`, `install-browser`, `config-print`.
- **Targets** are refs (`e15`), CSS (`"#main > button"`) or Playwright locator strings (`"getByRole('button', { name: 'Submit' })"`) ([CLI-RM] "Targeting elements").
- **Output contract.** Text has the same Markdown sections as MCP. Every command prints `### Page` and `### Snapshot` with a link such as `[Snapshot](.playwright-cli/page-2026-02-14T19-22-42-679Z.yml)`; the snapshot is a file, not inline ([CLI-RM] "Snapshots").
  - `--raw` (v0.1.5) strips everything but the result, which makes it pipeable: `TOKEN=$(playwright-cli --raw cookie-get session_id)`.
  - `--json` (pw#40284, 2026-04-17) wraps each reply as JSON.
  - Read-only getters emit raw output by default since v0.1.10 ([CLI-REL v0.1.5], [CLI-REL v0.1.10]).
- **Session and daemon model:**
  - `open` spawns a **detached per-session daemon** (`spawn(..., { detached: true })`) that owns the browser and a `BrowserBackend`. Each CLI invocation connects over a Unix socket or named pipe and sends `run {args, cwd, raw, json}` ([SRC:cli-client/session.ts], [SRC:cli-daemon/daemon.ts]).
  - Each workspace gets its own daemon (v0.0.64), and daemons exit if their socket file is deleted or replaced (v0.1.20).
  - Sessions are named with `-s=name` or `PLAYWRIGHT_CLI_SESSION`. `--session` was replaced by `-s=` and the old `session-*` commands by `list`, `close-all`, `kill-all` (v0.0.64).
  - **The CLI profile is in-memory by default** ("Incognito by default", v0.0.64). Opt into disk with `--persistent` or `--profile=<dir>` ([MCP-REL v0.0.64], whose body is the CLI changelog; [CLI-RM] "Sessions").
  - A headless session dies after 1 h idle.
  - A client older than its daemon is refused: "Client is vX, session is vY. Run playwright-cli open to restart" ([SRC:cli-client/session.ts]).
- **How it differs from MCP, in the vendor's words:** "CLI invocations are more token-efficient: they avoid loading large tool schemas and verbose accessibility trees into the model context." MCP "remains relevant for specialized agentic loops that benefit from persistent state, rich introspection, and iterative reasoning" ([MCP-RM] and [CLI-RM], identical text). Launch blurb: "token use … we expect to go down since snapshots are no longer forced into the LLM!" ([PW-REL v1.58.0]; [MCP-REL v0.0.58]).
- **Measured difference has shrunk.** After v0.0.69 put MCP snapshots on disk too, two 2026 blog tests found near-parity:
  - Checkly (2026-07-30): MCP 48k–50k tokens versus CLI 45k–48k of final context, no versions stated. Their verdict: "There's no big difference in using CLIs or MCPs these days." https://www.checklyhq.com/blog/mcp-vs-cli-token-efficiency/
  - dev.to, Aswani Kumar (edited 2026-07-24), `@playwright/mcp@0.0.78` vs `@playwright/cli@0.1.17`: about 3,580 versus 3,620 characters returned; fixed overhead was about 18k characters of MCP tool definitions versus about 12.6k for the CLI skill file. https://dev.to/aswani25/playwright-cli-vs-playwright-mcp-which-should-you-use-with-claude-code-1olh
  - The widely repeated earlier "114k vs 27k tokens (~4×)" figure is second-hand, and its origin is disputed: ytyng.com, test-lab.ai, or Microsoft (https://bug0.com/blog/playwright-cli-vs-playwright-mcp-ai-browser-testing-2026, 2026-06-26, vendor post).

### 11. Breaking changes and renames, MCP 0.0.7 to 0.0.83 (all pre-1.0)

Dated by README diff across all 73 tags, plus release notes.

| Version (date) | Change |
|---|---|
| v0.0.10 (2025-04-06) | `browser_choose_file`→`browser_file_upload`, `go_back/forward`→`navigate_back/forward`, `save_as_pdf`→`pdf_save`, `screenshot`/`move_mouse` and the vision tools re-cut as `browser_screen_*`, tabs split into `tab_list/new/close/select`. |
| v0.0.24 (2025-05-12) | `browser_wait`→`browser_wait_for`. |
| v0.0.25 (2025-05-13) | Sandbox on by default (`--no-sandbox` to opt out); `--isolated` added. |
| v0.0.27 | `--no-image-responses`→`--image-responses`. |
| **v0.0.31 (2025-07-17)** | **`--vision` deprecated, now `--caps=vision`.** `browser_screen_*` tools replaced by `browser_mouse_*_xy`. **`browser_pdf_save` moved behind `--caps=pdf`.** **`browser_generate_playwright_test` removed.** Tab indexes became 0-based ([MCP-REL v0.0.31] "Breaking"). |
| v0.0.32 | Streamable HTTP is the default, at `/mcp`. |
| v0.0.34 (2025-08-16) | `--extension` added: connect to the default Chrome profile via the companion extension. |
| v0.0.35 (2025-08-24) | Tab tools merged into `browser_tabs(action)`. `navigate_forward` removed "due to low usage counts" ([MCP-REL v0.0.35]). |
| v0.0.36→0.0.38→0.0.43 | `browser_verify_*` added under `--caps=verify`, dropped from the README in v0.0.38, re-added under `--caps=testing` with `browser_generate_locator` in v0.0.43. |
| v0.0.47 (2025-11-14) | `--allowed-origins`/`--blocked-origins` removed ("use `--proxy`"). Restored in v0.0.48. `browser_run_code` added. |
| v0.0.51 | `--snapshot-mode` added. `browser_network_requests` drops static resources by default. |
| v0.0.55 (2026-01-09) | Protocol and filesystem restrictions on by default: `file://` blocked, uploads limited to roots. Opt out with `--allow-unrestricted-file-access`. |
| v0.0.58–0.0.62 | playwright-cli ships inside `@playwright/mcp` (`npm i -g @playwright/mcp; playwright-cli`), then moves to its own `@playwright/cli` package. Tracing tools dropped at v0.0.62, then returned in v0.0.69. |
| v0.0.67/68 (2026-02-14) | `--no-sandbox` renamed `--no-chromium-sandbox` in v0.0.67, reverted in v0.0.68 the same day. The extension moved to the Chrome Web Store **with a new extension ID; older MCP versions could not connect** (mcp#1402: "update Playwright MCP to 0.0.67+"). |
| **v0.0.69 (2026-03-30)** | `browser_install` removed. **`--save-trace`/`--save-video` removed** and replaced by `--caps=devtools` tools. New `config`, `network` and `storage` caps. **Incremental snapshot mode removed; post-action snapshots go to files.** Non-ref selectors accepted. |
| **v0.0.71** | **Param `ref`→`target`; `element` optional on all tools.** |
| **v0.0.72 (2026-04-30)** | **`browser_run_code`→`browser_run_code_unsafe`.** `browser_network_requests` returns a numbered list plus the new `browser_network_request`. |
| v0.0.79 | `--output-mode` removed. Extension protocol v1 removed. |
| v0.0.81→v0.0.82 | `browser_webmcp_list`/`browser_webmcp_call` added, then replaced within 4 days by dynamic `webmcp_<tool>` tools. |
| CLI | `--session`→`-s=` (v0.0.64). `open --extension`→`attach --extension` (v0.1.5). `network`→`requests`/`request <n>` (v0.1.10). "Playwright MCP Bridge"→"Playwright Extension" (v0.1.9). Package `playwright-cli`→`@playwright/cli`, with the bin name kept (cli#327: "understood and expected"). |

**User reaction to churn.**
- mcp#769 (2025-07): users pinned `@playwright/mcp@0.0.31` to escape a profile-lock regression.
- mcp#1211 (2025-11): users pinned 0.0.42 to dodge a screenshot error that turned out to be Claude Code mislabeling images.
- mcp#1402 (2026-02): extension ID breakage, 16 comments.
- mcp#1211 again: users found the monorepo move confusing: "the move to the monorepo has made traversing that confusing" (tarekrached, 2025-11-21).

### 12. Samples (my own code, matching docs at the versions above)

#### 12a. MCP config and a tool-call sequence

`.mcp.json` (client config). Flags as in [MCP-RM]:
```json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["@playwright/mcp@0.0.83", "--isolated", "--headless",
               "--caps=devtools,network", "--codegen=python",
               "--secrets=.env.agent", "--output-dir=.playwright-mcp",
               "--timeout-action=8000"]
    }
  }
}
```
Alternatively, `--config pw-mcp.json` with:
```json
{ "browser": { "browserName": "chromium", "isolated": true,
               "contextOptions": { "viewport": { "width": 1280, "height": 720 } } },
  "capabilities": ["core", "devtools"], "snapshot": { "mode": "full", "boxes": false },
  "timeouts": { "action": 8000, "settle": 500 }, "imageResponses": "omit", "codegen": "typescript" }
```
Tool calls, with the response shape abridged to the sections emitted by `response.ts`:
```jsonc
// 1
{"name":"browser_navigate","arguments":{"url":"https://demo.playwright.dev/todomvc/"}}
// → ### Ran Playwright code
//   await page.goto('https://demo.playwright.dev/todomvc/');
//   ### Page
//   - Page URL: https://demo.playwright.dev/todomvc/#/
//   - Page Title: <title>                (illustrative values)
//   ### Snapshot
//   - [Snapshot](.playwright-mcp/page-2026-10-14T09-00-00-000Z.yml)   ← file link, not inline (≥ v0.0.69)
// 2  explicit snapshot (inline YAML) or a cheaper search
{"name":"browser_snapshot","arguments":{"depth":6}}
{"name":"browser_find","arguments":{"text":"What needs to be done"}}
// → - textbox "What needs to be done?" [ref=e5]
// 3  act by ref (element text is optional; it is used to describe() the locator)
{"name":"browser_type","arguments":{"element":"new todo input","target":"e5","text":"Buy milk","submit":true}}
// → ### Ran Playwright code
//   await page.getByRole('textbox', { name: 'What needs to be done?' }).fill('Buy milk');
//   await page.getByRole('textbox', { name: 'What needs to be done?' }).press('Enter');
// 4  stale ref → model-visible error, isError:true
{"name":"browser_click","arguments":{"target":"e99"}}
// → ### Error
//   Error: Ref e99 not found in the current page snapshot. Try capturing new snapshot.
// 5  escape hatch, batched (runs in the server process; RCE-equivalent)
{"name":"browser_run_code_unsafe","arguments":{"code":"async (page) => { await page.getByTestId('todo-item').first().hover(); return await page.getByTestId('todo-item').count(); }"}}
// → ### Result
//   1
```

#### 12b. playwright-cli shell session
```bash
npm i -g @playwright/cli@0.1.22 && playwright-cli install --skills
export PLAYWRIGHT_CLI_SESSION=todo          # or prefix each call with -s=todo
playwright-cli open https://demo.playwright.dev/todomvc/   # spawns daemon; in-memory profile
playwright-cli find "What needs to be done"                 # grep the snapshot, returns [ref=e5]
playwright-cli fill e5 "Buy milk" --submit
playwright-cli --raw eval "() => document.querySelectorAll('.todo-list li').length"   # → 1
playwright-cli --raw snapshot > after.yml
playwright-cli requests                    # numbered list; add --static to include images/fonts/scripts
playwright-cli --raw response-body 3 > body.json
playwright-cli state-save auth.json
playwright-cli list --json                  # structured session listing (SKILL.md form)
playwright-cli close                        # or: playwright-cli close-all / kill-all
```

#### 12c. TypeScript: core AI-ish APIs (Playwright ≥1.64)
```ts
import { chromium } from 'playwright';

const browser = await chromium.launch({ args: ['--enable-features=WebMCP'] });
const page = await browser.newPage();
await page.goto('https://demo.playwright.dev/todomvc/');

// AI-mode snapshot: YAML with [ref=eN], iframes inlined; JSON variant since 1.63.
const yaml = await page.ariaSnapshot({ mode: 'ai', depth: 8, boxes: true });
const tree = await page.ariaSnapshotJSON({ mode: 'ai' });          // nodes carry .ref, .cursor, .box

// Ref → locator (1.64) → resilient code (normalize, 1.59).
const input = page.getByRef('e5').describe('new todo input');      // describe() since 1.53
await input.fill('Buy milk');
await input.press('Enter');
console.log((await input.normalize()).toString());                 // getByRole('textbox', { name: … })

// Post-hoc diagnostics (1.56).
const errors = (await page.consoleMessages()).filter(m => m.type() === 'error');
const apiCalls = (await page.requests()).filter(r => r.url().includes('/api/'));

// Page-registered tools (experimental WebMCP, 1.64).
for (const tool of await page.webmcp.tools()) console.log(tool.name);
await browser.close();
```

### 13. Pains, ranked by evidence weight

Weight = independent reports + reactions + recurrence over time. Status as of 2026-10-14.

| # | Pain | Reports / links | Dates / versions | Status & maintainer response | Lesson for our SDK |
|---|---|---|---|---|---|
| 1 | **Output size and token burn** (snapshot after every action, console spam) | mcp#395 (+1=14, 8c), #889 (+13, 15c: "×6 tokens from v0.0.30 to v0.0.32"), #806 (+9), #631 (+7, "+1470 lines" for one keypress), #1772 (tool list 19.6 KB); cli#398; HN dragac 49372088, braebo 47679430, mambodog 47392567 (wrote a Haiku-summarizing wrapper) | 2025-05 → 2026-09; v0.0.2x–0.0.82 | **Refused at first:** "tokens will get cheaper faster than we can elaborate a meaningful pagination solution" (#395, 2025-05-12); "we don't think we'll be adding a way to limit the number of tokens" (#889, 2025-09-07); "Not acting on this atm" (#631). **Then reversed in steps:** incremental diffs (v0.0.43), snapshot modes (v0.0.51), file snapshots (v0.0.52), CLI (v0.0.58), snapshot-to-file default (v0.0.69), `--output-max-size` (v0.0.76), `browser_find` and distilled snapshots (v0.0.78). Server-side tool filtering still declined: "belongs in the host" (#1772, 2026-09-25). | Make observation **budgeted and addressable from day 1**: return a handle to the snapshot plus a summary, with `find`/`depth`/`target` scoping and a byte cap. Never inline unbounded page state by default. Keep the tool list small and stable for prompt caching. |
| 2 | **"Browser is already in use"** (profile lock; also masked launch failures) | mcp#636 (9c), #769 (25c), #891 (+11, 11c, "dumping ground"), #942 (16c; "misleading error message", DKingAlpha 2026-01-14); cli#293; HN hombre_fatal 47546241 ("only one Claude Code instance"), silverwind 47551864 | 2025-07 → 2026-03; regression after v0.0.31 | Users told to use `--isolated`: "The error message is telling you what to do" (#636). Clearer launch-failure diagnostics in v0.0.56. CLI made in-memory profiles the default (v0.0.64). Lock-file fix in v0.0.69. Profile dir keyed by workspace hash. | **Default to ephemeral isolated contexts.** Make shared and persistent profiles an explicit opt-in with a lease or owner. Give distinct error codes for "profile locked" vs "launch failed (missing deps / sandbox)". Make multi-client attach first-class. |
| 3 | **Need raw JS / batched code** | mcp#424 (+25, the top issue), #103 (11c, "view dom"); HN sozal 47351433 (another MCP: 330K vs 1.5M tokens and 12 vs 48 turns, attributed to its batching `execute` tool by heisen_berg 47352150), tanishqkanc 49006542 | 2025-04 → 2026-07 | `browser_evaluate` added (v0.0.31), `browser_run_code` (v0.0.47), renamed `_unsafe` (v0.0.72). Hardening report mcp#1784 closed without comment. | Ship an **escape hatch that batches several page operations in one call**, with an explicit trust label (page-context eval vs host-process code) and policy hooks. |
| 4 | **Turning agent runs into reusable test code** | mcp#123 (+16), #519 (23c), #734 (record human actions) | 2025-04 → 2025-09 | `generate_playwright_test` added, then removed for mixed feedback (v0.0.31). Per-action "Ran Playwright code" with normalized locators. Python/Java/C# codegen (v0.0.79). Recording tools (v0.0.80). Test Agents (1.56). | **Emit deterministic, normalized code for every AI action** in the caller's SDK language. That is our "cache/replay" story. |
| 5 | **Attach to the real logged-in browser / bot detection** | mcp#148 (+11, Google login); #1035 (+8, extension not in store); #1402 (16c, extension ID break); #941 (WSL extension); cli#211, cli#267; HN pitiflautico 49345367 | 2025-04 → 2026-04 | "This would be an uphill battle" (#148). Extension published to the Chrome Web Store 2026-02-24. `attach --extension\|--cdp` in the CLI. Stealth out of scope. | **Separate the "attach to the user's browser" path** (extension/CDP, tab picker, token) from launch. Version the extension protocol with a compatibility window. |
| 6 | **Install and environment failures** (Chrome not found, WSL, containers, root/sandbox, Cursor) | mcp#13 (+12, 18c), #943 (+11), #656 (+6), #635 ("hangs forever if Chrome isn't installed"); cli#219, #292, #309, #340 | 2025-03 → 2026-04 | Mostly "follow instructions" or "use webkit/firefox". Missing-executable path surfaced (v0.0.79). Sandbox disabled for bundled Linux Chromium (v0.0.81). | Bundle or auto-fetch the browser. Make **preflight `doctor`** errors actionable. Never hang on a missing binary. |
| 7 | **Visibility of what the agent does** (video, highlight) | mcp#695 (+10), #492 (+8); HN silverwind | 2025-05 → 2026-06 | "Would you consider saving a trace instead?" (#695). Then `--save-video` (v0.0.40), then video tools with action overlays (v0.0.76/82) and the `show` dashboard (CLI v0.1.1). | Treat **traces, video receipts and a live dashboard** as core, not add-ons. |
| 8 | **Snapshot fidelity** (iframes, visibility, nav races) | mcp#447 (20c, Power Apps, not_planned), #535 (21c), #439 (13c, "Execution context was destroyed", fixed in 0.0.26); 1.62.1 fixes for button names and image elements | 2025-05 → 2026-07 | Mixed. "Playwright MCP is like a human, what you see is what you get" (#535). | Expose **frame-qualified refs** (`f1e3`), and offer a structured JSON snapshot to complement YAML. Report "page navigated during snapshot" as a retryable error. |
| 9 | **Secrets and private data leaking into context or onto disk** | mcp#1566 (password value in snapshot), cli#317, mcp#1789 (artifacts written in repo), #1791 (tokens in tab URLs) | 2026-03 → 2026-10 | "use the secrets feature"; "Treat your .playwright-mcp as .env" (#1789). | **Redact password inputs and credential-looking URL params by default.** Write artifacts outside the repo, or add a `.gitignore` automatically. |
| 10 | **Extensibility and tool filtering** | mcp#442 (+18: custom tools, POMs, fixtures), #1772 | 2025-05 → 2026-09 | "createConnection accepts the context factory, but that's as far as we went" (#442). Filtering declined (#1772). | Provide a **custom-tool/plugin API** and server-side allow/deny lists (security, not just cost). |
| 11 | **Unstable surface and docs drift** | Renames in §11; mcp#1211 thread on the monorepo move; [CLI-RM] config schema stale; mcp#1788 (one-line tool descriptions; maintainer asks for a trajectory) | continuous | Still 0.0.x after 18 months. | Freeze names early: version the tool and verb vocabulary, and generate docs from schemas (they partly do, with `update-readme.js`). |

Minor items: Electron support (mcp#994 +12, closed "no reply"; cli#229 +10, closed without a comment, and no electron option in the v1.64.1 CLI commands). Scroll (mcp#267, "Archiving due to limited upvotes", later `browser_mouse_wheel` in v0.0.57 behind vision). Configurable timeouts (mcp#224, closed, then shipped in v0.0.37).

### 14. Likes, with quotes under 15 words

- "It works much faster for me than the MCP servers I tried." (egeozcan on playwright-cli, HN 2026-07-03, https://news.ycombinator.com/item?id=48772438)
- "highly effective for debugging locally deployed web apps with essentially zero setup" (lsaferite on playwright-cli with Claude Code, HN 2026-05-05, https://news.ycombinator.com/item?id=48029825)
- "playwright MCP often works very well." (hamasho, who said computer use was "mostly miss", HN 2026-05-06, https://news.ycombinator.com/item?id=48031468)
- "Playwright-cli (not mcp) is so much more efficient." (nunodonato, HN 2026-03-24, https://news.ycombinator.com/item?id=47500807)
- "after a bit of tuning it's about 1-2k context per interaction" (esperent, own CLI skill, HN 2026-03-16, https://news.ycombinator.com/item?id=47394277)
- "Writing just a big 'Thank you' for implementing this feature!" (KuzMaxOriginal on `browser_evaluate`, 2025-07-25, mcp#424)
- "Playwright is fantastically useful and a critical part of my workflow." (tarekrached, 2025-12-01, mcp#1211)
- "I really like Playwright and its a much better tool compared to others" (shashikuazuregithub, 2025-09-01, mcp#889, while complaining about token burn)
- Security note: Playwright's ARIA snapshot filters `display:none` prompt-injection text, unlike `textContent` and `innerHTML`. The same tester flagged eval tools as the bypass (guard402, HN 2026-03-16, https://news.ycombinator.com/item?id=47395353).
- Cautionary: an agent driving Playwright MCP hit a login wall and reset a superadmin password in the dev DB (macNchz, HN 2026-03-19, https://news.ycombinator.com/item?id=47433194). Lesson: credentials and human-handoff primitives.

### 15. Implications for our SDK (summary)

1. Treat snapshot delivery as an explicit contract: a handle or file plus a summary by default, search and scope verbs, a byte budget, and JSON plus YAML. Playwright converged here after 10 months of user pain.
2. Refs are snapshot-scoped and frame-qualified, and stale refs fail with a typed "re-observe" error. Every ref action records a `describe()`'d, `normalize()`'d locator, which gives code generation and replay for free.
3. Design one backend with two thin surfaces (protocol and CLI daemon) to avoid drift. Playwright did this; its docs still drift.
4. Use isolated, ephemeral contexts by default, explicit leases for persistent profiles, and distinct launch errors.
5. A batched code-execution verb is the biggest token and turn saver. Label its trust boundary in the name, as with `_unsafe`.
6. Microsoft built and then deleted an in-library `page.agent().perform/expect/extract` with a code cache (1.58). Before we commit, find out why it was cut; the public record shows only "Not ready for primetime!".

### Unverified

- Why `page.agent()` was removed. PR bodies are empty or "Not ready for primetime!" (pw#38882, pw#39626). I found no public rationale.
- Whether `npx playwright mcp` from Python, Java or .NET Playwright packages works. I only saw that `PW_LANG_NAME` drives the codegen default ([SRC:mcp/config.ts]); I did not test the language bindings' CLIs.
- Whether `browser_tabs.isolatedContext` (pw#43087) ships in `@playwright/mcp` 0.0.83. It is in the v1.64.1 tag source but missing from the 0.0.83 README, which pins an earlier alpha.
- The origin and method of the "114k vs 27k tokens" MCP vs CLI benchmark. Blogs attribute it to different sources. The Checkly and dev.to near-parity results are single-author tests; Checkly gives no versions.
- Electron support in the CLI. cli#229 was closed as "completed" with no maintainer comment, and the v1.64.1 command list has no electron option.
- Exact commit that removed `--save-trace`/`--save-video`. It is dated only by the README diff (present at v0.0.68, absent at v0.0.69); the GitHub commit search hit its rate limit.
- `--caps` help text lists only "vision, pdf, devtools", while the README documents `config`/`network`/`storage`/`testing` groups. I did not run the binary to confirm those caps are accepted, though `filteredTools` accepts any listed capability string.
- Reddit sentiment. WebSearch returned no on-topic Reddit thread. A blog-quoted Reddit line ("Playwright MCP is now surpassed with the Playwright CLI") was not traced to its source.

## 2.7 Notte (nottelabs/notte, v1.9.7, 2026-10-14)
Gathered 2026-10-14 from primary sources: tagged source files, package registries, the GitHub API, official docs and HN Algolia. Abbreviations:
- `N@` = `https://github.com/nottelabs/notte/blob/v1.9.7/`
- `H@` = `https://github.com/hyperbrowserai/HyperAgent/blob/a7ec1f4/`
- `M@` = `https://github.com/magnitudedev/browser-agent/blob/f1b587c/`

Code samples are my own and were not executed.

### Status, packaging, license
- **License: SSPL-1.0.** The LICENSE file at the tag is the MongoDB "Server Side Public License, VERSION 1" ([N@LICENSE](https://github.com/nottelabs/notte/blob/v1.9.7/LICENSE)), and the README badge says SSPL-1.0 ([N@README.md](https://github.com/nottelabs/notte/blob/v1.9.7/README.md)). PyPI `notte` declares `SSPL-1.0`. GitHub shows `NOASSERTION` only because it does not recognise SSPL.
- **Python packages:**
  - `notte` is the open-source local framework. `pip install notte`, then `patchright install`.
  - `notte-sdk` is the cloud client.
  - Both are at 1.9.7, uploaded 2026-10-14 ([PyPI notte](https://pypi.org/pypi/notte/json), [PyPI notte-sdk](https://pypi.org/pypi/notte-sdk/json)).
  - `notte` has 103 releases since 0.1.4 (2024-12-10). The cadence is roughly weekly: 1.9.3 2026-09-30, 1.9.4 10-01, 1.9.5 10-06, 1.9.6 10-07, 1.9.7 10-14.
  - The repo is a monorepo of `notte-core`, `notte-browser`, `notte-agent`, `notte-llm`, `notte-sdk` and `notte-integrations`. All require Python >=3.11.
  - The packages are version-locked at runtime: `check_notte_version` raises `ValueError` if `notte_core` and a sibling package differ ([N@packages/notte-core/src/notte_core/__init__.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/__init__.py)).
- **Node SDK:**
  - npm `notte-sdk` is **cloud-only**: no local engine and no AgentFallback export ([N@node-sdk/src/index.ts](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/index.ts)).
  - First published as 0.0.1 on 2026-04-04, now 1.9.7 (2026-10-14), released in lockstep with Python ([npm](https://registry.npmjs.org/notte-sdk)).
  - Issue #167 ("Add nodejs sdk") was open from 2025-03-12 to 2026-04-10 ([#167](https://github.com/nottelabs/notte/issues/167)).
- **Product framing has moved from "agent framework" to "cloud browsers + Functions".** The README's table is CLI / MCP / Browser Sessions / Browser Functions / Anything API / Fetch. The local agent sits in a collapsed "Run the open source agent locally" block ([N@README.md](https://github.com/nottelabs/notte/blob/v1.9.7/README.md)). Docs nav: [docs.notte.cc/llms.txt](https://docs.notte.cc/llms.txt).
- **Activity:** last push 2026-10-14 and 3 open issues (GitHub API).
  - External issue volume is low: the last 300 issues include very few from non-team users, and most "issues" are team tasks (`gh api repos/nottelabs/notte/issues`).
  - User evidence is therefore **thin**. Weight it accordingly.

### Entry and lifecycle
- **Local, `import notte`:**
  - Exports `notte.Session` (= `notte_browser.session.NotteSession`), `notte.Agent`, `notte.AgentFallback`, `notte.Client`/`NotteClient`, `notte.actions`, `notte.models` (= `LlmModel`), `notte.config`, `set_error_mode`, and vault placeholders `EMAIL`, `PASSWORD`, `MFA`, `CARD_*` ([N@src/notte/__init__.py](https://github.com/nottelabs/notte/blob/v1.9.7/src/notte/__init__.py)).
  - `NotteSession` is both `AsyncResource` and `SyncResource`, so it supports `with` and `async with`, plus `start()`/`stop()` and `astart()`/`astop()`.
  - Constructor (keyword-only): `perception_type`, `raise_on_failure`, `cookie_file`, `storage`, `tools`, `vault`, `persona`, `window`, `keep_alive`, `**LocalSessionStartRequestDict` (e.g. `headless`, `cdp_url`, `solve_captchas`, proxies, viewport) ([N@…/notte_browser/session.py#L132-L184](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L132-L184)).
  - `keep_alive=True` skips stop. The source's own log string says to never use it in production (same lines).
- **Cloud:**
  - `NotteClient(api_key=None)` reads `NOTTE_API_KEY`, then the CLI login keychain. Error text: "Provide api_key, set NOTTE_API_KEY, or run 'notte auth login'." ([N@…/notte_sdk/auth.py#L24](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/auth.py#L24)). `NOTTE_API_URL` overrides the server ([N@…/notte_sdk/client.py#L64](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/client.py#L64)).
  - Factories hang off the client: `client.Session`, `client.Agent`, `client.Vault`, `client.Persona`, `client.Function`, `client.AgentFallback`, `client.scrape(url=…)` (same file, L102-L198).
  - `RemoteSession` is a `SyncResource`. It also has `__aenter__`/`__aexit__`, `apage()` and `await_for_auth`, but the verbs (`execute`/`observe`/`scrape`) are **sync only** ([N@…/endpoints/sessions.py#L644](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/sessions.py#L644), `async def` appears only at L761, L784, L934 and L1361).
- **TS:**
  - `new NotteClient()` reads `NOTTE_API_KEY`.
  - `await client.Session({...}).use(async (session) => …)` is the scoped lifecycle; it is the equivalent of Python `with` ([N@node-sdk/src/session.ts#L973](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/session.ts#L973), README TS sample).

### Core verbs (exact shapes at v1.9.7)

| Verb | Python (local `NotteSession` and cloud `RemoteSession`) | Returns | TS (`Session`) |
|---|---|---|---|
| observe | `observe(instructions: str, perception_type=None, **pagination)` | `list[InteractionActionUnion]` (candidate actions) | `observe(options: ObserveRequest \| PerceptionType)` returns `Observation` ([session.ts#L910](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/session.ts#L910)) |
| | `observe(instructions=None, …)` | `Observation` (full page snapshot plus action space) | |
| | Overloads: [session.py#L380-L400](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L380-L400) | | |
| execute | `execute(action: BaseAction, *, raise_on_failure=None)` or `execute(type="click", id="B1" \| selector="…", …)`, with about 30 `Unpack[TypedDict]` overloads, one per action type | `ExecutionResult{action, success, message, data, exception, exception_detail}` ([observation.py#L404-L411](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/browser/observation.py#L404-L411)) | `execute(action, options: boolean \| {raiseOnFailure})` ([session.ts#L811](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/session.ts#L811)) |
| | Overloads: [session.py#L610-L727](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L610-L727) | | |
| scrape | `scrape()` | `str` (markdown) | `scrape({response_format: zodSchema, instructions})` returns `T`; with `raiseOnFailure: false` returns `StructuredData<T>` ([session.ts#L937-L942](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/session.ts#L937-L942)) |
| | `scrape(response_format=Model, instructions=…)` | `Model` | |
| | `…, raise_on_failure=False` | `StructuredData[Model]` | |
| | `scrape(instructions=…)` | `BaseModel` | |
| | `scrape(only_images=True)` | `list[ImageData]` | |
| | Overloads: [session.py#L1211-L1255](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L1211-L1255) | | |
| raw page | local: `session.page` / `await session.apage()` | Playwright `Page` | `await session.page()`, `await session.cdpUrl()` ([session.ts#L698-L730](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/session.ts#L698-L730)) |
| | cloud: `session.page` (lazy CDP connect), `session.cdp_url()` ([sessions.py#L1246-L1302](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/sessions.py#L1246-L1302)) | | |
| replay | local: `replay()` | `WebpReplay` ([session.py#L303](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L303)) | `replay()`, `downloadReplay()` |
| | cloud: `replay(wait=True, timeout=240)` | `ReplayResponse` (presigned `mp4_url`) ([sessions.py#L1047](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/sessions.py#L1047)) | |

**Action model.** Actions are pydantic classes with a `type` literal ([N@…/actions/actions.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/actions/actions.py)):
- Browser actions: `goto`, `goto_new_tab`, `switch_tab`, `go_back`, `reload`, `wait`, `press_key`, `scroll_up`/`down`, `captcha_solve`, `completion`, …
- Interaction actions: `click`, `fill`, `multi_factor_fill`, `check`, `select_dropdown_option`, `upload_file`, `download_file`.
- Tool actions: `scrape`, `email_read`, `sms_read`, `evaluate_js`.
- `InteractionAction` requires **either** `id` (an observe-assigned id such as `B1`/`I1`/`L3`) **or** `selector` (CSS, XPath or Playwright `internal:role=…`). It also carries `press_enter`, `timeout` (ms) and `text_label` ([actions.py#L1047-L1084](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/actions/actions.py#L1047-L1084)).
- Ids are only valid against the latest `observe()`. The docstring warns: "Only use IDs that appear in your live observe() output." ([sessions.py#L1691-L1725](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/sessions.py#L1691-L1725)).

**Raise vs result.**
- `raise_on_failure` defaults to `config.raise_on_session_execution_failure = true` (config.toml). It can be set per session or per call.
- `scrape` uses overloads on `Literal[True/False]` so the return type changes: unwrapped model or `StructuredData` wrapper.
- TS mixes naming styles: `response_format` and `solve_captchas` are snake_case, while `raiseOnFailure` is camelCase (session.ts L937-L942, README TS sample).

### Agent API
- **Local:**
  - `notte.Agent(session, vault=None, persona=None, notifier=None, agent_type=FALCO, trajectory=None, **AgentCreateRequestDict)` ([N@…/notte_agent/main.py#L30-L56](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-agent/src/notte_agent/main.py#L30-L56)).
  - Create options: `reasoning_model: LlmModel | str`, `use_vision: bool`, `max_steps: int`, `vault_id`, `persona_id`, `notifier_config`.
  - Run options: `task` (required), `url`, `response_format: type[BaseModel]`, `session_offset` (experimental: reuse the session's earlier steps as memory) ([N@…/notte_sdk/types.py#L2139-L2176](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/types.py#L2139-L2176)).
  - `agent.run(**)` / `await agent.arun(**)` return `AgentResponse{success, answer, trajectory(excluded), created_at, closed_at, status, llm_messages, llm_usage, duration_in_s, steps}` ([N@…/notte_agent/common/types.py#L18-L41](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-agent/src/notte_agent/common/types.py#L18-L41)).
  - An LLM-as-judge `CompletionValidator` checks the agent's completion claim ([N@…/notte_agent/agent.py#L71-L74](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-agent/src/notte_agent/agent.py#L71-L74)).
  - Defaults: `max_steps=20`, `use_vision=true`, `max_consecutive_failures=3` ([config.toml](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/config.toml)). The docs say `max_steps` is 1–150 ([docs agent](https://docs.notte.cc/sdk-reference/manual/agent.md)).
- **Cloud:**
  - `client.Agent(session=…, vault=…, max_steps=…)` exposes `.run()` (blocking), `.start()`, `.wait()`, `.watch_logs_and_wait()`, `.stop()`, `.status()` and `.replay()` ([N@…/endpoints/agents.py#L1034-L1247](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/agents.py#L1034-L1247)).
  - TS mirrors it with `run/start/stop/status/hasStarted/isActive` ([docs TS agent](https://docs.notte.cc/typescript-sdk-reference/manual/agent.md)). TS `response_format` accepts a Zod schema, which is converted with `z.toJSONSchema` ([N@node-sdk/src/agent.ts#L21-L31](https://github.com/nottelabs/notte/blob/v1.9.7/node-sdk/src/agent.ts#L21-L31)).
- **Custom tools:** `tools: list[BaseTool]` on the session ([session.py#L141-L153](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L141-L153)). There is no decorator-style tool API in the public surface. *(Python local only; I found no TS equivalent.)*
- **Credentials:**
  - Vaults inject credentials "without leaking them to the LLM (text + screenshots)" ([agent.py#L81-L82](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-agent/src/notte_agent/agent.py#L81-L82)).
  - Placeholders `notte.EMAIL`/`PASSWORD`/`MFA` are substituted at execute time ([session.py#L606-L608](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L606-L608)).
  - Cloud usage: `client.Vault(vault_id).add_credentials(url=, email=, password=)`, then `client.Agent(vault=vault, …)` ([docs agent](https://docs.notte.cc/sdk-reference/manual/agent.md)).

### Determinism: scripts, functions, fallback, replay
- **AgentFallback** is a context manager that **monkey-patches** `session.aexecute`/`ascrape` for the duration of the block ([N@…/notte_agent/agent_fallback.py#L29-L80](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-agent/src/notte_agent/agent_fallback.py#L29-L80)).
  - Inside the block every `execute` runs with `raise_on_failure=False`. On the first failure it spawns an agent with `task` on the same session, and the remaining `execute()` calls in the block are skipped.
  - Fields: `.success`, `.agent_invoked`, `.agent_response`, `.steps`.
  - **Restrictions:** `session.scrape()` inside the block raises `ValueError`, and so does `execute(..., raise_on_failure=True)` ([docs agent_fallback](https://docs.notte.cc/sdk-reference/manual/agent_fallback.md)).
  - Exists in Python local and cloud (`client.AgentFallback`, [N@…/notte_sdk/agent_fallback.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/agent_fallback.py)). **Not in TS.**
- **Agent to code to Function:**
  - `agent.workflow.code(as_workflow=True)` returns a script reproducing the agent's steps.
  - `agent.workflow.create_function()` returns a `NotteFunction`, a deployed serverless endpoint ([agents.py#L862-L905](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/endpoints/agents.py#L862-L905); `AgentsClient.function_code`/`create_function` at L717-L762).
  - A Function is called as `client.Function(function_id=…).run(**vars)` and returns `.result` (README).
  - TS: `session.getScript()` (docs nav, [llms.txt](https://docs.notte.cc/llms.txt)).
- **Local replay of saved actions:** `session.execute_saved_actions(actions_file)` loads an `ActionList` JSON, executes each action with `raise_on_failure=False`, and stops at the first failure ([session.py#L983-L998](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/session.py#L983-L998)). There is **no self-healing** here; healing is the AgentFallback's job.
- The HN founder describes the intended model: "write a web automation script where at some parts you have a very specialized agent" ([HN 44829000](https://news.ycombinator.com/item?id=44829000)). It also covers recovery: "fallback AI agents that can recover instead of just failing" ([HN 46808078](https://news.ycombinator.com/item?id=46808078)).

### Model config, errors, observability
- **Models:**
  - LiteLLM provider strings `provider/model` (dependency `litellm>=1.74.12`, [N@packages/notte-llm/pyproject.toml](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-llm/pyproject.toml)).
  - The `LlmModel` enum includes `gemini/gemini-3.5-flash`, `anthropic/claude-sonnet-4-5-20250929`, `openai/gpt-4o`, `groq/gpt-oss-120b`, `vertex_ai/…` and `openrouter/…`.
  - The API key env var is derived from the provider (`GEMINI_API_KEY`, `ANTHROPIC_API_KEY`, …). `ENABLE_OPENROUTER` reroutes models through OpenRouter ([N@…/common/config.py#L26-L146](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/common/config.py#L26-L146)).
  - Settings live in a TOML file, overridable with `NOTTE_CONFIG_PATH` (config.py#L374).
  - `perception_type` is `fast` (DOM) or `deep` (LLM page parse) ([config.toml](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/config.toml)).
- **Errors:**
  - The SDK exceptions `AuthenticationError`, `InvalidRequestError`, `NotteAPIExecutionError` and `NotteAPIError` are **siblings**: catching `NotteAPIError` does not catch the other three. The common base is `NotteBaseError(ValueError)` ([docs errors](https://docs.notte.cc/sdk-reference/errors.md), [N@…/notte_sdk/errors.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/errors.py)).
  - Errors carry three messages (`dev_message`, `user_message`, `agent_message`), selected with `notte.set_error_mode("developer"|"user"|"agent")` ([N@…/notte_core/__init__.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/__init__.py), [errors/base.py](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/errors/base.py)).
- **Observability:**
  - Session replay (WebP locally, MP4 in the cloud), live viewer (`session.viewer()`), and `agent.watch_logs_and_wait()` log streaming (sessions.py, agents.py).
  - Telemetry goes to PostHog and Scarf **on by default**; it is opt-out with `DISABLE_TELEMETRY=true` ([N@…/common/telemetry.py#L39-L45](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/common/telemetry.py#L39-L45)).
- **Import side effect:** importing `notte_sdk` calls `enable_nest_asyncio()`, which patches the global event loop unless `NOTTE_ENABLE_NEST_ASYNCIO` is not `"true"` ([client.py#L30](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/client.py#L30), [core __init__ L53-L63](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-core/src/notte_core/__init__.py#L53-L63)). See #509.
- **Compatibility policy:**
  - The internal doc requires optional fields to be `None`-omitted at the wire boundary so older strict clients keep working ([N@docs/backward-compatibility.md](https://github.com/nottelabs/notte/blob/v1.9.7/docs/backward-compatibility.md)).
  - The SDK nags on startup when a newer version exists, and the docs tell users who hit "extra inputs not permitted" to upgrade ([docs errors](https://docs.notte.cc/sdk-reference/errors.md)).
- **Python/TS parity:**
  - TS is generated from the OpenAPI spec (`node-sdk/src/lib/client/*.gen.ts`) plus a hand-written facade, and its docstrings say "mirrors Python …".
  - Gaps: no local engine, no AgentFallback and no `observe(instructions)` list overload in TS; casing is mixed.
  - The team keeps cross-language snippet parity tests (`docs/src/sniptest/parity.py`, `parity.json` in the [tree](https://github.com/nottelabs/notte/tree/v1.9.7/docs/src/sniptest)).

### Samples (own code)
```python
# Local OSS: hybrid script + AI, raw Playwright escape hatch
import notte
from pydantic import BaseModel

class Post(BaseModel):
    title: str
    points: int

class Posts(BaseModel):
    posts: list[Post]

with notte.Session(headless=True) as session:          # sync; `async with` + a* methods also exist
    session.execute(type="goto", url="https://news.ycombinator.com")
    more = session.observe(instructions="the 'More' pagination link")  # -> list of candidate actions
    if more:
        session.execute(more[0])                         # or type="click", selector="a.morelink"
    top = session.scrape(response_format=Posts, instructions="first 5 posts")  # -> Posts (raises on failure)
    session.page.screenshot(path="hn.png")              # raw Playwright Page
    agent = notte.Agent(session=session, reasoning_model="gemini/gemini-3.5-flash", max_steps=5)
    resp = agent.run(task="Open the comments of the top post")
    print(resp.success, resp.answer, len(resp.steps))
```
```python
# Cloud: scripted path with an agent fallback
from notte_sdk import NotteClient
client = NotteClient()  # NOTTE_API_KEY
with client.Session(proxies=True) as session:
    session.execute(type="goto", url="https://example.com/search")
    with client.AgentFallback(session=session, task="Search for 'laptops'", max_steps=6) as fb:
        session.execute(type="fill", selector="input[name=q]", value="laptops")
        session.execute(type="press_key", key="Enter")
    print(fb.success, fb.agent_invoked)
```
```ts
import { NotteClient } from 'notte-sdk';
import { z } from 'zod';
const client = new NotteClient();
const Title = z.object({ title: z.string() });
const out = await client.Session({ solve_captchas: true }).use(async (s) => {
  await s.execute({ type: 'goto', url: 'https://example.com' });
  const r = await s.execute({ type: 'click', selector: 'a' }, { raiseOnFailure: false });
  if (!r.success) console.warn(r.message);
  return s.scrape({ response_format: Title, instructions: 'page title' }); // -> {title}
});
```

### Pain points (ranked by evidence weight)

| Pain | Reports/links | Dates/versions | Status | API-design lesson |
|---|---|---|---|---|
| Model or provider key confusion: AI Studio vs Vertex key, opaque `InvalidAPIKeyError` | [#545](https://github.com/nottelabs/notte/issues/545) (7 comments, 2 users); maintainer moved it to call/Slack | 2025-09-08 → 09-11 | Closed without a documented fix | Error must name the env var read, the provider resolved and the model string; offer `doctor`-style validation of credentials before a run |
| Client/server schema drift breaks old clients (`extra_forbidden` on `AgentResponse.credit_usage`) | [#551](https://github.com/nottelabs/notte/issues/551); docs page for "extra inputs not permitted" ([errors](https://docs.notte.cc/sdk-reference/errors.md)); internal [backward-compatibility.md](https://github.com/nottelabs/notte/blob/v1.9.7/docs/backward-compatibility.md) | 2025-09-11, notte-sdk 1.6.9 | Fixed by upgrading; policy doc added later | Clients must ignore unknown response fields; servers omit `None`; version the wire contract independently of SDK releases |
| Import-time global side effect (`nest_asyncio`) breaks uvloop users | [#509](https://github.com/nottelabs/notte/issues/509) (7 comments) | 2025-08-20 → 10-16 | Closed "due to workaround". Still applied on import at v1.9.7 ([client.py#L30](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-sdk/src/notte_sdk/client.py#L30)); the thread's env-var name `NOTTE_DISABLE_NEST_ASYNCIO` is wrong, the real one is `NOTTE_ENABLE_NEST_ASYNCIO` | No import-time patching of the host runtime; offer real async APIs instead of nesting sync over async |
| Breaking kwarg/config changes shipped while the PyPI upload failed (`unexpected keyword argument 'headless'`) | [#334](https://github.com/nottelabs/notte/issues/334) | 2025-06-01 | Fixed the same day | Release atomically across packages; keep docs snippets tested against published versions (Notte now runs README/doc snippet tests: PR #1141 in the [v1.9.6…v1.9.7 compare](https://github.com/nottelabs/notte/compare/v1.9.6...v1.9.7)) |
| Local `Session(cdp_url=…)` always raised; `connect_cdp_browser` was never called | [#1123](https://github.com/nottelabs/notte/issues/1123), re-verified | Reported 2026-10-08 on 1.9.6; fixed by [PR #1152](https://github.com/nottelabs/notte/pull/1152), merged 2026-10-13; in v1.9.7 `new_window` branches to `connect_cdp_browser` ([playwright.py#L161-L175](https://github.com/nottelabs/notte/blob/v1.9.7/packages/notte-browser/src/notte_browser/playwright.py#L161-L175)) | **Fixed in 1.9.7.** Closed with 0 comments | The local and cloud paths share a parameter name but not code. Test every documented option on every backend (an "attach over CDP" path needs its own integration test) |
| Missing TS SDK for 13 months | [#167](https://github.com/nottelabs/notte/issues/167) (6 comments; users asking for an ETA), [HN 44861659](https://news.ycombinator.com/item?id=44861659) "no typescript support?" | 2025-03-12 → 2026-04-10 | Shipped as cloud-only | Day-one multi-language parity matters; generate bindings from one contract |
| TS package name mismatch (`@notte/sdk` in source vs `notte-sdk` on npm) | [#804](https://github.com/nottelabs/notte/issues/804) | 2026-05-08 → 07-28 | Fixed | One canonical package name, checked in CI |
| Credit pricing opaque; users pay for failed agent runs | [HN 44831780](https://news.ycombinator.com/item?id=44831780), [44849781](https://news.ycombinator.com/item?id=44849781), [44856070](https://news.ycombinator.com/item?id=44856070) (founder: "working on fail-credit refunds") | 2025-08 | Pricing page updated | Expose per-call cost/usage on every result (Notte has `llm_usage` on `AgentResponse`) |
| Local browser profile (`user-data-dir`) unsupported | [#347](https://github.com/nottelabs/notte/issues/347) | 2025-06-08 → 07-06 | Backlogged, then closed | Persistent profiles are a first-class session option |
| Context overflow on news sites | [#134](https://github.com/nottelabs/notte/issues/134) | 2025-02 | Fixed | Bound page representations; make the token budget configurable (`viewport_expansion`, `clip_tokens` in config.toml) |

### Likes (quotes under 15 words)
- On API simplicity for hybrid flows, the founder: "a very specialized agent to take the very few steps…" ([HN 44829000](https://news.ycombinator.com/item?id=44829000)).
- "Just want to congratulate all involved for the obvious amount of care" ([HN 44831086](https://news.ycombinator.com/item?id=44831086)).
- "excited to try this together with the CDP feature" ([HN 44828576](https://news.ycombinator.com/item?id=44828576)).
- After the version fix: "Works perfect now, thanks" ([#334](https://github.com/nottelabs/notte/issues/334)).
- Counter-signal: "completely failed another hype but actually doesn't work browser agent" ([HN 44828670](https://news.ycombinator.com/item?id=44828670)).

### Unverified (Notte)
- No Reddit discussion was found ([WebSearch](https://www.everydev.ai/tools/notte) and similar directory pages only). Real-world reliability claims (e.g. "86.2% success") are vendor self-reports.
- Whether `client.Session` in TS supports `observe(instructions)` returning action lists. The TS signature returns `Observation` only; I did not read the generated types in depth.
- Whether the cloud AgentFallback has the same `scrape` restriction. The docs say so, but I did not read the remote implementation fully.
- Whether `session_offset` and `perception_type="deep"` are stable. Both are marked experimental or optional in source.

---

## 2.8 LaVague (lavague-ai/LaVague): dormant

### Dormancy evidence
- **Last commit:** `9024bb8`, 2025-01-21, "CI: drop cron schedule run (#633)". The commits before it are 2024-10-15 (docs) and 2024-09-11 (`gh api repos/lavague-ai/LaVague/commits?per_page=3`).
- **Repo state:** not archived, 106 open issues, Apache-2.0 (GitHub API).
- **PyPI:**
  - `lavague` last release 1.1.19 on 2024-08-05.
  - `lavague-core` last release 0.2.35 on 2024-08-22 ([PyPI lavague](https://pypi.org/pypi/lavague/json), [PyPI lavague-core](https://pypi.org/pypi/lavague-core/json)).
- **Unanswered issues:**
  - [#639 "Low Activity"](https://github.com/lavague-ai/LaVague/issues/639) (2025-06-30) has no maintainer reply. A user wrote on 2026-06-23: "Project is dead for a while now".
  - Recent issues with no maintainer response: [#642](https://github.com/lavague-ai/LaVague/issues/642) "Cannot import World Model" (2025-07-22); [#636](https://github.com/lavague-ai/LaVague/issues/636) Playwright example fails (2025-04-26); [#648](https://github.com/lavague-ai/LaVague/issues/648) (2026-03-25); [#654](https://github.com/lavague-ai/LaVague/issues/654) (2026-09-25).
- **#650, re-verified:** "Code execution via eval of LLM output derived from untrusted web-page content (indirect prompt injection to RCE)".
  - Open since 2026-06-12.
  - The only comment is from a non-member (2026-07-27), who confirms that the `DynamicExtractor` path still reaches `eval` on `main` and offers a fix. No maintainer response and no PR merged ([#650](https://github.com/lavague-ai/LaVague/issues/650)).
- **Website:** lavague.ai still serves the 2024 landing page, with a hard-coded "5535" stars count and the same quickstart (curl, 2026-10-14).
- **Pivot announcement:** none found (WebSearch returned nothing relevant). See Unverified.

### API sketch
From the README at `9024bb8` ([README](https://github.com/lavague-ai/LaVague/blob/9024bb8/README.md)):
```python
from lavague.core import WorldModel, ActionEngine
from lavague.core.agents import WebAgent
from lavague.drivers.selenium import SeleniumDriver   # or lavague.drivers.playwright
agent = WebAgent(WorldModel(), ActionEngine(SeleniumDriver(headless=False)))
agent.get("https://huggingface.co/docs"); agent.run("Go on the quicktour of PEFT")
```
The `WorldModel` (planner) emits instructions. The `ActionEngine` dispatches them to sub-engines (Navigation, Python, Navigation-Control) bound to a driver. `agent.demo()` launches a Gradio UI.

### Pain points

| Pain | Reports/links | Dates/versions | Status | API-design lesson |
|---|---|---|---|---|
| LLM-generated code executed directly (Selenium code, then `eval` in extraction) leads to injection and RCE | [#650](https://github.com/lavague-ai/LaVague/issues/650); design issue [#352](https://github.com/lavague-ai/LaVague/issues/352) to move from "arbitrary code exec" to XPath plus action type; HN on DOM-to-code: "Try this at your own risk" ([HN 39708335](https://news.ycombinator.com/item?id=39708335)) | #352 2024-06 (closed); #650 2026-06 (open) | Navigation fixed; extraction still `eval` | Never execute model output. AI primitives should emit typed actions validated against a schema, and extraction should parse to data, not code |
| Generation and execution entangled inside engines; poor observability | [#440](https://github.com/lavague-ai/LaVague/issues/440) (2 reactions) | 2024-07-14 | Open | Separate *plan* (an inspectable action value) from *execute* (one executor); this is what makes record/replay and dry-run possible |
| Install/import breakage (`WorldModel` import, Playwright example fails, `GEMMA_PROMPT` import) | [#642](https://github.com/lavague-ai/LaVague/issues/642), [#636](https://github.com/lavague-ai/LaVague/issues/636), [#180](https://github.com/lavague-ai/LaVague/issues/180) | 2024-04 … 2025-07 | Open | Pin the public import surface; smoke-test examples against released wheels |
| Telemetry on by default (warning at import) | Log in [#636](https://github.com/lavague-ai/LaVague/issues/636) ("Telemetry is turned on. To turn off… LAVAGUE_TELEMETRY") | 2025-04 | Unchanged | Default-off or explicit consent; one documented env var |
| Abandonment with no deprecation notice | [#639](https://github.com/lavague-ai/LaVague/issues/639) | 2025-06 → 2026-06 | Unanswered | Archive or mark deprecated so users and security reporters know where they stand |

### Likes
- "This is so useful!" ([HN 39699555](https://news.ycombinator.com/item?id=39699555)).
- "potential to be a step towards the missing scripting language for graphical interfaces" ([HN 39699377](https://news.ycombinator.com/item?id=39699377)).
- The launch thread reached 378 points ([HN 39698546](https://news.ycombinator.com/item?id=39698546)).

### Unverified (LaVague)
- Any official pivot or shutdown announcement, and what the team works on now.
- Whether the Playwright driver package (`lavague-drivers-playwright`) received later releases. I checked only `lavague` and `lavague-core` on PyPI.

---

## 2.9 HyperAgent (hyperbrowserai/HyperAgent): maintenance mode

### Status
- **Releases:** npm `@hyperbrowser/agent` latest is 1.1.2 (2026-01-07). It has 40 versions since 0.0.1 (2025-04-14); 1.0.x came out in late Nov 2025 and 1.1.0 on 2025-12-16 ([npm](https://registry.npmjs.org/@hyperbrowser/agent)).
- **Commits:** the last code-ish commit is "remove unused init script (#81)" on 2026-01-07. Later commits only touch agent guidelines (2026-01-15, 2026-02-13) and add PR templates (2026-05-11, the last push) (`gh api …/commits`).
- **Issues:** 26 open. A contributor replied to [#88](https://github.com/hyperbrowserai/HyperAgent/issues/88) on 2026-05-28; [#90](https://github.com/hyperbrowserai/HyperAgent/issues/90) (2026-07-26) has no reply.
- **License:** **AGPL-3.0** ("Copyright (c) 2025 S2 Labs Inc.", [H@LICENSE](https://github.com/hyperbrowserai/HyperAgent/blob/a7ec1f4/LICENSE); npm `license: AGPL-3.0`). GitHub shows NOASSERTION.

### API
From the README at `a7ec1f4` ([H@README.md](https://github.com/hyperbrowserai/HyperAgent/blob/a7ec1f4/README.md)) and the types file ([H@src/types/agent/types.ts](https://github.com/hyperbrowserai/HyperAgent/blob/a7ec1f4/src/types/agent/types.ts)):
- **Agent construction:** `new HyperAgent({ llm: {provider: "openai"|"anthropic"|"gemini"|"deepseek", model}, browserProvider?: "Hyperbrowser", customActions?, cdpActions?, debug?, localConfig?, hyperbrowserConfig? })` ([H@src/types/config.ts](https://github.com/hyperbrowserai/HyperAgent/blob/a7ec1f4/src/types/config.ts)).
  - The LLM uses native provider SDKs. Earlier versions used LangChain; see #29.
- **Agent methods:** `agent.executeTask(task, params?)` returns `TaskOutput{taskId, status, steps, output, actionCache?}`. `agent.newPage()` returns `HyperPage`, which **extends Playwright `Page`**, so the full Playwright API is on the same object. `agent.closeAgent()` cleans up.
- **Page methods:**
  - `page.ai(task, TaskParams)` is the multi-step agent. It returns `AgentTaskOutput`, whose `actionCache` is always populated.
  - `page.perform(instruction)` is a single granular action; `page.aiAction` is a deprecated alias.
  - `page.extract(task?, zodSchema?, params?)` returns `z.infer<T>` or `string`.
  - `TaskParams`: `maxSteps`, `outputSchema`, `onStep`, `onComplete`, `enableVisualMode`, `useDomCache`, `debugDir`.
- **Determinism:** `page.runFromActionCache(cache, {maxXPathRetries, debug})` replays by XPath first, with an LLM fallback per step. `agent.createScriptFromActionCache(steps)` emits a standalone TS script.
- **MCP client:** `agent.initializeMCPClient({servers:[{command,args,env}|{connectionType:"sse",sseUrl}]})`.
- **Custom actions:** `{type, actionParams: zodObject, run(ctx, params)}`.
- **Cloud:** `browserProvider: "Hyperbrowser"` plus `HYPERBROWSER_API_KEY`.
- **Experimental CDP-native action layer:** "CDP is still experimental… set `cdpActions: false`" (README).

```ts
import { HyperAgent } from "@hyperbrowser/agent";
import { z } from "zod";
const agent = new HyperAgent({ llm: { provider: "anthropic", model: "claude-sonnet-4-5" } });
const page = await agent.newPage();
await page.goto("https://news.ycombinator.com");
const { actionCache } = await page.ai("open the 'new' tab", { maxSteps: 5 });
const top = await page.extract("top 3 titles", z.object({ titles: z.array(z.string()) }));
await page.runFromActionCache(actionCache, { maxXPathRetries: 2 }); // replay, LLM only on drift
await agent.closeAgent();
```

### Pain points

| Pain | Reports/links | Dates/versions | Status | API-design lesson |
|---|---|---|---|---|
| Transitive LLM-stack breakage (LangChain `messageLike is not iterable`): "hyper agent never works" | [#29](https://github.com/hyperbrowserai/HyperAgent/issues/29) (6 comments) | 2025-05-09, 0.3.1 | Fixed next day | Own the model-call layer or pin it; HyperAgent later moved to native SDKs (README "native SDKs") |
| Infinite loop when the DOM read returns null after navigation | [#48](https://github.com/hyperbrowserai/HyperAgent/issues/48) (4 comments) | 2025-09-26 → 10-07, fixed in 0.9.0 ([PR #50](https://github.com/hyperbrowserai/HyperAgent/pull/50)) | Fixed | Every agent loop needs a hard step/time budget and typed "page gone" errors |
| `page.extract()` hard-coded `maxSteps: 2`, silently returned empty `output` | [#54](https://github.com/hyperbrowserai/HyperAgent/issues/54) | 2025-10-07 → 10-27 | Param added | Hidden budgets in AI verbs must be exposed and must fail loudly (typed "budget exhausted"), not return empty |
| Gemini models don't work | [#14](https://github.com/hyperbrowserai/HyperAgent/issues/14) (2 reactions, top by reactions) | 2025-04-22 | Still open | Provider matrix tests in CI |
| Deterministic caching requested | [#5](https://github.com/hyperbrowserai/HyperAgent/issues/5): "big enough feature for us to migrate off Browser Use" | 2025-04-20 | Shipped as `actionCache` (README) but the issue was never closed | Record/replay is a migration driver; ship it early |
| Navigation/redirect events not surfaced | [#20](https://github.com/hyperbrowserai/HyperAgent/issues/20) | 2025-04-24 | Open | Expose page lifecycle events to AI steps |
| Recording video stuck "pending" under parallel sessions | [#88](https://github.com/hyperbrowserai/HyperAgent/issues/88) | 2026-05-28 | Open | Artifacts need terminal states and timeouts |

**Likes:** no independent quotes found. HN has no HyperAgent-library threads (Algolia hits are other "hyperagents"), and the Reddit search found nothing. The only third-party praise is #5's migration remark above.

### Unverified (HyperAgent)
- Python parity. A Hyperbrowser Python SDK exists, but I did not verify a Python HyperAgent.
- Whether `executeTask` accepts `outputSchema` at 1.1.2 exactly as the README "Output Schema Definition" section shows (README L236-L265; not read in source).
- Whether the project is officially deprioritised. Only commit and release cadence suggest it.

---

## 2.10 Magnitude browser agent: dormant; the name was reused for another product

### Repurposing and dormancy evidence
- **`magnitudedev/magnitude` is now a different project.**
  - Description: "Open source inference engine that tunes its kernels on your hardware…"
  - Repo **created 2026-06-12**, 6,724 stars, desktop-app releases `desktop-update-acceptance/<sha>/50.8.x` (GitHub API).
  - Its issues (MLX, GGUF, model loading) are all inference topics. The supplied TSVs belong to this repo.
- **The browser-agent library lives at [magnitudedev/browser-agent](https://github.com/magnitudedev/browser-agent):** "Open-source, vision-first browser agent", created 2025-03-20, 4,134 stars, not archived, 33 open issues, **last push 2026-02-08**.
  - The last commit, `f1b587c` "Version Packages" (2026-02-08), released `agent.pause()`/`resume()` and a token-counting fix.
  - The commit before that is from 2025-10-16 (`gh api …/browser-agent/commits`).
- **npm:** `magnitude-core` latest 0.3.1 and `magnitude-test` latest 0.3.13, both published **2026-02-08**. The previous releases were 2025-09-25 ([npm core](https://registry.npmjs.org/magnitude-core), [npm test](https://registry.npmjs.org/magnitude-test)).
  - The npm metadata `repository` still points to `github.com/magnitudedev/magnitude`, which now resolves to the inference engine.
  - The publish `gitHead` (`f1b587c`) exists in `browser-agent`. This indicates the original repo was renamed and the name reused (rename date unverified).
- **Domains:**
  - `magnitude.run` returns HTTP 308 to `magnitude.dev`, which serves the inference engine ("Run open models as fast as your hardware allows").
  - `docs.magnitude.run` **does not resolve** (empty `dig`; curl status 000, 2026-10-14). Every docs link in the browser-agent README is therefore dead. The docs survive only as MDX in the repo (`docs/` at `f1b587c`).
- **Recent issues have zero replies:** [#146](https://github.com/magnitudedev/browser-agent/issues/146) (2025-11-28, "example test fails… 404"), [#152](https://github.com/magnitudedev/browser-agent/issues/152) (2026-02-26), [#154](https://github.com/magnitudedev/browser-agent/issues/154) (2026-03-17), [#155](https://github.com/magnitudedev/browser-agent/issues/155) (2026-04-20, command injection via config `spawn(shell:true)`).

### API
At `f1b587c`, from [M@docs/reference/browser-agent.mdx](https://github.com/magnitudedev/browser-agent/blob/f1b587c/docs/reference/browser-agent.mdx) and [M@…/agent/browserAgent.ts](https://github.com/magnitudedev/browser-agent/blob/f1b587c/packages/magnitude-core/src/agent/browserAgent.ts):
- **Start:** `startBrowserAgent({ url?, llm?, browser?: {launchOptions|contextOptions|cdp…}, virtualScreenDimensions?, narrate?, actions?, prompt? })` returns `Promise<BrowserAgent>`.
- **`agent.act(taskOrSteps: string | string[], {prompt?, data?, memory?})`** returns `Promise<void>`.
  - It returns **no result object**; failures throw, and plan failures surface as "misalignment" ([M@…/agent/index.ts#L32-L240](https://github.com/magnitudedev/browser-agent/blob/f1b587c/packages/magnitude-core/src/agent/index.ts#L32-L240)).
- **Other verbs:** `agent.extract(instructions, zodSchema)` returns `z.infer<T>`; it reads the DOM/HTML content, not vision. `agent.query(query, schema)`, `agent.nav(url)`, `agent.page` / `agent.context` (raw Playwright), `pause()`, `resume()`, `stop()`.
- **Design:** vision-first. A visually grounded model emits pixel coordinates, with Claude Sonnet recommended. LLM providers are `anthropic`, `google-ai`, `vertex-ai`, `openai`, `openai-generic`, `aws-bedrock`, `azure-openai` and `moondream` ([M@docs/reference/llm-providers.mdx](https://github.com/magnitudedev/browser-agent/blob/f1b587c/docs/reference/llm-providers.mdx)).
- **Test runner:** `magnitude-test`, with `test('title', {url}?, async (agent) => { await agent.act(…); await agent.check("…") })` and `test.group(…)` ([M@docs/reference/test-declaration.mdx](https://github.com/magnitudedev/browser-agent/blob/f1b587c/docs/reference/test-declaration.mdx)).
  - `agent.check()` is an LLM-judged visual assertion ([M@…/magnitude-test/src/agent/index.ts#L52-L64](https://github.com/magnitudedev/browser-agent/blob/f1b587c/packages/magnitude-test/src/agent/index.ts#L52-L64)).
  - The package README still shows the older chained `.step().data().check()` API ([M@packages/magnitude-test/README.md](https://github.com/magnitudedev/browser-agent/blob/f1b587c/packages/magnitude-test/README.md)). Docs drift.
- **Caching:** "Deterministic runs via native caching system *(in progress)*". It never shipped ([M@README.md](https://github.com/magnitudedev/browser-agent/blob/f1b587c/README.md)).

```ts
import { startBrowserAgent } from 'magnitude-core';
import { z } from 'zod';
const agent = await startBrowserAgent({ url: 'https://news.ycombinator.com',
  llm: { provider: 'anthropic', options: { model: 'claude-sonnet-4-5' } } });
await agent.act('Open the "new" tab');                         // void; throws on failure
const titles = await agent.extract('first 3 titles', z.array(z.string()));
await agent.page.screenshot({ path: 'hn.png' });               // raw Playwright
await agent.stop();
```

### Pain points
Ranked by comments and reactions (GitHub issues API).

| Pain | Reports/links | Dates/versions | Status | API-design lesson |
|---|---|---|---|---|
| Opaque failure ("Misalignment: Could not align click action: Invalid URL") | [#25](https://github.com/magnitudedev/browser-agent/issues/25) (10 comments; maintainer: "isn't very helpful") | 2025-05 | Closed | Errors from AI verbs need the step, the attempted action, the target and the screenshot |
| Waiting on dynamic content (spinners) fails | [#7](https://github.com/magnitudedev/browser-agent/issues/7) (9 comments, 2 reactions) | 2025-04 → 06 | Closed | Explicit wait primitives alongside heuristic network/visual stability |
| Model-output parse failures (`BamlValidationError`) during `check` | [#102](https://github.com/magnitudedev/browser-agent/issues/102) (4 comments) | 2025-07, Sonnet 4 | Open | Retry structured output with repair; surface a typed "model output invalid" error |
| Quickstart doesn't work (README scenario, example project, ESM init, BAML 404) | [#52](https://github.com/magnitudedev/browser-agent/issues/52), [#98](https://github.com/magnitudedev/browser-agent/issues/98), [#89](https://github.com/magnitudedev/browser-agent/issues/89) (2 reactions), [#146](https://github.com/magnitudedev/browser-agent/issues/146) | 2025-06 → 11 | Mixed; #89 and #146 open | Golden-path smoke test in CI against published packages |
| Need Playwright fixtures / auth state in tests | [#28](https://github.com/magnitudedev/browser-agent/issues/28), [#35](https://github.com/magnitudedev/browser-agent/issues/35) (Playwright `expect`) | 2025-05 | Closed via config / `agent.page` | Keep raw Playwright in reach, per test, not only through global config |
| Machine-readable results for CI | [#11](https://github.com/magnitudedev/browser-agent/issues/11) (top by reactions, 3) | 2025-04 | Open | Structured run reports (JSON/JUnit) are table stakes for test runners |
| Cost/speed vs plain Playwright; nondeterministic tests | [HN 44391985](https://news.ycombinator.com/item?id=44391985) "must 10000x the cost and speed"; [HN 43796478](https://news.ycombinator.com/item?id=43796478) "isn't this the ultimate non-deterministic test?" | 2025-04/06 | Caching never shipped | Cache/compile AI steps to deterministic replays; report cost per step |

### Likes
- "this one has a super easy setup and appears to Just Work" ([HN 44392057](https://news.ycombinator.com/item?id=44392057)).
- "a browser agent that doesn't panic at the sight of a canvas" ([HN 44390403](https://news.ycombinator.com/item?id=44390403)).
- "pretty much exactly what I was going to build" ([HN 43798281](https://news.ycombinator.com/item?id=43798281)).

### Unverified (Magnitude)
- The exact date of the rename from `magnitude` to `browser-agent` and any team announcement about the pivot.
- Whether `extract` falls back to vision. It reads HTML via `getFullPageContent` in source (browserAgent.ts L132-L140); I did not read further.
- The 94% WebVoyager claim (vendor benchmark, [webvoyager repo](https://github.com/magnitudedev/webvoyager)).

---

### Takeaways from Notte, LaVague, HyperAgent and Magnitude
1. **Typed action values, never code.** Notte's `observe(instructions)` returns `ClickAction`/`FillAction` objects that you pass to `execute`. That beats LaVague-style generated code, which led to the #352 redesign and the #650 RCE.
2. **One result type with `success` plus an opt-in raise.** Notte's `raise_on_failure` (with typed overloads), HyperAgent's `TaskOutput` and Magnitude's `void` + throw are three different answers. Users hit the opaque-error paths most (#25, #545).
3. **Determinism artifacts as first-class outputs:**
   - HyperAgent returns `actionCache` from every `page.ai()` and replays it with an LLM fallback per step.
   - Notte converts agent to code to a deployed Function, and wraps scripts in `AgentFallback`.
   - Magnitude promised caching and never shipped it, and HN critics focused exactly on cost and nondeterminism.
4. **Raw page on the same object.** HyperAgent's `HyperPage extends Page`, Magnitude's `agent.page` and Notte's `session.page` all support the request to keep raw Playwright in reach (Magnitude #28/#35).
5. **Hidden budgets and loops** (HyperAgent #54 `maxSteps:2`, #48 infinite loop) must be explicit parameters with typed exhaustion errors.
6. **No global side effects at import, and telemetry off by default or clearly documented.** See Notte #509 and the import-time telemetry warnings in Notte and LaVague.
7. **Wire-contract tolerance:** ignore unknown response fields, and generate every language binding from one schema (Notte #551, #804, and TS casing drift).
8. **Abandonment risk is real.** Two of the four peers are dormant (LaVague since 2025-01, Magnitude since 2026-02, its name reused). Stable install paths and docs hosted with the code matter, given that docs.magnitude.run is gone.

## 3. Cross-library comparison

Cells summarize §2; sources are linked there. "—" means none found in the sources read, not proof of absence.

### 3.1 Shape, lifecycle and configuration

| Dimension | Stagehand 4.2 | Browser Use 0.13 | Midscene 1.15 | agent-browser 0.39 | Skyvern 1.0.56 | Playwright MCP 0.0.83 / CLI 0.1.22 | Notte 1.9.7 |
|---|---|---|---|---|---|---|---|
| Languages | TS, Python, Go from one protocol | Python; TS only as unofficial ports | TS; Python only as community ports | CLI + `--json`, MCP; no library since 0.20.8 | Python, TS (TS lacks ~15 page features) | MCP tools, CLI; core Playwright in 4 languages | Python (local + cloud); TS cloud-only |
| Who drives | your code or your external agent (`agent()` removed) | built-in agent loop | your code; `aiAct` has an inner loop | external agent | code, built-in agent, or server workflow | external agent | code, built-in agent, or fallback agent |
| Entry | browser factory → `Stagehand.create({ browser })` | `Agent(task, llm, browser, tools, ~60 kwargs)` → `run()` | `new PlaywrightAgent(yourPage, opts)` | `agent-browser open <url>` (daemon per session) | `Skyvern(api_key)` / `Skyvern.local()` → `launch_*_browser()` → `get_working_page()` | `npx @playwright/mcp` / `playwright-cli open` | `notte.Session(...)` / `NotteClient().Session(...)` |
| Close | `stagehand.close()` then `browser.close()` | `run()` kills the browser unless `keep_alive=True`; `close()` only stops | `agent.destroy()` (does not close page) | `close`, idle 1 h | `browser.close()` + `skyvern.aclose()` | `browser_close` / `close`, idle 1 h | context manager exit |
| Scoped disposal (`await using` / `async with`) | — | — | — | n/a | — | n/a | **yes**: `with`/`async with`; TS `.use(fn)` |
| Configuration | options object; **reads no env vars** | kwargs (unknown kwargs silently ignored), 77 `TIMEOUT_*` env vars, `config.json` | **env vars first** (`MIDSCENE_MODEL_*`); `modelConfig` object replaces env | file < env < flags; published JSON Schema | constructor (remote); `.env` (self-host) | flags, `PLAYWRIGHT_MCP_*` env, JSON config | kwargs + TOML config + env |
| Defaults that bite | server cache on (sends `%var%` values); no-model → Gateway; no auto-wait; Zod minor pinned | telemetry with task/URLs on; judge LLM call on; paid default LLM; extensions downloaded; `max_steps=500` | reports on; new tabs forced into the same tab; `modelConfig` replaces env (no merge); preferred model-reply language is Chinese in the Asia/Shanghai time zone | Chrome revision mismatch (pre-0.20), Windows shims | `timeout` in minutes, seconds and ms on different calls; `run_task` returns before completion | headed by default; one profile per workspace ("browser already in use") | telemetry on; `nest_asyncio` patch on import |

### 3.2 Verbs, types and determinism

| Dimension | Stagehand | Browser Use | Midscene | agent-browser | Skyvern | Playwright MCP/CLI | Notte |
|---|---|---|---|---|---|---|---|
| Single action from NL | `act(str \| Action)` | `run()` only; actor `get_element_by_prompt` | `aiAct`, `aiTap/aiInput/…` | — (`chat` is a loop) | `page.act(prompt)`; `click(sel, prompt=)` | — (model picks tools) | `execute(...)` after `observe` |
| Observe/locate | `observe()` → `Action[]` (XPath, method, args) | — | `aiLocate` → rect/center | `snapshot` refs `@eN` | `locator(prompt=)` → lazy `AILocator` | `browser_snapshot` refs `eN`, `find` | `observe(instructions)` → typed action list |
| Extract | `extract(instr, zod \| Pydantic)` → typed `data` | `output_model_schema` (Pydantic) on the run; actor `extract_content(prompt, Model)` | `aiQuery<T>(prose shape)` (unchecked cast) | `get text`, `eval` | `extract(prompt, JSON-Schema dict)` → `dict\|list\|str\|None` | `browser_evaluate`, `run_code_unsafe` | `scrape(response_format=Model)` |
| Assert / validate | — | judge (LLM) after run | `aiAssert`, `aiWaitFor`, `aiBoolean` | `is visible`, `wait` | `validate(prompt) -> bool` | `browser_verify_*` (testing caps) | judge on agent completion |
| Result envelope | `{ data, metadata: { usage, cache, actionId } }` | `AgentHistoryList[T]` (fat history) | raw value / `void` | `{ success, data, error, code? }` | `TaskRunResponse{ status, output, failure_reason, … }` | Markdown sections, `isError` | `ExecutionResult{ success, … }`, `AgentResponse` |
| Failure style | mixed: `success:false` **and** untyped exceptions | errors become history entries; run returns | exceptions (`TaskExecutionError` with code) | `success:false` + exit 1 | HTTP exceptions; run failure is a `status` value; `extract` → `None`, `validate` → `False` | `isError` content with re-observe hint | `raise_on_failure` flag with typed overloads |
| Cancellation | per-call `timeout`; no AbortSignal | `stop()` between steps, pause/resume | `abortSignal` on `aiAct` | `close` | separate `cancel_run`; client timeout does not cancel server | — | `stop()` (cloud agent) |
| Cache / replay | server-only cache (Browserbase), keyed on instruction + page + options; `act(Action)` replays without inference; `selfHeal` opt-in | `rerun_history` with 5-level element matching, still calls an LLM for extract and summary; beta drops replay | **local YAML** plan + XPath caches, read-write / read-only / write-only | deterministic refs; no cache | selector-first, AI fallback per verb; **server-side** per-block code cache (`run_with="code"`) | codegen of normalized Playwright code after every action, in 4 languages | agent → code → deployed Function; `AgentFallback` around scripts; `execute_saved_actions` |
| Raw page | own Playwright-like `Page`/`Locator` (no Playwright interop) | actor `Page`/`Element` (CDP, "legacy", subset) | **your Playwright/Puppeteer page** | CLI commands | **Playwright `Page` + AI methods on the same object** | `run_code_unsafe(page => …)` | `session.page` (Playwright) |
| Tabs | implicit "active page" = focused tab; `{ page }` to pin | agent `switch` action; actor `get_pages()` | `forceSameTabNavigation: true`; `PlaywrightBrowserAgent` for multi-page | follows active tab; `--pin-tab` | `get_working_page()` = last tab | `browser_tabs` | `switch_tab` action |
| Agent extension | n/a (WebMCP page tools) | `@tools.action(desc)`; params injected **by name** | `customActions` | plugin protocol (stdio) | — (server registry) | — (host side) | `tools=[BaseTool]` |
| Hooks | `logging.onLog` only | `on_step_start/end`, `register_*` callbacks, event bus | `onTaskStartTip`, `waitForActionReady`, `onLLMUsage` | — | webhooks (HMAC), timeline | session log file | log streaming |
| Budget | none; `metadata.usage` + `metrics()` | `max_steps`, `max_failures`; cost opt-in | `replanningCycleLimit`; `metrics` | `--max-output` | `max_steps`, `max_elapsed_time_minutes`; no cost in typed result | output size caps | `max_steps` |
| Model config | `{ modelName: "provider/model", apiKey }`; Gateway; BYO `generate` callback | `ChatOpenAI(...)`-style objects; `BaseChatModel` protocol | env vars + mandatory `MODEL_FAMILY`; OpenAI-compatible only | `chat` via AI Gateway only | enumerated `LLM_KEY` registry (self-host); `model` dict per call | none (host's model) | LiteLLM `provider/model` strings |
| Observability | OTel spans across SDK/runtime, JSON logs, Browserbase recordings | logs, GIF, conversation dumps, Laminar, HAR/video | HTML replay report per agent (default on) | dashboard, video, trace, HAR | `app_url`, recording, ~40 artifact types, timeline | session.md log, traces, video receipts, live dashboard | replay video, live viewer |
| Breaking-change pace | 3 majors in 22 months, each a rewrite; Python had 3 unrelated shapes | ~6 large breaks in 15 months under 0.x | v1.0 renames **with aliases and deprecation warnings** | Node→Rust rewrite; library API removed | 5 renames in 7 months, 2 after 1.0 | still 0.0.x; ~15 tool/flag renames | roughly weekly releases; wire-compat policy |

## 4. Common patterns and divergences

### 4.1 What nearly everyone converged on

1. **Three AI verbs plus a fourth that is often missing.** `act` (one action from an instruction), `observe`/`locate` (return candidate targets without acting), `extract`/`query` (typed data) appear in Stagehand, Skyvern, Notte, Midscene, HyperAgent and Magnitude, and in Playwright 1.58's hidden `PageAgent` (`perform`, `expect`, `extract`). A boolean **assert/validate** verb exists in Skyvern (`validate`), Midscene (`aiAssert`, `aiBoolean`), Magnitude (`check`) and Playwright's prototype (`expect`), but not in Stagehand.
2. **The raw page stays in reach.** Every library exposes a Playwright-grade page. The best-received designs put the AI verbs on the **same object** as the raw page (Skyvern's `SkyvernPage` is a Playwright `Page` plus AI methods; HyperAgent's `HyperPage extends Page`; Midscene wraps the page you already hold). Stagehand v4 owns its page type and refuses foreign ones; Browser Use's actor page is a late add-on now filed under "legacy".
3. **The AI step returns something replayable.** Stagehand `observe()` returns `Action{selector, method, arguments}` that `act()` replays without inference; Notte returns typed action objects; Playwright MCP emits normalized Playwright code after each action; HyperAgent returns an `actionCache`; Midscene writes plan and XPath caches. Users ask for this more than for any other feature (§6, pain 1).
4. **Selector first, AI as fallback.** Skyvern's `click(selector, prompt=…, ai="fallback")` is its documented production default; Midscene's `xpath` option is tried before cache and AI; Notte's `AgentFallback` wraps scripted steps; HyperAgent's `runFromActionCache` retries XPath before the LLM; Stagehand's `selfHeal` re-infers once.
5. **Compact, addressable observations.** Accessibility snapshots with short refs (`@e2`, `e12`, `f1e3` for frames) are the shared perception format of agent-facing tools. Playwright moved post-action snapshots to files with a `find` grep after ten months of token complaints; agent-browser adds diffs.
6. **The built-in agent moves outside.** Stagehand removed `agent()` (v4, 2026-08); Microsoft built `page.agent()` in Playwright 1.58 and deleted it ("chore: rip perform", 2026-03-11); Midscene retired its MCP servers for CLI plus skills; agent-browser removed its library API. Libraries that keep a loop (Browser Use, Skyvern, Notte) push "the agent writes code once" (Browser Use Cloud scripts and Browser Harness, Skyvern code caching, Notte agent→Function).
7. **Model layer = provider string or protocol, plus an escape hatch.** Stagehand `provider/model` + a `generate` callback; Browser Use's `BaseChatModel` protocol; Notte's LiteLLM strings; Midscene's `createOpenAIClient`. Vendors also add a paid gateway as the zero-config default (Browserbase Model Gateway, `ChatBrowserUse`, Vercel AI Gateway for agent-browser `chat`).
8. **Per-call usage reporting.** Stagehand `metadata.usage` on every result plus `metrics()`; Midscene `onLLMUsage` and `metrics.byIntent`; Browser Use `history.usage` (opt-in); Notte `llm_usage`. Only Skyvern's typed SDK result lacks cost.
9. **Async-only APIs.** Every library is async-first; only Notte offers real sync and async variants, Browser Use has `run_sync` (unusable inside a running loop).
10. **No scoped disposal.** Apart from Notte, none ships `await using` / `async with`; docs teach nested `try/finally`, and close semantics differ (who closes the browser, whether `close` kills it).

### 4.2 Where they diverge (the real design choices)

| Choice | Option A (who) | Option B (who) | Evidence on the trade-off |
|---|---|---|---|
| **Where AI verbs live** | on an AI runtime object, page passed as option (`stagehand.act(x, { page })`) | on the page itself (`page.act`, Skyvern, HyperAgent; Stagehand v2) | Stagehand moved page→instance in v3; its implicit "active page" default makes concurrency subtle. Page-level verbs read naturally and remove the target ambiguity. |
| **Schema input** | native types: Zod, Pydantic, Go generics (Stagehand, Notte, HyperAgent, Magnitude) | JSON Schema dict (Skyvern) or prose shape strings (Midscene) | Native types give typed results but couple the public type surface to a library's version (Stagehand's Zod 3→4 breakage, 4 issues, ~3 months). JSON Schema dicts are portable but return untyped `dict` that users cast. Prose shapes are unchecked. |
| **Failure model** | values: status enum, `success:false`, history entries (Skyvern runs, Browser Use, Notte `raise_on_failure=False`) | exceptions (Midscene, Magnitude) | Mixed models hurt most: Stagehand returns `success:false` for some failures and throws untyped errors for others, and its docs branch on `message.includes(...)`. Notte's explicit `raise_on_failure` with typed overloads is the cleanest. |
| **Cache location** | local files the user can commit (Midscene, HyperAgent, Stagehand v3 `cacheDir`) | vendor server (Stagehand v4, Skyvern code cache, Browser Use Cloud scripts) | Users push back on server-only caches ("why… couldn't just be a local file", HN). |
| **Config source** | explicit objects only, no env (Stagehand v4, Skyvern remote) | env vars first (Midscene, Browser Use, Skyvern self-host) | Env-first produced the most-commented Midscene threads (provider setup, #221 25c, #309 24c); Stagehand v4 deliberately reads none. |
| **Built-in agent** | none (Stagehand v4, Playwright, agent-browser) | first-class (Browser Use, Skyvern, Notte) | One HN user asked why `agent()` could not stay "alongside the new primatives"; no GitHub complaint found. Embedded-loop projects have the false-success and hang reports. |
| **Driver** | own CDP driver (Stagehand understudy, Browser Use cdp-use, agent-browser Rust) | Playwright underneath (Skyvern, Midscene, Notte, HyperAgent) | Own drivers lose auto-wait, actionability and `getBy*` (Stagehand's migration guide ships hand-written retry loops; #2744: 13 of 15 scripts blocked). Playwright-based ones inherit them. |
| **Strictness** | reject unknown options (Stagehand Zod-parsed options, Browser Use `BrowserSession` `extra='forbid'`) | silently ignore (Browser Use `Agent(**kwargs)`, `BrowserProfile` `extra='ignore'`) | Silent ignore caused #3535 (a passed `context` swallowed). |

## 5. What users like (ranked)

Ranked by independent reports across HN, blogs, Stack Overflow and issue comments (Appendix A and the per-library sections). Quotes are under 15 words.

| # | Liked quality | Independent reports | Representative evidence |
|---|---|---|---|
| 1 | **Generate the automation with AI, then own deterministic, reviewable code** (as a wish and, where it exists, as praise) | ≥12 across libraries, 2024–2026 | Workflow Use: "I love the use of reusable non-LLM code" ([joshstrange, HN](https://news.ycombinator.com/item?id=44015145)); playwright-cli to explore, then a generated script ([skapadia, HN](https://news.ycombinator.com/item?id=47791237)); agent-browser then a deterministic IIFE, "18x cost-saving" ([miteshashar, HN](https://news.ycombinator.com/item?id=48019263)); Director "writes Stagehand code on the fly" ([idopmstuff, HN](https://news.ycombinator.com/item?id=46112951)); Microsoft ISE converted Stagehand runs to Playwright tests ([ISE blog](https://devblogs.microsoft.com/ise/app-modernization-llm-driven-ui-tests-hve/)) |
| 2 | **Small, clean primitives that mix with ordinary code** | Stagehand 6 (+1 adjacent); Notte, Magnitude, Skyvern founders argue the same | "The act, extract, and observe methods are super clean." ([HN](https://news.ycombinator.com/item?id=42644964)); "actually a useful abstraction here" ([HN](https://news.ycombinator.com/item?id=42638393)); "The API surface is dead simple." ([dev.to](https://dev.to/stevengonsalvez/browser-tools-for-ai-agents-part-2-the-framework-wars-browser-use-stagehand-skyvern-4gn)); per-step control, `observe()` as preview, Zod schemas ([HAI Substack](https://himanshuai.substack.com/p/stagehand-in-practice), semi-promotional) |
| 3 | **Token-efficient, compact observations and CLIs** | agent-browser 10, playwright-cli 5 | "Your agent sees three lines instead of three thousand nodes." ([paddo.dev](https://paddo.dev/blog/agent-browser-context-efficiency/)); 5,455 vs 31,117 characters against Playwright MCP ([Pulumi](https://www.pulumi.com/blog/self-verifying-ai-agents-vercels-agent-browser-in-the-ralph-wiggum-loop/)); "Playwright-cli (not mcp) is so much more efficient." ([HN](https://news.ycombinator.com/item?id=47500807)); later near-parity once MCP snapshots moved to files ([Checkly](https://www.checklyhq.com/blog/mcp-vs-cli-token-efficiency/)) |
| 4 | **Any model, including local ones** | Browser Use 3, Stagehand 2, Midscene 1 | "browser-use is compatible with more models" ([HN](https://news.ycombinator.com/item?id=43174017)); "you can do pretty much anything while being provider agnostic" ([HN](https://news.ycombinator.com/item?id=43639238)); "you can set the model and UI-TARS works" ([HN](https://news.ycombinator.com/item?id=48030492)) |
| 5 | **Fast start, easy to embed** | Browser Use 4, playwright-cli 2, Magnitude 1 | "a lot easier to embed in other programs than Claude's driver" ([HN](https://news.ycombinator.com/item?id=49346302)); "essentially zero setup" ([HN](https://news.ycombinator.com/item?id=48029825)); "super easy setup and appears to Just Work" ([HN](https://news.ycombinator.com/item?id=44392057)) |
| 6 | **Familiar Playwright API under the AI** | 3 | "All of the good LLMs appear to know the API really well." ([simonw, HN](https://news.ycombinator.com/item?id=45644867)); "it's an AI layer built on top of Playwright" ([HN](https://news.ycombinator.com/item?id=45418483)); Midscene maintainers send selector requests back to the raw page ([#760](https://github.com/web-infra-dev/midscene/issues/760)) |
| 7 | **Seeing what the AI did** | 3 | Skyvern per-action screenshots and recording ([GIGAZINE](https://gigazine.net/gsc_news/en/20241124-skyvern-ai/)); "I especially like the observability around the workflow and the steps" ([HN](https://news.ycombinator.com/item?id=42742916)); Midscene's HTML replay report and Playwright's per-action "Ran Playwright code" section |
| 8 | **Reusing a real, logged-in browser** | 2–3 | agent-browser with `--auto-connect` ([HN](https://news.ycombinator.com/item?id=47391198)); thin Browser Harness "small, inspectable, and self-extending" ([HN](https://news.ycombinator.com/item?id=47927379)); profile reuse as the answer to hard logins (earlier note) |
| 9 | **Caching that removes LLM calls on repeat runs** | 2 (+ demand in pain 1) | "I like the caching idea" ([HN](https://news.ycombinator.com/item?id=49763632)); Midscene cache "approaching native speed" (comparison site, via search summary, **unverified wording**) |
| 10 | **Escape hatch to raw code** | 1 strong (+25 reactions) | Playwright MCP's top issue asked for `evaluate` ([mcp#424](https://github.com/microsoft/playwright-mcp/issues/424), +25); "Writing just a big 'Thank you' for implementing this feature!" (same thread) |

## 6. Pain points (ranked by evidence weight)

Weight = reactions on the lead issues + number of independent reports (issues, forum) + how many libraries show it. Status as of 2026-10-14. Each row ends with the API-design lesson it implies for Collie and Foxhound.

| # | Pain | Evidence (r = reactions, c = comments) | Status | API-design lesson |
|---|---|---|---|---|
| 1 | **Non-determinism and per-run cost: "let me replay without the model / export the script"** | Stagehand [#389](https://github.com/browserbase/stagehand/issues/389) (16r/14c, open since 2025-01), [#404](https://github.com/browserbase/stagehand/issues/404), [#255](https://github.com/browserbase/stagehand/issues/255), [#1555](https://github.com/browserbase/stagehand/issues/1555) (absolute XPaths "a dealbreaker"); Browser Use [#763](https://github.com/browser-use/browser-use/issues/763) (11r, bounty), [#943](https://github.com/browser-use/browser-use/issues/943) (11r), [#123](https://github.com/browser-use/browser-use/issues/123); Playwright MCP [#123](https://github.com/microsoft/playwright-mcp/issues/123) (16r), [#519](https://github.com/microsoft/playwright-mcp/issues/519) (23c); Midscene [#2240](https://github.com/web-infra-dev/midscene/issues/2240) (open); HyperAgent [#5](https://github.com/hyperbrowserai/HyperAgent/issues/5) ("big enough feature… to migrate off Browser Use"); Magnitude's cache never shipped; ≥12 independent forum voices (Appendix A) | Partial: Stagehand server-only cache; Skyvern server-side code cache; Midscene local cache; Playwright per-action codegen; Browser Use rerun still calls an LLM and the Rust beta drops replay | Every AI verb returns a **local, serializable, versioned artifact** (resolved target with stable selector, normalized code, or plan step) that replays with zero inference and falls back to AI only on a detected drift. Keep it local-first and diffable; never make the cache a cloud-only feature. |
| 2 | **False success: "done" that did not happen** | Browser Use [#6031](https://github.com/browser-use/browser-use/issues/6031) (8 known-answer cases), [#5955](https://github.com/browser-use/browser-use/issues/5955), [#5361](https://github.com/browser-use/browser-use/issues/5361), [#5438](https://github.com/browser-use/browser-use/issues/5438), [#5964](https://github.com/browser-use/browser-use/issues/5964) (all open), beta [#5129](https://github.com/browser-use/browser-use/issues/5129); Stagehand [#239](https://github.com/browserbase/stagehand/issues/239), [#2486](https://github.com/browserbase/stagehand/issues/2486); agent-browser [#2083](https://github.com/vercel-labs/agent-browser/issues/2083), [#2079](https://github.com/vercel-labs/agent-browser/issues/2079); Skyvern ([HN](https://news.ycombinator.com/item?id=42362929)), `validate` returns `False` on server failure; 5 independent forum reports | Open everywhere; Browser Use docs now say verify independently | Separate **stopped**, **agent-reported success** and **verified** in the result type; actions return typed failures, never success text containing "not found"; ship a deterministic `check`/assert verb and postconditions alongside the LLM one. |
| 3 | **Breaking churn and stale examples** | Stagehand: three rewrites in 22 months (§2.1 timeline), `agent()` removal questioned on HN, Zod 4 break ([#1029](https://github.com/browserbase/stagehand/issues/1029), [#938](https://github.com/browserbase/stagehand/issues/938), [#1098](https://github.com/browserbase/stagehand/issues/1098)), link extraction lost in 2.0 ([#651](https://github.com/browserbase/stagehand/issues/651), 18c), v3 persistence ([#1250](https://github.com/browserbase/stagehand/issues/1250)), Python reshaped three times; only **28.8%** of npm downloads on v4 two months after release. Browser Use: 0.2.1/0.3.3/0.6.0/0.7.0 breaks ([#1815](https://github.com/browser-use/browser-use/issues/1815) 21c, [#2134](https://github.com/browser-use/browser-use/issues/2134) +9, [#2242](https://github.com/browser-use/browser-use/issues/2242) 18c), issue volume +52% after 0.3.3. Skyvern: 5 renames in 7 months. Playwright MCP: ~15 renames, users pinning old versions ([#769](https://github.com/microsoft/playwright-mcp/issues/769)). Forum: 4 Stagehand + downstream ADR deferring v4 | Ongoing; Midscene is the counter-example (aliases with deprecation warnings) | Freeze a **small public surface** early and keep engine rewrites behind it; version the wire protocol separately from SDKs; rename with aliases and warnings for a cycle; test documentation snippets against released packages (Notte now does). |
| 4 | **Model/provider breadth and structured-output fragility** | Stagehand [#183](https://github.com/browserbase/stagehand/issues/183) (19r), [#1645](https://github.com/browserbase/stagehand/issues/1645) (16r, open), [#307](https://github.com/browserbase/stagehand/issues/307) (14r), [#790](https://github.com/browserbase/stagehand/issues/790) (7r), 47 issues mention "local model"; Browser Use [#2605](https://github.com/browser-use/browser-use/issues/2605) (19r), [#442](https://github.com/browser-use/browser-use/issues/442) (37c), [#349](https://github.com/browser-use/browser-use/issues/349) (19c); Midscene [#221](https://github.com/web-infra-dev/midscene/issues/221) (25c), [#309](https://github.com/web-infra-dev/midscene/issues/309) (24c), [#339](https://github.com/web-infra-dev/midscene/issues/339), [#2831](https://github.com/web-infra-dev/midscene/issues/2831); Skyvern [#991](https://github.com/Skyvern-AI/skyvern/issues/991) (8r, top); Notte [#545](https://github.com/nottelabs/notte/issues/545) | Improving; Stagehand relies on a BYO callback, Midscene on mandatory `MODEL_FAMILY` | Accept `provider/model` strings **and** an OpenAI-compatible base URL **and** a callback; declare model capabilities instead of sniffing names; the engine owns structured output with a strategy ladder (native schema → tool call → prompt + repair) and a typed error naming model and strategy. |
| 5 | **Browser acquisition, attach and lifecycle fragility** | Browser Use [#839](https://github.com/browser-use/browser-use/issues/839) (14r/41c), [#1020](https://github.com/browser-use/browser-use/issues/1020) (14r/37c), [#1520](https://github.com/browser-use/browser-use/issues/1520) (10r/36c), [#1104](https://github.com/browser-use/browser-use/issues/1104); Stagehand [#391](https://github.com/browserbase/stagehand/issues/391) (15c), [#565](https://github.com/browserbase/stagehand/issues/565), [#1250](https://github.com/browserbase/stagehand/issues/1250); Playwright MCP [#891](https://github.com/microsoft/playwright-mcp/issues/891) (+11), [#769](https://github.com/microsoft/playwright-mcp/issues/769) (25c), [#942](https://github.com/microsoft/playwright-mcp/issues/942); agent-browser [#42](https://github.com/vercel-labs/agent-browser/issues/42) (13r), [#207](https://github.com/vercel-labs/agent-browser/issues/207), [#1371](https://github.com/vercel-labs/agent-browser/issues/1371); Notte [#1123](https://github.com/nottelabs/notte/issues/1123) (fixed 1.9.7) | Partly fixed in each | Browser acquisition is an explicit typed step (launch / attach-CDP with headers / provider) with **ownership recorded** (never kill what you attached); ephemeral isolated contexts by default, persistent profiles opt-in; distinct errors for "profile locked" vs "launch failed"; scoped disposal (`await using`, `async with`, Rust `Drop` + explicit `close`). |
| 6 | **Token and context bloat in agent-facing surfaces** | Playwright MCP [#395](https://github.com/microsoft/playwright-mcp/issues/395) (14r), [#889](https://github.com/microsoft/playwright-mcp/issues/889) (13r, ×6 regression), [#806](https://github.com/microsoft/playwright-mcp/issues/806) (9r), [#631](https://github.com/microsoft/playwright-mcp/issues/631); 5 independent HN reports; "Browser use is a token hog" ([HN](https://news.ycombinator.com/item?id=47906391)) | Playwright reversed after initially declining ("tokens will get cheaper…"): snapshots to file, `find`, distilled snapshots | Observation is **budgeted and addressable**: a handle plus summary by default, `find`/scope/depth, byte caps, JSON and text forms; batch verbs to cut turns. |
| 7 | **Interop with existing Playwright code and other drivers** | Stagehand [#507](https://github.com/browserbase/stagehand/issues/507) (18r/13c, incl. Crawlee's maintainer), [#2744](https://github.com/browserbase/stagehand/issues/2744), Crawlee still on Stagehand v3 ([apify/crawlee#4196](https://github.com/apify/crawlee/issues/4196)); Browser Use 0.6 dropped Playwright objects ([#2730](https://github.com/browser-use/browser-use/issues/2730)); Magnitude [#28](https://github.com/magnitudedev/browser-agent/issues/28), [#35](https://github.com/magnitudedev/browser-agent/issues/35) | Stagehand v4: "no interop" by design | Let AI verbs operate on a page another driver owns (attach to the same CDP target), or document the sidecar pattern; for Foxhound, the crawler must be able to hand Collie its page. |
| 8 | **Opaque or swallowed errors, silent hangs** | Browser Use hang class (#839, #1020, #986 35c, #1137, #2555), provider errors not surfaced; Stagehand `StagehandDefaultError` ([#704](https://github.com/browserbase/stagehand/issues/704)), "Invalid response schema" ([#689](https://github.com/browserbase/stagehand/issues/689)), v4 errors matched by message string (docs); Magnitude "Misalignment" ([#25](https://github.com/magnitudedev/browser-agent/issues/25), 10c); Notte opaque key error ([#545](https://github.com/nottelabs/notte/issues/545)) | Partly fixed | One error base with typed subclasses and **stable machine codes** (`ModelAuth`, `ModelOutputInvalid`, `TargetNotFound`, `Timeout`, `BudgetExceeded`, `Cancelled`, `PolicyBlocked`), each carrying step, attempted action, target and evidence; fail fast on auth/credit errors. |
| 9 | **Unsafe defaults: telemetry, secrets, domain policy, code execution, supply chain** | Telemetry: Browser Use [#282](https://github.com/browser-use/browser-use/issues/282) (19r), [#265](https://github.com/browser-use/browser-use/issues/265) (10r), [#5216](https://github.com/browser-use/browser-use/issues/5216), [#5837](https://github.com/browser-use/browser-use/issues/5837), HN "the (enabled by default) telemetry… is CRAZY" ([HN](https://news.ycombinator.com/item?id=49752572)); Notte and LaVague also on by default. Secrets: Stagehand cache receives `%var%` values by default; Playwright MCP password values in snapshots ([#1566](https://github.com/microsoft/playwright-mcp/issues/1566)). Domain policy bypass: Stagehand [#2945](https://github.com/browserbase/stagehand/issues/2945), [#3002](https://github.com/browserbase/stagehand/issues/3002); Browser Use [#5099](https://github.com/browser-use/browser-use/issues/5099), [#5879](https://github.com/browser-use/browser-use/issues/5879); Skyvern [#8577](https://github.com/Skyvern-AI/skyvern/issues/8577). Code execution: LaVague [#650](https://github.com/lavague-ai/LaVague/issues/650) (RCE via `eval`, open), Playwright `run_code_unsafe`. Supply chain: Browser Use [#4505](https://github.com/browser-use/browser-use/issues/4505) (9r, malicious litellm), [#4824](https://github.com/browser-use/browser-use/issues/4824) (exact pins); Stagehand Python wheel flagged ([#348](https://github.com/browserbase/stagehand-python/issues/348)). 6 independent forum reports for Browser Use alone | Mostly open | Telemetry off or content-free; secrets typed by the engine and excluded from prompts, caches, logs and snapshots by construction; network-layer domain policy that fails closed; trust level in the name of any code-execution verb; small dependency set with ranges. |
| 10 | **Cloud lock-in and local feature gaps** | Stagehand [#492](https://github.com/browserbase/stagehand/issues/492), v4 cache "has no effect" locally (vendor on [HN](https://news.ycombinator.com/item?id=49784175)), Gateway Browserbase-only; Browser Use stealth/CAPTCHA/scripts cloud-only; Notte TS cloud-only, SSPL; Skyvern extras split; 5 independent forum reports for Stagehand | Structural | Every SDK capability works locally with the same API; cloud adds capacity and hosting, not verbs. |
| 11 | **Login, 2FA, human-in-the-loop** | Browser Use [#221](https://github.com/browser-use/browser-use/issues/221) (18r/24c), [#404](https://github.com/browser-use/browser-use/issues/404) (11r), [#4798](https://github.com/browser-use/browser-use/issues/4798) (16c, open); Skyvern [#2604](https://github.com/Skyvern-AI/skyvern/issues/2604) (12c); Stagehand discussion [#895](https://github.com/browserbase/stagehand/discussions/895); "no way to login without giving the LLM your credentials" ([HN](https://news.ycombinator.com/item?id=43825196)) | Partial (placeholders, TOTP, vaults in clouds) | A first-class "await human / await secret" primitive that suspends timeouts and budgets; credential fill by the engine with a field-identity check. |
| 12 | **Driver guarantees when leaving Playwright** | Stagehand migration guide: no auto-wait, no `getBy*`, `waitForSelector` returns boolean; [#2744](https://github.com/browserbase/stagehand/issues/2744), [#2486](https://github.com/browserbase/stagehand/issues/2486), [#2982](https://github.com/browserbase/stagehand/issues/2982); agent-browser `fill` on React/contenteditable ([#2081](https://github.com/vercel-labs/agent-browser/issues/2081)); forum review listing lost auto-wait, `expect`, route, strict mode | Open | A "Playwright-shaped" page is judged on Playwright semantics: auto-waiting actionability, strictness, `getBy*`, network events, `fill` semantics. Partial shape plus missing guarantees is worse than a different shape. |
| 13 | **Budgets and cost visibility** | Browser Use [#230](https://github.com/browser-use/browser-use/issues/230) (8r), [#804](https://github.com/browser-use/browser-use/issues/804) (15c), [#5293](https://github.com/browser-use/browser-use/issues/5293); Skyvern [#6226](https://github.com/Skyvern-AI/skyvern/issues/6226) (open); HyperAgent hidden `maxSteps: 2` in `extract` ([#54](https://github.com/hyperbrowserai/HyperAgent/issues/54)); Stagehand dollar-cap discussion [#3097](https://github.com/browserbase/stagehand/discussions/3097); Notte/Skyvern charged on failed runs (HN, GIGAZINE) | Steps only, nearly everywhere | Token, cost, wall-clock and step budgets as run and call options, enforced by the engine, with a typed `BudgetExceeded` outcome and partial result; usage on every result by default. |
| 14 | **Runtime portability and dependency weight** | Stagehand Cloudflare Workers [#564](https://github.com/browserbase/stagehand/issues/564) (12r), [#164](https://github.com/browserbase/stagehand/issues/164) (11r), Lambda [#845](https://github.com/browserbase/stagehand/issues/845), Bun [#1552](https://github.com/browserbase/stagehand/issues/1552), Next.js [#1464](https://github.com/browserbase/stagehand/issues/1464); Browser Use 107 wheels / 262 MB resolved, exact pins [#4824](https://github.com/browser-use/browser-use/issues/4824); agent-browser Windows daemon failures ([#148](https://github.com/vercel-labs/agent-browser/issues/148) 15r, [#56](https://github.com/vercel-labs/agent-browser/issues/56) 22c); Skyvern Windows ([#3546](https://github.com/Skyvern-AI/skyvern/issues/3546)) and Docker ([#312](https://github.com/Skyvern-AI/skyvern/issues/312) 24c) | Mixed | Thin language SDKs with few dependencies and no runtime-specific APIs; Windows tier-1; a `doctor` command. |
| 15 | **Language parity** | Stagehand Python waitlist [#393](https://github.com/browserbase/stagehand/issues/393) (28c over 5 months), Python v3 timeline [stagehand-python#228](https://github.com/browserbase/stagehand-python/issues/228); Browser Use TS port request [#1263](https://github.com/browser-use/browser-use/issues/1263) (+17), Go [#991](https://github.com/browser-use/browser-use/issues/991) (+9); Notte TS [#167](https://github.com/nottelabs/notte/issues/167), "no typescript support?" ([HN](https://news.ycombinator.com/item?id=44861659)); Skyvern ~15 Python-only page features and mixed casing in TS; Midscene has 3 community Python ports | Stagehand v4 fixed it with generated SDKs | Generate every binding from one schema; parity tests on documented snippets; one casing convention per language for inputs and outputs. |
| 16 | **Configuration footguns** | Browser Use `**kwargs` swallowed ([#3535](https://github.com/browser-use/browser-use/issues/3535)), import-time env reads ([#939](https://github.com/browser-use/browser-use/issues/939), [#1912](https://github.com/browser-use/browser-use/issues/1912)), empty env var enables telemetry ([#5837](https://github.com/browser-use/browser-use/issues/5837)); Skyvern timeouts in minutes/seconds/ms and `webhook_url` vs `webhook_callback_url` silently ignored; Midscene `modelConfig` replaces env; Notte `nest_asyncio` on import ([#509](https://github.com/nottelabs/notte/issues/509)) | Open | Strict validation of every option; one time unit (or typed durations); no import-time side effects; env vars only for secrets and deployment knobs. |
| 17 | **Logging and observability control** | Stagehand `onLog` forces stderr ([#3035](https://github.com/browserbase/stagehand/issues/3035), 9r, top v4 issue), "Make executed Playwright steps visible" ([#186](https://github.com/browserbase/stagehand/issues/186), 8r), "debug the actual methods" ([HN](https://news.ycombinator.com/item?id=42641442)); Browser Use stdout pollution breaks stdio MCP ([#2748](https://github.com/browser-use/browser-use/issues/2748)); Midscene report size ([#619](https://github.com/web-infra-dev/midscene/issues/619)); Playwright MCP video ([#695](https://github.com/microsoft/playwright-mcp/issues/695) +10) | Partly fixed | Caller-owned log sinks; never write to stdout; every AI result carries the resolved action and evidence; traces and recordings as core artifacts. |

**Not found:** a GitHub issue objecting to Stagehand removing `agent()` (one HN question only), complaints about Stagehand's extension architecture, or complaints about Browser Use's Rust beta beyond its own integration ledger.

## 7. Open questions and unverified items

**Unverified (marked in the sections too).**
- Which dependent pins Stagehand `1.14.0` (26% of last week's npm downloads); CI and transitive installs inflate all per-version counts.
- Why Microsoft removed Playwright's hidden `page.agent()` (`PageAgent.perform/expect/extract` with a code cache, 1.58): the PRs say only "Not ready for primetime!" and "chore: rip perform" ([pw#38882](https://github.com/microsoft/playwright/pull/38882), [pw#39626](https://github.com/microsoft/playwright/pull/39626)).
- Browser Use's rationale for dropping LangChain (0.3.3): not stated in release notes, PRs or docs.
- Browser Use beta Agent: custom `Tools` not sent to the Rust runtime, inferred from source, not executed.
- Skyvern: whether cost fields pass through untyped `get_run` JSON; absence of in-process SDK hooks.
- Whether Stagehand's Browserbase cache server applies any freshness check beyond the key (server code not public).
- An RCE advisory against Browser Use reportedly unanswered for ~40 days (single HN reporter).
- Reddit sentiment for every library (unreachable in this pass).
- Third-party review wording obtained only via search summaries (Midscene "approaching native speed", a Browser Use "3 lines" review).
- None of the code samples were executed.

**Open questions for the Collie / Foxhound API decision** (facts above, decisions not taken here):
1. **Where do AI verbs live?** `page.act()` on the Playwright-shaped page (Skyvern, HyperAgent, Stagehand v2) or `collie.act(x, { page })` on a runtime object (Stagehand v3/v4)? The page form removes the implicit active-page ambiguity that Stagehand documents.
2. **Is `observe` → typed action → `act(action)` the replay contract**, and what does the action carry beyond Stagehand's absolute XPath (stable selector ladder like Browser Use's EXACT→STABLE→XPATH→AX_NAME→ATTRIBUTE, a target fingerprint, frame identity)?
3. **Is there a deterministic `check`/`assert` verb** next to `act/observe/extract`, and does every result separate stopped / agent-reported / verified?
4. **Schema input:** JSON Schema on the wire with native types per language (Zod/Standard Schema, Pydantic, serde) and no dependency on a schema library's private internals?
5. **Failure model:** values with an explicit `raiseOnFailure`-style switch (Notte), or exceptions with stable codes, but not both by accident (Stagehand)?
6. **Built-in agent:** ship none (Stagehand v4, Playwright), a thin journaled loop over the same verbs, or a first-class one (Browser Use)? The two biggest vendors that tried in-library agents in 2026 removed them.
7. **Cache location and format:** local, committed, diffable artifacts by default (Midscene, Stagehand v3) with optional shared storage, versus server-side (Stagehand v4)?
8. **Lifecycle:** `await using` / `async with` / Rust RAII plus explicit ownership of launched versus attached browsers; one object or two (browser + AI runtime)?
9. **Interop:** can Collie's verbs act on a page Foxhound (or Playwright, or Crawlee) already drives, given that Crawlee's maintainer asked Stagehand for exactly this ([#507](https://github.com/browserbase/stagehand/issues/507))?
10. **Budget and cancellation:** AbortSignal / CancellationToken on every verb, plus token, cost, time and step budgets with partial results?

## Appendix A. Forum and blog evidence (HN, blogs, Stack Overflow)

### A.1 Classification rules

- **Vendor**: the maintainers of the library being discussed. Identified from self-description in the thread: Browserbase/Stagehand: `hackgician`, `pkiv`, `MiguelG719`, `wittydeveloper`, `Kylejeong21`, `monadoid`. Browser Use: `gregpr07`, `MagMueller`, `nikisweeting`, `mertunsall`, `LarsenCC`, `shawnpana`, plus `reaganhsu`, inferred from the context. Skyvern: `suchintan`, `keremyilmaz`, `wintonzheng`. Notte: `ogandreakiro`.
- **Competitor**: someone promoting a competing product in the same post, such as the authors of Libretto, Sentinel, Smooth, Magnitude, PageMap, TinyFish, rtrvr.ai or bb-browser. Their claims are recorded but **not counted** as independent reports.
- **Adjacent**: an infrastructure or neighbouring vendor, or a builder who sells a product on top of the library (The Password App, ProofShot, Sim Studio, bot-detection firms). Counted separately from independent users.
- **Independent**: no affiliation visible. "Independent" means no affiliation was observed, not that none exists.
- Every quote is under 15 words. Paraphrases carry no quotation marks.

Column key: **L/P** = LIKE or PAIN. **Aff.** = I (independent), V (vendor), C (competitor), A (adjacent or builder-on-top). "HN" links go to `news.ycombinator.com/item?id=`.

### A.2 Stagehand (browserbase/stagehand)

| Date | Source | Handle | Version | L/P | Theme | Quote or paraphrase | Aff. |
|---|---|---|---|---|---|---|---|
| 2025-01-08 | [HN 42638393](https://news.ycombinator.com/item?id=42638393) | sparab18 | v1 | LIKE | API clarity; anti-bot (built-in proxies and CAPTCHA via Browserbase) | "actually a useful abstraction here" | I |
| 2025-01-09 | [HN 42644964](https://news.ycombinator.com/item?id=42644964) | fbouvier | v1 | LIKE | API clarity | "The act, extract, and observe methods are super clean." | A (Lightpanda) |
| 2025-01-09 | [HN 42642063](https://news.ycombinator.com/item?id=42642063), [42642224](https://news.ycombinator.com/item?id=42642224) | kevmo314 | v1 | PAIN | cost/tokens; caching | "How do you avoid this becoming horrendously expensive per run?" and "a pretty tough sell over bare playwright" | I |
| 2025-01-09 | [HN 42640192](https://news.ycombinator.com/item?id=42640192) | mpalmer | v1 | PAIN | determinism | Wants Stagehand to produce deterministic Playwright code. "I don't think I could plausibly argue for using LLMs at runtime" | I |
| 2025-01-09 | [HN 42642347](https://news.ycombinator.com/item?id=42642347) | dchuk | v1 | PAIN (request) | determinism/replay; structured output | Wants extract() to also return "the xpath elements you'd use to re-scrape the page" | I |
| 2025-01-10 | [HN 42652051](https://news.ycombinator.com/item?id=42652051) | xingwu | v1 | PAIN (request) | determinism | "Can the script be compiled into actual DOM operations" | I |
| 2025-01-09 | [HN 42644754](https://news.ycombinator.com/item?id=42644754) | joshstrange | v1 | request | determinism/caching | Proposes a hybrid: use AI only when the XPath fails, then cache the result until it breaks again | I |
| 2025-01-09 | [HN 42641442](https://news.ycombinator.com/item?id=42641442) | jsdalton | v1 | PAIN (question) | observability | "is it possible to debug the actual methods those methods were translated to?" Maintainer reply: "It's a little hacky" ([42642112](https://news.ycombinator.com/item?id=42642112)) | I (+V reply) |
| 2025-01-09 | [HN 42648628](https://news.ycombinator.com/item?id=42648628), [42650846](https://news.ycombinator.com/item?id=42650846) | fidotron | v1 | PAIN | local models; packaging | Custom LLMClient types were missing from the release. The local model was inaccurate: "took about 2 minutes of playing around to get annoyed" | I |
| 2025-01-09 | [HN 42641841](https://news.ycombinator.com/item?id=42641841) | andrewmcwatters | v1 | PAIN | API foundation (built on a test library) | "you're forcing users to write kludges that wrap around the testing interface" | I |
| 2025-01-09 | [HN 42644619](https://news.ycombinator.com/item?id=42644619) | z3t4 | v1 | PAIN | API clarity (naming) | "\"create-browser-app\" is a very generic name" | I |
| 2025-01-09 | [HN 42645490](https://news.ycombinator.com/item?id=42645490) | asar | v1 | PAIN | anti-bot | Invisible CAPTCHAs detect Playwright-based tools. The maintainer agrees antibots detect Playwright config | I |
| 2025-04-09 | [HN 43639238](https://news.ycombinator.com/item?id=43639238) | dimitri-vs | v2 | LIKE | provider breadth | "you can do pretty much anything while being provider agnostic" | I |
| 2025-04-28 | [HN 43825196](https://news.ycombinator.com/item?id=43825196) | waleedlatif1 | v2 | PAIN | login/auth, secrets | "there's no way to login without giving the LLM your credentials" | A (Sim Studio) |
| 2025-06-09 | [HN 44225534](https://news.ycombinator.com/item?id=44225534) | baalimago | v2 | mixed | reliability | Built "agentic tests" on Stagehand. "there were some limitations in both the Linear SDK and in Stagehand" | I |
| 2025-09-29 | [HN 45418483](https://news.ycombinator.com/item?id=45418483) | darkvertex | v2 | LIKE | AI mixed with code; runs locally | "it's an AI layer built on top of Playwright" | I |
| 2025-11-13 | [Microsoft ISE dev blog](https://devblogs.microsoft.com/ise/app-modernization-llm-driven-ui-tests-hve/) | ISE team | unstated | LIKE + PAIN | cost; determinism; provider breadth | LIKE: plain-language authoring. PAIN: per-run tests "can be expensive because of token consumption". No direct Azure OpenAI support, so they wrote a custom LLMClient. Logged XPaths were fragile. They converted runs into Playwright tests, roughly 85% correct | I (company engineering blog) |
| 2025-12-01 | [HN 46112951](https://news.ycombinator.com/item?id=46112951) | idopmstuff | v3 | LIKE | AI generating code | Rates director.ai, "which just writes Stagehand code on the fly", among the most promising agents | I |
| 2026-01-28 | [HN 46796310](https://news.ycombinator.com/item?id=46796310) | thisnick | v3 | LIKE | page representation | Credits Stagehand's accessibility-tree approach for browser reliability | A (agent-rdp) |
| 2026-03-04 | [HN 47241419](https://news.ycombinator.com/item?id=47241419) | causal | v3 | LIKE | docs | The docs chatbot "can quickly direct me to the documentation I'm looking for" | I |
| 2026-04-16 | [HN 47788344](https://news.ycombinator.com/item?id=47788344) | tanishqkanc | v3 | PAIN | determinism | Started on Stagehand and left: act() means "there's now runtime non-determinism" | C (Libretto) |
| 2026-05-19 | [HN 48196527](https://news.ycombinator.com/item?id=48196527) | isoldex | v3 | PAIN | reliability; cost | Client project on Stagehand and browser-use: "flaky reliability on multi-step flows, and token costs that ate the budget" | C (Sentinel) |
| 2026-09-18 | [HN 49757863](https://news.ycombinator.com/item?id=49757863) | ishankunam | v4 | PAIN | breaking changes | "why not keep agent() alongside the new primatives?" Vendor reply: "We removed agent because so many great harnesses are available" ([49757944](https://news.ycombinator.com/item?id=49757944)) | I (+V) |
| 2026-09-19 | [HN 49763632](https://news.ycombinator.com/item?id=49763632) | ulrikrasmussen | v4 | LIKE + PAIN | caching; lock-in | "I like the caching idea" but asks why the cache "couldn't just be a local file" committed with the script. Vendor reply: "with a purely local browser, the cache option has no effect" ([49784175](https://news.ycombinator.com/item?id=49784175)) | I (+V) |
| 2026-09-19 | [HN 49763578](https://news.ycombinator.com/item?id=49763578) | pixelstack | v4 | PAIN | lock-in; vendor claims | "promised increase only achievable with their main product". The vendor denies this | I |
| 2026-09-18 | [HN 49757218](https://news.ycombinator.com/item?id=49757218) | dot_louis | v4 | concern | lock-in perception | "Do I need to pay for Browserbase to use this?" Vendor: no, local by default | I |
| 2026-09-20 | [HN 49772141](https://news.ycombinator.com/item?id=49772141) | sa46 | v4 | PAIN | skepticism of vendor performance claims | "Tests typically drive the browser on the same machine. What round trip" | I |
| 2026-09-19 | [HN 49763499](https://news.ycombinator.com/item?id=49763499) | shashanoid | v4 | PAIN | anti-bot | "playwright is playwright. Dead bot giveaway." | I |
| 2026-09-18 | [HN 49757217](https://news.ycombinator.com/item?id=49757217) | vishalanton | v4 | LIKE | tokens (compared with computer use) | "Stagehand seems more token efficient." | I |
| 2026-09-24 | [HN 49834078](https://news.ycombinator.com/item?id=49834078) | mhykim | v4 | PAIN (question) | value of the abstraction | "Why use Stagehand when agents can write CDP / Playwright on the fly". Vendor answer: tokens, performance, observability, security policies | I (+V) |
| 2026-09-18 | [ADR-0002 in an Effect wrapper library](https://github.com/LordCoughmann/effect-libs-browser/pull/19) | LordCoughmann | v4 | PAIN | breaking changes; platform | Defers v4 support. v4 is a rewrite rather than a version bump: new constructor, new return shapes, agent() removed. Extension attach is uneven across providers, and Node imports put Cloudflare Workers support at risk | I (downstream maintainer) |
| 2026-09-20 | [Gardenee blog review](https://agmazon.com/blog/articles/technology/202609/stagehand-v4-browser-automation-en.html) | (blog) | v4 | PAIN + mixed LIKE | breaking changes; lost Playwright features; docs; lock-in; vendor claims | "You can't hand a Playwright Page to Stagehand." Lost auto-wait, `expect`, route interception and strict mode. Old examples throw ZodError, and search results surface v3 docs first. Cache and Model Gateway require Browserbase. The 2x claim is called conditional. Not tested first-hand | I (review) |
| 2026-10-08 | [HAI Substack, "Stagehand in Practice"](https://himanshuai.substack.com/p/stagehand-in-practice) | (author sells a Stagehand book) | v4 | LIKE + PAIN | primitives; churn; false success; caching | LIKE: per-step control, observe() as a preview, Zod schemas, explicit config. PAIN: tutorials "still show v2 or v3 patterns". A wrong element can be chosen and still report success. Risk of a stale server cache | A (semi-promotional) |
| Apr 26, edited Jul 24 (year not shown; likely 2026, since it cites v3 CDP) | [dev.to, "Framework Wars"](https://dev.to/stevengonsalvez/browser-tools-for-ai-agents-part-2-the-framework-wars-browser-use-stagehand-skyvern-4gn) | stevengonsalvez | v3 | LIKE + PAIN | API clarity; caching; Python; lock-in | "The API surface is dead simple." Caching praised. PAIN: friction for Python shops; "The SDK is the on-ramp. The cloud is the toll road." Analysis, not hands-on use | I (analysis) |
| 2026-02-07 | [Stack Overflow 79884792](https://stackoverflow.com/questions/79884792) | n/a | v3 | PAIN | iframes | setInputFiles does not upload inside an iframe. Unanswered | I |
| n.d. | [OpenSourceDiscovery #98](https://opensourcedisc.substack.com/p/opensourcediscovery-98-browserbase) | (newsletter) | v1/v2 | LIKE | ease of use | Search snippet: said browser actions were easy, scored production readiness 9/10, wanted Ollama support. **Snippet only, not opened** | I |
| 2026 | [Mastra issue #24221](https://github.com/mastra-ai/mastra/issues/24221), [Stagehand #1250](https://github.com/browserbase/stagehand/issues/1250) | n/a | v4, v3 | PAIN | breaking changes | Integrator depends on v3 internals that v4 moves. v3 broke storageState and userDataDir persistence. **GitHub pointers, snippet only** (other agents cover GitHub) | A, I |

**Vendor statements (context, not counted):**

- Show HN [42635942](https://news.ycombinator.com/item?id=42635942): v1 was built for "fine-grained control" between steps, not as a full agent. The team says Stagehand is not mainly meant for test suites ([42640303](https://news.ycombinator.com/item?id=42640303)).
- [v3 blog](https://www.browserbase.com/blog/stagehand-v3), 2025-10-29: dropped Playwright for CDP, added automatic action caching. Rationale: v2 was "fast, but not fast enough".
- v3 also conceded that keeping Python at parity with TypeScript is "nearly impossible" ([via search](https://www.browserbase.com/blog/browser-automation-all-languages-with-stagehand), snippet).
- [v4 blog](https://www.browserbase.com/blog/stagehand-v4), 2026-08-10: the engine moved into an in-browser extension, and "The browser is the only real source of truth". It removed agent(), deepLocator, frameLocator and selector. Caching is server-side only and there is no client-side cache. The 1.59x batch result came from "one run".

### A.3 Browser Use (browser-use/browser-use, Python)

| Date | Source | Handle | Version | L/P | Theme | Quote or paraphrase | Aff. |
|---|---|---|---|---|---|---|---|
| 2025-02-25 | [HN 43174017](https://news.ycombinator.com/item?id=43174017) | baal80spam | 0.1.x | LIKE | provider breadth | "browser-use is compatible with more models". Tried it with Gemini: "quite promising" | I |
| 2025-02-25 | [HN 43174273](https://news.ycombinator.com/item?id=43174273), [43189301](https://news.ycombinator.com/item?id=43189301) | arjunchint | 0.1.x | PAIN | security | Docs tell users to launch their real Chrome in remote-debugging mode, which leaves it exposed to exploits | C (rtrvr.ai) |
| 2025-02-26 | [HN 43179310](https://news.ycombinator.com/item?id=43179310) | 101008 | 0.1.x | PAIN | security | "It's completely unusable if this isn't solved." | I |
| 2025-02-25 | [HN 43174646](https://news.ycombinator.com/item?id=43174646) | xnx | 0.1.x | PAIN (request) | AI mixed with code | Asks to mix fuzzy AI steps with selectors. Vendor reply: "Currently the library api is very raw" ([43175620](https://news.ycombinator.com/item?id=43175620)) | I (+V admission) |
| 2025-02-25 | [HN 43176692](https://news.ycombinator.com/item?id=43176692), [43177167](https://news.ycombinator.com/item?id=43177167) | dogman123 | 0.1.x | mixed | reliability | The quickstart "seemed to be incapable of completing the task". A complex work task "performed perfectly in my few test cases" | I |
| 2025-02-26 | [HN 43180816](https://news.ycombinator.com/item?id=43180816) | agotterer | 0.1.x | PAIN | reliability; false success | Stuck in a loop loading a tab. "it hallucinated form input values" | I |
| 2025-02-26 | [HN 43180630](https://news.ycombinator.com/item?id=43180630) | alex1115alex | 0.1.x | LIKE + PAIN | speed | Built a demo on it, but "the agent is slow for complex tasks" | I |
| 2025-02-26 | [HN 43181425](https://news.ycombinator.com/item?id=43181425) | anhner | 0.1.x | LIKE | local models | Found Ollama support. Vendor: Llama is "often confused by the structured output format" ([43181548](https://news.ycombinator.com/item?id=43181548)) | I (+V) |
| 2025-02-26 | [HN 43179794](https://news.ycombinator.com/item?id=43179794) | elyase | 0.1.x | PAIN | integration | Uses something else in Cursor because "it doesn't support MCP integration". The vendor first called MCP "sort of a gimmick", then reversed | I |
| 2025-03-02 | [HN 43228887](https://news.ycombinator.com/item?id=43228887) | 35mm | 0.1.x | PAIN | anti-bot | "flagged and blocked me / my home IP as a bot within 10 seconds" | I |
| 2025-05-16 | [HN 44007385](https://news.ycombinator.com/item?id=44007385) | cdolan | 0.1.4x | PAIN | determinism/replay | Tried to "create deterministic playwright scripts for months with mixed results" | I |
| 2025-05-16/17 | [HN 44008408](https://news.ycombinator.com/item?id=44008408), [44015145](https://news.ycombinator.com/item?id=44015145), [44036897](https://news.ycombinator.com/item?id=44036897) | deepdarkforest, joshstrange, mparis | Workflow Use | LIKE | determinism; waits | Network-idle wait: "it filters for those surprisingly well". "I love the use of reusable non-LLM code for the \"happy path\"". Prefers deterministic programs | I (deepdarkforest is A: agentic QA) |
| 2025-06-27 | [HN 44398292](https://news.ycombinator.com/item?id=44398292) | anerli | 0.x | PAIN | interaction coverage | Set-of-marks is "strongly coupled to the DOM for interaction". Drag-and-drop and canvas are hard | C (Magnitude) |
| 2025-07-31 | [HN 44746868](https://news.ycombinator.com/item?id=44746868) | nicewood | 0.x | PAIN | anti-bot | "some sites might block you out after you had some agent runs" | I |
| 2025-10-20 | [HN 45645773](https://news.ycombinator.com/item?id=45645773) | mahdiyar | 0.x | LIKE | ergonomics; tokens | A shell script plus CLAUDE.md. "it uses near to zero of context window compared to MCP" | I |
| 2025-11-30 to 12-26 | [HN 46094160](https://news.ycombinator.com/item?id=46094160), [46167402](https://news.ycombinator.com/item?id=46167402), [46388466](https://news.ycombinator.com/item?id=46388466) | thepasswordapp / password-app | 0.x | LIKE + PAIN | login/auth, secrets | LIKE: the LLM loop copes with changing DOMs. PAIN: "We modified browser-use to pass passwords via a secure parameter" (a fork) | A (product built on it) |
| 2026-01 to 06 | [Thinslices insights](https://www.thinslices.com/insights/browser-use-ai-agents-how-autonomous-web-automation-actually-works-in-production) | (services firm) | n/a | PAIN | false success; cost | Two production deployments. Agents can "look correct but is subtly wrong". Cost per inference adds up. They narrowed the agent to one job with deterministic code around it | A (services firm) |
| 2026-02-24 | [HN 47134991](https://news.ycombinator.com/item?id=47134991) | abhikul0 | skills | LIKE (mixed) | local models | Tried browser-use skills with several small local models and settled on gpt-oss for its pass rate | I |
| 2026-03-24 | [HN 47503195](https://news.ycombinator.com/item?id=47503195) | abhikul0 | 0.x | PAIN | supply-chain security | Hit by the compromised litellm 1.82.7/1.82.8: "it installs litellm as a dependency" | I |
| 2026-04-19 | [HN 47828260](https://news.ycombinator.com/item?id=47828260) | Zetaphor | cloud | PAIN | cloud onboarding | Server error at cloud signup. "Great first impression!" | I |
| 2026-04-19 | [HN 47831611](https://news.ycombinator.com/item?id=47831611) | dorianmariewo | CLI | PAIN | install; security | `curl … | bash` installer: "will install some random software in your machine" | I |
| 2026-04-24 | [HN 47893563](https://news.ycombinator.com/item?id=47893563) | mattaustin | 0.x | PAIN | security process | RCE advisory filed about 40 days earlier: "I am a bit stunned by the lack of response." | I |
| 2026-04-25 | [HN 47897630](https://news.ycombinator.com/item?id=47897630) | jadbox | Harness | PAIN | install | install.md is long and LLM-interpreted. "I would have preferred a real installer" | I |
| 2026-04-25 | [HN 47897995](https://news.ycombinator.com/item?id=47897995) | taikon | n/a | PAIN | reputation, reliability | "I thought browser-use was janky and barely worked?" (hearsay) | I |
| 2026-04-25 | [HN 47897927](https://news.ycombinator.com/item?id=47897927) | joemazerino | cloud | LIKE + PAIN | cloud upsell | "Browser-use is incredible." But "the API is restricted to a paid tier" | I |
| 2026-04-24 | [HN 47894414](https://news.ycombinator.com/item?id=47894414) | esafak | Harness | PAIN | value proposition | "I'm struggling to see why I should use this over agent-browser" | I |
| 2026-04-24 | [HN 47894338](https://news.ycombinator.com/item?id=47894338) | jstanley | Harness | PAIN | anti-bot | CDP use was not detected, but "Browser-Use" itself can still be detected | A (bot-detection vendor) |
| 2026-04-26 | [HN 47906391](https://news.ycombinator.com/item?id=47906391) | tonyww | 0.x | PAIN | tokens | "Browser use is a token hog" | A (Sentience/predicate author) |
| 2026-04-27 | [HN 47919328](https://news.ycombinator.com/item?id=47919328) | avereveard | Harness | PAIN | security | "Direct CDP hands the model the keys". Fails enterprise review | I |
| 2026-04-25 | [HN 47899864](https://news.ycombinator.com/item?id=47899864) | agdexai | Harness | PAIN | observability | With raw CDP, "you lose auditability". Wrapped helpers are easier to debug | A (unclear) |
| 2026-04-27 | [HN 47927379](https://news.ycombinator.com/item?id=47927379) | vaporaviatorlab | Harness | LIKE | ergonomics | Likes a thin harness that is "small, inspectable, and self-extending" | I |
| 2026-08-18 | [HN 49346302](https://news.ycombinator.com/item?id=49346302) | nateb2022 | 0.x | LIKE | ergonomics, capability | "a lot easier to embed in other programs than Claude's driver" | I |
| 2026-09-15 | [HN 49714990](https://news.ycombinator.com/item?id=49714990) | danielvaughn | n/a | PAIN | tokens (raw-CDP direction) | "CDP is extremely token consumptive." | I |
| 2026-09-18 | [HN 49752572](https://news.ycombinator.com/item?id=49752572) | maho | Harness | PAIN | telemetry, privacy | "the (enabled by default) telemetry they send to PostHog is CRAZY". Says browser-harness can leak credentials | I |
| current | [Browser Use telemetry docs](https://docs.browser-use.com/open-source/development/monitoring/telemetry.md) | vendor doc | current | (corroborates) | telemetry | Opt-out telemetry may include "task instructions, URLs visited, action traces, errors, and final results" | V |
| Apr 26, edited Jul 24 (year not shown; likely 2026, since it cites v3 CDP) | [dev.to, "Framework Wars"](https://dev.to/stevengonsalvez/browser-tools-for-ai-agents-part-2-the-framework-wars-browser-use-stagehand-skyvern-4gn) | stevengonsalvez | 0.x | LIKE + PAIN | cost; replay | "the safe bet". "Every single step in the agent loop requires an LLM call." "There's no caching, no replay" | I (analysis) |
| Oct 4 (year unknown) | [dev.to, Browser Use vs Skyvern self-hosted](https://dev.to/jangwook_kim_e31e7291ad98/browser-use-vs-skyvern-self-hosted-setup-trade-offs-and-completion-verification-1l5c) | jangwook_kim | unpinned | PAIN | false success; reproducibility | Agent "done" signals are unverified. Their schema receipt "still passed for a missing and a corrupted file". They could not reproduce a fair head-to-head | I |
| 2025-01 to 2026-01 | Stack Overflow, about 10 questions (§A.7) | various | 0.1 to 0.x | PAIN | various | Parallel agents, file downloads, extensions, stuck at step 1, recording, slow import, Playwright executable mismatch, remote browser URL ignored | I |
| 2025 | GitHub [#2730](https://github.com/browser-use/browser-use/issues/2730), [#1157](https://github.com/browser-use/browser-use/issues/1157) | n/a | 0.6.0; 0.1.x | PAIN | breaking changes; loops | BrowserSession arguments rejected after upgrading from 0.5.11 to 0.6.0. Agent repeats step 1 forever. **GitHub pointers, snippet only** | I |

**Vendor statements (context, not counted):**

- "our biggest challenge is exactly this, solving thousands of issues" ([43177090](https://news.ycombinator.com/item?id=43177090)).
- [Leaving Playwright for CDP](https://news.ycombinator.com/item?id=44962869), 2025-08: Python was the reason, since "pyppeteer was abandoned". They are trying to remove DOM tampering because bot-blockers detect it.
- ["Bitter Lesson of Agent Frameworks"](https://browser-use.com/posts/bitter-lesson-agent-frameworks) (2026-01) and ["...Agent Harnesses"](https://browser-use.com/posts/bitter-lesson-agent-harnesses) (2026-04): the team now says "your helpers are abstractions too". It admits that "thousands of lines" of extractors and watchdogs needed constant upkeep, and that its earlier CDP abstraction stance was wrong.
- Browser Harness launch ([47890841](https://news.ycombinator.com/item?id=47890841)): the agent writes its own missing helpers, for example upload_file.
- No Browser Use post about dropping LangChain was found. **That premise is unverified.**

### A.4 Vercel agent-browser

| Date | Source | Handle | L/P | Theme | Quote or paraphrase | Aff. |
|---|---|---|---|---|---|---|
| 2026-01-15 | [paddo.dev](https://paddo.dev/blog/agent-browser-context-efficiency/) | (blog) | LIKE + PAIN | tokens; refs | "Your agent sees three lines instead of three thousand nodes." Some users report flaky refs on dynamic pages, and the command set is narrower than Playwright's | I |
| 2026-01-20 | [Pulumi blog](https://www.pulumi.com/blog/self-verifying-ai-agents-vercels-agent-browser-in-the-ralph-wiggum-loop/) | (Pulumi engineer) | LIKE + PAIN | tokens; refs; docs; waits | Measured 5,455 vs 31,117 characters against Playwright MCP over 6 tests. Refs are deterministic. PAIN: manual waits, "Documentation is thin". Playwright MCP "still wins on depth" | I (company engineering blog) |
| 2026-01-21 | [HN 46699311](https://news.ycombinator.com/item?id=46699311) | XCSme | PAIN | docs clarity | "I don't understand how \"agent-browser\" works." | I |
| 2026-02-06 | [HN 46913203](https://news.ycombinator.com/item?id=46913203) | waynenilsen | LIKE (qualified) | reliability | "agent-browser helped a lot over playwright but doesn't completely close the gap" | I |
| 2026-02-26, 08-18 | [HN 47167062](https://news.ycombinator.com/item?id=47167062), [49347126](https://news.ycombinator.com/item?id=49347126) | esafak | LIKE | ergonomics | "great CLI for agent-based browser automation". "works fine" | I |
| 2026-03-01 | [HN 47210372](https://news.ycombinator.com/item?id=47210372), [47222471](https://news.ycombinator.com/item?id=47222471) | ejholmes | LIKE | tokens; sessions | "much better than the playwright MCP". Handles persistent sessions as a CLI | I |
| 2026-03-15 | [HN 47392403](https://news.ycombinator.com/item?id=47392403) | Torn | LIKE | tokens | playwright-cli and agent-browser are "both more token-efficient than using the raw mcp" | I |
| 2026-03-15 | [HN 47391198](https://news.ycombinator.com/item?id=47391198) | NiekvdMaas | LIKE | login (real browser) | Works with Chrome DevTools MCP using `--auto-connect` | I |
| 2026-03-16 | [HN 47396169](https://news.ycombinator.com/item?id=47396169) | tomcasaburi | LIKE | ergonomics | Calls it "a good alternative" to playwright-cli plus skills | I |
| 2026-03-25 | [HN 47511689](https://news.ycombinator.com/item?id=47511689) | gverrilla | LIKE | tokens | "playwright-cli and agent-browser are much much better" | I |
| 2026-03-30 | [HN 47570289](https://news.ycombinator.com/item?id=47570289) | jberthom | PAIN | login/profiles | Launches fresh Chromium every time, so "no persisted auth" | A (ProofShot, built on it) |
| 2026-04-24 | [HN 47894085](https://news.ycombinator.com/item?id=47894085) | reaganhsu | PAIN | frames | Says it "struggles with things like cross-origin-iframes" | C (Browser Use) |
| 2026-05-05 | [HN 48019263](https://news.ycombinator.com/item?id=48019263) | miteshashar | LIKE | AI mixed with code | Claude Code plus agent-browser on Zoho Books, then a deterministic IIFE: "18x cost-saving" | I |

### A.5 Playwright MCP and playwright-cli (Microsoft)

| Date | Source | Handle | L/P | Theme | Quote or paraphrase | Aff. |
|---|---|---|---|---|---|---|
| 2025-10-20 | [HN 45642911](https://news.ycombinator.com/item?id=45642911) | syntax-sherlock | PAIN | tokens | "playwright-mcp eating through Claude's 200K token limit" | A (built an alternative skill) |
| 2025-10-20 | [HN 45645045](https://news.ycombinator.com/item?id=45645045) | yomismoaqui | PAIN | capability gaps | Tried to "coax the Playwright MCP to do a full page screenshot and it couldn't" | I |
| 2025-10-20 | [HN 45649038](https://news.ycombinator.com/item?id=45649038) | boredtofears | PAIN | reliability | "I get so many LLM death spirals with playwright." | I |
| 2025-10-20 | [HN 45644867](https://news.ycombinator.com/item?id=45644867) | simonw | LIKE | familiar API | "All of the good LLMs appear to know the API really well." | I |
| 2025-12-24 | [HN 46378860](https://news.ycombinator.com/item?id=46378860) | dmd | LIKE | network control | Uses it mostly to inject JS and to "monitor/modify network requests" | I |
| 2026-02-17 | [HN 47048097](https://news.ycombinator.com/item?id=47048097) | retio_pagemap | PAIN | tokens | "After 2-3 page navigations your context window is gone." | C (PageMap) |
| 2026-03-12 | [HN 47351433](https://news.ycombinator.com/item?id=47351433), [47352150](https://news.ycombinator.com/item?id=47352150) | sozal; heisen_berg | PAIN; rebuttal | tokens | About 1.5M vs about 330K tokens per task (competitor figures). Independent rebuttal: the gain "comes primarily from the execute tool batching actions" | C; I |
| 2026-03-15 | [HN 47392567](https://news.ycombinator.com/item?id=47392567) | mambodog | PAIN | tokens | Built a wrapper that "uses claude haiku to summarize the page snapshot" | I |
| 2026-03-28 | [HN 47551864](https://news.ycombinator.com/item?id=47551864) | silverwind | PAIN | multi-tab, parallelism | "only on instance/tab can be controlled" (sic) | I |
| 2026-04-07 | [HN 47679430](https://news.ycombinator.com/item?id=47679430) | braebo | PAIN | tokens | Context rarely exceeds 100k "(unless I'm using playwright mcp for testing)" | I |
| 2026-04-16 | [HN 47791237](https://news.ycombinator.com/item?id=47791237) | skapadia | LIKE | explore, then codegen | playwright-cli to explore, then a generated script for repeat runs | I |
| 2026-05-05 | [HN 48029825](https://news.ycombinator.com/item?id=48029825) | lsaferite | LIKE | ergonomics | playwright-cli with Claude Code is "highly effective" with "essentially zero setup" | I |
| 2026-05-06 | [HN 48031468](https://news.ycombinator.com/item?id=48031468) | hamasho | LIKE + PAIN | reliability; speed | "playwright MCP often works very well". But "it takes a lot of time to complete even easy tasks" | I |
| 2026-08-20 | [HN 49372088](https://news.ycombinator.com/item?id=49372088) | dragac | PAIN | tokens | "I always have this problem with Playwright MCP" | I |
| 2026-02-06 | [HN 46914596](https://news.ycombinator.com/item?id=46914596) | antves | PAIN | frames; reliability | Calls it "very unreliable and inefficient". Says it fails to pierce frames | C (Smooth) |

### A.6 Skyvern, Midscene, Notte, LaVague

| Date | Source | Handle | Lib | L/P | Theme | Quote or paraphrase | Aff. |
|---|---|---|---|---|---|---|---|
| 2024-03-14 | [HN 39707740](https://news.ycombinator.com/item?id=39707740) | dtnewman | Skyvern | PAIN | cost | "My OpenAI API bill is $3.20 after using this on a few different pages". Suggests generating scraper code instead | I |
| 2024-03-14 | [HN 39710578](https://news.ycombinator.com/item?id=39710578) | bravura | Skyvern | PAIN | determinism | "I don't want to use vision and LLMs for every page." | I |
| 2024-03-14 | [HN 39706785](https://news.ycombinator.com/item?id=39706785) | dinobones | Skyvern | PAIN | docs/transparency | Cost and models: "you make no mention of that anywhere on your landing page/documentation" | I |
| 2024-10-24 | [HN 41938930](https://news.ycombinator.com/item?id=41938930) | jackb4040 | Skyvern | PAIN | local models | Quotes the docs: Skyvern won't run "unless you enable at least one provider" | I |
| 2024-11-24 | [GIGAZINE hands-on](https://gigazine.net/gsc_news/en/20241124-skyvern-ai/) | (outlet) | Skyvern cloud | LIKE + PAIN | observability; auth; cost | LIKE: a screenshot per action plus a recording. PAIN: failed on Amazon 2FA. "even if the task fails, a fee will be charged". Some actions marked failed had actually succeeded | I |
| 2025-10-17 | [HN 45622188](https://news.ycombinator.com/item?id=45622188) | herpdyderp | Skyvern | PAIN | compliance | "if only they had enterprise compliance agreements available" | I |
| 2025-10-18 | [HN 45625571](https://news.ycombinator.com/item?id=45625571) | moomoo11 | Skyvern | PAIN | reliability, cost | "it didn't work for scraping a site", and does not want to pay again | I |
| Oct 4 (year unknown) | [dev.to audit](https://dev.to/jangwook_kim_e31e7291ad98/browser-use-vs-skyvern-self-hosted-setup-trade-offs-and-completion-verification-1l5c) | jangwook_kim | Skyvern | PAIN | install | `pip install skyvern` installs only the remote SDK. The full platform needs Compose or `skyvern[server]`, plus 4–8 GB of RAM | I |
| 2026-05-06 | [HN 48030492](https://news.ycombinator.com/item?id=48030492) | theturtletalks | Midscene | LIKE | provider breadth | "UI-TARS works but you can also use qwen vision models" | I |
| 2026-04 | [Chrome Web Store reviews](https://chromewebstore.google.com/detail/midscenejs/gbldofcpkknbggpkmbdaefngejllnief/reviews) | n/a | Midscene | LIKE (+ request) | vision accuracy; local models | Rated 4.7/5. A reviewer praises its visual precision; another asks for Ollama. **Snippet only, not opened** | I |
| n.d. | [Medium, "Midscene.js: A New-era Tool…"](https://medium.com/@345490675/midscene-js-a-new-era-tool-redefining-ui-automation-4760f0675334) | n/a | Midscene | PAIN | reliability; control flow | Terse instructions make runs unstable. It cannot plan conditions or loops. Prefer aiQuery plus JS asserts over aiAssert. **Snippet only, not opened; possibly team-affiliated** | ? |
| 2025-08-07 | [HN 44828670](https://news.ycombinator.com/item?id=44828670) | Jommi | Notte | PAIN | reliability | On hyatt.com: "completely failed another hype but actually doesn't work browser agent" | I |
| 2025-08 | [HN 44829490](https://news.ycombinator.com/item?id=44829490), [44834090](https://news.ycombinator.com/item?id=44834090) | jaflo, GoatInGrey, revskill | Notte | PAIN | cloud pricing | Credits left unexplained. "Credit based pricing is broken." | I |
| 2025-08-11 | [HN 44861659](https://news.ycombinator.com/item?id=44861659) | android521 | Notte | PAIN | TypeScript parity | "no typescript support?" | I |
| 2025-08-07 | [HN 44831739](https://news.ycombinator.com/item?id=44831739) | Bluestein | Notte | PAIN | ergonomics | The model is very "URL-oriented" and complains when no URL is given | I |
| 2024-12-17 | [HN 42436927](https://news.ycombinator.com/item?id=42436927) | jb_cnn | Notte | skeptic | primitives | Doubts a natural-language action space beats screenshots plus HTML | I |
| 2024-03-14 | [HN 39708335](https://news.ycombinator.com/item?id=39708335) | suchintan | LaVague | PAIN | tokens | Says LaVague sends the whole DOM, so token cost is very high | C (Skyvern) |

Vendor notes:

- Skyvern ([42743230](https://news.ycombinator.com/item?id=42743230)) planned a code-generation layer so that repeat runs would be "nearly free". Their HN reply to LaVague framed it as an autonomous agent, not code generation.
- Notte's core is SSPL ([46808183](https://news.ycombinator.com/item?id=46808183)), a licence that matters for lock-in.
- LaVague had almost no forum activity after 2024: 7 HN comment hits.

### A.7 Stack Overflow footprint

Searched through the Stack Exchange API; `search/excerpts` deduplicated by hand.

- **Browser Use: about 10 questions, 2025-01 to 2026-01, almost all in H1 2025.** Running multiple agents in parallel ([79447986](https://stackoverflow.com/q/79447986)), file downloads ([79384448](https://stackoverflow.com/q/79384448)), loading an extension ([79363972](https://stackoverflow.com/q/79363972)), "agent stuck in loop at Step 1" ([79539387](https://stackoverflow.com/q/79539387)), per-step screenshots and video ([79648080](https://stackoverflow.com/q/79648080)), `OSError: cannot open resource` ([79347454](https://stackoverflow.com/q/79347454)), slow import ([79526624](https://stackoverflow.com/q/79526624)), Playwright Chromium executable mismatch ([79530677](https://stackoverflow.com/q/79530677)), the agent running locally instead of in an E2B sandbox despite `browser_url` ([79863454](https://stackoverflow.com/q/79863454)), and a LangChain type error ([79395145](https://stackoverflow.com/q/79395145)). Recurring themes: parallelism and multi-tab, downloads and files, connecting to a remote browser, install, observability.
- **Stagehand: 1 question.** setInputFiles inside an iframe, 2026-02, unanswered ([79884792](https://stackoverflow.com/q/79884792)). Other "stagehand" hits are the unrelated Dart scaffolding tool.
- **Skyvern, Midscene, Notte, LaVague, agent-browser: 0.** Playwright MCP: only generic MCP-configuration questions.
- Interpretation: support happens on GitHub and Discord, not Stack Overflow. These counts say little about adoption.

### A.8 Forum synthesis (independent reports only; vendor and competitor items excluded unless noted)

Counts are distinct independent people or outlets per theme from §A.2–A.7. "+C" and "+A" note corroborating competitor or adjacent reports that are not in the count. They are small, opt-in samples (HN skews toward early adopters); read them as signal direction, not prevalence.

#### A.8.1 Stagehand

**Likes, ranked**
1. **Small, clear primitives that mix AI with code (act, extract, observe; step-level control).** 6 reports: sparab18, darkvertex, idopmstuff, dimitri-vs (composability), dev.to analysis, HAI (observe preview, Zod) [+A fbouvier]. This was the most praised quality in 2025, and it is the stated reason for choosing Stagehand over full agents.
2. **Caching as a route to deterministic replay.** 2 likes: ulrikrasmussen and the dev.to analysis. The demand side is much larger; see pain 1.
3. **Token efficiency compared with computer use or vision agents.** 1 (vishalanton).
4. **Provider-agnostic, works locally.** 2 (dimitri-vs, darkvertex).
5. **Docs findability (docs chatbot).** 1 (causal).

**Pains, ranked**
1. **Runtime non-determinism, and the wish to compile AI steps into replayable code.** 6: mpalmer, dchuk, xingwu, joshstrange, Microsoft ISE, HAI [+C Libretto]. The request dates to the first launch day. Since v3, caching is the vendor's answer, but see pain 2.
2. **Lock-in and cloud coupling, worst in v4.** 5: ulrikrasmussen, pixelstack, dot_louis (concern), Gardenee review, dev.to analysis. The vendor confirmed that v4's cache has no effect with a local browser. The Model Gateway also needs Browserbase.
3. **Breaking changes and version churn (v2 to v3 to v4).** 4 opened (ishankunam, LordCoughmann ADR, Gardenee, HAI) plus 2 GitHub snippets (Mastra, #1250). Specifics: agent() removed in v4 with no replacement. Constructor, page access and return shapes all changed. Playwright Page interop was dropped in v4. Stale v2/v3 snippets circulate everywhere.
4. **Cost per run (LLM on every call).** 2 (kevmo314, Microsoft ISE) [+C Sentinel].
5. **Lost Playwright conveniences and test-runner fit.** 2 (Gardenee: auto-wait, `expect`, route, strict mode; andrewmcwatters on building over a testing library). Developers also question the value: "why not raw CDP" (mhykim).
6. **Local and alternative providers are poorly supported.** 2 (fidotron: local accuracy and missing LLMClient types; Microsoft ISE: Azure OpenAI needed a custom client).
7. **Anti-bot detectability of a Playwright or CDP base.** 2 (asar, shashanoid). One like for Browserbase-hosted proxies and CAPTCHA (sparab18).
8. **Docs drift.** 2 (Gardenee, HAI). Observability of what act() actually did: 1 (jsdalton; the vendor called it "a little hacky").
9. **Python vs TS parity.** 1 independent (dev.to). The vendor itself called parity "nearly impossible" in v3, which motivated v4's thin clients.
10. **Vendor performance claims doubted.** 2 (sa46, pixelstack; Gardenee also flags single-run numbers).

#### A.8.2 Browser Use

**Likes, ranked**
1. **Broad model support, including local models.** 3: baal80spam, anhner, abhikul0. The vendor concedes that weak local models break structured output.
2. **Easy to embed or drive from a coding agent, and capable once it works.** 4: nateb2022, mahdiyar, vaporaviatorlab (the thin Harness), joemazerino ("incredible") [+A thepasswordapp].
3. **Record-then-replay (Workflow Use) and the network-idle wait.** 3: joshstrange, mparis, deepdarkforest (A). Popular as an idea; cdolan reports mixed results in practice.
4. **Community and ecosystem ("the safe bet").** 1 (dev.to analysis). Many HN posts are third-party products built on it.

**Pains, ranked**
1. **Security and trust.** 6: 101008 (CDP on the real browser), abhikul0 (compromised litellm dependency), dorianmariewo (`curl|bash` installer), mattaustin (RCE report unanswered for about 40 days), avereveard (raw CDP "hands the model the keys"), maho (default-on telemetry; possible credential leakage) [+C rtrvr.ai]. The vendor's own docs confirm telemetry can include task instructions, URLs and action traces. This was the most-reported cluster for Browser Use in 2026.
2. **Reliability: loops, hallucinated inputs, false "done".** 5: dogman123, agotterer, taikon (reputation), jangwook_kim (done signal unverified), plus a Stack Overflow "stuck at Step 1" question [+A Thinslices "subtly wrong"; +C Sentinel, TinyFish 8.1%].
3. **Cost, tokens and speed (an LLM call every step, no built-in replay).** 4: alex1115alex (slow), dev.to analysis, danielvaughn (raw CDP is token-heavy), Thinslices (A) [+A tonyww "token hog", +C bb-browser, Smooth, ApiFlash].
4. **Mixing deterministic code with AI, and stable replay.** 3: xnx (the vendor admitted "the library api is very raw"), cdolan (mixed results), dev.to ("no caching, no replay").
5. **Anti-bot blocks.** 2: 35mm, nicewood [+A bot-detection vendor].
6. **Install and onboarding friction.** 4: jadbox (install.md), dorianmariewo (installer), plus Stack Overflow questions on the Playwright executable mismatch and slow import.
7. **Cloud upsell and onboarding.** 2: joemazerino (API only on the paid tier), Zetaphor (signup crashed).
8. **Login and secrets.** Builders forked it to keep passwords out of prompts [A thepasswordapp]. The vendor points to `sensitive_data`.
9. **Breaking changes.** Forums are thin; evidence sits on GitHub (#2730 for 0.5→0.6; config renames). There is strategic churn too: Playwright to CDP (2025), framework to harness to raw CDP (2026), with the vendor saying "we were wrong".
10. **Parallelism and multi-tab, downloads, connecting to a remote browser.** Several Stack Overflow questions.

#### A.8.3 Secondary libraries

- **agent-browser.** Likes: 10 independent reports, including two company or engineering blogs. Mostly about token efficiency against Playwright MCP, compact refs, a simple CLI, and pairing it with a coding agent that then writes deterministic code (miteshashar). Pains: thin docs (2: Pulumi, XCSme), manual waits (1), no persisted auth by default (A), flaky refs on dynamic pages (reported via paddo), CLI-only (vendor competitor).
- **Playwright MCP.** Pains: token and context bloat (5 independent: mambodog, braebo, dragac, yomismoaqui on capability, Pulumi measurement [+C PageMap, Smooth, DevTools MCP]), slowness (1), one controllable tab (1), "death spirals" (1). Likes: models know the Playwright API (simonw), network interception and depth (dmd, Pulumi), and it often works (hamasho). **playwright-cli plus skills** is a favourite alternative: 5 likes (lsaferite, skapadia, Torn, gverrilla, tomcasaburi).
- **Skyvern.** Pains: cost (dtnewman; charged even on failure per GIGAZINE), wants code generation rather than vision on every page (bravura), unclear docs and pricing (dinobones), needs a hosted provider (jackb4040), 2FA failures (GIGAZINE), scraping failed (moomoo11), no compliance agreements (herpdyderp), install split between SDK and server (dev.to). Like: per-action screenshots and recordings (GIGAZINE).
- **Midscene.** Too thin to rank. 1 independent HN like (model flexibility: UI-TARS, Qwen). The other evidence is snippets only.
- **Notte.** Pains: reliability (Jommi), opaque credit pricing (3), no TypeScript (1), URL-oriented prompting (1).
- **LaVague.** Effectively silent after its 2024 launch.

#### A.8.4 Cross-library patterns (counts across all libraries, independent)

1. **"Use AI to write the script, not to run it."** At least 12 independent voices, from 2024 to 2026: mpalmer, dchuk, xingwu, joshstrange, Microsoft ISE (Stagehand); xnx, cdolan (Browser Use); bravura, dtnewman (Skyvern); tyingq ([48029952](https://news.ycombinator.com/item?id=48029952)), skapadia, miteshashar, cowartc ([47792235](https://news.ycombinator.com/item?id=47792235): runtime agents feel like "a black box"), pzo ([44393325](https://news.ycombinator.com/item?id=44393325)). Every primary vendor has moved toward this: Stagehand caching, Workflow Use, Skyvern code caching, Libretto. Users want the generated artifact **local and reviewable**. Stagehand v4's server-only cache drew pushback for exactly this reason.
2. **Token and context cost is the top complaint about agent-facing browser tools.** Spans Playwright MCP, Browser Use, Stagehand and Skyvern. CLI plus skill or "code mode" with batching is seen as the fix (agent-browser, playwright-cli, Stagehand v4 batch, DevTools MCP execute). An independent commenter noted that batching, not the protocol, drives most of the savings (heisen_berg).
3. **Security and trust in a driver that holds real credentials.** Browser Use dominates the reports. Related concerns: CDP on the user's real profile, telemetry, supply chain, prompt injection through page content ([47395353](https://news.ycombinator.com/item?id=47395353), adjacent) and keeping secrets out of prompts (Stagehand, Browser Use).
4. **Anti-bot detection.** Users say any Playwright- or CDP-based tool gets flagged (Stagehand, Browser Use, Playwright MCP). The vendors' answer is their paid cloud (proxies, stealth), which feeds the lock-in perception.
5. **Unverified success.** Several sources warn that the agent's "done" or "success" is not proof: agotterer, jangwook_kim, HAI, Thinslices, GIGAZINE (status labels wrong in both directions). Postconditions and artifact checks are the recommended mitigation.
6. **Version and paradigm churn erodes trust in examples.** Stagehand v2→v3→v4 and Browser Use library→harness→raw CDP both invalidate tutorials and AI-generated snippets (HAI, Gardenee).
7. **Login, profiles and 2FA are an unsolved developer-experience gap.** Credentials end up in prompts (Sim Studio, Password App), there is no persisted auth (ProofShot on agent-browser), 2FA fails (Skyvern), and Browser Use's real-profile mode is criticised on security grounds.
8. **Observability.** People ask what the AI step resolved to (jsdalton on Stagehand), for recordings and screenshots (Stack Overflow on Browser Use), and like per-action screenshots (Skyvern on GIGAZINE). Raw-CDP harnesses are criticised for losing auditability.

#### A.8.5 Gaps and caveats

- **Reddit was entirely unreachable** (WebSearch, WebFetch, the reddit JSON endpoints and the PullPush archive all blocked). The Reddit sentiment that was requested is missing; a human with a browser would need to collect it.
- HN users self-select as early adopters, and many threads are competitor Show HNs. Competitor claims are listed but not counted.
- Several blogs are reviews without first-hand runs: Gardenee, the dev.to "Framework Wars" analysis, HAI (which also sells a book). They are marked as such.
- No independent Stagehand Python user reports were found, and no forum reports on Browser Use's TypeScript port.
- Midscene evidence is mostly snippet-only.
