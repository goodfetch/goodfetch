# Sinks, container pool and simulated power cut, with Docker (D-sinks, D-cloud-topology), October 9, 2026

Three tests announced as "out of reach without a cloud" can be done in part with Docker Desktop: the sinks and the container pool fully, the power cut approximately. Measurements in `local-dev`, on a Mac in use.

## Sinks

The storage lane writes three more formats, from the same Node worker and under the same independent read-back oracle:
- **`postgres-copy`**: PostgreSQL 17.11 in a local Docker container (`postgres:17-alpine`, `synchronous_commit`, `fsync` and `full_page_writes` enabled). Each batch goes through `COPY` in a transaction, and the acknowledgment follows the `COMMIT`.
- **`duckdb-table`**: a DuckDB file fed by the appender, one commit per batch. A DuckDB file accepts only one writer process.
- **`parquet-batch`**: one Parquet file per batch, encoded by DuckDB into a temporary file, synced to disk, renamed, then the directory is synced in turn.

Retained runs:
- `storage-sinks-2026-10-09T19-55-10-753Z-e5bb86`: 5 blocks, 40 batches of 32 or 512 rows, one writer;
- `storage-sinks-crash-2026-10-09T19-55-28-446Z-b1c63c`: `SIGKILL` of the writer after all acknowledgments.

The 60 cells are verified: each acknowledged row is read back intact, including after the writer crash.

| Sink | Verified rows/s, batches of 512 | Verified rows/s, batches of 32 |
| --- | --- | --- |
| DuckDB | 515,441 | 77,352 |
| SQLite WAL `synchronous=FULL` | 392,802 | 133,069 |
| JSONL + `fsync` | 118,921 | 7,426 |
| PostgreSQL `COPY` | 70,400 | 17,083 |
| Parquet, one file per batch | 47,135 | 3,072 |

### Reading

- **Throughput depends first on batch size.** Each durable commit costs a round trip to the disk, or even to a server. At 32 rows, Parquet pays for a file, two syncs and a rename per batch; JSONL pays for a full `fsync`.
- **Durability levels are not the same on macOS, and this table does not compare equal promises.**
  - Node's `fsync` (libuv) requests `F_FULLFSYNC`, which also flushes the disk cache. This is what JSONL and Parquet pay for.
  - SQLite with `fullfsync=OFF`, the default, and probably DuckDB stop at `fsync`, which on macOS does not flush this cache.
  - PostgreSQL syncs inside Docker's Linux VM, on a virtual disk.

  This last point requires verification: equal durability must be set by configuration before any throughput comparison.
- **For Automan**, SQLite remains the local default (D-storage). DuckDB is a very good local analytical sink, but with a single writer process. PostgreSQL `COPY` serves the cloud's shared state, and its cost is dominated by the round trip. Parquet suits bulk export, not writing in small batches.

## Container pool (D-cloud-topology)

New `topology` lane. A browser session is a tenant: it visits a page that sets its cookie, then reloads it. The oracle requires three things:
- the page is served;
- the cookie is set;
- the next tenant never arrives with the previous one's cookie.

The browser is `chromedp/headless-shell:151.0.7922.109`, an image pinned by digest, driven via CDP. Retained run: `container-pool-2026-10-09T20-01-19-342Z-147c8e`, 5 blocks of 8 sessions, pool of 2. The first run is replaced: it measured the pool's memory before the replacement containers had started.

| Policy | Verified blocks | Acquisition (ms) | Full session (ms) | Leaks between tenants | Idle memory |
| --- | --- | --- | --- | --- | --- |
| fresh container per session | 5/5 | 180 | 350 | 0 | 0 |
| warm pool, container discarded after use and replaced | 5/5 | ≈ 0 | 188 | 0 | ≈ 39 MiB per container |
| pool reused without reset | 0/5 | ≈ 0 | — | **30 out of 30** | — |
| Firecracker snapshot | `unsupported` | — | — | — | — |

### Reading

- **The warm pool removes startup from the session's path**: about 180 ms, half of a short session. The price is keeping containers idle, about 39 MiB each.
- **Reusing a container without resetting it leaks one tenant's session to the next**, every time. A pool must discard or reinitialize, browser context included, before serving another tenant. The lane verifies this, and this contract must remain a gate.
- Firecracker requires `/dev/kvm`, which the Docker Desktop VM does not expose. The comparison of the two topologies awaits a Linux host with KVM; the container pool is the only option measured.

## Simulated power cut

The test proceeds as follows:
1. Writers run **inside** Docker's Linux VM, one per format, in batches of 50 rows. PostgreSQL runs in the same VM and is written from the Mac.
2. Each acknowledgment follows a commit and is recorded on the Mac side.
3. After 5 s of writing, the VM is killed with `SIGKILL` (`com.docker.virtualization`), which amounts to cutting the guest machine's power: whatever Linux had not flushed to its virtual disk is lost.
4. After Docker restarts, each acknowledged batch must be present and complete.

The Mac's disk is not cut: the test proves the sync discipline of each sink in the guest machine, not the behavior of the physical disk. There were two series of three cuts. The project owner launched the first from their terminal; I did not have authorization to cut the VM while their Supabase stack was running in it. Once that stack was stopped, I launched the second, which also measures PostgreSQL. Raw data: [`power-cut-2026-10-09.json`](power-cut-2026-10-09.json) and [`power-cut-pg-2026-10-09.json`](power-cut-pg-2026-10-09.json).

| Sink | Acknowledged batches (6 cuts) | Acknowledged batches lost | Partial batches | Integrity |
| --- | --- | --- | --- | --- |
| SQLite WAL `synchronous=FULL` | 9,108 | **0** | 0 | ok |
| SQLite WAL `synchronous=NORMAL` | 108,803 | **96** (45 then 51, at one cut in each series) | 0 | ok |
| JSONL + `fsync` | 8,774 | **0** | 0 | ok |
| JSONL without `fsync` | 134,498 | **134,498, i.e. all of them** | 0 | ok |
| DuckDB | 5,402 | **0** | 0 | ok |
| Parquet, one file per batch | 2,755 | **0** | 0 | ok |
| PostgreSQL (3 cuts of the second series) | 2,324 | **0** | 0 | the server recovers through its WAL |

### Reading

- **Each sink keeps exactly the promise of its configuration.**
  - SQLite in `synchronous=FULL`, synced JSONL, DuckDB, Parquet (temporary file, `fsync`, rename, directory `fsync`) and PostgreSQL with `synchronous_commit` lose no acknowledged batch.
  - SQLite in `NORMAL` loses acknowledged commits at one cut out of three, in each series: this mode does not sync the WAL at each commit. It withstands an application crash, not a power cut, as its documentation says.
  - JSONL without `fsync` loses everything the kernel was keeping in cache.
- **No partial batch and no corrupted database**, even for the sinks that lose data: SQLite passes `integrity_check`, PostgreSQL replays its WAL, and each batch comes back whole or not at all.
- The cut is real. DuckDB's `package.json`, written without `fsync` by `npm install` just before a preliminary cut, came back at 0 bytes.
- Two script defects were fixed along the way. The first series did not pass the values for the PostgreSQL insert, so PostgreSQL is not measured in it. Then, the connection error expected at the moment of the cut stopped the whole script; it is now ignored.

### What this says for the architecture

The architecture requires WAL `synchronous=FULL` for local state (D-storage) and acknowledgment only after a durable write. The cut confirms it: `NORMAL` or an unsynced JSONL suffice against a process crash, already tested, but not against a power cut. Any "fast" mode must be displayed as non-durable in the face of a machine failure.

## Verdicts

`pnpm lab verdict`, without eligibility (`local-dev`):

| Decision | Option vs reference | Result |
| --- | --- | --- |
| D-sinks | Parquet vs PostgreSQL `COPY` | **worse** at 512 rows (0.72); `insufficient-n` at 32 rows (0.19) |
| D-sinks | DuckDB vs PostgreSQL `COPY` | `insufficient-n`: 7.2 times the throughput at 512 rows, 3.8 times at 32 |
| D-cloud-topology | Firecracker vs container pool | blocked by contract: not measurable without KVM; the container pool maintains isolation |

The blocks come from both runs: 5 throughput blocks and 1 from the crash run, which writes the same load. The gates pass for all sinks: no acknowledged row is lost, including after the writer crash.

## Limitations

- `local-dev` host, Docker Desktop. Throughputs and durations are indicative; PostgreSQL and the containers go through the VM.
- The sinks' durability levels are not equalized (see above); a throughput comparison at equal durability remains to be done.
- D-sinks remains subordinate to demand: these figures are ready for the day a customer asks for a sink.
