# goodfetch-collie

Collie, Goodfetch's Product for browser automation. This crate is Collie's
Rust SDK and its executable.

## Prerequisites

Building the crate from source needs, on the build machine:

- CMake
- make
- a C and C++ compiler
- libclang
- git

A build without one of them fails. On Debian or Ubuntu, beside the C
compiler that Rust links with:

```sh
apt install cmake make g++ libclang-dev git
```

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

The engine runs in a process group of its own and the crate installs no
signal handler, so a terminal's Ctrl+C reaches only your program, whose
default ends it at once, as SIGTERM's does, running no destructor and
before the Run can checkpoint. Close the Run on SIGINT and SIGTERM, then
the connection: wait for the signal beside the work, with
`tokio::signal::ctrl_c()` and `tokio::signal::unix::signal` for SIGTERM or
your own handler, then call `run.close()` and `collie.close()`. The engine
then stops the Run with a Checkpoint, and the next start under its Run ID
resumes from it (ADRs 0078 and 0093). A call cut short because the engine
ended fails with `engine_exited`, whose details hold the engine's
`exit_code`, the `signal` that ended it and `stderr_tail`, the last 64 KiB
of its stderr.

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

That build compiles V8 from source with its sandbox on, as the release
does, which often takes 30 minutes and needs Python 3 and curl beside the
tools above (ADR 0094). The executable it writes is unsigned: it is your own
build, and no Goodfetch signature covers it.

The release's own archive is signed. `cargo binstall goodfetch-collie`
installs it, without compiling:

```sh
cargo binstall goodfetch-collie
```

cargo-binstall checks only minisign signatures, which Goodfetch does not
make. The archive's signature is its Sigstore attestation, which `gh
attestation verify` checks. Download the archive cargo-binstall installed,
for your version and target, from the GitHub Release, and verify it:

```sh
gh release download v<version> --repo goodfetch/goodfetch --pattern 'goodfetch-collie-<target>.tar.xz'
gh attestation verify goodfetch-collie-<target>.tar.xz --repo goodfetch/goodfetch
```
