//! Each Plugin's settings (ADR 0033, issue 861): the `plugins` setting,
//! which gives a Plugin, by its name, the fields its manifest's
//! `settings_schema` declares, as `plugins.<name>.settings`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::catalog::{Authority, PriceValue, ProtocolVersion, Setting, ValueKind, ValueShape};
use crate::plugin::approvals::Approvals;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The keyword of a Plugin's settings schema that marks a field secret,
/// `"secret": true`, as Airbyte's connector specifications mark one
/// `airbyte_secret`: the field takes only a Secret reference.
pub const SECRET_KEYWORD: &str = "secret";

/// An operator's price override for one declared spend unit of a Plugin
/// (`plugins.<name>.spend.<unit>`, Operator policy only; ADR 0033, issue 3677).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpendOverride {
    /// The price per unit, such as `0.005 USD`, replacing the manifest's
    /// declared price (or supplying one for an unpriced unit) in currency
    /// Limit reservations and settlements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
}

/// A Plugin's entry in `plugins`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    /// Its settings, which its manifest's `settings_schema` validates.
    #[serde(default)]
    pub settings: Map<String, Value>,
    /// Whether the Operator policy allows it to load (`true`) or denies it
    /// (`false`); left out, only a first-party Plugin loads. Only an
    /// Operator policy sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<bool>,
    /// The digest the Operator policy pins it to, `sha256:<hex>`: a Plugin
    /// of another digest is refused. Only an Operator policy sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// The permissions the Operator policy approved for it, each on its
    /// own. Only an Operator policy sets it.
    #[serde(default, skip_serializing_if = "Approvals::is_empty")]
    pub approved: Approvals,
    /// Operator price overrides for the Plugin's declared spend units, by
    /// unit name (`plugins.<name>.spend.<unit>.price`). Only an Operator
    /// policy sets it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spend: BTreeMap<String, SpendOverride>,
}

impl PluginEntry {
    /// The fields only an Operator policy sets, which a nearer layer's
    /// entry for the same Plugin keeps.
    pub const POLICY_FIELDS: [&'static str; 4] = ["allow", "digest", "approved", "spend"];

    /// The entry `value` gives.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message.
    pub fn read(value: &Value) -> Result<Self, String> {
        let entry = Self::deserialize(value).map_err(|error| error.to_string())?;
        for (unit, override_entry) in &entry.spend {
            if let Some(price) = override_entry.price.as_deref()
                && PriceValue::parse(price).is_none()
            {
                return Err(format!(
                    "spend.{unit}.price must be a price such as `0.005 USD`, got `{price}`"
                ));
            }
        }
        Ok(entry)
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
                "allow": {
                    "type": "boolean",
                    "description": "Whether the Operator policy allows the Plugin to load (`true`) or denies it (`false`); left out, only a first-party Plugin loads. Only an Operator policy sets it.",
                },
                "digest": {
                    "type": "string",
                    "description": "The digest the Operator policy pins the Plugin to, `sha256:<hex>`: a Plugin of another digest is refused. Only an Operator policy sets it.",
                },
                "approved": {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "The permissions the Operator policy approved for the Plugin, each on its own. Only an Operator policy sets it.",
                    "properties": {
                        "effects": approved_names("The domains it may cause Effects on."),
                        "page_script": approved_names("The domains whose pages it may run script in."),
                        "session_credentials": approved_names("The domains whose logged-in Session's credentials it may use."),
                        "secrets": approved_names("The Secret references it may use."),
                        "secret_values": approved_names("The Secret references whose value it may receive."),
                        "browsers": approved_names("The digests of the browser downloads it may add."),
                        "direct": {
                            "type": "boolean",
                            "description": "Whether it may reach the network directly.",
                        },
                    },
                },
                "spend": {
                    "type": "object",
                    "description": "Operator price overrides for the Plugin's declared spend units, by unit name (`plugins.<name>.spend.<unit>.price`). Only an Operator policy sets it.",
                    "additionalProperties": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "price": {
                                "type": "string",
                                "description": "Price per unit, such as `0.005 USD`, replacing the manifest's declared price.",
                            },
                        },
                    },
                },
            },
        })
    },
    examples: || {
        vec![
            json!({ "settings": { "language": "fr", "token": "env:EXTRACTOR_TOKEN" } }),
            json!({
                "allow": true,
                "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "approved": {
                    "effects": ["shop.example"],
                    "page_script": ["shop.example"],
                    "session_credentials": ["shop.example"],
                    "secrets": ["env:SHOP_TOKEN"],
                    "secret_values": ["env:SHOP_TOKEN"],
                    "browsers": ["sha256:0000000000000000000000000000000000000000000000000000000000000000"],
                    "direct": true,
                },
                "spend": {
                    "solves": { "price": "0.005 USD" },
                },
            }),
            json!({ "allow": false }),
            json!({}),
        ]
    },
};

/// One of `approved`'s sets of domains, Secret references or digests,
/// described by `description`.
fn approved_names(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description,
    })
}

fn no_plugin() -> Value {
    json!({})
}

/// `plugins` (ADR 0033).
pub const PLUGINS: Setting = Setting {
    name: "plugins",
    description: "Each Plugin's settings, by its name, as `plugins.<name>.settings`: the fields its manifest's `settings_schema` declares, which the engine checks as it starts the Plugin, a value the schema refuses being `setting_invalid` naming its key, as `plugins.<name>.settings.<field>`, and a field left out taking the schema's `default`. A field the schema marks `\"secret\": true` takes only a Secret reference, such as `env:NAME`, a value given in its place being refused with `setting_invalid`; the engine resolves it, and passes it into the Plugin's environment only for a Plugin whose manifest's `permissions.grants` holds `secret_value`. The nearest layer's entry for a Plugin wins, and a Plugin it leaves out keeps the entry beneath. The engine reads a Plugin's settings from the Run's settings as the first call naming the Plugin starts it, and they hold for the rest of the Run: the setting is fixed for the Run's life, a start under the Run's ID that changes it being refused, and a page's value changes nothing. None by default. `config explain` returns each installed Plugin's schema. An Operator policy's entry also says whether the Plugin loads, as `plugins.<name>.allow` and `plugins.<name>.digest`, and approves each permission its manifest asks for on its own, as `plugins.<name>.approved`: a Plugin that is not first-party loads only when `allow` is `true`, `false` refuses any Plugin, and a pin refuses another digest, each with `plugin_not_allowed`; a call that needs a permission not approved fails with `plugin_permission_denied`. Those three fields are set in an Operator policy only, a lower layer that sets them being refused with `setting_not_applicable`, and a lower layer's entry for the Plugin keeps them. `goodfetch plugin add` writes them after a person approves the Plugin.",
    kind: ValueKind::Map(&ValueKind::Shape(&PLUGIN_ENTRY_SHAPE)),
    default: no_plugin,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
