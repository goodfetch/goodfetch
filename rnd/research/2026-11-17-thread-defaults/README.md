# #2200: the thread defaults measured, 2026-11-17

Spec #2165, issue #2200. These are the measurements of 2026-11-17 for the defaults of `governor.worker_threads` and `governor.blocking_threads`, the scripts that read them, and the window still to run.

## Setup

- **Commit:** `b715958c3`, compare's release musl build, profile-guided and order-linked.
- **Machine:** Hetzner Cloud CCX43, with 16 dedicated vCPUs (AMD EPYC-Milan) and 64 GB. It is the gate host, shared with the slots' builds.
- **Command:** `just compare release --machine 'Hetzner Cloud CCX43, 16 dedicated vCPUs' --cell <scenario>/foxhound [--foxhound-setting …]`. Only Foxhound's cells ran, 10 trials each, behind compare's quiet guard. Every timed trial started at a 1-minute load under 4.
- **Notation:**
  - "defaults" is 2 workers with a cap of 4 (`origin/main` on that day).
  - "per_core/512" is `governor.worker_threads = "per_core"` with `governor.blocking_threads = 512`.
  - "cap 512" is `governor.blocking_threads = 512` alone.
- **Statistics:** p is an exact two-sided Mann-Whitney U on 10 against 10 trials. For the pooled startup (n = 100 a side), it is the normal approximation.
- **Departures:**
  - **No strace.** A PATH shim hid `strace`, so the system-call cell is NotRun. The criteria read memory, time and CPU, and `strace` slows a browser cell about 14 times (#2683).
  - **Medians come from `trials.json`.** With `--cell`, compare's report check fails on the cells not asked for (#2742).
  - **The rss trials of that day are left out.** They ran at loads up to 14.

## Results

| File | What it holds |
|---|---|
| `results/compare-defaults-percore512.md` | Every scenario, defaults against per_core/512 (09:54Z and 14:19Z) |
| `results/compare-defaults-b-percore512-b.md` | The durable and polite cells re-run at low load |
| `results/compare-defaults-c-percore512-c.md`, `results/compare-defaults-c-block512-b.md` | raw and the sizes cells: defaults, per_core/512 and cap 512, run back to back (19:46Z–20:09Z) |
| `results/caps-startup.md`, `results/caps-startup-morning.md` | Foxhound's time to its first request, pooled over raw and the sizes cells |
| `results/daemon.md` | 8 and 16 concurrent Runs of 10,000 pages on one Daemon |

### Readings

**Startup is the cap's.**
- With cap 512, Foxhound's first request comes 0.2 s sooner than at the defaults: a median of 0.510 against 0.712 s, n = 100 a side, p 3.4e-6. The morning pair gives p 1.2e-21.
- This explains the small-site gains: sizes 100p@64 is −39.6% (p 0.004), 100p@1 is −26.7% (p 0.029) and 1000p@16 is −27.6% (p 0.005).
- Cap 512 costs 0.2–0.9 MiB of peak memory on those cells.
- On 2026-11-17 the owner made the cap's default `per_core`.

**Workers on one crawl: one small loss for the defaults, on polite.**
- The durable cells' first gap (−23 to −31%) did not hold at low load: +1.4% (p 0.436) and +4.0% (p 0.190).
- polite is −0.4% for per_core/512 (p 0.043), and −0.9% (p 0.035) in the first run: 0.3–0.7 s in 77 s. These runs do not tell the workers' share from the cap's; step 2 of `window.sh` does.
- per_core workers lose on sizes 10000p@16 (+8.6%, p < 0.001) and long (+28%, p < 0.001).
- They raise every peak: 10000p@64 goes from 44.2 to 67.4 MiB.

**Workers on concurrent Runs: 2 workers lose.**

| Runs | Wall: defaults → per_core/512 | Change | p | CPU | Peak RSS | Threads |
|---|---|---|---|---|---|---|
| 8 × 10,000 | 15.602 → 12.763 s | −18.2% | 0.0021 | — | 86.5 → 108.5 MiB | — |
| 16 × 10,000 | 57.456 → 24.307 s | −57.7% | < 0.0001 | 300.8 → 218.3 s | 117.9 → 150.4 MiB | 74 → 112 |

By the spec's rule, the default becomes the smallest count with no loss. That bisection (2, 4, 8 and per_core workers) is step 3 of `window.sh`.

**`F1.analysis_handoffs` on the CPU pool's own threads** (`just counts`, `b715958c3`, 2026-11-17 17:00Z):
- 1 per page against a limit of 1 in every cell, and 0 in `learning_and_block_classification_off`.
- `F1.analysis_handoffs.handler` is 0 in every cell, and 2 against a limit of 2 in `document_tier_handler`.

**Peak at 1,000 pages.**
- compare's raw/foxhound peak is 29.05 MiB, where #2198's `xtask rss` reading was 21.6 MiB.
- Private memory is 12.35 against about 7.6 MiB, and file-backed memory about 16.7 against 14.0 MiB.
- Its causes are not yet measured: that is step 5 of `window.sh`.

## Still to measure: `window.sh`

Run it from the repository's root, at the commit to measure, with that commit's `just compare` build. Run it only while the gate queue and every slot are idle. On a CCX43 it takes about 2.5 hours.

```sh
PC='"per_core"' rnd/research/2026-11-17-thread-defaults/window.sh   # PC=16 on a CCX43 until governor.blocking_threads takes per_core
```

| Step | What it settles | Read with |
|---|---|---|
| 1 | Caps 512, 4, 8, 6, 5 and per_core, on raw and the sizes cells (the owner's confirmation window of 2026-11-17) | `GF_COMPARE=target/compare caps.py 2200-cap512 2200-cap4 2200-cap8 2200-cap6 2200-cap5 2200-capPC` |
| 2 | per_core workers alone, at the default cap, on raw, the sizes cells and polite | `compare2.py 2200-wdefault 2200-wPC` |
| 3 | 2, 4, 8 and per_core workers on 8 and 16 concurrent Runs of a Daemon | `summ.py daemon target/research-2200/daemon.json` |
| 4 | The cap's saving and the peak at 100, 1,000 and 10,000 pages, with `xtask rss measure` as #2198 read them | `summ.py rss target/research-2200/rss/cap4.jsonl …/cap512.jsonl …/capPC.jsonl` |
| 5 | The causes of the gap between 29.05 and 21.6 MiB, at each Foxhound's peak | `attr.py target/research-2200/attr/p<k>` and `sections.py target/x86_64-unknown-linux-musl/release/goodfetch-foxhound target/research-2200/attr/p<k>` |

`attr.py` and `sampler_lib.py` are copies of #2198's, from `research/peak-memory-gap`. `sections.py` names the executable's resident pages by ELF output section, because compare's build writes no link map.
