# The Daemon schedules runs above the engine, and Goodfetch Cloud implements the same Schedule operations

The open-source framework schedules runs itself. A Schedule starts runs of one target (a Recipe or a code-free catalog run) on a time trigger, after a Watch finds a change, or after another run finishes. Schedules are kept by the Daemon, the only engine that outlives its Clients (ADR 0024), and are managed through `schedules.*` operations in the Operation catalog (ADR 0011), which Goodfetch Cloud's gateway implements with the same semantics (ADR 0031). The scheduler sits above the runs and starts them only through the public run operations. It passes each run's Limits and Baseline as ordinary parameters and tags the run with opaque labels kept in run state, so a scheduled run behaves like any other and Goodfetch Cloud's per-run engines (ADR 0001) need only what every Client gets: labels on a run and a caller-chosen run ID that makes `runs.start` idempotent. Crawlee leaves scheduling to cron or to Apify's platform ([Apify Schedules](https://docs.apify.com/platform/schedules)), and Firecrawl offers it only in its hosted API ([`/v2/monitor`](https://docs.firecrawl.dev/features/monitor)). Building it into the framework answers the monitoring use case (context code #3, `usecase-monitoring-leadgen`, in `rnd/research/market/pains-ranking.md`), scheduling and orchestration (pain #15) and self-host parity (#17). Decided in [Scheduling and recurring runs](https://github.com/goodfetch/goodfetch/issues/109).

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
- **An existing Rust scheduler crate**: rejected. tokio-cron-scheduler runs an hour off after a DST change until restarted ([#107](https://github.com/mvniekerk/tokio-cron-scheduler/issues/107)), and none documents overlap or catch-up. Goodfetch parses with `croner` (MIT), which documents its DST rules where the `cron` crate does not ([croner-rust](https://github.com/hexagon/croner-rust)), and writes the scheduling loop itself.
- **The previous run as the Baseline**: rejected. After one failed night every Record would look new, and a partial run would soft-delete every Record it did not reach (ADR 0026).
- **Only verified and complete runs as the Baseline**: rejected. Offset-paginated crawls and AI tasks without a Success check can never be both (ADR 0029), so their Schedules would get no change tracking.
- **Pre-creating future runs** (Prefect creates up to 100, up to 100 days ahead, and recreates them on every edit; [Prefect](https://docs.prefect.io/v3/concepts/schedules)): rejected. Upcoming occurrences are computed when asked, as Temporal shows them.
- **Schedules inside the Run index**: rejected. `goodfetch runs reindex` rebuilds the index from runs (ADR 0031), and a Schedule cannot be rebuilt from its runs.

## Consequences

**Targets.**
- **One target per Schedule:** a Recipe or a code-free catalog run (a crawl, an extraction, a Plan replay, an agent task), with its inputs and the run's own settings (layer 5 of ADR 0028), within what the Recipe exposes.
- **Recipe versions.** A published Recipe is pinned by digest. A local Recipe is named by path and runs as it is on disk at each occurrence; the digest that ran is recorded on the run. An SDK script becomes a local Recipe, since the Daemon cannot call handlers in a process that is not running.
- **Operator policy** applies as it stands when each occurrence fires (ADR 0031).

**Where Schedules are defined.**
- **From any Client:** created and changed through `schedules.*` operations, so the SDKs, CLI, MCP server, Console and Integrations all have them. The CLI offers `goodfetch schedules list|show|create|edit|pause|resume|run|delete|apply|preview`.
- **From files:** a Schedule file kept in the project, its own contract format (ADR 0030), reconciled into the Daemon by `goodfetch schedules apply`. Such Schedules are marked *from file*: `schedules.edit` and `schedules.delete` refuse them on every surface, and the Console disables those actions as it does for managed policies (ADR 0031), so a later `apply` never silently undoes an edit. Pause, resume and run now still work. A Recipe manifest may suggest a default Schedule but never creates one.
- **What `apply` does.** It needs the same role as creating a Schedule and records the person or token that ran it as the creator (see People). It creates and updates the file's Schedules, deletes those it created earlier from the same file that the file no longer holds, and leaves a Schedule's paused or running state alone unless the file sets `enabled`.

**Triggers.**
- **Time:** cron (five fields plus optional seconds, `L`, `W`, `#` and the `@daily`-style macros, parsed by `croner`) or a fixed interval (`every 15m`) counted from the Schedule's anchor time, not from when it was saved, unlike n8n ([common issues](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.scheduletrigger/common-issues/)).
- **Watch, then run:** each occurrence first runs a Watch, a small run that checks chosen pages or values with a conditional request (`ETag`, `Last-Modified`) or a hash of pages or of an extracted field. The target run starts only if something changed. changedetection.io works this way.
  - **Compared with what:** a Watch compares with what the Watch saw before the Schedule's latest run that qualifies as a Baseline (below), even when another Baseline is pinned, so a change whose target run failed, stopped, was rejected or ended incomplete is still detected next time. With no Baseline yet, the target run always starts. "Run now" starts the target without a Watch.
  - **A run like any other:** a Watch has Usage, Limits, Sessions and Access modes. Its Usage counts toward period Limits, a live Watch is the Schedule's live run for the overlap policy, and a failed Watch counts toward auto-pause.
  - **The benchmark:** ADR 0025's change-triggered agent runs as a Schedule with a Watch.
- **After a run:** a Schedule fires when a run of another Schedule or Recipe finishes with a chosen outcome, verified by default. A chain that would lead back to the same Schedule, directly or through Recipe triggers, is refused.
- **Acting on change without a Watch** comes from change tracking against the Baseline, the "only new or changed" Sink mode (ADR 0026) and a new "records changed" Notification event, as Firecrawl's monitor and Browse AI do.
  > Amended by [ADR 0103](0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md): change tracking identifies a Record across runs by its Dataset's `key`; how it compares the Records of a Dataset with no key is area 6's spec.
- Watches never use the Response cache, and a Schedule whose target sets `replay` is refused (ADR 0046).
- **Inbound webhooks** need no trigger of their own: ADR 0027's HTTP listener already lets a workflow tool start a run.

**Time zones and DST.**
- Every Schedule stores an explicit IANA time zone, defaulting to the host's at creation and never stored as "local", so it means the same thing in Goodfetch Cloud.
- Cron occurrences follow `croner`'s rules. A fixed-time occurrence in the skipped spring hour fires once, at the first valid time after the gap, and one in the repeated autumn hour fires once. A wildcard or step expression (`*/15 * * * *`) has no occurrences in the skipped hour and fires at each matching time of the repeated hour. Intervals count elapsed time. A conformance test covers both transitions for both kinds.
- Optional jitter, 0 by default, is the same delay at every occurrence of a Schedule (systemd's `FixedRandomDelay=`).
- No product minimum interval. Operator policy may set one, and Goodfetch Cloud sets its own.
- The Console and CLI preview the next occurrences in plain words before saving.

**Overlap.** When an occurrence fires while the Schedule's previous run is live, including one waiting in a Human pause, the Schedule's policy applies:
- `skip` (the default), recorded as *skipped: overlap*
- `queue_one`: at most one occurrence waits
- `allow`: runs concurrently, still under ADR 0004's per-domain admission
- `replace`: the scheduler aborts the live run, which checkpoints and stops at its next safe point (an Effect in flight finishes or is reported uncertain, ADR 0029), then the new run starts

**Missed occurrences.**
- An occurrence is missed when the Daemon was not running or the machine slept; the Daemon detects a sleep by the wall clock jumping.
- On start or wake, the latest missed occurrence runs once if it falls within the catch-up window: by default until the occurrence after it is due, so a weekday 9:00 Schedule missed on Friday can still catch up until Monday 9:00; settable, 0 meaning never. Every other one is recorded as *missed*. This is systemd's `Persistent=`, and n8n 2.36's opt-in run-once policy with a grace period.
- **Changes ADR 0024:** a Daemon holding an enabled Schedule of the active major (see Product majors) never stops idle. Creating the first Schedule on a Daemon not installed as a service offers `goodfetch daemon install`; until then, the Console and CLI warn that Schedules fire only while the Daemon runs.

**Limits across runs.**
- Each run keeps its own Limits. A Schedule may also carry period Limits: caps on the total Usage of its runs per day, week or month, counted in its time zone.
- Before a run starts, its Limits are clamped to what is left of the period, and that clamp is reserved against the period until the run ends and its actual Usage replaces it. Runs that overlap (`allow`, Watches, a resumed run) therefore cannot jointly exceed the period's Limit. Each run is held to its clamp like any Limit: a currency Limit is never exceeded, because ADR 0012 reserves each model call's worst case, while tokens, browser time and similar Usage may run over by what passes before the next safe point.
- A run counts in the period in which it started. Resuming a scheduled run, or raising its Limit to resume it, is clamped and reserved against that same period.
- Once a period's Limit is used up, occurrences are recorded as *skipped: Limit*, one Notification goes out per period, and the Schedule resumes next period, as Firecrawl records `skipped_no_credits`.
- Operator policy can cap period Limits.
- A scheduled run also counts against the Limits of ADR 0041 that apply to the Schedule's creator: the install, the Daemon, the person and the named token. A change that raises a period Limit from a surface other than the Console or an interactive CLI prompt, including `goodfetch schedules apply`, waits for a person's confirmation (ADR 0041).

**Baseline and soft-delete.** For a Schedule that targets a Partitioned crawl, each rule below reads the whole crawl's three axes and Records (ADR 0054).
- A Schedule's runs compare against its latest run whose Lifecycle is `finished`, whose Verdict is not `rejected` and whose Completeness is not `incomplete`, unless the Schedule pins a Baseline. A `failed` or `stopped` run never becomes the Baseline.
- `verified` and `needs-review` runs, with `complete` or `unverified` Completeness, may serve, because ADR 0029 never lets some runs be both verified and complete: an offset-paginated crawl is always `unverified`, and an AI task without a Success check always `needs-review`. Requiring both would leave such Schedules without change tracking. The cost is that a comparison with an `unverified` run may report as new a few Records it had missed.
- **Changes ADR 0026:** only a complete run may soft-delete Records missing since its Baseline, because an incomplete one cannot tell removed from not reached.
- Change tracking says when a run and its Baseline used different Clean-up rules, so a difference the rules caused is not reported as a change on the site. A Watch that hashes extracted content holds to the version its comparison point recorded, and starts the target, saying why, when it cannot get it (ADR 0047).

**Pauses.** A Schedule pauses itself on a timed-out Human pause (below), when its creator loses the right to run its target, and on repeated failures; each sends the "Schedule auto-paused" Notification and adds a "Needs you" item. After 3 consecutive occurrences whose run's Lifecycle is `failed`, whose Verdict is `rejected`, or that *failed to start* (settable, 0 meaning never), the Schedule pauses itself, sends a Notification and adds a "Needs you" item. `stopped` runs (a Limit, an abort, a pause timeout or exhausted resources, ADR 0040) and skipped or missed occurrences neither count nor reset the count; any other ending resets it. ADR 0026 likewise pauses a webhook Sink after three days of total failure or a `410`.

**Unattended runs.**
- A scheduled run resolves Secret references in the Daemon's context (its environment, keychain and vault logins), and its data key is wrapped by the Daemon's keychain key (ADR 0015).
- On a Daemon, a Schedule whose target uses the `caller` Secret provider is refused, since nobody is there to answer. Goodfetch Cloud's gateway is itself the caller that injects a tenant's secrets (ADR 0015), so it accepts them. An `env:` reference that the creating Client can resolve but the Daemon cannot raises a warning at creation. A reference that does not resolve when an occurrence fires fails the run before it starts, naming the reference.
- A Human pause in a scheduled run sends its Notification and "Needs you" item as usual. Its timeout is the Schedule's next due occurrence for a time or Watch trigger, and 24 hours for an after-a-run trigger, both settable.
- The timeout counts only once the paused work items are all that keep the run from ending; while other Sessions still make progress, the run is simply live. When it expires, it acts before the overlap policy, whatever the policy: the scheduler aborts the run, which checkpoints and ends `stopped` (an abort under ADR 0029), resumable by its run ID (ADR 0031), the occurrence that came due and any queued one are recorded as *skipped: paused*, and the Schedule pauses itself with a "Needs you" item naming the pause, such as a login needed on a domain. A next run would usually hit the same pause, and under `skip` an unanswered pause would otherwise block every later occurrence.

**The scheduler.**
- It is a component of the Daemon above the runs, and starts each run through `runs.start` like any Client.
- Each occurrence has an ID (the Schedule ID and the due time) written to the Schedule store before its run starts. An after-a-run occurrence is keyed by the upstream run's ID instead of a due time. Each run's ID is derived from the Schedule store's own random ID, the occurrence ID and the run's role (`watch` or `target`), so the same Schedule applied on two machines never yields the same run ID. `runs.start` with an existing run ID returns that run when the caller and inputs match, and otherwise refuses with a conflict. The ID's characters are left to the spec, since it also names the run's folder (ADR 0031). A scheduler restarted after a crash therefore never starts an occurrence twice, whenever it crashed. n8n's durable scheduler also queues runs in a database so that one instance picks up each, though its docs still allow duplicates ([durable scheduler](https://docs.n8n.io/deploy/host-n8n/configure-n8n/durable-scheduler.md)).
- The scheduler holds an exclusive OS lock on the Schedule store (ADR 0031's lock-file pattern), so a second Daemon on the same files cannot fire. In a Daemon group, only the Lead Daemon keeps the Schedule store and fires, and that role replaces the lock (ADR 0055).

**Storage and the Run index** (changes ADR 0031).
- **The Schedule store** is a per-user, per-major SQLite file beside the Run index. It is the source of truth for Schedules and is not rebuilt by `goodfetch runs reindex`.
  > Amended by [ADR 0098](0098-both-products-share-one-goodfetch-home-and-one-daemon-per-major-run-by-the-fullest-product-installed.md): the Schedule store serves both Products and records the lowest minor of its major that can use it. A Schedule records the Product its target needs, and a Daemon that lacks that Product fires nothing for it: each occurrence is recorded as missed with `not_in_product`, and a "Needs you" item names the fix.
- **Occurrence history.** Each occurrence records its due and fire times, its trigger (time, Watch, after a run, or manual "run now"), the Schedule version it used and its outcome: `started` with its run ID, `skipped: overlap`, `skipped: Limit`, `skipped: paused`, `skipped: no change`, `missed` or `failed to start`. Dagster's tick history ([skip reasons](https://dagster.io/docs/api/dagster/schedules-sensors)) and Firecrawl's check statuses play the same role.
- **Run labels.** `runs.start` accepts opaque labels stored in run state. The scheduler sets the Schedule ID, occurrence ID, trigger kind and creator, Run index rows show them, and `goodfetch runs reindex` restores them from run state. The Schedule's live run, for the overlap policy, is any live run carrying its label, a manually resumed one included. Watch runs are listed but hidden from the Runs list by default.
  > Wording note (agent decision, accepted by the owner on 2026-10-21; [#215](https://github.com/goodfetch/goodfetch/issues/215)): a Run's opaque strings are its **tags**, not labels, as the owner decided on 2026-10-16 ([#1230](https://github.com/goodfetch/goodfetch/issues/1230)), since the glossary's Label names a kind of crawl request. `run.start` and `crawl.start` take `tags`, at most 16 of at most 64 characters each, stored in run state and shown in the Run index row; the start operation is `run.start` (ADR 0099). The decision itself is unchanged.
  > Note (agent decision, as built; [#835](https://github.com/goodfetch/goodfetch/issues/835)): a reserved tag is written `goodfetch.<key>:<value>`, as the scheduler's keys `schedule`, `occurrence`, `trigger` (`time`, `watch`, `after_run`, `run_now`), `creator` and `role` (`watch` or `target`) name it, and `recipe`, the name of the Recipe a run ran, which a Recipe's starter sets; `run.list` reads a run's Recipe, Schedule, occurrence and trigger from them, and leaves out a run tagged `goodfetch.role:watch` unless its `watch_runs` asks for Watch runs. An erased row keeps no tags, so it is listed whatever its role was.
- **Edits** apply from the next occurrence. "Run now" starts an occurrence at once under the overlap policy.

**Retention.**
- **What a Schedule keeps by default:**
  - its last 10 runs
  - its current Baseline
  - any run still in "Needs you", whose Sinks are `behind` or `failed`, or that still holds dead-letter Records (ADR 0026)
  - pinned runs
  - the last 10 Watch runs, the last 10 Watches that found a change, and the Watch that ran before the Schedule's latest run that qualifies as a Baseline; if that Watch is missing anyway, the next target run starts
- **What goes:** an older run is discarded: its data key is destroyed (ADR 0015), and its file and directory are removed, or in the per-install layout its rows are deleted from the shared database (ADR 0002), and its Run index row is marked *erased*, never silently dropped; `goodfetch runs reindex` keeps such rows *erased* rather than *missing* (changes ADR 0031). Occurrence history is kept 90 days.
- **Settings:** every number is a setting, and Operator policy can cap them.
- **Precedents:** Kubernetes keeps 3 successful and 1 failed Job ([CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/)), Apify the 10 most recent runs ([storage](https://docs.apify.com/platform/storage/usage)), and Firecrawl monitor checks for 30 days by default.

**The Console and Notifications.**
- **A Schedules screen** joins ADR 0029's side menu. Each Schedule shows its target, trigger, timing in plain words, next occurrences, last outcome, period Usage against its period Limit, its state (on, paused, auto-paused) and a *from file* badge where it applies.
- **A Schedule drawer** shows the occurrence history as a timeline with skip reasons, and offers pause, resume, run now, edit and delete. Edit and delete are disabled for Schedules from files.
- **Links:** a Recipe's page lists its Schedules, and a run's drawer names the Schedule and occurrence that started it.
- **"Needs you"** gains auto-paused Schedules, Schedules held back by a used-up period Limit, and the warning that the Daemon is not installed as a service.
- **Changes ADR 0027:** Notifications can be about a Schedule as well as a run. New events: Schedule auto-paused; occurrences skipped by a Limit, once per period; missed occurrences, once per wake; and, for any run with a Baseline, records changed. ADR 0027's delivery ledger keys the once-per-period and once-per-wake events by Schedule and period or wake, so they are never repeated.

**People.**
- On a shared Daemon (ADR 0031), viewers read Schedules and their history. Members create, edit, pause, resume and run now Schedules for targets they may run. Operators act on any Schedule and set caps in Operator policy: number of Schedules, minimum interval, period Limits. Limits across runs, such as a person's, are kept in the Usage ledger instead (ADR 0041).
- Named tokens can be limited to `schedules.*` and to certain Recipes.
- Each Schedule records as its creator the person or token that last created, edited or applied its definition; pausing, resuming and run now do not change it. The creator's rights are checked at every occurrence, so editing another member's Schedule never lends it someone else's rights. If they no longer allow the target, the Schedule pauses with a "Needs you" item.
- Every change goes to the Audit log with a before-and-after diff, and each run records its Schedule and creator.
- Apify, Dagster+ and Temporal Cloud split schedule permissions the same way.

**Goodfetch Cloud.**
- Its gateway implements the `schedules.*` operations with its own durable scheduler and the semantics above.
- One Schedule conformance suite runs against a Daemon and a stub gateway, as the Console's does.
- Schedules move between a machine and the cloud through the file form and `goodfetch schedules apply`.
- Cloud-only matters (its minimum interval, pricing, regions) belong to the Cloud map.

**Product majors** (adds to ADR 0030).
- **Formats.** The Schedule file format is a versioned contract format, upgraded in place in the project like Plans. The Schedule store is per-user, per-major state, so a new major gets its own copy and the previous major's store is never changed.
- **One active major.** A per-user marker, outside the per-major folders, names the major whose scheduler fires. The first Schedule created on the machine writes it. Every Daemon reads it before each occurrence, so a Daemon of another major fires nothing, even while it runs and holds its own store's lock; its Schedules neither keep it from stopping idle nor record occurrences as missed.
- **Creating outside the active major.** Creating or enabling a Schedule in a major the marker does not name warns that it will not fire there and offers to switch.
- **Switching is its own confirmed step:** `goodfetch schedules activate --major <n>`, also offered as a prompt in the CLI and Console when a newer major's Daemon finds Schedules only in an older one. Switching to a newer major first copies and upgrades the older major's Schedules into the new store, after confirmation; a Schedule whose ID the new store already holds is reported and skipped. The newly active major starts at each Schedule's next occurrence, with no catch-up for the time it was inactive. `goodfetch upgrade-files` upgrades Schedule files in projects but never switches the marker, so trying a `0.y` preview, where each preview minor counts as a major, leaves the Schedules firing where they were. Because it also upgrades Recipe manifests and Plans in place, which the older major then refuses (ADR 0030), it warns when the project holds a Recipe that a Schedule of the active major targets: those Schedules would fail to start until the marker is switched or the files are restored.
- **Rolling back** is the same command naming the older major. Its store is as it was, so changes made under the newer major are not carried back, and the command says so.
