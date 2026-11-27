//! `session.http_credentials`'s value (ADR 0092; SA-27): a list of HTTP
//! Basic/Digest credentials, each bound to required registrable domains (or
//! `*`), with `password` a Secret reference, so credentials answer an HTTP
//! authentication challenge only from a listed domain and never on a browser
//! other code may read or a host reached through `tls.accept_invalid_certs`.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::ValueShape;
use crate::secret_reference::SecretReference;

/// One entry of `session.http_credentials` (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpCredential {
    /// The HTTP authentication username.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub username: String,
    /// The HTTP authentication password, as a Secret reference (`env:`,
    /// `keychain:`, `op://`, `caller:`), never a raw value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub password: String,
    /// The registrable domains (or `*`) allowed to receive these credentials
    /// on an HTTP 401 challenge; required and non-empty.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
}

/// Validates a list of [`HttpCredential`] entries.
fn check_credentials(value: &serde_json::Value) -> Result<(), String> {
    let list: Vec<HttpCredential> = serde_json::from_value(value.clone()).map_err(|error| {
        format!(
            "expected a list of `{{ username, password, domains }}` entries with `password` a Secret reference: {error}"
        )
    })?;
    for (index, entry) in list.iter().enumerate() {
        SecretReference::parse(&entry.password).map_err(|error| {
            format!("entry {index}'s `password` is not a valid Secret reference: {error}")
        })?;
        if entry.domains.is_empty() {
            return Err(format!(
                "entry {index} must list at least one registrable domain (or `*`) in `domains`"
            ));
        }
        for domain in &entry.domains {
            if domain.trim().is_empty() {
                return Err(format!("entry {index} contains an empty domain"));
            }
        }
    }
    Ok(())
}

/// `session.http_credentials`'s values: a list of [`HttpCredential`].
pub const HTTP_CREDENTIALS_SHAPE: ValueShape = ValueShape {
    name: "HttpCredentials",
    check: check_credentials,
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<Vec<HttpCredential>>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![json!([{
            "username": "alice",
            "password": "env:SITE_PASSWORD",
            "domains": ["example.com"],
        }])]
    },
};
