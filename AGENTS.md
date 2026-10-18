# Goodfetch

The repository of Goodfetch's two Products: Collie, for browser automation, and Foxhound, for crawling, which contains Collie (ADR 0084). `rnd/` holds the R&D material (pre-test proposals, plan, proposed ADRs, sourced research, comparative lab and its report), kept for inspiration only: nothing in it is an adopted term or decision, and it sets no process. The executable lab is `rnd/lab/` (see `rnd/lab/README.md`); its commands are run from that folder. `docs/` is reserved for the Products' final documentation.

## Money

Goodfetch has no revenue: no paid resource until the owner says so ([ADR 0076](docs/adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md)). Never create, enable or propose as pending anything that bills (runners, credits, accounts, services, assessments); choose the free tier or a local alternative, and record the paid option as declined. Live model calls go through the Vercel AI Gateway's free tier. Local secrets live in the git-ignored `.env`, filled by `scripts/env-setup.sh` from `.env.example`.

## Release status

Release status: `pre-release`. Follow the global pre-release policy; regenerate
changed contract output with `xtask codegen` in the same change.
Nothing is published yet, so no contract format changes version before the
first publish (ADR 0066).
Product-promised compatibility follows ADRs 0030, 0041 and 0047, including
versioned format readers, upgrade commands and published-version semantics.
Nightlies and previews count as publication: if any channel is published while
this status remains pre-release, ask the owner before a breaking change.

## Authorship

Follow global authorship rules. Required DCO sign-offs identify the human
signer; existing commits on `main` are not rewritten for attribution cleanup.

## Code style

Code follows [`STYLE.md`](STYLE.md), and the contracts follow ADR 0038. Tests follow [`TESTING.md`](TESTING.md): the seams in a spec's Testing Decisions are already agreed, so test there without asking.

Traps that break things:

- Only the protocol transport, the CLI's output writer and the `tracing` subscriber's writer touch stdout or stderr: when the binary is an Attached engine started by an SDK, or the stdio MCP server, stdout is the JSON-RPC channel. Everything else logs through `tracing`.
- Never edit generated code (`generated/` folders, the committed JSON Schema bundle and catalog file): change the Rust types in `goodfetch-protocol` and run `xtask codegen`.
- No task started outside its owner's tracker (no bare `tokio::spawn` or `spawn_blocking`), no `unwrap()` outside tests, no `#[allow]` (use `#[expect(lint, reason = "…")]`).
- A capability is a setting, never a Cargo feature, and nothing outside `crates/foxhound/` but `xtask` depends on a crate inside it (ADR 0087).
- Identifiers use `GLOSSARY.md` terms, never their `_Avoid_` terms.

## Language skills

Before writing or reviewing code in a language below, load its skills when your host has them. Those in bold are vendored at pinned commits in [`.agents/skills/`](.agents/skills/README.md), so every agent has them. `STYLE.md` and `TESTING.md` win where a skill differs: for example, the SDKs accept Standard Schema instead of depending on a schema library, and engine responses are not validated at runtime beyond the protocol version check.

| Code | Skills |
|---|---|
| Any typed code: Rust, TypeScript, Python | **`principle-type-system-discipline`** |
| TypeScript (`.ts`, `.tsx`) | **`typescript-best-practices`** |
| Python | **`ruff`**, **`uv`** |
| The Console and the React wrappers (React, shadcn on Base UI, Vite) | `vercel-react-best-practices` (its React rules, not its Next.js ones), `vercel-composition-patterns`, `react-mnne`, `shadcn` |
| The PostgreSQL and Supabase Sinks | `postgres`, `supabase-postgres-best-practices` |

Rust has no language skill: its section of `STYLE.md` and the references it names are the guide.

## Agent skills

Agent work follows Matt Pocock's skills as written; the repository adds standards, not process steps (ADR 0133).

### Issue tracker

GitHub Issues, with sub-issues, native dependencies and milestones. See `docs/agents/issue-tracker.md`.

### Triage labels

The default triage label vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/`; `rnd/` is inspiration only. See `docs/agents/domain.md`.
