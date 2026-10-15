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
//! both Products. Each Product's crate holds its connection type and implements the trait for it.

use std::fmt;
use std::future::IntoFuture;
use std::time::Duration;

use crate::lifecycle::{EngineInfo, Page, Run, Session, Start};
use crate::page::Locator;
use crate::results::Error;
use crate::run::RunView;
use crate::sessions::Identities;
use crate::settings::{ExplainedSetting, SettingKey};
use crate::time::{Call, CallFuture};

/// A JSON object of the wire, in snake_case, as the raw call's parameters.
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

/// One raw catalog call (G8), not sent until awaited. Its parameters are optional, so they are a
/// builder method, as every optional parameter is in Rust (ADR 0099).
#[must_use = "a call does nothing until it is awaited"]
pub struct RawCall {
    _private: (),
}

impl fmt::Debug for RawCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawCall(..)")
    }
}

impl RawCall {
    /// The operation's parameters: the wire's JSON, in snake_case.
    pub fn params(self, params: JsonObject) -> Self {
        let _ = params;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }
}

impl IntoFuture for RawCall {
    type Output = Result<serde_json::Value, Error>;
    type IntoFuture = CallFuture<serde_json::Value>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Any connection: what `goodfetch_collie::Collie` and `goodfetch_foxhound::Foxhound` share (ADRs
/// 0095, 0099), Collie's Run start included, so code written for both Products takes
/// `&impl Connection`.
pub trait Connection {
    /// The engine reached, fixed when the connection opened.
    fn engine(&self) -> &EngineInfo;

    /// The install's Saved identities (ADR 0092).
    fn identities(&self) -> Identities;

    /// The raw catalog call (G8): an operation this SDK does not generate, such as one a newer
    /// engine minor added. Its parameters name every value themselves and are the wire's JSON; so
    /// is the result. The call's timeout and cancellation apply, failures are the same
    /// [`crate::Error`] (an unknown code is kept with its group), nothing is retried, the engine
    /// validates the parameters strictly, and the Run trace records the call, marked raw. It
    /// returns the result only.
    fn call(&self, operation: &str) -> RawCall;

    /// The connection's effective settings, with the layer that set each.
    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>>;

    /// Closes the connection for every clone. Idempotent.
    fn close(&self) -> Call<()>;

    /// Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
    ///
    /// Starts a Run and returns once it is started (ADR 0102), as each connection's own `start`.
    fn start(&self) -> Start;

    /// Opens a Run by its Run ID to read it, never to drive it (ADR 0102).
    fn open_run(&self, run_id: &str) -> Call<RunView>;
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
                pub fn call(&self, operation: &str) -> RawCall {
                    let _ = operation;
                    unimplemented!()
                }
            }
        )*
    };
}

value_call!(Run => "run", Session => "session", Page => "page", Locator => "locator");
