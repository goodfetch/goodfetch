//! Block causes (ADRs 0035, 0045 and 0048): the closed list of reasons a
//! site refused a page, a type both Products share (ADR 0095). Collie's
//! values are listed here; `decoy`, which only Foxhound's Frontier
//! produces, is listed by Foxhound's own module.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Why a site refused a page (ADR 0035), on the wire as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true))]
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
}

impl BlockCause {
    /// Its name on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Challenge => "challenge",
            Self::Captcha => "captcha",
            Self::RateLimit => "rate_limit",
            Self::IpBlock => "ip_block",
            Self::Decoy => "decoy",
            Self::PaymentRequired => "payment_required",
            Self::ConsentRequired => "consent_required",
        }
    }

    /// What it means, the catalog's description of the value.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Challenge => "A page that checks the browser before it lets it through.",
            Self::Captcha => "A page that asks a person to prove they are human.",
            Self::RateLimit => "The site refused because too many requests came too fast.",
            Self::IpBlock => "The site refused the address the request came from.",
            Self::Decoy => {
                "A trap page a person would never reach; only Foxhound's Frontier produces it."
            }
            Self::PaymentRequired => "The site asked to be paid for the page (ADR 0045).",
            Self::ConsentRequired => {
                "A wall that asks for consent before it shows the page (ADR 0048)."
            }
        }
    }

    /// Its value of the shared type `block_cause`, for a module's entries.
    pub const fn shared_type_value(self) -> SharedTypeValue {
        SharedTypeValue {
            shared_type: SharedType::BlockCause,
            name: self.as_str(),
            description: self.description(),
            arrived_in: ARRIVED_IN,
        }
    }
}

/// Collie's Block causes: every one but `decoy`.
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
    shared_type_values: &[
        BlockCause::Challenge.shared_type_value(),
        BlockCause::Captcha.shared_type_value(),
        BlockCause::RateLimit.shared_type_value(),
        BlockCause::IpBlock.shared_type_value(),
        BlockCause::PaymentRequired.shared_type_value(),
        BlockCause::ConsentRequired.shared_type_value(),
    ],
};

#[cfg(test)]
mod tests;
