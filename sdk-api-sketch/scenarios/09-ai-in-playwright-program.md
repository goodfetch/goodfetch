# 09. Add an AI step to an existing Playwright program

## Task statement

> You have an existing Playwright program: it launches Chromium, opens https://books.toscrape.com/, clicks the "Travel" category, then clicks the first book and prints its title before closing the browser itself. Without rewriting it, add one step on the page it already drives, right after the "Travel" click: with a language model, extract the title and price of every book listed. The existing program then carries on with its page as before. The added step must not close, navigate or take over anything the program opened, and if it fails, the program must still be able to continue and close its browser.

## Target

`books.toscrape.com`: the Travel category page and a book page (checked 2026-10-14).

## Leader

Stagehand 3.7.3 on a Playwright page (TypeScript). Stagehand 4 refuses Playwright pages (ADR 0037), so version 3 is the latest that does this task.

## Pains

P20 interop with existing Playwright code, P41 driver guarantees, P06 never close what you did not start, P05 two libraries in one program.

## Failure points

1. The added step closes, or kills, the browser it did not launch.
2. The added step leaves the page navigated or in a different state.
3. The two libraries act on the page at the same time.
4. The browser was launched in a way the added step cannot attach to: the error must say how to launch it.
5. The model fails: the existing program continues.

## Setup

The existing program's own setup is the same for every version and is not counted: `npm install playwright@1.64.1` (or `pip install playwright==1.64.0`) and `npx playwright install chromium` (`playwright install chromium`); the Rust program is given a Chromium executable. Counted is what the added step needs:

- **Stagehand 3 (TypeScript):** `npm install @browserbasehq/stagehand@3.7.3 zod`, `export ANTHROPIC_API_KEY=…`. The existing program's launch must also open a debugging port, since Stagehand attaches over CDP (counted under Lines).
- **Goodfetch TypeScript:** `npm install @goodfetch/collie zod`, `export ANTHROPIC_API_KEY=…`. No browser: the step borrows the program's Chromium through its Playwright page (ADR 0056's relay), so no port is needed.
- **Goodfetch Python:** `pip install goodfetch-collie pydantic`, `export ANTHROPIC_API_KEY=…`.
- **Goodfetch Rust:** `cargo add goodfetch-collie serde --features serde/derive`, `cargo add tokio --features macros,rt-multi-thread`, `export ANTHROPIC_API_KEY=…`. Rust has no Playwright, so its existing program launches Chromium itself with `--remote-debugging-port` and drives it through a Session borrowed by that address (ADR 0056's `cdp_url`): **the Rust comparison is structural**, not the same program.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand 3 with Playwright (TypeScript) | 13 | 24 | 2 | 1 / 2 / 2 |
| Goodfetch TypeScript | 10 | 17 | 2 | 4 / 0 / 1 |
| Goodfetch Python | 13 | 24 | 2 | 4 / 0 / 1 |
| Goodfetch Rust | 21 | 40 | 3 | 5 / 0 / 0 |

Lines count only what the added step needs, not the existing program.

**How the lines were counted.** Each program was measured whole after its formatter (oxfmt, ruff format, rustfmt), then the existing program's own lines were taken away: 8 in TypeScript and 12 in Python, measured on the existing program alone. Stagehand's 24 are 23 added lines and the launch line it changes to open a port. Goodfetch TypeScript's 17 are its two imports, the schema and the `try` block; Python's 24 are its imports, the schema, the step's function and its call. Rust's 40 are the step (its schema, `list_books`, its call and their imports: 24) and the Borrowed Session the program drives (connecting, starting the Run, opening the Session from its `cdp_url`, finishing: 16).

**Concepts counted.**

- Stagehand 3 (13): `Stagehand`, `env`, `"LOCAL"`, `localBrowserLaunchOptions`, `cdpUrl`, `model`, `init()`, `extract()`, its `page` option, `close()`, Playwright's launch `args`; must call `init()` first, must close. Not counted, though the step needs them: Chromium's `--remote-debugging-port` and the `webSocketDebuggerUrl` of `/json/version`, since 3.7.3 opens `cdpUrl` as a WebSocket.
- Goodfetch TypeScript (10): `Collie`, `connect()`, `start()`, `borrow()`, `locator()`, `extract()`, `model`, `output`, `finish()`; must dispose (`await using`).
- Goodfetch Python (13): `AsyncCollie`, `connect()`, `start()`, `borrow()`, `locator()`, `extract()`, `model=`, `output`, `finish()`, `GoodfetchError`, `code`, `fix`; must close (`async with`).
- Goodfetch Rust (21): `Collie`, `scope`, `ConnectOptions`, `start()`, `new_session()`, `source()`, `SessionSource::CdpUrl`, `url`, `headers`, `domains`, `exclusive`, `new_page()`, `Page`, `locator()`, `extract()`, `Schema::of`, `model()`, `output`, `finish()`, `Error`; closed by `scope`.

### Failure points, scored

A failure point that the library rules out by design, and refuses with a typed error when a call tries it, counts as typed.

| Failure point | Stagehand 3 | Goodfetch TypeScript | Goodfetch Python | Goodfetch Rust |
|---|---|---|---|---|
| 1. The step closes or kills the browser | typed (by design): `close()` of a `cdpUrl` attach only drops its socket (3.7.3 source) | typed (by design): ending the borrow never closes it (ADR 0078); a raw close is `cdp_command_refused` | typed (by design), as TypeScript | typed (by design): a Borrowed browser's Session never closes it (ADR 0056) |
| 2. The page is left navigated or changed | silent: its piercer script runs in the current document (`runImmediately`) and stays until the next navigation, unreported | typed (by design): the verbs on a locator never navigate, and no browser-wide setting is changed (ADR 0056) | typed (by design), as TypeScript | typed (by design), as TypeScript |
| 3. Both libraries act at once | silent: two CDP clients, nothing orders them | silent (gap): the program's own Playwright calls are outside the per-page order; results are only marked unguarded (ADR 0056) | silent (gap), as TypeScript | typed: one driver, whose gestures run one at a time (ADR 0085) |
| 4. The browser cannot be attached to | generic: `fetch failed` or the WebSocket's `ECONNREFUSED`; nothing says to open a port | typed: no port needed (relay over the page's own CDP session); a Firefox or WebKit page is `borrow_requires_cdp` | typed, as TypeScript | typed (gap): no ADR names the code for an unreachable `cdp_url` |
| 5. The model fails | generic: the AI SDK's error, read as text; the program's `catch` carries on | typed: `model_unavailable`, `model_unauthorized`, `extraction_incomplete`…, with `fix`; the `catch` carries on | typed, as TypeScript (`except GoodfetchError`) | typed: the `Err` arm carries on |

**Verdict.**

- Lines and concepts: Goodfetch's TypeScript step is shorter and names fewer concepts than Stagehand's, because the borrow takes the program's Playwright page itself, with no debugging port, address lookup or `init()`. Python's is as long as Stagehand's TypeScript because its step is a function and it catches the typed error by name (S1).
- Rust needs more of both because it has no Playwright: the program reaches the browser by its CDP address and drives it through Goodfetch's Session, so the comparison is structural (G7).
- Failure point 3 is silent in Goodfetch's TypeScript and Python versions, which G1 does not accept: it is raised as [The program's own driver acting during a Collie call on a borrowed page](https://github.com/goodfetch/goodfetch/issues/533). In this program the step is awaited before Playwright acts again, so the overlap cannot happen here.

**Raised while writing it:** [The program's own driver acting during a Collie call on a borrowed page](https://github.com/goodfetch/goodfetch/issues/533), [Error codes for an unreachable cdp_url and a missing browser build](https://github.com/goodfetch/goodfetch/issues/534).

## After the sketch

A new Journey on the SDK surfaces, with a Borrowed browser (ADR 0056); Proving ground family: rendered and interactive pages.
