# Contributing to Goodfetch

Thank you for helping. This guide covers what every change needs; [`STYLE.md`](STYLE.md) says how code is shaped and [`TESTING.md`](TESTING.md) how it is tested.

## Sign off every commit (DCO)

Goodfetch takes contributions under the [Developer Certificate of Origin](https://developercertificate.org/), with no CLA (ADR 0018). Sign off each commit with your real name:

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line, which certifies that you wrote the change or have the right to submit it under the repository's licence, `MIT OR Apache-2.0`. The sign-off must name the commit's author. A pull request with an unsigned commit cannot merge: the supply-chain job checks every commit it adds but merges (`just dco` checks yours since `origin/main`).

## One entry point: `just`

Install [`just`](https://just.systems), Rust through `rustup` (the toolchain is pinned in `rust-toolchain.toml`), Node 24 with pnpm, and uv. CI calls the same recipes:

| Command | What it does |
|---|---|
| `just quick` | The Quick suite: formatting, lints and in-process tests. Run it before every push. |
| `just pre-merge` | The Pre-merge suite on this host: the Quick suite and every test. |
| `just release` | This host's cell of the Release suite. |
| `just release-ci` | Dispatches the Release suite on your pushed branch in CI and follows it. |
| `just test <suite> [packages]` | One suite's tests (`quick`, `pre-merge` or `release`), without lints. |
| `just fmt`, `just lint`, `just build` | Format, check and build Rust, TypeScript and Python. |
| `just codegen` | Regenerates everything generated from `goodfetch-protocol`'s types (ADR 0038). Run it after changing a contract and commit what it writes; `just lint` fails on drift. |
| `just supply-chain` | The pull request's supply-chain gates: `just audit` (the advisories of every lockfile), `just msrv` (the build on the minimum supported Rust), `just feature-combinations` (cargo-hack) and `just miri`. `just pre-merge` runs them, and `just dco`. |
| `just hooks` | Opts in to the git hooks: pre-commit checks formatting, pre-push runs `just quick`. |

A new dependency meets `STYLE.md`'s dependency bar. Its license is on ADR 0018's allowlist, [`license-allowlist.toml`](license-allowlist.toml), which only the owner changes; after a change, `just codegen` copies it into `deny.toml`. A third-party crate only Foxhound's capabilities use joins Collie's ban list in [`ci/deny/collie.toml`](ci/deny/collie.toml) in the pull request that adds it (ADR 0087).

A local run lists every part it could not run on your host, and why. Its exit code covers what ran; CI is the verdict.

## Pull requests

A pull request needs the "all green" check and the Chromium build gate to pass on its latest commit. Tests are written first and never retried (`TESTING.md`). Report a security flaw privately, as [`SECURITY.md`](SECURITY.md) says, never in a pull request.

A change that grows a Product's file past its Size budget's increase threshold (+1 MB per compressed release file, +3 MB per OCI image tag; ADRs 0057 and 0087) fails `xtask size increase` until the pull request body has a "Size increase" section with one line per file, naming it as the check's finding does, with a delta at least the measured one and its reason:

```markdown
## Size increase

- collie/x86_64-unknown-linux-gnu/collie.whl: +1.10 MB, the extractor's tables
```

and the owner accepts it when merging (ADR 0133) with a comment saying "Size increase accepted" that names the pull request's head commit, in full or abbreviated to at least 7 characters. A ceiling is never passed this way.
