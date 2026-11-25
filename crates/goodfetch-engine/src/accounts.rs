//! Who a Connection is (ADR 0149): its principal, resolved once at its
//! `hello` from how it reached the engine and held on the Connection, so
//! that dispatch reads it from memory and reads no store per message (ADR
//! 0140's `E1.store_reads`). For now a principal is the OS user, on a local
//! carriage, or a token's holder, on any other; an Account, a Named token's
//! scope and a Live view grant join it with the tickets that build them.
//!
//! A Daemon with accounts (`accounts.enabled`) holds its Accounts store,
//! opened once as it starts ([`AccountsStore`]); one without opens none and
//! knows only the OS user.

mod store;

use goodfetch_protocol::accounts::ACCOUNTS_ENABLED;
use goodfetch_protocol::connection::Surface;

pub use store::{AccountsStore, AccountsStoreError};

use crate::settings::{Resolved, get_or_default};

/// Whether a Daemon resolving `settings` has accounts: `accounts.enabled`
/// when it is set, and otherwise whether the Daemon is `exposed`, as
/// `daemon run` is and a machine's Daemon is not.
pub fn enabled(settings: Option<&Resolved>, exposed: bool) -> bool {
    get_or_default::<Option<bool>>(settings, &ACCOUNTS_ENABLED).unwrap_or(exposed)
}

/// How a Connection reached the engine, as who it may be reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carriage {
    /// An Attached engine's stdio or private endpoint, or a machine's
    /// Daemon's own listener, opened with its token: only the OS user
    /// reaches it.
    Local,
    /// An Exposed Daemon's address, even at loopback, where a proxy carries
    /// other machines' Connections (ADR 0049), or a Connection a gateway
    /// relays: whoever holds the token reaches it.
    Remote,
}

/// Who a Connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Principal {
    /// The OS user, on a local carriage: a person surface there answers
    /// as the person, as ADR 0029 has it.
    OsUser,
    /// Whoever holds the token its carriage asked for, whose surface claim
    /// counts for nothing: no person, until a Checked person arrives.
    TokenHolder,
}

impl Principal {
    /// Who a Connection that reached the engine on `carriage` is.
    pub(crate) fn of(carriage: Carriage) -> Self {
        match carriage {
            Carriage::Local => Self::OsUser,
            Carriage::Remote => Self::TokenHolder,
        }
    }

    /// Whether a Connection of this principal, its `hello` claiming
    /// `surface`, answers as a person: gives a person-only answer, and
    /// names the person it acts for in what it does.
    pub(crate) fn answers_as_a_person(self, surface: Surface) -> bool {
        match self {
            Self::OsUser => surface.is_a_person(),
            Self::TokenHolder => false,
        }
    }
}
