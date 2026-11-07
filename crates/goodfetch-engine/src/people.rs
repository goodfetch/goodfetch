//! Who a request is (ADRs 0024, 0031 and 0098; spec #592): the sign-in
//! driver, which owns what every way of proving it shares, and the
//! primitives that each verify one kind of credential into a
//! [`Principal`]. The single-user link is the first: on a person's own
//! machine, the Daemon's token, passed once in the Console's URL fragment,
//! signs in the OS user who owns the Daemon, with no sign-in step (ADR
//! 0024).
//!
//! The driver owns the sign-in rate limit (`console.sign_in.rate` attempts a
//! minute per address), creating a sign-in and its cookie, the Audit log's
//! entries and the CSRF token of a sign-in's `POST`s. Sign-ins live in the
//! accounts store, so they outlive the Daemon, and end after
//! `console.sign_in.idle_expiry` unused, when revoked, or when the Console
//! key rotates: the store keeps an HMAC of each cookie under the key, never
//! the cookie, so a new key verifies none. Cookies, keys and CSRF tokens
//! come from the OS generator, never from the engine's `Entropy` (ADR
//! 0059).

mod key;
mod offline;
mod store;

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::{Actor, ENDPOINT_AUTHENTICATION_FAILED};
use goodfetch_protocol::catalog::SharedTypeValue;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::people::{
    CONSOLE_KEY_ROTATED, SIGN_IN_REVOKED, SIGNED_IN, SignInRow, SignInSummary,
};
use goodfetch_protocol::run::PointInTime;
use hmac::{Hmac, Mac as _};
use jiff::Timestamp;
use secrecy::zeroize::Zeroizing;
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::audit::{AuditError, AuditLog, Origin};
use crate::clock::Clock;
use crate::endpoint::{Listener, same_token};
use crate::tasks::TaskOwner;

pub use key::{ConsoleKey, KeyError};
pub use store::{AccountsStore, StoreError};

/// How many bytes of the OS generator a sign-in's identifier holds.
const ID_BYTES: usize = 16;
/// How many bytes of the OS generator a cookie's secret part holds.
const SECRET_BYTES: usize = 32;
/// How many bytes of the OS generator a wrapping key holds.
const WRAPPING_KEY_BYTES: usize = offline::KEY_LENGTH;
/// The window the rate limit counts attempts in.
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Who a signed-in request is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Principal {
    /// The OS user who owns a single-user Daemon (ADR 0031).
    OsUser,
}

/// Why a credential signed nobody in, by the endpoint table's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInRefused {
    /// No credential came.
    TokenMissing,
    /// The credential is none the primitive issued, or a sign-in that has
    /// ended.
    TokenWrong,
    /// The address tried to sign in more often than `console.sign_in.rate`
    /// allows in a minute.
    RateExceeded,
}

impl SignInRefused {
    /// The rule's name, which the refusal's log record and Audit log entry
    /// carry.
    pub fn reason(self) -> &'static str {
        match self {
            Self::TokenMissing => "token_missing",
            Self::TokenWrong => "token_wrong",
            Self::RateExceeded => "sign_in_rate_exceeded",
        }
    }
}

/// The single-user sign-in link's primitive (ADR 0024): the token the
/// link's fragment carries signs in the OS user.
pub struct SingleUserLink {
    token: String,
}

impl SingleUserLink {
    /// The link whose fragment carries `token`, the Daemon's.
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// The principal `token` proves.
    fn verify(&self, token: &str) -> Result<Principal, SignInRefused> {
        if token.is_empty() {
            Err(SignInRefused::TokenMissing)
        } else if same_token(token, &self.token) {
            Ok(Principal::OsUser)
        } else {
            Err(SignInRefused::TokenWrong)
        }
    }
}

/// How long sign-ins last and how often an address may try: the values of
/// `console.sign_in.idle_expiry` and `console.sign_in.rate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignInPolicy {
    /// How long a sign-in lasts unused.
    pub idle_expiry: Duration,
    /// How many sign-in attempts one address may make in a minute.
    pub rate: u64,
}

impl Default for SignInPolicy {
    /// The settings' defaults: 30 days, 10 attempts a minute.
    fn default() -> Self {
        Self {
            idle_expiry: Duration::from_hours(30 * 24),
            rate: 10,
        }
    }
}

/// A sign-in just created: the cookie that carries it.
#[derive(Debug)]
pub struct SignedIn {
    cookie: String,
}

impl SignedIn {
    /// The cookie's value.
    pub fn cookie(&self) -> &str {
        &self.cookie
    }
}

/// Why signing in failed.
#[derive(Debug)]
pub enum SignInFailure {
    /// The credential signs nobody in, or the address tried too often.
    Refused(SignInRefused),
    /// The OS generator gave no cookie.
    Generator(getrandom::Error),
    /// The accounts store, the Console key or the Audit log failed.
    People(PeopleError),
}

/// A sign-in's offline cache key (ADR 0031): in the clear, for its own
/// page alone, and wrapped under the sign-in's wrapping key, as the page
/// keeps it. Its key is zeroized when it drops and never formatted.
pub struct OfflineKey {
    key: Zeroizing<Vec<u8>>,
    wrapped: Vec<u8>,
}

impl OfflineKey {
    /// The cache key, 32 bytes.
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Its wrapped form, which only its sign-in's cookie unwraps.
    pub fn wrapped(&self) -> &[u8] {
        &self.wrapped
    }
}

impl std::fmt::Debug for OfflineKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OfflineKey").finish_non_exhaustive()
    }
}

/// Why a sign-in got no offline cache key.
#[derive(Debug)]
pub enum OfflineKeyRefused {
    /// The cookie carries no sign-in that has not ended.
    NotSignedIn,
    /// The wrapped key does not open under the sign-in's wrapping key: it
    /// is another sign-in's, or was changed.
    DoesNotOpen,
    /// The OS generator gave no key.
    Generator(getrandom::Error),
}

/// Why the accounts store or the Console key could not serve a call.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PeopleError {
    /// The accounts store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The Console key failed.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// The Audit log could not record the change: a sign-in is then not
    /// made, and an ending, which no failure may keep alive, is.
    #[error("the Audit log cannot record the change: {0}")]
    Unrecorded(#[source] AuditError),
}

/// What a sign-in driver is made of.
pub struct SignIns {
    /// The single-user link's primitive.
    pub link: SingleUserLink,
    /// The OS user's name, the person a single-user Daemon signs in.
    pub person: String,
    /// The accounts store of the Daemon's major.
    pub store: AccountsStore,
    /// The Console key every major shares.
    pub key: ConsoleKey,
    /// The user's Audit log.
    pub audit: AuditLog,
    /// How long sign-ins last and how often an address may try.
    pub policy: SignInPolicy,
    /// Where time is read.
    pub clock: Clock,
    /// Where the store's, the key's and the Audit log's blocking calls run.
    pub owner: Arc<TaskOwner>,
}

/// The sign-in driver: signs a request's principal in through a primitive,
/// answers who a cookie is, and lists, revokes and ends sign-ins.
#[derive(Clone)]
pub struct SignInDriver {
    inner: Arc<Inner>,
}

struct Inner {
    link: SingleUserLink,
    person: String,
    store: AccountsStore,
    key: ConsoleKey,
    audit: AuditLog,
    policy: SignInPolicy,
    clock: Clock,
    owner: Arc<TaskOwner>,
    /// The moments each address tried to sign in within the last minute.
    attempts: Mutex<BTreeMap<IpAddr, VecDeque<Instant>>>,
}

impl SignInDriver {
    /// The driver `parts` make.
    pub fn new(parts: SignIns) -> Self {
        let SignIns {
            link,
            person,
            store,
            key,
            audit,
            policy,
            clock,
            owner,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                link,
                person,
                store,
                key,
                audit,
                policy,
                clock,
                owner,
                attempts: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// How long a sign-in lasts unused: `console.sign_in.idle_expiry`.
    pub fn idle_expiry(&self) -> Duration {
        self.inner.policy.idle_expiry
    }

    /// The person a single-user Daemon signs in: its OS user's name.
    pub fn person(&self) -> &str {
        &self.inner.person
    }

    /// Signs in the principal the link's `token` proves, for a request
    /// from `from`, after counting the attempt against the address's rate.
    ///
    /// # Errors
    ///
    /// When `from` tried too often in the last minute, `token` is missing
    /// or none the link carries, the OS generator gives no cookie, the
    /// accounts store or the Console key fails, or the Audit log cannot
    /// record the sign-in, which is then not made.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in created, its cookie
    /// unsent, so it signs nobody in.
    pub async fn with_link(&self, token: &str, from: IpAddr) -> Result<SignedIn, SignInFailure> {
        if !self.inner.attempted(from).await {
            return Err(SignInFailure::Refused(SignInRefused::RateExceeded));
        }
        let Principal::OsUser = self
            .inner
            .link
            .verify(token)
            .map_err(SignInFailure::Refused)?;
        let id = random_hex(ID_BYTES).map_err(SignInFailure::Generator)?;
        let secret = random_hex(SECRET_BYTES).map_err(SignInFailure::Generator)?;
        let wrapping_key = random_hex(WRAPPING_KEY_BYTES).map_err(SignInFailure::Generator)?;
        let cookie = format!("{id}.{secret}");
        let now = self.inner.now().to_string();
        let inner = Arc::clone(&self.inner);
        let created = cookie.clone();
        self.blocking(move || {
            let key = inner.key.read_or_create()?;
            inner.store.insert(&SignInRow {
                id: id.clone(),
                verifier: mac(&key, b"sign-in", &created),
                person: inner.person.clone(),
                created: now.clone(),
                last_used: now,
                wrapping_key,
            })?;
            let recorded = inner.record(
                Some(Surface::Console),
                &SIGNED_IN,
                json!({ "sign_in": id, "with": "single_user_link" }),
            );
            if recorded.is_err()
                && let Err(error) = inner.store.remove(&id)
            {
                // Its cookie is never sent, so it signs nobody in either way.
                tracing::error!(%error, "the accounts store kept a sign-in the Audit log could not record");
            }
            recorded
        })
        .await
        .map_err(SignInFailure::People)?;
        Ok(SignedIn { cookie })
    }

    /// Who the sign-in `cookie` carries is, if it is one that has not
    /// ended; its idle expiry starts again. A sign-in unused for longer
    /// than `console.sign_in.idle_expiry` ends here.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in's last use recorded.
    pub async fn principal(&self, cookie: &str) -> Option<Principal> {
        let cookie = cookie.to_owned();
        let inner = Arc::clone(&self.inner);
        self.blocking(move || inner.live(&cookie, true).map(|_| Principal::OsUser))
            .await
    }

    /// The CSRF token every `POST` of the sign-in `cookie` carries, if it is
    /// one that has not ended: an HMAC of the cookie under the Console key,
    /// so it changes with the key and is never stored.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in's last use recorded.
    pub async fn csrf_token(&self, cookie: &str) -> Option<String> {
        let cookie = cookie.to_owned();
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            inner.live(&cookie, true)?;
            let key = inner.key.read_or_create().ok()?;
            Some(mac(&key, b"csrf", &cookie))
        })
        .await
    }

    /// Whether `token` is the CSRF token of the sign-in `cookie`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in's last use recorded.
    pub async fn csrf_checked(&self, cookie: &str, token: &str) -> bool {
        self.csrf_token(cookie)
            .await
            .is_some_and(|expected| !token.is_empty() && same_token(token, &expected))
    }

    /// The offline cache key of the sign-in `cookie` carries (ADR 0031): with
    /// no `wrapped` key, a new one from the OS generator; else the one
    /// `wrapped` holds, which opens only under this sign-in's wrapping key.
    /// The wrapping key never leaves the Daemon, so once the sign-in ends no
    /// cache it encrypts can be read.
    ///
    /// # Errors
    ///
    /// `NotSignedIn` when `cookie` carries no sign-in that has not ended,
    /// `DoesNotOpen` when `wrapped` is another sign-in's or was changed, and
    /// `Generator` when the OS generator gives no key.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in's last use recorded.
    pub async fn offline_key(
        &self,
        cookie: &str,
        wrapped: Option<&[u8]>,
    ) -> Result<OfflineKey, OfflineKeyRefused> {
        let cookie = cookie.to_owned();
        let wrapped = wrapped.map(<[u8]>::to_vec);
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            let row = inner
                .live(&cookie, true)
                .ok_or(OfflineKeyRefused::NotSignedIn)?;
            let refused = |unwrapped| match unwrapped {
                offline::Unwrapped::Generator(error) => OfflineKeyRefused::Generator(error),
                offline::Unwrapped::DoesNotOpen => OfflineKeyRefused::DoesNotOpen,
            };
            match wrapped {
                None => offline::created(&row.wrapping_key, &row.id)
                    .map(|(key, wrapped)| OfflineKey { key, wrapped })
                    .map_err(refused),
                Some(wrapped) => offline::opened(&row.wrapping_key, &row.id, &wrapped)
                    .map(|key| OfflineKey { key, wrapped })
                    .map_err(refused),
            }
        })
        .await
    }

    /// Ends the sign-in `cookie` carries, as its person signing out: whether
    /// it was one that had not ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in ended.
    pub async fn sign_out(&self, cookie: &str) -> bool {
        let cookie = cookie.to_owned();
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            let Some(row) = inner.live(&cookie, false) else {
                return false;
            };
            let removed = inner.store.remove(&row.id).unwrap_or_else(|error| {
                tracing::error!(%error, "the accounts store did not end a sign-in");
                false
            });
            // The sign-in has ended, and the Console's sign-out answer has
            // no failure to show, so a missing entry is logged.
            if removed
                && let Err(error) = inner.record(
                    Some(Surface::Console),
                    &SIGN_IN_REVOKED,
                    json!({ "sign_in": row.id, "by": "sign_out" }),
                )
            {
                tracing::error!(%error, "the Audit log did not record a sign-out");
            }
            removed
        })
        .await
    }

    /// Every sign-in that has not ended, oldest first.
    ///
    /// # Errors
    ///
    /// When the accounts store fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future changes nothing.
    pub async fn list(&self) -> Result<Vec<SignInSummary>, PeopleError> {
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            let now = inner.now();
            Ok(inner
                .store
                .all()?
                .into_iter()
                .filter(|row| !inner.idle(row, now))
                .map(|row| SignInSummary {
                    id: row.id,
                    person: row.person,
                    created: PointInTime::from_rfc3339(row.created),
                    last_used: PointInTime::from_rfc3339(row.last_used),
                })
                .collect())
        })
        .await
    }

    /// Ends the sign-in `id`, at a person's request through `surface`:
    /// whether one that had not ended had it.
    ///
    /// # Errors
    ///
    /// When the accounts store fails, or `Unrecorded` when the Audit log
    /// cannot record an ending, which is made all the same.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the sign-in ended.
    pub async fn revoke(&self, id: &str, surface: Option<Surface>) -> Result<bool, PeopleError> {
        let id = id.to_owned();
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            let revoked = inner.store.remove(&id)?;
            if revoked {
                inner.record(
                    surface,
                    &SIGN_IN_REVOKED,
                    json!({ "sign_in": id, "by": "sign_in.revoke" }),
                )?;
            }
            Ok(revoked)
        })
        .await
    }

    /// Replaces the Console key, at a person's request through `surface`,
    /// which ends every sign-in of every major: how many sign-ins of this
    /// major's store it ended.
    ///
    /// # Errors
    ///
    /// When the OS generator gives no key, or the key or the accounts store
    /// cannot be written; `Unrecorded` when the Audit log cannot record the
    /// rotation, which is made all the same.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the key replaced.
    pub async fn rotate_key(&self, surface: Option<Surface>) -> Result<u64, PeopleError> {
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            inner.key.replace()?;
            let signed_out = inner.store.clear()?;
            inner.record(
                surface,
                &CONSOLE_KEY_ROTATED,
                json!({ "signed_out": signed_out }),
            )?;
            Ok(signed_out)
        })
        .await
    }

    /// Records that `listener` refused a request for the endpoint table's
    /// rule `reason`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the entry recorded.
    pub async fn refused(&self, listener: Listener, reason: &'static str) {
        let listener = listener.name();
        let inner = Arc::clone(&self.inner);
        self.blocking(move || {
            let origin = Origin {
                actor: Actor::ClientSurface {
                    surface: Surface::Console,
                },
                surface: None,
                layer: None,
                run_id: None,
            };
            if let Err(error) = inner.audit.record(
                &origin,
                &ENDPOINT_AUTHENTICATION_FAILED,
                json!({ "listener": listener, "reason": reason }),
                inner.now(),
            ) {
                tracing::error!(%error, "the Audit log did not record a refused request");
            }
        })
        .await;
    }

    async fn blocking<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.inner.owner.blocking(work).await
    }
}

impl Inner {
    /// Wall time now.
    fn now(&self) -> Timestamp {
        self.clock.timestamp()
    }

    /// Counts an attempt to sign in from `from`: whether the address may
    /// still try within the last minute.
    async fn attempted(&self, from: IpAddr) -> bool {
        let now = self.clock.now();
        let mut attempts = self.attempts.lock().await;
        attempts.retain(|_, moments| {
            while moments
                .front()
                .is_some_and(|moment| now.duration_since(*moment) >= RATE_WINDOW)
            {
                moments.pop_front();
            }
            !moments.is_empty()
        });
        let moments = attempts.entry(from).or_default();
        if u64::try_from(moments.len()).unwrap_or(u64::MAX) >= self.policy.rate {
            return false;
        }
        moments.push_back(now);
        true
    }

    /// The row of the sign-in `cookie` carries, if it has not ended; with
    /// `touch`, its last use is now. A sign-in past its idle expiry is
    /// removed.
    fn live(&self, cookie: &str, touch: bool) -> Option<SignInRow> {
        let id = sign_in_id(cookie)?;
        let key = match self.key.read_or_create() {
            Ok(key) => key,
            Err(error) => {
                tracing::error!(%error, "the Console key cannot be read, so no sign-in verifies");
                return None;
            }
        };
        let row = match self.store.find(id) {
            Ok(row) => row?,
            Err(error) => {
                tracing::error!(%error, "the accounts store cannot be read, so no sign-in verifies");
                return None;
            }
        };
        if !same_token(&mac(&key, b"sign-in", cookie), &row.verifier) {
            return None;
        }
        let now = self.now();
        if self.idle(&row, now) {
            if let Err(error) = self.store.remove(&row.id) {
                tracing::error!(%error, "the accounts store did not remove an expired sign-in");
            }
            return None;
        }
        if touch && let Err(error) = self.store.touch(&row.id, &now.to_string()) {
            tracing::warn!(%error, "the accounts store did not record a sign-in's use");
        }
        Some(row)
    }

    /// Whether `row` has gone unused longer than its idle expiry at `now`.
    fn idle(&self, row: &SignInRow, now: Timestamp) -> bool {
        let Ok(last_used) = row.last_used.parse::<Timestamp>() else {
            return true;
        };
        let Ok(idle_expiry) = jiff::SignedDuration::try_from(self.policy.idle_expiry) else {
            return false;
        };
        now.duration_since(last_used) > idle_expiry
    }

    /// Appends an entry of `kind` to the Audit log, by the person through
    /// `surface`; the caller decides what its failure means for its change.
    fn record(
        &self,
        surface: Option<Surface>,
        kind: &SharedTypeValue,
        details: Value,
    ) -> Result<(), PeopleError> {
        let origin = Origin {
            actor: Actor::Person {
                name: self.person.clone(),
            },
            surface,
            layer: None,
            run_id: None,
        };
        self.audit
            .record(&origin, kind, details, self.now())
            .map(drop)
            .map_err(PeopleError::Unrecorded)
    }
}

/// The OS user's name, as the engine's environment map gives it: `USER`
/// or `LOGNAME` on Unix, `USERNAME` on Windows; `os_user` when none is
/// set, as for a Daemon started with an empty environment.
pub fn os_user(environment: &BTreeMap<OsString, OsString>) -> String {
    ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .find_map(|name| {
            environment
                .get(&OsString::from(name))
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("os_user")
        .to_owned()
}

/// The identifier of the sign-in a cookie's value carries: its part before
/// the first `.`; none for a value with no `.`.
pub fn sign_in_id(cookie: &str) -> Option<&str> {
    cookie.split_once('.').map(|(id, _)| id)
}

/// HMAC-SHA-256 of `purpose` and `value` under `key`, in hex.
fn mac(key: &[u8], purpose: &[u8], value: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("invariant: HMAC takes a key of any length");
    mac.update(purpose);
    mac.update(b"\0");
    mac.update(value.as_bytes());
    data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
}

/// `bytes` bytes of the OS generator, in hex.
fn random_hex(bytes: usize) -> Result<String, getrandom::Error> {
    let mut drawn = vec![0; bytes];
    getrandom::fill(&mut drawn)?;
    Ok(data_encoding::HEXLOWER.encode(&drawn))
}

#[cfg(test)]
mod tests;
