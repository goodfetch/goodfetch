# Stagehand 4.1.0 and alternatives — contracts, efficiency and reliability

State studied as of **October 7, 2026**. Supplement to the initial overview; this note does not carry over its schedule constraint, which the user withdrew. Goodfetch aims for an ambitious, portable architecture, with a canonical Rust engine, TypeScript/Python SDKs and a possible Go extension via the same protocol. No benchmark, cloud deployment or workflow on a real site was run for this note.

**Method:** four new Exa searches, eight results requested for each, i.e. `sources_reviewed = 32`. This counter includes duplicates and discarded results; it does not measure 32 validated independent sources. The documents actually used are cited below. Direct reading of the npm tarball of **Stagehand 4.1.0**, its TypeScript declarations, and the distributed SDK and extension; readings of official documentation and public code; seven competitor licenses opened. Conclusions about the stable package are kept separate from the promises of the current documentation and from the behavior of non-public servers.

## 1. The right opposition is not "Stagehand imposes AI, Crawlee allows code"

Stagehand has three different surfaces: a deterministic Chromium driver, model-assisted perception/resolution primitives and cache/repair mechanisms. A `page.locator("#submit").click()` call requests no resolution from an LLM. `observe()` requests candidates; `act(Action)` can replay a candidate deterministically; `act("instruction")` asks the model to choose an action; `extract()` requests a structured output. Saying that Stagehand does not offer classic selectors is therefore false. [Page API](https://docs.stagehand.dev/v4/reference/page), [Locator API](https://docs.stagehand.dev/v4/reference/locator), [act](https://docs.stagehand.dev/v4/basics/act), [observe](https://docs.stagehand.dev/v4/basics/observe).

Crawlee operates mainly on another dimension: request queue, dispatch, retries, sessions, concurrency and HTTP/browser choice. These mechanisms alone do not replace semantic resolution or proof of a business result. For identical work, reading an API or parsing data with known code avoids inference; asking a model to understand a page can reduce development and maintenance cost. The full cost of a correct output must be compared, not just the engine's language or the presence of the word AI. [Crawlee features](https://crawlee.dev/js/docs/introduction).

A Crawlee–Stagehand v4 integration is not the old "pass the Playwright Page to Stagehand" scenario: v4 does not accept that object. A browser/session adapter and a clear ownership rule are needed, or else one of the runtimes must be left to drive the browser. CDP portability does not automatically provide compatibility of objects, events and contexts. [Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright).

## 2. Stagehand 4.1.0: what the primitives actually return

The stable version used here is the one in the [npm registry 4.1.0](https://registry.npmjs.org/@browserbasehq/stagehand/4.1.0), published on September 9. The [exact tarball](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz) contains notably `dist/index.d.mts`, `dist/index.mjs` and `dist/extension/service-worker.js`. These distributed files were opened in memory, without installation. The function names cited below come from the stable bundle; the GitHub links serve to locate the readable modules and do not claim to pin `main` to the npm version.

| Stable primitive | Input and returned object | Work actually required | What success does not demonstrate |
| --- | --- | --- | --- |
| `page.snapshot()` | `{ formattedTree, xpathMap, urlMap }` | Structured capture without LLM; mapping of elements/URLs | Completeness of all business information and later DOM state |
| `page.locator(selector).click/fill/innerText/...` | Actions: `void`; reads: value; `count()`: number | CSS/XPath/`text=` selector, resolution and browser operation | Semantic correctness of the selector and business result |
| `observe(instruction?, options?)` | `{ data: Action[], metadata }`; `Action` contains `selector`, `description`, `method?`, `arguments?` | Snapshot then structured generation, or list served from cache | Choice of the right candidate, execution or objective reached |
| `act(instruction: string)` | `{ data: { success, message, actionDescription, actions }, metadata }` | Wait for DOM/network stabilization, snapshot, model resolution, execution; possible second step | That the right entity was modified or that a complete process is finished |
| `act(action: Action)` | Same `ActResult` | Direct replay; no initial snapshot/inference; repair possible if enabled | That the old target still represents the same entity |
| `extract(instruction, schema)` | `{ data: typed value, metadata }` | Snapshot and structured generation; in stable 4.1.0, a second model call for completeness; or cached output | Truth, freshness, exhaustiveness or provenance of each field |

The signatures and fields were verified in the tarball. The documentation describes the [act](https://docs.stagehand.dev/v4/basics/act), [observe](https://docs.stagehand.dev/v4/basics/observe) and [extract](https://docs.stagehand.dev/v4/basics/extract) objects. The TS SDK also applies `resolvedSchema.parse(response.data)`: schema conformance is a concrete check, but an old price that is correctly typed remains old.

**Two important nuances:**

1. `act(Action)` does not use the model on its normal path, but **the stable controller still checks that a model or a gateway is configured before entering this path**. A local browser without a model can use `page`/`locator`, but must not assume that `stagehand.act(action)` will accept the same configuration. "No inference" and "no LLM configuration required" are distinct contracts. Reading of `createStagehandController → act$2 → act` in the tarball; corresponding module [stagehandController.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/controllers/stagehandController.ts).
2. In stable 4.1.0, the inference function `extract$1` successively calls `generateStructured("Extraction", ...)` and `generateStructured("Metadata", ...)`. The second receives the instruction and the extracted output, **not the source snapshot**. The service reads `completed` and logs it, but does not expose it in `ExtractResult`; `cacheValue` remains the output even when `completed` is false. This is the behavior of the public client/runtime; the server may still refuse a write according to its own rules. This does not allow asserting that an incomplete extraction is systematically served by the cloud. Modules [extract inference](https://github.com/browserbase/stagehand/blob/main/packages/extension/inference.ts), [extractService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/extractService.ts), stable tarball above.

### Exact example of a hybrid surface, with explicit checks

Illustrative example on a test catalog: the selectors and values must match the fixture. It shows the v4 return values, not an executed benchmark. The model is configured because `observe`/`extract` need it; `locator.fill` remains deterministic.

```ts
import { localBrowser, Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod/v4";

const browser = await localBrowser.launch({ headless: true });
const stagehand = await Stagehand.create({
  browser,
  model: { modelName: "openai/gpt-4.1", apiKey: process.env.OPENAI_API_KEY },
  cache: false,
  selfHeal: false,
});

try {
  const page = await browser.context.newPage("http://localhost:3000/catalogue");
  if (!(await page.waitForSelector("#search", { state: "visible" }))) {
    throw new Error("Search field missing");
  }
  await page.locator("#search").fill("SKU-42");

  const observation = await stagehand.observe(
    "Find the search submit button", { page, cache: false },
  );
  const candidates = observation.data.filter(a => a.method === "click");
  if (candidates.length !== 1) throw new Error("Ambiguous target");

  const execution = await stagehand.act(candidates[0], { page });
  if (!execution.data.success) throw new Error(execution.data.message);
  if (!(await page.waitForSelector('[data-sku="SKU-42"]', { state: "visible" }))) {
    throw new Error("Expected result missing");
  }

  const extraction = await stagehand.extract(
    "Extract SKU, name and price of the product with SKU-42",
    z.object({ sku: z.string(), name: z.string(), price: z.number() }),
    { page, cache: false, locator: page.locator('[data-sku="SKU-42"]') },
  );
  if (extraction.data.sku !== "SKU-42") throw new Error("Wrong product");
  console.log(extraction.data, extraction.metadata.usage);
} finally {
  await stagehand.close();
  await browser.close();
}
```

The SKU check does not prove the price value; a deterministic comparison with the source text, or an authoritative network response, would be necessary for that. This point must appear in Goodfetch's examples: each check must state the property it establishes.

### The v4 driver does not inherit Playwright's guarantees

The [official migration](https://docs.stagehand.dev/v4/migrations/playwright) says explicitly that v4 has no Playwright auto-waiting: a selector is resolved once. `waitForSelector()` returns a boolean; a prior wait does not replace a per-action retry during a re-render. `goto()` waits for `domcontentloaded` by default; Playwright waits for `load`. No `getBy*`, no general chaining/filter, no `waitForResponse/Request/Event/URL`, and no web-first assertions. The events exposed by `page.on()` are limited to `console` in the stable types. There is a single BrowserContext per browser. The driver is Chromium only.

On the other hand, Stagehand documents traversing iframes with `>>` and CSS piercing of Shadow DOMs, including closed ones. These capabilities must be distinguished from waits/retries: more accessible targets do not make an action temporally reliable. For `click`, Playwright documents checks for visibility, stability, event reception and enabled state, and offers auto-retrying assertions. Even these checks verify actionability, not a business result. [Playwright actionability](https://playwright.dev/docs/actionability), [Stagehand locators](https://docs.stagehand.dev/v4/reference/locator).

## 3. Cache, repair, batching: three different optimizations

### Three cached values

Reading `withCache`, `replayCachedActions`, `observe` and `extract` in stable 4.1.0 distinguishes:

| Method | Cached value | Handling of a HIT | Risk to test |
| --- | --- | --- | --- |
| `act(string)` | Array of actions | Replay against the current browser | Selector still valid but wrong entity; effect before a late error |
| `observe` | Array of candidate actions | List returned without new LLM resolution | Mapping that has become semantically wrong, or outdated discovery result |
| `extract` | **Structured output** | Output returned directly without new extraction | Old result if the key/matching does not distinguish the relevant change |

The cache does not rely only on the instruction: the runtime collects the CDP AX trees per frame and transmits URL, tree content, instruction/options and, for extract, schema. The documentation asserts that a change of content/structure causes a miss; the exact canonicalization/matching mechanism, the TTLs and the service's freshness policy are not established by the public code read. One must therefore neither say that every HIT returns a stale value, nor sell every HIT as proof of freshness. The model configuration is **deliberately excluded from the key**: changing models does not force a new resolution. [Official cache](https://docs.stagehand.dev/v4/best-practices/caching), [cacheService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/cacheService.ts).

The standard cache requires **key + Browserbase session**; it is not equivalent to a portable local OSS cache. Calls with `locator` or `ignoreLocators` bypass it; `extract({ screenshot: true })` bypasses it too, since pixels are not represented by its key. Reducing context can save tokens while preventing hits: measure this combination, do not blindly add up the gains. Variables are removed from the prompt via placeholders, but their values appear in the cache request; the documentation advises disabling the cache for credentials. [act](https://docs.stagehand.dev/v4/basics/act), [extract](https://docs.stagehand.dev/v4/basics/extract).

**Verified documentation divergence:** the current docs announce the cache as enabled by default. The stable 4.1.0 package has `cache` as optional in the schema, the SDK injects no default value, the Browserbase metadata inject apiKey/session/region, and `buildCacheContext` uses `initParams.cache ?? false`. `withCache` then disables the lookup when the option is absent. This static reading of the distributed path justifies **configuring `cache: true` explicitly and measuring `metadata.cache.status`**; it does not demonstrate the behavior of a different cloud version. Docs and package must be aligned before using the implicit gain in a financial calculation.

### Self-heal is not an oracle of the right target

`takeDeterministicAction` repairs after an exception from the operation, if `selfHeal` is enabled: recapture, new choice, new attempt. An action without a recognized method returns `success: false`. A timeout is propagated. The cache path replays with `selfHeal: false`, then its failure triggers the full pipeline through the cache wrapper. An incorrect but clickable button does not necessarily produce an exception: this scenario therefore does not trigger the observed repair. [actService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/actService.ts).

The Jev semantic guard proposals [PR 2954](https://github.com/browserbase/stagehand/pull/2954) address this problem, but PRs 2952–2955 and 3103 were open/unmerged at the time of the initial investigation, and `experimentalDecisions` is absent from the stable package. Do not incorporate them into the v4.1 guarantees. The Goodfetch engine must check target identity before effects and postconditions after them, with abstention on ambiguity; a mere model confidence does not replace these two contracts.

### Batching and external agent

`experimentalBatch()` is present in stable 4.1.0. It executes a self-contained JavaScript callback in the service worker, with JSON inputs/outputs, and reduces SDK–runtime round trips. It does not remove local CDP operations, the requested inference or the browser cost. It is not a transaction: a timeout does not retroactively cancel an action already launched. No host lexical closure, host filesystem, browser closing or nested batch. Active-page and chosen-page constraints must be explicit in multi-tab workflows. [Stagehand reference](https://docs.stagehand.dev/v4/reference/stagehand).

Finally, **`agent()` was removed in v4**. Code mode and an external agent loop use the API; this does not mean there is no autonomous capability in the Browserbase ecosystem, but the stable SDK no longer ships this old method. Blindly retrying an `act()` after an error can repeat an effect; the migration advises escalating at the observation level. [v3 migration](https://docs.stagehand.dev/v4/migrations/v3).

## 4. Six close alternatives, compared by contract

### Perceive, resolve, act, verify

| Approach | Capture and resolution | Execution / useful object | Verification and limit |
| --- | --- | --- | --- |
| **Browser Use**, historical and beta | Historical: DOM + AX + frames, indexed references, optional vision. Beta: Rust agent/runtime, browser script tools, Python wrapper and JSON-RPC. | Multi-step loop; `AgentHistoryList` history, actions/tools and final result. Deterministic initial actions exist. | Browser state, failure budgets and structured outputs; the agent's end of task or Pydantic validation do not prove truth. The beta ledger still lists synchronization/navigation/observability fixes. |
| **Skyvern** | Playwright-enhanced page, DOM/vision; direct selector or AI prompt; Python `AILocator` resolves lazily. | `act` returns None; click/fill notably return the resolved selector; extract returns the data; multi-step agent returns Task/WorkflowRunResponse. | `validate(prompt)` returns a bool via the model. Deterministic assertions remain possible via Playwright. Distinguish an LLM verdict from a verifiable business rule. |
| **AgentQL** | Accessibility/page capture, declarative query to a semantic backend; `standard` or `fast`. | `queryElements` → AQLResponseProxy whose fields drive Playwright Locators; `getByPrompt` → Locator or null; `queryData` → object. | Playwright handles execution/actionability; semantic selection comes first. Neither the valid locator nor the returned object proves business identity. Fast mode documents a precision trade-off. |
| **Magnitude** | Navigation/interaction guided by vision and coordinates; extraction documented on the DOM. | `agent.act` can accomplish one or more actions; `extract` directly returns the Zod schema; Playwright Page/Context accessible. | Test runner and `agent.check(description)` evaluated by AI; combine with code assertions when the condition is deterministic. A high-level objective is not comparable to a single Stagehand act. |
| **Imprint** | User recording of requests and interactions, then compilation into an API chain + DOM playbook. | Typed MCP; network/browser/DOM re-execution via several backends, without a new LLM decision on the deterministic path. | Verifier independent of compilation, suites and receipts; general maturity not demonstrated. Semantic errors are not repaired by simply changing transport. |
| **agent-browser / Playwright CLI** | Snapshots and references; classic selectors available. The external model chooses commands/code. agent-browser adds an optional AI chat. | Short commands and sessions; agent-browser is a Rust binary/daemon; Playwright CLI uses Playwright and can `run-code`. | Command return, exit/error and new capture; business assertions to be written. Playwright has its actionability checks; do not automatically attribute them to all CLI drivers. |

Contract sources: [BrowserSession](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/session.py), [Browser Use DOM](https://github.com/browser-use/browser-use/blob/main/browser_use/dom/service.py), [beta ledger](https://github.com/browser-use/browser-use/blob/main/BETA_AGENT_INTEGRATION_FEATURES.md), [Skyvern SDK](https://www.skyvern.com/docs/developers/browser-automations/actions-reference), [AgentQL Page](https://docs.agentql.com/javascript-sdk/api-references/agentql-page), [Magnitude BrowserAgent](https://docs.magnitude.run/reference/browser-agent.md), [Magnitude TestCaseAgent](https://docs.magnitude.run/reference/test-case-agent.md), [Imprint architecture](https://github.com/ashaychangwani/imprint/blob/main/docs/architecture.md), [agent-browser](https://github.com/vercel-labs/agent-browser), [Playwright CLI](https://github.com/microsoft/playwright-cli).

### Cache, replay and freshness

| Approach | What is reused | Data reading during the rerun | Economic consequence |
| --- | --- | --- | --- |
| **Browser Use Cloud V4 scripts** | Script in a persistent workspace; each run restarts an agent | The docs call for a live request, fetched_at, source hash, schema and script repair | Less rediscovery; **tokens still consumed** on each rerun. The cloud pattern is not automatically an OSS function of the beta. |
| **Skyvern code caching** | Executable sequences, per task or per block; progressive branch coverage | Script against the current page; conditions/waits/code remain live | `run_with="code"` can avoid the agent's inference and screenshots; fallback to the agent/regeneration. The expected effect must still be verified. |
| **AgentQL** | Resolved locators and calling program | Current Playwright reads or new queryData | No durable engine for compiled plans/freshness demonstrated in the contract consulted; backend paid for each new semantic resolution. |
| **Magnitude** | Anthropic prompt caching, memory and retained screenshots | The loop continues its perceptions/actions | Model prefix cache ≠ replay without LLM. The README still describes its native deterministic cache as "in progress". |
| **Imprint** | API workflow + DOM playbook + winning backend | New request to the site, variables/cookies/state captured during the run | Can eliminate browser and inference when fetch suffices. The README's timings and zero-token slogans are not reproduced results. |
| **CLI** | Written program, sessions/profiles and artifacts | Commands/scripts reread the browser; free orchestration | A deterministic script can run without a model; a coding-agent loop still consumes tokens. Reducing snapshot exposure can save context. |

Sources: [Browser Use scripts](https://docs.browser-use.com/cloud/agent/scripts), [Skyvern cache](https://www.skyvern.com/docs/developers/features/code-caching), [AgentQL Page](https://docs.agentql.com/javascript-sdk/api-references/agentql-page), [Magnitude memory](https://docs.magnitude.run/advanced/memory.md), [Magnitude README](https://github.com/magnitudedev/browser-agent), [Imprint](https://github.com/ashaychangwani/imprint), [Playwright CLI](https://github.com/microsoft/playwright-cli).

The most important distinction is between **response cache**, **resolution/plan cache**, **LLM prefix cache** and **backend/session cache**. They have neither the same freshness guarantees nor the same savings. Goodfetch must display these categories separately in its metrics.

### OSS, SDK and cloud: no presumed parity

| Project | License actually opened | Useful SDK / openness | Portability limit |
| --- | --- | --- | --- |
| **Stagehand** | [MIT](https://github.com/browserbase/stagehand/blob/main/LICENSE) | TS/Python/Go sharing a generated contract; local extension or Browserbase | Cloud cache/gateway are not an included OSS cache server. The JavaScript batch callback is not a native Python/Go closure. |
| **Browser Use** | [MIT](https://github.com/browser-use/browser-use/blob/main/LICENSE) | Historical Python agent and explicit Rust beta; terminal runtime [MIT](https://github.com/browser-use/terminal/blob/main/LICENSE) | Cloud V4/workspaces and Python version 0.13 are two surfaces. An identical Agent name does not imply an identical engine. |
| **Skyvern** | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern/blob/main/LICENSE) | Public backend, Python/TS SDK, self-host and cloud | The current reference lists AI locator, forms, several operations/iframes as **Python only**. Cloud anti-bot is separate from the public code. Do not copy as a permissive component without a licensing decision. |
| **AgentQL** | [MIT for public repository](https://github.com/tinyfish-io/agentql/blob/main/LICENSE) | Examples, Python/JS clients, Playwright Page | Public repository mainly examples; service resolution engine not demonstrated to be open. Running Chrome locally does not remove the API dependency. |
| **Magnitude** | [Apache-2.0](https://github.com/magnitudedev/browser-agent/blob/main/LICENSE) | TS, agent + runner; Playwright/Patchright; BYOK models | No three equivalent SDKs established. Full Playwright access available, unlike Stagehand v4. |
| **Imprint** | [MIT](https://github.com/ashaychangwani/imprint/blob/main/LICENSE) | Bun/TS, transferable MCP tools, separate credentials | A little-proven niche; captured private API and browser state can change. MCP carries a tool, not semantics shared across drivers. |
| **agent-browser / Playwright CLI** | [Apache-2.0 agent-browser](https://github.com/vercel-labs/agent-browser/blob/main/LICENSE), [Apache-2.0 CLI](https://github.com/microsoft/playwright-cli/blob/main/LICENSE) | Deterministic CLIs suited to coding agents; sessions; providers via CDP | Control tools, not on their own a durable business runtime with schemas, proofs, repair policy and scheduler. |

The licenses above describe the files/repositories consulted, not all of a vendor's commercial services. The [Skyvern README](https://github.com/Skyvern-AI/skyvern) and its [actions reference](https://www.skyvern.com/docs/developers/browser-automations/actions-reference) materialize the surface gaps; [Magnitude Playwright access](https://docs.magnitude.run/core-concepts/playwright.md) confirms access to Page/Context.

## 5. What these comparisons impose on an ambitious Rust architecture

**The canonical Rust engine is coherent for a portable product; its language is not its differentiation.** Browser Use beta and agent-browser already have Rust paths. Stagehand reduces round trips via an extension and batching. The choice of execution placement, of the IR and of verification contracts can weigh more than a few milliseconds of serialization.

The recommended boundary for Goodfetch is:

1. **Portable Rust core**: sessions, states/events, deadlines, cancellation, budget, concurrency, journal/replay and typed plans. Thin TS/Python/Go SDKs, negotiated protocol version and the same errors/results. Full local operation, including the plan cache.
2. **Execution adapters**: HTTP/fetch first when the contract allows it; native Chromium CDP; Playwright-compatible worker for flows that require its locators, waits, frames, interception and assertions; optional extension for the user's browser. Capabilities declared per backend, never silent emulation of unsupported functions.
3. **Observation and resolution separate from action**: DOM/AX/vision/network with frame/target identity and validity duration; deterministic resolver before the model; several candidates, abstention and fallback budgets. A `ResolvedTarget` carries the identity evidence that justifies the effect.
4. **Explicit verification**: schema, target, postcondition, freshness and provenance are five distinct checks. Technical `success` and business `verified` are not the same field. The oracle must be deterministic when possible, and flag its probabilistic nature when it depends on an LLM judge.
5. **Typed cache and documented invalidation**: reusable plans kept separate from outputs. A live extraction rereads the source; a cached result only under an explicit FreshnessPolicy with age/source/version. Semantic diff and target guards before replay; per-segment auto-heal under a budget and an effect policy.
6. **Comparable measurement**: cost and duration of first compilation, cold/warm replay, repair, correct output, false success and human intervention; number of captures, RPCs and inferences per primitive. A single public `extract` can hide several model calls.

A Rust sidecar is a good cross-language and isolation boundary. But "direct CDP" must still demonstrate actionability, frame/OOPIF coordination, reference invalidation, hit testing, Shadow DOM, navigation and DOM races. A high implementation capacity speeds up construction; it does not remove the compatibility matrix and the maintenance across Chromium versions. The [native agent-browser](https://github.com/vercel-labs/agent-browser/tree/main/cli/src/native) and [Browser Use terminal](https://github.com/browser-use/terminal) work are starting points to audit, rather than automatic proof of Playwright parity.

**Plausible economic differentiation:** frequent workflows whose outputs are fresh, attributable to sources and correctly verified, with an explainable full cost and bounded repair. The novelty remains to be established: Skyvern, Imprint and Browser Use already cover reusable code and repair; Imprint already retains verification evidence. Goodfetch's advantage must be demonstrated through its contracts, portability, operations and results on a corpus held out from development.

## 6. Uncertainties and decisive experiments

This research does not make it possible to rank universal reliability. Notably still open: the exact behavior of the Browserbase cache server, the maturity of the Browser Use Rust beta, Skyvern repairs on silent semantic errors, Imprint's generalization beyond its examples and the fidelity of Magnitude's vision assertions. The examples in the report are inspected against types/docs, not executed.

A useful corpus must force: a wrong target keeping a valid XPath, row reordering, data changing without a layout change, a new value under an identical URL, a rerender between wait and click, OOPIF navigation, Shadow DOM, auth expiry, a timeout after an effect, and retry. Measure detected failure and **false success** separately. Compare an optimized deterministic program, Stagehand without cache, with explicit cache, observe→replay, batching, a multi-step agent and a compiled script. The best tool changes according to this contract; the product goal is to choose the cheapest path that satisfies the requested verification.
