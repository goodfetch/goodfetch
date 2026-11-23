# The Products are named `goodfetch-collie` and `goodfetch-foxhound` in namespaces open to everyone, and carry one Product version

ADR 0084 made Collie and Foxhound two Products under Goodfetch, the organization and umbrella brand, and renamed per Product every identifier that names installed software. This decision sets those identifiers and the version line. What a person installs, runs or imports to get a Product carries its name: `goodfetch-collie` and `goodfetch-foxhound` wherever the namespace is open to everyone, the bare Product name under the namespaces Goodfetch owns. What both Products share keeps `goodfetch`. Both Products, and the packages people install, carry one version and are released together.

The [naming, packaging and versioning research](https://github.com/goodfetch/goodfetch/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) found, on 2026-10-14:
- **The bare names are taken where users look for them.** npm `collie` and `foxhound`, PyPI `collie` and `foxhound` and crates.io `collie` belong to other projects. ColliePWA (1,338 stars, a client for AI coding agents) puts `collie` on PATH, with `collie skill` and `collie docs` commands, for the developers Goodfetch addresses. SAP's Playwright-drivable Foxhound browser ships `foxhound-<version>` archives, and a Go scraping framework builds a `foxhound` executable.
- **Both names are registered marks for software:** COLLIE in the EU (018890716) and the US (8190894), FOXHOUND in France (4860964, class 42).
- **Every `goodfetch-*` and `@goodfetch/*` name is free.** On 2026-10-15, PyPI and crates.io still return 404 for `goodfetch-collie` and `goodfetch-foxhound`, and npm still reports the `@goodfetch` scope as "Scope not found".
- **A package that ships another's native engine carries that engine's exact version.** Biome, esbuild, Turborepo and Playwright's Python driver pin theirs exactly. Products that only share source code keep separate lines (Ruff and ty, oxlint and oxfmt), and lines that meet over a separately versioned contract bring a compatibility table or per-version tags.

Decided in [Product names, packages, executables and version line](https://github.com/goodfetch/goodfetch/issues/457), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/goodfetch/issues/447). The agent decided it on the owner's delegation ([round 1](https://github.com/goodfetch/goodfetch/issues/457#issuecomment-6179781532)). The reviews of [#503](https://github.com/goodfetch/goodfetch/pull/503) refined it and added the robots.txt token and the crate names. The owner then confirmed every decision ([confirmation](https://github.com/goodfetch/goodfetch/issues/457#issuecomment-6187731750)). The [multi-product builds research](https://github.com/goodfetch/goodfetch/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) also found that `uv` contains and also ships `uv_build`; PyPI lists both at 0.13.1 (2026-10-15).

## Decision

### Which names carry a Product

- **What a person installs, runs or imports to get a Product carries that Product's name; what both Products share stays `goodfetch`.**
- **Qualified wherever the namespace is first come, first served:** PATH, PyPI, crates.io, Homebrew formula names, Python and Rust import names, agent skill names and MCP server names take `goodfetch-collie` and `goodfetch-foxhound` (`goodfetch_collie` and `goodfetch_foxhound` where a hyphen is invalid).
  - One string per Product on every such channel, so `uvx goodfetch-collie` works as written: uvx runs the command named like the package unless told otherwise with `--from` ([uv tools guide](https://docs.astral.sh/uv/guides/tools/)).
  - The house mark travels with every use, beside the COLLIE and FOXHOUND marks that others hold. If clearance fails for a Product's name (ADR 0084), only the word after `goodfetch-` changes.
- **Bare under the namespaces Goodfetch owns or claims for itself:** the npm scope (`@goodfetch/collie`, claimed by the owner's free npm organization, below), the GHCR organization (`ghcr.io/goodfetch/collie`) and paths on `goodfetch.dev` (ADR 0034). Only Goodfetch can publish there, so the prefix already says who published.
- **`goodfetch` for everything both Products share:**
  - the GitHub organization, the npm scope, `goodfetch.dev`, the `ghcr.io/goodfetch/` prefix (ADR 0084) and the Homebrew tap `goodfetch/tap`;
  - the `GOODFETCH_` prefix, reserved by ADR 0034 and enforced by ADR 0083, and ADR 0083's deployment variables `GOODFETCH_HOME`, `GOODFETCH_BROWSERS_PATH`, `GOODFETCH_URL`, `GOODFETCH_TOKEN` and `GOODFETCH_TEST_SEED`;
  - Goodfetch home, a folder named `goodfetch` in each operating system's usual folders, as ADR 0031's `~/.local/share/goodfetch/`; what it holds per Product is [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486)'s (ADR 0098);
  - the internal crates, `goodfetch-<glossary term>` (`STYLE.md`), published at `0.0.x` (ADRs 0030 and 0032). A crate's name carries a Product's name only when it is that Product's executable crate or a crate the Product publishes for its users (below);
  - Python loggers under `goodfetch` and Rust `tracing` targets under `goodfetch::` (ADR 0078), and metrics under `goodfetch.*` (ADR 0050);
  - the sink system columns `_goodfetch_*` (ADR 0026), which stay the same whichever Product writes a row;
  - the helpers either Product may install for its own use: the Linux `goodfetch-sandbox` helper and the Windows filter broker (ADR 0033). ADR 0084's "never anything installed" covers what a person installs to get a Product, not the components both Products share.
- **One exception: the engine-path variable names its Product.** `GOODFETCH_COLLIE_ENGINE_PATH` and `GOODFETCH_FOXHOUND_ENGINE_PATH` replace `GOODFETCH_ENGINE_PATH`, since one path cannot be right for both Products' SDKs in one shell. Biome, esbuild and Turborepo likewise each name their own override (`BIOME_BINARY`, `ESBUILD_BINARY_PATH`, `TURBO_BINARY_PATH`).
  - Every SDK, crate and CLI of either Product recognizes both, on ADR 0083's closed list, so the other Product's variable is never an unknown name that fails `connect()`.
  - Each reads only its own Product's: the TypeScript and Python SDKs for their Attached engine, the Rust crate for the Script sandbox and `plugin-shim` (ADR 0024). The other Product's is recognized and not read, as `GOODFETCH_TEST_SEED` is (ADR 0083).
- **Foxhound's robots.txt product token follows the rule for namespaces open to everyone: `goodfetch-foxhound`,** replacing ADR 0086's bare `Foxhound`. Any crawler may claim a product token, and a Go scraping framework named Foxhound does Foxhound's job, so a bare token would share robots.txt groups with it. RFC 9309 allows letters, underscores and hyphens in a token and matches it case-insensitively ([RFC 9309 §2.2.1](https://www.rfc-editor.org/rfc/rfc9309.html#section-2.2.1)). It still names the crawler, never the organization, as ADR 0086 requires.

> Note (coordinator's decision, not read by the owner), 2026-11-22, [#3420](https://github.com/goodfetch/goodfetch/issues/3420): a crate's compile-fail tests crate, `<crate>-compile-fail` (`publish = false`, test-only), takes the name of the crate it tests, a Product's included, though it is neither a Product's executable crate nor a crate a Product publishes: `goodfetch-foxhound-compile-fail` holds `goodfetch-foxhound`'s. It is a test artifact named for the crate it tests, so a person never installs, runs or imports it. `STYLE.md` lists it as an exception to the internal crates' naming and as a fifth reason for a crate of its own: trybuild's generated project copies the dependencies and dev-dependencies of the crate that holds the tests, and a crate of their own builds the tested crate's library graph alone ([#3413](https://github.com/goodfetch/goodfetch/issues/3413)).

### Executables and commands

- **The executables are `goodfetch-collie` and `goodfetch-foxhound`** (`goodfetch-collie.exe` and `goodfetch-foxhound.exe` on Windows), on every channel.
- **A command is `<executable> <command> …`, with no Product group.** ADR 0034's `goodfetch collie …` and `goodfetch foxhound …` groups go.
- **A command both executables carry keeps one name, its flags, its `--json` output and its exit codes in both**, so moving a script from one Product to the other changes only the executable's name. What each executable carries is [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md)'s, and which commands each offers is decided in [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458).
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): each executable offers the commands of its Product's catalog, and Foxhound's carries every command of Collie's unchanged.
- **Where a text names a command without a Product,** it writes the command alone (`upgrade-files`, `browsers install`), or one executable as an example.

### Packages

| Channel | Collie | Foxhound |
|---|---|---|
| npm | `@goodfetch/collie` | `@goodfetch/foxhound` |
| npm platform packages | `@goodfetch/collie-<platform>-<arch>` | `@goodfetch/foxhound-<platform>-<arch>` |
| PyPI | `goodfetch-collie` | `goodfetch-foxhound` |
| crates.io | `goodfetch-collie` | `goodfetch-foxhound` |
| Homebrew | `goodfetch/tap/goodfetch-collie` | `goodfetch/tap/goodfetch-foxhound` |
| OCI image | `ghcr.io/goodfetch/collie` | `ghcr.io/goodfetch/foxhound` |
| Installers | `https://goodfetch.dev/install/collie.sh` and `.ps1` | `https://goodfetch.dev/install/foxhound.sh` and `.ps1` |

> Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): each language also has a Collie client package with no executable, `@goodfetch/collie-client` on npm and `goodfetch-collie-client` on PyPI (a pure `py3-none-any` wheel) and crates.io, on which both Products' SDK packages build at exactly their own version.

- **npm.** Each package is its Product's TypeScript SDK and a `bin` named after its executable, so `npx @goodfetch/collie` runs `goodfetch-collie`: `npm exec` runs a package's single `bin` entry ([npm docs](https://docs.npmjs.com/cli/v11/commands/npm-exec)). The platform packages use Node's `process.platform` and `process.arch` names (`darwin-arm64`, `darwin-x64`, `linux-arm64`, `linux-x64`, `win32-arm64` and `win32-x64`, one static musl build per Linux architecture, ADR 0024), as `@esbuild/*` and `@biomejs/cli-*` do. They are pinned exactly in `optionalDependencies`, with no install script (ADR 0024).
- **PyPI.** `py3-none` platform wheels (ADR 0024), each holding its Product's Python SDK and executable.
- **crates.io and the workspace.** `goodfetch-collie` is the crate Collie publishes as its Rust SDK, imported as `goodfetch_collie`, its engine linked (ADR 0024); Foxhound's is `goodfetch-foxhound`.
  - Its executable crate (ADR 0087), when that is not the published crate, and any further crate it publishes for its users are `goodfetch-<product>-<part>`.
  - The executable is `goodfetch-collie` or `goodfetch-foxhound` whatever its crate is named.
  - Every other crate, Foxhound's crates under `crates/foxhound/` included, keeps `goodfetch-<glossary term>`, as ADR 0087 rejects names as the boundary.
  - How many crates a Product publishes is #458's, and how ADR 0024's `cargo install` channel installs the executable is #459's.
    > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): Collie publishes two crates, `goodfetch-collie-client` and `goodfetch-collie`, and Foxhound one, `goodfetch-foxhound`.
- **Homebrew.** The tap's repository is `goodfetch/homebrew-tap`, which `brew tap goodfetch/tap` clones. A formula is named after the executable it installs. `brew install` resolves a short name to `homebrew/core` first ([Homebrew docs](https://docs.brew.sh/Taps)), so a tap formula named `collie` would lose to any later core `collie`, while `goodfetch-collie` keeps one name for the formula and the executable.
- **OCI.** ADR 0049's tags (`<v>`, `-slim`, `-full`, `-node`, `-python`) apply under each Product's image. Which tags each Product publishes is decided in [Release, signing and testing of two products](https://github.com/goodfetch/goodfetch/issues/459) (ADR 0094), and what the Browser host runs in [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/goodfetch/issues/498) (ADR 0096). A Helm chart is named after the Product whose image it runs, under `ghcr.io/goodfetch/charts/`.
- **Releases.** One tag, `vX.Y.Z`, and one GitHub Release per Product version carry both Products' files, each file named after its executable.
- **Adapters and Integrations** that a registry versions (ADRs 0011 and 0027) are named after the Product they serve: `@goodfetch/<product>-<target>` on npm and `goodfetch-<product>-<target>` on PyPI, unless the target's ecosystem requires a form. n8n requires a name that "starts with `n8n-nodes-` or `@<scope>/n8n-nodes-`" ([n8n docs](https://docs.n8n.io/integrations/community-nodes/build-community-nodes/)), so its node is `@goodfetch/n8n-nodes-<product>`. Which Product each serves is #458's.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): Framework adapters and workflow-tool Integrations exist once per Product; the Airbyte source and the Dataset loaders are Foxhound's only.
- **Agent skills and MCP servers.** A Product's skill, the server name its docs and `init`'s templates give its MCP server in an MCP host's configuration, and the name that server announces are `goodfetch-collie` or `goodfetch-foxhound`; a skill on one topic is `goodfetch-collie-<topic>`. Agent Skills names allow lowercase letters, digits and single hyphens, up to 64 characters ([specification](https://agentskills.io/specification)).
- **No meta-package.** `goodfetch` is published on no registry. Foxhound contains Collie, so a package installing both would install two engines for nothing and give a second way to get Foxhound, and Goodfetch would name something installed (ADR 0084).

### SDK names

| | TypeScript | Python | Rust |
|---|---|---|---|
| Collie | `import { Collie } from "@goodfetch/collie"` | `from goodfetch_collie import Collie, AsyncCollie` | `goodfetch_collie::Collie` |
| Foxhound | `import { Foxhound } from "@goodfetch/foxhound"` | `from goodfetch_foxhound import Foxhound, AsyncFoxhound` | `goodfetch_foxhound::Foxhound` |
| Collie's testing kit (ADR 0069) | `@goodfetch/collie/testing` | `goodfetch_collie.testing` | `goodfetch_collie::testing`, behind `test-util` |

- **The connection is named after its Product,** for ADR 0078's reason (`OpenAI`, `Stagehand`; S3): TypeScript `await Collie.connect()`, Python `AsyncCollie.connect(…)` and the sync `Collie.connect(…)`, Rust `Collie::connect(options).await`, `Collie::scope(…)` and `goodfetch_collie::blocking`; Foxhound's likewise, its testing kit under `@goodfetch/foxhound/testing`, `goodfetch_foxhound.testing` and `goodfetch_foxhound::testing`. They replace ADR 0078's `Goodfetch`, `AsyncGoodfetch` and `Goodfetch::connect`, and `gf.collie` and `gf.foxhound` name nothing now. What hangs from each connection, and whether a Foxhound connection reaches Collie's operations, is #458's.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): a connection has no `collie` or `foxhound` property, and a Foxhound connection carries every member of a Collie connection under the same names, plus Foxhound's own.
- **Python's import name is the PyPI package's name,** with `-` read as `_`, as `pydantic-ai` is imported as `pydantic_ai` and `langchain-openai` as `langchain_openai`.

### How prose writes the names

- **"Goodfetch Collie" and "Goodfetch Foxhound" at first mention** in anything read outside the project (titles, READMEs, registry descriptions, each docs page, release notes), then "Collie" and "Foxhound". Crawlee is presented the same way: crawlee.dev credits Apify beside Crawlee's own name ([naming research](https://github.com/goodfetch/goodfetch/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) §5).
- **"Goodfetch" alone never names a Product.**
- **ADRs, `GLOSSARY.md` and code** use the bare names. Counsel's clearance (ADRs 0034 and 0084) decides any filing.

### One Product version

- **Collie, Foxhound and the packages people install carry one version, the Product version,** and are released together from one commit under ADR 0030's calendar, channels, support policy and gates. The internal crates stay at `0.0.x` (ADRs 0030 and 0032).
- **Every release publishes both Products**, even when one has no change of its own, as Changesets' `fixed` groups publish "even when there are no changes" ([naming research](https://github.com/goodfetch/goodfetch/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) §4). A Collie change always reaches Foxhound, which contains it, so only a Foxhound-only change gives Collie a release without changes.
- **Foxhound X.Y.Z contains Collie X.Y.Z.**
- **Compatibility across Products uses the rules already in place.** A Client and an engine of either Product compare protocol versions in `hello` under ADR 0024's rule, as before; contract formats keep ADR 0030's majors, which change only with a Product major.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): before the version rule, an engine checks the Product the Client needs and refuses one its own Product does not contain with `not_in_product`.
- **One version does not tie installs together.** Nothing makes a project install both Products at the same version. Whether a Client of one Product may reach the other's engine, and what else `hello` carries, is #458's; what installs of different versions share is #486's.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): an engine accepts a Client whose needed Product its own contains, so a Foxhound engine accepts both Products' Clients and a Collie engine refuses Foxhound's at `hello` with `not_in_product`. A program that uses both Products' SDKs uses them at one Product version, since each pins the Collie client package exactly; the Products' executables and their installs still need not match.

### Both Products installed

- **Each Product's npm package, wheel, Homebrew formula and image carries only its own executable,** and none of them depends on one that carries the other Product's. How Foxhound's SDK reuses Collie's SDK code is #458's; how its crates use Collie's is ADR 0087's.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): Foxhound's SDK reuses Collie's through the Collie client package, which carries no executable: Foxhound's npm and PyPI packages depend on no Collie package but the client package, and Collie's on no Foxhound package.
- **Installing both collides nowhere.** The executables, packages, import names, skill and MCP server names and images all differ.
- **Collie's engine code is installed twice:** ADR 0087 projects Collie's executable at 65–83 MB raw and 27–33 MB compressed before the engine's own code, V8 included, under ceilings of 120 MB and 50 MB. Installing Ruff and ty, or oxlint and oxfmt, installs their shared code twice too.
- **Two SDKs in one program share no state.** ADR 0078 keeps nothing process-wide, so neither SDK reads a global the other could also hold, unlike two copies of Crawlee's core, which kept two global configurations. Whether one SDK accepts the other's values is #458's; what the installs share at run time (the browser cache, Goodfetch home, a Daemon) is #486's.
  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): one SDK accepts the other's values: both take them from the Collie client package, so a Foxhound handler's page and a Collie program's page are one type.

### The repository, and the names to claim

- **The repository becomes `goodfetch/goodfetch`.** It holds both Products, and Automan is the glossary's _Avoid_ term for Goodfetch. The rename comes before any trusted publisher is configured, since trusted publishers are configured per repository (ADR 0032). GitHub redirects web and Git traffic to the new name, except calls to actions the repository hosts, and only while no new repository takes the old name ([GitHub docs](https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository)). Links inside the repository are replaced in the pull request that follows the rename.
- **The agent registers, reserves and pays for nothing** (ADR 0076). Steps for the owner:
  1. Create the npm organization `goodfetch` on its free plan ("Unlimited public packages"). "Your organization name will also be your organization scope" ([npm docs](https://docs.npmjs.com/creating-an-organization)), so it holds every npm name above.
  2. Rename the repository to `goodfetch/goodfetch` before configuring trusted publishers, and never create another repository named `automan` in the organization, which would end the redirects.
  3. The PyPI and crates.io names are claimed by their first real upload, a crates.io crate's first one by hand (ADR 0032). A PyPI pending publisher "does not create a project or reserve a project's name until it is actually used to publish" ([PyPI docs](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)), and a placeholder is squatting under both registries' policies. Until then the names stay unreserved, a risk accepted.
- **This decision changes nothing about `goodfetch.dev`** or the trademark clearance and filings of ADRs 0034 and 0084.

## Considered Options

- **Bare `collie` and `foxhound` executables, from qualified packages:** rejected. ColliePWA puts `collie` on PATH for the same audience with the same `skill` and `docs` commands, so whichever comes first on PATH silently wins, and an agent skill that runs `collie` may reach the wrong program. `foxhound` meets the Go scraper's executable and SAP's Foxhound browser builds. `uvx goodfetch-collie` would also need `--from`.
- **Bare names where free, qualified elsewhere** (crates.io `foxhound`): rejected. Two spellings per Product, with nothing to tell users which channel uses which.
- **Coined short names** (`gfc`, `gff`, `gf-collie`): rejected. They carry no name a reader recognizes, and a short name beside the full one doubles every install line and doc.
- **A `goodfetch` launcher that dispatches to the installed Products** (`goodfetch collie …`, as `git` and `cargo` find subcommands): rejected. It names something installed after Goodfetch (ADR 0084) and adds an executable to sign, version and find on PATH.
- **Every name per Product, `COLLIE_*` variables and two homes included:** rejected. Every deployment variable would double, sharing that #486 may choose would mean renaming, and a bare `COLLIE_` prefix sits beside other Collie tools.
- **Every name kept `goodfetch`, the executables included:** rejected by ADR 0084, whose owner-confirmed decision renames what is installed.
- **Keeping ADR 0086's bare `Foxhound` robots.txt token:** rejected. Its groups would also catch rules written for the Go Foxhound crawler, and the token would be the one Product name used bare in a namespace open to everyone.
- **Product names on every crate a Product alone links** (`goodfetch-foxhound-frontier`): rejected. ADR 0087 draws the boundary by folder and checks, not by names, and a glossary term already says what a crate holds.
- **One `GOODFETCH_ENGINE_PATH` for both Products:** rejected. A path exported for one Product would silently redirect the other Product's SDKs in the same shell.
- **Unscoped npm names** (`goodfetch-collie`): rejected. An unscoped name does not show who published it, while only the organization can publish under `@goodfetch/`, so the scope proves where a package comes from.
- **`ghcr.io/goodfetch/goodfetch-collie`:** rejected. The organization prefix already qualifies the image.
- **A Python namespace package, `goodfetch.collie`,** as `google-cloud-storage` installs `google.cloud.storage`: rejected. Goodfetch publishes no `goodfetch` package on PyPI, so anyone may publish one, and if it ships a `goodfetch/__init__.py`, "the other sub-packages will not be importable" ([PyPA guide](https://packaging.python.org/en/latest/guides/packaging-namespace-packages/)).
- **`collie` as the Python import name:** rejected. ShopRunner's PyPI `collie` installs the same import name.
- **`Goodfetch` as the connection type in both packages:** rejected. A program using both would hold two unrelated `Goodfetch` types, and Goodfetch would name code. **`CollieClient`:** rejected, as ADR 0078 rejected `Client`.
- **A `goodfetch` meta-package installing both Products,** as `crawlee` pins its 14 `@crawlee/*` parts: rejected. Those parts make one product; Foxhound already contains Collie, so both would install two engines for nothing. **`goodfetch` as Foxhound's alias:** rejected by ADR 0084, which rejected Goodfetch as the full Product's name.
- **Placeholder packages for the bare `goodfetch` names,** as ADR 0034 planned for PyPI and crates.io: rejected. PEP 541 calls a package with no functionality name squatting, and crates.io's usage policy forbids crates that only reserve a name ([naming research](https://github.com/goodfetch/goodfetch/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) §2.2). Someone else may publish a `goodfetch` package; every install line, README and skill gives the qualified name.
- **Always "Goodfetch Collie" in prose:** rejected, heavy in long pages. **Always bare "Collie":** rejected. It drops the house mark beside the COLLIE and FOXHOUND marks that others hold, and searches for "Collie" or "Foxhound" alone find ColliePWA, SAP Foxhound and the Go Foxhound.
- **Independent version lines,** as Ruff and ty keep: rejected. ty does not contain Ruff, while Foxhound contains Collie and the two may share state on one machine (#486). ADR 0030 ties every contract format's major, the per-major Daemon and the per-major state to the product major, which two lines would split, and users would need a table of which Collie each Foxhound contains, as Stagehand v4 gates its separately versioned SDKs on `protocolVersion`.
- **A shared major with independent minors and patches:** rejected. Both Products get a minor every four weeks and a patch at every Chrome major (ADR 0030), so their lines would rarely differ, yet every compatibility statement would need a table.
- **Skipping a release of Collie when it has no change:** rejected. Collie's line would have gaps, and "Foxhound X.Y.Z contains Collie X.Y.Z" would no longer name a published Collie.
- **Keeping the repository name `automan`:** rejected. It is the glossary's _Avoid_ term for Goodfetch. **`goodfetch/foxhound`,** as Ruff's repository holds ty: rejected. It would file Collie's issues under Foxhound's name.

## Consequences

- **Amends ADR 0034.** Notes sit under its opening note, its identifiers, its rule that sub-names never stand alone, and its reservation.
- **Amends ADR 0030.** One Product version covers both Products. A note sits under its opening.
- **Amends ADR 0078.** The connection types are `Collie` and `Foxhound`. A note sits under its entry point.
- **Amends ADR 0024.** Each Product's SDKs read their own engine-path variable. A note sits under its packaging consequence.
- **Amends ADR 0032.** The tree's crates, executables and packages are named per Product, and the repository is `goodfetch/goodfetch`. Notes sit under the tree and its visibility item.
- **Amends ADR 0084.** Its "never anything installed" covers what a person installs to get a Product; components both Products share keep `goodfetch` names. A note sits under that item, and one under the item that left the strings to this decision.
- **Amends ADR 0086.** The robots.txt product token is `goodfetch-foxhound`. A note sits under its token item, and ADR 0021's note on the token gets one too.
- **Amends ADR 0087.** The published crates, executable crates and executables are named here. A note sits under its item on the published Rust crates.
- **Amends ADR 0049.** Each Product has its image and charts. Notes sit under the image table and the Helm chart's publishing item.
- **Amends ADR 0083.** Its deployment variables keep their names, except the engine-path variable, now one per Product and both on its closed list, and one Product version covers both Products' SDKs. Notes sit under its closed list and its strict reading.
- **Earlier ADRs.** As ADR 0084 reads "the `goodfetch` binary", an ADR written before this one reads:
  - `goodfetch <command>` as that command of each Product's executable that carries it;
  - the `goodfetch` crate, npm package, PyPI package, image and testing kit as each Product's;
  - `GOODFETCH_ENGINE_PATH` as each Product's engine-path variable;
  - "product version" and "product major" as the Product version and its major;
  - `Goodfetch`, `AsyncGoodfetch` and `Goodfetch::…` as each Product's connection type, and `gf.collie.…` and `gf.foxhound.…` as calls through the connection of the Product that carries them, as #458 shapes it.
- **Glossary.** **Product version** is added, and Goodfetch's entry says it names no Product and nothing a person installs to get one.
- **`STYLE.md` and `TESTING.md`** name the Products' crates, executables, testing kits, engine-path variables and robots.txt token. `STYLE.md` gives a crate a Product's name only when it is that Product's executable crate or a crate it publishes for its users, and `TESTING.md`'s settings cases check that an SDK recognizes the other Product's engine-path variable and does not read it. How each Product's executable is tested stays [Release, signing and testing of two products](https://github.com/goodfetch/goodfetch/issues/459)'s (ADR 0094).
- **ADR 0037.** Matrix rows RT-22, SDK-01, SDK-02, SDK-04, SDK-06, SDK-08 and SC-01 and pain row 21 cite this ADR; areas 1, 5, 11 and 13 list it.
- **Areas.** Area 1 (Repository, CI and release) builds the package names, the tap, the installers, one tag and release per Product version and the release of both Products together; area 11 (Client surfaces) builds the SDK names, each executable's command shape, and the skill and MCP server names; area 5 (Site access) builds the `goodfetch-foxhound` token's matching; area 13 (Self-hosted deployment and operations) builds the images' and charts' names.
- **Downstream.** [Apply the product-line decisions to the affected specs and tickets](https://github.com/goodfetch/goodfetch/issues/460) carries these names into the specs and tickets, and the SDK sketch of [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461) replaces its `goodfetch` placeholders once #458 settles each Product's SDK structure.
