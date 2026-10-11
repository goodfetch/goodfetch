# The Daemon schedules runs above the engine, and Goodfetch Cloud implements the same Schedule operations

The open-source framework schedules runs itself. A Schedule starts runs of one target (a Recipe or a code-free catalog run) on a time trigger, after a Watch finds a change, or after another run finishes. Schedules are kept by the Daemon, the only engine that outlives its Clients (ADR 0024), and are managed through `schedules.*` operations in the Operation catalog (ADR 0011), which Goodfetch Cloud's gateway implements with the same semantics (ADR 0031). The scheduler sits above the runs and starts them only through the public run operations. It passes each run's Limits and Baseline as ordinary parameters, so the engine executing a run never knows Schedules, as ADR 0001 requires. Crawlee leaves scheduling to cron or to Apify's platform ([Apify Schedules](https://docs.apify.com/platform/schedules)), and Firecrawl offers it only in its hosted API ([`/v2/monitor`](https://docs.firecrawl.dev/features/monitor)). Building it into the framework answers the monitoring use case (pain #3 in `rnd/research/market/pains-ranking.md`), scheduling and orchestration (#15) and self-host parity (#17). Decided in [Scheduling and recurring runs](https://github.com/Jbmanllr/automan/issues/109).

## Considered Options

- **No scheduler, with documented recipes for cron, systemd timers, Kubernetes CronJobs and workflow tools** (Crawlee, Scrapyd): rejected. Every user would solve overlaps, missed runs, time zones and spend across runs alone, and the Console could show neither upcoming runs nor why one did not happen. `goodfetch run` still works under any external scheduler.
- **Scheduling only in Goodfetch Cloud** (Firecrawl, Apify): rejected. It breaks ADR 0001's rule that the cloud does nothing that cannot be reproduced locally.
- **Scheduling inside the engine's run loop**: rejected. It would tie Schedules to one engine process, while Goodfetch Cloud runs one engine per run.
- **Several targets per Schedule** (up to 10 Actors and 10 tasks at Apify): rejected. Overlap, failure and history become ambiguous; two Schedules or an after-a-run trigger say the same thing.
- **Shell commands as targets** (cron, systemd, Kubernetes): rejected. The Daemon would run arbitrary programs with no Recipe, no recorded digest and no confinement, which matters on a shared Daemon.
- **Schedules only as Daemon objects** (Apify, Temporal, Firecrawl) or **only as files** (Kubernetes CronJobs, GitHub Actions): rejected. Teams want Schedules reviewed in pull requests, while a person in the Console or an agent through MCP wants one call. Both forms exist.
- **Concurrent runs by default** (Kubernetes `concurrencyPolicy: Allow`, Apify without `isExclusive`): rejected. Runs pile up when one outlasts its period ([oracle-base](https://oracle-base.com/blog/2023/07/14/when-overlapping-jobs-attack/)). Firecrawl (`skipped_overlap`), Zyte and Temporal skip by default.
- **Backfilling every missed occurrence** (Temporal backfill, Airflow `catchup`): rejected. A late run can only fetch the web as it is now, so ten catch-up runs would collect the same pages ten times.
- **Waking a sleeping machine** (systemd `WakeSystem=`, macOS `pmset schedule wake`): rejected. Both need root or polkit rights; catch-up on wake covers laptops, and servers do not sleep.
- **RRULE** (Browse AI): rejected. Cron plus anchored intervals cover what Schedules need and are what developers and agents already write.
- **An existing Rust scheduler crate**: rejected. tokio-cron-scheduler runs an hour off after a DST change until restarted ([#107](https://github.com/mvniekerk/tokio-cron-scheduler/issues/107)), and none documents overlap or catch-up. Goodfetch parses with `croner` (MIT), the only cron crate that documents its DST rules ([croner-rust](https://github.com/hexagon/croner-rust)), and writes the scheduling loop itself.
- **The previous run as the Baseline**: rejected. After one failed night every Record would look new, and a partial run would soft-delete every Record it did not reach (ADR 0026).
- **Pre-creating future runs** (Prefect creates up to 100, up to 100 days ahead, and recreates them on every edit; [Prefect](https://docs.prefect.io/v3/concepts/schedules)): rejected. Upcoming occurrences are computed when asked, as Temporal shows them.
- **Schedules inside the Run index**: rejected. `goodfetch runs reindex` rebuilds the index from runs (ADR 0031), and a Schedule cannot be rebuilt from its runs.

## Consequences

**Targets.**
- **One target per Schedule:** a Recipe or a code-free catalog run (a crawl, an extraction, a Plan replay, an agent task), with its inputs and the run's own settings (layer 5 of ADR 0028), within what the Recipe exposes.
- **Recipe versions.** A published Recipe is pinned by digest. A local Recipe is named by path and runs as it is on disk at each occurrence; the digest that ran is recorded on the run. An SDK script becomes a local Recipe, since the Daemon cannot call handlers in a process that is not running.
- **Operator policy** applies as it stands when each occurrence fires (ADR 0031).

**Where Schedules are defined.**
- **From any Client:** created and changed through `schedules.*` operations, so the SDKs, CLI, MCP server, Console and Integrations all have them. The CLI offers `goodfetch schedules list|show|create|edit|pause|resume|run|delete|apply|preview`.
- **From files:** a `schedules` section of project configuration, reconciled into the Daemon by `goodfetch schedules apply`. Such Schedules are marked *from file* and read-only in the Console, as managed policies are (ADR 0031). A Recipe manifest may suggest a default Schedule but never creates one.

**Triggers.**
- **Time:** cron (five fields plus optional seconds, `L`, `W`, `#` and the `@daily`-style macros, parsed by `croner`) or a fixed interval (`every 15m`) counted from the Schedule's anchor time, not from when it was saved, unlike n8n ([common issues](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.scheduletrigger/common-issues/)).
- **Watch, then run:** each occurrence first runs a Watch, a small run that checks chosen pages or values with a conditional request (`ETag`, `Last-Modified`) or a hash of pages or of an extracted field. The target run starts only if something changed. A Watch has Usage, Limits, Sessions and Access modes like any run. changedetection.io works this way.
- **After a run:** a Schedule fires when a run of another Schedule or Recipe finishes with a chosen outcome, verified by default. Loops between Schedules are refused.
- **Acting on change without a Watch** comes from change tracking against the Baseline, the "only new or changed" Sink mode (ADR 0026) and a new "records changed" Notification event, as Firecrawl's monitor and Browse AI do.
- **Inbound webhooks** need no trigger of their own: ADR 0027's HTTP listener already lets a workflow tool start a run.

**Time zones and DST.**
- Every Schedule stores an explicit IANA time zone, defaulting to the host's at creation and never stored as "local", so it means the same thing in Goodfetch Cloud.
- Cron occurrences follow `croner`'s rules: one in the skipped spring hour fires once, at the first valid time after the gap; one in the repeated autumn hour fires once. Intervals count elapsed time. A conformance test covers both transitions.
- Optional jitter, 0 by default, is the same delay at every occurrence of a Schedule (systemd's `FixedRandomDelay=`).
- No product minimum interval. Operator policy may set one, and Goodfetch Cloud sets its own.
- The Console and CLI preview the next occurrences in plain words before saving.

**Overlap.** When an occurrence fires while the Schedule's previous run is live, including one waiting in a Human pause, the Schedule's policy applies:
- `skip` (the default), recorded as *skipped: overlap*
- `queue_one`: at most one occurrence waits
- `allow`: runs concurrently, still under ADR 0004's per-domain admission
- `replace`: the live run checkpoints and stops at its next safe point (an Effect in flight finishes or is reported uncertain, ADR 0029), then the new run starts

**Missed occurrences.**
- An occurrence is missed when the Daemon was not running or the machine slept; the Daemon detects a sleep by the wall clock jumping.
- On start or wake, the latest missed occurrence runs once if it falls within the catch-up window: one period of the Schedule by default, settable, 0 meaning never. Every other one is recorded as *missed*. This is systemd's `Persistent=` and n8n 2.36's run-once with a grace period.
- **Changes ADR 0024:** a Daemon holding an enabled Schedule never stops idle. Creating the first Schedule on a Daemon not installed as a service offers `goodfetch daemon install`; until then, the Console and CLI warn that Schedules fire only while the Daemon runs.

**Limits across runs.**
- Each run keeps its own Limits. A Schedule may also carry period Limits: caps on the total Usage of its runs per day, week or month, counted in its time zone.
- Before a run starts, its Limits are clamped to what is left of the period. With the worst-case reservation of model calls (ADR 0012), the period total cannot overshoot.
- Once a period's Limit is used up, occurrences are recorded as *skipped: Limit*, one Notification goes out per period, and the Schedule resumes next period, as Firecrawl records `skipped_no_credits`.
- Operator policy can cap period Limits.

**Baseline and soft-delete.**
- A Schedule's runs compare against its latest run that ended verified and complete, unless the Schedule pins a Baseline. A run that ended `incomplete`, `unverified` or `rejected` never becomes the Baseline.
- **Changes ADR 0026:** only a complete run may soft-delete Records missing since its Baseline, because an incomplete one cannot tell removed from not reached.

**Repeated failures.** After 3 consecutive runs whose Lifecycle is `failed` or whose Verdict is `rejected` (settable, 0 meaning never), the Schedule pauses itself, sends a Notification and adds a "Needs you" item. Runs that end incomplete do not count. ADR 0026 auto-pauses failing webhook Sinks the same way.

**Unattended runs.**
- A scheduled run resolves Secret references in the Daemon's context (its environment, keychain and vault logins), and its data key is wrapped by the Daemon's keychain key (ADR 0015).
- A Schedule whose target uses the `caller` Secret provider is refused. An `env:` reference that the creating Client can resolve but the Daemon cannot raises a warning at creation. A reference that does not resolve when an occurrence fires fails the run before it starts, naming the reference.
- A Human pause in a scheduled run waits as usual, and later occurrences follow the overlap policy.

**The scheduler.**
- It is a component of the Daemon above the runs, and starts each run through `runs.start` like any Client.
- Each occurrence has an ID (the Schedule ID and the due time) written to the Schedule store before its run starts, so a scheduler restarted after a crash never starts it twice. n8n's durable scheduler takes the same approach ([durable scheduler](https://docs.n8n.io/deploy/host-n8n/configure-n8n/durable-scheduler.md)).
- The scheduler holds an exclusive OS lock on the Schedule store (ADR 0031's lock-file pattern), so a second Daemon on the same files cannot fire.

**Storage and the Run index** (changes ADR 0031).
- **The Schedule store** is a per-user, per-major SQLite file beside the Run index. It is the source of truth for Schedules and is not rebuilt by `goodfetch runs reindex`.
- **Occurrence history.** Each occurrence records its due and fire times, its trigger (time, Watch, after a run, or manual "run now"), the Schedule version it used and its outcome: `started` with its run ID, `skipped: overlap`, `skipped: Limit`, `skipped: no change`, `missed` or `failed to start`. Dagster's tick history ([skip reasons](https://dagster.io/docs/api/dagster/schedules-sensors)) and Firecrawl's check statuses play the same role.
- **Run index rows** gain the Schedule ID, the occurrence ID and the trigger kind. Watch runs are listed but hidden from the Runs list by default.
- **Edits** apply from the next occurrence. "Run now" starts an occurrence at once under the overlap policy.

**Retention.**
- **What a Schedule keeps by default:**
  - its last 10 runs
  - its current Baseline
  - any run still in "Needs you" or whose Sinks are `behind`
  - pinned runs
  - the last 10 Watch runs and every Watch that found a change
- **What goes:** older runs are erased as ADR 0015 erases a run, by destroying its data key. Occurrence history is kept 90 days.
- **Settings:** every number is a setting, and Operator policy can cap them.
- **Precedents:** Kubernetes keeps 3 successful and 1 failed Job ([CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/)), Apify the 10 most recent runs ([storage](https://docs.apify.com/platform/storage/usage)), and Firecrawl monitor checks for 30 days by default.

**The Console and Notifications.**
- **A Schedules screen** joins ADR 0029's side menu. Each Schedule shows its target, trigger, timing in plain words, next occurrences, last outcome, period Usage against its period Limit, its state (on, paused, auto-paused) and a *from file* badge where it applies.
- **A Schedule drawer** shows the occurrence history as a timeline with skip reasons, and offers pause, resume, run now, edit and delete. Edit and delete are disabled for Schedules from files.
- **Links:** a Recipe's page lists its Schedules, and a run's drawer names the Schedule and occurrence that started it.
- **"Needs you"** gains auto-paused Schedules, Schedules stopped by a period Limit, and the warning that the Daemon is not installed as a service.
- **Changes ADR 0027:** new Notification events: Schedule auto-paused; occurrences skipped by a Limit, once per period; records changed; missed occurrences, once per wake.

**People.**
- On a shared Daemon (ADR 0031), viewers read Schedules and their history. Members create, edit, pause, resume and run now Schedules for targets they may run. Operators act on any Schedule and set caps in Operator policy: number of Schedules, minimum interval, period Limits.
- Named tokens can be limited to `schedules.*` and to certain Recipes.
- Each Schedule records its creator, whose rights are checked at every occurrence. If they no longer allow the target, the Schedule pauses with a "Needs you" item.
- Every change goes to the Audit log with a before-and-after diff, and each run records its Schedule and creator.
- Apify, Dagster+ and Temporal Cloud split schedule permissions the same way.

**Goodfetch Cloud.**
- Its gateway implements the `schedules.*` operations with its own durable scheduler and the semantics above.
- One Schedule conformance suite runs against a Daemon and a stub gateway, as the Console's does.
- Schedules move between a machine and the cloud through the file form and `goodfetch schedules apply`.
- Cloud-only matters (its minimum interval, pricing, regions) belong to the cloud map.

**Product majors** (adds to ADR 0030).
- The Schedule file format is a versioned contract format.
- When a new major's Daemon first starts, or `goodfetch upgrade-files` runs, Schedules move to the new major's store: under both stores' locks, they are copied, upgraded and disabled in the old store with a pointer to the new one.
- A Schedule is never active in two majors. Rolling back moves it back the same way.
