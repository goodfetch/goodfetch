# Strand F: published skills for whole-system, cross-cutting architecture

Accessed 2026-10-31 with WebSearch and WebFetch only. No sign-ups, nothing paid.

**The gap this strand checks.** Matt Pocock's skills cover grilling, specs, tickets, implementation, TDD, diff review and module depth. None of them:

- designs the shared hot path every feature runs on, such as one parse per page or grouped database commits;
- owns quality attributes (NFRs) across specs: performance, memory, durability, security, cost;
- sets performance budgets or fitness functions;
- reviews a change against the whole system rather than its diff.

**How the quotes were taken.** WebFetch puts every page through a small model that will not return long passages word for word. Quotes are therefore short extracts that model returned from the named raw file. "VERIFIED" means the file was fetched from its own repository, not from a mirror. Commit SHAs and dates come from the GitHub REST API.

## Verdict

No published skill designs the shared hot path, or owns NFRs across a set of specs, in a form that slots into the Pocock flow. What exists comes in three kinds, and none closes the gap alone:

1. **A written quality bar with enforcement.** Examples: Addy Osmani's `constraint-driven-development` and the `/performance:protect` counter ceilings from melodic-software. This is the nearest thing to fitness functions and budgets.
2. **NFR and trade-off checklists inside a design or ADR prompt.** Examples: Anthropic's knowledge-work `system-design` and `architecture` skills, and the `architecture-critic` agent in `code-modernization`. These are prompt lists, not owners of a budget.
3. **Persona agents that list "scalability, performance, fitness functions" as expertise.** Examples: wshobson and VoltAgent. These are capability lists with no procedure that binds anything.

Whole-system review exists only for security: OpenAI's `security-threat-model` over the whole repository, and Trail of Bits' `differential-review`, which counts callers to size the blast radius. Nothing does the same for performance.

## Top 5 by fit

| Rank | Candidate | Gap it covers | Where in the Pocock flow | Adds a step, or works as a standard? | Conflict |
|---|---|---|---|---|---|
| 1 | Addy Osmani `constraint-driven-development` | Budgets and fitness functions as numbers with commands, ratchets, guarding the bar | Run once to set up, then between runs; the `CONSTRAINTS.md` it writes is read by implementers and by `code-review`'s Standards axis | One setup run; after that it works as a standard that the existing skills read | Low. Its frontend defaults (LCP, CLS, axe) must be replaced with the engine's own counters |
| 2 | melodic-software `performance` plugin (`goal`, `snapshot`, `verify`, `protect`) | Performance budgets as checked-in counter ceilings in CI | A performance ticket's acceptance criteria (`goal`); after a win (`protect`) | Adds a step for performance tickets; the ceiling file becomes the standard | Medium. `climb` hands code edits to its own `/implementation:implement`. Low adoption signal (28 stars) |
| 3 | Anthropic knowledge-work `engineering`: `system-design` and `architecture` | An NFR checklist (scale, latency, availability, cost), explicit trade-offs, "what we'd revisit" | Inside grilling, as a topic checklist; as an ADR consequences section | Reference or checklist only | Low to medium. Its ADR template overlaps `domain-modeling`'s ADRs |
| 4 | Anthropic `code-modernization` `architecture-critic` agent | Adversarial whole-design review: unstated NFRs, tracing one failure end to end | After `to-spec` and before `to-tickets`, or between runs as a whole-system review | Adds a review step (a subagent) | Low. It is tuned to legacy modernization and lives inside a large plugin |
| 5 | OpenAI `security-threat-model` | Whole-repo security: trust boundaries, assets, entry points, abuse paths | Between runs, or before a spec that touches a security surface | Adds a step, triggered by the user | None. It is a Codex skill, but the `SKILL.md` format carries over |

## Candidates

### 1. Matt Pocock's skills: checked for newer skills that plan the whole system

- **Repository:** https://github.com/mattpocock/skills. Head of `main` is `a496f55`, committed 2026-10-29 ("implement-spec: a fresh implementer subagent per ticket"). MIT licence, about 307.8k stars. VERIFIED.
- **Skills on `main`:**
  - `engineering/`: ask-matt, code-review, codebase-design, diagnosing-bugs, domain-modeling, grill-with-docs, implement, implement-spec, improve-codebase-architecture, pr, prototype, research, retro, setup-matt-pocock-skills, tdd, to-spec, to-tickets, triage, wayfinder, wizard.
  - `in-progress/`: chief-of-staff, claude-handoff, loop-me, setup-ts-deep-modules, writing-beats, writing-fragments, writing-shape.
  - `misc/` and `productivity/`: the rest of the catalog.
- **`wayfinder`** (`disable-model-invocation: true`).
  - Description: "Plan a huge chunk of work (more than one agent session can hold) as a shared map of decision tickets".
  - Ticket types are research, prototype, grilling and task. Grilling tickets are told to "Always call the Skill tool twice, for "grilling" and "domain-modeling"."
  - It never mentions architecture, performance, NFRs, quality attributes or budgets. It plans decisions, not system qualities.
  - **Fit:** wayfinder is the natural container for a "hot path" or "NFR budget" decision ticket, but it supplies no content for one.
- **`ask-matt`.** "Ask which skill or flow fits your situation. A router over the skills in this repo." It mentions no performance, NFRs or cross-cutting concerns. Its only architecture route is `/improve-codebase-architecture`.
- **`retro`** (user-only). It suggests improvements to the environment.
  - Its "automated checks" category says mechanical rules should become deterministic checks, and `CODING_STANDARDS.md` keeps the judgment calls.
  - It is the closest Pocock hook for turning a missed cross-cutting failure into a lint or fitness check after the fact. It is not a design step.
- **`improve-codebase-architecture`.** "Scan a codebase for deepening opportunities, present them as a visual HTML report, then grill through whichever one you pick." It covers module depth only: no performance, NFRs or data flow across features.
- **`code-review`.** The Standards axis searches "the repo for every file that documents how code should be written".
  - It names `CODING_STANDARDS.md` and `CONTRIBUTING.md` and includes any other standards documents it finds.
  - **This is the seam:** a repo-level constraints or budgets file is read automatically, with no new step. Review is still scoped to the diff since a fixed point.
- **`in-progress/setup-ts-deep-modules`** (user-invoked).
  - It wires dependency-cruiser so that each TypeScript package is reachable only through its entry points. That is a structural fitness function.
  - It covers TypeScript only, leaves layering "as a stub", and has no performance or quality-attribute content.
- **`in-progress/chief-of-staff`.** "Pursue a long-running goal in a single session by co-ordinating subagents." It covers orchestration and a wiki, with no system qualities.
- **Fit summary:** nothing in Pocock's skills, including `in-progress/`, owns the gap. Two hooks exist: `code-review` reads any standards file, and `retro` turns rules into checks.

### 2. Anthropic knowledge-work-plugins, `engineering` plugin

- **Repository:** https://github.com/anthropics/knowledge-work-plugins. The last commit touching `engineering/` is `c71b6f3`, dated 2026-09-21. Apache-2.0, about 34.1k stars.
- **Install:** `claude plugin marketplace add anthropics/knowledge-work-plugins`, then install `engineering`. VERIFIED.
- **Skills:** architecture, code-review, debug, deploy-checklist, documentation, incident-response, standup, system-design, tech-debt, testing-strategy. None sets `disable-model-invocation`.
- **`system-design`.**
  - Description: "Design systems, services, and architectures. Trigger with "design a system for", "how should we architect"…".
  - Framework: Requirements Gathering, High-Level Design, Deep Dive, Scale and Reliability, Trade-off Analysis.
  - Key lines: "Non-functional requirements (scale, latency, availability, cost)"; "Every decision has trade-offs. Make them explicit."; "Load estimation"; "Always identify what you'd revisit as the system grows."
- **`architecture`.** "Create or evaluate an architecture decision record (ADR)…". It treats non-functional needs (latency, cost, maintenance burden) as equal to features. Its ADR template has "[What we'll need to revisit]".
- **`code-review`.** "Review code changes for security, performance, and correctness." It lists "N+1 queries", "Algorithmic complexity (O(n²) in hot paths)" and "Unbounded queries or loops", but it reviews only "the provided code changes".
- **`tech-debt`.** "Identify, categorize, and prioritize technical debt." It gives a scoring formula and touches performance only through "Scaling limits".
- **Fit:**
  - `system-design`'s NFR list fits inside grilling as a topic checklist. It is a prompt list, generic and web-service shaped.
  - It replaces no Pocock step, but its ADR shape competes with `domain-modeling`'s ADRs.
  - The plugin has nothing that persists budgets across specs.

### 3. Anthropic claude-plugins-official

- **Repository:** https://github.com/anthropics/claude-plugins-official. Apache-2.0, about 45.5k stars, pushed 2026-10-31. VERIFIED.
- **Plugins:** 40 under `plugins/`. None is named for performance, system design or NFRs.
- **`code-modernization/agents/architecture-critic.md`.** The last commit touching the plugin is `c8874f5`, dated 2026-09-25.
  - Description: "Reviews proposed target architectures and transformed code against modern best practice." It is adversarial. Tools: Read, Glob, Grep, Bash.
  - Key lines: "Which non-functional requirements (latency, throughput, consistency) are unstated"; "What happens when service X is down? Trace one failure mode end-to-end."; "What's the simplest design that meets the stated requirements?"
  - **Fit:** the best published "review the design, not the diff" prompt. Use it on a spec before `to-tickets`, or on the whole system between runs. It adds a subagent step and is tuned to modernization.
- **`feature-dev/agents/code-architect.md`** (model: sonnet).
  - "Design for testability, performance, and maintainability."
  - "Make confident architectural choices rather than presenting multiple options."
  - **Fit:** a feature-scoped blueprint. Its single-option stance conflicts with grilling, which lays out options.
- **`claude-security`** (scan agents, SARIF) and **`security-guidance`.** Security only. Contents were not read in depth.
- **`anthropics/skills`.** Its 19 entries (docx, pdf, frontend-design, skill-creator and others) include no architecture or NFR skill. VERIFIED, as a listing.

### 4. obra/superpowers

- **Repository:** https://github.com/obra/superpowers. Head `2ce8aa1`, 2026-10-21 (v7.0.1). VERIFIED.
- **Skills (15):** brainstorming, writing-plans, executing-plans, subagent-driven-development, requesting-code-review, test-driven-development and others. None covers NFRs or performance.
- **`brainstorming`.** The design document has a "constraints" section, and the skill flags "a constraint, a conflict, or a cheaper path". It ends with "invoke superpowers:writing-plans and no other skill."
- **`writing-plans`.** The plan header has "**Architecture:** [2-3 sentences about approach]" and "Global Constraints" (version floors, dependency limits, naming rules, platform requirements). It has no performance budgets.
- **Fit:** none for the gap. It **conflicts**, because brainstorming → writing-plans → executing-plans is a parallel pipeline to grilling → to-spec → to-tickets → implement-spec.

### 5. wshobson/agents

- **Repository:** https://github.com/wshobson/agents. Head `7e2fb55`, 2026-10-30. MIT, about 42.0k stars.
- **Install:** `/plugin marketplace add wshobson/agents`. Size: 94 plugins, 202 agents, 185 skills. VERIFIED for the files named below.
- **`application-performance/agents/performance-engineer.md`** (model: inherit).
  - Lines: "Sets and enforces performance budgets to prevent regression"; "Measures performance comprehensively before implementing any optimizations"; "Create performance monitoring dashboard with SLI/SLO tracking".
  - It is mostly a persona and capability list, with a nine-step "Response Approach".
- **`comprehensive-review/agents/architect-review.md`** (model: opus).
  - Lines: "Reviews system designs and code changes for architectural integrity, scalability, and maintainability."; "Use PROACTIVELY for architectural decisions."; "Assess architectural impact of proposed changes (High/Medium/Low)".
  - It is a persona with a light procedure.
- **Other plugins:** `backend-development` has the backend-architect agent and skills such as `architecture-patterns`, `microservices-patterns` and `saga-orchestration`. `c4-architecture`, `performance-testing-review` (commands `ai-review` and `multi-agent-review`) and `observability-monitoring` were not read: UNVERIFIED.
- **Fit:** reference material at most. These are expertise lists, and a "budget" there is a trait, not an artifact. The `feature-development` command would compete with `implement-spec`.

### 6. VoltAgent/awesome-claude-code-subagents

- **Repository:** https://github.com/VoltAgent/awesome-claude-code-subagents. Head `f2e2ea7`, 2026-10-30. MIT, about 26.2k stars. Its own README says the subagents are provided "as is", with no audit of security or correctness.
- **`categories/04-quality-security/architect-reviewer.md`** (model: inherit; tools include Write and Edit). VERIFIED.
  - "Always prioritize long-term sustainability, scalability, and maintainability".
  - An "Evolutionary architecture:" list includes "Fitness functions", and there is a "Performance architecture" checklist.
- **Other files:** `performance-engineer.md`, `sre-engineer.md` and `chaos-engineer.md` exist but were not read.
- **Fit:** a checklist persona. Fitness functions are named, never designed or run.

### 7. Addy Osmani, agent-skills (strongest single match)

- **Repository:** https://github.com/addyosmani/agent-skills. Head `8bfcc1f`, 2026-10-31 ("bump plugin manifests to 0.6.15"). MIT, about 112.1k stars.
- **Shape:** 25 skills over a Define → Plan → Build → Verify → Review → Ship lifecycle, with `/spec /plan /build /test /review /ship` commands. VERIFIED for the files below.
- **`constraint-driven-development`.** The last commit on it is `ba3d99e`, committed 2026-10-29. Frontmatter has only `name` and `description`, so the model can invoke it.
  - Description: "Establishes a project's quality bar as a written contract and stops agents quietly lowering it." It "records everything in CONSTRAINTS.md, and watches the diff for a weakened bar".
  - It writes `CONSTRAINTS.md` at the repository root with three tables:
    - enforced: Dimension, Rule, Checked by, Runs at;
    - measured, not yet enforced: Metric, Today, Direction;
    - exceptions: ID, Rule, Path, Reason, Owner, Expires.
  - It adds the line "Read CONSTRAINTS.md before writing code. Do not weaken it to make a change pass." to AGENTS.md and CLAUDE.md.
  - Key lines: "A dimension with a number and no command in this column is an aspiration, not a constraint."; "Tightening the bar should be silent; loosening it should be loud."; "record where you are, then refuse to get worse."; "Block on the floor, warn on everything else for the first two weeks."
  - Steps: detect, then four questions, write the file, install tools, wire to the lifecycle, guard the bar, ratchets.
  - **Fit:** the published skill closest to "fitness functions plus budgets as a durable artifact".
    - The setup is a one-time step, or a between-runs step.
    - The file it writes then works as a standard: implementers read it through AGENTS.md, and Pocock's `code-review` Standards axis reads it with no change to that skill.
    - Its defaults (LCP ≤ 2500 ms, CLS ≤ 0.1, axe, coverage, mutation score) are web-app shaped. A crawler engine would replace them with its own counters, such as parses per page and commits per batch.
    - Its "Runs at" column names Addy's BUILD/VERIFY/REVIEW/SHIP phases, which map onto the repository's own gates.
    - It does not design the hot path. It records and enforces the numbers once someone has designed it.
- **`performance-optimization`.**
  - "Measure before optimizing."; "Set budgets and enforce them:" (for example API p95 < 200 ms, JS < 200 KB gzipped); "Catch reproducible regressions before merge with a performance budget."; "One query per row is the most common backend bottleneck."
  - **Fit:** reference for a performance ticket. It is web-leaning.
- **`doubt-driven-development`.** Fresh-context adversarial review: "Pass ARTIFACT + CONTRACT only. Do NOT pass the CLAIM." It scopes to a small unit, "the diff or the function — not the whole file", so it is not whole-system.
- **`code-review-and-quality`.** Five axes: correctness, readability, architecture, security, performance. It works "For each file changed", so it is diff-scoped.
- **`spec-driven-development`.** Its template has no NFR section, and its own SPECIFY → PLAN → TASKS → IMPLEMENT flow **conflicts** with to-spec and to-tickets.
- **`observability-and-instrumentation`.** It mentions SLOs only for alert thresholds.
- **`addyosmani/web-quality-skills`.** A separate repository: MIT, about 3.0k stars, pushed 2026-08-24. Lighthouse and Core Web Vitals skills, frontend only, not relevant to an engine. Its repository metadata was checked, but its skills were not read.

### 8. melodic-software/claude-code-plugins, `performance` and `architecture` plugins

- **Repository:** https://github.com/melodic-software/claude-code-plugins. MIT, **28 stars**, pushed 2026-10-31. The last commit touching `plugins/performance` is `3eeb073`, 2026-10-31. Very active, with 6,500+ commits, but little adoption. VERIFIED.
- **`performance` plugin:** skills `target`, `goal`, `snapshot`, `climb`, `verify`, `protect`, `go-faster`. Its README says: "A workflow that measures without enforcing these rules mostly generates confident numbers, which is worse than generating none."
  - **`protect`** (`user-invocable: true`, `disable-model-invocation: false`). A "Checked-in CI ceiling on the counter of a performance win … a regression fails the build".
    - "Counters only, never durations."
    - "A duration ceiling moves with runner load, fails at random, and teaches people to raise it."
    - "The ceiling is the measured value, never a round number above it."
    - "Raising one is a decision, stated with its reason in the PR body".
  - **`target`.** It ranks candidates by evidence tier (E1 to E4): "Never pick a target because its mechanism sounds expensive." With no evidence, the top recommendation is "instrument this first".
  - **`goal`** (from the README). A human-gated step that sets the metric, the exact command, and realistic and ideal targets before work starts.
- **Fit:**
  - `goal` and `protect` are the clearest published design of a deterministic performance fitness function, such as parses per page or commits per batch held as a counter ceiling in CI.
  - Use them as a reference or standard. Running `climb` adds its own implementation loop, which **conflicts** with implement-spec.
  - The adoption signal is weak.
- **`architecture` plugin.** It uses a deep-module lens (Ousterhout), `record-decision`, and C4 `map-*` skills with a `--check` CI mode. It is descriptive and structural, with no NFRs. Its README frames it as "module-level architecture friction". It overlaps Pocock's `improve-codebase-architecture`.
- **Not on `main`:** mirrors such as agent-skills.md and skills.sh list `systems-design/quality-attributes-taxonomy` and `fitness-functions`. `plugins/systems-design` does not exist on `main`, and the API returns no commits for that path. Treat it as **stale or removed: UNVERIFIED**.

### 9. Trail of Bits skills: security, whole-change blast radius

- **Repository:** https://github.com/trailofbits/skills. Head `2be6433`, 2026-10-28. **CC-BY-SA-4.0**, about 7.9k stars. 46 plugins. VERIFIED for the two below.
- **`differential-review`** (allowed-tools: Read Write Grep Glob Bash; model-invocable).
  - "Performs security-focused differential review … calculates blast radius by counting callers".
  - Lines: "Calculate blast radius early to prioritize"; "You'll miss transitive callers"; "High blast radius (50+ callers) + HIGH risk change".
  - **Fit:** a diff-anchored review that reaches out to callers, for security only. The owner's existing `blast-radius` skill already covers the general case.
- **`sharp-edges`.** "Identifies error-prone APIs, dangerous configurations, and footgun designs". It explicitly excludes performance. **Fit:** a design-level security review of settings and API surfaces, relevant to "every feature a setting".
- **Licence note:** ShareAlike applies if the text is vendored.

### 10. OpenAI skills (Codex): `security-threat-model`

- **Repository:** https://github.com/openai/skills. About 28.4k stars, and the API shows no top-level licence (per-skill licence UNVERIFIED). The skill was added in `5c8f1e2` on 2026-02-02. VERIFIED.
- **What it says:**
  - "Repository-grounded threat modeling that enumerates trust boundaries, assets, attacker capabilities…"
  - "Trigger only when the user explicitly asks to threat model…"
  - "Anchor every architectural claim to evidence in the repo and keep assumptions explicit."
  - "Pause and wait for user feedback before producing the final report."
- **Scope:** the whole repository or a subpath. It is the only published whole-system review skill found, and it covers security only.
- **Fit:** between runs, or before a spec touching an exposed surface (MCP server, Gateway, Sinks). It adds a user-triggered step and does not conflict.
- **Other curated skills:** `security-best-practices` and `security-ownership-map` exist but were not read.

### 11. Vercel agent-skills

- **Repository:** https://github.com/vercel-labs/agent-skills. VERIFIED, as a listing.
- **Skills:** composition-patterns, react-best-practices, react-native-skills, react-view-transitions, vercel-optimize, web-design-guidelines, writing-guidelines and deploy skills.
- **Fit:** all framework- or platform-specific performance rules. None owns system NFRs. Not relevant beyond the React wrappers.

### 12. Community "fitness function", "quality attributes" and "NFR" skills found through directories

All of these surfaced through mirrors (claudskills.com, skills.lc, agent-skills.md). None checked out at its claimed source, or none has any adoption:

- **`9uiLe/plugins` `quality-architecture`.** ISO/IEC 25010:2023 attribute prioritization with ATAM-style trade-offs; MIT. The repository exists (0 stars, pushed 2026-10-21), but the raw `SKILL.md` at the mirror's path returns 404. UNVERIFIED.
- **`Xopoko/plug-n-skills` `architecture-fitness-functions`.** "Use when architecture choices need executable guardrails". The repository returns 404. **Does not exist (any more).**
- **`reiserwang/Coding_Agent` `architecture-design`.** C4, ADRs, and `evaluate` scoring against five fitness functions. The repository returns 404. **Does not exist (any more).**
- **`jabrena/plinth` `032-architecture-adr-non-functional-requirements`.** An ISO 25010 NFR ADR with "measurable Quality Metrics". The repository exists (Apache-2.0, 457 stars, pushed 2026-10-31), but `skills/` on `main` has no `032-` entry: 124 entries, including `030-architecture-adr-general` and `033-architecture-diagrams`. UNVERIFIED, probably renamed or removed. Java-enterprise oriented.
- **Others:** "alexei-led methodology-architecture-fitness", frostaura `performance-budgeting` and mohitagw15856 `performance-budget` appeared only as directory listings. No source repository was confirmed. UNVERIFIED.

### 13. Reference points (already surveyed elsewhere): Spec Kit and BMAD

- **GitHub Spec Kit `/checklist` and `/analyze`.** The upstream README speaks of "skills" invoked per agent through an integrations guide. It does not state a Claude `.claude/skills` layout.
  - The community port `dceoy/speckit-agent-skills` packages `speckit-analyze` and `speckit-checklist` as `SKILL.md` for Claude and Codex. It is **AGPL-3.0**, has 161 stars, and was pushed 2026-10-29.
  - Installable as skills, yes, through that port. It brings the Spec Kit pipeline with it, which conflicts with to-spec and to-tickets.
- **BMAD TEA `nfr-assess`.** Installed through `npx bmad-method install` (choose TEA), which writes `.claude/commands/`. Search results say newer TEA renames it `bmad-tea-testarch-nfr`, but the migration page returned 404: UNVERIFIED.
  - It is not a standalone `SKILL.md`. It needs the BMAD agent and workflow runtime, and it conflicts with the Pocock flow.

## Not found, despite being widely mentioned

- **No skill named or shaped as an "ATAM" or "utility tree" run.** Every hit was a mirror listing with no live source.
- **No published design-doc skill with a "cross-cutting concerns" section,** in the Google design-doc style.
- **No skill that owns a set of NFRs across several specs,** or reviews a change against the whole system for performance.

## What this means for the Pocock flow

These are options, not adopted decisions.

1. **As a standard (no new process step).**
   - A repo-level constraints file in the shape of Addy's `CONSTRAINTS.md`. Each dimension has a number and a command. It carries counters only, in the style of melodic `protect`, plus ratchets and dated exceptions.
   - Pocock's `code-review` Standards axis already reads such a file, and implementers read it through an AGENTS.md pointer.
2. **Inside grilling, as a topic checklist.**
   - The NFR list from Anthropic's `system-design` (scale, latency, availability, cost, load estimation) and `architecture-critic`'s "unstated NFRs" and "trace one failure end-to-end".
   - It is reference text a grilling session reads, not a new step.
3. **Between runs.**
   - A whole-system review. `architecture-critic` as a subagent covers design, and `security-threat-model` covers security.
   - Pocock's `retro` covers turning missed failures into checks.
4. **What is still uncovered.**
   - No published skill designs the shared hot path itself.
   - That would remain a repository-local standard or ADR (one parse per page, grouped commits) enforced through item 1.
