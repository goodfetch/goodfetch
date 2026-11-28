//! The join tokens of a Daemon group (ADR 0055): `group token` mints one on
//! the Lead Daemon, and a Daemon that presents it with `group join` gets its
//! certificate signed. A token works once and for
//! [`JOIN_TOKEN_LIFETIME`]. The group store keeps each pending token by its
//! SHA-256 only ([`super::store`]), so a restart of the Lead Daemon forgets
//! none, and a copy of the store redeems none.

use std::time::{Duration, SystemTime};

use data_encoding::HEXLOWER;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use crate::endpoint::new_token;

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

impl JoinToken {
    /// A new token, minted at `now` and valid for [`JOIN_TOKEN_LIFETIME`].
    ///
    /// # Errors
    ///
    /// When the OS generator gives nothing.
    pub(crate) fn mint(now: SystemTime) -> Result<Self, getrandom::Error> {
        Ok(Self {
            token: SecretString::from(new_token()?),
            expires: now + JOIN_TOKEN_LIFETIME,
        })
    }

    /// Its SHA-256 in lowercase hex, which the group store keeps in its
    /// place.
    pub(crate) fn digest(&self) -> String {
        digest(self.token.expose_secret())
    }
}

/// The SHA-256 of the token `text`, in lowercase hex.
pub(super) fn digest(text: &str) -> String {
    HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

/// Why a join token was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JoinTokenRefused {
    /// No pending token is this one: never minted, already redeemed, or
    /// expired a lifetime ago or more and forgotten.
    #[error("the join token is unknown or already used")]
    Unknown,
    /// The token's lifetime ended before it was presented.
    #[error("the join token has expired")]
    Expired,
}
