# A program names its Run by an ID that a start resumes, ends it with a Run report, and reads what waits and why

ADR 0078 makes the Run a value the program opens, and ADR 0002 makes it resumable from its committed state, but nothing said how a program starts one, resumes it, follows it, ends it or reads what it did. This is where the leaders fail most visibly ([synthesis](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) pains 6 and 10, [crawl frameworks](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §6.3 and §6.4):
- **A Run is inferred from a storage folder.** "The second time the code is run it does not work" ([crawlee#1602](https://github.com/apify/crawlee/issues/1602)). Crawlee 3.18.2 purges its default queue on a second `run()`, crawlee-python 1.10.4 still does, and a restart with `purgeOnStart: false` did nothing ([crawlee#3315](https://github.com/apify/crawlee/issues/3315)). Its maintainers write that "people are still asking about the storages, purging, resuming" ([#1782](https://github.com/apify/crawlee/issues/1782)).
- **Resuming and retrying are fragile.** Resuming a Scrapy `JOBDIR` job is "only supported when the spider is paused by stopping it cleanly" ([jobs.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/jobs.rst#L104-L107)). Re-running a crawl's failed requests has been asked of Crawlee since 2022 ([crawlee#1363](https://github.com/apify/crawlee/issues/1363), open), and Firecrawl's v2 API has no resume and no retry.
- **Status is counts with no reasons.** Crawlee logs "Crawled N pages, F failed requests, desired concurrency C" every 10 s, and Scrapy "Crawled N pages (at R pages/min)" every 60 s. Neither says why a request waits, and Crawlee hung silently for years ([#505](https://github.com/apify/crawlee/issues/505), [#1829](https://github.com/apify/crawlee/issues/1829)).

Crawlee 4, unreleased, now continues a named queue and warns when a run processes nothing; Scrapy 2.19 adds a `/status` endpoint so its MCP server can say why a crawl is slow. Decided in [The Run in code](https://github.com/goodfetch/automan/issues/478), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/automan/issues/478#issuecomment-6181744417) and [round 5](https://github.com/goodfetch/automan/issues/478#issuecomment-6181977614). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). It builds on ADR 0093 ([PR #512](https://github.com/goodfetch/automan/pull/512)), whose crawl declaration a crawl's start takes; ADR 0095 ([PR #507](https://github.com/goodfetch/automan/pull/507)), whose connections carry these verbs; ADR 0101 ([PR #515](https://github.com/goodfetch/automan/pull/515)), whose `RunUsage` the report carries; and ADR 0103 ([PR #516](https://github.com/goodfetch/automan/pull/516)), whose Records and Sinks it counts. All four were in review when this was decided.

## Decision

### Starting a Run

- **Two verbs, which return once the Run is started, never at its end:**
  - `collie.start(options?)` returns a `Run`. A Foxhound connection carries it under the same name (ADR 0095).
  - `foxhound.startCrawl(crawl, options)` returns a `CrawlRun`, whose handlers run from then on. It is Firecrawl's `start_crawl` (S3). A crawl cannot share Collie's `start` on one connection: one name would have two return types (S2), and Rust has no overloading.
- **Their options:** `id` (below), `settings` (ADR 0083), `timeout` and `limits` (ADR 0080) and `plan` (ADR 0091); a crawl adds `seeds`, at least one.
- **Per language:** Python's `start(id=…)` and `start_crawl(crawl, seeds=[…], id=…)`; Rust's `collie.start().id("…")` and `fox.start_crawl(&crawl, seeds).id("…")`, `IntoFuture` builders (ADR 0080).
- They call the catalog's run operations (ADR 0036's `runs.start`), whose names on the CLI and MCP are [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)'s.

### The Run's ID

- **A program may name its Run** with `id`; otherwise the engine generates one. `run.id` is always set, and every error raised in the Run carries it (ADR 0079). An ID names one Run in the Run index for good (ADR 0031): a Run is never inferred from a storage folder.
- **Its characters** are area 3's spec, since the ID also names the Run's folder (ADR 0036). One the spec refuses fails with `run_id_invalid`.
- **The ID is the program's idempotency key.** A start whose ID already names a Run does what this table says, as Stripe's idempotency keys and ADR 0036's idempotent `runs.start` do:

  | The Run of that ID | The start |
  |---|---|
  | none | begins it: `run.startedAs` is `new` |
  | interrupted: no live writer after a crash; `stopped` by a close, an abort, its timeout, the engine's loss or exhausted resources; waiting for its script (ADR 0031); or `failed` | resumes it from its committed state, the run file on this machine or an imported Checkpoint, at the next run epoch (ADRs 0002, 0003): `resumed` |
  | `stopped` by a used-up Limit no person has raised | raises `limit_reached` (ADR 0041), whose `fix` names the Console, where a person raises the Limit and the Run waits for its script (ADR 0031) |
  | ended `finished` | returns it as it is, with no handler called: `ended`. Only `retryFailed` reopens it (below) |
  | live, with a writer: another program, or this one | raises `run_in_use` |
  | started with other inputs, or by another caller | raises `run_conflict`, listing each difference |
  | listed in the Run index, its state missing or unreadable | raises `run_state_unavailable`, and never starts afresh under that ID |

- **What one ID gives.** A program run again under its ID carries on where it stopped, which is how Scrapy's `JOBDIR`, Crawlee 4's named queue and Scrapling's `crawldir` behave (S3), with the name written in the code rather than found on disk (G2). A crash loop under one ID keeps one set of Limits and one Usage (G5). A Collie program run again under its ID finds its Sessions and its Effect journal, so an order placed before a crash is checked, never placed again (ADRs 0002, 0010, G5).
- **No restart and no purge.** Crawling again is a new Run, under a new ID or none; the old one stays in the Run index (S2). Recurring Runs are ADR 0036's Schedules.
- **Logged** through the host's logging (ADR 0078): a resume at info, a start that found its Run ended at warning.

### Resuming

- **The inputs must match,** or the start raises `run_conflict`:
  - the Product and the kind of Run, a crawl or a Collie Run;
  - a crawl's seeds, after the Frontier's URL rules (ADR 0005);
  - the Plan file (ADR 0091).

  A crawl's declaration is checked as ADR 0093 says: pending requests of a Label no longer declared raise `label_unknown`, a changed Payload schema follows ADR 0082, and a new Label is accepted. Handler code is the program's own and may change.
- **What may change**, each compared with what the program gave when the Run began:
  - `timeout`: the resumed Run's own, on a fresh clock (ADR 0080).
  - `limits`: an equal value leaves the Limit as it stands, a person's raise included; a lower one lowers it; a higher one raises `limit_raise_refused`, since only a person raises a Limit (ADR 0041).
  - `settings`: a changed value applies from the resume on and is recorded in the Run trace with its layer, and one pressing a site harder is recorded as ADR 0086 says. A key the settings schema marks as fixed for a Run's life raises `setting_not_applicable`, whose `fix` names a new Run. The fixed keys are those that decide the Run's scope or what its committed results mean, `crawl.scope.*` at least; area 3's spec lists them.
- **Each seed's answer.** A seed is checked as an enqueue is (ADR 0093), so one the run's domain policy refuses, a duplicate or one with an unsupported scheme is not added. `run.seeds` lists each seed's `Enqueued` answer, read with no round trip; on a resume, the answers given when the Run began. A seed is one request among many, so its refusal never raises (ADR 0079). A start whose seeds were all refused still starts, and its Run finishes at once unless `crawl.keep_alive` holds it.

### Following a Run

- **`run.progress()`** is ADR 0081's telemetry sequence (an `EventStream` in Rust) of `RunProgress`. It subscribes when called, and its first value is the Run's progress at once. Each later value is the latest after a change, coalesced under load (ADR 0024), and the last one says how the Run ended. There is no separate one-shot read (S2): a program reads one value and leaves the loop.
- **`RunProgress`**, for every Run:
  - `lifecycle`;
  - `pauses`: each Human pause waiting, with its Session, its reason, whether the program or the engine raised it, and since when (ADR 0092);
  - `message`: the program's own status line, set with `run.setProgressMessage(text)` and shown in the Console, as Crawlee's `setStatusMessage` sets one (ADR 0037's RT-15).
- **`CrawlRunProgress`** adds:
  - `requests`: the crawl's requests pending, being fetched, being handled and in a Human pause, and the ended ones by kind;
  - `waiting`: why due requests are held, one group per reason, each with its count, the domains it holds with when each is next admitted (a fixed number of domains per reason, the rest counted), and the setting behind the wait, with its value and layer (ADR 0079's `LimitSource`). The reasons are `domain_concurrency` and `domain_delay` (ADR 0086), `crawl_delay`, `backoff` (a 429, 502, 503, 504, or a connection refused or timed out), `robots_txt` (not served yet), `handler_concurrency` (`crawl.handler.concurrency` handlers already running), `governor` (ADR 0040's governor admitting nothing new), `session` (waiting for a Session of the pool, such as one logging in) and `human_pause`. A reason a newer engine sends is kept (G3);
  - `inFlight`: each request being fetched or handled, with its phase, tier, attempt and start time, a short list since concurrency bounds it;
  - `concurrency`: the handlers running against `crawl.handler.concurrency`, and the governor's level (normal, pressure or critical), its open pages against the allowance and what set that allowance: a cgroup, a Job object, `memory_allowance` or half of physical memory (ADR 0040). The research's autoscaling pain (CR §6.13) becomes a stated reason.
- **`run.outcomes({ kinds?, labels? })`** iterates the crawl requests' `CrawlRequestOutcome`s (ADRs 0079, 0093), each with its `endedAt`. It reads the Run's committed state in commit order from its first outcome, then follows new commits until the Run ends; its filters run in the engine (ADR 0081). A reader of committed state, as the Console pages Records (ADR 0031), it never slows the Run. A request `retryFailed` re-admits ends again, and its new outcome supersedes the old, so a program keys outcomes by `request.id`. There is no failed-request callback (ADR 0093): a failed request is read here, or counted in the progress and the report.

### Ending a Run

- **`run.finish()`** lets the Run finish, ends it `finished`, closes its Sessions and pages, and returns its Run report:
  - a Collie Run finishes at once, since its program says it is done;
  - a crawl finishes once its Frontier is empty and nothing is in flight;
  - a detached Run (below) finishes when the engine ends it.

  If something stops the Run first (a Limit, its timeout, a person, the engine), `finish()` returns the report saying `stopped`: an expected outcome (ADRs 0029, 0079). A Run that ends `failed` raises the error that failed it, as ADR 0079 has a failed ending of AI work raise.
- **Its wait** has no timeout of its own by default: the Run's own `timeout` bounds it (ADR 0080). A `timeout` or `signal` given to `finish()`, a cancelled task or a dropped future ends only the wait, and the Run goes on.
- **`run.stop()`** stops the Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an abort by its program (ADR 0029), and returns its report; the Run stays resumable by its ID.
- **`close()` is ADR 0078's, unchanged.** A scope left without `finish()` therefore stops the Run: honest, since the program never said it was done (G1).
- **A Collie Run its program finishes is `complete`:** the program defines its scope and says it reached its end. A crawl's Completeness stays ADR 0005's.
- **Keep-alive crawls** (ADR 0037's RT-12): with `crawl.keep_alive`, `false` by default, a crawl's Run stays `running` with an empty Frontier, waiting for `addSeed`, until `finish()` lets it end once drained.
- **Rust:** `crawl.serve(&run).await` returns the crawl's report once the Run ends, since serving is what lets a crawl finish; `goodfetch::blocking`'s `serve` does too. `run.finish()` exists as well, for Collie Runs and to end a keep-alive crawl from another future while `serve` runs (S1).

### The Run report

- **A Run report** is what a Run states about itself when it ends. **Run report** joins `GLOSSARY.md`, uniting ADR 0029's "run report" and ADRs 0041's and 0086's "run's result". `finish()` and `stop()` return it, `serve` in Rust, and a view (below) reads it later.
- **What it holds:**
  - the three axes (ADR 0029): `lifecycle` (`finished` or `stopped`), `stopReason` (a Limit by name, `timeout`, `abort` with who stopped it, `engine_lost` or `resources`), `verdict` and `completeness`, each with the reasons it is not `verified` or not `complete`;
  - `usage`: the final `RunUsage` of ADR 0101, its uncapped mark included (ADR 0041);
  - `weakened`: every setting that weakened a safeguard for this Run (ADR 0028), with its key, value, layer and origin, whether a costly control switched off (ADR 0023; the Run trace's "no trace" mark, ADR 0031) or a value pressing a site harder (ADR 0086);
  - `checkpoint`: the last Checkpoint written, or none;
  - `nothingDone` (G1): none, or `already_ended` (the start found the Run ended), `no_seed_handled` (no seed reached a handler: each was refused when added or skipped when fetched, counted by reason) or `nothing_pending` (a resume or a retry found nothing left);
  - `consoleUrl`: the Run's page in the Console, where its Run trace replays, when the engine knows a Console address (ADR 0031), or none;
  - `startedAt` and `endedAt`.
- **A crawl's report adds:**
  - `requests`: succeeded, failed, skipped, blocked and pending, with counts by skip reason, Error code and Block cause;
  - `records`: per Dataset, its committed, dropped and rejected Records (ADR 0103), the dropped ones counted by the Hook's reason;
  - `sinks`: per Sink, ADR 0026's delivery status (`complete`, `complete_with_rejections`, `behind`, `failed`) and its number of dead letters. A Sink may go on delivering after the Run ends, so a later read gives the status then.

### Retrying failed requests

- **`run.retryFailed({ blocked? })`** re-admits, in the same Run and its Datasets, every request that ended `failed`, and with `blocked: true` those that ended `blocked`. A skip is a decision (robots.txt, scope, a handler's `skip`), never re-admitted.
- Each re-admitted request keeps its ID, Label, Payload and depth. Its attempts go on counting, with a fresh `crawl.retry.max`.
- It returns how many it re-admitted; 0 says there was nothing to retry (G1).
- On an ended Run, it returns the Run to `running`, its Verdict and Completeness given anew when it ends, as ADR 0029's "it did not happen" already reopens a Run. Its Usage counts in the same Run, under its Limits (G5). A new Run seeded from the failures would have left the first one `incomplete` for good, with two Datasets to merge.

### Checkpoints

- **`run.checkpoint({ path? })`** writes a Checkpoint now, to the path given or the Run's own folder (ADR 0031), and returns its path, size, digest, run epoch, journal position and time, as ADR 0002 reports them.
- **`checkpoint.interval`** is ADR 0002's periodic interval, none by default locally.
- **On the same machine** a Run resumes from its run file and needs no Checkpoint. To resume elsewhere, the catalog's run import (ADR 0002) registers the Checkpoint, and a start with its ID resumes it. Session secrets stay encrypted in it (ADR 0015).

### Reading a Run the program does not hold

- **`collie.openRun(id)` returns a `RunView`, and `foxhound.openCrawlRun(id)` a `CrawlRunView`;** a Foxhound connection carries both (ADR 0095). Python's are `open_run` and `open_crawl_run`, Rust's the same. They serve a program that holds only an ID: an export of yesterday's crawl, a dashboard following a detached Run, another program's live Run. Temporal's `getHandle(id)` plays this part.
- **A view only reads:** `progress()`; `report()`, at once for an ended Run and at its end for a live one, which it never ends; and for a crawl, `outcomes()`, ADR 0103's `records(dataset)`, `rejectedRecords(dataset)` and `export(dataset, …)`, and its Key-value store's reads. The Dataset handles come from the program's own crawl declaration, so these reads are typed (ADR 0082).
- **It takes no writer lock** (ADR 0031) and gives the Run no work, so it never resumes an interrupted Run, which a start with the ID would. Closing it frees only the view. An ID the Run index does not list raises `run_not_found`.
- **Detaching.** The option is `detached: true` (Python `detached=True`, Rust `.detached()`). It exists only on the starts of Runs with no handlers in the program, a Recipe's run or a code-free catalog run (ADR 0036's targets), whose verbs come from the catalog or from their own tickets ([The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) for an Agent task). `collie.start` and `startCrawl` never take it, since the program drives a Collie Run and serves a crawl's handlers; the TypeScript and Python types leave it out (G3). A detached Run's value frees itself on `close()` (ADR 0078), and `finish()` waits for its end. A refusal on an Attached engine keeps ADR 0078's code, which area 11's spec names.

### Error codes

| Code | Group | When |
|---|---|---|
| `run_id_invalid` | `ProgramError` | An ID the engine refuses |
| `run_conflict` | `ProgramError` | The ID names a Run started with other inputs, or by another caller; the details list each difference |
| `run_ended` | `ProgramError` | A call that would give an ended Run new work, such as `newPage()` or `addSeed()` on a Run that `startedAs` `ended`; its `fix` names a new ID, or `retryFailed` |
| `run_not_found` | `ProgramError` | `openRun` or `openCrawlRun` with an ID the Run index does not list |
| `run_in_use` | `RefusedError` | The Run's writer's lock is held, by another program or this one (ADR 0031) |
| `limit_raise_refused` | `RefusedError` | A resume gave a Limit above the one the Run began with |
| `run_state_unavailable` | `UnavailableError` | The Run index lists the Run, but its state is missing or damaged |

## Considered Options

- **Refusing a start whose ID exists, with a separate `resume(id)`** (Temporal's default `FAIL`): rejected. Every crash-safe program would look the ID up and branch, the program of golden scenario 07 among them (T). **A conflict policy option** (`ifExists: "resume"`, Temporal's `USE_EXISTING`): rejected, a second way to resume (S2). **A start that always begins and a `resume` that always continues:** rejected, since the program would have to know which run it is.
- **Inferring the Run from a storage folder,** as Crawlee's queues and Scrapy's `JOBDIR`: rejected, the pain itself (G2). **Starting afresh under an ID whose Run finished,** as Scrapling forgets a completed crawl: rejected; an ID names one Run in the Run index, and an idempotency key replays its first answer.
- **A name kept apart from the Run ID,** as Temporal's workflow ID and run ID: rejected. A second identifier for the "run it again" a new ID already says (S2).
- **`foxhound.start(crawl, …)`**: rejected, beside Collie's `start` on the same connection (above). **`crawl.start(foxhound, …)`**, Crawlee's `crawler.run()`: rejected; the declaration ADR 0093 keeps free of I/O would start Runs, and an agent completing `foxhound.` would not find it (T). **`foxhound.crawl(…)`**: rejected; Firecrawl's `crawl` waits for the end (S3).
- **A start that resolves at the Run's end,** as Crawlee's `run()`: rejected; no `addSeed`, progress or early stop while it runs. **A wait verb per Product** (`finish` for Collie, `wait` for crawls): rejected, two concepts for one end. **`close()` ending a Collie Run `finished` when the program returns normally:** rejected; TypeScript's disposal cannot tell a return from a throw, and S1 asks for one rule.
- **Log lines only for progress,** as Crawlee and Scrapy: rejected, since a program cannot read them (G6). **A progress callback:** rejected; ADR 0081 shapes a sequence read to its end as an iterator. **A one-shot status read beside the sequence:** rejected (S2).
- **Outcomes as a lossless event stream that slows an unread Run** (ADR 0081's rule): rejected; a forgotten iterator would stall a crawl (G4, G5), and outcomes are committed state that a reader can page. **Counts only:** rejected; scenario 07 must list the failed URLs.
- **Retrying failures in a new Run:** rejected (above). **A failed-request callback** (Crawlee's `failedRequestHandler`): rejected by ADR 0093; a failed request is an outcome value.
- **Opening a past Run by starting its ID:** rejected; it needs the declaration and the seeds, and would resume an interrupted Run, which a reader must never do. **Only the catalog's reads:** rejected; no typed Records and no Export.
- **Raising when a seed is refused:** rejected; a seed is one request among many (ADR 0079).

## Consequences

- **Amends ADR 0029.** A finished crawl's Run also returns to `running` when its program retries its failed requests, its Verdict and Completeness given anew when it ends; and a Collie Run its program finishes is `complete`. A note sits under the Lifecycle item.
- **Amends ADR 0081.** A sequence read from a Run's committed state, as `outcomes()`, never slows its Run; the rule that an unread lossless sequence slows its Run covers what the engine must deliver. A note sits under the iterators item.
- **Amends ADR 0083.** The settings schema also marks, per setting, whether a resume may change it; `crawl.keep_alive` and `checkpoint.interval` join it. A note sits under its settings-schema consequence.
- **Amends ADR 0093** ([PR #512](https://github.com/goodfetch/automan/pull/512)): Rust's `crawl.serve(&run)` returns the crawl's Run report once the Run ends.
- **Adds to ADR 0041:** a resume that gives a Limit above the one the Run began with fails with `limit_raise_refused`, and a person's raise stands against an equal value in code.
- **Adds to ADR 0079:** a `CrawlRequestOutcome` carries `endedAt`, and a Run that ends `failed` makes `finish()` raise, as a failed ending of AI work does.
- **Adds to ADR 0080:** `finish()`'s wait has no timeout of its own; the Run's `timeout` bounds it.
- **Answers** ADR 0078's open points (the start verbs, `detached`), ADR 0079's (live states, reading outcomes, a call that did nothing), ADR 0081's (the progress sequence's name and start), ADR 0086's hand-off (the values that pressed harder, politeness waits), ADR 0093's (the start, refused seeds, reading failures), ADR 0101's (where the final `RunUsage` sits) and ADR 0103's (Records and Sinks in the report, opening a past Run).
- **Error codes.** The seven above join `goodfetch-protocol` in their groups, through `xtask codegen`'s naming checks (ADR 0079).
- **Glossary.** **Run report** joins `GLOSSARY.md`.
- **Area 3 (Engine runtime)** builds the idempotent start and its resume rules, the inputs check, the settings fixed for a Run's life, progress with its wait reasons, `outcomes` from committed state, `finish`, `stop`, keep-alive Runs, Checkpoints in code and the Run report. **Area 6 (Collection)** builds `retryFailed`'s re-admission and the seeds' answers. **Area 11 (Client surfaces)** builds the verbs, views and types per language and logs a resume. **Area 12 (Console)** shows the progress message and serves the Run's page that `consoleUrl` names. Each lists this ADR.
- **ADR 0037's matrix:** RT-10, RT-11, RT-12 and RT-15 cite this ADR; RT-12 (keep-alive Runs) and RT-15 (statistics and a status message) are decided.
- **`TESTING.md`** gains the Run conformance cases.
- **The `prototype/sdk-api` sketch** holds these signatures in its `run` files for each language, with scenario 07's program; the pain coverage matrix closes P09, P17, P47 and K05.
- **Other tickets of the map.**
  - [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500): a resume re-runs program code. A Collie program runs again from its top in the same Run, a crawl's requests that were in flight run again, and `retryFailed` re-runs failed requests' handlers, so a declared Effect is checked rather than repeated in each; a Run that `startedAs` `ended` runs nothing.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483): every scenario that opens a Run ends it with `finish()`, and scenario 07 is a start with an ID, `retryFailed()` and `finish()`.
  - [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) names the catalog's run operations on the CLI and MCP.
  - [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474): an Agent task started as a code-free Run may take `detached`.
