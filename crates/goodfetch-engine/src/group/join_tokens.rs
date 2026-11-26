//! The join tokens of a Daemon group (ADR 0055): `group token` mints one on
//! the Lead Daemon, and a Daemon that presents it with `group join` gets its
//! certificate signed. A token works once and for
//! [`JOIN_TOKEN_LIFETIME`]; the Lead Daemon keeps the tokens in memory only,
//! so a restart forgets the ones not yet redeemed.

use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret, SecretString};

use crate::endpoint::{new_token, same_token};

/// How long a join token is valid: 30 minutes **(shape)**, the default
/// lifetime of a Teleport join token that `tctl tokens add` mints.
pub const JOIN_TOKEN_LIFETIME: Duration = Duration::from_mins(30);

/// A join token just minted, which the operator hands to the Daemon that
/// joins.
#[derive(Debug)]
pub struct JoinToken {
    /// The token: 32 bytes of the OS generator, in lowercase hex.
    pub token: SecretString,
    /// When it stops being valid.
    pub expires: SystemTime,
}

/// Why a join token was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JoinTokenRefused {
    /// No pending token is this one: never minted, already redeemed,
    /// expired a lifetime ago or more and forgotten, or minted before the
    /// Lead Daemon restarted.
    #[error("the join token is unknown or already used")]
    Unknown,
    /// The token's lifetime ended before it was presented.
    #[error("the join token has expired")]
    Expired,
}

/// The join tokens minted and not yet redeemed.
#[derive(Debug, Default)]
pub struct JoinTokens {
    pending: Vec<JoinToken>,
}

impl JoinTokens {
    /// A new token, minted at `now` and valid for [`JOIN_TOKEN_LIFETIME`].
    /// Tokens that expired a lifetime ago or more are forgotten, so the
    /// pending ones stay few.
    ///
    /// # Errors
    ///
    /// When the OS generator gives nothing.
    pub fn mint(&mut self, now: SystemTime) -> Result<JoinToken, getrandom::Error> {
        self.pending
            .retain(|pending| pending.expires + JOIN_TOKEN_LIFETIME > now);
        let token = new_token()?;
        let expires = now + JOIN_TOKEN_LIFETIME;
        self.pending.push(JoinToken {
            token: SecretString::from(token.clone()),
            expires,
        });
        Ok(JoinToken {
            token: SecretString::from(token),
            expires,
        })
    }

    /// Redeems `given` at `now`: a pending token, compared in constant time,
    /// is used up whether or not it is still valid.
    ///
    /// # Errors
    ///
    /// [`JoinTokenRefused::Unknown`] when no pending token is `given`, and
    /// [`JoinTokenRefused::Expired`] when it is past its lifetime.
    pub fn redeem(&mut self, given: &str, now: SystemTime) -> Result<(), JoinTokenRefused> {
        let found = self
            .pending
            .iter()
            .position(|pending| same_token(given, pending.token.expose_secret()))
            .ok_or(JoinTokenRefused::Unknown)?;
        let redeemed = self.pending.swap_remove(found);
        if now < redeemed.expires {
            Ok(())
        } else {
            Err(JoinTokenRefused::Expired)
        }
    }
}

#[cfg(test)]
mod tests;
