//! The answer to a Client's `hello` (ADRs 0024, 0066 and 0095): the Product
//! check first, since its fix makes the version moot, then ADR 0030's
//! default rule on the protocol's `major.minor`. Product versions are
//! reported, never compared. A Browser host needs no Product, and it and
//! the engine settle on the lower of their minors, only another major
//! refused (ADR 0096). A refusal by Product or version is kept apart, so a
//! Daemon can mark the one it hands over on (ADR 0098).

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, ProtocolVersion};
use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::connection::{
    HelloParams, HelloResult, NOT_IN_PRODUCT, NotInProductDetails, PROTOCOL_VERSION_UNSUPPORTED,
    ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::formats::{self, Compatibility, Rule};
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use crate::composition::CallError;
use crate::product::{Identity, Requested};

/// A `hello` the engine accepted.
#[derive(Debug, PartialEq)]
pub(crate) struct Accepted {
    /// The reply.
    pub(crate) result: HelloResult,
    /// The surface whose rules the Connection follows: the `hello`'s own,
    /// or for a gateway's the Client's it relays (`client_surface`).
    pub(crate) surface: Surface,
    /// Whether it came from Goodfetch Cloud's gateway, which words
    /// `not_in_product` on the Connection (ADR 0096).
    pub(crate) behind_gateway: bool,
}

/// A `hello` the engine refused.
pub(crate) enum Refused {
    /// Its params were not a Client's `hello`.
    Invalid(CallError),
    /// The Client needs a Product this engine's does not contain.
    NotInProduct(String, Box<NotInProductDetails>),
    /// The Client speaks a protocol version this engine does not accept.
    Version(String, ProtocolVersionUnsupportedDetails),
}

impl Refused {
    /// Whether it refuses by Product or by version, on which a Daemon that
    /// is not its major's pick may hand over (ADR 0098).
    pub(crate) fn by_product_or_version(&self) -> bool {
        !matches!(self, Self::Invalid(_))
    }

    /// The reply's error, marked as a handover when `handover`.
    pub(crate) fn into_error(self, handover: bool) -> CallError {
        match self {
            Self::Invalid(error) => error,
            Self::NotInProduct(message, mut details) => {
                details.handover = handover;
                CallError::new(&NOT_IN_PRODUCT, message, &details)
            }
            Self::Version(message, mut details) => {
                details.handover = handover;
                CallError::new(&PROTOCOL_VERSION_UNSUPPORTED, message, &details)
            }
        }
    }
}

/// Accepts `hello`, naming the engine as `instance`, or refuses it.
pub(crate) fn answer(
    identity: &Identity,
    instance: &EngineInstance,
    hello: &HelloParams,
) -> Result<Accepted, Refused> {
    let behind_gateway = hello.surface == Surface::Cloud;
    let surface = match (hello.surface, hello.client_surface) {
        (surface, None) => surface,
        (Surface::Cloud, Some(client))
            if !matches!(client, Surface::Cloud | Surface::BrowserHost) =>
        {
            client
        }
        (Surface::Cloud, Some(_)) => {
            return Err(Refused::Invalid(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "a gateway's `client_surface` names the surface of a Client it relays, never `cloud` or `browser_host`",
            )));
        }
        (_, Some(_)) => {
            return Err(Refused::Invalid(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "only a gateway's `hello` (surface `cloud`) names `client_surface`",
            )));
        }
    };
    if hello.surface != Surface::BrowserHost {
        let Some(needs) = hello.needs else {
            return Err(Refused::Invalid(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "a Client's `hello` names the Product it needs in `needs`",
            )));
        };
        if !identity.product.contains(needs) {
            let (message, details) =
                identity.not_in_product(Requested::Hello, needs, behind_gateway);
            return Err(Refused::NotInProduct(message, Box::new(details)));
        }
    }
    let client = hello.protocol_version;
    if let Some((message, details)) = version_refusal(client, hello.surface == Surface::BrowserHost)
    {
        return Err(Refused::Version(message, details));
    }
    Ok(Accepted {
        result: HelloResult {
            protocol_version: PROTOCOL_VERSION.min(client),
            product: identity.product,
            product_version: identity.product_version.clone(),
            engine_instance: instance.clone(),
            private_endpoint: None,
            console_link: None,
        },
        surface,
        behind_gateway,
    })
}

/// The message and details of `protocol_version_unsupported` for a `hello`
/// that speaks protocol `client`, from a Browser host when `browser_host`;
/// `None` when the engine's protocol accepts it: ADR 0030's default rule for
/// a Client, the lower minor of one major for a Browser host (ADR 0096).
/// Goodfetch Cloud's gateway applies the same rule to its Clients
/// (ADR 0066).
pub fn version_refusal(
    client: ProtocolVersion,
    browser_host: bool,
) -> Option<(String, ProtocolVersionUnsupportedDetails)> {
    // During `0.y` previews each `0.y` minor counts as a major (ADR 0030),
    // so a preview engine accepts its own version alone.
    let preview = PROTOCOL_VERSION.major == 0;
    let line = if preview {
        PROTOCOL_VERSION.to_string()
    } else {
        format!("{}.x", PROTOCOL_VERSION.major)
    };
    let (rule, sender, accepts, matching) = if browser_host {
        (
            Rule::LowerMinor,
            "Browser host",
            if preview {
                line.clone()
            } else {
                format!("any {line}")
            },
            "of the engine's major",
        )
    } else {
        (
            formats::PROTOCOL.rule,
            "Client",
            if preview {
                line.clone()
            } else {
                format!("{}.0 to {PROTOCOL_VERSION}", PROTOCOL_VERSION.major)
            },
            "of the engine's own version",
        )
    };
    let fix = match rule.compatibility(PROTOCOL_VERSION, client) {
        Compatibility::Compatible => return None,
        Compatibility::NewerMinor | Compatibility::NewerMajor => format!(
            "Upgrade the engine to a version that speaks protocol {client}, or use a {sender} {matching}."
        ),
        Compatibility::OlderMajor => {
            format!("Upgrade the {sender} to one that speaks protocol {line}.")
        }
    };
    Some((
        format!(
            "This {sender} speaks protocol {client}, and this engine speaks {PROTOCOL_VERSION}: it accepts {accepts}."
        ),
        ProtocolVersionUnsupportedDetails {
            engine_protocol_version: PROTOCOL_VERSION,
            client_protocol_version: client,
            fix,
            handover: false,
        },
    ))
}
