# A Run's state lives in memory, its run file is written in batches, and a clean stop writes everything

Status: proposed, 2026-11-06, pending the owner's reading of the spec that implements it. It amends ADRs 0002 and 0138 and touches ADRs 0003, 0053, 0060 and 0134, as listed under Consequences.

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

- **A Foxhound Run's state lives in memory, owned by one actor.** The crawl's delivery loop owns the Frontier's work items, the visited set, the per-domain queues, the Leases and a run-private Politeness's Permits. Every page reads and changes this state with no run-file read or write on its path. A page's task sends its outcome and its keyed links to the actor over a bounded channel. URL keying, Scope and Payload checks stay pure work on the page's task. The actor only checks the visited set, inserts and leases.
- **State the crawl only writes waits for the next batch.** This covers outcomes, attempt errors, Records, Provenance marks, learned rules, the tier choice and Session scores. The crawl reads the last three back from memory.
- **The duplicate check stays exact.** No Bloom, cuckoo or other lossy filter decides that a URL was seen.

### The run file is written in Run-state batches

- **A batch is every change since the last batch, written by the run file's single writer in one transaction.** It is sealed when the first of three thresholds is reached:
  - `run_state.batch.pages`, 1,000 attempts ended by default;
  - `run_state.batch.interval`, 1 s by default, read on the `Clock` (ADR 0059);
  - `run_state.batch.bytes`, 8 MiB of rows by default.
- **One batch is written at a time.** While the writer writes one batch, the next keeps filling. The channel to the writer holds one sealed batch. When a second batch is sealed before the first is written, admission waits, which is backpressure, so the changes not yet written stay bounded.
- **`run_state.batch.sync`,** true by default, syncs each batch. A batch is one sync, so syncing costs about one `fsync` a second.
- **A batch is all or nothing.** The run file holds the Run as of one batch boundary, never half of a page. ADR 0138's savepoint per page leaves the batch path, since a batch is built from changes memory already accepted.
- **The run file keeps its tables and its format.** A batch writes the same rows that today's commits write, so Checkpoints, `run.resume`, `run.records`, the kept read-only connections, Sinks and the per-install layout read it unchanged. This keeps ADR 0002's portable file and rejects a second, log-shaped format.

### Durability levels

`run_state.durability` takes three values. A page never waits on the run file at `batch` or `off`.

| Level | When the run file is written | After a process crash | After a power cut |
|---|---|---|---|
| `full` (opt-in: absolute protection) | each attempt's claim before its fetch, and its outcome synced before anything is told outside, grouped across pages as ADR 0138 decides | only attempts in flight are redone | only attempts in flight are redone |
| `batch` (the default) | one batch per threshold, synced unless `run_state.batch.sync` is false | at most one batch and the attempts in flight are redone | the same when synced; unsynced, recent batches may roll back, without corruption |
| `off` | nothing until a clean stop, except what a spill writes | the run file carries the "unclean" mark, is discarded, and the Run starts again from nothing | the same |

- **`full` keeps every guarantee of ADR 0138's `full`, with the same model.** Memory is still the truth, and the writer is told to write each claim and each outcome as a batch of its own. What `full` costs and keeps is ADR 0138's: at most 2 run-file transactions and 1 sync an attempt.
- **`fast` and `periodic` leave.** Each still commits every page, so it pays the per-page cost without `full`'s protection. `batch` covers what each offered:
  - `fast`: a process crash loses about one page with `run_state.batch.pages: 1` and `run_state.batch.sync: false`;
  - `periodic`: `run_state.batch.interval` bounds a power cut's loss, as `run_state.commit_interval` did, and that setting leaves.

  Nothing is published, so no format changes version (ADR 0066).
- **The default becomes `batch`.** It loses at most about one second of work to a crash, where Spider, Scrapy without `JOBDIR`, and Colly lose the whole crawl, and Crawlee and Scrapy with `JOBDIR` sync nothing. `full` is one setting away and recorded as a choice. ADR 0002's "an acknowledged commit survives a power cut" now holds at `full`. At `batch`, an acknowledgement means the batch that holds it was written (below).
- **`off` weakens a guarantee against the default** (ADR 0028). The run report's `weakened` and `config explain` label it.

### What the outside is told, at `batch`

- **`run.finish`'s answer, a Checkpoint and a clean stop's answer come only after the last batch is written and synced.**
- **A Record becomes readable through `run.records`, a Sink or the SQL tab once its batch is written,** at most one `run_state.batch.interval` after its page ended. A reader never forces a batch (ADR 0103: a reader never slows the crawl).
- **An enqueue's answer to a handler, `run.progress` counts and a Provenance line** are told from memory, as early as today. Each promises nothing about a crash, as ADR 0138's note already says.
- **An Effect's journal entry is a barrier at every level** (ADRs 0003 and 0010). The batch that holds it is sealed and written before the Effect happens, and it is synced except at `off`. An Effect never happens twice after a resume.
- **The Usage ledger** keeps its own file and syncs (ADR 0138's note of #2110). After an unclean end below `full`, its reclaim charges each Slice whole, as ADR 0138's note of #1930 already decides. The bound of 64 deferred charges that forces a sync applies at `full` only.

### A clean stop writes everything

- **A clean stop drains first, then writes.** A clean stop is `run.pause`, an abort, a migration, a Run's end, a Client's end that stops its Runs, or ADR 0122's stop before a full disk. It admits nothing new and lets each attempt in flight end under its Lease. An attempt that the stop cancels returns its work item to pending, unfinished. Then the stop seals one last batch with every change memory holds, writes it, syncs it, and clears the "unclean" mark at every level.
- **A Run resumed after a clean stop redoes no attempt that had ended.** It fetches again only the attempts the stop cancelled.
- **At ADR 0122's `critical` storage level,** the last batch is still written to the run file, since it is small and the run file must resume the Run, but no Checkpoint copy is written. When that write fails, the Run resumes from its last written batch and redoes at most one batch, which the resumed Run's report states.
- **A Checkpoint** (signals, the periodic interval, a migration, a Partition handoff) seals and writes the current batch first, then copies the run file as ADR 0002's note of #218 decides.

### Memory stays bounded: the spill

- **`run_state.memory.budget`** is the most memory a Run's in-memory state may hold: 64 MiB by default. The actor counts the bytes it holds as it inserts and removes, and the ops listener exposes the count (ADR 0050).
- **Past the budget, the Run spills.** It removes from memory the visited-set entries and pending work items that a written batch already holds, coldest first. It keeps what it needs at once:
  - every Lease and change not yet written;
  - the head of each per-domain queue, up to ADR 0138's 64;
  - the learned rules and the tier choice.

  From then on, a visited-set lookup that misses in memory reads the run file on the kept read-only connections, one read transaction per links batch. A per-domain queue refills from the run file as it does today. The check stays exact.
- **Under memory pressure, the Run spills too.** It uses the Resource governor's levels (ADR 0040), which read the process's own cgroup and the host, where Spider reads only the host:
  - at `pressure`, the in-memory state stops growing, and new entries go to the run file;
  - at `critical`, it spills down to what it needs at once.

  Pressure gone, it grows again.
- **At `off`, a spill writes what it removes** in a batch of its own, unsynced, before removing it, since no batch has written it.
- **`run_state.memory.budget: 0`** keeps nothing in memory but what is needed at once. Every lookup then reads the run file, which is today's file-first behaviour and its memory profile.

### A run-private Politeness keeps its Permits in memory

- **With `politeness.sharing: run`, the Run's Permits, domain limits, backoff and 429 halving are plain values in the delivery actor.** This replaces the in-memory SQLite file of ADR 0053's note of #239 and the actor that owned it. Admission takes a Permit with no transaction and no thread hop.
- **The shared Politeness is unchanged.** The per-user Politeness file (`shared` and `per_daemon`) and the Politeness store stay as ADR 0053 decides, since other processes share them.

## Considered Options

- **Keep the file-first design and optimize it** (spec #1838's tickets): rejected as the whole answer. The measurement puts raw at about 1.8 M instructions a page after them, about 4.5 times Spider's, with the serial actor chain unchanged. Those tickets stay useful, and this ADR builds on them.
- **Memory-first only at `off`, with `full` and `fast` file-first as before:** rejected. It keeps two models of the Frontier, each with its own tests, and leaves the default as slow as today.
- **A separate append-only log as the journal, with the run file rebuilt from it** (rqlite's design, rejected in ADR 0002): rejected again. It brings a second format to every reader, Checkpoint and import. Batches of the same tables give the same grouping.
- **Keeping `fast` and `periodic` beside `batch`:** rejected. Each pays a transaction a page for less than `full`, and `batch`'s settings reach what each offered.
- **Spider's thresholds: 15,000 links, and 50 % and 80 % of host memory:** rejected. A count of links ignores how large each entry is, and host memory overshoots a container's limit (ADR 0040). A byte budget and the governor's levels bound the same thing more exactly.
- **A Bloom filter in front of the spilled visited set:** not adopted now. It would keep the check exact, since a "no" is certain and a "maybe" reads the file. But it adds memory of its own, and no measurement yet shows spilled lookups to be the cost. Left to a measurement.
- **Forcing a batch when `run.records` reads:** rejected. It lets a reader slow the crawl, against ADR 0103.
- **Default `full`:** rejected by the owner's decision of 2026-11-06. It is what made the default crawl about 12 times slower than Spider's.
- **Batches of 500 actions or 5 s, StormCrawler's, or 60 s, Crawlee's:** 1,000 pages or 1 s was chosen. A crash then loses about a second, while one transaction and one sync a second cost nothing measurable. Each value is a setting.

## Consequences

- **Amends ADR 0002.** Run state lives in memory, and the run file is written in Run-state batches. "An acknowledged commit survives a power cut" holds at `full`. The default is `batch`. The portable file, the single writer, Checkpoints and resume elsewhere are unchanged.
- **Amends ADR 0138's Durability levels.** The levels are `full`, `batch` and `off`. `fast`, `periodic` and `run_state.commit_interval` leave. `off` writes nothing during the crawl. ADR 0138's grouping, two writer connections and checkpointer stay for `full` and serve batches.
- **Note on ADR 0003.** At `batch` and `off`, a Lease lives in memory. A late commit is refused against it, as today, and the run epoch still fences another engine. The run fencing model's TLA+ variables and `goodfetch::lease` events are unchanged. The model gains the batch boundary as the state a crash returns to.
- **Note on ADR 0053.** A run-private Politeness keeps its Permits in memory, with no SQLite.
- **Note on ADR 0060.** The delivery actor is a task of the Run's owner, and the writer stays its dedicated thread through `tasks`. The batch timer reads the `Clock`. Spilled reads run on the owner's blocking threads. No runtime, thread or socket is added outside the rules. The crash VFS and LazyFS layers gain each level's crash point: inside a batch, between batches, and inside a clean stop's last batch.
- **ADR 0134's ceiling of 8 heap bytes per URL in the Frontier, at 100,000 URLs, needs the owner.** A Run holds up to its budget in memory before it spills, so the figure at 100,000 URLs is the budget over the URLs. The spec proposes measuring the ceiling at `run_state.memory.budget: 0`, and measuring growth past the budget as a new row. Only the owner moves a ceiling.
- **`STYLE.md`'s rule "The Frontier keeps in memory only bounded caches" changes** once the owner accepts this ADR. It becomes: the Frontier's memory is bounded by `run_state.memory.budget`, and the duplicate check stays exact.
- **`GLOSSARY.md`:** the Durability level's definition becomes `full`, `batch` or `off`. **Run-state batch** joins it: the changes to a Run's state since the last batch, written to its run file in one transaction. _Avoid_: flush, journal (an Effect journal is another thing), snapshot.
- **The comparison matrix:** scenario 1's Foxhound cell runs at the default level, `batch`. Scenario 2's `periodic` cell at 60 s becomes `batch` with `run_state.batch.interval: 60s`, the like-for-like cell against Crawlee's `persistStorage`.
- **Contracts change in place:** `run_state.durability`'s values and the new settings `run_state.batch.pages`, `.interval`, `.bytes`, `.sync` and `run_state.memory.budget` are added to `goodfetch-protocol`'s types and regenerated with `xtask codegen`.
- **Implemented by** spec [#2138](https://github.com/goodfetch/goodfetch/issues/2138), which follows spec #1577.
