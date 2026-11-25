//! Plugin confinement (ADR 0033): the policy an instance of a process Plugin
//! runs under, generated from its approved manifest; the fixed deny list of
//! paths no Plugin is ever granted; the seccomp lists per network level; and
//! the probe tables that test each rule (ADR 0067).
//!
//! The engine starts instances through this crate, and the
//! `goodfetch-sandbox` helper and the filter broker link it too, so it
//! depends on nothing of the engine (spec #596).

pub mod deny_list;
pub mod policy;
