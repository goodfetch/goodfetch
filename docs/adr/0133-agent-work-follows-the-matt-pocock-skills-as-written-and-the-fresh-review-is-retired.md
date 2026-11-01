# Agent work follows the Matt Pocock skills as written, and the fresh review is retired

Agents work Goodfetch's tickets with Matt Pocock's engineering skills (`/to-spec`, `/to-tickets`, `/implement-spec`, `/implement`, `tdd`, `code-review`, `pr` and the rest). The skills are self-sufficient, so the repository supplies only what they ask it for: the issue tracker, the triage labels, the domain docs, the glossary, the ADRs and the coding and testing standards. It adds no process step of its own.

The repository was written while an autonomous delivery mode in `AGENTS.md` had agents review and merge their own pull requests after a "fresh review on the exact head". The owner retired that mode on 2026-10-18 ([#1560](https://github.com/goodfetch/goodfetch/pull/1560)). Every rule that asked "the fresh review" to accept or check something then named a step that no one runs, and agents kept inventing it. An audit on 2026-10-18 found about 36 such places, and others that contradict or restate a skill.

## Decision

- **The skills' process is the process.** `TESTING.md`, `STYLE.md`, `docs/agents/` and the ADRs hold standards and configuration, never extra workflow steps, reviews or approvals.
  > Amended on 2026-11-01 by [ADR 0140](0140-every-shared-per-unit-path-has-one-owner-module-a-cost-contract-of-counted-work-and-a-benchmark.md), on the owner's instruction to decide every reversible choice, open to the owner's review: the repository may add standards that the skills consult at their own steps, such as a skill loaded by a hook when a Pocock skill runs, topics a grilling covers, and the Quality register that `code-review` reads. It still adds no step, review or approval to the skills' sequence.
- **The fresh review is retired.** Wherever a rule says a section of a pull request is "accepted by the fresh review on the exact head" (ADR 0057's "Size increase", ADR 0071's "Performance change"), the owner accepts it when merging. Wherever a rule says the fresh review or "the review" checks something, that rule is a review standard: the `code-review` skill applies it as one of the repository's documented standards, and the owner sees the result before merging.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22: the check reads the commenter's repository permission (admin) instead of the organization role, because the gate's fine-grained token cannot read organization memberships; a repository admin who is not an organization owner could also accept.
- **Tests sit at agreed seams only, as `tdd` says.** The seams in a spec's Testing Decisions count as agreed. There is no exception for a seam named in a pull request.
- **A pull request's evidence is the `pr` skill's Evidence section.** Each new test's red failure is its Before and the passing test its After; surviving mutants and a skipped mutants job are listed there. No rule names a separate Verification section.
- **No skill writes repository files or sets ticket fields.** Files a rule had `/to-spec` or `/to-tickets` write (the Journey catalog entries of ADR 0063, the stories file and its flags of ADRs 0072 and 0075) are standards: they exist before a test declares them, and tickets add them like any other work. `/to-tickets` opens no pull request and has no milestone step.
- **The implementation loop is `/implement-spec` as written,** limited to the current milestone's tickets. `docs/agents/issue-tracker.md` gives the start command and no other rule about runs.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): each run is one whole spec, every open ticket of that spec whatever its milestone, not the current milestone's tickets.

## Considered Options

- **Name an actor for the fresh review** (a review subagent before each merge): rejected. It re-creates the retired delivery mode, and `implement-spec` already runs `code-review` before it marks the pull request ready.
- **Rewrite each ADR's text in place:** rejected. ADRs are records; each amended bullet carries a note pointing here.

## Consequences

- **Amends ADRs 0037, 0057, 0058, 0062, 0063, 0064, 0071, 0072, 0073, 0074, 0075, 0077, 0087, 0124, 0125, 0126 and 0131.** A note sits at each affected bullet. Considered Options that cite autonomous delivery as a past reason (ADRs 0039, 0057 and 0063) stay as history.
- **`TESTING.md`** drops the other-seam exception and the fresh review, says Evidence instead of Verification, drops its refactoring rule (`tdd` holds it), and names no skill as the writer of a file.
- **`AGENTS.md`** drops the other-seam clause, and its `## Agent workflows` section becomes the `## Agent skills` block that `setup-matt-pocock-skills` writes.
- **`docs/agents/issue-tracker.md`**'s Implementation loop keeps only the start command scoped to the current milestone. On 2026-10-18 the owner removed the rules that added steps: Create, the `/to-tickets` milestone moves of ADR 0131, and ADR 0064's frontier order for `flaky-test` and `live-check` issues. Assign, the Order invariant, Frontier and Gate stay as tracker configuration.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): the Implementation loop starts `/implement-spec #<spec>`, one whole spec per run on `spec/<n>`; the Frontier no longer takes the current milestone first, and the Gate holds back no ticket.
- **Closing the spec:** `implement-spec` step 3 and `preflight` step 4 both mark the draft pull request as closing the spec and its tickets, a milestone-scoped run included. The repository takes no side.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): there is no milestone-scoped run. Each run's draft pull request closes its one spec and that spec's tickets.
- **Glossary.** No term changes.
