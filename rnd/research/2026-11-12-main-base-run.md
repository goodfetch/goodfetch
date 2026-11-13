# `main`'s base run: the matrix, the Callgrind cell and three hand-runs of a default crawl (2026-11-12)

This is #2281's run.

- **When:** 2026-11-12 23:54 to 2026-11-13 04:35 UTC.
- **Machine:** the rented Hetzner CCX43 (AMD EPYC-Milan, 16 threads, 61 GiB, Linux 7.0.0-38).
- **Commit:** `a0270a4dc`, which is `main` at `e720fe376` merged into this ticket's branch. The branch's own commits change only `xtask compare`: the strace exception, the trials file kept as a run goes, and Callgrind's output path.
- **Command:** `just compare` at its defaults (every scenario, 10 trials of each measured cell, release profile), which exited 0.
- **Where the files are:** the run is `local-controlled`. Its report, trials, Callgrind outputs, the hand-runs' files, the module readings and the scripts are filed in the private reports repository as `2026-11-12-e720fe376/`.

**In short:**

- On the raw cell, Foxhound fetches 973 pages a second. Spider, the best competitor, fetches 12,595.
- Foxhound executes 4.125 times Spider's instructions per page. That replaces the register's 22.799, which dates from before #1577's pass.
- SQLite is 31 % of one raw page's instructions, mimalloc 9 %, and `crossbeam_epoch` 6.5 %.
- A browser-fetch hop's fetch part, from `Page.navigate` to the body read, takes 33.3 ms. The kept page's floor is therefore 481 pages a second, under Spider's 12,595, so ADR 0020's rule makes `learned` the default without waiting for #2294.
- The stand-in at `fetch.tier: http` executes 6.885 times Spider's instructions per page.

## 1. What the commit holds

- **The matrix is the commit's:** raw, durable, polite, extraction and browser.
  - Each measured cell runs 10 trials, interleaved in random order.
  - Each timed trial started under a 1-minute load of 4: 0.76 to 3.98 across the run's 220 timed trials. Each trial's load is in the report's trials table.
- **None of the four tickets that change the matrix is in it.** #2283 (the polite scenario's 1,000 domains), #2136 (the extraction cell), #2284 (the crawl-like scenario) and #2282 (the Collie scenario) were all open at `e720fe376`. Compare a later reading with this one cell by cell:
  - polite is still the 50-domain site;
  - extraction still has Spider write raw HTML;
  - there is no crawl-like or Collie scenario.
- **Competitors' pins:**
  - Spider 2.53.9, its HTTP mode, with the 64-page broadcast channel;
  - Crawlee 3.18.2 on Node.js: `@crawlee/cheerio`, and `@crawlee/playwright` on Playwright 1.64.1;
  - Scrapy 2.19.0 on CPython, with scrapy-playwright 0.0.48 on Playwright 1.64.1;
  - every browser cell on Chrome for Testing 156.0.8078.4.
- **One timed trial was retried.** Browser, Crawlee, trial 4, stopped after the Proving ground received no request for 120 s. Its retry's figures count.
- **Foxhound's browser cell reads "not measured" for system calls.** Under strace it reached 999 of the 1,000 pages, so by the strace exception its later strace runs did not run. Every other figure of that cell counts.

## 2. The raw cell

Each figure is the median of 10 trials, with the lowest and highest in brackets. Memory is the process tree's.

| | Foxhound | Spider | Crawlee | Scrapy |
|---|---|---|---|---|
| Pages a second | 973.3 (877.1–1040.0) | 12,595.1 (11,702.3–12,951.7) | 258.3 (247.4–263.5) | 401.9 (398.9–406.2) |
| CPU seconds per 1,000 pages | 1.623 | 0.378 | 4.489 | 2.484 |
| Average cores | 1.56 | 4.70 | 1.16 | 1.00 |
| Peak RSS / PSS | 59.3 / 56.4 MiB | 19.3 / 12.0 MiB | 405.3 / 391.7 MiB | 81.3 / 78.1 MiB |
| Latency per page, p50 | 402.3 ms | 22.3 ms | | |
| System calls per page | 63.0 | 11.5 | | |
| TCP connections | 10 | 16 | | |
| Startup to the first request | 99.1 ms | 19.0 ms | | |

The other scenarios' pages a second:

| Scenario | Foxhound | Others |
|---|---|---|
| Durable | `full` 577.1, `periodic` 709.2 | none can resume after a kill |
| Polite | 39.2 | Crawlee 40.9, Scrapy 39.3 |
| Extraction | 780.9 | Spider 12,127.0, Crawlee 165.2, Scrapy 370.0 |
| Browser | 11.7 | Spider 28.1, Crawlee 16.5, Scrapy 28.8 |

## 3. The Callgrind cell and one raw page by module

**`foxhound.spider_instruction_ratio` is 4.125.** That is Foxhound's 2,042,697 instructions per page over Spider's 495,222.

- Each tool ran 3 times on scenario 1, each run of the same work. Foxhound's runs counted 2.042, 2.043 and 2.046 G instructions; Spider's 494.7, 495.2 and 495.5 M.
- The 2026-11-06 gap measurement read 4.30.

`by_module.py` reads the median run.

- It counts each instruction where it executed (a function's own cost) and puts each function in one module: a Rust function in its crate, a C function in the library that defines it.
- SQLite's own static functions (`yy_reduce`, `vdbeRecordCompareString`, …) are put in SQLite by the symbols of the `libsqlite3.a` linked into the build. Without that, they read as 10 % "C code in the executable".

| Module | Foxhound, instructions per page | Share | | Spider module | Instructions per page | Share |
|---|---:|---:|---|---|---:|---:|
| SQLite (C) | 631,312 | 30.9 % | | `libc.so.6` | 250,579 | 50.6 % |
| `libc.so.6` | 415,808 | 20.4 % | | `lol_html` | 81,962 | 16.6 % |
| Rust standard library | 249,990 | 12.2 % | | Rust standard library | 27,960 | 5.6 % |
| mimalloc (C) | 188,508 | 9.2 % | | `rustls_pki_types` | 25,633 | 5.2 % |
| `crossbeam_epoch` | 133,685 | 6.5 % | | `http` | 19,332 | 3.9 % |
| `lol_html` | 71,854 | 3.5 % | | `url` | 18,818 | 3.8 % |
| `url` | 40,868 | 2.0 % | | `hyper` | 12,015 | 2.4 % |
| `rusqlite` | 36,498 | 1.8 % | | `tokio` | 8,253 | 1.7 % |
| `goodfetch_crawl` | 34,076 | 1.7 % | | `encoding_rs` | 7,851 | 1.6 % |
| `tokio` | 32,721 | 1.6 % | | `spider` | 6,996 | 1.4 % |
| `serde_json` | 29,844 | 1.5 % | | `reqwest` | 6,706 | 1.4 % |
| `goodfetch_fetch_tier` | 29,126 | 1.4 % | | `aho_corasick` | 5,383 | 1.1 % |
| `goodfetch_extractor` | 27,173 | 1.3 % | | 48 other modules | 23,734 | 4.8 % |
| `goodfetch_engine` | 18,850 | 0.9 % | | | | |
| `goodfetch_frontier` | 17,803 | 0.9 % | | | | |
| 89 other modules | 84,581 | 4.1 % | | | | |
| **All** | 2,042,697 | 100 % | | **All** | 495,222 | 100 % |

Foxhound's heaviest functions per page:

- `__memcpy_avx_unaligned_erms`: 270,748 (13.3 %);
- `sqlite3VdbeExec`: 95,347 (4.7 %);
- `crossbeam_epoch`'s `Guard::flush`: 53,258 (2.6 %), with `Global::try_advance` and `Global::collect` another 59,848;
- the `BTreeMap<String, serde_json::Value>` inserts, drops and iteration: about 96,000 (4.7 %) together.

Spider spends half its instructions in libc (mostly `memcpy`) and a sixth in `lol_html`.

## 4. The three hand-runs of a default crawl

Every hand-run is set up the same way:

- **Site:** `scale.test/sites/1000/1` (1,000 pages), on a fresh Proving ground.
- **Settings:** only what names the crawl (`network.resolve` for the site's host, `egress.private_ranges` so it may be reached), the scenario's pace (`politeness.domain.delay: 0`, `politeness.domain.concurrency: 16`), and `fetch.tier` where a tier is forced (G-04 Q3).
- **Pages a second:** the Proving ground's 1,000 pages over the engine's wall time from start to exit.
- **CPU seconds:**
  - "waited" is the OS's accounting of the waited-for process tree, user plus system;
  - "sampled" is the engine's CPU plus its children's from `/proc` samples every 50 ms, every browser process counted.
- **Peak memory:** the whole tree's, browsers included, from those samples. RSS counts a page that processes share once per process; PSS counts it once.

| Run | Pages a second | Wall | CPU seconds, waited / sampled | Peak RSS / PSS, process tree | Processes at peak | Requests to the site |
|---|---:|---:|---|---|---:|---|
| `fetch.tier` left at `auto` | 43.4 | 23.06 s | 128.7 / 163.9 | 16.0 GiB / 2.77 GiB | 147 | 1,057 (36 duplicates, 20 `/favicon.ico`) |
| `fetch.tier: http` | 553.7 | 1.81 s | 2.9 / 2.8 | 76.6 MiB / 73.9 MiB | 1 | 1,001 |
| `fetch.tier: browser_fetch` | 51.0 | 19.62 s | 110.5 / 131.1 | 15.3 GiB / 2.42 GiB | 144 | 1,001 |

How to read the three runs:

- **Every run fetched all 1,000 pages.** Each finished `complete`, and its one error is the site's missing `robots.txt`.
- **The `auto` run:**
  - It launched browsers.
  - Its 20 `/favicon.ico` requests are ones only a rendering browser sends; they are its other 20 errors.
  - It is the behaviour of #2514, which landed before this commit. The same hand-run at `24202afd0`, earlier on 2026-11-12, read 9.9 pages a second with 477 errors.
- **The `http` run is a bracket.** That arm sends no `User-Agent`, cookie or proxy until #2315, #1605 and #1640 land.

## 5. The browser-fetch hop's split, and what ADR 0020's rule decides

**How it was timed.**

- The `browser_fetch` hand-run ran with `log.level` at `info,goodfetch::cdp::command=trace`, so the engine's stderr holds one line per CDP command sent, with its time.
- A hop sends, in order: `Target.createTarget`, the page's set-up, `Page.navigate`, the `Fetch` interception's commands, `Fetch.getResponseBody`, `Fetch.continueResponse` once the body is read, then `Target.closeTarget`.
- Each browser's launch probe is left out: its first seven commands, 13 probes.
- Lines carry no hop number. Every hop sends each command once, so the mean time between two commands of a hop is the difference of their mean send times, over the run's 1,001 hops (`hop_split.py`).

| Part of a hop | Mean |
|---|---:|
| Opening: `Target.createTarget` to `Page.navigate` | 101.97 ms |
| **Fetch part: `Page.navigate` to `Fetch.continueResponse` (the body read)** | **33.27 ms** |
| of which `Page.navigate` to `Fetch.getResponseBody` sent | 29.14 ms |
| Closing: `Fetch.continueResponse` to `Target.closeTarget` sent | 27.89 ms |
| A hop's slot at 16 in flight (16 × 18.493 s span / 1,001 hops) | 295.59 ms |
| Left for the close's answer and the wait for the next hop | 132.46 ms |

Opening and closing a page take four times the fetch itself. #2294's kept page, set up once and navigated by each hop, would remove them.

**The kept page's floor** is 16 in flight over the fetch part alone: 16 / 33.27 ms = **480.9 pages a second**.

**The comparison the rule asks for.**

- The best competitor's pages a second on the raw cell of the same run is Spider's **12,595.1**, the whole trial's figure; the steady-rate row arrives with #2175.
- The floor is under it, by 26 times.

**What the rule decides.** By ADR 0020's rule of 2026-11-10, the default `fetch.http_from` becomes `learned`, the ladder, and #2294 is not waited for.

- The value changes in a change of its own, once the Session's own transport is a real client (#2315, #1605, #1640) and the ladder (spec #2413) is built.
- That change also amends ADR 0023's row "Browser-fetch for identity consistency", as ADR 0020's note says.
- ADR 0020 records this reading in a note of 2026-11-13.

## 6. The defaults' instructions per page over Spider's: a stand-in read at `fetch.tier: http`

**A stand-in, not the register's figure.** The `http` hand-run, run three times under Callgrind, counted 3.395, 3.410 and 3.429 G instructions over its 1,000 pages. Its median is 3,409,634 instructions per page, which over Spider's 495,222 in the same run is **6.885**.

- It is read at `fetch.tier: http` on today's transport, which is not yet the real client.
- The register's row `foxhound.spider_instruction_ratio_at_defaults` stays "not run yet" until #2176 reads it on the real client (owner, G-08 Q7 and Q10).

Each module's share of one page at the defaults:

| Module | Instructions per page | Share |
|---|---:|---:|
| SQLite (C) | 819,921 | 24.0 % |
| `libc.so.6` | 375,364 | 11.0 % |
| Rust standard library | 372,051 | 10.9 % |
| `dom_query` | 265,369 | 7.8 % |
| `selectors` | 244,085 | 7.2 % |
| `crossbeam_epoch` | 190,843 | 5.6 % |
| mimalloc (C) | 176,504 | 5.2 % |
| `flatbuffers` | 162,833 | 4.8 % |
| zstd (C) | 108,013 | 3.2 % |
| `cssparser` | 84,323 | 2.5 % |
| `url` | 56,773 | 1.7 % |
| `html5ever` | 54,484 | 1.6 % |
| `rusqlite` | 46,523 | 1.4 % |
| `tokio` | 43,942 | 1.3 % |
| `goodfetch_crawl` | 39,010 | 1.1 % |
| `serde_json` | 38,344 | 1.1 % |
| `tendril` | 37,614 | 1.1 % |
| `dom_smoothie` | 33,028 | 1.0 % |
| `goodfetch_fetch_tier` | 31,965 | 0.9 % |
| `goodfetch_frontier` | 26,599 | 0.8 % |
| 101 other modules | 202,046 | 5.9 % |
| **All** | 3,409,634 | 100 % |

The defaults add 1.37 M instructions per page to the raw cell. Most of the difference is:

- the page's tree and the Record's text: `dom_query`, `selectors`, `cssparser`, `html5ever`, `tendril` and `dom_smoothie`, 0.72 M together;
- the Record's encoding and compression: `flatbuffers` and zstd, 0.27 M;
- SQLite's extra 0.19 M.

## 7. Figures over 1.10 times the pin

`ci/perf-reference.toml` now pins `5d900125e`, #1577's tuned build (G-09 Q5).

- `drift.reference`'s first comparison runs on this ticket's pull request's full gate.
- `xtask perf` fails a figure over 1.10 times its reference whatever `GATING` says, so a full gate that passes means no figure is over it.
- A figure that fails goes to the owner with its cause, for a fix or an amendment of ADR 0140. Each such figure is added here with the commits that moved it. The pin is not moved to pass.

## How to read it again

The scripts are filed with the run in the reports repository:

- `handrun.py` runs one hand-run, timed or under Callgrind;
- `run-handruns.sh` runs the three, then the three counted;
- `hop_split.py` reads the trace;
- `by_module.py` reads a Callgrind output. With `--c-library 'SQLite (C)=…/libsqlite3.a'` it attributes a linked C library's static functions.
