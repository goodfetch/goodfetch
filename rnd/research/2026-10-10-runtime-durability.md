# Run-state durability: competitors, SQLite WAL `synchronous`, and measured cost

Research date: 2026-10-10. Facts only; inferences are marked **[inference]**.
Source code was read from shallow clones of each project's default branch on 2026-10-10 (commit and date given), and checked against the latest *released* version where it matters.

---

## 1. Competitors: where run state lives and how durable it is

### Crawlee (JavaScript)

Latest stable on npm: `crawlee` **3.18.2**. v4 is at `4.0.0-rc.1` (npm dist-tags, https://registry.npmjs.org/crawlee).

**v3.18.2 (stable), `@crawlee/memory-storage` 3.18.2** (read from the npm tarball https://registry.npmjs.org/@crawlee/memory-storage/-/memory-storage-3.18.2.tgz):
- Storage is in memory and mirrored to JSON files under `./storage` (`datasets/`, `key_value_stores/`, `request_queues/`) when `persistStorage` is true. `persistStorage` defaults to `true` (env `CRAWLEE_PERSIST_STORAGE`) (`memory-storage.js`, lines 95-99 of the tarball).
- Each request is its own JSON file. Updates are awaited and written **in place** with `fs.writeFile` under a `proper-lockfile` lock (`background-handler/fs-utils.js`, `lockAndWrite`). There is **no temp-file-and-rename and no fsync**. **[inference]** A crash in the middle of a write can leave a truncated JSON file. A power cut can lose any write still in the OS page cache.
- In-memory only mode: `new MemoryStorage({ persistStorage: false })` or `CRAWLEE_PERSIST_STORAGE=false`.

**v4 (rc; main at `5ca137f`, 2026-10-09), https://github.com/apify/crawlee:**
- `persistStorage` defaults to `true` and selects `FileSystemStorageBackend`. Otherwise `MemoryStorageBackend` is used (`packages/core/src/configuration.ts` line 81, `packages/core/src/service-locator.ts` lines 49, 249).
- The file system backend is a **Rust napi-rs module**, `@crawlee/fs-storage-native` 0.2.2. Its source is https://github.com/apify/crawlee-storage (`5bea28e`, 2026-09-21; npm metadata at https://registry.npmjs.org/@crawlee/fs-storage-native).
- `atomic_write` writes a temp file in the same directory, then calls `fs::rename`. It **does not call `sync_all`/`fsync` on the file or the directory** (`crawlee-storage/src/utils.rs` line 81; grep for `sync_all|sync_data|fsync` returns nothing). The request-queue state lives in one file per request, "written eagerly on every mutation". `persist_state` is a no-op (`crawlee-storage/src/request_queue.rs`, around line 2342).
- **[inference]** This protects against torn files on a process crash. It does not make writes durable across a power cut: rename without fsync can leave the old file, or on some filesystems an empty one.

**Both versions:** `purgeOnStart` defaults to `true` (`packages/core/src/configuration.ts` line 56). The default storage is wiped at the start of a run, so resuming must be opted into.

### Crawlee (Python)

Latest on PyPI: **1.10.4**. Main is `1.10.5` (`50b7a1d`, 2026-10-08), https://github.com/apify/crawlee-python.
- The default storage client is `FileSystemStorageClient` (`src/crawlee/_service_locator.py` lines 96-102). The alternatives are `MemoryStorageClient` ("No data is persisted between process runs"), `SqlStorageClient` (SQLite, Postgres, MySQL or MariaDB through SQLAlchemy) and a Redis client (`src/crawlee/storage_clients/`).
- File system writes on Unix use `tempfile.mkstemp`, write, then `Path.replace` (atomic rename), with **no fsync**. On Windows the write goes directly to the target file with no temp file (`src/crawlee/_utils/file.py` lines 22-62).
- Request-queue ordering and in-progress state are held in a `RecoverableState` that is persisted to the KVS on `PersistState` events. These fire every `persist_state_interval`, which defaults to **1 minute** (`configuration.py` lines 90-99; `_file_system/_request_queue_client.py` lines 72, 137-147). Individual request files are written on mutation.
- **SQL client with SQLite sets `PRAGMA journal_mode=WAL` and `PRAGMA synchronous=NORMAL`** on every connection (`src/crawlee/storage_clients/_sql/_storage_client.py` lines 296-299). This is the exact trade-off Automan is considering, chosen as Crawlee's only SQLite setting.
- `purge_on_start` defaults to `True` (`configuration.py` lines 79-87).

### spider-rs (Spider)

Latest on crates.io: **2.53.9**. Main is 2.54.0 (`fb0f270`, 2026-10-01), https://github.com/spider-rs/spider.
- The frontier and visited set are in memory by default. The `disk` cargo feature (`disk = ["dep:sqlx"]`) adds "SQLite hybrid disk storage to balance memory usage" (`spider/Cargo.toml` line 246; `spider/README.md` line 136).
- The database is created at `$SQLITE_DATABASE_URL`, or the **OS temp dir** by default, as `spider_<crawl_id>.db`. It is **deleted on `Drop` unless `persist` is set** (`spider/src/features/disk.rs`, `get_db_path` and `impl Drop`).
- No pragmas are set. The connection uses `SqlitePool::connect_lazy(url)` (`disk.rs` line 111). sqlx 0.8 does not set `journal_mode` unless asked, and leaves `synchronous` at SQLite's default, FULL (https://raw.githubusercontent.com/launchbadge/sqlx/v0.8.6/sqlx-sqlite/src/options/mod.rs, lines 177-189). The effective mode is therefore the rollback journal (DELETE) with FULL.
- "Pause/resume" (`utils::pause`/`resume`) is in-process only (`website.rs`). No grep hit shows crash recovery or checkpoint-resume of the frontier. **[inference]** The disk feature offloads memory; it is not a durability feature.

### Scrapy

Latest on PyPI: **2.19.0**. Main at `be7bdc4` (2026-10-09), https://github.com/scrapy/scrapy.
- Persistence is opt-in through `JOBDIR`, which holds disk request queues (`requests.queue/`, queuelib `FifoDiskQueue`/`LifoDiskQueue` plus pickle), `requests.seen` (an append-mode binary fingerprint file in `RFPDupeFilter`) and `spider.state` (pickle written on close) (`scrapy/dupefilters.py` line 118; `scrapy/extensions/spiderstate.py`).
- queuelib writes with `os.write`, with no fsync (https://github.com/scrapy/queuelib, `queuelib/queue.py` line 128).
- The docs say: "Job pausing and resuming is only supported when the spider is paused by stopping it cleanly. Forced, sudden or otherwise unclean shutdown can lead to data corruption in the job directory" (`docs/topics/jobs.rst` lines 104-107; https://docs.scrapy.org/en/latest/topics/jobs.html).
- **Unreleased on main** (absent from the 2.19.0 tag; `.. versionadded:: VERSION`):
  - `JOBDIR_SYNC_EVERY` (default `0`) writes the scheduler's `active.json` every N changes, "so that a crawl killed before it can stop cleanly resumes from its latest state" (`docs/topics/settings.rst` lines 1677-1691; `scrapy/core/scheduler.py` lines 341, 492-503).
  - A new `DiskDupeFilter` uses SQLite with **`journal_mode=WAL` and `synchronous=NORMAL`** (`scrapy/dupefilters.py` lines 195-216). As written, it uses Python `sqlite3` implicit transactions and commits only in `close()`. **[inference]** Fingerprints recorded since the last commit would be lost on a crash; verify before relying on it.
- The default (no `JOBDIR`) is fully in memory.

### Colly (Go)

Latest release **v2.2.0**, https://github.com/gocolly/colly.
- The default `storage.Storage` is `InMemoryStorage` ("the default storage backend of colly ... keeps cookies and visited urls in memory") (`storage/storage.go` lines 27, 44-47; `colly.go` line 505). The queue also defaults to in-memory storage (`queue/`).
- Persistence comes only from external `Storage` implementations such as Redis or SQLite, with whatever durability they provide. No fsync is involved in core.

### Katana (ProjectDiscovery)

Latest release **v1.8.0**, https://github.com/projectdiscovery/katana.
- `-resume <file>` exists. The resume file is written **only on SIGINT/SIGTERM** by `os.WriteFile(json(state))`, with no fsync (`cmd/katana/main.go` lines 284-303; `internal/runner/runner.go` lines 158-161).
- The state is the set of in-flight *input* URLs (`internal/runner/executer.go` lines 25, 51), not the crawl frontier. A SIGKILL or crash leaves nothing. The file is removed after a normal completion.

### Firecrawl (self-hosted)

Main at `6f892bc` (2026-10-09), https://github.com/firecrawl/firecrawl.
- The queue ("NuQ") is in Postgres 17 (`apps/nuq-postgres`), with Redis for other state (`docker-compose.yaml`).
- Postgres keeps the default `synchronous_commit` (no override found) and is tuned with **group commit**: `commit_delay = 10` µs, `commit_siblings = 5`, under the comment "Reduce fsync overhead" (`apps/nuq-postgres/nuq.sql` lines 28-30).
- Redis runs as `redis-server --bind 0.0.0.0` with no AOF configured (`docker-compose.yaml` line 139). That means Redis defaults: RDB snapshots only (https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/).
- A FoundationDB backend is being added: "Replaces nuq-postgres once stable" (`docker-compose.yaml` line 195).

### Stagehand v4

Latest on npm: `@browserbasehq/stagehand` **4.2.0**.
- No local run-state store. v4 removed local `enableCaching`/`cacheDir`. Caching is now server-side on Browserbase: "The cache lives on Browserbase ... persists across script executions and across machines" (`packages/docs/v4/migrations/v3.mdx` lines 509, 601; `packages/docs/v4/basics/act.mdx` line 665, https://github.com/browserbase/stagehand).
- Agent loop continuation has "No equivalent. Your loop sits between steps, so log, cancel, or persist there" (`v3.mdx` line 598).

### Browser Use

Latest on PyPI: **0.13.11**. Main at `c75e847`, https://github.com/browser-use/browser-use.
- Agent state is in memory. It can be injected (`injected_agent_state`, `agent/service.py` line 436). History can be saved explicitly with `AgentHistoryList.save_to_file`, a plain `open(...,'w')` plus `json.dump` with no atomic rename or fsync (`browser_use/agent/views.py` lines 622-630).
- There is no durable queue and no crash resume.

### Temporal

Server **v1.32.1**; CLI **v1.9.1**.
- In production, Temporal uses an external database (Cassandra, PostgreSQL, MySQL).
- The dev server (`temporal server start-dev`) uses SQLite **in memory by default** (`mode=memory, cache=shared`). `--db-filename` switches to a file (https://github.com/temporalio/cli `internal/devserver/server.go` lines 328-335). The docs say: "By default, Workflow Executions are lost when the server process dies" (https://docs.temporal.io/cli/server).
- `--sqlite-pragma` is passed through as `_pragma` parameters. The server's SQLite plugin warns when there are multiple connections without WAL (https://github.com/temporalio/temporal `common/persistence/sql/sqlplugin/sqlite/plugin.go` lines 97-108, 204-205).

### Restate

Latest release **v1.7.13**. Main is `1.8.0-rc.2` (`752283e`), https://github.com/restatedev/restate.
- Restate has its own durable log (Bifrost; local or replicated loglets on RocksDB) plus partition stores.
- `rocksdb-disable-wal-fsync` ("Disable fsync of WAL on every batch") defaults to **false**, so the WAL is fsynced on every batch. `rocksdb-disable-wal` is "Dangerous" and hidden from the schema (`crates/types/src/config/bifrost.rs` lines 196-217, 297-312; `log_server.rs` lines 63, 445-448).
- The local loglet batches: `writer_batch_commit_count = 5000`, and `writer_batch_commit_duration = 0` (no time-based batching). **[inference]** This is group commit: one fsync covers every record that accumulated while the previous commit was in flight.

### DBOS

Python **3.2.0** on PyPI. Repos: https://github.com/dbos-inc/dbos-transact-py (`d3f2de7`) and https://github.com/dbos-inc/dbos-transact-ts (`702b4a0`).
- The Python system database defaults to **SQLite** `sqlite:///<app>.sqlite`. The docs say it "may be either Postgres or SQLite, though Postgres is recommended for production" (https://docs.dbos.dev/python/reference/configuration; `dbos/_dbos_config.py` lines 401-404).
- On SQLite, DBOS sets only `isolation_level="IMMEDIATE"`, `busy_timeout=30000` and `foreign_keys=ON` (`dbos/_sys_db_sqlite.py` lines 42-50). It sets **no `journal_mode` and no `synchronous`**, so the effective mode is SQLite's default: rollback journal (DELETE) with synchronous=FULL.
- No SQLite references were found in the TS `src/`, which is Postgres-only as far as grep shows.

### Inngest (self-hosted)

Latest release **v1.46.0**.
- `inngest start` by default runs "an in-memory Redis server for the queue and state store" with SQLite at `./.inngest/main.db`. "Queue and state store snapshots are periodically saved to the SQLite database, including prior to shutdown" (https://www.inngest.com/docs/self-hosting).
- The snapshot interval defaults to **60 s** (`StartDefaultPersistenceInterval = time.Second * 60`, https://github.com/inngest/inngest `pkg/consts/devserver.go` line 15).
- **[inference]** A hard crash can lose up to about 60 s of queue and run state. An external Redis (`--redis-uri`) and Postgres (`--postgres-uri`) are offered "to improve performance and reliability".
- SQLite is opened as `file:<path>?cache=shared` with no pragmas (`pkg/db/sqlite/migrations.go` line 124).

### Hatchet

Latest release **v0.110.5**.
- Hatchet uses PostgreSQL "for storing workflow state and metadata", with RabbitMQ optional. "Hatchet Lite" is a single image for "development, testing, or low-throughput production" (https://docs.hatchet.run/self-hosting).
- A GitHub code search found no `synchronous_commit` override (https://github.com/hatchet-dev/hatchet), so Postgres defaults apply.
- One migration sets some OLAP partition tables `UNLOGGED` (`cmd/hatchet-migrate/migrate/migrations/20260424190713_v1_0_99.go` line 50). The context looks like a migration step; it was not investigated further.

### Summary table

| Project (version) | Default run-state store | fsync on commit? | Fast or in-memory option | Default resumes after crash? |
|---|---|---|---|---|
| Crawlee JS 3.18.2 | In memory, mirrored to JSON files | No (in-place writeFile) | `persistStorage:false` | No (`purgeOnStart:true`) |
| Crawlee JS 4.0 rc | Rust file store (crawlee-storage) | No (tmp+rename, no fsync) | MemoryStorageBackend | No (`purgeOnStart:true`) |
| Crawlee Py 1.10.4 | File system client | No (tmp+replace; RQ state every 60 s) | Memory, SQL (SQLite WAL+**NORMAL**), Redis | No (`purge_on_start=True`) |
| Spider 2.53.9 | In memory; `disk` feature uses SQLite in temp dir | SQLite default (DELETE+FULL) | Default is in-memory | No (database deleted on drop) |
| Scrapy 2.19.0 | In memory; `JOBDIR` opt-in | No | Default is in-memory | Clean stop only (docs) |
| Colly v2.2.0 | In memory | n/a | Default | No |
| Katana v1.8.0 | In memory; resume file on SIGINT | No | Default | No (signal only) |
| Firecrawl (main) | Postgres + Redis | Yes (Postgres default) with commit_delay group commit | n/a | Yes, for the Postgres queue |
| Stagehand 4.2.0 | None local (Browserbase) | n/a | n/a | n/a |
| Browser Use 0.13.11 | In memory | No | Default | No |
| Temporal dev 1.9.1 | SQLite **in memory** | n/a | `--db-filename` for file | No by default |
| Restate 1.7.13 | Own log on RocksDB | **Yes, per batch** | `rocksdb-disable-wal-fsync` | Yes |
| DBOS Py 3.2.0 | SQLite (dev) or Postgres | Yes (SQLite default FULL) | n/a | Yes |
| Inngest 1.46.0 | In-memory Redis plus SQLite snapshots | Snapshots every 60 s | External Redis/PG | Up to 60 s loss **[inference]** |
| Hatchet 0.110.5 | Postgres | Yes (Postgres default) | n/a | Yes |

---

## 2. SQLite WAL: FULL vs NORMAL, precisely

All quotes are from sqlite.org, fetched 2026-10-10. The SQLite trunk VERSION is 3.55.0.

**What each mode syncs in WAL mode** (https://www.sqlite.org/pragma.html#pragma_synchronous):
> In WAL mode when synchronous is NORMAL (1), the WAL file is synchronized before each checkpoint and the database file is synchronized after each completed checkpoint and the WAL file header is synchronized when a WAL file begins to be reused after a checkpoint, but no sync operations occur during most transactions. With synchronous=FULL in WAL mode, an additional sync operation of the WAL file happens after each transaction commit.

**Durability and corruption (same page):**
> WAL mode is safe from corruption with synchronous=NORMAL ... WAL mode is always consistent with synchronous=NORMAL, but WAL mode does lose durability. A transaction committed in WAL mode with synchronous=NORMAL might roll back following a power loss or system crash. Transactions are durable across application crashes regardless of the synchronous setting or journal mode.

The page's matrix says WAL+FULL = ACID, WAL+NORMAL = "Maybe not durable", and EXTRA "is no different from FULL in WAL mode".

**Compile options** (https://www.sqlite.org/compile.html): the stock default is FULL, `SQLITE_DEFAULT_SYNCHRONOUS=2`. Under `SQLITE_DEFAULT_WAL_SYNCHRONOUS` the docs say:
> in WAL mode, complete database integrity is guaranteed with PRAGMA synchronous=NORMAL. With PRAGMA synchronous=NORMAL in WAL mode, recent changes to the database might be rolled back by a power loss, but the database will not be corrupted.

The same section recommends switching to NORMAL when using WAL.

**Answer to "does NORMAL lose only the last transactions on power loss without corruption?":**
- **Yes, per sqlite.org.** In WAL with NORMAL, a power loss or **OS crash** can roll back recently committed transactions but does not corrupt the database. An application or process crash loses nothing.
- How many transactions can be lost is **not bounded to "the last one".** It is everything committed since the last WAL sync. Under NORMAL that sync happens only at checkpoints, which run by default every 1000 WAL pages (https://www.sqlite.org/wal.html). **[inference]** That can be many transactions.
- Labelling precision: sqlite.org says "power loss **or system crash**" (OS crash or kernel panic), not just power cuts. A label of "survives process crash, not power cut/machine failure" is consistent with this, provided "machine failure" includes OS crashes and hard resets.
- The guarantee assumes the storage honours sync as a write barrier. From https://www.sqlite.org/howtocorrupt.html §3.1: "most consumer-grade mass storage devices lie about syncing". Also: "In WAL mode, the only time that a failed sync operation can cause database corruption is during a checkpoint operation. A sync failure during a COMMIT might result in loss of durability but not in a corrupt database file."

**WAL docs** (https://www.sqlite.org/wal.html §2.3):
> Writers sync the WAL on every transaction commit if PRAGMA synchronous is set to FULL but omit this sync if PRAGMA synchronous is set to NORMAL.

With NORMAL, "the checkpoint is the only operation to issue an I/O barrier or sync operation", so running checkpoints on a separate thread means "the main thread ... will never block on a sync operation".

### macOS caveat (verified)

- `man 2 fsync` on this Mac (macOS 27.0.1) says fsync "will flush all data from the host to the drive", but "the drive itself may not physically write the data ... and it may be written in an out-of-order sequence ... if the drive loses power or the OS crashes, the application may find that only some or none of their data was written." It points to `F_FULLFSYNC` for databases. `man 2 fcntl` documents `F_FULLFSYNC` (asks the drive to flush and acts as a barrier) and `F_BARRIERFSYNC` (fsync plus a barrier, with no guarantee about what is persisted on return).
- SQLite source, `src/os_unix.c` `full_fsync()` (trunk 3.55.0, https://github.com/sqlite/sqlite/blob/master/src/os_unix.c around lines 3778-3843): when `HAVE_FULLFSYNC` is set it calls `fcntl(fd, F_FULLFSYNC)` **only if `fullSync` is requested**, and otherwise calls plain `fsync()`. On `__APPLE__` without FULLFSYNC it uses `fsync`, not `fdatasync`.
- `PRAGMA fullfsync` and `PRAGMA checkpoint_fullfsync` both **default to off**. "Only Mac OS X supports F_FULLFSYNC." When `fullfsync` is on, `checkpoint_fullfsync` is irrelevant (https://www.sqlite.org/pragma.html#pragma_fullfsync, #pragma_checkpoint_fullfsync).
- **Consequence [inference, from the above]:** with a stock or bundled SQLite (rusqlite `bundled`, sqlx) on macOS, `synchronous=FULL` without `PRAGMA fullfsync=ON` does **not** make commits durable across a power loss, because the drive cache is not flushed. To get the "survives power cut" claim on macOS, FULL needs `fullfsync=ON`. On Linux, `fdatasync` is used and does flush the device cache on drives that honour flush.
- Apple's system `libsqlite3` reportedly differs: it historically enabled `checkpoint_fullfsync` by default (Buildbot 2012, https://lists.buildbot.net/pipermail/commits/2012-March/010205.html), and is reportedly patched to use `F_BARRIERFSYNC` (Apple developer forum reply, via https://mjtsai.com/blog/2022/02/17/apple-ssd-benchmarks-and-f_fullsync). **Unverified; this does not matter if Automan bundles SQLite.**

### Other SQLite-based systems

- **Litestream** recommends NORMAL in WAL: "fsync calls can be relaxed in exchange for durability without risking data corruption ... This is safe as the WAL file is append only" (https://litestream.io/tips/#synchronous-pragma). Replication is asynchronous, so Litestream adds its own data-loss window.
- **rqlite** (v10.5.3) opens SQLite with **`_sync=0` (synchronous=OFF)** and WAL (`db/state.go` lines 297-310, https://github.com/rqlite/rqlite). It switches temporarily to `synchronous=FULL` for each checkpoint (`db/db.go` lines 555-662). Durability comes from the Raft log, not from SQLite. **[inference]** The pattern is: put durability in an append-only log, and treat SQLite as a rebuildable materialization synced at checkpoints.
- **Turso** (the Rust rewrite, workspace 0.8.0) supports only `synchronous = OFF | FULL`; "Only `OFF` and `FULL` are supported in Turso" (https://github.com/tursodatabase/turso/blob/main/docs/sql-reference/pragmas.mdx). Its MVCC mode has a leader/waiter **group commit** (`core/mvcc/database/group_commit.rs`).
- **Crawlee Python SQL client** and **Scrapy main `DiskDupeFilter`** both use WAL+NORMAL (see section 1).
- **DBOS** and **spider-rs** leave SQLite at DELETE+FULL by default (see section 1).
- **PostgreSQL** is the closest analogue to a "fast mode". `synchronous_commit=off` ("asynchronous commit") can lose the most recent transactions if the database crashes, but does not corrupt. The docs contrast this with `fsync=off`, which can corrupt (https://www.postgresql.org/docs/current/wal-async-commit.html). Unlike SQLite NORMAL, PG async commit can lose data even when only the database *server process* crashes, because the WAL may still be in shared memory.

---

## 3. Measured cost of FULL vs NORMAL (WAL, batched commits)

### Own measurement (this machine; script at `scratchpad/bench/bench.py`)

Setup:
- Apple Silicon Mac, macOS 27.0.1 arm64, internal APFS SSD (94% full).
- Python 3.13.11 `sqlite3`, SQLite 3.51.0.
- One writer, `journal_mode=WAL`, `BEGIN IMMEDIATE ... COMMIT`.
- Rows of about 600 B with a UNIQUE `url` index, which looks like a frontier insert.
- Median of 3 runs. Default `wal_autocheckpoint` (1000 pages).

Clean run, with no concurrent processes:

| Rows/txn | NORMAL commits/s (rows/s) | FULL commits/s (rows/s) | FULL + `fullfsync=ON` commits/s (rows/s) |
|---|---|---|---|
| 1 | 10,976 (10,976) | 3,697 (3,697) | 153 (153) |
| 10 | 4,098 (40,980) | 2,676 (26,757) | 191 (1,908) |
| 100 | 723 (72,303) | 543 (54,270) | 104 (10,407) |
| 1000 | 50 (50,132) | 72 (71,965) | 46 (46,117) |

How to read it (measurement facts, then **[inference]**):
- On macOS, plain `fsync` (FULL without fullfsync) costs about 0.2-0.6 ms per commit. That is cheap because it does not flush the drive cache, which is why it is not power-safe (section 2).
- `F_FULLFSYNC` costs about **6-7 ms per commit** (about 150 commits/s), so one-row commits are about 70x slower than NORMAL.
- With batching the gap closes. At 100 rows/txn, FULL+fullfsync reaches about 10k rows/s against about 72k rows/s for NORMAL. At 1000 rows/txn, all three modes are within noise of each other (46-72k rows/s). The cost is per commit, not per row.
- Noise is large between runs. An earlier run contaminated by a concurrent process was discarded. Other clean-ish runs gave 1-row NORMAL at 4-11k/s and fullfsync at 120-160/s. Python per-statement overhead caps the 1-row NORMAL figure. A Rust prepared-statement loop would show a bigger NORMAL advantage at batch 1.
- **Not measured: Linux** (ext4 or xfs with `fdatasync` on NVMe). There, FULL flushes the device cache on every commit, which is comparable to `F_FULLFSYNC`. The cost depends heavily on whether the drive has power-loss protection.

### Published numbers (secondary, not verified at source)

- A write-up attributed to Travis Horn and reported by Sesame Disk (https://sesamedisk.com/sqlite-performance-tuning-high-throughput/) used 10,000 autocommit single-row inserts. WAL+FULL reached about 442 inserts/s and WAL+NORMAL about 33,135 inserts/s, roughly 75x. Hardware and OS are not stated, and the original post was not found.
- Apple SSD cache-flush cost: Hector Martin (2022, summarized at https://mjtsai.com/blog/2022/02/17/apple-ssd-benchmarks-and-f_fullsync) found Apple SSDs "more than 3 times slower than a lowly SATA SSD at flushing" their cache, while being faster without flushes.
- No authoritative sqlite.org benchmark comparing FULL and NORMAL was found.

---

## 4. Evidence for a different default or design

- **Group commit is how fsync-per-commit systems make FULL affordable.**
  - Restate fsyncs its WAL on every *batch*, with up to 5000 records per commit (section 1).
  - Firecrawl tunes Postgres `commit_delay`/`commit_siblings` explicitly to "reduce fsync overhead" (section 1). The PG docs describe `commit_delay` as group commit that stays a synchronous commit (https://www.postgresql.org/docs/current/runtime-config-wal.html, https://www.postgresql.org/docs/current/wal-async-commit.html).
  - Turso implements MVCC group commit.
  - SQLite has only one writer and no built-in cross-connection group commit. **[inference]** In SQLite, group commit means one writer task that drains a channel and commits many logical operations per transaction. Under FULL, the fsync cost per logical operation then falls by roughly the batch size, which the section 3 numbers show directly.
- **Durability in the log, SQLite as materialization:** rqlite runs SQLite with synchronous=OFF and gets durability from Raft (section 2). That is an alternative design if Automan's journal becomes the source of truth.
- **What competitors actually ship:**
  - None of the crawlers (Crawlee JS/Py, Spider, Scrapy, Colly, Katana) fsync run state by default.
  - Crawlee and Scrapy-main pick WAL+NORMAL whenever they use SQLite.
  - Only the durable-execution engines (Restate, DBOS with default SQLite FULL, Postgres-backed Temporal, Hatchet, Firecrawl) fsync commits by default.
  - **[inference]** A FULL default would put Automan above every crawler competitor on durability, and level with the durable-execution engines.
- **macOS:** a FULL default is only power-safe on macOS with `fullfsync=ON`, which costs about another order of magnitude (section 3). sqlite.org leaves `fullfsync` off by default.
- **Retries make loss tolerable:** crawling at-least-once with idempotent re-fetch means losing the last N frontier updates usually costs re-work rather than incorrect results. **[inference]** Exceptions are non-idempotent browser actions and anything Automan journals as having happened externally, such as a form submission.
