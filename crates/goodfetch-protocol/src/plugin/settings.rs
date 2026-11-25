//! Each Plugin's settings (ADR 0033, issue 861): the `plugins` setting,
//! which gives a Plugin, by its name, the fields its manifest's
//! `settings_schema` declares, as `plugins.<name>.settings`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The keyword of a Plugin's settings schema that marks a field secret,
/// `"secret": true`, as Airbyte's connector specifications mark one
/// `airbyte_secret`: the field takes only a Secret reference.
pub const SECRET_KEYWORD: &str = "secret";

/// A Plugin's entry in `plugins`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    /// Its settings, which its manifest's `settings_schema` validates.
    #[serde(default)]
    pub settings: Map<String, Value>,
}

impl PluginEntry {
    /// The entry `value` gives.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message.
    pub fn read(value: &Value) -> Result<Self, String> {
        Self::deserialize(value).map_err(|error| error.to_string())
    }
}

/// A Plugin's entry, as `plugins` maps each name to.
pub const PLUGIN_ENTRY_SHAPE: ValueShape = ValueShape {
    name: "PluginEntry",
    check: |value| PluginEntry::read(value).map(drop),
    schema: || {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "settings": {
                    "type": "object",
                    "description": "The Plugin's settings, which its manifest's `settings_schema` validates.",
                },
            },
        })
    },
    examples: || {
        vec![
            json!({ "settings": { "language": "fr", "token": "env:EXTRACTOR_TOKEN" } }),
            json!({}),
        ]
    },
};

fn no_plugin() -> Value {
    json!({})
}

/// `plugins` (ADR 0033).
pub const PLUGINS: Setting = Setting {
    name: "plugins",
    description: "Each Plugin's settings, by its name, as `plugins.<name>.settings`: the fields its manifest's `settings_schema` declares, which the engine checks as it starts the Plugin, a value the schema refuses being `setting_invalid` naming its key, as `plugins.<name>.settings.<field>`, and a field left out taking the schema's `default`. A field the schema marks `\"secret\": true` takes only a Secret reference, such as `env:NAME`, a value given in its place being refused with `setting_invalid`; the engine resolves it, and passes it into the Plugin's environment only for a Plugin whose manifest's `permissions.grants` holds `secret_value`. The nearest layer's entry for a Plugin wins, and a Plugin it leaves out keeps the entry beneath. The engine reads a Plugin's settings from the Run's settings as the first call naming the Plugin starts it, and they hold for the rest of the Run: a page's value changes nothing. None by default. `config explain` returns each installed Plugin's schema.",
    kind: ValueKind::Map(&ValueKind::Shape(&PLUGIN_ENTRY_SHAPE)),
    default: no_plugin,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
