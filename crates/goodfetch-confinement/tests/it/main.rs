//! goodfetch-confinement's integration tests (Pre-merge): each committed
//! probe table run through the probe executable, unconfined, then confined
//! by this OS's applier (ADR 0067). The tables are measured on x86-64,
//! where Linux's runs; another architecture needs tables of its own.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod launch;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod probes;
