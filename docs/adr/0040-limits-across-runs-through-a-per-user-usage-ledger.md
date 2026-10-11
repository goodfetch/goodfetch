# Limits across runs are kept in a per-user Usage ledger, and only a person raises a Limit

Before this ADR, Limits applied to one run (ADR 0012) or to a Schedule's runs over a period (ADR 0036). An agent that starts fifty runs through MCP, or a script that loops over `runs.start`, had no total cap. That is pain #5 (`cost-unpredictable-runaway`): a Cloud Run scraper with a $7 budget that only sent alerts ran up about $72,000 in two hours ([The Register](https://www.theregister.com/2020/12/10/google_cloud_over_run/)). Limits can now also attach to the install or Daemon, to a person, to a named token and to a Client connection. Every engine on the machine enforces them through one per-user Usage ledger. Decided in [Engine resource governance](https://github.com/Jbmanllr/automan/issues/115).

Comparable services cap spend in different ways:

- **Per key, with a period:** OpenRouter ([keys](https://openrouter.ai/docs/guides/overview/auth/management-api-keys)), Firecrawl ([billing](https://docs.firecrawl.dev/billing)) and LiteLLM. LiteLLM also caps per user, team and tag, and reserves each request's estimated cost before sending it ([budgets](https://docs.litellm.ai/docs/proxy/users)).
- **Per project:** OpenAI's hard limit, whose enforcement "is not instantaneous" ([spend limits](https://developers.openai.com/api/docs/guides/spend-limits)).
- **Per run only:** Apify, so concurrent runs add up ([pay per event](https://docs.apify.com/platform/actors/publishing/monetize/pay-per-event)).
- **No cap:** Browserbase ([plans](https://docs.browserbase.com/account/billing/plans.md)).

## Considered Options

- **Run and Schedule Limits only** (Apify): rejected. Concurrent and unscheduled runs add up with no total, which is pain #5.
- **Enforcement by the Daemon alone**: rejected. A script with an Attached engine (ADR 0024) never reaches the Daemon, so it would escape every cap.
- **A ledger per product major**, like the Run index and the Operator policy: rejected. Running an older major's binary would escape every cap set under a newer one.
- **Reserving the whole remaining Limit when a run starts**, as ADR 0036 does for a Schedule: rejected for the new scopes. On a Daemon-wide Limit, the first run would take everything left and a concurrent one would get nothing. ADR 0036 keeps its clamp, because a Schedule's runs rarely overlap.
- **Checking the total, then charging after the fact**: rejected. Concurrent requests pass the check together and overshoot, as LiteLLM's open bugs show ([#40095](https://github.com/BerriAI/litellm/issues/40095)).
- **A default amount**: rejected. Goodfetch knows neither the user's prices nor their volumes, and an arbitrary cap would stop production runs without warning.
- **Letting any Client with the right token raise a Limit**: rejected. An agent stopped by a cap would lift it itself, which defeats the cap.

## Consequences

**Scopes.**
- **Where a Limit attaches:**
  - a run (ADR 0012);
  - a Schedule's period (ADR 0036);
  - the install: every run of the user on the machine, whichever engine runs it;
  - a Daemon;
  - a person on a shared Daemon (ADR 0031);
  - a named token;
  - a Client connection, such as an MCP session or an SDK's connection, for as long as the connection lasts.
- **Periods.** Every scope except a run and a Client connection may count per day, week or month, in an explicit IANA time zone.
- **What a Limit can cap.** Any kind of Usage: currency, model tokens and calls, browser time, proxy bytes by kind, and paid solves (ADR 0035).
- **How they combine.** A run counts against every scope that applies to it, and the tightest one holds. A scheduled run counts against the scopes of its Schedule's creator. An Operator policy can lower any of these Limits for the engines of its major, never raise them.

**The Usage ledger.**
- **What it is.** A per-user SQLite file. Every engine of that user on the machine reserves against it: Attached engines, the Daemon, and engines embedded through the Rust crate.
- **What it holds.** The amounts and periods of the install, Daemon, person, named token and Client connection Limits, the leases taken against them, the Usage charged, and the scope-level Notifications and "Needs you" items below. The Console reads it beside the Run index.
- **Outside the majors.** It lives in a per-user folder outside the per-major folders, like ADR 0036's active-major marker, so running an older major's binary never escapes a cap. A person on a shared Daemon and a named token exist only on the Daemon of one major (ADR 0031), so their Limits apply on that Daemon.
- **Its format** (changes ADR 0030). The ledger is a contract format whose major stays frozen across product majors. A new version may only add tables and columns, which older engines ignore rather than refuse. A change that cannot be made that way needs a new ledger beside the old one, read by both.
- **One machine.** Caps across several machines belong to Goodfetch Cloud. Its gateway keeps its own ledger per Tenant, with these scopes and semantics, behind the same operations (ADR 0001).

**Reservation by slices.**
- **Leasing.** For each scope this ADR adds, a run leases a slice and leases another when it is used up. Each lease is one transaction on the ledger, so concurrent engines can never take the same Usage twice.
- **Slice size.** The smaller of 1 % of the scope's Limit and the remainder divided by twice the number of live runs in that scope, both settable. Slices therefore shrink as the remainder falls, and no run holds back what others could use.
- **Money.** A model call or paid solve still reserves its worst case before it is sent (ADR 0012). When the worst case exceeds the current slice, the run leases enough in the same transaction, or the call is refused. A currency Limit is therefore never exceeded.
- **Measured Usage.** Usage known only afterwards, such as browser time and proxy bytes, is charged against the run's slice. A run whose slice runs out either leases another or stops at its next safe point, so it can go past a Limit only by what runs consume before then, as in ADR 0036.
- **Returning and reclaiming.** A run returns what is left of its slices when it ends.
  - Each lease names its run and its engine, and every engine holds an OS lock while it lives (ADR 0031's lock-file test).
  - An engine that opens the ledger and finds a lease whose engine's lock is free reclaims it. It reads the run's journaled Usage from its run state and charges that in place of the lease.
  - When the run state cannot be read, the lease stays charged in full and is marked for review.

**When a Limit runs out.**
- **New runs.** They are refused before they start, with the Error code `limit_reached`, which names the scope and its period, so an agent can tell its user why.
- **Live runs.** A live run that cannot lease a new slice stops at its next safe point. Its Lifecycle is `stopped` and its Completeness `incomplete` (ADR 0029). It can be resumed once the period resets or the Limit is raised.
- **Schedules.** Their occurrences are recorded as *skipped: Limit*, as in ADR 0036.
- **Notifications.** A Notification goes out at 50 %, 80 % and 100 % of each Limit (all settable), and a "Needs you" item at 100 %.
  - **No duplicates.** The crossing is recorded in the ledger in the same transaction as the lease that crosses it, and only the engine that wrote it sends the Notification. Several engines crossing a threshold together therefore send it once.
  - **Where the item lives.** The "Needs you" item is kept in the ledger, and the Console lists it beside run items (ADR 0031).
- **Visibility.** The Console's Engine & settings screen and `goodfetch usage` show Usage against each Limit, by scope.

**No default amount, but uncapped runs are marked.**
- **The mark.** A run that can spend money (model calls, paid solves, metered proxies) with no currency Limit in any scope is marked *uncapped* in its result, in the Console and in the CLI. This applies ADR 0023's rule that results say what was left off.
- **First run.** `goodfetch init` and the Console's first launch offer to set a monthly Limit for the install.

**Who sets and raises a Limit** (changes ADRs 0012 and 0036).
- **Setting.** Each scope is set by its owner, within the scopes above it:
  - install: a person, in the Console or an interactive CLI prompt;
  - Daemon and person: the operator;
  - named token: whoever creates the token, within their own Limits;
  - Client connection: the Client when it connects, within its token's Limits;
  - run: whoever starts it.
- **Lowering.** Any Client may lower a Limit it is under, and the change applies at once.
- **Raising.** Raising a Limit that already applies is answered only in the Console or an interactive CLI prompt. That covers raising a run's Limit to resume it and raising a Schedule's period Limit. ADR 0029 restricts Reconciliation the same way, and ADR 0031's step-up asks for a passkey on a Daemon with accounts. It is offered on no other surface generated from the catalog: not MCP, code mode, the SDKs, agent skills, framework adapters, Integrations, nor the CLI when it is not interactive or runs with `--json`.
- **Schedules** (changes ADR 0036). A `schedules.edit` from another surface, or a `goodfetch schedules apply` of a Schedule file, that raises a period Limit is held as a "Needs you" item until a person confirms it. Until then, the previous Limit applies; the rest of the change applies at once.
- **What it does not prove.** As ADR 0029 says of Reconciliation, this is a policy boundary, not a proof of humanity. A program with the user's own rights can still edit local files or start a new run with a higher run Limit where no wider scope holds it. The install Limit is the cap such a program cannot raise through any Goodfetch surface.
