# Product design and interfaces

**Proposal of October 7, 2026.** The React/shadcn console, the macOS app and the flows described remain to be implemented. The [HTML mockup](design/automan-console.html) is a standalone design artifact with simulated data; it is neither a React app nor an integration of the engine.

## 1. Central object: a result and its history

The interface must answer three questions: what do I want to obtain, did the run obtain it, and what should I do if the result is missing or uncertain? The run brings together timeline, data and evidence. Infrastructure metrics explain the result; they do not replace its verdict.

Creation shows URL/objective, input/output and verification criteria. Advanced settings — pools, proxy, budgets, hooks and backends — appear when the user asks for them. The ordinary flow does not require understanding CDP, Tokio or leases.

### Roles served

| User | Main flow | Advanced control |
| --- | --- | --- |
| Developer | Create, test, export plan/SDK, inspect a failure | Code, schemas, capabilities, errors and traces |
| Analyst | Collect, verify, filter, export and track changes | Coverage, freshness and provenance |
| Agentic integrator | Find tools, permissions and schemas; plug in CLI/MCP | Budgets, refs/observations and paginated results |
| Operator | Identify blockage, capacity, health and recovery | Queues/pools/workers, limits, drains and costs |
| macOS user | Run locally, manage identities/files and background work | Same engine, same plans, daemon control |

## 2. Console navigation and structure

Main navigation: **Workflows, Runs, Data, Repairs, Infrastructure**. Identities, integrations, schedules and workspace preferences are accessible from their context and from the configuration. A visible selector chooses local/cloud and workspace; switching does not implicitly import an authenticated session.

The compact mockup groups data into the run detail and shows four main entries; the full product adds the dataset library and the necessary configuration pages.

```text
Environment/workspace selector             Search / command / user
────────────────────────────────────────────────────────────────────────────
Navigation    Filterable list        Detail of the selected object
Workflows     Name / version         Objective, inputs, output contract
Runs          State / date           Timeline | Data | Evidence | Usage
Data          Dataset / export       Projection, coverage, freshness, export
Repairs       Cause / candidate      Diff, tests, result, promotion/rollback
Infrastructure                     Queues | Workers | Pools | Limits
```

On opening, select a useful workflow or recent run; an empty installation offers "Create a workflow" and a deterministic example. Do not invent a dashboard filled with statistics when nothing has been run yet.

## 3. Creation flows

### 3.1 Deterministic collection

1. Choose "Collect data", enter URL/seeds and define the fields.
2. Choose rules or AI assistance to build the recipe; display provider/budget if assisted.
3. Preview a result on a bounded sample, then define useful identity and completeness.
4. Save a version and start a run; offer SDK/CLI for the same configuration.
5. Show data, coverage and status; export or set a frequency.

Creation assistance can produce a plan that replays without a model. The test displays the calls actually needed; the user does not assume that a JSON schema proves the values.

### 3.2 Automating a flow

Choose "Automate a flow", select identity, parameterized input and expected result. The user can write steps, record a demonstration or ask the AI for a plan. Steps and conditions are visible before qualification/publication. Entering a secret creates an identity reference, without inserting it into the exported plan.

A step requiring authentication offers a handoff to the corresponding session. The interface shows the targeted account/domain and the data needed; authentication expiry does not trigger a locator repair by default.

### 3.3 One-off agent

Enter objective, parameters, permitted domains/tools and budget. A progress view exposes decisions, tools and costs; screenshots are available in the detail. The agent's message remains distinct from the final verdict. The user can create a reusable plan candidate after a qualified run.

## 4. Run detail

### Timeline

Each step shows an understandable operation, duration, result, attempts and evidence. Interruptions are placed where they occur: timeout, model quota, missing data, wrong identity, ambiguous locator. A filter enables technical details; the main view explains the consequence.

State is displayed as text, not only as color. `succeeded` is rendered as "Verified result" if all required checks pass; a successful technical click does not use this label. `partial` specifies the missing coverage. `effect_unknown` displays the action possibly performed and the expected resolution.

### Data and evidence

Virtualized/paginated table, schema, filters, projection, per-row/per-field provenance when available, pagination/coverage and freshness. Large artifacts are loaded on demand. Export starts an inspectable object with progress and possible refusal.

A piece of evidence indicates its nature: external state, checksum, identity, structure test or model judgment. Sensitive information is redacted according to permissions; "missing screenshot" is explained, not replaced by a success placeholder.

### Usage

Display consumed/reserved/cap, calls and tokens, browser, traffic/proxy and storage. Estimated cost and applied price are distinguished; local without a known price grid shows the units. Retries and repairs remain in the run's cost and in its breakdown.

### Commands

Active run: stop, suspend admission according to contract, provide the required authentication/decision. Terminal run: inspect, propose a repair, reconcile an effect, then launch a linked continuation. A continuation creates a new run, with revalidated checkpoints and preserved history.

## 5. Repairs

A repair brings together the original failure, base version, diagnosis, candidate patch, tests, canary, cost and decision. The before/after comparison shows the modified locator/mapping/wait and the validation result; it does not merely show a rewritten AI instruction.

States: diagnosis, candidate, tests running, rejected, qualified, promoted, rolled back. Required business criteria remain visible and cannot be lowered by an automatic patch. "Promote" is available only if policy and tests allow it. "Reject" keeps the reason and the history.

A read repair can be automatic according to policy; the detail indicates why. An action with an unknown effect requires reconciliation before repetition. Rollback indicates its scope: plan version, not erasure of external effects.

## 6. Infrastructure and advanced settings

**Queues**: ready/running/delayed/dead-letter, producers, age, priority, scope, retries and delay reasons. Inspect/requeue a request according to permissions, export a checkpoint, see the crawl end condition.

**Workers**: version/capabilities/region, available capacity, CPU/memory of their process group, activity and heartbeat. Register, drain, update, stop according to actual state. The stop choice shows the runs that can be resumed and the effects to reconcile.

**Pools**: each resource has its own view and units. Chromium processes ≠ contexts ≠ pages ≠ sessions. See state, warmup, health, retired/quarantined, cap and reason for unavailability. Hooks and overrides are visible in the effective configuration.

**Concurrency/autoscale**: desired, admitted, active and limited; domain, provider, memory, capacity or sink responsible. Fixed mode makes it possible to reproduce a benchmark. Advanced settings are grouped and exportable; they do not clutter the first form.

## 7. Visual direction

### Proposed tokens

| Token | Value | Usage |
| --- | --- | --- |
| Surface | `#F7F9FC` | Work background |
| Panel | `#FFFFFF` | Detail and data |
| Text | `#172B4D` | Titles/main content |
| Action | `#2453C6` | Selection, focus and primary action |
| Verified | `#087F72` | Validated verdict accompanied by text |
| Attention | `#9A5B00` | Decision required/partial, with verified contrast |

System sans-serif typography, hierarchy by size/weight and left alignment; monospace for code only. Spacing 4/8/12/16/24/32, borders to separate objects/timeline, moderate rounding depending on the component. Actual palette and contrast to be checked in the implementation, not inferred from the choice of color names.

The product's character comes from the **timeline linked to evidence**, not from a grid of decorative cards. Table/list and inspector share the space; confirmations explain the change. An animation accompanies an action or progress, respects reduced motion and does not divert attention.

### shadcn and behavior

Candidate components: navigation/sidebar, resizable panels, data table, tabs, dialog/sheet, command palette, forms, select, progress, tooltip and notifications. They are composed around the product contracts; a base component does not prove the accessibility of the final application. [shadcn/ui](https://ui.shadcn.com/docs), [React](https://react.dev/learn), [console options](../research/2026-10-07-product-platform-options.md).

Review of the design stance: keep states and actions comparable across lists, timeline and repair; remove statistics and accents that do not help decide. Tokens do not replace checking the mockup at different sizes.

## 8. A truly native macOS app

SwiftUI for window, sidebar, lists, inspector and commands; AppKit when needed for files, shortcuts, window integration and advanced content. No wrapped React web screen alone constitutes the expected result.

App and daemon share plans/states via a local contract; closing the window is not equivalent to stopping the engine. The user chooses whether runs continue, sees background activity and can stop/drain. Sleep/wake triggers health/reconnect and reconciliation if necessary; the CDP socket is not assumed to have survived.

Mac-specific features: Keychain, file picker and destination permissions, notifications for run/decision, shortcuts, command menu, management of local identities and a signed/notarized engine. A permanent launch agent is activated by explicit choice and through the system mechanism. The cloud uses the same business views, with its context visible. [Apple sources and constraints](../research/2026-10-07-product-platform-options.md).

## 9. UI states and qualification

Each screen covers empty, loading, success, partial, network failure, lack of permission, quota and stale data. SSE loss reconnects with a cursor or announces a history gap; it does not turn a server run into a local failure. Actions use idempotent identifiers and readable waiting states.

Tests: full keyboard navigation, visible focus and focus restoration after a dialog, screen reader on statuses/tables, labels/fields/errors, responsive layout and contrast; long names/URLs/values, large datasets and low bandwidth. The design screenshots validate the presentation; they do not prove the engine's behavior.

The mockup serves to examine navigation and product decisions. The simulated interactions will be replaced by the real contracts of the first vertical slice; no feature is qualified solely on the basis of its appearance.
