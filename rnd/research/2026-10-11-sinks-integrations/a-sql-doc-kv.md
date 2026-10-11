Crate versions checked on crates.io API, 2026-10-11: sqlx 0.9.0 (2026-05-21), mysql_async 0.37.1 (2026-09-01), mysql 28.0.3 (2026-09-27), rusqlite 0.40.2 (2026-08-08), tiberius 0.13.0 (2026-09-25), tiberius-ng 0.13.1 (2026-08-30), mssql-tds 0.2.0 (2026-10-06), mongodb 3.9.1 (2026-09-10), redis 1.7.1 (2026-09-25), fred 10.1.0 (2025-02-27), testcontainers 0.28.0 (2026-08-06), testcontainers-modules 0.15.0 (2026-02-21).

Classes used below:
- **exactly-once fenced**: the destination can reject a stale epoch atomically with the data write.
- **idempotent keyed**: a deterministic overwrite by key, with no atomic epoch check.
- **at-least-once**: neither.

Generic fenced pattern, the same as the PostgreSQL one. **Inference**, built from the cited primitives:
1. In one transaction, lock or conditionally update the fence row: `UPDATE fence SET epoch = :e WHERE run_id = :r AND epoch <= :e`.
2. If 0 rows match, roll back.
3. Otherwise, insert the delivery ID into a dedupe table with a primary key, upsert the Records, and commit.

## MySQL (InnoDB)

- **Primitive.** InnoDB transactions. `SELECT ... FOR UPDATE` takes exclusive locks on the fence row until commit or rollback ([locking reads](https://dev.mysql.com/doc/refman/8.4/en/innodb-locking-reads.html)). `INSERT ... ON DUPLICATE KEY UPDATE` handles the idempotent Records ([doc](https://dev.mysql.com/doc/refman/8.4/en/insert-on-duplicate.html)).
- **Pitfall: unchanged rows.** By default the affected-rows count only includes rows actually changed. A fence `UPDATE` that sets the same epoch therefore reports 0 rows. The `CLIENT_FOUND_ROWS` flag switches the count to matched rows ([ROW_COUNT](https://dev.mysql.com/doc/refman/8.4/en/information-functions.html)).
  - sqlx always sets `FOUND_ROWS` ([source](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-mysql/src/connection/stream.rs)).
  - mysql_async leaves it off by default; it is set with the `client_found_rows` option ([source](https://github.com/blackbeam/mysql_async/blob/v0.37.1/src/opts/mod.rs)).
  - A safer route is `SELECT epoch ... FOR UPDATE` followed by a comparison in code.
- **Pitfall: upsert counts.** For `ON DUPLICATE KEY UPDATE`, the affected-rows value is 1 for an insert, 2 for an update and 0 when nothing changed. The doc advises against this statement on tables with several unique indexes ([doc](https://dev.mysql.com/doc/refman/8.4/en/insert-on-duplicate.html)).
- **Pitfall: DDL.** `CREATE TABLE` causes an implicit commit, so sink tables must be created outside the delivery transaction ([doc](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html)).
- **Class:** exactly-once fenced.
- **Rust client.**
  - `sqlx` 0.9.0: MIT OR Apache-2.0, about 162M downloads, community (launchbadge). It supports MySQL and MariaDB ([README](https://github.com/launchbadge/sqlx/blob/v0.9.0/README.md)).
  - Alternative: `mysql_async` 0.37.1: MIT/Apache-2.0, about 8M downloads, community (blackbeam).
  - Oracle lists no official Rust connector ([MySQL connectors](https://dev.mysql.com/downloads/connector/)).
- **Test double.** The official `mysql` Docker image ([Hub](https://hub.docker.com/_/mysql)). testcontainers-modules has a `mysql` feature, but its default tag is `8.1` ([source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/mysql/mod.rs)). Override the tag with `ImageExt::with_tag` ([source](https://github.com/testcontainers/testcontainers-rs/blob/0.28.0/testcontainers/src/core/image/image_ext.rs)).

## MariaDB (InnoDB)

- **Primitive.** The same as MySQL: transactions plus `INSERT ... ON DUPLICATE KEY UPDATE`. The affected-rows count is 1 for an insert and 2 for an update, unless `CLIENT_FOUND_ROWS` is set ([doc](https://mariadb.com/docs/server/reference/sql-statements/data-manipulation/inserting-loading-data/insert-on-duplicate-key-update)).
- **Extra: `INSERT ... RETURNING`.** Available since 10.5.0, and it can be combined with `ON DUPLICATE KEY UPDATE` ([doc](https://mariadb.com/docs/server/reference/sql-statements/data-manipulation/inserting-loading-data/insertreturning)).
- **DDL.** DDL causes an implicit `COMMIT` ([START TRANSACTION](https://mariadb.com/docs/server/reference/sql-statements/transactions/start-transaction)).
- **Storage engine.** Tables must use a transactional engine such as InnoDB, not MEMORY. The doc names MEMORY and CONNECT as non-transactional ([same page](https://mariadb.com/docs/server/reference/sql-statements/transactions/start-transaction)).
- **Class:** exactly-once fenced, on InnoDB.
- **Rust client.** There is no official MariaDB Rust connector. MariaDB's own guide recommends the community crates `mysql` and `mysql_async`, and says MariaDB contributed features to them ([guide](https://mariadb.com/docs/connectors/other/mariadb-rust-connector-guide)). `sqlx` also supports MariaDB.
- **Test double.** The official `mariadb` image ([Hub](https://hub.docker.com/_/mariadb)). testcontainers-modules has a `mariadb` feature, with default tag `11.3` ([source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/mariadb/mod.rs)).

## SQLite (sink file)

- **Primitive.** `BEGIN IMMEDIATE` starts a write transaction at once. SQLite allows only one writer at a time ([doc](https://www.sqlite.org/lang_transaction.html)).
- **Primitive.** UPSERT `ON CONFLICT DO UPDATE ... WHERE` has existed since 3.24.0. Its `WHERE` clause can turn the update into a no-op, for example when the stored epoch is newer ([doc](https://www.sqlite.org/lang_upsert.html)).
- **Fencing.** The fence row, dedupe row and data commit together in one file transaction.
- **Limit: network filesystems.** These can break the locks and fsync that SQLite relies on. The doc recommends access only from the host that owns the file ([doc](https://www.sqlite.org/useovernet.html)).
- **Class:** exactly-once fenced on a local disk. **Inference:** fencing is unsafe if a zombie writes the same file over NFS or SMB.
- **Rust client.** `rusqlite` 0.40.2: MIT, about 122M downloads, community. Alternative: sqlx's `sqlite` feature. There is no official SQLite Rust crate.
- **Test double.** None is needed: SQLite is in-process, so a temporary file is enough. rusqlite's `bundled` feature removes the system-library dependency.

## Microsoft SQL Server

- **Primitive.** Explicit transaction plus `SET XACT_ABORT ON`. With it, a run-time error rolls back the entire transaction ([doc](https://learn.microsoft.com/en-us/sql/t-sql/statements/set-xact-abort-transact-sql)). `THROW` honors `XACT_ABORT`; `RAISERROR` does not.
- **Upsert with `MERGE`.** It needs a terminating `;`. The doc says `HOLDLOCK` (that is, `SERIALIZABLE`) prevents unique-key violations on concurrent insert-or-update. It also says separate `INSERT` and `UPDATE` statements may block less under heavy concurrency ([MERGE](https://learn.microsoft.com/en-us/sql/t-sql/statements/merge-transact-sql), "Concurrency considerations").
- **Fence.** Lock the fence row with `UPDLOCK, HOLDLOCK` table hints ([hints](https://learn.microsoft.com/en-us/sql/t-sql/queries/hints-transact-sql-table)), then check `@@ROWCOUNT` ([doc](https://learn.microsoft.com/en-us/sql/t-sql/functions/rowcount-transact-sql)).
- **Class:** exactly-once fenced.
- **Rust client.**
  - `tiberius` 0.13.0: MIT/Apache-2.0, about 5.4M downloads, community (Prisma). It was released 2026-09-25 after a gap since 0.12.3 (2024-07). Its README lists SQL Server 2017 to 2022 as tested on CI ([README](https://github.com/prisma/tiberius)).
  - Microsoft now publishes `mssql-tds` 0.2.0: MIT, first release 2026-09-10, about 79k downloads ([repo](https://github.com/microsoft/mssql-rs)). The README calls it a "foundational protocol layer" shared across language bindings. **Inference:** it is a low-level TDS core, not yet a mature user-facing client.
  - A community fork, `tiberius-ng`, also exists. sqlx dropped MSSQL in 0.7 ([README](https://github.com/launchbadge/sqlx/blob/v0.9.0/README.md)).
- **Test double.** Image `mcr.microsoft.com/mssql/server:2025-latest`; `ACCEPT_EULA=Y` is required ([quickstart](https://learn.microsoft.com/en-us/sql/linux/install-upgrade/quickstart-install-docker)).
  - The image is supported only on x86-64. Rosetta 2 and QEMU emulation are "not tested or supported", which matters on Apple-silicon dev machines.
  - testcontainers-modules has an `mssql_server` feature. Its default tag is `2022-CU14-ubuntu-22.04`, and EULA acceptance is explicit through `with_accept_eula()` ([source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/mssql_server/mod.rs)).

## MongoDB

- **Primitive: transactions.** Multi-document transactions only work on replica sets or sharded clusters. The production-considerations page says standalone deployments do not support them. It also says the default lifetime is under one minute, and that a write conflict aborts the transaction ([doc](https://www.mongodb.com/docs/manual/core/transactions-production-consideration/), [overview](https://www.mongodb.com/docs/manual/core/transactions/)).
- **Fenced pattern in a transaction.** In a session transaction, run `updateOne({_id: run, epoch: {$lte: e}}, {$set: {epoch: e}})`. If `matchedCount == 0`, abort. Otherwise insert the delivery ID (unique `_id`) and upsert the Records, then commit.
- **Without transactions: one Record at a time.** A single-document operation is atomic ([doc](https://www.mongodb.com/docs/manual/core/transactions/)). Each document can carry its epoch, written with `updateOne({_id: k, epoch: {$lte: e}}, ..., {upsert: true})`.
  - If a newer epoch is stored, the filter fails and the upsert's insert collides on the unique `_id`. The doc says such upserts "fail due to a unique key collision" ([update doc](https://www.mongodb.com/docs/manual/reference/method/db.collection.update/)).
  - **Inference:** this fences each Record but not a whole batch atomically.
- **Retries.** Retryable writes need a replica set or sharded cluster and retry once by default ([doc](https://www.mongodb.com/docs/manual/core/retryable-writes/)).
- **Class:** exactly-once fenced on a replica set. On a standalone, it is per-document fenced or idempotent keyed.
- **Rust client.** `mongodb` 3.9.1: Apache-2.0, about 19M downloads, official. Its README says "officially supported MongoDB Rust driver" ([README](https://github.com/mongodb/mongo-rust-driver/blob/v3.9.1/README.md)). `StartTransaction::and_run` retries the callback for up to 120 seconds ([source](https://github.com/mongodb/mongo-rust-driver/blob/v3.9.1/driver/src/client/session/action.rs)).
- **Test double.** MongoDB documents the `mongodb/mongodb-community-server` image ([doc](https://www.mongodb.com/docs/manual/tutorial/install-mongodb-community-with-docker/)). The Docker Official Image is `mongo` ([Hub](https://hub.docker.com/_/mongo)).
  - testcontainers-modules has a `mongo` feature. `Mongo::repl_set()` starts a single-node replica set "useful for testing ... transactions", but its default tag is old: `5.0.6` ([source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/mongo/mod.rs)).

## Redis (hashes, JSON)

- **Primitive: `MULTI`/`EXEC` plus `WATCH`.** `WATCH` gives optimistic check-and-set: `EXEC` aborts if a watched key changed. There are no rollbacks, and a command that fails after `EXEC` does not stop the others ([doc](https://redis.io/docs/latest/develop/using-commands/transactions/)).
- **Since Redis 8.4.** `SET ... IFEQ` gives single-key compare-and-set on string keys ([same page](https://redis.io/docs/latest/develop/using-commands/transactions/)).
- **Strongest primitive: a Lua script or Function.** Execution is atomic and blocks the server ([EVAL](https://redis.io/docs/latest/develop/programmability/eval-intro/)).
  - Functions (Redis 7+) persist to the AOF and replicate, unlike the volatile `EVAL` script cache ([Functions](https://redis.io/docs/latest/develop/programmability/functions-intro/)).
  - Every key a script touches must be passed in `KEYS`. In Cluster, the keys must share a slot through hash tags ([cluster spec](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/)).
  - Fenced script (**inference**): compare the epoch in the fence key; if stale, return an error. Otherwise check and set the delivery-ID key, then `HSET` or `JSON.SET`.
- **JSON.** It is part of Redis Open Source with no module needed ([JSON](https://redis.io/docs/latest/develop/data-types/json/)). `JSON.SET` only has NX/XX existence conditions ([command](https://redis.io/docs/latest/commands/json.set/)), so epoch checks need a script.
- **Durability limits.**
  - The default `appendfsync everysec` can lose 1 second of writes ([persistence](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/)).
  - `WAIT` "does not make Redis a strongly consistent store", and acknowledged writes can be lost on failover ([WAIT](https://redis.io/docs/latest/commands/wait/)).
  - **Inference:** a failover can roll the fence key back, letting a zombie through.
- **Class:** exactly-once fenced against the live primary through a script or Function. It is weaker across crash or failover unless `appendfsync always` is used on a single primary.
- **Rust client.** `redis` (redis-rs) 1.7.1: BSD-3-Clause, about 113M downloads. It has `json`, `script`, `cluster-async` and `tokio-comp` features. Redis documents it but calls it "a third-party client library and is not developed or supported directly by Redis" ([redis.io](https://redis.io/docs/latest/develop/clients/rust/)). Alternative: `fred` 10.1.0, MIT, last release 2025-02.
- **Test double.** The official `redis:<version>` image runs Redis Open Source ([doc](https://redis.io/docs/latest/operate/oss_and_stack/install/install-stack/docker/)). **Inference:** `redis:8.x` includes JSON.
  - testcontainers-modules has a `redis` feature with `Redis` (default tag `5.0`) and `RedisStack` (`7.2.0-v8`) ([source](https://github.com/testcontainers/testcontainers-rs-modules-community/blob/v0.15.0/src/redis/standalone.rs)). Override the tag to 8.x.

## Cross-cutting note

testcontainers-modules 0.15.0 depends on `testcontainers ^0.27.0`, while the latest testcontainers is 0.28.0 (crates.io dependencies API). Use the re-exported `testcontainers_modules::testcontainers` to avoid two versions.
