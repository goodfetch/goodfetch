# Order of work after M1

The order adopted on 2026-11-01 for the work after [#1583](https://github.com/goodfetch/goodfetch/pull/1583). Each `/implement-spec` run is one whole spec, and milestones never scope a run (ADR 0136).

1. **Docs pull requests:** ADR 0138 (#1795), ADR 0139 (#1798), then ADR 0140 and the Quality register.
2. **Cleanup** of what the M1 run left behind.
3. **[The gate's spec](https://github.com/goodfetch/goodfetch/issues/1837)** (ADR 0140): counters, benchmarks, the register check and the floor guard.
4. **Today values:** that spec's last ticket writes each row's first figure and promotes the rows to Enforced.
5. **Code audits** [#1882](https://github.com/goodfetch/goodfetch/issues/1882) and [#1884](https://github.com/goodfetch/goodfetch/issues/1884), with the LSP. A finding on the page path goes to #1577's tickets and may go stale once #1577 lands; any other goes to the Shared paths spec. Each is an Exception.
6. **[#1577](https://github.com/goodfetch/goodfetch/issues/1577),** ADR 0138's optimization pass, which removes the page path's Exceptions. It waits for the open remainders of tickets #1583 part-merged only where its own tickets name them, and measures with `just compare-spider` and `just targets` as #1583 merged them.
7. **[The Shared paths spec](https://github.com/goodfetch/goodfetch/issues/1838):** the other paths' Exceptions.
8. **Features,** each spec with the path check of `system-qualities`. A ticket that extends a Shared path waits for its path's gate ticket.
