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
}

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
        }
    }

    /// Whether `higher`, a Recipe's or a run's value, allows no more than
    /// `lower`, the value the layers below give: a list or a map with no
    /// entry `lower` lacks, a count, a duration or a price no larger, and
    /// otherwise the same value. Both are of this kind.
    pub fn narrows(&self, higher: &Value, lower: &Value) -> bool {
        match self {
            Self::Count | Self::Duration => match (higher.as_u64(), lower.as_u64()) {
                (Some(higher), Some(lower)) => higher <= lower,
                _ => higher == lower,
            },
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
    /// Only Operator policy, as `wall.denied_domains` (ADR 0052).
    OperatorPolicy,
    /// Only Operator policy and user configuration, the machine's own
    /// layers, as `browser_cache.max_unused_age` (ADR 0098); anywhere else
    /// it is `setting_not_applicable`.
    Machine,
    /// Every layer but a Recipe, which may only expose it, as
    /// `session.source` (ADR 0056).
    NotByRecipe,
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
            Self::OperatorPolicy => "operator_policy",
            Self::Machine => "machine",
            Self::NotByRecipe => "not_by_recipe",
            Self::Fixed => "fixed",
        }
    }
}

#[cfg(test)]
mod tests;
