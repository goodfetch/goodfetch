## MCP, framework adapters and Integrations

Decided in [ADR 0070](../adr/0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md).

### Every surface

- **Drive each surface with its platform's own client, against a real engine or Daemon serving the local Proving ground.** No mocking library or hand-written fake of Goodfetch's API: no `nock`, fake HTTP API or stub listener.
  _Why:_ a fake of the engine passes where the real one fails, and misses the token, `Host` and `Origin` checks.
- **For Goodfetch's side, use only the doubles ADRs 0062 and 0069 define:** the in-memory adapter of the protocol in Quick-suite unit tests, and the Scripted engine for faults a real engine can't produce on demand.
- **Stand in for the platform in this order:** its real runtime run locally, its official test harness, a written adapter of its context.
- **Keep a Quick check over everything `xtask codegen` generates:** no person-only operation or setting, and no `page.lend` ([ADR 0089](../adr/0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md)), on any MCP profile, Framework adapter, Integration or skill, and each surface's operations map to catalog operations of the same class. No surface of Collie's carries an entry marked Foxhound (ADR 0095); that Foxhound's carry every entry of Collie's under the same name is the containment check of [Which Product's executable a test runs](levels-and-layout.md#which-products-executable-a-test-runs). The same check lists the public items of `goodfetch-collie-client` and `goodfetch-collie` and refuses any of Foxhound's.
- **Assert outcomes through the oracle and Error codes**, never a model's wording or a platform's message text.

### MCP server

- **Test the server with `rmcp`'s client:** in-memory in its crate, over stdio and Streamable HTTP in the binary's `tests/it`. MCP Journey variants run there too, on nextest.
- **Read stdio stdout with the strict framing reader**, `StrictStdout` ([Shared helpers](shared-helpers.md)), logging at `trace`. Any line that is not a JSON-RPC message fails the test.
- **Test two Streamable HTTP sessions at once:** each keeps its own Limits and its own cancellation.
- **Run the conformance suite at its pinned version, once per supported revision**, with `--expected-failures conformance/mcp/expected-failures/<revision>.yaml`:
  - `--requirements <revision>` for a revision with a frozen requirement set (today `2025-11-25` and `2026-07-28`), `--spec-version <revision>` for an older one;
  - against the shipped binary, whose expected-failures file lists only scenarios that need fixture tools or resources, each with a reason (a Quick check refuses other entries);
  - against the fixture server, with an empty expected-failures file;
  - once more for the extensions the server implements, each scenario by name with `--scenario` and without `--spec-version`, against `conformance/mcp/expected-failures/extensions.yaml`, since `--requirements` doesn't score extensions. These run at `2026-07-28` only; test an extension the server supports under an older revision with `rmcp` in `tests/it`.
- **Change the suite's version only in a pull request that updates the expected-failures files.**
- **Run the pinned Inspector CLI with `--format json --stored-auth-only`:** boot smoke of every packaged artifact over stdio and Streamable HTTP (the OCI image on Linux only), `--strict` on every profile's `tools/list`, and `--verify --require-digests` on skills once the server serves them.
- **Cover every supported revision in the client matrix:** TypeScript SDK v1 and v2 clients, the Python SDK and `rmcp`, pinned, over stdio and Streamable HTTP, plus the negotiation cases.
- **Add a revision to the supported list only with its expected-failures file and its matrix cells, in the same pull request.**

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
- **Produce the refused `hello` and the Error codes the Proving ground can't cause with the Scripted engine run as a Daemon with an HTTP listener**, one script per case; test every other case against the real Daemon.
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

### Journey variants

- **Run `adapter-<name>` and `integration-<name>` variants in the package's own runner, on Linux:** Vitest in `tests/journeys/J-NNN.test.ts` for TypeScript adapters and the n8n, Zapier, Pipedream, Activepieces and Power Automate Integrations; pytest in `tests/journeys/test_jNNN.py` for Python adapters, the LlamaIndex reader and the Airbyte source. Make has no automated variant.
- **Run `http-listener` variants on nextest** in the binary crate's `tests/it/journeys`, against a Daemon's HTTP listener with a token and port 0.

### Live checks and manual tests

- **Run unpinned releases only as Live checks:** the newest conformance suite, Inspector and MCP SDK clients, and each framework's `latest` and prerelease tags, nightly; the n8n scanner after each publish.
- **Keep real platform accounts out of every suite.** They run weekly and at release candidates, on free developer plans only, through a Cloudflare Quick Tunnel (`cloudflared tunnel --url`, a new `trycloudflare.com` URL each run, no account or domain), and only once the owner gives the go-ahead at M4, because the job exposes a Daemon (ADR 0076); a repository variable keeps the job off until then. Paid accounts are declined while there is no revenue.
  - **Re-point the platforms to each run's URL before its cases:** the Power Automate connector's host, and Make's base address wherever the app fixes one, change every run with the Quick Tunnel's address.
  - **The job's ADR 0043 endpoints are the tunnel's** as well as each platform's API host: `api.trycloudflare.com:443`, which creates the Quick Tunnel; the run's `*.trycloudflare.com:443`; and Cloudflare's tunnel hosts `region1.v2.argotunnel.com:7844` and `region2.v2.argotunnel.com:7844`, over TCP and UDP. `cloudflared` runs with `--no-autoupdate`, so it needs no update host. ADR 0070's named Cloudflare Tunnel on a `goodfetch.dev` subdomain is declined (ADR 0076).
- **Test by hand at each release candidate that changes them:** Claude Desktop, ChatGPT, VS Code and Cursor connecting to the MCP server; `paconn validate`; each platform's listing submission.
