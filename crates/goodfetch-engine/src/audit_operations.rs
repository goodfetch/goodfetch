//! The `audit.*` operations every Product carries (ADR 0016): `audit.export`
//! answers every entry of the user's Audit log with its hash, and
//! `audit.verify` verifies it, intact or the first sequence that does not
//! verify. A log of another major is refused with
//! `format_version_unsupported` (ADR 0066).

use std::sync::Arc;

use goodfetch_protocol::audit::{
    AUDIT_EXPORT, AUDIT_VERIFY, AuditExportParams, AuditExported, AuditVerified, AuditVerifyParams,
};
use goodfetch_protocol::formats::AUDIT_LOG;
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use crate::audit::{AuditError, AuditLog};
use crate::composition::{CallContext, CallError, Handler};
use crate::run::format_refusal;

/// The `audit.*` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&AUDIT_EXPORT, export),
        Handler::new(&AUDIT_VERIFY, verify),
    ]
}

async fn export(_: AuditExportParams, context: CallContext) -> Result<AuditExported, CallError> {
    let entries = on_the_log(&context, AuditLog::export).await?;
    Ok(AuditExported { entries })
}

async fn verify(_: AuditVerifyParams, context: CallContext) -> Result<AuditVerified, CallError> {
    on_the_log(&context, AuditLog::verify).await
}

/// What `read` answers of the user's Audit log, run under the owner's
/// blocking policy.
async fn on_the_log<T: Send + 'static>(
    context: &CallContext,
    read: fn(&AuditLog) -> Result<T, AuditError>,
) -> Result<T, CallError> {
    let runs = context.runs();
    let log = runs.audit().clone();
    let owner = Arc::clone(&runs.limits().owner);
    owner
        .blocking(move || read(&log))
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
