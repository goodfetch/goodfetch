# Each Product is an executable crate that links only its crates, and has its own Size budget

ADR 0084 decided that Goodfetch ships two Products, Collie and Foxhound, whose executable contains Collie's code, from one repository. This decides how they are built. Each Product is one executable crate of the one Cargo workspace, linking only the crates its capabilities need and built by its own `cargo build -p`. Every crate only Foxhound links lives under `crates/foxhound/`, and two cargo-deny checks prove nothing else depends on it. Foxhound adds to the engine through values that each executable crate composes, and a request for what Collie lacks fails with `not_in_product`, naming Foxhound. Each Product has its own Size budget. The research behind it ([multi-product builds](https://github.com/goodfetch/automan/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md), rechecked 2026-10-15 where cited) found:
- **In the closest precedents, a larger product that links a smaller one is built from separate packages, not from Cargo features.** `codex-cli` depends on the `codex-app-server` crate, which has its own `[[bin]]`; `uv` contains the build backend that ships alone as `uv_build`; `deno` and `denort` share `deno_lib`. Each smaller product has its own build invocation, since Cargo merges features across the packages of one invocation ([cargo#4463](https://github.com/rust-lang/cargo/issues/4463); the fix is still unstable, [cargo#14774](https://github.com/rust-lang/cargo/issues/14774)).
- **The other mechanisms drew the failures ADRs 0038 and 0057 predicted.** spider-rs's 129 Cargo features shipped broken combinations ([#349](https://github.com/spider-rs/spider/issues/349)), and Homebrew dropped build options that "could not be tested on our CI system" ([Homebrew 2.0.0](https://brew.sh/2019/02/02/homebrew-2.0.0/)). `deno compile`'s download of `denort` fails in CI ([deno#27988](https://github.com/denoland/deno/issues/27988)). Codex's V8 host, a companion executable, was left out of a package ([#38975](https://github.com/openai/codex/issues/38975)), shipped unnotarized ([#47521](https://github.com/openai/codex/issues/47521)) and skewed from its client ([#42111](https://github.com/openai/codex/issues/42111)) within three months.
- **A per-product dependency check is cheap and in production.** uv runs `cargo deny --manifest-path crates/uv-build/Cargo.toml --all-features check bans` against a `deny.toml` that bans `rustls`, `openssl`, `reqwest` and `schemars` ([check-lint.yml](https://github.com/astral-sh/uv/blob/main/.github/workflows/check-lint.yml), [deny.toml](https://github.com/astral-sh/uv/blob/main/crates/uv-build/deny.toml)). Codex bans `reqwest` except under listed direct parents, first-party crates among them, with cargo-deny's `wrappers` ([deny.toml](https://github.com/openai/codex/blob/main/codex-rs/deny.toml)).
- **A probe for this decision** (cargo-deny 0.20.2, Rust 1.93.0, a workspace of path crates, 2026-10-15): the check rooted at Collie's executable alone missed a shared crate that Collie does not link depending on a Foxhound crate, and a dev-dependency on one; a workspace-wide `wrappers` rule caught both and passed the clean graph.

Decided in [How the products are built, and each product's Size budget](https://github.com/goodfetch/automan/issues/454), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447), by the agent on the owner's delegation of 2026-10-14 (rounds 1 to 3 and an addendum on the ticket). What Collie carries is [Collie's exact capability set](https://github.com/goodfetch/automan/issues/455)'s; this ADR places its answers in the crate graph and sizes them.

## Decision

### One executable crate per Product

- **Each Product is one executable crate** in the one Cargo workspace, linking only the crates its capabilities need, so what a Product contains is its dependency graph.
- **Each is built by its own `cargo build -p` per target** (ADR 0084), with ADR 0057's one release profile.
- **No Cargo feature selects a Product,** and none switches a capability within one (ADR 0038, amended below).
- **This adds no companion executable and no download on first use.** The existing ones are unchanged: the sink processes (ADR 0026), the Linux sandbox helper and the Windows filter broker (ADR 0033), browsers, Plugins and the Clean-up rules' daily channel (ADRs 0024, 0033, 0047).
- **Each executable embeds only the data its own capabilities use:** its own Console build, its own offline docs and `init` starters, and the Clean-up rules, which both Products render pages with.

### Two sides of the crate graph

- **Every crate only Foxhound links lives under `crates/foxhound/`:**
  - Foxhound's executable crate;
  - the crates of what ADR 0084 and Collie's exact capability set make Foxhound's: the Frontier with its work-item leases, crawl scope, the Session pool, Politeness's per-domain admission and robots.txt gate, fetch tiers, Paid fetches, Datasets and Records with their change tracking, Exports, Sinks, DuckDB with search across runs and the SQL tab, and Partitioned crawls;
  - the sink processes, moved from `crates/sinks/` to `crates/foxhound/sinks/`.
- **Everything else stays under `crates/` and may be linked by Collie:** Collie's crates, the pieces both Products carry, `goodfetch-protocol` and the test-only crates. There is no third group, because the pieces both Products carry may depend on Collie's crates; the area 3 spec's engine already drives browsers through Collie's CDP driver.
- **Foxhound carries everything Collie carries by construction.** Collie's executable crate exposes its composition (below) as a library beside its `main`, and Foxhound's executable crate starts from it, as `codex-cli` links `codex-app-server`.
- **The published Rust crates through which each Product's SDK embeds its engine** (ADR 0024) sit on their Product's side. How many there are is [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/automan/issues/458)'s; their names, and every crate's and executable's, are [Product names, packages, executables and version line](https://github.com/goodfetch/automan/issues/457)'s.

### What CI checks

Every pull request runs the first three, each in seconds; the fourth runs where ADR 0057 puts it.

1. **The direction, workspace-wide.** No crate outside `crates/foxhound/`, other than `xtask`, depends on a crate inside it, as a normal, build or dev dependency. cargo-deny bans each Foxhound crate with `wrappers` listing Foxhound's own crates and `xtask` as its only allowed direct parents; `xtask codegen` writes the list from the folder, and CI fails when the committed list differs.
2. **Each Product's graph, rooted at its executable crate with `--all-features`** (uv's pattern, ADR 0084).
   - Collie's bans Foxhound's crates and the third-party crates only Foxhound's capabilities use, starting with DuckDB's crates, `wreq` and its BoringSSL, `x402-rs` and `alloy`, and the in-binary Sinks' database and object-store drivers. A third-party crate that only Foxhound's crates use joins the list in the pull request that adds it, and the fresh review checks it.
   - Each Product's bans the test-only crates (`goodfetch-test-support`, the Scripted engine).
3. **The Console's packages** (below): Collie's Console build fails to resolve an import of Foxhound's screens.
4. **Each Product's Size budget** (below), the backstop on the built files.

### How Foxhound adds to the engine

- **Each domain crate exports one value listing what it adds to the engine:** operation handlers, settings keys, run-state tables, Plugin kinds and Hook stages, Usage kinds and Limit scopes, Notification events, and its gates in admission.
- **Each Product's executable crate passes the values it links to the engine at start**, so the composition is a list in `main` and nothing registers itself by being linked. Deno builds its runtime the same way, from the extensions passed in `RuntimeOptions.extensions` ([jsruntime.rs](https://github.com/denoland/deno_core/blob/main/core/runtime/jsruntime.rs)), and `deno` and `denort` pass different lists. Foxhound's composition is Collie's plus Foxhound's values.
- **Code outside `crates/foxhound/` never names a Foxhound type.** Where Foxhound adds behaviour to the engine, the seam is an interface of the engine, such as the gate in admission that Foxhound's per-domain queues and Politeness fill. It is a real seam under `STYLE.md`'s two-adapter rule, because the two Products compose different sets.
- **Each executable crate tests that its composition has exactly one handler for each operation of its Product's catalog** (the catalogs are #458's). Tests and the Simulation (ADR 0060) compose the engine from the values they need.
- **The Console is one codebase with one build per Product.**
  - Foxhound's screens (Datasets with the SQL tab, Sinks, crawl progress, search across runs) are a workspace package of their own under `console/`, which Collie's Console build does not depend on.
  - pnpm's default layout links "only the direct dependencies" of a package ([pnpm](https://pnpm.io/motivation)), so Collie's build cannot import them, as Cargo refuses an undeclared crate.
  - Each Product's executable embeds its own Console build. Playwright builds its trace viewer and UI mode the same way, as entries of one package ([vite.config.ts](https://github.com/microsoft/playwright/blob/main/packages/trace-viewer/vite.config.ts)).

### A request for what the running Product lacks

Foxhound contains Collie, so only a Collie engine meets one: a Foxhound SDK or a raw Client on a Collie Daemon, a Recipe written for Foxhound, a `crawl.*` setting, `crawl` typed on Collie's CLI.
- **The Error code is `not_in_product`,** an `UnavailableError` (fix the setup, ADR 0079), not retryable. Its details name the running Product with its version and the Product that carries the request. Its `fix` names Foxhound and the install command for the channel the running Product came from, with #457's package names.
  - Example message: "`crawl.start` is part of Foxhound, and this engine is Collie 1.4.0. Fix: run it with Foxhound, which contains everything Collie does (`<install command>`), or connect to a Foxhound engine."
  - ripgrep ("PCRE2 is not available in this build of ripgrep"), Cargo ("no such command", then "help: find a package to install") and Playwright (the exact install command) name the fix the same way.
- **An explicit request is refused:** an operation; a CLI command, with the exit code of ADR 0069's table; a value in `settings`, a flag, a form or a Recipe; a Recipe step; enabling a Plugin kind or Hook stage the Product lacks.
- **A shared file is read without it.** User and project configuration and Operator policy may hold the other Product's keys, because one project or one Goodfetch home can serve both Products. A Product applies its own keys, and `config explain` lists the others as Foxhound's, so none is hidden. A key neither Product knows stays `setting_unknown` (ADR 0083).
- **How an engine knows.** `xtask codegen` writes, into a crate both Products link, the names of every operation, setting key, CLI command, Plugin kind and Hook stage that only Foxhound carries: names only, whatever shape #458 gives the catalogs. The settings schema (ADR 0028) records, for each key, the Products that honour it. On a Collie engine this check comes before ADR 0083's scope check, so `crawl.*` there answers `not_in_product`, not `setting_not_applicable`.

### Each Product's Size budget

| What is measured | Foxhound ceiling | Collie ceiling | Increase threshold per pull request |
|---|---|---|---|
| Compressed release file per platform: the npm platform package, the wheel and the GitHub Release archive | 60 MB | 50 MB | +1 MB |
| Raw executable per platform | 160 MB | 120 MB | — |
| Each published crate's tarball (`.crate`) | 8 MiB | 8 MiB | — |
| Each OCI image tag (ADR 0049) | none | none | +3 MB compressed |

- **Foxhound keeps ADR 0057's numbers.** ADR 0084 moved nothing out of what was the one binary.
- **Collie's ceilings come from its components,** measured on macOS arm64 with thin LTO, stripped, before the engine's own code (research §7, ADR 0057):
  - the CDP client, SQLite, HTTP with rustls, the HTTP stack that serves the Console, and the extractor, measured as one binary: 16.0 MB raw, 6.8 MB compressed;
  - the filter list engine of the Clean-up rules: +2.6 MB raw, +1.0 MB compressed;
  - the embedded Clean-up rules and Console: +4.1 MB, already compressed;
  - V8 for the Script sandbox, which Collie's exact capability set puts in Collie: +42.8 MB prebuilt, up to about +60 MB built from source with its sandbox (Codex's V8 host is 65.4–74.0 MB), +15 to 22 MB compressed;
  - so 65–83 MB raw and 27–34 MB compressed, or 73–93 MB and 30–38 MB with static musl's 12% more.
- **Neither Product counts the browser.** The Goodfetch Chromium stays outside every executable (ADR 0084), and its archive keeps the Chromium build gate's own 5 MB threshold (ADR 0074).
- **The headroom absorbs the engine's own code,** which no probe counted: 12–20 MB compressed and 27–47 MB raw over Collie's projection, as ADR 0057's 15–25 MB did for the full binary. The ceilings stop drift; a misplaced Foxhound dependency, such as DuckDB (+33.0 MB raw), is stopped earlier by Collie's ban list.
- **One increase threshold, +1 MB,** is about 3% of Collie's compressed file and 2–3% of Foxhound's, so a change to a crate both Products link trips both checks or neither.
- **ADR 0057's rules hold per Product:**
  - every target is measured, and the largest must fit;
  - the increase check compares each Product's build with `main`'s build of the same Product and profile;
  - the ceilings are checked on the release-profile files before any publish;
  - a pull request that trips a check passes with a "Size increase" section the fresh review accepts on its exact head;
  - a ceiling moves only by changing the ADR that sets it, with the owner's approval: ADR 0057 for Foxhound's, this ADR for Collie's.
- **ADR 0071's Console ceiling** (1 MB brotli) and increase check apply to each Product's Console build.
- Competitors at Collie's job: agent-browser is one 15.8–18.3 MB Rust binary, Playwright installs 18.3 MB, Stagehand v4's npm install is 33.0 MB, and Playwright's Python wheels are 33–46 MiB ([browser-automation products](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md)). The browser every Collie user downloads is 124–198 MB compressed (ADR 0057).

## Considered Options

- **Products as Cargo features of one crate** (`--features collie|foxhound`): rejected. Code would carry `cfg` branches for each Product, a combination matrix would need testing, and Cargo's feature unification would put Foxhound's dependencies into a Collie built in the same invocation. spider-rs and Homebrew show the failure, and ADR 0038 already rejected capabilities as features.
- **`cfg(feature = "foxhound")` inside the crates both Products link:** rejected for the same reasons, at a smaller scale.
- **Collie as the base, with Foxhound's parts downloaded on first use:** rejected. It brings ADR 0057's costs (a wait, offline failures, a signed and pinned artifact per part, proxies and mirrors) and Deno's download failures, and Foxhound would not work offline out of the box.
- **Foxhound's parts as companion executables beside Collie's:** rejected. Codex's V8 host shows the costs, and ADR 0084 already refused a process boundary between Collie and Foxhound for fencing and the Effect journal. **As process Plugins** (ADR 0033): rejected for the same boundary.
- **Three groups of crates** (shared, Collie's, Foxhound's): rejected. It adds a direction ADR 0084 does not ask for, keeping shared crates off Collie's, which the engine's use of Collie's CDP driver would break.
- **A crate-name prefix or a manifest metadata mark as the boundary:** rejected. Names are #457's and a rule on names breaks when they change; a metadata mark is invisible in the tree.
- **ADR 0084's per-Product check alone:** rejected. The probe shows it misses a shared crate that Collie does not link and dev-dependencies.
- **An allowlist of every crate Collie may use,** as rustc's `tidy` keeps 220: rejected. Every new shared dependency would need a list edit, where the bans already cover what matters.
- **A symbol check on each release binary,** as uv's trampolines run `cargo bloat`: rejected. It needs an unstripped release build per Product and matches mangled names, while the crate graph is exact for Rust code.
- **Self-registration at link time** (`inventory`, `linkme`): rejected. A Product's content would be whatever the linker kept, through a global registry, against ADR 0059's values.
- **One Console build in both executables, hiding Foxhound's screens on Collie:** rejected. Collie would ship Foxhound's code in its Console, and the boundary would hold for Rust only.
- **What a Collie engine says without this decision** (JSON-RPC's "method not found", `setting_unknown`, the CLI's usage error): rejected. It tells the user their program is wrong when the fix is the other Product.
- **Ignoring the other Product's settings, or warning and continuing:** rejected by ADR 0083 (G3) for explicit values. **Refusing configuration files that hold them:** rejected; installing Foxhound beside Collie would break Collie on a file both read.
- **Collie built with `opt-level = "z"`,** as uv builds `uv_build`: rejected for performance. ADR 0057 measured `"s"` and `"z"` 18% and 90% slower on its query, LLRT measured 7% for `"s"`, and Collie's hot paths (CDP messages, the guard, extraction) are what ADR 0071 counts. It would save a quarter of Collie's compressed file, a few MB beside the browser.
- **One set of ceilings for both Products:** rejected. Collie's would never bind.
- **Collie's ceilings set only after its first measured build:** rejected. Collie would have no hard stop until then, the gap ADR 0057 closed with numbers up front.

## Consequences

- **Amends ADR 0038.** Within a Product a capability is a setting, never a Cargo feature, and the Products differ only by the crates their executables link. Each Product's executable builds one fixed feature set in its own `cargo build -p`; features remain only on the published crates through which each Product's SDK embeds its engine, for heavy optional dependencies and test utilities, additive, with `cargo-hack` over each crate's combinations. Notes sit under its capabilities item and its rejected option "Capabilities as Cargo features".
- **Amends ADR 0057.** Each capability is in the executable of every Product that carries it, and the Size budget, its tiers and its rules apply per Product, with Collie's ceilings set here. Notes sit under its "What stays in the binary" item and under its table.
- **Amends ADR 0032.** The tree gains `crates/foxhound/`, with the sink processes in `crates/foxhound/sinks/`, and the pull-request tier runs the boundary checks. Notes sit under the tree and under the pull-request tier's dependency checks.
- **Amends ADR 0071.** Its Console ceiling and increase check apply to each Product's Console build. A note sits under the Console ceiling.
- **Amends ADR 0083.** The product check comes before the scope check, so a `crawl.*` setting on a Collie engine is `not_in_product`; a configuration file may hold the other Product's keys, which `config explain` lists; and every build of a Product has the same types. Notes sit under its refusal table and the paragraph after it.
- **Error codes.** `not_in_product` joins `goodfetch-protocol` in the `unavailable` group, not retryable, with an exit code in ADR 0069's table. The settings schema gains, per key, the Products that honour it.
- **`STYLE.md`.** Members live under `crates/`, `crates/foxhound/` and `crates/foxhound/sinks/`; the enforced direction reads "nothing outside `crates/foxhound/` depends on a crate inside it"; each Product's executable crate is a composition root; the Cargo features section is per Product; cargo-deny runs workspace-wide and per Product.
- **`AGENTS.md`.** The trap reads: a capability is a setting, never a Cargo feature, and nothing outside `crates/foxhound/` depends on a crate inside it.
- **Glossary.** *Size budget* is per Product. No new term: the values each executable composes are code, not domain language.
- **`TESTING.md`.** Collie's executable is tested against the generated table of Foxhound's names; each executable crate tests its composition's handlers; Foxhound's tests stay in Foxhound's crates.
- **ADR 0037's matrix.** SDK-03 cites this ADR. Areas 1, 3, 11 and 12 list it.
- **Areas.**
  - Area 1 (Repository, CI and release) builds the two checks, the per-Product builds and Size budgets, and `crates/foxhound/` in the workspace skeleton.
  - Area 3 (Engine runtime) builds the composition, the admission seam and `not_in_product`, and moves the modules that are Foxhound's out of `goodfetch-engine` (the per-domain queues and Politeness of `admission` and `politeness`, and `partition`). A piece that also serves one page, such as the proxy limits of the Politeness sharing layer that [Crawl politeness defaults](https://github.com/goodfetch/automan/issues/480) applies to a program's own navigation, stays outside `crates/foxhound/`.
  - Area 11 (Client surfaces) builds the CLI's refusal of Foxhound's commands.
  - Area 12 (Console) builds the per-Product Console builds.
- **Left to other tickets.** Release jobs, signing, debug symbols, the PyPI quota per project and the test suites per Product are [Release, signing and testing of two products](https://github.com/goodfetch/automan/issues/459)'s. What a Client of one Product does on an engine of the other at `hello` is #458's. Which executable a Browser host and Goodfetch Cloud's engines run, and which Console build Goodfetch Cloud serves, are [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/automan/issues/498)'s. [Apply the product-line decisions to the affected specs and tickets](https://github.com/goodfetch/automan/issues/460) carries all of this into the specs and tickets.
