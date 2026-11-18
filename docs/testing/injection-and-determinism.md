## Injection and determinism

Decided in [ADR 0059](../adr/0059-dependencies-are-injected-as-values-and-only-time-and-randomness-as-handles.md).

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

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests, and, in the [Simulation](simulation-model-checking-and-crash-testing.md#simulation) alone, `transport`'s network (ADR 0060). That network is a value with product adapters, the OS and the in-memory pair the Products' embeddable crates use, so its interface is public for them, not for tests, and it is not a third handle.
  _Why:_ a fake passes where the real thing fails, and a stand-in tests what users run.
- **Nothing reads time, chooses at random or reads the environment through a global.** `disallowed-methods` refuses `SystemTime::now`, std and Tokio `Instant::now` and constructors such as `Ulid::new` outside `Clock` and `Entropy`, and `std::env::var` outside `main`; `std::env::set_var` is `unsafe`.

### Time

- **A rule that depends on time is a pure function that takes "now"**, tested with no clock: lease expiry, a Schedule's next occurrence (DST included), backoff. The Politeness store's granted shares stay in its Lua script and are tested against a real Valkey.
- **The backoff's jitter, `Retry-After` and the resolver's cache are such functions** ([ADR 0118](../adr/0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md)). With a seeded `Entropy`, each backoff wait lies between `politeness.backoff.initial` and three times the previous wait (the first between `initial` and three times it), never below `initial` or above `politeness.backoff.max`, and two domains backing off at once draw different waits. A `Retry-After` in seconds, in each HTTP-date format, against a `Date` header or the engine's clock, `0`, a past date and an unparsable value each give their wait, and one past `politeness.retry_after.max` holds the domain until the site's time and ends the requests it holds at once. A cached answer expires at its TTL, a negative one at its negative TTL, a server failure is never kept, and an answer whose every address failed to connect is dropped.
- **Code that waits is tested in-process with the test `Clock`** (`#[tokio::test(start_paused = true)]`). Advancing the paused clock moves wall time with it, and `jump_wall` moves wall time alone.
  _Why:_ a default-length lease or a day-long interval then takes no real time.
- **Keep real I/O out of paused-clock tests**, apart from SQLite on tracked blocking threads.
  _Why:_ auto-advance fires whenever the runtime is idle, including during a real socket wait, so a lease can expire early (tokio #4522, #8232).
  - A test that must wait on a real socket under the paused clock, such as the resolver cache's against the Proving ground's DNS fixture, holds the clock still meanwhile: the current-thread runtime never auto-advances while a blocking task runs (`goodfetch-engine`'s `tests/it/held_clock.rs`).
  - The Happy Eyeballs race's end-to-end test relies on auto-advance instead: while its attempt to an address that drops packets waits, the race's attempt delay is the only timer, so the clock moves exactly to it.
- **A test of the binary sets short durations through settings.** No build has a test-only clock control.
  - A short timeout bounds only the call under test: give that call its own `timeout` where it takes one, and keep the Run's `timeout.navigation` at its default. That setting also bounds each command of each page's opening, so a Run with a few seconds of it fails its first `run.new_page` on a loaded host (fix-queue 121). The browser's launch waits under `timeout.launch` instead (issue 2813).
- **No test's outcome depends on the calendar.** A test of the binary runs on the real date, so it never compares what the date alone changes: every Run that holds to the embedded Clean-up rules carries `clean_up_rules_stale` from 14 days after their date on, and a test that is not about the Clean-up rules compares a Run report without it (`calendar_free` in Collie's `tests/it/clean_up_rules.rs`). The staleness rule takes "now" and is tested on any date.
  _Why:_ on 2026-11-05 the embedded rules turned 14 days old and six Collie tests that snapshot a Run report failed on every branch with no change to the code.
- **Never sleep for real** beyond a short duration the test set itself.
- **A timing-sensitive test runs in nextest's `timing-sensitive` test group** (`.config/nextest.toml`): at most four of them run at once, whatever the run's thread count, and the rest of the suite takes the other threads.
  - A test is timing-sensitive when a check waits on real time against a figure the test or its suite sets: a lease, a navigation timeout, a domain delay or rate, an idle timer, a race's delay, a kill's deadline, or the protocol client's per-message deadline over a long browser run.
  - To tell, look for such a figure in the test or its helpers. Or run it with `NEXTEST_TEST_THREADS=num-cpus` beside other load (`just repeat`, at `nice -n 10`): a timing-sensitive test passes alone and fails, or nears its figure, there.
  - The group selects whole modules of a test binary, so a new test in a listed module joins it. A timing-sensitive test elsewhere goes into a listed module, or its module joins the filter in the change that adds it.
  - The group fixes no flaky test. It keeps the run's own load off the timers and lengthens none of them, and a test in it still asserts no elapsed time.

  _Why:_ nextest starts a binary's tests together, so at every thread the long browser cases of one module ran at once. On a 16-thread host shared with other builds and test runs, their timers then starved with no change to the product. In one of two such runs of every Rust test, three `request_kinds` cases missed their 60 s message deadline. With the group, no timer of its tests failed in two runs at every thread or in five repeats of its tests.
- **A test that takes many cases through browser pages holds one page at a time:** it finishes each Run once that Run's cases are done and closes each case's page before the next opens. The SSRF table's runners on Chromium also run in nextest's `browser-fleet` test group, at most two at once, and a new one joins its filter in the change that adds it.
  _Why:_ while the host's memory is under pressure, the Resource governor admits no page past its floor, one ([ADR 0040](../adr/0040-one-resource-governor-per-engine-sized-from-measured-load.md)), and a page past it waits with no answer until the pressure ends. The runners once held about 20 Runs, browsers and pages each until they ended, so on a loaded host their next `run.new_page` outlasted the 60 s message deadline (fix-queue items 107 and 110).
- **A test whose own work keeps several cores busy for minutes runs alone:** the Daemon soak ([Memory at scale](performance-budgets.md#memory-at-scale)) takes every test thread of the run (`threads-required = "num-test-threads"` in `.config/nextest.toml`), so nextest starts nothing beside it, and its five-minute kill stays the profile's. Such a test fits the kill alone on a loaded host, or it is split or made cheaper; no cap is raised for it.
  _Why:_ its 40 crawls keep the Daemon's two to three cores busy throughout. They took 165 to 210 s alone on the M1 server and 354 to 578 s beside the rest of a Pre-merge run, which timed it out in every M1 deep-fix group's run (#1740): its time followed the run's load, not the code. Since #1765 a Run's pages share the cosmetic selectors it parsed and its crawl settles the attempts that ended together in one pass, so it takes 100 to 142 s alone at load 15 to 28 and 138 to 148 s in a Pre-merge run. Its time is CPU, not waiting: under a load of 30 to 65 it took 231 to 265 s (#2050). Since #2050 the dependencies its pages run build optimized in `dev` (ADR 0061's note of 2026-11-04), so it takes 76 to 97 s alone at a mean load of 15 to 25, where `main` took 108 to 117 s at 16.
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
- **Local addresses use real extra loopback addresses:** `127.0.0.0/8` on Linux and Windows (Windows not yet tested), `lo0` aliases on macOS, a network namespace under sudo where needed. Tests use `127.0.0.2` to `127.0.0.9`, the count written in `ci/os-services.sh`; macOS answers only on `127.0.0.1` without aliases (checked on macOS 27).
- **No test reaches the public internet.** Only the jobs ADR 0043 names go online, to their named endpoints; every other job runs under its deny with its Egress canary, reaching only its fixtures. Every pinned download, the test V8 build and old releases included, comes before the deny, which allows none of the pinned artifact mirror's hosts (ADR 0043, by the owner's decision of 2026-10-19).
- **Nothing leaves the machine unless configured** (ADR 0043). `xtask telemetry`, in `just lint`, fails on a setting key that names telemetry and on an analytics client in `Cargo.lock`, `uv.lock` or `pnpm-lock.yaml`. A Quick test holds the modules `modules.toml` lets call `goodfetch-engine`'s `http_client` to `pinned_download` and the kinds of Operator-configured endpoint. The Proving ground records each pinned download's request (a plain GET, the user agent `goodfetch-download`), and its connection record shows that `egress.offline` refuses one before it connects.
- **The deny takes the form its runner allows** (`ci/egress/egress.sh`), and every test step after it runs through `ci/vm/exec.sh` or `ci/egress/under-deny.sh`, which `xtask egress check` enforces:
  - in the owner's machine's VM, and on hosted Linux runners in a network namespace, nftables refuse all but the job's allowed endpoints, and the kernel log names every other refused connection;
  - on hosted Windows runners, firewall rules block the product's programs (the workspace's executables and the test runtimes) from all but loopback and the allowed endpoints, and the firewall's log names every other drop. A program the tests copy outside those folders after the deny is not blocked;
  - on the owner's machine without a VM (the Release suite's macOS cells), a sandbox profile refuses every connection but `127.0.0.1` and `::1`, since the runner's standard user cannot change the firewall. The allowed endpoints and `lo0` aliases are refused too, so every download comes before the deny, and a test that connects to an extra loopback address cannot run there yet; one that only binds it can. macOS records no refusal that user can read: the canary proves the sandbox by its `EPERM`, and another refused connection fails only the step that made it.
- **The keychain provider runs against the real keychain** of each OS (`gnome-keyring` started and unlocked on Ubuntu), with a unique service name per test. Consumers of Secrets use the environment or caller provider.
- **Every CI job with a test step runs `ci/os-services.sh setup` before the deny and its first test step**, which `xtask os-services workflows` enforces in `just lint`:
  - on Linux it starts a D-Bus session bus at a path in the job's temporary folder and `gnome-keyring-daemon`'s Secret Service, unlocked with a throwaway password drawn in the job, its keyring files and control socket in that folder too, apart from any keyring of the user's login session; later steps find the bus through `DBUS_SESSION_BUS_ADDRESS`, and `XDG_RUNTIME_DIR` stays the user's own runtime folder where systemd made one, where `systemctl --user` finds the user's manager. Run again while that session's Secret Service answers, it keeps the session;
  - on Linux it also installs a root-owned copy of `unshare` at `/usr/local/libexec/goodfetch-test-tools/unshare`, which Collie's PID 1 test runs, and, where AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and later), loads `ci/apparmor/goodfetch-test-unshare`, which gives them to that copy only (ADR 0049's way for Chrome): no sysctl changes, and `/usr/bin/unshare` stays restricted; and `ci/apparmor/goodfetch-chrome-for-testing`, which gives them to the pinned Chrome for Testing build and its headless shell wherever a test lays them out, so their sandbox starts. It fails unless the copy maps the user to root in a user namespace, and the test fails at once, naming the restriction, when its `unshare` cannot;
  - on macOS it adds the `lo0` aliases, or, on the owner's machine, whose runner user has no sudo, finds them there from `sudo ci/runner/loopback-aliases.sh`, which the owner runs once;
  - on every OS it then runs `xtask os-services check`, which binds each extra loopback address and, on Linux, stores, reads and removes an item through the Secret Service. A service that does not come up fails the job; nothing skips.
- **A test that needs an OS service sits in a module named after it:** `keychain` for the real keychain, `extra_loopback` for an extra loopback address, `tc_netem` for `tc netem` on Linux's loopback interface (`CAP_NET_ADMIN` in the test's network namespace, which CI gives the job's container and, on a runner's own system, `tc` as a file capability). A local `just test` on a host without the service lists it among the parts it could not run and skips those modules' Rust tests with nextest's `--skip`; in CI the same check fails instead, and so it does where `GOODFETCH_OS_SERVICES=required`: `just pre-merge` on a Linux host, which runs those tests itself rather than in `just linux`'s container, runs `ci/os-services.sh setup` first, takes its variables, sets that one and declares `GOODFETCH_TEST_DNS_NAMESPACE=sudo` unless the caller declared a way. On macOS, add the aliases with `sudo ifconfig lo0 alias 127.0.0.<n> up` (or `sudo ci/runner/loopback-aliases.sh`) to run them.
- **`daemon install` installs for real on each OS.** The unit file it generates is snapshot-tested. Its tests sit in `service_manager` modules, in nextest's `service-manager` test group, and reach the user's own service manager through the variables of the test's own session; on Linux they run in the Pre-merge suite against systemd's user manager, which needs a host or container booted with systemd, and on macOS and Windows in the Release suite only, so a local run of another suite skips them.

### Isolation

1. One process per test (nextest).
2. A per-test temporary `GOODFETCH_HOME`, in a folder whose name holds a space and a non-ASCII letter on every OS ([Accessibility, localization and visual regression](accessibility-localization-and-visual-regression.md)).
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder, and whose `TZ` and `LANG` are `UTC` and `C.UTF-8` unless the test names others.
4. No fixed ports: bind port 0 and report the address, or use a Unix socket or named pipe in the test's folder.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Certificate authorities are made per test: the Proving ground's from its seed, a Daemon group's by `group init` into the test's folder.
8. No test reads another test's data or depends on test order.

### SDKs

- **SDK durations and jitter are options:** tests pass short values and a fixed seed. In TypeScript, Vitest fake timers may drive timer logic that never reaches the engine.
