# Contributing to Goodfetch

Thank you for helping. This guide covers what every change needs; [`STYLE.md`](STYLE.md) says how code is shaped and [`TESTING.md`](TESTING.md) how it is tested.

## Sign off every commit (DCO)

Goodfetch takes contributions under the [Developer Certificate of Origin](https://developercertificate.org/), with no CLA (ADR 0018). Sign off each commit with your real name:

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line, which certifies that you wrote the change or have the right to submit it under the repository's licence, `MIT OR Apache-2.0`. The sign-off must name the commit's author. A pull request with an unsigned commit cannot merge: the supply-chain job checks every commit it adds but merges (`just dco` checks yours since `origin/main`).

## Write Conventional Commits

Each commit's subject is a [Conventional Commit](https://www.conventionalcommits.org/en/v1.0.0/), `<type>[(<scope>)][!]: <description>`, so that `xtask release` can write `CHANGELOG.md` from them with git-cliff ([`cliff.toml`](cliff.toml)):

```text
fix(xtask): keep each figure's latest value in the drift check
feat(collie)!: rename the session's timeout setting
```

- The type is one of `feat`, `fix`, `perf`, `docs`, `refactor` and `revert`, which the changelog lists, or `build`, `chore`, `ci`, `style` and `test`, which it leaves out.
- The scope, when there is one, is lowercase: the crate, package or area the commit changes, such as `xtask`, `protocol`, `ci` or `adr`.
- A `!` before the colon, or a `BREAKING CHANGE:` footer, marks a breaking change.
- The description says what the commit does, in the imperative.

The supply-chain job checks every commit a pull request adds but merges (`just commit-messages` checks yours since `origin/main`, and the pre-push hook of `just hooks` runs it on every push). The rule holds from the owner's decision of 2026-10-19: commits up to its recorded starting point, `9e62a6c5` (`conventional_commits_since` in the `justfile`), keep their messages, since history is never rewritten.

A commit after the starting point whose message cannot be reworded without rewriting history that other branches hold is listed in `ci/commit-messages.toml`, by its full hash with a one-line `reason`, and is not judged (agent decision, pending owner review). Each entry names one commit: the file takes no patterns and adds no type. An entry without a reason or with a short hash is an error; an entry whose commit is outside the checked range is not.

## One entry point: `just`

Install [`just`](https://just.systems), Rust through `rustup` (the toolchain is pinned in `rust-toolchain.toml`), Node 24 with pnpm, and uv. CI calls the same recipes:

| Command | What it does |
|---|---|
| `just quick` | The Quick suite: formatting, lints and in-process tests. Run it before every push, or let the pre-push hook of `just hooks` run it; never both. |
| `just pre-merge` | The Pre-merge suite on this host: every test, the Quick suite's included, and the stress run of new and changed tests, with `just perf`, `just linux` (the tests only Linux decides) and `just visual` (the visual projects) in containers beside them. |
| `just release` | This host's cell of the Release suite. |
| `just release-ci [cells] [commit]` | Dispatches the Release suite on your pushed branch in CI and follows it: every cell by default, or the ones named, separated by commas; on `main`, a commit of `main`, its head by default (ADR 0108). |
| `just test <suite> [packages]` | One suite's tests (`quick`, `pre-merge` or `release`), without lints. |
| `just fmt`, `just lint`, `just build` | Format, check and build Rust, TypeScript and Python. |
| `just codegen` | Regenerates everything generated from `goodfetch-protocol`'s types (ADR 0038). Run it after changing a contract and commit what it writes; `just lint` fails on drift. |
| `just supply-chain` | The pull request's supply-chain gates: `just audit` (the advisories of every lockfile, against the pinned snapshot of `ci/advisories.toml`; `just audit-live`, the daily Live check, reads the newest), `just msrv` (the build on the minimum supported Rust), `just feature-combinations` (cargo-hack), `just miri` and `just notices` (the third-party notice inventory's license check). `just pre-merge` runs them, `just dco` and `just commit-messages`. |
| `just hooks` | Opts in to the git hooks: pre-commit checks formatting, pre-push checks the sign-offs and subjects of the pushed commits, then runs `just quick` unless it already passed on the pushed tree. |

A new dependency meets `STYLE.md`'s dependency bar. Its license is on ADR 0018's allowlist, [`license-allowlist.toml`](license-allowlist.toml), which only the owner changes; after a change, `just codegen` copies it into `deny.toml`. `just notices` checks the license of every crate, npm package and Python distribution the workspace ships (never a development or build dependency, nor `xtask`'s or the test-only crates'; cargo-deny still checks every crate), and of every input a `third-party.toml` file declares: what no package manager lists, such as the Goodfetch Chromium's inputs, the Clean-up rules' inputs and the sources Collie's block-detection rules are seeded from. Each input there gives its `name`, `version` (a release or a pinned commit), the SPDX `license` its authors state, its `source` and its `use`: `unmodified`, `modified`, `compiled-filter-list` or `download-only` (ADRs 0018 and 0047); an input without its `license` or its `source` fails the check, and so does one that names webappanalyzer or Scrapfly's Antibot-Detector: those two are watchlists, never sources. The weekly watchlist job (`.github/workflows/watchlist.yml`) opens a `watchlist` issue for each vendor they list that no block-detection rule covers; a rule for it is written from the vendor's documentation and Goodfetch's own captures, and the vendor joins `crates/goodfetch-collie/src/block_detection/vendors.toml` with that rule (story 61). A third-party crate only Foxhound's capabilities use joins Collie's ban list in [`ci/deny/collie.toml`](ci/deny/collie.toml) in the pull request that adds it (ADR 0087).

A local run lists every part it could not run on your host, and why. Its exit code covers what ran; CI is the verdict.

## Pull requests

A pull request needs the "all green" check and the Chromium build gate to pass on its latest commit. Tests are written first and never retried (`TESTING.md`). Report a security flaw privately, as [`SECURITY.md`](SECURITY.md) says, never in a pull request.

A change that grows a Product's file past its Size budget's increase threshold (+1 MB per compressed release file, +3 MB per OCI image tag; ADRs 0057 and 0087) will fail `xtask size increase`, which the merge queue runs once the repository is public (no workflow runs it while it is private: the Release cells run `xtask size estimate`, which never fails), until the pull request body has a "Size increase" section with one line per file, naming it as the check's finding does, with a delta at least the measured one and its reason:

```markdown
## Size increase

- collie/x86_64-unknown-linux-gnu/collie.whl: +1.10 MB, the extractor's tables
```

and the owner accepts it when merging (ADR 0133) with a comment saying "Size increase accepted" that names the pull request's head commit, in full or abbreviated to at least 7 characters. A ceiling is never passed this way.
