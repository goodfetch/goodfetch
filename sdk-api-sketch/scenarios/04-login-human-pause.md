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

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Playwright (TypeScript) | | | | |
| Playwright (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces, ending in a Saved identity; Proving ground: a seeded login (ADR 0063).
