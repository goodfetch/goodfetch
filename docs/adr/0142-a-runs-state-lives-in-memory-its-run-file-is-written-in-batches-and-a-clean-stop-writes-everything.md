# A Run's state lives in memory, its run file is written in batches, and a clean stop writes everything

This ADR amends ADRs 0002, 0138 and 0140 and touches ADRs 0003, 0028, 0053, 0060, 0103 and 0134, as listed under Consequences. The owner took each of its open decisions on 2026-11-06, in wayfinder map [#2141](https://github.com/goodfetch/goodfetch/issues/2141): the Shared paths it touches ([#2142](https://github.com/goodfetch/goodfetch/issues/2142)), ADR 0134's ceilings ([#2143](https://github.com/goodfetch/goodfetch/issues/2143)), the batch and memory defaults ([#2144](https://github.com/goodfetch/goodfetch/issues/2144)), and the Durability levels with what readers see ([#2145](https://github.com/goodfetch/goodfetch/issues/2145)).

A Foxhound Run keeps its state in memory: the Frontier (its pending work items, its visited set and its per-domain queues), the Leases of the attempts in flight, and the Permits of a run-private Politeness. Memory is the truth the crawl reads and changes. The run file stays the portable SQLite file of ADR 0002, but it is now written in **Run-state batches**: the changes since the last batch, written in one transaction when a page count, a time or a byte size is reached. A clean stop writes every change and syncs it, so a Run resumed after a clean stop redoes nothing it had finished. Committing every attempt to the run file stays available as the opt-in level `full`, the absolute protection. Past a memory budget, or when the Resource governor reports memory pressure, the Run spills state that is already in the run file out of memory, so memory stays bounded. Every part of this is a setting, and `run_state.memory.budget: 0` with `run_state.durability: full` gives back the file-first behaviour ADR 0138 built.

The owner decided this on 2026-11-06 (offer absolute protection as an option; otherwise do as Spider does, even better if possible), after [the Spider gap measurement](../../rnd/research/2026-11-06-spider-gap.md). That measurement was taken on the Hetzner CCX43 at `origin/spec/1577` cc72b0f93, in `just compare`'s raw cell, 1,000 pages at 16 requests at once:

- **Foxhound ran 1.99 M instructions a page against Spider's 0.40 M, and 61 system calls against 12.** It reached 995 pages a second against Spider's 11,766 in #1931's matrix.
- **About 30 % of the instructions are architectural.** The run-state writer commits a claim and an outcome for every page, even at `durability: off`, with 8 WAL writes and 14 KB written a page. The run-private Politeness takes and gives back each Permit as an in-memory SQLite transaction on an actor of its own.
- **The page crosses three single-threaded hops,** with 33 futex calls a page against Spider's 5.5. Foxhound kept only 9 to 15 of its 16 requests in flight, at 1.1 to 1.7 cores. Spider kept 15 in flight at 2.3 to 3.7 cores.

At `off` a crash discarded the run file anyway, so the only promise those commits kept during the crawl was a clean stop's resume.

How other crawlers keep this state ([the frontier persistence survey](../../rnd/research/2026-11-06-crawler-frontier-persistence.md), each from its source at a pinned version):

- **Spider 2.53.9** keeps its visited set and pending links in memory. With its `disk` feature, it moves the visited set to SQLite past 15,000 links (`LINKS_VISITED_MEMORY_LIMIT`, `website.rs:1023`), or under host memory pressure: 50 % of host memory in use keeps at most 100 links in memory, and 80 % keeps none (`website.rs:1948-1976`, `utils/detect_system.rs:67`). That file only relieves memory. It is deleted on a clean drop, and a crash loses the whole crawl.
- **Scrapy's `JOBDIR`** appends each request and fingerprint without a sync. It writes its queue pointers only on a clean close.
- **Crawlee** writes one file per request without a sync, and persists its statistics every 60 s.
- **Heritrix** syncs its BerkeleyDB frontier only at a checkpoint, which is off by default.
- **StormCrawler** writes its status index in bulk: 500 actions or 5 s.
- **Nutch** rewrites its CrawlDb once per cycle.
- **The common pattern:** no crawler makes the page wait on a durable per-URL write. The working set lives in memory, the disk is a log or a snapshot behind it, and recovery replays work and drops duplicates. The opposite failure, marking a URL done before its fetch with no status to roll back (Colly), drops pages after a crash.

## Decision

### Memory holds the truth

- **A Foxhound Run's state lives in memory, owned by one actor.** That actor is the crawl's **Run state**, a module of `goodfetch-crawl` that the delivery loop runs. It owns the Frontier's admission, its work items, the visited set, the per-domain queues, the claims and the Leases, and it calls the store that holds a run-private Politeness's Permits. Every page reads and changes this state with no run-file read or write on its path. A page's task sends its outcome and its keyed links to the actor over a bounded channel. URL keying, Scope and Payload checks stay pure work on the page's task. The actor only checks the visited set, inserts and leases.
- **State the crawl only writes waits for the next batch.** This covers outcomes, attempt errors, Records, Provenance marks, learned rules, the tier choice and Session scores. The crawl reads the last three back from memory.
- **The duplicate check stays exact.** No Bloom, cuckoo or other lossy filter decides that a URL was seen.

### A page makes one round trip to the Run state

Decided by the owner on 2026-11-06 ([#2142](https://github.com/goodfetch/goodfetch/issues/2142)).

- **A page makes at most one round trip to the Run state's actor.** One message carries its outcome and its keyed links, and the one answer carries its Politeness Permit and the next Lease. The send and the answer are asynchronous and never block a worker thread. A page makes no round trip to the run-state writer, which only the actor's sealed batches reach, and none to a run-private Politeness, whose in-memory store the actor calls directly.
- **The state stays in its actor.** `STYLE.md`'s rule holds: state with one owner is an actor, with no `Mutex`. No state is shared by the pages' tasks under a lock.
- **The raw pass runs on the page's own task.** The streaming pass that reads the links and the head adds no hand-off. A format that needs a tree (the clean-up, readability, Markdown) adds one hand-off to the analysis pool, counted on its own row, so heavy CPU work never blocks an async worker. The extraction-path spec, which [#2158](https://github.com/goodfetch/goodfetch/issues/2158) writes, decides that pool and its row.

### The run file is written in Run-state batches

- **A batch is every change since the last batch, written by the run file's single writer in one transaction.** It is sealed when the first of three thresholds is reached:
  - `run_state.batch.pages`, 1,000 attempts ended by default;
  - `run_state.batch.interval`, 1 s by default, read on the `Clock` (ADR 0059);
  - `run_state.batch.bytes`, 8 MiB of rows by default.

  Each threshold also takes `none`, which never seals a batch.
- **One batch is written at a time.** While the writer writes one batch, the next keeps filling. The channel to the writer holds one sealed batch. When a second batch is sealed before the first is written, admission waits, which is backpressure, so the changes not yet written stay bounded.
- **`run_state.batch.per_attempt`,** false by default. When true, each claim is a batch of its own, written before its fetch, and each outcome is a batch of its own, written before anything is told outside. The writer still groups them across pages, as ADR 0138 decides. This is today's per-page path, and the thresholds are then unused.
- **`run_state.batch.sync`** decides which batches are synced:
  - `each`, the default: every batch is synced, so syncing costs about one `fsync` a second at the default thresholds;
  - `interval`: one batch is synced every `run_state.commit_interval`, 5 s by default, and an idle writer syncs once that interval ends, as #1927 built it;
  - `never`: no batch is synced. SQLite keeps its write-ahead log in order, so a process crash loses nothing written and a power cut may roll back recent batches, without corruption;
  - `off`: no batch and no checkpoint is synced, and SQLite does not keep its writes in order, so a crash can leave the run file corrupt. A run file written at `off` therefore carries the "unclean" mark (below).
- **The "unclean" mark.** While `run_state.batch.sync` is `off`, the run file carries the "unclean" mark, and a crash discards it and the Run starts again, as ADR 0138's `off` decides. A clean stop's last batch is synced and clears the mark. With every threshold `none` and `per_attempt` false, nothing is written during the crawl but what a spill writes (below).
- **Every key here is a per-Run setting.** Like `run_state.durability`, any layer may set it: the Run's own settings, user configuration, or an Operator policy, which may lock it (ADR 0028). It is read when the Run opens and when it resumes. `config explain` shows each key's value and where that value came from: a layer, or the preset of the Run's Durability level (below).
- **A batch is all or nothing.** The run file holds the Run as of one batch boundary, never half of a page. ADR 0138's savepoint per page leaves the batch path, since a batch is built from changes memory already accepted.
- **The run file keeps its tables and its format.** A batch writes the same rows that today's commits write, so Checkpoints, `run.resume`, `run.records`, the kept read-only connections, Sinks and the per-install layout read it unchanged. This keeps ADR 0002's portable file and rejects a second, log-shaped format.

### Durability levels

`run_state.durability` names a **preset** of the batch settings. Its five values are the four that exist today and `batch`, so no setting value that is valid today becomes invalid.

- **A level unfolds into its preset's keys at the layer that set it** (owner, 2026-11-06, [#2145](https://github.com/goodfetch/goodfetch/issues/2145)). ADR 0028's layers then resolve each key as any other setting:
  - a key set at the level's layer or a higher one overrides the preset's value for that key, so `periodic` with a 1 s `run_state.commit_interval` in the same layer is a starting point, not a cage;
  - a key set at a lower layer gives way to the preset of a level a higher layer chose, so a Run that asks for `fast` gets `fast` even when user configuration sets `run_state.batch.sync: each`;
  - a lock on the level locks every key of its preset at the preset's value, so a higher layer that sets one fails with `setting_locked` naming the lock, and an Operator policy that locks `full` gets `full`.

  `config explain` shows each key's value with its source, for example "preset of `run_state.durability: full` (Operator policy, locked)".

| Level | Preset | When the run file is written | After a process crash | After a power cut |
|---|---|---|---|---|
| `full` (opt-in: absolute protection) | `per_attempt: true`, `sync: each` | each claim before its fetch, and each outcome synced before anything is told outside, grouped across pages | only attempts in flight are redone | only attempts in flight are redone |
| `fast` | `per_attempt: true`, `sync: never` | as `full`, with no sync | only attempts in flight are redone | recent commits may roll back, without corruption |
| `periodic` | `per_attempt: true`, `sync: interval`, `commit_interval: 5s` | as `fast`, with one synced commit per `run_state.commit_interval` | only attempts in flight are redone | at most one `run_state.commit_interval` is redone, without corruption |
| `batch` (the default) | `per_attempt: false`, `pages: 1000`, `interval: 1s`, `bytes: 8MiB`, `sync: each` | one batch per threshold | at most one batch and the attempts in flight are redone | the same |
| `off` | `per_attempt: false`, `pages: 1000`, `interval: 1s`, `bytes: 8MiB`, `sync: off` | one batch per threshold, as `batch`, never synced | the run file carries the "unclean" mark, is discarded, and the Run starts again from nothing | the same |

- **One model serves every level.** Memory is the truth at every level, and a level only decides when the writer writes.
- **`full`, `fast` and `periodic` keep ADR 0138's guarantees and costs.** `full` makes at most 2 run-file transactions and 1 sync an attempt.
- **The default becomes `batch`.** It loses at most about one second of work to a crash. Spider, Scrapy without `JOBDIR`, and Colly lose the whole crawl, and Crawlee and Scrapy with `JOBDIR` sync nothing. `full` is one setting away.
- **What an acknowledgement means.** ADR 0002's "an acknowledged commit survives a power cut" holds at `full`. At `batch`, an acknowledgement means the batch that holds it was written and synced (below).
- **`weakened` is read against the default** (ADR 0028). The run report's `weakened` and `config explain` label each of these:
  - `off`;
  - a `run_state.batch.sync` other than `each`, which covers `fast`, `periodic` and `off`;
  - a threshold above its default, or `none`.

  `full`, and any threshold below its default, are recorded as chosen.
- **Nothing is published,** so no format changes version (ADR 0066).

### Nothing removed

Each behaviour available before this ADR has a setting that gives it back:

| Behaviour today | Setting that gives it back | After a process crash | After a power cut |
|---|---|---|---|
| `full`, the old default | `run_state.durability: full` | only attempts in flight are redone | only attempts in flight are redone |
| `fast` | `run_state.durability: fast` | only attempts in flight are redone | recent commits may roll back |
| `periodic` | `run_state.durability: periodic` | only attempts in flight are redone | at most one `run_state.commit_interval` |
| `off` | `run_state.durability: off` | the Run starts again from nothing | the same |
| a crawl that writes nothing to the run file until it stops | `run_state.durability: off` with `run_state.batch.pages`, `.interval` and `.bytes` set to `none`: only a spill writes, and Records become readable at the clean stop | the Run starts again from nothing | the same |
| `run_state.commit_interval` | unchanged: the interval of `run_state.batch.sync: interval`, which `periodic` sets | as the level | as the level |
| the file-first Frontier: the visited set and pending work items read from the run file, with only bounded caches in memory | `run_state.memory.budget: 0`, with any level | as the level | as the level |
| the run-private Politeness (`politeness.sharing: run`) | the same setting: the same caps, delays, backoff and 429 halving, held in memory rather than in an in-memory SQLite file | nothing is lost: the Permits were never in the run file | the same |
| the shared Politeness: the per-user file (`shared`, `per_daemon`) and the Politeness store | unchanged: `politeness.sharing: shared` or `per_daemon`, with the store set as ADR 0053 decides | unchanged, as ADR 0053 decides | unchanged |

### What the outside is told, at `batch`

- **`run.finish`'s answer, a Checkpoint and a clean stop's answer come only after the last batch is written and synced.**
- **A Record becomes readable through `run.records`, a Sink or the SQL tab once its batch is written,** about one `run_state.batch.interval` after its page ended at the default, and `run_state.batch.interval` sets that delay. At `sync: each`, SQLite shows a batch to other connections only once it is synced, so a reader never sees a Record that a crash can take back. A reader never forces a batch (ADR 0103: a reader never slows the crawl), and no reader reads Records from memory.
- **Below `sync: each`, a reader and a Sink see each batch once it is written, before its sync,** as at `fast` and `periodic` today (ADR 0138's note on ADR 0103). A power cut can then take back a Record a Sink already delivered, and `weakened` says so. At `off` that Record is lost with the whole run file. Holding Sinks back until a sync was not adopted (owner, 2026-11-06, [#2145](https://github.com/goodfetch/goodfetch/issues/2145)): it delays delivery by up to one `run_state.commit_interval` at `periodic` and until the clean stop at `fast`.
- **An enqueue's answer to a handler, `run.progress` counts and a Provenance line** are told from memory, as early as today. Each promises nothing about a crash, as ADR 0138's note already says.
- **An Effect's journal entry is a barrier at every level** (ADRs 0003 and 0010). The batch that holds it is sealed and written before the Effect happens, and it is synced unless `run_state.batch.sync` is `never` or `off`. An Effect never happens twice after a resume.
- **The Usage ledger** keeps its own file and syncs (ADR 0138's note of #2110). After an unclean end below `full`, its reclaim charges each Slice whole, as ADR 0138's note of #1930 already decides. The bound of 64 deferred charges that forces a sync applies at `full` only.

### A clean stop writes everything

- **A clean stop drains first, then writes.** A clean stop is `run.pause`, an abort, a migration, a Run's end, a Client's end that stops its Runs, or ADR 0122's stop before a full disk. It admits nothing new and lets each attempt in flight end under its Lease. An attempt that the stop cancels returns its work item to pending, unfinished. Then the stop seals one last batch with every change memory holds, writes it, syncs it, and clears the "unclean" mark at every level.
- **A Run resumed after a clean stop redoes no attempt that had ended.** It fetches again only the attempts the stop cancelled.
- **At ADR 0122's `critical` storage level,** the last batch is still written to the run file, since it is small and the run file must resume the Run, but no Checkpoint copy is written. When that write fails, the Run resumes from its last written batch and redoes at most one batch, which the resumed Run's report states.
- **A Checkpoint** (signals, the periodic interval, a migration, a Partition handoff) seals and writes the current batch first, then copies the run file as ADR 0002's note of #218 decides.

### Memory stays bounded: the spill

- **`run_state.memory.budget`** is the most memory a Run's in-memory state may hold: 64 MiB by default. It is a per-Run setting, like the batch keys, and an Operator policy may cap or lock it. The actor counts the bytes it holds as it inserts and removes, and the ops listener exposes the count (ADR 0050).
- **Past the budget, the Run spills.** It removes from memory the visited-set entries and pending work items that a written batch already holds, coldest first. It keeps what it needs at once:
  - every Lease and change not yet written;
  - the head of each per-domain queue, up to ADR 0138's 64;
  - the learned rules and the tier choice.

  From then on, a visited-set lookup that misses in memory reads the run file on the kept read-only connections, one read transaction per links batch. A per-domain queue refills from the run file as it does today. The check stays exact.
- **Under memory pressure, the Run spills too.** It uses the Resource governor's levels (ADR 0040), which read the process's own cgroup and the host, where Spider reads only the host:
  - at `pressure`, the in-memory state stops growing, and new entries go to the run file;
  - at `critical`, it spills down to what it needs at once.

  Pressure gone, it grows again.
- **Changes not yet written count against the budget.** When every threshold is `none`, nothing else bounds them: past the budget, or at the governor's `pressure`, the Run writes them in a batch of its own, synced as `run_state.batch.sync` says, before it removes anything. A spill likewise writes any entry it removes that no batch has written yet.
- **`run_state.memory.budget: 0`** keeps nothing in memory but what is needed at once. Every lookup then reads the run file, which is today's file-first behaviour and its memory profile.

### A run-private Politeness keeps its Permits in memory

- **With `politeness.sharing: run`, `goodfetch-engine`'s `politeness` keeps the Run's Permits, domain limits, delays, backoff, `Retry-After` and 429 halving in an in-memory store, which the Run state's actor calls** (owner, 2026-11-06, [#2142](https://github.com/goodfetch/goodfetch/issues/2142)). This replaces the in-memory SQLite file of ADR 0053's note of #239 and the actor that owned it. Admission takes a Permit with no SQLite, no transaction and no thread hop.
- **`politeness` stays the only owner of Politeness permits** (ADR 0140's E6). The same rules (caps, delays, 429 halving, `Retry-After`) live in one place and serve the in-memory store and the shared ones alike.
- **The shared Politeness is unchanged.** The per-user Politeness file (`shared` and `per_daemon`) and the Politeness store stay as ADR 0053 decides, since other processes share them.

## Considered Options

- **Keep the file-first design and optimize it** (spec #1838's tickets): rejected as the whole answer. The measurement puts raw at about 1.8 M instructions a page after them, about 4.5 times Spider's, with the serial actor chain unchanged. Those tickets stay useful, and this ADR builds on them.
- **Memory-first only at `off`, with `full` and `fast` file-first as before:** rejected. It keeps two models of the Frontier, each with its own tests, and leaves the default as slow as today.
- **A separate append-only log as the journal, with the run file rebuilt from it** (rqlite's design, rejected in ADR 0002): rejected again. It brings a second format to every reader, Checkpoint and import. Batches of the same tables give the same grouping.
- **Dropping `fast` and `periodic`, with `batch`'s settings covering them:** not adopted. The owner said on 2026-11-06 that no feature regresses, and that a feature may at most move under a setting the user can set. Each level now names a preset of the batch settings. The owner confirmed all five levels on 2026-11-06 ([#2145](https://github.com/goodfetch/goodfetch/issues/2145)).
- **State shared by the pages' tasks under locks,** changed inline by each page with no actor: not adopted (owner, 2026-11-06, [#2142](https://github.com/goodfetch/goodfetch/issues/2142)). It breaks `STYLE.md`'s rule that state with one owner is an actor with no `Mutex`, while one asynchronous round trip a page already carries everything the page needs from the Run state.
- **Building the tree on the page's task:** not adopted ([#2142](https://github.com/goodfetch/goodfetch/issues/2142)). A tree format's parse is heavy CPU work that would block an async worker; it takes one hand-off to the analysis pool, and the raw pass none.
- **Spider's thresholds: 15,000 links, and 50 % and 80 % of host memory:** rejected. A count of links ignores how large each entry is, and host memory overshoots a container's limit (ADR 0040). A byte budget and the governor's levels bound the same thing more exactly.
- **A Bloom filter in front of the spilled visited set:** not adopted now. It would keep the check exact, since a "no" is certain and a "maybe" reads the file. But it adds memory of its own, and no measurement yet shows spilled lookups to be the cost. Left to a measurement.
- **Forcing a batch when `run.records` reads:** rejected. It lets a reader slow the crawl, against ADR 0103.
- **Reading Records from memory before their batch is written:** rejected (owner, 2026-11-06, [#2145](https://github.com/goodfetch/goodfetch/issues/2145)). It adds a second read path, and a reader could see a Record that a crash takes back, against ADR 0103's "lossless because durable". The delay is set with `run_state.batch.interval` instead.
- **`off` writing nothing until a clean stop** (this ADR's first draft): not adopted as the level's preset (owner, 2026-11-06, [#2145](https://github.com/goodfetch/goodfetch/issues/2145)). A crawl at `off` would show no Record and deliver nothing to a Sink before its end, which `off` does today, and its unwritten Records would grow in memory with no bound. It stays reachable by setting the three thresholds to `none`.
- **A key set in any layer overriding a level's preset:** rejected (owner, 2026-11-06, [#2145](https://github.com/goodfetch/goodfetch/issues/2145)). An Operator policy's lock on `run_state.durability: full` would not hold against a user's `run_state.batch.sync: never`, and a lower layer's key would override a level a higher layer chose. Forbidding keys beside a level was rejected too: a level is a starting point (spec #2138's story 57).
- **A setting choosing whether Sinks wait for a sync** (`sinks.deliver: written | synced`): not adopted now. At the default the two are the same, and `full` is one setting away. It can be added later without breaking anything.
- **Default `full`:** rejected by the owner's decision of 2026-11-06. It is what made the default crawl about 12 times slower than Spider's.
- **Batches of 500 actions or 5 s, StormCrawler's, or 60 s, Crawlee's:** 1,000 pages, 1 s or 8 MiB, synced at each batch, was chosen by the owner on 2026-11-06 ([#2144](https://github.com/goodfetch/goodfetch/issues/2144)). A crash then loses about a second, the bound Redis's `appendfsync everysec` and MySQL's `innodb_flush_log_at_timeout` keep by default, while one transaction and one sync a second cost nothing measurable. A tighter 250 ms was not adopted while its cost is unmeasured. Each value is a setting.
- **A memory budget that is a share of the governor's allowance, or 256 MiB:** not adopted ([#2144](https://github.com/goodfetch/goodfetch/issues/2144)). A fixed 64 MiB holds about 700,000 URLs at the density ceiling of 95 bytes a URL, against Spider's 15,000 links, and gives the same figures on every machine; a share would not. Spilling already at `pressure` rather than stopping growth there was not adopted either: it reads the disk sooner for memory the governor has not yet asked back.

## Consequences

- **Amends ADR 0002.** Run state lives in memory, and the run file is written in Run-state batches. "An acknowledged commit survives a power cut" holds at `full`. The default is `batch`. The portable file, the single writer, Checkpoints and resume elsewhere are unchanged.
- **Amends ADR 0140** (owner, 2026-11-06, [#2142](https://github.com/goodfetch/goodfetch/issues/2142)). F3, Frontier admission, gets a new owner module, the crawl's Run state in `goodfetch-crawl`, which also owns claims and Leases. F2 stays the `run_state` writer's and, at `batch` and `off`, is counted per Run-state batch: 0 transactions and 0 waits on the run file per page, 1 transaction per batch. `full`, `fast` and `periodic` keep F2's contract unchanged. A new counted row holds a page to at most 1 round trip to the Run state's actor, 0 to the writer and 0 to a run-private Politeness. `politeness` stays E6's only owner, and at `politeness.sharing: run` its in-memory store counts 0 SQLite and 0 transactions per Permit; the shared Politeness file and store keep E6's contract. A tree format's hand-off to the analysis pool is counted on a row of its own, which the extraction-path spec adds. No row is loosened, and each new row joins the Quality register with the ticket that builds what it counts.
- **Amends ADR 0138's Durability levels.** `run_state.durability` names a preset of the batch settings and gains `batch`, the new default. `full`, `fast`, `periodic` and `run_state.commit_interval` keep their meaning. `off` now writes the same batches as `batch`, never synced (`run_state.batch.sync: off`), so its Records stay readable while it crawls and its memory stays bounded. ADR 0138's grouping, two writer connections and checkpointer serve every level.
- **Note on ADR 0003.** At `batch` and `off`, a Lease lives in memory. A late commit is refused against it, as today, and the run epoch still fences another engine. The run fencing model's TLA+ variables and `goodfetch::lease` events are unchanged. The model gains the batch boundary as the state a crash returns to.
- **Note on ADR 0028.** A Durability level unfolds into its preset's keys at the layer that set it, and a lock on the level locks those keys.
- **Note on ADR 0103.** A Record is readable once its batch is written, about one `run_state.batch.interval` after its page at the default, and never earlier from memory. Below `sync: each`, a Record a reader saw can be lost to a power cut, which `weakened` labels, as at `periodic` today.
- **Note on ADR 0053.** A run-private Politeness keeps its Permits in `politeness`'s in-memory store, which the Run state's actor calls, with no SQLite.
- **Note on ADR 0060.** The Run state's actor is a task of the Run's owner, and the writer stays its dedicated thread through `tasks`. The batch timer reads the `Clock`. Spilled reads run on the owner's blocking threads. No runtime, thread or socket is added outside the rules. The crash VFS and LazyFS layers gain each level's crash point: inside a batch, between batches, and inside a clean stop's last batch.
- **ADR 0134's ceiling of 8 heap bytes per URL in the Frontier, at 100,000 URLs, is kept, measured at `run_state.memory.budget: 0`** (the owner's decision of 2026-11-06, [#2143](https://github.com/goodfetch/goodfetch/issues/2143)). A Run holds up to its budget in memory before it spills, so at the default budget its memory may rise by up to the budget over the URLs before the spill flattens it, and bounded growth is measured past the spill. Three rows join the Quality register with the ticket that adds the spill: heap growth from 100,000 to 1,000,000 URLs at a 1 MiB test budget, at most 8 bytes per URL, with ADR 0134's Frontier at scale run at budgets 0 and 1 MiB (peak RSS at 1,000,000 at most 1.5 times its peak at 100,000); the Run state's heap at the default budget and 1,000,000 URLs, at most the budget plus 10 %; and heap bytes per URL before any spill, at 100,000 URLs and the default budget, at most 95 (Scrapy's seen set), tightened by the ratchet after the optimization pass.
- **`STYLE.md`'s rule "The Frontier keeps in memory only bounded caches" changes** with spec #2138's first ticket, since spec #1577's branch rewrites the same lines ([#2145](https://github.com/goodfetch/goodfetch/issues/2145)). It becomes: the Frontier's memory is bounded by `run_state.memory.budget`, and the duplicate check stays exact.
- **`GLOSSARY.md`,** with spec #2138's first ticket for the same reason: the Durability level's definition gains `batch`, and each value is a preset of the batch settings. **Run-state batch** joins it: the changes to a Run's state since the last batch, written to its run file in one transaction. _Avoid_: flush, journal (an Effect journal is another thing), snapshot.
- **The comparison matrix:** scenario 1's Foxhound cell runs at the default level, `batch`. Scenario 2 gains a `batch` cell at the default settings, which carries the durable cell's bar: it must beat Crawlee's default `persistStorage` (the owner, 2026-11-06, while charting [#2141](https://github.com/goodfetch/goodfetch/issues/2141)). Its `full` cell and its `periodic` cell at 60 s stay, measured without a bar.
- **Contracts change in place:** `run_state.durability` gains `batch`, its new default, and the new per-Run settings `run_state.batch.per_attempt`, `.pages`, `.interval`, `.bytes`, `.sync` (`each`, `interval`, `never` or `off`) and `run_state.memory.budget` are added to `goodfetch-protocol`'s types and regenerated with `xtask codegen`.
- **Implemented by** spec [#2138](https://github.com/goodfetch/goodfetch/issues/2138), which follows spec #1577.
