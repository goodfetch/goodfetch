//! The handles a program holds of Collie's values, which `xtask codegen`
//! writes from the catalog (ADR 0099): one form per declared kind, never
//! edited by hand. The crate re-exports them at its root; their blocking
//! twins, which it writes beside them, are [`crate::blocking`]'s.

pub(crate) mod handles;
