# Flaky tests without retries

Research for [#270](https://github.com/Jbmanllr/automan/issues/270), 2026-10-13. R&D material: nothing here is an adopted decision.

## Question

What do large engineering organizations and the research literature say about preventing, detecting and handling flaky tests when retries are not allowed, and which mechanisms fit Goodfetch's runners (nextest, Vitest, Playwright Test, pytest)?

Context: `STYLE.md` says "A flaky test is quarantined with an issue, never retried silently"; the owner wants no flaky tests and no retries.

A flaky test is one that "exhibits both a passing and a failing result with the same code" (Google's definition, used by GitHub and by most papers below). This note writes "flaky test" or "flaky failure", never the bare noun "flake", which `GLOSSARY.md` lists as an _Avoid_ term of Drift.

Versions checked on 2026-10-13: cargo-nextest 0.9.148 (2026-10-08), Vitest 5.0.4 (2026-10-12), Playwright 1.64.0 (2026-10-07), pytest 9.1.1 (2026-06-19), pytest-randomly 5.0.0 (2026-09-01), pytest-repeat 0.9.4 (2025-04-07), pytest-flakefinder 1.1.0 (2022-10-26), pytest-rerunfailures 16.7 (2026-09-17), pytest-socket 0.8.1 (2026-08-19).

## Summary

- **Every runner Goodfetch uses defaults to zero retries** (nextest `retries = 0`, Vitest `retry: 0`, Playwright "by default failing tests are not retried", pytest has none without a plugin). The one exception is Playwright's project template, which writes `retries: process.env.CI ? 2 : 0`. "No retries" is therefore the default to keep, not a setting to build.
- **Most large organizations retry, one large open-source project does not.** Google, Chromium, Meta, GitHub and Uber rerun failing tests before merge; Chromium says that with its ~20,000 flaky tests "pretty much no CL would ever pass" without retries. Kubernetes has had a "zero-flake" policy since 2019-12-13: "Test jobs must not automatically retry on test failures", with quarantine by a `[Flaky]` name tag plus a top-priority issue.
- **Most flaky tests are flaky from the day they are written** (78% in Luo et al. 2014). Chromium (20 runs of each new test, fatal if flaky), Mozilla (20 runs plus 10 browser restarts plus chaos mode for each added or changed test), web-platform-tests (10 restarts) and Datadog (up to 10 runs) all stress new or changed tests before merge. All four runners can repeat a test (`--stress-count`, `--repeats`, `--repeat-each`, pytest-repeat `--count`). A clean result proves little: Gruber et al. estimate 170 reruns for 95% confidence that a passing Python test is not flaky.
- **Random order finds order dependence**, the top cause in Python (59% of 7,571 flaky tests). pytest-randomly shuffles by default once installed and prints its seed; Vitest's `sequence.shuffle` is off by default and has an open bug where the seed does not reproduce file order (#11340); Playwright added `--shuffle [seed]` in 1.64 (October 2026); nextest has no shuffle, but runs every test in its own process, which removes in-process shared state.
- **Runners can rerun for diagnosis without turning red into green.** nextest `flaky-result = "fail"` (0.9.131) and Playwright `--fail-on-flaky-tests` (1.45) / `failOnFlakyTests` (1.52) still fail the run when a retry passes, but label the failure flaky. GitHub's three diagnostic reruns (same process, shifted in time, other host) classify 90% of flaky failures.
- **Root causes:** async waits lead every study (74 of 161 classified fixes in Luo 2014; 45.1% of UI flaky tests; 33.9% in a preliminary Rust study), then concurrency, order dependence, network, time, and resource limits (46.5% of flaky tests change failure rate with the CPU and memory available). Larger tests are flakier: at Google 0.5% of small, 1.6% of medium and 14% of large tests had a flaky run in a week.
- **After merge:** all four runners write JUnit XML, which every flaky-test tracker reads (Trunk, Datadog, BuildPulse, Codecov, Allure, ReportPortal). Without retries, "same commit, different results" only appears when the same commit runs more than once, for example in the merge queue, on `main` and in a nightly run.
- **Quarantine works when it has an owner, a deadline and a place where the test still runs.** Mozilla escalates in 2 business days then disables; Kubernetes files the issue in the current milestone as `priority/critical-urgent`; Uber files a ticket with a deadline; Chromium's disabled tests keep running on "reviver" builders; Datadog and Trunk keep quarantined tests running so a fix can be confirmed.
- **Non-deterministic checks live outside the blocking suite everywhere:** Mozilla's tier-1 jobs may not touch sites "whose content we do not control"; Kubernetes splits Blocking and Informing jobs; Chromium moves flaky tests to "FYI" builders; LangChain blocks sockets in unit tests and runs live-credential tests daily; Stagehand gates pull requests on live-model evals by a score (80% over 3 trials), not by pass or fail.

## 1. Policies: who retries, who quarantines, who reverts

### Facts

**Google**
- About 1.5% of all test runs report a flaky result, almost 16% of tests have some flakiness, and about 84% of observed pass-to-fail transitions involve a flaky test. Presubmit can rerun failing tests automatically, and a test marked flaky reports a failure "only if it fails 3 times in a row"; the author says this "encourages developers to ignore flakiness". A tool quarantines tests whose flakiness is too high, "removes the test from the critical path and files a bug", which "could easily mask a real race condition" ([Google Testing Blog, John Micco, 2016-05-27](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html)).
- Of 4.2 million tests, about 63,000 had a flaky run in a week; flakiness rises with test size (small 0.5%, medium 1.6%, large 14%), RAM use (r² 0.76) and binary size (r² 0.82); Java WebDriver tests were 10.45% flaky, Python WebDriver 18.72%, Android emulator 25.46%, and "size is more predictive than tool" ([Jeff Listfield, 2017-04](https://testing.googleblog.com/2017/04/where-do-our-flaky-tests-come-from.html)).
- Rerunning "is effectively trading CPU cycles for engineering time" and "is only delaying the need to address the root cause"; "as you approach 1% flakiness, the tests begin to lose value"; Google's flaky rate "hovers around 0.15%" ([Software Engineering at Google, ch. 11](https://abseil.io/resources/swe-book/html/ch11.html)).
- Bazel's `flaky = True` runs a test "up to three times, marking it as failed only if it fails each time", and "use of this attribute is generally discouraged" ([Bazel common definitions](https://bazel.build/reference/be/common-definitions)).

**Chromium**
- The commit queue retries each failed test shard "with the exact same configuration", then reruns the failing suite without the change, and fails the change only if a test failed twice with it and passed without it. Rationale: "adding retries increases the probability that a flaky test will land on tip of tree sublinearly, but mitigates the impact of the flaky test on unrelated CLs exponentially", and with "roughly ~20,000 unique flaky tests, without retries, pretty much no CL would ever pass the CQ" ([docs/infra/cq.md](https://chromium.googlesource.com/chromium/src/+/main/docs/infra/cq.md)).
- New tests in new test files are rerun on try builds (`repeat_count` default 20, at most 40 test targets); a flaky new test in a non-experimental suite fails the build ("fatal"). The footer `Validate-Test-Flakiness: skip` turns this off ([flakiness recipe module](https://chromium.googlesource.com/chromium/tools/build/+/main/recipes/recipe_modules/flakiness/api.py), [cq.md](https://chromium.googlesource.com/chromium/src/+/main/docs/infra/cq.md)).
- LUCI Analysis groups failures into clusters and files bugs automatically. Chromium's policies: P0 when 10 or more changes are blocked in a day ("Please disable the test(s) and priorise immediate action"); P1 when 7 or more gardened builds fail in 3 days, with remedies "in order of resolution preference": fix the test, move it to an FYI builder, or disable it ("least desirable as it reduces test coverage"); disabled tests "might continue running in reviver builders" ([luci-analysis.cfg](https://chromium.googlesource.com/chromium/src/+/main/infra/config/generated/luci/luci-analysis.cfg), [gtest_flake_tips.md](https://chromium.googlesource.com/chromium/src/+/main/docs/testing/gtest_flake_tips.md)).

**Mozilla**
- Sheriffs classify every failure on visible (tier 1 and 2) jobs; one tracking bug per test file; the triage owner is needinfo'd at "30 failures a week" ([Sheriffed intermittent failures](https://firefox-source-docs.mozilla.org/testing/sheriffed-intermittents/index.html)).
- Disabling policy (wiki, last edited 2017-05-30): escalate a test that is in the top 20, fails ≥8% of the time, or has >100 failures in 30 days; 2 business days for the author, 2 more for the module owner, then disable. A test landed or modified in the last 7 days is backed out; a test failing ≥30% is disabled first ([Test Disabling Policy](https://wiki.mozilla.org/Sheriffing/Test_Disabling_Policy)).
- Job tiers (last edited 2022-11-21): tier 1 breakage closes the tree; tier 2 failures get a bug and are "expected to be fixed in 2 business days"; tier 3 is monitored only by its owner. A new tier-1 suite must have "at most a 5% per job failure rate initially, and ideally <1% longer term"; trunk's typical intermittent failure rate is 2 to 4% ([Job Visibility Policy](https://wiki.mozilla.org/Sheriffing/Job_Visibility_Policy)).

**Meta**
- The probabilistic flakiness score (PFS), deployed since mid-2018, estimates each test's flakiness by Bayesian inference from results CI already produces, with no extra runs. "All end-to-end tests have some degree of flakiness"; PFS is the "second most powerful weapon", "trailing behind only the resource-intensive retries". When a score regresses, a ticket goes to the declared owner; tests not fixed in time are marked flaky and become "ineligible for change-based testing". The flakiness floor is "well below 1 percent" for unit tests and reaches 10% for some end-to-end frameworks ([Engineering at Meta, 2020-12-10](https://engineering.fb.com/2020/12/10/developer-tools/probabilistic-flakiness/)).

**GitHub**
- In GitHub's monolith, 1 in 11 commits had a red build caused by a flaky test. A failed test is rerun three times: in the same process, in the same process "shifted into the future", and on a different host. This identifies 90% of flaky failures (25% before), keeps the build green, scores each test's impact, and opens an issue assigned to whoever last changed the test or code. Commits with a flaky build fell to under 0.5%, "an 18x improvement" ([GitHub Blog, 2020-12-16, updated 2021-12-20](https://github.blog/engineering/engineering-principles/reducing-flaky-builds-by-18x/)).

**Uber**
- Testopedia tracks every test through the states new, stable, unstable, disabled and deleted; entering unstable files a JIRA ticket "with deadline" to the owning team. "Our general guidance is to avoid running flaky tests in CI", except tests marked critical; integration tests run "in non-blocking mode as FYI only". Uber runs all tests on `main` periodically "with limited resources" to expose resource-sensitive flakiness, and its landing pipeline has retry logic. It reports about 1,000 flaky tests of 600,000 in Go and 1,000 of 350,000 in Java ([Uber Blog, 2024-06-04](https://www.uber.com/en-US/blog/flaky-tests-overhaul/)).

**Spotify**
- Showing teams a table of their flaky tests "reduced test flakiness at Spotify from 6% to 4% in two months"; Odeneye visualizes results over time; Flakybot lets an engineer check a pull request's tests for flakiness before merging; end-to-end tests "are flaky by nature. Write fewer of them" ([Spotify Engineering, Jason Palmer, 2019-11-18](https://engineering.atspotify.com/2019/11/test-flakiness-methods-for-identifying-and-dealing-with-flaky-tests)).

**Kubernetes** (comparable large open-source project)
- "The project has a 'zero-flake' policy. Test jobs must not automatically retry on test failures", in effect since 2019-12-13 and "confirmed as policy in 2023". Quarantine: add `[Flaky]` to the test name (most jobs exclude it) with an issue in the current milestone labelled `priority/critical-urgent`, `lifecycle/frozen` and `kind/flake`. An assigned flaky test "is more important than anything else you're doing"; "every time a flake comes back, at least 2 hours of merge time is wasted"; an issue may be closed after 3 months without a recurrence ([flaky-tests.md](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-testing/flaky-tests.md), last changed 2026-03-02).
- A release-blocking job must "be able to pass 3 times in a row against the same commit", pass 75% of its runs in a week and have a responsive owner ([release-blocking-jobs.md](https://github.com/kubernetes/sig-release/blob/master/release-blocking-jobs.md)).

| Organization | Retries before merge | Detection | Action on a flaky test | Time to fix, ownership |
|---|---|---|---|---|
| Google | Yes (rerun, "flaky" = fails 3 times) | Flakiness monitor, change-of-flakiness finder | Automatic quarantine plus bug | Bug to developers |
| Chromium | Yes (shard retry, then without the change) | New tests run 20 times; LUCI Analysis clusters | Fix, else move to FYI builder, else disable (reviver builders keep running it) | Auto bugs P0 to P2 by impact |
| Mozilla | Sheriffs retrigger to classify | Test verification on changed tests; Treeherder | Back out if landed in last 7 days; else escalate, then disable | 2 + 2 business days |
| Meta | Yes ("most powerful weapon") | PFS from normal CI results | Excluded from change-based testing | Ticket to declared owner |
| GitHub | Yes, 3 diagnostic reruns, build kept green | Rerun classification, impact score | Auto issue | Assigned to last author |
| Uber | Yes in landing pipeline | Testopedia analyzers, limited-resource runs of `main` | Skipped in CI or FYI only | JIRA ticket with deadline |
| Spotify | Not stated | Odeneye, per-team table, Flakybot | Not stated | Teams |
| Kubernetes | **No** (since 2019) | TestGrid, triage dashboard | `[Flaky]` quarantine with issue | Milestone issue, top priority |

**Complaints.** Google: "developers dismiss a failing result as flaky only to later realize that it was a legitimate failure". In a survey of 335 developers, they worried less about compute than about "the loss of trust in the test outcomes" ([Gruber and Fraser, ICST 2022](https://arxiv.org/abs/2203.00483)). Builds released despite flaky failures had a median of 514 (Beta) and 234 (Production) crash reports, against 2 for builds whose tests all passed ([Rahman and Rigby, ESEC/FSE 2018](https://doi.org/10.1145/3236024.3275529)).

### Fit for Goodfetch

- Kubernetes is the closest precedent for the owner's rule: a large open-source project, no automatic retries, quarantine by name tag with an issue. Chromium's argument for retries rests on tens of thousands of existing flaky tests, which a new repository does not have.
- Every source that quarantines pairs it with an owner and a clock (Mozilla 2 + 2 business days, Kubernetes milestone issue, Uber deadline, Meta exclusion when late) and with a place where the quarantined test keeps running (Chromium reviver builders, Datadog and Trunk). `STYLE.md` already requires the issue; the owner, deadline and still-running place are the parts it does not yet say.
- Mozilla backs out a test landed in the last 7 days rather than quarantining it; Chromium blocks a flaky new test before it lands. Both treat a new flaky test as the change's fault, not the suite's.
- GitHub's re-run button (and `gh run rerun --failed`) lets anyone with write access re-run failed jobs, up to 50 times within 30 days, and `GITHUB_RUN_ATTEMPT` counts attempts ([re-running workflows](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs), [variables](https://docs.github.com/en/actions/reference/workflows-and-actions/variables)). A manual re-run of the "all green" job (ADR 0032) is a retry no runner setting prevents.

## 2. Before merge: stress runs, random order, order dependence

### Facts

**Why new tests.** "Most flaky tests (78%) are flaky the first time they are written", so "techniques that extensively check tests when they are first added can detect most flaky tests" ([Luo et al., FSE 2014](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf)). Detecting flakiness by reruns needs many runs: "a 95% confidence that a passing test case is not flaky on average would require 170 reruns" ([Gruber et al., ICST 2021](https://arxiv.org/abs/2101.09077)).

**What organizations run on new or changed tests**
- Chromium: each new test 20 times on try builds; flaky is fatal (section 1).
- Mozilla test verification: for each added or modified test, "Run each test 20 times in one browser", then "10 times in a new browser each time", then the sequence again in chaos mode; capped at 1 hour (`--max-verify-time` default 3600 s); an orange result means "Backout should be considered (but is not mandatory)"; `skip-if = verify` opts a test out ([Test Verification](https://firefox-source-docs.mozilla.org/testing/test-verification/index.html)).
- web-platform-tests: every pull request runs the affected tests with `--verify --verify-repeat-loop=0 --verify-repeat-restart=10` in Firefox Nightly (required) and Chrome Dev (not required) ([tools/ci/tc/tasks/test.yml](https://github.com/web-platform-tests/wpt/blob/master/tools/ci/tc/tasks/test.yml)).
- Datadog Early Flake Detection: a test not in the known-tests list "is automatically retried up to ten times"; any failure tags it flaky; tests slower than five minutes are skipped; a PR Gate can block the merge ([docs](https://docs.datadoghq.com/tests/flaky_tests/early_flake_detection/)).
- Spotify's Flakybot runs a pull request's tests on demand (section 1).

**Runner support**

| | nextest 0.9.148 | Vitest 5.0.4 | Playwright Test 1.64.0 | pytest 9.1.1 and plugins |
|---|---|---|---|---|
| Default retries | `retries = 0` ([default-config.toml](https://github.com/nextest-rs/nextest/blob/main/nextest-runner/default-config.toml)) | `retry` default `0` ([retry](https://vitest.dev/config/retry)) | "By default failing tests are not retried"; template sets `retries: process.env.CI ? 2 : 0` ([retries](https://playwright.dev/docs/test-retries), [create-playwright template](https://github.com/microsoft/create-playwright/blob/main/assets/playwright.config.ts)) | None in core; pytest-rerunfailures adds them |
| Fail when a retry passes | `flaky-result = "fail"`, `--flaky-result fail` (0.9.131) ([retries](https://nexte.st/docs/features/retries/)) | No option found | `--fail-on-flaky-tests` (1.45), `failOnFlakyTests` (1.52) ([release notes](https://playwright.dev/docs/release-notes)) | n/a |
| Repeat each test | `--stress-count N`, `--stress-duration` (0.9.103), one copy at a time ([stress tests](https://nexte.st/docs/features/stress-tests/)) | `--repeats N`: "If a test fails on any repetition, the whole test is reported as failed" ([repeats](https://vitest.dev/config/repeats)) | `--repeat-each N` ([CLI](https://playwright.dev/docs/test-cli)) | pytest-repeat `--count N` (with `-x` to stop at first failure); pytest-flakefinder `--flake-finder`, default 50 runs, last release 2022 |
| Select new or changed tests | Filtersets such as `rdeps(pkg)`; JSON listing to diff test names ([filtersets](https://nexte.st/docs/filtersets/)) | `--changed [ref]` ([changed](https://vitest.dev/config/changed)) | `--only-changed [ref]` (1.46) | None in core |
| Random order | None; process per test; libtest's `--shuffle` is unstable ([cli.rs](https://github.com/rust-lang/rust/blob/master/library/test/src/cli.rs)) | `sequence.shuffle` default `false`, `sequence.seed` default `Date.now()` ([sequence](https://vitest.dev/config/sequence)) | `--shuffle [seed]`, seed printed (1.64) | pytest-randomly: on when installed, prints `--randomly-seed`, resets `random.seed()` per test, `--randomly-seed=last` ([README](https://github.com/pytest-dev/pytest-randomly)) |
| Isolation default | Each test in its own process, "now, and will always be" ([why process-per-test](https://nexte.st/docs/design/why-process-per-test/)) | `isolate: true`; `restoreMocks`, `mockReset`, `unstubEnvs`, `unstubGlobals` default `false` | A failure discards the worker process and browser ([retries](https://playwright.dev/docs/test-retries)) | Shared process |
| JUnit XML | Yes, with `<flakyFailure>` and `<rerunFailure>` ([JUnit](https://nexte.st/docs/machine-readable/junit/)) | `junit` reporter | `junit` reporter | `--junitxml` |
| Exclude a quarantined test | Filterset (`not test(...)`), per-test overrides | Tags (4.1) and `--tags-filter` ([tags](https://vitest.dev/config/tags)) | Tags with `--grep-invert`, or `--test-list-invert <file>` (1.56) | Markers with `-m "not ..."` |

**Order dependence**
- Order dependence caused 12% of classified fixes in Luo 2014 (19 of 161), 74% of which were fixed "by cleaning the shared state between test runs"; 50.5% of 422 flaky Java tests in iDFlakies ([Lam et al., ICST 2019](https://doi.org/10.1109/ICST.2019.00038)); 59% of 7,571 flaky Python tests ([Gruber et al.](https://arxiv.org/abs/2101.09077)).
- In 81 JavaScript (Jest) projects, 55 order-dependent tests came from shared files (13) and "shared mocking state" (42), a cause "not reported previously" ([Hashemi et al., ICST 2025](https://arxiv.org/abs/2501.12680)).
- pytest-randomly's README cites research that random order "can effectively detect such dependencies", and notes googletest's shuffling.

**Complaints and requests**
- Vitest [#11340](https://github.com/vitest-dev/vitest/issues/11340) (open, 2026-09-24): with `sequence.shuffle.files`, "the same seed gives different file orders between runs", because glob order varies before the shuffle. Earlier issues reported the seed not being printed ([#2876](https://github.com/vitest-dev/vitest/issues/2876), [#9557](https://github.com/vitest-dev/vitest/issues/9557), both closed).
- nextest's `flaky-result = "fail"` was added for users who "would like to retry failing tests, but then mark them as failures" ([PR #3148](https://github.com/nextest-rs/nextest/pull/3148)).

### Fit for Goodfetch

- Each runner can run the stress check that Chromium and Mozilla run, without a plugin except in Python (pytest-repeat; pytest-flakefinder has had no release since 2022). Selecting "new or changed tests" is built into Vitest and Playwright; for nextest and pytest it means diffing test listings between the base and the change, which is how Chromium finds new tests.
- 20 runs catches tests that fail often, not rare ones (170 runs for 95% confidence on Python). Stress runs before merge and history after merge cover different failure rates.
- pytest-randomly fits the Python SDK, where order dependence dominates the evidence. For Vitest, shuffling works but reproduction by seed is broken for file order today (#11340), and mock and stub restoration is off by default while shared mocking state is the main JavaScript cause found. nextest's process-per-test model removes in-process order dependence; shared files, ports and Proving ground state between processes remain.
- A runner-level "retry, then fail anyway" (nextest `flaky-result = "fail"`, Playwright `failOnFlakyTests`) never turns a red run green but reports the failure as flaky. Whether that counts as a retry under the owner's rule is for the decision; the facts are that it costs the extra attempts and does not change the verdict.

## 3. After merge: result history and flaky-failure rates

### Facts

- All four runners write JUnit XML (section 2 table); every tracker below reads it or a library of its own.
- Developers surveyed want "dashboards showing test outcomes over time" ([Gruber and Fraser, 2022](https://arxiv.org/abs/2203.00483)); Spotify's single table moved its flakiness from 6% to 4% (section 1).

| Tool | Input | How it flags a flaky test | Quarantine | Notes |
|---|---|---|---|---|
| [Trunk Flaky Tests](https://docs.trunk.io/flaky-tests/detection) | JUnit uploaded by its CLI | Monitors: pass-on-retry (same commit), failure rate or failure count per branch and window | The CLI asks Trunk's backend after the run and "overrides the exit code" when every failure is quarantined; quarantined tests keep running and uploading; `--dry-run` gates without uploading ([quarantining](https://docs.trunk.io/flaky-tests/quarantining)) | Free up to 5 committers ([pricing](https://trunk.io/pricing)) |
| [Datadog Test Optimization](https://docs.datadoghq.com/tests/flaky_management/) | Datadog library inside the test process (dd-trace-js ≥ 5.23, dd-trace-py ≥ 3.0 with pytest ≥ 7.2, Playwright ≥ 1.38) | Early Flake Detection (new tests up to 10 runs), flakiness on branches | Quarantined (runs, failures ignored), Disabled (skipped); policies, for example disable after 30 days; Fixed after 30 days without a flaky result; "attempt to fix" reruns the test 20 times | Commercial; Rust is not in the library list |
| [BuildPulse](https://buildpulse.io/docs/flaky-tests/guides/Test%20Quarantining) | JUnit upload | Disruption percentage after at least 10 results | Quarantine API lists tests for CI to skip | Commercial |
| [Codecov Test Analytics](https://docs.codecov.com/docs/test-analytics) | JUnit XML only | Failure rates on `main` and branches | None; PR comment lists failed tests "known to be flaky on main" | [Pricing](https://about.codecov.io/pricing/) lists Flaky Test Detection and unlimited public-repo uploads |
| [Allure Report 3](https://allurereport.org/docs/test-stability/) | Allure results, or JUnit XML through its `junitxml` reader ([source](https://github.com/allure-framework/allure3/tree/main/packages/reader/src)) | "Flaky" mark if a test failed within the latest 5 launches, passed since, and failed in the latest | None | Apache-2.0; history kept in one `history.jsonl` file ([history](https://allurereport.org/docs/history-and-retries/)) |
| [ReportPortal](https://reportportal.io/docs/dashboards-and-widgets/FlakyTestCasesTableTop50/) | Agents or imports | Status changed at least once in the last 2 to 100 launches (default 30) | None | Apache-2.0, self-hosted server |
| LUCI Analysis, Treeherder, TestGrid | Their own result stores | Clusters, sheriff classification, result grids | Through their projects' policies | Built for Chromium, Mozilla and Kubernetes |

- GitHub detects a flaky result also by comparing builds of the same root tree hash: "If another build had different results ... the test failure was marked as flaky", which "only worked if a build was retried" (section 1 source).
- Meta's PFS needs no extra runs; it fits a model to the results CI already produces (section 1).
- Uber feeds two streams to Testopedia, periodic limited-resource runs of `main` and the landing pipeline, labelled so they do not interfere (section 1).

### Fit for Goodfetch

- With no retries, a flaky result shows up only as the same commit giving different results in different executions. ADR 0032 already runs the same commit more than once: pull request, merge queue, `main`, nightly. Recording results by commit across those tiers is what makes history work, and nightly runs on `main` are the "periodic" stream Uber and Meta use.
- ADR 0043's CI rule denies outbound traffic in every step that runs the product. Datadog's library reports from inside the test process, which that rule would block; tools that read JUnit files can upload from a separate job that runs none of the product, which the rule places outside it.
- Trunk's and Datadog's quarantine turn a failing job green after the run, by a setting held in their service. A quarantine list kept in the repository (a nextest filterset, Vitest tags, Playwright's `--test-list-invert` file, pytest markers) puts each quarantine through code review instead, next to its issue.
- Open-source history tools (Allure, ReportPortal) keep data on infrastructure Goodfetch controls; the three project-scale systems (LUCI Analysis, Treeherder, TestGrid) are larger than a single repository needs.

## 4. Root causes and fixes

### Facts

**Luo et al. 2014** ([FSE 2014](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf), DOI [10.1145/2635868.2635920](https://doi.org/10.1145/2635868.2635920)): 201 commits that fix flaky tests in 51 Apache projects, 161 classified.

| Cause | Fixes (of 161) | How fixed |
|---|---|---|
| Async wait | 74 | 54% by waiting for the event (`waitFor`); 34% of these tests enforced ordering with a time delay |
| Concurrency | 32 | 31% locks, 25% making code deterministic |
| Test order dependency | 19 | 74% by cleaning shared state |
| Resource leak | 11 | |
| Network | 10 | |
| Time | 5 | |
| IO | 4 | |
| Randomness | 4 | |
| Floating point | 3 | |
| Unordered collections | 1 | |

**Later studies**
- A survey of 76 papers sorts the field into causes, costs, detection and mitigation; it cites a survey in which 59% of developers deal with flaky tests monthly or more often ([Parry et al., TOSEM 2021](https://doi.org/10.1145/3476105)).
- Mozilla developers classifying 200 flaky tests they fixed found four new causes, "the most costly to fix": too restrictive range, test case timeout, test suite timeout and platform dependency; reproducing the failure is the main difficulty ([Eck et al., ESEC/FSE 2019](https://arxiv.org/abs/1907.01466)).
- UI tests: 105 of 235 (45.1%) flaky UI tests come from async waits, 52.0% in web projects; besides deleting the test, fixes are mostly reworking logic (46.0%) and fixing delays (39.3%) ([Romano et al., 2021](https://arxiv.org/abs/2103.02669)).
- Python: order dependency 59%, test infrastructure 28%, network and randomness most of the rest ([Gruber et al., ICST 2021](https://arxiv.org/abs/2101.09077)).
- Rust (preliminary, 53 fixed tests): async wait 33.9%, concurrency 24.5%, logic errors 9.4%, network 9.4%; Rust's randomized hash order already exposes collection-order flakiness ([Schroeder et al., arXiv 2025](https://arxiv.org/abs/2502.02760)).
- Resources: 46.5% of flaky tests across 52 Java, JavaScript and Python projects and 27 resource configurations are "Resource-Affected Flaky Tests"; "reducing the resources available when running tests is a cost-effective approach to detect more flaky failures" ([Silva et al., IEEE TSE 2024](https://arxiv.org/abs/2310.12132)).
- Clusters: 75% of 810 flaky Java tests belong to clusters that fail together (mean 13.5 tests), mostly from "intermittent networking issues and instabilities in external dependencies" ([Parry et al., arXiv 2025](https://arxiv.org/abs/2504.16777)).
- Stress noise: Shaker injects CPU, memory and I/O stress; a September 2026 preprint finds no advantage over plain reruns on Python (37.2% against 35.8%), with network and randomness dominating the reproducible cases ([Leal et al., arXiv 2026](https://arxiv.org/abs/2609.25528)).
- Google groups causes by layer: the tests (initialization, cleanup, time, timing, order), the framework (resources, scheduling collisions), the system under test (races, leaks), the OS and hardware (network, disk, other tasks); hermetic environments "are less likely to be flaky" ([Google Testing Blog, 2020-12-16](https://testing.googleblog.com/2020/12/test-flakiness-one-of-main-challenges.html)).

**Cause by cause, with what the runners and libraries offer**
- *Async waits.* Mozilla: a fixed `setTimeout(handler, 500)` "should raise an alarm in your head ... wait for the operation to be completed explicitly", usually an event ([Avoiding intermittent tests](https://firefox-source-docs.mozilla.org/testing/intermittent/index.html)). Playwright auto-waits for actionability checks before each action and discourages `waitForTimeout`: "Tests that wait for time are inherently flaky" ([auto-waiting](https://playwright.dev/docs/actionability), [Page.waitForTimeout](https://playwright.dev/docs/api/class-page#page-wait-for-timeout)). Chromium: make sure "event listeners are spawned before invoking the event" and wait for the right event, not a proxy ([gtest_flake_tips.md](https://chromium.googlesource.com/chromium/src/+/main/docs/testing/gtest_flake_tips.md)). Google: "making tests reactive or event driven ... can also remove flakiness" ([SWE book ch. 14](https://abseil.io/resources/swe-book/html/ch14.html)).
- *Concurrency.* loom 0.7.2 (2024-04-23) runs a test many times "permuting the possible concurrent executions ... under the C11 memory model" ([loom](https://github.com/tokio-rs/loom)); shuttle 0.9.6 (2026-10-10) implements "randomized concurrency testing techniques", including a randomized scheduler with probabilistic guarantees of finding bugs, and wraps `tokio` and `std::sync` ([shuttle](https://github.com/awslabs/shuttle)).
- *Order and shared state.* Section 2. nextest test groups act as mutexes or semaphores for tests that share a service, a global resource or a fixed port (0.9.48), and `NEXTEST_TEST_GLOBAL_SLOT` gives each running test a unique number (0.9.90) ([test groups](https://nexte.st/docs/configuration/test-groups/), [environment variables](https://nexte.st/docs/configuration/env-vars/)).
- *Time.* Tokio's paused clock (`#[tokio::test(start_paused = true)]`) auto-advances to the next timer when the runtime is idle ([tokio::time::pause](https://docs.rs/tokio/latest/tokio/time/fn.pause.html)); Playwright's Clock API (1.45) controls page time; Mozilla's tier rules forbid "time bombs" such as tests that break when summer time starts or a run crosses midnight; GitHub reruns a failure "shifted into the future" to detect time assumptions.
- *Network.* Google's small tests may not touch the network, medium tests only `localhost` ([SWE book ch. 11](https://abseil.io/resources/swe-book/html/ch11.html)); Mozilla's tier-1 jobs set `MOZ_DISABLE_NONLOCAL_CONNECTIONS` and may not use sites "whose content we do not control/have no SLA"; pytest-socket's `--disable-socket` fails any socket use including DNS ([pytest-socket](https://github.com/miketheman/pytest-socket)); turmoil 0.7.2 simulates hosts, latency, drops and partitions in one thread under a seeded RNG ([turmoil](https://github.com/tokio-rs/turmoil)).
- *Leaks and timeouts.* nextest marks a test leaky when its output handles stay open after exit (`leak-timeout` 200 ms in the default config) and can fail it (`result = "fail"`, 0.9.95); it marks tests SLOW after 60 s and terminates them only when `terminate-after` is set ([leaky tests](https://nexte.st/docs/features/leaky-tests/), [slow tests](https://nexte.st/docs/features/slow-tests/)). Vitest's `detectAsyncLeaks` reports timers and other async resources left by a test file, default off and slow ([detectAsyncLeaks](https://vitest.dev/config/detectasyncleaks)).
- *Resource limits on shared CI runners.* GitHub-hosted runners for public repositories have 4 CPUs and 16 GB on Linux and Windows and 3 CPUs (M1) and 7 GB on macOS arm64; private repositories get 2 CPUs and 8 GB on Linux and Windows ([GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)). nextest's own CI retries some integration tests on macOS because they "are a bit flaky on Mac GHA CI runners due to resource exhaustion" ([nextest .config/nextest.toml](https://github.com/nextest-rs/nextest/blob/main/.config/nextest.toml)); uv's nextest profile has no retries ([uv .config/nextest.toml](https://github.com/astral-sh/uv/blob/main/.config/nextest.toml)).

### Fit for Goodfetch

- Goodfetch's browser tests drive a real Chromium against Proving ground pages (`STYLE.md`), which is the large, browser-driving kind Google measured at 10 to 19% flaky. The evidence points to two levers: keep tests small where the interface allows, and wait on events (CDP events, page signals), never on fixed sleeps.
- ADR 0043's outbound deny in CI already removes the network cause by construction, as Mozilla's `MOZ_DISABLE_NONLOCAL_CONNECTIONS` and Google's medium-test rule do. pytest-socket is the Python equivalent inside the process.
- The Rust evidence (async wait, then concurrency) matches the tools Tokio's ecosystem ships: paused time, loom, shuttle and turmoil.
- Before the repository is public, ADR 0032's pull-request tier runs on private Linux runners with half the CPU and memory of the public ones, and macOS arm64 runners have 3 CPUs and 7 GB. Resource-affected tests (46.5%) will behave differently across those tiers; Uber and Silva et al. use deliberately constrained runs to expose them.

## 5. Checks that are non-deterministic by nature

### Facts

- **Google:** many large tests "do not belong in TAP. They are nonhermetic, too flaky, and/or too resource intensive", so they run in "a separate post-submit continuous build", and are also encouraged presubmit ([SWE book ch. 14](https://abseil.io/resources/swe-book/html/ch14.html)).
- **Mozilla:** tier-1 jobs must avoid "pulling the tip of external repositories" and sites with no SLA; tier-3 jobs are hidden and "all responsibilities for monitoring the results" fall on the owner ([Job Visibility Policy](https://wiki.mozilla.org/Sheriffing/Job_Visibility_Policy)).
- **Kubernetes:** jobs that are "slow, flaky, infrequent, or difficult to maintain" go on the Informing dashboard; they "do not need to be green before authorizing a release", but the CI Signal team checks them regularly, and an unexplained failure that is not a known flaky one blocks the release until it is investigated ([release-blocking-jobs.md](https://github.com/kubernetes/sig-release/blob/master/release-blocking-jobs.md)).
- **Chromium:** flaky tests move from critical builders to "FYI" builders; disabled tests keep running on reviver builders (section 1).
- **Uber:** integration tests run "as FYI only" (section 1).
- **web-platform-tests:** the pull-request stability check runs in Firefox Nightly as a required check and in Chrome Dev as a non-required one (section 2).
- **Live models:** the Vercel AI SDK says language models "are non-deterministic and calling them is slow and expensive", and ships mock models (`MockLanguageModelV4`) and `simulateReadableStream` for "repeatable and deterministic" tests ([AI SDK testing](https://github.com/vercel/ai/blob/main/content/docs/03-ai-sdk-core/55-testing.mdx)). LangChain runs unit tests with `--disable-socket --allow-unix-socket`, replays VCR cassettes "in playback-only mode", and runs "integration tests against partner libraries with live API credentials" daily at 13:00 UTC ([Makefile](https://github.com/langchain-ai/langchain/blob/master/libs/partners/openai/Makefile), [integration_tests.yml](https://github.com/langchain-ai/langchain/blob/master/.github/workflows/integration_tests.yml)).
- **Competitors:** Stagehand's pull-request CI runs regression evals with live models through Browserbase, 3 trials each, and fails when the score is below 80%; the labels `skip-evals` and `skip-regression-evals` skip them ([ci.yml](https://github.com/browserbase/stagehand/blob/main/.github/workflows/ci.yml), as of 2026-10-08). Browser Use triggers an evaluation on an external platform for pull requests by members, with `continue-on-error: true`, and wraps each CI test in `nick-fields/retry` with `max_attempts: 1` ([eval-on-pr.yml](https://github.com/browser-use/browser-use/blob/main/.github/workflows/eval-on-pr.yml), [test.yaml](https://github.com/browser-use/browser-use/blob/main/.github/workflows/test.yaml)). Playwright's own library tests set `retries: process.env.CI ? 3 : 0` ([tests/library/playwright.config.ts](https://github.com/microsoft/playwright/blob/main/tests/library/playwright.config.ts)); Crawlee's Vitest config sets `retry: process.env.RETRY_TESTS ? 3 : 0` and its CI sets `RETRY_TESTS: 1` ([vitest.config.mts](https://github.com/apify/crawlee/blob/master/vitest.config.mts), [test-ci.yml](https://github.com/apify/crawlee/blob/master/.github/workflows/test-ci.yml)).

### Fit for Goodfetch

- ADRs 0032 and 0043 already keep these checks out of the pull-request suite: real SaaS Sink accounts, every Chrome channel and recent major, the Private component and the public testnets run nightly; the benchmark and the online conformance against real vendors run on release candidates, and the online conformance "never blocks a release". That is the Kubernetes Informing model and Mozilla's tier split.
- What the sources add is the "still act on them" half: Kubernetes has a CI Signal team that triages Informing failures and blocks on unexplained ones; Mozilla's tier 2 gets a bug and 2 business days; tier 3 belongs to a named owner. The ADRs name where these checks run, not who triages a red nightly or by when.
- `STYLE.md` already tests Model providers through "a mock adapter replaying recorded exchanges", the pattern of the AI SDK's mock models and LangChain's playback-only cassettes. Live-model quality, where it is checked, is scored over several trials against a threshold (Stagehand), which is a benchmark or eval, not a pass-or-fail test.
- Two direct competitors (Playwright's own suite, Crawlee) retry in CI; Kubernetes is the comparable project that does not.

## Unverified

- Chromium's Flake Portal: the current flaky-test doc sends readers to LUCI Analysis but still mentions "the flake portal"; whether the Findit Flake Portal is retired was not confirmed.
- The tier-by-feature mapping on Codecov's pricing page (whether Flaky Test Detection is in the free or open-source tier) was not confirmed; nor whether Trunk, BuildPulse or Codecov read nextest's `<flakyFailure>` elements.
- Allure Report 3's `junitxml` reader was seen in the source tree, not run; whether it keeps nextest's `<flakyFailure>` detail was not checked. ReportPortal's JUnit import was not checked.
- Vitest: no "fail when a retry passes" option was found in the config docs; absence not confirmed in the source. Whether the seed is now printed for every shuffle mode (#9557 closed 2026-02-08) was not tested.
- nextest's docs say the leak timeout defaults to 100 ms; the default config on `main` says 200 ms.
- Mozilla's test verification page first says a mochitest runs "10 times, then ... another 5 times", then shows a summary of "20 times" and "10 times"; the summary is quoted above.
- pytest-randomly 5.0.0: the README says "Python 3.11 to 3.15 supported"; PyPI metadata says `>=3.10`.
- Datadog's "up to 75% of flaky tests can be identified" cites an unnamed study.
- Romano et al.'s venue (often cited as ICSE 2021) was not confirmed; the arXiv version is cited.
- The Rust study (53 tests) and the Shaker evaluation (September 2026) are preprints, not peer reviewed.
- Google's figures date from 2016 to 2020, Spotify's from 2019, GitHub's from 2020; current rates at those companies are not public.
- Playwright's `--shuffle` shipped in 1.64.0 on 2026-10-07; behaviour with serial suites and sharding was taken from the CLI docs, not tested.
