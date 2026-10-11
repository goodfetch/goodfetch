# One product version, a minor every four weeks after Chrome stable, and contracts versioned on their own

Every Automan package carries one version and is released together: the `automan` binary (engine, CLI, MCP server, Browser host and embedded Console), the Rust crate, the npm and PyPI SDKs, the agent skills, the agent-framework adapters (ADR 0011), the sink processes (ADR 0026), the Integration packages that a package registry versions (ADR 0027, such as the n8n node, the Airbyte source and the LangChain and LlamaIndex loaders), the Homebrew formula and the OCI image. Zapier and Make apps follow their platforms' own versioning. Each SDK release already bundles one exact engine (ADR 0024), so separate version numbers would only describe a pairing that is fixed anyway. Formats that outlive a release carry their own `major.minor` version, separate from the product's. `1.0.0` is V1: it ships when the map's completion criteria are met. The public previews before it are `0.y.z`, where any minor may break. From 1.0, a minor ships every four weeks, one week after each Chrome stable release, so every minor carries the current browser and the stealth material that matches it. Decided in [Release, versioning and documentation policy](https://github.com/Jbmanllr/automan/issues/100), on the evidence of the [release and versioning research](https://github.com/Jbmanllr/automan/blob/research/release-and-docs/rnd/research/2026-10-11-release-and-docs/release-versioning-support.md) and the [documentation research](https://github.com/Jbmanllr/automan/blob/research/release-and-docs/rnd/research/2026-10-11-release-and-docs/documentation.md).

## Considered Options

- **Independent versions per package** (Stagehand with Changesets, Temporal's SDKs): rejected. They let an SDK fix ship without an engine release, but users then need a compatibility matrix. Playwright, Dagger and Biome keep one version, and version skew between a global binary and a local SDK is the recurring pain in Prisma, Turborepo and Biome.
- **A long `1.0.0-alpha`, `beta` and `rc` series for the previews**: rejected. It suits a few weeks before launch, not months of previews. Ruff, uv and esbuild ship 0.x with breaking minors, and Crawlee for Python went from 0.x to a strict-semver 1.0 in September 2025. A short `1.0.0-rc.N` series still precedes 1.0.
- **A slower or unscheduled cadence** (Playwright about every 6 to 8 weeks with no fixed calendar, Deno every 12 weeks, Rust's 6-week trains): rejected. HTTP impersonation tools lag Chrome by 5 to 7 versions ([State of the art in stealth and challenge solving](https://github.com/Jbmanllr/automan/issues/64)), and following Chrome keeps the shipped browser and the stealth material consistent with each other.
- **Backports to older minors** (Vite backports to N-1 and N-2): rejected. Minors are monthly and non-breaking within a major, so upgrading is the fix.
- **Bypassing the release-age gates**: rejected. Since pnpm 11 and Yarn 4.15, new projects hold back releases younger than a day by default; pnpm 11 then silently installs the previous version instead of failing. The gates protect users from supply-chain attacks, so a hotfix ships with the steps to let it through the gate.
- **One Daemon per user across majors**, ADR 0024's first rule: rejected. While two majors are supported, a project on 1.x and one on 2.x would keep restarting each other's Daemon. Gradle likewise runs a separate daemon per version.
- **An online update check** (the `gh` CLI, Deno): rejected. It is more accurate than an age notice, but it is a network call from the user's machine, which users of a scraping framework may not want.

## Consequences

- **Contract versions.** These formats each carry their own `major.minor`, checked when read:
  - the JSON-RPC protocol (ADR 0024)
  - the sink protocol (ADR 0026)
  - run state and Checkpoints (ADR 0002)
  - the Plan file format (ADR 0009)
  - the Recipe manifest
  - the settings schema (ADR 0028)
  - Dataset and Export formats

  An engine reads every older minor of a format within its major. A newer minor than it knows, such as a Checkpoint written by 1.5 and resumed by 1.4 on another machine, is refused with a message to upgrade, as the protocol handshake refuses a newer Client. A file of an older major is refused with a message naming `automan migrate`, and one of a newer major with a message to upgrade. A format's major changes only with a product major. In this ADR, "major" and "minor" alone mean the product's version, not a format's.
- **What semver covers from 1.0.** A breaking change requires a new major in any of these:
  - the TypeScript and Python SDKs and the Rust crate's top-level `automan` API
  - the Operation catalog: operation names, inputs, outputs and classes, so MCP tool names and schemas too
  - CLI commands, flags, `--json` output and exit codes
  - setting keys and their meaning
  - the contract formats above
  - webhook Sink and Notification payloads
  - the HTTP listener (ADR 0027)
  - error codes

  Semver does not cover:
  - Console screens, although the Console's functions stay, such as the reconciliation of uncertain Effects (ADR 0029)
  - human-readable CLI output and log text
  - internal `automan-*` crates, published at `0.0.x` as uv does
  - anything Experimental
  - pinned browser builds and stealth material (fingerprints, impersonation profiles, challenge detectors)
  - AI prompts and the model-facing wording of tool descriptions

  A changed default is breaking only when it weakens a guarantee or a security control. A tuning default, such as a timeout, may change in a minor and is listed in the release notes. Dropping a runtime past its end of life (ADR 0024) or raising the minimum Rust version is announced one minor ahead and happens in a minor, as Pydantic, Vite and Playwright drop Node and Python lines.
- **Experimental.** From 1.1, a new operation, setting or SDK method may land Experimental. It is marked in the catalog, the docs, the tool description and the generated types, and warns once per process when used. It stays outside semver until promoted, so any minor may change or remove it, like Biome's nursery rules and Ruff's preview mode. Every V1 capability is stable at 1.0, so Experimental never defers V1.
- **Deprecation.** A deprecation lands in a minor. It shows as a warning in the SDKs and on the CLI's stderr, as `deprecated` in the MCP tool schema, and in the release notes, which name the replacement. Removal comes no earlier than the next major and no earlier than six months after the deprecation.
- **Channels.**
  - **Stable:** npm `latest`, PyPI, crates.io, Homebrew, the installers and the OCI `latest` tag.
  - **Beta:** an `x.y.0-rc.N` cut one week before every minor (npm `next`, PyPI `x.y.0rcN`, crates.io `-rc.N`).
  - **Patches:** shipped whenever a fix is ready, on the stable channel.
  - **Nightly:** built from `main` to npm `nightly`, PyPI `.devN`, GitHub pre-releases and an OCI `nightly` tag, but not to crates.io or Homebrew.
- **Browsers and stealth between minors.**
  - **Patches:** a patch may move each downloaded browser to a newer build within its own upstream major, security point releases included, and may update stealth material. This covers the pinned Chrome for Testing build, Playwright's Firefox and WebKit builds, Camoufox, Patchright and Lightpanda.
  - **Minors:** a new upstream major, Chrome's included, comes only with a minor.
  - **Release notes:** they always name every browser build.
  - **Supported Chrome:** besides the pinned build, a run may use an installed Chrome. That Chrome is supported on the current stable, beta or dev channel and the two previous stable majors, all tested against the guard (ADR 0019) and the conformance suite. A musl host's distribution Chromium follows the same rule.
  - **Best effort:** older builds, Edge and Brave run best effort, with a warning.
- **Majors.** A major ships only once breaking changes have been collected and announced. It comes with a migration guide with before-and-after examples. `automan migrate` upgrades every contract format, like `biome migrate`. It writes the new major's copy and never changes the files the previous major still uses. SDK code relies on the deprecation warnings rather than codemods.
- **Support.**
  - **Previews:** a `0.y` preview is supported until the next `0.y` minor ships.
  - **Current major:** only the latest minor gets patches.
  - **Previous major:** its last minor gets security and data-loss fixes for 12 months after the next major ships.
  - **Per-major state:** each major runs its own Daemon, with its own runtime file, socket and Console port, so a 1.x and a 2.x project never restart each other's (amends ADR 0024). The user-level settings and the per-install layout of ADR 0002 are also kept per major. The browser cache is shared, because its builds are checked by digest.
  - **After support ends:** versions are marked deprecated on npm, as Puppeteer does.
  - **Age notice:** a binary more than three months past its own release date prints one line a day. The line says that newer versions carry security fixes and a current Chrome, and gives the upgrade command. It compares dates only, makes no network call, and a setting turns it off. It goes to stderr only, never into `--json` output or a stdio protocol stream (Attached engine or MCP).
  - **Rust:** the crate's minimum supported Rust version is the latest stable minus two releases, tested in CI and raised only in a minor.
- **Hotfixes under release-age gates.**
  - **Advisory:** every security fix ships with a GitHub Security Advisory and a CVE, so `npm audit` and `pnpm audit` find it. `pnpm audit --fix` also adds the exclusion.
  - **Exclusion snippets:** the advisory and the release notes give a copy-paste exclusion for pnpm, Yarn, npm, Bun, Deno and uv. Each is scoped to `automan` and `@automan/*`, so the platform-binary packages are included, and reminds the user to remove it afterwards.
  - **Ungated channels:** the installers, Homebrew, the OCI image and GitHub Releases have no gate and carry the fix at once. With `AUTOMAN_ENGINE_PATH`, a gated npm or PyPI project can run the fixed engine the same day.
- **Security policy.** `SECURITY.md` uses GitHub private vulnerability reporting, as decided in [Security and compliance model](https://github.com/Jbmanllr/automan/issues/73). Each report is acknowledged within three business days, the fix ships as above, and disclosure is coordinated within 90 days of the report.
- **Release gates.** Any of these blocks a release:
  - **Conformance:** suites green on every supported platform and runtime (ADR 0024).
  - **Benchmark, for minors and majors:** ADR 0025's rule, run on the release candidate during its beta week against the previous Automan release. A correctness failure always blocks. A primary metric worse than the previous release by more than its Δ blocks unless the release notes explain it. A blocking result delays that minor; the next minor keeps its own date. A patch runs the correctness oracle only. Competitor rows are rerun alongside. When a competitor's configuration changed, they wait for ADR 0025's 14-day maintainer review, so they never hold up a release.
  - **API breakage checks:** cargo-semver-checks for the crate, an API Extractor report for TypeScript, `griffe check` for Python, and a diff of the Operation catalog and settings schema, each against the previous release of the same major. An unannounced break blocks.
  - **Notice inventory:** ADR 0018's license check.
  - **Supply chain:** signed releases (ADR 0016), and the supply-chain gates of [Security and compliance model](https://github.com/Jbmanllr/automan/issues/73):
    - trusted publishing with provenance on crates.io, npm and PyPI
    - GitHub artifact attestations and immutable releases
    - a CycloneDX SBOM
    - `cargo-deny` and `cargo-audit` and their npm and Python equivalents
    - Actions pinned by SHA
    - OpenSSF Scorecard
  - **Docs:** the docs build, and every example in them runs.
- **Changelog and release notes.** One `CHANGELOG.md`, generated with git-cliff from Conventional Commits, which the product's commits follow from the first preview on. A hand-written page for every minor gives the highlights, the deprecations and their replacements, the browser builds, the contract-format versions and a link to that minor's benchmark results, completed with the competitor rows once they are published.
- **Documentation.**
  - **Tool and hosting:** a Docusaurus site, chosen for its built-in versioning. Starlight's versioning is still a community plugin, Mintlify is proprietary hosting whose llms.txt covers only the default version, and MkDocs Material has been in maintenance since November 2025.
  - **Kept versions:** stable (the latest minor), `next` from `main`, and a frozen copy of the last minor of each previous major still supported. Each item carries "Added in" and "Deprecated in" badges generated from the catalog, and a banner marks `next` and old majors.
  - **When docs deploy:** stable docs deploy only from release tags, because docs ahead of the release are a recurring complaint (browser-use #902, scrapy #5324).
  - **Reference pages:** generated from the Operation catalog and the settings schema, one page per operation and per setting, with TypeScript, Python, Rust, CLI and MCP tabs showing each real signature, as Playwright and Pulumi generate every language from one source. TypeDoc, a Griffe-based Python reference, docs.rs and `clap-markdown` give each language its native reference in addition.
  - **Tested examples:** every code sample runs in CI against the Proving ground, through Rust doctests, Twoslash, pytest-examples and scripted CLI and MCP samples. Links are checked with lychee.
  - **For agents:**
    - every page is also served as `.md`, with `llms.txt` and `llms-full.txt`
    - the docs of the installed version are embedded in the binary, read with `automan docs` and served as resources by the engine's MCP server, as Next.js ships its docs in its package
    - skills ship inside the packages and install with `automan skills install`
    - `automan init` writes a short AGENTS.md index

    No separate docs MCP server is run.
  - **Automan Cloud:** its docs stay apart from the framework's. The framework docs never present a Cloud-only feature as available, and pages on features both offer say where each runs.
- **Left to the specs.** Release automation (release-plz or another tool for a lockstep, multi-registry release) and where the docs source lives in the repository, an item of the repository layout in the [V1 map](https://github.com/Jbmanllr/automan/issues/61). The docs domain follows [Product name: keep Automan or rename](https://github.com/Jbmanllr/automan/issues/83).
