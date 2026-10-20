# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`endpoint-table.toml` is the endpoint table (ADR 0067): one row per refusal rule, one cell per listener it applies to. `goodfetch-test-support`'s `EndpointRunner` reads it, and the Quick suite checks it.

`ssrf/` holds the SSRF case table (`cases.toml`, ADR 0067), which every layer's runner applies through `goodfetch-test-support`'s `SsrfTable::committed()` and `run_cases`, and IANA's IPv4 and IPv6 special-purpose registries, committed as IANA publishes them, each with a `.source.toml` giving its date. The Quick suite fails when a registry block that is not globally reachable has no case.

`snapshot-filters.json` holds the shared snapshot rules (ADR 0062): the run's values to replace literally, then the patterns, in order. A pattern keeps to the syntax Rust's `regex`, JavaScript and Python's `re` read alike, and a replacement is literal text, with neither `$` nor `\`. insta applies them through `snapshot_settings()`, Vitest and inline-snapshot through each package's `redact()`.

`locales.json` is ADR 0073's table of locales and time zones; the DST days are 2026's under the IANA rules (the European Union's last Sundays of March and October, the United States' second Sunday of March and first Sunday of November). `unicode-record.json` is ADR 0073's Unicode record, written with JSON escapes so every invisible character shows. Rust, TypeScript and Python read both files.

`canaries/` holds the Secret canaries' files (ADR 0067). `search.json` is the fixture set Rust's `Canaries::search`, TypeScript's `searchCanaries` and Python's `search_canaries` all pass: canaries, outputs that hold them raw, JSON-escaped, URL-encoded, in base64 (standard, URL-safe and inside a Basic header) and in hex, and the hit each search must give, with its output, byte offset and whether it is a planted copy; its values are not canary-shaped, so no secret scan flags them. `gitleaks.toml` is the `gitleaks` rule, one per kind, which `xtask codegen` writes from the kinds and `--check` keeps current; never edit it by hand. `artifacts/` holds canary-shaped values on purpose: `ci/canary-rule.sh` (`just lint`) shows each file of `leaked/` failing a scan with the rule and `clean/` passing, and the rule's allowlist passes over the folder, so a scan of commits does too.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.

`outcome-vectors/` holds the hand-written outcome vectors of the result document (ADR 0063): what a `journey` or `conformance` document holds and the outcome it must get, which `goodfetch-protocol`'s constructors and the TypeScript and Python helpers pass, and which `journey` documents the reader refuses for not holding exactly their Journey's steps, checks and tasks.
