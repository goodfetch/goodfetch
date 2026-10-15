//! Mapping the catalog to each language and surface (ADR 0099, decided in #471).
//!
//! Every name is the catalog's canonical snake_case words: functions, methods, fields and modules
//! keep them, types are UpperCamelCase with an initialism as one word (`CdpSession`, `JsonValue`,
//! C-CASE), and setting keys are UPPER_SNAKE constants (ADR 0083). A keyword used as a name is a raw
//! identifier, the spelling rustc's own help leads to: `route.r#continue()`, `keyboard.r#type(…)`,
//! `mouse.r#move(…)`. Reads are methods, never `get_` (C-GETTER).
//!
//! [`Connection`] is the trait both Products' connections implement, declared once in
//! `goodfetch-collie-client` (ADR 0095). `Collie` and `Foxhound` keep their own methods of the same
//! names, so a program calls them without importing the trait; the trait serves code written for
//! both Products. The sketch's placeholder [`Goodfetch`] implements it until #483 replaces it.

use crate::lifecycle::{EngineInfo, Goodfetch, Page, Run, Session};
use crate::page::Locator;
use crate::sessions::Identities;
use crate::settings::{ExplainedSetting, SettingKey};
use crate::time::Call;

/// A JSON object of the wire, in snake_case, as the raw call's parameters.
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

/// Any connection: what `Collie` and `Foxhound` share. Collie's Run start joins it once #478 names it.
pub trait Connection {
    /// The engine reached, fixed when the connection opened.
    fn engine(&self) -> &EngineInfo;

    /// The install's Saved identities (ADR 0092).
    fn identities(&self) -> Identities;

    /// The raw catalog call (G8): an operation this SDK does not generate, such as one a newer
    /// engine minor added. `params` names every value itself and is the wire's JSON; so is the
    /// result. The call's timeout and cancellation apply, failures are the same [`crate::Error`]
    /// (an unknown code is kept with its group), nothing is retried, the engine validates `params`
    /// strictly, and the Run trace records the call, marked raw. It returns the result only.
    fn call(&self, operation: &str, params: JsonObject) -> Call<serde_json::Value>;

    /// The connection's effective settings, with the layer that set each.
    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>>;

    /// Closes the connection for every clone. Idempotent.
    fn close(&self) -> Call<()>;
}

impl Connection for Goodfetch {
    fn engine(&self) -> &EngineInfo {
        Goodfetch::engine(self)
    }

    fn identities(&self) -> Identities {
        Goodfetch::identities(self)
    }

    fn call(&self, operation: &str, params: JsonObject) -> Call<serde_json::Value> {
        Goodfetch::call(self, operation, params)
    }

    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        Goodfetch::explain_settings(self, keys)
    }

    fn close(&self) -> Call<()> {
        Goodfetch::close(self)
    }
}

impl Goodfetch {
    /// The raw catalog call on the connection, as [`Connection::call`].
    pub fn call(&self, operation: &str, params: JsonObject) -> Call<serde_json::Value> {
        let _ = (operation, params);
        unimplemented!()
    }
}

macro_rules! value_call {
    ($($value:ty => $namespace:literal),* $(,)?) => {
        $(
            impl $value {
                #[doc = concat!(
                    "The raw catalog call on this value: `operation` must be in the `", $namespace,
                    "` namespace, and the value fills the parameter of that name; any other operation ",
                    "fails with a `ProgramError` before it is sent. A closed value fails with `closed`."
                )]
                pub fn call(&self, operation: &str, params: JsonObject) -> Call<serde_json::Value> {
                    let _ = (operation, params);
                    unimplemented!()
                }
            }
        )*
    };
}

value_call!(Run => "run", Session => "session", Page => "page", Locator => "locator");
