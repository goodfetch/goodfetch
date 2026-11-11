# #2198 sources: what could close Foxhound's last ~10 MiB to Spider

Primary-source notes only; no repository build, no benchmark. Read on the Hetzner server (`ubuntu-64gb-fsn1-1`, AMD EPYC-Milan VM, 16 vCPUs).
Four small scratch probes (one-line C or Rust files compiled in the scratchpad, never the repository) settled facts that no document states. They are marked **[probe]**.

Path shorthands:
- `$REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`
- `$MI` = `$REG/libmimalloc-sys-0.1.49/c_src/mimalloc/v3` (v3.3.2, `include/mimalloc.h:11` `MI_MALLOC_VERSION 30302`)
- `$TK` = `$REG/tokio-1.53.3` (the version in Cargo.lock)
- `$ZIG` = `~/.local/share/goodfetch/musl-toolchain/zig-0.17.1-cargo-zigbuild-0.24.1/zig` (`zig cc --version` prints clang 22.1.8)
- `$RS` = `~/.rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library`
- `$REPO` = the worktree `confident-edison-3e91c2` at `18c5dd365`, an older `main` than the measured `5d900125e`: line numbers in `crates/` can differ (the executable's runtime builder is at `executable.rs:886-889` in `5d900125e`)

---

## 1. mimalloc v3 (libmimalloc-sys 0.1.49)

### Which version gets built, and which crate features exist
- `$REG/libmimalloc-sys-0.1.49/build.rs:8-12` builds `v3` unless the `v2` feature is on. The workspace sets `mimalloc = { version = "0.1.48", default-features = false }` (`$REPO/Cargo.toml:272`) and turns on no features, so Foxhound runs v3.
- The libmimalloc-sys features are `arena, debug, debug_in_debug, extended, local_dynamic_tls, no_thp, override, secure, v2, win_direct_tls` (its `Cargo.toml` `[features]`). The `mimalloc` 0.1.52 crate forwards all of them except `arena`. There is no `v3` feature, because v3 is the default.
  - `override` defines `MI_MALLOC_OVERRIDE` and adds `-fno-builtin-malloc` (`build.rs:66-79`).
  - `secure` defines `MI_SECURE=4` (`:81-83`).
  - `local_dynamic_tls` builds with `-ftls-model=local-dynamic`; the default is `initial-exec` (`:89-97`).
  - `no_thp` defines `MI_NO_THP` (`:99-103`).
  - In a release build: `MI_DEBUG=0`, `MI_BUILD_RELEASE`, `NDEBUG` (`:105-115`).
  - Everything compiles as one translation unit, `src/static.c`, through cc-rs (`:24,61,117`).
- `extended` exposes `mi_option_set` (`$REG/libmimalloc-sys-0.1.49/src/extended.rs:617`) and `mi_collect` (`:234`). Its `mi_option_*` constants stop at index 21 (`extended.rs:497-563`). For the v3 options below you must pass the raw index from the v3 enum.
- **v3 option indices**, computed from `$MI/include/mimalloc.h:437-487`:
  - 4 `arena_eager_commit`, 5 `purge_decommits`, 9 `reserve_os_memory`, 15 `purge_delay`
  - 22 `destroy_on_exit`, 23 `arena_reserve`, 24 `arena_purge_mult`, 26 `disallow_arena_alloc`
  - 34 `generic_collect`, 35 `page_reclaim_on_free`, 36 `page_full_retain`, 37 `page_max_candidates`, 39 `pagemap_commit`
  - 40 `page_commit_on_demand`, 41 `page_max_reclaim`, 42 `page_cross_thread_max_reclaim`, 43 `allow_thp`, 44 `minimal_purge_size`

### Options that affect resident memory (env var `MIMALLOC_<NAME>`, defaults from `$MI/src/options.c:112-182`)
**Caution:** several header comments in `mimalloc.h` are stale (they say purge_delay =10, arena_purge_mult =10 and cross-thread reclaim =16). The table's defaults come from `options.c`, which is what runs.

| Option | Default | What it does (source) |
|---|---|---|
| `PURGE_DELAY` | **1000** ms (`options.c:140`) | Delay before freed arena slices are purged; 0 = immediate, -1 = never (`mimalloc.h:455`; readme `:360-364`). Arena delay = `purge_delay × arena_purge_mult` (`arena.c:2020-2023`). |
| `ARENA_PURGE_MULT` | 1 (`options.c:149`) | Multiplier above. |
| `PURGE_DECOMMITS` | 1 (`:127`) | Purge = decommit (`MADV_DONTNEED`, RSS drops at once); 0 = reset (`MADV_FREE`, RSS drops only under pressure) (`os.c:650-661`; readme `:365-369`). |
| `ARENA_RESERVE` | 1 GiB, in KiB (`:51-57,148`) | Virtual size of each new arena. Arenas grow ×2 every 8 arenas (`arena.c:316-331`); min 32 MiB (`types.h:650`). Read when an arena is reserved. |
| `ARENA_EAGER_COMMIT` | 2 (`:46-48,125`) | 2 = commit the whole arena eagerly on overcommit systems (Linux) (`arena.c:353-356`). The readme says this raises commit but not peak RSS (`readme.md:354-359`). |
| `PAGE_COMMIT_ON_DEMAND` | 0 (`:171`) | 0 = commit each page whole when it is allocated; 1 = commit on demand; 2 = on demand except on overcommit systems. Small pages are always committed (`arena.c:962-964`). |
| `PAGE_FULL_RETAIN` | 2 (`:166`) | Full *small* pages (≤ `MI_SMALL_MAX_OBJ_SIZE`, about 10 KiB) a theap keeps in a size-class queue before it abandons them (`page.c:749,771-776`). Clamped to -1..32 and **cached per theap when the theap is created** (`init.c:307-311`). -1 also disables abandoning (`init.c:309`, `page.c:388-396`). |
| `PAGE_MAX_CANDIDATES` | 4 (`:167`) | Pages searched for the best (fullest) candidate before allocating (`page.c:781-797`); read on each search (`_mi_option_get_fast`). |
| `PAGE_RECLAIM_ON_FREE` | 0 (`:165`) | On a free into an abandoned page: -1 never reclaims; 0 reclaims only into the originating theap; 1 reclaims across theaps (`free.c:328-340,375-384`). |
| `PAGE_MAX_RECLAIM` | -1, unlimited (`:95-97`) | Cap on pages a theap reclaims from itself per size class (`free.c:332,343`). |
| `PAGE_CROSS_THREAD_MAX_RECLAIM` | **32** (`:99-101`) | Same cap, across threads (`free.c:339`). |
| `GENERIC_COLLECT` | 10000 (`:164`) | Run `mi_theap_collect` every N calls to the generic allocation path. Retired pages are collected every 1000 calls (`page.c:995-1009`). |
| `RESERVE_OS_MEMORY` | 0 (`:75-77`) | Pre-reserves an arena at process init only (`init.c:1130-1131`). |
| `DESTROY_ON_EXIT` | 0 (`:147`) | Frees every arena at exit (`init.c:1178`). Exit-time only, so no effect on RSS during a run. |
| `ALLOW_THP` | 1 (`:103-109,176`) | 0 calls `prctl(PR_SET_THP_DISABLE)` for the process (`prim/unix/prim.c:222-232`). `MADV_HUGEPAGE` is applied only when large OS pages are allowed (`prim.c:412-419`). |
| `MINIMAL_PURGE_SIZE` | 0 → OS page size (`os.c:55-66`) | 2 MiB only when `allow_thp == 2`. The readme's "2 MiB when THP is detected" (`readme.md:373-375`) does not match the code at the default `allow_thp = 1`. |
| `ALLOW_LARGE_OS_PAGES`, `RESERVE_HUGE_OS_PAGES` | 0, 0 | Huge pages; off. |
| `PAGEMAP_COMMIT` | 0 on Linux (`:87-93`) | Commits the page map up front only when set or ≤ 1 MiB (`page-map.c:52,259`). |
| `DISALLOW_ARENA_ALLOC` | 0 | 1 = allocate straight from the OS instead of arenas (`arena.c:572,760`). |

- **There is no v3 "segment reset", "page reset", "eager_commit_delay", "segment_cache" or "max_segment_reclaim".** Each is a `deprecated_*` slot (`mimalloc.h:443,450-454,461`), and nothing outside `options.c` reads them (grep of `$MI/src`).
- **Sizes:** an arena slice is 64 KiB; small pages are 64 KiB, medium 512 KiB, large 4 MiB (`types.h:159-198`).
  - Small objects are ≤ 10 KiB, medium ≤ about 84 KiB (`types.h:423-425`).
  - Every page is committed at allocation unless `page_commit_on_demand` says otherwise.

### Purging: when it actually happens (relevant to *when* to sample RSS)
- **No background thread does it.** Freeing an arena slice schedules a purge `delay` ms later (`arena.c:2059-2083`).
- The purge runs only when a thread next passes through `_mi_arenas_collect`: on a page free (`page.c:419`), on page abandon (`page.c:312`), through `mi_theap_collect` (`theap.c:135`) or through `mi_collect` (`theap.c:149-151`).
- A purge visits at most `max_arena/4+1` arenas per call (`arena.c:2162-2208`).
- **Consequence:** right after a burst, freed memory stays resident for at least 1 s, and longer if the process goes quiet. `mi_collect(true)` forces the purge (`theap.c:135`: `MI_FORCE` → force purge, visit all).
- **Retired pages:** an empty page that is the only page in its queue is kept for 16 generic-alloc rounds (4 for non-small pages) before it is freed (`page.c:423-466,471-496`).

### Threads, theaps and abandonment in v3
- Each thread gets its own "theap" (thread-local heap) with its own page queues (`init.c:600-624`).
- **Full pages are abandoned at once** when `allow_page_abandon` is set, so other threads can share them (`page.c:382-397`).
- At thread exit, a pthread key destructor (`prim/unix/prim.c:956-958`) abandons the pages of all the thread's theaps (`init.c:628-646`). The pages are abandoned, not freed: they are reclaimed on a later free (rules above) or freed once empty (`free.c:378-384`).
- `is_in_threadpool` is always false on Unix (`prim/unix/prim.c:989-991`) unless `mi_thread_set_in_threadpool()` is called (`mimalloc.h:206`, `init.c:978`).
- **RSS implication** (an inference, not measured): each thread that allocates holds at least one partly used 64 KiB small page per size class it touches, plus up to `page_full_retain` full ones. More Tokio workers or blocking threads therefore raise the floor. Measure it by varying the thread count.

### Are options read before `main` in a static musl binary? Yes.
- The constructor `mi_process_attach` is `__attribute__((constructor(101)))` under clang (`$MI/src/prim/prim.c:31-43`). It calls `_mi_auto_process_init` → `mi_process_init` → `_mi_options_init`, which reads every option (`init.c:997-1010,1097-1102`; `options.c:191-207`).
- The env lookup walks `environ` directly, case-insensitively, and works "even before the C runtime is initialized" (`prim/unix/prim.c:810-840`; `MI_USE_ENVIRON` defaults on).
- musl sets `__environ` in `__init_libc` before `__libc_start_init` runs `.init_array` (`$ZIG/lib/libc/musl/src/env/__libc_start_main.c:23-26,59-65,80-92`).
- A lazy `mi_option_get` from an earlier `malloc` also initialises from `environ` (`options.c:271-280,633-704`).

### `mi_option_set` at run time (`options.c:298-312`)
- It writes the value and marks it initialised. There is no synchronisation (`options.c:25-30` comment). The change applies at the option's **next read**:
  - **Read on every use:** `purge_delay`, `arena_purge_mult`, `purge_decommits`, `page_commit_on_demand`, `page_max_candidates`, the reclaim limits, `page_reclaim_on_free` in `free.c`, `generic_collect`.
  - **Read only when an arena is reserved:** `arena_reserve`, `arena_eager_commit`. The first arena is reserved by the first allocation, which Rust's runtime makes before `main`, so setting either from `main` is too late for the first arena. Use the env var, or the build-time macros below.
  - **Cached per theap at creation** (`init.c:307-311`): `page_full_retain` and `page_reclaim_on_free`'s allow flags. A change reaches only theaps created afterwards; the main thread's theap keeps its old value.
  - **Process init only:** `reserve_os_memory` and huge-page reservation (`init.c:1121-1131`).
- **Build-time defaults:** `MI_DEFAULT_ARENA_RESERVE`, `_ARENA_EAGER_COMMIT`, `_PAGE_MAX_RECLAIM`, `_PAGE_CROSS_THREAD_MAX_RECLAIM`, `_ALLOW_THP`, `_RESERVE_OS_MEMORY`, `_PAGEMAP_COMMIT` and others are `#ifndef` macros (`options.c:36-109`). They can be passed as `-D` through cc-rs's `CFLAGS` / `CFLAGS_<target>` (`$REG/cc-1.8.0/src/lib.rs:70,169-175`). `purge_delay` has no macro: its 1000 is a literal (`options.c:140`).
- **Repo note:** at `5d900125e`, `crates/goodfetch-collie/src/executable/allocator.rs` exists and calls `mi_option_set(15, 0)` (purge delay 0) first thing in `main`, not through `MIMALLOC_PURGE_DELAY`, which would reach the browsers the engine starts. (`$REPO`, an older tree, lacks the file.)
- `#[global_allocator]` replaces only Rust's allocator (`$REPO/crates/foxhound/goodfetch-foxhound/src/main.rs:14-15`). Without `override`, `static.c` defines `mi_*` symbols only. `override` makes `alloc-override.c`, included by `alloc.c` through `static.c:23`, emit `malloc`/`free`/… aliases (`alloc-override.c:13-45`).
- **What C code uses instead is not musl's mallocng.** Zig 0.17's libc for musl targets exports its own `malloc` family from `$ZIG/lib/c/malloc.zig:25-40` (imported unconditionally except on MinGW, `$ZIG/lib/c.zig:63-68`). There is no `musl/src/malloc` directory in `$ZIG/lib/libc/musl/src`.
- That `malloc` wraps `std.heap.SmpAllocator` (`malloc.zig:44-47`):
  - per-thread freelists in power-of-two size classes, capped at the CPU count (≤ 128) (`$ZIG/lib/std/heap/SmpAllocator.zig:1-28,42-55,103`);
  - 64 KiB slabs mmapped on demand (`:51,155`);
  - **small frees go to a freelist and are never returned to the OS**; only allocations above 64 KiB are `munmap`ed (`:200-215`).
- cargo-zigbuild 0.24.1 removes rustc's self-contained musl `crt*.o` and `-lc` so that zig supplies libc (`src/zig/linker_args.rs:186-201`, https://github.com/rust-cross/cargo-zigbuild/blob/v0.24.1/src/zig/linker_args.rs).
- **[probe]** A `cargo zigbuild --target x86_64-unknown-linux-musl` hello that calls `libc::malloc` has `malloc` = `c.malloc.malloc` with `heap.SmpAllocator.alloc` and `heap.PageAllocator.map` in its symbol table.
- So SQLite (rusqlite `bundled`), BoringSSL (`btls-sys`), zstd and DuckDB's C++ `operator new` all use Zig's SmpAllocator today.
- Zig exports these symbols **weak** (`$ZIG/lib/c.zig:31-43`), so mimalloc's `override` would take precedence at link time without a duplicate-symbol error. That is inferred from the source, not linked here.
- **C/C++ crates in Foxhound's graph** (`cargo tree --offline -i`, Foxhound executable on musl): `libmimalloc-sys` (cc), `libsqlite3-sys` 0.38.2 (cc), `zstd-sys` (cc; it calls `cmake::Config` only on another path, `build.rs:8`), `ring` 0.17.14 (cc; rcgen, rustls), `btls-sys` 0.5.6 (BoringSSL through cmake; `btls`, the fetch tier), `libduckdb-sys` (cc, C++; `goodfetch-sql-tab` only). `brotli` is pure Rust. There is no aws-lc-sys or openssl-sys in Cargo.lock.

---

## 2. Tokio 1.53.3 threads, and every OS thread in the repository

### Defaults
- **Worker count:** `self.worker_threads.unwrap_or_else(num_cpus)` (`$TK/src/runtime/builder.rs:2008`).
  - `num_cpus()` reads `TOKIO_WORKER_THREADS` first: it panics if the value is not a usize or is 0, and falls back to `std::thread::available_parallelism()` (`$TK/src/loom/std/mod.rs:84-104`).
  - The env var is therefore honoured only when the Builder never calls `.worker_threads(n)`, and it is read when `build()` runs.
  - `available_parallelism` applies the cgroup quota and `sched_getaffinity` (`$RS/std/src/sys/thread/unix.rs:172-179`; docs `$RS/std/src/thread/functions.rs:655-694`). On this VM it is 16.
- **Blocking pool:** `max_blocking_threads` defaults to 512 (`builder.rs:306,540`). The pool's cap is `max_blocking_threads + worker_threads` (`builder.rs:2014`). Idle blocking threads exit after `KEEP_ALIVE = 10 s` (`$TK/src/runtime/blocking/pool.rs:172,212`).
  - Users of the pool: `tokio::fs` (`$TK/src/fs/mod.rs:11-19`), `ToSocketAddrs` DNS, stdio (`builder.rs:530-533`).
- **Thread stack size:** `thread_stack_size` defaults to `None` (`builder.rs:312`). The docs say "default stack size for spawned threads is 2 MiB" (`builder.rs:658-680`); with `None`, Tokio sets no size (`pool.rs:467-468`) and std's default applies.
  - std's default is `RUST_MIN_STACK` or `DEFAULT_MIN_STACK_SIZE = 2 MiB` on Unix, read once (`$RS/std/src/sys/thread/unix.rs:31`; `$RS/std/src/thread/lifecycle.rs:30-45`).
  - musl `mmap`s guard + stack + TLS per thread (`$ZIG/lib/libc/musl/src/thread/pthread_create.c:240-318`), so only touched stack pages count in RSS.

### Runtimes built in the repository (production paths)
- **Attached engine / executable main:** `new_multi_thread().enable_all()` with no `worker_threads`, `max_blocking_threads`, `thread_stack_size` or keep-alive (`$REPO/crates/goodfetch-collie/src/executable.rs:858-861`).
- **Daemon:** a multi-thread runtime for the foreground and launched Daemon (`daemon.rs:1110-1115`, used at `daemon/foreground.rs:162` and `daemon/launched.rs:180`). Current-thread runtimes are used for helpers (`daemon.rs:1103-1108`, `config.rs:235`, `browsers.rs:135`, `daemon/reaper.rs:56`, `daemon/launched.rs:699`, `otlp.rs:49`).
- **CLI command connection:** a multi-thread runtime with defaults (`$REPO/crates/goodfetch-cli/src/engine_choice.rs:565-570`, used at `:403,:536`).
- **Stub gateway (test tool):** `crates/goodfetch-stub-gateway/src/main.rs:79`.
- ADR 0060 allows runtime construction only in `main` and test helpers (`docs/adr/0060-…md:46`), and production uses multi-thread (`:33`).

### Other OS threads
- **`tasks::TaskOwner::blocking`:** under `BlockingPolicy::Threads`, `spawn_blocking` through the tracker (`crates/goodfetch-engine/src/tasks.rs:102-116`). The executable selects `Threads` (`crates/goodfetch-collie/src/executable.rs:429`; also `engine_choice.rs:777`). About 134 `.blocking(` call sites exist in `goodfetch-engine/src` and `crates/foxhound` (grep count).
- **Actor threads:** one dedicated `std::thread` per actor (`tasks.rs:175-203`). The users are:
  - `run-state-writer` (the SQLite writer, `crates/goodfetch-engine/src/run_state.rs:376`)
  - `politeness-file` (`crates/goodfetch-engine/src/politeness.rs:468`)
  - `cdp command log` (`crates/goodfetch-page/src/trace.rs:83`, browser tier)
- **Log-file writer thread** `log-file` (`crates/goodfetch-collie/src/executable/log_file.rs:85-97`).
- **DuckDB** (SQL tab only): the table function declares `set_max_threads(available_parallelism)` (`crates/foxhound/goodfetch-sql-tab/src/tables.rs:189-190`). The connection config sets `max_memory` but no `threads` (`sandbox.rs:178-190`), so DuckDB's default thread count applies whenever a query runs. Its threads are not started during a crawl unless the SQL tab is used (inferred from the call graph).
- **Vendored wreq:** `std::thread::spawn` in the TLS keylog writer (`vendor/wreq/src/tls/keylog.rs:131`, only when keylogging is on), and a `spawn_blocking` getaddrinfo resolver (`vendor/wreq/src/dns/gai/tokio.rs:41`). The engine resolves through hickory (`Cargo.toml:265-269`), so the gai path is probably unused; this was not traced.
- **Not used:** rayon, `tracing_appender`, io_uring (needs `tokio_unstable`; no `.cargo/config.toml` rustflags set it), and Tokio signal threads (signals go through the I/O driver).
- **Test-only:** `goodfetch-conformance` threads (`protocol/target.rs:523,570`).
- **Worker-count setting: none exists.** No CLI flag, settings field or env read sets `worker_threads`, `max_blocking_threads` or the stack size. A grep of `goodfetch-protocol/src`, `goodfetch-collie/src` and `goodfetch-cli/src` for worker/thread settings finds only the governor's `tokio_busy` metric.
  - The governor reads `num_workers()` (`crates/goodfetch-engine/src/governor.rs:338-342`).
  - `TOKIO_WORKER_THREADS` still works today, because Tokio reads it itself, but it would also reach child processes.

---

## 3. BOLT (apt: `bolt-21` 1:21.1.8 is installable, not installed)

- **Input requirements:**
  - x86-64/AArch64 ELF with an unstripped symtab; "to get maximum performance gains, they should be linked with relocations (`--emit-relocs` or `-q`)" (https://github.com/llvm/llvm-project/blob/release/21.x/bolt/README.md, "Input Binary Requirements"; Step 0, `--emit-relocs`).
  - Without relocations BOLT runs in **non-relocation mode**: functions are optimised in place and not reordered.
  - Non-relocation mode disables `-split-eh`, hot-text (`-hot-text`) and `--use-old-text` (`bolt/lib/Rewrite/RewriteInstance.cpp:2029-2031,2086-2088,2123-2124,2147-2148` on release/21.x).
  - Stripped binaries are refused unless `--allow-stripped` is given (`:2038-2043`).
- **Static non-PIE:** a missing `PT_DYNAMIC` → "static input executable detected", `IsStaticExecutable` (`RewriteInstance.cpp:5694-5698`). There is static-specific handling for `.rela.dyn` without `PT_DYNAMIC` (`:2399-2400`) and an `_init`/GOT workaround (`:3050-3062`; issue #100096).
- **Static-PIE:** `PT_DYNAMIC` is present, so LLVM 21 treats the binary as PIE, not static (TODO at `:5696`).
  - It reads `DT_RELR` (`:2371-2381,2522-2560`) and rewrites `.relr.dyn` **in place**. If the new relocations need more entries than the original section holds, it fails with "Offset overflow for relr.dyn section" (`:5344-5425`).
  - Issue #90388, "Segmentation fault when optimizing static-pie RELR binary" (2024), was closed as probably fixed by later static-binary patches (PR #100801 and others) without a confirmed fix (https://github.com/llvm/llvm-project/issues/90388).
- **Instrumentation on static binaries:**
  - **Static non-PIE in LLVM 21:** it errors out unless given `-instrumentation-sleep-time=N` (N > 0), because there is no fini hook (`bolt/lib/RuntimeLibs/InstrumentationRuntimeLibrary.cpp:61-66`). It needs relocation mode (`:45-49`) and patches `e_entry` to `__bolt_instr_start` (`RewriteInstance.cpp:4795-4797`; `bolt/runtime/instr.cpp:1763-1795`).
  - **Static-PIE in LLVM 21–22:** it hooks `DT_INIT`/`DT_FINI` or the first `.fini_array` slot (`RewriteInstance.cpp:1385-1425,5628-5650`), but statically linked libc does not read those dynamic entries. Result: **no profile is written** (issue #201371, clang/llvm-bolt 22, https://github.com/llvm/llvm-project/issues/201371). musl confirms this: `rcrt1.c` calls `_init` by symbol (`$ZIG/lib/libc/musl/crt/rcrt1.c:6,13`).
  - Fixed by PR #204475, "Instrument static PIEs through entry point", merged 2026-06-17 (`5f3e70ca97a8`). That puts the fix in LLVM 23 or later, not in apt's 21. It **still requires `--instrumentation-sleep-time`**, and the program must run long enough (maintainer comment on #201371).
- **Sampling without LBR (Milan VM: no `brs`/LBR flag in `/proc/cpuinfo`; `/sys/bus/event_source/devices/cpu/caps` has only `max_precise`):**
  - The README says to record cycles without `-j` and pass `-nl` to `perf2bolt`; "the quality of the profile information would not be as good, and performance gains with BOLT are expected to be lower" (README, Step 1 "For Applications" and the end of Step 2).
  - `perf2bolt -p perf.data -o perf.fdata <exe>`; `-p perf.data` is accepted by llvm-bolt directly (README Step 2).
- **Rust binaries:** rustc's own `librustc_driver` is BOLT-optimised in Rust's release pipeline. Its flags:
  - link: `-Clink-args=-Wl,-q` (https://github.com/rust-lang/rust/blob/main/src/bootstrap/src/bin/rustc.rs, lines 225-229);
  - instrument: `-instrument --instrumentation-file-append-pid`;
  - optimise: `-reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-strategy=cdsplit -split-all-cold -jump-tables=move -icf=all -update-debug-sections` (https://github.com/rust-lang/rust/blob/main/src/tools/opt-dist/src/bolt.rs, lines 11-95);
  - `--use-old-text` stays off because it "succeeds very non-deterministically" (`bolt.rs:85-91`).
  - That is a shared library, not a static musl executable. **No primary source found** for BOLT on musl specifically.
- **RSS effect of layout: no primary source quantifies it.** Mechanism, inferred:
  - BOLT emits the new `.text` in a new segment unless `--use-old-text`, so the file grows.
  - Untouched original text pages are never faulted in.
  - RSS from text = touched pages, which `-reorder-functions` and `-split-functions`/`-split-all-cold` pack together.
  - An lld-only alternative for startup page faults: `--bp-startup-sort=function --irpgo-profile=<file>` (`lld/ELF/Options.td:148-153` on release/22.x).
- **Static-PIE vs non-PIE RSS, inferred from source:**
  - musl's static-PIE start applies `REL`/`RELA`/`RELR` relative relocations itself (`$ZIG/lib/libc/musl/ldso/dlstart.c:135-150`). This dirties every `.data.rel.ro`/`.got` page that holds a pointer, turning it into private anonymous RSS whether or not it is used later.
  - A static non-PIE binary has no such relocations, so those pages stay clean and file-backed.
  - Worth one measurement: RssFile vs RssAnon in `/proc/pid/status`.

---

## 4. PGO of the C/C++ code built through `zig cc`

- **Flags pass through. [probe]** With `zig cc -target x86_64-linux-musl -O2 -fprofile-generate`, the cc1 line carries `-fprofile-instrument=llvm`, and the IR has `__llvm_profile_raw_version`, `__profc_*` and `__profd_*` in `__llvm_prf_*` sections. `-fprofile-instr-generate` (front-end) also instruments, and `-fprofile-use=<file>` is read by clang (its errors below come from clang's reader).
- **No profile runtime. [probe]** The link line is `crt1.o … libc.a libzigc.a libcompiler_rt.a` with no `libclang_rt.profile`. The binary keeps `__llvm_prf_cnts`/`__llvm_prf_data` but has no `__llvm_profile_*` symbols and writes no `.profraw`. `$ZIG/lib` contains no `InstrProfiling*` sources (grep).
- **Raw formats differ, so Rust's `profiler_builtins` cannot be the runtime for clang-22 objects:**
  - LLVM 22: `INSTR_PROF_RAW_VERSION 10`, `INSTR_PROF_INDEX_VERSION 13` (https://github.com/llvm/llvm-project/blob/release/22.x/llvm/include/llvm/ProfileData/InstrProfData.inc, lines 723-725).
  - LLVM 23: raw 11, index 14 (release/23.x, same file, lines 776-778). v11 adds `UniformCounterPtr` and `OffloadDeviceWaveSize` to each data record (release/23.x `InstrProfData.inc:80-99`).
  - **[probe]** clang 22's `__profd_` is `{i64,i64,i64,i64,ptr,ptr,i32,[3 x i16],i32}` and its raw version constant is `0x010000000000000A` (v10 + IR flag). rustc 1.99's is `{i64,i64,i64,i64,i64,ptr,ptr,i32,[3 x i16],i16,i32}` and `0x010000000000000B`. The musl `libprofiler_builtins` rlib's `__llvm_profile_raw_version` is 11.
  - Both are concatenated into one `__llvm_prf_data` with different record sizes, and only one comdat `__llvm_profile_raw_version` survives. The raw reader also requires an exact version match (`llvm/lib/ProfileData/InstrProfReader.cpp:584` on release/23.x).
  - cc-rs deliberately does **not** forward `-Cprofile-generate`/`-Cprofile-use` to the C compiler, for this exact reason (`$REG/cc-1.8.0/src/flags.rs:131-132,355-367`; rust-lang/cc-rs#1986).
- **Can llvm-profdata 23 write an index that clang 22 accepts? No.**
  - `llvm-profdata` 23.1.1 (rustup llvm-tools) has no option to write an older index version; `merge --help-hidden` offers only `--memprof-version`.
  - **[probe]** It writes index v14 (`0x010000000000000e`), and clang 22 rejects it: "unsupported instrumentation profile format version". clang 21 rejects it too.
  - clang 22's reader rejects any index newer than 13 and accepts older ones (`llvm/lib/ProfileData/InstrProf.cpp:1688-1691` on release/22.x).
  - Text profiles are not accepted by `-fprofile-use` (**[probe]** "bad magic").
  - **Workable path:** instrument the C code with zig's clang 22 and link an LLVM-22 compiler-rt `profile` runtime, which you must build yourself (zig ships none). Do it in a training build **without** `-Cprofile-generate`, so two runtimes never meet.
  - Then merge with an LLVM 21 or 22 `llvm-profdata`. Raw v10 is the same in 21 and 22 (`/usr/lib/llvm-21/lib/clang/21/include/profile/InstrProfData.inc:723-725`). Index v12 from LLVM 21 or v13 from 22 is accepted by clang 22. apt offers `llvm-21-tools` / `llvm-21` 1:21.1.8.
  - Pass it through `CFLAGS_x86_64_unknown_linux_musl` / `CXXFLAGS_…` (cc-rs env vars, `cc-1.8.0/src/lib.rs:70,169-175,256`).
- **How each crate compiles and whether it gets `-ffunction-sections -fdata-sections`:**
  - cc-rs adds both by default on every non-Apple-mobile target (`cc-1.8.0/src/lib.rs:2736-2743`).
  - cmake-rs puts cc's compiler args into `CMAKE_<LANG>_FLAGS` (`$REG/cmake-0.1.58/src/lib.rs:728-742`), so cmake-built C gets them too.
  - BoringSSL forces `-fno-function-sections -fno-data-sections` only under `FIPS_SHARED` (`$REG/btls-sys-0.5.6/deps/boringssl/CMakeLists.txt:345-352`).
  - Per crate: `libmimalloc-sys` cc (one TU, so all of mimalloc's functions sit in one file but still get per-function sections); `libsqlite3-sys` cc (`build.rs:150-170`); `zstd-sys` cc, which also adds the flags itself plus `-fmerge-all-constants` (`build.rs:276,358-360`); `ring` cc (`build.rs:508`); `btls-sys` cmake (`build/main.rs:211-213`); `libduckdb-sys` cc, C++17 (`build_bundled_cc.rs:70-76,110-142`).
  - **[probe]** `zig cc` itself adds no `-ffunction-sections` (cc1 line); cc-rs does.

---

## 5. lld `--symbol-ordering-file` on data

- **Same mechanism for every output section (lld release/22.x `lld/ELF/Writer.cpp`):**
  - `buildSectionOrder` maps each named global or local `Defined` symbol in the file to its input section (`:1104-1165`).
  - `sortInputSections` sorts every output section except `.init`/`.fini` (`:1247-1262,1288-1296`), not only executable ones. `.rodata.*` input sections are reordered within `.rodata`, and `.data.rel.ro.*` within `.data.rel.ro`, with the same priority list.
  - The executable flag only changes where the ordered block goes on thunk targets (`:1173-1245`; x86-64 has no thunk spacing).
  - With `--symbol-ordering-file`, explicit symbols get priorities below any call-graph-profile priority (`:1131`).
  - `--call-graph-ordering-file` cannot be combined with it (`Driver.cpp:1922-1927`).
- **rustc emits a section per symbol for data by default:**
  - `fdata_sections = ffunction_sections`, which defaults to `target.function_sections` = true (https://github.com/rust-lang/rust/blob/1.99.0/compiler/rustc_codegen_llvm/src/back/write.rs, lines 208-211; `rustc_target/src/spec/mod.rs:2882`).
  - **[probe]** On musl, `.rodata.<mangled static>`, `.data.rel.ro.<mangled>` and `.data.<mangled>` each carry a named global symbol, so they are orderable.
- **Anonymous constants are not orderable by name:**
  - **[probe]** rustc puts string literals and other promoted constants in `.rodata..Lanon.<hash>.N` / `.data.rel.ro..Lanon.<hash>.N`. The only symbol is an unnamed `STT_SECTION`, because `.L` names are assembler-local and never reach the symtab. An ordering file cannot name them.
  - Float constants go to `.rodata.cst8` (`SHF_MERGE`) under `.LCPI*` local labels.
- **Merged sections (`SHF_MERGE`: `.rodata.str1.1`, `.rodata.cst*`) cannot be ordered:**
  - `finalizeInputSections` replaces every `MergeInputSection` with a `MergeSyntheticSection` (`OutputSections.cpp:204-260`). It runs in the driver (`Driver.cpp:3492-3501`) before the writer sorts.
  - A symbol in a merge section maps to a `MergeInputSection` that is no longer in the list, so it is silently ignored. `maybeWarnUnorderableSymbol` does not warn for this case (`Symbols.cpp:292-322`).
  - zstd's `-fmerge-all-constants` (above) adds such sections on the C side.
- An alternative that needs no names for data: `--bp-compression-sort=data|both` (`Options.td:150-151`) orders data sections by content similarity, not by access. **No primary source found** on whether that reduces RSS.
