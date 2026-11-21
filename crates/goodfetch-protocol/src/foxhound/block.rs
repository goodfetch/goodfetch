//! The Block cause only Foxhound carries (ADR 0095): `decoy`, which its
//! Frontier gives a page reached through a trap. Foxhound's [`BlockCause`]
//! is Collie's with `decoy` added, so Foxhound's crates and programs know
//! it while Collie's keep it as an unknown value (ADR 0079), as each
//! Product's TypeScript and Python packages do.

use schemars::JsonSchema;
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

use crate::block::{BlockCause as CollieBlockCause, UNKNOWN_CAUSE};
use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};
use crate::open_value::UnknownValue;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Why a site refused a page (ADR 0035), on the wire as written.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum BlockCause {
    /// A page that checks the browser before it lets it through.
    Challenge,
    /// A page that asks a person to prove they are human.
    Captcha,
    /// The site refused because too many requests came too fast.
    RateLimit,
    /// The site refused the address the request came from.
    IpBlock,
    /// A trap page a person would never reach, such as a labyrinth of
    /// hidden links; only Foxhound's Frontier produces it (ADR 0095).
    Decoy,
    /// The site asked to be paid for the page (ADR 0045).
    PaymentRequired,
    /// A wall that asks for consent before it shows the page (ADR 0048).
    ConsentRequired,
    /// A cause a newer engine gives that this reader does not know, kept as
    /// received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl BlockCause {
    /// Its name on the wire.
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Challenge => "challenge",
            Self::Captcha => "captcha",
            Self::RateLimit => "rate_limit",
            Self::IpBlock => "ip_block",
            Self::Decoy => "decoy",
            Self::PaymentRequired => "payment_required",
            Self::ConsentRequired => "consent_required",
            Self::Unknown(cause) => cause.as_str(),
        }
    }

    /// What it means, the catalog's description of the value.
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Challenge => CollieBlockCause::Challenge.description(),
            Self::Captcha => CollieBlockCause::Captcha.description(),
            Self::RateLimit => CollieBlockCause::RateLimit.description(),
            Self::IpBlock => CollieBlockCause::IpBlock.description(),
            Self::Decoy => {
                "A trap page a person would never reach; only Foxhound's Frontier produces it."
            }
            Self::PaymentRequired => CollieBlockCause::PaymentRequired.description(),
            Self::ConsentRequired => CollieBlockCause::ConsentRequired.description(),
            Self::Unknown(_) => UNKNOWN_CAUSE,
        }
    }

    /// Its value of the shared type `block_cause`, for a module's entries.
    pub const fn shared_type_value(&'static self) -> SharedTypeValue {
        SharedTypeValue {
            shared_type: SharedType::BlockCause,
            name: self.as_str(),
            description: self.description(),
            arrived_in: ARRIVED_IN,
        }
    }
}

/// A cause as Foxhound knows it, from one Collie's type read: a value
/// Collie kept as unknown is read again as Foxhound's, so one only Foxhound
/// carries, as `decoy`, is known here.
impl From<CollieBlockCause> for BlockCause {
    fn from(cause: CollieBlockCause) -> Self {
        match cause {
            CollieBlockCause::Challenge => Self::Challenge,
            CollieBlockCause::Captcha => Self::Captcha,
            CollieBlockCause::RateLimit => Self::RateLimit,
            CollieBlockCause::IpBlock => Self::IpBlock,
            CollieBlockCause::PaymentRequired => Self::PaymentRequired,
            CollieBlockCause::ConsentRequired => Self::ConsentRequired,
            CollieBlockCause::Unknown(cause) => {
                let read: Result<Self, serde::de::value::Error> =
                    Self::deserialize(cause.as_str().into_deserializer());
                read.unwrap_or(Self::Unknown(cause))
            }
        }
    }
}

/// What `blocked` details, as a Foxhound program reads them: its cause is
/// Foxhound's, so `decoy` is a known value.
pub type BlockedDetails = crate::page::BlockedDetails<BlockCause>;

/// Foxhound's Block cause, `decoy`.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[BlockCause::Decoy.shared_type_value()],
};

#[cfg(test)]
mod tests;
