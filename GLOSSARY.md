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
