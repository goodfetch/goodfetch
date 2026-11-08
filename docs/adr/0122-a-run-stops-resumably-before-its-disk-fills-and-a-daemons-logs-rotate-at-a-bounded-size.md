# A Run stops resumably before its disk fills, and a Daemon's logs rotate at a bounded size

The resource governor samples the free space of every filesystem a Run writes to. It works at three storage levels, `normal`, `low` and `critical`:
- **At `low`,** a Run sheds the writes it can do without (Run trace video, new Response cache entries, periodic Checkpoints onto that filesystem) and carries a `storage_low` warning.
- **At `critical`,** every Run whose essential writes are on that filesystem stops resumably, with the stop reason `resources` and `resource: "storage"`. It keeps enough room to record the stop, and a start with its ID resumes it once space is freed.

A write that fails before a sample sees it rolls back, is never acknowledged, and stops the Run the same way. WARC files keep only whole, committed records.

Logs are bounded as follows. `goodfetch daemon run` logs to stdout for its platform to rotate. Every other Daemon writes JSON lines to files it rotates at 10 MiB, keeping five. The default level is `info`, which logs state changes and never one line per request.

**What this answers.** The [pitfall catalog](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md)'s rows O03 ("the disk fills under a running crawl") and O10 ("log volume on long runs"), from items 4.B6 and 4.B13 of the [common pitfalls research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md). [ADR 0116](0116-a-saved-file-is-written-whole-or-not-at-all-and-a-failed-write-says-whether-its-path-or-its-storage-failed.md) covers the files a program saves, and [ADR 0040](0040-one-resource-governor-per-engine-sized-from-measured-load.md) covers memory. Neither says what a running Run does when its own writes fail, and ADR 0037's RT-17 left logs to a spec.

**Comparable projects, re-read on 2026-10-16.**
- **Heritrix 3's `DiskSpaceMonitor`** checks free space on each stats snapshot. It requests a crawl pause when free space on any monitored path drops below `pauseThresholdMiB`, 8192 by default, and a person resumes the crawl ([bean reference](https://heritrix.readthedocs.io/en/stable/bean-reference.html)).
- **Elasticsearch** has three disk watermarks, low 85 %, high 90 % and flood stage 95 %. At flood stage it blocks writes with `index.blocks.read_only_allow_delete` and releases the block automatically below the high watermark ([cluster settings](https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-cluster.html)).
- **etcd** raises a `NOSPACE` alarm that leaves only reads and deletes until an operator disarms it ([maintenance](https://etcd.io/docs/v3.5/op-guide/maintenance/)).
- **PostgreSQL** says a full data disk "will not result in data corruption", and advises acting before the disk is full ([disk usage](https://www.postgresql.org/docs/current/diskusage.html)).
- **SQLite** may roll back the whole transaction on `SQLITE_FULL` or `SQLITE_IOERR`, and advises an explicit `ROLLBACK` ([transactions](https://www.sqlite.org/lang_transaction.html)).
- **Kubernetes** evicts pods below 10 % `nodefs.available` or 5 % `nodefs.inodesFree` ([node-pressure eviction](https://kubernetes.io/docs/concepts/scheduling-eviction/node-pressure-eviction/)).
- **Scrapy and Crawlee** have no disk guard. Scrapy's `LOG_LEVEL` defaults to `DEBUG`, and its `LOG_FILE` is appended to and never rotated ([settings](https://docs.scrapy.org/en/latest/topics/settings.html)).
- **Rotation by the service manager.**
  - The kubelet rotates container logs at 10 Mi and keeps 5 files ([logging](https://kubernetes.io/docs/concepts/cluster-administration/logging/)).
  - journald caps its files at 10 % of the filesystem, at most 4G ([journald.conf](https://man7.org/linux/man-pages/man5/journald.conf.5.html)).
  - Docker's default `json-file` driver performs no rotation, and Docker's docs advise the `local` driver "to prevent disk-exhaustion" ([configure](https://docs.docker.com/engine/logging/configure/)).
- **`tracing-appender`** 0.2.5 rotates by time only, and its `max_log_files` keeps the last `n` files ([Builder](https://docs.rs/tracing-appender/0.2.5/tracing_appender/rolling/struct.Builder.html)).

Decided in [A disk that fills under a long Run, and log volume](https://github.com/goodfetch/goodfetch/issues/553), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/553#issuecomment-6204412536), its [first addendum](https://github.com/goodfetch/goodfetch/issues/553#issuecomment-6204734833), its [second addendum](https://github.com/goodfetch/goodfetch/issues/553#issuecomment-6204844106) and its [third addendum](https://github.com/goodfetch/goodfetch/issues/553#issuecomment-6206021043). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). It holds in both Products: the storage levels, the stops and the logs are the engine's, and only WARC output and the per-domain Politeness are Foxhound's (ADR 0084); the Politeness file also holds Collie's per-proxy limits (ADR 0088).

## Decision

### The storage levels

- **What is sampled.** The resource governor of ADR 0040 also samples, at each of its samples, every filesystem holding a Run's writes:
  - the run file's folder, or the per-install database (ADR 0002);
  - the Goodfetch home, with the Usage ledger, the Politeness file, the Run index, the Response cache and the log folder;
  - the folder of the Run trace and of WARC output;
  - the engine's temporary folder.

  Each filesystem is sampled once, however many of these it holds.
  > Note (agent decision, pending owner review), 2026-10-23, [#1566](https://github.com/goodfetch/goodfetch/issues/1566): the engine's temporary folder is the OS's (`std::env::temp_dir`). Two folders are on one filesystem when they share a Unix device or a Windows volume, read on each folder's nearest existing parent. A Run's essential writes are on its run folder's filesystem and the Goodfetch home's (the Usage ledger and the Politeness file); its `storage` in `run.progress` covers those and the temporary folder, and is published when its worst level changes. The stop names the Run's folder when its run file's filesystem is `critical`, else the Goodfetch home; a failed Usage ledger or Politeness write names that file's folder. `storage_low` is handed over once per folder a Run meets at `low` or worse. A start, a resume and a Checkpoint read their destination's filesystem afresh rather than wait for a sample. Run trace video, the Response cache, Exports, Run trace copies and the "no trace" mark do not exist yet in the engine, so nothing of theirs is shed or refused today; each area that adds one adds its rule.
  > Note (agent decision, accepted by the owner on 2026-10-28, deep review of M1 (583-2, 583-7, 213-9, O30 (a1) and (b1))), 2026-10-29, [#1566](https://github.com/goodfetch/goodfetch/issues/1566): in the per-install layout a Run's run state is the install database, so its essential writes are on the per-major folder's filesystem and the Goodfetch home's, its run folder holding only its lock, page bodies and default Checkpoint; the per-major folder is sampled beside the run folders, the home and the temporary folder, and a stop for run state names it. A start and a resume are refused with `resources_exhausted` while either essential filesystem is at `critical`, the first found named, run state's before the home's. `storage_low` is handed over once per folder across the Run's run epochs: a resume hands over none for a folder the report already names.
- **What is read.** The space and the inodes available to the engine's user: `statvfs`'s `f_bavail` and `f_favail` on Unix, and `GetDiskFreeSpaceExW`'s space available to the caller on Windows, which honours quotas. A per-user quota on Linux, which `statvfs` does not show, is met as a failed write (below).
- **Three storage levels,** set by two settings of a new `storage.*` area (ADR 0083), each a size in bytes, applying where the engine process reads its configuration (ADR 0028) and capped by Operator policy. Left unset, each takes its built-in default, computed per filesystem:

  | Level | Free space below | Default |
  |---|---|---|
  | `low` | `storage.low_free` | the smaller of 5 GiB and 10 % of the filesystem |
  | `critical` | `storage.critical_free` | the smaller of 1 GiB and 2 % of the filesystem |

  - **Inodes.** Free inodes go by the same percentages, where the filesystem reports them.
  - **Why two levels.** Two levels follow Elasticsearch's watermarks and ADR 0040's pressure and critical.
  - **Why the smaller of two values.** The percentage keeps a small disk usable, and the fixed size keeps a large one from stopping with hundreds of gigabytes free. Heritrix's 8 GiB alone would stop a crawl on a 16 GB disk before it starts.
  - A level drops back as soon as a sample finds the space again.
  - A level set to `0` is off, and a Run then meets only a failed write (below). The report's `weakened` lists it, as it lists any setting that weakens a safeguard (ADRs 0028, 0102).
  - The defaults keep the user's machine usable and the Run resumable with nothing set (G5).
- **Where it shows.**
  - `run.progress()` carries `storage`: the worst level among the filesystems the Run writes to, with that filesystem's folder and free space.
  - A change of level is logged at warning, or at info on the way back.
  - The Console's "Needs you" lists a filesystem at `low` or `critical`, with its folder.

### At `low`: shed what a Run can do without

Nothing stops at `low`, and admission goes on. Holding new work, as ADR 0040 does under memory pressure, would stall for ever, since a full disk never frees itself, unlike memory. Instead:
- **The Run trace** stops recording video. Snapshots and the HAR go on, in the order ADR 0031's size cap already sheds them.
- **The Response cache** stores no new entry. It evicts nothing beyond its own cap, so `replay` (ADR 0046) keeps serving what it holds.
- **A periodic Checkpoint** whose location is on that filesystem is skipped, since a copy is as large as the run file and could spend the room kept for the stops between two samples; the run file still holds the state. A Checkpoint on another filesystem is still written, and an explicit `run.checkpoint()` still writes until `critical`.
- **The warning.** The Run carries the warning `storage_low` in its report, naming the folder, the free space when it was raised and what was shed (G1, G6).

### At `critical`: every Run there stops resumably

- **Which Runs.** Every Run with an essential write (below) on that filesystem stops at its next safe point, as ADR 0040's last resort stops a Run.
  - It is `stopped` and `incomplete` (ADR 0029).
  - Its report's `stopReason` is `resources`, with `resource: "storage"` and the folder.
  - A start with its ID resumes it from its run file once space is freed (ADR 0102).
  - Unlike ADR 0040's one Run at a time, every such Run stops together, because stopping one Run frees no disk.
  - **Held pages keep ADR 0040's rule.** A page under a person's control in the Live view is never ended by the engine. Its Run admits nothing new and stops once that person hands back or leaves, unless a write fails first, which stops it as below. A page waiting in a Human pause with nobody on it ends with its Run, as ADR 0040 has it.
- **No Checkpoint copy.** The stop writes no Checkpoint copy onto a filesystem at `critical`, since a copy is as large as the run file; the run file on this machine is what resumes it. A Checkpoint whose location is on another filesystem is still written, as ADR 0002's clean stop has it. The report's `checkpoint` names the last one written.
- **The reserve stays for the stops.**
  - A new Run whose run file would go to that filesystem is refused at its start with `resources_exhausted`.
  - An Export, a Checkpoint or a Run trace copy that the engine would write there fails at once with `file_write_failed`, side `engine`, reason `storage_full` (ADR 0116), before writing a byte.
  - The Run trace of any Run on that filesystem stops, and the Run carries the "no trace" mark (ADR 0031).
  - `/readyz` fails while the filesystem holding run state is at `critical` (ADR 0050).
    > Amended by the owner on 2026-10-28 (deep review of M1, O30 (b1)): `/readyz` also fails while the Goodfetch home's filesystem is at `critical`, since its Usage ledger and Politeness file are essential writes of every Run, and a Daemon that can run no Run is not ready.
- **No automatic resume.** A crawl's handlers and a Collie program's steps run in the program, so the engine cannot resume a Run alone. The stop raises ADR 0027's Notification for Completeness `incomplete`, which a storage stop always is, and the Run is listed in "Needs you". A person frees space, and a start with the Run's ID resumes it (ADR 0102). Elasticsearch releases its block automatically because an index has no program behind it.
- **Expected, not thrown.** As for any stop, `finish()` returns the report saying `stopped` (ADR 0102; G1).

### A write that fails before a sample sees it

A quota, another process filling the disk between two samples, or a filesystem that misreports its space makes a write fail with no warning.
- **The run file.** The writer issues `ROLLBACK` after `SQLITE_FULL` or `SQLITE_IOERR`, as SQLite advises, so the state is the same after any of them. Nothing in the failed commit is acknowledged.

  > Note (agent decision, pending owner review), 2026-10-28, deep review 213-1: the writer also treats `SQLITE_CORRUPT` as a failed write, since SQLite's Unix VFS reports a read that meets EIO as `SQLITE_IOERR_CORRUPTFS`, which reaches the caller as `SQLITE_CORRUPT`, and the two cannot be told apart. A write of a group that meets any of these, or finds that SQLite ended the transaction, ends the group: the writes after it never run, since each would commit on its own outside the ended transaction, and every write of the group is answered with the storage failure.
- **Then the Run stops as at `critical`,** from its last acknowledged commit:
  - its in-flight work is redone on resume, delivered at least once and made visible once (ADR 0003);
  - an Effect whose journal entry was not committed is checked first on resume, or reported uncertain, as after a crash (ADRs 0010, 0029, 0106).
  - It attempts no commit beyond the small one that records the stop.
- **When even that commit fails,** the Run is left interrupted, with no live writer, as after a crash. A start with its ID resumes it (ADR 0102). The call waiting on the Run (`finish()`, `stop()`, Rust's `serve`) raises `resources_exhausted`, since no report was recorded to return, and a view's `report()` raises `run_interrupted` (ADR 0102).
- **The same rule for every essential write.** A failed write of the Usage ledger, the Politeness file or WARC output stops the Runs it serves in the same way.
- **The file is never damaged.** A full disk never corrupts the run file: SQLite rolls back, and the crash VFS proves it (ADR 0060).

### Which writes are essential

| Write | Essential | At `low` | When its write fails |
|---|---|---|---|
| The run file, or the per-install database (ADR 0002) | yes | written | stops the Run |
| The Usage ledger: Limits depend on it (ADR 0041) | yes | written | stops every Run of the engine |
| The Politeness file: no request it limits is sent without its permit, per domain or per proxy (ADRs 0053, 0088) | yes | written | stops every Run of the engine that takes its permits |
| WARC output (CO-30): the Run's own output, as its Records are | yes | written | stops the Run |
| A periodic Checkpoint (ADR 0002) | no | skipped on that filesystem | skipped, with a `file_write_failed` warning; the run file still holds the state |
| The Run trace (ADR 0031) | no | no video | stops for the Run, which carries the "no trace" mark |
| A Response cache entry (ADR 0046) | no | not stored | not stored |
| The Run index row (ADR 0031) | no | written | written later, and `goodfetch runs reindex` rebuilds it |
| An engine's log lines | no | written | dropped and counted (below) |

- **WARC files hold only whole, committed records.**
  - The run file commits each WARC file's length together with the attempt whose records it holds.
  - A resume cuts each WARC file back to its committed length, so neither a torn record nor the capture of a request the stop left uncommitted stays in it. The request is fetched again and captured once, since ADR 0003's outputs "become visible exactly once".
  - Heritrix instead leaves `.warc.gz.open` files that a crash may truncate and a person must validate.
- **No new per-Run byte cap.** The parts a Run can do without already have caps: a Run trace has 1 GB per Run (ADR 0031), the Response cache 10 GB per user (ADR 0046), and logs the cap below. The run file and WARC output are the Run's own output, bounded by its Limits (ADR 0041). A byte cap per Run would stop Runs on an arbitrary number while the disk still has room.

### `resources_exhausted`

ADR 0040 named the code without a group. It is an `UnavailableError`, not retryable: the machine's memory or storage is what Goodfetch depends on, and the setup must change (ADR 0079).
- **Raised** only where no report says it: a start refused at `critical`, and the waiting call of a Run whose stop could not be recorded. Otherwise the Run's report carries the stop.
- **Its details:**
  - `resource`: `memory` or `storage`; a value the SDK does not know is kept as a string (G3).
  - For storage:
    - `path`: the folder on the full filesystem, or `null` where the engine has no folder of its own (Goodfetch Cloud, as in ADR 0116);
    - `freeBytes`;
    - `limit`: the setting that set the level, with its value and layer, as ADR 0079's `limit` names one.
- **Its `fix` (G6)** names the folder to free, or `storage.critical_free` to lower, and then a start with the Run's ID.

### Logs

- **`goodfetch daemon run`** keeps ADR 0049's JSON lines on stdout, for the platform to collect and rotate: the kubelet, journald, or the container runtime.
  - The Compose files `goodfetch init` writes, and the docs' Docker and Compose guides, set the `local` logging driver, since `json-file` does not rotate by default.
  - The Helm chart relies on the kubelet's rotation.
- **Every other Daemon writes its own files.** That covers a Daemon started on demand (ADR 0024), by `goodfetch daemon start`, by launchd or systemd activation, or by the Windows logon task (ADR 0031).
  - They are JSON lines, in the Goodfetch home's log folder, one set of files per Daemon.
  - A file rotates when it reaches `log.file.max_size` (10 MiB), and `log.file.max_files` (5) are kept, the kubelet's defaults. A Daemon's logs therefore stay under about 50 MiB. The rotation is by size, because a quiet day and a busy hour then cost the same.
  - The docs name the folder for each OS.
- **Engines that write no log file by default.** An Attached engine sends its logs only as log events (ADR 0078), and writes JSON lines to the Goodfetch home's log folder only when `log.output` is `file` in its configuration, as test support sets it for ADR 0072's exercise audit. A Rust program's linked engine logs through the program's own `tracing` subscriber. The CLI's output is the CLI's design.
  > Agent decision, accepted by the owner on 2026-10-21 (#353): until the engine reads its configuration and the `log.*` settings (#1566), the executable's `GOODFETCH_LOG_OUTPUT` stands in for `log.output`, as `GOODFETCH_LOG` already stands in for `log.level`: `file` writes the JSON lines to `<Goodfetch home>/logs/<executable>-<process ID>.jsonl` beside the text on stderr, `none` or unset writes no file, and any other value fails the start. `TestEnv`'s map sets `GOODFETCH_LOG_OUTPUT=file` and a `GOODFETCH_LOG` that adds the dispatch records at `trace`, the level `rpc` records them at today. Both variables go when the settings arrive, and test support then writes `log.output: file` and the level into the test's configuration instead.
- **The default level is `info`,** and `info` logs state changes only. Examples are a Run started, resumed or stopped, a domain backing off (ADR 0102), a storage level changing, and a Daemon draining. A line per request, per fetch or per command is `debug` or below. So at the default level the volume follows what happens to Runs, never the number of requests; Scrapy's `DEBUG` default is the opposite. What a Run did per request stays in its run state and its Run trace, read through `outcomes()` and the Console (ADR 0102).
- **Settings,** in a new `log.*` area, applying where an engine process reads its configuration (ADR 0028):

  | Key | Default |
  |---|---|
  | `log.level` | `info`, or a `tracing` filter directive |
  | `log.output` | `stdout` under `daemon run`; `file` for every other Daemon; `none` for an Attached engine, whose logs go as log events |
  | `log.format` | `json`; `text` for a person reading a terminal |
  | `log.file.max_size` | 10 MiB |
  | `log.file.max_files` | 5 |

  - They apply to any engine process, read from its configuration (ADR 0028). `log.output: stdout` on an Attached engine, whose stdout is its protocol channel, fails with `setting_not_applicable` (ADR 0083), and `log.file.*` apply wherever `log.output` is `file`.
  - The rpc dispatch records ADR 0072's exercise audit reads (each request's method and Error code, each event's type) are per-request lines, so they are `debug`: test support sets `log.level` to include them.
  - An SDK still sets its connection's level through its host's logger and never logs on its own (ADR 0078; G4).
  > Note (agent decision, pending owner review), 2026-11-08: a person's command that runs on an Attached engine logs in its own process, so the rpc dispatch records reach its stderr, and under `log.output = file` its `<executable>-<process ID>.jsonl`, whenever `log.level` includes them: `trace`, or a directive naming `goodfetch::rpc::dispatch`. `config explain` is such a command since it calls `config.explain` on an engine (ADR 0083's note of 2026-11-08). The records hold each request's method and Error code, never a value, and the default level leaves them out; stdout, and so `--json`, stays the command's alone.
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#257](https://github.com/goodfetch/goodfetch/issues/257): `log.format` arrives first, as `text` or `json`, set in user configuration or an Operator policy only, and governs the logs on stderr, or on stdout under `daemon run`; a log file stays JSON lines. Its default is unset, which keeps text on stderr, where a person reads a CLI command's output and the CLI's output is its own design, and JSON lines on stdout under `daemon run`, rather than `json` everywhere. `GOODFETCH_LOG` and `GOODFETCH_LOG_OUTPUT` still stand in for `log.level` and `log.output` until #1566, and every timestamp reads the engine's `Clock`.
  > Note (agent decision, accepted by the owner on 2026-10-28, deep review of M1 (213-O2)), 2026-10-23, [#1566](https://github.com/goodfetch/goodfetch/issues/1566): the `log.*` settings replace both variables, which leave ADR 0083's list, and test support writes `log.output = "file"` and its `log.level` into each test's user configuration. `log.output` names where an engine process writes its JSON lines beside its own stream (stderr, or stdout under `daemon run`): `stdout` is `daemon run`'s alone, and fails with `setting_not_applicable` in any other process, an Attached engine and the CLI's commands included, whose stdout is their protocol or their output; `file` under `daemon run` adds the files to its stdout. A Daemon's files are `daemon-<major>.jsonl`, rotated to `daemon-<major>.1.jsonl` and on, one set for the Daemon across its restarts; any other process's are `<executable>-<process ID>.jsonl`, a set of their own, so concurrent processes never share a file. At its start such a process keeps the newest `log.file.max_files` sets of its executable, by when each set was last written, its own included, and removes the older sets, so the folder stays bounded as a Daemon's does. A file rotates when the next line would take it past `log.file.max_size`. `log.level` is read when the process starts, and a directive `tracing` cannot read fails the start with `setting_invalid`. The count of dropped lines is a JSON line of its own, `fields.dropped`, written just before the next line.
- **A log write never blocks or stops anything.** The writer is lossy and non-blocking. A line it cannot write, because its buffer is full or the disk is, is dropped and counted, and the next line written says how many were dropped. That is ADR 0078's rule that logs are telemetry, "never slowing a run".

## Considered Options

- **Failing the Run at the first failed commit:** rejected. `failed` blames the Run for the machine, and with no space kept back the stop itself cannot be recorded (G1).
- **Pausing the Run until space returns, as Heritrix does:** rejected. A paused Run holds browsers, leases and memory for as long as nobody acts, and ADR 0037's RT-11 already makes a pause a checkpoint, a stop and a resume.
- **Acting only when a write fails:** rejected as the only guard. By then the stop's own commit fails too. It stays as the backstop.
- **Holding admission at `low`, as ADR 0040 does under memory pressure:** rejected. Nothing frees disk by itself, so the Runs would wait for ever without stopping.
- **Stopping one Run at a time, the largest first, as ADR 0040 does:** rejected. Stopping a Run frees no disk.
- **A fixed threshold only** (Heritrix's 8 GiB) **or a percentage only** (Elasticsearch, Kubernetes): rejected. The first stops a small disk before it starts, and the second stops a large one with hundreds of gigabytes free.
- **Resuming automatically once space returns** (Elasticsearch's release): rejected. A Run's handlers are in its program, and only a start with its ID brings them back (ADR 0102).
- **Dropping WARC records with a warning:** rejected. WARC is what a crawl that asks for it exists to produce, and a silent gap is G1's failure that looks like a result.
- **Evicting Response cache entries to free space:** rejected. It would break `replay` without warning, and the cache already has its own cap.
- **A byte cap per Run:** rejected. It stops Runs on an arbitrary number while the disk has room, and Limits already bound a Run's work.
- **Logging only to stdout or stderr:** rejected. A Daemon started on demand, by launchd or by a Windows logon task has no manager that keeps or rotates its output.
- **Always writing the engine's own files:** rejected. In a container they would duplicate what the platform already collects and rotates, on a filesystem that may be ephemeral.
- **Daily rotation, as `tracing-appender` offers:** rejected. A busy day's file is unbounded.
- **`debug` by default** (Scrapy): rejected. It costs CPU and disk per request, which Scrapy's own [broad crawls guide](https://docs.scrapy.org/en/latest/topics/broad-crawls.html) tells users to switch off.

## Consequences

- **Adds to ADR 0040.**
  - The governor samples free storage.
  - At `critical`, every Run with an essential write on that filesystem stops together, with `resource: "storage"`, and writes no Checkpoint copy there.
  - `resources_exhausted` is an `UnavailableError`, not retryable, with `resource` in its details.
  - A note sits under its last resort.
- **Adds to ADR 0116.** At `critical`, an Export, a Checkpoint or a Run trace copy that the engine would write on that filesystem fails at once with `file_write_failed`, side `engine`. A note sits under its engine's side.
- **Amends ADR 0102.**
  - The report's `stopReason` `resources` carries `resource` and, for storage, the folder.
  - `run.progress()` carries `storage`.
  - `storage_low` joins the report's warnings.
  - The call waiting on a Run whose stop could not be recorded raises `resources_exhausted`.
  - Notes sit under its report's fields and its `progress()`.
- **Amends ADR 0002.** A stop at `critical` writes no Checkpoint copy onto the full filesystem, a periodic Checkpoint onto a filesystem at `low` is skipped, and a periodic Checkpoint whose write fails is skipped with a warning. A note sits under its Checkpoints bullet.
- **Amends ADR 0031.**
  - At `low`, a Run trace stops recording video, and at `critical` or on a failed write it stops, with the "no trace" mark.
  - "Needs you" lists a filesystem at `low` or `critical`, and a Run stopped by storage.
  - A note sits under its Run traces' size cap and names both.
- **Amends ADR 0046.** At `low`, the Response cache stores no new entry. A note sits under its size cap.
- **Amends ADR 0050.**
  - `/readyz` also fails while the filesystem holding run state is at `critical`.
  - The metrics gain each sampled filesystem's free space and storage level, labelled by what it holds (run state, home, trace, WARC, temporary), a bounded set.
  - A note sits under `/readyz`.
- **Adds to ADR 0078.** An Attached engine also writes JSON lines to the Goodfetch home's log folder when `log.output` is `file` in its configuration; it still sends its logs as log events, and its stderr is unchanged. A note sits under its stderr bullet.
- **Adds to ADR 0072.** An Attached engine writes the structured log its exercise audit reads when test support sets `log.output: file` and a `log.level` that includes the rpc dispatch records.
- **Adds to ADR 0049.** `init`'s Compose files and the Docker and Compose guides set the `local` logging driver, and every Daemon outside `daemon run` writes its own rotated files.
- **Adds to ADR 0083.** The `storage.*` and `log.*` areas join the settings schema.
- **ADR 0037's matrix:**
  - RT-17 (structured logging) becomes `decided`, citing ADRs 0078 and 0122;
  - RT-03 (resource budget) cites this ADR for storage.
- **`TESTING.md`** gains the storage cases and the Daemon log cases.
- **Error codes.** `resources_exhausted` joins `goodfetch-protocol` in the `unavailable` group, not retryable, with its exit code in ADR 0069's table. It is Collie's code, so Rust has `ErrorCode::RESOURCES_EXHAUSTED` (ADR 0099). `storage_low` is a warning code, which area 3's spec lists.
- **Who builds what.**
  - **Area 3 (Engine runtime)** builds the storage sampling and its levels, the stops and the rollback on a failed write, the log writer, its rotation and its settings.
  - **Area 6 (Collection)** builds the committed WARC length and the cut back on resume.
  - **Area 13 (Self-hosted deployment and operations)** builds the logging driver in the Docker and Compose guides, the `/readyz` condition and the metrics, and documents the log folder for each OS.
  - **Area 12 (Console)** builds the "Needs you" entry.
  - Each lists this ADR.
- **The pitfall catalog's rows O03 and O10** become Handled, proved by the storage and Daemon log cases.
- **The `prototype/sdk-api` sketch** holds `resource` and its folder in `RunStopReason`, `storage` in `RunProgress`, the `storage_low` warning, and `ResourcesExhaustedError` with its details, in each language.

> Note (agent decision, pending owner review), 2026-10-28, step 7 review (Standards A3): where the Resource governor reads the filesystems the Runs write to is a value of its `System`, as where it reads the machine is (`System::linux_files`): the OS's own calls, or a `FilesystemSource` given with `System::with_filesystems`, which answers which filesystem holds a folder and what it reads. The engine's tests give it their own view of filesystems through that interface, in place of the test-only `cfg` branches the governor's storage module had.
