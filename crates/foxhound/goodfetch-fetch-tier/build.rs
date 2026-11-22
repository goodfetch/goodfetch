//! Sets `goodfetch_callgrind_names` on this crate alone when
//! `GOODFETCH_CALLGRIND_NAMES` is set, as `xtask counts --valgrind` sets it
//! on the build it reads `E1.handled_page_wire_share` in: the wire's
//! functions are never inlined there, so that Callgrind names them. The
//! cfg's other reader, `goodfetch-engine`, has the same script. The
//! variable enters this crate's fingerprint, so Cargo never reuses a build
//! of the other kind. It downloads nothing (STYLE.md).
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment"
)]

/// The variable `xtask counts` sets, named after the cfg.
const VARIABLE: &str = "GOODFETCH_CALLGRIND_NAMES";

fn main() {
    println!("cargo::rerun-if-env-changed={VARIABLE}");
    if std::env::var_os(VARIABLE).is_some() {
        println!("cargo::rustc-cfg=goodfetch_callgrind_names");
    }
}
