//! The blocking twin.

pub use goodfetch_collie_client::blocking::Run;
pub use twins::Waited;

#[path = "generated/twins.rs"]
mod twins;

/// The connection without async.
pub struct Foxhound {
    inner: super::Foxhound,
}

impl Foxhound {
    /// Opens a connection.
    pub fn connect(inner: super::Foxhound) -> Self {
        Self { inner }
    }
}
