# Duration measurements on a quiet Mac (local-controlled), October 10, 2026

The "Implement Spec - 1" session (timeframe project), whose Swift compilations were loading the Mac, was paused at the project owner's request. Each campaign waited for the host to qualify as `local-controlled` before starting. A block is excluded from the verdict if the foreign CPU exceeds 150% at its end, this CPU excluding the run's own processes (correction of October 10).

Retained runs:
- `browser-driver-rtt-0-2026-10-09T23-21-22-913Z-2bbed1`: 8 clean blocks out of 8;
- `browser-driver-rtt-20-2026-10-09T23-53-33-623Z-3dd31a`: 7 out of 8;
- `browser-driver-rtt-100-2026-10-10T00-17-31-377Z-ae1e25`: 5 out of 6;
- `browser-driver-rtt-300-2026-10-10T03-02-31-508Z-5366fd`: 4 out of 6;
- `http-transport-v2-2026-10-10T02-45-30-587Z-b23a72`;
- `durable-scheduler-2026-10-10T02-45-45-825Z-edc50c`;
- `durable-admission-2026-10-10T02-46-32-139Z-ef1b25`;
- `portal-routing-2026-10-10T02-47-13-937Z-4adc3c`;
- `light-engine-paired-2026-10-10T02-55-16-672Z-17bf22`.

They replace, as evidence, the `local-dev` runs of the same campaigns. The browser driver runs of October 8, already in `local-controlled`, are still cited and add to the new ones.

## Verdicts

| Decision | Result on a quiet Mac | Before (loaded Mac) |
| --- | --- | --- |
| **D-adaptive-routing** | **better, eligible**: `sampled-contract` on a static page at 0.004 times the browser's time, equivalent elsewhere (it keeps the browser). `api-capture` **better** on GraphQL and infinite scroll (0.008); `insufficient-n` on the other rendered pages, at 0.005-0.006 | `insufficient-n`, not eligible |
| **D-http-transport** | **non-inferior, eligible**: wreq 1.01, Impit 1.02, Node 1.00 versus reqwest; pooled Crawlee **worse** (0.86); Spider blocked by the contract | non-inferior, not eligible |
| **D-scheduling** | `insufficient-n` (5 blocks out of 22 requested): the fair queue gives 2.6 times more work to small domains (CI 2.31 to 2.93) | better (2.84), not eligible |
| **D-light-engine** | blocked by the contract (shadow DOM), now in `local-controlled` | blocked |
| **D-browser-driver** | **blocked by the contract**: native Playwright, the reference, exceeds its deadline twice on the JS catalog at 100 ms | `worse` at 0 ms |

### Browser driver: estimate outside the contract

The correctness gate reads the cells of the option and of its reference. The two timeouts of native Playwright at 100 ms therefore block all comparisons, whereas the guarded drivers are exact everywhere. The durations, recomputed without this gate, with the blocks of October 8 and 10 (task-time ratio versus native Playwright, lower = faster):

| Driver | Latency | Form | Clicks | JS catalog |
| --- | --- | --- | --- | --- |
| Rust CDP + guard | 0 ms | 1.33 | 1.21 | 0.33 |
| Rust CDP + guard | 20 ms | 0.80 | 0.84 | 0.73 |
| Rust CDP + guard | 100 ms | 0.70 | 0.74 | 0.79 |
| Rust CDP + guard | 300 ms | 0.68 | 0.72 | 0.80 |
| Node CDP + guard | all | identical to Rust within ±0.02 | | |
| Puppeteer + guard | 0 to 300 ms | 1.62 to 2.32 | 1.62 to 1.84 | 0.41 to 0.93 |

All these differences are clear-cut: the confidence intervals are less than 0.1 wide.

## Reading

- **The browser driver's tipping point lies between 0 and 20 ms.** On a local loop, the guard costs 20 to 33% on forms and clicks: it checks the element before acting. From 20 ms of latency between driver and browser, direct CDP with the guard becomes 16 to 32% faster than Playwright, and the gap grows with latency. A hosted browser is always remote: **CDP with the guard is the right default**. The driver's language changes nothing: Rust and Node are identical. Guarded Puppeteer remains the slowest.
- **Native Playwright is not an exact reference when remote**: two timeouts on the JS catalog at 100 ms. D-browser-driver already chooses the guard; the decision's reference should be a driver that is exact at all latencies.
- **Adaptive routing is eligible**: the compared sample costs nothing more than the browser where it keeps it, and on a static page it falls to 0.4% of its time. API discovery divides the time by 120 to 200 once the route is learned.
- **The transport does not separate anything on throughput**, at concurrency 4. The choice of reqwest rests on its CPU and memory cost, measured on October 9.
- **The per-domain fair queue remains clearly better** (2.6 times), but more dispersed than on October 8. It would take 22 blocks for the 10% margin. The effect is not in doubt; the formal verdict awaits more blocks.

## Limitations

- Quiet Mac, but not a reference host: nothing is publishable before the bare Linux host.
- Only the blocks whose foreign CPU stayed under 150% count; at 300 ms, 4 clean blocks out of 6, supplemented by those of October 8.
