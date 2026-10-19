# Compatibility corpus

The Compatibility corpus (ADR 0066): committed files, one set per version of each format the registry in `goodfetch-protocol` lists.

- `<format>/head/` holds the current version, written by `xtask compat write` from the format's writer: the `compat-<format>` example of the package that owns it, which runs the format's fixed scenarios. Never edit it by hand.
- `<format>/<major>.<minor>/` holds a frozen set with its `SHA256SUMS`, copied from a release tag's `head/` by `xtask compat freeze --tag <tag>`. Its files never change; only its `snapshots/` may.
- Each set holds `format.json`, the format's name, version, storage and schema, beside the writer's files.
- `releases.toml` records what each release shipped.

`xtask compat check` (Quick suite) fails on a `head/` set that differs from what `write` produces, a changed frozen byte, a missing `head/` set or a missing frozen set for a version `releases.toml` records, incomplete schema coverage, and a schema changed while its version equals its newest frozen version. Pre-release, only `head/` sets exist; each area adds its format's set in the change that creates the format.
