//! The `certs.*` operations every Product carries (ADR 0042): `certs.list`
//! reads the caller's pins in the user's known-certificates store, and
//! nothing of another person's partition on a shared Daemon. `certs.accept`
//! pins the changed key that stopped a host in place of its pin, recorded
//! in the Audit log, and `certs.forget` removes a pin, recorded too: both are a person's
//! answers, which the catalog marks person only, so the RPC layer refuses
//! them to every surface but the Kennel's and the CLI's with
//! `person_required`. A store of another major is refused with
//! `format_version_unsupported` (ADR 0066).

use std::sync::Arc;

use goodfetch_protocol::audit::{Actor, CERTIFICATE_CHANGE_ACCEPTED, CERTIFICATE_PIN_FORGOTTEN};
use goodfetch_protocol::formats::KNOWN_CERTIFICATES;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::tls::{
    CERTS_ACCEPT, CERTS_FORGET, CERTS_LIST, CertificatePin, CertsAcceptParams, CertsForgetParams,
    CertsForgotten, CertsListParams, CertsListed,
};
use serde_json::json;

use crate::audit::{AuditError, Origin};
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::engine_context::CALLER;
use crate::host_name;
use crate::known_certificates::{Found, KeyHash, KnownCertificatesError};
use crate::run::format_refusal;

/// The `certs.*` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&CERTS_LIST, list),
        Handler::new(&CERTS_ACCEPT, accept),
        Handler::new(&CERTS_FORGET, forget),
    ]
}

async fn list(_: CertsListParams, context: CallContext) -> Result<CertsListed, CallError> {
    let engine = context.engine();
    let store = engine.known_certificates().clone();
    let owner = Arc::clone(engine.owner());
    let pins = owner
        .blocking(move || store.list(CALLER))
        .await
        .map_err(|error| refusal(error, "read"))?;
    Ok(CertsListed { pins })
}

async fn accept(
    params: CertsAcceptParams,
    context: CallContext,
) -> Result<CertificatePin, CallError> {
    let host = host_of(&params.host)?;
    let key = KeyHash::from_text(&params.spki_sha256).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{}` is no key hash: `sha256/` and the standard base64 of 32 bytes",
                params.spki_sha256
            ),
        )
    })?;
    let engine = context.engine().clone();
    let origin = Origin {
        actor: Actor::Person {
            name: context.person(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id: None,
    };
    let owner = Arc::clone(engine.owner());
    let store = engine.clone();
    let now = engine.now();
    let accepted = owner
        .blocking(move || {
            let accepted = store
                .known_certificates()
                .accept(CALLER, &host, &key, now)?;
            let pinned = match accepted.previous {
                Found::Pinned(pinned) => json!(pinned.to_string()),
                Found::Unpinned | Found::Unreadable => json!(null),
            };
            let details = json!({
                "host": host,
                "pinned_spki_sha256": pinned,
                "accepted_spki_sha256": key.to_string(),
            });
            Ok((accepted.pin, details))
        })
        .await
        .map_err(|failed: Failed| failed.call_error("written"))?;
    let (pin, details) = accepted;
    engine
        .audit()
        .record(&origin, &CERTIFICATE_CHANGE_ACCEPTED, details, now)
        .await
        .map_err(|error| Failed::Unrecorded(error).call_error("written"))?;
    Ok(pin)
}

async fn forget(
    params: CertsForgetParams,
    context: CallContext,
) -> Result<CertsForgotten, CallError> {
    let host = host_of(&params.host)?;
    let engine = context.engine().clone();
    let origin = Origin {
        actor: Actor::Person {
            name: context.person(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id: None,
    };
    let owner = Arc::clone(engine.owner());
    let (store, removed) = (engine.clone(), host.clone());
    let forgotten = owner
        .blocking(move || Ok(store.known_certificates().forget(CALLER, &removed)?))
        .await
        .map_err(|failed: Failed| failed.call_error("written"))?;
    if forgotten {
        let details = json!({ "host": host });
        engine
            .audit()
            .record(&origin, &CERTIFICATE_PIN_FORGOTTEN, details, engine.now())
            .await
            .map_err(|error| Failed::Unrecorded(error).call_error("written"))?;
    }
    Ok(CertsForgotten { host, forgotten })
}

/// `host` as the store keys it: as the URL parser leaves a host.
fn host_of(host: &str) -> Result<String, CallError> {
    host_name::key(host).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{host}` names no host"),
        )
    })
}

/// Why a person's answer failed.
#[derive(Debug, thiserror::Error)]
enum Failed {
    /// The store refused it, or could not be read or written.
    #[error(transparent)]
    Store(#[from] KnownCertificatesError),
    /// It was made, but the Audit log could not record it.
    #[error("the answer was made, but the Audit log cannot record it: {0}")]
    Unrecorded(#[source] AuditError),
}

impl Failed {
    /// The failure a call answers when the store could not be `done`, or
    /// its answer was not recorded.
    fn call_error(self, done: &str) -> CallError {
        match self {
            Self::Store(error) => refusal(error, done),
            unrecorded @ Self::Unrecorded(_) => {
                CallError::protocol(ProtocolFailure::InternalError, unrecorded.to_string())
            }
        }
    }
}

/// The failure a call answers when the store could not be `done`.
fn refusal(error: KnownCertificatesError, done: &str) -> CallError {
    match error {
        KnownCertificatesError::Format {
            found,
            compatibility,
        } => format_refusal(&KNOWN_CERTIFICATES, &found, compatibility),
        KnownCertificatesError::NoSuchChange { host } => CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "no changed key of {host} waits to be accepted with that hash: `certs list` shows the key that stopped it"
            ),
        ),
        other => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the known-certificates store cannot be {done}: {other}"),
        ),
    }
}
