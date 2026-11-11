# Where does Foxhound's peak memory go?

Date: 2026-11-06. Ticket: [#2150](https://github.com/goodfetch/goodfetch/issues/2150), on the map [#2141](https://github.com/goodfetch/goodfetch/issues/2141). It follows the memory section of the Spider gap study (`.claude/handoff/wiki/research-spider-gap-20261106.md`, not in the repository), which left the heap unexplained.

**Answer in one paragraph.** At 1,000 pages and 16 in flight, Foxhound's raw cell peaks at **102–106 MiB**. Only **4.6 MiB of that is live heap**. About **61 MiB is mimalloc's arena**, and **about 45–50 MiB of it is there only because mimalloc asks Linux for transparent huge pages**. With `MIMALLOC_ALLOW_THP=0`, the same binary peaks at **55.5–55.8 MiB** and is flat from 100 to 10,000 pages (52.8 to 57.2 MiB). The executable's own pages are **36.6 MiB of the 158 MiB file**: 27.6 MiB of code that actually runs, 5.1 MiB of relocation fix-ups and 3.6 MiB of read-only data. DuckDB is 47.8 MiB of the code but only 0.6 MiB of what is resident, plus about 1.2 MiB of the fix-ups. Shared libraries add 4.3 MiB and thread stacks about 4 MiB. Almost all of this is fixed per process or per Run. The live heap is flat: 3.9–4.6 MiB at every site size and concurrency measured. Spider's 23 MiB is a 5.5 MiB binary, a live heap of 13.3 MiB that grows with the site (44 MiB at 10,000 pages, mostly HTTP read buffers), and glibc's arenas. So the gap is not data Foxhound keeps. It is one allocator option (reducible, no ADR change) and the size of the executable that runs (mostly not ADR 0057's capabilities: DuckDB's resident share is about 2 MiB).

## Setup

- **Tree:** `origin/spec/1577` at `ad3a67458`, the branch with `just compare`'s raw cell (PR #2074). The earlier study measured `cc72b0f93` on the same branch. The 11 files between them change no allocation path that this note relies on. `main` (`b360ab6b1`) has no raw cell, so it was not measured.
- **Host:** the Hetzner CCX43 (16 threads, 64 GB, Linux 7.0.0-38). Transparent huge pages: `enabled = madvise`, `defrag = madvise`. Other agents shared the host, and every build and measurement ran through `load-guard` (`LOAD_GUARD_HEAVY=1`).
- **Builds**, release profile (fat LTO), feature `executable`:
  - `fh-mimalloc`: Foxhound as it ships, 158.5 MB.
  - `fh-system`: a measurement-only variant with `Counting<std::alloc::System>` in place of `Counting<mimalloc::MiMalloc>` in `crates/foxhound/goodfetch-foxhound/src/main.rs`. The edit was reverted after the build and is in no commit. The repository has no `bench-system` feature or profile; `heap.rs` benches use the system allocator, but no executable does.
  - `xtask/compare-spider` (Spider 2.53.9), release.
- **Trials:** `xtask compare trial --scenario raw --cell foxhound|spider --mode timed`, against a fresh Proving ground (seed 1574), driven by `trial.sh` and `counted.sh`. These are copies of the earlier study's scripts with the paths and the concurrency made settable. **Peak** means the trial's own `peak_bytes`: the kernel's maximum RSS of the waited-for tool process.
- **Instruments:**
  - `sampler.py`: every 10 ms, the RSS from `/proc/<pid>/smaps_rollup`. At each new peak it saves `smaps`, `status` and the executable's present pages from `/proc/<pid>/pagemap`.
  - `binattr.py`: maps present pages onto ELF sections and `nm` symbols, then onto crates. Rust symbols go by their crate path; C and C++ symbols by name prefix.
  - `heaptrack` 1.5 on `fh-system` and Spider, launched as `/usr/bin/heaptrack` does but with its notices kept off stdout, which is the engine's JSON-RPC channel. This is why the earlier study's heaptrack attempt broke.
  - `MIMALLOC_SHOW_STATS=1`.
  - mimalloc and Tokio runtime options set through their own environment variables (`MIMALLOC_*`, `TOKIO_WORKER_THREADS`). No Goodfetch setting was involved.
- **Scripts** are in [`2026-11-06-foxhound-peak-memory/`](2026-11-06-foxhound-peak-memory/). They hold this run's absolute paths (`/Users/jb/agent-targets/mem-research`).

## 1. The peak, part by part

Taken at the 10 ms sample with the largest RSS, 1,000 pages and 16 in flight (run `fh-raw-1000-c16`, 106.1 MiB sampled, 105.8 MiB kernel peak):

| Part | MiB | What it is | Per |
|---|---|---|---|
| mimalloc arena (`[anon:mimalloc]`) | 61.1 | 4.6 MiB of live heap (§3), about 7 MiB of per-thread page slack, and about 45–50 MiB of transparent-huge-page backing (§4) | process + Run |
| Executable's own pages | 36.6 | §2 | process (18.1) + Run (18.5) |
| Shared libraries | 4.3 | libc 1.9, libstdc++ 1.5 (DuckDB's C++ runtime, linked dynamically in this gnu build), libm 0.6, the loader and libgcc 0.3 | process |
| Other anonymous memory | 4.0 | thread stacks of 29–31 threads, TLS, the loader | process + concurrency |
| **Total** | **106.0** | | |

The three repeats at 1,000 pages peaked at 105.8, 102.3 and 106.2 MiB. The earlier study's 115–118 MiB was measured at `cc72b0f93`, under other load. The repeats' spread follows how much of the arena got huge pages (`AnonHugePages` 20, 28 and 32 MiB), which depends on how fragmented host memory is at the time. That explanation is inferred, not isolated.

## 2. The executable's resident pages, by section and crate

`binattr.py` on the pagemap at peak and on the idle engine, which was started and left 3 s before any request:

| Section | In the file | Resident idle | Resident at peak |
|---|---|---|---|
| `.text` | 100.75 | 10.11 | 27.65 |
| `.rodata` | 10.75 | 2.67 | 3.62 |
| `.rela.dyn` | 3.05 | 3.05 | 3.05 |
| `.data.rel.ro` | 2.04 | 2.04 | 2.04 |
| `.eh_frame`, `.gcc_except_table`, the rest | 11.4 | 0.2 | 0.2 |
| **Total** | **158.5 MB file** | **18.1** | **36.6** |

- **Relocations, 5.1 MiB, all paid at load.** This is a PIE, and the dynamic loader reads all 133,773 relative relocations (3.05 MiB of `.rela.dyn`). It writes 128,459 of them into `.data.rel.ro`, which makes those 2.04 MiB private dirty pages. DuckDB's symbols receive 38,413 of them, 29%, about 1.2 MiB of the two (`relocs.py`). Most of the others land in unnamed Rust vtables and constants.
- **Code, 27.65 MiB resident of 100.75 MiB.** It is code the process ran, plus its neighbours, because the kernel maps up to 64 KiB around each fault (`fault_around_bytes`). That default was not read on this host, since debugfs needs root. 10.1 MiB of it is resident before the first request, and 17.5 MiB is added by the Run.

| Crate (`.text`) | In the file | Resident at peak |
|---|---|---|
| `goodfetch_engine` | 7.24 | 4.18 |
| `goodfetch_protocol` | 5.20 | 2.11 |
| `tokio` | 3.27 | 1.81 |
| `goodfetch_crawl` | 1.45 | 1.27 |
| `goodfetch_fetch_tier` | 1.31 | 1.06 |
| std, core, alloc (non-generic) | 1.24 | 1.01 |
| `serde_json` | 1.47 | 0.96 |
| `jsonschema` (+ `referencing` 0.19, `fancy_regex` 0.18, `schemars` 0.49) | 1.29 | 0.91 |
| `zbus` (D-Bus) | 1.08 | 0.82 |
| SQLite (C) | 0.74 | 0.69 |
| **DuckDB (C++)** | **47.82** | **0.60** |
| `goodfetch_collie`, `goodfetch_page`, `goodfetch_frontier` | 4.69 | 1.61 |
| everything else | about 24 | about 10.6 |

All `goodfetch_*` crates together are 10.5 MiB of resident code (4.1 idle). Generic code is counted under the crate whose type it was instantiated for.

- **Read-only data, 3.6 MiB:** almost all unnamed (string literals, tables). `encoding_rs` 0.11 and `brotli_decompressor` 0.06 are the named ones.
- **Spider's executable:** 15.8 MB, 5.5 MiB resident.

## 3. The live heap, by owner (heaptrack, `fh-system`)

| Run | Peak live heap | Allocation calls |
|---|---|---|
| 10 pages, 16 in flight | 3.97 MB | 82,520 |
| 100 pages | 4.25 MB | 284,049 |
| 1,000 pages | 4.56 MB | 2,323,434 |
| 10,000 pages | 4.59 MB | 22,538,347 |
| 1,000 pages, 1 in flight | 3.91 MB | 1,750,986 |
| 1,000 pages, 64 in flight | 4.60 MB | 4,167,582 |

By the leaf-most Goodfetch frame at the 1,000-page peak (`owners.py`, `topstacks.py`; 4.35 MiB in the folded stacks):

| Owner | MiB | Allocator | Per |
|---|---|---|---|
| SQLite page caches and lookaside: the run file (`run_state::open_file` 0.53, `open_connections` 0.13, `readers` 0.13), the Politeness in-memory store (0.15), `work_items::add` (0.05) | 1.22 | SQLite | Run |
| `goodfetch_dataset::declaration`: the Dataset declaration's compiled JSON Schema | 0.60 | `jsonschema`, `serde_json`, `fluent_uri` | Run |
| `goodfetch_counted_work::compiled`: each `CountedWork::default()` builds three bounded `moka` caches, about 90 KiB. At least six live instances were found (`Engine::new`, `Metrics::new`, `start_runs`, `main`, `lease::Commit::write` → `work_items::Positions::of`, …). Whether the commit path builds one per call was not checked. | 0.58 | own | process + Run |
| `goodfetch_crawl::delivery::deliver`: pages in flight | 0.55 | own | page in flight |
| `goodfetch_fetch_tier::transports`: HTTP client pools | 0.19 | hyper | Run |
| settings, CLI output, logs, engine | about 0.4 | various | process |
| about 815 KiB still allocated at exit (statics) | | | process |

**Per page in flight** is about 11 KiB of live heap: (4.60 − 3.91) MB over 63 more in flight. The live heap does not grow with the site.

## 4. Why 4.6 MiB of heap costs 61 MiB of RSS

mimalloc v3 (`libmimalloc-sys` 0.1.49) reserves a 1 GiB arena. With `allow_thp` on (its default), it calls `madvise(MADV_HUGEPAGE)` on that arena (`c_src/mimalloc/v3/src/prim/unix/prim.c:412-423`). On a host whose THP mode is `madvise`, as here and on Ubuntu's default, the first touch of a 2 MiB-aligned region then faults in a whole 2 MiB page. The per-thread heaps' 64 KiB pages (`MI_SMALL_PAGE_SIZE`, `include/mimalloc/types.h:196`) are spread across those regions by 29 thread heaps (`theaps: 29`, `MIMALLOC_SHOW_STATS`). Purges split the huge pages but leave their touched 4 KiB pages resident. That last step is inferred from the counters, not traced: `AnonHugePages` is 16 MiB at idle and 20–32 MiB at peak, and the arena's committed size at peak is 63.7 MiB for 67 pages in use.

The binary was the same in every run below; only environment variables changed. Kernel peaks in MiB, 16 in flight unless stated:

| Configuration | Idle | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|---|
| As shipped (mimalloc, THP allowed, purge delay 0 set in code) | 39.0 | 94.6 | 102.3–106.2 | 105.5 |
| `MIMALLOC_ALLOW_THP=0` | 27.0 | 52.8 | 55.5, 55.8 | 57.2 |
| `MIMALLOC_ARENA_EAGER_COMMIT=0` | 27.4 | 52.3 | 60.2, 61.8 | 62.7 |
| `MIMALLOC_PURGE_DELAY=0` (already the in-code value, ADR 0138) | 41.5 | 91.0 | 104.2, 113.3 | 108.3 |
| `TOKIO_WORKER_THREADS=4` | 37.3 | 79.9 | 79.1, 81.8 | 84.4 |
| `TOKIO_WORKER_THREADS=4` + `MIMALLOC_ALLOW_THP=0` | 26.6 | 49.2 | 50.3, 51.5 | 55.6 |
| System allocator (`fh-system`, glibc) | 24.2 | 49.9 | 53.8 | 60.9 |
| System allocator, `TOKIO_WORKER_THREADS=4` | | | 51.5 | |

- **Turning THP off halves the peak** and makes it flat with site size. It also matches glibc, so changing allocators is not needed.
- **Wall time** was 1.11 and 1.12 s with THP off, against 1.03, 1.12 and 1.13 s as shipped (1,000 pages). This is one cell on a shared host, not a benchmark: whether THP buys mimalloc speed elsewhere is **unverified**.
- **Concurrency and threads.** As shipped, the peak was 81.3, 83.2, 102–106 and 123.3 MiB at 1, 4, 16 and 64 in flight. With glibc it was 49.0, 53.8 and 70.0 MiB at 1, 16 and 64. The live heap barely moves (§3), so the growth is per-thread allocator state. Tokio starts one worker per core (`Builder::new_multi_thread()` with no `worker_threads`, `crates/goodfetch-collie/src/executable.rs:886`): 16 on this host. On a host with more cores the fixed part grows. That is an extrapolation, not measured.

## 5. Fixed per process, per Run and per page in flight

| Scope | As shipped | THP off | Made of |
|---|---|---|---|
| Per process (idle engine) | 39.0 MiB | 27.0 MiB | executable 18.1 (code 10.1, relocations 5.1, rodata 2.7), arena 16.0 (all huge pages) or 2.9, libraries 4.2, 24 threads |
| Per Run (10-page peak − idle) | +42.5 MiB | about +25 MiB (from the 100-page peak) | code faulted in by the Run +17.5, arena +23 (THP) or +6, SQLite caches, Dataset schema, counted-work caches, HTTP pools |
| Per page in flight | about 11 KiB live; RSS +0.4 MiB per extra in flight (16→64) | | mostly threads and their allocator state, not pages |
| Per page of site | 0 (live heap 4.56 → 4.59 MB from 1,000 to 10,000) | | the run file holds the frontier on disk |

## 6. What Spider's 23 MiB holds

At 1,000 pages (kernel peak 23.1 MiB): executable 5.5, `[heap]` 2.5, other anonymous memory (glibc per-thread arenas) 12.6, libraries 2.5. Its peak grows with the site: 18.6, 23.1 and 52.0 MiB at 100, 1,000 and 10,000 pages. Its live heap at peak, from heaptrack:

| | 1,000 pages | 10,000 pages |
|---|---|---|
| Total | 13.32 MB | 43.95 MB |
| `bytes` buffers under hyper's HTTP/1 `Buffered` read path (`BytesMut::reserve_inner`) | 7.91 | 32.42 |
| `spider` (the `subscribe` channel, 2.4 at both sizes; the streamed `Page`s 0.4 and 1.9) | 3.10 | 5.04 |
| `http` (cloned `HeaderMap`s) | 0.65 | 2.34 |
| visited set (`HashMap<CaseInsensitiveString, ()>`) | not among the top stacks | 0.39 |

What keeps hyper's read buffers alive (probably page bodies sliced from them and still queued in the broadcast channel) is **unverified**.

Spider keeps more live data than Foxhound at every size, and more per page. Its RSS is smaller because its binary is a tenth of the size, and because glibc puts no huge pages under its heap.

## 7. Classification

| Part (1,000 pages) | MiB | Class | Why |
|---|---|---|---|
| THP backing of mimalloc's arena | about 45–50 | **reducible, no ADR change** | One mimalloc option. `MIMALLOC_ALLOW_THP=0` is verified. Setting it in code, next to the purge delay in `goodfetch-collie`'s `executable::allocator` (ADR 0138's memory note), is **unverified**: mimalloc reads `allow_thp` when the OS layer initializes and when each arena is mapped, and the arena may be mapped before `main`. `arena_eager_commit = 0` does most of the same (60–62 MiB). Either could also be a setting. Still needed: the speed effect, and the shipped static musl build (ADR 0024). |
| Remaining allocator slack (per-thread pages) | about 7 | reducible, no ADR change | Fewer runtime threads: Tokio's worker count is a per-core default today. |
| Live heap | 4.6 | small; partly reducible | `CountedWork::default()` builds about 90 KiB of caches each time, more than once per process. The SQLite caches are the run file's (ADR 0138). |
| Code that runs | 27.6 | mostly required by the work; partly reducible without ADR change | The engine, protocol, Tokio, crawl and fetch code. Candidates: `zbus` 0.8 (a D-Bus client during a crawl), JSON Schema validation about 1.8 (`jsonschema`, `referencing`, `fancy_regex`, `schemars`), `clap` 0.4. Each needs a look at why it runs. None is a capability ADR 0057 adds. |
| Relocations | 5.1 | reducible without ADR change; DuckDB's 1.2 of it is ADR 0057's | Packed relative relocations (`-z pack-relative-relocs`, DT_RELR) would shrink `.rela.dyn`'s 3.05 MiB to a small fraction. That is not measured, and on the static musl build the loader's support must be checked. The 2.04 MiB of `.data.rel.ro` stays while the binary is PIE. |
| DuckDB's resident code and libstdc++ | 0.6 + 1.5 | **needs ADR 0057 amended** (and 0087/0088) | DuckDB is 47.8 MiB of code, but a raw crawl touches 0.6 MiB of it. Together with its relocations, keeping DuckDB in the binary costs about 3 MiB of RSS here, and it costs disk, not memory. libstdc++ is a separate library only in the gnu build; the static musl build links it in. |
| Read-only data, libc, stacks | 3.6 + 2.8 + 4 | required | |

Nothing in the peak needs ADR 0057 amended to fall by more than about 3 MiB. The rest of the binary's 36.6 MiB is code the crawl runs, and the 50 MiB that separates Foxhound from glibc-level memory is one allocator option.

## Not verified

- The gnu `just compare` build was measured, not the static musl release executable that ADR 0138's targets name (#2114). Its relocations, libstdc++ and THP behaviour may differ.
- Wall-time effect of `allow_thp = 0` beyond the single cell above.
- Whether `mi_option_set(allow_thp, 0)` in code before threads start has the effect the environment variable has.
- The kernel's fault-around size on this host, which sets how much neighbouring code §2 counts as resident.
- The mechanism behind the THP inflation (§4) and behind Spider's retained read buffers (§6), each inferred from counters.
- The per-commit frequency of `CountedWork::default()`.
- macOS (no THP; the owner's machine) was not measured.
