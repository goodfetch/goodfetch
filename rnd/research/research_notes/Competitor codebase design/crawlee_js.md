# Crawlee for JavaScript/TypeScript (apify/crawlee): codebase design notes

Read-only study of the source, its config, CI, docs and issue tracker. The permalink base for citations is
`https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/`, and every `file:line` link below points into it.
"Maintainers say" marks a statement made in the repo or tracker; anything else is my reading of the code.

## 1. Snapshot

- **Repo / commit**: `apify/crawlee` at `02899241d570624a0ce0b77dbcb44b9d58a98ade`, committed 2026-10-15 17:19 +0200 on `master`.
  `master` now carries **v4 in its RC phase**: v3 moved to a `3.x` maintenance branch, and rc.1 shipped on 2026-10-06 ([#4047](https://github.com/apify/crawlee/issues/4047)).
  All workspace packages are `4.0.0` except `@crawlee/stagehand` (`3.17.0`), and `lerna.json` still says `3.18.1` ([lerna.json:5](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/lerna.json#L5)).
- **Languages** (tokei, excluding `node_modules` and `website/versioned_docs`): TypeScript has 484 files and 91,520 lines (60,430 code). There are also 144 JavaScript files (7.1k lines), 109 Markdown files (27.3k lines) and 66 MDX files (6.9k lines). The 27.3k YAML lines are almost all `pnpm-lock.yaml`.
- **Shipped source** (`packages/*/src`): 203 TS files and 44,389 lines, of which 23,945 are code, **15,145 are comments (34%)** and 5,299 are blank.
  **Tests** (`test/`): 149 TS files, 43,383 lines and 33,941 code lines, so there is **1.4× more test code than shipped code**. 133 of these are `*.test.ts` files.
  **Docs** (`docs/` plus `website/src`): 318 files and 25.9k lines.
- **Packages**: 19 workspace packages, all published to npm (pnpm workspaces + Turbo + Lerna, [pnpm-workspace.yaml](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/pnpm-workspace.yaml), [turbo.json](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/turbo.json)).
  `@crawlee/jsdom` and `@crawlee/linkedom` were moved to their own repos for 4.0 ([#4113](https://github.com/apify/crawlee/issues/4113)).

| package dir (npm name) | src files | lines | code | | package dir | src files | lines | code |
|---|---|---|---|---|---|---|---|---|
| basic-crawler (`@crawlee/basic`) | 39 | 12,162 | 6,599 | | types | 9 | 1,127 | 393 |
| core | 48 | 11,219 | 5,781 | | fs-storage | 7 | 937 | 573 |
| playwright-crawler | 9 | 3,609 | 1,940 | | otel | 7 | 774 | 527 |
| browser-pool | 21 | 3,431 | 2,018 | | cli | 4 | 334 | 260 |
| utils | 15 | 3,036 | 1,715 | | http-client | 4 | 275 | 180 |
| puppeteer-crawler | 7 | 2,391 | 1,129 | | cheerio-crawler | 3 | 243 | 122 |
| http-crawler (`@crawlee/http`) | 5 | 1,672 | 982 | | impit-client | 1 | 137 | 83 |
| stagehand-crawler | 7 | 1,408 | 656 | | templates (+11 template projects) | 1 | 76 | 63 |
| browser-crawler (`@crawlee/browser`) | 3 | 1,280 | 730 | | got-scraping-client | 1 | 64 | 48 |
| | | | | | crawlee (meta-package) | 2 | 17 | 14 |

**10 largest source files** (`wc -l`). Of the 203 files, 24 are over 500 lines and 8 are over 1,000.

| # | file | lines |
|---|---|---|
| 1 | [basic-crawler/src/internals/basic-crawler.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts) (1,836 code + 930 comment lines) | 3,180 |
| 2 | [basic-crawler/src/internals/throttling-request-manager.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/throttling-request-manager.ts) | 1,213 |
| 3 | [core/src/storages/request-queue.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/request-queue.ts) | 1,205 |
| 4 | [browser-pool/src/browser-pool.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-pool/src/browser-pool.ts) | 1,163 |
| 5 | [core/src/storages/key-value-store.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/key-value-store.ts) | 1,064 |
| 6 | [playwright-crawler/src/internals/utils/playwright-utils.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/playwright-crawler/src/internals/utils/playwright-utils.ts) | 1,041 |
| 7 | [playwright-crawler/src/internals/adaptive-playwright-crawler.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts) | 1,026 |
| 8 | [http-crawler/src/internals/http-crawler.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts) | 1,010 |
| 9 | [core/src/storages/request-list.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/request-list.ts) | 973 |
| 10 | [puppeteer-crawler/src/internals/utils/puppeteer-utils.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/puppeteer-crawler/src/internals/utils/puppeteer-utils.ts) | 949 |

## 2. Paradigm

- **Runtime and concurrency**
  - Crawlee runs on Node.js (≥22) and uses its single-threaded event loop with async/await.
  - Parallelism comes from a task loop. `AutoscaledPool`, now `@internal`, books each task against a `ConcurrencySystem`, a shared "governor" that scales its budget from pluggable `LoadSignal`s for CPU, memory, event loop and storage backend ([concurrency-system.ts:203-218](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/autoscaling/concurrency-system.ts#L203-L218), [load-signal.ts:36-66](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/autoscaling/load-signal.ts#L36-L66)).
  - Sharing one `ConcurrencySystem` caps the combined load of several crawlers.
  - Cancellation is cooperative and timeout-driven: `addTimeoutToPromise` wraps the work, and `tryCancel()` checkpoints are sprinkled through the code ([browser-crawler.ts:556](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-crawler/src/internals/browser-crawler.ts#L556)). There are only about 18 `AbortSignal`/`AbortController` mentions in all of `src`.
  - `AsyncLocalStorage` carries ambient state: the scoped service locator and the per-request storage transaction ([transaction.ts:156](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/transaction.ts#L156)).
- **Style: an inheritance spine with composition inside it**
  - Inheritance chain on the HTTP side: `BasicCrawler` → `HttpCrawler` → `DOMCrawler` → `CheerioCrawler`.
  - Inheritance chain on the browser side: `BasicCrawler` → abstract `BrowserCrawler` → `PlaywrightCrawler` / `PuppeteerCrawler` / `StagehandCrawler`.
  - `AdaptivePlaywrightCrawler` extends `BasicCrawler` and *composes* an inner `CheerioCrawler` and an inner `PlaywrightCrawler`, reusing their context pipelines ([adaptive-playwright-crawler.ts:484-511](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L484-L511)).
  - v4 moved much of the variation out of overrides and into a typed middleware `ContextPipeline`, injectable collaborators behind 12 `I*` interfaces, and option objects. For example, `Statistics` subclassing was replaced by a `stateExtension` option ([statistics.md:44](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/upgrading/upgrading-v4/statistics.md?plain=1#L44)).
  - Helpers are module-level functions; the browser utils are free functions over a `page`.
- **Public API style**: the user picks a crawler class, passes a single options object holding `requestHandler` (or a callable `Router`), and calls `await crawler.run(urls)`. The handler destructures a context bag (`{ request, page, enqueueLinks, pushData, log }`); see [README.md](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/README.md). The options are validated at runtime with zod `strictObject` schemas, so unknown keys throw.
- **Typing strictness**
  - The shared `@apify/tsconfig` 0.2.0 sets `strict`, `noImplicitOverride`, `noImplicitReturns` and `noUnusedLocals`/`Parameters`. It does *not* set `noUncheckedIndexedAccess` or `exactOptionalPropertyTypes` ([apify-tsconfig/tsconfig.json](https://github.com/apify/apify-tsconfig/blob/0b2f65f5333dee2837ef09a3fe0e68076e03227b/tsconfig.json)).
  - The compiler is TypeScript 7 (`typescript: ^7.0.2`, with a `typescript-v6` alias also present).
  - The generics are heavy. `BasicCrawler` takes 5 type parameters (`Context, ContextExtension, ExtendedContext, Routes, StatisticStateExtension`, [basic-crawler.ts:706-712](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L706-L712)), and `BrowserCrawler` takes 8.
  - Escape hatches in `src`: `typescript/no-explicit-any` is off, and I count 40 `as any`, 58 `: any`, 39 `as unknown as` and 4 `@ts-ignore`/`@ts-expect-error`.

## 3. Organization

- **Top-level layout**:

  | path | contents |
  |---|---|
  | `packages/` | 19 packages, each with `src/index.ts` plus `src/internals/**` |
  | `test/` | all tests, centralized at the root |
  | `docs/` | guides and examples, with the examples type-checked |
  | `docs/public-api/` | API-Extractor reports |
  | `docs/upgrading/` | upgrade guides |
  | `website/` | Docusaurus site, plus `versioned_docs` for 3.10–3.18 (1,575 files) |
  | `scripts/` | build/publish scripts and the custom lint plugin |
  | `.github/workflows/` | 10 workflows |
  | `.claude/CLAUDE.md` | instructions for AI agents |

- **Dependency direction**. I took this from `package.json` and checked it against the actual `from '@crawlee/...'` imports. The graph is an acyclic DAG:

  ```
  types ← http-client ← utils ← fs-storage ← core ← basic ← http ← cheerio
                                        core ← browser-pool ← browser ← {playwright, puppeteer, stagehand}
  playwright → cheerio (adaptive crawler)   impit-client → core   basic ⇢ impit-client (optional, dynamic import)
  crawlee (meta) → basic, browser, cheerio, core, fs-storage, http, playwright, puppeteer, utils
  ```

  Anomalies:
  - `core` imports the *concrete* `FileSystemStorageBackend` in order to pick a default ([service-locator.ts:3](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/service-locator.ts#L3), [:249-256](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/service-locator.ts#L249-L256)).
  - `impit-client` depends on `core` only for the ambient `serviceLocator` logger ([impit-client/src/index.ts:1](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/impit-client/src/index.ts#L1)). `browser-pool` depends on `core` for the logger, `SessionError`, `CriticalError` and `serviceLocator`.
  - `utils` is not a leaf: it fetches sitemaps and robots.txt through `@crawlee/http-client` ([utils sitemap.ts:7](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/utils/src/internals/sitemap.ts#L7)).
  - `basic` reaches `impit-client` through a dynamic `import()` that static scans miss ([basic-crawler.ts:113-137](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L113-L137)).
  - There is no boundary checker (no dependency-cruiser, madge or knip) in devDependencies or CI.
  - The repo's own `.claude/CLAUDE.md` draws a simplified version of this hierarchy that omits the `core → fs-storage` and `utils → http-client` edges.
- **Intra-package import cycles**. I found these with a Tarjan SCC script over runtime (non-`import type`) relative imports:
  - In `core`: `configuration ↔ service-locator ↔ local-event-manager ↔ event-manager`, and `storages/key-value-store ↔ storages/utils`.
  - In `utils`: `robots ↔ sitemap`.
  - In `http`: `file-download.ts` imports its own package barrel ([file-download.ts:20-21](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/file-download.ts#L20-L21)).
- **Public and internal API are separated three ways**:
  1. A `./internal` subpath export in `core` and `utils` ([utils/package.json:9-13](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/utils/package.json#L9-L13)), with a comment that explains why `core/index.ts` avoids `export *` so semver-free symbols don't leak ([core/src/index.ts:13-15](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/index.ts#L13-L15), [core/src/internal.ts:1](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/internal.ts#L1)).
  2. `@internal` TSDoc tags, 239 of them.
  3. Committed API-Extractor reports that CI checks, plus a written policy: anything exported is promised unless it is tagged `@internal` ([docs/public-api/README.md](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/public-api/README.md), [docs/guides/public-api.mdx](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/guides/public-api.mdx)).

  Counter-pattern: every layer re-exports its parent wholesale. `@crawlee/basic` has `export * from '@crawlee/core'`, `@crawlee/playwright` has `export * from '@crawlee/browser'`, and so on, so one symbol is importable from 4–5 packages ([basic-crawler/src/index.ts:1](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/index.ts#L1), [crawlee/src/index.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/crawlee/src/index.ts)).
- **Naming conventions**
  - Kebab-case file names are lint-enforced ([oxlint.config.ts:114](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/oxlint.config.ts#L114)).
  - Native `#private` is required instead of `_x` or TS `private` (a custom rule, [oxlint.config.ts:37-62](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/oxlint.config.ts#L37-L62)).
  - Extension interfaces are `I`-prefixed (`IRequestManager`, `ISessionPool`, `IBrowserPool`, `IStatistics`, `IConcurrencySystem`, …).
  - Package directories are named `*-crawler` but published as `@crawlee/basic`, `@crawlee/http`, and so on.
- **Tests, fixtures and docs**
  - Tests live in `/test/<area>`, not next to their package. `test/core/crawlers/` holds the Cheerio, Playwright, Puppeteer and adaptive crawler tests, and `test/shared/` holds helpers and data.
  - E2E scenarios are 48 mini "actor" projects ([test/e2e](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/test/e2e)).
  - Doc examples are `.ts` files that the MDX imports.
- **File-size discipline**: the median file is small, but there is no budget. 8 files exceed 1k lines, and the `BasicCrawler` constructor alone spans [basic-crawler.ts:972-1371](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L972-L1371) (400 lines).

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| Middleware pipeline (chain of responsibility + cleanup stack) | [context-pipeline.ts:37-97](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawlers/context-pipeline.ts#L37-L97), [:158-214](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawlers/context-pipeline.ts#L158-L214) | Builds each crawler's context step by step, typed (`compose<Ext>` returns `Ctx & Ext`). Cleanups run in reverse. Setup failures (`ContextPipelineInitializationError`) are told apart from handler failures (`RequestHandlerError`). | **Good** idea, ported from crawlee-python ([#2479](https://github.com/apify/crawlee/issues/2479)). It removed the v3 "hard to keep track of" context building. Weak spots: it mutates one object through `Object.defineProperty`, it swallows define failures at `debug` level ([:189-193](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawlers/context-pipeline.ts#L189-L193)), and it needs `as any` in `compose`. |
| Service locator + ambient scope (`AsyncLocalStorage` + `Proxy`) | [service-locator.ts:320-408](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/service-locator.ts#L320-L408), used at [basic-crawler.ts:1043-1070](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1043-L1070) | Supplies config, events, storage and logger without threading them through every constructor. A crawler can get its own services. | **Mixed/bad**. The per-crawler isolation works by *overwriting every prototype method on the instance* with a wrapper that enters `AsyncLocalStorage` ([:339-392](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/service-locator.ts#L339-L392)). There are 75 `serviceLocator.` call sites in 36 files, so the dependencies are invisible in signatures. The set-once semantics took four iterations in the Python port before JS adopted them ([#3710](https://github.com/apify/crawlee/issues/3710)). |
| Inject-or-default ownership wrapper | [owned-or-injected.ts:12-116](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/owned-or-injected.ts#L12-L116) | Decides who tears down a collaborator (session pool, browser pool, request queue): user-injected means borrowed, crawler-built means owned. `ifOwned()` gates the lifecycle hooks. | **Good**. Explicit and typed (`Injected` vs `Owned` subtype), and documented as the deliberate complement to the locator. |
| Template method | [browser-controller.ts:221-225](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-pool/src/abstract-classes/browser-controller.ts#L221-L225), [:311-319](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-pool/src/abstract-classes/browser-controller.ts#L311-L319); [base-http-client.ts:52-63](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-client/src/base-http-client.ts#L52-L63) | Public `close()`/`newPage()`/`sendRequest()` hold the shared logic (state, cookies, redirects), and subclasses implement `_close`, `_newPage` or `fetch`. | **Good**. HTTP adapters come out at 48–83 code lines (impit, got-scraping, fetch). It needs a lint allow-list for the `_` names ([oxlint.config.ts:46-57](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/oxlint.config.ts#L46-L57)). |
| Plugin / abstract factory | [browser-plugin.ts:99-178](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-pool/src/abstract-classes/browser-plugin.ts#L99-L178) (`createController`, abstract `_launch`, `addProxyToLaunchOptions`) | Supports several browser libraries (Playwright, Puppeteer, Stagehand, remote) behind one pool. | **Good** at the pool layer. The crawler layer above it is still duplicated per library (§5). |
| Lifecycle hook lists | [browser-pool.ts:222-277](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-pool/src/browser-pool.ts#L222-L277) (pre/post launch, page create and page close); crawler `pre/postNavigationHooks` ([http-crawler.ts:493-507](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L493-L507)) | User customization without subclassing. | **Good**. In v4 the nav hooks run as pipeline steps that share one navigation-timeout window. |
| Strategy | [load-signal.ts:36-66](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/autoscaling/load-signal.ts#L36-L66) (CPU, memory, event loop, storage backend); `IRenderingTypePredictor`; `createSessionFunction` | Swappable overload detection and policy. | **Good**: a small interface (`name`, `overloadedRatio`, `start/stop`, `getSample`). |
| Decorator / composite over one interface | [throttling-request-manager.ts:269](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/throttling-request-manager.ts#L269), [request-manager-tandem.ts:21](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/request-manager-tandem.ts#L21) | Per-domain throttling, and list+queue combination, both wrapping any `IRequestManager`. `recordPacingSignal` is forwarded through the wrappers. | **Good** composition. The decorator has grown to 1,213 lines. |
| Unit of work (journaled transaction) | [transaction.ts:156-171](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/transaction.ts#L156-L171), [:407](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/transaction.ts#L407), [:457](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/storages/transaction.ts#L457) | A handler's dataset, KV and queue writes are buffered and committed only on success, so a retry does not duplicate output. | **Good** semantics. **Mixed** plumbing: the transaction is found through `AsyncLocalStorage` rather than passed in, and `afterStorageCommit` throws when none is active ([basic-crawler.ts:1437-1449](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1437-L1449)). |
| Dispatch table (callable router) | [router.ts:281-534](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/router.ts#L281-L534), factory [:492-533](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/router.ts#L492-L533) | Routes by `request.label`, validates `userData` with a Standard Schema per label, supports per-route timeouts, and infers types from a route map. | **Mixed**. The typing and validation are strong. But the router is a function whose prototype is rewired (`Object.setPrototypeOf(func, obj)` at [:530](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/router.ts#L530)), and it has 3 overloads for back-compat. |
| Observer / event bus | [event-manager.ts:60-121](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/events/event-manager.ts#L60-L121) | Periodic `persistState`/`systemInfo`, platform `migrating`/`aborting`, status messages. | **Mixed**. Payloads are untyped (`listener: (...args: any[]) => any`, `emit(event, ...args: unknown[])`), and the logger is grabbed from the locator in a field initializer. |
| Layered schema config, immutable | [configuration.ts:47-77](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/configuration.ts#L47-L77), [:202-234](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/configuration.ts#L202-L234) | One field table (`field(zodSchema, 'CRAWLEE_X')`) drives precedence (constructor > env > `crawlee.json` > default), coercion, accessors and docs. It is subclassable through `protected static fields` ([:167](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/configuration.ts#L167)). | **Good**. One flaw: the constructor mutates the *global* log level ([:185](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/configuration.ts#L185)), which leaks one instance's setting process-wide. |
| Per-invocation run object | [crawler-run.ts:30](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawler-run.ts#L30) | A fresh object per `run()`, so per-run state cannot leak, "no reset checklist to keep in sync". | **Good**, and introduced late ([#3087](https://github.com/apify/crawlee/issues/3087)). |
| Factory method for defaults | [basic-crawler.ts:1381](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1381), overridden in [http-crawler.ts:461](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L461) | Lets a subclass tune the default `ConcurrencySystem` while keeping the user's shortcuts. | **Good**, narrow seam. |
| Instrumentation by monkey-patching | [otel/src/constants.ts:56-140](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/otel/src/constants.ts#L56-L140) | OpenTelemetry spans without touching the core. | **Bad**. It patches *private* methods by name (`handleRequest`, `requestFunctionErrorHandler`, …). The crawler packages have to keep them as TS `private` with `oxlint-disable … -- patched by @crawlee/otel` ([basic-crawler.ts:2602](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2602), [:2824](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2824), [:2947](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2947), [browser-crawler.ts:659](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-crawler/src/internals/browser-crawler.ts#L659), [http-crawler.ts:540](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L540)). That is a hidden cross-package contract, and `AdaptivePlaywrightCrawler` needs its own entry because it does not call `super`. |
| Lazy proxy for an optional dependency | [basic-crawler.ts:113-137](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L113-L137); [fs-storage file-system-storage.ts:20-27](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/fs-storage/src/file-system-storage.ts#L20-L27) | Uses impit when installed and falls back to `fetch` with a warning. Loads the Rust napi storage binary only when it is used, so importing `core` does not crash on platforms without the binary. | **Mixed**. The runtime capability fallback is pragmatic, but `LazyDefaultHttpClient.fetch()` throws, which violates LSP, and the graph edge is invisible to tooling. |

## 5. Separation of concerns & DRY

**How they avoid duplication**
- **Template-method bases** keep the adapters tiny (the HTTP clients and browser controllers above).
  - `@crawlee/types` is types-only (`export type *`, [types/src/index.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/types/src/index.ts)) and holds the storage, session, browser-pool and logger contracts that both the frontends and the backends implement.
  - The storage backend contract is four interfaces ([types/src/storages.ts:438-486](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/types/src/storages.ts#L438-L486)).
  - `MemoryStorageBackend` (in core) and `FileSystemStorageBackend` (Rust-backed) share little code: 13–26% line similarity by my diff script. That is fine, because they are genuinely different.
- **Option schemas compose by spread**: each subclass's `optionsShape` is `{...Parent.optionsShape, …}` ([http-crawler.ts:384-396](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L384-L396), [browser-crawler.ts:376-393](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-crawler/src/internals/browser-crawler.ts#L376-L393)).
- **Pipelines are reused by composition**: the adaptive crawler extends the inner crawlers' `contextPipeline` instead of re-implementing it.
- **Across language ports**: JS and Python are separate, hand-maintained codebases. Designs are ported by hand in both directions: ContextPipeline went Python→JS ([#2479](https://github.com/apify/crawlee/issues/2479)), and so did the service locator ([#3073](https://github.com/apify/crawlee/issues/3073)).
  - The only *shared code* is the Rust file-system storage: one Cargo workspace with `napi-rs` (→ `@crawlee/fs-storage-native`) and `maturin` (Python) bindings ([apify/crawlee-storage Cargo.toml](https://github.com/apify/crawlee-storage/blob/5bea28ede93f81ee27cc6ee4298dd2a608aa4472/Cargo.toml)).
  - The new early-stage Rust port keeps behavioural parity with **golden files recorded by running Crawlee-JS from source as an oracle** ([crawlee-rs/conformance](https://github.com/apify/crawlee-rs/tree/a160072b977a45c328614386b8ef49cbc1b41d9f/conformance)).

**Where they fail (with evidence)**
1. **Playwright and Puppeteer are parallel copies.** My diff script (`difflib`, code lines only, library names normalized) gives these matches:
   - `click-elements.ts` (Playwright vs Puppeteer): **82%**, 235 of ~290 code lines.
   - `playwright-crawler.ts` vs `puppeteer-crawler.ts`: 71%.
   - `*-utils.ts`: 54%, with the same 8 exported functions (`injectFile`, `injectJQuery`, `gotoExtended`, `blockRequests`, `compileScript`, `infiniteScroll`, `saveSnapshot`, `parseWithCheerio`).
   - `*-browser-pool.ts`: 81%.

   **The copies have drifted.** `infiniteScroll` stops at `scrolledDistance >= maxScrollHeight` in Playwright ([playwright-utils.ts:452](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/playwright-crawler/src/internals/utils/playwright-utils.ts#L452)) but at `>` in Puppeteer ([puppeteer-utils.ts:575](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/puppeteer-crawler/src/internals/utils/puppeteer-utils.ts#L575)), and only Puppeteer centres the mouse first ([:536](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/puppeteer-crawler/src/internals/utils/puppeteer-utils.ts#L536)). The browser-pool plugin layer abstracts the *driver*, but no shared "page operations" trait sits above it.
2. **The "navigation phase" is implemented twice.** `HttpCrawler` and `BrowserCrawler` both extend `BasicCrawler` directly, and each re-declares the same pieces:
   - `skipGuard`/`windowGuard` ([http-crawler.ts:471-490](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L471-L490) vs [browser-crawler.ts:439-466](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-crawler/src/internals/browser-crawler.ts#L439-L466)).
   - `#handleBlockedRequestByContent` ([http:703](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/http-crawler/src/internals/http-crawler.ts#L703), [browser:763](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/browser-crawler/src/internals/browser-crawler.ts#L763)).
   - The `navigationTimeoutSecs`, `saveResponseCookies` and pre/post hook options.

   The validation has drifted: browser refines `navigationTimeoutSecs > 0` and HTTP does not.
3. **Options have two sources of truth.** The hand-written `BasicCrawlerOptions` interface (~480 lines of TSDoc, from [basic-crawler.ts:223](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L223)) sits next to a separate zod `optionsShape` ([:908-965](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L908-L965)). The shape mostly checks kind only (`anyObject`, `anyFunction`, `anyNumber`; [utils schemas.ts:9-32](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/utils/src/internals/schemas.ts#L9-L32)), so the types are not inferred from the schema and the two must be kept in sync by hand.
4. **`BasicCrawler` is a god class**: 3,180 lines and a 400-line constructor. It owns:
   - the per-crawler locator scope and request-manager composition (with throttling and tandem);
   - robots.txt ([:2474](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2474)) and the rate-limit bookkeeping ([:2499](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2499));
   - status-message logging ([:1614](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1614)), and data export to CSV/JSON (`pushData`/`exportData`, [:2241-2271](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2241-L2271));
   - the retry and error policy, plus the storage transactions.

   v4 extracted `ConcurrencySystem`, `CrawlerRun`, `ThrottlingRequestManager` and `SessionPool`/`Statistics` behind interfaces. A 2026-09-03 maintainer comment listed per-run state still left on the instance, along with a real bug: `BrowserCrawler.teardown()` destroyed the pool on every run ([#3087 comment](https://github.com/apify/crawlee/issues/3087)). [PR #4111](https://github.com/apify/crawlee/pull/4111), which extracted `CrawlerRun`, merged on 2026-09-08 and closed the issue.
5. **Ambient coupling leaks through abstractions.** The service locator turns "needs a logger" into a package dependency on `core` (impit-client, browser-pool). The abstraction package (`core`) depends on a concrete adapter (`fs-storage`), which in turn has to lazy-load its native module so that importing `core` stays safe ([file-system-storage.ts:20-23](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/fs-storage/src/file-system-storage.ts#L20-L23)).
6. **The tooling map is repeated three times.** The same `@crawlee/*` → `src` alias table appears in [tsconfig.json](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/tsconfig.json), [test/tsconfig.json](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/test/tsconfig.json) and [vitest.config.mts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/vitest.config.mts). Separately, a post-build script rewrites the emitted `.d.ts` and inserts `// @ts-ignore` for optional peers. One of its cases carries the comment "don't ask me why, but this one is needed too" ([scripts/typescript-fixes.mjs:24-33](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/scripts/typescript-fixes.mjs#L24-L33)).

## 6. Contracts & extension points

- **Supported extension surface** (maintainers say): the request, error and failed-request handlers, pre/post navigation hooks, `ContextPipeline` composition, and the `ISessionPool`, `IBrowserPool` and `IRequestManager` interfaces ([upgrading-v4/internals.md:36-44](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/upgrading/upgrading-v4/internals.md?plain=1#L36-L44)). There are also `IConcurrencySystem`, `IStatistics` (+`stateExtension`), `IProxyConfiguration`, `IRenderingTypePredictor`, `StorageBackend`, `BaseHttpClient` and `extendContext`.
  - `extendContext` runs *before* navigation, so it cannot see `page`/`$` ([basic-crawler.ts:1535-1539](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1535-L1539)). This ordering surprised users ([#3420](https://github.com/apify/crawlee/issues/3420)).
  - The compose API went through 6 design alternatives ([#3265](https://github.com/apify/crawlee/issues/3265)) and landed on an `onCleanup` registrar ([context-pipeline.ts:13-25](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawlers/context-pipeline.ts#L13-L25)).
  - Subclassing is still an extension point, but a fragile one. Maintainers warn that overriding a hook without calling `super` where expected "usually compiles and then misbehaves at runtime" ([public-api.mdx](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/guides/public-api.mdx))
- **Storage contract**: `StorageBackend` is an async factory for `Dataset`/`KeyValueStore`/`RequestQueue` sub-backends. `StorageIdentifier` is a discriminated union (`id` | `name` | `alias` | default) enforced with `?: never`. Lifecycle members are optional (`storageExists?`, `purge?`, `teardown?`, `stats?`) ([types/src/storages.ts:420-486](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/types/src/storages.ts#L420-L486)). Memory and filesystem backends live here; the platform backend comes from outside the repo, since the docs name `ApifyClient` ([:472](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/types/src/storages.ts#L472)).
- **Config**: 14 `CRAWLEE_*` fields in one table, resolved in a documented precedence order and immutable after construction (setters throw). See the §4 config row.
- **Error hierarchy** ([core errors.ts:6-103](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/errors.ts#L6-L103), [basic errors.ts:8-93](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/errors.ts#L8-L93)). Error class decides retry semantics:

  | error class | effect |
  |---|---|
  | `NonRetryableError` | never retried |
  | `CriticalError` (extends `NonRetryableError`) | stops the crawler |
  | `SessionError` / `SessionRetiredError` | retires the session, then retries |
  | `RetryRequestError` | retries, ignoring the retry budget |
  | `RequestThrottledError` (extends `RetryRequestError`) | requeued at the front, costs no retry |
  | `RequestValidationError` | schema-checked `userData` failed (carries `issues`) |
  | `ContextPipeline{Initialization,Cleanup,Interrupted}Error` | pipeline failures |

  - Classification is done with `instanceof` in one place ([basic-crawler.ts:2825-2918](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2825-L2918), [:2997-3022](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L2997-L3022)), plus per-request flags (`noRetry`, `maxRetries`).
  - Proxy errors are detected by **message substring** ([:1580-1582](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L1580-L1582), [utils blocked.ts:15](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/utils/src/internals/blocked.ts#L15)), although the public-API guide tells users never to match on log text.
  - There are no error codes; wrapping uses `cause`.
- **Logging and observability**
  - `CrawleeLogger` is an interface ([types/src/logger.ts:22-86](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/types/src/logger.ts#L22-L86)): `child({prefix})`, levels, `warningOnce`, `deprecated`, and `data?: Record<string, unknown>`.
  - Adapters derive from `BaseCrawleeLogger` ([core log.ts:38](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/core/src/log.ts#L38)), so users can plug in pino and similar loggers.
  - OTel lives in a separate package and works by patching methods (§4).
  - Status messages go out on the event bus for the platform to forward.
- **Deprecation**
  - Deprecated APIs are marked with `@deprecated` TSDoc (only 4 left on master), the lint rule `typescript/no-deprecated: warn`, and runtime `log.deprecated()` (e.g. [playwright-utils.ts:221](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/playwright-crawler/src/internals/utils/playwright-utils.ts#L221)).
  - Deprecated options are folded into their replacement (`requestList`/`requestQueue` → `requestManager`, [basic-crawler.ts:288-298](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L288-L298)).
  - Every major gets an upgrade guide. v4's is 11 files and about 2.6k lines, split by audience: users, subclassers, storage-backend authors ([docs/upgrading/upgrading-v4](https://github.com/apify/crawlee/tree/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/upgrading/upgrading-v4)).
  - Releases are lock-step (all packages, one version, `--force-publish`) with canary, RC and stable dist-tags ([RELEASE.md](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/RELEASE.md)).

## 7. Quality machinery

- **Lint** (`oxlint --type-aware`, extending `@apify/oxlint-config`; [oxlint.config.ts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/oxlint.config.ts)).
  - Rules that are on: `import/extensions: always`, `eqeqeq: smart`, `typescript/consistent-type-imports`, `consistent-type-definitions: interface`, `only-throw-error`, `no-misused-promises`, `no-mixed-enums`, `prefer-optional-chain`, `no-underscore-dangle` (with an allow-list) and `unicorn/filename-case: kebabCase`.
  - Rules that are off: `no-explicit-any`, `no-unnecessary-condition`, `switch-exhaustiveness-check`, `no-unsafe-type-assertion` and `no-floating-promises` (the last only for tests).
  - Some shared-config rules are off by admission, because "the codebase does not currently satisfy" them ([:116-122](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/oxlint.config.ts#L116-L122)).
  - A **custom lint rule** `crawlee/prefer-private-fields` bans TS `private` in favour of `#` ([scripts/oxlint-plugin.ts:9-30](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/scripts/oxlint-plugin.ts#L9-L30)) and has its own test (`scripts/oxlint-plugin.test.ts`). `src` contains 39 lint-disable comments; 20 give a `--` reason and 19 do not.
- **Format**: `oxfmt` with printWidth 120, 4 spaces, single quotes and trailing commas ([.oxfmtrc.json](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.oxfmtrc.json)). husky and lint-staged run oxlint `--fix` and oxfmt before each commit.
- **Type-check**: strict `@apify/tsconfig` (§2). CI type-checks the tests (`tsc-check-tests`) and the **doc examples** (`docs/package.json` `typecheck`) separately. JSDoc examples are *not* compiled; one is not even valid JS (`extendContext(context) => ({`, [basic-crawler.ts:265](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/basic-crawler.ts#L265)).
- **Tests** (vitest 5; [vitest.config.mts](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/vitest.config.mts))
  - Two projects: `unit` and `browser`. The browser project is capped in workers because browser tests are "~70% of the suite's CPU time".
  - Tests run against `src` through aliases, with no build step.
  - Type-level tests use `expectTypeOf` (e.g. `navigation-hook-types.test.ts`, 198 lines).
  - Integration tests run on every PR against Docker browserless and httpbin ([test-integration.yml](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/test-integration.yml)).
  - E2E runs nightly: 48 actor scenarios against MEMORY, LOCAL or PLATFORM storage, with threshold assertions such as `requestsSucceeded > 40` ([test-e2e.yml](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/test-e2e.yml), [test/e2e/cheerio-default/test.mjs](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/test/e2e/cheerio-default/test.mjs)).
  - CI sets `RETRY_TESTS: 1`, which means `retry: 3` ([vitest.config.mts:66](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/vitest.config.mts#L66), [test-ci.yml:10](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/test-ci.yml#L10)): flakiness is absorbed rather than surfaced.
  - The largest test file is 4,680 lines (`test/core/crawlers/basic-crawler.test.ts`).
  - There is no coverage gate in CI.
- **CI** ([test-ci.yml](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/test-ci.yml))

  | job | what it does |
  |---|---|
  | Build & Test | Ubuntu, Node 22/24/26; Windows is commented out as "extremely unstable" ([:30-32](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/test-ci.yml#L30-L32)); runs the 30-min-capped tests |
  | **Public API surface** | `pnpm api:check` fails when the committed `docs/public-api/*.api.md` (17 reports, ~5k lines) is stale, or when a public signature references an `@internal` type ([CONTRIBUTING.md:66-77](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/CONTRIBUTING.md?plain=1#L66-L77)) |
  | Docs build | builds the site, then runs nginx header assertions |
  | Lint | oxlint plus an oxfmt check |
  | Canary publish | publishes on every push to `master` |

  Other workflows: a conventional-commit PR-title check, and an AI review / auto-approval workflow ([factory_review.yaml](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/.github/workflows/factory_review.yaml)).
- **Contribution rules** ([CONTRIBUTING.md](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/CONTRIBUTING.md)): include tests, use conventional commits (changelogs are generated from them), don't rebase after review, PRs are squash-merged, and regenerate the API reports when the public surface changes. `.claude/CLAUDE.md` tells agents to write the failing test first and to keep changes minimal.
- **Design docs / ADRs**: none in the repo. The design rationale lives in **issues** (e.g. [#3087](https://github.com/apify/crawlee/issues/3087), [#3265](https://github.com/apify/crawlee/issues/3265), [#3710](https://github.com/apify/crawlee/issues/3710), [#4101](https://github.com/apify/crawlee/issues/4101)) and in the upgrade guides, which double as change records with reasons.

## 8. Pain points

**Maintainers say:**
- v3 context building "is hard to understand and keep track of", which is why ContextPipeline was ported from Python ([#2479](https://github.com/apify/crawlee/issues/2479)).
- Component management was "scattered between `StorageManager` and `Configuration`" ([#3073](https://github.com/apify/crawlee/issues/3073)).
- Per-run state on the crawler is "temporal coupling" ([#3087](https://github.com/apify/crawlee/issues/3087)). Its 2026-09-03 status comment listed what was then left: resetting was "a hand-written checklist at the top of `run()`", and the pool teardown bug meant multi-run tests "process nothing". `CrawlerRun` ([PR #4111](https://github.com/apify/crawlee/pull/4111)) closed the issue on 2026-09-08.
- The service-locator semantics went through four iterations in Python (force flag → replace-until-read → strictly set-once + per-crawler locators). JS adopted the end state, and the maintainer wrote the history down "to make sure that we don't needlessly re-learn the same lessons" ([#3710 comment](https://github.com/apify/crawlee/issues/3710)). Single-service replacement is still an acknowledged "ergonomic wart".
- `@crawlee/core` was too broad for the Apify SDK, which only needs storages, config, events and proxy. The proposal is to push crawler-specific modules down into `@crawlee/basic` ([#4101](https://github.com/apify/crawlee/issues/4101)), and the current layout reflects it: autoscaling, session pool and router now live in `basic`.
- `jsdom` and `linkedom` were moved out: they were heavy dependencies that every install and CI run paid for, they were rarely used, and they were tied to the monorepo release cadence ([#4113](https://github.com/apify/crawlee/issues/4113)).
- v3 TS `private` was reachable at runtime, so v4 converts everything to `#`. The guide lists dozens of members that were "`public` or `protected` only by accident" ([internals.md:34-60](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/upgrading/upgrading-v4/internals.md?plain=1#L34-L60)). Every crawler gained extra type parameters, which broke explicit annotations ([internals.md:90-94](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/docs/upgrading/upgrading-v4/internals.md?plain=1#L90-L94)).
- A user reported silent `pushData` breakage after updates. The maintainer's first hypothesis was multiple installed copies of `@crawlee/core`, which a design built on module-level singletons is exposed to ([#2117](https://github.com/apify/crawlee/issues/2117)).

**My inference:**
- The biggest structural debts are:
  - the `BasicCrawler` hub (§5.4);
  - parallel Playwright/Puppeteer trees with measurable drift (§5.1);
  - ambient services through `AsyncLocalStorage`/Proxy plus method rewriting;
  - observability that depends on private method names;
  - generic-parameter growth (5–8 parameters per crawler);
  - options declared twice.
- v4 is a large, well-documented cleanup, but it arrived as a breaking major *after* years of a public surface leaking internals. Most of the seams (the `I*` interfaces, `OwnedOrInjected`, `CrawlerRun`) were retrofitted.

## 9. Scorecard (1–5)

| Dimension | Score | Justification |
|---|---|---|
| Layering & dependency direction | 3 | The package DAG is acyclic, the base is types-only and there are `./internal` subpaths. But `core` → concrete `fs-storage`, adapters depend on `core` for a logger, there are 4 intra-package cycles, export-star cascades, and no boundary check (§3). |
| Module/file size discipline | 2 | 8 files over 1k lines, a 3,180-line `BasicCrawler` with a 400-line constructor, a 4,680-line test file, and no size budget. |
| DRY (incl. across language ports) | 2 | Playwright/Puppeteer copies at 70–82% similarity with drift, HTTP/browser navigation duplicated, options declared twice, and the JS/Python ports kept in sync by hand. Partly redeemed by the shared Rust storage and the Rust golden conformance suite. |
| Separation of concerns | 3 | v4 extracted the concurrency governor, run object, throttling decorator and storage transaction, but the crawler class still owns robots, rate limits, export, retries and locator scoping. |
| Extension model | 4 | A documented supported surface, 12 `I*` interfaces, a typed `ContextPipeline`, hook lists, browser plugins and inject-or-own lifecycle. Minus fragile `super`-ordering and otel's monkey-patching. |
| Contracts & types | 4 | Strict TS, an API surface enforced in CI with committed reports, type tests, Standard Schema route validation and discriminated storage IDs. Minus `any`/casts, untyped events and the two option sources of truth. |
| Error model | 3 | A clear class hierarchy maps to retry/session/stop semantics, with `cause` wrapping. Minus no error codes, message-substring classification and mutable per-request flags. |
| Test architecture | 3 | 1.4× test-to-src code, unit/browser split, PR integration tests, nightly e2e on three storage backends and type tests. Minus root-level tests not mirroring packages, 3× CI retries, no coverage gate and no Windows. |
| Tooling/CI enforcement | 4 | Type-aware oxlint with a tested custom rule, a format check, an API-surface diff, doc-example type-checks, a Node matrix and a PR-title check. Minus admitted disabled rules, the `.d.ts` patch script and no dependency-boundary lint. |
| In-code documentation | 4 | 34% comment lines, 890 `@apilink` cross-references, ~230 TSDoc examples and frequent "why" comments. Minus TSDoc examples that are not compiled, one of them invalid. |

## 10. Lessons for Goodfetch

1. **Adopt staged, typed context building (Crawlee's `ContextPipeline`), but make it static.** Crawlee's `ContextPipeline` (§4) fixed v3's tangled context setup. Keep three things from it: reverse-order cleanups, the split between setup errors and handler errors, and navigation hooks as pipeline steps that share one timeout budget. In Rust, express each layer's added fields in the type system instead of Crawlee's runtime `defineProperty` merge with silent failures. Avoid an `extendContext` whose run order surprises users ([#3420](https://github.com/apify/crawlee/issues/3420)): document and test the order. ADR 0104's Hooks return a change rather than extend a context, so this is a design input for how the Engine builds each stage's context, not something ADR 0104 decided.
2. **Avoid an ambient service locator; keep explicit, owned handles.** Crawlee's `AsyncLocalStorage` + `Proxy` + per-instance method rewriting (§4) hides dependencies in 36 files and forced adapters to depend on `core` just to log. Its semantics took four iterations in the Python port before JS adopted the end state ([#3710](https://github.com/apify/crawlee/issues/3710)). In a Tokio engine, prefer passing context and handles explicitly over task-locals or global statics for config, storage and logging, and log through `tracing` spans instead of a fetched logger.
3. **Adopt `OwnedOrInjected` semantics from day one**, for example an enum of owned vs borrowed (`Arc`) collaborators where only owned ones are torn down. Pair it with a per-invocation run object (`CrawlerRun`) so per-run state cannot leak. Crawlee retrofitted both in a breaking major after shipping lifecycle bugs ([#3087](https://github.com/apify/crawlee/issues/3087)).
4. **Avoid parallel per-backend implementations of high-level operations.** Playwright and Puppeteer helpers are 54–82% copies and already disagree (`>=` vs `>` in `infiniteScroll`, §5.1). Collie's single CDP client helps, but any second driver or engine should sit below one shared set of page operations, with a duplication check in CI.
5. **Adopt a committed, CI-diffed public-surface report with a written promise policy.** Crawlee's `api:check` plus `docs/public-api/*.api.md`, together with "exported = promised unless `@internal`", turned API drift into a reviewable diff. For Goodfetch that means the Rust public API (e.g. `cargo public-api`) *and* the `xtask codegen` outputs, diffed in PRs, with a lint that rejects public signatures that reference internal types.
6. **Avoid two sources of truth for options or config.** Crawlee keeps a hand-written TS options interface next to a loosely checking zod shape (§5.3). Goodfetch already derives SDK types from Rust protocol types, so keep validation derived from those same types and validate deeply at the boundary. Do follow Crawlee's single field table, documented precedence and immutable config object, and avoid its global side effect (`log.setLevel` in the constructor).
7. **Avoid observability that depends on private method names.** `@crawlee/otel` patches `private` methods and needs lint suppressions in `basic-crawler`, `browser-crawler` and `http-crawler` (§4). Make spans and events a first-class, stable contract emitted at defined seams (request, navigation, retry, failure).
8. **Adopt type-based error classification with stable codes; never match on message text.** Crawlee's class hierarchy cleanly encodes retry, session-retire and stop semantics, but it classifies proxy failures by substring and has no codes. A Rust error enum with retryability as data, plus stable codes in the protocol, gives all the SDKs the same semantics.
9. **Adopt cross-language conformance by golden files.** Apify keeps a Rust port honest by running Crawlee-JS from source as an oracle and committing golden JSON that the Rust tests replay. It also shares one Rust storage crate through napi-rs and maturin (§5). Goodfetch's generated SDKs should replay one shared set of golden protocol transcripts, so the ports don't drift the way Crawlee's hand-synced JS and Python codebases had to be reconciled issue by issue.
10. **Enforce size and boundary budgets mechanically before code accumulates.** Crawlee's `typescript-fixes.mjs` (`.d.ts` patching) and its lint rules disabled "for now" are what results when this doesn't happen. Its strengths came from tooling (a custom lint rule with tests, type-checked doc examples, type-level tests). Its weaknesses are exactly the unenforced areas: file size, package boundaries, duplication, flaky-test retries. Add those checks to `xtask` or CI while the codebase is still empty.
