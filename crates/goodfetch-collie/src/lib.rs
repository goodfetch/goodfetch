//! Collie, Goodfetch's Product for browser automation: the crate published
//! as Collie's Rust SDK, which holds Collie's executable crate (ADRs 0087,
//! 0090 and 0094). Its own crate, since it is a separate executable
//! (STYLE.md, reason 1).
//!
//! As the SDK, a connection is the Product: [`Collie::connect`] opens one
//! to a linked engine of its own, which runs in the program's process,
//! [`Collie::scope`] opens one around a body and closes it, and
//! [`blocking::Collie`] does the same for a program without async. Its
//! handles are cheap clones, `Send + Sync + 'static`.
//!
//! ```
//! use goodfetch_collie::{Collie, ConnectOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let home = tempfile::tempdir()?;
//! # let options = ConnectOptions::default()
//! #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
//! # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
//! let report = Collie::scope(options, async |collie| {
//!     collie.start().await?.finish().await
//! })
//! .await?;
//! println!("{} ended {:?}", report.run_id, report.lifecycle);
//! # Ok::<(), goodfetch_collie::Error>(())
//! # })?;
//! # Ok(())
//! # }
//! ```
//!
//! Its library also exposes Collie's composition of the engine, which
//! Foxhound's starts from, the [`linked`] connection both Products' SDKs
//! open, and, under the `executable` feature, the executable's entry,
//! which both Products' `main` call. Under the `test-util` feature,
//! `testing` is Collie's testing kit: a real engine for a test, in a
//! temporary Goodfetch home (ADR 0105).
#![warn(missing_docs)]

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the pages read a cause and its rule; `blocked` carries the rest of the evidence (#711)"
    )
)]
mod block_detection;
pub mod blocking;
mod collie;
mod composition;
mod engine_choice;
#[cfg(feature = "executable")]
pub mod executable;
pub mod linked;
mod pins;
mod runtime;
mod sample;
#[cfg(feature = "test-util")]
pub mod testing;
mod trace_context;

pub use collie::Collie;
pub use composition::{composition, composition_with};
pub use engine_choice::{EngineChoice, EngineFallback, EngineInfo, EngineKind, RemoteDaemon};
pub use goodfetch_collie_client::{
    AiWorkResult, Answer, BoxError, Call, Connection, Ended, Error, ErrorCode, ErrorDetails,
    EventStream, Exchange, FieldValues, HandlerlessStart, KeyValueStore, Limits, Locator, Money,
    MoneyError, Page, PartialOutput, Query, RawCall, Response, Run, RunView, Schema, SchemaError,
    Session, Start, StoredBytes, UsageKind, events, schemars, settings,
};
pub use goodfetch_protocol::page::ai_work::{Claim, NotVerifiedDetails, Reported};
pub use goodfetch_protocol::page::events::PageRequest;
pub use goodfetch_protocol::page::read::{
    FieldLocator, FieldMatch, FieldTransform, ListField, PageField, PageRead, RowsField,
    UnreadField, UnreadReason, ValueField,
};
pub use goodfetch_protocol::run::{
    AxisReason, CatalogTarget, Recording, RecordingMode, RunId, RunReport, StartedAs, StopReason,
    Tags, Verdict,
};
pub use goodfetch_protocol::settings::{
    ExplainedSetting, RunOrigin, SettingsExplanation, SettingsLayer,
};
pub use linked::ConnectOptions;
pub use pins::pinned_artifacts;
