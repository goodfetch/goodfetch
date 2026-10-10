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

**Effect**:
An action of a run that changes something on a website or third-party service, such as submitting a form or placing an order. A resumed run checks whether an effect already happened instead of repeating it, and reports it as uncertain when it cannot tell.
_Avoid_: side effect, mutation, write

**Recipe**:
A portable package of user code, described by a manifest, that runs the same locally or on Automan Cloud and can be published.
_Avoid_: actor, app, pack, script

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
A run's statement of whether its results cover everything in scope: complete, incomplete (stopped early) or unverified (finished, but the engine cannot vouch that nothing was missed).
_Avoid_: success, done, finished (for coverage)

**Session**:
A reusable browsing identity of a run: its cookies, login state, identity profile and proxy, shared across the pages fetched under it.
_Avoid_: login, profile, context

**Session pool**:
The sessions a run keeps, scoring, rotating and retiring them as they fail.
_Avoid_: session manager, account pool

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
A system outside the engine that receives a run's committed results while the run is in progress.
_Avoid_: output, pipeline, connector

**Baseline**:
A previous run that a run's results are compared with to report what changed.
_Avoid_: snapshot, reference run

## AI and agents

**Plan**:
The concrete steps an AI produced for a task, each with a target and its Fingerprint, kept as a versioned file and replayed without a model.
_Avoid_: cache, script, prompt

**Fingerprint**:
The identity of a Plan step's target, checked before any Effect to tell the same element from a look-alike.
_Avoid_: selector, locator

**Drift**:
A Plan step whose target no longer matches its Fingerprint.
_Avoid_: breakage, flake

**Repair**:
One bounded recompilation of a Plan after Drift, checked in full before it acts and kept only after a verified run.
_Avoid_: self-healing, auto-fix

**Effect journal**:
The record of the Effects a run has performed, consulted so that a retried or corrected step never performs one twice.
_Avoid_: action log, history

**Operation catalog**:
The single list of engine operations, each classed read, idempotent write or Effect, that every surface exposes to people and agents.
_Avoid_: tool list, API surface

**Code mode**:
Driving the engine through a short program written by a model that calls only the Operation catalog.
_Avoid_: scripting, eval

**Check**:
A condition declared by the user that the engine evaluates live to decide whether a step or a task succeeded.
_Avoid_: assertion, judge, validator

**Agent**:
A model-driven loop over the Operation catalog toward an objective, whether Automan's built-in one or an outside one.
_Avoid_: bot, assistant; a single AI extraction is not an agent

**Model provider**:
Whatever answers a run's model calls: a model company reached directly, a gateway, or a local model server.
_Avoid_: LLM vendor, backend

**Decision provider**:
A model that picks one option from a fixed list with a confidence, or abstains.
_Avoid_: classifier, router

**Grounding**:
The requirement that every extracted value appears in its page or follows from a declared transform.
_Avoid_: verification, fact-check

**Human pause**:
The state of a Session's work items while they wait for a person to complete a login, MFA, a CAPTCHA, a confirmation or a review.
_Avoid_: manual step, HITL
