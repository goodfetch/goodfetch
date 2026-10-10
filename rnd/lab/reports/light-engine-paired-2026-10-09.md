# Light engine versus Chrome, paired within the same run (D-light-engine), October 9, 2026

Until now, Lightpanda and Chrome ran in separate runs: the verdict, which pairs blocks within a run, had nothing to compare. A bench campaign can now list several harness browsers (`browserHosts`). Each driver then runs once per browser and per block, in a randomized order, and the browser becomes a factor of the cell.

Run: `light-engine-paired-2026-10-09T16-29-54-931Z-c62471`, `local-dev`. It covers 4 drivers, 5 scenarios, 5 blocks and 5 iterations, with the Automan guard on the CDP and Puppeteer drivers, and native Playwright.

## Results

CPU and memory of the driver and the browser together, per driver, median across blocks:

| Driver | CPU per task, Chrome (ms) | CPU per task, Lightpanda (ms) | Ratio | Max RSS, Chrome | Max RSS, Lightpanda |
| --- | --- | --- | --- | --- | --- |
| `rust-cdp` | 1243 | 58 | 0.05 | 2860 MiB | 43 MiB |
| `node-cdp` | 1289 | 97 | 0.07 | 2830 MiB | 93 MiB |
| `puppeteer` | 1209 | 145 | 0.12 | 2911 MiB | 116 MiB |
| `playwright` | 3296 | 190 | 0.06 | 1788 MiB | 203 MiB |

Statuses per scenario:

| Scenario | Chrome | Lightpanda |
| --- | --- | --- |
| `http-catalog`, `js-catalog`, `form`, `clicks` | verified, all drivers | verified, all drivers |
| `shadow` | verified; Puppeteer `unsupported` | **failure** (timeout) for `rust-cdp`, `node-cdp` and Playwright; Puppeteer `unsupported` |

On the catalogs, a task takes 3 to 10 ms with Lightpanda versus 60 to 170 ms with Chrome. On forms and clicks, the duration is dominated by the guard's waits, and the gap narrows (170 to 240 ms versus 260 to 415 ms).

## Mechanical verdict

D-light-engine is **blocked by the contract** (`oracle-exact`): shadow DOM fails on Lightpanda. The gate covers the whole matrix, so an incompatible scenario blocks the option, whatever its figures on the rest.

## What this says for the architecture

- Lightpanda cannot be the default engine: it does not cover the whole actionability contract.
- On a compatible corpus, it costs 8 to 20 times less CPU and 15 to 65 times less memory. That is the order of magnitude that justifies a **tier chosen by capabilities**. As the architecture provides (§5.2), `capabilities()` and the workflow manifest declare what a task requires, and the engine chooses the backend before creating the session, never midway.
- The learned route of adaptive routing can carry this choice just like HTTP, browser or API: a compatible route validated on a sample moves to the light engine.

## Limitations

- `local-dev` host: CPU and durations are indicative; the verdict requires `local-controlled`.
- The actionability matrix on Lightpanda (`light-engine-actionability-…1d5ca3`) does not carry the host as a factor. It is still read in its own report, and the `false-success-count` gate is removed from the decision.
