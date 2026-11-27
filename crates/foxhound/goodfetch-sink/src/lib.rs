//! Foxhound's Sinks: how a crawl's committed Records reach a system outside
//! the engine (ADR 0026). The delivery contract says what every Sink is
//! given and answers, the delivery driver holds what every Sink shares
//! (ADR 0129: order by record number per Dataset, the delivery ID and run
//! epoch of ADR 0008, the retry schedule and what each failure leads to),
//! and each Sink fills only the primitives `probe`, `write`, `finish` and
//! `map_failure`; the driver also gives each Record its key under a keyed
//! write mode (ADR 0103). The in-binary SQLite Sink is the first, in every
//! write mode. [`Sinks`] is the crate's composition value, which the
//! crawl's Run asks to check its `sinks` at the start, to probe them
//! against its `crawl.min_sink_guarantee`, to deliver to them while it goes
//! and to finish them at its end; [`contribution`] answers `sink.probe`.

mod contract;
pub mod dead_letters;
mod driver;
pub mod files;
mod guarantee;
pub mod object_store_sink;
pub mod report;
mod run;
pub mod schedule;
pub mod schema;
mod sqlite;
mod sync;
pub mod throughput;
pub mod webhook;

pub use contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy,
    Written,
};
pub use driver::{Driver, RedeliverOutcome, RetryOutcome, Status};
pub use files::{
    DEFAULT_MAX_AGE, DEFAULT_MAX_BYTES, DEFAULT_MAX_RECORDS, FileDestination, FileProbe,
    FilesConfig, FilesError, FilesSink, ParsedPartKey, PutOutcome, format_part_key, parse_part_key,
    part_extension,
};
pub use guarantee::{meets, require};
pub use object_store_sink::{ObjectStoreBackend, ObjectStoreDestination, ObjectStoreError};
pub use report::{SinkReportPart, SinkStateRow};
pub use run::{Delivery, Sinks, contribution};
pub use schedule::{
    PAUSE_AFTER, PauseReason, RetryDecision, RetryState, SCHEDULE, SinkNotification,
    SinkNotificationEvent, next_retry,
};
pub use sqlite::{SqliteError, SqliteSink};
pub use throughput::{
    AdaptiveConcurrency, BatchPolicy, DEFAULT_WRITE_TIMEOUT, Preset, RateLimit, Throughput,
    TokenBucket, write_bounded,
};
pub use webhook::{DEFAULT_MAX_BATCH_BYTES, WebhookConfig, WebhookError, WebhookSink};
