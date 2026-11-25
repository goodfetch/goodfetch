//! The names of Plugin kinds and Hook stages as open value types (ADR
//! 0099): each Collie value is an associated constant, marked `collie` by
//! being defined here (ADR 0095), and a name this program does not know,
//! such as Foxhound's `sink` or a newer engine's, stays a value. A
//! manifest, a `hello` and the `hooks` setting spell each by these names
//! (ADR 0104).

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};

use crate::inline_value::open_value_type;

open_value_type!(
    /// A Plugin kind's `snake_case` name, such as `hook` or `model`.
    PluginKindName,
    "a Plugin kind's name"
);

impl PluginKindName {
    /// A Hook, run at a fixed stage of a navigation, a page or a Run.
    pub const HOOK: Self = Self::from_static("hook");
    /// A Browser backend, which supplies a Session's browser.
    pub const BROWSER: Self = Self::from_static("browser");
    /// An Extractor, which turns a page into its main content.
    pub const EXTRACT: Self = Self::from_static("extract");
    /// A Model provider, which answers model calls (ADR 0101).
    pub const MODEL: Self = Self::from_static("model");
    /// A Decision provider, which picks one of several options.
    pub const DECISION: Self = Self::from_static("decision");
    /// A Secret provider, which resolves the references of its scheme.
    pub const SECRET: Self = Self::from_static("secret");
    /// A Challenge solver.
    pub const CHALLENGE: Self = Self::from_static("challenge");
    /// A Proxy provider.
    pub const PROXY: Self = Self::from_static("proxy");
    /// A Search provider.
    pub const SEARCH: Self = Self::from_static("search");
    /// A Code source, which reads a one-time code for a login.
    pub const CODE: Self = Self::from_static("code");
    /// A Notification channel.
    pub const NOTIFICATION: Self = Self::from_static("notification");

    /// Collie's kinds, each with what its implementations do.
    pub const COLLIE: [(Self, &'static str); 11] = [
        (
            Self::HOOK,
            "A Hook, run at a fixed stage of a navigation, a page or a Run.",
        ),
        (
            Self::BROWSER,
            "A Browser backend, which supplies a Session's browser.",
        ),
        (
            Self::EXTRACT,
            "An Extractor, which turns a page into its main content.",
        ),
        (Self::MODEL, "A Model provider, which answers model calls."),
        (
            Self::DECISION,
            "A Decision provider, which picks one of several options.",
        ),
        (
            Self::SECRET,
            "A Secret provider, which resolves the references of its scheme.",
        ),
        (Self::CHALLENGE, "A Challenge solver."),
        (Self::PROXY, "A Proxy provider."),
        (Self::SEARCH, "A Search provider."),
        (
            Self::CODE,
            "A Code source, which reads a one-time code for a login.",
        ),
        (Self::NOTIFICATION, "A Notification channel."),
    ];
}

impl JsonSchema for PluginKindName {
    fn schema_name() -> Cow<'static, str> {
        "PluginKindName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        open_schema(
            "A Plugin kind's `snake_case` name: one of Collie's, or another Product's or a newer engine's, which a reader keeps as received.",
            &Self::COLLIE.map(|(kind, description)| (kind.as_str().to_owned(), description)),
        )
    }
}

open_value_type!(
    /// A Hook stage's `snake_case` name, such as `page_ready`.
    HookStageName,
    "a Hook stage's name"
);

impl HookStageName {
    /// Before a main-frame navigation is sent.
    pub const BEFORE_NAVIGATION: Self = Self::from_static("before_navigation");
    /// Once a navigation's response arrived, before the engine's own page
    /// work.
    pub const AFTER_NAVIGATION: Self = Self::from_static("after_navigation");
    /// At a navigation's `load`.
    pub const PAGE_READY: Self = Self::from_static("page_ready");
    /// Once a Run started; observe-only.
    pub const RUN_STARTED: Self = Self::from_static("run_started");
    /// Once a Checkpoint was taken; observe-only.
    pub const CHECKPOINT_TAKEN: Self = Self::from_static("checkpoint_taken");
    /// Once a Run moved to another Group member; observe-only.
    pub const RUN_MIGRATED: Self = Self::from_static("run_migrated");
    /// Once a Limit was reached; observe-only.
    pub const LIMIT_REACHED: Self = Self::from_static("limit_reached");
    /// Once a Run ended; observe-only.
    pub const RUN_ENDED: Self = Self::from_static("run_ended");

    /// Collie's stages, each with when its Hooks run.
    pub const COLLIE: [(Self, &'static str); 8] = [
        (
            Self::BEFORE_NAVIGATION,
            "Before a main-frame navigation is sent.",
        ),
        (
            Self::AFTER_NAVIGATION,
            "Once a navigation's response arrived, before the engine's own page work.",
        ),
        (Self::PAGE_READY, "At a navigation's `load`."),
        (Self::RUN_STARTED, "Once a Run started; observe-only."),
        (
            Self::CHECKPOINT_TAKEN,
            "Once a Checkpoint was taken; observe-only.",
        ),
        (
            Self::RUN_MIGRATED,
            "Once a Run moved to another Group member; observe-only.",
        ),
        (
            Self::LIMIT_REACHED,
            "Once a Limit was reached; observe-only.",
        ),
        (Self::RUN_ENDED, "Once a Run ended; observe-only."),
    ];
}

impl JsonSchema for HookStageName {
    fn schema_name() -> Cow<'static, str> {
        "HookStageName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        open_schema(
            "A Hook stage's `snake_case` name: one of Collie's, or another Product's or a newer engine's, which a reader keeps as received.",
            &Self::COLLIE.map(|(stage, description)| (stage.as_str().to_owned(), description)),
        )
    }
}

/// An open enumeration's schema (ADR 0079): a `oneOf` of its known values,
/// each a string constant with its description, marked `"open": true`, so
/// each SDK keeps a value it does not know.
fn open_schema(description: &str, values: &[(String, &str)]) -> Schema {
    let values: Vec<Value> = values
        .iter()
        .map(|(value, description)| {
            json!({ "type": "string", "const": value, "description": description })
        })
        .collect();
    json_schema!({
        "description": description,
        "open": true,
        "oneOf": values,
    })
}
