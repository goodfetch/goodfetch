# TLA+ models

The TLA+ models of the model checks (ADR 0060; TESTING.md, Model checking).

- `tools.toml` pins the pair of TLA+ tools, each a URL with its SHA-256; `just tlc-fetch` keeps them in `target/tla-tools/`.
- Each folder holding a `model.toml` is one model: `spec` names its TLA+ module, and `traces` the files and folders of the modules it describes, a change to which selects it. `small.cfg` holds the Pre-merge and Release suites' constants, `large.cfg` the nightly job's.
- `sample/` is the TLC job's self-test: `just tlc sample` passes, and its `broken.cfg` variant must fail on its invariant (`just tlc-self-test`).
- `slices/` is the Usage ledger's slices and reclaim (ADR 0041): Slices taken, charged, given back and reclaimed from dead engines' runs never charge past the Limit nor lose what a run journaled. The `ledger` module traces each of its transitions under `goodfetch::ledger`.

Run `just tlc <model>`, or `just tlc --all`; TLC needs Java 11 or later.
