# One binary carries every capability, under a size budget CI enforces

The `goodfetch` binary keeps every capability built in, V8 and DuckDB included, and is held to a **Size budget**: a ceiling per released file and an increase check on every pull request, both enforced by CI. V8 and DuckDB are about 80% of the binary (+42.8 MB and +33.0 MB of a 94.9 MB probe), and the projection for V1 is about 95–120 MB raw and 35–45 MB compressed per platform before the engine's own code. That is the size of Deno (96 MB, 42 MB compressed) and Bun (89 MB, 42 MB), and it fits every channel. Until now no ADR set a number, so a new dependency had no cost anyone could see, and a channel limit would only show up when a publish failed. Decided in [Binary-size budget per distribution channel](https://github.com/Jbmanllr/automan/issues/197), from the measurements of [Size limits per distribution channel and the size cost of each binary component](https://github.com/Jbmanllr/automan/issues/196) ([findings](https://github.com/Jbmanllr/automan/blob/research/binary-size-budget/rnd/research/2026-10-12-binary-size-limits-and-costs.md)).

**Which channel limits bind.**
- **PyPI:** 100 MB per file and 10 GB per project, both raised on request; yanking frees nothing ([storage limits](https://docs.pypi.org/project-management/storage-limits/)).
- **crates.io:** 10 MiB per `.crate` upload, with no documented way to raise it ([publish_limits.rs](https://github.com/rust-lang/crates.io/blob/main/src/config/publish_limits.rs)). The binary never ships in a crate (ADRs 0024 and 0051), but the `goodfetch` crate carries what the engine embeds, because ADR 0047 forbids downloads at build time.
- **npm:** no documented limit. Users report a 117.9 MB tarball accepted and 198.7 MB and more refused with `E413` ([npm/cli#4794](https://github.com/npm/cli/issues/4794)); unverified by npm.
- **Everything else** is far above the binary: GitHub Releases under 2 GiB per asset, GHCR 10 GB per layer, Lambda container images 10 GB, Cloud Run none.

**Competitors.** Deno, Bun and Node publish no size budget, and their users complain about it ("Why is the Deno 2 binary 136 MB?", [deno#26516](https://github.com/denoland/deno/issues/26516); a Bun issue asking for a smaller build drew 40 reactions, [bun#2541](https://github.com/oven-sh/bun/issues/2541)). Stagehand bundled with Playwright went past Convex's 42.9 MiB function limit ([stagehand#1610](https://github.com/browserbase/stagehand/issues/1610)). Front-end projects fail CI on size with `size-limit` or `bundlewatch`. What every browser-automation user downloads is the browser: Chrome headless shell is 124 MB compressed and Chrome 198 MB, and that, not the driver, is what breaks Lambda's zip limit.

## Considered Options

- **V8, DuckDB or both downloaded on first use**: rejected. Splitting both saves about 30 MB compressed per platform, about 15% of an install once the browser is counted. It costs a wait before the first script or the first search, an `artifact_unavailable_offline` error under `offline` (ADR 0043) for features that need no network today, two more signed and pinned executables (ADR 0039), and the download failures behind proxies and mirrors that Playwright needed five settings to handle. It would also undo ADR 0051's worker that runs from the same binary.
- **Dropping ICU from V8** (`v8_enable_i18n_support = false`, −10.9 MB): rejected. Scripts lose `Intl`, which ADR 0051 promises and extraction scripts use to read dates and prices in a site's locale, without saying so. LLRT drops it and its users asked for it back.
- **ICU stored compressed and decompressed at worker start** (−6 MB on disk): rejected for performance. Every worker would hold its own 10.9 MB copy instead of sharing the binary's pages, about 175 MB at 16 workers. Decompressing once into the per-user cache and handing each worker the file would share it again, but workers have no file access (ADR 0051), so it adds a mechanism for what the binary already gives.
- **`opt-level = "s"` or `"z"`** (−18% or −25% on the probe): rejected for performance. The measured DuckDB query ran about 18% or 90% slower, and AWS's LLRT measured 7% slower on average for `"s"` ([llrt#1811](https://github.com/awslabs/llrt/issues/1811)); ADR 0025's benchmark measures exactly that speed.
- **`panic = "abort"`** (a further −1.5%): rejected. A panicking task would kill the engine and every run in it, where today it fails only that task.
- **UPX compression**: rejected. It does not support macOS, breaks Platform signatures (ADR 0039), keeps the whole image resident and draws antivirus false positives on Windows.
- **Ceilings alone**: rejected. They catch the break, not the drift before it.
- **An increase check alone**: rejected. Small accepted increases add up with no hard stop.
- **Release-profile builds in every CI tier**: rejected. Fat LTO takes about 20 minutes per target; a thin-LTO build compared with `main`'s thin-LTO build measures growth accurately, since the two profiles differ by a stable 4%.
- **The owner approving every increase**: rejected. It stops autonomous delivery for routine dependency updates; the ceilings, which only the owner moves, remain the hard stop.
- **Nightly wheels on PyPI, pruned by a job**, as DuckDB does (79 `.devN` versions live, 52 GB, [cleanup_pypi.yml](https://github.com/duckdb/duckdb-python/blob/main/.github/workflows/cleanup_pypi.yml)): rejected. PyPI has no API to delete a release ([warehouse#12934](https://github.com/pypi/warehouse/issues/12934) is open), so the job scripts the web forms with the account's password and two-factor secret stored in CI. That breaks ADRs 0016 and 0032's token-free trusted publishing, and depends on forms PyPI may change.
- **A separate `goodfetch-nightly` PyPI project**, as TensorFlow's `tf-nightly`: rejected. It has the same quota and the same pruning problem, and is one more name to guard.
- **DuckDB's SQLite reader linked statically**: rejected for distribution. The `duckdb` crate offers no option for it and refuses extension configs, so it would need a patched build or a self-built libduckdb; DuckDB's own build notes say static linking "does not properly work" on Windows because of symbol collisions ([sqlite_scanner.cmake](https://github.com/duckdb/duckdb/blob/main/.github/config/extensions/sqlite_scanner.cmake)). A second SQLite beside `rusqlite`'s is a likely further collision, not verified.
- **Reading run files through the engine's own DuckDB table functions** (the `duckdb` crate's `vtab` feature over `rusqlite`): rejected. It reimplements type mapping and parallel scans that DuckDB's maintained extension already provides.

## Consequences

### What stays in the binary

- Every capability stays in the one `goodfetch` binary of ADR 0024: V8 for the Script sandbox (ADR 0051), DuckDB for search and the SQL tab (ADR 0031) and the DuckDB Sink, and the in-binary Sinks of ADR 0026. No capability leaves the binary to meet the Size budget. Browsers, sink processes and Plugins stay downloads, as before, and DuckDB's SQLite reader becomes one (below), because DuckDB ships it only as an extension.
- **Embedded data that is decompressed once, or served as it is, is stored compressed.** Every channel already compresses its files, so this saves disk space after install (about 6 MB), not download size; the compressed ceiling below is unaffected.
  - The serialized Clean-up rules with zstd (9.06 MB to 3.67 MB), decompressed when they are loaded, in about 6 ms, then dropped once deserialized. **Changes ADR 0047.**
  - The Console's assets precompressed with brotli (0.99 MB to 0.40 MB for the prototype) and served as they are with `Content-Encoding: br`, so no request compresses anything. It needs a small handler, because `tower-http`'s `precompressed_br` expects the uncompressed file beside the `.br` one. **Changes ADR 0031.**
  - The offline docs (ADR 0030) and the `goodfetch init` templates (ADR 0049), compressed the same way.
- **V8 keeps full ICU, uncompressed in the binary.** ICU is the 10.9 MB of locale data behind `Intl`. Each Script sandbox worker is the `goodfetch` binary itself, so data stored in the binary is shared through the page cache by every worker. ICU stored compressed would need a private decompressed copy in each worker for its whole life, because V8 reads ICU data in place: about 175 MB at 16 workers, to save 6 MB of disk and almost nothing of download. The same rule applies to any data a worker reads for its whole life. **Confirms ADR 0051.**

### Release builds

- The release profile uses fat LTO with one codegen unit, `opt-level = 3`, unwinding on panic, and `strip`. Every other profile, including pull-request and merge-queue builds, keeps thin LTO.
- Each release publishes the debug symbols of every binary beside it (split DWARF or `.dSYM` on Unix, `.pdb` on Windows), signed and attested like the binaries, so a crash backtrace can be read. **Changes ADR 0030.**

### The Size budget

| What is measured | Ceiling | Increase threshold per pull request |
|---|---|---|
| Compressed release file per platform: the npm platform package, the wheel and the GitHub Release archive | 60 MB | +1 MB |
| Raw `goodfetch` binary per platform | 160 MB | — |
| `goodfetch` crate tarball (`.crate`) | 8 MiB | — |
| Each OCI image tag (ADR 0049) | none | +3 MB compressed |

- Every V1 target of ADR 0024 is measured, and the largest must fit. Static musl Linux is the largest so far for V8 (8–12% above macOS), and DuckDB's own musl build is its largest.
- The 60 MB ceiling leaves 15–25 MB over the projection, 40 MB under PyPI's per-file limit and about half of what npm is known to accept. The 8 MiB ceiling leaves 2 MiB under crates.io's limit, which has no documented raise. Images get no ceiling because browsers dominate them and their registries allow 10 GB, but an accidental layer still trips the increase check.
- Each release's notes list every measured size, and the sizes are kept per version so growth can be charted.

### Where CI enforces it (changes ADR 0032)

- **Every pull request:** a Linux x64 size estimate posted as a comment, which never blocks.
- **Merge queue:** the increase check, per target of the full matrix, comparing each build with `main`'s build of the same profile. It blocks.
- **Nightly and release candidate:** the ceilings, measured on the release-profile files themselves. They block: a nightly over a ceiling is not published, and neither is a release candidate or a release (**changes ADR 0030**).

### Who moves it

- A pull request that trips the increase check passes when its body has a "Size increase" section with the measured delta per target and its reason, and the fresh review accepts that section explicitly on the pull request's exact head.
- A ceiling moves only by changing this ADR, with the owner's approval.

### PyPI (changes ADR 0030)

- **Nightly wheels leave PyPI.** They are attached to the nightly GitHub pre-release, which has no total size limit, and listed by a simple repository index (PEP 503, with PEP 691's JSON form) that the nightly job publishes on `goodfetch.dev`. Each file is listed with its SHA-256 and keeps its attestation. The index and the pre-releases keep the last 30 nightlies. Users install with `pip install --pre goodfetch --index-url https://goodfetch.dev/nightly/simple --extra-index-url https://pypi.org/simple`, or a uv index pinned to the `goodfetch` package. npm keeps its `nightly` tag, and the OCI image its `nightly` tag: neither has a quota.
- PyTorch, JAX, NumPy and SciPy publish nightlies to their own index the same way. uv, ruff, Deno, polars and Playwright publish none to PyPI.
- **The project quota.** About 50 releases a year (a release candidate and a minor every 4 weeks, a patch per Chrome major), six wheels each at about 40 MB, use about 12 GB a year, more than PyPI's 10 GB default. Before the first public release Goodfetch asks for 100 GB through [pypi/support](https://github.com/pypi/support), as DuckDB was granted ([support#6896](https://github.com/pypi/support/issues/6896)). The release job reads the project's usage and warns when it passes 70% of the quota, so the next increase is asked for before a publish can fail.

### DuckDB's SQLite reader (changes ADRs 0031 and 0043)

- Search across runs and the SQL tab read run files through DuckDB's official SQLite reader (`sqlite_scanner`), an out-of-tree extension that DuckDB would otherwise fetch from its repository on first use.
- It becomes a pinned download: the exact build for the binary's DuckDB version and platform, checked by digest, fetched on the first search or SQL query into the shared cache, then loaded from that file with DuckDB's own signature check on. Autoloading and installing extensions from the network stay off.
- `goodfetch browsers install` fetches it with the other pinned artifacts, and under `offline` a missing copy fails that query with `artifact_unavailable_offline`; the rest of the Console works.
- It is about 8.5–12 MB compressed per platform and lies outside the Size budget, like browsers and sink processes.
