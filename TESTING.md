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

## Journeys

Decided in [ADR 0063](docs/adr/0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md). A Journey is a goal a person reaches through Goodfetch's public surfaces; Journeys are the end-to-end tests.

### The catalog

- **One Journey per file, `journeys/J-NNN.toml`**, validated by `journeys/journey.schema.json`, which `xtask codegen` emits. Unknown fields fail.
- **A new Journey takes the number in `journeys/catalog.toml`** and increments it in the same pull request. If another pull request merged the same number first, take the next one.
- **Never change an ID on `main`, and never reuse one.**
- **Never change a Journey's `goal` under its ID.** Fix wording, steps, data or surfaces freely; for a different goal, set `status = "retired"` with `reason` and `replaced_by`, keep the file, and add a new Journey.
- **Write `goal` with RFC 2119 words**, readable as a specification without the code.
- **Write steps declaratively** ("export the Dataset as CSV"), never as clicks or flags, so the wording survives a change of implementation. Pin a step with `on` only when the goal needs that surface; give `manual` a reason only when no automated check can do it.
- **List every surface on which a person can reach the goal**, each with the milestone by which it must be proved, never earlier than the surface ships.
- **Fill `covers` with feature IDs** (`row:CO-18` for an ADR 0037 row).
- **Keep a Journey to its goal.** Edge cases, Error codes and limits go to unit, integration and conformance tests.
- **Each area's `/to-spec` adds the Journeys of its user stories**, and each ticket names the variants (`J-NNN@<surface>`) it makes pass.

### Data and isolation

- **Each Journey has its own `seed`.** Never share one between Journeys.
- **Each variant run starts from scratch:** its own Proving ground on port 0 with the Journey's seed, its own Goodfetch home and cleared environment, `resolve` mapping the Proving ground's hosts to loopback, and its certificate authority trusted through the per-domain exception.
- **Build the starting state through public surfaces** in the first steps. Never copy in a run file, database or home folder, except an older-format file for an upgrade Journey.
- **Run against the local copy only.** Online results stay conformance reports.
- **Drive the shipped artefacts**: the `goodfetch` binary and the SDK packages built from the commit, never an engine linked into the test.
- **When a generator change moves a task a Journey names, update the Journey in the same pull request.**

### Tests and results

- **Carry the ID where the runner selects by it:**
  - nextest: a module `j002` (the ID lowercased, hyphen dropped) in the test's path;
  - Vitest: the tag `J-002`, declared from the catalog, `strictTags` on;
  - pytest: `@pytest.mark.journey("J-002")`, registered, `strict_markers` on;
  - Playwright Test: the tag `@J-002`.
- **One test per variant.**
- **Each variant run writes one result document with `kind = "journey"`.** Never set its outcome: it is derived from the steps, the checks and the oracle's answers. A step not reached, or any task not `verified`, fails the variant.
- **The catalog check fails** on a file that does not parse, an ID that differs from its file name or repeats, a task missing from the Proving ground's task list for the seed, a test naming a retired or unknown Journey or an unlisted surface, two tests for one variant, a variant milestone earlier than its surface, or a retired Journey without a reason.

### Owner tests

- **List an owner test's "Journeys to try" from the catalog:** every variant of its milestone, by ID and title, with its manual steps.
- **An owner test is ready only when every automated variant of its milestone is `proved` on `main`.**
- **A finding on a goal no Journey covers adds a Journey.**
