//! Which partition of the Response cache keeps a response, and which a
//! lookup reads (ADR 0046), as pure functions of the Session's [`Keeper`]
//! and the request and response.
//!
//! A request with no cookie or credential, whose response sets no cookie
//! and is not `private`, is the public partition's. Any other is its
//! Session's keeper's: its Saved identity's, named as the identity is,
//! which outlives the Session and leaves with the identity; or, for a
//! Session with none, the Session's own, `session:<Run ID>:<Session ID>`,
//! which leaves when the Session ends. With no keeper, such a response is
//! kept nowhere. A lookup reads the public partition and its keeper's for
//! a request with no cookie or credential, and its keeper's alone for one
//! with, so no Session ever reads another's partition.

use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunId;

use super::freshness::{self, CacheRequest, ResponseHead};

/// The partition of the responses that carry nothing of a person's.
pub(crate) const PUBLIC: &str = "public";

/// What the name of a Session's own partition starts with.
const SESSION: &str = "session:";

/// Whose partition a Session's responses that are not public are kept in
/// (ADR 0046).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Keeper {
    /// The Session's Saved identity, by its name: its entries outlive the
    /// Session, and leave when the identity is deleted.
    SavedIdentity(String),
    /// The Session itself, which has no Saved identity: its entries leave
    /// when it ends.
    Session {
        /// The Run the Session is one of.
        run_id: RunId,
        /// The Session.
        session_id: SessionId,
    },
}

impl Keeper {
    /// The name of its partition in the index. None for a Saved identity
    /// whose name another partition's could be read as, `public` or one
    /// starting `session:`: its responses are then kept nowhere rather
    /// than shared.
    pub(crate) fn partition(&self) -> Option<String> {
        match self {
            Self::SavedIdentity(name) => {
                (name != PUBLIC && !name.starts_with(SESSION)).then(|| name.clone())
            }
            Self::Session { run_id, session_id } => Some(of_session(run_id, session_id)),
        }
    }
}

/// The name of the partition of the Session `session_id` of the Run
/// `run_id`: the Run ID has no `:`, so no two Sessions' names meet.
pub(crate) fn of_session(run_id: &RunId, session_id: &SessionId) -> String {
    format!("{SESSION}{run_id}:{}", session_id.0)
}

/// The partition `head`, the response to `request`, is kept in under
/// `keeper`: the public one, or the keeper's for a request with a cookie
/// or a credential or a response that sets a cookie or is `private`; none
/// when such a response has no partition to be kept in.
pub(crate) fn kept_in(
    keeper: Option<&Keeper>,
    request: &CacheRequest,
    head: &ResponseHead,
) -> Option<String> {
    if freshness::credentialed(request) || freshness::personal(head) {
        keeper?.partition()
    } else {
        Some(PUBLIC.to_owned())
    }
}

/// The partitions a lookup of `request` under `keeper` reads, the second
/// the first again when it reads one: the public one and the keeper's for
/// a request with no cookie or credential, the keeper's alone for one
/// with; none when such a request has no partition to read.
pub(crate) fn read_from(keeper: Option<&Keeper>, request: &CacheRequest) -> Option<[String; 2]> {
    let own = keeper.and_then(Keeper::partition);
    if freshness::credentialed(request) {
        let own = own?;
        return Some([own.clone(), own]);
    }
    Some([PUBLIC.to_owned(), own.unwrap_or_else(|| PUBLIC.to_owned())])
}

#[cfg(test)]
mod tests;
