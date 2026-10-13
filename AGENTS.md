# Goodfetch

Web scraping and automation framework project. `rnd/` holds the R&D material (pre-test proposals, plan, proposed ADRs, sourced research, comparative lab and its report), kept for inspiration only: nothing in it is an adopted term or decision, and it sets no process. The executable lab is `rnd/lab/` (see `rnd/lab/README.md`); its commands are run from that folder. `docs/` is reserved for the product's final documentation.

## Delivery

Delivery mode: autonomous. Agents merge their own pull requests after a fresh review recorded on the pull request's exact head and green required checks. Deploying to production and running production migrations still need the owner's explicit request. Production means running services (the online Proving ground, Goodfetch Cloud); a release through the gated pipeline of ADRs 0030 and 0032 is not a production deploy, nor are the docs site's deploys from `main` (ADR 0030); neither needs a separate request.

## Release status

Release status: pre-release. Nothing has been published on any channel, no one outside the project runs Goodfetch, and any running service holds only test data. Change a contract, setting, format or stored shape in place: update every caller, regenerate with `xtask codegen` and commit its output in the same change, and add no alias, shim, fallback, deprecation or migration for an earlier state of this repository. A contract format's version does not change before Goodfetch is published, so no reader for an earlier shape is owed. Deploying to, migrating or resetting a running service still needs the owner's explicit request (Delivery).

The compatibility the product promises its users is V1 scope and is built as its ADRs specify, including ADR 0030's versioned contract formats with their readers, `goodfetch upgrade-files`, and semver and deprecation from 1.0 (any `0.y` preview minor may break); the Usage ledger's add-only versions and *must understand* Limits (ADR 0041); and the Clean-up rules' format majors (ADR 0047). The owner switches this to released when Goodfetch is first published on any channel (the nightlies and the `0.y` previews start in M4, ADR 0058), and ADR 0030 and each format's own ADR then govern every breaking change.

## Code style

Code follows [`STYLE.md`](STYLE.md), and the contracts follow ADR 0038. Traps that break things:

- Only the protocol transport, the CLI's output writer and the `tracing` subscriber's writer touch stdout or stderr: when the binary is an Attached engine started by an SDK, or the stdio MCP server, stdout is the JSON-RPC channel. Everything else logs through `tracing`.
- Never edit generated code (`generated/` folders, the committed JSON Schema bundle and catalog file): change the Rust types in `goodfetch-protocol` and run `xtask codegen`.
- No task started outside its owner's tracker (no bare `tokio::spawn` or `spawn_blocking`), no `unwrap()` outside tests, no `#[allow]` (use `#[expect(lint, reason = "…")]`).
- A capability is a setting, never a Cargo feature.
- Identifiers use `GLOSSARY.md` terms, never their `_Avoid_` terms.

## Agent skills

### Issue tracker

Issues and specs live in the GitHub Issues of `Jbmanllr/automan`, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical roles, with the default labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: the glossary is `GLOSSARY.md` at the repo root and the ADRs are in `docs/adr/`, both created lazily. `rnd/GLOSSARY.md` and `rnd/adr/` are R&D material for inspiration only, not adopted terms or decisions. See `docs/agents/domain.md`.
