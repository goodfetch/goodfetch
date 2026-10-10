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
