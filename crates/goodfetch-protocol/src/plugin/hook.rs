//! The Hook Plugin kind and the `hooks` setting (ADRs 0033, 0088 and 0104;
//! issue 954): process-Plugin (and Script) Hooks named in settings, ordered
//! layer by layer from Operator policy to the run layer, and the `hook.call`
//! primitive that runs a process-Plugin Hook at its stage.
//!
//! Every stage is named once, by its catalog `snake_case` name
//! ([`HookStageName`]), across manifests, the `hooks` setting, `hook.call`,
//! the Run trace and the SDKs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, PluginKind, PluginMethod, ProtocolVersion, Setting, Stability, ValueKind,
    ValueShape, schema_of,
};
use crate::page::PageId;
use crate::plugin::{HookStageName, Permissions};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Hook Plugin kind's method, its wire name with its kind's.
pub const HOOK_CALL: &str = "hook.call";

/// What `hook.call` sends to a process-Plugin Hook at a Collie page stage
/// (ADRs 0033, 0088 and 0104): the stage name and its `{ page, url, run }`
/// context, with no `request` or Label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookCallParams {
    /// The Hook stage running now, by its single catalog `snake_case` name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: HookStageName,
    /// The page being navigated, for the three page stages (`before_navigation`,
    /// `after_navigation`, `page_ready`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page: Option<PageId>,
    /// The navigation's URL, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The Run the Hook runs in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run: RunId,
}

/// What a process-Plugin Hook answers to `hook.call` at Collie's page stages:
/// nothing (`{}`), acting only through allowed `page.*` and `net.fetch` calls
/// back. Any unknown field is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookCallResult {}

/// One entry of the `hooks` setting (ADRs 0033, 0088 and 0104):
/// `{ name, stage, plugin | script, permissions? }`, with Foxhound's `labels`
/// and `datasets` accepted on Foxhound and refused on Collie with
/// `not_in_product` outside configuration files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookEntry {
    /// The Hook's unique name across every layer of a scope; a repeated name
    /// fails the scope's opening with `hook_duplicate`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The stage the Hook runs at, by its single catalog `snake_case` name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stage: HookStageName,
    /// The installed process Plugin implementing the `hook` kind (mutually
    /// exclusive with `script`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugin: Option<String>,
    /// An inline Script Hook path or source (mutually exclusive with `plugin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub script: Option<String>,
    /// Optional inline permissions declared for a Script Hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permissions: Option<Permissions>,
    /// Foxhound-only: crawl Labels this Hook is scoped to. Refused on Collie
    /// with `not_in_product`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub labels: Option<Vec<String>>,
    /// Foxhound-only: crawl Datasets this Hook is scoped to. Refused on Collie
    /// with `not_in_product`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub datasets: Option<Vec<String>>,
}

impl HookEntry {
    /// The fields of a `hooks` entry that only Foxhound honours (ADR 0088).
    pub const FOXHOUND_FIELDS: [&'static str; 2] = ["labels", "datasets"];

    /// Reads and validates one `hooks` entry from `value`.
    ///
    /// # Errors
    ///
    /// What is wrong with the entry, for a `setting_invalid` message.
    pub fn read(value: &Value) -> Result<Self, String> {
        let entry = Self::deserialize(value).map_err(|error| error.to_string())?;
        if entry.name.trim().is_empty() {
            return Err("a Hook entry's `name` must not be empty".to_owned());
        }
        match (entry.plugin.as_deref(), entry.script.as_deref()) {
            (Some(plugin), None) if !plugin.trim().is_empty() => {}
            (None, Some(script)) if !script.trim().is_empty() => {}
            (Some(_), Some(_)) => {
                return Err("a Hook entry sets either `plugin` or `script`, never both".to_owned());
            }
            _ => {
                return Err(
                    "a Hook entry must set either a non-empty `plugin` or `script`".to_owned(),
                );
            }
        }
        Ok(entry)
    }
}

/// The shape of one entry in the `hooks` setting.
pub const HOOK_ENTRY_SHAPE: ValueShape = ValueShape {
    name: "HookEntry",
    check: |value| HookEntry::read(value).map(drop),
    schema: || {
        let stages: Vec<Value> = HookStageName::COLLIE
            .iter()
            .map(|(stage, description)| {
                json!({
                    "const": stage.as_str(),
                    "description": description,
                })
            })
            .chain(std::iter::once(json!({
                "type": "string",
                "pattern": "^[a-z][a-z0-9_]*$",
                "description": "Another Product's or a newer engine's Hook stage name.",
            })))
            .collect();
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "stage"],
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The Hook's unique name across every layer of the scope.",
                },
                "stage": {
                    "anyOf": stages,
                    "description": "The Hook stage, by its single catalog `snake_case` name.",
                },
                "plugin": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The installed Hook Plugin name (mutually exclusive with `script`).",
                },
                "script": {
                    "type": "string",
                    "minLength": 1,
                    "description": "An inline Script Hook (mutually exclusive with `plugin`).",
                },
                "permissions": {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "Permissions declared for a Script Hook.",
                    "properties": {
                        "domains": {
                            "type": "array",
                            "items": { "type": "string" },
                        },
                        "secrets": {
                            "type": "array",
                            "items": { "type": "string" },
                        },
                        "grants": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["secret_value", "effects", "page_script", "session_credentials"],
                            },
                        },
                    },
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Foxhound only: crawl Labels this Hook is scoped to.",
                },
                "datasets": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Foxhound only: crawl Datasets this Hook is scoped to.",
                },
            },
        })
    },
    examples: || {
        vec![
            json!({
                "name": "cookie-setter",
                "stage": "before_navigation",
                "plugin": "site-cookies",
            }),
            json!({
                "name": "block-ads",
                "stage": "page_ready",
                "script": "hooks/block.js",
                "permissions": {
                    "domains": ["shop.example"],
                    "secrets": ["env:HOOK_TOKEN"],
                    "grants": ["page_script"],
                },
                "labels": ["listing"],
                "datasets": ["products"],
            }),
        ]
    },
};

const HOOK_ENTRY_KIND: ValueKind = ValueKind::Shape(&HOOK_ENTRY_SHAPE);

fn no_hooks() -> Value {
    json!([])
}

/// The `hooks` setting (ADRs 0033, 0088 and 0104): an ordered list of Hooks
/// appended layer by layer from Operator policy to the run layer.
pub const HOOKS: Setting = Setting {
    name: "hooks",
    description: "Hooks named in settings: each entry names `{ name, stage, plugin | script, permissions? }`. Layers append in order from Operator policy to the run layer, never removing an earlier layer's Hooks; a lock on `hooks` in Operator policy forbids adding more, and a repeated `name` fails with `hook_duplicate`.",
    kind: ValueKind::List(&HOOK_ENTRY_KIND),
    default: no_hooks,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Hook Plugin kind.
pub const HOOK: PluginKind = PluginKind {
    name: "hook",
    description: "Runs at a fixed stage of a navigation, a page or a Run, acting through allowed `page.*` and `net.fetch` calls back.",
    methods: &[PluginMethod {
        name: "call",
        description: "Runs the Hook for one stage invocation with its `{ page, url, run }` context.",
        params: schema_of::<HookCallParams>,
        result: schema_of::<HookCallResult>,
    }],
    chosen_by: None,
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// The `hooks` setting. The Hook kind itself is omitted from `plugin_kinds`
/// because a Hook's SDK form is a crawl's Hook in code rather than a generated
/// SDK-handler base class (ADR 0104).
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[HOOKS],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HOOK_ENTRY_SHAPE, HookEntry};
    use crate::plugin::HookStageName;

    #[test]
    fn hook_entry_accepts_plugin_or_script_and_rejects_neither_or_both() {
        let plugin = json!({
            "name": "set-cookie",
            "stage": "before_navigation",
            "plugin": "cookie-plugin",
        });
        let parsed = HookEntry::read(&plugin).unwrap();
        assert_eq!(parsed.stage, HookStageName::BEFORE_NAVIGATION);
        assert_eq!(parsed.plugin.as_deref(), Some("cookie-plugin"));

        let both = json!({
            "name": "both",
            "stage": "page_ready",
            "plugin": "a",
            "script": "b.js",
        });
        assert!(HookEntry::read(&both).is_err());

        let neither = json!({
            "name": "neither",
            "stage": "after_navigation",
        });
        assert!(HookEntry::read(&neither).is_err());

        for example in (HOOK_ENTRY_SHAPE.examples)() {
            (HOOK_ENTRY_SHAPE.check)(&example).unwrap();
        }
    }
}
