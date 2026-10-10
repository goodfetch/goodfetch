# Reproducing the October 8 qualifications

From `/Users/jb/Dev/automan/lab`, with the Node pinned in `.node-version` and the dependencies/bootstrap already built:

```sh
node --test --test-concurrency=1 tests/*.test.ts tests/integration/*.test.ts
./node_modules/.bin/tsc --noEmit
cargo fmt --manifest-path adapters/workflow-rust/Cargo.toml --check
node tools/qualification-suite.ts
node tools/qualification-current.ts
```

Both suites create new unique folders, without replacing the previous ones. `qualification-suite.ts` covers the five domains and the open arrivals; `qualification-current.ts` covers the lab fixes and the remaining Spider transports. Do not run the suites or builds in parallel: the measured resources would share the machine.

The first retained suite is `results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0`. The requalification is `results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2`. Each keeps its copy of the sources and the paths of its campaigns in its `summary.json`.

To regenerate the analysis, provide the first suite, the five campaign folders referenced by the second, then the second suite itself. The latter provides the frozen sources of the HTTP campaigns that do not copy their own source:

```sh
node reports/analyze-integrated.mjs results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0 <native> <recovery> <browser> <sqlite-readers> <spider-transport> results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/audit-integrated.py results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/render-integrated.py results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/update-integrated-index.py results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0 results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
```

The placeholders must be replaced by the exact folders from the requalification summary. The analysis runs after the benchmarks. The audit distinguishes comparator compliance, correction of lab errors and raw-data invariants. A completed suite does not imply that every backend succeeded: the states and sample sizes are kept in the cross-cutting report.
