//! Foxhound's testing kit (ADR 0105), behind the `test-util` feature, which
//! builds test values alone (ADR 0087): the client crate's [`FakeError`],
//! which builds `Error::fake`, the error the engine would answer with one
//! of either Product's codes.
//!
//! ```
//! use goodfetch_foxhound::testing::FakeError;
//! use goodfetch_foxhound::{Error, error_code};
//!
//! let error = Error::fake(error_code::RESPONSE_UNAVAILABLE, "the site answered 503");
//! assert!(error.retryable());
//! ```

pub use goodfetch_collie_client::testing::FakeError;
