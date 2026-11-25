//! Plugin confinement (ADR 0033): the policy an instance of a process Plugin
//! runs under, generated from its approved manifest; the fixed deny list of
//! paths no Plugin is ever granted; the seccomp lists per network level;
//! the probe tables that test each rule (ADR 0067); the engine's startup
//! probe of what the host allows, which tells whether a Plugin can be
//! confined at all (ADR 0049); and each OS's applier, Linux's so far.
//!
//! The engine starts instances through this crate, and the
//! `goodfetch-sandbox` helper and the filter broker link it too, so it
//! depends on nothing of the engine (spec #596).

pub mod deny_list;
pub mod host;
pub mod launch;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod policy;
pub mod probes;
pub mod seccomp;
