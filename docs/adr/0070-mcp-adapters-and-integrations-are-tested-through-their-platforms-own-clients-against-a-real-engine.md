# MCP, framework adapters and Integrations are tested through their platforms' own clients against a real engine

The MCP server, the Framework adapters and the Integrations are each driven, in tests, by the client their platform really uses: the official MCP SDK clients and the MCP Inspector, each agent framework's own agent loop with its own scripted model, each workflow tool's own runtime or official test harness. On Goodfetch's side they always meet a real engine or Daemon serving the local Proving ground, never a mock. The official MCP conformance suite runs over HTTP once per supported spec revision, against the shipped binary and against a fixture server built from the same MCP code. A matrix of official SDK clients covers each revision over stdio and HTTP. What only a real platform account or an unpinned release can show runs as Live checks (ADR 0064), and the accounts wait for the owner's approval.

Comparable projects test these surfaces unevenly ([findings](https://github.com/Jbmanllr/automan/blob/research/agent-surface-testing/rnd/research/2026-10-13-agent-surface-testing.md)):
- **MCP.** The reference servers drive a real SDK client against the server, then start each built server "the way a user's client does", since "a wrong `bin` path … or a server that dies during the `initialize` handshake all pass the unit tests". Apify runs the official conformance suite in CI, pinned, once per spec revision, with a baseline of the scenarios that need fixture tools. The suite tests servers over HTTP only ([conformance#258](https://github.com/modelcontextprotocol/conformance/issues/258), open). Users' main complaint is tool schemas that clients refuse.
- **Adapters.** No framework publishes a test suite for third-party tool adapters in TypeScript. The AI SDK ships mock models (`ai/test`) that drive a tool call with no model. `langchain-tests` covers Python tools. Stagehand pins its contract once and tests only each framework's wiring.
- **Integrations.** Apify mocks its own API with `nock` in its n8n and Zapier tests; Firecrawl's and Browserbase's n8n nodes have no tests. Platforms certify by human review with accounts.
- **Loaders.** LangChain lists document loaders under "Not these ❌" ("High maintenance burden") for integrations it lists in its docs, while `BaseLoader` stays in `langchain-core`. LlamaIndex.TS is archived and "no longer maintained".

Decided in [MCP, framework adapter and Integration test stack](https://github.com/Jbmanllr/automan/issues/286) ([grilling record](https://github.com/Jbmanllr/automan/issues/286#issuecomment-6152373951)), with the agent answering on the owner's delegation, on the facts of [Testing MCP, framework adapters, Integrations and agent skills](https://github.com/Jbmanllr/automan/issues/267). The rules every test follows are in `TESTING.md`. Suites (Quick, Pre-merge, Release) are ADR 0061's, Live checks ADR 0064's, layout and test support ADR 0062's.

## Decision

### One shape for every surface

- **Goodfetch's side is real.** The engine, the Daemon and its HTTP listener run for real, serving the local Proving ground, started through `goodfetch-test-env` (ADR 0062). No test of these surfaces mocks Goodfetch.
- **The platform's side is, in this order:** its real runtime run locally; its official test harness; a written adapter of its context (ADR 0062's double for a true external).
- **Four layers per surface:**
  1. checks of the generated surface, with no process (Quick);
  2. the surface driven through its platform's client against a real engine (Pre-merge, when affected);
  3. the same across the platform versions Goodfetch supports (Release suite);
  4. Live checks for unpinned releases and real accounts.
- **Across every generated surface** (MCP profiles, adapters, Integrations, skills), a Quick check refuses any person-only operation or setting (ADRs 0011, 0029, 0041, 0042, 0045, 0048, 0052, 0056), and checks that each surface's operations map to catalog operations of the same class and that none the catalog offers it is missing.

### MCP server

- **Its own tests use `rmcp`, the official Rust SDK's client.** Unit tests drive the server over an in-memory transport: profiles, annotations from the operation class (read, idempotent write, Effect), each Error code's tool result, cancellation, progress, docs resources, Code mode only where a profile opts in. Integration tests and the MCP Journey variants (ADR 0063) run on nextest in the binary crate's `tests/it`, over stdio and Streamable HTTP.
- **stdout stays pure.** Every stdio test reads stdout with a strict framing reader that fails on any line that is not a JSON-RPC message, with logging at `trace`.
- **The conformance suite**, pinned to an exact version (today `0.2.0-alpha.12`, the line that holds the per-revision requirement sets):
  - runs once per supported revision with `--spec-version` and `conformance/mcp/expected-failures/<revision>.yaml`;
  - against the shipped binary, whose baseline may list only scenarios that need fixture tools or resources, each with its reason;
  - against a **fixture server**, a test target built from the same MCP server code with a catalog of the suite's fixture tools, never shipped, with an empty baseline;
  - with a separate run, without `--spec-version`, for the extensions the server implements;
  - Pre-merge when MCP code, the catalog or a baseline changes; always in the Release suite, on Linux.
- **stdio conformance** comes from the client matrix and the Inspector until the suite gains a stdio mode; the nightly Live check below shows when it does, and adopting it is a reviewed change.
- **The MCP Inspector CLI**, pinned, with `--format json --stored-auth-only`:
  - boot smoke of each packaged artifact (npm, PyPI, archive, OCI image): `initialize`, `tools/list`, one `tools/call` against the Proving ground, `resources/list`, over stdio and Streamable HTTP. Release suite on every OS; Pre-merge on the build tree when MCP or packaging changes;
  - `--strict` on `tools/list` for every profile, Pre-merge when the catalog or MCP code changes. ADR 0065's projection snapshot and 2,048-character description limit stay as decided there;
  - `--verify` on `skills/list` and `skills/get` from the first release whose MCP server serves skills (area 11 decides whether it does).
- **The client matrix** covers the revisions hosts negotiate:
  - clients, pinned: the TypeScript SDK's v1 line (`@modelcontextprotocol/sdk`), its v2 client (`@modelcontextprotocol/client`), the Python SDK (`mcp`) and `rmcp`;
  - cells: each supported revision × each client that speaks it × stdio and Streamable HTTP. Each connects, lists tools, calls a read tool and an Effect against the Proving ground, cancels a long call and reads a docs resource;
  - negotiation cases: an unsupported older revision, an unknown newer one, and a `2026-07-28` request with a missing or stale `_meta` version, each answered as the spec requires;
  - Pre-merge: the newest revision with the TypeScript v2 client over both transports on Linux. Release suite: every cell on Linux, and the stdio cells on every OS;
  - a revision joins the supported list only with its baseline and its cells, in the same pull request.
- **MCP hosts.** Model-driven use of hosts stays in ADR 0065's Evaluations. Claude Desktop, ChatGPT (remote MCP with OAuth), VS Code and Cursor have no scriptable client that runs without a model or a desktop session: connecting each, listing tools and calling one is a manual test at each release candidate that changes MCP code.

### Framework adapters

- **Quick:** `xtask codegen --check`; a snapshot per adapter of the tools as its framework converts them; `ToolsUnitTests` from a pinned `langchain-tests` on every generated Python LangChain tool.
- **Pre-merge**, each tool driven through the framework's own agent loop with the framework's own scripted model, against an Attached engine on the Proving ground:
  - Vercel AI SDK: `generateText` and `ToolLoopAgent` with `MockLanguageModelV4`;
  - Mastra: an `Agent` on the AI SDK mock model Mastra accepts, with Mastra's model-free quick checks;
  - LangChain and LangGraph in TypeScript: `FakeListChatModel` or `FakeStreamingChatModel`, which implement `bindTools`;
  - LangChain and LangGraph in Python: `GenericFakeChatModel`, through one subclass in test support that adds the `bind_tools` it lacks; `ToolsIntegrationTests` on one tool per operation class and on every tool a Journey uses.

  Each test scripts a read, an Effect and a failing call, and asserts the oracle's outcome and the Error code, never a model's wording (ADR 0065). An adapter test proves wiring; operation behaviour belongs to the SDK and engine tests.
- **Versions.** Pre-merge uses the lockfile's version. The Release suite runs the lowest and highest published versions of each declared peer range and one version of each major inside it, on ADR 0024's runtime matrix. Lowest versions come from a test package that pins the framework, since pnpm's `lowest-direct` never lowers peers; Python uses uv's `--resolution lowest-direct`.
- **Frameworks reached only through MCP** (ADR 0011's list): each docs page's example has a model-free test through that framework's own MCP client, which connects, lists tools and calls one tool. Pre-merge when the page, MCP code or the catalog changes; Release suite always.

### Loaders

- **LangChain keeps the loader form**, in TypeScript and Python. The Dataset loader ships inside the one LangChain package per language that also carries the LangChain Framework adapter, as LangChain's provider packages do (`langchain-apify` ships tools, a loader and a retriever together). A retriever was not chosen: a Dataset has no query or ranking.
- **LlamaIndex keeps a Python reader** as Goodfetch's own package. **No TypeScript LlamaIndex loader is shipped**, since LlamaIndex.TS is archived. TypeScript users load Datasets through the LangChain package or the TypeScript SDK. A framework returns to the list when it has a maintained release line.
- **Tests:** a crawl on the Proving ground fills a Dataset; the documents, their metadata and lazy streaming are checked against the oracle's records. Pre-merge, against a real engine.

### Integrations

- **Goodfetch's side:** a Daemon with its HTTP listener on, a token and port 0. Each Integration's tests cover starting a run and a Recipe, fetching Records, one trigger per record range through the thin webhook, no repeated trigger after a resumed run, a refused token, a refused `hello` version (ADR 0066), and every Error code it maps. No Integration test uses `nock` or another mock of Goodfetch.
- **The platform's side:**

| Integration | Quick | Pre-merge, against a real Daemon |
|---|---|---|
| n8n | `n8n-node lint` in strict mode; the verification rules (no runtime dependency, no environment or file access) | a pinned n8n image loads the node; workflows imported and run with `n8n execute --id`; triggers receive the thin webhook |
| Zapier | `zapier-platform validate --without-style` | `createAppTester` runs every trigger, action and search with real HTTP; REST-hook subscribe and unsubscribe |
| Make | the app definition against the Make Apps Editor's JSON schemas; each module mapped to a catalog operation | none: Make runs only in its cloud |
| Pipedream | Pipedream's ESLint rules, build, component-key checks | each component's `run` with a written adapter of Pipedream's `$`, real HTTP |
| Activepieces | lint, build | `createMockActionContext` for each action; a pinned Activepieces image runs one flow per trigger |
| Power Automate | `ConnectorPackageValidator.ps1` at a pinned commit; OpenAPI 2.0 validation | every operation sent by an OpenAPI 2.0 client built from the definition |
| Airbyte source | Airbyte's offline QA checks; manifest validation | the CDK's standard suite for the source's form, plus incremental reads by record number from saved state |

  Integrations run on Linux, where their platforms' runtimes run.

### Live checks

| Live check | When | What it blocks |
|---|---|---|
| Newest conformance release, Inspector and MCP SDK clients (`latest` and prerelease tags) against `main`'s build | nightly | nothing directly |
| Each framework's `latest` and prerelease tags against the adapters | nightly | nothing directly |
| n8n scanner on the published node | after each nightly and release publish | nothing directly |
| Real platform accounts: Zapier's full `validate` and one Zap per trigger and action; one Make scenario per module plus an error scenario; each Pipedream component; the Power Automate connector in a developer environment | weekly, and each release candidate whose Integration changed | nothing directly |

- "Nothing directly" means ADR 0064's two-day rule for unexplained failures applies.
- **Real platform accounts are proposed, not approved.** The owner approves each account (a Zapier developer account with a non-expiring test account, a Make account with a testing app, a Pipedream account, a Power Apps Developer Plan environment) case by case, like other paid test resources. Until then each job exists and is disabled by a repository variable.
- **Reach:** the account job starts an Exposed Daemon with TLS and a token, serving the local Proving ground, behind a Cloudflare Tunnel on a `goodfetch.dev` subdomain. The tunnel and each platform's API host are named endpoints of that job only (ADR 0043).

### Manual tests

At each release candidate that changes them:
- the GUI MCP hosts above;
- `paconn validate`, which needs an interactive device-code login and has no service-principal login;
- the submissions to each platform's listing (n8n Creator Portal, Zapier, Make, Pipedream, Activepieces, Microsoft Partner Center, Airbyte), which people review.

## Considered Options

- **Mocking Goodfetch's API in Integration tests** (Apify's `nock`): rejected. ADR 0062 bans mocks of owned code, and a mock misses the listener's token, `Host` and `Origin` checks (ADR 0017) and the webhook ledger.
- **Only baselining the conformance suite's fixture scenarios** (Apify): rejected. Those protocol paths would never be checked. The fixture server checks them on Goodfetch's own MCP code.
- **A fixture profile in the shipped binary**: rejected. ADR 0059 keeps test-only controls out of the binary, and users would see test tools.
- **An HTTP shim to run the conformance suite over stdio**: rejected. It tests the shim's framing.
- **The TypeScript client for every MCP test**: rejected for the server's own tests. It puts Node into every Rust crate's tests; the TypeScript and Python clients run in the matrix instead.
- **Only the newest MCP revision**: rejected. Hosts embed older SDK versions for months.
- **Calling each adapter tool's `execute` directly**: rejected. It skips the framework's schema conversion and tool loop, where adapters break.
- **Replacing the LangChain loaders with retrievers**: rejected. A Dataset has no query, so a retriever would return a list in a made-up order.
- **A TypeScript LlamaIndex loader on the archived 0.12 line**: rejected. Nothing upstream would fix a break.
- **Real platform accounts in a suite**: rejected. Their inputs move without a commit (ADR 0064).

## Consequences

- **Amends ADR 0027.** "Dataset loaders for LangChain and LlamaIndex in TypeScript and Python" becomes a LangChain Dataset loader in TypeScript and Python, inside the LangChain package that carries the Framework adapter, and a LlamaIndex reader in Python. A note marks the line.
- **Amends ADR 0032's tree.** `integrations/` no longer holds the LangChain loaders (they live in `adapters/`) or a TypeScript LlamaIndex loader. `conformance/mcp/` holds the conformance baselines, the fixture server's catalog and the MCP client matrix (a TypeScript and a Python package). A note marks the line.
- **Adds to ADR 0011.** Each framework that connects through its MCP client has a model-free docs example test.
- **Adds to ADR 0043.** The account job may reach its tunnel host and each platform's API host; the nightly Live checks may reach the npm and PyPI registries to fetch unpinned releases.
- **Adds to ADR 0061.** The Pre-merge and Release placements above.
- **Adds to ADR 0064's Live check table:** the four Live checks above.
- **Adds to ADR 0063:** MCP Journey variants run on nextest with `rmcp`.
- **Adds to ADR 0037:** area 11 (Client surfaces) builds this decision.
- **Assumed interfaces:**
  - [Console and docs test stack](https://github.com/Jbmanllr/automan/issues/282) can run a model-free framework test as a docs page's example;
  - [Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279) counts each Integration action and trigger and each Framework adapter as a feature.
- **Left to area 11's spec:** the MCP server's supported revisions, whether it serves skills, each adapter's peer range, package names, and the Airbyte source's form (Python CDK or Connector Builder manifest).
- **The "Test infrastructure" spec builds:** the fixture server; the client matrix packages; the strict stdout reader; `goodfetch-test-env` starting a Daemon with its HTTP listener and printing its address and token; the pinned n8n and Activepieces images; the Pipedream `$` adapter; the LangChain `bind_tools` subclass; the tunnel job. It confirms two facts first: that the Make Apps Editor's schemas can be used outside the editor, and that Activepieces runs as one container with an embedded database and an in-memory queue. If either fails, that cell falls back to the next option in the preference order.
