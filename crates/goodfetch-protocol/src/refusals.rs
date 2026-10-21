//! The generated refusal cases (ADR 0066): for every registered format, a
//! fabricated version, the rule that decides it and the verdict that rule's
//! table gives; for a protocol, the fabricated `hello` of each who opens it,
//! the Product check of every pair of Products among them (ADRs 0094 and
//! 0095), and the reply it gets. `xtask compat refusals` writes one file per
//! case under `generated/refusals/<format>/`, and `xtask codegen --check`
//! holds them to the registry.

use serde::{Deserialize, Serialize};

use crate::catalog::{Product, ProtocolVersion};
use crate::formats::{Compatibility, HelloFrom, Rule};

/// One generated refusal case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalCase {
    /// The format's name.
    pub format: String,
    /// The rule that decides the version: the format's own, or, for a
    /// Browser host's `hello`, [`Rule::LowerMinor`] (ADR 0096).
    pub rule: Rule,
    /// The reader's version, which is the format's current one.
    pub reader: ProtocolVersion,
    /// The fabricated version, a file's header or the version `hello` names.
    pub found: ProtocolVersion,
    /// For a shared store, the lowest minor it records (ADR 0098).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest_minor: Option<u16>,
    /// What the rule's table gives for these versions.
    pub compatibility: Compatibility,
    /// For a protocol, the fabricated `hello` and its reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hello: Option<FabricatedHello>,
}

/// A fabricated `hello`, and the reply the engine, or the group, gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FabricatedHello {
    /// Who sends it.
    pub from: HelloFrom,
    /// The Product of the engine, or of the group, that answers it.
    pub engine: Product,
    /// The Product a Client needs, or the one a Group member runs; none for
    /// a Browser host, which needs no Product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Product>,
    /// For a Browser host, the protocol version a Session needs of it, which
    /// the engine refuses before sending once it is beyond the settled one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_needs: Option<ProtocolVersion>,
    /// The reply.
    pub reply: Reply,
}

/// The reply to a fabricated `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    /// Accepted, both sides then speaking `protocol_version`.
    Accepted {
        /// The protocol version the reply names.
        protocol_version: ProtocolVersion,
    },
    /// Refused with this Error code, before any other message, and then
    /// the Connection closes.
    Refused {
        /// The Error code.
        error_code: String,
    },
}
