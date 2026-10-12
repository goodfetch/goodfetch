# Size limits per distribution channel and the size cost of each binary part

Research of **October 12, 2026** for ticket [#196](https://github.com/Jbmanllr/automan/issues/196) (map [#61](https://github.com/Jbmanllr/automan/issues/61)), feeding the decision in [#197](https://github.com/Jbmanllr/automan/issues/197). Question: which hard limits and practical thresholds bound what Goodfetch ships, and what does each part add to the `goodfetch` binary and its packages?

**Sources.** Official docs of each channel and registry source code, read on 2026-10-12. Registry metadata APIs (npm, PyPI, GitHub Releases, Docker Hub, MCR) for competitor sizes. Maintainers' issues for user pains.

**Measured versus cited.** Facts marked **M** were measured for this note (method in §6); everything else is cited (**C**) with its link. Claims found only in secondary sources are tagged *unverified*. MB means 10^6 bytes. The host was heavily loaded by unrelated work (load average 60–990), so build times are indicative only; sizes are not affected. Like everything in `rnd/`, this note informs a decision and makes none: #197 decides the budget.

---

## 1. Summary

- **Only three channels have a limit that a 50–150 MB binary can hit.**
  - **crates.io:** 10 MiB per `.crate` upload (compressed) and 512 MiB unpacked, overridable per crate **C**. The binary cannot ship inside a crate; this already matches ADRs 0024 and 0051 (the crate builds the engine from source and downloads the `goodfetch` binary on first use). But the `goodfetch` crate's own tarball must hold whatever the engine embeds at build time (§2.2).
  - **PyPI:** 100 MB per file and 10 GB per project by default **C**. One wheel fits. The project quota does not survive ADR 0030's nightly `.devN` wheels: at about 40 MB × 6 wheels a day, 10 GB is gone in about six weeks (§2.3).
  - **npm:** no documented limit. Users report a 117.9 MB tarball accepted and tarballs of 198.7 MB and more refused with `E413` *(unverified: user reports only, no npm statement)* **C**.
- **Everything else is far above the binary.** GitHub Releases: under 2 GiB per asset, 1,000 assets per release. GHCR: 10 GB per layer. Lambda container images: 10 GB. Cloud Run: no image limit. Homebrew: none documented **C**.
- **What dominates the binary is V8 and DuckDB.** On macOS arm64, stripped, thin LTO **M**:

  | Build | Size |
  |---|---|
  | Tokio + serde_json baseline | 0.62 MB |
  | Every measured part except V8 and DuckDB (§3.1) | 19.11 MB |
  | + DuckDB (bundled, core only) | 52.08 MB |
  | + V8 (`v8` 152.2.0) | **94.85 MB** |
  | Same, gzip -9 / zstd -19 / xz -9 | 34.5 / 24.8 / 21.2 MB |

  V8 is 45% of that binary and DuckDB 35%. Every other measured part together is 20%. The Sinks of ADR 0026 add up to 21 MB more (§3.1), and embedded data 5–15 MB (§3.2): about **105–130 MB, 40–50 MB gzip**, before the engine's own code.
- **Two size facts the ADRs do not yet account for.**
  - A quarter of ADR 0051's +42.8 MB for V8 is ICU data for `Intl`. The prebuilt `librusty_v8.a` (v152.2.0) carries it as `icudtl_dat.o`, a 10.88 MB constant (`_icudt78_dat`), and it is linked into the probe binary **M**. Removing it needs a from-source V8 build without i18n support, which ADR 0051 already plans to build from source.
  - DuckDB's SQLite reader, which ADR 0031 uses for search across runs and the SQL tab, is not built into DuckDB. It is an extension "autoloaded on first use from the official extension repository" **C**. Under ADR 0043 that is an unconfigured connection unless it is linked statically (larger build) or becomes a pinned download.
- **Comparable single binaries are 90–130 MB unpacked and 30–45 MB compressed**: Bun 89 MB, Deno 96 MB, Node 127 MB; Claude Code's native binary is an outlier at 257 MB **M** (registry metadata, §4). Playwright's PyPI wheel is 48 MB because it bundles Node. Chromium alone (205–298 MB unpacked) is what breaks Lambda's zip limit, not the driver.
- **Size levers.**
  - On the 94.85 MB probe **M**: fat LTO with one codegen unit −4.4%; adding `panic = "abort"` −5.9%; adding `opt-level = "s"` −18% (77.8 MB); `"z"` −25% (71.5 MB). The prebuilt V8 does not shrink, so on the rest the saving is −33% (`"s"`) and −45% (`"z"`). A DuckDB scan ran about 18% slower at `"s"` and 90% slower at `"z"` on a loaded host.
  - LLRT measured −29% at `"s"` for 7% average CPU cost and no startup change **C**.
  - `panic = "abort"` turns a panicking Tokio task into a dead engine **C**. UPX does not support macOS and keeps the whole image resident **C**.
  - Splitting V8 or DuckDB into a first-use download saves about 43 or 33 MB per platform, at the cost of an offline download, a second signed artefact and a first-use delay (§5).

## 2. Channel limits

### 2.1 Table

| Channel | Hard limit | Practical threshold or behaviour | Source |
|---|---|---|---|
| npm (registry.npmjs.org) | Not documented. The Open Source Terms set no size cap. | 117.9 MB tarball accepted (Oct 2026); 198.7 MB, 229.9 MB and 290.5 MB refused with `E413` *(unverified: user reports)*. `libnpmpublish` sends the tarball base64-encoded in a JSON body, which inflates it by 4/3; a body cap near 256 MB would fit these reports *(unverified inference)*. No bandwidth cap; the terms forbid using the registry "as a general purpose database". | [npm/cli#4794](https://github.com/npm/cli/issues/4794), [mjolnir#1269](https://github.com/BrokkAi/mjolnir/issues/1269), [dirigible#7771](https://github.com/eclipse-dirigible/dirigible/issues/7771), [terms](https://docs.npmjs.com/policies/open-source-terms), [publish.js](https://github.com/npm/cli/blob/latest/workspaces/libnpmpublish/lib/publish.js) |
| PyPI | **100.0 MB per file, 10.0 GB per project.** Both can be raised on request. | File increase: needs one release under the limit first, then a [pypi/support](https://github.com/pypi/support) issue. Project increase: remove unneeded files first; yanking frees nothing. 51 limit requests were open on 2026-10-12; recently closed ones took 0–12 days. | [storage limits](https://docs.pypi.org/project-management/storage-limits/) |
| crates.io | **10 MiB per `.crate`** (`10 * 1024 * 1024`), 512 MiB unpacked, 30,000 tarball entries; overridable per crate in the database. No documented process for a size increase. | `v8` 152.2.0's `.crate` is 36.6 MB **M**, so overrides are granted. docs.rs has no network, so a `build.rs` that downloads must skip it under `DOCS_RS`. | [publish_limits.rs](https://github.com/rust-lang/crates.io/blob/main/src/config/publish_limits.rs), [docs.rs builds](https://docs.rs/about/builds) |
| Homebrew | Not documented. | homebrew-core builds from source; binary-only software "belongs in a cask". A tap's bottles can live on GitHub Packages or any `root_url`. | [Acceptable Formulae](https://docs.brew.sh/Acceptable-Formulae), [Bottles](https://docs.brew.sh/Bottles), [taps](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap) |
| GHCR (OCI) | **10 GB per layer**, 10-minute upload timeout. | Storage and bandwidth free for public packages. | [Container registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry), [billing](https://docs.github.com/en/billing/concepts/product-billing/github-packages) |
| Docker Hub | No size limit documented. | Pull limit per 6 h: 100 unauthenticated, 200 Personal. | [usage](https://docs.docker.com/docker-hub/usage/) |
| AWS Lambda, zip | **50 MB zipped** direct upload, **250 MB unzipped** with layers (cannot be raised), 5 layers. | ADR 0049 already rejected it: browsers do not fit. | [quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html) |
| AWS Lambda, image | **10 GB uncompressed.** `/tmp` 512 MB to 10,240 MB; memory up to 10,240 MB. | Images load lazily in 512 KiB chunks; one study found only 6.4% of image data is read at startup. Chunks came 67% from the worker cache, 32% from the AZ cache (median 550 µs), 0.06% from S3 (36 ms). A function idle for weeks goes `Inactive` and its next call fails while the image is re-optimized. | [images](https://docs.aws.amazon.com/lambda/latest/dg/images-create.html), [Brooker et al., ATC '23](https://arxiv.org/pdf/2305.13162) |
| Cloud Run | Image: "no direct limit". Memory up to 32 GiB. | The writable filesystem is in memory and counts against the instance's memory. Image size "does not affect container startup times" (image streaming). Startup CPU boost lasts startup + 10 s. | [quotas](https://docs.cloud.google.com/run/quotas), [tips](https://docs.cloud.google.com/run/docs/tips/general), [CPU](https://docs.cloud.google.com/run/docs/configuring/services/cpu) |
| GitHub Releases | **Under 2 GiB per asset, 1,000 assets per release.** | "no limit on the total size of a release, nor bandwidth usage". | [About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases) |
| `curl \| sh`, PowerShell | None of their own: they fetch from GitHub Releases. dist's installers fetch the archive at install time; only its MSI bundles the binary. | — | [dist installers](https://axodotdev.github.io/cargo-dist/book/installers/index.html) |
| winget, Scoop | Not found (winget) / not checked (Scoop). | — | — |
| Reference: Vercel Functions | 250 MB uncompressed (500 MB Python; 5 GB beta). | — | [limits](https://vercel.com/docs/functions/limitations) |
| Reference: Cloudflare Workers | 64 MiB uncompressed on Free and Paid; no compressed limit. | — | [limits](https://developers.cloudflare.com/workers/platform/limits/) |

### 2.2 crates.io: what the `goodfetch` crate must carry

ADR 0024 links the engine into the Rust crate, built from source. Big C/C++ dependencies arrive as their own `-sys` crates, each under its own limit, so they do not count against `goodfetch`'s 10 MiB **M** (`.crate` sizes in the local Cargo cache):

| Crate | `.crate` size |
|---|---|
| `v8` 152.2.0 (bindings; downloads a 40 MB prebuilt at build time) | 36.6 MB (above 10 MiB: an override) |
| `aws-lc-sys` 0.46.0 (rustls' default crypto) | 9.66 MB |
| `libduckdb-sys` 1.10506.0 (amalgamation) | 6.21 MB |
| `libsqlite3-sys` 0.38.2 | 5.36 MB |
| `btls-sys` 0.5.6 (BoringSSL, for `wreq`) | 5.16 MB |

What does count: anything the engine embeds and builds from source. ADR 0047 forbids build-time downloads, so the crate must carry its inputs:
- the Console's built assets: **0.40 MB** brotli for the current prototype (§3.2);
- the embedded Clean-up rules as rule text: EasyList, EasyPrivacy, Fanboy Cookie Monster and Fanboy Annoyance are **6.72 MB raw, 2.18 MB gzip -9** (sum of the four files as served on 2026-10-12, overlap not removed) **M**, plus Brave's unbreak list, the surrogates and autoconsent (not measured);
- the embedded docs (ADR 0030) and templates (ADR 0049), not measured.

That is about 3 MB of the 10 MiB before any source code. `aws-lc-sys` at 9.66 MB shows how close a crate can run to the limit.

### 2.3 PyPI: the project quota against ADR 0030's cadence

ADR 0024 ships `py3-none` platform wheels for six targets (one musl wheel per Linux architecture, dual-tagged). ADR 0030 publishes stable releases, an `rc` each minor, patches when ready, and **nightly `.devN` wheels to PyPI**.

| Wheel size (compressed) | Per release (6 wheels) | Releases until 10 GB | Nightlies alone |
|---|---|---|---|
| 10 MB (no V8, no DuckDB, with Sinks: ~40 MB raw, projected) | 60 MB | ~166 | ~24 weeks |
| 20 MB (no V8: the 52 MB probe is 19.2 MB gzip -9) | 120 MB | ~83 | ~12 weeks |
| 40 MB (everything, §3.3 projection) | 240 MB | ~41 | ~6 weeks |

Arithmetic from the measured compressed sizes in §1. Wheels are zip (deflate), so their size is close to gzip -9. The quota is cumulative: only deleting files frees it, and the docs say yanking does not. So a nightly on PyPI needs either a project increase or a job that deletes old dev releases. Whether PyPI offers deletion through an API (rather than the web UI) was not checked.

### 2.4 Platform packages on npm, pnpm and Yarn

- `os` matches `process.platform`, `cpu` matches `process.arch`, and `libc` applies only when `os` is `linux` ([package.json docs](https://docs.npmjs.com/cli/v11/configuring-npm/package-json)). npm checks `libc` since 9.6.3 (2023-03-30) and has `--libc` since 10.3.0 (npm/cli changelog) **C**.
- pnpm filters with `supportedArchitectures` (`os`/`cpu`/`libc`) ([pnpm](https://pnpm.io/settings/dependency-resolution)); Yarn has the same setting ([yarnrc](https://yarnpkg.com/configuration/yarnrc)) **C**.
- [npm/cli#4828](https://github.com/npm/cli/issues/4828) (a lockfile made on one platform keeps only that platform's optional packages) is closed via PR #8184, but users still report it on npm 11.16–11.17 and no fixed version is named **C**. ADR 0024 already handles this with its first-use error.

## 3. Component costs

### 3.1 Code, measured

macOS arm64 (`aarch64-apple-darwin`), Rust 1.93.0, release profile with `strip = true` and `lto = "thin"`. Each row adds one part to the same Tokio + serde_json baseline and calls its main entry point on input from `argv`, so the linker keeps its real code paths **M**.

| Part | Crate (version, features) | Stripped binary | Added |
|---|---|---|---|
| Baseline | `tokio` 1 (rt-multi-thread, net, fs, process, …), `serde_json` | 0.62 MB | — |
| Allocator (ADR 0024) | `mimalloc` 0.1.52 | 0.73 MB | +0.10 MB |
| Run state | `rusqlite` 0.40.2, `bundled` | 2.39 MB | +1.76 MB |
| CDP transport (stand-in for the in-house CDP client) | `tokio-tungstenite` 0.30 | 1.00 MB | +0.37 MB |
| Console server | `axum` 0.8.9 + `tower-http` 0.7.1 (`fs`, brotli and gzip compression) | 2.74 MB | +2.11 MB |
| Default extractor (ADR 0007) | `dom_smoothie` 0.18.2 | 2.06 MB | +1.43 MB |
| Filter list engine (code only) | `adblock` 0.13.3, default features | 3.53 MB | +2.91 MB |
| HTTP with rustls | `reqwest` 0.13.5, `rustls` (aws-lc-rs), http2, gzip/br/deflate/zstd | 5.77 MB | +5.15 MB |
| HTTP impersonation | `wreq` 6.0.0-rc.31 (BoringSSL fork `btls`) + `wreq-util` 3.0.0-rc.14, every Emulation profile | 6.12 MB | +5.49 MB |
| DuckDB, core | `duckdb` 1.10506.0 (DuckDB 1.5.6), `bundled` | 33.65 MB | **+33.02 MB** |
| DuckDB + Parquet + JSON | same, `parquet`, `json` | 36.45 MB | +35.82 MB |
| V8 (Script sandbox) | `v8` 152.2.0, prebuilt static library, `--jitless` | 43.43 MB | **+42.81 MB** |
| All of the above but V8 and DuckDB | — | 19.11 MB | +18.49 MB |
| + DuckDB core | — | 52.08 MB | +32.97 MB over the row above |
| + V8 | — | 94.85 MB | +42.78 MB over the row above |

The V8 figure matches ADR 0051's +42.8 MB. Parts overlap (shared `hyper`, `rustls`, compression crates), so individual additions sum to 0.8 MB more than the combined build.

**Sinks built into the binary (ADR 0026)**, measured the same way in a separate package over the same baseline **M**:

| Sink library | Crate (version, features) | Added |
|---|---|---|
| Object stores | `object_store` 0.14.2 (`aws`, `gcp`, `azure`, `http`) | +7.54 MB |
| MongoDB | `mongodb` 3.9.1, default features | +6.48 MB |
| MySQL / MariaDB | `mysql_async` 0.37.1 (`minimal-rust`, `rustls-tls`, `ring`) | +4.20 MB |
| Parquet export | `parquet` 60.0.0 (`arrow`, `zstd`, `snap`) + `arrow-array` | +3.87 MB |
| SQL Server | `tiberius` 0.13.0 (`rustls`, `tds73`) | +3.65 MB |
| SMTP | `lettre` 0.11.23 (`tokio1-rustls`, `ring`) | +1.82 MB |
| PostgreSQL | `tokio-postgres` 0.7.18, no TLS | +0.72 MB |
| All seven together | — | **+21.46 MB** |

They share `rustls`, `hyper` and compression code with the HTTP stacks above, so in the engine they would add less than 21 MB; how much less was not measured. `sqlx` could not be used: its `libsqlite3-sys` conflicts with `rusqlite`'s in one dependency graph (Cargo's `links` rule) **M**.

**Not measured** (no code exists yet or no representative crate): `goodfetch-protocol` and the rest of the engine's own code, the MCP server, model-gateway clients (ADR 0012), OpenTelemetry, the SFTP/FTP and Notification Sinks, PostgreSQL over TLS, and Windows builds (§3.4).

### 3.2 Embedded data

| Data | Raw | Compressed | Source |
|---|---|---|---|
| Embedded Clean-up rules, serialized `adblock` engine | 9.06 MB | 3.67 MB | ADR 0047 (measured there) |
| autoconsent rules and library | — | ~0.1 MB | ADR 0047 |
| Console, current prototype build (`prototype/console-screens`, `vite build`, 35 TSX files, Inter fonts) | 0.99 MB | 0.44 MB gzip -9, 0.40 MB brotli -q 11, 0.41 MB zstd -19 | **M** |
| V8 ICU data, already inside the +42.8 MB of V8 | 10.88 MB | 4.6 MB (Deno's `deno_core_icudata` crate, same kind of data) | `icudtl_dat.o` in `librusty_v8.a` **M**; [deno_core_icudata](https://github.com/denoland/deno_core_icudata) **C** |
| Embedded docs (ADR 0030), templates (ADR 0049) | not measured | — | — |

The Console prototype is a lower bound: the shipped Console adds the SQL tab, the trace viewer and video. The JS bundle is one 625 KB chunk (195 KB gzip).

### 3.3 Projection for one `goodfetch` binary

Arithmetic on the measured rows, macOS arm64, before the unmeasured engine code: about 95 MB of measured code, plus up to 21 MB of Sinks, plus 5–15 MB of embedded data depending on whether it is stored compressed. So **roughly 105–130 MB stripped, about 40–50 MB gzip** per platform (gzip ratio of the measured binary, 36%), with V8 and DuckDB as 76 MB of it. The engine's own code comes on top. These bounds are an estimate, not a measurement.

### 3.4 Other platforms

A static musl build of the same probe was attempted in a `rust:1.93-alpine` container and abandoned: installing the C toolchain (`apk add`) had not finished after 100 minutes on the day's network. So this note has **no measured musl or Windows numbers** for DuckDB, the HTTP stacks or the full set.

From the JavaScript sandbox research of the same day (branch `research/js-sandbox-engines`, §3.6–3.7) **M**: `v8` on static musl gives 46.98 MB (aarch64) and 48.52 MB (x86_64, `static-pie`) for the whole bench binary, against 43.25 MB on macOS arm64. On aarch64 musl it needs `-C link-arg=-lgcc` to link.

DuckDB's own release (v1.5.6) gives a feel for platform spread **C**: `libduckdb` zips are 42.7 MB (linux-amd64-musl), 37.8 MB (linux-arm64), 36.2 MB (osx-universal) and 13.4 MB (windows-amd64); the Linux x64 `.so` unpacks to 70.6 MB. Windows was not measured here.

## 4. Competitor reference points

Measured on 2026-10-12 from registry metadata (npm `dist.unpackedSize` and tarball `Content-Range`; PyPI JSON `urls[].size`; GitHub release asset `size`; zip central directories read with HTTP Range requests for unpacked sizes; Docker Hub and MCR manifests for compressed image sizes) **M**.

| Tool | Artifact | Version | Compressed | Unpacked |
|---|---|---|---|---|
| Playwright | `playwright-core` npm | 1.64.0 | 3.17 MB | 13.66 MB |
| | PyPI wheel (bundles a Node driver), linux x64 | 1.63.0 | 48.22 MB | — |
| | Docker `mcr.microsoft.com/playwright:v1.64.0-noble` | 1.64.0 | 985.65 MB | — |
| Chrome for Testing | `chrome-headless-shell-linux64.zip` | 155.0.8059.39 | 124.2 MB | 281.6 MB (binary 204.7 MB) |
| | `chrome-linux64.zip` | same | 198.2 MB | 416.5 MB (binary 297.7 MB) |
| @sparticuz/chromium (Chromium packed for Lambda) | npm | 153.0.0 | 70.0 MB (Brotli inside) | 70.1 MB |
| Crawlee | `@crawlee/core` npm / Python wheel | 3.18.2 / 1.10.4 | 0.17 / 0.43 MB | 0.75 MB |
| | Docker `apify/actor-node-playwright-chrome` | 2026-10-10 | 879.2 MB | — |
| Puppeteer | `puppeteer-core` npm | 25.13.0 | 1.18 MB | 6.01 MB |
| Stagehand | npm | 4.2.1 | 1.06 MB | 3.51 MB |
| spider | `@spider-rs/spider-rs` platform package, linux-x64-gnu | 0.0.157 | 12.52 MB | 30.90 MB |
| | CLI | v2.52.2 | no prebuilt binary (cargo install only) | — |
| | `spider_client` (npm / PyPI) | 0.2.5 / 0.1.93 | 0.02 MB | 0.08 MB |
| Lightpanda | raw binary, linux x86_64 / macOS | 1.0.0 | shipped uncompressed | 188.3 / 94.1 MB |
| Deno | linux x64 zip → `deno` | 2.9.7 | 41.6 MB | 95.8 MB |
| | `denort` (base of `deno compile`) | 2.9.7 | 34.4 MB | 104.6 MB |
| | PyPI wheel, largest | 2.9.7 | 42.3 MB | — |
| Bun | linux x64 zip → `bun` | 1.4.3 | 42.0 MB | 89.0 MB |
| | npm `@oven/bun-darwin-aarch64` | 1.4.3 | 31.5 MB | 70.5 MB |
| Node.js | `linux-x64.tar.xz` → `bin/node` | 24.21.0 | 31.9 MB | 126.6 MB |
| LLRT (AWS, QuickJS) | `llrt-lambda-x64.zip` | 0.9.0-beta | 6.3 MB | 6.5 MB |
| workerd | `workerd-linux-64.gz` | 1.20261012.1 | 39.1 MB | — |
| DuckDB | CLI linux-amd64 | 1.5.6 | 21.3 MB | 61.9 MB |
| uv / ruff | PyPI wheel, linux x64 | 0.13.0 / 0.17.0 | 17.9 / 10.4 MB | — |
| Biome | `@biomejs/cli-linux-x64` | 2.5.15 | 24.7 MB | 66.7 MB |
| Claude Code | `@anthropic-ai/claude-code-linux-x64` | 2.1.296 | 114.9 MB | 257.1 MB |
| Codex | `@openai/codex` linux-x64 package | 0.162.1 | 165.5 MB | 452.5 MB |

Not found: Playwright's driver zip on its CDN (guessed URLs returned 404) and spider's crate size.

### 4.1 User pains

| Tool | Complaint | Link |
|---|---|---|
| Deno | "Why is the Deno 2 binary 136 MB?" | [deno#26516](https://github.com/denoland/deno/issues/26516) (2024-10) |
| Deno | Ask for a `--strip` to shrink `deno compile` output | [deno#9198](https://github.com/denoland/deno/issues/9198) (2021-01) |
| Deno | Packager finds the binary "very large"; strip + `upx --lzma` makes it 4.66x smaller | [pkgx#937](https://github.com/pkgxdev/pkgx/issues/937) (2024-01) |
| Bun | `bun build --compile` makes a 91 MB "Hello World" | [bun#5854](https://github.com/oven-sh/bun/issues/5854) (2023-09) |
| Bun | "Bun binary big", asks for a strippable build (40 reactions) | [bun#2541](https://github.com/oven-sh/bun/issues/2541) (2023-04) |
| Playwright | `playwright-core` nearly doubled; slower Lambda start | [playwright#7241](https://github.com/microsoft/playwright/issues/7241) (2021-06) |
| Playwright | Lambda / serverless support (34 reactions) | [playwright#2404](https://github.com/microsoft/playwright/issues/2404) (2020-05) |
| Playwright Python | How to shrink the Docker image | [playwright-python#1594](https://github.com/microsoft/playwright-python/issues/1594) (2022-10) |
| Puppeteer | Deploying on Lambda (50 comments) | [puppeteer#323](https://github.com/puppeteer/puppeteer/issues/323) (2017-08) |
| @sparticuz/chromium | The Chromium layer is too big for Lambda | [chromium#217](https://github.com/Sparticuz/chromium/issues/217) (2024-01) |
| Stagehand | Bundled with `playwright-core` it is 51.55 MiB, over Convex's 42.92 MiB limit; asks for a "lite" package | [stagehand#1610](https://github.com/browserbase/stagehand/issues/1610) (2026-01) |
| Crawlee | Bundling for Lambda with esbuild fails (10 reactions) | [crawlee#2208](https://github.com/apify/crawlee/issues/2208) (2023-11) |
| PyPI | Projects ask for 200 MiB to 650 MiB per file, and 100 GB per project | [support#4069](https://github.com/pypi/support/issues/4069), [#6391](https://github.com/pypi/support/issues/6391), [#7112](https://github.com/pypi/support/issues/7112) |
| Prisma | Dropped its Rust engines (about 14 MB, 7 MB gzip, to 1.6 MB) partly over per-OS engine deployment problems | [Prisma blog](https://www.prisma.io/blog/prisma-orm-without-rust-latest-performance-benchmarks), [Prisma 7](https://blog.prisma.io/blog/announcing-prisma-orm-7-0-0) |

Pattern: complaints are about a runtime binary near 100 MB, about Chromium on Lambda, and about bundles that exceed a platform's function limit. None found about the 48 MB Playwright Python wheel or about installing spider.

## 5. Size levers and what they cost

**Measured on the full probe binary** (every part in §3.1 but the Sinks, macOS arm64) **M**. Each profile inherits from the one above it. V8 is a prebuilt static library that no profile recompiles, so the "without V8" column subtracts its 42.78 MB. The `cc` crate passes Cargo's `opt-level` to the C/C++ compiler, so DuckDB, SQLite and BoringSSL are affected too.

| Profile | Stripped | Change | Without V8 | DuckDB query (median of 5) |
|---|---|---|---|---|
| `release`: `lto = "thin"`, `strip` | 94.85 MB | — | 52.07 MB | 1.90 s |
| + `lto = "fat"`, `codegen-units = 1` | 90.67 MB | −4.4% | 47.89 MB | not run |
| + `panic = "abort"` | 89.27 MB | −5.9% | 46.48 MB | 2.00 s |
| + `opt-level = "s"` | 77.80 MB | −18.0% | 35.02 MB | 2.24 s (+18%) |
| `opt-level = "z"` instead | 71.48 MB | −24.6% | 28.69 MB | 3.65 s (+92%) |

The query was `SELECT CAST(sum((i*i) % 7) AS VARCHAR) FROM range(300000000) t(i)` through the `duckdb` crate, one process per run, interleaved across profiles. The host's load average was 540–870, so these timings show direction, not magnitude. Clean build times on that host were 6 min (thin) and 20–23 min (fat) **M**.

| Lever | Size effect | Speed, startup or other cost | Source |
|---|---|---|---|
| `opt-level = "s"` | −28.8% on LLRT (10.99 → 7.83 MB, with LTO, `codegen-units = 1`, `panic = "abort"`) | Mixed workloads +6.6% slower; JSON +22%, regex +23%, HTTP fetch +13%; no startup change (≈5.5 ms warm) | [llrt#1811](https://github.com/awslabs/llrt/issues/1811) **C** |
| `opt-level` per crate (`[profile.release.package.X]`) | Applies `"s"` only to cold crates (Console server, docs, Sinks) | Hot paths keep `3` | [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) **C**; the split itself is an idea, not measured |
| Fat LTO, `codegen-units = 1` | Smaller "but not always" | LTO gives 10–20% or more speed; longer builds | [perf book](https://nnethercote.github.io/perf-book/build-configuration.html) **C** |
| `panic = "abort"` | Removes unwinding code | A panicking Tokio task kills the engine instead of reaching its `JoinHandle`; `catch_unwind` stops working | [catch_unwind](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html), [Tokio Builder](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html) **C** |
| `strip` | Already in the measurements above | Crash backtraces need separately published debug info | [Rust 1.77](https://blog.rust-lang.org/2024/03/21/Rust-1.77.0/) **C** |
| UPX | Claims 50–70% | macOS unsupported since 4.2.0; breaks code signatures; no shared pages, whole image resident; antivirus false positives on Windows | [upx.pod](https://github.com/upx/upx/blob/devel/doc/upx.pod), [NEWS](https://github.com/upx/upx/blob/devel/NEWS), [SANS ISC](https://isc.sans.edu/diary/28960) **C** |
| Store embedded data compressed | Clean-up rules 9.06 → 3.67 MB; ICU 10.88 → ~4.6 MB only with a source build of V8 that loads ICU data through `v8::icu::set_common_data_78` | zstd decompresses at ~1.5 GB/s: about 6 ms for the 9 MB rules (arithmetic), paid once on first use, as ADR 0047 already loads them | [zstd](https://github.com/facebook/zstd) **C** |
| Serve the Console pre-compressed | Embeds the brotli size (0.40 MB instead of 0.99 MB) | None at run time (`Content-Encoding: br`); `tower-http`'s `precompressed_br` expects the uncompressed file too, so embedding only `.br` needs a small handler | [ServeDir](https://docs.rs/tower-http/latest/tower_http/services/struct.ServeDir.html) **C** |
| V8 built without ICU (`v8_enable_i18n_support = false`) | −10.88 MB | `Intl` absent in scripts; needs a source build (~30 min per target) | §3.2 **M**; [rusty_v8](https://github.com/denoland/rusty_v8) **C** |
| V8 from source: lite mode / jitless, no WebAssembly | No size number found | Source build ~30 min per target; jitless is ~40% slower on Speedometer 2 (already chosen by ADR 0051) | [v8 jitless](https://v8.dev/blog/jitless), [v8 lite](https://v8.dev/blog/v8-lite), [rusty_v8](https://github.com/denoland/rusty_v8) **C** |
| V8 as a first-use download | −42.8 MB per platform | A ~15 MB pinned download (gzip -9: 34.5 − 19.2 MB, measured) before the first script (ADR 0043 list), a second signed executable (ADR 0039), changes ADR 0051's "worker mode of the same binary" | measured size; costs from ADRs |
| DuckDB as a first-use download | −33.0 MB per platform (~12 MB gzip, estimated at the measured 36% ratio) | Same kind of costs; only the Console's search and SQL tab and the DuckDB Sink use it | measured size |
| DuckDB extensions on demand (as today) | Keeps `sqlite`, `icu`, `httpfs` out | Network on first use unless `extension_directory` or a local file is used; extension binaries are tied to the DuckDB version by URL | [sqlite extension](https://duckdb.org/docs/current/core_extensions/sqlite.html), [offline install](https://duckdb.org/docs/current/extensions/advanced_installation_methods.html) **C** |

**Startup.** On Linux the binary is mapped and paged in on demand, so file size matters less than the pages touched at start *(secondary sources: [LWN comment](https://lwn.net/Articles/1048297/))*; LLRT's −29% gave no cold-start change. Lambda reads image chunks lazily (§2.1). On macOS the first launch runs Gatekeeper, the code-signature check and XProtect; one trace measured about 0.05 s for the notarization lookup *(unverified: [eclecticlight.co](https://eclecticlight.co/2025/03/26/how-macos-sequoia-launches-an-app/))*. No measured Windows Defender cost by file size was found.

**First-use downloads in practice.** Playwright needed proxy, CA, mirror, cache-path and timeout settings for its browser downloads ([docs](https://playwright.dev/docs/browsers)). `deno compile` and `bun build --compile` download their runtime per target on first use ([Deno](https://docs.deno.com/runtime/reference/cli/compile/), [Bun](https://bun.com/docs/bundler/executables)). esbuild says its install-script fallback is less robust behind proxies ([esbuild](https://esbuild.github.io/getting-started/)) **C**. Goodfetch already has this machinery for browsers and the Clean-up rules (ADRs 0024, 0043, 0047), so one more pinned download reuses it.

**Separate library or executable.** A static musl binary cannot portably `dlopen` a shared library *(unverified here)*; the portable split is a separate executable, as ADR 0051's worker already is. `ort`'s `load-dynamic` shows the dynamic-library pattern on glibc, macOS and Windows ([ort](https://ort.pyke.io/setup/linking)) **C**.

### 5.1 Options for #197 (no recommendation)

Sizes are the §3.3 projection (measured parts plus Sinks and embedded data, before the engine's own code), macOS arm64.

| Option | Binary per platform | What it costs |
|---|---|---|
| A. Everything in one binary | ~105–130 MB, ~40–50 MB gzip | Fits npm, PyPI per file, Releases and images. PyPI's project quota and nightlies need handling (§2.3). Comparable to Deno and Bun. |
| B. V8 downloaded on first use | ~60–90 MB | First script waits for a ~15 MB (gzip) download; offline needs `goodfetch browsers install` or a path; a second artefact to sign and pin. |
| C. DuckDB downloaded on first use | ~70–100 MB | The same costs, on the Console's search and SQL tab and the DuckDB Sink. DuckDB already needs its `sqlite` extension from somewhere. |
| D. Both on first use | ~30–55 MB | Both sets of costs; the smallest packages and PyPI footprint. |
| E. Any of the above + profile levers | §5 | Speed cost of `"s"` where applied; abort semantics if `panic = "abort"`. |

## 6. Method

- **Size probe.** One Cargo package, each part behind a feature; `main` parses `argv[1]` and passes it to each enabled part's main entry point (open a database and run a query, compile a Filter list and match a request, build a client and send a request, run a script, serve a router), so no part is dead code. `cargo build --release --no-default-features --features <part>`, profile `strip = true`, `lto = "thin"`, size by `stat -f %z`. V8 used the same prebuilt `librusty_v8.a` (v152.2.0) as the JavaScript sandbox research, through `RUSTY_V8_ARCHIVE`. Crate versions are those Cargo resolved on 2026-10-12 and are listed in §3.1.
- **Compression.** `gzip -9`, `zstd -19`, `xz -9`, `brotli -q 11` on the stripped binaries and on a tar of the Console's `dist/`.
- **Console.** `git archive origin/prototype/console-screens`, `npm ci --ignore-scripts`, `npx vite build` (Vite 8.3.4), skipping `tsc -b`.
- **Filter lists.** The four list files fetched from easylist.to and fanboy.co.nz, `wc -c` and `gzip -9 | wc -c`.
- **Sinks.** A second package built the same way (`sinkbench`), one feature per Sink library, then all seven.
- **Profiles.** Custom Cargo profiles inheriting from `release` (`fat`, `fatabort`, `smalls`, `small`), built clean; the DuckDB timing ran each binary five times, interleaved, with Python's `time.perf_counter`.
- **ICU.** `ar -x` of `icudtl_dat.o` from `librusty_v8.a` and `size -m`; `size -m` on the V8 probe shows a 12.38 MB `__TEXT,__const` section, and the ICU header string `icudt78l` is present in that binary and absent from the baseline.
- **Competitors.** Registry APIs as listed in §4; no downloaded file was executed.

## 7. Not verified

- npm's real publish limit: no official number, only user reports of `E413`.
- Size of the shipped engine's own code, `goodfetch-protocol`, the MCP server and the remaining Sinks.
- Sizes on static musl (x86_64, aarch64) and Windows for everything but V8; macOS arm64 only here.
- Runtime cost of `opt-level` beyond one DuckDB query on a loaded host, and startup time by profile.
- Size saved by a from-source V8 build in lite mode or without WebAssembly (ICU's 10.88 MB is measured; the rest is not).
- Whether PyPI lets a project delete old dev releases through an API.
- Whether static musl `dlopen` works for a split DuckDB or V8.
- How a large binary affects Windows Defender's first-run scan.
