Crates checked on crates.io on 2026-10-11: pgvector 0.4.2 (2026-05-22), sqlx 0.9.0, tokio-postgres 0.7.18, qdrant-client 1.19.0 (2026-08-04), pinecone-sdk 0.1.2 (2024-09-06), weaviate-community 0.2.2 (2024-02-17), weaviate-client 0.1.0 (2023-03-08), milvus-sdk-rust 3.0.2 (2026-09-16), chroma 0.15.0 (2026-06-30), chromadb 2.3.0 (2025-06-28), lancedb 0.40.0 (2026-10-07), lance 13.0.0, testcontainers 0.28.0, testcontainers-modules 0.15.0 (2026-02-21). Download counts are all-time totals from `https://crates.io/api/v1/crates/<name>`.

# Vector database destinations

Classes used below:
- **exactly-once fenced**: the write and the stale-epoch check happen atomically.
- **idempotent keyed**: rewriting the same key gives the same result, but nothing checks the epoch atomically.
- **at-least-once**: neither of the above.

## pgvector (PostgreSQL)

- **Write semantics.** pgvector is a Postgres extension, so vector rows are ordinary rows inside Postgres transactions ([pgvector README](https://github.com/pgvector/pgvector)).
  - `INSERT ... ON CONFLICT (id) DO UPDATE` gives a keyed upsert ([PostgreSQL INSERT](https://www.postgresql.org/docs/current/sql-insert.html)).
  - The ID can be any type that has a unique index, so a text delivery ID works directly.
- **Fencing.** In one transaction you can update a fencing row (`UPDATE fence SET epoch=$e WHERE run=$r AND epoch<=$e`), check the row count, write the data, and commit.
  - This rejects a stale epoch atomically with the write. It is the same pattern as the Postgres sink.
  - Inference: the pattern relies on standard Postgres transactions, not on any pgvector feature.
- **Class:** exactly-once fenced, batch-atomic.
- **Rust client.**
  - `pgvector` 0.4.2, released 2026-05-22, about 19.8M downloads, MIT OR Apache-2.0 ([crates.io](https://crates.io/crates/pgvector)).
  - It is community-maintained in the pgvector GitHub org by the pgvector author.
  - It supports Rust-Postgres (`postgres` feature), SQLx (`sqlx` feature) and Diesel ([README](https://github.com/pgvector/pgvector-rust)).
  - Use it with `sqlx` 0.9.0 (162M downloads) or `tokio-postgres` 0.7.18 (74M downloads).
- **CI test double.**
  - Image: `pgvector/pgvector:pg18-trixie` ([pgvector README, v0.8.7](https://github.com/pgvector/pgvector/blob/v0.8.7/README.md)). The latest pgvector tag is v0.8.7.
  - The `testcontainers-modules` `postgres` feature defaults to `postgres:11-alpine`.
  - You can swap the image with `ImageExt::with_name("pgvector/pgvector")` and `.with_tag("pg18-trixie")`. Both methods exist in testcontainers 0.28.0 (`src/core/image/image_ext.rs`).

## Qdrant

- **IDs.** Point IDs must be unsigned 64-bit integers or UUIDs ([Points](https://qdrant.tech/documentation/manage-data/points/)).
  - Delivery IDs therefore need a mapping. Inference: a UUIDv5 of the delivery ID is deterministic.
- **Upsert semantics.** "All APIs in Qdrant, including point loading, are idempotent" ([Points](https://qdrant.tech/documentation/manage-data/points/)).
  - Upserting an existing ID replaces the point: the old point is deleted, then inserted with only the vectors you specify ([Points](https://qdrant.tech/documentation/manage-data/points/)).
- **Conditional write.** `update_filter` on upsert has been "Available as of v1.16.0" ([Points, Conditional Updates](https://qdrant.tech/documentation/manage-data/points/)).
  - Only existing points that match the filter are updated. Points that do not match keep their current state.
  - New points are inserted "regardless of the filter" ([API: upsert points](https://api.qdrant.tech/api-reference/points/upsert-points)).
  - The docs give an optimistic-concurrency example that uses a `version` payload field.
- **Update mode.** `update_mode` (v1.17.0+) takes `upsert` (the default), `insert_only` or `update_only` ([Points](https://qdrant.tech/documentation/manage-data/points/)).
- **Durability and ordering.**
  - `wait=true` returns only after the change is applied. Otherwise the response means only "acknowledged" (written to the WAL).
  - `ordering` takes `weak` (the default), `medium` or `strong`. `strong` serializes writes through a permanent leader ([Consistency guarantees](https://qdrant.tech/documentation/scaling/consistency-guarantees/)).
- **Skipped points are not reported.** The response holds only `status` and `operation_id`. It does not say which points failed `update_filter` ([API](https://api.qdrant.tech/api-reference/points/upsert-points)).
- **Fencing.** Store `epoch` in the payload and upsert with `update_filter: epoch <= my_epoch`.
  - This fences each record: a zombie cannot overwrite a point that already carries a higher epoch.
  - It is not batch-atomic.
  - It does not stop a zombie from inserting a point that does not exist yet.
  - A zombie whose write was rejected cannot tell from the response.
  - Inference: no batch-level transaction or cross-collection fence is documented.
- **Class:** idempotent keyed, with conditional per-record fencing (not batch-atomic).
- **Rust client.**
  - `qdrant-client` 1.19.0, released 2026-08-04, about 4.5M downloads, Apache-2.0. It is official ([qdrant/rust-client](https://github.com/qdrant/rust-client)).
  - `UpsertPointsBuilder` has `update_filter()` and `update_mode()` (crate source `src/builders/upsert_points_builder.rs`).
  - The latest server is v1.19.2 (2026-10-05).
- **CI test double.**
  - Image: `qdrant/qdrant` ([Quickstart](https://qdrant.tech/documentation/quick-start/)), about 50M Docker Hub pulls.
  - `testcontainers-modules` 0.15.0 has no `qdrant` feature. Use `GenericImage`.

## Pinecone

- **Upsert semantics.** "If a record ID already exists, upserting overwrites the entire record" ([Upsert records](https://docs.pinecone.io/guides/index-data/upsert-data)).
  - IDs are strings of at most 512 characters.
  - A batch holds up to 1,000 records or 2 MB.
  - The page does not say whether a batch is atomic.
- **Update.**
  - Update by ID merges metadata fields.
  - Update by metadata filter applies `setMetadata` to up to 100,000 matching records.
  - Neither one is a precondition on the current value ([Update records](https://docs.pinecone.io/guides/manage-data/update-data)).
- **Consistency.** "Pinecone is eventually consistent" ([Update records](https://docs.pinecone.io/guides/manage-data/update-data)).
- **No conditional write.** I found no conditional write, If-Match or version check in the upsert or update docs. This is a search result, not proof that none exists.
- **Class:** idempotent keyed.
- **Rust client.**
  - `pinecone-sdk` 0.1.2, released 2024-09-06, about 224k downloads, Apache-2.0. It is official ([pinecone-rust-client](https://github.com/pinecone-io/pinecone-rust-client)).
  - The README says it is "still in an alpha state".
  - There has been no release since 2024-09, although the repo was pushed on 2026-08-14.
- **CI test double: Pinecone Local.**
  - It is an in-memory emulator with two images ([archived docs page, 2026-06-15](http://web.archive.org/web/20260615180750/https://docs.pinecone.io/guides/operations/local-development)):
    - `ghcr.io/pinecone-io/pinecone-local` is the database emulator.
    - `ghcr.io/pinecone-io/pinecone-index` is a single-index emulator.
  - Limits: API version 2025-01, no persistence, no auth, at most 100,000 records per index, and no namespace management.
  - The live URL now redirects to the quickstart (HTTP 308 on 2026-10-11), and `llms.txt` no longer lists the page.
  - The images still exist on ghcr.io with tags `latest`, `v0.7.0` and `v1.0.0.rc0`.
  - Inference: the emulator may be de-emphasized in the docs, so its status should be watched.
  - `testcontainers-modules` has no Pinecone feature.

## Weaviate

- **IDs.** Object IDs are UUIDs ([REST spec v1.40.0](https://github.com/weaviate/weaviate/blob/v1.40.0/openapi-specs/schema.json)).
  - The docs recommend deterministic UUIDs (`generate_uuid5`) ([Batch import](https://docs.weaviate.io/weaviate/manage-objects/import)).
- **Write semantics.**
  - `PUT /objects/{class}/{id}` replaces the whole object. `PATCH` does a JSON merge patch ([Update objects](https://docs.weaviate.io/weaviate/manage-objects/update)).
  - Batch import "is idempotent based on the object UUIDs ... it will be overwritten" (OpenAPI `/batch/objects`, v1.40.0).
  - Batch errors are reported per object, so a batch can partly succeed ([Batch import](https://docs.weaviate.io/weaviate/manage-objects/import)).
- **No conditional write.** The only parameter on PUT, PATCH and batch is `consistency_level`. There is no precondition or If-Match parameter (OpenAPI spec, v1.40.0).
- **Consistency.**
  - Writes support `ONE`, `QUORUM` (the default) or `ALL`.
  - Data uses a leaderless design and is "eventually consistent". Raft is used only for schema ([Consistency](https://docs.weaviate.io/weaviate/concepts/replication-architecture/consistency)).
- **Class:** idempotent keyed.
- **Rust client: no official maintained client.**
  - Official clients are Python, TS/JS, Java, C# and Go ([Client libraries](https://docs.weaviate.io/weaviate/client-libraries)). The community page lists no Rust client ([Community](https://docs.weaviate.io/weaviate/client-libraries/community)).
  - `weaviate-client` 0.1.0 (2023-03-08, about 3k downloads) is in the weaviate org and is abandoned.
  - `weaviate-community` 0.2.2 (2024-02-17, about 8k downloads, MIT) is stale.
  - Inference: a thin REST or gRPC client over `reqwest`/`tonic` is the realistic path.
  - The latest server is v1.40.0 (2026-10-07).
- **CI test double.**
  - Image: `cr.weaviate.io/semitechnologies/weaviate:1.40.0` ([Docker install](https://docs.weaviate.io/deploy/installation-guides/docker-installation)).
  - `testcontainers-modules` has a `weaviate` feature, which defaults to `semitechnologies/weaviate:1.28.2`. Override the tag.

## Milvus and Zilliz Cloud

- **Primary keys.** The primary key is INT64 or VARCHAR. `auto_id` defaults to false ([Primary field](https://milvus.io/docs/primary-field.md)).
  - Use VARCHAR with `auto_id=false` to key by delivery or record ID.
- **Upsert, override mode (the default).** The upsert "inserts the data carried in the request payload and deletes the existing entity" ([Upsert](https://milvus.io/docs/upsert-entities.md)). Omitted nullable fields become null.
  - With AutoID enabled, override mode generates a new primary key, so it is not deterministic. Avoid AutoID.
- **Upsert, merge mode.** Merge mode (`partial_update=True`, v2.6.2+) reads the existing entity with strong consistency, then merges.
  - "Concurrent upserts to the same entity are not atomic across requests" ([Upsert](https://milvus.io/docs/upsert-entities.md)).
  - Milvus v3.0.1 added internal "optimistic CAS validation" for partial updates ([release v3.0.1](https://github.com/milvus-io/milvus/releases/tag/v3.0.1)). It protects the merge, but the caller cannot set a condition.
- **No conditional write.** I found no user-supplied conditional upsert in the docs or the v3.0.x release notes.
- **Zilliz Cloud.** It documents the same override and merge semantics, with no conditional write ([Zilliz upsert](https://docs.zilliz.com/docs/upsert-entities)).
- **Class:** idempotent keyed.
- **Rust client.**
  - `milvus-sdk-rust` 3.0.2, released 2026-09-16, about 24k downloads. It is official: "The official Rust SDK for Milvus" ([repo](https://github.com/milvus-io/milvus-sdk-rust)).
  - crates.io shows the license as non-standard, while GitHub reports Apache-2.0.
  - The 3.x line targets Milvus 3.x. It has `upsert` with `partial_update`.
  - The latest server is v3.0.2 (2026-09-20).
- **CI test double.**
  - Milvus standalone runs from `standalone_embed.sh` with embedded etcd ([Docker install](https://milvus.io/docs/install_standalone-docker.md)).
  - The image is `milvusdb/milvus:v3.0.1` (script at tag v3.0.2).
  - `testcontainers-modules` has no Milvus feature. Use `GenericImage`.

## Chroma

- **Write semantics.**
  - IDs are strings: "Each record needs a unique string `id`".
  - `add` with an existing ID "will be ignored without throwing an error" ([Add data](https://docs.trychroma.com/docs/collections/add-data)).
  - `upsert` creates missing IDs and updates existing ones ([Update data](https://docs.trychroma.com/docs/collections/update-data)).
- **Metadata merges.** Updates merge metadata key by key, and a key set to None is deleted (`merge_update_metadata` in [rust/segment/src/types.rs @1.5.9](https://github.com/chroma-core/chroma/blob/1.5.9/rust/segment/src/types.rs)).
  - Retries stay deterministic if every write sends the full key set.
- **No conditional write.** The docs describe no conditional or where-guarded write and say nothing about batch atomicity.
- **Class:** idempotent keyed.
- **Rust client.**
  - `chroma` 0.15.0, released 2026-06-30, about 1.6M downloads, Apache-2.0. It is official: it lives at `rust/chroma` in [chroma-core/chroma](https://github.com/chroma-core/chroma), and the docs show a Rust `ChromaHttpClient` ([Client-server](https://docs.trychroma.com/docs/run-chroma/client-server)).
  - The community `chromadb` 2.3.0 is archived. Its README says "Please use ... the official Chroma Rust client" ([chromadb-rs](https://github.com/Anush008/chromadb-rs)).
  - The latest server release is 1.5.9 (2026-05-05).
- **CI test double.**
  - Image: `chromadb/chroma` on port 8000 ([Docker](https://docs.trychroma.com/guides/deploy/docker)).
  - `testcontainers-modules` has no Chroma feature. Use `GenericImage`.

## LanceDB

- **Merge insert.**
  - `merge_insert(&["id"])` joins on any column or composite key, so a string delivery ID works.
  - `when_matched_update_all(Some(cond))` updates only matched rows that satisfy an SQL condition that can use `target.` and `source.`, for example `"target.last_update < source.last_update"`. Rows that fail the condition "will be left as they are" (lancedb 0.40.0 `src/table/merge.rs`; [Update docs](https://docs.lancedb.com/tables/update)).
  - `MergeResult` returns `version`, `num_inserted_rows`, `num_updated_rows` and `num_attempts`.
- **Commits.**
  - Each commit writes a new manifest version through put-if-not-exists or rename-if-not-exists, so exactly one writer wins ([Lance transactions](https://lance.org/format/table/transaction/)).
  - S3 needs no external commit coordinator any more ([Storage](https://docs.lancedb.com/storage)).
  - `conflict_retries` (default 10) re-runs the merge on contention (lance 13.0.0 `merge_insert.rs`).
  - Inference: the condition is therefore re-evaluated against the latest version.
- **Fencing.**
  - Use the condition `target.epoch <= source.epoch` with `when_not_matched_insert_all()`.
  - All rows in one `merge_insert` land in one atomic commit, so the epoch check is atomic with the write, but per row.
  - A stale row is silently kept and shows up as a lower `num_updated_rows`.
  - Unmatched rows are still inserted by a zombie.
  - Inference: there is no multi-table transaction, so a separate fence table cannot be committed atomically with the data table.
- **Class:** idempotent keyed, with conditional per-record fencing (atomic per commit).
- **Rust client.**
  - `lancedb` 0.40.0, released 2026-10-07, about 1.6M downloads, Apache-2.0. It is official, and the core is written in Rust ([lancedb/lancedb](https://github.com/lancedb/lancedb)).
- **CI test double.**
  - It runs in-process: `lancedb::connect(tmpdir)` opens a local directory (crate doc example). No container is needed.
