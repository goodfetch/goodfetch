# Linux reference host

This document describes the preparation of the `reference` class host, the only class whose figures can be published. The host is not provisioned yet (phase P9). Until then, a decision is worth at best `local-controlled`.

The automatically applied gates are in [`lab/hosts/reference.json`](../lab/hosts/reference.json). `pnpm lab preflight reference` checks them before each run and the engine rechecks them at each block; a host that fails is refused, never silently downgraded.

## Hardware and system

- Dedicated physical machine (gate `virtualized = false`): no VM, no shared cloud instance. A rented *bare metal* instance is suitable.
- Recent Linux with unified cgroup v2 (`/sys/fs/cgroup/cgroup.controllers` present), `systemd` and `/proc/<pid>/smaps_rollup` (kernel ≥ 4.14).
- x86_64 or arm64 architecture, recorded in each run. A change of machine or architecture opens a new series: runs are only compared with each other on the same host.
- No other active service during the campaigns (gate `foreign-cpu-percent ≤ 10`, load per CPU ≤ 0.1, free memory ≥ 30%).

## CPU settings

To be applied at each boot, for example via a systemd unit:

```bash
sudo cpupower frequency-set -g performance
```

- `performance` governor on all cores (gate `cpu-governor`).
- Turbo and SMT: choose a policy, keep it fixed for the whole series and record it in the host description. Disabling turbo reduces variance; keeping SMT brings conditions closer to production. Changing either one opens a new series.
- No thermal warning during a block (gate `thermal-warning`).

## Memory and CPU per run

The `/proc/<pid>/stat` (CPU) and `smaps_rollup` (anonymous PSS + swap) probes are active as soon as the host is Linux. The `cgroup-memory-peak-bytes` metric (`memory.peak`, page cache included) and the cgroup CPU of terminated children require a systemd scope per run:

```bash
systemd-run --user --scope -p MemoryAccounting=yes -p CPUAccounting=yes pnpm lab run <campagne>
```

The parser exists (`lab/src/core/metrics/linux.ts`). Creation of the scope by the engine is to be wired up when the host is provisioned; until then, these two probes are declared unavailable with their reason.

## Network

- RTT only: the lab's user-space latency proxy (`lab/src/net/latency-proxy.ts`) works without privileges; each run records its paired calibration.
- Loss, jitter, bandwidth: `tc netem` on a dedicated interface or a network namespace, set by the operator. The lab detects and records the profile; it does not set it itself.

## Toolchains and sources

- Node, pnpm, Rust and Go at the lab's pinned versions (`.node-version`, `package.json`, lockfiles), then `pnpm install --frozen-lockfile` and `pnpm bootstrap`.
- The native SQLite build (`.so`) must be qualified on Linux before any `sqlite-readers` campaign in the `reference` class.
- Committed and clean sources (gate `git-clean`): a run on modified sources is capped at `local-dev`.

## Commissioning

1. Provision the machine and apply the settings above.
2. `pnpm lab ci` must be green.
3. `pnpm lab preflight reference` must grant the `reference` class.
4. Run the contract suite (`pnpm lab suite campaigns/v2/suites/qualification-current.json`), which must reproduce the expected counts.
5. Run the priority 0 reruns listed in the ledger (`rerun.priority`), then priorities 1 and 2.

## Rehearsal in a container

`lab/docker/linux.Dockerfile` builds a Linux image from committed sources only. On October 9, 2026, this image passed the bootstrap, CI and the whole contract suite ([report](../lab/reports/linux-container-2026-10-09.md)). A container in a virtual machine remains of class `local-dev`: it validates the Linux path, not the durations.

