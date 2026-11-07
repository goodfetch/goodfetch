//! Goodfetch's protocol-aware Rust test helpers (ADR 0062's note of
//! 2026-11-07): the helpers that need Goodfetch's contract, which
//! `goodfetch-test-support` cannot take since it depends on no Goodfetch
//! crate. Its first are the test home's layout, the paths in a test's
//! Goodfetch home where the commit's executables keep their files, read from
//! `goodfetch-protocol`'s `home`, and the robots.txt readings a Run's trace
//! records. Only ever a dev-dependency, never shipped, and never taken by
//! `goodfetch-protocol` or the Proving ground.

mod home;
mod run_trace;

pub use home::{HomeLayout, product_version};
pub use run_trace::{robots_txt_read, robots_txt_readings};
