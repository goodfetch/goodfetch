# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`endpoint-table.toml` is the endpoint table (ADR 0067): one row per refusal rule, one cell per listener it applies to. `goodfetch-test-support`'s `EndpointRunner` reads it, and the Quick suite checks it.

`ssrf/` holds the SSRF case table (`cases.toml`, ADR 0067), which every layer's runner applies through `goodfetch-test-support`'s `SsrfTable::committed()` and `run_cases`, and IANA's IPv4 and IPv6 special-purpose registries, committed as IANA publishes them, each with a `.source.toml` giving its date. The Quick suite fails when a registry block that is not globally reachable has no case.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.

`outcome-vectors/` holds the hand-written outcome vectors of the result document (ADR 0063): what a `journey` or `conformance` document holds and the outcome it must get, which `goodfetch-protocol`'s constructors and the TypeScript and Python helpers pass.
