# HTTP transports in v2 (D-http-transport), October 9, 2026

D-http-transport had no mechanical analysis: its runs came from the old harness, without reqwest, wreq or Impit. Two v2 campaigns put the six candidate transports on the same catalog. The decision receives an analysis that associates each option with its backend. The `policy-compliance` gate is removed from this decision: robots.txt and the other site preferences belong to the engine's site policy (D-identity), and no transport applied them on its own, the reference included. Measurements in `local-dev` on committed sources, Mac in use.

## Closed loop, incident catalog

The first run, `http-transport-v2-2026-10-09T15-30-27-630Z-d8bf7a`, is `diagnostic`. It runs five variants through the conformance gate: normal page, redirect, gzip, chunked and large response. Three backends fail there before any measurement:

| Backend | Refused variant | Cause |
| --- | --- | --- |
| `reqwest-http` | `http-gzip`: records missing | the adapter compiled reqwest without the `gzip` feature; the compressed body was not decoded |
| `wreq-http` | `http-redirect`: records missing | wreq 6.0.0-rc.31 defaults to `redirect::Policy::none()`, whereas its documentation announces 10 redirects; the adapter did not set a policy |
| `spider-http` | `http-redirect`: `HTTP 302` | Spider refuses a redirect to loopback (native SSRF check); expected and kept, as the decision states |

The first two were configuration defects in the lab's adapter. They are fixed: both stacks decode gzip, brotli, deflate and zstd, and wreq follows 10 redirects.

Run retained after the fix: `http-transport-v2-2026-10-09T16-13-29-199Z-130b39`, 5 blocks. Five transports pass the whole catalog; Spider remains blocked by its SSRF refusal. Medians per backend:

| Backend | Verified tasks/s | p50 (ms) | p95 (ms) | CPU per task (ms) | Max RSS | Startup (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| `reqwest-http` | 127.3 | 13.7 | 60.2 | 1.62 | 10.2 MiB | 5 |
| `wreq-http` | 128.4 | 12.9 | 61.5 | 1.43 | 8.0 MiB | 4 |
| `impit-http` | 123.7 | 14.1 | 61.7 | 16.69 | 95.4 MiB | 175 |
| `node-http` | 119.2 | 14.9 | 63.3 | 18.70 | 115.8 MiB | 154 |
| `crawlee-http-pooled` | 82.0 | 35.1 | 79.8 | 70.22 | 277.0 MiB | 696 |
| `spider-http` | blocked (loopback redirect refused) | | | | | |

The three contract campaigns that use these adapters were redone with the fixed binaries:
- `http-incidents-2026-10-09T16-17-14-005Z-41e172`;
- `access-policy-2026-10-09T16-19-19-822Z-f128cf`;
- `tls-fingerprint-2026-10-09T16-20-40-847Z-1d6b5d`.

The statuses per backend and per variant are the same as on October 8. The only exception, the intermittent failure of Crawlee HTTP on the TLS fingerprint, was already known.

## Open arrivals at 250 per second

The first run, `http-open-v2-2026-10-09T15-30-40-832Z-1b2a42`, ran with the old binaries. It is replaced by `http-open-v2-2026-10-09T16-13-49-759Z-f9b6c9`, with normal pages, 8 per task, and at most 4 simultaneous tasks, over 5 blocks. Its 17,422 attempts are verified and none fails:

| Backend | Verified tasks/s | p50 (ms) | p95 (ms) | CPU per task (ms) | Max RSS | Startup (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| `reqwest-http` | 116.0 | 30.6 | 33.0 | 1.51 | 9.4 MiB | 11 |
| `wreq-http` | 116.2 | 30.9 | 32.9 | 1.74 | 7.3 MiB | 10 |
| `impit-http` | 117.1 | 30.5 | 33.2 | 2.45 | 100.0 MiB | 152 |
| `spider-http` | 113.4 | 29.7 | 38.7 | 4.24 | 30.6 MiB | 16 |
| `crawlee-http-pooled` | 46.9 | 72.6 | 119.5 | 30.32 | 512.3 MiB | 1019 |
| `node-http` | 119.0 | 29.6 | 32.5 | 2.48 | 228.3 MiB | 169 |

## Reading

- **Throughput is bounded by concurrency, not by the transport.** With 4 simultaneous tasks and an 8-page task, served at 3 ms per page, the ceiling is around 130 tasks/s. Five transports come close to it. Pooled Crawlee remains far behind: its orchestration costs 30 to 70 ms of CPU per task and several hundred megabytes.
- **What separates the transports is cost.** reqwest and wreq require about 1.5 ms of CPU per task, less than 11 MiB of memory, and start in about ten milliseconds. On the incident catalog, Impit and Node consume 10 times more CPU and 10 times more memory.
- **A default configuration is not a contract.** Two transports out of six failed the catalog because of their configuration, not their stack: decompression was missing, and wreq followed no redirect by default, contrary to its documentation. The engine must set these options itself and cover them with the conformance gate, whatever transport is chosen.

## Mechanical verdict

`pnpm lab verdict D-http-transport` is computed on the closed-loop run only. In open arrivals, any transport that keeps up reaches the offered throughput: this metric does not separate anything there. The rule is non-inferiority at 10% against reqwest:

| Option | Result | Throughput ratio (CI) |
| --- | --- | --- |
| wreq | non-inferior | 1.005 (0.983 to 1.028) |
| Impit | non-inferior | 0.958 (0.927 to 0.987) |
| Node | `insufficient-n` (5/14 blocks) | 0.901 (0.808 to 0.976) |
| Pooled Crawlee | `insufficient-n` (5/14 blocks), clearly below | 0.623 (0.567 to 0.683) |
| Spider | blocked by the contract | — |

The `local-dev` host does not make this verdict eligible: `local-controlled` is required. It nevertheless confirms the reading: at equal throughput, the choice is made on CPU and memory cost, and reqwest remains the default, with wreq as a variant at the same cost.

## Limitations

- `local-dev` host: durations and CPU are indicative. The non-inferiority verdict requires `local-controlled`.
- Concurrency 4 saturates before the transport; a campaign at higher concurrency will show where each transport plateaus.
