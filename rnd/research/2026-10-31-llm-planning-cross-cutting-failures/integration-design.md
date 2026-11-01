# Owning system qualities inside the Matt Pocock workflow: integration design

Proposal, 2026-10-31, for the owner. It follows `report.md` (sections 5, 7 and 11). A3 is written and installed globally as `~/.agents/skills/system-qualities/`, which supersedes section 3's outline. Its user-level hooks (`UserPromptExpansion` and `PreToolUse` on `Skill`, script `hooks/pocock-step-context.py`) and the global `AGENTS.md` pointer are in place too; the Goodfetch pieces are not adopted.

**Goal.** The Pocock skills keep the lead: `wayfinder` or `grill-with-docs`, then `/to-spec`, `/to-tickets` and `/implement-spec`. The project as a whole gets an owner for its system qualities (shared hot paths and their per-unit cost; memory, durability, security and cost). Additions are triggered automatically at the right step, without editing the Pocock skills and without a new step in their sequence.

## 0. Generic, not Goodfetch-specific

The mechanism applies to any project the owner builds with the Pocock skills. The Goodfetch examples (page, Collie action, JSON-RPC message, ledger charge) are one application of it.

| Layer | Lives in | Applies to |
|---|---|---|
| The `system-qualities` skill, its setup mode, and the formats of the paths ADR and of the register | `~/.agents/skills/` | Every project |
| Hooks that load the skill at `/wayfinder`, `/grill-with-docs`, `/to-spec`, `/to-tickets` and `code-review` | `~/.claude/settings.json` (user level) | Every project |
| The pointer to the skill | The owner's global `AGENTS.md`, section "My skills inside Pocock workflows", beside `code-quality-development` | Every project |
| Which shared paths exist and their unit. Examples: per request or query (web app), per file (CLI), per record (data pipeline), per frame (game), per page, action or message (Goodfetch) | The project's ADR, decided by its first architecture grilling | One project |
| The register's numbers and commands, and the gate | The project's testing standard and its CI or merge command | One project |

- **Setup mode.** Run when the owner asks: it grills the paths ADR, writes the register section and the `docs/agents/domain.md` and `AGENTS.md` pointers. A project with no shared path ends after one question and one line. Before Setup, the skill still runs its path check from the code and reports the paths it finds as open questions.
- **Repository rules.** Repository-specific rules such as Goodfetch's ADR 0133 still need their own amendment (section 8). Elsewhere the global `AGENTS.md` already allows additional skills inside Pocock workflows.

## 1. The five additions

| # | Addition | What it is | Why |
|---|---|---|---|
| A1 | **Hot-path ADR** (next free number after 0139) | A living ADR: an inventory of every per-unit path, with its unit, owning module and cost contract (counted work per unit). ADR 0138 is the per-page part. | The only artifact every spec reads (report §5). It is the question the four wayfinder maps never asked. |
| A2 | **Quality register in `TESTING.md`** | One section in three tables, after Addy Osmani's `constraint-driven-development`: **Enforced** (Dimension, Rule, Checked by, Runs at), **Measured** (Metric, Today, Direction), **Exceptions** (ID, Rule, Path, Reason, Owner, Expires). It replaces today's scattered budget text. | One standard in one file. `code-review`'s Standards axis already reads `TESTING.md`. |
| A3 | **A short custom glue skill, `system-qualities`** (one page) | It does only what no published skill does: read the project's paths ADR and register, list the paths a piece of work touches, and ask what it adds per unit (section 3). It **calls** the existing skills instead of copying them: Anthropic's `system-design` for the global architecture grilling, and Addy's `constraint-driven-development` for the register. It follows melodic's counter rule ("Counters only, never durations"). | It turns "ask the question" into something an agent cannot skip. |
| A4 | **Triggers** | (a) One line in `docs/agents/domain.md`, the file Pocock's skills read "before exploring". (b) Claude Code hooks in `.claude/settings.json` that load A3 at the right skill. | (a) is Pocock's own seam. (b) is deterministic: Claude Code's docs say rules that must hold every time belong in hooks, not in `CLAUDE.md`. |
| A5 | **The gate** | `xtask perf` reads A2's Enforced rows, and `just merge` fails on a breach or on an expired exception. | Without enforcement, a decision drifts (ADR 0134's informational ceilings). |

## 2. When each piece acts, step by step

| Moment | What triggers it | What happens | Output |
|---|---|---|---|
| **Once, now** | The owner starts it | A grilling on "what a page, a Collie action, a JSON-RPC message, an Audit entry and a ledger charge go through, and what each may cost". Run it through `wayfinder` if it is too large for one session, with the map's **destination** saying so. Then `constraint-driven-development` runs once, patched to write into `TESTING.md` instead of `CONSTRAINTS.md`. | A1, and A2's first rows |
| **A new wayfinder map** | Hook on `/wayfinder` loads A3 | A3 adds a grilling ticket, "Which shared paths does this effort touch, and does the hot-path ADR change?". It blocks the tickets that touch those paths, and A3 is listed in the map's **Notes** ("skills every session should consult": wayfinder's own field). | A map that cannot skip the question again |
| **Each feature grilling** | Hook on `/grill-with-docs` or the wayfinder grilling ticket; `domain.md` says to read A1 | A3 adds its questions. Which paths does the feature run on? What does it add per unit? Does it need a path A1 lacks? If the answer changes a path or its contract, `domain-modeling` **amends A1 in the same session**, as it already does for any ADR. | A1 amended, or confirmed unchanged |
| **`/to-spec`** | Hook loads A3; `to-spec` already "respect[s] any ADRs in the area" | Implementation Decisions name each path touched, its owner, the cost added per unit, and the benchmark that shows it. | Spec lines |
| **`/to-tickets`** | Hook loads A3 | When a path has no owner module or benchmark yet, building them is prefactoring, which `to-tickets` already puts first. The ticket that extends a path has its A2 row and benchmark as acceptance criteria. | Ticket order and criteria |
| **`/implement-spec`, each implementer** | `AGENTS.md` line "Read the Quality register in `TESTING.md` before writing code. Do not weaken it to make a change pass."; optionally a `SubagentStart` hook | `tdd` as written; the implementer runs the counts locally | Code within the contract |
| **Each merge** | `just merge` (A5) | Counts against A2; an expired exception fails | Pass or fail |
| **Step 7, `code-review`** | A `PreToolUse` hook on the Skill tool when the skill is `code-review` adds A3's review items; the Standards axis reads `TESTING.md` and `STYLE.md` anyway | Reports a new parse, open, prepare or commit on a path outside its owner, or per-unit work that depends only on the Run | Review findings |
| **Between `/implement-spec` runs, as the owner chooses** | The owner | `improve-codebase-architecture` with the direction "the hot paths of A1"; `retro` after an incident such as #1577; occasionally `architecture-critic` or `security-threat-model` | Candidates and new checks |
| **A gate fails, or a limit must loosen** | The failing count | Loud by design: loosening needs the owner and an A1 amendment, through a short grilling | A1 and A2 changed together |

## 3. The `system-qualities` skill (A3), in outline

- **Description, for auto-triggering:** "Use when planning, grilling, specifying, ticketing or reviewing work that runs on a shared per-unit path (per page, per action, per message, per record, per request, per sample) or creates one, and when a performance, memory, durability, security or cost limit changes."
- **Procedure:**
  1. Read A1 and A2.
  2. List the paths the work touches, each with its unit and owner.
  3. For each path, name the counted work added per unit: parses, compiles, prepares, connections, transactions, fsyncs, file opens, configuration reads, CDP round trips.
  4. Check the work against the cost contract.
  5. If the contract or the path changes, say so as an ADR change for `domain-modeling`.
  6. Check that A2 has a row and a benchmark for the path.
  7. Ask the quality checklist: load and scale, latency, memory, durability, security and cost. Each is in scope with a counted measure, or out of scope with a reason.
- **Per step:** questions for grilling; a ticket for a wayfinder map; Implementation Decisions lines for `to-spec`; ticket order and criteria for `to-tickets`; findings for `code-review`.
- **What it never does:** write an ADR itself (that is `domain-modeling`'s job), plan or implement (that is Pocock's), or add an approval.

## 4. "The architecture changes as features are grilled": how it stays current

The global architecture is not frozen up front.
- **Set once:** grilling the global paths now gives A1 its first version.
- **Amended by the feature that changes it:** each feature grilling checks A1 and amends it in the same session when it changes a path or a contract. The architecture is then always current, and always written down before tickets are cut.
- **Three safety nets** for what a grilling misses:
  1. `/to-spec` and `/to-tickets` restate the paths;
  2. the gate catches any unannounced added work per unit;
  3. a failing gate reopens A1 loudly.

## 5. Keeping `TESTING.md`'s register current

- **One source per value:**
  - A1 or ADR 0134 decides each value or the rule that sets it ("1.10× the tuned figure").
  - The register holds the current number and the command that checks it.
  - `xtask perf` reads the register; it holds no constant of its own.
- **Tightening is silent:** a merge that improves a counted figure updates the register's number (the ratchet). `xtask perf` proposes the edit in the same pull request.
- **Loosening is loud:** a higher number, a removed row or a new exception needs the owner and a dated exception (owner, expiry, default 90 days, after Addy's skill), or an ADR amendment.
- **A check keeps it honest:** a `just merge` check (`xtask register check`, new) fails when:
  - a row's command does not exist;
  - a constant disagrees with the register;
  - an exception has expired;
  - a hot path in A1 has no row.

  This follows `principle-encode-lessons-in-structure`: rules become checks, not text.
- **No maintenance step:** the register changes in the pull request that changes the figure or the ADR, and never in a separate task.

## 6. Addy Osmani's `constraint-driven-development`: where and when

- **Once, now,** to build A2. It is installed as it is (`npx skills`, 2026-10-31), with no patch: `system-qualities` Setup tells it to write `TESTING.md`'s register instead of `CONSTRAINTS.md` and to use counters instead of its web defaults (LCP, CLS), and the project's `AGENTS.md` line says the bar lives in the register, so the skill finds it when it triggers on its own.
- **Then automatically,** through its own description ("when an agent keeps silencing checks or skipping tests to get to green"): it guards the bar when an agent tries to weaken a check. The model invokes it; it needs no hook.
- **Not on every ticket.** A3 covers the per-ticket questions.

## 7. Anthropic's `system-design` and `architecture`: used as they are

- **`system-design`** drives the global architecture grilling: requirements, high-level design, deep dive, scale and reliability, trade-offs. It is used with `grilling` and `domain-modeling`, which write the ADR.
- **`architecture`** evaluates a paths ADR when useful. It does not write ADRs: `domain-modeling` owns them. It is set to `user-invocable-only` in `~/.claude/settings.json` (`skillOverrides`), so only a typed `/architecture` runs it; the setting survives `npx skills update`.
- **`system-design`** stays model-invocable: `system-qualities` loads it in Setup and when the path check finds a new shared path.
- **Installed** on 2026-10-31 with `npx skills add anthropics/knowledge-work-plugins --skill system-design --skill architecture -a claude-code -a codex -g -y`, so they sit in `~/.agents/.skill-lock.json` beside the other skills and carry no `engineering:` prefix. Without `-a`, the install fails on the PromptScript agent, which has no global skills.
- **Nothing is copied** into A3, which calls them.

## 8. What this changes in the repository's rules: an owner decision

ADR 0133 says the repository "adds no process step of its own" and holds "standards and configuration, never extra workflow steps". This design keeps the Pocock sequence and adds no approval. It does, however, add:
- a skill loaded at existing steps;
- topics a grilling must cover;
- a ticket that `wayfinder` adds to a map.

That is a change to ADR 0133's rule, and only the owner can make it. Recommended wording for an amendment:

> The repository may add standards that the skills consult at their own steps: a skill loaded by a hook when a Pocock skill runs, topics a grilling covers, and the quality register that `code-review` reads. It still adds no step, review or approval to the skills' sequence.

## 9. Order of work, if adopted

1. ADR 0133 amendment (owner).
2. The global grilling, giving A1 (owner, HITL).
3. A2 from `constraint-driven-development`, patched.
4. A3 written with `skill-authoring`, vendored.
5. A4: the line in `domain.md`, the `AGENTS.md` line, and hooks in `.claude/settings.json`.
6. A5: `xtask perf` reads the register; `xtask register check`.

Steps 3 to 6 are repository work. They can go through `/to-spec` and `/to-tickets` like any feature, once A1 exists.
