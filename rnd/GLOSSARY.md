# Goodfetch

Goodfetch brings together web data collection, browser automation and model-assisted workflows. This vocabulary is shared by the framework, the interfaces and the hosted service.

## Automation

**Workflow**:
Reusable definition of an objective, its parameters, its steps and the conditions used to verify its result.
_Avoid_: scraper for any automation, agent for any workflow.

**Plan**:
Executable, versioned representation of a workflow, with its operations, dependencies, conditions and capability requirements.
_Avoid_: prompt, mere list of clicks.

**Run**:
One execution of a specific version of a workflow with given parameters and a given identity scope.
_Avoid_: workflow when talking about an execution, job to refer indiscriminately to a step or a run.

**Attempt**:
One execution of a unit of work, initial or repeated after a failure; it remains attached to the same run.
_Avoid_: new run for each retry.

**Repair**:
Production and qualification of a new version of a plan after a detected drift; on its own, it does not prove that the result is correct.
_Avoid_: self-healing as a universal guarantee.

**Agent**:
Decision loop that chooses operations based on the objective and the observations; it can be provided by Goodfetch or by an external system.
_Avoid_: agent for a mere model-based extraction.

## Collection and results

**Frontier**:
Ordered set of crawl requests to process, with their scope, their priority and their claim state.
_Avoid_: workflow queue, history of visited URLs.

**Crawl request**:
Unit of the frontier identified by its URL, its method, its parameters and its collection context.
_Avoid_: page when an HTTP response is enough.

**Dataset**:
Versioned collection of structured results, with a schema, provenance and deduplication contract.
_Avoid_: cache, mere output file.

**Artifact**:
Content associated with a run, such as a download, a screenshot, an observation or a trace, distinct from structured results.
_Avoid_: verified result for any artifact.

**Verified result**:
Result whose required identity, content, completeness or effect conditions have been satisfied by the declared validators.
_Avoid_: success when only the transport or the click succeeded.

## Execution

**Worker**:
Execution process that advertises capabilities and available capacity, then claims units of work.
_Avoid_: tab, asynchronous task or machine to automatically mean a worker.

**Backend**:
Implementation of a means of accessing the site: HTTP transport, CDP browser or Playwright driver, for example.
_Avoid_: engine for every interchangeable dependency.

**Session**:
Scope of identity and browsing state used to interact with a site; it can include cookies, storage and browser context.
_Avoid_: browser, CDP connection or account as synonyms.

**Pool**:
Set of reusable resources of the same kind, with admission, availability, health and retirement.
_Avoid_: single pool mixing sessions, tabs, browsers and workers.

**Lease**:
Temporary right to claim a unit of work or a session, whose holder and validity are identifiable.
_Avoid_: permanent lock, proof of exactly-once execution.

**Workflow package**:
Distributable unit that brings together versioned workflows, their input and output contracts, their required capabilities and their tests.
_Avoid_: Actor in the public Goodfetch contract, a term reserved for the comparison with Apify.
