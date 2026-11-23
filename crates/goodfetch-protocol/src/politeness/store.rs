//! The Politeness store's setting (ADR 0053): `politeness.store`, the Valkey
//! or Redis server through which engines on several machines share
//! Politeness, set on the machine, in user configuration or an Operator
//! policy, with the namespace every engine sharing it names. It is reached
//! as one server, through Sentinel or as a Cluster; TLS is required off
//! loopback, and the password is a Secret reference (ADR 0015).

use std::borrow::Cow;
use std::net::IpAddr;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};
use crate::secret_reference::{REFERENCE_PATTERN, SecretReference};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a namespace may hold: anything but a brace.
const NAMESPACE_PATTERN: &str = "^[^{}]+$";

/// What a store's address looks like; [`StoreUrl::parse`] says the rest.
const URL_PATTERN: &str = "^(rediss?|redis\\+unix|unix)://[^@?#]*$";

/// The namespace every engine sharing one Politeness names: a non-empty
/// name with no brace, which would make a hash tag of its own and move a
/// key's parts across Cluster slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreNamespace(String);

impl StoreNamespace {
    /// Reads `text` as a namespace.
    ///
    /// # Errors
    ///
    /// When it is empty or holds a brace.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.contains(['{', '}']) {
            return Err("a store's `namespace` is a name with no `{` or `}`".to_owned());
        }
        Ok(Self(text.to_owned()))
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for StoreNamespace {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for StoreNamespace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for StoreNamespace {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "StoreNamespace".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": NAMESPACE_PATTERN })
    }
}

/// The address of one of the store's servers, sentinels or Cluster nodes:
/// `rediss://` (TLS) anywhere, `redis://` on a loopback host only, or a
/// Unix socket on this machine (`redis+unix://`, `unix://`). It carries no
/// user, password or query, so no password reaches it in clear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreUrl(String);

impl StoreUrl {
    /// Reads `text` as a store's address.
    ///
    /// # Errors
    ///
    /// What is wrong with it, never repeating it, since a password may
    /// have been pasted into it.
    pub fn parse(text: &str) -> Result<Self, String> {
        const SHAPE: &str = "a store's address is a `rediss://`, `redis://` or `redis+unix://` URL";
        let Some((scheme, rest)) = text.split_once("://") else {
            return Err(SHAPE.to_owned());
        };
        if rest.contains(['@', '?', '#']) {
            return Err("a store's address carries no user, password or query: set `username`, and `password` as a Secret reference such as `env:VALKEY_PASSWORD`".to_owned());
        }
        match scheme {
            "redis+unix" | "unix" if rest.starts_with('/') => Ok(Self(text.to_owned())),
            "rediss" | "redis" => {
                let authority = rest.split('/').next().unwrap_or_default();
                let host = if let Some(bracketed) = authority.strip_prefix('[') {
                    bracketed.split(']').next().unwrap_or_default()
                } else {
                    authority.split(':').next().unwrap_or_default()
                };
                if host.is_empty() {
                    return Err("a store's address names its host".to_owned());
                }
                if scheme == "redis" && !is_loopback(host) {
                    return Err("`redis://` would send the store's password and keys in clear: off loopback a store's address is `rediss://` (TLS)".to_owned());
                }
                Ok(Self(text.to_owned()))
            }
            _ => Err(SHAPE.to_owned()),
        }
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is reached over TLS.
    pub fn tls(&self) -> bool {
        self.0.starts_with("rediss://")
    }
}

/// Whether `host` is this machine's loopback: `localhost` or a loopback
/// address.
fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

impl Serialize for StoreUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for StoreUrl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for StoreUrl {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "StoreUrl".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": URL_PATTERN })
    }
}

/// How the store's servers are reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoreAddressing {
    /// One server.
    Server {
        /// Its address.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: StoreUrl,
    },
    /// The primary that Sentinel elects for a service, found again through
    /// the sentinels after a failover.
    Sentinel {
        /// The service whose primary the sentinels name.
        #[schemars(length(min = 1), extend("arrived_in" = "0.1"))]
        service: String,
        /// The sentinels' addresses, at least one; the primary is reached
        /// over TLS when they are.
        #[schemars(length(min = 1), extend("arrived_in" = "0.1"))]
        sentinels: Vec<StoreUrl>,
    },
    /// A Cluster, discovered from these nodes, at least one; the nodes it
    /// finds are reached as these are.
    Cluster {
        /// The nodes' addresses.
        #[schemars(length(min = 1), extend("arrived_in" = "0.1"))]
        nodes: Vec<StoreUrl>,
    },
}

/// The Secret reference's text, as `password` writes it.
mod reference_text {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::secret_reference::SecretReference;

    #[expect(
        clippy::ref_option,
        reason = "serde's `with` hands a field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(
        reference: &Option<SecretReference>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match reference {
            Some(reference) => serializer.serialize_some(reference.as_str()),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SecretReference>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        text.map(|text| {
            SecretReference::parse(&text).map_err(|error| {
                serde::de::Error::custom(format!(
                    "`password` is a Secret reference such as `env:VALKEY_PASSWORD`: {error}; a password is never taken in its place"
                ))
            })
        })
        .transpose()
    }
}

/// `politeness.store`'s value: the Politeness store and the namespace this
/// engine shares Politeness under (ADR 0053).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolitenessStoreConfiguration {
    /// The namespace: every engine that names it shares Politeness, and
    /// shares nothing with another namespace on the same store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub namespace: StoreNamespace,
    /// How the store's servers are reached.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub addressing: StoreAddressing,
    /// The user the store's access list names; its default user when left
    /// out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub username: Option<String>,
    /// The Secret reference of the user's password, resolved when the
    /// engine connects; none for a store without one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "reference_text"
    )]
    #[schemars(with = "Option<String>", pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub password: Option<SecretReference>,
}

impl PolitenessStoreConfiguration {
    /// Reads a value of `politeness.store`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message; it never
    /// repeats a password given in place of its reference.
    pub fn read(value: &Value) -> Result<Self, String> {
        let configuration: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("expected a Politeness store: {error}"))?;
        match &configuration.addressing {
            StoreAddressing::Server { .. } => {}
            StoreAddressing::Sentinel { service, sentinels } => {
                if service.is_empty() {
                    return Err("a Sentinel store names its `service`".to_owned());
                }
                if sentinels.is_empty() {
                    return Err("a Sentinel store lists at least one of its `sentinels`".to_owned());
                }
            }
            StoreAddressing::Cluster { nodes } => {
                if nodes.is_empty() {
                    return Err("a Cluster store lists at least one of its `nodes`".to_owned());
                }
            }
        }
        Ok(configuration)
    }
}

impl crate::catalog::SettingJson for PolitenessStoreConfiguration {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// `politeness.store`'s values: a [`PolitenessStoreConfiguration`].
pub const POLITENESS_STORE_SHAPE: ValueShape = ValueShape {
    name: "PolitenessStoreConfiguration",
    check: |value| PolitenessStoreConfiguration::read(value).map(drop),
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<PolitenessStoreConfiguration>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![
            json!({
                "namespace": "crawl",
                "addressing": { "kind": "server", "url": "rediss://valkey.example:6380" },
                "username": "crawler",
                "password": "env:VALKEY_PASSWORD",
            }),
            json!({
                "namespace": "crawl",
                "addressing": {
                    "kind": "sentinel",
                    "service": "politeness",
                    "sentinels": ["rediss://sentinel.example:26379"],
                },
            }),
            json!({
                "namespace": "crawl",
                "addressing": { "kind": "cluster", "nodes": ["rediss://node.example:6379"] },
            }),
        ]
    },
};

fn none() -> Value {
    Value::Null
}

/// `politeness.store` (ADR 0053).
pub const POLITENESS_STORE: Setting = Setting {
    name: "politeness.store",
    description: "The Politeness store: a Valkey or Redis server through which every engine naming the same `namespace` shares Politeness across machines, for Runs at `politeness.sharing: shared`. `addressing` reaches it as one `server` by its `url`, through `sentinel`, by the `service` whose primary the `sentinels` elect, or as a `cluster` discovered from its `nodes`. Every address is `rediss://` (TLS), except a loopback host's `redis://` or a Unix socket's `redis+unix://`, and carries no user or password: `username` names the store's user and `password` is the Secret reference of its password, such as `env:VALKEY_PASSWORD`; a password given in its place is refused with `setting_invalid`. Set in user configuration or an Operator policy only. None by default, for Politeness shared on the machine alone.",
    kind: ValueKind::Shape(&POLITENESS_STORE_SHAPE),
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("politeness_store"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

#[cfg(test)]
mod tests;
