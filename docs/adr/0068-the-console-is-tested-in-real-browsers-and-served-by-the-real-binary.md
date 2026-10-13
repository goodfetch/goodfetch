# The Console is tested in real browsers against an in-memory engine and served by the real binary, and the docs site as it is built

The Console's components and screens are tested in real browsers with Vitest Browser Mode, against an in-memory adapter of the protocol that the Console's Client takes as a value. Storybook keeps a story for each state. Everything a browser does with the Console as users get it runs on Playwright Test against the `goodfetch` binary built from the commit, under its production CSP: the Console's served tests, its Journey variants, and the Console conformance suite of ADR 0031, which also runs against a stub gateway. The docs site is tested as it is built: strict Docusaurus checks, lychee offline, and Playwright over every page. What depends on other people's servers runs as Live checks.

No ADR fixed how the Console and the docs site are tested. The Console is React with shadcn on Base UI and Vite in the [V1 Console prototype](https://github.com/Jbmanllr/automan/issues/80), embedded in the binary, talking JSON-RPC over a same-origin WebSocket under a strict CSP with Trusted Types (ADR 0031). Comparable projects show the shape:
- **Base UI**, the Console's component library, tests itself in Chromium through Vitest Browser Mode by default; jsdom "does not do any layout", and Supabase Studio polyfills pointer events and `ResizeObserver` by hand so its Radix menus work there.
- **Every comparable console** (Grafana, Supabase Studio, Prefect, PostHog, Temporal UI) runs Playwright against its real backend, and all of them retry. Grafana's strict-CSP run exists but is not a gate.
- **Storybook's Jest test runner** loses support in Storybook 11; its Vitest addon runs stories as Browser Mode tests.
- **Docusaurus** only warns on broken anchors and Markdown links by default, and screenshots every `sitemap.xml` page of its own site with Playwright.

Decided in [Console and docs test stack](https://github.com/Jbmanllr/automan/issues/282) ([grilling record](https://github.com/Jbmanllr/automan/issues/282#issuecomment-6152362340)), with the agent answering on the owner's delegation. The facts come from [Testing a web console and a docs site](https://github.com/Jbmanllr/automan/issues/265) ([findings](https://github.com/Jbmanllr/automan/blob/research/web-console-testing/rnd/research/2026-10-13-web-console-testing.md)) and [Testing TypeScript and Python SDKs over a separate engine](https://github.com/Jbmanllr/automan/issues/266) ([findings](https://github.com/Jbmanllr/automan/blob/research/sdk-testing/rnd/research/2026-10-13-sdk-testing.md)). It builds on ADR 0059 and on ADRs 0061 to 0064 (suites, layout, Journeys, no retries and Live checks). The rules every test follows are in `TESTING.md`.

## Decision

### Components: Vitest Browser Mode

- **Two Vitest projects in `console/`:**
  - `unit`: `src/**/*.test.ts` in Node, for logic with no DOM (state, protocol mapping, formatting, URL state, the reconnection policy). It is in the Quick suite.
  - `component`: `src/**/*.test.tsx`, stories and `tests/app/`, in Vitest 5 Browser Mode with `@vitest/browser-playwright`. It starts a browser, so it is in the Pre-merge suite (ADR 0061).
- **jsdom and happy-dom are not used.** Base UI's positioning, focus and pointer handling need a real layout.
- **Mount with `vitest-browser-react`, find with `vitest/browser` locators, act with its `userEvent`.** Events go through CDP, and the locators are Playwright's. `@testing-library/user-event` is not used: Vitest's guide says it "simulates events instead of actually triggering them".

### The engine below the browser tier

- **The Console's Client takes its transport and its body fetcher as values** (ADR 0059). Component and app tests pass an **in-memory adapter of the protocol**:
  - it answers catalog operations with values the test sets, typed by the generated catalog types;
  - it pushes events with positions, drops the connection and forgets positions on request;
  - it serves page bodies, screenshots and Exports by handle;
  - it records every request;
  - it validates every request and answer against the committed JSON Schema bundle, so a test cannot hand the Console what the engine could never send;
  - a call the test did not answer fails the test.
- **It lives in `@goodfetch/test-support`** (ADR 0062). If [SDK and CLI test stack](https://github.com/Jbmanllr/automan/issues/285) builds the TypeScript SDK's fake engine with this interface, the Console uses that one.
- **MSW is not used.** It replaces the global `WebSocket`, knows nothing of the catalog, and can send what no engine sends. `vi.mock` and `vi.spyOn` stay banned (ADR 0062).
- **Test data comes from builders with literal defaults** over the generated types; a test overrides what it asserts on. No random data generator.

### Storybook as the catalog of states

- **Storybook 10** (`@storybook/react-vite`, `@storybook/addon-vitest`) keeps `*.stories.tsx` beside each component: one story per state, with the in-memory adapter's data.
- **Every story renders as a test** in the `component` project and fails on a thrown error. Stories hold no assertions and no play functions.
- **Behaviour tests are `.test.tsx` files**, named by ADR 0062's rules, and may reuse a story's arrangement through `composeStories`.
- Storybook 11 is adopted when it is stable.

### The served Console: Playwright Test against the binary

- **Every browser test of the Console as users get it runs on Playwright Test** against the `goodfetch` binary built from the commit, serving its embedded Console. Never against a Vite server.
  - `console/tests/served/`: the Console's integration tests through the served app.
  - `console/journeys/`: the `console` variants of Journeys, tagged `@J-NNN` (ADR 0063).
  - `conformance/console/`: the Console conformance suite (below).
- **Each test starts its own Daemon** through a Playwright fixture of `@goodfetch/test-support`, which calls `goodfetch-test-env` (ADR 0062): its own Goodfetch home, cleared environment, port 0, and its own Proving ground when it needs pages. Each test gets a fresh browser context.
- **Sign-in follows the product's own path:** the single-user sign-in link the binary gives, Playwright's virtual WebAuthn authenticator for passkeys and step-up, and a local OIDC provider, pinned by digest, for single sign-on. A test never writes a cookie or storage state it did not get from the Console.
- **Time:** the server's durations are short settings (ADR 0059); the browser's clock is `page.clock`.
- **Faults:** a dropped or stalled connection uses `routeWebSocket` with `connectToServer()`, between the real browser and the real binary; an engine fault acts on the real process.
- **Writing rules:**
  - locators by role, label or text; `data-testid` only for an element with no accessible role or name; never CSS classes or XPath;
  - web-first assertions and `expect.poll`; never `waitForTimeout` (refused by oxlint's `no-restricted-properties`) or `networkidle`;
  - every test fails on a console error, an uncaught page error, a CSP violation or a failed same-origin request, through the shared fixture;
  - traces are kept on failure and uploaded with the JUnit files (ADR 0061).
- **CSP:** the binary has no switch that relaxes its CSP, as ADR 0059 refused test-only clock controls. Every served test runs under it. One served test proves an injected inline script and a string-to-script sink are blocked. The binary crate's `tests/it` checks every Console route's headers with an HTTP client, which is ADR 0031's "CI checks these headers".

### The Console conformance suite

- **It is the Console's browser suite pointed at a backend.** It runs on Playwright Test, takes its target's endpoint and authentication as configuration, and writes area 2's result document with `kind = "conformance"` for each target.
- **Targets:** the Daemon; the stub gateway; Goodfetch Cloud's gateway, run from the private repository.
- **Cases** cover what the Console needs from a backend: `runs.list`, event resumption from a position and the snapshot when the position is gone, lossless events, "Needs you", large files by handle, sign-in and step-up, the SSE-plus-`POST` fallback, the read-only banner and reconnection, the content origin, and the security headers.
- **The stub gateway** is a test-only executable in the public repository (`publish = false`). It serves the same Console with the same headers, starts an Attached engine per run, keeps its own index for `runs.list`, forwards every other operation to the engine that owns the run, and authenticates the way the Cloud gateway does. It proves the Console uses nothing a Daemon offers beyond the catalog.

### The live-view packages

- `<goodfetch-live-view>` and its React wrapper (ADR 0031) get Vitest Browser Mode tests and `publint` and `@arethetypeswrong/cli`, like every package.
- Served tests embed the component in a page on a second loopback origin with a grant from the real binary: allowed and refused `frame-ancestors`, view-only input dropped by the engine, a control grant resolving a Human pause, expiry and revocation.

### Browsers and suites

- **One Playwright version for the pnpm workspace**, a catalog entry that Vitest's provider shares. Its browsers are fetched before ADR 0043's deny and cached by version.
- **One Node line** (the newest ADR 0024 supports) runs the Console's and the docs site's tests. Their matrix is browsers.

| Suite | Console | Docs site |
|---|---|---|
| Quick | `unit` | build code's Node unit tests |
| Pre-merge (Linux) | `component` on Chromium; served tests, Journey variants and conformance (Daemon and stub gateway) on Chromium, as `xtask affected` selects | strict build of every kept version, `lychee --offline`, build-output checks, Playwright on Chromium, when `docs/`, `website/` or the catalog change |
| Release | `component` on Chromium, Firefox and WebKit (Linux); served tests, Journey variants and conformance on every OS runner, with every browser Playwright ships for it, plus Chromium and WebKit phone emulation | the same on Chromium, Firefox and WebKit |

### The docs site

- **Docusaurus fails the build on every broken reference:** `onBrokenLinks`, `onBrokenAnchors`, `onDuplicateRoutes`, `markdown.hooks.onBrokenMarkdownLinks`, `markdown.hooks.onBrokenMarkdownImages` and `markdown.hooks.onUnusedMarkdownDirectives` are `throw`.
- **Every kept version is built.** A build-output test checks that every page has its `.md` twin and an entry in `llms.txt`.
- **lychee runs offline in the suites:** `lychee --offline --include-fragments` over the built site. External links are a nightly Live check, `docs-external-links`.
- **Playwright Test in `website/tests/`** runs against the built site served locally under `/docs/` on port 0, never against a Vercel preview:
  - one test per `sitemap.xml` page of every kept version, named by its path, failing on a non-200 answer, a console error, a hydration error or a failed same-site request;
  - tests of the interactive components (language tabs, version switcher, copy buttons, search).
- **Build code** (plugins, generators) has Node unit tests in the Quick suite; generated pages follow ADR 0062's snapshot rules.
- **After each production deploy**, a Live check, `docs-deploy-smoke`, loads the home page, a page of each kept version, a `.md` page and `llms.txt` through `goodfetch.dev/docs`, and checks their content types.
- **No Vale and no cspell.** `typos` already reads `docs/`. Many `_Avoid_` terms of the glossary are ordinary words, so the review checks terms.

### Where accessibility, localization and visual checks plug in

[Accessibility, localization and visual regression](https://github.com/Jbmanllr/automan/issues/283) decides what is checked and to what standard. These are its runners:
- **Accessibility:** `@axe-core/playwright` through a shared fixture of the served tests and the docs pass; Storybook's a11y addon on every story; oxlint's `jsx-a11y` plugin; keyboard and focus through Playwright and Vitest's `userEvent`.
- **Visual:** Vitest's `toMatchScreenshot` on stories, Playwright's `toHaveScreenshot` in served and docs tests.
- **Localization:** a locale is a parameter of a Vitest project and of a Playwright project, so a pseudo-locale or a right-to-left locale runs the same tests.

## Considered Options

- **jsdom or happy-dom for components:** rejected. They do no layout, and Base UI's primitives need hand-written polyfills there. Base UI itself tests in Chromium.
- **Testing Library's `user-event`:** rejected. Vitest's guide says it simulates events in Browser Mode.
- **MSW for the WebSocket:** rejected. It patches a global, and its handlers are not checked against the catalog.
- **Playwright's story-and-gallery component testing (1.62):** rejected for now. It is new, needs a gallery page we own, and Storybook's Vitest addon gives the same real-browser tests with a catalog people can browse.
- **No Storybook:** rejected. The Console has many states per screen, and #283 needs one place to audit and screenshot each.
- **Play functions as interaction tests:** rejected. Behaviour would live in two places, outside ADR 0062's naming.
- **Cypress or WebdriverIO end to end:** rejected. Cypress runs inside the page, which the CSP and Trusted Types fight, and neither shares Vitest's provider.
- **A mocked-API browser tier against `vite preview`** (Temporal UI): rejected. It is not the shipped artefact and carries no CSP; the in-memory adapter already covers mocked engines below.
- **One Daemon per Playwright worker:** rejected. State would leak between tests run in random order (ADR 0064).
- **A test switch that relaxes the CSP:** rejected, as ADR 0059 rejected test-only controls in the binary.
- **A protocol-only Console conformance suite:** rejected. It would repeat the catalog's conformance and not show the Console works on a backend.
- **lychee online in every pull request:** rejected. Outbound traffic is denied (ADR 0043), and other people's outages would fail changes.
- **Docs tests against Vercel previews** (Supabase): rejected. The preview is not pinned, and suite jobs are offline.
- **Vale with the glossary's `_Avoid_` terms, or cspell:** rejected. `typos` already runs, and a mechanical term rule would flag ordinary words.

## Consequences

- **Adds to ADR 0031:** what its Console conformance suite is, its targets and cases, and the stub gateway; the headers check is an HTTP test in the binary crate; the live-view packages' tests.
- **Adds to ADR 0030:** the strict Docusaurus settings, the build-output check, lychee offline in the suites, and the docs Playwright pass.
- **Adds to ADR 0064:** two Live checks owned by area 1, `docs-external-links` (nightly) and `docs-deploy-smoke` (after each production deploy).
- **Adds to ADR 0062:** `@goodfetch/test-support` also gives Playwright fixtures and the in-memory adapter of the protocol; the Console's layout is the table in `TESTING.md`.
- **Adds to ADR 0063:** the `console` surface's Journey tests live in `console/journeys/`.
- **Changes `STYLE.md`:** its TypeScript section says the Console and the docs site are tested in browsers on one Node line.
- **ADR 0037's area table** lists this ADR in the rows of area 1 (the docs site) and area 12 (the Console).
- **Area 12's spec must confirm**, since the Console's stack is its own:
  1. React 19, Base UI through shadcn (`base-mira`) and Vite, as in the prototype; Vitest, Storybook's `react-vite` framework and `vitest-browser-react` depend on Vite and React;
  2. the Client takes its transport and body fetcher as values, and whether it is the generated TypeScript SDK's client running in the browser;
  3. the supported browsers (assumed: current Chrome, Edge, Firefox and Safari) and phones;
  4. the single-user sign-in link the fixture follows, and the OIDC provider used in tests;
  5. the build order: the Console bundle before the binary that embeds it;
  6. the move from the prototype's ESLint, Prettier and TypeScript 6 to oxlint, oxfmt and TypeScript 7, with Vitest's open TypeScript 7 issue for test files ([vitest#11144](https://github.com/vitest-dev/vitest/issues/11144));
  7. the i18n library, if any, under the CSP (with [Accessibility, localization and visual regression](https://github.com/Jbmanllr/automan/issues/283));
  8. the stub gateway, with area 14 confirming the gateway contract it stands for.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)): its docs-site section gains the strict build, the offline lychee run, the docs Playwright pass and the two Live checks; its pnpm workspace pins one Playwright version.
- **The "Test infrastructure" spec** builds the in-memory adapter of the protocol, the Playwright fixtures (Daemon, sign-in, the failure listener), the Playwright reporter that writes area 2's result document for Journeys and the Console conformance suite (which [Conformance result documents from the TypeScript and Python test runners](https://github.com/Jbmanllr/automan/issues/170) does not cover), and the local static server for the docs.
