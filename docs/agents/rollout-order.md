# Order of work

The order the owner adopted on 2026-11-01 for the specs after [#1583](https://github.com/goodfetch/goodfetch/pull/1583), with Foxhound's refactor placed first on 2026-11-06 in [How are the browser pool spec's bookkeeping choices settled?](https://github.com/goodfetch/goodfetch/issues/2149). Each `/implement-spec` run is one whole spec, and milestones never scope a run (ADR 0136). Each refactor spec runs right after the tickets its performance criterion is measured with, so that criterion can pass on its own pull request to `main`.

1. **[#1577](https://github.com/goodfetch/goodfetch/issues/1577),** ADR 0138's optimization pass, which removes the page path's Exceptions: merged on 2026-11-07 ([#2074](https://github.com/goodfetch/goodfetch/pull/2074)). Only its figures, ceilings and `GATING` tickets remain open, and they run at step 10.
2. **The Shared paths spec's HTTP path tickets** (#1838): [#2133](https://github.com/goodfetch/goodfetch/issues/2133), [#2134](https://github.com/goodfetch/goodfetch/issues/2134), [#2135](https://github.com/goodfetch/goodfetch/issues/2135) and [#1896](https://github.com/goodfetch/goodfetch/issues/1896). #1896 blocks the memory-first Run state's admission ticket (#2167).
3. **[Spec: memory-first Run state](https://github.com/goodfetch/goodfetch/issues/2138)** (ADR 0142), the largest gap: raw HTTP. Its own finalizing ticket may move it.
4. **The Shared paths spec's CDP tickets** (#1838): [#1892](https://github.com/goodfetch/goodfetch/issues/1892), [#1894](https://github.com/goodfetch/goodfetch/issues/1894) and [#2071](https://github.com/goodfetch/goodfetch/issues/2071).
5. **[Spec: browser pool grows with concurrency](https://github.com/goodfetch/goodfetch/issues/2139)** (ADR 0143). Its page reuse ticket is blocked by #1892, whose registry of worlds drops at `Page.frameNavigated`.
6. **[Spec: the extraction path](https://github.com/goodfetch/goodfetch/issues/2164)** tests a clean-up rule only where it can match, builds rules once per version and host, and reads the body as it arrives.
7. **[Spec: analysis backpressure and host fairness](https://github.com/goodfetch/goodfetch/issues/2194).** Its tickets are blocked by the Run state's admission ticket (#2167) and the extraction path's streaming ticket (#2187).
8. **[Spec: Foxhound's peak memory](https://github.com/goodfetch/goodfetch/issues/2165):**
   - What it changes: huge pages off, a profile-guided, hot-ordered static-PIE, and lazy start-up.
   - When it can run: it needs only #2133 from the steps above, so it may run earlier.
   - The research that may extend it: [#2198](https://github.com/goodfetch/goodfetch/issues/2198), still running, on the ~10 MiB left at 100 and 1,000 pages.
9. **[The Shared paths spec](https://github.com/goodfetch/goodfetch/issues/1838):** its remaining tickets, the other paths' Exceptions.
10. **#1577's closing tickets:** [#1931](https://github.com/goodfetch/goodfetch/issues/1931), [#1932](https://github.com/goodfetch/goodfetch/issues/1932) and [#1933](https://github.com/goodfetch/goodfetch/issues/1933) read the figures, the owner approves the ceilings, and `GATING` is enforced. They run last, once every refactor spec has landed.
11. **Features,** each spec with the path check of `system-qualities`. A ticket that extends a Shared path waits for its path's gate ticket.
