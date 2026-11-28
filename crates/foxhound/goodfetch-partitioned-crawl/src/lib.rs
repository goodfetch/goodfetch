//! Foxhound's Partitioned crawls across Group members (ADRs 0054, 0056, 0087,
//! 0088, 0095 and 0140):
//!
//! - [`rendezvous`]: Highest Random Weight (rendezvous) hashing of domain keys
//!   to Partitions, built once per `partition_map_epoch`.
//! - [`validation`]: Start-time refusals (`partitions_need_group`,
//!   `partitions_exceed_group_members`, `session_bound_to_machine`,
//!   `browser_borrowed`).
//! - [`forwarding`]: Credit-based at-least-once link forwarding over the peer
//!   link with `sender_run_epoch` and `partition_map_epoch`.
//! - [`aggregation`]: Lifecycle, Verdict, Completeness, Usage and crawl-part
//!   aggregation across Partitions.
//! - [`store`]: Lead Daemon coordination persistence and per-Partition
//!   run-state membership table for `LimitScope { kind: "partitioned_crawl" }`.
//! - [`coordinator`]: Engine contribution wiring `crawl.start`, `run.*`
//!   lifecycle/report/records guards, and mTLS peer-link extension handlers.

pub mod aggregation;
pub mod coordinator;
pub mod forwarding;
pub mod handoff;
pub mod rendezvous;
pub mod store;
pub mod validation;

pub use coordinator::contribution;
