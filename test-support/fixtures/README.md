# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`endpoint-table.toml` is the endpoint table (ADR 0067): one row per refusal rule, one cell per listener it applies to. `goodfetch-test-support`'s `EndpointRunner` reads it, and the Quick suite checks it.

`ssrf/` holds the SSRF case table (`cases.toml`, ADR 0067), which every layer's runner applies through `goodfetch-test-support`'s `SsrfTable::committed()` and `run_cases`, and IANA's IPv4 and IPv6 special-purpose registries, committed as IANA publishes them, each with a `.source.toml` giving its date. The Quick suite fails when a registry block that is not globally reachable has no case.

`snapshot-filters.json` holds the shared snapshot rules (ADR 0062): the run's values to replace literally, then the patterns, in order. A pattern keeps to the syntax Rust's `regex`, JavaScript and Python's `re` read alike, and a replacement is literal text, with neither `$` nor `\`. insta applies them through `snapshot_settings()`, Vitest and inline-snapshot through each package's `redact()`.

`locales.json` is ADR 0073's table of locales and time zones; the DST days are 2026's under the IANA rules (the European Union's last Sundays of March and October, the United States' second Sunday of March and first Sunday of November). `unicode-record.json` is ADR 0073's Unicode record, written with JSON escapes so every invisible character shows. Rust, TypeScript and Python read both files.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.

`outcome-vectors/` holds the hand-written outcome vectors of the result document (ADR 0063): what a `journey` or `conformance` document holds and the outcome it must get, which `goodfetch-protocol`'s constructors and the TypeScript and Python helpers pass, and which `journey` documents the reader refuses for not holding exactly their Journey's steps, checks and tasks.

`engine-scripts/` holds the Scripted engine's scripts (ADR 0069), grouped by capability: `self-test/` holds the scripts every player plays to a tiny Client in its own tests, `hello/` the refused `hello`, `attached-engine/` the faults of an Attached engine (a crash mid-call, garbage on stdout), and `daemon/` the scripts a Client plays to the Scripted engine found as a Daemon, each `hello` carrying the token `scripted-daemon-token` (a refused `hello`, a dead PID in the runtime file, a connection dropped mid-call). `goodfetch-test-support`'s Quick suite validates every message step against the committed JSON Schema bundle and catalog. `script.schema.json` is the script format's schema, which `xtask codegen` writes from `goodfetch-test-support`'s `Script` type; never edit it by hand.
