# Rust test tooling in 2026

Research of **October 13, 2026** for ticket [#263](https://github.com/Jbmanllr/automan/issues/263). Question: which maintained, widely adopted Rust tools should Goodfetch use for each kind of test, and how do comparable Rust projects keep their suites fast? For each tool: current version and release cadence, maintenance and adoption, what it does and its limits, support on static musl Linux, macOS and Windows, and its cost in CI time.

**Sources.** Official docs read from their source repositories (`gh api`, 2026-10-13), release notes and changelogs, crates.io metadata (versions, licences, MSRVs, downloads), GitHub repository metadata and issues, and the CI configuration of ten comparable projects, read at a pinned commit (§12). Suite times come from the GitHub Actions API of public runs (§12.3). Rust's own release notes and project goals for `cargo test` and the toolchain.

**Conventions.** "Releases in 12 months" counts crates.io versions published since 2026-10-13 minus one year. Downloads are crates.io totals; tools mostly installed as prebuilt binaries (nextest, cargo-mutants, cargo-llvm-cov) are undercounted there. Anything not confirmed in a primary source is in §14. Like everything in `rnd/`, this note informs a decision and makes none. "Fit for Goodfetch" paragraphs are analysis, kept apart from the cited facts.

---

## 1. Summary

**Every item already decided (STYLE.md, ADR 0032) still fits.** What changed or needs pinning:

- **cargo-nextest 0.9.148** (2026-10-07; 48 releases in 12 months) stays the runner. Pin `nextest-version = "0.9.148"` in `.config/nextest.toml`: before 0.9.148 a stress run with fail-fast off exited 0 when only an earlier iteration failed. Nextest still **does not run doctests** ([#16](https://github.com/nextest-rs/nextest/issues/16), open since 2022, 124 reactions), so `goodfetch`'s doctests need a separate `cargo test --doc` step, as Tokio, Ruff, Zed and Turso do.
- **Retries are off by default (`retries = 0`), but nothing locks them off.** `--retries` and `NEXTEST_RETRIES` override the repository config. Since 0.9.131, `flaky-result = "fail"` turns a test that passes only on retry into a failure. That is how STYLE.md's "never retried silently" rule can be enforced. Vector is the counter-example: `retries = 3` in its default profile.
- **proptest 1.12.0** (2026-10-10, MSRV 1.88) and **proptest-state-machine 0.9.0** are current. Failures are persisted as **seeds**, not values. A new Hypothesis-based option, **hegeltest**, is in beta with breaking changes allowed (§4).
- **insta 1.49.0**: on CI (`CI` set) it writes nothing and fails a mismatched snapshot. `cargo insta test --unreferenced reject --test-runner nextest` also fails on orphaned snapshot files; Ruff runs exactly that in CI.
- **cargo-fuzz 0.13.2** needs nightly and AddressSanitizer, which Rust supports only on `*-linux-gnu`, Apple and FreeBSD targets, not musl. Basic Windows support exists since 0.13.0. Fuzzing therefore runs on Linux glibc, which loses nothing for platform-neutral decoders.
- **Miri** runs under nextest (`cargo miri nextest run`, used by Tokio), but nextest archives do not work under Miri. **cargo-hack 0.6.45** is current.
- **One integration-test binary per crate is right against one binary per file, but not the end state for a very large crate.** uv measured this in 2026 (§12.2). Merging 41 targets into one cut its benchmark from about 35 s to 9–14 s. Two years later, at 2,958 tests, one 96.5 MiB binary made every local test edit relink the whole suite (7.5 s). Twelve command-area binaries brought that edit to 2.6 s, against 2.58 s for one binary per file, at 591 MiB of disk instead of 1.7 GiB.

**What the other bullets add:**

| Need | Leading tool (version, date) | Main limit for Goodfetch |
|---|---|---|
| Mutation testing | cargo-mutants 27.1.0 (2026-06-02) | Runs the suite once per mutant; ignores `cfg(target_os)`, so functions for other platforms show as missed |
| Coverage | cargo-llvm-cov 0.9.1 (2026-09-06) | Separate instrumented build; branch coverage and doctests are nightly-only; `aarch64-pc-windows-msvc` does not work by default |
| CLI tests | assert_cmd 2.2.2, insta-cmd 0.7.0, snapbox 1.2.2 / trycmd 1.2.1 | insta-cmd: 1 release in 2 years. trycmd/snapbox have their own snapshot format, separate from insta |
| Containers | testcontainers 0.28.0 (2026-08-06) | Docker only on GitHub's Linux runners; cleanup is `Drop`-based with no resource reaper ([#577](https://github.com/testcontainers/testcontainers-rs/issues/577) open) |
| HTTP mocks | httpmock 0.8.3, mockito 1.7.2, wiremock 0.6.5 | wiremock: no release or commit since 2025-08-24; only httpmock records and replays real exchanges |

**Build speed facts:**

- Rust 1.90 (2025-09-18) links with LLD by default on `x86_64-unknown-linux-gnu`. On ripgrep, linking for an incremental rebuild became 7× faster, end to end 40% faster.
- Cargo 1.99 (2026-10-01) turns incremental compilation off by default when `CI` is set.
- sccache cannot cache crates that invoke the linker, and every test binary does.
- `Swatinem/rust-cache` caches dependencies only. A repository's Actions cache is 10 GB by default.
- A public-repository Linux runner has 4 vCPUs, 16 GB RAM and a 14 GB SSD; an arm64 macOS runner has 3 vCPUs and 7 GB RAM.

**Comparable projects:**

- **nextest** is used by uv, Ruff, Tokio, Zed, Turso, Materialize and Vector. Cargo and Nushell use `cargo test`; Deno uses a mix with its own file-driven runner.
- **Suite times** for the main Rust test job are 2–5 min on 16-vCPU paid runners (uv, Ruff) and 4–9 min on GitHub's free runners (Tokio). Cargo's single-binary suite takes 6–24 min depending on the OS (§12.3).

---

## 2. Runner: cargo-nextest against `cargo test`

### 2.1 Facts

**Version, cadence, adoption.** 0.9.148, released 2026-10-07 ([changelog](https://nexte.st/changelog/)). 48 crates.io releases in 12 months; 13.2 M crates.io downloads; 3,340 GitHub stars; licence Apache-2.0 OR MIT ([crates.io](https://crates.io/crates/cargo-nextest), [repo](https://github.com/nextest-rs/nextest)). The CLI and machine-readable output follow semver ([stability](https://nexte.st/docs/stability/)). Building nextest itself needs Rust 1.91; it can run tests built by much older toolchains (Rust 1.41 "known to work").

**Platforms.** Prebuilt binaries cover Linux x86_64 and aarch64, each as glibc and as a **statically linked musl** build. They also cover a universal macOS binary, Windows x86_64, aarch64 and i686, FreeBSD and illumos ([pre-built binaries](https://nexte.st/docs/installation/pre-built-binaries/)). Installing from `taiki-e/install-action` took under 0.2 min in every CI run checked in §12.3.

**Execution model.** One process per test, "now, and will always be" the default. The [design page](https://nexte.st/docs/design/why-process-per-test/) lists the costs: in-memory state such as semaphores, caches or shared servers can no longer be shared between tests. Process creation is "quite slow on Windows", and can be slow on macOS when anti-malware scanning interferes. In return, timeouts, retries, cancellation and output capture work per test, and one segfault does not take other tests with it.

**Speed against `cargo test`.** Nextest's [benchmarks](https://nexte.st/docs/benchmarks/) (Rust 1.66, 16-core Ryzen, test run only, build excluded) show 1.37× to 3.38× faster runs: Tokio 24.27 s → 11.60 s, omicron 444 s → 202 s. cargo-mutants reports the opposite on some trees. Nextest lets straggling tests run to completion after a failure, so a tree with fast unit tests and slow integration tests can be slower under nextest ([cargo-mutants: nextest](https://mutants.rs/nextest.html)). `--max-fail=1:immediate` (since 0.9.111) terminates the running tests instead ([running](https://nexte.st/docs/running/)).

**Features the ticket asks about:**

- **Stress runs for flake detection** (0.9.103, 2025-08-24). `--stress-count=N|infinite` or `--stress-duration=24h` run the selected tests in a loop. Iterations are sequential: the same test is not run in parallel with itself. In JUnit, each iteration's `<testsuite>` gets `@stress-N` ([stress tests](https://nexte.st/docs/features/stress-tests/)). **0.9.148 fixed two bugs**: a stress run with fail-fast off exited 0 if only an earlier iteration failed, and a very short `--stress-duration` could run nothing ([changelog 0.9.148](https://nexte.st/changelog/)).
- **Test groups and serial tests** (0.9.48). `[test-groups] serial = { max-threads = 1 }` plus a per-test override `test-group = "serial"` acts as a mutex or semaphore across processes. In-process mutexes such as `serial_test` do not work across processes. The docs recommend port 0 or Unix sockets over serializing port users. Since 0.9.133, `group(name)` selects a group's tests ([test groups](https://nexte.st/docs/configuration/test-groups/)). `threads-required` reserves several slots for a heavy test.
- **Partitioning and sharding.** Three modes ([partitioning](https://nexte.st/docs/ci-features/partitioning/)):
  - `--partition slice:m/n` (0.9.127, 2026-02-13): round-robin over all tests after filtering.
  - `--partition hash:m/n`: deterministic by binary and test name, so adding tests never moves others.
  - `count:`: deprecated.
- **Archives.** `cargo nextest archive --archive-file x.tar.zst` packs test binaries, their dynamic libraries, non-test binaries used by integration tests, build-script `OUT_DIR`s one level deep, and Cargo metadata ([archiving](https://nexte.st/docs/ci-features/archiving/)). The target machine needs nextest and the same source checkout, but not Cargo; use the same nextest version on both. Source code is not included. `-E` filters which binaries go in (0.9.109), and `archive.include` adds extra files. Archives are "not supported under Miri" ([Miri](https://nexte.st/docs/integrations/miri/)).
- **JUnit.** `[profile.ci.junit] path = "junit.xml"` writes `target/nextest/ci/junit.xml` in the Jenkins format, with one `<testsuite>` per binary ([JUnit](https://nexte.st/docs/machine-readable/junit/)). Flaky tests get `<flakyFailure>`/`<flakyError>`, and reruns that kept failing get `<rerunFailure>`.
- **Setup scripts.** Still **experimental**: enabled with `experimental = ["setup-scripts"]`, tracking issue [#978](https://github.com/nextest-rs/nextest/issues/978) open ([setup scripts](https://nexte.st/docs/configuration/setup-scripts/)).
  - Scripts run serially before the matching tests, scoped by filterset and platform, and can export variables to tests through `$NEXTEST_ENV`.
  - A failing script ends the run. Since 0.9.148, a timed-out script always fails the run.
  - There are no teardown scripts ([#683](https://github.com/nextest-rs/nextest/issues/683) open).
  - Wrapper scripts (0.9.98) are also experimental.
- **Retries and how to forbid them.** All from [retries](https://nexte.st/docs/features/retries/) and the [default config](https://github.com/nextest-rs/nextest/blob/main/nextest-runner/default-config.toml):
  - The default is `retries = 0` and `flaky-result = "pass"`.
  - `--retries N` or `NEXTEST_RETRIES` override the config and disable per-test overrides.
  - Backoff can be fixed or exponential, with jitter.
  - **`flaky-result = "fail"` (0.9.131, 2026-03-18) makes a test that fails and then passes on retry count as failed**. It can be set per test. `--flaky-result` and `NEXTEST_FLAKY_RESULT` override it.
  - No setting stops a command line from turning retries on. `nextest-version = { required = "…" }` makes older nextest versions exit with code 92 ([minimum versions](https://nexte.st/docs/configuration/minimum-versions/)).
- **Timeouts.** The default `slow-timeout` of 60 s only marks a test SLOW. `terminate-after = N` kills it after N periods: SIGTERM to its process group, then SIGKILL after a 10 s grace period, or immediately through a Job Object on Windows ([slow tests](https://nexte.st/docs/features/slow-tests/)). `on-timeout = "pass"` (0.9.115) suits time-boxed fuzz-style tests. `global-timeout` (0.9.100) caps the whole run.
- **Recording and rerun** (0.9.123, experimental). Runs are recorded locally; `cargo nextest run -R latest` reruns what failed or did not run. Recordings can be exported from CI as portable archives or Perfetto traces ([record, replay and rerun](https://nexte.st/docs/features/record-replay-rerun/)).
- **Custom harnesses.** A `harness = false` binary works with nextest if it answers `--list --format terse` and the other libtest options. libtest-mimic (0.4.0, or 0.5.2 and later) and datatest-stable, for data-driven tests from files, already do ([custom test harnesses](https://nexte.st/docs/design/custom-test-harnesses/)).

**`cargo test`'s own limits, stated by the Rust project.** The accepted 2026 goal "Finish the libtest json output experiment" ([rust-project-goals 2026/libtest-json](https://github.com/rust-lang/rust-project-goals/blob/main/src/2026/libtest-json.md)) lists them:

- test binaries do not run in parallel;
- there is no summary across binaries;
- JUnit output is wrong with several binaries;
- a binary that exits unexpectedly is not reported as failed.

`--format json` has been nightly-only since 1.70. libtest has no per-test timeout and no retries ([nextest design appendix](https://nexte.st/docs/design/why-process-per-test/)). On the other side, `cargo test` runs doctests, and Miri under `cargo miri test` can catch data races between tests sharing a process, which Miri under nextest cannot.

**User complaints.** Most-reacted open nextest issues: doctests (#16, +124), machine-readable output for `run` (#20, +13; partly met by the experimental libtest-JSON output), setup and teardown scripts (#683, +9), incremental runs (#174) ([search](https://github.com/nextest-rs/nextest/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)).

### 2.2 Fit for Goodfetch

- **Settings this implies:**
  - `retries = 0` and `flaky-result = "fail"` in the default and `ci` profiles, so a retry turned on from the command line cannot hide a flake;
  - `nextest-version = { required = "0.9.148" }`;
  - a nightly `--stress-count` job over tests suspected of flakiness;
  - quarantine through `#[ignore = "issue #N"]` or a `default-filter` exclusion, with `--run-ignored only` in a non-required job.
- **Browser conformance.** Process-per-test means each test process that needs a Chromium starts or reaches one itself (STYLE.md: "CDP is never mocked").
  - Sharing one browser across tests needs it out of process. Because setup scripts have no teardown, a browser or Proving ground page server started by a setup script would be left running (it would show as leaky). The CI job or `just` recipe would start and stop it around `cargo nextest run`. *(Inference from the setup-scripts docs.)*
  - Alternatively, the conformance suites can be a `harness = false` binary built on libtest-mimic or datatest-stable, which nextest supports.
- **Windows and macOS process cost.** Nextest's own docs name these as the costly platforms. The full matrix of ADR 0032 runs them in the merge queue, not on every pull request.
- **Doctests.** Only `goodfetch` has them (STYLE.md), so one `cargo test --doc -p goodfetch` step covers them.

---

## 3. Snapshots: insta

### 3.1 Facts

- **Version, cadence, adoption.** insta 1.49.0 (2026-10-03); 15 releases in 12 months; 110.9 M downloads; licence Apache-2.0; MSRV 1.66 ([crates.io](https://crates.io/crates/insta), [changelog](https://github.com/mitsuhiko/insta/blob/master/CHANGELOG.md)). 1.49.0 added prebuilt `cargo-insta` binaries for Linux arm64. It also fixed `cargo insta test --all-targets --test-runner nextest`, which no longer runs doctests. Declared in the root manifests of uv, Ruff and Materialize (§12).
- **Redactions** (feature `redactions`) apply to serde-serialized snapshots ([redactions](https://insta.rs/docs/redactions/)):
  - selectors: `.key`, `[index]`, `[]`, `.*`, and `.**` for a deep match;
  - static replacements: `".id" => "[uuid]"`;
  - `dynamic_redaction` callbacks, which can also assert a value's format before replacing it;
  - `sorted_redaction` for sets and maps with no fixed order;
  - `rounded_redaction(n)` for floats.

  Plain-text output uses **filters** instead (feature `filters`, regex replacement). 1.48.0 added `strip_ansi_escape_codes`.
- **Update modes and CI.** `INSTA_UPDATE` defaults to `auto`: `new` locally, which writes `.snap.new` and, for inline snapshots, `.pending-snap` files; `no` on CI, which writes nothing ([crate docs](https://docs.rs/insta/1.49.0/insta/)). CI is detected when `CI` is set to anything but `false`, `0` or empty, or when `TF_BUILD` is set ([`utils.rs`](https://github.com/mitsuhiko/insta/blob/master/insta/src/utils.rs)). Other modes are `always`, `unseen` and `force`. `INSTA_PENDING_DIR` writes pending snapshots to a separate directory; uv uploads that directory as a CI artifact on failure ([uv `test.yml`](https://github.com/astral-sh/uv/blob/59e5912/.github/workflows/test.yml)).
- **Review flow.**
  - `cargo insta review`: interactive; `a` accepts, `r` rejects, `s` skips.
  - `accept`, `reject` and `pending-snapshots --as-json`, for editor integrations.
  - `cargo insta test`: runs tests and forces snapshot assertions to pass so that all changes are collected; `--review` follows with a review, and `--check` only asserts.
  - `--unreferenced=reject|delete|warn` handles snapshot files no test produced.
  - `--test-runner nextest` uses nextest and, unless `--disable-nextest-doctest` is given, runs doctests separately ([cargo-insta docs](https://insta.rs/docs/cli/), [`cli.rs`](https://github.com/mitsuhiko/insta/blob/master/cargo-insta/src/cli.rs)).
  - Settings can live in `.config/insta.yaml`.
- **insta-cmd 0.7.0** (2026-06-14) snapshots a `std::process::Command` with `assert_cmd_snapshot!` and finds the crate's binary with `get_cargo_bin` ([Testing CLIs](https://insta.rs/docs/cmd/)). It is small: 52 stars, 4.4 M downloads, and one release in two years (0.6.0 2024-04-11, then 0.7.0). Its October 2025 commits fixed `env_remove()` in snapshots and raised the MSRV to 1.68. Ruff declares it ([Ruff `Cargo.toml`](https://github.com/astral-sh/ruff/blob/350c634/Cargo.toml)).
- **Platforms.** insta is a pure-Rust library linked into test binaries. `cargo-insta` installs by script on Unix and Windows, or with `cargo install`.
- **CI cost.** No separate build: snapshots are compared inside the normal test run. Ruff's CI step `cargo insta test --all-features --unreferenced reject --test-runner nextest --disable-nextest-doctest` took 2.1 min on a 16-vCPU runner (§12.3).
- **User complaints.** The most-reacted open issues are small: a configurable snapshot root (#416, +8), shorter snapshot file names (#377), and the wrong function name when the macro sits in a helper (#357) ([search](https://github.com/mitsuhiko/insta/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)).

### 3.2 Fit for Goodfetch

- **CI flags.** STYLE.md's "snapshot files are never edited by hand" maps onto `INSTA_UPDATE=no` on CI, which is automatic, plus `cargo insta test --check --unreferenced reject`, which also catches orphaned files.
- **Volatile fields** in JSON-RPC responses (`run_id`, timestamps, durations) are what redactions are for. CLI and log text needs filters.
- **Nextest.** `--test-runner nextest` keeps insta on the same runner as everything else.

---

## 4. Property-based testing: proptest, against quickcheck, bolero, arbitrary (and hegeltest)

### 4.1 Facts

| Crate | Version (date) | Releases 12 mo | Downloads | Licence / MSRV | Model |
|---|---|---|---|---|---|
| proptest | 1.12.0 (2026-10-10) | 4 | 208.7 M | MIT OR Apache-2.0 / 1.88 | Explicit `Strategy` values, integrated shrinking |
| proptest-state-machine | 0.9.0 (2026-10-10) | 4 | 3.5 M | same / 1.88 | Reference model plus system under test |
| quickcheck | 1.1.0 (2026-02-10) | 1 (previous: 1.0.3, 2021-01-15) | 73.1 M | Unlicense OR MIT / 1.85 | One `Arbitrary` generator per type |
| bolero | 0.13.7 (2026-10-03) | 3 | 6.0 M | MIT / 1.68 | One test target run as a property test or under libFuzzer, AFL, honggfuzz or Kani |
| arbitrary | 1.5.0 (2026-10-05) | 1 | 176.1 M | MIT OR Apache-2.0 / 1.71 | Builds structured values from fuzzer bytes |
| hegeltest | 0.49.4 (2026-10-13) | 236 | 0.9 M | MIT / 1.86 | Hypothesis's engine over the Hegel protocol; beta |

Sources: crates.io pages of each crate, read 2026-10-13.

- **proptest.**
  - **Defaults** ([`config.rs`](https://github.com/proptest-rs/proptest/blob/main/proptest/src/test_runner/config.rs)): 256 cases, unlimited shrink iterations, a random seed by default, and environment overrides such as `PROPTEST_CASES`, `PROPTEST_RNG_SEED`, `PROPTEST_RNG_ALGORITHM`, `PROPTEST_MAX_SHRINK_TIME`, `PROPTEST_TIMEOUT` and `PROPTEST_FORK`.
  - **Failure persistence** ([book](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/failure-persistence.md)). Failures are stored by default with `FileFailurePersistence::SourceParallel("proptest-regressions")`. What is stored is the seed that produced the failure, not the value, so a changed strategy still yields a valid input. Failing seeds are replayed before new cases, and the book says to commit the files. `PROPTEST_DISABLE_FAILURE_PERSISTENCE` turns persistence off.
  - **Where the files go** ([`file.rs`](https://github.com/proptest-rs/proptest/blob/main/proptest/src/test_runner/failure_persistence/file.rs)). The lookup walks up from the test's source file to the first directory holding `lib.rs` or `main.rs`. Under `tests/` without one, it prints a warning on every failure and falls back to a file beside the test; if some directory above the package has a `lib.rs`, it can write outside the package ([#233](https://github.com/proptest-rs/proptest/issues/233), open since 2021; fix proposed in [#670](https://github.com/proptest-rs/proptest/pull/670), 2026-10-05).
  - **1.12.0** raised the MSRV to 1.88, moved to `rand` 0.10, and fixed two soundness issues ([changelog](https://github.com/proptest-rs/proptest/blob/main/proptest/CHANGELOG.md)).
  - **Limits** ([limitations](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/limitations.md)): property testing rarely finds single-value edge cases in a large space; generating complex values "can be up to an order of magnitude slower than in QuickCheck" ([vs QuickCheck](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/vs-quickcheck.md)).
  - **Maintenance and adoption.** 150 open issues and pull requests; declared by Deno, Vector, Zed, Cargo and Materialize (§12).
- **State-machine testing** ([book](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/state-machine.md)). You implement `ReferenceStateMachine` (initial state, transitions, apply) and `StateMachineTest` (the system under test, post-conditions, invariants), then run `prop_state_machine!`. Failing sequences shrink to a minimal transition list and persist as seeds like any proptest. The repository's examples are a heap API and an echo server with several clients.
- **quickcheck.** One generator and shrinker per type, so constrained inputs need newtypes (proptest book). Its own [README](https://github.com/BurntSushi/quickcheck) points to proptest for better shrinking. Its runner reads only `QUICKCHECK_TESTS`, `QUICKCHECK_MAX_TESTS`, `QUICKCHECK_GENERATOR_SIZE` and `QUICKCHECK_MIN_TESTS_PASSED` ([`tester.rs`](https://github.com/BurntSushi/quickcheck/blob/master/src/tester.rs)), with no seed variable and no failure persistence. 1.1.0 came five years after 1.0.3.
- **bolero** ([book](https://camshaft.github.io/bolero), [source](https://github.com/camshaft/bolero/tree/master/book/src)).
  - `bolero::check!()` runs on stable under `cargo test` as a random property test, replaying any stored corpus.
  - `cargo bolero test --engine libfuzzer|afl|honggfuzz|kani` fuzzes the same target. Sanitizers need nightly. Under Miri, isolation must be disabled for it to read corpora.
  - `cargo-bolero` needs `binutils-dev` and `libunwind-dev` on Linux unless built with `--no-default-features`.
  - 260 stars; one maintainer's repository.
- **arbitrary** is the trait cargo-fuzz and bolero use to turn fuzzer bytes into typed values. 1.5.0 moved `derive_arbitrary` to `syn` 3 (MSRV 1.71) and fixed array-generation soundness ([changelog](https://github.com/rust-fuzz/arbitrary/blob/main/CHANGELOG.md)).
- **hegeltest** ([README](https://github.com/hegeldev/hegel-rust), [crate docs](https://github.com/hegeldev/hegel-rust/blob/main/src/lib.rs)).
  - It is "in beta" and "may make breaking changes", with 236 releases in 12 months.
  - Tests are `#[hegel::test] fn t(tc: TestCase)`, drawing values with `tc.draw(...)`.
  - The engine is a shared library, `libhegel_c`, that the build script compiles with a nested `cargo` invocation and that tests load at runtime. The `static-engine` feature links it in instead.
  - Seeds: `HEGEL_SEED`, `HEGEL_DERANDOMIZE`, and a `#[hegel::reproduce_failure("…")]` blob. Profiles (`development`, `ci`, `workload`) are chosen automatically.

### 4.2 Fit for Goodfetch

- **proptest stays the right choice.** It is the most used, its MSRV of 1.88 is under Goodfetch's (latest stable minus two, 1.97 today, ADR 0030), and it has persisted seeds and state-machine testing.
- **Run-state rules and the Frontier** fit proptest-state-machine: a reference model of a run's Checkpoints and Effects, checked against the real store.
- **Regression files** *(derived from `file.rs`, not tested)*. With STYLE.md's `tests/it/main.rs` layout, the lookup finds `tests/it/main.rs`, so a failure in `tests/it/frontier.rs` lands in `tests/proptest-regressions/frontier.txt`, inside the package. Unit tests in `src/` write to `<crate>/proptest-regressions/…`.
- **hegeltest** is worth a later look for Hypothesis-quality shrinking. Its beta status and runtime-loaded engine conflict with the dependency bar's "maintained or deliberately stable" and with static musl, so it is not a candidate now.
- **bolero** would merge property tests and fuzz targets, but it is far less adopted than proptest plus cargo-fuzz.

---

## 5. Fuzzing tools: cargo-fuzz, cargo-afl, bolero; corpus handling

Continuous fuzzing and security use belong to [Security testing](https://github.com/Jbmanllr/automan/issues/273); this section covers the tools.

### 5.1 Facts

| Tool | Version (date) | Engine | Toolchain | Platforms (documented) |
|---|---|---|---|---|
| cargo-fuzz + libfuzzer-sys | 0.13.2 (2026-06-09) + 0.4.13 (2026-06-04) | libFuzzer, in-process, coverage-guided | nightly | x86-64 and aarch64 Unix; basic Windows via MSVC ASan since 0.13.0 |
| cargo-afl / afl | 0.18.2 (2026-05-11) | AFL++ | stable (C compiler and make needed) | x86-64 Linux, x86-64 and arm64 macOS |
| cargo-bolero | 0.13.7 (2026-10-03) | libFuzzer (default), AFL, honggfuzz, Kani | stable without sanitizers | Linux needs `binutils-dev`, `libunwind-dev` |

- **cargo-fuzz.**
  - **Platforms.** Its [README](https://github.com/rust-fuzz/cargo-fuzz) still says libFuzzer needs LLVM sanitizer support, so x86-64 and aarch64 Unix only, "not Windows", on nightly with a C++11 compiler. The [changelog](https://github.com/rust-fuzz/cargo-fuzz/blob/main/CHANGELOG.md) for 0.13.0 (2025-06-25) added "basic support for fuzzing on windows via `--no-include-main-msvc`", and the [Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz/windows.html) documents it through MSVC AddressSanitizer.
  - **0.13.0 also** added `--codegen-units` (default 1, the best fuzzing throughput, but slow to compile) and line-tables-only debug info by default. 0.13.2 added `cargo fuzz init --fuzz-engine`.
  - **Commands:** `init`, `add`, `run`, `fmt`, `tmin` (minimize a crashing input), `cmin` (minimize the corpus), `coverage`.
  - **Corpus and outputs.** Crashes go to `fuzz/artifacts/<target>/` ([tutorial](https://rust-fuzz.github.io/book/cargo-fuzz/tutorial.html)). The whole dependency tree is built with `--cfg fuzzing` ([guide](https://rust-fuzz.github.io/book/cargo-fuzz/guide.html)).
- **AddressSanitizer targets.** Rust's [sanitizer docs](https://github.com/rust-lang/rust/blob/master/src/doc/unstable-book/src/compiler-flags/sanitizer.md) list `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, `x86_64-unknown-freebsd` and the Fuchsia targets. No musl target is listed.
- **CI smoke test.** The [Rust Fuzz Book's CI page](https://rust-fuzz.github.io/book/cargo-fuzz/ci.html) builds every target and runs each for `-max_total_time=300` (5 min) in a matrix, uploading `fuzz/artifacts` on failure. Ruff builds its fuzz targets on PRs with `cargo fuzz build -s none`, without a sanitizer, and fuzzes daily in a separate workflow ([Ruff `ci.yaml`](https://github.com/astral-sh/ruff/blob/350c634/.github/workflows/ci.yaml), `daily_fuzz.yaml`). Nextest's `on-timeout = "pass"` exists for time-boxed fuzz-style tests (§2).
- **afl.rs.** [README](https://github.com/rust-fuzz/afl.rs) and [setup](https://rust-fuzz.github.io/book/afl/setup.html): CMPLOG is on by default. `cargo afl system-config` needs root. `fuzz_with_reset!` resets static state between persistent-mode iterations.
- **Corpus handling with bolero.** The corpus a fuzz run produces is replayed by plain `cargo test`. It "should either be committed to the project repository or be stored/restored from storage, like S3" ([corpus replay](https://github.com/camshaft/bolero/blob/master/book/src/features/corpus-replay.md)).
- **Adoption.** Fuzz crates exist in Ruff, Deno (`libs/npm/fuzz`), Tokio, Nushell, Turso and nine Materialize crates (§12). Nushell runs its fuzz targets on Linux, macOS and Windows in CI (§12.3).

### 5.2 Fit for Goodfetch

- **cargo-fuzz still fits** STYLE.md's "one target per decoder". The targets build and run on Linux glibc (`x86_64-unknown-linux-gnu`), not musl. A pull-request job can build all targets cheaply with `-s none`, as Ruff does, and leave fuzzing time to the work tracked in [#273](https://github.com/Jbmanllr/automan/issues/273).
- **Corpora.** A small seed corpus per decoder in the repository makes `cargo test` replay known inputs. Large corpora belong in storage.

---

## 6. Mutation testing: cargo-mutants

### 6.1 Facts

- **Version, cadence, adoption.** 27.1.0 (2026-06-02); 7 releases in 12 months; 0.72 M downloads; 1,329 stars; MIT; MSRV 1.88 ([crates.io](https://crates.io/crates/cargo-mutants)). "Behavior, output formats, command-line syntax, json output formats, etc, may change from one release to the next" ([limitations](https://mutants.rs/limitations.html)). Nextest documents the integration ([nextest: cargo-mutants](https://nexte.st/docs/integrations/cargo-mutants/)).
- **Cost model** ([performance](https://mutants.rs/performance.html)). Each viable mutant costs one incremental build plus one test run, so anything that speeds up build or test "will have a multiplicative effect". The guide suggests:
  - a `[profile.mutants]` with `debug = "none"`;
  - a ramdisk for `TMPDIR`;
  - a faster linker: Mold gives about 20%, and Wild cut one tree's run "by more than half";
  - skipping doctests with `-- --all-targets`.

  The default test timeout is 5× the baseline test time, with a 20 s minimum ([timeouts](https://mutants.rs/timeouts.html)).
- **Incremental runs on a diff.** `--in-diff FILE` tests only mutants overlapping the diff's changed regions ([in-diff](https://mutants.rs/in-diff.html)). It ignores test-only changes and is "not a substitute for a full test run". The official [PR workflow example](https://github.com/sourcefrog/cargo-mutants/blob/main/examples/workflows/in-diff.yml) runs `git diff origin/<base>..`, then `cargo mutants --no-shuffle -vV --in-diff git.diff`, and uploads `mutants.out`.
- **`--iterate`** skips mutants caught or unviable in the previous run ([iterate](https://mutants.rs/iterate.html)).
- **Sharding.** `--shard k/n` splits the mutants across machines with no coordination ([shards](https://mutants.rs/shards.html)). It uses the `slice` (default) or `round-robin` algorithm and `--baseline=skip` to avoid a baseline per shard.
- **CI.** `--in-place` avoids copying the tree. Install from `taiki-e/install-action`. GitHub annotations appear when `$GITHUB_ACTION` is set ([CI](https://mutants.rs/ci.html)).
- **Nextest.** `--test-tool=nextest`, or `test_tool = "nextest"` in `.cargo/mutants.toml` ([nextest](https://mutants.rs/nextest.html)). Doctests do not run under nextest, so mutants only doctests would catch show as missed. Some trees, cargo-mutants itself included, are slower under nextest.
- **Limits** ([limitations](https://mutants.rs/limitations.html)):
  - It needs a hermetic test suite: flaky tests produce wrong conclusions.
  - It does not see macro- or build-script-generated code.
  - It reads `RUSTFLAGS` but not `.cargo/config.toml`.
  - It "does not yet understand conditional compilation, such as `#[cfg(target_os = "linux")]`", so it reports functions for other platforms as missed.
- **User complaints.** The most-reacted open issues: an HTML report (#282) and alternative test commands (#413) ([search](https://github.com/sourcefrog/cargo-mutants/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)).

### 6.2 Fit for Goodfetch

- **Pull requests:** `--in-diff` on the changed Rust code, Linux only, with nextest.
- **Full runs:** sharded on a schedule.
- **Platform code.** The `cfg(target_os)` limit means OS-specific modules (sandbox, Job Objects, `PR_SET_PDEATHSIG`) need excluding or reading with care.
- **Doctests.** Mutants caught only by `goodfetch`'s doctests will look missed under nextest.

---

## 7. Coverage measurement: cargo-llvm-cov (information, not a gate)

### 7.1 Facts

- **Version, cadence, adoption.** 0.9.1 (2026-09-06); 15 releases in 12 months; 8.3 M downloads; Apache-2.0 OR MIT; MSRV 1.87 ([crates.io](https://crates.io/crates/cargo-llvm-cov)). It uses rustc's `-C instrument-coverage` and the `llvm-tools-preview` component ([README](https://github.com/taiki-e/cargo-llvm-cov)).
- **What it does.**
  - Line and region coverage; branch and MC/DC coverage are unstable, behind `--branch` and `--mcdc`. Doctest coverage is nightly-only (`--doctests`).
  - `cargo llvm-cov nextest` runs nextest.
  - Exports lcov, Cobertura, Codecov JSON and HTML.
  - `show-env` wraps arbitrary cargo commands.
  - `--fail-under-lines`, `-functions` and `-regions` thresholds exist; they are opt-in.
- **Platforms** (README, "as of 2025-12-30"). `{x86_64,i686}-pc-windows-msvc` and `{x86_64,aarch64}-pc-windows-gnullvm` are confirmed to work. `x86_64-pc-windows-gnu` and `aarch64-pc-windows-msvc` do not work with the default setup. Linux and macOS work.
- **CI cost.**
  - It builds a separate instrumented copy. By default it cleans some artifacts to avoid stale data, unless `--no-clean`, `--no-report` or `--no-run` is given.
  - Nextest has no doctests, so coverage from them must be merged from a nightly run ([nextest: test coverage](https://nexte.st/docs/integrations/test-coverage/)).
  - Turso runs `cargo llvm-cov nextest` as its Linux test step: 8.8 min on a 4-vCPU runner, after a 2.3 min build. Its uninstrumented nextest steps took 4.0 min on an 8-vCPU Windows runner and 3.0 min on a 6-vCPU macOS runner, which is not a like-for-like comparison (§12.3).
  - Vector swaps `cargo nextest run` for `cargo llvm-cov nextest --no-report` behind `COVERAGE=true` ([Vector `Makefile`](https://github.com/vectordotdev/vector/blob/88da8cb/Makefile)).
- **User complaints.** Excluding `#[test]` code automatically (#123, +36), branch coverage (#8), doctests (#2) ([search](https://github.com/taiki-e/cargo-llvm-cov/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)).

### 7.2 Fit for Goodfetch

- **A non-blocking job.** A Linux x64 `cargo llvm-cov nextest` job reporting lcov or HTML, with no `--fail-under-*`, matches "information rather than a gate" and avoids the Windows arm64 gap.
- **Doctests.** `goodfetch`'s doctest coverage would need the nightly `--doctests` path or is simply left out.

---

## 8. CLI testing: trycmd and snapbox, assert_cmd, insta-cmd

### 8.1 Facts

| Crate | Version (date) | Releases 12 mo | Downloads | What it is |
|---|---|---|---|---|
| assert_cmd | 2.2.2 (2026-05-11) | 7 | 90.3 M | Runs a binary, then asserts on exit status, stdout and stderr with predicates |
| snapbox | 1.2.2 (2026-05-26) | 10 | 13.8 M | Snapshot toolbox for values, CLI output and filesystem changes; `[..]` wildcards; `SNAPSHOTS=overwrite` updates |
| trycmd | 1.2.1 (2026-07-21) | 8 | 7.5 M | File-driven CLI cases (`.toml`, `.trycmd`, `.md`, including README examples); `TRYCMD=overwrite` / `dump` |
| insta-cmd | 0.7.0 (2026-06-14) | 1 | 4.4 M | `assert_cmd_snapshot!` on a `Command` through insta |

Sources: crates.io, the [snapbox](https://github.com/assert-rs/snapbox/blob/main/crates/snapbox/src/lib.rs) and [trycmd](https://github.com/assert-rs/snapbox/blob/main/crates/trycmd/src/lib.rs) crate docs, and the [assert_cmd changelog](https://github.com/assert-rs/assert_cmd/blob/master/CHANGELOG.md).

- **assert_cmd** sits in the same organization as snapbox (assert-rs, maintained by Ed Page, who also maintains Cargo's test infrastructure). Its locator API changed around Cargo's `build-dir`:
  - 2.0.17 (2025-04-16) added `cargo_bin!`, "which will work with Cargo's `build-dir`".
  - 2.1.0 (2025-10-28) deprecated `Command::cargo_bin` in favour of `cargo_bin!` and `cargo_bin_cmd!`.
  - 2.1.3 (2026-03-10) un-deprecated it; 2.2.0 turns a bad path into a panic.

  Declared in the root manifests of uv, Vector, Nushell and Materialize (§12).
- **Binary location.** Cargo builds a package's binaries before its integration tests and sets `CARGO_BIN_EXE_<name>` for them ([Cargo book: integration tests](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#integration-tests)). `build.build-dir`, stabilized in Cargo 1.91 (2025-10-30), has a layout that "may change without notice" ([Rust 1.91 release notes](https://github.com/rust-lang/rust/blob/master/RELEASES.md)). Tools that guess binary paths are fragile; the env-var route is not.
- **trycmd's users,** per its README: typos, cargo-edit, and clap for its examples. snapbox is the assertion layer of Cargo's own test suite (`cargo-test-support` depends on it; [Cargo](https://github.com/rust-lang/cargo/blob/3960fa6/crates/cargo-test-support/Cargo.toml)).
- **Who uses what.**
  - Ruff uses insta-cmd.
  - uv wraps assert_cmd plus insta with filters in its `uv-test` crate's `uv_snapshot!` macro ([uv-test `Cargo.toml`](https://github.com/astral-sh/uv/blob/59e5912/crates/uv-test/Cargo.toml)).
  - Cargo uses snapbox.

### 8.2 Fit for Goodfetch

- **Snapshot format.** The CLI's human output, `--json` output (under semver, ADR 0030) and stderr hints are snapshot material. Since insta is already chosen, assert_cmd plus insta (uv's pattern) or insta-cmd (Ruff's) keep one snapshot format and one review flow. trycmd and snapbox would add a second.
- **Maintenance.** insta-cmd's low cadence is the risk. assert_cmd is the better-maintained base, and insta's `filters` handle paths and durations.
- **Location.** These tests belong in the binary crate's `tests/it`, where `CARGO_BIN_EXE_goodfetch` is set.

---

## 9. Containers and emulators driven from tests: testcontainers-rs

### 9.1 Facts

- **Version, cadence, adoption.** testcontainers 0.28.0 (2026-08-06); 11 releases in 12 months; 45.4 M downloads; 1,137 stars; MIT OR Apache-2.0; MSRV 1.88. `testcontainers-modules` 0.15.0 (2026-02-21) is the community module crate ([crates.io](https://crates.io/crates/testcontainers), [repo](https://github.com/testcontainers/testcontainers-rs)). It is "the official Rust language fork" of Testcontainers.
- **API.**
  - `GenericImage` or an `Image` impl, an async runner, a sync runner (`blocking` feature), and wait strategies.
  - Optional features: Docker Compose, reusable containers, host-port exposure. TLS uses `ring` (default) or `aws-lc-rs` ([`Cargo.toml`](https://github.com/testcontainers/testcontainers-rs/blob/main/testcontainers/Cargo.toml)).
  - Containers are exposed on random host ports, so tests can run in parallel ([quickstart](https://github.com/testcontainers/testcontainers-rs/blob/main/docs/quickstart/testcontainers.md)).
- **Cleanup.**
  - Removal relies on `Drop`. `TESTCONTAINERS_COMMAND=keep` keeps containers (quickstart).
  - The opt-in `watchdog` feature removes containers on SIGTERM, SIGINT or SIGQUIT from a background thread ([`watchdog.rs`](https://github.com/testcontainers/testcontainers-rs/blob/main/testcontainers/src/watchdog.rs)).
  - There is no Ryuk resource reaper ([#577](https://github.com/testcontainers/testcontainers-rs/issues/577), open since 2024-04; [#949](https://github.com/testcontainers/testcontainers-rs/issues/949), 2026-06).
  - Containers held in statics are never dropped ([#528](https://github.com/testcontainers/testcontainers-rs/issues/528)).
- **Runtime.** It needs a Docker-API-compatible runtime. It is tested on Docker on Linux and Docker Desktop on macOS and Windows; Podman works through its Docker socket ([requirements](https://github.com/testcontainers/testcontainers-rs/blob/main/docs/system_requirements/docker.md), [configuration](https://github.com/testcontainers/testcontainers-rs/blob/main/docs/features/configuration.md)).
- **GitHub runners.**
  - Docker container actions, job containers and service containers need a Linux runner; on GitHub-hosted runners, "an Ubuntu runner" ([GitHub docs](https://docs.github.com/en/actions/tutorials/use-containerized-services/use-docker-service-containers)).
  - arm64 macOS runners have no nested virtualization ([runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)).
  - `ubuntu-slim` runners cannot run Docker-in-Docker.
- **Comparable use.**
  - Vector gates its integration tests behind 48 `*-integration-tests` Cargo features, with 32 Docker Compose files under `tests/integration/` ([Vector `Cargo.toml`](https://github.com/vectordotdev/vector/blob/88da8cb/Cargo.toml)).
  - Cargo turns on its container tests with `CARGO_CONTAINER_TESTS=1` in CI ([Cargo `main.yml`](https://github.com/rust-lang/cargo/blob/3960fa6/.github/workflows/main.yml)).
  - None of the ten projects declares `testcontainers` in its root manifest (§12).

### 9.2 Fit for Goodfetch

- **Image pinning.** ADR 0032 already runs the sink suites against RustFS and LocalStack on Linux only, with images pinned by digest. testcontainers fits that: an image reference can include the digest *(unverified for testcontainers-rs's API; Docker accepts `name@sha256:…`)*.
- **Leaked containers.** Nextest kills a timed-out test with SIGKILL after the grace period, which skips `Drop`. A test killed this way can leak its container. Enable the `watchdog` feature, label containers per CI run, and run a cleanup step after the job. *(Inference from §2 and §9.1.)*

---

## 10. HTTP mocks for true externals: wiremock, httpmock, mockito

### 10.1 Facts

| Crate | Version (date) | Last commit | Downloads | Async / sync | Notable |
|---|---|---|---|---|---|
| wiremock | 0.6.5 (2025-08-24) | 2025-08-24 | 88.0 M | async only, tokio or async-std | Matchers, response templating, in-process server pool |
| httpmock | 0.8.3 (2026-02-04) | 2026-10-13 | 31.8 M | both | **Record and playback**, forward/proxy modes, HTTPS, HTTP/2, standalone server and Docker image, YAML mocks; MSRV 1.88 |
| mockito | 1.7.2 (2026-02-02) | 2026-08-01 | 59.1 M | both | Local pool of servers, HTTP/1 and 2, multiple hosts |

Sources: crates.io; READMEs of [wiremock](https://github.com/LukeMathWalker/wiremock-rs), [httpmock](https://github.com/httpmock/httpmock) and [mockito](https://github.com/lipanski/mockito); [httpmock recording docs](https://github.com/httpmock/httpmock/blob/master/docs/website/src/content/docs/record-and-playback/recording.mdx).

- **wiremock** has had no release and no commit for over 13 months. Pull requests from November 2025 and February 2026 are unanswered, among them a deadpool update ([issues](https://github.com/LukeMathWalker/wiremock-rs/issues?q=sort%3Acreated-desc)). It is still the most downloaded and is declared by uv (inside `uv-test`) and Vector (§12).
- **httpmock** records real exchanges in two ways: by forwarding (the client points at the mock server, which forwards matching requests) or as a proxy. Recordings are saved and played back later (`record` feature).
- **mockito** is declared by Nushell (§12).

### 10.2 Fit for Goodfetch

- **The mock layer.** STYLE.md's "true external" category uses "a mock adapter replaying recorded exchanges". That can sit at the adapter's Rust trait, with no HTTP mock at all, or at HTTP. For HTTP, httpmock is the only one of the three that records and replays, and it is actively maintained.
- **wiremock's server pool** only helps tests that share a process. Under nextest every test is its own process, so the pool brings nothing *(inference)*.
- **Recordings.** They would need redacting before commit, since model-provider exchanges carry keys (ADR 0015).

---

## 11. Speed of the test build

### 11.1 Facts

- **Linkers.**
  - Rust 1.90.0 (2025-09-18) uses LLD by default on `x86_64-unknown-linux-gnu`. On ripgrep, incremental-rebuild linking is 7× faster and end-to-end compile time 40% shorter; from-scratch debug builds are 20% faster. `-C linker-features=-lld` reverts ([Rust blog, 2025-09-01](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable/)).
  - mold 3.0.0 (2026-10-05) and Wild 0.10.0 (2026-08-04) are faster still on Linux. Wild supports only Linux (x86-64, AArch64, RISC-V, LoongArch64, initial PPC64LE); Mach-O and Windows are "looking ahead" items ([Wild README](https://github.com/wild-linker/wild)).
  - uv, Ruff, Turso, Vector and Nushell install mold in CI; Deno and Materialize pass `-fuse-ld=lld` (§12).
- **Cargo defaults.** Cargo 1.99.0 (2026-10-01) disables incremental compilation by default when `CI` is set ([RELEASES.md, 1.99.0 compatibility notes](https://github.com/rust-lang/rust/blob/master/RELEASES.md)). `[profile.test]` inherits `dev` ([Cargo book: profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)). `debug = "line-tables-only"` (Rust 1.71+) keeps backtraces with file and line numbers and drops the rest of the debug info.
- **Profiles used by comparable projects.**
  - uv: `[profile.dev] debug = "line-tables-only"`, plus `fast-build` (`opt-level = 1`, `debug = 0`) and `fast-build-nightly` with `panic = "abort"` used with `-Z panic-abort-tests` in CI ([uv `Cargo.toml`](https://github.com/astral-sh/uv/blob/59e5912/Cargo.toml)).
  - Materialize: `[profile.ci]` inherits its optimized profile, with `debug = "line-tables-only"` and debug assertions on ([Materialize `Cargo.toml`](https://github.com/MaterializeInc/materialize/blob/33384fd/Cargo.toml)).
  - Tokio also runs its workspace with `-Zpanic-abort-tests` ([Tokio `ci.yml`](https://github.com/tokio-rs/tokio/blob/1989b46/.github/workflows/ci.yml)).
- **sccache** 0.18.0 (2026-09-16). "Crates that invoke the system linker cannot be cached. This includes `bin`, `dylib`, `cdylib`, and `proc-macro` crates", and "incrementally compiled crates cannot be cached" ([README](https://github.com/mozilla/sccache)). Test binaries are linked, so sccache only caches dependency compilation. It has a GitHub Actions cache backend. Zed uses sccache on a Cloudflare R2 bucket ([Zed `steps.rs`](https://github.com/zed-industries/zed/blob/1773013/tooling/xtask/src/tasks/workflows/steps.rs)).
- **Swatinem/rust-cache** v2.9.2 (2026-08-06) ([README](https://github.com/Swatinem/rust-cache)).
  - **What it caches:** `~/.cargo` and `./target`, but strips workspace crates, incremental artifacts and anything older than a week, and sets `CARGO_INCREMENTAL=0`. Caching workspace crates is "generally not effective".
  - **Key:** the rustc version and host, `RUSTFLAGS`-like variables, and hashes of `Cargo.lock`, `Cargo.toml`, `rust-toolchain` and `.cargo/config.toml`.
  - **Options:** `save-if` limits saving to chosen branches; `cache-workspace-crates` exists, and uv turns it on.
  - **GitHub's limits** ([dependency caching](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)): 10 GB per repository by default (configurable, beyond that billed), eviction after 7 days unused, and 200 uploads per minute.
- **Splitting test binaries.**
  - Cargo compiles every file in `tests/` as a separate crate and binary and runs them serially. Its book says many integration tests may be better as "a single integration test" with modules ([Cargo book](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#integration-tests)).
  - Aleksey Kladov's 2021 post reports that moving Cargo's own suite to one binary cut test compile time 3× and on-disk artifacts 5×. It recommends `tests/it/main.rs`, and for internal libraries no integration tests at all ([matklad, "Delete Cargo Integration Tests"](https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html)).
  - uv's 2026 measurements refine this (§12.2).
- **Doctests.** Since the 2024 edition, rustdoc merges doctests into one binary where it can ([edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/rustdoc-doctests.html)). cargo-mutants' advice that "every doctest example becomes a separate test binary" predates this.
- **`cargo nextest archive`** separates build from run (§2.1). In the checked projects, partitioning runs mostly rebuild per job: uv's three Windows partitions each build, sharing one rust-cache entry. Deno uploads one test-server binary as an artifact (§12.3).
- **Windows.** uv runs its Windows tests from a Dev Drive, copying the checkout to it first ([uv `test.yml`](https://github.com/astral-sh/uv/blob/59e5912/.github/workflows/test.yml)).
- **Runner sizes.** Public-repository GitHub runners: Linux and Windows x64 and arm64 have 4 vCPUs, 16 GB RAM and a 14 GB SSD; Intel macOS 4 vCPUs and 14 GB; arm64 macOS 3 vCPUs (M1) and 7 GB ([runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)). uv, Ruff, Zed, Turso and Nushell buy larger runners from Depot, Namespace, Blacksmith or Incredibuild for their test jobs (§12.3).

### 11.2 Fit for Goodfetch

- **What CI already gets.** Rust 1.99's CI default and rust-cache both turn incremental off. LLD is already the default on `x86_64-unknown-linux-gnu` but not on the musl target *(the default linker of `x86_64-unknown-linux-musl` was not checked)*.
- **What is left to choose:**
  - a `ci` or `test` profile with `debug = "line-tables-only"`;
  - mold or Wild for the Linux test jobs;
  - Dev Drive for Windows;
  - a 14 GB disk budget per runner: one uv-style 12-binary layout used 591 MiB for one package's integration binaries alone.
- **sccache** would only speed up dependencies, which rust-cache already restores. ADR 0032's "rust-cache, saving only from `main`" stays sound.

---

## 12. Prior art

### 12.1 Layout, helpers and runner

Read at the commits linked in the table, 2026-10-06 to 2026-10-13. "Binaries" counts `tests/*.rs` and `tests/*/main.rs` files directly under a directory with a `Cargo.toml`. It misses `[[test]]` targets with custom paths, which matter for Deno and Nushell.

| Project (commit) | Runner in CI | Integration-test binaries | Shared helpers | Test-only features |
|---|---|---|---|---|
| [uv `59e5912`](https://github.com/astral-sh/uv/tree/59e5912) | nextest (`ci-linux`, `ci-macos`, `ci-windows` profiles; Windows `--partition hash:N/3`); JUnit | 19 in 7 packages; `crates/uv` has 12 command-area harnesses | `uv-test` crate (`TestContext`, `uv_snapshot!`, insta filters, assert_cmd, wiremock) | `test-crates-io`, `test-pypi`, `test-python`, `test-slow`… gate tests needing network or Python |
| [Ruff `350c634`](https://github.com/astral-sh/ruff/tree/350c634) | `cargo insta test … --test-runner nextest`, then `cargo test --doc` | 43 in 15 packages; separate `*_integration_tests` crates | `ty_test`, `ruff_mdtest` (Markdown-driven tests) | `testing`, `test-uv` |
| [Deno `a18ce33`](https://github.com/denoland/deno/tree/a18ce33) | `cargo test` for its test crates, nextest for `deno_core`, Miri on nightly | One test crate per suite under `tests/` (`specs`, `integration`, `unit`, `node_compat`…), each `autotests = false` with one `harness = false` target on `file_test_runner`; 2,102 spec directories | `tests/util/server` (`test_util`, a test server binary) | none found |
| [Tokio `1989b46`](https://github.com/tokio-rs/tokio/tree/1989b46) | nextest, then `cargo test --doc`; `cargo miri nextest run --lib`; `cargo hack test --each-feature`; loom; valgrind | **253** in 6 packages (187 in `tokio/tests`) | `tokio-test` crate; `tests-integration`, `tests-build` crates | `test-util` (time control) |
| [Vector `88da8cb`](https://github.com/vectordotdev/vector/tree/88da8cb) | nextest (`retries = 3`, JUnit); `cargo llvm-cov nextest` when `COVERAGE=true` | 12 in 4 packages, plus feature-gated integration tests with 32 Compose files | `vector-lib` test utilities, `k8s-test-framework`, `fakedata` | 48 `*-integration-tests` features; `test`, `test-utils` |
| [Nushell `fab6af9`](https://github.com/nushell/nushell/tree/fab6af9) | `cargo test --workspace --all-targets` with `.cargo/ci.toml`; fuzz targets on 3 OSes | 31 in 15 packages; the root `tests/main.rs` is one `harness = false` binary (kitest harness) | `nu-test-support` (`nu!` macro, `Playground`), `nu-plugin-test-support` | none found |
| [Zed `1773013`](https://github.com/zed-industries/zed/tree/1773013) | `cargo nextest run --workspace --no-fail-fast`, filtered to `rdeps(changed packages)`; doctests separate; Miri scheduled | 9 in 7 packages (most tests are unit tests) | gpui's test context, behind features | **`test-support` on 93 crates** |
| [Cargo `3960fa6`](https://github.com/rust-lang/cargo/tree/3960fa6) | `cargo test` | `tests/testsuite/main.rs`: one binary over 994 source files | `cargo-test-support` (published, snapbox-based), `cargo-test-macro` | `CARGO_CONTAINER_TESTS=1` env |
| [Turso `15ea4bc`](https://github.com/tursodatabase/turso/tree/15ea4bc) | `cargo llvm-cov nextest` (Linux), nextest elsewhere; nextest `loom` and `shuttle` profiles | 21 in 13 packages | simulator, differential oracle and fuzzer, `sqltest` crates under `testing/` | `test_helper` on 6 crates |
| [Materialize `33384fd`](https://github.com/MaterializeInc/materialize/tree/33384fd) | nextest on Buildkite (`ci`, `coverage`, `sanitizer` profiles with JUnit); `cargo miri nextest --partition count:` | 48 in 24 packages | `#[mz_ore::test]` (sets up logging), `mz-test-util`, mock crates (`frontegg-mock`, `oidc-mock`) | `test` on `mz-ore` and 4 more |

Sources: each project's `.config/nextest.toml`, CI workflow files, root and crate `Cargo.toml`, read through `git show` at the commit above. Feature counts scan `[features]` tables for names beginning `test`, `testing` or `test_helper`.

Patterns:

- **Change-scoped test selection.** Zed computes changed packages and runs `rdeps(...)` filtersets ([Zed `run_tests.rs`](https://github.com/zed-industries/zed/blob/1773013/tooling/xtask/src/tasks/workflows/run_tests.rs)).
- **Serialization and priorities through nextest.** `serial` groups: uv for keyring tests, Ruff for file-watching tests, Zed for its `db` package. Long tests start first with `priority` (Zed, Materialize, uv on Windows). Heavy tests reserve 4 slots with `threads-required` (Materialize).
- **Slow-timeout termination as a deadlock stop-gap.** uv: 10 s × 12; Ruff on CI: 1 s × 60; Materialize: 90 s × 2; Vector: 30 s × 4.

### 12.2 uv's measurement of binary layouts

[uv #8093](https://github.com/astral-sh/uv/pull/8093) (merged 2024-10-11) moved all of uv's integration tests into one binary. uv then had 41 top-level targets and a nextest run saw 91 binaries; its benchmark went from about 35 s to 9–14 s. [uv #17551](https://github.com/astral-sh/uv/pull/17551) (2026-02-07) moved the shared `TestContext` into a separately compiled `uv-test` crate. [uv #19770](https://github.com/astral-sh/uv/pull/19770) (2026-06-10) then split the suite (62 modules, 2,958 tests) into 12 command-area harnesses, measured on one commit:

| Layout | Harnesses | No-op | Localized test edit | Shared `uv-test` edit | Package rebuild | Size of integration binaries |
|---|---:|---:|---:|---:|---:|---:|
| Single | 1 | 1.10 s | 7.52 s | 10.20 s | 35.18 s | 96.5 MiB |
| Command areas | 12 | 1.22 s | 2.60 s | 7.60 s | 33.30 s | 591.2 MiB |
| Per file | 62 | 2.01 s | 2.58 s | 17.51 s | 42.95 s | 1,700.9 MiB |

The PR also reports CI time "effectively unchanged": runner-minutes 25:53 before against 26:01 after.

### 12.3 Suite times

Measured from the GitHub Actions API on 2026-10-13: the latest successful run on the default branch or merge queue. "Step" is the test command alone and "job" includes setup. Runner sizes differ, so rows compare poorly across projects.

| Project, run | Job (runner) | Test step | Job |
|---|---|---:|---:|
| uv [38091256194](https://github.com/astral-sh/uv/actions/runs/38091256194) (2026-10-10) | Linux (Depot, 16 vCPU) | 2.3 min | 3.8 min |
| | macOS (Namespace macOS 15) | 4.3 | 4.8 |
| | Windows, each of 3 hash partitions (Namespace 16×32) | 2.3–2.7 | 3.5–3.8 |
| Ruff [38369239946](https://github.com/astral-sh/ruff/actions/runs/38369239946) (2026-10-13) | Linux (Depot, 16 vCPU) | 2.1 (+0.2 doctests) | 4.3 |
| | macOS (Namespace) / Windows (Namespace 16×32) | 2.3 / 3.4 | 2.7 / 4.1 |
| Tokio [38291398920](https://github.com/tokio-rs/tokio/actions/runs/38291398920) (2026-10-12) | all crates, all features: `ubuntu-latest` / `macos-latest` / `windows-latest` (GitHub-hosted) | 3.5 / 3.4 / 7.9 | 3.8 / 4.1 / 8.8 |
| | Miri (`ubuntu-latest`) | — | 21.1 |
| Zed [38400489199](https://github.com/zed-industries/zed/actions/runs/38400489199) (2026-10-13) | Linux (Namespace 16×32) / macOS (Namespace large) / Windows (self-hosted, 32 vCPU) | 6.7 / 10.1 / 8.2 | 7.4 / 10.6 / 8.6 |
| | doctests (Linux) | 1.4 | 2.0 |
| Cargo [38297981747](https://github.com/rust-lang/cargo/actions/runs/38297981747) (2026-10-12, merge queue) | `cargo test -p cargo`: Linux x64 beta / nightly (`ubuntu-latest`) | 5.8 / 8.2 | 9.9 / 14.4 |
| | macOS arm64 nightly / Windows x64 MSVC stable / Windows arm64 MSVC stable | 14.3 / 15.8 / 21.3 | 21.6 / 22.8 / 29.3 |
| Turso [38366703531](https://github.com/tursodatabase/turso/actions/runs/38366703531) (2026-10-13) | Linux (Blacksmith 4 vCPU): build, then nextest under llvm-cov | 2.3 + 8.8 | 16.8 |
| | Windows (8 vCPU) / macOS (6 vCPU): plain nextest | 4.0 / 3.0 | 11.6 / 6.9 |
| Nushell [38391017232](https://github.com/nushell/nushell/actions/runs/38391017232) (2026-10-13) | Linux / Windows (Incredibuild) / macOS (`macos-latest`); whole job incl. check, clippy, build, test | — | 9.9 / 7.7 / 14.6 |
| Deno [34212081300](https://github.com/denoland/deno/actions/runs/34212081300) (2026-09-08, latest successful push run on `main`) | specs shard 1/2, integration shard 1/2 (Linux arm64) | — | 7.9, 8.1 |
| | libs (macOS 14) / `deno_core` tests (Linux x64) | — | 14.2 / 24.2 |

Not determined: Vector (its `test.yml` merge-queue run contains only a gate job) and Materialize (CI on Buildkite, not checked). None of the ten projects publishes a suite-time figure outside CI logs, except uv's PR tables in §12.2.

### 12.4 Fit for Goodfetch

- **Shape.** The closest analogues for the engine's shape (one CLI binary, many crates, real external processes) are uv and Ruff:
  - nextest with a `ci` profile and JUnit;
  - a separately compiled test-support crate;
  - insta with redactions or filters;
  - doctests as a separate step;
  - Windows split by hash partition.
- **Test helpers.** Zed's `test-support` features on 93 crates and Tokio's `test-util` correspond to STYLE.md's `test-util` feature, which STYLE.md allows only on `goodfetch`. uv's choice, a separate `uv-test` crate, keeps helpers out of every crate's features. That suits a workspace where features exist only on the embeddable crate.
- **Vector's model.** Its 48 integration-test features are the opposite: STYLE.md's "a capability is never a Cargo feature" and ADR 0032's change detection make that model unnecessary for the Sinks.

---

## 13. Already decided: does each choice still fit?

| Decision (STYLE.md, ADR 0032) | Still fits? | What changed or must be pinned |
|---|---|---|
| cargo-nextest as the runner | Yes | Require ≥ 0.9.148 (stress-run exit code fix). Add `flaky-result = "fail"` with `retries = 0`. Doctests need `cargo test --doc -p goodfetch`. Setup scripts are experimental with no teardown, so long-lived helpers start outside nextest. |
| proptest for parsers | Yes | 1.12.0 (MSRV 1.88). Add proptest-state-machine for stateful modules. Commit `proptest-regressions/`. hegeltest is a beta to revisit, not a replacement. |
| insta, with redactions | Yes | CI writes nothing by default. Add `--unreferenced reject` and run through `cargo insta test --test-runner nextest`. |
| cargo-fuzz per decoder | Yes | Nightly; runs on `x86_64-unknown-linux-gnu`, not musl, since ASan has no musl target. A pull request can `cargo fuzz build -s none`. |
| Miri on opted-in `unsafe` | Yes | `cargo miri nextest run` runs tests in parallel (up to 3–4× faster per nextest). Archives are unsupported under Miri. Tokio's Miri job takes about 21 min on a free runner. |
| cargo-hack over `goodfetch`'s features | Yes | 0.6.45 (2026-05-30); `--feature-powerset` with `--depth` bounds the matrix. |
| One integration-test binary per crate | Yes, as the default | Avoid per-file binaries; Cargo's book, matklad and uv all agree. Once one crate's suite grows into thousands of tests, uv's measurement favours a handful of area harnesses plus a separate test-support crate. |
| `Swatinem/rust-cache`, saving only from `main` | Yes | v2.9.2. Workspace crates are not cached by default; mind the 10 GB repository cache. |

---

## 14. Unverified

- Whether testcontainers-rs accepts a digest-pinned image reference (`name@sha256:…`) through `GenericImage` without workarounds. Not tested.
- The default linker for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (whether LLD applies). The 1.90 change names only `x86_64-unknown-linux-gnu`.
- Where proptest writes regression files under the `tests/it/main.rs` layout. Derived from reading `file.rs`, not observed in a run.
- hegeltest's behaviour on static musl targets, given its runtime-loaded `libhegel_c`. Not checked.
- That a test killed by nextest's SIGKILL leaks its testcontainers container. Inferred from the `Drop`-based cleanup; not reproduced.
- wiremock's pool giving no benefit under process-per-test. Inferred from its README's description of an in-process pool.
- `afl.rs` on Linux arm64 and Windows: the book lists only x86-64 Linux and x86-64 and arm64 macOS; other platforms were not tested.
- Nextest's published speedups (1.37–3.38×) were measured with Rust 1.66 and older project revisions. Not re-measured.
- Suite times in §12.3 come from single runs on mixed runner classes. They are indicative, not benchmarks.
- Nushell's `kitest` harness and its compatibility with nextest. Not checked.
