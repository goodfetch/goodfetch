//! `goodfetch-filter-broker` (ADRs 0033 and 0039, issue 879): the minimal
//! Windows service that adds and removes persistent Windows Filtering Platform
//! (WFP) filters scoped to one live `AppContainer` at a time, behind a named
//! pipe only the installing user can open, verifying its caller against a
//! Goodfetch release attestation under its bundled Sigstore trust root.
//!
//! It links `goodfetch-confinement` (and `goodfetch-protocol`) and never
//! `goodfetch-engine`. Built once per release and shared by both Products.

pub mod broker;
pub mod events;
#[cfg(windows)]
pub mod win32;
