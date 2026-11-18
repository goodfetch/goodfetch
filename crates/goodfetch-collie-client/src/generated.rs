//! The handles a program holds of Collie's values and their blocking
//! twins, which `xtask codegen` writes from the catalog (ADR 0099): one
//! form per declared kind, never edited by hand. The crate re-exports them
//! at its root and in [`crate::blocking`].

pub(crate) mod blocking;
pub(crate) mod handles;
