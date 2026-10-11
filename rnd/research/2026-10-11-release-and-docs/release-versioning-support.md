# Release, versioning and support practices: competitor and ecosystem research

Research date: 2026-10-11. Facts only. Every fact carries a source URL. "Not verified" means I searched and could not find a primary source; it does not mean the thing does not exist.

Tool note: the Exa MCP returned HTTP 402 (out of credits) on the first call, so this research used the Parallel web search/fetch MCP plus official docs, changelogs, npm/PyPI pages and GitHub.

---

## Q1. Lockstep vs independent versions; how engine/protocol compatibility is expressed

### Browser automation and scraping

**Playwright (Node, Python, Java, .NET)**
- Lockstep version numbers across language ports: PyPI `playwright` 1.63.0 (Sep 15, 2026) matches npm 1.63. https://pypi.org/project/playwright
- The Python package is a client that talks JSON over a pipe to a bundled Node driver. A single `DRIVER_VERSION` file (the `playwright-core` npm version) is the source of truth. The wheel build downloads that `playwright-core` plus a matching Node binary and assembles per-platform bundles. https://github.com/microsoft/playwright-python/blob/main/CLAUDE.md
- Python ships `py3-none-<platform>` wheels (for example `playwright-1.19.1-py3-none-manylinux_2_17_aarch64...whl`). https://pypi.org/project/playwright/1.19.1
- Browser pinning: "Each version of Playwright needs specific versions of browser binaries"; "every time you update Playwright, you might need to re-run the install CLI command." https://playwright.dev/docs/browsers
- Each release note lists the exact browser builds and the branded stable channels it was tested against. For example, 1.61 shipped Chromium 149.0.7827.55, Firefox 151.0 and WebKit 26.5, and was tested against Chrome 149 and Edge 149. https://playwright.dev/docs/release-notes
- Component skew is detected at runtime:
  - An integrity check fails when `@playwright/test` and `playwright-core` versions differ. https://github.com/microsoft/playwright/issues/22599
  - The Docker image tag must match the npm version. The error message names the current and required image. https://github.com/kubeshop/testkube/issues/3256
  - Remote `connect()` is "tightly version-coupled" (client and server must match), while `connectOverCDP` is generally backward and forward compatible. https://docs.browserless.io/baas/versions ; https://www.browserstack.com/docs/automate/playwright/troubleshoot-errors

**Puppeteer**
- release-please monorepo (`release-please-config.json`, `.release-please-manifest.json`). `puppeteer`, `puppeteer-core` and `@puppeteer/browsers` are versioned per package, with "Synchronize puppeteer versions" chores. https://github.com/puppeteer/puppeteer ; https://github.com/puppeteer/puppeteer/blob/main/CHANGELOG.md
- "Every Puppeteer release is tightly bundled with a specific browser release." https://pptr.dev/faq
- A published table maps Puppeteer versions to Chrome for Testing and Firefox versions (for example v25.12.0 maps to CfT 154.0.8037.57 and Firefox 156.0.1). https://pptr.dev/supported-browsers

**Crawlee JS**
- Lerna monorepo. All `@crawlee/*` packages share one version; changelogs contain "Version bump only for package @crawlee/basic". https://crawlee.dev/js/api/basic-crawler/changelog
- Crawlee v3 continued the Apify SDK numbering ("we decided to keep the versioning and release Crawlee as v3"). https://crawlee.dev/js/docs/upgrading/upgrading-to-v3

**Crawlee Python**
- Separate repository with its own version line (1.x; changelog shows 1.10.x). It is not lockstep with JS (3.18.1). https://github.com/apify/crawlee-python/blob/master/CHANGELOG.md ; https://github.com/apify/crawlee/blob/master/CHANGELOG.md

**Stagehand (v4, 2026)**
- Independent versions. "Each package can be selected independently in a Changeset; there are no fixed groups"; "TypeScript and Python do not have to be bumped together."
- The JSON-RPC protocol package (`packages/protocol`) is "versioned for compatibility tracking and is never published." Protocol semver rules:
  - compatible correction: protocol patch;
  - "backward-compatible capability that a new client may require": protocol minor;
  - "breaking wire or transport change": protocol major plus a coordinated release.
- The in-browser extension is embedded in each SDK, so "an SDK release is what delivers an extension fix to users."
- Source: https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md
- The v3 Python SDK was a Stainless-generated API client with its own versioning. https://docs.stagehand.dev/v3/sdk/python

**spider-rs**
- Independent versions:
  - the Rust `spider` crate is at v2.5x with very large patch numbers (v2.51.208, v2.53.4);
  - the PyPI `spider_rs` package is at 0.0.57;
  - the npm package is `@spider-rs/spider-rs`.
- Sources: https://github.com/spider-rs/spider/commits ; https://pypi.org/project/spider-rs ; https://github.com/spider-rs/spider/blob/main/README.md

**Firecrawl**
- The HTTP API is versioned in the URL (`/v1/`, `/v2/`). https://docs.firecrawl.dev/migrate-to-v2
- The server, SDKs and API each have their own version:
  - server release v2.8.0 (Feb 2026);
  - `firecrawl-py` 4.45;
  - Ruby `firecrawl-sdk` 1.17.0.
- In the SDK, "v1 methods (feature-frozen)" remain available.
- Sources: https://github.com/firecrawl/firecrawl/releases/tag/v2.8.0 ; https://pypi.org/project/firecrawl/ ; https://rubygems.org/gems/firecrawl-sdk

**Browser Use and Scrapy**
- Browser Use is a single Python package (0.13.x). https://pypi.org/project/browser-use/0.13.8
- Scrapy is a single Python package (2.x). https://doc.scrapy.org/en/latest/versioning.html

### Rust binary distributed through npm or PyPI

**Biome**
- The npm package `@biomejs/biome` "doesn't ship any binaries"; it delegates to `@biomejs/cli-<platform>` packages. https://biomejs.dev/reference/vscode
- All Biome crates are published under a single version, on demand. https://github.com/biomejs/biome/blob/main/RELEASES.md
- The config file's `$schema` URL embeds the version. A mismatch produces "The configuration schema version does not match the CLI version", and the fix is `biome migrate`. https://github.com/biomejs/biome/issues/5817

**Ruff and uv**
- The `ruff`, `ruff_linter` and `ruff_wasm` crates follow Ruff's versioning. https://docs.astral.sh/ruff/versioning/
- uv's internal crates are versioned `0.0.x` with "no stability guarantees"; their patch number increments on every uv release.
- The uv lockfile schema version "is considered part of the public API", so it only changes in a minor (breaking) release. Cache versions are internal.
- Source for uv: https://docs.astral.sh/uv/reference/policies/versioning/

**Turborepo**
- Platform binaries ship through npm (`turbo-darwin-arm64`, `turbo-linux-64`, and others). https://turborepo.dev/docs/support-policy
- A global `turbo` "defers to" the locally installed version. https://turborepo.dev/docs/getting-started/installation ; https://turborepo.dev/blog/turbo-1-7-0

**esbuild**
- Platform binaries are installed through `optionalDependencies`. https://github.com/evanw/esbuild/issues/1819
- The JS host checks the binary's version: "Host version "0.14.0" does not match binary version "0.13.8"". https://github.com/evanw/esbuild/issues/1806

**Prisma**
- The CLI and `@prisma/client` should be the same version; the CLI warns on mismatch. https://github.com/prisma/orm/issues/3037
- Rust engines are identified by a git hash carried by `@prisma/engines-version` ("Default Engines Hash"). `prisma version` prints it. https://www.prisma.io/docs/cli/v7/version ; https://github.com/prisma/engines-wrapper
- `@prisma/engines` is "intended for Prisma's internal use. Its release cycle does not follow SemVer." https://www.npmjs.com/package/@prisma/engines
- As of Prisma 7, the TypeScript query compiler (no Rust binaries) is the default. https://www.prisma.io/docs/orm/v6/more/internals/engines.md
- The v8 RC line uses exact peers between internal packages ("`@prisma/orm-toolchain` moves its exact `@prisma/cli-engine` peer") so the CLI runs "on the one engine version it installs". https://github.com/prisma/prisma/releases?after=1.24.0

**Tauri**
- Users must keep "the same minor version of the npm package `@tauri-apps/api` and cargo crate `tauri` synced." https://v2.tauri.app/develop/updating-dependencies
- Since plugins 2.2.0, "the versions for the Rust and JavaScript packages of each plugin will be in sync". https://tauri.app/release/process/all-versions
- Core packages (`tauri` 2.12.0, `@tauri-apps/api` 2.12.0, `tauri-cli` 2.12.0) move together, but helper crates (`tauri-build` 2.7.0, `tauri-bundler` 2.10.0) are independent. https://v2.tauri.app/release/

**Pydantic**
- `pydantic` pins an exact `pydantic-core`. A mismatch raises a SystemError ("pydantic 2.13.4 explicitly requires pydantic_core==2.46.4"). https://github.com/google/adk-python/issues/6041
- Since 2.12, pydantic raises an error "if an incompatible pydantic-core version is installed". https://pypi.org/project/pydantic/2.13.0b2
- Since v2.13, pydantic-core lives in the pydantic repo, tagged `core-vx.y.z`. https://github.com/pydantic/pydantic/blob/main/docs/version-policy.md

**Polars**
- Semver. https://docs.pola.rs/development/versioning
- Whether the Rust crate and the Python package are numbered in lockstep was not verified.

**Temporal**
- A shared Rust "Core SDK" powers the TypeScript, Python, .NET and Ruby SDKs, which link it in-process. https://github.com/temporalio/sdk-rust ; https://temporal.io/blog/why-rust-powers-core-sdk
- The SDKs version independently: TypeScript 1.x, Python 1.32.0. https://github.com/temporalio/sdk-python/blob/main/CHANGELOG.md
- The Core crate's "APIs ... are not considered stable and may break at any time." https://docs.rs/temporalio-sdk-core/latest/temporalio_sdk_core/index.html
- Server releases state "Minimum SDK versions" (for example Go v1.46.0, Python 1.30.0, TypeScript v1.20.2). https://github.com/temporalio/temporal/releases

**Dagger**
- "The Dagger CLI is released in tandem with the Dagger Engine and thus shares a version number"; "SDKs automatically provision a Dagger Engine at a compatible version". https://docs.dagger.io/next/getting-started/installation ; https://archive.docs.dagger.io/0.9/faq
- The engine enforces `MinimumClientVersion = "v0.19.0"` and `MinimumModuleVersion = "v0.9.9"`.
- Modules pin an `engineVersion`, and the engine gates semantics on it (for example `MinimumDangV2ModuleVersion = "v0.21.5"`).
- Source: https://github.com/dagger/dagger/blob/main/engine/version.go

**Pulumi**
- The CLI, engine and language SDKs live in one monorepo and share v3.x numbering. https://github.com/pulumi/pulumi ; https://pkg.go.dev/github.com/pulumi/pulumi/sdk/v3
- Language hosts speak gRPC. https://pkg.go.dev/github.com/pulumi/pulumi/sdk/v3@v3.260.0
- Docs are now versioned per CLI and SDK version back to v3.150.0. https://www.pulumi.com/blog/previous-cli-and-sdk-version-docs/

**Deno, oxc/oxlint and Rolldown/Vite**
- Deno is a single binary. https://docs.deno.com/runtime/fundamentals/stability_and_releases/
- oxlint follows semver. https://oxc.rs/docs/guide/usage/linter/versioning
- Rolldown reached 1.0 stable. https://voidzero.dev/posts/announcing-rolldown-1-0
- How these tie their npm platform packages to the binary version was not verified.

---

## Q2. Pre-1.0 vs 1.0; semver policy; public API definition; deprecation

### Scraping and browser automation

| Project | Status | Policy |
|---|---|---|
| Playwright | 1.x | No published semver or deprecation policy found. Python's alpha page promised to "become semver compatible and the API will be frozen" once the alpha label was dropped (https://pypi.org/project/playwright/1.9.0/). In practice, minor releases carry "Breaking Changes" sections: 1.60 removed macOS 14 WebKit support and the `@playwright/experimental-ct-svelte` package (https://playwright.dev/docs/release-notes). Users have filed issues about API removals in minors, for example `addTestCommand` in 1.25 (https://github.com/microsoft/playwright/issues/16458). |
| Puppeteer | v25 (major bumps frequent) | Breaking changes go into majors. v25 made packages ESM-only, set Node 22 as the minimum and removed deprecated APIs (https://pptr.dev/CHANGELOG). |
| Crawlee JS | 3.18.1 stable; v4 on `next` | The v4 upgrade guide lists many breaking changes, including native optional dependencies (`impit`, `@crawlee/fs-storage-native`): "--omit=optional ... is no longer safe" (https://crawlee.dev/js/docs/next/upgrading/upgrading-to-v4). Open issues are milestoned 4.0 as of Sep 2026 (https://github.com/apify/crawlee/issues). |
| Crawlee Python | 1.0 on Sep 15, 2025 | "From now on, Crawlee for Python will strictly follow semantic versioning ... breaking changes will only occur in major releases." https://crawlee.dev/blog/crawlee-for-python-v1 |
| Stagehand | v4 on Aug 10, 2026 | v2→v3 removed the Playwright dependency (https://docs.stagehand.dev/v3/migrations/v2). v3→v4 removed `agent()` and moved the SDK surface; for Python and Go "this is a rewrite" (https://docs.stagehand.dev/v4/migrations/v3). The v3 Python SDK "generally follows SemVer conventions, though certain backwards-incompatible changes may be released as minor versions" (https://docs.stagehand.dev/v3/sdk/python). |
| Browser Use | 0.13.x | No semver commitment found. About 155 releases at roughly one every 4 days (https://releasealert.dev/github/browser-use/browser-use). |
| Scrapy | 2.x | Version A.B.C: B releases may contain backward-incompatible changes that are "explicitly mentioned in the release notes"; underscore-prefixed names are private. "We aim to maintain support for deprecated Scrapy features for at least 1 year" (https://doc.scrapy.org/en/latest/versioning.html). Deprecations use `ScrapyDeprecationWarning` (https://github.com/scrapy/scrapy/blob/master/scrapy/utils/deprecate.py). Example: 2.19 removes `start_requests()`, deprecated in 2.13 (https://docs.scrapy.org/en/master/news.html). |
| spider-rs | 2.x | No written policy found. |
| Firecrawl | API v2, SDK 4.x | API-level versioning by URL. v1 SDK methods are feature-frozen (https://pypi.org/project/firecrawl/). |

### Rust binary plus SDK projects

| Project | Status | Policy |
|---|---|---|
| Ruff | 0.x | "uses the minor version number for breaking changes and the patch version number for bug fixes. Ruff does not yet have a stable API." Preview mode: preview rules stay at least one minor release before stabilization; anything behind preview may change (https://docs.astral.sh/ruff/versioning/). |
| uv | 0.x, "stable software" | Minor releases are breaking; patch releases are fixes and enhancements. "The care we take ... is proportional to the expected real-world impact." Lockfile schema is public API (https://docs.astral.sh/uv/reference/policies/versioning/). |
| Biome | 2.x | Semver with a toolchain-specific guide. Patch: false-positive fixes and some formatting fixes. Minor: new optional config options, new recommended rules, promoting nursery rules. Major: config changes that alter formatting or report more errors, and changes to the public API (https://biomejs.dev/internals/versioning). |
| oxlint | 1.x | Semver, with stated exclusions: nursery rules, JS plugins and type-aware linting may break in any release. "Minor version ... New rules ... not considered breaking changes even if they cause new errors" (https://oxc.rs/docs/guide/usage/linter/versioning). |
| Vite | 8.x | Semver. Some features are marked experimental (https://vite.dev/releases). |
| esbuild | 0.x (0.28) | Minor releases are breaking. Every breaking release starts with "This release deliberately contains backwards-incompatible changes ... pinning the exact version ... (recommended) or ... ~0.24.0" (https://github.com/evanw/esbuild/releases/tag/v0.25.0 ; https://esbuild.github.io/faq/). |
| Prisma | 7 stable; 8 RC | "Beginning with version 3.x.x, Prisma ORM adheres strictly to the SemVer." Preview and Early Access features are opt-in and are only promoted to GA in majors (https://www.prisma.io/docs/about/prisma/releases). |
| Tauri | 2.x | "tauri is following Semantic Versioning 2.0" (https://www.npmjs.com/package/@tauri-apps/api). |
| Pydantic | 2.x | "We will not intentionally make breaking changes in minor releases of V2. Functionality marked as deprecated will not be removed until the next major V3." It lists changes not considered breaking, such as ValidationError `msg`, `ctx` and `loc`, and JSON Schema reference format (https://pydantic.dev/docs/validation/dev/get-started/version-policy). |
| Polars | 1.x | Semver. "Deprecated functionality is removed two breaking releases after the deprecation happens" (deprecated in 1.2.3, kept in 2.0.0, removed in 3.0.0) (https://docs.pola.rs/development/versioning). |
| Deno | 2.x | Semver minor and patch on the stable channel (https://docs.deno.com/runtime/fundamentals/stability_and_releases/). |
| Dagger | 0.21.x, 1.0-beta docs | Treats the module API as a contract: "renaming a function or changing a default is a breaking change" (https://docs.dagger.io/sdks). |
| Temporal | SDKs 1.x | The Python changelog has a ":boom: Breaking Changes" section. Behavior changes ship behind SDK flags that stay default-disabled for two releases and are enabled in the third (https://github.com/temporalio/sdk-python/blob/main/CHANGELOG.md). |
| Pulumi | 3.x | No written policy found beyond release notes. |

---

## Q3. Release channels and cadence

| Project | Channels and publishing mechanism | Cadence |
|---|---|---|
| Playwright | npm dist-tags `latest`, `beta` (`1.58.2-beta-1771982315000`) and `next` (`1.59.0-alpha-2026-03-17`) (https://www.npmjs.com/package/@playwright/test?activeTab=versions) | No published fixed cadence found. Observed minors: 1.61 on Jun 29, 1.62 on Jul 31 and 1.63 on Sep 15, 2026 on PyPI (https://pypi.org/project/playwright). The third-party table versio.io lists 1.53 (Jun 2025) to 1.63 (Sep 2026), about one minor every 6 weeks (https://www.versio.io/en/product-release-end-of-life-eol-microsoft-playwright.html). |
| Puppeteer | npm `latest`; release-please | Frequent; Chrome rolls are released as minors. |
| Crawlee JS | "automated beta builds for every merged code change" on npm `next` (https://www.npmjs.com/package/crawlee) | As needed. |
| Stagehand | Changesets release PR. A `preview` label builds unpublished TS, wheel and extension artifacts that expire after 30 days (https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md) | On merge of the release PR. |
| Browser Use | PyPI only | About every 4 days. |
| Biome | Prereleases twice a week to pkg.pr.new (purged after about 30 days); npm `beta` before a stable; npm `latest`. VS Code extension uses even and odd numbers for stable and preview (https://github.com/biomejs/biome/blob/main/RELEASES.md ; https://biomejs.dev/internals/versioning) | No ETAs ("The team doesn't provide ETAs"). |
| Ruff | VS Code extension uses even and odd minors because VS Code lacks prerelease tags (https://docs.astral.sh/ruff/versioning/) | Weekly cadence not verified. |
| Turborepo | npm `-canary.N` prereleases (for example 2.7.5-canary.15) (https://socket.dev/npm/package/turbo/dependencies/2.11.8-canary.1) | Frequent. |
| Vite | Minors always get a beta; majors get alpha and beta | Patch roughly weekly, minor roughly every 2 months, major roughly yearly and aligned with Node EOL (https://vite.dev/releases). |
| Deno | `stable`, `lts`, `rc`, `canary` (multiple per day, one per commit), plus `alpha` and `beta` during a major prerelease; `deno upgrade lts|rc|canary` (https://docs.deno.com/runtime/fundamentals/stability_and_releases/ ; https://docs.deno.com/runtime/reference/cli/upgrade) | A minor every 12 weeks, with weekly patches (https://docs.deno.com/runtime/contributing/release_schedule/). |
| Prisma | npm `dev` tag on every push to main (now `8.0.0-rc.X-dev.N`); RC line `8.0.0-rc.N`. "a ^0.x range can never match a 8.0.0-rc.N pre-release" (https://github.com/prisma/prisma/releases?after=1.24.0) | "typically every two weeks" (https://www.prisma.io/docs/about/prisma/releases). |
| Tauri | npm `next` tag (https://www.npmjs.com/package/@tauri-apps/api?activeTab=versions); covector workflow (https://npm.io/package/@tauri-apps/api) | As needed. |
| Polars | Pre-releases exist (section present) (https://docs.pola.rs/development/versioning) | "a new minor version is released about once every two weeks." |
| Pydantic | PyPI alpha and beta prereleases (2.13.0b2) (https://pypi.org/project/pydantic/2.13.0b2) | Roughly yearly majors were "expected" (older policy text) (https://pydantic.dev/docs/validation/2.8/get-started/version-policy). |
| Dagger | Dev builds via `DAGGER_COMMIT=head`; running them against a stable engine replaces the engine (https://docs.dagger.io/next/getting-started/installation) | Frequent. |
| Pulumi | Single stream | Very frequent: about 29 releases between v3.236.0 (May 6, 2026) and v3.265.0 (Sep 25, 2026), counted from pkg.go.dev dates (https://pkg.go.dev/github.com/pulumi/pulumi/sdk/v3@v3.236.0 ; https://pkg.go.dev/github.com/pulumi/pulumi/sdk/v3). |
| Rust (toolchain) | nightly, beta and stable train | Every 6 weeks (https://rust-lang.github.io/rfcs/0507-release-channels.html ; https://blog.rust-lang.org/2014/10/30/Stability). |
| Scrapy | `dev`-suffixed development releases (https://doc.scrapy.org/en/latest/versioning.html) | As needed. |

**Prerelease semantics worth knowing**
- npm: prereleases do not satisfy stable ranges (the Prisma note above).
- npm: `npm deprecate pkg@"1.x"` also deprecates 1.x prereleases. https://unpkg.com/npm@11.7.0/docs/output/commands/npm-deprecate.html

---

## Q4. Support windows, LTS and security backports

| Project | Policy |
|---|---|
| Vite | Current minor gets regular fixes. Previous major (latest minor) and previous minor get important fixes and security patches. Second-to-last major and minor get security patches only. Example: 8.3 regular; 7.3 and 8.2 important and security; 6.4 and 8.1 security only. https://vite.dev/releases ; https://v8.vite.dev/releases |
| Turborepo | "Major versions of Turborepo will be supported for two years from the release date of the next major version." Critical security backports "will land as semver-minor releases, even if they are breaking changes." https://turborepo.dev/docs/support-policy |
| Deno | LTS channel: one stable minor line with only backward-compatible fixes, covering security patches and critical bugs. History: v2.1 (Feb–Apr 2025), v2.2 (May–Oct 2025), v2.5 (Nov 2025–Apr 2026), v2.9 (Jul 1, 2026–Jan 31, 2027). https://docs.deno.com/runtime/fundamentals/stability_and_releases/ |
| Pydantic | V1 gets critical bug and security fixes "until the release of Pydantic V3". It previously said "for one year after the release of V2". https://pydantic.dev/docs/validation/dev/get-started/version-policy |
| Pydantic AI | Deprecations are not removed before the next major, which comes no sooner than 3 months after V2.0. V1 gets security fixes for at least 6 months after V2. https://pydantic.dev/docs/ai/project/version-policy/ |
| Prisma | "Prisma ORM 7 remains fully supported" while 8 is in RC. https://www.prisma.io/docs/orm |
| Prisma | 7.9.1 was a patch "to resolve a security advisory in a transitive dependency". https://github.com/prisma/prisma/releases?after=2.20.0 |
| Puppeteer | Auto-deprecates old versions on npm, for example "< 24.15.0 is no longer supported". The rationale (issue #7939) is to deprecate versions older than the current Chrome Stable milestone. https://www.npmjs.com/package/puppeteer/v/22.6.2 ; https://github.com/puppeteer/puppeteer/issues/7939 |
| Playwright | No LTS or support window found. The third-party versio.io says it "does not publish fixed end-of-life dates ... 'latest version' model" (not an official statement): https://www.versio.io/en/product-release-end-of-life-eol-microsoft-playwright.html. Patch releases exist on the current minor (1.61.1, 1.59.1, 1.58.2): https://github.com/microsoft/playwright/releases?after=v1.6.1. The VS Code extension dropped support for Playwright ≤1.43: https://github.com/microsoft/playwright-vscode/releases |
| Crawlee JS | Older doc versions are marked "no longer actively maintained". https://crawlee.dev/js/docs/3.11/upgrading/upgrading-to-v3. A support window for v3 after v4 was not verified. |
| Scrapy | On backports: "backporting to 2.6.x is enough"; older versions are not patched for new dependency breakage. "If we had to update all Scrapy versions every time ... maintenance nightmare." https://github.com/scrapy/scrapy/issues/5635 |
| Temporal Server | Announces deprecated API removals across server releases. For example, deprecated Worker Versioning was planned for removal in v1.33. https://github.com/temporalio/temporal/releases |
| Ruff, uv, Biome, oxc, esbuild, Browser Use, spider-rs, Firecrawl, Stagehand, Dagger, Pulumi, Tauri, Polars | No written LTS or backport window found. |

---

## Q5. Release and API-breakage tooling, and who uses what

### Tool facts

- **release-plz** (Rust): release PRs driven by conventional commits, git-cliff changelogs and cargo-semver-checks. Detected breakage forces a major bump (for example 0.4.21 to 0.5.0). Supports `version_group` for packages sharing a version. "The only release tool that integrates cargo-semver-checks." https://release-plz.dev/docs/semver-check ; https://release-plz.dev/docs/why ; https://release-plz.dev/docs/config
- **cargo-semver-checks**: lints a crate's API via rustdoc JSON, which is unstable. It "won't [catch every violation] — not yet". https://crates.io/crates/cargo-semver-checks ; https://release-plz.dev/docs/semver-check
- **release-please**: conventional commits (`feat!:` means major). Rust workspaces need manifest mode and the `cargo-workspace` plugin. https://www.npmjs.com/package/release-please
- **Changesets**: per-PR `.changeset/*.md` files, then a release PR (Stagehand and Biome usage below).
- **git-cliff**: conventional-commit changelog generator, used by release-plz. https://git-cliff.org/
- **Changie**: changelog fragments; Dagger's CHANGELOG is "generated by Changie". https://github.com/dagger/dagger/blob/main/CHANGELOG.md
- **covector**: Tauri's workflow `.github/workflows/covector-version-or-publish.yml`. https://npm.io/package/@tauri-apps/api
- **API Extractor**: produces an API report, a Markdown file of API signatures, for review. https://api-extractor.com/pages/overview/demo_api_report
- **@arethetypeswrong/cli (`attw --pack .`)**: detects TypeScript types and ESM resolution problems under node10, node16 and bundler resolution. https://github.com/arethetypeswrong/arethetypeswrong.github.io/blob/main/packages/cli/README.md
- **griffe check**: compares a Python package against a git ref or a PyPI version to find API breakages (for example a parameter kind changed). https://mkdocstrings.github.io/griffe/guide/users/checking ; https://libraries.io/pypi/griffe

### Who uses what (verified only)

| Project | Tooling |
|---|---|
| Puppeteer | release-please (https://github.com/puppeteer/puppeteer) |
| Stagehand | Changesets (https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md) |
| Biome | Changesets (`@changesets/cli` 3.0.1 and `changeset version` in package.json) (https://github.com/biomejs/biome/blob/main/package.json). Biome's own `packageManager` is `pnpm@12.3.4`. |
| Crawlee JS | Lerna plus conventional commits ("See Conventional Commits for commit guidelines") (https://github.com/apify/crawlee/blob/master/CHANGELOG.md) |
| Tauri | covector |
| Dagger | Changie |
| Pydantic | Git tags `v*`, with `core-v*` for pydantic-core (https://github.com/pydantic/pydantic/blob/main/docs/version-policy.md) |
| Griffe itself | Runs `griffe check` in CI as a non-blocking informational job (https://github.com/mkdocstrings/griffe/blob/main/duties.py) |
| Playwright, Ruff/uv, oxc, Turborepo, Deno, Prisma, Pulumi, Polars, Temporal, Firecrawl, Browser Use, spider-rs | Specific release tool not verified |

---

## Hotfix problem: release-age gates (cooldowns) in 2026

### npm, pnpm, Yarn, Bun and Deno

**pnpm**
- `minimumReleaseAge` (in minutes) was added in v10.16.0. https://pnpm.io/10.x/settings ; https://pnpm.io/blog/releases/10.16
- `minimumReleaseAgeExclude` also arrived in 10.16.0 (by package name). Patterns came in 10.17.0 and exact versions (`nx@21.6.5`, `webpack@4.47.0 || 5.102.1`) in 10.19.0. https://pnpm.io/10.x/settings
- **pnpm 11.0 (Apr 28, 2026)** turned it on by default: `minimumReleaseAge` defaults to `1440` (1 day), `minimumReleaseAgeStrict` defaults to `false`, and `blockExoticSubdeps` defaults to `true`. Opt out with `minimumReleaseAge: 0`. https://pnpm.io/blog/releases/11.0
- Strictness depends on configuration. It is "true if minimumReleaseAge is explicitly configured, false otherwise". The built-in default is non-strict, so pnpm falls back to an older version when nothing satisfies the gate. https://github.com/pnpm/pnpm.io/blob/main/docs/settings/dependency-resolution.md
- `minimumReleaseAgeIgnoreMissingTime` defaults to `true`: pnpm fails open when the registry omits publish times. Same source.
- `pnpm audit --fix` "adds the minimum patched version for each advisory to `minimumReleaseAgeExclude` ... so security fixes can be installed without waiting." https://pnpm.io/blog/releases/11.0
- pnpm 12 exists (Biome pins `pnpm@12.3.4`). cooldowns.dev says "Since pnpm 12.3.0, an explicitly configured minimumReleaseAge is strict by default". I did not verify this in pnpm's own docs. https://cooldowns.dev/

**Yarn Berry**
- `npmMinimalAgeGate` (minutes or a duration).
  - Introduced in 4.10.0 per Andrew Nesbitt and several guides. Yarn's security page says "Yarn 4.12 introduced", so the sources conflict.
  - The default became `1d` in the 4.15 branch (May 2026); PR #7135 was merged 2026-05-18. The Yarn post says the default applies "for new projects".
- `npmPreapprovedPackages` accepts globs and exact locators and bypasses the gate. `--no-time-gate` on `yarn add` and `yarn up` bypasses it for a single command.
- A known parsing bug ignored day suffixes (berry#6991).
- Sources: https://yarnpkg.com/features/security ; https://github.com/yarnpkg/berry/pull/7135 ; https://v6.yarnpkg.com/blog ; https://nesbitt.io/2026/03/04/package-managers-need-to-cool-down.html ; https://gist.github.com/mcollina/b294a6c39ee700d24073c0e5a4e93104

**npm**
- `--before <date>` is long-standing. https://github.com/npm/cli/issues/5702
- `min-release-age` (in days) arrived in **npm 11.10.0 (Feb 2026)**, with default `null` (off).
- When a window blocks the patched version `npm audit fix` would install, "npm keeps the package at its vulnerable version, warns ... and exits with a non-zero code. To install the fix, add the package to `min-release-age-exclude`."
- `min-release-age-exclude[]` takes names or minimatch globs. Only the named package is exempt, not its dependencies.
- If `before` and `min-release-age` are set in the same source, `before` wins.
- cooldowns.dev dates `min-release-age-exclude` to 11.17.0; I did not verify the exact version.
- Sources: https://docs.npmjs.com/cli/v11/using-npm/config/ ; https://www.brandonpugh.com/til/node/package-version-cooldown ; https://cooldowns.dev/
- A proposal to default to 7 days is open (npm/rfcs#864). Some commenters argue the cooldown delays patches. https://github.com/npm/rfcs/issues/864
- An exclusion request for CVE mitigation: https://github.com/npm/rfcs/issues/895

**Bun**
- `install.minimumReleaseAge` (in seconds) arrived in Bun 1.3 (Oct 2025), with default `null` (disabled).
- Related settings: `minimumReleaseAgeExcludes` and the CLI flag `--minimum-release-age`.
- Exact version requests "still respect the age gate".
- Sources: https://bun.sh/docs/runtime/bunfig ; https://bun.com/docs/pm/cli/install

**Deno**
- `minimumDependencyAge` arrived in Deno 2.6 (minutes, ISO 8601 or RFC 3339). https://deno.com/blog/v2.6
- Per cooldowns.dev, a 24-hour default applies since Deno 2.9, and since 2.8 Deno honors `min-release-age` in `.npmrc`. https://cooldowns.dev/

### Python and Rust

**uv**
- `exclude-newer` accepts RFC 3339 timestamps, "friendly" durations (`24 hours`, `1 week`) or ISO 8601 durations. It is compared against each artifact's upload time.
- `exclude-newer-package = { pkg = false }` exempts a single package.
- Sources: https://docs.astral.sh/uv/reference/cli/ ; https://github.com/astral-sh/uv/blob/main/uv.schema.json
- A bug made `uv pip compile` ignore the per-package override (since fixed). https://github.com/astral-sh/uv/issues/19239

**pip**
- `--uploaded-prior-to` was added in 26.0, and relative `PnD` durations in 26.1.
- There is no per-package exemption. The workaround is to unset `PIP_UPLOADED_PRIOR_TO` for the fix install.
- Sources: https://pip.pypa.io/en/stable/user_guide ; https://cooldowns.dev/

**Poetry**
- `solver.min-release-age` and `min-release-age-exclude`. Renovate sets `POETRY_SOLVER_MIN_RELEASE_AGE`. https://docs.renovatebot.com/key-concepts/minimum-release-age/ ; https://github.com/mprpic/cooldowns

**Cargo**
- RFC 3923 adds `registry.global-min-publish-age`, per-registry `min-publish-age` and the override `CARGO_RESOLVER_INCOMPATIBLE_PUBLISH_AGE=allow` "for exceptions like urgent security fixes". https://rust-lang.github.io/rfcs/3923-cargo-min-publish-age.html
- cargo#15973 closed as completed via PR #17012. https://github.com/rust-lang/cargo/issues/15973
- cooldowns.dev lists Cargo as "Unstable on nightly". The third-party `cargo-cooldown` crate also exists. https://cooldowns.dev/

### Update bots

**Dependabot**
- `cooldown` (default-days, semver-major/minor/patch-days, include and exclude) shipped July 2025.
- Since July 2026, a default 3-day cooldown applies to version updates with no configuration.
- Security updates are exempt.
- Sources: https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference ; https://github.com/dependabot/dependabot-core/discussions/15582

**Renovate**
- `minimumReleaseAge`; "Security updates bypass any minimumReleaseAge checks."
- Passes `--before` to npm when regenerating lockfiles.
- The preset `security:minimumReleaseAgeNpm` gives 3 days in the Renovate 42 best-practices config.
- Sources: https://docs.renovatebot.com/key-concepts/minimum-release-age/ ; https://nesbitt.io/2026/03/04/package-managers-need-to-cool-down.html

### How urgent fixes are handled under these gates

- Guidance found:
  - Add a narrow per-package exemption, install the fix, then remove the exemption. cooldowns.dev warns: "always revert bypass exemptions after installing the fix." https://cooldowns.dev/ ; https://github.com/arnica/depsguard
  - Native cooldowns "can't tell a security fix from any other new release". Pair them with vulnerability alerting. https://cooldowns.dev/
  - pnpm 11 automates this: `pnpm audit --fix` writes the patched version into `minimumReleaseAgeExclude`. https://pnpm.io/blog/releases/11.0
  - npm documents the exclude as the remedy when `audit fix` is blocked. https://docs.npmjs.com/cli/v11/using-npm/config/
- I found no maintainer of a CLI or SDK tool publishing "add our package to your exclude list" instructions for its own users. The examples found are generic (internal scopes such as `@myorg/*`, `@platformatic/*`). This was not verified across the ecosystem.

**Implications for a binary-in-optionalDependencies layout**
- The gates apply per package version and to transitive dependencies, so `@scope/cli-<platform>` packages are gated like the main package. pnpm, npm, Yarn and Bun all accept globs (`@scope/*`), which covers the main package plus its platform packages.
- npm notes that "only the named package is exempt; its own dependencies still follow the release-age policy unless they also match a pattern". https://docs.npmjs.com/cli/v11/using-npm/config/

---

## User pains (real reports)

**Playwright**
- Every upgrade needs new browser builds ("Executable doesn't exist ... Looks like Playwright Test or Playwright was just installed or updated"). CI broke overnight with "no code changed". https://github.com/microsoft/playwright/issues/12772
- The Docker image, the client and the runner must all match exactly. https://github.com/kubeshop/testkube/issues/3256 ; https://dev.to/raju_lingampalli_8813d86f/any-facing-playwright-docker-image-issue-as-below-3d55
- A hoisted `@playwright/test` or `playwright-core` skew failed the integrity check. https://github.com/microsoft/playwright/issues/22599
- `npx playwright test` grabbing a newer core broke an API in a minor release: "your CI can randomly fail one day after a release". https://github.com/microsoft/playwright/issues/16458
- Remote `connect()` version coupling with Browserless and BrowserStack. https://station.railway.com/questions/browserless-chrome-mismatch-with-playwri-fb51057b

**Puppeteer**
- v19 moved the browser cache to `~/.cache/puppeteer`: "Could not find expected browser (chrome) locally" right after upgrading. Users reverted. https://github.com/puppeteer/puppeteer/issues/9119
- A cache-path problem with 21.x. https://github.com/puppeteer/puppeteer/issues/12006
- A user asked for an urgent release carrying a Chromium security fix: "Please release 21.3.0 ASAP". https://github.com/puppeteer/puppeteer/issues/10939

**Crawlee**
- The v4 guide warns that `--omit=optional` from v3 Dockerfiles now breaks installs because of native deps. https://crawlee.dev/js/docs/next/upgrading/upgrading-to-v4
- Crawlee Python had to cap the `stagehand` dependency "below the breaking 4.x API". https://github.com/apify/crawlee-python/blob/master/CHANGELOG.md
- Apify Python SDK v3 inherited the Crawlee 1.0 storage API breaks. https://docs.apify.com/sdk/python/docs/upgrading/upgrading-to-v3
- I did not find a high-signal Reddit or HN thread specifically about Crawlee v2→v3 or v3→v4 pain (not verified).

**Stagehand**
- v3 broke storage-state persistence (`userDataDir` ignored, `storageState()` removed), and the issue complains about "API Changes Not Documented". https://github.com/browserbase/stagehand/issues/1250
- v3 cannot run in Cloudflare Workers. https://github.com/browserbase/stagehand/issues/1381
- v4 removed `agent()` outright, and Python and Go users face a "rewrite". https://docs.stagehand.dev/v4/migrations/v3

**Browser Use**
- The 0.1.45→0.2.4 migration introduced BrowserSession and BrowserProfile and broke shared-browser patterns. https://github.com/browser-use/browser-use/discussions/1794
- Exact `==` pins cause dependency conflicts and leave CVE-flagged pins in place. https://github.com/browser-use/browser-use/issues/4824
- Pinning `openai==1.99.2` broke users. https://github.com/browser-use/browser-use/issues/2634

**Scrapy**
- 2.2→2.3 broke `parse()` for one user. https://github.com/scrapy/scrapy/issues/4854
- 2.13 broke third-party handlers with "TypeError ... download_request() takes 2 positional arguments". https://github.com/jxlil/scrapy-impersonate/issues/55
- An unpinned pyOpenSSL broke `pip install Scrapy` on older lines, and maintainers declined broad backports. https://github.com/scrapy/scrapy/issues/5635

**Prisma**
- A CLI/client "version mismatch" persisted after an upgrade until `node_modules` was wiped. https://github.com/prisma/prisma/issues/15924
- Wrong engine binary target or OpenSSL detection. https://github.com/prisma/prisma/issues/23212 ; https://github.com/prisma/orm/discussions/21508
- Stale engine hash after an upgrade in a monorepo. https://github.com/prisma/prisma/issues/24690

**Turborepo**
- `npx -y turbo` pulled v2 against a v1 repo, and the global binary failed to infer the local v1. https://github.com/vercel/turborepo/issues/8309

**Biome**
- The VS Code extension fell back to a downloaded global 1.9.3 instead of the local binary. https://github.com/biomejs/biome-vscode/issues/403
- mise-managed per-project binaries were not found. https://github.com/biomejs/biome-vscode/issues/528
- "Could not determine the version of Biome binary". https://github.com/biomejs/biome-vscode/issues/1030
- Schema version mismatch warnings repeated after an upgrade. https://github.com/biomejs/biome/issues/5817

**esbuild**
- "Host version ... does not match binary version" when multiple esbuild versions coexist (Storybook, Nuxt, pnpm). https://github.com/storybookjs/storybook/issues/26112 ; https://github.com/evanw/esbuild/issues/1806
- A 0.x minor break hit AWS CDK users who installed `esbuild@0` unpinned ("Why was an EXPERIMENTAL deliberate breaking change introduced as a minor version bump?!"). https://github.com/evanw/esbuild/issues/3819

**Pydantic**
- A pydantic and pydantic-core exact-pin skew raised SystemError at startup in a hosted runtime. https://github.com/google/adk-python/issues/6041

**Semver for 0.x in general**
- Tooling treats every 0.y.z release as potentially breaking. https://github.com/semver/semver/issues/734

---

## Patterns (short summary)

1. **Lockstep across languages appears where one engine sits behind thin clients.** Examples: Playwright (driver), Dagger (CLI equals engine), Pulumi, Tauri core, Biome crates. **Independent versions appear where SDKs are generated or hand-written per language.** Examples: Stagehand v4, Temporal, Firecrawl, spider-rs, Crawlee JS vs Python.
2. **Compatibility is usually enforced at runtime, not only in docs.** Examples:
   - exact-version integrity checks (Playwright, esbuild, pydantic-core, Prisma);
   - minimum-client constants (Dagger `MinimumClientVersion`, Temporal "Minimum SDK versions");
   - a separately versioned, unpublished protocol package (Stagehand);
   - schema-version warnings with a migrate command (Biome).
3. **0.x tools redefine semver** so that minor means breaking (Ruff, uv, esbuild). 1.x tools carve out explicit "not covered by semver" zones: preview or nursery rules (Ruff, Biome, oxlint), experimental features (Vite, Pydantic), internal crates at 0.0.x (uv), or internal packages that "do not follow SemVer" (`@prisma/engines`).
4. **Deprecation windows are time-based or major-based.** Examples: Scrapy at least 1 year; Polars two breaking releases; Pydantic until the next major; Temporal behavior flags over three releases.
5. **Channels and support:**
   - Channels are commonly npm dist-tags (`next`, `beta`, `canary`, `dev`) or per-commit builds, sometimes on pkg.pr.new.
   - Support windows, where they exist, are formulaic: Vite's N/N-1/N-2 minors and majors, Turborepo's two years after the next major, Deno's roughly 6-month LTS.
   - Most scraping and AI-browser projects publish no support window.
6. **Release-age gates are now on by default in pnpm 11 (1 day), Yarn ≥4.15 (1 day) and Deno 2.9 (24h, per cooldowns.dev), and in Dependabot (3 days for version updates only).**
   - Every package manager offers glob or per-package exemptions. pnpm automates the exemption for audit fixes, and npm fails `audit fix` loudly with a pointer to the exclude list.
   - Bots exempt security updates; package managers do not.
