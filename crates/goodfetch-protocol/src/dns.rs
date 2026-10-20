//! The Error codes of name resolution: the engine's own DNS resolver, which
//! every Product's static musl executable uses on Linux and every Session
//! with a local address uses on every OS, and the system resolver the engine
//! asks on macOS and Windows (ADRs 0024, 0044 and 0059).
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A name no resolver Goodfetch asked has an address for.
pub const NAME_NOT_RESOLVED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "name_not_resolved",
    description: "The resolver found no address for the host name. The engine's own resolver, used on Linux and for Sessions with a local address, reads the system's resolver configuration and hosts file but asks neither NSS modules (such as a directory service or a VPN client's) nor multicast DNS, so a name only they resolve, such as a `.local` name, fails here. Map such a name to its address with the `resolve` setting, or correct the name.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A resolver whose name servers gave no usable answer.
pub const NAME_LOOKUP_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "name_lookup_failed",
    description: "The engine's own resolver got no usable answer for the host name: its name servers timed out, refused the query, failed or could not be reached. Check the name servers of the system's resolver configuration, or map the name with the `resolve` setting.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Which resolver the engine asked for a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NameResolver {
    /// The engine's own resolver, which reads the system's resolver
    /// configuration and hosts file and asks its name servers itself.
    Engine,
    /// The operating system's resolver, on macOS and Windows.
    System,
}

/// What `name_not_resolved` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NameNotResolvedDetails {
    /// The host name asked for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The resolver that found no address.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resolver: NameResolver,
    /// Whether the name is a `.local` name, which multicast DNS answers on
    /// most networks and the engine's own resolver never asks it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub multicast_dns_name: bool,
}

/// What `name_lookup_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NameLookupFailedDetails {
    /// The host name asked for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The resolver whose name servers failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resolver: NameResolver,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[NAME_NOT_RESOLVED, NAME_LOOKUP_FAILED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
