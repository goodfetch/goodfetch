## Browser tests and the docs site

Decided in [ADR 0068](../adr/0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md), as [ADR 0137](../adr/0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md) amends it. Which accessibility, localization and visual checks run on these runners is in [Accessibility, localization and visual regression](accessibility-localization-and-visual-regression.md).

### Where each test lives

Each suite contains the one before, so the table gives the first suite that runs each test; [Browsers](#browsers) says which browsers each suite adds.

| Path | What it tests | Runner | First suite |
|---|---|---|---|
| a TypeScript package's `*.test.tsx` | one component, or a test-support helper that needs the DOM | Vitest `component` (Browser Mode) | Pre-merge |
| a TypeScript package's `*.spec.ts` | a page served to a real browser | Playwright Test | Pre-merge |
| the live-view packages' `*.test.ts(x)` | `<goodfetch-live-view>` and its React wrapper | Vitest Browser Mode | Pre-merge |
| `website/src/**/*.test.ts` | the docs site's build code | Vitest (Node) | Quick |
| `website/tests/` | the built docs site | Playwright Test | Pre-merge |
| `docs/**/*.md(x)` code blocks, and the published packages' READMEs | the docs code samples, extracted by `xtask docs-samples` | `twoslash` in the docs build; the TypeScript SDK's Vitest, the Python SDK's pytest, rustdoc, trycmd, the MCP server's tests | Pre-merge |
| the files beside each migration guide, with their manifest | the guide's "before" programs, never run | tsc, pyright or rustc against the pinned competitor | Pre-merge |

### Components

- **A test that renders or needs the DOM is a `.test.tsx` file** and runs in Vitest Browser Mode with the Playwright provider. A `.test.ts` file runs in Node and never touches the DOM. Never jsdom or happy-dom.
  _Why:_ Base UI's positioning, focus and pointer handling need a real layout.
- **Mount with `vitest-browser-react`, find with `vitest/browser` locators, act with `vitest/browser`'s `userEvent`.** Never `@testing-library/user-event`.
  _Why:_ events are then real browser input sent by Playwright, and the locators are Playwright's.
- **Use the shared Vitest fixture of `@goodfetch/test-support`; it resets hover in `beforeEach`** (`userEvent.unhover(document.body)`). It is the `component` project's setup file, `@goodfetch/test-support/vitest-browser/setup`, whose `hoverReset` the root `vitest.config.ts` runs before every `.test.tsx` test.
  _Why:_ the Playwright provider keeps the pointer position and hover state between tests of one file, and tests run shuffled.
- **Test data comes from builders with literal defaults.** Override only what the test asserts on; no random data.

### Browser state

- **Set colour scheme, reduced motion, locale, time zone and online status through Playwright:** context options (`colorScheme`, `reducedMotion`, `locale`, `timezoneId`), `page.emulateMedia` and `context.setOffline` in served tests; the provider's context options and the `@goodfetch/test-support` browser command in component tests.
  - In a component test, `browserState({ colorScheme, reducedMotion, locale, timeZone, online })` from `@goodfetch/test-support/vitest-browser` sets any of the five until the test ends, when the page gets the project's state back. Colour scheme and reduced motion work in every browser. Locale, time zone and online status are DevTools Protocol overrides, so only Chromium takes them per test; elsewhere the command fails with `provider_unsupported`, and a project sets them in its provider's context options.
  - Online status there is `navigator.onLine` and its events, never `context.setOffline`, which would also cut the page's own connection to Vitest and hang the run.
- **Never redefine a property of `document`, `navigator` or `window`.** oxlint's `no-restricted-properties` refuses `Object.defineProperty`, `Object.defineProperties` and `Reflect.defineProperty` in test files, and `vi.stubGlobal` is banned ([Writing a test](writing-a-test.md)).

### Served tests

- **A served test imports `test` and `expect` from `@goodfetch/test-support/playwright`,** and gets its browser context, pages and API requests from there. Never write a cookie or storage state yourself: the fixtures refuse `storageState`, and oxlint refuses `addCookies`, `setStorageState` and `document.cookie` in test files.
- **Make faults where they happen:** `routeWebSocket` with `connectToServer()` for a dropped or stalled connection, the real process for an engine fault. `routeWebSocket` replaces the page's `WebSocket` with a shim that forwards to the real binary; use it only to drop or delay real messages, never to invent one.
- **Find by role, label or text.** `data-testid` only where an element has no accessible role or name; never CSS classes or XPath.
- **Wait with web-first assertions and `expect.poll`.** Never `waitForTimeout` (oxlint refuses it) or `networkidle`.
- **A console error, an uncaught page error, a CSP violation or a failed same-origin request fails the test**, through the shared fixture's `failOnErrors`. A CSP violation is seen twice: by the page's `securitypolicyviolation` event and as the report the browser sends to the CSP report endpoint (`application/csp-report` or `application/reports+json`). A same-origin request fails when it gets no answer or an HTTP status of 400 or above; one the browser cancelled because the page navigated away or closed does not. A test tool's own fault is the one exception: an entry of `known-issues.ts` in `@goodfetch/test-support` names the browser and the Playwright version it was seen on, its upstream report, and the errors it raises, and the helper whose action raises them tolerates exactly those errors around that action alone (`PageErrorWatch.tolerating`): a CSP violation the entry matches, and beside each one console message and one report it matches. Tolerated errors fail nothing and are attached to the report as `known-issues`. A Vitest test fails when the installed Playwright moves, so each bump checks every entry again. The one entry: Playwright inserts an inline `<style>` before every WebKit screenshot, which the production CSP refuses, so the shared screenshot helper tolerates that refusal, from no source file, during its comparison.
- **`toMatchAriaSnapshot` and Vitest's ARIA snapshots follow [Snapshots](snapshots.md).**
- **Tests run under the page's own CSP, never a relaxed one.** No Product serves a page, so the policy these rules call the production CSP is the strict one test support's fixture pages serve (`test-support/typescript/tests/csp-pages.ts`: the page's own scripts and styles only, Trusted Types required, violations reported to the page's origin). Playwright's `bypassCSP` is never set: the shared fixture fails a test that sets it, and in test files oxlint refuses `newContext`, `browser.newPage`, `launchPersistentContext`, `launch`, `connect` and `connectOverCDP`, so every browser and context comes from the fixture. `context.newPage()` is allowed; API calls use the fixture's `request`.
- **Keep traces on failure** and upload them with the JUnit files: `playwright.config.ts` keeps a failed test's trace (`trace: 'retain-on-failure'`) in its output folder, `target/junit/playwright/`, beside the JUnit file.
- **The tools served tests rely on are proved under the production CSP's Trusted Types** on Chromium, Firefox and WebKit (Firefox and WebKit in the Release suite, whose cells install every browser of the locked Playwright before the deny, `ci/host/fetch.sh`; until a Release run has passed, local runs on macOS and on Linux x86-64 show them there, [ADR 0068](../adr/0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md)'s note of 2026-11-21), by `test-support/typescript/tests/trusted-types.spec.ts`: `page.clock`, `routeWebSocket`'s shim with `connectToServer()`, axe's injected script and the virtual authenticator's `credentials.install()`, each its own test on a fixture page that sets `require-trusted-types-for 'script'`. A tool that fails there is replaced or reported to the owner; the CSP is never relaxed.

### Browsers

- **One Playwright version for the whole pnpm workspace**, shared by Vitest's provider.
- **The browser tests and the docs site's tests run on one Node line;** their matrix is browsers.
- **Pre-merge:** Chromium on Linux. **Release:** components, the live-view packages and served tests on every OS runner with every browser Playwright ships for it, plus phone emulation on Chromium and WebKit for served tests; the docs site's build, lychee and Playwright on Linux, on Chromium, Firefox and WebKit; the docs code samples as [Docs code samples](#docs-code-samples) says. `playwright.config.ts` adds its `firefox`, `webkit`, `translation` and `elsewhere` projects beside Chromium's (and, in the pinned image, the shared visual config its `visual-firefox` and `visual-webkit` projects), when `GOODFETCH_TEST_SUITE` is `release`; the CI image carries Chromium.

### Docs site

- **Docusaurus throws on every broken reference:** `onBrokenLinks`, `onBrokenAnchors`, `onDuplicateRoutes` and `markdown.hooks`' `onBrokenMarkdownLinks`, `onBrokenMarkdownImages` and `onUnusedMarkdownDirectives`.
- **Every kept version is built, and every page has its `.md` twin, an `llms.txt` entry and its content in `llms-full.txt`,** in its own Product's section (`/docs/collie/`, `/docs/foxhound/`), or for a root page in the root's; the root `llms.txt` lists both sections' files and the root pages ([ADR 0097](../adr/0097-one-docs-site-renders-each-products-section-from-one-source-and-neither-product-leads-with-access.md)).
- **lychee runs with `--offline --include-fragments --root-dir <root>` in the suites**, over the built site copied to `<root>/docs/`, so absolute `/docs/` links resolve. External links are the Live check `docs-external-links`.
- **Playwright tests the built site served locally under `/docs/` on port 0,** never a Vercel preview: every `sitemap.xml` page of every kept version answers 200 with no console, hydration or same-site request error, and the interactive components work. Search is tested against an index built with the site; a hosted search service is tested only by `docs-deploy-smoke`.
- **After each production deploy, the Live check `docs-deploy-smoke`** checks the deployed pages, and a `.md` page, `llms.txt` and `llms-full.txt` of each Product's section and of the root, with their content types.
- **`typos` checks the docs' spelling.** No Vale, no cspell.
- **The wording check, `xtask wording`, runs in the Quick suite** ([ADR 0097](../adr/0097-one-docs-site-renders-each-products-section-from-one-source-and-neither-product-leads-with-access.md)): it fails on a term of ADR 0035's list ("bypass" in any form, "undetectable", "undetected", "solve any CAPTCHA"), word-bounded and case-insensitive, in the prose of the docs pages (not `docs/adr/`, `docs/agents/`, `docs/testing/` or `rnd/`), READMEs, `CHANGELOG.md`, skills, the catalog's, settings schema's and Error codes' texts, and package descriptions and keywords. Code is skipped. Rewrite the sentence; extend the list only on counsel's review.

### Docs code samples

- **Give every fenced block in `docs/` a language;** use `text` for output and prose. The docs build fails on an empty info string.
- **`xtask docs-samples` extracts every sample** from `.md` and `.mdx` pages into `target/docs-samples/`, with a manifest. The docs build's remark plugin lists every sample block it sees; a difference between that list and the manifest, or a runner whose case count differs from the manifest, fails the check.
- **Tag a block that can't run `no_run`** after its language, and an MCP pair on either of its blocks (`json mcp-request no_run`). A `rust`, `ts` or `python` block is still compiled, type-checked or linted; a `console` block or an MCP pair is only listed in the manifest.
- **Mark MCP samples `json mcp-request` and `json mcp-answer`.** Other `json` blocks are not samples.
- **Each surface's tests run its samples:** `twoslash` type-checks `ts` and `tsx` in the docs build (our own remark plugin, on the TypeScript 6 alias) and the TypeScript SDK's tests run them; Ruff lints and the Python SDK's pytest runs `python`; `#[cfg(doctest)]` includes in `goodfetch-collie` and `goodfetch-foxhound` run `rust`; trycmd runs the `.trycmd` files from each executable crate's `tests/it` (`...` for any lines, `[..]` within a line); an MCP server test plays each `mcp-request` and `mcp-answer` pair through `rmcp`.
- **Run samples against the binary built from the commit and a local Proving ground,** never the online one. Hide setup with the tool's own convention (`# ` in Rust, `---cut---` in Twoslash). Write the online Proving ground's origin, `https://proving-ground.goodfetch.dev`, as a reader would: `xtask docs-samples extract` writes `__GOODFETCH_DOCS_ORIGIN__` in its place, for the runner to replace with the local one, and a `rust` string literal that starts with it as a macro reading `GOODFETCH_DOCS_ORIGIN` when the doctest compiles. The manifest lists the lines it replaced.
- **Run a sample against the executable of the Product its page documents,** Collie's for what both Products carry, extracting it from each Product's rendering ([ADR 0097](../adr/0097-one-docs-site-renders-each-products-section-from-one-source-and-neither-product-leads-with-access.md)). A page rendered for both Products runs its samples once, on Collie's executable; its Foxhound rendering is checked statically against Foxhound's packages, never run: Twoslash for `ts`, Ruff and pyright for `python`, a `no_run` doctest for `rust`. A page rendered for Foxhound only runs on Foxhound's executable. No code block of a Product's rendering, `console` and MCP pairs included, may carry a name from the generator's list of the other Product, except in a part marked as naming that Product, which the generator leaves as written and checks statically against that Product's packages. Every published package's README is extracted too, and its samples run on that package's Product's executable.
- **Keep a migration guide's "before" programs out of the samples:** each is a file beside its guide, with a manifest pinning the competitor's version, type-checked against it (tsc, pyright, rustc) and never run, and the page includes the file rather than a fenced block. The type-check runs in Pre-merge when the guide or its manifest changes, and in the Release suite.
- **Only `docs/` (`next`) runs.** Frozen versions were tested at their release.
- **Pre-merge:** the affected samples on one runtime. **Release:** every sample on its surface's own matrix ([SDK and CLI tests](sdk-and-cli-tests.md), [MCP, framework adapters and Integrations](mcp-framework-adapters-and-integrations.md)).
