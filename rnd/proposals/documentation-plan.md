# Complete documentation to produce with Automan

**Requirement and work plan — October 7, 2026.** This document describes the product's future documentation. It is not user documentation for Automan, whose implementation does not yet exist. The API examples in the dossier are proposals.

## 1. Expected level

Plan documentation comparable in coverage to [Stagehand](https://docs.stagehand.dev/v4/first-steps/introduction), [Crawlee](https://crawlee.dev/js/docs/introduction) and [Spider](https://docs.rs/spider/latest/spider/), with a clear distinction between getting started, concepts, guides, references and operations. A collection of READMEs and happy-path examples is not enough to cover queues, workers, pools, recovery, AI and service.

The web and macOS interfaces have their guides. Developers have a complete reference per SDK and a shared contract; agent integrators have documented CLI/MCP and examples of external orchestration. Operators have the limits and the diagnosis/recovery procedures.

**Proposed delivery rule: each implemented and qualified capability is accompanied by its verified docs and examples.** Future capabilities remain in the development plan/version, without appearing as available in the stable documentation.

## 2. Target structure

```text
Discover
  Why Automan, use cases, architecture, capabilities and limits
  Choose deterministic, assisted, agent or compiled plan
  Open source framework, self-hosting and hosted service

Get started
  Install and verify the environment
  First workflow from the web
  First TypeScript / Python extraction
  First HTTP crawl, then browser
  CLI and first MCP integration
  Install/use the native macOS app
  Run locally, then in the cloud

Concepts
  Workflow / plan / version / run / attempt
  Frontier / request / deduplication / discovery / recrawl
  Worker / process / async task / browser / context / page
  Session / identity / proxy / capabilities / ownership
  Validation / completeness / freshness / unknown effect
  Lease / recovery / idempotence / journal / outbox
  Admission / concurrency / budgets / backpressure / autoscaling
  Perception / plan / data caches

Collection guides
  HTTP, TLS, headers, cookies, auth, redirects and downloads
  CSS / XPath / JSON / JSON-LD / Markdown / tables / cleaning
  Seeds, routers, labels, links, XHR/JS and sitemaps
  Scope, robots, depth, budgets and canonicalization
  Pagination, infinite scroll, virtualization and completeness
  Adaptive HTTP and equivalence qualification
  Streaming, large documents, formats and normalization
  Datasets, append/upsert, exports, provenance and changes

Browser guides
  Local and remote launch/connect, backends and versions
  Locators, waits, actionability and invalidated references
  Pages/popups, frames/OOPIF, shadow DOM and navigation
  Inputs, forms, upload/download, dialogs, screenshot/PDF
  Network, interception, events and trusted code
  Authentication, identities, storage and handoff
  Sessions, proxies/fingerprints and access restrictions

AI and agent guides
  Providers, local/remote models and BYOK
  Observe/act/extract and typed decisions
  DOM/AX, vision, projections and observation control
  Integrated agent and external orchestration
  Tools and permissions, memory, limits and costs
  Recorder, parameters and plan compilation
  Replay, validation and freshness
  Drift detection, repair, tests, canary and rollback

Operate
  Persistent queues, priorities, leases, delays and dead-letter
  Retries, backoff, timeouts, unknown effect and reconciliation
  HTTP pools, browsers, contexts/pages, sessions, proxies, models
  Workers, heartbeat, capacities, regions, draining and updates
  Concurrency, autoscale controllers and causes of limitation
  Storage, backup/restore, retention and migration
  Scheduling, timezone, overlap, webhooks and outbox
  Logs, traces, metrics, cost and redaction
  Deploy locally, container, self-hosting and cloud fleet

References
  Public TypeScript / Python / Rust
  CLI, options, stdout/stderr, NDJSON and exit codes
  MCP, transports/revisions, tools/profiles/resources and auth
  REST API, events/SSE/cursors and private protocol when public
  Workflow/plan/package/configuration schemas
  Errors, statuses, capabilities, limits and effective defaults
  Backends / browsers / OS / versions matrix

Interfaces and service
  Console, creation, runs, data, repair and operations
  macOS: lifecycle, files, credentials and background work
  Cloud: organizations, roles, secrets, fleet, quotas and usage
  Billing: units, failures, retries/repairs and estimates
  Packages, registry, versions and provenance

Examples and integrations
  Catalog, authenticated export, changes and RAG content
  Plans without AI, assisted creation then replay, auto repair
  TypeScript/Python, cloud SDK and callbacks/packages
  Agents via MCP/CLI/SDK, connectors and sinks
  Crawlee/Playwright integrations and queue/session boundaries

Maintain and contribute
  Contribution, internal architecture and plugin/backend contract
  Fixtures, reproducible benchmarks and qualification method
  Changelog, versions, migrations, deprecations and support
  Diagnostics, known issues and published limitations
```

This navigation is a target to be refined after the first usability tests. Completeness is tracked by the capability matrix, not by the number of pages.

## 3. Contract for a reference page

For each method, command, configuration and tool: purpose, signature/schema, required and optional parameters, units, **effective** defaults, result, errors, permissions, capabilities and versions. Include retry/timeout/cancellation, possible effects, resources engaged, local/cloud behavior and cache/freshness when they apply.

Show at least one minimal example and one relevant error case; a pool configuration example must specify the resource counted. Indicate what depends on the backend/provider rather than promising general parity.

Generated references do not replace explanations of the guarantees. For example, a `maxConcurrency` type does not explain whether the limit counts tasks, tabs or HTTP requests; the concepts and guides carry this distinction.

## 4. Contract for a guide

Each guide starts from a concrete objective, specifies the prerequisites, chooses a reference workflow and ends with an observable result and a diagnosis of possible failures. The reader sees what they can verify, what data is persisted and what costs/effects may be incurred.

Deterministic guides work without a model key. Agentic guides distinguish a test provider from real execution, display budgets and provenance, and describe `partial`, `needs_review` or `effect_unknown` stops. Repair guides also show a rejected candidate, not only a success.

## 5. Source, generation and versions

- Write guides/concepts in the repository and revise them along with behavior changes.
- Generate references from public types/schemas and CLI/MCP metadata; verify the SDKs' idiomatic facades.
- Publish stable, supported versions and development, with a version banner and a compatibility matrix.
- Link changelog and migration to each breaking change; keep documented URLs or provide redirects.
- Document the default values of the distributed version, not only those of a `main` branch or a marketing page.
- Link each capability to its pages, examples and tests. A contract change checks these consumers.

The docs site may use a maintained generator with search, navigation, TypeScript/Python/Rust/CLI blocks and static export; its selection will be made once the first content and publication constraints exist. Do not choose a second engine execution platform for the needs of the documentation site.

## 6. Verification and delivery criteria

| Requirement | Expected evidence |
| --- | --- |
| Coverage | Each delivered capability linked to the relevant concept/guide/reference; no undocumented public parameter or error |
| Accuracy | Generated references verified against the distributed schemas and defaults |
| Examples | Quickstarts and deterministic examples run on controlled fixtures in CI |
| AI | Contracts/budgets tested with a simulated provider; real examples qualified separately with identified model/version/cost |
| CLI/MCP | Quickstart commands run; JSON output/errors/tool schemas and protocol revision verified |
| Recovery | Crash/lease/session/unknown effect guides linked to behavior tests, not to slogans |
| Web/macOS | Flows and screenshots updated for the delivered version; failure states covered |
| Compatibility | Supported backends/versions/OS distinguished from experimental ones and from unavailable capabilities |
| Publication | Site build, essential local/external links and navigation/search checked |
| Migration | Before/after example and data recovery when the change requires it |

The documentation does not guarantee speed or universal success. Published benchmarks provide parameters, baselines, fixtures, quality, costs, unfavorable results and limits.

## 7. Work to integrate into the increments

**From the first increment**: introduction, installation, initial concepts, HTTP/web quickstart, run states, minimal API reference and errors. Future pages are in the backlog, not announced as stable.

**At each increment**: guides for the delivered capability, reference/errors/capabilities, reproducible example, migration if necessary and troubleshooting. Queues/pools/autoscaling and agents/repair have their explicit documentation increments, with the same requirement as the feature.

**Before the first framework release**: local SDK/CLI/MCP flows, supported self-hosting, licensing/components, contribution, compatibility matrix, benchmark protocol and consistent links.

**Before the service and the macOS app**: installation/lifecycle/credentials and local-cloud docs for macOS; organizations, schedules, quotas, retention, usage/billing and limits for the cloud. Launching a surface includes its documentation.
