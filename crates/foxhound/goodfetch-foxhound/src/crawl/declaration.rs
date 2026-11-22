//! A Label's whole declaration (ADRs 0093 and 0099): its name, the JSON
//! Schema its Payloads are checked against and its own settings, which
//! [`Crawl::label`](super::Crawl::label) takes beside a bare name.

use std::fmt;
use std::marker::PhantomData;

use goodfetch_collie_client::settings::Settings;
use goodfetch_collie_client::{Error, Schema};
use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration as WireLabel;
use serde::Serialize;
use serde::de::DeserializeOwned;

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
