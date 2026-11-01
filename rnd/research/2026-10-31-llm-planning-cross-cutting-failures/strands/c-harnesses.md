# Strand C: how coding-agent workflows and harnesses handle cross-cutting qualities

Access date for every source: 2026-10-31. Method: WebSearch and WebFetch only, on official docs, official blogs and repository files at the named commit or release. Each claim is marked **VERIFIED** (read in a primary source this session) or **UNVERIFIED** (secondary source, inference, or not read). Quotes are short excerpts from the fetched page.

The failure under study: every task is correct and reviewed against its own spec, but nothing owns system-wide qualities (performance, memory, durability, security, cost), so the system degrades as a whole. For example, every feature re-parses the same data, or adds its own database commit on a hot path.

## Summary table

| Tool | Version/date seen | NFR step/template/gate (name) | Automatic or optional | Cross-spec/whole-system view? | Persistent standards file | Source URL |
|---|---|---|---|---|---|---|
| GitHub Spec Kit | v1.2.5 (28 Oct); `main` at `7370e59` (2026-10-30) | plan `Technical Context` (Performance Goals, Constraints, Scale/Scope); plan `Constitution Check` GATE; clarify "Non-Functional Quality Attributes"; analyze "missing non-functional task coverage"; checklist `performance.md`/`security.md`; tasks "Phase N: Polish & Cross-Cutting Concerns" | Constitution Check is a gate inside `/plan` ("ERROR if violations unjustified"). Technical Context fields may be filled with "NEEDS CLARIFICATION". `/clarify` is "expected" before `/plan` but can be skipped. `/analyze` and `/checklist` are user-invoked. Polish phase is last and optional in practice | No. Every command reads one `FEATURE_DIR` plus the constitution. Nothing reads other features' plans | `/memory/constitution.md` (principles, "Performance Standards" as an example heading). Checked by `/plan` gate and by `/analyze` (constitution conflicts are CRITICAL) | github.com/github/spec-kit/tree/main/templates |
| Kiro (AWS) | Docs pages updated 2026-08-04, 2026-10-15, 2026-10-21 | No NFR section in requirements (EARS) or design. design.md covers "Error handling and testing strategy". "Design-First" mode suggested when "System must meet strict non-functional requirements" | Spec phases have approval gates (Quick Spec skips them). "Analyze Requirements" offered before design. Hooks automatic once configured | No. "Each spec's three-phase workflow and task waves operate within that spec" (no cross-spec step documented) | `.kiro/steering/` (`product.md`, `tech.md`, `structure.md`, custom e.g. `security-policies.md`). Context only, not checked. Hooks (incl. Pre/Post Task Execution) can run checks | kiro.dev/docs/specs/, kiro.dev/docs/steering/, kiro.dev/docs/hooks/types/ |
| BMAD Method | v6.12.4 (14 Oct); `main` at `61e01f8` (2026-10-18) restructured as `skills/` | PRD template: optional "Cross-Cutting NFRs", "Performance Budgets", "Operational Requirements", "Constraints and Guardrails" (incl. Cost). Architecture spine: "Consistency Conventions", "Invariants & Rules", "Inherited Invariants". Readiness gate has no NFR check | All NFR sections optional ("Adapt-In Menu"). Architecture reviewer gate scaled to stakes; may be skipped for a prototype | Partly. Architecture spine is written per altitude (initiative, feature, epic) and binds the stories under it: "An epic spine fixes the invariants the epic's stories must share" | Architecture spine; `project-context.md` (being retired on `main`) | github.com/bmad-code-org/BMAD-METHOD |
| BMAD TEA (Test Architect) module | v3.6.0 (31 Oct), separate repo | `nfr-assess` (security, performance, reliability, maintainability; PASS/CONCERNS/FAIL/N/A); `trace` Phase 2 gate decision | Optional: "TEA is optional. There are five engagement models". `nfr-assess` listed as "(optional)" in the greenfield release gate | Yes when scoped to `system`; scope can be epic, story or system. Runs at release, after evidence exists | Reads NFRs from PRD, architecture or `test-design` | bmad-code-org.github.io/bmad-method-test-architecture-enterprise/ |
| Tessl (spec-driven-development tile) | tile 2.0.1; repo `b8fdff7` (2026-03-30) | `requirement-gathering` gap category "Missing non-functional requirements (performance, security)"; output "Constraints" item | Steering rules (`spec-before-code` etc.) are context. Skills invoked on demand. `work-review` is on-demand | Partial. Scans `specs/` for specs "related to the request". `work-review` finds specs whose `targets` match changed files | Tile steering rules; `specs/*.spec.md` with `targets` and `[@test]` links | github.com/tesslio/spec-driven-development-tile |
| Matt Pocock skills | `main` at `a496f55` (2026-10-29), 38 skills | None. No mention of performance/NFR/security in `to-spec`, `to-tickets`, `code-review`, `implement-spec`, `improve-codebase-architecture` | n/a | `implement-spec` step 7 runs `code-review` on the integration branch once all tickets finish (Standards + Spec axes). `to-tickets` sequences prefactoring and expand-contract across tickets | Repo standards docs (`CODING_STANDARDS.md`/`CONTRIBUTING.md`), ADRs, glossary, read by `code-review` Standards axis | github.com/mattpocock/skills |
| Claude Code | docs read 2026-10-31 (v2.1.3xx era) | No NFR template. `/code-review` (correctness, plus "reuse, simplification, and efficiency" at higher effort); `/security-review`; managed Code Review; hooks; agent-team hooks `TaskCompleted`/`TeammateIdle` | `/code-review`, `/security-review` user- or Claude-invoked. Managed Code Review automatic per repo but never blocks. Hooks deterministic and blocking (exit 2) | `/code-review` reviews a diff/branch/path. Managed Code Review reads "the diff and the surrounding code". No whole-system NFR step | `CLAUDE.md` (advisory; doc says use hooks for rules that "can't have exceptions"); `REVIEW.md` for managed review | code.claude.com/docs/en/best-practices, /code-review-local, /code-review, /agent-teams |
| Anthropic engineering posts | 2024-12-19, 2025-06-13, 2025-11-26 | "Effective harnesses": feature list JSON, one feature per session, end-to-end test at session start. No NFR mention | n/a | Startup smoke test only. Suggests future "quality assurance agent" | `claude-progress.txt`, feature list | anthropic.com/engineering/effective-harnesses-for-long-running-agents |
| OpenAI Codex | docs read 2026-10-31 (no page date) | GitHub Code Review flags "only P0 and P1 issues"; `## Code Review Rules` in AGENTS.md; Security Review with threat models | Automatic reviews when enabled per repo or per user | Code Review "reviews the pull request diff". Security Review adds "supporting repository context" | `AGENTS.md` (concatenated root to leaf, 32 KiB cap); not enforced | learn.chatgpt.com/docs/agent-configuration/agents-md, learn.chatgpt.com/docs/third-party/github |
| Cursor | docs read 2026-10-31 | Bugbot: "bugs, security issues, and code quality problems" (performance not listed). Plan Mode: no NFR mention | Bugbot automatic on every PR update by default; non-blocking unless fail-on-unresolved plus branch protection | Bugbot "analyzes PR diffs" plus PR comments | `.cursor/rules` (Always/Intelligent/Files/Manual), `AGENTS.md`, Team Rules; `BUGBOT.md` for review. All context, not controls | cursor.com/docs/bugbot, cursor.com/docs/context/rules |
| Devin (Cognition) | docs read 2026-10-31 | Playbooks (Procedure, Specifications, Forbidden Actions…); Devin Review (bugs, security with CWE IDs). Performance only as a `REVIEW.md` example | Devin Review can be automatic; check not required unless branch rule says so | Devin Review: diff plus codebase context; DeepWiki indexes the repo | Knowledge (deprecated, moving to Skills), `REVIEW.md`, reads `AGENTS.md`/`CLAUDE.md` | docs.devin.ai/work-with-devin/devin-review.md |
| Aider | v0.86.0 (9 Aug) | None. Architect/editor mode is a two-model edit split | Auto-lint on by default; `--auto-test` opt-in | No | `CONVENTIONS.md` via `--read` (context only) | aider.chat/docs/usage/modes.html |
| OpenHands | docs read 2026-10-31; Planning Mode BETA (Mar 2026 update) | None. Planning Mode writes `PLAN.md` | Plan mode user-selected | No | `AGENTS.md`, `.agents/skills/`, keyword-triggered skills | docs.openhands.dev/overview/skills |
| SWE-agent / mini-swe-agent | docs read 2026-10-31 | None | n/a | No: single task/issue, linear history | None | swe-agent.com/latest/, mini-swe-agent.com/latest/ |
| GitHub Copilot (coding agent, code review) | docs read 2026-10-31 | Code review: "bugs, security vulnerabilities, and style inconsistencies"; Balanced adds "cross-service changes". Separate GitHub Code Quality (CodeQL) for reliability/maintainability | Automatic via user setting or ruleset; does not block by default | "Full project context gathering analyzes your entire repository" | `.github/copilot-instructions.md`, `.github/instructions/*.instructions.md`, `AGENTS.md`, skills | docs.github.com/en/copilot/concepts/agents/code-review |
| claude-task-master | task-master-ai@0.43.1 (31 Mar) | None in `parse-prd` except research mode: "security concerns, or scalability issues not explicitly mentioned in the PRD". Complexity analysis scores effort only | Research mode opt-in | No cross-task quality check | None | github.com/eyaltoledano/claude-task-master |
| Agent OS (Builder Methods) | v3.0.0 (20 Jan) | None. `shape-spec` asks "Are there any constraints or requirements I should know about?" | Standards injected after user confirms | No | `agent-os/standards/` + `index.yml`; injected, never compliance-checked | github.com/buildermethods/agent-os |
| Amp (Sourcegraph/Amp) | docs read 2026-10-31; checks post 2026-02-04 | `.agents/checks/*.md` review checks; official example `perf.md` "Flags common performance anti-patterns". Oracle plugin reviews and advises | `amp review` or on request; not automatic per the post | One agent per check, scoped by directory; reviews "outstanding changes" | `AGENTS.md`, `.agents/checks/` | ampcode.com/news/liberating-code-review |
| CodeRabbit | docs read 2026-10-31 | Pre-merge checks (built-in: docstring coverage, title, description, issue assessment; custom natural-language checks). No performance built-in | Automatic per PR; custom check blocks only in `error` mode with Automatic Approvals | Code graph analysis (definitions, references, co-changed files) | `.coderabbit.yaml` `path_instructions`; picks up `AGENTS.md`, `.cursorrules` | docs.coderabbit.ai/pr-reviews/pre-merge-checks.md |
| Greptile | docs read 2026-10-31 | Custom rules (`.greptile/config.json`, `rules.md`); no performance category found | Automatic PR review | Yes: repository graph; "Identifies all code that could be affected by changes" | `.greptile/`, `greptile.json` | greptile.com/docs/how-greptile-works/graph-based-codebase-context.md |
| Qodo | docs read 2026-10-31 | Review Standards, Rule Miner; areas "correctness, standards compliance, architecture, and risk" | Automatic on PR publish/push | Full repository, plus cross-repo impact tracing | Review Standards mined from codebase and PR history | docs.qodo.ai/code-review.md |
| MetaGPT | `main` read 2026-10-31 | None. Architect `WriteDesign` nodes: Implementation approach, File list, Data structures and interfaces, Program call flow, Anything UNCLEAR. PRD has no NFR node | Always runs (fixed SOP) | Architect designs the whole program once, then engineers implement files | None | github.com/FoundationAgents/MetaGPT/blob/main/metagpt/actions/design_api_an.py |
| Cognition "Don't build multi-agents" | 2025-06-12 | n/a (essay) | n/a | Argues decomposition loses shared decisions | n/a | cognition.com/blog/dont-build-multi-agents |

## Per-tool notes

### GitHub Spec Kit

Release v1.2.5 (28 Oct; year not shown on the releases page, taken as 2026 from the commit date). `main` at `7370e592f3cd9c06e5b698983957e227c75b30e2`, 2026-10-30. Files read: `templates/plan-template.md`, `spec-template.md`, `tasks-template.md`, `constitution-template.md`, `templates/commands/{analyze,checklist,clarify,plan,implement}.md`.

1. **NFR step/template/gate.**
   - `plan-template.md` → **Technical Context**: "**Performance Goals**: [domain-specific, e.g., 1000 req/s, 10k lines/sec, 60 fps or NEEDS CLARIFICATION]", "**Constraints**: [domain-specific, e.g., <200ms p95, <100MB memory, offline-capable or NEEDS CLARIFICATION]", "**Scale/Scope**: …". VERIFIED.
   - `plan-template.md` → **Constitution Check**: "*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*" Body is "[Gates determined based on constitution file]". VERIFIED.
   - `spec-template.md`: mandatory "Success Criteria" / "Measurable Outcomes" ("must be technology-agnostic and measurable"). The template has no NFR section: performance appears only as an example success criterion ("System handles 1000 concurrent users without degradation"). VERIFIED.
   - `/clarify` taxonomy has "Non-Functional Quality Attributes": performance, scalability, "Reliability & availability", observability, "Security & privacy", compliance. At most five questions in total, so not every category becomes a question. VERIFIED.
   - `/analyze` flags "Success Criteria requiring buildable work (performance, security, availability) not reflected in tasks"; HIGH for "ambiguous security/performance attribute"; MEDIUM for "missing non-functional task coverage"; "Constitution conflicts are automatically CRITICAL". VERIFIED.
   - `/checklist` ("UNIT TESTS FOR REQUIREMENTS WRITING") offers `performance.md`, `security.md`, and a "Non-Functional Requirements" category ("are they specified?"). It tests the requirements text, not the code. VERIFIED.
   - `tasks-template.md` "Phase N: Polish & Cross-Cutting Concerns": "Improvements that affect multiple user stories", "Performance optimization across all stories", "Security hardening". VERIFIED.
2. **Automatic or optional.**
   - `/plan` runs the Constitution Check: "Evaluate gates (ERROR if violations unjustified)", then "Re-evaluate Constitution Check post-design". VERIFIED.
   - The Technical Context fields allow "NEEDS CLARIFICATION", and `/plan` sends unknowns to Phase 0 research. Nothing requires a performance budget to be set. VERIFIED.
   - `/analyze` is read-only and user-invoked. VERIFIED.
   - `/implement` stops if a checklist has unchecked items, but a user "yes" continues. VERIFIED.
   - `/implement` completion validation checks tasks, tests and plan adherence. Performance is only "Polish and validation: … performance optimization", not a completion check. VERIFIED.
3. **Cross-spec view.** None. `/plan` reads only the current feature spec and the constitution: the page "does not mention" other features' specs or plans. `/analyze` reads one `FEATURE_DIR`. `/implement` does no cross-feature verification. VERIFIED. The tasks "Polish & Cross-Cutting" phase is cross-story within one feature, not across features. VERIFIED.
4. **Persistent standards.** `/memory/constitution.md`. The template has "Security Requirements, Performance Standards, etc." only as example section names, and "All PRs/reviews must verify compliance" as an example governance rule. VERIFIED. It is checked automatically at the `/plan` gate and by `/analyze`. Only MUST principles are checked, and only against the current feature's artifacts. VERIFIED.

**Relevance.** Spec Kit has the richest NFR template fields of the tools surveyed. They are per-feature, though: a constitution principle such as "one commit per batch on the hot path" would be checked against each plan in isolation. No step sums budgets across features (e.g., ten features each "<200ms p95" on the same path). Inference, not stated by Spec Kit.

### Kiro (AWS)

1. **NFR step.**
   - requirements.md uses EARS ("WHEN [condition/event] THE SYSTEM SHALL [expected behavior]"). No NFR section is documented. VERIFIED (kiro.dev/docs/specs/feature-specs/, updated 2026-08-04).
   - The design phase covers "System architecture and component design", "Sequence diagrams and data flow", and "Error handling and testing strategy". VERIFIED (kiro.dev/docs/specs/, updated 2026-10-15).
   - The only NFR mention: Design-First mode suits cases where "System must meet strict non-functional requirements (latency, throughput, compliance)". VERIFIED.
   - There is a "Correctness" (property-based testing) page that I did not read. UNVERIFIED.
2. **Automatic or optional.** Approval gates between phases are implied: Quick Spec runs "without approval gates". VERIFIED, but the gate contents are not described. "Analyze Requirements" checks "inconsistencies, ambiguities, and gaps" before design, and is offered, not forced. VERIFIED.
3. **Cross-spec view.** None documented. Run-all-tasks builds a dependency graph and runs tasks in "waves" within one spec. VERIFIED.
4. **Persistent standards.**
   - Steering: `product.md`, `tech.md`, `structure.md`, "included in every interaction by default". Inclusion modes are always, fileMatch, manual and auto. VERIFIED (updated 2026-10-21).
   - The page describes no validation step, so steering is context only. VERIFIED. It suggests a `security-policies.md` in `always` mode. VERIFIED.
   - Hooks: Prompt Submit, Agent Stop, Session Start/End, Pre/Post Tool Use, File Create/Save/Delete, **Pre Task Execution / Post Task Execution** ("Triggers after a spec task completes execution"), Manual. VERIFIED.
   - Hooks "activate automatically when a session starts". VERIFIED. A Post Task Execution hook is the natural slot for a per-task benchmark, but no performance example is given. VERIFIED (absence).

### BMAD Method

Current stable is v6.12.4 (14 Oct). `main` at `61e01f8` (2026-10-18) has moved to a flat `skills/` layout ahead of v7: the v6.12.2 notes say "newer releases target v7". VERIFIED. The v4-era "PO master checklist" was not found in the visible part of either tree listing (both listings were truncated). UNVERIFIED as absent.

1. **NFR step.**
   - `skills/bmad-prd/assets/prd-template.md`: the "Essential Spine" holds per-feature NFRs, "only if any apply uniquely to this feature". The "Adapt-In Menu → Cross-cutting quality and shape" offers **Cross-Cutting NFRs**: "system-wide non-functional requirements not tied to a single feature (performance, security, reliability, observability)", plus **Constraints and Guardrails** (Safety, Privacy, Cost), **Performance Budgets** (developer products: latency, throughput, resource use) and **Operational Requirements** (SLAs, RTO/RPO). VERIFIED.
   - Architecture spine template: **Consistency Conventions** ("Defaults that bind where independent builders would drift"; row "State & cross-cutting (mutation, errors, logging, config, auth)"), **Invariants & Rules** ("calls a future builder can't read off compliant code"), and **Inherited Invariants** ("A local decision that contradicts one is a conflict to surface, not an override"). The spine template has no NFR or performance row. VERIFIED.
2. **Automatic or optional.**
   - Every PRD NFR section is optional ("add the clusters the product calls for"). VERIFIED.
   - The architecture reviewer gate checks "every dimension the altitude owns is decided, deferred, or an open question" and treats "a whole dimension left silent" as a finding. It singles out the operational/environmental envelope, not performance. Its rigour scales with stakes: "a throwaway prototype may run it quietly or skip the gate entirely". VERIFIED.
   - The sprint-planning readiness gate (v6.12.4 `references/readiness-gate.md`) checks traceability, independence, recorded decisions and artifact conflicts. It has no NFR or performance check. VERIFIED.
3. **Cross-spec view.** Yes, structurally: "An epic spine fixes the invariants the epic's stories must share". Spines exist per altitude (initiative → features → epics → stories). VERIFIED. `bmad-code-review` on `main` is a script-driven "several independent reviewers in parallel". Its context was not visible in SKILL.md. UNVERIFIED.
4. **Persistent standards.** The architecture spine. `project-context.md` (v6.12.4 template: Policy, Conventions, Known pitfalls, with no NFRs) is described as retired on `main`. VERIFIED. No automatic compliance check of code against the spine was found. UNVERIFIED (absence).

### BMAD TEA (Test Architect, separate repo)

The repo is `bmad-code-org/bmad-method-test-architecture-enterprise`, latest release v3.6.0 (31 Oct).

1. **NFR step.** `nfr-assess` audits "non-functional requirement (NFR) evidence across security, performance, reliability, and maintainability". Statuses: PASS "Applicable NFR thresholds met, backed by evidence", CONCERNS, FAIL "A threshold breached…", N/A. Inputs are "NFRs defined in PRD, requirements doc, architecture, or `test-design`". Evidence examples include a load test report ("k6, Artillery, or JMeter run against a staging deploy"). VERIFIED.
2. **Automatic or optional.** "TEA is optional. There are five engagement models." The greenfield release gate lists "`nfr-assess` (optional), `trace` Phase 2". Missing evidence yields CONCERNS, not FAIL. VERIFIED.
3. **Cross-spec view.** The scope can be an epic, a story or `system`. It runs late ("Before production release", "Phase 4 or Gate"). VERIFIED. It is the only surveyed workflow with a named system-scope NFR assessment. It audits evidence after the build rather than shaping the plan.
4. **Persistent standards.** It reads NFRs from the PRD and architecture. There is no separate standards file.

### Tessl

The registry tile `tessl-labs/spec-driven-development`. `tile.json` version 2.0.1; repo `b8fdff7` (2026-03-30).

1. **NFR step.** `skills/requirement-gathering/SKILL.md` step 2 lists "Missing non-functional requirements (performance, security)" as a gap category. Its output includes "Constraints" covering "performance, security, compatibility requirements". VERIFIED. The spec format (`docs/spec-format.md`) covers functional requirements with `[@test]` links and `targets`, with no NFR section. VERIFIED.
2. **Automatic or optional.** The steering rules are context: "Never begin implementation without an approved spec." Skills run on demand. `work-review` runs when asked or "before marking a task done". VERIFIED.
3. **Cross-spec view.** Partial. Requirement gathering scans `specs/` for related specs. `work-review` finds specs whose `targets:` match `git diff --name-only HEAD~1`, but runs `[@test]` links across `specs/*.spec.md`. VERIFIED. No NFR aggregation.
4. **Persistent standards.** Tile steering rules, and the registry for third-party library specs. A third-party review says the Spec Registry launched in open beta on 2025-09-16. UNVERIFIED.

### Matt Pocock skills

`main` at `a496f5513d4a4cf889bc48e189e579f4b0bdf6aa` (2026-10-29, "implement-spec: a fresh implementer subagent per ticket (#1342)"). A local copy will be analysed separately; here is the current list and an NFR keyword check of the key skills.

- **engineering:** ask-matt, code-review, codebase-design, diagnosing-bugs, domain-modeling, grill-with-docs, implement-spec, implement, improve-codebase-architecture, pr, prototype, research, retro, setup-matt-pocock-skills, tdd, to-spec, to-tickets, triage, wayfinder, wizard.
- **in-progress:** chief-of-staff, claude-handoff, loop-me, setup-ts-deep-modules, writing-beats, writing-fragments, writing-shape.
- **misc:** git-guardrails-claude-code, migrate-to-shoehorn, scaffold-exercises, setup-pre-commit.
- **productivity:** grill-me, grilling, handoff, teach, to-questionnaire, wait-what, writing-for-agents.

VERIFIED (38 SKILL.md paths).

The NFR keyword check came back empty:

- `to-spec` sections are Problem Statement, Solution, User Stories, Implementation Decisions, Testing Decisions, Out of Scope, Further Notes. None of performance, non-functional, NFR, security, scale, memory, cost or cross-cutting appears. VERIFIED.
- `to-tickets`: no NFR terms. It has cross-ticket sequencing: prefactoring first, expand-contract with a contract ticket "blocked by every migrate batch". VERIFIED.
- `code-review`: axes are Standards (repo standards docs plus a Fowler smell baseline) and Spec. It works on `git diff <fixed-point>...HEAD`, with no performance terms. VERIFIED.
- `implement-spec` step 7: "Once all tickets are complete, call the Skill tool with `code-review` on the integration branch". That is the only whole-spec review, and it uses the two axes above. VERIFIED.
- `improve-codebase-architecture`: no performance terms. It is hot-spot-driven from `git log`, and `disable-model-invocation: true` makes it manual. VERIFIED.
- `diagnosing-bugs` describes itself as covering "performance regressions" (seen in the local skill listing, not re-read in the repo). UNVERIFIED for the repo copy.

### Claude Code (Anthropic)

1. **NFR step.** Claude Code has no NFR template; it is a harness.
   - `/code-review` "looks for correctness bugs" and, "Depending on your model and effort level, the review also covers reuse, simplification, and efficiency cleanups". It follows `CLAUDE.md`. VERIFIED.
   - The managed Code Review service "focuses on correctness: bugs that would break production". Agents "examine the diff and the surrounding code"; "A review never approves or blocks the pull request". It is tunable with `REVIEW.md`. VERIFIED.
   - `/security-review` (blog 2025-08-06) checks injection, XSS, authN/Z, insecure data handling and dependencies. It runs on demand, and the GitHub Action runs on PRs. VERIFIED.
   - The agent-teams doc's example of a parallel review assigns one teammate to "checking performance impact". It is an example prompt, not a feature. VERIFIED.
2. **Automatic or optional.** The best-practices doc splits instructions into non-deterministic ("Your prompt, `CLAUDE.md`, skills") and deterministic ("Hooks, … Tests, typecheck, a `Stop` hook, CI"). "If something has to happen every time, make it a hook, a deny rule, or a managed setting. `CLAUDE.md` is for guidelines, and Claude is allowed to decide a guideline doesn't apply." VERIFIED. Agent teams add `TaskCreated`/`TaskCompleted`/`TeammateIdle` hooks, where "Exit with code 2 to prevent completion". These are a natural slot for a per-task benchmark gate. VERIFIED. Agent teams are experimental and off by default. VERIFIED.
3. **Cross-spec view.** None built in. `/code-review` targets a diff, branch, path or range. VERIFIED. The agent-teams doc warns: "For sequential tasks, same-file edits, or work with many dependencies, a single session or subagents are more effective". VERIFIED.
4. **Persistent standards.** `CLAUDE.md` (advisory; the doc recommends keeping it short), `.claude/settings.json` hooks (enforced), managed settings, `REVIEW.md` (managed review only). VERIFIED.

**Engineering posts.**

- "Effective harnesses for long-running agents" (2025-11-26) uses an initializer agent, a JSON feature list where the coding agent only flips `passes`, one feature per session, `claude-progress.txt`, and an end-to-end smoke test at session start. VERIFIED. It has no discussion of performance or other non-functional qualities. VERIFIED (absence). As future work it names "specialized agents like a testing agent, a quality assurance agent, or a code cleanup agent". VERIFIED.
- "Building effective agents" (2024-12-19) covers orchestrator-workers and evaluator-optimizer. It does not discuss context loss in decomposition or system-wide quality. VERIFIED (absence).
- "How we built our multi-agent research system" (2025-06-13): "Without detailed task descriptions, agents duplicate work, leave gaps…"; "most coding tasks involve fewer truly parallelizable tasks than research". VERIFIED.

### OpenAI Codex

1. **NFR step.** None as a template. GitHub Code Review: "Codex flags only P0 and P1 issues so review comments stay focused on high-priority risks". VERIFIED. A secondary source says P1 covers performance or concurrency. UNVERIFIED. Rules are written in a `## Code Review Rules` section of the nearest AGENTS.md. VERIFIED. An older English page title "Review guidelines" appears in search snippets. UNVERIFIED.
2. **Automatic or optional.** Automatic reviews are enabled per repository by admins, or per user via "Automatic review". VERIFIED.
3. **Cross-spec view.** "Codex reviews the pull request diff". Security Review analyzes the diff plus "supporting repository context" and configured threat models. VERIFIED.
4. **Persistent standards.** AGENTS.md is global (`AGENTS.override.md` or `AGENTS.md`), then root to current directory, at most one file per directory, concatenated with the closest last, capped at 32 KiB. Codex "reads `AGENTS.md` files before doing any work". The docs mention no enforcement; to verify loading, they suggest asking Codex to summarize the instructions. VERIFIED.

### Cursor

1. **NFR step.** None in Plan Mode (asks questions, researches the codebase, writes an editable plan; no NFR mention). VERIFIED. Bugbot "identifies bugs, security issues, and code quality problems"; performance is not listed. VERIFIED.
2. **Automatic or optional.** Bugbot "Runs automatic reviews on every PR update" by default. Findings default to a `neutral` conclusion. Blocking requires the fail-on-unresolved option plus a required check. VERIFIED.
3. **Cross-spec view.** "Bugbot analyzes PR diffs", plus existing PR comments. VERIFIED.
4. **Persistent standards.** Rules come in Always Apply, Apply Intelligently, Apply to Specific Files and Apply Manually, plus AGENTS.md (nested) and Team Rules (which can be marked required). They are context: "AI guidance should not be your only security control". `.cursor/rules` do not apply to Bugbot, which uses `.cursor/BUGBOT.md`. VERIFIED.

### Devin (Cognition)

1. **NFR step.** Playbooks have Procedure, Specifications ("what should be true after Devin is done?"), Advice and Pointers, Forbidden Actions, Required from User. Playbooks are moving to Skills from 2026-10-24. VERIFIED. Devin Review checks bugs (by confidence) and security (CWE IDs). Performance appears only as an example `REVIEW.md` topic ("database queries inside loops"). VERIFIED.
2. **Automatic or optional.** "Devin can automatically review PRs". Its status "does not make it a merge requirement" unless branch rules require it. VERIFIED.
3. **Cross-spec view.** Devin Review uses the diff plus "relevant context from the rest of the codebase". DeepWiki indexes repos (page listed but not read). VERIFIED for Review; UNVERIFIED for DeepWiki detail.
4. **Persistent standards.** Knowledge (trigger-retrieved or pinned; deprecated in favour of Skills). It auto-reads `AGENTS.md`, `CLAUDE.md`, `.cursorrules` and similar files. `**/REVIEW.md` is read by Review. VERIFIED.

**"Don't build multi-agents"** (Walden Yan, 2025-06-12).

- Principles: "Share context, and share full agent traces, not just individual messages", and "Actions carry implicit decisions, and conflicting decisions carry bad results". VERIFIED.
- Flappy Bird example: two subagents built a Mario-style background and a mismatched bird. Even with shared context, conflicting unstated assumptions persist. VERIFIED.
- This is the closest published statement of our failure mode. Each slice's implicit decisions (here, a quality choice) are invisible to the others.

### Aider

v0.86.0 (9 Aug).

- Architect mode: "An architect model will propose changes and an editor model will translate that proposal into specific file edits". This splits planning from editing within one request, not across tasks. VERIFIED.
- Lint and test: auto-lint is on by default. `--test-cmd` with `--auto-test` runs tests after each edit, and is opt-in. VERIFIED.
- `CONVENTIONS.md` is loaded with `--read`, or with `read:` in `.aider.conf.yml`. The page describes no enforcement. VERIFIED.
- Aider has no NFR or cross-task feature. VERIFIED (absence on the pages read).

### OpenHands

- Context comes from always-on repository context (`AGENTS.md`, also `CLAUDE.md`/`GEMINI.md`), keyword-triggered skills, and `.agents/skills/` (the legacy `.openhands/microagents/` is still supported). VERIFIED.
- Planning Mode BETA "generate[s] a structured PLAN.md file outlining the steps needed to implement your feature" (openhands.dev March 2026 product update). VERIFIED.
- No NFR or cross-task quality step was found. VERIFIED (absence on pages read).

### SWE-agent / mini-swe-agent

- SWE-agent lets a model "autonomously use tools to fix issues in real GitHub repositories". It is in maintenance-only mode, and the page recommends mini-swe-agent. VERIFIED.
- mini-swe-agent is bash-only with a linear history, scores ">74% on the SWE-bench verified benchmark", and has no planning or quality layer. VERIFIED.
- Both are scoped to one task or issue by design: the benchmark unit is a single issue. Inference.

### GitHub Copilot (coding agent and code review)

- Code review covers "common issues such as bugs, security vulnerabilities, and style inconsistencies". Balanced adds "complex logic, security-sensitive code, and cross-service changes". VERIFIED.
- "Full project context gathering analyzes your entire repository to better understand the context of code changes". VERIFIED.
- Reviews run automatically when the user setting or a ruleset is enabled. They don't count toward required approvals by default. VERIFIED.
- Instructions: `.github/copilot-instructions.md`, `.github/instructions/**/*.instructions.md` (path-specific), root `AGENTS.md`, `.github/skills/`, all read from the PR head branch. VERIFIED.
- GitHub Code Quality (CodeQL-powered) covers reliability and maintainability separately. Merge gating comes from its rulesets. VERIFIED.
- The coding agent runs tests and linters in a cloud environment, and opens "one pull request per task". VERIFIED.
- No performance category was found. VERIFIED (absence).

### claude-task-master

task-master-ai@0.43.1 (31 Mar).

- `parse-prd` task fields: `details`, `testStrategy`, `dependencies` (only on lower IDs). It has no NFR section. Research mode asks for "potential technical challenges, security concerns, or scalability issues not explicitly mentioned in the PRD". The prompt also says "Always aim to provide the most direct path to implementation, avoiding over-engineering". VERIFIED (`src/prompts/parse-prd.json`).
- Complexity analysis outputs `complexityScore` (1-10), `recommendedSubtasks` and `expansionPrompt`. It considers "implementation effort, technical challenges, dependencies, and testing requirements". It has no NFR or cross-task conflict check. VERIFIED (`src/prompts/analyze-complexity.json`).

### Agent OS (Builder Methods)

v3.0.0 (20 Jan).

- `shape-spec` reads `agent-os/standards/index.yml` "to identify relevant standards based on the feature being built". The user confirms which apply, and they are copied into `standards.md`. Its only constraint question is "Are there any constraints or requirements I should know about?". It reads no other specs. VERIFIED.
- `inject-standards` does relevance matching against index descriptions. The page describes no compliance check: the only validation is that the requested paths exist. VERIFIED.
- The default profile ships only `global/tech-stack.md`. VERIFIED.

### Amp

- **Checks** (post 2026-02-04): "Checks are user-defined invariants or review criteria scoped to specific parts of your codebase". They live in `.agents/checks/`. VERIFIED.
- The official example `perf.md` ("Flags common performance anti-patterns": nested loops over the same collection, `includes()` in loops, sorting in loops). VERIFIED.
- "The `code_review` tool will kick off a separate agent for each check". The post claims this gives a stronger guarantee than putting the same rule in AGENTS.md. VERIFIED.
- Checks are scoped to the directory containing `.agents/`. Reviews are invoked with `amp review` or on request, not automatically. VERIFIED.
- Oracle: "`@amp/oracle` adds the `oracle` tool. It reviews code and advises on hard decisions, and it never edits files". It is a plugin and agent-invoked. VERIFIED.
- Amp is the only surveyed tool whose official docs ship a performance review rule as a first-class example. It still reviews changes, not the system.

### AI review bots: CodeRabbit, Greptile, Qodo

| Bot | Diff vs codebase | Performance | Blocking |
|---|---|---|---|
| CodeRabbit | Code graph analysis plus "50+ open-source linters and security scanners"; context "based on your entire repository" | No built-in performance check | Pre-merge checks (Docstring Coverage, PR Title, PR Description, Issue Assessment, custom). A check blocks only in `error` mode, when it fails, and with Automatic Approvals |
| Greptile | "builds a complete graph of your repository"; "Identifies all code that could be affected by changes"; "Finds inconsistencies … based on existing code" | No performance category found | Not checked |
| Qodo | Reviews the full repository; cross-repo review "traces impact in both directions" | Areas are correctness, standards compliance, architecture and risk; no performance category | Not stated |

Sources and verification status:

- **CodeRabbit.** Pre-merge checks and the review overview are VERIFIED. The code graph details (definitions, references, files co-changed in commit history) come from the docs search snippet: the integration page returned 404 on fetch, so they are UNVERIFIED-fetch. `path_instructions` and automatic pickup of `AGENTS.md`/`.cursorrules` are VERIFIED.
- **Greptile.** VERIFIED. Custom rules live in `.greptile/config.json` or `rules.md` (search snippet of official docs; UNVERIFIED-fetch).
- **Qodo.** VERIFIED (docs.qodo.ai/code-review.md). Rule Miner turns recurring PR-history patterns into rules. VERIFIED.

All three run per PR. All three widen *context* beyond the diff, mainly to catch breakage and inconsistency, but none documents an aggregate quality budget or a performance gate.

### Multi-agent frameworks: MetaGPT, ChatDev, Claude Code agent teams

- MetaGPT's Architect `WriteDesign` nodes are Implementation approach, File list, Data structures and interfaces, Program call flow, and Anything UNCLEAR. The PRD nodes include Requirement Pool and Requirement Analysis but no NFR node. VERIFIED (`metagpt/actions/design_api_an.py`, `write_prd_an.py` on `main`). The architect role gives a single whole-program design up front, but it carries no quality-attribute fields.
- ChatDev was not read. UNVERIFIED.
- Claude Code agent teams: see the Claude Code section above.

## Patterns across tools

**What is common.**

1. **NFRs appear as optional template fields, early.**
   - Spec Kit: Technical Context, the `/clarify` taxonomy, `/checklist`.
   - BMAD: the PRD Adapt-In "Cross-Cutting NFRs" and "Performance Budgets".
   - Tessl: requirement gathering's "Missing non-functional requirements".
   - Kiro: Design-First mode.
   - Each is filled per feature, can be left as "NEEDS CLARIFICATION" or omitted, and is never re-summed.
2. **Project-wide standards files are near-universal, and almost always advisory.**
   - Advisory files: constitution, steering, AGENTS.md/CLAUDE.md, `.cursor/rules`, Knowledge, conventions, Agent OS standards.
   - Only Spec Kit checks its constitution automatically (`/plan` gate and `/analyze` CRITICAL), and only against the current feature's artifacts.
   - Claude Code's docs say outright that `CLAUDE.md` is guidance the model may decide doesn't apply, and that rules without exceptions belong in hooks or CI. Amp makes the same argument for running a separate agent per check.
3. **Review is diff-centred everywhere.** The managed reviewers (Claude Code Review, Codex, Cursor Bugbot, Devin Review, Copilot, CodeRabbit, Greptile, Qodo) widen the context to callers and related files to find breakage or inconsistency. Their default focus is correctness and security; performance is at most a user-written rule (Amp `perf.md`, Devin `REVIEW.md` example, Codex P1 per a secondary source).
4. **Deterministic gating exists only as generic hooks or CI.** Examples are Claude Code `Stop`/`TaskCompleted` hooks, Kiro Pre/Post Task Execution hooks, Aider `--auto-test`, and CodeRabbit `error`-mode checks. None ships a benchmark, a memory budget or a commit-count probe. The project must supply the probe.
5. **Decomposition tools optimise for independence.** Spec Kit stories are "independently testable", and cross-story dependencies "break independence". BMAD's readiness gate requires "stories are independently completable". Pocock's tickets are vertical slices. task-master aims for "the most direct path to implementation". Independence is what makes each slice locally reviewable, and also what hides shared hot paths.

**What is missing.**

1. **No surveyed tool has a step that adds up the cost of all features on a shared path.** None takes a per-request or per-page budget and checks that the sum of every slice's additions stays within it. BMAD TEA `nfr-assess` at `system` scope comes closest, but it is optional, runs at release, and audits evidence rather than shaping the plan.
2. **No owner for a shared resource across specs.** Only BMAD's architecture spine ("Consistency Conventions", "Inherited Invariants", epic-level invariants "the epic's stories must share") gives a place to say "one parse per page" or "commits are batched". Even there, nothing found checks compliance at review time.
3. **No reviewer compares a change against sibling tasks.** Reviews compare a change with its own spec, plus repository context for breakage. None asks whether this change duplicates work another task already does on the same path (re-parsing, an extra commit). Cognition's essay names the cause: "Actions carry implicit decisions, and conflicting decisions carry bad results."
4. **No quality regression baseline in the agent loop.** The long-running-harness guidance protects functional regressions (feature list, startup smoke test) but not performance or resource regressions.

**Implication for us (inference).** In every surveyed tool, system-wide qualities hold only if a project adds three things:

- a durable, cross-spec owner for each shared resource, such as an ADR or an architecture-spine invariant;
- a deterministic probe that measures it, as a hook or CI benchmark;
- a review step that reads sibling changes or the whole hot path, not just the diff.

None of the surveyed tools provides all three by default.

## Sources (accessed 2026-10-31)

- Spec Kit: https://github.com/github/spec-kit/tree/7370e592f3cd9c06e5b698983957e227c75b30e2/templates (plan, spec, tasks, constitution templates; commands analyze, checklist, clarify, plan, implement); releases https://github.com/github/spec-kit/releases
- Kiro: https://kiro.dev/docs/specs/ ; https://kiro.dev/docs/specs/feature-specs/ ; https://kiro.dev/docs/steering/ ; https://kiro.dev/docs/hooks/ ; https://kiro.dev/docs/hooks/types/
- BMAD: https://github.com/bmad-code-org/BMAD-METHOD (skills/bmad-prd/assets/prd-template.md, skills/bmad-architecture/{SKILL.md,assets/spine-template.md,references/reviewer-gate.md}, skills/bmad-code-review/SKILL.md, skills/bmad-project-context/SKILL.md at `61e01f8`; v6.12.4 src/bmm-skills/plan/bmad-sprint-planning/references/readiness-gate.md and bmad-project-context/references/template.md); releases https://github.com/bmad-code-org/BMAD-METHOD/releases
- BMAD TEA: https://bmad-code-org.github.io/bmad-method-test-architecture-enterprise/how-to/workflows/run-nfr-assess ; https://bmad-code-org.github.io/bmad-method-test-architecture-enterprise/explanation/engagement-models ; https://github.com/bmad-code-org/bmad-method-test-architecture-enterprise/releases
- Tessl: https://github.com/tesslio/spec-driven-development-tile (tile.json, docs/spec-format.md, skills/requirement-gathering, skills/work-review, rules/spec-before-code.md at `b8fdff7`); secondary: https://codemyspec.com/blog/tessl-review
- Matt Pocock skills: https://github.com/mattpocock/skills/tree/a496f5513d4a4cf889bc48e189e579f4b0bdf6aa (to-spec, to-tickets, code-review, implement-spec, improve-codebase-architecture)
- Claude Code: https://code.claude.com/docs/en/best-practices ; https://code.claude.com/docs/en/code-review-local ; https://code.claude.com/docs/en/code-review ; https://code.claude.com/docs/en/agent-teams ; https://claude.com/blog/automate-security-reviews-with-claude-code
- Anthropic engineering: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents ; https://www.anthropic.com/engineering/building-effective-agents ; https://www.anthropic.com/engineering/multi-agent-research-system
- OpenAI Codex: https://learn.chatgpt.com/docs/agent-configuration/agents-md ; https://learn.chatgpt.com/docs/third-party/github
- Cursor: https://cursor.com/docs/bugbot ; https://cursor.com/docs/context/rules ; https://cursor.com/docs/agent/plan-mode
- Devin/Cognition: https://docs.devin.ai/product-guides/creating-playbooks ; https://docs.devin.ai/work-with-devin/devin-review.md ; https://docs.devin.ai/onboard-devin/knowledge-onboarding.md ; https://cognition.com/blog/dont-build-multi-agents
- Aider: https://aider.chat/docs/usage/modes.html ; https://aider.chat/docs/usage/conventions.html ; https://aider.chat/docs/usage/lint-test.html ; https://github.com/Aider-AI/aider/releases
- OpenHands: https://docs.openhands.dev/overview/skills ; https://openhands.dev/blog/openhands-product-update---march-2026
- SWE-agent: https://swe-agent.com/latest/ ; https://mini-swe-agent.com/latest/
- GitHub Copilot: https://docs.github.com/en/copilot/concepts/agents/code-review ; https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent
- claude-task-master: https://github.com/eyaltoledano/claude-task-master (README.md, src/prompts/parse-prd.json, src/prompts/analyze-complexity.json); releases page
- Agent OS: https://github.com/buildermethods/agent-os (commands/agent-os/shape-spec.md, inject-standards.md); releases page
- Amp: https://ampcode.com/news/liberating-code-review ; https://ampcode.com/docs/models-and-subagents
- CodeRabbit: https://docs.coderabbit.ai/pr-reviews/pre-merge-checks.md ; https://docs.coderabbit.ai/guides/review-instructions ; https://docs.coderabbit.ai/guides/code-review-overview ; https://docs.coderabbit.ai/integrations/code-graph-analysis (404 on fetch; search snippet only)
- Greptile: https://www.greptile.com/docs/how-greptile-works/graph-based-codebase-context.md ; https://www.greptile.com/docs/code-review/custom-standards (snippet only)
- Qodo: https://docs.qodo.ai/code-review.md
- MetaGPT: https://github.com/FoundationAgents/MetaGPT/blob/main/metagpt/actions/design_api_an.py ; https://github.com/FoundationAgents/MetaGPT/blob/main/metagpt/actions/write_prd_an.py
