//! What a Setting's values may be and who may set them (ADRs 0028 and
//! 0083): the kind of its values, which checks a value, gives its JSON
//! Schema and decides whether one value narrows another, and its authority,
//! the layers that may set it and how.

use std::cmp::Ordering;
use std::net::IpAddr;

use serde_json::{Value, json};

use crate::secret_reference::{REFERENCE_PATTERN, SecretReference};

/// The kind of a Setting's values, as every layer writes them: JSON on the
/// wire, the same nesting in a TOML file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// `true` or `false`.
    Boolean,
    /// A whole number from 0.
    Count,
    /// A whole number from 0 to this one, as `network.max_json_depth`,
    /// which the engine could not apply past its ceiling; a larger value is
    /// `setting_invalid`. It caps and narrows as a count does.
    CountUpTo(u64),
    /// A duration in whole milliseconds, the one unit of every surface's
    /// wire (ADR 0080).
    Duration,
    /// Any text, such as a path or a domain.
    Text,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// An amount and its ISO 4217 currency, `"0.05 USD"` (ADR 0045).
    Price,
    /// An IPv4 or IPv6 address.
    Address,
    /// A list of values of one kind.
    List(&'static ValueKind),
    /// Names, such as domains or hosts, each with a value of one kind.
    Map(&'static ValueKind),
    /// A value of one kind, or `null`, inside a list or a map.
    Optional(&'static ValueKind),
    /// A Secret reference, `provider:rest`, never a value (ADR 0015).
    SecretReference,
    /// A value of a shape the protocol defines once as a type, such as
    /// `session.source`'s tagged union (ADR 0056).
    Shape(&'static ValueShape),
}

/// A shape of values the protocol defines once as a type: its name, what
/// checks a value against it, its JSON Schema with nothing referenced
/// outside it, and values of it that together give each of its fields a
/// value, for the corpora that need them.
#[derive(Debug)]
pub struct ValueShape {
    /// The type's name.
    pub name: &'static str,
    /// Whether `value` is of the shape, or what is wrong with it.
    pub check: fn(&Value) -> Result<(), String>,
    /// Its JSON Schema, every subschema inline.
    pub schema: fn() -> Value,
    /// Values of it, the first the most common, that together give each of
    /// its fields a value.
    pub examples: fn() -> Vec<Value>,
}

/// Two shapes are one when they name one type.
impl PartialEq for ValueShape {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for ValueShape {}

impl ValueKind {
    /// Whether `value` is of this kind, or what is wrong with it.
    ///
    /// # Errors
    ///
    /// What the value lacks, for a `setting_invalid` message.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        let wrong = |expected: &str| Err(format!("expected {expected}, found `{value}`"));
        match self {
            Self::Boolean if value.is_boolean() => Ok(()),
            Self::Boolean => wrong("`true` or `false`"),
            Self::Count | Self::Duration if value.is_u64() => Ok(()),
            Self::Count => wrong("a whole number from 0"),
            Self::CountUpTo(most) => match value.as_u64() {
                Some(count) if count <= *most => Ok(()),
                _ => wrong(&format!("a whole number from 0 to {most}")),
            },
            Self::Duration => wrong("a whole number of milliseconds"),
            Self::Text if value.is_string() => Ok(()),
            Self::Text => wrong("text"),
            Self::Choice(words) => match value.as_str() {
                Some(word) if words.contains(&word) => Ok(()),
                _ => wrong(&format!("one of {}", words.join(", "))),
            },
            Self::Price => match value.as_str().and_then(price) {
                Some(_) => Ok(()),
                None => wrong("an amount and its currency, such as `0.05 USD`"),
            },
            Self::Address => match value.as_str().map(str::parse::<IpAddr>) {
                Some(Ok(_)) => Ok(()),
                _ => wrong("an IP address"),
            },
            Self::List(kind) => match value.as_array() {
                Some(items) => items.iter().try_for_each(|item| kind.check(item)),
                None => wrong("a list"),
            },
            Self::Map(kind) => match value.as_object() {
                Some(entries) => entries.values().try_for_each(|entry| kind.check(entry)),
                None => wrong("a table of names"),
            },
            Self::Optional(_) if value.is_null() => Ok(()),
            Self::Optional(kind) => kind.check(value),
            Self::SecretReference => match value.as_str().map(SecretReference::parse) {
                Some(Ok(_)) => Ok(()),
                Some(Err(error)) => Err(format!(
                    "expected a Secret reference such as `env:NAME`: {error}; a value is never taken in its place"
                )),
                None => Err("expected a Secret reference such as `env:NAME`".to_owned()),
            },
            Self::Shape(shape) => (shape.check)(value),
        }
    }

    /// Whether `higher`, a Recipe's or a run's value, allows no more than
    /// `lower`, the value the layers below give: a list or a map with no
    /// entry `lower` lacks, a count, a duration or a price no larger, and
    /// otherwise the same value. Both are of this kind.
    pub fn narrows(&self, higher: &Value, lower: &Value) -> bool {
        match self {
            Self::Count | Self::CountUpTo(_) | Self::Duration => {
                match (higher.as_u64(), lower.as_u64()) {
                    (Some(higher), Some(lower)) => higher <= lower,
                    _ => higher == lower,
                }
            }
            Self::Price => match (
                higher.as_str().and_then(price),
                lower.as_str().and_then(price),
            ) {
                (Some(higher), Some(lower)) => {
                    higher.currency == lower.currency && higher.amount_cmp(&lower).is_le()
                }
                _ => higher == lower,
            },
            Self::List(_) => match (higher.as_array(), lower.as_array()) {
                (Some(higher), Some(lower)) => higher.iter().all(|item| lower.contains(item)),
                _ => higher == lower,
            },
            Self::Map(kind) => match (higher.as_object(), lower.as_object()) {
                (Some(higher), Some(lower)) => higher.iter().all(|(name, entry)| {
                    lower
                        .get(name)
                        .is_some_and(|allowed| kind.narrows(entry, allowed))
                }),
                _ => higher == lower,
            },
            Self::Optional(kind) if !higher.is_null() && !lower.is_null() => {
                kind.narrows(higher, lower)
            }
            _ => higher == lower,
        }
    }

    /// Its values' JSON Schema.
    pub fn schema(&self) -> Value {
        match self {
            Self::Boolean => json!({ "type": "boolean" }),
            Self::Count => json!({ "type": "integer", "minimum": 0 }),
            Self::CountUpTo(most) => json!({ "type": "integer", "minimum": 0, "maximum": most }),
            Self::Duration => json!({
                "type": "integer",
                "minimum": 0,
                "description": "A duration in milliseconds.",
            }),
            Self::Text => json!({ "type": "string" }),
            Self::Choice(words) => json!({ "enum": words }),
            Self::Price => json!({
                "type": "string",
                "pattern": PRICE_PATTERN,
            }),
            Self::Address => json!({ "type": "string", "format": "ip" }),
            Self::List(kind) => json!({ "type": "array", "items": kind.schema() }),
            Self::Map(kind) => json!({ "type": "object", "additionalProperties": kind.schema() }),
            Self::Optional(kind) => json!({ "anyOf": [kind.schema(), { "type": "null" }] }),
            Self::SecretReference => json!({
                "type": "string",
                "pattern": REFERENCE_PATTERN,
                "description": "A Secret reference, such as `env:NAME`.",
            }),
            Self::Shape(shape) => (shape.schema)(),
        }
    }
}

/// A price's text: an amount without sign or leading zero, and a currency.
const PRICE_PATTERN: &str = "^(0|[1-9][0-9]*)(\\.[0-9]+)? [A-Z]{3}$";

struct Price<'a> {
    whole: &'a str,
    fraction: &'a str,
    currency: &'a str,
}

impl Price<'_> {
    fn amount_cmp(&self, other: &Price<'_>) -> Ordering {
        let width = self.fraction.len().max(other.fraction.len());
        let pad = |fraction: &str| format!("{fraction:0<width$}");
        self.whole
            .len()
            .cmp(&other.whole.len())
            .then_with(|| self.whole.cmp(other.whole))
            .then_with(|| pad(self.fraction).cmp(&pad(other.fraction)))
    }
}

fn price(text: &str) -> Option<Price<'_>> {
    let (amount, currency) = text.split_once(' ')?;
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let well_formed = !whole.is_empty()
        && digits(whole)
        && (whole == "0" || !whole.starts_with('0'))
        && digits(fraction)
        && (fraction.is_empty() != amount.contains('.'))
        && currency.len() == 3
        && currency.bytes().all(|byte| byte.is_ascii_uppercase());
    well_formed.then_some(Price {
        whole,
        fraction,
        currency,
    })
}

/// A price as [`ValueKind::Price`] reads it: an amount in millionths of
/// its currency, the unit of a currency Limit (ADR 0041), and the ISO 4217
/// currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceValue {
    /// The amount, in millionths of the currency, rounded up past the
    /// sixth decimal so that a price is never read below what it says.
    pub millionths: u64,
    /// The currency, such as `USD`.
    pub currency: String,
}

impl PriceValue {
    /// Reads `text`, such as `"0.05 USD"`; none when it is not a price or
    /// its amount does not fit.
    pub fn parse(text: &str) -> Option<Self> {
        let price = price(text)?;
        let (kept, rest) = price
            .fraction
            .split_at(price.fraction.len().min(MILLIONTHS_DIGITS));
        let fraction: u64 = format!("{kept:0<MILLIONTHS_DIGITS$}").parse().ok()?;
        let whole: u64 = price.whole.parse().ok()?;
        let up = u64::from(rest.bytes().any(|digit| digit != b'0'));
        let millionths = whole
            .checked_mul(1_000_000)?
            .checked_add(fraction)?
            .checked_add(up)?;
        Some(Self {
            millionths,
            currency: price.currency.to_owned(),
        })
    }
}

/// The decimals of a millionth.
const MILLIONTHS_DIGITS: usize = 6;

/// Who may set a Setting, by layer (ADR 0028 and the ADRs that amend it).
/// Operator policy may lock or cap any key that is not [`Authority::Fixed`].
#[derive(Clone, Copy, Debug)]
pub enum Authority {
    /// Every layer.
    Every,
    /// Every layer, but a Recipe and the run only narrow the value the
    /// layers below give, as `paid_fetch.domains` (ADR 0045).
    Narrowed,
    /// Only Operator policy and user and project configuration, as
    /// `tls.trusted_cas` and Paid fetches' exchange rates (ADRs 0042, 0045);
    /// a Recipe's or a run's value is `setting_locked`.
    Configuration,
    /// As [`Authority::Configuration`], except that a Recipe or a run may
    /// give the values this accepts, as `tls.accept_invalid_certs`'s entries
    /// without a pin, which ask for trust on first use (ADR 0042).
    ConfigurationExcept(fn(&Value) -> bool),
    /// As [`Authority::Configuration`], except that project configuration
    /// gives only the values this accepts, as `proxy.providers`, whose
    /// prices only the machine's own layers set since a currency Limit
    /// trusts them (O12 of the deep review); another project value is
    /// `setting_locked`.
    ConfigurationProjectLimited(fn(&Value) -> bool),
    /// Only Operator policy, as `wall.denied_domains` (ADR 0052).
    OperatorPolicy,
    /// Only Operator policy, as `stealth.allowed_builds` (spec #599);
    /// anywhere else it is `setting_not_applicable`, unlike
    /// [`Authority::OperatorPolicy`]'s `setting_locked`.
    OperatorPolicyAlone,
    /// Only Operator policy and user configuration, the machine's own
    /// layers, as `browser_cache.max_unused_age` (ADR 0098); anywhere else
    /// it is `setting_not_applicable`.
    Machine,
    /// Only project configuration, the project file, as `timeout.connect`,
    /// which `connect()` and the CLI read before they connect and apply
    /// themselves (ADR 0083); anywhere else it is `setting_not_applicable`,
    /// since only the engine reads the other layers, once reached.
    ProjectFileAlone,
    /// Every layer but a Recipe, which may only expose it, as
    /// `session.source` (ADR 0056).
    NotByRecipe,
    /// As [`Authority::NotByRecipe`], and never a Client acting for a model,
    /// whose value is `setting_locked`, as `proxy.server` and the proxy's
    /// credentials, which send a Secret reference's value to a destination
    /// they also name (ADR 0092).
    NotByRecipeOrModel,
    /// Every layer but a Client acting for a model, whose value is
    /// `setting_locked`, as `proxy.kind`, whose kinds' prices the person
    /// pays (O11 of the deep review).
    NotByModel,
    /// Every layer but a Client acting for a model, whose value is
    /// `setting_locked`, and a Recipe, unless the Operator policy sets the
    /// boolean setting this names to `true`, as `egress.private_ranges`
    /// (ticket #622, ADR 0017's note).
    NotByModelRecipeAllowedBy(&'static str),
    /// Every layer, but a Recipe and a Client acting for a model only
    /// narrow the list of hosts the layers below give, each entry a host
    /// and its subdomains, as the domain policy's `egress.allowed_domains`
    /// and `egress.blocked_domains` (spec #563, G5); a widening value from
    /// them is `setting_locked`. The program's own code and configuration
    /// set any value.
    HostsNarrowedByRecipeOrModel(HostList),
    /// Every layer, but a Recipe and a Client acting for a model never give
    /// a value the key's `weakens` names unless the layers below already
    /// give that value, which is `setting_locked`; the program's own code,
    /// configuration and an Operator policy set any value, as
    /// `consent.handling: off` (#2434).
    NotWeakenedByRecipeOrModel,
    /// No layer: an honesty guarantee that stays at its built-in value
    /// whatever the layer (ADR 0028).
    Fixed,
}

impl Authority {
    /// Its name in the catalog.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Every => "every",
            Self::Narrowed => "narrowed",
            Self::Configuration => "configuration",
            Self::ConfigurationExcept(_) => "configuration_except",
            Self::ConfigurationProjectLimited(_) => "configuration_project_limited",
            Self::OperatorPolicy => "operator_policy",
            Self::OperatorPolicyAlone => "operator_policy_alone",
            Self::Machine => "machine",
            Self::ProjectFileAlone => "project_file_alone",
            Self::NotByRecipe => "not_by_recipe",
            Self::NotByRecipeOrModel => "not_by_recipe_or_model",
            Self::NotByModel => "not_by_model",
            Self::NotByModelRecipeAllowedBy(_) => "not_by_model_recipe_allowed_by_operator_policy",
            Self::HostsNarrowedByRecipeOrModel(_) => "hosts_narrowed_by_recipe_or_model",
            Self::NotWeakenedByRecipeOrModel => "not_weakened_by_recipe_or_model",
            Self::Fixed => "fixed",
        }
    }
}

/// What a list of hosts of [`Authority::HostsNarrowedByRecipeOrModel`]
/// does with the hosts it lists, which says which way it narrows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostList {
    /// It allows them: a value narrows by listing no host the value below
    /// does not cover, as `egress.allowed_domains`.
    Allows,
    /// It blocks them: a value narrows by covering every host the value
    /// below lists, as `egress.blocked_domains`.
    Blocks,
}

/// A scope a program's code opens with its own `settings` (ADR 0083): the
/// connection, a Run, a crawl's Label, a Session or a page. A setting
/// applies within the scopes that may set it, and everything opened beneath
/// them inherits it: a Label's, its requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingScope {
    /// `connect()`, whose settings are its Runs' defaults.
    Connection,
    /// Starting a Run.
    Run,
    /// Declaring a crawl's Label, whose own settings its requests take over
    /// their Run's (ADR 0093).
    Label,
    /// Opening a Session.
    Session,
    /// Opening a page.
    Page,
}

impl SettingScope {
    /// Its name in the catalog.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Run => "run",
            Self::Label => "label",
            Self::Session => "session",
            Self::Page => "page",
        }
    }
}

/// A value of a Setting as the wire carries it (ADR 0083): what each type
/// the SDKs' typed keys take implements, here for the values of every
/// [`ValueKind`] and the types of the shapes this crate defines, and in a
/// Product's client crate through [`SettingWord`] for its choices of words.
pub trait SettingJson {
    /// The value's JSON.
    fn into_setting_json(self) -> Value;
}

/// A choice of words a Setting takes, which goes on the wire as its word:
/// what each generated choice enum of a Product's client crate implements.
pub trait SettingWord {
    /// The word, as the catalog spells it.
    fn word(&self) -> &'static str;
}

/// `duration` in whole milliseconds, rounded up, so that no duration but
/// zero sends 0: how a duration Setting and a call's timeout are sent.
#[must_use]
pub fn wire_millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// The JSON of a value of a shape's type, which its own `Serialize` gives.
pub(crate) fn shape_json(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("invariant: a value of the protocol's shapes is JSON")
}

/// Its word.
impl<T: SettingWord> SettingJson for T {
    fn into_setting_json(self) -> Value {
        Value::from(self.word())
    }
}

impl SettingJson for bool {
    fn into_setting_json(self) -> Value {
        Value::Bool(self)
    }
}

impl SettingJson for u64 {
    fn into_setting_json(self) -> Value {
        Value::from(self)
    }
}

impl SettingJson for String {
    fn into_setting_json(self) -> Value {
        Value::String(self)
    }
}

/// Its [`wire_millis`].
impl SettingJson for std::time::Duration {
    fn into_setting_json(self) -> Value {
        Value::from(wire_millis(self))
    }
}

/// Null for `None`.
impl<T: SettingJson> SettingJson for Option<T> {
    fn into_setting_json(self) -> Value {
        self.map_or(Value::Null, SettingJson::into_setting_json)
    }
}

impl<T: SettingJson> SettingJson for Vec<T> {
    fn into_setting_json(self) -> Value {
        Value::Array(
            self.into_iter()
                .map(SettingJson::into_setting_json)
                .collect(),
        )
    }
}

impl<T: SettingJson> SettingJson for std::collections::BTreeMap<String, T> {
    fn into_setting_json(self) -> Value {
        Value::Object(
            self.into_iter()
                .map(|(name, entry)| (name, entry.into_setting_json()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests;
