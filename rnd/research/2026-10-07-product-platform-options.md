# Goodfetch: platforms, surfaces and product boundary

**State studied: October 7, 2026. Status: design proposals.** No application, cloud execution, performance measurement or security qualification was carried out for this note. The small-team and 3–6-month timeline constraint has been removed; it governs none of the choices below.

This note complements the [runtime ADR](../adr/0001-runtime-language.md), the [Crawlee/Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md) and the [Stagehand audit](2026-10-07-stagehand-alternatives-followup.md). It verifies platform surfaces against official sources and distinguishes their facts from our recommendations. Current documents are consulted as of this date; their links do not constitute a frozen version to install. Versions, MCP SDKs and browser capabilities will need to be pinned during implementation.

## 1. Proposed decisions

Goodfetch should expose **one canonical Rust runtime**, TypeScript/Python SDKs, a CLI, an MCP adapter, a web console and a native macOS app. A local execution must be useful without a subscription, a mandatory model or a connection to the Goodfetch cloud. The future SaaS brings hosted execution, fleet, sharing, governance and operations; it uses the same contracts and workers without claiming that its entire topology exists on a local computer.

**Explicit web choice: React + shadcn/ui + Vite for the operational console**, produced as a static bundle. The daemon can serve it locally; the SaaS serves the same product behind its API and its identity. Next.js is a relevant choice for the public site and for pages requiring server rendering, and becomes an alternative for the console if a concrete need for SSR or a BFF justifies that server. This SPA choice is contextual, not a general recommendation against React frameworks.

**Explicit macOS choice: SwiftUI with AppKit for specialized components**, connected to the common daemon. An app essentially made of a React WebView does not satisfy this decision. Reuse schemas, data and design tokens; genuinely implement navigation, windows, commands, lists, shortcuts and system integrations with the native APIs.

**Distribution choice: binary/daemon rather than FFI as the main interface of the SDKs and the app.** The Rust library remains available for embedding; an FFI binding requires a measured benefit or a precise deployment constraint. The private Goodfetch protocol and the standard MCP protocol are two distinct contracts.

## 2. What the runtime choice actually changes

The references demonstrate no universal consensus on Rust. Stagehand v4 places its runtime in a Chrome extension; Crawlee exists in TS and Python; Spider is Rust. Impit also shows that an engine exposed in TS/Python can delegate transport to native code. The client language, the location of state and the total cost of an execution are different decisions. [Stagehand](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md), [Crawlee Python](https://crawlee.dev/python/docs/guides/architecture-overview), [Impit](https://github.com/apify/impit), [Spider](https://docs.rs/crate/spider/2.53.9/source/).

| Architecture | Contract and topology advantage | Cost or limit to qualify | Goodfetch view |
| --- | --- | --- | --- |
| Canonical Rust + thin clients | One implementation of sessions, budgets, crawl and resumption; common local/cloud binary; control over the core's resources | Packaging per OS/architecture, IPC, inter-process async and maintenance of browser adapters | Proposed target, without promising a latency gain from the language alone |
| TS-first + API/clients | Direct integration with Playwright/Crawlee and web recipe code; same ecosystem for console and tools | Node and dependencies in the worker; non-TS clients also go through a contract; memory/CPU profiling needed | Credible alternative, and Playwright compatibility backend in the Rust target |
| Python-first + API/clients | Integration with data pipelines, agents and model libraries | Distribution of the environment; possible native extensions; same ownership and durability issues | Credible alternative; Python SDK proposed without a second standalone engine |
| Runtime in a Chrome extension | State and operations close to the page; independent clients; access to certain browser integrations | Lifecycle of the extension and the browser, permissions, distribution; HTTP acquisition and durable journal still have to be organized elsewhere | Possible adapter or recorder, not the sole authority of the whole platform |

An extension therefore has a useful place even with Rust: perception close to the DOM, recorder capture, binding to a user's browser. Its storage must not become the only source of the resumption journal. An extension or a TS worker that owns the session must be its only controller while it is in use; not two concurrent CDP loops. The Rust driver, the Playwright worker and the extension advertise their real capabilities behind the public contract. The Chromey/Chromiumoxide choice remains a separate qualification.

The gain from placement, batching or avoiding a render is measured at equivalent quality, with Chromium, IPC, model and all attempts included in the cost. A Rust CLI paired with a browser and a Node worker is not a single GC-free process.

## 3. Frontend: SPA chosen, Next.js possible under conditions

React recommends starting new apps with a framework; its documentation also allows building with Vite when constraints justify the choice, while recalling the routing and data responsibilities that follow from it. Our specific constraint is a console connected to a local or remote executor, with no SEO need for authenticated runs. [Creating a React app](https://react.dev/learn/creating-a-react-app), [building from scratch](https://react.dev/learn/build-a-react-app-from-scratch).

Vite produces a bundle deployable on static hosting. The execution engine can therefore serve this bundle without adding Node in local production. The router, the query client, and the handling of subscriptions, errors and identity must be chosen explicitly; Vite does not provide these contracts. [Vite build](https://vite.dev/guide/build).

Next.js can also do a static export. In this mode, features that require a per-request server are not available; server code executed at build time does not become a dynamic local API. Cookies, Server Actions and authentication flows requiring its server must not be used as if the same exported bundle had them. A Next deployment with a server can provide SSR/BFF, with an additional process and authentication boundary to operate. [Next.js static export](https://nextjs.org/docs/app/guides/static-exports), [deployment](https://nextjs.org/docs/app/getting-started/deploying).

shadcn/ui distributes component code that the application owns and adapts; it offers Vite and Next installations. This choice helps visual consistency without, on its own, taking charge of the screens, the accessibility of their composition or the performance of large tables. [Overview](https://ui.shadcn.com/docs), [Vite](https://ui.shadcn.com/docs/installation/vite), [Next](https://ui.shadcn.com/docs/installation/next).

The console must present product operations: projects, recipes, crawl frontier, runs, data, sessions, evidence, costs and connections. It clearly distinguishes verified result, partial result, intervention request and uncertain effect. "Cancellation requested" remains visible until the runtime acknowledges it; an interface disconnection does not automatically become a cancelled run. Data has its own freshness and provenance, even when the plan comes from the cache. Technical driver details belong in diagnostics, not in the nominal flow.

**SPA choice gate:** carry out the user flows on the local daemon and the cloud API with the same build, deep routing, connection/reconnection and pagination of large data. Re-evaluate Next if SSR, organizational integration or an indispensable BFF changes this result. Measure a real navigation and a real list; neither React nor Next proves the console's fluidity.

## 4. macOS: a true native app and a shared runtime

SwiftUI describes views, controls and scenes; Apple provides for AppKit integration. `NSViewRepresentable` makes it possible to integrate an AppKit view, with its lifecycle and coordination, into SwiftUI. Our app can use SwiftUI for structure and AppKit for complex lists, editing, commands or window behaviors that require it. [SwiftUI](https://developer.apple.com/documentation/swiftui), [NSViewRepresentable](https://developer.apple.com/documentation/swiftui/nsviewrepresentable).

The daemon manages journal, browser and runs. The app is a client with file selection, menus, shortcuts, windows and access to secrets through a native integration. Keychain is the system API for encrypted storage of small secrets; the access rules between app, helper and CLI must be qualified instead of assuming that a signature automatically gives them shared access. [Keychain Services](https://developer.apple.com/documentation/security/keychain-services).

Two lifecycles are proposed: launch the runtime on demand, or enable a per-user background service for local runs and schedules. `SMAppService`, available from macOS 13, manages helpers included in the bundle; their registration and launch remain subject to the applicable system approval. A user LaunchAgent is the normal target; no need for root privileges has been established. Define the minimum macOS after an inventory of the APIs, with macOS 13 as a coherent candidate for this integration. [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice).

The proposed main channel is per-user local IPC, with socket permissions and peer verification, plus a loopback HTTP adapter for the console. The CLI and SDKs can launch a subprocess over stdio or connect to the daemon according to an explicit option. A single daemon writes a workspace's journal; concurrent startup and version discovery must be resolved without races. Quitting the UI, putting the machine to sleep and dropping the client connection have separately documented effects.

FFI avoids serialization but transfers to the binding the ABI, memory ownership, async calls, cancellation, callbacks and error sharing. A crash of the embedded code can affect its host. The separate process provides a lifecycle and diagnostic boundary; it does not by itself constitute a security sandbox. Apple specifies that a `Process` child inherits the sandbox of the sandboxed app and recommends XPC when a helper needs different entitlements. An XPC broker can therefore be useful for a sandboxed distribution, without rewriting the Rust engine. [Process](https://developer.apple.com/documentation/foundation/process).

Proposed direct distribution: Developer ID–signed bundle, Hardened Runtime, notarization and ticket stapled to the relevant deliverables. Apple notarization is an automated check of the code and the signature, not a product validation comparable to App Review. Qualify the Rust helper, any Node worker, the managed browser and the signed update together. Mac App Store distribution is a separate decision, requiring real qualification of the sandbox/entitlements and of browser launching. [Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## 5. SDK, CLI, API and MCP contracts

The SDKs are thin on execution but idiomatic: types, builders, async helpers, streaming and errors specific to each language. They do not recreate the routing algorithm, the retries or the success semantics. A user callback remains code executed by its host; it does not automatically migrate to the cloud. A model provider goes through a provider reference on the worker side or an adapter explicitly available on the client side, with timeouts and limits.

| Surface | Main contract | Property it must not claim |
| --- | --- | --- |
| Rust library | Public internal module API, version qualification | Isolation of arbitrary code by the language alone |
| TS/Python SDK, CLI | Versioned private Goodfetch protocol; stdio or connection to the daemon/service | Ownership of a second engine or autonomous resumption of a session |
| Console/macOS app | Authorized commands, snapshots and events | Source of authority for terminal states and costs |
| Cloud API | Jobs, sessions, recipes, datasets, events and artifacts | Authentication via a `tenantId` freely supplied by the client |
| MCP adapter | Tools and resources, according to a qualified MCP version | Implicit durability, business identity or private Goodfetch protocol |

The private protocol can adopt the ADR's proposal of bidirectional JSON-RPC over stdio/secure WebSocket, with generated types, structured errors, budgets, message limits and explicit capabilities. Large artifacts are transferred by references and bounded streams. The public API can offer HTTP commands and SSE events with an Goodfetch cursor; no established need requires replacing all these surfaces with WebSocket.

**Beware of the MCP version:** `latest` currently points to revision **2026-07-28**, whose standard transports are stdio and Streamable HTTP. It carries version/capabilities per request. Current MCP must not be credited with a protocol initialization/session or with independent JSON-RPC requests sent by the server as in earlier revisions. The private bidirectional Goodfetch contract remains possible. [MCP transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).

Streamable HTTP 2026 removed the stream GET and protocol sessions; sampling/elicitation interactions go through requests embedded in results. The spec requires Origin validation and recommends localhost binding/auth locally. Its SSE does not support `Last-Event-ID`; our journal cursor is therefore not a guarantee offered by its transport. Qualify the versions actually supported by the targeted clients and their SDK; a compatibility adapter may be necessary. [Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http).

In the cloud, Goodfetch auth is mandatory in the proposal even though authorization is optional in the MCP standard. For a protected HTTP MCP endpoint, apply the MCP/OAuth requirements: metadata discovery, tokens intended for that resource and audience validation; do not recycle a bearer token from another provider. Over stdio, the spec suggests retrieving credentials from the environment rather than applying this HTTP flow. [MCP authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

A long call creates a persistent `RunId` and returns a receipt after its acceptance has been persisted. Observation, resumption and cancellation are done on this run. Cancelling an MCP request or closing a stream cancels the work of that request according to the protocol version; the mapping to the cancellation of a detached job must be explicitly defined by the tool. A lost response to `run.create` is resolved with an idempotency key and a read of the receipt, without silently creating a second workflow.

## 6. Local/cloud: parity of results, operational differences

Parity targets a versioned recipe, its requirements, results, errors, verifications and policies. It does not mean copying cookies, access to local files, identical IP/proxy, unlimited capacity or portability of arbitrary functions. Each environment advertises its capabilities. A project export is inspectable, with recipe, schemas and secret references; secret values do not travel implicitly.

Local mode keeps journal, checkpoints and datasets in embedded adapters, SQLite being the proposal for the journal. The cloud has a control plane for identity, scheduler, admission, quotas, secrets and billing; workers execute the same plans close to the browsers. PostgreSQL and object storage are proposed adapters. A common logical schema and transition tests are needed, not an abstraction that pretends SQLite and a distributed system have the same transactions.

The cloud authorizes a command by principal, organization and workspace derived from a validated identity. Capabilities distinguish data reading, execution, recipe publishing, access to connections and external actions. Secrets are references resolved just in time, with scope and audit. Page content, downloaded files and model responses remain untrusted data; they cannot extend the run's authorizations.

Isolation must cover browser accounts, cookies, data caches, keys, proxy, temporary storage and artifacts. A browser context is a unit of state; it is not complete isolation of untrusted client code. Trusted recipes can share a worker according to policy. For arbitrary third-party code/plugins, choose a container or VM/sandbox with resource, network and filesystem restrictions; qualify the guarantees actually obtained. Access to the internal network and to local files must be an explicit capability for the deployments that need it.

A useful target is three compatible modes: local computer, self-hosted service, Goodfetch SaaS. A remote browser adapter, for example Browserbase, is an infrastructure choice; it does not force sending data or models to its gateway. The BYOK, hosted provider and BYOC modes are described separately, with data location, operational responsibility and cost.

## 7. Durability: define the guarantees before the screens

Separate the crawl frontier from the journal of workflow steps. A collected page can be rescheduled according to the freshness policy; an action "send an order" must not be replayed on the same principle. The queue owner assigns a lease/epoch; an old worker cannot acknowledge a new attempt. The journal assigns an identifier to each attempt and effect, and retains acceptance, start, observations, result and validation.

A local checkpoint write must have a defined and tested transactional boundary. The cloud must define the atomic database writes, the event outbox, artifact publication and their reconciliation. An external operation does not become exactly-once thanks to the journal. After a crash between external effect and acknowledgment, the run is `effect_unknown` until reconciliation or intervention; provider idempotency or a business key may allow a safe retry when their contract guarantees it.

Final result, evidence and billing are linked to the same attempt. An export failure does not fictitiously cancel an already verified effect; a validation failure does not erase the costs incurred. Caches separate perception, plan and data. A plan can be reusable while its result is stale. Tenant/session caches are shared only within an explicitly authorized scope.

The UI displays the authoritative snapshot and catches up on events with cursor, deduplication and pagination. A slow client does not block the worker and does not accumulate unbounded memory. On daemon restart, Mac wake-up or cloud reconnect, the interfaces first re-read the durable state. Define the rules for local schedules missed during sleep rather than promising permanent availability of the computer.

## 8. What the studied services actually show

**Apify** distinguishes the Actor, a cloud program with structured inputs, Docker configuration and schemas, from the operation of the service and its distribution in the Store. This deployable unit inspires a versioned recipe package; an Actor adapter could also distribute an Goodfetch worker. Consumption includes compute, storage, transfers and proxies. A run price displayed according to the current plan is not a sufficient historical ledger for our billing. [Actors](https://docs.apify.com/actors), [usage and resources](https://docs.apify.com/actors/running/usage-and-resources).

**Browserbase** provides browsers, APIs and managed services. Functions hosts TypeScript code close to the browser, creates one session per invocation and retains results/logs; this confirms the value of placement, without proving that the Goodfetch Rust runtime is accepted as is in this service. The vendor figure "less than 5 ms" is not a general measurement of our workflows. Stagehand OSS and the managed fleet are different scopes. [Overview](https://docs.browserbase.com/introduction), [Functions](https://docs.browserbase.com/platform/functions/overview), [Stagehand license](https://github.com/browserbase/stagehand/blob/main/LICENSE).

**Spider** presents its OSS crawler as a reusable core, with a service that adds fleet operations and infrastructure capabilities. Its marketing page also places certain AI functions on the cloud side, whereas our current reading does find `spider_agent` OSS: the module/version must be specified, and this marketing boundary must not be generalized into "no local agent". Parity of the custom cloud browser is not demonstrated by the presence of Chromey OSS. [OSS vs cloud](https://spider.cloud/open-source-vs-cloud/), [audit of the published code](2026-10-07-crawlee-spider-contracts-followup.md).

**Firecrawl** documents the collection and processing APIs in the self-hosted stack; LLM formats require a configured provider. Agent/Browser, dashboard and enterprise controls belong to its cloud offerings depending on availability. Self-hosting transfers auth, TLS, persistence, upgrades and recovery to the operator; its quickstart does not establish a secure and durable production. The main repository is AGPL-3.0, unlike the permissive references: do not turn a functional inspiration into code reuse without an inventory of components/licenses. [OSS or cloud](https://docs.firecrawl.dev/contributing/open-source-or-cloud), [self-host](https://docs.firecrawl.dev/contributing/self-host), [license](https://github.com/firecrawl/firecrawl/blob/main/LICENSE).

These examples show possible boundaries between accessible code and operated service; they prove neither solvent demand for Goodfetch, nor a margin, nor an acquired functional equivalence. Public limits may change; contract and tested capability take precedence over a product name.

## 9. Proposed monetization and cost contract

The OSS core can remain complete for executing, collecting, validating, resuming and exporting locally. Propose an explicit permissive license for the project's own code, subject to an inventory of its dependencies. SDKs, protocols and formats remain documented. The commercial difference lies in the resources and services provided: hosting, isolated fleet, available schedulers, integrated proxies, managed storage, collaboration, SSO, audit, support and recipe development/maintenance.

Custom integrations and their maintenance are a service offering distinct from the SaaS. The recipe Store is a distribution option to validate, not a revenue assumption. Features that guarantee the correctness of a local run should not depend on a subscription to behave correctly.

Before pricing, define append-only usage accounting: run/attempt, consumed line item, unit, quantity, provider, BYOK/managed mode, time, region and tariff version. Separate estimate, observed consumption, infrastructure cost and billable amount. A repair, a failed page and cancelled model calls can consume resources; their billability must be transparent in the commercial contract. Any credits have a versioned conversion, not a magic technical unit.

The budget is an admission and execution policy: cap, reservation, committed spend, stop threshold and reconciliation of delayed provider data. No exact cap should be promised if an already committed call can exceed it. Test duplicate billing, lost events, late provider, tariff change and refund; the business ledger cannot be reduced to the metrics chart. The price per verified result remains a product metric with visible coverage/failures, not an excuse to omit the costs of attempts.

## 10. Open decisions and verifiable gates

| Decision or risk | Evidence required before making it a promise |
| --- | --- |
| Main native driver and capabilities | Same CDP/Playwright corpus: waits, navigation, OOPIF/frames, downloads, auth, connection loss and cancellation; equivalent quality before speed gain |
| Rust/SDK/local/cloud parity | Contract tests for states, errors, budgets, ordering/idempotency and version compatibility; no divergence hidden by generated types |
| Current MCP and existing clients | Pinned protocol/SDK versions; real targeted clients; transports, auth, input-required, notifications and cancellation tested per version |
| Local process and secrets | Two concurrent clients; discovery/upgrade; IPC permissions; hostile web page against the local endpoint; Keychain/helper/CLI and absence of secrets in traces |
| macOS app and packaging | Clean Mac, signing/notarization, helper/browser, no UI blocking, background permission, stop/restart/sleep, update and recovery; explicit target architectures |
| Durability and external effects | Crash injection before/after effect, validation, commit and acknowledgment; expired leases; proof of no blind replay and visibility of `effect_unknown` |
| SaaS isolation | Cross tenant/session/cache/artifact/secrets tests; third-party code and hostile network; memory/CPU/disk limits; sandbox model published per offering |
| Console and app at large volumes | Pagination, export, backpressure, bounded consumption, reconnection with cursor; keyboard/screen reader and real product tasks |
| Business model | Reconciled usage/cost and explainable invoice; tolerance to duplicates; interviews/paid offers validating value without extrapolating competitor benchmarks |
| OSS/cloud boundary and license | Public features/responsibilities matrix; reproducible self-hosted installation; pinned components and licenses; project export contract |

The gates organize a common ambition, without an artificial timeline. Any unvalidated commercial or technical gain remains an identified hypothesis.
