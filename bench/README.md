# Benchmarks

The benchmark harness and its TypeScript targets (ADRs 0025 and 0071), moved from `rnd/lab`'s v2 campaign runner and cleaned up. Area 2 fills this folder.

- `hosts/`: the host classes above `local-dev`, each with the gates preflight checks before a run. `local-controlled` is a named machine whose figures are labelled with it and compared within one report (ADR 0025's note of 2026-11-10); `reference` is the bare-metal Linux host, declined while Goodfetch has no revenue (ADR 0076).
- `src/host.ts`: the host snapshot and the gates that grant a class, refuse it or, when the campaign allows, downgrade it. A gate it cannot measure fails, and uncommitted changes cap the host at `local-dev`.
