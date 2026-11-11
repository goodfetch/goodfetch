# Goodfetch

The repository of Goodfetch's two Products: Collie, for browser automation, and Foxhound, for crawling, which contains Collie (ADR 0084). `rnd/` is R&D material kept for inspiration only: nothing in it is an adopted term or decision, and it sets no process. Run the lab's commands from `rnd/lab/` (see its `README.md`). `docs/` holds the Products' final documentation, the ADRs and the agent configuration.

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

Goodfetch has no revenue: spend nothing until the owner says so (ADR 0076). For every runner, credit, account, service or assessment, choose the free tier or a local alternative and record the paid option as declined; never create, enable or list as pending anything that bills. Live model calls go through the Vercel AI Gateway's free tier. The owner approved renting one Hetzner Cloud CCX43 server by the hour (€0.531 an hour including VAT), with its Primary IPv4 and a 200 GB volume (about €10 a month), for the implementation and its gate, until the owner ends it. While it is rented, it also runs the nightly fuzzing, which is no reason to keep it; #2454 builds that job. Each published version's comparison report also comes from it while the owner rents it (ADR 0025).

## Secrets

Local secrets live in the git-ignored `.env`, filled by `scripts/env-setup.sh` from `.env.example`. Run a `justfile` recipe's lines by hand without `bash -x` or `set -x`, and search their logs for error text: recipes such as `just lint` expand `$(gh auth token)`, so a shell trace prints the live `GITHUB_TOKEN`.

## Code style

Code follows [`STYLE.md`](STYLE.md), and the contracts follow ADR 0038. Tests follow [`TESTING.md`](TESTING.md): read its Contents, then only the sections for what you touch. The checks each role runs, implementer, merger or pull request, are its [Checks by role](TESTING.md#checks-by-role); an orchestrator's brief points there and lists none.

The quality bar lives in the Quality register of [`TESTING.md`](TESTING.md#quality-register); the project has no `CONSTRAINTS.md`. Read the register before writing code on a Shared path (ADR 0140). Change a limit only by the register's rules: tighten it in the same pull request, loosen it with the owner.

Traps that break things, each explained in `STYLE.md`:

- Write to stdout or stderr only from the protocol transport, the CLI's output writer and the `tracing` subscriber's writer; everything else logs through `tracing`. Stdout is the JSON-RPC channel when the binary is an Attached engine started by an SDK, or the stdio MCP server. Test-only executables are exceptions: `goodfetch-scripted-engine` (ADR 0069), `goodfetch-test-env` (ADR 0062) and the counts benchmark harnesses that `xtask counts` reads (`examples/counts.rs`, ADR 0140) write their own stdout, and none ships in a Product.
- Change contracts in `goodfetch-protocol`'s Rust types, then run `xtask codegen` and commit its output in the same change; generated code (`generated/` folders, the committed JSON Schema bundle and catalog file) is never edited by hand.
- Spawn every task through its owner's tracker, never with a bare `tokio::spawn` or `spawn_blocking`. Outside tests, return an error or `expect("invariant: …")` rather than `unwrap()`. Silence a lint with `#[expect(lint, reason = "…")]`, never `#[allow]`.
- Make a capability a setting, never a Cargo feature. Outside `crates/foxhound/`, only `xtask` depends on a crate inside it (ADR 0087).
- Name identifiers with `GLOSSARY.md` terms, never their `_Avoid_` terms.

## Merging

CI runs locally (ADR 0135); GitHub Actions runs the Release suite only.

- Merge a pull request only with `just merge <number>`: it gates the pull request's exact head in a clean, locked worktree and merges only when the gate passes. While the owner rents it, `just merge` runs on the Hetzner server (ADR 0135's amendment of 2026-10-22).
- During the refactor, until the owner ends it, a pull request merges on the quick gate and the full gate runs once per finished spec (the owner's decision of 2026-11-10, ADR 0135). `just merge` runs the docs gate when `xtask affected` prints `gate=docs`, and the quick gate when it prints `gate=quick`. When it prints `gate=full`, a test reads the change: the full gate runs on a spec's pull request, whose head branch is `spec/<n>`, and the quick gate on every other. `just merge <number> --full` asks for the full gate on any pull request. The full gate is also taken by the tenth change a test reads since the last full gate, the nine before it having merged on the quick gate, and by the pull request that pins the reference build (the owner's decisions of 2026-11-10, grilling of G-09, Qb and Q5, and of 2026-11-11 on which change takes it, « Yes X10, yes a–i », answering the chief of staff's question b, ADR 0135). `just merge` takes the tenth to the full gate by itself: it counts those merges from the merge messages on `main`'s first-parent line (#2457). The pull request that pins the reference build merges with `--full`. A quick merge of a change a test reads does not run the Pre-merge suite, perf, the stress run, the visual tests, the Linux-only tests or the full gate's other steps, and its merge message says so; the next full gate, a spec's pull request's or the count's, is where they run.
- The backstop has a switch, `ci/gate-backstop.state` (`on` or `off`), which only the owner changes; it is off since 2026-11-10 (ADR 0135's amendment of that date). While it is on, `just merge` refuses once 10 merges have landed on `main` since the backstop's last green run, and only the owner passes `--past-cap` (ADR 0135's amendment of 2026-11-04); while it is off, that cap counts nothing.
- In a stack of pull requests, merge the bottom one first.
- When several ready pull requests would each get the full gate, combine them into one branch and run `just merge` once.
- Dispatch the Release suite with `just release-ci`. On `main`, name the hosted cells: the default, `all`, includes the macOS cells, which queue for the unregistered `owner-machine` runner and fail after 24 hours.
- Keep every other workflow disabled: when a merge brings a new workflow file to `main`, disable it in the repository's Actions settings unless an ADR turns it on.
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
| The Console and the React wrappers (React, shadcn on Base UI, Vite) | `vercel-react-best-practices` (its React rules, not its Next.js ones), `vercel-composition-patterns`, `writing-react-effects`, `shadcn`, `shadcn-guardrails` |
| The PostgreSQL and Supabase Sinks | `postgres`, `supabase-postgres-best-practices` |

Rust has no Rust-specific skill: its section of `STYLE.md` and the references it names are the guide.

## Agent skills

Agent work follows Matt Pocock's skills as written; the repository adds standards, not process steps (ADR 0133). Where a skill asks for more checks or more implementers at once than `TESTING.md`'s [Checks by role](TESTING.md#checks-by-role) allows, such as `/implement`'s full test suite at the end or `/implement-spec`'s maximum concurrency, Checks by role wins.

### Issue tracker

GitHub Issues, with sub-issues, native dependencies and milestones. See `docs/agents/issue-tracker.md`.

### Triage labels

The default triage label vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.
