//! The person-only entries of the Console's sign-ins (spec #592; ADR
//! 0029): `sign_in.list` lists the sign-ins that have not ended,
//! `sign_in.revoke` ends one, and `console.rotate_key` replaces the
//! Console key, which ends them all. The catalog keeps them to the Console
//! and an interactive CLI prompt. They reach the sign-in driver of the
//! Daemon that serves the Console; an engine that serves none, such as an
//! Attached engine, holds no sign-in to list or end.

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::people::{
    CONSOLE_ROTATE_KEY, ConsoleKeyRotated, ConsoleRotateKeyParams, SIGN_IN_LIST, SIGN_IN_REVOKE,
    SignInList, SignInListParams, SignInRevokeParams, SignInRevoked,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::people::{PeopleError, SignInDriver};

/// The `sign_in.*` and `console.rotate_key` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&SIGN_IN_LIST, list),
        Handler::new(&SIGN_IN_REVOKE, revoke),
        Handler::new(&CONSOLE_ROTATE_KEY, rotate_key),
    ]
}

async fn list(_: SignInListParams, context: CallContext) -> Result<SignInList, CallError> {
    let sign_ins = driver(&context)?
        .list()
        .await
        .map_err(|error| failed(&error))?;
    Ok(SignInList { sign_ins })
}

async fn revoke(
    params: SignInRevokeParams,
    context: CallContext,
) -> Result<SignInRevoked, CallError> {
    let revoked = driver(&context)?
        .revoke(&params.id, Some(context.surface()))
        .await
        .map_err(|error| failed(&error))?;
    Ok(SignInRevoked { revoked })
}

async fn rotate_key(
    _: ConsoleRotateKeyParams,
    context: CallContext,
) -> Result<ConsoleKeyRotated, CallError> {
    let signed_out = driver(&context)?
        .rotate_key(Some(context.surface()))
        .await
        .map_err(|error| failed(&error))?;
    Ok(ConsoleKeyRotated { signed_out })
}

/// The sign-in driver of the Daemon the call reached.
fn driver(context: &CallContext) -> Result<SignInDriver, CallError> {
    context
        .daemon()
        .and_then(|daemon| daemon.sign_ins())
        .ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "this engine serves no Console, so it holds no sign-in: reach the Daemon that serves it",
            )
        })
}

fn failed(error: &PeopleError) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the Console's sign-ins cannot be read or changed: {error}"),
    )
}
