# Extended durable runtime faults, October 8, 2026

Contract of D-durable-runtime, executed in `local-dev` on committed sources. Retained run: `runtime-faults-extended-2026-10-08T17-49-02-487Z-54a5e6`. Node and Rust runtimes, 24 jobs per cell, 3 blocks. The contract remains the recovery contract: each job verified once, with its exact records, without loss or duplicate.

| Fault | What happens | Node | Rust |
| --- | --- | --- | --- |
| `write-error` | the commit transaction fails without a crash (simulated I/O error) | 1 restart, 24/24 verified | 1 restart, 24/24 verified |
| `lock-held` | another connection holds the write lock for 0.5 s, under the 5 s busy timeout | no restart, 24/24 | no restart, 24/24 |
| `lock-starved` | the lock is held for 6 s, beyond the busy timeout | 1 restart, 24/24 verified | 1 restart, 24/24 verified |

Both runtimes hold the contract on all three faults, across all three blocks. These faults complement the process crashes already covered (before and after commit, during and after acknowledgment, coordinator stopped). Durability beyond a process crash (VM power cut) remains out of scope for this matrix.
