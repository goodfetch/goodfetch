# Every sink follows one delivery contract, exactly-once where the destination can enforce it

Committed results leave the engine through sinks (PostgreSQL, S3-compatible stores, GCS, Azure Blob, signed webhooks) and exports (JSON, JSONL, CSV, XLSX, XML, Parquet). Every delivery carries a stable delivery ID and the run epoch and is retried with backoff. Where the destination can compare and set (PostgreSQL through a fencing table, object stores through conditional writes) the write is exactly-once and a stale epoch is rejected, as ADR 0003 requires; webhooks are at-least-once with the ID for deduplication. Decided in [Adopt collection decisions](https://github.com/Jbmanllr/automan/issues/71).

## Considered Options

- **At-least-once everywhere, as competitors do**: rejected. No surveyed competitor guarantees exactly-once results: Crawlee v4 buffers handler writes but commits at least once, Apify's datasets take no idempotency key, Scrapy re-emits items on resume and its remote exports never retry, Bright Data says its trigger is not idempotent.
- **DuckDB as a sink**, the R&D's working view: rejected. DuckDB reads Parquet and SQLite directly, so it is documented as a reader of exports.
- **Choosing sinks by throughput**: rejected. The lab's sink throughputs are `insufficient-n` with unequal durability; capability and fencing decide.

## Consequences

- The results stream numbers records in commit order and never drops one: a slow consumer slows delivery or resumes from its last number, unlike Spider's broadcast channel, which drops pages.
- Further sinks and integrations are catalogued in [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88).
