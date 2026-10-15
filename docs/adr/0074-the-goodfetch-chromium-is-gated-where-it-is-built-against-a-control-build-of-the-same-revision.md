# The Goodfetch Chromium is gated where it is built, against a Control build of the same revision

Each Goodfetch Chromium is built on the pull request that changes its inputs, and tested there by the **Chromium build gate**, on every platform, before its bytes can reach a suite or a user. The gate runs a selected subset of Chromium's own test suites, the web-platform-tests of every surface the patches change, and Khronos' WebGL conformance suite. It runs each one on the Goodfetch Chromium and on a **Control build**: the same tree at the same revision, built with the same arguments but without Goodfetch's patches. A failure on the Goodfetch Chromium alone is the patches'; a failure on both is upstream's. The gate is a required check on every head, and passes at once where the committed digests' attestations already name that head's inputs. Expected results are committed per runner, and each entry has a category and evidence. Goodfetch's own tests then check every patched surface through the Proving ground, against the build fetched by digest, in the Pre-merge and Release suites. The release pipeline signs the tested bytes and launches them once more on every OS.

ADR 0035 builds the Goodfetch Chromium in a release job, and a patch that no longer applies blocks the release. The browser was tested only by the stealth fixtures, the guard matrix and the conformance suites, so the C++ the patches change was reached only from outside, and the merge queue could not test a new major's build before merge. The facts, checked on 2026-10-13:
- **Two-week majors.** Chrome has shipped a Stable major every two weeks since Chrome 153 on 2026-09-08 ([Chrome blog](https://developer.chrome.com/blog/chrome-two-week-start)), so about 26 builds a year, plus security updates.
- **Builds closest to Goodfetch's test nothing.** ungoogled-chromium only validates that its patches apply ([ci.yml](https://github.com/ungoogled-software/ungoogled-chromium/blob/master/.github/workflows/ci.yml)); fingerprint-chromium and Thorium run no tests. Their users report startup crashes after a major ([ungoogled-chromium #3550](https://github.com/ungoogled-software/ungoogled-chromium/issues/3550), [windows #562](https://github.com/ungoogled-software/ungoogled-chromium-windows/issues/562)) and a canvas protection that silently stopped working ([#3375](https://github.com/ungoogled-software/ungoogled-chromium/issues/3375)). fingerprint-chromium is five majors behind Stable and publishes its patches "when the next version is published" ([README](https://github.com/adryfish/fingerprint-chromium)), so Goodfetch carries the series forward itself.
- **Projects that test do it with filter and expectation files.** Brave runs upstream unit and browser tests with commented filter files per suite, OS and architecture, and generates the list of upstream-flaky tests from Chromium's LUCI Analysis ([test/filters](https://github.com/brave/brave-core/tree/master/test/filters)). Puppeteer keeps `TestExpectations.json` and edits it on each Chrome roll ([#15517](https://github.com/puppeteer/puppeteer/pull/15517)). CEF runs `ceftests` with a documented exclusion list ([chromium_update.md](https://github.com/chromiumembedded/cef/blob/master/docs/chromium_update.md)).
- **Chromium tells a CL's failure from an existing one by rebuilding without the CL** and rerunning only the failing tests "without patch" ([parsing_test_results.md](https://chromium.googlesource.com/chromium/src/+/main/docs/parsing_test_results.md)). Chrome for Testing cannot play that part for Goodfetch: its build arguments and domain handling differ from ungoogled-chromium's.
- **The source has no tests by default.** ungoogled-chromium builds from Google's lite tarball, which has `web_tests`, `chrome/test/data`, `third_party/webgl` and other test trees removed; Google publishes a separate testdata tarball ([export_tarball.py](https://chromium.googlesource.com/chromium/tools/build/+/main/recipes/recipe_modules/chromium/resources/export_tarball.py), [publish_tarball.py](https://chromium.googlesource.com/chromium/tools/build/+/main/recipes/recipes/publish_tarball.py)).
- **Every runner retries by default somewhere.** Chromium's test launcher retries a failure once (`retry_limit = 1U`, [test_launcher.h](https://chromium.googlesource.com/chromium/src/+/main/base/test/launcher/test_launcher.h)) and `run_web_tests.py` three times; wptrunner's `--retry-unexpected` defaults to 0 ([arguments](https://web-platform-tests.org/running-tests/command-line-arguments.html)).
- **Hosted macOS runners cannot hold a build:** every macOS runner lists 14 GB of disk, against Chromium's 100 GB ([larger runners](https://docs.github.com/en/actions/reference/runners/larger-runners)).

Decided in [Testing the Goodfetch Chromium build on each Stable major](https://github.com/goodfetch/goodfetch/issues/310), with the agent answering on the owner's delegation and researching the facts itself; its sources are in the ticket's grilling record. The rules every test follows are in `TESTING.md`.

**Suites and tiers.** The suites are ADR 0061's Quick, Pre-merge and Release suites, each containing the one before; the Release suite runs in the merge queue (on manual dispatch while the repository is private), nightly and in every release. Checks of unpinned inputs are Live checks (ADR 0064). The Chromium build gate is in no suite, like ADR 0067's test V8 build: its tests need the build's tree.

## Decision

### Where the build and the gate run

- **The build's inputs** are the Chromium version, ungoogled-chromium's tag, the fingerprint patch series, Goodfetch's own patches, the build scripts and the build arguments, all under `chromium/`. **Its test inputs** are the WPT and Khronos pins, the expectations, the launch-arguments file `xtask codegen` writes from the engine's code, and the gate's own workflow file, so a change to how the gate runs never passes on an old verdict. The gate hashes each set as a Git tree. The digest manifest and the protocol JSON are outputs, in neither set, so committing them changes neither hash.
- **A Chromium pull request** is a pull request that changes the build's inputs or its test inputs. A scheduled job opens one when Google's version history shows a new Stable version, as Puppeteer's bot does. The scheduled job runs none of the product.
- **The Chromium build gate is a required check on every head**: every pull request (to `main` or to a release branch, cherry-picks included) and the merge queue. It never uses a path filter, since a skipped required check stays pending forever (ADR 0032). This overrides "the only required check" of ADRs 0032 and 0061, which carry a one-line note.
  - **It passes fast** when every digest in the committed manifest has a verdict attestation that names this head's two tree hashes. Most heads take this path, with no build.
  - **Otherwise, on a pull request, it builds** the Goodfetch Chromium, runs the tests below on it and on its Control build, and checks the expectations. When only the test inputs differ, it fetches both builds and their test packages by digest and reruns the tests. A pull request to a release branch follows the same rule, so a cherry-picked Chromium change whose launch arguments differ there reruns its tests.
  - **In the merge queue, a mismatch fails.** A build cannot fit the queue's 120-minute timeout, so the pull request runs the gate instead.
  - **Before the owner approves the first measured build** (ADR 0035) and its manifest is committed, the gate passes and builds nothing.
  - Its build jobs hold no secret and a token with no permissions.
- **A later job attests and publishes** the bundle and its chromedriver as release assets, and the Control build and both builds' test packages as CI-only assets, which no channel ships. Release assets do not expire, unlike workflow artifacts. A file over GitHub's 2 GiB limit per release asset is split into parts below it, each with its digest.
- **Two attestations bind the bytes to their inputs.**
  - The **build attestation**, made once per build, records the build-input tree hash, the source commit and the workflow.
  - A **verdict attestation**, made on each digest by every passing gate run (a build or a rerun of the tests alone), records both tree hashes, the source commit and the workflow.

  The author commits a manifest of each platform's digests, and the gate checks every digest's build attestation against this head's build inputs, so a digest cannot point at bytes built from other inputs. The fast path reads only verdict attestations, so a change of test inputs after the build has its own verdict for the merge queue to find. A fork's pull request is rebuilt from a branch of the main repository, since forks get no `id-token`.
- **The Release suite fetches the build by digest.** The release pipeline signs those bytes (ADR 0039) and never rebuilds them.

### The Control build

- **It is the gate's tree with Goodfetch's patch series reverted:** fingerprint-chromium's patches and Goodfetch's own. ungoogled-chromium's patches, domain substitution and pruning stay, so they are upstream for Goodfetch.
- **It is built after the published build is finished,** incrementally in the same output directory, so only the patched translation units recompile and the binaries relink. It is published only as a CI-only asset (above).
- **Every gate test runs on both builds,** each against its committed expectations, with the same launch arguments. Running the Control build is not a retry: it is a different build, and the Goodfetch Chromium's result stands.

### Chromium's own suites

- **The tree:** the lite tarball plus the testdata tarball of the same version, both checked against Google's published hashes.
- **The suites:** those that `gn refs --all --testonly=true --type=executable` finds linking a file the series changes, within an allowlist: `unit_tests`, `components_unittests`, `content_unittests`, `blink_unittests`, `blink_platform_unittests`, `headless_browsertests`, `content_browsertests`, `browser_tests`, and `sandbox_linux_unittests` on Linux, which always runs.
- **What runs of each:** the unit-test suites and `headless_browsertests` whole; `content_browsertests` and `browser_tests` only the test files in the directories the series touches. The gate generates that list from the series and prints it in its report.
- **How:** with `--test-launcher-retry-limit=0`, sharded by the launcher's own hash sharding, from packages made from `gn`'s runtime deps (the inputs of Chromium's own isolates) and run on standard runners. Tests whose package does not fit a standard runner run on the build runner.

### web-platform-tests

- **The runner** is upstream wptrunner, `./wpt run chrome`, at a pinned WPT commit: the commit of wpt.fyi's first Chrome Stable run, on Linux, on the first day the adopted major's version ran there. A security update within the major keeps the pin; the next major's Chromium pull request moves it. If wpt.fyi has no such run, the pin stays and the pull request says so. wpt.fyi's own results never gate.
- **The browser runs as the engine launches it:** with the arguments of the `engine` Fingerprint method and a fixed Identity profile seed, read from a file `xtask codegen` writes from the engine's code. chromedriver is built from the same tree.
- **The directories** are those `chromium/surfaces.toml` names for each patched surface. Today: `html/canvas`, `webaudio`, `webgl`, `webcodecs`, `css/css-fonts`, `css/css-font-loading`, `font-access`, `html/webappapis/system-state-and-capabilities/the-navigator-object`, `screen-orientation`, `screen-details`, `window-management`, `css/cssom-view`, `css/mediaqueries`, `hr-time`, `client-hints`, `ua-client-hints`, `webrtc`, `webrtc-stats`, `mediacapture-streams`, `media-capabilities`, `permissions`, `device-memory`, `battery-status`, `netinfo`, `speech-api`, `keyboard-map`, `gamepad`, `geolocation`, `compute-pressure`, `audio-output`, and test262's `intl402`. Without `intl402`, these held 6,203 tests and 37,794 subtests in Chrome 155's Stable run on wpt.fyi.
- **Chunks** are deterministic: `--total-chunks` with `--chunk-type hash`.

### WebGL

- The gate runs Khronos' `webgl1_conformance` and `webgl2_conformance` through Chromium's `run_gpu_integration_test.py` with `--browser=exact`, on SwiftShader with `--enable-unsafe-swiftshader`, on both builds, with typ's `--retry-limit=0` and no `RetryOnFailure` expectation.
- If the testdata tarball lacks `third_party/webgl`, the suite comes from the Khronos repository at the commit Chromium's DEPS pins. If the harness cannot run outside a full checkout, the question returns to the owner; it is never skipped.

### Expected results

- **One current set, in each runner's own format,** under `chromium/expectations/`. Git versions it with the pin, so a release branch carries its major's set.
  - Chromium suites: filter files per suite and platform, read through `--test-launcher-filter-file`, one `-Suite.Test` line per entry with its comment.
  - WPT and WebGL: wptrunner metadata, one tree for the Control build and one for the Goodfetch Chromium, written by `wpt update-expectations` and reviewed.
- **Every entry has one category, with its evidence:**
  - `upstream`: the test fails on the Control build too, with a link (a Chromium or ungoogled-chromium issue, or the ungoogled setting that causes it);
  - `upstream-flaky`: the test varies on the Control build;
  - `intended`: a deliberate effect of a named patch, on the Goodfetch Chromium only, naming its surface row.
- **The gate checks the entries:**
  - The two WPT trees differ only in `intended` entries.
  - An `upstream` entry whose test passes on the Control build in the same run fails the gate.
  - An expected failure that passes is an unexpected result. For excluded gtests, the Chromium pull request runs its `upstream` exclusions once on the Control build and fails on any that pass.
- **`upstream-flaky` entries are upstream exclusions,** not ADR 0064's quarantine: they exclude Chromium's tests, never Goodfetch's.
  - **Generated entries:** each Chromium pull request that changes the Chromium version regenerates them from LUCI Analysis with Brave's threshold, a flake rate of 1% or more over 30 days, and commits them with the command that made them. Each generated file has one tracking issue. An entry lasts until the regeneration drops it.
  - **Other entries** need the gate's diagnostic run showing both outcomes on the Control build. Each has its own issue, a link to the upstream report, and a date.
  - **Recheck:** each Chromium pull request that changes the Chromium version runs every other entry 20 times on the Control build of each platform the entry names, inside the test stage's 90-minute budget. 20 passes on a platform make the entry stale there, which fails the gate until it is removed for that platform. An entry not renewed by a recheck that shows both outcomes expires after 8 weeks, and the gate then fails until it is renewed or removed.
  - **They block no milestone or release,** unlike quarantined tests. Their issues sit with area 8, outside the milestone.
- **Flakiness on the Goodfetch Chromium alone is a patch bug.** It is fixed, never entered. A flaky entry excludes the test; it never widens a Goodfetch Chromium expectation to "pass or fail".
- **The Chromium pull request lists the entries it adds and removes.**

### Goodfetch's own tests of the build

- **`chromium/surfaces.toml`** lists every surface the series changes, with its patches, its WPT directories and its Goodfetch tests. Infrastructure patches (build fixes, background traffic, the variations seed) get rows of their own. A Quick-suite check fails when a patch file has no row or a row has no test, as ADR 0067's confinement check does for policy rules.
- **Identity consistency:** each surface reports the Identity profile's value in the main frame, a cross-origin iframe, dedicated, shared and service workers, and the HTTP headers. The same seed gives the same values across launches, different seeds give different ones, and nothing changes within a Session.
- **Version truth:** the user agent, client hints' full version list and `navigator.userAgentData` report the pinned version. The HTTP profile captured at build (ADRs 0020 and 0035) matches the browser's own ClientHello and HTTP/2 settings.
- **Codecs and fonts:** every codec that `canPlayType` and Media Capabilities claim decodes a Proving ground sample. Every font the profile lists renders differently from the fallback, and no host font outside the profile is reachable.
- **A quiet browser:** under a NetLog, a launch, an idle minute and a Proving ground navigation connect only to the Proving ground. A request to a Google host mapped by `resolve` (ADR 0059) carries the `X-Client-Data` header of the shipped variations seed (ADR 0043).
- **CDP:** the gate converts the tree's protocol definitions to JSON, which the author commits beside the digests. A Quick-suite check fails when a method, event or parameter the driver uses is missing from it. The guard matrix and the driver's tests run on the build, as before.
- **Sandbox:** ADR 0067's renderer checks, and `sandbox_linux_unittests` in the gate.
- **No crashes:** each test that launches the build gives it a crash-dump folder in the test's Goodfetch home and fails on any dump.

### Placement

| What | Where |
|---|---|
| `chromium/surfaces.toml` check; CDP protocol check; the no-retry check's Chromium and wptrunner flags | Quick suite |
| The patch series applies to the pinned version (ungoogled-chromium's `validate_patches.py`, fetching only the touched files) | Pre-merge suite on Linux, when a build input changes |
| Identity consistency, version truth, codecs and fonts, the quiet browser and the renderer checks, beside the guard matrix and the stealth fixtures, on the build fetched by digest | Pre-merge suite on Linux when affected; Release suite on every OS |
| Build; Chromium's suites, WPT and WebGL on both builds; expectation checks; size | Chromium build gate, a required check on every head: it builds and tests only on a Chromium pull request, every platform, in no suite |
| Signed-bundle smoke: launch, CDP handshake, renderer sandbox and one stealth fixture, on the signed bytes | release pipeline, every OS, before publishing |
| Chromium patch preview; Chromium build currency | Live checks |

- **The gate's tests are tests** under ADR 0064: they pin every input (tarballs by hash, the WPT and Khronos commits, the expectations) and never retry. They are in no suite, an exception to ADR 0061's rule that only checks non-deterministic by nature stay outside the suites, and to ADR 0072's "every test belongs to a suite". They declare no Feature; Goodfetch's own tests of the build do. Their results go to the results store under the tier `chromium-build`, as JUnit XML.
- **The signed-bundle smoke** exists because signing changes the bytes, and a wrong macOS entitlement breaks a renderer while every signature check passes.

### Live checks

| Live check | Tier | What it blocks | Owner |
|---|---|---|---|
| Chromium patch preview: `validate_patches.py` against Chrome Beta's version | weekly, and when Beta reaches a new major | nothing directly | area 8 |
| Chromium build currency: Stable has carried a version for 7 days that the pinned build lacks, from Google's version history API | daily | nothing directly | area 8 |

Both run on standard runners and run none of the product. ADR 0064's rules apply: a fail opens a `live-check` issue, and its cause must be stated within two business days or the next minor waits.

### What blocks a new major

1. The Chromium build gate is green on every platform.
2. The Chromium pull request passes the Release suite in the merge queue, with every Goodfetch test above on the new digests.
3. The patch that carries it passes ADR 0032's gates and the signed-bundle smoke on every OS.

The online conformance against real vendors still runs on that patch and never blocks (ADR 0043).

- **Triage of a red gate.** Its report shows each unexpected result on both builds. The gate's diagnostic run tells flaky from broken: 20 more runs of each failing test on each build, sharded, each job timing out after 60 minutes rather than ADR 0064's 10, since browser tests are long. It never changes the verdict. The pull request then fixes the patch, or adds the entry its evidence supports.
- **A held major.** The pinned build keeps shipping, and the engine's version check (ADR 0035) warns users as it falls behind. If a patch still cannot pass when the next Stable version ships, the owner decides whether that major ships without the patch, with a warning naming the surface. The gate is never weakened.

### Platforms, budgets and cost

> Amended by [ADR 0076](0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md): the hosted build runners and the rented macOS runner are declined while there is no revenue; the gate's free design is decided before M3.

- **Every platform the build ships** runs the whole gate on its own build: the series has platform code (fonts, GPU strings), and users get these bytes.
- **macOS has no hosted runner that fits a build.** The macOS gate runs where ADR 0035's macOS build runs, a third-party or self-hosted macOS runner the owner chooses.
- **Budgets** follow ADR 0061's method (p50 of the last 20 runs, a breach opens an issue):

  | Job | Budget (p50) | Hard cap |
  |---|---|---|
  | Build, per platform | 4 h | 6 h (GitHub's limit); a build that cannot fit is staged over chained jobs, ungoogled-chromium's method |
  | Test stage, per platform | 90 min | 3 h per job |

- **The gate's runner minutes are a paid test resource.** At published prices (Linux 64-core x64 $0.162 a minute, Windows 64-core x64 $0.322, Linux 64-core arm64 $0.098; [pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)), and Windows 64-core arm64 $0.194, a four-hour gate costs about $39 on Linux x64, $24 on Linux arm64, $77 on Windows x64 and $47 on Windows arm64: about $190 per build for the four hosted platforms, or about $7,500 a year at 40 builds. The two macOS builds come on top, on a runner whose price is known only once the owner chooses it. This is an estimate. ADR 0035's first measured build now measures the gate too, and the owner approves the cadence from that figure.
- **A full gate on the first Beta of each major** would find conflicts about three weeks early. It starts only on the owner's approval.

### Size and performance

- **Size:** the gate measures each platform's compressed archive. There is no ceiling. Growth of more than 5 MB over the previous build passes only with ADR 0057's "Size increase" section in the pull request body, accepted by the fresh review on the exact head. Each release's notes list the sizes.
- **Performance:** no new gate. The Performance series and the benchmark (ADRs 0071 and 0025) run on the pinned build, and each build change is recorded as an event in the series, so a shift is attributed to it.

## Considered Options

- **No upstream suites, as ungoogled-chromium, fingerprint-chromium, Thorium and Electron do:** rejected. The patches' C++ would be reached only from outside, and those projects' users find the crashes.
- **Every upstream suite, whole:** rejected. `browser_tests` alone runs on 70 shards on Chromium's own builder, mostly over code no patch touches.
- **Chromium's `run_web_tests.py` and `run_wpt_tests.py`:** rejected. They need `web_tests` and their expectations encode Google's build. Upstream wptrunner with Goodfetch's own metadata tests the binary as shipped.
- **Full WPT:** rejected. 124,682 tests and 2.26 million subtests per platform per build, almost all on surfaces no patch changes.
- **Chrome for Testing as the control:** rejected. Its build arguments and domain handling differ from ungoogled-chromium's, so a difference could come from either. It remains the default browser outside `stealth`, tested as ADR 0024 says.
- **Hand-maintained expectations without a control:** rejected. An `upstream` entry could not be verified, and a patch regression could be waved through as upstream.
- **Expectations generated at run time from the Control build:** rejected. Nobody would review a change in upstream behaviour.
- **Retrying "with patch", as Chromium's CQ does:** rejected by ADR 0064.
- **Keeping the build in the release job:** rejected. The merge queue would test the old build, and a new major would meet its first test after merge.
- **Live detection sites as the fingerprint test, as CloakBrowser does:** rejected as a test. Their verdicts move without a commit; real vendors stay ADR 0035's online rung, a Live check.
- **Puppeteer's or Playwright's test suites against the build:** rejected. They test their own drivers; Goodfetch's driver and guard are tested by their own matrix.

## Consequences

- **Amends ADRs 0032 and 0061:** "all green" is no longer the only required check; the Chromium build gate is one too, on every head. ADR 0067's test V8 build already was. Both carry a one-line note.
- **Amends ADRs 0061 and 0072:** the gate's tests are pinned tests in no suite, an exception to ADR 0061's list of what stays outside the suites and to ADR 0072's "every test belongs to a suite"; they declare no Feature. Both carry a one-line note.
- **Amends ADR 0035.** "The build runs as its own release job on paid large runners" becomes: the build runs on the Chromium pull request, as the Chromium build gate, and the release pipeline signs and publishes the bytes whose digests the pull request committed. A patch that no longer applies now blocks that pull request. ADR 0035 carries a one-line note.
- **Adds to ADR 0030:** a Chrome major or security update ships only after the Chromium build gate and the signed-bundle smoke, beside the conformance suite and the guard.
- **Adds to ADR 0032:** the gate's attested-input binding and its fast path on heads whose inputs are unchanged; the signed-bundle smoke joins the patch pipeline's gates and every release's; the scheduled job that opens Chromium pull requests runs none of the product.
- **Adds to ADR 0043:** the gate fetches the lite and testdata tarballs by hash, the WPT and Khronos commits and the LUCI Analysis lists before the deny; its tests run under the deny.
- **Adds to ADR 0057:** the Goodfetch Chromium's compressed archive per platform is measured, with no ceiling and an increase threshold of 5 MB per build, passed by its "Size increase" section.
- **Adds to ADR 0064:** `upstream-flaky` entries, upstream exclusions apart from its quarantine list, with an issue, a recheck at each Chromium pull request that changes the Chromium version and an 8-week expiry, which block no milestone or release; two Live checks, the Chromium patch preview and the Chromium build currency, owned by area 8; the no-retry check refuses `--test-launcher-retry-limit` other than 0 (and a gtest run without it), `--num-retries`, wptrunner's `--retry-unexpected` above 0, `--rerun` above 1 and `--repeat-until-unexpected`, typ's `--retry-limit` and `--isolated-script-test-launcher-retry-limit` other than 0 and `--retry-only-retry-on-failure-tests`, and a `RetryOnFailure` expectation; the results store gains the tier `chromium-build`.
- **Amends ADR 0064:** the gate's diagnostic run has a 60-minute job timeout, not 10 minutes. ADR 0064 carries a one-line note.
- **Adds to ADR 0067:** `sandbox_linux_unittests` in the gate, and the renderer check on the signed bytes.
- **Adds to ADR 0071:** each build change is an event in the Performance series.
- **ADR 0037's rows** for areas 1, 2 and 8 list this ADR.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)): the signing jobs receive the bundles published from the Chromium pull request, not from a release build; user stories 24 and 85's "the guard when it carries a new Chrome major" gains the gate and the signed-bundle smoke; the release notes list the build's sizes.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/goodfetch/goodfetch/issues/152)): the Proving ground gains the consistency probe pages (frames and every worker kind), codec samples, font pages and a Google-host stand-in that records `X-Client-Data`.
- **Area 8 (Stealth and Challenge solving)** builds the gate, `chromium/surfaces.toml`, the expectations and the Chromium pull request's job.
- **The "Test infrastructure" spec** builds: the gate workflow and its runtime-deps packaging, the expectation checks, the `chromium/surfaces.toml` and protocol checks, the codegen of launch arguments, the crash-dump check, and the helpers of the consistency, codec, font and quiet-browser tests.
- **Owner approvals:** the gate's runner minutes, from the first measured build; the macOS build and gate runner; the full gate on Beta majors.

  > Amended by [ADR 0076](0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md): these paid resources are declined while there is no revenue, not pending; the gate's free design is decided before M3.
- **Unverified, checked by the first implementation tickets:**
  - whether `browser_tests` and the other suites build and run on an ungoogled tree with the testdata tarball added;
  - whether the testdata tarball holds `third_party/webgl`, and whether the WebGL harness runs outside a full checkout;
  - whether LUCI Analysis can be queried without a Google account;
  - the sizes of the runtime-deps packages and the gate's real duration;
  - whether wptrunner's xunit log serves as JUnit, or needs a converter in test support.

  A negative answer returns to the owner as a decision, never as a skipped test.
