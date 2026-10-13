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

## MCP, framework adapters and Integrations

Decided in [ADR 0070](docs/adr/0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md).

### Every surface

- **Drive each surface with its platform's own client, against a real engine or Daemon serving the local Proving ground.** Never mock Goodfetch: no `nock`, fake API or stub listener.
  _Why:_ a mock of the engine passes where the real one fails, and misses the token, `Host` and `Origin` checks.
- **Stand in for the platform in this order:** its real runtime run locally, its official test harness, a written adapter of its context.
- **Keep a Quick check over everything `xtask codegen` generates:** no person-only operation or setting on any MCP profile, Framework adapter, Integration or skill, and each surface's operations map to catalog operations of the same class.
- **Assert outcomes through the oracle and Error codes**, never a model's wording or a platform's message text.

### MCP server

- **Test the server with `rmcp`'s client:** in-memory in its crate, over stdio and Streamable HTTP in the binary's `tests/it`. MCP Journey variants run there too, on nextest.
- **Read stdio stdout with the strict framing reader**, logging at `trace`. Any line that is not a JSON-RPC message fails the test.
- **Run the conformance suite at its pinned version, once per supported revision**, with `--spec-version` and `conformance/mcp/expected-failures/<revision>.yaml`:
  - against the shipped binary, whose baseline lists only scenarios that need fixture tools or resources, each with a reason (a Quick check refuses other entries);
  - against the fixture server, with an empty baseline;
  - once more without `--spec-version` for the extensions the server implements.
- **Change the suite's version only in a pull request that updates the baselines.**
- **Run the pinned Inspector CLI with `--format json --stored-auth-only`:** boot smoke of every packaged artifact over stdio and Streamable HTTP, `--strict` on every profile's `tools/list`, and `--verify` on skills once the server serves them.
- **Cover every supported revision in the client matrix:** TypeScript SDK v1 and v2 clients, the Python SDK and `rmcp`, pinned, over stdio and Streamable HTTP, plus the negotiation cases.
- **Add a revision to the supported list only with its conformance baseline and its matrix cells, in the same pull request.**

### Framework adapters

- **Drive each tool through the framework's own agent loop with its own scripted model**, never by calling the tool's `execute` alone: `MockLanguageModelV4` for the Vercel AI SDK and Mastra, `FakeListChatModel` or `FakeStreamingChatModel` for LangChain.js, `GenericFakeChatModel` through test support's `bind_tools` subclass for Python LangChain.
- **Script a read, an Effect and a failing call** in each adapter's tests.
- **Run `ToolsUnitTests` on every generated Python LangChain tool**, and `ToolsIntegrationTests` on one tool per operation class and every tool a Journey uses, from a pinned `langchain-tests`.
- **Test the lowest and highest version of each declared peer range, and one of each major inside it**, in the Release suite. Pin the lowest in a test package (pnpm's `lowest-direct` never lowers peers); use uv's `--resolution lowest-direct` in Python.
- **Give each framework that connects only through MCP a model-free test** of its docs example: its own MCP client connects, lists tools and calls one tool.
- **Check Dataset loaders against the oracle's records**, including metadata and lazy streaming.

### Integrations

- **Start the Daemon through `goodfetch-test-env`** with its HTTP listener, a token and port 0.
- **Cover, for each Integration:** starting a run and a Recipe, fetching Records, one trigger per record range, no repeated trigger after a resumed run, a refused token, a refused `hello` version, and every Error code it maps.
- **Run each platform's offline validator in the Quick suite**, and the Integration in its platform's local runtime or official harness in the Pre-merge suite:

| Integration | Quick | Pre-merge |
|---|---|---|
| n8n | `n8n-node lint` (strict); verification rules | pinned n8n image, `n8n execute --id` |
| Zapier | `zapier-platform validate --without-style` | `createAppTester`, real HTTP |
| Make | Make Apps Editor JSON schemas; module-to-operation map | none (Live check) |
| Pipedream | Pipedream ESLint rules, build, key checks | `run` with the `$` adapter |
| Activepieces | lint, build | `createMockActionContext`; pinned Activepieces image |
| Power Automate | `ConnectorPackageValidator.ps1`; OpenAPI 2.0 validation | every operation through an OpenAPI 2.0 client |
| Airbyte source | offline QA checks; manifest validation | CDK standard suite; incremental reads from saved state |

### Live checks and manual tests

- **Run unpinned releases only as Live checks:** the newest conformance suite, Inspector and MCP SDK clients, and each framework's `latest` and prerelease tags, nightly; the n8n scanner after each publish.
- **Keep real platform accounts out of every suite.** They run weekly and at release candidates, only once the owner has approved each account, behind a repository variable until then.
- **Test by hand at each release candidate that changes them:** Claude Desktop, ChatGPT, VS Code and Cursor connecting to the MCP server; `paconn validate`; each platform's listing submission.
