//! The `certs.*` operations every Product carries (ADR 0042): `certs.list`
//! reads the caller's pins in the user's known-certificates store, and
//! nothing of another person's partition on a shared Daemon. `certs.accept`
//! pins the changed key that stopped a host in place of its pin, recorded
//! in the Audit log, and `certs.forget` removes a pin, recorded too: both are a person's
//! answers, which the catalog marks person only, so the RPC layer refuses
//! them to every surface but the Console's and the CLI's with
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

use crate::audit::Origin;
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::engine_context::CALLER;
use crate::known_certificates::{Found, KeyHash, KnownCertificatesError};
use crate::run::format_refusal;
use crate::tls::host_key;

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
    owner
        .blocking(move || {
            let now = engine.now();
            let accepted = engine
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
            if let Err(error) =
                engine
                    .audit()
                    .record(&origin, &CERTIFICATE_CHANGE_ACCEPTED, details, now)
            {
                tracing::warn!(%error, "the Audit log cannot record an accepted change");
            }
            Ok(accepted.pin)
        })
        .await
        .map_err(|error| refusal(error, "written"))
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
    let removed = host.clone();
    let forgotten = owner
        .blocking(move || {
            let forgotten = engine.known_certificates().forget(CALLER, &removed)?;
            if forgotten {
                let details = json!({ "host": removed });
                if let Err(error) = engine.audit().record(
                    &origin,
                    &CERTIFICATE_PIN_FORGOTTEN,
                    details,
                    engine.now(),
                ) {
                    tracing::warn!(%error, "the Audit log cannot record a forgotten pin");
                }
            }
            Ok(forgotten)
        })
        .await
        .map_err(|error| refusal(error, "written"))?;
    Ok(CertsForgotten { host, forgotten })
}

/// `host` as the store keys it: as the URL parser leaves a host.
fn host_of(host: &str) -> Result<String, CallError> {
    host_key(host).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{host}` names no host"),
        )
    })
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
