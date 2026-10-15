# One docs site renders each Product's section from one source, and neither Product leads with access

ADR 0084 made Collie and Foxhound two Products under the Goodfetch brand, Foxhound's executable containing Collie's code, and ADR 0090 gave them their names and one Product version. This decision sets how the documentation and `goodfetch.dev` present them.

ADR 0030's site at `goodfetch.dev/docs` gets a section per Product, `/docs/collie/` and `/docs/foxhound/`. Each section is rendered from one source with its own Product's names, so Foxhound's docs contain Collie's as its executable contains Collie's code. A chooser at the docs root asks one question. Each Product leads with what Goodfetch guarantees and runs locally, never with stealth or access. Migration guides follow every leader the SDK API was judged against and every parity reference of ADR 0037, and ADR 0035's wording policy reaches every text published about either Product, under a check.

The [docs-site, migration-guide and stealth-wording research](https://github.com/goodfetch/automan/blob/research/docs-positioning/rnd/research/2026-10-15-docs-positioning.md) found, on 2026-10-15:
- **Vendors join several products' docs under one shell.**
  - Crawlee runs two Docusaurus builds on one domain, joined by a switcher.
  - Astral runs three MkDocs builds under `docs.astral.sh`.
  - Browser Use and HashiCorp put each product under its own path in one site, with a product switcher.
  - playwright.dev generates four language sites from one source, before the build.
- **"Which one" pages exist where products overlap.** Examples: Browser Use's "Which Browser Use do I need?", Playwright's page comparing Library and Test, Stagehand's local-versus-Browserbase table, HashiCorp's "Terraform versus alternatives".
- **A crawler's docs link out to the driver's page API.** Crawlee does not document the `Page` it hands to handlers, and on 2026-10-15 its links into Stagehand's reference returned 404 after Stagehand's v4 docs moved.
- **Migration guides from competitors share a skeleton, and their code never runs in CI.** The skeleton is principles, a mapping table and before/after code; Stagehand's 2026 guides add porting rules for coding agents. Examples come from Playwright, Crawlee for Python, Stagehand and ty. The field lints or type-checks guide code at most.
- **Stealth tools put detection-defeat words in their registry metadata.** Examples: "Undetected", `cloudflare-bypass`, `anti-bot`. Camoufox's registry text says only "fingerprint".

Decided in [Docs site and positioning of two Products](https://github.com/goodfetch/automan/issues/499), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447), by the agent on the owner's delegation of 2026-10-14 ([round 1](https://github.com/goodfetch/automan/issues/499#issuecomment-6181529601), [grilling record](https://github.com/goodfetch/automan/issues/499#issuecomment-6181537258)). It builds on ADR 0088 (what Collie carries) and ADR 0090 (names and the Product version), and also on:
- the [browser-automation products research](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md);
- the [naming and packaging research](https://github.com/goodfetch/automan/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) §5;
- the [stealth legal note](https://github.com/goodfetch/automan/blob/research/stealth-legal-exposure/rnd/research/2026-10-10-stealth-legal-exposure.md).

## Decision

### One site, a section per Product

- **The docs stay one site at `goodfetch.dev/docs`** (ADR 0030), with a section per Product:

  | Path | Holds |
  |---|---|
  | `goodfetch.dev/docs/` | The chooser, the release notes and the benchmark results, each item naming its Product |
  | `goodfetch.dev/docs/collie/` | Collie's docs |
  | `goodfetch.dev/docs/foxhound/` | Foxhound's docs, Collie's included |

- **Both sections always show the same Product version.** Both Product trees sit in one Docusaurus docs instance, so the version cut of ADR 0030's release pull request copies both. Docusaurus versions each docs instance with its own command, while one instance's folders share its one version line.
- **A header switcher moves between the Products,** keeping the page when the other section has it.
- **Search returns the reader's section and the root pages only.** Docusaurus's contextual search narrows results by language and version, and includes every docs instance's tag at once, so it never scopes by section. The scoping comes from path contexts or facet filters. Which search tool, and how it scopes, is area 1's.
- **Only the root holds pages outside the two sections.** The glossary has no collective term for the pieces both Products carry (ADR 0084), and no "platform" section collects them.

### One source, rendered per Product

- **Each docs page is written once, and rendered into the section of each Product it documents, with that Product's names.** Those are the Product's name ("Goodfetch Collie" at first mention, ADR 0090), its executable, packages, imports, connection type, engine-path variable, installer, image, skill and MCP server names. Prose is written to read correctly in both sections.
- **A page declares its Products by the rule that marks the Operation catalog:**
  - what Collie carries renders in both sections, since Foxhound contains Collie (ADR 0088);
  - what only Foxhound carries renders in Foxhound's;
  - a page written for one Product's reader (its getting started, a migration guide, Collie's "When you need many pages") declares that Product;
  - a part of a page may declare its own Products.

  How the catalog marks each entry is [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/automan/issues/458)'s.
- **Foxhound's section groups what it takes from Collie under a heading that names Collie,** and each such page says that it is Collie's. A Foxhound reader thus learns that a handler's page is a Collie page (ADR 0088). Collie's section shows nothing of Foxhound's but "When you need many pages".
- **Reference pages are generated per Product** from the marked catalog and settings schema (ADR 0030).
  - Each section lists its own Product's entries, so Foxhound's lists Collie's too.
  - Each page names the Product its entry belongs to.
  - A setting only Foxhound honours appears only in Foxhound's reference.
- **A generator renders each Product's tree before the build,** as playwright.dev renders each language's tree from `microsoft/playwright/docs/src`.
  - MDX partials with props are not used: a prop in a heading shows raw in Docusaurus's table of contents ([facebook/docusaurus#9772](https://github.com/facebook/docusaurus/issues/9772)).
  - The generator's markers, and where its output is written, are area 1's.

### Choosing a Product

- **A chooser page at the docs root asks one question:** does your program or agent decide which pages to visit (Collie), or does the run discover pages and keep what it gets from each (Foxhound)?
- **A table of tasks follows:**
  - Collie: fill a form, read the page a program reached, let an Agent reach a goal, repeat an automation on a Schedule.
  - Foxhound: crawl a site, collect Records into a Dataset, deliver them to a Sink, track what changed across a crawl's Records.
- **Then it says:**
  - Foxhound contains Collie, so one program never needs both.
  - Two programs may each use one, and both installed collide nowhere (ADR 0090).
  - What two installs share is [Collie and Foxhound installed on one machine](https://github.com/goodfetch/automan/issues/486)'s.
- **The marketing site's home asks the same question,** and each Product's docs home links the chooser and the other Product.
- **Collie's "When you need many pages" shows the move to Foxhound.** The import, the connection type and the executable's name change; page code, settings, and commands with their flags and output do not (ADR 0090; how the SDKs share Collie's code is #458's).

### What each Product leads with

- **Each Product leads with what Goodfetch guarantees and runs on the user's machine.**
  - **Collie:**
    - a Playwright-shaped page API (ADR 0019);
    - AI actions whose Effects are journaled (ADR 0010), whose Plans replay without a model (ADR 0009) and whose result carries a Verdict (ADR 0029);
    - a Script sandbox for model-written code (ADR 0051);
    - the Console, Schedules, the stealth catalog and Clean-up rules, which run locally where competitors keep them in their clouds (ADR 0088).
  - **Foxhound:** crawls that are polite by default (ADR 0086), resume from Checkpoints (ADR 0002), and deliver Records under each Sink's guarantee with their Completeness reported (ADRs 0005 and 0008). Every page it renders is a Collie page.
  - What is said where (taglines, READMEs, the landing pages) is copy, written under these rules.
- **No opening leads with stealth, blocks, challenges or access.** An opening is a tagline, a registry description, a README's first section, a docs home or a landing page's first screen.
  - Stealth appears among the capabilities, in ADR 0035's terms.
  - No page promises access to a site (ADR 0037, pain 1).
  - The legal note makes marketing its own prong of circumvention law: §1201(a)(2)(C), InfoSoc 6(2)(a), L.335-3-1 II 4°.
- **No separate "by Goodfetch" credit is needed.** Each Product's first mention already carries the brand (ADR 0090), and the site's header and footer belong to Goodfetch.
- **Goodfetch Cloud is absent from both Products' docs until it exists.**
  - Afterwards, it is one deployment target beside ADR 0049's, never the default or the recommended path.
  - No capability is documented as needing it (ADRs 0001 and 0030).
  - Stagehand's docs recommend Browserbase and show stealth as cloud-only, while Crawlee lists Apify beside AWS and GCP. Goodfetch takes Crawlee's side.
- **Claims about competitors appear only in migration guides and benchmark results,** each with the competitor's version, the date and a source. Performance figures follow ADR 0025.
- **The marketing site presents Goodfetch with both Products side by side,** with a landing page per Product at `goodfetch.dev/collie` and `goodfetch.dev/foxhound`, each linking its docs section. It lives in its own repository (ADR 0030), under the same rules.

### Migration guides

- **V1 ships a guide from each leader a golden scenario is set against, and from each parity reference of ADR 0037, plus Crawlee with Stagehand:**

  | Product | Guides |
  |---|---|
  | Collie | From Playwright; from Stagehand (v4, with a part for v3's `cacheDir`, `llmClient` and `env`); from Browser Use |
  | Foxhound | From Crawlee (TypeScript and Python); from Scrapy, scrapy-playwright included; from Spider (Rust); from Crawlee with Stagehand |

  - Each guide sits in its target Product's section.
  - Crawlee's Stagehand integration accepts Stagehand 3 only, though Stagehand 4.0.0 shipped on 2026-08-10. The Crawlee-with-Stagehand guide therefore covers ADR 0037's V3 rows: a Foxhound handler's page has Collie's AI actions (ADR 0088).
  - A guide from another tool (Puppeteer, Selenium, Firecrawl, Crawl4AI) can be added in any minor, as docs only.
- **A guide has six parts:**
  1. **Concepts:** what changes in them.
  2. **Mapping table:** the competitor's API against Goodfetch's, linked to the reference, with a blank cell and its reason where nothing matches.
  3. **Programs:** before and after, side by side, in each language both offer.
  4. **Defaults that change results:**
     - robots.txt obeyed (ADR 0086);
     - consent refused (ADR 0048);
     - Clean-up rules on (ADR 0047);
     - an uncertain Effect never repeated (ADR 0010);
     - a Session the program opened never rotated (ADR 0088).
  5. **Porting rules for a coding agent.**
  6. **The competitor's version and date.**
- **The "after" code is docs samples, so it runs in CI** (ADR 0068).
- **The "before" code is type-checked, never run.** Each "before" program is a file beside its guide, with a manifest pinning the competitor's version, type-checked against it (tsc, pyright, rustc) as golden scenarios' leader programs are. The page includes the file rather than a fenced block, so it is not a docs sample. A guide a golden scenario backs reuses that scenario's programs.
- **The Live check `migration-guides-current`,** owned by area 11, run nightly (or before each release candidate while the nightly tier is off, as ADR 0068's `docs-external-links`) and blocking nothing directly (ADR 0064), reads the registries. It fails when a guide's competitor has a newer major than the one pinned, or a newer minor for a `0.y` competitor such as Browser Use, since ADR 0030 counts a `0.y` minor as a major. It only reads registries, so it costs nothing (ADR 0076).

### Per-Product artifacts

- **The repository's README presents Goodfetch and both Products:** the chooser's question, each Product's install line and links to each section.
- **Each published package's README is its Product's.** It gives:
  - the Product's first-mention name and its one-sentence glossary definition;
  - the install line and one minimal example;
  - links to its section and the chooser.

  It is what npm, PyPI and crates.io render.
  - Collie's client package's README says that a program installs a Product's package and that only a library serving both Products uses the client package (#458's). Platform packages carry one line pointing to their Product's package.
  - README code is docs samples, extracted and run like the docs. A Rust crate's README is included under `#[cfg(doctest)]`, as clap and env_logger include theirs.
  - Per-Product GitHub repositories remain a later option (ADR 0084).
- **Agent skills are rendered from the same source as the docs,** one source per topic for each Product that carries it, as #458 decides for skills, so their topics match the pages.
- **Each executable embeds its own section's rendering** for its `docs` command and its MCP server's resources (ADR 0030), and its `init` writes an AGENTS.md index pointing to them. Which data each executable embeds is [How the products are built, and each product's Size budget](https://github.com/goodfetch/automan/issues/454)'s.
- **Each section has its own llms files:** `/docs/collie/llms.txt` and `llms-full.txt`, and Foxhound's likewise.
  - The root has its own pair: `/docs/llms.txt` lists both sections' files and the root pages, and `/docs/llms-full.txt` holds the root pages' content. llms.txt v2 allows a file at any subpath, and the most specific file applies.
  - Every page's `.md` twin, llms entry and `llms-full.txt` content are checked in its own section's files, or the root's for a root page.
- **Samples.**
  - **A page rendered for both Products runs its samples once, on Collie's executable,** as a test runs on Collie unless it needs Foxhound ([Release, signing and testing of two products](https://github.com/goodfetch/automan/issues/459)).
  - **Its Foxhound rendering is checked statically against Foxhound's packages, never run:** TypeScript type-checked by Twoslash, Python linted by Ruff and type-checked by pyright (beyond the lint ADR 0068 gives a `no_run` Python block), Rust compiled as a `no_run` doctest. A wrong import or name in Foxhound's rendering fails.
  - **A page rendered for Foxhound only runs its samples on Foxhound's executable.**
  - **`xtask docs-samples` reads each Product's rendering.**

### Wording

- **ADR 0035's wording policy covers every text published about either Product.** That includes texts it did not name: registry descriptions and keywords, release notes, the repository's description and topics, and the marketing site. Collie carries stealth and the Challenge solver (ADR 0088), so stealth pages render in both sections.
- **A lint enforces the list,** run by `just lint` in the Quick suite, so every later suite runs it too (ADR 0061).
  - It is an `xtask` check, word-bounded and case-insensitive, over prose. It skips code, since third-party APIs fix some names.
  - It reads:
    - the docs pages (not `docs/adr/`, `docs/agents/` or `rnd/`), the READMEs and the skills;
    - the descriptions in the catalog and the settings schema, and the Error codes' messages;
    - the Console's message catalog;
    - every package's description and keywords.
  - Its list is one file: "bypass" in any form, "undetectable", "undetected" and "solve any CAPTCHA". Counsel's review may extend it.
  - It is not Vale: ADR 0068 keeps Vale out, and a list of four phrases needs no prose linter.
- **The marketing site's repository keeps the same list.**
- **Counsel's review before V1** (ADR 0035) covers both Products' stealth pages, the registry metadata and the marketing site.

## Considered Options

- **Product-first paths** (`goodfetch.dev/collie/docs`, as Crawlee serves `/js/docs`): rejected. A rewrite per Product, and a change to ADR 0030's address, for no reader gain.
- **Subdomains** (`collie.goodfetch.dev`): rejected. Two hosts to configure for what paths give, since llms.txt allows a file per subpath.
- **A domain per Product,** as Vercel's products each have: rejected. COLLIE and FOXHOUND are registered marks of other holders (ADR 0084), and a domain costs money (ADR 0076).
- **A section per Product as two docs instances or two builds,** as Crawlee builds JavaScript and Python apart: rejected. Docusaurus cuts each instance's versions with its own command, so one Product version would need a script to keep them together, as playwright.dev's `version-all` does.
- **A shared section** for the pieces both Products carry, with one Product's names as the example: rejected.
  - A Foxhound reader would copy `goodfetch-collie` commands and `Collie` imports, and an agent reading Foxhound's offline docs would run the wrong executable.
  - Foxhound contains Collie, so the section would hold most of Collie's docs, under a collective name the glossary does not have.
- **Collie's section only, with Foxhound's linking into it,** as Crawlee links to playwright.dev for `Page`: rejected. It has the same wrong names, and Foxhound's offline docs would carry Collie's pages.
- **A Product tab on every sample,** beside the language tabs: rejected. Every `.md` twin would carry both Products, and a reader would choose twice per block.
- **Cards or a menu only, without a chooser,** as Astral's landing cards and Vercel's shared menu show their products: rejected. Their products complement each other; Collie and Foxhound overlap, since one contains the other, and that is where the field writes a chooser.
- **One getting-started that installs Foxhound for everyone,** as Playwright's starts with the test runner, which contains the library: rejected. A user who drives a page would install the crawler, DuckDB and Sinks, which ADR 0084 rejected.
- **Leading with access** ("never blocked", "undetected"), as stealth tools' registry pages do: rejected. It is the "marketed for circumvention" prong the legal note names, and Goodfetch cannot promise access to any site (ADR 0037, pain 1).
- **Leading with AI,** as Stagehand and Browser Use frame themselves: rejected. It is not what sets Collie apart; the journal, verified Plans and Verdicts are, and they are what Collie's AI actions lead with.
- **Goodfetch Cloud as the recommended deployment,** as Stagehand's docs recommend Browserbase: rejected. ADR 0001 keeps self-hosting at parity, and ADR 0030 keeps the Cloud's docs apart.
- **Only the five guides the ticket listed:** rejected. It misses Browser Use, the leader of a golden scenario, and Spider, a parity reference.
- **Guides from Puppeteer, Selenium, Firecrawl and Crawl4AI too:** rejected for V1. No golden scenario backs them. Collie's page API is Playwright-shaped, so Playwright's own guide from Puppeteer bridges Puppeteer users.
- **Running the "before" code of guides:** rejected. Running it would drive real browsers and call live models in CI for a competitor's program; type-checking against the pinned version keeps it honest at no cost.
- **Running each sample of a shared page on both executables:** rejected. A test runs once, on Collie unless it needs Foxhound (#459), and Foxhound's rendering differs only in names, which the type checks and the compile catch.
- **The wording policy kept by review only:** rejected. It would miss registry metadata, where the field's defeat wording lives, and would drift as agents write docs.
- **Vale for the wording check,** as GitLab, Temporal, Datadog and Grafana run it: rejected. It amends ADR 0068 to enforce four phrases.

## Consequences

- **Amends ADR 0030.**
  - Reference pages are generated per Product, an entry rendering in each section whose Product carries it.
  - Each Product has its own llms files, offline docs and skills, rendered from one source.

  Notes sit under its reference pages and its "For agents" item.
- **Amends ADR 0035.** The wording policy covers every text published about either Product, its list gains "undetected" and every form of "bypass", a lint enforces it, and counsel's review before V1 covers both Products' stealth pages, the registry metadata and the marketing site. Notes sit under the policy and under its counsel review.
- **Amends ADR 0068.** The build-output test and `docs-deploy-smoke` check each section's llms files and the root's, samples run per rendering (a shared page's once, on Collie's executable), a Foxhound rendering's Python is also type-checked by pyright, and a migration guide's "before" programs are included files, not samples. Notes sit under its build-output test, its deploy smoke check, its `no_run` rule and its samples' binary.
- **ADR 0037.**
  - Matrix row SDK-07 and pain row 8 cite this ADR.
  - The sentence under the Stagehand v3 rows places the Crawlee-with-Stagehand guide in Foxhound's docs.
  - Areas 1 and 11 list this ADR.
- **`TESTING.md`.** Its docs-site rules check llms files per section and the root, and run the wording lint; its docs-sample rules run a shared page's samples on Collie's executable, check Foxhound's rendering statically and keep guides' "before" programs out of the samples; its Live checks list `migration-guides-current`.
- **Glossary.** No new term. The sections are named after their Products, and the glossary's _Avoid_ "platform" holds.
- **Areas.**
  - Area 1 (Repository, CI and release) builds the sections, the generator, the switcher, scoped search, per-section llms files and the wording check.
  - Area 11 (Client surfaces) builds the migration guides with their Live check, the package READMEs, the rendered skills and each executable's offline docs.
- **Owner steps.** None. This decision changes nothing about `goodfetch.dev`, the marketing site's repository (ADR 0030) or the trademark steps of ADRs 0034 and 0084, and the agent registers and pays for nothing (ADR 0076).
- **Downstream.**
  - [Apply the product-line decisions to the affected specs and tickets](https://github.com/goodfetch/automan/issues/460) carries this into area 1's and area 11's specs and tickets.
  - [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461) writes the guides' programs from its golden scenarios.
