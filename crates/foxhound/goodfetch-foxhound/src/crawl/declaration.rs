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
    wire: WireLabel,
    settings: Settings,
    payload: PhantomData<fn() -> P>,
    borrows: PhantomData<&'a ()>,
}

impl<P> LabelDeclaration<'_, P> {
    /// The Label `name`, which takes any Payload of type `P` and its Run's
    /// settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            wire: WireLabel::default(),
            settings: Settings::new(),
            payload: PhantomData,
            borrows: PhantomData,
        }
    }

    /// Checks each Payload of the Label against `schema`: the engine
    /// refuses one that does not fit it with `payload_invalid`, at a seed
    /// or an enqueue (ADR 0082).
    #[must_use]
    pub fn payload(mut self, schema: &Schema<P>) -> Self
    where
        P: Serialize + DeserializeOwned,
    {
        self.wire.payload = Some(schema.wire_schema().clone());
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

    /// The Label's name, and what `crawl.start` declares it with.
    ///
    /// # Errors
    ///
    /// `setting_not_applicable` for the first key of its settings a Label
    /// does not set.
    pub(super) fn into_parts(self) -> Result<(String, WireLabel), Error> {
        let Self {
            name,
            mut wire,
            settings,
            payload: _,
            borrows: _,
        } = self;
        wire.settings = settings.within(SettingScope::Label)?;
        Ok((name, wire))
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
            .field("payload", &self.wire.payload)
            .field("settings", &self.settings)
            .finish()
    }
}
