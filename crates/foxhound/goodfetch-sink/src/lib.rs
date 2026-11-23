//! Foxhound's Sinks: how a crawl's committed Records reach a system outside
//! the engine (ADR 0026). The delivery contract says what every Sink is
//! given and answers, the delivery driver holds what every Sink shares
//! (ADR 0129: order by record number per Dataset, the delivery ID and run
//! epoch of ADR 0008, the retry schedule and what each failure leads to),
//! and each Sink fills only the primitives `probe`, `write` and
//! `map_failure`. The in-binary SQLite Sink is the first.

mod contract;
mod driver;
mod sqlite;

pub use contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy,
    Written,
};
pub use driver::{Driver, Status};
pub use sqlite::SqliteSink;
