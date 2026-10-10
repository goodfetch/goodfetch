# Proposed API: short in use, explicit in production

**Proposal of October 7, 2026. All examples are contracts to implement, not available code.** The final names and schemas will need to go through an ergonomics review and cross-SDK tests. The `.example` domains below are fictitious.

## 1. Design rules

1. A simple case fits in a few lines; advanced parameters are grouped into reusable policies.
2. Deterministic without model configuration. A natural-language instruction is an explicit AI call, with cost and provenance.
3. Same run, types and semantics locally and in the cloud. No internal Playwright/Chromey object in the portable contract.
4. Plan reference ≠ cached data; fresh extraction by default for a new run.
5. Each operation returns its technical status; a run has a result and a validation verdict that are distinct.
6. Streaming, pagination, deadlines and cancellation from the initial contract.
7. Costly policies — AI escalation, repair, proxy — are visible in the plan and the log.

## 2. One-off extraction

A selector-based extraction does not use an LLM. `access: "auto"` selects HTTP or browser according to capabilities/history and validation, without introducing an agent loop. The schema provides types; the validators carry the business contract.

```ts
import { automan } from "@automan/sdk";
import { z } from "zod";

const client = await automan.connect({ target: "local" });

const result = await client.extract({
  url: "https://catalogue.example/products/p-17",
  schema: z.object({ sku: z.string(), price: z.number() }),
  selectors: {
    sku: { css: "[data-sku]", attribute: "data-sku" },
    price: { css: ".price", transform: "decimal", locale: "fr-FR" },
  },
  access: "auto",
  validate: { identity: { field: "sku", equals: "p-17" } },
});

console.log(result.data, result.validation, result.runId);
await client.close();
```

The SDK converts the supported schema to JSON Schema and checks the restrictions. An arbitrary JavaScript refinement is not automatically portable; it remains a code validator in a package or is expressed as a standard rule. Python exposes Pydantic/JSON Schema models with the same contractual limit.

An assisted extraction is explicitly different:

```ts
const result = await client.extract({
  url: "https://catalogue.example/products/p-17",
  instruction: "Extract the displayed SKU and unit price including VAT.",
  schema: Product,
  access: "browser",
  model: "extraction-default",
  validate: ProductChecks,
});
```

`model` is a configured provider alias; no model version is implicitly presented as portable or permanent. If the provider is missing, the AI call fails with a configuration error before incurring any cost. An error does not trigger a paid repair by default in a one-off extraction.

## 3. Browser: deterministic and natural in the same session

```ts
const session = await client.browser.open({
  backend: "auto",
  identity: "supplier-readonly",
  requires: ["browser.downloads", "browser.frames"],
});

try {
  const page = await session.newPage();
  await page.goto("https://supplier.example/exports");

  // Deterministic: locator re-resolved at the moment of acting, without a model.
  const download = await page.expectDownload({ timeoutMs: 30_000 });
  await page.locator({ role: "button", name: "Export" }).click();
  const file = await download.result();
  await file.save({ destination: "artifacts", name: "export.csv" });

  // Explicit AI: typed resolution, then execution and postcondition.
  const actions = await page.observe({
    instruction: "Find the button to export the previous month.",
    model: "decision-default",
  });
  await page.act(actions.requireUnique(), {
    expect: { download: { mime: "text/csv" } },
  });
} finally {
  await session.close();
}
```

`observe` returns candidates and their document/page/frame references, never a target silently chosen among several. `act(Action)` executes an already resolved action without inference. `act({ instruction, model, expect })` is a separate assisted shortcut. The effects contract depends on the workflow: clicking "Confirm order" does not inherit the retry policy of a read.

`expectDownload` acknowledges that the wait is installed before returning its handle: it precedes the click so as to also capture an immediate download. The handle carries a deadline, a result and cancellation. Waits for other events that may occur during an action follow the same registration contract.

A session has an owning backend. If a capability is missing, choose the appropriate backend when opening the session; an active session does not arbitrarily switch from CDP to Playwright. Direct access to the native driver is a non-portable code extension, identified as such.

## 4. Crawl: portable recipe or code handler

### 4.1 Portable recipe

```ts
const run = await client.crawl({
  seeds: ["https://catalogue.example/"],
  workflow: "catalogue.collect@7",
  queue: { name: "catalogue-octobre", resume: true },
  scope: { sameDomain: true, maxDepth: 6, maxRequests: 100_000 },
  limits: { http: 128, browserPages: 12, perDomain: 4 },
  autoscale: true,
  retry: { maxAttempts: 4, backoff: "exponential-jitter" },
  dataset: { name: "catalogue", mode: "upsert", key: ["sku"] },
});

for await (const event of run.events()) {
  console.log(event.type, event.progress);
}
const outcome = await run.result();
```

The concurrency values are **illustrative**, not optimal values or a capacity promise. Per-resource limits are bounded by global quotas. Resuming loads the existing queue, without an implicit purge. The upsert dataset requires a declared stable identity; an append dataset with a commit key is a different policy.

### 4.2 Local TypeScript handler

```ts
const run = await client.crawl({
  seeds: ["https://catalogue.example/"],
  handler: async ({ document, request, discover, emit }) => {
    await discover({ selector: "a.product", label: "product" });
    if (request.label === "product") {
      await emit({
        key: document.attribute("[data-sku]", "data-sku"),
        value: readProduct(document),
      });
    }
  },
});
```

This callback stays in the Node process of the attached client. It is not serialized as JSON to Rust and is not resumed in the middle of its lines after a crash. For the cloud, `automan package build` creates a versioned package with runtime/handler, dependencies, schemas, permissions and tests; the engine hands it work units in an isolated host. Handlers must respect their retry contract and avoid hidden non-idempotent mutations in a collection.

`emit` and `discover` acknowledge **staging**, tied to the request/attempt/lease epoch. The engine publishes data and new requests together with the acknowledgement after the handler succeeds and validation passes. A failed handler does not silently publish its provisional outputs. An explicit partial-results policy can publish a validated subset and its partial state; the callback's direct external effects remain outside this transaction.

## 5. Versioned workflow and repair

```ts
const workflow = await client.workflows.create({
  name: "supplier.monthly-export",
  input: ExportInput,
  output: ExportDataset,
  identity: "supplier-readonly",
  plan: MonthlyExportPlan,
  validate: [AccountIdentity, MonthCoverage, CsvSchema],
  repair: {
    mode: "validated-auto",
    allowed: ["locator", "wait", "extraction-mapping"],
    maxCandidates: 2,
    budget: { maxModelCalls: 4 },
    tests: "supplier-export-regressions",
  },
});

const run = await workflow.run({ month: "2026-09" });
const outcome = await run.result();
```

`validated-auto` requires the tests and the required validators; otherwise the repair remains a `needs_review` candidate. The proposed modes are `off`, `suggest`, `validated-auto`. A workflow with non-idempotent effects does not become repeatable because a locator is repaired. The limit values are illustrative; the run's global cap also applies to repair.

Compilation is a separate durable object:

```ts
const candidate = await client.workflows.compile({
  goal: "Download the requested month's export from the supplier portal.",
  parameters: ExportInput,
  identity: "supplier-readonly",
  model: "planner-default",
  validate: [AccountIdentity, MonthCoverage, CsvSchema],
});

const report = await candidate.test({ suite: "supplier-export-regressions" });
if (report.qualified) await candidate.publish();
```

A publication fixes a new version. The test run displays data/evidence/cost and any qualification refusal. `qualified` means that this suite's requirements pass, not that all future sites will work. A recorder can produce the same type of candidate from a demonstration, with redaction and reviewed parameters.

## 6. Built-in or external agent

```ts
const run = await client.agent.run({
  goal: "Find and retrieve the spec sheets of the three requested models.",
  input: { models: ["A17", "B20", "C42"] },
  tools: ["browser.read", "browser.navigate", "extract", "artifacts.write"],
  domains: ["catalogue.example"],
  model: "planner-default",
  validate: RequestedModelsPresent,
  limits: { maxActions: 40, maxModelCalls: 20, timeoutMs: 180_000 },
});
```

An external agent calls the tools via SDK, CLI or MCP and can obtain a compact description of the capabilities, then the details on demand. No agent framework is mandatory. Reusable site/plan memory is separate from the run's conversational state. The terminal result is decided by the contract and the validators, not solely by the agent's text.

## 7. Common shape of results and errors

```json
{
  "runId": "run_demo_42",
  "workflowVersion": "supplier.monthly-export@8",
  "status": "succeeded",
  "data": { "datasetId": "dataset_demo_9", "rowCount": 248 },
  "validation": {
    "status": "passed",
    "checks": ["account-identity", "month-coverage", "csv-schema"],
    "evidence": ["artifact_demo_12"]
  },
  "usage": { "modelCalls": 0, "browserMs": 4300, "attempts": 1 },
  "eventsCursor": "cursor_demo_27"
}
```

Demo data and figures only. Results also contain the identity scope, timestamps/freshness, backend/capabilities, warnings and inconclusive checks. Secrets and voluminous observations are excluded from the envelope by default.

Proposed errors: `CAPABILITY_UNAVAILABLE`, `TARGET_AMBIGUOUS`, `REFERENCE_STALE`, `PAGE_GONE`, `AUTH_REQUIRED`, `RATE_LIMITED`, `BUDGET_EXCEEDED`, `DATA_INCOMPLETE`, `VALIDATION_FAILED`, `LEASE_LOST`, `EFFECT_UNKNOWN`, `CLIENT_HANDLER_DISCONNECTED`. Each error indicates retryability, possible effect, evidence, next action and attempt. An unknown error from a newer version remains representable by its code and its compatible details.

`run.events({ after })` resumes from a cursor. If the requested history has expired, `EVENT_CURSOR_EXPIRED` provides a snapshot and a new resumption point; the client does not pretend to have received the missing events. `run.cancel()` is idempotent; its acknowledgement of the request does not mean that all operations have already stopped.

A `waiting` run can resume with `runs.resume` once its wait is resolved. A terminal run keeps its history: `runs.reconcile` attaches a proven effect resolution to it, then `runs.fork({ fromRun, resolution })` creates a new run if a continuation is needed. Checkpoints are revalidated according to their freshness and confirmed effects are not repeated. An unknown effect without an authorized resolution forbids forking the affected mutating steps.

## 8. CLI

```sh
automan extract 'https://catalogue.example/products/p-17' --recipe product --json
automan run supplier.monthly-export --input ./month.json --wait --json
automan crawl catalogue.collect --seeds ./urls.txt --queue catalogue --resume
automan runs inspect run_demo_42 --events --json
automan runs cancel run_demo_42
automan workflows repair supplier.monthly-export --from-run run_demo_42 --suggest
automan package build ./supplier-workflow
automan serve --local
automan mcp --stdio --profile read
```

The paths are example parameters. `--json` reserves stdout for data; logs/diagnostics go to stderr. `--events` produces bounded NDJSON events and a terminal state. Proposed exit codes: 0 verified success/successful administrative command, 1 failure, 2 partial, 3 decision required, 4 unknown effect, 130 user interruption. Scripts can choose to wait for the terminal state or to detach; the run outlives the CLI when the daemon owns its execution.

Installation via signed/packaged binaries, resolution of compatible versions, `doctor`, version/capabilities and explicit updates. Installation of browsers/profiles is observable; no mandatory `cargo` on supported binary platforms.

## 9. MCP

Proposed profiles: `read`, `browser`, `workflows`, `operator`. The advertised catalog depends on the available capabilities/permissions. Names are stable and input/output schemas explicit.

| Tool | Purpose and result |
| --- | --- |
| `automan_capabilities` | Available capabilities, limitations, detailed schemas on demand |
| `automan_extract` | One-off extraction with schema/policy and referenced result |
| `automan_browser_open` | Scoped session/page and qualified backend |
| `automan_browser_observe` | Bounded observation with references and generation |
| `automan_browser_act` | Typed action and postcondition; effect permissions checked |
| `automan_workflow_run` | Durable run of a version or explicit reference |
| `automan_run_inspect` | State, checks, costs and paginated next events |
| `automan_run_cancel` | Idempotent cancellation request |
| `automan_dataset_read` | Projection/pagination and provenance |
| `automan_repair_propose` | Bounded candidate attached to the failed run, never implicit promotion |

stdio mode serves local agents, Streamable HTTP remote uses. The adapter follows the negotiated MCP version, the authorization and the tool model of the current specification; the bidirectionality of Automan's internal RPC does not define that of MCP. Run events are data inspectable via a compatible tool/resource, not JSON-RPC requests arbitrarily initiated by the server. [MCP transports 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

No `eval`/arbitrary code in the standard profile. An explicit advanced capability can serve trusted code with its own permissions. Resources/artifacts have references, scopes and access duration. Tool annotations help the client, but do not replace permission enforcement by the engine.

## 10. Cloud API and advanced ergonomics

REST resources: workflows/versions/packages, runs/steps/events, queues/requests, datasets/exports, artifacts, identities/sessions, pools/workers, schedules, repairs, usage and workspace roles. Unambiguous commands: start, cancel, pause admission, drain worker, propose/test/promote repair, rotate/retire session. Durable operations return an identifier and an inspectable state.

Idempotency keys for run creation and repeatable commands; version/ETag for concurrent configuration changes; cursors for lists/events; explicit dates/units. Secrets are references. The console and macOS app use these contracts instead of adding invisible behaviors specific to their interface.

Advanced parameters for pools, hooks, plugins and schedulers are described by schemas and importable presets. A low-complexity configuration remains readable; the user can export the effective configuration, see the overrides and the exact cause limiting their run.
