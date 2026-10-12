# Code-mode sandboxes in practice: engines, confinement and author contracts

Survey of October 12, 2026, for [issue 125](https://github.com/Jbmanllr/automan/issues/125) (Code-mode JavaScript sandbox engine). The sandbox it decides runs two things:

- **Code mode** (ADR 0011): a short program written by a model. It calls only the Operation catalog, has no network or files, journals its calls in the Effect journal (ADR 0010), and runs under time and memory limits.
- **Script Hooks** (ADR 0033): small JavaScript functions carried in Recipes or settings. They get only `net.fetch` and the `page.*` operations of their stage.

This note records how comparable systems work today. It states facts and options; it does not decide.

**Method.**
- **Primary sources only.** Official docs, vendor engineering posts, source code read on GitHub (`main` unless a commit is linked), GitHub security advisories and NVD, all read on 2026-10-11 and 2026-10-12. Every claim carries the link it came from.
- **Nothing was built or run.** Behaviour is inferred from docs and code.
- **Items that could not be checked are marked unverified** and gathered in section 9.
- **Spot checks.** Four claims were re-checked by hand against the source:
  - OpenAI's programmatic tool calling runtime;
  - the arXiv paper in §4.2;
  - the LaVague CVE (through the NVD API);
  - the rquickjs memory-limit note.

## Summary

| System | Engine | Confinement | How code reaches tools | Author contract | Limits | Journal / replay |
|---|---|---|---|---|---|---|
| **Cloudflare Code Mode** (`@cloudflare/codemode` 0.5.3) | V8 isolate per run, through the Dynamic Worker Loader (workerd) | `globalOutbound: null` makes `fetch`/`connect` throw; no filesystem; on Cloudflare's side, isolates sit inside a process sandbox | `Proxy` namespaces, `await codemode.tool(args)`, over Workers RPC to the host | JS async arrow function. TS syntax is refused in the prompt; types are shown, not compiled. `console` is captured | 60 s wall clock (JS `Promise.race`), plan CPU limit, 128 MB per isolate, result truncated at ~24k chars | **Yes**: `CodemodeRuntime` logs each call by host-assigned position and replays by re-running the code; argument mismatch fails the run |
| **OpenAI programmatic tool calling** (2026-07-09) | Fresh hosted V8 runtime per program | No Node, network, filesystem, subprocess or `console` | `await tools.name(args)`; tools opt in with `allowed_callers` | JS with top-level `await`; output through `text()`/`image()` | Not documented | Opaque `fingerprint` to resume or replay a paused program |
| **Anthropic code execution + programmatic tool calling** | Linux container (Python 3.11, Bash) | Internet fully off; workspace files only | Async Python functions; the API pauses with a `caller` | Python, top-level `await` | 1 CPU, 5 GiB RAM, 90 s per cell, ~4 min per pending tool result | No program replay (container state persists instead) |
| **Pydantic Monty / Pydantic AI CodeMode** | Rust interpreter of a Python subset | Language-level: no ambient filesystem, environment or network | Host functions; execution suspends at each call | Python subset; optional `ty` type check | Memory, execution time (clock paused during host calls), 1000 suspensions, 100 tool calls | Snapshots: suspended state serialises to bytes and resumes elsewhere |
| **goose / pctx** | `deno_core` 0.412 (V8) embedded in Rust; goose first used Boa | No filesystem or env; network only to configured hosts | One TS namespace per MCP server | **TypeScript type-checked** (typescript-go) before running | 10 s | No |
| **UTCP code-mode** | `isolated-vm` (V8) | Separate isolate | Generated TS interfaces; bridged `callTool` | Async function body | 128 MB, 30 s | No |
| **Mastra code mode** | Host `node`, E2B, `isolated-vm` or QuickJS | Depends on the transport chosen | `external_*` functions run on the host | JS async function | `memoryLimitMb` | No |
| **@sebastianwessel/quickjs**, komode | QuickJS compiled to Wasm (quickjs-emscripten) | In-process Wasm; fetch and a virtual filesystem are opt-in | Host functions | JS, or TS transpiled with `typescript` | Timeout, memory, stack (komode: 120 s, 64 MB, 25 calls) | No |
| **smolagents** `LocalPythonExecutor` | AST interpreter in CPython | Import allowlist, **escaped by several CVEs**; remote executors are E2B, Docker, Modal, Blaxel | Python function stubs | Python | 10M operations, 30 s, 50k chars of output | No |
| **Shopify Functions** | QuickJS in Wasm (Javy) | Wasm; no I/O, clock or randomness; the host performs any fetch | GraphQL input in, list of operations out | `export function run(input)`, ES2020, no event loop | 11M instructions, 10 MB memory, 20 kB output | n/a (pure function) |
| **Figma plugins** | QuickJS in Wasm, on the main thread | No network in the sandbox; the UI iframe follows the manifest CSP | `figma.*` API | ES2020+, minimal `console` | None published ("no way to interrupt") | n/a |
| **Playwright MCP** `browser_run_code_unsafe` | Node `vm` context in the MCP server process | None: the tool calls itself RCE-equivalent | `page` object | `async (page) => {…}` | Not documented | No |
| **Stagehand v4** facade `run` | Chrome extension service worker | Facade object only; documented as trusted code | Playwright-shaped `page`/`context` | JS callback | 60 s / 75 s deadlines | No |

**What bears on issue 125:**

1. **The two hosted code modes closest to ADR 0011 both chose V8 with nothing ambient.**
   - Cloudflare uses one isolate per run with outbound network set to `null`.
   - OpenAI uses one fresh V8 runtime per program, with no `console`, network or filesystem.
   - Each calls tools through a single host-provided object (`codemode.*`, `tools.*`).
   - Goose's code mode, the one Rust precedent, moved from Boa to `deno_core` (§1.6). The reason was not published.
2. **Two systems already replay a program against a journal of tool calls.** Cloudflare's `CodemodeRuntime` and Pydantic's Monty both key calls by position and resume by re-running (or restoring) the program. Cloudflare's runtime refuses a call whose method or arguments differ from the journal entry at that position. This matches the rule the code-mode lab report adopted.
3. **Most hosted systems refuse TypeScript syntax in the program.** Cloudflare's prompt forbids it; OpenAI and Anthropic accept JavaScript or Python only. Declarations are written in TypeScript (Cloudflare) or as Python stubs (Anthropic, Pydantic, smolagents) because they are a compact way to describe tools. The exceptions are pctx, which type-checks real TS before running, and the QuickJS libraries, which transpile it.
4. **No source isolates "TypeScript declarations versus JSON Schema" as the cause of better results.** The measured gains come with fewer model round trips and with filtering data inside the program (§4.2).
5. **In-process sandboxes built on a general-purpose runtime keep failing.**
   - vm2 has about 60 new advisories in 2026.
   - n8n's `node:vm` runner has been escaped more than once in 2026.
   - So has smolagents' AST interpreter.
   - Figma's Realms shim failed in 2019.

   **Interpreters built for confinement have advisories too.**
   - QuickJS-NG had about ten in May–July 2026, including a critical RCE.
   - Vendors running fully untrusted code add a hard boundary (microVM, Wasm, or isolate plus OS sandbox plus fast V8 patching).
6. **Engine-level limits differ:**
   - **V8:** termination callable from another thread, heap limit with a near-limit callback; the process aborts if the callback does not intervene.
   - **QuickJS:** memory limit, an interrupt polled every 10,000 operations, and a stack limit.
   - **Boa:** only loop, recursion and stack limits. It has no interrupt and no memory limit.
   - **All three:** wall time while awaiting a host call must be enforced by the host.
7. **No scraping competitor runs a hook string in a sandbox inside the engine.** Hooks carried as strings run either in the target page (Firecrawl, Zyte, ScrapingBee, Spider Cloud, Browserless v2) or through an unsandboxed `eval` in Node (Apify). Code hooks (Crawlee, Scrapy, spider-rs) are trusted in-process code. Browserless left vm2 by moving function execution into the browser.

## 1. Agent code-execution and code-mode systems

### 1.1 Cloudflare Code Mode

**Package and engine**
- `@cloudflare/codemode` 0.5.3 (2026-10-02), MIT, marked experimental. [npm](https://registry.npmjs.org/@cloudflare/codemode), [source](https://github.com/cloudflare/agents/tree/main/packages/codemode)
- `DynamicWorkerExecutor` calls `env.LOADER.load({ compatibilityDate, compatibilityFlags: ["nodejs_compat"], mainModule, modules, globalOutbound, env })`. Every run gets a new isolate (`load()`, not `get(id)`). [executor.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/executor.ts)

**Network**
- `globalOutbound` defaults to `null`, so `fetch()` and `connect()` throw.
- A `Fetcher` routes traffic through a gateway; `undefined` inherits full internet access.
- Sources: [executor.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/executor.ts), [egress control](https://developers.cloudflare.com/dynamic-workers/usage/egress-control/)

**Tool exposure**
- Each namespace (`codemode`, `state`, …) is a `Proxy` global.
- A call JSON-encodes its arguments and goes to `ToolDispatcher.call(name, argsJson)`, an `RpcTarget` on the host.
- Errors return as `{error}` and are re-thrown inside the sandbox.
- Names are sanitised (`my-tool` → `my_tool`); inputs are validated on the host.
- Sources: [executor.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/executor.ts), [README](https://github.com/cloudflare/agents/blob/main/packages/codemode/README.md)

**Executors**
- `Executor.execute(code, providers, {connectors}) → {result, error?, logs?}` must never throw.
- Two implementations ship: the Worker Loader one and an iframe one, which cannot pre-empt a `while(true)`.
- The README only sketches a Node or QuickJS executor ("build your own").
- Source: [executor-types.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/executor-types.ts)

**Author contract**
- The default tool description tells the model to write plain JavaScript, with no type annotations, interfaces or generics. [shared.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/shared.ts)
- `normalizeCode` parses the code with acorn (`sourceType: "module"`) and strips Markdown fences.
- It unwraps `export default`, turns a trailing expression into `return`, and otherwise wraps the code in `async () => {…}`.
- It never strips or checks types. [normalize.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/normalize.ts)

**Globals, results and logs**
- Globals: the full workerd runtime with `nodejs_compat`. `console.log/warn/error` are overridden into a `__logs` array. [executor.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/executor.ts)
- Results: `{result, logs}`. On error the tool throws `Code execution failed: …` with the logs appended. [run-code.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/run-code.ts)
- The 2025 post had results returned only through `console.log`. [blog](https://blog.cloudflare.com/code-mode/)

**Declarations**
- `generateTypesFromJsonSchema` turns JSON Schema or Zod into `type XInput`/`XOutput` aliases plus a `declare const codemode: {…}` with JSDoc per field, inserted into the tool description. [json-schema-types.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/json-schema-types.ts)
- Large APIs use discovery instead: `codemode.search()` and `codemode.describe("conn.method")`, which return declarations on demand. [proxy-tool.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/proxy-tool.ts)

**Limits**
- The executor's `timeout` defaults to 60,000 ms, as a wall-clock `Promise.race` inside the isolate.
- The executor passes no `limits` to the loader. [executor.ts at 48d9c36](https://github.com/cloudflare/agents/blob/48d9c36f5f1ff18a91fbf0659b9a8e026d1cac43/packages/codemode/src/executor.ts)
- Dynamic Workers accept `limits: {cpuMs, subRequests}` and throw immediately on overrun. [Dynamic Workers limits](https://developers.cloudflare.com/dynamic-workers/usage/limits/)
- Workers allow 128 MB per isolate; paid CPU time is 30 s by default, up to 5 min. [Workers limits](https://developers.cloudflare.com/workers/platform/limits/)
- Results are truncated structurally at about 6,000 tokens (4 chars per token). [truncate.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/truncate.ts)

**Durable runtime**
- `CodemodeRuntime` is a Durable Object facet. It logs every connector call and every `codemode.step(name, fn)` under a host-assigned `seq` in SQLite.
- A paused run (for example, one awaiting approval) re-runs the same code, and logged calls return their recorded result.
- Details are in §6. Sources: [runtime.ts](https://github.com/cloudflare/agents/blob/main/packages/codemode/src/runtime.ts), [runtime.md](https://github.com/cloudflare/agents/blob/48d9c36f5f1ff18a91fbf0659b9a8e026d1cac43/docs/codemode/runtime.md)

**Posts**
- **2025-09-26, "Code Mode: the better way to use MCP":** claims models are better at writing code that calls MCP than at calling MCP directly. No benchmark. [blog](https://blog.cloudflare.com/code-mode/)
- **2026-02-20:** the Cloudflare API, 2,500+ endpoints, exposed through `search()` and `execute()` in about 1,000 tokens instead of about 1.17M. [blog](https://blog.cloudflare.com/code-mode-mcp/)
- **2026-03-24:** Dynamic Workers enter open beta. Isolates start in a few milliseconds and use a few MB. [blog](https://blog.cloudflare.com/dynamic-workers/), [changelog](https://developers.cloudflare.com/changelog/post/2026-03-24-dynamic-workers-open-beta/)

**Platform security model**
- V8 isolates inside a process sandbox (namespaces plus seccomp, with no direct filesystem or network).
- V8 patches rolled out in under 24 hours.
- `Date.now()` frozen during execution; no shared memory. [2020 post](https://blog.cloudflare.com/mitigating-spectre-and-other-security-threats-the-cloudflare-workers-security-model/)
- 2025 additions: the V8 sandbox and memory protection keys per isolate. [2025 post](https://blog.cloudflare.com/safe-in-the-sandbox-security-hardening-for-cloudflare-workers/)

### 1.2 OpenAI

**Programmatic tool calling** (launched with GPT-5.6, 2026-07-09). Sources: [guide](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling.md), [changelog](https://developers.openai.com/api/docs/changelog.md)
- **Runtime:** each program runs in a fresh, isolated V8 runtime, in JavaScript with top-level `await`. There is no Node.js, package installation, network, filesystem, subprocess, `console`, or state between programs.
- **Calls and output:** tools are called as `await tools.name(args)`, and `Promise.all` is allowed. Output goes through `text(...)` and `image(...)`.
- **Opt-in:** a tool is callable from code only if its `allowed_callers` includes `"programmatic"`. Function, custom, MCP, shell, apply_patch and code_interpreter tools qualify.
- **Pause and resume:** a client function call pauses the program. The `program` item carries an opaque `fingerprint` for resume or replay; with `store: false` the client must replay every output item in order.
- **Documentation gaps:** timeouts, memory and call caps are not documented.
- **OpenAI's routing advice:** use direct calls for single lookups, writes, approvals and citations, and measure against a direct-calling baseline.

**Code Interpreter** is a separate, Python-only container.
- Memory tiers are 1, 4, 16 and 64 GB, and a container expires after 20 minutes unused. [guide](https://developers.openai.com/api/docs/guides/tools-code-interpreter)
- Containers take `network_policy: disabled | allowlist`. [containers API](https://developers.openai.com/api/reference/resources/containers/methods/create.md)
- **Agents SDK:** `CodeInterpreterTool` and `ProgrammaticToolCallingTool` are hosted. Local sandbox agents run in Docker, or in E2B, Modal and others. The local Unix client adds no OS confinement on Linux. [tools](https://openai.github.io/openai-agents-python/tools/), [sandbox guide](https://openai.github.io/openai-agents-python/sandbox/guide/)

### 1.3 Anthropic

**Code execution tool.** Source: [code execution tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/code-execution-tool)
- Versions are `code_execution_20250825`, `_20260120` (adds REPL state and programmatic calling) and `_20260521`.
- Container: Linux, Python 3.11, 1 CPU, 5 GiB RAM, 5 GiB disk. Internet is disabled.
- Errors include `execution_time_exceeded` and `output_file_too_large`. Containers expire 30 days after creation.

**Programmatic tool calling.** Source: [programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling)
- A tool opts in with `allowed_callers: ["code_execution_20260120"]`.
- Tools become async Python functions that take a dict and return a string.
- The API pauses with `stop_reason: "tool_use"` and a `caller` naming the run; intermediate results stay out of the model's context.
- Limits: 90 s wall clock per cell, and about 4 min per pending tool result, after which a `TimeoutError` is raised in the code.
- Not supported: `strict` tools, MCP connector tools.

**Engineering posts**
- "Code execution with MCP" (2025-11-04):
  - presents MCP tools as TypeScript files (`./servers/<server>/<tool>.ts`), discovered progressively;
  - reports one workflow going from 150,000 to 2,000 tokens;
  - names sandboxing, resource limits and monitoring as the cost.
  - [post](https://www.anthropic.com/engineering/code-execution-with-mcp)
- "Advanced tool use" (2025-11-24): programmatic calling cut tokens 37% (43,588 → 27,297) and raised two internal evaluations by 2.9 and 4.7 points. [post](https://www.anthropic.com/engineering/advanced-tool-use)

### 1.4 Pydantic

**mcp-run-python** (Pyodide inside Deno) is **archived**.
- Its notice says there is no way to run Python safely in Pyodide with reasonable latency: Python can reach JavaScript, files the runtime can reach, and memory.
- It ran Deno with `--allow-net` by default. [repo](https://github.com/pydantic/mcp-run-python), [main.py](https://github.com/pydantic/mcp-run-python/blob/main/mcp_run_python/main.py)

**Monty** (MIT, v1.1.0 on 2026-10-05): a Python-subset interpreter written in Rust, published on crates.io, PyPI and npm. [repo](https://github.com/pydantic/monty)
- **Confinement** is language-level: with no host functions or mounts configured, there is no filesystem, environment or network. [security.md](https://github.com/pydantic/monty/blob/main/docs/security.md)
- **Tool calls:** host functions suspend execution while the host runs them.
- **Limits:** `max_memory`; execution-time budgets that **pause during host calls**; recursion depth 1000; `max_suspensions` 1000 (cannot be disabled); a host-side `request_timeout` that kills the worker; print output capped at 10 MiB. [resource-limits.md](https://github.com/pydantic/monty/blob/main/docs/resource-limits.md)
- **Snapshots:** suspended state serialises to bytes and resumes in another process. [snapshots.md](https://github.com/pydantic/monty/blob/main/docs/snapshots.md)
- **Type checking:** optional with `ty`. [type-checking.md](https://github.com/pydantic/monty/blob/main/docs/type-checking.md)

**Pydantic AI `CodeMode`** puts a single `run_code` tool on Monty. [docs](https://pydantic.dev/docs/ai/harness/code-mode/)
- Python stubs are generated from each tool's JSON Schema.
- The final expression is returned.
- Defaults: 30 s, 256 MiB, 100 nested tool calls per run.

### 1.5 Hugging Face smolagents

- `CodeAgent` takes `executor_type` `local`, `e2b`, `docker`, `modal` or `blaxel`. [agents.py](https://github.com/huggingface/smolagents/blob/main/src/smolagents/agents.py)
- The Pyodide+Deno Wasm executor was removed on 2026-05-29. [PR 2321](https://github.com/huggingface/smolagents/pull/2321)
- `LocalPythonExecutor` interprets the AST operation by operation. [local_python_executor.py](https://github.com/huggingface/smolagents/blob/main/src/smolagents/local_python_executor.py)
  - It applies an import allowlist plus lists of dangerous modules and functions.
  - Caps: `MAX_OPERATIONS` 10M, `MAX_WHILE_ITERATIONS` 1M, 30 s, and 50,000 chars of output (head and tail kept).
- smolagents' own documentation says no local Python sandbox can ever be fully secure. [secure code execution](https://huggingface.co/docs/smolagents/main/en/tutorials/secure_code_execution)
- Escapes are listed in §8.

### 1.6 JavaScript code mode embedded elsewhere

**goose (Block)**
- Shipped code mode in v1.17.0 (2025-12-15) on **Boa**, calling back into Rust and then MCP. [post](https://goose-docs.ai/blog/2025/12/15/code-mode-mcp)
- Current docs use **pctx**, with `list_functions`, `get_function_details` and `execute_typescript`. [docs](https://goose-docs.ai/docs/guides/managing-tools/code-mode)

**pctx** (Rust, MIT) embeds `deno_core` 0.412 and `deno_ast`. [Cargo.toml](https://github.com/portofcontext/pctx/blob/main/Cargo.toml), [code-mode.md](https://github.com/portofcontext/pctx/blob/main/docs/code-mode.md)
- **Shape:** each MCP server becomes a TypeScript namespace (`await gdrive.getSheet({...})`).
- **Type checking:** code is checked with typescript-go before running.
- **Limits:** 10 s timeout; no filesystem or environment; network only to configured hosts.
- **Result:** `{success, stdout, output, diagnostics}`.

**UTCP code-mode** runs on `isolated-vm` with 128 MB and 30 s. [client source](https://github.com/universal-tool-calling-protocol/code-mode/blob/main/typescript-library/src/code_mode_utcp_client.ts)
- Code runs as an async function body, with `console` captured.
- The source notes that a hung host call can pin a native thread.

**Mastra code mode** runs JS async functions that call `external_*` tools on the host. [docs](https://mastra.ai/docs/agents/code-mode.md)
- Transports: local `node`, E2B, `@mastra/isolated-vm` or `@mastra/quickjs`.

**QuickJS-in-Wasm libraries**
- @sebastianwessel/quickjs 3.1.0 has `executionTimeout`, `memoryLimit`, `maxStackSize`, opt-in `allowFetch` and `allowFs` (memfs), and `transformTypescript`. [options](https://github.com/sebastianwessel/quickjs/blob/main/src/types/SandboxOptions.ts)
- komode: 120 s, 64 MB, 25 tool calls, 50,000-char result. [repo](https://github.com/0xmmo/komode)
- quickjs-emscripten 0.32.0 provides `setMemoryLimit`, `setMaxStackSize` and `setInterruptHandler`. Its Asyncify build suspends on only one async host call at a time. [repo](https://github.com/justjake/quickjs-emscripten)

**isolated-vm** itself is in maintenance mode.
- Its `memoryLimit` is a guideline that an attacker can exceed 2–3×.
- Leaking a `Reference` gives the guest full control. [repo](https://github.com/laverdet/isolated-vm)

## 2. Platforms that run untrusted code

| System | Engine | Isolation | Network / files | Author contract | Limits |
|---|---|---|---|---|---|
| Deno CLI permissions | V8 | Runtime permission checks | Off unless `--allow-*` | JS/TS modules, top-level `await`, web APIs | None by default |
| Deno Sandbox (beta 2026-02-03) | Linux | Firecracker microVM | `allowNet` egress proxy; secret placeholders swapped in only for approved hosts | Any process | 2 vCPU, 768 MB–4 GB, 10 GB disk, 30 min |
| Vercel Sandbox (GA 2026-01-30) | Ubuntu, Node LTS, Python 3.14 | Firecracker microVM | Open by default; deny-all or domain/CIDR policy | Shell or SDK | 5 min default, up to 24 h; 2 GB per vCPU |
| E2B | Linux; Jupyter kernels (ijavascript on Node 20) | Firecracker microVM, nftables | Internet on by default; allow/deny lists | `runCode(code, {language: 'js'\|'ts'})`: TS, top-level `await`, ESM | 1 h (Hobby) / 24 h (Pro) |
| Val Town | Deno 2.9.6 | Pool of Deno subprocesses | Network on, no filesystem | TS/TSX modules | 1 or 10 min wall clock, 4 GiB, 5,000 log lines |
| Shopify Functions | QuickJS in Wasm (Javy, which depends on rquickjs 0.14) | Wasm | None; the `fetch` target only describes a request | `export function run(input)`; no event loop, timers, `fetch`, `crypto` or `URL`; `TextEncoder` only | 11M instructions, 10 MB memory, 256 kB binary, 128 kB in / 20 kB out |
| Figma plugins | QuickJS in Wasm | Wasm VM on the main thread | None in the sandbox; UI iframe under CSP | ES2020+, minimal `console` | Not published |
| n8n Code node | Node | Separate runner process plus `node:vm` | No filesystem or HTTP | Items in, items out | 300 s, 1 GiB payload |
| Zapier Code | Node 22 / Python 3.13 | AWS Lambda | `fetch` allowed | `inputData` in, `output` out | 1 s–2 min by plan, 512 MB, 6 MB I/O |
| Windmill | Bun / Deno | Optional nsjail or `unshare` (off by default) | nsjail can cut the network | TS `main()` | Configurable |
| Apify Actors | Node / Chromium | Docker container per run | Full network | `pageFunction(context)` returns JSON | 128 MB–32 GB |

Sources:
- **Deno:** [security](https://docs.deno.com/runtime/fundamentals/security/), [Sandbox post](https://deno.com/blog/introducing-deno-sandbox), [Sandbox docs](https://docs.deno.com/sandbox/)
- **Vercel:** [concepts](https://vercel.com/docs/sandbox/concepts), [firewall](https://vercel.com/docs/sandbox/concepts/firewall), [pricing](https://vercel.com/docs/sandbox/pricing)
- **E2B:** [security](https://docs.e2b.dev/faq/security-and-compliance.md), [JS](https://docs.e2b.dev/code-interpreting/supported-languages/javascript.md), [infra](https://github.com/e2b-dev/infra)
- **Val Town:** [runtime](https://docs.val.town/reference/runtime/), [limits](https://www.val.town/limits)
- **Shopify / Javy:** [JS for Functions](https://shopify.dev/docs/apps/build/functions/programming-languages/javascript-for-functions), [limits](https://shopify.dev/docs/api/functions/latest), [network access](https://shopify.dev/docs/apps/build/functions/network-access), [Javy](https://github.com/bytecodealliance/javy)
- **Figma:** [how plugins run](https://developers.figma.com/docs/plugins/how-plugins-run/)
- **n8n:** [task runners](https://docs.n8n.io/hosting/configuration/task-runners/), [runner source](https://github.com/n8n-io/n8n/blob/master/packages/@n8n/task-runner/src/js-task-runner/js-task-runner.ts)
- **Zapier:** [Code by Zapier](https://help.zapier.com/hc/en-us/articles/45405528551181-Using-Code-by-Zapier)
- **Windmill:** [isolation](https://www.windmill.dev/docs/advanced/security_isolation)
- **Apify:** [resources](https://docs.apify.com/platform/actors/running/usage-and-resources), [Web Scraper](https://apify.com/apify/web-scraper)

**History worth keeping**
- **Val Town** went through four runtimes: Node `vm` (escapable), then vm2, then Deno Workers (escaped through a nested worker), then Deno subprocesses. [post](https://blog.val.town/first-four-val-town-runtimes)
- **Figma**
  - Figma shipped the Realms shim in 2019. [2019 post](https://www.figma.com/blog/how-we-built-the-figma-plugin-system/)
  - Escapes were reported, and Figma moved to QuickJS compiled to Wasm, keeping it even after Realms was patched. [update](https://www.figma.com/blog/an-update-on-plugin-security/)
- **n8n** made task runners the default in 2.0 (2025-12-08). The JS runner is still `node:vm` in a separate process. [2.0 changes](https://docs.n8n.io/2-0-breaking-changes/)
- **Deno's own advice for untrusted code** is to add OS sandboxing or a VM (gVisor, Firecracker) on top of permissions, and to treat `--allow-run` and `--allow-ffi` as allowing everything. [security](https://docs.deno.com/runtime/fundamentals/security/)

## 3. Browser-automation peers

| Product | Model writes a program over the tools? | Engine | Confinement | Gating |
|---|---|---|---|---|
| Playwright MCP | Yes: `browser_run_code_unsafe` (renamed from `browser_run_code` on 2026-04-29) | Node `vm` context in the MCP server, with `fetch`, `Buffer`, `crypto` and timers exposed since 2026-09-28 | None; the README says the server is not a security boundary | On by default (`core`); PRs to hide or block it were closed |
| Playwright MCP `browser_evaluate` | No: page JS only | Page via `page.evaluate` | Browser page | On by default |
| Stagehand v4 | Yes: the facade's `run` tool (experimental) | Extension service worker via `experimentalBatch` | Facade object only; documented as trusted code; 60 s / 75 s | Experimental API |
| Browser Use | Code-Use (Python `exec`) removed 2026-03-21; the CLI and Harness still `exec` piped Python | Local CPython | None | `evaluate` (page JS) on by default |
| agent-browser | No: `eval` runs page JS; `batch` is a command list | Page via CDP | Browser page | `--action-policy`, `--confirm-actions eval` |
| Skyvern | Yes: Code block, copilot-written blocks, generated Python replay scripts | In-process CPython `exec` / `importlib` | Restricted builtins plus AST denylist; the source calls it a guardrail, not a sandbox; 300 s | `CODE_BLOCK_MODE`, enabled by default |
| Kernel | Yes: `browser_repl`, `execute_playwright_code` | Node.js process in the browser VM | The VM is the boundary; documented as unrestricted; 150 / 300 s | API / MCP |

Sources:
- **Playwright:** [runCode.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/tools/backend/runCode.ts), [README](https://github.com/microsoft/playwright-mcp/blob/main/README.md), [PR 42975](https://github.com/microsoft/playwright/pull/42975), [PR 42691](https://github.com/microsoft/playwright/pull/42691), [evaluate.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/tools/backend/evaluate.ts)
- **Stagehand:** [stagehand.mdx](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/reference/stagehand.mdx), [callbackBatch.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/callbackBatch.ts), [facade contract](https://github.com/browserbase/stagehand/blob/main/packages/integrations/core/src/facade/contract.ts)
- **Browser Use:** [removal commit](https://github.com/browser-use/browser-use/commit/43b5e4ce1d813db6641c82783a40f7b91fe9dde2), [tools](https://github.com/browser-use/browser-use/blob/main/browser_use/tools/service.py), [harness run.py](https://github.com/browser-use/browser-harness/blob/main/src/browser_harness/run.py)
- **agent-browser:** [security](https://github.com/vercel-labs/agent-browser/blob/main/docs/content/docs/security.mdx)
- **Skyvern:** [block.py](https://github.com/Skyvern-AI/skyvern/blob/main/skyvern/forge/sdk/workflow/models/block.py), [code_block_security.py](https://github.com/Skyvern-AI/skyvern/blob/main/skyvern/forge/sdk/copilot/code_block_security.py), [config.py](https://github.com/Skyvern-AI/skyvern/blob/main/skyvern/config.py)
- **Kernel:** [code mode](https://kernel.sh/docs/browsers/code-mode-webmcp.md), [Playwright execution](https://kernel.sh/docs/browsers/playwright-execution.md)

None of these peers runs model code in a confined engine with only an allowlisted API, journaled calls and a memory cap. Where code mode exists, the boundary is one of:
- the whole browser VM (Kernel);
- a service worker (Stagehand);
- nothing (Playwright MCP, Browser Use, Skyvern).

## 4. Author contract

### 4.1 What each system offers the author

| System | Language accepted | Shape | Top-level `await` | Globals | Tool call | Results, logs, errors back to the model |
|---|---|---|---|---|---|---|
| Cloudflare Code Mode | JS; the prompt forbids TS syntax | Async arrow function, normalised by acorn | Inside the function | workerd plus `nodejs_compat`; `console` captured | `await codemode.x(args)` | `{result, logs}`; error message with logs; truncated to ~24k chars |
| OpenAI programmatic calling | JS | Program | Yes | No `console`; `text()`, `image()` | `await tools.x(args)` | `program_output` (`completed` / `incomplete`) |
| Anthropic programmatic calling | Python | REPL cell | Yes | Python standard library | `await x({...})` returns a string | stdout, stderr, return code |
| Pydantic CodeMode | Python subset (optional type check) | Snippet; last expression returned | Yes (`asyncio.gather`) | Restricted subset | Host function | `{output}` plus value |
| pctx (goose) | **TypeScript, type-checked** | Program | Yes | Deno subset without filesystem or env | `await ns.x(args)` | `{success, stdout, output, diagnostics}` |
| UTCP code-mode | JS (no transpile found) | Async function body | Yes | `console` captured | Bridged `callTool` | Captured logs plus value |
| @sebastianwessel/quickjs | JS or TS (transpiled) | Module | Yes | Opt-in `fetch` and filesystem | Host functions | `{ok, data}` |
| E2B code interpreter | JS or TS | Jupyter cell | Yes | Node 20 | n/a | `{results, logs, error}` |
| Shopify Functions | JS, built with esbuild then Javy | `export function run(input)` | No: async throws at run time | `TextEncoder`/`TextDecoder` only | None (declarative output) | Output JSON; 1 kB of logs |
| Figma plugins | JS (ES2020+) | Plugin script | n/a | Minimal `console`; no `fetch` or timers in the sandbox | `figma.*` | n/a |

The prompt asked about `URL`, `TextEncoder`, `structuredClone`, `crypto.randomUUID`, `Temporal` and `Intl`. Among the code modes, only Shopify documents a globals list (`TextEncoder`/`TextDecoder` and nothing else). OpenAI documents only what is absent. Cloudflare inherits workerd's full set, which was not enumerated here (section 9).

### 4.2 Do models do better against TypeScript declarations than with JSON tool calls?

**Claims**
- **Cloudflare (2025-09)** asserts that models write code against an API better than they call MCP tools, arguing that code is better represented in training data. It gives no measurement. [blog](https://blog.cloudflare.com/code-mode/)
- **Cloudflare (2026)** gives token figures only: about 1,000 instead of about 1.17M tokens for 2,500 endpoints, and an "81%" cut. [code-mode-mcp](https://blog.cloudflare.com/code-mode-mcp/), [dynamic-workers](https://blog.cloudflare.com/dynamic-workers/)
- **Anthropic:** 98.7% fewer tokens in one example workflow; −37% tokens and +2.9 / +4.7 points on two internal evaluations; +11% on BrowseComp/DeepSearchQA with 24% fewer input tokens. Each mixes code with fewer round trips and filtering inside the program. Anthropic's own feature exposes Python stubs, not TypeScript. [code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp), [advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use), [programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling)

**Measurements**
- **CodeAct (Wang et al., ICML 2024):** up to 20% higher success rate with code actions than with JSON or text actions, across 17 LLMs. [arXiv 2402.01030](https://arxiv.org/abs/2402.01030)
- **"The Bitter Lesson of Tool Calling"** (Patel et al., 2026-08-06, BFCL v4, 14 models, typed Python stubs). [arXiv 2608.06370](https://arxiv.org/abs/2608.06370)
  - Programmatic calling matches or beats JSON calling in 11 of 14 models, and in 13 of 14 under parallel fan-out.
  - GPT-5.6 improves by 10.6%.
  - JSON calling degrades 2.3% under context flooding.
- **Goodfetch's own lab:** code mode with a journal needed 3 to 15 times fewer tokens than function tools, at the same correctness. [code-mode report](../lab/reports/code-mode-journal-2026-10-09.md)

**Counter-evidence**
- OpenAI recommends direct calls for single, adaptive, write or approval steps. [guide](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling.md)
- Cloudflare's own package now shows TypeScript declarations but asks for plain JavaScript.

**Gap:** no source found compares TypeScript declarations against JSON Schema while holding the execution model constant.

## 5. Resource limits

### 5.1 What systems apply

| System | CPU | Wall time | Memory | Output cap | Tool-call cap | Enforced by |
|---|---|---|---|---|---|---|
| Cloudflare Workers (paid) | 30 s default, up to 5 min; I/O wait excluded | None for HTTP; 15 min for cron, alarms and queues | 128 MB per isolate | — | 10k subrequests (up to 10M) | CPU counted between `enterJs` and `exitJs`; error 1102 |
| Dynamic Workers | `limits.cpuMs` | — | Not stated | — | `limits.subRequests` | Immediate exception |
| Cloudflare Code Mode | Plan limit | 60 s, includes tool waits | Isolate | ~24k chars; 1 MB per durable value | None | JS timer |
| Anthropic programmatic calling | 1 CPU | 90 s per cell; ~4 min per pending tool | 5 GiB | `output_file_too_large` (size not stated) | None stated | Container |
| OpenAI programmatic calling | — | — | — | — | — | Not documented |
| Pydantic Monty | Execution time, clock paused during host calls | Host `request_timeout` | `max_memory` (256 MiB in CodeMode) | 10 MiB of print | 1000 suspensions, 100 tool calls | Interpreter |
| smolagents local | 10M operations | 30 s | — | 50k chars | — | AST counters plus timeout |
| Shopify Functions | 11M Wasm instructions | — | 10 MB plus 512 kB stack | 20 kB | n/a | Wasm metering |
| Val Town | — | 1 / 10 min | 4 GiB | 5,000 log lines × 8,192 chars | — | Platform |
| Vercel Sandbox | Active CPU billed | 5 min default, up to 24 h | 2 GB per vCPU | — | — | microVM |
| Temporal TS (for comparison) | — | 5 s per activation (`isolateExecutionTimeoutMs`), mapped to deadlock error TMPRL1101 | — | — | — | `vm` `runInContext` timeout |

Sources:
- **Cloudflare:** [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [limit-enforcer.h](https://github.com/cloudflare/workerd/blob/main/src/workerd/io/limit-enforcer.h), [Dynamic Workers limits](https://developers.cloudflare.com/dynamic-workers/usage/limits/), [codemode API](https://developers.cloudflare.com/agents/api-reference/codemode/)
- **Monty:** [resource-limits.md](https://github.com/pydantic/monty/blob/main/docs/resource-limits.md)
- **smolagents:** [utils.py](https://github.com/huggingface/smolagents/blob/main/src/smolagents/utils.py)
- **Shopify:** [errors](https://shopify.dev/docs/apps/build/functions/monitoring-and-errors)
- **Temporal:** [vm.ts](https://github.com/temporalio/sdk-typescript/blob/main/packages/worker/src/workflow/vm.ts), [worker-options.ts](https://github.com/temporalio/sdk-typescript/blob/main/packages/worker/src/workflow/worker-options.ts)

**No system documents a cap on the number of tool calls per program**, except Pydantic (100 per run, 1000 suspensions) and komode (25). Cloudflare and Vercel exclude I/O wait from CPU time. Monty pauses its execution clock during host calls. Cloudflare's 60 s and Anthropic's 90 s limits are wall clock and include tool time.

### 5.2 How the candidate engines enforce limits

**V8 through rusty_v8 / deno_core**
- `TerminateExecution()` can be called from any thread, without the V8 lock. rusty_v8 exposes it as `IsolateHandle::terminate_execution`, alongside `request_interrupt`. [v8-isolate.h](https://github.com/v8/v8/blob/main/include/v8-isolate.h), [isolate.rs](https://github.com/denoland/rusty_v8/blob/main/src/isolate.rs)
- Heap limit: `CreateParams::heap_limits`. Near the limit V8 runs GCs and calls the near-heap-limit callback. If the limit is not raised, V8 aborts the whole process. [isolate_create_params.rs](https://github.com/denoland/rusty_v8/blob/main/src/isolate_create_params.rs)
- Deno's pattern: in the callback, set an out-of-memory flag, call `terminate_execution()`, and return double the limit so the termination can unwind. [cli/lib/worker.rs](https://github.com/denoland/deno/blob/a18ce33715e30cd2b0d99c7e322ef11a65490e4d/cli/lib/worker.rs), [jsruntime.rs](https://github.com/denoland/deno/blob/main/libs/core/runtime/jsruntime.rs)
- deno_core has no CPU-time limit of its own; the embedder measures time and terminates.
- `deno_core` (0.412.0, 2026-09-16) now lives in `denoland/deno/libs/core`. The old repository is archived. [deno_core](https://github.com/denoland/deno_core), [README](https://github.com/denoland/deno/blob/main/libs/core/README.md)
  - It provides no TypeScript and no permission layer.
  - It installs `console`, `queueMicrotask` and `Deno.core`.
  - RUSTSEC-2024-0405 showed `op_panic` reachable from JS. [advisory](https://rustsec.org/advisories/RUSTSEC-2024-0405.html), [01_core.js](https://github.com/denoland/deno/blob/main/libs/core/01_core.js)
- rusty_v8 builds with the **V8 sandbox off** unless the `v8_enable_sandbox` feature is set. [build.rs](https://github.com/denoland/rusty_v8/blob/main/build.rs)

**QuickJS-NG through rquickjs** (0.14.0, 2026-09-18; vendors QuickJS-NG) [rquickjs](https://github.com/DelSkayn/rquickjs)
- The C API offers `JS_SetMemoryLimit` (checked before each allocation), `JS_SetMaxStackSize` and `JS_SetInterruptHandler`. [quickjs.h](https://github.com/quickjs-ng/quickjs/blob/master/quickjs.h)
- The interrupt handler is polled every 10,000 calls or backward jumps, and raises an uncatchable error. It is never polled while the program awaits a host promise. [quickjs.c](https://github.com/quickjs-ng/quickjs/blob/6d46d07d04041b40f4f49eaa7fdebe44c314c699/quickjs.c)
- rquickjs exposes `set_memory_limit`, `set_max_stack_size` (default 256 KiB), `set_interrupt_handler` and `memory_usage` on both `Runtime` and `AsyncRuntime`. Promises map to Rust futures. [base.rs](https://github.com/DelSkayn/rquickjs/blob/master/core/src/runtime/base.rs), [raw.rs](https://github.com/DelSkayn/rquickjs/blob/HEAD/core/src/runtime/raw.rs)
- **Caveat:** the `set_memory_limit` doc comment says it is a no-op with the `rust-alloc` or `allocator` features. The vendored C code checks the limit above the allocator, so the comment may be stale; it needs a test. [base.rs](https://github.com/DelSkayn/rquickjs/blob/master/core/src/runtime/base.rs)
- rquickjs states it does not aim to provide system or web APIs.

**Boa** (v0.22, 2026-08-28)
- `RuntimeLimits` covers loop iterations (unlimited by default), recursion (512) and stack size; exceeding one raises an uncatchable error. [runtime_limits.rs](https://github.com/boa-dev/boa/blob/main/core/engine/src/vm/runtime_limits.rs)
- There is no interrupt or timeout API: [issue 3442](https://github.com/boa-dev/boa/issues/3442) is open, and [PR 5475](https://github.com/boa-dev/boa/pull/5475) and [PR 5380](https://github.com/boa-dev/boa/pull/5380) were closed unmerged.
- No memory limit was found in the source.

## 6. Replay against a journal, and determinism rules

| System | Journal key | Date and random | Timers | Divergence detection | Versioning |
|---|---|---|---|---|---|
| **Cloudflare `CodemodeRuntime`** | Position (`seq` assigned by the host on arrival) | Not virtualised; wrap in `codemode.step` | n/a | Connector, method and sorted-key arguments; results not compared; failure returned to the model as data | None (same code re-run) |
| **OpenAI programmatic calling** | Opaque `fingerprint` | Not documented | Not documented | Not documented | — |
| **Pydantic Monty** | Snapshot of interpreter state at a suspension | — | `max_total_sleep_secs` | n/a (state restored, not re-run) | — |
| Temporal TS | Per-type sequence counters | `Date` = activation time; seeded `Math.random`; no `crypto.randomUUID` | `setTimeout` becomes a durable timer | Command type and ID at the same position; activity arguments not compared | `patched` markers |
| Restate TS | Journal index | `ctx.date`, `ctx.rand` seeded by invocation ID | `ctx.sleep` | Type, parameters and payloads; error 570 | Immutable deployments |
| Vercel Workflow | Seeded-RNG ordinal plus step name | Seeded `Math.random` and `crypto`; log-driven `Date` | `sleep` | `ReplayDivergenceError` | Seed includes deployment ID |
| AWS Lambda durable functions | Hash of the positional counter | Wrap in `context.step` | `context.wait` | Type, Name, SubType → `NonDeterministicExecutionError` | — |
| DBOS TS | Integer step ID plus name | Wrap in a step | Durable sleep | `DBOSUnexpectedStepError` | Source-hash app version, `DBOS.patch` |
| Cloudflare Workflows | Step name (plus occurrence count in local dev) | Wrap in `step.do` | `step.sleep` | None documented | None |
| Inngest | SHA-1 of step ID, `:N` on repeats | Wrap in `step.run` | `step.sleep` | None (order warning only) | Documented add, remove and rename semantics |
| LangGraph functional API | Hash of name, call index and path | Wrap in `@task` | — | None (new ID re-executes) | — |

### Cloudflare `CodemodeRuntime`

This is the nearest precedent for the Effect journal. Sources: [runtime.md](https://github.com/cloudflare/agents/blob/48d9c36f5f1ff18a91fbf0659b9a8e026d1cac43/docs/codemode/runtime.md), [runtime.ts](https://github.com/cloudflare/agents/blob/48d9c36f5f1ff18a91fbf0659b9a8e026d1cac43/packages/codemode/src/runtime.ts)

- **Matching:** the Nth call on a re-run must match the Nth logged call (connector, method, `stableStringify` of the arguments).
- **On divergence:** the run fails, and the model receives an error telling it to wrap nondeterministic work in `codemode.step()`.
- **Concurrency:** `Promise.all` over calls is not replay-stable, because positions follow arrival order. The docs ask for calls to be awaited one at a time in runs that might pause.
- **Entry states:** `executing`, `applied`, `pending` and `reverted`. A crash between `executing` and `applied` re-executes the call.
- **After a pause:** every further call gets a pause decision and records nothing, so code that catches the pause sentinel cannot cause effects.
- **Size cap:** logged values are capped at 1 MB and never truncated, since truncation would corrupt replay; an oversized value fails the run.

The documented replay case is resuming the *same* program after approval. Re-running a *corrected* program is the case the Goodfetch lab measured.

### Other systems' sources

- **Temporal:** [global-overrides.ts](https://github.com/temporalio/sdk-typescript/blob/main/packages/workflow/src/global-overrides.ts), [internals.ts](https://github.com/temporalio/sdk-typescript/blob/main/packages/workflow/src/internals.ts), [activity state machine](https://github.com/temporalio/sdk-rust/blob/main/crates/sdk-core/src/worker/workflow/machines/activity_state_machine.rs), [versioning](https://docs.temporal.io/develop/typescript/workflows/versioning)
- **Restate:** [journaling results](https://docs.restate.dev/develop/ts/journaling-results), [errors.rs](https://github.com/restatedev/sdk-shared-core/blob/main/src/vm/errors.rs), [versioning](https://docs.restate.dev/operate/versioning)
- **Vercel Workflow:** [vm/index.ts](https://github.com/vercel/workflow/blob/main/packages/core/src/vm/index.ts), [replay-divergence.ts](https://github.com/vercel/workflow/blob/main/packages/core/src/replay-divergence.ts)
- **AWS Lambda durable functions:** [replay-validation.ts](https://github.com/aws/aws-durable-execution-sdk-js/blob/main/packages/aws-durable-execution-sdk-js/src/utils/replay-validation/replay-validation.ts), [determinism](https://docs.aws.amazon.com/durable-execution/patterns/best-practices/determinism/)
- **DBOS:** [error.ts](https://github.com/dbos-inc/dbos-transact-ts/blob/main/src/error.ts), [upgrading](https://docs.dbos.dev/typescript/tutorials/upgrading-workflows)
- **Cloudflare Workflows:** [rules](https://developers.cloudflare.com/workflows/build/rules-of-workflows/), [context.ts](https://github.com/cloudflare/workers-sdk/blob/main/packages/workflows-shared/src/context.ts)
- **Inngest:** [engine.ts](https://github.com/inngest/inngest-js/blob/main/packages/inngest/src/components/execution/engine.ts), [versioning](https://www.inngest.com/docs/learn/versioning)
- **LangGraph:** [functional API](https://docs.langchain.com/oss/javascript/langgraph/functional-api)
- **Azure Durable Functions:** [code constraints](https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-code-constraints)

### Patterns common to the JS durable runtimes

- **Clock and randomness:** Temporal and Vercel virtualise them (a frozen or log-driven `Date`, a seeded `Math.random`). The others ask authors to wrap them in a step.
- **Weak references:** both remove `WeakRef` and `FinalizationRegistry`.
- **Position plus a type or name check** is the common key: Temporal, Restate, AWS, DBOS, Cloudflare Code Mode.
- **Name-keyed systems** (Cloudflare Workflows, Inngest) detect no divergence.

## 7. Hooks in scraping and automation frameworks

| Product | Hook points | How shipped | Where it runs | Sandbox |
|---|---|---|---|---|
| Crawlee JS 3.18 / 4.0-rc | `preNavigationHooks`, `postNavigationHooks`, `requestHandler`, `errorHandler`, `failedRequestHandler` | Functions in code. In v4 a hook may return a partial context that is merged in | Engine process (Node) | No |
| Crawlee Python 1.10 | `pre_navigation_hook`, `post_navigation_hook`, router handlers | Decorated coroutines | Engine process | No |
| Scrapy | Downloader and spider middlewares, item pipelines, signals | Classes enabled in settings | Engine process | No |
| scrapy-playwright | `playwright_page_methods` (`PageMethod("evaluate", js)`), init callback | Method names plus arguments in request `meta`, or callables | Engine (callables) and page (`evaluate`) | No / page |
| spider-rs | `on_link_find`, `on_should_crawl`, `on_link_blocked`, `subscribe`; `execution_scripts`, `automation_scripts` | Rust closures; **JS strings keyed by URL path** | Engine (closures); page (JS through `page.evaluate`, automation up to 60 s) | No / page |
| Spider Cloud | `execution_scripts`, `automation_scripts` | JS or JSON in the request | Page | Page |
| Apify scrapers | `pageFunction`, `preNavigationHooks`, `postNavigationHooks` | **JS strings in Actor input**, compiled with `vm.runInThisContext` | Node; Web Scraper's `pageFunction` in the page | No (the user's own container) |
| Firecrawl | `actions` (up to 50, including `executeJavascript`); Interact code | JSON and strings | Page; Interact code in a "browser sandbox" | Page / not stated |
| Browserless v2 | `/function`, BrowserQL `evaluate` | JS/ESM over HTTP | A browser tab (v1 ran Node under vm2) | Browser |
| Bright Data Scraper Studio | Interaction and parser code | Code in the hosted IDE | Their "main JavaScript context" | Not documented |
| Zyte API | `actions` with `evaluate` and `source`; browser scripts (`interaction`) | JSON; deployed TypeScript classes | Page (`evaluate`); scripts not stated | Not documented |
| ScrapingBee | `js_scenario` `evaluate` | JSON | Page, 40 s for the whole scenario | Page |
| ScraperAPI | Fixed instruction set, no JS | JSON | n/a | n/a |

Sources:
- **Crawlee:** [v3 browser-crawler](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts), [v4](https://github.com/apify/crawlee/blob/master/packages/browser-crawler/src/internals/browser-crawler.ts), [Python](https://github.com/apify/crawlee-python/blob/master/src/crawlee/crawlers/_playwright/_playwright_crawler.py)
- **Scrapy:** [downloader middleware](https://docs.scrapy.org/en/latest/topics/downloader-middleware.html), [spider middleware](https://docs.scrapy.org/en/latest/topics/spider-middleware.html), [scrapy-playwright](https://github.com/scrapy-plugins/scrapy-playwright/blob/main/README.md)
- **spider-rs:** [configuration.rs](https://github.com/spider-rs/spider/blob/main/spider/src/configuration.rs), [chrome_common.rs](https://github.com/spider-rs/spider/blob/main/spider/src/features/chrome_common.rs), [client config](https://github.com/spider-rs/spider-clients/blob/main/javascript/src/config.ts)
- **Apify:** [tools.ts](https://github.com/apify/actor-scraper/blob/master/packages/scraper-tools/src/tools.ts), [web-scraper setup](https://github.com/apify/actor-scraper/blob/master/packages/actor-scraper/web-scraper/src/internals/crawler_setup.ts)
- **Firecrawl:** [advanced scraping](https://docs.firecrawl.dev/advanced-scraping-guide.md), [safe-mode.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/safe-mode.ts), [browser execute](https://docs.firecrawl.dev/api-reference/endpoint/browser-execute.md)
- **Browserless:** [function handler](https://github.com/browserless/browserless/blob/main/src/shared/utils/function/handler.ts), [issue 3726](https://github.com/browserless/browserless/issues/3726), [BQL evaluate](https://docs.browserless.io/bql-schema/operations/mutations/evaluate)
- **Bright Data:** [functions](https://docs.brightdata.com/products/scraper-studio/functions.md)
- **Zyte:** [browser actions](https://docs.zyte.com/zyte-api/usage/browser.html), [IDE](https://docs.zyte.com/zyte-api/ide/index.html)
- **ScrapingBee:** [js_scenario](https://www.scrapingbee.com/documentation/js-scenario/)
- **ScraperAPI:** [instruction set](https://docs.scraperapi.com/making-requests/customizing-requests/rendering-javascript/render-instruction-set)

**Where hooks run**
- **Engine-side hooks are always trusted code**, written in the engine's own language and running in-process: Crawlee, Scrapy, spider-rs closures, Apify hooks.
- **Hooks carried as data are page JavaScript.** Spider's `execution_scripts`, Firecrawl's `executeJavascript`, Zyte's and ScrapingBee's `evaluate`, and Browserless functions all run in the page.
- **The one exception is Apify's Node hooks.** They also travel as data, but are compiled with `vm.runInThisContext` and confined only by the user's own container.
- **Firecrawl's Safe Mode** removes `executeJavascript` from the allowed actions.
- **Browserless** answered the vm2 CVEs by moving functions into the browser.

## 8. Sandbox escapes and advisories (dated)

| Date | Project | ID | What | Source |
|---|---|---|---|---|
| 2018 | static-eval | CVE-2017-16226 | Breakout | [GHSA](https://github.com/advisories/GHSA-5mjw-6jrh-hvfq) |
| 2019-10-04, 10-21 | realms-shim (Figma) | GHSA-6jg8-7333-554w, GHSA-7cg8-pq9v-x98q | Sandbox breakout; Figma moved to QuickJS in Wasm | [GHSA](https://github.com/advisories/GHSA-7cg8-pq9v-x98q), [Figma](https://www.figma.com/blog/an-update-on-plugin-security/) |
| 2020-08-25 | safe-eval | CVE-2020-7710 | Breakout | [GHSA](https://github.com/advisories/GHSA-hrpq-r399-whgw) |
| 2021-03-30 | isolated-vm | CVE-2021-21413 | Misused `Reference` reaches the Node isolate | [advisories](https://github.com/laverdet/isolated-vm/security/advisories) |
| 2022-03-18 | notevil | CVE-2021-23771 | Escape | [GHSA](https://github.com/advisories/GHSA-8g4m-cjm2-96wq) |
| 2022-03-25 | Deno | CVE-2022-24783 | FFI permission bypass | [GHSA](https://github.com/denoland/deno/security/advisories/GHSA-838h-jqp6-cf2f) |
| 2023-04-06 to 07-12 | vm2 | CVE-2023-29017, -30547, -32314, -37466, -37903 | Escapes | [advisories](https://github.com/patriksimek/vm2/security/advisories) |
| 2023-07-09 | vm2 | — | Discontinued | [README history](https://github.com/patriksimek/vm2/commits/main/README.md) |
| 2024-05-07 | Deno | CVE-2024-34346 | `/proc` and `/dev` access amounts to allow-all | [GHSA](https://github.com/denoland/deno/security/advisories/GHSA-23rx-c3g5-hv9w) |
| 2024-07-18 | deno_core (via rustyscript) | RUSTSEC-2024-0405 | `op_panic` reachable from JS | [RustSec](https://rustsec.org/advisories/RUSTSEC-2024-0405.html) |
| 2024-08-14 | Boa | CVE-2024-43367 | DoS | [GHSA](https://github.com/boa-dev/boa/security/advisories/GHSA-f67q-wr6w-23jq) |
| 2025-04-27 | QuickJS / QuickJS-NG | CVE-2025-46687, -46688 | Heap overflow | [NVD](https://nvd.nist.gov/vuln/detail/CVE-2025-46687) |
| 2025-07-27 | smolagents | CVE-2025-5120 | Sandbox escape through allowlisted modules; fixed in 1.17.0 | [GHSA](https://github.com/advisories/GHSA-6v92-r5mx-h5fx) |
| 2025-09-03 | smolagents | CVE-2025-9959 | Escape through dunder attributes | [NVD](https://services.nvd.nist.gov/rest/json/cves/2.0?cveId=CVE-2025-9959) |
| 2025-10-16 | QuickJS | CVE-2025-62490 to -62496 | Use-after-free, type confusion, overflow | [NVD](https://nvd.nist.gov/vuln/detail/CVE-2025-62494) |
| 2025-10-24 | vm2 | — | Project revived (3.12.2 on 2026-09-08) | [README history](https://github.com/patriksimek/vm2/commits/main/README.md) |
| 2025-12-23 | smolagents | CVE-2025-14931 | Pickle deserialisation RCE in the remote executor | [GHSA](https://github.com/advisories/GHSA-q9r5-6hrr-9ph7) |
| 2025-12-19, 12-24 | n8n | CVE-2025-68613, CVE-2025-68668 | Expression RCE; Pyodide Code node command execution | [advisories](https://github.com/n8n-io/n8n/security/advisories) |
| 2026-01 | QuickJS-NG | CVE-2026-0821, -0822, -1144, -1145 | TypedArray and Atomics bugs | [NVD](https://nvd.nist.gov/vuln/detail/CVE-2026-0821) |
| 2026-01-25 | vm2 | CVE-2026-22709 | Escape | [GHSA](https://github.com/patriksimek/vm2/security/advisories/GHSA-99p7-6v5w-7xg8) |
| 2026-02-25 | n8n | CVE-2026-27495 | JS task-runner escape | [GHSA](https://github.com/n8n-io/n8n/security/advisories/GHSA-jjpj-p2wh-qf23) |
| 2026-03-27 | smolagents | CVE-2026-4963 | Incomplete fix of CVE-2025-9959; no patched version listed | [GHSA](https://github.com/advisories/GHSA-54fq-v6x8-244g) |
| 2026-05 to 2026-09 | vm2 | About 60 GHSAs (e.g. CVE-2026-26956, a Wasm escape) | Escapes, DoS, NodeVM allowlist bypasses | [advisories](https://github.com/patriksimek/vm2/security/advisories) |
| 2026-05-11 | QuickJS-NG | CVE-2026-37630 | Arbitrary code execution via `js_mapped_arguments_mark` | [NVD](https://nvd.nist.gov/vuln/detail/CVE-2026-37630) |
| 2026-05-27 | Deno | CVE-2026-49859, -49860 | `fetch` and WebSocket post-DNS check bypass | [GHSA](https://github.com/denoland/deno/security/advisories/GHSA-cpgj-f7g3-2pp2) |
| 2026-05-28, 07-31 | QuickJS-NG | GHSA-m53g-qx47-3h38 (critical RCE in `toReversed`) plus 8 more | Use-after-free and out-of-bounds writes, some on out-of-memory paths | [advisories](https://github.com/quickjs-ng/quickjs/security/advisories) |
| 2026-08-05 | n8n | GHSA-m3hg-p5r9-fg9h | JS runner `vm` escape through EventEmitter prototype pollution | [GHSA](https://github.com/n8n-io/n8n/security/advisories/GHSA-m3hg-p5r9-fg9h) |
| 2026-08-07 | isolated-vm | GHSA-864f-rcv7-6rh4 | `ExternalCopy` type confusion leading to control-flow hijack | [advisories](https://github.com/laverdet/isolated-vm/security/advisories) |
| 2026-09-04 | LaVague | CVE-2026-85694 | `eval` of LLM output taken from page content: prompt injection to RCE. [Issue 650](https://github.com/lavague-ai/LaVague/issues/650) is still open | [NVD API](https://services.nvd.nist.gov/rest/json/cves/2.0?cveId=CVE-2026-85694) |
| 2026-09-24 | QuickJS (Bellard) | CVE-2026-88378 | Out-of-bounds write in `JS_ReadFunctionTag` (bytecode reader) | [NVD](https://nvd.nist.gov/vuln/detail/CVE-2026-88378) |

**Related warnings**
- **Node's documentation** says `node:vm` is not a security mechanism. [vm.md](https://github.com/nodejs/node/blob/main/doc/api/vm.md)
- **vm2's README** now expects new bypasses and recommends isolated-vm, processes or microVMs for fully untrusted code. [vm2](https://github.com/patriksimek/vm2)
- **No advisories found** in the GitHub advisory database or RustSec for rquickjs, rquickjs-sys or the `deno_core` crate itself. RUSTSEC-2024-0405 was filed against rustyscript. [advisory-db](https://github.com/rustsec/advisory-db)

## 9. Not verified

**Cloudflare**
- Whether the Code Mode sandbox has `URL`, `TextEncoder`, `structuredClone`, `crypto.randomUUID`, `Intl` and `Temporal`. workerd's global set was not enumerated.
- Whether Dynamic Workers have a memory limit other than 128 MB.
- The internals of Cloudflare's production CPU enforcer (closed source).
- The production cache key of Cloudflare Workflows. Only the local-dev code was read.

**Model vendors**
- OpenAI programmatic calling: time and memory limits, maximum tool calls, globals (timers, `Intl`, `Temporal`), and how tool declarations are shown to the model.
- Anthropic: the stub format shown to Claude for programmatic tools, and the size behind `output_file_too_large`.

**Agent and code-mode tools**
- Why goose moved from Boa to pctx.
- Mastra's default limits.
- @sebastianwessel/quickjs's default memory and timeout.
- Whether UTCP code-mode accepts TypeScript syntax.

**Engines**
- Whether rquickjs's "memory limit is a no-op with a custom allocator" note still holds.
- Whether QuickJS-NG out-of-memory errors are catchable from JS.
- Whether `Deno.core.ops.op_panic` is still reachable from plain scripts on `deno_core` 0.412.

**Platforms**
- Figma: the exact QuickJS build in use today and any timeout.
- Shopify: which Wasm runtime executes Functions.
- n8n: which code path still uses vm2 (it is still a dependency in 2026).
- Deno Sandbox's maximum lifetime: the docs index says 30 min, the timeouts page names no maximum.
- E2B's default resources.

**Scraping and automation products**
- The isolation behind Browserbase Functions and Director, Hyperbrowser Sandboxes, Kernel VMs, Firecrawl Interact, Bright Data's IDE and Zyte browser scripts.
- Whether Stagehand's facade `run` tool is in a published npm release (only `main` was read).

**Other**
- LaVague: no report or warning written by LaVague itself was found; the only sources are the third-party issue and the CVE.
- No source was found for a per-program tool-call cap at Cloudflare, OpenAI or Anthropic.
