# B1–B4. Workflow tools, Airbyte, destination-protocol bridges, AI data loaders

Research date: 2026-10-11. Primary sources only: vendor docs, GitHub, and the npm and PyPI registries.
"**Inference**" marks anything not stated in a source.
Exa search was out of credits, so WebSearch/WebFetch, `gh api` and the npm and PyPI JSON APIs were used instead.

---

## B1. Workflow tools

### B1.1 How scraping vendors appear in each workflow tool

#### n8n

- On n8n's integration page, a verified community node reads "built and maintained by our partners at X and verified by n8n". https://n8n.io/integrations/apify/
- n8n Cloud only offers verified community nodes. "Unverified community nodes aren't available on n8n cloud." https://docs.n8n.io/integrations/community-nodes/installation/
- Self-hosted n8n installs community nodes from npm, either in the GUI or from the command line. https://docs.n8n.io/integrations/community-nodes/installation/

| Vendor | Package | Status | Triggers | Actions (as documented) | Auth |
|---|---|---|---|---|---|
| Apify | `@apify/n8n-nodes-apify` (0.8.0, MIT, npm provenance attestation present) | Verified community node, maintained by Apify. https://n8n.io/integrations/apify/ | "On new Apify Event": an Actor or task run reaches a terminal status (Succeeded, Failed, Aborted, Timed Out, Any). https://docs.apify.com/platform/integrations/n8n | Run an Actor; Run an Actor and get dataset; Web Fetch; Get last run; Run task; Run task and get dataset; Get user runs list; Get run; Get runs; Abort run; Get dataset items; Get key-value store record. https://docs.apify.com/platform/integrations/n8n | API key (cloud and self-hosted). OAuth2 on n8n Cloud only. https://docs.apify.com/platform/integrations/n8n |
| Firecrawl | `@mendable/n8n-nodes-firecrawl` (2.2.0, MIT; repo `firecrawl/n8n-nodes-firecrawl`). The unscoped `n8n-nodes-firecrawl` is a separate third-party package. https://registry.npmjs.org/@mendable%2Fn8n-nodes-firecrawl/latest | Verified community node, maintained by Firecrawl. https://n8n.io/integrations/firecrawl/ | None listed | Scrape, Batch scrape, Crawl (+status/cancel/errors), Map, Search, Agent, Browser sessions, Extract (legacy), account usage. https://n8n.io/integrations/firecrawl/ | API key (**inference**: the page does not state it) |
| Bright Data | `@brightdata/n8n-nodes-brightdata` (1.1.1, MIT, published by Bright Data staff accounts). https://registry.npmjs.org/@brightdata%2Fn8n-nodes-brightdata/latest | Verified community node, maintained by Bright Data. https://n8n.io/integrations/brightdata/ | None listed | Marketplace Dataset, Web Scraper, Web Unlocker, Scrape By URL, Trigger Collection By URL, snapshot operations, Web Search. https://n8n.io/integrations/brightdata/ | API key. https://docs.brightdata.com/integrations/n8n |
| ScrapingBee | `n8n-nodes-scrapingbee` (1.0.2, MIT; repo `ScrapingBee/scrapingbee-n8n`). https://registry.npmjs.org/n8n-nodes-scrapingbee/latest | Verified community node, maintained by ScrapingBee. https://n8n.io/integrations/scrapingbee/ | None | HTML API, Google Search, Amazon/Walmart/YouTube APIs, ChatGPT/Gemini API, Usage. https://n8n.io/integrations/scrapingbee/ | API key (**inference**) |
| Browserbase | `n8n-nodes-browserbase` (1.4.1, MIT; repo `browserbase/n8n-node`). https://registry.npmjs.org/n8n-nodes-browserbase/latest | Community node, as Browserbase's own docs describe it. No n8n verified listing was found (n8n.io/integrations/browserbase returned 404). https://docs.browserbase.com/integrations/n8n/introduction | None documented | Resources: Agent (Stagehand browser agent), Search, Fetch. https://docs.browserbase.com/integrations/n8n/introduction | Browserbase API key, plus an optional model API key. https://docs.browserbase.com/integrations/n8n/introduction |
| Browse AI | No n8n node found. npm searches for "n8n browse ai" and "n8n-nodes browse-ai" returned none on 2026-10-11. n8n's Browse AI page points to the generic HTTP Request node. https://n8n.io/integrations/browse-ai/ | — | — | — | — |

Notes:
- Bright Data's own docs still describe an older community node, `n8n-nodes-brightdata` from n8nhackers, and say it is "not officially developed by Bright Data". The scoped `@brightdata/...` package is the one n8n verified. https://docs.brightdata.com/integrations/n8n and https://registry.npmjs.org/n8n-nodes-brightdata/latest
- For self-hosted trigger nodes, n8n must produce webhook URLs that outside services can reach, not `localhost`. https://unpkg.com/@apify/n8n-nodes-apify@0.6.10/README.md

#### Zapier

| Vendor | Triggers | Actions | Searches | Auth |
|---|---|---|---|---|
| Apify | Finished Actor Run; Finished Task Run | Run Actor; Run Task; Scrape Single URL; Set Key-Value Store Record | Fetch Dataset Items; Get Key-Value Store Record; Find Last Actor Run; Find Last Task Run | OAuth. https://zapier.com/apps/apify/integrations and https://docs.apify.com/platform/integrations/zapier |
| Firecrawl | None | Crawl Website; Extract Data; Map Data; Scrape Page; Search Data; Run Agent; Cancel Agent | Get Agent Status; Get Data From /Crawl; Get Data From /Extract | Listed as "other". https://zapier.com/apps/firecrawl/integrations |
| Browse AI | New Task (task finished) | Run Task; Bulk Run Tasks (up to 50,000) | None | OAuth. https://zapier.com/apps/browse-ai/integrations |
| Bright Data | None | Scrape Website; Run a Web Unlocker; Download Snapshot Content | None | Listed as "other". https://zapier.com/apps/bright-data/integrations |
| ScrapingBee | None | Use HTML API; Retrieve Data From Dedicated APIs; Check Usage | None | Listed as "other". https://zapier.com/apps/scrapingbee/integrations |
| Browserbase | No app found (zapier.com/apps/browserbase/integrations returned 404). | | | |

- The Zapier pages do not say whether the vendor or Zapier built each app.
- **Inference**: these listings come from public integrations, which Zapier only allows from the API owner or a contractor of the API owner (see B1.2b).

#### Make

On Make, a "Verified" badge means "reviewed by Make". "Official Vendor" means "supported and maintained by the vendor".

| Vendor | Status | Triggers | Actions / searches | Auth |
|---|---|---|---|---|
| Apify | Verified, Official Vendor (Apify). https://www.make.com/en/integrations/apify | Watch Actor Runs; Watch Task Runs | Run an Actor; Run a Task; Scrape Single URL; Make an API Call; search Get Dataset Items | OAuth (recommended) or API token. https://docs.apify.com/platform/integrations/make |
| Firecrawl | Verified, Official Vendor (Mendable). https://www.make.com/en/integrations/firecrawl | None | Scrape, Crawl, Map, Extract (+status), Make an API Call; search: Search a Website | Not stated on the page |
| Browse AI | Verified, Official Vendor (Browse AI Inc.). https://www.make.com/en/integrations/browse-ai | Watch Task Execution Finished | Run a Task; Make an API Call | Not stated on the page |
| Bright Data | Verified, Official Vendor (Bright Data). https://www.make.com/en/integrations/brightdata | None | Run a Web Unlocker; Download/Get Snapshot (content, metadata, part); Filter a Dataset; Make an API Call | Not stated on the page |
| ScrapingBee | Verified, Official Vendor (ScrapingBee). https://www.make.com/en/integrations/scrapingbee | None | 14 actions (Extract Data, Google SERP, Amazon/Walmart, YouTube, Ask ChatGPT, Usage…) | Not stated on the page |
| Browserbase | No app found (make.com/en/integrations/browserbase returned 404). | | | |

#### Pipedream

Pipedream components live in the public `PipedreamHQ/pipedream` monorepo under `components/<app>/{actions,sources}`. https://github.com/PipedreamHQ/pipedream/tree/master/components

| App | Sources (triggers) | Actions | Auth field in `<app>.app.mjs` |
|---|---|---|---|
| apify | new-finished-actor-run-instant; new-finished-task-run-instant | run-actor; run-task-synchronously; scrape-single-url; get-dataset-items; get-kvs-record; set-key-value-store-record | `$auth.api_token` |
| firecrawl | none | scrape-page; crawl-url; get-crawl-status; map-url; search; extract-data; get-extract-status | `$auth.api_key` |
| browse_ai | task-completed; task-finished-with-error | execute-task | `$auth.api_key` |
| bright_data | none | scrape-website; scrape-serp; unlock-website | `$auth.api_key` |
| scrapingbee | none (app file only) | none | — |
| browserbase | new-session | create-session; create-context; list-projects | `$auth.api_key` |

- Sources: `gh api repos/PipedreamHQ/pipedream/contents/components/<app>` on 2026-10-11. Example: https://github.com/PipedreamHQ/pipedream/tree/master/components/apify
- Component packages name Pipedream as author. For example, `@pipedream/apify` lists "Pipedream <support@pipedream.com>". https://github.com/PipedreamHQ/pipedream/blob/master/components/apify/package.json

#### Activepieces

Pieces are TypeScript packages in `activepieces/activepieces` under `packages/pieces/community/<name>`. https://github.com/activepieces/activepieces/tree/main/packages/pieces/community

| Piece | Triggers | Actions | Auth | Authors |
|---|---|---|---|---|
| `@activepieces/piece-apify` 0.3.1 | watch-actor-runs; watch-task-runs | ~37 actions: run-actor, run-task, scrape-single-url, get-dataset-items, KV store, AI variants… | `PieceAuth.SecretText` / `CustomAuth` (API token) | `buttonsbond` (community) |
| `@activepieces/piece-firecrawl` 0.5.0 | none | scrape, batch-scrape, crawl, map, extract, search-web, start-agent… | `firecrawlAuth` (API key, **inference**) | community authors |
| `@activepieces/piece-browse-ai` 0.2.0 | task-finished-successfully; task-finished-with-error | run-robot; list-robots; get-task-details | — | `aryel780` (community) |

- No `bright-data`, `scrapingbee` or `browserbase` piece exists. Related pieces that do exist: `browserless`, `scrapeless`, `webscraping-ai`, and `scrapegrapghai` (directory name spelled this way). This comes from a listing of 737 community piece directories on 2026-10-11. https://github.com/activepieces/activepieces/tree/main/packages/pieces/community

#### Power Automate

- The `microsoft/PowerPlatformConnectors` repo, `dev` branch, has 1,193 connector folders. The only scraping-vendor match is `independent-publisher-connectors/ScrapingBee`. https://github.com/microsoft/PowerPlatformConnectors/tree/dev/independent-publisher-connectors/ScrapingBee
- That connector's publisher is "Troy Taylor" and its `stackOwner` is "ScrapingBee". It authenticates with an API key in the query string (`api_key`). Operations: Scrap URL, Get usage, Perform Google search. https://github.com/microsoft/PowerPlatformConnectors/blob/dev/independent-publisher-connectors/ScrapingBee/apiProperties.json
- No Apify, Firecrawl, Bright Data, Browse AI or Browserbase connector folder exists. Other matches are unrelated scrapers ("Advanced Scraper (Independent Publisher)") and the certified IA-Connect Web Browser connector.

#### IFTTT

- Apify has an IFTTT service. Triggers: Actor Run Finished, Task Run Finished. Actions: Run Actor, Run Task. Queries: Get Dataset Items, Scrape Single URL, Get Key-Value Store Record. https://docs.apify.com/platform/integrations/ifttt and https://ifttt.com/apify
- To connect, the user is redirected to the Apify login. **Inference**: this is OAuth. https://docs.apify.com/platform/integrations/ifttt
- No IFTTT services were found for Firecrawl, Bright Data, Browse AI, ScrapingBee or Browserbase (web search, 2026-10-11).
- IFTTT polls trigger endpoints "about every hour" unless the service uses the Realtime API. https://ifttt.com/docs/api_reference

### B1.2 How a third party publishes

#### (a) n8n community node

- The package name must start with `n8n-nodes-` or `@<scope>/n8n-nodes-`. https://docs.n8n.io/integrations/creating-nodes/deploy/submit-community-nodes/
- Add the keyword `n8n-community-node-package`, and list nodes and credentials under the `n8n` key in `package.json`. https://docs.n8n.io/integrations/creating-nodes/deploy/submit-community-nodes/
- Use the `n8n-node` CLI (`@n8n/node-cli` ≥ 0.23.0 for the provenance flag) to scaffold, lint (`npm run lint`) and run locally (`npm run dev`). https://docs.n8n.io/integrations/creating-nodes/deploy/submit-community-nodes/
- Requirements for verification (https://docs.n8n.io/integrations/creating-nodes/build/reference/verification-guidelines/):
  - The node must have no runtime dependencies: "verified community nodes aren't allowed to use any run-time dependencies."
  - It must not access environment variables or the file system.
  - It must be written in TypeScript.
  - The license must be MIT.
  - The npm repository URL must point to a public GitHub repo, and the npm and repo authors must match.
  - The interface and docs must be in English only.
  - One package covers one third-party service. Logic or flow-control nodes are not accepted.
  - The node must not duplicate an existing node.
- Run the scanner with `npx @n8n/scan-community-package <pkg>`. It is a "Static code analyser for n8n community packages" (v0.38.0). https://docs.n8n.io/integrations/creating-nodes/build/reference/verification-guidelines/ and https://registry.npmjs.org/@n8n%2Fscan-community-package/latest
- Provenance: "From May 1st 2026, nodes submitted for verification must be published using GitHub Actions with a provenance statement." n8n will not accept nodes published from a local machine. https://docs.n8n.io/integrations/creating-nodes/deploy/submit-community-nodes/
  - Example: `@apify/n8n-nodes-apify@0.8.0` carries an SLSA provenance attestation. https://registry.npmjs.org/@apify%2Fn8n-nodes-apify
- Submit through the n8n Creator Portal. n8n "reserves the right to reject nodes that compete with any of n8n's paid features." https://docs.n8n.io/integrations/creating-nodes/deploy/submit-community-nodes/
- Implication for Automan:
  - An n8n node that shells out to, or bundles, an `automan` client library would break the no-runtime-dependency rule.
  - A verified node would have to talk to Automan over HTTP using n8n's built-in request helpers.
  - **Inference**: an unverified community node (self-hosted only) has no such limit.

#### (b) Zapier app

- There are two equivalent build tools (https://docs.zapier.com/platform/quickstart/ui-vs-cli):
  - **Platform UI**: browser-based, no npm modules, limited auth (no OAuth v1).
  - **Platform CLI** (`zapier-platform`): local Node.js, git, unit tests in CI, custom code and middleware.
  - UI→CLI conversion works with `zapier convert`. There is no CLI→UI export.
- Private integrations:
  - Any developer can build one. https://docs.zapier.com/platform/publish/integration-publishing-requirements
  - They are shared by public invite link (unlimited, cannot be revoked) or by email invite. Docs give the email-invite cap as both 100 and 200. https://docs.zapier.com/integrations/manage/sharing and https://docs.zapier.com/platform/quickstart/private-vs-public-integrations
- Going public requires (https://docs.zapier.com/platform/publish/integration-publishing-requirements):
  - The integration owner is the API's company or a developer contracted by it.
  - The app is fully launched, not beta or invite-only.
  - A non-expiring test account.
  - Every trigger, action and search tested in a live Zap with a successful run.
  - API documentation links.
  - An admin on a company email domain.
- Flow: submit in Integration Home → review → **Beta** for 90 days (listed with a Beta tag) → **Public**. https://docs.zapier.com/platform/publish/public-integration
  - Beta ends early the next business day if a Zapier embed in the partner's product produces a signup. https://docs.zapier.com/platform/publish/public-integration
- User thresholds:
  - Publishing itself has none.
  - Partner Program tiers depend on active users and health score: Bronze (no minimum), Silver 50, Gold 350, Platinum 3,000 (or 350 with 20% quarterly growth). Tiers are reassessed quarterly. https://docs.zapier.com/integrations/publish/partner-program
- **Inference**: Automan as a self-hosted engine has no single API owner endpoint. A public Zapier app would need a hosted Automan service, or a per-user base-URL setting pointing at the user's own instance, and that instance must be publicly reachable (see B1.3).

#### (c) Make custom app

- Build in the Make web interface or in VS Code via the Apps SDK. https://developers.make.com/custom-apps-documentation
- Visibility levels (https://developers.make.com/custom-apps-documentation/app-visibility):
  - **Private**: author or organization only.
  - **Public**: an invite link from "Publish". This is irreversible; modules cannot be deleted afterwards, only hidden.
  - **Approved**: available to all Makers after an app review.
- Review prerequisites (https://developers.make.com/custom-apps-documentation/app-review/prerequisites.md):
  - The app targets a web service "not already available in Make", with its own domain (not a redirect).
  - It follows the mandatory best practices: sanitized secrets, error handling, a universal module, and `limit` plus pagination on search, trigger and RPC modules.
  - Every module is used in a test scenario, including one that produces an error.
  - The page does not say that the developer must own the API.

#### Quick notes on the others

- **Pipedream**
  - Contribute by PR to `PipedreamHQ/pipedream`. Automated lint checks run on the PR. https://pipedream.com/docs/components/contributing/guidelines
  - Components are `.mjs` ES modules with `key`/`name`/`version`/`description` and `annotations` (`destructiveHint`, `openWorldHint`, `readOnlyHint`). https://pipedream.com/docs/components/contributing/guidelines
  - Licensing: the repo is not OSI open source. It uses the "Pipedream Source Available License Version 1.0", which excludes commercial SaaS that competes with Pipedream. https://github.com/PipedreamHQ/pipedream/blob/master/LICENSE
- **Activepieces**
  - Pieces are TypeScript.
  - The repo is MIT except `packages/ee/` and `packages/server/api/src/app/ee`. https://github.com/activepieces/activepieces/blob/main/LICENSE
  - Three ways to share a piece:
    - Contribute by PR. After merge, a GitHub action publishes the piece to npm. https://www.activepieces.com/docs/developers/sharing-pieces/contribute
    - Publish to public npm under your own scope. https://www.activepieces.com/docs/build-pieces/sharing-pieces/community
    - Install as a private piece (paid edition). https://www.activepieces.com/docs/developers/sharing-pieces/private
- **Power Automate**
  - Custom connectors use OpenAPI 2.0 or Postman v2. OpenAPI 3 is not supported. https://learn.microsoft.com/en-us/connectors/custom-connectors/faq
  - Two certification paths (https://learn.microsoft.com/en-us/connectors/custom-connectors/submit-certification):
    - **Verified publisher**: owns the service.
    - **Independent publisher**: does not own it. Submits by PR to `microsoft/PowerPlatformConnectors`; the name is suffixed "(Independent Publisher)". https://learn.microsoft.com/en-us/connectors/custom-connectors/certification-submission-ip
  - Certified connectors are released only as Premium. Certification takes about 3–5 business days, plus 7–10 days for regional rollout. https://learn.microsoft.com/en-us/connectors/custom-connectors/faq
  - Supported auth: OAuth2, Basic, and API key (except through the on-premises data gateway). The client credentials grant is not supported. https://learn.microsoft.com/en-us/connectors/custom-connectors/faq
- **IFTTT**
  - Publishing is free.
  - The service implements IFTTT's Service API (directly or through a "shim app"). Each endpoint maps 1-to-1 to a trigger, action or query.
  - Review "can take up to a week". Launch needs at least twelve published Applets. https://ifttt.com/docs
  - Services that touch user data must authenticate users (OAuth2). Others send the `IFTTT-Service-Key` header. https://ifttt.com/docs/api_reference

### B1.3 Can cloud SaaS reach a self-hosted or localhost Automan?

- **Zapier**
  - Zapier cannot use localhost. Its docs state that "localhost, yourcomp.local and 127.0.0.1 are not valid" and suggest ngrok or a similar tunnel with a static HTTPS domain. https://docs.zapier.com/powered-by-zapier/embedding-zapier/elements-security
  - For self-hosted databases, Zapier requires the host to be reachable from the internet and allowlisted for Zapier's egress IPs. https://help.zapier.com/hc/en-us/articles/8496216020877-Zapier-IP-addresses-and-allowlisting
- **Make**
  - The On-premise agent (Enterprise plan only) lets scenarios reach local apps "without changing the settings of your firewall".
  - Only the HTTP Agent app is supported.
  - Source: https://help.make.com/on-premise-agent
  - On other plans, the endpoint must be public. **Inference**: in practice that means a tunnel such as ngrok or Cloudflare Tunnel.
- **Power Automate**
  - Custom connectors can route through the on-premises data gateway.
  - The gateway must be shared at Admin level for custom connectors.
  - API-key auth is not available through the gateway.
  - Request size is capped at 3,182,218 bytes.
  - Custom code does not run through the gateway.
  - Sources: https://learn.microsoft.com/en-us/connectors/custom-connectors/faq and https://learn.microsoft.com/en-us/connectors/custom-connectors/write-code
- **IFTTT and Pipedream**: both are cloud-hosted. **Inference**: like Zapier, they need a public HTTPS endpoint or a tunnel.
- **n8n (self-hosted) and Activepieces (self-hosted, MIT)**
  - **Inference**: they run on the user's network, so they can call a local Automan directly.
  - Inbound webhooks to n8n behind a proxy need `N8N_WEBHOOK_URL`, which replaced `WEBHOOK_URL` and is deprecated from 2.35.0. https://docs.n8n.io/hosting/configuration/configuration-examples/webhook-url/

---

## B2. Airbyte

### B2.1 How a vendor ships a source

- Build options (https://docs.airbyte.com/platform/connector-development):
  - **Connector Builder** (UI): recommended for API sources. Sources only.
  - **Low-code CDK**: a declarative YAML manifest, optionally with custom Python components. Sources only.
  - **Python CDK**: most flexible, most maintenance.
  - **Java CDK**: Airbyte "does not currently accept Java CDK contributions."
  - Community TypeScript and .NET CDKs also exist; Airbyte does not support them.
- Manifest-only connectors are tagged `language:manifest-only` and `cdk:low-code` in the registry. Example: `source-apify-dataset`. https://connectors.airbyte.com/files/registries/v0/oss_registry.json
- The latest declarative component schema (airbyte-python-cdk v7.35.0) defines `DatetimeBasedCursor`, `IncrementingCountCursor` (for an increasing integer `cursor_field`), `StateDelegatingStream` and `DeclarativeStream`. https://github.com/airbytehq/airbyte-python-cdk/blob/v7.35.0/airbyte_cdk/sources/declarative/declarative_component_schema.yaml
- `airbyte-cdk` (Python) is MIT. https://pypi.org/pypi/airbyte-cdk/json

### B2.2 Support levels

From https://docs.airbyte.com/integrations/connector-support-levels:

- **Airbyte** (registry value `certified`): maintained by Airbyte. Official SLAs require Premium Support.
- **Enterprise**: Airbyte-built, paid add-on for Enterprise and Pro customers.
- **Marketplace** (registry value `community`): community-maintained. No SLA, and breaking changes are possible.
- **Custom**: your own workspace connectors.
- **Archived**: no further updates.

Registry counts, OSS registry on 2026-10-11 (https://connectors.airbyte.com/files/registries/v0/oss_registry.json):
- Sources: 590 total; 61 `certified` (Airbyte-maintained), 6 `certified` under the "Airbyte Enterprise" license, 523 `community`.
- Destinations: 54 total; 14 certified, 40 community.

### B2.3 Incremental sync by cursor

- **Declarative `DatetimeBasedCursor`** (https://docs.airbyte.com/platform/connector-development/config-based/understanding-the-yaml-file/incremental-syncs):
  - Fields: `cursor_field`, `datetime_format`, `start_datetime`, `end_datetime`, `step` (window, e.g. `P1D`), `cursor_granularity` (e.g. `PT1S`), `lookback_window`.
  - `start_time_option` and `end_time_option` inject the window bounds into requests.
  - "Upon a successful sync, the final stream state will be the datetime of the last record emitted."
- **Protocol level** (https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol):
  - The source emits `STATE` messages: `STREAM` is preferred; `GLOBAL` is for shared state; `LEGACY` is being phased out.
  - The platform persists the last state the destination confirmed and passes it back with `read --state`.
  - `cursor_field` is required in `ConfiguredAirbyteStream` when `sync_mode` is `incremental`.

### B2.4 Contribution process

From https://docs.airbyte.com/platform/contributing-to-airbyte/submit-new-connector:

- **Low-code (preferred)**:
  - In Connector Builder, choose Publish → Contribute to Marketplace.
  - Builder opens a PR on `airbytehq/airbyte` using your GitHub classic token.
  - The Airbyte team reviews it.
- **Python CDK**: open a discussion or issue first.
- **PR requirements**:
  - Passing tests, plus README, docs and icon (CI enforces these).
  - Reviewers check auth, pagination, incremental sync where possible, and unit tests for custom components.
  - A sandbox account if the API needs one.

### B2.5 Scraping vendors with Airbyte sources

- **Apify Dataset**: `airbyte/source-apify-dataset` v2.2.62. Manifest-only, community ("Marketplace"), license ELv2. https://connectors.airbyte.com/files/registries/v0/oss_registry.json
  - Auth: Apify API token plus dataset ID.
  - Streams: `dataset_collection`, `dataset`, `item_collection`, `item_collection_website_content_crawler`.
  - Docs claim Full Refresh and Incremental. https://docs.airbyte.com/integrations/sources/apify-dataset
  - Discrepancy: the current `manifest.yaml` shows offset pagination and `primary_key: id`, but no `incremental_sync` block. https://github.com/airbytehq/airbyte/blob/master/airbyte-integrations/connectors/source-apify-dataset/manifest.yaml
- **Web Scrapper** (webscraper.io cloud): `airbyte/source-web-scrapper` v0.0.68. Manifest-only, community, ELv2, full refresh only. https://docs.airbyte.com/integrations/sources/web-scrapper
- **Not present** in the OSS or Cloud registries (searched by name and image on 2026-10-11): Firecrawl, Bright Data, Browse AI, ScrapingBee, Browserbase, Zyte, Oxylabs, Diffbot, Spider. https://connectors.airbyte.com/files/registries/v0/cloud_registry.json

---

## B3. Destination-protocol bridges

### B3.1 The Airbyte Protocol, as a destination sees it

- **Message envelope** (protocol v0.19.0, MIT):
  - Message types: `RECORD`, `STATE`, `LOG`, `SPEC`, `CONNECTION_STATUS`, `CATALOG`, `TRACE`, `CONTROL`, `DESTINATION_CATALOG`. https://github.com/airbytehq/airbyte-protocol/blob/v0.19.0/protocol-models/src/main/resources/airbyte_protocol/v0/airbyte_protocol.yaml
  - The docs page lists only the first seven types in the enum, and describes `CONTROL` separately. https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol
- **Destination contract** (https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol):
  - Signature: `write(Config, AirbyteCatalog, Stream<AirbyteMessage>(stdin)) -> Stream<AirbyteStateMessage>`.
  - Emitting a state message means "all records which came before it have been successfully written".
  - "Every state message the destination received must be returned back to the platform, in order."
- **Docker CLI** (https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol-docker):
  - Command: `docker run --rm -i <destination-image> write --config <file> --catalog <file>`, reading `AirbyteMessage`s on stdin.
  - Images must set `AIRBYTE_ENTRYPOINT` to match the `ENTRYPOINT`.
  - There is also a socket mode (`DATA_CHANNEL_MEDIUM=SOCKET`) for records.
- **`ConfiguredAirbyteStream`** (protocol v0.19.0 yaml above):
  - Required fields: `stream`, `sync_mode`, `destination_sync_mode`.
  - `destination_sync_mode` enum: `append`, `overwrite`, `append_dedup`, `update`, `soft_delete`. The docs page shows only the first three.
  - `primary_key` is "REQUIRED if `destination_sync_mode` is `*_dedup`".
  - Optional `generation_id`, `minimum_generation_id`, `sync_id`.
- **Bulk CDK (Kotlin destinations)** (https://github.com/airbytehq/airbyte/blob/master/airbyte-cdk/bulk/core/load/src/main/kotlin/io/airbyte/cdk/load/command/DestinationStreamFactory.kt):
  - Maps `APPEND_DEDUP` to `Dedupe(primaryKey, cursor)`.
  - Reads `generationId`, `minimumGenerationId` and `syncId` from the catalog.
  - Fails if a primary-key or cursor column is missing from the schema. https://github.com/airbytehq/airbyte/blob/master/airbyte-cdk/bulk/core/load/src/main/kotlin/io/airbyte/cdk/load/command/DestinationCatalog.kt

**Can a non-Airbyte program drive a destination image directly?**
- Yes, per the documented Docker interface: pipe newline-delimited `AirbyteMessage` JSON into `docker run -i … write --config … --catalog …` and read back the echoed `STATE` messages.
- **Inference**: a Rust engine would need to:
  - Generate a `ConfiguredAirbyteCatalog` with a JSON schema per stream.
  - Set `destination_sync_mode: append_dedup` and `primary_key: [["delivery_id"]]`, with a monotonic cursor such as an emitted-at timestamp.
  - Fill `generation_id`, `minimum_generation_id` and `sync_id`.
  - Interleave `STATE` checkpoints and treat each echoed state as a commit acknowledgement.
- **Inference**: undocumented behaviors of the Airbyte platform (for example per-state record counts) may also be needed. This is untested.

**Docker requirement.**
- Connectors are distributed as Docker images (`dockerRepository` in the registry).
- PyAirbyte can run Python destinations from a pip-installed virtualenv without Docker (`get_destination(...)`, or `docker_image=True` to opt into Docker).
- But "most destination connectors are written in Java, and for this reason are only available as Docker-based connectors." https://airbytehq.github.io/PyAirbyte/airbyte/destinations.html
- PyAirbyte's own repo license is now ELv2 (`LICENSE.md`). https://github.com/airbytehq/PyAirbyte/blob/main/LICENSE.md

### B3.2 Singer and Meltano

- **Singer spec v0.3.0** (https://github.com/singer-io/getting-started/blob/master/docs/SPEC.md):
  - A tap runs as `tap --config CONFIG [--state STATE] [--catalog CATALOG]` and writes JSON lines to stdout.
  - A target reads them on stdin (`tap | target`).
  - `SCHEMA` messages carry `stream`, `schema` and `key_properties`. `key_properties` is required, but may be an empty list.
  - `bookmark_properties` is optional.
  - `RECORD` messages have an optional `time_extracted`.
  - `STATE` messages carry `value`, whose meaning is tap-defined.
- **Meltano SDK** (`singer-sdk`, Apache-2.0):
  - It promises at-least-once delivery. "Most target implementation will simply use the primary key to merge newly received records." https://sdk.meltano.com/en/latest/implementation/at_least_once.html and https://pypi.org/pypi/singer-sdk/json
  - Meltano itself is MIT. https://pypi.org/pypi/meltano/json
- **Meltano Hub** lists Singer targets as "loaders". https://hub.meltano.com/loaders/
  - Example: MeltanoLabs `target-postgres` (MIT) has `load_method` set to `append-only`, `upsert` or `overwrite`. https://github.com/MeltanoLabs/target-postgres
- **Licenses vary**:
  - singer-io `target-csv`, `target-stitch` and `tap-github`: AGPL-3.0.
  - MeltanoLabs `target-jsonl`: AGPL-3.0.
  - datamill-co `target-postgres`: MIT.
  - `singer-python`: Apache-2.0.
  - Sources: `gh api repos/<repo>` license fields, e.g. https://github.com/singer-io/target-csv
- **Inference**: a Rust engine can emit Singer JSON lines to a target's stdin with `key_properties: ["delivery_id"]`, and choose a target whose load method is upsert to get dedup.

### B3.3 dlt (dltHub)

- `dlt` is Apache-2.0 (v1.31.0). https://pypi.org/pypi/dlt/json
- **`write_disposition="merge"`** (https://dlthub.com/docs/general-usage/merge-loading):
  - It uses `primary_key` (record identity) and/or `merge_key` (a partition to replace).
  - Strategies:
    - `delete-insert` (default): dedups by primary key in staging, all in one transaction.
    - `upsert`: needs a unique primary key, no dedup. Supported on Athena, BigQuery, Databricks, MSSQL, Postgres, Snowflake, and delta/iceberg on the filesystem.
    - `scd2`, `insert-only`, `cdc`.
  - The `dedup_sort` and `hard_delete` hints control which duplicate wins and when rows are deleted.
- **`@dlt.destination`** (https://dlthub.com/docs/dlt-ecosystem/destinations/destination):
  - It is a custom sink function `(items, table)`.
  - "There is no notion of write dispositions", so merge and dedup do not apply. Batches retry as whole units, so duplicates are possible.
- **Inference**: dlt is a Python library, not a stdin protocol. To use it, Automan output (e.g. JSONL) would be wrapped in a `@dlt.resource(primary_key="delivery_id", write_disposition="merge")` in a small Python shim. dlt's built-in destinations then provide the dedup.

### B3.4 Licensing summary

- **Airbyte monorepo** `LICENSE` is the full Elastic License 2.0 text. https://github.com/airbytehq/airbyte/blob/master/LICENSE
- **Connector relicensing**:
  - On 2025-07-22/23, PRs #63723 ("migrate connectors from MIT to ELv2") and #63730 ("migrate licenses to elv2") changed every connector's `license: MIT` to `ELv2`. https://github.com/airbytehq/airbyte/pull/63723 and https://github.com/airbytehq/airbyte/pull/63730
  - Airbyte's post says past contributions keep their original terms and new contributions are ELv2. It also says "ELv2 is source-available rather than open source by OSI definition." https://airbyte.com/blog/move-to-elv2
- **Discrepancy 1**: the published OSS registry still shows `MIT` for 38 sources and 16 destinations, among them `destination-csv`, `destination-local-json`, `destination-kafka`, `destination-elasticsearch`, `destination-sqlite` and `destination-redis`. **Inference**: these are versions published before the switch. https://connectors.airbyte.com/files/registries/v0/oss_registry.json
- **Discrepancy 2**: the current `metadata.yaml` on master says ELv2 for all checked destinations, including postgres, s3, bigquery, snowflake, duckdb, csv and local-json. https://github.com/airbytehq/airbyte/blob/master/airbyte-integrations/connectors/destination-csv/metadata.yaml
- **Discrepancy 3**: Airbyte's license FAQ still says "Our own connectors remain open-source". This conflicts with the 2025 change. https://docs.airbyte.com/platform/developer-guides/licenses/license-faq
- **ELv2 restriction**: no providing the software "to third parties as a hosted or managed service" that exposes a substantial set of its features. https://github.com/airbytehq/airbyte/blob/master/LICENSE
- **Still MIT**: the `airbyte-protocol` repo (https://github.com/airbytehq/airbyte-protocol) and `airbyte-python-cdk` (https://github.com/airbytehq/airbyte-python-cdk/blob/main/LICENSE.txt).
- **Other tools**: Singer targets vary (AGPL, MIT, Apache); `singer-sdk` is Apache-2.0; Meltano is MIT; dlt is Apache-2.0 (sources in B3.2–B3.3).

---

## B4. AI framework data loaders

| Vendor | LangChain | LlamaIndex | Vendor-maintained? |
|---|---|---|---|
| Apify | `langchain-apify` 0.1.7, Apache-2.0, author "Apify Technologies s.r.o.", repo `apify/langchain-apify`. Classes: `ApifyDatasetLoader`, `ApifyCrawlLoader`, `ApifyWrapper`, `ApifyActorsTool`, `ApifySearchRetriever`, and many `Apify*Tool`s. https://pypi.org/pypi/langchain-apify/json and https://github.com/apify/langchain-apify. The old `langchain_community` `ApifyDatasetLoader` is deprecated since 0.3.18 in favor of `langchain_apify.ApifyDatasetLoader`. https://github.com/langchain-ai/langchain-community/blob/master/libs/community/langchain_community/document_loaders/apify_dataset.py | `llama-index-readers-apify` 0.6.0, MIT. Readers `ApifyActor` and `ApifyDataset`; maintainer `drobnikj`. https://github.com/run-llama/llama_index/tree/main/llama-index-integrations/readers/llama-index-readers-apify | LangChain: yes (Apify org). LlamaIndex: in the run-llama monorepo, authored by `drobnikj`, who is also an npm maintainer of `@apify/*` packages (**inference**: an Apify employee). |
| Firecrawl | `langchain-firecrawl` 0.1.0 (2026-06-04), MIT, repo `firecrawl/langchain-firecrawl`, class `FirecrawlLoader`. https://pypi.org/pypi/langchain-firecrawl/json. Also the community `FireCrawlLoader` in `langchain_community.document_loaders.firecrawl`. https://github.com/langchain-ai/langchain-community/blob/master/libs/community/langchain_community/document_loaders/firecrawl.py | `FireCrawlWebReader` in `llama-index-readers-web` 0.7.0 (uses `firecrawl-py>=4.3.3`). https://github.com/run-llama/llama_index/tree/main/llama-index-integrations/readers/llama-index-readers-web | `langchain-firecrawl`: yes (Firecrawl org). Community loader and LlamaIndex reader: no; the LlamaIndex class author is listed as "llama-index". |
| Spider | `SpiderLoader` in `langchain_community.document_loaders.spider` (uses `spider-client`, MIT, author "Spider"). https://github.com/langchain-ai/langchain-community/blob/master/libs/community/langchain_community/document_loaders/spider.py and https://pypi.org/pypi/spider-client/json | `SpiderWebReader` in `llama-index-readers-web`; class author `WilliamEspegren`. https://github.com/run-llama/llama_index/blob/main/llama-index-integrations/readers/llama-index-readers-web/pyproject.toml | No standalone vendor package. `langchain-spider` and `llama-index-readers-spider` return 404 on PyPI. **Inference**: the reader author is a Spider founder. |
| Crawl4AI | No official loader found. PyPI 404 for `langchain-crawl4ai` and `llama-index-readers-crawl4ai`. No LangChain or LlamaIndex files in the `unclecode/crawl4ai` tree. Crawl4AI is Apache-2.0 with an added attribution clause. https://github.com/unclecode/crawl4ai/blob/main/LICENSE | none found | — |
| ScrapeGraphAI | `langchain-scrapegraph` 1.6.0, MIT, repo `scrapegraphai/langchain-scrapegraph`. Tools, not loaders: `SmartScraperTool`, `SearchScraperTool`, `MarkdownifyTool`. https://github.com/scrapegraphai/langchain-scrapegraph | `llama-index-tools-scrapegraphai` 0.2.2 (2025-10-02), MIT, author Marco Vinciguerra; a tool spec, not a reader. It is not in the current run-llama `tools/` directory. https://pypi.org/pypi/llama-index-tools-scrapegraphai/json | Yes for LangChain (ScrapeGraphAI org). **Inference**: also for LlamaIndex (author is a ScrapeGraphAI founder). |

Other vendor packages seen while checking:
- **Bright Data**:
  - `langchain-brightdata` 0.2.0, MIT, repo `luminati-io/langchain-brightdata`. https://pypi.org/pypi/langchain-brightdata/json
  - `llama-index-tools-brightdata` 0.4.0, MIT, author "Bright Data". https://pypi.org/pypi/llama-index-tools-brightdata/json
- **Browserbase**: `BrowserbaseLoader` in langchain-community, and `BrowserbaseWebReader` in `llama-index-readers-web`. https://github.com/run-llama/llama_index/tree/main/llama-index-integrations/readers/llama-index-readers-web
- **`llama-index-readers-web` license**: it declares `license = "GPL-3.0-or-later"` in `pyproject.toml` (v0.7.0). It also bundles readers for AgentQL, Hyperbrowser, Olostep, Oxylabs, ScrapFly, Scrapy, ZenRows and Zyte. https://github.com/run-llama/llama_index/blob/main/llama-index-integrations/readers/llama-index-readers-web/pyproject.toml
  - **Inference**: an Automan reader would be cleaner as its own `llama-index-readers-automan` package than inside this GPL bundle.
- **LangChain's own docs**: community document loaders are user-contributed and not reviewed or endorsed by LangChain, as summarized from a search result. https://docs.langchain.com/oss/python/integrations/document_loaders
