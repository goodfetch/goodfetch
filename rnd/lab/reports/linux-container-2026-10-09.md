# Contract pass on Linux, in a container, October 9, 2026

Rehearsal of the reference host commissioning ([rnd/plan/reference-host.md](../../plan/reference-host.md)) in a Linux container on the project owner's Mac:
- Docker Desktop, Debian bookworm, arm64, cgroup v2;
- image `lab/docker/linux.Dockerfile`, built from committed sources only (`git archive`);
- pinned toolchains: Node 25.5.0, pnpm 12.9.1, Rust 1.93.0, Go 1.27.1.

**What this pass is worth.** The container runs in a virtual machine, on a shared host. Its runs are of class `local-dev`, and the `reference` class requires a dedicated physical machine. They validate the Linux path (builds, probes, contract counts); their durations are not used for any decision and will never be published.

## Steps

1. `pnpm bootstrap`: dependencies, Chromium for Linux arm64, Rust adapters (Spider, storage, CDP, extraction, reqwest, wreq), shared SQLite.
2. `pnpm workflow-build`: Rust runtime.
3. `pnpm lab ci`: green.
4. `pnpm lab suite campaigns/v2/suites/qualification-current.json`: 6 steps out of 6, with the counts of the macOS qualification.

| Run | Verified tasks |
| --- | --- |
| `runtime-native-qualified-2026-10-09T00-41-00-780Z-a4457d` | 30 |
| `runtime-recovery-qualified-2026-10-09T00-50-37-703Z-632789` | 42 |
| `browser-actionability-guarded-2026-10-09T00-51-42-342Z-19842f` | 162 |
| `sqlite-readers-2026-10-09T00-54-43-859Z-d4dfa6` | 108 |
| `spider-http-transport-supported-2026-10-09T00-55-30-963Z-582f6b` | 60 |
| `runtime-http-backends-2026-10-09T00-55-32-223Z-35f7fa` | 24 |

## What had to be ported

- **Shared SQLite.** It is built as `libautoman_sqlite.so` (`-shared`), and the Node addon and the Go storage engine link against it. The Go toolchain is pinned per platform, with the official checksums. On macOS, the rebuild produces a byte-for-byte identical library.
- **Runtime resources.** On Linux, the sampler reads `/proc/<pid>/stat` and `smaps_rollup` (PSS); the `proc_pid_rusage` probe remains specific to macOS.
- **Self-contained ledger.** The summaries of the native runs existed only in `results/`, which is not versioned: on a fresh clone, no verdict could be recomputed and CI failed. The ledger now keeps a copy of each `run.v2.json`.
- **Chromium sandbox.** In the container, the adapters run as root and Chromium refuses to start there with its sandbox. The first pass gave 108/162 on the actionability matrix: Puppeteer and Crawlee with Stagehand did not start. They now launch Chromium with `--no-sandbox` as root on Linux, as Playwright already does.

## Remaining for the reference host

- A dedicated physical machine, with the `performance` governor and the gates of `hosts/reference.json`.
- Per-run systemd scopes, for `cgroup-memory-peak-bytes` and the CPU of terminated children.
- Lightpanda is only pinned for macOS arm64; its campaigns do not run on Linux yet.
