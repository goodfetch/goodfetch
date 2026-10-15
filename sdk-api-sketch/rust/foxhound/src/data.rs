//! Foxhound's typed data (ADR 0082, decided in #469): the handles on a crawl's Datasets and Labels,
//! and the Payload's Error details. Schemas are the Collie client crate's (ADR 0095).

use std::marker::PhantomData;
use crate::SchemaIssue;

/// A handle on a crawl's Dataset of `T`, the type it was declared with, or [`serde_json::Map`] when
/// it declares none. The engine validates every Record against its JSON Schema. A crawl declares it
/// with `Crawl::dataset`, a handler pushes with `ctx.push`, and a program reads with `run.records`
/// (ADR 0103, [`crate::output`]). A cheap clone (ADR 0081).
#[derive(Debug)]
pub struct Dataset<T> {
    _type: PhantomData<fn(T) -> T>,
}

impl<T> Clone for Dataset<T> {
    fn clone(&self) -> Self {
        Self { _type: PhantomData }
    }
}

impl<T> Dataset<T> {
    /// The Dataset's name.
    pub fn name(&self) -> &str {
        unimplemented!()
    }

    /// The fields that identify a Record (ADR 0103); empty when it declares none.
    pub fn key(&self) -> &[String] {
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

