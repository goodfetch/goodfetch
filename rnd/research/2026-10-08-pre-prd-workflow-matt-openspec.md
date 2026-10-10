# Going from the lab to PRD v2: how to use Matt Pocock's skills and OpenSpec

**Date:** October 8, 2026.

**Update of October 8:** OpenSpec was removed from the repository after this study. Only Matt Pocock's skills are used. The sections on OpenSpec remain as a record of the comparison.

**Question:** Automan has almost no product code. There is research, two ADRs, a glossary and a measurement laboratory. What do Matt Pocock's skills and OpenSpec recommend for arriving at a solid PRD, then at implementation?

**Sources:**
- The `SKILL.md` files installed in `~/.agents/skills/`. Compared on October 8 with `mattpocock/skills@main`, they are identical.
- The documentation pages `https://github.com/mattpocock/skills/blob/main/docs/...`.
- OpenSpec 1.14.1: `docs/` and `docs-lab/` from the `Fission-AI/OpenSpec` repository (tag `v1.14.1`), the built-in `spec-driven` schema, and the skills generated in `.claude/skills/openspec-*`.
- The state of the repository on `origin/main` (`8d518cd`).

Unless otherwise stated, quotations are short excerpts from the sources. Passages marked **[Interpretation]** give my analysis.

---

## 1. Key takeaways

1. **The two tools do different things, at the same moment.**
   - In Matt Pocock's set, the PRD is now called a **spec**: `to-prd` was renamed `to-spec`. His skills go from the vague idea to the spec, then to tickets.
   - OpenSpec explicitly says it is not a PRD tool: "A 40-page PRD is a different artifact with a different job" (`docs/existing-projects.md`). An OpenSpec spec is a **per-change behavior contract**. It only makes sense once implementation has started.
2. **Before the PRD, Matt's skills lead.** OpenSpec brings almost nothing at this stage, except `/opsx:explore`, which duplicates grilling.
3. **The right entry point is `/wayfinder`.** It is made for an effort "too big for one session" where "the route… [is] still unclear". It stops when the map is clear and hands over to `/to-spec #<map>` (`ask-matt/SKILL.md`, "On-ramps" section).
4. **Your existing tools already do part of wayfinder's work.** The `lab/decisions/D-*.json` registry, with its computed verdicts, covers a good part of what a wayfinder map provides. A second registry must not be created: the map must **point** to the lab's decisions, not copy them (§4).
5. **Three points must be settled before starting** (§3):
   - PRD v1 was produced by an obsolete skill;
   - the phase tickets are labeled `ready-for-agent` even though they fit on one line;
   - the `wayfinder:*` labels do not exist on GitHub.
6. **The critical path is the product, not the benchmarks.**
   - `D-pricing` and `D-feature-priority` depend on `D-segment`.
   - No interview has taken place (#11).
   - A PRD v2 without a chosen segment would have the same flaw as v1: 80 user stories, with no priority at all.

---

## 2. The path each tool lays out

### Matt Pocock

Source: `ask-matt/SKILL.md`, `docs/engineering/*.md`.

```
[mapping: /wayfinder] → /grill-with-docs → (/prototype detour) → /to-spec → /to-tickets → /implement | /implement-spec → /retro
```

| Skill | What it does | What it produces | Role for Automan now |
|---|---|---|---|
| `wayfinder` | Decision map: "producing decisions, not deliverables". A single decision settled per session (except research). | A `wayfinder:map` issue and `wayfinder:{research,prototype,grilling,task}` sub-issues, with native GitHub blocking | **Entry point** |
| `grill-with-docs` | Interview that asks all the pending questions at once. Facts are for the agent to find; decisions belong to the user. | `GLOSSARY.md` and ADRs, created along the way. The rest lives only in the conversation. | Settle the `grilling` tickets, then fill the last gaps before the spec |
| `research` | Background agent that looks for an **external fact** in primary sources | A sourced Markdown file | `research` tickets |
| `prototype` | Answers **one** design question. Throwaway, without tests. | A `prototype/<name>` branch | Rare: your lab goes further than a prototype (§4) |
| `to-spec` | **Interview-free** synthesis of the conversation and the map. It first proposes the test points ("seams"). | **A GitHub issue**: problem, solution, user stories, implementation decisions, tests, out of scope | **Produces PRD v2** |
| `to-tickets` | Splits into vertical slices, one per context window, with blocking relationships and sub-issues | `ready-for-agent` issues | After PRD v2 |
| `domain-modeling` | An ADR is only justified if the decision is hard to undo, surprising without context **and** the result of a real trade-off | `docs/adr/NNNN-*.md`, status `proposed`/`accepted`… | One ADR per settled decision |

**Points from the docs to respect:**
- **A single context window.** Chain grill, to-spec and to-tickets without compacting or clearing the context, within a limit of about 150k tokens.
- **The spec is disposable** once the work is delivered: "Your `GLOSSARY.md` and ADRs are the files meant to last".
- **Known wayfinder mistakes** (`docs/engineering/wayfinder.md`):
  - The agent writes production code from the map, for example by noting "this map carries execution" in its Notes.
  - A 27-ticket map that ends up going stale. The advice: "scope the map to a bounded destination… Wayfinder is 'prototypemaxxing', not 'planmaxxing'".
- **Known grill-with-docs bug.** When launched inside another spec orchestrator, it does not write its files, and does not report it. It must therefore not be launched inside `/opsx:explore`.
- **`ready-for-agent` label set by `to-spec`.** An autonomous agent watching this label would try to implement the whole spec. The label must be removed after `/to-tickets`.

### OpenSpec 1.14.1

Sources: `docs/concepts.md`, `docs/workflows.md`, `docs/existing-projects.md`, `schemas/spec-driven/schema.yaml`.

- **The model:**
  - `openspec/specs/` describes the current behavior of the system.
  - `openspec/changes/<name>/` contains a proposed change: `proposal.md` → `specs/` (`ADDED/MODIFIED/REMOVED/RENAMED` deltas) + `design.md` → `tasks.md`.
  - Archiving merges the deltas into `specs/`.
- **What a spec does not contain:** no library choice or internal detail. "If the implementation can change without changing externally visible behavior, it likely does not belong in the spec". Technical choices go in `design.md` and in the ADRs.
- **Project without code (greenfield):**
  - `specs/` starts empty and fills up as changes come in: "Resist the urge to back-fill everything".
  - Existing PRDs and research are **raw material for explore**, not specs to convert.
  - In 1.14.1, archiving does create the main spec of a new capability (fix for bug #1222).
- **Before the PRD:** `/opsx:explore` ("when you're not sure, explore before you propose").
  - It writes nothing without your agreement.
  - `/opsx:onboard` is a tutorial. It looks for a small coding task and is of no interest here.
- **Custom schema:** one can be forked, for example `research-first` (research → proposal → tasks). But a `prd` or `research` artifact is **never merged** into `specs/`.
- **Stores (beta):** planning outside the repository or across multiple repositories. Useless for Automan.
- **Task tracking:** no integration with a ticketing tool. The convention is one change = one branch = one PR.

**[Interpretation]** After the PRD, OpenSpec brings what Matt's set lacks: a **durable behavior spec**. Matt's spec is a disposable snapshot, whereas OpenSpec keeps the truth up to date in `openspec/specs/`. The two tools complement each other, provided each has its own role (§5).

---

## 3. What in the repository slows down the path

| # | Problem | Source | Proposed fix |
|---|---|---|---|
| 1 | `issues/prd.md` (PRD v1) has the format of `write-a-prd`. This skill comes from one of Matt's workshop repositories (`ai-engineer-workshop-2026-project`) and is **superseded** by `to-spec`. It writes `issues/*.md` and forbids GitHub. | `~/.agents/.skill-lock.json` | Mark v1 "superseded — source material for v2". Stop using `write-a-prd` and `prd-to-issues`. |
| 2 | PRD v1 freezes Rust, React/shadcn, macOS and the "Crawlee+Spider+Stagehand" scope before the corresponding decisions. `D-core-language` is still open. | `issues/prd.md:9`, `lab/decisions/D-core-language.json` | v2 must follow from the verdicts, not the other way around. |
| 3 | Three possible places for specs: GitHub Issues (`AGENTS.md`), `issues/prd.md` and `openspec/specs/`. | `AGENTS.md`, `openspec/` | Assign them explicitly (§5). |
| 4 | Issues #6 to #9 fit on one line but carry `ready-for-agent`. | `gh issue list` | Remove the label: these are lab workstreams, not fully specified tasks. |
| 5 | The labels `wayfinder:map`, `:research`, `:prototype`, `:grilling` and `:task` do not exist. The setup skill does not create them. | `docs/engineering/setup-matt-pocock-skills.md` | Create them before the first run. |
| 6 | `D-storage` is "decided" without a verdict or ADR. ADR-001 is "proposed" even though `D-core-language` has no valid verdict. | `lab/decisions/D-storage.json`, `docs/rd-plan.md:76` | Align before mapping. |
| 7 | `roadmap-validation.md` starts with "console + API", whereas `rd-plan.md` (approved) starts with the lab and the decisions. | `docs/roadmap-validation.md:9` | Flag that `roadmap-validation.md` is outdated, or align it. |
| 8 | The interview guide announced by `rd-plan.md:71` is not in the repository. | `research/market/pricing-signals.md` §5 | It is the first product deliverable to produce. |

---

## 4. Making wayfinder and the lab fit together

**[Interpretation]** Your lab does not correspond to any of Matt's skills:
- it is not a `prototype`, which would be disposable and without tests;
- building the lab is "doing", which goes against wayfinder's "Plan, don't do" rule.

The cleanest separation is the following.

| Layer | Role | Where it lives |
|---|---|---|
| **Decisions** | What must be settled before PRD v2 | Wayfinder map: a `wayfinder:map` issue and one sub-issue per open `D-*` decision |
| **Evidence** | Measurements and computed verdicts | `lab/decisions/`, `lab/verdicts/`, `lab/reports/`. **Remains the source of truth.** |
| **Lab construction** | Harness phases P5 to P9 | Ordinary issues (#5 to #9), handled **outside** wayfinder, in normal sessions |

**The wayfinder map:**
- **Destination, bounded:** "Automan PRD v2 for the chosen segment, each blocking decision settled by a verdict or an ADR".
- **Notes:** "The lab campaigns run outside the map. A ticket is resolved when `lab/verdicts/D-x.json` is eligible or an ADR is accepted. The map carries no execution."
- **Tickets, by type:**
  - `grilling`: `D-segment` first, then `D-pricing` and `D-feature-priority`, blocked by `D-segment`. Also the decisions that get settled through discussion: `D-ai-layer`, the scope of `D-identity`, `D-agent-interface` on the product side.
  - `task` (with a human): the JTBD interviews (#11), the price test, the paid PoC. The technical tickets whose verdict awaits a lab campaign (`D-browser-driver`, `D-core-language`, `D-http-transport`, `D-durable-runtime`…) point to their `D-*.json` file, without copying it.
  - `research`: the external facts still missing. For example, the legal conditions of ADR-002, or up-to-date competitor pricing.
- **Blocking:** native dependencies between issues mirror the `dependsOn` of the `D-*` registry.
- **Size:** about 15 decisions, below the 27 tickets cited as the breaking point. If a technical decision does not influence the PRD (storage, sinks…), it goes into "Out of scope", not into a ticket.

**Resolving a ticket:**
1. A session settles **one** decision: grilling, or reading a verdict.
2. `domain-modeling` writes the ADR if the three conditions are met (hard to undo, surprising, real trade-off). The status of the `D-*.json` is updated at the same time.
3. A comment is posted on the ticket, the ticket is closed, then a line is added to "Decisions so far".

---

## 5. Dividing roles between Matt and OpenSpec

| Phase | Tool | Authoritative artifact |
|---|---|---|
| Research and measurement (today) | `wayfinder`, `grill-with-docs`, `research`, plus the lab | GitHub map, `lab/decisions/`, ADRs, `GLOSSARY.md` |
| PRD v2 | `/to-spec #<map>` in the **same window** as a final `grill-with-docs` | **GitHub issue**: PRD v2. `issues/prd.md` is archived as a source. |
| Splitting | `/to-tickets` | GitHub sub-issues (remove `ready-for-agent` from the PRD) |
| Implementation, per ticket | `/opsx:propose` (or `new`/`continue` at the start), then `/opsx:apply`, then `/opsx:archive` after merge | `openspec/changes/` during the work, `openspec/specs/` afterwards. The ticket and the PR point to the change. |

**[Interpretation]** Why this split:
- Both tools agree: the PRD is not an OpenSpec spec.
- Matt's spec is disposable, whereas `openspec/specs/` lasts.
- Each ticket becomes an OpenSpec change, which gives a three-step review: proposal, delta, code (`docs/team-workflow.md`).
- Matt's `/implement` and `/opsx:apply` overlap. **Choose one.** I recommend `/opsx:apply`, which keeps the record in the spec. The trade-off is that TDD at the test points is lost, unless you add `rules.tasks` in the config.

**Possible additions to `openspec/config.yaml` later** (short, as the docs recommend):
- `rules.proposal`: cite the ticket's issue, as well as the lab evidence or the ADR that justifies the change.
- `rules.design`: cite the ADR identifier; a `proposed` ADR commits to nothing.
- `rules.tasks`: each group starts with a test at the test point agreed in the spec.

**Simpler variant:** do everything with Matt (`to-tickets` → `implement`), without OpenSpec. You only lose the durable behavior spec. Choose this variant if you do not want to maintain `openspec/specs/`.

---

## 6. Action plan

1. **Cleanup, in one session:**
   - create the `wayfinder:*` labels;
   - remove `ready-for-agent` from issues #6 to #9;
   - retire PRD v1 `issues/prd.md` (deleted on October 8);
   - record the §5 split in `AGENTS.md`;
   - fix `D-storage` (verdict or ADR).
2. **`/wayfinder`** with the destination from §4, in one session. The first grilling, breadth-first, must confirm that fuzzy areas really remain. Otherwise, go directly to step 4.
3. **Resolve the map, one ticket per session**, starting with `D-segment`.
   - Prepare the interview guide. `/to-questionnaire` can draft it from `research/market/`.
   - Meanwhile, the lab advances P6 to P9 in separate sessions.
4. **When the map is clear,** chain in a single context window: `/grill-with-docs` (last gaps), then `/to-spec #<map>` (PRD v2), then `/to-tickets`.
5. **Implement** ticket by ticket with OpenSpec (or `/implement`), then `/retro`.

## 7. Limitations of this analysis

- Part of the OpenSpec 1.14.1 docs on openspec.dev are only page outlines: `explore`, `existing codebases`, the FAQ. `https://openspec.dev/docs/existing-projects` returns a 404. I cited the GitHub `docs/` version instead.
- Matt's `in-progress` skills (`chief-of-staff`, `claude-handoff`, `loop-me`…) can change at any time. This plan does not rely on them.
- Matt's docs advise against versioning research files ("ADRs yes. Everything else archive or delete"). Automan chose to keep `research/`. This choice remains valid, but no skill automatically rereads these files: they must be given explicitly to the grilling.
