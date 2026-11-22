//! The Model provider interface (ADRs 0012, 0101 and 0125): the one call
//! every model call of the engine goes through, [`ModelProvider::call`],
//! and the [`CallContext`] it carries. It holds no adapter, retry, repair
//! or counting: the built-in adapters, a Plugin's provider and the Scripted
//! model each implement it, and the driver around it retries, repairs and
//! counts. Its own crate so the compiler enforces the direction (STYLE.md,
//! reason 2): the engine never depends on it, and the Scripted model, a
//! test adapter, takes the interface without the code ADR 0065 tests.

mod call;

pub use call::{CallContext, ModelProvider};
