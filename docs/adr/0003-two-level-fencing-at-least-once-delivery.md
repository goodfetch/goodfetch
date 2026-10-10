# Two-level fencing: work-item leases and a run epoch, with at-least-once delivery

A user handler runs out of process and may keep running after it has timed out, and a migrated run may leave a zombie copy alive on the old machine. So each work item is leased with a monotonic epoch, and the run itself carries an epoch that every resume increments. Handlers are delivered at least once, but their outputs are staged and become visible exactly once, when the current attempt commits under a valid lease and epoch. This is how Temporal, Hatchet, Restate and DBOS fence stale attempts; Crawlee, Apify, Scrapy and spider-rs use expiring locks at most, and accept a late commit from an expired attempt. Decided in [Adopt runtime and state decisions](https://github.com/Jbmanllr/automan/issues/70).

## Consequences

- The run epoch cannot be enforced inside the run's own file, since a zombie writes to its own copy. Every write leaving the engine (checkpoint, export, sink write) carries the epoch, and the destination enforces it: the cloud either kills the old copy before resuming or rejects stale epochs with a compare-and-set. A conformance test runs two copies of one run against such a store.
- Every outgoing browser or network command is checked against the lease and the run epoch, so a handler past its lease can send nothing more.
- A lease lasts the handler timeout plus a margin, a renewal only lengthens it, and expiry is judged on the engine's monotonic clock alone.
- Fencing cannot undo an effect a website has already received. The engine offers an effect journal to every run: an effect is checked rather than repeated on resume, and reported as uncertain when it has no check. When agents must use it is decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).
