//! A fixture engine crate.

mod clock;
mod dispatch;
mod retry;

pub use clock::Tick;
pub use dispatch::dispatch;
