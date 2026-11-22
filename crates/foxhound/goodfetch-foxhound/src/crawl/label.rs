//! A crawl's Labels (ADRs 0082, 0093 and 0099): what [`Crawl::label`]
//! declares, a name and the schema of its Payloads, and the [`Label`] it
//! returns, by which a seed, an enqueue and a handler send a Payload of
//! the Label's type, refused before anything is sent when its schema
//! refuses it.
//!
//! [`Crawl::label`]: super::Crawl::label

use std::fmt;
use std::sync::Arc;

use goodfetch_collie_client::schemars::JsonSchema;
use goodfetch_collie_client::{Error, Schema, SchemaError};
use goodfetch_protocol::foxhound::frontier::{
    PAYLOAD_INVALID, PayloadInvalidDetails, PayloadIssue,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::CrawlId;

/// What [`Crawl::label`] declares (ADRs 0082, 0093 and 0099): a Label's
/// name and the schema of its Payloads, of type `P`. A bare name declares
/// the Label with the schema `schemars` derives for `P`, on the side `P`
/// deserializes from; [`LabelDeclaration::payload`] gives a schema of its
/// own, such as a JSON Schema written by hand for [`Value`] Payloads.
///
/// ```
/// use goodfetch_foxhound::{Crawl, LabelDeclaration, Schema};
/// use serde_json::{Value, json};
///
/// # fn declare() -> Result<(), Box<dyn std::error::Error>> {
/// let mut crawl = Crawl::new();
/// let isbn = Schema::json(json!({
///     "type": "object",
///     "properties": { "isbn": { "type": "string" } },
///     "required": ["isbn"],
/// }))?;
/// let book = crawl.label(LabelDeclaration::<Value>::new("book").payload(isbn))?;
/// # let _ = book;
/// # Ok(())
/// # }
/// ```
///
/// [`Crawl::label`]: super::Crawl::label
pub struct LabelDeclaration<P> {
    pub(super) name: String,
    pub(super) payload: Schema<P>,
}

impl<P: JsonSchema + Serialize + DeserializeOwned> LabelDeclaration<P> {
    /// The Label `name`, whose Payloads the schema `schemars` derives for
    /// `P` types.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            payload: Schema::derived(),
        }
    }
}

impl<P> LabelDeclaration<P> {
    /// Types the Label's Payloads by `schema` in place of the one
    /// `schemars` derives.
    #[must_use]
    pub fn payload(self, schema: Schema<P>) -> Self {
        Self {
            payload: schema,
            ..self
        }
    }
}

impl<P: JsonSchema + Serialize + DeserializeOwned> From<&str> for LabelDeclaration<P> {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl<P> fmt::Debug for LabelDeclaration<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelDeclaration")
            .field("name", &self.name)
            .field("payload", &self.payload)
            .finish()
    }
}

/// A Label a crawl declares (ADR 0093), whose requests carry a Payload of
/// type `P`: what a seed, an enqueue and a handler name. Cloning it is
/// cheap.
pub struct Label<P> {
    pub(super) name: Arc<str>,
    pub(super) crawl: CrawlId,
    /// The schema its Payloads are sent by.
    pub(super) payload: Arc<Schema<P>>,
}

impl<P> Label<P> {
    /// The Label's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<P: Serialize + DeserializeOwned> Label<P> {
    /// `payload` as the JSON its schema sends, or the `payload_invalid`
    /// that refuses it before anything is sent.
    pub(super) fn encode(&self, payload: &P) -> Result<Value, Error> {
        self.payload
            .encode(payload)
            .map_err(|error| payload_refused(&self.name, &error))
    }
}

impl<P> Clone for Label<P> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            crawl: self.crawl,
            payload: Arc::clone(&self.payload),
        }
    }
}

impl<P> fmt::Debug for Label<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Label").field(&self.name).finish()
    }
}

/// The `payload_invalid` of a Payload for `label` its schema refused
/// before anything was sent: it is not JSON, or does not read back as
/// sent; the details name the issue's JSON Pointer, as the engine's own.
fn payload_refused(label: &str, error: &SchemaError) -> Error {
    let path = match error {
        SchemaError::NotJson { path, .. } | SchemaError::ChangesOnReadBack { path, .. } => {
            path.clone()
        }
        _ => String::new(),
    };
    Error::of_sdk(
        &PAYLOAD_INVALID,
        &format!("the Payload for the Label `{label}` was not sent: {error}"),
        "Give a Payload that is JSON and that its type's `Deserialize` reads back as it was given.",
        &PayloadInvalidDetails {
            label: label.to_owned(),
            issues: vec![PayloadIssue {
                path,
                message: error.to_string(),
            }],
            limit: None,
            size: None,
            max_size: None,
        },
    )
}
