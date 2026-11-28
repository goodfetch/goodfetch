//! Continuous crawl and Revisit settings and types (ADR 0139; spec #1836).
//!
//! A crawl sets `crawl.continuous` to keep running when its Frontier empties
//! and revisit each URL according to its Label's `revisit.policy`.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ProtocolVersion, Setting, SettingJson, ValueKind, ValueShape, Yielding,
    shape_json,
};
use crate::foxhound::crawl::CRAWL_KEEP_ALIVE;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn no() -> Value {
    Value::Bool(false)
}

fn never_policy() -> Value {
    json!({ "kind": "never" })
}

fn conditional_signature() -> Value {
    Value::String("conditional+signature".to_owned())
}

fn skip_unchanged() -> Value {
    Value::String("skip".to_owned())
}

/// The values `revisit.change_check` takes (ADR 0139).
pub const REVISIT_CHANGE_CHECK_VALUES: &[&str] =
    &["conditional+signature", "conditional", "signature", "none"];

/// How a Continuous crawl checks whether a revisited URL changed
/// (`revisit.change_check`, ADR 0139).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RevisitChangeCheck {
    /// Send `If-None-Match` / `If-Modified-Since`; a 304 means unchanged,
    /// otherwise the Page signature decides.
    #[default]
    #[serde(rename = "conditional+signature")]
    ConditionalSignature,
    /// Send `If-None-Match` / `If-Modified-Since`; every 200 counts as changed.
    #[serde(rename = "conditional")]
    Conditional,
    /// Send no validators; compare the Page signature.
    #[serde(rename = "signature")]
    Signature,
    /// Send no validators and skip signature comparison; every revisit counts as changed.
    #[serde(rename = "none")]
    None,
}

impl RevisitChangeCheck {
    /// Its wire and setting name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConditionalSignature => "conditional+signature",
            Self::Conditional => "conditional",
            Self::Signature => "signature",
            Self::None => "none",
        }
    }

    /// Parses a [`RevisitChangeCheck`] from a setting string.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "conditional+signature" => Some(Self::ConditionalSignature),
            "conditional" => Some(Self::Conditional),
            "signature" => Some(Self::Signature),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Parses a [`RevisitChangeCheck`] from a JSON setting value.
    #[must_use]
    pub fn read(value: &Value) -> Option<Self> {
        value.as_str().and_then(Self::named)
    }

    /// Whether this mode sends conditional request validators on document tiers.
    #[must_use]
    pub const fn uses_conditional(self) -> bool {
        matches!(self, Self::ConditionalSignature | Self::Conditional)
    }

    /// Whether this mode compares the Page signature on a 200 response.
    #[must_use]
    pub const fn uses_signature(self) -> bool {
        matches!(self, Self::ConditionalSignature | Self::Signature)
    }
}

/// The values `revisit.on_unchanged` takes (ADR 0139).
pub const REVISIT_ON_UNCHANGED_VALUES: &[&str] = &["skip", "handler"];

/// What a Continuous crawl does when a revisit finds the page unchanged
/// (`revisit.on_unchanged`, ADR 0139).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevisitOnUnchanged {
    /// Run no handler, push no Record, and end the revisit `unchanged`.
    #[default]
    Skip,
    /// Run the handler anyway.
    Handler,
}

impl RevisitOnUnchanged {
    /// Its wire and setting name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Handler => "handler",
        }
    }

    /// Parses a [`RevisitOnUnchanged`] from a setting string.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "skip" => Some(Self::Skip),
            "handler" => Some(Self::Handler),
            _ => None,
        }
    }

    /// Parses a [`RevisitOnUnchanged`] from a JSON setting value.
    #[must_use]
    pub fn read(value: &Value) -> Option<Self> {
        value.as_str().and_then(Self::named)
    }
}

/// How a Continuous crawl schedules revisits of a URL (`revisit.policy`, ADR 0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RevisitPolicy {
    /// Never revisit the URL after its first visit, such as articles.
    Never,
    /// Revisit at a fixed interval, in whole milliseconds (at least 1).
    Every {
        /// Interval between revisits in milliseconds (at least 1).
        #[schemars(extend("arrived_in" = "0.1"))]
        interval: u64,
    },
}

impl SettingJson for RevisitPolicy {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

impl RevisitPolicy {
    /// Parses a [`RevisitPolicy`] from a JSON setting value.
    ///
    /// # Errors
    ///
    /// Returns a reason string when `value` is not a valid policy or has a zero interval.
    pub fn read(value: &Value) -> Result<Self, String> {
        let policy: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "`revisit.policy` must be `{{ \"kind\": \"never\" }}` or `{{ \"kind\": \"every\", \"interval\": <ms> }}`: {error}"
            )
        })?;
        match policy {
            Self::Never => Ok(policy),
            Self::Every { interval } if interval > 0 => Ok(policy),
            Self::Every { .. } => {
                Err("`revisit.policy` `every` interval must be at least 1 ms".to_owned())
            }
        }
    }
}

/// `revisit.policy`'s values: a [`RevisitPolicy`].
pub const REVISIT_POLICY_SHAPE: ValueShape = ValueShape {
    name: "RevisitPolicy",
    check: |value| RevisitPolicy::read(value).map(drop),
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<RevisitPolicy>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![
            json!({ "kind": "never" }),
            json!({ "kind": "every", "interval": 3_600_000 }),
        ]
    },
};

/// `crawl.continuous` (ADR 0139).
pub const CRAWL_CONTINUOUS: Setting = Setting {
    name: "crawl.continuous",
    description: "Whether a crawl's Run is a Continuous crawl (ADR 0139): it stays `running` when its Frontier empties, implies `crawl.keep_alive`, and revisits each URL when due under its Label's `revisit.policy` until `run.finish`, `run.stop`, an abort, a Limit or `timeout.run` ends it. Fixed for the Run's life: a resume that changes it is refused with `setting_not_applicable`. `false` by default.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: true,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `revisit.policy` (ADR 0139).
pub const REVISIT_POLICY: Setting = Setting {
    name: "revisit.policy",
    description: "How a Continuous crawl schedules revisits of each URL, set for the Run or for one of its Labels: `{ \"kind\": \"never\" }` for pages read once, or `{ \"kind\": \"every\", \"interval\": <ms> }` for a fixed interval in milliseconds. Accepted on a batch crawl, where `config explain` marks it inactive (`yields_to` `crawl.continuous`).",
    kind: ValueKind::Shape(&REVISIT_POLICY_SHAPE),
    default: never_policy,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn any_value(_: &Value) -> bool {
    true
}

fn is_false(value: &Value) -> bool {
    value == &Value::Bool(false)
}

fn is_true(value: &Value) -> bool {
    value == &Value::Bool(true)
}

/// `revisit.policy` yields to `crawl.continuous = false`: a batch crawl does
/// not revisit URLs, so `config explain` marks it inactive (ADR 0139).
pub const REVISIT_POLICY_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &REVISIT_POLICY,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// `crawl.keep_alive: false` yields to `crawl.continuous = true`: a Continuous
/// crawl always implies `crawl.keep_alive` (ADR 0139).
pub const KEEP_ALIVE_YIELDS_TO_CONTINUOUS: Yielding = Yielding {
    key: &CRAWL_KEEP_ALIVE,
    when: is_false,
    to: &CRAWL_CONTINUOUS,
    to_values: is_true,
    unless: None,
    reason: "a Continuous crawl implies `crawl.keep_alive`",
};

/// `revisit.change_check` (ADR 0139).
pub const REVISIT_CHANGE_CHECK: Setting = Setting {
    name: "revisit.change_check",
    description: "How a Continuous crawl checks whether a revisited URL changed, set for the Run or for one of its Labels: `conditional+signature` (the default) sends `If-None-Match` and `If-Modified-Since`, treating 304 as unchanged and otherwise comparing the Page signature; `conditional` treats every 200 as changed; `signature` sends no validators and compares the Page signature; `none` treats every revisit as changed. A URL last served by a browser tier always uses `signature`. Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::Choice(REVISIT_CHANGE_CHECK_VALUES),
    default: conditional_signature,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `revisit.on_unchanged` (ADR 0139).
pub const REVISIT_ON_UNCHANGED: Setting = Setting {
    name: "revisit.on_unchanged",
    description: "What a Continuous crawl does when a revisit finds the page unchanged, set for the Run or for one of its Labels: `skip` (the default) runs no handler, pushes no Record, and ends the request `unchanged`; `handler` runs the handler anyway. Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::Choice(REVISIT_ON_UNCHANGED_VALUES),
    default: skip_unchanged,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `revisit.change_check` yields to `crawl.continuous = false`.
pub const REVISIT_CHANGE_CHECK_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &REVISIT_CHANGE_CHECK,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// `revisit.on_unchanged` yields to `crawl.continuous = false`.
pub const REVISIT_ON_UNCHANGED_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &REVISIT_ON_UNCHANGED,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        CRAWL_CONTINUOUS,
        REVISIT_POLICY,
        REVISIT_CHANGE_CHECK,
        REVISIT_ON_UNCHANGED,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
