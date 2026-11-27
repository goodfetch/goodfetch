//! Foxhound's Sinks: how a crawl's committed Records reach a system outside
//! the engine (ADR 0026). The delivery contract says what every Sink is
//! given and answers, the delivery driver holds what every Sink shares
//! (ADR 0129: order by record number per Dataset, the delivery ID and run
//! epoch of ADR 0008, the retry schedule and what each failure leads to),
//! and each Sink fills only the primitives `probe`, `write` and
//! `map_failure`. The in-binary SQLite Sink is the first. [`Sinks`] is the
//! crate's composition value, which the crawl's Run asks to check its
//! `sinks` at the start, to probe them against its
//! `crawl.min_sink_guarantee` and to deliver to them while it goes;
//! [`contribution`] answers `sink.probe`.

mod contract;
mod driver;
mod guarantee;
mod run;
mod sqlite;
mod sync;

pub use contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy,
    Written,
};
pub use driver::{Driver, Status};
pub use guarantee::{meets, require};
pub use run::{Delivery, Sinks, contribution};
pub use sqlite::SqliteSink;
