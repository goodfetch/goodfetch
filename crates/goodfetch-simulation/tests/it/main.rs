//! The Simulation harness's own scenarios, through turmoil's adapter of
//! `transport`. They compile only under `--cfg tokio_unstable`, in the
//! Simulation's job (`just simulate`, ADR 0060).

#[cfg(tokio_unstable)]
mod simulation;
