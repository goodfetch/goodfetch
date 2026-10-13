# Security is tested per trust boundary, with canaries as the oracle and fuzzing split between public and private CI

Goodfetch's security tests are organised by **trust boundary**, the places where untrusted input or code meets something worth protecting, and each boundary has a suite, a tier and an oracle. Refusals are generated from the policy they test, assert the exact error and pair with a positive control. Leaks are found with **Secret canaries**, planted fake secrets searched for in every output. Fuzzing runs per pull request in the public repository and in batch in the private one, so a crash on `main` stays private until it is fixed. Five reviews stay manual and are listed as manual tests.

The ADRs already require `cargo-fuzz` per decoder and Miri (`STYLE.md`), Plugin confinement CI on every OS (ADR 0033), reset-gate canaries (ADRs 0001 and 0015), the CI egress canary (ADR 0043), Console header checks and origin-escape pages (ADR 0031), and supply-chain and signing gates (ADRs 0018, 0030, 0032, 0039). What they did not settle is how those suites are shaped, what else runs, where fuzzing runs and how its crashes are handled, and what stays manual. Comparable projects show the shapes:
- **Policy-generated, exact, controlled refusals.** Firecracker generates its seccomp tests from its `seccompiler` filter; Chromium asserts `EPERM` or `SIGSYS` per call and pairs a denial with a positive control; gVisor runs each syscall test natively, then sandboxed.
- **One regression test per bypass.** Deno added a spec test for each 2026 network-permission bypass, asserting that "no socket was opened to the denied IP". Firecrawl added SSRF regression tests on 2026-09-30.
- **Private fuzz reports.** gVisor keeps them private "because automated fuzzing reports may contain undisclosed vulnerabilities".
- **No scanner covers the protocol.** ZAP's API scan and Schemathesis import OpenAPI or GraphQL only, and nothing actively scans a WebSocket. The MCP conformance suite does test MCP over HTTP.
- **Redaction is best effort.** Playwright's maintainers call trace filtering "heuristics/best effort", and Playwright MCP keeps one redaction regression test per leak.

Decided in [Security testing](https://github.com/Jbmanllr/automan/issues/288) (grilling ticket), on the facts of [Security testing](https://github.com/Jbmanllr/automan/issues/273) ([findings](https://github.com/Jbmanllr/automan/blob/research/security-testing/rnd/research/2026-10-13-security-testing.md)). The agent answered the grilling on the owner's delegation. The rules every test follows are in `TESTING.md`.

**Suite tiers.** [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276) defines the suites. This ADR names them **quick**, **pre-merge**, **release** and **nightly**, and assumes only that each exists and that pre-merge can run a suite on every OS when a change touches it, as ADR 0032 already does for the sink suites.

## Decision

### Trust boundaries

`TESTING.md` lists the boundaries, each with its suite, tier and oracle: untrusted decoders; the Script sandbox; Plugin confinement; Chromium's sandbox; egress; listening endpoints and the Daemon group's peer link; secrets; file outputs; the Audit log's hash chain; the reset gate; supply chain and workflows. A boundary added by a later decision gets its row in the same change.

### Fuzzing

- **Targets.** Every decoder of input Goodfetch did not write itself has a `cargo-fuzz` target. `TESTING.md` keeps the list: protocol framing (JSON-RPC, the plugin protocol, the peer protocol, the Script sandbox's pipe protocol, catalog-call arguments), page content (CDP messages, HTML and Markdown, sitemaps, robots.txt and Site preference files), files from elsewhere (run-state import, Checkpoint and session-export bundles, Plugin manifests and OCI image indexes, Plugin bundle unpacking, Recipes and settings files, Filter lists, Session source formats including `Cookies.binarycookies`, extraction schemas), and URL canonicalisation with the resolved-IP classification.
- **Oracles.** No crash, no sanitizer error, no hang past the target's limit; plus round-trip or differential checks where a pair exists, such as the resolved-IP classification against a reference table built from the IANA registries.
- **Replay on every OS.** Each target's body is a plain function. A `#[test]` replays the committed seeds and every past crasher on every OS, since cargo-fuzz does not build on Windows (Wasmtime's pattern).
- **Pre-merge, public repository:** `cargo fuzz check` on every target, then ClusterFuzzLite's `code-change` mode for 600 s on the targets the change affects, on Linux x64, from the latest pruned corpus.
- **Nightly, private repository:** ClusterFuzzLite's `batch` mode for 3,600 s over public `main`, then `prune` and `coverage`. The private repository ADR 0032 already has keeps crashes, corpora and logs private. Runner minutes beyond the plan's included minutes are a paid test resource the owner approves.
- **Images by digest.** ClusterFuzzLite's images are run by digest from Goodfetch's own workflow, never through its action's mutable `:v1` tag. They are pulled before ADR 0043's deny, and the fuzz jobs run under it.
- **Crashes.**
  - In a pull request, the crash is fixed in that pull request.
  - On `main`, the crash opens a private security advisory draft, and the fix lands from the advisory's private fork under `SECURITY.md`'s coordinated disclosure (ADR 0030).
  - Either way, the crasher, minimised with `cargo fuzz tmin`, is committed as a regression input with its fix.
- **Corpus.** Seeds and a dictionary are committed per target. The nightly prune publishes the minimised corpus, which holds no crashing input, where pull-request fuzzing restores it before the deny.
- **Toolchain.** Fuzzing uses a nightly toolchain pinned in its own file, apart from `rust-toolchain.toml`.
- **OSS-Fuzz.** Its rule is "a significant user base and/or" critical infrastructure, so the owner applies once Goodfetch is published and has users. Its batch fuzzing then replaces the private batch run, and pull-request fuzzing stays.
- **Sanitizers beyond fuzzing.** A nightly run on Linux glibc executes the tests of every crate that opts into `unsafe` under AddressSanitizer, covering the FFI and system-call modules Miri cannot run.

### Confinement

- **Plugin confinement (changes ADR 0033).** Cases are generated from each generated policy (seccomp, Landlock, Seatbelt, AppContainer and the broker's filters), one probe per rule at each network level; a rule without a probe, or a probe without a rule, fails. A refused call is checked for the exact error ADR 0033 gives. Each probe first succeeds unconfined in the same test. Each runner asserts the mechanism it expects (Landlock ABI, user namespaces, the mount-namespace fallback), so a silent fallback fails: Ubuntu 22.04 covers the path without Landlock ABI 6, and 24.04 and 26.04 the ABI 6 path. A container under Docker's default seccomp profile on a kernel before 6.12 is refused with `confinement_unavailable`. ADR 0033's hand-written engine refusals stay, beside the generated cases.
- **The Script sandbox, as if V8 were already broken (adds to ADR 0051).**
  - **OS layer:** ADR 0033's probes run under exactly the policy generated for the worker. A live worker is also inspected: `Seccomp:` and `NoNewPrivs:` in `/proc/<pid>/status` on Linux, an AppContainer token on Windows, and on macOS the method the spec finds.
  - **V8 layer:** nightly on Linux, a test build of V8 with `v8_enable_memory_corruption_api` runs corruption scripts through the worker. Each must end as `script_crashed` and reach nothing outside it.
  - **If V8 refuses the build under `--jitless`,** the question returns to the owner. It is never skipped.
- **Chromium's sandbox (adds to ADRs 0017 and 0049).**
  - **Linux:** every renderer of Chrome for Testing and of the Goodfetch Chromium build shows `Seccomp: 2` and `NoNewPrivs: 1`, on Ubuntu 24.04 with its AppArmor restriction on.
  - **Windows:** `chrome://sandbox`, read through CDP, shows every process sandboxed.
  - **macOS:** the renderer runs under Seatbelt, by the method the spec finds.
  - **The startup probe of ADR 0049** is tested in its refuse and record cases.
- **Every OS before merge** when a change touches confinement, a policy generator, the plugin protocol or the worker (a requirement on the suites decision); otherwise in release.

### Egress and hostile input

- **One SSRF case table**, kept as data and run at every layer, with "no connection reached the target" as the oracle (the fixture records every connection) plus the typed refusal.
  - **Families:** encodings; IPv6 forms (IPv4-mapped, NAT64, 6to4, Teredo, zone IDs); every IANA special-purpose block that is not globally reachable; names with one public and one private answer; redirects of every status and kind, and to `file:`, `data:`, `javascript:` and `gopher:`; rebinding; the upstream proxy path; Plugin traffic.
  - **Layers:** the resolved-IP classification in-process (property tests against a reference table); the HTTP tiers and the egress proxy; browser Sessions for every request type ADR 0014 lists; raw CDP through the egress proxy; the relays of ADRs 0042 and 0044; a Borrowed browser, where the test asserts the rebinding gap is recorded in Provenance (ADR 0056); Sinks and webhooks under Goodfetch Cloud's "no private endpoints" policy.
- **Rebinding** uses a DNS fixture in the Proving ground serving TTL-0 alternation and mixed answers. End to end it runs on Linux, in a network and mount namespace whose resolver points at the fixture. In-process it runs on every OS, with a scripted sequence of answers.
- **The IANA special-purpose registries** are committed as data with their date. A scheduled job that runs none of the product opens an issue when IANA's files change.
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
  - The MCP conformance suite runs against MCP over HTTP.
  - A ZAP baseline scan of the Console runs nightly: image by digest, `-silent`, `allow_issue_writing: false`, its FAIL rules committed, and under ADR 0043's deny, so its canary catches a call home.
  - Nuclei and Schemathesis are not used.

### Secrets

- **Secret canaries** are planted through the Proving ground's login flows and through every Secret provider, for every kind of secret: a typed password, a TOTP seed, a proxy password, a model API key, a webhook signing secret, a Sink credential, a Session cookie, an endpoint token, the run data key.
- **Where they are searched, during and after a run,** raw and in the encodings Goodfetch produces (JSON-escaped, URL-encoded, base64, hex):
  - logs, Run traces, HAR files and console captures, and screenshots and recordings where masking applies;
  - the Audit log, the Effect journal, and Checkpoints outside their encrypted fields;
  - Exports and Notifications;
  - JSON-RPC errors, error messages kept in run state, and model requests;
  - the Console's offline cache, `goodfetch doctor`'s bug-report file, and the SDKs' `repr`, `toString` and error messages.
- **Redaction property tests.** Generated secret values that are hard to redact go through each formatter and serializer. Failing cases are committed in `proptest-regressions/`.
- **CI outputs.** The `gitleaks` binary, pinned by checksum, scans each job's logs, test reports, snapshots and artifacts before they upload, with the canaries as a rule. A hit fails the job. GitHub secret scanning and push protection are on.

### Other rules

- **Audit log tamper evidence (ADR 0016).** Verification fails on any edit, deletion, reordering or truncation of entries or of the chain's tail, including a Daemon group's single chain. These are property tests.
- **One regression test per bypass.** Every security fix, Goodfetch's own or a class a comparable project disclosed, gets a test at the layer it slipped through.
  - A scheduled job that runs none of the product lists new advisories from a fixed set of repositories and opens an issue.
  - A person triages the issue.
- **A security test never skips.** On a host that lacks a mechanism, it asserts the refusal instead. A missing tool fails the job.
- **Static analysis.** CodeQL scans Rust, TypeScript, Python and the workflows in pre-merge once the repository is public, and a high or critical alert blocks the merge.

### Tiers

| Part | Tier |
|---|---|
| Corpus and crasher replay, in-process SSRF and hostile-input cases, redaction property tests, Audit log tamper tests | quick; replay also pre-merge on every OS |
| `cargo fuzz check` and `code-change` fuzzing; confinement, Chromium sandbox, end-to-end egress and endpoint suites on Linux; gitleaks; CodeQL; MCP conformance over HTTP when MCP changes | pre-merge, plus every OS when the change touches the boundary |
| `batch` fuzzing, prune and coverage (private repository); ASan run; V8 memory-corruption build; ZAP baseline | nightly |
| Every suite on every OS, and every nightly-only part again on the release commit | release |

Secret canary searches run in every tier that produces outputs.

### Manual tests

Only these stay manual:
1. **Threat-model review**, on each minor's release candidate. A person walks the trust-boundary table against the minor's changes and confirms each new decoder, listener, Plugin kind, widening setting, Secret kind and output has its test. The result goes on the release pull request.
2. **Vulnerability triage**, on each fuzz crash on `main`, external report and advisory-watch issue. A person judges it against `SECURITY.md`'s vulnerability definition and either opens the private advisory or closes the issue with a reason.
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
- **A hand-written fuzz cron:** rejected. ClusterFuzzLite already stores, prunes, deduplicates and reports coverage.
- **ClusterFuzzLite's action as published:** rejected. It pulls a mutable image tag that SHA pinning does not cover.
- **Replay through cargo-fuzz only:** rejected. It skips Windows, where path handling differs most.
- **ADR 0033's hand list of refusals alone:** rejected. It drifts from the policy, and a probe that fails for an unrelated reason passes as a refusal.
- **Trusting V8's upstream fuzzing for the Script sandbox:** rejected. It tests V8, not the worker's confinement around it.
- **Off-the-shelf scanners as the endpoint suite:** rejected. None imports JSON-RPC or actively scans a WebSocket.
- **Nuclei for endpoint regressions:** rejected. Its templates would repeat the endpoint table in a second format, and its out-of-band checks use public servers by default.
- **A heuristic secret scanner as the oracle:** rejected. It misses Goodfetch's own encodings and raises false alarms. Canaries are exact, and gitleaks stays a second net.
- **gitleaks' Action:** rejected. It needs a licence key in organisations; the MIT binary does the same scan.
- **Public tools for DNS rebinding (rbndr, nip.io):** rejected. ADR 0043's deny blocks them, and a local fixture keeps the cases in CI.
- **Skipping a security test where the host lacks a mechanism:** rejected. A skipped refusal test is how a silent weakening ships.
- **More manual review:** rejected. Each item left manual is one no tool can do.

## Consequences

- **Amends `STYLE.md`:**
  - The untrusted-input rule becomes "every decoder of input Goodfetch did not write itself", with the list in `TESTING.md` and replay on every OS.
  - The secret rule's "a unit test proves the secret is not printed" becomes Secret canaries and redaction property tests.
- **Changes ADR 0033:** its "CI on every OS" cases are generated from the policy, check the exact error, carry a positive control and assert the mechanism in use. The hand-listed engine refusals stay.
- **Changes ADR 0032:**
  - pull-request fuzzing, gitleaks over CI outputs and CodeQL join the pull-request tier;
  - batch fuzzing joins the private repository's nightly run;
  - the ASan run, the V8 memory-corruption build and the ZAP baseline join the nightly tier;
  - every container image a workflow runs is pinned by digest, as Actions are pinned by SHA;
  - the IANA and advisory watch jobs run none of the product, so they sit outside ADR 0043's rule, like the watchlist job.
- **Adds to ADR 0030:** `SECURITY.md` publishes what counts as a vulnerability: anything that crosses a trust boundary of `TESTING.md`'s table, any memory unsafety, and a limit that never fires. A fuzz crash on `main` follows its coordinated disclosure.
- **Adds to ADRs 0017, 0049 and 0051:** the tests of the resolved-IP check, Chromium's sandbox and the Script sandbox worker described above.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)). It gains:
  - the jobs above;
  - the private repository's batch fuzzing;
  - the published corpus;
  - the digest pin check for images;
  - repository settings for secret scanning, push protection and code-scanning merge protection.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/Jbmanllr/automan/issues/152)). The Proving ground gains:
  - the DNS rebinding fixture;
  - a fixture that records every connection;
  - redirect, header, download-name and decompression-bomb pages;
  - login flows that plant Secret canaries;
  - a public page that calls the local Daemon, beside the origin-escape pages.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)):
  - its fuzzing line gains the Script sandbox's pipe protocol and catalog-call arguments;
  - Seam 2 gains the worker's OS-layer inspection.
- **Areas 4 (Security and egress) and 7 (Plugin protocol)** build the suites of their boundaries by these rules. ADR 0037's rows for areas 1, 2 and 4 list this ADR.
- **The "Test infrastructure" spec** builds:
  - the policy-to-probe generator;
  - the SSRF and hostile-input tables and their runners;
  - the connection-recording and DNS fixtures with the namespace setup;
  - the endpoint table's runner;
  - the canary planting and search helpers;
  - the fuzz replay harness;
  - the published corpus location;
  - the watch jobs.
- **Left to other tickets of the map:**
  - prompt-injection and fake-success pages ([Testing AI features and agent skills](https://github.com/Jbmanllr/automan/issues/281));
  - the MCP test stack ([MCP, framework adapter and Integration test stack](https://github.com/Jbmanllr/automan/issues/286));
  - the Console's test stack and its virtual authenticator ([Console and docs test stack](https://github.com/Jbmanllr/automan/issues/282));
  - manual-test IDs ([Journey catalog: format, IDs, seeded data and surfaces](https://github.com/Jbmanllr/automan/issues/278));
  - the time budgets ([Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276)).
- **Unverified, checked by the first implementation tickets:**
  - whether Landlock is enabled in GitHub's Ubuntu runner kernels;
  - whether V8's memory-corruption API works under `--jitless`;
  - how to read a macOS renderer's Seatbelt state;
  - whether Chromium validates header values passed through CDP.

  A negative answer returns to the owner as a decision, never as a skipped test.
