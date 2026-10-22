//! Who a request is (ADRs 0024 and 0031): the sign-in driver, which owns
//! what every way of proving it shares, creating a sign-in and the cookie
//! that carries it, and the primitives that each verify one kind of
//! credential into a [`Principal`]. The single-user link is the first: on
//! a person's own machine, the Daemon's token, passed once in the
//! Console's URL fragment, signs in the OS user who owns the Daemon, with
//! no sign-in step (ADR 0024). Cookies come from the OS generator, never
//! from the engine's `Entropy` (ADR 0059). Sign-ins live as long as the
//! Daemon until the accounts store keeps them (spec #592).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use tokio::sync::Mutex;

/// How many bytes of the OS generator a sign-in's cookie holds.
const COOKIE_BYTES: usize = 32;

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
    /// The credential is none the primitive issued.
    TokenWrong,
}

impl SignInRefused {
    /// The rule's name, which the refusal's log record carries.
    pub fn reason(self) -> &'static str {
        match self {
            Self::TokenMissing => "token_missing",
            Self::TokenWrong => "token_wrong",
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
        } else if same_secret(token, &self.token) {
            Ok(Principal::OsUser)
        } else {
            Err(SignInRefused::TokenWrong)
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

/// The sign-in driver: signs a request's principal in through a primitive,
/// and answers who a cookie is.
pub struct SignInDriver {
    link: SingleUserLink,
    /// The cookie of each sign-in, all the OS user's.
    sign_ins: Mutex<BTreeSet<String>>,
}

impl SignInDriver {
    /// The driver of a single-user Daemon, signing in through `link`.
    pub fn new(link: SingleUserLink) -> Self {
        Self {
            link,
            sign_ins: Mutex::new(BTreeSet::new()),
        }
    }

    /// Signs in the principal the link's `token` proves.
    ///
    /// # Errors
    ///
    /// When `token` is missing or none the link carries, or the OS
    /// generator gives no cookie.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future signs nobody in.
    pub async fn with_link(&self, token: &str) -> Result<SignedIn, SignInFailure> {
        let Principal::OsUser = self.link.verify(token).map_err(SignInFailure::Refused)?;
        let cookie = new_cookie().map_err(SignInFailure::Generator)?;
        self.sign_ins.lock().await.insert(cookie.clone());
        Ok(SignedIn { cookie })
    }

    /// Who the sign-in `cookie` carries is, if it is one.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future changes nothing.
    pub async fn principal(&self, cookie: &str) -> Option<Principal> {
        self.sign_ins
            .lock()
            .await
            .iter()
            .any(|signed_in| same_secret(cookie, signed_in))
            .then_some(Principal::OsUser)
    }
}

/// Why signing in failed.
#[derive(Debug, PartialEq, Eq)]
pub enum SignInFailure {
    /// The credential signs nobody in.
    Refused(SignInRefused),
    /// The OS generator gave no cookie.
    Generator(getrandom::Error),
}

/// Whether two secrets are equal, in time that depends only on their
/// lengths.
fn same_secret(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

/// A new cookie value: 32 bytes of the OS generator, in hex.
fn new_cookie() -> Result<String, getrandom::Error> {
    let mut bytes = [0; COOKIE_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().fold(String::new(), |mut cookie, byte| {
        let _ = write!(cookie, "{byte:02x}");
        cookie
    }))
}

#[cfg(test)]
mod tests;
