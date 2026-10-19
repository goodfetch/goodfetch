//! The conformance harness (ADRs 0032, 0033, 0063 and 0069): it runs a
//! suite against a target and a running Proving ground and writes one
//! `conformance` result document, and its writer turns a case log another
//! runner wrote into the same document. Every outcome comes from
//! `goodfetch-protocol`'s constructor, so a capability the suite did not
//! reach is `unproved`, never `proved`.
//!
//! It depends on no Goodfetch crate but `goodfetch-protocol`: it tests
//! through the interface, never engine internals. Its first suite, the
//! self-test, proves the Proving ground's own oracle against known-right
//! and known-wrong reference results.

mod case_list;
mod writer;

pub use case_list::{CaseList, DeclaredCapability, DeclaredCase};
pub use writer::{CaseLogError, WriteError, read_case_log, write_document};
