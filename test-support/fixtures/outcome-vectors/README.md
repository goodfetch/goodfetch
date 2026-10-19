# Outcome vectors

Hand-written data stating the result document's outcome rules (ADR 0063): each file gives what a document holds and the outcome it must get. `goodfetch-protocol`'s constructors pass every file, and the TypeScript and Python helpers that write `journey` documents pass them in their own runners. Never generate them from the code they test.

- `journey/*.json`: `steps` (`passed`, `failed`, `not_reached`, `manual`), `checks` (`passed`, `failed`, `not_reached`), `tasks` (each named task's oracle outcome, `verified`, `incomplete`, `false_success` or `failed`, or `null` when the oracle never answered) and the variant's `outcome` (`proved` or `failed`).
- `conformance/*.json`: one capability's `cases`, each with its `entries` and each entry's `checks` (whether each held), the `case_outcomes` they give (`passed`, `failed`, `not_reached`) and the capability's `outcome` (`proved`, `unproved` or `failed`).

Each file has a `description` saying what it shows. A new rule or edge case adds a file.
