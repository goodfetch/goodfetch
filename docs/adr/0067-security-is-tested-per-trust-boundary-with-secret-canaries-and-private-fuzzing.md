# Security is tested per trust boundary, with Secret canaries as the oracle and fuzzing kept private

Goodfetch's security tests are organised by **trust boundary**, the places where untrusted input or code meets something worth protecting, and each boundary has its tests, their suite and an oracle. Refusals are generated from the policy they test, assert the exact error and pair with a positive control. Leaks are found with **Secret canaries**, planted fake secrets searched for in every output. Fuzzing runs only in a private repository, on every push to `main` and nightly, so no crash is public before its fix; the public suites replay the committed corpus and every past crasher. Five reviews stay manual and are listed as manual tests.

The ADRs already require `cargo-fuzz` per decoder (`TESTING.md`) and Miri (`STYLE.md`), Plugin confinement CI on every OS (ADR 0033), the reset gate's planted cookies, tokens and secret values (ADRs 0001 and 0015), the CI's Egress canary (ADR 0043), Console header checks and origin-escape pages (ADR 0031), and supply-chain and signing gates (ADRs 0018, 0030, 0032, 0039). What they did not settle is how those suites are shaped, what else runs, where fuzzing runs and how its crashes are handled, and what stays manual. Comparable projects show the shapes:
- **Policy-generated, exact, controlled refusals.** Firecracker generates its seccomp tests from its `seccompiler` filter; Chromium asserts `EPERM` or `SIGSYS` per call and pairs a denial with a positive control; gVisor runs each syscall test natively, then sandboxed.
- **One regression test per bypass.** Deno added a spec test for each 2026 network-permission bypass, asserting that "no socket was opened to the denied IP". Firecrawl added SSRF regression tests on 2026-09-30.
- **Private fuzz reports.** gVisor keeps them private "because automated fuzzing reports may contain undisclosed vulnerabilities".
- **No scanner covers the protocol.** ZAP's API scan and Schemathesis import OpenAPI or GraphQL only, and nothing actively scans a WebSocket. The MCP conformance suite does test MCP over HTTP.
- **Redaction is best effort.** Playwright's maintainers call trace filtering "heuristics/best effort", and Playwright MCP keeps one redaction regression test per leak.

Decided in [Security testing](https://github.com/goodfetch/goodfetch/issues/288) (grilling ticket), on the facts of [Security testing](https://github.com/goodfetch/goodfetch/issues/273) ([findings](https://github.com/goodfetch/goodfetch/blob/research/security-testing/rnd/research/2026-10-13-security-testing.md)). The agent answered the grilling on the owner's delegation. The rules every test follows are in `TESTING.md`.

**Suites and tiers.** [ADR 0061](0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md) defines three suites, each containing the one before: the **Quick suite**, the **Pre-merge suite** (every pull request, on Linux) and the **Release suite** (the full matrix: the merge queue, nightly and every release; on manual dispatch before each milestone's owner test and before any release while the merge queue does not exist). Every OS before a merge therefore comes from the Release suite in the merge queue. Checks with an input that moves without a commit are **Live checks**, in no suite ([ADR 0064](0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md)).

## Decision

### Trust boundaries

`TESTING.md` lists the boundaries, each with its tests, their suite and the OSes they run on: untrusted decoders; the Script sandbox; Plugin confinement; Chromium's sandbox; egress; listening endpoints and the Daemon group's peer link; secrets; file outputs; the Audit log's hash chain; the reset gate; supply chain and workflows. A boundary added by a later decision gets its row in the same change.

### Fuzzing

- **Targets.** Every decoder of input Goodfetch did not write itself has a `cargo-fuzz` target. `TESTING.md` keeps the list: protocol framing (JSON-RPC, the plugin protocol, the peer protocol, the Script sandbox's pipe protocol, catalog-call arguments), page content (CDP messages, HTML and Markdown, sitemaps, robots.txt and Site preference files), files from elsewhere (run-state import, Checkpoint and session-export bundles, Plugin manifests and OCI image indexes, Plugin bundle unpacking, Recipes and settings files, Filter lists, Session source formats including `Cookies.binarycookies`, extraction schemas), and URL canonicalisation with the resolved-IP classification.
- **Oracles.** No crash, no sanitizer error, no hang past the target's limit; plus round-trip or differential checks where a pair exists, such as the resolved-IP classification against a reference table built from the IANA registries.
- **Replay on every OS.** Each target's body is a plain function. A `#[test]` replays the committed seeds and every past crasher on every OS, since cargo-fuzz does not build on Windows (Wasmtime's pattern).
- **In the suites.** The replay is in the Quick suite. `cargo fuzz check` of every target is in the Pre-merge suite, on Linux x64.
- **Fuzzing itself is a Live check** ([ADR 0064](0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md)), since its corpus and libFuzzer's seed move without a commit. It never runs in a pull request: ClusterFuzzLite's `code-change` mode reports a crash that already exists on `main` unless continuous builds tell it apart, and a public pull request's logs and artifacts would then disclose it.
- **Where it runs.** All fuzzing runs in a private repository of the organization, against the main repository's `main`, with ClusterFuzzLite:
  - **Continuous builds:** every push to `main` builds and stores the fuzz targets.
  - **Push fuzzing:** each push then runs `code-change` for 600 s on the targets it affects, chosen from the latest coverage report, or on every target when no report exists yet.
  - **Nightly:** `batch` for 3,600 s over every target, then `prune` and `coverage`.
  - **Containment:** crashes, corpora, coverage, logs and results stay in that repository. They never reach ADR 0064's results store, which becomes public with the main repository, nor its published report. The job that builds and runs the code holds no secret and a token with no permissions; a later job stores its results.
- **Cost and phases.** Runner minutes in a private repository are paid beyond the plan's included minutes, so fuzzing starts when the owner approves it as a paid test resource. Until then, the replay and `cargo fuzz check` run, and nothing else.

  > Amended by [ADR 0076](0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md): paid fuzzing minutes, GitHub Secret Protection while private and the independent assessment are declined while there is no revenue; fuzzing stays within the included minutes, `gitleaks` stands in until the repository is public, and the test V8 build runs on the owner's machine.
- **Images by digest.** ClusterFuzzLite's images run by digest from Goodfetch's own workflow, never through its action's mutable `:v1` tag. They are pulled before ADR 0043's deny, and the fuzz jobs run under it.
- **Crashes.**
  - A crash whose input also crashes the previous continuous build is a known one: it joins its advisory, or opens one if it has none.
  - A new crash opens a private security advisory draft on the main repository, never a public issue. Repository security advisories exist only on public repositories, so until the main repository is public, a crash opens an issue in the private fuzz repository instead. Either replaces ADR 0064's `live-check` issue.
  - Once the main repository is public, the fix lands from the advisory's private fork under `SECURITY.md`'s coordinated disclosure (ADR 0030); before that, through an ordinary pull request, which is private too. The crasher, minimised with `cargo fuzz tmin`, is committed with the fix as a regression input.
- **What it blocks.** An open fuzz crash (its advisory draft, or its issue in the private repository) blocks the promotion of the next minor until it is fixed, or until triage (manual test 2) closes it as not a vulnerability. Patches are not held, since a patch may carry the fix. **No private result is ever posted on a public commit**: a status or check there, or two of them side by side, would let anyone infer when a crash exists.
  - The release pipeline reads the private results itself, when each step starts. For a release candidate or a patch, it reads the Private component run for that commit (ADR 0032). For promotion to `x.y.0`, it reads that run and whether any fuzz crash is open. A red run or an open crash stops the step, and the public log says only that a private gate holds it.
  - So a crash found during the beta week still holds the minor. This leaves one accepted inference: a published candidate whose public gates are green, held at promotion by a private gate, can only mean an open fuzz crash, and the end of the hold dates its fix. It cannot be removed, since a minor that misses its date with every public gate green says the same, and it reveals neither the target nor the input, only during the beta week.
  - It reads them with the **private gates App**, a GitHub App with read-only access: the private repositories' workflow runs and issues, and, once the main repository is public, its repository security advisories, so that it sees open advisory drafts. The App's private key is a secret of the release pipeline's protected environment, held only by the job that reads the gates.
- **Corpus.** Seeds and a dictionary are committed per target. The grown corpus stays in the private repository.
- **Toolchain.** Fuzzing and the ASan run use one nightly toolchain pinned in its own file, apart from `rust-toolchain.toml`, since both need `-Zsanitizer`.
- **OSS-Fuzz.** Its rule is "a significant user base and/or" critical infrastructure, so the owner applies once Goodfetch is published and has users. Its fuzzing then replaces the private batch run. OSS-Fuzz keeps its reports private until fixed.
- **Sanitizers beyond fuzzing.** The Release suite runs the tests of every crate that opts into `unsafe` under AddressSanitizer, on Linux glibc, covering the FFI and system-call modules Miri cannot run.

### Confinement

- **Plugin confinement (changes ADR 0033).** Cases are generated from each generated policy (seccomp, Landlock, Seatbelt, AppContainer and the broker's filters), one probe per rule at each network level; a rule without a probe, or a probe without a rule, fails. A call the seccomp, Landlock or Seatbelt policy refuses is checked for the exact error ADR 0033 gives, so a filter that starts killing the process, or allowing the call, fails; resource-limit probes expect the limit's own Error code. Each probe first succeeds unconfined in the same test. Each runner asserts the mechanism it expects (Landlock ABI, user namespaces, the mount-namespace fallback), so a silent fallback fails: Ubuntu 22.04 covers the path without Landlock ABI 6, and 24.04 and 26.04 the ABI 6 path. A container under Docker's default seccomp profile on a kernel before 6.12 is refused with `confinement_unavailable`. ADR 0033's hand-written engine refusals stay, beside the generated cases.
- **The Script sandbox, as if V8 were already broken (adds to ADR 0051).**
  - **OS layer:** ADR 0033's probes run under exactly the policy generated for the worker. A live worker is also inspected: `Seccomp:` and `NoNewPrivs:` in `/proc/<pid>/status` on Linux, an AppContainer token on Windows, and on macOS the method the spec finds.
  - **V8 layer:** in the Release suite, on Linux x64, a test build of V8 with `v8_enable_memory_corruption_api` runs corruption scripts through the worker. Each must end as `script_crashed` and reach nothing outside it.
  - **The test V8 is built once per V8 version, outside the merge queue.**
    - A build job, in no suite, runs on a pull request that changes the V8 version. It skips when a test build of that version is already published.
    - It is its own required check, not part of the Pre-merge suite's "all green" check, so it does not count against that check's 20-minute budget.
    - The job that builds holds no secret and a token with no permissions. A later job attests the build and publishes it as a release asset, as the Goodfetch Chromium is (ADR 0035). Release assets do not expire, unlike workflow artifacts and caches, so every later patch of that line can still fetch it.
    - The pull request's author commits the published digest beside the version, and a check refuses a digest that does not match the attested asset.
    - A pull request from a fork gets no `id-token` to attest with, so a V8 change from a fork is rebuilt from a branch of the main repository.
    - The Release-suite job fetches the build by digest before ADR 0043's deny and only runs the corruption scripts, so it fits ADR 0061's job budgets.
    - A V8 change cannot merge until its test build exists. The build job's runner minutes beyond the plan's included minutes are a paid test resource the owner approves.
  - **If V8 refuses the build under `--jitless`,** the question returns to the owner. It is never skipped.
- **Chromium's sandbox (adds to ADRs 0017 and 0049).**
  - **Linux:** every renderer of Chrome for Testing and of the Goodfetch Chromium build shows `Seccomp: 2` and `NoNewPrivs: 1`, on Ubuntu 24.04 with its AppArmor restriction on.
  - **Windows:** `chrome://sandbox`, read through CDP, shows every process sandboxed.
  - **macOS:** the renderer runs under Seatbelt, by the method the spec finds.
  - **The startup probe of ADR 0049** is tested in its refuse and record cases.
- **Suites.** These tests run on Linux in the Pre-merge suite and on every OS in the Release suite, so on every OS before each merge once the merge queue exists.

### Egress and hostile input

- **One SSRF case table**, kept as data and run at every layer, with "no connection reached the target" as the oracle (the fixture records every connection) plus the typed refusal.
  - **Families:** encodings; IPv6 forms (IPv4-mapped, NAT64, 6to4, Teredo, zone IDs); every IANA special-purpose block that is not globally reachable; names with one public and one private answer; redirects of every status and kind, and to `file:`, `data:`, `javascript:` and `gopher:`; rebinding; the upstream proxy path; Plugin traffic.
  - **Layers:** the resolved-IP classification in-process (property tests against a reference table); the HTTP tiers and the egress proxy; browser Sessions for every request type ADR 0014 lists; raw CDP through the egress proxy; the relays of ADRs 0042 and 0044; a Borrowed browser, where the test asserts the rebinding gap is recorded in Provenance (ADR 0056); Sinks and webhooks under Goodfetch Cloud's "no private endpoints" policy.
- **Rebinding** uses a DNS fixture in the Proving ground serving TTL-0 alternation and mixed answers. End to end it runs on Linux, in a network and mount namespace whose resolver points at the fixture. In-process it runs on every OS, with a scripted sequence of answers.
- **The IANA special-purpose registries** are committed as data with their date. A watch job, which runs none of the product, opens an issue when IANA's files change.
- **Hostile input beside SSRF:**
  - header values with CR, LF or NUL from Recipes, Sinks, webhooks and CDP;
  - a Secret bound to a domain never following a cross-origin redirect;
  - path traversal into Export file names, Key-value store keys, screenshot and Run trace paths, Plugin bundles and page-suggested download names, with each OS's cases (absolute paths, `..`, Windows reserved names, trailing dot or space, `\\?\`, UNC, alternate data streams, links in archives, APFS Unicode normalisation);
  - extraction schemas with a `$ref` to a file, a URL or a private address, refused;
  - no comparison of page content spawning a process, checked under a seccomp filter that refuses `execve`;
  - decompression bombs, endless bodies, deep JSON nesting and oversized messages, each ending in its typed limit error.

### Endpoints

- **One endpoint table:** a rule per row, a listener per column. The listeners are the Console, the Daemon's socket and WebSocket, the HTTP listener, the ops listener, MCP over HTTP, the Live view, raw CDP, the Attached engine's private endpoint, an Exposed Daemon and the Daemon group's peer link.
- **The rows** come from ADRs 0017, 0031, 0049, 0050 and 0055:
  - tokens: missing, wrong, expired, revoked, scoped elsewhere, from another listener, or in a query string;
  - a foreign `Host`, and a foreign or missing `Origin` on the request and on the WebSocket handshake;
  - plain HTTP where TLS is configured, and `X-Forwarded-*` from an unconfigured address;
  - the Console's cookie flags, CSRF token, sign-in rate limit and step-up;
  - the ops probes' empty answers and `/metrics`' scoped token;
  - Live view grants;
  - the Exposed Daemon's bootstrap link;
  - the peer link refusing another group's certificate authority, a revoked Group member or an expired certificate;
  - remote MCP's token audience and no passthrough.

  Each refusal is also checked in the Audit log.
- **Protocol fuzzing from the schema.** The committed JSON Schema bundle drives `proptest` and `proptest-state-machine` over every operation and transport. Every invalid call gets a typed error and changes no state.
- **Scanners:**
  - The MCP conformance suite runs against MCP over HTTP, scheduled as [ADR 0070](0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md) decides.
  - A ZAP baseline scan of the Console runs in the Release suite, on Linux: image by digest, `-silent`, `allow_issue_writing: false`, its FAIL rules committed, its crawl bounded by depth rather than by time so that it stays pinned, and under ADR 0043's deny, so its Egress canary catches a call home.
  - Nuclei and Schemathesis are not used.

### Secrets

- **Secret canaries** (the reset gate's planted cookies, tokens and secret values are Secret canaries too) are planted through the Proving ground's login flows and through every Secret provider, for every kind of secret: a typed password, a TOTP seed, a proxy password, a model API key, a webhook signing secret, a Sink credential, a Session cookie, an endpoint token, the run data key.
- **Where they are searched, during and after a run,** raw and in the encodings Goodfetch produces (JSON-escaped, URL-encoded, base64, hex):
  - logs, Run traces, HAR files and console captures, and screenshots and recordings where masking applies;
  - the Audit log, the Effect journal, and Checkpoints outside their encrypted fields;
  - Exports and Notifications;
  - JSON-RPC errors, error messages kept in run state, and model requests;
  - the Console's offline cache, `goodfetch doctor`'s bug-report file, and the SDKs' `repr`, `toString` and error messages.
- **Redaction property tests.** Generated secret values that are hard to redact go through each formatter and serializer. Failing cases are committed in `proptest-regressions/`.
- **CI outputs.** The `gitleaks` binary, pinned by checksum, scans each job's logs, test reports, snapshots and artifacts before they upload, with the Secret canaries as a rule. A hit fails the job. This is a step of every CI job, not a suite test.
- **Commits.** Commit scanning covers every committed file, Model recordings (ADR 0065) included. Once the repository is public, GitHub secret scanning and push protection are on, free there. Until then, the Pre-merge suite runs `gitleaks` over each pull request's commits; GitHub's paid Secret Protection on the private repository waits for the owner's approval as a paid test resource.

### Other rules

- **Audit log tamper evidence (ADR 0016).** Verification fails on any edit, deletion, reordering or truncation of entries or of the chain's tail, including a Daemon group's single chain. These are property tests.
- **One regression test per bypass.** Every security fix, Goodfetch's own or a class a comparable project disclosed, gets a test at the layer it slipped through.
  - A watch job lists new advisories from a fixed set of repositories and opens an issue.
  - A person triages the issue.
- **A security test never skips where it is scheduled.** The OSes a test runs on are written in `TESTING.md`'s table, and a test absent from an OS the table does not give it is not skipped. On a host where it is scheduled, a test never returns early, is never ignored at run time, and, where the host lacks a mechanism, asserts the refusal (`confinement_unavailable`, `local_address_unavailable`) instead. A missing tool fails the job.
- **Static analysis.** CodeQL scans Rust, TypeScript, Python and the workflows in the Pre-merge suite once the repository is public (code scanning is free only there), and a high or critical alert blocks the merge.

### Placement

| Part | Placement |
|---|---|
| Corpus and crasher replay; in-process SSRF and hostile-input cases; redaction property tests; Audit log tamper tests; the check that every policy rule has a probe | Quick suite |
| `cargo fuzz check`; confinement, Script sandbox, Chromium sandbox, end-to-end egress, hostile-input and endpoint suites on Linux; MCP conformance over HTTP where ADR 0070 places it; CodeQL once public, `gitleaks` over commits until then | Pre-merge suite |
| Each suite above on every OS its row in `TESTING.md` gives it; the ASan run (Linux glibc); the V8 memory-corruption scripts (Linux x64, on the build fetched by digest); the ZAP baseline (Linux) | Release suite |
| Push and nightly fuzzing, in a private repository | Live check (ADR 0064), with its own results and blocking rule |
| The test V8 build | a build job on the pull request that changes the V8 version, in no suite |
| The IANA and advisory watch jobs | nightly tier, running none of the product |
| `gitleaks` over each job's outputs | a step of every CI job |

Secret canary searches belong to the tests that produce the outputs, in whichever suite those run.

**Before the repository is public** (ADR 0032): the Quick and Pre-merge suites run on Linux pull requests as above; the Release suite, with every security part of it, runs on manual dispatch before each milestone's owner test and before any release; the watch jobs start with the nightly tier once the repository is public; CodeQL and GitHub's secret scanning wait for it too; fuzzing waits for the owner's approval of its minutes.

### Manual tests

Only these stay manual:
1. **Threat-model review**, on each minor's release candidate. A person walks the trust-boundary table against the minor's changes and confirms each new decoder, listener, Plugin kind, widening setting, Secret kind and output has its test. The result goes on the release pull request.
2. **Vulnerability triage**, on each new fuzz crash, external report and advisory-watch issue. A person judges it against `SECURITY.md`'s vulnerability definition and either opens the private advisory or closes the issue with a reason.
3. **Independent security assessment**, before 1.0 and before each later major. This is a paid test resource the owner approves. Each finding becomes a fix and a regression test.
4. **Confinement where CI cannot run**, on each minor's release candidate:
   - WSL1's refusal;
   - Bridge Sinks under Docker Desktop and Podman machine on macOS and Windows;
   - the host-firewall recipes the docs give for `direct`, which need a person's approval on the host.
5. **Passkeys on real authenticators**, on each minor's release candidate: sign-in, step-up and recovery on a platform authenticator and a hardware key. Automated tests use a virtual authenticator.

## Considered Options

- **Security tests organised per tool:** rejected. Boundaries no tool reaches, such as the Script sandbox and the peer link, would go untested.
- **OSS-Fuzz now:** not open to Goodfetch until it has users.
- **All ClusterFuzzLite modes in the public repository:** rejected. Batch crashes, logs and artifacts on `main` would be public before their fix.
- **`code-change` fuzzing in public pull requests, with batch fuzzing private:** rejected. Without continuous builds a pull request reports crashes already on `main`, and with them its job still has to download `main`'s build and compare in public logs. Wasmtime, serde_json, tokio and image-rs also run only `cargo fuzz check` on pull requests.
- **A hand-written fuzz cron:** rejected. ClusterFuzzLite already stores, prunes, deduplicates and reports coverage.
- **ClusterFuzzLite's action as published:** rejected. It pulls a mutable image tag that SHA pinning does not cover.
- **Replay through cargo-fuzz only:** rejected. It skips Windows, where path handling differs most.
- **ADR 0033's hand list of refusals alone:** rejected. It drifts from the policy, and a probe that fails for an unrelated reason passes as a refusal.
- **Trusting V8's upstream fuzzing for the Script sandbox:** rejected. It tests V8, not the worker's confinement around it.
- **Off-the-shelf scanners as the endpoint suite:** rejected. None imports JSON-RPC or actively scans a WebSocket.
- **Nuclei for endpoint regressions:** rejected. Its templates would repeat the endpoint table in a second format, and its out-of-band checks use public servers by default.
- **A heuristic secret scanner as the oracle:** rejected. It misses Goodfetch's own encodings and raises false alarms. Secret canaries are exact, and gitleaks stays a second net.
- **gitleaks' Action:** rejected. It needs a licence key in organisations; the MIT binary does the same scan.
- **Public tools for DNS rebinding (rbndr, nip.io):** rejected. ADR 0043's deny blocks them, and a local fixture keeps the cases in CI.
- **Skipping a security test at run time where the host lacks a mechanism:** rejected. A skipped refusal test is how a silent weakening ships.
- **More manual review:** rejected. Each item left manual is one no tool can do.

## Consequences

- **Amends `TESTING.md` and `STYLE.md`:**
  - `TESTING.md`'s untrusted-input rule (moved there from `STYLE.md` by ADR 0062) becomes "every decoder of input Goodfetch did not write itself", replayed on every OS, with the list under Security.
  - `STYLE.md`'s "a unit test proves the secret is not printed" becomes Secret canaries and redaction property tests, and its Miri rule points to the AddressSanitizer run.
- **Changes ADR 0033:** its "CI on every OS" OS-confinement cases are generated from the policy, check the exact error, carry a positive control and assert the mechanism in use. The hand-listed engine refusals stay. ADR 0033 carries a one-line note.
- **Adds to ADR 0032:**
  - `cargo fuzz check` and the security suites join the pull-request tier on Linux, CodeQL once public, and `gitleaks` over commits until then;
  - `gitleaks` over outputs joins every job;
  - the ASan run, the V8 memory-corruption scripts and the ZAP baseline join the Release suite;
  - the test V8 build job runs on each pull request that changes the V8 version, as its own required check, and publishes the build as an attested release asset whose digest is committed;
  - the release pipeline reads the Private component run, and for promotion also open fuzz crashes, from the private repositories through the read-only private gates App, and nothing private is posted on public commits; ADR 0032 carries a one-line note, since its list of the beta week's gates is closed;
  - fuzzing runs in a private repository, under the owner's approval of its minutes;
  - every container image a workflow runs is pinned by digest, as Actions are pinned by SHA;
  - the IANA and advisory watch jobs run none of the product, so they sit outside ADR 0043's rule, like the watchlist job.
- **Adds to [ADR 0064](0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md)'s Live checks table** a fuzzing row: owned by area 4; tier: every push to `main` and nightly, in the private repository; what it blocks: the promotion of the next minor while a fuzz crash is open; decided in ADR 0067.
- **Amends ADR 0064 for fuzzing alone.** Its results stay in the private repository, never in the results store, and a crash goes to a private advisory draft, or to an issue in the private repository before the main one is public, never to a `live-check` issue. ADR 0064 carries a one-line note.
- **Adds to ADR 0043:** the test V8 build joins CI's pinned artifact mirror, as a CI-only entry that the Release-suite job fetches by digest before the deny. It is not one of the product's pinned downloads.
- **Adds to ADR 0030:** `SECURITY.md` publishes what counts as a vulnerability: anything that crosses a trust boundary of `TESTING.md`'s table, any memory unsafety, and a limit that never fires. A new fuzz crash follows its coordinated disclosure, and an open one blocks the promotion of the next minor, as above.
- **Adds to ADRs 0017, 0049 and 0051:** the tests of the resolved-IP check, Chromium's sandbox and the Script sandbox worker described above.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)). It gains:
  - the jobs above;
  - the private repository's fuzzing workflows;
  - the digest pin check for images;
  - repository settings for secret scanning, push protection and code-scanning merge protection once public.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/goodfetch/goodfetch/issues/152)). The Proving ground gains:
  - the DNS rebinding fixture;
  - a fixture that records every connection;
  - redirect, header, download-name and decompression-bomb pages;
  - login flows that plant Secret canaries;
  - a public page that calls the local Daemon, beside the origin-escape pages.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/goodfetch/goodfetch/issues/213)):
  - its fuzzing line gains the Script sandbox's pipe protocol and catalog-call arguments;
  - Seam 2 gains the worker's OS-layer inspection.
- **Areas 4 (Security and egress) and 7 (Plugin protocol)** build the suites of their boundaries by these rules. ADR 0037's rows for areas 1, 2 and 4 list this ADR.
- **The "Test infrastructure" spec** builds:
  - the policy-to-probe generator;
  - the SSRF and hostile-input tables and their runners;
  - the connection-recording and DNS fixtures with the namespace setup;
  - the endpoint table's runner;
  - the Secret canary planting and search helpers;
  - the fuzz replay harness;
  - the watch jobs.
- **The security jobs' time budgets** are those of ADR 0061. The test V8 build job is in no suite, so no suite budget covers it.
- **Left to other tickets of the map:**
  - the Console's test stack and its virtual authenticator ([Console and docs test stack](https://github.com/goodfetch/goodfetch/issues/282));
- **Decided since:** the MCP test stack, including where MCP conformance runs, in [ADR 0070](0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md). Prompt-injection and fake-success pages in [ADR 0065](0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). The Journey catalog in [ADR 0063](0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md) gives no separate manual-test IDs. The five manual security tests are not Journeys. Tests 1 to 3 are reviews, not product use. Tests 4 and 5 run on each minor's release candidate on hosts and hardware CI lacks, and have no automated step, while a Journey's `manual` steps are listed for a milestone's owner test and leave the variant proved on its automated steps. They are named by title and recorded on the release pull request.
- **Unverified, checked by the first implementation tickets:**
  - whether Landlock is enabled in GitHub's Ubuntu runner kernels;
  - whether V8's memory-corruption API works under `--jitless`;
  - how to read a macOS renderer's Seatbelt state;
  - whether Chromium validates header values passed through CDP.

  A negative answer returns to the owner as a decision, never as a skipped test.
