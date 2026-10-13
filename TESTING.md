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

## Upgrades and version mismatches

Decided in [ADR 0066](docs/adr/0066-upgrades-are-tested-from-a-frozen-corpus-and-skew-from-every-released-minor.md). Tiers are named as [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276) defines them.

### The format registry

- **Every versioned format is registered in `goodfetch-protocol`** with its version, schema and rule. A format read by an engine or spoken by a peer and missing from the registry is a bug.
  _Why:_ every check below is generated from the registry, so a registered format cannot miss one.
- **Expected outcomes come from the compatibility functions**, never from a hard-coded version pair. Each function is itself tested against a literal table.

### The Compatibility corpus

- **A new format lands with its `compat/<format>/head/` set and its scenario** in `xtask compat write`.
- **Never edit a fixture by hand.** Regenerate `head/` with `xtask compat write`; `xtask compat check` fails on drift.
- **Never change a frozen set** (`compat/<format>/<major>.<minor>/`). If the reading of an old file must change, change its snapshot in a reviewed pull request whose changelog entry says so.
  _Why:_ a frozen file is what a released version really wrote.
- **Change a format's schema only with a version bump** once a frozen set exists. `xtask compat check` fails otherwise.
- **Bump a format's version only on `main`.** A patch never changes one.
- **Fixtures cover the whole schema:** every property in a JSON document and a non-null value in every column. They hold synthetic data and Secret references only.
- **A corpus test is golden.** It reads, writes back and compares with its snapshot. A migrated SQLite store must also match a fresh store's schema. Checkpoints resume, Plans replay without a model, Recipes run and transcripts replay.
  _Why:_ "it opens" misses a misread.

### `goodfetch upgrade-files`

- **Each source in `compat/upgrade-paths.toml` is tested from every frozen set:** golden result, idempotent, the previous major's files unchanged, unrelated files untouched. A source not in the table and a newer major are refused.

### Refusals

- **Every refusal is tested from a fabricated version**, generated per format: a newer minor, an older major, a newer major, and each exception's own outcome (Usage-ledger family, Response cache, Clean-up rules, Operator policy). The Error code and message are snapshotted.
- **A handshake refusal is the reply to `hello`**, before any other message, and the connection then closes. Never test refusal with a time budget.
- **Fabricate a peer's version through the conformance harness's own `hello`.** No build carries a version override.

### Released artifacts

- **Skew covers every released minor of the current major**, latest patch of each: SDKs, the Rust crate's client, the CLI and the MCP server against the current Daemon (the SDKs over stdio too), the Console, the Browser host and first-party Plugins.
- **An old Client runs its own release's journeys** from its tag. A journey that asserts something outside semver is fixed on `main` and excluded by ID, with a reason, in `compat/skew-exclusions.toml`.
- **Rolling upgrades run under load** in a three-member group, once with the Lead Daemon last and once with it first, then a promotion from an older Group backup and the rollbacks. The Helm chart upgrades with `ct install --upgrade`.
- **Older engines open the files the current engine wrote.** The Usage-ledger family opens; every other format is refused. Two majors run side by side on one machine.
- **Old artifacts come only from `compat/releases.toml`,** through the pinned artifact mirror, checked by digest and by `gh attestation verify --bundle`.
- **Never skip for a missing artifact.** A missing or mismatched artifact fails the job. An empty version list runs the current build against itself.
  _Why:_ a test that passes because nothing ran hides decay, as SQLite's and Cargo's skipped old-binary tests do.

### Tiers

| Check | Tier |
|---|---|
| Corpus golden reads, fabricated refusals, compatibility-function tables, `upgrade-files` from fixtures | quick |
| `xtask compat check`, corpus replays and `upgrade-files` against the binary | pre-merge |
| Skew against the latest released minor, when a protocol, a handshake or the registry changes | pre-merge |
| The full skew matrix, rolling upgrades, older engines, two majors, the Helm upgrade | nightly |
| The nightly set on each release candidate's, stable release's and patch's own bytes | release, blocking |
