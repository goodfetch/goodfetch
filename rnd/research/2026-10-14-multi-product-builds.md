# Shipping several products from one native codebase, and what each mechanism costs

Research of **October 14, 2026** for ticket [#450](https://github.com/goodfetch/automan/issues/450) (map [#447](https://github.com/goodfetch/automan/issues/447), feeding [#454](https://github.com/goodfetch/automan/issues/454)). Question: how do native (Rust or similar) projects ship several products or editions from one codebase, and what does each mechanism cost in build, test, signing and user experience? The decision it informs: whether Collie and Foxhound ship as separately installable products from one repository, Foxhound's binary containing Collie, with CI proving no Collie crate depends on a Foxhound crate. That would amend ADRs 0024, 0026, 0032, 0038, 0039 and 0057 and `STYLE.md`'s crate rules.

**Sources.** Source code, Cargo manifests, lockfiles and CI workflows read through the GitHub API at each project's default branch on 2026-10-14 unless a commit or version is given; release asset sizes from `gh api repos/<o>/<r>/releases/latest`; crates.io, npm and PyPI metadata APIs; official docs; issue trackers. Prior Goodfetch research reused and re-checked where cited: [binary-size limits](https://github.com/goodfetch/automan/blob/research/binary-size-budget/rnd/research/2026-10-12-binary-size-limits-and-costs.md) (2026-10-12), [repository layout and CI](https://github.com/goodfetch/automan/blob/research/repo-layout-ci/rnd/research/2026-10-11-repo-layout-ci.md) (2026-10-11), [platform code signing](https://github.com/goodfetch/automan/blob/research/platform-code-signing/rnd/research/2026-10-11-platform-code-signing.md) (2026-10-11), [JavaScript sandbox engines](https://github.com/goodfetch/automan/blob/research/js-sandbox-engines/rnd/research/2026-10-12-js-sandbox-engines.md) (2026-10-12), [Rust test tooling](https://github.com/goodfetch/automan/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md) (2026-10-13).

**Versions inspected.** Codex `rust-v0.162.1` (2026-10-09) and `main` at `835cd7552` (2026-10-14); Deno v2.9.7 (2026-09-17); uv 0.13.1 (2026-10-14); Ruff 0.17.0 (2026-10-09); ty 0.0.87 (2026-10-13); Bun v1.4.3; `spider` 2.53.9 (2026-09-05); Tokio 1.53.3 (2026-10-13); cargo-hack 0.6.45; cargo-deny 0.20.2. Others are given where cited.

**Measured versus cited.** Facts marked **M** were measured for this note (release asset sizes, lockfile graph queries, arithmetic); everything else is cited (**C**) with its link. MB means 10^6 bytes. Like everything in `rnd/`, this note informs a decision and makes none.

> Draft in progress: sections 2–4 and the size estimate are being completed.

---

## 1. Summary

(Draft; completed at the end of the research.)

- **Codex is the closest precedent for "Foxhound contains Collie".** One Cargo workspace (`codex-rs`) ships a superset binary (`codex`, whose `codex app-server` subcommand runs the app server) and a subset binary (`codex-app-server`) built in separate per-target jobs, plus helper executables. Windows x64 sizes: `codex.exe` 333.5 MB, `codex-app-server.exe` 255.2 MB **M**. In 2026-07 Codex moved V8 out of both into a companion executable, `codex-code-mode-host` (74.7 MB on Windows), and made a missing host fall back to direct tools with a one-time warning, or fail closed when code mode is required **C**.

---

## 3. Cases

### 3.1 OpenAI Codex: a superset binary, a subset binary and companion executables

**What ships.** Codex `rust-v0.162.1` (2026-10-09) has 176 release assets **M**. Per target it publishes these executables (Linux x86_64 musl and Windows x64 sizes, **M**):

| Executable | Role | Linux x64 musl, `.tar.gz` | Windows x64 `.exe`, raw |
|---|---|---|---|
| `codex` | the CLI; also runs the app server through `codex app-server` | 111.4 MB | 333.5 MB |
| `codex-app-server` | the app server alone, for IDE and desktop clients | 86.1 MB | 255.2 MB |
| `codex-code-mode-host` | V8 sandbox for code mode, a separate process | 27.5 MB | 74.7 MB |
| `codex-responses-api-proxy` | a privileged proxy | 5.0 MB | 6.0 MB |
| `bwrap` (Linux) / `codex-windows-sandbox-setup`, `-service`, `codex-command-runner` (Windows) | OS sandbox helpers | 0.26 MB | 17.7 / 56.1 / 8.2 MB |
| `codex-package-<target>` | archive of the CLI with its helpers | 162.9 MB | 159.7 MB `.tar.gz` |
| `codex-app-server-package-<target>` | archive of the app server with its helpers | 116.3 MB | 125.8 MB `.tar.gz` |
| `codex-symbols-<target>[-app-server]` | debug symbols, per product | 262.6 MB (CLI), 204.5 MB (app server) | 589.5 MB |

- The CLI depends on the app server's crate: `codex-cli` lists `codex-app-server`, `codex-app-server-daemon` and `codex-app-server-protocol` ([cli/Cargo.toml](https://github.com/openai/codex/blob/main/codex-rs/cli/Cargo.toml)), and its `AppServer` subcommand is "[experimental] Run the app server or related tooling" ([cli/src/main.rs](https://github.com/openai/codex/blob/main/codex-rs/cli/src/main.rs)). `codex-app-server` declares its own `[[bin]]` ([app-server/Cargo.toml](https://github.com/openai/codex/blob/main/codex-rs/app-server/Cargo.toml)) **C**. So the subset product is a library crate with a thin binary, and the superset binary links it, which is the shape the working hypothesis gives Collie and Foxhound.
- **One binary, several names.** The `codex-arg0` crate dispatches on the executable's name: invoked as `codex-linux-sandbox` it runs the Landlock sandbox, as `apply_patch` it applies a patch, and the CLI puts such aliases on a temporary `PATH` entry for its children ([arg0/src/lib.rs](https://github.com/openai/codex/blob/main/codex-rs/arg0/src/lib.rs)) **C**. The Linux sandbox helper is therefore not a separate file to sign, while the Windows sandbox helpers are.

**V8 moved into a companion executable.** Code mode (an agent writing JavaScript that calls tools) runs in V8.
- In the lockfile at `main` (2026-10-14), `v8` is a dependency only of `codex-code-mode-runtime` (and of a `codex-v8-poc` crate), and `codex-code-mode-runtime` only of `codex-code-mode-host`; `codex`, `codex-app-server` and `codex-core` depend on `codex-code-mode`, the client side, which has no V8 **M** ([Cargo.lock](https://github.com/openai/codex/blob/main/codex-rs/Cargo.lock), queried with `tomllib`). The runtime turns on V8's sandbox (`v8 = { features = ["v8_enable_sandbox"] }`) and links ICU data through `deno_core_icudata` ([code-mode-runtime/Cargo.toml](https://github.com/openai/codex/blob/main/codex-rs/code-mode-runtime/Cargo.toml)) **C**.
- The move was a four-phase stack: "Add protocol and host crates…", "Create the new standalone binary", "Create a new IPC `CodeModeSessionProvider`", "Remove v8 from core and only use IPC provider" ([#27724](https://github.com/openai/codex/pull/27724), merged 2026-06-12). The last phase "Move[d] the V8 implementation into a dedicated `codex-code-mode-runtime` crate used by `codex-code-mode-host`, removing the embedded runtime fallback from the Codex process" ([#36217](https://github.com/openai/codex/pull/36217), merged 2026-07-30) **C**.
- **How a missing part fails.** The same PR resolves "the host executable from the active installation layout", checks "its availability before selecting tools", and falls back "to direct tools with a one-time warning when optional code mode is unavailable", while "`code_mode_only` and `disable_in_process_fallback` configurations" stay "fail-closed"; its tests cover "missing hosts and symlinks" ([#36217](https://github.com/openai/codex/pull/36217)) **C**. Later PRs added gRPC transport and remote code-mode hosts for the app server ([#35098](https://github.com/openai/codex/pull/35098), 2026-07-24; [#38041](https://github.com/openai/codex/pull/38041), 2026-08-11) **C**.

**What it costs in CI.**
- The release workflow builds per target and per **bundle**: `primary` builds `codex codex-code-mode-host codex-responses-api-proxy` (plus `bwrap` on Linux), `app-server` builds `codex-app-server codex-code-mode-host`, each with its own `cargo build --bin …` on its own runner, for four Unix targets and, in a separate workflow, two Windows targets with a third bundle of sandbox helpers ([rust-release.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml), [rust-release-windows.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release-windows.yml)) **C**. Each matrix is also doubled across `build-system: [cargo, bazel]`, the Bazel half `continue-on-error`. So the host is compiled twice per target, and the workflow skips the copy from the `app-server` bundle: "Both variants package the host, but only the primary bundle publishes standalone binary archives to avoid duplicate release asset names" **C**.
- Build jobs have `timeout-minutes: 120` on `macos-15-xlarge` and similar runners **C**. Larger runners are billed even for public repositories (§5.4), so this layout is not available on Goodfetch's free tier as written.
- macOS signing runs as a separate job per target and bundle, on `ubuntu-latest`, in the protected `codesigning` environment, with `rcodesign` and an Azure Key Vault key reached over PKCS#11 ([rust-release.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml)) **C**: four signing jobs per build system for two macOS targets, each signing two or three executables.
- Debug symbols are archived per bundle "before stripping the Cargo binaries", with `CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed` on macOS ([rust-release.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml)) **C**; each product's symbol archive is larger than its binary (table above).

**Boundary enforcement.** Codex's `deny.toml` uses cargo-deny's `wrappers` as a dependency-direction ratchet: `reqwest` is banned except as a direct dependency of `codex-http-client` ("Intended owner"), `codex-code-mode`, `codex-responses-api-proxy` ("owns its blocking upstream HTTP transport independently of Codex") and listed third-party crates, with the reason "new direct reqwest dependencies must use codex-http-client; temporary wrappers track migration debt"; `async-trait` is banned the same way ([deny.toml](https://github.com/openai/codex/blob/main/codex-rs/deny.toml)) **C**. No check was found that the `codex-app-server` binary lacks CLI-only crates (**unverified**: only `deny.toml` and the release workflows were read).

---

## 5. Cargo features, combination testing and product boundaries

### 5.1 Cargo features as the product switch

- The Cargo book says features "should be _additive_. That is, enabling a feature should not disable functionality", and that mutually exclusive features "should be avoided if at all possible, because it requires coordinating all uses of the package in the dependency graph" ([Features, feature unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification), read 2026-10-14). Removing a feature, or removing one from `default`, is a SemVer-breaking change ([SemVer compatibility](https://doc.rust-lang.org/cargo/reference/features.html#semver-compatibility)) **C**.
- "By default, tests, docs, and other tooling such as Clippy will only run with the default set of features." The book leaves the combination strategy to each project ([Feature combinations](https://doc.rust-lang.org/cargo/reference/features.html#feature-combinations)) **C**.
- **A workspace build unifies features across the packages it selects.** "When a dependency is used by multiple packages, Cargo will use the union of all features enabled on that dependency" ([feature unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)). So `cargo build --workspace` builds a shared dependency once with the union of every product's features, and a Collie binary built that way can differ from the same binary built with `cargo build -p collie`. The issue has been open since 2017 ([cargo#4463](https://github.com/rust-lang/cargo/issues/4463), "Feature selection in workspace depends on the set of packages compiled", 36 thumbs-up). The fix is unstable: `-Z feature-unification` with `resolver.feature-unification = "selected" | "workspace" | "package"`, default `"selected"`; `"package"` builds dependencies per package "preferring duplicate builds" ([Cargo unstable: feature-unification](https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification), tracking [cargo#14774](https://github.com/rust-lang/cargo/issues/14774), open) **C**. Until it is stable, each product binary must be built in its own `cargo build -p <product>` invocation to match what a user's `cargo install` gets. Codex's per-bundle release builds (§3.1) are consistent with that, though its workflow does not give the reason.
- **Feature-chosen editions are what Homebrew gave up.** Homebrew 2.0.0 (2019-02-02) removed every option from homebrew-core because "Formulae with options had to be built from source, could not be tested on our CI system and provided a disproportionate support burden on our volunteer maintainers" ([Homebrew 2.0.0](https://brew.sh/2019/02/02/homebrew-2.0.0/)) **C**. Options survive only in third-party taps.

### 5.2 Spider-rs: 129 features, a handful tested

- `spider` 2.53.9 (2026-09-05) declares **129 features** (crates.io API, read 2026-10-14) **M**, including `chrome`, `chrome_stealth`, `smart`, `webdriver`, `agent_*`, `cache_*`, `disk`, `io_uring`, `wreq` and ten `reqwest_*` TLS variants ([crates.io](https://crates.io/crates/spider/2.53.9/features)).
- Its CI tests a few fixed sets, not combinations: `cargo check --workspace`, `cargo test -p spider_agent` with default and with `openai search_serper`, `cargo clippy -p spider --features chrome`, a `--features chrome` stack-frame guard gated on live-test secrets, and `io_uring` on and off ([rust.yml](https://github.com/spider-rs/spider/blob/main/.github/workflows/rust.yml), [lint.yml](https://github.com/spider-rs/spider/blob/main/.github/workflows/lint.yml), main on 2026-10-14) **C**. No `cargo hack` anywhere in its workflows.
- Users hit the untested combinations in released versions: `glob` broken in 2.44.8 ([#348](https://github.com/spider-rs/spider/issues/348), 2026-02-04: "Probably needs a co-dependent feature"), `chrome` broken in 2.44.19 ([#349](https://github.com/spider-rs/spider/issues/349), 2026-02-06), `adblock` broken in `spider_chrome` 2.36.74 ([#288](https://github.com/spider-rs/spider/issues/288), 2025-04-23), `cache_chrome_hybrid` E0432 ([#245](https://github.com/spider-rs/spider/issues/245), 2025-01-17), and behaviour that changes with `chrome` on ([#201](https://github.com/spider-rs/spider/issues/201), `with_limit(1)`, 2024-08-08) **C**.
- Spider's CLI is itself a feature-chosen edition: `spider_cli` defaults to `status_code, chrome, warc, spider_cloud`, and its README documents `cargo install -F smart spider_cli` ([spider_cli/Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider_cli/Cargo.toml), [README](https://github.com/spider-rs/spider/blob/main/spider_cli/README.md)). Its workspace holds `spider`, `spider_worker`, `spider_cli`, `spider_utils`, `spider_agent`, `spider_agent_types`, `spider_agent_html` and `spider_mcp` ([Cargo.toml](https://github.com/spider-rs/spider/blob/main/Cargo.toml)) **C**. The last GitHub release with assets is v2.52.2 (2026-03-31).

### 5.3 Tokio and cargo-hack

- Tokio 1.53.3 (2026-10-13) has 17 features ([crates.io](https://crates.io/crates/tokio/1.53.3/features)) **M**. Its CI uses `cargo hack` in at least six places ([ci.yml](https://github.com/tokio-rs/tokio/blob/master/.github/workflows/ci.yml), read 2026-10-14) **C**:
  - `cargo hack test --each-feature` for the integration and macro build test crates;
  - `cargo hack check --all --feature-powerset --exclude-features "…" --depth 2 --keep-going`, in a three-way matrix (stable flags, `--cfg tokio_unstable`, unstable plus `io-uring,taskdump,schedule-latency`);
  - `--feature-powerset --depth 2` again for i686, and `--each-feature` for cross targets with `-Zbuild-std` and for `wasm32-unknown-emscripten`, each with an exclusion list;
  - `cargo hack --remove-dev-deps` plus `-Z minimal-versions` checks.
  The workflow has 45 `runs-on` jobs in total.
- cargo-hack v0.6.45 (2026-05-30) options ([README](https://github.com/taiki-e/cargo-hack#readme)) **C**: `--each-feature` runs each feature alone plus `--no-default-features`, default, and `--all-features`; `--feature-powerset` runs every combination, "deduplicate[s] any fully equivalent feature combinations", and recommends `--depth` "when using this flag results in a very large number of feature combinations"; `--group-features`, `--mutually-exclusive-features`, `--at-least-one-of`, `--exclude-features`, `--optional-deps`, `--partition M/N` (split runs across CI jobs) and `--keep-going`. It recommends `--no-dev-deps` to avoid [cargo#4866](https://github.com/rust-lang/cargo/issues/4866).
- **Arithmetic** for 129 features (not run) **M**: the full powerset is 2^129 builds; `--depth 2` is 129 + C(129,2) = 129 + 8,256 = 8,385 builds plus the default, no-default and all-features runs; `--each-feature` is 132. Tokio keeps its powerset tractable with 17 features, exclusions and depth 2.

### 5.4 Proving product A contains none of product B's code

Cargo itself enforces the first boundary: a crate can only name crates it depends on, and the crate graph has no cycles. So "the `collie` crates never depend on `foxhound` crates" is a compile error to break, not a convention. What it does not catch is a heavy dependency reaching Collie through a shared crate, or through a feature turned on in a shared crate. The tools below close that gap.

| Tool | What it proves | How it runs in CI | Who uses it | Limits |
|---|---|---|---|---|
| `cargo tree -i <crate> -e normal -p <product>` | The crate is absent from one product's normal-dependency graph for a given feature set; exits non-zero when absent | uutils coreutils: "Verify ariadne is compiled out" fails if `cargo tree -p coreutils --no-default-features --features feat_os_unix -e normal -i ariadne` succeeds ([CICD.yml](https://github.com/uutils/coreutils/blob/main/.github/workflows/CICD.yml), 2026-10-14) | uutils | Crate-level only; checks a graph, not the linked binary; one feature set per run |
| cargo-deny `[bans] deny` | Named crates are banned from the graph; `wrappers` allows only listed direct parents; `features` deny/allow/exact per crate ([bans config](https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html)) | `cargo deny --manifest-path crates/<product>/Cargo.toml check bans`: a normal package manifest makes that crate the only root ([CLI common options](https://github.com/EmbarkStudios/cargo-deny/blob/main/docs/src/cli/common.md)); one config per product | Codex's `reqwest` ratchet (§3.1); widespread for licences and advisories (0.20.2, 2026-07-09) | Crate-level; `--features` only for a single crate; a per-product ban list needs one config or invocation per product |
| guppy / cargo-hakari workspace-hack | Nothing about absence; it pins one feature set per shared dependency to stop rebuilds ("up to 1.7x" cumulative, Omicron workspace) ([cargo-hakari 0.9.39 about](https://docs.rs/cargo-hakari/latest/cargo_hakari/about/index.html)) | `cargo hakari generate` / `verify` | Oxide (`omicron-workspace-hack`); Firefox and rustc have hand-made equivalents | **Works against a product boundary**: it unifies features across products on purpose, so a lean product inherits the full product's feature set; "some crates may accidentally start skipping features they really need" |
| Symbol or size check on the built binary (`nm`, `cargo bloat`) | Absence in the linked artefact itself | Not found in a competitor workflow in this pass | — | Needs a release-profile build per product |

### 5.5 Sharing one protocol between products

- Cargo forbids a product crate from reaching another product's crates, so the shared contract has to live in a crate both depend on. Goodfetch already has one (`goodfetch-protocol`, ADR 0038). Codex does the same: `codex-app-server-protocol` is shared by the CLI and the app server, and `codex-code-mode-protocol` was extracted first, as "shared session, runtime, response, and tool-definition types", before V8 left the main binaries ([#27724](https://github.com/openai/codex/pull/27724)) **C**.
- Rust has no stable ABI, so the protocol crate is shared at build time, not at run time: Bevy deprecated dynamic plugins in 0.14 because "the current dynamic plugin system is unsound and will be removed in 0.15", noting that Rust "may produce incompatible libraries across Rust versions, or even subsequent compilations" ([bevy_dynamic_plugin docs](https://docs.rs/bevy_dynamic_plugin), feedback [bevy#13080](https://github.com/bevyengine/bevy/issues/13080)); its docs point to `stabby` for a stable ABI **C**.

### 5.6 Parts downloaded on first use: DuckDB extensions

- Built-in extensions are "statically linked into the binary"; others are installed once and loaded each session, and autoloading "can install and load the core extensions as soon as they are used in a query" ([extensions overview](https://duckdb.org/docs/current/extensions/overview.html)) **C**.
- Installed extensions live under `~/.duckdb/extensions/<duckdb_version>/<platform>/` and are shared only between clients of "the same DuckDB version"; `extension_directories` exists for "package managers or air-gapped environments"; `INSTALL … FROM '<url>'` and `custom_extension_repository` point at a mirror ([installing extensions](https://duckdb.org/docs/current/extensions/installing_extensions.html)) **C**.
- Moving a part out of the binary broke offline users on upgrade: after DuckDB JDBC 1.5.0.0, `CALL start_ui_server()` "now attempts to automatically download the `ui` extension", failing with `Extension Autoloading Error … Failed to download extension "ui" at URL "http://extensions.duckdb.org/v1.5.0/osx_arm64/ui.duckdb_extension.gz"` ([duckdb-java#623](https://github.com/duckdb/duckdb-java/issues/623), 2026-03-26, open) **C**. ADR 0057 already records why signed, dlopen'ed extensions do not fit a static musl binary or the hardened runtime.

## 6. Release cost per extra executable

### 6.1 Signing and notarization

- Platform-signing facts are in the [platform code signing research](https://github.com/goodfetch/automan/blob/research/platform-code-signing/rnd/research/2026-10-11-platform-code-signing.md) (2026-10-11). The ones that scale with the number of executables:
  - The notary service accepts DMG, signed flat `.pkg` and ZIP; a ticket "can't be stapled" to a ZIP or to a standalone binary, so a bare CLI is checked online ([Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow), as cited there). One ZIP can carry several binaries in one submission, but each Mach-O inside must be signed with the hardened runtime and a timestamp ([Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)) **C**.
  - Windows SmartScreen reputation builds "from the file hash plus the publisher certificate", and each new unsigned version starts from zero ([SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)) **C**. A second executable is a second file to sign on every release; signed with the same certificate it inherits publisher reputation.
  - Astral (uv, ruff) and Codex sign and notarize from Linux with `rcodesign` and a Key Vault key; that is per file but scripted ([uv sign-release-binaries.yml](https://github.com/astral-sh/uv/blob/main/.github/workflows/sign-release-binaries.yml), [codex rust-release.yml](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml)) **C**.
- Packaging tools treat each product as its own app: dist (cargo-dist) makes each Cargo package with binaries an "App" with its own archive per target; several binaries of one package ship together in one archive; a bare `v1.0.0` tag releases every package with that version, while `my-app-v1.0.0` releases one ([dist concepts](https://axodotdev.github.io/cargo-dist/book/reference/concepts.html)) **C**.

### 6.2 Debug symbols

- Codex rust-v0.162.1 publishes symbols per product and per target: on aarch64-apple-darwin `codex-symbols-…-app-server.tar.gz` is 267.6 MB beside 346.5 MB for the CLI; Windows x64 symbols are 589.5 MB (GitHub release assets, read 2026-10-14) **M**. So every extra product adds a symbol archive per target, each larger than the binary.
- Bun publishes a `-profile` build per target (145.9–380.2 MB zipped, bun-v1.4.3) **M**, presumably its unstripped variant (**unverified**: Bun's installation docs do not describe profile builds). Bun's docs now say it ships "a single x64 binary per platform", keeping the `-baseline` assets only as aliases ([installation](https://bun.com/docs/installation), read 2026-10-14) **C**. uv 0.13.1, ruff 0.17.0, Deno v2.9.7 and sentry-cli 3.8.0 publish no symbol assets (asset lists, read 2026-10-14) **M**.

### 6.3 Reproducibility

- Because of workspace feature unification (§5.1), a product binary is only reproducible if it is built with the same package selection every time.
- Cargo's `profile.trim-paths` (RFC 3127) went through a call for testing on 2026-09-09 before stabilization; its tracking issue closed on 2026-10-06 ([cargo#12137](https://github.com/rust-lang/cargo/issues/12137)) **C**. Whether it is in a stable Cargo release yet is **unverified**.

### 6.4 CI matrix and the free tier

- GitHub Actions is free for "public repositories that use standard GitHub-hosted runners"; larger runners "are always charged for" ([Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)) **C**. The Free plan runs 20 concurrent jobs, at most 5 on macOS; a job may run 6 hours; a matrix may generate 256 jobs per run ([Actions limits](https://docs.github.com/en/actions/reference/limits)) **C**.
- ADR 0057 records fat LTO at about 20 minutes per target. A second product binary built in its own `cargo build -p` (needed for the reason in §5.1) roughly doubles release-profile link work per target, unless the products share one binary. That figure is arithmetic, not a measurement.

### 6.5 How a missing part fails

- Codex: a missing code-mode host falls back to direct tools with a one-time warning, or fails closed when configured as required (§3.1) **C**.
- Playwright 1.64.1 throws `Executable doesn't exist at <path>` with a box saying "Looks like Playwright was just installed or updated. Please run the following command to download new browsers:" and the exact install command for the SDK's language, or for `ffmpeg` "Video rendering requires ffmpeg binary" ([registry/index.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/registry/index.ts)) **C**. Issues with that text are among its most commented: [#7249](https://github.com/microsoft/playwright/issues/7249) (caching browsers in CI, 60 comments), [#5767](https://github.com/microsoft/playwright/issues/5767) (21), [#12772](https://github.com/microsoft/playwright/issues/12772) (16), [#33673](https://github.com/microsoft/playwright/issues/33673) (2024-11-19).
- Splitting one part into two variants breaks pipelines that mirror parts: after Playwright split Chromium into `chromium` and `chromium-headless-shell`, a user mirroring browsers from `browsers.json` found the shell missing in 1.50.0 ([#34508](https://github.com/microsoft/playwright/issues/34508), 2025-01-27), and another got `Executable doesn't exist at …/chromium_headless_shell-1200/…` only on 1.58 inside Docker ([#39122](https://github.com/microsoft/playwright/issues/39122), 2026-02-04) **C**.
- Cargo answers a missing subcommand with "no such command: `x`", then "help: view all installed commands with `cargo --list`" and "help: find a package to install `x` with `cargo search cargo-x`" ([cargo main.rs](https://github.com/rust-lang/cargo/blob/master/src/bin/cargo/main.rs)) **C**.
- pandas 3.0.6 names the package to install for a missing optional dependency: "`Import X` failed. … Use pip, conda, or your preferred package management tool to install the X package." ([_optional.py](https://github.com/pandas-dev/pandas/blob/main/pandas/compat/_optional.py)) **C**.

### 6.6 Editions that share a namespace

- opencv-python ships four packages and says "you should **SELECT ONLY ONE OF THEM**. Do not install multiple different packages in the same environment. There is no plugin architecture: all the packages use the same namespace (`cv2`)" ([README](https://github.com/opencv/opencv-python#installation-and-usage)) **C**. Its headless edition still broke headless hosts in 4.13.0.90 ([#1183](https://github.com/opencv/opencv-python/issues/1183), 2026-01-19).
- psycopg2 ships `psycopg2` and `psycopg2-binary` with the same import name ([README](https://github.com/psycopg/psycopg2#installation)) **C**.

### 6.7 What the direct competitors do

- Crawlee layers its crawler on browser libraries as optional peers, not as an edition: `crawlee` 3.18.2 depends on every `@crawlee/*` package and declares `playwright` and `puppeteer` as optional `peerDependencies`; `@crawlee/playwright` does the same (npm metadata, read 2026-10-14) **C**. Crawlee for Python 1.10.4 uses extras: `crawlee[playwright]`, and a `stagehand` extra that pulls Playwright, so a Crawlee handler can hand its page to Stagehand ([PyPI](https://pypi.org/project/crawlee/1.10.4/)) **C**.
- Playwright and Puppeteer each split a core library from the package that downloads browsers: `playwright-core` 1.64.1 (13.66 MB unpacked, no dependencies) under `playwright` (5.03 MB); `puppeteer-core` 25.13.0 (6.01 MB) under `puppeteer` (npm metadata, 2026-10-14) **C**.

## 7. Sizes

### 7.1 What each part costs

Most numbers come from ADR 0057's probe (macOS arm64, stripped, thin LTO, Rust 1.93), measured in the [binary-size research](https://github.com/goodfetch/automan/blob/research/binary-size-budget/rnd/research/2026-10-12-binary-size-limits-and-costs.md) (2026-10-12).

| Part | Variant | Added to the binary | Source |
|---|---|---|---|
| Tokio + serde_json | baseline | 0.62 MB total | binary-size research §3.1 |
| SQLite | `rusqlite` 0.40.2 `bundled` | +1.76 MB | same; SQLite itself: "less than 900KiB" with all features ([about](https://www.sqlite.org/about.html), 2025-11-13) |
| CDP transport | `tokio-tungstenite` 0.30 (stand-in for the in-house client) | +0.37 MB | binary-size research §3.1 |
| CDP generated types | `chromiumoxide_cdp` 0.9.1: 111k generated lines, `.crate` 651 KB | not measured | [CDP client research](https://github.com/goodfetch/automan/blob/research/rust-cdp-clients/rnd/research/2026-10-10-rust-cdp-clients.md); crates.io API |
| HTTP + rustls | `reqwest` 0.13.5, aws-lc-rs, http2, compression | +5.15 MB | binary-size research §3.1 |
| HTTP impersonation | `wreq` (BoringSSL) | +5.49 MB | same |
| Console server | `axum` + `tower-http` | +2.11 MB | same |
| Extraction / filter lists | `dom_smoothie` +1.43, `adblock` +2.91 | +4.34 MB | same |
| DuckDB | core, bundled | +33.02 MB | same |
| V8 | `v8` 152.2.0 prebuilt, jitless; 10.88 MB of it is ICU | +42.81 MB | same; static libraries 38–41 MB gzipped per target ([rusty_v8 v152.2.0 assets](https://github.com/denoland/rusty_v8/releases/tag/v152.2.0)) |
| Sinks | seven sink libraries | up to +21.46 MB | same |
| rustls provider | ring vs aws-lc-rs | **unverified**: no published comparison found; `aws-lc-sys` 0.46.0's `.crate` is 9.66 MB against `ring` 0.17.14's 1.50 MB | crates.io API |

### 7.2 A Collie-only binary, by arithmetic

Code mode and Script Hooks run in V8 (ADR 0051), and both serve Automation as much as Crawl. So the answer depends on whether Collie keeps them.

| Build | Parts | Estimate (macOS arm64, stripped) |
|---|---|---|
| Full `goodfetch` today | everything measured | 94.85 MB + Sinks + engine code (ADR 0057: 95–120 MB) |
| Collie with Code mode and Script Hooks | baseline, mimalloc, rusqlite, CDP, reqwest/rustls, axum, V8 | 0.62 + 0.10 + 1.76 + 0.37 + 5.15 + 2.11 + 42.81 ≈ **52.9 MB** (−44%) |
| Collie without V8 | same without V8 | ≈ **10.1 MB** |
| Collie without V8 and the Console server | same without axum | ≈ **8.0 MB** |

These are sums of separate measurements. Parts share code (the research found individual additions over-sum by 0.8 MB), the engine's own code is not counted, and musl Linux is 8–12% larger for V8. The browser download dwarfs all of them: Chrome headless shell is about 124 MB compressed (ADR 0057).
