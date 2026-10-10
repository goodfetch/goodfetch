# AI-in-the-loop browser frameworks: architecture, dependencies and user pains

Research of **October 10, 2026** for ticket [#91](https://github.com/Jbmanllr/automan/issues/91) (map [#61](https://github.com/Jbmanllr/automan/issues/61)). It feeds [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72): every decision there should solve the pains found here and keep the praised API shapes. Like the rest of `rnd/`, this note is input for inspiration; it adopts no term or decision.

It covers about 40 frameworks and services where AI drives a browser run (agent) or where code calls AI to resolve, extract or repair what broke. For each it gives architecture, model layer, interfaces, caching/replay/repair, secrets, human-in-the-loop, cost, evaluation, dependencies, license and adapters, then the user pains and the praised designs. It refreshes the 2026-10-07 notes ([browser frameworks](2026-10-07-browser-frameworks.md), [agent capability matrix](2026-10-07-agent-capability-matrix.md), [Stagehand contracts and alternatives](2026-10-07-stagehand-alternatives-followup.md)) and the [parity list](https://github.com/Jbmanllr/automan/blob/research/parity-list/rnd/research/2026-10-10-parity-list.md) (Stagehand 4.2.0, Crawlee, Spider).

## Summary

- **Two camps, and the volume is in the thin one.** Thin, deterministic primitives that an external agent drives: Playwright MCP (6.56M npm downloads/week), Vercel agent-browser (1.66M/week), Stagehand v4 (1.69M npm + 0.40M PyPI/week), Cua Driver, Libretto. Embedded agent loops: Browser Use (1.71M PyPI/week, the one large exception), Skyvern, Notte, Midscene `aiAct`, Nova Act. Several embedded-loop projects are dormant or archived (LaVague, Agent-E, Laminar Index, Bytebot, Magnitude's browser agent, Shortest). **Stagehand v4 removed `agent()`** and Cua deprecated its agent SDK in favour of a driver plus a bring-your-own agent. Sources in [§2.1](#21-stagehand-browserbase-incl-director), [§3](#3-mcp-cli-and-sdk-primitives-vision-agents-testers-and-recorders).
- **"Learn once, replay without a model, repair on drift" is now common, but no open-source tool verifies business correctness on replay.** Skyvern caches code per block and regenerates on failure; HyperAgent has `actionCache`; Midscene caches plans and XPaths locally; Crawlee Python caches LLM-written selectors per route; Crawl4AI's `generate_schema` validates its selectors once. Stagehand's cache became **Browserbase-only** in v4, and Browser Use Cloud reruns still start an agent. Replay checks are "the selector still resolves" or the user's own assertions. The most careful promotion contracts are commercial: Anchor's human-published repair drafts and Momentic's "re-cache only if the rest of the test passes".
- **False success is the top cross-tool pain.** Agents report "done" when nothing happened (Browser Use [#6031](https://github.com/browser-use/browser-use/issues/6031), [#5438](https://github.com/browser-use/browser-use/issues/5438); Stagehand [#239](https://github.com/browserbase/stagehand/issues/239); agent-browser [#2083](https://github.com/vercel-labs/agent-browser/issues/2083)). Independent human evaluation put Browser Use at **30.0%** on Online-Mind2Web against its self-reported 89.1% WebVoyager ([arXiv 2504.01382](https://arxiv.org/abs/2504.01382)). Skyvern credits its validator phase for its WebVoyager jump.
- **Other recurring pains:** cost and tokens (snapshot bloat, "token hog", "Computer Use is 45x more expensive" at 495 HN points); requests to export or rerun without the model; local and non-frontier models, which also fail on structured output; login, 2FA and profile reuse; browser lifecycle flakiness; lock-in to the vendor cloud; upstream churn (LangChain, Stagehand v3→v4, the OpenAI `computer-use-preview` shutdown).
- **Safety controls leak or disclaim themselves.** Domain allowlists are bypassable in Stagehand ([#2945](https://github.com/browserbase/stagehand/issues/2945), [#3002](https://github.com/browserbase/stagehand/issues/3002)), Browser Use ([#5099](https://github.com/browser-use/browser-use/issues/5099), [#5879](https://github.com/browser-use/browser-use/issues/5879)) and Skyvern ([#8577](https://github.com/Skyvern-AI/skyvern/issues/8577)). Playwright MCP states that its secrets, origin filter and server are "not a security boundary". Stagehand's server cache receives `%var%` secret values. agent-browser has the richest opt-in set: vault the model never sees, allowlist that also covers workers and WebRTC, content boundaries, action policies and confirmations.
- **Money limits are rare.** Only Firecrawl `/agent` (`maxCredits`, partial result, resumable) and Crawlee Python's `PydanticAiCrawler` (Pydantic AI `UsageLimits`) stop on spend; most tools stop on step count, and Browser Use users ask for a budget cap ([#5293](https://github.com/browser-use/browser-use/issues/5293)).
- **Interfaces converge on MCP plus code mode plus CLI-and-skills.** All 15 agent frameworks surveyed consume MCP natively or through a first-party add-on; 20 of 22 browser/scraping tools ship an MCP server. Native per-framework packages exist mostly for LangChain, LlamaIndex and the Vercel AI SDK, and the AI SDK wrappers have tiny downloads. Microsoft's Playwright MCP README now steers coding agents to CLI plus skills, Midscene **retired** its MCP servers, and OpenAI recommends code execution over its structured computer tool for its flagship model.
- **Rust cores are not unique any more:** Browser Use's beta runtime, Skyvern's rustwright, BrowserOS's CDP/MCP crates, agent-browser's daemon and Spider are all Rust.

## How to read this note

- Versions, star counts and download figures were read on **2026-10-10** through `gh`, the npm registry and downloads API (week 2026-10-02 to 2026-10-08), PyPI and pypistats, unless a cell says otherwise. Issue counts in parentheses are GitHub reactions (mostly +1) at reading time.
- **Self-reported** means the vendor ran the benchmark on its own product. Benchmark numbers come only from the cited source and were not reproduced. OSWorld and WebVoyager variants differ and must not be compared across rows.
- "(unverified)" marks a claim not traced to a primary source. "None found" or "—" means not found in the sources read, **not** proof of absence.
- Quotes are under 15 words and linked to their author.
- Section 1 is the overview, sections 2 to 5 are per-tool profiles, section 6 covers agent frameworks and adapters, sections 7 and 8 group the pains and praised designs, section 9 is the synthesis for Automan and section 10 the method.

## 1. Overview

Each cell is sourced in the per-tool section it summarizes. "Who drives" says whether a model loop inside the tool decides the steps (loop) or the caller does (code / external agent).

### 1.1 Browser agents, SDKs and MCP/CLI drivers

| Tool | License | Language | Version (date) | Stars | Who drives | Perception | Browser control | Model layer | Replay without a model | Spend cap |
|---|---|---|---|---:|---|---|---|---|---|---|
| Stagehand | MIT | TS, Python, Go | 4.2.0 (2026-10-08) | 25.6k | code or external agent (v4 removed `agent()`) | accessibility tree (+screenshot) | own CDP driver in a Chrome extension | Vercel AI SDK + BYO callback | Browserbase cache only; `act(Action)` | none (metrics only) |
| Browser Use | MIT | Python (+Rust beta) | 0.13.11 (2026-10-07) | 117.5k | built-in loop | DOM + accessibility (+vision) | `cdp-use` | own clients | history rerun; Cloud scripts still start an agent | `max_steps` |
| Skyvern | AGPL-3.0 | Python | 1.0.55 (2026-10-01) | 23.2k | loop, DAG workflow, or code | vision + DOM | Playwright (+rustwright) | LiteLLM | per-block code cache, agent fallback | `max_steps` |
| Magnitude (dormant) | Apache-2.0 | TS | 0.3.1 (2026-02-08) | 4.1k | both | vision coordinates | Patchright | BAML | planned, never shipped | none |
| Nanobrowser | Apache-2.0 | TS | 0.2.0 (2026-10-02) | 14.0k | built-in multi-agent | DOM (+vision) | puppeteer-core in extension | Vercel AI SDK | none | none |
| BrowserOS | AGPL-3.0 | TS, Rust, C++, Go | 0.51.0 (2026-10-05) | 13.9k | external agent via MCP (+own agent) | via tools | Chromium fork, Rust CDP | Vercel AI SDK | none (video replay) | none |
| Playwright MCP | Apache-2.0 | TS | 0.0.83 (2026-09-28) | 38.0k | external agent | accessibility snapshot (vision opt-in) | Playwright | none (client's) | codegen; healer in Playwright Test Agents | none |
| agent-browser | Apache-2.0 | Rust | 0.39.0 (2026-10-09) | 43.8k | external agent | accessibility refs + screenshots, diffs | own Rust CDP daemon | AI Gateway (chat only) | none (deterministic refs) | output cap only |
| Midscene.js | MIT | TS | 1.15.0 (2026-10-10) | 15.1k | code (`aiAct` inner loop) | pure vision | Playwright, Puppeteer, extension, mobile, desktop | `openai` SDK, OpenAI-compatible | local plan + XPath cache | none |
| Notte | SSPL-1.0 | Python | 1.9.7 (2026-10-08) | 2.0k | loop or scripted | DOM element IDs | Patchright or CDP | LiteLLM | agent → generated function; agent fallback | `max_steps` |
| AgentQL | MIT repo, hosted engine | Python, JS | 1.19.0 (2026-05-13) | 1.5k | code | accessibility tree, resolved server-side | Playwright | proprietary hosted | none (re-resolves every run) | n/a |
| Cua | MIT (+FSL, AGPL parts) | Rust, Python, TS, Swift | cua-sdk 0.4.1 (2026-10-05) | 29.3k | external agent | OS accessibility + screenshots | OS-level, VMs | BYO | none | n/a |
| Libretto | MIT | TS | 0.6.45 (2026-08-14) | 0.9k | coding agent writes Playwright | accessibility + screenshot | Playwright | the coding agent's | output is plain Playwright code; network → API | n/a |
| HyperAgent | AGPL-3.0 | TS | npm 1.1.2 (2026-01-07) | 1.6k | code or loop | accessibility (+visual mode) | Playwright / Patchright, CDP first | own clients | `actionCache` XPath replay, LLM fallback | none found |
| Nova Act | Apache-2.0 SDK, AWS service | Python | 3.4.187.0 (2026-04-30) | 0.9k | loop inside each `act()`, code orchestrates | screenshots | Playwright | Nova only | none | `max_steps`; billed per agent hour |

Dormant or archived: LaVague (last commit 2025-01), Agent-E (2025-05), Shortest (last release 2025-04, issues disabled), Laminar Index (archived), Bytebot (archived). Details in [§3](#3-mcp-cli-and-sdk-primitives-vision-agents-testers-and-recorders).

### 1.2 AI extraction and crawl

| Tool | License | Version (date) | Stars | Where AI sits | Model layer | Reuse without a model | Spend cap | Injection defense |
|---|---|---|---:|---|---|---|---|---|
| Firecrawl | AGPL-3.0 (SDKs, MCP MIT) | `firecrawl-py` 4.50.0 (2026-10-08) | 190.2k | per-page JSON extraction, `/agent` loop, `/interact` | Vercel AI SDK | none (`maxAge` page cache) | `/agent` `maxCredits`, partial result | `checkPromptInjection` classifier |
| Crawl4AI | Apache-2.0 + attribution clause | 0.9.4 (2026-09-23) | 85.1k | per-page extraction; `generate_schema` once | LiteLLM fork | generated CSS/XPath schema | usage tracking only | none documented |
| ScrapeGraphAI | MIT | 2.3.1 (2026-10-06) | 31.7k | LLM graphs; code-generating graphs | LangChain 1.x | generated code | none found | none found |
| Spider | MIT | 2.53.9 (2026-09-05) | 2.8k | `spider_agent` loop, self-healing selectors; cloud `/ai/*` | own clients | model-call cache; selectors | `max_rounds`, cost tier | none found |
| Crawlee Python `PydanticAiCrawler` | Apache-2.0 | 1.10.4 (2026-10-06) | 9.6k | per-page or selector generation | Pydantic AI | selectors cached per route label | `UsageLimits` | none found |
| Crawlee JS `@crawlee/stagehand` | Apache-2.0 | 3.18.2 (2026-09-29) | 26.1k | Stagehand v3 primitives in a crawler | Stagehand v3 | Stagehand's | none | none |
| Scrapling | BSD-3 | 0.4.15 (2026-08-23) | 86.7k | none: non-LLM adaptive relocation | n/a | stored element signatures | n/a | n/a |

### 1.3 Hosted browsers and computer-use APIs

| Service | Open parts | Agent loop | AI price unit | Replay/repair | Secrets | Human-in-the-loop |
|---|---|---|---|---|---|---|
| Browserbase (+Director) | Stagehand (MIT) | hosted Director; Stagehand primitives | Model Gateway at token price | server-side action cache; Director exports a Stagehand script | 1Password in Director | live view, takeover |
| Steel | steel-browser (Apache-2.0) | none | none | none (session replay video) | Credentials API, TOTP, field blur | live viewer |
| Hyperbrowser | HyperAgent (AGPL-3.0) | hosted Browser Use, Claude/OpenAI/Gemini CUA, HyperAgent | credits | HyperAgent `actionCache` | none documented | live URL |
| Anchor | SDKs only | browser-use (default), OpenAI CUA, Gemini CU | $0.01 per AI step | AI fallback per step; repair drafts a human publishes | identities with MFA, `secret_values` | `human_intervention`, live view |
| Kernel | kernel-images (Apache-2.0) | none; deploy your own next to the browser | none | none (video replays) | Fill from Vault, Managed Auth with reauth | live view |
| Browserless | SSPL or commercial | MCP `browserless_agent` | none | none | none documented | `liveURL`, `captchaFound` event |
| Bedrock AgentCore Browser | none | none | none | DOM/action recordings | IAM | live view endpoint |
| OpenAI `computer` tool | Agents SDK (MIT) | client runs the loop | model tokens | none | guidance only | `pending_safety_checks` acknowledgement |
| Anthropic computer use | none | client runs the loop | model tokens (~4.5k tokens of tool overhead per request) | none | guidance only | classifiers steer to confirm |
| Gemini computer use | reference repo (Apache-2.0) | client runs the loop | model tokens | none | guidance only | `safety_decision` / `require_confirmation` |

## 2. Browser agents and AI SDKs

Stagehand (and Browserbase Director), Browser Use (library, beta Rust runtime, browser-harness, workflow-use, Cloud), Skyvern, Magnitude, Nanobrowser, BrowserOS.

### 2.1 Stagehand (Browserbase), incl. Director

**Identity.** An "SDK for browser agents": deterministic Chromium driver plus `act`/`observe`/`extract`. [MIT](https://github.com/browserbase/stagehand/blob/main/LICENSE); TypeScript (Node ≥22.18), Python ≥3.11 and Go SDKs generated from one protocol.
- **Version:** `@browserbasehq/stagehand` **4.2.0**, published 2026-10-08 ([npm](https://registry.npmjs.org/@browserbasehq/stagehand)). PyPI `stagehand` is also 4.2.0 (2026-10-08). The `v3-latest` dist-tag is 3.7.3 (2026-08-28); a 4.2.1 alpha is out (2026-10-10).
- **Release-page caveat:** GitHub "latest release" still shows `@browserbasehq/stagehand@3.7.3`, because v4 is not published as GitHub Releases.
- **Stars and activity:** **25,613★** (2026-10-10); last commit 2026-10-09 ([`1fc88499`](https://github.com/browserbase/stagehand/commit/1fc88499)).
- **Downloads/week:** npm 1,692,386 ([api.npmjs](https://api.npmjs.org/downloads/point/last-week/@browserbasehq/stagehand)); PyPI 401,072 ([pypistats](https://pypistats.org/api/packages/stagehand/recent)).

**Architecture.**
- **Who drives:** code-called primitives. v4 dropped the built-in loop: "`agent()` is gone. Nothing in v4 replaces it one-for-one." There are two replacement modes ([migration](https://docs.stagehand.dev/v4/migrations/v3)):
  - **Code mode** (recommended): a coding assistant writes a script once.
  - **Tool calling:** expose the full page/locator API as tools.
- **Perception:** accessibility tree. `page.snapshot()` returns `formattedTree` plus `xpathMap`; `extract({screenshot:true})` adds vision ([extract](https://docs.stagehand.dev/v4/basics/extract)).
- **Browser control:** own CDP driver ("understudy"). The runtime lives in a **Chrome extension service worker**, and the SDKs talk to it via JSON-RPC over CDP ([CONTRIBUTING](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md)).
- **Not Playwright:** v4 has no auto-wait, no `getBy*`, no `route` and no `expect` ([Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright)). Users list the missing Playwright APIs in [#2744](https://github.com/browserbase/stagehand/issues/2744).

**Model layer.**
- **Library:** Vercel AI SDK inside the extension: `ai`, `@ai-sdk/{openai,anthropic,google,groq,cerebras}` ([extension package.json @ 4.2.0 commit](https://github.com/browserbase/stagehand/blob/2d605b099996e10c3e3ab0dcf293db8d482a2c97/packages/extension/package.json)).
- **Model naming:** model names must use one of five native provider prefixes; anything else, including local or self-hosted models and Bedrock or Azure, goes through a **bring-your-own-LLM callback** ([models](https://docs.stagehand.dev/v4/configuration/models)).
- **Gateway:** the optional Browserbase Model Gateway auto-selects a model when `model` is omitted, at "market-price tokens", and works only with Browserbase browsers.

**Interfaces.**
- **SDKs and CLI:** TS/Python/Go SDK, plus the `browse` CLI 0.11.1 ([npm `browse`](https://registry.npmjs.org/browse), 14.3k downloads/week).
- **MCP / agent integrations:** a stdio MCP "facade" plus native bindings exposing `run`/`snapshot`/`screenshot`. `run` executes model-written JS in the extension ([agent frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)).
- **WebMCP:** page-declared tools are supported.
- **Old MCP server archived:** the former [`mcp-server-browserbase`](https://github.com/browserbase/mcp-server-browserbase) (3.4k★) was **archived** 2026-07-20.
- **Batching:** `experimentalBatch()` runs a JS callback next to the browser ([reference](https://docs.stagehand.dev/v4/reference/stagehand)).

**Caching, replay, self-heal.**
- **Server-side only:** the cache sits on Browserbase servers, keyed on instruction + page content + options. Model config is deliberately excluded from the key. A hit returns "with no LLM inference and no token cost".
- **Default and threshold:** on by default with a Browserbase browser. A hit-count threshold sets how many identical results are needed before serving ([caching](https://docs.stagehand.dev/v4/best-practices/caching)).
- **Local browsers:** with a local browser, "the `cache` option has no effect and every call runs inference".
- **Replay and self-heal:** `observe()` → `act(Action)` replays without inference. `selfHeal` re-infers once if the recorded selector no longer resolves ([act](https://docs.stagehand.dev/v4/basics/act)).
- **No semantic check on replay:** a selector that resolves to the wrong element is not detected. The semantic-guard PRs were still unmerged at the 2026-10-07 reading ([PR 2954](https://github.com/browserbase/stagehand/pull/2954)).

**Secrets, human-in-the-loop, cost, safety.**
- **Secrets:** `%var%` variables are never sent to the LLM, **but** "server-side caching… sends variable values to the cache service". The docs say to set `cache:false` on calls that carry credentials ([act](https://docs.stagehand.dev/v4/basics/act)).
- **Human-in-the-loop:** cookbook pattern only ([approve a form submission](https://docs.stagehand.dev/v4/cookbooks/approve-form-submission)); cloud live view.
- **Cost:** `metadata.usage` and `metrics()` report tokens and inference time. There is no budget cap (per the parity list, AI-14).
- **Prompt-injection / domain controls:** `setDomainPolicy` exists, but see [#2945](https://github.com/browserbase/stagehand/issues/2945) and [#3002](https://github.com/browserbase/stagehand/issues/3002). Docs say "Use Browserbase as the isolation boundary for untrusted tasks".

**Evaluations.** An `evals` package lives in the repo ([packages](https://github.com/browserbase/stagehand/tree/main/packages)). No current WebVoyager or Online-Mind2Web number was found for v4 (unverified that none exists).

**Dependencies.**
- **npm 4.2.0 runtime deps:** `zod 4.4.3`, `@browserbasehq/sdk ^2.16`, `@opentelemetry/api`, `@opentelemetry/core`. The AI SDK is bundled inside the extension.
- **PyPI 4.2.0:** `browserbase`, `httpx`, `opentelemetry-api`, `pydantic`, `websockets` ([PyPI JSON](https://pypi.org/pypi/stagehand/4.2.0/json)).

**Agent-framework adapters.** All marked experimental ([packages/integrations](https://github.com/browserbase/stagehand/tree/main/packages/integrations), [overview](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)):
- **Agent frameworks:** Claude Agent SDK, Claude CUA toolset, Codex SDK, Cursor SDK, CrewAI (via MCP), Mastra, Vercel AI SDK, Deep Agents (LangChain), Eve, Pi, fx, Gemini CUA.
- **Not found:** LlamaIndex. A user asked in a [discussion](https://github.com/browserbase/stagehand/discussions) and no adapter was found.

**Browserbase Director.**
- **What it is:** a hosted no-code product that turns plain English into a browser agent and **exports the Stagehand script** of the successful run. Runs can be one-off, scheduled or parallel on Browserbase. 1Password integration and manual takeover cover login and payment steps ([Director page](https://www.browserbase.com/director), [Product Hunt post](https://www.browserbase.com/blog/director-product-hunt)).
- **History:** launched 2025-06-17 with the $40M Series B ([changelog](https://www.browserbase.com/changelog/introducing-director)); "Director 2.0" shipped 2025-10-21 ([changelog](https://www.browserbase.com/changelog/director2)).
- **Openness:** closed source; no public repo found. director.ai returned HTTP 429 to curl.

**User pains** (keyword issue counts on 2026-10-10: `local model` 55, `cache` 29, `login` 18, `ollama` 18, `breaking` 8, `stuck` 8, `slow` 3; several keyword queries hit the search rate limit and were not counted).
- **Model and provider breadth:** "Expand LLMs to Ollama and beyond" [#183](https://github.com/browserbase/stagehand/issues/183) (19); "Support latest Vercel AI SDK (LanguageModelV3)" [#1645](https://github.com/browserbase/stagehand/issues/1645) (16, open); "llm Options" [#307](https://github.com/browserbase/stagehand/issues/307) (14); "No Documentation on Setting Up Locally with Ollama" [#1164](https://github.com/browserbase/stagehand/issues/1164).
- **Repeatability and deterministic replay:**
  - "Return plan for extraction" [#389](https://github.com/browserbase/stagehand/issues/389) (16, open since 2025-01); "XPath Caching and Reuse" [#404](https://github.com/browserbase/stagehand/issues/404) (6); "Can StageHand write scripts that can be rerun… without more OpenAI calls?" [#255](https://github.com/browserbase/stagehand/issues/255).
  - On HN, a commenter asked "How do you avoid this becoming horrendously expensive per run?" ([kevmo314](https://news.ycombinator.com/item?id=42642063)).
- **Integration with existing Playwright code:** "Attach to an existing Playwright Page" [#507](https://github.com/browserbase/stagehand/issues/507) (18); "API gaps blocking a full Playwright exit" [#2744](https://github.com/browserbase/stagehand/issues/2744).
- **Runtime portability:** Cloudflare Workers [#564](https://github.com/browserbase/stagehand/issues/564) (12) and [#164](https://github.com/browserbase/stagehand/issues/164) (11); Lambda bundling [#845](https://github.com/browserbase/stagehand/issues/845); Bun/Node 24 [#1552](https://github.com/browserbase/stagehand/issues/1552); Deno [#3152](https://github.com/browserbase/stagehand/issues/3152).
- **Vendor lock-in / local parity:** "add context persistence to locally run version, or disclose project properly as an onramp to Browserbase" [#492](https://github.com/browserbase/stagehand/issues/492). The v4 removal of the local cache deepens this.
- **Hallucinated completion:** "'Completed' returned as true before extracting entire list" [#239](https://github.com/browserbase/stagehand/issues/239); "extract() totally broken for links" [#912](https://github.com/browserbase/stagehand/issues/912).
- **Docs and product confusion:** a Reddit user lost ~40 minutes choosing between the SDK, the browse CLI and Browserbase ([r/LLMDevs](https://www.reddit.com/r/LLMDevs/comments/1vn530v/confused_about_browserbase_product_lines/), from the 2026-10-07 note, not re-opened). Another user called the docs "thin" ([r/AI_Agents](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/)).
- **Observability ergonomics:** "logging.onLog cannot be used without also writing every log to stderr" [#3035](https://github.com/browserbase/stagehand/issues/3035) (9, open).

**Praised.**
- "actually a useful abstraction here" and built-in proxies/CAPTCHA "delightful" ([sparab18, HN](https://news.ycombinator.com/item?id=42638393)).
- "The act, extract, and observe methods are super clean." ([fbouvier, HN](https://news.ycombinator.com/item?id=42644964)).
- "the sessions actually... complete?", compared with Browser Use ([r/AI_Agents](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/)).
- The maintainer's "fine-grained control a first-class citizen" pitch ([hackgician, HN](https://news.ycombinator.com/item?id=42650486)) frames the primitives-not-loop design that v4 doubled down on.

---

### 2.2 Browser Use (library, beta Rust runtime, CLI/browser-harness, workflow-use, Cloud)

**Identity.**
- **Library:** [MIT](https://github.com/browser-use/browser-use/blob/main/LICENSE), Python ≥3.11. Latest is **0.13.11**, 2026-10-07 ([release](https://github.com/browser-use/browser-use/releases/tag/0.13.11)).
- **Stars and activity:** **117,536★** (2026-10-10); last commit 2026-10-07.
- **Downloads/week:** 1,709,436 on PyPI ([pypistats](https://pypistats.org/api/packages/browser-use/recent)).
- **Three products** ([README @0.13.11](https://github.com/browser-use/browser-use/blob/0.13.11/README.md)):
  1. **Cloud:** hosted agent + browser (API V4).
  2. **CLI:** a skill for Claude Code, Codex and similar agents, installed via `browser-use skill install`.
  3. **Python library:** the OSS agent.

**Architecture.**
- **Who drives (library):** an LLM agent loop (`Agent.run()` → `AgentHistoryList`), with an optional planner, `max_failures`, `max_actions_per_step`, `step_timeout`, `llm_timeout`, `fallback_llm`, loop detection and `flash_mode` ([agent/service.py @0.13.11](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/service.py)).
- **Perception:** hybrid. The DOM is merged with accessibility trees per frame into indexed elements, with optional screenshots (`use_vision`) ([DOM service](https://github.com/browser-use/browser-use/blob/main/browser_use/dom/service.py)).
- **Browser control:** direct CDP via `cdp-use`, a `bubus` event bus and watchdogs ([session.py](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/session.py)). Playwright was dropped earlier.
- **Beta Rust runtime:** `from browser_use.beta import Agent` spawns `browser-use-terminal sdk-server --transport stdio`, a JSON-RPC client to a Rust agent/session/browser server shipped as `browser-use-core` wheels via the `[core]` extra ([integration ledger](https://github.com/browser-use/browser-use/blob/0.13.11/BETA_AGENT_INTEGRATION_FEATURES.md), [beta/service.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/beta/service.py)).
  - **Activity mismatch:** `browser-use-core` is still at 0.13.3 (2026-08-16), and the [`browser-use/terminal`](https://github.com/browser-use/terminal) repo (MIT, 650★) was last pushed 2026-08-16 (last release v0.1.8, 2026-06-12), while the Python wrapper keeps moving.
- **browser-harness:**
  - **What it is:** [MIT](https://github.com/browser-use/browser-harness), 18,376★, v0.1.13 (2026-09-04), last commit 2026-10-09.
  - **Design:** one editable CDP websocket to the user's real Chrome. The agent "writes missing helpers as it works", and an MCP server (`browser-harness-mcp`) is included ([README](https://github.com/browser-use/browser-harness)).
  - **Dependencies:** `cdp-use`, `fetch-use`, `pillow`, `websockets`.
  - **Downloads caveat:** its ~1.64M PyPI downloads/week are inflated because `browser-use` 0.13.11 **pins `browser-harness==0.1.13` as a hard dependency** ([pyproject](https://github.com/browser-use/browser-use/blob/0.13.11/pyproject.toml)).
- **workflow-use:**
  - **What it is:** [AGPL-3.0](https://github.com/browser-use/workflow-use), 4,217★. PyPI v0.2.11 dates from 2025-11-19; the last commit (2026-08-27) was a dependency-vulnerability fix.
  - **Mechanism:** record or generate a parameterized "semantic workflow", replay it without AI, and fall back to Browser Use when a step fails.
  - **Maturity:** the README warns "very early development… don't recommend using this in production".

**Model layer.**
- **Own thin clients:** `browser_use.llm` ships ChatOpenAI, ChatAnthropic, ChatGoogle, ChatGroq, ChatOllama, Azure, AWS, OCI, DeepSeek, Mistral, Cerebras, OpenRouter and a Vercel AI Gateway client ([llm/](https://github.com/browser-use/browser-use/tree/0.13.11/browser_use/llm)). The SDKs are hard dependencies: `openai`, `anthropic`, `google-genai`, `groq`, `ollama`.
- **LiteLLM:** an optional `ChatLiteLLM` wrapper only. LiteLLM is not a dependency, after a malicious-litellm supply-chain incident: "[Security] CLI install script pulls malicious litellm 1.82.8" [#4505](https://github.com/browser-use/browser-use/issues/4505) (9).
- **Own model:** BU2 (`ChatBrowserUse(model='bu-2-0')`) via the Browser Use gateway is recommended.
- **Local models:** supported via Ollama.

**Interfaces.**
- **Library:** Python agent with custom tools (`Tools().action`).
- **MCP:** server in OSS ([MCP doc](https://docs.browser-use.com/open-source/customize/integrations/mcp-server)) and in Cloud.
- **CLI:** `browser-use` / `bu` / `browser` entry points.
- **Cloud:** REST API V4 with workspaces, runs and events, plus an x402 agent-wallet payment path ([docs index](https://docs.browser-use.com/llms.txt)).

**Caching, replay, self-heal, verification.**
- **Library replay:** `rerun_history` / `load_and_rerun` in the beta agent ([beta/service.py L6206](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/beta/service.py)).
- **Success judging:** the **LLM judge is on by default** (`use_judge=True`), with optional `ground_truth` ([agent/judge.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/judge.py), [views.py](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/agent/views.py)).
- **Cloud "rerunnable scripts":** the agent saves a tested `script.py` in a workspace. Each later run "starts an agent again" (still consuming tokens), re-fetches live data and repairs the code if the site changed ([scripts](https://docs.browser-use.com/cloud/agent/scripts)).
- **Retired feature:** Cloud Skills are retired; existing ones still run.

**Secrets, human-in-the-loop, cost, safety.**
- **Secrets:**
  - `sensitive_data` placeholders, optionally scoped per domain pattern ([sensitive data](https://docs.browser-use.com/open-source/examples/templates/sensitive-data)).
  - A placeholder suffixed `bu_2fa_code` auto-generates a TOTP via `pyotp` ([registry/service.py L473](https://github.com/browser-use/browser-use/blob/0.13.11/browser_use/tools/registry/service.py)).
  - Cloud V4 has `secretBindings` with allowed hostnames, documented with the caveat that aliases "do not guarantee that the agent cannot see the value" ([secrets](https://docs.browser-use.com/cloud/guides/secrets)). It also supports 1Password with TOTP.
- **Human-in-the-loop:** Cloud V4 provides a `live_view_url`, human takeover, then a follow-up run in the same session ([HITL](https://docs.browser-use.com/cloud/agent/human-in-the-loop)). OSS HITL was the 18-reaction request [#221](https://github.com/browser-use/browser-use/issues/221).
- **Cost:** `calculate_cost=True` gives `history.usage` and a token-cost service ([costs](https://docs.browser-use.com/open-source/development/monitoring/costs)). Cloud has `GET session cost`. There is no hard budget cap in the library; `max_steps` is the bound. Users propose one in [#5293](https://github.com/browser-use/browser-use/issues/5293).
- **Prompt injection / domains:** `allowed_domains` / `prohibited_domains` on the profile, with known bypasses ([#5099](https://github.com/browser-use/browser-use/issues/5099), [#5879](https://github.com/browser-use/browser-use/issues/5879)).
- **Telemetry:** PostHog telemetry is a hard dependency (`posthog==7.7.0`). Early issues [#282](https://github.com/browser-use/browser-use/issues/282) (19) and [#265](https://github.com/browser-use/browser-use/issues/265) (10) were PostHog SSL errors.

**Evaluations.**
- **Self-reported:** 89.1% WebVoyager (Dec 2024, [post](https://browser-use.com/posts/sota-technical-report)).
- **Independent:** **30.0%** on Online-Mind2Web, human-evaluated by OSU/Berkeley (COLM 2025) ([arXiv 2504.01382](https://arxiv.org/abs/2504.01382)).
- **Self-reported Cloud v3:** `bu-max` "291/300 = 97%" on Online-Mind2Web, where pass/fail was judged by Claude over the agent's final response, not by humans ([browser-use/online-mind2web](https://github.com/browser-use/online-mind2web)).
- **Own benchmark:** "BU Bench V2" (vendor-built, 60-task subset shown in README).

**Dependencies.** `cdp-use`, `bubus`, `pydantic`, `httpx`, `aiohttp`, `mcp`, `posthog`, `pyotp`, provider SDKs, `browser-use-sdk`, `browser-harness`. Optional extras: `core` (Rust wheels), `aws`, `oci`, `video`, `eval` (`lmnr`) ([pyproject](https://github.com/browser-use/browser-use/blob/0.13.11/pyproject.toml)).

**Agent-framework adapters.**
- **Anthropic SDK:** "Browser Use toolsets for Claude" implements Claude's 31 browser actions ([README](https://github.com/browser-use/browser-use/blob/0.13.11/README.md)).
- **Coding agents and agent platforms:** CLI skill for Claude Code, Codex, Cursor, Hermes, OpenClaw and Pi; Claude Managed Agents; Vercel Marketplace/eve; n8n ([docs index](https://docs.browser-use.com/llms.txt)).
- **LangChain:** only an `examples` extra (`langchain-openai`).
- **Not found:** first-party OpenAI Agents SDK, Pydantic AI, Mastra, CrewAI, LlamaIndex and ADK adapters (unverified that none exists; ADK users ask on [r/agentdevelopmentkit](https://www.reddit.com/r/agentdevelopmentkit/comments/1k3w4n0/browseruse_vs_stagehand_for_web_browser_agents)).

**User pains** (keyword issue counts on 2026-10-10: `tokens` 182, `login` 164, `cache` 128, `ollama` 102, `stuck` 93, `slow` 79, `iframe` 62, `sensitive_data` 56, `token cost` 50, `shadow` 44, `captcha` 42, `breaking change` 26, `infinite loop` 23, `cloudflare` 19, `prompt injection` 15, `2FA` 13, `memory leak` 8).
- **Flakiness / stuck browser (historically dominant):**
  - "Chromium stuck, need manual intervention" [#839](https://github.com/browser-use/browser-use/issues/839) (14 reactions, 41 comments); "blank chromium page at step 1" [#1020](https://github.com/browser-use/browser-use/issues/1020) (14/37); "Chrome ≥136 no longer supports CDP with default user-data-dir" [#1520](https://github.com/browser-use/browser-use/issues/1520) (10/36).
  - Recent: no page-crash recovery [#5067](https://github.com/browser-use/browser-use/issues/5067); CDP instability on Windows [#5048](https://github.com/browser-use/browser-use/issues/5048).
- **False success / hallucinated clicks:**
  - Known-answer regression set [#6031](https://github.com/browser-use/browser-use/issues/6031); "click… report 'element not found' as success" [#5438](https://github.com/browser-use/browser-use/issues/5438); MCP failures reported with `isError=false` [#5252](https://github.com/browser-use/browser-use/issues/5252).
  - A Reddit user spent "two weeks watching browser-use hallucinate clicks" ([r/AI_Agents](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/)).
- **Speed and cost:**
  - "ability to rerun same thing faster" [#763](https://github.com/browser-use/browser-use/issues/763) (11); "Export a generated script" [#943](https://github.com/browser-use/browser-use/issues/943) (11).
  - "Browser use is a token hog" ([tonyww, HN](https://news.ycombinator.com/item?id=47906391)); "relatively expensive to run and are very slow" ([deadfece, HN](https://news.ycombinator.com/item?id=43201407)).
- **Local and open models:** Gpt-OSS/Ollama [#2605](https://github.com/browser-use/browser-use/issues/2605) (19); Ollama example broken [#584](https://github.com/browser-use/browser-use/issues/584) (8). A maintainer said Llama "is often confused by the structured output format" ([HN](https://news.ycombinator.com/item?id=43181548)).
- **Login, 2FA and anti-bot:** manual data entry for login/MFA [#404](https://github.com/browser-use/browser-use/issues/404) (11); Capsolver integration [#652](https://github.com/browser-use/browser-use/issues/652) (9); human-like browsing [#947](https://github.com/browser-use/browser-use/issues/947) (20).
- **Security:** CDP/no-sandbox concerns raised at launch ([arjunchint, HN](https://news.ycombinator.com/item?id=43174273)); supply-chain incident [#4505](https://github.com/browser-use/browser-use/issues/4505); allowlist bypasses (above).
- **Docs and breaking changes:** "validation error with Browser and BrowserSession classes" [#1815](https://github.com/browser-use/browser-use/issues/1815) (21 comments); stale profile-sync docs [#6020](https://github.com/browser-use/browser-use/issues/6020).
- **browser-harness:** the top issue is "FAKE self healing" [#145](https://github.com/browser-use/browser-harness/issues/145) (13). It argues "self-healing" just means the LLM appends helpers. Other open issues: daemon liveness [#865](https://github.com/browser-use/browser-harness/issues/865) and screenshot hangs on hidden tabs [#882](https://github.com/browser-use/browser-harness/issues/882). The install.md approach was called "long and complex" ([jadbox, HN](https://news.ycombinator.com/item?id=47897630)).
- **workflow-use:** recorder extension fails to load [#99](https://github.com/browser-use/workflow-use/issues/99) (5) and [#165](https://github.com/browser-use/workflow-use/issues/165); iframes not recorded [#72](https://github.com/browser-use/workflow-use/issues/72); "How to write assertions?" [#79](https://github.com/browser-use/workflow-use/issues/79).

**Praised.**
- "Browser-use is incredible." ([joemazerino, HN](https://news.ycombinator.com/item?id=47897927)).
- Browser Harness was called "Absolutely stunning." ([michelhabib, HN](https://news.ycombinator.com/item?id=47911893)).
- The maintainers' pitch to "extract xPaths from the agent runs and re-run these scripts deterministically" ([MagMueller, HN](https://news.ycombinator.com/item?id=43181564)) is the pattern users keep asking for.
- One user called real-profile reuse for logged-in sites the practical answer to LinkedIn-style login ([HN](https://news.ycombinator.com/item?id=43175311)).

---

### 2.3 Skyvern

**Identity.**
- **License and language:** [AGPL-3.0](https://github.com/Skyvern-AI/skyvern/blob/main/LICENSE); Python 3.11–3.14, plus a TS client.
- **Version:** **v1.0.55**, 2026-10-01 ([release](https://github.com/Skyvern-AI/skyvern/releases/tag/v1.0.55)).
- **Stars and activity:** **23,179★** (2026-10-10); last commit 2026-10-09 ("Task V3" post-action screenshot settle).
- **Downloads/week:** PyPI 1,569 (most usage is likely Cloud or Docker); npm `@skyvern/client` 3,694.

**Architecture.**
- **Who drives:** both modes.
  - **Agent:** a planner/actor/validator "swarm of agents" ([README](https://github.com/Skyvern-AI/skyvern)), plus a DAG workflow engine with loops, conditionals and blocks.
  - **Code-called:** a Playwright-compatible SDK where `page.click(prompt=…)`, `page.act`, `page.extract`, `page.validate` (returns bool) and `page.agent.run_task/login/download_files` mix with plain selectors.
- **Perception:** vision LLM over screenshots plus DOM.
- **Browser control:** Playwright-managed Chromium, with CDP connect to external Chrome. The new [rustwright](https://github.com/Skyvern-AI/rustwright) is an optional extra (`skyvern[rustwright]`). rustwright is MIT, 913★, "Playwright's API on a Rust CDP engine… no driver subprocess. (Alpha)", 0.3.0 on PyPI (2026-08-24).

**Model layer.** LiteLLM: `litellm>=1.84` in the `local`/`server` extras ([pyproject @v1.0.55](https://github.com/Skyvern-AI/skyvern/blob/v1.0.55/pyproject.toml)). Supported: OpenAI, Anthropic, Azure, Bedrock, Gemini, Ollama and any OpenAI-compatible endpoint ([local LLMs](https://www.skyvern.com/docs/integrations/local-llms.md)). The server also pulls `openai-agents`.

**Interfaces.** REST API, Python and TS SDKs, a CLI with "skills", an MCP server (`fastmcp`) ([MCP](https://www.skyvern.com/docs/developers/getting-started/mcp.md)), a no-code workflow builder UI, and webhooks.

**Caching, replay, self-heal.**
- **Recording:** "Code Caching" records a successful run and generates code. Later runs use `run_with="code"`: "No screenshots, no LLM reasoning".
- **Fallback:** "If the cached code hits something unexpected… Skyvern re-runs with the full agent and regenerates the cache".
- **Granularity:** caches are **per block**, with **progressive caching across branches**. Conditional, wait and code blocks always run live ([code caching](https://www.skyvern.com/docs/developers/features/code-caching.md)).
- **Verification:** an LLM validator phase. Skyvern 2.0 attributes its jump to 85.85% to "a Validation phase" after observing that tasks reported success when they had not succeeded ([2.0 blog](https://www.skyvern.com/blog/skyvern-2-0-state-of-the-art-web-navigation-with-85-8-on-webvoyager-eval/)). The OSS also contains a [self-heal reliability service](https://github.com/Skyvern-AI/skyvern/blob/main/skyvern/services/self_heal_reliability_service.py).

**Secrets, human-in-the-loop, cost, safety.**
- **Secrets:**
  - A credential layer over external vaults: Bitwarden by default, 1Password, Azure Key Vault or custom. "Secrets never appear in logs, API responses, or LLM prompts" ([store credentials](https://www.skyvern.com/docs/developers/credentials/store-credentials.md)).
  - 2FA via stored TOTP secret, pushed codes, pull-from-your-endpoint or magic links ([2FA](https://www.skyvern.com/docs/developers/credentials/handle-2fa.md)).
  - A raw-secret safety screen exists ([#8849](https://github.com/Skyvern-AI/skyvern/issues/8849)).
- **Human-in-the-loop:** a `HUMAN_INTERACTION` workflow block ([client type](https://github.com/Skyvern-AI/skyvern/blob/main/skyvern/client/types/human_interaction_block.py)) and webhooks for runs that "require human intervention".
- **Cost:** a `max_steps` cap ends runs as `timed_out`. Credits are sold at ~1,200 actions for $29 and ~6,200 for $149 ([cost control](https://www.skyvern.com/docs/developers/optimization/cost-control.md)). Self-hosters ask for token-usage exposure ([#6226](https://github.com/Skyvern-AI/skyvern/issues/6226)).
- **Domain controls:** a blocked-host gate exists but is partial ([#8577](https://github.com/Skyvern-AI/skyvern/issues/8577)). A CodeBlock sandbox escape was reported and closed ([#6868](https://github.com/Skyvern-AI/skyvern/issues/6868)).

**Evaluations.**
- **WebVoyager:** 85.85%, self-reported, with full run published at eval.skyvern.com for audit ([blog](https://www.skyvern.com/blog/skyvern-2-0-state-of-the-art-web-navigation-with-85-8-on-webvoyager-eval/)).
- **WebBench (co-built by Halluminate and Skyvern):** 2,454 open tasks on 452 sites, split READ/WRITE. Per the vendor, Skyvern 2.0 was best on WRITE tasks and Claude 3.7 CUA best on READ ([Skyvern blog](https://blog.skyvern.com/web-bench-a-new-way-to-compare-ai-browser-agents/), [WebBench](https://github.com/Halluminate/WebBench)). This is not independent, because Skyvern co-authored it.

**Dependencies.** Base install is lightweight (pydantic, httpx, structlog, typer, rich). The `local`/`server` extras add `litellm`, `playwright`, `fastapi`, `sqlalchemy`, `anthropic`, `openai`, `fastmcp` and `openai-agents`; `cloud` adds `temporalio` and OpenTelemetry.

**Agent-framework adapters.**
- **Shipped:** `skyvern-langchain` 0.2.1 and `skyvern-llamaindex` 0.2.2, both 2026-01-07 ([PyPI](https://pypi.org/project/skyvern-langchain/)), plus MCP.
- **No-code connectors:** Zapier, Make, n8n, Workato.
- **Rejected:** an AG2 integration request was closed ([#5369](https://github.com/Skyvern-AI/skyvern/issues/5369)).
- **Not found:** Vercel AI SDK, Mastra, CrewAI and ADK adapters.

**User pains** (keyword counts: `cache` 44, `login` 30, `tokens` 27, `ollama` 18, `stuck` 14, `captcha` 8, `slow` 7).
- **Self-hosting and install friction (dominant):** Docker + Claude models [#312](https://github.com/Skyvern-AI/skyvern/issues/312) (24 comments); WSL Docker browser [#733](https://github.com/Skyvern-AI/skyvern/issues/733) (17); Windows psycopg [#3546](https://github.com/Skyvern-AI/skyvern/issues/3546); circular import on main [#7762](https://github.com/Skyvern-AI/skyvern/issues/7762); missing env docs [#6324](https://github.com/Skyvern-AI/skyvern/issues/6324).
- **Model breadth and local models:** OpenRouter [#991](https://github.com/Skyvern-AI/skyvern/issues/991) (8); Ollama [#239](https://github.com/Skyvern-AI/skyvern/issues/239) (3) and [#92](https://github.com/Skyvern-AI/skyvern/issues/92); "local vision model" [#180](https://github.com/Skyvern-AI/skyvern/issues/180). Vision dependence made local support late ([suchintan, HN](https://news.ycombinator.com/item?id=41939149)).
- **Speed and cost:** "Why is the task running so slowly?" [#1271](https://github.com/Skyvern-AI/skyvern/issues/1271); "Task Execution Performance" [#4375](https://github.com/Skyvern-AI/skyvern/issues/4375). One HN commenter was interested "if we can get the cost down at scale" ([lyime](https://news.ycombinator.com/item?id=42742916)).
- **Credentials:** Bitwarden master-password errors [#2604](https://github.com/Skyvern-AI/skyvern/issues/2604) (12 comments).
- **Correctness doubts:** "what ensures it's the correct one in that context?" [#5637](https://github.com/Skyvern-AI/skyvern/issues/5637). Trust for purchases was also questioned ([happyopossum, HN](https://news.ycombinator.com/item?id=42742284)).

**Praised.**
- "I especially like the observability around the workflow and the steps" ([lyime, HN](https://news.ycombinator.com/item?id=42742916)).
- "super cool that you've open sourced it under the AGPL" ([mmaunder, HN](https://news.ycombinator.com/item?id=41937339)).
- Publishing full eval traces for independent audit was well received ([suchintan, HN](https://news.ycombinator.com/item?id=42744200)).

---

### 2.4 Magnitude (browser-agent), now dormant

**Identity.**
- **License and language:** [Apache-2.0](https://github.com/magnitudedev/browser-agent/blob/main/LICENSE); TypeScript.
- **Versions:** `magnitude-core` **0.3.1**, `magnitude-test` 0.3.13 and `magnitude-mcp` 0.1.3, all **2026-02-08**.
- **Stars and activity:** **4,134★**; the last commit, 2026-02-08, was the release commit.
- **Downloads/week:** npm `magnitude-core` 7,486, `magnitude-test` 1,502, `magnitude-mcp` 15.
- **Pivot:** the org's active repo is now [`magnitudedev/magnitude`](https://github.com/magnitudedev/magnitude) (6,625★), "Open source inference engine for agents". It was launched on HN 2026-09-30 as "Self-optimizing inference engine for agents" ([HN 49911995](https://news.ycombinator.com/item?id=49911995)).

**Architecture.**
- **Who drives:** both. `agent.act()` accepts high-level goals or single steps, `agent.extract(zod)` extracts, and there is a test runner with AI `check()` assertions.
- **Perception:** **vision-first**. A visually grounded model returns pixel coordinates; extraction uses the DOM ([README](https://github.com/magnitudedev/browser-agent)).
- **Browser control:** Playwright, aliased to **Patchright** (`"playwright": "npm:patchright@^1.52.0"`); the full Page/Context is accessible.

**Model layer.** [BAML](https://github.com/magnitudedev/browser-agent/blob/main/packages/magnitude-core/package.json) (`@boundaryml/baml`).
- **Providers:** anthropic, claude-code, aws-bedrock, google-ai, vertex-ai, openai, azure-openai, openai-generic ([types.ts](https://github.com/magnitudedev/browser-agent/blob/main/packages/magnitude-core/src/ai/types.ts)).
- **Model requirement:** a "large visually grounded model" is required (Claude Sonnet 4 or Qwen-2.5VL 72B).

**Interfaces.** TS library, test-runner CLI, MCP server (`magnitude-mcp`), `create-magnitude-app`.

**Caching and replay.** "Deterministic runs via native caching system *(in progress)*" ([README](https://github.com/magnitudedev/browser-agent)). It was still in progress when development stopped; there is an open request "Caching actions after agent takes them" [#132](https://github.com/magnitudedev/browser-agent/issues/132). Only Anthropic prompt caching and memory with screenshot masking exist ([memory/](https://github.com/magnitudedev/browser-agent/tree/main/packages/magnitude-core/src/memory)).

**Secrets, HITL, cost, safety.** Data passes through the `act(…, {data})` option. No vault, HITL or budget features were found (unverified). An open security report covers command injection via the config file (`spawn` with `shell:true`, [#155](https://github.com/magnitudedev/browser-agent/issues/155)).

**Evaluations.** WebVoyager **93.9%** pass@1 (self-reported), with traces and a reproduction repo ([magnitudedev/webvoyager](https://github.com/magnitudedev/webvoyager)). No independent run was found.

**Dependencies.** `@boundaryml/baml`, `patchright`, `sharp`, `zod` 3, `pino`, `posthog-node`, `magnitude-extract`.

**Adapters.** MCP only. No framework adapters were found.

**User pains** (small issue base; top issue has 3 reactions).
- **Model and provider errors:** `BamlClientHttpError 404` [#146](https://github.com/magnitudedev/browser-agent/issues/146); Groq via BAML [#46](https://github.com/magnitudedev/browser-agent/issues/46); Bedrock image size on Retina [#144](https://github.com/magnitudedev/browser-agent/issues/144).
- **Interaction gaps:** file upload fails [#152](https://github.com/magnitudedev/browser-agent/issues/152); Vue filterable selects [#154](https://github.com/magnitudedev/browser-agent/issues/154); timeout/retry for `act()` [#121](https://github.com/magnitudedev/browser-agent/issues/121); machine-readable output [#11](https://github.com/magnitudedev/browser-agent/issues/11) (3).
- **Cost of vision testing:** "must 10000x the cost and speed, doesn't it?" ([axlee, HN](https://news.ycombinator.com/item?id=44391985)). The maintainer concedes Claude "can get expensive in some cases" ([anerli, HN](https://news.ycombinator.com/item?id=44392151)).

**Praised.** Coordinates-over-set-of-marks was argued as the differentiator: the founder notes Browser Use's set-of-marks keeps it "strongly coupled to the DOM" ([anerli, HN](https://news.ycombinator.com/item?id=44398292)). Mixing agent steps with your own code was offered as the reason to use a framework over raw Claude ([anerli, HN](https://news.ycombinator.com/item?id=44390404)). The Show HN posts drew 179 and 145 points ([43796003](https://news.ycombinator.com/item?id=43796003), [44390005](https://news.ycombinator.com/item?id=44390005)).

---

### 2.5 Nanobrowser

**Identity.**
- **What it is:** Chrome/Edge extension, "Open-Source… Alternative to OpenAI Operator". [Apache-2.0](https://github.com/nanobrowser/nanobrowser); TypeScript.
- **Version:** **v0.2.0**, 2026-10-02 ([release](https://github.com/nanobrowser/nanobrowser/releases/tag/v0.2.0)); the previous release was v0.1.13 (2025-11-22).
- **Stars and activity:** **14,048★**; last commit 2026-10-02. The cadence is slow: the gap between the last two releases is 10 months.

**Architecture.**
- **Who drives:** a multi-agent loop (Planner + Navigator; earlier also a Validator), run in the extension background ([agents/](https://github.com/nanobrowser/nanobrowser/tree/master/src/background/agent/agents)).
- **Perception:** DOM via an injected `buildDomTree.js` (Browser-Use-derived indexed elements), plus optional screenshots.
- **Browser control:** `puppeteer-core` inside the extension, over chrome.debugger CDP ([package.json](https://github.com/nanobrowser/nanobrowser/blob/master/package.json)).

**Model layer.** v0.2.0 moved "LLM calls… on the Vercel AI SDK instead of LangChain" (`ai` 7, `@ai-sdk/{openai,anthropic,google,azure,deepseek,xai}`, OpenRouter provider).
- **Local models:** Ollama goes through its OpenAI-compatible API.
- **Breaking change:** Groq, Cerebras and Llama API providers were **removed**, and their saved keys are deleted on upgrade ([v0.2.0 notes](https://github.com/nanobrowser/nanobrowser/releases/tag/v0.2.0)).
- **Per-agent models:** each agent can use a different model.

**Interfaces.** Side-panel chat only. MCP client support is an open request ([#123](https://github.com/nanobrowser/nanobrowser/issues/123)); there is no API or CLI.

**Caching and replay.** None. "Replay of tasks" is an open request ([#147](https://github.com/nanobrowser/nanobrowser/issues/147)), as is a persistent knowledge store ([#173](https://github.com/nanobrowser/nanobrowser/issues/173)).

**Secrets, HITL, cost, safety.**
- **Secrets:** keys stay local ("Your credentials stay with you").
- **Prompt injection:** a `guardrails` sanitizer wraps page content in `<nano_untrusted_content>` tags and filters patterns ([messages/utils.ts](https://github.com/nanobrowser/nanobrowser/blob/master/src/background/agent/messages/utils.ts), [guardrails/](https://github.com/nanobrowser/nanobrowser/tree/master/src/background/services/guardrails)).
- **Domain controls:** a URL **firewall** (allow/deny) setting ([firewall.ts](https://github.com/nanobrowser/nanobrowser/blob/master/src/storage/lib/settings/firewall.ts)).
- **HITL and cost:** no HITL pause or cost accounting found (unverified).

**Evaluations.** None published (none found).

**Dependencies.** Vercel AI SDK, `puppeteer-core`, React, `zod` 4, `jsonrepair`, `posthog-js`, WXT build.

**Adapters.** None. It is an end-user product.

**User pains** (top reactions are all provider requests).
- **Provider and local-model breadth:** Firefox [#37](https://github.com/nanobrowser/nanobrowser/issues/37) (9); Ollama [#16](https://github.com/nanobrowser/nanobrowser/issues/16) (7); Groq [#120](https://github.com/nanobrowser/nanobrowser/issues/120) (7, a provider later removed); LM Studio [#35](https://github.com/nanobrowser/nanobrowser/issues/35) (5); OpenRouter [#19](https://github.com/nanobrowser/nanobrowser/issues/19) (5); Bedrock [#132](https://github.com/nanobrowser/nanobrowser/issues/132).
- **Structured-output failures with non-frontier models:** glm-4.7 [#291](https://github.com/nanobrowser/nanobrowser/issues/291); think-tag stripping wipes content [#330](https://github.com/nanobrowser/nanobrowser/issues/330) and [#337](https://github.com/nanobrowser/nanobrowser/issues/337).
- **Interaction reliability:** dropdowns [#310](https://github.com/nanobrowser/nanobrowser/issues/310); scrolling [#149](https://github.com/nanobrowser/nanobrowser/issues/149) and [#253](https://github.com/nanobrowser/nanobrowser/issues/253); a fixed post-action sleep [#336](https://github.com/nanobrowser/nanobrowser/issues/336).
- **Missing reuse:** replay [#147](https://github.com/nanobrowser/nanobrowser/issues/147) (3); memories [#144](https://github.com/nanobrowser/nanobrowser/issues/144) (4).

**Praised.** Free, BYO-key and local is the draw: "100% Free… Privacy-Focused" ([README](https://github.com/nanobrowser/nanobrowser)). No independent praise quotes of substance were found; HN traction was low (9 points, [43261000](https://news.ycombinator.com/item?id=43261000)).

---

### 2.6 BrowserOS (incl. BrowserOS neo)

**Identity.**
- **Repo:** [AGPL-3.0](https://github.com/browseros-ai/BrowserOS/blob/main/LICENSE) monorepo; languages are TypeScript, Rust, Python, C++ and Go ([languages API](https://api.github.com/repos/browseros-ai/BrowserOS/languages)).
- **Two products:**
  - **BrowserOS:** a Chromium fork with a built-in agent, **v0.51.0** (2026-10-05, Chromium 155).
  - **BrowserOS neo / "BrowserClaw":** a secondary local browser for coding agents over MCP. Recent tags: `ext-browserclaw/v0.2.34.0` (2026-10-09), `claw-server/v0.0.68` (2026-10-07).
- **Stars and activity:** **13,864★**; last commit 2026-10-09; very high release cadence.

**Architecture.**
- **Who drives (neo):** the external coding agent (Claude Code, Codex, Cursor, OpenClaw, Hermes…) through MCP ([README](https://github.com/browseros-ai/BrowserOS)).
- **Who drives (BrowserOS):** its own agent, plus a visual workflow builder.
- **Browser control:** a forked Chromium. A Rust CDP and MCP stack (`crates/browseros-cdp`, `browseros-core`, `browseros-mcp` with `rmcp`, `tokio`, `rquickjs` ([Cargo.toml](https://github.com/browseros-ai/BrowserOS/blob/main/packages/browseros-agent/crates/browseros-mcp/Cargo.toml))) sits beside a Bun/Hono TS server and a Go CLI.
- **Why a fork:** the claimed rationale is that extensions cannot see cross-origin iframes and are bot-detectable ([johnsmith1840, HN](https://news.ycombinator.com/item?id=46726210)).
- **Agent tab isolation:** agent tabs are grouped separately from the user's, and closing a session closes its tabs ([tabs doc](https://github.com/browseros-ai/BrowserOS/blob/main/docs/neo/tabs-and-isolation.mdx)).

**Model layer.** Vercel AI SDK v7 (`ai`, `@ai-sdk/{anthropic,openai,google,azure,amazon-bedrock,openai-compatible,mcp}`, OpenRouter) plus `acpx` to drive coding agents ([server package.json](https://github.com/browseros-ai/BrowserOS/blob/main/packages/browseros-agent/apps/server/package.json)). Ollama and LM Studio are supported. v0.51.0 "retired" the built-in BrowserOS model and removed the account requirement ([release](https://github.com/browseros-ai/BrowserOS/releases/tag/v0.51.0)).

**Interfaces.** An MCP server (31 tools per an older README; unverified for neo), one-click connect for coding agents, a Go CLI, a workflow graph builder, and scheduled tasks.

**Caching and replay.** Session replay is **video plus an action timeline for audit**, not re-execution: "Every session is saved as a scrubbable video on your disk" ([README](https://github.com/browseros-ai/BrowserOS)). Workflows exist in BrowserOS (classic); no deterministic code cache was found.

**Secrets, HITL, cost, safety.**
- **Secrets / login:** imports Chrome logins, passwords and cookies so agents act as the user. That is powerful, but it is the widest blast radius in this set.
- **Privacy:** local-only storage under `~/.browserclaw/`.
- **HITL:** a live dashboard shows each agent's tab.
- **Access control:** "IAM for agents" enforcement at the Chromium click level was claimed as early-stage ([felarof, HN](https://news.ycombinator.com/item?id=46724720)).
- **Cost:** the README claims "fewer tokens" than Claude's Chrome extension and the Codex browser, without published data.

**Evaluations.** None published (none found).

**Dependencies.** Listed under Architecture and Model layer above.

**Adapters.** MCP to Claude Code, Codex, Cursor, VS Code, Zed, OpenCode, Hermes, OpenClaw and Antigravity. No library-level framework adapters.

**User pains.**
- **Platform and packaging (dominant):** Linux for neo [#2174](https://github.com/browseros-ai/BrowserOS/issues/2174) (23) and [#2650](https://github.com/browseros-ai/BrowserOS/issues/2650) (9); Flatpak [#164](https://github.com/browseros-ai/BrowserOS/issues/164) (21); Windows sandbox start failure [#2408](https://github.com/browseros-ai/BrowserOS/issues/2408) (6); console pop-ups [#1492](https://github.com/browseros-ai/BrowserOS/issues/1492) (6).
- **Agent extension auto-deleting:** [#378](https://github.com/browseros-ai/BrowserOS/issues/378) (21) and [#388](https://github.com/browseros-ai/BrowserOS/issues/388) (21).
- **MCP connectivity after updates:** [#2865](https://github.com/browseros-ai/BrowserOS/issues/2865) and [#2866](https://github.com/browseros-ai/BrowserOS/issues/2866); remote MCP clients [#178](https://github.com/browseros-ai/BrowserOS/issues/178) (9).
- **Ethics and trust:** "agent does not respect or request robots.txt" [#3](https://github.com/browseros-ai/BrowserOS/issues/3) (9); "Make BrowserOS not rely on anything google" [#131](https://github.com/browseros-ai/BrowserOS/issues/131) (7).
- **Fork-vs-extension skepticism:** "I still don't buy the… whole Browser and not a Chrome Extension argument" ([arjunchint, HN](https://news.ycombinator.com/item?id=46725255)).

**Praised.** Local-first, logged-in, bring-your-own-agent is the pitch users star for. One commenter wants it to work "like Claude in chrome" with desktop objects ([Johnny_Bonk, HN](https://news.ycombinator.com/item?id=46724325)). Few substantive third-party praise quotes were found.

---

### 2.7 Cross-tool comparison

| | Stagehand 4.2 | Browser Use 0.13 | Skyvern 1.0.55 | Magnitude 0.3 | Nanobrowser 0.2 | BrowserOS 0.51 |
|---|---|---|---|---|---|---|
| License | MIT | MIT (workflow-use AGPL) | AGPL-3.0 | Apache-2.0 | Apache-2.0 | AGPL-3.0 |
| Driver | own CDP via extension | cdp-use (Py); Rust beta | Playwright (+rustwright opt) | Patchright | puppeteer-core in extension | Chromium fork + Rust CDP |
| Perception | AX tree (+screenshot) | DOM+AX (+vision) | vision + DOM | vision coords | DOM (+vision) | n/a (agent's tools) |
| Who drives | your code / your agent | built-in loop | loop, DAG, or code | both | built-in multi-agent | external agent via MCP |
| LLM lib | Vercel AI SDK | own clients | LiteLLM | BAML | Vercel AI SDK | Vercel AI SDK |
| Replay w/o LLM | cloud cache only; `act(Action)` | history rerun; Cloud scripts still run an agent | per-block code cache, progressive | no (planned) | no | no (video replay) |
| Vault / 2FA | `%var%` (leaks to cache) | placeholders + TOTP; Cloud 1Password | Bitwarden / 1Password / Azure + TOTP / push / pull | data option | local keys | imported Chrome logins |
| Budget cap | none | `max_steps` | `max_steps` | none found | none found | none found |

**Implications for Automan.**
- **Rust-core claims are not unique.** Browser Use (beta), Skyvern (rustwright) and BrowserOS (Rust CDP/MCP crates) all ship Rust runtimes.
- **Openings competitors leave.** The clearest gaps are:
  - a **local, portable plan/code cache with semantic target verification**: Stagehand moved its cache to the cloud, and Browser Use reruns still invoke an agent;
  - **deterministic, effect-based success checks** rather than LLM judges;
  - **domain policy enforced at the network layer**, which three of these tools leak;
  - **first-class budget caps**.
- **License positioning.** Skyvern's AGPL and Stagehand's cloud-only cache are the two positions that matter most against a permissive local-first engine.

## 3. MCP, CLI and SDK primitives, vision agents, testers and recorders

Playwright MCP and Test Agents, Vercel agent-browser, Midscene.js, Notte, AgentQL, LaVague, Agent-E, Shortest, Laminar and Index, Bytebot, Cua, and record/replay tools (Libretto, Lightfeed, Imprint, Autotab, rtrvr, Browserless BrowserQL, AI testing SaaS).

### 3.1 Playwright MCP (microsoft/playwright-mcp) and Playwright Test Agents

**Identity.** An MCP server that exposes Playwright to LLM clients. [Apache-2.0](https://github.com/microsoft/playwright-mcp), written in TypeScript. Latest release is [v0.0.83 (2026-09-28)](https://github.com/microsoft/playwright-mcp/releases), still a 0.0.x series. The repo has about 38.0k stars and its last push was on 2026-10-08. `@playwright/mcp` gets about 6.56M npm downloads a week. Its sibling is [Playwright CLI](https://github.com/microsoft/playwright-cli) (13.9k stars, v0.1.22, `@playwright/cli` about 1.08M downloads a week). Playwright core is at [v1.64.0 (2026-10-07)](https://github.com/microsoft/playwright/releases).

**Architecture.** The external agent drives the loop and the server only provides primitives. Perception is the accessibility snapshot by default, and the README pitches this as needing "No vision models" ([README](https://github.com/microsoft/playwright-mcp)). Coordinate tools are opt-in via `--caps=vision`, and `--snapshot-boxes` adds bounding boxes. Browser control goes through Playwright. It can launch Chromium, Firefox or WebKit, attach over CDP (`--cdp-endpoint`), or attach to the user's logged-in Chrome or Edge through the "Playwright Extension" (`--extension`).

**Model layer.** None. It is model-agnostic because the MCP client brings the model.

**Interfaces.**
- MCP over stdio, or HTTP via `--port`. Docker image `mcr.microsoft.com/playwright/mcp`.
- Tool groups are gated by `--caps`: core, tabs, network, storage, devtools, vision, pdf, testing and config ([tools list](https://github.com/microsoft/playwright-mcp#tools)).
- `browser_run_code_unsafe` runs arbitrary Playwright code ("code mode").
- The README now steers coding agents to CLI plus skills, because the CLI is "more token-efficient" ([README](https://github.com/microsoft/playwright-mcp)).

**Caching, replay and repair.** The MCP server has no cache. Its replay mechanism is code emission: `--codegen typescript|python|java|csharp` returns Playwright code for each action. Repair lives in **Playwright Test Agents**, shipped in [v1.56 (2025-10-06)](https://github.com/microsoft/playwright/releases/tag/v1.56.0):
- `npx playwright init-agents --loop=vscode|claude|codex|opencode` generates planner, generator and healer agent definitions made of instructions plus MCP tools ([docs](https://github.com/microsoft/playwright/blob/main/docs/src/test-agents-js.md)).
- The healer "Replays the failing steps", inspects the UI, patches locators, waits or data, and reruns "until it passes or until guardrails stop the loop". It can mark a test skipped if it believes the functionality is broken.
- Verification is therefore the test's own `expect` assertions.

**Secrets, HITL, cost and injection.**
- `secrets` (dotenv file) replaces matching plain text in responses. The schema itself calls this "a convenience and not a security feature" ([config schema](https://github.com/microsoft/playwright-mcp#configuration-file)).
- `--allowed-origins` and `--blocked-origins` exist, but `--allowed-origins` "does not serve as a security boundary". The file-access guard is also "not a secure boundary".
- The README section on security states that it "is **not** a security boundary".
- There is no budget or token accounting. Output-size controls are `--snapshot-mode none` and `--image-responses omit`.
- `browser_handle_dialog` is the only explicit human-in-the-loop-ish primitive. Human approval otherwise comes from the MCP client.

**Evals.** No WebVoyager, Online-Mind2Web or OSWorld numbers are published by Microsoft.

**Dependencies.** `playwright` and `playwright-core` pinned to 1.64 alpha builds ([npm](https://registry.npmjs.org/@playwright/mcp/latest)), plus `@modelcontextprotocol/sdk` (from dependabot PR #1787).

**Adapters.** Install recipes for Claude Code, Codex, Copilot, Cursor, Gemini CLI, Goose, Junie, Kiro, VS Code and others ([README](https://github.com/microsoft/playwright-mcp)). There are no framework SDK adapters beyond MCP itself.

**User pains.**
- **Context and token bloat:**
  - [#395 snapshot too long, add pagination (14 reactions)](https://github.com/microsoft/playwright-mcp/issues/395)
  - [#889 limit tool output size (13)](https://github.com/microsoft/playwright-mcp/issues/889)
  - [#806 easy to reach token limit (9)](https://github.com/microsoft/playwright-mcp/issues/806)
  - An HN user wrote "I always have this problem with Playwright MCP" on token usage ([HN](https://news.ycombinator.com/item?id=49372088)).
- **Profile locking and launch:**
  - [#891 browser already in use (11)](https://github.com/microsoft/playwright-mcp/issues/891)
  - [#769 (25 comments)](https://github.com/microsoft/playwright-mcp/issues/769)
  - [#13 Windows chrome not found (12)](https://github.com/microsoft/playwright-mcp/issues/13)
  - [#148 "controlled by automated test software" (16)](https://github.com/microsoft/playwright-mcp/issues/148)
- **Extensibility and escape hatches:**
  - [#424 evaluate method (25)](https://github.com/microsoft/playwright-mcp/issues/424)
  - [#442 custom tools/prompts (18)](https://github.com/microsoft/playwright-mcp/issues/442)
  - [#123 codegen during prompt flow (16)](https://github.com/microsoft/playwright-mcp/issues/123)
  - [#994 Electron (13)](https://github.com/microsoft/playwright-mcp/issues/994)
- **Security hardening (recent):**
  - [#1784 unconstrained run_code_unsafe](https://github.com/microsoft/playwright-mcp/issues/1784)
  - [#1789 logs/snapshots risk committing private data](https://github.com/microsoft/playwright-mcp/issues/1789)
  - [#1791 redact session values in URLs](https://github.com/microsoft/playwright-mcp/issues/1791)
- **Test Agents:**
  - [#38610 Python support (52)](https://github.com/microsoft/playwright/issues/38610)
  - [#37739 unusable in Cursor (41)](https://github.com/microsoft/playwright/issues/37739)
  - [#37735 reuse existing POMs (17)](https://github.com/microsoft/playwright/issues/37735)
  - [#40560 .NET (14)](https://github.com/microsoft/playwright/issues/40560)
  - [#39235 MCP test server hangs with parallel healers](https://github.com/microsoft/playwright/issues/39235)

**Praised.**
- "Playwright MCP seems worth the extra context" ([kylemh, HN](https://news.ycombinator.com/item?id=45706355)).
- "MCPs are deterministic, SKILLS.md isn't." ([AftHurrahWinch, HN](https://news.ycombinator.com/item?id=45644100)).
- "a Playwright MCP available and works right out of the box" ([undefuser, HN](https://news.ycombinator.com/item?id=49811652)).

---

### 3.2 Vercel agent-browser (vercel-labs/agent-browser)

**Identity.** A browser CLI for AI agents. [Apache-2.0](https://github.com/vercel-labs/agent-browser/blob/main/LICENSE), written in Rust. Latest release is [v0.39.0 (2026-10-09)](https://github.com/vercel-labs/agent-browser/releases). The repo has 43.8k stars, about 1.66M npm downloads a week and 849 open issues and PRs. Its last push was on 2026-10-10, so it is very active.

**Architecture.**
- The external agent drives (CLI or MCP). An optional `chat` subcommand adds its own loop.
- A Rust CLI talks to a "Pure Rust daemon using direct CDP, no Node.js required", which persists between commands and idles out after 1 h ([Architecture](https://github.com/vercel-labs/agent-browser#architecture)).
- Perception is an accessibility snapshot with refs (`@e1`), plus annotated screenshots, `diff snapshot` and `diff screenshot`, and semantic `find role ...`.
- Engines are `chrome`, `lightpanda` and `obscura` (experimental). There is also an iOS Safari provider via Appium.
- Cloud providers: Browserbase, Browserless, Browser Use, Kernel and AgentCore ([README providers](https://github.com/vercel-labs/agent-browser#browserbase)).

**Model layer.** BYO agent. `chat` and the dashboard use Vercel AI Gateway (default `anthropic/claude-sonnet-4.6`, overridable with `--model`) streamed with the AI SDK UI Message Stream protocol ([AI Chat](https://github.com/vercel-labs/agent-browser#ai-chat)).

**Interfaces.**
- CLI with `--json`, `batch` (args or stdin JSON) and skills.
- `agent-browser mcp` (stdio, MCP 2025-11-25) with tool profiles `core`, `network`, `react`, `all` and readOnly/openWorld annotations.
- An experimental WebMCP client.
- Observability dashboard on :4848.
- `@agent-browser/sandbox` (Vercel Sandbox microVMs) and the `@agent-browser/eve` extension.
- An out-of-process plugin protocol `agent-browser.plugin.v1` ([README](https://github.com/vercel-labs/agent-browser)).

**Caching, replay and repair.** There is no LLM plan cache or self-healing. It is determinism by design: refs, `diff`, `screenshot --if-changed`, and session state save and restore. Verification is left to the agent.

**Secrets, HITL, cost and injection.** All of these are opt-in ([Security](https://github.com/vercel-labs/agent-browser#security)):
- An encrypted auth vault: "The LLM never sees passwords". `auth login` fails if the credential field is swapped before entry.
- `--content-boundaries` markers that separate page output from tool output.
- `--allowed-domains`, which also blocks subresources and WebRTC and fails closed for workers.
- `--action-policy` files and `--confirm-actions eval,download` (HITL).
- `--max-output`.
- State files can be encrypted with AES-GCM via `AGENT_BROWSER_ENCRYPTION_KEY`.
- No token or cost budget beyond output limits (unverified).

**Evals.** None for task success. The repo benchmarks compare its old Node daemon against the Rust daemon (noted 2026-10-07; not re-run).

**Dependencies.** Tokio, tokio-tungstenite, serde, reqwest (rustls), aes-gcm and image ([Cargo.toml](https://github.com/vercel-labs/agent-browser/blob/main/cli/Cargo.toml)).

**Adapters.** Skills for Claude Code, Codex, Cursor, Gemini CLI, Copilot, Goose, OpenCode and Windsurf, plus eve and Vercel Sandbox. No LangChain or OpenAI Agents packages (unverified).

**User pains.**
- **Install and browser revision drift:**
  - [#107 headless_shell revision mismatch (15)](https://github.com/vercel-labs/agent-browser/issues/107)
  - [#244 (13, open)](https://github.com/vercel-labs/agent-browser/issues/244)
  - [#324 Volta symlink (8)](https://github.com/vercel-labs/agent-browser/issues/324)
- **Windows:**
  - [#148 shims call /bin/sh (15)](https://github.com/vercel-labs/agent-browser/issues/148)
  - [#262 (10)](https://github.com/vercel-labs/agent-browser/issues/262)
  - [#132 daemon port EACCES](https://github.com/vercel-labs/agent-browser/issues/132)
  - [#382 Defender quarantine](https://github.com/vercel-labs/agent-browser/issues/382)
- **Login and profile reuse:**
  - [#42 --user-data-dir (13)](https://github.com/vercel-labs/agent-browser/issues/42)
  - [#207 --profile broken (9)](https://github.com/vercel-labs/agent-browser/issues/207)
  - [#1319 extension-based connection (6)](https://github.com/vercel-labs/agent-browser/issues/1319)
  - [#1247 focus stealing (7)](https://github.com/vercel-labs/agent-browser/issues/1247)
- **Coverage gaps:**
  - [#120 stealth mode (15, 14 comments)](https://github.com/vercel-labs/agent-browser/issues/120)
  - [#279 cross-origin iframes (12)](https://github.com/vercel-labs/agent-browser/issues/279)
  - [#66 video recording (12)](https://github.com/vercel-labs/agent-browser/issues/66)
- **Action correctness, which matters for Automan:**
  - [#2083 click "✓ Done" but misses under smooth scroll](https://github.com/vercel-labs/agent-browser/issues/2083)
  - [#2081 fill doesn't clear React/contenteditable](https://github.com/vercel-labs/agent-browser/issues/2081)
  - [#2079 browser silently swapped, cookies lost](https://github.com/vercel-labs/agent-browser/issues/2079)
  - [#1371 orphaned Chrome at high CPU](https://github.com/vercel-labs/agent-browser/issues/1371)

**Praised.**
- "a great CLI for agent-based browser automation" ([esafak, HN](https://news.ycombinator.com/item?id=47167062)).
- "playwright-cli and agent-browser are much much better" ([gverrilla, HN](https://news.ycombinator.com/item?id=47511689)).
- A competitor's self-published claim of 16% lower tokens than agent-browser ([Kuri, HN](https://news.ycombinator.com/item?id=47857965)) shows agent-browser is the reference to beat.
- The Stagehand founder notes "agent-browser is CLI-only" ([HN](https://news.ycombinator.com/item?id=49784241)). He is a competitor.

---

### 3.3 Midscene.js (web-infra-dev/midscene, ByteDance web-infra)

**Identity.** A vision-driven GUI agent plus an E2E test framework. [MIT](https://github.com/web-infra-dev/midscene/blob/main/LICENSE), written in TypeScript. Latest release is [v1.15.0 (2026-10-10)](https://github.com/web-infra-dev/midscene/releases). The repo has 15.1k stars, and `@midscene/web` gets about 61k npm downloads a week. It is very active.

**Architecture.**
- Code calls primitives: `aiAct`, `aiTap`, `aiInput`, `aiAssert`, `aiQuery`, `aiWaitFor` ([README](https://github.com/web-infra-dev/midscene)).
- `aiAct` runs an internal plan loop.
- Perception is "pure vision": a screenshot goes to the VLM for localization, and DOM is optional for extraction ([Model Strategy](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/model-strategy.mdx)).
- Control runs through Playwright, Puppeteer, a desktop-Chrome "bridge mode" extension, Android (ADB), iOS (WDA), HarmonyOS, and macOS/Windows/Linux desktop.

**Model layer.**
- Uses the `openai` SDK 6.3.0 against OpenAI-compatible endpoints, plus `@ui-tars/action-parser` ([npm](https://registry.npmjs.org/@midscene/core)).
- A `MIDSCENE_MODEL_FAMILY` setting selects model adapters: Doubao Seed, Qwen3.x, Gemini 3.x, GPT, GLM, UI-TARS.
- Self-hostable open models: Qwen, UI-TARS ([model config](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/model-common-config.mdx)).
- Only vision models that are stable for GUI work can be used, not arbitrary LLMs.

**Interfaces.**
- JS SDK, YAML runner, "Midscene Test" (YAML cases plus TypeScript Nodes, beta), platform CLIs, Midscene Skills, Chrome extension and Playground.
- **MCP was retired.** The page says "Midscene no longer ships MCP servers"; 1.9.8 (2026-06-22) was the last version with MCP ([mcp.mdx](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/mcp.mdx)).

**Caching, replay and repair** ([caching.mdx](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/caching.mdx)):
- Two caches, keyed by prompt: a plan cache for `aiAct`, and an element-locate cache holding an XPath (web only) that is checked for validity on reuse.
- Strategies are `read-write`, `read-only` (with manual `flushCache()`) and `write-only`. Files go to `./midscene_run/cache/*.cache.yaml`.
- If a cached plan fails mid-run, it falls back to live planning and clears that stale flow. It does not write back the fallback's partial plan, because the page state changed.
- `aiQuery`, `aiBoolean` and `aiAssert` are never cached.
- The docs show one example cutting 51 s to 28 s.
- Verification is `aiAssert`/`aiWaitFor` (visual) plus the HTML replay report.

**Secrets, HITL, cost and injection.**
- Data goes only to the chosen model provider ([data-privacy](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/data-privacy.md)).
- `DEBUG=midscene:ai:profile:stats` prints latency and token usage. Reports also carry usage. LangSmith and Langfuse wrappers are available ([observability doc](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/model-debugging-observability.mdx)).
- No secret vault, budget cap or injection defense is documented (unverified).

**Evals (all self-reported).** From the [README](https://github.com/web-infra-dev/midscene):
- AndroidWorld 93.1% with Gemini-3.5-Flash, "with validator adjustments".
- MobileWorld 78.6%.
- AppControlBench 96.7%: 58 of 60 tasks at $0.59 total model cost.
- No WebVoyager or Online-Mind2Web figures.

**Dependencies.** openai, zod, undici, jsonrepair and js-yaml. `@midscene/web` depends on `puppeteer-core` 24.6 and socket.io, with Playwright and Puppeteer as peers.

**Adapters.** Playwright fixture (`PlaywrightAiFixture`), Puppeteer, Gherkin/BDD and LangSmith/Langfuse. No AI-SDK, LangChain or OpenAI-Agents adapters.

**User pains.**
- **LLM output parsing and schema errors:**
  - [#2831 invalid JSON from LLM (3, open)](https://github.com/web-infra-dev/midscene/issues/2831)
  - [#2049](https://github.com/web-infra-dev/midscene/issues/2049)
  - [#339 400 invalid response_format schema (4, 12 comments)](https://github.com/web-infra-dev/midscene/issues/339)
- **Provider configuration friction (the most-commented issues):**
  - [#221 Azure (25 comments)](https://github.com/web-infra-dev/midscene/issues/221)
  - [#309 Ollama + UI-TARS (24)](https://github.com/web-infra-dev/midscene/issues/309)
  - [#186 non-OpenAI models (13)](https://github.com/web-infra-dev/midscene/issues/186)
- **Wanting deterministic output:**
  - [#2240 export AI steps as Playwright scripts (4, open)](https://github.com/web-infra-dev/midscene/issues/2240)
  - [#426 RAG/app knowledge (5)](https://github.com/web-infra-dev/midscene/issues/426)

**Praised.** "you can set the model and UI-TARS works" ([theturtletalks, HN](https://news.ycombinator.com/item?id=48030492)). HN engagement is otherwise very low; most of the community appears to be on Lark/Discord (unverified).

---

### 3.4 Notte (nottelabs/notte)

**Identity.** A web-agent framework plus a cloud browser platform, written in Python with a Node SDK. **[SSPL-1.0](https://github.com/nottelabs/notte/blob/main/LICENSE)**; GitHub reports NOASSERTION, and the README nonetheless says "Opensource Core". Latest release is [v1.9.7 (2026-10-08)](https://github.com/nottelabs/notte/releases). The repo has 2.0k stars. PyPI `notte` gets about 458 downloads a week and npm `notte-sdk` about 1.2k. It is active.

**Architecture.**
- Hybrid: an agent loop (`notte.Agent`, `max_steps`) plus scripted primitives (`session.execute(type="click", id=...)`, `observe()`, `scrape()`) ([README](https://github.com/nottelabs/notte)).
- Perception is DOM-derived, with an action space of element IDs (exact pipeline unverified). There is a vision-capable reasoning model.
- Browser control uses `patchright` (a stealth Playwright fork) locally, or any CDP URL (Browserbase, Steel, Hyperbrowser, Kernel), or Notte's cloud sessions.

**Model layer.** `notte-llm` uses **LiteLLM** plus `llamux` routing and tiktoken ([pyproject](https://github.com/nottelabs/notte/blob/main/packages/notte-llm/pyproject.toml)). It is therefore multi-provider, with local models via LiteLLM (unverified in docs).

**Interfaces.**
- Python SDK (local `notte` or hosted `notte_sdk`), Node SDK and REST.
- A hosted MCP at `https://api.notte.cc/mcp` (OAuth or bearer) ([mcp-server.mdx](https://github.com/nottelabs/notte/blob/main/docs/src/mcp-server.mdx)).
- `notte-cli`, `notte-skills`, a `claude-managed-agents` folder, and "Functions" (serverless and scheduled).

**Caching, replay and repair.**
- **Agent to Function conversion** turns a successful run into Python code ([workflows.mdx](https://github.com/nottelabs/notte/blob/main/docs/src/features/agents/workflows.mdx)). Its own table admits the result "Breaks if page changes".
- **Agent Fallback** spawns an agent when a scripted step fails ([fallback.mdx](https://github.com/nottelabs/notte/blob/main/docs/src/features/agents/fallback.mdx)).
- Replays are MP4 recordings for debugging, not re-execution ([replay.mdx](https://github.com/nottelabs/notte/blob/main/docs/src/features/agents/replay.mdx)).
- Verification is `response.success` plus user asserts.

**Secrets, HITL, cost and injection.**
- Vaults (credentials, MFA) attach to agents, and Personas provide email, phone and 2FA. The README is cloud-oriented here.
- Core depends on `pyotp`, `cryptography`, `restrictedpython` and `hvac` (HashiCorp Vault) ([notte-core](https://github.com/nottelabs/notte/blob/main/packages/notte-core/pyproject.toml)).
- Live viewer for HITL.
- The cloud adds CAPTCHA solving and proxies.
- Budgets are via `max_steps`. No injection defense is documented (unverified).

**Evals (self-run, own harness).** From [open-operator-evals](https://github.com/nottelabs/open-operator-evals):
- Results: 86.2% agent self-report and 79.0% LLM-judged, against 60.2% for Browser-Use, on a WebVoyager-style set. The judge is GPT-4 using the WebVoyager prompt.
- The repo was last pushed in 2025-04, so this is stale.

**Dependencies.** patchright, litellm, openai, pydantic, trafilatura, markdownify, posthog and opentelemetry.

**Adapters.** MCP-based examples for Agno, CrewAI, Google ADK, LangChain, Mastra and Vercel AI SDK ([docs snippets](https://github.com/nottelabs/notte/tree/main/docs/src/snippets/integrations/mcp)). The `notte-integrations` package covers browser-use, Browserbase, Hyperbrowser, Slack, Discord and LangChain Google GenAI.

**User pains.**
- Very low issue engagement: the top issues have 1 reaction, and the tracker is dominated by bot PRs.
- Representative issue: [#1123 local `Session(cdp_url=...)` always raises](https://github.com/nottelabs/notte/issues/1123).
- The license (SSPL) is the main adoption risk. This is my inference, not a user quote.

**Praised.** No independent praise found. HN posts got 3 to 7 points ([HN 43324924](https://news.ycombinator.com/item?id=43324924)).

---

### 3.5 AgentQL (tinyfish-io/agentql, TinyFish)

**Identity.** A semantic query language over web pages, with Playwright SDKs and a REST API. The [repo is MIT](https://github.com/tinyfish-io/agentql/blob/main/LICENSE) but contains examples and templates; the resolution engine is a hosted service that needs an API key. It is in Python and JS. PyPI `agentql` is at [1.19.0 (2026-05-13)](https://pypi.org/project/agentql/) with about 11.8k downloads a week, and npm `agentql` is at 1.18.1 with about 3k. The repo has 1.5k stars, and [agentql-mcp](https://github.com/tinyfish-io/agentql-mcp) has 181. Activity is maintenance (dependabot and template fixes).

**Architecture.**
- Code-called primitives that wrap a Playwright Page: `query_elements` returns proxies over Locators, `get_by_prompt` returns a Locator, and `query_data` returns data ([AgentQL Page](https://docs.agentql.com/api-references/agentql-page)).
- Perception: the SDK builds an accessibility tree and exposes `get_last_accessibility_tree`. Matching runs server-side; the algorithm is not public.
- Modes are `standard` and `fast`, where fast trades accuracy.
- Browser control is Playwright, local or remote over CDP.

**Model layer.** Proprietary and hosted. No BYO model and no local inference.

**Interfaces.**
- Python and JS SDKs, REST and Debugger extension.
- MCP (`extract-web-data` only).
- Integrations for LangChain, LlamaIndex, Dify, Langflow and Zapier ([integrations](https://docs.agentql.com/integrations), [repo](https://github.com/tinyfish-io/agentql-integrations)).

**Caching, replay and repair.** "Resilience to UI changes" comes from re-resolving queries at runtime, so every run pays a service call. There is no documented compiled plan or cache (noted 2026-10-07, still true in the README).

**Secrets, HITL, cost and injection.** Not documented beyond API keys and the stealth-mode examples. Free tier: 300 calls (noted 2026-10-07).

**Evals.** None published.

**Dependencies.** Playwright, httpx, pydantic and `tf-playwright-stealth` ([PyPI](https://pypi.org/pypi/agentql/json)).

**User pains.**
- **Mis-resolution:** [#121 "resolved as useless span" (1)](https://github.com/tinyfish-io/agentql/issues/121).
- **Vendor dependency and self-hosting:** [#108 self-hosting pipeline](https://github.com/tinyfish-io/agentql/issues/108).
- **API key:** [#122](https://github.com/tinyfish-io/agentql/issues/122).
- **Broken templates:** [#161](https://github.com/tinyfish-io/agentql/issues/161), [#162](https://github.com/tinyfish-io/agentql/issues/162).
- Overall engagement is low.

**Praised.** No independent quote found. The co-founder pitched it at "fragile web automation scripts breaking due to UI changes" ([HN](https://news.ycombinator.com/item?id=41298266)); this is a self-claim.

---

### 3.6 LaVague (lavague-ai/LaVague): dormant

**Identity.** A "Large Action Model" framework. [Apache-2.0](https://github.com/lavague-ai/LaVague/blob/main/LICENSE), written in Python. The repo has 6.4k stars. The last commit was **2025-01-21**, and the last PyPI `lavague` release was 1.1.19 in 2024-08. It gets about 36 PyPI downloads a week. It is not archived but is effectively abandoned.

**Architecture.** A World Model (an LLM producing instructions from page state) feeds an Action Engine that **generates and executes Selenium or Playwright code**. Drivers are Selenium, Playwright and a Chrome extension, with parity gaps ([README](https://github.com/lavague-ai/LaVague)). Perception is HTML text with RAG over the DOM, and vision was added later.

**Model layer.** LlamaIndex 0.10 plus LangChain 0.1 ([lavague-core deps](https://pypi.org/pypi/lavague-core/json)), with OpenAI by default.

**Interfaces.** Python, Gradio and a Chrome extension. LaVague QA turns Gherkin into tests.

**Repair.** None; code is regenerated each step. It has a token counter for cost estimation.

**Security.** [#650 "eval of LLM output … indirect prompt injection to RCE" (open, 2026-06)](https://github.com/lavague-ai/LaVague/issues/650). The fix PR [#651](https://github.com/lavague-ai/LaVague/pull/651) is unmerged. This is the canonical cautionary tale for code-generating agents.

**Pains.**
- [#272 fully local agent (5)](https://github.com/lavague-ai/LaVague/issues/272)
- [#440 separate generation from execution (2)](https://github.com/lavague-ai/LaVague/issues/440)
- [#642 cannot import World Model](https://github.com/lavague-ai/LaVague/issues/642)

**Praised.** "a step towards the missing scripting language for graphical interfaces" ([shadowgovt, HN 378 pts](https://news.ycombinator.com/item?id=39699377)).

---

### 3.7 Agent-E (EmergenceAI/Agent-E): low activity

**Identity.** [MIT](https://github.com/EmergenceAI/Agent-E), written in Python, with 1.25k stars. The last commit was **2025-05-12** and there are no releases. It is effectively unmaintained, and an [ag2 migration PR #132](https://github.com/EmergenceAI/Agent-E/pull/132) is still open.

**Architecture.** An AG2 (AutoGen) multi-agent setup: a planner plus a browser-navigation agent with nested chat. It uses a "skills library" of sensing skills (`get_dom_with_content_type`, which does DOM distillation) and action skills (click, enter text, open url). The README says it deliberately avoids LLM-written code because "it is not going to execute malicious unknown code" ([README](https://github.com/EmergenceAI/Agent-E#architecture)). Control is Playwright.

**Evals.** The [paper](https://arxiv.org/abs/2407.13032) claims it "beats other SOTA text and multi-modal web agents" on WebVoyager by 10 to 30% "in most categories". This is self-reported, on a branch rather than main. A 73.2% overall figure circulates (unverified).

**Pains.**
- Hangs and no response: [#29](https://github.com/EmergenceAI/Agent-E/issues/29), [#76 (9 comments)](https://github.com/EmergenceAI/Agent-E/issues/76), [#93 stuck on Google](https://github.com/EmergenceAI/Agent-E/issues/93).
- Local models: [#28 Ollama](https://github.com/EmergenceAI/Agent-E/issues/28).
- [#85 cumulative token count](https://github.com/EmergenceAI/Agent-E/issues/85).
- [#128 how to use credentials](https://github.com/EmergenceAI/Agent-E/issues/128).
- [#133 no automatic evaluator](https://github.com/EmergenceAI/Agent-E/issues/133).

**Ideas worth keeping.** DOM distillation modes and "change observation" feedback after each action (paper).

---

### 3.8 Shortest (antiwork/shortest): stalled

**Identity.** Natural-language E2E tests. [MIT](https://github.com/antiwork/shortest), written in TypeScript, with 5.7k stars. The last release was **v0.4.9 (2025-04-01)** and the last commit 2026-05-21. **Issues are disabled.** Even so, `@antiwork/shortest` still gets about 52k npm downloads a week.

**Architecture.**
- The test runner calls an LLM loop per `shortest("...")` test.
- Perception and control use **Anthropic computer-use tools** (`computer_20250124`, `bash_20250124`, which are screenshot and coordinate based) over Playwright, plus custom tools for GitHub 2FA login, Mailosaur email and navigate ([src](https://github.com/antiwork/shortest/tree/main/packages/shortest/src/ai/tools)).

**Model layer.** Vercel AI SDK v4 with `@ai-sdk/anthropic` only ([provider.ts](https://github.com/antiwork/shortest/blob/main/packages/shortest/src/ai/provider.ts)). The OpenAI/Gemini PR [#465](https://github.com/antiwork/shortest/pull/465) was closed unmerged.

**Caching.**
- When `caching.enabled` is set, the runner replays the cached steps of the latest passing run, skipping screenshots, as coordinate actions ([runner](https://github.com/antiwork/shortest/blob/main/packages/shortest/src/core/runner/index.ts)).
- On `CacheError` it navigates back to the initial URL and reruns with AI.
- Success is judged by the AI verdict plus `.after()` callbacks (code assertions).
- Note: coordinate replay is fragile by construction.

**Pains (from PRs, since issues are off).** [#460 new Anthropic key format broke runs](https://github.com/antiwork/shortest/pull/460) and the single-provider lock-in.

---

### 3.9 Laminar (lmnr-ai/lmnr) and Index (lmnr-ai/index)

**Laminar.** OSS observability for agents. [Apache-2.0](https://github.com/lmnr-ai/lmnr), with a Rust backend and TypeScript frontend. The repo has 3.4k stars and its latest release is [v0.2.5 (2026-09-13)](https://github.com/lmnr-ai/lmnr/releases). PyPI `lmnr` 0.7.66 gets about 1.2M downloads a week; this is likely inflated by transitive installs (unverified). npm `@lmnr-ai/lmnr` gets about 50k.
- **Browser-specific feature.** Session replay synced to OpenTelemetry spans for Browser Use, Stagehand, Playwright, Puppeteer and Skyvern ([docs](https://laminar.sh/docs/tracing/browser-agent-observability)).
- **Privacy.** `sessionRecordingOptions.maskInputOptions` keeps typed values out of recordings.
- **Other features** ([README](https://github.com/lmnr-ai/lmnr)):
  - "Signals": plain-English behavior detectors such as "agent is stuck in a loop", with Slack alerts.
  - Evals SDK and CLI.
  - MCP and CLI for SQL over traces.
  - "20x trace compression".
- **Cost.** It syncs model costs for self-hosted instances ([PR #2516](https://github.com/lmnr-ai/lmnr/pull/2516)).
- **Adapters (tracing)** ([llms.txt](https://laminar.sh/docs/llms.txt)): Vercel AI SDK, LangChain/LangGraph, Mastra, Pydantic AI, OpenAI Agents SDK, Google ADK, Claude Agent SDK, Microsoft Agent Framework, Temporal, LiteLLM, Claude Code, Codex and Kernel.
- **Pains:**
  - [#2176 self-hosted ClickHouse system logs grow unbounded (8 comments)](https://github.com/lmnr-ai/lmnr/issues/2176)
  - [#2404 OSS build 404s](https://github.com/lmnr-ai/lmnr/issues/2404)
  - [#1515 MCP broken](https://github.com/lmnr-ai/lmnr/issues/1515)
- **Praised.** "I really like the stack these folks have chosen." ([benreesman, HN](https://news.ycombinator.com/item?id=41453058)).

**Index.** The browser agent. **Archived.** [Apache-2.0](https://github.com/lmnr-ai/index), 2.4k stars, last release v0.1.13 (2025-06-05).
- Python with Playwright and vision-capable reasoning LLMs: Gemini 2.5 Pro, Claude 3.7 and o4-mini, via native anthropic, openai, google-genai and groq SDKs ([deps](https://pypi.org/pypi/lmnr-index/json)).
- Pydantic structured output, CLI and a serverless API.
- No published benchmark found in the README.
- HN launch: [98 pts](https://news.ycombinator.com/item?id=43773720). Pains: [#13 file download](https://github.com/lmnr-ai/index/issues/13), [#12 provider init errors](https://github.com/lmnr-ai/index/issues/12).

---

### 3.10 Bytebot (bytebot-ai/bytebot): archived

**Identity.** A self-hosted "AI desktop agent" running in a containerized Ubuntu/XFCE desktop. [Apache-2.0](https://github.com/bytebot-ai/bytebot), written in TypeScript, with 11.1k stars. The **repo is archived**; the last commit was 2025-09-11 and there are no releases.

**Architecture.** A NestJS agent service plus a desktop daemon plus a Next.js UI, with REST APIs and VNC live view ([README](https://github.com/bytebot-ai/bytebot#architecture)). Perception is screenshots with mouse and keyboard, in the computer-use style. Providers are Anthropic, OpenAI and Gemini, with others via LiteLLM.

**HITL.** "Takeover Mode", and password managers inside the VM.

**Evals.** None.

**Pains.**
- Providers: [#101 OpenRouter (15)](https://github.com/bytebot-ai/bytebot/issues/101), [#151 LM Studio (4)](https://github.com/bytebot-ai/bytebot/issues/151).
- Reliability:
  - [#84 all tasks failed (17 comments)](https://github.com/bytebot-ai/bytebot/issues/84)
  - [#166 stuck in loop](https://github.com/bytebot-ai/bytebot/issues/166)
  - [#73 "only a toy, cannot do real job"](https://github.com/bytebot-ai/bytebot/issues/73)
  - [#158 VNC delay](https://github.com/bytebot-ai/bytebot/issues/158)
- [#181 "Is this project dead?"](https://github.com/bytebot-ai/bytebot/issues/181)

---

### 3.11 Cua (trycua/cua)

**Identity.** Computer-use infrastructure. MIT except Cua Spaces, which is FSL-1.1-MIT, and an optional AGPL perception extension ([Licensing](https://github.com/trycua/cua#licensing)). Written in Rust, Python, TypeScript and Swift. The repo has 29.3k stars. Latest release is [cua-sdk-v0.4.1 (2026-10-05)](https://github.com/trycua/cua/releases). It is very active, with 1,151 open issues and PRs.

**Recent direction changes.**
- `cua-agent`, `cua-computer` and `cua-som` are **deprecated**, and `cuabot` is flagged "Do not run it: it has known security issues" ([README](https://github.com/trycua/cua#deprecated-packages)).
- PR [#4938](https://github.com/trycua/cua/pull/4938) removes workflows "ahead of the Cloud Fleets shutdown".

**Architecture.**
- **Cua Driver**: an MCP-over-stdio, CLI and SDK driver for native apps and browsers on macOS, Windows and Linux. It uses OS accessibility plus screenshots and delivers input in the background without stealing focus.
- An optional `cua-perception` extension (OmniParser, which is AGPL, plus PP-OCR) parses screenshots into regions. A pixel click must carry the one-use `capture_id` of the observation it came from ([driver README](https://github.com/trycua/cua/blob/main/libs/cua-driver/README.md)).
- A `run_steps` (formerly `run_actions`) batch tool, and `run_script` on by default with an operator opt-out ([PR #4936](https://github.com/trycua/cua/pull/4936)).
- Lume (macOS/Linux VMs on Apple Silicon), sandboxes via gVisor, Spaces ("Teleport" signed-in apps through an encrypted Keyvault after user approval, and multiplayer human/agent cursors).

**Model layer.** BYO model. CUA-S1 is a small "System 1" forms model with weights on Hugging Face.

**Evals.** cua-bench is a task, evaluator and trajectory-export harness. Recent PRs add "fail-closed benchmark scoring and reproducible evidence receipts" ([#4929](https://github.com/trycua/cua/pull/4929)). No OSWorld figure appears in the current README.

**Adapters.** Connection guides for Claude Code, Codex, Cursor and OpenClaw ([docs](https://cua.ai/docs/cua-driver/guides/connect-your-agent)).

**Pains.**
- [#118 Windows support (10)](https://github.com/trycua/cua/issues/118).
- Lume VM storage and disk: [#22 (5)](https://github.com/trycua/cua/issues/22), [#28](https://github.com/trycua/cua/issues/28), [#925](https://github.com/trycua/cua/issues/925).
- [#1808 ~50% idle CPU](https://github.com/trycua/cua/issues/1808).
- [#1493 Intel macOS install](https://github.com/trycua/cua/issues/1493).
- [#4798 Gemini rejects tool schemas](https://github.com/trycua/cua/issues/4798).
- At launch, a user reported the agent "got ahead of itself and starting typing things in the wrong place" ([HN](https://news.ycombinator.com/item?id=43774258)).

---

### 3.12 Record/replay and "learn once, replay" tools (brief)

**Libretto ([cashew-labs/libretto](https://github.com/cashew-labs/libretto), formerly saffron-health).**
- MIT, TypeScript, 905 stars, [v0.6.45 (2026-08-14)](https://github.com/cashew-labs/libretto/releases). npm `libretto` gets about 19k downloads a week.
- "Development-time AI": a coding-agent skill plus CLI (`open`, `snapshot` with screenshot and compact accessibility tree, `exec` for Playwright TypeScript, `run`). It records user actions and captures network traffic to convert UI scripts into direct API calls ([README](https://github.com/cashew-labs/libretto)).
- Output is plain Playwright TypeScript that the user owns.
- "Libretto PR agents" open GitHub PRs to fix failing scripts ([Show HN](https://news.ycombinator.com/item?id=48939710)).
- Launch: [134 points](https://news.ycombinator.com/item?id=47780971). Praise: "it's always that 80%-99% automated slice where the frontier is" ([etwigg](https://news.ycombinator.com/item?id=47783042)).
- Critique: "why cant I just create my own skill with the prompt" ([pryelluw](https://news.ycombinator.com/item?id=48941751)).

**Lightfeed browser-agent ([lightfeed/browser-agent](https://github.com/lightfeed/browser-agent)).**
- MIT, 17 stars, v0.2.3 (2026-04-24), forked from HyperAgent.
- The AI navigation run is saved as a JSON plan with `xpath` plus `cssPath` per step. Replay uses "zero LLM tokens", and `aiFallback` re-plans only the drifted step.
- Extraction always re-runs the AI. LangChain chat models are used as the LLM interface ([README](https://github.com/lightfeed/browser-agent)).

**Imprint ([ashaychangwani/imprint](https://github.com/ashaychangwani/imprint)).**
- MIT, 24 stars, v0.6.6 (2026-07-27).
- `imprint teach` records network plus DOM and compiles `workflow.json` (API replay) and `playbook.yaml` (DOM fallback) into an MCP server.
- A backend ladder escalates fetch, then stealth-fetch, then CDP, then DOM. It has an independent live verifier and redacts credentials and PII before anything reaches the LLM.
- The compile step uses Claude Code, Codex or the Anthropic API.
- The README's "Zero tokens at runtime" and 200 ms to 9 s figures are self-claims.

**Autotab.** Teach-by-demonstration browser that turns tasks into an API ([Show HN 2024, 159 points](https://news.ycombinator.com/item?id=42197741)). No 2026 activity found (unverified). A competitor blog says its scripts "break when HTML structure changes" ([Skyvern blog](https://www.skyvern.com/blog/browser-use-vs-autotab-which-ai-automation-tool-is-better/), a biased source).

**rtrvr.ai.** A closed-source, DOM-only (no screenshots) planner with sub-agents, available as a Chrome extension, cloud, API, MCP and CLI ([docs](https://rtrvr.ai/docs/introduction)). It self-reports 81.39% on Halluminate Web Bench (323 tasks, June 2025) ([blog](https://www.rtrvr.ai/blog/web-bench-results)).

**Browserless BrowserQL / MCP.** SSPL or commercial license ([LICENSE](https://raw.githubusercontent.com/browserless/browserless/main/LICENSE)). The hosted MCP's `browserless_agent` tool keeps state across calls ([docs](https://docs.browserless.io/mcp/overview)). BrowserQL is a stealth scraping DSL. No learn-and-replay mechanism was found.

**AI testing SaaS: Momentic, Reflect, QA Wolf.**
- **Momentic** keeps a per-step "step cache" of locator metadata. When the element is missing it re-resolves from the plain-language description and re-caches only "if the rest of the test passes". A post-run triage agent then opens a PR ([auto-heal](https://momentic.ai/docs/reliability/auto-heal), [auto-maintenance](https://momentic.ai/docs/reliability/auto-maintenance)). This is the closest commercial analogue to a "verify before writing cache" rule.
- **Reflect** (SmartBear) calls OpenAI on every run for prompt steps ([docs](https://reflect.run/docs/recording-tests/testing-with-ai/)).
- **QA Wolf** generates Playwright code with human review (third-party sources only).

---

### 3.13 Cross-cutting observations

- **Two camps.** (a) Thin, deterministic, agent-driven primitives: Playwright MCP/CLI, agent-browser, Cua Driver, Libretto. (b) Embedded agent loops: Midscene `aiAct`, Notte, Shortest, Index, Bytebot, LaVague, Agent-E. The big download numbers (6.6M and 1.7M a week) are all in camp (a).
- **CLI plus skills over MCP for coding agents.** Microsoft now says this ([README](https://github.com/microsoft/playwright-mcp)), and Midscene went further by retiring MCP entirely ([mcp.mdx](https://github.com/web-infra-dev/midscene/blob/main/apps/site/docs/en/mcp.mdx)). Token bloat is the most-voted pain on Playwright MCP.
- **Replay designs seen:**
  - Prompt-keyed plan cache plus XPath with fallback and no write-back of partial plans (Midscene).
  - Last-passing-run coordinate replay (Shortest; fragile).
  - Agent run to generated code with agent fallback (Notte).
  - JSON plan with xpath plus css and per-step AI fallback (Lightfeed).
  - Network to API compilation with a verifier (Imprint, Libretto).
  - Verify-then-cache (Momentic).
  - Test-assertion-gated healing (Playwright healer).
  - None of these OSS caches verify business correctness beyond the user's assertions.
- **Security posture is mostly "not a boundary".** Playwright MCP disclaims its own secrets and origin filters. agent-browser has the richest opt-in controls (vault, domain allowlist incl. WebRTC and workers, content boundaries, action policy, confirmations). LaVague shows the RCE risk of `eval`ing LLM output (#650). Cua binds pixel clicks to a one-use capture id.
- **Abandonment rate is high among embedded-loop OSS agents.** LaVague and Agent-E are dormant, and Index and Bytebot are archived. Cua deprecated its agent SDK in favor of driver plus BYO coding agent.

## 4. AI extraction and crawl

Firecrawl (JSON mode, `/extract`, FIRE-1, `/agent`, `/interact`), Crawl4AI, ScrapeGraphAI, Spider, Crawlee AI integrations and Apify MCP, plus comparables.

### Cross-cutting takeaways

1. **Four ways AI is placed in the loop:**
   - **(a) LLM-per-page extraction** from Markdown or HTML: Firecrawl JSON mode, Crawl4AI `LLMExtractionStrategy`, ScrapeGraphAI `SmartScraperGraph`, Crawlee `PydanticAiDirectExtractor`, llm-scraper, Parsera.
   - **(b) The LLM generates selectors or code once, then they replay deterministically:** Crawl4AI `generate_schema`/`generate_pattern`, Crawlee `PydanticAiSelectorExtractor`, ScrapeGraphAI `CodeGeneratorGraph`/`ScriptCreatorGraph`, llm-scraper `generate`.
   - **(c) Navigation agents:** Firecrawl `/agent` and `/interact`, Spider `spider_agent` / `RemoteMultimodalEngine`, Spider Cloud `/ai/browser`, Crawlee + Stagehand.
   - **(d) Repair:** Spider `SelfHealingConfig`, Kadoa's "self-healing pipelines", and Scrapling's non-LLM similarity relocation.
2. **The strongest open-source pattern for cost** is (b) plus a cache keyed by route or layout. When a page matches no cached selector set, the system regenerates. Crawlee Python 1.8+ ships exactly this ([guide](https://github.com/apify/crawlee-python/blob/master/docs/guides/pydantic_ai_crawler.mdx)). Users on HN asked for this shape as far back as 2024 ([HN 40107935](https://news.ycombinator.com/item?id=40107935), [HN 40292021](https://news.ycombinator.com/item?id=40292021)).
3. **The open-source vs cloud gap is sharpest at Firecrawl.** Self-hosted Firecrawl lacks Agent, Browser, Interact, screenshots and actions ([self-host guide](https://docs.firecrawl.dev/contributing/self-host.md)). For Spider, prompt extraction exists **only** on the cloud `/ai/*` routes ([spider.cloud llms.txt](https://spider.cloud/llms.txt)).
4. **Budgets and verification are uneven:**
   - **Firecrawl Agent** has `maxCredits`, a `partial` result with `partialSchemaValid`, and paid-call approvals.
   - **Crawlee** passes Pydantic AI `UsageLimits` through, plus retries on validation failure.
   - **Crawl4AI** tracks usage (`show_usage`, `TokenUsage`) but has no cap that we found.
   - **ScrapeGraphAI** and **Spider** expose no budget primitive that we found in the docs read. One exception: `spider_agent` offers a `CostTier`/`max_cost_tier` model-routing policy and `with_max_rounds`.
5. **Firecrawl is the only tool in this group that documents prompt-injection mitigation for extraction:** a classifier gate, `checkPromptInjection` ([JSON mode docs](https://docs.firecrawl.dev/features/llm-extract.md)).
6. **Model layers:**

   | Tool | Model layer |
   |---|---|
   | Firecrawl | Vercel AI SDK internally (`ai` 6.0.86 plus `@ai-sdk/*` and `ollama-ai-provider-v2`, [apps/api/package.json](https://github.com/firecrawl/firecrawl/blob/main/apps/api/package.json)) |
   | Crawl4AI | A LiteLLM fork, `unclecode-litellm==1.81.13`, adopted after a LiteLLM PyPI supply-chain compromise ([README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md), [pyproject](https://github.com/unclecode/crawl4ai/blob/v0.9.4/pyproject.toml)) |
   | ScrapeGraphAI | LangChain 1.x |
   | Crawlee Python | Pydantic AI |
   | Crawlee JS | Stagehand v3 |
   | Spider | Its own OpenAI-compatible and Gemini clients |

---

### 4.1 Firecrawl

**What it is**
- Web-data API for agents: scrape, crawl, map, search, JSON extraction, `/agent`, `/interact`, monitors and change tracking.
- License: core is **AGPL-3.0** ([LICENSE](https://github.com/firecrawl/firecrawl/blob/main/LICENSE)). SDKs and the MCP server are MIT.
- Language: TypeScript.
- Stars: 190,156 (read 2026-10-10). Last push 2026-10-10; 539 open issues ([repo](https://github.com/firecrawl/firecrawl)).
- GitHub latest Release: v2.11.0 (2026-06-19). Releases lag `main`.
- SDKs: `firecrawl-py` 4.50.0 (2026-10-08, about 1.26M downloads/week, [PyPI](https://pypi.org/project/firecrawl-py/)) and `@mendable/firecrawl-js` 4.46.0 (2026-10-08, about 1.21M downloads/week).
- `firecrawl-mcp` 3.29.1 (2026-10-09, about 117k downloads/week).

**Architecture: where AI sits**
- **JSON mode on `/scrape`** is LLM extraction from page content. It costs +4 credits per page ([llm-extract](https://docs.firecrawl.dev/features/llm-extract.md), [billing](https://docs.firecrawl.dev/billing.md)).
- **`/extract`** takes multiple URLs and wildcards. Its own docs now call it superseded by `/agent` ([extract](https://docs.firecrawl.dev/features/extract.md)).
- **FIRE-1** is still documented as a "preview" agent model on `/extract` that "controls browser actions" ([extract §FIRE-1](https://docs.firecrawl.dev/features/extract.md)).
- **`/agent`** needs no URLs. It searches, navigates and extracts on the `spark-2` model with `effort` low/medium/high. Spark 1 is deprecated and routes to spark-2. It also has orchestrator/subagent traces, artifacts and snapshots, plus live-view URLs ([agent](https://docs.firecrawl.dev/features/agent.md)).
- **`/interact`** runs after a scrape and takes either a natural-language `prompt` or Playwright `code`. Sessions are stateful and support persistent profiles ([interact](https://docs.firecrawl.dev/features/interact.md)).
- **Legacy `actions`** on scrape are deterministic and are now discouraged in favor of Interact ([scrape](https://docs.firecrawl.dev/features/scrape.md)).
- **Change tracking** is deterministic diffing of Markdown (`git-diff`). A `json` mode runs a schema extraction on both the current and previous versions and compares field by field. Snapshots persist without expiry ([change tracking](https://docs.firecrawl.dev/features/change-tracking.md)).

**Perception and browser control**
- Perception is HTML→Markdown/JSON. Screenshots come through the Fire-engine/cloud stack.
- Browser control:
  - Self-host: bundled Playwright service plus fetch.
  - Cloud: Fire-engine (CDP/TLS) ([SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md)).

**Model and provider layer**
- Internally the Vercel AI SDK with OpenAI, Anthropic, Google, Vertex, Groq, xAI, Fireworks, DeepInfra and Ollama providers ([package.json](https://github.com/firecrawl/firecrawl/blob/main/apps/api/package.json)).
- Self-host ships "no model provider". You connect OpenAI, an OpenAI-compatible endpoint or Ollama yourself ([SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md)).
- Cloud `/agent` uses Firecrawl's own Spark models.

**Interfaces**
- REST v2, plus SDKs for Python, Node, Go, Java, Ruby, Rust, Elixir, .NET and PHP (`apps/`).
- CLI.
- Hosted MCP at `mcp.firecrawl.dev` supports keyless use, OAuth or an API key ([MCP](https://docs.firecrawl.dev/mcp-server.md)). It exposes about 27 tools, including `firecrawl_agent`, `firecrawl_interact`, `firecrawl_monitor_*`, `firecrawl_parse` and `firecrawl_credit_usage` ([MCP README](https://github.com/firecrawl/firecrawl-mcp-server)).
- Native ChatGPT/Codex and Claude connectors.
- Webhooks for agent lifecycle events.

**Caching, reuse and self-healing**
- `maxAge` cache for scrapes ("Faster Scraping") ([docs index](https://docs.firecrawl.dev/llms.txt)).
- We found no generated selector that is replayed without an LLM, and no self-healing selector feature.
- Output checks:
  - JSON-schema extraction; failures surface as 400/422.
  - `/agent` returns `partialSchemaValid` on a stopped run.

**Cost, secrets, human-in-the-loop and injection**
- Credits are 15 tokens each for extract.
- `/agent` has a `maxCredits` default of 2,500. A stop sets `stopReason: credit_limit_reached` and returns a best-effort `partial`. You can resume via `threadId`. Failed runs are refunded ([agent](https://docs.firecrawl.dev/features/agent.md)).
- Human-in-the-loop for third-party paid data providers ("Alexandria"): `requireApproval` → `pendingApproval` → `approve`/`decline`. There is no auto-accept of terms.
- Live view of the agent's browser.
- **Prompt injection:** the `checkPromptInjection` classifier runs before any LLM format. It blocks with 403 `SCRAPE_PROMPT_INJECTION_DETECTED` and costs +4 credits ([llm-extract](https://docs.firecrawl.dev/features/llm-extract.md)).
- Enterprise controls: key restrictions, IP allow-lists, threat protection and SIEM ([docs index](https://docs.firecrawl.dev/llms.txt)).

**Evaluation**
- Spark 2 is claimed to have "accuracy comparable" to Spark 1 (self-reported, no numbers).
- No independent benchmark was read.

**Key dependencies**
- Self-host: NuQ PostgreSQL queue, Redis, RabbitMQ, Playwright service, optional FoundationDB ([SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md)).

**Agent-framework adapters**
- Docs guides for OpenAI, Google ADK, Vercel AI SDK, LangChain, LangGraph, LlamaIndex, Mastra and ElevenAgents.
- Quickstarts for Claude Code, Codex CLI and Gemini CLI ([v2 docs index](https://docs.firecrawl.dev/_llms/en/v2.md)).

**User pains**

The most-reacted issues are self-host gaps, not AI quality:
- [#1129 proxy on self-hosted](https://github.com/firecrawl/firecrawl/issues/1129) (18 reactions).
- [#1390 "All scraping engines failed"](https://github.com/firecrawl/firecrawl/issues/1390) (14).
- [#2350 waterfall loop when blocked by anti-bot](https://github.com/firecrawl/firecrawl/issues/2350) (10 reactions, 25 comments).
- [#1169 asks for an identifiable UA so sites can block it](https://github.com/firecrawl/firecrawl/issues/1169) (7, open).

AI-specific (46 issues match "extract" in the title):
- Local models: [#1294 `/extract` failing with Ollama llama3.2:1b](https://github.com/firecrawl/firecrawl/issues/1294) (3) and [#1467 Ollama extract expects a Google key](https://github.com/firecrawl/firecrawl/issues/1467) (5).
- Providers: [#1081 Azure OpenAI](https://github.com/firecrawl/firecrawl/issues/1081) (5) and [#3169 OpenAI-compatible proxies break LLM extract](https://github.com/firecrawl/firecrawl/issues/3169).
- Schema handling: [#1836 JS SDK extract returns empty data with Zod schema](https://github.com/firecrawl/firecrawl/issues/1836) (3).

Community:
- A self-host-optimized fork exists ([Firecrawl-Simple, HN 42056056](https://news.ycombinator.com/item?id=42056056), 35 points). One commenter: "AGPL means you can just fork and remove everything you don't like" ([hardwaresofton](https://news.ycombinator.com/item?id=42056970)).
- Rust alternatives are appearing ([Draco, HN 49148163](https://news.ycombinator.com/item?id=49148163)).

**Praised**
- API shapes: one prompt and an optional schema on `/agent`; async jobs with webhooks.
- Firecrawl positions these itself; independent praise was thin in the sources read.

---

### 4.2 Crawl4AI

**What it is**
- Python crawler that produces LLM-ready Markdown and structured extraction. Optional Docker server with MCP.
- License: Apache-2.0 per the API, but the LICENSE adds a **mandatory attribution clause** (prior local note; [README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md)).
- Version: **0.9.4** (2026-09-23).
- Stars: 85,143 (read 2026-10-10); 237 open issues. About 288k PyPI downloads/week.
- A "Crawl4AI Cloud API" is in closed beta ([README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md)).

**Architecture: where AI sits** (all optional)
- **`LLMExtractionStrategy`** ([docs](https://github.com/unclecode/crawl4ai/blob/v0.9.4/docs/md_v2/extraction/llm-strategies.md)):
  - Input is `markdown`, `fit_markdown` or `html`.
  - Modes are `schema` (Pydantic JSON schema) or `block`.
  - Chunking via `chunk_token_threshold`, `overlap_rate` and `word_token_rate`, with chunks processed in parallel.
- **`JsonCssExtractionStrategy.generate_schema` / `JsonXPathExtractionStrategy.generate_schema`** ([docs](https://github.com/unclecode/crawl4ai/blob/v0.9.4/docs/md_v2/extraction/no-llm-strategies.md)):
  - The LLM is used once to produce CSS/XPath; the result is then reused with no LLM.
  - `validate=True` (default) runs the schema against the HTML and refines selectors if it extracts nothing.
- **`RegexExtractionStrategy.generate_pattern`** is the same "LLM once, cache pattern" idea.
- **Other LLM classes:** `LLMContentFilter` and `LLMTableExtraction` ([source](https://github.com/unclecode/crawl4ai/blob/v0.9.4/crawl4ai/content_filter_strategy.py)).
- **Adaptive crawling** ([docs](https://github.com/unclecode/crawl4ai/blob/v0.9.4/docs/md_v2/core/adaptive-crawling.md)):
  - Stops when coverage, consistency and saturation say "enough".
  - The `statistical` strategy needs no model.
  - The `embedding` strategy uses embeddings plus an LLM for query expansion.
- Perception: HTML→Markdown (with pruning or BM25 filters), HTML, and screenshots.
- Browser control: Playwright/Patchright, managed browser or CDP; also HTTP.

**Model and provider layer**
- Any LiteLLM provider string (`openai/...`, `ollama/...`) via `LLMConfig` ([docs](https://github.com/unclecode/crawl4ai/blob/v0.9.4/docs/md_v2/extraction/llm-strategies.md)).
- Pinned to the `unclecode-litellm` fork since 0.8.6, after the LiteLLM supply-chain incident ([README changelog](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md)).

**Interfaces**
- Python library and CLI.
- Docker REST server.
- MCP over SSE `/mcp/sse` and WebSocket `/mcp/ws`. Tools: `md`, `html`, `screenshot`, `pdf`, `execute_js`, `crawl`, `ask` ([Docker README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/deploy/docker/README.md)).
- C4A-Script interaction DSL.

**Caching and verification**
- HTTP/result cache (`CacheMode`).
- Generated schemas are saved and reused by the caller, so the cache is the user's job, as in the docs example.
- `generate_schema` validates and refines its output.
- `LLMExtractionStrategy` only "tries to parse"; the docs tell users to post-validate with Pydantic themselves ([docs](https://github.com/unclecode/crawl4ai/blob/v0.9.4/docs/md_v2/extraction/llm-strategies.md)).

**Cost, secrets and security**
- Usage tracking: `usages`, `total_usage`, `show_usage()`, and a `TokenUsage` accumulator for schema generation.
- **We found no spend cap.**
- Rate-limit backoff is configurable in `LLMConfig`.
- **Prompt-injection mitigation:** none documented in the pages read.
- Docker server security:
  - A string of advisories was fixed in 0.8.7 and later: RCE, SSRF, auth bypass, and a hard-coded JWT secret ([README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md)).
  - An egress pinning proxy guards against DNS rebinding in server mode only ([egress_policy.py](https://github.com/unclecode/crawl4ai/blob/v0.9.4/crawl4ai/egress_policy.py)).
- Human-in-the-loop: none found.

**Evaluation**
- No extraction-accuracy numbers were found in the docs read.

**Key dependencies**
- playwright, patchright, playwright-stealth, unclecode-litellm, lxml, bs4, rank-bm25, nltk, pydantic ([pyproject](https://github.com/unclecode/crawl4ai/blob/v0.9.4/pyproject.toml)).

**Agent-framework adapters**
- MCP. Integrations with others (e.g. LangChain/LlamaIndex) are community-built or (unverified).

**User pains**
- Top issue: [#107 "Please respect robots.txt"](https://github.com/unclecode/crawl4ai/issues/107) (31 reactions).
- [#1924 LLMTableExtraction unusable with the Docker job-queue API](https://github.com/unclecode/crawl4ai/issues/1924) (11).
- [#1256 memory leak on repeated /md via Docker](https://github.com/unclecode/crawl4ai/issues/1256) (10, open).
- LLM-specific (96 issues mention `LLMExtractionStrategy`):
  - [#1202 llm_strategy returns empty extracted_content](https://github.com/unclecode/crawl4ai/issues/1202) (13 comments).
  - [#979 "'str' has no attribute 'choices'"](https://github.com/unclecode/crawl4ai/issues/979) (22 comments).
  - [#989 rate limit → no usage attribute](https://github.com/unclecode/crawl4ai/issues/989).
  - [#649 Gemini/Ollama provider asks for OpenAI auth](https://github.com/unclecode/crawl4ai/issues/649).
  - [#55 can't use local LLM](https://github.com/unclecode/crawl4ai/issues/55).
- Schema generation:
  - [#1663 Claude returns Markdown-wrapped JSON despite JSON mode](https://github.com/unclecode/crawl4ai/issues/1663).
  - [#1672 make generate_schema robust across multi-page samples](https://github.com/unclecode/crawl4ai/issues/1672).
- Chunking: [#1909 NlpSentenceChunking destroys sentence order](https://github.com/unclecode/crawl4ai/issues/1909).

**Praised**
- "LLM once → reusable CSS/XPath schema" is the API shape the docs push hardest.
- HN threads were low-signal ([HN 43827993](https://news.ycombinator.com/item?id=43827993)).

---

### 4.3 ScrapeGraphAI

**What it is**
- Python library of LLM "graphs": fetch → parse → generate-answer nodes.
- License: MIT. Version **2.3.1** (2026-10-06).
- Stars: 31,671 (read 2026-10-10). But `scrapegraphai` gets only about 5.7k PyPI downloads/week, against about 49k/week for the cloud SDK `scrapegraph-py` ([pypistats](https://pypistats.org/api/packages/scrapegraphai/recent)).
- The README leads with the commercial cloud ([README](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/v2.3.1/README.md)).

**Architecture**
- AI is central:
  - `SmartScraperGraph`, with multi/lite/concat variants.
  - `SearchGraph`, `DepthSearchGraph`, `OmniScraperGraph` (images), `ScreenshotScraperGraph`.
  - Document, CSV, JSON and XML scrapers.
  - `SpeechGraph`.
  - **`CodeGeneratorGraph` and `ScriptCreatorGraph`**, which generate reusable scraping code. CodeGenerator chains Fetch → Parse → GenerateAnswer (validation) → PromptRefiner → HtmlAnalyzer → GenerateCode ([graphs dir](https://github.com/ScrapeGraphAI/Scrapegraph-ai/tree/v2.3.1/scrapegraphai/graphs), [code_generator_graph.py](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/v2.3.1/scrapegraphai/graphs/code_generator_graph.py)).
- Perception: HTML→text (html2text, minify-html), chunked by `semchunk` against `model_tokens`; screenshots for vision graphs.
- Browser: Playwright / undetected-playwright, plus `free-proxy` ([pyproject](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/v2.3.1/pyproject.toml)).

**Model and provider layer**
- LangChain 1.x: OpenAI, Mistral, AWS, Ollama and community providers. Config is `{"llm": {"model": "ollama/llama3.2", "model_tokens": 8192}}`.

**Interfaces**
- Python library only in OSS.
- The cloud (v2) offers REST Scrape, Extract (5 credits), Search, Crawl, Monitor, Schema generation, the `just-scrape` CLI with an agent skill, an MCP server, and "Agent Payments" (crypto wallet) ([cloud docs index](https://docs.scrapegraphai.com/llms.txt), [extract](https://docs.scrapegraphai.com/services/extract.md)).

**Verification**
- `jsonschema` and Pydantic are dependencies, and the cloud accepts a JSON schema.
- No caching or selector reuse is found in the OSS path, except through generated code.

**Cost, secrets and privacy**
- No budget primitive found.
- **Telemetry is ON by default**: it posts to `sgai-oss-tracing.onrender.com` and opts out via `SCRAPEGRAPHAI_TELEMETRY_ENABLED=false` ([telemetry.py](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/v2.3.1/scrapegraphai/telemetry/telemetry.py)).
- No prompt-injection mitigation found.

**Evaluation**
- None found.

**Key dependencies**
- langchain stack, playwright, semchunk, tiktoken, ddgs, and the `scrapegraph-py` cloud SDK.

**Agent-framework adapters (cloud)**
- Vercel AI SDK, LangChain, LangGraph, LlamaIndex, CrewAI, Agno, LiteLLM MCP gateway, n8n/Make/Zapier ([cloud docs index](https://docs.scrapegraphai.com/llms.txt)).

**User pains**
- Dominated by LangChain churn:
  - [#926 "PromptTemplate missing variables"](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/926) (19 reactions).
  - [#1017 LangChain import break](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/1017) (8).
  - [#1078 ChatOllama removed breaks all graphs](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/1078).
- Cost: [#1036 OpenAI Batch API support](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/1036) (7).
- Empty output: [#893 empty JSON output](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/893) (13 comments).
- Event loop: [#179 asyncio.run in a running loop](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/179) (6).

**Praised / debated**
- The launch thread had 194 points and 63 comments ([HN 40290596](https://news.ycombinator.com/item?id=40290596)).
- Key critique: "using LLMs for web data extraction fundamentally does not work unless you generate code" ([KhoomeiK](https://news.ycombinator.com/item?id=40293091)).
- Request: an LLM builder that will "generate optimised scraping rules for any page" ([nodoodles](https://news.ycombinator.com/item?id=40292021)).

---

### 4.4 Spider (spider-rs) and Spider Cloud

**What it is**
- Rust crawler workspace: `spider`, `spider_agent`, `spider_agent_html`, `spider_agent_types`, `spider_mcp`, `spider_cli`, `spider_worker`, `spider_utils`.
- License: MIT. Crates `spider`, `spider_agent` and `spider_mcp` are at **2.53.9** (2026-09-05). `spider` has about 50k recent downloads; `spider_agent` about 6k ([crates.io](https://crates.io/crates/spider_agent)).
- Stars: 2,768 (read 2026-10-10). The GitHub Release (v2.52.2, 2026-03-31) lags crates.io.

**Architecture: where AI sits**
- **`openai` / `gemini` features on `spider`:** the model generates browser-executable JS from a prompt (`with_openai(GPTConfigs::new(model, prompt, max_tokens))`). `cache_openai` / `cache_gemini` add a moka cache of model calls ([README](https://github.com/spider-rs/spider/blob/main/spider/README.md), [Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml)).
- **`spider_agent`** ([README](https://github.com/spider-rs/spider/blob/main/spider_agent/README.md)) is a multimodal automation and research agent:
  - Search providers: Serper, Brave, Bing, Tavily.
  - OpenAI-style tool calling.
  - **HTML diff mode**, sending only changes after the first round. It is claimed to cut tokens by 50-70% (self-reported).
  - Planning mode and parallel synthesis.
  - Confidence-based retries.
  - **Self-healing selectors** ("auto-repair failed selectors with LLM diagnosis").
  - `generate_schema` from examples.
  - Dependency-graph concurrent action chains.
  - `skills` registry injected into the prompt.
  - `memvid` memory.
  - `scripting`: sandboxed RustPython/Boa that the LLM can call (`RunPython` / `RunJavaScript`), with a `cap-std` filesystem sandbox.
- Perception: cleaned HTML (`spider_agent_html`), screenshots via `with_remote_multimodal`, and Chrome AI (Gemini Nano) budgets ([automation config](https://github.com/spider-rs/spider/blob/main/spider_agent/src/automation/config.rs)).
- Browser control: chromey (CDP) or WebDriver.

**Model and provider layer**
- Its own clients: OpenAI and OpenAI-compatible, Gemini, Anthropic extended-thinking budget (`with_thinking_budget`), and on-device Chrome AI.
- `CostTier` / `max_cost_tier` routes models per round ([spider_agent_types config](https://github.com/spider-rs/spider/blob/main/spider_agent_types/src/config.rs)).

**Interfaces**
- Rust API.
- `spider_cli`.
- **`spider_mcp`** (stdio) with tools `spider_scrape`, `spider_crawl`, `spider_links`, `spider_transform`. **None of them are AI tools** ([spider_mcp README](https://github.com/spider-rs/spider/blob/main/spider_mcp/README.md)).
- Spider Cloud ([llms.txt](https://spider.cloud/llms.txt)):
  - Hosted MCP at `mcp.spider.cloud`.
  - **`/ai/{scrape,crawl,search,links,browser}`** take a prompt and need an "AI plan". Prompt extraction is available only there.
  - Per-second caps by AI Studio tier: 1/5/10/25 rps.
  - `spider-agent` CLI and `spider-cloud-agent` crate.
  - x402 payments.

**Cost, verification and injection**
- Bounds: `max_rounds`, per-call `max_tokens`, and the cost-tier model policy.
- **No spend accounting surfaced in the docs read.**
- A code search for "injection" hit only skill-context limits; **no prompt-injection defense was found**.
- Verification: confidence scores and retries. No schema-validation judge was documented.

**Evaluation**
- Only self-reported README impacts: "~30% reduction in parse errors" from tool calling, and 50-70% token reduction from HTML diff.

**Agent-framework adapters (cloud)**
- LangChain, LlamaIndex, CrewAI, Agno, FlowiseAI, Zapier ([llms.txt](https://spider.cloud/llms.txt)).

**User pains**
- GitHub issues are few and low-signal, all closed (top: [#217 publish CLI binaries](https://github.com/spider-rs/spider/issues/217), 3 reactions).
- No AI-specific complaints found. The AI surface is new and lightly adopted (`spider_agent` has about 22.6k total downloads).

**Praised**
- No user quotes found in this pass.

---

### 4.5 Crawlee AI integrations (Apify)

**What it is**
- Crawlee JS 3.18.2 (2026-09-29; 26,103 stars) and Crawlee Python 1.10.4 (2026-10-06; 9,583 stars; about 238k PyPI downloads/week). Both Apache-2.0.

**Python `PydanticAiCrawler`** (experimental, from 1.8.0) ([guide](https://github.com/apify/crawlee-python/blob/master/docs/guides/pydantic_ai_crawler.mdx))
- HTTP fetch plus Parsel, then `context.extract(Model)`.
- **`PydanticAiDirectExtractor`** makes one LLM call per page and supports any schema shape.
- **`PydanticAiSelectorExtractor`**:
  - Generates CSS selectors on the first page of a route and **caches them per `cache_tag` (default: request label)**.
  - Reuses them with no model call.
  - Regenerates when no cached variant matches.
  - Falls back to another extractor for unsupported shapes.
- **Distillers** (`CleanHtml`, `Skeleton`, or custom such as Markdown) trade tokens against quality. `scope=` restricts to a subtree.
- `retries` re-asks the model when its output fails Pydantic validation.
- `ai_usage` accumulates per context and per crawl. **`usage_limits` (Pydantic AI `UsageLimits`) raises `UsageLimitExceeded`.**
- Model layer: Pydantic AI (OpenAI, Anthropic, Gemini, Ollama). The default is `openai:gpt-5.4-nano`.
- Origin: [#1593](https://github.com/apify/crawlee-python/issues/1593) noted demand for ScrapeGraphAI/Parsera-style extraction, which led to [PR #1964](https://github.com/apify/crawlee-python/pull/1964).

**JS `@crawlee/stagehand`**
- `StagehandCrawler` extends `BrowserCrawler` with `page.act/extract/observe`.
- `env: LOCAL | BROWSERBASE`; keys from OpenAI, Anthropic or Google ([README](https://github.com/apify/crawlee/blob/master/packages/stagehand-crawler/README.md)).
- Still pinned to **Stagehand ^3** (dev 3.0.7). About 3.5k npm downloads/week.
- Open requests:
  - [crawlee#4196 forward cdpUrl for remote browsers](https://github.com/apify/crawlee/issues/4196).
  - [crawlee-python#2154 support Stagehand 4.x](https://github.com/apify/crawlee-python/issues/2154).
- Crawlee Python's Stagehand integration caps `stagehand<4` (parity list).

**Apify MCP server**
- `@apify/actors-mcp-server` 0.17.4 (2026-10-07; 10,417 stars; about 37k npm downloads/week).
- Hosted at `mcp.apify.com` (Streamable HTTP; SSE removed). OAuth or a bearer token.
- Dynamic tool discovery: `search-actors` → `fetch-actor-details` → `call-actor`, plus `apify--rag-web-browser` and `apify--web-fetch`.
- Destructive tools such as `delete-actor` are opt-in only.
- Agentic payments: x402, Skyfire, AGI ([README](https://github.com/apify/apify-mcp-server)).

**Agent-framework adapters**
- Agno, Claude (Desktop and Code plugin), CrewAI, Google ADK, Haystack, LangChain, LangGraph, LlamaIndex, Mastra, n8n, OpenAI (Agents SDK, Assistants) and Vercel AI SDK ([docs index](https://docs.apify.com/llms.txt)).

**User pains**
- AI-specific issue volume is near zero; the features are recent.
- The main gap is the Stagehand version lag.
- Most-reacted adjacent request: [crawlee-python#1392 Scrapling as a parser](https://github.com/apify/crawlee-python/issues/1392) (5 reactions, 18 comments).

---

### 4.6 Comparables (brief)

**Scrapling** (BSD-3, v0.4.15 2026-08-23, 86,661 stars, about 213k PyPI downloads/week)
- **Non-LLM** adaptive relocation: `css(..., auto_save=True)` then `adaptive=True` relocates by stored element signatures.
- Ships an MCP server and `page.markdown()` ([README](https://github.com/D4Vinci/Scrapling)).
- Known limits:
  - [#259 relocates only 1 of N siblings after a class rename](https://github.com/D4Vinci/Scrapling/issues/259).
  - [#260 0 matches when the tag type changes](https://github.com/D4Vinci/Scrapling/issues/260).
- Its README benchmarks measure parsing speed (self-reported).

**LLM Scraper** (mishushakov, MIT, TypeScript, 6,937 stars; npm 2.0.0 2026-02-07, about 480 downloads/week)
- Playwright plus **Vercel AI SDK 6**, with Zod or JSON-Schema schemas.
- Streaming partial objects.
- **`generate` emits a reusable Playwright script** ([README](https://github.com/mishushakov/llm-scraper)).
- Top issues are local models: [#4 Ollama](https://github.com/mishushakov/llm-scraper/issues/4) (19 reactions) and [#1](https://github.com/mishushakov/llm-scraper/issues/1) (5). Also [#5 post-schema-validation check](https://github.com/mishushakov/llm-scraper/issues/5).
- HN launch: 88 points ([HN 40100824](https://news.ycombinator.com/item?id=40100824)). One user asked for "generating a reusable script with an LLM" ([jusgu](https://news.ycombinator.com/item?id=40103671)). Another: "scary openai bills feeding websites into gpt-4" ([jeffybefffy519](https://news.ycombinator.com/item?id=40104208)).

**Jina Reader / ReaderLM-v2**
- `r.jina.ai/<url>` prefix → LLM-friendly Markdown ([repo](https://github.com/jina-ai/reader), Apache-2.0, 12,127 stars, last push 2026-05-22).
- ReaderLM-v2 (HTML→Markdown/JSON small model) is **CC-BY-NC-4.0**, which makes it non-commercial ([HF](https://huggingface.co/jinaai/ReaderLM-v2)).

**Parsera** (GPL-2.0-or-later, 0.2.6 2025-10-08, about 200 downloads/week, 1,367 stars)
- `Parsera().run(url, elements={...})` on Playwright, with a hosted API ([README](https://github.com/raznem/parsera)).
- Low activity.

**Kadoa** (commercial, finance-focused)
- Claims "self-healing pipelines that are monitored and repaired automatically" and run-success metrics ([kadoa.com](https://www.kadoa.com/), marketing, unverified figures).

**Diffbot** (commercial)
- Extract APIs and Knowledge Graph. Its docs now pitch "Diffbot Skills" for agent harnesses ([docs.diffbot.com](https://docs.diffbot.com/), og-description only; details unverified).

---

### Theme tally (from the issues sampled above)

| Theme | Count | Examples |
|---|---|---|
| Local / OpenAI-compatible model friction | about 8 issues | Firecrawl #1294, #1467, #3169; Crawl4AI #649, #55; llm-scraper #4, #1; ScrapeGraphAI #1078 |
| Empty or malformed LLM output | about 5 | Firecrawl #1836; Crawl4AI #1202, #979, #1663; ScrapeGraphAI #893 |
| Token cost on large pages and batching | — | HN 40104208; ScrapeGraphAI #1036 |
| Upstream framework churn | — | ScrapeGraphAI on LangChain; Crawlee on Stagehand v3 |
| Cloud vs open-source feature gap | — | Firecrawl self-host table and #1129, #1390; Spider `/ai/*` cloud-only |
| Anti-bot and crawl etiquette | — | Firecrawl #2350, #1169; Crawl4AI #107 |

We found no explicit "hallucinated fields" issue cluster. It shows up indirectly as requests for post-validation (llm-scraper #5) and in the Crawl4AI docs telling users to validate output themselves.

## 5. Hosted browsers and computer-use APIs

Steel, Hyperbrowser and HyperAgent, Anchor and b0.dev, Kernel, Browserless, Amazon Nova Act, Bedrock AgentCore Browser, and the computer-use APIs of OpenAI, Anthropic and Google.

### Cross-cutting findings (read first)

1. **The computer-use loop contract has converged across OpenAI, Anthropic and Google.** In each case the model returns actions, the client executes them in its own browser or VM, and the client sends back a screenshot. Every vendor's GA surface changed in 2026:
   - OpenAI moved to **batched `actions[]`**. [OpenAI migration](https://developers.openai.com/api/docs/guides/tools-computer-use-integration)
   - Anthropic moved to a **17-member `computer_toolset_20260801`** that needs no beta header. [Anthropic](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
   - Google added **0-999 normalized coordinates plus an `intent` field and browser/desktop/mobile environments**. [Gemini](https://ai.google.dev/gemini-api/docs/computer-use)
2. **OpenAI now recommends "code execution" over the structured computer tool for its flagship model.** The model writes Playwright or PyAutoGUI code against a persistent sandbox. ([OpenAI guide](https://developers.openai.com/api/docs/guides/tools-computer-use)) Kernel's in-VM Playwright execution and its `browser_repl` take the same shape. ([Kernel](https://www.kernel.sh/docs/browsers/playwright-execution)) This "code mode" is relevant to an engine exposing JSON-RPC primitives.
3. **Infra vendors are moving from per-step agents to "compile once, replay deterministically, heal with AI."** Examples:
   - Anchor b0.dev and its self-healing drafts. [Anchor](https://docs.anchorbrowser.io/advanced/self-healing)
   - HyperAgent `actionCache` / `runFromActionCache`. [HyperAgent](https://github.com/hyperbrowserai/HyperAgent)
   - Nova Act workflows. [Nova Act](https://github.com/aws/nova-act)
4. **Prompt-injection defense is now a documented product feature at all three model vendors:**
   - Anthropic: screenshot classifiers that steer the model. [Anthropic](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
   - OpenAI: `pending_safety_checks`, surfaced by the Agents SDK as `on_safety_check`. [Agents SDK](https://openai.github.io/openai-agents-python/tools/)
   - Google: `safety_decision` / `require_confirmation`, policy categories, and opt-in `enable_prompt_injection_detection`. [Gemini](https://ai.google.dev/gemini-api/docs/computer-use)
5. **Credential isolation from the model is table stakes at the infra layer:**
   - Steel blurs injected fields and supports TOTP. [Steel](https://docs.steel.dev/overview/credentials-api/overview)
   - Kernel offers Fill-from-Vault and Managed Auth with reauth. [Kernel](https://www.kernel.sh/docs/auth/overview)
   - Anchor offers identities with MFA. [Anchor](https://docs.anchorbrowser.io/)
   - Nova Act's guidance: type secrets via Playwright, never in the prompt. [Nova Act](https://github.com/aws/nova-act)

---

### 5.1 Steel (steel-browser OSS + Steel Cloud)

**What it is.** An "open-source browser API for AI agents & apps".
- **License and language:** [Apache-2.0](https://github.com/steel-dev/steel-browser), TypeScript.
- **Adoption:** 7,780 stars.
- **Latest release:** `v0.5.4-beta`, 2026-08-25. It is still a beta tag.
- **SDKs:**
  - npm `steel-sdk` 0.18.0 (2026-03-16), about 44.8k downloads/week.
  - PyPI `steel-sdk` 0.19.0 (2026-06-23).
- **Cloud pricing is infrastructure only; there is no AI-step unit.** ([pricing](https://docs.steel.dev/overview/pricinglimits))
  - Launch: $0 + usage, $0.10/browser-hour, $10/GB proxy, $3 per 1k CAPTCHAs, 15-minute sessions, 10 concurrent.
  - Scale: $250/month, $0.08/h, $6/GB, $1 per 1k CAPTCHAs, 1-hour sessions, 100 concurrent.
  - Browser tools (`/scrape`, `/screenshot`, `/pdf`): $5 per 1k.

**Architecture.**
- **Who drives:** Steel does not run an agent loop. It is a session/CDP host that your agent (or a third-party framework) drives.
- **Control:** Puppeteer/CDP internally. Clients connect with Puppeteer, Playwright or Selenium. [README](https://github.com/steel-dev/steel-browser)
- **Perception:** HTML, markdown, readability, screenshots and PDF "to reduce token usage", with an unverified "up to 80%" saving claim. [intro](https://docs.steel.dev/overview/intro-to-steel)

**Model layer.** None built in. It integrates with the Claude, OpenAI and Gemini computer-use tools. [integrations](https://docs.steel.dev/integrations)

**Interfaces.**
- REST (OpenAPI/Swagger), CDP websocket, Selenium WebDriver.
- MCP server `steel-mcp-server` v2.0.1 (2026-08-24, 58 stars).
- x402 pay-per-call. [integrations](https://docs.steel.dev/integrations)

**Caching/replay.** Session replays and live viewer for debugging only. There is no action cache. [intro](https://docs.steel.dev/overview/intro-to-steel)

**Secrets and human-in-the-loop.**
- The Credentials API is in beta and free. It uses envelope encryption (AES-256-GCM per credential plus an org KMS key) and injects credentials by origin and namespace.
- TOTP is supported via `totpSecret`.
- `blurFields: true` blurs injected fields so vision agents cannot read them.
- Agents are told to "wait at least 2 seconds" for injection. [credentials](https://docs.steel.dev/overview/credentials-api/overview)
- Live session viewer. No documented prompt-injection defense; that is left to the model vendor.

**Evaluation.** Steel's own `browserbench` measures session-lifecycle latency, not task success. Earlier notes found Kernel ahead of Steel in Steel's own data (794 ms vs 894 ms). (Prior local note, not re-run.)

**Key OSS dependencies** ([api/package.json](https://github.com/steel-dev/steel-browser/blob/main/api/package.json)):
- Browser and server: `puppeteer-core`, `fastify`.
- Fingerprinting: `fingerprint-generator`, `fingerprint-injector`.
- Proxies: `proxy-chain`, `http-proxy`, `socks-proxy-agent`.
- Content extraction: `defuddle`, `jsdom`, `cheerio`, `mupdf`.
- Storage: `duckdb-async`, `level`.

**Adapters shipped** ([integrations](https://docs.steel.dev/integrations)):
- Agent frameworks: Vercel AI SDK, OpenAI Agents SDK, Claude Agent SDK, LangGraph, Pydantic AI, Mastra, CrewAI, Agno, AgentKit.
- Browser agents: Browser Use, Stagehand, Magnitude, Notte.
- Computer-use wiring for the Claude, Gemini and OpenAI tools.
- Coding agents: Claude Code, Codex, OpenClaw, Pi.

**User pains** (low issue volume; top issue has 6 reactions):
- Missing docs. [#123](https://github.com/steel-dev/steel-browser/issues/123)
- Self-hosted fingerprint generation failures. [#295](https://github.com/steel-dev/steel-browser/issues/295), [#302](https://github.com/steel-dev/steel-browser/issues/302) (Chrome 146)
- No cluster/session affinity. [#144](https://github.com/steel-dev/steel-browser/issues/144)
- Performance and reliability. [#97](https://github.com/steel-dev/steel-browser/issues/97)
- Pricing clarity. [#32](https://github.com/steel-dev/steel-browser/issues/32)
- Pricing data conflict: a competitor comparison lists Steel at $5/GB proxy and $2 per 1k CAPTCHAs ([Anchor vs Steel](https://llms.anchorbrowser.io/anchor-browser-vs-steel)), which differs from Steel's own page. Trust Steel's page.

**Praised.** Steel positions the Apache runtime as "the exit guarantee" ([Steel comparison](https://llms.steel.dev/articles/browser-infrastructure-for-ai-agents-compared)). This is self-description, not independent praise.

---

### 5.2 Hyperbrowser (cloud) + HyperAgent (OSS)

**What it is.** Cloud Chrome plus hosted agents.
- **SDKs:**
  - npm `@hyperbrowser/sdk` 1.1.0 (2026-10-08, MIT), about 72k downloads/week.
  - PyPI `hyperbrowser` 1.10.0 (2026-10-08).
- **HyperAgent** is the OSS agent:
  - License is **AGPL-3.0** ([LICENSE](https://github.com/hyperbrowserai/HyperAgent/blob/main/LICENSE); npm `@hyperbrowser/agent` 1.1.2, 2026-01-07, about 7.8k downloads/week).
  - 1,593 stars. Last GitHub release v1.0.0 (2025-11-19); last push 2026-05-11, so activity is slowing.
- **MCP repo** `hyperbrowserai/mcp`: MIT, 789 stars, last push 2025-11-20 (stale).
- **Pricing** ([pricing](https://www.hyperbrowser.ai/pricing)):
  - 1 credit = $0.001; browser 100 credits/h ($0.10/h); proxy 10,000 credits/GB.
  - Plans: Free (5k credits), $30, $100.
- **Hosted agents:** the Browser Use page says steps are billed in credits unless you bring your own keys (`useCustomApiKeys`). [browser-use](https://docs.hyperbrowser.ai/agents/browser-use)
- Per-step credit rates were not found on the pricing page (unverified).

**Architecture.** Two layers:
- **Hosted agents (vendor runs the loop).** Browser-Use, Claude Computer Use, OpenAI CUA, Gemini CU, Meta (Muse Spark) CU, Grok CU, Jev CU and HyperAgent. Each runs as an async job with `startAndWait()` and a `live_url`. [agents overview](https://www.hyperbrowser.ai/docs/agents/overview)
- **HyperAgent (code-called primitives on Playwright):**
  - `page.perform()` uses the accessibility tree with no screenshot and one LLM call per action.
  - `page.ai()` runs a multi-step loop with optional visual mode (screenshots with overlays).
  - `page.extract(schema)` returns data against a Zod schema.
  - `executeTask()` runs a whole task.
  - Execution is "CDP first" with OOPIF/iframe tracking and an ad-frame filter. [README](https://github.com/hyperbrowserai/HyperAgent)

**Model layer.** HyperAgent ships native OpenAI, Anthropic, Gemini and DeepSeek clients. [README](https://github.com/hyperbrowserai/HyperAgent)

**Interfaces.** SDK, REST, MCP server. HyperAgent itself is an MCP client (Composio example) and has a CLI (`cli.sh`).

**Caching/replay/repair.**
- `page.ai()` returns an `actionCache` (XPath, frame index, method per step).
- `runFromActionCache()` replays by XPath with `maxXPathRetries` and falls back to the LLM only when the page changed.
- `createScriptFromActionCache()` emits a standalone script. [README](https://github.com/hyperbrowserai/HyperAgent)
- This is close to Automan's compile/replay idea.

**Secrets and human-in-the-loop.** Live URL per task. No documented vault or prompt-injection defense in what I read (unverified).

**Evaluation.** HyperAgent has an `evals/` directory, but I found no published numbers (unverified).

**Key OSS dependencies** ([package.json](https://github.com/hyperbrowserai/HyperAgent/blob/main/package.json)):
- Browser: `playwright-core`, `patchright` (stealth Playwright fork).
- Model and tool SDKs: `@anthropic-ai/sdk`, `openai`, `@google/genai`, `@modelcontextprotocol/sdk`.
- Schemas and conversion: `zod`, `turndown`, `jimp`.

**Adapters.** Stagehand on Hyperbrowser; Browser Use hosted. [agents overview](https://www.hyperbrowser.ai/docs/agents/overview)

**User pains** (low volume: top issue has 2 reactions):
- Gemini models do not work. [#14](https://github.com/hyperbrowserai/HyperAgent/issues/14)
- Requests for page-change and redirect events. [#20](https://github.com/hyperbrowserai/HyperAgent/issues/20)
- Early request for "cache AI tasks to be run deterministically later" [#5](https://github.com/hyperbrowserai/HyperAgent/issues/5); since addressed by `actionCache`.
- The AGPL license is a reuse constraint.

**Praised.** No independent quotes found.

---

### 5.3 Anchor Browser (+ b0.dev)

**What it is.** Commercial cloud browsers for agents: CDP/REST sessions, natural-language tasks, versioned automation tasks, identities with MFA, profiles/volumes, recordings, live view and batch sessions. [docs](https://docs.anchorbrowser.io/)
- **SDKs:** npm `anchorbrowser` 1.0.0 (2026-07-26, Apache-2.0, about 35.5k downloads/week); PyPI `anchorbrowser` 1.0.1 (2026-07-28). The infrastructure itself is closed.
- **Pricing has a priced AI unit:** "AI Steps … $0.01/step", browser $0.09/h, proxy $8/GB, $0.01 per browser init in the calculator. [pricing](https://anchorbrowser.io/pricing)
- The plan tiers are expressed in credits.
- Anchor's own comparison page lists $0.05/browser-hour, which is inconsistent with its pricing page. [comparison](https://llms.anchorbrowser.io/anchor-browser-vs-steel)

**b0.dev.** Anchor's build-time agent (launch post dated 2026-01-21).
- Coding agents "plan and deploy deterministic browser tasks" and fall back to AI only when needed.
- Claims (all self-reported): 80x fewer tokens, 24x faster. [b0.dev launch](https://anchorbrowser.io/blog/anchor-launches-b0-dev---new-agentic-browser-paradigm)

**Architecture.**
- **Runtime agent:** `agentTask` / perform-web-task API. The agent can be `browser-use` (default), `openai-cua`, `gemini-computer-use` or `jev`.
- **Parameters include:** `use_vision`, `detect_elements`, `max_steps` (default 40), `output_schema`, `secret_values`, `human_intervention`.
- **Providers:** OpenAI, Gemini, Groq, Azure, Anthropic, xAI. [perform-web-task](https://docs.anchorbrowser.io/agentic-browser-control/ai-task-completion)

**Caching/replay/repair** ([self-healing](https://docs.anchorbrowser.io/advanced/self-healing)):
- **Workflow steps:** `ui`, `network`, `logic` and `agent`.
- **AI fallback (during a run):** an agent retries the failing step in the same browser, and the step is tagged **AI Fallback**.
- **Self-healing (after a run):** proposes a **draft version** that a human reviews and publishes via `POST /v2/tasks/{taskId}/publish-draft`. "Nothing goes live automatically."
- This is the most mature repair contract in this group.

**Interfaces.** REST, CDP, SDKs, and a hosted MCP at `api.anchorbrowser.io/mcp` with 24 tools; the MCP docs show Playwright test generation. [MCP](https://docs.anchorbrowser.io/advanced/mcp) There is also a programmatic "agent access" key flow. [docs](https://docs.anchorbrowser.io/)

**Secrets and human-in-the-loop.**
- Identities with MFA, `secret_values`, `human_intervention`, live view. [docs](https://docs.anchorbrowser.io/)
- Cloudflare Web Bot Auth "verified browser agents" on the $500 plan. [pricing](https://anchorbrowser.io/pricing)

**Evaluation.** "89% Score on … WebVoyager", self-reported, and attributed to using browser-use as a core component. [perform-web-task](https://docs.anchorbrowser.io/agentic-browser-control/ai-task-completion)

**Adapters.** Any MCP client (Cursor, VS Code, Claude, ChatGPT). [MCP](https://docs.anchorbrowser.io/advanced/mcp)

**User pains.** The SDK repos have about 2 stars and no public issue signal.

**Praised.** Vendor-hosted testimonials only:
- "built our MVP in less than an hour" (Benjamin Klieger, Groq). [anchorbrowser.com](https://www.anchorbrowser.com/)
- "the APIs are well-designed" (Samvit Jatia, Composio). [anchorbrowser.com](https://www.anchorbrowser.com/)

---

### 5.4 Kernel (kernel.sh, formerly onkernel)

**What it is.** "Internet runtime for agents": Chromium running on unikernel VMs.
- **Creation latency (self-reported):** about 30 ms p50 / 105 ms p99. [intro](https://www.kernel.sh/docs/introduction)
- **Standby:** a browser enters standby 5 s after the last client disconnects. [intro](https://www.kernel.sh/docs/introduction)
- **OSS parts:**
  - `kernel-images`: Apache-2.0, Go, 1,097 stars; the GitHub org moved from `onkernel` to `kernel`.
  - `hypeman` (VM runtime) and a `cloud-hypervisor` fork. [intro](https://www.kernel.sh/docs/introduction)
- **SDKs:**
  - npm `@onkernel/sdk` 0.123.0 (2026-10-09), about 584k downloads/week.
  - PyPI `kernel` 0.123.0.
  - CLI and a remote MCP server (`kernel/kernel-mcp-server`, MIT).
- **Pricing** ([pricing](https://www.kernel.sh/pricing)):
  - Usage at $0.0000166667 per GB-second; no charge for idle time or proxies.
  - Plans: $0 / $30 / $200 per month.
- There is no AI unit; Kernel does not sell model steps.

**Architecture.**
- Kernel provides no agent loop. You bring one, or deploy it with **App platform** (`kernel deploy` / `kernel invoke`), where it runs co-located with the browser. [intro](https://www.kernel.sh/docs/introduction)
- Control surfaces:
  - **Playwright execution** in the browser VM, exposed as `POST /browsers/{id}/playwright/execute` with `page`/`context`/`browser` and a `webmcp` helper; timeout default 60 s, max 300 s. [Playwright execution](https://www.kernel.sh/docs/browsers/playwright-execution)
  - **Computer controls:** OS-level click, move and drag with Bézier "human-like" paths, typing with an optional `--typo-chance`, keys, scroll, screenshots. [computer controls](https://www.kernel.sh/docs/browsers/computer-controls)
  - CDP and WebDriver BiDi.
  - A YAML accessibility-tree snapshot in the same ref-based format as Playwright MCP.
  - A **WebMCP** bridge (`listTools` / `invokeTool`, with `awaiting_submission` for populated-but-unsubmitted forms).

**Model layer.** Bring your own. Guides cover the Anthropic, OpenAI, Gemini, OpenAGI and Yutori (Navigator n1.5) computer-use models. [integrations](https://www.kernel.sh/docs/integrations/overview)

**Interfaces.** REST, SDKs, CLI, remote MCP (OAuth 2.1 with dynamic client registration). MCP tools include `execute_playwright_code`, `webmcp` and `browser_repl`. [MCP](https://www.kernel.sh/docs/reference/mcp-server)

**Caching/replay.** "Replays" are session videos, not action re-execution. (Prior note; consistent with the current docs.)

**Secrets** ([auth](https://www.kernel.sh/docs/auth/overview)):
- **Fill from Vault:** your agent drives the login and calls `fill`.
- **Managed Auth:** Kernel runs the login, SSO and MFA flow, saves the result to a profile, runs health checks and attempts automatic reauthentication.
- 1Password integration.
- The docs state the boundary explicitly: filled values are readable by page scripts and by any agent with unrestricted browser access.

**Human-in-the-loop and other features.** Live view; payments and wallets; Web Bot Auth; a "config registry" of recommended per-site settings. [pricing/blog](https://www.kernel.sh/pricing)

**Evaluation.** Lifecycle benchmarks only. [benchmarks](https://www.kernel.sh/benchmarks)

**Adapters** ([integrations](https://www.kernel.sh/docs/integrations/overview)):
- Agent frameworks: Browser Use, Stagehand, Claude Agent SDK, Claude Managed Agents, Claude Code/Desktop plugin.
- Other tools: Vercel agent-browser and fx, Hermes, Replit, Laminar, Terraform, Val Town.

**User pains.** `kernel-images` issues are low-reaction and mostly about the VNC/WebRTC live view:
- Copy/paste. [#31](https://github.com/kernel/kernel-images/issues/31)
- WebRTC disconnects on unikernels. [#27](https://github.com/kernel/kernel-images/issues/27)
- Double cursors. [#28](https://github.com/kernel/kernel-images/issues/28)

**Praised.** Steel's comparison concedes Kernel's "agent code on its own platform, colocated with the browser", and calls the trade-off architectural lock-in. [Steel comparison](https://llms.steel.dev/articles/browser-infrastructure-for-ai-agents-compared)

---

### 5.5 Browserless (BrowserQL, MCP, browser agent)

**What it is.** Managed and self-hosted browser infrastructure.
- **License:** "SSPL-1.0 OR Browserless commercial license", not permissive (prior note; the GitHub API reports NOASSERTION). There is an open license-clarification issue with 27 reactions. [#3850](https://github.com/browserless/browserless/issues/3850)
- **Adoption:** 13,797 stars, latest release v2.57.0 (2026-09-30).
- **Pricing** ([pricing](https://www.browserless.io/pricing)):
  - **Unit** = up to 30 s per connection; reconnects are new units.
  - Proxies: residential 6 units/MB, datacenter 2 units/MB.
  - CAPTCHA: 10 units per successful solve.
  - Plans: Free (1k units, 2-minute sessions), $25, $140, $350 per month billed annually.
- There is no AI unit.

**Architecture.** WebSocket CDP for Puppeteer/Playwright; Chrome, WebKit and Firefox; BrowserQL (a GraphQL-style automation language and editor); REST endpoints; a "Smart Scrape" API using cascading strategies. [llms.txt](https://docs.browserless.io/llms.txt)

**AI layer.** The MCP `browserless_agent` tool runs a "snapshot-based" ReAct loop: Reason, Act, Observe. Since MCP server v1.33.0 sessions are one-shot unless you opt in to keep them. [browser agent](https://docs.browserless.io/mcp/browser-agent)

**Interfaces.** BrowserQL, REST, CDP, two MCP servers (automation and docs), and a Claude Code plugin. [AI integrations](https://docs.browserless.io/ai-integrations/start)

**Human-in-the-loop.** **Hybrid Automation**: `Browserless.liveURL` over CDP returns a short-lived link with no token embedded. Events include `Browserless.liveComplete` and `Browserless.captchaFound`, intended for login, 2FA and CAPTCHA handoff. [hybrid automation](https://docs.browserless.io/baas/monitor-sessions/hybrid-automation)

**Adapters** ([AI integrations](https://docs.browserless.io/ai-integrations/start)):
- Agent frameworks: Browser Use, Stagehand, LangChain, Inngest AgentKit, Vercel AI SDK, Claude Agent SDK.
- Vercel agent-browser.
- OpenAI and Anthropic SDKs via their MCP connectors.
- Hermes, opencode.

**User pains** (by reactions):
- License confusion. [#3850](https://github.com/browserless/browserless/issues/3850) (27 reactions)
- Kubernetes autoscaling metrics and health checks. [#1698](https://github.com/browserless/browserless/issues/1698), [#1420](https://github.com/browserless/browserless/issues/1420), [#3892](https://github.com/browserless/browserless/issues/3892)
- Chrome security-patch lag. [#3529](https://github.com/browserless/browserless/issues/3529)
- The top issue (128 reactions) is a complaint about outreach spam to stargazers. [#2234](https://github.com/browserless/browserless/issues/2234)

**Praised.** "You're doing an amazing job!" (23 reactions). [#41](https://github.com/browserless/browserless/issues/41)

---

### 5.6 Amazon Nova Act (SDK + AWS service)

**What it is.** An AWS service (GA 2025-12-02) plus an SDK for UI-workflow agents.
- **Model:** "Powered by a custom Nova 2 Lite model" trained with reinforcement learning on "hundreds of simulated web environments". [AWS News](https://aws.amazon.com/blogs/aws/build-reliable-ai-agents-for-ui-workflow-automation-with-amazon-nova-act-now-generally-available/?utm_custom=ft_lightsail), [aboutamazon](https://www.aboutamazon.com/news/aws/aws-agentic-ai-amazon-bedrock-nova-models)
- **SDK:** `aws/nova-act`, Apache-2.0, Python, 918 stars. Latest release v3.4.187.0 (2026-04-30); PyPI is the same. Versions older than 3.0 are unsupported. [repo](https://github.com/aws/nova-act)
- **Pricing:** **$4.75 per agent hour** of elapsed time; time spent waiting on a human (HITL) is excluded. [Nova pricing](https://aws.amazon.com/nova/pricing/)

**Architecture.**
- **Who drives:** Nova Act runs the agent loop inside each `act()` call, and your Python code orchestrates the calls.
- **`act()`** is natural language. Each call is a sequence of steps of the form "observe page, take action", with `max_steps` default 30.
- **`act_get(schema=…)`** returns structured output validated with Pydantic.
- A **Workflow** context manager calls the `CreateWorkflowRun` and `UpdateWorkflowRun` service APIs.
- **`nova.page`** exposes the raw Playwright `Page`.
- The vendor describes the model, orchestrator and actuator as a "vertically integrated" stack. [user guide](https://docs.aws.amazon.com/nova-act/latest/userguide/what-is-nova-act.html), [repo](https://github.com/aws/nova-act)

**Model layer.** Single vendor: `model_id="nova-act-latest"`. [repo](https://github.com/aws/nova-act)

**Interfaces.** Python SDK, IDE extension (chat-to-script), CLI, playground, AWS console. Tools via `@tool`, remote MCP and Strands Agents are in preview. [repo](https://github.com/aws/nova-act)

**Human-in-the-loop.** `HumanInputCallbacksBase` with two patterns:
- `approve()`: a screenshot is shown to a reviewer.
- `ui_takeover()`: live-stream control, for example to solve a CAPTCHA. [repo](https://github.com/aws/nova-act)

**Safety and secrets.**
- `SecurityOptions` allow-lists for `file://` navigation and uploads.
- **State guardrails:** a callback on each observation returns PASS or BLOCK by URL, and BLOCK raises `ActStateGuardrailError`.
- Secrets are typed via Playwright, never put in the prompt.
- Session persistence options: local profile, S3 (optionally KMS-encrypted), or AgentCore Browser profiles. [repo](https://github.com/aws/nova-act)

**Cost accounting.** A `time_worked` utility excludes human-wait time. [repo](https://github.com/aws/nova-act)

**Evaluation** (all self-reported):
- "over 90% reliability on early customer workflows". [AWS News](https://aws.amazon.com/blogs/aws/build-reliable-ai-agents-for-ui-workflow-automation-with-amazon-nova-act-now-generally-available/?utm_custom=ft_lightsail)
- Leading on ScreenSpot and GroundUI Web; no numbers quoted on the page. [Amazon Science](https://www.amazon.science/blog/prototype-to-production)

**User pains** (low volume):
- MCP integration request. [#17](https://github.com/aws/nova-act/issues/17) (8 reactions, since shipped in preview)
- `ActProtocolError` on the sample. [#8](https://github.com/aws/nova-act/issues/8)
- Narrow allowed screen resolution. [#53](https://github.com/aws/nova-act/issues/53)

---

### 5.7 Amazon Bedrock AgentCore Browser

**What it is.** A managed, isolated browser runtime, delivered as AWS cloud only. [dev guide](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html)
- **Options:** the managed `aws.browser.v1`, or custom browsers with session recording, network settings and IAM roles.
- **Sessions:** default 15 minutes, maximum 8 hours.
- **Endpoints:** an Automation endpoint (WebSocket/CDP) and a Live View endpoint where the user can watch and interact.
- **Observability:** CloudTrail; recordings capture DOM, actions, console and network to S3, replayable in the console.
- **SDK:** PyPI `bedrock-agentcore` 1.24.1 (2026-10-07).

**Model layer.** None. You drive it with Strands, Nova Act or Playwright. [dev guide](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html)

**Pricing** ([pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)):
- $0.0895 per vCPU-hour plus $0.00945 per GB-hour, billed per second on **active consumption**: CPU during I/O wait is free.
- Profiles are billed at S3 rates from 2026-04-15.
- Worked example: 10-minute session at 80% I/O wait costs about $0.0123.

**Evaluation.** None (infrastructure only).

---

### 5.8 OpenAI computer use (Responses API `computer` tool, code-execution mode, Agents SDK, ChatGPT agent)

**Models and status.**
- **Retired:** `computer-use-preview` was deprecated 2026-04-22 and **shut down 2026-07-23**. [deprecations feed](http://deprecations.info/v1/feed.xml)
- **Current GA tool:** `{"type":"computer"}` on GPT-5.4+ / GPT-5.6 / **`gpt-6.1-sol`** (the docs' default example). [guide](https://developers.openai.com/api/docs/guides/tools-computer-use)
- **Flagship guidance:** for **`gpt-6-astra`**, OpenAI "recommend[s] code execution". [guide](https://developers.openai.com/api/docs/guides/tools-computer-use)
- **GPT-6.1 Sol** launched 2026-09-29 at $2 input / $10 output per million tokens (third-party summary). [Fello AI](https://felloai.com/gpt-6-1-sol/)

**Loop contract (computer tool).**
- The model returns a `computer_call` with **batched `actions[]`**: `click`, `double_click`, `drag`, `move`, `scroll`, `keypress`, `type`, `wait`, `screenshot`.
- The first call may be screenshot-only.
- The client executes the actions in order and returns a `computer_call_output` containing a `computer_screenshot` (`detail: "original"`).
- The conversation continues via `previous_response_id`.
- Preview-era requirements are gone: `truncation:"auto"`, the display dimensions and the environment field are no longer needed. [migration](https://developers.openai.com/api/docs/guides/tools-computer-use-integration)

**Code-execution mode.** A plain function tool (`exec_js` / `exec_py`) receives a code string and runs it in a persistent Playwright browser or PyAutoGUI desktop. Output is returned via `display()` and `log()`. The sample uses a 1440x900 viewport and a 20-turn cap. [guide](https://developers.openai.com/api/docs/guides/tools-computer-use)

**Safety.**
- `pending_safety_checks` on a call; codes seen in the wild include `malicious_instructions`. [community thread](https://community.openai.com/t/computer-tool-has-unacknowledged-safety-check/1147615)
- The client must acknowledge each check (Azure documents `acknowledged_safety_checks`). [MS Foundry](https://learn.microsoft.com/en-us/azure/foundry/agents/how-to/tools/computer-use)
- Agents SDK `ComputerTool.on_safety_check`: returning False raises `UserError`. If no callback is set, the SDK "proceeds… and leaves the checks unacknowledged". [Agents SDK Python](https://openai.github.io/openai-agents-python/tools/)
- Agents SDK JS `computerTool({needsApproval, onSafetyCheck})` evaluates approval per batched action. [Agents SDK JS](https://openai.github.io/openai-agents-js/guides/tools/)
- Guidance: restrict the environment with allow-lists; treat screen content as untrusted ("typing sensitive information … counts as transmission"); confirm consequential actions; bound steps/time/cost and verify outcomes. [guide](https://developers.openai.com/api/docs/guides/tools-computer-use)
- Release notes mention asynchronous "misalignment monitoring" for supported Responses requests. [release notes RSS](https://openai.com/products/release-notes/rss.xml)

**Agents SDK.**
- Versions: `openai-agents` 0.23.1 (PyPI, 2026-10-02); `@openai/agents` 0.20.0 (npm, 2026-10-08, about 2.19M downloads/week).
- `ComputerTool` is a local harness over the `Computer` / `AsyncComputer` interface. The SDK picks the GA or preview wire shape from the effective model.
- The SDK's default model `gpt-5.6-luna` supports computer use. [Agents SDK Python](https://openai.github.io/openai-agents-python/tools/)

**Consumer product.**
- Operator was folded into ChatGPT agent mode (July 2025) and now lives in the "ChatGPT Work" cloud browser (third-party summary). [Coursiv](https://coursiv.io/blog/chatgpt-operator)
- The Atlas browser was scheduled to stop on 2026-08-09. Codex adds Mac "Computer Use" and "Record & Replay". [ChatGPT release notes](https://help.openai.com/en/articles/6825453-how-can-i-contact-openai)

**Evaluation (self-reported).**
- OSWorld 2.0 (offline set, partial reward): GPT-6.1 Sol beats GPT-6 Sol by 7 points at max effort for "less than half the cost", and is within 2.1 points of Astra at about one-seventh of the cost per task. [OpenAI](https://openai.com/index/introducing-gpt-6-1-sol/)
- GPT-6 Sol: 60.5% on OSWorld 2.0 at xhigh, as relayed by a third party. [Puter](https://developer.puter.com/ai/openai/gpt-image-2.5-flare/)
- Online-Mind2Web: "OpenAI Operator 61%" in Browser Use's table. That is a third party and a competitor, so not independent. [Browser Use benchmarks](https://browser-use.com/benchmarks.md)

**User pains:**
- Model access 404s and the preview shutdown. [cua-sample-app #62](https://github.com/openai/openai-cua-sample-app/issues/62), [#17](https://github.com/openai/openai-cua-sample-app/issues/17)
- Runs with 500 errors that never complete. [#54](https://github.com/openai/openai-cua-sample-app/issues/54)
- **Refuses login/signup.** [#39](https://github.com/openai/openai-cua-sample-app/issues/39)
- Input-token overhead. [#25](https://github.com/openai/openai-cua-sample-app/issues/25)
- Computer tool combined with a function call bug on `gpt-5.6-luna`. [OpenAI forum](https://community.openai.com/c/api/7?page=13)
- Cost theme: "Computer Use is 45x more expensive than structured APIs" (495 points). One commenter: latency and cost "are absolute dealbreakers" (jacktu). [HN](https://news.ycombinator.com/item?id=48024859)

**Praised.** Batched `actions[]` and code mode reduce round-trips. No independent quote under 15 words found.

---

### 5.9 Anthropic computer use (toolset, Claude in Chrome, Claude Agent SDK)

**Tool versions** ([computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)):
- **`computer_toolset_20260801`** is current. It needs no beta header and is the **only** option on Claude 5.5+ (Claude API and Google Cloud).
- `computer_20251124` (beta header `computer-use-2025-11-24`) remains for Opus 4.5 through Fable/Mythos 5.1, and for 5.5 models on Bedrock.
- `computer_20250124` remains for older models.

**Action space.** 17 members:
- **Observe:** `screenshot`; `zoom` with `region:[x0,y0,x1,y1]` at full resolution.
- **Click:** `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, with optional `coordinate` and modifier `text`.
- **Mouse:** `left_click_drag`, `mouse_move`.
- **Keyboard:** `type`; `key` with `repeat` 1-100; `hold_key` up to 300 s.
- **Other:** `scroll` (direction, amount, coordinate, modifiers); `wait` up to 300 s.
- Further members exist; the excerpt I read did not list all 17 (unverified).
- **Config:** `configs` per member (`enabled`, `defer_loading`), `cache_control`, `allowed_callers:["direct"]`.

**Loop contract.**
- The model emits `tool_use` blocks with `toolset_name:"computer"`. The client runs them in order and returns all `tool_result`s in one user message.
- After a failure, the remaining calls must return the prescribed text "Not executed: an earlier computer action in this turn failed."
- Screenshots up to 2576 px on the long edge / 4784 visual tokens on Opus 4.7+. [computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)

**Cost accounting.**
- The toolset adds about 4,500 input tokens per request; disabling zoom saves about 410.
- Screenshots and zoom images are billed as image input. [computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
- Thinking-effort advice: Opus 4.7 `high`; Sonnet/Opus 4.6 `medium`; `low` "uses *fewer* output tokens than disabling thinking".

**Prompt-injection defenses.**
- Classifiers "automatically scan what the tools return, such as screenshots" and steer the model to confirm the instruction came from the user. [computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
- Claude in Chrome attack-success-rate (ASR) history:
  - August 2025 research preview: 23.6% ASR, cut to 11.2% with mitigations. [Anthropic](https://www.anthropic.com/news/claude-for-chrome)
  - November 2025: 1% ASR for Opus 4.5. [Anthropic](https://www.anthropic.com/news/prompt-injection-defenses)
  - Help-center article: "<0.08%" (Opus 4.8). [Help Center](https://support.claude.com/en/articles/12902428-use-claude-in-chrome-safely)
  - **GA 2026-08-26:** 0% for Sonnet 5 and Opus 5 with probes plus classifiers, 0.3% for Fable 5. All are internal adaptive-attacker evaluations. [GA post](https://claude.com/blog/claude-in-chrome-generally-available)

**Claude in Chrome.** GA on all paid plans; works with Claude Code, Cowork and Desktop. [Claude for Chrome](https://claude.com/claude-for-chrome)

**Claude Agent SDK.**
- Versions: npm `@anthropic-ai/claude-agent-sdk` 0.3.296 (about 12M downloads/week); PyPI `claude-agent-sdk` 0.2.165.
- No first-party hosted browser. Browser access comes from MCP (Playwright MCP, vendor MCPs) or the Chrome integration. Steel, Kernel and Browserless all ship Claude Agent SDK guides. [Steel](https://docs.steel.dev/integrations), [Kernel](https://www.kernel.sh/docs/integrations/overview), [Browserless](https://docs.browserless.io/ai-integrations/start)
- Whether the SDK exposes Claude in Chrome as an option is unverified.

**Evaluation (self-reported).** OSWorld 2.1 "81.8% partial". This is the first column of the Opus 5.5 launch table, so the column-to-model mapping is inferred from the excerpt (unverified). [Opus 5.5](https://www.anthropic.com/claude-opus-5-5)
- Third-party aggregators disagree. BenchLM shows Opus 5.5 at 48.7% on OSWorld 2.0, a different benchmark version or harness. [BenchLM](https://benchlm.ai/compare/claude-opus-5-5-vs-north-micro-vision-instruct)
- Do not mix these numbers.

**User pains** ([claude-code](https://github.com/anthropics/claude-code/issues/16128), [quickstarts](https://github.com/anthropics/claude-quickstarts/issues/116)):
- Claude in Chrome feature gaps:
  - Bedrock auth. [#16128](https://github.com/anthropics/claude-code/issues/16128) (121 reactions)
  - Targeting specific Chrome profiles. [#15125](https://github.com/anthropics/claude-code/issues/15125) (89)
  - Extension not connecting. [#21371](https://github.com/anthropics/claude-code/issues/21371) (79)
  - Browser selection, WSL, other Chromium browsers. [#14536](https://github.com/anthropics/claude-code/issues/14536), [#14367](https://github.com/anthropics/claude-code/issues/14367), [#14370](https://github.com/anthropics/claude-code/issues/14370)
- Demo-harness pains:
  - No token-usage display. [#116](https://github.com/anthropics/claude-quickstarts/issues/116)
  - Crashes. [#278](https://github.com/anthropics/claude-quickstarts/issues/278)
  - noVNC connection failures. [#42](https://github.com/anthropics/claude-quickstarts/issues/42)
- The docs themselves note click-precision differences between models ("Sonnet 4.6 is more mechanically precise at clicking than Claude Opus 4.6"). [computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)

---

### 5.10 Google Gemini computer use (and Project Mariner)

**Models** ([Gemini computer use](https://ai.google.dev/gemini-api/docs/computer-use)):
- `gemini-3.8-flash` is recommended.
- `gemini-3.5-flash-lite` is the low-latency option.
- `gemini-3-flash-preview` is also listed.
- The original `gemini-2.5-computer-use-preview-10-2025` was launched 2025-10-07. [DeepMind](https://deepmind.google/blog/introducing-the-gemini-25-computer-use-model)
- SDKs: npm `@google/genai` 2.28.0, PyPI `google-genai` 2.29.0.

**Loop contract.**
- Configured as `tools=[{"type":"computer_use","environment":"browser"|"desktop"|"mobile"}]` via `client.interactions.create`.
- The model returns `function_call` steps with **0-999 normalized x/y** and an `intent` string.
- The client executes the call and returns a screenshot.
- A `safety_decision` may require confirmation, in which case the client returns `safety_acknowledgement: true`. [Gemini computer use](https://ai.google.dev/gemini-api/docs/computer-use)

**Action space** (browser and desktop environments):
- Clicks: `click`, `double_click`, `triple_click`, `middle_click`, `right_click`.
- Mouse: `mouse_down`, `mouse_up`.
- Keyboard: `press_key`, `key_down`, `key_up`, `hotkey`.
- Observe and scroll: `take_screenshot`; `scroll` with `magnitude_in_pixels`.
- The mobile environment adds `open_app` and `long_press`.
- Predefined actions can be excluded and custom functions (for example `yield_to_user`) added. [Gemini computer use](https://ai.google.dev/gemini-api/docs/computer-use)

**Safety.**
- Policy categories: `FINANCIAL_TRANSACTIONS`, `SENSITIVE_DATA_MODIFICATION`, `COMMUNICATION_TOOL`, `ACCOUNT_CREATION`, `DATA_MODIFICATION`, `USER_CONSENT_MANAGEMENT`, `LEGAL_TERMS_AND_AGREEMENTS`.
- Opt-in `enable_prompt_injection_detection` for 3.5 Flash-Lite and later; it blocks execution when triggered.
- A recommended system-instruction template stops before irrecoverable actions and forbids solving CAPTCHAs.
- Logging guidance: log screenshots, actions and safety responses. [Gemini computer use](https://ai.google.dev/gemini-api/docs/computer-use)

**Project Mariner.** The product page says its "computer use capabilities [are coming] into the Gemini API". The 2.5 model powered Mariner, the Firebase Testing Agent and AI Mode agentic features. [Mariner](https://deepmind.google/models/project-mariner), [DeepMind](https://deepmind.google/blog/introducing-the-gemini-25-computer-use-model) Its current consumer status is unverified.

**Evaluation.**
- **2.5 CU:** leads Online-Mind2Web, WebVoyager and AndroidWorld using a mix of self-reported numbers and **Browserbase-run evaluations with identical harnesses and human grading**; this is the most independent figure in this section. [DeepMind](https://deepmind.google/blog/introducing-the-gemini-25-computer-use-model), [eval PDF](https://storage.googleapis.com/deepmind-media/gemini/computer_use_eval_additional_info.pdf)
- **3.8 Flash:** OSWorld 2.0 is self-computed: max over 3 runs, 1080p, 500 steps, with batched tool calls added for all models, and run before the 08.08 patch. [methodology PDF](https://storage.googleapis.com/deepmind-media/gemini/gemini_3-8_flash_model_evaluation.pdf) I did not extract the score itself.
- Third-party, competitor-run: "Gemini CUA 69%" and "Stagehand (Gemini 2.5 CU) 65%" on Online-Mind2Web. [Browser Use](https://browser-use.com/benchmarks.md)

**User pains:**
- `safety_acknowledgement` is still demanded after the user confirms in the ADK UI. [#97](https://github.com/google-gemini/computer-use-preview/issues/97) (7 reactions)
- "Coordination inaccuracy". [#83](https://github.com/google-gemini/computer-use-preview/issues/83)
- `PLAYWRIGHT_HEADLESS=false` ignored. [#119](https://github.com/google-gemini/computer-use-preview/issues/119)
- Forum: `gemini-3-flash-preview` "constantly overwhelmed with 503 errors" with "<50% success"; computer use not enabled on 3.1-pro-preview. [Google forum](https://discuss.ai.google.dev/t/future-of-computer-use/171397)
- The reference repo has 3,217 stars; Apache-2.0.

**Praised.** "I love the Gemini API as a tool" (TylerShetrompf). [Google forum](https://discuss.ai.google.dev/t/future-of-computer-use/171397)

---

### User pain themes across this group

| Theme | Evidence |
|---|---|
| Cost per step / tokens | HN "45x more expensive" (495 pts) [HN](https://news.ycombinator.com/item?id=48024859); toolset overhead of about 4.5k tokens per request [Anthropic](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool); input-token overhead [OpenAI #25](https://github.com/openai/openai-cua-sample-app/issues/25); token-usage UI request [#116](https://github.com/anthropics/claude-quickstarts/issues/116) |
| Latency / availability | Gemini 503s [forum](https://discuss.ai.google.dev/t/future-of-computer-use/171397); OpenAI runs with 500s that never complete [#54](https://github.com/openai/openai-cua-sample-app/issues/54) |
| Safety refusals and acknowledgement friction | CUA refuses login/signup [#39](https://github.com/openai/openai-cua-sample-app/issues/39); Gemini acknowledgement loop [#97](https://github.com/google-gemini/computer-use-preview/issues/97) |
| Coordinate precision | Gemini [#83](https://github.com/google-gemini/computer-use-preview/issues/83); Anthropic's own model-precision note [docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool); Nova Act resolution limits [#53](https://github.com/aws/nova-act/issues/53) |
| Model churn / deprecation | `computer-use-preview` shut down 2026-07-23 [feed](http://deprecations.info/v1/feed.xml); 404 issues [#62](https://github.com/openai/openai-cua-sample-app/issues/62) |
| Self-hosting / ops | Steel fingerprint failures [#295](https://github.com/steel-dev/steel-browser/issues/295); Browserless k8s metrics [#1698](https://github.com/browserless/browserless/issues/1698); license confusion [#3850](https://github.com/browserless/browserless/issues/3850) |
| Session/browser selection | Claude in Chrome profiles and browsers [#15125](https://github.com/anthropics/claude-code/issues/15125), [#14536](https://github.com/anthropics/claude-code/issues/14536) |

### Relevance to Automan (brief)

- **Wire contract:** batched actions, plus a "code mode" function tool that runs scripts next to the browser (OpenAI, Kernel), are the two shapes to support over JSON-RPC.
- **Safety-check plumbing:** acknowledgement callbacks (OpenAI `on_safety_check`, Gemini `safety_acknowledgement`) map naturally onto an engine-level approval hook.
- **Repair contract:** Anchor's "AI fallback during the run, draft repair after the run, human publishes" and HyperAgent's XPath-first replay with LLM fallback are the clearest existing contracts.
- **Licenses:** HyperAgent is AGPL-3.0 and Browserless is SSPL or commercial. Steel, Kernel images and Nova Act SDK are Apache-2.0.

## 6. Agent frameworks and adapters

### 6.1 Adoption signals for agent frameworks

All numbers were read on 2026-10-10.
- **npm.** "Week" is `api.npmjs.org/downloads/point/last-week/<pkg>`, covering 2026-10-02 to 2026-10-08. "Month" is `last-month`.
- **PyPI.** "Week" and "Month" are `pypistats.org/api/packages/<pkg>/recent`, fields `last_week` and `last_month`, without mirrors.
- **GitHub.** Stars come from `gh repo view`. Licenses come from `gh api repos/<r>/license` and the registry metadata.
- **Latest version.** Read from `registry.npmjs.org/<pkg>` (dist-tag `latest`, with its publish time) and from `pypi.org/pypi/<pkg>/json` (upload time).

#### 1a. Table

| Framework | Lang | Exact package | Week | Month | GitHub stars (repo) | Latest version (date) | License | How tools are defined | Consumes MCP servers natively? |
|---|---|---|---:|---:|---|---|---|---|---|
| **MCP SDK (baseline)** | TS | `@modelcontextprotocol/sdk` | 65,440,256 | 270,975,265 | 13,548 (modelcontextprotocol/typescript-sdk) | 1.32.1 (2026-10-05) | MIT | n/a (protocol) | n/a. The v2 line ships as split packages: `@modelcontextprotocol/server` had 10,051,078/wk and `@modelcontextprotocol/client` had 6,881,460/wk. The repo's latest release is v2.3.1. |
| **MCP SDK (baseline)** | Py | `mcp` | 60,601,326 | 235,193,557 | 24,534 (modelcontextprotocol/python-sdk) | 2.3.0 (2026-10-02) | MIT | n/a | n/a |
| Vercel AI SDK | TS | `ai` | 29,759,300 | 124,369,041 | 27,228 (vercel/ai) | 7.0.137 (2026-10-09) | Apache-2.0 (npm) | `tool({ description, inputSchema })` with Zod or Standard Schema, or `jsonSchema()` ([docs](https://ai-sdk.dev/docs/foundations/tools)) | **Yes.** The client is in `@ai-sdk/mcp` (`createMCPClient`; 4,924,926/wk). The docs call it a "lightweight client" for tool conversion and recommend HTTP for production ([docs](https://ai-sdk.dev/docs/ai-sdk-core/mcp-tools)). |
| OpenAI Agents SDK | Py | `openai-agents` | 3,022,772 | 12,449,784 | 29,951 (openai/openai-agents-python) | 0.23.1 (2026-10-02) | MIT | `@function_tool` builds a Pydantic schema from the signature and docstring | **Yes.** It offers `MCPServerStdio`, `MCPServerStreamableHttp` and the hosted MCP tool ([docs](https://openai.github.io/openai-agents-python/mcp/)). |
| OpenAI Agents SDK | TS | `@openai/agents` | 2,190,895 | 8,850,401 | 3,903 (openai/openai-agents-js) | 0.20.0 (2026-10-08) | MIT | `tool({ parameters: z.object(...) })` (Zod) | **Yes** ([docs](https://openai.github.io/openai-agents-js/guides/mcp/)). Extensions ship in `@openai/agents-extensions` (349,051/wk). |
| LangChain | Py | `langchain` | 37,288,414 | 170,457,766 | 147,560 (langchain-ai/langchain) | 1.4.4 (2026-10-08) | MIT | `@tool` decorator, Pydantic `args_schema`, `StructuredTool` | **Yes, now built in.** It lives in `langchain.mcp` via `langchain[mcp]`. The former `langchain-mcp-adapters` (1,585,128/wk) is "no longer actively maintained" ([README](https://github.com/langchain-ai/langchain-mcp-adapters), [docs](https://docs.langchain.com/oss/python/langchain/mcp)). |
| LangChain core | Py | `langchain-core` | 34,673,190 | 144,826,657 | (same repo) | 1.6.9 (2026-10-08) | MIT | (same) | (same) |
| LangChain | TS | `langchain` | 3,058,638 | 13,637,562 | 18,252 (langchain-ai/langchainjs) | 1.5.16 (2026-10-08) | MIT | `tool(fn, { schema: z.object })` (Zod) | **Yes.** It uses `@langchain/mcp-adapters` (269,222/wk) ([docs](https://docs.langchain.com/oss/javascript/langchain/mcp)). |
| LangChain core | TS | `@langchain/core` | 6,246,380 | 26,667,542 | (same) | 1.2.17 (2026-10-06) | MIT | (same) | (same) |
| LangGraph | Py | `langgraph` | 12,117,152 | 44,769,952 | 43,018 (langchain-ai/langgraph) | 1.2.14 (2026-10-06) | MIT | LangChain tools | **Yes**, through LangChain MCP. Elicitation surfaces as a LangGraph interrupt ([blog](https://www.langchain.com/blog/mcp-in-langchain-stateless-protocol-elicitation-and-more)). |
| LangGraph | TS | `@langchain/langgraph` | 3,765,811 | 16,186,326 | 3,342 (langchain-ai/langgraphjs) | 1.4.21 (2026-10-07) | MIT | LangChain tools | **Yes**, through the adapters |
| Claude Agent SDK | TS | `@anthropic-ai/claude-agent-sdk` | 11,982,584 | 52,607,175 | 1,798 (anthropics/claude-agent-sdk-typescript) | 0.3.296 (2026-10-09) | "SEE LICENSE IN README.md" (Anthropic terms; the repo has no SPDX license) | `tool(name, desc, zodShape, handler)`, wrapped in `createSdkMcpServer`. **Custom tools are in-process MCP servers** ([docs](https://code.claude.com/docs/en/agent-sdk/custom-tools)). | **Yes, MCP is the only tool interface.** It takes stdio, HTTP or SSE servers through `mcpServers` and has tool search for large tool sets ([docs](https://code.claude.com/docs/en/agent-sdk/mcp)). |
| Claude Agent SDK | Py | `claude-agent-sdk` | 7,672,945 | 29,777,372 | 8,244 (anthropics/claude-agent-sdk-python) | 0.2.165 (2026-10-08) | MIT | `@tool(name, desc, {schema})` and `create_sdk_mcp_server` | **Yes** (same as TS) |
| Pydantic AI | Py | `pydantic-ai` (meta) | 1,382,669 | 5,383,780 | 20,531 (pydantic/pydantic-ai) | 2.55.0 (2026-10-09) | MIT | Plain function, with the signature and docstring turned into a Pydantic or JSON schema. `Toolset` abstraction. | **Yes.** MCP servers are toolsets (`MCPServerStdio` and others) ([docs](https://ai.pydantic.dev/mcp/client/)). |
| Pydantic AI (slim) | Py | `pydantic-ai-slim` | 6,391,947 | 23,908,854 | (same) | 2.55.0 (2026-10-09) | MIT | (same) | (same). The slim package is what libraries depend on, for example Crawlee's `PydanticAiCrawler`. |
| Mastra | TS | `@mastra/core` | 2,013,156 | 8,271,046 | 28,689 (mastra-ai/mastra) | 1.75.0 (2026-10-07) | Apache-2.0 on npm. The repo reports NOASSERTION because it mixes licenses, including an enterprise folder. | `createTool({ id, description, inputSchema: z.object })` ([docs](https://mastra.ai/docs/agents/using-tools)) | **Yes.** `MCPClient` in `@mastra/mcp`, and Mastra can also serve MCP ([docs](https://mastra.ai/docs/mcp/overview)). It also has a first-party **Browser** abstraction: `MastraBrowser`, `StagehandBrowser`, `AgentBrowser`, `FirecrawlBrowser` ([index](https://mastra.ai/llms.txt)). |
| CrewAI | Py | `crewai` | 627,314 | 2,442,764 | 59,532 (crewAIInc/crewAI) | 1.15.27 (2026-10-09) | MIT (repo) | `BaseTool` subclass with a Pydantic `args_schema`, or `@tool` | **Yes.** Agent `mcps=` field and `MCPServerAdapter` ([docs](https://docs.crewai.com/en/mcp/overview)). `crewai-tools` is versioned in lockstep (1.15.27). |
| LlamaIndex | Py | `llama-index-core` | 1,537,799 | 6,574,473 | 52,459 (run-llama/llama_index) | 0.14.25 (2026-09-21) | MIT | `FunctionTool.from_defaults(fn)` and `ToolSpec` classes | **Via package** `llama-index-tools-mcp` (`McpToolSpec`) ([LlamaHub](https://llamahub.ai/l/tools/llama-index-tools-mcp)) |
| LlamaIndex | TS | `llamaindex` | 134,619 | 584,271 | 3,082 (run-llama/LlamaIndexTS) | 0.12.1 (**2025-12-02**, so stale) | MIT | `tool({ parameters: zod })` | Partial (`@llamaindex/tools`). The repo has had **no push since 2026-03-11**. |
| Google ADK | Py | `google-adk` | 2,199,657 | 10,109,331 | 21,768 (google/adk-python) | 2.11.0 (2026-10-02) | Apache-2.0 | Plain function (signature and docstring), `FunctionTool`, toolsets | **Yes.** `McpToolset` ([docs](https://adk.dev/tools-custom/mcp-tools/)) |
| Google ADK | TS | `@google/adk` | 369,278 | 1,206,352 | 1,432 (google/adk-js) | 2.2.1 (2026-10-06) | Apache-2.0 | `FunctionTool` with a Zod schema | Yes (`MCPToolset`) |
| Microsoft Agent Framework | Py | `agent-framework` | 45,996 | 189,927 | 14,046 (microsoft/agent-framework) | 1.21.0 (2026-10-08) | MIT | Python function with `Annotated` types or `@ai_function`. .NET also exists, but NuGet was not measured. | **Yes.** MCP stdio, HTTP and hosted MCP tools ([docs](https://learn.microsoft.com/en-us/agent-framework/user-guide/model-context-protocol/)) |
| AutoGen (predecessor) | Py | `autogen-agentchat` | 91,496 | 379,923 | 61,342 (microsoft/autogen) | 0.7.5 (**2025-09-30**) | Code MIT, docs CC-BY-4.0 | `FunctionTool` | Yes (`McpWorkbench`). It is effectively frozen in favour of Agent Framework: last push 2026-04-15. |
| Semantic Kernel | Py | `semantic-kernel` | 70,782 | 300,767 | 28,639 (microsoft/semantic-kernel) | 1.45.0 (2026-10-06) | MIT | `@kernel_function` plugins | Yes (MCP plugins). Most of its usage is .NET, which this table does not count. |
| smolagents | Py | `smolagents` | 121,302 | 425,605 | 29,770 (huggingface/smolagents) | 1.26.0 (**2026-05-29**) | Apache-2.0 | `@tool` (type hints and docstring) or `Tool` class. **`CodeAgent` writes Python that calls tools.** | **Yes.** `MCPClient` and `ToolCollection.from_mcp` ([docs](https://huggingface.co/docs/smolagents/tutorials/tools)) |
| Agno | Py | `agno` | 490,504 | 1,779,568 | 42,651 (agno-agi/agno) | 3.1.2 (2026-10-08) | Apache-2.0 | `Toolkit` classes, plain functions, `@tool`. **Ships a large first-party toolkit directory** that includes browser and scraping vendors (see §6.2). | **Yes.** `MCPTools` ([docs](https://docs.agno.com/tools/mcp/overview)) |
| DSPy | Py | `dspy` | 1,192,443 | 5,312,050 | 38,582 (stanfordnlp/dspy) | 3.4.0 (2026-09-25) | MIT | `dspy.Tool(fn)` | **Via conversion.** `dspy.Tool.from_mcp_tool` ([docs](https://dspy.ai/tutorials/mcp/)) |
| *Strands Agents (added)* | Py | `strands-agents` | 8,752,876 | 37,112,498 | 8,754 (strands-agents/sdk-python) | 1.59.0 (2026-10-08) | Apache-2.0 | `@tool` decorator | Yes (`MCPClient`). It is the integration target for Nova Act and Bright Data, and `strands-agents-tools` ships `bright_data.py` and a `browser/` tool. |
| *Strands Agents (added)* | TS | `@strands-agents/sdk` | 534,780 | 2,336,918 | (sdk-typescript, not read) | 1.20.0 (2026-10-08) | Apache-2.0 | Zod | Yes |

#### 1b. Reading the numbers and their caveats

- **MCP is the baseline every framework consumes.** All 15 frameworks above can consume MCP servers. Thirteen do it in the core package. The other two use a first-party add-on: LlamaIndex through `llama-index-tools-mcp`, and DSPy through a converter. The MCP SDKs out-download every framework: `@modelcontextprotocol/sdk` has about 65M/wk and `mcp` about 61M/wk. Much of that is transitive, pulled in by agent hosts, IDE plugins and the Claude Agent SDK itself. Even so, it means that **an MCP server reaches every framework in this table without a dedicated adapter.**
- **CI and transitive downloads inflate counts.** Examples:
  - `langchain-core` (34.7M/wk on PyPI) is pulled in by `langchain`, `langgraph` and every `langchain-*` partner package.
  - `@langchain/core` is 2x `langchain` on npm.
  - `pydantic-ai-slim` (6.4M) is 4.6x the `pydantic-ai` meta-package, because libraries depend on slim.
  - `ai` (29.8M/wk) is a dependency of many SDKs and starters, so it measures ecosystem gravity, not unique apps.
  - `@anthropic-ai/claude-agent-sdk` (12M/wk) is inflated by the Claude Code ecosystem and CI runners.
- **Stars and downloads disagree.** AutoGen has 61K stars but only 91K/wk and is frozen. LlamaIndex TS is stale. CrewAI has 59K stars but only 0.63M/wk. Strands has 8.8K stars but 8.75M/wk, probably because of AWS default usage. Use downloads for reach and recency of release for health.
- **A practical ranking by reach for a TS plus Python engine:**
  - **TS:** AI SDK, then Claude Agent SDK, then LangChain/LangGraph, then OpenAI Agents, then Mastra.
  - **Python:** LangChain/LangGraph, then Strands, then Claude Agent SDK, then Pydantic AI, then OpenAI Agents, then Google ADK, then LlamaIndex, then DSPy, then CrewAI, then Agno.
- **Tool definition styles reduce to one shape.** Every framework accepts "name + description + JSON Schema + async handler". In TS that is Zod, Standard Schema or JSON Schema. In Python it is a Pydantic model or a signature turned into JSON Schema. **One catalog, emitted as JSON Schema, can be wrapped mechanically for each framework.** The adapter cost is a thin per-framework shim, not a re-modelling.
- **MCP itself changed.** Spec 2026-07-28 removes protocol sessions (`Mcp-Session-Id`) and the `initialize` handshake, and moves tasks into an extension ([changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)).

### 6.2 Adapter matrix (browser and scraping tools against agent frameworks)

**Legend**
- **Y**: official. Shipped by the vendor or documented on the vendor's own docs with a guide.
- **Y(M)**: the official guide wires the vendor's **MCP server** into that framework, with no native tool package. This was judged from the guide's text; there is no separate adapter.
- **C**: community or framework-maintained. Listed in the framework's own directory, or maintained by the framework team rather than the vendor.
- **—**: not found. This does not prove absence.

Sources:
- **Vendor `llms.txt` doc indexes**, fetched 2026-10-10.
- **Package registries.**
- **Framework directories:**
  - LangChain tool and loader index pages (`docs.langchain.com/oss/{python,javascript}/integrations/{tools,document_loaders}`).
  - The LlamaIndex repo tree, at `llama-index-integrations/tools/*` and `readers/llama-index-readers-web`.
  - The CrewAI repo tree, at `crewai_tools/tools/*`.
  - Agno `libs/agno/agno/tools/*`.
  - Mastra `llms.txt`.
  - The AI SDK tools registry (`ai-sdk.dev/tools-registry`).
  - The ADK integrations catalog (`adk.dev/integrations/`).
  - `strands-agents/tools`.

| Tool | MCP server | Vercel AI SDK | OpenAI Agents SDK | LangChain / LangGraph | LlamaIndex | CrewAI | Pydantic AI | Mastra | Claude Agent SDK / Claude skills | Google ADK | Agno / smolagents / other |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **Stagehand** (Browserbase) | Y, through the Browserbase MCP server ([docs](https://docs.stagehand.dev/v3/integrations/mcp/introduction)) | Y(M) ([v4 frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)) | — | Y for JS ([v3 LangChain JS](https://docs.stagehand.dev/v3/integrations/langchain/introduction)) and Deep Agents ([v4](https://docs.stagehand.dev/v4/integrations/agent-frameworks/deep-agents)). C for Python ([LangChain page](https://docs.langchain.com/oss/python/integrations/tools/stagehand)). | — | Y(M) ([v4](https://docs.stagehand.dev/v4/integrations/agent-frameworks/crewai)) and C (`crewai_tools/tools/stagehand_tool`) | — | Y(M) ([v4](https://docs.stagehand.dev/v4/integrations/agent-frameworks/mastra)) and C (`@mastra/stagehand` 0.3.5, 69,502/wk, Mastra-maintained) | Y: Claude Toolset ([doc](https://docs.stagehand.dev/v4/integrations/agent-frameworks/claude-cua-toolset-quickstart)) and Claude Code / Codex / Cursor CLI agents | — | Eve, Convex |
| **Browser Use** | Y ([OSS](https://docs.browser-use.com/open-source/customize/integrations/mcp-server), [Cloud](https://docs.browser-use.com/cloud/guides/mcp-server)) | Y ([Vercel tutorial](https://docs.browser-use.com/cloud/tutorials/integrations/vercel)); SDK `browser-use-sdk`, 98,236/wk | — | — | — | — | — | — | Y: "toolsets for Claude" ([doc](https://docs.browser-use.com/open-source/customize/integrations/toolsets-for-claude)), Claude Code, Claude Managed Agents, skills | — | n8n, OpenClaw, Hermes |
| **Skyvern** | Y ([doc](https://www.skyvern.com/docs/developers/getting-started/mcp)) | — | — | Y (PyPI `skyvern-langchain` 0.2.1, Skyvern author) | Y (PyPI `skyvern-llamaindex` 0.2.2, Skyvern author) | — | — | — | Y: CLI and skills ([doc](https://www.skyvern.com/docs/integrations/cli)) | — | Zapier, Make, n8n, Workato |
| **Playwright MCP** | Y. It *is* one (`@playwright/mcp`, 6,562,334/wk). Its README now steers coding agents to [playwright-cli plus skills](https://github.com/microsoft/playwright-cli). | through MCP only | through MCP only | C: `PlayWrightBrowserToolkit` in langchain-community (the library, not MCP) | C (`llama-index-tools-playwright`) | C (`selenium_scraping_tool` is the only DOM driver) | — | — | Y: CLI plus skills (Microsoft) | C: ADK's own Gemini **Computer Use** toolset drives Playwright ([catalog](https://adk.dev/integrations/computer-use/)) | — |
| **agent-browser** (Vercel Labs) | — (CLI plus skills, consumes **WebMCP** page tools; the README hints at an MCP text mode but has no server setup docs) | (Vercel-native CLI) | — | — | — | — | — | C: `@mastra/agent-browser` 0.5.3, 42,878/wk | Y: skills ([skills.sh](https://skills.sh/vercel-labs/agent-browser)) | — | Browserbase, Kernel and Browserless provide cloud-browser backends for it |
| **Midscene.js** | **Retired.** The last MCP packages are 1.9.8, and the docs say "no longer ships MCP servers" ([doc](https://midscenejs.com/mcp.md)). | — | — | — | — | — | — | — | Y: skills ([doc](https://midscenejs.com/skills.md)) | — | Playwright and Puppeteer integration |
| **Magnitude** | Y (npm `magnitude-mcp` 0.1.3, magnitudedev repo) | — | — | — | — | — | — | — | — | — | runs on Steel ([Steel doc](https://docs.steel.dev/integrations/magnitude)) |
| **Notte** | Y ([doc](https://docs.notte.cc/mcp-server.md)) | Y(M) ([doc](https://docs.notte.cc/integrations/vercel-ai-sdk.md)) | — (OpenAI CUA guide only) | Y ([doc](https://docs.notte.cc/integrations/langchain.md)) and C, listed in LangChain JS as `NotteBrowserToolkit` | — | Y ([doc](https://docs.notte.cc/integrations/crewai.md)) | — | Y(M) ([doc](https://docs.notte.cc/integrations/mastra.md)) | Y: Claude Code, Claude Managed Agents | Y(M) ([doc](https://docs.notte.cc/integrations/google-adk.md)) | Agno Y(M) ([doc](https://docs.notte.cc/integrations/agno.md)), n8n, OpenClaw |
| **AgentQL** (TinyFish) | Y ([doc](https://docs.agentql.com/integrations/mcp)) | — | — | Y (`langchain-agentql` 1.0.1, tinyfish-io; listed in LangChain tools and loaders) | Y (`llama-index-tools-agentql` 1.4.0, TinyFish author; LlamaHub) | — | — | — | — | Y(M) ([doc](https://docs.agentql.com/integrations/adk)) | Agno C (`agno/tools/agentql.py`); Dify, Langflow, Zapier |
| **Firecrawl** | Y (`firecrawl-mcp`, 117,089/wk; [doc](https://docs.firecrawl.dev/mcp-server.md)) | Y (`firecrawl-aisdk`; [doc](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/vercel-ai-sdk.md); in the AI SDK tools registry) | — (raw OpenAI SDK guide only) | Y ([LangChain](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/langchain.md), [LangGraph](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/langgraph.md)); listed in LangChain | Y ([doc](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/llamaindex.md)) and C (readers-web `firecrawl_web`) | C (three tools in `crewai_tools`) | — | Y ([doc](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/mastra.md)) and C (Mastra `tools/firecrawl` and `FirecrawlBrowser`) | Y: Claude Code quickstart and Anthropic guide | Y(M) ([doc](https://docs.firecrawl.dev/developer-guides/llm-sdks-and-frameworks/google-adk.md)) | Agno C (`firecrawl.py`); AutoGen Y |
| **Crawl4AI** | Y (MCP endpoint in its Docker server, per [docs](https://docs.crawl4ai.com/)) | — | — | — | — | — | — | — | Y: downloadable skill (`crawl4ai-skill.zip`) | — | Agno C (`crawl4ai.py`) |
| **ScrapeGraphAI** | Y ([doc](https://docs.scrapegraphai.com/services/mcp-server/introduction.md)) | Y (`@scrapegraph-ai/ai-sdk`; [doc](https://docs.scrapegraphai.com/integrations/vercel_ai.md)) | — (raw OpenAI guide) | Y (`langchain-scrapegraph` 1.6.0; [LangChain](https://docs.scrapegraphai.com/integrations/langchain.md), [LangGraph](https://docs.scrapegraphai.com/integrations/langgraph.md)) | Y ([doc](https://docs.scrapegraphai.com/integrations/llamaindex.md)) | Y ([doc](https://docs.scrapegraphai.com/integrations/crewai.md)) and C (`scrapegraph_scrape_tool`) | — | — | Y: agent skill and Claude Code MCP | — | Agno Y ([doc](https://docs.scrapegraphai.com/integrations/agno.md)); LiteLLM, n8n, Zapier |
| **Spider** | Y (hosted `mcp.spider.cloud`) | — | — | Y ([doc](https://spider.cloud/docs/integrations/langchain)) | Y ([doc](https://spider.cloud/docs/integrations/llamaindex)) and C (readers-web `spider_web`) | Y ([doc](https://spider.cloud/docs/integrations/crewai)) and C (`spider_tool`) | — | — | Y: agent `SKILL.md` | — | Agno Y ([doc](https://spider.cloud/docs/integrations/agno)); Rust `spider-agent` CLI with a JSON schema contract |
| **Crawlee / Apify** | Y (`@apify/actors-mcp-server`, 36,668/wk; [doc](https://docs.apify.com/mcp.md)) | Y(M) ([doc](https://docs.apify.com/integrations/vercel-ai-sdk.md)) | Y(M) ([doc](https://docs.apify.com/integrations/openai-agents.md)) | Y (`langchain-apify`; [doc](https://docs.apify.com/integrations/langchain.md), [LangGraph](https://docs.apify.com/integrations/langgraph.md)) | Y ([doc](https://docs.apify.com/integrations/llama-index.md)) | Y ([doc](https://docs.apify.com/integrations/crewai.md)) and C (`apify_actors_tool`) | — (Crawlee Python *uses* Pydantic AI internally via `PydanticAiCrawler`) | Y(M) ([doc](https://docs.apify.com/integrations/mastra.md)) | Y: Claude, Claude Code, agent skills ([doc](https://docs.apify.com/agents/skills.md)) | Y(M) ([doc](https://docs.apify.com/integrations/google-adk.md)) | Agno Y, Strands Y, Haystack Y, Dify, Flowise |
| **Steel** | Y ([cookbook](https://docs.steel.dev/cookbook/mcp)) | Y ([doc](https://docs.steel.dev/integrations/ai-sdk)) | Y ([doc](https://docs.steel.dev/integrations/openai-agents-sdk)) | Y, LangGraph ([doc](https://docs.steel.dev/integrations/langgraph)) | — | Y ([doc](https://docs.steel.dev/integrations/crewai)) | **Y** ([doc](https://docs.steel.dev/integrations/pydantic-ai)) | Y ([doc](https://docs.steel.dev/integrations/mastra)) | Y: Claude Agent SDK ([doc](https://docs.steel.dev/integrations/claude-agent-sdk)), Claude Code, skills | Y ([cookbook](https://docs.steel.dev/cookbook/google-adk)) | Agno Y; LangChainGo; hosts Browser Use, Magnitude, Notte |
| **Hyperbrowser** | Y ([doc](https://hyperbrowser.ai/docs/integrations/model-context-protocol.md)) | — | — (raw function-calling guide) | Y (`langchain-hyperbrowser` 0.4.1; listed in LangChain tools and loaders) | Y ([doc](https://hyperbrowser.ai/docs/integrations/llamaindex.md)) and C (readers-web `hyperbrowser_web`) | C (`hyperbrowser_load_tool`) | — | — | Y: hosted Claude computer-use API | — | Stagehand |
| **Anchor Browser** | Y, hosted and OSS ([doc](https://docs.anchorbrowser.io/advanced/mcp.md)) | — | — | Y (`langchain-anchorbrowser` 0.2.0; [doc](https://docs.anchorbrowser.io/agent-frameworks/langchain.md)) | — | Y ([doc](https://docs.anchorbrowser.io/agent-frameworks/crewai.md)) | — | — | Y: Anthropic computer use | — | "Custom agent framework" guide |
| **Kernel** | Y (tools include `execute_playwright_code` and `browser_repl`; [doc](https://kernel.sh/docs/reference/mcp-server.md)) | Y (`@onkernel/ai-sdk`; [doc](https://kernel.sh/docs/integrations/vercel/ai-sdk.md)) | Y(M) ([doc](https://kernel.sh/docs/integrations/openai/agents-sdk.md)) | — | — | — | — | Y, cookbook ([doc](https://kernel.sh/docs/cookbooks/mastra-web-task-assistant.md)) | Y: Claude Agent SDK ([doc](https://kernel.sh/docs/integrations/claude/claude-agent-sdk.md)), Claude Managed Agents, skills | — | "Code Mode with Browser REPL and WebMCP" ([doc](https://kernel.sh/docs/browsers/code-mode-webmcp.md)) |
| **Browserbase** | Y ([doc](https://docs.browserbase.com/integrations/mcp/introduction.md)) | Y (`@browserbasehq/ai-sdk`; [doc](https://docs.browserbase.com/integrations/vercel/introduction.md); in the AI SDK tools registry) | — (OpenAI CUA guide only) | Y ([doc](https://docs.browserbase.com/integrations/langchain/introduction.md), [Deep Agents](https://docs.browserbase.com/integrations/langchain/deepagents.md)) | C (readers-web `browserbase_web`) | Y ([doc](https://docs.browserbase.com/integrations/crew-ai/python.md)) and C (`browserbase_load_tool`) | — | Y ([doc](https://docs.browserbase.com/integrations/mastra/introduction.md)) | Y: Anthropic Managed Agents ([doc](https://docs.browserbase.com/integrations/anthropic/managed-agents/introduction.md)), skills | Y(M) ([doc](https://docs.browserbase.com/integrations/google-adk/introduction.md)) | Agno Y ([doc](https://docs.browserbase.com/integrations/agno/introduction.md)) and C (`browserbase.py`); AgentKit, Braintrust |
| **Browserless** | Y ([doc](https://docs.browserless.io/mcp/overview.md)) | Y ([doc](https://docs.browserless.io/ai-integrations/vercel-ai-sdk.md)) | — (raw OpenAI SDK page) | Y ([doc](https://docs.browserless.io/ai-integrations/langchain.md)) and C (listed in LangChain Py and JS) | — | — | — | — | Y: Claude Agent SDK ([doc](https://docs.browserless.io/ai-integrations/claude-agent-sdk.md)), Claude Code plugin | — | AgentKit, n8n, Zapier, Make |
| **Amazon Nova Act** | — official; C (PyPI `nova-act-mcp`, individual author). Nova Act *consumes* remote MCP. | — | — | — (LangChain lists the AWS **AgentCore Browser** toolkit instead) | — | — | — | — | — | — | **Strands Agents Y** ([README](https://github.com/aws/nova-act)); Python `@tool` |
| **Bright Data** | Y (`@brightdata/mcp`; [doc](https://docs.brightdata.com/products/mcp-server/overview.md)) | Y (`@brightdata/ai-sdk`, 47/wk; [doc](https://docs.brightdata.com/integrations/vercel-ai-sdk)) | — | Y (`langchain-brightdata` 0.2.0; [doc](https://docs.brightdata.com/integrations/langchain)) | Y (`llama-index-tools-brightdata` 0.4.0; [doc](https://docs.brightdata.com/integrations/llama-index)) | Y ([doc](https://docs.brightdata.com/integrations/crew-ai)) and C (`brightdata_tool`) | — | Y ([doc](https://docs.brightdata.com/integrations/mastra)) and C (Mastra `tools/brightdata`) | Y: agent skills (`brightdata/skills`) and Claude Code MCP | — | Agno Y ([doc](https://docs.brightdata.com/integrations/agno)); Strands Y ([doc](https://docs.brightdata.com/integrations/strands)) and C (`strands_tools/bright_data.py`); Haystack |

#### 2a. What the matrix says

1. **MCP is universal.** 20 of 22 tools ship an MCP server. Midscene **retired** its server in favour of skills plus CLI. agent-browser is CLI plus skills first. Microsoft's Playwright MCP README now recommends CLI plus skills for coding agents.
2. **Claude skills and CLI are the new second channel.** 18 of 22 have a Claude-side entry: skills, a Claude Code guide, the Agent SDK or Managed Agents. The newest movers ship *skills plus CLI* rather than per-framework packages: Midscene, Playwright, agent-browser, Spider, Bright Data and Skyvern.
3. **LangChain/LangGraph is the most covered framework,** with 15 of 22 tools. It is followed by CrewAI (13, many inside `crewai-tools`), the Vercel AI SDK (12), Mastra (11), Agno (11) and LlamaIndex (10).
4. **Several frameworks are rarely targeted.** **OpenAI Agents SDK** (3: Apify, Steel, Kernel), **Google ADK** (7, mostly MCP wiring) and **Pydantic AI** (1: Steel) are rarely targeted despite high downloads. Vendors rely on those frameworks' native MCP clients.
5. **Many "integrations" are only MCP wiring guides (Y(M)).** Apify, Notte, AgentQL, Kernel and Firecrawl on ADK all fall here. Native per-framework packages exist mostly for LangChain (`langchain-*`), LlamaIndex (`llama-index-tools-*`) and the AI SDK (`firecrawl-aisdk`, `@browserbasehq/ai-sdk`, `@onkernel/ai-sdk`, `@scrapegraph-ai/ai-sdk`, `@brightdata/ai-sdk`). The AI SDK wrappers have tiny downloads, from 47 to 10,635 per week. Compare `firecrawl-mcp` at 117K/wk and `@playwright/mcp` at 6.6M/wk. **Users pick MCP or SDK over framework wrappers.**
6. **Frameworks increasingly own the browser adapter themselves.** Mastra ships `@mastra/stagehand` and `@mastra/agent-browser` plus a `MastraBrowser` abstraction. Agno ships `browserbase`, `crawl4ai`, `spider`, `brightdata` and `agentql` toolkits. CrewAI bundles about a dozen scraping tools. ADK has a Gemini Computer Use plus Playwright toolset. For an engine, a good MCP server plus a clean SDK makes it easy for framework teams to add Automan themselves.

### 6.3 Cross-cutting pains of building browser tools for agents

1. **Tool-schema token bloat.** Tool definitions load into context before the first turn.
   - Anthropic measured "58 tools consuming approximately 55K tokens", and has seen "134K tokens before optimization". Its Tool Search Tool cuts this about 85% ([Advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use)).
   - Playwright MCP has an open request for server-side `--include-tools` / `--exclude-tools` ([#1772](https://github.com/microsoft/playwright-mcp/issues/1772)).
   - OpenAI Agents users asked to filter MCPServerStdio tools ([#851](https://github.com/openai/openai-agents-python/issues/851)). Duplicate tool names across servers break runs ([#464](https://github.com/openai/openai-agents-python/issues/464)).
   - **For Automan: catalog profiles must trim *schemas*, not only hide calls, and names must be namespaced.**
2. **Too many tools versus code mode.**
   - Cloudflare: "LLMs are better at writing code to call MCP, than at calling MCP directly" ([Code Mode](https://blog.cloudflare.com/code-mode/)).
   - Anthropic: code execution over MCP cut one workflow "from 150,000 tokens to 2,000 tokens" ([Code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp)).
   - Vercel removed 80% of an agent's tools and got "5x faster, 37% fewer tokens" ([blog](https://vercel.com/blog/we-removed-80-percent-of-our-agents-tools)).
   - Browser vendors converge on this. Stagehand v4's framework contract is just `run` (JS against a Playwright-shaped `page`), `snapshot` and `screenshot` ([doc](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)). Kernel's MCP exposes `execute_playwright_code` and `browser_repl` ([doc](https://kernel.sh/docs/reference/mcp-server.md)).
   - This supports #72 Q6, code mode in a sandbox.
3. **Per-action observation size: snapshots and screenshots.**
   - Playwright MCP returns the page state after each action, and users hit context compaction quickly ([#1274](https://github.com/microsoft/playwright-mcp/issues/1274)).
   - Another issue asks for a delta snapshot mode and reports about 65% token savings ([#1654](https://github.com/microsoft/playwright-mcp/issues/1654)).
   - Screenshots stay base64 in the response even when saved to a file ([#1581](https://github.com/microsoft/playwright-mcp/issues/1581)).
   - Image cost scales with pixels, roughly w×h/750 tokens per Claude's vision docs ([doc](https://platform.claude.com/docs/en/build-with-claude/vision)).
   - agent-browser added `screenshot --if-changed` "to save tokens", and caps WebMCP summaries at 16 tools / 4 KiB ([README](https://github.com/vercel-labs/agent-browser)).
   - The Playwright README itself says CLI plus skills "avoid loading large tool schemas and verbose accessibility trees" ([README](https://github.com/microsoft/playwright-mcp)).
   - **For Automan: diff or delta observations, opt-in screenshots and file references instead of inline blobs.**
4. **Long-running calls and timeouts.**
   - Hosts impose hard ceilings. Claude Code ignored `MCP_TIMEOUT` above 60 s ([#16837](https://github.com/anthropics/claude-code/issues/16837), open). Claude Desktop has a hard-coded 60 s initialize timeout ([#84136](https://github.com/anthropics/claude-code/issues/84136)).
   - In the OpenAI Agents SDK, users repeatedly ask how to raise the MCP tool timeout ([#1892](https://github.com/openai/openai-agents-python/issues/1892)), and `client_session_timeout_seconds=0` silently means no timeout ([#4017](https://github.com/openai/openai-agents-python/issues/4017)).
   - MCP's answer is **tasks**. Spec 2026-07-28 moved them into an official extension with polling (`tasks/get`) and server-initiated handles ([changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [tasks](https://modelcontextprotocol.io/specification/2026-07-28/basic/utilities/tasks)).
   - **For Automan: crawls and agent runs must return a handle quickly, then support poll and stream. Never block a tool call for minutes.**
5. **Streaming progress.** Framework tool interfaces are request/response. Progress only flows through MCP notifications or task updates, and few framework adapters surface them. The AI SDK MCP client is "lightweight", meant for tool conversion only ([doc](https://ai-sdk.dev/docs/ai-sdk-core/mcp-tools)). Recommendation: progress goes to the event or journal stream, and tool results carry summaries.
6. **Session lifecycle across tool calls.**
   - Browser state is inherently stateful, but the plumbing is going stateless. MCP 2026-07-28 **removed protocol-level sessions and `Mcp-Session-Id`**.
   - LangChain's old adapter opened "a new MCP `ClientSession` for each tool invocation" by default ([README](https://github.com/langchain-ai/langchain-mcp-adapters)). Stateful backends needed workarounds ([#362](https://github.com/langchain-ai/langchain-mcp-adapters/issues/362)).
   - LangChain's new built-in MCP embraces the stateless core ([blog](https://www.langchain.com/blog/mcp-in-langchain-stateless-protocol-elicitation-and-more)).
   - Stagehand notes that snapshot element IDs "are valid only for the latest snapshot".
   - **For Automan: browser sessions must be explicit, addressable resources (session id as a tool argument, with TTL and explicit close) that survive transport reconnects. They must not hang off the MCP connection.**
7. **Server startup and connection fragility.** Cold-start `npx`/`uvx` servers exceed host init timeouts ([#84136](https://github.com/anthropics/claude-code/issues/84136)). Concurrent sessions cause connect storms ([#83495](https://github.com/anthropics/claude-code/issues/83495)). A single prebuilt Rust binary sidecar has an advantage here.
8. **Secrets leak into tool output.** Playwright MCP snapshots serialized password values ([#1566](https://github.com/microsoft/playwright-mcp/issues/1566)). Users want session tokens redacted from tab URLs in responses ([#1791](https://github.com/microsoft/playwright-mcp/issues/1791)). Observation output needs redaction by design.

#### Implications for the "which adapters" decision in #72 Q5 (researcher's synthesis, not a decision)

- **Tier 0, always:** the MCP server, built for MCP 2026-07-28 (stateless, tasks extension), with profiles that trim schemas. It reaches all 15 frameworks above.
- **Tier 0 too:** agent skills plus the `--json` CLI. This is the channel coding agents now prefer, and Playwright, Midscene and agent-browser all moved there.
- **Tier 1, thin generated shims** from the catalog's JSON Schema. In TS: Vercel AI SDK (largest TS reach), plus Mastra and LangChain.js. In Python: LangChain/LangGraph (largest Python reach and most vendor coverage). Claude Agent SDK needs nothing beyond MCP, because its custom tools are in-process MCP.
- **Tier 2, by MCP only plus a docs page:** OpenAI Agents SDK, Pydantic AI, Google ADK, CrewAI, LlamaIndex, Agno, Strands, smolagents and DSPy. All have native MCP clients, and vendors rarely ship more for them.

## 7. User pains across tools

Themes are grouped from the per-tool pain lists above. Counts are GitHub reactions on the cited issue, or keyword issue counts where stated (coarse: title or body match, not deduplicated). Reddit was mostly unreachable in this pass (see [§10](#10-method-and-sources)), so community signal comes from GitHub, Hacker News and vendor forums.

| # | Theme | Representative evidence |
|---:|---|---|
| 1 | **False success / hallucinated completion** | Browser Use known-answer regressions [#6031](https://github.com/browser-use/browser-use/issues/6031), "element not found" reported as success [#5438](https://github.com/browser-use/browser-use/issues/5438), MCP errors with `isError=false` [#5252](https://github.com/browser-use/browser-use/issues/5252); Stagehand "Completed" before the list was extracted [#239](https://github.com/browserbase/stagehand/issues/239); agent-browser click "✓ Done" that missed [#2083](https://github.com/vercel-labs/agent-browser/issues/2083); browser-harness "FAKE self healing" [#145](https://github.com/browser-use/browser-harness/issues/145) (13); Online-Mind2Web 30.0% vs 89.1% self-reported ([paper](https://arxiv.org/abs/2504.01382)); Skyvern added a validator after seeing false successes ([blog](https://www.skyvern.com/blog/skyvern-2-0-state-of-the-art-web-navigation-with-85-8-on-webvoyager-eval/)) |
| 2 | **Cost, tokens and speed** | Browser Use keyword count `tokens` 182; "Browser use is a token hog" ([HN](https://news.ycombinator.com/item?id=47906391)); Playwright MCP snapshot too long [#395](https://github.com/microsoft/playwright-mcp/issues/395) (14), output size [#889](https://github.com/microsoft/playwright-mcp/issues/889) (13), token limit [#806](https://github.com/microsoft/playwright-mcp/issues/806) (9); "Computer Use is 45x more expensive than structured APIs" (495 points, [HN](https://news.ycombinator.com/item?id=48024859)); Stagehand "horrendously expensive per run?" ([HN](https://news.ycombinator.com/item?id=42642063)); Magnitude vision testing cost ([HN](https://news.ycombinator.com/item?id=44391985)); llm-scraper "scary openai bills" ([HN](https://news.ycombinator.com/item?id=40104208)) |
| 3 | **"Let me rerun it without the model" / export a script** | Stagehand "Return plan for extraction" [#389](https://github.com/browserbase/stagehand/issues/389) (16), XPath caching [#404](https://github.com/browserbase/stagehand/issues/404), rerun without OpenAI calls [#255](https://github.com/browserbase/stagehand/issues/255); Browser Use rerun faster [#763](https://github.com/browser-use/browser-use/issues/763) (11), export a script [#943](https://github.com/browser-use/browser-use/issues/943) (11); Midscene export to Playwright [#2240](https://github.com/web-infra-dev/midscene/issues/2240); Magnitude caching [#132](https://github.com/magnitudedev/browser-agent/issues/132); Nanobrowser replay [#147](https://github.com/nanobrowser/nanobrowser/issues/147); HyperAgent [#5](https://github.com/hyperbrowserai/HyperAgent/issues/5); ScrapeGraphAI launch thread asking for generated rules ([HN](https://news.ycombinator.com/item?id=40292021)) |
| 4 | **Local, open and alternative providers** | Stagehand keyword `local model` 55, Ollama [#183](https://github.com/browserbase/stagehand/issues/183) (19), AI SDK v3 models [#1645](https://github.com/browserbase/stagehand/issues/1645) (16); Browser Use keyword `ollama` 102, gpt-oss/Ollama [#2605](https://github.com/browser-use/browser-use/issues/2605) (19); llm-scraper Ollama [#4](https://github.com/mishushakov/llm-scraper/issues/4) (19); Bytebot OpenRouter [#101](https://github.com/bytebot-ai/bytebot/issues/101) (15); Nanobrowser Ollama [#16](https://github.com/nanobrowser/nanobrowser/issues/16) (7); Firecrawl Ollama extract [#1467](https://github.com/firecrawl/firecrawl/issues/1467) (5); Crawl4AI provider asks for OpenAI auth [#649](https://github.com/unclecode/crawl4ai/issues/649); Midscene Ollama + UI-TARS [#309](https://github.com/web-infra-dev/midscene/issues/309) (24 comments) |
| 5 | **Structured output breaks with weaker models** | Midscene invalid JSON [#2831](https://github.com/web-infra-dev/midscene/issues/2831), schema 400 [#339](https://github.com/web-infra-dev/midscene/issues/339); Crawl4AI [#979](https://github.com/unclecode/crawl4ai/issues/979) (22 comments), Markdown-wrapped JSON [#1663](https://github.com/unclecode/crawl4ai/issues/1663), empty output [#1202](https://github.com/unclecode/crawl4ai/issues/1202); Nanobrowser glm-4.7 [#291](https://github.com/nanobrowser/nanobrowser/issues/291); ScrapeGraphAI empty JSON [#893](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/893); Firecrawl empty data with Zod [#1836](https://github.com/firecrawl/firecrawl/issues/1836); a Browser Use maintainer on Llama and structured output ([HN](https://news.ycombinator.com/item?id=43181548)) |
| 6 | **Login, 2FA and profile reuse** | Browser Use keyword `login` 164, manual login/MFA [#404](https://github.com/browser-use/browser-use/issues/404) (11); agent-browser `--user-data-dir` [#42](https://github.com/vercel-labs/agent-browser/issues/42) (13), `--profile` [#207](https://github.com/vercel-labs/agent-browser/issues/207) (9); Playwright MCP "browser already in use" [#891](https://github.com/microsoft/playwright-mcp/issues/891) (11); Claude in Chrome profile targeting [#15125](https://github.com/anthropics/claude-code/issues/15125) (89); Skyvern Bitwarden errors [#2604](https://github.com/Skyvern-AI/skyvern/issues/2604); OpenAI CUA refuses login/signup [#39](https://github.com/openai/openai-cua-sample-app/issues/39) |
| 7 | **Browser lifecycle and flakiness** | Browser Use stuck Chromium [#839](https://github.com/browser-use/browser-use/issues/839) (14, 41 comments), blank page [#1020](https://github.com/browser-use/browser-use/issues/1020), Chrome 136 user-data-dir [#1520](https://github.com/browser-use/browser-use/issues/1520), no crash recovery [#5067](https://github.com/browser-use/browser-use/issues/5067); agent-browser orphaned Chrome [#1371](https://github.com/vercel-labs/agent-browser/issues/1371), browser silently swapped [#2079](https://github.com/vercel-labs/agent-browser/issues/2079); browser-harness daemon liveness [#865](https://github.com/browser-use/browser-harness/issues/865); Bytebot all tasks failed [#84](https://github.com/bytebot-ai/bytebot/issues/84) |
| 8 | **Security controls that leak or disclaim** | Domain policy bypass: Stagehand [#2945](https://github.com/browserbase/stagehand/issues/2945), fail-open default [#3002](https://github.com/browserbase/stagehand/issues/3002); Browser Use [#5099](https://github.com/browser-use/browser-use/issues/5099), [#5879](https://github.com/browser-use/browser-use/issues/5879); Skyvern [#8577](https://github.com/Skyvern-AI/skyvern/issues/8577). Secrets in output: Playwright MCP password values in snapshots [#1566](https://github.com/microsoft/playwright-mcp/issues/1566), URL tokens [#1791](https://github.com/microsoft/playwright-mcp/issues/1791), logs [#1789](https://github.com/microsoft/playwright-mcp/issues/1789); Stagehand cache receives variable values ([act](https://docs.stagehand.dev/v4/basics/act)). Code execution: LaVague prompt injection to RCE [#650](https://github.com/lavague-ai/LaVague/issues/650), Playwright MCP `run_code_unsafe` [#1784](https://github.com/microsoft/playwright-mcp/issues/1784), Skyvern CodeBlock escape [#6868](https://github.com/Skyvern-AI/skyvern/issues/6868), Crawl4AI server advisories ([README](https://github.com/unclecode/crawl4ai/blob/v0.9.4/README.md)). Supply chain: malicious LiteLLM pulled by an install script, Browser Use [#4505](https://github.com/browser-use/browser-use/issues/4505); Crawl4AI moved to a LiteLLM fork |
| 9 | **Cloud lock-in and open-source gaps** | Stagehand "disclose project properly as an onramp to Browserbase" [#492](https://github.com/browserbase/stagehand/issues/492); v4 cache and Model Gateway need Browserbase ([migration](https://docs.stagehand.dev/v4/migrations/v3)); self-hosted Firecrawl lacks agent, interact, actions and screenshots ([self-host](https://docs.firecrawl.dev/contributing/self-host.md)), proxy [#1129](https://github.com/firecrawl/firecrawl/issues/1129) (18); Spider prompt extraction only on cloud `/ai/*` ([llms.txt](https://spider.cloud/llms.txt)); AgentQL self-hosting [#108](https://github.com/tinyfish-io/agentql/issues/108) |
| 10 | **Upstream churn and breaking changes** | ScrapeGraphAI on LangChain [#926](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/926) (19), [#1078](https://github.com/ScrapeGraphAI/Scrapegraph-ai/issues/1078); Crawlee still on Stagehand v3 ([crawlee-python#2154](https://github.com/apify/crawlee-python/issues/2154)); Browser Use session classes [#1815](https://github.com/browser-use/browser-use/issues/1815); OpenAI `computer-use-preview` shut down 2026-07-23 ([feed](http://deprecations.info/v1/feed.xml)); Nanobrowser 0.2.0 deleted saved keys of removed providers ([release](https://github.com/nanobrowser/nanobrowser/releases/tag/v0.2.0)) |
| 11 | **Install, platform and packaging** | agent-browser browser revision [#107](https://github.com/vercel-labs/agent-browser/issues/107) (15), Windows shims [#148](https://github.com/vercel-labs/agent-browser/issues/148) (15); BrowserOS Linux [#2174](https://github.com/browseros-ai/BrowserOS/issues/2174) (23); Skyvern Docker [#312](https://github.com/Skyvern-AI/skyvern/issues/312); Playwright Test Agents Python [#38610](https://github.com/microsoft/playwright/issues/38610) (52); MCP servers started with `npx`/`uvx` exceed host init timeouts ([claude-code#84136](https://github.com/anthropics/claude-code/issues/84136)) |
| 12 | **Anti-bot, CAPTCHA and crawl etiquette** | Browser Use human-like browsing [#947](https://github.com/browser-use/browser-use/issues/947) (20), Capsolver [#652](https://github.com/browser-use/browser-use/issues/652); agent-browser stealth [#120](https://github.com/vercel-labs/agent-browser/issues/120) (15); Firecrawl anti-bot loop [#2350](https://github.com/firecrawl/firecrawl/issues/2350); Crawl4AI "Please respect robots.txt" [#107](https://github.com/unclecode/crawl4ai/issues/107) (31); BrowserOS robots.txt [#3](https://github.com/browseros-ai/BrowserOS/issues/3) |
| 13 | **Observability and usage reporting** | Stagehand `onLog` forces stderr [#3035](https://github.com/browserbase/stagehand/issues/3035) (9); Skyvern token usage for self-hosters [#6226](https://github.com/Skyvern-AI/skyvern/issues/6226); Claude quickstart token display [#116](https://github.com/anthropics/claude-quickstarts/issues/116); Agent-E cumulative tokens [#85](https://github.com/EmergenceAI/Agent-E/issues/85) |
| 14 | **Tool plumbing for agents** | Tool-schema bloat (Anthropic measured 58 tools at ~55K tokens, [post](https://www.anthropic.com/engineering/advanced-tool-use)); per-action snapshot size ([playwright-mcp#1274](https://github.com/microsoft/playwright-mcp/issues/1274), delta mode [#1654](https://github.com/microsoft/playwright-mcp/issues/1654)); host timeouts around 60 s ([claude-code#16837](https://github.com/anthropics/claude-code/issues/16837), [openai-agents-python#1892](https://github.com/openai/openai-agents-python/issues/1892)); stateful browser sessions over a now stateless MCP ([spec 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/changelog)) |
| 15 | **Abandonment risk** | Dormant or archived: LaVague, Agent-E, Laminar Index, Bytebot ("Is this project dead?" [#181](https://github.com/bytebot-ai/bytebot/issues/181)), Magnitude's browser agent, Shortest; Cua deprecated its agent packages; `mcp-server-browserbase` archived 2026-07-20 |

## 8. Praised API shapes and architecture choices

| Shape | Who has it | What users and reviewers say |
|---|---|---|
| **A few typed primitives** (`act` / `observe` / `extract`) mixed with plain code | Stagehand; Skyvern's `page.click(prompt=…)`, `page.validate`; Midscene `aiAct`/`aiQuery`/`aiAssert`; HyperAgent `perform`/`extract`; Nova Act `act`/`act_get` | "The act, extract, and observe methods are super clean." ([fbouvier](https://news.ycombinator.com/item?id=42644964)); "actually a useful abstraction here" ([sparab18](https://news.ycombinator.com/item?id=42638393)); Magnitude's founder argues for mixing agent steps with your own code ([anerli](https://news.ycombinator.com/item?id=44390404)) |
| **Accessibility snapshot with stable refs, no vision by default** | Playwright MCP, agent-browser, Stagehand | "a Playwright MCP available and works right out of the box" ([undefuser](https://news.ycombinator.com/item?id=49811652)); "playwright-cli and agent-browser are much much better" ([gverrilla](https://news.ycombinator.com/item?id=47511689)) |
| **CLI plus skills for coding agents**, MCP for the rest | Playwright CLI, agent-browser, Midscene, Libretto, browser-harness | "a great CLI for agent-based browser automation" ([esafak](https://news.ycombinator.com/item?id=47167062)); counterpoint: "MCPs are deterministic, SKILLS.md isn't." ([AftHurrahWinch](https://news.ycombinator.com/item?id=45644100)) |
| **Generate code or selectors once, replay deterministically** | Skyvern code cache, HyperAgent `actionCache`, Crawlee selector cache, Crawl4AI `generate_schema`, Libretto, Director export | "fundamentally does not work unless you generate code" ([KhoomeiK](https://news.ycombinator.com/item?id=40293091)); "it's always that 80%-99% automated slice where the frontier is" ([etwigg](https://news.ycombinator.com/item?id=47783042)); Browser Use's own "extract xPaths… re-run these scripts deterministically" pitch ([MagMueller](https://news.ycombinator.com/item?id=43181564)) |
| **Output is ordinary code the user owns** | Libretto (Playwright TS), Playwright `--codegen`, HyperAgent `createScriptFromActionCache`, Director (Stagehand script), llm-scraper `generate` | Users asked for "generating a reusable script with an LLM" ([jusgu](https://news.ycombinator.com/item?id=40103671)) |
| **Visible steps and auditable traces** | Skyvern workflow observability and public eval traces, Midscene HTML replay report, Laminar replay synced to spans | "I especially like the observability around the workflow and the steps" ([lyime](https://news.ycombinator.com/item?id=42742916)) |
| **Reuse of a real, logged-in profile** | browser-harness, BrowserOS, Playwright extension mode, Claude in Chrome | Profile reuse called the practical answer for LinkedIn-style logins ([HN](https://news.ycombinator.com/item?id=43175311)); Claude in Chrome profile targeting is an 89-reaction request ([#15125](https://github.com/anthropics/claude-code/issues/15125)) |
| **Open source with an exit** | Skyvern (AGPL), Steel (Apache runtime), Firecrawl forks | "super cool that you've open sourced it under the AGPL" ([mmaunder](https://news.ycombinator.com/item?id=41937339)); "AGPL means you can just fork and remove everything you don't like" ([hardwaresofton](https://news.ycombinator.com/item?id=42056970)) |
| **Safety controls as product features** (documented, not praised in threads yet) | agent-browser vault ("The LLM never sees passwords"), content boundaries, action policies; Cua's one-use `capture_id` for pixel clicks; Anchor drafts a human publishes; Firecrawl injection classifier; Gemini safety categories | Vendor documentation only; no independent praise found |

## 9. Synthesis for Automan

Facts and implications for the decisions in [#72](https://github.com/Jbmanllr/automan/issues/72), not decisions.

### 9.1 Cross-cutting pains Automan's design must answer

1. **Proof of success.** No surveyed tool proves business success on replay; most rely on an LLM judge (Browser Use `use_judge=True`, Skyvern's validator) or the user's assertions. The lab's whole-plan fingerprint check and effect journal ([rnd/README.md §4.15–4.16](../README.md)) target exactly this gap.
2. **Spend.** Users want a cost ceiling and a cheap rerun; tools give step caps and token reports.
3. **Provider breadth without fragility.** Local and OpenAI-compatible models are the most requested feature in almost every tracker, and those models are the ones that break structured output.
4. **Logins.** Profiles, vaults, TOTP and human handoff are each solved somewhere, but not together in an open-source engine.
5. **Safety boundaries that hold.** Allowlists must cover every network path (workers, WebSockets, WebRTC, `javascript:` URLs); secrets must stay out of prompts, caches, snapshots and logs.
6. **Lock-in.** The open-source/cloud boundary is a recurring complaint (Stagehand cache, Firecrawl self-host, Spider `/ai/*`).

### 9.2 Design patterns that work, with their source

- **Primitives over a built-in loop, with the agent outside**: the high-volume tools (Playwright MCP, agent-browser, Stagehand v4) do this; Stagehand v4 recommends code mode or tool calling instead of `agent()`.
- **Code mode next to the browser**: OpenAI recommends code execution for its flagship computer-use model; Stagehand's agent contract is `run`/`snapshot`/`screenshot`; Kernel exposes `execute_playwright_code` and `browser_repl`; Anthropic and Cloudflare report large token cuts (150,000 → 2,000 tokens in [one workflow](https://www.anthropic.com/engineering/code-execution-with-mcp)).
- **Compile, replay, fall back per step, regenerate**: Skyvern (per block, progressive across branches), HyperAgent, Lightfeed, Midscene (no write-back of a partial plan after fallback), Crawlee (per route label).
- **Verify before caching or promoting**: Momentic re-caches only if the rest of the test passes; Anchor's repair is a draft a human publishes; Playwright's healer is gated by the test's assertions.
- **Compile to network calls when possible**: Libretto and Imprint turn recorded UI flows into API calls, with a DOM fallback ladder.
- **Small observations**: diffs and `--if-changed` screenshots (agent-browser), HTML diff mode (Spider, self-reported 50–70% token cut), delta snapshots (Playwright MCP request, ~65% claimed).
- **Secrets typed by the engine, never by the model**: agent-browser vault with a field-swap check, Steel field blur, Kernel Managed Auth with reauth, Skyvern vault adapters and 2FA push/pull, Nova Act's "type secrets via Playwright" rule.
- **Human pause as a run state**: Browser Use Cloud live view then follow-up in the same session; Browserless `liveURL` with a `captchaFound` event; Nova Act `approve()` and `ui_takeover()`, with human-wait time excluded from billing; Skyvern `HUMAN_INTERACTION` block.
- **Spend limit with a partial, resumable result**: Firecrawl `/agent` `maxCredits` → `credit_limit_reached` + `partial` + resume by `threadId`; Crawlee `UsageLimits`.
- **Confirmation categories for consequential actions**: Gemini's policy categories (financial, account creation, legal terms…), OpenAI `pending_safety_checks`, agent-browser `--confirm-actions`.
- **Adapters**: one JSON-Schema catalog maps mechanically onto every framework's tool shape; MCP reaches all 15 frameworks; native wrappers are rarely used ([§6](#6-agent-frameworks-and-adapters)).

### 9.3 Open questions for the decisions in #72

| Decision | Facts from this survey | Open questions |
|---|---|---|
| **Engine owns every model call** (Q8) | Tools that own calls can cache, account, redact and judge (Stagehand extension, Browser Use, Skyvern); tools that don't push cost and safety to the client (Playwright MCP, agent-browser). Owning calls makes provider breadth the engine's job: Stagehand lags the AI SDK ([#1645](https://github.com/browserbase/stagehand/issues/1645)), and LiteLLM brought a supply-chain incident to Browser Use and Crawl4AI. | Where do calls from an external agent's own model fit (they are outside the engine by nature)? Does the engine accept a BYO model callback through the SDK, like Stagehand's BYO-LLM callback, so exotic providers need no engine release? |
| **Provider layer** (bring your own key, local models) | Requested everywhere; Rust has no equivalent of the AI SDK or LiteLLM. Weak models break structured output; tools add JSON repair (Nanobrowser `jsonrepair`), validation retries (Crawlee `retries`) or model-family adapters (Midscene). Gateways exist (Browserbase Model Gateway, Browser Use gateway, Vercel AI Gateway). | OpenAI-compatible + native Anthropic + Gemini + callback, or more? Is structured-output repair and retry part of the provider contract? Which local models are qualified, and is that tested in the lab? |
| **Money limits** | Only Firecrawl and Crawlee stop on spend; others count steps. Prices change per model and gateway. | Limits in currency, tokens or both? Where does the price table come from, and how is a call that would cross the limit handled (pre-call estimate vs stop after)? Is the stop a partial, resumable result as in Firecrawl? |
| **observe / act / extract primitives** | The most praised API shape. Stagehand documents that `act()` can fail after its effect happened, so it advises `observe()` first. Skyvern and Midscene add a boolean `validate` / `aiAssert`. Midscene never caches queries or assertions. | Is a `check` / `validate` primitive part of the set? Which primitives are cacheable (act, observe) and which always run live (assertions, extraction of fresh data)? |
| **Built-in agent** | Stagehand removed it; Cua deprecated its agent; Playwright and agent-browser never had one. Browser Use, Skyvern, Notte, Nova Act and the hosted services keep one; several embedded-loop projects died. Independent success rates are low (30% for Browser Use on Online-Mind2Web). | Is the built-in agent a thin, journaled loop over the same catalog (and therefore replaceable by any external agent), and what does it add beyond the external-agent path: login recovery, repair, extraction discovery? |
| **Vision** | Accessibility tree is the default of the high-volume tools; vision-first tools (Midscene, Magnitude, computer-use APIs) cover canvas and desktop. Coordinate precision is a recurring pain; Cua binds each pixel click to a one-use `capture_id`. Screenshots cost roughly w×h/750 tokens on Claude. | Vision opt-in per call or per Recipe? Should coordinate actions be bound to the observation they came from, as Cua does? |
| **Recorder** | workflow-use's recorder is fragile (extension load, iframes not recorded); Libretto and Imprint record network traffic and compile to API calls; Director and Codex "Record & Replay" are commercial. | Does recording capture network calls as well as DOM actions, so a plan can climb a fetch → browser ladder? How are iframes and shadow DOM handled? |
| **AI extraction** | Two shapes: per-page LLM extraction and LLM-written selectors cached per route (Crawlee, Crawl4AI with a validation pass). Pains: empty or malformed output, chunking order, token cost on big pages. Firecrawl alone gates extraction with an injection classifier. Stagehand's unmerged PR 2955 copies values deterministically from selected elements instead of letting the model write them. | Is a "grounding" check (each extracted value must appear in the source) part of validation? What key decides when cached selectors are stale (route label, layout fingerprint)? |
| **Agent login with secrets** | Placeholders (Browser Use, Stagehand `%var%`), vault fills the model never sees (agent-browser, Kernel, Steel, Skyvern), TOTP, managed reauth (Kernel). Leaks: Stagehand's cache receives values; Kernel and Browser Use Cloud document that page scripts or the agent can still read filled values. OpenAI's CUA refuses logins. | Are secrets excluded from cache keys, journals and observations by construction? Are secrets scoped to domains, with a check that the target field did not change before typing (agent-browser)? |
| **Human pause** | Live view + takeover + resume in the same session is common in hosted services; Nova Act excludes human-wait time from billing; Gemini's confirmation loop frustrates users ([#97](https://github.com/google-gemini/computer-use-preview/issues/97)). | Does a paused run hold its browser, for how long, and do Limits (time, money) stop counting while paused? Which events announce "needs human" (CAPTCHA, 2FA, confirmation)? |
| **Prompt-injection safety** | Model vendors ship classifiers and confirmation hooks; tool-level defenses are untrusted-content tags (Nanobrowser), content boundaries (agent-browser), a pre-extraction classifier (Firecrawl), action policies. Allowlists leak in three tools; code execution has produced RCE reports. | Is the domain policy enforced at the network layer for every request type? Which effect classes require confirmation by default? Does the engine pass through vendor safety checks (OpenAI, Gemini) as journal events? |
| **Plan promotion after repair** | Automatic regeneration (Skyvern), no write-back of partial plans (Midscene), verify-then-cache (Momentic), human-published drafts (Anchor), PR agents (Libretto), healer gated by assertions (Playwright). | Is a repaired plan promoted only after a fully verified run, and as a reviewable diff of the versioned plan file? Who approves in autonomous mode? |
| **Adapters** (Q5 open point) | MCP reaches every framework; 18 of 22 tools have a Claude-side entry (skills, Claude Code, Agent SDK); native wrappers concentrate on LangChain, LlamaIndex and the AI SDK but have small downloads. Reach by downloads: AI SDK and Claude Agent SDK lead in TS, LangChain/LangGraph in Python. | Beyond MCP and skills, which thin generated shims ship in V1 (researcher's tiering in [§6](#6-agent-frameworks-and-adapters) proposes AI SDK, Mastra, LangChain JS/Python)? |
| **CLI for agents** (Q7) | The market is moving coding agents to CLI plus skills (Playwright, Midscene, agent-browser), while the lab measured a model-dependent CLI (Haiku left 18 of 45 tasks unfinished). | Does the tentative "CLI serves humans first" answer hold, given that competitors now pitch the CLI to agents? |

## 10. Method and sources

- **Process.** Five parallel research passes on 2026-10-10 (browser agents; MCP/CLI/vision/test/record tools; AI extraction and crawl; hosted browsers and computer-use APIs; agent frameworks and adapters), then a merge. The 2026-10-07 notes and the parity list were used as leads only and re-checked.
- **Primary sources read.**
  - GitHub through `gh api`: repository metadata, releases, commits, license files, READMEs, `package.json` / `pyproject.toml` / `Cargo.toml`, and docs sources inside repositories (Playwright Test Agents, Midscene, Notte, Cua driver, BrowserOS neo).
  - Issue searches sorted by reactions and by comments for each tool, plus about 40 keyword issue counts (Stagehand, Browser Use, Skyvern).
  - Registries: npm registry and downloads API, PyPI JSON, pypistats `recent`, crates.io; about 90 adapter package-existence probes.
  - Vendor docs as Markdown or `llms.txt`: Stagehand, Browser Use, Skyvern, Midscene, Notte, Firecrawl, ScrapeGraphAI, Spider Cloud, Apify, Steel, Hyperbrowser, Anchor, Kernel, Browserbase, Browserless, Bright Data, AgentQL, Laminar; OpenAI, Anthropic and Gemini computer-use guides; OpenAI Agents SDK docs; the Nova Act README and user guide; AWS pricing pages; DeepMind evaluation PDFs; the MCP 2026-07-28 changelog.
  - Framework directories: LangChain tool and loader indexes, LlamaIndex and CrewAI tool trees, Agno toolkits, Mastra `llms.txt`, the AI SDK tools registry, the ADK integrations catalog, `strands-agents/tools`.
  - Community: Hacker News through the Algolia API (about 20 threads read in full), vendor forums (OpenAI, Google AI).
  - Articles: Anthropic "Code execution with MCP" and "Advanced tool use", Cloudflare "Code Mode", Vercel "We removed 80% of our agent's tools", LangChain "MCP in LangChain".
- **Caveats.**
  - **Exa was unavailable** (HTTP 402, credits exhausted) from the first call, and Context7 was not needed; searches used the fallback web search and fetch tools, vendor indexes and registry APIs. Coverage of non-vendor pages is therefore thinner than planned.
  - **Reddit was mostly unreachable** (search domain blocked, JSON API refused). The few Reddit links come from the 2026-10-07 note and were not re-opened; they are marked where used.
  - GitHub search rate limits cut some keyword counts (Stagehand captcha, cloudflare, memory leak, tokens). Keyword counts are coarse and not deduplicated.
  - Reaction counts are small for most infrastructure repositories, so their pain themes rest on few data points; computer-use API pains come mostly from forums and HN.
  - Download counts include CI and transitive installs (for example `browser-harness` is a hard dependency of `browser-use`; `langchain-core`, `pydantic-ai-slim` and `@modelcontextprotocol/sdk` are pulled transitively).
  - No tool was installed or run, and no benchmark was reproduced. Only the Online-Mind2Web paper and the Browserbase-run Gemini 2.5 evaluation are independent of the vendor measured; Browser Use's Online-Mind2Web table is a competitor's evaluation.
  - Docs read from `main` can be ahead of the latest published release. Not verified: Browserbase Director internals, BrowserOS neo's MCP tool count, the full 17-member Anthropic toolset list, Hyperbrowser per-step credit rates, Project Mariner's current status, and Libretto's third-party benchmark figures (excluded).
