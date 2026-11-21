//! Local addresses (ADR 0044): `egress.local_addresses`, the addresses a
//! Session's connections leave from, as a list of addresses, a network
//! interface by name or an IPv6 prefix; `egress.rotation_block.ipv4` and
//! `.ipv6`, the sizes of the network blocks new Sessions are spread across;
//! and `local_address_unavailable`, the Error code of a value that yields
//! no address the host can use.

use std::fmt;
use std::net::{IpAddr, Ipv6Addr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind, ValueShape,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The longest interface name the shape takes: Windows' friendly names
/// are the longest any OS gives.
const INTERFACE_NAME_LIMIT: usize = 256;

/// What `egress.local_addresses` names (ADR 0044).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalAddresses {
    /// These addresses, each of the host.
    Addresses(Vec<IpAddr>),
    /// The addresses a network interface holds when a Session starts.
    Interface(String),
    /// The addresses of an IPv6 prefix.
    Prefix(Ipv6Prefix),
}

/// An IPv6 prefix: its first address and its length in bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Prefix {
    /// Its address, the bits past its length zero.
    pub network: Ipv6Addr,
    /// Its length, from 0 to 128.
    pub length: u8,
}

impl Ipv6Prefix {
    /// Whether `address` is in it.
    pub fn contains(&self, address: Ipv6Addr) -> bool {
        mask(address, self.length) == self.network
    }
}

/// `address` with every bit past the first `length` zero.
fn mask(address: Ipv6Addr, length: u8) -> Ipv6Addr {
    let bits = u128::from(address);
    let kept = u128::MAX.checked_shl(128 - u32::from(length)).unwrap_or(0);
    Ipv6Addr::from(bits & kept)
}

impl fmt::Display for Ipv6Prefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.network, self.length)
    }
}

/// The JSON [`LocalAddresses::parse`] reads it back from.
impl Serialize for LocalAddresses {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Addresses(addresses) => addresses.serialize(serializer),
            Self::Interface(name) => serializer.serialize_str(name),
            Self::Prefix(prefix) => serializer.collect_str(prefix),
        }
    }
}

impl LocalAddresses {
    /// Reads `egress.local_addresses`' value: a list of IP addresses, an
    /// IPv6 prefix such as `2001:db8:1::/48`, or an interface's name.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for a `setting_invalid` message.
    pub fn parse(value: &Value) -> Result<Self, String> {
        match value {
            Value::Array(entries) if entries.is_empty() => {
                Err("expected at least one address in the list".to_owned())
            }
            Value::Array(entries) => entries
                .iter()
                .map(|entry| match entry.as_str().map(str::parse::<IpAddr>) {
                    Some(Ok(address)) => Ok(address),
                    _ => Err(format!(
                        "expected an IP address in the list, found `{entry}`"
                    )),
                })
                .collect::<Result<_, _>>()
                .map(Self::Addresses),
            Value::String(text) => match text.split_once('/') {
                Some((network, length)) => prefix(text, network, length).map(Self::Prefix),
                None if text.is_empty()
                    || text.len() > INTERFACE_NAME_LIMIT
                    || text.chars().any(char::is_control) =>
                {
                    Err(format!("expected an interface's name, found `{text}`"))
                }
                None if text.parse::<IpAddr>().is_ok() => {
                    Err(format!("an address goes in a list, such as `[\"{text}\"]`"))
                }
                None => Ok(Self::Interface(text.clone())),
            },
            _ => Err(format!(
                "expected a list of IP addresses, an interface's name or an IPv6 prefix, found `{value}`"
            )),
        }
    }
}

/// The prefix `text` writes as `network/length`.
fn prefix(text: &str, network: &str, length: &str) -> Result<Ipv6Prefix, String> {
    let address = match network.parse::<IpAddr>() {
        Ok(IpAddr::V6(address)) => address,
        Ok(IpAddr::V4(_)) => {
            return Err(format!(
                "a prefix is an IPv6 one; list IPv4 addresses one by one, found `{text}`"
            ));
        }
        Err(_) => return Err(format!("expected an IPv6 prefix, found `{text}`")),
    };
    let length = length
        .parse::<u8>()
        .ok()
        .filter(|length| *length <= 128)
        .ok_or_else(|| format!("a prefix's length is from 0 to 128, found `{text}`"))?;
    Ok(Ipv6Prefix {
        network: mask(address, length),
        length,
    })
}

/// `egress.local_addresses`' values: what [`LocalAddresses::parse`] reads.
pub const LOCAL_ADDRESSES_SHAPE: ValueShape = ValueShape {
    name: "LocalAddresses",
    check: |value| LocalAddresses::parse(value).map(drop),
    schema: || {
        json!({
            "oneOf": [
                {
                    "type": "array",
                    "items": { "type": "string", "format": "ip" },
                    "minItems": 1,
                    "description": "IP addresses of the host.",
                },
                {
                    "type": "string",
                    "description": "An interface's name, such as `eth0`, or an IPv6 prefix, such as `2001:db8:1::/48`.",
                },
            ],
        })
    },
    examples: || {
        vec![
            json!(["192.0.2.10", "2001:db8::10"]),
            json!("eth0"),
            json!("2001:db8:1::/48"),
        ]
    },
};

fn none() -> Value {
    Value::Null
}

/// `egress.local_addresses` (ADR 0044's `local_addresses`).
pub const EGRESS_LOCAL_ADDRESSES: Setting = Setting {
    name: "egress.local_addresses",
    description: "The addresses a Session's connections and DNS queries leave from (ADR 0044): a list of the host's IP addresses, such as `[\"192.0.2.10\", \"2001:db8::10\"]`, a network interface's name, such as `eth0`, whose addresses are read when each Session starts, or an IPv6 prefix, such as `2001:db8:1::/48`. Each Session binds one address of each IP family for its life, new Sessions spread round-robin across the blocks `egress.rotation_block.ipv4` and `.ipv6` size. A prefix covers the host's addresses inside it, and on Linux any address of it once `net.ipv6.ip_nonlocal_bind` is 1 and the prefix is routed locally. A value that yields no address the host can use refuses the run with `local_address_unavailable`, and a site with no address in a family the Session holds fails with `address_family_unavailable`. None by default: the address the OS chooses. A Recipe may expose it but never sets it, since its addresses are the machine's own.",
    kind: ValueKind::Shape(&LOCAL_ADDRESSES_SHAPE),
    default: none,
    clearable: true,
    authority: Authority::NotByRecipe,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A block size of `bits` bits at most.
fn block_size(value: &Value, bits: u64) -> Result<(), String> {
    match value.as_u64() {
        Some(size) if size <= bits => Ok(()),
        _ => Err(format!(
            "expected a prefix length from 0 to {bits}, found `{value}`"
        )),
    }
}

/// `egress.rotation_block.ipv4`'s values: a prefix length from 0 to 32.
pub const IPV4_BLOCK_SHAPE: ValueShape = ValueShape {
    name: "Ipv4BlockSize",
    check: |value| block_size(value, 32),
    schema: || json!({ "type": "integer", "minimum": 0, "maximum": 32 }),
    examples: || vec![json!(24)],
};

/// `egress.rotation_block.ipv6`'s values: a prefix length from 0 to 128.
pub const IPV6_BLOCK_SHAPE: ValueShape = ValueShape {
    name: "Ipv6BlockSize",
    check: |value| block_size(value, 128),
    schema: || json!({ "type": "integer", "minimum": 0, "maximum": 128 }),
    examples: || vec![json!(64)],
};

fn twenty_four() -> Value {
    json!(24)
}

fn sixty_four() -> Value {
    json!(64)
}

/// `egress.rotation_block.ipv4` (ADR 0044).
pub const EGRESS_ROTATION_BLOCK_IPV4: Setting = Setting {
    name: "egress.rotation_block.ipv4",
    description: "The prefix length of the IPv4 blocks new Sessions are spread across, round-robin, when `egress.local_addresses` gives several: 24 by default, so consecutive Sessions leave from different /24 networks while the addresses allow it. Set for a connection or a Run.",
    kind: ValueKind::Shape(&IPV4_BLOCK_SHAPE),
    default: twenty_four,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `egress.rotation_block.ipv6` (ADR 0044).
pub const EGRESS_ROTATION_BLOCK_IPV6: Setting = Setting {
    name: "egress.rotation_block.ipv6",
    description: "The prefix length of the IPv6 blocks new Sessions are spread across, round-robin, when `egress.local_addresses` gives several: 64 by default, the block a rate limiter such as Cloudflare's counts as one client. Set for a connection or a Run.",
    kind: ValueKind::Shape(&IPV6_BLOCK_SHAPE),
    default: sixty_four,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A value of `egress.local_addresses` that yields no address the host can
/// use (ADR 0044).
pub const LOCAL_ADDRESS_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "local_address_unavailable",
    description: "`egress.local_addresses` yields no address this host can send from: a listed address the host does not hold, an interface it does not have or that holds no address, or an IPv6 prefix with none of the host's addresses in it and, on Linux, not routed locally with `net.ipv6.ip_nonlocal_bind` on; or a Session whose browser this engine does not launch on this machine (a Borrowed browser, a third-party CDP browser, a Browser host's) or whose `https://` proxy the egress relay does not chain through. The binding is never ignored: assign the address, correct the value, route the prefix to the host, or open the Session on a browser the engine launches here.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Why `egress.local_addresses` yields no usable address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalAddressFailure {
    /// A listed address is none the host can send from.
    NotHeld,
    /// No interface has this name.
    UnknownInterface,
    /// The interface holds no address.
    InterfaceWithoutAddress,
    /// The prefix holds none of the host's addresses, and the host does not
    /// let a socket bind any address of it.
    PrefixUnusable,
    /// The Session's browser is not one this engine launches on this
    /// machine: a Borrowed browser, a third-party CDP browser, or a Browser
    /// host's, whose connections no address of this machine can be claimed
    /// for (ADRs 0044 and 0056).
    NotLaunchedHere,
    /// The Session's proxy is one the egress relay does not chain through
    /// (an `https://` proxy), so its browser would connect to it from an
    /// address the OS chooses.
    ProxyNotRelayed,
}

/// What `local_address_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalAddressUnavailableDetails {
    /// The address, interface or prefix that yields nothing, as the
    /// setting writes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entry: String,
    /// Why it yields nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: LocalAddressFailure,
    /// The setting it comes from: `egress.local_addresses`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: String,
}

#[cfg(test)]
mod tests;
