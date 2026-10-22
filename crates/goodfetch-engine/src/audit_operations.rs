//! The `audit.*` operations every Product carries (ADR 0016): `audit.verify`
//! verifies the user's Audit log, intact or the first sequence that does
//! not verify. A log of another major is refused with
//! `format_version_unsupported` (ADR 0066).

use std::sync::Arc;

use goodfetch_protocol::audit::{AUDIT_VERIFY, AuditVerified, AuditVerifyParams};
use goodfetch_protocol::formats::AUDIT_LOG;
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use crate::audit::AuditError;
use crate::composition::{CallContext, CallError, Handler};
use crate::run::format_refusal;

/// The `audit.*` handlers.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&AUDIT_VERIFY, verify)]
}

async fn verify(_: AuditVerifyParams, context: CallContext) -> Result<AuditVerified, CallError> {
    let runs = context.runs();
    let log = runs.audit().clone();
    let owner = Arc::clone(&runs.limits().owner);
    owner
        .blocking(move || log.verify())
        .await
        .map_err(|error| match error {
            AuditError::Format {
                found,
                compatibility,
            } => format_refusal(&AUDIT_LOG, &found, compatibility),
            other => CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the Audit log cannot be read: {other}"),
            ),
        })
}
