# Automan

Automan is an open-source framework for collecting web data, automating browsers and running AI-assisted workflows. Automan Cloud runs the same engine as a hosted service.

## Product

**Automan**:
The open-source framework: the engine and every surface that drives it, such as the SDKs, the CLI, the MCP server and the agent skills.
_Avoid_: engine (for the whole framework), library, platform

**Engine**:
The runtime at the core of Automan that executes every run, whichever surface starts it.
_Avoid_: framework, core, backend

**Automan Cloud**:
The hosted service that runs the engine for customers on Automan's infrastructure.
_Avoid_: platform, Automan (for the hosted service)

**Console**:
The web interface for operating Automan, served locally by the engine or by Automan Cloud.
_Avoid_: dashboard, inspector, app

**Client**:
Any program that drives the engine through its operations: an SDK, the CLI, the MCP server, the Console or Automan Cloud. Speaking CDP to a browser does not make a program a Client.
_Avoid_: frontend, binding, consumer

**Attached engine**:
An engine started by one Client for its own use, which stops when that Client goes away.
_Avoid_: sidecar, child engine

**Daemon**:
A long-lived engine on the local machine that several Clients share and that outlives each of them.
_Avoid_: server, service, background engine

**Private component**:
A closed component kept outside the open-source framework, used by Automan Cloud and licensed under contract to vetted customers, that holds a capability whose public release would itself create legal exposure.
_Avoid_: enterprise edition, premium module, closed core

**Browser host**:
The Automan process that runs beside a browser and carries out a Session's operations for an engine running elsewhere.
_Avoid_: worker, agent, sidecar

**Plugin**:
A package that adds implementations of one or more Plugin kinds to the engine, such as delivery to a Sink, a fetch backend or a Secret provider; the reverse of an Integration, which puts Automan inside another tool.
_Avoid_: extension (a browser extension is something else), add-on, connector, module

**Plugin kind**:
One of the fixed places where the engine accepts a Plugin, each with its own contract and conformance suite.
_Avoid_: extension point, hook (a Hook is one kind), slot

**Hook**:
The Plugin kind whose implementations run at a fixed stage of a request, a navigation, a page, a Record or a run's lifecycle, and may change, skip, retry or fail the work at that stage, or only observe it.
_Avoid_: middleware (for a Hook scoped by label), callback, interceptor

**Script Hook**:
A Hook written as a short script that the engine runs in its own sandbox, so it can travel inside a Recipe or settings without an SDK or a separate process.
_Avoid_: inline hook, user script

**Plugin index**:
The signed list of community Plugins that people search to find one, with each Plugin's conformance results.
_Avoid_: registry (for the index; packages live in their own registries), marketplace, hub

**Experimental**:
The status of an operation, setting or SDK method released outside semver's compatibility promise until it is promoted.
_Avoid_: unstable, nursery

## Pillars

**Extraction**:
Turning a page or a response into structured data or clean content.
_Avoid_: scraping, scraper

**Crawl**:
Discovering and visiting many pages from seeds, within a declared scope.
_Avoid_: scraping, spidering, scraper

**Automation**:
Driving a browser session through actions toward an objective.
_Avoid_: scraping, scraper, bot

## Execution

**Run**:
One piece of work the engine carries out from start to finish, with its own inputs, state, limits and usage. The unit of isolation, resumption and counting.
_Avoid_: job, execution, session (for a whole run)

**Run index**:
The catalog of every run of one user and one product major on a machine, pointing at each run's own state, from which it can be rebuilt.
_Avoid_: run registry, run database, history

**Effect**:
An action of a run that changes something on a website or third-party service, such as submitting a form or placing an order. A resumed run checks whether an effect already happened instead of repeating it, and reports it as uncertain when it cannot tell.
_Avoid_: side effect, mutation, write

**Reconciliation**:
A person's recorded answer about an uncertain Effect: it happened, it did not happen, or it stays uncertain. An agent can ask for one but never give it.
_Avoid_: resolution, manual fix, override (for this answer)

**Verdict**:
A run's statement of whether its Success checks, Page checks and Grounding prove its committed results: verified, rejected by a failed Success check, or needing a person's review.
_Avoid_: status, success, result (for the verdict)

**Recipe**:
A portable package of user code, described by a manifest, that runs the same locally or on Automan Cloud and can be published.
_Avoid_: actor, app, pack, script

**Operator policy**:
The settings an operator fixes for every run on a machine or Daemon, including the values runs may not exceed or change.
_Avoid_: admin config, global settings, defaults

**Usage**:
The raw quantities a run consumed, such as requests, browser time or LLM tokens, without any price.
_Avoid_: cost, consumption, credits

**Checkpoint**:
A consistent, self-contained copy of a run's state from which the run can resume, on the same machine or another one.
_Avoid_: snapshot, savepoint, backup

**Limit**:
A cap on a run's usage; reaching it stops the run cleanly.
_Avoid_: budget, quota

**Tenant**:
A customer of Automan Cloud. The engine never knows tenants.
_Avoid_: account, organization, user (for a customer)

## Collection

**Frontier**:
The URLs a run has discovered but not yet visited, with the list of those already visited and the rules that decide when two URLs are the same page.
_Avoid_: queue, request list, to-do list

**Completeness**:
A run's statement of whether its results cover everything in scope: complete, incomplete (ended early) or unverified (finished, but the engine cannot vouch that nothing was missed).
_Avoid_: success, done, finished (for coverage)

**Session**:
A reusable browsing identity of a run: its cookies, login state, Access mode, Identity profile and proxy, shared across the pages fetched under it.
_Avoid_: login, profile, context

**Access mode**:
How a Session presents itself to sites, fixed when it opens: standard (an ordinary browser), identified (a declared, signed agent) or stealth (masked).
_Avoid_: profile, access profile, stealth level

**Identity profile**:
The consistent set of values a Session presents to sites, such as its user agent, network and browser fingerprints, signing key, language and time zone, fixed for the Session's life.
_Avoid_: fingerprint (alone), persona, profile (alone)

**Session pool**:
The sessions a run keeps, scoring, rotating and retiring them as they fail.
_Avoid_: session manager, account pool

**Live view**:
A Session's browser shown to a person while it works, which that person can take control of during a Human pause.
_Avoid_: screencast, stream, remote desktop

**Run trace**:
The recording of what a run's Sessions did, such as their screens, page snapshots and network traffic, kept so a person can replay it.
_Avoid_: session recording, replay (for the recording), video

**Page check**:
A declaration by a run's code of what an expected page and a login page look like, against which every page fetched in a session is judged.
_Avoid_: contract, validator, assertion

**Dataset**:
The append-only collection of a run's Records.
_Avoid_: results table, output, items

**Record**:
One unit of structured data in a Dataset.
_Avoid_: row, document, item (an item is a unit of work)

**Key-value store**:
A run's named storage for files, blobs and the user's own state.
_Avoid_: KVS, blob store, cache

**Export**:
A file the engine writes from a run's results in a chosen format.
_Avoid_: dump, download

**Sink**:
A system outside the engine that receives a run's committed Records, as rows or messages or as Export files, while the run is in progress or afterwards.
_Avoid_: output, pipeline, connector

**Guarantee class**:
How strongly a Sink keeps a delivery from being duplicated or overwritten by a stale copy of the run, probed for each deployment: Fenced, Record-fenced, Keyed or At-least-once.
_Avoid_: delivery semantics, QoS, consistency level

**Notification**:
A message about a run's progress or outcome, sent to a person or a system, that carries no Records.
_Avoid_: alert, event (for the message), webhook (for every channel)

**Integration**:
A package that puts Automan inside another tool, such as a workflow tool node or a document loader; one that drives the engine is a Client.
_Avoid_: connector, plugin, app

**Baseline**:
A previous run that a run's results are compared with to report what changed.
_Avoid_: snapshot, reference run

## AI and agents

**Plan**:
The concrete steps of a task, produced by an AI or recorded from a person, each with a target and its Target fingerprint, kept as a versioned file and replayed without a model.
_Avoid_: cache, script, prompt

**Target fingerprint**:
What identifies a Plan step's target to a person, checked before any Effect to tell the same element from a look-alike.
_Avoid_: fingerprint (alone, it means a browser or TLS identity), selector, locator

**Drift**:
The state of a Plan step whose target no longer matches its Target fingerprint.
_Avoid_: breakage, flake

**Repair**:
One bounded recompilation of a Plan after Drift, checked in full before it acts and kept only after a verified run.
_Avoid_: self-healing, auto-fix

**Effect journal**:
The record of the Effects and model calls of a run, consulted so that a retried, corrected or resumed step never performs an Effect twice.
_Avoid_: action log, history

**Operation catalog**:
The single list of engine operations, each classed read, idempotent write or Effect, that every surface exposes to people and agents.
_Avoid_: tool list, API surface

**Code mode**:
Driving the engine through a short program written by a model that calls only the Operation catalog.
_Avoid_: scripting, eval

**Success check**:
A condition declared by the user that the engine evaluates live to decide whether a step or a task succeeded. A Page check judges every page of a Session; a Success check judges an outcome.
_Avoid_: check (alone), assertion, judge, validator

**Agent**:
A model-driven loop over the Operation catalog toward an objective, whether Automan's built-in one or an outside one; a single AI extraction is not an Agent.
_Avoid_: bot, assistant

**Model provider**:
Whatever answers a run's model calls: a model company reached directly, a gateway, a local model server, or a model run inside the engine.
_Avoid_: LLM vendor, backend

**Decision provider**:
A Model provider whose model picks one option from a fixed list with a confidence, or abstains.
_Avoid_: classifier, router

**Grounding**:
The requirement that every extracted value appears in its page or follows from a declared transform.
_Avoid_: verification, fact-check

**Human pause**:
The state of a Session's work items while they wait for a person to complete a login, MFA, a CAPTCHA, a confirmation or a review.
_Avoid_: manual step, HITL

## Security and compliance

**Secret reference**:
The name by which a run points to a credential, such as a password, a TOTP seed or an API key, without ever holding its value.
_Avoid_: secret (for the reference), credential ID, variable

**Secret provider**:
Whatever turns a Secret reference into its value at the moment of use: the environment, the OS keychain, an encrypted local file, the caller, or an external vault.
_Avoid_: vault (for every provider), secret store, credential manager

**Provenance**:
The record of how a Record was obtained: where and when it was fetched, under which Session and signing identity, by which extractor, Plan or model, and under which Site preferences.
_Avoid_: lineage, metadata, source (alone)

**Purpose**:
What a run declares its results are for (search, AI input, AI training, or plain collection), against which Site preferences are judged.
_Avoid_: use case, intent, mode

**Site preferences**:
What a site states about how it may be crawled and used, such as its robots.txt and its AI-use signals.
_Avoid_: robots (for all of them), policy, rules

**Audit log**:
The tamper-evident record of a run's security and compliance events, such as secret uses, confirmations and overrides. The Effect journal prevents repeated Effects; the Audit log answers who did what.
_Avoid_: journal (alone), event log, history

## Measurement

**Proving ground**:
The website Automan owns and generates, whose every page has a known expected result, against which the benchmark and the conformance suites run.
_Avoid_: fixtures site, test site, sandbox, demo site
