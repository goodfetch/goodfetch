# Security testing beyond what the ADRs already require

Research of **October 13, 2026** for ticket [#273](https://github.com/Jbmanllr/automan/issues/273). Question: what security testing practice fits Goodfetch's surfaces beyond what the ADRs already require? Already required, and not re-examined here: `cargo-fuzz` per decoder and Miri (`STYLE.md`), Plugin confinement CI on every OS with refusal cases (ADR 0033), reset-gate canaries (ADRs 0001 and 0015), the CI egress canary (ADR 0043), Console security headers and origin-escape pages (ADR 0031), prompt-injection and fake-success pages (ADR 0025), tests that secrets are never printed (`STYLE.md`), and the supply-chain and signing gates (ADRs 0018, 0030, 0032, 0039).

**Sources.** Official docs and source code, read through the GitHub API at their default branch on 2026-10-13 unless a version is given: OSS-Fuzz and ClusterFuzzLite docs; the sandbox and permission tests of Chromium, Deno, gVisor, Firecracker and Wasmtime; V8's sandbox README; the OWASP SSRF cheat sheet and ASVS 5.0; PayloadsAllTheThings; ZAP, Nuclei, Schemathesis and the MCP specification and conformance suite; GitHub Docs; the GitHub Security Advisories of Deno and Firecrawl; RustSec; competitors' security pages.

**Measured versus cited.** One small experiment was run: how the Rust `url` crate and Node's WHATWG `URL` parse SSRF address encodings (§3). Measured facts are marked **M**; everything else is cited and marked **C**. Like everything in `rnd/`, this note informs a decision; it does not make one.

---

## 1. Summary

- **OSS-Fuzz is not open to Goodfetch yet; ClusterFuzzLite is.** OSS-Fuzz takes projects with "a significant user base and/or" that are "critical to the global IT infrastructure" **C**. ClusterFuzzLite runs the same tooling inside GitHub Actions, on Linux only, with libFuzzer and AddressSanitizer for Rust **C**. Its default is 600 s of fuzzing per pull request; its documented batch example fuzzes 3,600 s every 6 hours, with corpus pruning and coverage once a day **C**.
- **Two ClusterFuzzLite details clash with ADRs as written.** Its run action pulls the Docker image by the mutable tag `clusterfuzzlite-run-fuzzers:v1`, which pinning the action by SHA (ADR 0032) does not pin **C**. Its crash inputs and corpora go to GitHub artifacts or a storage repository, which are public on a public repository **C**. gVisor keeps its fuzzing reports on a private list "because automated fuzzing reports may contain undisclosed vulnerabilities" **C**.
- **The comparable projects test confinement in four ways that Goodfetch's ADR 0033 suite can borrow:**
  - **One test per forbidden call, with the exact outcome.** Chromium asserts `EPERM` for some calls and a `SIGSYS` death for others **C**.
  - **Tests generated from the policy file.** Firecracker checks every rule in its `seccompiler` filter with allowed arguments, then with changed ones **C**. Goodfetch uses `seccompiler` too.
  - **The same test outside and inside the sandbox.** Each gVisor syscall test runs natively on Linux and under each gVisor platform **C**.
  - **One regression test per bypass.** Deno added one spec test for each of its 2026 network-permission bypasses, each asserting that no socket reached the denied address **C**.
- **V8 ships a way to test its own sandbox.** A V8 build with `v8_enable_memory_corruption_api = true`, run with `--sandbox-testing`, lets JavaScript corrupt memory inside the V8 sandbox, as an exploit would **C**. Goodfetch builds V8 from source (ADR 0051), and the `v8` crate accepts extra GN arguments (`EXTRA_GN_ARGS`) **C**.
- **SSRF checks must see past address encodings.** The Rust `url` crate (2.5.8) turns decimal, hex, octal, shortened and enclosed-alphanumeric hosts into `127.0.0.1`, as WHATWG URL parsing requires **M**. It leaves `[::ffff:127.0.0.1]` as an IPv6 address **M**. A check that only tests IPv4 ranges misses it unless the address is passed through `to_canonical` first, stable since Rust 1.75 **C**.
- **None of the off-the-shelf HTTP scanners can import a JSON-RPC API or actively scan a WebSocket:**
  - ZAP's API scan imports OpenAPI, SOAP or GraphQL only **C**. Its WebSocket support is interception, manual fuzzing and passive scripts **C**.
  - Schemathesis tests OpenAPI and GraphQL APIs only **C**.
  - Nuclei's DAST templates cover SSRF, CRLF, LFI and redirects **C**.
  - The official MCP conformance suite (npm `0.1.16`, 2026-10-01) does test MCP over HTTP: it has a DNS-rebinding scenario and an authorization suite **C**.
- **Scanners call home by default, so ADR 0043's canary would trip on them:**
  - ZAP updates add-ons at start unless `-silent` is set **C**. In 2025, a bug made ZAP call home even under `-silent` **C**.
  - Nuclei checks for updates unless `-duc` is set **C**. Its out-of-band checks use public servers such as `oast.pro` unless `-ni` is set or a self-hosted interactsh server is used **C**.
- **Redacting secrets after the fact fails in practice; exact canaries work as a test oracle:**
  - Playwright's maintainers declined to filter secrets from traces, calling every approach "heuristics/best effort" **C**. Users still ask for it in 2026 **C**.
  - Playwright MCP has one redaction regression test per reported leak **C**.
  - GitHub masks only values registered with `add-mask`, and only in logs **C**. Its secret scanning covers the git history, issues, pull requests, discussions, wikis and gists; the docs page does not mention Actions logs or artifacts **C**.
- **Competitors publish attestations rather than test suites:**
  - Browserbase, Apify and Firecrawl each claim SOC 2 Type II **C**.
  - Browserbase cites unnamed third-party penetration tests **C**. Firecrawl takes reports through Bugcrowd **C**. Apify runs a disclosure policy with no bounty, excluding automated scanning and missing headers **C**.
  - Firecrawl is the only one whose repository runs SSRF regression tests in CI, added 2026-09-30 **C**.
  - Firecrawl's 2026 advisories add two bug classes Goodfetch shares: secrets leaked through error messages, and a shell reached from page content **C**.

---

## 2. Continuous fuzzing

### Facts

**OSS-Fuzz eligibility and terms.**

- **Who gets in.** "To be accepted to OSS-Fuzz, an open-source project must have a significant user base and/or be critical to the global IT infrastructure" ([accepting new projects](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/accepting_new_projects.md)). The FAQ adds "exposure to remote attacks" and the number of dependents as criteria, decided "case-by-case" ([FAQ](https://github.com/google/oss-fuzz/blob/master/docs/faq.md)) **C**.
- **Contact.** The primary contact must be an established committer whose email is tied to a Google account ([accepting new projects](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/accepting_new_projects.md)) **C**.
- **Rust.** Integration uses `cargo fuzz`; "the only supported fuzzing engine and sanitizer are `libfuzzer` and `address`" ([Rust guide](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/new-project-guide/rust_lang.md)) **C**.
- **Machines.** Fuzzing machines have one core, and a fuzz target should stay under 2.5 GB. An input taking more than about 25 s or 2.5 GB is filed as a timeout or out-of-memory bug, one per target at a time ([FAQ](https://github.com/google/oss-fuzz/blob/master/docs/faq.md)) **C**. x86_64 is the default architecture, i386 optional ([new project guide](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/new_project_guide.md)) **C**.
- **Disclosure.** Bugs open to the public 90 days after notification or when the fix ships, whichever is earlier, with a 14-day grace period ([disclosure guidelines](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/bug_disclosure_guidelines.md)) **C**.
- **AI remediation.** OSS-Fuzz now attaches proposed fixes ("remediation") to private bug reports by default, reviewed by a person unless the project opts out of the review ([new project guide](https://github.com/google/oss-fuzz/blob/master/docs/getting-started/new_project_guide.md), `disable_remediation`) **C**.
- **Corpora.** Corpora and test cases OSS-Fuzz produces may be reused freely ([FAQ](https://github.com/google/oss-fuzz/blob/master/docs/faq.md)) **C**.
- **What a good integration has.** A seed corpus, a dictionary, and the seed corpus replayed as a regression test in the project's own CI ([ideal integration](https://github.com/google/oss-fuzz/blob/master/docs/advanced-topics/ideal_integration.md)) **C**.

**Comparable projects on OSS-Fuzz** (project files read 2026-10-13):

| Project | On OSS-Fuzz | Notes |
|---|---|---|
| Wasmtime | Yes, libFuzzer + ASan | CI runs `cargo fuzz check` on every fuzz directory with a nightly "pinned to match the version of Rust used on OSS-Fuzz" ([main.yml](https://github.com/bytecodealliance/wasmtime/blob/main/.github/workflows/main.yml)) **C** |
| Deno | Integration since 2026-03-02, but `fuzzing_engines: none` with an `indexer` target | Not fuzzed through it ([project.yaml](https://github.com/google/oss-fuzz/blob/master/projects/deno/project.yaml)) **C** |
| V8, rustls, tokio, serde_json, quick-xml, servo, image-rs, SQLite, DuckDB, QuickJS | Yes | Project directories exist in `google/oss-fuzz/projects` **C** |
| hyper, h2, `url`, html5ever, lol-html, Firecracker, gVisor, Playwright | No project directory | **C** |

**What comparable Rust projects run on each pull request:**

- rustls runs CIFuzz with `fuzz-seconds: 150` ([cifuzz.yml](https://github.com/rustls/rustls/blob/main/.github/workflows/cifuzz.yml)) **C**.
- serde_json, tokio and image-rs only check that their fuzz targets build (`cargo fuzz check`) **C**.

**cargo-fuzz.**

- Version 0.13.2, 2026-06-09 ([crates.io](https://crates.io/crates/cargo-fuzz)) **C**.
- It works "only on x86-64 and Aarch64, and only on Unix-like operating systems (not Windows)", and needs a nightly compiler ([README](https://github.com/rust-fuzz/cargo-fuzz/blob/main/README.md)) **C**.
- Its subcommands include `cmin` (corpus minimisation), `tmin` (test-case minimisation) and `coverage`. Its sanitizers are address, leak, memory and thread, and a "careful" mode rebuilds the standard library with extra checks ([options.rs](https://github.com/rust-fuzz/cargo-fuzz/blob/main/src/options.rs)) **C**.

**ClusterFuzzLite** ([running](https://github.com/google/clusterfuzzlite/blob/main/docs/running_clusterfuzzlite.md), [GitHub Actions](https://github.com/google/clusterfuzzlite/blob/main/docs/running-clusterfuzzlite/github_actions.md), [build integration](https://github.com/google/clusterfuzzlite/blob/main/docs/build_integration.md)):

- **Platform.** It "supports libFuzzer targets built with Clang on Linux" and builds in Docker. For Rust, as on OSS-Fuzz, only the `address` sanitizer is supported **C**.
- **Modes.**

  | Mode | Purpose | Documented default or example |
  |---|---|---|
  | `code-change` | Fuzz a pull request; stop at the first crash | 600 s; starts from the batch corpus if one exists |
  | `batch` | Fuzz every target for a set time; build the corpus | Example: `fuzz-seconds: 3600`, cron every 6 hours; "should be run on a schedule such as once daily" |
  | `prune` | Minimise the corpus | Daily; "mandatory" when batch fuzzing |
  | `coverage` | HTML coverage report from the batch corpus | Daily; lets `code-change` run only affected targets |
  | continuous builds | Keep `main` builds so a pull request reports only new crashes | On push to `main` |

  **C**
- **Options.** `parallel-fuzzing` (default off, one core), `minimize-crashes` (default off), `report-unreproducible-crashes` (default off), and `output-sarif` for GitHub code scanning **C**.
- **Storage.** Corpora and crashes go to a "storage repo" through a personal access token, or else to GitHub Actions artifacts **C**. A crashing input is uploaded as an artifact **C**.
- **Maintenance.** The repository's last push was 2026-02-12, with 34 open issues and one tag (`v1`) ([GitHub API](https://api.github.com/repos/google/clusterfuzzlite)) **C**. Its run action is `using: 'docker'` with `image: 'docker://gcr.io/oss-fuzz-base/clusterfuzzlite-run-fuzzers:v1'` ([action.yml](https://github.com/google/clusterfuzzlite/blob/main/actions/run_fuzzers/action.yml)) **C**.

**GitHub limits that size fuzzing jobs.**

- **Runners.** A standard Linux runner has 4 CPUs and 16 GB of RAM on public repositories, 2 CPUs and 8 GB on private ones ([GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)) **C**.
- **Job length.** A job runs at most 6 hours ([limits](https://docs.github.com/en/actions/reference/limits)) **C**.
- **Retention.** Artifacts and logs are kept 90 days by default, settable from 1 to 90 days for public repositories ([retention](https://docs.github.com/en/organizations/managing-organization-settings/configuring-the-retention-period-for-github-actions-artifacts-and-logs-in-your-organization)) **C**.
- **Who sees what.** Downloading an artifact needs read access to the repository ([download artifacts](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts)) **C**. Code scanning alerts need write access, while pull-request annotations are visible with read access ([code scanning alerts](https://docs.github.com/en/code-security/concepts/code-scanning/code-scanning-alerts)) **C**.

**Keeping fuzz reports private.**

- gVisor runs syzkaller continuously, with a public dashboard (`syzkaller.appspot.com/gvisor`) **C**. Its reports go to a "**private** syzkaller bug tracking list" because "automated fuzzing reports may contain undisclosed vulnerabilities" ([GOVERNANCE.md](https://github.com/google/gvisor/blob/master/GOVERNANCE.md)) **C**.

**Generators and oracles kept apart from the fuzzer.**

- Wasmtime keeps its "test case generators and oracles" in the `wasmtime-fuzzing` crate, "fuzzer-agnostic", with fuzz targets as "a couple of lines of glue code" ([contributing-fuzzing.md](https://github.com/bytecodealliance/wasmtime/blob/main/docs/contributing-fuzzing.md)) **C**.
- Its oracles include differential ones, for example running a module with and without optimisations and comparing the results **C**.

### Fit for Goodfetch

- **OSS-Fuzz later, ClusterFuzzLite now.** Goodfetch has no users before its first release, so the OSS-Fuzz criterion is not met yet. ClusterFuzzLite gives the same libFuzzer + ASan builds today, and an OSS-Fuzz application can follow once V1 has users. The STYLE decoders are what OSS-Fuzz looks for: CDP messages, JSON-RPC and plugin-protocol framing, HTML and Markdown, sitemaps and run-state import.
- **Fuzzing time per tier**, mapped onto ADR 0032's tiers. The per-tier numbers are the projects' documented defaults, not measured needs.

  | ADR 0032 tier | What runs | Source of the figure |
  |---|---|---|
  | Every pull request | `cargo fuzz check` of every target. The committed corpus and every past crasher replay as ordinary tests on every OS, through the plain harness described below. `code-change` fuzzing of the affected targets on Linux | ClusterFuzzLite default 600 s; rustls uses 150 s |
  | Merge queue and `main` | The same replay on the full matrix; continuous builds for `code-change` | — |
  | Nightly | `batch` fuzzing, then `prune` and `coverage` | ClusterFuzzLite example 3,600 s every 6 hours; prune and coverage daily |
  | Release candidate | Nothing extra is documented by these projects | — |

- **Replay on Windows and macOS through a plain harness.** cargo-fuzz cannot build on Windows. Following Wasmtime, each decoder's fuzz body can live in a library function that a normal `#[test]` also calls on the stored corpus. The regression then runs on every OS without sanitizers, and libFuzzer adds the sanitizers on Linux.
- **Nightly toolchain.** Fuzzing needs nightly, while the workspace builds on a stable MSRV. Wasmtime's answer is a nightly pinned to the one OSS-Fuzz uses.
- **Three conflicts with ADRs as written:**
  - **Image pinning.** ADR 0032 pins every Action by SHA, but the ClusterFuzzLite action pulls a mutable image tag (`:v1`). Pinning it means running the images by digest from Goodfetch's own workflow rather than through the action.
  - **Public crashes.** On a public repository, batch-fuzzing crashes stored as artifacts or in a storage repository are readable by anyone with read access, which is everyone. Keeping batch fuzzing in the private repository ADR 0032 already has, as gVisor keeps its reports private, would avoid that. `code-change` crashes are tied to a pull request that is already public.
  - **Egress deny.** Fuzz jobs run product code, so ADR 0043's deny applies to them. Image pulls and corpus restores come before the deny, and corpus uploads after it.
- **The Script sandbox is covered upstream.** V8 itself is on OSS-Fuzz. What Goodfetch adds is a target for its own boundary: the pipe protocol between the engine and the Script sandbox worker, and the decoding of catalog-call arguments.

---

## 3. Sandbox and confinement test suites

### Facts

**Chromium** ([GitHub mirror](https://github.com/chromium/chromium)).

- **Linux.** `sandbox/linux/seccomp-bpf/bpf_tests.h` defines `BPF_TEST_C`, which "runs a test function in a sub-process, under a seccomp-bpf policy … without failing on configurations that are allowed to not support seccomp-bpf in their kernels" **C**. `BPF_DEATH_TEST_C` also asserts "the nature of death" **C**.
- **Linux baseline policy tests** ([baseline_policy_unittest.cc](https://github.com/chromium/chromium/blob/main/sandbox/linux/seccomp-bpf-helpers/baseline_policy_unittest.cc)) **C**:
  - **Allowed calls:** `BaselinePolicyBasicAllowed`, `CreateThread`, `Socketpair`, `GetRandom`.
  - **Calls that fail with an error:** `EPERM_open`, `EPERM_socket`, `EPERM_connect`, `EPERM_bind`, `EPERM_listen`, `ForkErrno`.
  - **Calls that kill the process:** a `TEST_BASELINE_SIGSYS` line per forbidden call, among them `ptrace`, `keyctl`, `init_module`, `chroot`, `mq_open` and `fanotify_init`.
- **Windows** ([validation_tests/suite.cc](https://github.com/chromium/chromium/blob/main/sandbox/win/tests/validation_tests/suite.cc)) **C**:
  - The validation suite runs inside the sandbox. It covers the file system, the registry, the desktop and alternate desktops, windows, process access between lock-down levels, threads, and a memory limit.
  - It pairs the denial with a positive control: `TestMemoryLimit` ("an over-limit allocation will be denied") next to `TestMemoryNoLimit` ("a large allocation will succeed absent limits").
- **macOS.** `sandbox/mac` has `seatbelt_unittest.cc` and `seatbelt_exec_unittest.cc` **C**.
- **Checking the sandbox status.** Chromium's `chrome://sandbox` page is built for Windows, Android, Linux and ChromeOS, not macOS ([BUILD.gn](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/webui/sandbox/BUILD.gn)) **C**. On Linux, `/proc/<pid>/status` shows a process's seccomp mode and `NoNewPrivs` ([kernel docs](https://github.com/torvalds/linux/blob/master/Documentation/filesystems/proc.rst)) **C**.
- **Ubuntu 23.10 and later.** Ubuntu's AppArmor restriction on unprivileged user namespaces breaks Chromium's "primary sandbox" for builds without a matching AppArmor profile ([Chromium doc](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md)) **C**. The documented fixes, from widest to narrowest, are turning off `apparmor_restrict_unprivileged_userns`, writing a profile for the binary, or using the setuid helper **C**. `--no-sandbox` "should never be used when browsing the open web" **C**.

**Deno** ([tests/specs/permission](https://github.com/denoland/deno/tree/main/tests/specs/permission), [SECURITY.md](https://github.com/denoland/deno/blob/main/.github/SECURITY.md)).

- **Security model.** "All JavaScript run in Deno is considered untrusted," and "all runtime I/O is considered to be privileged and must always be guarded by a runtime permission" **C**.
- **Regression tests for 2026 network bypasses.** Each is a spec test with expected output **C**:

  | Spec test | First commit | What it proves |
  |---|---|---|
  | `deny_net_numeric_hostname`, `…_subnet` | 2026-04-09 (#33203) | `--deny-net=127.0.0.1` also blocks `2130706433` and `0x7f000001` |
  | `deny_net_fetch_resolved_ip`, `deny_net_websocket_resolved_ip` | 2026-05-27 (#34236) | `fetch` and `WebSocket` to `localhost` are refused after resolution; the `fetch` test also asserts "no socket was opened to the denied IP" |
  | `deny_import_resolved_ip` | 2026-08-12 (#36486) | Imports are checked against the resolved IP |
  | `deny_net_node_tcp_wrap_bind_resolved_ip` | 2026-09-08 (#36482) | The Node compatibility layer's `bind` is checked against the resolved IP |

- **The matching advisories**, all published 2026-05-27 ([Deno advisories](https://github.com/denoland/deno/security/advisories)) **C**:
  - GHSA-cpgj-f7g3-2pp2, "`fetch()` API sandbox bypass via missing DNS resolution check";
  - GHSA-83pc-3rw9-qpwj, the WebSocket equivalent;
  - GHSA-v8fw-85r8-5m23, numeric hostname aliases in `TCPWrap`;
  - GHSA-8xpq-cjcf-3wh9, "Permission Bypass via Unicode Normalization Mismatch on macOS (APFS)", high;
  - GHSA-968w-xfqw-vp9q, path traversal through `package.json`'s `main` that bypasses `--allow-read`.

  Earlier ones include GHSA-f27p-cmv8-xhm6 (2025-01-06), "Authorization headers not dropped when redirecting cross-origin", and GHSA-7w8p-chxq-2789 (2025-06-03), where `Deno.env.toObject()` ignored `--deny-env` **C**.

**gVisor.**

- **The same test natively and in the sandbox.** "This is a test suite for Linux system calls. It runs under both gVisor and Linux." Each test file yields a `native` target and a target per gVisor platform (systrap, ptrace, KVM), and new tests should pass on Linux first ([test/syscalls/README.md](https://github.com/google/gvisor/blob/master/test/syscalls/README.md)) **C**.
- **Fuzzing seccomp programs.** `test/secfuzz` does "fuzz-based testing of seccomp-bpf programs". Its `DiffFuzzer` "fuzzes two seccomp programs" against each other ([secfuzz.go](https://github.com/google/gvisor/blob/master/test/secfuzz/secfuzz.go)) **C**.

**Firecracker** ([tests/integration_tests/security](https://github.com/firecracker-microvm/firecracker/tree/main/tests/integration_tests/security)).

- **Seccomp filter tests generated from the filter file.** `test_seccomp_validate.py` reads the `seccompiler` JSON filter. For every rule, it runs a helper that makes the call with the allowed arguments (must succeed), then with each argument changed (must fail) **C**.
- **Seccomp on by default.** `test_seccomp.py` asserts "the seccomp filter denies an unallowed syscall", that a filter is installed by default, and that `--no-seccomp` installs none **C**.
- **Jailer tests.** `test_jail.py` covers an empty jailer ID, a destination that is a symlink or a hard link, the chroot layout, cgroups v1 and v2, default and custom resource limits, positive and negative file-size limits, and a new PID namespace **C**.
- **Dependency audit.** `test_sec_audit.py` runs `cargo audit` **C**.

**Wasmtime.**

- **What counts as a vulnerability.** The project publishes "What is considered a security vulnerability?" ([doc](https://github.com/bytecodealliance/wasmtime/blob/main/docs/security-what-is-considered-a-security-vulnerability.md)) **C**:
  - Anything that undermines the sandbox, any memory unsafety in the host, and a denial of service while *executing* (for example, fuel that never yields) count as vulnerabilities.
  - Wrong results that stay inside the sandbox do not.
- **Miri.** CI runs a Miri matrix over several crates and spec-test scripts ([main.yml](https://github.com/bytecodealliance/wasmtime/blob/main/.github/workflows/main.yml)) **C**.
- **Terminal output.** Wasmtime filters escape sequences when untrusted output goes to a terminal ([security.md](https://github.com/bytecodealliance/wasmtime/blob/main/docs/security.md)) **C**.

**V8's sandbox** ([src/sandbox/README.md](https://github.com/v8/v8/blob/main/src/sandbox/README.md)).

- **Threat model.** The sandbox "assumes that an attacker can arbitrarily and concurrently modify any memory inside the sandbox address space" **C**.
- **Testing mode.** Testing needs a build with `v8_enable_memory_corruption_api = true`, run with `--sandbox-testing` or `--sandbox-fuzzing` **C**:
  - The build exposes a `Sandbox` object whose `MemoryView` reads and writes inside the sandbox.
  - A crash filter separates harmless crashes from sandbox violations. The filter is "currently only available on Linux".
  - `--sandbox-testing` exits 0 on a harmless crash. `--sandbox-fuzzing` exits non-zero and needs the corruption API compiled in.
- **Regression tests.** `test/mjsunit/sandbox` holds 68 entries, many named `regress-<bug>.js` and run with `// Flags: --sandbox-testing` **C**.
- **Goodfetch's build path.** The `v8` crate's build script reads `GN_ARGS` and `EXTRA_GN_ARGS` when building from source (`V8_FROM_SOURCE`) ([build.rs](https://github.com/denoland/rusty_v8/blob/main/build.rs)). Its last release is v152.2.0, 2026-08-20 **C**.

**GitHub-hosted runners' kernels** (runner image READMEs, images of 2026-09-27 to 2026-10-04):

| Image | Kernel |
|---|---|
| Ubuntu 22.04 | 6.8.0-1064-azure |
| Ubuntu 24.04, x64 and arm64 | 6.17.0-1022-azure |
| Ubuntu 26.04 | 7.0.0-1012-azure |

([runner-images](https://github.com/actions/runner-images/tree/main/images/ubuntu)) **C**. ADR 0033's Landlock ABI 6 needs kernel 6.12 or later.

### Fit for Goodfetch

- **Generate the refusal cases from the policy.** ADR 0033 lists the refusal cases by hand. Firecracker's pattern derives them from the same `seccompiler` filter, and from the same manifest that generates the Seatbelt profile. Then a rule added to the policy without a test, or a test that no longer matches the policy, fails CI.
- **Pair each refusal with a positive control.** Chromium (`TestMemoryNoLimit`) and gVisor (native then sandboxed) show the pattern. Each probe first succeeds outside confinement, then fails inside it. Without the control, a probe that fails for an unrelated reason, such as a missing file or a typo in a socket family, passes as a refusal.
- **Assert the exact failure.** ADR 0033 makes a refused call return an error rather than kill the Plugin. Chromium's split between `EPERM_*` and `SIGSYS` tests is the template: the test checks the error code, so a filter that starts killing the process, or allowing the call, fails.
- **Deno's bypass history is a ready list of cases for ADR 0017's resolved-IP check:**
  - numeric aliases, checked both before and after resolution;
  - every transport (`fetch`, WebSocket, Node `bind`, imports);
  - "no socket was opened" as the assertion, which ADR 0033's engine refusals can reuse.

  The APFS Unicode-normalization bypass applies to the macOS Plugin store and to Export paths (§4).
- **Test the Script sandbox worker as if V8 were already broken.** ADR 0051's threat model is that a V8 bug "hands the script whatever process it runs in". Two layers can be tested:
  - **The V8 sandbox:** a test build of V8 with `v8_enable_memory_corruption_api = true`, run with `--sandbox-testing`, exercises it through the `v8` crate's `EXTRA_GN_ARGS`.
  - **The OS confinement around the worker:** ADR 0033's OS-confinement probes, run as native code inside a test variant of the worker process, check it directly, without depending on an exploit.

  Whether `--sandbox-testing` works under `--jitless` was not checked.
- **Test that the browser's sandbox is really on.** ADR 0017 keeps Chromium's sandbox on, and a browser that silently loses it still passes every functional test:
  - On Linux, a test can read `Seccomp:` and `NoNewPrivs:` from each renderer's `/proc/<pid>/status`.
  - On Windows, it can read `chrome://sandbox`.
  - On macOS there is no such page.

  On Ubuntu 24.04 runners, the Goodfetch Chromium build (ADR 0035) needs its own AppArmor profile, because Ubuntu's profile covers only `/opt/google/chrome/chrome`. The same restriction applies to the engine's own use of user namespaces for Plugins.
- **Both Linux paths are on free runners.** Ubuntu 22.04 (kernel 6.8) exercises ADR 0033's path without Landlock ABI 6, and 24.04 (kernel 6.17) the ABI 6 path, if Landlock is enabled in that kernel (not verified).
- **A published vulnerability definition.** Wasmtime's page shows how one makes triage of fuzzing and external reports predictable. For example, is a Script sandbox time limit that never fires a vulnerability? Wasmtime's answer for fuel is yes.

---

## 4. SSRF, header injection and path traversal test sets

### Facts

**OWASP SSRF Prevention Cheat Sheet** ([source](https://github.com/OWASP/CheatSheetSeries/blob/master/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.md), last changed 2026-10-04) **C**:

- **Rebinding.** "Domain allowlisting alone does not prevent DNS rebinding." Validate the resolved addresses, then make sure "the HTTP client connects only to validated addresses".
- **Parser disagreement.** "Treat parser disagreement as a rejection." Its example is `http://example.com\@evil.com`: WHATWG reads the host as `example.com`, while CPython's `urllib.parse` reads it as `evil.com`.
- **Redirects.** Disable redirect following in the client, or re-check each hop.
- **Deny list.** When a deny list is unavoidable, its minimum is: AWS, GCP and Azure metadata (`169.254.169.254`, `metadata.google.internal`); `127.0.0.0/8`, `0.0.0.0/8` and `::1/128`; RFC 1918; `fc00::/7`; `fe80::/10`; multicast. It points to the IANA special-purpose registries for the rest.

**IANA special-purpose registries** ([IPv4](https://www.iana.org/assignments/iana-ipv4-special-registry/), [IPv6](https://www.iana.org/assignments/iana-ipv6-special-registry/), read 2026-10-13). They add blocks the OWASP minimum omits **C**:

- IPv4: `100.64.0.0/10` (shared address space), `192.0.0.0/24`, `198.18.0.0/15` (benchmarking), `240.0.0.0/4`, `255.255.255.255/32`.
- IPv6: `::ffff:0:0/96` (IPv4-mapped), `64:ff9b::/96` and `64:ff9b:1::/48` (IPv4-IPv6 translation), `2002::/16` (6to4), `2001::/32` (Teredo), `::/128`.

**Cloud metadata.** AWS documents the metadata service at `http://169.254.169.254/` and, over IPv6, `http://[fd00:ec2::254]/` ([AWS](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/instancedata-data-retrieval.html)) **C**.

**Public payload corpora.**

- **PayloadsAllTheThings, SSRF** (MIT, pushed 2026-08-27) ([README](https://github.com/swisskyrepo/PayloadsAllTheThings/blob/master/Server%20Side%20Request%20Forgery/README.md)) **C**. Its bypass families:
  - IPv6 forms (`[::]`, `[0000::1]`, `[::ffff:127.0.0.1]`);
  - names that resolve to loopback (`localtest.me`, `*.127.0.0.1.nip.io`, `.localhost`, `ip6-localhost`);
  - the rest of `127.0.0.0/8`;
  - short forms (`0`, `127.1`);
  - decimal, octal and hex encodings;
  - URL encoding and enclosed alphanumerics;
  - redirects with 307 and 308;
  - DNS rebinding;
  - parser discrepancies (`http://127.1.1.1:80\@127.2.2.2:80/`, `http://1.1.1.1 &@2.2.2.2# @3.3.3.3/`);
  - non-HTTP schemes (`file`, `dict`, `gopher`, `ldap`, `tftp`, `sftp`, `jar`, `netdoc`).
- **rbndr** (GPL-3.0, last push 2020): a name server that answers with a random choice of the addresses in the queried name and a very low TTL ([README](https://github.com/taviso/rbndr)) **C**.
- **NCC Group's Singularity** (MIT, pushed 2026-07-21): reports about 3 s to rebind Chromium, Firefox and Safari with its "Multiple answers" strategy against `0.0.0.0` ([README](https://github.com/nccgroup/singularity)) **C**.

**Measured parsing** **M**. Rust `url` 2.5.8, built locally on macOS arm64, with Node 25.5.0's WHATWG `URL` giving the same hosts:

| Input host | Parsed host |
|---|---|
| `0x7f000001`, `2130706433`, `0177.0.0.1`, `127.1`, `0x7f.1`, `127.0.0.1.`, `127.0.0.1%2e`, `①②⑦.⓪.⓪.①` | `127.0.0.1` |
| `0` | `0.0.0.0` |
| `[::ffff:127.0.0.1]` | IPv6 `::ffff:127.0.0.1` (`url`); `[::ffff:7f00:1]` (Node) |
| `127.1.1.1:80\@127.2.2.2:80` | `127.1.1.1` |
| `1.1.1.1 &@2.2.2.2# @3.3.3.3` | `2.2.2.2` |

In Rust, `IpAddr::to_canonical` (stable since 1.75.0) turns an IPv4-mapped IPv6 address into IPv4, and `is_global` is still nightly-only (`#![feature(ip)]`, issue #27709) ([std docs](https://doc.rust-lang.org/std/net/enum.IpAddr.html)) **C**.

**Prior art in a direct competitor.** Firecrawl added "safeFetch SSRF guard regression tests" to its server CI on 2026-09-30 (#4864) ([test-server.yml](https://github.com/firecrawl/firecrawl/blob/main/.github/workflows/test-server.yml), [safeFetch.proxy.test.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/utils/safeFetch.proxy.test.ts)) **C**. The tests include:

- loopback IPv4 and bracketed IPv6 literals refused "before dispatch";
- the guard "re-applie[d] … when the redirect interceptor follows a Location";
- "never opens a proxy tunnel to a private IP literal".

Its Playwright service now sends browser traffic through a local proxy that refuses internal hosts ([api.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/playwright-service-ts/api.ts)) **C**.

**Header injection.**

- **The `http` crate (1.5.0) refuses control characters** ([value.rs](https://github.com/hyperium/http/blob/master/src/header/value.rs)) **C**:
  - `HeaderValue::from_str` allows "only visible ASCII characters (32-126) and horizontal tab".
  - `from_bytes` allows bytes 32–255 except 127, plus tab.
  - Both reject `\n`.
  - Only the `unsafe` `from_maybe_shared_unchecked` skips the check, and only in release builds.
- **Credentials across redirects.** `reqwest` (0.13.5) removes `Authorization`, `Cookie`, `Cookie2`, `Proxy-Authorization` and `WWW-Authenticate` when a redirect changes host, port or scheme ([redirect.rs](https://github.com/seanmonstar/reqwest/blob/master/src/redirect.rs)) **C**. Deno shipped this bug as GHSA-f27p-cmv8-xhm6 **C**.
- **File names in headers.** ASVS 5.0 V5.4.2: file names served in headers must be "encoded or sanitized (e.g., following RFC 6266)" ([ASVS 5.0 V5](https://github.com/OWASP/ASVS/blob/master/5.0/en/0x14-V5-File-Handling.md)) **C**.

**Path traversal in file outputs.**

- **Absolute paths.** `PathBuf::push`: "If `path` is absolute, it replaces the current path"; on Windows, a rooted path without a prefix replaces all but the prefix ([std docs](https://doc.rust-lang.org/std/path/struct.PathBuf.html#method.push)) **C**.
- **Windows names** ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file), updated 2025-04-11) **C**:
  - The reserved names are `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9` and the superscript-digit forms (`COM¹`), even with an extension (`NUL.txt`).
  - A name must not end in a space or a period.
  - `C:tmp.txt` is relative to drive C's current directory.
  - `\\?\` turns off path parsing.
  - Characters 1–31 are allowed in alternate data streams.
- **Archive crates.** RustSec advisories in 2025–2026 ([advisory-db](https://github.com/rustsec/advisory-db/tree/main/crates)) **C**:
  - `tar` RUSTSEC-2026-0067, "`unpack_in` can chmod arbitrary directories by following symlinks" (fixed in 0.4.45);
  - `zip` RUSTSEC-2025-0168, "Incorrect path canonicalization during Archive Extraction Leading to Arbitrary File Write";
  - PAX-header desynchronisation in `tokio-tar` (RUSTSEC-2025-0111, no fix) and `astral-tokio-tar` (RUSTSEC-2026-0145).
- **A capability-based filesystem.** `cap-std` (4.0.3, Bytecode Alliance) "features protection against CWE-22 … Path Traversal" and against opening `/proc/self/mem` ([README](https://github.com/bytecodealliance/cap-std)), and has a fuzz target over path tokens **C**.
- **ASVS 5.0.** V5.3.2 says to build file paths from "internally generated or trusted data"; V5.3.3 says decompression "ignores user-provided path information to prevent … zip slip" **C**.

### Fit for Goodfetch

- **The test set, one table, run at three layers:**
  - the engine's resolved-IP check, as unit and property tests;
  - the HTTP tier and the egress proxy against local fixtures;
  - browser Sessions on Proving ground pages, for every request type ADR 0014 lists: navigation, subresources, workers, WebSockets, WebRTC and `javascript:`.

  | Family | Cases |
  |---|---|
  | Encodings | Decimal, hex, octal, short, mixed, trailing dot, `%2e`, enclosed alphanumerics; the parser already normalises these (**M** above), so the cases guard against a second parser (a Plugin's, a Sink's, a proxy's) disagreeing |
  | IPv6 | `::`, `::1` spellings, IPv4-mapped, NAT64, 6to4 embedding `127.0.0.1` (`2002:7f00:1::`), Teredo, `fd00:ec2::254`, link-local with zone |
  | Ranges | Every IANA block above, not only OWASP's minimum; `0.0.0.0` (Singularity's target) |
  | Names | `localhost`, `*.localhost`, `ip6-localhost`, a name resolving to each blocked block, a name with both a public and a private answer |
  | Redirects | 301, 302, 303, 307, 308 to each blocked target; `Refresh` header; meta refresh; script navigation; redirect to `file:`, `data:`, `javascript:`, `gopher:` |
  | Rebinding | TTL-0 alternation and mixed answers, with "no connection reached the fixture" as the assertion (Deno's oracle) |
  | Proxy path | A private literal or name through an upstream proxy (ADR 0017's proxy rule; Firecrawl's 2026-09-30 tests) |
  | Plugin traffic | Every case through ADR 0033's per-instance proxy, where a run's allowlist never applies |

- **Rebinding needs a local name server.** The public tools (1u.ms, rbndr, nip.io) are external, and ADR 0043's CI deny would block them. A small local DNS fixture serving rebinding and mixed answers keeps the cases inside CI.
- **Header injection is mostly closed by the types.** The `http` crate already refuses CR and LF. The tests left are:
  - values that reach the browser through CDP rather than `http` (Recipe headers, `Fetch.continueRequest`); whether Chromium validates them was not checked;
  - Sink and webhook header settings;
  - `Content-Disposition` on the Console's file `GET`s (ADR 0031);
  - the rule that a Secret bound to a domain (ADR 0014) never follows a cross-origin redirect: Deno's GHSA-f27p, as a Goodfetch regression test.
- **Path traversal:**
  - **Where it can happen:** Export file names, Key-value store keys, screenshot and Run trace paths, and Plugin bundle unpacking. ADR 0033 already refuses traversal, links and device files when unpacking a Plugin.
  - **Cases:** absolute paths (`PathBuf::push` replaces), `..`, Windows reserved names and superscript digits, a trailing dot or space, `C:` drive-relative paths, `\\?\` and UNC paths, alternate data streams, symlinks and hard links in archives (Firecracker's jailer tests and the `tar` advisory), and Unicode normalisation on APFS (Deno's GHSA-8xpq).
  - **A structural fix:** `cap-std` keeps every open inside one directory handle, so the test checks a property of the API rather than of each call site.

---

## 5. Dynamic scanning of the HTTP and WebSocket surfaces, and authentication tests

### Facts

**ZAP** (stable 2.17.0, 2025-12-15; [baseline](https://github.com/zaproxy/zaproxy-website/blob/main/site/content/docs/docker/baseline-scan.md), [API scan](https://github.com/zaproxy/zaproxy-website/blob/main/site/content/docs/docker/api-scan.md)).

- **Baseline scan.** It spiders for 1 minute by default, then runs passive rules only, so it "doesn't perform any actual 'attacks'". Findings are WARN unless a config file makes them FAIL. It is "intended to be ideal to run in a CI/CD environment" **C**.
- **Exit codes.** 0 success, 1 at least one FAIL, 2 WARN only, 3 other failure **C**.
- **API scan.** It imports "OpenAPI, SOAP, or GraphQL" definitions, then runs an active scan tuned for APIs **C**.
- **WebSockets.** ZAP's WebSocket add-on can "intercept and show", set breakpoints, "fuzz WebSocket messages", and "passively scan WebSocket messages and raise alerts with scripts" ([introduction](https://github.com/zaproxy/zap-extensions/blob/main/addOns/websocket/src/main/javahelp/org/zaproxy/zap/extension/websocket/resources/help/contents/introduction.html)) **C**.
- **Header rules.** Its passive rules include "Content Security Policy (CSP) Header Not Set" (10038), the CSP analyser (10055) and the anti-clickjacking header (10020) **C**.
- **Authentication.** Header-based authentication goes through `ZAP_AUTH_HEADER`, `ZAP_AUTH_HEADER_VALUE` and `ZAP_AUTH_HEADER_SITE` ([docs](https://github.com/zaproxy/zaproxy-website/blob/main/site/content/docs/getting-further/authentication/handling-auth-yourself.md)) **C**.
- **Calls home.**
  - The baseline script runs `-addonupdate` unless `-silent` is among the ZAP options ([zap-baseline.py](https://github.com/zaproxy/zaproxy/blob/main/docker/zap-baseline.py)) **C**.
  - `-silent` "ensures ZAP does not make any unsolicited requests, including check for updates" ([cmdline](https://github.com/zaproxy/zaproxy-website/blob/main/site/content/docs/desktop/cmdline.md)) **C**.
  - On 2025-10-21 the ZAP team reported that a new rule had "triggered a Check For Updates call even when the silent mode was used", fixed in a later add-on release ([blog](https://github.com/zaproxy/zaproxy-website/blob/main/site/content/blog/2025-10-21-zap-was-not-so-silent/index.md)) **C**.
- **The GitHub Action.** `zaproxy/action-baseline` (v0.15.0) defaults to the image `ghcr.io/zaproxy/zaproxy:stable` and to `allow_issue_writing: true` ([action.yml](https://github.com/zaproxy/action-baseline/blob/master/action.yml)) **C**.

**Nuclei** (v3.11.1, 2026-08-08; templates v10.5.0, 2026-10-05; both MIT; [README](https://github.com/projectdiscovery/nuclei/blob/dev/README.md)).

- **Template types.** `dns`, `file`, `http`, `headless`, `tcp`, `ssl`, `websocket`, `code` and `javascript` **C**.
- **DAST mode.** `-dast` runs fuzzing templates. The `dast/vulnerabilities` set includes `ssrf`, `crlf`, `lfi`, `redirect`, `cmdi`, `ssti` and `xss` ([nuclei-templates](https://github.com/projectdiscovery/nuclei-templates/tree/main/dast/vulnerabilities)) **C**.
- **Inputs and outputs.** Input modes are `list`, `burp`, `jsonl`, `yaml`, `openapi`, `swagger` and `http`. `-se` exports SARIF **C**.
- **Calls home and signing.** `-duc` disables the automatic update check. The out-of-band server defaults to public hosts (`oast.pro`, `oast.live`, and others), unless `-iserver` points to a self-hosted interactsh (v1.4.1, MIT) or `-ni` disables it. `-dut` refuses unsigned templates **C**.
- **The GitHub Action.** `projectdiscovery/nuclei-action` v3 defaults `version` to `latest` ([README](https://github.com/projectdiscovery/nuclei-action)) **C**.

**Schemathesis** (v4.31.0, 2026-10-12, MIT) "tests OpenAPI and GraphQL APIs by generating inputs from your schema", including stateful sequences ([README](https://github.com/schemathesis/schemathesis)) **C**.

**MCP.**

- **The specification** ([2026-07-28, Streamable HTTP](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/streamable-http.mdx), [authorization security](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/authorization/security-considerations.mdx)) **C**:
  - Servers "**MUST** validate the `Origin` header on all incoming connections", answering 403 to an invalid one.
  - Local servers "**SHOULD** bind only to localhost".
  - A server "**MUST** only accept tokens specifically intended for themselves" and "**MUST NOT** pass through the token it received".
- **The conformance suite** ([modelcontextprotocol/conformance](https://github.com/modelcontextprotocol/conformance), npm `@modelcontextprotocol/conformance` 0.1.16, 2026-10-01) **C**:
  - It tests a server by URL.
  - Its `dns-rebinding` scenario sends non-localhost `Host` and `Origin` and requires an HTTP 4xx ([dns-rebinding.ts](https://github.com/modelcontextprotocol/conformance/blob/main/src/scenarios/server/dns-rebinding.ts)).
  - It has an `auth` suite and a mock authorization server.

**OWASP ASVS 5.0.0** (2025-05-30; [V4](https://github.com/OWASP/ASVS/blob/master/5.0/en/0x13-V4-API-and-Web-Service.md)) **C**:

- V4.4.2: during the WebSocket handshake, "the Origin header field is checked against a list of origins allowed".
- V4.4.3 and V4.4.4: dedicated WebSocket tokens are obtained through the authenticated HTTPS session.
- ASVS 5.0 also adds a chapter, V17, for WebRTC.

**Chrome's Local Network Access.** Chrome 142 (stable 2025-10-28) puts requests from public sites to local or loopback addresses behind a permission prompt ([Chrome blog](https://developer.chrome.com/blog/local-network-access), [Chrome 142](https://developer.chrome.com/release-notes/142)) **C**.

### Fit for Goodfetch

- **No scanner covers the protocol itself, so a schema-driven fuzzer would.** The HTTP listener (ADR 0027) and the Console's WebSocket (ADR 0031) carry JSON-RPC over the Operation catalog. None of ZAP's API scan, Schemathesis or Nuclei can import that. The catalog's committed JSON Schema bundle (ADR 0038) can feed a `proptest` generator in Goodfetch's own suite:
  - every operation, valid and mutated arguments, sent over each transport;
  - asserting a typed error and no state change;
  - the same generator driving a state-machine test through `proptest-state-machine`.
- **One endpoint test table across every listener.** The listeners are the Console, the HTTP listener, the ops listener, MCP over HTTP, the Live view, raw CDP and the Attached engine's private endpoint. ADRs 0017, 0024, 0031 and 0050 set the rules; a test per rule per listener checks them:
  - **Tokens:** none, wrong, expired, revoked, from another listener, and passed in a query string, which must be refused, since query strings reach logs.
  - **Host and Origin:** a foreign `Host`, and a foreign or missing `Origin` on the WebSocket handshake (ASVS V4.4.2; the MCP rule).
  - **Console web protections:** `SameSite=Strict` and `HttpOnly`, the CSRF token on the `POST` fallback, and rate-limited sign-in.
  - **Program tokens:** a named token used outside its operations, Recipes, runs or Datasets.
  - **The ops listener:** `/livez` and `/readyz` return a status and nothing else; `/metrics` refuses the protocol token if only a scoped read-only token is valid.
  - **Live view grants:** single-use, bound to `frame-ancestors`, revoked when the Session ends (ADR 0031).
  - **Remote MCP:** token audience and no passthrough, as the MCP specification requires.
- **Off-the-shelf scanners where they fit:**
  - **MCP conformance suite:** its server scenarios, `dns-rebinding` included, run against the MCP over HTTP surface whenever MCP code changes.
  - **ZAP baseline:** a nightly run against the Console catches header and cookie regressions beyond ADR 0031's own header check. It authenticates by setting `ZAP_AUTH_HEADER=Cookie`. Settings: image by digest, `allow_issue_writing: false`, `-silent`.
  - **Nuclei:** Goodfetch-owned self-contained templates for each fixed endpoint bug, with `-duc -ni`, templates pinned and signed (`-dut`).
- **Scanners and the egress canary.** Both scanners phone home by default, which ADR 0043's canary would flag. ZAP's 2025 `-silent` bug is the kind of regression the canary exists to catch.
- **Local Network Access is not a defence to rely on.** It protects only Chrome users who have not granted the permission. The ADR 0017 checks must hold without it, and the Proving ground's origin-escape pages (ADR 0031) can include a public page calling the local Daemon.

---

## 6. Secrets: scanning test outputs, and property tests of redaction

### Facts

- **The `secrecy` crate.** In 0.10.3, `SecretBox`'s `Debug` prints `SecretBox<T>([REDACTED])` ([lib.rs](https://github.com/iqlusioninc/crates/blob/main/secrecy/src/lib.rs)) **C**.
- **`proptest`** (1.12.0) persists failing cases under `proptest-regressions/`, and "it is recommended to check these files in" so CI replays them ([book](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/failure-persistence.md)) **C**.
- **Playwright traces.** On issue [#19992](https://github.com/microsoft/playwright/issues/19992), a maintainer (2024-04-03) wrote that every approach to filtering secrets from traces was a "heuristics/best effort approach", since an application can log a password "in altered forms" **C**. The advice was to guard trace access like a password. In 2026, users still ask for masking ([#38673](https://github.com/microsoft/playwright/issues/38673), open) **C**.
- **A user's measurement on the same issue** (2026-09-19) **C**:
  - 9 planted secrets produced 35 occurrences across 6 files of one trace.
  - A text-level redactor broke the trace's JSON.
  - Redacting only string values with a list of structural fields to keep left 3 occurrences.
- **Playwright MCP.** Its tests include "secrets are redacted in console log artifact" (issue #41509), "redacts url parameter values" (playwright-mcp #1791) and "empty secret value is ignored" ([secrets.spec.ts](https://github.com/microsoft/playwright/blob/main/tests/mcp/secrets.spec.ts), Playwright 1.64.0 released 2026-10-07) **C**.
- **GitHub masking.** `add-mask` prevents "a string or variable from being printed in the log", and must run before the value is printed ([workflow commands](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands)) **C**. The page says nothing about artifacts.
- **GitHub secret scanning.** It covers the git history on all branches, issues, pull requests, discussions, wikis and secret gists, and "runs automatically for free" on public repositories ([about secret scanning](https://docs.github.com/en/code-security/secret-scanning/introduction/about-secret-scanning)) **C**. Actions logs and artifacts are not listed.
- **Secret scanners:**

  | Tool | Version | Licence | Modes | Note |
  |---|---|---|---|---|
  | gitleaks | 8.30.1 (2026-03-21) | MIT | `git`, `dir`, `stdin`; `--redact`, `--baseline-path` | [README](https://github.com/gitleaks/gitleaks) |
  | gitleaks-action | 3.0.0 (2026-05-30) | Not MIT since v2 | — | A licence key is "required for Organizations" ([README](https://github.com/gitleaks/gitleaks-action)) |
  | TruffleHog | 3.99.2 (2026-10-08) | AGPL-3.0 | — | — |
  | detect-secrets | 1.5.0 (2024-05-06) | Apache-2.0 | — | — |

  **C**
- **Secrets leaking through error messages.** Firecrawl's GHSA-3p54-jg6f-68r8 (CVE-2026-72904, 2026-07-29, fixed in 2.11.32): a user-supplied JSON Schema `$ref` was dereferenced against the worker's filesystem. File contents came back to the user inside a persisted error message, exposing "environment variables and application secrets" ([advisory](https://github.com/firecrawl/firecrawl/security/advisories/GHSA-3p54-jg6f-68r8)) **C**.

### Fit for Goodfetch

- **Exact canaries as the oracle, as the reset gate already does.** ADR 0001's reset gate plants canaries and scans after a run is discarded. The same canaries can be scanned for during and after a live run, in every output Goodfetch writes:
  - **Logs and traces:** logs, Run traces, HAR files, console captures.
  - **Records of the run:** the Audit log, the Effect journal, checkpoints (outside the encrypted fields), Exports, Notifications.
  - **What Clients see:** JSON-RPC errors, and error messages persisted in run state (Firecrawl's CVE-2026-72904 path).
  - **Test outputs:** `insta` snapshots, and test reports and logs in CI.

  The search looks for each canary and for the encodings Goodfetch itself produces: JSON-escaped, URL-encoded, base64, hex. Playwright's maintainers are right that arbitrary transformations cannot be caught, which is why the oracle is limited to Goodfetch's own encodings.
- **Plant canaries in every kind of secret, through the Proving ground's login flows.** The kinds are a typed password, a TOTP seed, a proxy password inside a proxy URL's userinfo, a model API key, a webhook signing secret, and Session cookie values. This covers the paths ADR 0014 names, such as screenshots masked and secrets typed at the moment of input.
- **Property tests.** A `proptest` strategy generates secret values meant to break redaction:
  - quotes, backslashes, newlines and `%`;
  - Unicode, very long values, and values equal to common words;
  - empty values (Playwright MCP's case);
  - values that are substrings of each other.

  It pushes them through each formatter and serializer, and asserts the value is absent. Failing cases go to `proptest-regressions/` in git.
- **gitleaks as a second net, not the oracle.** The `gitleaks` binary (MIT) in `dir` mode runs over CI logs and artifacts after each job. The Action is not an option: it needs a licence key for organisation repositories, and ADR 0032 moves the repository into an organisation.

---

## 7. Prior art: what Deno, Playwright, Browserbase, Firecrawl and Apify publish

| Project | Disclosure channel | Attestations | Published security tests | Source |
|---|---|---|---|---|
| Deno | GitHub private advisories; legal safe harbour | — | Permission spec tests, one per bypass (§3); OSS-Fuzz integration with no fuzzing engine (§2) | [SECURITY.md](https://github.com/denoland/deno/blob/main/.github/SECURITY.md) **C** |
| Playwright | Microsoft MSRC (`SECURITY.md` standard block) | — | MCP tests for secret redaction, request blocking (allowed and blocked origins and hosts) and HTTP transport lifecycle | [SECURITY.md](https://github.com/microsoft/playwright/blob/main/SECURITY.md), [tests/mcp](https://github.com/microsoft/playwright/tree/main/tests/mcp) **C** |
| Browserbase | Not stated on the page | "SOC 2 Type II Certified", "HIPAA compliant", "comprehensive third-party penetration testing with successful results" (no dates or vendor) | None public. Each browser runs "in a dedicated VM"; "after each session, the virtual machine is killed and recreated from scratch" | [security guide](https://docs.browserbase.com/guides/security) **C** |
| Firecrawl | Bugcrowd engagement, with a form for those without an account | SOC 2 Type II ([enterprise docs](https://docs.firecrawl.dev/enterprise)) | SSRF regression tests in server CI (2026-09-30); 4 repository advisories, 2 in 2026 | [security](https://www.firecrawl.dev/security), [advisories](https://github.com/firecrawl/firecrawl/security/advisories) **C** |
| Apify | `security@apify.com`; "a vulnerability disclosure policy, not a bug bounty program"; acknowledgement within 5 business days | SOC 2 Type II | None public. Priority areas include "Actor sandbox escape" and "cross-tenant data exposure"; out of scope: "automated scanning" and "missing HTTP security headers" | [platform security](https://docs.apify.com/platform/security), [disclosure policy](https://docs.apify.com/security/vulnerability-disclosure) **C** |

- **Firecrawl's 2026 advisories:**
  - GHSA-7843-ffpq-5f9p (CVE-2026-72905, critical, 2026-07-29): change tracking's `git-diff` mode passed page content to a shell. It was fixed by "replacing shell-based diff generation with an in-process diff implementation" in 2.11.31.
  - GHSA-3p54-jg6f-68r8: the `$ref` file read (§6).

  Its earlier SSRF advisories, CVE-2024-56800 and CVE-2025-57818, are already in ADR 0017 **C**.
- **What users report.** Playwright users have asked since 2023 for secrets to be kept out of traces (§6) **C**. Separately, a search summary says Firecrawl's self-hosting guide turns API authentication off in its quickstart ([self-host](https://docs.firecrawl.dev/contributing/self-host)); the page itself was not read.

### Fit for Goodfetch

- **Nobody publishes a suite to copy.** None of the five publishes a security test suite that Goodfetch could adopt whole. The useful parts are Deno's one-test-per-bypass rule and Firecrawl's late-added SSRF regression tests. Firecrawl's 2026 advisories give two more cases:
  - **No shell for diffs.** Change tracking compares page content (Goodfetch's Baselines and Watches), so no comparison should spawn a shell or an external diff.
  - **No external `$ref`.** User-supplied JSON Schemas (extraction schemas) must not resolve external `$ref`s. A test with `$ref` to `file:///etc/passwd`, to a URL and to a private address checks it.
- **A published definition and policy.** Apify's out-of-scope list (automated scanning, missing headers) and Wasmtime's vulnerability definition (§3) show what a Goodfetch `SECURITY.md` might state. Goodfetch's Exposed Daemon and Goodfetch Cloud make the scope question real.

---

## 8. Unverified

- Whether Landlock is enabled in the `6.17.0-azure` kernel of GitHub's Ubuntu 24.04 runners, so that ADR 0033's ABI 6 path actually runs there.
- Whether V8's `--sandbox-testing` and the memory corruption API work in a `--jitless` build, and whether the `v8` crate builds with `v8_enable_memory_corruption_api = true` through `EXTRA_GN_ARGS`.
- Whether anonymous visitors, or only signed-in users, can download Actions artifacts from a public repository; the docs say "read access".
- Whether GitHub secret scanning covers Actions logs or artifacts; the page does not list them.
- Whether Chromium validates header values passed through CDP (`Network.setExtraHTTPHeaders`, `Fetch.continueRequest`) against CR and LF.
- What the OSS-Fuzz `indexer` target in Deno's integration does; read only from `project.yaml`.
- Which Chrome versions extend Local Network Access to WebSocket (reported as Chrome 147 by a search summary, not read in release notes).
- Firecrawl's statement that its self-host quickstart disables authentication; taken from a search summary.
- Browserbase's and Apify's SOC 2 report periods and penetration-test scope; only available through their trust centres.
- The Playwright trace measurement in §6 comes from one user's comment, not from the Playwright team.
- Alibaba Cloud's and other providers' metadata addresses were not checked against their docs; only AWS's were.
