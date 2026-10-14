//! Typed data: schemas, Records and Payloads (decided in #469).
//!
//! On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its
//! schemas as types deriving `schemars::JsonSchema` and serde's traits, using the [`schemars`] this
//! crate re-exports so the derive always matches the SDK's version, or as a JSON Schema document by
//! hand, whose values are [`serde_json::Value`], Rust's idiom for JSON that G7 allows (G8). A crate that does not depend on `schemars`
//! itself derives through the re-export with `use goodfetch::schemars::{self, JsonSchema}`, or with
//! `#[schemars(crate = "goodfetch::schemars")]`. `schemars`, serde and serde_json are in this crate's
//! public API, each naming exactly one major, so a new major of any is a new Goodfetch major
//! (C-STABLE, ADR 0030). A Rust type is both what a
//! program sends and what it receives.
//!
//! A custom `Deserialize` that returns an error is a rejection: an extraction sends it back to the
//! model for another attempt (ADR 0012), and any other call fails with it; the parse runs in the calling future, so a panic unwinds in the caller's own
//! call, and `schema_validator_failed` never reaches Rust.

use std::marker::PhantomData;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use schemars;

use crate::results::Error;

/// A schema the engine validates against, with the Rust type its values parse into.
#[derive(Debug, Clone)]
pub struct Schema<T> {
    _type: PhantomData<fn() -> T>,
}

impl<T: JsonSchema + DeserializeOwned> Schema<T> {
    /// The schema `schemars` derives for `T`.
    pub fn of() -> Self {
        unimplemented!()
    }
}

impl Schema<serde_json::Value> {
    /// A JSON Schema document written by hand: the library-free way down.
    pub fn from_json(document: serde_json::Value) -> Self {
        let _ = document;
        unimplemented!()
    }
}

/// A Run's Dataset of `T`, the type it was declared with, or [`serde_json::Map`] when it declares
/// none. The engine validates every Record against its JSON Schema; how a Dataset is declared, how
/// Records are read and what becomes of a rejected one are #479's. A cheap clone (ADR 0081).
#[derive(Debug)]
pub struct Dataset<T> {
    _type: PhantomData<fn(T) -> T>,
}

impl<T> Clone for Dataset<T> {
    fn clone(&self) -> Self {
        Self { _type: PhantomData }
    }
}

impl<T: Serialize> Dataset<T> {
    /// The Dataset's name.
    pub fn name(&self) -> &str {
        unimplemented!()
    }

    /// Provisional: push or pull is #479's.
    pub async fn push(&self, record: &T) -> Result<(), Error> {
        let _ = record;
        unimplemented!()
    }
}

/// A handle on a Label, carrying its Payload's type; `Label<()>` when it has none.
///
/// A program names the destination of an enqueue with this handle, never with a bare string, so a
/// misspelled Label or a wrong Payload is a compile error. How a crawl declares its Labels and
/// handlers is #477's.
#[derive(Debug)]
pub struct Label<P> {
    _payload: PhantomData<fn(P) -> P>,
}

impl<P> Label<P> {
    /// The Label's name.
    pub fn name(&self) -> &str {
        unimplemented!()
    }
}

impl<P> Clone for Label<P> {
    fn clone(&self) -> Self {
        unimplemented!()
    }
}

/// One problem a validator found, at a path inside the value.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SchemaIssue {
    /// What is wrong.
    pub message: String,
    /// Where, as field names and indexes.
    pub path: Vec<PathSegment>,
}

/// A step of a [`SchemaIssue`]'s path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// An object's field.
    Field(String),
    /// An array's index.
    Index(usize),
}

/// Details of `payload_invalid`: a Payload failed its Label's schema, was not JSON, or exceeded
/// `crawl.payload.max_size`. Returned by the enqueue call, or the outcome of a request whose
/// handler's parse rejected a Payload that a changed native rule no longer accepts.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PayloadInvalid {
    /// The Label enqueued to.
    pub label: String,
    /// Empty when the Payload was refused for its size; the error's `limit` then names
    /// `crawl.payload.max_size`.
    pub issues: Vec<SchemaIssue>,
    /// The Payload's size as serialized JSON.
    pub size_bytes: u64,
}

/// Details of `payload_schema_changed`: a resume found pending Payloads that fail their Label's
/// changed schema, and nothing ran. Fix the schema (a default for the new field) or start a new Run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PayloadSchemaChanged {
    /// The Label whose schema changed.
    pub label: String,
    /// Pending requests of this Label whose Payload fails its current schema.
    pub failing: u64,
    /// Pending requests of this Label.
    pub pending: u64,
    /// The first failing Payload's problem.
    pub first_issue: SchemaIssue,
}
