# End-to-end tests follow a catalog of numbered Journeys kept in the repository

Goodfetch's end-to-end tests are **Journeys**: goals a person reaches through Goodfetch's public surfaces, each a TOML file `journeys/J-NNN.toml` with an assigned, never-reused ID, its own Proving ground seed and tasks, the surfaces it runs on and the milestone each surface must prove it by. Each surface's own test runner implements a Journey in its own idiom, carries its ID, starts a fresh Proving ground and Goodfetch home for it, and writes the same result document as the conformance suites. The catalog is also where the owner tests of ADR 0058 take their list.

The owner asked for "end-to-end tests driven by a catalog of user journeys, each with a stable ID and its own seeded test data". The facts come from [Journey catalogs, test IDs and feature traceability](https://github.com/Jbmanllr/automan/issues/269) ([findings](https://github.com/Jbmanllr/automan/blob/research/journeys-traceability/rnd/research/2026-10-13-journeys-traceability.md)):
- **Every mature catalog lives in the repository.** Kubernetes generates `conformance.yaml` from comments above its tests; Chromium's installable web apps keep their critical user journeys as tables of numbered actions and generate the tests; Deno, Crawlee and Stagehand use a folder or file per scenario, the path being the ID.
- **Stable IDs are assigned, never derived from content, and renames are explicit.** Chromium keeps a "next: 185" counter; web-features turns a renamed ID into a `moved` stub; TestRail and Allure TestOps document that a renamed test becomes a new case.
- **A seed alone does not pin data.** proptest and Faker warn that the same seed may give other values once the generator changes.
- **Each runner carries an ID differently.** nextest has no tag predicate, only names and paths; Vitest 4.1+ rejects undeclared tags by default; pytest only with `strict_markers`; Playwright's JUnit report drops tags.
- **Data is isolated by starting from scratch**, not by cleaning up: a browser context per Playwright test, a namespace per Kubernetes test, a database per `#[sqlx::test]`.
- **Live sites make end-to-end tests flaky**: Crawlee's e2e suite crawls its own docs site and has an issue open since April 2025 asking to self-host its pages.

Decided in [Journey catalog: format, IDs, seeded data and surfaces](https://github.com/Jbmanllr/automan/issues/278), with the grilling answered by the agent on the owner's delegation. The rules every Journey test follows are in `TESTING.md`.

## Decision

### What a Journey is

- **A Journey** is one goal a person reaches through Goodfetch's public surfaces (the CLI, an SDK, MCP, the Console, an Integration), written as steps from a starting state to an outcome checked from outside the engine: by the Proving ground's oracle, by what the Proving ground observed, and by what the surface shows.
- **It is not a Proving ground task.** A task is one thing the Proving ground asks a tool to do on one page family, with an expected result (ADR 0025). A Journey uses tasks as its starting data and their oracle as its judge, and adds the surface, the steps and the checks a person makes.
- **It is not a conformance case.** A conformance case proves one capability of a contract that several targets implement (ADR 0033), and its result decides what a target may claim. A Journey proves a goal works end to end through one surface. Neither replaces the other.
- **It is not one of area 3's multi-process tests.** Those drive processes with `goodfetch-protocol`'s types, the way a Client does, to test lifetimes, `kill -9` and groups. A Journey drives a surface the way a person does.
- **A Journey is the top of the test pyramid.** It proves the goal on its happy path and the failure a person must see; edge cases, error codes and limits belong to unit, integration and conformance tests. A behaviour a lower test proves is not repeated in a Journey beyond what its goal needs.

### The catalog: files in the repository

- **One TOML file per Journey in a top-level `journeys/` folder**, named after its ID: `journeys/J-001.toml`. `journeys/catalog.toml` holds the next free number. No test-management service holds Journeys.
- **The format is defined by one Rust type** that deserializes with `deny_unknown_fields`. `xtask codegen` emits its JSON Schema to `journeys/journey.schema.json`, committed, so editors and the TypeScript and Python helpers validate the same shape. TOML is read by `toml` in Rust, `smol-toml` in TypeScript and `tomllib` in Python (standard since 3.11).
- **A Journey file holds:**

  | Field | Meaning |
  |---|---|
  | `id` | `J-` and at least three digits, equal to the file name |
  | `title` | A short name in the glossary's terms |
  | `goal` | What the person achieves and what MUST be true at the end, in RFC 2119 words, readable as a specification without the code (Kubernetes' rule for conformance descriptions) |
  | `area` | The ADR 0037 area whose spec owns it |
  | `status` | `active` or `retired`; a retired one names `replaced_by` (a list, possibly empty) and a `reason` |
  | `covers` | Feature IDs it exercises, each `<kind>:<id>`: `row:CO-18` for an ADR 0037 matrix row (parity, Stagehand v3 or pain), and the kinds [Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279) adds |
  | `[proving_ground]` | `seed` (an unsigned 64-bit integer) and `tasks`, the task IDs it uses; empty when the goal touches no page |
  | `[surfaces]` | Each surface the goal is reachable on, with the milestone (`M1` to `M4`) by which that surface must prove it |
  | `[[steps]]` | Each step in declarative words (Cucumber's rule: the wording survives a change of implementation); `on` pins a step to one surface; `manual` gives the reason a step needs a person |
  | `[[expect]]` | Each check of the outcome; `task` asks the oracle to answer `verified` for that task |

- **Surfaces**, with the first milestone that ships each (ADR 0058): `cli` and `console` (M1); `sdk-rust`, `sdk-typescript`, `sdk-python`, `mcp-stdio`, `mcp-http` and `skills` (M2); `http-listener`, `adapter-<name>` and `integration-<name>` (M4). A Journey lists every surface on which a person can reach its goal, not one per Journey. Unpinned steps run on each listed surface; a pinned step, such as "watch the run in the Console", runs on its own surface in every variant.
- **A variant** is one Journey on one surface, written `J-001@cli`. It is the unit that runs, passes and is counted.

### IDs

- **Assigned, never derived.** A new Journey takes the number in `journeys/catalog.toml` and increments it in the same pull request. Two open pull requests that take the same number conflict on that file; the one merged second takes the next number. Once on `main`, an ID never changes and is never reused.
- **The goal never changes under an ID.** Fixing words, steps, data or surfaces keeps the ID. A different goal retires the old Journey (`status = "retired"`, `replaced_by`, `reason`), whose file stays, and takes a new ID. A split retires the old one and lists both successors. This is web-features' `moved` and `split` without a separate stub file.
- **Where a test carries the ID** (selection; the result document is the record):
  - Rust (nextest): the ID, lowercased without its hyphen, is a module in the test's path (`journeys::j002::…`), since nextest selects only by name and path.
  - Vitest: the tag `J-002`, declared in the config from the catalog, with `strictTags` on.
  - pytest: `@pytest.mark.journey("J-002")`, registered, with `strict_markers` on.
  - Playwright Test: the tag `@J-002`.

### Seeded data and isolation

- **Each Journey has its own seed**, written in its file when it is created. The first catalog uses the Journey's number.
- **Each variant run starts from scratch.** It starts its own Proving ground on port 0 with the Journey's seed, its own Goodfetch home and cleared environment (ADR 0059), the `resolve` mapping of the Proving ground's host names to loopback, and the Proving ground's certificate authority trusted through ADR 0042's per-domain exception. Nothing is shared with another variant, so the Proving ground's per-task observations need no namespace.
- **The data is pinned by the seed plus the Proving ground built from the same commit.** The result document records both. A change to a page family that changes a task a Journey names fails the catalog check below, so the pull request that changes the generator updates the Journeys it moves.
- **Starting state is built through public surfaces** in the Journey's first steps: settings, Limits, a Saved identity from a Proving ground login, an earlier run. No run file, database or home folder is copied in, except an older-format file an upgrade Journey starts from (the upgrade fixtures of [Upgrade and version-mismatch testing](https://github.com/Jbmanllr/automan/issues/287)).
- **Logins** use accounts the Proving ground derives from the seed, so each variant has its own accounts.
- **Journeys run against the local copy only**, under ADR 0043's outbound deny. Online-copy results stay conformance reports.
- **Journeys drive the shipped artefacts**: the `goodfetch` binary and the SDK packages built from the commit, never an engine linked into the test.

### Runners and results

- **Each surface's own runner runs its variants**: nextest for `cli` and `sdk-rust`, Vitest for `sdk-typescript`, pytest for `sdk-python`, Playwright Test for `console`. The MCP, skills, adapter and Integration runners are those their own tickets choose. Each surface keeps its Journey tests in its own package; where exactly is [Test layout, naming, shared helpers and the test-first process](https://github.com/Jbmanllr/automan/issues/277)'s.
- **Each variant run writes one result document** (area 2's, built by [`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/Jbmanllr/automan/issues/169)) with `kind = "journey"`. It records the variant, each step's outcome (`passed`, `failed`, `not_reached`, `manual`), each task's oracle answer with its evidence, the Proving ground's version and seed, the binary's and the surface package's versions, and the platform and runtime.
- **The outcome is derived, never set by the test**, in area 2's words:
  - `proved` when every automated step passed, every check passed and every named task is `verified`;
  - `failed` when any of them failed, an oracle answer is `incomplete`, `false_success` or `failed`, or a step was not reached;
  - `unproved` for a declared variant with no test or no run, computed by the catalog report from the absence of a document.

  A `manual` step leaves the variant `proved` on its automated steps and is listed for the owner test.

### The catalog check and report

The "Test infrastructure" spec builds them. The check fails when:
1. a file does not parse under the type, or its name differs from its `id`;
2. two Journeys share an ID, or an ID is not below the next free number;
3. a task a Journey names is not in the Proving ground's task list for its seed;
4. a test carries the ID of a retired or unknown Journey, or a surface the Journey does not list, or two tests carry the same variant;
5. a variant's milestone is earlier than its surface's first milestone;
6. a retired Journey has no `reason`.

The report lists each variant's latest outcome on `main` by milestone and area. Whether `covers` names existing features, and the "every feature is tested" check, are [Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279)'s.

### Ownership and review

- **Each area's `/to-spec` adds the Journeys of its user stories** to the catalog, in its `/to-tickets` pull request or the first ticket's, and `/to-tickets` names, in each ticket, the variants that ticket makes `proved`. A variant without a test is `unproved` until then.
- **Catalog changes are reviewed like code** in the pull request that makes them; the fresh review checks that no goal changed under an existing ID.

### Owner tests

- **The owner test of M1, M2 and M3 lists Journeys**: its "Journeys to try" are the variants whose milestone is that milestone, by ID and title, with their manual steps. The owner runs them by hand from a local build, and also tries anything else.
- **An owner test is ready only when every automated variant of its milestone is `proved` on `main`**, as the CTS Verifier runs only on a device that passed the automated suite.
- **A finding the owner makes on a goal no Journey covers adds a Journey**, in the finding's issue.

### The first catalog: M1's Journeys

Each Journey below lists its later surfaces too. Families are the Proving ground's; `catalog` is the first one, built by [Proving ground tracer: one seeded family, its task list and its oracle](https://github.com/Jbmanllr/automan/issues/158). The scenarios of [M1 owner test: a fully working extraction and crawl framework](https://github.com/Jbmanllr/automan/issues/289) map one to one onto J-001 to J-010.

| ID | Title | Surfaces (milestone) | Proving ground | Covers |
|---|---|---|---|---|
| J-001 | Start the Daemon and open the local Console with no login; the system browser opening is a manual step | `cli`, `console` (M1) | none | `row:SDK-01`, `row:CN-03` |
| J-002 | Crawl a site over HTTP to Markdown and export its Dataset as JSON, CSV and Parquet | `cli` (M1); `sdk-rust`, `sdk-typescript`, `sdk-python`, `mcp-stdio`, `mcp-http` (M2) | `catalog` | `row:SA-01`, `row:CO-18`, `row:CO-25`, `row:CO-27`, `row:CO-29` |
| J-003 | Crawl a JavaScript-rendered site with browser rendering, with screenshots and a PDF of each page | `cli` (M1); the SDKs and MCP (M2) | [rendered and interactive pages](https://github.com/Jbmanllr/automan/issues/166) | `row:SA-05`, `row:CO-15`, `row:CO-20`, `row:CO-21`, `row:SA-09` |
| J-004 | Follow a running Crawl in the Console: Runs, the run's drawer with its three axes, Datasets, Engine & settings | `console` (M1), the Crawl started by a step pinned to `cli` | `catalog` | `row:CN-03`, `row:CN-04`, `row:RT-15` |
| J-005 | Resume a Crawl after the Daemon is killed, with no Record delivered twice | `cli` (M1); the SDKs (M2) | `catalog` | `row:RT-10`, `row:RT-11`, `row:durability-resume` |
| J-006 | Stop a run at an install Limit and see it in `goodfetch usage` and the Console | `cli`, `console` (M1) | `catalog` | `row:CO-08`, `row:cost-unpredictable-runaway` |
| J-007 | Two Crawls of one site share Politeness, never more than its in-flight cap together | `cli` (M1) | [Crawl families](https://github.com/Jbmanllr/automan/issues/164) | `row:CO-09`, `row:compliance-site-owner-politeness` |
| J-008 | Crawl through a proxy, see a Block cause reported and the Session pool react | `cli`, `console` (M1) | [Crawl families](https://github.com/Jbmanllr/automan/issues/164) and the forward proxy | `row:SA-22`, `row:ST-08`, `row:CO-12` |
| J-009 | See where each setting's value comes from | `cli` (`goodfetch config explain`), `console` (Engine & settings) (M1) | none | `row:RT-22` |
| J-010 | Run a Crawl and see nothing leave the machine beyond its targets and pinned downloads | `cli` (M1) | `catalog` | `row:SC-10` |
| J-011 | Extract fields and page metadata (JSON-LD, Open Graph) from product pages into Records | `cli` (M1); the SDKs and MCP (M2) | [Extraction families](https://github.com/Jbmanllr/automan/issues/165) | `row:CO-14`, `row:CO-19`, `row:data-extraction-quality` |
| J-012 | Move a Checkpoint to another Goodfetch home and finish the run there | `cli` (M1); the SDKs (M2) | `catalog` | `row:RT-10` |

Pain slugs are written as `row:` because they are rows of ADR 0037's matrix. The M1 specs of areas 4, 5, 6 and 12 add their own Journeys when they are written.

### What the M1 tracers implement

**[Proving ground tracer: one seeded family, its task list and its oracle](https://github.com/Jbmanllr/automan/issues/158):**
- The family is `catalog`, ported from the lab's HTTP catalog: paginated listing pages and product pages, each product with its ID, name and price.
- **Task IDs** are `<family>/<ordinal>` (`catalog/1`), the ordinal counting from 1 in the task list's order, the same for the same Proving ground version and seed. This is the concrete form of area 2's "seed-derived id": the seed scopes it, and the seed travels with it.
- **The task list** is served as JSON: `proving_ground` (`version`, `seed`) and `tasks`, each with `id`, `family`, `kind` (`crawl` or `extract` for this family), `start_url` and `description`.
- **`serve` binds port 0 when asked**, and its first output line is one JSON object with `address`, `version` and `seed`, so a test helper reads it.
- **The version** is the workspace version plus the commit, so the result document pins the generator.
- **The seed** is an unsigned 64-bit integer, written in decimal.
- **The oracle's answer** names the task ID it judged.
- A test shows the same seed gives the same task list, IDs and start URLs included.

**[`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/Jbmanllr/automan/issues/169):**
- The result document has a `kind`: `conformance` or `journey`.
- A `journey` document carries the variant (`journey_id`, `surface`), each step's outcome, each task's oracle answer and evidence, and the versions of the binary and the surface package in place of a target. The common fields (platform, runtime, the Proving ground's version and seed) stay.
- Both kinds derive their outcome from what they hold, through a constructor in `goodfetch-protocol`; a test cannot write `proved` beside a `false_success` answer.
- Tests show a sample `journey` document validates against the schema, and that one task answering `false_success` makes it `failed`.

### IDs that feature coverage references

[Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279) counts by: the Journey ID (`J-NNN`), the variant ID (`J-NNN@<surface>`), the surface names above, and each Journey's `covers` list. It owns the feature-ID kinds beyond `row:` and the check that every feature has a test.

## Considered Options

- **A test-management service** (Qase, TestRail, Allure TestOps): rejected. The Proving ground and benchmark are published so anyone can rerun them (ADR 0025), and a catalog in a service would be invisible to them. Its reviews happen outside the pull requests that autonomous delivery reviews (AGENTS.md); Qase approves case reviews only in its UI, and TestRail versions cases only in its Enterprise plan. Every CI job runs under ADR 0043's deny, and an uploader would be one more named exception. The services' strength, recording manual runs, is covered by the owner test's issue.
- **Metadata in test code with a generated list** (Kubernetes): rejected. A Journey runs in four or more runners (nextest, Vitest, pytest, Playwright Test), so no single runner's code can hold it, and the steps a person follows would be split across languages.
- **Executable Gherkin** (Cucumber, cucumber-rs, pytest-bdd): rejected. Gherkin has no ID keyword, and step definitions would be written and kept in step with each other in three languages. The declarative step wording is kept without the glue.
- **Tests generated from the catalog** (Chromium's dPWA): rejected. Chromium generates one language's tests from shared actions; Goodfetch's surfaces differ in idiom (an SDK call, a CLI line, a Console click), so each surface writes its own test against the same steps.
- **A folder per Journey, the path being the ID** (Deno, Crawlee): rejected. A path names the content, so a better name means a rename, which is how TestRail and Allure lose history.
- **Kebab-case slugs as IDs** (web-features): rejected. A Journey spans areas, and its title grows with each surface and feature; a number never needs to move.
- **A revision in each ID** (OpenFastTrace's `dsn~name~2`): rejected. Retiring the Journey and assigning a new number gives the same voiding with plain IDs.
- **One Proving ground shared by every Journey in a job, with a namespace per run** (wptserve's stash): rejected. A fresh instance starts in milliseconds and leaves no observation shared, which is the "start from scratch" rule.
- **One seed for the whole catalog:** rejected. The owner asked for each Journey's own seeded data, and a catalog-wide seed would couple every Journey to every generator change.
- **A separate Journey result format, or JUnit properties:** rejected. Area 2's result document is already written by every runner ([Conformance result documents from the TypeScript and Python test runners](https://github.com/Jbmanllr/automan/issues/170)); Playwright's JUnit drops tags.
- **The owner tests kept as free text:** rejected. Their scenarios would drift from the automated Journeys and never be counted.

## Consequences

- **Adds to ADR 0032:** the `journeys/` folder at the root of the repository.
- **Adds to ADR 0058:**
  - each owner test lists its milestone's variants by Journey ID as "Journeys to try", with their manual steps;
  - it is ready only when it is the only open issue of its milestone and every automated variant of that milestone is `proved` on `main`;
  - a finding on an uncovered goal adds a Journey.

  [M1 owner test: a fully working extraction and crawl framework](https://github.com/Jbmanllr/automan/issues/289)'s "Scenarios to try" become "Journeys to try" with J-001 to J-010, plus J-011 and J-012.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/Jbmanllr/automan/issues/152)):
  - a task's "seed-derived id" is `<family>/<ordinal>`, scoped by the seed;
  - `serve`'s first output line is the JSON object above, and it binds port 0;
  - the result document has a `kind`, and its `journey` kind is written by every surface's runner, the Vitest and pytest helpers of [Conformance result documents from the TypeScript and Python test runners](https://github.com/Jbmanllr/automan/issues/170) included;
  - outcomes are derived by a constructor, never set.

  ADR 0037's area table lists this ADR in area 2's row.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)): its "Scenarios" that need several processes or machines are called multi-process tests, since *scenario* is an `_Avoid_` term for Journey.
- **The glossary** gains *Journey* and *Proving ground task*.
- **Left to other tickets of the test strategy map,** each building on the interface above:
  - which tier runs which variants, and the local command that selects them by ID, surface, milestone or area: [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276);
  - the Journey tests' folders and the helper that starts a variant's Proving ground, home and engine: [Test layout, naming, shared helpers and the test-first process](https://github.com/Jbmanllr/automan/issues/277);
  - the feature inventory and the coverage check: [Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279);
  - what a failing or flaky Journey test does: [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280);
  - Journeys of AI features and the `skills` surface, on scripted models: [Testing AI features and agent skills](https://github.com/Jbmanllr/automan/issues/281);
  - the `console`, SDK, CLI, MCP, adapter and Integration runners: [Console and docs test stack](https://github.com/Jbmanllr/automan/issues/282), [SDK and CLI test stack](https://github.com/Jbmanllr/automan/issues/285), [MCP, framework adapter and Integration test stack](https://github.com/Jbmanllr/automan/issues/286).
- **The "Test infrastructure" spec** builds the Journey type and its schema, the catalog check and report, the twelve files of the first catalog, and each language's helper that reads a Journey, starts its Proving ground and writes its result document.
