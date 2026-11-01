# Every shared per-unit path has one owner module, a cost contract of counted work and a benchmark

Most features of Collie, Foxhound and the engine do their work on one of fourteen **Shared paths**: the route that every page, request, Record, CDP command, action, navigation, protocol message, Audit entry, charge, governor sample or trace entry takes. Each Shared path has one owner module that every feature's work on it goes through. Each has a **Cost contract**: the **Counted work** that one unit of the path may cost, counted by deterministic counters and never timed. Each is measured by a named benchmark of the whole composition. The limits live in the **Quality register** of `TESTING.md`, which starts at today's figures and only ratchets down. Every known gap between today's code and its contract is an Exception that expires. Decided with the owner in the `system-qualities` Setup grilling of 2026-11-01, which the owner asked for on that date, retroactively, for the whole repository.

## Context

On 2026-10-30, Foxhound crawled about 80 pages a second against Spider's 11,145, and 337 against 12,983 like for like, at 165–181 MiB of resident memory against 25 MiB (#1577). The profile of that crawl (Hetzner CCX43, tip `0fd15a3a7`, the Proving ground's 1,000-page site at 16 requests at once) found the cause in structure, not in any one feature:

- **8 HTML parses** and about 614 CSS selectors compiled twice per page;
- **135 SQL statement preparations**, about 8 run-state commits and 7.5 fsyncs (in `full`) per page;
- **329 system calls** per page against Spider's 12, among them `provenance.jsonl` opened twice per page, `configuration.toml` and `operator-policy.toml` read 650 times and `/proc/meminfo` opened 177 times during the crawl.

ADR 0138 decides the page path's fix. A read of the code at `203f17f42`, checked again at `779a2ffa0` (strand E of the 2026-10-31 research, `rnd/research/2026-10-31-llm-planning-cross-cutting-failures/strands/e-codebase-scan.md`), found the same pattern on twelve more paths, in 17 confirmed and 5 suspected cases:

- every CDP command opens, writes and closes `trace/cdp-commands.jsonl`;
- every page read creates its isolated world again, and every guard check sends the 9,631-byte `check.js` source again;
- every Audit entry reads both TOML files, opens its store twice and runs a schema set-up transaction;
- every usage charge reads every Limit back from the Usage ledger;
- every governor sample reads both TOML files and opens `/proc/meminfo` three times;
- every links batch rebuilds the crawl's Scope from the run file and compiles its globs;
- every incoming JSON-RPC message is walked about five times and deep-copied once.

Each spec added its own per-unit work: its own parse, open, commit or settings read. Each piece was correct on its own. No decision owned any path's total cost, so no review could see the sum. ADR 0071's +5 % comparison sees one change against its base, never the slow accretion of many. ADR 0134 made the absolute ceilings informative until the optimization pass, and they measured 234 times against 2.

How proven projects own these paths. Each was read at a pinned version on 2026-11-01, in [the per-unit path research](../../rnd/research/2026-11-01-per-unit-path-architecture/):

- **One owner per path, built once, walked per unit.** Scrapy's middleware and pipeline managers precompute their method chains at start. Crawlee keeps hook arrays and a `Router`. Spider has one `Website` coordinator. Playwright sends every command through one connection object.
- **"Off" is free by construction.** Scrapy removes a disabled component from its chain (`NotConfigured`), and Spider compiles a feature out. Neither tests an `if` per unit inside shared code.
- **Run invariants are compiled once, in a bounded cache keyed by the source text.** parsel keeps a 256-entry CSS translation cache and a 2,048-entry XPath cache; Spider uses `OnceLock`. Counter-examples: Crawlee builds a `Minimatch` per page, and Playwright compiles a glob into a `RegExp` on every request (`urlMatches`, v1.64.1).
- **A browser's utility world is registered once and injected code is called by handle.** Playwright and Puppeteer register the world once per session with `Page.addScriptToEvaluateOnNewDocument(worldName)`, keep its context id from Chromium's events, evaluate their injected script once per context and call it by handle afterwards. Stagehand v3 recreates the world on every read and sends its whole script on every call, as Collie does today. Chromium's source (`InspectorPageAgent::createIsolatedWorld`, `main` at `fc2ff626`) answers a second `createIsolatedWorld` with the same world name with the same context, so the cost is a round trip, not a leak.
- **One writer per log file, holding one buffered handle.** Spider's WARC writer keeps a 256 KiB `BufWriter`, and Playwright's tracing flushes at 64 KiB or at action boundaries. No surveyed project opens a file per unit.
- **Per-unit cost guards.** Only Scrapy gates per-unit cost in CI (CodSpeed, instruction counts on every pull request touching `scrapy/**`). No project counts structural events per unit, such as parses, compilations, prepared statements, commits or syncs.

## Decision

### The Shared paths

Every Shared path names one unit, one owner module, one Cost contract and one benchmark. **Counted work** is work that repeats exactly from run to run: parses; selector, glob, regex and schema compilations; statement preparations; connections and files opened; transactions; syncs; configuration reads; tasks spawned; CDP commands; encodes and decodes. Durations and resident memory stay Performance targets (ADR 0134).

| Path | Unit | Owner module | Cost contract, per unit | Benchmark |
|---|---|---|---|---|
| **F1** Page analysis | one fetched page | the Page analysis of ADR 0138, whose crate #1577's spec places | ADR 0138's counts: at most 1 HTML parse (0 on the streaming path), 0 selector compilations, at most 1 readability run, 0 file opens; at most 2× its body's bytes allocated (ADR 0134) | B1 |
| **F2** Run-state writer | one attempt (its claim and its outcome) | `goodfetch-engine`'s `run_state` writer | ADR 0138's counts: at most 2 transactions (`full`, `fast`), at most 1 sync at `full` at any concurrency, 0 statement preparations, 0 reads through the writer, 1 configuration read per Run | B1 |
| **F3** Frontier admission | one links batch; one enqueued URL | `goodfetch-frontier`'s admission, given the Run invariants built when the Run starts | per batch: 0 reads of the Scope, Payload rules or Fragments from the run file, 0 compilations, 0 validator builds, 0 statement preparations, 0 transactions of its own (it runs in F2's claim); per URL with a payload: 1 validation, 1 serialization (the one stored) | B1 |
| **F4** Record | one Record | `goodfetch-dataset`'s `Datasets`, built when the crawl starts | 0 declaration reads, 0 validator builds, at most 1 validation, 1 serialization, 0 deep copies of its data, 0 lookups of a value constant for the Run, 0 transactions of its own (it commits with F2's outcome, ADR 0103) | B1 |
| **C1** CDP command | one command sent to a browser | `goodfetch-cdp`'s connection (`send_raw`, which `send` calls), the one seam every command crosses; the CDP command log is a stage it is given once | 1 encode, 1 send, 0 directories created, 0 files opened; with the CDP command log off, nothing sent to its writer | B2 |
| **C2** Page read | one engine read in a frame | `goodfetch-page`'s driver, holding one registry of context ids per frame and document | 1 CDP command (`Runtime.callFunctionOn`), 0 `Page.createIsolatedWorld`; per session, the world registered once with `Page.addScriptToEvaluateOnNewDocument` and created once for each frame that already exists | B2 |
| **C3** Action | one attempt of a click, fill, press or other guarded action | `goodfetch-page`'s guard, calling `check.js` by its handle | per check: 1 `Runtime.callFunctionOn` on the kept handle, 0 sends of `check.js`'s source (sent once per execution context); CDP commands per attempt, per action kind: a ratchet on the measured figure, with no cap yet | B2 |
| **C4** Navigation | one navigation judged, or one document no call waited for | `goodfetch-page`'s `recovery`, which composes the Block markers, the Page check and the consent detector | at most 1 detection read (every detector's function in one call), 0 glob or regex compilations (the Page check compiles when its Session opens), 1 Provenance entry through E5 | B2 |
| **C5** Intercepted request | one request a page sends | `goodfetch-page`'s interception `pipeline` | with no rule: 0 `Fetch` messages; per paused request: 1 match against the compiled Filter lists, 1 CDP answer, 0 compilations, 0 files opened, 0 tasks spawned per refusal, 1 entry sent to E5 | B2 |
| **E1** JSON-RPC message | one message in, one answer or event out | `goodfetch-engine`'s `rpc`: its framing and `composition`'s dispatch | per request: 1 decode straight into typed params, 0 deep copies; per answer and per event: 1 encode straight into bytes, with no intermediate `Value`; allocations and bytes per message under ADR 0134's tuned ceilings, on small and large params | B3 |
| **E2** Audit entry | one entry | `goodfetch-engine`'s `audit` | 0 configuration reads, 0 store opens, 0 schema set-ups, 0 statement preparations, 0 prune passes, 1 transaction; the head file's rewrite and syncs per entry: a ratchet | B3 |
| **E3** Usage charge | one charge; one Slice taken | `goodfetch-engine`'s `ledger::charging` | per charge the held Slices cover: 0 SQL statements, 0 opens (the Limits are held in memory and read again only when the ledger itself changes them); per Slice taken: 1 transaction, 1 sync, 0 opens, 0 statement preparations | B1, B3 |
| **E4** Governor sample | one sample; one page opened in the browser pool | `goodfetch-engine`'s `governor` sampler | per sample: 0 configuration reads, at most 1 `/proc/meminfo` open, 0 opens of `self/cgroup` or `self/mountinfo` (read when the engine starts), each process's memory read once; per page opened: 0 memory walks (the pool reads the sampler's last measure) | B3 |
| **E5** Run-trace entry | one Provenance, request-log or CDP command-log entry | `goodfetch-engine`'s `run_trace`, as one writer per Run that holds one buffered handle per file | 0 directories created, 0 files opened, 0 tasks spawned; 0 formatting when its kind of entry is off | B1, B2 |

### What every path keeps

- **The plug-in rule.** A feature on a Shared path adds its work through the path's owner module: either the owner itself, or a stage the owner builds once and calls. The feature declares what it reads and what it adds. It builds per unit nothing that depends only on the engine's, the Run's, the Session's or the document's inputs. A second loop, handler or queue for the same unit beside the owner is a second entry point, and is refused.
- **What is built once, and at which scope:**
  - **The engine:**
    - one snapshot of the machine's settings (the Operator policy and user configuration), resolved when the engine starts, refreshed when the engine writes its own configuration (`config set`) and resolved again when a Run is admitted. A hand edit of either file while the engine runs is seen at the next Run or the next write by the engine.
    - each SQLite store (the Audit log, the Usage ledger, known certificates, the Run index), opened and set up once with its connections kept and every statement prepared through `prepare_cached`. The writer is an actor of `tasks` and the readers run on its blocking threads, as `run_state` does (ADR 0060).
    - the bounded cache of compiled selectors of ADR 0138, which also holds compiled globs and regular expressions, keyed by their text.
  - **The Run:** the Run invariants (the Scope, Payload rules and Fragments with their compiled patterns and validators), the Datasets and their validators (ADR 0138), and the Run-trace writer and its file handles.
  - **The Session:** its Page check matchers, and the utility world's registration.
  - **The document:** the utility world's context id and the handle of `check.js` in it, replaced only when Chromium reports the context created, destroyed or cleared.
- **A feature switched off removes its work, not only its effect** (ADR 0138's rule, for every path). For example, with no route and no Filter list, interception sends no `Fetch` message. With the CDP command log off, nothing is sent to its writer.
- **Each benchmark runs the whole composition with its default features on, plus one run with each feature on the path that can be switched off turned off.** Each benchmark checks that the work it counts happened, as Scrapy's benchmarks assert that their selectors still match. Its own tests also plant one extra unit of each counted work and expect it to fail.

### How work is counted

- **Each owner module counts its own counted work** at the moment it does it. The counts are exposed as metrics of the ops listener (ADR 0050), as ADR 0138 decides for the page path. A count shared between threads lives in its crate's `sync` module and is checked under loom (ADR 0060).
- **A benchmark reads the counters on the paused runtime** and divides each count by the units of the outcome that did the work (per click, per fill, per read, per navigation with and without a Page check, per page served, blocked or redirected, per charge within a Slice, per Slice taken), never by the sample's mix of outcomes.
- **Instructions, allocations and bytes stay gungraun's** (ADR 0071), measured beside the counts.
- **System calls are not counted as such** (ADR 0138): futex and epoll waits depend on scheduling. Their causes (files opened, syncs, configuration reads) are counted.

### The benchmarks

- **B1, the whole crawl:** `goodfetch-foxhound`'s in-memory crawl of #1572 (`crawl::in_memory_pages`), with a counts cell beside its existing heap cell. Its fixture is extended so that the crawl meets every outcome its paths have: a redirect, a blocked page, a payload under a Label's schema, a Record under a Dataset's schema, a Page check and a Limit.
- **B2, a Collie Session:** a new `goodfetch-page` scenario over the in-memory CDP end that `check_and_arm` already uses, with a scripted browser that answers each command at once. It opens a page, navigates with and without a Page check, clicks, fills, presses, reads, and sends requests, some of them blocked, with a navigation in the middle so that the world and `check.js` are set up again exactly once. Goodfetch's counts do not depend on Chrome, so a real browser's time stays a Performance target.
- **B3, the engine:** `goodfetch-engine`'s `dispatch`, extended from `engine.info` to large params (a `crawl.request_handled` Record, a `route.fulfill` body) and events, beside new scenarios for Audit entries, charges and governor samples, the last over a stand-in for `/proc` whose seam the gate's spec places.

### Interface changes this decides

- **The CDP command log (spec #577) becomes a setting of the Run trace, off by default,** which the tests and the conformance cases that read `trace/cdp-commands.jsonl` turn on. Its name is set by the spec that builds it. Off, it does no work, as Playwright's protocol log (`DEBUG=pw:protocol`) is off by default.
- **The Audit log prunes when the engine starts and then once a day,** on a task of `tasks`, not before every entry. An entry past `audit.retention` may then stay up to a day longer.
- **The Run-trace writer flushes its buffer** (a `write`, with no sync) when a group of outcomes commits (F2), when its buffer passes 64 KiB, and when the Run ends. A process crash then loses at most the entries since the last group commit, so the Run trace stays in step with the run state.

### The Quality register

- **`TESTING.md`'s "Quality register" holds every limit of these contracts**, in `system-qualities`' register format: the Floor, the Enforced and Measured tables, and the Exceptions. The project has no `CONSTRAINTS.md`.
- **Every Cost contract's count has a row.** It is Measured until the gate's spec builds its command, and Enforced from that pull request on, failing the merge gate. Promotion is a tightening and needs no approval. It does not wait for ADR 0134's `GATING` switch, which keeps governing ADR 0071's +5 % comparison until #1577's pull request.
- **Retroactive adoption.** Every row starts at its figure measured on its benchmark, which is taken once the gate's spec builds it after [#1583](https://github.com/goodfetch/goodfetch/pull/1583) merges. Every gap between today's code and a contract is an Exception with the owner and an expiry date (2027-01-30, 90 days). Each Exception names the spec that removes it: #1577 for ADR 0138's counts, or the Shared paths spec for the others. No row is set at a value the current code fails on its first gate.
- **Runs at.** The rows of counted work run at implementer done, at most 60 seconds per benchmark family (`TESTING.md`'s guidance for the Quick suite). The rows read under Valgrind (instructions, allocations, bytes) run at the merge gate through `xtask perf`.
- **Coverage is a Measured row,** "must not fall" within 0.5 %. ADR 0072's weekly coverage job compares it and opens an issue when it falls, as `xtask budgets` does, and no merge fails on it.
- **The register is the source of the numbers.** The register check compares `xtask perf`'s ceilings with the register and fails on a difference. It also fails when a path in this ADR has no row, when a count in a Cost contract has no row, or when a row allows more than its contract.

### Open questions

These are recorded here, and each blocks only the tickets that would extend its path:

1. **F1's crate.** The Page analysis's crate is set by #1577's spec.
2. **C3's cap on CDP commands per action kind.** Set from the first measure of B2. Playwright's `locator.click()` sends about 15 messages, and Collie's click sends at least 5.
3. **C4's one detection read.** Whether the consent detector can run in the same call as the Block markers and the Page check: it clicks and checks again. If it cannot, the cap becomes 2, with the owner's approval.
4. **The SDKs' framing per message** (TypeScript's `collie-client/src/framing.ts`, and Python's generated layer). It is not a Shared path yet. The protocol count snapshots cover its bytes, and its CPU cost per message has not been measured.

## Considered Options

- **A review rule only, with no counted contracts:** rejected. Strand E's 17 cases each passed review, because no reviewer could see the sum.
- **Ceilings on time or throughput per path:** rejected. A shared runner varies by about 2.7 % (ADR 0071), and pages per second stays a Performance target. The counts gate the causes, which repeat exactly.
- **Counting system calls with strace, or function calls under Callgrind:** rejected. Futex and epoll waits depend on scheduling (ADR 0138), and a function's name or inlining changes without any change of cost.
- **One owner for the whole engine (a cost registry with a budget per feature):** rejected. A path with two owners has none, and a registry beside the code would drift from it. The owner module that every unit crosses already sees every feature's work.
- **Each feature caching its own result:** rejected, as ADR 0138 rejects it for the page. The next feature's module would add its own work again.
- **The driver's `Browser` as C1's owner:** rejected. Most commands pass its `try_send_within` and `send_unleased`, which record the CDP command log, but the launch's `Browser.close` (`launch.rs`) goes straight to the connection. `goodfetch-cdp`'s connection is the one seam every command crosses, so its counts see them all. The CDP command log becomes a stage the connection is given once and calls, not work beside it.
- **Keeping one page read per detector on a navigation, under a ratchet:** rejected for now. It would leave three round trips per navigation, where one composed read gives each detector the same view. Open question 3 keeps the fallback.
- **Re-reading the settings files on each change, through a stat of their modification time or a file watcher:** rejected. A stat is counted work per unit. A watcher needs its own thread and code per platform (ADR 0060). No ADR promises that a hand edit is seen before the next Run.
- **A write per Run-trace entry with a kept handle:** rejected. It still costs a system call per entry, which no surveyed project pays.
- **Coverage gated at the merge gate:** rejected. ADR 0072 decides that coverage means every Feature is tested, not a percentage, and an instrumented build of the whole workspace on every gate would take hours.
- **Separate ADRs for Foxhound's, Collie's and the engine's paths:** rejected. The register check reads one inventory, and the built-once rules (the settings snapshot, the stores, the compile cache, the Run-trace writer) serve paths in all three.

## Consequences

- **Three specs carry this decision, in order:**
  1. **The gate's spec:** the benchmarks B1 to B3, the counters, the commands of the Enforced rows, the floor guard and the register check. It measures every row's first figure, and the path tickets that `/to-tickets` cuts are blocked on its benchmark ticket.
  2. **[#1577](https://github.com/goodfetch/goodfetch/issues/1577),** ADR 0138's optimization pass, which removes F1's, F2's and the Datasets' Exceptions.
  3. **The Shared paths spec,** which removes the Exceptions of C1 to C5, E1 to E5, F3 and F4. It starts with one audit ticket per family of paths: the owner's entry point is read with the LSP's call hierarchy, so that cases strand E's grep could not see are found.
- **Open tickets are swept once with the path check** after #1583 merges, since tickets already implemented on `milestone/m1-core` close then. A ticket that adds per-unit work to a Shared path gains its register rows and benchmark among its acceptance criteria, and is blocked by the gate's benchmark ticket. A ticket whose design goes around an owner module is rewritten, or marked `ready-for-human`.
- **Every later step applies the path check** of `system-qualities` (`/to-spec`, `/to-tickets`, `/implement-spec`, `code-review`), through the pointers in `docs/agents/domain.md` and `AGENTS.md`.
- **Amends ADR 0134:** path counts gate once their commands exist, whatever `GATING` says, and the register is the source of the ceilings' numbers.
- **Notes on:**
  - **ADR 0040:** the governor reads the settings snapshot, and the browser pool reads the sampler's last measure.
  - **ADR 0041:** Limits are held in memory between the ledger's own changes.
  - **ADR 0050:** the counts of counted work are metrics.
  - **ADR 0072:** the weekly coverage job compares project coverage with the register's figure and opens an issue when it falls.
  - **ADR 0138:** its counts are F1's and F2's contract, unchanged.
  - **Spec #577:** the CDP command log is a setting, off by default.
- **Contracts change in place.** The CDP command log's setting is added to `goodfetch-protocol`'s types and regenerated with `xtask codegen`. Nothing is published, so no format changes version (ADR 0066).
- **`GLOSSARY.md` gains four terms:** Shared path, Cost contract, Counted work and Quality register.
