//! A Product crate of the test: its own types, a module of the blocking
//! twin, a re-export of another crate's type and a generated one.

pub mod blocking;
mod foxhound;
mod generated;
mod hidden;

pub use foxhound::{Foxhound, PAID_FETCHES};
pub use generated::Handle;
pub use goodfetch_collie_client::Run;

#[cfg(all(test, feature = "slow"))]
pub mod slow_tests {
    pub fn helper() {}
}

#[cfg(test)]
mod tests {
    pub fn helper() {}
}
