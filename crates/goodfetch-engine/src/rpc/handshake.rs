//! The answer to a Client's `hello` (ADRs 0024, 0066 and 0095): the Product
//! check first, since its fix makes the version moot, then ADR 0030's
//! default rule on the protocol's `major.minor`. Product versions are
//! reported, never compared. A Browser host needs no Product.

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::connection::{
    HelloParams, HelloResult, PROTOCOL_VERSION_UNSUPPORTED, ProtocolVersionUnsupportedDetails,
    Surface,
};
use goodfetch_protocol::formats::{self, Compatibility};
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use crate::composition::CallError;
use crate::product::{Identity, Requested};

/// A `hello` the engine accepted.
#[derive(Debug, PartialEq)]
pub(crate) struct Accepted {
    /// The reply.
    pub(crate) result: HelloResult,
    /// Whether it came from Goodfetch Cloud's gateway, which words
    /// `not_in_product` on the Connection (ADR 0096).
    pub(crate) behind_gateway: bool,
}

/// Accepts `hello`, or refuses it with the Error code its reply carries.
pub(crate) fn answer(identity: &Identity, hello: &HelloParams) -> Result<Accepted, CallError> {
    let behind_gateway = hello.surface == Surface::Cloud;
    if hello.surface != Surface::BrowserHost {
        let Some(needs) = hello.needs else {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "a Client's `hello` names the Product it needs in `needs`",
            ));
        };
        if !identity.product.contains(needs) {
            return Err(super::not_in_product(
                identity,
                Requested::Hello,
                needs,
                behind_gateway,
            ));
        }
    }
    let client = hello.protocol_version;
    let fix = match formats::PROTOCOL
        .rule
        .compatibility(PROTOCOL_VERSION, client)
    {
        Compatibility::Compatible => {
            return Ok(Accepted {
                result: HelloResult {
                    protocol_version: PROTOCOL_VERSION,
                    product: identity.product,
                    product_version: identity.product_version.clone(),
                },
                behind_gateway,
            });
        }
        Compatibility::NewerMinor | Compatibility::NewerMajor => format!(
            "Upgrade the engine to a version that speaks protocol {client}, or use a Client of the engine's own version."
        ),
        Compatibility::OlderMajor => format!(
            "Upgrade the Client to one that speaks protocol {}.x.",
            PROTOCOL_VERSION.major
        ),
    };
    Err(CallError::new(
        &PROTOCOL_VERSION_UNSUPPORTED,
        format!(
            "This Client speaks protocol {client}, and this engine speaks {PROTOCOL_VERSION}: it accepts {}.0 to {PROTOCOL_VERSION}.",
            PROTOCOL_VERSION.major
        ),
        &ProtocolVersionUnsupportedDetails {
            engine_protocol_version: PROTOCOL_VERSION,
            client_protocol_version: client,
            fix,
        },
    ))
}
