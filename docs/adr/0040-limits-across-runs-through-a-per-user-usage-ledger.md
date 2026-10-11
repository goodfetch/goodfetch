# Limits across runs are kept in a per-user Usage ledger, and only a person raises a Limit

Before this ADR, Limits applied to one run (ADR 0012) or to a Schedule's runs over a period (ADR 0036). An agent that starts fifty runs through MCP, or a script that loops over `runs.start`, had no total cap. That is pain #5 (`cost-unpredictable-runaway`): a Cloud Run scraper with a $7 budget that only sent alerts ran up about $72,000 in two hours ([The Register](https://www.theregister.com/2020/12/10/google_cloud_over_run/)). Limits can now also attach to the install or Daemon, to a person, to a named token and to a Client connection. Every engine on the machine enforces them through one per-user Usage ledger. Decided in [Engine resource governance](https://github.com/Jbmanllr/automan/issues/115).

Comparable services cap spend in different ways:

- **Per key, with a period:** OpenRouter ([keys](https://openrouter.ai/docs/guides/overview/auth/provisioning-api-keys)), Firecrawl ([billing](https://docs.firecrawl.dev/billing)) and LiteLLM. LiteLLM also caps per user, team and tag, and reserves each request's estimated cost before sending it ([budgets](https://docs.litellm.ai/docs/proxy/users)).
- **Per project:** OpenAI's hard limit, whose enforcement "is not instantaneous" ([spend limits](https://developers.openai.com/api/docs/guides/spend-limits)).
- **Per run only:** Apify, so concurrent runs add up ([pay per event](https://docs.apify.com/platform/actors/publishing/monetize/pay-per-event)).
- **No cap:** Browserbase ([plans](https://docs.browserbase.com/account/billing/plans.md)).

## Considered Options

- **Run and Schedule Limits only** (Apify): rejected. Concurrent and unscheduled runs add up with no total, which is pain #5.
- **Enforcement by the Daemon alone**: rejected. A script with an Attached engine (ADR 0024) never reaches the Daemon, so it would escape every cap.
- **Reserving the whole remaining Limit when a run starts**, as ADR 0036 does for a Schedule: rejected for the new scopes. On a Daemon-wide Limit, the first run would take everything left and a concurrent one would get nothing. ADR 0036 keeps its clamp, because a Schedule's runs rarely overlap.
- **Checking the total, then charging after the fact**: rejected. Concurrent requests pass the check together and overshoot, as LiteLLM's open bugs show ([#40095](https://github.com/BerriAI/litellm/issues/40095)).
- **A default amount**: rejected. Goodfetch knows neither the user's prices nor their volumes, and an arbitrary cap would stop production jobs without warning.
- **Letting any Client with the right token raise a Limit**: rejected. An agent stopped by a cap would lift it itself, which defeats the cap.

## Consequences

**Scopes.**
- **Where a Limit attaches:**
  - a run (ADR 0012);
  - a Schedule's period (ADR 0036);
  - the install or a Daemon, set in Operator policy;
  - a person on a shared Daemon (ADR 0031);
  - a named token;
  - a Client connection, such as an MCP session or an SDK's connection, for as long as the connection lasts.
- **Periods.** Every scope except a run and a Client connection may count per day, week or month, in an explicit IANA time zone.
- **What a Limit can cap.** Any kind of Usage: currency, model tokens and calls, browser time, proxy bytes by kind, and paid solves (ADR 0035).
- **How they combine.** A run counts against every scope that applies to it, and the tightest one holds. Operator policy can cap every scope's Limits.

**The Usage ledger.**
- **What it is.** A per-user SQLite file kept beside the Run index (ADR 0031). Every engine of that user on the machine reserves against it: Attached engines, the Daemon, and engines embedded through the Rust crate.
- **Outside the majors.** It sits outside the per-major folders, like ADR 0036's active-major marker, so running an older major never escapes a cap. Its format is a contract format versioned under ADR 0030, and every supported major reads it.
- **One machine.** Caps across several machines belong to Goodfetch Cloud. Its gateway keeps its own ledger per Tenant, with these scopes and semantics, behind the same operations (ADR 0001).

**Reservation by slices.**
- **Leasing.** For each scope this ADR adds, a run leases a slice: by default 1 % of that scope's Limit, settable. It leases another when the slice is used up, and returns what is left when it ends. Each lease is one transaction on the ledger, so concurrent engines can never take the same Usage twice.
- **Money.** A model call or paid solve still reserves its worst case before it is sent (ADR 0012), so a currency Limit is never exceeded.
- **Measured Usage.** Usage that is only known afterwards, such as browser time and proxy bytes, can go past a Limit by at most one slice per live run.

**When a Limit runs out.**
- **New runs.** They are refused before they start, with the Error code `limit_reached`, which names the scope and its period, so an agent can tell its user why.
- **Live runs.** A live run that cannot lease a new slice stops at its next safe point. Its Lifecycle is `stopped` and its Completeness `incomplete` (ADR 0029). It can be resumed once the period resets or the Limit is raised.
- **Schedules.** Their occurrences are recorded as *skipped: Limit*, as in ADR 0036.
- **Alerts.** A Notification goes out at 50 %, 80 % and 100 % of each Limit (all settable), and a "Needs you" item at 100 %. ADR 0027's delivery ledger keys them by scope and period, so none repeats.
- **Visibility.** The Console's Engine & settings screen and `goodfetch usage` show Usage against each Limit, by scope.

**No default amount, but uncapped runs are marked.**
- **The mark.** A run that can spend money (model calls, paid solves, metered proxies) with no currency Limit in any scope is marked *uncapped* in its result, in the Console and in the CLI. This applies ADR 0023's rule that results say what was left off.
- **First run.** `goodfetch init` and the Console's first launch offer to set a monthly Limit for the install.

**Who sets and raises a Limit** (changes ADRs 0012 and 0036).
- **Setting.** Each scope is set by its owner, within the scopes above it:
  - install or Daemon: the operator, through Operator policy;
  - person: the operator;
  - named token: whoever creates the token, within their own Limits;
  - Client connection: the Client when it connects, within its token's Limits;
  - run: whoever starts it.
- **Lowering.** Any Client may lower a Limit it is under.
- **Raising.** Raising a Limit that already applies, including raising a run's Limit to resume it, is answered only in the Console or an interactive CLI prompt. ADR 0029 restricts Reconciliation the same way, and ADR 0031's step-up asks for a passkey on a Daemon with accounts. It is offered on no other surface generated from the catalog: not MCP, code mode, the SDKs, agent skills, framework adapters, Integrations, nor the CLI when it is not interactive or runs with `--json`.
