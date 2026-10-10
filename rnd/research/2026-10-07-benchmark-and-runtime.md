# Automan — performance, runtime and evidence protocol

Research from October 7, 2026. The findings below come from primary sources; the proposed choices and thresholds are explicitly recommendations. No Automan framework was implemented and no comparative benchmark was run during this study.

## 1. The right unit of performance

An HTTP crawler, a browser that executes JavaScript, an interaction driver and an agent loop provide different services. The number of pages per second is not enough to compare an agent that fills in a form with a crawler that downloads HTML. The protocol must fix the requested result, the execution conditions and the validator before timing anything.

**Recommendation: optimize the cost and latency of a verified business result**, with a minimum reliability frontier. Also publish throughput, model calls, tokens, bytes transferred, the memory of the entire process tree and human interventions. A missing, partial or stale result, or one attributed to the wrong account, is a failure, even if the program finishes without an exception.

The distinction between a successful action and an accomplished goal is central in WebArena: its evaluators check the effects of the task, rather than a resemblance between click sequences. That does not make it a benchmark representative of all customers. [WebArena paper](https://arxiv.org/html/2307.13854v4), [canonical repository](https://github.com/web-arena-x/webarena).

## 2. Rust: real interest, evidence required

The total time includes network waits, browser activity, inference, protocol round trips, framework work and retries. Their durations and overlaps must be instrumented; a simple sum of CPU time does not describe the critical path.

**Illustrative calculation, not a measurement:** if 5% of a workflow's time comes from host code that Rust could speed up tenfold, the total gain is `1 / (0.95 + 0.05 / 10) = 1.047`, i.e. about 4.7%. This calculation assumes the other costs are unchanged. Rust can also reduce memory, change concurrency or simplify distribution; these benefits must be measured separately.

Vercel already offers a native Rust tool, `agent-browser`. The current README states that its daemon requires neither Node nor Playwright. The Rust manifest notably uses Tokio, WebSocket, Serde and Reqwest; `main.rs` selects a native daemon mode. The earlier picture of a "Rust CLI in front of a Node/Playwright daemon" therefore does not correctly describe the current state of `main`. This finding concerns the sources inspected, and is not an independent benchmark. [README](https://github.com/vercel-labs/agent-browser), [Cargo.toml](https://github.com/vercel-labs/agent-browser/blob/main/cli/Cargo.toml), [main.rs](https://github.com/vercel-labs/agent-browser/blob/main/cli/src/main.rs).

**Current direction:** the user removed the initial constraint of a small team/release within 3–6 months. The synthesis now recommends a canonical Rust runtime for distribution, cross-language sharing and resource control, with a reference Playwright backend. The language remains an architecture choice, not proof of a gain. The native driver must qualify action waits, frames and navigation cycles.

The threshold proposed for a **backend optimization or substitution** is a gain of at least 20% on a useful cost — full cost, end-to-end latency or memory at constant throughput and reliability — on the target scope. It no longer conditions the initial choice of the Rust runtime. This threshold is a suggested project rule, not a market truth. A distribution constraint can also justify a component, with its own comparison of maintenance costs.

## 3. CDP does not replace a complete driver

Playwright notably checks target uniqueness, visibility, stability, event reception and enabled state before certain clicks. Its locators find the target again at the moment of the action. Reproducing these guarantees is substantial work; sending a CDP command does not bring them automatically. [Actionability](https://playwright.dev/docs/actionability), [Locators](https://playwright.dev/docs/locators).

Playwright specifies that `connectOverCDP()` offers lower fidelity than its connection over the Playwright protocol. CDP attachment to Chromium remains useful, notably for cloud providers, but must be qualified with the functions actually used. [BrowserType](https://playwright.dev/docs/api/class-browsertype).

WebDriver BiDi describes bidirectional communication with commands and events; it is a portability track worth following. That does not justify implementing several protocols in the first product. [W3C specification](https://www.w3.org/TR/webdriver-bidi/), [Chrome overview](https://developer.chrome.com/blog/webdriver-bidi).

**Proposed priority optimizations:** avoid opening a browser when HTTP is enough; keep sessions healthy; run the worker near the cloud browser; reduce unnecessary observations; group operations whose dependencies are known; use precise condition waits; reduce the number of model calls. Blindly grouping clicks that depend on dynamic state would cost reliability.

## 4. Lightweight browser: Lightpanda is worth watching, not making mandatory

Lightpanda is a separate browser written in Zig, with JavaScript and without a graphics rendering engine equivalent to Chromium's. Its code is under AGPL-3.0. The README also describes an agent mode that produces PandaScript, replayable without a model on the deterministic path. These are capabilities announced and documented by the project, not reproduced here. [Repository](https://github.com/lightpanda-io/browser), [license](https://github.com/lightpanda-io/browser/blob/main/LICENSE).

Its benchmark is worth reading: it uses a Go/chromedp crawler, 933 URLs from an Amiibo demo site and several levels of parallelism. The current technical document mentions an AWS `m5.xlarge` machine, whereas the README summary speaks of `m5.large`. The multi-process Lightpanda and multi-tab Chrome comparisons do not allow deriving a universal multiplier for authenticated workflows, visual captures or complex interactions. The published figures are not Automan measurements. [Benchmark protocol and results](https://github.com/lightpanda-io/demo/blob/main/BENCHMARKS.md#crawler-benchmark).

**Proposal:** keep an adaptation point for a lightweight browser, experiment later on workloads with no need for visual fidelity, and keep Chromium as the qualified fallback. The license boundary and API compatibility must be studied for the intended distribution; a separate process is not enough, on its own, to conclude on the obligations of an integration.

## 5. Compile and replay already exists

The literature and products converge on reusing decisions and code:

| Primary source | What is studied or described | Limitation to keep in mind |
| --- | --- | --- |
| [Agent Workflow Memory, ICML 2025](https://proceedings.mlr.press/v267/wang25bx.html) | Induction and reuse of workflows for web agents | Reusing a workflow memory is not the same as removing all inference at runtime. |
| [WALT](https://arxiv.org/html/2510.01524v1) | Discovery of site functionalities and their transformation into reusable tools | Results on WebArena/VisualWebArena; not proof on a customer's portals. |
| [Agent JIT Compilation](https://arxiv.org/html/2605.21470v2) | Executable plans, pre/postcondition invariants and choice based on estimated cost | 37 tasks across five applications; some tasks are selected or constructed to compare scheduling strategies. The gains do not automatically generalize. |
| [Agentic Compilation](https://arxiv.org/html/2604.09718v1) | Deterministic JSON plan produced by compilation | Preprint; reliability figures after human correction must not be attributed to autonomous execution. |
| [Growing Harness](https://arxiv.org/abs/2609.26760) | Persistent code repairs, trace-based localization and rejection of regressions | Recent preprint; offline training, validation and cost must be counted. |
| [Anchor self-healing](https://docs.anchorbrowser.io/advanced/self-healing) | AI fallback during the run and new workflow version after a failure | Commercial product feature; accuracy and cost not reproduced here. |
| [OpenAdapt Flow](https://github.com/OpenAdaptAI/openadapt-flow) | Demonstration, compilation, local replay, effect verification and explicit stop | Project self-described as beta; its qualification is specific to the workflows and environments tested. |
| [FlowReplay](https://github.com/kuilenren/flowreplay) | Portable format, multiple locators and replay | Alpha project; its comparison tables are the maintainer's claims, not an independent evaluation. |

**Strategic inference:** compiling, caching, repairing or returning evidence do not individually constitute a defensible novelty. A possible opportunity lies in the quality of an implementation, its ease of integration and the maintenance of a precise customer scope.

## 6. Three families of benchmarks

### A. Extraction and crawling

Build locally served fixtures: static HTML, pagination, delayed JavaScript content, scroll with a virtualized list, authorized API, downloads and expired session. Provide an expected dataset, check duplicates and missing fields, and verify pagination completeness.

Compare Automan to a well-written HTTP/Playwright script, then to the relevant modes of Crawlee, Spider and Scrapy. The configurations must reach the same precision and completeness. A comparison against a forced browser mode when the competitor has an adaptive HTTP mode would be misleading.

### B. Recurring workflows

Take three read-only pilot workflows: download an export from an authenticated portal, browse a catalog with pagination and stock, track a change then produce a structured result. Measure first learning and re-executions separately.

Compare the appropriate tools: manual Playwright, Stagehand, Browser Use and Skyvern if the workflow and the available interfaces allow a fair comparison. Do not score Crawlee on an autonomous agent task it is not meant to plan.

Test: first run without cache, warm run, new parameters, language change, structure change, authentication expiry, partial response and worker interruption. Caches are tied to the account scope and the recipe version; no implicit sharing of authenticated state between customers.

### C. Generalization and perturbations

BrowserGym and AgentLab are a means of reusing WebArena, WorkArena and other benchmarks. AgentLab documents problems with versions, site state and dependencies between tasks. Concurrent runs can corrupt their validity if the tasks modify the same environment. [BrowserGym](https://github.com/ServiceNow/BrowserGym), [AgentLab](https://github.com/ServiceNow/AgentLab).

Add controlled variants: hidden button, order change, fake success message, missing value, iframe, shadow DOM, expired cookie and selector change. Also measure the frequency of unjustified stops. A library that refuses everything can display zero silent errors while remaining unusable.

DoomArena provides injection scenarios in observations and tools. These tests are relevant as soon as an untrusted page is sent to a model: page data must remain data, with limited action capabilities and effect verification. [DoomArena project](https://servicenow.github.io/DoomArena/), [paper](https://arxiv.org/html/2504.14064v3).

## 7. Metrics and publication rules

| Metric | Proposed definition |
| --- | --- |
| Verified success | The evaluator, independent of the plan, validates the goal and completeness. |
| Silent error | The engine announces a success that the validator refutes. |
| Unjustified stop | The task was executable, but the engine abstains. |
| Cost per verified success | All costs of attempts, failures, retries, compilation and repair, divided by verified successes. Separate machine cost and human labor. |
| Latency | p50/p95/p99 for each terminal state, maximum delay and failure rate; clearly identify quantiles that cover only successes. |
| Throughput | Verified successes per minute at fixed resources and concurrency. |
| Footprint | Maximum and average memory of all processes, CPU-seconds, traffic and storage. |
| Repair | Frequency, cost, delay, validated correction rate and regressions introduced. |
| Intervention | Number and minutes of human intervention per 100 runs. |

Pin versions, commits, browser, models, prompts, region, proxies, resources, concurrency limit and date. Publish the fixtures, recipes, validators and redacted traces. Run several repetitions, keep the data used to tune heuristics separate from the final set, and report uncertainty intervals. One hundred repetitions do not prove 99.9% reliability; they mostly serve to detect frequent defects.

**Suggested initial goal:** on at least three pilot workflows, obtain correctness comparable to a good manual script. Against an agent baseline declared before the trials, aim for either a halved median latency or a full cost per success reduced by at least 50%. Against the best HTTP/browser path already available, measure the possible throughput or memory gains separately. Do not promise that a replay beats an optimal deterministic script. Publish unfavorable results and the perturbations that still break the engine. This is a continuation criterion, not a commercial promise.

**Amortization fixed before the trials:** present creation, compilation, repair and maintenance separately, then full cost at 1, 10, 100 and 1,000 executions. Also declare cadence, observation duration and site change rate. Dividing by successes alone is not enough if the number of repetitions is chosen after the results.

## 8. Reliability requires recovery semantics

Illustrative calculation under an independence assumption: twenty steps that are 99% reliable give only `0.99^20 = 81.79%` complete success. With 99.9% per step, it becomes `98.02%`. Real failures can be correlated; the figures therefore do not predict a production rate.

For extraction, a retry can often be made idempotent through a key and deduplication. For an external action, losing the connection after a click sometimes leaves its effect indeterminate. A persistent queue and checkpoints do not automatically offer an "exactly once" guarantee on a third-party site.

**Contract recommendation:** states `verified`, `failed`, `partial`, `needs_review` and `effect_unknown`. After an uncertain effect, read the external state when possible before replaying; otherwise suspend according to the workflow's policy. The first pilots remain read-only to focus the work on completeness, extraction and recovery.

## 9. Illustrative economics of repetition

Entirely hypothetical example: 100 executions of an agent at €0.50 cost €50. A compilation at €3, runs at €0.03 and five repairs at €0.50 cost €8.50, i.e. 83% less. This calculation deliberately omits cost items and constitutes neither a vendor price nor an Automan forecast. The complete model must add browsers, proxies, errors, storage, support and maintenance work.

The break-even point depends on the number of repetitions, the upkeep cost and site stability. A workflow executed only once may remain better served by a general-purpose agent. A workflow with a stable API may remain better served by a direct API integration. The right strategy is to choose the cheapest sufficient means and to verify that it reaches the goal.
