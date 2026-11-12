//! The cookies of a Session's own HTTP transport (RFC 6265; ADR 0021;
//! issue 1605): each Session that sends its HTTP itself keeps one jar, so
//! a cookie a hop's answer sets is sent by the Session's later hops, a
//! redirect's next hop first, and by no other Session's. A Session that
//! may move to the browser keeps its cookies in its browser context
//! instead, which browser-fetch sends from (`transports`).
//!
//! The storage model is `cookie_store`'s: the domain and path a cookie is
//! set for, `Secure`, its expiry. The jar adds what a browser adds: a
//! cookie whose `Domain` is a public suffix is kept for its own host alone
//! when the host is that suffix, and dropped otherwise, so no site sets a
//! cookie for every site of `.com`; and it is bounded as a browser's is
//! (STYLE.md, bounded memory), a `Set-Cookie` past [`MAX_SIZE`] bytes or
//! past [`MAX_COOKIES`] kept being dropped.
#![expect(
    clippy::disallowed_types,
    reason = "a jar and the table of jars are each read or written in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use cookie_store::{Cookie, CookieStore, RawCookie};
use goodfetch_protocol::page::SessionId;
use url::Url;

use crate::driver::{Hop, Reuse, Sending, Sent, Transport};

/// The longest `Set-Cookie` value kept, in bytes: RFC 6265bis's limit on a
/// cookie's name and value, which browsers apply.
const MAX_SIZE: usize = 4096;

/// The most cookies one jar keeps: Chrome's own bound on a profile's jar
/// is of this order.
const MAX_COOKIES: usize = 3000;

/// One Session's cookie jar.
#[derive(Default)]
pub(crate) struct Jar(Mutex<CookieStore>);

impl fmt::Debug for Jar {
    /// How many cookies it holds, never one of them (STYLE.md, secrets).
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_any()
            .count();
        formatter
            .debug_struct("Jar")
            .field("cookies", &held)
            .finish()
    }
}

impl Jar {
    /// The `Cookie` header a request to `url` carries: the jar's cookies
    /// that match it, those of the longest path first, then by name; none
    /// when none matches.
    pub(crate) fn cookie(&self, url: &Url) -> Option<String> {
        let jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut matching: Vec<(usize, &str, &str)> = jar
            .matches(url)
            .into_iter()
            .map(|cookie| {
                (
                    cookie.path().map_or(0, str::len),
                    cookie.name(),
                    cookie.value(),
                )
            })
            .collect();
        matching.sort_by(|(path, name, _), (other_path, other_name, _)| {
            other_path.cmp(path).then(name.cmp(other_name))
        });
        let pairs: Vec<String> = matching
            .into_iter()
            .map(|(_, name, value)| format!("{name}={value}"))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    /// Keeps the cookies the answer from `url` sets in `headers`, names
    /// in lowercase: one a browser would refuse is dropped.
    pub(crate) fn keep(&self, url: &Url, headers: &[(String, String)]) {
        let set = headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value);
        let mut jar = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for value in set {
            if value.len() > MAX_SIZE {
                continue;
            }
            let Ok(mut cookie) = RawCookie::parse(value.as_str()) else {
                continue;
            };
            if let Some(domain) = cookie.domain().map(|domain| {
                domain
                    .trim_start_matches('.')
                    .trim_end_matches('.')
                    .to_ascii_lowercase()
            }) && psl::domain_str(&domain).is_none()
                && !domain.is_empty()
            {
                // A public suffix: the host's own cookie when the host is
                // that suffix, no cookie otherwise (RFC 6265 §5.3 step 5).
                let host = url.host_str().map(|host| host.trim_end_matches('.'));
                if host != Some(domain.as_str()) {
                    continue;
                }
                cookie.unset_domain();
            }
            // A cookie the storage model refuses, such as one for another
            // domain, is no cookie.
            let Ok(cookie) = Cookie::try_from_raw_cookie(&cookie, url) else {
                continue;
            };
            if !cookie.is_expired() && !make_room(&mut jar, &cookie) {
                continue;
            }
            let _ = jar.insert(cookie.into_owned(), url);
        }
    }
}

/// Whether `jar` may take `cookie`: it holds it already, or holds fewer
/// than [`MAX_COOKIES`] once those that expired are dropped, which a full
/// jar does first.
fn make_room(jar: &mut CookieStore, cookie: &Cookie<'_>) -> bool {
    let domain = String::from(&cookie.domain);
    if jar.contains_any(&domain, &cookie.path, cookie.name())
        || jar.iter_any().count() < MAX_COOKIES
    {
        return true;
    }
    let expired: Vec<(String, String, String)> = jar
        .iter_any()
        .filter(|held| held.is_expired())
        .map(|held| {
            (
                String::from(&held.domain),
                held.path.to_string(),
                held.name().to_owned(),
            )
        })
        .collect();
    for (domain, path, name) in &expired {
        jar.remove(domain, path, name);
    }
    !expired.is_empty()
}

/// The jars of a Run's Sessions that send their HTTP themselves, each
/// made at its Session's first hop and dropped as the Session retires.
#[derive(Debug, Default)]
pub(crate) struct Jars(Mutex<BTreeMap<SessionId, Arc<Jar>>>);

impl Jars {
    /// The jar of `session`, empty at its first use.
    pub(crate) fn of(&self, session: &SessionId) -> Arc<Jar> {
        Arc::clone(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(session.clone())
                .or_default(),
        )
    }

    /// Drops the jar of `session`, retired: its cookies end with it.
    pub(crate) fn forget(&self, session: &SessionId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session);
    }
}

/// A transport as one Session sends on it: each hop presents the
/// Session's cookies after the transport's own headers, where the browser
/// places `Cookie`, and each answer's cookies are kept in its jar.
pub(crate) struct WithJar {
    pub(crate) transport: Arc<dyn Transport>,
    pub(crate) jar: Arc<Jar>,
}

impl Transport for WithJar {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            let sent = self.transport.send(hop, reuse).await?;
            self.jar.keep(&hop.url, &sent.headers);
            Ok(sent)
        })
    }

    fn presents(&self, url: &Url) -> Vec<(String, String)> {
        let mut headers = self.transport.presents(url);
        if let Some(cookie) = self.jar.cookie(url) {
            headers.push(("cookie".to_owned(), cookie));
        }
        headers
    }
}

#[cfg(test)]
mod tests;
