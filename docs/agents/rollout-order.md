# Order of work

The order the owner adopted on 2026-11-01 for the specs after [#1583](https://github.com/goodfetch/goodfetch/pull/1583), with Foxhound's refactor placed first on 2026-11-06 in [How are the browser pool spec's bookkeeping choices settled?](https://github.com/goodfetch/goodfetch/issues/2149). Each `/implement-spec` run is one whole spec, and milestones never scope a run (ADR 0136). Each refactor spec runs right after the tickets its performance criterion is measured with, so that criterion can pass on its own pull request to `main`.

1. **[#1577](https://github.com/goodfetch/goodfetch/issues/1577),** ADR 0138's optimization pass, which removes the page path's Exceptions. It waits for the open remainders of tickets #1583 part-merged only where its own tickets name them, and measures with `just compare` (which replaced `just compare-spider`, #1901) and `just targets` as #1583 merged them.
2. **The Shared paths spec's HTTP path tickets** (#1838): [#2133](https://github.com/goodfetch/goodfetch/issues/2133), [#2134](https://github.com/goodfetch/goodfetch/issues/2134) and [#2135](https://github.com/goodfetch/goodfetch/issues/2135).
3. **[Spec: memory-first Run state](https://github.com/goodfetch/goodfetch/issues/2138)** (ADR 0142), the largest gap: raw HTTP. Its own finalizing ticket may move it.
4. **The Shared paths spec's CDP tickets** (#1838): [#1892](https://github.com/goodfetch/goodfetch/issues/1892), [#1894](https://github.com/goodfetch/goodfetch/issues/1894) and [#2071](https://github.com/goodfetch/goodfetch/issues/2071).
5. **[Spec: browser pool grows with concurrency](https://github.com/goodfetch/goodfetch/issues/2139)** (ADR 0143). Its page reuse ticket is blocked by #1892, whose registry of worlds drops at `Page.frameNavigated`.
6. **The extraction-path spec,** once written.
7. **[The Shared paths spec](https://github.com/goodfetch/goodfetch/issues/1838):** its remaining tickets, the other paths' Exceptions.
8. **Features,** each spec with the path check of `system-qualities`. A ticket that extends a Shared path waits for its path's gate ticket.
