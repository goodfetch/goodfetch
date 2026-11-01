# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase. The repo is single-context.

## Before exploring, read these

- **`GLOSSARY.md`** at the repo root.
- **`docs/adr/`**: read ADRs that touch the area you're about to work in.
- **[ADR 0140](../adr/0140-every-shared-per-unit-path-has-one-owner-module-a-cost-contract-of-counted-work-and-a-benchmark.md)** (the Shared paths) and the **Quality register** in [`TESTING.md`](../../TESTING.md#quality-register): read before work that runs on a Shared path.
- **`rnd/`** is R&D material kept for inspiration only: the draft vocabulary in `rnd/GLOSSARY.md`, the ADRs proposed in `rnd/adr/`, the research, the lab and its report in `rnd/README.md`.

The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) records terms and decisions as they get resolved.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `GLOSSARY.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
