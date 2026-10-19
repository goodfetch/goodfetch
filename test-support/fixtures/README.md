# Shared fixtures

Data that several languages' tests read, each file at most 256 KiB (ADR 0062). The Test infrastructure spec fills this folder.

`schemas/` holds the JSON Schemas of `goodfetch-test-support`'s file types, such as `goodfetch-test-env`'s description and the test tool manifest. `xtask codegen` writes them from the crate's Rust types and `xtask codegen --check` fails when they drift; never edit them by hand.

`outcome-vectors/` holds the hand-written outcome vectors of the result document (ADR 0063): what a `journey` or `conformance` document holds and the outcome it must get, which `goodfetch-protocol`'s constructors and the TypeScript and Python helpers pass.
