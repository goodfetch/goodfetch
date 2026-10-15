//! Typed data: schemas, Records and Payloads (decided in #469).
//!
//! On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its
//! schemas as types deriving `schemars::JsonSchema` and serde's traits, using the [`schemars`] this
//! crate re-exports so the derive always matches the SDK's version, or as a JSON Schema document by
//! hand, whose values are [`serde_json::Value`], Rust's idiom for JSON that G7 allows (G8). A crate that does not depend on `schemars`
//! itself derives through the re-export with `use goodfetch_collie::schemars::{self, JsonSchema}`, or with
//! `#[schemars(crate = "goodfetch_collie::schemars")]`. `schemars`, serde and serde_json are in this crate's
//! public API, each naming exactly one major, so a new major of any is a new Goodfetch major
//! (C-STABLE, ADR 0030). A Rust type is both what a
//! program sends and what it receives.
//!
//! A custom `Deserialize` that returns an error is a rejection: an extraction sends it back to the
//! model for another attempt (ADR 0012), and any other call fails with it; the parse runs in the calling future, so a panic unwinds in the caller's own
//! call, and `schema_validator_failed` never reaches Rust.

use std::marker::PhantomData;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

pub use schemars;


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
