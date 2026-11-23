# Compatibility corpus

The Compatibility corpus (ADR 0066): committed files, one set per version of each format the registry in `goodfetch-protocol` lists.

- `<format>/head/` holds the current version, written by `xtask compat write` from the format's writer: the `compat-<format>` example of the package that owns it, which runs the format's fixed scenarios. Never edit it by hand.
- `<format>/<major>.<minor>/` holds a frozen set with its `SHA256SUMS`, copied from a release tag's `head/` by `xtask compat freeze --tag <tag>`. Its files never change; only its `snapshots/` may.
- Each set holds `format.json`, the format's name, version, storage and schema, beside the writer's files.
- `releases.toml` records what each release shipped.
- `api.toml` names the previous release's commit that `xtask api-check` compares the contracts with, `just semver-checks` the Rust crates under semver, and `xtask sdk-api` the TypeScript and Python SDK packages, and the breaks announced for the next major.
- `upgrade-paths.toml` lists the majors `upgrade-files` upgrades from (ADR 0066), empty before the first release; `goodfetch-engine`'s `upgrade` module reads it, and its tests derive the refusals from it.

`xtask compat check` (Quick suite) fails on a `head/` set that differs from what `write` produces, a writer whose second run writes other bytes than its first (each writer runs twice, and a passing check notes the digest both runs wrote, `note[identical]`), a changed frozen byte, a missing `head/` set or a missing frozen set for a version `releases.toml` records, incomplete schema coverage, and a schema changed while its version equals its newest frozen version. Pre-release, only `head/` sets exist; each area adds its format's set in the change that creates the format.

Each registered format's refusal cases are generated from its rule's table by `xtask compat refusals`, which `xtask codegen` also runs: one file per case under `crates/goodfetch-protocol/generated/refusals/<format>/`, held to the registry by `xtask codegen --check`. Each case names a fabricated version, the rule that decides it and its verdict, and for a protocol the fabricated `hello` and its reply, the Product check of every pair of Products and the Browser host's lower minor among them. `goodfetch-protocol`'s Quick test judges every case by its rule's compatibility function, the engine's runs the protocol's `hello`s in process and snapshots each refusal, and the conformance harness sends them to each Product's running executable (Pre-merge).
