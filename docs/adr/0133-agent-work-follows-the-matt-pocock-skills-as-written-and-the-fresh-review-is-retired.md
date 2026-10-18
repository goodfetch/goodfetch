# Agent work follows the Matt Pocock skills as written, and the fresh review is retired

Agents work Goodfetch's tickets with Matt Pocock's engineering skills (`/to-spec`, `/to-tickets`, `/implement-spec`, `/implement`, `tdd`, `code-review`, `pr` and the rest). The skills are self-sufficient, so the repository supplies only what they ask it for: the issue tracker, the triage labels, the domain docs, the glossary, the ADRs and the coding and testing standards. It adds no process step of its own.

The repository was written while an autonomous delivery mode in `AGENTS.md` had agents review and merge their own pull requests after a "fresh review on the exact head". The owner retired that mode on 2026-10-18 ([#1560](https://github.com/goodfetch/goodfetch/pull/1560)). Every rule that asked "the fresh review" to accept or check something then named a step that no one runs, and agents kept inventing it. An audit on 2026-10-18 found about 36 such places, and others that contradict or restate a skill.

## Decision

- **The skills' process is the process.** `TESTING.md`, `STYLE.md`, `docs/agents/` and the ADRs hold standards and configuration, never extra workflow steps, reviews or approvals.
- **The fresh review is retired.** Wherever a rule says a section of a pull request is "accepted by the fresh review on the exact head" (ADR 0057's "Size increase", ADR 0071's "Performance change"), the owner accepts it when merging. Wherever a rule says the fresh review or "the review" checks something, that rule is a review standard: the `code-review` skill applies it as one of the repository's documented standards, and the owner sees the result before merging.
- **Tests sit at agreed seams only, as `tdd` says.** The seams in a spec's Testing Decisions count as agreed. There is no exception for a seam named in a pull request.
- **A pull request's evidence is the `pr` skill's Evidence section.** Each new test's red failure is its Before and the passing test its After; surviving mutants and a skipped mutants job are listed there. No rule names a separate Verification section.
- **`/to-tickets` opens no pull request.** Files a rule had it commit "in its `/to-tickets` pull request" (the Journey catalog entries of ADR 0063, the stories file of ADR 0072) land in the first ticket's pull request.
- **The implementation loop is `/implement-spec` as written,** limited to the current milestone's tickets. `docs/agents/issue-tracker.md` gives the start command and no other rule about runs.

## Considered Options

- **Name an actor for the fresh review** (a review subagent before each merge): rejected. It re-creates the retired delivery mode, and `implement-spec` already runs `code-review` before it marks the pull request ready.
- **Rewrite each ADR's text in place:** rejected. ADRs are records; each amended bullet carries a note pointing here.

## Consequences

- **Amends ADRs 0037, 0057, 0058, 0062, 0063, 0071, 0072, 0073, 0074, 0075, 0077, 0087, 0124, 0125 and 0126.** A note sits at each affected bullet. Considered Options that cite autonomous delivery as a past reason (ADRs 0039, 0057 and 0063) stay as history.
- **`TESTING.md`** drops the other-seam exception and the fresh review, says Evidence instead of Verification, and points at `tdd` for refactoring.
- **`AGENTS.md`** drops the other-seam clause, and its `## Agent workflows` section becomes the `## Agent skills` block that `setup-matt-pocock-skills` writes.
- **`docs/agents/issue-tracker.md`**'s Implementation loop keeps only the start command scoped to the current milestone. The milestone rules (Create, Assign, Order, Frontier, Gate) and ADR 0064's order for `flaky-test` and `live-check` issues stay until the owner decides on them.
- **Open in the skills:** `implement-spec` step 3 marks its draft pull request as closing the spec and its tickets, while `preflight` step 4 says it closes only the tickets it built, never the spec. The repository no longer takes a side; the skills settle it.
- **Glossary.** No term changes.
