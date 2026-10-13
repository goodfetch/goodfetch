# Testing

How Goodfetch is tested: every surface, at every level. Each rule comes with its reason, and the hard-to-reverse ones are decided in ADRs. [`STYLE.md`](STYLE.md) shapes the code these tests drive.

## Injection and determinism

Decided in [ADR 0059](docs/adr/0059-dependencies-are-injected-as-values-and-only-time-and-randomness-as-handles.md).

### What is injected, and how

| Dependency | Injected as | In tests |
|---|---|---|
| Wall time, monotonic time, sleeps | the `Clock` handle | its test version under Tokio's paused clock |
| Random choices (jitter, pool picks, fingerprint seeds, ID randomness) | the `Entropy` handle | a fixed seed, printed at test start |
| Secret randomness (keys, tokens, run data keys, CAs) | nothing: the OS generator | the OS generator |
| Data, caches and configuration | the `GOODFETCH_HOME` value | a per-test temporary folder |
| SQLite and files | paths under that home | real files |
| Listen addresses | a value (port 0, or a socket path) | the bound address reported back |
| Host names | the `resolve` value | Proving ground hosts mapped to loopback |
| Environment | an explicit map | a cleared map for each child process |
| Keychain, service manager, processes | nothing: the real service | the real service, unique names |

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests.
  _Why:_ a fake passes where the real thing fails, and a stand-in tests what users run.
- **Nothing reads time, chooses at random or reads the environment through a global.** `disallowed-methods` refuses `SystemTime::now`, std and Tokio `Instant::now` and constructors such as `Ulid::new` outside `Clock` and `Entropy`, and `std::env::var` outside `main`; `std::env::set_var` is `unsafe`.

### Time

- **A rule that depends on time is a pure function that takes "now"**, tested with no clock: lease expiry, a Schedule's next occurrence (DST included), backoff. The Politeness store's granted shares stay in its Lua script and are tested against a real Valkey.
- **Code that waits is tested in-process with the test `Clock`** (`#[tokio::test(start_paused = true)]`). Advancing the paused clock moves wall time with it, and `jump_wall` moves wall time alone.
  _Why:_ a default-length lease or a day-long interval then takes no real time.
- **Keep real I/O out of paused-clock tests**, apart from SQLite on tracked blocking threads.
  _Why:_ auto-advance fires whenever the runtime is idle, including during a real socket wait, so a lease can expire early (tokio #4522, #8232).
- **A test of the binary sets short durations through settings.** No build has a test-only clock control.
- **Never sleep for real** beyond a short duration the test set itself.
- **Clocks Goodfetch doesn't own stay real:** the Valkey server's (short durations), Chromium's (Proving ground pages take their time step from the task), and the Script sandbox's live `Date.now`, whose tests compare the order of Effects.

### Randomness

- **Every random choice draws from `Entropy`** (`chacha20`, portable across versions).
- **A test runs on a fixed seed.** `GOODFETCH_TEST_SEED` overrides it, and the seed is printed when the test starts.
  _Why:_ a test nextest kills on a timeout never reaches a failure message.
- **Secrets never come from `Entropy`.** The Proving ground's seeded test certificate authority is the one exception.
- **Hash order never reaches output or ordering.** `iter_over_hash_type` is denied; use `BTreeMap` or `IndexMap` where order shows.

### Storage, network and OS services

- **Each test gets its own `GOODFETCH_HOME`** in a temporary folder, and writes nowhere else.
- **Proving ground host names resolve through the `resolve` setting** to loopback, which the run allowlists for tests.
- **Proxies are tested against the Proving ground's real forward proxy.**
- **Local addresses use real extra loopback addresses:** `127.0.0.0/8` on Linux and Windows (Windows not yet tested), `lo0` aliases on macOS, a network namespace under sudo where needed.
- **No test reaches the public internet.** Only the jobs ADR 0043 names go online, to their named endpoints; every other job runs under its deny with the canary.
- **The keychain provider runs against the real keychain** of each OS (`gnome-keyring` started and unlocked on Ubuntu), with a unique service name per test. Consumers of Secrets use the environment or caller provider.
- **`daemon install` installs for real on each OS.** The unit file it generates is snapshot-tested.

### Isolation

1. One process per test (nextest).
2. A per-test temporary `GOODFETCH_HOME`.
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder.
4. No fixed ports: bind port 0 and report the address, or use a Unix socket or named pipe in the test's folder.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Certificate authorities are made per test: the Proving ground's from its seed, a Daemon group's by `goodfetch group init` into the test's folder.
8. No test reads another test's data or depends on test order.

### SDKs and the Console

- **SDK durations and jitter are options:** tests pass short values and a fixed seed. In TypeScript, Vitest fake timers may drive timer logic that never reaches the engine.
- **The Console's end-to-end tests set time with Playwright's `page.clock`** (`setFixedTime` first). Its component tests use Vitest fake timers.

## Console and docs site

Decided in [ADR 0068](docs/adr/0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md). Which accessibility, localization and visual checks run on these runners is decided by [Accessibility, localization and visual regression](https://github.com/Jbmanllr/automan/issues/283).

### Where each test lives

| Path | What it tests | Runner | Suite |
|---|---|---|---|
| `console/src/**/*.test.ts` | logic with no DOM | Vitest `unit` (Node) | Quick |
| `console/src/**/*.test.tsx` | one component | Vitest `component` (Browser Mode) | Pre-merge |
| `console/src/**/*.stories.tsx` | each state of a component, rendered as a test | Vitest `component` (Storybook's addon) | Pre-merge |
| `console/tests/app/` | the whole Console against the in-memory engine | Vitest `component` | Pre-merge |
| `console/tests/served/` | the Console served by the binary | Playwright Test | Pre-merge |
| `console/journeys/` | the `console` variants of Journeys | Playwright Test | Pre-merge, Release |
| `conformance/console/` | the Console conformance suite | Playwright Test | Pre-merge, Release |
| `website/src/**/*.test.ts` | the docs site's build code | Vitest (Node) | Quick |
| `website/tests/` | the built docs site | Playwright Test | Pre-merge, Release |

### Components

- **A test that renders or needs the DOM is a `.test.tsx` file** and runs in Vitest Browser Mode with the Playwright provider. A `.test.ts` file runs in Node and never touches the DOM. Never jsdom or happy-dom.
  _Why:_ Base UI's positioning, focus and pointer handling need a real layout.
- **Mount with `vitest-browser-react`, find with `vitest/browser` locators, act with `vitest/browser`'s `userEvent`.** Never `@testing-library/user-event`.
  _Why:_ events then go through CDP, and the locators are Playwright's.
- **The engine is the in-memory adapter of the protocol from `@goodfetch/test-support`, passed to the Console's Client as its transport and body fetcher.** Never MSW, never a patched global `WebSocket`. A call the test did not answer fails the test, and every message is validated against the schema bundle.
- **Test data comes from builders with literal defaults.** Override only what the test asserts on; no random data.
- **Each state of a component has a story** (`*.stories.tsx`, Storybook 10). Stories hold no assertions and no play functions. Behaviour goes in `.test.tsx`, which may reuse a story through `composeStories`.

### The served Console

- **Browser tests of the Console as users get it run on Playwright Test against the `goodfetch` binary built from the commit.** Never against a Vite server.
- **Each test starts its own Daemon** through the `@goodfetch/test-support` fixture (`goodfetch-test-env`), with its own home, port 0 and fresh browser context.
- **Sign in the way a person does:** the binary's single-user sign-in link, Playwright's virtual authenticator for passkeys and step-up, a local OIDC provider pinned by digest. Never write a cookie or storage state yourself.
- **Make faults where they happen:** `routeWebSocket` with `connectToServer()` for a dropped or stalled connection, the real process for an engine fault.
- **Find by role, label or text.** `data-testid` only where an element has no accessible role or name; never CSS classes or XPath.
- **Wait with web-first assertions and `expect.poll`.** Never `waitForTimeout` (oxlint refuses it) or `networkidle`.
- **A console error, an uncaught page error, a CSP violation or a failed same-origin request fails the test**, through the shared fixture.
- **Tests run under the production CSP.** The binary has no switch that relaxes it.
- **Keep traces on failure** and upload them with the JUnit files.

### The Console conformance suite

- **It runs the Console against a backend given as configuration:** the Daemon and the stub gateway here, Goodfetch Cloud's gateway from the private repository. Each target run writes a result document with `kind = "conformance"`.
- **Add a case when the Console starts to need something new from a backend.**

### Browsers

- **One Playwright version for the whole pnpm workspace**, shared by Vitest's provider.
- **The Console's and the docs site's tests run on one Node line;** their matrix is browsers.
- **Pre-merge:** Chromium on Linux. **Release:** components on Chromium, Firefox and WebKit; served tests, Journeys and conformance on every OS runner with every browser Playwright ships for it, plus phone emulation on Chromium and WebKit.

### Docs site

- **Docusaurus throws on every broken reference:** `onBrokenLinks`, `onBrokenAnchors`, `onDuplicateRoutes` and `markdown.hooks`' `onBrokenMarkdownLinks`, `onBrokenMarkdownImages` and `onUnusedMarkdownDirectives`.
- **Every kept version is built, and every page has its `.md` twin and an `llms.txt` entry.**
- **lychee runs with `--offline --include-fragments` in the suites.** External links are the nightly Live check `docs-external-links`.
- **Playwright tests the built site served locally under `/docs/` on port 0,** never a Vercel preview: every `sitemap.xml` page of every kept version answers 200 with no console, hydration or same-site request error, and the interactive components work.
- **After each production deploy, the Live check `docs-deploy-smoke`** checks the deployed pages, a `.md` page and `llms.txt` with their content types.
- **`typos` checks the docs' spelling.** No Vale, no cspell.
