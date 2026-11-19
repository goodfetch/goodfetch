//! Foxhound, Goodfetch's Product for crawling, which contains Collie: the
//! crate published as Foxhound's Rust SDK, which holds Foxhound's executable
//! crate (ADRs 0087, 0090 and 0094). Its library exposes Foxhound's
//! composition of the engine: Collie's, then Foxhound's own contributions.
//! Its own crate, since it is a separate executable (STYLE.md, reason 1).
//!
//! As the SDK, a connection is the Product: [`Foxhound::connect`] opens one
//! to a linked Foxhound engine of its own, which runs in the program's
//! process, [`Foxhound::scope`] opens one around a body and closes it, and
//! [`blocking::Foxhound`] does the same for a program without async. Its
//! handles are cheap clones, `Send + Sync + 'static`.
//!
//! Which crate holds which type (ADR 0117): this crate holds [`Foxhound`]
//! and its composition; `goodfetch-collie-client` holds every value a
//! connection hands out, [`Run`], [`Session`], [`Page`], [`Locator`],
//! [`Error`] and [`Connection`], the trait both Products' connections
//! implement, which a helper written for either Product takes (ADRs 0095
//! and 0099), which this crate re-exports.
//!
//! ```
//! use goodfetch_foxhound::{ConnectOptions, Foxhound};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let home = tempfile::tempdir()?;
//! # let options = ConnectOptions::default()
//! #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
//! # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
//! let report = Foxhound::scope(options, async |foxhound| {
//!     foxhound.start().await?.finish().await
//! })
//! .await?;
//! println!("{} ended {:?}", report.run_id, report.lifecycle);
//! # Ok::<(), goodfetch_foxhound::Error>(())
//! # })?;
//! # Ok(())
//! # }
//! ```
#![warn(missing_docs)]

pub mod blocking;
mod composition;
#[cfg(feature = "executable")]
pub mod executable;
mod foxhound;
mod sample;

pub use composition::{composition, composition_with};
pub use foxhound::{Foxhound, PAID_FETCHES};
pub use goodfetch_collie::ConnectOptions;
pub use goodfetch_collie_client::{
    BoxError, Call, Connection, Error, ErrorCode, ErrorDetails, EventStream, Exchange, FieldValues,
    Limits, Locator, Money, MoneyError, Page, Query, Response, Run, Session, Start, UsageKind,
    events,
};
pub use goodfetch_protocol::page::events::PageRequest;
pub use goodfetch_protocol::page::read::{
    FieldLocator, FieldMatch, FieldTransform, ListField, PageField, PageRead, RowsField,
    UnreadField, UnreadReason, ValueField,
};
pub use goodfetch_protocol::run::{RunId, RunReport, StartedAs};
