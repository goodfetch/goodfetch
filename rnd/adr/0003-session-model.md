# ADR-003: long authenticated session model

**Status: proposed, based on the lab's mechanical verdict; R&D draft.**
**Date: October 9, 2026.**
**Final decision: user / project owner.**
**Registry decision: `D-session-model`; verdict `lab/verdicts/D-session-model.json`; report `lab/reports/portal-2026-10-09.md`.**

## Context

The B2B portals segment relies on long authenticated sessions: a login with a CSRF token, sessions that expire, refresh tokens, runs interrupted then resumed. The architecture already sets several rules. The session carries an identity, cookies, an owning backend and a lease ([architecture §2.1](../proposals/architecture.md:59)), and the session pool manages expiration, refresh and quarantine ([§4.1](../proposals/architecture.md:143)). Expired sessions are not blindly reused ([§2.2](../proposals/architecture.md:83)). A policy distinguishes expired authentication from other causes, under a single attempt budget ([§3.4](../proposals/architecture.md:125)). ADR-002 imposes a single identity profile per session, across HTTP and browser.

What remained to be decided was how the engine detects session loss and what it keeps from one run to the next. The lab's `portal` lane compared three policies on five situations, over HTTP and in the browser, against a site whose oracle counts records and logins (`portal-sessions-2026-10-09T15-26-01-462Z-3f0b51`, 5 blocks).

## Decision

1. **Each page of a session is judged against the recipe's contract, never on the HTTP status alone.**
   - The expected page (the authenticated listing) is a success.
   - A recognized login form is a lost session.
   - Any other page stops the task with `E_CONTRACT`.
2. **The session state is part of the run journal.** Cookies, cursor and already validated records are written to it, and a new process or a new browser context resumes them. The resumed state is revalidated on the first page. If it is refused, through revocation or expiration, the engine logs in again once.
3. **The recipe declares session recovery**: refresh endpoint and cookie, then login with a credentials reference. Recovery consumes the common attempt budget; at most two recoveries per page.
4. Importing and exporting a session remain operations distinct from exporting the plan. The journal's cookies have the session's tenant and identity scope ([§8.3](../proposals/architecture.md:318)).

## Evidence

| Policy | False successes | Logins per resumed run | Verdict (logins, against `relogin-each-run`) |
| --- | --- | --- | --- |
| `contract` (journal + page contract) | 0 | 1 | **better**, ratio 0.50, 5/5 blocks, over HTTP and in the browser |
| `relogin-each-run` (contract, without resumed state) | 0 | 2 | reference |
| `status-only` (resumed state, HTTP status) | 20/20 on the expiration served as `200` | 1 | **blocked by the contract** (`oracle-exact`) |

- When the portal responds `200` with its login form at the URL of a report, `status-only` reads an empty page and declares the crawl finished: 20 or 40 records are missing depending on the tier.
- Resuming the journaled state avoids a login on every restart. A session revoked during the stop is detected on the first page and costs a single login, without loss or duplicate.
- Logins are counted by the site and do not depend on the host: the decision can be made in `local-dev`. The durations, which are indicative, are not used for the verdict.

## Consequences

- The recipe schema carries a **session page contract**, an authentication marker and the expected shape of the listing, as well as a **recovery policy**.
- A run's journal stores the session state encrypted at rest, with the same retention and the same rights as the traces ([§8.3](../proposals/architecture.md:318)).
- The documentation shows this case as an example: a portal that returns `200` on expiration is the most common trap.

## Limitations

- Synthetic fixtures. MFA, SSO and device-bound tokens are not covered; they fall under a resumption with human intervention (`needs_review`).
- A single account per session. Account rotation is not addressed here.
