## Property tests, fuzzing and doctests

- **Parsers and stateful rules get property tests:** `proptest` in Rust, `hypothesis` in Python. Their regression files are committed.
- **Untrusted input** gets a `cargo-fuzz` target for every decoder of input Goodfetch did not write itself, replayed by the `#[test]` that `fuzz_replay!` writes, on every OS. The list and the rest of fuzzing are under [Security](security.md) (ADR 0067).
- **Doctests only on the public crates** (`goodfetch-collie`, `goodfetch-foxhound` and `goodfetch-collie-client`, ADRs 0090 and 0095), where they are the examples of ADR 0030. `goodfetch-collie-client`'s doctests connect through `goodfetch-collie`, a path-only dev-dependency, which `cargo publish` drops, since a published crate keeps only the dev-dependencies that specify a `version` ([Cargo reference](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#development-dependencies)).
