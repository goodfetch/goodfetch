# One repository for every surface, build once and publish per registry, CI in tiers

Every surface released together under ADR 0030 lives in one public repository, together with the Proving ground and the benchmark harness, because multi-language projects that release in lockstep keep one repository (Ruff, uv, Biome, Deno and Turborepo around a Rust core; Dagger, which releases every SDK at its engine's version; and Stagehand since v4, which keeps its TypeScript, Python and Go SDKs over one JSON-RPC protocol package), while those split per language lag: Playwright's language ports trail the core by one to three weeks, spider-rs's bindings sit behind its core, and Crawlee JS and Python keep separate version lines. Pydantic merged pydantic-core into its main repository in November 2025. The tree:

```
crates/            Cargo workspace: the public `automan` crate, internal `automan-*` crates, the `automan` binary
crates/sinks/      one crate per sink process (ADR 0026)
console/           the Console app, built and embedded in the binary (ADR 0024)
sdks/typescript/   npm `automan` and the generated per-platform packages
sdks/python/       the `py3-none` platform wheels
adapters/          Vercel AI SDK, LangChain/LangGraph (TypeScript and Python), Mastra (ADR 0011)
integrations/      n8n, Airbyte source, LangChain and LlamaIndex loaders, Zapier, Make, Pipedream, Activepieces, Power Automate (ADR 0027)
skills/            agent skills
conformance/       the cross-surface conformance suites, run against the Proving ground
proving-ground/    seed generator, page server, oracles, and deploy/ for the online copy
bench/             the benchmark harness, competitor adapters, and results/
docs/              the product documentation's content, plus adr/ and agents/
website/           the Docusaurus configuration and theme
rnd/               frozen R&D record
```

Releases are **built once and published per registry**: no tool publishes crates.io, npm platform packages, PyPI, Homebrew and an OCI image from one version, so, like uv, Turborepo and Oxc, Automan writes its own publishing jobs. CI runs in tiers, so that the full platform and runtime matrix of ADR 0024 runs before every merge without holding up each pull request. Decided in [Repository layout and CI](https://github.com/Jbmanllr/automan/issues/107), on the evidence of the [repository layout, CI and release research](https://github.com/Jbmanllr/automan/blob/research/repo-layout-ci/rnd/research/2026-10-11-repo-layout-ci.md).

## Considered Options

- **A repository per language or per surface** (Playwright, Crawlee, spider-rs, Temporal): rejected. Every one of them shows version lag and parity tickets, and a lockstep release (ADR 0030) would then need pinning files or submodules to hold the versions together.
- **The Proving ground in its own repository**, as web-platform-tests is: rejected. A feature, its conformance test and the page that test needs then change in two coordinated pull requests. Chromium, which imports web-platform-tests, lives with sync conflicts, rate-limit failures and reverts. Playwright, Scrapy, Crawlee and Firecrawl keep their test sites in the product repository.
- **The Proving ground in TypeScript**, a direct move of `rnd/lab/fixtures`: rejected. It must serve millions of seeded pages without becoming the bottleneck, and the online copy deploys more simply as one static binary. The lab's fixtures are about 1,600 lines, so the rewrite is small.
- **The benchmark harness rewritten in Rust**: rejected. It orchestrates competitors written in JavaScript, Python and Rust, does no hot-path work, and its method is the one ADR 0025 adopted.
- **The docs in their own repository** (Deno, Biome, Playwright's website): rejected. Every example runs in CI (ADR 0030), and examples break when docs live apart; Polars moved its docs into the main repository for that reason.
- **dist** (formerly cargo-dist) **for the whole release**: rejected. Its last release was v0.33.0 on 2026-09-11, most recent commits are from Dependabot, and it has no PyPI, crates.io or npm trusted-publishing jobs; uv now maintains by hand the workflow that dist first generated.
- **release-please**: rejected. It has open bugs with linked Cargo and npm versions and with prereleases, which the rc and nightly channels of ADR 0030 need.
- **Changesets** (Biome): rejected. It versions npm packages only, not Cargo or Python, and its own docs warn that prerelease mistakes are very hard to fix.
- **Postinstall downloads for npm** (dist's and GoReleaser's npm wrappers): rejected. They fail under `--ignore-scripts` and offline; Biome, Turborepo, Deno, Rolldown, Oxc and Rspack all publish per-platform packages, as ADR 0024 already requires.
- **A graph build system** (Nx, moon, Bazel): rejected. It adds a second build system; the native workspaces and the change detection below cover what CI needs.
- **The `rnd/` folder moved to an archive repository**, as Rust and Python keep their proposals: rejected. ADRs, tickets and pull requests link into `rnd/` about 180 times, and single-product repositories such as CockroachDB keep their frozen records in place.

## Consequences

- **Workspaces.** One Cargo workspace, one pnpm workspace (Console, TypeScript SDK, adapters, TypeScript Integrations, harness) and one uv workspace (Python SDK, adapters and Python Integrations), tied together by `just` as the single task runner.
- **Proving ground.**
  - **Language.** Its seed generator, page server and oracles are written in Rust.
  - **Independence.** It never depends on engine crates, so the oracle stays independent of the tool it judges (ADR 0025). CI enforces the rule.
  - **Moving the lab.** The lab's fixtures and oracles are ported into it.
  - **Release asset.** Each release attaches the Proving ground as a standalone binary and seed, so competitors can rerun the benchmark without the whole repository.
  - **Online copy.** Its deployment code is public in `proving-ground/deploy/`. The challenge vendors' keys sit in a protected GitHub environment, and every deployment needs the owner's approval and starts from a release tag.
- **Benchmark harness.** The lab's campaign runner, statistics, verdicts, claims and ledger move to `bench/` in TypeScript, cleaned up rather than rewritten. Each competitor adapter stays in that competitor's own language.
- **Benchmark results.** `bench/results/<version>/` holds the small files: verdicts, summaries, configurations and claims, dated as ClickBench does. Raw data and redacted traces go into release assets, linked by digest; Deno, which keeps its benchmark history in git, carries a side repository of about 3 GB.
- **`rnd/` is frozen in place** when V1 work starts:
  - `rnd/README.md` gets a banner saying it is archived and not maintained.
  - Nothing is edited except broken links, and lab code stays as history after its fixtures and harness move.
  - `pnpm lab ci` runs only when files under `rnd/` change, and is never a required check.
  - `research/*` branches stay unmerged.
- **Private components.** The public repository holds each Private component's interface, a no-op implementation that keeps the public engine whole, and that interface's conformance suite (ADR 0018). The private repository builds against public `main` every night and against every release candidate. A release candidate is not promoted to stable while that run is red, so the challenge solver cannot silently fall out of a release. Public CI never sees private code or secrets.
- **Release automation.** This settles the item ADR 0030 left to the specs.
  - **Versioning.** An in-house `xtask release`, a small Rust program, sets the product version in every released manifest, keeps the internal `automan-*` crates at `0.0.x` and leaves the Zapier and Make apps to their platforms' versioning (ADR 0030), writes the changelog with git-cliff and opens a release pull request.
  - **Build.** Merging the release pull request builds every target once, with artifact attestations, signatures (ADR 0016) and a CycloneDX SBOM for the archives.
  - **Promotion.** A release candidate is published to the beta channel first. It is promoted to stable only once its beta week's gates are green: the benchmark (ADR 0025), the Private component run and the conformance suites; a patch runs the correctness oracle instead (ADR 0030).
  - **Publishing.** One job per registry: crates.io, npm (platform packages before the root), PyPI, the OCI image with its SBOM, and the GitHub Release.
  - **Credentials.** Every registry job publishes through OIDC trusted publishing only.
  - **Re-runs.** A re-run skips versions already published, and the tag is pushed only after npm succeeds.
  - **Installers.** The `curl | sh` and PowerShell installers are written by hand. Automan's own Homebrew tap is updated by the release job.
- **CI tiers.**
  - **Every pull request:**
    - One change-detection job decides what runs; workflow-level path filters are never used on required checks, because a skipped required check stays pending forever.
    - Formatting, lints and unit tests on Linux.
    - Tests of the SDKs and adapters the change touches, on one runtime each.
    - The suites of changed Sinks against emulators and containers on Linux only, images pinned by digest: RustFS for S3, and LocalStack for the AWS queues and streams (SQS, SNS, Kinesis, Firehose) under its free open-source licence, whose token, required since March 2026, is a CI secret. The merge queue runs every Sink's suite, so ADR 0026's rule that every pull request passes the sink suite before it merges still holds.
    - The docs build and example runs when docs change.
    - DCO, the notice inventory (ADR 0018), the API breakage checks (ADR 0030), `cargo-deny`, `cargo-audit` and their npm and Python equivalents, a check that every Action is pinned by SHA, and a build on the minimum supported Rust version.
    - One "all green" job, the only required check.
  - **Merge queue and `main`:** the full matrix of ADR 0024:
    - Platforms: Linux x64 and arm64 (glibc and musl), macOS arm64 and Intel (`macos-15-intel` or `macos-26-intel`), Windows x64 and arm64. If GitHub retires its Intel macOS runners, the x64 build is cross-compiled and tested under Rosetta 2 on arm64 runners.
    - Runtimes: every supported Node line, Bun and Deno; every supported CPython, with its free-threaded build from 3.14t on, on Linux.
    - Browser conformance on every operating system.

    Every target has a free GitHub-hosted runner for public repositories. The free plan's 20 concurrent jobs, 5 of them macOS, are why this tier waits for the merge queue.
  - **Nightly:**
    - Real SaaS sink accounts, which the owner approves case by case (ADR 0026).
    - Free-threaded Python on every operating system.
    - Every installed Chrome that ADR 0030 supports: the stable, beta and dev channels, every stable major of the last eight weeks, and Extended Stable.
    - The Private component run.
    - OpenSSF Scorecard.
    - The nightly channel's publish.
  - **Release candidate:** the benchmark of ADR 0025 runs on the reference host. The host is registered as a self-hosted runner that only tag and manual dispatch events reach, never pull-request code.
- **Caching.** `Swatinem/rust-cache`, saving only from `main`.
- **Organization.** The repository moves to a GitHub organization before the first release, because GitHub's documentation offers the merge queue to organization-owned repositories, and trusted-publishing identities name the repository. The organization also holds the private repository, the Homebrew tap and release rights for more than one maintainer. Its name follows [Product name: keep Automan or rename](https://github.com/Jbmanllr/automan/issues/83), as does the docs domain; GitHub redirects the old URLs. Until the move, `main` runs the full matrix after each merge.
