# What closes Foxhound's last ~10 MiB to Spider at 100 and 1,000 pages?

Date: 2026-11-07. Ticket: [#2198](https://github.com/goodfetch/goodfetch/issues/2198), on the map [#2141](https://github.com/goodfetch/goodfetch/issues/2141). It follows [#2162](https://github.com/goodfetch/goodfetch/issues/2162) ([write-up](2026-11-07-peak-memory-cuts.md)). Its base is the build that the spec [#2165](https://github.com/goodfetch/goodfetch/issues/2165) settles: static-PIE with DT_RELR, PGO and the order file through `zig ld.lld`, and huge pages off from an `.init_array.00100` constructor.

**Answer.** Peak RSS in MiB on the shipped static musl executable, scenario 1, 16 in flight, Spider in the same rounds. Each figure is the median of 3, with each executable dropped from the page cache before each trial:

| Build and settings | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|
| #2165's build (`fh-k`) | 29.7 | 33.8 | 36.7 |
| + order file from an exact touch map (§5) | 28.2 | 31.6 | 38.4 |
| + 2 Tokio workers, blocking pool capped at 4, C `malloc` to mimalloc | **21.1** | **21.6** | **21.7** |
| The same with 1 Tokio worker | 20.2 | 20.1 | 20.6 |
| **Spider** | **18.4** | **22.6** | **53.7** |
| *Separate round:* 2 workers as above, static-PIE | 21.3 | 22.1 | 21.8 |
| *Separate round:* the same, static **non-PIE** | 20.0 | 20.8 | 22.0 |
| *Separate round:* Spider | 18.4 | 22.6 | 54.8 |

- **1,000 pages now meets Spider** with four cuts together, and 10,000 pages stays 32 MiB under it. **100 pages still misses by 1.6–2.7 MiB**, and every remaining cut measured closes at most part of that gap. What remains at 100 pages is the code Foxhound runs (about 8.7 MiB of its executable is touched, §5, against Spider's 5.5 MiB executable and 2.3 MiB of shared libraries) and the 2.1 MiB that static-PIE's start-up dirties (§4).
- **The combination is faster, not slower.** With 2 workers, a pool cap of 4, the order file and C `malloc` to mimalloc, a 1,000-page crawl is 10.2% faster in wall time (paired 95% interval −14.1% to −4.6%, p = 0.006) and uses 18.6% less CPU. At 10,000 pages wall time is unchanged (+1.8%, 95% interval −14.6% to +18.6%), and CPU is 24.9% lower. With 1 worker the crawl is 16% faster at both sizes and uses 33% less CPU.
- **Per cut, alone, at 1,000 pages** (against `fh-k`'s 33.6 in the same rounds):

  | Cut | Saves | Cost |
  |---|---|---|
  | Tokio workers 16 → 2 | 7.2 MiB | 5.4% faster, 14% less CPU |
  | Tokio workers 16 → 1 | 9.0 MiB | 15% faster, 31% less CPU |
  | C code's `malloc` to mimalloc (Zig's `SmpAllocator` today) | 2.7 MiB | +5.3% CPU |
  | Order file from an exact touch map (alone, its own rounds) | 2.2 MiB | wall and CPU unchanged |
  | Blocking pool capped at 4 threads | 1.7 MiB | 5.5% faster at 1,000 pages, none at 10,000 |
  | Static non-PIE instead of static-PIE | 0–1.3 MiB (1.0–1.3 at 100 pages) | loses ASLR of the image |

- **These cuts don't help:**
  - **mimalloc's arena options:** none moves the peak outside noise (§1).
  - **The blocking pool's keep-alive:** no effect.
  - **BOLT:** its static-PIE output crashes, and its non-PIE output is 2.9 MiB larger and 15–19% slower with the only profile this host can take, perf sampling without LBR (§6).
  - **`MADV_RANDOM` on the executable:** saves 1.2 MiB on a cold page cache only (§5).
  - **PGO of the C and C++ code:** not built. At most 1.05 MiB of C/C++ code is touched, already packed by the order file, and the toolchain has no profile runtime for it (§7).
- **The unattributed private mappings** that #2162 left (1–3 MiB) are Zig's `SmpAllocator`: Zig's libc `malloc`, which the C code (SQLite, BoringSSL, zstd, DuckDB) uses. It never gives small freed blocks back. They vanish when the C code's `malloc` goes to mimalloc (§2).
- **The 24 thread stacks** are Tokio's threads, 16 workers and 10–14 blocking-pool threads, all named `tokio-rt-worker`. Each touches 90–150 KiB of stack, 2.3–4.0 MiB in all at 1,000 pages. The worker count and the pool cap cut them; there is no separate stack cut (§3).
- **Recommended defaults:**

  | Cut | Default | Kept as an option |
  |---|---|---|
  | Worker count, a new setting | 2 | 1, and "one per core" (today) |
  | Blocking-pool cap, a new setting | 4 | today's 512 |
  | C code's `malloc` | mimalloc, through `libmimalloc-sys`'s `override` | build change |
  | Order file | widened from a touch map | build change |
  | mimalloc options and stack size | unchanged | |
  | Static non-PIE (1.0–1.3 MiB at 100 pages, loses image ASLR) | owner's call | |
  | BOLT, `MADV_RANDOM` | none | |

  Only scenario 1 was measured: 1 worker wins there, but it was not tried on a CPU-heavy crawl (§3).

## Setup

- **Tree:** `origin/main` at `5d900125e`. In `crates/` it differs from #2162's `64c31c0ec` by one change to `goodfetch-counted-work`: one `Arc` behind a `CountedWork`.
- **Host, toolchain and protocol:** as in #2162:
  - **Host:** the Hetzner CCX43 (16 threads, Linux 7.0.0-38), shared with other agents. Every build and trial ran through `load-guard` (`LOAD_GUARD_HEAVY=1`), one heavy suite at a time.
  - **Toolchain:** cargo-zigbuild 0.24.1 with Zig 0.17.1 (clang and LLD 22.1.8), and rustc 1.99.0 (LLVM 23.1.1).
  - **Trials:** `xtask compare trial --scenario raw`, 16 in flight, against a fresh Proving ground (seed 1574). A figure is the trial's kernel peak (`peak_bytes`), the median of 3. The configurations are interleaved within each round, Spider runs in the same rounds, and every executable is dropped from the page cache before every trial.
  - **Wall time:** interleaved runs without a sampler, 12 rounds at 1,000 pages and 6 at 10,000. Each round runs the base and every variant once, in an order rotated each round. Each variant pairs with that round's base. Reported as the paired median difference with a 95% bootstrap interval and a sign test (`wall.py`).
- **#2165's build, rebuilt.** No binary from #2162 was kept, so the PGO pipeline ran again: an instrumented build (1,404 s), training on scenario 1 (23 s), the profile-guided build (748 s). A re-link through `zig ld.lld` then made it a static-PIE with `-z pack-relative-relocs` and #2162's order file (8,174 functions). It comes in two binaries:
  - `fh-2165`: that build. The measurement-only `PEAKCUT_THP=3` constructor of #2162 turns huge pages off before mimalloc's constructor, in every trial.
  - `fh-k`: the same with a measurement-only patch to the executable's Tokio builder (`knobs.py`, reverted after the build and in no commit). It reads `PEAKGAP_MAX_BLOCKING`, `PEAKGAP_KEEP_ALIVE_MS` and `PEAKGAP_STACK`; with none set it builds today's runtime. The profile-guided build took 632 s. In the same rounds, `fh-2165` and `fh-k` peaked at 30.5 / 33.4 / 36.1 and 30.1 / 33.6 / 40.1; 10,000 pages varies by ±4 MiB between rounds. `fh-k` is the base of every comparison.
- **Re-links** add objects or flags to `fh-k`'s saved link (`add-obj.sh`, #2162's `relink.sh`). A re-link that adds nothing reproduces the binary byte for byte. Each one takes seconds.
- **New instruments:**
  - `sampler2.py`: starts the executable itself, so that under Yama's `ptrace_scope = 1` it may read each thread's stack pointer from `/proc/<pid>/task/<tid>/syscall` at the peak.
  - `attr.py`: puts each resident mapping with its owner: the executable, mimalloc's arena (the 1 GiB reservation), a thread's stack (the mapping its stack pointer is in), or another anonymous mapping.
  - `pageown.py`: maps the executable's resident pages onto lld's link map (`-Map`). It splits them by origin (the Rust object, each C/C++ crate, Zig's runtime) and, for read-only data, by kind: named, anonymous (`.rodata..L*`) or merged strings.
  - `touched.py`: lists the sections on touched pages (§5).
- **Sources.** The facts below about mimalloc, Tokio, BOLT, `zig cc` and lld come from their source at the versions in use. They are cited as `file:line` in [`sources.md`](2026-11-07-peak-memory-gap/sources.md), whose four `[probe]` facts come from one-line scratch builds.
- **Scripts and results** are in [`2026-11-07-peak-memory-gap/`](2026-11-07-peak-memory-gap/): `results/peaks.txt` for every peak, `wall.txt`, `attr.txt`, `exe-pages.txt`, `bolt.txt` and `build-times.txt`. #2162's scripts that these reuse are in [`2026-11-07-peak-memory-cuts/`](2026-11-07-peak-memory-cuts/).

## Where the peak goes at 1,000 pages

`attr.py` and `pageown.py` at the peak, for `fh-k` (16 workers) and Spider in the same round:

| | Foxhound `fh-k` | Spider |
|---|---|---|
| Executable, code (`r-x`) | 9.8 | 3.9 |
| Executable, read-only data and relocated `.data.rel.ro` | 3.75 + 2.18 | 1.7 |
| Shared libraries (glibc, libm, ld.so, libgcc) | none (static) | 2.6 |
| Heap: mimalloc's arena / glibc's arenas and brk | 12.1 | 7.4 + 2.0 |
| Other anonymous mappings | 3.2 (Zig's `SmpAllocator`, §2) | 3.0 |
| Thread stacks | 4.0 (26 Tokio threads) | 1.5 (34 threads) |

Spider is a dynamically linked host (gnu) build, as in #2150 and #2162, with 16 Tokio workers and 16 connection threads. The two lines that matter are the executable (15.7 against 8.2 with Spider's libraries) and the heap that scales with the thread count.

## 1. mimalloc's arena options

The arena held 12.1 MiB at the 1,000-page peak with 16 workers. Each option was set through its `MIMALLOC_*` variable, which mimalloc reads before `main` (`sources.md` §1). The peaks, median of 3, at 100 / 1,000 / 10,000 pages, against `fh-k`'s 30.1 / 33.6 / 40.1 in the same rounds:

| Option (default) | Set to | Peaks | What it does (`options.c`) |
|---|---|---|---|
| `page_full_retain` (2) | 0 | 30.1 / 34.2 / 37.3 | full small pages a thread keeps per size class |
| `page_reclaim_on_free` (0) | 1 | 29.9 / 33.5 / 36.6 | reclaims abandoned pages across threads |
| `generic_collect` (10000) | 1000 | 29.8 / 32.3 / 41.6 | collects 10 times as often |
| `arena_reserve` (1 GiB) | 32 MiB | 30.1 / 33.7 / 36.8 | a smaller virtual arena |
| `page_max_candidates` (4) | 16 | 30.0 / 33.4 / 37.6 | searches more pages for a fuller one |

None moves the peak beyond the ±1 MiB noise at 100 and 1,000 pages, or the ±4 MiB at 10,000.
- `purge_delay` is already 0: `allocator.rs` sets it first thing in `main` at `5d900125e`.
- `arena_eager_commit` commits address space only, which on Linux's overcommit costs no resident memory (`readme.md:354-359`).

The arena shrinks with the thread count instead: 8.0 MiB with 4 workers, and 8 MiB of anonymous memory in all with 2 workers and the other cuts (§3). Each thread that allocates holds at least one partly used 64 KiB page per size class it touches (`sources.md` §1, "Threads").

**Default: leave mimalloc's options as they are.**

## 2. The unattributed mappings: Zig's `malloc`

The 3.2 MiB of anonymous mappings outside the arena at the 1,000-page peak belong to Zig's libc `malloc`. That is 19 mappings of 48 KiB to 1.8 MiB, with no name and no guard page.
- **The cause:**
  - The `mimalloc` crate replaces only Rust's allocator.
  - The C code (SQLite, BoringSSL, zstd and DuckDB's `operator new`) calls `malloc`. In a cargo-zigbuild musl build, that `malloc` is Zig's own `lib/c/malloc.zig`, not musl's. It wraps `std.heap.SmpAllocator`, which keeps small freed blocks on per-thread free lists and never returns them to the OS (`sources.md` §1).
- **The measurement:** a re-link with an extra object, `malloc-shim.c`, that defines `malloc`, `free`, `calloc`, `realloc`, `aligned_alloc`, `posix_memalign`, `malloc_usable_size` and the rest as calls into mimalloc's `mi_*`. That is what `libmimalloc-sys`'s `override` feature does. Zig defines the same symbols weak, so the shim's win without a clash; the link map shows `malloc` from `malloc-shim.o`.
- **The result:** the 19 mappings shrink to 6 holding 0.1 MiB, and the arena grows by 2 MiB.

| | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|
| `fh-k` | 30.1 | 33.6 | 40.1 |
| C `malloc` to mimalloc | 28.4 | 30.9 | 32.8 |
| In combination (2 workers, pool cap 4, ord3 order file): without the shim → with it | 23.8 → 22.3 | 25.9 → 22.7 | 27.9 → 22.6 |

- **Cost:** +5.3% CPU at 1,000 pages (95% interval +3.6% to +8.8%) and +5.9% at 10,000. Wall time is +3.7% and +6.3%, neither significant (p = 0.39, 0.22).
- **In the combination:** the shim's CPU cost shrinks to about 3 points at 1,000 pages (−17.9% CPU with it against −21.0% without) but stays at 10,000 pages (−12.5% against −23.6%).
- **Not tested:** whether the cost comes from `purge_delay = 0` purging the C code's freed spans at once.

**Default: route the C code's `malloc` to mimalloc**, through `libmimalloc-sys`'s `override` feature: a build change with no feature removed. With 2 workers it is the difference between 25.9 and 22.7 MiB at 1,000 pages.

## 3. Tokio's threads: workers, blocking pool and stacks

The executable builds its runtime with `Builder::new_multi_thread().enable_all()` and no other setting (`executable.rs:886-889`). Tokio's defaults therefore apply:
- one worker per core, here 16;
- up to 512 blocking threads plus the workers, each kept 10 s after it idles;
- std's 2 MiB stacks.

The engine's `BlockingPolicy::Threads` sends its blocking work to that pool (`tasks.rs:102-116`). No setting changes any of this today. Tokio reads `TOKIO_WORKER_THREADS` itself, which is how the worker count was varied here. As an environment variable it would also reach the browsers Collie starts, so a shipped setting should call `.worker_threads(n)` instead.

**Stacks.** At the 1,000-page peak `fh-k` runs 26 Tokio threads (16 workers, 10 blocking) and 30 at 10,000 pages. All are named `tokio-rt-worker`, and their stacks hold 2.3–4.0 MiB. The other threads are `main`, `run-state-writer` and `run-state-wal-checkpoint`, plus `politeness-file` when it runs; they hold 0.02–0.18 MiB each.
- Only touched stack pages are resident, 90–150 KiB a thread. A smaller `thread_stack_size` would only lower the reservation, so it was not measured.
- The cut is the thread count: 4 workers leave 13–16 Tokio threads holding 0.96–1.38 MiB.

**Peaks**, median of 3:

| Workers | 100 pages | 1,000 pages | 10,000 pages | Threads at the 1,000-page peak |
|---|---|---|---|---|
| 16 (today, one per core) | 30.1 | 33.6 | 40.1 | 29 |
| 8 | 28.4 | 31.5 | 35.4 | 21 |
| 4 | 26.5 | 28.6 | 34.1 | 17 |
| 2 | 25.3 | 26.4 | 32.0 | 15 |
| 1 | 24.2 | 24.6 | 28.6 | 13 |
| 16, blocking pool capped at 4 | 29.7 | 31.9 | 36.5 | 23 |
| 16, keep-alive 1 s instead of 10 s | 30.0 | 33.0 | 38.0 | 27 |

**Throughput on scenario 1**, paired against 16 workers:

| Workers | Wall, 1,000 pages | Wall, 10,000 pages | CPU, 1,000 / 10,000 |
|---|---|---|---|
| 1 | −15.0% [−24.0, −12.3], 12/12 faster | −13.3% [−18.4, −4.7] | −31% / −31% |
| 2 | −5.4% [−8.6, −2.9], p = 0.039 | −4.2% [−11.7, −3.2], p = 0.031 | −14% / −15% |
| 4 | +1.6%, not significant | −0.2%, not significant | −3% / −5%, not significant |
| 8 | +4.1%, not significant | +0.1%, not significant | +2% / +1%, not significant |
| 16, pool capped at 4 | −5.5% [−12.1, −1.4], p = 0.039 | +4.6%, not significant | −8.7% / −1.1% |

Scenario 1 is a raw crawl of a local Proving ground. Its pages are cheap to fetch and parse, so 16 workers mostly contend: fewer workers spend less CPU stealing and waking.
- **Not measured:** a crawl whose pages cost real CPU (TLS, large documents, extraction) could need more than one or two cores. That is why the recommendation is 2, with today's "one per core" kept as an option.
- **The pool cap and DNS:** with a cap of 4, the blocking pool queues the engine's file and SQLite work, which helped here. Whether it slows resolution was not measured, because the engine resolves through hickory rather than the pool.
- **The governor** reads `num_workers()` (`governor.rs:338-342`), so it follows the setting.

**Defaults:**
- The worker count: a setting, default **2**, with **1** and **one per core** (today) as options.
- The blocking-pool cap: a setting, default **4**, with today's **512** as an option.
- Keep-alive: unchanged.

## 4. Static-PIE's relocated data

With static-PIE, musl's start-up applies the relative relocations itself (`ldso/dlstart.c:135-150`). That writes to every page of `.data.rel.ro` holding a pointer, so all 2.11 MiB of it becomes private memory whether used or not. Static non-PIE leaves those pages clean, and only the touched ones are mapped, from the file. Measured three times:

| Comparison | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|
| `fh-k` (PIE) against `fh-pgo-order` (non-PIE, phase 2) | 30.1 → 29.1 | 33.6 → 32.5 | 40.1 → 36.7 |
| The same, BOLT round (§6) | 29.9 → 28.8 | 33.1 → 33.1 | 37.5 → 37.5 |
| The whole combination, PIE → non-PIE (separate round) | 21.3 → 20.0 | 22.1 → 20.8 | 21.8 → 22.0 |

That makes 1.0–1.3 MiB at 100 pages. #2165 chose static-PIE for the image's ASLR. Whether 1.3 MiB at 100 pages is worth losing it is the owner's call, and it does not close the 100-page gap on its own.

## 5. The executable's pages: an exact touch map and a wider order file

At the 1,000-page peak `fh-k`'s executable is 15.7 MiB resident:
- 9.6 MiB of `.text`: Rust 7.6, SQLite 1.06, DuckDB 0.60;
- 3.6 MiB of `.rodata`: Rust 2.1, merged strings 1.3;
- 2.1 MiB of relocated `.data.rel.ro`.

**Much of that was never touched.** On a file-backed fault the kernel maps the cached neighbours of the page too, 64 KiB at a time (`fault_around_bytes` = 65,536 here). On a cold cache, read-around has also put the neighbours in the cache.
- **The exact touch map.** One 1,000-page trial ran with fault-around at a single page: `fault_around_bytes` was set to 4,096 through debugfs for that trial only, then restored. It used a re-link (`exe-random.c`) that calls `madvise(MADV_RANDOM)` on the executable's read-only segments, turning read-around off.
- **The result:** every resident page of the executable is then one the process touched. The executable is resident at **10.8 MiB**:
  - `.text` 6.9 MiB: Rust 5.8, SQLite 0.82, DuckDB 0.08, mimalloc 0.08, Zig's libc 0.07;
  - `.rodata` 1.6 MiB;
  - `.data.rel.ro` 2.1 MiB.
- **What that leaves:** about 5 MiB of the normal resident size is untouched neighbours. #2162's order file packed only the 8,174 functions that the Rust profile counted.

**The page cache's state changes the figure.** With the whole file in the page cache, as right after an install or copy, `fh-k`'s executable is 17.9–18.2 MiB resident instead of 15.7, peaking at 32.6 / 35.1 against 29.7 / 33.8.

**`MADV_RANDOM` alone** (the `exe-random.c` re-link):

| | Executable resident | Peak, 100 pages | Peak, 1,000 pages |
|---|---|---|---|
| Cold page cache | 15.7 → 14.5 | 30.0 → 28.6 | 33.5 → 35.5 |
| Warm, from earlier trials | 15.9 → 14.7 | 30.6 → 29.2 | 33.6 → 31.9 |
| Whole file cached | 18.0 → 18.1–18.4 | 32.0 → 32.6 | 35.7 → 36.2 |

The cold 1,000-page median comes from runs of 35.5, 36.4 and 31.7, so it is noise. `MADV_RANDOM` only stops read-around: fault-around still maps cached neighbours. **Not recommended.**

**The wider order file.** `touched.py` lists every section on a touched page from lld's link map:
- 9,772 functions beyond the PGO list (Rust and C/C++);
- 8,924 named data symbols;
- 40,182 anonymous Rust constants (`.rodata..L*` sections).

The new order file is the PGO order, then those functions, then that data. Two details matter:
- **Mangled names.** lld's `-Map` prints symbols demangled unless given `--no-demangle`. A first order file (`ord3`) took its Rust and C++ names from a demangled map, so lld silently dropped them; only the C names and the constants applied. It saved 1.0 MiB.
- **Anonymous constants.** No symbol names them, so `objcopy --add-symbol` gives each touched section a local symbol (`pg_anon_N`) in the Rust object before the re-link. lld's `--symbol-ordering-file` orders data sections by their symbols the same way as functions (`sources.md` §5). Merged strings (`.rodata.str*`) cannot be ordered.

The order file with mangled names (`ord5`, 61,912 entries, 1,855 of them unmatched), against `fh-k` in the same rounds:

| | Executable resident | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|---|
| `fh-k`, cold | 15.8 | 29.7 | 33.8 | 36.7 |
| `ord5`, cold | **13.8** | 28.2 | 31.6 | 38.4 (runs 41.6, 38.4, 37.5) |
| `fh-k`, whole file cached | 18.2 | 32.6 | 35.1 | |
| `ord5`, whole file cached | **14.0** | 27.9 | 31.8 | |

- **Where it saves:** with `ord5`, `.text` drops from 9.6 to 8.3 MiB (DuckDB from 0.60 to 0.12) and `.rodata` from 3.6 to 3.0.
- **Cache state no longer matters:** the executable reads the same, 13.8 or 14.0 MiB, whatever the page cache holds.
- **Cost:** none measurable: −1.6% wall time at 1,000 pages and +6.5% at 10,000, neither significant (p = 0.77, 0.69). CPU −1.8% and +0.4%.
- **A second pass adds nothing.** A touch map of the `ord3` layout touched the same 10.77 MiB. Intersecting both maps dropped only 3% of the entries, so the touched set is close to what the crawl really executes.

**For the release build**, this adds two steps to #2165's pipeline:
- A touch-map run after the profile-guided build. Setting `fault_around_bytes` needs root (debugfs), so it suits the release machine, not a developer's laptop.
- An `objcopy` pass over the Rust object before the final link.

Both run through the same `zig ld.lld` re-link as #2165's order file.

**Default: build the order file from the touch map.** It is a build change with no feature removed, and it also makes the bar independent of the page cache's state.

## 6. BOLT

BOLT 21 (`bolt-21` from Ubuntu, installed for this measurement and removed after) needs a link with relocations (`-q`) and a symbol table. `-q` cannot go together with Zig's `--strip-all`, so the inputs were re-links of #2165's link without `-s` and with `-q`. The profile came from `perf record -e cycles:u` on the PGO training workload, three 1,000-page crawls and one 10,000-page crawl. This EPYC Milan VM has no LBR, so `perf2bolt -nl` turned it into a profile, and BOLT ran with Rust's own flags for `librustc_driver` (`-reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-strategy=cdsplit -split-all-cold -jump-tables=move -icf=all`). The output was then stripped.
- **On static-PIE (#2165's build):** BOLT finished in 174 s, but the output crashes at start-up (SIGSEGV on `--version`). LLVM 21 treats a static-PIE as a PIE and rewrites `.relr.dyn` in place. This matches llvm-project issue #90388, "Segmentation fault when optimizing static-pie RELR binary", which was closed without a confirmed fix. BOLT's instrumentation mode, the better profile, writes no profile for a static-PIE before LLVM 23 (#201371, fixed by PR #204475).
- **On static non-PIE:** it runs. Against its own input, `fh-pgo-order`:

| | 100 pages | 1,000 pages | 10,000 pages | Wall, 1,000 / 10,000 | CPU |
|---|---|---|---|---|---|
| non-PIE, PGO and order file | 28.8 | 33.1 | 37.5 | | |
| + BOLT | 32.5 | 36.0 | 43.3 | +19.3% (11/12 slower, p = 0.006) / +14.9% (6/6) | +23% / +21% |

The sampled profiles cover only 856–1,523 of 131,820 functions. BOLT's own quality report puts the CFG flow-conservation gap at 100% for the hottest 1,000 functions: without branch records it reorders blocks on guesses, and it moves code the PGO layout had already packed.

**Default: no BOLT.** Not verified: BOLT with an LBR profile on hardware that has it, or with LLVM 23's static-PIE instrumentation.

## 7. PGO of the C and C++ code

**The bound on memory** comes from the exact touch map (§5). At 1,000 pages the C/C++ code touched is 1.05 MiB: SQLite 0.82, mimalloc 0.08, DuckDB 0.08, Zig's libc and libc++ 0.07. The order file of §5 already packs it, so C/C++ PGO could at most shrink part of that 1 MiB. Most of PGO's memory gain in #2162 came from grouping hot code, and the order file now does that for C/C++ too.

**The bound on speed** comes from the BOLT profile's user cycles on scenario 1, split by owner with the link map:

| Owner | Share of user cycles |
|---|---|
| Rust | 40–41% |
| SQLite | 28–31% |
| `compiler_rt`'s `memcpy`, `memcmp`, `bcmp` | 12% |
| mimalloc | 11–12% |
| Zig's libc | 5% |
| DuckDB | 0% |

A C PGO would mostly speed up SQLite.

**The toolchain cannot build it as pinned** (`sources.md` §4):
- `zig cc` passes `-fprofile-generate` to clang 22 but links no profile runtime, and Zig ships none.
- Rust's `profiler_builtins` writes raw profile v11 (LLVM 23), while clang 22's objects are v10, with a different record layout.
- `llvm-profdata` 23 writes index v14, which clang 22 rejects, and has no option to write an older one.
- A working path needs three things:
  - an LLVM 22 `compiler-rt` profile runtime, built from source;
  - a separate C-only training build;
  - LLVM 21 or 22's `llvm-profdata`.

That is about two more full builds per release, for under 1 MiB at best. **Not built.** **Default: none for memory.** C PGO is a speed question, of up to the SQLite share, for later.

## 8. What still misses Spider

With 2 workers, a pool cap of 4, C `malloc` to mimalloc and the `ord5` order file, static-PIE:

| | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|
| Foxhound | 21.1 | 21.6 | 21.7 |
| Spider | 18.4 | 22.6 | 53.7 |
| Gap | **+2.7** | −1.0 | −32.0 |

At the sampled 100-page peak, the combination and Spider break down as follows:

| | Foxhound | Spider | Gap |
|---|---|---|---|
| File-backed (`RssFile`) | 11.6 | 7.8 | +3.8 |
| Private (`RssAnon`) | 9.9 | 7.6 | +2.3 |

- **File-backed:** Foxhound's 11.6 MiB is its executable: the code and data the crawl runs (8.7 MiB touched, §5) plus fault-around at the edges. Spider's 7.8 MiB is its executable (5.5) and its shared libraries (2.3).
- **Private:** Foxhound's 9.9 MiB includes 2.1 MiB of `.data.rel.ro` that static-PIE's start-up writes to (§4). Without it, Foxhound's private memory equals Spider's.

What remains is the code Foxhound runs and the PIE relocation.
- **What 100 pages would still need:**
  - 1 worker brings it to 20.2;
  - static non-PIE to 20.0 with 2 workers (−1.3).

  Together they would come close to 19 MiB, but that combination was not measured.
- **Beyond that:** less code on the path, such as lazy start-up (#2162's estimate: 0.8 MiB of code once laid out, plus 0.6 MiB of heap) or fewer crates on the crawl's path. These are code changes, not settings.

## Not verified

- **Other scenarios:** only scenario 1 was run. The worker-count and pool-cap results may differ on crawls whose pages cost real CPU, on TLS, or with the browser tier.
- **Other platforms:** macOS and arm64 were not measured.
- **Combinations not measured:** 1 worker with non-PIE; the pool cap without the worker change at 10,000 pages beyond 3 rounds; `thread_stack_size` (only touched pages count, so only the reservation would shrink).
- **The shim's CPU cost:** whether it comes from `purge_delay = 0` purging the C code's freed memory at once.
- **BOLT with a better profile:** with LBR, or with LLVM 23's instrumentation of static-PIE.
- **Spider's 10,000-page trial** in phase 2's first round failed (no JSON); that cell is the median of 2.
- **Measurement-only scripts:** `knobs.py`, `thp-modes.py` (#2162), `malloc-shim.c` and `exe-random.c` are in no commit of the product. The shim stands in for `libmimalloc-sys`'s `override` feature, which was not built as such.
