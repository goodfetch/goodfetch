# Goodfetch

The repository of Goodfetch's two Products: Collie, for browser automation, and Foxhound, for crawling, which contains Collie (ADR 0084). `rnd/` is R&D material kept for inspiration only: nothing in it is an adopted term or decision, and it sets no process. Run the lab's commands from `rnd/lab/` (see its `README.md`). `docs/` holds the Products' final documentation, the ADRs, the sections of `TESTING.md` (`docs/testing/`) and the agent configuration.

## Release status

Release status: `pre-release`. Follow the global pre-release policy.
Nothing is published yet, so no contract format changes version before the
first publish (ADR 0066).
Product-promised compatibility follows ADRs 0030, 0041 and 0047, including
versioned format readers, upgrade commands and published-version semantics.
Nightlies and previews count as publication: if any channel is published while
this status remains pre-release, ask the owner before a breaking change.

From the first preview, the owner's answer of 2026-11-11 (ADR 0030's note of
that day) is the owner's word for every break between previews. The next
preview's `upgrade-files` must upgrade every file the previous preview
wrote, and its release pull request lists each break for the owner's approval.
A patch of a preview breaks nothing, and the five add-only files only add.
Nightlies freeze nothing (ADR 0066) and get no upgrade of their own.

## Money

Goodfetch has no revenue: spend nothing until the owner says so (ADR 0076). For every runner, credit, account, service or assessment, choose the free tier or a local alternative and record the paid option as declined; never create, enable or list as pending anything that bills. Live model calls go through the Vercel AI Gateway's free tier. The owner approved renting one Hetzner Cloud CCX53 server by the hour (€1.017 an hour including VAT; the owner rescaled it from a CCX43 on 2026-11-26, ADR 0076), with its Primary IPv4 and a 500 GB volume (about €25 a month; the owner's size since 2026-11-12, ADR 0076), for the implementation and its gate, until the owner ends it. While it is rented, the fuzzing run, started by hand, runs on it too, which is no reason to keep it. Each published version's comparison report also comes from it while the owner rents it (ADR 0025). The owner also approved (2026-11-21) a payment method on the GitHub organization, under budgets that stop usage when reached: $20 a month for Actions beyond its included minutes (raised from $10 the same day) and $0 for every other metered product (ADR 0076); raising a budget or removing a stop is the owner's.

## Secrets

Local secrets live in the git-ignored `.env`, filled by `scripts/env-setup.sh` from `.env.example`. Run a `justfile` recipe's lines by hand without `bash -x` or `set -x`, and search their logs for error text: recipes such as `just lint` expand `$(gh auth token)`, so a shell trace prints the live `GITHUB_TOKEN`. On the gate host, the fuzzing run (`just fuzz-nightly`) builds and runs the fuzz targets as the system user `goodfetch-fuzz`, which `ci/fuzz-user.sh` set up and which cannot read the job user's home, gh's `hosts.yml` or `.env` (#2564); keep `.env` mode 600.

## Code style

Write every repository and tracker artifact (ADRs, specs, tickets, glossary entries, `STYLE.md`/`TESTING.md` edits, pull request bodies) in English, even when the owner talks in French; owner quotes may keep their language.

Code follows [`STYLE.md`](STYLE.md), and the contracts follow ADR 0038. Tests follow [`TESTING.md`](TESTING.md): read its Contents, then only the sections for what you touch. The checks each role runs, implementer, merger or pull request, are its [Checks by role](docs/testing/checks-by-role.md); an orchestrator's brief points there and lists none.

This file's rules are decisions, and its commands, paths, names and accounts of what the code does are facts: where a fact here disagrees with the code, the code is right, so follow it and correct this file in the same change.

The quality bar lives in the Quality register of [`TESTING.md`](TESTING.md#quality-register); the project has no `CONSTRAINTS.md`. Read the register before writing code on a Shared path (ADR 0140). Change a limit only by the register's rules: tighten it in the same pull request, loosen it with the owner, except for a temporary Exception that an open ticket removes, which the coordinator decides and reports (the register's rule of 2026-11-21).

Traps that break things, each explained in `STYLE.md`:

- Write to stdout or stderr only from the protocol transport, the CLI's output writer and the `tracing` subscriber's writer; everything else logs through `tracing`. Stdout is the JSON-RPC channel when the binary is an Attached engine started by an SDK, or the stdio MCP server. Test-only executables are exceptions: `goodfetch-scripted-engine` (ADR 0069), `goodfetch-test-env` (ADR 0062), `goodfetch-toy-plugin` and `goodfetch-scripted-plugin` (ADR 0033) and the counts benchmarks that `xtask counts` reads (`examples/counts.rs` of `goodfetch-engine`, `goodfetch-page` and `goodfetch-foxhound`, and the last's `examples/counts-extraction.rs`, ADR 0140) and `just compare`'s handled program (`examples/compare-handled.rs` of `goodfetch-foxhound`, ADR 0025) write their own stdout, and none ships in a Product.
- Change contracts in `goodfetch-protocol`'s Rust types, then run `xtask codegen` and commit its output in the same change; generated code (`generated/` folders, the committed JSON Schema bundle and catalog file) is never edited by hand.
- Spawn every task through its owner's tracker, never with a bare `tokio::spawn` or `spawn_blocking`. Outside tests, return an error or `expect("invariant: …")` rather than `unwrap()`. Silence a lint with `#[expect(lint, reason = "…")]`, never `#[allow]`.
- Make a capability a setting, never a Cargo feature. Outside `crates/foxhound/`, only `xtask` depends on a crate inside it (ADR 0087).
- Name identifiers with `GLOSSARY.md` terms, never their `_Avoid_` terms.

## Merging

CI runs locally (ADR 0135): `just merge` runs every unit of its gate itself, and GitHub Actions runs the Release suite alone; the pull-request workflow has been disabled since 2026-11-25 (ADR 0135's note of that day).

- **Until the owner ends the 3-day push, or about 2026-11-28** (the owner's decision of 2026-11-25: « tu fais en sorte que ça arrête de passer des heures de test. […] tu fais pas de test alors, ok? », ADR 0135's note of that day): no `just preflight` or `just batch-check`, and no counts, compare, stress or Windows reruns; implementers run only the quick checks on the packages they touched. The bullets below hold again when it ends.
- Merge a pull request only with `just merge <number>`: it gates the pull request's head in a clean, locked worktree and merges only when the gate passes. A head without `main`'s tip needs no refresh first: the gate merges the tip in, pushes that merge to the pull request's branch and gates it (ADR 0135's amendment of 2026-11-17), with what `just codegen` writes on it; a conflict with the tip only in the folders `xtask codegen` owns is settled that way, and a head that conflicts with the tip anywhere else is refused (ADR 0135's note of 2026-11-26). After a red gate, pull the branch before the next push. While the owner rents it, `just merge` runs on the Hetzner server (ADR 0135's amendment of 2026-10-22).
- `just merge` refuses, before it holds anything, a pull request whose merge would close other issues than its body's `Closes #<n>` lines name, one issue a line, or whose commit messages name an issue after a closing keyword that no such line names; a batch's `Part of #<n>` lines close nothing (ADR 0135's note of 2026-11-22).
- The full gate runs only when the owner asks for it, with `just merge <number> --full`, an option only the owner passes, like `--past-cap` (the owner's decision of 2026-11-26, ADR 0135's note of that day: « full gates seulement sur ma demande »). Every other pull request, a spec's last included, takes the gate `xtask affected` picks: the docs gate on `gate=docs`, the quick gate on `gate=quick` or `gate=full`. No rule, branch name, count of quick merges or finished spec takes the full gate. A quick merge of a change a test reads skips the Pre-merge suite, perf, the stress run, the visual tests, the Linux-only tests and the full gate's other steps, and its merge message says so; the next full gate the owner asks for runs them.
- The backstop has a switch, `ci/gate-backstop.state` (`on` or `off`), which only the owner changes; it is off (ADR 0135's amendment of 2026-11-10 on the backstop's switch). While it is on, `just merge` refuses once 10 merges have landed on `main` since the backstop's last green run, and only the owner passes `--past-cap` (ADR 0135's amendment of 2026-11-04); while it is off, that cap counts nothing.
- In a stack of pull requests, merge the bottom one first.
- One spec, one pull request, one review, one quick gate (the owner's decisions of 2026-11-25 and 2026-11-26, ADR 0136's notes of those days). Every ticket of a spec commits on the spec's one branch, `spec/<n>`, base `main` (a branch already named `spec/<n>-partK` keeps its name), and its one pull request's body holds the `Closes #<n>` line of each ticket it closes. `code-review`, with the disprove pass, runs once, on the whole pull request, when the spec's last ticket is done and before its `just merge`; no size threshold, ticket or part takes a review or a pull request of its own. Each `just merge` carries one spec's pull request, never the work of several specs. A spec lands in two pull requests only when another spec needs its code before the rest is done; the first is reviewed the same way before its gate. A pull request that finishes a spec goes first in the gate queue. Before a spec's `just merge`, run `just batch-check` in its worktree: the installs, then `just quick` once over the spec's whole change, which its tickets no longer run themselves (the owner's decision of 2026-11-23).
- Dispatch the Release suite with `just release-ci`. On `main`, name the hosted cells: the default, `all`, includes the macOS cells, which queue for the unregistered `owner-machine` runner and fail after 24 hours.
- Keep every workflow but the Release suite disabled: when a merge brings a new workflow file to `main`, disable it in the repository's Actions settings unless an ADR turns it on.
- Change a repository setting only together with the text that records it: open the pull request amending the ADR or this file when the setting changes, never after.

## Authorship

Sign off every commit (`git commit -s`) as the human signer; existing commits on
`main` are not rewritten for attribution cleanup.

## Language skills

Before writing or reviewing code in a language below, load its skills when your host has them. Those in bold are vendored at pinned commits in [`.agents/skills/`](.agents/skills/README.md), so every agent has them. The ADRs, `STYLE.md` and `TESTING.md` win where a skill differs: for example, the SDKs accept Standard Schema instead of depending on a schema library, and engine responses are not validated at runtime beyond the protocol version check.

| Code | Skills |
|---|---|
| Any typed code: Rust, TypeScript, Python | **`principle-type-system-discipline`** |
| TypeScript (`.ts`, `.tsx`) | **`typescript-best-practices`** |
| Python | **`ruff`**, **`uv`** |
| The React wrappers (React, shadcn on Base UI, Vite) | `vercel-react-best-practices` (its React rules, not its Next.js ones), `vercel-composition-patterns`, `writing-react-effects`, `shadcn`, `shadcn-guardrails` |
| The PostgreSQL and Supabase Sinks | `postgres`, `supabase-postgres-best-practices` |

Rust has no Rust-specific skill: its section of `STYLE.md` and the references it names are the guide.

## Agent skills

Agent work follows Matt Pocock's skills as written; the repository adds standards, not process steps (ADR 0133). Where a skill or `~/.agents/AGENTS.md` asks for other checks, gates or implementers at once than `TESTING.md`'s [Checks by role](docs/testing/checks-by-role.md) allows, such as `/implement`'s full test suite at the end, a ready gate on the pull request to `main`, `/implement-spec`'s maximum concurrency or the global file's "at most 5 live implementers, start with 3", Checks by role wins: implementers run from 3 to 10 at once, the number `run/slot-target` sets from CPU, memory and storage pressure (the owner's decision of 2026-11-26), and the full gate runs only on the owner's `--full`. One departure from a skill is the owner's (2026-11-11, ADR 0133's notes of that day): a `/implement-spec` run has one implementer for the whole spec, in one slot worktree, in place of the skill's implementer and merger for each ticket, and several runs may go side by side. [The Implementation loop](docs/agents/issue-tracker.md#implementation-loop) says how such a run goes.

### Issue tracker

GitHub Issues, with sub-issues, native dependencies and milestones. See `docs/agents/issue-tracker.md`.

### Triage labels

The default triage label vocabulary, and what an issue needs before `ready-for-agent`: its `Existing code` list and the load-proofs check. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.
