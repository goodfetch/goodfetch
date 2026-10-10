# Automan — final R&D report

**Date:** October 10, 2026. **Status:** post-test report. **Final decision:** project owner.

This folder gathers everything that was used to **choose** Automan's architecture and stack: the proposals written before testing, the R&D plan, the proposed ADRs, the sourced research and the lab that measured each choice.

Nothing in this report is a hand-written figure: each result points to a run in the [ledger](lab/ledger/results-ledger.json), to a computed verdict ([`lab/verdicts/`](lab/verdicts/)) or to a dated report ([`lab/reports/`](lab/reports/)).

---

## 1. In brief

The architecture proposed on October 7 largely holds. The tests did not overturn it; they **refined** it on eleven points, four of which the proposal did not contain:

1. **A "discovered API" tier** between HTTP and the browser: JSON traffic is captured during a browser sample, then the API is replayed. Once the route is learned, a run is 120 to 200 times faster than in the browser, with no loss of accuracy.
2. **A mandatory effect journal for the agents' code mode**: a correction never replays an action the site has already received. Without this journal, a real model produced 17 false successes out of 45.
3. **Learned frontier rules**: no-effect parameters learned by page signature, trap paths closed, cursor pagination preferred. Offset pagination on a live table loses rows and reports itself complete.
4. **A session model judged on page content**: a portal that returns `200` with its login form fools any client that relies on the HTTP status.

The core choices are **confirmed by measurement**:
- Rust/Tokio core: 0.51 times Node's CPU per verified job;
- SQLite WAL `synchronous=FULL`: no loss across 6 simulated power cuts;
- reqwest as the default HTTP transport;
- compiled replay for AI: 0.17 to 0.34 times the model cost.

One point **changes in nature**: the default browser driver is a **direct CDP client with the Automan actionability guard**, and Playwright is no longer the behavioral reference. The guard is exact on all engines. Native Playwright lets clicks on moving targets through and exceeds its timeouts when remote.

What **remains open** mostly comes down to three things:
- **hardware**: a Linux reference host to publish figures, and KVM for Firecracker;
- **demand**: segment, pricing, priorities, sinks;
- a **scope excluded** from this R&D: the stealth profile.

---

## 2. Method and strength of evidence

Each experiment settles a **decision** from the register ([`lab/decisions/`](lab/decisions/)), declared **before** the measurements: options, reference, primary metric, contract gates, smallest effect of interest (10%), statistical rule and required host class. The verdict is computed mechanically (paired block design, bootstrap, Wilcoxon, TOST). It takes one of these values: `better`, `worse`, `equivalent`, `non-inferior`, `insufficient-n`, `blocked-by-contract` or `contract-met`.

**Contract gates first.** An option that produces a single false success (a result declared complete that the independent oracle refutes) is blocked, whatever its speed.

**Host classes.**

| Class | Definition | What it allows |
| --- | --- | --- |
| `local-dev` | Mac in use | Contracts and counts (logins, requests, tokens, lost rows): they do not depend on the host |
| `local-controlled` | Idle Mac: load per CPU at most 0.35, foreign CPU at most 150%, checked before the run and at each block | Deciding on durations |
| `reference` | Dedicated bare-metal Linux | Publishing figures. **Never reached in this R&D** |

**Strength of evidence in this report.**
- 🟢 **Strong**: contract verified by oracle, or eligible mechanical verdict.
- 🟡 **Medium**: clear and consistent effect, but formal verdict `insufficient-n`, or ineligible host.
- 🟠 **Weak**: indicative, a single measurement, or conditions too far from reality.
- ⚪ **Not tested**: desk research or design choice only.

All durations come from a Mac M3 Max (macOS 27). None is publishable as is.

---

## 3. Comparison table: stack before testing and recommended after

"Before" draws on the [proposal](proposals/proposition-automan.md), the [architecture](proposals/architecture.md), [ADR-001](adr/0001-runtime-language.md), [ADR-002](adr/0002-access-identity-stealth.md) and the [October 7 qualification](lab/reports/qualification-2026-10-07.md), produced by the design sessions before any decision campaign.

| # | Area | Before testing | Recommended now | Verdict | Strength |
| --- | --- | --- | --- | --- | --- |
| 1 | Core language | Rust/Tokio "candidate, not a demonstrated winner"; Node and Go credible | **Rust/Tokio** | `better` (CPU 0.51×), eligible | 🟢 |
| 2 | Browser driver | Native CDP (Chromey/Chromiumoxide to be qualified) + Playwright worker for compatibility and as reference | **Direct CDP + Automan guard** by default; Playwright only for Firefox and WebKit, with the same guard | blocked (Playwright reference timed out); clear out-of-contract estimate | 🟡 |
| 3 | Actionability contract | Playwright as behavioral reference | **Automan guard**, engine-independent | contract 390/390 | 🟢 |
| 4 | HTTP transport | reqwest as the base; Impit and wreq as options | **reqwest** by default, wreq as a variant; options set by the engine and covered by a gate | `non-inferior`, eligible | 🟢 |
| 5 | Identity and access | 3 mutually exclusive profiles per session, `identified` by default, opt-in `stealth` catalog | Unchanged; **browser-executed HTTP tier** as soon as an identity must cross the escalation; site policy in the engine, not in the transport | contract; duration not eligible | 🟢 / ⚪ stealth |
| 6 | HTTP or browser routing | "Result-based adaptive HTTP", per-route learning | Unchanged in principle, **plus a discovered-API tier**; sample-based comparison, declared total as invalidation | `better`, eligible | 🟢 |
| 7 | Frontier | Configurable canonicalization, deduplication, computed end of crawl | **Plus** learned no-effect parameters, closed trap paths, cursor preferred over offset | contract (naive blocked) | 🟢 |
| 8 | Long sessions | Session object, no blind reuse of expired sessions | **Page judged on the recipe's contract; session state in the run journal** ([ADR-003](adr/0003-session-model.md)) | `better` (logins 0.5×) | 🟢 |
| 9 | Admission and scheduling | Bounded queue, per-domain budgets; fairness "to be proven" | **Bounded per-domain queues, served round-robin**, under a global cap; admission journaled through group commits, `E_CAPACITY` refusal | 2.6× for small domains, `insufficient-n` | 🟡 |
| 10 | Durable execution | Leases, checkpoints, idempotent write before acknowledgment | Unchanged: **journal + leases + epoch**, start of each job journaled | `contract-met` | 🟢 |
| 11 | Local storage | SQLite WAL, single batched writer; JSONL for export | **SQLite WAL `synchronous=FULL`, single writer, bounded batches** | decided; power cut: 0 loss | 🟢 |
| 12 | Sinks | PostgreSQL for the cloud; other sinks not examined | PostgreSQL `COPY` for shared state; local analytical DuckDB (single writer); Parquet for batch export; **adoption on demand** | all durable; throughputs `insufficient-n` | 🟡 |
| 13 | Extraction | HTML5 DOM parser, streaming lol-html, htmd, trafilatura-class extractor: candidates | **rs-trafilatura** for main content, **htmd** for Markdown conversion, **html5ever/scraper** for structural parsing | `insufficient-n` by blocks; decided per page | 🟢 |
| 14 | Light engine | Lightpanda candidate, out of process (AGPL) | **No default engine**; tier chosen by capabilities | blocked (shadow DOM) | 🟢 |
| 15 | AI layer | Discover, compile and repair outside normal replay, budgets | Unchanged; **whole plan verified by fingerprint before the first effect**, one bounded repair | `better` (cost 0.17-0.34×) | 🟢 |
| 16 | Agent interfaces | SDK, CLI, MCP driving the engine | One operation catalog, served **identically by MCP and by function tools**; **code mode only with effect journal**; CLI for humans and scripts | MCP ≡ SDK; journaled code mode better | 🟢 / 🟡 |
| 17 | Cloud topology | Container pool or Firecracker snapshot: deferred | **Warm pool discarded or reset after each tenant**; Firecracker to be measured | pool: contract; Firecracker not tested | 🟡 / ⚪ |
| 18 | Product surfaces | React/shadcn + Vite console; SwiftUI macOS app; Apache-2.0 license | Unchanged, **not tested** | — | ⚪ |
| 19 | Observability | tracing + OpenTelemetry export, GenAI conventions | Unchanged, **not tested** | — | ⚪ |
| 20 | Market | Candidate segments from research | Unchanged, **pending interviews** | — | ⚪ |

---

## 4. Each decision: why change, why stay, alternatives

### 4.1 Core language (D-core-language) 🟢

- **Before.** Rust/Tokio proposed in ADR-001, with the explicit caveat "does not demonstrate an advantage on total time". Node and Go were credible alternatives.
- **Measured.** Same durable workflow, same SQLite build, quiet Mac. Rust uses **0.51 times Node's CPU per verified job** (8 blocks, eligible). Synchronous writers on a common SQLite library give similar throughputs across Node, Rust and Go ([workflow, October 8](lab/reports/workflow-2026-10-08.md)). Recovery is exact under four injected failure points ([runtime faults](lab/reports/runtime-faults-2026-10-08.md)).
- **Why stay.** Cost per job matters for a service that bills per verified result, and Rust keeps recovery exact. There is no reason to change.
- **Plausible alternatives.**
  - Node, if maintaining a native driver became the dominant cost. The Node and Rust CDP drivers have identical durations, so the language has no effect on the browser side.
  - Go, for distribution; not measured on the full workflow.
- **Limitation.** Mac, not the reference host.

### 4.2 Browser driver (D-browser-driver) 🟡

- **Before.** Native CDP via Chromey or Chromiumoxide, to be qualified, plus a Playwright worker for compatibility and as **behavioral reference**.
- **Measured, on a quiet Mac, with the October 8 and 10 blocks** ([report](lab/reports/quiet-host-2026-10-10.md)). Task time relative to native Playwright (lower = faster):

  | Driver ↔ browser latency | Form | Clicks | JS catalog |
  | --- | --- | --- | --- |
  | 0 ms (same machine) | 1.33 | 1.21 | 0.33 |
  | 20 ms | 0.80 | 0.84 | 0.73 |
  | 100 ms | 0.70 | 0.74 | 0.79 |
  | 300 ms | 0.68 | 0.72 | 0.80 |

  These figures apply to Rust CDP with guard. Node CDP with guard gives the same within ±0.02. Puppeteer with guard is the slowest.
- **Why change.**
  - A hosted browser is always remote from the driver. From 20 ms on, direct CDP with guard is 16 to 32% faster.
  - Native Playwright is not an exact reference: it lets a click on a still-moving target through ([cross-engine](lab/reports/cross-engine-2026-10-09.md)), and it exceeded its timeout twice at 100 ms.
  - The Automan guard, for its part, is exact everywhere.
- **Why the verdict remains blocked.** The accuracy gate also reads the reference's cells, and the reference, Playwright, failed. The durations above are an out-of-contract estimate, with very narrow intervals (width < 0.1).
- **For the owner to decide.** Replace the decision's reference with a driver that is exact at all latencies. I did not do it after the fact, so as not to bias the verdict.
- **Plausible alternatives.**
  - Native Playwright on local loopback, if it is 20 to 33% faster at 0 ms and the guard's accuracy is not required.
  - The runtime as a Chrome extension, à la Stagehand v4: placement close to the browser, not measured in the final campaigns.
- **Undecided.** The Rust CDP library (Chromey, Chromiumoxide or an in-house client): the prototype uses a direct WebSocket client.

### 4.3 Actionability contract 🟢

- **Before.** Playwright actionability served as the reference.
- **Measured.** The Automan guard, made of page scripts on the DOM (uniqueness, visibility, stability, target test, event interception), passes **390 cells out of 390** on Chromium, WebKit and Firefox, under Linux and macOS. It had already passed 162 cells out of 162 on the Chromium actionability matrix. Two guard defects were found and fixed along the way ([driver contracts](lab/reports/browser-driver-contracts-2026-10-08.md)).
- **Why change.** The contract belongs to Automan and no longer depends on the engine or the driver.
- **Limitation.** Playwright's Firefox build does not start under macOS 27; it is verified under Linux.

### 4.4 HTTP transport (D-http-transport) 🟢

- **Before.** reqwest as the base; Impit and wreq as optional specialized transports.
- **Measured, on a quiet Mac.**
  - wreq, Impit and Node are non-inferior to reqwest in throughput; pooled Crawlee is worse (0.86).
  - At equal throughput, reqwest and wreq cost about 1.5 ms of CPU per task, less than 11 MiB of memory and around ten ms of startup. Impit and Node cost 10 times more in CPU and memory ([transports](lab/reports/http-transport-2026-10-09.md)).
  - The gate found two adapter configuration defects: reqwest compiled without gzip decompression, and wreq without a redirect policy (its actual default is `none`, contrary to its documentation).
  - Spider refuses redirects to loopback, an SSRF check that was kept.
- **Why stay on reqwest.** It is the leanest, at the same throughput, and its options are under control.
- **What changes.** The engine itself sets decompression, redirects and timeouts, and a conformance gate checks them, whatever the transport.
- **Plausible alternatives.**
  - wreq, at the same cost, if a browser TLS fingerprint is needed on the pure HTTP side.
  - Impit, if Chrome emulation is needed and the CPU cost is acceptable.
- **Limitation.** At concurrency 4, concurrency caps out before the transport does: we do not know where each transport caps out on its own.

### 4.5 Identity and access (D-identity, ADR-002) 🟢 / ⚪

- **Before.** ADR-002: three mutually exclusive profiles per session (`identified` by default, `standard`, opt-in `stealth`), Web Bot Auth, site-preference policy, tiered CAPTCHA.
- **Measured, as contract.**
  - A session keeps a single profile.
  - Web Bot Auth is verified independently with Cloudflare's library.
  - robots.txt, Content-Signal, TDMRep and meta `noai` are applied according to the declared purpose.
  - **Impit and Node's HTTPS client change the TLS/HTTP2 fingerprint at the moment the session moves to the browser**; only the browser-executed HTTP tier (`browser-fetch`) keeps a consistent identity ([identity](lab/reports/identity-2026-10-08.md), [TLS fingerprint](lab/reports/tls-fingerprint-2026-10-08.md)).
  - The proxy pool handles stickiness, failover and typed exhaustion.
- **Why stay on ADR-002.** Its principles are confirmed.
- **What changes.** The HTTP tier of a session whose identity crosses the escalation runs in the browser. Site preferences belong to the engine, not the transport: the `policy-compliance` gate was removed from D-http-transport, because no transport could pass it on its own.
- **Plausible alternatives.** An impersonation transport (wreq, Impit) for purely HTTP sessions, without escalation to the browser.
- **Not tested.** The duration cost of identity consistency (ineligible host). The **stealth profile**: patched browsers, fingerprints, behavior, CAPTCHA; outside the scope of this R&D.

### 4.6 HTTP, browser or API routing (D-adaptive-routing) 🟢

- **Before.** Architecture §5.3: result-based `auto` routing, learned per route; "an HTML size or two empty datasets are not enough".
- **Measured.** The same listing is served in six ways: HTML, JS shell, partial hydration, stale HTML, GraphQL and infinite scroll ([portals](lab/reports/portal-2026-10-09.md)).
  - The "browser if the HTTP page is empty" heuristic declares stale prices and partial pages complete.
  - The compared sample is exact on 120 runs out of 120.
  - API discovery, which captures the JSON during the sample and then infers a page- or cursor-based plan validated against the browser, is exact on 120 runs out of 120.
  - On a quiet Mac, the sample is **better and eligible** (0.004 of browser time on a static page, equivalent elsewhere), and the discovered API is better on GraphQL and infinite scroll (0.008) ([quiet Mac](lab/reports/quiet-host-2026-10-10.md)).
- **Why stay.** The architecture's principle is confirmed.
- **What changes.** **API discovery is added as a routing tier**, with the same scope and the same invalidations as an HTTP decision.
- **Plausible alternatives.** Browser only: the exact reference, but the most expensive.
- **Limitation.** Simple API plans only: no request signing or single-use token.

### 4.7 Frontier (D-frontier) 🟢

- **Before.** §3.1: configurable canonicalization, deduplication within a scope, computed end of crawl.
- **Measured.**
  - With `sid` and `utm` in every link, permuted sort orders and an endless calendar, the naive frontier exhausts its budget without being able to conclude.
  - With no-effect parameters learned by page signature, trap paths closed and the cursor preferred, the canonical frontier finishes in 75 requests.
  - **Offset pagination on a table modified during the crawl loses rows and reports itself complete: 15 false successes out of 15.**
- **What changes.** The three learned rules are added, and a crawl stopped by its budget is never a complete result.
- **Plausible alternatives.** Canonicalization declared only in the recipe: it suffices as long as the site is known, not for discovery.

### 4.8 Long sessions (D-session-model, ADR-003) 🟢

- **Before.** A session has an identity, cookies, a backend and a lease; an expired session is not reused blindly.
- **Measured.**
  - A portal that returns `200` with its login form on expiry causes **20 false successes out of 20** for a client that relies on the HTTP status.
  - Judging each page on the recipe's contract eliminates them.
  - Resuming the journaled session state halves the logins; a session revoked during the shutdown costs a single login, with no loss.
- **What changes.** [ADR-003](adr/0003-session-model.md) is proposed.
- **Plausible alternatives.** Reconnecting on each run: exact, but one more login per resumption, which a portal with MFA makes costly.

### 4.9 Admission and scheduling (D-scheduling) 🟡

- **Before.** Bounded queue, per-domain budgets; fairness remained to be proven.
- **Measured.**
  - With one heavy domain under overload, bounded per-domain queues served round-robin give small domains **2.6 times** more work than a global queue (CI 2.31 to 2.93, quiet Mac), and 2.84 times on a loaded Mac.
  - Admission is journaled through group commits. Work refused with `E_CAPACITY` is never executed. The start of each job is journaled, which preserves politeness after a crash ([admission](lab/reports/admission-2026-10-08.md)).
- **Why `fair-domain`.** The effect is large and the mechanism obvious. The formal verdict requires 22 blocks; 5 were measured.
- **Plausible alternatives.** Global FIFO queue: simpler, acceptable without a dominant domain.

### 4.10 Durable execution (D-durable-runtime) 🟢

- **Before.** Leases, checkpoints, idempotent write before acknowledgment.
- **Measured.**
  - **Contract held**: lossless admission, exact recovery under injected failures (before or after commit, during acknowledgment, starved lock, write error), exact oracle.
  - Idempotent resubmission is acknowledged as a duplicate without double execution.
- **Why stay.** The contract holds.
- **Not eligible.** Acknowledgment latency (a duration) was not measured on a qualified host.

### 4.11 Local storage (D-storage) 🟢

- **Before.** SQLite WAL and a single batched writer.
- **Measured.**
  - Batches increase throughput in all configurations, and one writer does better than four.
  - **Simulated power cut** (Docker VM killed mid-write, 6 cuts): `synchronous=FULL` loses no acknowledged commit. `NORMAL` loses some (96 out of 108,803): it survives a process crash, not a power cut. Unsynced JSONL loses everything. No database is corrupted ([sinks and power cuts](lab/reports/sinks-topology-2026-10-09.md)).
- **Why stay.** It is exactly the choice that was decided.
- **What changes.** Any "fast" mode must be displayed as non-durable against a machine failure.

### 4.12 Sinks (D-sinks) 🟡

- **Measured.**
  - PostgreSQL `COPY` (local container), DuckDB and batched Parquet keep every acknowledged row, after a writer crash as after a power cut; PostgreSQL loses none of its 2,324 acknowledged commits across 3 power cuts.
  - Throughputs at 512 rows per batch: DuckDB 515,000/s, SQLite 393,000/s, JSONL 119,000/s, PostgreSQL 70,000/s, Parquet 47,000/s.
- **Why adopt nothing now.** The decision depends on customer demand.
- **What these throughputs do not say.** Durability levels are not equal on macOS: Node requests `F_FULLFSYNC`, SQLite and DuckDB a plain `fsync`, and PostgreSQL runs in a VM.
- **Working view.**
  - SQLite for local state;
  - PostgreSQL `COPY` for the cloud's shared state;
  - DuckDB for local analytics, with a single writer;
  - Parquet for batch export.

### 4.13 Extraction (D-extraction, D-markdown) 🟢 / 🟡

- **Before.** Several candidates: HTML5 DOM parser, streaming lol-html, htmd, trafilatura-class extractor.
- **Measured on WCXB.** Held-out test split, 511 pages, paired analysis **per page** ([per-page analysis](lab/reports/extraction-paired-2026-10-09.md), [re-run of October 10](lab/reports/extraction-rerun-2026-10-10.md)):
  - rs-trafilatura reaches an F1 of 0.884;
  - it beats the html5ever reference by **0.165** (CI 0.144 to 0.186), and Readability or dom-smoothie by about 0.06;
  - but 504 of the 511 ground-truth files are tagged `rs-trafilatura-draft` (the README describes drafting by Claude, then human review), and WCXB and rs-trafilatura have the same author.
- **Measured on WebMainBench.** 545 pages with manually calibrated Markdown ground truth ([re-run](lab/reports/extraction-rerun-2026-10-10.md)):
  - Readability (0.847) and dom-smoothie (0.846) slightly beat rs-trafilatura (0.820), by +0.027 and +0.025 (CIs above zero, median page nearly tied);
  - dom-smoothie is 5 to 6 times faster per page than rs-trafilatura (local-dev host).
- **lol-html.** It builds no tree and cannot score content.
  - Its streaming boilerplate removal reaches 0.716 F1 on WCXB, the level of all body text (0.719), and 0.746 on WebMainBench.
  - To read fields (title, meta, h1, links, images), it takes 0.27 of scraper's time per page (local-dev host, under load), with 99.6% or more agreement. The differences come from html5ever's tree normalization: cloned `<a>` elements and newline normalization.
- **Markdown, measured for the first time** (structure oracle on headings, lists, table cells and code blocks):
  - on WebMainBench, with the same content HTML from rs-trafilatura, mdka (0.455) and html2md (0.444) beat htmd (0.410) per page;
  - rs-trafilatura's own Markdown (0.357), fast_html2md (0.352) and dom-smoothie's Markdown mode (0.331) fall below it.
  - On 30 curated fixtures, htmd loses code in `<pre>` without `<code>` and tables with row headers. fast_html2md, the lol_html converter used by spider_transformations, produces no usable table or code block.
- **Mechanical verdicts** (64 shards per corpus):
  - on WCXB, rs-trafilatura, Readability and dom-smoothie are `better` than the html5ever reference, and htmd `worse`;
  - lol-html and every Markdown comparison remain `insufficient-n`.
- **Why change.** The proposal left the choice open. The chain retained so far is rs-trafilatura (main content), then a Markdown converter, with html5ever/scraper for selectors and fields. WebMainBench reopens the choice between rs-trafilatura and dom-smoothie, and the Markdown results question htmd.
- **Plausible alternatives.**
  - dom-smoothie or Readability for main content;
  - mdka or html2md for Markdown;
  - lol-html as a fast path for fields and links when DOM normalization is not needed.
- **Not tested.**
  - The full WebMainBench (7,809 pages, tag-level annotations).
  - The representative parsing campaign: malformed HTML, large DOM, streaming from 1 to 1,000 MiB.

### 4.14 Light engine (D-light-engine) 🟢

- **Before.** Lightpanda candidate, out of process (AGPL).
- **Measured.** Chrome and Lightpanda paired in the same run:
  - **shadow DOM fails** on Lightpanda;
  - on the rest, it uses 1/8 to 1/20 of Chrome's CPU and 1/15 to 1/65 of its memory ([paired light engine](lab/reports/light-engine-paired-2026-10-09.md)).
- **What changes.** It is not a default engine, but a tier chosen by the declared capabilities, before the session is created.

### 4.15 AI layer (D-ai-layer) 🟢

- **Before.** Discover, compile and repair outside normal replay, with budgets.
- **Measured, with three real models through a gateway.**
  - Compiled replay costs **0.17 to 0.34** times the model spend of an agent that replans on each run, over 12 blocks.
  - No false success. Verifying the **whole plan by fingerprint before the first effect** rejects a target replaced by a destructive control ([real models](lab/reports/ai-real-models-2026-10-09.md)).
- **Why stay.** The proposal's principle is confirmed, and its mechanics refined.

### 4.16 Agent interfaces (D-agent-interface) 🟢 / 🟡

- **Measured.** 675 tasks, 3 models, 5 interfaces ([journaled code mode](lab/reports/code-mode-journal-2026-10-09.md)).
  - MCP and function tools are **equivalent**.
  - The CLI depends on the model: Claude Haiku leaves 18 tasks out of 45 unfinished.
  - Raw code mode produces 17 false successes out of 45 with GPT-5.6 Luna.
  - **Code mode with an effect journal produces 0 false successes**, in 3 to 5 times fewer turns than MCP (verdict `better` on price reading, `insufficient-n` elsewhere, CI 0.16 to 0.26).
- **What changes.** A single operation catalog is exposed through MCP and the SDK, and code mode only with the effect journal.

### 4.17 Cloud topology (D-cloud-topology) 🟡 / ⚪

- **Measured.** Browsers in Docker:
  - a warm pool removes about 180 ms of startup per session, for about 39 MiB per idle container;
  - **a pool that returns its containers without resetting them leaks one tenant's cookie to the next, 30 times out of 30.**
- **What changes.** A pool must discard or reset its container before each tenant, and this contract must be a permanent gate.
- **Not tested.** Firecracker, for lack of KVM.

---

## 5. Data that do not really allow a decision

| Data | Why it does not decide | What I decided anyway | How to decide it |
| --- | --- | --- | --- |
| Browser driver, durations | Verdict blocked: the Playwright **reference** exceeded its timeouts, and the gate also reads the reference | CDP + guard by default, on the out-of-contract estimate | Change the reference to an exact driver (owner's decision), then recompute |
| Rust CDP library | Not compared: the prototype uses a direct client | None; direct client in the meantime | Qualify Chromey and Chromiumoxide against the guard |
| Scheduler, 2.6× | `insufficient-n` (5 blocks out of 22) | `fair-domain` | 17 more blocks on a quiet Mac |
| Journaled code mode, turns | `insufficient-n` on 2 variants out of 3 (a correction present in some blocks only) | Offer it | Pairing by task rather than by block |
| Discovered API, 4 renderings out of 6 | `insufficient-n` despite a two-order-of-magnitude gap | Keep it as a tier | More blocks, or a rule that accounts for the observed effect |
| Sink throughputs | Unequal durability on macOS; PostgreSQL in a VM; ineligible host | No adoption | Equalized durability (`fullfsync`), Linux reference host |
| Cost of identity consistency | Durations on an ineligible host | None; the contract suffices for the choice | Rerun on a quiet Mac |
| Runtime acknowledgment latency | Ineligible host | None; the contract holds | Rerun on a quiet Mac |
| Transports at high concurrency | Concurrency 4: cap reached before the transport | reqwest, on cost | Campaign at higher concurrency |
| Extraction by blocks | 7 page types = 7 blocks | Decided by the per-page analysis | Already done |
| All durations | Mac, not bare-metal Linux | Design decisions only | Reference host ([preparation](plan/reference-host.md)) |
| Firecracker | No KVM | Container pool | Linux with KVM |
| Requested segment, pricing, priorities, sinks | Desk research only | None | Interviews ([kit](research/market/)) |
| Stealth profile | Out of scope | None | Owner's decision and work |

---

## 6. Method lessons

These are lab defects found along the way, all fixed. To keep in mind going forward:

1. **Judge a block on foreign CPU, not on load average.** Load average includes the benchmark itself: it made 7 clean blocks out of 8 look noisy.
2. **A default configuration is not a contract.** reqwest without gzip and wreq without redirects failed the catalog; only the conformance gate caught it.
3. **A gate that reads the reference can block an exact option.** This is the case for the browser driver; a decision's reference must be exact everywhere.
4. **The sample-size rule does not account for the observed effect.** It requires 100 to 200 blocks for 100× gaps. The right pairing unit (the page, the task) must be chosen rather than repeating deterministic blocks.
5. **Durability depends on the OS.** On macOS, `fsync` does not flush the disk cache. To be equalized before any throughput comparison between sinks.
6. **Never modify sources during a campaign.** The engine marks the run `invalid-provenance`, and it happened once.
7. **Always check that a commit contains what it claims.** A fix rejected mid-command left a commit with a misleading title, and PostgreSQL unmeasured during the first series of power cuts.

---

## 7. Map of the `rnd/` folder

| Folder | Contents | Status |
| --- | --- | --- |
| [`GLOSSARY.md`](GLOSSARY.md) | Domain vocabulary drafted during the R&D | R&D draft |
| [`proposals/`](proposals/) | Pre-test proposals: [proposal](proposals/proposition-automan.md), [architecture](proposals/architecture.md), [API](proposals/api.md), [product design](proposals/product-design.md), [documentation plan](proposals/documentation-plan.md), [console mockup](proposals/design/automan-console.html) | Historical, written before the tests |
| [`plan/`](plan/) | [R&D plan](plan/rd-plan.md), [reference host](plan/reference-host.md) | Phases P0 to P9 |
| [`adr/`](adr/) | **Proposed** ADRs: [0001 language](adr/0001-runtime-language.md), [0002 access and identity](adr/0002-access-identity-stealth.md), [0003 sessions](adr/0003-session-model.md) | R&D drafts, not adopted |
| [`research/`](research/) | Sourced research ([index](research/README.md)) and [market track](research/market/) | Dated sources |
| [`lab/`](lab/) | Test harness: engine, lanes, fixtures and oracles, [decisions](lab/decisions/), [ledger](lab/ledger/), [verdicts](lab/verdicts/), [reports](lab/reports/); usage guide in [`lab/README.md`](lab/README.md) | Executable from `rnd/lab` |
