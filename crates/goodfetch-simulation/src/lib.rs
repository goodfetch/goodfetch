//! The Simulation (ADR 0060): hosts that run the engine's own code on
//! turmoil's simulated network and paused clocks, every source of chance
//! drawn from one seed, so a failing run replays exactly.
//!
//! turmoil's network reaches the engine through `transport`'s in-memory
//! network, the one a test already hands it: [`listen`] feeds a Daemon's
//! listener with the connections turmoil accepts, and [`connect`] answers
//! the Client's end of a Connection turmoil carries. No product code
//! changes, and none of this builds without `--cfg tokio_unstable`, under
//! which turmoil seeds each host's runtime too; the Products' `cargo-deny`
//! bans the crate from their shipped graphs (ADRs 0087 and 0125).
//!
//! One seed derives the run's streams, each its own: turmoil's network,
//! the Fault plan generator's, and each host's, from which every boot of
//! that host draws a fresh `Entropy` ([`Simulation`]).

#![cfg(tokio_unstable)]

mod harness;
mod network;
mod seeds;

pub use harness::{Fault, Host, Injection, Simulation};
pub use network::{connect, listen};
/// The version of turmoil the Simulation pins, for scenarios that inject
/// its faults and read its time.
pub use turmoil;
