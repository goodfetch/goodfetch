# A saved file is written whole or not at all, and a failed write says whether its path or its storage failed

When a program saves a file, the path ends up holding either the whole file or what it held before. A write that fails raises one of two Error codes, split by remedy. `path_not_writable` means the path the program chose cannot be written, so the program must change. `file_write_failed` means the storage failed: a full disk, a quota or an I/O error on the program's machine, or any failure where the engine keeps its files, so the setup must change. This covers every file a program saves (an Export, a Download, a Checkpoint, a Run trace, a screenshot or PDF given a `path`, a Plan file, a Response recording) and every file the engine writes for it to save.

**What this answers.**
- Golden scenario 05's failure point 7, "the export fails (disk full, path not writable)". It scored **generic** in all three Goodfetch programs: [ADR 0103](0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md) gives `run.export` and `saveAs` no Error code for a write that fails, and ADR 0079's groups left open which group each failure belongs to.
- The pain coverage matrix's rows P48 (failure semantics, shown by scenario 05) and P53 (files, saving downloads among them), and pain 10 of the [research on competitors' APIs](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) (stringly errors).

**Competitors, re-read at tagged releases on 2026-10-15.** None gives a failed write a code of its own: each passes the OS error through, and Scrapy only logs it.
- **Playwright 1.64.0.** `Download.saveAs(path)` creates missing parent folders. It then has the server `copyFile` to the path (a local browser), or streams the bytes into `fs.createWriteStream(path)` in the client (a remote one). A failure rejects with Node's own file error, and the remote path leaves what was written ([client/artifact.ts](https://github.com/microsoft/playwright/blob/v1.64.0/packages/playwright-core/src/client/artifact.ts#L38-L52), [artifactDispatcher.ts](https://github.com/microsoft/playwright/blob/v1.64.0/packages/playwright-core/src/server/dispatchers/artifactDispatcher.ts#L53-L65)).
- **Crawlee 3.18.2.** `exportData(path, format)` runs `ensureDir`, then `writeFile` straight onto the path, so a failure throws the raw `fs` error and can leave a partial file ([basic-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1392-L1400)).
- **Scrapy 2.19.0.** `FileFeedStorage` creates the parent folder and opens the target in append mode unless `overwrite` is set ([feedexport.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L186-L198)). A failed store is logged as "Error storing …" and counted, and the crawl still finishes ([feedexport.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L617-L626)).
- **Rust std 1.90** names these failures portably in `io::ErrorKind`: `StorageFull`, `QuotaExceeded`, `PermissionDenied`, `ReadOnlyFilesystem`, `IsADirectory`, `NotADirectory`, `InvalidFilename` and `FileTooLarge`, stable since 1.83 ([io/error.rs](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/io/error.rs#L231-L380)). Node puts the OS's name on `err.code` ([common system errors](https://nodejs.org/api/errors.html#common-system-errors)), and Python raises `OSError` subclasses with `errno`.
- **Atomic replacement** is the ecosystems' practice: write a temporary file in the same folder, then rename it over the path. tempfile 3.23.0's `persist` "will atomically replace" the target ([src/file/mod.rs](https://github.com/Stebalien/tempfile/blob/v3.23.0/src/file/mod.rs#L687)), and Python's `os.replace` is atomic under POSIX ([os.replace](https://docs.python.org/3/library/os.html#os.replace)).

Decided in [Error code of a failed Export write](https://github.com/goodfetch/goodfetch/issues/527), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/527#issuecomment-6194741642), its [first addendum](https://github.com/goodfetch/goodfetch/issues/527#issuecomment-6194846735) its [second addendum](https://github.com/goodfetch/goodfetch/issues/527#issuecomment-6196232385), its [third addendum](https://github.com/goodfetch/goodfetch/issues/527#issuecomment-6196527078) and its [fourth addendum](https://github.com/goodfetch/goodfetch/issues/527#issuecomment-6196687562). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). Both codes are Collie's, in its client package, since `Download`, `Checkpoint` and `RunTrace` are Collie's; Foxhound's Export uses them as it uses the rest of that package ([ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md)).

## Decision

### Which writes it covers

- **Every write to a path the program gives.**
  - The writes:
    - `saveAs(path)` on an `Export` (ADR 0103);
    - `saveAs(path)` on a `Download` (ADR 0085);
    - `saveAs(path)` on a `Checkpoint` or a `RunTrace` (ADR 0102);
    - the `path` option of `screenshot()` and `pdf()` (ADR 0085);
    - a Run's Plan file, which the SDK writes at the `plan` path when a Run promotes a version, and when a later Run naming the file starts with a version a person accepted (ADR 0091), and its Response recording, which it writes at its path when a recording Run ends (ADR 0105).
  - These share one name or one option with one meaning: copy a file to the program's machine. They therefore fail one way (S2, S3).
  - A `screenshot()` or `pdf()` whose file write fails raises, and does not return its bytes.
  - A Plan file or a Response recording is already written atomically (ADRs 0091, 0105), and the file at the path is left as it was when its write fails.
  - **At a Run's start.** A version a person accepted is written by the call that starts the next Run naming the file (`collie.start`, `foxhound.startCrawl`; ADRs 0091, 0102), before replaying. A failed write makes that call raise its code, and the Run does no work; the accepted version stays accepted with its Run in the Run index, so the next start writes it again (G1).
  - **At the Run's end.** A promoted version and a recording are written by the call that ends the Run (`finish()`, `stop()`, `close()`, Rust's `serve`; ADR 0102). A failed write makes that call raise its code once the Run has ended (G1).
    - The Run's ending is unchanged, and its report keeps a `RunWarning` whose code is the Error code and whose message names the path. The program reads that report with `openRun(id).report()` (ADR 0102).
    - New Plan versions whose write failed stay pending with the Run for a person, as versions not promoted do (ADR 0102).
    - When the Run itself ended `failed`, that call raises the Run's own error, as ADR 0102 says, and the write's failure is only the warning.
    - What the Run keeps so a recording's write can be made again is area 11's spec.
- **Every write the engine makes of a file it hands the program:**
  - `run.export` writing the Export;
  - `run.checkpoint` writing the Checkpoint;
  - `run.trace` when it writes its copy.
- **Not `stream()`.** A program that writes the streamed bytes itself meets its own language's file errors. That is the way down (G8).
- **Not a Download that failed in the browser.** It keeps ADR 0085's `download_failed`.

### Two codes, split by remedy

ADR 0079 gives each code exactly one group, named for its remedy, and one retryable flag. One code cannot serve both "change the path" and "free space". A split by side (engine or program) does not help either: a full disk on the program's machine is not a bug in the program. ADR 0079's rule is a new type only where the remedy differs.

| Code | Group | Retryable | When |
|---|---|---|---|
| `path_not_writable` | `ProgramError` | no | The program's path cannot be written for a reason the path causes: permission denied, a read-only filesystem, a folder at the path, a file where a parent folder should be, a name the filesystem refuses, a file too large for that filesystem, or a path not found once its parents are made |
| `file_write_failed` | `UnavailableError` | no | The bytes could not be written for any other reason: no space or quota left, or an I/O error, on the program's machine; or any failure where the engine keeps its files, which the program never chose |

- **Neither is retryable.** The same call fails the same way until a person frees space or fixes the path or the setup. `run_state_unavailable` (ADR 0102), `secret_unavailable` (ADR 0092) and `not_in_product` (ADR 0087) are not retryable for the same reason.
  - A crawl handler whose `ctx.run.export` lets either code escape ends its request `failed`, with no retries spent (ADR 0093).
- **Class names.** `PathNotWritableError` and `FileWriteFailedError`, which shadow no built-in (ADR 0079).
- **The CLI.** Each code has an exit code in ADR 0069's table, so a CLI command whose file write fails exits with that code.

### What the errors carry

- **`details`:**
  - `reason`: Rust's `io::ErrorKind` name in snake_case.
    - For `path_not_writable`: `permission_denied`, `read_only_filesystem`, `is_a_directory`, `not_a_directory`, `invalid_filename`, `file_too_large` or `not_found`.
    - For `file_write_failed` on the program's side: `storage_full`, `quota_exceeded` or `other`. On the engine's side, any of them.
    - A reason the SDK does not know is kept as a string (ADR 0079).
  - `osCode`: the OS's own name for the error, such as `ENOSPC`, `EACCES` or `ERROR_DISK_FULL`, or `null`.
    > Note (agent decision, accepted by the owner on 2026-10-21), 2026-10-21, [#218](https://github.com/goodfetch/goodfetch/issues/218): a Checkpoint's write fails through the OS (`osCode` from its error number) when its file cannot be created, synced or renamed, and through SQLite when its bytes cannot be written; SQLite gives its own code, `SQLITE_FULL` read as `storage_full`, `SQLITE_READONLY` as `read_only_filesystem`, and no OS error name, so `osCode` is then `null`. `file_write_failed` has no exit code yet: no CLI command writes a Checkpoint.
  - `path`:
    - on the program's side, the program's path as given;
    - on the engine's side, the folder where the engine keeps the Run, as `run_state_unavailable` gives it, or `null` where the engine has no folder of its own (Goodfetch Cloud).
  - `side`, on `file_write_failed` only: `engine` or `program`.
- **The cause.** When the SDK runtime wrote the program's path itself, the native error is chained in the language's own way (ADR 0079): Node's system error, Python's `OSError`, or Rust's `std::io::Error` as `source()`.
  - The SDK runtime renders these errors in ADR 0079's layout, as it renders `closed`.
  - When a local engine wrote the path (below), the error comes over the protocol, as every engine error does, with no native error to chain: `reason`, `osCode` and the message carry what the OS said.
- **`fix` (G6):**
  - **`path_not_writable`** names the path argument and what to change: a folder this user can write, a file name rather than a folder, a name the filesystem accepts, or a filesystem that holds a file this size (for an Export, its `compression`, ADR 0103).
  - **`file_write_failed` on the program's side** says to free space and make the same call again: `saveAs`, since the engine still holds the file, or `screenshot()` or `pdf()` for a file written through their `path` option.
  - **`file_write_failed` on the engine's side** says to free space where the engine keeps its Runs, or to ask the Daemon's operator, and then call `export`, `checkpoint` or `trace` again.

### Written whole or not at all

- **A temporary file, then a rename (G1).** A write to the program's path goes to a hidden temporary file in the same folder, `.<name>.<random>.partial`. The file is synced, then renamed over the path once every byte is written.
  - The path therefore holds either what it held before or the whole file.
  - A failure, a deadline (ADR 0080), a cancellation or a crash never leaves a partial file there.
  - On any failure, deadline or cancellation, the temporary file is removed.
  - Only a process killed mid-save leaves its temporary file. The file is hidden and never at the path.
- **`saveAs` keeps Playwright's meaning where it matches (S3):**
  - it creates missing parent folders, as Playwright, Crawlee and Scrapy do;
  - it replaces an existing file, as Playwright's `copyFile` and `createWriteStream` do;
  - it resolves a relative path against the program's working directory.
- **Either process may write the program's path.** The SDK runtime may write it, or a local engine may copy the file there directly, as Playwright's server does. Both follow this rule and raise these codes, and a full disk there is `file_write_failed` with side `program`, since `side` names the storage that failed, not the process.
- **On Windows,** where a leading dot does not hide a file, the temporary file also gets the hidden attribute.
  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-23, [#790](https://github.com/goodfetch/goodfetch/issues/790): the engine's writer of a program's path (`run_files`, used by `download.save_as` on an Attached engine) creates the temporary file hidden on Windows, syncs it, clears the attribute, then renames it over the path; `path_not_writable`'s details are `{reason, os_code, path, fix}`, `fix` naming the path argument. A folder at the path is refused before any byte is written (`is_a_directory`, `EISDIR`), and a file where a parent folder should be is `not_a_directory` (`ENOTDIR`). While the Run folder's filesystem is at `critical`, a file a Run keeps for the program (a Download's now, an Export's later) fails at once with `file_write_failed`, side `engine`, reason `storage_full`, writing nothing (ADR 0122).
- **On the engine's side.** `run.export`, `run.checkpoint` and `run.trace` (when it writes its copy) return their value only once its file is complete, with its size and digest (an Export's SHA-256, ADR 0103; a Checkpoint's digest, ADR 0102), and the engine removes an unfinished file. So an `Export`, a `Checkpoint` or a `RunTrace` a program holds is always whole.
  > Amended by [ADR 0122](0122-a-run-stops-resumably-before-its-disk-fills-and-a-daemons-logs-rotate-at-a-bounded-size.md): while the filesystem where the engine would write it is at the `critical` storage level, `run.export`, `run.checkpoint` and `run.trace`'s copy fail at once with `file_write_failed`, side `engine`, reason `storage_full`, before writing, so the reserve stays for the Runs' stops.
  > Note (agent decision, as built), 2026-10-26, [#980](https://github.com/goodfetch/goodfetch/issues/980): the engine writes an Export through the same hidden temporary file, synced and renamed, as a kept Download. A write the storage refuses (`storage_full` with `ENOSPC`: the bytes go through the raw file, so the OS code is kept) or a reader of the Records that stops first removes the temporary file and answers `file_write_failed`, side `engine`, its `path` the Run's folder. The `critical` check reads the Run folder's filesystem at the call, as a Checkpoint's does.

### Golden scenario 05

Failure point 7 becomes **typed** in all three languages, with no change to the programs:
- a path the program cannot write raises `path_not_writable`, naming the path;
- a full disk raises `file_write_failed` with its side, naming what to free;
- `books.json` is never left half-written.

## Considered Options

- **Only the Export:** rejected. `saveAs` has one meaning on four values (S2, S3), and a Download or a Checkpoint saved onto a full disk would stay generic.
- **One `export_write_failed`, with the side and the OS error in its details:** rejected. One group cannot carry both remedies. A program that catches `ProgramError` to find its own bugs would also catch full disks, or the other way round, and ADR 0079 rejects groups that do not tell the caller what to do.
- **Two codes, split by side:** rejected for the same reason. A full disk on the program's machine is not a program bug.
- **The language's own file error passed through** (Playwright, Crawlee): rejected by G6 and pain 10. It carries no `code`, `group`, `fix` or `operation`, and in Rust every call returns `goodfetch::Error` anyway. It stays reachable as the chained cause.
- **Logging a failed store and finishing** (Scrapy): rejected by G1. It is scenario 05's silent score.
- **`file_write_failed` retryable:** rejected. Nothing changes until a person acts, so a crawl handler would spend its retries on a full disk.
- **Writing in place and leaving the file** (Playwright's remote path, Crawlee): rejected by G1. A truncated file looks like an Export.
- **Writing in place and deleting on failure:** rejected. An earlier file at the path is lost either way, and a killed process still leaves a truncated file.
- **Appending by default** (Scrapy): rejected. A second run would mix two Exports in one file.
- **Refusing a missing parent folder or an existing file:** rejected by S3. Playwright's `saveAs` creates folders and overwrites.

## Consequences

- **Amends ADR 0103.** `run.export` and `Export.saveAs` raise `path_not_writable` or `file_write_failed`, and a saved Export is written whole or not at all. Notes sit under its Exports section and its Error codes.
- **Amends ADR 0085.** `Download.saveAs` and the `path` option of `screenshot()` and `pdf()` follow the same rule and codes, beside `download_failed`. Notes sit under its Files section and its Error codes table.
- **Amends ADR 0102.** `run.checkpoint()`, `run.trace()`'s copy, and their `saveAs` follow the same rule and codes. Notes sit under its Checkpoints and The Run trace sections.
- **Amends ADRs 0091 and 0105.** A failed write of the Plan file or the Response recording raises `path_not_writable` or `file_write_failed` from the call during which it happens (a Run's start for an accepted Plan version, its end otherwise), leaving the file as it was. Notes sit under ADR 0091's promotion bullet and ADR 0105's `mode: "record"` bullet.
- **Amends ADR 0102 again.** The call that ends a Run raises a failed write of its Plan file or Response recording once the Run has ended, unless the Run itself failed; the report keeps a `RunWarning`, and Plan versions not written stay pending. A note sits under its Ending a Run's promotion bullet.
- **Adds to ADR 0079.** Reasons of a failed file write that an SDK does not know are kept as strings, as its other open values are. A note sits under its newer engine's values.
- **Error codes.**
  - `path_not_writable` joins `goodfetch-protocol` in the `program` group, and `file_write_failed` in the `unavailable` group. Neither is retryable.
  - Both join through `xtask codegen`'s naming checks (ADR 0079), with an exit code each in ADR 0069's table.
  - They are Collie's codes, so Rust has `ErrorCode::PATH_NOT_WRITABLE` and `ErrorCode::FILE_WRITE_FAILED` (ADR 0099).
- **`TESTING.md`** gains the file-saving conformance cases.
- **Who builds what.**
  - **Area 6 (Collection)** builds the engine's side for Exports.
  - **Area 3 (Engine runtime)** builds it for Checkpoints and the Run trace's copy.
  - **Area 11 (Client surfaces)** builds the program's side in each SDK: the temporary file and rename, the reasons from each platform's errors, and the two classes with their details.
  - Each lists this ADR.
- **ADR 0037's matrix:**
  - CO-22 (file download) and CO-27 (Export) cite this ADR;
  - pain 23 (`rel-incorrect-incomplete`) cites it for files that are never left partial.
- **Other tickets of the map.** [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485) receives P48's and P53's filled rows and scenario 05's failure point 7, now typed.
- **The `prototype/sdk-api` sketch** holds the two classes in its `results` files for each language, with `saveAs`'s notes in its `output`, `page` and `run` files.
