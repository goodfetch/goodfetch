# Sources

Every source behind `report.md`, with what it supports. Web sources were all accessed on 2026-10-31.

The status column says how far each source was read:
- **V**: verified at the primary source, in full text or in the sections that hold the cited figures.
- **VA**: verified from the abstract or landing page only.
- **P**: partly verified.
- **U**: unverified, seen only in secondary sources.
- **COI**: the vendor sells a related product.
- **✓**: independently re-checked in `strands/v-verification.md`.

Full notes, with quotes, are in the strand files: `strands/a-evidence.md`, `b-concepts.md`, `c-harnesses.md`, `d-practices.md`, `e-codebase-scan.md` and `v-verification.md`.

## Local sources (Goodfetch)

| Source | Supports |
|---|---|
| `.claude/handoff/m1/reports/profile-2026-10-30/report.md` (tip `0fd15a3a7`) | The per-page costs: 48.9 % clean-up, 7.5 fsyncs, 142 KB WAL, 135 prepares, 8 parses, 32× Spider's instructions, 329 syscalls against 12. |
| Issue #1577 and comments 6400933727, 6534699879, 6534856008, 6535125517, 6535428265, 6536582421 | The optimization pass as tuning, the like-for-like matrix, the figures of 80 against 11,145 and 337 against 12,983 pages/s, and the first `compare-spider` figure of 31.6 against 4,981. |
| `docs/adr/0023-*.md` | Costly controls are switchable and reported. |
| `docs/adr/0071-*.md` and its 2026-10-22 amendment | The per-change +5 % budget against the merge base, and its informational switch. |
| `docs/adr/0133-*.md` | "The repository adds standards, not process steps." |
| `docs/adr/0134-*.md` and its 2026-10-22 amendments | The absolute ceilings, "accepted steps under 5% add up unseen", the optimization pass, and `GATING` informational. |
| Draft ADR 0138 (now committed on its own branch, `54e259491`, worktree `.claude/worktrees/docs-adr-0138/`); `.claude/reports/grilling-1577-per-page-cost/notes.md` | The planned structural fix: Page analysis, demand plan, Durability levels, 7 counts, lints, and step-7 items. |
| `.claude/handoff/m1/perf-gate-orchestrator-notes.md`, `reports/perf-checkpoint-3.md`, `reports/perf-bisect-100.md`, `post-batch.sh` | The periodic `just perf` measured startup and dispatch, and noted waste it did not fix (`Scope::widen`). |
| `.claude/handoff/m1/reports/fix-perf-informational-m1.md` | The `GATING = Informational` change of 2026-10-22. |
| `.claude/handoff/m1/reports/t1572.md` | The first whole-crawl benchmark, 2026-10-27: 234.45× against the 2× ceiling, "noted only". |
| `STYLE.md` (Design; Memory and hot paths), `TESTING.md` (Performance budgets) | The current hot-path rules cover bytes, channels and String clones, not parses, commits or prepares. |
| `~/.agents/skills/{to-spec,to-tickets,implement-spec,code-review,improve-codebase-architecture,codebase-design,grilling,tdd,diagnosing-bugs}/SKILL.md` (commit `62a4b41`, 2026-10-23) | No performance, NFR or budget section; Duplicated Code is limited to "the change"; `improve-codebase-architecture` is user-run and aimed at testability; `diagnosing-bugs` has a performance branch. |
| `strands/e-codebase-scan.md` (code at `203f17f42`) | The 17 confirmed and 5 suspected per-unit cases beyond the crawl, and the gaps in benchmarks and ceilings. |

## Evidence: agent-built software (question 1)

| Source | Status | Supports |
|---|---|---|
| Orlanski et al., SlopCodeBench, arXiv:2603.24755 — https://arxiv.org/abs/2603.24755 | V ✓ | Erosion rises in 77 % and verbosity in 75.5 % of trajectories. Agent code is 2.3× more verbose and 2.0× more eroded than human code, and degrades 5–7× faster per step. Quality prompts change the start, not the rate. Cost per checkpoint grows 2.2×. |
| He, Miller, Agarwal, Kästner, Vasilescu, arXiv:2511.04427 (MSR 2026) — https://arxiv.org/abs/2511.04427 | V ✓ | The speed gain from Cursor fades after about 2 months. Warnings stay +30.3 % and complexity +41.6 %. Duplication is +7 %, not significant. |
| Deng et al., SWE-Milestone (v1 "EvoClaw"), arXiv:2603.13428 — https://arxiv.org/abs/2603.13428 | V ✓ | Scores fall from over 80 % on isolated tasks to at most 38.03 % in continuous evolution. Recall grows while precision saturates. |
| Shastry et al., SWE-STEPS, arXiv:2604.03035 — https://arxiv.org/abs/2604.03035 | V | Success falls 15–25 points once state carries over; spillover from earlier code is the main failure. |
| Chen et al., SWE-CI, arXiv:2603.03823 — https://arxiv.org/abs/2603.03823 | V ✓ | All 20 models score worse on the Maintainability Index on solved problems; 15 of 20 do better on Pylint. |
| He X. et al., SWE-Perf, arXiv:2507.12415 — https://arxiv.org/abs/2507.12415 | V | Agents gain 2.26 % against 10.85 % for experts on repository-level performance. |
| Ma et al., SWE-fficiency, arXiv:2511.06090 — https://arxiv.org/abs/2511.06090 | VA | Agents reach less than 0.23× the expert speedup; they struggle to reason across functions. |
| Peng et al., arXiv:2512.21757 — https://arxiv.org/abs/2512.21757 | VA ✓ | Performance is validated in 45.7 % of agent performance PRs against 63.6 % of human ones. |
| Opu et al., arXiv:2512.24630 — https://arxiv.org/abs/2512.24630 | VA (numbers U) | Topics of agent performance PRs (adjacent). |
| Huang D. et al., EffiBench, arXiv:2402.02037 — https://arxiv.org/abs/2402.02037 | VA | Correct LLM code takes 3.12× the time; worst cases are 13.89× the time and 43.92× the memory. |
| Du et al., Mercury, arXiv:2402.07844 — https://arxiv.org/abs/2402.07844 | VA | Pass rate is above the efficiency-weighted "Beyond" score. |
| Waghjale et al., ECCO, arXiv:2407.14044 — https://arxiv.org/abs/2407.14044 | VA | Trade-off between efficiency and correctness. |
| Rajput et al., arXiv:2601.01215 — https://arxiv.org/abs/2601.01215 | VA | Correct solutions to the same task differ widely in memory use. |
| Liu et al., "Debt Behind the AI Boom", arXiv:2603.28592 — https://arxiv.org/abs/2603.28592 | V | AI commits introduce about 1.5× more security issues than they fix; 22.7 % of introduced issues survive to HEAD. |
| Shukla et al., arXiv:2506.11022 — https://arxiv.org/abs/2506.11022 | V (with caveats) | Iterations aimed at efficiency add memory-safety issues: optimising one quality degrades another. |
| Veracode 2025 GenAI Code Security Report (press release) — https://www.veracode.com/press-release/ai-generated-code-poses-major-security-risks-in-nearly-half-of-all-development-tasks-veracode-research-reveals/ | VA, COI | The insecure option is chosen in 45 % of tasks. |
| Palmer, Statement on CVE-2025-48757 — https://mattpalmer.io/posts/statement-on-CVE-2025-48757/ | V, COI | Case: 10.3 % of scanned Lovable apps exposed data through Row Level Security gaps. |
| The Register on Replit, 2025-07-21 — https://www.theregister.com/software/2025/07/21/vibe-coding-service-replit-deleted-production-database/719783 | U | Case, press only: an agent deleted a production database. |
| Carlini, "Building a C compiler with a team of parallel Claudes" — https://www.anthropic.com/engineering/building-c-compiler | V | Case: correctness had a near-perfect verifier, performance none; the output is slower than GCC `-O0`. |
| Huang H. et al., "More Code, Less Reuse", arXiv:2601.21276 — https://arxiv.org/abs/2601.21276 | V | Agent PRs show about 1.87× the semantic redundancy of human PRs. |
| GitClear 2024 — https://www.gitclear.com/coding_on_copilot_data_shows_ais_downward_pressure_on_code_quality ; 2025 — https://www.gitclear.com/ai_assistant_code_quality_2025_research | VA, COI | Copy-pasted lines go from 8.3 % to 12.3 %; moved lines from 25 % to under 10 %. No method is published. |
| CodeRabbit, State of AI vs Human Code Generation — https://www.coderabbit.ai/blog/state-of-ai-vs-human-code-generation-report | VA, COI | "Excessive I/O ~8×" in AI PRs (470 PRs). |
| Ehsani et al., arXiv:2601.15195 — https://arxiv.org/abs/2601.15195 | VA | Agent PRs are rejected as duplicates or for unwanted features. |
| Brahmi et al., MSR 2026 Mining Challenge — https://2026.msrconf.org/details/msr-2026-mining-challenge/28/ | VA | Agents' self-admitted debt names the problem without a resolution. |
| Mujahid and Imran, arXiv:2601.07786 — https://arxiv.org/abs/2601.07786 | VA | Self-admitted debt in comments about AI-written code. |
| Horikawa et al., Agentic Refactoring, arXiv:2511.04824 — https://arxiv.org/abs/2511.04824 | VA | Agents refactor in 26 % of commits, mostly at a low level. |
| Shypula et al., PIE, arXiv:2302.07867 — https://arxiv.org/abs/2302.07867 | VA | Counter-evidence: optimisation at function level works when measured well. |
| Aleithan et al., SWE-Bench+, arXiv:2410.06992 — https://arxiv.org/abs/2410.06992 | VA | 31 % of "resolved" patches pass only because of weak tests. |
| Wang, Pradel, Liu, arXiv:2503.15223 — https://arxiv.org/abs/2503.15223 | VA | 29.6 % of plausible patches behave differently from the ground truth. |
| METR (Rein), Aug 2025 — https://metr.org/blog/2025-08-12-research-update-towards-reconciling-slowdown-with-time-horizons/ | V | 0 of 15 agent PRs are mergeable as they stand. |
| METR (Whitfill et al.), 2026-03-10 — https://metr.org/notes/2026-03-10-many-swe-bench-passing-prs-would-not-be-merged-into-main/ | V ✓ | Maintainers merge about 24 points fewer patches than the automated grader passes. |
| Zhong, Raghunathan, Carlini, ImpossibleBench, arXiv:2510.20270 — https://arxiv.org/abs/2510.20270 | V | Agents game the per-change guard by modifying tests. |
| Daly et al., MongoDB change-point detection, arXiv:2003.00584 — https://arxiv.org/abs/2003.00584 | VA | Threshold detection is "not sufficient" and misses changes below the threshold (human-written code). |
| Becker et al., METR RCT, arXiv:2507.09089 — https://arxiv.org/abs/2507.09089 | VA | Adjacent: developers were 19 % slower while feeling faster. |
| DORA 2024 — https://cloud.google.com/blog/products/devops-sre/announcing-the-2024-dora-report ; DORA 2025 — https://cloud.google.com/blog/products/ai-machine-learning/announcing-the-2025-dora-report | VA | Adjacent: AI adoption goes with lower delivery stability; "amplifies what's already there". |
| Borg et al., Echoes of AI, arXiv:2507.00788 — https://arxiv.org/abs/2507.00788 | VA | Counter-evidence: no maintainability penalty in a supervised setting. |
| Watanabe et al., arXiv:2509.14745 — https://arxiv.org/abs/2509.14745 | VA | 45 % of merged Claude Code PRs needed follow-up changes. |
| Perry et al., arXiv:2211.03622 ; Pearce et al., arXiv:2108.09293 | VA | Adjacent security evidence. |
| OX Security, Army of Juniors — https://www.ox.security/army-of-juniors | VA, COI | Adjacent; per-pattern figures U. |
| Böckeler, Maintainability sensors (2026) — https://martinfowler.com/articles/sensors-for-coding-agents.html | V | Duplicated routes and hooks, one change touching 40+ files, agents raising thresholds, debt compounding. |
| Böckeler, Pushing AI autonomy (2025) — https://martinfowler.com/articles/pushing-ai-autonomy.html | V | Brute-force fixes, false claims of success, unrequested features. |
| Böckeler, Understanding SDD (2025) — https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html | V | Spec Kit regenerated existing classes as duplicates. |
| Beck, Augmented Coding (2025) — https://newsletter.kentbeck.com/p/augmented-coding-beyond-the-vibes | V | Accumulated complexity stalled the agent. |
| Ronacher, The Coming Loop (2026) — https://lucumr.pocoo.org/2026/6/23/the-coming-loop/ | V | Agents "duplicate code", code "too local in its reasoning". |
| Lin (Cursor), Scaling long-running autonomous coding (2026) — https://cursor.com/blog/scaling-agents ; Willison (2026) — https://simonwillison.net/2026/Jan/19/scaling-long-running-autonomous-coding/ | V | "Agents conflict, duplicate work, and drift"; periodic fresh starts. |
| Young (Anthropic), Effective harnesses for long-running agents — https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents | V | The harness guards functional regressions only, with no NFR mention. |

## Concepts (question 2)

| Source | Status | Supports |
|---|---|---|
| Perry and Wolf 1992 — https://webhome.csc.uvic.ca/~hausi/480/papers/perry.pdf | V | Architectural drift against erosion: the best single term. |
| Foote and Yoder, Big Ball of Mud, 1997 — http://www.laputan.org/mud/mud.html | V | Piecemeal growth; "focus first on features … then … performance"; Beck's "make it work, right, fast" as quoted there. |
| Kiczales et al., AOP, ECOOP 1997 — https://www.cs.ubc.ca/~gregor/papers/kiczales-ECOOP1997-AOP.pdf | V ✓ | Cross-cutting aspects; image filters each looping on their own give "terrible performance". |
| Parnas 1972, via Colyer — https://blog.acolyer.org/2016/09/05/on-the-criteria-to-be-used-in-decomposing-systems-into-modules/ ; 1978, via Cunningham — https://john.cs.olemiss.edu/~hcc/researchMethods/notes/ClassicParnas/ACMannotated/ClassicParnasRevisionAnnotated.pdf | V (reproductions) | Decomposition by processing step duplicates shared decisions; a module is "a work assignment". |
| Conway 1968 — https://www.melconway.com/Home/Committees_Paper.html | V | The ticket graph is the organisation. |
| Thoughtworks Radar, Inverse Conway Maneuver — https://www.thoughtworks.com/radar/techniques/inverse-conway-maneuver | V | Shape the ticket graph like the target architecture. |
| Brooks, The Mythical Man-Month (archive.org) — https://archive.org/stream/MythicalManMonth/ | V | Conceptual integrity, "one mind"; the second-system effect does not fit this case. |
| Kazman, Klein, Clements, ATAM, SEI TR-004 — https://www.sei.cmu.edu/library/atam-method-for-architecture-evaluation/ | V | Risk as an "architecturally important decision" not made; sensitivity points; utility tree. |
| Barbacci et al., QAW, SEI TR-016 — https://www.sei.cmu.edu/library/quality-attribute-workshops-qaws-third-edition/ | VA | Quality drivers are found early. |
| Bass, Clements, Kazman, §4.3 (2nd ed.) — https://people.ece.ubc.ca/matei/EECE417/BASS/ch04lev1sec3.html | V | The six-part quality-attribute scenario. |
| Chen, Ali Babar, Nuseibeh, ASRs, IEEE Software 2013 — https://eprints.lancs.ac.uk/id/eprint/64499 | VA | Architecturally significant requirements. |
| ISO/IEC 25010 reproduction — https://iso25000.com/en/iso-25000-standards/iso-25010 | V (reproduction) | Performance efficiency as a product-level characteristic. |
| Thoughtworks Radar, Architectural fitness function — https://www.thoughtworks.com/radar/techniques/architectural-fitness-function ; Ford et al., Building Evolutionary Architectures — https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/ | V / U (taxonomy) | Fitness functions; holistic against atomic. |
| SAFe: Architectural Runway — https://framework.scaledagile.com/architectural-runway ; Enablers — https://framework.scaledagile.com/enablers ; NFRs — https://framework.scaledagile.com/nonfunctional-requirements | V | Runway, enablers, emergent design against intentional design, NFRs in the definition of done. |
| Kadlec, Performance Budgets That Stick (2019) — https://timkadlec.com/remembers/2019/03/performance-budgets-that-stick/ | V | A budget is "enforced … a hard line". |
| Mihajlija, Performance budgets 101 — https://web.dev/articles/performance-budgets-101 | V | Absolute limits; a new feature must pay for itself. |
| Knuth 1974 — https://gwern.net/doc/cs/algorithm/1974-knuth.pdf | V | The full "premature optimization" passage, with "that critical 3%". |
| Duffy, The "premature optimization is evil" myth (2010) — http://joeduffyblog.com/2010/09/06/the-premature-optimization-is-evil-myth/ | V | "Peanut butter" problems. |
| Rousskov, squid-dev, Death by a thousand cuts (2010) — https://list-archive.squid-cache.org//mail-archive/squid-dev/201011/0122.html | V | Changes each negligible add up to 3–10 %. |
| Hardin 1968 — https://www.garretthardinsociety.org/articles/art_tragedy_of_the_commons.html | V | The tragedy of the commons on a shared per-page budget. |
| Team Topologies, key concepts — https://teamtopologies.com/key-concepts ; InfoQ 2019 — https://www.infoq.com/news/2019/07/team-topologies-cognitive-loa/ | V | Platform and enabling teams; cognitive load. |
| Vaughan, The Challenger Launch Decision — https://press.uchicago.edu/ucp/books/book/chicago/C/bo22781921.html | VA | Normalization of deviance. |
| Strathern 1997 — https://www.cambridge.org/core/journals/european-review/article/improving-ratings-audit-in-the-british-university-system/FC2EE640C0C44E3DB87C29FB666E9AAB | VA | Goodhart; the popular wording is U. |
| Betterer — https://phenomnomnominal.github.io/betterer/docs/introduction | V | Ratchet. |
| Lehman 1996 — https://www.cs.kent.edu/~jmaletic/cs63902/Papers/Lehman96.pdf | V | Increasing complexity and declining quality "unless work is done". |
| Booch, The Accidental Architecture (2006) — https://computer.org/publications/tech-news/on-architecture/the-accidental-architecture | P | Accidental architecture (wording U). |
| Wirfs-Brock 2011 (Poppendieck's last responsible moment) — https://wirfs-brock.com/rebecca/blog/2011/01/18/agile-architecture-myths-2-architecture-decisions-should-be-made-at-the-last-responsible-moment/ | V (reproduction) | The last responsible moment and its critique. |
| Smith, SPE Then and Now (WOSP-C 2015) — https://research.spec.org/icpe_proceedings/2015/wospc/p1.pdf | V ✓ | Fix-it-later and its premises. |
| Cockburn bio — https://alistaircockburn.com/Bio ; Venners, Tracer Bullets and Prototypes — https://www.artima.com/articles/tracer-bullets-and-prototypes | V / U (Crystal Clear wording) | Walking skeleton; tracer bullets. |

## Tools and harnesses (question 3)

| Source | Status | Supports |
|---|---|---|
| GitHub Spec Kit templates at `7370e59` — https://github.com/github/spec-kit/tree/7370e592f3cd9c06e5b698983957e227c75b30e2/templates ; releases | V ✓ | Technical Context NFR fields that accept NEEDS CLARIFICATION; the Constitution Check gate; `/clarify`, `/analyze`, `/checklist`; a single-feature scope. |
| Kiro docs — https://kiro.dev/docs/specs/ , /specs/feature-specs/ , /steering/ , /hooks/types/ | V | No NFR section; Design-First mode; steering as context; task hooks. |
| BMAD-METHOD at `61e01f8` and v6.12.4 — https://github.com/bmad-code-org/BMAD-METHOD | V ✓ | Optional Cross-Cutting NFRs and Performance Budgets; spine invariants; readiness gate with no NFR check. |
| BMAD TEA — https://bmad-code-org.github.io/bmad-method-test-architecture-enterprise/how-to/workflows/run-nfr-assess | V | Optional `nfr-assess` at system scope, run at release. |
| Tessl SDD tile at `b8fdff7` — https://github.com/tesslio/spec-driven-development-tile | V | "Missing non-functional requirements" as a gap category during requirement gathering. |
| Matt Pocock skills at `a496f55` — https://github.com/mattpocock/skills | V ✓ | No NFR section in `to-spec`, `to-tickets`, `code-review` or `implement-spec`. |
| Claude Code docs — https://code.claude.com/docs/en/best-practices , /code-review-local , /code-review , /agent-teams ; https://claude.com/blog/automate-security-reviews-with-claude-code | V ✓ | `CLAUDE.md` is advisory; hooks are deterministic; review is diff-centred and never blocks. |
| Anthropic, Building effective agents — https://www.anthropic.com/engineering/building-effective-agents ; Multi-agent research system — https://www.anthropic.com/engineering/multi-agent-research-system | V | No system-wide quality step; "agents duplicate work" without detailed task descriptions. |
| OpenAI Codex docs — https://learn.chatgpt.com/docs/agent-configuration/agents-md , https://learn.chatgpt.com/docs/third-party/github | V | Review of the PR diff; AGENTS.md not enforced. |
| Cursor docs — https://cursor.com/docs/bugbot , /context/rules , /agent/plan-mode | V | Bugbot scope; rules as context. |
| Devin docs — https://docs.devin.ai/work-with-devin/devin-review.md , /product-guides/creating-playbooks | V | Devin Review scope; performance only as a REVIEW.md example. |
| Cognition, Don't build multi-agents — https://cognition.com/blog/dont-build-multi-agents | V ✓ | "Actions carry implicit decisions, and conflicting decisions carry bad results." |
| Aider docs — https://aider.chat/docs/usage/modes.html , /conventions.html , /lint-test.html | V | No NFR or cross-task step. |
| OpenHands — https://docs.openhands.dev/overview/skills ; https://openhands.dev/blog/openhands-product-update---march-2026 | V | Planning Mode writes PLAN.md; no NFR step. |
| SWE-agent — https://swe-agent.com/latest/ ; mini-swe-agent — https://mini-swe-agent.com/latest/ | V | Scoped to a single issue. |
| GitHub Copilot code review — https://docs.github.com/en/copilot/concepts/agents/code-review ; coding agent — https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent | V | Review categories; not blocking by default. |
| claude-task-master — https://github.com/eyaltoledano/claude-task-master | V | Research mode covers "scalability issues"; complexity is scored on effort only. |
| Agent OS — https://github.com/buildermethods/agent-os | V | Standards are injected, never checked. |
| Amp — https://ampcode.com/news/liberating-code-review | V | The `.agents/checks/perf.md` example. |
| CodeRabbit — https://docs.coderabbit.ai/pr-reviews/pre-merge-checks.md ; Greptile — https://www.greptile.com/docs/how-greptile-works/graph-based-codebase-context.md ; Qodo — https://docs.qodo.ai/code-review.md | V / P (CodeRabbit code graph) | Review bots: graph context, no performance gate. |
| MetaGPT — https://github.com/FoundationAgents/MetaGPT/blob/main/metagpt/actions/design_api_an.py | V | The architect role has no quality-attribute node. |

## Practices (question 4)

| Source | Status | Supports |
|---|---|---|
| rustc-perf triage README — https://github.com/rust-lang/rustc-perf/blob/master/triage/README.md ; comparison analysis — https://github.com/rust-lang/rustc-perf/blob/master/docs/comparison-analysis.md ; triage 2026-10-26 — https://raw.githubusercontent.com/rust-lang/rustc-perf/master/triage/2026/2026-10-26.md ; Kobzol 2025 — https://kobzol.github.io/rust/rustc/2025/06/09/why-doesnt-rust-care-more-about-compiler-performance.html | V | Per-merge comparison plus weekly net change and a dashboard across releases; not blocking. |
| Firefox performance sheriffing — https://firefox-source-docs.mozilla.org/testing/perfdocs/perf-sheriffing.html | V | Relative thresholds (2 %), a human rota, backout after the fact. |
| Chromium, Regressions to competitive benchmarks — https://chromium.googlesource.com/chromium/src/+/main/docs/benchmark_performance_regressions.md | V ✓ | Revert after one business day; 0.3 % threshold on Speedometer and JetStream. |
| SQLite, Measuring and reducing CPU usage — https://www.sqlite.org/cpu.html | V | Cachegrind repeats to 7 or more digits; a fixed workload across 61 releases; micro-optimisations add up. |
| Go wiki, PerformanceMonitoring — https://go.dev/wiki/PerformanceMonitoring | V ✓ | The baseline is the latest Go release: a fixed baseline. |
| Lighthouse CI configuration — https://github.com/GoogleChrome/lighthouse-ci/blob/main/docs/configuration.md | V | Absolute `error` assertions that block. |
| size-limit — https://github.com/ai/size-limit | V | An absolute limit that blocks, with the diff shown for information. |
| Gungraun regressions — https://gungraun.github.io/gungraun/latest/html/regressions.html | V | Hard limits; exit code 3. |
| CodSpeed docs and pricing — https://codspeed.io/docs/instruments/cpu , /docs/features/performance-checks , /pricing | V | CPU simulation; relative to `main`; admin acknowledgement (a process step); SaaS. |
| Bencher thresholds — https://bencher.dev/docs/explanation/thresholds/ ; pricing — https://bencher.dev/pricing/ | V | Statistical thresholds drift with their window; the `static` threshold is absolute; self-hosting is free. |
| criterion.rs — https://bheisler.github.io/criterion.rs/book/user_guide/command_line_options.html ; github-action-benchmark — https://github.com/benchmark-action/github-action-benchmark | V | Comparison with the previous result; 200 % by default, not failing (the pattern to avoid). |
| TechEmpower FrameworkBenchmarks — https://github.com/TechEmpower/FrameworkBenchmarks | V ✓ | Archived on 2026-03-24: no longer usable. |
| ClickBench — https://github.com/ClickHouse/ClickBench ; ClickHouse over the years — https://clickhouse.com/blog/clickhouse-over-the-years-with-benchmarks | V | Rules for a competitor benchmark; a fixed-workload comparison across releases found regressions. |
| Ren et al., Google-Wide Profiling — https://research.google/pubs/google-wide-profiling-a-continuous-profiling-infrastructure-for-data-centers/ | VA | Continuous profiling. |
| Kanev et al., Profiling a warehouse-scale computer (2015) — https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/44271.pdf | V | No "killer application"; the datacenter tax is about 30 %; flat profiles. |
| FBDetect, SOSP 2024 — https://tangchq74.github.io/FBDetect-SOSP24.pdf | V ✓ | Tiny regressions add up to about 4,000 servers (Meta's serverless platform). |
| Parca — https://www.parca.dev/docs/overview ; Pyroscope — https://grafana.com/docs/pyroscope/latest/introduction/ | V | Free, self-hosted profiling with diffs. |
| ArchUnit — https://www.archunit.org/ ; import-linter — https://import-linter.readthedocs.io/en/stable/ ; cargo-deny bans — https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html ; dhat-rs — https://docs.rs/dhat/latest/dhat/ | V | Structural fitness functions; allocation budgets per operation. |
| OpenAI, Harness engineering — https://openai.com/index/harness-engineering/ (403; copy read) | U | Linters with remediation text for agents; background clean-up agents. |
| Malte Ubl, Design Docs at Google — https://www.industrialempathy.com/posts/design-docs-at-google/ | V | A "cross-cutting concerns" section; performance is not named. |
| arc42 §10 — https://docs.arc42.org/section-10/ ; Volere — https://www.volere.org/templates/volere-requirements-specification-template/ | V | Quality scenarios with fit criteria. Volere is paid (declined). |
| Maranzano et al. 2005 ; arXiv:1305.2402 ; SEI ATAM retrospective — https://apps.dtic.mil/sti/tr/pdf/ADA636364.pdf | U / P | Architecture reviews; effectiveness figures not read. |
| TIGER_STYLE.md — https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md | V | "Think about performance from the outset"; back-of-the-envelope sketches. |
| Duffy, Performance Culture (2016) — https://joeduffyblog.com/2016/04/10/performance-culture/ | V | Gated performance tests on every commit; regressions reverted "without question"; "Layers of regressions pile on top of one another". |
| LMAX Disruptor — https://lmax-exchange.github.io/disruptor/disruptor.html ; Seastar — https://seastar.io/shared-nothing/ | V | Mechanical sympathy; design shaped by performance. |
| Anthropic, AI-resistant technical evaluations — https://www.anthropic.com/engineering/AI-resistant-technical-evaluations | V ✓ | A known target changed agent behaviour (pre-release Opus 4.5 checkpoint). |
| DeepMind, AlphaEvolve — https://deepmind.google/discover/blog/alphaevolve-a-gemini-powered-coding-agent-for-designing-advanced-algorithms/ | V | Automated evaluators; 0.7 % of Google's compute recovered. |
| Kevin-32B — https://cognition.com/blog/kevin-32b ; arXiv:2507.11948 | P | Reinforcement learning on speed and correctness. |
| Sakana AI CUDA Engineer coverage — https://techcrunch.com/?p=2969225 ; EvoEngineer, arXiv:2510.03760 | P | Counter-case: an evaluator was gamed. |
| GSO — https://gso-bench.github.io/ | P | Agents are weak at whole-repository optimisation. |

## Existing skills (report section 11)

| Source | Status | Supports |
|---|---|---|
| Addy Osmani, `constraint-driven-development` — https://github.com/addyosmani/agent-skills (`8bfcc1f`; skill file `skills/constraint-driven-development/SKILL.md`) | V (re-read by this session) | A quality register with three tables, ratchets, exceptions that expire after 90 days, "loosening should be loud", and the AGENTS.md line. |
| melodic-software `performance` plugin (`protect`, `goal`, `target`) — https://github.com/melodic-software/claude-code-plugins (`3eeb073`) | V | Counter ceilings only, never durations; a ceiling equals the measured value. |
| Anthropic knowledge-work `engineering` plugin (`system-design`, `architecture`) — https://github.com/anthropics/knowledge-work-plugins (`c71b6f3`) | V | An NFR checklist and an ADR template that states trade-offs. |
| Anthropic `code-modernization` `architecture-critic` — https://github.com/anthropics/claude-plugins-official (`c8874f5`) | V | An adversarial review of the design: unstated NFRs, one failure traced end to end. |
| OpenAI `security-threat-model` — https://github.com/openai/skills (`5c8f1e2`) | V | A security review of the whole repository. |
| Matt Pocock `wayfinder`, `retro`, `ask-matt`, `in-progress/*` — https://github.com/mattpocock/skills (`a496f55`) | V | No quality-attribute content; `wayfinder` holds decision tickets; `retro` turns rules into checks. |
| obra/superpowers (`2ce8aa1`); wshobson/agents (`7e2fb55`); VoltAgent/awesome-claude-code-subagents (`f2e2ea7`); trailofbits/skills (`2be6433`); vercel-labs/agent-skills | V | Competing pipelines or persona agents; security-only or framework-specific skills. |
| Directory listings: 9uiLe `quality-architecture`, Xopoko `architecture-fitness-functions`, reiserwang `architecture-design`, jabrena `032-…`, melodic `systems-design` | U / missing | Listed in directories; missing or unconfirmed at their source. |
