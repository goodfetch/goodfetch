# goodfetch-collie

Collie, Goodfetch's Product for browser automation. This crate is Collie's
Rust SDK and its executable.

## Prerequisites

Building the crate from source needs, on the build machine:

- CMake
- a C and C++ compiler
- libclang
- git

## The Rust SDK

```sh
cargo add goodfetch-collie
```

A connection is the Product: `Collie::connect` starts Collie's engine in
your program's own process, and `Collie::scope` opens a connection, lends it
to a body and closes it after. Its handles are cheap clones that are `Send`,
`Sync` and `'static`, and its futures run on any executor. A program that
does not use async calls the same names under `goodfetch_collie::blocking`.

A connection, and every Run, Session or page under it, belongs to the
process that opened it: a child created by `fork` that uses one gets
`connection_forked` before anything reaches the parent's engine, and
connects on its own (ADR 0078).

### Schemas

A program types data with a schema in its own types (ADR 0082): a type
deriving `JsonSchema`, through the `schemars` this crate re-exports, and
serde's `Serialize` and `Deserialize`, or a JSON Schema written by hand
(`Schema::json`), whose values are `serde_json::Value`. The engine receives
the schema's deserialize side; a sent value must parse back as itself, and
an answer is decoded by the type's own `Deserialize`. Only JSON crosses: a
value that is not JSON (NaN, an infinity, a map key that is not a string)
fails with `SchemaError::NotJson`, naming its path as a JSON Pointer.
A member that answers its operation's whole result answers an `Answer`,
which keeps the fields this SDK does not know, readable through
`Answer::raw`; one that answers a single field of its result, such as
`page.goto`'s `Response`, answers that field alone. An input's builder has
no setter for a field its operation does not take.

Supported range, tested at its floor, which `Cargo.lock` pins. Each names
one major in the SDK's types, so a new major of one is a new Goodfetch
major:

| Library | Versions |
|---|---|
| schemars | 1.2.3 and later 1.x |
| serde | 1.0.229 and later 1.x |
| serde_json | 1.0.154 and later 1.x |

## The executable

The executable is behind the `executable` feature, so that a program
embedding the crate never compiles it:

```sh
cargo install goodfetch-collie --locked --features executable
```

`cargo binstall goodfetch-collie` installs the release's own archive instead,
without compiling.
