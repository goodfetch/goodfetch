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
