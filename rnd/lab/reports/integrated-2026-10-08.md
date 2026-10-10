# Automan cross-cutting qualification — October 8, 2026

**Rust/Tokio remains the candidate for the portable core.** Node/TypeScript is the experimental control and the compatibility path. The native Rust workflow reaches throughput close to Node's with less measured CPU and footprint; none of these campaigns yet qualifies a Rust Chromium/CDP driver.

The prototype executes **acquisition → extraction → validation → bounded queue → batched writer → commit → ACK**, separately in Node and Rust. The writer is a dedicated process. Both ledgers load the same SQLite 3.53.4 library and the same generic C SQL shim, in WAL FULL; their job logic is independent. Extraction is specialized for the lab corpus.

The first suite covers the five requested domains, including transports and saturation. A second suite requalifies the Rust configuration, click-stability and SQLite-initialization fixes. The campaigns are sequential; the initial data and diagnostics remain preserved.

[First suite](../results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0/summary.json), [requalification](../results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2/summary.json). Sources/binaries unchanged during the retained campaigns. [Reproducible analysis](integrated-analysis.json), [SHA-256 catalog](integrated-evidence-catalog.json), [source audit](integrated-source-audit.json), [contract audit](integrated-verification.json).

## Full runtime: Rust keeps its appeal

**30/30 cells verified**, three randomized blocks, **47,232 jobs / 1,511,424 records**. C × 192 jobs per cell, 16 pages × 2 records/job, server delay 4 ms/page, C1/4/16, batch 1/4 with B ≤ C. Work windows of about fifteen seconds. Weighted throughput = exact jobs / work time + finalization. Full throughput adds startup and the external oracle.

| C | Jobs/batch | Node jobs/s | Rust jobs/s | Node full/s | Rust full/s | Node CPU ms/job | Rust CPU ms/job |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 11.39 | 11.88 | 11.20 | 11.83 | 9.48 | 2.63 |
| 4 | 1 | 47.48 | 48.90 | 46.54 | 48.55 | 4.61 | 1.77 |
| 4 | 4 | 45.29 | 46.94 | 44.41 | 46.57 | 4.18 | 1.44 |
| 16 | 1 | 197.70 | 196.66 | 190.93 | 192.48 | 2.78 | 1.42 |
| 16 | 4 | 189.11 | 195.58 | 182.96 | 191.46 | 2.52 | 1.23 |

At C16/B1: about 197 jobs/s for both implementations; Rust uses 1.42 ms CPU/job versus 2.78 for Node, i.e. about 49% less measured CPU. Maximum sampled footprint per cell: Rust 136–178 MiB, Node 314–325 MiB. This supports Rust for efficiency at comparable tested guarantees, without proving a universally faster scraper.

Copying the entire configuration corpus per Rust producer was replaced by immutable `Arc` sharing. The first C16/B1 series gave 205.55 Rust / 205.30 Node jobs/s and 204–248 / 306–327 MiB of footprint. The two series were run at different times on a shared Mac: the change is real in the code, but this memory and throughput difference is not an isolated causal test of the `Arc` alone. The first series and its binary are preserved. B4 batching of the full workflow does not systematically beat B1: acquisitions, actual batch size and drain also matter.

[summary](../results/runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e/summary.json), [manifest](../results/runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e/manifest.json)

[First native series](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json), [previous Rust binary](evidence-integrated/artifacts/automan-workflow-rust-before-arc).

## HTTP acquisition: comparing lifecycles and saturation

**24/24 cells**, 23,040 jobs / 737,280 records in the durable workflow. The runtime is Node when an external backend is used; resources then include its descendants. Native Rust is another complete implementation. A shared pool and a fresh crawler per task have different costs.

| Runtime/backend | C | Weighted jobs/s | Min–max per block | CPU ms/job |
| --- | --- | --- | --- | --- |
| node/crawlee-http-pooled | 16 | 20.13 | 9.56–49.83 | 43.55 |
| node/crawlee-http-pooled | 4 | 26.17 | 20.44–31.04 | 45.29 |
| node/native | 16 | 154.58 | 109.57–195.40 | 4.45 |
| node/native | 4 | 47.63 | 47.47–47.95 | 5.14 |
| node/spider-http | 16 | 187.89 | 186.79–189.20 | 5.36 |
| node/spider-http | 4 | 37.07 | 26.02–47.07 | 9.22 |
| rust/native | 16 | 142.19 | 90.66–198.77 | 2.02 |
| rust/native | 4 | 49.82 | 49.71–50.05 | 1.55 |

Large gaps between blocks appear, particularly for Crawlee pooled and some C16 cells. The host diagnostic indicates other heavy activity; it documents a possible confounding factor and does not prove the cause of each slowdown. No general runtime penalty can be inferred from these variations alone.

[summary](../results/runtime-http-backends-2026-10-07T23-29-49-072Z-2b2b75/summary.json), [manifest](../results/runtime-http-backends-2026-10-07T23-29-49-072Z-2b2b75/manifest.json)

[Load diagnostic](host-load-2026-10-08.json).

The open-loop campaigns offer 20/100/250 tasks/s, concurrency 4, three 12 s windows, eight pages with a 3 ms delay. They concern acquisition alone, without a durable ledger. A refused task counts in the denominator; admitted tasks may finish after the window. `queueMs` measures generator lag, with no hidden business queue.

| Offered/s | Backend | Exact | Offered | Rejected | Exact/s | p95 success ms |
| --- | --- | --- | --- | --- | --- | --- |
| 20 | node-http | 719 | 720 | 1 | 20.00 | 40.90 |
| 20 | spider-http | 720 | 720 | 0 | 20.03 | 44.96 |
| 20 | crawlee-http | 563 | 720 | 157 | 15.50 | 453.23 |
| 20 | crawlee-http-pooled | 717 | 720 | 3 | 19.82 | 142.47 |
| 100 | node-http | 3593 | 3600 | 7 | 99.61 | 32.64 |
| 100 | spider-http | 3590 | 3600 | 10 | 99.55 | 33.14 |
| 100 | crawlee-http | 844 | 3600 | 2756 | 23.27 | 215.84 |
| 100 | crawlee-http-pooled | 2433 | 3600 | 1167 | 67.29 | 72.69 |
| 250 | node-http | 4330 | 9000 | 4670 | 120.12 | 32.84 |
| 250 | spider-http | 4331 | 9000 | 4669 | 120.10 | 33.18 |
| 250 | crawlee-http | 982 | 9000 | 8018 | 27.05 | 187.58 |
| 250 | crawlee-http-pooled | 2559 | 9000 | 6441 | 70.80 | 72.76 |

Node fetch and Spider HTTP are good simple paths on this corpus. The pool avoids some of the costs of a fresh crawler per task, without erasing its guarantees and orchestration overhead. The rejections at 250/s expose bounded capacity; low latency on successes alone does not mean the entire load was served.

[summary](../results/http-open-20-2026-10-07T23-52-50-976Z-19b9fb/summary.json), [manifest](../results/http-open-20-2026-10-07T23-52-50-976Z-19b9fb/manifest.json)

[summary](../results/http-open-100-2026-10-07T23-55-27-645Z-9f6596/summary.json), [manifest](../results/http-open-100-2026-10-07T23-55-27-645Z-9f6596/manifest.json)

[summary](../results/http-open-250-2026-10-07T23-58-01-322Z-66a668/summary.json), [manifest](../results/http-open-250-2026-10-07T23-58-01-322Z-66a668/manifest.json)

Transport: Node/fresh Crawlee/pooled Crawlee each verify 15 conformance checks and 45 measurements in the first series. Spider refuses the three loopback redirects: its native SSRF policy blocks hops to a private address, even when the local start is intentional. This is not proof that it cannot follow public redirects. The policy was not bypassed; the [preserved Spider 2.53.9 source](evidence-integrated/artifacts/spider-2.53.9-website.rs) contains `is_ssrf_redirect` and `ssrf_limited_policy`. The separate normal/gzip/chunked/large-response series gives **60/60 rows verified** (45 normal measurements; the variants are conformance checks).

[summary](../results/http-transport-2026-10-07T23-52-40-042Z-be9fbc/summary.json), [manifest](../results/http-transport-2026-10-07T23-52-40-042Z-be9fbc/manifest.json)

[summary](../results/spider-http-transport-supported-2026-10-08T00-26-16-511Z-7ce401/summary.json), [manifest](../results/spider-http-transport-supported-2026-10-08T00-26-16-511Z-7ce401/manifest.json)

## Browser: Stagehand's raw gain requires explicit guarantees

Six backends × nine variants × three blocks. The native policy adds only the wait for document initialization. The common guard enforces uniqueness, visibility, enablement, stability over two intervals between three RAF observations spaced at least 16 ms apart, then hit-test. The oracle verifies target, order, phase and trusted event. For the three negative variants, it requires refusal + zero effect; a verified cell therefore does not always imply a click.

The initial guard had four false successes on a moving target. A repeated public test reproduced them; the fix passed sixteen interactions and then the new matrix. The first attempt, limited to moving the observation into RAF, still failed: its diagnostic log remains preserved.

| Backend | Native: conformant/27 | Fixed guard: conformant/27 | Non-conformant native variants |
| --- | --- | --- | --- |
| stagehand-batch | 3 | 27 | ambiguous, never-uncovered, hidden, moving, never-enabled, overlay, rerender, delayed |
| playwright | 27 | 27 | — |
| crawlee-stagehand | 27 | 27 | — |
| stagehand | 4 | 27 | rerender, hidden, never-uncovered, ambiguous, moving, never-enabled, overlay, delayed |
| crawlee-playwright | 27 | 27 | — |
| puppeteer | 16 | 27 | rerender, overlay, never-uncovered, ambiguous |

| Guarded backend | p50 normal task ms | p50 moving target ms | p50 normal startup ms |
| --- | --- | --- | --- |
| puppeteer | 180.85 | 340.61 | 505.58 |
| crawlee-playwright | 1,008.81 | 1,288.09 | 519.98 |
| stagehand | 185.36 | 338.87 | 834.62 |
| stagehand-batch | 179.87 | 359.75 | 856.81 |
| playwright | 323.78 | 489.70 | 530.72 |
| crawlee-stagehand | 792.72 | 974.65 | 855.28 |

The historical 50-click result (Stagehand 0.95 s / Playwright 2.71 s) used common availability conditions. The new matrix shows why it should not be turned into an immediate replacement for Playwright: the tested native Stagehand primitives often fail on overlay, enablement, mutation and ambiguity. Playwright, Crawlee/Playwright and the Crawlee/Stagehand v3 extension pass this native matrix; the extension's handler uses a Playwright page attached via CDP.

The fixed guard provides another explicitly tested configuration; it does not prove all of Playwright's guarantees nor the removal of the race between guard and click. Refusals and successful clicks have distinct latencies. Stagehand v4 does not expose the same locator timeout: guards/Playwright/Puppeteer budget 800 ms, Stagehand outer deadline 8 s. Startups are separated. No LLM call is made; observe/act/extract and self-heal remain unmeasured.

[summary](../results/browser-actionability-2026-10-07T23-38-33-163Z-ccad1f/summary.json), [manifest](../results/browser-actionability-2026-10-07T23-38-33-163Z-ccad1f/manifest.json)

[summary](../results/browser-actionability-2026-10-08T00-21-16-186Z-b21a28/summary.json), [manifest](../results/browser-actionability-2026-10-08T00-21-16-186Z-b21a28/manifest.json)

[Browser contract](../docs/browser-actionability.md), [red regression](qualification-guard-red.log), [RAF attempt still failing](qualification-guard-green.log), [fixed regression](qualification-guard-calibrated-green.log).

## SQLite: batches, writers, readers and checkpoints

**108/108 cells verified**, **3,456,000 records**. Node N-API, synchronous Rust FFI and Go/cgo load the same C artifact: W1/4, B100/1000, readers absent/short/long, three blocks. Each cell contains 32,000 rows of 512 bytes, WAL FULL, eight producers. The write windows are short; the oracle rereads every record and integrity after closing.

| Binding | Writers | Batch rows | Write rows/s | Full-cycle rows/s | p95 RPC ACK ms |
| --- | --- | --- | --- | --- | --- |
| go-sync | 1 | 100 | 208,926 | 113,184 | 8.61 |
| go-sync | 1 | 1000 | 244,607 | 116,784 | 42.30 |
| go-sync | 4 | 100 | 196,643 | 101,639 | 13.09 |
| go-sync | 4 | 1000 | 242,298 | 117,333 | 127.11 |
| node-shared | 1 | 100 | 166,064 | 82,091 | 13.11 |
| node-shared | 1 | 1000 | 207,894 | 86,986 | 58.52 |
| node-shared | 4 | 100 | 189,052 | 83,242 | 13.43 |
| node-shared | 4 | 1000 | 183,632 | 83,065 | 140.92 |
| rust-sync | 1 | 100 | 197,406 | 108,411 | 8.44 |
| rust-sync | 1 | 1000 | 268,954 | 126,678 | 37.31 |
| rust-sync | 4 | 100 | 178,049 | 96,881 | 20.76 |
| rust-sync | 4 | 1000 | 269,638 | 125,840 | 104.07 |

B1000 increases W1 throughput in all three bindings. W4 is not systematically slower: Node/B100 wins in this short series and Rust/B1000 is close to W1. Large W4 batches do, however, markedly increase p95 RPC latency, with no consistent throughput gain; the historical results and this series motivate the single writer as the baseline, not a universal rule.

The 31 startup failures of the first Rust/Go W4 matrix occur before insertion. The schema and WAL mode were being created simultaneously. Common initialization is now performed once before opening the real concurrent writers; the W4 test with a long reader went from red to green. The writers remain independent during the measured window: the fix does not artificially serialize their writes.

A long reader must keep seeing zero rows during the inserts. PASSIVE then leaves non-checkpointed frames; after release, TRUNCATE succeeds with the WAL at zero. Short readers run 2 ms transactions every 10 ms, with a bounded increasing count. All these invariants are checked in the audit. A dedicated writer and batched transactions remain the starting point supported by these experiments; several writers do not remove SQLite's single write lock. [SQLite WAL](https://www.sqlite.org/wal.html).

The p95 RPC values come from the raw ACKs. The CPU/footprint measurements for this track include the already-running coordinator, whose initial counters are subtracted; its earlier allocations prevent ranking the languages' intrinsic memory. The historical JSON/JSONL, durable snapshot, NORMAL/FULL and SQLx results remain in the earlier reports, with their separate contracts and builds.

[summary](../results/sqlite-readers-2026-10-08T00-25-35-434Z-92d4f3/summary.json), [manifest](../results/sqlite-readers-2026-10-08T00-25-35-434Z-92d4f3/manifest.json)

[Readers/checkpoints contract](../docs/sqlite-readers.md), [diagnostic of the 31 failures](sqlite-initialization-diagnostics.json), [red regression](qualification-writers-red.log), [six fixed cases](qualification-writers-green.log), [historical writes](workflow-2026-10-08.md).

## Durability: two runtimes, same observable invariants

**42/42 cells verified**, 1,008 jobs / 8,064 records, 42 writer restarts and 12 coordinator restarts. SIGKILL before commit, after commit, during ACK, after ACK; new coordinator after claim or commit; three repeated faults with a slowed writer. The initial series also had 42/42 cells verified.

| Runtime/case | Cells | Writer restarts | Coord. restarts | Full-cycle jobs/s | p50 coord. recovery ms |
| --- | --- | --- | --- | --- | --- |
| node/native/4/2/after-ack/0 | 3 | 3 | 0 | 17.22 | — |
| node/native/4/2/after-commit/0 | 3 | 3 | 0 | 17.36 | — |
| node/native/4/2/batch-committed/0 | 3 | 0 | 3 | 17.30 | 282.57 |
| node/native/4/2/before-commit+during-ack+after-ack/20 | 3 | 9 | 0 | 6.78 | — |
| node/native/4/2/before-commit/0 | 3 | 3 | 0 | 17.42 | — |
| node/native/4/2/during-ack/0 | 3 | 3 | 0 | 17.32 | — |
| node/native/4/2/job-claimed/0 | 3 | 0 | 3 | 17.11 | 1,057.86 |
| rust/native/4/2/after-ack/0 | 3 | 3 | 0 | 21.23 | — |
| rust/native/4/2/after-commit/0 | 3 | 3 | 0 | 21.09 | — |
| rust/native/4/2/batch-committed/0 | 3 | 0 | 3 | 20.32 | 59.95 |
| rust/native/4/2/before-commit+during-ack+after-ack/20 | 3 | 9 | 0 | 7.21 | — |
| rust/native/4/2/before-commit/0 | 3 | 3 | 0 | 20.95 | — |
| rust/native/4/2/during-ack/0 | 3 | 3 | 0 | 21.18 | — |
| rust/native/4/2/job-claimed/0 | 3 | 0 | 3 | 20.60 | 1,017.54 |

Results and the receipt are committed before ACK, then ACK is recorded in a separate transaction. Replaying an already-committed job reuses the receipt; old lease tokens are rejected. The external oracle reopens the database after shutdown and compares every record. The checkpoints at crash time attest a claim without results or a commit with results before recovery.

For coordinator recovery, throughputs use the full external time: the work time of the recovered process alone would omit part of the incident. The result queue remains bounded by C even with a slow writer. These tests prove the prototype's invariants under process crashes; they do not qualify power loss, exactly-once external effects, heartbeat or cloud cluster.

[summary](../results/runtime-recovery-qualified-2026-10-08T00-20-09-515Z-ad3e70/summary.json), [manifest](../results/runtime-recovery-qualified-2026-10-08T00-20-09-515Z-ad3e70/manifest.json)

[Workflow and reproduction](../docs/runtime-workflow.md).

## Verification, reproducibility and choices still open

The post-campaign audit passes **510 checks**, including trusted/refusal effects, bounds, restarts, checkpoints and frozen sources. It complements the record oracles executed in the campaigns.

The **57 contract tests pass**, zero failures/skips, including false success, protocol/timeout, recovery of both runtimes, fencing, macOS CPU calibration, moving-target click and W4 startup. TypeScript and Rust formatting pass; the current Rust binary was rebuilt before requalification. [Test log](qualification-current-tests.log), [build](qualification-current-build.log).

The manifests pin versions, seeds, environment, and SHAs of sources and binaries. The sources are copied before the tests and audited again afterwards. The catalog references the original evidence without exporting the oracle's private inputs; the full databases and exports remain in the results folders. The analysis aggregates raw events for quantiles, without averaging per-block p95s. Comparator errors and the lab's initial errors remain distinguished.

The macOS probe is calibrated against getrusage and converts Mach counters with `mach_timebase_info`. It measures the native coordinator and descendants, excludes fixture/oracle in the native track, samples at 100 ms and may miss spikes or process exits. RSS may count shared pages several times; footprint and I/O bytes are distinct indicators. The first series with erroneous CPU units was interrupted and excluded. A 400 ms close timer was fixed before the retained campaigns.

The lab currently supports: continuing with Rust for the durable HTTP core; keeping Node for reference and compatibility; preferring a simple HTTP path when rendering is not needed; keeping Playwright as the robust interaction reference and evaluating Stagehand with its explicit guarantees; starting SQLite with one writer and batched transactions. These are prototype directions grounded in this corpus, not a frozen final architecture.

It remains to qualify a co-located Rust/CDP driver against Node, dedicated Linux, TLS/HTTP2/proxies/429 and retry, general/streaming parsing, a holdout corpus, multi-tenant sessions, continuous admission/heartbeat, additional sinks and perception/LLM with identical budgets. The next experiments must resolve these specific uncertainties before any promise of universal performance.

[Coverage matrix](../docs/coverage-matrix.md), [gates and evidence](../docs/qualification-next.md), [index of all experiments](index.html).
