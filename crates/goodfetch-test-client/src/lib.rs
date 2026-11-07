//! Goodfetch's protocol-aware Rust test helpers (ADR 0062's note of
//! 2026-11-07): the helpers that need Goodfetch's contract, which
//! `goodfetch-test-support` cannot take since it depends on no Goodfetch
//! crate. They are the test home's layout, the paths in a test's Goodfetch
//! home where the commit's executables keep their files, read from
//! `goodfetch-protocol`'s `home`; the robots.txt readings a Run's trace
//! records; and the one test Client stack every Product's tests share: the
//! `hello` built from the protocol's types, an Attached engine over stdio
//! and a Client Connection to a Daemon or a private endpoint. Only ever a
//! dev-dependency, never shipped, and never taken by `goodfetch-protocol`
//! or the Proving ground.

mod attached;
mod daemon_client;
mod hello;
mod home;
mod run_trace;

pub use attached::Attached;
pub use daemon_client::DaemonClient;
pub use hello::{Hello, hello, newer_minor};
pub use home::{HomeLayout, product_version};
pub use run_trace::{robots_txt_read, robots_txt_readings};
