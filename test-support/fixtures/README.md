# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.
