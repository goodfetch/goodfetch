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

## AI features and agent skills

Decided in [ADR 0065](docs/adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). Tiers are named as [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276) defines them: quick, pre-merge, release, nightly.

### Stand-ins for a model

| Layer | Stand-in | Tier |
|---|---|---|
| In-process AI logic | the Scripted model | quick |
| Each adapter's wire format | Model recordings replayed by `httpmock` at the adapter's base URL | quick, pre-merge |
| The binary, journeys, AI end to end | the Proving ground's Fake model, in all three formats | pre-merge |
| SDK code that calls a model | a model callback registered through the SDK handler | pre-merge |
| Local model servers | the real server, pinned, with a pinned small model | nightly, release |

- **No test of an AI feature calls a live model.** Only the jobs below go online, and they are measurements, not tests.
  _Why:_ providers no longer offer determinism: current Anthropic models refuse `temperature`, Gemini ignores it, and OpenAI's `seed` is best effort.
- **The Scripted model fails on a call beyond its script and on unused steps.** Script errors and faults as replies, and assert on the requests it recorded.
- **AI journeys name the Fake model as their Model provider.** Ask it for faults (rate limit, overload, timeout, cut stream, malformed JSON, refusal, wrong but plausible answer) through the task, never through a test-only switch in the binary.
- **Check Usage against the Fake model's own token count.**

### What an AI test asserts

- **Assert outcomes Goodfetch owns:** the Success check result, Grounding, the Plan file, the Effect journal, the Record, the Error code, Usage, what the oracle saw. Never a model's wording.
- **Never rely on temperature or a seed for determinism.**
- **Snapshot the whole request each AI operation sends, per format, with `insta`.** A prompt or tool-description change then shows as a reviewed diff.

### Model recordings

- **Record each cell of the matrix** per format on the direct provider: text, tool call and result turn, parallel tool calls, structured output, streaming of each, usage (cached and reasoning tokens), prompt caching, search grounding, image input, and the errors the engine branches on. Add the named gateways for the OpenAI-compatible format. A missing cell fails a check.
- **Match on method, path and the canonical JSON body.** An unmatched request fails, and so does an unused recording.
  _Why:_ matching on the path alone replays a stale answer to a changed request.
- **Replay each stream whole and re-cut at seeded boundaries.** The stream parsers also have `proptest` and a fuzz target.
- **Keep only allowlisted headers at write time:** `content-type`, the version and beta headers the adapter sends, `retry-after` and the rate-limit headers the adapters read. Remove key query parameters, and replace request and organization IDs. A pre-merge check fails on anything else, or on anything shaped like a provider key.
- **Record only against Proving ground pages and tasks.**
- **Refresh on cause:** a changed request, a canary report, or 30 days before a manifest model's retirement. Use the recorder job, which opens a pull request; review the diff. Never refresh on a timer or automatically.
- **Write faults that aren't worth recording as explicit `httpmock` stubs**, marked as synthetic.

### Live checks

- **The nightly provider canary compares shapes, never content.** It checks fields, types, stream events, error bodies and the headers adapters read, against each recording. Each difference opens an issue.
- **Local servers run for real:** llama.cpp server and Ollama nightly on CPU, vLLM and LM Studio where a runner supports them, every model on the tested list in the release tier. Assert protocol behaviour only, send every sampling parameter the engine relies on, and record the server version and model digest.
- **The in-engine Decision provider model is checked on a labelled set with a margin on every OS.** Never compare confidences exactly.

### Evaluations

- **Evaluations are `bench/` campaigns on the local Proving ground, never `cargo-nextest` tests.** They come in three sizes: pull-request, nightly (plus a weekly run on larger models), and ADR 0025's release benchmark.
- **The oracle grades every trial.** No model grades an Evaluation. Record the engine's Success check and Grounding results beside the oracle's verdict.
- **Run 3 trials per task and model** at the pull-request and nightly sizes, each from a clean Goodfetch home.
- **Report** the verified rate with its 95% Wilson interval, pass^3, false successes, safety violations, tokens and dollars per verified result, and the Success check and Grounding disagreements with the oracle. State the bound that "zero" can claim at the trial count (about 3/*n*).
- **A false success or a safety violation fails an Evaluation at any size.**
- **The regression rule** pairs tasks against the last accepted baseline on the same models. It fails only when the verified rate is confidently worse by more than Δ (block-bootstrap, ADR 0025). An inconclusive result is reported, never a failure. Δ starts at 10 points.
- **A task joins its regression set at pass^3 ≥ 0.9 on every nightly model for five nights.** Add harder tasks once a regression set passes everything for 30 nights. Add a task for every AI bug fixed.
- **The pull-request size uses development seeds; nightly and release use held-out seeds.**
  _Why:_ prompts tuned on the tasks that judge them overfit.
- **Run the pull-request size only when a pull request changes prompts, request snapshots, catalog descriptions or skills.** Never give model keys to a fork's pull request.
- **A confirmed nightly regression opens an issue and blocks the next release candidate** until it is fixed or the release notes explain it.
- **Pin models in the manifest; run old and new side by side for five nights before dropping one.** Record the model, any fingerprint the provider returns, the gateway, the local server's version and digest, the commit, the seed and the prompt snapshot hashes.
- **Every live job runs under a currency Limit equal to its cap** and prints its worst case first. Live jobs stay disabled until the owner approves the budget in ADR 0065.

### Agent skills and tool descriptions

- **Skills pass, with no model, in pre-merge:**
  - `xtask codegen --check`;
  - `claude plugin validate --strict` at a pinned version;
  - the Agent Skills frontmatter limits;
  - every operation, command and option they name exists in the catalog;
  - every example command runs against the local Proving ground and is checked by the oracle.
- **Skill Evaluations run the CLI with and without the skills**, through promptfoo, on Claude Code (small, mid and large models) and Codex. The oracle grades; `skill-used` is a signal. Open an issue on any skill that doesn't beat its baseline on some model.
- **Snapshot the catalog's MCP projection per profile.** Keep each description at most 2,048 characters, and record each profile's total tool-definition size in the snapshot.
- **The tool-selection Evaluation checks a model's first tool call without executing it.**

### Manual tests

- **After a confirmed Evaluation regression, read the failed trials' Run traces** before accepting or fixing it.
  _Why:_ only a person can tell a wrong grade or a new failure mode from a real regression.
- **LM Studio**, where no CI runner can host its headless server: run the tested-list cases by hand before each release.
