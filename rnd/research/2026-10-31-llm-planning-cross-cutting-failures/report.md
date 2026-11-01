# Locally correct, globally slow: cross-cutting quality failures in agent-built software

Research report, 2026-10-31. It answers five questions about one failure class: software planned and built by LLM coding agents, split into specs, tickets and vertical slices, where each piece is correct on its own and the system degrades as a whole. Foxhound's per-page crawl cost (#1577) is the case that prompted it.

**Status.** R&D material (`rnd/`): nothing here is an adopted decision. The recommendations are proposals for the owner.

**Method.**
- Five research strands, each written to `strands/`:
  - (A) empirical evidence;
  - (B) established engineering concepts;
  - (C) a survey of 22 agent tools and harnesses;
  - (D) practices that work;
  - (E) a read-only scan of Goodfetch's own code for the same pattern.
- An independent check of 18 load-bearing claims against their primary sources (`strands/v-verification.md`).
- Every web source was accessed on 2026-10-31. `sources.md` lists every source with what it supports.
- Local material read first: the profile (`.claude/handoff/m1/reports/profile-2026-10-30/report.md`), #1577 with its comments, ADRs 0023, 0071, 0133 and 0134, draft ADR 0138 with its grilling notes, the M1 run's perf reports, and the Matt Pocock skills in `~/.agents/skills` (local copy at commit `62a4b41`, 2026-10-23; upstream `mattpocock/skills` at `a496f55`, 2026-10-29).
- Nothing was built or run. Exa was unavailable; nothing was paid for.

---

## 1. Executive summary

1. **The failure is real, documented and has names.** The established name is **architectural drift** (Perry and Wolf 1992): structure lost through insensitivity to an architecture nobody stated, as opposed to *erosion*, which violates a stated one.
   - Its mechanism is **piecemeal growth** without its counterweight (Foote and Yoder 1997).
   - What drifted is a **cross-cutting quality attribute**: performance efficiency, an "aspect" in Kiczales et al.'s 1997 sense.
   - The timing error has a performance-engineering name of its own, **fix-it-later** (Connie Smith).
   - The guards failed through **normalized budgets**: relative per-change thresholds a commons can be grazed under, and absolute ceilings turned informative.

   The proposed short name is **unowned cross-cutting quality drift**.
2. **Agent workflows make it more likely and faster.** The 2026 evidence on agents extending their own code is strong and consistent:
   - Erosion rises in most trajectories, at 5–7 times the human rate per step (SlopCodeBench).
   - Scores fall from over 80% on isolated tasks to at most 38% in continuous evolution (SWE-Milestone).
   - Adopting Cursor raises lasting complexity while its speed gain fades within about two months (He et al., difference in differences).
   - Agents measure performance less often than humans when they claim it (45.7% against 63.6% of performance PRs).
   - Quality prompts lower the starting point but **do not change the rate of decline**. Structure has to do that.
3. **No surveyed tool closes the gap by default.** Across 22 tools:
   - **Cross-cutting qualities appear only as optional, per-feature fields:** Spec Kit's *Performance Goals*, BMAD's optional *Cross-Cutting NFRs*, Kiro's *Design-First* mode.
   - **Project-wide standards are advisory context**, with one exception: Spec Kit's constitution gate, which checks only the current feature.
   - **Review is centred on the diff everywhere.** No tool adds up what all features cost on a shared path, names an owner for a shared resource across specs, or reviews a change against its sibling tasks.
   - **Matt Pocock's skills have no performance, NFR or budget section** in `to-spec`, `to-tickets`, `code-review` or `implement-spec`. They assume the project's ADRs and standards carry cross-cutting architecture.
4. **What works is structural, absolute and automatic.**
   - **An owner for each shared path:** an architecture decision and a module that features plug into, per Parnas, Brooks, SAFe's runway and Team Topologies' platform.
   - **Holistic fitness functions with absolute, blocking limits:** Lighthouse CI `error`, size-limit, Gungraun hard limits, Duffy's gated rings, Chromium's revert after one day.
   - **Comparison against a fixed baseline, not only the parent commit:** Go against the last release, SQLite across 61 releases, ClickHouse "over the years".
   - **A measurable target agents can run themselves:** Anthropic's take-home, AlphaEvolve.

   All of these fit "the repository adds standards, not process steps" (ADR 0133). Sheriff rotas and periodic review meetings do not, and are not needed if the checks gate.
5. **Goodfetch had the right instruments, aimed at the wrong place and set to non-blocking.**
   - ADR 0134 named the risk itself: "accepted steps under 5% add up unseen".
   - On 2026-10-22 every perf figure became informational until the optimization pass.
   - The periodic `just perf` measured startup and JSON-RPC dispatch, not a crawl.
   - The first whole-crawl benchmark arrived on 2026-10-27 and read 234.45× against its 2× ceiling.
6. **The same pattern exists outside the crawl.** Strand E found 17 confirmed and 5 suspected cases:
   - the CDP command log opens a file per command;
   - each page read creates a new isolated world;
   - each Audit entry re-reads both TOML files and opens two SQLite connections with an fsync;
   - every ledger call opens a connection;
   - the crawl Scope is rebuilt and its globs recompiled per links batch on the writer;
   - each governor sample re-reads the config and `/proc/meminfo` three times.

   Collie's per-action, per-navigation and per-request paths have **no ceiling at all**.
7. **Top recommendations** (section 7):
   1. A hot-path ownership ADR that generalises ADR 0138 to every per-unit path of both Products.
   2. Exact structural counts per unit as **blocking** ceilings on each path.
   3. A fixed-baseline drift gate.
   4. No quality gate informational without an expiry.
   5. A `TESTING.md` rule that each hot path has a whole-composition benchmark from the change that creates it.

---

## 2. The failure class, named

### 2.1 What happened in Goodfetch, as a timeline

| Date | Event | Source |
|---|---|---|
| ADR 0071 (Oct 2026) | Per-change Performance budget: +5% instructions and heap against the change's **merge base**. An increase passes with a "Performance change" section. | `docs/adr/0071-*.md` |
| 2026-10-18 | ADR 0134 adds absolute ceilings (2× body bytes per crawled page, 8 B per Frontier URL, 1.10× tuned figures) and Performance targets (pages/s at least Spider's). Its context states the risk: "accepted steps under 5% add up unseen". Speed is deferred to an **optimization pass at the end of M1** (#1577). | `docs/adr/0134-*.md`, #1577 |
| 2026-10-20/21 | The M1 run's periodic `just perf`, every 10 merged tickets, measures **startup and dispatch only** and notes waste it does not fix, such as `Scope::widen` preparing its INSERT per seed, 55% of a benchmark. | `.claude/handoff/m1/perf-gate-orchestrator-notes.md`, `reports/perf-checkpoint-3.md` |
| 2026-10-22 | Owner decision: **all performance gating informational** until the pass ends (`GATING = Informational`), because from a nearly empty `main` "the per-change budget produced paperwork rather than signal". | ADR 0071 and 0134 amendments; `reports/fix-perf-informational-m1.md` |
| 2026-10-27 | #1572's first **whole-crawl** heap benchmark reads **234.45×** body bytes against the 2× ceiling, "noted only". The first `compare-spider` figure is 31.6 against 4,981 pages/s. | `reports/t1572.md`, #1577 comment of 2026-10-30 |
| 2026-10-30/31 | Owner finds the comparison unacceptable. The profile shows 32× Spider's instructions, 329 syscalls against 12 per page, and 8 commits, 8 parses and 135 prepares per page. A grilling session drafts ADR 0138. | profile report; #1577 comments 6534699879 … 6536582421 |

Each feature spec (trap detection, canonical, politeness, durability, ledger, robots.txt, datasets, cosmetic clean-up) added its own layer to every page's path. Each layer did its own parse, commit and SQL. Each was correct, tested and reviewed against its own spec. No ADR, spec or ticket designed the shared per-page pipeline or owned its total cost. ADR 0002's promise of "many operations grouped per commit by the single writer" was never checked by anything that measured it.

### 2.2 The established names

| Part of the failure | Established term | Primary source | Fit |
|---|---|---|---|
| Structure lost without anyone violating a stated design | **Architectural drift**: "due to insensitivity about the architecture … results in a lack of coherence and clarity of form" | Perry and Wolf 1992 | Exact. Better than *erosion*, because there was no stated pipeline to violate. |
| The growth mechanism | **Piecemeal growth**, whose risk is to "gradually erode the overall structure", countered only by "Refactor unrelentingly". Also "focus first on features … then … performance" as the force that produces mud. | Foote and Yoder, *Big Ball of Mud*, 1997 | Exact: every unit of work was a feature slice, and nothing was charged with consolidation. |
| What drifted | **Cross-cutting concern / aspect**: properties that "affect the performance or semantics of the components in systemic ways". The motivating image-filter example, where each filter loops separately, gives "terrible performance", and the fix needs "a more global perspective". | Kiczales et al., ECOOP 1997 | The closest published precedent: the per-feature HTML parse, with images in place of pages. |
| Why feature slicing duplicates base work | Decomposition by processing step rather than **information hiding**; a module is "a work assignment" | Parnas 1972, 1978 | A ticket is a Parnas module, so a feature slice re-makes the shared hard decisions (parse, commit, prepare). |
| The decomposition is the design | **Conway's law**: "the very act of organizing a design team means that certain design decisions have already been made" | Conway 1968 | The ticket graph is the organisation. No ticket for the pipeline means no pipeline. |
| No single holder of the design | **Conceptual integrity**: "the design must proceed from one mind, or from a very small number of agreeing resonant minds" | Brooks, *The Mythical Man-Month*, ch. 4 | Each slice is a fresh mind. Integrity has to live in artifacts every session loads. |
| The unmade decision | ATAM **risk**: "architecturally important decisions that have not been made"; the pipeline was a **sensitivity point** for throughput | Kazman, Klein, Clements, SEI TR-004, 2000 | Exact SEI wording for "no ADR owned the per-page pipeline". |
| Deferring speed | **Fix-it-later**, premise "You can tune software later, if necessary". Also Knuth's full passage: "we should not pass up our opportunities in that critical 3%". | Smith 2015 (SPE); Knuth 1974, p. 268 | The per-page path *is* a crawler's critical 3%. Knuth's rule protected it; the deferral misread him. |
| Cost spread too thin to profile | **"Peanut butter"** problems, "death by a thousand cuts", flat profiles | Duffy 2010; Rousskov, squid-dev 2010; Kanev et al. 2015 ("no killer application", datacenter tax ≈ 30%) | Explains why a late pass is expensive: no hotspot is left to find. |
| Each slice gains, all share the cost | **Tragedy of the commons**: a positive utility "nearly +1" against "only a fraction of -1" | Hardin 1968 | The per-page budget is a commons; a review of one diff against its parent sees +1 every time. |
| Guards that fire but do not block | **Normalization of deviance**; a budget must be "integrated, and enforced … a hard line" | Vaughan 1996; Kadlec 2019 | An informative ceiling is not a budget in Kadlec's sense. |
| Holistic checks missing | **Holistic fitness function** (versus atomic) | Ford, Parsons, Kua; Thoughtworks Radar 2017 | Per-page cost is holistic: only the pipeline with every feature on shows it. |

**Proposed name:** *architectural drift by piecemeal growth around an unowned cross-cutting quality attribute, under a fix-it-later stance with normalized budgets.* Short form: **unowned cross-cutting quality drift**. On its own, *architectural drift* is the best single established term. ATAM's *risk* is the best term for the root cause.

**What is new in the agent case** (inference from strands B and C):
1. **Slices land in hours,** so the "last responsible moment" (Poppendieck) for shared decisions passes almost at once.
2. **Agents keep no memory across tickets** and see the system through their brief and the diff, so Conway's law holds strongly.
3. **Every review is scoped to one diff and its spec,** so Hardin's asymmetry is built into the process rather than a matter of motive.
4. **Agents read "informational" literally** and record a breach as a known item instead of stopping (Böckeler 2026 saw agents raise lint thresholds rather than refactor).

### 2.3 Which classical ideas transfer, and how

| Idea | Transfers as | Limit |
|---|---|---|
| Quality attribute scenarios (Bass, Clements, Kazman) | A spec states a whole-path scenario: stimulus, environment *with every feature on*, artifact *the whole path*, and a response measure that becomes a gate. | Written once, so it needs a gate to stay true. An agent can scope the scenario to its own slice, which recreates the failure one level up. |
| ATAM, utility tree, sensitivity points | Before tickets, list the sensitivity points each feature touches. Any ticket that touches one goes through that point's owner (an ADR or a module). | ATAM is a facilitated multi-day workshop. An LLM agrees with itself, so it serves as a prompt, not as assurance. |
| Fitness functions | Triggered, holistic and absolute: a whole crawl of a reference corpus, counted, on every merge. | Holistic checks are slow and noisy, so teams demote them. Here they were demoted by decision. Counted figures avoid the noise. |
| Architectural runway and enablers (SAFe) | An enabler ticket builds the shared path first, and feature slices consume it. | It needs someone with authority to refuse a slice. In agent work that authority is a gate. |
| Performance budgets | Absolute limits per unit: parses, commits and prepares per page. | Allocating a budget per ticket needs a cost model, and agents cannot invent a sound one. Measure instead. |
| Conway and Inverse Conway | Shape the ticket graph like the target architecture: the first ticket owns the path and its cost contract, and features are plug-ins. | It needs the target architecture first, which was the missing piece. |
| Team Topologies | The per-page pipeline as a platform with an owner. A context window is the agent's cognitive load. | Agents are short-lived, so the owner must live in artifacts (ADR, module, gate). |
| Walking skeleton and tracer bullets | The first slice is the thin end-to-end path *with its benchmark and ceiling*, and every later slice adds to that one path. | A skeleton fixes the shape, not the cost, unless it carries the gate. |
| Second-system effect | Does **not** fit: there was no over-design from pent-up ideas. Only Brooks's chapter 5 epigraph fits: "add little to little and there will be a big heap". | Do not cite it as the cause. |

---

## 3. Evidence (question 1)

Grouped by sub-class. **Direct** means the source shows each change locally acceptable and the system degrading; **adjacent** means generic quality evidence. VERIFIED means the primary text, or the abstract where noted, was read by strand A and spot-checked in `strands/v-verification.md`. Vendor reports from firms selling related products are marked COI. All accessed 2026-10-31.

### 3.1 Architecture by accretion and erosion over iterative agent changes (strong)

- **SlopCodeBench** (Orlanski et al., arXiv:2603.24755, v2 May 2026, NeurIPS 2026) — https://arxiv.org/abs/2603.24755
  - Setup: 15 models in their native CLI harnesses (Claude Code, Codex CLI, …) extend their own code across 196 checkpoints without seeing the tests.
  - Erosion rises in **77%** of trajectories and verbosity in **75.5%**.
  - Against 473 human repositories, agent code is **2.3× more verbose and 2.0× more eroded**, and degrades **5–7× faster per step**.
  - Cost per checkpoint grows 2.2×.
  - Anti-slop and plan-first prompts cut the initial verbosity and erosion **but do not change the rate**. This is direct evidence that small per-step increments compound and that prompting does not bend the slope.
- **He, Miller, Agarwal, Kästner, Vasilescu**, "Speed at the Cost of Quality" (arXiv:2511.04427, MSR 2026) — https://arxiv.org/abs/2511.04427
  - Setup: staggered difference in differences on 806 repositories that adopted Cursor against 1,380 matched controls.
  - Velocity gains vanish after about two months.
  - Static-analysis warnings stay about **+30%** and cognitive complexity about **+41.6%**; the paper also gives 25% elsewhere.
  - The accumulated complexity predicts slower work later.
- **SWE-Milestone** (formerly EvoClaw, arXiv:2603.13428) — https://arxiv.org/abs/2603.13428
  - Scores fall from **>80%** on isolated tasks to at most **38.03%** in continuous evolution.
  - Recall of new features grows roughly linearly while precision (not breaking what exists) saturates.
- **SWE-STEPS** (arXiv:2604.03035) — https://arxiv.org/abs/2604.03035. Success falls 15–25 points once state carries over between PRs. The dominant failure is spillover from earlier inefficient or buggy code.
- **SWE-CI** (arXiv:2603.03823) — https://arxiv.org/abs/2603.03823. On the problems they solve, all 20 models score **worse than the human reference solutions on the Maintainability Index**, while 15 of 20 beat them on Pylint. Local, per-file quality improves while system-level maintainability declines.
- **Practitioner experiments.**
  - Böckeler, "Maintainability sensors for coding agents", 2026 (https://martinfowler.com/articles/sensors-for-coding-agents.html): near-identical route files, a re-implemented hook, one change touching **40+ files**, and agents raising lint thresholds instead of refactoring.
  - Böckeler, "How far can we push AI autonomy", 2025 (https://martinfowler.com/articles/pushing-ai-autonomy.html).
  - Kent Beck, "Augmented Coding: Beyond the Vibes", 2025 (https://newsletter.kentbeck.com/p/augmented-coding-beyond-the-vibes): accumulated complexity stalled the agent.
  - Armin Ronacher, "The Coming Loop", 2026 (https://lucumr.pocoo.org/2026/6/23/the-coming-loop/): agents "duplicate code, invent bad abstractions"; code "too local in its reasoning".
  - Cursor, "Scaling long-running autonomous coding", 2026 (https://cursor.com/blog/scaling-agents): "agents conflict, duplicate work, and drift"; "periodic fresh starts" needed.

### 3.2 Cross-cutting non-functional qualities with no owner (moderate for performance and security; thin for memory, durability, cost)

- **SWE-Perf** (arXiv:2507.12415, ICML 2026) — https://arxiv.org/abs/2507.12415. On real repositories, the best agent gains **2.26%** against the experts' **10.85%**. The gap grows with the number of functions involved: whole-path optimisation is what agents do worst.
- **SWE-fficiency** (arXiv:2511.06090) — https://arxiv.org/abs/2511.06090. Agents reach **< 0.23×** the expert speedup; reasoning about execution across functions is a named failure mode.
- **Peng et al.**, "How Do Agents Perform Code Optimization?" (arXiv:2512.21757) — https://arxiv.org/abs/2512.21757. Explicit performance validation appears in **45.7% of agent performance PRs against 63.6% of human ones**. This is the closest primary evidence that nobody owns the measurement.
- **Function-level efficiency** (abstracts): correct code that is slower or uses more memory.
  - EffiBench (https://arxiv.org/abs/2402.02037): GPT-4 at 3.12× the canonical time; worst cases 13.89× time and 43.92× memory.
  - Mercury (https://arxiv.org/abs/2402.07844).
  - Rajput et al. 2026, memory divergence (https://arxiv.org/abs/2601.01215).
- **Security.**
  - Liu et al. 2026 (https://arxiv.org/abs/2603.28592), 302,579 AI-authored commits: AI commits introduce about 1.5× as many security issues as they fix, and 22.7% of introduced issues survive to HEAD.
  - Shukla et al. 2025 (https://arxiv.org/abs/2506.11022): efficiency-focused iterations produced the most memory-safety issues. Optimising one quality silently degrades another when no one owns both.
  - Veracode 2025 (COI): insecure choice in 45% of tasks.
- **Cost as a quality:** SlopCodeBench's cost per checkpoint grows 2.2×.
- **Cases.**
  - Carlini, "Building a C compiler with a team of parallel Claudes" (Anthropic, 2026, https://www.anthropic.com/engineering/building-c-compiler): output slower than GCC at `-O0`. Correctness had a "nearly perfect" verifier; performance had none until late.
  - Lovable, CVE-2025-48757 (https://mattpalmer.io/posts/statement-on-CVE-2025-48757/): 10.3% of scanned apps exposed data through missing or wrong Row Level Security. COI: the researcher's organisation is a competitor.
  - Replit's production database deletion (2025): reported only by the press, not checked at the primary source.
  - The Tea app breach is **not** evidence: nothing ties it to AI-written code.

### 3.3 Duplicated base work across slices (moderate)

- **Huang et al.**, "More Code, Less Reuse" (arXiv:2601.21276, MSR 2026) — https://arxiv.org/abs/2601.21276. Agent PRs show about **1.87×** the semantic redundancy of human PRs (0.2867 against 0.1532), and reviewers are milder toward them. Measured on one repository.
- **Böckeler**, "Understanding Spec-Driven-Development" (2025) — https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html. With Spec Kit, the agent treated research notes on existing classes "as a new specification and generated them all over again, creating duplicates".
- **GitClear** 2024 and 2025 (COI; method not published): copy-pasted lines rose from 8.3% to 12.3%, and moved (refactored) lines fell from 25% to under 10%. The trend is correlated with the AI era, not attributed per line.
- **Counter-evidence:** He et al. find duplicate line density up 7%, **not significant**.
- **Gap:** no primary study shows agents re-parsing or re-reading the same data across features. Goodfetch's profile (8 parses, 135 prepares and 8 commits per page) is itself a well-measured case of it. The nearest outside figure is CodeRabbit's "excessive I/O ~8× more common" in AI PRs (COI, 470 PRs).

### 3.4 "Correct first, fast later" deferrals that never pay off (indirect)

No study follows deferred performance work in agent-built projects over time. Four strands of evidence point the same way:
1. **Agents recover little performance when asked later** (SWE-Perf, SWE-fficiency, above).
2. **Writing well up front does not bend the curve:** SlopCodeBench's quality prompts change the start, not the slope.
3. **Debt is admitted and left.**
   - Liu et al.: 22.7% of introduced issues survive to HEAD.
   - Brahmi et al., MSR 2026 (https://2026.msrconf.org/details/msr-2026-mining-challenge/28/): agents' self-admitted debt describes the problem without a resolution.
   - Horikawa et al. (https://arxiv.org/abs/2511.04824): agents refactor in 26% of commits, mostly at a low level.
4. **Classical evidence.** Smith's SPE history: "In the late 1970s all of these premises became false". Kanev et al.: flat profiles leave no hotspot for a later pass.

Counter-evidence: function-level optimisation against a good measure works (PIE, https://arxiv.org/abs/2302.07867), and so do targeted optimisation loops (section 6.6).

### 3.5 Relative-only guards missing slow drift (indirect but convergent)

- **Small steps compound** (SlopCodeBench's growth rates; He et al.; SWE-Milestone's snowball).
- **The per-change guard is weak or gamed.**
  - SWE-Bench+ (https://arxiv.org/abs/2410.06992): 31% of "resolved" patches passed only because the tests were weak.
  - PatchDiff (https://arxiv.org/abs/2503.15223): 29.6% of plausible patches behave differently from the ground truth.
  - METR, Aug 2025 (https://metr.org/blog/2025-08-12-research-update-towards-reconciling-slowdown-with-time-horizons/): 0 of 15 agent PRs mergeable as-is.
  - METR, Mar 2026 (https://metr.org/notes/2026-03-10-many-swe-bench-passing-prs-would-not-be-merged-into-main/): maintainers merge about 24 points fewer patches than the automated grader passes.
  - ImpossibleBench (https://arxiv.org/abs/2510.20270): when the tests contradict the spec, Claude models mostly cheat by **modifying the tests**.
- **Human performance CI shows the mechanism directly.**
  - MongoDB's change-point detection (Daly et al., ICPE 2020, https://arxiv.org/abs/2003.00584): threshold-based detection was "not sufficient" and missed sub-threshold changes.
  - Meta's FBDetect (SOSP 2024): in Meta's serverless platform, about a tenth of roughly 800 regressions a year are 0.005–0.01% CPU, which together "would have wasted around 4,000 servers".
  - Squid 3.2: changes each "negligible" added up to 3–10%.
- **Goodfetch's own record** (section 2.1): ADR 0134 named this risk, then every figure was made informational.

### 3.6 Adjacent evidence

- **METR's RCT** (https://arxiv.org/abs/2507.09089): experienced developers were 19% slower with early-2025 tools while believing they were 20% faster.
- **DORA:**
  - 2024 (https://cloud.google.com/blog/products/devops-sre/announcing-the-2024-dora-report): more AI adoption, lower delivery stability.
  - 2025 (https://cloud.google.com/blog/products/ai-machine-learning/announcing-the-2025-dora-report): "AI doesn't fix a team; it amplifies what's already there".
- **Echoes of AI** (https://arxiv.org/abs/2507.00788): no maintainability penalty in a supervised setting (counter-evidence).
- **Watanabe et al.** (https://arxiv.org/abs/2509.14745): 45% of merged Claude Code PRs needed follow-up changes.

### 3.7 Where the evidence is thin

- No study measures whether agent-written specs or plans assign anyone to performance, memory, durability or cost.
- No study tracks the runtime or memory of a growing agent-built system across many changes.
- No primary study shows agents duplicating parsing or I/O across features.
- No agent-specific comparison of relative against absolute budgets exists.
- No study of durability defects (fsync, transactions, crash consistency) in agent code.

Goodfetch's profile and its guard timeline are a better-measured case than most of the published ones. A short write-up of it would be a contribution of its own, but it should not be published while ADR 0076 applies.

---

## 4. How agent workflows and harnesses address it (question 3)

From `strands/c-harnesses.md`, each claim read in the tool's own docs or repository at the version shown, 2026-10-31.

| Tool | Version/date seen | Step, template section or gate for cross-cutting qualities | Automatic or optional | Looks across specs or the whole system? | Persistent standards, and are they checked? |
|---|---|---|---|---|---|
| GitHub Spec Kit | v1.2.5; `main` `7370e59` (2026-10-30) | Plan *Technical Context* (Performance Goals, Constraints, Scale/Scope); *Constitution Check* GATE; `/clarify` "Non-Functional Quality Attributes"; `/analyze` "missing non-functional task coverage"; `/checklist` `performance.md`; tasks "Polish & Cross-Cutting Concerns" | The constitution gate runs in `/plan` (error if unjustified). The fields accept "NEEDS CLARIFICATION". `/analyze` and `/checklist` are user-run. The polish phase comes last. | No: every command reads one feature folder and the constitution | `constitution.md`, checked by `/plan` and `/analyze`, against the current feature only |
| Kiro (AWS) | docs updated 2026-08-04 to 10-21 | No NFR section in EARS requirements or design. "Design-First" mode suggested for "strict non-functional requirements" | Phase approvals; Analyze Requirements is offered; hooks are automatic once configured | No: task waves stay within one spec | `.kiro/steering/` (context only); Pre/Post Task Execution hooks could run a probe |
| BMAD Method | v6.12.4; `main` `61e01f8` | PRD Adapt-In "Cross-Cutting NFRs", "Performance Budgets", "Constraints and Guardrails"; architecture spine "Consistency Conventions", "Inherited Invariants" | All optional; the architecture reviewer gate scales with stakes and may be skipped | Partly: "An epic spine fixes the invariants the epic's stories must share" | Architecture spine; no automatic check of code against it |
| BMAD Test Architect | v3.6.0 | `nfr-assess` (security, performance, reliability, maintainability) | Optional: "(optional)" in the release gate; missing evidence gives CONCERNS | Yes, at `system` scope, but at release and as an audit | Reads NFRs from the PRD and architecture |
| Tessl SDD tile | 2.0.1; `b8fdff7` | Requirement gathering lists "Missing non-functional requirements" | Skills on demand | Partly: finds related specs by `targets` | Steering rules (context) |
| Matt Pocock skills | upstream `a496f55` (2026-10-29), 38 skills; local `62a4b41` | **None** in `to-spec`, `to-tickets`, `code-review`, `implement-spec`, `improve-codebase-architecture`. `codebase-design` lists "performance characteristics" as part of one module's interface. `code-review`'s Duplicated Code smell is limited to "the change". | n/a | `implement-spec` step 7 runs `code-review` on the integration branch. `improve-codebase-architecture` scans the codebase for testability and AI-navigability; it is `disable-model-invocation: true` | Repository standards, ADRs and glossary, read by `code-review`'s Standards axis |
| Claude Code | docs read 2026-10-31 | `/code-review` (correctness, plus "efficiency" at higher effort), `/security-review`, managed Code Review, hooks | Hooks are deterministic and can block (exit 2). The doc says to put what "has to happen every time" in a hook, deny rule or managed setting, because "Claude is allowed to decide a guideline doesn't apply". Managed review never blocks. | Diff plus surrounding code | `CLAUDE.md` (advisory), `REVIEW.md` |
| Anthropic harness posts | 2024-12-19; 2025-06-13; 2025-11-26 | Feature list, progress file, end-to-end smoke test. **No NFR mention** | n/a | Functional regressions only | Progress file |
| OpenAI Codex | docs 2026-10-31 | Code review flags only P0/P1; "Code Review Rules" in AGENTS.md | Automatic when enabled | "reviews the pull request diff" | `AGENTS.md` (not enforced) |
| Cursor | docs 2026-10-31 | Bugbot: "bugs, security issues, and code quality problems" (performance not listed); Plan Mode has no NFR section | Bugbot automatic; blocks only if configured | PR diff | `.cursor/rules`, `BUGBOT.md` (context) |
| Devin | docs 2026-10-31 | Playbooks; Devin Review (bugs, security); performance only as a `REVIEW.md` example | Review automatic, not required | Diff plus codebase context | Knowledge/Skills, `REVIEW.md` |
| Aider | v0.86.0 | None (architect/editor is a two-model edit split) | Auto-lint on; auto-test opt-in | No | `CONVENTIONS.md` (context) |
| OpenHands | docs 2026-10-31 | None; Planning Mode writes `PLAN.md` | User-selected | No | `AGENTS.md`, skills |
| SWE-agent / mini-swe-agent | docs 2026-10-31 | None | n/a | No: one issue by design | None |
| GitHub Copilot | docs 2026-10-31 | Code review: "bugs, security vulnerabilities, and style inconsistencies"; CodeQL Code Quality separately | Automatic if configured; not blocking by default | "Full project context gathering" | `copilot-instructions.md`, `AGENTS.md` |
| claude-task-master | 0.43.1 | Research mode asks about "scalability issues"; complexity scores effort only | Opt-in | No | None |
| Agent OS | v3.0.0 | "Are there any constraints or requirements I should know about?" | Standards injected after confirmation | No | `agent-os/standards/` (never compliance-checked) |
| Amp | checks post 2026-02-04 | `.agents/checks/perf.md` example: "Flags common performance anti-patterns" | `amp review`, on request | Changes, scoped by directory | `.agents/checks/` |
| CodeRabbit / Greptile / Qodo | docs 2026-10-31 | Custom pre-merge checks or rules; no built-in performance check | Automatic per PR; blocking only in special modes | Repository graph or code graph context | Configuration files; Qodo mines rules from history |
| MetaGPT | `main` 2026-10-31 | Architect `WriteDesign` with no quality-attribute node | Fixed SOP | One whole-program design up front | None |
| Cognition, "Don't build multi-agents" | 2025-06-12 | Essay: "Actions carry implicit decisions, and conflicting decisions carry bad results" | n/a | Argues that splitting loses shared decisions | n/a |

**Patterns** (strand C, verified per tool):
1. Cross-cutting qualities enter as **optional, early, per-feature fields**, never re-added across features.
2. Project-wide standards files are near-universal and **advisory**. Claude Code's own docs say rules without exceptions belong in hooks or CI.
3. **Review is diff-centred.** Some bots widen context to callers to catch breakage, but none reviews a change against its sibling tasks or a path's total cost.
4. Deterministic gating exists only as generic hooks or CI. **No tool ships a benchmark, a memory budget or a commit-count probe.**
5. **Decomposition tools optimise for independence:** "independently testable", "independently completable", vertical slices. Independence is what makes each slice reviewable and what hides a shared hot path.

So the Pocock skills are not unusual. They assume, as Spec Kit's constitution and BMAD's spine do more explicitly, that cross-cutting architecture lives in the project's own artefacts. ADR 0133 puts Goodfetch's standards exactly there. The failure is that those artefacts never owned the per-page path. The skills are not to blame for that.

---

## 5. Why the agent workflow did not catch it here, by level

| Level | Could it see across slices? | What happened | Verdict |
|---|---|---|---|
| Wayfinder maps (#61 "fully decided framework", #262 test strategy, #447 products, #461 SDK API) | Yes: a map covers a whole area of decisions | 102 grilling tickets, each on one topic (clean-up #119, politeness #480, size budgets #197/#454…). **None asks how a page goes through every feature, or what that costs per page.** Performance comes in only as measurement: map #61's completion criterion (d) "Measured speed: a reproducible comparative benchmark … against Crawlee, Spider", benchmark protocol #79, regression testing #272/#284 (giving ADR 0071). Speed was a figure to measure at the end, not a decision to take. | **Failed** the same way: wayfinder charts the questions it is given, and this one was never asked |
| ADRs and standards | Yes: the only level that sees across specs | No ADR owned the per-page pipeline or a per-unit total. ADR 0002's group-commit promise went unchecked; ADR 0134 deferred speed. | **Failed**, the root cause |
| Grilling and `/to-spec` | Partly: one area at a time, reading the ADRs | The template has no quality section. Specs respected the ADRs, which said nothing about the shared path. | Did as written |
| `/to-tickets` | No: vertical slices by construction | Each slice cut "through every layer", including its own parse and commit | Did as written; Conway and Parnas predict the result |
| `tdd` and implementers | No | Correct code at the agreed seams | Did as written |
| `code-review`, Standards and Spec | Diff against standards and spec only | `STYLE.md`'s hot-path rules covered bytes and channels, not parses, commits or prepares per page | Could not see it |
| Measured guards | Yes, if aimed and blocking | Relative (+5% against the merge base), then informational; the periodic check had no crawl benchmark until 2026-10-27 | **Failed:** aimed elsewhere, then normalised |

---

## 6. What works (question 4), weighed against ADR 0133

ADR 0133 allows ADRs, `STYLE.md`/`TESTING.md` standards, configuration and gates, and no new process steps. A gate in `just merge` or the Release suite is a standard the existing merge applies, not a step.

| Practice | Evidence | Fits ADR 0133? |
|---|---|---|
| **An owner for each shared path:** an ADR plus a module features plug into | Parnas 1972; Brooks; SAFe runway and enablers; Team Topologies platform; BMAD's "Inherited Invariants". Strand E shows each unowned path re-invents its own opens, connections and parses. | **Yes:** an ADR and a `STYLE.md` rule |
| **Absolute, blocking ceilings** | Lighthouse CI `error`, size-limit, Gungraun hard limits (exit code 3), Bencher `static`, dhat assertions; Duffy: "All commits must pass a set of gated performance tests"; Kadlec: "enforced … a hard line"; Chromium reverts a 0.3% competitive-benchmark regression after one business day | **Yes:** a gate |
| **Exact structural counts** (parses, compiles, prepares, transactions, fsyncs, opens per unit) | SQLite: cachegrind repeats to "7 or more significant digits" against about 1 for wall time; ADR 0138's argument that a 5% second parse passes a 1.10× ceiling twice | **Yes:** a gate plus `TESTING.md` |
| **Comparison against a fixed baseline** (last release, pinned build), not only the parent | Go: "the baseline is always the latest overall release"; SQLite across 61 releases; ClickHouse "over the years" found older releases faster; Rust's weekly net change. Most per-change detectors (Perfherder's window, rustc-perf's IQR fence, github-action-benchmark at 200%) move along with slow drift. | **Yes:** a gate |
| **Ratchet:** the ceiling only tightens | Betterer; blog advice; no canonical primary | **Yes:** a gate rule |
| **A competitor benchmark as the budget** | ClickBench's rules; TechEmpower (archived 2026-03-24, so no longer usable); Chromium's competitive benchmarks | **Yes:** a Release-suite cell or gate |
| **Continuous profiling / profile diff** | Google-Wide Profiling; Kanev et al.; FBDetect; Parca and Pyroscope are free and self-hosted. At fleet scale this does not apply yet; a HEAD-against-baseline profile diff does. | **Yes:** a gate or `TESTING.md` |
| **Fitness functions for structure** | Thoughtworks Radar; ArchUnit; import-linter; cargo-deny `wrappers`; Clippy `disallowed-methods` (already used by Goodfetch); OpenAI's harness post (secondary): custom linters with remediation text for agents | **Yes:** lints and a gate |
| **Quality scenarios in the spec** (arc42 §10, Volere §12, back-of-the-envelope sketch) | arc42 makes scenarios "specific and measurable"; TIGER_STYLE: "Think about performance from the outset"; Smith's SPE early model. Effectiveness evidence is anecdotal. | **Yes**, as a content standard for specs that `/to-spec` reads through the ADRs and `docs/agents/`. Not a new step. |
| **Walking skeleton carrying the gate** | Cockburn, Freeman and Pryce, Hunt and Thomas (verified only through the Artima interview); TIGER_STYLE | **Yes:** `to-tickets` already says "Any prefactoring should be done first". The owner module and its benchmark are prefactoring. |
| **A measurable target the agent can run** | Anthropic take-home (a pre-release Opus 4.5 checkpoint "found the trick" once told the achievable cycle count); AlphaEvolve recovered 0.7% of Google's compute; Kevin-32B. Counter-case: Sakana's CUDA engineer gamed its evaluator (1.13× fell to 0.82× on replication). | **Yes**, if the gate checks the output before timing it (Goodfetch's same-work check already does) |
| Periodic whole-codebase architecture scans | Weak evidence: practitioner claims; Böckeler's scheduled sensors; OpenAI's background clean-up agents (secondary) | A scheduled agent review **would be a process step**. `improve-codebase-architecture` is a skill the **owner** runs, so running it is skill usage, not a repository step. |
| Sheriff rotas, weekly human triage, admin acknowledgement (CodSpeed) | Mozilla, Chromium, Rust | **No:** process steps. Not needed when the gate blocks. |
| Quality prompts ("write efficient code", "plan first") | SlopCodeBench: they lower the start, not the slope | Allowed but **insufficient** on their own |

**Conclusion for ADR 0133.** Everything that works here is a standard or a gate:
- an owner in an ADR;
- a rule in `STYLE.md`;
- a count in `TESTING.md`;
- a check `just merge` runs.

Nothing needs a new workflow step. The one thing that resembles a step, a periodic architecture scan, is better done by making the whole-path checks gate on every merge, which is the per-merge form of a scan.

---

## 7. Recommendations for Goodfetch (question 5), in priority order

Each item names the level it lands at and the evidence behind it. ADR 0138 already covers the per-page crawl path: one Page analysis, a demand plan, Run invariants built once, grouped commits, seven structural counts as ceilings, `disallowed-methods` lints and two step-7 review items. These recommendations **generalise** it, not repeat it.

### R1. One ADR that gives every per-unit hot path an owner and a cost contract (ADR; next free number after 0139)

- **Inventory.** List the hot paths of both Products and the engine, one per unit:
  - Foxhound's page, URL admission, Record and request;
  - Collie's action, navigation, page read and intercepted request;
  - the engine's JSON-RPC message, Audit entry, ledger charge and governor sample.
- **Owner and contract.** Each path gets one owning module and a **cost contract**: the counted work allowed per unit (parses, selector, regex and validator compiles, statement prepares, SQLite connections opened, transactions, fsyncs, file opens, configuration reads, CDP round trips).
- **Plug-in rule.** A feature on a path plugs into its owner and states what it adds. A new path gets its owner in the change that creates it.
- **Shared owners from strand E:**
  - a Run-trace writer that keeps one handle per file (CDP log, `requests.jsonl`, Provenance);
  - a store opener that sets up each SQLite store once per engine (Audit log, ledger, known certificates);
  - one machine-settings snapshot, invalidated when the files change (admission, governor, Audit);
  - Run invariants built once (Scope, Payload rules, Fragments, validators);
  - one cached isolated world per document, and the guard script kept as a handle.
- **Evidence:** Parnas; Kiczales; Brooks's conceptual integrity; ATAM risk; SAFe enablers; strand E's 17 confirmed cases; ADR 0002's unkept group-commit promise.
- **Why an ADR:** it is the only level that sees across specs (section 5), and `/to-spec` and `/to-tickets` already read the ADRs in the area they touch.

### R2. Exact structural counts per unit as blocking ceilings on every inventoried path (gate + `TESTING.md`)

- **Extension.** Extend ADR 0138's seven crawl counts to every path in R1, with the counts that matter there. File opens, fsyncs, connections and CDP round trips are exact and do not depend on scheduling. Futex and epoll waits do, which is why ADR 0138 excludes them.
- **When they block.** From the change that adds each count. Not after a later pass, and independent of `GATING`.
- **Where they run.** Through the ops listener metrics ADR 0138 already plans, read by the in-memory scenario of each path.
- **Evidence:** Lighthouse CI, size-limit and Gungraun hard limits; Duffy; Chromium; SQLite on exactness; strand E's gap ("No absolute figure in ADR 0134 counts syscalls, file opens, fsyncs, SQLite connections or CDP round trips per unit").

### R3. A fixed-baseline drift gate beside the per-change +5% (gate; amends ADR 0071 and 0134)

- **What it compares.** `xtask perf` also compares HEAD with a pinned **reference build**: the tuned build of #1577, then each release tag.
- **Threshold.** A cumulative threshold: for example, no more than 1.10× the reference on any counted figure without an owner-approved ADR amendment. That is what ADR 0134's 1.10× ceilings already mean once set. This makes the reference explicit and keeps it fixed between owner decisions.
- **Evidence:** Go `perf_vs_release`; SQLite; ClickHouse "over the years"; Rust's weekly net change; FBDetect's 0.005% regressions adding up to about 4,000 servers in Meta's serverless platform; ADR 0134's own context.

### R4. A quality gate is never informational without an owner and an expiry (ADR amendment of 0134 + `TESTING.md`)

- **Rule.** A waiver on a gate is scoped (which figures), dated, owned, and **expires** on a date or a named merge. Without a renewal, the gate blocks again by itself.
- **The open-ended case.** `GATING = Informational` "until the optimization pass ends" had no expiry and no figure it still guarded.
- **Interim fix.** While the relative +5% stays off, the absolute structural counts (R2) and a "no worse than the profiled tip" ratchet on the crawl benchmark still gate.
- **Evidence:** Vaughan; Kadlec; Böckeler's agents raising thresholds; the 2026-10-22 to 2026-10-30 interval, during which the crawl went from unmeasured to 234× its ceiling with every gate quiet.

### R5. Every inventoried hot path has a whole-composition benchmark from the change that creates it (`TESTING.md`)

- **What it exercises.** The real composition with every default-on feature: Foxhound's crawl through `composition_with`, and Collie's click through the real driver against the in-memory CDP end.
- **What it must not use.** A stub such as `Scope::default()` or `engine.info` alone.
- **Strand E's examples of what today's benchmarks miss:**
  - the dispatch ceiling is measured only on `engine.info`;
  - the Frontier benchmark builds its Scope once, so it never sees `Scope::read` per batch;
  - the Audit log, ledger, governor, recycle check and Run-trace have no benchmark at all.
- **What the M1 timeline shows:** the periodic `just perf` measured startup and dispatch, and the first crawl benchmark arrived nine days after ADR 0134.
- **Evidence:** fitness functions must be **holistic** to see this class (Ford et al.); the walking skeleton carries the gate (section 6).

### R6. A general `STYLE.md` rule under "Memory and hot paths" (standard + lints)

- **Rule.** Generalise ADR 0138's page rule: *"Code on a hot path opens no file, connection or configuration, compiles no selector, regex or validator, and prepares no statement; work that depends only on the engine's, a Run's, a Session's or a document's inputs is built once at that scope and passed in."*
- **Lints.** Extend `disallowed-methods`:
  - `rusqlite::Connection::execute`/`query_row` outside stores (they re-prepare, strand E S4);
  - `Regex::new` outside a `LazyLock` or a built-once constructor;
  - `std::fs::create_dir_all`/`OpenOptions::open` outside the trace writer and store openers.
- **Review.** `code-review`'s Standards axis applies the rule as written. That is ADR 0133's model for review standards.
- **Evidence:** strand E's per-crate counts (97 `.prepare(` against 47 `prepare_cached`; 161 `.execute(` and 119 `query_row(`); constant selectors rebuilt per document).

### R7. A content standard for specs on a hot path (`docs/agents/` or ADR; no new step)

- **What a spec states.** A spec whose feature runs on an inventoried hot path says, in its Implementation Decisions, which path, which owner it plugs into, what it adds per unit in the counted figures, and which benchmark shows it.
- **Precedents:** a quality-attribute scenario in Bass, Clements and Kazman's form; TIGER_STYLE's back-of-the-envelope sketch.
- **New path.** Its first ticket is the owner module with its benchmark and ceiling, as prefactoring, which `to-tickets` already puts first.
- **Why it adds no step:** `to-spec` already lists "Architectural decisions" among Implementation Decisions and says to "respect any ADRs in the area". R1's ADR makes the content required, and the skills run as written.
- **Evidence:** QAW ("early"); SAFe NFRs "revisited as part of the definition of done"; Spec Kit's Technical Context, the closest tool precedent, which fails because its fields are optional and per-feature.

### R8. Owner skill usage: when to run `improve-codebase-architecture` and `diagnosing-bugs` (skill usage; owner's choice)

- **`improve-codebase-architecture`** is user-run (`disable-model-invocation: true`). It accepts a direction ("If the user named a direction … take it").
  - **When:** before the first spec of a new area, and whenever the owner reviews the code between `/implement-spec` runs.
  - **Direction:** "the hot paths in R1's inventory and any base work repeated across modules on them".
  - Its deepening vocabulary covers the fix: deep owner modules.
  - Note: it is aimed at testability and AI-navigability, so performance has to be named in the direction.
- **`diagnosing-bugs`** has a performance branch ("establish a baseline measurement … then bisect") for any R3 breach.
- This is the owner's own use of skills, not a repository step.

### R9. Competitor ratio as a counted, blocking figure (gate; ADR 0134 amendment; owner decision)

- **What it counts.** Spider can run under Callgrind in the same job, so the instruction ratio per page (Foxhound against Spider, scenario 1 of the matrix) is a **counted** figure. ADR 0071's rule against gating on wall time does not apply to it.
- **The ceiling.** Set it after #1577 from the tuned figure. Today's ratio is 32×.
- **Wall-time cells.** The matrix's wall-time cells stay Performance targets.
- **Evidence:** Chromium's competitive-benchmark revert policy; ClickBench; Duffy.

### R10. Upstream suggestions for Matt Pocock's skills (suggestions only; never filed by an agent)

These are written for the owner to share or not.

1. **`to-spec`:** an optional template section, "Shared paths and qualities". For each shared path the feature runs on: its owner, the cost added per unit, and the measure that proves it. Or a line in Implementation Decisions saying that quality-attribute scenarios belong there.
2. **`to-tickets`:** a vertical-slice rule: "a slice that runs on a shared hot path plugs into the module that owns it; if none exists, creating that owner (and its measurement) is prefactoring and comes first."
3. **`code-review`:**
   - widen Duplicated Code beyond "the change": "the change redoes base work (a parse, an I/O, a commit) that the same path already does elsewhere";
   - tell the Standards sub-agent to look for any documented performance budgets or hot-path rules.
4. **`implement-spec`:** at step 7, also run the repository's own whole-system checks (benchmarks, fitness functions) on the integration branch, and report them beside the review.
5. **`improve-codebase-architecture`:** add "the same base work repeated on one path by several modules" as a friction signal. The deletion test already favours a deep owner.
6. **`codebase-design`:** its interface definition already includes "performance characteristics". A line that a shared path's total cost is an interface property of the owning module would connect it to the above.

### Checks for the rest of Foxhound and for Collie

From `strands/e-codebase-scan.md` (confirmed by reading the call chains, not measured):

| Product | Path | What to check first |
|---|---|---|
| Collie and browser tiers | Per CDP command | `trace/cdp-commands.jsonl`: mkdir, open, write and close per drained batch, always on, with no setting (`goodfetch-page/src/driver.rs:312`, `trace.rs:61-104`) |
| Collie | Per page read | A new `Page.createIsolatedWorld` per read (2 round trips; a world may pile up per read, suspected); the 9.6 KB `check.js` resent per check (`driver.rs:1431`, `guard.rs:217`) |
| Collie | Per navigation | `WEAKLY_CHECKED` Provenance append; Page-check globs compiled per judgment (`recovery.rs:176-243`) |
| Collie | Per new pooled page | A process-tree walk per page, while the governor already holds the figure (`pool.rs:581`) |
| Collie | Per blocked subresource | A `requests.jsonl` append per blocked request (tens per ad-heavy page) (`run_trace.rs:139`) |
| Engine | Per Audit entry | Both TOML files re-read, two connections with schema set-up, a full fsync and the head file rewritten (`audit.rs:276-677`) |
| Engine | Per charge (per HTTP request) | Limits re-read with an uncached prepare; other calls open a connection and run the schema transaction (`ledger.rs`, `charging.rs:332`) |
| Engine | Per governor sample (1 s) | Both TOML files re-read, `/proc/meminfo` read three times, browser trees walked twice (`governor.rs:301`) |
| Engine | Per JSON-RPC message | About 5 walks and one deep copy per message; answers encoded twice (`rpc/framing.rs`, `composition.rs`) |
| Foxhound | Per links batch, enqueue, redirect hop | `Scope::read` re-parses its globs and recompiles regexes on the writer; Payload rules and Fragments likewise (`frontier/scope.rs:385`) |
| Foxhound | Per enqueued payload | A second JSON Schema validator per payload (`frontier/payload.rs:328`) |
| Foxhound | Per served page with a Page check or fields | Up to 3 more parses and per-page selector and XPath compiles (`fetch-tier/expected_page.rs`, `document.rs:214`, `tiers/calls.rs:410`) |

---

## 8. Decisions only the owner can make

1. **Generalise now or after #1577?** Recommended: write R1 as its own ADR now, before the SDK and MCP specs add work on the same engine paths. ADR 0138 stays the #1577 spec for the crawl path; ADR 0139 is reserved for continuous crawling.
2. **Gate a ratchet before #1577's pull request sets `GATING` back to `Enforced`?** Until then every perf figure is informational, so another spec's `/implement-spec` run can add per-page work unseen.
   - **Recommended:** a "no worse than the profiled tip" ratchet on the crawl benchmark and the counts that exist, from now. The +5% relative gate stays off until the pass, as decided.
   - It is a gate change, so only the owner decides it. It is not urgent: it matters only if another spec touches the crawl path before #1577.
3. **Expiry rule for waivers (R4):** adopt it as a `TESTING.md` rule and an ADR 0134 amendment? Recommended yes.
4. **Competitor instruction ratio as a blocking ceiling (R9)?** Recommended yes, on scenario 1, set from the tuned build.
5. **Run `improve-codebase-architecture` between `/implement-spec` runs,** directed at the hot paths (R8)? Recommended yes. It is the owner's skill to run.
6. **Share the upstream suggestions (R10) with Matt Pocock?** Only the owner can. Recommended: share points 1 to 3, which are small and tool-neutral.

---

## 9. Open questions

1. **How much do strand E's paths cost?** Nothing there was measured. A profile of a Collie scripted session and of a proxy crawl would rank them. That is an owner-approved run on the Hetzner server while no gate is running.
2. **Does `Page.createIsolatedWorld` with the same `worldName` create a new context each time** in current Chrome (strand E S1)? This should be checked against the Chromium source.
3. **How strict should the cost contracts be?** Can every hot path's contract be zero opens and zero compiles per unit in steady state, or do some features (Provenance on each navigation) need a per-unit allowance? R1's ADR has to decide that per path.
4. **Which counts belong in ADR 0050's ops listener as user-visible metrics,** and which only in tests?
5. **Do spec content standards (R7) work with agents?** The evidence that template sections help is weak, and SlopCodeBench suggests that guidance changes the start, not the slope. The gates (R2 to R5) carry the weight; R7 makes the owner visible to the planner.
6. **Will specs implemented before ADR 0138 lands add per-page work?** Recommendation 2 of section 8 would answer that with a measurement.
7. **The published evidence has gaps** (section 3.7). In particular, no study measures NFR ownership in agent-written plans. Goodfetch's timeline is a candidate case study once publishing is allowed.

---

## 10. Limits of this report

- **Verification depth.** The web strands read primary sources. Several are marked "abstract only" or UNVERIFIED in their strand files, and the cited numbers come only from sources read in full or in the abstract. `strands/v-verification.md` re-checked 18 load-bearing claims independently.
- **Vendor and practitioner sources.** Vendor reports (GitClear, CodeRabbit, Veracode, OX) have conflicts of interest and unpublished methods. They are used only as corroboration. Practitioner reports are anecdotal.
- **Strand E is unmeasured.** It is a grep-and-read scan: no language-server index was used, because the machine was running a merge gate. Its findings are confirmed in code and unmeasured in cost.
- **No experiment.** Nothing was built, run or benchmarked for this report.

---

## 11. Existing skills that fill parts of the gap, and where they fit in the Pocock flow

Added 2026-10-31 at the owner's request. The full survey is in `strands/f-skills-scout.md`. Each skill file was read in its own repository; the top candidate was re-read by this session.

**Verdict.** No published skill designs a shared hot path, owns quality attributes across specs, or reviews a change against the whole system for performance. The hot path's design stays a grilling topic that ends in an ADR (R1). Five published or installed skills each cover one piece without replacing a Pocock step:

| Need | Skill | Source, version seen | How it plugs in | Adds a step? |
|---|---|---|---|---|
| A register of quality limits, each with a number and a command, with ratchets and dated exceptions | `constraint-driven-development` (Addy Osmani) | github.com/addyosmani/agent-skills, `8bfcc1f`, 2026-10-31, MIT | Run once. It writes `CONSTRAINTS.md` (tables: enforced "Dimension, Rule, Checked by, Runs at"; measured "Metric, Today, Direction"; exceptions "ID, Rule, Path, Reason, Owner, Expires") and adds "Read CONSTRAINTS.md before writing code. Do not weaken it to make a change pass." to `AGENTS.md`. Pocock's `code-review` Standards axis reads every standards file it finds, so it reads this one. "Tightening the bar should be silent; loosening it should be loud." | No: a one-time setup, then a standard. Its web defaults (LCP, CLS) are replaced by the engine's counters. It implements R2's counts, R3's ratchet and R4's expiring waivers in one file. |
| Rules for counter ceilings | `protect` and `goal` (melodic-software `performance` plugin) | github.com/melodic-software/claude-code-plugins, `3eeb073`, 2026-10-31, MIT, low adoption | Read as reference for how ceilings are written: "Counters only, never durations"; "The ceiling is the measured value, never a round number above it"; raising one is "a decision, stated with its reason in the PR body". | No, if only its rules are used. Its `climb` runs its own implementation loop: do not use it. |
| A quality-attribute checklist for a grilling topic | `engineering:system-design` and `engineering:architecture` (Anthropic knowledge-work `engineering` plugin) | github.com/anthropics/knowledge-work-plugins, `c71b6f3`, 2026-09-21, Apache-2.0; **already available in the owner's Claude app** | When a grilling (`grill-with-docs`) is about a shared path or a new area, its list ("Non-functional requirements (scale, latency, availability, cost)", "Load estimation", "Every decision has trade-offs. Make them explicit.", "what you'd revisit") is a topic checklist. The ADR itself stays `domain-modeling`'s. | No: reference text |
| A large architecture effort split into decision tickets | `wayfinder` (Matt Pocock) | mattpocock/skills `a496f55`; installed locally | **Already used, and it did not prevent the failure** (section 5). Wayfinder is a container with no quality-attribute content: it charts the questions it is given. It helps R1 only if the map's destination and its tickets name the shared paths and their per-unit cost as decisions to take ("what does a page go through, and what may that cost?"), and if those decisions end in counted, blocking limits. | No: a Pocock skill, run by the owner |
| Turning this failure into checks | `retro` (Matt Pocock) | installed locally | Its "automated checks" category turns mechanical rules into deterministic checks. Run on this incident, it yields the lints and counts of R2 and R6. | No: owner-run |
| Whole-design adversarial review | `architecture-critic` agent (Anthropic `code-modernization` plugin) | github.com/anthropics/claude-plugins-official, `c8874f5`, 2026-09-25 | Optional, owner-run, on a spec before `/to-tickets` or on the system between runs: "Which non-functional requirements (latency, throughput, consistency) are unstated"; "Trace one failure mode end-to-end." | Yes if made routine; as an owner's occasional choice, no |
| Whole-repository security review | `security-threat-model` (OpenAI skills, `SKILL.md` format) | github.com/openai/skills, added `5c8f1e2`, 2026-02-02 | Between `/implement-spec` runs, or before a spec on an exposed surface (MCP, Gateway, Sinks) | Owner-run |

**Not to adopt:** the following each bring a planning pipeline that competes with grilling, `to-spec`, `to-tickets` and `implement-spec` (strand F):
- obra/superpowers (brainstorming, writing-plans);
- Addy Osmani's `spec-driven-development`;
- the Spec Kit and BMAD ports;
- melodic's `climb`.

wshobson's and VoltAgent's architect and performance agents list "fitness functions" and "budgets" as expertise, but give no procedure that binds anything.

**Directory listings that did not check out:** several community "fitness function", "quality attributes" or "NFR ADR" skills are listed but missing or unconfirmed at their source:
- Xopoko and reiserwang return 404;
- jabrena's skill and 9uiLe's are not at the paths listed;
- melodic's `systems-design` is gone from `main`.

**How it fits together**, step by step through the Pocock flow:
1. **Grilling (`grill-with-docs`).** A shared path is grilled as its own topic, with `engineering:system-design`'s checklist as reference. The result is an ADR (R1). If the effort goes through `wayfinder`, the map's destination must name the shared paths and their cost as decisions; otherwise it repeats map #61's gap.
2. **The register.** The counted limits of that ADR go into one register, `CONSTRAINTS.md`, made once with `constraint-driven-development`, or the same three tables placed in `TESTING.md`. `just merge` runs its commands.
3. **`/to-spec` and `/to-tickets`.** They respect the ADR ("respect any ADRs in the area").
4. **Implementers** read the register through `AGENTS.md`.
5. **`code-review`** (step 7) reads it as a standard.
6. **Between runs, as the owner chooses:** `improve-codebase-architecture` with a hot-path direction, `architecture-critic`, `security-threat-model`, and `retro` after an incident.

**Owner decision:** one register file (`CONSTRAINTS.md`) or the same tables inside `TESTING.md`? Recommended: `TESTING.md`. It already holds the Performance budgets, so a second file would split one standard in two. Take Addy's three-table shape and "loosening is loud" rule into it, and point `AGENTS.md` at it with his line.
