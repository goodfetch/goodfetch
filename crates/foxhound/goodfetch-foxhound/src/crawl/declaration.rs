//! A Label's and a Dataset's whole declarations (ADRs 0093, 0099 and
//! 0103): a Label's name, the JSON Schema its Payloads are checked against
//! and its own settings, which [`Crawl::label`](super::Crawl::label) takes
//! beside a bare name; a Dataset's name, the schema of its Records and its
//! key, which [`Crawl::dataset`](super::Crawl::dataset) takes beside a bare
//! name.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use goodfetch_collie_client::settings::Settings;
use goodfetch_collie_client::{Error, Schema, SchemaError};
use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration as WireLabel;
use goodfetch_protocol::foxhound::dataset::{
    DATASET_KEY_INVALID, DatasetDeclaration as WireDataset, DatasetKeyInvalidDetails, KeyRefused,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A Label as a crawl declares it (ADRs 0093 and 0099), whose requests
/// carry a Payload of type `P`: its name, and, through builder methods
/// named after TypeScript's options, the schema the engine checks each
/// Payload against (`payload`) and the settings its requests take over
/// the Run's (`settings`). [`Crawl::label`](super::Crawl::label) takes it,
/// or a bare name, which declares neither. Like `Crawl<'a>`, it is a
/// declaration whose `'a` is the one its crawl's functions may borrow
/// (ADR 0112).
///
/// ```
/// use goodfetch_foxhound::settings::{Settings, fetch};
/// use goodfetch_foxhound::{Crawl, LabelDeclaration, Schema, schemars};
///
/// #[derive(schemars::JsonSchema, serde::Serialize, serde::Deserialize)]
/// struct Book {
///     shelf: u32,
/// }
///
/// let mut crawl = Crawl::new();
/// let book = crawl.label(
///     LabelDeclaration::<Book>::new("book")
///         .payload(&Schema::derived())
///         .settings(Settings::new().set(fetch::TIER, fetch::Tier::Browser)),
/// )?;
/// assert_eq!(book.name(), "book");
/// # Ok::<(), goodfetch_foxhound::Error>(())
/// ```
pub struct LabelDeclaration<'a, P> {
    name: String,
    schema: Option<Schema<P>>,
    settings: Settings,
    borrows: PhantomData<&'a ()>,
}

impl<P> LabelDeclaration<'_, P> {
    /// The Label `name`, which takes any Payload of type `P` and its Run's
    /// settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            settings: Settings::new(),
            borrows: PhantomData,
        }
    }

    /// Checks each Payload of the Label against `schema`, such as
    /// [`Schema::derived`] or a JSON Schema written by hand for [`Value`]
    /// Payloads: the engine refuses one that does not fit it with
    /// `payload_invalid`, at a seed or an enqueue (ADR 0082), and the SDK
    /// refuses one that is not JSON, or does not read back as given,
    /// before anything is sent.
    ///
    /// [`Value`]: serde_json::Value
    #[must_use]
    pub fn payload(mut self, schema: &Schema<P>) -> Self {
        self.schema = Some(schema.clone());
        self
    }

    /// Gives the Label's requests `settings` over the Run's: keys whose
    /// scopes include a Label (ADR 0093), such as `fetch.tier` or
    /// `crawl.content_types`; any other fails the declaration with
    /// `setting_not_applicable`.
    #[must_use]
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// The Label's name, what `crawl.start` declares it with, and the
    /// schema its Payloads are sent by, if it has one.
    ///
    /// # Errors
    ///
    /// `setting_not_applicable` for the first key of its settings a Label
    /// does not set.
    pub(super) fn into_parts(self) -> Result<(String, WireLabel, Option<Schema<P>>), Error>
    where
        P: Serialize + DeserializeOwned,
    {
        let Self {
            name,
            schema,
            settings,
            borrows: _,
        } = self;
        let wire = WireLabel {
            payload: schema.as_ref().map(|schema| schema.wire_schema().clone()),
            settings: settings.within(SettingScope::Label)?,
        };
        Ok((name, wire, schema))
    }
}

impl<P> From<&str> for LabelDeclaration<'_, P> {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl<P> From<String> for LabelDeclaration<'_, P> {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl<P> fmt::Debug for LabelDeclaration<'_, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelDeclaration")
            .field("name", &self.name)
            .field("payload", &self.schema)
            .field("settings", &self.settings)
            .finish()
    }
}

/// How a Dataset with a schema writes a Record before a push: through the
/// schema's own encode, so one that is not JSON or does not read back as
/// given is refused before anything is sent.
pub(super) type RecordEncoder<I> = Arc<dyn Fn(&I) -> Result<Value, SchemaError> + Send + Sync>;

/// A Dataset as a crawl declares it (ADRs 0099 and 0103), whose Records
/// are of type `I`: its name, and, through builder methods named after
/// TypeScript's options, the schema the engine checks each Record against
/// (`schema`) and the top-level fields that identify one Record (`key`).
/// [`Crawl::dataset`](super::Crawl::dataset) takes it, or a bare name,
/// which declares neither: such a Dataset takes any JSON object.
///
/// ```
/// use goodfetch_foxhound::{Crawl, DatasetDeclaration, Schema, schemars};
///
/// #[derive(schemars::JsonSchema, serde::Serialize, serde::Deserialize)]
/// struct Book {
///     isbn: String,
///     title: Option<String>,
/// }
///
/// let mut crawl = Crawl::new();
/// let books = crawl.dataset(
///     DatasetDeclaration::<Book>::new("books")
///         .schema(&Schema::derived())
///         .key(&["isbn"]),
/// )?;
/// assert_eq!(books.name(), "books");
/// # Ok::<(), goodfetch_foxhound::Error>(())
/// ```
pub struct DatasetDeclaration<I> {
    name: String,
    schema: Option<(Value, RecordEncoder<I>)>,
    key: Vec<String>,
}

impl<I> DatasetDeclaration<I> {
    /// The Dataset `name`, which takes any Record of type `I` that writes
    /// as a JSON object, and has no key.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            key: Vec::new(),
        }
    }

    /// Identifies a Record of the Dataset by the top-level fields `key`,
    /// each one its schema requires: the engine refuses a Record that
    /// lacks one with `record_invalid`, and a Sink upserts by them.
    #[must_use]
    pub fn key(mut self, key: &[impl AsRef<str>]) -> Self {
        self.key = key.iter().map(|field| field.as_ref().to_owned()).collect();
        self
    }

    /// The Dataset's name, which the crawl checks is its only one.
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// What `crawl.start` declares the Dataset with, and how a push writes
    /// its Records, when it has a schema.
    ///
    /// # Errors
    ///
    /// `dataset_key_invalid` when its key names a field that is no
    /// top-level field its schema requires.
    pub(super) fn into_parts(self) -> Result<(WireDataset, Option<RecordEncoder<I>>), Error> {
        let Self { name, schema, key } = self;
        let (schema, encoder) = schema.unzip();
        if let Some(field) = schema
            .as_ref()
            .and_then(|schema| not_required(schema, &key))
        {
            return Err(Error::of_sdk(
                &DATASET_KEY_INVALID,
                &format!(
                    "the Dataset `{name}`'s key names `{field}`, which is no top-level field its schema requires"
                ),
                "Key a Dataset only by fields its schema requires, or require the field in its schema.",
                &DatasetKeyInvalidDetails {
                    dataset: name,
                    refused: KeyRefused::NotRequired {
                        field: field.to_owned(),
                    },
                },
            ));
        }
        Ok((WireDataset { name, schema, key }, encoder))
    }
}

impl<I: Serialize + DeserializeOwned + 'static> DatasetDeclaration<I> {
    /// Checks each Record of the Dataset against `schema`, such as
    /// [`Schema::derived`] or a JSON Schema written by hand for [`Value`]
    /// Records: the SDK refuses one that is not JSON, or that `I`'s own
    /// `Deserialize` does not read back as given, before anything is sent,
    /// and the engine one that does not fit its JSON Schema, each with
    /// `record_invalid`.
    #[must_use]
    pub fn schema(mut self, schema: &Schema<I>) -> Self {
        let wire = schema.wire_schema().clone();
        let schema = schema.clone();
        self.schema = Some((wire, Arc::new(move |record| schema.encode(record))));
        self
    }
}

/// The first field of `key` that the JSON Schema `schema` does not list
/// among its top-level `required` fields, as the engine refuses it.
fn not_required<'k>(schema: &Value, key: &'k [String]) -> Option<&'k str> {
    let required = schema.get("required").and_then(Value::as_array);
    key.iter()
        .find(|field| {
            !required.is_some_and(|required| {
                required
                    .iter()
                    .any(|name| name.as_str() == Some(field.as_str()))
            })
        })
        .map(String::as_str)
}

impl<I> From<&str> for DatasetDeclaration<I> {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl<I> From<String> for DatasetDeclaration<I> {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl<I> fmt::Debug for DatasetDeclaration<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatasetDeclaration")
            .field("name", &self.name)
            .field("schema", &self.schema.as_ref().map(|(wire, _)| wire))
            .field("key", &self.key)
            .finish()
    }
}
