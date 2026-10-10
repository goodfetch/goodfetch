# Automan lab — test and measurement engineering

**Research of October 7, 2026.** Primary sources consulted: tool documentation, the authors' own work, Google Testing/SRE, ACM and NIST. The "now" prescriptions are our adaptation to the [lab](../lab/README.md), not a universal standard nor proof of performance already achieved.

## 1. The right target: a correct result under an explicit contract

Separate three questions: does the harness measure correctly; does each backend fulfill the contract; with what resources and durations does it fulfill it? A call that returns without error is not enough. WebArena verifies the functional correctness of tasks and the resulting state rather than the identity of a sequence of actions. [The authors' paper and environment](https://webarena.dev/og/).

**Now:** each scenario carries an input, a postcondition, a deadline and an independent oracle. Count `verified`, `failed`, `timeout`, `unsupported`, `partial` and unknown effects explicitly; do not mix the adapter's declaration with the validator's verdict. A fast but incorrect scenario stays among the failures and its cost among the expenses.

The runner obtains the truth from the fixture server: complete set of IDs and values, version, event and effect journal, checksum of a file. The adapter returns only the observations it actually obtained. The expected data are not passed along with the command. For a local fixture, this code independence is not a security isolation against an agent with access to the whole filesystem: future AI evaluations will also need to limit the exposed tools.

## 2. Choosing tests by risk, cost and fidelity

The pyramid is a heuristic; Google now also proposes SMURF: speed, maintainability, utilization of resources, reliability and fidelity. The historical 70/20/10 ratio is not an obligation applicable to all projects. [Google, SMURF](https://testing.googleblog.com/2024/10/smurf-beyond-test-pyramid.html), [original pyramid](https://testing.googleblog.com/2015/04/just-say-no-to-more-end-to-end-tests.html).

| Layer | What we test now | What it lets us diagnose |
| --- | --- | --- |
| Small tests | Protocol validation, IDs, statistics, parsing, attempt state, oracle errors | Lab bug without launching Chromium |
| Local integration | Real fixture server + adapter; fragmented JSONL; timeout and shutdown | Contract between components and lifecycle |
| Browser conformance | Click/typing/wait, delayed or replaced DOM, ambiguous target, effect journal | Actual guarantee of the primitives |
| Workflow | Small complete crawl, form and rendered extraction | Useful end-to-end pipeline |
| Performance/stress | Load, saturation, cold/warm, repetitions | Cost and capacity after functional correctness |

No test that only checks that a constant string is present. Prefer an assertion on an observable property; check the relevant edge cases. High line coverage proves execution, not that the assertions detect an error. [Stryker, explanation of mutation](https://stryker-mutator.io/blog/introduction-to-mutation-testing/).

## 3. Hermetic fixtures and isolation

Google describes hermetic servers with configurable connections, test data and logs, enabling reproducible local tests. It specifies their startup costs and size limits; hermetic does not mean free. [Hermetic Servers](https://testing.googleblog.com/2012/10/hermetic-servers.html).

**Now:** server on loopback and OS-assigned port; known corpus and seed; private state per `runId`; base URL passed explicitly; browser profile specific to the run; Crawlee queue/storage specific to the campaign. Tests independent of their order, with no dependency on a public page, a personal cookie or an AI key.

Provide explicit reset, shutdown in `finally`, readiness by event/endpoint, and a usable journal. The reset must not erase previous results: a new campaign folder and a new identity are enough. The count, IDs, delays, mutations and graphs are controlled by the corpus.

**Next:** distinguish external validation on public sites from a local campaign. Cloud services require accessible fixtures and a documented region/network. Their variability belongs to a separate series.

## 4. Testing the lab with positive and negative controls

Mutation testing checks that tests fail when a defect is introduced; equivalent mutants prevent treating a 100% score as an absolute guarantee. [Stryker, principle](https://stryker-mutator.io/docs/), [equivalent mutants](https://stryker-mutator.io/docs/mutation-testing-elements/equivalent-mutants/).

**Now:** no need to add a mutation engine to every adapter. A suite of bad adapters/test doubles covers the most dangerous errors:

- A correct control passes; zero results, an ID that is missing, duplicated or out of domain, a stale value or a fake success message fail.
- An adapter announces `verified` without performing the click: the server has no effect, the verdict is a failure.
- Invalid JSON, polluted stdout, an oversized line, a response to the wrong ID and a late response lead neither to a false success nor to an unbounded wait.
- Crash after sending, no response, refusal to shut down and concurrent shutdown give an explicit terminal state.
- `unsupported` reduces coverage; it adds no zero latency to the distribution of successes.

For the statistics, use small numerical datasets whose mean and quantiles can be computed independently; include empty, one element, equal values and invalid parameters. For a result validator, check both acceptance of a legitimate output and rejection of a wrong output.

## 5. Properties, models and transformations

fast-check generates command sequences, shrinks a counterexample and allows replaying it by seed/path; its authors warn that a model copied from the implementation can reproduce the same bug. [Model-based testing](https://fast-check.dev/docs/advanced/model-based-testing/).

**Now:** simple invariants with saved seeds, without creating a second architecture: unique IDs, exact expected set, attempt count preserved, a single terminal response per request, reset without inter-run leakage. Replaying a recorded counterexample becomes a regression test.

Metamorphic tests check necessary relations between executions when the complete output is hard to know; these relations do not constitute a proof of complete correctness. [The authors' work on metamorphic relations](https://arxiv.org/html/2406.05397).

**Proposed adaptations:** changing the order of links or adding cycles does not change the set of entities of a crawl whose contract ignores order; splitting a JSONL message into arbitrary fragments does not change the message received; inserting a decorative element without modifying the accessible role/name does not change the target of a semantic locator. These relations must follow the contract: changing a label can legitimately break an exact locator.

**Next:** a leases/retries/commit model for durability; a controlled scheduler for races; targeted fuzzing of the protocol. Reserve the model for states that are actually implemented.

## 6. Browser interaction: waiting for the right event

Playwright documents locators, actionability checks, auto-waiting assertions and test isolation. It advises against fixed delays as synchronization, and points out the significant cost of traces recorded everywhere. [Best practices](https://playwright.dev/docs/best-practices), [timed wait](https://playwright.dev/docs/api/class-page#page-wait-for-timeout).

**Now:** total deadline, condition-based waits, event waiter armed before the triggering action, then validation of the effect. The delays injected into the fixture are controlled perturbations; they must not be copied as sleeps into the adapter.

Minimum conformance: visible/unique target, temporary overlay, disabled becoming enabled, re-render that invalidates a reference, typing with `input`/`change` events, click and server effect. A direct modification of `.value` is not automatically equivalent to user typing; the scenario states which events are required.

Separate a raw command on an already-ready page from a reliable action on a changing page. If a CDP adapter adds checks or a resolver to reach the common contract, measure these costs too. Keep detailed diagnostics/traces in a mode separate from the benchmarks; screenshots on failure or diagnostic replay.

## 7. Bounded supervision and memory

Node documents that `kill()` sends a signal and does not guarantee that all descendants stop; on POSIX, a `detached` child becomes the leader of a group/session. [Child process](https://nodejs.org/api/child_process.html). Streams must respect `write() === false` and wait for `drain`, or use `pipeline()` to propagate errors and shutdown. [Backpressure](https://nodejs.org/learn/modules/backpressuring-in-streams).

**Now:** direct spawn without a shell, one persistent adapter process, command IDs, stdout reserved for the protocol, stderr drained, size/volume limits, startup/run/close deadlines. Idempotent shutdown; stop the resources created by the run, then TERM and bounded escalation if necessary; monitor the browser descendants and the outcome of the shutdown. Do not use a global `pkill` that would hit the user's browser.

Limit in-flight tasks, queue length and buffers. The fixture server and the runner also have limits. A load with imposed arrivals must not grow an array of promises without bound: reject and count admission when capacity is reached.

**Next:** groups/cgroups on Linux, test subprocesses that ignore TERM, a browser interrupted in the middle of an action, a slowed sink and a large response. RSS readings are observations with a method and not proof of total physical memory; descendants and shared pages count.

## 8. Open load, closed load and saturation

The closed model maintains a concurrency level and slows its arrivals along with the system; the open model imposes arrivals independent of durations. k6 explains coordinated omission and publishes the iterations not started when the generator's resources are insufficient. [Models](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/open-vs-closed/), [dropped iterations](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/dropped-iterations/).

**Now:** explicit closed mode for the first stress tests, concurrency and total-duration limits, all statuses counted. Do not call this mode a capacity measurement at a fixed arrival rate.

**From the first open mode onward:** record for each task the planned arrival, admission, start, end, validation; publish the number planned/started/finished/rejected, queue delay and generator lag. Keep valid throughput and failures alongside the quantiles. Calibrate server/generator separately: their saturation invalidates a conclusion about the crawler.

**Next:** ramps, bursts, slow host, 429/Retry-After, backlog/catch-up, fairness and soak. Find the sustainable throughput with stable quality and backlog, rather than only the instantaneous peak. The number of browsers remains an explicit budget, not a loop that launches without bound.

## 9. Honest repetitions and statistics

Google Benchmark distinguishes warmup, repetitions and random interleaving to limit the effect of a changing system state. Kalibera/Jones analyze several levels of variation and recommend repeating at the level that produces the uncertainty. [Official guide](https://google.github.io/benchmark/user_guide.html), [variance reduction](https://google.github.io/benchmark/reducing_variance.html), [the authors' paper](https://kar.kent.ac.uk/33611/).

**Now:** monotonic clocks; cold/warm separated; warmups declared and excluded from the summary; order randomized by blocks with a seed; same scenarios and seeds per block; raw results preserved. Compare components and then complete frameworks in two separate tracks. The spawn/init cost does not disappear: publish startup and execution.

The block/process is a relevant unit of repetition; 50 clicks along the same path are not 50 independent workflows. A first smoke test proves executability; a few repetitions give an exploration, not a robust advantage. The final publication will require a campaign sized according to its variance and the useful effect sought.

Publish empirical p50/p95/p99 with `n`, population and quantile method; never average the p95s of subgroups. Do not substitute a zero duration for failures nor arbitrarily assign them the deadline as observed latency. The latency of successes and the count of all states are two necessary views. Averages hide long latencies. [Google SRE](https://sre.google/workbook/monitoring/).

**Next:** paired comparison by scenario/seed/block, effect interval computed at the block level and quality/resource curves. Freeze corpus, tuning and promotion threshold before the final campaign; publish unfavorable results and optimized configurations separately from defaults.

With zero failures, the exact one-sided 95% upper bound is `1 - 0.05^(1/n)` under a binomial assumption: **2.9513%** failure at `n=100`, **0.09997%** at `n=2,995`. Calculation verified with Python; it does not hold for correlated trials within a single session or for generalizing to all sites. NIST describes binomial intervals, including cases with small sample sizes/few failures. [Exact bounds](https://www.itl.nist.gov/div898/software/dataplot/refman2/auxillar/exacbici.htm).

## 10. Incidents and durability: verifying histories

Jepsen reasons about the histories of operations allowed by a consistency model; its controlled experiments show the value of targeted faults and an independent validator. [Consistency models](https://jepsen.io/consistency), [example of a search for lost acknowledged writes](https://jepsen.io/analyses/mongodb-3-4-0-rc3).

**Now:** test crash/hang/recovery of the adapter protocol; record attempt, response/absence, terminal state and server effect. Do not promise workflow durability because a worker restarts or a queue exists.

**Next, after implementing real persistence:** crash before/after result, commit and ack, expired lease with the old worker still alive; assertions on preserved acknowledged results, losses, duplicates, unknown effects and fencing. The "single effect" contract is tested only on the fixture that provides the required idempotence; a third-party site does not necessarily offer it.

## 11. AI: separate tasks, trials and oracles

Anthropic distinguishes task, trial, grader, transcript and harness; recommends a clean environment, a reference that passes and deterministic graders when possible. The environment's resources can change agents' scores. [Evaluation method](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), [infrastructure noise](https://www.anthropic.com/engineering/infrastructure-noise).

**Now:** deterministic primitives without a provider. The protocol provides for cost and status; a later fake provider tests budgets and outages, not the agent's competence.

**Next:** final data/seeds/mutations held out from tuning, private oracle inaccessible to the agent's tools, caches and sessions per trial; cold discovery, replay, repair and refused-repair series. Count all attempts, calls/tokens, budgets and model versions; verify fresh values and real effects. A repair does not change the oracle. LLM judgment reserved for aspects that cannot be verified by code, with a rubric and independent calibration.

Anthropic has documented agents that identify a benchmark and then find its answers; simply hiding a file behind a format or obfuscation does not protect an oracle. [Research on contamination and eval awareness](https://www.anthropic.com/engineering/eval-awareness-browsecomp).

## 12. Provenance and initial deliverable batch

ACM distinguishes available artifacts, evaluated artifacts and validated results; a useful artifact is documented, consistent, complete and executable. Publishing scripts and data does not by itself prove an independent reproduction. [ACM policy](https://www.acm.org/publications/policies/artifact-review-and-badging-current).

**Now:** manifest with commit/dirty diff or source hash, locks/checksums, runtime/OS/CPU/arch, browser/revision/flags, adapter/config, scenario/oracle/corpus, seed/order, deadlines/concurrency, cache and metrics/method. Single folder, raw JSONL, JSON/CSV summary and replay command. No secrets, private prompts or personal cookies in the manifest. The report describes the skips/unsupported and the exact scope of the measurements.

The smallest complete batch is: fixture server; Crawlee/Spider/Stagehand adapters and controls; doctor/bootstrap; smoke; independent conformance; bounded runner; exploratory cold/warm and concurrency campaign; reproducible report; tests of the harness's failures. Then add the browser surfaces, open load, RTT, durability, soak and AI through successively executable contracts. No need for a cluster, advanced dashboard, distributed broker or full matrix before this first proof.
