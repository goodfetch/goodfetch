# Qualification matrix

This matrix keeps operational code, unqualified fixtures and experiments to be added distinct. The "everything" scope is an extensible program; no future row is presented as already tested.

| Family | Operational in this lab | Next qualification / decision |
| --- | --- | --- |
| HTTP acquisition | Pagination, record correctness, freshness by seed, Crawlee pool, Website Spider, native fetch; controlled redirect/gzip/chunked/large responses; Spider loopback redirect refused by its SSRF policy | TLS/HTTP2, streaming without materialization, Impit, 429/503, Retry-After, proxy |
| JS rendering | Asynchronous catalog; independent conformance | Complex SPAs, hydration, service workers, blocked network, qualified adaptive fallback |
| Navigation | `domcontentloaded` + content condition | back/forward, redirect, popup, new target, long polling, aborted navigation |
| Clicks | 5/50 actions, ordered effects, overlay and postconditions; Stagehand batch; native/guarded campaigns run on 9 variants including blocked/ambiguous target | actionability on an independent corpus, coordinates, stale reference between guard and action, cancellation with partial effect |
| Input | fill, select, checkbox, input event and server effect | detailed press/type, IME, contenteditable, keyboard/shortcuts, validation change |
| DOM | Extraction and node counting; re-render before interaction | Large parameterized DOMs, mutation between resolution and click, full DOM snapshot, AX, semantic references |
| Frames/shadow | Open shadow; iframe routes available | Same-origin iframe adapter then OOPIF/cross-origin; closed shadow and slots |
| Media/files | Download route and byte oracle available | Download event adapter, upload, screenshots (bytes + visual oracle), PDF |
| Sessions | Fresh page, documented cookies/profile policies | Equivalent independent contexts, login, cookies/storage, reuse/cold, cross-tenant leakage |
| Concurrency | Closed loop, open loop, bounded admission, visible rejections, several writer processes | Capacity curve C1/4/16/64, queues, priority, backpressure toward sink, multi-domain fairness |
| JSON writing | Buffered/sync JSONL, whole sync snapshot + rename | Encoder alone vs sink alone, streaming without materialization, large records, gzip/zstd, sharded files |
| SQLite writing | WAL FULL/NORMAL, DELETE FULL, batched transactions, contention, integrity_check; Node/Rust sync/Go on the same SQLite artifact; short/long readers and checkpoints verified on 108 cells | indexes/upsert, temp store/cache_size, large streaming corpus, isolation of the encoder alone |
| Other sinks | Reusable worker interface | Arrow/Parquet, PostgreSQL/COPY, DuckDB, object/S3, compression; same commit contracts and oracles |
| Incidents | Hung/corrupted worker, descendants, false success; writer crashes before/after commit and during/after ACK; new coordinator after claim/commit; repeated faults with slow writer | Crash during click, browser kill, full disk, corruption, network loss, repeated arbitrary faults |
| Durable workflow | Node and Rust/Tokio HTTP prototypes: acquisition/extraction, oracle, bounded queue, batch, commit, ack, leases, fencing and idempotent recovery; 30 native cells and 42 recovery cells verified | Heartbeat, outbox, continuous admission, multi-domain scheduler, external effects, integrated Go core, Chromium |
| AI | V3 blocked at the model client; v4 primitives without a model | observe/act/extract, discovery/compile/replay/repair, same model and budgets, tokens/cost, fresh/stale cache |
| Topology | Local SDK/worker/browser | Controlled injected RTT, remote vs co-located SDK, remote batch, identical CPU/RAM, cloud/local separated |
| Browsers/OS | Local Chrome for Testing on macOS | Dedicated Linux, Firefox/WebKit, Lightpanda/Hyperspeed, compatibility and unchanged oracles |
| Metrology | Raw data, manifests, SHA, repetitions, quantiles/sample sizes, RSS; macOS CPU/footprint/I/O probe calibrated for runtime/SQLite readers | Network bytes, Linux/cgroups, thermal stability, paired intervals, holdout set |

## Competitors to add

Priority to **Chromey/chromiumoxide native Rust and chromedp/rod Go** to isolate the driver; **Crawlee AdaptivePlaywrightCrawler/Impit, Scrapy, Colly, Scrapling and Crawl4AI** for acquisition/parsing/pools; **agent-browser, Browser Use, Skyvern, Magnitude and AgentQL** for perception/agents. Firecrawl, Browserbase, Browserless, Steel, Spider Cloud, Zyte and the data collection providers belong to separate product/topology lanes. The versions, licenses and reasons for priority are in the [sourced shortlist](../../research/2026-10-07-lab-competitor-shortlist.md).

A library listed in the research is not automatically installed or benchmarked. Adding a player according to a precise architectural question, with a real adapter and an oracle, avoids a catalog of packages without comparable experiments.

## Gates before an architecture decision

1. Exact result and expected effects on a defined corpus; unsupported cases and errors made explicit.
2. Equivalent lifecycles, actionability and durability, or presented as different configurations.
3. Load curves and full cost, not just command time or overall average.
4. Tuning corpus then holdout, perturbations and repetitions on a stable machine.
5. Integrated prototype acquisition → validation → sink → ack; recovery verified before adopting the durable engine.

The historical Node HTTP prototype is described in [workflow.md](workflow.md), and the native Node/Rust comparison in [runtime-workflow.md](runtime-workflow.md). It takes up **native HTTP + bounded queue + batched sink with a single writer**. Rust/Tokio remains the candidate for the portable core; Node serves as a reference and as a compatibility backend. The Rust Chromium driver and the deterministic plans still require their own qualification. The results of each campaign determine its recommendations, without turning a language preference into a measured result.

[Executed cross-cutting qualification and limits](../reports/integrated-2026-10-08.md), [contract audit](../reports/integrated-verification.json). Guards 162/162 on this corpus; this does not qualify all actionability guarantees.
