# Both Products are released and signed together, and each test runs on Collie unless it needs Foxhound

ADR 0084 made Collie and Foxhound two Products from one repository, Foxhound's executable containing Collie's code. ADR 0087 builds each with its own `cargo build -p` under its own Size budget, and ADR 0090 gives both one Product version and publishes both at every release. This decides how they are released, signed and tested.
- A release gates both Products and publishes both, or neither.
- Each Product publishes the same kinds of files on every channel, five image tags and a Helm chart included.
- Both carry Platform signatures under ADR 0039's one entity, which adds no account and no fee.
- A test that runs an executable runs once: on Collie's when Collie carries everything the test needs, on Foxhound's otherwise. Foxhound proves that it contains Collie where its executable is composed, not by running Collie's tests again.
- Collie's executable is built and tested from M1, and its page automation is first tried at M3's owner test.

The [research for this decision](https://github.com/goodfetch/goodfetch/blob/research/multi-product-release-testing/rnd/research/2026-10-15-multi-product-release-testing.md) (2026-10-15) found:
- **Releases gate on every product, and the smaller product's own files get little testing.** uv, Codex and Deno publish nothing when any product's build fails. None runs the smaller product's suite against its own release build: its code is tested once, in a workspace-wide run, with a thin layer on its own files.
  - uv runs `uv_build`'s own package in one CI job, added after a release shipped it with a broken shim ([uv#12058](https://github.com/astral-sh/uv/pull/12058)), and smoke-runs every `uv_build` wheel before release.
  - Codex's CLI runs four tests of its `app-server` command, beside the app server's 173 test files, which run the app server's own executable.
- **A workspace test run tests the union of features.** Cargo unifies the features of the packages one invocation builds, and test builds add the dev-dependencies' features ([resolver](https://doc.rust-lang.org/cargo/reference/resolver.html#features)). The setting that would avoid it is still unstable ([cargo#14774](https://github.com/rust-lang/cargo/issues/14774)), and no project was found that checks two executables resolve their shared crates alike.
- **Publishing is never atomic across registries.** Every precedent relies on re-runs that skip what is published. Codex keeps its GitHub release a draft until every file is on it, and uv's images went missing for a version after a partial publish ([uv#15367](https://github.com/astral-sh/uv/issues/15367)).
- **A changelog split by package hides what a containing product shipped.** Lerna files a commit under each package whose paths it changed, so Crawlee's `crawlee` package, which re-exports the others, read "Version bump only" for a release that shipped fixes in them.
- **Image families multiply per variant:** Apify publishes 21 `actor-*` repositories, Browserless 8 browser images and Playwright four repositories. Users complain of an image that does not match their library's version and of stale floating tags, not of how many there are.
- **PyPI's 10 GiB limit is per project,** an organization pools nothing, and an increase is a free request: 2026's closed requests took 14 days at the median, and DuckDB got 100 GiB in 62 days.
- **Signing scales by file, not by account.** Apple documents a limit of 75 notarizations a day and accepts several executables in one ZIP, and Artifact Signing's Basic tier counts 5,000 signing operations a month across its account.

Decided in [Release, signing and testing of two products](https://github.com/goodfetch/goodfetch/issues/459), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/goodfetch/issues/447), by the agent on the owner's delegation of 2026-10-14 ([grilling record](https://github.com/goodfetch/goodfetch/issues/459#issuecomment-6181380299), [addendum](https://github.com/goodfetch/goodfetch/issues/459#issuecomment-6181479134), [second addendum](https://github.com/goodfetch/goodfetch/issues/459#issuecomment-6189106912)). It builds on [How the products are built, and each product's Size budget](https://github.com/goodfetch/goodfetch/issues/454) (ADR 0087), [Product names, packages, executables and version line](https://github.com/goodfetch/goodfetch/issues/457) (ADR 0090) and [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455) (ADR 0088), and on the [multi-product builds research](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md).

## Decision

### One release for both Products

- **A release gates both Products and publishes both, or neither.** Every gate of ADRs 0030 and 0032 runs on both Products' files, and a failure in either holds both, so ADR 0090's "Foxhound X.Y.Z contains Collie X.Y.Z" always names a published Collie.
  - A fix that the other Product's failure holds back follows ADR 0030's usual path: the failure is fixed, or its change reverted on the release branch, and the release runs again.
  - A publish that fails partway is finished by a re-run, which skips what is published (ADR 0032). The GitHub Release is published, and so made immutable, only once both Products' files are on it, and the tag is pushed only after both Products' npm packages are published.
- **Before any publish, every file of both Products is run from its own bytes,** on every target, on the system it targets, beside ADR 0039's Platform signature gate, which already checks the signed files on macOS and Windows: each archive's executable prints its version and answers an Attached engine's `hello`, and each wheel and npm package installs into a fresh environment and starts its engine. A failure blocks the release.
- **Each target is built in one job that builds both executables,** each with its own `cargo build -p` in the same target folder (ADR 0087), Collie's first.
  - Shared crates whose resolved features match compile once, and the release keeps one build job per target, which the free plan's five concurrent macOS jobs need (ADR 0076). uv builds `uv` and `uv_build` the same way; Codex builds its app server on separate, larger runners, which are billed.
  - The job takes about twice ADR 0057's 20 minutes of fat LTO, within ADR 0061's three-hour job budget. The merge queue's thin-LTO size builds (ADR 0057) follow the same rule.
- **What both Products use is built once, not once per Product:** the Linux `goodfetch-sandbox` helper and the Windows filter broker (ADR 0033), each first-party Plugin and the Proving ground's release asset (ADR 0032), at each release; the Goodfetch Chromium on the pull request that changes its inputs, which the release signs and publishes without rebuilding it (ADR 0074).
  - A first-party Plugin stays one OCI artifact per platform, usable by each Product that carries its Plugin kind; the sink processes are Foxhound's (ADR 0087).

### What each Product publishes

Every channel carries both Products, with the same kinds of files, under ADR 0090's names:

| Channel | Each Product, at each release |
|---|---|
| GitHub Release, one per Product version (ADR 0090) | an archive per target, its debug symbols per target, its CycloneDX SBOM, and their attestations |
| npm | its package and its six platform packages |
| PyPI | its platform wheels |
| crates.io | its crate, whose binary target installs with `cargo install` or `cargo binstall` (below) |
| Homebrew | its formula in `goodfetch/tap` |
| Installers | its `.sh` and `.ps1` |
| OCI | its image's tags and its Helm chart (below) |
| Nightly (ADR 0030) | the same on npm `nightly`, the nightly GitHub pre-release, the nightly Python index (one project page per Product) and each image's `nightly` tag |

> Amended by [ADR 0141](0141-the-kennel-is-a-tray-tool-of-its-own-workspace-that-reaches-the-daemon-as-an-observing-person-surface-and-is-published-apart-from-the-products.md): a release also publishes the Kennel's files beside the Products' own, its signed and notarized `.app` and its install script, under the Kennel's own version, not the Products'.

- **What both Products' packages share is published once:** the internal `goodfetch-*` crates, and the Collie client packages through which both Products' SDKs share Collie's code, which [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458) (ADR 0095) decides.
- **Debug symbols and the SBOM are per executable.** Each Product's archive has its own debug symbols, named after its executable, signed and attested as ADR 0057 says, as Codex publishes symbols per product and target. Each has its own SBOM, since the two dependency graphs differ.
- **Both Products' files stay far under GitHub's limit of 1,000 per release;** Codex's largest release had 318.
- **Each Product's crate, packages and catalog are checked for API breakage** (ADR 0030) against the same Product's previous release of the major.
- **The release notes list each Product's files with their sizes,** and say when Collie has no change of its own and is released to keep the one Product version.
- **One `CHANGELOG.md`** (ADR 0030) groups each release's entries in two: changes that reach both Products, and changes that reach Foxhound only. Foxhound's users read both groups and Collie's the first, so a change is never hidden from a Product it reaches, as Crawlee's per-package files hide it.
  - A commit reaches Foxhound only when every path it changes is on Foxhound's side: `crates/foxhound/`, Foxhound's Console package (ADR 0087) and Foxhound's own SDK files.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no executable embeds a Console build, no Size budget counts one, Foxhound's screens are no workspace package of their own, no release test walks a Console's routes, and no Daemon serves a Console. The Stub gateway stays for the cloud-readiness conformance, without a Console build.
  - The paths decide, not a scope written by hand.

### `cargo install`

- **`cargo install goodfetch-collie --locked --features executable` builds Collie's executable from source,** and Foxhound's likewise. The crate's binary target requires the `executable` feature, which carries the executable's own dependencies, V8 first, so that a program embedding the crate never compiles them, as Protocol, Operation catalog and SDKs across two products decides. Without the feature, Cargo answers that no binary is available and names the feature to pass.
  - The build is the release's: V8 from source with its sandbox on (ADR 0051), which [rusty_v8's README](https://github.com/denoland/rusty_v8/blob/main/README.md) says "often takes 30 minutes" and which needs Python 3, curl and libclang. The docs say so where they give the line, as [Deno's](https://docs.deno.com/runtime/getting_started/installation/) and [uv's](https://docs.astral.sh/uv/getting-started/installation/) installation pages say that `cargo install` builds from source.
  - It is the user's own build, with no Platform signature (ADR 0039).
- **`cargo binstall goodfetch-collie` installs the release's own archive.** Each Product's crate declares `[package.metadata.binstall]` pointing at its archive on the GitHub Release of the same version, so [cargo-binstall](https://github.com/cargo-bins/cargo-binstall/blob/main/SUPPORT.md) installs the signed executable without compiling. cargo-binstall checks only minisign signatures, which Goodfetch does not make; the archive's Sigstore attestation is checked with `gh attestation verify`.

### Images and Helm charts

- **Each Product publishes ADR 0049's five tags under its image** (ADR 0090), and `nightly`: `ghcr.io/goodfetch/collie:<v>`, `-slim`, `-full`, `-node` and `-python`, and Foxhound's likewise.
  - Each tag holds what ADR 0049 lists, with that Product's executable, and the `-node` and `-python` tags that Product's SDK of the same version.
  - Collie's `-slim` holds the engine only, which drives remote browsers through a Browser host or a CDP provider: the HTTP fetch tiers that ADR 0049's `-slim` also serves are Foxhound's (ADR 0088). Collie drives Firefox, WebKit and Camoufox too, so its `-full` is as useful as Foxhound's.
  - Both images build from one recipe whose last layer is the executable, so the two Products' tags share every other layer.
- **Every target ADR 0049 supports is supported for both Products.** CI runs each Product's image on each target CI hosts, with that Product's smoke run (Collie an automation, Foxhound a crawl), and the Cloud Run deploy of each release candidate deploys both images in its one job.
- **The two Helm charts, `ghcr.io/goodfetch/charts/collie` and `ghcr.io/goodfetch/charts/foxhound`, come from one chart source** that differs only by the image it runs. A Quick check renders both with `helm template` and fails on any other difference.
  - ADR 0049's `kind` test runs each chart: install, run, drain.
  - ADR 0066's Helm upgrade runs on Foxhound's chart (below).

### Platform signatures

- **Both Products' executables carry ADR 0039's Platform signatures** on macOS and Windows, under its one legal entity, its Developer ID and its Artifact Signing account. The second Product adds no account, no certificate and no fee; ADR 0039's accounts remain owner steps before the first public release, whose fees the owner approves.
- **macOS:** each executable is signed with the hardened runtime and no `allow-jit` entitlement, since both embed V8 without JIT (ADRs 0051 and 0088). Both executables of a target go to the notary service in one submission, a ZIP of the two, so the nightly's submissions stay about two a day, far under Apple's 75.
- **Windows:** one Artifact Signing call per architecture signs both `.exe` files. Collie's adds about 60 signing operations a month, two each night, which keeps the account near 1,100 of the Basic tier's 5,000 (ADR 0039). A file signed with the publisher's certificate inherits the publisher's SmartScreen reputation instead of starting from zero ([multi-product builds](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §6.1).
- **The Platform signature gate** (ADR 0039) verifies every signed file of both Products before any publish.

### PyPI's storage limit

- **`goodfetch-collie` and `goodfetch-foxhound` are two PyPI projects, each under its own 10 GiB default.**
  - Foxhound's use is ADR 0057's estimate: about 50 releases a year with six wheels of about 40 MB each, about 12 GB a year.
  - Collie's wheels, with V8 (ADR 0088) and under the 50 MB ceiling of ADR 0087, are about 30 MB each. Every release publishes Collie too (ADR 0090), so Collie uses about 9 GB a year.
- **The owner asks [pypi/support](https://github.com/pypi/support) for 100 GiB for each project right after its first upload.** A project exists only once something is published to it, and PyPI asks for a first release under the limit. The request is free and took 14 days at the median in 2026; filing it is an owner step, since the projects are the owner's.
- **The release job reads each project's usage** and warns when either passes 70% of its limit (ADR 0057).

### Collie's milestone

- **Collie's executable is built and tested from M1.** Its crate holds the composition Foxhound's executable starts from and roots Collie's boundary checks (ADR 0087), so the engine tracer builds both executables, `xtask size` and the composition tests cover both from M1, and each later gate that builds, measures or runs an executable covers both from its start.
  - uv, Codex and Deno each added their smaller product a year or more after the larger one; uv's first `uv_build` shipped broken.
  - Goodfetch ships both in V1 (ADR 0084), so both exist from the first tracer.
- **Each owner test of M1, M2 and M3 installs both executables from a local build** and lists the Journeys of both Products due at its milestone. M1's tries Collie's share of M1, the Daemon, the Console and settings (J-001 and J-009), beside Foxhound's crawls.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **M3 is Collie's milestone:** its owner test is the first to try Collie's page automation, which ADR 0058's M3 holds: the guard and the page API, Firefox and WebKit, the Browser host, Session sources and Borrowed browsers, stealth and the Challenge solver, and the Live view with takeover.
- **Collie's AI actions come with M4,** where ADR 0058 puts AI and Plans, and both Products are first published at M4, since nothing is published before it.

### Which executable a test runs

- **A test that runs an executable runs Collie's when Collie carries everything the test needs, and Foxhound's otherwise.** It runs once, not once per Product.
  - A test whose setup or steps need Foxhound anywhere, such as ADR 0066's rolling upgrade, whose load includes a Partitioned crawl, runs on Foxhound's.
  - A test that measures an executable itself runs on each: its size (ADR 0087), its startup and its idle memory.
- **Each package is tested against its own Product's executable:** each executable crate's `tests/it` (its CLI, its MCP server, its exit codes and its `--help` tree), and each Product's SDK packages, Console build, image and chart. Each Product's SDK packages implement every Client conformance case and pass the completeness checks for their own Product's catalog (ADR 0069).
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **One test build per target serves both Products.** CI builds a target's tests once (ADR 0061), in one workspace invocation, so the in-process tests and the executables the tests run share one resolution of features: Collie's executable in a test links the same shared code as Foxhound's.
  - A test build per Product would compile again every crate whose features differ and every crate above it, and Cargo has no stable setting to avoid that.
  - What each Product ships, its own `cargo build -p` (ADR 0087), is checked on its own bytes by the release: the conformance suites and the correctness oracle (ADR 0032) and the run of every file before publish (above).

  > Agent decision, as built (spec #325's review, M8, 2026-10-22): the executables tests run through the test tool manifest (`xtask test-tools`) build in one Cargo invocation of their own, apart from the test build, so both Products' executables share one resolution of features with each other but not with the in-process tests. Built inside the test build, they would take the dev-dependencies' features, test support's Tokio `test-util` among them, and tests would run executables unlike those that ship. The cost is that the crates whose features differ between the two resolutions (`rcgen`, `derive_more`, `encoding_rs`, `serde_core` on 2026-10-22) compile twice per suite. An executable crate's own `tests/it` still runs `CARGO_BIN_EXE_*` from the test build (ADR 0062).

  > Owner decision (#2089, 2026-11-06): every package selection builds the build side with one feature resolution, so a subset build (`-p a -p b`, an `xtask affected` selection, a test build) reuses the C and C++ a `-sys` crate's build script compiled. On 2026-11-04 a five-crate `cargo clippy -p …` rebuilt DuckDB's C++ for 20 minutes in a `target/` that `just lint` had warmed, because its build script's `serde` took other features. `goodfetch-workspace-hack`, which every member depends on, lists for each dependency that build scripts and proc-macros use with more than one feature set the union of those sets; `ci/workspace-hack.sh` writes it with cargo-hakari, and `just lint` checks it is current. A build script then compiles and runs once per profile, whatever the selection.
  >
  > Agent decision, as built (#2089): the code side keeps one resolution per selection. cargo-hakari's union of it takes the dev-dependencies' features and crates (Tokio's `test-util`, proptest, tempfile) and Foxhound's (Arrow), which every member would then build, Collie's executable among them: against this ADR's rule above that tests run executables like those that ship, and ADR 0087's that each Product ships what it uses. The workspace-hack keeps only cargo-hakari's build-dependency tables, so both executables resolve their code as before (`cargo tree -e normal` on each, unchanged but for the empty workspace-hack and a proc-macro's features). What still differs between selections is Rust, which sccache serves. `goodfetch-protocol` does not take it, since the Kennel's own workspace links it by path (ADR 0141) and would take every line; it builds no C or C++. The fuzz crates' lockfiles gain the workspace-hack and its build-side crates. Cargo's `resolver.feature-unification` (cargo#14774), once stable, would give the code side one resolution without the extra crates.
- **Foxhound proves that it contains Collie at each seam where its executable is composed,** not by running Collie's tests again, as Codex's CLI tests its `app-server` command and not the app server:
  1. its engine composition contains Collie's (ADR 0087's test);
  2. every generated contract of Collie's (its catalog entries, setting keys, Error codes, events, CLI commands with their flags, and MCP tools) is in Foxhound's, unchanged, whatever shape [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458) (ADR 0095) gives the catalogs: a Quick check over `xtask codegen`'s output;
  3. every route of Collie's Console build is in Foxhound's and renders: a served test on Foxhound's executable, in the Pre-merge suite;
     > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  4. each Collie Journey that has a `cli` variant runs it on Foxhound's executable too, as the variant `J-NNN@foxhound-cli`: the same test given Foxhound's executable, whose CLI keeps Collie's commands, flags, `--json` output and exit codes (ADR 0090). It shows end to end that Foxhound's composition (its admission gates, Hooks and defaults) leaves Collie's behaviour as it is.

  These four checks are how [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md)'s rule that a capability Collie carries behaves the same in both Products is tested; that capability's own suites run once, on Collie's executable.
- **Journeys.** Each Journey file names its Product in a `product` field: Collie when Collie carries every step, Foxhound otherwise.
  - Its surfaces are that Product's, and its variants keep the form `J-NNN@<surface>`; `J-NNN@foxhound-cli` is the only variant that names a Product.
  - ADR 0063's catalog check accepts `foxhound-cli` only on a Collie Journey that lists `cli`, checks it against `cli`'s first milestone, and fails a test that carries it for a Foxhound Journey.
  - J-001 and J-009 are Collie's; the other ten Journeys of ADR 0063's first catalog are Foxhound's.
- **Conformance suites** run on the executable of the smallest Product that carries their contract: browser conformance and the Plugin kinds Collie carries on Collie's, Sinks and Fetch backends on Foxhound's. Collie's `plugins test` refuses a Sink or Fetch backend Plugin with `not_in_product` (ADR 0087).
  - ADR 0088's Session ownership and proxy limit cases run on Collie's, for a Session a program or an Agent opened and for a Collie Run's proxy limits; the case of a crawl's Session pool runs on Foxhound's.
- **The benchmark** (ADR 0025) runs each family on the smallest Product that carries it, and startup and binding overhead on each Product's executable; each published result names its Product. The patch's correctness oracle (ADR 0032) does the same.
- **Performance budgets** (ADR 0071): each Product's executable crate has its own startup benchmarks, and each Product its own resident-memory ceilings for an idle Attached engine and an idle Daemon; the 1,000-page crawl runs on Foxhound's. The Performance series measures both executables and each Console build.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Skew and upgrades** (ADR 0066):
  - each released minor's Journeys run, with that release's Clients, against the current executable of each Journey's Product;
  - each Product's SDKs run over stdio through its own engine-path variable (ADR 0090), and each Product's Console build is served by its older and current Daemon;
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - pairs across Products are tested where a promise covers them. [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458) (ADR 0095) lets a Foxhound engine accept a Collie Client: its Product check in `hello` is tested for every pair of Products with fabricated `hello`s, as version refusals are (ADR 0066), and no released Collie Client runs again against Foxhound's executable, which serves Collie's operations as Collie's does (the containment checks above);
  - an older Collie opens the files the current engine writes in the formats both Products carry, and an older Foxhound those in Foxhound's. Where [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486) (ADR 0098) lets two installs share a file, an older Collie also opens the one the current Foxhound wrote, with Foxhound's own rows in it (such as the Usage ledger's);
  - rolling upgrades and the Helm upgrade run on Foxhound, as above; Collie's chart is the same chart with Collie's image, and the Daemon group's code is the same.
- **Installed packages** (ADR 0069): each Product's packages install into fresh environments, and one environment per package manager installs both Products' packages and runs both executables, proving that they collide nowhere (ADR 0090).
- **The Feature inventory** (ADR 0072) records the Products that carry each Feature, from ADR 0087's generated table of Foxhound's names. A Feature is tested once, not once per Product, and `xtask features report` gives each Product's view.
- **Suites** (ADR 0061): each suite covers both Products. `xtask affected` reaches only Foxhound's tests from a change confined to Foxhound's side, and both Products' from any other change. The time budgets are unchanged; the second executable's builds and tests count against them, and a breach is fixed as ADR 0061 says (shard, speed up, move a test), never with paid runners (ADR 0076).
- **Live checks** (ADR 0064) follow the same rule: real SaaS Sinks on Foxhound's executable; installed Chrome channels, the online conformance and live-model Evaluations on Collie's wherever Collie carries their subject; the Cloud Run deploy on both images.

## Considered Options

- **Publishing a Product whose gates pass while the other's fail,** as Browserless publishes the images that built: rejected. Collie's line would have gaps, or a Foxhound would contain a Collie that was never published, against ADR 0090; uv, Codex and Deno all hold every product on any product's failure.
- **A release pipeline per Product:** rejected. Two pipelines for one Product version would need to agree on one tag, one GitHub Release and one set of gates, and could still publish one Product alone.
- **Each Product built in its own job per target,** as Codex builds its app server: rejected. It doubles the release's build jobs against the free plan's five concurrent macOS jobs, and Codex's runners are billed (ADR 0076).
- **Every test run on both executables:** rejected. It doubles the Release suite against ADR 0061's 60-minute merge-queue budget, to find what the containment checks find in seconds; in one test build, both executables link the same shared code.
- **Every test run on Foxhound's executable only,** since it contains Collie: rejected. Collie's composition, Console build and embedded data would never run, and Collie's users install them; uv's untested `uv_build` shipped broken.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **A test build per Product,** so that each executable is tested under its own features: rejected. It compiles every crate twice wherever a feature differs, for differences Cargo's additive features should not produce, and the release runs each Product's own bytes before publish. When `resolver.feature-unification` becomes stable (cargo#14774), area 1 may revisit it.
- **A check that both executables resolve their shared crates with the same features:** rejected. Foxhound's fetch tiers turn on features of shared HTTP crates that Collie need not carry, so the check would need a list of reviewed exceptions, the speed bump ADR 0087 rejected for crates; no project was found running one.
- **No Journey variant on Foxhound, the composition checks alone:** rejected. They show that Collie's definitions are present in Foxhound, not that Foxhound's admission gates, Hooks and defaults leave Collie's behaviour unchanged.
- **Foxhound variants on every surface of every Collie Journey:** rejected. It doubles Collie's Journeys, and only the CLI is the same across executables; each SDK surface is a different package (ADR 0090).
- **A Product in every variant ID** (`J-002@foxhound-cli` for every Foxhound Journey): rejected. Each Journey belongs to one Product, named in its file, so only the variant that crosses to the other Product needs it in its ID.
- **Skew over every pair of every Product's Clients and engines:** rejected. It tests pairs no promise makes; pairs across Products are tested where Protocol, Operation catalog and SDKs across two products or Collie and Foxhound installed on one machine make a promise. **Skew on Foxhound only:** rejected. Collie's packages are separate packages with their own compatibility promise (ADR 0066).
- **Running the release's files only on Linux:** rejected. Windows and macOS are where a signed or packaged file fails to start, and the Platform signature gate already checks signed files there; Linux's archives and wheels run on Linux.
- **The executable's dependencies on by default,** so that a bare `cargo install goodfetch-collie` works: rejected by Protocol, Operation catalog and SDKs across two products, since every program embedding the crate would compile V8. **The `v8` crate's prebuilt library for `cargo install` builds:** rejected. It has V8's own sandbox off (ADR 0051), so a `cargo install` build would run a weaker Script sandbox than the release, and it downloads at build time. **An executable crate of its own** (`goodfetch-collie-cli`): rejected by ADR 0090's one name per Product on every channel. **A minisign key for cargo-binstall:** rejected. It is a second signing secret beside Sigstore's for the one channel that would read it.
- **Fewer image tags for Collie:** rejected. Collie drives remote browsers (`-slim`) and Firefox, WebKit and Camoufox (`-full`), and one tag scheme keeps every guide and `init` template the same for both Products. **One image holding both executables:** rejected by ADR 0090's image names, and it would ship Foxhound to every Collie user.
- **One Helm chart with a Product value:** rejected by ADR 0090's chart names; one chart source with a rendering check gives the same single source. **Collie's own rolling and Helm upgrade tests:** rejected. Its chart and its Daemon group's code are Foxhound's, which the upgrade under a Partitioned crawl tests.
- **A changelog per Product,** as Lerna writes one per package: rejected. Every shared change would be listed twice, and Crawlee's containing package showed "Version bump only" for a release that fixed its parts. **One ungrouped changelog,** as uv's and Codex's: rejected, since a Collie user would have to sort out Foxhound's entries. **Grouping by commit scope:** rejected, since scopes are written by hand.
- **Asking PyPI for more space only for Foxhound:** rejected. Collie's wheels, with V8, reach the default within about a year too.
- **Collie's executable unsigned, or signed under another account:** rejected. A stock Windows 11 with Smart App Control enforcing refuses to start an unsigned executable (ADR 0039), and another account would add an account and a fee.
- **Notarizing each executable in its own submission:** rejected. Twice the submissions, toward Apple's 75 a day, where one ZIP holds both.
- **Collie's executable built only from M3 or M4:** rejected. Foxhound's executable starts from Collie's composition and ADR 0087's checks are rooted at Collie's crate, and Collie's capabilities that M1 builds (the Daemon, the Console, settings) would go untested through its own executable until then.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Collie's AI actions moved from M4 into M3,** so that M3's owner test tries the whole of Collie: rejected. It reorders the milestones the owner set (ADR 0058) and grows M3 by area 9's core, and it moves no release, since nothing is published before M4.

## Consequences

- **Amends ADR 0030.** A release gates both Products and publishes both, or neither; its notes list each Product's sizes; the benchmark and the API breakage checks run per Product; the changelog groups entries by the Products they reach. Every file of both Products runs from its own bytes before any publish. Notes sit under its size budget channel, its conformance, benchmark and API breakage gates, and its changelog.
- **Amends ADR 0032.** Each target's job builds both executables; what both use is built once; every registry job publishes both; conformance suites run on the smallest Product that carries their contract. Notes sit under its build, publishing and conformance harness items.
- **Amends ADR 0039.** Both Products' executables are signed and notarized, both of a target in one submission, under the same accounts. Notes sit under its macOS and Windows items and its Artifact Signing service.
- **Amends ADR 0049.** Each Product publishes the five tags, every Supported target is supported for both, and the two charts come from one source. Notes sit under its image table, its target table and its Helm chart.
- **Amends ADR 0057.** Debug symbols are per executable, the nightly index lists each Product, and each PyPI project has its own limit and request. Notes sit under its debug symbols, nightly wheels and project items.
- **Amends ADR 0058.** Both executables are built from M1, owner tests install both, and M3 is Collie's milestone. Notes sit under M1's build and test gates, under M3, and under its gates.
- **Amends ADR 0061.** Each suite covers both Products. A note sits under its Release suite.
- **Amends ADR 0063.** A Journey names its `product`, its surfaces are that Product's, `J-NNN@foxhound-cli` runs each Collie Journey's `cli` variant on Foxhound, J-001 and J-009 are Collie's, and its catalog check accepts `foxhound-cli` only on a Collie Journey that lists `cli`. Notes sit under its file table, its surfaces, its shipped artefacts, its catalog check and its first catalog.
- **Amends ADR 0066.** Skew runs per Product, a Collie Client on a Foxhound engine is tested at `hello`, files shared across Products follow Collie and Foxhound installed on one machine, rolling and Helm upgrades run on Foxhound, and older engines open files per Product. Notes sit under its skew matrix, its old Clients, its rolling upgrades and its older engines.
- **Amends ADR 0068.** Each Product's Console build is tested served by its own executable, with a route walk on Foxhound's. A note sits under its served Console.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Amends ADR 0069.** Each Product's SDK packages run against their own executable, and one environment installs both Products. Notes sit under its real binary and its installed packages.
- **Amends ADR 0071.** Startup benchmarks and idle memory ceilings are per executable, and the crawl's ceiling is Foxhound's. Notes sit under its startup benchmarks and its memory ceilings.
- **Amends ADR 0072.** A Feature is tested once, not once per Product, and the inventory records each Feature's Products. A note sits under its rule on surfaces.
- **Amends ADR 0024.** Its `cargo install` channel installs each Product's executable with the `executable` feature, built from source, and `cargo binstall` installs the release's own archive. A note sits under its list of channels.
- **Amends ADR 0025.** Each family runs on the smallest Product that carries it, and startup and binding overhead on each. A note sits under its families.
- **Earlier test ADRs** (0059, 0062, 0064, 0065, 0067, 0070, 0073, 0074 and 0075) read through ADR 0084's rule and this one: a test of "the binary" runs Collie's executable when Collie carries everything it needs, Foxhound's otherwise.
- **Glossary.** A **Journey** is reached through a Product's public surfaces. No new term.
- **`TESTING.md`** gains "Which Product's executable a test runs" under its levels, and rules for Journeys, upgrades, SDKs and the CLI, performance budgets, the served Console, docs samples and Feature coverage.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **ADR 0037.** Rows CL-01 and CL-02 cite this ADR, and areas 1, 2, 3, 11, 12 and 13 list it.
- **Areas.**
  - Area 1 (Repository, CI and release) builds both executables in each target's job, the release's run of every file, each channel's files per Product, the grouped changelog, the PyPI usage warning per project, and the Quick containment check over generated contracts.
  - Area 2 (Proving ground and conformance) places each conformance suite and benchmark family on its Product, and builds the Journey `product` field, the `foxhound-cli` variant and the catalog check on them.
  - Area 3 (Engine runtime) builds the engine tracer's two executables and each Product's startup benchmarks and memory ceilings.
  - Area 11 (Client surfaces) builds each Product's SDK tests and the installed-packages run with both Products.
  - Area 12 (Console) builds each Product's served tests and the route walk on Foxhound's.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - Area 13 (Self-hosted deployment and operations) builds both Products' images and charts from one source, their rendering check, and each target's run per Product.
- **Owner steps.** The two PyPI requests for 100 GiB, each right after its project's first upload. Nothing else: ADR 0039's entity, accounts and fees are unchanged, and the agent registers, requests and pays for nothing (ADR 0076).
- **Left to other tickets.** What two installs share (Collie and Foxhound installed on one machine) sets the files an older Collie opens from a current Foxhound; which executable and image a Browser host runs is [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/goodfetch/issues/498)'s (ADR 0096); how the docs site presents each Product, and so which Product a sample runs on, is [Docs site and positioning of two Products](https://github.com/goodfetch/goodfetch/issues/499)'s (ADR 0097). [Apply the product-line decisions to the affected specs and tickets](https://github.com/goodfetch/goodfetch/issues/460) carries this decision into the specs and tickets.
