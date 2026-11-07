# Limits across runs are kept in a per-user Usage ledger, and only a person raises a Limit

Before this ADR, Limits applied to one run (ADR 0012) or to a Schedule's runs over a period (ADR 0036). An agent that starts fifty runs through MCP, or a script that loops over `runs.start`, had no total cap. That is pain #5 (`cost-unpredictable-runaway`): a Cloud Run scraper with a $7 budget that only sent alerts ran up about $72,000 in two hours ([The Register](https://www.theregister.com/2020/12/10/google_cloud_over_run/)). Limits can now also attach to the install or Daemon, to a person, to a named token and to a Client connection. Every engine on the machine enforces them through one per-user Usage ledger. Decided in [Engine resource governance](https://github.com/goodfetch/goodfetch/issues/115).

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
  - the install: every run of the user on the machine, whichever engine runs it, or in a Daemon group on every Group member's machine (ADR 0055);
  - a Daemon, which in a Daemon group is the whole group (ADR 0055);
  - a Partitioned crawl (ADR 0054);
  - a person on a shared Daemon (ADR 0031);
  - a named token;
  - a Client connection, such as an MCP session or an SDK's connection, for as long as the connection lasts.
- **Periods.** Every scope except a run and a Client connection may count per day, week or month, in an explicit IANA time zone.
- **What a Limit can cap.** A kind of Usage (model tokens and calls, browser time, proxy bytes by kind, paid solves under ADR 0035), or its price in a currency through ADR 0012's price table.
- **Paid fetches.** The Usage kind `paid_fetch` counts Paid fetches and their amounts by currency, reserved like paid solves. Each Limit row records who set it (a person interactively, a managed policy, or a Client), and a Client that lowers a person's Limit adds its own row, because only a Limit a Client could not set allows paying; run and Schedule Limits never do (ADR 0045).
- **How they combine.** A run counts against every scope that applies to it, and the tightest one holds. A scheduled run counts against the scopes of its Schedule's creator.

**Limits across runs are not settings** (changes ADR 0028).
- **Where they live.** The install, Daemon, person, named token and Client connection Limits are kept in the Usage ledger. They are not resolved through ADR 0028's five layers, so `goodfetch config explain` does not show them; `goodfetch usage` does.
- **Operator policy.** An Operator policy may still cap them for the engines of its own major: the stricter of the policy's cap and the ledger's Limit holds. A cap kept only in a per-major policy is escaped by another major, so the operator's Daemon and person Limits are written to the ledger, through the Console with ADR 0031's step-up or an interactive CLI prompt. ADR 0036's operator caps on Schedules stay in Operator policy; a person's Limit is a ledger Limit.
- **Managed machines.** A *managed* Operator policy (ADR 0031) may name an install Limit, so a fleet deployed by MDM or Ansible gets its cap without a person at each machine. Each engine writes it to the ledger when it starts, as a row of its own keyed by the policy's major and marked managed. Only that policy changes that row, apart from the upgrade below, and an older major still in use writes its own row back when it starts; the tightest row holds, so a lower Limit set by anyone else is a separate row and never edits it. `goodfetch upgrade-files` replaces older majors' managed rows with the new major's, and rows of majors no longer supported (ADR 0030) are dropped, so an uninstalled major's cap does not linger.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#232](https://github.com/goodfetch/goodfetch/issues/232): a policy is *managed* when its Operator policy file has a `[managed]` table, whose `install_limits` is an array of `{ usage_kind, currency, period, amount }`. Each engine writes them when its registry starts, before it reclaims, as rows `set_by = "managed_policy"` with the column `managed_major` naming its major: a Limit of the same Usage kind, currency and period keeps its row and its Usage and takes the new amount; one the policy no longer names, or a policy no longer managed, drops that major's row and its Slices. `upgrade-files` gives the new major the source major's managed rows (unless it has its own) and drops every other older major's; `usage list` names each managed row's `managed_major`.
- **Containers.** In a container, the ledger needs a persistent volume. Otherwise period Limits restart whenever the container does, and the CLI warns when the ledger sits on a temporary filesystem.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#232](https://github.com/goodfetch/goodfetch/issues/232): the warning comes from the `usage` commands (the generated `usage list` and `usage set`, and `usage raise`), when the deepest mount above the ledger's path in `/proc/self/mounts` is a `tmpfs` or `ramfs`; elsewhere than Linux no temporary filesystem is found. `daemon run` does not warn yet.

**The Usage ledger.**
- **What it is.** A per-user SQLite file. Every engine of that user on the machine reserves against it: Attached engines, the Daemon, and engines embedded through the Rust crate.
- **What it holds.**
  - The Limits of the scopes above, as generic rows: scope kind, scope ID, Usage kind or currency, period and amount.
  - The slices taken against them, and the Usage charged.
  - The scope-level Notifications and "Needs you" items below.

  The Console reads it beside the Run index.
  > Note (agent decision, pending owner review), 2026-11-05, [#2110](https://github.com/goodfetch/goodfetch/issues/2110): a read of the Limits (a run's start, each charge, its end) leaves a ledger no engine created uncreated and finds no Limit in it, so a run that no Limit holds writes and syncs none of the ledger; the next read tries again, so a Limit set meanwhile holds the next charge. A run that a Limit holds syncs the ledger once per Slice it takes, in that Slice's transaction as above, and once as it gives them back. A connection no longer copies the write-ahead log into the file as it closes (`SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE`), which synced both again: each commit at `synchronous=FULL` already synced the log, so a Slice taken or given back survives a power cut once its call answered, as before.
- **Outside the majors.** It lives in a per-user folder outside the per-major folders, like ADR 0036's active-major marker, so running an older major's binary never escapes a cap. A person on a shared Daemon and a named token exist only on the Daemon, or the Daemon group, of one major (ADRs 0031 and 0055), so their Limits apply there.
- **Its format** (changes ADR 0030). The ledger is a contract format, added to ADR 0030's list. Its major stays frozen across product majors, so every supported major opens it.
  - **New versions.** A new version adds rows, tables and columns.
  - **Fails closed.** Each Limit row carries a *must understand* flag. An engine that finds a flagged Limit whose scope kind, Usage kind, currency or period it cannot evaluate refuses to start runs it cannot place outside that Limit. The refusal uses the Error code `limit_not_understood`, which names the Limit and asks for an upgrade. An older engine therefore never runs past a Limit it cannot read. A live run on such an engine that meets a new flagged Limit when it takes its next slice stops at its next safe point with the same Error code.
  - **Breaking changes.** A change that cannot be made by adding goes to a second ledger. Every engine that knows it keeps both up to date, so older engines still enforce what they can read.
- **One machine or one Daemon group.** In a Daemon group, the Lead Daemon keeps the ledger, and on each Group member's machine every engine of the user, Attached engines included, takes its slices there, so the install, Daemon, person, named token and Client connection Limits hold across the group, and a Partitioned crawl adds its own scope; a slice is reclaimed by the Group member that owns or adopts its run, which reports the charge to the Lead Daemon (ADRs 0054 and 0055). Goodfetch Cloud's gateway keeps its own ledger per Tenant, with these scopes and semantics, behind the same operations (ADR 0001).

**Reservation by slices.** "Slice" is used here so as not to collide with the work-item lease of ADR 0003.
- **Taking a slice.** For each scope this ADR adds, a run takes a slice and takes another when it is used up. Each slice is taken in one transaction on the ledger, so concurrent engines can never take the same Usage twice.
- **Slice size.** The smaller of 1 % of the scope's Limit and the remainder divided by twice the number of live runs in that scope, both settable. Slices therefore shrink as the remainder falls, and no run holds back what others could use. A settable minimum slice keeps the number of ledger transactions bounded: a remainder smaller than it goes whole to the run that asks.
- **Reserved costs.** A model call or paid solve reserves its worst case before it is sent (ADR 0012). When the worst case exceeds the current slice, the run takes enough in the same transaction, or the call is refused. The reservation is journaled in the run before the call is sent. A currency Limit is therefore never exceeded by reserved costs.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#232](https://github.com/goodfetch/goodfetch/issues/232): the interface is `Run::reserve(usage_kind, currency, worst)`, which answers a reservation, and `Run::settle(reservation, cost)`, which journals what the worst case did not cost; a currency Limit holds only reservations in its own currency. A reservation still open when the run ends, or when a reclaimer finds it, stays charged at its worst case. The slices model adds the reservation to `take_slice` (a Slice at least the worst case, or none) and a `settle` transition. `Run::allowed_by(usage_kind, currency)` answers who allows paying (ADR 0045): the setter of the tightest Limit holding the run that a person or a managed policy set, or none.
  > Note (agent decision, pending owner review), 2026-10-28, step 7 review (Standards, Glossary): `Reservation`, the name the note above gives `Run::reserve`'s answer and the ledger's type, is a GLOSSARY.md entry of its own, the worst case a run takes from its Slices and settles at what it cost. GLOSSARY.md listed "reservation" among the words to avoid for a Slice, which the entry keeps ("lease, reservation (both for a Slice)"), so the code keeps the word this ADR's accepted interface uses instead of renaming it.
- **Measured Usage.** Usage known only afterwards, such as browser time and proxy bytes, is charged against the run's slice. A run whose slice runs out either takes another or stops at its next safe point. It can go past a Limit only by what runs consume before then, as in ADR 0036. A metered proxy's bytes count toward a currency Limit only when the price table (ADR 0012) has a price for that proxy, and then as measured Usage.
- **Returning and reclaiming.** A run returns what is left of its slices when it ends.
  - **What each slice records:** its run, the run epoch (ADR 0003) and the location of the run's state. The engine that writes a run holds the run's writer lock (ADR 0031).
  - **Reclaiming.** An engine that opens the ledger reclaims a slice whose run's writer lock is free, or held at a later epoch: either way, the engine that took it is gone. In a Daemon group, the Group member that owns or adopts the run reclaims it and reports the charge to the Lead Daemon (ADR 0055). It reads the run's state and charges the Usage journaled under that epoch in place of the slice. A reservation still open in the run's state, such as a model call sent just before the crash, is charged at its worst case.
  - **Unreadable state.** When the run state cannot be read, the slice stays charged in full and is marked for review.
  - **Unsent Notifications.** The reclaiming engine also sends any threshold Notification the dead engine recorded but never marked sent.

**When a Limit runs out.**
- **New runs.** They are refused before they start, with the Error code `limit_reached`, which names the scope and its period, so an agent can tell its user why. `limit_reached` and `limit_not_understood` are not retryable (ADR 0038): the run waits for a new period, a person or an upgrade.
- **Live runs.** A live run that cannot take a new slice stops at its next safe point. Its Lifecycle is `stopped` and its Completeness `incomplete` (ADR 0029). It can be resumed once the period resets or the Limit is raised.
  > Amended by [ADR 0107](0107-a-runs-model-allowance-refuses-its-model-calls-once-spent-while-the-run-goes-on.md): a Run may also hold a Model allowance (`ai.allowance.*`), which is not a Limit and is not kept in the Usage ledger: once reached, the Run's model calls are refused with `model_allowance_reached` while the Run goes on, and it leaves the uncapped mark as it is, since any layer may set or raise it.
- **Schedules.** Their occurrences are recorded as *skipped: Limit*, as in ADR 0036.
- **Notifications.** A Notification goes out at 50 %, 80 % and 100 % of each Limit (all settable), and a "Needs you" item at 100 %.
  - **No duplicates.** The crossing is recorded in the ledger in the same transaction as the slice that crosses it, and only the engine that wrote it sends the Notification, then marks it sent. Several engines crossing a threshold together therefore send it once.
  - **Where the item lives.** The "Needs you" item is kept in the ledger, and the Console lists it beside run items (ADR 0031).
- **Visibility.** The Console's Engine & settings screen and `goodfetch usage` show Usage against each Limit, by scope.

**No default amount, but uncapped runs are marked.**
- **The mark.** A run is marked *uncapped* in its result, in the Console and in the CLI, when it can spend money with nothing capping that spending:
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#232](https://github.com/goodfetch/goodfetch/issues/232): a run is marked when it reserves a cost in a currency no Limit holding it caps; the Run report carries `uncapped: true`, and run state keeps it. Metered proxies mark it once area 8 prices them.
  - model calls or paid solves with no currency Limit in any scope;
  - metered proxies with neither a proxy-bytes Limit nor a currency Limit and a price for that proxy.

  This applies ADR 0023's rule that results say what was left off.
- **First run.** `goodfetch init` and the Console's first launch offer to set a monthly Limit for the install.
  > Note (agent decision, pending owner review), 2026-10-27, [#830](https://github.com/goodfetch/goodfetch/issues/830): the Console makes its offer on its Overview while `usage.list` lists no install Limit: a monthly Limit on `paid_solves` in US dollars, 10 by default, its month counted in the browser's time zone, set through the person-only `usage.raise`. "Not now" is remembered in the browser (`localStorage` of the Console's origin, so per major), and the offer is not made again there; an install Limit set anywhere ends the offer on every browser.

**Who sets and raises a Limit** (changes ADRs 0012 and 0036, adds to ADR 0031).
- **Setting.** Each scope is set by its owner, within the scopes above it:
  - install: a person on a single-user machine, the operator on a Daemon with accounts, or a managed Operator policy;
  - Daemon and person: the operator, or the OS user on a single-user Daemon;
  - named token: whoever creates the token, within their own Limits;
  - Client connection: the Client when it connects, within its token's Limits;
  - run: whoever starts it.
- **Lowering.** A Client may lower, at once, the Limits that cover only its own work: its runs, its Client connection, its named token, on a Daemon with accounts its own person, and on a single-user machine the install and the Daemon, which cover only that user's work. Setting such a Limit where none applied counts as lowering, so CI, the OCI image and an agent can always cap their own work. Lowering or first setting a Limit shared with others (on a Daemon with accounts: the Daemon's, another person's or the install's) needs the role that sets it.
- **Raising.** Raising a Limit that already applies is answered only in the Console or an interactive CLI prompt. That covers raising a run's Limit to resume it and raising a Schedule's period Limit.
  - **Same rule as Reconciliation.** ADR 0029 restricts Reconciliation the same way. ADR 0031's step-up, which asks for a passkey on a Daemon with accounts, extends to raising a Limit.
  - **Not offered elsewhere.** It is offered on no other surface generated from the catalog: not MCP, code mode, the SDKs, agent skills, framework adapters, Integrations, nor the CLI when it is not interactive or runs with `--json`.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#232](https://github.com/goodfetch/goodfetch/issues/232): raising is the operation `usage.raise`, which the catalog marks *person only* (`Reach::PersonOnly`). It sets the person's amount in place of the rows of its scope, Usage kind, currency and period that no managed policy set, keeping their Usage. The engine refuses it with `person_required` (refused, not retryable) to a Connection whose `hello` names any surface but `cli` or `console`; the SDK generators and the CLI's generated commands leave it out, and the CLI offers it as `usage raise`, which asks at the terminal and refuses with `person_required` when stdin is no terminal. The Console's step-up is area 12's. `usage.set` takes no `scope_id` but the caller's own for the install, its person, its Daemon and its Client connection.
- **Schedules** (changes ADR 0036). A `schedules.edit` from another surface, or a `goodfetch schedules apply` of a Schedule file, that raises a period Limit is held as a "Needs you" item until a person confirms it. Until then, the previous Limit applies; the rest of the change applies at once.
- **What it does not prove.** As ADR 0029 says of Reconciliation, this is a policy boundary, not a proof of humanity. A program with the user's own rights can still get round it:
  - by editing local files;
  - by starting a new run with a higher run Limit where no wider scope holds it;
  - by reconnecting to get a fresh Client connection Limit;
  - by creating a new Schedule, or deleting one and creating it again, with a higher period Limit.

  The install Limit is the cap such a program cannot raise through any Goodfetch surface.
