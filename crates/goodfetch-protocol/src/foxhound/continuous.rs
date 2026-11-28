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

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[CRAWL_CONTINUOUS, REVISIT_POLICY],
    hook_stages: &[],
    shared_type_values: &[],
};
