//! `egress.private_ranges` (ADRs 0016 and 0017): the ranges and hosts the
//! addresses a page can reach may use although they are not globally
//! reachable, for tests against a local site or an intranet; and
//! `egress.recipe_private_ranges`, the Operator policy's allowance that
//! lets a Recipe set it.

use std::net::IpAddr;

use serde_json::{Value, json};

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The longest host name an entry may be (RFC 1035's 253 octets).
const HOST_NAME_LIMIT: usize = 253;

/// One entry of `egress.private_ranges`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivateRange {
    /// A CIDR block, or one address when `prefix` is its family's length.
    Block {
        /// Its address as written.
        first: IpAddr,
        /// Its length in bits.
        prefix: u8,
    },
    /// A host name, in lower case and without a trailing dot, whose
    /// connections may use private addresses, a cloud-metadata address
    /// never among them.
    Host(String),
}

impl PrivateRange {
    /// Reads one entry: `10.0.0.0/8`, `127.0.0.1`, `fd00::/8` or
    /// `intranet.example`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message.
    pub fn parse(entry: &str) -> Result<Self, String> {
        let invalid =
            || format!("expected a CIDR range, an IP address or a host name, found `{entry}`");
        if let Some((address, prefix)) = entry.split_once('/') {
            let first: IpAddr = address.parse().map_err(|_| invalid())?;
            let bits = if first.is_ipv4() { 32 } else { 128 };
            let prefix = prefix
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= bits)
                .ok_or_else(invalid)?;
            return Ok(Self::Block { first, prefix });
        }
        if let Ok(first) = entry.trim_matches(['[', ']']).parse::<IpAddr>() {
            let prefix = if first.is_ipv4() { 32 } else { 128 };
            return Ok(Self::Block { first, prefix });
        }
        let host = entry
            .strip_suffix('.')
            .unwrap_or(entry)
            .to_ascii_lowercase();
        let label_ok = |label: &str| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        };
        if host.is_empty() || host.len() > HOST_NAME_LIMIT || !host.split('.').all(label_ok) {
            return Err(invalid());
        }
        Ok(Self::Host(host))
    }

    /// Reads the setting's value, a list of entries.
    ///
    /// # Errors
    ///
    /// What is wrong with the first entry that does not read.
    pub fn list(value: &Value) -> Result<Vec<Self>, String> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::Array(entries) => entries
                .iter()
                .map(|entry| match entry.as_str() {
                    Some(text) => Self::parse(text),
                    None => Err(format!(
                        "expected a CIDR range, an IP address or a host name, found `{entry}`"
                    )),
                })
                .collect(),
            _ => Err(format!(
                "expected a list of CIDR ranges, IP addresses and host names, found `{value}`"
            )),
        }
    }
}

/// `egress.private_ranges`' values: what [`PrivateRange::list`] reads.
pub const PRIVATE_RANGES_SHAPE: ValueShape = ValueShape {
    name: "PrivateRanges",
    check: |value| PrivateRange::list(value).map(drop),
    schema: || {
        json!({
            "type": "array",
            "items": {
                "type": "string",
                "description": "A CIDR range such as `127.0.0.0/8`, an IP address such as `169.254.169.254`, or a host name such as `intranet.example`.",
            },
        })
    },
    examples: || vec![json!(["127.0.0.0/8", "intranet.example"])],
};

fn empty_list() -> Value {
    json!([])
}

fn no() -> Value {
    Value::Bool(false)
}

fn lists_any(value: &Value) -> bool {
    value.as_array().is_some_and(|entries| !entries.is_empty())
}

/// `egress.private_ranges` (ADRs 0016 and 0017; spec #563's shape).
pub const EGRESS_PRIVATE_RANGES: Setting = Setting {
    name: "egress.private_ranges",
    description: "The ranges and hosts that addresses a page can reach may use although they are not globally reachable, for tests against a local site or an intranet: CIDR ranges such as `127.0.0.0/8`, IP addresses, and host names, each name's connections allowed any address that is not globally reachable. A cloud-metadata address (`169.254.169.254`, `fd00:ec2::254`, `100.100.100.200`) is reached only when listed by its exact address, never through a range or a host name that covers it. Empty by default; set by the program's own code, configuration or an Operator policy, by a Recipe only where the Operator policy's `egress.recipe_private_ranges` allows it, and never by a Client acting for a model, whose value is `setting_locked`. A run that lists any records it once in the Audit log as `private_addresses_allowed` and in its Run report's `weakened`.",
    kind: ValueKind::Shape(&PRIVATE_RANGES_SHAPE),
    default: empty_list,
    clearable: false,
    authority: Authority::NotByModelRecipeAllowedBy(EGRESS_RECIPE_PRIVATE_RANGES.name),
    weakens: Some(lists_any),
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `egress.recipe_private_ranges`: the Operator policy's allowance for a
/// Recipe to set `egress.private_ranges`, which ticket #622's "a Recipe may
/// set it only where Operator policy allows it" asks for (ADR 0017's note).
pub const EGRESS_RECIPE_PRIVATE_RANGES: Setting = Setting {
    name: "egress.recipe_private_ranges",
    description: "Whether a Recipe may set `egress.private_ranges`. `false` by default, so a Recipe's value is `setting_locked`; set only in an Operator policy.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::OperatorPolicy,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};
