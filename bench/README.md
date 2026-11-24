# Benchmarks

The benchmark harness and its TypeScript targets (ADRs 0025 and 0071), moved from `rnd/lab`'s v2 campaign runner and cleaned up. Area 2 fills this folder.

- `hosts/`: the host classes above `local-dev`, each with the gates preflight checks before a run. `local-controlled` is a named machine whose figures are labelled with it and compared within one report (ADR 0025's note of 2026-11-10); `reference` is the bare-metal Linux host, declined while Goodfetch has no revenue (ADR 0076).
- `src/host.ts`: the host snapshot and the gates that grant a class, refuse it or, when the campaign allows, downgrade it. A gate it cannot measure fails, and uncommitted changes cap the host at `local-dev`.
- `src/provenance.ts`: the commit, whether the checkout is dirty, and a fingerprint of the harness's sources.
- `src/metrics.ts`: the metrics a run records and the probe each reads; a metric the host cannot measure is `null` with its reason, never 0.
- `src/resources.ts`: the adapter's process tree, sampled: CPU from `/proc/<pid>/stat` and proportional memory from `smaps_rollup` on Linux, RSS from `ps`.
- `src/campaign.ts`: a campaign file: its adapters, the Proving ground tasks they run, the blocks, warmup, iterations, concurrency, timeouts and restarts. Each adapter starts in the campaign file's folder.
- `src/adapter.ts`: the competitor adapter protocol, one JSON line a message on the adapter's stdin and stdout: `hello` (the task kinds it claims), `prepare` (the Proving ground's address), `run` (one task) and `close`. A call past its time stops the adapter and fails alone; the calls beside it fail as `sibling_abort`. A slot restarts a stopped adapter with a new generation, up to the campaign's limit.
- `src/proving-ground.ts`: the Proving ground's task list and oracle, on the address it answers on.
- `src/order.ts`: the seeded order of the adapters in each block, and the nearest-rank quantile.
- `src/cell.ts`: a cell's attempts and its metrics. The failure rate counts failed, falsely successful, incomplete, timed-out and crashed attempts against the verified ones; a timeout's aborted siblings count for neither.
- `src/runner.ts` and `src/cli.ts`: a run. Each block runs its adapters in a seeded order; each cell (an adapter on a task in a block) runs a conformance attempt, the warmup, then the measured attempts in a closed loop, and a cell that fails its gate is not measured. An attempt's status is the oracle's outcome, or what its error's code says. The run writes `run.json` (provenance, host, Proving ground seed and task digests, each cell's metrics) and `attempts.jsonl` in a new folder:

  ```sh
  node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>
  ```
