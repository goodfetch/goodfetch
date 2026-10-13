# Testing

How Goodfetch is tested: every surface, at every level. Each rule comes with its reason, and the hard-to-reverse ones are decided in ADRs. [`STYLE.md`](STYLE.md) shapes the code these tests drive.

## Injection and determinism

Decided in [ADR 0059](docs/adr/0059-dependencies-are-injected-as-values-and-only-time-and-randomness-as-handles.md).

### What is injected, and how

| Dependency | Injected as | In tests |
|---|---|---|
| Wall time, monotonic time, sleeps | the `Clock` handle | its test version under Tokio's paused clock |
| Random choices (jitter, pool picks, fingerprint seeds, ID randomness) | the `Entropy` handle | a fixed seed, printed at test start |
| Secret randomness (keys, tokens, run data keys, CAs) | nothing: the OS generator | the OS generator |
| Data, caches and configuration | the `GOODFETCH_HOME` value | a per-test temporary folder |
| SQLite and files | paths under that home | real files |
| Listen addresses | a value (port 0, or a socket path) | the bound address reported back |
| Host names | the `resolve` value | Proving ground hosts mapped to loopback |
| Environment | an explicit map | a cleared map for each child process |
| Keychain, service manager, processes | nothing: the real service | the real service, unique names |

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests.
  _Why:_ a fake passes where the real thing fails, and a stand-in tests what users run.
- **Nothing reads time, chooses at random or reads the environment through a global.** `disallowed-methods` refuses `SystemTime::now`, std and Tokio `Instant::now` and constructors such as `Ulid::new` outside `Clock` and `Entropy`, and `std::env::var` outside `main`; `std::env::set_var` is `unsafe`.

### Time

- **A rule that depends on time is a pure function that takes "now"**, tested with no clock: lease expiry, a Schedule's next occurrence (DST included), backoff. The Politeness store's granted shares stay in its Lua script and are tested against a real Valkey.
- **Code that waits is tested in-process with the test `Clock`** (`#[tokio::test(start_paused = true)]`). Advancing the paused clock moves wall time with it, and `jump_wall` moves wall time alone.
  _Why:_ a default-length lease or a day-long interval then takes no real time.
- **Keep real I/O out of paused-clock tests**, apart from SQLite on tracked blocking threads.
  _Why:_ auto-advance fires whenever the runtime is idle, including during a real socket wait, so a lease can expire early (tokio #4522, #8232).
- **A test of the binary sets short durations through settings.** No build has a test-only clock control.
- **Never sleep for real** beyond a short duration the test set itself.
- **Clocks Goodfetch doesn't own stay real:** the Valkey server's (short durations), Chromium's (Proving ground pages take their time step from the task), and the Script sandbox's live `Date.now`, whose tests compare the order of Effects.

### Randomness

- **Every random choice draws from `Entropy`** (`chacha20`, portable across versions).
- **A test runs on a fixed seed.** `GOODFETCH_TEST_SEED` overrides it, and the seed is printed when the test starts.
  _Why:_ a test nextest kills on a timeout never reaches a failure message.
- **Secrets never come from `Entropy`.** The Proving ground's seeded test certificate authority is the one exception.
- **Hash order never reaches output or ordering.** `iter_over_hash_type` is denied; use `BTreeMap` or `IndexMap` where order shows.

### Storage, network and OS services

- **Each test gets its own `GOODFETCH_HOME`** in a temporary folder, and writes nowhere else.
- **Proving ground host names resolve through the `resolve` setting** to loopback, which the run allowlists for tests.
- **Proxies are tested against the Proving ground's real forward proxy.**
- **Local addresses use real extra loopback addresses:** `127.0.0.0/8` on Linux and Windows (Windows not yet tested), `lo0` aliases on macOS, a network namespace under sudo where needed.
- **No test reaches the public internet.** Only the jobs ADR 0043 names go online, to their named endpoints; every other job runs under its deny with the canary.
- **The keychain provider runs against the real keychain** of each OS (`gnome-keyring` started and unlocked on Ubuntu), with a unique service name per test. Consumers of Secrets use the environment or caller provider.
- **`daemon install` installs for real on each OS.** The unit file it generates is snapshot-tested.

### Isolation

1. One process per test (nextest).
2. A per-test temporary `GOODFETCH_HOME`.
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder.
4. No fixed ports: bind port 0 and report the address, or use a Unix socket or named pipe in the test's folder.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Certificate authorities are made per test: the Proving ground's from its seed, a Daemon group's by `goodfetch group init` into the test's folder.
8. No test reads another test's data or depends on test order.

### SDKs and the Console

- **SDK durations and jitter are options:** tests pass short values and a fixed seed. In TypeScript, Vitest fake timers may drive timer logic that never reaches the engine.
- **The Console's end-to-end tests set time with Playwright's `page.clock`** (`setFixedTime` first). Its component tests use Vitest fake timers.

## Security

Decided in [ADR 0067](docs/adr/0067-security-is-tested-per-trust-boundary-with-canaries-and-split-fuzzing.md). Suite names (quick, pre-merge, release, nightly) follow the suites decision; "every OS before merge" means pre-merge runs the suite on every OS when a change touches its boundary.

### Trust boundaries

Every boundary has a suite. A decision that adds a boundary adds its row in the same change.

| Boundary | Suite | Tier |
|---|---|---|
| Untrusted decoders | fuzz targets and their replay | replay: quick, and every OS in pre-merge; fuzzing: pre-merge and nightly |
| Script sandbox | worker confinement probes and inspection; V8 memory-corruption build | pre-merge, every OS before merge; V8 build nightly |
| Plugin confinement | probes generated from each policy; engine refusals | pre-merge, every OS before merge |
| Chromium's sandbox | renderer sandbox checks; startup probe cases | pre-merge on Linux, every OS in release |
| Egress | SSRF case table at every layer; hostile-input set | in-process: quick; end to end: pre-merge, every OS before merge |
| Endpoints and the peer link | endpoint table; protocol fuzzing; MCP conformance; ZAP baseline | pre-merge; ZAP nightly |
| Secrets | Secret canary searches; redaction property tests; gitleaks over CI outputs | every tier |
| File outputs | path traversal cases per OS | pre-merge, every OS before merge |
| Audit log | tamper property tests | quick |
| Reset gate | ADR 0001's canaries | as ADR 0001 sets |
| Supply chain and workflows | ADR 0032's gates; CodeQL; image digest pins | pre-merge |

Release runs every suite on every OS, and every nightly-only part again on the release commit.

### Fuzzing

- **Every decoder of input Goodfetch did not write itself has a `cargo-fuzz` target.** Today: JSON-RPC, plugin-protocol, peer-protocol and Script sandbox pipe framing; catalog-call arguments; CDP messages; HTML and Markdown; sitemaps, robots.txt and Site preference files; run-state import; Checkpoint and session-export bundles; Plugin manifests, OCI image indexes and bundle unpacking; Recipes and settings files; Filter lists; Session source formats, `Cookies.binarycookies` included; extraction schemas; URL canonicalisation and the resolved-IP classification.
- **Write each target's body as a plain function**, and replay its seeds and every past crasher from a `#[test]` on every OS.
  _Why:_ cargo-fuzz does not build on Windows, and path handling differs most there.
- **Check more than crashes where a pair exists:** round-trip for encoders and decoders, and the resolved-IP classification against the reference table built from the IANA registries.
- **Commit seeds and a dictionary per target.** Pull-request fuzzing restores the published, pruned corpus before ADR 0043's deny.
- **Pre-merge:** `cargo fuzz check` on every target, then ClusterFuzzLite `code-change` for 600 s on the affected targets, Linux x64.
- **Nightly, in the private repository:** ClusterFuzzLite `batch` for 3,600 s, then `prune` and `coverage`.
  _Why:_ a crash on `main` stays private until its fix ships.
- **Run ClusterFuzzLite's images by digest** from Goodfetch's workflow, never through its action's `:v1` tag.
- **Fix a pull request's crash in that pull request.** A crash on `main` opens a private security advisory draft and is fixed from its private fork. Either way, commit the crasher, minimised with `cargo fuzz tmin`, as a regression input with the fix.
- **Fuzz on a nightly toolchain pinned in its own file**, apart from `rust-toolchain.toml`.
- **Run the tests of every crate that opts into `unsafe` under AddressSanitizer nightly**, on Linux glibc.

### Confinement

- **Generate Plugin confinement cases from the policy.** One probe per rule of each generated seccomp, Landlock, Seatbelt, AppContainer and broker policy, at each network level. A rule without a probe, or a probe without a rule, fails.
- **Assert the exact error** ADR 0033 gives for a refused call. A kill or an allowed call fails the test.
- **Pair every refusal with a positive control:** the same probe first succeeds unconfined, in the same test.
  _Why:_ a probe that fails for an unrelated reason would otherwise pass as a refusal.
- **Assert the mechanism each runner uses** (Landlock ABI, user namespaces, mount-namespace fallback), so a silent fallback fails. A container under Docker's default seccomp profile on a kernel before 6.12 must refuse with `confinement_unavailable`.
- **Test the Script sandbox worker as if V8 were broken:** run the probes under the worker's own generated policy, and inspect a live worker (`Seccomp:` and `NoNewPrivs:` on Linux, an AppContainer token on Windows, the spec's method on macOS). Nightly on Linux, run corruption scripts on a V8 build with `v8_enable_memory_corruption_api`; each must end as `script_crashed`.
- **Check that every renderer is sandboxed** for each browser Goodfetch launches: `Seccomp: 2` and `NoNewPrivs: 1` on Linux (Ubuntu 24.04 with its AppArmor restriction on), `chrome://sandbox` on Windows, Seatbelt on macOS. Test ADR 0049's startup probe in its refuse and record cases.

### Egress and hostile input

- **Keep SSRF cases in one table, as data, and run it at every layer:** the resolved-IP classification in-process, the HTTP tiers, the egress proxy, browser Sessions for every request type of ADR 0014, raw CDP, the relays of ADRs 0042 and 0044, a Borrowed browser, Plugin traffic, and Sinks and webhooks under "no private endpoints".
- **Cover every family:** encodings; IPv6 forms (IPv4-mapped, NAT64, 6to4, Teredo, zone IDs); every IANA special-purpose block that is not globally reachable; names with mixed answers; redirects of every kind and to non-HTTP schemes; rebinding; the upstream proxy path.
- **Assert that no connection reached the target**, from the Proving ground's connection-recording fixture, and the typed refusal.
- **Test rebinding against the Proving ground's DNS fixture**: end to end on Linux in a network and mount namespace whose resolver points at it; in-process on every OS with a scripted sequence of answers. Never use a public rebinding service.
- **Keep the IANA special-purpose registries committed with their date.** A scheduled job outside ADR 0043's rule opens an issue when they change.
- **Test the hostile-input set where each case can happen:** CR, LF or NUL in header values (Recipes, Sinks, webhooks, CDP); a domain-bound Secret across a cross-origin redirect; path traversal into every file Goodfetch writes from outside input, with each OS's cases; external `$ref` in extraction schemas; no process spawned to compare page content (checked under a seccomp filter that refuses `execve`); decompression bombs, endless bodies, deep nesting and oversized messages ending in their typed limit errors.

### Endpoints

- **Run the endpoint table against every listener:** the Console, the Daemon's socket and WebSocket, the HTTP listener, the ops listener, MCP over HTTP, the Live view, raw CDP, the Attached engine's private endpoint, an Exposed Daemon and the peer link.
- **Cover every row:** missing, wrong, expired, revoked, out-of-scope and other-listener tokens, and tokens in a query string; a foreign `Host`; a foreign or missing `Origin` on the request and the WebSocket handshake; plain HTTP where TLS is configured; `X-Forwarded-*` from an unconfigured address; each listener's own rules (ADRs 0031, 0049, 0050, 0055, and MCP's audience and no passthrough). Check each refusal in the Audit log.
- **Fuzz the protocol from the committed JSON Schema bundle** with `proptest` and `proptest-state-machine`, over every transport. Every invalid call gets a typed error and changes no state.
- **Run the MCP conformance suite against MCP over HTTP.**
- **Run a ZAP baseline scan of the Console nightly:** image by digest, `-silent`, `allow_issue_writing: false`, FAIL rules committed, under ADR 0043's deny.

### Secrets

- **Plant a Secret canary for every kind of secret** (typed password, TOTP seed, proxy password, model API key, webhook signing secret, Sink credential, Session cookie, endpoint token, run data key), through the Proving ground's login flows and every Secret provider.
- **Search every output for every canary**, raw and JSON-escaped, URL-encoded, base64 and hex. The outputs are:
  - logs, Run traces, HAR files and console captures, and screenshots and recordings where masking applies;
  - the Audit log, the Effect journal, and Checkpoints outside their encrypted fields;
  - Exports and Notifications;
  - JSON-RPC errors, error messages in run state, and model requests;
  - the Console's offline cache, `goodfetch doctor`'s file, and the SDKs' `repr`, `toString` and errors.

  _Why:_ redaction is best effort against arbitrary transforms, but Goodfetch's own encodings are known.
- **Property-test redaction** with generated values that are hard to redact (quotes, backslashes, newlines, `%`, Unicode, very long, empty, common words, values that contain one another), through every formatter and serializer. Commit `proptest-regressions/`.
- **Scan each CI job's logs, test reports, snapshots and artifacts with the `gitleaks` binary** (pinned by checksum, canaries added as a rule) before they upload. A hit fails the job.

### Other rules

- **Property-test the Audit log's hash chain:** an edit, deletion, reordering or truncation of entries or of the tail fails verification, a Daemon group's single chain included.
- **Add one regression test per bypass** at the layer it slipped through, Goodfetch's own or a class a comparable project disclosed. Cite the advisory in a comment.
- **Never skip a security test.** On a host that lacks a mechanism, assert the refusal (`confinement_unavailable`, `local_address_unavailable`). A missing tool fails the job.
- **Pin every container image a workflow runs by digest.**
- **Run CodeQL** on Rust, TypeScript, Python and the workflows in pre-merge once the repository is public. A high or critical alert blocks the merge.

### Manual security tests

Only these stay manual. Their IDs follow the Journey catalog's format.

| Manual test | When | What a person does | Recorded on |
|---|---|---|---|
| Threat-model review | each minor's release candidate | Walks the trust-boundary table against the minor's changes and confirms each new decoder, listener, Plugin kind, widening setting, Secret kind and output has its test | the release pull request |
| Vulnerability triage | each fuzz crash on `main`, external report and advisory-watch issue | Judges it against `SECURITY.md`'s vulnerability definition; opens the private advisory or closes with a reason | the advisory or issue |
| Independent security assessment | before 1.0 and each later major; paid, owner-approved | An outside tester attacks the boundaries; each finding gets a fix and a regression test | a report linked from the release notes |
| Confinement where CI cannot run | each minor's release candidate | WSL1's refusal; Bridge Sinks under Docker Desktop and Podman machine on macOS and Windows; the documented host-firewall recipes for `direct` | the release pull request |
| Passkeys on real authenticators | each minor's release candidate | Sign-in, step-up and recovery on a platform authenticator and a hardware key | the release pull request |
