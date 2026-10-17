# Goodfetch Lab — results and decisions to test

Snapshot of **October 7, 2026**, on an Apple M3 Max, 14 logical CPUs, 36 GiB of RAM, macOS 27 arm64. The laboratory is installed and runnable. Crawlee, Spider Rust, Stagehand v4, its batch, the **`@crawlee/stagehand` integration with Stagehand v3**, Playwright and Puppeteer were run locally. The JSON/JSONL and SQLite writes have independent Node and Rust workers.

The first results support three directions: **avoid the browser when HTTP is enough; reuse resources; batch writes behind bounded admission**. They make it possible to propose the next Goodfetch prototype, without yet choosing its language on performance alone or announcing a universally superior scraper.

## Evidence and reproduction

The retained HTML reports, manifests and raw samples are gathered in the [experiment index](index.html). A copy of this evidence is in `evidence/`; the complete storage databases/files remain in the local `lab/results/` folders indicated by the index. The intermediate build attempts are kept, but are not used for the tables below.

Versions run: **Node 25.5.0, pnpm 12.9.1, Rust 1.93.0, Crawlee 3.18.2, Spider 2.53.9, Playwright 1.63.0, Puppeteer 25.12.0, Stagehand 4.1.0 and 3.7.3, SQLx 0.8.6**. Chrome for Testing **153.0.8010.12** is provided explicitly to the adapters. Locks, source SHAs, browser and executables are recorded. No source change was detected during the retained experiments.

```sh
cd /Users/jb/Dev/goodfetch/rnd/lab
pnpm install --frozen-lockfile
pnpm bootstrap
pnpm run doctor
pnpm test
pnpm check
pnpm integration
pnpm bench campaigns/qualification.json
pnpm bench campaigns/http-lifecycle.json
pnpm bench campaigns/arrival-stress.json
pnpm bench campaigns/browser-primitives.json
pnpm storage campaigns/storage-smoke.json
pnpm storage campaigns/storage-contention.json
pnpm storage campaigns/storage-process-crash.json
```

Run the campaigns separately, on an available machine. The figures remain exploratory: the corpus is small, the network is local, and the laptop's thermal state and applications are not controlled as they would be on a reserved machine.

## Engine qualification

The common campaign recorded **270 attempts: 245 verified, 23 unsupported, 2 failed**. It tests HTTP catalog, JS catalog, form, 50 clicks and open shadow; normal, delay, overlay and DOM replacement first pass the compliance gate. The 35 compliant cells then each get one warmup and two measurements. This small number mainly serves to detect integration problems.

**Spider Chrome and Spider Smart failed on the JavaScript catalog**: the six expected records are missing. The measurements for these cells are blocked. Their HTTP results pass. This is a non-qualification of these configurations/adapters on this fixture; a targeted investigation is still needed before attributing the cause to the library, the browser or the DOM retrieval configuration. There is no silent fallback that would convert the failure into a success.

The other missing capabilities are declared: HTTP-only does not handle forms/shadow; Spider is used here as a collection adapter, without an implementation of actions; Puppeteer shadow is not implemented in this adapter. A capability missing from the adapter is not a claim of general incapability of the upstream project.

The interactive pages measure their **real 1280×720 viewport** and the lab checks it. The test site keeps the effects of the interactions and the oracle checks their order, their values and the trusted events. The selectors are known: AI discovery and reasoning are not part of this series.

## HTTP: lifecycles and load

Catalog of **16 pages / 32 records per task**, server delay of 2 ms/page, concurrency of four tasks, three blocks of 24 measurements. All backends verified **72/72 tasks**. Throughput concerns complete verified catalogs, not pages.

| Configuration | Verified tasks/s | p50 task, ms | p95 task, ms |
| --- | ---: | ---: | ---: |
| Node fetch + specialized parser | 88.7 | 44.0 | 50.9 |
| Spider HTTP, Website per task | 92.1 | 42.7 | 45.8 |
| Crawlee HTTP, new crawler per task | 18.3 | 193.8 | 413.2 |
| Crawlee HTTP, crawler/queue shared per block | 25.4 | 81.8 | 542.1 |

Source: [HTTP report](evidence/http-lifecycle-2026-10-07T21-29-31-257Z-2b0754/report.html), [raw data](evidence/http-lifecycle-2026-10-07T21-29-31-257Z-2b0754/attempts.jsonl). The minimalist backends do less orchestration work and do not have the same parser. The small Node/Spider gap here demonstrates no intrinsic language advantage. The Crawlee pool's gain comes with a shared queue and a different internal concurrency; it does not isolate the instantiation cost alone. Its idle queue also shows high latency: the qualification C1 run took about 500 ms for a small catalog. The pool's wake-up settings deserve a separate experiment.

The independent-arrival stress test offers **20 tasks/s**, with an eight-page catalog, a delay of 10 ms/page and at most four active tasks. Two blocks each offer 40 tasks. Node and Spider HTTP verify **80/80**; new-crawler Crawlee verifies **72/80** and rejects eight arrivals for lack of capacity, without error on the admitted tasks. The verified throughput, drain included, is respectively 19.60 / 19.56 / 16.91 tasks/s. Source: [open-load report](evidence/arrival-stress-2026-10-07T21-31-09-975Z-ba6970/report.html).

**Rejections stay in the denominator.** The generator does not artificially slow down to wait for a busy backend. Arrivals concern complete tasks; no unbounded queue quietly absorbs the overload. These two load points do not yet constitute a capacity curve or a sustained test.

## Browser primitives

The repeated campaign uses three blocks, five measurements per scenario and backend, sequences of 50 clicks, the compliance mutations and the same real viewport. It verifies **690 tasks across all phases, including 345 measurements**, and declares three Puppeteer shadow cells unsupported. The measurements contain **4,500 controlled clicks**, and all phases together contain 9,000. No failure was observed in this campaign.

| Backend, 15 measurements per cell | p50 JS catalog, ms | p50 form, ms | p50 50-click task, ms |
| --- | ---: | ---: | ---: |
| Playwright | 149.0 | 346.6 | 2,714.6 |
| Puppeteer | 74.6 | 298.8 | 3,449.2 |
| Stagehand v4 | 138.9 | 213.5 | 945.5 |
| Stagehand v4 batch | 130.9 | 215.8 | 939.4 |
| Crawlee + Playwright | 862.8 | 975.3 | 3,317.8 |
| Crawlee + Stagehand v3 | 882.9 | 765.0 | 3,209.3 |

Source: [browser report](evidence/browser-primitives-2026-10-07T21-33-08-604Z-f6b1a0/report.html), [raw data](evidence/browser-primitives-2026-10-07T21-33-08-604Z-f6b1a0/attempts.jsonl). The time starts at task launch and includes navigation, checks, actions, RPC and oracle; it does not measure the isolated click. The variants follow one another according to the same predefined cycle. With 15 values, the empirical p95 used by the report is the observed maximum: it is not a reliable estimate of a distribution tail in production.

Stagehand v4 is faster than Playwright on this local click sequence. **Batching brings no additional net gain demonstrated here**: the medians are close and their variations do not provide statistical proof. The other operations also show that speed depends on the task; no global ranking should replace these cells.

Stagehand v4 uses its Page/Locator primitives without a model; its batch applies only to clicks. `@crawlee/stagehand` is a **v3** track, with a new browser/crawler per task and a Playwright Page. It remains distinct from v4. A model client that fails on every call is used in this track to detect accidental inference.

The native guarantees remain different. Playwright notably performs its actionability checks; the lab's common postconditions prove these fixtures, without making the SDKs universally equivalent. Stagehand v4 uses an extension close to the browser; Playwright already uses CDP for Chromium. The useful question is therefore about **placement, RPC round trips and guarantees**, not about the mere presence of the CDP acronym. [Playwright Chromium code](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/chromium/crBrowser.ts), [Stagehand v4 migration](https://docs.stagehand.dev/v4/migrations/playwright).

The published Stagehand v4 result on Wikipedia with remote RTT is not a prediction for this local lab. Our series reproduces neither its corpus nor its topology. Injecting controlled RTTs and comparing a remote SDK, a co-located worker and an extension will be needed to qualify the benefit of batching. See the [sourced audit](../../research/2026-10-07-lab-engine-evidence.md).

## Writing: contracts, batching and concurrency

**80/80 contention cells pass**, with 96,960 records read back and verified. Each cell has a new sink; eight producers feed one or four **real writer processes** sharing the same file/database. Two blocks, 24 batches per block, 512-byte payload, batches of 1 or 100 records. Buffered JSONL, fsync JSONL, SQLite WAL FULL/NORMAL and DELETE FULL remain distinct contracts.

**The SQLite versions run differ: Node 3.53.4, Rust 3.46.0.** Node uses DatabaseSync; Rust SQLx uses its asynchronous thread/driver and one connection per writer. These differences prevent attributing the figures to the language alone. The data, transactions, effective PRAGMA values and compilation options are recorded.

| Pipeline, one writer | Batch 1, records/s | Batch 100, records/s |
| --- | ---: | ---: |
| Node JSONL buffered | 5,990 | 312,830 |
| Rust JSONL buffered | 13,769 | 919,313 |
| Node JSONL fsync/batch | 231 | 21,647 |
| Rust JSONL fsync/batch | 243 | 22,950 |
| Node SQLite WAL FULL | 5,337 | 164,624 |
| Rust SQLite WAL FULL | 6,396 | 76,490 |
| Node SQLite WAL NORMAL | 7,326 | 194,998 |
| Rust SQLite WAL NORMAL | 8,767 | 84,617 |
| Node SQLite DELETE FULL | 2,531 | 123,827 |
| Rust SQLite DELETE FULL | 2,248 | 66,887 |

Source: [storage report](evidence/storage-contention-2026-10-07T21-30-24-424Z-6bec59/report.html), [details and oracles](evidence/storage-contention-2026-10-07T21-30-24-424Z-6bec59/summary.json). Throughput aggregated as verified records / sum of write windows. The windows are short; no statistical ranking or sustained throughput is demonstrated. The largest cells contain only 2,400 records: they do not qualify the continuous behavior of checkpoints or large files. Record construction, JSON encoding, RPC, sink and ack logging are included; startup and close/oracle are separate. Rewriting the growing JSON snapshot adds yet another distinct cost. These are not pure encoder microbenchmarks.

Batching has a major effect in these configurations. **Four SQLite writers do not improve these cells**: WAL FULL batch 100 goes from 164,624 to 110,816 records/s on the Node side and from 76,490 to 39,597 on the Rust side. SQLite keeps a single active writer per database; multiplying producers and multiplying connections are not the same thing. [Official WAL documentation](https://www.sqlite.org/wal.html).

The storage smoke test also qualifies the synchronized JSON snapshot with rename and directory sync: **44 cells pass, 4 are unsupported** because several writers would replace the same document without coordination. The three formats do not provide the same services: sequential export, publication of a complete document, queryable transactional state.

The crash probe keeps **2,048 records in 8/8 cells**, after a SIGKILL sent after the acknowledgements. The system and its caches remain active. This verifies a process crash, without proving durability after a power cut or recovery after an interruption in the middle of a transaction. The lab does not enable F_FULLFSYNC and does not establish physical equality of fsync/SQLite guarantees on macOS. [Storage research](../../research/2026-10-07-lab-storage-engineering.md).

## Architecture proposal derived from these measurements

| Layer | Proposal to prototype | Evidence still needed |
| --- | --- | --- |
| Admission and scheduler | Bounded queue, per-domain/job budgets, explicit cancellation | Fairness, priority, saturation and propagation of sink slowness |
| Acquisition | Native HTTP first, output validation, browser upgrade if needed | HTTP2/TLS/proxy, authenticated content, independent corpus, cost of upgrades |
| Browser | Browser/page pools near the worker, deterministic plan with postconditions | Isolation between tenants, CDP crash, frames/OOPIF, RTT and large DOMs |
| AI | Discover/compile/repair outside normal replay, cost limits and validation | Same model/budget, semantic drift, cache failure, holdout |
| Sink | Single batched writer; JSONL export, SQLite WAL for local state | Integrated backpressure, readers/checkpoints, commit window and max batch latency |
| Durability | Leases/checkpoints and idempotent write before ack | Crash between effect/commit/ack, duplication, recovery and ack log |
| Interfaces | SDK/CLI/MCP driving the engine; web console and macOS app | A real common durable HTTP workflow, then browser qualification |

**Rust/Tokio remains the candidate for the portable core**, not a demonstrated winner. Keeping Node backends for compatibility is consistent; a Rust/CDP driver will have to beat a co-located Node baseline at equal guarantees to justify its cost. The current Rust SQLx sink is not automatically faster than Node SQLite. Adding a Rust baseline with a synchronous binding and **the same SQLite build**, then Go, would be a more defensible runtime comparison.

The next decisive deliverable is an **acquisition → extraction → validation → batched writer → commit → ack** prototype, with a bounded queue and idempotent recovery. This lab currently measures acquisition and writing in separate tracks; it does not yet implement this durable Goodfetch workflow. Its integrated latency and consumption remain to be demonstrated before freezing the architecture.

## Remaining coverage and competitors

The [coverage matrix](../docs/coverage-matrix.md) details the future primitives: frames, files, captures, complete AX/snapshots, session reuse/isolation, degraded network, full sink, readers/checkpoints, CPU/I/O, C16/64 load and recovery. iframe/download fixtures already exist, without announced qualified adapters.

Comparison priority: **Chromey/chromiumoxide and chromedp/rod** to isolate the drivers; **Crawlee Adaptive/Impit, Scrapy, Colly, Scrapling, Crawl4AI** for acquisition/pools; **agent-browser, Browser Use, Skyvern, Magnitude, AgentQL** for agents/perception. Firecrawl, Browserbase, Browserless, Steel, Spider Cloud, Zyte and the collection services require a separate topology/cost series. The [sourced shortlist](../../research/2026-10-07-lab-competitor-shortlist.md) gives the reasons and the sources; their presence in this list does not mean they are already installed.

## Laboratory verification

Final checks: **14/14 unit tests, 1/1 real Stagehand integration**, `pnpm check`, both `cargo fmt --check` and `pnpm run doctor` pass. Doctor receives responses from the 12 backends and the two writers; it does not replace their functional qualification. The 108 local links checked exist, the 161 evidence files match their SHAs, and the HTML index was rendered and its storage filter verified. See the [verification record](verification-2026-10-07.json).

The automated tests exercise the contracts: missing/duplicate/wrong data, false success of a worker, protocol corruption/out-of-order messages, timeout and recalcitrant descendants, saturation and deadline, sink reopen and statistical denominators. A real Stagehand integration checks clicks with overlay and page dimensions. Performance has no fragile threshold in the unit tests.

The references and method prescriptions are in the [test engineering best practices](../../research/2026-10-07-lab-testing-engineering.md) and the [storage research](../../research/2026-10-07-lab-storage-engineering.md). The scripts make it possible to repeat these experiments and to add an incorrect adapter as a negative control before considering its timings.
