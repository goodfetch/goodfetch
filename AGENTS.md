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

## Money

Goodfetch has no revenue: spend nothing until the owner says so (ADR 0076). For every runner, credit, account, service or assessment, choose the free tier or a local alternative and record the paid option as declined; never create, enable or list as pending anything that bills. Live model calls go through the Vercel AI Gateway's free tier. Local secrets live in the git-ignored `.env`, filled by `scripts/env-setup.sh` from `.env.example`. The owner approved renting one Hetzner Cloud CCX43 server by the hour (€0.531 an hour including VAT) for the implementation and its gate, until the owner ends it.

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

- Merge a pull request only with `just merge <number>`: it gates the pull request's exact head in a clean, locked worktree, runs the quick gate instead when `xtask affected` prints `gate=quick` and the docs gate when it prints `gate=docs`, and merges only when the gate passes. It refuses once 10 merges have landed on `main` since the backstop's last green run; only the owner passes `--past-cap` (ADR 0135's amendment of 2026-11-04). In a stack of pull requests, merge the bottom one first. While the owner rents it, `just merge` runs on the Hetzner server (ADR 0135's amendment of 2026-10-22).
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

Agent work follows Matt Pocock's skills as written; the repository adds standards, not process steps (ADR 0133).

### Issue tracker

GitHub Issues, with sub-issues, native dependencies and milestones. See `docs/agents/issue-tracker.md`.

### Triage labels

The default triage label vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.
