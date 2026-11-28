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

/// Default initial interval for `revisit.policy: adaptive` and `estimated`: 1 day (ADR 0139).
pub const REVISIT_DEFAULT_INITIAL_MS: u64 = 86_400_000;
/// Default minimum interval for `revisit.policy: adaptive` and `estimated`: 1 hour (ADR 0139).
pub const REVISIT_DEFAULT_MIN_MS: u64 = 3_600_000;
/// Default maximum interval for `revisit.policy: adaptive` and `estimated`: 30 days (ADR 0139).
pub const REVISIT_DEFAULT_MAX_MS: u64 = 2_592_000_000;
/// Default shrink rate on change for `revisit.policy: adaptive` (ADR 0139).
pub const REVISIT_ADAPTIVE_DEFAULT_RATE_CHANGED: f64 = 0.5;
/// Default growth rate on unchanged for `revisit.policy: adaptive` (ADR 0139).
pub const REVISIT_ADAPTIVE_DEFAULT_RATE_UNCHANGED: f64 = 0.5;
/// Default target freshness for `revisit.policy: estimated` (ADR 0139).
pub const REVISIT_ESTIMATED_DEFAULT_TARGET_FRESHNESS: f64 = 0.9;

const fn default_revisit_initial() -> u64 {
    REVISIT_DEFAULT_INITIAL_MS
}

const fn default_revisit_min() -> u64 {
    REVISIT_DEFAULT_MIN_MS
}

const fn default_revisit_max() -> u64 {
    REVISIT_DEFAULT_MAX_MS
}

const fn default_rate_changed() -> f64 {
    REVISIT_ADAPTIVE_DEFAULT_RATE_CHANGED
}

const fn default_rate_unchanged() -> f64 {
    REVISIT_ADAPTIVE_DEFAULT_RATE_UNCHANGED
}

const fn default_target_freshness() -> f64 {
    REVISIT_ESTIMATED_DEFAULT_TARGET_FRESHNESS
}

fn adaptive_default_policy() -> Value {
    json!({
        "kind": "adaptive",
        "initial": REVISIT_DEFAULT_INITIAL_MS,
        "min": REVISIT_DEFAULT_MIN_MS,
        "max": REVISIT_DEFAULT_MAX_MS,
        "rate_changed": REVISIT_ADAPTIVE_DEFAULT_RATE_CHANGED,
        "rate_unchanged": REVISIT_ADAPTIVE_DEFAULT_RATE_UNCHANGED,
    })
}

fn conditional_signature() -> Value {
    Value::String("conditional+signature".to_owned())
}

fn skip_unchanged() -> Value {
    Value::String("skip".to_owned())
}

fn default_revisit_order() -> Value {
    json!(["sources", "first_visits", "revisits"])
}

fn default_revisit_domain_share() -> Value {
    json!(0.5)
}

fn default_revisit_on_error() -> Value {
    json!(7_200_000)
}

fn default_revisit_gone_after() -> Value {
    json!(1)
}

/// One kind of work a domain's turn orders in a Continuous crawl
/// (`crawl.revisit.order`, ADR 0004, ADR 0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevisitOrderKind {
    /// Due source reads (sitemaps, syndication feeds).
    Sources,
    /// First visits of discovered URLs.
    FirstVisits,
    /// Due page revisits.
    Revisits,
}

impl RevisitOrderKind {
    /// Its wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sources => "sources",
            Self::FirstVisits => "first_visits",
            Self::Revisits => "revisits",
        }
    }
}

/// The order a domain's turn picks among sources, first visits and revisits
/// (`crawl.revisit.order`, ADR 0004, ADR 0139): a permutation of all three
/// [`RevisitOrderKind`] variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<RevisitOrderKind>", into = "Vec<RevisitOrderKind>")]
pub struct RevisitOrder([RevisitOrderKind; 3]);

/// An invalid [`RevisitOrder`] (not a permutation of all three kinds).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "`crawl.revisit.order` must list each of `sources`, `first_visits`, `revisits` exactly once"
)]
pub struct InvalidRevisitOrder;

impl Default for RevisitOrder {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl RevisitOrder {
    /// The default turn order: `sources`, `first_visits`, `revisits`.
    pub const DEFAULT: Self = Self([
        RevisitOrderKind::Sources,
        RevisitOrderKind::FirstVisits,
        RevisitOrderKind::Revisits,
    ]);

    /// Builds a [`RevisitOrder`] from `kinds`, requiring all three distinct kinds.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRevisitOrder`] when `kinds` does not contain each of
    /// `sources`, `first_visits`, and `revisits` exactly once.
    pub fn new(kinds: [RevisitOrderKind; 3]) -> Result<Self, InvalidRevisitOrder> {
        if kinds[0] != kinds[1] && kinds[0] != kinds[2] && kinds[1] != kinds[2] {
            Ok(Self(kinds))
        } else {
            Err(InvalidRevisitOrder)
        }
    }

    /// The three kinds in priority order.
    #[must_use]
    pub const fn kinds(self) -> [RevisitOrderKind; 3] {
        self.0
    }

    /// Parses a [`RevisitOrder`] from a JSON setting value.
    ///
    /// # Errors
    ///
    /// Returns a reason string when `value` is not a permutation of the three kinds.
    pub fn read(value: &Value) -> Result<Self, String> {
        serde_json::from_value::<Self>(value.clone()).map_err(|error| {
            format!(
                "`crawl.revisit.order` must be a list of `sources`, `first_visits`, `revisits` each once: {error}"
            )
        })
    }
}

impl TryFrom<Vec<RevisitOrderKind>> for RevisitOrder {
    type Error = InvalidRevisitOrder;

    fn try_from(kinds: Vec<RevisitOrderKind>) -> Result<Self, Self::Error> {
        let array: [RevisitOrderKind; 3] = kinds.try_into().map_err(|_| InvalidRevisitOrder)?;
        Self::new(array)
    }
}

impl From<RevisitOrder> for Vec<RevisitOrderKind> {
    fn from(order: RevisitOrder) -> Self {
        order.0.to_vec()
    }
}

impl SettingJson for RevisitOrder {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

/// `crawl.revisit.order`'s values: a [`RevisitOrder`].
pub const REVISIT_ORDER_SHAPE: ValueShape = ValueShape {
    name: "RevisitOrder",
    check: |value| RevisitOrder::read(value).map(drop),
    schema: || {
        json!({
            "type": "array",
            "items": {
                "type": "string",
                "enum": ["sources", "first_visits", "revisits"]
            }
        })
    },
    examples: || {
        vec![
            json!(["sources", "first_visits", "revisits"]),
            json!(["revisits", "first_visits", "sources"]),
        ]
    },
};

/// Maximum share of a domain's starts revisits may take while first visits
/// wait (`crawl.revisit.domain_share`, ADR 0004, ADR 0139): a finite number
/// in `0.0..=1.0`, defaulting to `0.5`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct RevisitDomainShare(f64);

/// A number outside `[0.0, 1.0]` for [`RevisitDomainShare`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is not a finite share between 0.0 and 1.0")]
pub struct InvalidRevisitDomainShare(pub f64);

impl Default for RevisitDomainShare {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl RevisitDomainShare {
    /// The default share: `0.5` (at most half of a domain's starts).
    pub const DEFAULT: Self = Self(0.5);

    /// Validates `share` as a finite number in `0.0..=1.0`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRevisitDomainShare`] when `share` is not finite or
    /// outside `0.0..=1.0`.
    pub fn new(share: f64) -> Result<Self, InvalidRevisitDomainShare> {
        if share.is_finite() && (0.0..=1.0).contains(&share) {
            Ok(Self(share))
        } else {
            Err(InvalidRevisitDomainShare(share))
        }
    }

    /// The numeric share in `0.0..=1.0`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Parses a [`RevisitDomainShare`] from a JSON setting value.
    ///
    /// # Errors
    ///
    /// Returns a reason string when `value` is not a finite number in `0.0..=1.0`.
    pub fn read(value: &Value) -> Result<Self, String> {
        let Some(number) = value.as_f64() else {
            return Err(format!(
                "`crawl.revisit.domain_share` must be a number between 0 and 1, found `{value}`"
            ));
        };
        Self::new(number).map_err(|error| error.to_string())
    }
}

impl TryFrom<f64> for RevisitDomainShare {
    type Error = InvalidRevisitDomainShare;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RevisitDomainShare> for f64 {
    fn from(share: RevisitDomainShare) -> Self {
        share.0
    }
}

impl SettingJson for RevisitDomainShare {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

/// `crawl.revisit.domain_share`'s values: a [`RevisitDomainShare`].
pub const REVISIT_DOMAIN_SHARE_SHAPE: ValueShape = ValueShape {
    name: "RevisitDomainShare",
    check: |value| RevisitDomainShare::read(value).map(drop),
    schema: || {
        json!({
            "type": "number",
            "minimum": 0.0,
            "maximum": 1.0
        })
    },
    examples: || vec![json!(0.5), json!(0.25)],
};

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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// Multiplicative adaptive schedule (Nutch / `StormCrawler`, ADR 0139):
    /// shrinks by `rate_changed` after a change and grows by `rate_unchanged`
    /// after none, clamped to `[min, max]`. Default for pages.
    Adaptive {
        /// Initial revisit interval in milliseconds (default: 1 day).
        #[serde(default = "default_revisit_initial")]
        #[schemars(extend("arrived_in" = "0.1"))]
        initial: u64,
        /// Minimum revisit interval in milliseconds (default: 1 hour).
        #[serde(default = "default_revisit_min")]
        #[schemars(extend("arrived_in" = "0.1"))]
        min: u64,
        /// Maximum revisit interval in milliseconds (default: 30 days).
        #[serde(default = "default_revisit_max")]
        #[schemars(extend("arrived_in" = "0.1"))]
        max: u64,
        /// Fractional shrink applied after a changed visit, in `(0, 1]` (default: 0.5).
        #[serde(default = "default_rate_changed")]
        #[schemars(extend("arrived_in" = "0.1"))]
        rate_changed: f64,
        /// Fractional growth applied after an unchanged visit, in `(0, 1]` (default: 0.5).
        #[serde(default = "default_rate_unchanged")]
        #[schemars(extend("arrived_in" = "0.1"))]
        rate_unchanged: f64,
    },
    /// Change-rate estimator schedule (Cho and Garcia-Molina, ADR 0139):
    /// estimates change rate from visits and changes starting from a prior of
    /// two intervals of `initial`, and schedules to reach `target_freshness`,
    /// clamped to `[min, max]`.
    Estimated {
        /// Prior interval in milliseconds (default: 1 day).
        #[serde(default = "default_revisit_initial")]
        #[schemars(extend("arrived_in" = "0.1"))]
        initial: u64,
        /// Minimum revisit interval in milliseconds (default: 1 hour).
        #[serde(default = "default_revisit_min")]
        #[schemars(extend("arrived_in" = "0.1"))]
        min: u64,
        /// Maximum revisit interval in milliseconds (default: 30 days).
        #[serde(default = "default_revisit_max")]
        #[schemars(extend("arrived_in" = "0.1"))]
        max: u64,
        /// Target expected freshness, strictly in `(0, 1)` (default: 0.9).
        #[serde(default = "default_target_freshness")]
        #[schemars(extend("arrived_in" = "0.1"))]
        target_freshness: f64,
    },
}

impl SettingJson for RevisitPolicy {
    fn into_setting_json(self) -> Value {
        shape_json(self)
    }
}

impl RevisitPolicy {
    /// Default Revisit policy for pages: `adaptive` with 1 day initial, 1 hour
    /// min, 30 days max, and both rates 0.5 (ADR 0139).
    #[must_use]
    pub const fn default_adaptive() -> Self {
        Self::Adaptive {
            initial: REVISIT_DEFAULT_INITIAL_MS,
            min: REVISIT_DEFAULT_MIN_MS,
            max: REVISIT_DEFAULT_MAX_MS,
            rate_changed: REVISIT_ADAPTIVE_DEFAULT_RATE_CHANGED,
            rate_unchanged: REVISIT_ADAPTIVE_DEFAULT_RATE_UNCHANGED,
        }
    }

    /// Parses a [`RevisitPolicy`] from a JSON setting value.
    ///
    /// # Errors
    ///
    /// Returns a reason string when `value` is not a valid policy, has a zero
    /// duration, has `min > max`, has a rate outside `(0, 1]`, or has a
    /// `target_freshness` outside `(0, 1)`.
    pub fn read(value: &Value) -> Result<Self, String> {
        let policy: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "`revisit.policy` must be `never`, `every`, `adaptive`, or `estimated`: {error}"
            )
        })?;
        match policy {
            Self::Never => Ok(policy),
            Self::Every { interval } if interval > 0 => Ok(policy),
            Self::Every { .. } => {
                Err("`revisit.policy` `every` interval must be at least 1 ms".to_owned())
            }
            Self::Adaptive {
                initial,
                min,
                max,
                rate_changed,
                rate_unchanged,
            } => {
                validate_bounds("adaptive", initial, min, max)?;
                if !rate_changed.is_finite() || rate_changed <= 0.0 || rate_changed > 1.0 {
                    return Err(
                        "`revisit.policy` `adaptive.rate_changed` must be in (0, 1]".to_owned()
                    );
                }
                if !rate_unchanged.is_finite() || rate_unchanged <= 0.0 || rate_unchanged > 1.0 {
                    return Err(
                        "`revisit.policy` `adaptive.rate_unchanged` must be in (0, 1]".to_owned(),
                    );
                }
                Ok(policy)
            }
            Self::Estimated {
                initial,
                min,
                max,
                target_freshness,
            } => {
                validate_bounds("estimated", initial, min, max)?;
                if !target_freshness.is_finite()
                    || target_freshness <= 0.0
                    || target_freshness >= 1.0
                {
                    return Err(
                        "`revisit.policy` `estimated.target_freshness` must be in (0, 1)"
                            .to_owned(),
                    );
                }
                Ok(policy)
            }
        }
    }
}

fn validate_bounds(kind: &str, initial: u64, min: u64, max: u64) -> Result<(), String> {
    if initial == 0 || min == 0 || max == 0 {
        return Err(format!(
            "`revisit.policy` `{kind}` durations (`initial`, `min`, `max`) must be at least 1 ms"
        ));
    }
    if min > max {
        return Err(format!(
            "`revisit.policy` `{kind}` `min` ({min}) must not exceed `max` ({max})"
        ));
    }
    Ok(())
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
            adaptive_default_policy(),
            json!({
                "kind": "estimated",
                "initial": REVISIT_DEFAULT_INITIAL_MS,
                "min": REVISIT_DEFAULT_MIN_MS,
                "max": REVISIT_DEFAULT_MAX_MS,
                "target_freshness": REVISIT_ESTIMATED_DEFAULT_TARGET_FRESHNESS,
            }),
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
    description: "How a Continuous crawl schedules revisits of each URL, set for the Run or for one of its Labels: `never` for pages read once, `every` for a fixed interval, `adaptive` (the default for pages: initial 1 day, min 1 hour, max 30 days, both rates 0.5) for multiplicative adjustment, or `estimated` (Cho and Garcia-Molina's estimator targeting `target_freshness`). Accepted on a batch crawl, where `config explain` marks it inactive (`yields_to` `crawl.continuous`).",
    kind: ValueKind::Shape(&REVISIT_POLICY_SHAPE),
    default: adaptive_default_policy,
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

/// `crawl.revisit.order` (ADR 0004, ADR 0139).
pub const CRAWL_REVISIT_ORDER: Setting = Setting {
    name: "crawl.revisit.order",
    description: "Order in which each turn of a domain in a Continuous crawl picks among `sources`, `first_visits` and `revisits`: `[\"sources\", \"first_visits\", \"revisits\"]` by default. Within each kind, ADR 0004's priority and `forefront` order holds. Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::Shape(&REVISIT_ORDER_SHAPE),
    default: default_revisit_order,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.revisit.domain_share` (ADR 0004, ADR 0139).
pub const CRAWL_REVISIT_DOMAIN_SHARE: Setting = Setting {
    name: "crawl.revisit.domain_share",
    description: "Maximum share of a domain's starts revisits may take while first visits wait on that domain in a Continuous crawl (0.0 to 1.0, `0.5` by default). Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::Shape(&REVISIT_DOMAIN_SHARE_SHAPE),
    default: default_revisit_domain_share,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `crawl.revisit.order` yields to `crawl.continuous = false`.
pub const CRAWL_REVISIT_ORDER_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &CRAWL_REVISIT_ORDER,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// `crawl.revisit.domain_share` yields to `crawl.continuous = false`.
pub const CRAWL_REVISIT_DOMAIN_SHARE_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &CRAWL_REVISIT_DOMAIN_SHARE,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// `revisit.on_error` (ADR 0139).
pub const REVISIT_ON_ERROR: Setting = Setting {
    name: "revisit.on_error",
    description: "How long a Continuous crawl waits before revisiting a URL whose revisit fetch failed retryably, in whole milliseconds, set for the Run or for one of its Labels: 2 hours (7,200,000 ms) by default (StormCrawler's `fetchInterval.fetch.error`). Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::Duration,
    default: default_revisit_on_error,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `revisit.gone_after` (ADR 0139).
pub const REVISIT_GONE_AFTER: Setting = Setting {
    name: "revisit.gone_after",
    description: "How many consecutive 404 or 410 revisit responses end a URL `gone` so it is not revisited again, set for the Run or for one of its Labels: 1 by default (at least 1). Accepted on a batch crawl, where `config explain` marks it inactive.",
    kind: ValueKind::CountFromOne,
    default: default_revisit_gone_after,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `revisit.on_error` yields to `crawl.continuous = false`.
pub const REVISIT_ON_ERROR_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &REVISIT_ON_ERROR,
    when: any_value,
    to: &CRAWL_CONTINUOUS,
    to_values: is_false,
    unless: None,
    reason: "a batch crawl does not revisit URLs",
};

/// `revisit.gone_after` yields to `crawl.continuous = false`.
pub const REVISIT_GONE_AFTER_YIELDS_TO_BATCH: Yielding = Yielding {
    key: &REVISIT_GONE_AFTER,
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
        REVISIT_ON_ERROR,
        REVISIT_GONE_AFTER,
        CRAWL_REVISIT_ORDER,
        CRAWL_REVISIT_DOMAIN_SHARE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
