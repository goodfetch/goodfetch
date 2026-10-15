# Research: presenting two Products in one docs site, migration guides from competitors, docs tooling and stealth wording

Gathered on 2026-10-15 for [Docs site and positioning of two Products](https://github.com/goodfetch/automan/issues/499), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447). Primary sources only: the live docs sites, their source repositories (read with `gh api`) and the package registries' JSON. Short quotes only; everything else is paraphrased. **Unverified** marks what could not be confirmed. Nothing here is a decision.

Part 1 covers how vendors with several open-source products present them (Apify and Crawlee, Browserbase and Stagehand, Astral, Vercel, Playwright, Browser Use, HashiCorp) and the migration guides that crawling and browser-automation projects publish. Part 2 covers Docusaurus 3.10.2 for a site with two product sections, llms.txt, package READMEs and tested examples, prose checks (Vale and alternatives), and how stealth packages word their registry metadata.

---

# Part 1. Multi-product docs sites and migration guides

## Part A. How vendors with several open-source products present them

### 1. Apify / Crawlee

Hosts and tooling
- crawlee.dev is the shared home: a JavaScript/Python chooser with one card per language ("Learn more", `npx crawlee create` vs `uvx 'crawlee[cli]' create`) and "Get started with JS / Python" buttons (https://crawlee.dev/).
- Every page reports `generator: Docusaurus v3.10.2` (https://crawlee.dev/, https://crawlee.dev/js/docs/quick-start, https://crawlee.dev/python/docs/quick-start).
- JS and Python are two separate Docusaurus builds from two repos sharing one domain:
  - `apify/crawlee` `website/docusaurus.config.js` sets `baseUrl: '/'` with docs at `routeBasePath: 'js/docs'`, API at `js/api`, and edit links to `apify/crawlee` (https://github.com/apify/crawlee/blob/master/website/docusaurus.config.js).
  - `apify/crawlee-python` `website/docusaurus.config.js` sets `baseUrl: '/python/'` and edit links to `apify/crawlee-python` (https://github.com/apify/crawlee-python/blob/master/website/docusaurus.config.js).
- Each build has its own version dropdown:
  - JS: current 3.18, `4.0 (RC)` at `/js/docs/next`, 3.17 down to 3.10, plus 2.2 and 1.3 pointing to the predecessor Apify SDK docs at sdk.apify.com (https://crawlee.dev/js/docs/quick-start; meta `docusaurus_version` 3.18).
  - Python: current 1.10, `Next`, 0.6 (https://crawlee.dev/python/docs/quick-start; meta `docusaurus_version` 1.10).
- The navbar logo opens a three-item product switcher (Crawlee JavaScript / Crawlee Python / Crawlee home). The JS site links to Python by absolute URL, and the reverse is also true (https://crawlee.dev/js/docs/quick-start).
- llms.txt: the JS config lists the Python `llms.txt` as an optional link (config above). On 2026-10-15, https://crawlee.dev/llms.txt served the *Python* index (byte-identical to https://crawlee.dev/python/llms.txt), and https://crawlee.dev/js/llms.txt returned 404.
- Versions: `crawlee` npm 3.18.2 (2026-09-29), 4.0.0-rc.1 tagged `rc` (https://registry.npmjs.org/crawlee); `crawlee` PyPI 1.10.4 (2026-10-06) (https://pypi.org/pypi/crawlee/json).

Vendor credit
- Home page: "Built with 🤍 by Apify. Forever free and open-source." (https://crawlee.dev/).
- Footer on every page: "Crawlee is forever free and open source", "© 2026 Apify", and a "More → Apify platform" link to apify.com (https://crawlee.dev/js/docs/quick-start).
- Code samples carry a "Run on" button that opens the snippet in Apify Console (https://crawlee.dev/js/docs/quick-start, https://crawlee.dev/python/docs/quick-start).

Paid platform vs open source
- The Apify platform appears in three places:
  - one page inside each language's "Deployment" section, beside AWS and GCP (JS: apify-platform, aws-browsers, aws-cheerio, gcp-browsers, gcp-cheerio; https://crawlee.dev/js/docs/deployment/apify-platform)
  - Python's equivalent section: Apify platform, AWS Lambda, Cloud Run, Cloud Run functions (https://crawlee.dev/python/docs/deployment/apify-platform)
  - the last tutorial chapter, "Running your crawler in the Cloud". It names Apify the home of Crawlee projects, deploys there with the Apify SDK and CLI, mentions the free tier, and points to the Deployment section for other platforms (https://crawlee.dev/js/docs/introduction/deployment; source https://github.com/apify/crawlee/blob/master/docs/introduction/09-deployment.mdx).
- The Apify platform page promotes a free account but says Crawlee stays open source and runs locally or on any cloud. It also notes that Crawlee is not tested in other cloud environments (https://github.com/apify/crawlee/blob/master/docs/deployment/apify-platform.mdx).
- Platform docs live on a separate Docusaurus site, docs.apify.com (generator Docusaurus v3.10.2). It hosts "SDK for JavaScript/Python" at `/sdk/js` and `/sdk/python` and links back to crawlee.dev (https://docs.apify.com/).

Helping the reader choose
- The only chooser is the home page's language cards. No "JS vs Python" comparison page was found in the sitemap (https://crawlee.dev/sitemap.xml). **Unverified** that none exists.

Shared pieces and cross-product references
- Shared concepts (guides, deployment, upgrading) are written separately in each repo; both sites have their own Stagehand, Playwright and deployment guides (JS sitemap above; https://crawlee.dev/python/llms.txt).
- The browser `page` object is not documented; the docs link out to it:
  - JS API reference renders `page: Page` as plain, unlinked text (https://crawlee.dev/js/api/playwright-crawler/interface/PlaywrightCrawlingContext). The source comment points to `pptr.dev/api/puppeteer.page` and `playwright.dev/docs/api/class-page` (https://github.com/apify/crawlee/blob/master/packages/browser-crawler/src/internals/browser-crawler.ts).
  - The PlaywrightCrawler example comments point to playwright.dev for `Page` (https://github.com/apify/crawlee/blob/master/docs/examples/playwright-crawler.ts).
  - Python API reference describes `page` as the Playwright Page object for the current page, without a link (https://crawlee.dev/python/api/class/PlaywrightCrawlingContext). Python guides link to playwright.dev/python for launch, context and `Page` options (https://github.com/apify/crawlee-python/blob/master/docs/guides/playwright_crawler.mdx).
- Stagehand methods follow the same pattern. The JS StagehandCrawler guide summarises `act/extract/observe/agent` in one example each and links to `docs.stagehand.dev/reference/{act,extract,observe,agent}` (https://github.com/apify/crawlee/blob/master/docs/guides/stagehand-crawler.mdx). On 2026-10-15 those four links redirected (308) to `/v4/reference/*`, which returned 404.

### 2. Browserbase / Stagehand

Hosts and tooling
- Three hosts:
  - stagehand.dev: marketing site (https://www.stagehand.dev/)
  - docs.stagehand.dev: Mintlify (`generator: Mintlify`); root redirects to `/v4/first-steps/introduction` (https://docs.stagehand.dev/)
  - docs.browserbase.com: separate Mintlify site, "Browserbase Documentation" (https://docs.browserbase.com/)
- Stagehand docs source is `packages/docs` (Mintlify `docs.json`, theme `maple`) in the SDK monorepo (https://github.com/browserbase/stagehand/blob/main/packages/docs/docs.json).
- One site carries three versions (v4, v3, v2). v3 adds per-language dropdowns: TypeScript, Python, Java, Go, Ruby (same `docs.json`).
- Versions: `@browserbasehq/stagehand` npm latest 4.2.1 (2026-10-11); 4.0.0 shipped 2026-08-10; `v3-latest` tag is 3.7.3 (https://registry.npmjs.org/@browserbasehq%2fstagehand). PyPI `stagehand` 4.2.1 (2026-10-10) (https://pypi.org/pypi/stagehand/json).

Vendor credit
- stagehand.dev footer reads "stagehand / by browserbase". "deploy to production" and "deploy to [B]" link to browserbase.com (https://www.stagehand.dev/).
- The docs introduction has a "Built by Browserbase" card. It also says Browserbase recommends running Stagehand on Browserbase, because a hosted browser enables server-side caching and the Model Gateway (https://docs.stagehand.dev/v4/first-steps/introduction).

Local vs Browserbase
- The "Browser" page offers three browser factories:
  - `browserbase.launch`: cloud, production
  - `localBrowser.launch`: local, development and debugging
  - `localBrowser.connect`: attach over CDP
- The same page has an "Environment comparison" table: scalability, stealth, proxies, session persistence, debugging, setup, server-side caching and Model Gateway (Browserbase only), cost, and "Best for" (https://docs.stagehand.dev/v4/configuration/browser.md).
- "Deploying Stagehand" recommends Browserbase Functions and links to docs.browserbase.com. The only "Other deployment options" accordion is "Deploy on Vercel", which still runs on Browserbase (https://docs.stagehand.dev/v4/best-practices/deployments.md).
- docs.browserbase.com has an external "Stagehand ↗" tab to docs.stagehand.dev. It also has its own Stagehand quickstart and many Stagehand-based integration pages that link back to docs.stagehand.dev (https://docs.browserbase.com/llms.txt, https://docs.browserbase.com/welcome/quickstarts/stagehand.md).

Migration guides listed in https://docs.stagehand.dev/llms.txt (complete list)
- Migrate v3 to v4: https://docs.stagehand.dev/v4/migrations/v3
- **Migrate Playwright to v4** (from another library): https://docs.stagehand.dev/v4/migrations/playwright
- Migrate TypeScript v2 to v3: https://docs.stagehand.dev/v3/migrations/v2
- Migrate Python v2 to v3: https://docs.stagehand.dev/v3/migrations/python
- Not listed, but live: **Migrate Browser Use to v4** (from another library) returns 200 at https://docs.stagehand.dev/v4/migrations/browser-use. Its source frontmatter has `noindex: true`, and `docs.json` lists only `v4/migrations/v3` and `v4/migrations/playwright` (https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/browser-use.mdx). Added 2026-08-20.

### 3. Astral (uv, Ruff, ty)

Hosts and tooling
- astral.sh is the company home; its "Tools" lead with Ruff, and the footer lists Ruff, uv and ty (https://astral.sh/).
- On 2026-10-15 a banner announced Astral joining OpenAI's Codex team (blog dated 2026-03-19; https://astral.sh/blog/openai).
- docs.astral.sh is a landing page, "Astral Docs", with three cards: Ruff, ty, uv (https://docs.astral.sh/).
- Each product is a separate MkDocs Material build under its own path:
  - uv: `mkdocs-1.6.1, mkdocs-material-9.7.6` (https://docs.astral.sh/uv/)
  - Ruff: `mkdocs-material-9.7.7` (https://docs.astral.sh/ruff/)
  - ty: `mkdocs-material-9.7.7` (https://docs.astral.sh/ty/)
- Each repo owns its own `mkdocs.yml` with `site_url` docs.astral.sh/uv/, /ruff/ or /ty/:
  - https://github.com/astral-sh/uv/blob/main/mkdocs.yml
  - https://github.com/astral-sh/ruff/blob/main/mkdocs.template.yml (`mkdocs.yml` inherits a generated file)
  - https://github.com/astral-sh/ty/blob/main/mkdocs.yml
- None of the three `nav` blocks contains another product.
- Versions on PyPI: uv 0.13.1 (2026-10-14), ruff 0.17.0 (2026-10-09), ty 0.0.87 (2026-10-13) (https://pypi.org/pypi/uv/json and the same path for ruff and ty).

Vendor credit
- One sentence on each product's home page:
  - "uv is backed by Astral, the creators of Ruff" (https://docs.astral.sh/uv/)
  - Ruff: backed by Astral, the creators of uv and ty (https://docs.astral.sh/ruff/)
  - ty: backed by Astral, the creators of uv and Ruff (https://docs.astral.sh/ty/)
- Headers and footers carry only the product's own GitHub, Discord, PyPI and X links (same pages).

Paid platform
- No paid product appears in the docs: https://docs.astral.sh/pyx/ returned 404, and the astral.sh home page did not mention pyx. **Unverified** where, or whether, any Astral commercial product is documented.

Helping the reader choose, and cross-references
- The products are complementary; no "which one" page exists beyond the landing cards.
- Ruff's FAQ "How does Ruff compare to ty, Mypy, Pyright or Pyre?" explains that Ruff is a linter, not a type checker, and recommends using both (https://docs.astral.sh/ruff/faq/).
- Cross-references are links, not copies:
  - Ruff docs link to uv docs from installation and tutorial; ty docs link to uv for `uvx` and installation (https://docs.astral.sh/ty/).
  - ty's migration table maps checks that ty lacks to linked Ruff rules (39 lines link to docs.astral.sh/ruff) (https://github.com/astral-sh/ty/blob/main/docs/coming-from-mypy-or-pyright.md).
- Competitor migration guides:
  - ty, "Coming from mypy or pyright": tips, recommended config, and a rule-mapping table across ty/Ruff, mypy and Pyright, where a blank cell means no equivalent (https://docs.astral.sh/ty/coming-from-mypy-or-pyright/).
  - uv, "Migrating from pip to a uv project". The migration index says other tools' guides are not yet written (https://docs.astral.sh/uv/guides/migration/).
  - uv, "Compatibility with pip" (https://docs.astral.sh/uv/pip/compatibility/).

### 4. Vercel (Next.js, AI SDK, Turborepo)

Hosts and tooling
- Each product has its own domain: nextjs.org, ai-sdk.dev, turborepo.dev.
- All three show the same header ("Vercel OSS" switcher) and the same Vercel global footer:
  - The first header item is a Vercel logomark labelled "Vercel Open Source", linking to vercel.com/oss.
  - The switcher groups products as Latest (eve), Frameworks (Next.js, Svelte, Nuxt, Nitro), SDKs (AI SDK, Chat SDK, Flags SDK, Workflow SDK) and Other (Turborepo, shadcn/ui, SWR, just-bash, deepsec).
  - Every entry links to that product's own domain (https://nextjs.org/, https://ai-sdk.dev/, https://turborepo.dev/).
- Docs tooling:
  - Turborepo uses Fumadocs (`fumadocs-ui` 16.2.2) on Next.js 16.3.8 (https://github.com/vercel/turborepo/blob/main/apps/docs/package.json).
  - nextjs.org and ai-sdk.dev expose no generator meta. They are Next.js apps (`_next/static` assets); the exact docs framework is **Unverified**.
- Agent delivery: all three answer with Markdown when the request does not accept `text/html`:
  - nextjs.org returns `text/markdown` with `Vary: Accept` (matched path `/llms.md`).
  - ai-sdk.dev returns an index listing v7/v6/v5 doc surfaces (https://ai-sdk.dev/llms.txt).
- Versions: next 16.4.1 (2026-10-14), ai 7.0.140 (2026-10-14), turbo 2.11.8 (2026-10-13) (https://registry.npmjs.org/next and the same path for ai and turbo).

Vendor credit
- Next.js: page title "Next.js by Vercel - The React Framework". Footer has "About Vercel", "Next.js + Vercel" and "Contact Sales". The Markdown home says it is maintained by Vercel (https://nextjs.org/).
- AI SDK: hero says the SDK is "powered by Vercel". The home page then promotes Vercel AI Gateway, Sandbox and Workflows (https://ai-sdk.dev/).
- Turborepo: no "by Vercel" line on the home page; the header has "Enterprise", which goes to vercel.com/contact/sales (https://turborepo.dev/). The Remote Caching page calls Vercel "the creators and maintainers of Turborepo" (https://turborepo.dev/docs/core-concepts/remote-caching).

Paid platform vs open source
- Next.js "Deploying" opens with a table: Node.js server, Docker, static export and adapters. It lists Vercel and Bun as the two "Verified Adapters", and Appwrite, Amplify, Cloudflare, Deno, Firebase and Netlify under "Other Platforms". No section is dedicated to Vercel (https://nextjs.org/docs/app/getting-started/deploying, lastUpdated 2026-08-06).
- The nextjs.org navbar "Deploy" item goes to vercel.com/new, and "Templates" to vercel.com/templates/next.js (https://nextjs.org/).
- Turborepo Remote Caching gives Vercel its own section first: free on all plans, even for apps not hosted on Vercel. A "Self-hosting" section follows (https://turborepo.dev/docs/core-concepts/remote-caching).

Helping the reader choose
- The shared "Vercel OSS" menu is the only cross-product navigation. No "which Vercel OSS product" page was found. **Unverified** that none exists.

Migration guides
- Next.js, from other tools: "Create React App" and "Vite" (https://nextjs.org/docs/app/guides/migrating/from-create-react-app, https://nextjs.org/docs/app/guides/migrating/from-vite).
  - The Vite guide (lastUpdated 2026-09-28) has a "Why Switch?" section followed by nine numbered steps and no mapping table.
- Next.js, internal: Pages Router to App Router, "Migrating to Cache Components", version upgrade guides 14/15/16, codemods, and "Upgrade with a coding agent" (https://nextjs.org/docs/llms.txt).
- AI SDK: version-to-version guides only, 3.0→3.1 through 6.x→7.0, plus "Migrate Your Data to AI SDK 5.0" and "Migrating from RSC to UI" (https://ai-sdk.dev/docs/migration-guides, https://ai-sdk.dev/docs/ai-sdk-rsc/migrating-to-ui).
  - LangChain and LlamaIndex appear only as interop adapters (https://ai-sdk.dev/providers/adapters/langchain).
- Cross-product references: AI SDK has per-framework getting-started pages (Next.js App and Pages Router, Nuxt, Svelte, TanStack Start, Expo, Node.js) and links to nextjs.org/docs on 17 lines of its full corpus (https://ai-sdk.dev/llms.txt).

### 5. Playwright (playwright.dev)

Hosts and tooling
- One domain with four language sites (`/`, `/python/`, `/java/`, `/dotnet/`), all Docusaurus (`generator: Docusaurus v3.10.1`) (https://playwright.dev/docs/library).
- The playwright.dev repo README says content is pulled from the microsoft/playwright repo (https://github.com/microsoft/playwright.dev).
  - `npm run roll` runs `src/generate.js` to fill `nodejs/`, `python/`, `java/` and `dotnet/`.
  - `build` runs four separate Docusaurus builds, with `--out-dir` build, build/python, build/java and build/dotnet (https://github.com/microsoft/playwright.dev/blob/main/package.json).
- Source is `microsoft/playwright/docs/src`, which marks language-specific content in three ways:
  - per-language files: `intro-js.md`, `intro-python.md`, `library-csharp.md`, … (63 of 106 entries)
  - `* langs: js` / `* langs: csharp, java` lines under a heading, as in https://github.com/microsoft/playwright/blob/main/docs/src/browsers.md (5 markers there) and https://github.com/microsoft/playwright/blob/main/docs/src/api/class-page.md (93 markers, including `alias-java:` entries)
  - language-tagged code fences (`python async`, `python sync`)
- Version: playwright npm 1.64.1 (2026-10-14) (https://registry.npmjs.org/playwright).

Playwright Test vs Library
- The "Library" page says the Library provides the browser-automation APIs and Playwright Test adds a fully managed end-to-end test runner (https://playwright.dev/docs/library).
- For end-to-end testing it recommends `@playwright/test` in most cases.
- It shows the same task twice: Library script with `node:assert` vs test with `expect`.
- A "Key Differences" table compares installation, browser install, import, initialization, assertions, cleanup and running.

Do guides default to the test runner?
- In the Node.js sidebar, "Getting Started" is test-runner based (writing, generating and running tests, Trace viewer, CI). A "Playwright Test" category follows. "Guides" starts with "Library" (https://playwright.dev/docs/intro).
- Guide snippets are mostly fragments that assume a `page`. By count, 15 of 44 JS blocks in `locators.md` use `test()` or `expect()`, as do 1 of 17 in `input.md` and 3 of 3 in `browsers.md` (sources under https://github.com/microsoft/playwright/tree/main/docs/src).
- The Python intro covers the pytest plugin and notes the library can also serve general browser automation (https://playwright.dev/python/docs/intro).

MCP and CLI placement
- The Node.js site navbar has top-level items "Docs", "MCP" (`/mcp/introduction`), "CLI" (`/agent-cli/introduction`), "API" and the language picker (https://playwright.dev/docs/intro).
- The Python navbar has no MCP or CLI item (https://playwright.dev/python/docs/intro).
- MCP and CLI are separate Docusaurus docs-plugin instances (`id: 'mcp'`, `id: 'agent-cli'`) configured only in `nodejs/docusaurus.config.ts`. They are authored as `.mdx` in the playwright.dev repo, not generated from docs/src (https://github.com/microsoft/playwright.dev/tree/main/mcp, https://github.com/microsoft/playwright.dev/tree/main/agent-cli).
- Each introduction has a comparison table pointing to the other, "Playwright MCP vs Playwright CLI" and "Playwright CLI vs MCP" (https://playwright.dev/mcp/introduction, https://playwright.dev/agent-cli/introduction).
  - Rows: how it works, token cost, setup.
- Versions: `@playwright/mcp` 0.0.83, `@playwright/cli` 0.1.22 (both 2026-09-28; https://registry.npmjs.org/@playwright%2fmcp).

Migration guides (Node.js only; Python, Java and .NET sitemaps contain none)
- The sidebar has a "Migration" category with three entries (https://github.com/microsoft/playwright.dev/blob/main/nodejs/sidebars.js):
  - Protractor: https://playwright.dev/docs/protractor
  - Puppeteer: https://playwright.dev/docs/puppeteer
  - Testing Library: https://playwright.dev/docs/testing-library
- There is no Cypress guide; https://playwright.dev/docs/cypress returns 404. Structure is in Part B.

### 6. Browser Use (docs.browser-use.com)

- One Mintlify host (`generator: Mintlify`). The root redirects to `/cloud/quickstart` (https://docs.browser-use.com/).
- Cloud and open source are separated as two Mintlify "products" in the switcher, "Cloud" and "Open Source", each under its own path (`/cloud/`, `/open-source/`).
  - Open Source pages get a blue accent via a `html[data-product="open-source"]` CSS rule (page HTML).
- Cloud tabs: Cloud SDK, API v4, v3 and v2 (page HTML).
- "Which Browser Use do I need?" is a three-way product map, mirrored from the GitHub README (https://docs.browser-use.com/cloud/which-product.md):
  1. fully hosted cloud agent (V4 API)
  2. CLI for an existing coding agent
  3. open-source Python library, run locally or connected to a cloud browser
  - plus "Need only a browser?", which points to the Browser API over CDP
- What `llms.txt` tells agents (https://docs.browser-use.com/llms.txt):
  - The title line describes the docs as covering the Cloud APIs and "the separate open-source Python library".
  - An "Agent Instructions" block says the open-source library and the hosted `browser-use-sdk` have different APIs. It recommends Cloud API V4 for new integrations and gives a dedicated note on open-source `is_done` / `is_successful` semantics.
  - The index has 175 `/cloud/` links vs 54 `/open-source/` links.
  - The OSS section includes "Prompts for Vibecoders" and an "open-source" agent skill (https://docs.browser-use.com/open-source/vibecoding.md).
- Cross-product: the OSS "Remote Browser" page covers connecting to Browser Use Cloud, Browserbase or any CDP endpoint (https://docs.browser-use.com/open-source/customize/browser/remote.md).
- Docs source: no `docs/` folder in `browser-use/browser-use` (https://github.com/browser-use/browser-use). **Unverified** where the docs source lives.
- Version: `browser-use` PyPI 0.13.11 (2026-10-07) (https://pypi.org/pypi/browser-use/json).

### 7. Umbrella example: HashiCorp Developer (developer.hashicorp.com)

- One Next.js host (`og:site_name` "HashiCorp Developer") with a global "Products" mega-menu (https://developer.hashicorp.com/). The menu is grouped by category:
  - Infrastructure Lifecycle Management: Terraform, Packer, Nomad, Waypoint, Vagrant
  - Security Lifecycle Management: Vault, Boundary, Vault Radar, Consul
  - a separate HashiCorp Cloud Platform entry
- Each product gets its own path prefix (`/terraform`, `/vault`, …) and a product subnav: Install, Tutorials, Documentation (https://developer.hashicorp.com/terraform/docs).
- The Documentation dropdown lists the open-source pieces and the paid editions side by side as sibling doc sets: Intro, Configuration Language, Terraform CLI, HCP Terraform, Terraform Enterprise, MCP Server, Terraform Migrate, Plugin Development, Registry Publishing (https://developer.hashicorp.com/terraform/docs).
- A comparison page, "Terraform versus alternatives", sits in the product's Intro (https://developer.hashicorp.com/terraform/intro/vs).
- Source is one repo with a content folder per doc set: `terraform`, `terraform-enterprise`, `terraform-migrate`, `terraform-mcp-server`, `vault`, `vault-hcp`, `hcp-docs`, … (https://github.com/hashicorp/web-unified-docs/tree/main/content).

## Part B. Migration guides from competitors (crawling and browser automation)

### Crawlee for Python: from Scrapy
- "Migrating from Scrapy": https://crawlee.dev/python/docs/guides/scrapy-migration (in the current 1.10 docs).
  - Source: https://github.com/apify/crawlee-python/blob/master/docs/guides/scrapy_migration.mdx
  - First commit 2026-07-13, last 2026-10-08.
- Structure:
  - intro on why concepts carry over (shared Parsel selectors); installation
  - "Conceptual differences": `scrapy crawl` vs a plain script, built-ins vs add-ons, Twisted vs asyncio, spider class vs Router
  - "Mapping key concepts" table, Scrapy → Crawlee, 28 rows, each with an API link
  - side-by-side Scrapy/Crawlee tabs for: canonical quotes spider, labels, `cb_kwargs`/`user_data`, CrawlSpider rules, export, concurrency and throttling, proxies, errback, FormRequest, and scrapy-playwright → PlaywrightCrawler
  - per-example "Note that" bullets and a conclusion
- No explicit "no equivalent" list. Behaviour differences appear inline; for example, Scrapy's per-request `meta={'playwright': True}` maps to AdaptivePlaywrightCrawler.
- Code lives in separate `.py` files under `docs/guides/code_examples/scrapy_migration/`: 12 Scrapy files and 11 Crawlee files.
- CI coverage: ty type-checks `docs` (includes `docs`, ignores `unresolved-import` for docs), and the ruff config covers it, excluding only `website/versioned_docs` (https://github.com/apify/crawlee-python/blob/master/pyproject.toml). Both run in the reusable "Lint check" and "Type check" jobs (https://github.com/apify/crawlee-python/blob/master/.github/workflows/_checks.yaml).
  - No job that executes the examples was found. **Unverified** that none runs them.
- Other Crawlee for Python guides: no BeautifulSoup-user, Puppeteer-user or Playwright-user migration guide (index of https://crawlee.dev/python/llms.txt). Version guides: "Upgrading to v0.x" and "Upgrading to v1" (https://crawlee.dev/python/docs/upgrading).

### Crawlee JS
- No guide from Scrapy, Puppeteer or Playwright exists in the docs (sitemap https://crawlee.dev/sitemap.xml).
  - A comparison blog post exists: "Scrapy vs. Crawlee", 2024-04-23 (https://crawlee.dev/blog/scrapy-vs-crawlee).
- "Upgrading to v3" targets Apify SDK v2 users (https://crawlee.dev/js/docs/upgrading/upgrading-to-v3).
  - It is a list of breaking changes between Apify SDK v2 and Crawlee v3, with Crawlee presented as the SDK's successor that keeps its version numbering.
  - About 19 top-level sections, including the monorepo, renamed options, removal of `requestAsBrowser`, and an "Apify SDK" section on platform helpers.
  - Inline `// Before:` / `// After:` snippets; no mapping table.
  - Source: https://github.com/apify/crawlee/blob/master/docs/upgrading/upgrading-v3.md. Released with crawlee 3.0.0 on 2022-07-13 (https://registry.npmjs.org/crawlee); the file history starts at a 2026-10-09 move.
- Version guides v1, v2 and v3. "Upgrading to v4" exists only in `next`, split into 10 sub-pages (autoscaling, browser-management, http-clients, …) (https://crawlee.dev/js/docs/next/upgrading/upgrading-to-v4).
- CI coverage: `.ts` examples imported into docs are type-checked by the "Typecheck documentation examples" step (`pnpm typecheck` = `tsc --noEmit` in the `docs` workspace) (https://github.com/apify/crawlee/blob/master/.github/workflows/test-ci.yml, https://github.com/apify/crawlee/blob/master/docs/package.json).
  - Inline fenced snippets, such as those in upgrading-v3.md, are not covered by that step.

### Scrapy
- No guide for migrating from another framework (docs tree https://github.com/scrapy/scrapy/tree/master/docs).
- The FAQ answers "How does Scrapy compare to BeautifulSoup or lxml?" and "Can I use Scrapy with BeautifulSoup?" (https://docs.scrapy.org/en/latest/faq.html). Scrapy 2.19.0, 2026-09-10 (https://pypi.org/pypi/scrapy/json).

### Stagehand
- "Migrate Playwright to v4": https://docs.stagehand.dev/v4/migrations/playwright. Added 2026-08-14 (https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/playwright.mdx).
  - Opens by stating there is no interop: v4 drives CDP directly with no Playwright dependency.
  - "What changed": Stagehand is not a test framework, so no fixtures, `expect()`, retries or reporter; then a side-by-side hello world (CodeGroup).
  - "Port a script": recommended order and a complete port.
  - "API differences" per area: launch, contexts, locators, waiting, assertions, network/storage/files, debugging.
  - "Rebuilding the missing APIs": recipes for `getBy*`, `filter()`, strict mode, auto-wait.
  - "Quick reference" tables, Playwright → Stagehand v4, about 114 table lines across the page; "no equivalent" is defined as no built-in method.
  - "Agent instructions": a kickoff prompt and porting rules to paste into `AGENTS.md`/`CLAUDE.md`; then troubleshooting.
  - TypeScript throughout.
- "Migrate Browser Use to v4" is unlisted (`noindex`, not in nav or llms.txt): https://docs.stagehand.dev/v4/migrations/browser-use. Added 2026-08-20.
  - Explains that v4 has no `Agent` object, then gives a side-by-side hello world, "Code mode", "Tool calling", an assistant prompt, migration order, breaking changes (init, task → explicit steps, models, structured output, sensitive data, custom tools, browser config), quick reference and troubleshooting.
  - Python throughout.
- CI: the docs job runs `vitest` tests that keep SDK reference pages in sync with the TS/Python/Go public API and resolve internal v4 links. It also runs `mint validate`, `mint broken-links --check-anchors --check-redirects --check-snippets` and `mint a11y` (https://github.com/browserbase/stagehand/blob/main/packages/docs/package.json, https://github.com/browserbase/stagehand/blob/main/.github/workflows/ci.yml).
  - No step compiles or runs the migration guides' code blocks.

### Browser Use
- No guide from another library: no migration or upgrade page in https://docs.browser-use.com/llms.txt.
- "Playwright Integration" is an interop example, not a migration (https://docs.browser-use.com/open-source/examples/templates/playwright-integration.md).
- CI testing of docs: **Unverified** (docs source repo not found).

### Firecrawl
- No migration guide, whether from competitors or between versions, appears in the current index or sitemap (https://docs.firecrawl.dev/llms.txt, https://docs.firecrawl.dev/_llms/en/v2.txt, https://docs.firecrawl.dev/sitemap.xml).
- https://docs.firecrawl.dev/migrate-to-v2 now redirects to /introduction.
- "Data migration" exists as a use-case page, not a tool migration (https://docs.firecrawl.dev/use-cases/data-migration). firecrawl-py 4.50.0, 2026-10-08.

### Crawl4AI
- Only internal migration notes exist, "Table Extraction v0.7.3" and "WebScrapingStrategy Migration Guide". They live in `docs/md_v2/migration/` and are live but absent from `mkdocs.yml` nav (https://docs.crawl4ai.com/migration/webscraping-strategy-migration/, https://github.com/unclecode/crawl4ai/tree/main/docs/md_v2/migration).
- No guide from Scrapy, Firecrawl or Playwright.
- Site: MkDocs with the `mkdocs-terminal` theme, "Crawl4AI Documentation (v0.9.x)"; https://docs.crawl4ai.com/llms.txt returned 503 on 2026-10-15. crawl4ai 0.9.5, 2026-10-14.

### Spider (spider.cloud / spider-rs)
- No migration guide. The spider.cloud llms.txt guides list contains none, and `/docs/migration` returns 404 (https://spider.cloud/llms.txt).
  - Competitor content takes the form of benchmark blog posts instead, e.g. "Firecrawl vs Crawl4AI vs Spider" and "Spider Browser vs Kernel and Browserbase" (listed in https://spider.cloud/llms.txt).
- The spider-rs repo has no migration doc, only READMEs, CHANGELOG and SKILLS files (https://github.com/spider-rs/spider).

### Playwright
- Three guides, all JS-only sources (`*-js.md`) (https://github.com/microsoft/playwright/tree/main/docs/src):
  - From Puppeteer: https://playwright.dev/docs/puppeteer (source created 2022-09-06, last change 2024-02-06)
  - From Protractor: https://playwright.dev/docs/protractor (2021-09-09 / 2023-10-06)
  - From Testing Library: https://playwright.dev/docs/testing-library (2022-05-31 / 2026-08-07)
- Shared skeleton: "Migration Principles" bullets, a "Cheat Sheet" two-column API mapping table (Puppeteer 24 rows, Protractor 12, Testing Library 18), then before/after example(s), sequentially rather than in tabs.
  - Each guide adds topic sections: for example Polyfilling `waitForAngular`, Replacing `waitFor`/`within`, Testing.
  - Each ends with "Playwright Test Super Powers" (what the test runner adds) and "Further Reading".
- Behaviour differences are stated in principles and table cells: ElementHandle discouraged in favour of Locators, auto-waiting makes explicit waits unnecessary, Locators are strict, and WebKit has no Puppeteer equivalent.
- No Cypress guide (404).
- CI: the "Lint snippets" job runs `utils/doclint/linting-code-snippets/cli.js`, which lints every code fence in `docs/src` (ESLint for JS/TS, plus Python, Java and C# linters). The "docs & lint" job runs `npm run doc` (doclint) (https://github.com/microsoft/playwright/blob/main/.github/workflows/infra.yml).
  - Snippets are linted, not executed.

### Crawlee's Stagehand integration
- JS: `@crawlee/stagehand` latest 3.18.2 (2026-09-29), `rc` 4.0.0-rc.1 (2026-10-06), and `v4` 4.0.0-beta.229 (2026-10-14) all peer-depend on `@browserbasehq/stagehand: ^3.0.0` (https://registry.npmjs.org/@crawlee%2fstagehand). That is Stagehand 3.x only, while Stagehand's npm `latest` is 4.2.1 and 4.0.0 shipped 2026-08-10.
- Python: crawlee 1.10.4 extra `stagehand` requires `stagehand>=3.19.5,<4.0.0` (https://pypi.org/pypi/crawlee/1.10.4/json; same on master, https://github.com/apify/crawlee-python/blob/master/pyproject.toml). PyPI `stagehand` latest is 4.2.1, and 4.0.0 shipped 2026-08-10.
- Official guides:
  - JS "StagehandCrawler guide": https://crawlee.dev/js/docs/guides/stagehand-crawler-guide, present in versioned docs since 3.16 (crawlee 3.16.0 released 2026-02-06). Covers architecture, Key features (act/extract/observe/agent), requirements, limitations (Chromium only, reduced fingerprinting), configuration, "Using with Browserbase" and further reading.
  - Python "Stagehand crawler": https://crawlee.dev/python/docs/guides/stagehand-crawler (first commit 2026-05-11). Covers installation, StagehandOptions, AI page operations (act/extract/observe/execute), Browserbase integration and browser-configuration limitations.
- Neither guide mentions Stagehand v4. **Unverified** whether a v4-compatible release is planned.

## Patterns (evidence only)

- **Separate builds per product or language, joined by a shared shell.**
  - Crawlee runs two Docusaurus builds from two repos under one domain, with a logo switcher.
  - Astral runs three MkDocs builds under docs.astral.sh with a card landing page.
  - Playwright generates four builds from one source.
  - Vercel keeps a domain per product with an identical "Vercel OSS" header menu and footer.
  - Only HashiCorp and Browser Use put several products in one site, using a product switcher with per-product paths.
- **Vendor credit is a single line, plus the vendor's platform as one deployment option among several.**
  - Credit lines: "by Apify", "backed by Astral", "Next.js by Vercel", "by browserbase".
  - Platform placement varies:
    - Crawlee: Apify beside AWS/GCP, plus the tutorial's last chapter.
    - Next.js: Vercel as one of two verified adapters.
    - Turborepo: Vercel cache section first, self-hosting after.
    - Stagehand: Browserbase is the recommended default, and a comparison table shows features available only there.
- **Third-party library APIs are linked, not copied.** Crawlee leaves `Page` unlinked in its API reference and points to playwright.dev/pptr.dev in prose. Its Stagehand guide summarises and links to docs.stagehand.dev. Several of those Stagehand links returned 404 after Stagehand's v4 docs restructure.
- **"Which one" pages exist where products overlap, not where they are complementary.**
  - Browser Use: "Which Browser Use do I need?".
  - Playwright: Library vs Test page; MCP vs CLI tables.
  - Stagehand: local vs Browserbase table.
  - Ruff: FAQ on Ruff vs type checkers.
  - HashiCorp: "Terraform versus alternatives".
  - Crawlee (JS vs Python) and Vercel have only cards or menus.
- **Competitor migration guides share a skeleton.** The common shape is principles or conceptual differences, an API mapping table, before/after code, then per-topic differences.
  - Examples: Playwright from Puppeteer/Protractor/Testing Library, Crawlee for Python from Scrapy, Stagehand from Playwright/Browser Use, ty from mypy/Pyright.
  - Newer guides (Stagehand 2026) add a "Quick reference" table and a ready-made coding-agent prompt and rules block.
  - Most competitor-focused vendors in crawling (Firecrawl, Crawl4AI, Spider, Scrapy, Browser Use) publish none; they use benchmark posts or FAQs instead.
- **Guide code is at most type-checked or linted in CI, never executed.**
  - Playwright lints every docs snippet in four languages.
  - Crawlee JS type-checks imported `.ts` examples.
  - Crawlee for Python type-checks and lints `.py` examples.
  - Stagehand checks reference-page sync, links and snippets but not guide code.

---

# Part 2. Docs tooling, READMEs, prose checks and stealth wording

## Part C. Docusaurus for a site with two product sections

### Current release

- Current stable is `@docusaurus/core` **3.10.2**, published 2026-07-10 (npm `latest`); a `4.0.0-canary-*` tag also exists ([registry](https://registry.npmjs.org/@docusaurus/core), [GitHub release v3.10.2](https://github.com/facebook/docusaurus/releases/tag/v3.10.2)). Docusaurus 4 is tracked in an open umbrella issue opened 2026-02-06 ([#11719](https://github.com/facebook/docusaurus/issues/11719)).

### C1. Multi-instance docs and versioning

- `@docusaurus/plugin-content-docs` supports several instances on one site, each with a unique `id`, its own `path`, `routeBasePath` and sidebar file; one instance may omit `id` and becomes `default` ([Docs Multi-instance, 3.10.2](https://docusaurus.io/docs/docs-multi-instance)).
- The guide's own note says multi-instance is only useful for versioned docs; plain multiple sidebars work within one instance. It warns that very large instances are better as separate sites ([Docs Multi-instance](https://docusaurus.io/docs/docs-multi-instance)).
- The guide's use case is two doc sets with possibly different versioning and release lifecycles (its example: Android SDK v1.0/v1.1 and iOS SDK v1.0/v2.0) ([Docs Multi-instance](https://docusaurus.io/docs/docs-multi-instance)).
- `routeBasePath` is a per-instance string (default `'docs'`, no trailing slash) ([plugin-content-docs options](https://docusaurus.io/docs/api/plugins/@docusaurus/plugin-content-docs)). `baseUrl` is the site-wide path segment after `url` ([docusaurus.config.js `baseUrl`](https://docusaurus.io/docs/api/docusaurus-config)). Inference from these two definitions: `baseUrl: '/docs/'` with `routeBasePath: 'collie'` and `'foxhound'` gives `/docs/collie/` and `/docs/foxhound/`. **Unverified** by a build in this research.
- Versioned files per instance: the default instance uses `versions.json`, `versioned_docs/`, `versioned_sidebars/`; an instance with an `id` uses `[pluginId]_versions.json`, `[pluginId]_versioned_docs/`, `[pluginId]_versioned_sidebars/` ([Versioned paths](https://docusaurus.io/docs/docs-multi-instance)).
- Each instance gets its own CLI command: `docusaurus docs:version 1.0.0` for the default instance, `docusaurus docs:version:<pluginId> 1.0.0` for the others ([Tagging new versions](https://docusaurus.io/docs/docs-multi-instance)).
- Lockstep: the docs describe only the per-instance commands; no option to version several instances together is documented in the multi-instance guide, the versioning guide or the plugin options ([Docs Multi-instance](https://docusaurus.io/docs/docs-multi-instance), [Versioning](https://docusaurus.io/docs/versioning), [plugin options](https://docusaurus.io/docs/api/plugins/@docusaurus/plugin-content-docs)). Absence of an undocumented mechanism is **Unverified**. Plugin options `lastVersion`, `onlyIncludeVersions`, `versions`, `includeCurrentVersion` and `disableVersioning` are all per instance ([plugin options](https://docusaurus.io/docs/api/plugins/@docusaurus/plugin-content-docs)).
- Field example of manual lockstep: playwright.dev runs four separate Docusaurus builds and a `version-all` npm script that chains `docs:version stable` for each ([playwright.dev package.json, Docusaurus 3.10.1](https://github.com/microsoft/playwright.dev/blob/main/package.json)).
- One instance with two top-level folders: `docs:version` copies the **full** `docs/` folder into `versioned_docs/version-<name>/`, snapshots the sidebars and appends to `versions.json` ([Versioning, "Tagging a new version"](https://docusaurus.io/docs/versioning)). So `docs/collie/` and `docs/foxhound/` in one instance share one version line; each folder can have its own sidebar ([Using multiple sidebars](https://docusaurus.io/docs/sidebar/multiple-sidebars)).
- Inference from the same copy rule: a partial imported from outside the instance's `path` (for example `@site/shared/_x.mdx`) is not snapshotted with a version. **Unverified**.

### C2. Navbar per instance

- `docsVersionDropdown`, `docsVersion` and `docSidebar` navbar items take `docsPluginId` (default `'default'`) ([theme configuration, navbar](https://docusaurus.io/docs/api/themes/configuration)). The multi-instance guide shows one `docsVersionDropdown` per instance (`docsPluginId: 'ios'`, `'android'`) ([Docs navbar items](https://docusaurus.io/docs/docs-multi-instance)).
- Dropdown behaviour (source, v3.10.2): it lists only its own instance's versions, keeps the same doc when switching if the doc id exists, else falls back to the version's main doc, and saves a preferred version per plugin id. No code hides it outside its instance's routes; with one item it renders a plain link ([DocsVersionDropdownNavbarItem.tsx](https://github.com/facebook/docusaurus/blob/v3.10.2/packages/docusaurus-theme-classic/src/theme/NavbarItem/DocsVersionDropdownNavbarItem.tsx)). Showing one dropdown only on its section would need custom code. **Unverified** whether an option exists elsewhere.
- The dropdown also accepts `versions` (subset or custom labels), `dropdownItemsBefore` and `dropdownItemsAfter` ([theme configuration](https://docusaurus.io/docs/api/themes/configuration)).

### C3. Search scoped per section

- Algolia `contextualSearch` is on by default; the docs say it limits results to the current language and docs version, plus unversioned pages ([Search, Contextual search](https://docusaurus.io/docs/search)).
- Facets in source (v3.10.2): `language:<locale>` AND any of `docusaurus_tag:default` or `docusaurus_tag:docs-<pluginId>-<versionName>` ([useAlgoliaContextualFacetFilters.ts](https://github.com/facebook/docusaurus/blob/v3.10.2/packages/docusaurus-theme-search-algolia/src/client/useAlgoliaContextualFacetFilters.ts), [docsSearch.ts](https://github.com/facebook/docusaurus/blob/v3.10.2/packages/docusaurus-plugin-content-docs/src/client/docsSearch.ts)).
- `useDocsContextualSearchTags` emits one tag **for every docs instance**: the browsed version for the active instance, else the preferred or last version of each other instance ([docsSearch.ts](https://github.com/facebook/docusaurus/blob/v3.10.2/packages/docusaurus-plugin-content-docs/src/client/docsSearch.ts)). Consequence: on `/docs/collie/…` results still include the other instance's last (or preferred) version. Scoping by instance is not built in.
- To filter by hand, you turn `contextualSearch` off and set `searchParameters.facetFilters`; with contextual search on, the two filter sets are merged ([Search](https://docusaurus.io/docs/search)). The DocSearch Docusaurus crawler template declares `language`, `version`, `docusaurus_tag` (and others) as facets ([DocSearch templates](https://docsearch.algolia.com/docs/templates/)).
- playwright.dev uses one Algolia index per language site (`playwright-nodejs`, `playwright-python`, …), since each language is a separate build ([nodejs config](https://github.com/microsoft/playwright.dev/blob/main/nodejs/docusaurus.config.ts), [python config](https://github.com/microsoft/playwright.dev/blob/main/python/docusaurus.config.ts)).
- DocSearch cost: Docusaurus says it is free for developer docs and technical blogs, after you apply ([Search](https://docusaurus.io/docs/search)). DocSearch eligibility covers public technical documentation and technical blogs; sites that are not production-ready or carry non-technical content are usually declined (page updated 2026-08-06, docs v5.x) ([Who can apply](https://docsearch.algolia.com/docs/who-can-apply/), [source](https://github.com/algolia/docsearch/blob/main/packages/website/docs/who-can-apply.md)).
- DocSearch Plan Terms, effective 2026-07-24: free plan for developer docs and technical blogs on public sites. Limits: 5,000,000 records, 50,000,000 search requests and 5,000,000 crawled resources a month. Required: a visible Algolia logo linking to algolia.com. No support; Algolia may terminate at its sole discretion. Includes automatic enrolment in "DocSearchMCP" with a licence to crawl and index for it ([DocSearch Plan Terms](https://www.algolia.com/policies/docsearch-plan-specific-terms)). The terms do not mention open-source status as a condition. `@docsearch/js` latest is 5.1.2 (2026-10-05) ([registry](https://registry.npmjs.org/@docsearch/js)).
- `@easyops-cn/docusaurus-search-local` 0.55.3 (2026-07-29, MIT). `searchContextByPaths` builds a separate index per listed sub-path and scopes results to the context matching the current URL. `docsPluginIdForPreferredVersion` takes one plugin id; `hideSearchBarWithNoSearchContext` and `useAllContextsWithNoSearchContext` control pages outside any context ([registry](https://registry.npmjs.org/@easyops-cn/docusaurus-search-local), [README](https://github.com/easyops-cn/docusaurus-search-local#readme)). Scoping is by path, not by plugin id. A maintainer confirmed the URL-based scoping on 2024-01-12 ([issue #394](https://github.com/easyops-cn/docusaurus-search-local/issues/394)). How it handles several versions per instance is **Unverified**.
- `@cmfcmf/docusaurus-search-local` 2.0.1 (2025-10-25, MIT): searches only the browsed docs version ([README](https://github.com/cmfcmf/docusaurus-search-local#readme), [registry](https://registry.npmjs.org/@cmfcmf/docusaurus-search-local)). Multi-instance support was added in 0.8.0 (2021-12-04) ([issue #33](https://github.com/cmfcmf/docusaurus-search-local/issues/33)). An open request to filter results by product, opened 2024-01-11 ([issue #200](https://github.com/cmfcmf/docusaurus-search-local/issues/200)). Per-instance scoping is **Unverified**.

### C4. One source rendered into two sections with different names

- MDX partials: a file named with a leading `_` creates no page and can be imported into other docs; the imported component receives props (the docs' example renders `{props.name}`) ([MDX and React, Importing Markdown](https://docusaurus.io/docs/markdown-features/react#importing-markdown), [_markdown-partial-example.mdx](https://github.com/facebook/docusaurus/blob/main/website/versioned_docs/version-3.10.2/guides/markdown-features/_markdown-partial-example.mdx)). So a wrapper page per section could import `_shared.mdx` and pass a product name as a prop.
- Limit: when a prop is used in a partial's heading, the TOC shows the literal `{props.…}` text. Open issue since 2024-01-21 ([#9772](https://github.com/facebook/docusaurus/issues/9772)). Headings from imported partials do appear in the TOC: the earlier issue was closed as completed on 2024-01-19 ([#3915](https://github.com/facebook/docusaurus/issues/3915)).
- Remark/rehype plugins can transform Markdown at build time ([MDX plugins](https://docusaurus.io/docs/markdown-features/plugins)). Each docs instance takes its own `remarkPlugins`, `rehypePlugins` and `beforeDefaultRemarkPlugins` ([plugin options](https://docusaurus.io/docs/api/plugins/@docusaurus/plugin-content-docs)). A per-instance token-substitution plugin is therefore possible, but none is built in. **Unverified**: no first-party "variables" feature found. `customFields` holds arbitrary config values ([docusaurus.config.js `customFields`](https://docusaurus.io/docs/api/docusaurus-config)).
- **Unverified**: whether two docs instances may point their `path` at the same folder.
- Playwright's approach (generator in `microsoft/playwright.dev`, sources in `microsoft/playwright/docs/src`):
  - `npm run roll` reads `$SRC_DIR/docs/src` and writes one docs tree per language into `nodejs/docs`, `python/docs`, `java/docs`, `dotnet/docs`. Each tree is its own Docusaurus site (`baseUrl` `/`, `/python/`, …) built separately ([README](https://github.com/microsoft/playwright.dev#readme), [generate.js](https://github.com/microsoft/playwright.dev/blob/main/src/generate.js), [package.json](https://github.com/microsoft/playwright.dev/blob/main/package.json)).
  - File level: a file name containing `-js`, `-python`, `-java` or `-csharp` (for example `intro-js.md`, `test-assertions-csharp-java-python.md`) goes only to those languages. The suffix is stripped in the output name ([generator.js, guides loop](https://github.com/microsoft/playwright.dev/blob/main/src/generator.js), [docs/src listing](https://github.com/microsoft/playwright/tree/main/docs/src)).
  - Section level: a bullet `* langs: js` (or `* langs: csharp, java`) right under a heading keeps that node and its children only for the listed languages; the marker is removed ([generator.js `filterForLanguage`](https://github.com/microsoft/playwright.dev/blob/main/src/generator.js), [markdown.js `filterNodesForLanguage`](https://github.com/microsoft/playwright.dev/blob/main/src/markdown.js), example in [auth.md](https://github.com/microsoft/playwright/blob/main/docs/src/auth.md)).
  - Code blocks: the fence info string's language (`js`, `python`, `java`, `csharp`) filters blocks per language. `generic`, common snippets, and `<lang> browser` or `sh python`-style second tokens are kept by rule. `tab=` attributes build tab groups ([generator.js `formatComment`](https://github.com/microsoft/playwright.dev/blob/main/src/generator.js)).
  - Text substitution: `%%VERSION%%` is replaced with the language binding's version ([generator.js `rewriteVersion`](https://github.com/microsoft/playwright.dev/blob/main/src/generator.js)).
  - HTML comments: the generator code read treats `<!--` lines only as a rendering detail (no blank line between consecutive generated comment lines), not as a language marker ([markdown.js](https://github.com/microsoft/playwright.dev/blob/main/src/markdown.js)). Any other comment-based marker is **Unverified**.

### C5. llms.txt

- Proposal v2 (changes dated August 2026; original 2024-09-03, Jeremy Howard). The file may sit at the root `/llms.txt` or at any subpath (for example `/docs/llms.txt`). A file covers URLs under its path, and agents should use the most specific file. v2 also recommends `rel="describedby"` and `rel="alternate" type="text/markdown"` links ([llmstxt.org](https://llmstxt.org/), [Changes](https://llmstxt.org/changes.html), [source](https://github.com/AnswerDotAI/llms-txt/blob/main/nbs/index.qmd)). So `/docs/collie/llms.txt` conforms.
- Docusaurus has no official llms.txt plugin; a proposal issue is open since 2025-02-04 ([#10899](https://github.com/facebook/docusaurus/issues/10899)).
- `docusaurus-plugin-llms` **0.6.1**, released 2026-10-02/03, MIT, about 138k monthly downloads ([registry](https://registry.npmjs.org/docusaurus-plugin-llms), [CHANGELOG](https://github.com/rachfop/docusaurus-plugin-llms/blob/main/CHANGELOG.md), [npm search API](https://registry.npmjs.org/-/v1/search?text=docusaurus%20llms&size=20)).
  - `docsDir` takes an array of sections (`path`, `routeBasePath`, `label`) for multi-instance sites. This yields **one** `llms.txt` with a heading per section ([content generation](https://rachfop.github.io/docusaurus-plugin-llms/docs/content-generation)).
  - `customLLMFiles` writes extra files scoped by globs, each with its own `filename`, for example `llms-python.txt` ([content generation](https://rachfop.github.io/docusaurus-plugin-llms/docs/content-generation)).
  - `versions` writes a full set of files per version under `<outDir>/<path>/`, for example `/stable/llms.txt`, and each version entry takes its own `docsDir` sections ([multi-version](https://rachfop.github.io/docusaurus-plugin-llms/docs/multi-version)).
  - Partials (`_*.mdx`) are skipped as pages and inlined where imported ([content generation](https://rachfop.github.io/docusaurus-plugin-llms/docs/content-generation)).
  - A separate `llms.txt` written into each section's subpath (for example `/docs/collie/llms.txt`) is not documented as a feature. **Unverified** whether `versions[].path` or a second plugin registration can do it.
- `@signalwire/docusaurus-plugin-llms-txt`: stable **1.2.2** (2025-07-23); prerelease `2.0.0-alpha.7` (2025-11-10); MIT; repo last pushed 2026-10-05 ([registry](https://registry.npmjs.org/@signalwire/docusaurus-plugin-llms-txt), [repo](https://github.com/signalwire/docusaurus-plugins)).
  - It converts built HTML to Markdown and writes one `llms.txt` (plus optional `llms-full.txt`), grouped into manual `sections` (route globs) or auto-sections by path depth ([README, v2 docs](https://github.com/signalwire/docusaurus-plugins/blob/main/packages/docusaurus-plugin-llms-txt/README.md)).
  - `includeVersionedDocs` defaults to `false` for llms.txt.
  - Per-section separate files are not documented. **Unverified**.

## Part D. Package READMEs and tested examples

- **crates.io**: the Cargo `readme` field names a file sent to the registry on publish. crates.io renders it as Markdown on the crate page, and `README.md` is picked up by default ([Cargo manifest, `readme`](https://doc.rust-lang.org/cargo/reference/manifest.html#the-readme-field)). Without a `documentation` URL, crates.io links the crate's docs.rs page ([manifest, `documentation`](https://doc.rust-lang.org/cargo/reference/manifest.html#the-documentation-field)). docs.rs builds docs for every library published to crates.io ([docs.rs about](https://docs.rs/about)).
- **rustdoc include pattern**: the rustdoc book shows `#[doc = include_str!("../../README.md")]` for using an external file as docs ([the doc attribute](https://doc.rust-lang.org/rustdoc/write-documentation/the-doc-attribute.html)).
- **README doctests**: rustdoc runs documentation examples as tests ([Documentation tests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html)). The same page shows `#[doc = include_str!("../README.md")] #[cfg(doctest)] pub struct ReadmeDoctests;` to test a README without putting it in the public docs, and suggests `rust` fence tags for README code also read by other tools. `cargo test` runs doc tests by default ([cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html)).
- **npm**: npmjs.com shows the package README, a `README.md` in the package root, on the package page; doc last edited 2025-11-04 ([About package README files](https://docs.npmjs.com/about-package-readme-files)).
- **PyPI**: `[project] readme` names the long description shown on the PyPI project page. Format comes from the extension (`.md` is GitHub-flavoured Markdown, `.rst` is reStructuredText) or an explicit `content-type` ([Writing your pyproject.toml, `readme`](https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#readme)).
- **Projects testing README code in CI**:
  - `clap`: `src/lib.rs` has the `ReadmeDoctests` pattern at lines 108–110 ([lib.rs](https://github.com/clap-rs/clap/blob/main/src/lib.rs#L108-L110)). CI runs `make test-<features>`, which runs `cargo test` ([ci.yml](https://github.com/clap-rs/clap/blob/main/.github/workflows/ci.yml), [Makefile](https://github.com/clap-rs/clap/blob/main/Makefile)).
  - `env_logger` uses the same pattern ([lib.rs](https://github.com/rust-cli/env_logger/blob/main/src/lib.rs)). `bon` makes its README the crate-level docs with `#![doc = include_str!("../README.md")]` ([lib.rs](https://github.com/elastio/bon/blob/master/bon/src/lib.rs)).
  - Pydantic AI: `tests/test_examples.py` collects examples from `README.md`, `docs` and package directories with `pytest_examples.find_examples` ([test_examples.py](https://github.com/pydantic/pydantic-ai/blob/main/tests/test_examples.py)). CI's `docs-only` job runs `pytest tests/test_examples.py` ([ci.yml](https://github.com/pydantic/pydantic-ai/blob/main/.github/workflows/ci.yml)). Tool: `pytest-examples` 0.0.18 (2025-05-06, MIT) ([PyPI](https://pypi.org/pypi/pytest-examples/json)).
  - Pydantic (core library) tests `docs/` and docstrings with `pytest-examples`, not its README ([tests/test_docs.py](https://github.com/pydantic/pydantic/blob/main/tests/test_docs.py)).

## Part E. Prose checks

### Vale

- Licence: MIT. Repo moved from `errata-ai/vale` to `vale-cli/vale` (GitHub redirect) ([repo](https://github.com/vale-cli/vale), [LICENSE](https://github.com/vale-cli/vale/blob/v3/LICENSE)). Latest release **v3.24.0**, 2026-10-01; previous v3.23.0 (2026-09-25), v3.22.0 (2026-09-17) ([releases](https://github.com/vale-cli/vale/releases)). PyPI wrapper `vale` 3.22.0.0 (2026-09-18, MIT) ([PyPI](https://pypi.org/pypi/vale/json)).
- `existence` rule: `tokens` are compiled into a word-bounded, non-capturing group (`\b(?:…)\b`). `ignorecase: true` makes matches case-insensitive. `nonword: true` removes the default `\b` boundaries. `exceptions` lists strings to ignore. `raw` gives full regex control. `vocab: false` turns off vocabularies for the rule ([existence](https://vale.sh/docs/checks/existence)). Field example: GitLab's `gitlab_base.Simplicity` bans "easy", "simple", "simply" and similar with `extends: existence`, `ignorecase: true`, `vocab: false` ([Simplicity.yml](https://gitlab.com/gitlab-org/gitlab/-/blob/master/doc/.vale/gitlab_base/Simplicity.yml)).
- Scoping by files: `.vale.ini` section headers are globs matched on the file path, for example `[*.{md,mdx}]` or `[docs/src/*.md]`. A file takes every matching section in order, and rule levels can differ per section ([.vale.ini](https://vale.sh/docs/vale-ini), [Globbing](https://vale.sh/docs/guides/globbing)). In Vale's glob syntax, `*` also crosses `/` ([Globbing](https://vale.sh/docs/guides/globbing)).
- MDX: Vale v3.18.0 (2026-08-20) and later parse `.mdx` natively; earlier versions needed the external `mdx2vast`. JSX, ESM and expressions are treated as code. Since v3.19.0 (2026-08-26) JSX children are linted and scoped by component name (`text.class.<Name>`, `IgnoredClasses`) ([MDX format](https://vale.sh/docs/formats/mdx), [v3.18.0](https://github.com/vale-cli/vale/releases/tag/v3.18.0), [v3.19.0](https://github.com/vale-cli/vale/releases/tag/v3.19.0)).
- Docs projects running Vale in CI:
  - **GitLab**: root `.vale.ini` sets `StylesPath = doc/.vale` and `[*.md] BasedOnStyles = gitlab_base, gitlab_docs` ([.vale.ini](https://gitlab.com/gitlab-org/gitlab/-/blob/master/.vale.ini)). The CI job `docs-lint markdown` uses an image pinned to Vale 3.21.0 and runs `scripts/lint-doc.sh`; a second job writes a Vale code-quality report ([docs.gitlab-ci.yml](https://gitlab.com/gitlab-org/gitlab/-/blob/master/.gitlab/ci/docs.gitlab-ci.yml)).
  - **Temporal** (`temporalio/documentation`): `.vale.ini` uses `Packages = Std`, `Vocab = Temporal`, `[*.{md,mdx}]` ([.vale.ini](https://github.com/temporalio/documentation/blob/main/.vale.ini)). Workflow `vale-ci.yml` runs `vale-cli/vale-action@v3.0.0` with `--config=.vale-ci.ini` on PRs touching `docs/**`, posting PR-review comments ([vale-ci.yml](https://github.com/temporalio/documentation/blob/main/.github/workflows/vale-ci.yml)).
  - **Datadog** (`DataDog/documentation`): `.vale.ini` uses styles from the separate `DataDog/datadog-vale` repo ([.vale.ini](https://github.com/DataDog/documentation/blob/master/.vale.ini)). Workflow `vale_linter.yml` installs `vale==3.9.1` on PRs and lints changed lines ([vale_linter.yml](https://github.com/DataDog/documentation/blob/master/.github/workflows/vale_linter.yml)).
  - **Grafana**: root `.vale.ini` with `BasedOnStyles = Grafana` ([.vale.ini](https://github.com/grafana/grafana/blob/main/.vale.ini)). A `vale` job in `documentation-ci.yml` runs `grafana/writers-toolkit/vale-action` in the `grafana/vale` image ([documentation-ci.yml](https://github.com/grafana/grafana/blob/main/.github/workflows/documentation-ci.yml)).

### Alternatives

- **alex**: MIT. npm latest 11.0.1 (2023-08-18); latest GitHub release 11.0.0 (2022-11-04); last commit and push 2024-11-27; not archived ([registry](https://registry.npmjs.org/alex), [repo](https://github.com/get-alex/alex)). It flags insensitive or inconsiderate wording; it is not a general term-ban linter ([registry description](https://registry.npmjs.org/alex)).
- **textlint**: MIT, actively maintained. npm latest 15.8.0 (2026-08-01); last commit 2026-10-14 ([registry](https://registry.npmjs.org/textlint), [repo](https://github.com/textlint/textlint)). Describes itself as a pluggable natural-language linter.

## Part F. Stealth wording in the field (registry metadata, read 2026-10-15)

| Package (registry) | Version, date | Registry description or summary (paraphrased or short quote) | Keywords or topics of note |
| --- | --- | --- | --- |
| `puppeteer-extra-plugin-stealth` (npm) | 2.11.2, 2023-03-01 | "Stealth mode": techniques to make headless puppeteer detection "harder" | `stealth`, `stealth-mode`, `detection-evasion` |
| `rebrowser-puppeteer` (npm) | 24.8.1, 2025-05-08 | Drop-in puppeteer replacement that passes "modern automation detection tests" | `bot-detection`, `captcha`, `scraping` |
| `rebrowser-patches` (npm) | 1.0.19, 2025-05-09 | Patches to "avoid automation detection and leaks"; also names Cloudflare and DataDome CAPTCHA pages | `stealth`, `bot-detection`, `captcha`, `cloudflare`, `datadome` |
| `patchright` (npm) | 1.64.0, 2026-10-14 | Copies Playwright's generic "A high-level API to automate web browsers" | none in registry; GitHub repo description "Undetected version of the Playwright…"; topics `undetectable`, `undetected`, `stealth`, `antidetect`, `cloudflare-bypass` |
| `patchright` (PyPI) | 1.63.0, 2026-09-20 | "Undetected Python version of the Playwright testing and automation library." | no keywords; classifiers include `Topic :: Software Development :: Testing` |
| `camoufox` (PyPI) | 0.5.8, 2026-10-06 | "Wrapper around Playwright to help launch Camoufox" | `fingerprint`, `scraping`, `firefox`, `playwright`; GitHub repo description "Anti-detect browser"; topics `antidetect`, `antidetect-browser` |
| `nodriver` (PyPI) | 0.50.6, 2026-10-08 | Summary is only a docs link | `anti-bot`, `bot-detection`, `cloudflare-bypass`, `anti-detection`, `captcha`; GitHub: "Successor of Undetected-Chromedriver", aimed at sites with "annoying anti bot systems" |
| `undetected-chromedriver` (PyPI) | 3.5.5, 2024-02-17 | Selenium Chrome replacement, "Not triggered by CloudFlare/Imperva/hCaptcha and such", with a "results may vary" disclaimer | no PyPI keywords; GitHub description claims it passes all bot-mitigation systems; topics `anti-bot`, `anti-detection`, `cloudflare-bypass` |
| `playwright-extra` (npm) | 4.3.6, 2023-03-01 | "Teach playwright new tricks through plugins." | `stealth`, `recaptcha` |

Sources: npm registry JSON for [puppeteer-extra-plugin-stealth](https://registry.npmjs.org/puppeteer-extra-plugin-stealth), [rebrowser-puppeteer](https://registry.npmjs.org/rebrowser-puppeteer), [rebrowser-patches](https://registry.npmjs.org/rebrowser-patches), [patchright](https://registry.npmjs.org/patchright), [playwright-extra](https://registry.npmjs.org/playwright-extra). PyPI JSON for [patchright](https://pypi.org/pypi/patchright/json), [camoufox](https://pypi.org/pypi/camoufox/json), [nodriver](https://pypi.org/pypi/nodriver/json), [undetected-chromedriver](https://pypi.org/pypi/undetected-chromedriver/json). GitHub repo metadata for [Kaliiiiiiiiii-Vinyzu/patchright](https://github.com/Kaliiiiiiiiii-Vinyzu/patchright), [daijro/camoufox](https://github.com/daijro/camoufox), [ultrafunkamsterdam/nodriver](https://github.com/ultrafunkamsterdam/nodriver), [ultrafunkamsterdam/undetected-chromedriver](https://github.com/ultrafunkamsterdam/undetected-chromedriver), [rebrowser/rebrowser-patches](https://github.com/rebrowser/rebrowser-patches), [berstend/puppeteer-extra](https://github.com/berstend/puppeteer-extra).

- Word counts in the README text shipped in the registry JSON are case-insensitive substring counts, so they include package names. **Stealth** wording:
  - `puppeteer-extra-plugin-stealth`: "stealth" 75, "evasion" 29, "bypass" 2.
  - `playwright-extra`: "stealth" 28, "evasion" 4.
- **Undetectable** wording:
  - `rebrowser-patches`: "undetectable" 3, "anti-bot" 2, "stealth" 2, "bypass" 1.
  - npm `patchright`: "undetectable" 1, "undetected" 1, "antibot" 1, "stealth" 2.
  - PyPI `patchright`: "undetectable" 1, "undetected" 2, "stealth" 2.
  - `nodriver`: "undetected" 6, "bypass" 1, "anti-bot" 1.
  - `undetected-chromedriver`: "undetected" 29, "anti-bot" 3.
- **Fingerprint** wording only:
  - `camoufox` (PyPI): "fingerprint" 3, none of the other terms.
  - `rebrowser-puppeteer`: none of the terms.
- Maintenance signals: puppeteer-extra repo last push 2024-07-18; rebrowser repos last push 2025-05; patchright, camoufox and nodriver pushed in October 2026 (GitHub API `pushed_at`, same repo links).

## Patterns

- Docusaurus 3.10.2 versions each docs instance separately (own command, own `<id>_versions.json`). Lockstep across instances is not documented and is done by scripting (playwright.dev's `version-all`). One instance with two folders shares one version line by construction.
- Built-in Algolia contextual search filters on language and per-instance version tags, but includes every instance's tag at once. Per-section scoping needs custom `facetFilters`, separate indexes (playwright.dev) or a local plugin's path contexts (`searchContextByPaths`).
- Reusing one source under two product names is done in the field by a pre-build generator with in-file markers (Playwright: file suffixes, `* langs:` bullets, fence languages, `%%VERSION%%`), not by Docusaurus runtime features. MDX partials with props work in body text but show raw `{props.…}` in TOC headings (open #9772).
- llms.txt v2 explicitly allows per-subpath files with most-specific-wins. The leading Docusaurus plugin documents per-version subdirectory output and scoped custom files, but not per-section subpath files.
- Registries render the README on npm, PyPI and crates.io. Testing README code in CI is established practice: Rust via `#[cfg(doctest)]` + `include_str!` (clap, env_logger), Python via `pytest-examples` (Pydantic AI).
- In registry metadata, Patchright (PyPI), undetected-chromedriver and nodriver use detection-defeat wording ("undetected", "bypass", "anti-bot"). puppeteer-extra and playwright-extra use "stealth" and "evasion". Camoufox's registry text uses only "fingerprint", while its GitHub description says "Anti-detect browser". npm Patchright's registry description is Playwright's neutral text.
