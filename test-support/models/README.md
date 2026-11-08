# TLA+ models

The TLA+ models of the model checks (ADR 0060; TESTING.md, Model checking).

- `tools.toml` pins the pair of TLA+ tools, each an asset of this repository's `tla-tools-…` release with its SHA-256, since upstream replaces the 1.8.0 prerelease at each build; `just tlc-fetch` keeps them in `target/tla-tools/`, and the file's comments give the steps to move the pair.
- `trace-probe/` is not a model (it has no `model.toml`): it is the etcd-style trace-validation probe that checks a new pair of tools, run by the commands in `tools.toml`.
- Each folder holding a `model.toml` is one model: `spec` names its TLA+ module, and `traces` the files and folders of the modules it describes, a change to which selects it. `small.cfg` holds the Pre-merge and Release suites' constants, `large.cfg` the nightly job's.
- `sample/` is the TLC job's self-test: `just tlc sample` passes, and its `broken.cfg` variant must fail on its invariant (`just tlc-self-test`).
- `run-fencing/` is the run fencing model (ADR 0003): work-item leases, the run epoch each start and resume takes, Checkpoints, and the compare-and-set on the run epoch of the Fenced and Keyed destinations. Its actions are the transitions `goodfetch::lease` traces (`take_epoch`, `grant`, `renew`, `expire`, `release`, `commit`, `refuse_commit`), and its `zombie.cfg` variant must fail, showing the model reaches a zombie copy (`just tlc-self-test`).
- `slices/` is the Usage ledger's slices and reclaim (ADR 0041): Slices taken, charged, given back and reclaimed from dead engines' runs never charge past the Limit nor lose what a run journaled. The `ledger` module traces each of its transitions under `goodfetch::ledger`.

Run `just tlc <model>`, or `just tlc --all`; TLC needs Java 11 or later.
