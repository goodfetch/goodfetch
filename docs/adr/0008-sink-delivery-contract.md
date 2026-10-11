# Every sink follows one delivery contract, exactly-once where the destination can enforce it

Committed results leave the engine through sinks (PostgreSQL, S3-compatible stores, GCS, Azure Blob, signed webhooks) and exports (JSON, JSONL, CSV, XLSX, XML, Parquet). Every delivery carries a stable delivery ID and the run epoch and is retried with backoff. Where the destination can compare and set (PostgreSQL through a fencing table, object stores that support conditional writes) the write is exactly-once and a stale epoch is rejected, as ADR 0003 requires. Webhooks are at-least-once: the receiver deduplicates on the delivery ID and rejects a run epoch lower than the highest it has seen, which the documentation and the webhook signature make explicit. Object stores without conditional writes put the delivery ID and run epoch in each object key, so a retry rewrites the same object and a stale copy writes beside, never over, the current one. Decided in [Adopt collection decisions](https://github.com/Jbmanllr/automan/issues/71).

Competitor evidence: [storage and sinks research](https://github.com/Jbmanllr/automan/blob/research/storage-sinks-competitors/rnd/research/2026-10-10-storage-sinks-competitors.md).

## Considered Options

- **At-least-once everywhere, as competitors do**: rejected. No surveyed competitor guarantees exactly-once results: Crawlee v4 buffers handler writes but commits at least once, Apify's datasets take no idempotency key, Scrapy has no item-level deduplication on resume and its remote exports never retry, Bright Data says its trigger is not idempotent.
- **DuckDB as a sink**, the R&D's working view: rejected. DuckDB reads Parquet and SQLite directly, so it is documented as a reader of exports.
- **Choosing sinks by throughput**: rejected. The lab's sink throughputs are `insufficient-n` with unequal durability; capability and fencing decide.

## Consequences

- The results stream numbers records in commit order and never drops one: a slow consumer slows delivery or resumes from its last number, unlike Spider's broadcast channel, which drops pages.
- Webhook signatures follow the Standard Webhooks specification (ADR 0017).
- The catalog of further Sinks, the four guarantee classes that replace the three cases above, and how each class is probed are in ADR 0026. ADR 0026 also reverses the rejection of DuckDB as a sink: DuckDB and MotherDuck become Sinks, and DuckDB still reads run files and Exports. Both decided in [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88).
