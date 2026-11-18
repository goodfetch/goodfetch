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

## The executable

The executable is behind the `executable` feature, so that a program
embedding the crate never compiles it:

```sh
cargo install goodfetch-collie --locked --features executable
```

`cargo binstall goodfetch-collie` installs the release's own archive instead,
without compiling.
