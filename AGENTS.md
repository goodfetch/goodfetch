# Goodfetch

Web scraping and automation framework project. `rnd/` holds the R&D material (pre-test proposals, plan, proposed ADRs, sourced research, comparative lab and its report), kept for inspiration only: nothing in it is an adopted term or decision, and it sets no process. The executable lab is `rnd/lab/` (see `rnd/lab/README.md`); its commands are run from that folder. `docs/` is reserved for the product's final documentation.

## Delivery

Delivery mode: autonomous. Agents merge their own pull requests after a fresh review recorded on the pull request's exact head and green required checks. Deploying to production, running production migrations and resetting a running service's data still need the owner's explicit request. Production means running services (the online Proving ground, Goodfetch Cloud); a release through the gated pipeline of ADRs 0030 and 0032 is not a production deploy, nor are the docs site's deploys from `main` (ADR 0030); neither needs a separate request.

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

Code follows [`STYLE.md`](STYLE.md), and the contracts follow ADR 0038. Traps that break things:

- Only the protocol transport, the CLI's output writer and the `tracing` subscriber's writer touch stdout or stderr: when the binary is an Attached engine started by an SDK, or the stdio MCP server, stdout is the JSON-RPC channel. Everything else logs through `tracing`.
- Never edit generated code (`generated/` folders, the committed JSON Schema bundle and catalog file): change the Rust types in `goodfetch-protocol` and run `xtask codegen`.
- No task started outside its owner's tracker (no bare `tokio::spawn` or `spawn_blocking`), no `unwrap()` outside tests, no `#[allow]` (use `#[expect(lint, reason = "…")]`).
- A capability is a setting, never a Cargo feature.
- Identifiers use `GLOSSARY.md` terms, never their `_Avoid_` terms.

## Agent workflows

For issue work, read [issue tracker](docs/agents/issue-tracker.md); for triage,
read [label mapping](docs/agents/triage-labels.md). Before changing terminology
or ADRs, read [domain docs](docs/agents/domain.md).
