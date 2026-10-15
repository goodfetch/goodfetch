# Playwright: codebase design notes

Read on 2026-10-16 from a local shallow clone. Method: reading files, `tokei`, `wc`, `grep`, and three small Python scripts that measure file sizes, file similarity and stale `DEPS.list` entries. No code from the repository was built or run. Every link points to commit `0d7c141`. "Inference" marks my own judgement. Everything else is read from the code, the repository's docs or the issue tracker.

Permalink base: `https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/`. Every link below is a full permalink.

## 1. Snapshot

| Item | Value |
|---|---|
| Repo | [microsoft/playwright](https://github.com/microsoft/playwright), branch `main` |
| Commit | `0d7c1411b0afdf8b78664c490ee473857690d66c`, 2026-10-15 14:13 -0700 |
| Version | `1.65.0-next` ([package.json:4](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/package.json#L4)), Apache-2.0 |
| Languages | TypeScript (311k code lines repo-wide, 1,295 files), JavaScript (117k; mostly test assets and `utils/` scripts), TSX (11k), YAML (4.6k; 4.5k of it is the protocol), Markdown (75k lines of docs). Browser patches add C++/Obj-C/JS for Firefox and WebKit. |
| Whole repo (tokei) | 506,025 code lines in 2,469 files. Most of this is tests, test assets and generated files. |
| Hand-written product code | about 124.7k code lines under `packages/` once generated files, `third_party`, JSON and CSS are excluded (TS 107k, TSX 11k, protocol YAML 4.5k). Comments: 18.6k lines, about 15% of code. |
| Tests | 146.8k code lines, not counting 119k lines of `tests/assets`. That is 1.18 times the product code. 104 + 162 + 136 + 102 spec files in `playwright-test/`, `library/`, `page/`, `mcp/`. |
| Other | `utils/` 11.3k (build, codegen, lint tooling), `browser_patches/` 10.7k, `docs/src` 181 files |

**Packages.** There are 20 directories under `packages/`. Sixteen have a `package.json`. Four are source-only folders that the build bundles into other packages: `injected`, `isomorphic`, `protocol` and `utils`. Nine npm packages are public: `playwright-core`, `playwright`, `@playwright/test`, `playwright-{chromium,firefox,webkit}` and `@playwright/browser-{chromium,firefox,webkit}`. The rest are private: the UI apps and `@playwright/client`. `playwright-core` declares **no runtime dependencies**, because third-party code is vendored and bundled through `utilsBundle`. CONTRIBUTING sets "a very high bar for new dependencies" ([CONTRIBUTING.md:128](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/CONTRIBUTING.md#L128)).

Hand-written code lines per area (tokei, generated files excluded):

| Area | Code lines | Files | Role |
|---|---:|---:|---|
| `playwright-core/src/server` | 36,133 | 159 | Engine: browser-agnostic core, 5 backends, dispatchers, tracing, recorder, registry |
| `playwright-core/src/client` | 7,355 | 51 | JS client (`ChannelOwner` classes behind the public API) |
| `playwright-core/src/tools` | 13,374 | 112 | MCP server, CLI daemon, dashboard, trace CLI |
| `playwright/src` | 21,751 | 121 | Test runner: runner, worker, loader, transform, matchers, reporters |
| `isomorphic` | 11,381 | 64 | Pure code with no Node or DOM globals: selectors, locators, trace model, codegen |
| `injected` | 9,271 | 40 | Scripts that run inside the page |
| `protocol` | 4,838 | 21 | `spec/*.yml` (19 files) plus serializers and validator primitives |
| `utils` | 3,068 | 29 | Node-only helpers: logging, process launch, zones, network |
| UI apps | about 14.5k | about 180 | `trace-viewer`, `html-reporter`, `web`, `dashboard`, `recorder`, `extension` (React) |

Shared server code, excluding backends and dispatchers, is about 18.8k code lines. Each backend is much smaller: chromium 3.7k, firefox 1.6k, webkit 2.2k, webkit/webview 2.2k, bidi 2.6k, android 1.4k, electron 0.3k.

**Ten largest hand-written source files** (line counts with `wc -l`; tests excluded):

| # | File | Lines | Note |
|---|---|---:|---|
| 1 | `packages/injected/src/injectedScript.ts` | 1,938 | one class, `InjectedScript`, runs from line 94 to 1835 |
| 2 | `packages/playwright-core/src/server/frames.ts` | 1,929 | `Frame` alone runs from line 514 to about 1870, about 100 methods |
| 3 | `packages/injected/src/recorder/recorder.ts` | 1,813 | |
| 4 | `packages/playwright/src/matchers/expectLibrary.ts` | 1,749 | adapted from Jest: "Files merged into one" ([L9-L11](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/matchers/expectLibrary.ts#L9-L11)) |
| 5 | `packages/playwright-core/src/tools/cli-daemon/commands.ts` | 1,457 | 103 `declareCommand` blocks |
| 6 | `packages/injected/src/roleUtils.ts` | 1,456 | |
| 7 | `browser_patches/firefox/juggler/TargetRegistry.js` | 1,421 | runs inside the patched Firefox |
| 8 | `packages/playwright-core/src/server/registry/index.ts` | 1,398 | browser download and install |
| 9 | `packages/playwright-core/src/server/webkit/wkPage.ts` | 1,362 | |
| 10 | `packages/playwright-core/src/server/webkit/webview/wvPage.ts` | 1,316 | about 50% identical to #9 (see §5) |

The largest test files are `tests/playwright-test/reporter-html.spec.ts` (3,959 lines) and `tests/library/trace-viewer.spec.ts` (2,817).

**Generated files are committed and are larger than any hand-written file:**

- `types/types.d.ts`: 27,173 lines, generated from `docs/src/api`. It is committed twice, byte for byte, in `playwright-core` and `playwright-client`.
- Chromium `protocol.d.ts`: 24,776 lines, from `utils/protocol-types-generator`. Also committed twice: in `server/chromium/` and in `types/`.
- `test.d.ts`: 11,512 lines.
- WebKit and WebView `protocol.d.ts`: 10,094 and 8,452 lines.
- `server/channels.d.ts` and `client/channels.d.ts`: 5,371 and 5,370 lines.
- `protocol/src/validator.ts`: 3,241 lines.
- `bidiProtocolCore.ts`: 3,119 lines, under `third_party`.

Across the 710 hand-written `.ts`/`.tsx` files under `packages/`, the median is 127 lines and the 90th percentile is 472. Sixty-three files are over 500 lines, 13 over 1,000 and 4 over 1,500.

## 2. Paradigm

- **Runtime and concurrency.** Node.js (`engines.node >=20`) on a single-threaded event loop with `async`/`await` and `EventEmitter`.
  - Parallel work happens in separate OS processes. Browsers talk to the engine over a pipe or WebSocket. Test workers are started with `child_process.fork` ([processHost.ts:56](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/runner/processHost.ts#L56)).
  - When client and server run in one process, every message still passes through the protocol, and each delivery waits for the next event-loop turn with `setImmediate` ([inprocess.ts:49-50](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/inprocess.ts#L49-L50)).
  - The current API call and test step are carried through async code with `AsyncLocalStorage` "zones" ([zones.ts:23-55](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/utils/zones.ts#L23-L55)).
  - Cancellation and deadlines use an explicit `Progress` object that is passed into every server operation ([progress.ts:25-35](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/progress.ts#L25-L35)).
- **Style.** Class-based OO with shallow, framework-like inheritance:
  - 32 client classes extend `ChannelOwner`.
  - Server objects extend `SdkObject`.
  - 33 dispatchers extend `Dispatcher<Object, Channel, ParentScope>`.
  - Engine variation uses two mechanisms side by side: abstract base classes with template methods (`Browser`, `BrowserContext`, `BrowserType`), and **composition** through the `PageDelegate` interface, held by `Page` as `readonly delegate` ([page.ts:180](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/page.ts#L180)).
  - The test runner and MCP tools use more functional code: fixture functions, `defineTool({...})` records, and reporters as callback objects.
- **Public API style.** An object-oriented, Promise-based API (`page.goto`, `locator.click`) with options objects. Locators are lazy and chainable. Actions wait automatically. Events use `page.on(...)`. Language ports keep the same shape and add idioms: Python has both sync and async, Java is sync, .NET is async. The JS API is declared in Markdown under `docs/src/api/*.md` (55 class files) and compiled into `types.d.ts`.
- **Typing.**
  - `tsconfig.json` sets `strict: true` and `noImplicitOverride: true`, but relaxes `useUnknownInCatchVariables: false`, so catch variables are typed `any` ([tsconfig.json:29-36](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tsconfig.json#L29-L36)).
  - `any` is common: `as any` appears 78 times in `client/`, 161 in `server/` and 30 in `tools/`. `no-explicit-any` is not enabled.
  - Types are checked at runtime at the process boundary: generated validators run on every message, in both directions, on both sides (§6).

## 3. Organization

**Top level:**

- `packages/`: product code.
- `tests/`: all test suites, sorted by area.
- `docs/src/`: guides plus `api/`, `test-api/`, `test-reporter-api/` and `electron-api/`. The API specification there is the source of truth for public types.
- `utils/`: build, codegen, doclint, `check_deps.js`, the custom ESLint plugin.
- `browser_patches/`: Firefox "juggler" and WebKit patches.
- `examples/`.
- `.github/workflows` (24 workflows) and `.azure-pipelines` (publish only).
- Design docs live in `.claude/skills/playwright-dev/*.md`, about 2.7k lines, for example [library.md](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.claude/skills/playwright-dev/library.md) on the client, server and dispatcher layers.

**Layers inside `playwright-core/src`.** `client/` (the API objects) talks to `server/dispatchers/` only through the protocol. The dispatchers adapt messages to the `server/` domain objects, which drive the `server/{chromium,firefox,webkit,bidi,android,electron}` backends. The repo's agent guide states the rule: "Client code NEVER imports server code. Server code NEVER imports client code" ([AGENTS.md:103](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/AGENTS.md#L103)).

**Dependency direction is declared in 51 `DEPS.list` files and enforced** by [utils/check_deps.js](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/check_deps.js), which runs inside `npm run lint`. Facts I verified:

- **Composition root.** Only `server/playwright.ts` may import the engine folders ([server/DEPS.list:23-29](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/DEPS.list#L23-L29)). It builds them in one place ([playwright.ts:61-65](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/playwright.ts#L61-L65)). The shared server cannot reach a backend.
- **Client.** `client/` may import `@isomorphic/**`, `@protocol/**` and five named `@utils` files ([client/DEPS.list](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/DEPS.list)). `injected/` may import only `@isomorphic/**` ([injected/src/DEPS.list](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/injected/src/DEPS.list)).
- **`isomorphic` is a leaf.** It has no top-level `DEPS.list`, so the checker lets its files import only files in the same folder. It imports no external packages. ESLint also bans Node globals (`process`) and DOM globals in it ([eslint.config.mjs:444-458](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs#L444-L458)).
- **Test runner.** `worker/` may not import `runner/`, and `runner/` may not import `worker/`, so the process boundary is enforced statically.
- **Modes.** A rule can be `"strict"` (only listed imports, and the target must also be strict), `***` (anything), or `node_modules/<pkg>`, which lists each third-party package allowed per folder or file.
- **Gaps.**
  - Type-only imports are exempt ([check_deps.js:138-145](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/check_deps.js#L138-L145)).
  - A `// @no-check-deps` comment skips the check ([L189-L195](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/check_deps.js#L189-L195)).
  - `tools/backend` is allowed `../../..`, which is all of `src/`.
  - **Five rule sections name files that no longer exist**, for example `[devtoolsController.ts]` ([server/DEPS.list:21](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/DEPS.list#L21)) and `[inProcessFactory.ts] **` ([src/DEPS.list:13-14](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/DEPS.list#L13-L14)). The checker does not detect stale rules.
  - `src/common/` contains only a `DEPS.list`.
  - ESLint adds `no-restricted-imports` rules on top, for example "tools/ must not import from client/" ([eslint.config.mjs:417-443](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs#L417-L443)). That makes two separate tools for boundaries.

**Public vs internal API.**

- The only public contract is the documented API. Maintainers state it plainly: "Playwright has no notion of hidden API, everything which is not documented or exposed can be removed at any point" ([#28615](https://github.com/microsoft/playwright/issues/28615)).
- At runtime, internals are marked by naming convention, not by encapsulation. `_method()` means "used by other code in the same file", `private _method()` means class-only, and a plain `method()` is used across files ([AGENTS.md:108-111](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/AGENTS.md#L108-L111)).
- The published `.d.ts` hides the `_` members.
- `playwright-core` exports `./lib/coreBundle`, `./lib/utilsBundle` and others so that the `playwright` package can reach in. Sibling packages therefore depend on internal deep paths.

**Naming and layout.**

- Engine-prefixed files: `crPage.ts`, `ffPage.ts`, `wkPage.ts`, `bidiPage.ts`, `wvPage.ts`.
- `xxxDispatcher.ts` files mirror the server class names.
- Specs are flat per area and named `area-feature.spec.ts`, for example `browsercontext-add-cookies.spec.ts`.
- Generated files carry the header "This file is generated by …, do not edit manually" ([generate_channels.js:130](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L130)).
- Fixtures sit next to the suites: `tests/config/*Fixtures.ts`, `tests/page/pageTest.ts`. A local test server (`tests/config/testserver`) and `tests/assets` keep tests hermetic ([CONTRIBUTING.md:79](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/CONTRIBUTING.md#L79)).

**File-size discipline is moderate.** The median file is small (127 lines), but several "god" units remain: `Frame` with about 1,350 lines and about 100 methods, `InjectedScript` with about 1,740 lines, and `cli-daemon/commands.ts` with 103 commands in one file. No lint rule limits file or function size.

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| Schema → codegen for both RPC sides | [generate_channels.js:239-383](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L239-L383). A single YAML spec emits client `Channel` interfaces (`options: TimeoutOptions`) and server interfaces (`progress: Progress`) ([L383](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L383)), plus validators, structs and metadata ([L483-L488](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L483-L488)) | Keeps 34 interfaces and about 327 commands in sync across client and server | **Good.** The script exits with status 1 when output changes, so lint catches drift. The generator is a hand-rolled 488-line string builder with no tests of its own. |
| Dynamic proxy stub | `ChannelOwner._createChannel` uses a `Proxy` to turn any property access into a validated RPC call ([channelOwner.ts:157-187](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/channelOwner.ts#L157-L187)) | No per-method client stub code | **Mixed.** Very DRY, but a JS-specific trick. Ports have to generate or hand-write their own stubs. Typing comes only from the generated interface. |
| Mirrored object tree (remote proxy plus identity map) | Server `Dispatcher` registers a GUID and sends `__create__`, `__adopt__` and `__dispose__` ([dispatcher.ts:62-83](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L62-L83), [L213-L225](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L213-L225)). The client rebuilds the tree ([connection.ts:266-289](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L266-L289)) | Object lifetimes and parent scopes match across the wire. Disposing a parent disposes its children. | **Good.** Bounded by per-type GC buckets (10k objects; 100k for handles) ([L43-L48](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L43-L48), [L281-L295](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L281-L295)). Tested in [channels.spec.ts](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/library/channels.spec.ts#L40). |
| Factory registry | `registerObjectFactories({ Page: …, Frame: … })` ([connection.ts:98-141](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L98-L141)) | Maps wire type names to client classes | **Good** but hand-listed. `SocksSupport` maps to a `DummyChannelOwner`, and `headers` is marked "TODO remove?" ([L88](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L88)). |
| Bridge or strategy for engines | `PageDelegate` interface ([page.ts:59-110](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/page.ts#L59-L110)) implemented by `CRPage`, `FFPage`, `WKPage`, `BidiPage` and `WVPage` | One `Page`/`Frame` core over five protocols | **Good, but leaky.** The interface carries browser workarounds: "WebKit hack", "Work around … Firefox" ([L99-L109](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/page.ts#L99-L109)). Even so, shared server code checks the browser name only about 14 times. |
| Template method | `BrowserContext` public methods call `protected abstract doGrantPermissions`, `doClearCookies` and so on ([browserContext.ts:281-301](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/browserContext.ts#L281-L301), [L390-L401](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/browserContext.ts#L390-L401)) | Shared validation and normalization, engine-specific effect | **Good.** The `doX` naming makes the hooks easy to spot. |
| Composition root | [server/DEPS.list:23-29](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/DEPS.list#L23-L29) together with [playwright.ts:61-65](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/playwright.ts#L61-L65) | Keeps backends out of shared code | **Good.** Enforced by tooling, not by convention. |
| Observer fan-out (instrumentation bus) | `createInstrumentation()` is a `Proxy` that forwards every `on*` hook to registered listeners, filtered per context ([instrumentation.ts:136-172](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/instrumentation.ts#L136-L172)). Tracing, Recorder, Debugger and Screencast implement `InstrumentationListener`. | Adds tracing, recording and debugging without editing call sites | **Good.** The dispatcher calls `onBeforeCall` and `onAfterCall` around every command ([dispatcher.ts:365-391](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L365-L391)). Hooks are untyped at the proxy (`any`). |
| Cancellation and deadline token | `ProgressController.run` builds a `Progress` with `race`, `wait`, `signal` and `log` ([progress.ts:82-162](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/progress.ts#L82-L162)). `__abort__` messages cancel by call ID ([dispatcher.ts:306-313](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L306-L313)) | Every server operation honours the timeout, client abort and target close | **Good.** It is backed by a type-aware lint rule (§7). |
| Metadata-driven cross-cutting concerns | Protocol `flags: {slowMo, snapshot, pause}` and `title` ([page.yml:120-129](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/page.yml#L120-L129)) → generated `methodMetainfo` → read by the dispatcher ([dispatcher.ts:392-393](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L392-L393)) and by tracing ([tracing.ts:545-546](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/trace/recorder/tracing.ts#L545-L546)) | Snapshot, pause and slow-motion behaviour is declared once per command | **Good.** The generator checks the rules, for example "must have a title unless internal" ([generate_channels.js:317-352](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L317-L352)). |
| Ambient context (zones) | `_wrapApiCall` stores an `apiZone` in `AsyncLocalStorage` so that nested internal calls are not reported twice ([channelOwner.ts:189-224](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/channelOwner.ts#L189-L224)) | Attributes API names, stack frames and steps to the user's call | **Mixed.** Powerful, but it is implicit state that each language port re-implements in its own way. |
| Adapter | `wrapReporterAsV2` and `ReporterV2Wrapper` ([reporterV2.ts:63-72](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/reporters/reporterV2.ts#L63-L72)) | Keeps the public v1 reporter API while the internals move to v2 | **Good.** This is expand–contract inside the code. |
| Composite and decorator | `Multiplexer implements ReporterV2` ([multiplexer.ts:23](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/reporters/multiplexer.ts#L23)) and `RedactingReporter` ([redactingReporter.ts:78](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/reporters/redactingReporter.ts#L78)) | Runs N reporters as one and wraps secret redaction around them | **Good.** |
| Chain of version upgraders | `_modernize_N_to_N+1` loop up to `latestVersion = 10`, with frozen type snapshots `versions/traceV3..V10.ts` ([traceModernizer.ts:38](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/isomorphic/trace/traceModernizer.ts#L38), [L239-L247](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/isomorphic/trace/traceModernizer.ts#L239-L247)) | Old trace files stay readable forever | **Good idea, mixed execution.** The next upgrader is looked up by string method name through `(this as any)[...]`. |
| Declarative tool registry plus decorator | `defineTool`, and `defineTabTool`, which wraps a handler with modal-state guards ([tool.ts:58-84](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/backend/tool.ts#L58-L84)). A runtime `capability` filter selects tools ([tools.ts:83-84](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/backend/tools.ts#L83-L84)) | MCP tools as data with zod schemas. Capabilities are runtime settings. | **Good.** The CLI re-declares the same commands and points at them by string `toolName` (§5). |
| Dependency injection through fixtures | `FixturePool` and `FixtureRunner` ([fixtures.ts:82](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/common/fixtures.ts#L82), [fixtureRunner.ts:183](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/worker/fixtureRunner.ts#L183)). Playwright's own tests switch implementations with `PWPAGE_IMPL` ([pageTest.ts:31-38](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/page/pageTest.ts#L31-L38)) | One spec suite runs against browser, Android, Electron and WebView | **Good.** It is the main user extension point. |

## 5. Separation of concerns and DRY

**Where duplication is avoided:**

1. **One protocol spec, many outputs.** `packages/protocol/spec/*.yml` (19 files) is merged by [protocol_spec.js](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/protocol_spec.js#L30-L43), which rejects duplicate definitions at line 39.
   - In this repo it produces client and server `channels.d.ts`, shared `structs.d.ts`, runtime validators, method metadata, and .NET channel classes ([generate_dotnet_channels.js:182](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_dotnet_channels.js#L182)).
   - The Python port generates `_protocol.py` from the same spec ([update_api.sh](https://github.com/microsoft/playwright-python/blob/c363ce9d84d18b6e88db4d956fcc2f155d904a05/scripts/update_api.sh)).
   - The spec uses `$mixin` for shared option groups such as `LaunchOptions` and `ContextOptions` ([mixins.yml](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/mixins.yml#L15-L115)).
2. **Docs as API source.** `docs/src/api/*.md` holds per-language annotations: `* langs:` (492 uses), `alias-python`/`-java`/`-csharp` names, `* since:` (3,064), `* deprecated:` (27) and `* discouraged:` (100).
   - It produces the JS `types.d.ts` ([generate_types/index.js](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_types/index.js#L31-L51)) and the C# interfaces ([generateDotnetApi.js](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/doclint/generateDotnetApi.js#L45)).
   - Python generates its sync and async public surface (`_generated.py`, about 970 KB) from the same docs.
   - **The compiler closes the loop.** 44 client classes declare `implements api.X`, for example [client/page.ts:90](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/page.ts#L90). An undocumented change to a public signature therefore fails `tsc`.
3. **One engine for all languages.** Ports do not re-implement automation. They start the Node engine as a "driver" over stdio: `run-driver` ([driver.ts:30-46](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/cli/driver.ts#L30-L46)). `sdkLanguage` (`javascript|python|java|csharp`, [core.yml:36-42](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/core.yml#L36-L42)) lets the server generate locators and code in the caller's language. JS uses the same path even in-process ([inprocess.ts:27-62](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/inprocess.ts#L27-L62)), so JS is "just another port" with zero-copy buffers (`useRawBuffers`).
4. **Shared server over five protocols.** About 18.8k shared code lines against 1.6–3.7k per backend. `Page`, `Frame`, `dom.ts` and the injected script hold the auto-wait and actionability logic once.
5. **Injected and isomorphic code is shared between the server, the browser page and the UIs.** Selector parsing, locator generation and the trace model live in `isomorphic`. They are bundled into page scripts as string constants ([generate_injected.js](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_injected.js#L28-L94)) and also used by trace-viewer and the reporters.

**Where it fails (evidence):**

- **Two contracts, hand-mapped.** The public API (docs Markdown) and the wire protocol (YAML) are separate schemas. One API change takes six hand-made steps: docs → client → protocol → dispatcher → server → tests ([api.md:5-225](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.claude/skills/playwright-dev/api.md#L5)). For `goBack` that means the method declared in [class-page.md:2435](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-page.md#L2435), [page.yml:120](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/page.yml#L120), [client/page.ts:524](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/page.ts#L524) and [pageDispatcher.ts:162](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/pageDispatcher.ts#L162). Most of the 3.3k lines of dispatcher code is pass-through boilerplate.
- **A thick client is repeated in each language.** Client-side policy lives outside the protocol and must be re-implemented in Python, Java and .NET:
  - timeout defaults ([timeoutSettings.ts:28](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/timeoutSettings.ts#L28));
  - option normalization, such as `null` → `'no-override'` ([client/page.ts:538-545](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/page.ts#L538-L545));
  - `verifyLoadState`, waiters, and API-zone stack attribution.

  The Python `_impl/_page.py` alone is 56 KB of hand-written code. Python keeps "facts that the docs do not contain" in a separate `python_rules.js` ([update_api.sh](https://github.com/microsoft/playwright-python/blob/c363ce9d84d18b6e88db4d956fcc2f155d904a05/scripts/update_api.sh)). Codegen ownership is inconsistent: .NET generators live in this repo, while Python's (`scripts/codegen/`) and Java's (`tools/api-generator`) live in their port repos.
- **Copy-and-rename backend.** `server/webkit/webview/wv*.ts` mirrors `webkit/wk*.ts`. With the `WK`/`WV` prefixes normalized, I measured identical lines:

  | File | Identical lines |
  |---|---:|
  | ExecutionContext | 98% |
  | Workers | 95% |
  | InterceptableRequest | 92% |
  | Connection | 83% |
  | ProvisionalPage | 60% |
  | Page | 50% (613 lines) |

  Each copy imports its own `protocol.d.ts` (8,452 vs 10,094 lines). Inference: the typed protocol client is tied to one schema, which pushed the team to copy rather than abstract.
- **Error code is duplicated across layers.** `serializeError` and `parseError` exist twice, in [server/errors.ts:67-85](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/errors.ts#L67-L85) and [client/errors.ts:52-77](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/errors.ts#L52-L77). The same goes for `TimeoutError`, `TargetClosedError` and `AbortError`.
- **One config option, six declarations.** For example, MCP `allowedHosts` is declared in:
  - the CLIOptions type ([config.ts:37](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/config.ts#L37));
  - the CLI→config mapping ([L378](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/config.ts#L378));
  - the env mapping ([L423](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/config.ts#L423));
  - the INI type table ([configIni.ts:173](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/configIni.ts#L173));
  - the public `config.d.ts` ([L127](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/config.d.ts#L127));
  - the commander option ([program.ts:36](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/mcp/program.ts#L36)).

  `configFromEnv` names `PLAYWRIGHT_MCP_*` 53 times by hand.
- **CLI commands re-declare the MCP tool schemas.** They are linked by string `toolName`, with 10 commands using `toolName: ''` ([commands.ts:58-133](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/cli-daemon/commands.ts#L58-L133)).
- **Examples are written once per language.** The API docs hold about 350 JS, 547 Python, 287 Java and 273 C# code blocks, and some options are forked per language ([class-page.md:1309-1321](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-page.md#L1309-L1321)). CI at least compiles them (§7).
- **The domain depends on wire types.** 37 of 138 non-dispatcher server files import the generated `channels`, for example `abstract addCookies(cookies: channels.SetNetworkCookie[])` ([browserContext.ts:283](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/browserContext.ts#L283)). This is a deliberate DRY trade-off: no separate domain types, at the cost of coupling the engine to the protocol shape.

## 6. Contracts and extension points

- **Protocol.**
  - **Shape.** JSON messages `{id, guid, method, params, metadata}`. Replies are `{id, result | error, errorDetails, log}`. Events are `{guid, method, params}`. Lifecycle uses the reserved methods `__create__`, `__adopt__`, `__dispose__`, `__abort__` and `__waitInfo__` ([dispatcher.ts:297-400](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L297-L400)).
  - **Size.** 34 interfaces with single inheritance (`ElementHandle extends JSHandle`) and 333 method-metadata entries, 86 of them `internal: true` (protocol-only, not public API).
  - **Validation.** Params, results, events, initializers and error details are validated on **both** sides and in both directions ([channelOwner.ts:161-166](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/channelOwner.ts#L161-L166), [connection.ts:253-259](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L253-L259), [dispatcher.ts:322-332](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L322-L332), [L376-L377](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/dispatchers/dispatcher.ts#L376-L377)).
  - **Event subscriptions.** The client turns server events on and off with `updateSubscription` only when it has listeners ([channelOwner.ts:72-111](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/channelOwner.ts#L72-L111)), which avoids sending unwanted network events.
  - **Versioning.** There is no protocol version field. Compatibility is lockstep by release: "major and minor version … needs to match" ([class-browsertype.md:95](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-browsertype.md#L95)). It is checked only on the remote server from the client's User-Agent, and a mismatch produces an ASCII-box error ([playwrightServer.ts:331-352](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/remote/playwrightServer.ts#L331-L352)).
  - **stdout.** It is the RPC channel in driver mode ([driver.ts:37](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/cli/driver.ts#L37)). For non-JS ports the server rewrites lone surrogates in strings, because the .NET JSON parsers reject them (L39-L45).
- **API source of truth.** `docs/src/api` ("Every part of the public API should be documented … in the same change", [CONTRIBUTING.md:51](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/CONTRIBUTING.md#L51)). The doc linter checks links, code-block languages and section anchors for every language ([doclint/cli.js:128-280](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/doclint/cli.js#L128-L280)).
- **Config.**
  - The library takes typed options objects on `launch` and `newContext`, which become protocol params through mixins.
  - The test runner uses `playwright.config.ts` plus fixture `use` options.
  - **Environment variables are not centralized.** I counted about 141 distinct `process.env.*`/`getFromENV` names across 79 package files. `getFromENV` also reads `npm_config_*` ([env.ts:17-22](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/utils/env.ts#L17-L22)).
- **Errors.**
  - The public JS API exposes only `errors.TimeoutError`. Python adds a base `Error` ([class-error.md](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-error.md)) and Java a `PlaywrightException`, each declared per language in the docs.
  - On the wire an error is `{message, stack, name, code}`. The client rebuilds known classes by matching the **name string** ([client/errors.ts:58-77](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/errors.ts#L58-L77)) and appends the server's call log ([connection.ts:247-256](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L247-L256)).
  - Protocol errors from a closed target are rewritten into `TargetClosedError` with the close reason ([server/errors.ts:54-65](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/errors.ts#L54-L65)).
  - **Typed error payloads** exist for each method through `errorDetails` in the spec ([frame.yml:873-879](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/frame.yml#L873-L879), [page.yml:202-208](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/page.yml#L202-L208)).
  - About 25 `Error` subclasses are spread across packages, with names reused (`ExpectError` in both `server/frames.ts:107` and `matchers/matcherHint.ts:46`).
- **Logging and debug.** `DEBUG=pw:api,pw:protocol,pw:channel,…` uses `debug` with a **closed set of log names** typed as `LogName` ([debugLogger.ts:20-66](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/utils/debugLogger.ts#L20-L66)). `DEBUG_FILE` sends output to a file with ANSI codes removed. Each call keeps a per-call `log[]` that travels back with errors ("Call log:", [connection.ts:327-334](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/connection.ts#L327-L334)).
- **Tracing and instrumentation.** `SdkObject` carries `attribution` (playwright, browser, context, page, frame) and a shared `instrumentation` ([instrumentation.ts:36-74](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/instrumentation.ts#L36-L74)). Hooks: `onBeforeCall`, `onBeforeInputAction`, `onCallLog`, `onAfterCall`, page and browser lifecycle, dialog and download ([L106-L134](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/instrumentation.ts#L106-L134)). The client has a second instrumentation layer that the test runner uses for steps (`clientInstrumentation.ts`). Trace files are versioned (V3–V10) and upgraded on read.
- **Extension points for users.**
  - Custom selector engines (`selectors.register`, [server/selectors.ts:58](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/selectors.ts#L58)).
  - Test fixtures (`test.extend`), custom reporters, and custom `expect` matchers.
  - MCP capabilities chosen at runtime (`--caps`).
  - The test-runner plugin interface stays internal ([plugins/index.ts:20-24](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright/src/plugins/index.ts#L20-L24)) and was deliberately hidden (§8).
- **Deprecation.** `* deprecated:` in the docs becomes a JSDoc `@deprecated` in the generated types ([generate_types/index.js:492-493](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_types/index.js#L492-L493)). `* discouraged:` marks soft deprecation. Deprecated options stay and become no-ops, for example `Page.isHidden.timeout` ([class-page.md:2813-2815](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-page.md#L2813-L2815)). Reporter v1 to v2 goes through an adapter.

## 7. Quality machinery

- **`npm run lint` runs everything** ([package.json:38](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/package.json#L38)): eslint, `tsc`, doclint, `check-deps`, `generate_channels` and `generate_types` (which fail on drift), `lint-tests`, `test-types` and `lint-packages`. `lint-tests` runs every suite with `--list --forbid-only` ([lint_tests.js:23-31](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/lint_tests.js#L23-L31)). `lint-packages` checks workspace version consistency and lockfile v3 ([workspace.js:212-220](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/workspace.js#L212-L220)).
- **ESLint** ([eslint.config.mjs](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs)) carries style rules (CONTRIBUTING: "Coding style is fully defined in eslint.config.mjs") and architecture rules:
  - `no-restricted-properties` bans `process.exit`, `process.stdout` and `process.stderr` in all of `packages/` ([L345-L355](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs#L345-L355)).
  - In the injected code, web globals must come from captured builtins ("Use InjectedScript.utils.builtins.setTimeout instead", [L226-L243](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs#L226-L243)). This guards against pages that override globals and against clock emulation; see [utilityScript.ts:36-66](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/injected/src/utilityScript.ts#L36-L66).
  - `import/order` and a copyright `notice` header are required.
  - `no-floating-promises` covers `playwright`, `isomorphic`, `injected`, `client`, `protocol` and tests, **but not `server/`**.
- **Custom type-aware lint rule `progress/await-must-use-progress`** ([eslint-plugin-progress/index.js:119-129](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/eslint-plugin-progress/index.js#L119-L129), enabled for `server/` and `utils/` at [eslint.config.mjs:488-500](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/eslint.config.mjs#L488-L500)). In any function that takes `progress: Progress`, every awaited async call must pass `progress` first or be wrapped in `progress.race()`. Cancellation discipline is checked by the compiler, not by review.
- **Type-check.** Root `tsc` with `strict` over `packages/`, plus `tests/tsconfig.json`, plus type tests of the generated `.d.ts` (`utils/generate_types/test`).
- **Tests.**
  - `tests/page` (136 specs) holds behaviour written once and run per engine and per `PWPAGE_IMPL`.
  - `tests/library` (162) covers lifecycle and features.
  - `tests/playwright-test` (104) runs the test runner through a **pinned published release**, `@playwright/test ^1.64.0-alpha` ([stable-test-runner/package.json:4](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/playwright-test/stable-test-runner/package.json#L4)), so the runner under test never tests itself.
  - `tests/mcp` (102), `tests/installation` (22), plus Android, Electron, BiDi, WebView and stress suites.
  - A `driver` test mode runs the library suite through the out-of-process pipe ([testMode.ts:19-50](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/config/testMode.ts#L19-L50)). I found no workflow in `.github` that sets `PWTEST_MODE`, so it is not verified in CI.
  - There are 299 browser-specific `skip`/`fixme`/`fail` annotations out of 720 in `page` and `library`.
- **CI.**
  - `infra.yml` builds, runs `npm run lint`, then **fails if the tree is dirty** after the build ([infra.yml:25-32](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/infra.yml#L25-L32)), which proves generated files are committed. The dependency audit runs with `continue-on-error`.
  - The `lint-snippets` job installs Python, .NET and Java and compiles the doc snippets in every language ([infra.yml:36-58](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/infra.yml#L36-L58)).
  - The test matrix covers {chromium, firefox, webkit} × Linux, macOS and Windows ([tests_primary.yml:38](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/tests_primary.yml#L38), [tests_secondary.yml:31](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/tests_secondary.yml#L31)), branded channels (Chrome stable and beta, Edge stable and dev), Bun, Electron, Android, Docker, a WebView simulator and the VS Code extension.
  - Large self-hosted runners are used (`playwright-x64-ubuntu24-64-core`).
- **Contribution rules.**
  - Unsolicited PRs without an approved issue are closed, and "Low-Quality AI Contributions … will be closed" ([CONTRIBUTING.md:14-15](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/CONTRIBUTING.md#L14-L15)).
  - A test is required for almost any change (L59). Commit messages follow Conventional Commits. Comments must "have an explicit purpose" (L47).
  - The Copilot review prompt limits reviews to semantic issues ([copilot-instructions.md](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/copilot-instructions.md)).
- **Design docs.** Architecture, API-change, tools, vendoring and trace-system guides are in `.claude/skills/playwright-dev/` (2,686 lines). There are no ADRs.

## 8. Pain points

What maintainers or users say (issue tracker):

- **Plugins are deliberately hidden.** The test-runner plugin system was hidden in PR #13908 and will not be exposed: "There are no plans to expose it as of today" (maintainer, [#28615](https://github.com/microsoft/playwright/issues/28615)). Requests for codegen plugins were closed ([#28474](https://github.com/microsoft/playwright/issues/28474), [#29970](https://github.com/microsoft/playwright/issues/29970)). The extension surface is kept small on purpose.
- **Test-runner source transform.** TypeScript, Babel, ESM and `tsconfig` handling collected a 35-comment meta-issue ([#7121](https://github.com/microsoft/playwright/issues/7121)) and follow-ups such as [#12829](https://github.com/microsoft/playwright/issues/12829) and [#7066](https://github.com/microsoft/playwright/issues/7066). The transform layer (`playwright/src/transform`, 1k lines) is a long-running source of friction.
- **Version lockstep.** Client and remote server must match major.minor ([class-browsertype.md:95](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/docs/src/api/class-browsertype.md#L95)). The server's error text tells users to "keep your local Playwright version in sync" ([playwrightServer.ts:341-350](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/remote/playwrightServer.ts#L341-L350)).
- **Caveat.** `gh search` gave noisy results for architecture topics. I found no maintainer statements about internal code quality, so the next list is my own reading.

My inference from the code:

1. **God classes.** `Frame` (about 1,350 lines) and `InjectedScript` (about 1,740) take on every new action. Nothing limits size.
2. **The engine abstraction leaks into the interface.** `PageDelegate` holds named workarounds for single browsers ([page.ts:99-109](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/page.ts#L99-L109)). Tests carry 299 browser-specific skips.
3. **The WebView backend was copied** (50–98% identical files) instead of sharing code with WebKit.
4. **Configuration sprawl.** About 141 environment variables are read ad hoc. MCP options are declared six times.
5. **Rot in the guard rails.** Five stale `DEPS.list` sections, a `src/common/` folder holding only `DEPS.list`, "TODO remove?" on `Connection.headers`.
6. **Two error stacks** (client and server) with duplicated classes and name-string reconstruction. About 25 scattered `Error` subclasses with name clashes.
7. **The client is thick and repeated in each port.** Timeouts, option normalization and waiters live in client code that every language repo re-implements. The 6-step API process means at least 4 hand-written places per method in this repo alone.
8. **Weak spots in the lint config.** `useUnknownInCatchVariables: false` and widespread `as any` (269 in `client`/`server`/`tools`). No `no-floating-promises` on the server, the most async-heavy code.

## 9. Scorecard

| Dimension | Score (1–5) | Justification |
|---|:-:|---|
| Layering and dependency direction | 4 | Strict client/server split enforced by `check_deps.js` and ESLint. Composition root for backends. `isomorphic` is a true leaf. Minus: type-only imports exempt, `**` wildcards, 5 stale rules. |
| Module and file size discipline | 3 | Median file is 127 lines, but 13 files exceed 1,000 lines and `Frame` and `InjectedScript` are god classes. No size rule. |
| DRY (including language ports) | 3 | Excellent codegen from protocol and docs. But the API and protocol are two hand-mapped contracts, the client is thick in each port, the WebView backend is a copy, config is declared six times, and examples are written in four languages. |
| Separation of concerns | 4 | Client, dispatcher, server and backend layers. Instrumentation listeners. Metadata-driven tracing, pause and slowMo. Minus: the server domain uses wire types, and `PageDelegate` carries workarounds. |
| Extension model | 3 | Internal seams are good (instrumentation, factories, `defineTool`). The public surface is narrow: selectors, fixtures, reporters, matchers. Plugins hidden on purpose. |
| Contracts and types | 4 | Generated typed channels, runtime validation both ways, docs → `.d.ts` → `implements`. Minus: unversioned lockstep protocol, `any`-heavy code. |
| Error model | 3 | Small public set and typed `errorDetails` per method. Minus: name-string dispatch, duplicated client and server classes, error classes declared per language in docs. |
| Test architecture | 4 | One behaviour suite across engines through fixtures, hermetic server, pinned-release runner, lifecycle tests. Minus: huge spec files, many engine skips, driver mode not run in CI. |
| Tooling and CI enforcement | 4 | Drift-failing codegen, clean-tree check, custom type-aware Progress rule, snippet compilation in 4 languages, deps check. Minus: stale-rule blind spot, no `no-floating-promises` on the server, audit not blocking. |
| In-code documentation | 3 | Public API documentation is exhaustive. Code comments are sparse (about 15% of lines) and purpose-driven. Architecture docs exist but sit in agent-skill files, with no ADRs. |

## 10. Lessons for Goodfetch

Mapping notes:

- Playwright's YAML → TS/.NET codegen is the analogue of Goodfetch's protocol crate → `xtask codegen`.
- Playwright's Node "driver" is the analogue of the Attached engine.
- ESLint `no-restricted-properties` maps to Clippy restriction lints, for example `print_stdout` and `print_stderr`.

1. **Adopt: drift-proof codegen in CI.** Keep the "regenerate, then fail if anything changed" step ([generate_channels.js:488](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L488)) and the clean-tree check after build ([infra.yml:25-32](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/infra.yml#L25-L32)). Also check meaning inside the generator, for example "every non-internal command has a title" ([L317-L352](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/utils/generate_channels.js#L317-L352)). Unlike Playwright, give the generator its own tests.
2. **Adopt: declare cross-cutting behaviour per command in the protocol crate.** Examples: snapshot, pause, slow-motion, title for logs, internal vs public. Generate a metadata table and let the dispatcher and tracing read it, as `flags` and `methodMetainfo` do in Playwright (§4). This keeps handlers free of tracing code.
3. **Avoid: two hand-mapped contracts.** Playwright needs Markdown for the API plus YAML for the protocol, and a 6-step manual chain per method (§5). Goodfetch should have **one** Rust source from which the public SDK surface, the wire protocol and the API docs are all generated, or encode any API→protocol mapping in that source.
4. **Avoid: a thick per-language client.** Timeouts, default values, option normalization and waiting belong in the engine. Then every generated SDK stays a thin, mechanical layer. Playwright pays for this in each port: Python's hand-written `_impl/_page.py` is 56 KB.
5. **Adopt: the embedded path goes through the same dispatch layer.** Playwright's in-process JS client still sends protocol messages ([inprocess.ts:27-62](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/inprocess.ts#L27-L62)), with raw buffers as the only shortcut. The Rust SDK and the engine embedded in Foxhound can then behave exactly like remote SDKs. **Also run the full suite in the out-of-process mode in CI.** Playwright has the mode but no workflow uses it.
6. **Adopt: a cancellation and deadline object on every async engine call, enforced by a tool.** Pass `Progress` as the first argument, use `race`, cancel through `__abort__` (§4). Enforce it with a custom lint, as Playwright does with `await-must-use-progress`. In Rust, use a dylint or `xtask` check, or make the type system require `&Progress` on the engine traits.
7. **Adopt: machine-checked module boundaries with a single composition root.** Only one module may name the concrete backends ([server/DEPS.list:23-29](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/server/DEPS.list#L23-L29)). The Cargo crate graph enforces only the coarse layers, so add an `xtask` check for module-level rules. **Unlike `check_deps.js`, also flag rules that name missing files and do not exempt type-only imports.**
8. **Avoid: backends copied because the protocol client is typed to one schema.** WebView copies WebKit at 50–98% (§5). If Collie ever drives a second CDP flavour (Electron, WebView or a vendor fork), make the session and domain layer generic over the protocol types rather than forking files.
9. **Adopt: one settings schema for every config surface.** CLI flags, environment variables, config-file keys, docs and capability toggles should all be derived from one place. Playwright's MCP declares each option six times by hand and reads about 141 environment variables ad hoc. Its capability filter for MCP tools ([tools.ts:83-84](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/tools/backend/tools.ts#L83-L84)) supports Goodfetch's rule that capabilities are runtime settings.
10. **Adopt: versioned persisted formats with frozen per-version types and a chained upgrader.** This is Playwright's `traceV3..V10` plus `_modernize_N_to_N+1` (§4). Use typed dispatch instead of string method lookup. This matches Goodfetch's versioned-reader commitments.
11. **Decide early: wire-protocol versioning.** Playwright's protocol has no version and depends on major.minor lockstep, enforced only by a User-Agent check on the remote server. If Goodfetch SDKs may talk to an engine of a different version (Goodfetch Cloud, Proving ground), put a protocol version and capability negotiation in the handshake before the first publish.
12. **Adopt typed errors in the schema; avoid name-string errors.**
    - Keep: typed `errorDetails` per command ([frame.yml:873](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/protocol/spec/frame.yml#L873)), the call log attached to errors, and turning "target closed" into one well-known error with the close reason.
    - Avoid: name-string error reconstruction with copies of each class on both sides ([client/errors.ts:58-77](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/packages/playwright-core/src/client/errors.ts#L58-L77)).
    - Goodfetch: a generated error-code enum, mirrored into each SDK.
13. **Adopt: one behaviour suite, many backends; and the runner tested by a pinned release.** The page suite is chosen by fixture ([pageTest.ts:31-38](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/page/pageTest.ts#L31-L38)). The runner suite uses a pinned published release ([stable-test-runner](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/tests/playwright-test/stable-test-runner/package.json#L4)). Also compile the doc snippets in every SDK language in CI ([infra.yml:36-58](https://github.com/microsoft/playwright/blob/0d7c1411b0afdf8b78664c490ee473857690d66c/.github/workflows/infra.yml#L36-L58)).
