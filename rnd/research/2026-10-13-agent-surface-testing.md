# Testing MCP, framework adapters, Integrations and agent skills

**Date:** October 13, 2026. **Ticket:** [Testing MCP, framework adapters, Integrations and agent skills](https://github.com/Jbmanllr/automan/issues/267), map [#262](https://github.com/Jbmanllr/automan/issues/262). Research input only: nothing here is a decision.

**Method.** Primary sources only: official docs, specs, source code read through `gh api`, package registries (npm, PyPI, crates.io) and release pages, all checked on 2026-10-13. Three angles per surface: what competitors and comparable projects ship, what the platform leaders recommend, and what users complain about. Versions are the latest published on 2026-10-13 unless a date says otherwise. Claims marked **(unverified)** were not confirmed against a primary source. "Fit for Goodfetch" notes are interpretation and are kept apart from the facts.

## Question

What official or standard test tooling exists for each agent-facing and third-party surface Goodfetch ships, and how do comparable projects use it?

The surfaces (ADRs 0011, 0027, 0032): the MCP server over stdio and Streamable HTTP; adapters for the Vercel AI SDK, LangChain and LangGraph (TypeScript and Python) and Mastra; Integrations for n8n, Airbyte (source), LangChain and LlamaIndex loaders, Zapier, Make, Pipedream, Activepieces and Power Automate; agent skills; the CLI as coding agents use it.

## Summary: the facts a decision needs

- **MCP has an official conformance suite, but it tests servers over HTTP only.** `@modelcontextprotocol/conformance` connects to a running server by URL; stdio support is an open request ([conformance#258](https://github.com/modelcontextprotocol/conformance/issues/258), opened 2026-04-29). Its requirement sets per spec revision (`2025-11-25`, `2026-07-28`) and its `tier-check` exist only in the `0.2.0-alpha` line (latest `0.2.0-alpha.12`, 2026-10-01; latest stable `0.1.16`, 2026-03-30). About a dozen server scenarios call fixture tools such as `test_simple_text`, so a product server must list them in an expected-failures baseline, as Apify's MCP server does.
- **The MCP Inspector CLI is the official scriptable smoke test, and it covers stdio.** Version 2.10.1 (2026-10-08) connects over stdio, SSE or Streamable HTTP, prints JSON, has stable exit codes, and has `--strict` for tool-schema portability and `--verify` for skills served over MCP.
- **Comparable MCP servers test with a real SDK client.** The reference servers use an in-memory transport plus a boot smoke of the built artifact. Playwright MCP uses Playwright Test fixtures over stdio. Firecrawl pins the tool surface with contract snapshots and caps description length. Apify runs the official suite in CI with one baseline per spec revision, plus model-driven evals.
- **No framework offers a standard test suite for third-party tool adapters in TypeScript.** The Vercel AI SDK ships mock models (`ai/test`, `MockLanguageModelV4`) that can drive a tool call without a live model. Mastra ships evals with deterministic "quick checks". LangChain's standard tests (`langchain-tests` 1.1.9) cover Python tools and retrievers; the JavaScript package is private and covers chat models only.
- **Workflow platforms mostly test by hand inside their own product, and certify by human review.**
  - n8n verifies nodes by human review in its Creator Portal, after a lint and a scanner pass. It forbids runtime dependencies and environment or file access, and since 2026-05-01 it requires provenance-signed publishing.
  - Zapier requires that every trigger and action ran in a live Zap, and asks for a non-expiring test account for its staff.
  - Microsoft certifies through Partner Center, with a 48-hour test window and 10 to 14 days of deployment.
  - Make reviews test scenarios and their logs.
  - Pipedream and Activepieces review pull requests to their own repositories.
- **Two Integrations named in ADR 0027 meet a headwind.** LangChain's integration guide lists document loaders under "Not these ❌" ("High maintenance burden"), while tools and retrievers are encouraged. LlamaIndex.TS is deprecated and its repository archived; the Python LlamaIndex repository no longer accepts new integration packages, so each one is published from its owner's repository.
- **Airbyte has an offline-capable standard suite.** The CDK's "FAST" standard tests (`airbyte-cdk` 7.35.0) run spec, check, discover and read. Airbyte now accepts new API sources only as Connector Builder YAML manifests. Its acceptance tests need sandbox credentials.
- **For agent skills, only structure can be checked without a model.**
  - Offline checks: `claude plugin validate` (no authentication needed, per the MCP reference servers' CI), the Agent Skills reference validator `skills-ref validate`, and frontmatter limits.
  - Behaviour needs live model calls: `claude plugin eval` (Claude Code v2.1.269+, three runs per case, with a no-plugin baseline), promptfoo's `skill-used` assertion, Inspect AI's `skill()` tool, and OpenAI's `codex exec --json` recipe.
  - Anthropic's own guide says "There is not currently a built-in way to run these evaluations" for its evaluation format.
- **The competitors test these surfaces unevenly.**
  - Apify tests the most: official MCP conformance, n8n tests with `nock`, Zapier `validate` plus `nock`, LangChain standard tests, and an offline Power Automate validator it wrote itself.
  - Firecrawl tests its MCP server thoroughly but its n8n node has no tests.
  - Browserbase's n8n node is lint-only. Its skills have an offline validator, and 2 of its 18 skills have evals.
  - Crawl4AI's MCP bridge serves SSE and WebSocket, and CI runs none of its MCP tests.

## A. MCP server (stdio and Streamable HTTP)

### Official tooling

**The conformance suite** ([modelcontextprotocol/conformance](https://github.com/modelcontextprotocol/conformance), README read on 2026-10-13):
- It "validates MCP implementations" in two modes. For servers it connects "to the running server as an MCP client"; for clients it starts a test server and runs the client against it.
- Server mode takes `--url` only. The SDK integration guide says "Your server must be **running before** invoking the conformance tool" and that you manage start, health-check and cleanup yourself ([SDK_INTEGRATION.md](https://github.com/modelcontextprotocol/conformance/blob/main/SDK_INTEGRATION.md)).
- stdio is not covered. [Issue #258](https://github.com/modelcontextprotocol/conformance/issues/258) "Add support for a stdio testing" is open. A commenter notes that the only path today is "to wrap the stdio server in an HTTP shim", and another that stdio "is not currently expressible in the scenario contract".
- Every scenario also checks each JSON-RPC message against the spec's JSON schema for the negotiated version (`wire-schema-valid`).
- `--expected-failures <yaml>` turns known failures into a baseline. An unlisted failure exits 1, and a listed scenario that starts passing also exits 1 as a stale entry. Entries can name a single check (`scenario:check-id`).
- `--requirements <revision>` runs what one spec revision requires, frozen at its release. `requirements/2026-07-28.yaml` says it is anchored at `0.2.0-alpha.10`, published 2026-07-27. The 2026-07-28 revision is stateless, with per-request `_meta`; revisions up to 2025-11-25 use the initialize handshake. A scenario that applies to both must run once under each.
- `tier-check` scores an SDK against SEP-1730 tiers. Extensions such as Tasks and MCP Apps are "not required for any tier".
- A composite GitHub Action is provided (`uses: modelcontextprotocol/conformance@…`, inputs `mode`, `url`, `expected-failures`, `suite`).
- Versions: npm `latest` is `0.1.16` (2026-03-30) and `alpha` is `0.2.0-alpha.12` (2026-10-01). GitHub releases stop at v0.1.16.
- Fixture dependence: the server scenarios ask the server under test to implement named tools. For example, `src/scenarios/server/tools.ts` says "Implement tool `test_simple_text` with no arguments".
- The official Rust SDK runs the suite. `known-sdks.ts` builds `cargo build -p mcp-conformance` and serves `http://localhost:3000/mcp`, and [rust-sdk](https://github.com/modelcontextprotocol/rust-sdk/tree/main/.github/workflows) has a `conformance.yml` workflow. The latest `rmcp` is 3.5.1 (crates.io, 2026-10-05).
- Spec revisions published: `2025-11-25` and `2026-07-28` (released 2026-07-28), per the [spec repo releases](https://github.com/modelcontextprotocol/modelcontextprotocol/releases).

**The MCP Inspector CLI** ([modelcontextprotocol/inspector](https://github.com/modelcontextprotocol/inspector), v2.10.1, 2026-10-08):
- The README calls the CLI "a scriptable command-line client for automation, CI, and fast agent feedback loops".
- The [smoke-testing guide](https://github.com/modelcontextprotocol/inspector/blob/main/docs/cli-smoke-testing.md) says a smoke test "is deliberately not a conformance suite — it is the check you run on every commit and every deploy". The server "can be a **stdio** command or an **HTTP/SSE** URL".
- `--method initialize` is "a connect-only probe".
- `--format json` prints one JSON envelope.
- Exit codes are stable: 3 for authentication required, 4 for unreachable, 5 for a tool error (`isError:true`), and 6 for a `--strict` schema problem.
- `--stored-auth-only` never starts interactive OAuth, so CI cannot hang. The interactive flow otherwise waits "up to 15 minutes".
- The guide says "In CI, pin an exact version", because `npx` resolves the latest release.
- `--strict` with `tools/list` reports tool schemas that are "perfectly legal JSON Schema and still be refused by the client", such as a bare `true` schema ([CLI README](https://github.com/modelcontextprotocol/inspector/blob/main/clients/cli/README.md)).
- `--verify` with `skills/list` or `skills/get` runs "the SEP-2640 conformance, digest and frontmatter checks" over skills served through MCP, and exits 7 on failure (same README).

**Microsoft MCP server certification** ([Learn, preview, ms.date 2026-09-25](https://learn.microsoft.com/en-us/microsoft-copilot-studio/mcp-certification)):
- It is open to verified publishers who "own or control the MCP server endpoint".
- The package needs a manifest, a tools file, `intro.md` and an Azure Key Vault authentication configuration.
- Steps: "Automated validation", then a "Functional and safety review", where "Evaluation proof can help accelerate review".
- "DCR isn't supported today."
- The legacy process remains available "through October 31, 2026".

### How comparable servers test

- **Reference servers** ([modelcontextprotocol/servers](https://github.com/modelcontextprotocol/servers)):
  - The everything server's suite links "a real SDK `Client` to a real server … over `InMemoryTransport`, and asserts only on what crosses the wire" ([harness.ts](https://github.com/modelcontextprotocol/servers/blob/main/src/everything/__tests__/harness.ts)). It also has `stdio.test.ts`, `streamable-http.test.ts` and `sse.test.ts`.
  - A separate boot smoke starts each built server "the way a user's client does, over every transport it implements", and calls one tool. Its reason: the unit suites import the source, so "a wrong `bin` path … or a server that dies during the `initialize` handshake all pass the unit tests and fail here" ([smoke-servers.mjs](https://github.com/modelcontextprotocol/servers/blob/main/scripts/smoke-servers.mjs)).
- **Playwright MCP** (v0.0.83, 2026-09-28). The core "was moved to the Playwright monorepo" ([CONTRIBUTING](https://github.com/microsoft/playwright-mcp/blob/main/CONTRIBUTING.md)). There, [`tests/mcp/fixtures.ts`](https://github.com/microsoft/playwright/blob/main/tests/mcp/fixtures.ts) starts the server through the SDK's `StdioClientTransport`, against a local `TestServer`. Custom matchers (`toHaveResponse`) compare the generated code, page state and accessibility snapshot. About 100 spec files cover tools, the CLI (`cli-*.spec.ts`), `http.spec.ts` and `sse.spec.ts`.
- **Browserbase MCP** ([mcp-server-browserbase](https://github.com/browserbase/mcp-server-browserbase), v3.0.0, 2026-03-31, now **archived**: "This repository is archived and no longer maintained").
  - It had one vitest smoke test listing "exactly 6 tools … via STDIO".
  - Its evals used the third-party `mcpvals` library (0.4.0, last published 2025-11-04). Natural-language workflows, such as "Open a browser and go to example.org", were run against live Browserbase and model keys, with `passThreshold: 0.7`.
- **Firecrawl MCP** ([firecrawl-mcp-server](https://github.com/firecrawl/firecrawl-mcp-server), `firecrawl-mcp` 3.29.1). Tests use `node:test` against the built server, spawned over stdio and HTTP, with the Firecrawl API replaced by a local fake (`helpers/exchange-api.mjs`).
  - `contract-snapshot.test.mjs` pins "the tool surface, schemas, annotations, server instructions and HTTP surface of every way the server can be started".
  - `mcp-description-budget.test.mjs` asserts each tool description is at most 2,048 characters, because, per its comment, "Claude Code truncates each tool description … at 2,048 characters" **(unverified against Claude Code docs)**.
- **Apify MCP** ([apify-mcp-server](https://github.com/apify/apify-mcp-server)):
  - Vitest unit tests, plus integration tests per transport (`stdio.test.ts`, `actor.server_streamable.test.ts`, `actor.server_stateless.test.ts`).
  - A conformance job runs `conformance sdk --path . --mode server --suite all --spec-version <rev> --expected-failures <file>` for both `2026-07-28` and `2025-11-25`, pinned to `@modelcontextprotocol/conformance` 0.2.0-alpha.9 ([test_conformance.sh](https://github.com/apify/apify-mcp-server/blob/master/scripts/test_conformance.sh)).
  - Its [2026-07-28 baseline](https://github.com/apify/apify-mcp-server/blob/master/scripts/conformance_expected_failures_2026_07_28.yaml) lists the scenarios that need fixture tools and resources ("Needs fixture tool `test_simple_text`", and so on). It also notes that extension-tagged scenarios (Tasks) are never run when `--spec-version` is passed.
  - Model-driven evals run Claude Code against the server: `tool-call` items "check the first attempted call without executing it", and `agent` items "run to completion and use an LLM judge". Results go to Langfuse, with separate PR and merge datasets ([evals/README.md](https://github.com/apify/apify-mcp-server/blob/master/evals/README.md)).
- **Crawl4AI** (v0.9.4, 2026-09-23). Its Docker server's [`mcp_bridge.py`](https://github.com/unclecode/crawl4ai/blob/main/deploy/docker/mcp_bridge.py) exposes "WS+SSE MCP endpoints"; Streamable HTTP does not appear in the file. `tests/mcp/test_mcp_sse.py` is a script that prints `list_tools()`, not an assertion. The only CI workflow running pytest runs the security suite (`deploy/docker/tests/test_security_*.py`).

### What users complain about

- **Tool schemas that clients reject.** Firecrawl MCP issues report a `$dynamicRef` schema the client cannot read ([#112](https://github.com/firecrawl/firecrawl-mcp-server/issues/112)), "Gemini function calling rejects firecrawl_scrape due to anyOf schema conflict" ([#164](https://github.com/firecrawl/firecrawl-mcp-server/issues/164)), and a schema "that admits no keys, breaking JSON extraction for schema-validating MCP clients" ([#373](https://github.com/firecrawl/firecrawl-mcp-server/issues/373)). Apify reports value constraints "stripped from Actor tool schemas" ([#1266](https://github.com/apify/apify-mcp-server/issues/1266)).
- **Unannounced surface changes.** A Firecrawl MCP issue asks whether a `tools/list` change, with "one tool removed", was intentional ([#524](https://github.com/firecrawl/firecrawl-mcp-server/issues/524)).
- **No stdio conformance.** See conformance#258 above.

### Comparison

| Tool | What it checks | Offline or needs an account | Certification |
|---|---|---|---|
| `@modelcontextprotocol/conformance` server mode | Protocol scenarios, wire messages against the spec schema, per-revision requirement sets | Offline against a local HTTP server; stdio not supported | SEP-1730 SDK tiers (`tier-check`), for SDKs rather than servers |
| MCP Inspector CLI 2.10.1 | Connect, list, call; exit codes; `--strict` schema portability; `--verify` skills over MCP | Offline; stdio, SSE or Streamable HTTP | None |
| Official SDK client in the server's own tests (reference servers, Playwright, Firecrawl, Apify) | Behaviour of each tool over in-memory, stdio or HTTP transports | Offline when the upstream is faked | None |
| Contract snapshot of `tools/list` (Firecrawl) | Tool names, schemas, annotations and instructions stay unchanged unless the snapshot is updated | Offline | None |
| Model-driven evals (Apify with Claude Code and Langfuse; Browserbase with `mcpvals`) | Whether an agent picks and uses the tools | Live model, plus the vendor's account | None |
| Microsoft MCP certification (preview) | Package validation, then functional and safety review | Partner Center account; a remote server reachable by Microsoft | Yes, for Copilot Studio and Azure Foundry |

### Fit for Goodfetch

- The suite fits the Streamable HTTP transport as Apify uses it: the Daemon's endpoint, the pinned alpha line, and one baseline per revision.
  - The fixture-tool scenarios would be baselined as "not applicable", as Apify does.
  - The alternative is a test-only catalog profile that exposes the fixture tools, which is what the official Rust SDK's `mcp-conformance` crate does for its own server.
  - Pinning matters, because the required features live in an alpha line that moves.
- stdio needs a different path until conformance#258 lands. Options: in-process tests with a real SDK client (reference-servers style), plus a boot smoke of the released binary through the Inspector CLI (pinned).
- Since every tool comes from the Operation catalog (ADR 0011), Firecrawl's contract snapshot would amount to a snapshot of the catalog's MCP projection per profile. `--strict` and a description-length budget can run on that snapshot.
- ADR 0032 already has MCP cases run in their own test runner and write the conformance result document. Model-driven evals are the only MCP tests that need the network: a live model, and Goodfetch Cloud only if it is the target. Under ADR 0032 they would sit with the other jobs that may reach named endpoints, not on every pull request.

## B. Framework adapters (Vercel AI SDK, LangChain and LangGraph, Mastra)

### Facts

- **Vercel AI SDK** (`ai` 7.0.138). [`packages/ai/test/index.ts`](https://github.com/vercel/ai/blob/main/packages/ai/test/index.ts) exports, among others, `MockLanguageModelV4`, `MockLanguageModelV3`, `MockEmbeddingModelV4`, `mockValues` and `mockId`.
  - The [testing page](https://ai-sdk.dev/docs/ai-sdk-core/testing) states: "To enable you to unit test your code that uses the AI SDK, the AI SDK Core includes mock providers and test helpers."
  - Its `ToolLoopAgent` example uses a `MockLanguageModelV4` whose first `doGenerate` returns a `tool-call`, then checks that the tool ran.
  - No certification or listing programme for tool packages was found **(unverified absence)**.
- **LangChain, Python** ([`langchain-tests`](https://github.com/langchain-ai/langchain/tree/master/libs/standard-tests) 1.1.9, 2026-05-21):
  - `unit_tests` covers chat models, embeddings and tools. `ToolsUnitTests` has `test_init`, `test_init_from_env`, `test_has_name`, `test_has_input_schema` and `test_input_schema_matches_invoke_params`.
  - `integration_tests` covers base stores, caches, chat models, embeddings, indexers, retrievers, sandboxes, tools and vector stores. `ToolsIntegrationTests` checks that invoke output matches the output schema; `RetrieversIntegrationTests` checks `k` and that invoke returns documents.
  - There is **no document-loader suite**.
  - The [docs](https://docs.langchain.com/oss/python/contributing/standard-tests-langchain) define unit tests as "without access to external services" and recommend "pinning to the latest version of langchain-tests to avoid unexpected changes".
- **LangChain, JavaScript.** [`internal/standard-tests`](https://github.com/langchain-ai/langchainjs/tree/main/internal/standard-tests) is `@langchain/standard-tests` 0.0.23, `"private": true`, and covers chat models only. npm returns 404 for the name.
- **LangGraph.** LangGraph agents call LangChain tools. No separate standard suite for tools was found **(unverified absence)**.
- **LangChain's listing rules** ([integration guide](https://docs.langchain.com/oss/python/contributing/integrations-langchain)):
  - "All new integrations must be published as independent packages to PyPI."
  - Packages under 50,000 monthly downloads are listed through an issue; "Hosted guide (50,000+ monthly downloads, or featured by maintainers)".
  - Tools and retrievers are encouraged. "Document Loaders: High maintenance burden" sits under "Not these ❌".
- **Mastra** (`@mastra/core` 1.76.0, `@mastra/evals` 1.10.6):
  - `runEvals` runs a dataset through an agent or workflow with scorers ([running in CI](https://github.com/mastra-ai/mastra/blob/main/docs/src/content/en/docs/evals/running-in-ci.mdx)).
  - `@mastra/evals/vitest` adds `expectEvals(...).toPass(0.8)` and a reporter ([Vitest integration](https://github.com/mastra-ai/mastra/blob/main/docs/src/content/en/docs/evals/vitest-integration.mdx)).
  - "Quick Checks" such as `checks.calledTool`, `checks.toolOrder` and `checks.noToolErrors` "run instantly without an LLM" ([quick checks](https://github.com/mastra-ai/mastra/blob/main/docs/src/content/en/docs/evals/quick-checks.mdx)). The agent itself still calls its model.
  - Mastra's own tests drive agents with the AI SDK's `MockLanguageModelV2` ([mock-models.ts](https://github.com/mastra-ai/mastra/blob/main/workflows/_test-utils/src/mock-models.ts)). Mastra's docs do not show users how to do this **(unverified absence)**.
- **Stagehand's adapters** ([packages/integrations](https://github.com/browserbase/stagehand/tree/main/packages/integrations)):
  - The tool contract ("tool descriptions, schemas, runtime validators, and the agent system prompt") "is defined once in `core/` and imported everywhere else, never restated".
  - The Vercel AI SDK and Mastra examples consume it over MCP stdio (`createMCPClient`, `MCPClient`).
  - Each example's test covers "only what is unique to this example", for instance environment variables reaching the spawned server. It calls `tools.snapshot.execute(...)` directly, with no model. The contract is pinned by `core/tests/facade-contract.test.ts`.
- **langchain-apify** (0.1.7, 2026-08-10). Its integration test subclasses `ToolsIntegrationTests` ([test_tools.py](https://github.com/apify/langchain-apify/blob/main/tests/integration_tests/test_tools.py)). Unit tests also cover document loaders and retrievers.

### Comparison

| Tool | What it checks | Offline or needs an account | Certification |
|---|---|---|---|
| `ai/test` mock models (AI SDK 7) | Tool wiring inside `generateText` or `ToolLoopAgent`, with scripted model replies | Offline | None |
| `langchain-tests` `ToolsUnitTests` | Construction, name, input schema, example invoke parameters | Offline | None; listing through an issue |
| `langchain-tests` `ToolsIntegrationTests`, `RetrieversIntegrationTests` | Output matches schema; retriever returns documents and honours `k` | Needs the external service | None |
| `@langchain/standard-tests` (JavaScript) | Chat models only | Not published | None |
| Mastra `runEvals`, quick checks, `@mastra/evals/vitest` | Tool called, order, errors, text; LLM scorers optional | Quick checks need no LLM to score, but the agent run needs a model (or a mock) | None |

### Fit for Goodfetch

- The Stagehand layout matches ADR 0011's generated adapters. The contract is pinned once from the Operation catalog; each adapter's own test checks only its framework wiring. With `ai/test` mock models, a Vercel AI SDK or Mastra adapter can run a full tool call against a local engine with no model.
- For Python LangChain tools, `ToolsUnitTests` costs a few lines per tool class. `ToolsIntegrationTests` could run against an engine serving the Proving ground instead of a vendor account. TypeScript LangChain tools have no published suite to reuse.

## C. Integrations: each platform's toolkit and certification

### n8n

- **Tooling.** `@n8n/node-cli` 0.52.2 (`n8n-node new | build | dev | lint | release | cloud-support`). The [n8n-node tool page](https://docs.n8n.io/connect/create-nodes/build-your-node/using-the-n8n-node-tool) and [node-cli README](https://github.com/n8n-io/n8n/blob/master/packages/%40n8n/node-cli/README.md) describe it.
  - `dev` "runs n8n with your node". "From n8n 3.0, it starts that instance in a container, so you need Docker or Podman installed". The current n8n release is 2.43.4, so 3.0 is not out yet **(n8n 3.0 date unverified)**.
  - `cloud-support enable` sets "Strict mode", which "enforces the default ESLint configuration and community node rules required for n8n Cloud verification".
  - The linter is `@n8n/eslint-plugin-community-nodes` 0.36.0 ([node linter](https://docs.n8n.io/connect/create-nodes/test-your-node/node-linter)).
- **Testing.** The [test section](https://docs.n8n.io/connect/create-nodes/test-your-node) has two parts: run the node locally (manual) and the linter. No unit-test runner is offered to community nodes. n8n's own `NodeTestHarness` (with `nock`) lives in `packages/core/nodes-testing/`, but it is not in the published `n8n-core` 2.16.1 tarball (checked by listing the tarball).
- **Scanner.** `npx @n8n/scan-community-package` (0.39.0) "Checks npm provenance and runs static analysis" ([README](https://github.com/n8n-io/n8n/tree/master/packages/%40n8n/scan-community-package)). It downloads the *published* package and the source named by its provenance attestation (`scanner.mjs`), so it needs network access and an already-published version.
- **Verification** ([guidelines](https://docs.n8n.io/connect/create-nodes/build-your-node/reference/verification-guidelines)):
  - The package must not include "any external dependencies".
  - The code "must not interact with environment variables or attempt to read/write files".
  - The licence must be MIT, with English only.
  - The scanner must pass.
  - "From May 1st 2026 you must publish **ALL** community nodes using a GitHub action and include a provenance statement".
  - Submission goes through the [Creator Portal](https://creators.n8n.io/nodes), and n8n reviews it.
- **Complaints.** Forum threads report a submission "still under review" since December 2025 ([thread](https://community.n8n.io/t/community-node-for-securevector-is-still-under-review-submitted-back-in-dec-2025/253757)) and one stuck in "Automated review in progress" for days ([thread](https://community.n8n.io/t/community-node-submission-stuck-in-automated-review-for-several-days/256365)).
- **Prior art.**
  - Apify's node (`n8n-nodes-apify` 0.8.1) runs lint, type-check, build and `jest` with `nock` and its own `executeWorkflow` helper ([ci.yml](https://github.com/apify/n8n-nodes-apify/blob/master/.github/workflows/ci.yml), `nodes/Apify/__tests__/`).
  - Firecrawl's (`@mendable/n8n-nodes-firecrawl` 2.2.0) has no test files and only a publish workflow.
  - Browserbase's (`n8n-nodes-browserbase` 1.4.1) CI runs `npm ci`, `lint` and `build`.

### Zapier

- **Tooling.** `zapier-platform-cli` 19.2.0 (2026-10-07). The command is now `zapier-platform`; the docs mark `zapier test` "(deprecated)" ([CLI tutorial](https://docs.zapier.com/integrations/quickstart/cli-tutorial)).
  - `zapier-platform test` "is a wrapper around `npm test` that also validates the structure of your integration" ([cli.md](https://github.com/zapier/zapier-platform/blob/main/packages/cli/docs/cli.md)).
  - `zapier-platform validate` runs "the standard validation routine powered by json-schema". `--without-style` will "Forgo pinging the Zapier server to run further checks". The CLI posts the definition to Zapier's `/check` endpoint, anonymously when not logged in, for "integration checks" (`validateApp` in [api.js](https://github.com/zapier/zapier-platform/blob/main/packages/cli/src/utils/api.js)).
  - Unit tests use `zapier.createAppTester(App)`; the [testing page](https://docs.zapier.com/integrations/build-cli/testing-and-debugging) shows HTTP mocked with Nock.
- **Publishing requirements** ([page](https://docs.zapier.com/integrations/publish/integration-publishing-requirements)):
  - Every trigger, action and search tested in a Zapier account: "Every test Zap has been turned on and has at least one successful run".
  - "Integrations using sandbox/test/dev endpoints will not be published".
  - HTTPS only, and "Your app has been fully launched to the public".
  - A test account that "must be non-expiring", for Zapier's integration-testing address.
  - An admin team member with an email address on the API owner's domain.
- **Prior art.** Apify's [test.yml](https://github.com/apify/apify-zapier-integration/blob/master/.github/workflows/test.yml) runs `zapier-platform validate --without-style`, `lint`, then `mocha` tests with `nock`. A second job runs with a live test-user token.

### Make

- **Tooling.** Make custom apps are edited in Make's web editor or the "Make Apps Editor" VS Code extension. Local development "doesn't provide access to … seamless integration with Scenario Builder for continuous testing". You deploy to a separate "testing app" origin and test it in Scenario Builder ([local development](https://developers.make.com/custom-apps-documentation/make-apps-editor/apps-sdk/local-development-for-apps); pages marked beta, per the search summary **(unverified)**). No local test runner was found.
- **Review.** The [prerequisites](https://developers.make.com/custom-apps-documentation/app-review/prerequisites) say: "you must create test scenarios to show that the custom app works". Run each module in at least one scenario, and add one scenario "that produces an error message". The logs must not contain personal data, and "the reviewer won't be able to access old logs", so scenarios are re-run right before review. Make's QA team then reviews the app ([overview](https://developers.make.com/custom-apps-documentation/app-review)).
- **Complaint.** The `makecomapp.json` origins, which hold app IDs, end up in version control, and a community thread asks whether to track them ([thread](https://community.make.com/t/leave-origins-config-out-of-version-control/56760)).

### Pipedream

- **Registry.** Components are contributed by pull request to [PipedreamHQ/pipedream](https://github.com/PipedreamHQ/pipedream), and a Pipedream team member reviews them ([CONTRIBUTING](https://github.com/PipedreamHQ/pipedream/blob/master/CONTRIBUTING.md)).
- **Pull-request checks** ([pull-request-checks.yaml](https://github.com/PipedreamHQ/pipedream/blob/master/.github/workflows/pull-request-checks.yaml), [components-pr.yaml](https://github.com/PipedreamHQ/pipedream/blob/master/.github/workflows/components-pr.yaml)): ESLint on changed files, a TypeScript build, component-key and duplicate-key checks, an advisory version-bump check, a publish dry-run, and spellcheck.
- **Tests.** The [guidelines](https://pipedream.com/docs/components/contributing/guidelines) state: "Pipedream does not currently support unit tests to validate that changes to app files are backwards compatible with existing components." The `components/` tree on `master` holds 3,401 app directories and no `*.test.*` file (git tree listing, 2026-10-13). `pd dev` and `pd publish` work against a deployed component in a Pipedream account ([CLI reference](https://pipedream.com/docs/components/contributing/cli/reference); account requirement inferred, **unverified**).

### Activepieces

- **Tests.** "Testing pieces is **optional** but recommended for complex logic", with Vitest and `createMockActionContext` from `@activepieces/pieces-framework` (0.32.0) ([testing pieces](https://github.com/activepieces/activepieces/blob/main/docs/build-pieces/misc/testing-pieces.mdx)).
- **Contribution.** "A maintainer will review your work closely"; after merge, "An automatic GitHub action will package it and create an npm package" ([contribute](https://github.com/activepieces/activepieces/blob/main/docs/build-pieces/sharing-pieces/contribute.mdx)). Private pieces are published to an instance with `npm run publish-piece-to-api` and an API key ([publish](https://github.com/activepieces/activepieces/blob/main/docs/build-pieces/misc/publish-piece.mdx)). No certification programme was found **(unverified absence)**.

### Power Automate (Power Platform custom connectors)

- **`paconn`.** `paconn validate --api-def …` sends the OpenAPI file to the Power Apps service with `enable_certification_rules=True` ([validate.py](https://github.com/microsoft/PowerPlatformConnectors/blob/dev/tools/paconn-cli/paconn/operations/validate.py)), so it needs `paconn login`. Login uses the device-code flow, and "Service Principle authentication is not supported" ([README](https://github.com/microsoft/PowerPlatformConnectors/blob/dev/tools/paconn-cli/README.md)). The last PyPI release is 0.0.21, uploaded 2023-02-20.
- **`pac connector`.** It has `create`, `download`, `init`, `list` and `update`, and no validate command ([reference, ms.date 2026-02-25](https://learn.microsoft.com/en-us/power-platform/developer/cli/reference/connector)).
- **The connectors repository.** Pull requests to [microsoft/PowerPlatformConnectors](https://github.com/microsoft/PowerPlatformConnectors) run [`swagger_validation.yml`](https://github.com/microsoft/PowerPlatformConnectors/blob/dev/.pipelines/swagger_validation.yml). It posts each changed `apidefinition.swagger.json` to a hosted validator, with `IsCertificationValidation` for certified and independent-publisher folders, and then runs a change validation of the old swagger against the new one. `scripts/ConnectorPackageValidator.ps1` checks a package zip offline.
- **Verified publisher certification** ([process, ms.date 2026-06-30](https://learn.microsoft.com/en-us/connectors/custom-connectors/submit-for-certification)):
  - "you must open-source the connector" in Microsoft's repository.
  - Submission goes through Partner Center, with review in "24 to 48 hours".
  - A preview-environment link "is live for only 48 business hours" for your own testing; then "Go live".
  - Deployment takes "10 to 14 days", as a Premium connector.
  - The [overview](https://learn.microsoft.com/en-us/connectors/custom-connectors/submit-certification) separates verified publishers ("Owns the underlying service") from independent publishers, and says connectors to external infrastructure "are automatically assigned as *Premium tier connectors*".
- **Prior art.** Apify's connector repository runs its own offline `scripts/validate.py`, which validates "against Microsoft certification policies", needs "no third-party dependencies" and runs in CI ([validate.yml](https://github.com/apify/apify-microsoft-power-automate-integration/blob/main/.github/workflows/validate.yml)).

### Airbyte (source)

- **Testing pyramid** ([testing connectors](https://github.com/airbytehq/airbyte/blob/master/docs/platform/connector-development/testing-connectors/README.md)):
  - QA checks (static).
  - Unit tests, which "don't require access to the source".
  - Integration tests.
  - Connector Acceptance Tests, for which "Credentials to a source/destination sandbox account are **required**".
  - Regression tests, now "(deprecated)".
  - "Passing tests are required to merge a connector pull request."
- **CDK standard tests.** `airbyte-cdk` 7.35.0 (2026-10-09) ships "FAST Airbyte Standard Tests" ([`airbyte_cdk/test/standard_tests`](https://github.com/airbytehq/airbyte-python-cdk/tree/main/airbyte_cdk/test/standard_tests)). `SourceTestSuiteBase` has `test_check`, `test_discover`, `test_spec`, `test_basic_read` and `test_fail_read_with_bad_catalog`; `DeclarativeSourceTestSuite` covers manifests. They run through `airbyte-cdk connector test`. `airbyte_cdk.test.mock_http` mocks HTTP.
- **QA checks** ([list](https://github.com/airbytehq/airbyte/blob/master/docs/community/contributing-to-airbyte/resources/qa-checks.md)) include "Connectors must be licensed under MIT or Elv2", "Connectors must use HTTPS only", documentation sections, and version increments.
- **Contribution scope** ([contributing](https://github.com/airbytehq/airbyte/blob/master/docs/community/contributing-to-airbyte/README.md)): Airbyte accepts "New API connectors built with the Connector Builder" (YAML). It is "not reviewing/accepting new Java connectors".

### LangChain loaders and LlamaIndex readers

- **LangChain.** There is no document-loader standard test, and loaders are a discouraged integration type (see B).
- **LlamaIndex, Python.**
  - CONTRIBUTING says: "**we are no longer accepting new integration packages** in this repository. New integrations should be maintained in their own repositories and published to PyPI independently". Also: "If you're integrating with a remote system, **mock** it", and in-repo CI fails "if test coverage is less than 50%" ([CONTRIBUTING](https://github.com/run-llama/llama_index/blob/main/CONTRIBUTING.md)).
  - There is no standard reader suite. The Apify reader's test only asserts that `BaseReader` is in the class's ancestry ([test_readers_apify.py](https://github.com/run-llama/llama_index/blob/main/llama-index-integrations/readers/llama-index-readers-apify/tests/test_readers_apify.py)).
  - `llama-index-readers-web` 0.7.0 is `GPL-3.0-or-later`; `llama-index-readers-apify` 0.6.0 is MIT.
- **LlamaIndex, TypeScript.** [LlamaIndex.TS](https://github.com/run-llama/LlamaIndexTS) is archived. Its README states: "**This project is deprecated and no longer maintained.**" The last push was 2026-03-11, and the `llamaindex` npm package is at 0.12.1 (last modified 2025-12-31).

### Comparison

| Platform | Tool | What it checks | Offline or needs an account | Certification |
|---|---|---|---|---|
| n8n | `n8n-node lint` with `@n8n/eslint-plugin-community-nodes` (strict mode) | Node, credential and `package.json` rules | Offline | Required for verification |
| n8n | `n8n-node dev` | Manual run in a local n8n (a container from n8n 3.0) | Offline (Docker or Podman) | None |
| n8n | `@n8n/scan-community-package` | Provenance plus static analysis of the published tarball and its attested source | Network; the package must be published | Must pass for verification |
| n8n | Creator Portal | Human review against the guidelines | n8n account | **Verified** node |
| Zapier | `createAppTester` with Nock, `zapier-platform test` | Your unit tests, plus schema validation | Offline when mocked | None |
| Zapier | `zapier-platform validate` | JSON-schema structure, plus Zapier-side "integration checks" | Schema offline; checks need the network (anonymous or logged in) | Feed publishing review |
| Zapier | Publishing review | Live Zaps with successful runs, production HTTPS endpoint, test account for Zapier staff | Zapier account; a public production service | **Public** listing |
| Make | Scenario Builder on a testing app | Manual scenarios and logs | Make account | QA **review** for public apps |
| Pipedream | ESLint, build, key and version checks in PR CI | Static structure | Offline | Pull-request review into the registry |
| Pipedream | `pd dev`, `pd publish` | Manual run of a deployed component | Pipedream account (inferred) | None |
| Activepieces | Vitest with `createMockActionContext` (optional) | Your action logic | Offline | Pull-request review; none beyond |
| Power Automate | `paconn validate` | Swagger against certification rules | Microsoft login (device code; no service principal) | Input to certification |
| Power Automate | Repository swagger pipeline, `ConnectorPackageValidator.ps1` | Hosted validator plus breaking-change check; package layout offline | Pipeline on Microsoft's repository; script offline | Part of certification |
| Power Automate | Partner Center certification | Package review, publisher's 48-hour test, deployment | Partner Center seller account; owning the service | **Certified** (Premium) |
| Airbyte | CDK FAST standard tests, `mock_http` | Spec, check, discover, read | Offline when mocked; real reads need the source | None |
| Airbyte | Connector Acceptance Tests, QA checks | Protocol conformance; packaging, docs and licence | Sandbox credentials; QA offline | Merge gate for the Airbyte catalog |
| LangChain loaders | (none) | No loader suite | – | Listing only; loaders discouraged |
| LlamaIndex readers | pytest with mocks (convention) | Your own tests | Offline | None; own repository |

### Fit for Goodfetch

- **Offline in CI.** Everything except four checks can run offline. The four exceptions:
  - Zapier's integration checks need the network.
  - `paconn validate` needs a Microsoft login, which is why Apify wrote an offline validator.
  - The n8n scanner needs a published package, so it fits after publishing rather than before.
  - Airbyte's acceptance tests need a source. An engine serving the Proving ground could play the "sandbox account".
- **Certification and accounts.** Certification needs accounts and a reachable production service:
  - a non-expiring account for Zapier staff;
  - a preview environment that Microsoft tests;
  - Make reviewers reading scenario logs.

  ADR 0027 says Zapier rejects localhost and leaves Goodfetch Cloud to a later map. The Zapier rule against "sandbox/test/dev endpoints" may push a public Zapier app toward Goodfetch Cloud as its base URL. Whether a user-supplied URL for an Exposed Daemon is accepted is **unverified**.
- **Verified n8n nodes.** A verified node cannot depend on the TypeScript SDK package ("no external dependencies"). The generated node would carry its own HTTP calls to the HTTP listener, which is consistent with ADR 0027.
- **Two consequential discrepancies with ADR 0027** for the map:
  - LangChain discourages document loaders, but encourages retrievers and tools.
  - LlamaIndex.TS is deprecated and archived, so a TypeScript LlamaIndex loader has no maintained framework to plug into.

## D. Agent skills

### Facts

- **Anthropic's guide** ([skill authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices)):
  - "Build evaluations first": three scenarios, a no-skill baseline, then iterate.
  - Its JSON example has `skills`, `query`, `files` and `expected_behavior`, with the note: "There is not currently a built-in way to run these evaluations. Users can create their own evaluation system."
  - "Test your Skill with all the models you plan to use it with" (Haiku, Sonnet and Opus).
  - Frontmatter rules: `name` at most 64 characters, lowercase letters, numbers and hyphens, with no reserved words "anthropic" or "claude"; `description` at most 1,024 characters and non-empty. The body should stay "under 500 lines".
- **`claude plugin validate`** ([plugin manifest reference](https://code.claude.com/docs/en/plugins-reference)):
  - It is "the authoritative check for a manifest".
  - Results are "Validation passed", "passed with warnings" (`--strict` turns warnings into failures in CI) or "Validation failed".
  - It checks paths, MCP server entries (Claude Code v2.1.281+) and reserved names.
  - The MCP reference servers run it as a pinned, non-skipping CI step. Their script notes: "It needs no authentication — verified against a scrubbed environment and a clean HOME" ([verify-skills-cli.mjs](https://github.com/modelcontextprotocol/servers/blob/main/scripts/verify-skills-cli.mjs)).
- **`claude plugin eval`** ([plugin evals](https://code.claude.com/docs/en/plugin-evals)):
  - Requires Claude Code v2.1.269 or later.
  - Each case runs "three times by default". Each case is also run "with no plugin loaded", giving `WITH`, `W/OUT` and `Δ`.
  - Of six grader types, "`regex`, `tool_used`, `tool_order`, and `file_exists` are computed from the transcript and files and cost nothing, while `llm` and `baseline` call a judge model".
  - Mock MCP servers answer tools from Markdown files ("A run never starts your plugin's real MCP servers unless you ask"); recorded agent-mock answers can be replayed.
  - In CI, use `--json`, `--threshold` and `--trust-plugin`; exit 0 means every case met the threshold, and "The with-minus-without delta is reported but never changes the exit code".
  - "Every eval run and every judge grader is a real model call on your account".
  - Its case format is separate from skill-creator's `evals/evals.json`.
- **The Agent Skills open specification** ([agentskills/agentskills](https://github.com/agentskills/agentskills)):
  - `skills-ref validate path/to/skill` is a reference validator, "intended for demonstration purposes only".
  - [Evaluating skills](https://github.com/agentskills/agentskills/blob/main/docs/skill-creation/evaluating-skills.mdx) defines `evals/evals.json` (prompt, expected output, files) and runs each case "once **with the skill** and once **without it**", each run starting "with a clean context".
- **promptfoo** (0.124.1). The [test-agent-skills guide](https://github.com/promptfoo/promptfoo/blob/main/site/docs/guides/test-agent-skills.md) compares skill versions through the Claude Agent SDK and Codex providers. Its deterministic `skill-used` and `not-skill-used` assertions check invocation, and `--repeat 3` is suggested "for nondeterministic agent behavior". For Codex, "`skill-used` signal is inferred from successful reads of the matching `SKILL.md`".
- **Inspect AI** (0.3.279). The [`skill()` tool](https://github.com/UKGovernmentBEIS/inspect_ai/blob/main/docs/tools-standard.qmd) "copies them into the sample's sandbox". `inspect_swe` (0.2.71) runs Claude Code, Codex and others with `skills` and `mcp_servers` options. A `mockllm` provider exists in Inspect's source (`src/inspect_ai/model/_providers/mockllm.py`).
- **OpenAI.** The [eval-skills post](https://developers.openai.com/blog/eval-skills) uses a custom runner around `codex exec --json` (JSONL events). Its deterministic checks cover commands run, files created and command order, followed by a rubric pass with `--output-schema`. It uses neither `openai/evals` nor the Evals API. The [openai/evals](https://github.com/openai/evals) repository's last commit was 2026-04-14.
- **Skills served over MCP.** The Inspector CLI's `--verify` checks the digests and frontmatter that SEP-2640 requires (see A). The [ext-skills](https://github.com/modelcontextprotocol/ext-skills) repository holds the extension **(SEP-2640 text not read)**.
- **Complaints about triggering.** "[MODEL] Custom skills are not reliably auto-triggered by the model" ([claude-code#30387](https://github.com/anthropics/claude-code/issues/30387), 2026-03-03). "Claude Code assistant not aware of available skills in .claude/skills/ directory" ([#9716](https://github.com/anthropics/claude-code/issues/9716), open, 75 reactions). "`paths:` skills can never activate in auto mode" ([#90004](https://github.com/anthropics/claude-code/issues/90004), open).
- **Prior art.**
  - Browserbase's [skills](https://github.com/browserbase/skills) repository runs `validate-skills.mjs` in CI, "Zero dependencies", checking frontmatter, licence and banned files. 2 of its 18 skills carry `evals/evals.json`.
  - Apify's [agent-skills](https://github.com/apify/agent-skills) CI generates agent files from a template.
  - Firecrawl keeps its skills in the [CLI repository](https://github.com/firecrawl/cli) (`skills/*/SKILL.md`) and syncs them to plugin repositories with a workflow (`sync-plugin.yml`).
  - Crawl4AI ships a skill as a zip in its docs (`docs/md_v2/assets/crawl4ai-skill.zip`).

### Comparison

| Tool | What it checks | Needs a live model? | Offline or needs an account | Certification |
|---|---|---|---|---|
| `claude plugin validate` (`--strict`) | Manifest, paths, MCP entries, reserved names | No | Offline, no authentication | Prerequisite for the Anthropic directory (not checked here) |
| `skills-ref validate`, own validators (Browserbase) | Frontmatter and layout per the open spec | No | Offline | None |
| Inspector `--verify` | Digests and frontmatter of skills served over MCP | No | Offline against a local server | None |
| `claude plugin eval` | Behaviour: regex, tool used and order, files, LLM rubric; with and without the plugin | Yes (the agent run; judges for two grader types) | Anthropic credentials | None |
| skill-creator `evals/evals.json` and the open-spec method | With and without the skill, by hand or agent-run | Yes | Model account | None |
| promptfoo `skill-used` | Invocation and quality across skill versions and agents (Claude, Codex) | Yes | Model account | None |
| Inspect AI `skill()`, `inspect_swe` | Agent tasks in a sandbox with skills | Yes (or `mockllm` for wiring) | Model account | None |
| OpenAI `codex exec --json` recipe | Commands, files, order; rubric | Yes | OpenAI account | None |

### Fit for Goodfetch

- **Per pull request.** Structure can be gated with no model: `claude plugin validate --strict` (pinned, as the reference servers do), the open-spec frontmatter rules, and, if skills are generated from the Operation catalog as ADR 0011 says, a check that the generated skills match the catalog. The installed-versus-bundled drift check in Playwright CLI (section E) is a related pattern.
- **Triggering and quality.** Whether a model picks the skill and finishes the task can only be measured with live calls, repeated runs and a no-skill baseline. That puts these evals outside per-pull-request CI and inside the jobs ADR 0043 lets reach named endpoints.
- **Several agents.** `claude plugin eval` covers Claude only; promptfoo covers both Claude and Codex.

## E. The CLI as coding agents use it

- **Playwright CLI** ([microsoft/playwright-cli](https://github.com/microsoft/playwright-cli)). The CLI's behaviour is tested in the Playwright monorepo (`tests/mcp/cli-*.spec.ts`, including `cli-json.spec.ts` and `cli-help.spec.ts`). The CLI repository ships a skill. `skillCheck.js` compares the skill bundled in `playwright-core` with the copies installed under `.claude/skills/` and `.agents/skills/`, and prints a framed warning when they differ.
- **Firecrawl CLI** (`firecrawl-cli` 1.26.3). Vitest tests cover argument parsing and commands (`src/__tests__/cli-argv.test.ts`, `commands/*.test.ts`). Its skills live beside the code and are synced to its Claude, Codex and Cursor plugin repositories by CI.
- **Apify.** Its MCP evals drive Claude Code itself (see A).
- **OpenAI.** Its recipe drives `codex exec --json` and grades the JSONL trace (see D).
- **`claude plugin eval`.** It runs the agent in Claude Code's OS-level sandbox when Bash is granted: "network access is limited to domains you grant" ([plugin evals](https://code.claude.com/docs/en/plugin-evals)).

### Fit for Goodfetch

- **Deterministic per pull request.** The CLI's `--json` output and Error codes can be tested like any Client. The ADR 0011 R&D finding ("Haiku left 18 of 45 tasks unfinished, without skills") is a model-dependent result, so it belongs to the same live-model eval tier as the skills.
- **Tooling for that tier.** Both `claude plugin eval` (Claude Code) and promptfoo (Claude Agent SDK and Codex) can run the CLI plus skills in a sandbox, with a no-skill baseline.

## F. Prior art at a glance

| Surface | Firecrawl | Apify | Browserbase and Stagehand | Crawl4AI |
|---|---|---|---|---|
| MCP server | `firecrawl-mcp` 3.29.1: stdio and HTTP spawn tests against a fake API, contract snapshots, description budget | Official conformance in CI (two revisions, baselines), per-transport integration tests, Claude Code evals in Langfuse | Self-hosted server archived (had one stdio smoke test plus `mcpvals` evals); hosted server closed source | SSE and WebSocket bridge in the Docker server; MCP scripts not in CI |
| Framework adapters | Own `langchain-firecrawl` package (repository only, no workflows) | `langchain-apify` 0.1.7 on `ToolsIntegrationTests` | One contract in `core/`, per-framework examples over MCP stdio, contract tests | None found |
| n8n | `@mendable/n8n-nodes-firecrawl` 2.2.0, no tests | `n8n-nodes-apify` 0.8.1, jest with nock in CI | `n8n-nodes-browserbase` 1.4.1, lint and build in CI | None found |
| Zapier, Make, Pipedream, Power Automate | Not inspected | Zapier: validate `--without-style`, mocha with nock, live job. Power Automate: own offline validator. Pipedream fork present | Not found | Not found |
| Agent skills | Skills in the CLI repository, synced to plugins | `agent-skills` repository, generated agent files | 18 skills, offline validator, 2 with evals | Skill zip in docs |

## Unverified

- Firecrawl's statement that Claude Code truncates tool descriptions and server instructions at 2,048 characters (a test comment, not checked against Claude Code docs).
- When n8n 3.0 ships. The docs describe `n8n-node dev` behaviour "From n8n 3.0", while npm's latest is 2.43.4.
- Whether the Firecrawl, Apify and Browserbase n8n nodes are *verified* today. ADR 0027's claim was not rechecked: n8n's community-node API was not reachable from this session.
- Whether Zapier publishes a public integration whose base URL each user supplies (an Exposed Daemon) under requirement 5.4.
- Make's "beta" label on local development pages, and the version of the Make Apps Editor extension (taken from a search summary).
- That `pd dev` and `pd publish` require a Pipedream login (inferred from the CLI reference).
- Whether Microsoft plans a successor to `paconn` (last release 2023-02-20); `pac connector` has no validate command.
- When `@modelcontextprotocol/conformance` 0.2.0 becomes stable.
- The content of SEP-2640 (skills over MCP); only the Inspector's description of it was read.
- The publication date of OpenAI's eval-skills post: a search result gave 2026-01-22, and the page itself showed none.
- The date LlamaIndex.TS was archived: the last push was 2026-03-11.
- Absence claims: no certification programme for Vercel AI SDK tool packages or Activepieces pieces, no LangGraph-specific tool suite, and no Mastra doc on mock models.
- The functional tests Microsoft runs during MCP certification, beyond "Functional and safety review".
- Whether Anthropic's plugin directory requires `claude plugin validate` to pass.
