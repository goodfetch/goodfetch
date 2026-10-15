# R&D plan: a decision-driven lab

**Status: approved by the project owner on October 8, 2026.** This plan redirects the [existing lab](../lab/README.md). It does not rebuild it. It produces three outputs: a stack settled by measured decisions, provable performance claims and a market positioning. The parameters below are a program. No result follows from them yet.

## 1. Principle

Each experiment settles a **decision** from the `lab/decisions/` registry. A decision declares in advance:

- its options and its baseline;
- its primary metric;
- its contract gates, which must be at 100% before any performance verdict;
- the smallest effect of interest Δ, the decision rule and the required host class.

The verdict is computed, never written by hand. Candidate public claims (`lab/claims/`) are declared before the measurements.

The host classes are `local-dev`, `local-controlled` and `reference` (Linux bare metal). Everything can run locally. The reference host is deferred, and its reruns come last. A metric that cannot be measured on a host is declared `unavailable`, never approximated.

## 2. Harness overhaul (prerequisite for any new campaign)

| Phase | Content | Acceptance |
| --- | --- | --- |
| P0 | `goodfetch/goodfetch` repository, skills configured, ADRs in `docs/adr/` | Done on October 8, 2026 |
| P1 | Decision and claim registry, metric catalog, schemas (`lab/schemas/`), import of the 43 existing runs as `run.v2`, ledger with role, validity class A/B/C and rerun need | Done on October 8, 2026 ([#12](https://github.com/goodfetch/goodfetch/pull/12)): idempotent import, originals unchanged, integrity checked against the SHA catalogs, `pnpm lab validate` green |
| P2 | Single engine: provenance, host preflight and postflight, compliance gate, warmup, blocks, finalization. Typed error codes, failure isolation (`aborted` ≠ failure), oracle outside the measured time, metric availability | Done on October 8, 2026 ([#13](https://github.com/goodfetch/goodfetch/pull/13)): count parity with v1; a stuck adapter no longer fails the block; a noisy host is refused |
| P3 | The five other lanes ported to the engine, all matrices as data | Done on October 8, 2026 ([#15](https://github.com/goodfetch/goodfetch/pull/15)): historical counts reproduced by a real suite (30, 42, 162, 108, 60, 24) |
| P4 | Block-paired statistics (log-ratio, bootstrap, Hodges–Lehmann, Wilcoxon, TOST), computed sample size, mechanical verdicts, reports generated from the data only | Done on October 8, 2026: golden tests, mechanical verdict on real data (`D-core-language`: insufficient blocks, not eligible on local-dev), report generated with no hand-written figure |
| P5 | Network (user-space latency proxy, netem for the reference), paired RTT calibration, Linux probes, `pnpm lab ci` | Done on October 8, 2026: calibration within tolerance, `pnpm lab ci` green and stable across repeated runs (79 unit and contract tests, 31 integration tests), `/proc` and `smaps_rollup` probes wired in. The proxy replaces toxiproxy (no binary to pin, no privilege); dnctl is dropped for the same reason. Remaining for P6: the `driver-browser` link, and for P9: the per-run cgroup scope ([host preparation](reference-host.md)) |
| P6 | Decision campaigns from §3, in slices, on a `local-controlled` host | In progress. Slices 1 to 3 delivered on October 8, 2026: Rust and Node CDP drivers at command parity, harness browser, calibrated `driver-browser` link, actionability, primitive and navigation matrices (cross-origin iframes, popups, files), Collie v2 guard (interception at the moment of the gesture, lost gestures). Slice 5a (HTTP incidents): Spider HTTP is the only backend that produces false successes; no retry configured in the reference adapters. Slice 5c partial (robots.txt, challenge served as 200): no backend detects a challenge; node-http ignores robots.txt. Slice 5b partial: reqwest, wreq and Impit adapters added to both matrices (explicit errors, no retry, robots.txt and challenge not handled); TLS fingerprint (JA4) and HTTP/2 fixture: Crawlee changes fingerprint mid-session, Impit is close but lags behind this Chrome. Slice 5d (identity profile, October 8): single-profile session, Web Bot Auth signed and verified by Cloudflare's library, robots.txt, Content-Signal, TDMRep and noai preferences per purpose, isolation; only an HTTP tier executed by the session's browser keeps a consistent identity, Impit and Node are blocked by the consistency gate ([report](../lab/reports/identity-2026-10-08.md)); sticky upstream proxies with failover delivered; the stealth profile remains to be done. The lab's JA4 computation mishandled Chrome's GREASE: fixed, fingerprint campaigns rerun, conclusions unchanged except for Chrome, which keeps a single JA4. Slice 6 partial: write error and lock contention injected in both runtimes, exact recovery; open admission and multi-domain scheduler added to the Rust runtime on October 8: no loss, explicit refusals and per-domain limits held under overload; a bounded per-domain queue gives small domains 2.8 times the verified throughput of a global queue (D-scheduling, `local-dev`); recovery after a crash between commit and acknowledgement, with idempotent resend and slots held until the deadline of in-flight requests ([report](../lab/reports/admission-2026-10-08.md)). First measurements on an idle Mac: D-core-language settled for Rust; verdict grid of D-browser-driver per scenario and RTT ([report](../lab/reports/decisions.md)). RTT 300 ms to redo (4 noisy blocks out of 6) |
| P7 | Extraction quality and lightweight engines | In progress. `extraction` lane on WCXB (CC-BY-4.0) delivered on October 8, 2026: rs-trafilatura 0.884 F1 on the held-out test set, ahead of Readability and dom_smoothie; Lightpanda: compatible for reading and simple forms, false successes under guard on overlay and ambiguous target, without iframes, popups or downloads; frozen real pages (T1): the 511 WCXB test pages, rendered by Chrome and Lightpanda behind an archive proxy with no network access, give extraction identical to the raw HTML, and Lightpanda renders all of them ([report](../lab/reports/frozen-pages-2026-10-08.md)); an archive with subresources remains to be built for pages constructed in JavaScript |
| P8 | AI layer and interfaces | In progress. `ai` lane with a fake model delivered on October 8, 2026: compiled replay at one call per session, 3 to 6 times fewer tokens, zero false successes on silent drift after fixing plan validation; three real models (Anthropic, OpenAI, Google, through the Vercel AI gateway) on October 9: zero false successes, compiled replay 3 to 6 times cheaper, cost cross-checked against a dated price list ([report](../lab/reports/ai-real-models-2026-10-09.md)); interface arena on October 9: MCP and function tools equivalent, CLI model-dependent, code mode the cheapest but blocked by false successes when a fix replays the effects ([report](../lab/reports/interface-arena-2026-10-09.md)) |
| P9 | Linux reference host and publishable reruns | Waiting for the machine. Rehearsal on October 9, 2026 in a Linux container (Docker, arm64): bootstrap, CI and full contract suite at the macOS qualification counts, after porting the shared SQLite as `.so`, the resource sampler and the Chromium sandbox ([report](../lab/reports/linux-container-2026-10-09.md)); the container remains `local-dev` |

## 3. Decision campaigns

The 16 campaigns proposed on October 8 are taken up here, each tied to a decision. The No. refers to that original list. The order follows value for the decision, then dependencies.

| Order | No. | Campaign | Decision | Phase / host | Adaptation to the refactored lab |
| --- | --- | --- | --- | --- | --- |
| 1 | 4 | Rust/CDP driver against a direct Node/CDP reference, Playwright, Stagehand, Stagehand batch, `@crawlee/stagehand`. Navigation, resolution, click, typing, extraction, alone then in sequences; CDP commands, latency, browser CPU and memory included | D-browser-driver | P6, local-controlled | The Node/CDP reference isolates the language from the protocol. The Collie guard is applied to both drivers, and the number of CDP commands becomes a catalog metric |
| 2 | 5 | Complete primitives (hover, double click, right click, drag-and-drop, scroll, keyboard, `contenteditable`, Unicode/IME), target replaced or moved between the last check and the click, cancellation in sequence | D-browser-driver | P6 | These are the decision's **contract gates**: no speed verdict without them. Missing capabilities declared `unsupported` |
| 3 | 7 | RTT 0/20/100/300 ms, local then remote SDK, sequences of 1/10/50 actions, batched or not; order, partial effects, errors | D-browser-driver, D-agent-interface | P5 + P6 | These RTT values replace the 0/40/240 profiles of the initial plan. `driver-browser` link |
| 4 | 2 | Capacity of the complete HTTP workflow: native Node, native Rust, integrated Spider and Crawlee, C1/4/16/64. Three profiles: small response **without delay**, pagination with delay, large responses | D-core-language, D-http-transport | P6, local-controlled | The no-delay profile is essential: the current server delay can mask the computation gap. Capacity is measured under open load, and the C1–C64 sweep is labeled "closed load" |
| 5 | 8 | HTTP transports and incidents: TLS, HTTP/2, connection reuse, proxies, compression, controlled public redirects, loops; 429/503, Retry-After, truncated response, disconnection, timeout, slow server | D-http-transport, D-identity | P6 | The [ADR-002](../adr/0002-access-identity-stealth.md) tests are added: HTTP → browser fingerprint consistency, Web Bot Auth, robots/Content-Signal/TDMRep, challenge served as `200`. The SSRF controls are kept: Spider's loopback refusal is never bypassed |
| 6 | 3 | Representative parsing: CSS, malformed HTML, Unicode, missing fields, duplicates, large DOM, schema validation; acquisition, parsing, validation, then complete pipeline; streaming 1/100/1,000 MiB with a memory-bounded oracle | D-extraction, D-core-language | P6 (T0) then P7 (T1) | Complemented by frozen real pages (Web Page Replay) and extraction quality (WCXB F1 per page type) |
| 7 | 6 | Navigation, frames and files: SPA/hydration, back/forward, popup, new target, interrupted navigation, long polling, service worker; same-origin and cross-origin iframes, shadow DOM and slots, upload/download, screenshots, PDF, DOM/AX snapshots | D-browser-driver (gates) | P6 | Extension of T0 to the B2B portals segment. The same fixtures serve the documentation examples |
| 8 | 9 | Continuous admission of the complete pipeline: increasing load, spikes, slowed writer; latency from admission; bounded queue, backpressure, explicit rejection, draining, no loss among accepted jobs | D-durable-runtime | P6, local-controlled | New decision separate from D-storage |
| 9 | 10 | Scheduler, leases, multi-domain: fast and slow domains, priorities, global and per-domain limits; heartbeat, renewal, expiry during a long acquisition, late stale worker | D-durable-runtime | P6 | Gates: fairness, no starvation, commit with a stale token rejected |
| 10 | 11 | Extended fault injection: random stops (acquisition, parsing, coordinator, writer, browser), full disk, write errors, prolonged lock, corrupted data; hard stop of a VM | D-durable-runtime, D-storage | Local; VM reset on local-controlled (local VM) or on reference | Each contract declares what must survive. The VM hard reset is the only durability test beyond a process crash |
| 11 | 14 | Durable browser workflow and plan replay: navigation/actions → extraction → validation → commit → ACK; discovery and compilation, replay without LLM, text or structure change, repair; external effect with lost response | D-ai-layer, D-durable-runtime | P8 | Addition: **silent drift** suite (wrong target still clickable, same layout with a changed price). The `effect_unknown` state is checked, and a replay is never assumed safe |
| 12 | 15 | Agentic capabilities: Stagehand AI, Browser Use, agent-browser, Skyvern, Crawl4AI… Same held-out corpus, model, tools and budgets; discovery, extraction, multi-step, replay, repair; tokens, calls, cost, duration, cold or warm cache; malicious injections in pages | D-ai-layer, D-agent-interface | P8 | Fake provider first (token oracle), then 2 to 3 real models with a cost cap. pass@1 and pass^k. Addition of the interface arena: MCP, CLI, SDK, code mode |
| 13 | 12 | JSON/JSONL/SQLite broken down: encoding, transport, writing, synchronization; batches of 1 to 10,000; records of 128 B/4 KiB/64 KiB; out-of-cache corpus; index, upsert, constraints, long-running readers; sharded and compressed JSONL | D-storage | P6, low priority | D-storage remains `decided` for its base (single writer, batches, WAL FULL). This campaign refines the settings only if usage requires it |
| 14 | 13 | Other sinks: PostgreSQL/COPY, Parquet/Arrow, DuckDB, object storage; batch, flush, recovery, interrupted multipart, idempotence | D-sinks | After the market track | Opened only for the sinks requested by customers (D-feature-priority) |
| 15 | 16 | Sessions, isolation and endurance: cookies/login/storage, reused contexts, several tenants with canary data; no leak; 8 h of mixed load with recycling and incidents | D-identity, D-cloud-topology, D-durable-runtime | Isolation in P6; endurance on local-controlled (overnight) then on reference | Endurance monitors memory, descriptors, orphan processes, WAL, queues and throughput |
| 16 | 1 | Confirmation on a dedicated machine: Node/Rust, Spider and browser comparisons on Linux; same versions, Chromium and CPU/RAM limits; randomized order paired by block | All technical decisions | P9, reference | Condition for publishing any quantified claim |

Decisions added by this integration: **D-durable-runtime** (admission, scheduling, leases, recovery) and **D-sinks** (output formats and destinations).

## 4. Existing results

The 43 runs already executed are imported without modification and classified in `lab/ledger/`:

- **A, contract**: remains valid as long as the versions do not change.
- **B, relative comparison on the same host**: serves as orientation and as a pilot for sample size.
- **C, absolute figure**: not publishable without a reference rerun.

Superseded or diagnostic runs keep their reason. To rerun first: Node/Rust runtime (`789d5e`), integrated HTTP backends (`2b2b75`, noisy host) and browser timings (`b21a28`), absorbed by campaigns 1, 3, 4 and 16 above.

## 5. "Market demand" track (parallel)

- **Multi-agent workflow**:
  - collection from GitHub, Reddit, HN, reviews and 2026 reports;
  - deduplication and filtering of promotional content;
  - coding against a taxonomy, then independent second coding and source verification;
  - synthesis in `research/market/`.
- **JTBD ("switch") interviews** conducted by the project owner: 10 to 15. The guide and the pricing questionnaire are provided.
- **Feeds** D-segment, D-pricing and D-feature-priority.

## 6. Phase outputs

- Retained claims, with their evidence and their host class.
- Market synthesis.

The detail of each phase's files and tasks is tracked in the repository's GitHub issues, one per phase.

## 7. Promotion gates, without invented figures

- **Behavior**: all mandatory invariants of the conformance suite pass; each uncovered capability is displayed as unqualified. A perfect suite does not establish reliability on all real sites.
- **Backend**: replace/promote a backend only at comparable quality with a measured useful advantage, or a clearly documented capacity/distribution reason. The previous gain targets are project targets, not observations.
- **AI/repair**: comparison on a separate corpus, full costs and silent errors/refusals included; no promotion based merely on the same model's confidence.
- **Publication**: stable contracts for the announced scope, examples/docs, compatibility, known issues, packaging and recovery procedure verified.
- **Service**: capacity/isolation/quotas and usage actually accounted for; the SLA/price sold follows from measurements and the supported scope.

Production SLOs — availability, queue latency, capacity and recovery objectives — remain to be set after measuring the first system. The dossier does not turn "ultra high-performance" into a figure without a workload or evidence.
