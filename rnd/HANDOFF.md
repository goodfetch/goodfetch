# Handoff prompt — Automan

> Paste this into a new session (or give it to Wayfinder) to take over the project. It explains what the owner wants to build, who the main competitors are, what the R&D phase did and how to continue. Everything referenced lives in this repository (`Jbmanllr/automan`).

---

## Your role

You are taking over **Automan** after a completed, evidence-driven R&D phase. Your job is to turn that R&D into the real product documentation (adopted ADRs, glossary, architecture, then a PRD) and, later, into the implementation plan. The project owner makes every final decision; you propose, justify with evidence, and ask when a choice is genuinely theirs.

Read in this order before doing anything:
1. [`rnd/README.md`](README.md) — the final R&D report: stack before vs after testing, decision by decision, with evidence strength and open questions.
2. [`rnd/proposals/proposition-automan.md`](proposals/proposition-automan.md) and [`rnd/proposals/architecture.md`](proposals/architecture.md) — the product vision and the architecture written before testing.
3. [`rnd/adr/`](adr/) — three proposed ADRs (runtime language, access and identity, session model).
4. [`rnd/research/market/`](research/market/) — demand research, competitors, segments, pricing signals, interview kit.
5. [`AGENTS.md`](../AGENTS.md) and [`docs/agents/`](../docs/agents/) — repository conventions and the configuration of the engineering skills (issue tracker, triage labels, domain docs).

---

## 1. What the owner wants to build

**Automan is an open-source web data and automation framework, plus a hosted service built on the same engine.** One engine executes everything; every surface consumes it: a web console, TypeScript and Python SDKs, a CLI, MCP tools for agents, a native macOS app and the cloud.

The engine combines what today requires several tools:
- **A durable crawl frontier**: deduplication, discovery, priorities, leases, per-domain politeness, computed end of crawl, exact recovery after a crash.
- **Several ways to reach a site, chosen per route on evidence**: plain HTTP, a discovered JSON API, a light engine when the page allows it, a full browser when it must. The engine learns the cheapest route that still gives the same verified result.
- **Reusable plans**: a workflow is compiled once, replayed deterministically, and repaired within a budget when the site drifts. AI is used to create and repair plans, not on every run.
- **Explicit verification**: a result is "done" only when it passes a contract. A blocked page, an empty list, a login page served with HTTP 200, or a table that changed while being paged must never come back as a success.
- **Identity and compliance by default**: the agent declares itself (Web Bot Auth signatures, product token), applies robots.txt, Content-Signal and TDMRep for the declared purpose, and treats 401/402/403/challenges as stop signals. A `stealth` profile (proxies, fingerprints, CAPTCHA handling) exists as an explicit opt-in catalog, owned by the project owner.
- **Local first, same semantics in the cloud**: plans, results, sessions and diagnostics are portable; fundamental functions need no cloud key; users can bring their own models and proxies.

**Business model (proposed, not validated):**
- an open framework (Apache-2.0 proposed);
- a hosted service with managed browsers, proxies and support, billed per **verified result** rather than per request or credit, with hard caps.

Segment, pricing and feature priorities are still to be settled by customer interviews.

**The differentiation it aims for.** These are hypotheses from the market research; the lab has confirmed some of them technically:
1. A local, versionable cache and replay, independent of any vendor cloud.
2. Typed failures and zero false success.
3. Billing that does not count failures.
4. Sessions that survive restarts and updates.
5. The same product when self-hosted.
6. Politeness and identity by default.
7. An agent interface that is cheap in tokens and stable across versions.

---

## 2. Main competitors

Automan sits at the junction of four markets. None of these players covers all of it; users often combine several of them. That combination is the opening.

| Category | Players | What they do well | Where users say they fall short | What Automan takes or answers |
| --- | --- | --- | --- | --- |
| **Open-source crawling frameworks** | Crawlee (Apify), Scrapy, Spider-rs, Crawl4AI, Scrapling, Colly | Frontier, autoscaling, sessions, proxies, pipelines; Spider's fast native HTTP; Crawl4AI's LLM-ready Markdown | Results and acknowledgement are not one transaction; re-executable handlers; on-disk visit records are not a durable frontier; anti-bot becomes a full-time job | A durable frontier with exact recovery; routing on verified results; journaled sessions |
| **Browser automation** | Playwright, Puppeteer, Stagehand v4 + Browserbase, Lightpanda, Chrome DevTools MCP / Playwright MCP | Locators, actionability, multiple engines (Playwright); natural-language `observe/act/extract` and batching close to Chrome (Stagehand); a very light engine (Lightpanda) | Native Playwright let clicks land on moving targets in our tests; Stagehand's cache is tied to Browserbase's cloud; Lightpanda fails on shadow DOM | Direct CDP driver with Automan's own actionability guard (exact on every engine); light engine as a capability-chosen tier |
| **AI browser agents** | Browser Use, Skyvern, Magnitude, OpenAI/Anthropic computer-use agents | Agent loops, multimodal perception, workflows from a goal | Non-determinism, cost and latency per run; a model's own validation is not truth | Compile once, replay deterministically (0.17–0.34× the model cost in our tests); MCP and function tools from one catalog; code mode with an effect journal |
| **Hosted scraping and extraction APIs, proxies** | Firecrawl, ScrapingBee, Zyte, Bright Data, Apify platform, Oxylabs, Browserbase / Steel / Hyperbrowser (hosted browsers) | One API call for rendering, proxies and unblocking; LLM-ready output; managed infrastructure | Price and opaque credits; failures billed; lock-in; reliability varies by site | Billing per verified result; the same engine self-hosted; typed failures |

Cloudflare is a **gatekeeper rather than a competitor**: its verified-bot programs and Web Bot Auth are why Automan's default profile is an identified agent.

The full landscape covers 64 players, with prices, sources and dates:
- [commercial landscape](research/2026-10-07-commercial-landscape.md);
- [competitors' strengths and gaps](research/market/competitors-strengths-gaps.md);
- [crawl capability matrix](research/2026-10-07-crawl-capability-matrix.md);
- [agent capability matrix](research/2026-10-07-agent-capability-matrix.md).

---

## 3. The R&D phase, explained

**Why it existed.** Before writing a PRD, the owner wanted the stack chosen by measurement, not by opinion, and performance claims that can be proven. The [plan](plan/rd-plan.md) ran phases P0–P9.

**How it works.**
- **Decisions are declared before measuring.** Each question is a decision file in [`lab/decisions/`](lab/decisions/), with:
  - options and a baseline;
  - a primary metric and the smallest effect of interest (10%);
  - contract gates;
  - a statistical rule;
  - the host class required.
- **The [lab](lab/README.md) is the test harness.** It holds fixture sites with private oracles (they know the true records), adapters for every candidate backend, lanes per domain (bench, storage, workflow, runtime, actionability, extraction, AI, identity, admission, portal, topology), and an engine that records provenance and host load.
- **Contract first.** An option that produces a single **false success** (a result reported complete that the independent oracle refutes) is blocked, whatever its speed.
- **Verdicts are computed, never typed.** Blocks are paired and compared with log-ratios, a bootstrap, Wilcoxon and TOST. The result is `better`, `worse`, `equivalent`, `non-inferior`, `insufficient-n`, `blocked-by-contract` or `contract-met`, stored in [`lab/verdicts/`](lab/verdicts/). Every run is classified in the [ledger](lab/ledger/results-ledger.json) and cited in a dated [report](lab/reports/).
- **Host classes:**
  - `local-dev`: Mac in use; fine for contracts and counts.
  - `local-controlled`: quiet Mac, checked before the run and at every block; fine for timings.
  - `reference`: bare-metal Linux; required to publish numbers, **not available yet**.

**What it concluded.** The architecture of October 7 holds. The tests confirmed the core choices:
- Rust/Tokio core;
- SQLite WAL `synchronous=FULL`, with zero acknowledged commits lost over 6 simulated power cuts;
- reqwest for HTTP;
- compiled AI replay.

They also added four things the proposal lacked:
1. **A discovered-API routing tier**: 120–200× faster than a browser once a route is learned.
2. **An effect journal for agent code mode**: 17/45 false successes without it, 0 with it.
3. **Learned frontier rules**: offset pagination on a live table loses rows and claims completion; cursor pagination does not.
4. **A session model that judges page content, not HTTP status.**

They changed one thing: the default browser driver is **direct CDP with Automan's actionability guard**, and Playwright is no longer the behavioural reference.

The full before/after table, with evidence strength (🟢 strong, 🟡 medium, 🟠 weak, ⚪ untested) for 20 domains, is in [`rnd/README.md`](README.md), sections 3–5.

**What is still open:**
- the browser-driver verdict is blocked: the Playwright baseline timed out, and changing the baseline is the owner's call;
- the scheduler and some routing and code-mode verdicts need more blocks;
- no number is publishable before a bare-metal Linux reference host;
- Firecracker is untested (it needs KVM);
- segment, pricing, feature priorities and the sinks customers want need interviews;
- the stealth profile was out of scope.

---

## 4. Guide: how to continue

**Next steps, in order:**
1. **Adopt decisions as final ADRs** in `docs/adr/`, using the Matt Pocock format (`docs/agents/domain.md`). Write one ADR per 🟢 row of the report's table. Each cites its verdict (`rnd/lab/verdicts/D-*.json`) and report. The ADRs in `rnd/adr/` are drafts to rework, not adopted decisions.
2. **Build the final glossary** at the repository root from [`rnd/GLOSSARY.md`](GLOSSARY.md), adding the terms the R&D introduced: learned route, discovered-API tier, session page contract, effect journal, actionability guard, false success, host class.
3. **Write the final architecture** in `docs/`, starting from `rnd/proposals/architecture.md` and integrating the report's eleven refinements (section 1).
4. **Keep open questions explicit** (🟡 and ⚪ rows, report section 5), each with what would settle it.
5. **Write the PRD** only once interviews have given segment, pricing and priorities. The [interview guide](research/market/interview-guide.md), [pricing questionnaire](research/market/pricing-questionnaire.md) and [recruitment kit](research/market/recruitment.md) are ready; the owner runs the interviews.

**Working rules in this repository:**
- **Git:** one branch and one pull request per piece of work against `main`. Merge only when checks pass, and check the exit code, not filtered output.
- **Files:** never delete maintained files; prefer reversible exclusions.
- **Scope:** keep repository-scoped requests inside the repository; do not propose machine-wide changes.
- **Lab:**
  - Commands run from `rnd/lab` (`pnpm lab validate | run | import | verdict | report | ci`).
  - Never edit lab sources while a campaign runs: the run is then marked `invalid-provenance`.
  - Timing campaigns need a quiet Mac. Check with `pnpm lab preflight local-controlled`. If another session is compiling, ask it to pause through the owner.
- **Evidence:**
  - Every new option is declared in `lab/decisions/` before it is measured.
  - Every selected run is classified in `lab/ledger/classification.json` and cited in a report.
  - Never change a decision's baseline or rule after seeing results without the owner's approval.
  - Never publish `local-dev` or `local-controlled` numbers as product claims.
- **Restricted work:** detection-evasion tooling (patched browsers, fingerprint spoofing, behaviour simulation, CAPTCHA solving) is owner-led, not something to build autonomously.

**Key places:**

| Path | What it is |
| --- | --- |
| `rnd/README.md` | Final R&D report: start here |
| `rnd/proposals/` | Pre-test vision, architecture, API, product design, docs plan, console mock |
| `rnd/plan/` | R&D plan, reference-host preparation |
| `rnd/adr/` | Proposed ADRs |
| `rnd/GLOSSARY.md` | R&D vocabulary draft |
| `rnd/research/` | Sourced technical and market research |
| `rnd/lab/` | Test harness, decisions, ledger, verdicts, reports |
| `docs/` | Final product documentation (to be written); `docs/agents/` configures the skills |
