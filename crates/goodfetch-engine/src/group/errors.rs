//! Shared `CallError` builders for Daemon group operations and peer handlers
//! (ADR 0055).

use goodfetch_protocol::group::{
    GROUP_MEMBER_UNKNOWN, LEAD_CANNOT_LEAVE, LEAD_UNCONFIRMED, LeadEpoch, NOT_LEAD_DAEMON,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use super::authority::GroupMemberName;
use super::store::{self, Revocation};
use crate::CallError;
use crate::user_store::UserStoreError;

/// The revocation list number after revoking `name`, or the error it refuses
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
pub(super) fn lead_cannot_leave() -> CallError {
    CallError::new(
        &LEAD_CANNOT_LEAVE,
        "The Lead Daemon cannot leave its Daemon group, nor be revoked from it: promote another Group member to Lead Daemon first.",
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

/// The refusal of a Lead Daemon that has not yet confirmed its lead epoch.
pub(super) fn lead_unconfirmed(
    lead_epoch: LeadEpoch,
    confirmed_by: usize,
    total_members: usize,
) -> CallError {
    CallError::new(
        &LEAD_UNCONFIRMED,
        format!(
            "The Lead Daemon has not yet confirmed lead epoch {} ({confirmed_by} of {total_members} last-known Group members reached): wait for a majority, or run `group lead --confirm` once no other Lead Daemon runs.",
            lead_epoch.get()
        ),
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
