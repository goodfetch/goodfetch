---
title: Install
description: Install Collie's Rust crate and its executable, the tools a build from source needs, and the way to the release's signed archive.
products: [collie]
---

# Install

## Rust

The Rust SDK is the `goodfetch-collie` crate:

```sh
cargo add goodfetch-collie
```

### What a build from source needs

Cargo builds the crate from source on your machine, and Collie's engine
compiles BoringSSL and generates its bindings there (ADR 0124). The build
machine needs:

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

### The executable

The executable is behind the crate's `executable` feature, so that a program
embedding the crate never compiles it. This line builds it from source:

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
