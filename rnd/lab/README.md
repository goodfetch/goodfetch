# Goodfetch Lab

Operational local lab: crawl engines, browser and storage primitives, with controlled data and independent validation. Built October 7–8, 2026. The first results qualify configurations on these fixtures; they demonstrate neither universal superiority nor Goodfetch's final architecture.

The [new results for the durable workflow and common SQLite](reports/workflow-2026-10-08.md) extend the lab with Node/Rust sync/Go, recovery and Crawlee pool wake-up.

The [executed cross-cutting qualification](reports/integrated-2026-10-08.md) adds the
[native Node and Rust workflow](docs/runtime-workflow.md), crashes during/after
ACK, native or guarded actionability, SQLite readers/checkpoints and
HTTP transports. These campaigns are not yet a final choice of core.

The [executed results and decisions to test](reports/qualification-2026-10-07.md) include HTTP, browser primitives and concurrent writes. The [HTML index](reports/index.html) gives access to the manifests, raw data and retained reports.

## Decisions, claims and ledger

The lab is driven by the [R&D plan](../plan/rd-plan.md). Each campaign serves a decision in `decisions/`: options, baseline, primary metric from the [catalog](metrics/catalog.json), contract gates, minimum effect Δ, rule and host class. Candidate public claims are in `claims/`, declared before measurement. Editorial notes carry no measured figure: figures come from the verdicts.

```sh
pnpm lab validate   # schemas and references of the register, claims and ledger
pnpm lab import     # re-expresses results/ as run.v2 under ledger/imported and regenerates the ledger
pnpm lab ledger     # role, status, rerun and validity classes of each run
```

### v2 engine and host classes

```sh
pnpm lab preflight local-controlled          # checks the host: load, memory, thermals, AC power, foreign processes, committed sources
pnpm lab run campaigns/v2/bench/http-smoke.json
```

`pnpm lab run` executes a v2 campaign (`schemas/campaign-bench.v2.schema.json`); an old `campaigns/*.json` is converted and capped at `local-dev`. The engine chains provenance (git SHA, dirty state, source fingerprint), host check, conformance gate, warmup, measured blocks and finalization into `results/<id>/run.v2.json` and `attempts.v2.jsonl`. Thresholds for the `local-controlled` and `reference` classes: `hosts/*.json`. A host that does not pass the gates is refused, or explicitly downgraded if the campaign allows it; uncommitted sources cap the run at `local-dev`.

All campaigns are versioned data under `campaigns/v2/`: `bench/` for the attempts engine, and `storage/`, `workflow/`, `runtime/`, `actionability/`, `sqlite-readers/` for the lanes (`schemas/campaign-lane.v2.schema.json`). Each campaign names its decisions and requests a host class. A lane keeps its own cell executor and oracle; the generic engine (`src/core/lanes.ts`) adds provenance, host check, randomized blocks, typed statuses, `run.v2.json` and `cells.raw.jsonl` with the full detail of each cell. The old CLIs (`pnpm storage`, `pnpm workflow`, `pnpm runtime`, `pnpm actionability`, `pnpm sqlite-readers`) call the same executors and remain available for comparison.

```sh
pnpm lab validate                                          # register + all v2 campaigns
pnpm lab suite campaigns/v2/suites/qualification-current.json   # chains the campaigns and compares against the expected counts
```

A suite (`schemas/suite.v2.schema.json`) declares the expected counts for each step; it fails if a count differs or if provenance changed during the run. A new v2 run enters the ledger as `unreviewed` until it is classified.

### Verdicts and report

```sh
pnpm lab verdict            # verdicts/<decision>.json for each decision that has an `analysis` block
pnpm lab report             # reports/decisions.md, generated from decisions/ and verdicts/ only
```

A decision declares in `analysis` the lane, the factors that designate each option, the pairing factors and the main stratum. For each paired block, the verdict computes ln(option / baseline) of the primary metric. It then produces an interval by seeded block bootstrap: (1 − α) for superiority, (1 − 2α) for equivalence and non-inferiority. Hodges–Lehmann and exact signed Wilcoxon are given as robustness checks.

The outcome is mechanical, compared with ±ln(1 + Δ): `better`, `worse`, `equivalent`, `trivial`, `non-inferior` or `inconclusive`. It is `blocked-by-contract` if a gate fails or cannot yet be evaluated, and `insufficient-n` if there are fewer blocks than the observed dispersion requires (minimum five). A verdict is eligible for the decision only if the observed host reaches the required class.

Method: an attempt's status comes from a typed error code (`src/core/errors.ts`), never from the text of a message. A timeout affects only the stuck attempt: its in-flight neighbors are `aborted` (`E_SIBLING_ABORT`), excluded from the failure rate, and the worker restarts within the `maxRestarts` limit. Fixtures and oracle run in a separate process (`fixtures/worker.ts`); `taskMs` covers only the `run` call, and `oracleMs` is measured separately. A metric with no probe on the host is `null`, with its reason in `metricAvailability`, never 0. Attempts blocked by a gate remain visible as `skipped-gate`.

### Network, Linux and local CI

```sh
pnpm lab ci     # typecheck, unit and contract tests, timing-precision tests (run alone), register and campaign validation
```

A bench campaign can declare `network: { link: "browser-origin", rttMs }`. The engine first calibrates the profile: one-byte ping-pongs, direct and through the proxy, are alternated, and the run records the paired median of the added delay. Out of tolerance (the larger of 2 ms and 10%), time and resource metrics become unavailable; contract metrics remain valid. Fixtures are then served through a user-space TCP proxy (`src/net/latency-proxy.ts`), without privileges, on macOS as on Linux. It adds half the RTT in each direction, preserves byte order and never releases a packet before its due time. The `driver-browser` link arrives with the P6 CDP adapter; loss and jitter are handled by `tc netem` on the reference host.

On Linux, the process tree's CPU comes from `/proc/<pid>/stat` and proportional memory (anonymous PSS + swap) from `smaps_rollup`. cgroup v2 counters require a systemd scope per run and remain declared unavailable until then. Host preparation: [`rnd/plan/reference-host.md`](../plan/reference-host.md).

### Browser drivers (D-browser-driver)

```sh
pnpm lab run campaigns/v2/bench/browser-driver-rtt-0.json          # then -20, -100, -300: driver-browser link
pnpm lab run campaigns/v2/actionability/browser-actionability-extended.json
```

Two direct CDP drivers send the same command sequence for each operation: `node-cdp` (`adapters/cdp-node`) and `rust-cdp` (`adapters/cdp-rust`, compiled by `pnpm bootstrap`). They inject the same page scripts, read from `adapters/browser-scripts/`: waits, extraction, and the Collie guard (uniqueness, visibility, enablement, stability, hit-test, shadow DOM included). A comparison between the two therefore isolates the language used for the protocol. Each task reports its number of CDP commands, and an integration test requires it to be identical between the two languages.

The `firefox-bidi` backend (`adapters/firefox-bidi`, issue 2460) drives the installed stock Firefox over WebDriver BiDi. It uses Collie's own Guard scripts (`crates/goodfetch-page/src/guard/check.js` and `disarm.js`), run in a BiDi sandbox, with no call into the page's main world. `FIREFOX_BIN=<firefox> node src/firefox-bidi-campaign.ts` runs the three cross-engine campaigns on it. `node adapters/firefox-bidi/probes.ts <firefox>` tries the six CDP operations BiDi has no command for. `node adapters/firefox-bidi/chromium-control.ts <chrome> [blocks]` runs the same Guard over raw CDP as a control. The results are in [`reports/firefox-bidi-guard-2026-11-18.md`](reports/firefox-bidi-guard-2026-11-18.md).

The `browser-driver-primitives` matrix (`actionability` lane, `config.scenario: "primitives"`) checks ten gestures with a fresh adapter per cell: hover, double click, right click, drag-and-drop via pointer events, mouse wheel, Enter key, `contenteditable`, Unicode input, target moved under the pointer (a decoy takes its place) and target replaced by a destructive control (`retarget`, which must be refused without effect). The oracle requires a single effect of the right type, coming from an `isTrusted` event, and no effect on a decoy. Native HTML5 drag-and-drop and IME composition are outside this matrix.

The `browser-navigation-frames-files` matrix (`config.scenario: "navigation"`) covers seven portal situations: SPA route via `pushState`, going back in history (real restoration, `persisted` or `back_forward`), popup opened by `window.open`, same-origin iframe, cross-origin iframe (separate process in Chromium), file upload, and download with exact bytes. Popups and frames are handed to the task as pages: guard, gesture and postcondition apply to them in the same way. In direct CDP, a same-process frame goes through an isolated world, an out-of-process frame through auto-attach, and clicks remain in main-page coordinates.

The `http-incidents` matrix (`config.scenario: "http-catalog"`, decision D-http-transport) triggers an incident on the second page of a three-page catalog: 429 with `Retry-After`, single 503, truncated body, dropped connection, redirect loop, 8 s response. A retry must respect `Retry-After`; the four corrupted or endless cases must end in an explicit error, which the lane passes on to the oracle. The oracle distinguishes incomplete data returned as a success (false success) from complete data delivered after the deadline.

The `access-policy` matrix (decisions D-identity and D-http-transport) checks two contracts from ADR-002: no request to a page disallowed by robots.txt when the campaign asks to respect it (`config.respectRobots`), and no success on a challenge page served with 200.

The `tls-fingerprint` matrix serves the catalog over HTTPS (local self-signed certificate, `config.insecureTls`) behind a relay that computes the JA4 of each ClientHello; the HTTP/2 server records settings, pseudo-header order and header order. The contract: the same TLS and HTTP/2 identity throughout the session.

The `extraction` lane (D-extraction) measures main-content extraction quality on the WCXB corpus, downloaded outside git according to `corpora/wcxb.manifest.json`: word-level F1, required snippets, boilerplate present, with the exact definitions of its `evaluate.py`; each cell writes its predictions for cross-checking. Options of the lane:
- `corpus: "corpora/webmainbench"` reads WebMainBench's 545-page subset, prepared by `tools/prepare-webmainbench.ts` according to `corpora/webmainbench.manifest.json`;
- `shards` spreads the pages over that many cells, stratified by type, so that a verdict pairs enough blocks;
- `markdown: true` also scores the output against the ground-truth Markdown with the structure oracle (`src/extraction-markdown.ts`; curated cases in `fixtures/markdown-structure/`);
- `task: "fields"` saves structural fields, compared between parsers by `tools/extraction-fields.ts`.

`tools/extraction-paired.ts` gives the per-page paired analysis.

`browserHost: "lightpanda"` (bench) and `config.browserEngine: "lightpanda"` (matrices) launch Lightpanda 1.0.0, pinned under `.toolchains/` and always out of process (AGPL-3.0), and connect the same drivers to it: the engine's compatibility is read with Chromium's oracles.

The `extraction` lane can also render each page in Chrome or Lightpanda before extracting. Frozen real pages are then served by an archive proxy (`fixtures/replay-proxy.ts`), which refuses any request outside the archive.

The `admission` lane (D-durable-runtime, D-scheduling) sends open Poisson arrivals across several domains to the Rust runtime's `admission` mode. A job is acknowledged only once journaled, and a full queue refuses it explicitly. The oracle verifies from the outside the absence of loss, the queue bound and the per-domain limit on simultaneous requests.

The `identity` lane (D-identity) moves a session from HTTP to the browser on a site served behind the TLS relay. The site records the fingerprint, headers, cookie and Web Bot Auth signature of each request; it verifies the signature with Cloudflare's `web-bot-auth` library, against the agent's key directory, and judges refusals according to profile and purpose.

The `ai` lane (D-ai-layer) pits an agent that plans on each execution against a compiled plan replayed without the model, on pages that drift silently; the fake model (`fixtures/fake-llm.ts`) counts its own tokens, which the lab compares with the agent's.

With `browserHost: "harness"`, the harness launches one Chromium per driver and per block, with the same flags (`src/core/browser-host.ts`). It samples this browser along with the adapter, so the browser's CPU and memory count for all drivers. Playwright and Puppeteer connect to it via CDP. A `network: { link: "driver-browser" }` profile places the latency proxy between the driver and the browser. `actionPolicies` sets each driver's policy: `guarded` applies the guard before each click, input and selection; `native` relies on the driver's own guarantees.

The bench produces one cell per driver, scenario and block, which makes it possible to pair the blocks. The `policy`, `rttMs` and `browserHost` factors appear in it. In a decision, `analysis.gateLanes` indicates the lane where each gate reads its cells: actionability conformance and absence of false success come from the matrix, not from the bench. A gate passes only on cells of the option itself. Each analyzed option is compared with the baseline, with its own gates.

`ledger/classification.json` holds the human judgment on each run (role, reason, validity A contract / B relative same host / C absolute, decisions, rerun). The import adds the mechanical facts to it: counts, provenance, integrity recomputed against the SHA catalogs in `reports/`. It is idempotent and never modifies `results/`. Imported runs remain capped at the `local-dev` host class.

## Getting started

Prerequisites: Node >=24, pnpm **12.9.1**, Rust/Cargo and C/C++ build tools. The executed Node version, **25.5.0**, is specified in `.node-version`; other versions require requalification, notably for SQLite. The executed environment is recorded in each manifest. The lab uses Node's native TypeScript stripping, without a development server.

```sh
cd /Users/jb/Dev/goodfetch/rnd/lab
pnpm install --frozen-lockfile
pnpm bootstrap
pnpm check
pnpm test
pnpm integration
pnpm run doctor
pnpm bench campaigns/smoke.json
```

`bootstrap` installs Chrome for Testing into `.browsers/`, then compiles the three Spider variants and the Rust writer in release mode, with their lockfiles. Initial downloads/builds can take several minutes. The lab's profiles and outputs are separate from personal Chrome. No cloud account or model is needed for these campaigns.

`bootstrap` also builds the common SQLite bindings on macOS arm64: Node N-API, synchronous Rust FFI and Go/cgo. Go 1.27.1 is installed locally under `.toolchains/`, without modifying the global Go. Archives are pinned by SHA-256; the SQLite 3.53.4 source is also verified against the published SHA3-256. This native build is currently qualified only on macOS arm64.

The full `smoke` can exit with code 1 when a competitor does not respect a fixture's contract: the report is produced anyway. `unsupported` represents a missing capability; an incomplete result remains `failed`. `doctor` checks installation and protocol, not business correctness.

## Installations and configurations

| Backend | Version | Work and lifecycle |
| --- | --- | --- |
| `node-http` | Node from the manifest | Native fetch + specialized parser for the fixtures; minimal control, not a general scraper |
| `crawlee-http` | Crawlee 3.18.2 | Fresh CheerioCrawler per task, in-memory queue, internal concurrency 1 |
| `crawlee-http-pooled` | Crawlee 3.18.2 | One `keepAlive` crawler and one queue per block; pool shared between tasks |
| `spider-http` | Spider 2.53.9 | `basic,sync`, without default features; Website per task, internal concurrency 1 |
| `spider-chrome` | Spider 2.53.9 | Separate binary with `chrome`; browser managed by Spider |
| `spider-smart` | Spider 2.53.9 | Separate binary with `smart`, which enables Chrome on the dependency side; native rendering decisions |
| `playwright` | 1.63.0 | Browser per block, fresh page/context per task |
| `puppeteer` | puppeteer-core 25.12.0 | Browser per block, page per task; shadow not qualified in this adapter |
| `stagehand` | 4.1.0 | Local browser + extension, Page/Locator primitives; cache and self-heal disabled |
| `stagehand-batch` | 4.1.0 | `experimentalBatch` for the click sequence only; other scenarios identical to `stagehand`; concurrency 1 |
| `crawlee-playwright` | Crawlee 3.18.2 / PW 1.63.0 | Crawler and browser per task; Playwright's native guarantees |
| `crawlee-stagehand` | @crawlee/stagehand 3.18.2 / Stagehand 3.7.3 | Requested integration, launched locally; Playwright Page attached via CDP; model boundary forbidden |
| Node storage | `node:sqlite` of the executed Node | One synchronous connection per writer process |
| Rust storage | SQLx 0.8.6 | Tokio + pool limited to one connection per writer process |
| `node-shared` | Node 25.5.0 / common SQLite 3.53.4 | Synchronous N-API binding, same shared C library |
| `rust-sync` | Rust / common SQLite 3.53.4 | Synchronous FFI, same shared C library |
| `go-sync` | Go 1.27.1 / common SQLite 3.53.4 | Synchronous cgo, same shared C library |

**`@crawlee/stagehand` currently accepts only Stagehand v3.** The `adapters/stagehand-v3` workspace avoids injecting v4 into it. Dependencies are pinned in pnpm/Cargo.lock. A Zod peer warning remains for v3's transitive Ollama provider; this provider is outside the qualification. The v3 adapter explicitly replaces the model client with a client that fails on any call. V4 uses the primitives without AI instructions or model configuration. The AI, cloud cache and repair variants are not measured here.

The install scripts of `esbuild` and `better-sqlite3` are explicitly allowed. Those of `@google/genai`, `bufferutil` and `protobufjs` are disabled; their optional functions are not qualified. No installation with `--force`.

The macOS 27/Xcode 27 build required `release.build-override.strip=false, debug=true` to make the build-time dylibs loadable. The executable is still compiled in release mode. The Spider `basic` feature also pulls in SQLite dependencies through its TLS feature; it does not imply crawl persistence in these trials.

## Crawl and browser campaigns

```sh
pnpm bench campaigns/http-stress.json
pnpm bench campaigns/http-lifecycle.json
pnpm bench campaigns/arrival-stress.json
pnpm bench campaigns/browser-primitives.json
pnpm bench campaigns/qualification.json
pnpm bench campaigns/smoke.json --backends playwright,stagehand,crawlee-stagehand
```

Each call produces `results/<name>-<date>-<id>/`: `manifest.json`, `attempts.jsonl`, `summary.json`, `summary.csv`, `report.html`. Folders are unique; old experiments are kept. To regenerate a report:

```sh
pnpm report results/<folder>
```

The HTTP catalog is paginated. The JavaScript catalog loads its data via fetch. The oracle checks all IDs, prices, duplicates and omissions. The forms check name, select, checkbox, input event and server-side effect. The clicks check the sequence and the events declared trusted. The current mutations are temporary overlay, delayed button and DOM replacement. Open Shadow DOM is tested; iframe/download have fixtures, but their adapters are not qualified and are not advertised as supported.

An explicit condition determines page availability and the postcondition. The wrappers use `domcontentloaded`, then this condition. Condition polling at 8 ms is not an arbitrary wait for success. Playwright's additional native checks remain active; Stagehand does not silently receive all of those guarantees. The test proves the result on these fixtures, not a universal actionability equivalence.

For each backend and block: separate startup, conformance for each mutation, warmup excluded from statistics, then measurements. Any non-conformance stops the cell's measurements. Blocks randomize the backend order by seed. Not all variants have a randomized order: this point must be strengthened before a statistical publication.

`closed` maintains a bounded number of active tasks. `open` produces independent arrivals and rejects when capacity is occupied. There is no hidden FIFO queue. `queueMs` refers to the admission/generator delay, not time in a business queue. The deadline bounds admission; already active tasks remain bounded by their timeout. A timeout terminates the worker and its process group, and can therefore make the worker's other tasks fail.

Throughput is that of **complete verified tasks**, not of pages or RPCs. The report distinguishes success quantiles, quantiles of all started attempts, errors, rejections, unsupported, and sample sizes. It aggregates the raw samples, never the blocks' p95s. Per-block details and measurement windows remain available.

## Write campaigns

```sh
pnpm storage campaigns/storage-smoke.json
pnpm storage campaigns/storage-contention.json
pnpm storage campaigns/storage-process-crash.json
pnpm shared-build
pnpm storage campaigns/storage-shared-contention.json
pnpm storage campaigns/storage-shared-checkpoint.json
```

| Contract | Acknowledgment |
| --- | --- |
| `jsonl-buffered` | Append completed on the OS side; no disk sync requested |
| `jsonl-durable` | Append then file sync for each batch |
| `json-snapshot-durable` | Entire growing JSON document, synced temporary file, rename, directory sync; single writer |
| `sqlite-wal-full` | One transaction per batch, WAL, synchronous FULL |
| `sqlite-wal-normal` | One transaction per batch, WAL, synchronous NORMAL; series distinct from FULL |
| `sqlite-delete-full` | One transaction per batch, DELETE journal, synchronous FULL |

`producers` sets the generator's active tasks. `writers` creates real independent processes, each with a connection, on **the same sink of the cell**. Several SQLite writers remain subject to a single active writer in SQLite. `busy_timeout` and `wal_autocheckpoint=1000` are explicit; the effective PRAGMAs, `sqlite_version` and compile options are recorded. The three new bindings load **a single common SQLite file**, whose actually loaded path, SHA-256, source ID, flags/options and PRAGMAs are checked. The compilation sources are tied to the manifest and a stale binary is refused. Their bindings, allocations and encodings remain different: this comparison is more controlled, without isolating the language alone. The historical native Node / Rust SQLx tracks keep their separate builds.

Different SQLite versions make it impossible to attribute a gap to the language alone.

The workload builds the same records and encodes JSON even for the relational track. `serializationMs` measures construction + encoding per batch; `writerAckMs` also includes the sink; the main time includes the worker exchanges. This is not a pure microbenchmark of JSON.stringify/serde_json. The snapshot rewrites the entire document, whereas JSONL appends and SQLite inserts: compare these distinct usages according to need, without a blind common ranking.

After drain and close, the oracle reopens the files, compares all expected records with the acknowledged batches, looks for losses/duplicates/incorrect values and runs `PRAGMA integrity_check`. Only rows read back correctly count toward useful throughput. Close/checkpoint and the oracle have a separate time, `finalizationMs`. Bytes include the SQLite sidecars present at the time of the check.

The crash campaign sends SIGKILL **after the acknowledgments** and reopens the sink. It verifies retention after a process crash, not a stop in the middle of a transaction, idempotent recovery or a power cut. The macOS/fsync/F_FULLFSYNC limits are detailed in the storage research.

## Durable integrated workflow

```sh
pnpm workflow campaigns/workflow-throughput.json
pnpm workflow campaigns/workflow-recovery.json
pnpm workflow campaigns/workflow-pool-wakeup.json
```

The [historical prototype contract](docs/workflow.md) documents acquisition/extraction → validation → bounded queue → batched writer → commit → ack, leases and idempotent receipts, with its embedded Node SQLite sink. The [new native workflow](docs/runtime-workflow.md) implements this cycle separately in Node and Rust/Tokio, with the same SQLite artifact loaded by both writers. Go remains qualified in the write track.

```sh
pnpm workflow-build
node tools/qualification-suite.ts
node tools/qualification-current.ts
```

## Reproducibility and limitations

- The fixtures and the private oracle live in the coordinator; the worker receives only its task's public URL. This avoids accidental false successes; it is not isolation against a hostile adapter capable of forging events or inspecting the host.
- Recent manifests pin configs, seeds, actual runtime, OS, CPU, memory, browser and SHA-256, locks, sources and Rust executables. A source modification during a recent campaign results in exit code 1 and a warning. The first construction campaigns predate this safeguard.
- The historical tracks sample RSS at 250 ms. The new runtime and SQLite reader tracks also use a macOS CPU/RSS/footprint/I/O probe at 100 ms, calibrated against getrusage; they can miss peaks and process ends. Shared pages may be added together. The historical workflow includes the coordinator/oracle; the native workflow measures its coordinator and its descendants, with fixture and oracle outside the scope. Hardware CPU/RAM limits are not enforced.
- The lab bounds concurrency, calls, JSONL output, delay and corpus: at most one million fixture records retained by the coordinator, and 64 MiB of estimated corpus per storage cell. The largest tests require a streaming oracle. It is not a network sandbox and does not provide hardware isolation. The current tests target loopback; no campaign on public sites is needed.
- Profiles, OS cache and reused connections differ by backend. Lifecycles are separate rows. `crawlee-http-pooled` serves precisely to avoid drawing conclusions from the sole cost of a fresh crawler per task.
- p95/p99 with a few dozen samples remain exploratory. The zero-failure bound assumes independent/representative Bernoulli trials; this corpus does not establish them. No production SLO is announced.
- Do not run several campaigns or compilations in parallel when interpreting performance. The construction trials sometimes coexisted with a compilation; they serve primarily for qualification and must be repeated on a stable host.

See the [coverage matrix](docs/coverage-matrix.md), the [adapter protocol](docs/protocol.md), the [general best practices](../research/2026-10-07-lab-testing-engineering.md) and those for [storage](../research/2026-10-07-lab-storage-engineering.md). The [sourced shortlist](../research/2026-10-07-lab-competitor-shortlist.md) distinguishes the competitors to add from the installed backends.

## Adding an experiment

First define the observable result and the oracle; add a backend-independent fixture, then an adapter capability. Make the lab reject an incorrect adapter before collecting its timings. Give each configuration an explicit name, a lifecycle, budgets, versions and a durability contract. Add a campaign JSON and run `pnpm check`, `pnpm test`, then a targeted real qualification.

The automated tests cover oracle, persistence, false successes, corrupted/out-of-order protocol, timeout and descendants, overload/admission, metrics and config validation. The real campaigns cover the installed libraries. Performance must not become unstable assertions on a laptop; portable CI validates the contracts, and performance comparisons use a reserved machine.

The TypeScript runner is a measurement tool that drives several processes/languages. Its choice does not set the language of the future Goodfetch engine. Rust/Tokio remains the candidate for the portable core; Node is the measurement reference and a compatibility path. The native Rust/Node HTTP workflow comparison is verified on 30 cells; the native Rust Chromium driver and an integrated Go core remain to be qualified.

[Commands for reproducing and regenerating the evidence](reports/qualification-reproduce.md). The fixed guards pass 162/162 cells and the SQLite readers/checkpoints 108/108; the failures of the native configurations remain visible in the report.
