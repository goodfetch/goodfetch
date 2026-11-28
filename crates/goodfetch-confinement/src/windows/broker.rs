//! Windows filter broker IPC wire contract, persistent WFP filter compiler,
//! named-pipe SDDL builder, and metadata tag format (ADRs 0033 and 0039,
//! issue 879).
//!
//! Pure cross-platform types shared by `goodfetch-filter-broker` (the Windows
//! service), `goodfetch-confinement` (the Windows launcher client and probes),
//! and `goodfetch-engine`. Contains no `unsafe` code.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use goodfetch_protocol::plugin::NetworkLevel;
use goodfetch_protocol::plugin::index::ProvenanceAttestation;
use serde::{Deserialize, Serialize};

/// Default named pipe served by `goodfetch-filter-broker` on Windows.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\goodfetch-filter-broker";

/// Windows Service Control Manager name of the filter broker service.
pub const BROKER_SERVICE_NAME: &str = "GoodfetchFilterBroker";

/// Human-readable display name registered with Windows Service Control Manager.
pub const BROKER_DISPLAY_NAME: &str = "Goodfetch Filter Broker";

/// Windows Event Log source name under which every filter change is logged.
pub const BROKER_EVENT_SOURCE: &str = "GoodfetchFilterBroker";

/// Default installation path of `goodfetch-filter-broker.exe` on Windows.
pub const DEFAULT_BROKER_DEST: &str = r"C:\Program Files\Goodfetch\goodfetch-filter-broker.exe";

/// Persistent WFP provider GUID owned exclusively by `goodfetch-filter-broker`
/// (`5960a7b1-4c2d-4f8e-9b12-7d3e8f0a1b2c`).
pub const PROVIDER_GUID: u128 = 0x5960_a7b1_4c2d_4f8e_9b12_7d3e_8f0a_1b2c;

/// Persistent WFP sublayer GUID owned exclusively by `goodfetch-filter-broker`
/// (`5960a7b2-4c2d-4f8e-9b12-7d3e8f0a1b2c`).
pub const SUBLAYER_GUID: u128 = 0x5960_a7b2_4c2d_4f8e_9b12_7d3e_8f0a_1b2c;

/// Prefix embedded in every persistent WFP filter's display name so a
/// restarted broker can identify its own filters and their `AppContainer`
/// across crashes (`goodfetch-filter-broker:<app_container>:<instance>:<rule>`).
pub const FILTER_TAG_PREFIX: &str = "goodfetch-filter-broker:";

/// TCP IP protocol number (`IPPROTO_TCP`).
pub const IPPROTO_TCP: u8 = 6;

/// One operator-configured private or loopback destination that a `direct`
/// Plugin on Windows is permitted to connect to (ADR 0033).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedDestination {
    /// Target IP address (IPv4 or IPv6). Link-local and cloud-metadata
    /// addresses are always refused regardless of configuration (ADR 0033).
    pub ip: IpAddr,
    /// Optional TCP/UDP destination port; when `None`, any port on `ip` is
    /// allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Which WFP ALE layer a filter attaches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterLayer {
    AleAuthConnectV4,
    AleAuthConnectV6,
    AleAuthRecvAcceptV4,
    AleAuthRecvAcceptV6,
}

/// Whether a WFP filter blocks or permits matching traffic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterAction {
    Block,
    Permit,
}

/// An IPv4 address + subnet mask or IPv6 address + prefix length matched by a
/// WFP remote-address condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum IpRange {
    V4 { addr: Ipv4Addr, mask: Ipv4Addr },
    V6 { addr: Ipv6Addr, prefix_len: u8 },
}

/// One persistent WFP filter rule compiled for a live `AppContainer` instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WfpRule {
    /// Tagged filter name (`goodfetch-filter-broker:<app_container>:<instance>:<rule>`).
    pub tag: String,
    /// Short rule identifier (e.g. `block.inbound_v4`, `permit.proxy_loopback`).
    pub rule_name: String,
    /// ALE layer on which this filter runs.
    pub layer: FilterLayer,
    /// Block or permit.
    pub action: FilterAction,
    /// Filter weight within [`SUBLAYER_GUID`]: higher weights win.
    /// - `100`: non-overridable blocks (all inbound; link-local & metadata)
    /// - `80`: operator-configured private/loopback permits at `direct`
    /// - `50`: `proxy_tcp` loopback permit, or `direct` private/loopback blocks
    /// - `10`: `proxy_tcp` default-deny outbound blocks
    pub weight: u8,
    /// Optional IP protocol match (e.g. `Some(IPPROTO_TCP)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u8>,
    /// Optional remote IP range condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_ip: Option<IpRange>,
    /// Optional remote port condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
}

/// Why a broker request or filter specification was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum BrokerProtocolError {
    #[error("network level `{0}` does not use the Windows filter broker")]
    UnsupportedNetworkLevel(String),
    #[error("`proxy_tcp` requires a non-zero loopback `proxy_port`")]
    MissingProxyPort,
    #[error("AppContainer name `{0}` is invalid (must be non-empty and contain no `:`)")]
    InvalidAppContainerName(String),
    #[error("instance ID `{0}` is invalid (must be non-empty and contain no `:`)")]
    InvalidInstanceId(String),
    #[error("link-local or cloud-metadata address `{0}` can never be allowed for a Plugin")]
    LinkLocalOrMetadataForbidden(IpAddr),
    #[error("installing user SID `{0}` is invalid for a named-pipe SDDL")]
    InvalidUserSid(String),
}

/// A message sent from an engine (or `plugin-shim` / test harness) to
/// `goodfetch-filter-broker` over the named pipe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BrokerRequest {
    /// Authenticates this engine connection by supplying its Goodfetch release
    /// provenance attestation; the broker verifies the connected peer PID's
    /// executable digest against this attestation under its bundled Sigstore
    /// trust root.
    Authenticate {
        engine_id: String,
        attestation: ProvenanceAttestation,
    },
    /// Liveness check (`windows::host::read`).
    Ping,
    /// Installs persistent WFP filters (refusing filters first) for one
    /// `AppContainer` BEFORE the instance starts, and attaches the broker to
    /// the instance's Job Object handle.
    PrepareInstance {
        instance_id: String,
        app_container_name: String,
        /// Numeric `HANDLE` value of the instance's Job Object in the calling
        /// process (duplicated by the broker via `DuplicateHandle`).
        job_handle: u64,
        network_level: NetworkLevel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proxy_port: Option<u16>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_destinations: Vec<AllowedDestination>,
    },
    /// Terminates the instance's Job Object (if needed), waits until it has
    /// zero active processes, and only then removes its persistent WFP filters.
    ReleaseInstance { instance_id: String },
    /// Acknowledges a `ServiceStopping` notification once the engine has
    /// stopped every dependent Plugin instance under the crash rules.
    DrainAck,
}

/// A reply or server-initiated notification sent from `goodfetch-filter-broker`
/// over the named pipe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrokerResponse {
    /// Connection authenticated against the bundled Sigstore trust root.
    Authenticated { engine_id: String },
    /// Liveness acknowledgement.
    Pong,
    /// All refusing and permitting persistent filters are active in WFP; the
    /// instance may now start.
    Prepared {
        instance_id: String,
        app_container_name: String,
        filter_ids: Vec<u64>,
    },
    /// The instance's Job Object is empty and all of its persistent filters
    /// have been removed and logged.
    Released {
        instance_id: String,
        removed_filters: Vec<u64>,
    },
    /// The broker service is stopping: the engine must immediately stop every
    /// dependent (`proxy_tcp` / `direct`) instance under the crash rules and
    /// release it before sending `DrainAck`.
    ServiceStopping,
    /// Drain acknowledgement received.
    Drained,
    /// Request refused (authentication failure, invalid parameters, or OS error).
    Error { code: String, message: String },
}

/// Formats the metadata tag stored as a persistent WFP filter's display name.
#[must_use]
pub fn format_filter_tag(app_container_name: &str, instance_id: &str, rule_name: &str) -> String {
    format!("{FILTER_TAG_PREFIX}{app_container_name}:{instance_id}:{rule_name}")
}

/// Parses a persistent WFP filter's display name tag back into
/// `(app_container_name, instance_id, rule_name)`.
#[must_use]
pub fn parse_filter_tag(tag: &str) -> Option<(&str, &str, &str)> {
    let rest = tag.strip_prefix(FILTER_TAG_PREFIX)?;
    let (app_container, rest) = rest.split_once(':')?;
    let (instance_id, rule_name) = rest.split_once(':')?;
    if app_container.is_empty() || instance_id.is_empty() || rule_name.is_empty() {
        return None;
    }
    Some((app_container, instance_id, rule_name))
}

/// Returns `true` if `ip` is a link-local or cloud-metadata address that must
/// NEVER be allowed for any Plugin even when configured by an operator
/// (ADR 0033).
#[must_use]
pub fn is_link_local_or_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            // 169.254.0.0/16 covers both IPv4 link-local and AWS/GCP/Azure IMDS (169.254.169.254).
            a == 169 && b == 254
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_link_local_or_metadata(IpAddr::V4(mapped));
            }
            let segments = v6.segments();
            // fe80::/10 link-local
            if (segments[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // AWS IPv6 IMDS fd00:ec2::254
            v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)
        }
    }
}

/// Returns `true` if `ip` requires an `AppContainer` loopback exemption on
/// Windows (`127.0.0.0/8` or `::1`).
#[must_use]
pub fn needs_loopback_exemption(
    level: NetworkLevel,
    allowed_destinations: &[AllowedDestination],
) -> bool {
    match level {
        NetworkLevel::None => false,
        NetworkLevel::ProxyTcp => true,
        NetworkLevel::Direct => allowed_destinations
            .iter()
            .any(|dest| dest.ip.is_loopback()),
    }
}

/// Builds the Windows SDDL security descriptor restricting the broker's named
/// pipe exclusively to `LocalSystem` (`SY`), `Administrators` (`BA`), and the
/// installing user (`user_sid`), denying every other local user (ADR 0033).
///
/// # Errors
///
/// Returns [`BrokerProtocolError::InvalidUserSid`] if `user_sid` does not look
/// like a valid Windows SID (`S-1-...` with digits and hyphens only).
pub fn pipe_sddl_for_user_sid(user_sid: &str) -> Result<String, BrokerProtocolError> {
    let valid = user_sid.starts_with("S-1-")
        && user_sid.len() > 4
        && user_sid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-' || byte == b'S');
    if !valid {
        return Err(BrokerProtocolError::InvalidUserSid(user_sid.to_owned()));
    }
    // D:P  -> Protected DACL (blocks inherited ACEs)
    // SY   -> LocalSystem (the service itself)
    // BA   -> Builtin Administrators (elevated maintenance)
    // SID  -> The installing user alone (read/write pipe access)
    Ok(format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{user_sid})"))
}

fn make_rule(
    app_container_name: &str,
    instance_id: &str,
    rule_name: &str,
    layer: FilterLayer,
    action: FilterAction,
    weight: u8,
    (protocol, remote_ip, remote_port): (Option<u8>, Option<IpRange>, Option<u16>),
) -> WfpRule {
    WfpRule {
        tag: format_filter_tag(app_container_name, instance_id, rule_name),
        rule_name: rule_name.to_owned(),
        layer,
        action,
        weight,
        protocol,
        remote_ip,
        remote_port,
    }
}

fn push_inbound_blocks(rules: &mut Vec<WfpRule>, app_container: &str, instance_id: &str) {
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.inbound_v4",
        FilterLayer::AleAuthRecvAcceptV4,
        FilterAction::Block,
        100,
        (None, None, None),
    ));
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.inbound_v6",
        FilterLayer::AleAuthRecvAcceptV6,
        FilterAction::Block,
        100,
        (None, None, None),
    ));
}

fn compile_proxy_tcp_rules(app_container: &str, instance_id: &str, port: u16) -> Vec<WfpRule> {
    let mut rules = Vec::with_capacity(5);
    push_inbound_blocks(&mut rules, app_container, instance_id);
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.outbound_v4",
        FilterLayer::AleAuthConnectV4,
        FilterAction::Block,
        10,
        (None, None, None),
    ));
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.outbound_v6",
        FilterLayer::AleAuthConnectV6,
        FilterAction::Block,
        10,
        (None, None, None),
    ));
    rules.push(make_rule(
        app_container,
        instance_id,
        "permit.proxy_loopback",
        FilterLayer::AleAuthConnectV4,
        FilterAction::Permit,
        50,
        (
            Some(IPPROTO_TCP),
            Some(IpRange::V4 {
                addr: Ipv4Addr::LOCALHOST,
                mask: Ipv4Addr::BROADCAST,
            }),
            Some(port),
        ),
    ));
    rules
}

const DIRECT_PRIVATE_V4: &[(&str, Ipv4Addr, Ipv4Addr)] = &[
    (
        "block.this_net_v4",
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::new(255, 0, 0, 0),
    ),
    (
        "block.rfc1918_10",
        Ipv4Addr::new(10, 0, 0, 0),
        Ipv4Addr::new(255, 0, 0, 0),
    ),
    (
        "block.cgnat_v4",
        Ipv4Addr::new(100, 64, 0, 0),
        Ipv4Addr::new(255, 192, 0, 0),
    ),
    (
        "block.loopback_v4",
        Ipv4Addr::new(127, 0, 0, 0),
        Ipv4Addr::new(255, 0, 0, 0),
    ),
    (
        "block.rfc1918_172",
        Ipv4Addr::new(172, 16, 0, 0),
        Ipv4Addr::new(255, 240, 0, 0),
    ),
    (
        "block.rfc1918_192",
        Ipv4Addr::new(192, 168, 0, 0),
        Ipv4Addr::new(255, 255, 0, 0),
    ),
    (
        "block.multicast_v4",
        Ipv4Addr::new(224, 0, 0, 0),
        Ipv4Addr::new(240, 0, 0, 0),
    ),
    (
        "block.reserved_v4",
        Ipv4Addr::new(240, 0, 0, 0),
        Ipv4Addr::new(240, 0, 0, 0),
    ),
];

const DIRECT_PRIVATE_V6: &[(&str, Ipv6Addr, u8)] = &[
    ("block.unspecified_v6", Ipv6Addr::UNSPECIFIED, 128),
    ("block.loopback_v6", Ipv6Addr::LOCALHOST, 128),
    (
        "block.ula_v6",
        Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0),
        7,
    ),
    (
        "block.multicast_v6",
        Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0),
        8,
    ),
    (
        "block.v4mapped_loopback_v6",
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 0),
        104,
    ),
    (
        "block.v4mapped_rfc1918_10_v6",
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0),
        104,
    ),
    (
        "block.v4mapped_link_local_v6",
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xa9fe, 0),
        112,
    ),
    (
        "block.v4mapped_rfc1918_172_v6",
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xac10, 0),
        108,
    ),
    (
        "block.v4mapped_rfc1918_192_v6",
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xc0a8, 0),
        112,
    ),
];

fn push_metadata_blocks(rules: &mut Vec<WfpRule>, app_container: &str, instance_id: &str) {
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.link_local_metadata_v4",
        FilterLayer::AleAuthConnectV4,
        FilterAction::Block,
        100,
        (
            None,
            Some(IpRange::V4 {
                addr: Ipv4Addr::new(169, 254, 0, 0),
                mask: Ipv4Addr::new(255, 255, 0, 0),
            }),
            None,
        ),
    ));
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.link_local_v6",
        FilterLayer::AleAuthConnectV6,
        FilterAction::Block,
        100,
        (
            None,
            Some(IpRange::V6 {
                addr: Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0),
                prefix_len: 10,
            }),
            None,
        ),
    ));
    rules.push(make_rule(
        app_container,
        instance_id,
        "block.metadata_imds_v6",
        FilterLayer::AleAuthConnectV6,
        FilterAction::Block,
        100,
        (
            None,
            Some(IpRange::V6 {
                addr: Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254),
                prefix_len: 128,
            }),
            None,
        ),
    ));
}

fn compile_direct_rules(
    app_container: &str,
    instance_id: &str,
    allowed_destinations: &[AllowedDestination],
) -> Vec<WfpRule> {
    let mut rules = Vec::new();
    push_inbound_blocks(&mut rules, app_container, instance_id);
    push_metadata_blocks(&mut rules, app_container, instance_id);
    for &(name, addr, mask) in DIRECT_PRIVATE_V4 {
        rules.push(make_rule(
            app_container,
            instance_id,
            name,
            FilterLayer::AleAuthConnectV4,
            FilterAction::Block,
            50,
            (None, Some(IpRange::V4 { addr, mask }), None),
        ));
    }
    for &(name, addr, prefix_len) in DIRECT_PRIVATE_V6 {
        rules.push(make_rule(
            app_container,
            instance_id,
            name,
            FilterLayer::AleAuthConnectV6,
            FilterAction::Block,
            50,
            (None, Some(IpRange::V6 { addr, prefix_len }), None),
        ));
    }
    for (index, dest) in allowed_destinations.iter().enumerate() {
        let (layer, range) = match dest.ip {
            IpAddr::V4(addr) => (
                FilterLayer::AleAuthConnectV4,
                IpRange::V4 {
                    addr,
                    mask: Ipv4Addr::BROADCAST,
                },
            ),
            IpAddr::V6(addr) => (
                FilterLayer::AleAuthConnectV6,
                IpRange::V6 {
                    addr,
                    prefix_len: 128,
                },
            ),
        };
        rules.push(make_rule(
            app_container,
            instance_id,
            &format!("permit.configured_{index}"),
            layer,
            FilterAction::Permit,
            80,
            (None, Some(range), dest.port),
        ));
    }
    rules
}

/// Compiles the ordered list of persistent WFP filter rules for one live
/// `AppContainer` instance at `level` (`ProxyTcp` or `Direct`).
///
/// **Invariant:** Every [`FilterAction::Block`] (refusing) rule is emitted
/// strictly BEFORE every [`FilterAction::Permit`] rule, so callers adding rules
/// in order guarantee refusing filters are always in place first (ADR 0033).
///
/// # Errors
///
/// Returns [`BrokerProtocolError`] when `level` is `None`, `proxy_tcp` omits a
/// valid `proxy_port`, `app_container_name`/`instance_id` are invalid, or any
/// `allowed_destinations` entry targets a link-local or cloud-metadata address.
pub fn compile_rules(
    app_container_name: &str,
    instance_id: &str,
    level: NetworkLevel,
    proxy_port: Option<u16>,
    allowed_destinations: &[AllowedDestination],
) -> Result<Vec<WfpRule>, BrokerProtocolError> {
    if app_container_name.is_empty() || app_container_name.contains(':') {
        return Err(BrokerProtocolError::InvalidAppContainerName(
            app_container_name.to_owned(),
        ));
    }
    if instance_id.is_empty() || instance_id.contains(':') {
        return Err(BrokerProtocolError::InvalidInstanceId(
            instance_id.to_owned(),
        ));
    }
    for dest in allowed_destinations {
        if is_link_local_or_metadata(dest.ip) {
            return Err(BrokerProtocolError::LinkLocalOrMetadataForbidden(dest.ip));
        }
    }
    match level {
        NetworkLevel::None => Err(BrokerProtocolError::UnsupportedNetworkLevel(
            level.as_str().to_owned(),
        )),
        NetworkLevel::ProxyTcp => {
            let Some(port) = proxy_port.filter(|&port| port != 0) else {
                return Err(BrokerProtocolError::MissingProxyPort);
            };
            Ok(compile_proxy_tcp_rules(
                app_container_name,
                instance_id,
                port,
            ))
        }
        NetworkLevel::Direct => Ok(compile_direct_rules(
            app_container_name,
            instance_id,
            allowed_destinations,
        )),
    }
}
