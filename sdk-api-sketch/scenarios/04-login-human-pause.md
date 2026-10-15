# 04. Log in once with a person, reuse the login headless

## Task statement

> Log in to https://quotes.toscrape.com/login once with a person at the keyboard. Open a visible browser at the login page, fill the username from the program and the password from a secret the program never holds in clear (an environment variable or the system's keychain), then wait for the person to check the form and press Login, with no time limit while waiting and a way to cancel. Keep the logged-in state under a name. In a second, headless run, reuse that state to open https://quotes.toscrape.com/ and confirm that the page shows a Logout link. If the kept state has expired, say so instead of failing on a missing element. The password must not appear in any log, trace or saved file.

## Target

`quotes.toscrape.com/login`: a form with a CSRF token, where any username and password log in (checked 2026-10-14).

## Leader

Playwright 1.64.1 (TypeScript) and 1.64.0 (Python): `page.pause()`, then `storageState` saved and reused.

## Pains

P40 login and a person in the loop, P14 secrets, P50 cookies and the Session, P08 deadlines (a pause suspends them).

## Failure points

1. The person never finishes, or closes the window.
2. The kept state has expired or been revoked.
3. The secret is missing or cannot be read.
4. The password reaches a log, a trace, a screenshot or the kept file.
5. The kept state is written where other users can read it.
6. The headless run is treated by the site as a different browser.

## Setup

Counted apart from lines. Every version needs the password where its Secret reference or the program reads it, here an environment variable (`env:QUOTES_PASSWORD`; a `keychain:` reference instead needs the keychain entry), and a desktop for the visible window. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057).

| Program | Commands | Count |
|---|---|---|
| Playwright (TypeScript) | `npm install playwright@1.64.1`; `npx playwright install chromium`; `export QUOTES_PASSWORD=…` | 3 |
| Playwright (Python) | `pip install playwright==1.64.0`; `playwright install chromium`; `export QUOTES_PASSWORD=…` | 3 |
| Goodfetch TypeScript | `npm install @goodfetch/collie`; `export QUOTES_PASSWORD=…` | 2 |
| Goodfetch Python | `pip install goodfetch-collie`; `export QUOTES_PASSWORD=…` | 2 |
| Goodfetch Rust | `cargo add goodfetch-collie`; `cargo add tokio --features macros,rt-multi-thread,signal`; `export QUOTES_PASSWORD=…` | 3 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Playwright (TypeScript) | 21 | 42 | 3 | 1 / 2 / 3 |
| Playwright (Python) | 22 | 39 | 3 | 1 / 2 / 3 |
| Goodfetch TypeScript | 41 | 59 | 2 | 6 / 0 / 0 |
| Goodfetch Python | 39 | 58 | 2 | 6 / 0 / 0 |
| Goodfetch Rust | 52 | 86 | 3 | 6 / 0 / 0 |

Programs, each one program with two entry points (`--reuse` runs the second, headless run): [`leaders/04-login-human-pause.playwright.ts`](leaders/04-login-human-pause.playwright.ts), [`leaders/s04_login_human_pause_playwright.py`](leaders/s04_login_human_pause_playwright.py) (the sync API, which Playwright's Python docs show first), [`../typescript/scenarios/04-login-human-pause.ts`](../typescript/scenarios/04-login-human-pause.ts), [`../python/scenarios/s04_login_human_pause.py`](../python/scenarios/s04_login_human_pause.py) (the sync client, matching the leader; Ctrl+C cancels the call in flight, ADR 0081), [`../rust/examples/s04_login_human_pause.rs`](../rust/examples/s04_login_human_pause.rs). The login page's "Password" label points at the username field (`<label for="username">Password</label>`, checked 2026-10-15), so every program finds the password field by its id: `getByLabel("Password")` would fill the username field.

The Playwright programs cannot meet one requirement of the task: they read the password from the environment and hold it in clear to pass it to `fill`. With `page.pause()` the person checks the form, presses Login, then Resume in the Playwright Inspector; the pause has no condition, so the program checks for the Logout link afterwards.

**Concepts counted** (each name once per program; obligations last):

- Playwright (TypeScript), 21: `chromium`, `launch`, `headless`, `newContext`, `newPage`, `goto`, `getByLabel`, `fill`, `locator`, `pause`, `getByRole`, `"link"`, `name`, `waitFor`, `storageState` (the method and the option), `path`, `or`, `isVisible`, `close`; must close the browser, must await. The Inspector's Resume button is a further thing the person must know, not counted.
- Playwright (Python), 22: `sync_playwright`, `chromium`, `launch`, `headless`, `new_context`, `new_page`, `goto`, `get_by_label`, `fill`, `locator`, `pause`, `get_by_role`, `"link"`, `name`, `wait_for`, `storage_state`, `path`, `or_`, `is_visible`, `close`; must close the browser, must leave `sync_playwright()`.
- Goodfetch TypeScript, 41: `Collie`, `LoggedOutError`, `secret`, `connect`, `start`, `settings`, `secrets`, `reference`, `domains`, the `env:` reference form, `newSession`, `browser`, `headless`, `newPage`, `goto`, `getByLabel`, `fill`, `locator`, `pause`, `reason`, `until`, `timeout`, `signal`, `getByRole`, `"link"`, `name`, `saveIdentity`, `finish`, `source`, `kind`, `"saved_identity"`, `session`, `pageCheck`, `loginPage`, `selector`, `recovery`, `humanPause`, `agent`, `waitFor`; must close (`await using`), must await. `AbortController` and `process` are the platform's.
- Goodfetch Python, 39: the TypeScript names in snake_case, without `signal` (Ctrl+C cancels); must close (`with`).
- Goodfetch Rust, 52: `settings::browser`, `settings::session`, `HEADLESS`, `PAGE_CHECK`, `recovery`, `HUMAN_PAUSE`, `AGENT`, `AriaRole`, `Link`, `Collie`, `ConnectOptions`, `scope`, `ElementState`, `Visible`, `Error`, `ErrorCode`, `LOGGED_OUT`, `code`, `PageCheck`, `new`, `login_page`, `PageMatch`, `selector`, `SecretBinding`, `reference`, the `env:` reference form, `SessionSource`, `saved_identity`, `Settings`, `secret` (the function and the builder method), `set`, `start`, `settings`, `new_session`, `source`, `new_page`, `goto`, `get_by_label`, `fill`, `locator`, `pause`, `reason`, `until`, `no_timeout`, `get_by_role`, `name`, `wait_for`, `save_identity`, `finish`; must await (builders), must close (`scope`), must use the Run report (`#[must_use]`). `IntoFuture`, `FromStr` and Tokio's `select!` and `ctrl_c` are the standard library's and the executor's.

**Failure points, scored**

| Failure point | Playwright (TypeScript) | Playwright (Python) | Goodfetch (all three) |
|---|---|---|---|
| 1. The person never finishes, or closes the window | generic: the pause waits without limit; Ctrl+C or a closed window ends it with a "Target page, context or browser has been closed" `Error`; Resume pressed before Login is the Logout check's `TimeoutError` | generic: as TypeScript | typed: waits without limit; the program cancels it (`signal`, Ctrl+C, a dropped future), a person's Decline is `HumanPauseDeclinedError` with their note, and "Done" counts only once the Logout link shows (`until`). A closed window or a quit browser ends it at once with `CallCancelledError`, whose details say `by: "browser"` and `closed: "page"` (or `"browser"`); a lost Live view ends nothing (ADR 0113) |
| 2. The kept state expired or was revoked | typed: the program's own check finds the Login link and returns `expired` | typed: as TypeScript | typed: the Page check finds the Login link and `goto` throws `LoggedOutError`, listing each recovery step skipped, which the program returns as `expired` |
| 3. The secret is missing | generic: the program's own `Error` ("Set QUOTES_PASSWORD") | generic: `KeyError: 'QUOTES_PASSWORD'` | typed: `SecretUnavailableError` from `fill`, with `failure: "missing"`, the reference and its provider |
| 4. The password reaches a log, trace, screenshot or the kept file | silent: the program holds it in clear and nothing masks it; a trace of this run keeps the login POST body in its network log, password included (Playwright's tracing reference) | silent: as TypeScript | typed (prevented): the program never holds it, and the engine masks it by value as `%password%` in logs, the Run trace, screenshots and returned values (ADR 0092) |
| 5. The kept state is readable by other users | silent: `storageState` writes cookies as clear-text JSON with the process's umask | silent: as TypeScript | typed (prevented): a Saved identity, encrypted in the install's store, never a clear-text file (ADRs 0035, 0056); no SDK writes a `storageState` |
| 6. The headless run looks like another browser | silent: headless Chromium sends another user agent (`HeadlessChrome`) than the headful run that logged in, and nothing reports it | silent: as TypeScript | typed (prevented): the `standard` Access mode presents one real browser's identity, headless or not, with no `HeadlessChrome` token (ADR 0021), and the Saved identity keeps the Session's identity; a site that still refuses raises `BlockedError` or `LoggedOutError` |

"Typed (prevented)" counts as typed: the failure cannot happen through the API, and the leader's program has no way to make it typed.

**Verdict.** Goodfetch's programs are longer (59 and 58 lines against 42 and 39) and need about twice the concepts (41 and 39 against 21 and 22). The extra names are the Secret placeholder's binding (`secrets`, `reference`, `domains`, `secret`), the Page check and the unattended recovery switches (`pageCheck`, `loginPage`, `recovery`, `humanPause`, `agent`), and `finish()`: they keep the password out of the program and of everything kept (G5), make an expired login a typed outcome instead of a guess from a link (G1, G6), and say that no person is there to log in again (G5). The leader's shorter programs leave three failure points silent and cannot keep the password out of the program at all. Rust's 86 lines come from builders, `scope` closures, `tokio::select!` for Ctrl+C and its own outcome enum (S1); it has no scored leader.

**Raised while writing it:** [Ending a Human pause by closing the window](https://github.com/goodfetch/goodfetch/issues/532), answered by ADR 0113: failure point 1's closed window is typed on `CallCancelledError`'s details, not on its wording.

## After the sketch

A new Journey on the SDK surfaces, ending in a Saved identity; Proving ground: a seeded login (ADR 0063).
