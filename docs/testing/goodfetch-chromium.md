## Goodfetch Chromium

Decided in [ADR 0074](../adr/0074-the-goodfetch-chromium-is-gated-where-it-is-built-against-a-control-build-of-the-same-revision.md). Suites are those of [Suites, commands and time budgets](suites-commands-and-time-budgets.md); Live checks, diagnostic runs and the results store are those of [Flaky tests and Live checks](flaky-tests-and-live-checks.md).

### The Chromium build gate

- **The build's inputs** are the files under `chromium/` that make the build (the Chromium version, ungoogled-chromium's tag, the fingerprint patch series, Goodfetch's own patches, the build scripts and arguments). **Its test inputs** are the WPT and Khronos pins, the expectations, the launch-arguments file and the gate's own workflow file, so a change to how the gate runs never passes on an old verdict. The digest manifest and the protocol JSON are outputs, in neither set. A pull request that changes either set is a Chromium pull request.
- **The Chromium build gate is a required check on every head:** every pull request (to `main` or a release branch, cherry-picks included) and the merge queue, never behind a path filter.
  - It passes fast when every digest in the committed manifest has a verdict attestation naming this head's two input tree hashes.
  - Otherwise, on any pull request, it builds and tests on every platform the build ships; when only test inputs differ, it fetches both builds and their packages by digest and reruns the tests.
  - In the merge queue, a mismatch fails; run the gate in the pull request.
  - Until the owner approves the first measured build and its manifest is committed, it passes and builds nothing.
- **Build where the gate runs, and nowhere else.** The gate builds from the lite and testdata tarballs of the version, checked against Google's hashes. A later job attests and publishes the build, its chromedriver, the Control build and both builds' test packages; only the Goodfetch Chromium and chromedriver are ever shipped. Split a file over GitHub's 2 GiB release-asset limit into parts, each with its digest.
- **Commit the manifest of each platform's digests and the protocol JSON the gate produced.** Each build has a build attestation (build-input tree hash, source commit, workflow), and every passing gate run, a build or a rerun of the tests alone, adds a verdict attestation on each digest (both tree hashes, source commit, workflow). The gate refuses a digest whose build attestation names other build inputs. A fork's Chromium pull request is rebuilt from a branch of the main repository.
- **Never rebuild in a suite or a release.** Suites fetch the build by digest; the release pipeline signs the published bytes.

### The Control build

- **The Control build is the gate's tree with Goodfetch's patch series reverted** (fingerprint-chromium's patches and Goodfetch's own), rebuilt incrementally after the published build, with the same build arguments. ungoogled-chromium's own patches stay.
- **Run every gate test on both builds,** each against its committed expectations, with the same launch arguments.
  _Why:_ a failure on the Goodfetch Chromium alone is the patches'; on both, it is upstream's. Running the Control build is never a retry: the Goodfetch Chromium's result stands.

### What the gate runs

- **Chromium's own suites:** those `gn refs --all --testonly=true --type=executable` finds linking a patched file, within `unit_tests`, `components_unittests`, `content_unittests`, `blink_unittests`, `blink_platform_unittests`, `headless_browsertests`, `content_browsertests` and `browser_tests`; plus `sandbox_linux_unittests` on Linux, always. Unit suites and `headless_browsertests` run whole; `content_browsertests` and `browser_tests` run the test files in the directories the series touches, listed in the gate's report.
- **Pass `--test-launcher-retry-limit=0` on every gtest run.** The launcher retries once by default.
- **Shard with the launcher's own hash sharding,** from packages made from `gn`'s runtime deps, on standard runners; a package too big for one runs on the build runner. `xtask chromium package` makes them: it keeps each file a suite's `<suite>.runtime_deps` list names under its SHA-256, splits a file over 2 GiB into parts each kept under its own digest, writes a manifest of digests, and fails on a listed file that is missing.
- **web-platform-tests:** `./wpt run chrome` at the pinned WPT commit (the commit of wpt.fyi's first Chrome Stable run on the first day the adopted major's version ran there; security updates keep it), with the chromedriver built from the same tree, on the directories `chromium/surfaces.toml` names, in `--chunk-type hash` chunks. wpt.fyi's results never gate.
- **Launch the browser as the engine does:** the `engine` Fingerprint method's arguments with a fixed Identity profile seed, from the file `xtask codegen` writes from the engine's code.
- **WebGL:** Khronos' `webgl1_conformance` and `webgl2_conformance` through Chromium's `run_gpu_integration_test.py` with `--browser=exact` and typ's `--retry-limit=0`, on SwiftShader with `--enable-unsafe-swiftshader`.
- **Size:** each platform's compressed archive. Growth of more than 5 MB over the previous build needs ADR 0057's "Size increase" section in the pull request body, accepted by the owner when merging ([ADR 0133](../adr/0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md)).

### Expected results

- **Keep one current set, in each runner's own format,** under `chromium/expectations/`: filter files per suite and platform for gtests (one `-Suite.Test` line per entry, with its comment), and two wptrunner metadata trees, one per build, written by `wpt update-expectations` and reviewed. Git versions them with the pin.
- **Give every entry one category and its evidence:**
  - `upstream`: fails on the Control build too, with a link;
  - `upstream-flaky`: varies on the Control build;
  - `intended`: a deliberate effect of a named patch, on the Goodfetch Chromium only, naming its surface row.
- **The two WPT trees differ only in `intended` entries.** A check enforces it.
- **An `upstream` entry whose test passes on the Control build fails the gate,** and so does an expected failure that passes. Each Chromium pull request runs the excluded `upstream` gtests once on the Control build and fails on any that pass.
- **`upstream-flaky` entries are upstream exclusions, not the quarantine list:** they exclude Chromium's tests, never Goodfetch's.
  - Generated entries come from LUCI Analysis (a flake rate of 1% or more over 30 days), regenerated in each Chromium pull request that changes the Chromium version and committed with the command that made them, one tracking issue per file. `xtask chromium expectations regenerate <suite>…` writes them, one file per suite and platform with entries, from LUCI Analysis's CI history, which needs no Google account. It counts only bots of configurations the build has, so no sanitizer, ChromeOS or mobile bots, and needs 10 verdicts at least. It opens a file's tracking issue when the file names none, and removes the files left without entries.
  - Every other entry needs a diagnostic run showing both outcomes on the Control build, and has its own issue, an upstream link and a date.
  - Each Chromium pull request that changes the Chromium version rechecks those entries with 20 runs on the Control build of each platform they name, inside the 90-minute test stage. 20 passes on a platform fail the gate until the entry is removed there; an entry not renewed by a recheck showing both outcomes expires after 8 weeks.
  - They block no milestone or release; their issues sit with area 8.
- **Never enter flakiness of the Goodfetch Chromium alone.** It is a patch bug: fix the patch. An entry excludes a flaky test; it never widens a Goodfetch Chromium expectation to "pass or fail".
- **List the entries added and removed** in the Chromium pull request's description.
- **`xtask chromium expectations check` enforces these rules.**
  - Entry format: a filter file gives an entry its category and evidence in the comment line above it, `# <category> <key>=<value>…`. A metadata entry gives them in its `category:` key.
  - Evidence each category needs:
    - `upstream`: `link=https://…`, or the ungoogled `setting=…` that causes it.
    - Hand-written `upstream-flaky`: `issue=#<n>`, `upstream=https://…`, the diagnostic `run=https://…` and `date=YYYY-MM-DD`.
    - Generated `upstream-flaky`: `rate=…%`, in a file under `gtest/generated/` headed `# Generated by: <command>` and `# Tracking issue: #<n>`.
    - `intended`: `patch=` and its `surface=` row.
  - Results it reads: with a run's JUnit files of both builds (`--control`, `--goodfetch`) and the recheck's (`--recheck`), it also fails on an entry that passed where it should not, and on a stale entry.

### Goodfetch's own tests of the build

- **Give every patch file a row in `chromium/surfaces.toml`,** with the surface it changes, its WPT directories and its Goodfetch tests; infrastructure patches (build fixes, background traffic, the variations seed) get rows of their own. The Quick suite fails on a patch without a row or a row without a test.
- **Check Identity consistency on the Proving ground:** each surface reports the Identity profile's value in the main frame, a cross-origin iframe, dedicated, shared and service workers, and the HTTP headers; the same seed gives the same values across launches, different seeds give different ones, and nothing changes within a Session.
- **Check version truth:** user agent, client hints' full version list and `navigator.userAgentData` report the pinned version, and the build's captured HTTP profile matches its own ClientHello and HTTP/2 settings.
- **Check codecs and fonts:** every codec claimed decodes a Proving ground sample; every font the profile lists renders differently from the fallback, and no host font outside the profile is reachable.
- **Check that the browser is quiet:** under a NetLog, a launch, an idle minute and a Proving ground navigation connect only to the Proving ground, and a request to a Google host mapped by `resolve` carries the shipped variations seed's `X-Client-Data` header.
- **These checks read the Proving ground through `goodfetch-test-support`'s helpers** (#404), each shown with a stock Chrome for Testing build on the probe pages:
  - `PageValues::read(&observations)` gathers each surface's values from every `ProbeContext` and from the headers of its report requests, and `identity_report` compares them with the main frame's: a differing value, or a context that reported nothing, makes it inconsistent. A context's `null` says it lacks the surface and is left out; a language list is compared with `Accept-Language` as Chromium expands it (`en-US` is sent as `en-US,en`).
  - `proving_ground.codec_samples(&codecs)` lists the sample each claimed codec decodes (`CodecSample::path()` on `CODECS_HOST`), refusing a codec without one, and `proving_ground.font_samples(&families)` the font page for the listed families (`path` on `FONTS_HOST`) with each family it measures and the result it expects, listed families first, then every other host font.
  - `NetLog::read(path)` lists every socket a `--log-net-log` launch connected (each TCP connection attempt and connected UDP socket, with its address), and refuses a file cut short, as a killed browser leaves it: interrupt the browser so it writes the file whole.
- **Check the CDP protocol in the Quick suite:** every method, event and parameter the driver uses is in the committed protocol JSON.
- **Fail on any crash dump:** each test that launches the build gives it a crash-dump folder in the test's Goodfetch home.
- **Smoke the signed bytes** in the release pipeline, on every OS, before publishing: launch, the CDP handshake, the renderer sandbox check and one stealth fixture.
  _Why:_ signing changes the bytes, and a wrong macOS entitlement breaks a renderer while every signature check passes.

### Where each part runs

| Part | Where |
|---|---|
| `chromium/surfaces.toml` check; CDP protocol check; the no-retry check's Chromium launcher, wptrunner and typ flags | Quick suite |
| The patch series applies to the pinned version (`validate_patches.py`, fetching only the touched files) | Pre-merge suite on Linux, when a build input changes |
| Identity consistency, version truth, codecs and fonts, the quiet browser, the renderer checks, the guard matrix and the stealth fixtures, on the build by digest | Pre-merge suite on Linux when affected; Release suite on every OS |
| Build, Chromium's suites, WPT and WebGL on both builds, expectation checks, size | Chromium build gate: required on every head, building and testing only on a Chromium pull request, every platform |
| Signed-bundle smoke | release pipeline, every OS |
| Chromium patch preview (weekly and at each new Beta major); Chromium build currency (daily) | Live checks |

- **A new major or security update ships only when** the gate is green on every platform, the Chromium pull request passes the Release suite, and the patch passes ADR 0032's gates and the signed-bundle smoke. The online conformance still never blocks.
- **Triage a red gate without a retry:** read the report's result on both builds, let the gate's diagnostic run (20 runs of each failing test on each build, sharded, 60 minutes per job) tell flaky from broken, then fix the patch or add the entry the evidence supports. Never skip a test without an entry.
- **A held major never weakens the gate.** The pinned build keeps shipping. If a patch still cannot pass when the next Stable version ships, the owner decides whether that major ships without it.
- **Budgets:** build ≤ 4 h p50 per platform (6 h cap; stage a build that cannot fit over chained jobs), test stage ≤ 90 min p50 per platform (3 h cap per job). The gate's hosted runner minutes, the macOS runner and a full gate on Beta majors are declined while there is no revenue (ADR 0076); the gate's free design is decided before M3.
- **Results** go to the results store under the tier `chromium-build`, as JUnit XML.
