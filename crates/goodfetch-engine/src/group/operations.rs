//! The `group.*` handlers an operator calls (ADR 0055): `group.init` forms
//! a Daemon group with this Daemon as its Lead Daemon, `group.token` mints
//! a join token on it, `group.join` makes this Daemon a Group member of
//! the group whose Lead Daemon it dials, `group.leave` takes it out, and
//! `group.revoke` removes another Group member, on the Lead Daemon.
//! Forming and joining open the peer listener, and leaving closes it. The
//! group store is the Daemon's: an Attached engine that can start its
//! major's Daemon forwards each call to it, and only a Daemon, or an
//! engine that cannot start one, runs these handlers.

use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::group::{
    GROUP_EXISTS, GROUP_INIT, GROUP_JOIN, GROUP_LEAVE, GROUP_MEMBER_UNKNOWN, GROUP_REVOKE,
    GROUP_TOKEN, GroupFormed, GroupInitParams, GroupJoinParams, GroupJoined, GroupLeaveParams,
    GroupLeft, GroupMemberRevoked, GroupRevokeParams, GroupTokenParams, JoinTokenMinted,
    LEAD_CANNOT_LEAVE, NOT_IN_A_GROUP, NOT_LEAD_DAEMON,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use secrecy::ExposeSecret;

use super::identity::{KeptIdentity, renew_at};
use super::peering::{self, Asking};
use super::store::Revocation;
use super::{GroupAuthority, GroupMemberKey, GroupMemberName, JoinToken, store};
use crate::composition::{CallContext, Contribution, Handler};
use crate::known_certificates::KeyHash;
use crate::transport::{PeerListener, listen_peer};
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
        .with_handler(Handler::new(&GROUP_JOIN, join))
        .with_handler(Handler::new(&GROUP_LEAVE, leave))
        .with_handler(Handler::new(&GROUP_REVOKE, revoke))
        .guarded(&kept_by_the_daemon())
}

async fn init(params: GroupInitParams, context: CallContext) -> Result<GroupFormed, CallError> {
    let store = store_of(&context);
    if store::is_in_a_group(&store)
        .await
        .map_err(|error| store_failed(&error))?
    {
        return Err(group_exists());
    }
    let name = member_name(&params.name)?;
    let (listener, peer_address) = listen(&params.peer_address).await?;
    let now = context.engine().clock().wall();
    let mut authority = GroupAuthority::create(now).map_err(|error| internal(&error))?;
    let authority_key = authority.key_hash().to_string();
    let lead = KeptIdentity::lead(&mut authority, &name, peer_address.clone(), now)
        .map_err(|error| internal(&error))?;
    let Some(lead_epoch) = store::form(&store, authority.kept(), lead.clone(), now)
        .await
        .map_err(|error| store_failed(&error))?
    else {
        return Err(group_exists());
    };
    peering::start(
        context.engine(),
        context.identity().clone(),
        store,
        listener,
        &lead,
        lead_epoch,
    )
    .map_err(|error| internal(&error))?;
    Ok(GroupFormed {
        lead_epoch,
        peer_address,
        authority_key,
    })
}

async fn token(_: GroupTokenParams, context: CallContext) -> Result<JoinTokenMinted, CallError> {
    let store = store_of(&context);
    let kept = store::authority(&store)
        .await
        .map_err(|error| store_failed(&error))?
        .ok_or_else(not_lead)?;
    let authority_key = GroupAuthority::restore(kept)
        .map_err(|error| internal(&error))?
        .key_hash()
        .to_string();
    let now = context.engine().clock().wall();
    let minted = JoinToken::mint(now).map_err(|error| internal(&error))?;
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

async fn join(params: GroupJoinParams, context: CallContext) -> Result<GroupJoined, CallError> {
    let store = store_of(&context);
    if store::is_in_a_group(&store)
        .await
        .map_err(|error| store_failed(&error))?
    {
        return Err(group_exists());
    }
    let name = member_name(&params.name)?;
    let pin = KeyHash::from_text(&params.authority_key).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{}` is no authority key: give the `sha256/…` that `group token` answered on the Lead Daemon.",
                params.authority_key
            ),
        )
    })?;
    let (listener, peer_address) = listen(&params.peer_address).await?;
    let key = GroupMemberKey::generate().map_err(|error| internal(&error))?;
    let clock = context.engine().clock();
    let admitted = peering::ask_to_join(
        &Asking {
            lead: &params.lead,
            pin,
            token: &params.token,
            name: &name,
            public_key: &key.public_key(),
            peer_address: &peer_address,
        },
        context.identity(),
        clock,
    )
    .await?;
    let received_at = clock.wall();
    let member = KeptIdentity {
        name: name.as_str().to_owned(),
        key: key.kept(),
        certificate: admitted.certificate,
        authority: admitted.authority,
        revocations: admitted.revocations,
        peer_address: peer_address.clone(),
        lead_address: params.lead,
        lead_name: admitted.lead_name,
        certificate_expires_at: admitted.certificate_expires_at,
        renew_at: renew_at(received_at, admitted.certificate_expires_at),
    };
    if !store::joined(&store, member.clone(), admitted.lead_epoch)
        .await
        .map_err(|error| store_failed(&error))?
    {
        return Err(group_exists());
    }
    peering::start(
        context.engine(),
        context.identity().clone(),
        store,
        listener,
        &member,
        admitted.lead_epoch,
    )
    .map_err(|error| internal(&error))?;
    Ok(GroupJoined {
        lead_epoch: admitted.lead_epoch,
        peer_address,
        certificate_expires_at: admitted.certificate_expires_at,
    })
}

async fn leave(_: GroupLeaveParams, context: CallContext) -> Result<GroupLeft, CallError> {
    let store = store_of(&context);
    let (kept, lead_epoch) = store::identity(&store)
        .await
        .map_err(|error| store_failed(&error))?
        .ok_or_else(not_in_a_group)?;
    if kept.is_lead() {
        return Err(lead_cannot_leave());
    }
    peering::ask_to_leave(
        &kept,
        lead_epoch,
        context.identity(),
        context.engine().clock(),
    )
    .await?;
    store::leave(&store)
        .await
        .map_err(|error| store_failed(&error))?;
    peering::stop(context.engine()).await;
    Ok(GroupLeft { name: kept.name })
}

async fn revoke(
    params: GroupRevokeParams,
    context: CallContext,
) -> Result<GroupMemberRevoked, CallError> {
    let store = store_of(&context);
    let name = member_name(&params.name)?;
    let revocation = store::revoke(&store, params.name, context.engine().clock().wall())
        .await
        .map_err(|error| store_failed(&error))?;
    Ok(GroupMemberRevoked {
        revocation_list: revoked(revocation, &name)?,
        name: name.as_str().to_owned(),
    })
}

/// The number of the revocation list that removed the Group member `name`,
/// as the store answers `revocation`.
///
/// # Errors
///
/// `not_lead_daemon` off the Lead Daemon, `lead_cannot_leave` for the Lead
/// Daemon's own name and `group_member_unknown` for a name the group has
/// no Group member of.
pub(super) fn revoked(revocation: Revocation, name: &GroupMemberName) -> Result<u64, CallError> {
    match revocation {
        Revocation::Revoked(number) => Ok(number),
        Revocation::NotLead => Err(not_lead()),
        Revocation::Lead => Err(lead_cannot_leave()),
        Revocation::Unknown => Err(CallError::new(
            &GROUP_MEMBER_UNKNOWN,
            format!(
                "The Daemon group has no Group member named `{}`: it left, was revoked, or never joined.",
                name.as_str()
            ),
            &(),
        )),
    }
}

/// The refusal of the Lead Daemon's leaving or revocation.
fn lead_cannot_leave() -> CallError {
    CallError::new(
        &LEAD_CANNOT_LEAVE,
        "The Lead Daemon cannot leave its Daemon group, nor be revoked from it: promote another Group member to Lead Daemon first.",
        &(),
    )
}

/// The refusal of a Daemon in no group.
fn not_in_a_group() -> CallError {
    CallError::new(
        &NOT_IN_A_GROUP,
        "This Daemon belongs to no Daemon group; form one with `group init`, or join one with `group join`.",
        &(),
    )
}

/// Opens the peer listener at `address`, the `host:port` an operator gave:
/// the listener and the `host:port` it took, port 0 replaced by the one
/// the system chose.
async fn listen(address: &str) -> Result<(PeerListener, String), CallError> {
    let cannot = |error: &dyn std::fmt::Display| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("This Daemon cannot listen for peer links at `{address}`: {error}."),
        )
    };
    let listener = listen_peer(address).await.map_err(|error| cannot(&error))?;
    let port = listener
        .local_address()
        .map_err(|error| cannot(&error))?
        .port();
    let host = address.rsplit_once(':').map_or(address, |(host, _)| host);
    Ok((listener, format!("{host}:{port}")))
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

/// `name` as a Group member name, or the refusal of the call that gave it.
fn member_name(name: &str) -> Result<GroupMemberName, CallError> {
    GroupMemberName::new(name)
        .map_err(|error| CallError::protocol(ProtocolFailure::InvalidParams, error.to_string()))
}

/// The refusal of a Daemon that is in a group already.
fn group_exists() -> CallError {
    CallError::new(
        &GROUP_EXISTS,
        "This Daemon belongs to a Daemon group already; mint a join token with `group token` on its Lead Daemon.",
        &(),
    )
}

/// The refusal of a Daemon that keeps no group's authority.
pub(super) fn not_lead() -> CallError {
    CallError::new(
        &NOT_LEAD_DAEMON,
        "This Daemon is not a Daemon group's Lead Daemon; run `group token` on the Lead Daemon, or `group init` to form a group here.",
        &(),
    )
}

/// The failure of a call this Daemon could not finish: `error` is its
/// own, not the caller's.
pub(super) fn internal(error: &dyn std::fmt::Display) -> CallError {
    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
}

/// The error a call answers when the store fails it.
pub(super) fn store_failed(error: &UserStoreError) -> CallError {
    match error {
        UserStoreError::Format {
            found,
            compatibility,
            ..
        } => crate::run::format_refusal(&store::GROUP_STORE, found, *compatibility),
        _ => CallError::protocol(ProtocolFailure::InternalError, error.to_string()),
    }
}
