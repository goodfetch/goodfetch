//! The `certs.*` operations every Product carries (ADR 0042): `certs.list`
//! reads the caller's pins in the user's known-certificates store, and
//! nothing of another person's partition on a shared Daemon. A store of
//! another major is refused with `format_version_unsupported` (ADR 0066).

use std::sync::Arc;

use goodfetch_protocol::formats::KNOWN_CERTIFICATES;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::tls::{CERTS_LIST, CertsListParams, CertsListed};

use crate::composition::{CallContext, CallError, Handler};
use crate::known_certificates::KnownCertificatesError;
use crate::run::format_refusal;
use crate::runs::CALLER;

/// The `certs.*` handlers.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&CERTS_LIST, list)]
}

async fn list(_: CertsListParams, context: CallContext) -> Result<CertsListed, CallError> {
    let limits = context.runs().limits();
    let store = limits.known_certificates.clone();
    let owner = Arc::clone(&limits.owner);
    let pins = owner
        .blocking(move || store.list(CALLER))
        .await
        .map_err(|error| match error {
            KnownCertificatesError::Format {
                found,
                compatibility,
            } => format_refusal(&KNOWN_CERTIFICATES, &found, compatibility),
            other => CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the known-certificates store cannot be read: {other}"),
            ),
        })?;
    Ok(CertsListed { pins })
}
