# Audit of public benchmarks: Stagehand, Crawlee and Spider-rs

Survey of October 7, 2026. This document examines sources and scripts; it does not present a comparative benchmark run by us. The only local check is a small Node expression that confirms the argument handling and URL concatenation of the Spider stub. The comments on defects concern the code pinned below, without presuming which code was used for the older published measurements.

## Conclusion

**None of the benchmarks examined makes it possible to rank Stagehand, Crawlee and Spider-rs for all of Automan's tasks.** The Spider benchmark compares a Rust library with Colly, the Node `crawler` package and wget; Crawlee and Stagehand do not appear in it. An agent benchmark compares Stagehand 3.2.1 with Browser Use and Sentinel on a few tasks. A crawler benchmark mainly measures Markdown outputs and their usefulness for a document retrieval pipeline. These experiments concern different contracts.

The published results remain useful for formulating hypotheses. They demonstrate neither "Rust is always faster", nor "a script always beats an agent", nor the current reliability of Stagehand 4.1.0. An honest ranking requires equivalent pages, outputs and final states, as well as the costs of failed attempts.

## 1. Spider: read the script before quoting the multipliers

Sources pinned at commit **`fb0f270d77e19442de59350d9dbbacf52f1804b4`**, corresponding to `main` at the time of the survey: [published protocol](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/BENCHMARKS.md), [crawl.rs harness](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/crawl.rs), [Go stub](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/go_colly.rs), [Node stub](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/node_crawler.rs).

The published table notably announces a crawl of 185 pages in 73 ms for Rust, 32 s for Go and 15 s for Node on a Mac M1 Max. It describes machines and a remote site, but the current harness does not guarantee that each competitor finishes with the same URLs and the same content. These are the vendor's figures, not measurements we have reproduced.

### Visible asymmetries

- **Libraries compared:** `spider`, Colly, the Node `crawler` package, wget. The Node row does not refer to Crawlee, Playwright or Stagehand.
- **Timing:** Spider is called in-process with a Tokio runtime created before the measurement; Go, Node and wget are launched in a process at each iteration. In decentralized mode, the worker is started before timing. This experiment can measure the throughput of a warm run, but does not constitute an equivalent comparison of startups or of the total cost of workers.
- **Work done:** the harness asserts neither the number of valid pages, nor URL equality, nor content quality. Command outputs are collected without any assertion on their success code. A run that does less work can therefore appear fast.
- **Go concurrency:** the stub calls `colly.NewCollector()` without enabling `colly.Async()`. The [Colly source](https://github.com/gocolly/colly/blob/master/colly.go) explicitly exposes this option; the generated run does not enable asynchronous concurrency in the program. Colly environment variables could change the configuration, but they are not set by this harness. This row therefore does not represent a comparable optimization of Colly.
- **Network:** remote sites, caching, server rate limits and blocks affect the results. The link and resource policies of the stubs do not constitute a verified common specification.

### Two problems in the current Node stub

The program is launched with `node ./node-crawler.js <url>`, but takes `argv.slice(1)[0]` as the base URL. In this usual invocation, that yields the script path. It also builds links by `base + href` concatenation, even when `href` is already an absolute URL.

Limited local check, without running a crawl:

```js
const argv = ['/usr/bin/node', './node-crawler.js', 'https://fixture.invalid'];
argv.slice(1)[0]; // './node-crawler.js'
'https://fixture.invalid' + 'https://fixture.invalid/page';
// 'https://fixture.invalidhttps://fixture.invalid/page'
```

**Permitted conclusion:** fix and instrument these stubs before any comparative reproduction. These observations do not prove that the older measurements used exactly this code, and do not allow concluding that there was an intent to deceive. They prevent reusing the table today as proof of Spider's superiority over Crawlee or Stagehand.

## 2. Agents: a Stagehand 3.2.1 result is not a Stagehand 4 result

The [browser-agent-benchmark](https://github.com/ArasHuseyin/browser-agent-benchmark) repository compares its author's product, Sentinel, with Browser Use and Stagehand. The author declares this conflict of interest. Audit pinned at commit **`8672aaa405ad371d947a2e84f850f20464cb6350`**: [methodology](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/METHODOLOGY.md), [aggregated results](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/results/aggregated/summary.json), [limitations](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/docs/limitations.md).

The JSON results are dated **April 23, 2026**, use `gemini-3-flash-preview` and give, after direct aggregation of the fields:

| Declared configuration | Tasks | Attempts | Successes | Duration and token samples present |
| --- | ---: | ---: | ---: | ---: |
| Browser Use 0.12.6 | 9 | 45 | 45 | 45 |
| Sentinel 4.1.2 | 9 | 45 | 45 | 45 |
| Stagehand 3.2.1 | 9 | 45 | 39 | 39 |
| Stagehand hybrid 3.2.1 | 4 | 20 | 18 | 18 |

The totals are calculated from the JSON, not estimated from a chart. They were not reproduced by running the tasks. The older prose of `METHODOLOGY.md` still presents a different series, notably 32/45 for Stagehand. The file and the series cited must therefore be identified precisely.

The aggregated durations and tokens here cover only successes. The methodology acknowledges that failed runs are excluded from the cost statistics, including very resource-intensive attempts. The displayed average cost is therefore not enough to calculate **the total cost per success**, which must include all attempts. The p95 values computed with five runs per task are also very uninformative.

The documentation says the raw data is committed. Yet at this SHA, the `results/raw/` tree contains only `.gitkeep`; only the summaries are visible in `results/aggregated/`. This prevents recalculating the distributions of all attempts from this repository. It does not prove that logs do not exist anywhere else.

Finally, the modes differ: Stagehand is used with primitives and a hybrid variant, Browser Use with an autonomous loop. The comparison has use-case value, but does not cleanly separate all variables. April's tasks and versions do not determine October's result for Stagehand 4, another model, recurring workflows or a massive crawl.

## 3. Markdown/RAG crawlers: different objective, different configuration

The [llm-crawler-benchmarks](https://github.com/AIMLPM/llm-crawler-benchmarks) repository is maintained by the developers of markcrawl, one of the tools compared; the product interest is declared. Audit at commit **`02d513cf491732450c30b26b71a7e06a6ca4d49e`**: [effective methodology](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/reports/METHODOLOGY.md), [Crawlee runner](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/runners/crawlee_runner.py), [worker](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/crawlee_worker.py).

The pipeline examines the crawl, Markdown conversion, content, document retrieval and answers. This measures a useful end goal, but not an authenticated interaction or the safety of a click. The different versions of the documentation describe successive corpora and stages; avoid mixing their tables as if they were a single experiment.

The Crawlee configuration studied uses **Python and Playwright**, with one subprocess per call to work around an event loop difficulty described by the runner. This startup is included in the duration. It represents neither Crawlee TypeScript, nor `HttpCrawler`, nor `CheerioCrawler`, nor all browser pool configurations. HTTP baselines and browser configurations also provide different services.

The methodological text distinguishes repetitions of the speed crawl from document quality measurements, some of which remain on a single series. A model-based relevance check is not an independent oracle of truth. **Spider and Stagehand are not compared in this experiment**; no ranking of the trio follows from it.

## 4. Stagehand evaluations and Reddit anecdotes

The [Stagehand evals](https://www.stagehand.dev/evals) page describes a model evaluation, displayed as updated on **July 10, 2026**. It does not constitute a Stagehand/Crawlee/Spider comparison. Models, versions and objectives must accompany any reuse of metrics.

The [audited Reddit testimonials](2026-10-07-reddit-evidence-followup.md) report problems with sessions, cost and apparent success. They do not provide a common protocol for the trio. An April 2025 report announcing more than 500,000 tokens for an agent does not automatically describe the current Stagehand 4. A gain from a Browserbase + Stagehand stack cannot be attributed to the SDK alone.

## 5. What Automan must publish to prove its advantage

The general protocol remains detailed in [benchmark and runtime](2026-10-07-benchmark-and-runtime.md). This research specifies the comparators to isolate:

| Contract | Relevant comparators | Verified result |
| --- | --- | --- |
| HTTP crawl | Spider HTTP, Crawlee HTTP/Cheerio, Scrapy, Colly async, Automan HTTP | Same set of URLs, expected content, coverage, errors |
| Rendered crawl | Spider Chrome, Crawlee Playwright, Crawl4AI, Automan CDP | Same content after JavaScript, declared resources and isolation |
| Known actions | Playwright, Stagehand page/locator, agent-browser CLI, Automan plans | Observed final state, unique effects, safe recoveries |
| Semantic discovery | Stagehand act/observe/extract, Browser Use, Skyvern, spider_agent, Automan | Same model if compatible, same budget and task oracle |
| Repeated execution with drift | Stagehand cache/replay, Scrapling heuristics, Automan workflows | Success after controlled mutations, false successes, stale data |
| Incident and recovery | Framework + storage actually configured | No undeclared lost work, no duplicate effects, actionable diagnosis |

Publish separately cold and warm startup, initial decision and replay, browser and LLM time, volume actually validated, total memory including child processes, p50/p95 with enough samples, total cost of all attempts per success and anonymized traces. Pin versions, machines, authorized sites, caches, proxies, concurrency, robots policies and delays. A ranking without these contracts often confuses less work with more performance.

**Architecture decision:** keep Rust as the common runtime hypothesis, while requiring measured gains per subsystem. Existing benchmarks justify a comparative experiment; they do not constitute its conclusion.
