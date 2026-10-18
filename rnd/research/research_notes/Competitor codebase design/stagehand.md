# Stagehand (browserbase/stagehand): codebase design study

Studied at commit [`10418f9`](https://github.com/browserbase/stagehand/tree/10418f98831285b3cddca5328c8013a94f22672a) (2026-10-15, "[fix]: add new webmcp launch flag (#3198)"). The code is the primary source. Sections 8 and 10 separate what maintainers say from my own reading. "B/" stands for `https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/` in the link text, and every link is a full permalink.

> **Headline.** The `main` branch is now **Stagehand v4**, a protocol-first monorepo. The browser runtime that executes `act/extract/observe` ships as a **Chrome MV3 extension**. The TypeScript, Python and Go SDKs talk to it over **bidirectional JSON-RPC tunnelled through CDP**. The extension drives pages through its own CDP layer, "understudy", with no Playwright. `agent()` was removed. The whole v3 tree (`core`, `server-v3`, CLI) moved to a frozen `v3` branch ([PR #2671](https://github.com/browserbase/stagehand/pull/2671)). This is closer to Goodfetch's shape (a protocol crate, generated SDKs, a direct CDP client) than to any v3 design.

## 1. Snapshot

| Item | Value |
|---|---|
| Repo / commit | `browserbase/stagehand` @ `10418f98831285b3cddca5328c8013a94f22672a`, 2026-10-15 13:25 −0700 |
| Languages (tokei, whole repo; Code column) | TypeScript 1,142 files / 187,375 · Go 84 / 23,739 · Python 82 / 18,469 · JavaScript 9 / 4,290 · MDX 158 files (62k lines of docs prose) · YAML 22k (mostly `pnpm-lock.yaml`) |
| Workspace | pnpm 11 + Turborepo + `just`. 27 pnpm packages: 9 under `packages/` and 18 under `packages/integrations/` ([pnpm-workspace.yaml](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/pnpm-workspace.yaml#L1-L3)). There are also 9 `pyproject.toml` and 5 `go.mod` |
| Published | `@browserbasehq/stagehand` (sdk-ts 4.2.1), PyPI `stagehand`, the Go module (tagged separately) and the `browse` CLI. `protocol`, `extension`, `integrations`, `evals` and `docs` are private ([CONTRIBUTING.md#L21-L36](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/CONTRIBUTING.md#L21-L36)) |

Size per package (tokei `Total`: files / lines / code):

| Package | Files | Lines | Code | Notes |
|---|---|---|---|---|
| `packages/protocol` | 42 | 16,104 | 15,285 | Includes the 7,737-line generated `stagehand.v4.json`. The hand-written TS is 3,666 lines |
| `packages/extension` | 169 | 41,648 | 36,115 | 19.6k code lines outside tests: runtime, understudy CDP layer, LLM, services |
| `packages/sdk-ts` | 112 | 22,854 | 20,139 | `src/` is only 4.9k code lines. The rest is tests and examples |
| `packages/sdk-python` | 64 | 18,708 | 15,482 | Includes 3.8k lines of generated Pydantic models |
| `packages/sdk-go` | 84 | 27,807 | 23,369 | Includes the 3,088-line `models.gen.go` and an embedded extension zip |
| `packages/integrations` | 234 | 35,053 | 30,930 | MCP "facade" core plus 10 agent-framework adapters and examples |
| `packages/evals` | 421 | 84,861 | 73,653 | Larger than the product. Includes a TUI (11.5k) and 24.9k lines of tests |
| `packages/cli` (`browse`) | 200 | 28,439 | 24,352 | oclif CLI, linted separately with ESLint |
| `packages/docs` | 193 | 70,202 | 7,377 | Mintlify. v2, v3 and v4 docs are all kept |
| `packages/examples` | 100 | 9,495 | 8,204 | |
| `rules/` | 13 | 3,527 | 3,234 | Custom oxlint plugin and ast-grep architecture tests |
| `scripts/` | 33 | 3,290 | 2,876 | Release tooling, with its own tests |

The product core (protocol TS, the extension outside tests, and sdk-ts `src/`) is about **28k code lines**. Evals and tests dominate the repository.

File-size distribution for hand-written source (TS/Py/Go, excluding tests, generated code and docs): 830 files, mean 168 lines, 68 files over 500 lines and 15 over 1,000. The ten largest:

| Lines | File |
|---|---|
| 3,519 | [`packages/integrations/core/src/facade/runtime.ts`](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/core/src/facade/runtime.ts) (one exported function, serialized via `Function#toString`) |
| 2,770 | [`packages/extension/understudy/page.ts`](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/page.ts) |
| 2,110 | [`packages/protocol/schemas.ts`](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schemas.ts) |
| 1,511 | `packages/evals/lib/braintrust-report.ts` |
| 1,478 | `packages/evals/scripts/render-braintrust-core-report.ts` |
| 1,435 | `packages/evals/core/tools/cdp_code.ts` |
| 1,285 | [`packages/extension/understudy/context.ts`](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/context.ts) |
| 1,262 | `packages/sdk-go/cdp_client.go` |
| 1,243 | `packages/sdk-go/browserbase_client.go` |
| 1,214 | `packages/integrations/fx-sdk/src/session.ts` |

Not counted above: a 1,689-line built JS copy of the facade committed twice ([B/…/deepagents/src/stagehand_deepagents/_assets/playwright_facade.js](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/deepagents/src/stagehand_deepagents/_assets/playwright_facade.js)), and test files of up to 2,940 lines (`extension/tests/stagehand-clients.test.ts`).

## 2. Paradigm

- **Runtime topology.** The system has "two halves", as [CONTRIBUTING.md#L21-L25](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/CONTRIBUTING.md#L21-L25) puts it.
  - The SDK holds the browser handle.
  - The runtime is a Chrome extension service worker that serves JSON-RPC.
  - The SDK attaches to the worker target. It sends requests with `Runtime.evaluate` calling `__stagehandReceiveFromHost`. It receives with `Runtime.addBinding` on `__stagehandSendToHost` ([B/packages/sdk-ts/src/cdpClient.ts#L370-L422](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/cdpClient.ts#L370-L422); [B/packages/extension/service-worker.ts#L30-L101](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/service-worker.ts#L30-L101)).
  - The worker then opens its own CDP WebSocket back to the browser to drive pages (`BrowserContext.create(cdpUrl, …)`, same file, L44-L52).
  - An offscreen-document heartbeat keeps the MV3 worker alive (`service-worker-lifecycle/heartbeat-manager.ts`).
  - The JSON-RPC is bidirectional: the server can call `llm.generate` back on the client.
- **Concurrency.** Everything runs on single-threaded `async/await` event loops.
  - In the TS runtime, lifecycle operations are serialized through a promise chain (`enqueueLifecycle`, [runtime.ts#L1033](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtime.ts#L1033-L1041)).
  - Each operation gets one `Progress`, a shared deadline plus `AbortController` that every downstream step consults ([B/packages/extension/understudy/progress.ts#L13-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/progress.ts#L13-L60)).
  - Python is asyncio-only. Go puts `context.Context` first on every blocking call, as its design note requires ([B/packages/sdk-go/idiomatic_go.md](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-go/idiomatic_go.md)).
- **Style.** The code favours composition and functions; inheritance is used only by `Error` subclasses. A grep of the extension, sdk-ts and protocol found 52 classes and no non-error `extends`.
  - Controllers are factory closures, for example `createStagehandController(runtime)` returning `{init, act, …}` ([stagehandController.ts#L27-L166](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/controllers/stagehandController.ts#L27-L166)).
  - Services are module functions that take a single options object (`actService.act({...})`).
  - Classes are used only for stateful identities: `StagehandRuntime`, `Page`, `Frame`, `CdpConnection`, `StagehandLogger`.
- **Public API.** It follows the Playwright object model:
  - `Stagehand.create({browser, model})` is a static factory with a private constructor.
  - Browser factories `localBrowser.launch/connect` and `browserbase.launch/connect`.
  - `Page`, `Locator`, `BrowserContext` and `Response` are thin remote proxies.
  - `act/observe/extract` take overloads, and `extract` takes a Zod or Pydantic schema.
  - Results are `{data, metadata:{usage, cache}}` envelopes.
  - The barrel uses explicit named exports only ([B/packages/sdk-ts/src/index.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/index.ts)).
- **Typing.** Zod v4 schemas are the source of truth (`z.infer`), and protocol objects must use `z.strictObject`, enforced by a test. The compiler settings are lax, though:
  - protocol, extension and sdk-ts set `"noImplicitAny": false` with only `strictNullChecks` ([B/packages/extension/tsconfig.json#L4-L5](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/tsconfig.json#L4-L5)).
  - evals sets `"strict": false` ([B/packages/evals/tsconfig.json#L4-L6](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/evals/tsconfig.json#L4-L6)).
  - Only the CLI and examples use `"strict": true`.
  - Python is stricter: `ty` with error-level rules and ruff `ANN`.

## 3. Organization

- **Top level.** The `packages/` folder holds:
  - `protocol`: the contract.
  - `extension`: the server.
  - `sdk-ts`, `sdk-python` and `sdk-go`: the clients.
  - `integrations`: the agent-harness MCP facade plus per-framework adapters.
  - `evals`, `cli`, `docs` and `examples`.

  Repo-level `rules/` holds architecture lint and tests, and `scripts/release` holds release tooling.

  The repo has no `src/` for the server: extension files sit flat at the package root (20 root `.ts` files: `runtime.ts`, `rpcRouter.ts`, `prompt.ts`, `inference.ts`, `utils.ts`, …).
- **Extension internals:**
  - `controllers/` holds per-RPC-object facades.
  - `services/` holds act, extract, observe, cache and llm.
  - `llm/` holds the AI SDK, gateway and client-LLM backends.
  - `understudy/` is the CDP layer: connection, page, frame, locator, a11y snapshot and network.
  - `dom/` holds scripts injected into pages.
  - `clients/` holds RPC, cache HTTP and Chrome runtime clients.
  - `types/private/` holds internal types. Public types are the protocol's, which is a clean public/internal split.
  - A residue of v3 remains: `handlers/handlerUtils/` sits beside `services/`, and six files still start with `// lib/v3/...` path comments ([B/packages/extension/understudy/cdp.ts#L1](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/cdp.ts#L1)).
- **Package dependency direction** (declared `workspace:*` dependencies, checked in each package.json):
  - `protocol` is a leaf.
  - `extension` and `sdk-ts` depend only on `protocol`. sdk-ts bundles the built extension zip at build time ([B/packages/sdk-ts/tsdown.config.ts#L26-L32](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/tsdown.config.ts#L26-L32)).
  - `integrations/core` and `cli` depend on `@browserbasehq/stagehand`.
  - `evals` depends on everything, plus the old major via an npm alias.
  - Inside the extension, `understudy/` imports nothing from `services/`, `llm/`, `inference` or `prompt`, so AI depends on browser and never the reverse (grep-verified).
  - **One boundary violation:** `extension/callbackBatch.ts` imports SDK *source* by relative path (`../sdk-ts/src/page.js`, `browserContext.js`, `clientSchemas.js`, …) without declaring the dependency ([B/packages/extension/callbackBatch.ts#L15-L27](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/callbackBatch.ts#L15-L27)).
- **Public vs internal.**
  - `protocol` exposes subpath exports only ([B/packages/protocol/package.json](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/package.json)).
  - A test, "keeps low-level RPC clients out of public SDK exports", enforces that `RPCClient` is not exported ([B/rules/ast-grep/sdk-parity.test.ts#L459](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/rules/ast-grep/sdk-parity.test.ts#L459)).
  - Go keeps transports under `internal/`.
- **Naming.**
  - Wire methods are `object.snake_case` (`page.wait_for_selector`), and the TS registry keys are camelCase (`pageWaitForSelector`).
  - TS file names mix camelCase (`rpcRouter.ts`), kebab-case (`schema-registry.ts`, `wire-casing.ts`) and snake_case (`evals/core/tools/cdp_code.ts`). No lint rule enforces one convention.
- **Tests, fixtures and docs.**
  - Tests live per package in `tests/`, except understudy, whose tests are co-located (`understudy/*.test.ts`).
  - Type tests use `*.test-d.ts`.
  - The root [vitest.config.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/vitest.config.ts#L1-L35) enumerates every include glob.
  - Wire fixtures live in `protocol/tests/fixtures/`.
  - Docs live in `packages/docs/v4/**` and are tested against the SDKs.
- **File-size discipline.** It is uneven. The mean is low (168 lines), but the core CDP `Page` has about 100 members in 2,770 lines, and `StagehandRuntime` has 93 methods. See §5.

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| **Schema-first method registry** (registry + data-driven codegen) | [schema-registry.ts#L129-L542](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schema-registry.ts#L129-L542); [build-json-rpc-schema.ts#L1-L102](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/json-rpc/build-json-rpc-schema.ts#L1-L102) | One table `{name, params, result, paramsWire?, resultWire?}` per method, which becomes one JSON Schema document, which feeds the Python and Go generators | **Good.** A single source of truth for 80 methods and notifications. TS consumes the Zod objects directly with no codegen step |
| **Typed method descriptors as commands** | [commandClient.ts#L6-L12](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/commandClient.ts#L6-L12); [sdk-ts/src/page.ts#L109-L117](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/page.ts#L109-L117) | `send(StagehandMethods.pageGoto, params)` pairs params and result types at compile time, so the SDK cannot misspell a method or mistype a payload | **Good.** The same interface is implemented in-process by the extension (`InProcessCommandClient`), so SDK objects run inside the worker for batch callbacks |
| **Remote proxy + opaque handle table** | [ResponseHandleTable#L27](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/responseHandleTable.ts#L22-L40) | Live CDP objects (responses) stay server-side. Clients hold random-UUID handles with a capacity bound (10,000) | **Good.** CDP request and session IDs never cross the wire |
| **Composition root + injected adapters with null-object defaults** | [service-worker.ts#L30-L95](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/service-worker.ts#L30-L95); [runtime.ts#L286-L319](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtime.ts#L286-L319) | Chrome APIs, WebSocket factory, log sink and client-LLM are injected, so the runtime is testable outside Chrome | **Good**, with a small duplication: three near-identical notify-and-catch closures (L53-L77) |
| **Front controller (switch dispatcher)** | [rpcRouter.ts#L140-L527](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/rpcRouter.ts#L140-L527) | Routes the validated request to a controller | **Bad.** 76 hand-written `case`s, each repeating `parseParams(StagehandMethods.X, request.params)`. The code admits "the generated request union does not currently narrow params from the method name" ([L75-L76](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/rpcRouter.ts#L75-L76)). A name-to-method map exists ([schema-registry.ts#L586](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schema-registry.ts#L582-L588)) but does not drive dispatch. Exhaustiveness is checked by an AST test, not the compiler |
| **Facade layers** (controller, then runtime, then understudy) | [locatorController.ts#L15-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/controllers/locatorController.ts#L15-L60); [runtime.ts#L843-L981](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtime.ts#L843-L981) | Each layer adds one concern: logging, then the deadline (`runLocator`), then CDP | **Mixed.** Most locator and page controllers are a pure pass-through plus `logger.debug`, so three hops add little |
| **Around-advice / decorator `withCache`** | [cacheService.ts#L197-L340](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/services/cacheService.ts#L197-L340) | One fail-open cache intercept wraps act, observe and extract (`onHit` / `execute`). Every cache failure falls back to execution | **Good.** The policy lives in one place. The leak is `page: unknown`, duck-typed via `asCachePage` |
| **Strategy for LLM backends** | [llmService.ts#L13-L35](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/services/llmService.ts#L13-L35); [aiSdkClient.ts#L132-L174](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/llm/aiSdkClient.ts#L132-L174) | Picks the client-supplied LLM, the Browserbase gateway or a Vercel AI SDK provider | **Good** selection logic. The provider set is a closed 5-value enum ([schemas.ts#L15-L17](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schemas.ts#L15-L17)), so a new provider means a release |
| **Reverse RPC callback** (inversion of control across processes) | [sdk-ts/src/stagehand.ts#L200-L205](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/stagehand.ts#L200-L205); [clientLlmClient.ts#L10-L19](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/llm/clientLlmClient.ts#L10-L19) | A user-supplied `generate()` is served by the SDK when the server sends `llm.generate`. The result is validated against a schema derived from the request | **Good.** This is the main extension point, and it works across languages |
| **Progress / deadline object** (cancellation token) | [progress.ts#L13-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/progress.ts#L13-L60) | One per-call deadline with named phases and `onLateResult` cleanup for resources that arrive after timeout | **Good.** It is a Playwright idiom, and `throwIfStopped` checks the clock too |
| **Transport multiplexer** | [cdp.ts#L36-L45, #L102](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/cdp.ts#L36-L110) | Flattened CDP sessions over one injected text transport. Inflight calls are tracked per session, and responses and events are Zod-validated | **Good** layering ("does not interpret Page/DOM/Runtime semantics"). The weakness is error classification by message prefix ([L23-L29](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/cdp.ts#L23-L29)) |
| **Pipeline** (a11y and DOM hybrid snapshot) | [capture.ts#L42-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/a11y/snapshot/capture.ts#L42-L60) | A five-step documented flow: scope, build per-session DOM indexes, per-frame maps, iframe XPath prefixes, merge. Each step is an exported function | **Good.** The structure is readable and testable step by step |
| **State machine as a discriminated union** | [runtimeState.ts#L1-L12](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtimeState.ts#L1-L12); [runtime.ts#L331](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtime.ts#L331-L333) | Makes the `idle` / `initialized{initParams}` states explicit | **Mixed.** The type is right, but a zustand store for a two-state machine is an odd dependency, and other lifecycle flags live beside it as loose booleans (`initializationInProgress`, `stagehandInstanceClosing`) |
| **Code-as-data** (closure serialized with `Function#toString`) | [facade/runtime.ts#L155-L163](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/core/src/facade/runtime.ts#L155-L163) | Ships a Playwright-compatible API into the worker for "code mode" agents | **Bad as built.** One 3,350-line closure that cannot import anything. A `Proxy` guard ([L334](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/core/src/facade/runtime.ts#L334-L340)) records unsupported calls. Committed JS copies have already drifted (§5) |
| **Architecture fitness functions** (cross-language AST tests) | [sdk-parity.test.ts#L103-L176](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/rules/ast-grep/sdk-parity.test.ts#L103-L176); [protocol-schema-strictness.test.ts#L7-L28](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/rules/ast-grep/protocol-schema-strictness.test.ts#L7-L28) | ast-grep parses TS, Python and Go to check two things. Every registered method is called by each SDK and routed by exactly one endpoint. Every protocol object is strict | **Good.** The most transferable idea in the repository (§7) |

## 5. Separation of concerns & DRY

**What works**

- **One contract, derived everywhere.** Zod in `schemas.ts` produces `stagehand.v4.json`. Python is generated with datamodel-code-generator plus AST post-processing (`sdk-python/scripts/generate.py`), and Go with a custom generator (`sdk-go/internal/generator/main.go`, 554 lines). CI fails on drift (§7). [protocol/README.md#L23-L31](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/README.md#L23-L31) states a clear rule: a schema goes in `protocol` if it crosses the wire, and in each SDK's client schemas if not.
- **The integration contract is defined once.** Tool descriptions, schemas, validators and the system prompt live in `integrations/core` and are "imported everywhere else, never restated" ([integrations/README.md#L1-L7](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/README.md#L1-L7)).
- **Intentional duplication is labelled and tested.** The MCP tool contract carries hand-written JSON Schema (the wire bytes) and Zod (runtime validation). A comment explains why both exist and names the drift test ([facade/contract.ts#L3-L22](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/core/src/facade/contract.ts#L3-L22)).
- **AI and browser are separated inside the extension.** The `understudy/` CDP layer has no imports from `services/`, `llm/`, `inference.ts` or `prompt.ts`.

**Where it fails** (with evidence)

1. **Shotgun surgery per RPC method.**
   - Adding a method touches about 9 places: schema, registry, `just generate`, controller, service, router `case`, the TS, Python and Go SDKs, and the docs page ([protocol/README.md#L32-L45](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/README.md#L32-L45)).
   - Parity tests catch omissions but do not remove the boilerplate. Examples are the 76-case router and `StagehandRuntime`, where 70 of 93 methods are 1:1 `page*/locator*/context*/response*` delegations ([runtime.ts#L327-L1156](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/runtime.ts#L327-L1156)).
2. **God class in the CDP layer.** `understudy/Page` (2,770 lines, about 100 members) owns many unrelated subsystems:
   - OOPIF session adoption ([L465](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/page.ts#L465)).
   - The entire WebMCP tool-tracking subsystem (about 550 lines, [L604-L1163](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/page.ts#L604-L1163); 204 WebMCP mentions in the file).
   - Navigation, screenshots ([L1511](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/page.ts#L1511)) and PDF.
   - Keyboard and macOS command normalization ([L2332-L2640](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/page.ts#L2332-L2640)).

   (For comparison, v3's top-level `V3` class was 2,381 lines: [v3 branch v3.ts](https://github.com/browserbase/stagehand/blob/09063f63d6ce26445737302ec5a110d45d9db53c/packages/core/lib/v3/v3.ts). The god object moved down a layer rather than disappearing.)
3. **Copy-paste across handlers.** `act`, `observe` and `extract` in the controller repeat the same 30-line block: initialized check, model/gateway resolution, service call, metrics record ([stagehandController.ts#L64-L151](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/controllers/stagehandController.ts#L64-L151)). A `runOperation` helper exists (L33-L41) but stops short of it.
4. **Two independent JSON-RPC peers in the same language.**
   - [sdk-ts/src/rpcClient.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/rpcClient.ts) (592 lines) and [extension/clients/rpcClient.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/clients/rpcClient.ts) (317 lines) implement the same `send/receive/receiveResponse/rejectPending/sendError` set.
   - `asError` is byte-identical in both, while `markSpanError` has already diverged: only the SDK records the RPC status code.
   - Other small duplicates: `LOG_LEVEL_PRIORITY` is defined twice ([sdk-ts stagehand.ts#L369](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/stagehand.ts#L369), [extension logger.ts#L17](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/logger.ts#L17)) instead of in `protocol`.
5. **Copied build artifacts that drifted.**
   - The deepagents integration commits two byte-identical 1,689-line copies of the built facade JS ([src/…/_assets/playwright_facade.js](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/deepagents/src/stagehand_deepagents/_assets/playwright_facade.js), [examples/managed/tools/_assets/playwright_facade.js](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/integrations/deepagents/examples/managed/tools/_assets/playwright_facade.js)).
   - Neither copy contains the TS source's `hiddenPageIds` option or its accessibility-tree role fallback, so they have drifted.
   - I found no script in the repo that regenerates them or checks for drift.
6. **Leaky wire-casing abstraction.**
   - TS works in camelCase while the wire is snake_case. The conversion is a runtime deep key transform (`camelcase-keys`/`snakecase-keys`) with a default "opaque keys" list, an acronym table (`base_url` → `base_URL`), and per-method `paramsWire`/`resultWire` overrides in the registry ([wire-casing.ts#L5-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/json-rpc/wire-casing.ts#L5-L60); [schema-registry.ts#L544-L556](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schema-registry.ts#L544-L556)).
   - Every new free-form field is a casing bug waiting to happen.
7. **Cross-language re-implementation** (inherent, but large). There are four CDP clients (sdk-ts 928 lines, Python 734, Go 1,262, extension 525), four JSON-RPC peers (592 / 602 / 1,088 / 317) and three Chrome launchers (618 / 1,131 / 558). This is mitigated by `cdp-parity` and `rpc-parity` AST tests ([rules/ast-grep/](https://github.com/browserbase/stagehand/tree/10418f98831285b3cddca5328c8013a94f22672a/rules/ast-grep)).
8. **AI concerns placed in the browser.** Model inference runs inside the extension, so provider API keys are sent into the (possibly remote, Browserbase-hosted) browser as `stagehand.init` params ([schemas.ts#L676-L686](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/schemas.ts#L676-L686)). The Anthropic client must set `anthropic-dangerous-direct-browser-access` ([aiSdkClient.ts#L154-L166](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/llm/aiSdkClient.ts#L154-L166)). This deployment choice couples AI configuration to the browser runtime.
9. **Grab-bags and dead code.**
   - `extension/utils.ts` mixes Zod schema transforms, model-provider string helpers, XPath trimming and `toTitleCase` ([L93-L397](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/utils.ts#L93-L397)).
   - The `@Instrumented` decorator ([instrumentedDecorator.ts#L14](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/instrumentedDecorator.ts#L14)) is used only by its own test. It still needs a custom TypeScript pre-transform plugin in both Vite and the root Vitest config ([instrumentedDecoratorBuild.ts#L4-L27](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/instrumentedDecoratorBuild.ts#L4-L27); [vitest.config.ts#L2-L5](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/vitest.config.ts#L2-L5)).

## 6. Contracts & extension points

- **Extension APIs.**
  - **Client LLM:** pass `model: {generate}` and the server calls back over `llm.generate`.
  - **Gateway:** if no key is set, inference goes through the Browserbase Model Gateway.
  - **Providers:** a closed enum (openai, anthropic, google, groq, cerebras).
  - There is no plugin API for custom actions, tools or handlers.
  - `agent()` was deleted in v4, and the migration guide states "Nothing in v4 replaces it one-for-one" ([docs v4/migrations/v3.mdx#L6-L20](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/docs/v4/migrations/v3.mdx#L6-L20)).
  - Agent frameworks plug in through the 3-tool MCP facade (`run`, `snapshot`, `screenshot`) and one harness adapter per framework (`integrations/*-sdk`, 390–1,528 lines each).
- **Config.**
  - SDK options are Zod-validated (`StagehandCreateOptionsSchema`, [sdk-ts/src/clientSchemas.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/clientSchemas.ts)), resolved into `Resolved*` types, and then partly forwarded as `stagehand.init` params.
  - Debug knobs are environment variables read deep inside the transport, such as `STAGEHAND_CDP_LOG`, which writes with `appendFileSync` and bypasses the logger ([cdpClient.ts#L308-L330](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/cdpClient.ts#L308-L330)).
- **Errors are the weakest contract.**
  - 26 error classes across extension (6), sdk-ts (5) and integrations (15), 28 with the CLI's two, all `extends Error` directly, with no common base and no codes.
  - There are 126 bare `throw new Error(...)` in the extension and sdk-ts outside tests.
  - On the wire, Zod failures map to `invalidParams`. Every domain error becomes JSON-RPC `-32603 internalError` with `data: {name}` ([extension/clients/rpcClient.ts#L238-L250](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/clients/rpcClient.ts#L238-L250)).
  - The TS SDK rethrows a plain `Error` and restores only the name `TimeoutError` ([sdk-ts/src/rpcClient.ts#L435-L441](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/rpcClient.ts#L435-L441)).
  - The SDKs diverge: Go exports `RPCError{Code, Message, Data}`; Python has `RPCError` but does not export it from `__init__`; TS has no RPC error class at all.
  - CDP errors are classified by message substring (`"Cannot find context with specified id"`, [frame.ts#L249,#L318](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/understudy/frame.ts#L245-L320)).
  - **This is a regression from v3**, which had a `StagehandError` hierarchy with 55 subclasses ([v3 sdkErrors.ts](https://github.com/browserbase/stagehand/blob/09063f63d6ce26445737302ec5a110d45d9db53c/packages/core/lib/v3/types/public/sdkErrors.ts)).
- **Logging and observability.**
  - The extension's `StagehandLogger` opens an OpenTelemetry span for **every log line**, even ones below the threshold, before filtering, and Zod-parses each one ([logger.ts#L97-L115](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/logger.ts#L97-L115)).
  - Logs then travel as `stagehand.log` notifications.
  - W3C `traceparent`/`tracestate` ride in the JSON-RPC envelope ([json-rpc/schemas.ts#L30-L39](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/json-rpc/schemas.ts#L30-L39)) and are extracted into a server span per request ([rpcRouter.ts#L84-L112](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/rpcRouter.ts#L84-L112)). Context propagation across the process boundary is done well.
  - On the client, the TS SDK writes every log to `process.stderr` before calling the user's `onLog` ([stagehand.ts#L377-L395](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/stagehand.ts#L377-L395)). This is open issue [#3035](https://github.com/browserbase/stagehand/issues/3035), and I confirmed it at this commit.
- **Versioning and deprecation.**
  - The protocol has its own SemVer, separate from package versions. The rule is: majors equal, server minor ≥ client minor, prereleases exact ([protocol-version.ts#L35-L66](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/protocol-version.ts#L35-L66); [README#L47-L72](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/README.md#L47-L72)).
  - Negotiation returns a discriminated union with a typed reason ([runtimeCompatibility.ts](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/runtimeCompatibility.ts)).
  - Deprecations use language-native markers: JSDoc `@deprecated`, linted by `typescript/no-deprecated: warn`, and Go `// Deprecated:`. Python marks them in docstrings only, with no `warnings.warn`.
  - Legacy surfaces stay explicit: `legacyClientModels` in the protocol document ([build-json-rpc-schema.ts#L49-L54](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/protocol/json-rpc/build-json-rpc-schema.ts#L49-L54)), and `LEGACY_RUN_TOOL_DESCRIPTION` for `--surface=legacy`.
- **Cross-language SDK contract.**
  - Generated wire models are wrapped by hand-written idiomatic object layers. The Go rationale is spelled out: "Generate the public Go wire types. Hand-write the small client, object wrappers…" ([idiomatic_go.md](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-go/idiomatic_go.md)).
  - v3 instead used Stainless-generated SDKs over a server OpenAPI spec ([docs v3/migrations/python.mdx#L4-L8](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/docs/v3/migrations/python.mdx#L4-L8)).
  - Go cannot run JS builds, so the built extension zip is committed into `sdk-go/internal/extensionassets/` with a CI drift job.
  - The cache's server-side contract lives in a private repo ("See stagehand-api-v3's lib/cache/statelessCache.ts", [cacheClient.ts#L4-L12](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/extension/clients/cacheClient.ts#L4-L12)). It is mirrored by hand in Zod, not shared.

## 7. Quality machinery

- **Formatting.** `oxfmt` at the root ignores `packages/cli/**` ([.oxfmtrc.json](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/.oxfmtrc.json)). Python uses `ruff format` and Go uses `gofmt` (CI asserts `gofmt -l` is empty, [ci.yml#L217-L218](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/.github/workflows/ci.yml#L217-L218)).
- **Lint.**
  - Type-aware oxlint adds only two standard rules plus a custom plugin on top of oxlint's defaults ([oxlint.config.ts#L1-L39](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/oxlint.config.ts#L1-L39)):
    ```ts
    rules: { "no-console": "error", "typescript/no-deprecated": "warn", ...stagehandRuleConfig },
    options: { typeAware: true },
    ```
  - The custom rules are `no-local-protocol-schema-inference`, `no-loose-json-schema-in-protocol-operations` and `no-renamed-imports` ([rules/oxlint/stagehand-plugin.ts#L1-L23](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/rules/oxlint/stagehand-plugin.ts#L1-L23)). They encode architecture: types must come from `z.infer` in `protocol/types.ts`, and protocol operations may not use loose JSON.
  - The CLI is a tooling island: it uses ESLint with `eslint-plugin-security` (`packages/cli/eslint.config.mjs`).
  - Python: ruff `select = ["ANN", "E", "F", "I", "PYI", "UP"]`, and `ty` with `unsound-return-statement`, `missing-override-decorator`, `unused-ignore-comment`, … set to `error` ([pyproject.toml#L61-L79](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-python/pyproject.toml#L61-L79)).
  - Go: `go vet` only. The repo's own Go design note asks for `staticcheck` and `go test -race` ([idiomatic_go.md#L18, #L220](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-go/idiomatic_go.md#L18)), but neither appears in CI or the justfile, and there are no `Fuzz*` tests.
- **Type-check.** `tsc --noEmit` per package through `turbo run typecheck`, at the lax settings listed in §2. Type-level tests (`*.test-d.ts`, 52 `expectTypeOf` assertions in protocol) are compiled by the package tsconfig.
- **Test types.**
  - Vitest unit tests.
  - Real-Chrome "browser-runtime" tests (`sdk-ts/tests/browser-runtime`).
  - An integration matrix discovered at CI time ([ci.yml#L513-L563](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/.github/workflows/ci.yml#L513-L563)).
  - Wire-transcript fixtures.
  - pytest, `go test`, and a Windows process-management job.
  - Contract tests for the CLI's cloud API.
  - **Architecture tests:** nine ast-grep suites (`sdk-parity`, `rpc-parity`, `cdp-parity`, `example-parity`, `sdk-field-pipeline`, `sdk-client-schema-parity`, `protocol-schema-strictness`, `go-union-strictness`, `stale-identity`). For example, "constructs every declared request field in every SDK" ([sdk-field-pipeline.test.ts#L56-L113](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/rules/ast-grep/sdk-field-pipeline.test.ts#L56-L113)).
  - **Docs-to-code tests:** "keeps every public callable in sync across TypeScript, Python, Go, and reference pages" ([docs/tests/sdk-reference.test.ts#L223](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/docs/tests/sdk-reference.test.ts#L223)).
- **Evals.**
  - `packages/evals` is a full harness: TUI, Braintrust/LangSmith/OTel, dataset suites (WebVoyager, OnlineMind2Web, …), and runner/tool-adapter pairs for about 12 agent harnesses (`framework/*Runner.ts` + `*ToolAdapter.ts`).
  - CI gates internal PRs on a **regression-eval score ≥ 80%** ([ci.yml#L684-L685](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/.github/workflows/ci.yml#L684-L685)), which can be skipped by label.
- **CI** ([ci.yml](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/.github/workflows/ci.yml)):
  - Path-based `determine-changes` gating.
  - `check` job: build, `git diff --exit-code` on `stagehand.v4.json` (L199), `generate.py --check` (L205) and Go `generator run . --check` (L215).
  - The "Extension drift (Go embed vs extension build)" job (L446-L477).
  - `uv lock --check`, ruff, ty and `go vet`.
  - External-fork PRs are mirrored into internal PRs by a 30 KB workflow (`external-contributor-pr.yml`).
- **Release.**
  - Changesets with independent versions.
  - A custom validator restricts which packages may appear and forbids mixing `browse` with SDK releases in one changeset ([scripts/release/check-changesets.ts#L1-L40](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/scripts/release/check-changesets.ts)).
  - Release scripts have unit tests.
  - Preview builds are triggered by a `preview` label.
- **Contribution rules.**
  - [CONTRIBUTING.md](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/CONTRIBUTING.md) covers the architecture, `just fmt/check/test`, "never hand-edit generated", the protocol-change sequence and a changeset decision table.
  - `AGENTS.md` only covers stacked-PR mechanics.
  - The PR template asks for "why / what changed / test plan".
  - Cubic AI review applies a docs prose style guide ([cubic.yaml](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/cubic.yaml)).
  - Design docs exist only per package: protocol README, `idiomatic_go.md`, evals `ARCHITECTURE.mmd`, `harness-contract.md`. There are no ADRs.
  - Documentation drift exists: [sdk-ts/src/browser/README.md#L17-L20](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/sdk-ts/src/browser/README.md#L17-L20) says the browser API "remains TypeScript-local for v4", but Python and Go now ship launchers.

## 8. Pain points

**What maintainers say**

- **The v4 rewrite was a clean break.**
  - [PR #2671 "Stagehand v4"](https://github.com/browserbase/stagehand/pull/2671) changed +142,872 / −150,757 lines across 1,522 files.
  - Main became "a protocol-first monorepo with three SDKs over a shared core".
  - The v3 tree ("`core`, `cli`/browse, `server-v3`) and its 14 workflows" moved to a protected `v3` branch, and anything still needed is "ported in follow-up PRs".
  - v4 was built in a separate spike repo and imported first as a nested, isolated pnpm workspace ([PR #2382](https://github.com/browserbase/stagehand/pull/2382)). That PR also removed an earlier `server-v4` design.
- **Reasons given for removing `agent()`.** It "was built for models that couldn't reliably drive a browser on their own". The guide recommends "code mode" or tool calling instead ([v4/migrations/v3.mdx#L12-L20](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/docs/v4/migrations/v3.mdx#L12-L20)).
- **The architecture has been rebuilt three times.**
  - v2 depended on Playwright.
  - v3 announced "Removing Playwright Dependency" ([v3/migrations/v2.mdx#L32](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/docs/v3/migrations/v2.mdx#L32)), and Python became a "Stainless-based" pure API client.
  - v4 moved the runtime into an extension with its own codegen.
- **Open issues on v4 (user and auditor reports):**
  - [#2782](https://github.com/browserbase/stagehand/issues/2782): `Stagehand.create` times out with the official extension preloaded on Browserbase. Extension discovery and handshake are a new failure surface.
  - [#2945](https://github.com/browserbase/stagehand/issues/2945) and [#3002](https://github.com/browserbase/stagehand/issues/3002): domain policy is bypassed by service-worker and WebSocket traffic, and fails open until it is set.
  - [#2725](https://github.com/browserbase/stagehand/issues/2725): the Go SDK leaks an uploaded extension on one validation path. Lifecycle cleanup diverges between language ports, and the parity tests do not cover it.
  - [#3035](https://github.com/browserbase/stagehand/issues/3035): logging to stderr cannot be disabled when `onLog` is set.
  - [#3152](https://github.com/browserbase/stagehand/issues/3152): Deno extension upload fails.
  - From v3: [#1390](https://github.com/browserbase/stagehand/issues/1390), a `process.once('unhandledRejection')` handler that crashes on the second CDP error.

**My inferences from the code** (not maintainer statements)

- **Migration debt in evals.**
  - `evals` still depends on the previous major through an npm alias, `"stagehand-v3": "npm:@browserbasehq/stagehand@3.7.1"` ([evals/package.json#L53](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/evals/package.json#L53)).
  - 113 evals TS files import from it, and agent bench tasks use "the stagehand-v3 SDK" ([defineTask.ts#L49-L60](https://github.com/browserbase/stagehand/blob/10418f98831285b3cddca5328c8013a94f22672a/packages/evals/framework/defineTask.ts#L49-L60)).
  - The benchmark harness therefore measures parts of a product that is no longer on `main`.
- **The error model regressed** (§6). Users must string-match messages.
- **Strictness is uneven.** Evals is the largest package at 74k code lines, and it is the least strict (`strict: false`). The CLI uses a different linter.
- **Code-as-string distribution** (the facade closure and the callback batch) makes the most agent-facing code the hardest to modularize, and its copies have already drifted.

## 9. Scorecard

| Dimension | Score | Justification |
|---|---|---|
| Layering & dependency direction | 4 | The package DAG is clean: protocol, then extension/sdk-ts, then integrations, then evals. The CDP layer never imports AI code. The one violation is `callbackBatch.ts` importing `../sdk-ts/src` without declaring it (§3) |
| Module/file size discipline | 3 | Mean 168 lines, but 15 files exceed 1,000 lines, including a 3.5k-line single closure, a 2.8k-line `Page` with about 100 members, and a 93-method runtime facade |
| DRY (incl. across language ports) | 3 | Contract defined once, generated, with drift and parity checks. But: 76-case router boilerplate, copy-pasted controllers, two TS RPC peers, duplicated level tables, drifted committed JS copies, and four CDP/RPC re-implementations across languages (tested, not shared) |
| Separation of concerns | 3 | `services/`, `understudy/`, `controllers/` and `llm/` are split well and the snapshot pipeline is clean. `Page` and `StagehandRuntime` are god objects, AI runs inside the browser with keys shipped in, and logging is entangled with tracing |
| Extension model | 3 | Reverse-RPC client LLM and an MCP facade with per-harness adapters exist. The provider enum is closed, there is no action or tool plugin API, and `agent()` was removed without a built-in replacement |
| Contracts & types | 4 | Strict Zod schemas as source of truth, a JSON Schema artifact, codegen drift checks, protocol SemVer and typed negotiation. Weakened by `noImplicitAny: false` and runtime key-casing transforms |
| Error model | 2 | Flat classes with no base and no codes. Everything crosses the wire as `-32603` plus a name, errors are classified by message substring, the TS, Python and Go surfaces are inconsistent, and v3's typed hierarchy was lost |
| Test architecture | 4 | Layered unit, type, real-browser, integration, wire-fixture and eval-gate tests, plus cross-language architecture and docs-sync tests. Test files are huge (up to 2.9k lines) and the root include list is maintained by hand |
| Tooling/CI enforcement | 4 | Generated-artifact drift checks for schema, Python, Go and the embedded extension, a changeset validator, and an eval threshold. Minimal lint rule sets, lax TS strictness, Go limited to `vet`, and a separate CLI toolchain |
| In-code documentation | 3 | Strong module headers explain intent (`cdp.ts`, `capture.ts`, `cacheClient.ts`, `contract.ts`). Comment density is low (2.3% in `sdk-ts/src`, 2.0% in protocol), the 7.7k-line JSON Schema has 63 `description`s, and stale `// lib/v3/...` headers remain |

## 10. Lessons for Goodfetch

1. **Adopt: drive dispatch from the protocol registry, not just codegen.**
   - Stagehand's registry feeds Python and Go generation, yet its router is 76 hand-written `case`s and its runtime has 70 pass-through methods (§4, §5.1).
   - In `goodfetch-protocol`, the method table should also generate the server-side dispatcher and the handler trait, so adding a method means a schema plus one handler and the compiler finds the rest.
2. **Adopt: architecture fitness tests across languages and docs.**
   - Stagehand's ast-grep suites (every method called by every SDK, every field constructed and consumed, docs reference parity, schema strictness, retired identities) are the best idea in the repo (§7).
   - Goodfetch's `xtask codegen` should come with equivalent checks over the generated SDKs and the docs, plus `--check` drift gates for every committed artifact.
3. **Adopt: version the protocol independently, with a typed handshake.** Stagehand separates protocol SemVer from package versions, applies an explicit compatibility rule, and reports incompatibility as a discriminated reason (§6). This fits ADR-gated releases. Capability negotiation should stay in runtime settings, never compiled-in enums (Stagehand's closed provider enum forces releases).
4. **Avoid: an error model that collapses at the wire.**
   - Define a closed, serde-tagged error-code enum in the protocol crate, with structured data, and map it to each SDK's idiom (Python exceptions, Go `errors.As`, TS classes).
   - Never classify CDP errors by message substring. Wrap CDP `code` and `message` into typed variants once, at the transport.
   - Stagehand lost v3's 55-class hierarchy and now leaks `-32603` plus a name (§6).
5. **Avoid: runtime key-casing transforms.** Stagehand's camel-to-snake deep conversion with opaque-key lists and acronym tables is fragile (§5.6). Fix wire names at the type level once (`#[serde(rename_all = "snake_case")]` plus explicit renames) and treat user maps as opaque types, not key lists.
6. **Avoid: god objects in the CDP layer. Set a size budget and enforce it.**
   - Split a page into collaborators (frames/OOPIF, input/keyboard, capture, network, per-feature trackers such as WebMCP), with the page as a thin aggregate.
   - Lint the budget, for example with a clippy `too_many_lines` threshold or an xtask check. Stagehand's `Page` and `StagehandRuntime` show what happens without one.
7. **Adopt: one deadline object per operation, and fail-open intercepts.**
   - Stagehand's `Progress` (one shared deadline, named phases, late-result cleanup) maps directly to a Tokio `Deadline` + `CancellationToken` passed through every step.
   - `withCache` shows how to put an optional subsystem (caching, self-heal) behind one intercept that can never fail the primary operation.
8. **Avoid: shipping logic as serialized source and committing copied artifacts.** If Goodfetch injects JS into pages or workers, build it from modular source in-tree, embed it at build time (`include_str!` of a build output), and drift-check it as Stagehand does for the Go-embedded extension. Do not do what the deepagents facade copies did (§5.5).
9. **Adopt the propagation, avoid the coupling, for observability.**
   - Carry `traceparent` in the protocol envelope and open a server span per request: that part Stagehand does well.
   - Do not make every log line a span or validate logs on the hot path.
   - Route SDK output only through `tracing` or the user's sink, never unconditionally to stderr (Stagehand issue #3035). This matches Goodfetch's stdout/stderr rule.
10. **Avoid: uneven strictness and tooling islands.** Stagehand runs `noImplicitAny: false` in its core, `strict: false` in evals, ESLint for the CLI only, and Go with `vet` only despite its own design doc. Apply one strictness bar (`clippy -D warnings` plus pedantic groups, the same lint set for generated SDK wrappers) across every crate and SDK, including test-support and eval code.
