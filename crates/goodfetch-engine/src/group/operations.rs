//! The `group.*` handlers an operator calls (ADR 0055): `group.init` forms
//! a Daemon group with this Daemon as its Lead Daemon, and `group.token`
//! mints a join token on it. The group store is the Daemon's: an Attached
//! engine that can start its major's Daemon forwards each call to it, and
//! only a Daemon, or an engine that cannot start one, runs these handlers.

use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::group::{
    GROUP_EXISTS, GROUP_INIT, GROUP_TOKEN, GroupFormed, GroupInitParams, GroupTokenParams,
    JoinTokenMinted, NOT_LEAD_DAEMON,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use secrecy::ExposeSecret;

use super::{GroupAuthority, JoinToken, store};
use crate::composition::{CallContext, Contribution, Handler};
use crate::user_store::{UserStore, UserStoreError};
use crate::{CallError, kept_by_the_daemon};

/// What Daemon groups add to an engine: the group store and the `group.*`
/// handlers an operator calls, each call sent to the major's Daemon when
/// the engine is an Attached engine that reaches it.
pub fn contribution() -> Contribution {
    Contribution::new("group")
        .with_user_store(&store::STORE)
        .with_handler(Handler::new(&GROUP_INIT, init))
        .with_handler(Handler::new(&GROUP_TOKEN, token))
        .guarded(&kept_by_the_daemon())
}

async fn init(_: GroupInitParams, context: CallContext) -> Result<GroupFormed, CallError> {
    let authority = GroupAuthority::create(context.engine().clock().wall())
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?;
    let authority_key = authority.key_hash().to_string();
    let formed = store::form(&store_of(&context), authority.kept())
        .await
        .map_err(|error| store_failed(&error))?;
    let Some(lead_epoch) = formed else {
        return Err(CallError::new(
            &GROUP_EXISTS,
            "This Daemon belongs to a Daemon group already; mint a join token with `group token` on its Lead Daemon.",
            &(),
        ));
    };
    Ok(GroupFormed {
        lead_epoch,
        authority_key,
    })
}

async fn token(_: GroupTokenParams, context: CallContext) -> Result<JoinTokenMinted, CallError> {
    let store = store_of(&context);
    let not_lead = || {
        CallError::new(
            &NOT_LEAD_DAEMON,
            "This Daemon is not a Daemon group's Lead Daemon; run `group token` on the Lead Daemon, or `group init` to form a group here.",
            &(),
        )
    };
    let kept = store::authority(&store)
        .await
        .map_err(|error| store_failed(&error))?
        .ok_or_else(not_lead)?;
    let authority_key = GroupAuthority::restore(kept)
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?
        .key_hash()
        .to_string();
    let now = context.engine().clock().wall();
    let minted = JoinToken::mint(now)
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?;
    if !store::record_token(&store, &minted, now)
        .await
        .map_err(|error| store_failed(&error))?
    {
        return Err(not_lead());
    }
    Ok(JoinTokenMinted {
        token: DaemonToken::new(minted.token.expose_secret().to_owned()),
        expires_at: minted.expires,
        authority_key,
    })
}

/// The engine's group store, which [`contribution`] declares.
fn store_of(context: &CallContext) -> UserStore {
    context
        .engine()
        .user_store(&store::STORE)
        .expect(
            "invariant: the contribution that registers these handlers declares the group store",
        )
        .clone()
}

/// The error a call answers when the store fails it.
fn store_failed(error: &UserStoreError) -> CallError {
    match error {
        UserStoreError::Format {
            found,
            compatibility,
            ..
        } => crate::run::format_refusal(&store::GROUP_STORE, found, *compatibility),
        _ => CallError::protocol(ProtocolFailure::InternalError, error.to_string()),
    }
}
