# Order of work

The order the owner adopted on 2026-11-01 for the specs after [#1583](https://github.com/goodfetch/goodfetch/pull/1583). Each `/implement-spec` run is one whole spec, and milestones never scope a run (ADR 0136).

1. **[#1577](https://github.com/goodfetch/goodfetch/issues/1577),** ADR 0138's optimization pass, which removes the page path's Exceptions. It waits for the open remainders of tickets #1583 part-merged only where its own tickets name them, and measures with `just compare` (which replaced `just compare-spider`, #1901) and `just targets` as #1583 merged them.
2. **[The Shared paths spec](https://github.com/goodfetch/goodfetch/issues/1838):** the other paths' Exceptions.
3. **Features,** each spec with the path check of `system-qualities`. A ticket that extends a Shared path waits for its path's gate ticket.
