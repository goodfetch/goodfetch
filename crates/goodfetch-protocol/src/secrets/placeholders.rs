//! The setting that binds each Secret placeholder (ADR 0092):
//! `secrets.placeholders.<name>`, the placeholder `name` bound to a Secret
//! reference, or to a code source, and to the registrable domains its value
//! may go to, at the connection, a Run or a Session. A Recipe never binds
//! one and a Client acting for a model never sets one.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};
use crate::secret_reference::{REFERENCE_PATTERN, SecretReference};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The entry of `domains` that lets a value go to every domain, taken only
/// when written (ADR 0092).
pub const EVERY_DOMAIN: &str = "*";

/// What a Secret placeholder is bound to: where its value comes from and
/// the registrable domains it may go to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundPlaceholder {
    /// Where its value comes from.
    pub source: PlaceholderSource,
    /// The registrable domains, each with its subdomains, or `*`, its
    /// value may go to; never empty.
    pub domains: Vec<String>,
}

/// Where a bound placeholder's value comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceholderSource {
    /// A Secret reference; with `totp`, a TOTP seed whose current code is
    /// typed in its place (ADR 0015).
    Reference {
        /// The reference.
        reference: SecretReference,
        /// Whether its value is a TOTP seed.
        totp: bool,
    },
    /// One of ADR 0006's email or SMS code sources, by its name.
    CodeSource(String),
}

impl BoundPlaceholder {
    /// Reads one placeholder's value of `secrets.placeholders`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message, never
    /// repeating a value given in place of a reference.
    pub fn read(value: &Value) -> Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or("expected a table of `reference` or `code_source`, and `domains`")?;
        if let Some(field) = fields
            .keys()
            .find(|field| !FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "`{field}` is no field of a placeholder's binding; it takes `reference` and `totp`, or `code_source`, and `domains`"
            ));
        }
        let source = source(fields)?;
        let domains = domains(fields.get("domains"))?;
        Ok(Self { source, domains })
    }
}

/// The table [`BoundPlaceholder::read`] reads it back from: the reference's
/// text, with `totp` only when true, or the code source, beside `domains`.
impl Serialize for BoundPlaceholder {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let mut table = serializer.serialize_map(None)?;
        match &self.source {
            PlaceholderSource::Reference { reference, totp } => {
                table.serialize_entry("reference", reference.as_str())?;
                if *totp {
                    table.serialize_entry("totp", totp)?;
                }
            }
            PlaceholderSource::CodeSource(name) => table.serialize_entry("code_source", name)?,
        }
        table.serialize_entry("domains", &self.domains)?;
        table.end()
    }
}

impl crate::catalog::SettingJson for BoundPlaceholder {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// The fields a placeholder's binding takes.
const FIELDS: [&str; 4] = ["reference", "totp", "code_source", "domains"];

fn source(fields: &Map<String, Value>) -> Result<PlaceholderSource, String> {
    let totp = match fields.get("totp") {
        None => None,
        Some(Value::Bool(totp)) => Some(*totp),
        Some(_) => return Err("expected `totp` to be `true` or `false`".to_owned()),
    };
    match (fields.get("reference"), fields.get("code_source")) {
        (Some(reference), None) => {
            let reference = reference
                .as_str()
                .map(SecretReference::parse)
                .ok_or("expected `reference` to be a Secret reference such as `env:NAME`")?
                .map_err(|error| {
                    format!(
                        "expected `reference` to be a Secret reference such as `env:NAME`: {error}; a value is never taken in its place"
                    )
                })?;
            Ok(PlaceholderSource::Reference {
                reference,
                totp: totp.unwrap_or(false),
            })
        }
        (None, Some(code_source)) => {
            if totp.is_some() {
                return Err(
                    "`totp` goes with a `reference` to a TOTP seed, never with a `code_source`"
                        .to_owned(),
                );
            }
            match code_source.as_str() {
                Some(name) if !name.is_empty() => {
                    Ok(PlaceholderSource::CodeSource(name.to_owned()))
                }
                _ => Err("expected `code_source` to name a code source".to_owned()),
            }
        }
        _ => Err("expected either a `reference` or a `code_source`, and only one".to_owned()),
    }
}

fn domains(value: Option<&Value>) -> Result<Vec<String>, String> {
    let entries = value
        .ok_or("expected `domains`, the registrable domains its value may go to, or `[\"*\"]`")?
        .as_array()
        .ok_or("expected `domains` to be a list of registrable domains")?;
    if entries.is_empty() {
        return Err(
            "expected `domains` to list at least one registrable domain, or `\"*\"`".to_owned(),
        );
    }
    entries
        .iter()
        .map(|entry| {
            let domain = entry
                .as_str()
                .ok_or("expected each of `domains` to be a registrable domain")?;
            registrable(domain).map(|()| domain.to_owned())
        })
        .collect()
}

/// Whether `domain` is `*` or a registrable domain as written: a host
/// below one is refused naming it, and a public suffix is refused.
fn registrable(domain: &str) -> Result<(), String> {
    if domain == EVERY_DOMAIN {
        return Ok(());
    }
    match psl::domain_str(&domain.to_ascii_lowercase()) {
        Some(registrable) if registrable == domain => Ok(()),
        Some(registrable) => Err(format!(
            "`{domain}` in `domains` is not a registrable domain: write `{registrable}`, which covers its subdomains"
        )),
        None => Err(format!(
            "`{domain}` in `domains` is a public suffix or no domain, not a registrable domain"
        )),
    }
}

/// A placeholder's binding, as `secrets.placeholders` maps each name to.
pub const BOUND_PLACEHOLDER_SHAPE: ValueShape = ValueShape {
    name: "BoundPlaceholder",
    check: |value| BoundPlaceholder::read(value).map(drop),
    schema: || {
        let domains = json!({
            "type": "array",
            "minItems": 1,
            "items": { "type": "string" },
            "description": "The registrable domains, each with its subdomains, its value may go to; `*` for every domain, only when written.",
        });
        json!({
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["reference", "domains"],
                    "properties": {
                        "reference": {
                            "type": "string",
                            "pattern": REFERENCE_PATTERN,
                            "description": "The Secret reference its value comes from, such as `env:GITHUB_PASSWORD`.",
                        },
                        "totp": {
                            "type": "boolean",
                            "description": "Whether the reference's value is a TOTP seed, whose current code is typed in its place.",
                        },
                        "domains": domains,
                    },
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["code_source", "domains"],
                    "properties": {
                        "code_source": {
                            "type": "string",
                            "description": "The email or SMS code source its one-time code comes from.",
                        },
                        "domains": domains,
                    },
                },
            ],
        })
    },
    examples: || {
        vec![
            json!({ "reference": "env:GITHUB_PASSWORD", "domains": ["github.com"] }),
            json!({ "reference": "op://Private/GitHub/otp", "domains": ["github.com"], "totp": true }),
            json!({ "code_source": "inbox", "domains": ["example.com"] }),
        ]
    },
};

fn no_placeholder() -> Value {
    json!({})
}

/// `secrets.placeholders` (ADR 0092).
pub const SECRETS_PLACEHOLDERS: Setting = Setting {
    name: "secrets.placeholders",
    description: "Each Secret placeholder's binding, as `secrets.placeholders.<name>`: `reference`, the Secret reference its value comes from, with `totp: true` when that value is a TOTP seed whose current code is typed, or `code_source`, the email or SMS code source its one-time code comes from; and `domains`, required, the registrable domains its value may go to, each with its subdomains, `*` only when written. A host below a registrable domain, such as `www.example.com`, is refused with `setting_invalid` naming the domain. `secret(\"name\")` in code and `%name%` in an AI instruction name the placeholder, and the nearest of a Session, its Run and the connection that binds it wins. None by default. A Recipe declares names and never binds one, and a Client acting for a model never sets one: either's value is `setting_locked`.",
    kind: ValueKind::Map(&ValueKind::Shape(&BOUND_PLACEHOLDER_SHAPE)),
    default: no_placeholder,
    clearable: false,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

#[cfg(test)]
mod tests;
