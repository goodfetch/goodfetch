# How much does each peak-memory cut save on the shipped musl executable?

Date: 2026-11-07. Ticket: [#2162](https://github.com/goodfetch/goodfetch/issues/2162), on the map [#2141](https://github.com/goodfetch/goodfetch/issues/2141). It measures, on the static musl executable ADR 0024 ships, each cut that [#2151](https://github.com/goodfetch/goodfetch/issues/2151) decided, after [#2150](https://github.com/goodfetch/goodfetch/issues/2150) measured the gnu build only (`research/foxhound-peak-memory`). Its figures fill the "(to confirm by #2162)" markers of the peak-memory spec draft [#2165](https://github.com/goodfetch/goodfetch/issues/2165).

**Answer.** On the shipped static musl executable, at 16 in flight in scenario 1, with Spider in the same rounds (kernel peaks in MiB, median of 3, every executable dropped from the page cache before each trial):

| Build | 100 pages | 1,000 pages | 10,000 pages | Idle |
|---|---|---|---|---|
| As shipped | 86.2 | 97.4 | 106.0 | 32.3 |
| Huge pages off (set before mimalloc's constructor) | 43.9–44.2 | 47.9–48.6 | 53.3–53.7 | 17.0–17.9 |
| + PGO | 30.9 | 34.0 | 41.8 | 15.5 |
| + PGO + linker order file (**best measured, non-PIE as shipped**) | **29.1** | **32.7** | **35.9** | 15.3 |
| The same as static-PIE with packed relocations (ASLR) | 30.2 | 33.3 | 39.2 | 16.2 |
| **Spider** | **18.3** | **22.1** | **52.3** | |

- **Huge pages off** saves 42–52 MiB and costs no measurable speed (−0.9% wall time at 1,000 pages, 95% interval −3.6% to +3.1%; +2.1% at 10,000, within that size's ±9% noise). **Set in `main`, `allow_thp = 0` does nothing**: mimalloc's constructor maps and advises the arena before `main`. It works, exactly as `MIMALLOC_ALLOW_THP=0`, when set from a constructor that runs before mimalloc's (§1).
- **Packed relocations do not apply to the shipped build**: Zig links the musl executable as a static **non-PIE** (`ET_EXEC`), so it has no runtime relocations and no ASLR for its image. As a static-PIE it would cost 5–6 MiB; with DT_RELR, which musl's start-up applies, 0.5–1.5 MiB (§2).
- **Hot code laid out together** is the big cut left: PGO halves the executable's resident pages at peak (31.2 → 15.8 MiB), and an order file from the same profile takes them to 14.7 (`.text` 26.2 → 9.7 MiB). PGO also makes the crawl **7.6–7.8% faster in wall time and 15% cheaper in CPU**; the order file changes neither. A PGO release build is about two full builds plus 23 s of training (§3).
- **Lazy start-up** is worth 2.9 MiB of resident code on today's layout but only 0.8 MiB once the code is laid out, plus about 0.6 MiB of heap from `CountedWork` (§4).
- **The bar:** At 10,000 pages Foxhound is level with Spider with the huge-pages cut alone (53.3–53.7 against Spider's 52.3–54.2 in the same rounds) and 16 MiB under it with the layout (35.9). At 1,000 and 100 pages it stays **10.6 and 10.8 MiB above** after every cut measured, and about 9 MiB above once lazy start-up is added (estimated). What remains is the executable's 14.7 MiB against Spider's 5.5, and 19 MiB of private memory against Spider's 12.5 (§5).
- **Every linker-side cut needs a toolchain change**: Zig's `cc`, through which cargo-zigbuild links, refuses `-z pack-relative-relocs` and `--symbol-ordering-file` and ignores `-static-pie`. The figures above come from Zig's own LLD (`zig ld.lld`) run on the same inputs Zig's `cc` passed it (§2, §3).
- **Measurement trap:** the executable's resident pages swing by up to 9 MiB with the page cache's state, so the spec must name the state the bar is read in. `just compare` also measures the gnu build today, not the musl executable (§6).

## Setup

- **Tree:** `origin/spec/1577` at `64c31c0ec`, which has `just compare`'s raw cell and the musl build path of #2114 (`ci/musl/cargo`: cargo-zigbuild 0.24.1, Zig 0.17.1, its bundled musl and LLD 22.1.8).
- **Host:** the Hetzner CCX43 (16 threads, 64 GB, Linux 7.0.0-38, ext4). Transparent huge pages: `enabled = madvise`, `defrag = madvise`. Shared with other agents; every build and measurement ran through `load-guard` with `LOAD_GUARD_HEAVY=1`, one heavy suite at a time.
- **Builds**, profile `release` (fat LTO, one codegen unit), target `x86_64-unknown-linux-musl`, feature `executable`:
  - `fh-base`: Foxhound as it ships (`ci/musl/cargo build`), 120.7 MB.
  - `fh-x`: a measurement-only variant (`thp-modes.py`, reverted after each build, in no commit) that adds four ways to turn mimalloc's huge pages off, chosen at run time by `PEAKCUT_THP` (§1). Unset, it behaves as `fh-base`.
  - Linker-only variants are **re-links** of `fh-x`'s or the PGO build's saved link: `link-save.sh` is rustc's linker for the final link; it runs cargo-zigbuild's own linker with `ZIG_VERBOSE_LINK=1`, saves the `ld.lld` command Zig ran and every input, and `relink.sh` re-runs it through `zig ld.lld` with extra flags. A re-link with no extra flag reproduces `fh-x` byte for byte (same SHA-256).
  - Spider 2.53.9 (`xtask/compare-spider`), the Proving ground and `xtask`: host (gnu) release builds, as in #2150.
- **Trials:** `xtask compare trial --scenario raw` (scenario 1 of ADR 0138's matrix), 16 in flight, against a fresh Proving ground (seed 1574), driven by #2150's `trial.sh`, `counted.sh` and `wrap.sh` with this run's paths. **Peak** is the trial's `peak_bytes`, the kernel's maximum RSS of the tool's process. Each figure is the median of 3 trials, with the configurations interleaved within each trial round, and Spider measured in the same rounds.
- **Instruments:** #2150's `sampler.py` (RSS every 10 ms; at each new peak, `smaps`, `status` and the executable's present pages from `pagemap`) and `binattr.py` (present pages onto sections and crates), run on `fh-x-syms`, which is `fh-x` re-linked without `-s`: the same loaded sections at the same addresses, with the symbol table kept.
- **Page cache:** every executable, Spider's included, is dropped from the page cache before every trial (`evict.py`, `posix_fadvise(DONTNEED)`), so each trial pages it in through its own faults, as after a reboot. §6 says why this matters.
- **Scripts** are in [`2026-11-07-peak-memory-cuts/`](2026-11-07-peak-memory-cuts/), with this run's absolute paths (`/Users/jb/agent-targets/peak-cuts`).

## 1. Huge pages off, set in code

mimalloc v3 (`libmimalloc-sys` 0.1.49) initializes in a constructor that runs before `main`: `mi_process_attach`, `__attribute__((constructor(101)))` under clang (`c_src/mimalloc/v3/src/prim/prim.c:35-43`). It calls `_mi_os_init` (`os.c:98`), which reads `allow_thp` and, when it is 0, calls `prctl(PR_SET_THP_DISABLE)` (`prim/unix/prim.c:223-232`). Each OS allocation then calls `madvise(MADV_HUGEPAGE)` only while `allow_thp` is set (`prim.c:413`). The first allocation, which reserves the arena, also comes before `main`. Four ways to set the option, one binary (`fh-x`), kernel peaks in MiB:

| How `allow_thp = 0` is set | 100 | 1,000 | 10,000 | Idle | `THP_enabled` | Works? |
|---|---|---|---|---|---|---|
| Not set (as shipped, `fh-base`) | 86.2 | 97.4 | 106.0 | 32.3 | 1 | |
| `MIMALLOC_ALLOW_THP=0` in the environment (`fh-base`) | 45.1 | 49.5 | 53.4 | 18.3 | 0 | yes |
| 1: `mi_option_set` in `main`, beside the purge delay (#2151's wording) | 91.3 | 97.4 | 101.3 | 32.1 | 1 | **no** |
| 2: as 1, plus `prctl(PR_SET_THP_DISABLE)` in `main` | 47.0 | 50.5 | 55.2 | 19.8 | 0 | yes, 2–3 MiB above |
| 3: `mi_option_set` in a constructor placed before mimalloc's (`.init_array.00100`) | 44.2 | 48.6 | 53.3 | 17.9 | 0 | **yes, as the variable** |

- **Set in `main`, the option does nothing**: the arena is already mapped and advised for huge pages, and the per-process `prctl` is never made. `AnonHugePages` stays at 26–48 MiB at peak, as shipped (30–36).
- **Mode 3 matches the environment variable** at every size, in private memory too (RssAnon 13.2 / 18.7 / 22.5 MiB, against 7.7 / 18.7 / 22.3; the 100-page figure differs by when the sampler caught the peak, not by size). The constructor runs before mimalloc's because LLD sorts `.init_array.NNNNN` by priority and musl runs the array in order. It allocates nothing. The engine setting that turns huge pages back on is read after `main` starts, when the option is already 0 and `prctl` already set: turning them back on then means `prctl(PR_SET_THP_DISABLE, 0)` and `mi_option_set(allow_thp, 1)`, which advises only the memory mapped afterwards. Alternatively the constructor reads the setting's environment variable itself (mode 3 reads `PEAKCUT_THP` with `getenv`, which musl allows that early).
- **Inherited by child processes:** `PR_SET_THP_DISABLE` survives `fork` and `exec`. With mode 3, mode 2 or the variable, every process the engine starts (Collie's browsers) runs with transparent huge pages off. The spec should say whether that is wanted, or clear it (`prctl(PR_SET_THP_DISABLE, 0)`) in the child before `exec`.
- **Mode 2** works but leaves 2–3 MiB more private memory than mode 3: whatever the arena got as huge pages before `main` stays.

### Wall time with huge pages on and off

Scenario 1, `fh-x` unset (on) against mode 3 (off), no sampler, 30 pairs per size, the order within each pair alternating, `user + system` CPU from the trial's own figures:

| | On | Off | Off − on, paired median (95% bootstrap interval) | Sign test |
|---|---|---|---|---|
| 1,000 pages, wall | 1.007 s, 993 pages/s | 1.014 s, 986 pages/s | −9.4 ms, **−0.9%** (−3.6% to +3.1%) | off slower in 13 of 30, p = 0.59 |
| 1,000 pages, CPU | 1.59 s | 1.59 s | −0.1% (−4.4% to +2.5%) | |
| 10,000 pages, wall | 8.74 s, 1,144 pages/s | 8.51 s, 1,175 pages/s | +181 ms, +2.1% (−8.8% to +9.5%) | off slower in 17 of 30, p = 0.59 |
| 10,000 pages, CPU | 14.21 s | 13.97 s | +2.2% (−4.7% to +6.2%) | |

**No speed loss is detectable.** At 1,000 pages the interval rules out a loss above 3.1% in wall time. At 10,000 pages the crawl's own spread (6.7–9.9 s on the same binary) bounds the effect only to about ±9%; the medians favour huge pages off. On this host, at load averages 1.9 to 4.2.

## 2. Packed relative relocations

**The shipped musl executable is not position-independent.** `fh-base` is `ET_EXEC`, linked `-static -no-pie` at base `0x1000000`, with no `.rela.dyn` and no dynamic section. rustc asks for a static-PIE: its linker command for the musl target passes musl's `self-contained/rcrt1.o`. Zig's `cc` ignores `-static-pie` and links a non-PIE with its own `crt1.o`. So:

- The image has **no ASLR** today. #2151 rejected a non-PIE build because it loses ASLR, and the shipped build already is one.
- It has **no runtime relocations**: the 5.1 MiB of relocation pages #2150 found is a cost of the gnu PIE build only. On the shipped build, nothing remains of it to cut.

To measure the cut as #2151 meant it, `fh-x`'s saved link was re-linked as a static-PIE (`pie-args.sh`): musl's self-relocating `rcrt1.o` in place of `crt1.o`, `-pie`, base 0. Zig's runtime archives (libc, libc++, libc++abi, libunwind, ubsan_rt, zigc, compiler_rt) had to be the ones Zig builds for a `-fPIE` link (`pic.list`): with the shipped ones LLD stops at `R_X86_64_64 cannot be used against local symbol` in musl's `vfscanf.o`. The Rust and C objects themselves were already PIC.

- **musl applies DT_RELR.** Zig 0.17.1's musl `rcrt1.c` includes `ldso/dlstart.c`, which applies `DT_RELR` after `DT_REL` and `DT_RELA` (`dlstart.c:143-156`), as musl has since 1.2.4. The packed binary starts at a randomized base (`randomize_va_space` 2) and crawls every page correctly in every trial below.
- **Sizes:** the static-PIE has 140,115 relative relocations, a 3.21 MiB `.rela.dyn`. Packed, `.relr.dyn` is 35 KiB, 1.1% of it.

| THP off (mode 3) | 100 | 1,000 | 10,000 | Idle | Executable's pages at 1,000 | RssAnon at 1,000 |
|---|---|---|---|---|---|---|
| Non-PIE, as shipped | 44.2 | 48.6 | 53.3 | 17.9 | 31.2 | 18.7 |
| Static-PIE | 50.2 | 54.2 | 58.8 | 22.5 | 36.3 | 20.9 |
| Static-PIE, packed relocations | 45.8 | 49.1 | 56.7 | 18.9 | 32.6 | 19.7 |

**Toolchain changes this cut needs**, and how these figures were made:

- A static-PIE at all: Zig's `cc` ignores rustc's `-static-pie` and links `-no-pie`. In a probe, `zig cc -fPIE` did link a static-PIE (`-static -pie`, `rcrt1.o`, Zig's PIC runtimes); passing `-fPIE` through cargo-zigbuild was not tried.
- Zig's runtime archives built for PIE: the ones a non-PIE link uses are not PIC.
- `-z pack-relative-relocs`: Zig's `cc` refuses it ("unsupported linker extension flag"); so does it `--pack-dyn-relocs=relr`. These figures come from Zig's LLD run directly (`zig ld.lld`, LLD 22.1.8) on the inputs Zig's `cc` passed it, with the substitutions above: a link Zig's own linker produces, but not through `zig cc` or cargo-zigbuild as pinned.

ASLR through static-PIE costs about 5–6 MiB: the relocation table read at start (3.2 MiB) and `.data.rel.ro`'s 2.1 MiB, written at start and so private. Packed relocations remove the first: **static-PIE with DT_RELR costs about 0.5–1.5 MiB over today's non-PIE**, the dirtied `.data.rel.ro`. The 10,000-page median (56.7) includes one trial at 53.0; the three were 53.0, 57.6 and 56.7.

## 3. Hot code laid out together

`fh-x`'s code laid out three ways, all with huge pages off (mode 3):

- **PGO:** an instrumented build (`-Cprofile-generate`), trained on scenario 1 (an idle start, then 100, 1,000 three times and 10,000 pages, all complete), merged with `llvm-profdata` (rustup's `llvm-tools`), then a profile-guided build (`-Cprofile-use`). Rust code only: the C and C++ Zig compiles get no profile. The trained crawl ran 8,178 of 127,969 instrumented functions (6.4%).
- **Order file** (`orderfile.py`): those 8,178 functions, hottest first, given to LLD as `--symbol-ordering-file`, so they sit together at the start of `.text`. Built from the same profile; applied both to the PGO build and, alone, to the plain one.
- **Section prefixes** (`-z keep-text-section-prefix`): LLD keeps PGO's `.text.hot`, `.text.unlikely` and `.text.startup` apart.
- BOLT was not tried: it is not installed on the host, and needs `--emit-relocs`, which Zig's `cc` refuses too.

| THP off | 100 | 1,000 | 10,000 | Idle | Executable's pages: idle → 1,000-page peak | `.text` at peak |
|---|---|---|---|---|---|---|
| Plain (`fh-x`) | 43.9 | 47.9 | 53.7 | 17.0 | 13.7 → 31.2 | 26.2 |
| Order file, no PGO | 38.1 | 41.9 | 46.4 | 16.0 | 13.8 → 24.8 | |
| PGO | 30.9 | 34.0 | 41.8 | 15.5 | 10.8 → 15.8 | |
| **PGO + order file** | **29.1** | **32.7** | **35.9** | 15.3 | 10.6 → **14.7** | **9.7** |
| PGO + order file + section prefixes | 29.5 | 35.2 | 41.9 | 14.8 | 10.1 → 14.9 | |
| PGO + section prefixes, no order file (first pass) | 33.5 | 37.1 | 41.2 | 14.7 | 10.1 → 19.2 | |
| PGO + order file, static-PIE + DT_RELR | 30.2 | 33.3 | 39.2 | 16.2 | 11.5 → 15.7 | |
| Spider | 18.3 | 22.1 | 52.3 | | 5.5 | |

- **PGO does most of it.** It moves cold code out of the hot paths and the build comes out 15% smaller (102.9 MB against 120.6), so the kernel's fault-around maps far fewer cold neighbours. The order file adds about 1 MiB at peak on top of PGO, and 6.4 MiB on a build without PGO. Section prefixes help a little at idle and hurt at peak, so they are not worth keeping.
- **What stays resident at peak** in the best layout (`fh-pgo-order`, 14.7 MiB): `.text` 9.68, `.rodata` 3.66, `.data.rel.ro` 1.27. By crate, `.text`: `goodfetch_engine` 1.25, `goodfetch_crawl` 0.82, other C 0.67, DuckDB 0.52, std 0.51, Tokio 0.51, SQLite 0.51, `goodfetch_fetch_tier` 0.38, `goodfetch_protocol` 0.35 (`runs/attr-d-order-r2-raw-1000-c16.txt`). `.rodata` is now a quarter of what is left; nothing here orders it.
- **Wall time**, scenario 1, interleaved pairs, huge pages off in both:

| | Plain | PGO + section prefixes (first pass, its order file empty) | Paired difference (95% interval) | Sign test |
|---|---|---|---|---|
| 1,000 pages, 20 pairs, wall | 1.035 s, 966 pages/s | 0.918 s, 1,089 pages/s | **−7.8%** (−10.8% to −5.3%) | faster in 17 of 20, p = 0.003 |
| 1,000 pages, CPU | 1.64 s | 1.35 s | **−15.4%** | |
| 10,000 pages, 20 pairs, wall | 8.38 s, 1,194 pages/s | 8.22 s, 1,217 pages/s | **−7.6%** (−10.5% to −1.1%) | faster in 14 of 20, p = 0.12 |
| 10,000 pages, CPU | 14.64 s | 12.95 s | **−15.3%** | |

  The order file alone, PGO with it against PGO without, 20 and 10 pairs: −0.3% (−2.3% to +4.7%) at 1,000 pages, +0.4% at 10,000, CPU +0.4% and +1.4%: no effect either way.
- **Build duration** on this host (16 threads, shared): a plain musl release build from an empty musl target 926 s; the instrumented build 924 s (it reused some crates compiled before a restart, so a cold one is no shorter); training and merge 23 s; the profile-guided build from an empty target 751 s. A PGO release is therefore about **two full release builds plus 23 s**. If the profile is committed, as #2165 plans, only the profile-guided build runs at release, at a plain build's cost; the instrumented build and the training run only when the profile is refreshed. The order file is a linker flag: a re-link takes about 1 s.
- **Toolchain changes this cut needs:**
  - PGO use (`-Cprofile-use`) builds through cargo-zigbuild as pinned, unchanged. `-Cllvm-args=-no-pgo-warn-mismatch` keeps a slightly stale profile quiet (this LLVM spells it so, not `-pgo-warn-mismatch=false`).
  - The **instrumented** build's link fails through Zig's `cc`: rustc passes `-u __llvm_profile_runtime` as two arguments and Zig's `cc` reads the symbol as an input file. It links once passed as `-Wl,-u,__llvm_profile_runtime` (`link-save.sh` rewrites it).
  - `llvm-profdata` comes from rustup's `llvm-tools` component; `rust-toolchain.toml` does not list it.
  - The order file needs `--symbol-ordering-file`, which Zig's `cc` refuses. These figures come from Zig's LLD run directly (`zig ld.lld`, LLD 22.1.8) on the inputs Zig's `cc` passed it: a link wrapper doing that, or another linker set-up, is needed.

## 4. Lazy start-up

Resident code of each part on the musl build, from `fh-x-syms`'s pages at the 1,000-page peak, huge pages off (idle in brackets), with why it runs during a code-free crawl. The call chains come from reading `spec/1577`; the two strongest (the commit path and the keychain probe) were checked at the cited lines.

| Part | Resident code at peak (idle), MiB | Why it runs in a code-free crawl | Could be |
|---|---|---|---|
| JSON Schema validation: `jsonschema` 0.82, `schemars` 0.54, `referencing` 0.19, `fancy_regex` 0.16 | **1.71** (0.30) | `crawl.start` declares the Dataset `pages` (`code_free.rs:148-203`) with a schema that schemars generates at run time (`goodfetch-protocol/src/page.rs:1636-1640`, from `code_free.rs:208`). `Datasets::declare` compiles it with `jsonschema::validator_for` (`goodfetch-dataset/src/declaration.rs:135, 523-528`), and every commit validates each Record against it (`lease.rs:277` → `declaration.rs:481-483`). That is also #2150's 0.60 MiB of heap. | A schema built once per process (or a constant) and a validator built on first use. Not validating the engine's own Records would be a design change against ADR 0138. |
| `zbus` (D-Bus) | **0.78** (0.03) | `crawl.start` → `Settings::open` → `host_defaults()` (once per process) → `secrets::host_defaults` → `keychain.available()` (`goodfetch-engine/src/secrets.rs:197`) opens a Secret Service session, only to choose `run_state.key_reference`'s host default. That default is read only when a Run's data key is made or unwrapped (`runs.rs:3285-3335`). | Probed when a data key is first needed. |
| `clap` (`clap_builder`) | **0.39** (0.39) | `main` → `parse(start)` builds the whole derived `CommandLine` for every start (`executable.rs:460-483, 837-856`). `attached-engine` needs only its variant and two flags. | A fast path for `attached-engine` before clap. |
| `CountedWork::default()` caches | heap, about **0.58** (#2150's heaptrack, not re-measured) | `Positions::of` builds a new `CountedWork` on every call (`goodfetch-crawl/src/work_items.rs:444`): on every commit (`lease.rs:281` → `staging.rs:201`) and every staged admission (`staging.rs:487`). Several once-per-process placeholders build their own too. The Payload counts made at commit and staging land in these throwaway instances, so they are lost: a counting defect as well as an allocation. | One instance per engine, passed down. |

Together the three code parts are **2.88 MiB** of the executable's resident pages at peak (0.72 at idle), plus about 0.6 MiB of heap.

## 5. What remains, part by part

Each page count, best measured build (PGO + order file, huge pages off, non-PIE as shipped), against Spider in the same rounds, and the kernel's split at the median trial:

| | 100 pages | 1,000 pages | 10,000 pages |
|---|---|---|---|
| Foxhound | 29.1 | 32.7 | 35.9 |
| Spider | 18.3 | 22.1 | 52.3 |
| **Foxhound − Spider** | **+10.8** | **+10.6** | **−16.4** |
| Foxhound, executable's pages (RssFile) / private (RssAnon) | 14.7 / 15.7 | 14.7 / 19.1 | 14.7 / 21.8 |
| Spider, file-backed (executable 5.4–5.5 + libc) / private | 7.7 / 5.5 | 8.0 / 12.5 | 7.5 / 46.3 |

The 1,000-page peak, part by part (`fh-pgo-order`, 33.8 MiB at the sampled peak):

| Part | MiB | Spider | Note |
|---|---|---|---|
| Executable `.text` | 9.7 | | was 26.2 before the layout cut |
| Executable `.rodata` | 3.7 | | unchanged by any cut here |
| Executable `.data.rel.ro` and data | 1.3 | | |
| mimalloc's arena (the 1 GiB mapping) | 11.4 | | #2150's live heap is 4.6 MiB of it; the rest is per-thread page slack |
| Other private mappings of 1–3 MiB | 4.2 | | not attributed here |
| Thread stacks (24 threads) | 2.2 | | Tokio starts one worker per core: 16 here |
| Other private memory | about 1.3 | | |
| **Total** | **33.8** | **22.1** | Spider: executable 5.5, libc and loader 2.5, private 12.5 at 1,000 pages (13.9 in phase A) |

- **Lazy start-up**, not built here, would take about 0.8 MiB of code (on this layout) and 0.6 MiB of `CountedWork` heap off: **about 27.7, 31.3 and 34.5 MiB, still 9–10 MiB above Spider at 100 and 1,000 pages.** This is an estimate by subtraction.
- **ASLR:** as static-PIE with DT_RELR, 30.2 / 33.3 / 39.2: 0.6–3.3 MiB more (the dirtied `.data.rel.ro` and run-to-run spread at 10,000 pages).
- **Left for the bar at 100 and 1,000 pages**, none measured here: `.rodata` (3.7 MiB; data symbols can go in an order file too), Tokio's worker count (#2150: 4 workers instead of 16 took 3–4 MiB off with huge pages off, on the gnu build), mimalloc's per-thread slack (about 7 MiB of the arena), and the unattributed 1–3 MiB mappings. Spider's own private memory is 5.5 MiB at 100 pages and grows with the site (46 MiB at 10,000), while Foxhound's grows from 15.7 to 21.8.

## 6. A measurement trap: the page cache

The executable's resident pages depend on how the file came into the page cache, not only on the binary. Idle, the same `fh-base`:

| State of the file in the page cache | Executable's resident pages, idle |
|---|---|
| Just written by `cp` | 23.1 MiB |
| As found after other work | 16.0 MiB |
| Dropped (`posix_fadvise(DONTNEED)`), then paged in by this run's faults | 14.1 MiB |
| Run again after that | 14.9 MiB |

A file written through `write()` sits in the page cache in large folios, and a fault maps the whole folio; demand faults bring smaller ones. The first pass of this study, without eviction (`runs-a1/`), found 7 MiB more executable pages for `fh-x` than for `fh-base` (40.6 against 33.1 at peak), though they differ by one small function. `just compare` builds the executable and then runs it, so its figures can carry up to about 9 MiB of this. It also builds `xtask test-tools --cargo-profile release` without `--shipped-target`, so today it measures the host's gnu build, not the musl executable this bar is about (reported from #2165's blast radius on `origin/main` `5d900125e`; `just targets` and `just rss` do pass it). The spec's bar should name the page-cache state it is read in. Dropping the file before each trial, as here, is the reproducible choice: it matches a first run after a reboot.

## Not verified

- macOS (no transparent huge pages; the owner's machine) and arm64 were not measured.
- PGO here instruments Rust code only: the C and C++ compiled by Zig (SQLite, mimalloc, BoringSSL, zstd, brotli, DuckDB) get no profile and are not in the order file. The profile is trained on scenario 1 alone; a profile for release would cover the other scenarios too, and its gain on them is not measured.
- The lazy start-up gain is an estimate by subtraction (§4, §5); no lazy build was made.
- The 1–3 MiB private mappings of §5 are not attributed.
- The wall-time results are from one shared host (load averages 1.9–4.2); at 10,000 pages the huge-pages result bounds the effect only to about ±9%.
- Static-PIE, packed relocations and the order file were made by re-linking with `zig ld.lld` (Zig's PIC runtime archives taken from a separate `zig c++ -fPIE` build for static-PIE); the release-build way to get them (a link wrapper, `-fPIE` through cargo-zigbuild, or another linker set-up) was not built.
- The CountedWork heap figure is #2150's gnu measurement; the call chains in §4 come from reading the code, not from a trace.
- The kernel's folio sizes (§6) are inferred from the RSS figures; `/proc/kpageflags` needs root.
