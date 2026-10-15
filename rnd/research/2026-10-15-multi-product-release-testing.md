# Releasing, signing and testing a superset product and a subset product from one workspace

Research of **October 15, 2026** for the wayfinder ticket [#459 Release, signing and testing of two products](https://github.com/goodfetch/automan/issues/459). Context: Goodfetch ships Collie (one-page browser automation) and Foxhound (a crawler whose executable is built from Collie's crates plus its own, so Foxhound contains Collie), from one Cargo workspace, at one version, from one commit.

**Sources.** Workflow files, manifests and test folders of the comparable projects, each read at a pinned commit on 2026-10-14/15 (table in §11); official documentation (Cargo, PyPI, Apple, Microsoft, GitHub, Playwright, Crawlee, uv); registry APIs (PyPI JSON, MCR and Docker Hub tag lists, GitHub Releases, Azure Retail Prices); maintainers' issues and pull requests for user pains. Every claim carries its link. Claims that could not be confirmed in a primary source are marked **unverified**. Like everything in `rnd/`, this note informs a decision and makes none.

---

## 1. Summary

- **A. Testing.** None of the four precedents runs the subset product's full test suite against the subset's own release build. The pattern is: the subset's logic is tested once inside the workspace-wide test run (features unified), plus a thin extra layer on the subset's own artefact.
  - **uv / `uv_build`:** the build backend is tested through the *superset* binary (`uv build-backend`, 31 integration tests) plus 29 unit tests, in one `cargo nextest run --workspace`. The standalone `uv_build` gets one dedicated CI job that builds it alone and drives it through pip, uv and `python -m build`, and `--help` smoke tests on every release wheel. That job was added after a release shipped a broken `uv_build` shim ([uv#12058](https://github.com/astral-sh/uv/pull/12058)). A `cargo deny` check on `crates/uv-build/Cargo.toml` alone bans the network stack from the subset's dependency graph.
  - **Codex / `codex-app-server`:** the app-server suite (173 `.rs` files under `tests/suite`) spawns the `codex-app-server` binary built by the workspace test build; the `codex` CLI has its own 4 tests of its `app-server` subcommand. Releases build `codex-app-server` in a **separate** `cargo build --bin` job (its own feature resolution) and only verify its signatures, not its behaviour.
  - **Deno / `denort`:** `deno` and `denort` are built in **one** `cargo build -p deno -p denort -p test_server` invocation (same features), and the built `denort` is downloaded into the `integration` and `specs` test jobs, where `deno compile` tests run it end to end.
  - **Zed / remote server:** tests run once with `cargo nextest run --workspace`; the remote server's own package is only compile-checked on its own (`cargo check --release --package remote_server`) for three targets.
- **B. Releasing together.** All three gate *publication* on *every* product's build: one failed build publishes nothing. Publication itself is not atomic. uv publishes `uv` and `uv_build` to PyPI in two parallel jobs, with Docker in parallel; the GitHub release waits for both; crates.io is last and "isn't idempotent on retry". Codex creates a draft GitHub release, uploads with `--clobber` and only then un-drafts it; npm skips already-published versions. Deno uploads each platform's files from its own build job to S3 and to a draft GitHub release. Browserless deliberately lets the images that built publish when another image failed.
- **C. Changelogs.** uv: one CHANGELOG, sections from PR labels; there is an `area:build-backend` label but no build-backend section. Codex: one release note per tag, taken from the tag commit's message, no per-product split. Deno: one `Releases.md` of conventional-commit titles whose scopes (`compile`, `desktop`) name the area. Crawlee (lerna, fixed mode): one CHANGELOG per package, **attributed by commit path**. As a result the `crawlee` meta-package's 3.18.1 entry reads "Version bump only" although it shipped core and Playwright fixes.
- **D. OCI images.** Playwright: four MCR repositories (18,606 tags in the base one), tags `vX.Y.Z[-noble|-jammy|-resolute][-amd64|-arm64]`; it stopped publishing `:latest` in v1.47. Apify: 21 `actor-*` repositories on Docker Hub, tags `<node>-<library version>[-slim]` (723 tags on `actor-node-playwright-chrome`). Browserless: 8 browser images, tags `latest` and `v2.x.y`. uv: one distroless image plus 44 derived-base tags, each also with `X.Y.Z-<base>` and `X.Y-<base>`; no image for `uv_build`. User pains found: version mismatch between image and library, an undocumented `latest`, and missing tags after partial publishes.
- **E. PyPI.** Defaults are **100 MiB per file and 10 GiB per project** (`MAX_FILESIZE = 100 * ONE_MIB`, `MAX_PROJECT_SIZE = 10 * ONE_GIB`; the docs say "100.0 MB" and "10.0 GB"). An increase is a [pypi/support](https://github.com/pypi/support) issue; no fee is mentioned anywhere. 114 project-limit requests closed in 2026 had a median of 14 days (IQR 8–23); 27 were open. DuckDB went 10 → 30 GB after 18 months (2021–2022, with pushback on per-merge dev releases), then 30 → 100 GiB in 62 days (2025). An organization has **no pooled quota**: an admin-set organization limit raises *each* of its projects' limits.
- **F. Signing.** Apple notarization is reached through the 99 USD/year Apple Developer Program (Developer ID). No per-submission fee is documented. Apple's only documented limit is "Limit notarizations to 75 per day". One ZIP, DMG or PKG submission can hold several executables. Azure Artifact Signing (formerly Trusted Signing): **Basic 9.99 USD/month, 5,000 signatures/month; Premium 99.99 USD/month, 100,000; overage 0.005 USD per signature**. It needs a paid Azure subscription and is not prorated.
- **G. GitHub Releases.** "Each file … must be under 2 GiB"; "Up to 1000 release assets"; "no limit on the total size of a release, nor bandwidth usage".
- **H. Cargo.** Confirmed: building several workspace packages at once unifies their dependencies' features. Avoiding that "requires separate `cargo` invocations". `resolver.feature-unification` (`selected` default, `workspace`, `package`) is still **unstable** (nightly `-Z feature-unification`). Its tracking issue [cargo#14774](https://github.com/rust-lang/cargo/issues/14774) is open, with no open stabilization PR on 2026-10-15. The underlying issue [cargo#4463](https://github.com/rust-lang/cargo/issues/4463) has been open since 2017. No project was found that asserts two binaries resolve shared crates with the same features. Projects use separate per-package builds (uv, Codex release), per-package checks (Zed, Deno), a per-manifest dependency ban (uv), or a feature-unifying hack crate (Zed until October 2025).
- **I. Timing.** All three subsets were added later, not shipped from the start:
  - `uv_build`: first upload 2025-02-12 (rc), first release 2025-03-06, a year after uv (2024-02-15).
  - `codex-app-server` as its own asset: 2026-04-24, about a year after the Rust CLI's first GitHub release (2025-04-30).
  - `denort`: first asset in Deno v1.40.5 (2024-02-15), 5.5 years after Deno v0.1.0.

---

## 2. A. Which tests run against which executable

### 2.1 Overview

| | Superset → subset | Subset logic tested where | Subset's own built binary run in CI | Subset tested under its own build's feature set |
|---|---|---|---|---|
| **uv** | `uv` contains the build backend; `uv_build` ships it alone on PyPI | Workspace `cargo nextest run --workspace` (with test features): 29 unit tests in `uv-build-backend`, 31 integration tests through `uv build-backend` | Yes: one integration job builds the `uv_build` wheel and uses it as PEP 517 backend via pip, uv and `build`; every release wheel and sdist gets `--help` smoke tests | Yes for that job and for releases: maturin builds `-m crates/uv-build/Cargo.toml` alone. `cargo deny --manifest-path crates/uv-build/Cargo.toml check bans` checks the subset's own graph |
| **Codex** | `codex` CLI links `codex-app-server`; `codex-app-server` is a standalone release asset | `codex-rs/app-server/tests` (173 `.rs` files under `tests/suite`) spawn the `codex-app-server` binary from the test build; `codex-rs/cli/tests/app_server.rs` (4 tests) runs `codex app-server` | Yes, but the test-build binary (workspace-unified), not the release binary. The release binary is only signature-verified | No: tests use the workspace build; the release builds `--bin codex-app-server --bin codex-code-mode-host` in a separate job, so its features can differ from what was tested |
| **Deno** | `deno compile` writes the user's program onto `denort`; `denort` is also a release asset | `tests/specs/compile` (73 cases) and integration tests drive `deno compile`, which uses the built `denort` (`DENORT_BIN`) | Yes: the `denort` artefact from the build job is downloaded into the `integration` and `specs` test jobs | Shared: `deno` and `denort` are built in one invocation, so the shipped `denort` has the unified features that were tested. A separate `cargo check -p deno -p denort --features quickjs` covers the QuickJS variant |
| **Zed** | Editor and remote server share crates (not a strict superset) | `cargo nextest run --workspace`; remote-editing tests run the server in process with fakes (`FakeFs`, `FakeHttpClient`) | Not found | Compile-only: `cargo check --locked --release --package remote_server` on Linux musl, macOS arm64, Windows x64 |

### 2.2 uv and `uv_build` (commit `3ea045bc`, 2026-10-14)

- **Workspace test run.** Linux: `cargo nextest run --locked … --features test-python-patch,test-universal,native-auth,secret-service --workspace --profile ci-linux`. macOS and Windows use `--no-default-features` with their own feature lists ([test.yml L120–126](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/test.yml#L120-L126)). One run, features unified across the workspace.
- **The subset's logic is tested through the superset binary.**
  - [`crates/uv/tests/build/build_backend.rs`](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/crates/uv/tests/build/build_backend.rs) has 31 `#[test]`s. They call `context.build_backend()`, which runs the `uv` binary's hidden `build-backend` subcommand ([uv-test lib.rs L1802](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/crates/uv-test/src/lib.rs#L1802)).
  - The binary path comes from `NEXTEST_BIN_EXE_uv`/`CARGO_BIN_EXE_uv`.
  - The file says: "We can't test end-to-end here including the PEP 517 bridge code since we don't have a uv wheel."
  - `uv-build-backend` has 22 + 7 unit tests (`lib.rs`, `wheel.rs`).
- **The subset's own artefact gets one CI job.** `integration-uv-build-backend` ([test-integration.yml L1156–1205](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/test-integration.yml#L1156-L1205)) does the following:
  1. It runs `./uv build -v crates/uv-build`, building the `uv_build` package on its own.
  2. It installs a test package through pip using that backend.
  3. It builds sdist and wheel through uv with `--force-pep517`.
  4. It builds through `python -m build`.
  5. Each time it imports the result.
  - Its comment: the Python package "is not covered by uv's integration tests since they can't depend on having a Python package (only the binary itself is built before running Rust's tests)".
- **Why that job exists.** [uv#12058](https://github.com/astral-sh/uv/pull/12058) (merged 2025-03-07, the day after `uv_build` was introduced in [uv#11446](https://github.com/astral-sh/uv/pull/11446)) states: "I somehow missed running an actual integration test of the PEP 517 API in CI and the python shim was using the old uv CLI interface still."
- **Release smoke tests.** Every `uv_build` sdist and wheel is installed and runs `uv-build --help` and `python -m uv_build --help`. This includes a native Windows arm64 job ([build-release-binaries.yml L104, L434, L471](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/build-release-binaries.yml#L104)). The `uv` wheels run `uv --help`, `uvx --help`.
- **The subset's own dependency graph is checked.**
  - [check-lint.yml L185–187](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/check-lint.yml#L185-L187) runs `cargo deny --manifest-path crates/uv-build/Cargo.toml --all-features check bans`.
  - [`crates/uv-build/deny.toml`](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/crates/uv-build/deny.toml) bans `rustls`, `openssl`, `reqwest` ("The build backend does not need network access") and `schemars`.
  - uv#11446: "It is part of the workspace, but has much less dependencies for its own binary. We're using cargo deny to enforce that the network stack is not part of the dependencies."
- **Change detection.** The `plan` job flags `crates/uv-build/` changes to trigger release builds on pull requests ([plan.yml](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/plan.yml)).
- **Product relationship.** "The `uv` executable also includes a copy of the build backend, which will be used during builds performed by uv … if its version is compatible … Other build frontends … will always use the `uv_build` package" ([build-backend.md, Bundled build backend](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/docs/concepts/build-backend.md)). `uv_build` "follows the same versioning policy as uv"; `crates/uv-build/Cargo.toml` is at 0.13.1, like uv.

### 2.3 Codex and `codex-app-server` (commit `1ffa95ee`, 2026-10-14)

- **CI split.** Per the [workflows README](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/README.md):
  - Pull requests run Bazel `test //...` ([bazel.yml](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/bazel.yml)).
  - The full Cargo `nextest` matrix runs after merge on `main` (`rust-ci-full.yml`), built with `cargo nextest archive` at the workspace root, without `-p` ([rust-ci-full-nextest-platform.yml L168–178](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/rust-ci-full-nextest-platform.yml#L168-L178)).
- **Subset tests run the subset binary from the test build.**
  - The harness defaults to `codex_utils_cargo_bin::cargo_bin("codex-app-server")` ([test_app_server.rs L2100–2104](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/codex-rs/app-server/tests/common/test_app_server.rs#L2100-L2104)).
  - The suite is one test binary (`tests/all.rs`) aggregating 173 `.rs` files under `tests/suite`.
- **The superset tests only its own entry point.** [`codex-rs/cli/tests/app_server.rs`](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/codex-rs/cli/tests/app_server.rs) has 4 tests that run `codex app-server …` (strict config, JSON log events). The app-server suite is not re-run through `codex`.
- **The release builds the subset separately.**
  - The `build` matrix has `bundle: [primary, app-server]`. The `app-server` rows build `binaries: "codex-app-server codex-code-mode-host"` with `cargo build --target … --release --bin …` ([rust-release.yml L93–145, L253–269](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/rust-release.yml#L253-L269)).
  - That separate invocation resolves features for that bin selection only.
  - The release then verifies code signatures (`codesign --verify`, `lipo -verify_arch`) but does not execute the app-server.
- **Why separate.** [codex#19447](https://github.com/openai/codex/pull/19447): "building it by tacking another `--bin` onto the existing release `cargo build` invocations would lengthen those jobs. This change keeps `codex-app-server` on its own release bundle so it can build in parallel." Its verification section lists YAML parsing and matrix review only.

### 2.4 Deno and `denort` (commit `a18ce337`, 2026-10-06)

- **One build for both.** `const packagesToBuild = ["deno", "denort", "test_server"]`, then `cargo build --release --locked -p deno -p denort -p test_server --bin deno --bin denort --bin test_server` ([ci.ts L995–1000](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/.github/workflows/ci.ts#L995-L1000)). The only post-build smoke check runs `deno eval "console.log(1+2)"`, not `denort` (L1212).
- **The subset artefact is exercised end to end.**
  - Test jobs download the `deno` artefact. They also download the `denort` artefact for the `integration` and `specs` crates (L1426–1430).
  - The harness sets `DENORT_BIN` to the `denort` next to `deno` ([builders.rs L225–229](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/tests/util/lib/builders.rs#L225-L229)).
  - `deno compile` uses `DENORT_BIN` in development instead of downloading ([binary.rs L1796–1807](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/cli/standalone/binary.rs#L1796-L1807)).
  - `tests/specs/compile` has 73 cases.
- **Tests run per test package.** They run as `cargo test -p <test package> --test <crate>` against the prebuilt artefacts (L1496–1511).
  - `denort`'s own `[[test]] integration` is a no-op whose comment reads "this file exists to cause the executable to be built when running cargo test" ([integration_tests_runner.rs](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/cli/rt/integration_tests_runner.rs)).
  - Test jobs are skipped on tag builds (`step.if(isNotTag…)`, L1416).
- **Variant check.** A `cargo check --locked -p deno -p denort --no-default-features --features quickjs` "keeps compiling for the deno + denort binaries" (L1205).

### 2.5 Zed and the remote server (commit `3e145061`, 2026-10-15)

- Tests run with `cargo nextest run --workspace` ([run_tests.yml L363](https://github.com/zed-industries/zed/blob/3e145061d861ed32bb6a08dad7b03f2b5e455ad8/.github/workflows/run_tests.yml#L363)).
- [`remote_editing_tests.rs`](https://github.com/zed-industries/zed/blob/3e145061d861ed32bb6a08dad7b03f2b5e455ad8/crates/remote_server/src/remote_editing_tests.rs) runs a `HeadlessProject` in process with `FakeFs` and `FakeHttpClient`.
- Three jobs `check_remote_server_*` run `cargo check --locked --release --package remote_server --target …` for `x86_64-unknown-linux-musl`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` (L837–940).
- **Unverified:** whether any CI job runs the built remote server binary. None was found in `run_tests.yml`.

---

## 3. B. Releasing products together

| | Build gate | Publication | Re-run after a partial publish |
|---|---|---|---|
| **uv** | `build-release-binaries` builds `uv` and `uv_build` in the same per-platform jobs. Signing (`assemble-release-artifacts`) needs all builds plus a two-person `release-gate` approval. One failure stops everything | `publish-docker` and `publish-pypi` run in parallel. Inside `publish-pypi`, `pypi-publish-uv` and `pypi-publish-uv-build` are two parallel jobs. `publish-github` needs Docker and PyPI to succeed. `publish-crates` runs after PyPI and is not a dependency of the GitHub release | PyPI: "you can retry the exact same command; existing identical files will be ignored" ([package.md L168–175](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/docs/guides/package.md#L168-L175)). crates.io: "it's not critical to the release and isn't idempotent on retry" ([release.yml L377](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/release.yml#L377)) |
| **Codex** | `release` needs `build` (all `primary` and `app-server` legs), `finalize-macos`, `build-windows`, npm staging. Bazel legs are `continue-on-error` ("advisory while Cargo remains the release path") | The `release` job creates the GitHub release as a **draft** if absent ("Look up this tag, including drafts"). It uploads every file with `gh release upload --clobber`, then sets `--draft=false` (L1762–1790). R2, DotSlash, npm and winget run after. A per-tag `concurrency` lock means "out-of-order builds cannot roll back GitHub, npm, R2" | GitHub upload is idempotent by design (`--clobber` on an existing draft). npm: output matching "previously published / cannot publish over / version already exists" is skipped ([L2024](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/rust-release.yml#L2024)). `codex-app-server` is not on npm |
| **Deno** | Per-platform build jobs; there is no cross-platform gate before upload | On a tag, **each build job** syncs its zips to `s3://dl-deno-land/release/<tag>/` and uploads to a GitHub release with `draft: true` ([ci.ts L1228–1310](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/.github/workflows/ci.ts#L1228-L1310)). A later manual step publishes the draft. `deno` and `denort` of one platform come from the same job, so they upload together | **Unverified:** behaviour of a re-run on an existing draft. `aws s3 sync` re-uploads changed files |
| **Browserless** (OCI) | Per image × architecture matrix | `images-merge` runs "even when some images-build legs failed … `!cancelled()` lets the good images publish anyway"; each image refuses to publish unless it has all its own architecture digests ([docker-publish.yml L332–343](https://github.com/browserless/browserless/blob/24b29b9b2ced74ccfffdd95367093bde5cc00278/.github/workflows/docker-publish.yml#L332-L343)) | A failed image keeps its previous `latest` |

Reported consequences of partial publication:

- uv: "There was an authentication issue … the extended Docker images … were not published to DockerHub for 0.8.12. Pinned images like `docker.io/astral/uv:0.8.12-alpine` will be missing" ([uv#15367](https://github.com/astral-sh/uv/issues/15367)).
- uv: `0.8.10-bookworm` missing while `0.8.9-bookworm` existed ([uv#15262](https://github.com/astral-sh/uv/issues/15262)).
- uv ordered crates.io publication after PyPI "for consistency/idempotency/recovery purposes", because its publish workflows had been scheduled in parallel and "race to win"; the PyPI and Docker jobs still run in parallel ([uv#16987](https://github.com/astral-sh/uv/issues/16987), [uv#16989](https://github.com/astral-sh/uv/pull/16989)).

---

## 4. C. Changelogs and release notes

- **uv.**
  - One `CHANGELOG.md` (older series in `changelogs/0.x.md`), generated by `rooster` from PR labels. Sections: Breaking changes, Enhancements, Preview features, Configuration, Performance, Bug fixes, Rust API, Documentation, Other changes ([pyproject.toml `[tool.rooster]`](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/pyproject.toml)).
  - The repository has an `area:build-backend` label, but no section maps to it. `uv_build` entries sit in the thematic sections. In 0.5–0.7 their titles were prefixed "Build backend:"; later they name `uv_build` in the text.
  - Releases that move the version add a prose line for `uv_build` users. For example 0.13.0: "If your `[build-system]` table includes an upper bound on `uv_build`, update it to allow `uv_build` 0.13".
  - The GitHub release (42 assets for 0.13.1) carries only `uv`; `uv_build` is on PyPI only.
- **Codex.**
  - One GitHub release per tag for all binaries.
  - The notes are "the commit message for the commit the tag points at", capped at 124,998 characters ([rust-release.yml L1641–1659](https://github.com/openai/codex/blob/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb/.github/workflows/rust-release.yml#L1641-L1659)).
  - Sections: New Features, Bug Fixes, Documentation, Chores, Changelog. App-server changes appear inline ("Add app-server MCP event streaming"), with no per-product section (checked 0.150.0, 0.160.0, 0.162.0, 0.162.1).
- **Deno.**
  - One [`Releases.md`](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/Releases.md); the draft release body is its latest section (`tools/release/05_create_release_notes.ts`).
  - Entries are conventional-commit PR titles with scopes, e.g. `fix(desktop): correct libdenort unpack path`, `perf(compile): remove swc from denort (#27721)`.
- **Crawlee (lerna).**
  - `lerna.json` uses fixed mode (one `version`, 3.18.1) with `"conventionalCommits": true` and ignores `**/test/**` and `**/*.md`.
  - Lerna attributes commits to a package's CHANGELOG by **path**: `generator.commits({ path: pkg.location })` for fixed and independent modes ([update-changelog.ts](https://github.com/lerna/lerna/blob/b00fa0321a42947bb4113bc7906021f04131d0db/libs/core/src/lib/conventional-commits/update-changelog.ts)).
  - The scope (`**playwright:**`) is shown but does not drive attribution.
  - A package with no changed files gets "**Note:** Version bump only for package …". The `crawlee` meta-package, which re-exports the others, shows exactly that for 3.18.1, while the root CHANGELOG lists fixes in `core` and `playwright` for the same version ([packages/crawlee/CHANGELOG.md](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/packages/crawlee/CHANGELOG.md), [root CHANGELOG.md](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/CHANGELOG.md)).

| Attribution method | Used by |
|---|---|
| PR labels → sections (product not a section) | uv |
| Commit message of the tag commit (hand-written) | Codex |
| Conventional-commit scope in the title (displayed, not routed) | Deno, Crawlee |
| Commit paths → per-package file | Crawlee (lerna) |

---

## 5. D. OCI images for product families

| Family | Repositories | Tag scheme | Counts (2026-10-15) |
|---|---|---|---|
| **Playwright** | `mcr.microsoft.com/playwright`, `/playwright/python`, `/playwright/java`, `/playwright/dotnet` | `vX.Y.Z`, `vX.Y.Z-{noble,jammy,resolute}`, each also `-amd64`/`-arm64`; canary tags; old `focal`/`jammy`/`latest`/`next` still listed | 18,606 / 994 / 1,396 / 916 tags ([MCR tag list API](https://mcr.microsoft.com/v2/playwright/tags/list)) |
| **Apify (Crawlee)** | 21 `apify/actor-*` repositories on Docker Hub: `actor-node`, `actor-node-playwright{,-chrome,-firefox,-webkit,-camoufox}`, `actor-node-puppeteer-chrome`, `actor-python`, `actor-python-playwright{,-chrome,-firefox,-webkit,-camoufox}`, `actor-python-selenium` and others | `<node major>-<library version>[-slim]`, plus `<node major>`, `latest`, `beta`; Python images `<python>` | `actor-node-playwright-chrome` 723 tags, `actor-node` 58, `actor-python` 19 |
| **Browserless** | `ghcr.io/browserless/{chromium,chrome,edge,firefox,webkit,webkit-heavy,multi,multi-heavy}` (+ `base`); `chrome` and `edge` are amd64-only | `latest` from `main`, `v2.x.y` from `v2.*` tags | 8 images |
| **uv** | `ghcr.io/astral-sh/uv` (and Docker Hub `astral/uv`) | Distroless `latest`, `X.Y.Z`, `X.Y`; 44 derived-base tags (`alpine`, `debian-slim`, `trixie`, `python3.x-{alpine,trixie,trixie-slim}`, `*-dhi`), each also as `X.Y.Z-<base>` and `X.Y-<base>` | No `uv_build` image |

Sources: [Playwright Docker docs](https://playwright.dev/docs/docker), [Playwright v1.47.0 notes](https://github.com/microsoft/playwright/releases/tag/v1.47.0), [apify-actor-docker README](https://github.com/apify/apify-actor-docker/blob/26f9e008cd54547ae219c04176b39ca911e2bb02/README.md), [Crawlee Docker images guide](https://crawlee.dev/js/docs/guides/docker-images), [Docker Hub API](https://hub.docker.com/v2/namespaces/apify/repositories), [Browserless docker-publish.yml](https://github.com/browserless/browserless/blob/24b29b9b2ced74ccfffdd95367093bde5cc00278/.github/workflows/docker-publish.yml), [uv Docker guide](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/docs/guides/integration/docker.md).

Leader practice and user pains:

- **Pin and match.**
  - Playwright: "It is recommended to always pin your Docker image to a specific version"; a mismatch leaves Playwright "unable to locate browser executables"; "The Playwright package/dependency is not included in the image".
  - Crawlee: "pinning both the Node.js version **and** the automation library version"; "Match the automation library version in your `package.json` with the version in your Docker image tag".
- **`latest`.**
  - Playwright v1.47.0: "The `:latest`/`:focal`/`:jammy` tag … is no longer being published. Pin to a specific version". Users were still pulling a stale `latest` ([playwright#32676](https://github.com/microsoft/playwright/issues/32676), [#32483](https://github.com/microsoft/playwright/issues/32483)).
  - A maintainer reply: "We never documented `:latest`".
- **Wrong or inconsistent image.**
  - Apify: "Chromium not found in actor-node-playwright-chrome", 26 comments ([apify-actor-docker#87](https://github.com/apify/apify-actor-docker/issues/87)).
  - Apify: images "ship `/pw-browsers/chromium-1223` … regardless of the Playwright version named in the tag" ([#291](https://github.com/apify/apify-actor-docker/issues/291), 2026-05).
  - Browserless: "What is the difference between the `chrome-stable` and `puppeteer-x` tags … Which image should I pull?" ([browserless#1709](https://github.com/browserless/browserless/issues/1709)).
  - playwright-python: users expected the package inside the image ([playwright-python#1978](https://github.com/microsoft/playwright-python/issues/1978)).
- **Missing tags after a partial publish:** uv#15367 and uv#15262 (§3); a documented tag not matching the published one ([uv#7029](https://github.com/astral-sh/uv/issues/7029)).
- **Too many images or tags.** No complaint about the *number* of images or tags was found in these four trackers. **Unverified absence:** a keyword search only.

---

## 6. E. PyPI storage

- **Defaults.**
  - [docs.pypi.org storage limits](https://docs.pypi.org/project-management/storage-limits/): "The current default limits are 100.0 MB for individual files and 10.0 GB for the entire project."
  - In Warehouse source they are binary units: `MAX_FILESIZE = 100 * ONE_MIB`, `MAX_PROJECT_SIZE = 10 * ONE_GIB` ([warehouse/constants.py](https://github.com/pypi/warehouse/blob/f3998ac958bb1a0054c7b5ed112e10292d138ba2/warehouse/constants.py)).
  - "Yanking a release or file does not free up storage space."
- **Requesting more.**
  - File limit: first upload "at least one release for the project that's under the limit", then file an issue with the project link, release size, indexes and a reason.
  - Project limit: "Remove unneeded releases or files first … If that is not possible, we can sometimes increase your limit."
  - The page states no cost and no turnaround time. No fee appears in the request template or in the closed requests read for this note.
- **Turnaround.**
  - Measured from [pypi/support](https://github.com/pypi/support) `limit request` issues on 2026-10-15: 114 "Project Limit Request" issues opened in 2026 and closed had a median of **14 days** (25th–75th percentile 8–23).
  - 27 project-limit requests were open, the oldest from 2025-11-18. "Closed" does not always mean granted.
- **DuckDB, the typical large grant.**
  - [pypi/support#1129](https://github.com/pypi/support/issues/1129), opened 2021-06-01: a reviewer questioned per-merge dev releases ("not reasonable to use PyPI as dump for artifacts that are generated on each commit"). The project moved to automated deletion of pre-releases, and reported releases of about 650 MB each. An admin set the limit to **30 GB** on 2022-12-21 ("Please continue removing dev releases as much as possible"), about 18.5 months after opening.
  - [pypi/support#6896](https://github.com/pypi/support/issues/6896), opened 2025-07-09: "Project limit of 100 GiB granted" on 2025-09-09, 62 days later.
- **Organizations.**
  - The organization docs mention no storage limit ([organization accounts](https://docs.pypi.org/organization-accounts/), [FAQ](https://docs.pypi.org/organization-accounts/org-acc-faq/): "there is no limit to the number of users").
  - In source, `Organization` has admin-set `upload_limit` and `total_size_limit` columns "for projects in this organization" ([organizations/models.py](https://github.com/pypi/warehouse/blob/f3998ac958bb1a0054c7b5ed112e10292d138ba2/warehouse/organizations/models.py)).
  - A project's effective limit is `max(default, project limit, organization limit)` ([packaging/models.py `upload_limit_size`, `total_size_limit_value`](https://github.com/pypi/warehouse/blob/f3998ac958bb1a0054c7b5ed112e10292d138ba2/warehouse/packaging/models.py)).
  - So an organization limit is a per-project floor raise, not a pooled quota across its projects.
  - Organizations are "free of charge for community projects" and paid for corporate ones.
- **For scale.** One uv 0.13.1 release is 19 files, 326.9 MB in total, largest 20.1 MB; `uv_build` 0.13.1 is 19 files, 26.1 MB (PyPI JSON API). Two products on PyPI are two projects, each with its own 10 GiB.

---

## 7. F. Signing costs that scale with executables

**Apple.**

- **Prerequisite.** Notarization requires Developer ID signing. "You must be the Account Holder of your development team in the Apple Developer Program" to generate the certificate ([Developer ID](https://developer.apple.com/developer-id/)).
- **Price.** The program is "99 USD per membership year" ([What's included](https://developer.apple.com/programs/whats-included/)).
- **Fee.** No per-submission fee is documented. **Unverified absence:** none appears on the notarization pages read.
- **Limits.** [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow), section "Avoid long notarization response times and size limits", lists "Limit notarizations to 75 per day." It also says "Notarization completes for most software within 5 minutes, and for 98 percent of software within 15 minutes."
  - Whether the 75 counts per account, team or key is not documented. Forum threads ask and get no definitive answer ([thread 706413](https://developer.apple.com/forums/thread/706413), [783347](https://developer.apple.com/forums/thread/783347)), so the scope is **unverified**.
- **Batching.** "The notary service accepts disk images (UDIF format), signed flat installer packages, and ZIP archives", so several executables fit in one submission. "Although tickets are created for standalone binaries, it's not currently possible to staple tickets to them."

**Azure Artifact Signing (formerly Trusted Signing).**

- **Quotas.** The [pricing page](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/) lists:
  - Basic: 5,000 signatures/month, "1 of each Certificate Profile type".
  - Premium: 100,000 signatures/month, "10 of each".
  - The page renders prices client-side, so its dollar figures could not be read there.
- **Prices.** The [Azure Retail Prices API](https://prices.azure.com/api/retail/prices?currencyCode=USD&$filter=serviceName%20eq%20%27Trusted%20Signing%27) (service still named "Trusted Signing", effective 2024-06-01, queried 2026-10-15) gives:
  - **Basic Account 9.99 USD/month**
  - **Premium Account 99.99 USD/month**
  - **Signature Overage 0.005 USD per signature**
- **Conditions.**
  - "The pricing isn't calculated on a pro rata basis."
  - "The signing quota encompasses all signing activities across any certificate profile within an Artifact Signing account."
  - "Artifact Signing doesn't support free, trial, or sponsored Azure subscriptions … you must have a paid Azure subscription" ([FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq), updated 2026-10-06).

**What the precedents sign.**

- uv signs and notarizes `uv`, `uvx` and `uv-build` on macOS, and signs `uv.exe`, `uvx.exe`, `uvw.exe` and `uv-build.exe` on Windows with Azure Artifact Signing parameters (endpoint, signing account, certificate profile) ([sign-release-binaries.yml L55–67, L195–215](https://github.com/astral-sh/uv/blob/3ea045bc2a1bede580a108eb6d30abfb575e5d6c/.github/workflows/sign-release-binaries.yml#L55-L67)).
- Deno signs only `deno`:
  - macOS with `rcodesign`, no notarization step found.
  - Windows with `Azure/artifact-signing-action` filtered to `deno.exe`, on `main` pushes as well as tags (`isMainOrTag`) ([ci.ts L794–822, L865–880](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/.github/workflows/ci.ts#L865-L880)).
  - `denort` is not signed: it is the base `deno compile` rewrites.
- Codex signs the app-server bundle like the primary bundle: macOS through Azure Key Vault, Linux via sigstore, Windows signing.

---

## 8. G. GitHub Releases limits

[About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases):

- "Each file included in a release must be under 2 GiB."
- "Up to 1000 release assets may be associated with a single release."
- "There is no limit on the total size of a release, nor bandwidth usage."

For scale:

- Codex's largest release had 318 assets (`rust-v0.162.0-alpha.20`); its latest stable (`rust-v0.162.1`) has 176, of which 35 are app-server files.
- Deno's largest has 56.
- uv 0.13.1 has 42.

---

## 9. H. Cargo feature unification

- **Confirmed.**
  - [Cargo resolver, Features](https://doc.rust-lang.org/cargo/reference/resolver.html#features): "When building multiple packages in a workspace (such as with `--workspace` or multiple `-p` flags), the features of the dependencies of all of those packages are unified. If you have a circumstance where you want to avoid that unification for different workspace members, you will need to build them via separate `cargo` invocations."
  - Also: "Dependencies are resolved with the union of all features enabled on them."
  - Resolver 2 does not unify across "target architectures not currently being built", build-dependencies and proc-macros. "Dev-dependencies do not activate features unless building a Cargo target that needs them (like tests or examples)" ([features](https://doc.rust-lang.org/cargo/reference/features.html#feature-resolver-version-2)). So a test run also brings in dev-dependency features that a release build does not.
  - A `cargo test --workspace` or `cargo nextest run --workspace` therefore compiles shared crates with the union of all members' (and dev-dependencies') features, which can differ from a `cargo build -p <one package>`.
- **`resolver.feature-unification`.**
  - [Unstable features](https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification): "`-Z feature-unification` enables the `resolver.feature-unification` configuration option … If the `-Z feature-unification` unstable flag is not enabled, then the `resolver.feature-unification` configuration will be ignored."
  - Values: `selected` (default: "Merge dependency features from all packages specified for the current build"), `workspace` ("across all workspace members, regardless of which packages are specified"), `package` ("considered on a package-by-package basis, preferring duplicate builds").
  - Tracking issue [cargo#14774](https://github.com/rust-lang/cargo/issues/14774): **open**. Both modes are implemented ([#15157](https://github.com/rust-lang/cargo/pull/15157) 2025-02, [#15684](https://github.com/rust-lang/cargo/pull/15684) 2025-07). Unresolved items: option naming, and `cargo tree` for `package`.
  - Recent activity: more `workspace` tests and fixes merged in September 2026 ([#17466](https://github.com/rust-lang/cargo/pull/17466), [#17469](https://github.com/rust-lang/cargo/pull/17469)), and a Zulip thread on partially stabilizing `workspace` (2026-09-17).
  - No open stabilization PR was found on 2026-10-15. **No stable Cargo has it.**
- **The underlying complaint.** [cargo#4463](https://github.com/rust-lang/cargo/issues/4463) "Feature selection in workspace depends on the set of packages compiled" has been open since 2017-09-03 with 48 comments. Maintainer notes: "this issue is still open regarding having features change based on what is being built simultaneously"; "The `cargo hack` plugin will automatically expand `cargo check --workspace` … to `cargo check -p …`"; "`resolver.feature-unification = "package"` can control this behavior".
- **Tooling.**
  - [cargo-hack](https://github.com/taiki-e/cargo-hack): with `--workspace`, "Unlike cargo, it does not compile all members at once", which is per-package, like separate invocations. It also has `--each-feature` and `--feature-powerset`.
  - [cargo-hakari](https://docs.rs/cargo-hakari/latest/cargo_hakari/about/index.html) generates a "workspace-hack" crate so dependencies "are always built with the same feature set, regardless of which workspace packages are being built".
- **What projects do.** No project was found that *asserts* two binaries resolve shared crates with identical features. Observed practices:
  - **Separate invocation per shipped binary:** uv's `uv_build` (maturin `-m crates/uv-build/Cargo.toml`) and Codex's `app-server` release bundle. This is the shipped artefact's own feature set, but in Codex's case not the one tested.
  - **One invocation for both shipped binaries:** Deno (`-p deno -p denort`), so the shipped subset carries the superset's unified features.
  - **Per-package compile check:** Zed (`cargo check -p remote_server`), Deno (`cargo check -p deno -p denort --features quickjs`).
  - **Per-manifest dependency ban:** uv (`cargo deny --manifest-path crates/uv-build/Cargo.toml check bans`).
  - **Feature-unifying hack crate:** Zed used `workspace-hack` (hakari), removed in [zed#40216](https://github.com/zed-industries/zed/pull/40216) (2025-10-17). Its maintainers said removal grows the build "from ~1700 to ~2000 crates … mainly a problem when … running tests across the normal and remote binaries (which is where feature-unification nets us the most sharing)". They expect the savings back "when rust-lang/cargo#14774 … is finished".

---

## 10. I. Was the subset shipped from the start?

| Subset | Superset's first release | Subset's first release | Introduced by |
|---|---|---|---|
| `uv_build` (PyPI) | `uv` 0.0.5, 2024-02-15 | 0.5.30rc1 uploaded 2025-02-12; first regular release 0.6.3, 2025-03-06 | [uv#11446](https://github.com/astral-sh/uv/pull/11446) "A minimal build backend for uv: uv_build", merged 2025-03-06; integration test [uv#12058](https://github.com/astral-sh/uv/pull/12058) the next day |
| `codex-app-server` (GitHub asset) | Oldest Rust release in the GitHub list: 2025-04-30 | `rust-v0.126.0-alpha.1`, 2026-04-24; first stable `rust-v0.128.0`, 2026-04-30 (19 app-server files) | [codex#19447](https://github.com/openai/codex/pull/19447) "ci: publish codex-app-server release artifacts", merged 2026-04-24 |
| `denort` (GitHub asset) | Deno v0.1.0, 2018-08-23 | v1.40.5, 2024-02-15 | (PR not traced) |

Sources: PyPI JSON API for `uv` and `uv-build`; GitHub Releases API for `openai/codex` (1,226 releases) and `denoland/deno` (390 releases), read on 2026-10-15.

---

## 11. Implications for Goodfetch (facts only, no decision)

1. A workspace-wide test run tests Collie's crates with Foxhound's features unified in. A standalone `cargo build -p` of Collie can resolve differently, and stable Cargo has no setting to prevent that (§9).
2. In every precedent the subset's own binary gets far less testing than its logic. uv shipped a broken subset once before adding the dedicated job (§2.2).
3. All three precedents gate publication on every product's build. None makes multi-channel publication atomic; they rely on idempotent re-runs (draft plus `--clobber`, skip already-published) instead (§3).
4. Path-based changelog attribution under-reports for a product that contains another (the Crawlee meta-package, §4). Label- or commit-message-based notes do not split per product unless a section is defined.
5. Each extra product multiplies image tags (one more image per browser variant in Apify and Browserless). Users' main pain is version mismatch and stale floating tags, not tag count (§5).
6. PyPI quotas are per project, and organizations do not pool them. A second PyPI project gets its own 10 GiB; increases are free and take a median of about two weeks (§6).
7. Apple's 75/day applies to notarizations, and one ZIP can hold several executables. Azure's Basic quota (5,000/month) is counted per signature across profiles; each signed executable is one signature (§7).

---

## 12. Pinned sources

| Project | Commit inspected | Date |
|---|---|---|
| astral-sh/uv | [`3ea045bc`](https://github.com/astral-sh/uv/tree/3ea045bc2a1bede580a108eb6d30abfb575e5d6c) | 2026-10-14 |
| openai/codex | [`1ffa95ee`](https://github.com/openai/codex/tree/1ffa95ee8a02ec68f6ccd758998bcb7d4b32bcdb) | 2026-10-14 |
| denoland/deno | [`a18ce337`](https://github.com/denoland/deno/tree/a18ce33715e30cd2b0d99c7e322ef11a65490e4d) | 2026-10-06 |
| zed-industries/zed | [`3e145061`](https://github.com/zed-industries/zed/tree/3e145061d861ed32bb6a08dad7b03f2b5e455ad8) | 2026-10-15 |
| browserless/browserless | [`24b29b9b`](https://github.com/browserless/browserless/tree/24b29b9b2ced74ccfffdd95367093bde5cc00278) | 2026-10-14 |
| apify/apify-actor-docker | [`26f9e008`](https://github.com/apify/apify-actor-docker/tree/26f9e008cd54547ae219c04176b39ca911e2bb02) | 2026-10-14 |
| apify/crawlee | [`8cb2eb61`](https://github.com/apify/crawlee/tree/8cb2eb61a0356a96205c08e02ed49cadc97081e5) | 2026-10-14 |
| lerna/lerna | [`b00fa032`](https://github.com/lerna/lerna/tree/b00fa0321a42947bb4113bc7906021f04131d0db) (v10.1.1) | 2026-10-12 |
| pypi/warehouse | [`f3998ac9`](https://github.com/pypi/warehouse/tree/f3998ac958bb1a0054c7b5ed112e10292d138ba2) | 2026-10-15 |

**Unverified or not checked:**

- The scope of Apple's 75/day (account, team or key).
- Absence of an Apple notarization fee (absence in the docs read).
- Whether Zed runs the built remote server binary anywhere in CI.
- Deno's re-run behaviour on an existing draft release.
- Absence of "too many tags" complaints (keyword search only).
- The PR that introduced `denort` assets.
