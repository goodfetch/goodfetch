# Journey catalogs, test IDs and feature traceability

Research of **October 13, 2026** for ticket [#269](https://github.com/Jbmanllr/automan/issues/269) (map [#262](https://github.com/Jbmanllr/automan/issues/262)), feeding [Journey catalog: format, IDs, seeded data and surfaces](https://github.com/Jbmanllr/automan/issues/278).

**Question.** How do mature projects keep a catalog of end-to-end user journeys with stable IDs and per-journey test data, and prove in CI that every feature has a test without a coverage percentage?

**Sources.** Source files read through the GitHub API at each repository's default branch on 2026-10-13, and official documentation pages read the same day. Each fact gives its link; versions and last-change dates are given where they matter. Three angles: what comparable projects and direct competitors ship (Crawlee, Stagehand), what the leaders of the problem space do (Kubernetes, web-platform-tests, Chromium, Google SRE, OpenTelemetry, GitLab, LUCI), and what users complain about (issues and vendor caveats). Anything not confirmed in a primary source is listed under [Unverified](#7-unverified).

**Facts and fit.** Each section gives facts first, then a **Fit for Goodfetch** note, which is this note's reading of the facts against ADRs 0025, 0037 and 0058 and the [Proving ground and conformance spec](https://github.com/Jbmanllr/automan/issues/152). Like everything in `rnd/`, this note informs a decision and makes none: #278 decides.

---

## 1. Summary

- **Three catalog shapes exist, and every mature one keeps the catalog in the repository.**
  - *Metadata in the test code, with a generated list committed and diffed in CI.* Kubernetes: 462 conformance tests, each a `framework.ConformanceIt(...)` call preceded by a `Release` / `Testname` / `Description` comment; a script regenerates `test/conformance/testdata/conformance.yaml`, and a test fails when the committed list is stale. Changes to that file need SIG Architecture's review.
  - *A hand-kept data file with stable IDs, from which tests or documents are generated.* Chromium's installable web apps (dPWA) list their critical user journeys as Markdown tables of numbered actions; a script generates the browser tests, and each generated test's name encodes the action IDs. OpenTelemetry keeps its spec compliance matrix as YAML and regenerates the Markdown in CI.
  - *One folder per scenario, the path being the ID.* Deno (`tests/specs/<category>/<test>/__test__.jsonc`), Crawlee (`test/e2e/<scenario>/`), Stagehand's evals (`tasks/bench/<category>/<name>.ts`, run as `category/name`), web-platform-tests (the test's URL path).
- **Stable IDs are assigned, never derived from content, and renames are explicit.** Chromium numbers actions from a counter kept in the table header ("next: 185"). web-features turns a renamed ID into a `kind: moved` (or `split`) stub that redirects. LUCI ResultDB records a test's `previous_test_id`. OpenFastTrace puts a revision in each requirement ID (`dsn~name~2`) so that raising it voids every link to the old meaning. Where a service derives the ID from the test's name, its own docs say a rename creates a new case (TestRail's `trcli`, Allure TestOps).
- **A seed alone does not pin data.** proptest stores failing *seeds* and warns that a changed strategy may make the seed produce a different value; Faker warns that a new Faker version may give different values for the same seed. Seed-derived IDs stay stable only with the generator's version recorded beside the seed.
- **Each test runner carries an ID differently** (§3.1):
  - Vitest 4.1+ has tags declared in config, rejected when unknown by default (`strictTags: true`), selected with `--tags-filter`, and listed with `vitest list --json`.
  - pytest has markers. Unknown ones only warn unless `strict_markers` (default `false`) is set. `-m "journey(id='X')"` selects by a marker's keyword argument, and `record_property` writes a property into the JUnit XML.
  - Playwright has `tag` and `annotation` on each test. Its JSON report carries the tags; its JUnit report does not, by maintainers' choice.
  - cargo-nextest has no tag or attribute predicate. Its filtersets select by test name, package, binary, kind, platform and test group only, so in Rust an ID must live in the test's path or name.
- **"Every feature has a test" is proved by enumerating the feature list, not by line coverage** (§3.2):
  - Kubernetes: an audit job lists the API operations the conformance run actually called and compares them with the OpenAPI spec; every GA endpoint must be hit, or appear in `ineligible_endpoints.yaml` with a reason and a link, or in a short `pending_eligible_endpoints.yaml`.
  - GitLab: every controller action, API endpoint, Sidekiq worker and RSpec example must declare a `feature_category` from a generated list. A spec walks every route, and a RuboCop rule flags tests without one.
  - OpenFastTrace: its trace reports every requirement that lacks coverage, and its return code can gate CI.
  - OpenTelemetry: the matrix is checked for drift but is self-reported, with no link to tests.
- **Per-journey data is isolated by starting from scratch, not by cleaning up**: a fresh browser context per Playwright test (the docs call cleaning up in between the riskier strategy), a namespace per Kubernetes e2e test, a database per `#[sqlx::test]` with composable SQL fixtures, UUID-keyed server state in web-platform-tests' server.
- **Test-management services link code to cases by an ID written in the test**: Qase `qase.id(1)` or a `QaseID` annotation, TestRail `[C123]` in the JUnit name or a `test_id` property, Allure TestOps `@allure.id`. Review and history then live in the service: TestRail's case versioning is an Enterprise feature (7.3, January 2022); Qase's case reviews are proposed by API but approved only in its UI; Qase ships an official MCP server, and no official TestRail one was found.
- **Manual tests are a declared, separate category.** web-platform-tests marks them `-manual` and warns they run "approximately never". Android's CTS Verifier supplements the automated suite for what needs a person and saves a report.
- **What users complain about**:
  - Crawlee's e2e tests are flaky because they crawl live sites, including its own docs site; an issue open since April 2025 asks to self-host the target pages.
  - TestRail and Allure TestOps say a renamed test becomes a new case.
  - Playwright's JUnit report drops tags.
  - Vitest users asked for tags for two years before 4.1 shipped them.
  - OpenTelemetry contributors found the matrix ambiguous and its one-line-per-feature layout useless for `git blame`.
  - web-platform-tests warns that glob patterns silently capture unrelated new tests.

---

## 2. Catalogs: formats, ID schemes and how IDs stay stable

### 2.1 Facts

**Google: critical user journeys.**
- The SRE Workbook (chapter "Implementing SLOs", section "Modeling User Journeys") defines a critical user journey as a sequence of tasks that is a core part of a user's experience. Its example for online shopping: searching for a product, adding it to a cart, completing a purchase ([sre.google](https://sre.google/workbook/implementing-slos/)).
- Google's "Product-Focused Reliability for SRE" (Crous, Roth, Hurd, undated) says CUJs model user objectives as goals with a set of tasks or steps. It expects product managers and other non-engineering roles to own the list of objectives ([sre.google](https://sre.google/resources/practices-and-processes/product-focused-reliability-for-sre/)).

**Chromium.**
- Kombucha, the declarative interaction-test API, is "the preferred way to create end-to-end interaction and critical user journey regression tests for Chrome Desktop" ([docs/testing/pixel_tests.md](https://github.com/chromium/chromium/blob/main/docs/testing/pixel_tests.md)). The UI starter guide asks for end-to-end CUJ interaction tests in `interactive_ui_tests` ([docs/ui/starter_guide.md](https://github.com/chromium/chromium/blob/main/docs/ui/starter_guide.md)).
- **The dPWA catalog** ([integration-testing-framework.md](https://github.com/chromium/chromium/blob/main/chrome/browser/web_applications/docs/integration-testing-framework.md), moved there on 2026-03-17):
  - The CUJs are Markdown tables in [`critical_user_journeys.md`](https://github.com/chromium/chromium/blob/main/chrome/test/webapps/data/critical_user_journeys.md), last changed 2026-09-03. Each row starts with a platform column (`W`, `M`, `L`, `C`), followed by a sequence of actions with arguments.
  - The actions are catalogued in [`actions.md`](https://github.com/chromium/chromium/blob/main/chrome/test/webapps/data/actions.md). Each has a numeric "Unique Identifier", and the column header keeps the next free number ("next: 185").
  - A script, [`generate_framework_tests_and_coverage.py`](https://github.com/chromium/chromium/blob/main/chrome/test/webapps/generate_framework_tests_and_coverage.py), prints the minimal set of tests that covers the journeys, says which existing tests are now unnecessary, and writes per-platform coverage files.
  - The generated tests are named from the action IDs and arguments, for example `WAI_32NotPromotableNoShortcutBrowserWebApp_11NotPromotable_34NotPromotable_94_163NotPromotable`. Their bodies are marked as generated and not to be edited ([web_app_integration_browsertest_mac_win_linux.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/views/web_apps/web_app_integration_browsertest_mac_win_linux.cc)).
  - The rule: a feature counts as "supported" only if its CUJs are described and integration tests generated for them (the doc's "MUST").
- **Web tests' expectations** live in a flat `TestExpectations` file: one line per test or directory, with a bug link, platform modifiers and the expected outcome. For example `crbug.com/12345 [ Win Debug ] fast/html/keygen.html [ Crash ]`. A test expected to fail still runs, so that a fix is noticed ([web_test_expectations.md](https://github.com/chromium/chromium/blob/main/docs/testing/web_test_expectations.md)).

**Kubernetes conformance** (master on 2026-10-13; latest release v1.37.1, 2026-09-23).
- **The catalog file.** [`test/conformance/testdata/conformance.yaml`](https://github.com/kubernetes/kubernetes/blob/master/test/conformance/testdata/conformance.yaml) has 462 entries. Each has `testname` (short, human), `codename` (the full Ginkgo test name, ending in `[Conformance]`), `description`, `release` and `file`.
  - It is generated: [`walk.go`](https://github.com/kubernetes/kubernetes/blob/master/test/conformance/walk.go) runs the e2e binary in dry mode and reads the comment block within 5 lines above each conformance test.
  - It refuses names that carry `[Alpha]`, `[Feature:…]` or `[Flaky]`.
  - The folder's [README](https://github.com/kubernetes/kubernetes/blob/master/test/conformance/README.md): adding or removing a conformance test makes a test fail until the golden list is updated, and changes to it need SIG Architecture's review.
- **The format in code.** The test calls `framework.ConformanceIt(...)`, which adds the `[Conformance]` text and a Ginkgo label `Conformance` ([ginkgowrapper.go](https://github.com/kubernetes/kubernetes/blob/master/test/e2e/framework/ginkgowrapper.go)). A comment above it carries the metadata. `Description` must state the required behaviour with RFC 2119 keywords, detailed enough to be read as a specification without the code ([conformance-tests.md](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/conformance-tests.md), last changed 2026-09-13).
- **Promotion** ([same doc](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/conformance-tests.md)):
  - A test is first written as an ordinary e2e test.
  - It must be stable, with no flakes, and have run for at least two weeks.
  - A separate pull request then promotes it. That PR may only switch `It` to `ConformanceIt`, add the metadata comment and regenerate the YAML, and it goes through a review board.
  - The doc also requires that a test have "a name that is a literal string".
- **Demotion** follows the same process and can be backported. The latest change to the file, on 2026-08-24, demoted the guaranteed pod resize tests to e2e ([#141554](https://github.com/kubernetes/kubernetes/pull/141554)).
- **The published report.** A per-release Conformance Document is generated from the metadata, so readers see what is tested without reading code.

**web-platform-tests** ([out-of-band-metadata.md](https://github.com/web-platform-tests/wpt/blob/master/docs/writing-tests/out-of-band-metadata.md), last changed 2026-07-27).
- **The test ID is the file's path.** Flags live in the file name: `-manual`, `-visual`, `.https`, `.tentative`, and so on ([file-names.md](https://github.com/web-platform-tests/wpt/blob/master/docs/writing-tests/file-names.md)).
- **Per-directory YAML.**
  - `META.yml` gives the spec link and the suggested reviewers.
  - `WEB_FEATURES.yml` maps the directory's tests, by file pattern, to web-features IDs. Rules apply top to bottom, and a file matched once is not matched again. An empty list excludes a file.
  - A JSON manifest from feature ID to literal test files is generated on every commit to the main branch and attached to a release.
- **The docs' two caveats.** A pattern like `b*` can silently capture unrelated tests added later. A test that checks several features is better left unclassified.
- **Triage links live outside.** Links from test results to bugs live in a separate [`wpt-metadata`](https://github.com/web-platform-tests/wpt-metadata) repository: YAML per directory, keyed by test path and optionally browser.

**web-features IDs** ([guidelines.md](https://github.com/web-platform-dx/web-features/blob/main/docs/guidelines.md); v3.42.0, 2026-10-09).
- **The scheme.** An ID is the feature file's name: lowercase letters, digits and hyphens, and short (`aborting`, not `abort-controller-and-abort-signal`).
- **Moving an ID.** The guidelines allow a move for a misspelling, a guideline breach, a feature that should not have existed, or confusion reported by consumers. The old file then becomes `kind: moved` with a `redirect_target`. A feature split in two becomes `kind: split` with `redirect_targets`.
- **When not to move.** A superseded feature keeps its ID and is marked `discouraged` instead.

**Deno** ([tests/specs/README.md](https://github.com/denoland/deno/blob/main/tests/specs/README.md); Deno v2.9.7, 2026-09-17).
- **Layout and ID.** Integration tests of the `deno` binary are folders `tests/specs/<category>/<test>/` (54 categories on 2026-10-13), each with a `__test__.jsonc`. The test ID is the path: `cargo test specs::category_name::test_name`.
- **The file.** It declares `args`, an expected `output` file (`.out`, with wildcards such as `[WILDCARD]` and `[UNORDERED_START]`), `exitCode`, `if` (an OS), `steps`, named sub-`tests` and `tempDir`. A JSON Schema ([schema.json](https://github.com/denoland/deno/blob/main/tests/specs/schema.json)) also defines `envs`, `ignore`, `variants`, `timeout` (default 300 s) and `flaky` (retry up to 3 times).

**Crawlee** (direct competitor; v3.18.2, 2026-09-29).
- **Layout and ID.** [`test/e2e/`](https://github.com/apify/crawlee/tree/master/test/e2e) has 45 scenario folders, such as `cheerio-max-requests`, `playwright-robots-file` and `session-rotation`. Each holds an `actor/` (a runnable Apify Actor) and a `test.mjs` of assertions. The folder name is the ID, and [`run.mjs`](https://github.com/apify/crawlee/blob/master/test/e2e/run.mjs) runs one folder when given its name.
- **Data.** The actors crawl live sites, for example `https://crawlee.dev` in `cheerio-max-requests`.

**Stagehand evals** (direct competitor; `main` on 2026-10-13, [packages/evals/README.md](https://github.com/browserbase/stagehand/blob/main/packages/evals/README.md)).
- **Layout and ID.** Tasks are discovered from `tasks/bench/<category>/` with no registration: about 40 `act`, 25 `extract` and 12 `observe` files on 2026-10-13. They are run by tier, category or `category/name` (`extract/extract_text`), and a task can carry `tags`.
- **Data.** Tasks load pages from `stagehand-eval-sites.vercel.app`, a site Browserbase hosts, and compare the result with a hard-coded expected value.

**OpenTelemetry spec compliance matrix** ([spec-compliance-matrix/](https://github.com/open-telemetry/opentelemetry-specification/tree/main/spec-compliance-matrix); spec v1.61.0, 2026-09-14).
- **Source.** `template.yaml` lists the features by section and heading; one YAML per language holds each feature's status (`+`, `-`, `?`, `N/A`). Features are identified by their display name, with no separate ID.
- **Check.** `make compliance-matrix` regenerates `spec-compliance-matrix.md`, and the `compliance-matrix-check` job fails on any `git diff` ([checks.yaml](https://github.com/open-telemetry/opentelemetry-specification/blob/main/.github/workflows/checks.yaml)). The YAML replaced a hand-edited table in [#4631](https://github.com/open-telemetry/opentelemetry-specification/pull/4631) (2025-08-29).

**Gherkin** ([reference](https://github.com/cucumber/docs/blob/main/content/docs/gherkin/reference.md)).
- **No ID keyword.** A `.feature` file holds one `Feature`, optional `Rule`s and `Scenario`/`Example`s. Its keywords include no identifier; tags (`@…`) are the only metadata, placed above a feature to group features independently of folders.
- **Declarative wording.** Cucumber's own guidance asks for scenarios that describe behaviour ("When Bob logs in"), not procedures. Its test: will the wording need to change if the implementation does? ([better-gherkin.md](https://github.com/cucumber/docs/blob/main/content/docs/bdd/better-gherkin.md)).

**LUCI ResultDB, Chromium's test result store** ([common.proto](https://github.com/luci/luci-go/blob/main/resultdb/proto/v1/common.proto), [test_metadata.proto](https://github.com/luci/luci-go/blob/main/resultdb/proto/v1/test_metadata.proto), last changed 2025-12-05).
- **The structured ID.** A `TestIdentifier` has a module name and scheme (such as `gtest` or `junit`), a module variant (ABI, OS, build flags) with an 8-character hash, a coarse name (package or directory), a fine name (class or file) and a case name. Nested `describe` levels are joined with `:`.
- **Renames.** `TestMetadata` carries `previous_test_id`, the ID the test had before, plus a source location and a bug component.

**OpenFastTrace** ([concepts_and_terms.md](https://github.com/itsallcode/openfasttrack/blob/main/doc/user_guide/introduction/concepts_and_terms.md); 4.10.0, 2026-09-20).
- **IDs.** A specification item ID is artifact type, name and revision, for example `dsn~validate-authentication-request~1`.
- **Links in code.** Tests and code cover items with tags in comments, such as `// [impl->dsn~validate-authentication-request~1]` ([input_format_support.md](https://github.com/itsallcode/openfasttrack/blob/main/doc/user_guide/reference/input_format_support.md)).
- **Revisions.** A semantic change raises the revision, which voids every existing link until the covering items are checked; fixing a typo does not.

### 2.2 Comparison

| Catalog | Form | ID scheme | How an ID survives change | Checked in CI by |
|---|---|---|---|---|
| Kubernetes conformance | Comment above the test → generated YAML | Full test name (`codename`) plus a short `testname` | Changing a name changes the golden file, which SIG Architecture reviews; demotion has a process | Regeneration plus a failing test when the list is stale; an audit job for endpoint coverage |
| Chromium dPWA CUJs | Markdown tables → generated C++ tests | Numeric action IDs from a counter; test name built from them | IDs never reused (the "next" counter); journeys are action sequences, not names | The generator says which tests to add or remove; coverage TSV per platform |
| web-platform-tests | Files; per-directory YAML for metadata | The test's path | Directory-level YAML with patterns; feature IDs redirected by web-features | Manifest regenerated on every commit; lint |
| web-features | One YAML per feature | Kebab-case slug | `kind: moved` / `kind: split` stubs | Build of the distribution files |
| Deno | Folder per test with `__test__.jsonc` | `category/test` path | Path only | JSON Schema; the spec runner |
| Crawlee | Folder per scenario | Folder name | Path only | The e2e runner (scheduled) |
| Stagehand evals | File per task under a category | `category/name` | Path only | The evals runner |
| OpenTelemetry | YAML features plus YAML status per language → generated Markdown | Feature display name | None beyond the name | Regenerate and `git diff --exit-code` |
| Gherkin | `.feature` files | None built in; tags by convention | Tags | The Cucumber runner |
| LUCI ResultDB | Result records | Structured: module, variant, coarse, fine, case | `previous_test_id` | Not applicable (a results store) |
| OpenFastTrace | Requirements in Markdown, tags in code | `type~name~revision` | Revision bump voids links | The trace's return code |

### 2.3 Real entries of each format

Kubernetes `conformance.yaml` (first entry on master):

```yaml
- testname: Priority and Fairness FlowSchema API
  codename: '[sig-api-machinery] API priority and fairness should support FlowSchema
    API operations [Conformance]'
  description: ' The flowcontrol.apiserver.k8s.io API group MUST exist in the /apis
    discovery document. ...'
  release: v1.29
  file: test/e2e/apimachinery/flowcontrol.go
```

Kubernetes metadata in code (the doc's sample):

```go
/*
  Release: v1.9
  Testname: Kubelet: log output
  Description: By default the stdout and stderr from the process being
  executed in a pod MUST be sent to the pod's logs.
*/
framework.ConformanceIt("it should print the output to logs", func() { ... })
```

Chromium `critical_user_journeys.md` (one row) and `actions.md` (one row):

```text
| WMLC | install_by_user(Standalone) | launch(Standalone) | check_app_title(Standalone, StandaloneOriginal) |
| check_app_badge_empty | Site |  | 2 | Not Implemented | Check that the 'badge' on the app icon is empty |  |
```

Chromium `TestExpectations` (the doc's example):

```text
crbug.com/12345 [ Win Debug ] fast/html/keygen.html [ Crash ]
```

web-platform-tests `url/WEB_FEATURES.yml` and `url/META.yml`:

```yaml
rules:
- url-statics-canparse.*: [url-canparse]
- a-element*: [base, url]
- "*": [url]
```

```yaml
spec: https://url.spec.whatwg.org/
suggested_reviewers:
  - mikewest
  - domenic
```

web-features rename stub (the guidelines' template):

```yaml
kind: moved
redirect_target: numeric-separators
```

Deno `tests/specs/run/045_proxy/__test__.jsonc`:

```json
{
  "args": "run -L debug --allow-net --allow-env --allow-run --allow-read --reload --quiet --config ../../../config/deno.json proxy_test.ts",
  "output": "proxy_test.ts.out"
}
```

Crawlee `test/e2e/cheerio-max-requests/test.mjs` (abridged):

```js
const { stats, datasetItems } = await runActor(testActorDirname);
await expect(stats.requestsSucceeded >= 10, 'All requests finished');
await expect(datasetItems.length > 5 && datasetItems.length < 15, 'Number of dataset items');
```

Stagehand `tasks/bench/extract/extract_aigrant_targeted.ts` (abridged):

```ts
export default defineBenchTask(
  { name: "extract_aigrant_targeted" },
  async ({ stagehand, page }) => {
    await page.goto("https://stagehand-eval-sites.vercel.app/sites/aigrant/");
    // ... extract, then compare with { company_name: "Coframe" }
```

OpenTelemetry `spec-compliance-matrix/rust.yaml` (abridged):

```yaml
sections:
  - name: Traces
    features:
      - heading: '[TracerProvider](specification/trace/api.md#tracerprovider-operations)'
        features:
          - name: Create TracerProvider
            status: '+'
```

Gherkin (the reference's example):

```gherkin
Feature: Highlander
  Rule: There can be only One
    Example: Only One -- More than one alive
      Given there are 3 ninjas
```

LUCI ResultDB `TestIdentifier` (the proto's examples): module `chrome/test:blink_webkit_tests`, scheme `gtest`, fine name `WebDialogBrowserTest`, case `topLevelTest:with_context:does_something`.

OpenFastTrace tag in code:

```java
// [impl->dsn~validate-authentication-request~1]
```

### 2.4 What users complain about

- **Live sites make e2e flaky.**
  - Crawlee's maintainers wrote that their e2e tests are "relatively flaky" because the pages they crawl change, their own sites included. [#2906](https://github.com/apify/crawlee/issues/2906), opened 2025-04-01 and still open, asks for a self-hosted site.
  - [#3674](https://github.com/apify/crawlee/issues/3674) (2026-05-26) replaced one third-party page with a local `node:http` server, after the page changed and the scenario failed on every scheduled run.
  - [#2905](https://github.com/apify/crawlee/issues/2905) fixed tests broken by a redesign of the documentation site.
- **Patterns over-match.** web-platform-tests' own docs warn that a broad glob silently adds unrelated new tests to a feature, and that tests checking several features are better left unclassified ([out-of-band-metadata.md](https://github.com/web-platform-tests/wpt/blob/master/docs/writing-tests/out-of-band-metadata.md)).
- **A wide table hides history.** On [opentelemetry-specification#3976](https://github.com/open-telemetry/opentelemetry-specification/issues/3976), a contributor noted that with one Markdown line per feature for every language, `git blame` cannot tell when a language's status changed. That argument led to the YAML source. Contributors also found the matrix ambiguous about experimental features. A blank cell means "status not known".
- **Procedural scenarios rot.** Cucumber's guidance warns that procedural Gherkin must change whenever the implementation does ([better-gherkin.md](https://github.com/cucumber/docs/blob/main/content/docs/bdd/better-gherkin.md)).

### 2.5 Fit for Goodfetch

*Reading, not decision.*

- **Existing IDs.** Goodfetch already has two ID schemes in [ADR 0037's matrix](../../docs/adr/0037-v1-scope-matrix.md): `PREFIX-NN` for its 147 parity and Stagehand v3 rows (`RT-01`, `SA-13`, `V3-05`), and kebab-case slugs for its 24 pains (`access-antibot-ip`). Those are *capability* IDs, the analogue of web-features IDs or GitLab's feature categories (§3.2). A journey is a sequence across capabilities, like a Chromium CUJ across actions or a Google CUJ across tasks. So the facts point to two catalogs linked many-to-many: journeys with their own IDs, each naming the matrix rows it exercises.
- **Rename rules.** None of the matrix's ID rules for renames, splits or reuse is written down. Chromium's "next" counter, web-features' `moved`/`split` stubs and OpenFastTrace's revision are three proven ways to write them.
- **Where the catalog lives.** Kubernetes keeps the catalog in code and generates the YAML. Chromium and OpenTelemetry keep a data file and generate the rest. A journey in Goodfetch runs through several surfaces (CLI, three SDKs, MCP, Console), whose tests sit in three runners (nextest, Vitest, pytest; §3.1). A data file outside any one runner, which each runner's tests reference by ID, matches the Chromium and OpenTelemetry shape. Metadata in a single runner's code, as in Kubernetes, would cover one surface only.
- **Seeded starting data.** The Proving ground's tasks already carry seed-derived IDs (area 2's spec), which gives each journey seeded starting data without a factory. Section 4's facts on seeds apply to them.
- **The owner's scenarios.** ADR 0058's owner tests list scenarios to try by hand. Written as catalog entries with IDs, they become the manual journeys of §6, with the same IDs as the automated ones where both exist.

---

## 3. Traceability: tags per runner, matrices checked in CI, reports

### 3.1 Tags and annotations per runner

| Runner (version, date) | How a test carries an ID or tag | Declared and validated? | Select by it | List without running | Reaches JUnit XML? |
|---|---|---|---|---|---|
| cargo-nextest 0.9.148 (2026-10-08) | Only through its name and path: no tag or attribute predicate exists | — | Filtersets: `test()`, `package()`, `deps()`, `rdeps()`, `binary_id()`, `kind()`, `binary()`, `platform()`, `group()`, `default()`, with `and`/`or`/`not`/`-` | `cargo nextest list --message-format json` | Yes: one `<testcase>` per test, one `<testsuite>` per binary |
| Vitest 5.0.4 (2026-10-12); tags since 4.1.0 (2026-03-12) | `test('…', { tags: ['db'] }, …)`, on suites too; `annotate()` adds runtime notes | Tags must be defined in config; `strictTags` default `true` throws on unknown tags; a TypeScript `TestTags` union can type them | `--tags-filter="db && !flaky"`, `name/*` wildcards; `TestRunner.matchesTags` at runtime | `vitest list --json` (static parse since Vitest 5); `--list-tags=json` | Not checked |
| pytest 9.1.1 (2026-06-19) | `@pytest.mark.<name>(…)`; `record_property` | Registered in `markers`; unknown ones warn, and error only under `strict_markers` (default `false`) or `strict` (default `false`) | `-m "slow"`; keyword arguments: `-m "device(serial='123')"` (int, str, bool, None only) | `pytest --collect-only` | Yes, via `record_property` → `<properties>`; the docs warn this breaks the latest JUnit XML schema |
| Playwright 1.64.0 (2026-10-07) | `{ tag: '@fast' }` or `@token` in the title; `{ annotation: { type, description } }`; `test.info().annotations` at runtime | No declared list of tags in the docs; tags must start with `@` | `--grep @fast`, `--grep-invert`, regex lookaheads for AND; `grep` in config | `--list` | JSON report has `tags`; JUnit does not ([#38190](https://github.com/microsoft/playwright/issues/38190), closed 2025-11: "not all JUnit readers support tags") |

Sources: [nextest filtersets reference](https://github.com/nextest-rs/nextest/blob/main/nextest-filtering/reference.md), [nextest list](https://github.com/nextest-rs/nextest/blob/main/site/src/docs/machine-readable/list.md), [nextest JUnit](https://github.com/nextest-rs/nextest/blob/main/site/src/docs/machine-readable/junit.md); [Vitest test tags](https://github.com/vitest-dev/vitest/blob/main/docs/guide/test-tags.md), [strictTags](https://github.com/vitest-dev/vitest/blob/main/docs/config/stricttags.md), [Vitest CLI](https://github.com/vitest-dev/vitest/blob/main/docs/guide/cli.md), [Vitest annotations](https://github.com/vitest-dev/vitest/blob/main/docs/guide/test-annotations.md); [pytest marks](https://github.com/pytest-dev/pytest/blob/main/doc/en/how-to/mark.rst), [marker examples](https://github.com/pytest-dev/pytest/blob/main/doc/en/example/markers.rst), [reference: strict, strict_markers](https://github.com/pytest-dev/pytest/blob/main/doc/en/reference/reference.rst), [record_property](https://github.com/pytest-dev/pytest/blob/main/doc/en/how-to/output.rst); [Playwright annotations](https://github.com/microsoft/playwright/blob/main/docs/src/test-annotations-js.md), [CLI](https://github.com/microsoft/playwright/blob/main/docs/src/test-cli-js.md), [reporter types](https://github.com/microsoft/playwright/blob/main/packages/playwright/types/testReporter.d.ts).

Two more facts on runner-side IDs:
- **Playwright's own test ID** is computed from the file name, test title and project name, and is unique only within one Playwright session ([testReporter.d.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright/types/testReporter.d.ts)). A rename changes it.
- **pytest's docs** show a `test_id` marker copied into the JUnit `<properties>` by a `pytest_collection_modifyitems` hook ([output.rst](https://github.com/pytest-dev/pytest/blob/main/doc/en/how-to/output.rst)). That is the same property name TestRail's `trcli` reads (§5).

### 3.2 Requirement-to-test matrices checked in CI

**Kubernetes: every GA endpoint is exercised by conformance.**
- **What is checked.** The `pull-kubernetes-audit-kind-conformance` presubmit runs when `test/conformance/testdata/` or `api/openapi-spec/` change, and a 3-hourly periodic runs on master ([conformance-audit.yaml](https://github.com/kubernetes/test-infra/blob/master/config/jobs/kubernetes/sig-arch/conformance-audit.yaml)). Both run the conformance suite on a kind cluster with audit logging.
  - [`audit_log_parser.py`](https://github.com/kubernetes/test-infra/blob/master/experiment/audit/audit_log_parser.py) turns the logs into OpenAPI operation IDs.
  - The operations are compared with `swagger.json`, minus `ineligible_endpoints.yaml` (each entry: `endpoint`, `reason`, `link`) and `pending_eligible_endpoints.yaml`.
  - [`kubernetes_api_analysis.py`](https://github.com/kubernetes/test-infra/blob/master/experiment/audit/kubernetes_api_analysis.py) compares the pull request's operations with the latest CI run's, to show operations added or removed.
- **The pending list in use.** It holds endpoints whose tests have not run long enough yet. A 2026-05-06 commit added DeviceTaintRule endpoints there because tests "first have to run for a while", and a 2026-07-07 commit promoted their tests ([history](https://github.com/kubernetes/kubernetes/commits/master/test/conformance/testdata/pending_eligible_endpoints.yaml)).
- **The rule behind it.** SIG Architecture does not let new features land without conformance tests for their stable APIs. APISnoop opens a pull request every weekend that shows conformance-coverage gaps, so an endpoint promoted to GA without a test is caught ([2023 spotlight](https://github.com/kubernetes/contributor-site/blob/master/content/en/blog/2023/sig-architecture-conformance-spotlight.md)).
- **No percentage.** The measure is the set of untested eligible endpoints, which must stay empty.

**GitLab: every route, worker and test example names its feature category** ([feature categorization](https://gitlab.com/gitlab-org/gitlab/-/blob/master/doc/development/feature_categorization/_index.md)).
- **What must declare it.** Each Sidekiq worker, batched background migration, controller action, API endpoint and RSpec example declares `feature_category`, taken from `config/feature_categories.yml`. That file is generated from the handbook's `stages.yml`.
- **How it is checked.**
  - `spec/controllers/every_controller_spec.rb` walks all routes, checks that every action has a category, and checks that the categories exist and the configured actions are still routes.
  - The `RSpec/FeatureCategory` RuboCop rule flags examples without one. The `:shared` category is no longer accepted for tests.
- **What it serves.** Error budgets, alert routing, team attribution, and finding the owner of a flaky test.
- **End-to-end tests** add `:product_group`, `:smoke` and `:quarantine` metadata ([rspec_metadata_tests.md](https://gitlab.com/gitlab-org/gitlab/-/blob/master/doc/development/testing_guide/end_to_end/best_practices/rspec_metadata_tests.md)).

**OpenTelemetry: a drift check on a self-reported matrix.** See §2.1. CI proves the Markdown matches the YAML, never that a `+` has a test behind it.

**Chromium dPWA: coverage of the journey catalog.** The generator writes per-platform `coverage_<platform>.tsv` files with a coverage percentage at the top. The percentage counts the catalog's actions that the generated tests execute, and it drops when sheriffs disable a test ([integration-testing-framework.md](https://github.com/chromium/chromium/blob/main/chrome/browser/web_applications/docs/integration-testing-framework.md)). It is a percentage, but of catalog actions, not code lines.

**OpenFastTrace: uncovered requirements fail the trace.** The report lists defect items: requirements without the coverage their type needs, such as `impl` or `utest`. Its `quiet` verbosity exists "in case only the return code is used" ([oft_command_line.md](https://github.com/itsallcode/openfasttrack/blob/main/doc/user_guide/reference/oft_command_line.md)).

**Qase: a traceability matrix built from linked cases** ([docs](https://docs.qase.io/general/get-started-with-the-qase-platform/requirements-traceability-report)).
- **Coverage.** Requirements are pulled from an external tracker (Jira, GitHub or GitLab, among others), and a requirement counts as covered when at least one test case references it.
- **Versions.** Each matrix is a snapshot, regenerated by hand (there is no schedule). It flags cases whose requirement changed after their last run.
- **Export.** CSV or PDF.

### 3.3 Reports

- **Kubernetes** generates a Conformance Document per release from the metadata, and publishes results by provider on TestGrid ([conformance-tests.md](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/conformance-tests.md)).
- **Allure Report** (open source) groups results by behaviour hierarchy when tests set `@allure.epic` / `@allure.feature` / `@allure.story`. Tests can also carry `@allure.id`, `@allure.label`, `@allure.link` / `@allure.issue` / `@allure.testcase`, and `@allure.manual`, which asks Allure TestOps to create or update a manual case from the result ([pytest reference](https://allurereport.org/docs/pytest-reference/)).
- **JUnit XML** is the common carrier: nextest writes it per profile, and TestRail's `trcli` and pytest's `record_property` use its `<properties>`. Playwright's JUnit drops tags (§3.1).

### 3.4 Fit for Goodfetch

*Reading, not decision.*

- **Three runners, one ID.** The three runners Goodfetch uses (STYLE.md: nextest, Vitest, pytest) carry an ID in three different ways, and nextest only through the name.
  - STYLE.md says Rust test names describe the behaviour (`resumed_run_never_repeats_an_effect`) and puts all tests of a crate in one binary with modules (`tests/it/main.rs`).
  - So in Rust an ID can live in the module path (for example a module per journey) while the function keeps its behavioural name. nextest's `test()` matcher sees the whole path.
  - Vitest's config-declared tags with `strictTags` and pytest's markers under `strict_markers` give the same guarantee that a mistyped ID fails.
- **A cross-runner join point exists.** Area 2's spec already has the SDKs', the CLI's and MCP's cases "run in those surfaces' own test runners and write the same result document". That document is where a journey ID could be recorded once for every runner, independent of JUnit's uneven support for tags.
- **Counting coverage without a percentage.** Kubernetes and GitLab prove "every feature has a test" by walking a generated list of features. Goodfetch has one: the Operation catalog, generated from Rust types (ADRs 0011 and 0038). The Kubernetes pattern transposes as: every operation is exercised by a passing case, or listed as ineligible with a reason and a link, or pending for a short while. The GitLab pattern transposes to ADR 0037's rows: each row names at least one test or journey ID. Both are set checks with no percentage.
- **Results versus declarations.** OpenTelemetry's matrix is the case to avoid: a regenerated table that nobody's test confirms. Area 2's `proved` / `unproved` / `failed` per capability, filled from results, is the opposite.

---

## 4. Per-journey seeded data: factories, seeds, snapshots, isolation

### 4.1 Facts

- **Seeds and generator versions.**
  - **proptest** stores the seed of a failing case in `proptest-regressions/` and recommends committing it. It stores the seed, not the value, and says that if the strategy changed, the seed "may or may not" reproduce the failing value ([failure-persistence.md](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/failure-persistence.md)).
  - **Faker** (`@faker-js/faker` v10.6.0): `faker.seed(123)` makes a sequence reproducible. Upgrading Faker may change the values for the same seed. Relative dates need a fixed `refDate`, because "today" changes ([usage.md](https://github.com/faker-js/faker/blob/main/docs/guide/usage.md)).
- **Start from scratch rather than clean up** (Playwright, [browser-contexts.md](https://github.com/microsoft/playwright/blob/main/docs/src/browser-contexts.md)). Each test gets a new browser context with its own storage and cookies. The docs compare "start from scratch" with "cleanup in between" and name the risks of the second: forgotten clean-ups, and state such as visited links that cannot be cleaned.
- **Per-worker accounts** (Playwright, [test-parallel-js.md](https://github.com/microsoft/playwright/blob/main/docs/src/test-parallel-js.md) and [auth.md](https://github.com/microsoft/playwright/blob/main/docs/src/auth.md)).
  - Use `TestInfo.workerIndex` or `parallelIndex` to create one database user per worker.
  - Log in once in a `setup` project and reuse its saved `storageState`: the recommended approach for tests without server-side state. For tests that modify server-side state, the recommended approach is one account per parallel worker.
- **A namespace per test** (Kubernetes e2e, [framework.go](https://github.com/kubernetes/kubernetes/blob/master/test/e2e/framework/framework.go)). Every test has at least one namespace unless creation is skipped. `BeforeEach` creates it, and failed tests dump its contents.
- **A database per test** (`#[sqlx::test]`, [docs](https://github.com/launchbadge/sqlx/blob/main/src/macros/test.md)).
  - Each annotated test gets a new database with migrations applied.
  - Fixtures are SQL scripts, composable, named in the attribute.
  - Databases of passing tests are deleted, and those of failing tests are kept for debugging. SQLite needs no `DATABASE_URL`.
- **Server-side state keyed per test** (wptserve's [stash](https://github.com/web-platform-tests/wpt/blob/master/tools/wptserve/docs/stash.rst)). Keys must be UUIDs so that clients never collide. Values are write-once, read-once, and visible only to the same path by default.
- **Copies and wipes in Deno and Crawlee.** Deno's `tempDir: true` copies the test's files to a temporary directory before running. Crawlee's runner clears storage between scenarios ([run.mjs](https://github.com/apify/crawlee/blob/master/test/e2e/run.mjs)).
- **Owned target pages.** Stagehand's evals load copies of pages from a site Browserbase controls; Crawlee is moving scenarios from live sites to local servers (§2.4).

### 4.2 Fit for Goodfetch

*Reading, not decision.*

- **Versioned seeds.** A journey's seeded data in Goodfetch is mostly Proving ground pages: a pure function of (seed, family, page id), tasks with seed-derived IDs, and the oracle (area 2's spec). proptest's and Faker's caveats say the seed is only half the key: the Proving ground's version must travel with it. Area 2's result document already records "the Proving ground's version and seed", so a journey entry could pin (Proving ground version, seed, task IDs) the same way.
- **Engine state starts from scratch.** Engine-side state needs no factory: each run has its own SQLite run file (ADR 0002), and STYLE.md already prefers SQLite in a temporary file. The `#[sqlx::test]` and Kubernetes-namespace pattern means one fresh store per journey run, removed on success and kept on failure.
- **Logins.** Journeys that need a login can follow Playwright's split: a setup step saves a Saved identity once, and journeys that change server-side state get one account each.
- **Per-task server state.** The Proving ground's per-task server-side observations already resemble wptserve's stash: state keyed by task.

---

## 5. In the repository or in a test-management service

### 5.1 Facts

| | Qase | TestRail | Allure TestOps | In the repository (Kubernetes, WPT, Chromium, OpenTelemetry, OpenFastTrace) |
|---|---|---|---|---|
| Where cases live | Qase cloud, project code plus number | TestRail, case IDs `C123` | Allure TestOps database (Allure ID) | Files in git |
| How code links to a case | `qase(1, 'title')`, `qase.id(1)` or a `QaseID` annotation (Playwright reporter 2.7.0); can auto-create cases | JUnit name `[C123] test_case_1`, a `test_id` property, or an `automation_id` built from class and name (`trcli` v1.15.3, 2026-09-23) | `@AllureId("123")` (Java), `@allure.id("123")` (pytest), `@allure.id:123` in a Playwright title ([how-to](https://docs.qameta.io/allure-testops/howto/test-cases/automate-manual)); otherwise a generated ID | The ID is the path, name or metadata in the same commit |
| What a rename does | Not documented in the sources read | With `automation_id` matching, a renamed test creates a new case unless the field is updated by hand ([trcli README](https://github.com/gurock/trcli)) | Generated IDs break on a method rename; bind an explicit ID first ([test keys](https://docs.qameta.io/allure-testops/briefly/test-cases/test-keys)) | Reviewed in the pull request that renames |
| Versions and review | Case reviews: proposed through the API, approved and merged only in the UI ([qase-mcp-server](https://github.com/qase-tms/qase-mcp-server)); traceability snapshots | Version compare and restore, and case reviews with approvals, in TestRail Enterprise since 7.3 (2022-01-10), optional per project ([announcement](https://www.testrail.com/blog/announcing-testrail-7-3/)) | Not checked | git history; pull-request review (Kubernetes adds a review board) |
| CI checks | Reporter uploads results | `trcli` parses JUnit, Robot or Cucumber output and uploads | Results uploaded (`allurectl`); automated cases are created or updated from them once the launch is closed ([test cases](https://docs.qameta.io/allure-testops/briefly/test-cases/)) | Regenerate and diff; audit jobs (§3.2) |
| Agent access | Official MCP server, hosted or self-run, MIT, 42 tools (created 2025-09-23, pushed 2026-10-13) | No official MCP server found; several community ones | Not checked | Files an agent reads and edits like code |

### 5.2 Fit for Goodfetch

*Reading, not decision.*

- **Why the facts point to the repository.** Every leader studied here (Kubernetes, web-platform-tests, Chromium, OpenTelemetry) keeps its catalog in the repository. The services' advantages are a UI, manual-run tracking and history, and they depend on an ID written into the test anyway. Three Goodfetch facts weigh against a service:
  - The Proving ground and benchmark are published so anyone can rerun them (ADR 0025); a catalog in a paid service would be invisible to outside contributors and competitors.
  - CI jobs run under ADR 0043's outbound deny, with only named jobs reaching outside (area 2's spec), so a reporter uploading to a service is one more named exception.
  - Delivery is autonomous with fresh reviews on pull requests (AGENTS.md), and a service's reviews live outside them.
- **What a service would add for manual runs**, its strength (§6), can be had in the repository as a recorded report per owner test, the way Android's CTS Verifier saves one.

---

## 6. Manual testing that remains

### 6.1 Facts

- **web-platform-tests** ([manual.md](https://github.com/web-platform-tests/wpt/blob/master/docs/writing-tests/manual.md)).
  - Manual tests are named `name-manual.ext`, and an automated test is preferred whenever possible: automated tests run hundreds of times a day, manual ones a handful of times a year.
  - The doc lists what still needs a person: observing animation or video, browser security UI, OS interaction such as drag and drop from the desktop, non-default browser configuration, and the physical environment (vibration, sensors).
- **Android's CTS Verifier** supplements the automated Compatibility Test Suite (page updated 2026-08-20, [source.android.com](https://source.android.com/docs/compatibility/cts/verifier)).
  - It covers APIs that need a person to interact with or position the device: audio quality, touchscreen, accelerometer, camera.
  - The device under test must already pass CTS.
  - Results are saved as a report on the device and copied to a computer.
- **Session-based test management.** James and Jon Bach describe it as activity-based test management, an alternative to artifact-based management ([satisfice.com](https://www.satisfice.com/download/session-based-test-management), created 2000-11-01). Test session reports are part of it ([satisfice.com/sbtm](https://www.satisfice.com/sbtm)). The charter, session-sheet and debrief formats could not be read in a primary source (§7).
- **Kubernetes** leaves out of conformance what cannot be tested through the API from inside the cluster (root on nodes, raw network interfaces, provider-specific features). The audit records those endpoints in `ineligible_endpoints.yaml` with a reason (§3.2). It does not write manual tests for them.
- **Allure** marks a test `@allure.manual` to create or update a manual case in Allure TestOps from the automated result (§3.3).

### 6.2 Fit for Goodfetch

*Reading, not decision.*

- **The owner tests as manual journeys.** ADR 0058's owner tests are the manual gate between milestones. The CTS Verifier gives a model: an automated suite that must pass first, then a short list of person-only checks whose results are saved as a report.
- **The same catalog.** Recorded as catalog entries marked manual, with the same IDs as automated journeys where both exist (in the way web-platform-tests marks manual tests by name), the owner's scenarios would be counted by the same "every row has a test" check (§3.4), as manual.
- **What needs a person.** web-platform-tests' list (visual judgement, OS dialogs, physical devices) is close to Goodfetch's: the Live view with takeover, a Human pause solved by a person, MFA on a real phone, the look of the Console. Everything else has an automated path through the Proving ground.

---

## 7. Unverified

- **Session-based test management details**: the charter template ("Explore *target* with *resources* to discover *information*", attributed to Elisabeth Hendrickson's *Explore It!*), session length, the session sheet, debrief and TBS metrics. Seen only in secondary summaries; the Bachs' PDF was not reachable.
- **GitLab's manual verification step** (`workflow::verification`, testing in staging): from a search-result summary of the GitLab handbook; the page's body was not read.
- **TestRail**: no official MCP server was found, which does not prove there is none. The current plan list for case versioning: the support article returned HTTP 403; the 7.3 announcement (2022) is the source used.
- **Qase**: whether test cases keep a per-case version history, how a rename of a linked test behaves, and which plan includes the traceability matrix. Not found in the docs read.
- **Allure TestOps**: agent access (MCP or similar) and case review and versioning. Not checked. Its test-keys page is undated.
- **nextest**: *why* it has no tag predicate (that libtest's test list carries only names) is this note's inference; the reference only shows that no such predicate exists.
- **Vitest's JUnit reporter**: whether it writes tags. Not checked.
- **Google's CUJ material** read here is SRE guidance on SLOs plus Chromium's engineering docs. No Google document was found that describes a company-wide CUJ test catalog format.
