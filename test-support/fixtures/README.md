# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`endpoint-table.toml` is the endpoint table (ADR 0067): one row per refusal rule, one cell per listener it applies to. `goodfetch-test-support`'s `EndpointRunner` reads it, and the Quick suite checks it.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.
