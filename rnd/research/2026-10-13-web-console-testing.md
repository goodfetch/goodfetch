# Testing a web console and a docs site

Research for [#265 Testing a web console and a docs site](https://github.com/Jbmanllr/automan/issues/265), part of the map [Goodfetch V1: fully decided test strategy](https://github.com/Jbmanllr/automan/issues/262). Researched on 2026-10-13. Versions are the npm `latest` tags on that day (npm registry), unless stated otherwise.

## Question

What is the current standard way to test a React web interface like the Console (components, integration, end to end, accessibility, visual regression, localization), and its docs site? What do comparable developer consoles run?

Context from the ticket and the ADRs: no ADR fixes the Console's stack. The [V1 Console prototype](https://github.com/Jbmanllr/automan/issues/80) used React, shadcn on Base UI (Mira style) and Vite. The Console is embedded in the `goodfetch` binary and talks to the engine over a same-origin WebSocket, under a strict Content Security Policy with Trusted Types (ADR 0031). ADR 0031 also says "One Console conformance suite runs against a Daemon and a stub gateway". The docs site is Docusaurus. Its examples already run through Twoslash, pytest-examples and scripted CLI and MCP samples, and lychee checks its links (ADR 0030). This note does not reopen those.

## Summary: the facts a decision rests on

- **Component tests.** The current tools run component tests in a real browser:
  - Vitest Browser Mode has been stable since Vitest 4 (October 2025). Vitest 5.0 shipped on 2026-09-03.
  - Storybook's Vitest addon runs stories as tests in that mode. Storybook 11 (in alpha) ends support for the older Jest-based test runner.
  - Base UI, the library the Console prototype uses, runs its own unit tests in Chromium through Vitest Browser Mode by default.
  - jsdom does no layout. Supabase Studio has to polyfill `PointerEvent`, `ResizeObserver` and `DOMRect` before Radix menus work in jsdom.
- **Playwright component testing changed.** Playwright 1.62 (July 2026) replaced its experimental component-testing packages with a framework-agnostic "story and gallery" model built into `@playwright/test`. The old `@playwright/experimental-ct-react` stopped at 1.62.1.
- **End to end.** Playwright Test 1.64.0 (2026-10-07) is the standard. It has fixtures, tags, sharding with blob reports, traces, `page.clock`, `routeWebSocket` (since 1.48), virtual WebAuthn passkeys (1.61), test locks (1.63), `--shuffle` (1.64) and `failOnFlakyTests`.
  - Every comparable console runs its end-to-end suite against a real backend: Grafana, Supabase Studio, Prefect, PostHog, Temporal UI, and Sentry (Selenium).
  - Temporal UI also has a second Playwright tier that runs against mocked APIs.
  - MSW 3.0 (2026-09-28) mocks WebSockets by replacing the global `WebSocket` class.
- **Accessibility.** axe-core 4.14 is the common engine, through `@axe-core/playwright` 4.13 and Storybook's a11y addon.
  - axe's only WCAG 2.2 rule (`target-size`) is off by default.
  - Deque reports that axe fully covers 57% of issues by volume (2021, about 300,000 issues). GOV.UK found that axe detected 29% of 142 deliberate barriers (2017–2018).
  - Guidepup automates VoiceOver and NVDA only (no Linux or Orca). Base UI runs it with NVDA on a `windows-2025` runner.
  - On its text, the European Accessibility Act covers listed consumer products and services. A free developer tool is not among them.
- **Visual regression.**
  - Rendering differs across OS, CPU architecture and font stacks. Playwright maintainers say arm64 and amd64 Docker images are expected to produce different screenshots.
  - Self-hosted options: Playwright `toHaveScreenshot` and Vitest `toMatchScreenshot`, both free.
  - Hosted options:
    - **Argos:** MIT. Free Hobby plan of 5,000 screenshots. Its open-source sponsorship requires "not for commercial use".
    - **Chromatic:** free plan of 5,000 snapshots. Open-source support case by case.
    - **Percy:** free plan of 5,000 screenshots. Unlimited through BrowserStack's open-source programme.
  - Lost Pixel was archived in April 2026.
- **Localization.**
  - Of the consoles checked, Grafana (19 locales plus a pseudo-locale and a lint rule for untranslated strings) and Sentry (44 gettext locales) localize.
  - Temporal UI has i18n plumbing with English only.
  - Supabase Studio, PostHog, Dagster and Prefect's React UI have no i18n library. PostHog's i18n request was closed "not planned".
  - Apify, Browserbase and Firecrawl document no language setting for their web interfaces.
- **Docs site.**
  - Docusaurus 3.10.2 fails the build on broken links by default. It only warns on broken anchors, broken Markdown links and duplicate routes.
  - Docusaurus itself runs Lighthouse CI and Argos screenshots of every sitemap page. Supabase runs axe on its docs through Playwright, tagged `@a11y`.
- **Retries are common upstream.** Comparable suites retry on CI: Supabase Studio 5, Temporal UI 3, Grafana, Prefect and PostHog 1. That conflicts with the owner's "no retries" rule, which the map handles in [#280](https://github.com/Jbmanllr/automan/issues/280).

---

## 1. Components and integration

### Facts

**Vitest Browser Mode**
- **Stable since Vitest 4.** Vitest 4.0 (2025-10-22) removed the `experimental` tag from Browser Mode ([Vitest 4 blog](https://vitest.dev/blog/vitest-4), section "Browser Mode is Stable"). Vitest 4 also added `toMatchScreenshot` visual regression and Playwright traces (same post).
- **Providers.** Browser Mode needs a provider: `preview`, `playwright` or `webdriverio`. The docs recommend Playwright "because it supports parallel execution", and warn that the default `preview` provider "relies on simulating events instead of using Chrome DevTools Protocol" ([Browser Mode guide](https://vitest.dev/guide/browser/)).
- **Use Vitest's `userEvent`.** The same guide does not recommend using `@testing-library/user-event` directly, because "it simulates events instead of actually triggering them". It points to `userEvent` from `vitest/browser`, which goes through CDP or WebDriver.
- **Vitest 5.0 (2026-09-03).** It adds a provider-independent Trace View for Browser Mode. When a locator fails, Vitest now prints the ARIA tree ([Vitest 5 blog](https://vitest.dev/blog/vitest-5)).
- **Speed, from the Vitest 5 benchmark** (Apple M4, median of 3, same post):

  | Reference app | Environment | Vitest 4.1 | Vitest 5.0 |
  | --- | --- | --- | --- |
  | react-spa | jsdom (`vmThreads` pool) | 1.25 s | 1.07 s |
  | react-spa | Browser Mode, Chromium | 2.40 s | 2.01 s |

  The pools differ, so this is indicative only.
- **Limitations** ([Browser Mode guide](https://vitest.dev/guide/browser/), "Limitations"):
  - Blocking dialogs (`alert`, `confirm`) are mocked.
  - `vi.spyOn` on an imported module namespace throws. `vi.mock(path, { spy: true })` is the workaround.
- **Not a full replacement for end-to-end tests.** The [Why Browser Mode](https://vitest.dev/guide/browser/why) page says it "does not completely replace standalone end-to-end test runners", and lists "Longer Initialization" as a drawback.
- **ARIA snapshots** are available in Browser Mode from Vitest 4.1.4, flagged experimental upstream ([ARIA Snapshots guide](https://vitest.dev/guide/browser/aria-snapshots)).
- **Versions:** `vitest` 5.0.4, `@vitest/browser-playwright` 5.0.4, `vitest-browser-react` 2.3.0.

**jsdom and happy-dom**
- **jsdom** (30.1.2) "still does not do any layout or rendering". It has no navigation, and it returns zeros for layout properties such as `getBoundingClientRects()` ([jsdom README](https://github.com/jsdom/jsdom#unimplemented-parts-of-the-web-platform)).
- **happy-dom** (21.0.1) describes itself as "a JavaScript implementation of a web browser without its graphical user interface" ([README](https://github.com/capricorn86/happy-dom)).
- **The jsdom workarounds Supabase Studio needs** ([`apps/studio/tests/setup/radix.js`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/apps/studio/tests/setup/radix.js)):
  - It implements a custom `PointerEvent` and stubs `ResizeObserver`, `DOMRect` and `scrollIntoView`, because jsdom lacks them and Radix dropdowns need them.
  - It also mocks `matchMedia` and adds `jsdom-testing-mocks`.
  - Its Vitest config sets `retry: IS_CI ? 2 : 0` ([`vitest.config.ts`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/apps/studio/vitest.config.ts)).
- **Base UI runs real browsers by default.** Base UI (`mui/base-ui` at `81686db`) makes Chromium its default test target: `"test": "pnpm test:chromium"`. It also has `test:jsdom`, `test:firefox` and `test:webkit` scripts, all on Vitest 5.0.2 with `@vitest/browser-playwright` 5.0.2 ([package.json](https://github.com/mui/base-ui/blob/81686db2e4ed3581941b7f8aa5feddac6af76152/package.json)).

**Testing Library**
- `@testing-library/react` 16.3.3, `@testing-library/dom` 10.4.2, `@testing-library/user-event` 14.6.8.
- Its query priority puts `getByRole` first: "This should be your top preference for just about everything" ([Queries: Priority](https://testing-library.com/docs/queries/about#priority)).

**Storybook**
- **Versions:** Storybook 10.6.2 (10.0.0 was released 2025-10-28). 11.0.0 is in alpha (`alpha.6`).
- **The Vitest addon** (`@storybook/addon-vitest` 10.6.2) turns stories into Vitest tests through portable stories. It runs them in Browser Mode on Playwright's Chromium, and it "does not need to run Storybook to test your stories" ([Vitest addon docs](https://storybook.js.org/docs/writing-tests/integrations/vitest-addon)).
  - Requirements: a Vite-based Storybook framework, Vitest 3.0 or later, and MSW 2.0 or later if MSW is installed.
  - It runs interaction tests (play functions), accessibility checks and coverage. Visual tests go through Chromatic.
- **The test runner loses support.** The Jest- and Playwright-based `@storybook/test-runner` (0.26.0) loses official support in Storybook 11: it "no longer receives fixes or compatibility updates and prints a warning on every run" ([MIGRATION.md, "Test runner support ended"](https://github.com/storybookjs/storybook/blob/next/MIGRATION.md#test-runner-support-ended)). Grafana, PostHog and Temporal UI still use it (section 7).
- **Weekly npm downloads** (2026-10-05 to 2026-10-11, [npm downloads API](https://api.npmjs.org/downloads/point/2026-10-05:2026-10-11/@storybook/addon-vitest)):
  - `@storybook/addon-vitest`: about 6.0 M.
  - `@storybook/test-runner`: about 2.4 M.

**Playwright component testing**
- **The new model.** Playwright 1.62 (2026-07-24) moved component testing to "a stories and galleries model". A built-in `mount` fixture of `@playwright/test` navigates to a gallery page that your own dev server serves ([release notes 1.62](https://playwright.dev/docs/release-notes#version-162)).
- **The old packages stopped.** Playwright 1.63 announced that `@playwright/experimental-ct-react`, `-react17` and `-vue` "will no longer be updated" ([release notes 1.63](https://playwright.dev/docs/release-notes#version-163)). npm shows the last version of `@playwright/experimental-ct-react` as 1.62.1 (2026-07-30).
- **Why it changed.** The component-testing guide on Playwright's `main` branch explains the old design "kept the packages experimental forever". The new model is framework-agnostic: "The only framework-specific piece is the gallery page — a small module that you own." The gallery is generated through the `playwright-component-testing` agent skill or written by hand ([test-components doc source](https://github.com/microsoft/playwright/blob/main/docs/src/test-components-js.md)).

**Mocking the network in component tests**
- **MSW 3.0.0 (2026-09-28)** is ESM-only and needs Node 22 or later. It renames the WebSocket `connection` event to `websocket:connection`, and it "no longer patches `setTimeout` to circumvent fake timers" ([release v3.0.0](https://github.com/mswjs/msw/releases/tag/v3.0.0)).
- **MSW's WebSocket interception** works on "WebSocket connections created using the global WHATWG `WebSocket` class", not through the Service Worker. Intercepted connections are not opened unless `server.connect()` is called ([`@mswjs/interceptors` README, WebSocketInterceptor](https://github.com/mswjs/interceptors#websocketinterceptor); [MSW WebSocket docs](https://mswjs.io/docs/websocket)).
- `msw-storybook-addon` 3.0.3 wires MSW into Storybook.

### Comparison

| Option | Runs in | Real layout and events | Notes |
| --- | --- | --- | --- |
| Vitest + jsdom (30.1) | Node | No | Fastest. Radix-style primitives need polyfills (Supabase Studio). Most comparable consoles use this or Jest. |
| Vitest + happy-dom (21.0) | Node | No | PostHog uses it next to jsdom. |
| Vitest Browser Mode + Playwright provider (5.0) | Chromium, Firefox or WebKit | Yes (CDP) | Stable since Vitest 4. Base UI's default. About 2× slower than jsdom on Vitest's react-spa benchmark. |
| Storybook Vitest addon (10.6) | Vitest Browser Mode | Yes | Stories become tests, plus a11y checks. Requires Vite. |
| Storybook test runner (0.26) | Jest + Playwright against a running Storybook | Yes | Support ends in Storybook 11. |
| Playwright component testing (1.62+ story/gallery model) | Any browser through your dev server | Yes | Not React-specific. The old `experimental-ct-*` packages are discontinued. |

**What depends on React:**
- `@testing-library/react`, `vitest-browser-react` and `@storybook/react-vite` are React bindings.
- Playwright's gallery page is framework-specific code that the project owns.
- Vitest, MSW, Playwright, axe and the visual-regression services are framework-agnostic.

### Fit for Goodfetch

- **Base UI expects real browsers.** The Console's component library tests itself in real browsers. In jsdom, the same primitives need the polyfills Supabase writes by hand. That favours Vitest Browser Mode with the Playwright provider for component tests.
- **Shared tooling.** That choice keeps Vitest, which STYLE.md already mandates for the TypeScript workspace, and shares Playwright with the end-to-end tier.
- **Storybook is optional.** If the Console keeps Storybook for its screens, the Vitest addon is the supported path, since the test runner is ending. If it does not, Playwright's gallery model gives component tests without Storybook.
- **The engine boundary.** Mocking the Console's WebSocket to the engine at component level works with MSW's `ws` API, which replaces `WebSocket`. Whether that mock should be MSW or a fake Client generated from the Operation catalog is a design question for the Test infrastructure spec.

---

## 2. End to end

### Facts

**Playwright Test 1.64.0** (2026-10-07; [release notes](https://playwright.dev/docs/release-notes))
- **Test organisation:**
  - Fixtures, including worker-scoped ones ([docs](https://playwright.dev/docs/test-fixtures)).
  - Tags and annotations, filtered with `--grep @tag` ([docs](https://playwright.dev/docs/test-annotations)).
  - Projects. In 1.64 a project can be kept out of the default run with `default: false`.
- **Sharding:** `--shard=x/y` with the `blob` reporter and `merge-reports` ([docs](https://playwright.dev/docs/test-sharding)).
- **Traces:** retained on failure. Since 1.63, traces can also record aria and screen snapshots on every action. Since 1.61, "HAR and trace recordings now include WebSocket requests" (release notes 1.61, 1.63).
- **Clock:** `page.clock` (since 1.45) overrides `Date`, `Temporal.Now`, the timers, `requestAnimationFrame`, `performance` and `Event.timeStamp`. Its docs recommend `setFixedTime` first ([Clock](https://playwright.dev/docs/clock)).
- **WebSocket mocking:** `page.routeWebSocket` and `context.routeWebSocket` (since 1.48) mock a WebSocket. "By default, the routed WebSocket will not connect to the server". `connectToServer()` proxies and edits frames instead ([WebSocketRoute](https://playwright.dev/docs/api/class-websocketroute)).
- **Features that support deterministic, independent tests:**
  - `failOnFlakyTests` (config option since 1.52) fails the run if any test passed only on retry.
  - `--shuffle` (1.64) "helps to find tests that accidentally depend on each other".
  - Test `lock`s (1.63) stop tests that share a resource from running concurrently.
  - `retryStrategy: 'isolated'` (1.62).
- **Authentication:**
  - The virtual WebAuthn authenticator (`context.credentials`, 1.61) registers and answers passkeys without hardware.
  - `storageState` can save those credentials (1.62).
- **Accessibility-tree checks:** `toMatchAriaSnapshot` (1.49) and `Locator.within()` (1.64).

**Weekly npm downloads** (2026-10-05 to 2026-10-11, [npm downloads API](https://api.npmjs.org/downloads/point/2026-10-05:2026-10-11/@playwright/test)): `@playwright/test` about 76 M, `msw` about 23 M.

**What comparable consoles run end to end**

| Project | Backend under test | Shards | CI retries | Source |
| --- | --- | --- | --- | --- |
| Grafana | Built Grafana server with provisioned data and test plugins | 8 | 1 | [`pr-e2e-tests.yml`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/.github/workflows/pr-e2e-tests.yml), [`playwright.config.ts`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/playwright.config.ts) |
| Supabase Studio | Local Supabase stack started by the Supabase CLI | 2 (weighted 62:38) | 5 | [`studio-e2e-test.yml`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/.github/workflows/studio-e2e-test.yml), [`e2e/studio/playwright.config.ts`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/e2e/studio/playwright.config.ts) |
| Temporal UI | Two tiers: "integration" with mocked APIs (`page.route`) against `vite preview`, and "e2e" against a real Temporal dev server, a worker and a codec server | none | 3 (1 locally) | [`playwright.config.ts`](https://github.com/temporalio/ui/blob/5bf3c5333a32d315d09bba4da399f9f72b2ab025/playwright.config.ts), [`tests/global-setup.ts`](https://github.com/temporalio/ui/blob/5bf3c5333a32d315d09bba4da399f9f72b2ab025/tests/global-setup.ts) |
| Prefect (React `ui-v2`) | `prefect server` with PostgreSQL, plus a second job with authentication on | 3 | 1 | [`ui-v2-e2e-tests.yml`](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/.github/workflows/ui-v2-e2e-tests.yml), [`ui-v2/playwright.config.ts`](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/ui-v2/playwright.config.ts) |
| PostHog | Docker Compose stack | affected specs only | 1 in CI (config default 3) | [`ci-e2e-playwright.yml`](https://github.com/PostHog/posthog/blob/c4545bc76c04bb67ef08a3b4188b6dac41b93b5b/.github/workflows/ci-e2e-playwright.yml), [`playwright/playwright.config.ts`](https://github.com/PostHog/posthog/blob/c4545bc76c04bb67ef08a3b4188b6dac41b93b5b/playwright/playwright.config.ts) |
| Sentry | Full stack. Acceptance tests in pytest with Selenium, 5 or 10 matrix instances | 5–10 | not checked | [`acceptance.yml`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/.github/workflows/acceptance.yml), [`selenium.py`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/src/sentry/testutils/pytest/selenium.py) |
| Dagster | No browser end-to-end suite found in `js_modules` | – | – | [Buildkite step](https://github.com/dagster-io/dagster/blob/c97898c3e4a8a3cc5e32fcc2ca97aea810342ed9/.buildkite/dagster-buildkite/dagster_buildkite/steps/dagster_ui.py) |

**Grafana under a strict CSP.** Grafana has an opt-in script, `e2e:playwright:strict-csp`. It starts the server with a Content Security Policy that drops `'unsafe-eval'` and enforces `require-trusted-types-for 'script'`, "so a blocked new Function/eval throws and fails the test" ([`e2e-playwright/start-server`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/e2e-playwright/start-server)). Its pull-request workflow only documents in a comment how to turn it on.

**User complaints:**
- Flakiness is the recurring cost of end-to-end suites that run against a real backend. The retry counts in the table show how much flakiness each project tolerates.
- PostHog runs `test-quarantine.yml` and `weekly-flaky-report.yml` workflows ([workflows](https://github.com/PostHog/posthog/tree/c4545bc76c04bb67ef08a3b4188b6dac41b93b5b/.github/workflows)).

### Fit for Goodfetch

- **Test the real binary.** The comparable consoles all test the real server. For Goodfetch, the real server is the `goodfetch` binary serving the embedded Console, which matches ADR 0031's "Console conformance suite runs against a Daemon and a stub gateway".
- **What only the real binary can show:** CSP headers, Trusted Types, the `HttpOnly` cookie, COOP and CORP, the separate content origin, and the brotli-embedded assets.
- **Useful built-ins for the Console's own features:**
  - `routeWebSocket`: dropped connections, which ADR 0031's read-only banner and reconnection must handle.
  - `page.clock`: the 30-day idle expiry and idle stop.
  - The virtual WebAuthn authenticator: passkeys and step-up.
  - Test locks: Daemon-wide settings.
- **Fit with the owner's requirements:**
  - `failOnFlakyTests` and `--shuffle` support "no flaky tests and no retries", which none of the comparable projects meet. They all retry.
  - Tags and projects support the quick, pre-merge and release tiers ([#276](https://github.com/Jbmanllr/automan/issues/276)).
  - Grafana's strict-CSP mode shows that running end-to-end tests under the production CSP is feasible. The Console's CSP is strict by default, so its suite would always run that way.

---

## 3. Accessibility

### Facts

**Engines and integrations**
- **axe-core 4.14.0** (2026-10-05, MPL-2.0).
  - Its WCAG 2.2 section holds one rule, `target-size` (tags `wcag22aa`, `EN-301-549v4`). These rules "are disabled by default, until WCAG 2.2 is more widely adopted and required" ([rule descriptions](https://github.com/dequelabs/axe-core/blob/develop/doc/rule-descriptions.md#wcag-22-level-a--aa-rules)).
  - The API tags include `EN-301-549` and `RGAA` ([API.md](https://github.com/dequelabs/axe-core/blob/develop/doc/API.md#axe-core-tags)).
- **`@axe-core/playwright` 4.13.0** (2026-08-11, MPL-2.0; about 14.1 M weekly downloads).
  - Playwright's guide uses `AxeBuilder.withTags(['wcag2a','wcag2aa','wcag21a','wcag21aa'])` and recommends a shared fixture.
  - It warns that `exclude()` disables *all* rules for the element and its descendants.
  - It says "automated testing cannot detect all types of WCAG violations" ([Accessibility testing](https://playwright.dev/docs/accessibility-testing)).
- **Storybook's a11y addon** (10.6.2) runs axe on each story. Run through the Vitest addon, `parameters.a11y.test` takes `'off'`, `'todo'` (warn) or `'error'` (fail in CI).
  - By default it runs the WCAG 2.0 and 2.1 A/AA rules plus best practices, and disables the `region` rule.
  - WCAG 2.2 has to be added through `runOnly` ([Accessibility tests](https://storybook.js.org/docs/writing-tests/accessibility-testing)).
- **Lint:** `eslint-plugin-jsx-a11y` (6.10.2) is built into oxlint as the `jsx-a11y` plugin ([oxlint plugins](https://oxc.rs/docs/guide/usage/linter/plugins.html)).

**How much automated checks catch.** The two studies measure different things:
- **Deque (2021-03-10).** Deque analysed over 2,000 audits, 13,000+ pages and nearly 300,000 issues. It found that "on average, 57 percent of accessibility issues were completely covered" by automated testing with its axe tools.
  - The share is by **volume of issues**, not by WCAG success criteria. Frequent issue types raise it.
  - The study is vendor-produced. Its full method is in a separate report ([Deque blog](https://www.deque.com/blog/automated-testing-study-identifies-57-percent-of-digital-accessibility-issues/)).
- **GOV.UK (2017-02-24, updated 2018-04-13).** GOV.UK built a page with 142–143 deliberate barriers.
  - All tools together found about 71% of them; 29% were missed by every tool.
  - aXe alone found 29%, or 30% counting items flagged for review ([blog](https://accessibility.blog.gov.uk/2017/02/24/what-we-found-when-we-tested-tools-on-the-worlds-least-accessible-webpage/), [audit results](https://alphagov.github.io/accessibility-tool-audit/)).
  - Conclusion: tools "cannot be relied on, by themselves".

**Keyboard and focus**
- Playwright: `keyboard.press('Tab')` with `expect(locator).toBeFocused()`, and `toMatchAriaSnapshot` for structure ([LocatorAssertions](https://playwright.dev/docs/api/class-locatorassertions)).
- Vitest Browser Mode: `userEvent.tab()` and `userEvent.keyboard()` through CDP ([interactivity API](https://vitest.dev/api/browser/interactivity)).
- Since 1.63, Playwright Test has `reducedMotion`, `forcedColors` and `contrast` options (release notes 1.63).

**Screen-reader automation (Guidepup)**
- **Versions:** `@guidepup/guidepup` 0.35.0 (2026-10-01, MIT); about 56 k weekly downloads.
- **Coverage:** it drives "VoiceOver on MacOS and NVDA on Windows with a single API" ([README](https://github.com/guidepup/guidepup)).
- **No Linux.** Orca support is an open request ([guidepup#107](https://github.com/guidepup/guidepup/issues/107)). JAWS is requested too ([#125](https://github.com/guidepup/guidepup/issues/125)).
- **Setup.** The GitHub setup action "is no longer supported". Use the `@guidepup/setup` CLI instead ([setup-action](https://github.com/guidepup/setup-action)).
- **Base UI runs it.** Base UI runs Guidepup with `@guidepup/playwright` 0.19.1 in a "Screen Reader (Windows / NVDA)" job on `windows-2025`, with two specs ([`ci.yml`](https://github.com/mui/base-ui/blob/81686db2e4ed3581941b7f8aa5feddac6af76152/.github/workflows/ci.yml), [`test/screen-reader`](https://github.com/mui/base-ui/tree/81686db2e4ed3581941b7f8aa5feddac6af76152/test/screen-reader)).
- **A browser-only alternative.** `@guidepup/virtual-screen-reader` (0.33.0) simulates a screen reader inside the page.

**WCAG 2.2**
- WCAG 2.2 became a W3C Recommendation on 2023-10-05 ([What's new](https://www.w3.org/WAI/standards-guidelines/wcag/new-in-22/)). It removed 4.1.1 Parsing.
- Its new A/AA criteria:
  - 2.4.11 Focus Not Obscured (Minimum)
  - 2.5.7 Dragging Movements
  - 2.5.8 Target Size (Minimum)
  - 3.2.6 Consistent Help
  - 3.3.7 Redundant Entry
  - 3.3.8 Accessible Authentication (Minimum)
- **EN 301 549 v4.1.1**, the update meant to become the harmonised standard under the Accessibility Act, was published on 2026-09-02 and adopts WCAG 2.2 AA. Deque, writing on 2026-09-08, says it carries no presumption of conformity until it is cited in the Official Journal, expected around 2026-11-30 ([Deque](https://www.deque.com/blog/en-301-549-v4-1-1-is-final-what-changed-what-it-means-and-what-you-should-do/)).

**Does the European Accessibility Act apply?** Directive (EU) 2019/882 ([EUR-Lex](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32019L0882)):
- **Scope (Article 2).** It covers listed products placed on the market after 28 June 2025. It also covers listed services "provided to consumers": electronic communications, access to audiovisual media, passenger transport elements, consumer banking, e-books and dedicated software, and e-commerce services.
- **E-commerce definition (Article 3(30)).** "e-commerce services" means services provided at a distance "at the individual request of a consumer with a view to concluding a consumer contract".
- **Microenterprises.** Microenterprises providing services (fewer than 10 persons, and €2 M or less in turnover or balance sheet) are exempt from the service requirements (Article 3(23) and Article 4).
- **No developer-tool category.** Nothing in the list names developer tools or business-to-business software.

**User complaints and gaps**
- Docusaurus has 25 issues labelled `domain: a11y`. One example: accessibility checkers object to where the `h1` sits ([#11261](https://github.com/facebook/docusaurus/issues/11261)).
- Guidepup's open issues include NVDA start failures on Windows paths with spaces ([#164](https://github.com/guidepup/guidepup/issues/164)).

### Fit for Goodfetch

- **WCAG 2.2 needs explicit configuration.** axe does not check WCAG 2.2 out of the box. A WCAG 2.2 AA target needs the `wcag22aa` tag enabled in every axe entry point: the Playwright fixture, Storybook's `runOnly`, and the docs scan. Even then, only `target-size` is automated.
- **Some criteria need a real layout.** Focus Not Obscured and Target Size depend on layout, so they need a real browser (Vitest Browser Mode or Playwright), not jsdom.
- **Screen readers.** Guidepup needs Windows or macOS runners. Comparable projects run a few targeted specs (Base UI has two), not whole suites.
- **Manual testing remains.** The two studies put automated coverage between about 30% of deliberate barriers and 57% of issues by volume. Following the owner's rule, what automation cannot cover goes to a listed manual test.
- **On the directive's text,** the Console and docs of a free developer tool are not a listed consumer product or service. Goodfetch Cloud's sign-up and billing would need their own reading if consumers can buy it. This is not legal advice.

---

## 4. Visual regression

### Facts

**Self-hosted assertions**
- **Playwright `toHaveScreenshot`.**
  - "Browser rendering can vary based on the host OS, version, settings, hardware, power source … headless mode". Snapshot names carry the browser and platform, for example `chromium-darwin` ([Visual comparisons](https://playwright.dev/docs/test-snapshots)).
  - It waits "until two consecutive page screenshots yield the same result" ([PageAssertions](https://playwright.dev/docs/api/class-pageassertions)).
  - Defaults: `animations: "disabled"` (finite animations fast-forwarded, infinite ones cancelled), `caret: "hide"`, `threshold` 0.2 ([params](https://github.com/microsoft/playwright/blob/main/docs/src/api/params.md)).
  - `stylePath` hides dynamic regions.
  - Since 1.62 and 1.64, snapshots can be stored as WebP.
- **Vitest `toMatchScreenshot`** (Browser Mode).
  - Vitest strongly recommends Docker containers, CI-only workflows or cloud services, because rendering depends on "GPU, drivers … Operating System, Font rendering pipelines, Browser … headless or headed" ([Visual regression testing](https://vitest.dev/guide/browser/visual-regression-testing)).
  - It names Azure App Testing and Chromatic as cloud options.
  - It uses `pixelmatch` by default and lets other comparators plug in.
- **Docker.** Playwright's image is `mcr.microsoft.com/playwright:v<version>-noble`.
  - The docs recommend `--init` and `--ipc=host`.
  - `run-server` lets a macOS or Windows host drive browsers inside the container ([Docker](https://playwright.dev/docs/docker)).
- **CPU architecture matters.** Asked about matching screenshots across arm64 and amd64 Docker, a Playwright maintainer answered: "It is expected that arm docker image vs intel docker image produce different screenshots" ([playwright#13873](https://github.com/microsoft/playwright/issues/13873)). The reporter, elastic-charts, saw "about 10% of our screenshots" fail across architectures, with diffs from 0.01% to over 5%.

**Hosted services** (pricing pages read on 2026-10-13)

| Service | Licence of the code | Free tier | Paid entry | Open-source terms | Status |
| --- | --- | --- | --- | --- | --- |
| Argos | Whole platform MIT ([repo](https://github.com/argos-ci/argos)) | Hobby: 5,000 screenshots/month, "for personal projects" | Pro: $100/month, 35,000 screenshots, then $0.004 (Storybook $0.0015) | Sponsorship case by case. Requires "open source and not for commercial use", the Argos README banner and a dofollow link ([docs](https://argos-ci.com/docs/learn/billing-and-subscription/open-source)) | Active. Self-hosting "not officially supported or documented" (AWS, PostgreSQL, RabbitMQ, Redis, S3, DynamoDB) |
| Chromatic | CLI MIT. Service proprietary | 5,000 snapshots/month, Chrome only | Starter: $179/month, 35,000 snapshots, 4 browsers | "Building in the open? We'd love to support your project", eligibility case by case ([pricing](https://www.chromatic.com/pricing)) | Active. Storybook, Playwright (`@chromatic-com/playwright` 1.0.0), Vitest |
| Percy (BrowserStack) | CLI MIT. Service proprietary | 5,000 screenshots/month, unlimited users ([docs](https://www.browserstack.com/docs/percy/overview/plans-and-billing)) | Desktop: $199/month (billed annually), 10,000 screenshots | BrowserStack's open-source programme gives "Unlimited testing … Live, Automate and Percy" ([open source](https://www.browserstack.com/open-source)) | Active |
| Lost Pixel | MIT | – | – | – | **Archived 2026-04-22.** "Lost Pixel is joining Figma. We are sunsetting the product" ([repo](https://github.com/lost-pixel/lost-pixel)). Last release v3.22.0, 2024-11-14 |

- **Determinism aids:**
  - Argos's `argosScreenshot` "waits for fonts, images, and `aria-busy` loaders before capturing". Its recommended launch options disable subpixel text and font hinting. It uploads only from CI ([Argos docs corpus](https://argos-ci.com/docs/llms-full.txt)).
  - Argos's diff engine is the open-source `odiff`.
- **What comparable projects do** (section 7):
  - **PostHog:** Storybook screenshots with `jest-image-snapshot` inside a pinned image (`ghcr.io/posthog/playwright:v1.60.0`), on Chromium and WebKit shards, gated by its own "Visual Review" service.
  - **Temporal UI:** Chromatic with `onlyChanged` and `exitZeroOnChanges`.
  - **Sentry:** component snapshots captured with Playwright Chromium and uploaded from CI.
  - **Docusaurus and Base UI:** Argos.
  - **Grafana, Supabase and Prefect:** GitHub code search finds no `toHaveScreenshot` in their repositories.

### Fit for Goodfetch

- **Pin the environment.** Screenshot tests are only deterministic in one pinned environment: a pinned Playwright image and one CPU architecture. Contributors on Apple Silicon would generate reference screenshots through `run-server` in the pinned amd64 image, or let CI generate them.
- **Paid tools need approval.** Map #262 says the owner approves paid test resources ticket by ticket.
- **Hosted services and Goodfetch's licence.** Goodfetch has a commercial Goodfetch Cloud, which likely rules out Argos's "not for commercial use" sponsorship. Chromatic's support is case by case. Percy is free only through BrowserStack's open-source programme or its 5,000-screenshot tier.
- **The free self-hosted path.** Playwright `toHaveScreenshot` or Vitest `toMatchScreenshot` in a pinned image is free. It needs a reviewing workflow for reference-screenshot updates. ("Baseline" is a Goodfetch term for a previous run, so this note says "reference screenshot".)
- **Pseudo-states.** PostHog's `storybook-addon-pseudo-states` (10.6.2) covers hover and focus states in screenshots.

---

## 5. Localization

### Facts

**Which consoles localize** (checked 2026-10-13)

| Console | Localized? | Mechanism and evidence |
| --- | --- | --- |
| Grafana | Yes | 19 locales under [`public/locales`](https://github.com/grafana/grafana/tree/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/public/locales), none of them right-to-left. i18next ^26.3.6 and react-i18next ^17.0.9 through `@grafana/i18n`, with a "Pseudo" locale via `i18next-pseudo` ([`i18n.tsx`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/packages/grafana-i18n/src/i18n.tsx)). An ESLint rule `@grafana/i18n/no-untranslated-strings` ([README](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/packages/grafana-i18n/src/eslint/README.md)). Crowdin upload and download workflows. A `verify-i18n` CI job that re-extracts strings and fails on uncommitted changes. |
| Sentry | Yes | Jed and gettext `.po` catalogs, with 44 `supported_locales` ([`catalogs.json`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/src/sentry/locale/catalogs.json)). The locale comes from `?lang=`, user options or the system language ([`initializeLocale.tsx`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/static/app/bootstrap/initializeLocale.tsx)). A `localeDebug` mode highlights translated strings ([`locale.tsx`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/static/app/locale.tsx)). |
| Temporal UI | Plumbing only | i18next ^22.4.15 with a `translate` helper. The only locale is `en` ([`src/lib/i18n/locales`](https://github.com/temporalio/ui/tree/5bf3c5333a32d315d09bba4da399f9f72b2ab025/src/lib/i18n/locales)). |
| Supabase Studio | No | No i18n library in [`apps/studio/package.json`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/apps/studio/package.json). The repository's `i18n/` folder only holds README translations. |
| PostHog | No | No i18n library. An "i18n" request ("Currently posthog is available in english only") was closed as `not_planned` in 2024 ([#13055](https://github.com/PostHog/posthog/issues/13055)). |
| Dagster | No | No i18n library. An "i18n Language Support" request has been open since 2024-05-20 ([#21965](https://github.com/dagster-io/dagster/issues/21965)). |
| Prefect (`ui-v2`) | No | No i18n library in [`ui-v2/package.json`](https://github.com/PrefectHQ/prefect/blob/302beb37eb79cd6ba37fcaa9fa002c4429b45702/ui-v2/package.json). |
| Apify, Browserbase, Firecrawl | Not documented | Their full docs corpora (`llms-full.txt`) describe no language setting for the web interface. Firecrawl's *docs* are localized: `/es/`, `/fr/`, `/ja/`, `/pt-BR/` and `/zh/` pages serve `lang="ja"` and so on. |

**User complaint.** Without localization, users fall back on browser translation, which breaks React apps. PostHog's "Browser translation breaks the site" ([#15658](https://github.com/PostHog/posthog/issues/15658)) points to [facebook/react#11538](https://github.com/facebook/react/issues/11538). Dagster's request cites Google Translate mangling its concepts.

**React i18n libraries**

| Library | Version (2026-10-13) | Licence | Weekly downloads | Message format | Notes |
| --- | --- | --- | --- | --- | --- |
| react-i18next / i18next | 17.0.16 / 26.4.4 | MIT | ≈17.9 M | i18next JSON. ICU through `i18next-icu` | Used by Grafana and Temporal |
| react-intl (FormatJS) | 12.1.4 | BSD-3-Clause | ≈3.3 M | ICU MessageFormat | The FormatJS CLI compiles pseudo-locales `xx-LS` (long), `xx-AC` (caps), `xx-HA`, `en-XA` (accented, bracketed) and `en-XB` (bidi) ([CLI docs](https://formatjs.github.io/docs/tooling/cli)) |
| Lingui | 6.9.1 | MIT | ≈1.3 M | ICU MessageFormat ([docs](https://lingui.dev/guides/message-format)) | Compile-time macros. "Catalogs are compiled ahead of time, so the runtime ships without a MessageFormat parser" ([README](https://github.com/lingui/js-lingui)). Built-in `pseudoLocale`, including a right-to-left pseudolocale ([guide](https://lingui.dev/guides/pseudolocalization)). `eslint-plugin-lingui` 0.16.0 has `no-unlocalized-strings` |
| Paraglide JS | 2.27.0 | MIT | ≈0.78 M | Compiled to typed ESM functions | "Tree-shakable translations". Claims up to 70% smaller i18n bundles ([README](https://github.com/opral/paraglide-js)) |

Downloads cover 2026-10-05 to 2026-10-11 ([npm downloads API](https://api.npmjs.org/downloads/point/2026-10-05:2026-10-11/react-i18next)).

**Detecting hard-coded strings**
- `eslint-plugin-i18next`'s `no-literal-string` (6.1.5, ISC), Lingui's `no-unlocalized-strings`, and Grafana's own rule.
- Oxlint runs ESLint plugins through `jsPlugins`, which "are currently in alpha" ([JS plugins](https://oxc.rs/docs/guide/usage/linter/js-plugins.html)).
- Pseudo-locales also reveal hard-coded and concatenated strings (Lingui guide).

**Right-to-left support in the Console's stack**
- shadcn/ui has had "first-class support for right-to-left (RTL) layouts" since its January 2026 changelog. The CLI rewrites physical classes to logical ones ([RTL docs](https://github.com/shadcn-ui/ui/blob/main/apps/v4/content/docs/rtl/index.mdx)).
- Base UI's `DirectionProvider` "Enables RTL behavior for Base UI components" ([docs source](https://github.com/mui/base-ui/tree/81686db2e4ed3581941b7f8aa5feddac6af76152/docs/src/app/(docs)/react/utils/direction-provider)).

### Fit for Goodfetch

- **Most comparable consoles ship English only.** Among those that localize, Grafana's pattern is the complete one to copy if the Console localizes: extraction verified in CI, a pseudo-locale, a lint rule for untranslated strings, and translation-platform sync.
- **CSP.** Lingui's compiled catalogs avoid a runtime parser. Whether each library avoids `eval` and `new Function` under the Console's Trusted Types policy should be confirmed by running the Console under its CSP (see Unverified).
- **What depends on React.** All four libraries have React bindings. Lingui and Paraglide also cover other frameworks.
- **A test is possible even with English only.** A pseudo-locale (accented and lengthened) and an RTL pseudo-locale, run through the same component and end-to-end suites, find hard-coded strings, truncation and mirroring bugs without any real translation.

---

## 6. Docs site

### Facts

**Docusaurus build checks.** Docusaurus is at 3.10.2 (2026-07-10), and 4.0.0 canaries have been published since 2026-09-14. Defaults in [3.10 `docusaurus.config.js` docs](https://github.com/facebook/docusaurus/blob/main/website/versioned_docs/version-3.10.2/api/docusaurus.config.js.mdx):

| Check | Default | Notes |
| --- | --- | --- |
| `onBrokenLinks` | `throw` | "only available for a production build" |
| `onBrokenAnchors` | warn | – |
| `markdown.hooks.onBrokenMarkdownLinks` | `warn` | Top-level `onBrokenMarkdownLinks` was deprecated in 3.9 and is removed in v4 |
| `markdown.hooks.onBrokenMarkdownImages` | `throw` | – |
| `markdown.hooks.onUnusedMarkdownDirectives` | `warn` | – |
| `onDuplicateRoutes` | warn | – |

**What Docusaurus runs on its own site**
- Lighthouse CI (`treosh/lighthouse-ci-action` 12.6.2) on pull requests ([`lighthouse-report.yml`](https://github.com/facebook/docusaurus/blob/main/.github/workflows/lighthouse-report.yml), [`lighthouserc.json`](https://github.com/facebook/docusaurus/blob/main/.github/workflows/lighthouserc.json)).
- Argos visual regression that "take[s] screenshots of all pages found in `sitemap.xml` with Playwright" ([`argos/README.md`](https://github.com/facebook/docusaurus/blob/main/argos/README.md)). Its spec skips a list of pages that are flaky because of widgets, hydration errors or dynamic content ([`screenshot.spec.ts`](https://github.com/facebook/docusaurus/blob/main/argos/tests/screenshot.spec.ts)).
- A cspell spelling check (`cspell-problem-matcher.json` in its workflows).

**How others check docs accessibility**
- **Supabase** runs Playwright with `@axe-core/playwright` ^4.12.1 on its docs, against Vercel preview deployments, through an `e2e:docs:a11y` script that greps for `@a11y` ([`e2e/docs/package.json`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/e2e/docs/package.json), [`docs-e2e.yml`](https://github.com/supabase/supabase/blob/6f36fe879bebb36bf6a95083af6205c524522933/.github/workflows/docs-e2e.yml)).
- **pa11y-ci** 5.0.0 (2026-10-05; LGPL-3.0-only) tests "a list of URLs or a sitemap" (`--sitemap`). Its README says passing it "does not guarantee that your site is accessible" ([README](https://github.com/pa11y/pa11y-ci)).
- **Lighthouse CI** (`@lhci/cli` 0.15.1, 2025-06-25): its accessibility category is a score, not a pass/fail gate per rule (see Unverified).
- Docusaurus's docs claim the theme pays "Attention to accessibility" ([introduction](https://github.com/facebook/docusaurus/blob/main/website/docs/introduction.mdx)). Its tracker holds 25 issues labelled `domain: a11y` (search on 2026-10-13).

**Other docs sites built on Docusaurus.** Apify's docs run Docusaurus 3.10.2 (its HTML `generator` meta tag).

### Fit for Goodfetch

- **Turn warnings into failures.** Raising `onBrokenAnchors`, `markdown.hooks.onBrokenMarkdownLinks` and `onDuplicateRoutes` to `throw` costs one config line each. It closes the internal-link gaps lychee does not see (anchors and routes generated at build time).
- **The docs need their own axe pass.** The Docusaurus theme, swizzled components and generated reference pages are not Console components. Following Supabase and Docusaurus, a sitemap-driven Playwright pass with `@axe-core/playwright` fits:
  - against the Vercel preview ADR 0030 already provides;
  - in light and dark themes, since Grafana runs its Storybook a11y checks under 8 themes.
- **Licence check.** ADR 0018's licence allowlist covers what ships in a release. Whether a dev-only tool under LGPL-3.0 (pa11y-ci) or MPL-2.0 (axe-core, `@axe-core/playwright`) needs an allowlist decision is for the deciding ticket. ADR 0018 already accepts MPL-2.0 for unmodified files.

---

## 7. Prior art: what comparable consoles run in CI

The commits read are the default-branch heads on 2026-10-13. Versions are the ranges declared in each `package.json` (a `catalog:` entry resolves in the workspace).

| Project (commit) | UI stack | Unit and component | Storybook | End to end | Accessibility | Visual | i18n | Lint |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Grafana** ([`4c526b8`](https://github.com/grafana/grafana/tree/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c)) | React 19.2.8, rspack | Jest 29.7.0, jsdom, Testing Library react 16.3.0, MSW 2.10.4, `jest-fail-on-console`. Vitest ^4.1.10 only for build scripts | 10.3.6 in `grafana-ui`, with test runner ^0.24.3 and `msw-storybook-addon` | Playwright 1.56.1 with `@grafana/plugin-e2e`. 8 shards, real server, 1 retry. Opt-in strict CSP | `@axe-core/playwright` ^4.11.1 smoke suite tagged `@a11y` ([`accessibility.spec.ts`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/e2e-playwright/smoke-tests-suite/accessibility.spec.ts)). Storybook a11y run in 8 themes, including deuteranopia and tritanopia ([`storybook-a11y.yml`](https://github.com/grafana/grafana/blob/4c526b8b9f1e7bdccd1ddc8fd480a0084a20104c/.github/workflows/storybook-a11y.yml)). `eslint-plugin-jsx-a11y` 6.10.2 | None found | Yes (section 5) | ESLint 9.32.0 |
| **Supabase Studio** ([`6f36fe8`](https://github.com/supabase/supabase/tree/6f36fe879bebb36bf6a95083af6205c524522933)) | React ^19.2.6, TanStack Start, Vite ^8 | Vitest ^5.0.0, jsdom, Testing Library react ^16.3.3, MSW ^2.3.0, `jsdom-testing-mocks`. 2 retries on CI | None in Studio | Playwright ^1.59.1 against a local Supabase stack. 2 shards, 5 retries | Docs only: `@axe-core/playwright` ^4.12.1 | None found | No | ESLint with a lint "ratchet", knip |
| **Temporal UI** ([`5bf3c53`](https://github.com/temporalio/ui/tree/5bf3c5333a32d315d09bba4da399f9f72b2ab025)), *Svelte, not React* | SvelteKit, Vite ^6.4.2 | Vitest ^3.2.6, jsdom ^20, `TZ=UTC` | 10.5.5 with addon-a11y and test runner ^0.24.4 | Playwright ^1.55.1: mocked "integration" tier and real-server "e2e" tier. Desktop and 320 px mobile projects. 3 retries | `@axe-core/playwright` ^4.10.1 with a custom accessibility reporter. An "A11y PR Triage" workflow | Chromatic (`chromaui/action` v11.27.0) | Plumbing, English only | ESLint, stylelint, `eslint-plugin-playwright` |
| **Dagster** ([`c97898c`](https://github.com/dagster-io/dagster/tree/c97898c3e4a8a3cc5e32fcc2ca97aea810342ed9)) | React ^18.3.1, webpack | Jest ^29.7.0, jsdom, Testing Library react ^16, run through tox on Buildkite ([`tox.ini`](https://github.com/dagster-io/dagster/blob/c97898c3e4a8a3cc5e32fcc2ca97aea810342ed9/js_modules/tox.ini)) | 10.2.10 with `@chromatic-com/storybook` ^4.1.1 (no Chromatic CI job visible) | None found | None found | None visible | No | ESLint 9, `tsgo` |
| **PostHog** ([`c4545bc`](https://github.com/PostHog/posthog/tree/c4545bc76c04bb67ef08a3b4188b6dac41b93b5b)) | React ^18.3.1, Vite | Jest, jsdom and happy-dom ^20.9.0, Testing Library react ^14.3.1, MSW ^2.14.6 | Storybook with test runner ^0.24.4, addon-a11y ^10.4.3, pseudo-states | Playwright 1.60.0 against a Docker Compose stack. 1 retry on CI | Storybook a11y addon | `jest-image-snapshot` ^6.4.0 in a pinned Playwright image, Chromium and WebKit, gated by its own Visual Review | No | oxlint ^1.72.0, stylelint |
| **Sentry** ([`c87c450`](https://github.com/getsentry/sentry/tree/c87c4508d36f126ef0db812fd5f8b060fcf4b70e)) | React 19.3.0, rspack | Jest 30.5.1, jsdom 30.1.1, Testing Library react 16.3.2, `jest-fail-on-console` | Own stories with preview links (`stories-preview.yml`) | pytest and Selenium acceptance tests | Not found in CI | Component snapshots with Playwright ^1.61.1 Chromium ([`snapshot.ts`](https://github.com/getsentry/sentry/blob/c87c4508d36f126ef0db812fd5f8b060fcf4b70e/tests/js/sentry-test/snapshots/snapshot.ts)), uploaded from CI | Yes (section 5) | oxlint 1.85.0, oxfmt, ESLint 10.8.0, knip 6.40.0 |
| **Prefect `ui-v2`** ([`302beb3`](https://github.com/PrefectHQ/prefect/tree/302beb37eb79cd6ba37fcaa9fa002c4429b45702)), *React with shadcn on Radix and Vite, the closest stack* | React 19.2.8, Vite ^8.2.0 | Vitest ^4.1.11, jsdom ^30, Testing Library, MSW ^2.15.0, 4 shards | 10.5.5 with `msw-storybook-addon` ^3 | Playwright ^1.62.0 against `prefect server` and PostgreSQL. 3 shards, 1 retry | `@axe-core/playwright` ^4.12.1 | None found | No | Biome ^2.5.6, ESLint ^10.8 |
| **Base UI** ([`81686db`](https://github.com/mui/base-ui/tree/81686db2e4ed3581941b7f8aa5feddac6af76152)), *the Console's component library* | React 19.3.0 | Vitest 5.0.2 Browser Mode: Chromium by default, plus jsdom, Firefox and WebKit | – | Vitest e2e project | Guidepup NVDA on `windows-2025` | Argos (`test:argos`) | – | – |

**Patterns across the table:**
- **Unit tests:** Jest with jsdom is still the most common runner among large, older consoles (Grafana, Dagster, PostHog, Sentry). Projects on Vite use Vitest (Supabase, Prefect, Temporal), and the component library itself uses Vitest Browser Mode.
- **End to end:** Playwright everywhere except Sentry. Every project tests against a real backend. Most retry on CI.
- **Storybook:** three of the seven projects still use the test runner that Storybook 11 stops supporting.
- **Accessibility:** axe through Playwright and/or Storybook in five of the seven.
- **Localization:** only two of the seven localize.

---

## Unverified

- **Translation coverage per locale** in Grafana's 19 and Sentry's 44 locales was not measured. The locale lists exist, but how much of the interface each one translates is unknown. Whether Sentry's RTL locales (`ar`, `he`, `fa`) render right to left was not checked.
- **Competitors' web interfaces.** No language setting in the Apify, Browserbase or Firecrawl interfaces comes from their docs only. Their interfaces were not inspected.
- **CSP compatibility.** Whether `page.clock`, `routeWebSocket`, axe injection, MSW, and react-intl's or i18next's runtime work unchanged under the Console's `require-trusted-types-for 'script'` policy was not tested. Grafana's opt-in strict-CSP run suggests Playwright works, but it is not a CI gate there.
- **Shard weighting.** `PWTEST_SHARD_WEIGHTS`, which Supabase uses for weighted shards, does not appear in Playwright's public docs.
- **Font waiting.** Whether Playwright's `toHaveScreenshot` waits for web fonts is not documented in the API docs read. Argos documents it for `argosScreenshot`.
- **Dagster and Chromatic.** Whether Dagster runs Chromatic is unknown: the addon is installed but no public CI job was found (its CI is Buildkite plus a private copy).
- **Sentry's snapshot review.** How Sentry reviews uploaded snapshots (`frontend-snapshots-upload.yml`) was not read.
- **Prices change.** Chromatic's overage on its Pro plan, and Percy's free-tier history retention, were not stated on the pages read.
- **EN 301 549 v4.1.1.** The publication date (2026-09-02) and the expected Official Journal citation (around 2026-11-30) come from Deque's post, not from ETSI or the Official Journal.
- **Absence claims.** "None found" for visual regression in Grafana, Supabase and Prefect rests on GitHub code search for `toHaveScreenshot` and on their workflow lists. Other tools may exist under other names.
- **The Lighthouse CI gate.** Whether Docusaurus's Lighthouse workflow fails on an accessibility-score drop, or only comments, was not checked.
- **Vitest benchmark.** The Browser Mode versus jsdom gap uses different pools in the Vitest 5 benchmark, so the 2× figure is indicative only.
