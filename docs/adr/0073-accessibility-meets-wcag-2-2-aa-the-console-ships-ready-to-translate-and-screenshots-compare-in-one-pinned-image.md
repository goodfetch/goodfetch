# Accessibility meets WCAG 2.2 AA, the Console ships in English ready to translate, and screenshots compare in one pinned image

The Console, the docs site and the Live view packages' own controls are held to WCAG 2.2 AA. axe runs with the `wcag22aa` tag on every Storybook story, at the end of every served test and on every docs page, and browser tests measure what axe cannot: keyboard walks, focus, reflow, text spacing, motion and forced colours. Real screen readers run as a Live check and in a manual pass. The CLI's colour never carries meaning alone. V1 ships every interface in English, but the Console is internationalized: every string comes from a Lingui catalog, two pseudo-locales (one right to left) run its tests, and formatting follows the browser's locale and time zone. Nothing Goodfetch produces depends on the host's locale or time zone. Visual regression uses self-hosted Playwright and Vitest screenshots, compared only inside one pinned Linux x86-64 image, with zero mismatched pixels and reference screenshots committed beside their tests.

No ADR set an accessibility target, said whether V1 is translated, or fixed how screenshots are compared. [ADR 0068](0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md) fixed the runners these checks plug into: `@axe-core/playwright` through a shared fixture, Storybook's a11y addon, oxlint's `jsx-a11y`, `toMatchScreenshot` and `toHaveScreenshot`, and a locale as a project parameter. Comparable projects show the shape:
- **axe's WCAG 2.2 rules are off by default** "until WCAG 2.2 is more widely adopted" ([rule descriptions](https://github.com/dequelabs/axe-core/blob/develop/doc/rule-descriptions.md#wcag-22-level-a--aa-rules)). EN 301 549 v4.1.1 (2026-09-02) adopts WCAG 2.2 AA. Automated tools find a minority of barriers: Deque reports 57% of issues by volume, and GOV.UK found that axe detected 29% of 142 deliberate barriers.
- **Screen-reader automation is partial.** Guidepup drives VoiceOver and NVDA only, and Base UI runs two NVDA specs on a Windows runner. `@guidepup/virtual-screen-reader` simulates one over the DOM.
- **Of seven consoles checked, only Grafana and Sentry localize.** Grafana pairs its 19 locales with a pseudo-locale, a lint rule for untranslated strings and a re-extraction job. Temporal UI has i18n plumbing with English only. Users without a translation run browser translation, which can crash React apps ([react#11538](https://github.com/facebook/react/issues/11538)).
- **Screenshots are stable only in one pinned environment.** A Playwright maintainer says arm64 and amd64 Docker images "are expected" to produce different screenshots ([playwright#13873](https://github.com/microsoft/playwright/issues/13873)). Argos's open-source sponsorship requires "not for commercial use"; Percy is free through BrowserStack's open-source programme or a 5,000-screenshot tier; Lost Pixel is archived.

Decided in [Accessibility, localization and visual regression](https://github.com/goodfetch/goodfetch/issues/283) ([grilling record](https://github.com/goodfetch/goodfetch/issues/283#issuecomment-6154845613)), with the agent answering on the owner's delegation. The facts come from [Testing a web console and a docs site](https://github.com/goodfetch/goodfetch/issues/265) ([findings](https://github.com/goodfetch/goodfetch/blob/research/web-console-testing/rnd/research/2026-10-13-web-console-testing.md)) and the official pages the grilling record cites. It builds on ADRs 0059, 0061 to 0064, 0068, 0069, 0071 and 0072. The rules every test follows are in `TESTING.md`.

**Two product decisions here need the owner's confirmation:** that V1 ships in English only, ready to translate (below, "What V1 translates"), and that the docs publish an accessibility statement.

## Decision

### The target

- **WCAG 2.2 AA** for the Console, the docs site and the Live view packages' own controls.
- **The remote page in the Live view is third-party content**, under WCAG's statement of partial conformance. Its controls, its accessible name and its announcements are held to AA.
- **The docs publish an accessibility statement:** the target, known exceptions and how to report a barrier, linked from every page's footer.
- **The CLI:** colour and motion never carry information alone, `NO_COLOR` is honoured as no-color.org states it, and output to a pipe holds no escape sequence.

### Automated accessibility checks

- **Lint:** every oxlint `jsx-a11y` rule is an error.
- **Storybook stories:** Storybook's a11y addon runs on every Storybook story (a story here is a Storybook story, never ADR 0072's `story:` Feature) with `parameters.a11y.test = 'error'` and the tags `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`, `wcag22aa` and `best-practice`, in light and dark themes. Storybook's default disabling of `region` stays, since a story is a fragment.
- **Served Console and docs:** the shared Playwright fixture runs axe, with the same tags, at the end of every served test and every `console` Journey variant, and wherever a test calls `expectAccessible(page)` (an open dialog, drawer or menu). The docs pass runs axe on every `sitemap.xml` page of `next`, in light and dark. axe runs wherever these tests run, so in every browser in the Release suite.
- **Exceptions:** a test never disables a rule. A known false positive is excepted only in one exception file the fixture reads, each entry naming the rule, a selector, the reason and an open issue; the fixture fails when the issue is closed or the selector matches nothing. axe's "incomplete" results are attached to the report and never fail a test.
- **Beyond axe,** measured by browser tests with shared helpers:
  - **keyboard walk** per screen: Tab and Shift+Tab reach every element with an interactive role, focus is never lost to `body`, Escape closes each overlay and returns focus to its trigger; the ordered accessible names are a snapshot;
  - **focus visible:** each stop's element looks different focused and unfocused;
  - **focus not obscured:** no stop's element is entirely covered by sticky headers, drawers or toasts;
  - **reflow:** at 320 × 256 CSS pixels there is no horizontal page scroll, except inside data tables and code blocks, which scroll in their own container;
  - **text spacing and 200% text:** no text is clipped by an `overflow` box without a way to read it in full. These run on the Console's `tests/screens/` in the `component` project: served tests run under the production CSP, which refuses an injected style sheet, and Playwright has no option for either;
  - **target size:** axe's `target-size` rule through `wcag22aa`;
  - **dragging:** every drag has a single-pointer alternative, tested by performing the action through it;
  - **reduced motion:** under `reducedMotion: 'reduce'`, `document.getAnimations()` holds no running animation after an action;
  - **colour never alone, forced colours and more contrast:** screenshots (below).
- **Announcements:** `@guidepup/virtual-screen-reader` tests in the `component` project assert the spoken log of live regions ("Needs you", toasts, errors, run-state changes), route titles and Live view control changes.
- **Features** ([ADR 0072](0072-every-feature-in-a-generated-inventory-has-a-passing-test-and-code-coverage-and-mutants-are-signals.md)): axe, keyboard walks, focus, reflow, announcement and screenshot tests declare no Feature. They check how a screen is built, not the outcome its `screen:` Feature names, which the screen's own behaviour tests declare. The keyboard walk has one named served case per `screen:` Feature of the inventory, ending with the fixture's axe pass, so a new screen gets both without a list to keep. WCAG 2.2 AA is a target the checks enforce, not a Feature, and needs no exception. The accessibility statement is a page of the docs: area 1's spec adds its user story, and a docs test declares that `story:` by asserting the page is served and linked from every page's footer.
- **Real screen readers:** a Live check, `screen-reader`, runs Guidepup with NVDA on Windows and VoiceOver on macOS over a few served flows, nightly. It asserts expected names and roles in the spoken log, never whole phrases. It is owned by area 12 and blocks nothing directly (ADR 0064); while the nightly tier is off (ADR 0032), it runs before each release candidate. Speech and runner images move without a commit, so it cannot be a test.

### The Live view

- **A documented key always moves focus out** of the Live view while a person holds control, and is shown on screen in control mode. A served test proves that it returns focus to the Console and that every other key reaches the remote page. Area 12 names the key.
- **Keyboard layouts and IME:** a Proving ground page records each `keydown` (`key`, `code`) and composition event. On Chromium, a served test drives the Live view with layout pairs (AZERTY `code=KeyQ key=a`, QWERTZ `code=KeyY key=z`, a dead-key sequence) and a composition sent to the Console page through CDP's `Input.imeSetComposition`, and compares the page's record with literal expected events.
- **Console fields:** Enter during a composition never submits a form or runs a search.

### What V1 translates

*A product decision the owner should confirm.*

- **V1 ships in English:** the Console, the docs, the CLI, error messages, MCP descriptions and skills.
- **The Console is internationalized.** Every string comes from its message catalog, with ICU messages; dates, numbers, sizes, durations, lists and plurals are formatted from the browser's locale and time zone; layouts work right to left; each Error code maps to a catalog message, falling back to the engine's English message. Adding a language is a catalog file.
- **This is not a scope cut.** The readiness work is in V1; only translated content waits. If the owner chooses to ship translations in V1, every rule below stays, and each shipped locale adds a completeness check (every key translated), its own axe and screenshot runs, and a manual review of glossary terms.

### Localization checks

- **Lingui** holds the catalog, for area 12 to confirm: it compiles catalogs ahead of time, so no message parser runs under Trusted Types or counts against ADR 0071's bundle ceiling, and it generates pseudo-locales.
- **`lingui extract`** runs in the Quick suite and fails on an uncommitted catalog change.
- **`lingui/no-unlocalized-strings`** runs through oxlint's JS plugins, pinned. They are alpha, so the first fixtures ticket proves the rule runs; if it does not, ESLint runs that one plugin alone.
- **Data renders inside `translate="no"`:** URLs, names, IDs, page text and every value from the engine. Browser translation then leaves data alone, and the pseudo-locale test can tell data from interface text.
- **Two pseudo-locales:** `en-XA` (accented, about 35% longer, bracketed) and `ar-XB` (`rightToLeft: true`). They are Chromium's pseudo-locale tags, well-formed BCP 47, so `Intl` accepts them as formatting locales. The `component` project and the served tests run under both in the Release suite:
  - under `en-XA`, every visible text node is bracketed or inside `translate="no"`, no text is clipped, and the keyboard walk passes;
  - under `ar-XB`, the root has `dir="rtl"` and arrow keys in menus and tabs follow the direction.
- **Browser translation:** a Release-suite served project adds a page script that wraps every text node in a `<font>` element as it appears, as page translators do; the served tests must pass with no uncaught error.

### Locale and time zone independence

- **The Console's formatting module** uses only `Intl` with the active locale and zone. Its expected strings are asserted only in the `unit` project, on one pinned Node line and so one ICU. Browser tests never assert formatted text: they assert the machine form (`<time datetime>`, a `data-value`) or a role, since each browser ships its own ICU and CLDR.
- **One shared table of locales and time zones** in `goodfetch-test-support`, read by the Rust, TypeScript and Python test helpers: `en-US`, `de-DE`, `fr-FR`, `tr-TR`, `ar-EG`, `hi-IN`, `ja-JP`; `UTC`, `America/St_Johns` (−3:30), `Asia/Kathmandu` (+5:45), `Pacific/Chatham` (+12:45), `Australia/Lord_Howe` (a 30-minute DST shift), and the DST gap and overlap days of `Europe/Paris` and `America/New_York`. Period Limits and Schedules take their zone cases from it.
- **An "elsewhere" served project** reruns the served tests and `console` Journey variants with `locale: 'tr-TR'` and `timezoneId: 'Asia/Kathmandu'` in the Release suite.
- **The CLI's human output reads no locale** (English, `.` decimals, ISO 8601). A `tests/it` case runs the snapshot commands under `LANG` and `LC_ALL` set to `tr_TR.UTF-8` and gets the same bytes. Human output shows times in the local zone with their offset, for area 11 to confirm; a test checks that two `TZ` values print the same instant. `--json` is always UTC.
- **Engine results never depend on the host's locale or zone**, except where a setting names a zone. `goodfetch-test-env` sets `TZ=UTC` and `LANG=C.UTF-8` in every child's environment unless a test names others. The Release suite reruns the engine's integration tests and the Journeys once on Linux and macOS with `TZ=Pacific/Chatham` and `LANG=tr_TR.UTF-8`; Script sandbox results must match.

### Languages, encodings and paths

- **The Proving ground's encodings family** (area 2's CO-24 pages) covers at least Shift_JIS, EUC-KR, GB18030, Big5, windows-1251, ISO-8859-2, ISO-8859-15, UTF-16 with a BOM, a charset that contradicts its `<meta>`, and no declared charset. **Its language family** covers Arabic and Hebrew, CJK without spaces, Devanagari, Thai, combining marks, emoji sequences, astral characters, IDN host names and non-ASCII paths and queries. The oracle holds the expected text in UTF-8, in the normalization form served.
- **One Unicode record** in `goodfetch-test-support` (bidi text with marks, the NFC and NFD forms of one word, a ZWJ emoji sequence, an astral character, a zero-width joiner, a BOM, and a tab and a newline inside a field) round-trips byte for byte through every Sink conformance case and every Export format, and renders in a Console story with each cell isolated by `dir="auto"`.
- **Each test's folder name holds a space and a non-ASCII letter** on every OS, so every Goodfetch home in a test lives under such a path.

### Visual regression

- **Self-hosted:** `toMatchScreenshot` on stories and `toHaveScreenshot` in served and docs tests. No hosted service in V1; adopting one is a paid test resource the owner approves.
- **One environment:** visual tests are their own projects (`visual` in Vitest and in Playwright), run only inside the Playwright image pinned by digest, on Linux x86-64, in both suites. No other OS or architecture schedules them.
- **Determinism:**
  - the Console and the docs ship their fonts as files, and the shared helper awaits `document.fonts.ready` before each screenshot;
  - a fixed instant (`page.clock.setFixedTime`; Vitest fake timers in components), `locale: 'en-US'`, `timezoneId: 'UTC'`, builder data with literal defaults, no random value on screen;
  - viewport and `deviceScaleFactor: 1` set per project, `reducedMotion: 'reduce'`, animations disabled, caret hidden;
  - masks only over pixels Goodfetch does not own, the Live view's frames.
- **No suite writes a Reference screenshot:** the shared Playwright config sets `updateSnapshots: 'none'` in the suites, since Playwright's default writes a missing one.
- **Zero tolerance:** the default per-pixel threshold and zero mismatched pixels. A config check fails if `maxDiffPixels`, `maxDiffPixelRatio`, `allowedMismatchedPixels`, `allowedMismatchedPixelRatio` or `threshold` is set outside the shared config.
- **Repeatability per browser is unverified for Firefox and WebKit.** Chromium in a pinned image is what comparable projects run (PostHog); nothing in the findings shows the same for Firefox's and WebKit's Linux builds. The first fixtures ticket measures each browser with ADR 0064's 20 repeated runs. If a browser's screenshots do not repeat, it stays out of the `visual` projects, its served tests and axe keep running, and an issue owned by area 12 tracks it until 20 runs repeat, when it joins them. No tolerance is ever added. After that measurement, a screenshot test that stops repeating is a Flaky test under ADR 0064: quarantined, never removed.
- **A Reference screenshot is neither a fixture nor a snapshot** in ADR 0062's sense. Their 256 KiB limit, their `.source.toml` and "CI never writes a snapshot" do not apply, and its own rules do: its provenance is the pinned image's digest and the workflow run named in the commit that added it.
- **Reference screenshots** sit beside their tests (Vitest's `__screenshots__/`, Playwright's `-snapshots/`), named without the platform. Each is at most 512 KiB, checked in the Quick suite; a larger one is clipped to the region the test is about. No Git LFS.
- **Updates come only from the pinned image.** The `update-screenshots` workflow, dispatched on a pull request's branch, regenerates changed references and uploads them as an artifact with the image digest and the commit it ran on. It is the only writer of Reference screenshots and is not a suite job, so no suite ever writes one. `just screenshots-fetch <run>` refuses a run whose digest is not the pinned one or whose commit is not the branch's head, then applies them; the author commits them with their own sign-off and names the run in the commit message. No bot commits.
- **Locally,** `just` runs the visual projects through Docker or Podman in the same image; a host with neither fails that step with an error naming the fix. Whether the amd64 image under emulation on Apple Silicon gives CI's pixels is unverified, and arm64 and amd64 images are expected to differ ([playwright#13873](https://github.com/microsoft/playwright/issues/13873)): the first fixtures ticket measures it, and if it does not, local runs on such hosts dispatch the visual job to CI and follow it, like `just release-ci`.
- **Review:** a reference change is in the pull request's diff, which GitHub shows as 2-up, swipe or onion skin, and the fresh review checks it is intended. A failed comparison uploads the expected, actual and diff images with the report.
- **Orphans:** a run of every visual project fails on a reference screenshot no test read.
- **What is screenshotted:**
  - every Storybook story, light and dark, on Chromium; hover, focus and pressed states come from real `userEvent` input in `.test.tsx` files;
  - served key screens (sign-in, Runs, a run page in each Lifecycle and Verdict, "Needs you" with each item kind, the Live view watching and in control, Datasets and the SQL tab, Recipes, Schedules, Engine & settings, an error state): light and dark at 1280 × 800 on Chromium in Pre-merge; on Firefox and WebKit, and at a phone viewport on Chromium and WebKit, in Release; under `forcedColors: 'active'`, `contrast: 'more'` and both pseudo-locales on Chromium in Release;
  - the Console's `tests/screens/` under a greyscale filter, text spacing and 200% text, in Vitest's `visual` project in Release, where a style sheet can be injected;
  - docs page templates, not every page: home, a guide, a generated reference page, a page with language tabs and code, search open, the version banner and a 404, light and dark, desktop and phone.

### Placement

| Check | Suite |
|---|---|
| `jsx-a11y`, `no-unlocalized-strings`, the `lingui extract` check, the formatting table, reference-screenshot sizes, the tolerance check | Quick |
| axe on every Storybook story and at the end of every served test and Journey variant; keyboard walks, focus, reflow, target size, reduced motion; virtual screen reader; layout and IME tests; the CLI's colour and locale cases; docs axe; visual projects on Chromium | Pre-merge (Linux) |
| axe in served Console tests in every browser on every OS runner; text spacing and 200% text; pseudo-locales; the elsewhere and translation projects; the host-elsewhere engine run; Firefox, WebKit, phone and special-mode screenshots; the orphan check | Release |
| `screen-reader` | Live check, nightly |

### Manual tests

Only these stay manual, on each minor's release candidate once the Console ships, recorded on the release pull request:
1. **Screen-reader pass:** NVDA with Firefox and Chrome on Windows, VoiceOver with Safari on macOS and iOS, TalkBack with Chrome on Android, over the minor's Console Journeys and changed screens, a few docs pages, and the CLI's common commands in a terminal.
2. **Keyboard, zoom and contrast pass:** keyboard only, 400% zoom, the OS magnifier, a Windows contrast theme and speech input on changed screens, in Chrome, Firefox and Safari.
3. **Real IMEs and keyboard layouts:** Japanese, Chinese Pinyin and Korean IMEs, AZERTY and a dead-key layout, in the Live view and Console fields, on macOS and Windows. A CDP composition is not the OS's IME.
4. **Browser translation:** Chrome's and Safari's page translation over the main screens.
5. **WCAG review of new screens** for what no tool judges: meaningful names and instructions, error suggestions, consistent help, redundant entry, sensory-only cues; the accessibility statement is kept current.

## Considered Options

- **WCAG 2.1 AA, axe's default tags:** rejected. Superseded, and it misses target size, focus not obscured, dragging and accessible authentication.
- **WCAG 2.2 AAA:** rejected. W3C does not recommend AAA as a general policy for whole sites.
- **An `@a11y` smoke suite** (Grafana): rejected. It covers only the pages someone listed; axe at the end of every served test covers every screen tests reach.
- **Storybook's a11y addon in `'todo'` mode:** rejected. Warnings are ignored.
- **Guidepup in the Release suite:** rejected. Its speech and its runner images move without a commit, which makes it a Live check by ADR 0064.
- **English only with no i18n layer** (PostHog, Supabase Studio): rejected. Hard-coded and concatenated strings creep in, and adding a language later touches every screen.
- **Translated V1** (Grafana, Sentry): not chosen, pending the owner. It needs translators or a paid service and a glossary review per language, for every docs version too.
- **i18next or react-intl:** rejected for Lingui. Both parse messages at run time unless precompiled, and react-intl's bundle is larger against the Console's ceiling.
- **Asserting formatted text in browser tests:** rejected. Each browser ships its own ICU and CLDR, so the expected string would depend on the browser version.
- **A hosted visual service** (Argos, Chromatic, Percy): rejected for V1. Argos's sponsorship excludes commercial use, the others are paid beyond a free tier, and uploads would cross ADR 0043's deny. GitHub's image diff is enough to review.
- **Screenshots on every OS runner, or a tolerance to absorb differences:** rejected. Rendering differs by OS and architecture, and a tolerance hides real one-pixel regressions such as a lost focus ring.
- **Screenshots of every docs page** (Docusaurus): rejected. Every content edit would churn references; templates catch theme regressions.
- **Git LFS for reference screenshots:** rejected. LFS has bandwidth quotas and needs every checkout to fetch it; small committed PNGs suffice.
- **A bot that commits updated references:** rejected. Its commits would carry no DCO sign-off.
- **A pseudo-states addon for hover and focus:** rejected. Real input through `userEvent` is what ADR 0068 already uses.

## Consequences

- **Adds to ADR 0068:** the `visual` projects, run only in the pinned image; the axe tags and the exception file; `expectAccessible`; the pseudo-locale, elsewhere and translation projects; the reduced-motion, reflow and text-spacing checks.
- **Amends ADR 0061's Release suite:** visual projects run on one platform, Linux x86-64 in the pinned image, not on every platform. An amendment note marks its platform bullet.
- **Adds to ADR 0064:** a Live check, `screen-reader`, owned by area 12, nightly (before each release candidate while the nightly tier is off), blocking nothing directly. `TESTING.md`'s list of Live checks names it.
- **Adds to ADR 0059:** each child's environment sets `TZ` and `LANG` (UTC and `C.UTF-8` by default), and each test's folder name holds a space and a non-ASCII letter. `TESTING.md`'s isolation list says so.
- **Adds to ADR 0069:** the CLI's colour properties (stripped colour output equals `NO_COLOR` output; `NO_COLOR` empty or not; `--color always` overrides it), the escape-byte check on piped output in the shared CLI helper, and the locale and time zone cases.
- **Adds to ADR 0031:** the Live view's escape key, its accessible name and announcements, and the layout and IME tests.
- **Adds to ADR 0062:** a Reference screenshot is neither a fixture nor a snapshot; its size limit, writer and provenance are its own, as above.
- **Adds to ADR 0072:** accessibility and visual tests declare no Feature, and take their case tables from the inventory's `screen:` Features; the accessibility statement's user story is declared by a docs test.
- **Also adds to ADR 0062:** `goodfetch-test-support` holds the shared locale and time zone table and the Unicode record; `@goodfetch/test-support` the axe fixture, the exception file reader and the keyboard-walk, focus and clipped-text helpers.
- **Adds to ADR 0026 and the Export formats:** the Unicode record in every Sink conformance case and every Export format.
- **ADR 0037's area table** lists this ADR in the rows of area 1 (the pinned image, the `update-screenshots` workflow, the Live check's job), area 2 (the encodings and language families), area 3 (host-independent results), area 6 (the Unicode record through Sinks and Exports), area 11 (the CLI) and area 12 (the Console).
- **Area 12's spec must confirm:** Lingui; the Live view's escape key; reduced-motion behaviour; fonts shipped as files; formatting from the browser's locale and zone; Error codes mapped to catalog messages; `translate="no"` on data.
- **Area 11's spec must confirm:** the CLI's human output reads no locale and shows local times with their offset.
- **Area 2's spec** gains the listed encodings and language pages.
- **Area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)) gains the pinned Playwright image by digest, the `update-screenshots` workflow and `just screenshots-fetch`, the visual job, and the docs' accessibility statement page.
- **The "Test infrastructure" spec** builds the axe fixture and exception file, `expectAccessible`, the keyboard-walk, focus, reflow and clipped-text helpers, the locale and zone table, the Unicode record, the visual config with its tolerance, size and orphan checks, the translation-emulation script, and the emulation measurement on Apple Silicon.
- **Owner decisions:** V1 in English, ready to translate; the published accessibility statement; any hosted visual service later.
