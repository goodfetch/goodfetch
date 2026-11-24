# End-to-end tests follow a catalog of numbered Journeys kept in the repository

Goodfetch's end-to-end tests are **Journeys**: goals a person reaches through Goodfetch's public surfaces, each a TOML file `journeys/J-NNN.toml` with an assigned, never-reused ID, its own Proving ground seed and tasks, the surfaces it runs on and the milestone each surface must prove it by. Each surface's own test runner implements a Journey in its own idiom, carries its ID, starts a fresh Proving ground and Goodfetch home for it, and writes the same result document as the conformance suites. The catalog is also where the owner tests of ADR 0058 take their list.

The owner asked for "end-to-end tests driven by a catalog of user journeys, each with a stable ID and its own seeded test data". The facts come from [Journey catalogs, test IDs and feature traceability](https://github.com/goodfetch/goodfetch/issues/269) ([findings](https://github.com/goodfetch/goodfetch/blob/research/journeys-traceability/rnd/research/2026-10-13-journeys-traceability.md)):
- **Every mature catalog lives in the repository.** Kubernetes generates `conformance.yaml` from comments above its tests; Chromium's installable web apps keep their critical user journeys as tables of numbered actions and generate the tests; Deno, Crawlee and Stagehand use a folder or file per scenario, the path being the ID.
- **Stable IDs are assigned, never derived from content, and renames are explicit.** Chromium keeps a "next: 185" counter; web-features turns a renamed ID into a `moved` stub; TestRail and Allure TestOps document that a renamed test becomes a new case.
- **A seed alone does not pin data.** proptest and Faker warn that the same seed may give other values once the generator changes.
- **Each runner carries an ID differently.** nextest has no tag predicate, only names and paths; Vitest 4.1+ rejects undeclared tags by default; pytest only with `strict_markers`; Playwright's JUnit report drops tags.
- **Data is isolated by starting from scratch**, not by cleaning up: a browser context per Playwright test, a namespace per Kubernetes test, a database per `#[sqlx::test]`.
- **Live sites make end-to-end tests flaky**: Crawlee's e2e suite crawls its own docs site and has an issue open since April 2025 asking to self-host its pages.

Decided in [Journey catalog: format, IDs, seeded data and surfaces](https://github.com/goodfetch/goodfetch/issues/278), with the grilling answered by the agent on the owner's delegation. The rules every Journey test follows are in `TESTING.md`.

## Decision

### What a Journey is

- **A Journey** is one goal a person reaches through Goodfetch's public surfaces (the CLI, an SDK, MCP, the Console, an Integration), written as steps from a starting state to an outcome checked from outside the engine: by the Proving ground's oracle, by what the Proving ground observed, and by what the surface shows.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no suite, test layout, Journey surface, security boundary, Performance budget, Feature kind (`screen:`) or actor exists for the Console.
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
  | `covers` | Feature IDs it exercises, each `<kind>:<id>`: `row:CO-18` for an ADR 0037 matrix row (parity, Stagehand v3 or pain), and the kinds [Feature coverage and traceability](https://github.com/goodfetch/goodfetch/issues/279) adds |
  | `[proving_ground]` | `seed` (an unsigned 64-bit integer) and `tasks`, each Proving ground task it uses as `{ id, digest }`; the whole table is left out when the goal touches no page |
  | `[surfaces]` | Each surface the goal is reachable on, with the milestone (`M1` to `M4`) by which that surface must prove it |
  | `[[steps]]` | Each step in declarative words (Cucumber's rule: the wording survives a change of implementation); `on` pins a step to one surface; `manual` gives the reason a step needs a person |
  | `[[expect]]` | Each check of the outcome; `task` asks the oracle to answer `verified` for that task; `surfaces` scopes it to those variants (agent decision as built, spec #325's review M9: an SDK package's install checks belong to the `sdk-*` variants only, so J-002's `cli` variant never proves them) |

  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): a Journey also names its `product`: Collie when Collie carries every step, Foxhound otherwise.

- **Surfaces**, with the first milestone that ships each (ADR 0058): `cli` and `console` (M1); `sdk-rust`, `sdk-typescript`, `sdk-python`, `mcp-stdio`, `mcp-http` and `skills` (M2); `http-listener`, `adapter-<name>` and `integration-<name>` (M4). A Journey lists every surface on which a person can reach its goal, not one per Journey. Unpinned steps run on each listed surface; a pinned step, such as "watch the run in the Console", runs on its own surface in every variant.
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): a Journey's surfaces are its Product's. Each Collie Journey with a `cli` variant also runs it on Foxhound's executable as `J-NNN@foxhound-cli`, the only variant that names a Product.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **A variant** is one Journey on one surface, written `J-001@cli`. It is the unit that runs, passes and is counted.

### IDs

- **Assigned, never derived.** A new Journey takes the number in `journeys/catalog.toml` and increments it in the same pull request. Two open pull requests that take the same number conflict on that file; the one merged second takes the next number. Once on `main`, an ID never changes and is never reused.
- **The goal never changes under an ID.** Fixing words, steps, data or surfaces keeps the ID. A different goal retires the old Journey (`status = "retired"`, `replaced_by`, `reason`), whose file stays, and takes a new ID. A split retires the old one and lists both successors. This is web-features' `moved` and `split` without a separate stub file.
- **Where a test carries the ID** (selection; the result document is the record):
  - Rust (nextest): the ID, lowercased without its hyphen, is a module in the test's path (`journeys::j002::…`), since nextest selects only by name and path.
  - Vitest: the tag `J-002`, declared in the config from the catalog, with `strictTags` on.
  - pytest: `@pytest.mark.journey("J-002")`, registered, with `strict_markers` on.
  - Playwright Test: the tag `@J-002`.

  > Agent decision, accepted by the owner on 2026-10-21 (#346, 2026-10-19): a test also carries its surface, since one Journey file holds several variants (`cli` and `foxhound-cli` in one executable crate) and `xtask journeys select` must print one exact filter per runner. nextest names the test after the surface (`journeys::j002::cli`); Vitest and Playwright add the surface's tag (`sdk-typescript`, `@console`); pytest writes `@pytest.mark.journey(id="J-002", surface="sdk-python")`, keywords because pytest's `-m` matches only keyword arguments. `TESTING.md`, Journeys, has the rule.

### Seeded data and isolation

- **Each Journey that uses Proving ground pages has its own seed**, written in its file when it is created. The first catalog uses the Journey's number. A Journey that touches no page (J-001, J-009) has no seed. `GOODFETCH_TEST_SEED` (ADR 0059) overrides only the `Entropy` seed of in-process tests, never a Journey's seed, which is part of its data.
- **Each variant run starts from scratch.** It starts its own Proving ground on port 0 with the Journey's seed, its own Goodfetch home and cleared environment (ADR 0059), the `resolve` mapping of the Proving ground's host names to loopback, and the Proving ground's certificate authority trusted through ADR 0042's per-domain exception. Nothing is shared with another variant, so the Proving ground's per-task observations need no namespace.
- **The data is pinned by the seed, the Proving ground built from the same commit and each task's digest.** The result document records the seed and the version. Each named Proving ground task is pinned by its `digest`, a hash the Proving ground computes over the task's entry and its expected result with every URL's port removed, so the digest is the same on whatever port the run binds. A generator change that alters, moves or removes a task a Journey names therefore fails the catalog check, so the pull request that changes the generator updates the Journeys it moves, digest included, after checking the Journey still fits.
- **Starting state is built through public surfaces** in the Journey's first steps: settings, Limits, a Saved identity from a Proving ground login, an earlier run. No run file, database or home folder is copied in, except an older-format file an upgrade Journey starts from (the upgrade fixtures of [Upgrade and version-mismatch testing](https://github.com/goodfetch/goodfetch/issues/287)).
- **Logins** use accounts the Proving ground derives from the seed, so each variant has its own accounts.
- **Journeys run against the local copy only**, under ADR 0043's outbound deny. Online-copy results stay conformance reports.
- **Journeys drive the shipped artefacts**: the `goodfetch` binary and the SDK packages built from the commit, never an engine linked into the test.
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): the executable and SDK packages of the Journey's Product; the one exception is `J-NNN@foxhound-cli`, which runs a Collie Journey's `cli` variant on Foxhound's executable.
  > Note ([#439](https://github.com/goodfetch/goodfetch/issues/439)): in a skew run (ADR 0066) the Client is a released one and the executable the current one of its Product, given by path.

### Runners and results

- **Each surface's own runner runs its variants**: nextest for `cli` and `sdk-rust`, Vitest for `sdk-typescript`, pytest for `sdk-python`, Playwright Test for `console`. The MCP, skills, adapter and Integration runners are those their own tickets choose.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Each surface keeps its Journey tests in a `journeys` folder of the package that ships it**, beside its integration tests (ADR 0062, which leaves this to this ADR), one file per Journey:
  - Rust: `tests/it/journeys/j002.rs` in the `goodfetch` binary's crate for `cli` and in the public `goodfetch` crate for `sdk-rust`, so the ID is in the test's path;
  - TypeScript: `tests/journeys/J-002.test.ts` in the TypeScript SDK's package, and `tests/journeys/J-002.spec.ts` in the Console's;
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - Python: `tests/journeys/test_j002.py` in the Python SDK's package, beside `tests/unit/` and `tests/integration/`.
- **Journey tests run in the suites ADR 0061 gives their package's integration tests**: the Pre-merge suite when their package is touched, the Release suite always. They are not among the checks non-deterministic by nature that ADR 0061 keeps out of the suites, since each runs on its own local Proving ground. The local command that selects variants by ID, surface, milestone or area is the "Test infrastructure" spec's.
- **A variant's Proving ground, home and environment come from ADR 0062's test support**: `goodfetch-test-support` in Rust, and `goodfetch-test-env` through the two `test-support/` packages in TypeScript and Python. The engine is the shipped binary, never one the helper links.
- **Each variant run writes one result document** (area 2's, built by [`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/goodfetch/goodfetch/issues/169)) with `kind = "journey"`. It records the variant, each step's outcome (`passed`, `failed`, `not_reached`, `manual`), each task's oracle answer with its evidence, the Proving ground's version and seed, the binary's and the surface package's versions, and the platform and runtime.
- **The outcome is derived, never set by the test**, in area 2's words:
  - `proved` when every automated step passed, every check passed and every named task is `verified`;
  - `failed` when any of them failed, an oracle answer is `incomplete`, `false_success` or `failed`, or a step was not reached;
  - `unproved` for a declared variant with no test or no run, computed by the catalog report from the absence of a document.

  A `manual` step leaves the variant `proved` on its automated steps and is listed for the owner test.
- **How "derived, never set" holds on every runner.**
  - In Rust, the outcome comes from a constructor in `goodfetch-protocol`.
  - The TypeScript and Python helpers derive it with the same rule, tested against a shared set of outcome vectors: steps and oracle answers with the outcome they give. The vectors are hand-written data, so they live in `test-support/fixtures/outcome-vectors/` (ADR 0062's place for data several languages read), never in the generated schema bundle.
  - Every reader of result documents (the catalog report, the coverage check, the release gate) loads them through a Rust reader in `goodfetch-protocol` that recomputes the outcome and rejects a document whose stored outcome disagrees. A helper that writes `proved` beside a `false_success` answer fails CI there.
  - Those readers also compare each `journey` document with its Journey and reject one whose steps, checks and named Proving ground tasks are not exactly the Journey's. A helper that leaves a failed or unreached step out of the document therefore fails CI too.

  > Agent decision, accepted by the owner on 2026-10-21 (#350, 2026-10-20): the Rust helper, `goodfetch-test-support`'s `JourneyRun`, adds `check(wording, body)` for a check that names no task, and `expect_task(task, submission)` submits the tool's result to the variant's Proving ground's oracle and records the answer and each check naming the task. A `foxhound-cli` variant in Collie's executable crate takes Foxhound's executable from the test tool manifest (`tools()`'s `foxhound` entry), since Collie's crate takes no Foxhound crate. The executable's Product and version in the document are those its `--version` reports. Each run writes `<id>@<surface>.json` into the folder `GOODFETCH_TEST_RESULT_DOCUMENTS` names, which `just test` sets to `target/result-documents/`, else into the test's own folder. `TESTING.md`, Journeys, has the rules.

  > Agent decision, as built (spec #325's review, H2, 2026-10-22): each `[[expect]]` naming a task is judged on its own submission, since two checks may name one task (J-007's two Crawls of one site, J-013's and J-022's two scripts or requests on one product page). The helpers' `expect_task(check, submission)` (`expectTask` in TypeScript) takes the check's wording rather than the task, and records that check alone from its answer. A `journey` document holds each named task once per check that names it, in the Journey's task order and then the checks' order, each with the answer to that check's submission, or once with no answer when no check names it; `read_journey_document` refuses one that holds a task fewer times (`missing_task`), so a second check is never passed on the first check's answer. The document's fields are unchanged.

  > Agent decision, accepted by the owner on 2026-10-21 (no ticket, 2026-10-20): each run also writes its document in the test's own folder, and `Finished::path()` names that copy. In the folder `GOODFETCH_TEST_RESULT_DOCUMENTS` names, two tests that run the same variant (the helper's own tests do) wrote one file, and a test read the other's document under load (`journey_run::a_complete_passing_run_is_proved_and_accepted` read `Failed`).

### The catalog check and report

The "Test infrastructure" spec builds them. The check fails when:
1. a file does not parse under the type, or its name differs from its `id`;
2. two Journeys share an ID, or an ID is not below the next free number;
3. a Proving ground task a Journey names is missing from the task list for its seed, or its digest differs from the one the Journey pins;
4. a test carries the ID of a retired or unknown Journey, or a surface the Journey does not list, or two tests carry the same variant;
5. a variant's milestone is earlier than its surface's first milestone;
6. a retired Journey has no `reason`.

> Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): a Collie Journey that lists `cli` also has the variant `J-NNN@foxhound-cli`, which rule 5 checks against `cli`'s first milestone; a test carrying `foxhound-cli` for a Foxhound Journey, or for a Collie Journey without `cli`, fails rule 4.

The report lists each variant's latest outcome on `main` by milestone and area. Whether `covers` names existing features, and the "every feature is tested" check, are [Feature coverage and traceability](https://github.com/goodfetch/goodfetch/issues/279)'s.

> Agent decision, accepted by the owner on 2026-10-21 (#355, 2026-10-20): `xtask journeys report` reads the result documents `main`'s latest Release run kept in the results store: each Release cell writes them into its JUnit folder (`GOODFETCH_TEST_RESULT_DOCUMENTS` is `target/junit/result-documents/` there), so the store keeps them as `result-documents/<id>@<surface>.json` beside the cell's JUnit files. "The latest Release run" is the Feature report's: the newest commit of `main` the Release suite's test runs name. Each document goes through `goodfetch-protocol`'s reader; a variant is `proved` or `failed` as its accepted documents give across the cells (`failed` when any is), `rejected` when the reader refuses any of them or a document names another variant than its file, never `proved` then, and `unproved` when none was kept. A `foxhound-cli` variant is grouped under Foxhound. The report also groups by Product (ADR 0094), prints Markdown, and with `--out` writes `journeys.json` and `journeys.md`; `--owner-test <milestone>` prints the "Journeys to try" (every active Journey of either Product with a variant due at that milestone, its manual steps, then each `manual` exception whose Feature is due at it by the Feature report's rule). The daily test-health job adds the report to its summary and attaches both files. `TESTING.md`, Journeys, has the rules.

### Ownership and review

- **Each area's `/to-spec` adds the Journeys of its user stories** to the catalog, in its `/to-tickets` pull request or the first ticket's, and `/to-tickets` names, in each ticket, the variants that ticket makes `proved`. A variant without a test is `unproved` until then.
  > Amended by [ADR 0075](0075-a-user-story-needs-a-journey-only-when-it-is-a-persons-goal-a-variant-can-reach.md): `/to-spec` adds Journeys only for its stories with `journey = true`.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no skill writes repository files or ticket fields. The stories file (each story's flags and milestone) and the Journeys of flagged stories must exist before a test declares them; tickets add them like any other work.
- **Catalog changes are reviewed like code** in the pull request that makes them; the fresh review checks that no goal changed under an existing ID.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; this check is a review standard the `code-review` skill applies, and the owner sees its result before merging.

### Owner tests

- **The owner test of M1, M2 and M3 lists Journeys**: its "Journeys to try" are the variants whose milestone is that milestone, by ID and title, with their manual steps. The owner runs them by hand from a local build, and also tries anything else.
- **An owner test is ready only when every automated variant of its milestone is `proved` on `main`**, as the CTS Verifier runs only on a device that passed the automated suite.
- **A finding the owner makes on a goal no Journey covers adds a Journey**, in the finding's issue.

### The first catalog: M1's Journeys

Each Journey below lists its later surfaces too. Families are the Proving ground's; `catalog` is the first one, built by [Proving ground tracer: one seeded family, its task list and its oracle](https://github.com/goodfetch/goodfetch/issues/158). The items of [M1 owner test: a fully working extraction and crawl framework](https://github.com/goodfetch/goodfetch/issues/289) map one to one onto J-001 to J-010.

| ID | Title | Surfaces (milestone) | Proving ground | Covers |
|---|---|---|---|---|
| J-001 | Start the Daemon and open the local Console with no login; the system browser opening is a manual step | `cli`, `console` (M1) | none | `row:CN-03` |
| J-002 | Crawl a site over HTTP to Markdown and export its Dataset as JSON, CSV and Parquet | `cli` (M1); `sdk-rust`, `sdk-typescript`, `sdk-python`, `mcp-stdio`, `mcp-http` (M2) | `catalog` | `row:SA-01`, `row:CO-18`, `row:CO-25`, `row:CO-27`, `row:CO-29` |
| J-003 | Crawl a JavaScript-rendered site with browser rendering, with screenshots and a PDF of each page | `cli` (M1); the SDKs and MCP (M2) | [rendered and interactive pages](https://github.com/goodfetch/goodfetch/issues/166) | `row:SA-05`, `row:CO-15`, `row:CO-20`, `row:CO-21`, `row:SA-09` |
| J-004 | Follow a running Crawl in the Console: Runs, the run's drawer with its three axes, Datasets, Engine & settings | `console` (M1), the Crawl started by a step pinned to `cli` | `catalog` | `row:CN-03`, `row:CN-04`, `row:RT-15` |
| J-005 | Resume a Crawl after the Daemon is killed, with no Record delivered twice | `cli` (M1); the SDKs (M2) | `catalog` | `row:RT-10`, `row:RT-11`, `row:durability-resume` |
| J-006 | Stop a run at an install Limit and see it in `goodfetch usage` and the Console | `cli`, `console` (M1) | `catalog` | `row:cost-unpredictable-runaway` |
| J-007 | Two Crawls of one site share Politeness, never more than its in-flight cap together | `cli` (M1) | [Crawl families](https://github.com/goodfetch/goodfetch/issues/164) | `row:CO-09`, `row:compliance-site-owner-politeness` |
| J-008 | Crawl through a proxy, see a Block cause reported and the Session pool react | `cli`, `console` (M1) | [Crawl families](https://github.com/goodfetch/goodfetch/issues/164) and the forward proxy | `row:SA-22`, `row:ST-08`, `row:CO-12` |
| J-009 | See where each setting's value comes from | `cli` (`goodfetch config explain`), `console` (Engine & settings) (M1) | none | `row:RT-22` |
| J-010 | Run a Crawl and see nothing leave the machine beyond its targets and pinned downloads | `cli` (M1) | `catalog` | `row:SC-10` |
| J-011 | Extract fields and page metadata (JSON-LD, Open Graph) from product pages into Records | `cli` (M1); the SDKs and MCP (M2) | [Extraction families](https://github.com/goodfetch/goodfetch/issues/165) | `row:CO-14`, `row:CO-19`, `row:data-extraction-quality` |
| J-012 | Move a Checkpoint to another Goodfetch home and finish the run there | `cli` (M1); the SDKs (M2) | `catalog` | `row:RT-10` |

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

> Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): J-001 and J-009 are Collie's Journeys, the others Foxhound's.

Pain slugs are written as `row:` because they are rows of ADR 0037's matrix. The M1 specs of areas 4, 5, 6 and 12 add their own Journeys when they are written.

### What the M1 tracers implement

**[Proving ground tracer: one seeded family, its task list and its oracle](https://github.com/goodfetch/goodfetch/issues/158):**
- The family is `catalog`, ported from the lab's HTTP catalog: paginated listing pages and product pages, each product with its ID, name and price.
- **Task IDs** are `<family>/<ordinal>` (`catalog/1`), scoped by the seed, which travels with them. A family has the same tasks, with the same IDs and kinds, for every seed; the seed changes only their data (pages, products, start URLs and expected results). Ordinals are append-only within a family: a new task takes the next ordinal, and a removed one leaves a gap that is never reused. This is the concrete form of area 2's "seed-derived id".
- **The task list** is served as JSON: `proving_ground` (`version`, `seed`) and `tasks`, each with `id`, `family`, `kind` (`crawl` or `extract` for this family), `start_url` (with the port `serve` bound), `description` and `digest`.
- **The digest** is the lowercase hex SHA-256 of one JSON object, `{"entry": …, "expected": …}`: the task's entry without `digest`, and its expected result. Before hashing, every URL in both has its port removed (`http://catalog.test:51234/p/1` becomes `http://catalog.test/p/1`), and the object is serialized in the JSON Canonicalization Scheme (RFC 8785).
- **`serve` binds port 0 when asked**, and its first output line is one JSON object with `address`, `version` and `seed`, so a test helper reads it.
- **The version** is the workspace version plus the commit, so the result document pins the generator.
- **The seed** is an unsigned 64-bit integer, written in decimal.
- **The oracle's answer** names the task ID it judged.
- Tests show the same seed on the same port gives the same task list, IDs, start URLs and digests included; two `serve` runs with the same seed on different ports give the same digests; two seeds give the same task IDs and kinds; and a change to a task's expected result changes its digest.

**[`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/goodfetch/goodfetch/issues/169):**
- The result document has a `kind`: `conformance` or `journey`.
- A `journey` document carries the variant (`journey_id`, `surface`), each step's outcome, each task's oracle answer and evidence, and the versions of the binary and the surface package in place of a target. The common fields (platform, runtime, the Proving ground's version and seed) stay.
- Both kinds derive their outcome from what they hold, through a constructor in `goodfetch-protocol`, and its reader recomputes the outcome of a loaded document and rejects a mismatch.
- The outcome vectors are committed in `test-support/fixtures/outcome-vectors/`, and the Rust constructor passes them.
- Tests show a sample `journey` document validates against the schema, that one Proving ground task answering `false_success` makes it `failed`, and that the reader rejects a document stored as `proved` beside that answer.

### IDs that feature coverage references

[Feature coverage and traceability](https://github.com/goodfetch/goodfetch/issues/279) counts by: the Journey ID (`J-NNN`), the variant ID (`J-NNN@<surface>`), the surface names above, and each Journey's `covers` list. It owns the feature-ID kinds beyond `row:` and the check that every feature has a test.

## Considered Options

- **A test-management service** (Qase, TestRail, Allure TestOps): rejected. The Proving ground and benchmark are published so anyone can rerun them (ADR 0025), and a catalog in a service would be invisible to them. Its reviews happen outside the pull requests that autonomous delivery reviews (AGENTS.md); Qase approves case reviews only in its UI, and TestRail has versioned cases since 7.3 (2022) in its Enterprise plan. Every CI job runs under ADR 0043's deny, and an uploader would be one more named exception. The services' strength, recording manual runs, is covered by the owner test's issue.
- **Metadata in test code with a generated list** (Kubernetes): rejected. A Journey runs in four or more runners (nextest, Vitest, pytest, Playwright Test), so no single runner's code can hold it, and the steps a person follows would be split across languages.
- **Executable Gherkin** (Cucumber, cucumber-rs, pytest-bdd): rejected. Gherkin has no ID keyword, and step definitions would be written and kept in step with each other in three languages. The declarative step wording is kept without the glue.
- **Tests generated from the catalog** (Chromium's dPWA): rejected. Chromium generates one language's tests from shared actions; Goodfetch's surfaces differ in idiom (an SDK call, a CLI line, a Console click), so each surface writes its own test against the same steps.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **A folder per Journey, the path being the ID** (Deno, Crawlee): rejected. A path names the content, so a better name means a rename, which is how TestRail and Allure lose history.
- **Kebab-case slugs as IDs** (web-features): rejected. A Journey spans areas, and its title grows with each surface and feature; a number never needs to move.
- **A revision in each ID** (OpenFastTrace's `dsn~name~2`): rejected. Retiring the Journey and assigning a new number gives the same voiding with plain IDs.
- **One Proving ground shared by every Journey in a job, with a namespace per run** (wptserve's stash): rejected. A fresh instance starts in milliseconds and leaves no observation shared, which is the "start from scratch" rule.
- **Task IDs from a hash of the task's content:** rejected. IDs are assigned, never derived from content (the findings); the digest is a separate pin that the Journey records, so a changed task fails loudly without renaming anything.
- **One seed for the whole catalog:** rejected. The owner asked for each Journey's own seeded data, and a catalog-wide seed would couple every Journey to every generator change.
- **A separate Journey result format, or JUnit properties:** rejected. Area 2's result document is already written by every runner ([Conformance result documents from the TypeScript and Python test runners](https://github.com/goodfetch/goodfetch/issues/170)); Playwright's JUnit drops tags.
- **The owner tests kept as free text:** rejected. Their items would drift from the automated Journeys and never be counted.

## Consequences

- **Adds to ADR 0038**, so ADR 0037's amendment rule carries it to every area's spec: each area's `/to-spec` adds the Journeys of its user stories to the catalog, and its `/to-tickets` names in each ticket the variants that ticket makes `proved`.
  > Amended by [ADR 0075](0075-a-user-story-needs-a-journey-only-when-it-is-a-persons-goal-a-variant-can-reach.md): only the Journeys of its stories with `journey = true`.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no skill writes repository files or ticket fields. The stories file (each story's flags and milestone) and the Journeys of flagged stories must exist before a test declares them; tickets add them like any other work.
- **Adds to ADR 0032:** the `journeys/` folder at the root of the repository.
- **Adds to ADR 0058:**
  - each owner test lists its milestone's variants by Journey ID as "Journeys to try", with their manual steps;
  - it is ready only when it is the only open issue of its milestone and every automated variant of that milestone is `proved` on `main` (amends ADR 0058's readiness rule, which a note there records);
  - a finding on an uncovered goal adds a Journey.

  [M1 owner test: a fully working extraction and crawl framework](https://github.com/goodfetch/goodfetch/issues/289)'s "Scenarios to try" become "Journeys to try" with J-001 to J-010, plus J-011 and J-012.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/goodfetch/goodfetch/issues/152)):
  - a task's "seed-derived id" is `<family>/<ordinal>`, scoped by the seed;
  - `serve`'s first output line is the JSON object above, and it binds port 0;
  - the result document has a `kind`, and its `journey` kind is written by every surface's runner, the Vitest and pytest helpers of [Conformance result documents from the TypeScript and Python test runners](https://github.com/goodfetch/goodfetch/issues/170) included;
  - outcomes are derived by a constructor, never set.

  ADR 0037's area table lists this ADR in area 2's row.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/goodfetch/goodfetch/issues/213)): its "Scenarios" that need several processes or machines, and the "delivery, leases, credits and admission scenarios" of its in-memory adapter paragraph, are called multi-process tests, since *scenario* is an `_Avoid_` term for Journey.
- **The glossary** gains *Journey* and *Proving ground task*.
- **Left to other tickets of the test strategy map,** each building on the interface above:
  - the feature inventory and the coverage check: [Feature coverage and traceability](https://github.com/goodfetch/goodfetch/issues/279);
  - what a failing or flaky Journey test does: decided since in [ADR 0064](0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md);
  - Journeys of AI features and the `skills` surface: decided since in [ADR 0065](0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). A Journey that uses an AI feature names the Proving ground's Fake model as its Model provider: the variant's own Proving ground serves it, and the Journey's setup steps point Goodfetch's Model provider setting at it through a public surface, so the shipped binary calls it like any provider. The in-process Scripted model never appears in a Journey;
  - the `console`, SDK, CLI, MCP, adapter and Integration runners: [Console and docs test stack](https://github.com/goodfetch/goodfetch/issues/282), [SDK and CLI test stack](https://github.com/goodfetch/goodfetch/issues/285), [MCP, framework adapter and Integration test stack](https://github.com/goodfetch/goodfetch/issues/286).
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **[`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/goodfetch/goodfetch/issues/169)** builds the Rust constructor, the reader and the outcome vectors; [Conformance result documents from the TypeScript and Python test runners](https://github.com/goodfetch/goodfetch/issues/170) builds the helpers' derivation against them.
- **The "Test infrastructure" spec** builds the Journey type and its schema, the catalog check and report, the readers' comparison of a document with its Journey, the command that selects variants, the twelve files of the first catalog, and each language's helper that reads a Journey, starts its Proving ground through ADR 0062's test support and writes its result document.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): *Journey* is the glossary's term and is capitalised wherever it names one, here and in `TESTING.md`; the lowercase "journeys" above are quotes (the owner's request, Chromium's critical user journeys).

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): `sdk-rust` variants drive their Product's shipped executable through the crate's client, as a Daemon, never the engine the crate can link (ADR 0069).

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): "every URL's port removed" includes URLs inside string values, such as an absolute link in an expected Markdown body: the digest removes the port of every `http` or `https` URL wherever it appears in a string, which the Proving ground's digest tests prove.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): **Adds to ADR 0061:** Journey tests run with their package's integration tests, in the Pre-merge suite when the package is touched and in the Release suite.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): **Adds to ADR 0062:** each surface keeps its Journey tests in a `journeys` folder of the package that ships it, beside its integration tests.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): the MCP, adapter and Integration runners left to [#286](https://github.com/goodfetch/goodfetch/issues/286) are decided since in [ADR 0070](0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md), whose "Adds to ADR 0063" names them, as the Console's and the SDK and CLI runners are in ADRs 0068 and 0069.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

> Note of 2026-10-22 (spec #326's review, ST-1; agent decision, as built): a `journey` document's facts hold `test_errors`, each error that failed the variant's test outside its steps, checks and tasks (a page error or an accessibility violation found once the body ended, ADR 0068), and the variant is `proved` only when that list is empty. Without it, a Console variant whose test failed at teardown kept a `proved` document. The shared outcome vectors state the rule (`the-test-failed-after-its-checks`), which `goodfetch-protocol`, the TypeScript and the Python helpers pass.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
