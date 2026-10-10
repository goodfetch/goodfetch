# Automan — capability matrix for agents, plans and repair

Research consolidated as of **October 7, 2026**. This document contributes to the architecture/design/PRD dossier: it covers the browser runtime, agents, plan compilation, perception, tools and repair. It sets no limit on team size or timeline. No Automan prototype or competitor benchmark was run.

**Traceability:** reuse of the local investigations [frameworks](2026-10-07-browser-frameworks.md), [Stagehand contracts and alternatives](2026-10-07-stagehand-alternatives-followup.md), [CDP and latency](2026-10-07-stagehand-cdp-latency.md), [Crawlee/Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md). Three new Exa searches, eight results requested each: `sources_reviewed = 24`, a counter of requested results including duplicates/discarded ones, not validated independent sources. New targeted reads: Scrapling parser/storage/license; Playwright MCP README/license; agent-browser README, Rust MCP, snapshots, reference lifecycle and security.

**Evidence levels:** `P` = published package/version inspected; `D` = documented by the vendor; `C` = code inspected, in this research or a cited local report; `A` = Automan proposal; `T` = behavior/performance to be demonstrated. `C` establishes an implementation path, not its universal reliability. A "not established" cell means the investigation did not qualify this capability; it does not prove its absence.

New pinned references: Scrapling [`43dee004866e1c46843a9ac293cc1494aa7915d6`](https://github.com/D4Vinci/Scrapling/tree/43dee004866e1c46843a9ac293cc1494aa7915d6), agent-browser [`98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba`](https://github.com/vercel-labs/agent-browser/tree/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba), Playwright MCP [`f183dad4a52965583e3cc1d59b88cdc279e2e57d`](https://github.com/microsoft/playwright-mcp/tree/f183dad4a52965583e3cc1d59b88cdc279e2e57d). These SHAs pin the sources consulted; they do not give each feature stable-release status.

## 1. Design decision

**Automan must own an execution contract shared by deterministic programs, agents and compiled plans.** The HTTP/CDP acquisition mode, the model and the chosen framework remain adapters. A browser, a cache, a similarity score or a valid JSON output must not on their own decide that the business task succeeded.

The four modes must coexist and be visible:

| Proposed Automan mode `A` | Who decides the steps? | LLM during a normal run? | Verifiable promise |
| --- | --- | --- | --- |
| Deterministic program | Code/plan provided by the developer | No | Same logic; current data read according to the freshness policy |
| Interactive agent | External or built-in loop, perceptions and tools | Yes, depending on provider | Budget, states, attempts and evidence inspectable; no universal guarantee of success |
| Assisted compilation | Agent explores/records, then produces a qualified plan | No if all plan nodes are deterministic | Measured savings on reruns; versioned plan and verified results |
| Hybrid plan / repair | Stable plan, targeted escalation on one step | Only at declared points | Escalation cost and effect of each retry known; abstention possible |

Replay and repair already exist at Skyvern, Imprint and Browser Use. The matrix must therefore guide reuse and acceptance criteria, without claiming fictitious novelty.

## 2. Matrix — substrate and browser tools

| Granular capability | Stagehand 4.1.0 | Playwright + CLI/MCP | Current agent-browser | Browser Use |
| --- | --- | --- | --- | --- |
| Deterministic execution without inference | `P/C` Page/Locator; act(Action) avoids inference but still requires a configured model/gateway | `D/C` locators, actions, scripts; MCP/CLI client model optional | `D/C` Rust commands; AI chat optional | `D/C` actions/tools and scripts; legacy Agent and beta to be distinguished |
| Chromium backend | `P/C` Understudy CDP in extension | `C` Chromium backend already uses CDP; other Firefox/WebKit backends | `C` native Rust daemon/CDP | `C` legacy Python/CDP; beta Rust server, with Python helpers/worker |
| Snapshot for agent | `P` hybrid AX/DOM tree, XPath and URL maps | `D` AX snapshots, depth/region/boxes depending on tool | `D/C` compact AX, refs, removedRefs | `C` DOM + AX + frames, references, screenshot depending on configuration |
| Reference identity | `P/C` selectors and frame IDs used by the runtime; business guard to add | `D` snapshot refs and locators; scope depends on the document | `D/C` surviving DOM refs kept, replaced ones invalidated, IDs not recycled; virtual AX refs limited to the snapshot | `C` state and indices projected into the loop; validity to qualify per backend |
| Frames/Shadow DOM | `D` iframe `>>`, CSS piercing shadow including closed | `D/C` locators/frames; contract depends on the browser | `C` OOPIF sessions, hit-test; `D` iframe snapshot with explicit limits | `C` per-frame services; beta and legacy not presumed identical |
| Action auto-wait/retry | `D/P` no Playwright contract; one-shot resolution and explicit waits | `D` actionability + web-first assertions | `C` native checks; full Playwright parity `T` | `C` watchdogs/budgets; Playwright parity `T` |
| Network and scripts | `P/D` evaluate/init scripts; no route interception or equivalent network waits | `D` interception, requests, console; arbitrary server code via an explicitly unsafe tool | `D` routes/requests/HAR/eval/MCP profiles | `C/D` script tools, events/watchdogs; surface depends on generation |
| Execution proximity | `P/D` extension and experimentalBatch | `D` script/run-code next to the driver; remote depending on deployment | `C` resident daemon; batching/tools depending on profile | `C` resident beta runtime and SDK protocol |
| Isolation and ownership | `P/D` one Context per browser; create several browsers for isolated profiles | `D` Contexts; MCP persistent profile exclusive per instance, or isolated | `D` distinct sessions/browsers; strict pin-tab available for shared CDP | `C/D` BrowserSession, durable beta sessions; depends on the configured identity |
| Agent-ready MCP/CLI | TS/Python/Go SDK `P`; external client/tooling to choose | `D` CLI+skills and typed MCP; two different context usages | `D/C` JSON CLI and stdio MCP, profiles/pagination/annotations | `D/C` terminal/harness and wrappers; separate cloud SDK |
| Page-provided tools | `P` tools/invoke/result/cancel types; actual availability `T` | `D` WebMCP collection and option to disable | `D/C` experimental WebMCP, compact discovery and schema on demand | Not qualified in this investigation |

Sources: [Stagehand 4.1.0 stable types/bundle](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz), [Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright), [Stagehand batch](https://docs.stagehand.dev/v4/reference/stagehand), [Playwright Chromium](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/chromium/crBrowser.ts), [actionability](https://playwright.dev/docs/actionability), [Playwright CLI](https://github.com/microsoft/playwright-cli), [pinned MCP README](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/README.md), [pinned agent-browser README](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md), [pinned Rust snapshot](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/cli/src/native/snapshot.rs), [BrowserSession](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/session.py), [beta ledger](https://github.com/browser-use/browser-use/blob/main/BETA_AGENT_INTEGRATION_FEATURES.md).

**Reference limits to keep in the PRD:** the [pinned agent-browser docs](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/skill-data/core/references/snapshot-refs.md) state that a single iframe level is expanded in the snapshot and that some cross-origin content is skipped. Its iframe invalidation tests do not establish universal coverage of nested frames. Likewise, the fact that two frameworks use CDP gives neither object compatibility nor the same waits: external `connectOverCDP` has a [documented lower fidelity](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp) than the connection to the Playwright server.

## 3. Matrix — perception, planning, validation and replay

| Capability | Stagehand | Skyvern | Magnitude | Imprint | Scrapling |
| --- | --- | --- | --- | --- | --- |
| Semantic resolution | `P/C` observe/act via model over the tree; extract with optional screenshot | `D` prompt and vision; selector + AI fallback; Python AILocator | `D/C` coordinate-based vision, tasks of variable granularity | `C/D` interpretation during recording compilation | `C` structural similarity without LLM |
| Multi-step agent | `P/D` agent() removed in v4; external loop/code mode | `D/C` multi-step agents and workflows | `D` act can cover several interactions | `C` compiler agent; deterministic runtime | Not established; fetch/parser/spider separate |
| Reusable compiled plan/script | `P/C` replayable actions; not a complete durable business IR | `D/C` code caching per task/block/branch | `D` native deterministic cache announced as in progress; not acquired | `C/D` workflow API + DOM playbook + emitted MCP | `C` persisted element properties, not an effects plan |
| Typed results | `P/C` Zod/Pydantic/Go + data/metadata | `D` extract, validate bool, Task/WorkflowRunResponse | `D` Zod extract; separate runner | `C/D` MCP tools and generated validation | `C` Selectors/text/JSON, business checks by the caller |
| Verification independent of the agent | `C` schema; completed judge without source snapshot, not exposed | `D` LLM validate; code assertions possible via Playwright | `D` LLM check; Page/Context access for code | `C/D` verifier independent of compilation, and receipts | `C` resemblance score; no proof of business identity |
| Output cache | `P/C` SDK prepares cacheValue and returns cacheHit; server storage/invalidation opaque | Cache mainly of scripts `D` | Provider prompt caching `D`, no guaranteed output | Artifacts/plan/backend; rereads the source on the replay path `D/C` | Fingerprint recording then relocation `C` |
| Repair triggered by | `P/C` operation exception; failed cache replay → inference | `D` failed cached code → agent and regeneration | Not qualified as durable workflow repair | `C/D` transport failures escalate the ladder; semantics handled by compiler/verifier | `C` selector returns no result, if adaptive is enabled and properties saved |
| Wrong target still valid | No acquired semantic guard `P/C` | To test with flow validations `T` | To test with assertions `T` | To test outside generated suites `T` | Target returned directly without relocation `C` |
| Business freshness on rerun | Opaque server cache policy `T` | Live script, invariants to define `D/T` | Current observations; invariants to define `D/T` | Current requests; receipts and invariants to define `D/C/T` | New parsed response; old target fingerprint, current data read `C` |

Sources: [inspected Stagehand contracts](2026-10-07-stagehand-alternatives-followup.md), [Skyvern actions](https://www.skyvern.com/docs/developers/browser-automations/actions-reference), [Skyvern code caching](https://www.skyvern.com/docs/developers/features/code-caching), [Magnitude BrowserAgent](https://docs.magnitude.run/reference/browser-agent.md), [Magnitude checks](https://docs.magnitude.run/reference/test-case-agent.md), [Magnitude memory/prompt cache](https://docs.magnitude.run/advanced/memory.md), [Imprint architecture](https://github.com/ashaychangwani/imprint/blob/main/docs/architecture.md), [Imprint verifier](https://github.com/ashaychangwani/imprint/blob/main/src/imprint/live-verifier.ts), [Scrapling adaptive docs](https://scrapling.readthedocs.io/en/latest/parsing/adaptive/), [pinned parser](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py).

Two additional references avoid underestimating the market: **Browser Use Cloud V4** keeps scripts in a workspace, rereads the sources and offers repair; each rerun nevertheless restarts a model and consumes tokens. **AgentQL** returns Playwright Locators after a semantic query, or data after queryData; the calling program keeps control. [Browser Use scripts](https://docs.browser-use.com/cloud/agent/scripts), [AgentQL Page](https://docs.agentql.com/javascript-sdk/api-references/agentql-page).

**Spider agent must also appear as a real reference.** In Spider/spider_agent 2.53.9, the published Rust loop has browser actions, tools, error feedback, round limits and repetition detection. Its success depends mainly on the model's plan.done. The local audit does not demonstrate runtime activation of the planning_mode, confidence_strategy, html_diff_mode and self_healing options: exposing an option does not establish its effect. [Published loop](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/src/automation/browser.rs), [published configuration](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/src/automation/config.rs), [local audit](2026-10-07-crawlee-spider-contracts-followup.md).

## 4. Scrapling: a useful mechanism, a narrower contract than "self-healing"

The pinned parser `C` computes an average of resemblances: tag, text, attributes and their values, class/id/href/src, tag path, parent and siblings. It notably uses SequenceMatcher; this score **is not a calibrated probability**. `relocate` walks all elements, takes the maximum score, and returns all ties exceeding the threshold; the default is **40%**. `xpath`/`css` immediately return the results of a selector that works. Adaptation only starts on an empty selection. `auto_save` keeps the first element, and can also replace the fingerprint after relocation. [Pinned parser](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py).

The SQLite storage uses base domain + identifier and `INSERT OR REPLACE`; without a valid URL, the partition can become default. It has locking and WAL, but this key does not intrinsically isolate tenant, account or business route. Taking over the storage as is for a multi-tenant platform would therefore be insufficient. [Pinned storage](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/core/storage.py).

**Automan reuse `A`:** an economical structural candidate provider, limited to the relevant subtree, with raw score, list of ties and explanation of the features. Add identity invariants, a margin between first/second candidate, abstention and business validation before any effect. Never automatically promote the new fingerprint on resemblance alone. The threshold will be calibrated on a corpus held out from development `T`, not copied as a 40% confidence. This mechanism can repair an extraction without paying for an LLM; it does not on its own repair auth, navigation, pagination, mutation or workflow resumption.

## 5. License, cloud dependency and reuse choices

| Reference | License of the code consulted | Useful boundary | Proposed choice `A` |
| --- | --- | --- | --- |
| Stagehand | [MIT](https://github.com/browserbase/stagehand/blob/main/LICENSE) | Public SDK/runtime; Browserbase cache/gateway remain services | Versioned capability adapter; not a mandatory cloud dependency of the core |
| Browser Use / terminal | [MIT Python](https://github.com/browser-use/browser-use/blob/main/LICENSE), [MIT terminal](https://github.com/browser-use/terminal/blob/main/LICENSE) | Legacy Python, beta Rust, cloud V4 differ | Study protocol/runtime/providers and bridge; avoid taking over the whole terminal without a coupling audit |
| agent-browser | [Apache-2.0](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/LICENSE) | Rust daemon, MCP and native modules; out-of-process plugins | Adapter candidate and source of refs/frames/pinning mechanisms; encapsulate a version |
| Playwright, CLI/MCP | [Apache-2.0 MCP](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/LICENSE), [Apache-2.0 CLI](https://github.com/microsoft/playwright-cli/blob/main/LICENSE) | Public driver/tests/tools; separate hosting providers | Compatible worker and contract baseline; CLI/MCP as clients of the core, not two Automan engines |
| Skyvern | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern/blob/main/LICENSE) | Public backend; cloud anti-bot measures separate | Product/bench reference; AGPL reuse to be decided explicitly |
| Magnitude | [Apache-2.0](https://github.com/magnitudedev/browser-agent/blob/main/LICENSE) | TS agent/runner, accessible Playwright, BYOK model | Vision/checks adapter; keep probabilistic nature and costs |
| Imprint | [MIT](https://github.com/ashaychangwani/imprint/blob/main/LICENSE) | Compiler, API/DOM ladder, verifier and public tools | Study capture, network chain parameters and receipts; qualify beyond the examples |
| Scrapling | [BSD-3-Clause](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/LICENSE) | Public parser/fetchers/spider; structural scores without a model | Python adapter or similarity provider for the Rust contract; own storage |
| AgentQL | [MIT for the public repository](https://github.com/tinyfish-io/agentql/blob/main/LICENSE) | Examples/clients; inference API engine not shown to be open | Optional provider; do not confuse local browser and local inference |
| Spider agent | [MIT in published manifest](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/Cargo.toml) | Rust core, providers and compiled features | Library candidate; require proof that each option is consumed |

These licenses are properties of the sources consulted, not of complete services. Extracting a module must preserve the applicable notices and obligations. The choices are proposals, not the result of a complete dependency audit.

## 6. Automan contracts to build

The following types describe a target contract `A`, not a published API or an acquired implementation. The common schema must generate SDK models and CLI/MCP contracts; useful fields remain idiomatic per language.

| Object | Proposed minimal fields/guarantees | Why own it |
| --- | --- | --- |
| `CapabilitySet` | backend/version/protocol; locators, frames, shadow, network, scripts, profiles, cancellation and restrictions | Reject an incompatible plan before any effect; no silently simulated parity |
| `SessionLease` | sessionId, account/tenantRef, backend, browser/context/page/target, ownership, lease/version | Avoid global active pages, identity collisions and closures by another client |
| `Observation` | observationId, capturedAt, URL/origin, frame/document epochs, modalities present, artifacts/redactions, scope | Make cost, provenance and reference validity explicit |
| `TargetIntent` | role/name/context, business identifier, allowed strategy/alternatives | Keep the intent beyond the selector |
| `ResolvedTarget` | backend ref + document/frame; raw candidates/scores; decision source; guards and expiration | Distinguish resemblance, identity and ability to act |
| `ActionRequest` | actionId, sessionLease/version, target, arguments/secrets refs, budget, declared effect, preconditions | Every mutation goes through the same policy, agent or program |
| `ActionResult` | executed/failed/indeterminate, evidence, errorClass, attempts, cost; distinct effect status | Exception after click ≠ click not performed |
| `VerificationResult` | rule/version, observed value, expected, evidence refs, verdict/unknown; deterministic/probabilistic | Distinguish valid type, correct target, confirmed effect and truth of the data |
| `RunResult` | verified/partial/failed/needs_review/effect_unknown; outputs + provenance + budget/cost | Forbid business success inferred solely from the absence of an exception or an LLM done |

A source hash proves the identity of an observed artifact, not its truth. A run timestamp does not prove a fresh network request: keep observation/request time, source and the cache policy actually applied.

### Plan IR and assisted compilation

The versioned plan must carry inputs/output schemas, required capabilities, session/identity contract, budgets, policy, invariants and compilation provenance. Typed nodes: HTTP, Navigate, Observe, Resolve, Action, Extract, Assert, Wait, Branch, bounded Loop, Subplan, Checkpoint, ToolCall. A Code extension is isolated and declares its permissions/effects; it does not bypass the journal.

A plan is declared **deterministic** only if every allowed path can run without model resolution. An explicit model fallback makes it hybrid. The compiler does not copy an answer from the learning run into the output; it produces operations that reread the sources. Secrets become references injected at the last moment, not values recorded in the prompts or the plan.

Proposed pipeline: **intent/recording → budgeted exploration → candidate IR → static verification → fixtures → authorized live validation → immutable version → activation**. Exploration keeps the rejected alternatives and the useful source context; the activated recipe contains the explicit branches, the guards and their limits. Acceptance criteria must be established independently of the output desired by the same agent: business fixture, source response, invariants or developer checks. A separate verifier that rereads the same hallucinations is not independent in the sense of truth.

Effectful workflows use simulation/fixtures or an authorization covering their live validation. Compilation does not by itself create any additional authorization. This principle governs real mutations during learning as much as during replay.

### Validation and bounded repair

Repair must produce a **new candidate version**, with diff, reason, source and tests; it does not modify the last known-good plan in place. Pipeline: diagnose the failing property → classify the previous effect → observe → propose a local patch → verify preconditions/invariants → replay the admissible segment → compare outputs/effects → promote or abstain.

| Failure | Proposed admissible response | Response to exclude from the contract |
| --- | --- | --- |
| Target gone | New structural/AX/LLM candidate, guards, then action | First match with the maximum score without an identity check |
| Selector works, wrong entity | Reject via business guard; repair intent/resolution | Wait for an exception to repair |
| Data changed normally | Read the current value and verify schema/freshness | Treat every business change as a plan breakage |
| Incomplete extraction | Reread/source/rendering/scoping based on diagnostics, validation | Save the result as a success because the JSON is valid |
| Timeout after possible mutation | Verify the effect, then authorized resumption or effect_unknown | Blindly re-execute the step or switch backend to erase the uncertainty |
| Expired auth | Explicit session/auth transition, revalidation of identity | Implicit account rotation to continue |
| Incompatible backend | Another announced session/backend and supported transfer | Two drivers writing simultaneously to the same page |

Budgets must cover all layers: model cost, captures, transport, execution, repair and intervention. A read-only declaration by a tool/page is not sufficient proof of absence of effect. The user's policy can allow automatic activation on qualified reads; ambiguous cases remain needs_review/effect_unknown. No exactly-once promise on the effects on a third-party site without an idempotency/transaction mechanism provided by that site.

### Separate caches, explainable invalidation

| Proposed cache | Value | Required policy |
| --- | --- | --- |
| Plan/compilation | IR and versions/tests | Compatible inputs/output schema, policy and capabilities |
| Resolution | locator/fingerprint/target intent | session/account/route/document scope, guards, abstention; score ≠ oracle |
| Result | Extracted value | Explicit FreshnessPolicy, source/version, age, invalidation and visible HIT decision |
| HTTP/source | Response and transport validation | ETag/last-modified/cache rules; live contract on demand |
| Model/provider | Prefix/token cache | Read/write costs and model version; do not present as zero inference |
| Session/backend | Profile, state and winning access path | Tenant/account/origin, expiration, auth state and backend constraints |

The local plan cache is part of the proposed OSS product. Live results must reread a source; serving an old value requires an explicitly authorized policy. Stagehand shows why this separation matters: extract caches a value, whereas Skyvern mostly caches code and Magnitude prompt prefixes.

## 7. Native CDP, extension and Playwright: adapters rather than an ownership conflict

| Backend | Proposed best use | Guarantee to qualify `T` |
| --- | --- | --- |
| Rust HTTP | Accessible APIs/HTML, parsing/extraction, crawl | Headers/auth/pagination/freshness and data equivalence; rendering not needed |
| Native Rust CDP | Controlled portable engine, cloud worker close to Chrome, composed scripts/actions | Actionability, hit-testing, OOPIF, refs, navigation, dialogs/downloads, connection loss/cancel |
| Extension | User's browser and existing sessions, permissions and execution close to the page | Version compatibility, permissions/contexts, service worker lifecycle, lost connection and tab ownership |
| Playwright worker | Flows using its locators/waits/network; integrations and independent baseline | Serialization, forwarding/cancellation, costs; capabilities depending on browser and connection mode |

**One owner of session and lifecycle.** The runtime grants an execution lease; the chosen browser worker owns the connection, events and mutation of the browser. An agent, SDK or MCP server consumes this contract. Pages of the same account can be concurrent if the plan allows it, but dependent transitions remain ordered. A new client does not take over another's "active tab".

The [agent-browser pin-tab](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md) provides a concrete reference: closing a tab → tab_gone instead of falling back to the neighboring one. The [Playwright MCP](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/README.md) points out that a persistent profile cannot be opened by several instances in parallel. Automan must make these cross-client rules native, with owned/borrowed attach and a close that respects ownership. Resumption after a crash revalidates URL, document, auth and checkpoints; it does not restore DOM refs as if they were still alive.

The Crawlee/Spider frontier is separate from the step engine. An external framework can own its queue; Automan then executes the plan under the assigned request. Do not duplicate the same request under two durable schedulers. The [published Crawlee bridge](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json) remains Stagehand v3: v4 compatibility is work to be done/qualified, not acquired.

## 8. Agent-ready surface: same authority via SDK, CLI and MCP

Proposal `A`: a compact core profile — session, observe, resolve, execute, extract, plan.run — then discovery of compile/repair/network/debug/crawl profiles. The exact names and schemas remain to be stabilized. Pagination, out-of-context artifacts, snapshot depth/scope and partial reads prevent a permanent copy of the DOM in every response. Results keep IDs, state, cost, verification and artifact refs; the text serves as a summary, not as the sole contract.

agent-browser already has this kind of MCP profiles, typed fields, pagination and readOnly/openWorld annotations `D/C`. Its [pinned implementation](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/cli/src/mcp.rs) is a reusable reference for ergonomics; the Automan runtime must however call its canonical protocol rather than expose arrays of arbitrary commands as its only API. CLI extraArgs are not a stable permission model.

**MCP and WebMCP are distinct.** MCP exposes our tools to the agent client. WebMCP potentially exposes tools declared by a page: discovery, bounded catalog, schema/version/origin/frame, invocation and state/result/cancel. These declarations are page data, not authorizations. Security/read-only annotations must not override policy and identity. The [pinned agent-browser WebMCP](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md) already describes compact discovery and schema on demand; availability depends on the engine and remains experimental.

The [current Playwright MCP](https://playwright.dev/docs/getting-started-mcp) names its script tool browser_run_code_unsafe and describes its execution in the server as equivalent to arbitrary code execution. Automan must distinguish DOM evaluation from host code, declare the requested capability and isolate code/plugins with the granted permissions. The [agent-browser controls](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/docs/content/docs/security.mdx) also show limited scopes: opt-in features, domain filters incompatible with several attach/profile modes, out-of-process plugins not automatically sandboxed. A browser-level domain list is not a universal OS boundary.

## 9. Acceptance criteria and gaps to build

The following requirements are proposed, not acquired guarantees:

- **CAP-01 — announced parity:** the same recipe passes the contract tests on all backends declared compatible; an incapable backend refuses before any effect.
- **OBS-01 — validity:** refs invalidated after replacement of document/frame/element, context and origin preserved; bounded captures with explicit scope.
- **ID-01 — isolation:** two accounts/tenants share neither cookies, credentials, scoped plan cache nor tab mutation; attach/close respect ownership.
- **ACT-01 — effects:** timeout before/after effect differentiated; no uncovered automatic resumption; effect_unknown survives a crash.
- **PLAN-01 — compilation:** a qualified deterministic plan executes all its planned branches without an LLM; output values reread, never learned by copying.
- **VER-01 — result:** false success, detected failure and abstention measured separately; versioned validators and evidence available.
- **CACHE-01 — freshness:** a data change under the same URL/layout does not serve an old value when the policy requires live.
- **REPAIR-01 — promotion:** immutable patch, target/effect validation, nominal + regression corpus; ambiguity leads to abstention.
- **TOOL-01 — shared authority:** SDK/CLI/MCP follow the same budgets, cancellations and policies; a page/plugin cannot increase its permissions.

Essential corpus `T`: incorrect target still clickable, reordered rows, data change without layout change, re-render between wait/click, nested frames/OOPIF, open/closed shadow depending on backend, expired auth, closed tab, lost CDP and timeout after effect. Compare optimized program, free agent, compiled plan and repair, with first compilation and cold/warm reruns separated. Publish full costs, p50/p95, calls/tokens, human intervention, effects and correct outputs. A language advantage or a configuration option replaces none of these results.

**Reuse synthesis:** Stagehand contracts and ergonomics; Playwright locators/waits and baseline; agent-browser native refs/pinning/MCP; Browser Use runtime/protocol and loop; Skyvern block-level code; Magnitude vision/checks; Imprint API/DOM capture and verifier; Scrapling structural candidates; Spider crawl/adaptation and Rust agent. Automan's own work is articulating them under a portable IR, an exclusive identity, known effects and qualified proof of results. This combination remains to be implemented and demonstrated.
