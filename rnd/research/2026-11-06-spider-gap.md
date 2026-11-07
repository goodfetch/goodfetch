# Why Spider is 12–14× faster than Foxhound in `just compare`'s HTTP cells (2026-11-06)

Measured on 2026-11-06, 15:35–16:55 UTC, on the Hetzner CCX43 (16 threads), at `origin/spec/1577` cc72b0f93 (the tree #1931 measured), in the worktree `.claude/worktrees/gap-http`, release profile (fat LTO). The binaries were built once: Foxhound with `xtask test-tools --cargo-profile release`, and Spider 2.53.9 with `xtask/compare-spider`'s `--locked` build. No product code was changed. Raw outputs, scripts and loads are under `/g/gap-http/`. The browser cell is out of scope, because another agent covers it.

**Answer in one paragraph.** The gap is mostly not product cost. In raw, Foxhound executes about **5× Spider's instructions per page** (1.99 M against 0.40 M, user plus kernel) and **5× its system calls** (61 against 12). It also runs the page through **three single-threaded hops**: the Politeness actor, the analysis pool and the run-state writer. Each hop is a SQLite transaction or a thread wake-up, so it **keeps only about 9–15 requests in flight out of 16, at 1.1–1.7 cores**. Spider keeps about 15 in flight at 2.3–3.7 cores. Those two factors multiply to roughly the 12× wall gap. About 27% of Foxhound's per-page instructions are plain implementation waste that no ticket names. About 35% is SQLite on single actors, which the design requires on every page even at `durability: off` with Politeness non-binding: that part is architectural. The memory gap (117 against 24 MB) is mostly fixed per process, not per page. In extraction, the comparison is not like-for-like: Spider writes raw HTML, while Foxhound extracts the main text, and 58% of Foxhound's instructions there go to the clean-up step. Spec #1838's open tickets recover roughly 10–15% of raw's instructions and none of the serialization.

## 1. Same work?

What each tool is asked to do per page, read from `xtask/src/compare/matrix.rs`, `compare/foxhound.rs:244-290`, `xtask/compare-spider/src/main.rs` and the Scrapy and Crawlee sides:

| Per page | Foxhound (raw / extraction) | Spider (raw / extraction) |
|---|---|---|
| Fetch | `http` tier (reqwest transport, pooled), 16 in flight, egress check | reqwest, 16 permits |
| robots.txt | off (`respect_robots_txt: false`), nothing fetched | off (`with_respect_robots_txt(false)`) |
| Politeness | `sharing: run`, `delay: 0`, `concurrency: 16`: still consulted on every page | none |
| Dedup and frontier | run file (SQLite), `durability: off` | in-memory `HashSet` |
| Parse | one streaming pass for links and head (raw); a tree, clean-up and readability for the `text` format (extraction) | one lol_html streaming pass while receiving |
| Kept | raw: no Record; extraction: a Record with `text` and `title` | raw: nothing; extraction: URL, title and **raw HTML** to `pages.jsonl` |
| Learning, block classification, Provenance | off | n/a |

- **The 1,000 pages:** the same-work check counts pages from the Proving ground's observations, never from a tool's report.
- **Every run here passed it.** Spider fetched exactly 1,000 distinct pages in 1,000 requests and 462,362 bytes, the same as Foxhound, with nothing missing or extra, at 100, 1,000 and 10,000 pages (`*.json-perfstat` → `work`).
- **The extraction cell is not like-for-like.** Spider is asked for no main-text extraction: `main.rs:51` writes `page.get_html()`. Foxhound's figure carries clean-up and readability, which Spider never runs.

## 2. Where Foxhound's time and memory go

### Instructions, cores and system calls

Instructions come from `perf stat` at 100, 1,000 and 10,000 pages. Each figure is the slope between 1,000 and 10,000 pages; "fixed" is the intercept. Cores and wall time come from the timed runs, whose 1-minute load was 17–70: wall times are indicative only.

| Cell | Instr./page (user + kernel) | Fixed per Run | Context switches / page | Syscalls / page | Wall / page, marginal | Cores used |
|---|---|---|---|---|---|---|
| Foxhound raw | 1.99 M (1.60 + 0.39) | 0.25–0.5 G | 13.7 | 61 | 0.46–0.79 ms | 1.1–1.7 |
| Spider raw | 0.40 M (0.34 + 0.05) | 0.085 G | 1.3 | 12 | 0.04–0.06 ms | 2.3–3.7 |
| Foxhound extraction | 8.05 M (7.62 + 0.43) | ≈ 0.5 G | 16.8 | 77 | 0.6 ms | 2.5–3.3 |
| Spider extraction | 0.41 M | 0.083 G | 1.2 | 13 | 0.03 ms | — |

Callgrind, user space, 1,000 pages (the harness's own counted mode):

| Cell | Instructions |
|---|---|
| Foxhound raw | 2.09 G |
| Foxhound extraction | 8.15 G |
| Spider raw | 0.487 G |

That makes **4.30× in raw**. The register's `foxhound.spider_instruction_ratio` still reads 22.799 from before the pass (`TESTING.md:1465`), so it is stale.

**The Proving ground's view** (`scripts/obs.py`, 3 runs each, load about 30):

| | Foxhound raw | Spider raw |
|---|---|---|
| Time to first request | 70–160 ms | 15–28 ms |
| Slot gap (a connection's answer to its next request), p50 | 4.9–7.9 ms | 1.2–2.1 ms |
| Connections in use | 9–14 | 16–17 |

By Little's law, Foxhound holds about 9 requests in flight out of 16.

**Per thread, raw, 10,000 pages, 4.79 s wall** (`threads-cpu.txt`):

- `run-state-write`: 1.67 s;
- `politeness-file`: 1.24 s;
- 16 Tokio workers: 4.58 s in all.

No thread is saturated. The page's critical path crosses the single actors in turn, each crossing a futex wake-up (33 futex calls a page against Spider's 5.5).

### Breakdown of raw's per-page instructions

Callgrind, inclusive, per page. Parts overlap where noted. The class is (a) required by a product promise, (b) implementation that could shrink with no change of behaviour, or (c) architectural.

| # | Part | k instr./page | Share | Class | Notes |
|---|---|---|---|---|---|
| R1 | **A default `CountedWork` built and dropped per links batch** | 377 | 18% | **(b)** | `Positions::of` builds `CountedWork::default()`, a fresh `Arc<Compiled>` of three moka caches (`goodfetch-crawl/src/work_items.rs:444`). `counting()` replaces it at once (`enqueue/links.rs:573`, `work_items.rs:497`), and dropping the moka caches costs 365 k (crossbeam-epoch flush and collect, 171 k of memcpy). **No ticket names it.** |
| R2 | Frontier admission of the page's links, the rest | 173 | 8% | (a)+(b) | `work_items::admit` 155 k, plus the Scope, Payload rules and Fragments read once per batch, 16 k. Deferred to the commit, so it runs once in raw. |
| R3 | Run-state writer, the rest: claim, lease, outcome (`work_items::commit` 98 k) and SQLite | ≈ 290 | 14% | **(c)** | One writer thread for every page, at `durability: off` too: 8.2 WAL `pwrite`s and 2.8 lock `fcntl`s a page, 14 KB a page written to disk. |
| R4 | Politeness permit: take 161 k and give back 166 k on the `politeness-file` actor | 373 | 18% | (b) + **(c)** | An in-memory SQLite (`goodfetch-engine/src/politeness.rs:435-441`) and about 9 statements prepared per permit (`taken_room`, `in_flight` ×2, `next_turn`, `generation`, `execute`s): 190 k of `sqlite3Prepare`. The cap of 16 equals the handler concurrency and the delay is 0, so it can never bind. |
| R5 | Decode and the one streaming pass (`Decoded::of` 201 k, of which lol_html is 178 k) | 201 | 10% | (a) | ADR 0119 decoding; ADR 0138's one streaming pass. The body is buffered whole and then read (`goodfetch-extractor/src/stream.rs:85`): 14 k a page goes to building the rewriter. |
| R6 | The attempt task: HTTP send, egress, response (`On::on_http` minus R5) | ≈ 190 | 9% | (a)/(b) | Spider's whole fetch path is about 70 k (hyper and reqwest). |
| R7 | **Progress of in-flight requests: serialized to JSON 32 times a page** (`progress::InFlight::show`, `goodfetch-crawl/src/progress.rs:267`), plus the publish task | 124 + 68 | 9% | **(b)** | **No ticket names it.** |
| R8 | The delivery loop (`drive`/`deliver`: leases, scheduling) | 136 | 7% | (b)/(c) | |
| R9 | Periodic: needs-you's Run-index opens (795 opens a Run), governor, Usage ledger | ≈ 95 | 5% | (b) | Time-driven, so inflated under Callgrind; covered by #1886, #1888, #1889. |
| — | Fixed per Run: start, `hello`, `crawl.start`, finish | 0.25–0.5 G a Run | — | (a)/(b) | 70–90 ms to the first request, against Spider's 10–28. |

**System calls a page** (`fh-raw-1000.straceall`, by target file):

| What | Per page |
|---|---|
| futex (actor hand-offs) | 33 |
| `pwrite64` to `run.sqlite-wal` | 8.0 |
| `fcntl` on `run.sqlite-shm` | 2.4 |
| **Usage ledger existence probes** | 3 `newfstatat`, 2 `openat`, 1 `statx`, all ENOENT |
| `writev` and `recvfrom` | 1 + 1 |

- **The ledger probes come from `Ledger::open_existing`/`absent`** (`goodfetch-engine/src/ledger.rs:316-331`), once per charge, with no Limit set.
- **No counter sees them.** `ChargeOpens` counts only an open that succeeded (`ledger.rs:345`), so `E3.opens` shows 0.125 while about 6 failing calls run per page.

**Extraction** (8.15 M a page):

| Part | Per page | Share |
|---|---|---|
| The `formats` analysis | 5.69 M | 70% |
| Clean-up (`goodfetch_clean_up_rules::removal::remove`) | 4.73 M | 58% |
| └ `Matcher::match_element` (7,968 calls) | 2.51 M | |
| └ cached selector lookups (`rules_selector`, 613 calls) | 0.81 M | |
| └ the page's cosmetic rule list, built per page (`PageRules::of` + `css_rules`) | 1.10 M | |
| Readability (dom_smoothie) | 0.78 M | |
| Run-state writer | 0.78 M | |
| Politeness | 0.33 M | |

- **Clean-up** is (a), required by ADR 0047 and by the `text` format's main content. Its cost is (b): the rules depend only on the host and the Run, and every rule is tested against every element instead of being indexed by id, class or tag.

### Memory

Peak RSS from `/proc/<pid>/smaps` at the 10 ms sample with the largest RSS (`*.smaps`, `*.status`):

| Run | Peak | Executable's own pages | Heap | Other |
|---|---|---|---|---|
| Foxhound raw, 1,000 pages | 115–118 MB | 42 MB of the 158 MB binary | mimalloc arenas 65 MB (66.4 MiB committed at peak, `MIMALLOC_SHOW_STATS`) | 8 MB |
| Foxhound, 10 pages | 86 MB | 40 MB | 36 MB | |
| Spider raw, 1,000 pages | 25 MB | 6.7 MB | 16 MB anon | |

- **Foxhound's peak is flat with site size:** 98 MB at 100 pages, 116 MB at 1,000 and 114 MB at 10,000. Its memory is fixed per process and per Run, plus the working set of 16 in flight.
- **Spider's grows with the site:** 18, 24 and 60 MB. At 10,000 pages the ratio is 1.9×.
- **Classification:** the 42 MB of the executable's own pages follow ADR 0057's one binary with every capability (a/c). The 36–65 MB of heap is not broken down: mimalloc hides it from DHAT and heaptrack, so it needs a `bench-system` build. That part is unverified.

## 3. Classification summary (raw)

| Class | Share of raw's 2.09 M/page | Parts |
|---|---|---|
| (a) product promise | ≈ 25% | R5, part of R6, part of R2 |
| (b) implementation | ≈ 45% | R1, R7, R4's prepares, R8, R9, the ledger probes |
| (c) architectural | ≈ 30% | R3, R4's actor and its in-memory SQLite, the futex chain |

On wall time, (c) weighs more than its instruction share. It is what keeps Foxhound at about 9 requests in flight and about 1.4 cores: with 5× the work per page and about 2.5× less parallelism, the product is the 12× gap.

The architectural point: at `durability: off`, a crash discards the run file anyway (`docs/throughput.md`). The only promise left during the crawl is a resumable clean stop (ADR 0138). Yet every page still takes three SQLite transactions on the writer, and the Politeness permit is a SQLite transaction on a second actor while it can never bind.

## 4. Mapping to the tickets

What spec #1577 already did (reports in `spec-1577/reports/`):

- one parse and the streaming pass (#1910, #1911);
- the demand plan (#1909);
- selectors compiled once (#1912);
- prepared run-file statements (#1907);
- grouped commits and the Durability levels (#1924–#1927, #2117);
- Politeness switched off when it can be (#1920);
- the allocator purge (#1928).

Together they took raw from 11.0 M to 2.09 M instructions a page (t1904 against this run) and the matrix from 440 to 995 pages a second.

Spec #1838's open tickets against the parts above:

| Ticket | Part | Raw saving, estimated |
|---|---|---|
| #1899 (Politeness statements, ≤ 2 transactions) | R4's 190 k of prepares | ≈ 150 k |
| #1896 (Run invariants built once) | part of R2 | ≈ 20–40 k |
| #2069 (staged admission not run twice) | not reached in raw: admission is deferred and runs once | 0 here |
| #1886, #1888, #1885, #1887, #1889 (stores open once, charge without SQL, governor) | R9 and the ledger probes | ≈ 50–90 k, and about 6 syscalls a page |
| #1897 (Record copies) | extraction only, `dataset` 67 k | small |
| #1898, #2066, #2071 (RPC and CDP encoding) | not reached in raw: 11 k a page of RPC | 0 |
| #1890–#1895 (Run trace, CDP, browser) | Provenance is off, no browser | 0 |
| #1934, #2106 (lint, `tokio::fs`) | no direct saving | 0 |

- **After them, raw is still about 1.8–1.85 M instructions a page,** about 4.5× Spider.
- **The serial actor chain is unchanged,** so the wall gap shrinks by perhaps 10–15%.
- **No ticket touches** R1, R7, R3, R4's actor, the clean-up's matching cost, or the fixed memory.

## 5. The codebases side by side: one page, Spider 2.53.9 against Foxhound at cc72b0f93

Spider's source is `~/.cargo/registry/src/index.crates.io-*/spider-2.53.9/`, at the version `xtask/compare-spider/Cargo.lock` pins, with features `basic` and `sync`. `basic` turns on `disk` (sqlx), `balance`, `cookies`, `ua_generator` and the rest of the `__basic` features (`Cargo.toml`). The Foxhound paths are under `crates/`.

| Per page | Spider does | Foxhound does | Difference is |
|---|---|---|---|
| HTTP client and reuse | reqwest with its default keep-alive pool, built once (`website.rs:3024-3070`); the concurrency is a `tower` `ConcurrencyLimitLayer` (`website.rs:3249`) | reqwest transport for plain HTTP, pooled and new clients (`fetch-tier/src/transports.rs:355-369`); wreq with BoringSSL emulation for TLS (`transports/impersonated.rs:356-394`); 9–15 connections used | implementation (same pooling); the TLS fingerprint is a promise (ADR 0035) |
| Admission and dedup | `is_allowed` looks the link up in an in-memory `HashSet` (`website.rs:2110-2130`); `insert_link_unchecked` inserts it (`website.rs:1948-1976`), kept in memory below 15,000 links (`LINKS_VISITED_MEMORY_LIMIT`, `website.rs:1023`) and spilled to SQLite only above that or under memory pressure | every link batch goes to the run file in SQLite through the single writer: `defer_links` (`goodfetch-crawl/src/enqueue/links.rs:235`), `work_items::admit` (`work_items.rs:379`), then the outcome's commit (`lease.rs:257`, `run_state.rs:1382`), at every Durability level | **architecture**: ADR 0002/0138 resumability. At `off`, only a clean stop needs a run file, so an in-memory frontier checkpointed at a clean stop would keep the promise |
| Task model | the crawl loop draws links from a stream and spawns one task per link into a `JoinSet` under a semaphore (`website.rs:8325-8415`, `utils/mod.rs:10010`); the task fetches and parses in place, with no hop (`website.rs:8663`) | a Tokio task per attempt, plus hops to the `politeness-file` actor (`politeness.rs:992`, `:1102`), the analysis pool (`goodfetch-engine/src/tasks.rs:147`) and the `run-state-write` actor (`tasks.rs:204`): 33 futex calls a page against 5.5 | **architecture** (actors own SQLite connections) |
| Politeness | none asked: the static delay is 0 (`website.rs:8363`), and auto-throttle only adds a delay (`:8371`) | a Permit from an in-memory SQLite under the shared store's rules (`politeness.rs:26`, `:435-441`), about 9 statements and 2+ transactions per page | the promise is that a cap is obeyed (ADR 0053, 0086); a cap of 16 at concurrency 16 with delay 0 could be skipped (implementation), and a run-private permit store need not be SQLite (architecture) |
| robots.txt | off | off; nothing fetched (#1919) | same |
| Parse and link extraction | lol_html streaming handlers run on each chunk as it arrives (`page.rs:5164-5229`, `build_link_extract_handlers` `page.rs:4266`), with links pushed into the task's `HashSet` (`page.rs:2878`); 62 k instructions a page in lol_html | the body is buffered whole, then decoded, then one lol_html pass (`fetch-tier/src/tiers.rs:327`, `:494`; `goodfetch-extractor/src/stream.rs:85`), whose rewriter is built per page (14 k); 178 k a page | implementation (both are one streaming pass; Foxhound's happens after the body and does more: head, `<base>`, AI-use signals) |
| What is kept and where | the body as a `Vec<u8>` in the `Page` (`page.rs:5210-5240`), broadcast to the subscriber (`website.rs:15080`), and spooled to disk only under `balance`'s memory pressure (`:15094`) | no body kept in raw; a Record in the Dataset in extraction; claim, lease and outcome rows in the run file, 14 KB a page written | the run file is architecture (as above); the Records are a promise |
| Output | one JSON line per page in extraction (`main.rs:51-58`), with no text extraction | `text` through clean-up and readability, plus a Record validated and stored | product promise (ADRs 0007, 0047); **not the same work** |
| Progress | none | in-flight requests serialized to JSON 32 times a page (`goodfetch-crawl/src/progress.rs:267`) | implementation |
| Bookkeeping | none | Usage ledger probe per charge (`goodfetch-engine/src/ledger.rs:316-331`), governor samples, Run-index opens by needs-you | product (ADR 0041 Limits, 0040 governor); the cost per page is implementation (#1886, #1888, #1889) |
| Allocator | none set: glibc malloc (no `#[global_allocator]` in `src/`, nor in the harness's `main.rs`) | mimalloc wrapped in `Counting` (`goodfetch-foxhound/src/main.rs:14-15`), 1 GiB reserved and 66 MiB committed at peak | implementation |
| Process and protocol | one process, nothing on stdout until the end | an Attached engine over JSON-RPC: `hello`, `crawl.start`, `run.finish`; no message per page in a code-free crawl (rpc is 11 k a page) | promise (ADR 0024), not a per-page cost |
| Binary | 15.8 MB, 6.7 MB of it resident | 158 MB, 42 MB of it resident | ADR 0057 (one binary with every capability) |

**What Foxhound could adopt without breaking a promise:**

- **Fix R1:** pass the batch's `CountedWork` to `Positions::of`, or make its placeholder hold no `Compiled`. Expected saving: 18% of raw's instructions.
- **Cut R7:** publish in-flight progress on change, or on read, rather than serializing it 32 times a page. Expected saving: about 9%.
- **Skip a Politeness permit that cannot bind,** and keep a run-private Permit store in memory with no SQLite. Both keep ADR 0086's promise: any cap set still holds.
- **Probe the Usage ledger once per engine,** and count the failed opens.
- **At `durability: off`, keep the frontier and the leases in memory,** and write the run file only at a clean stop or Checkpoint. A crash discards the file at `off` anyway. This changes ADR 0138's commit design, so it needs an owner decision.
- **Run lol_html on the chunks as they arrive.**
- **Clean-up:** build the cosmetic rule list once per host for each Run, and match only the rules whose key (id, class or tag) occurs in the page, as adblock's own cosmetic cache does.
- **Not open to change by promise:** robots.txt and Politeness when asked, the egress check, Records and their validation, ADR 0047's clean-up for formats, and the one binary.

## 6. Recommendation

1. **File two quick tickets now,** each with a counted row on B1:
   - R1 (the `CountedWork` placeholder): −18% of raw's instructions;
   - R7 (progress serialization): −9%.
2. **Add a counter for the failed ledger opens** (`E3.opens` misses them).
3. **Correct the comparison's extraction cell.** Either Spider extracts the same text (main content and title), or the cell is labelled raw HTML against main text. Today it compares unlike work.
4. **The rest needs new thought, not tickets.** One question for the owner: should `durability: off` and a non-binding, run-private Politeness keep the page off the SQLite actors entirely? An in-memory frontier and Permits, persisted at a clean stop, would be an ADR 0138 amendment. It is the only change that addresses the serialization behind most of the wall gap. After #1838 as filed, expect about 4.5× Spider's instructions a page and still about 10× its wall time.
5. **The memory gap** is mostly fixed per process. Profile the 36–65 MB heap on a `bench-system` build before planning anything.

## Commands run

Every Bash call was prefixed "gap http:". The timed runs ran through `load-guard` (`LOAD_GUARD_HEAVY=1`).

- **perf:** installed as `linux-tools-7.0.0-38-generic`, free, through apt. `kernel.perf_event_paranoid` was set to -1 only for the perf runs, under a `trap`, and restored to 4 (`paranoid.log`).
- **Disk:** the volume filled at 16:10 UTC and failed the first build. The target was moved to `/Users/jb/agent-targets/gap-http` (`build.log`, `build2.log`).

| Step | Command |
|---|---|
| Build | `load-guard bash -c 'cargo run --bin xtask -- test-tools --cargo-profile release …; cargo build --locked --manifest-path xtask/compare-spider/Cargo.toml --profile release …'` |
| One trial | `scripts/trial.sh <scenario> <cell> <exe> <pages> timed\|callgrind [out]`: starts the Proving ground as `xtask/src/rss/crawl.rs:84` does (seed 1574), runs `xtask compare trial` and, with `OBS=`, saves `/observations` |
| perf stat | `scripts/counted.sh <tag> <scenario> <cell> <exe> perfstat 100 1000 10000` (`scripts/wrap.sh`: `perf stat -e instructions:u,instructions:k,cycles,task-clock,context-switches,…`); the Records reader's second engine goes to `*.reader.*` |
| System calls | `counted.sh … strace` (`strace -f -c`) and `stracefull` (`strace -f -tt -y`); the per-file tallies come from `fh-raw-1000.straceall` |
| Callgrind | `trial.sh raw foxhound <exe> 1000 callgrind fh-raw-1000.callgrind`, the same for extraction and Spider; `callgrind_annotate --inclusive=yes` (`--tree=caller` and `--tree=calling`); `scripts/cg_parts.py`, `cg_callers.py`, `cg_callees.py` |
| perf record | `counted.sh … record` (DWARF): the thread split only, since user frames did not unwind (`fh-*-1000.self.txt`) |
| Memory | `counted.sh … smaps` (the largest-RSS sample's `/proc/<pid>/smaps` and `status`); `MIMALLOC_SHOW_STATS=1` (`mi-raw-1000.err`) |
| Threads | `counted.sh … tasks` (`/proc/<pid>/task/*/stat` every 20 ms; `threads-cpu.txt`) |
| Timed | `load-guard scripts/timed.sh` (3 rounds × 100, 1,000 and 10,000 pages × both tools × raw and extraction, loads in `timed/*.load`); `scripts/obs.py` on `obs/*.json` (`obs/summary.txt`) |

`just counts` was not re-run. The B1 to B3 rows quoted are the register's Today figures at this tip (`TESTING.md` § Counted work), measured by #1881 and the tickets of the pass.
