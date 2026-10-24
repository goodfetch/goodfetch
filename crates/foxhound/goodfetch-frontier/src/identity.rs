//! A crawl request's identity (ADRs 0005, 0093 and 0120): its method, its
//! URL's key and its body's hash, or the `key` the program declared, which
//! replaces them. The key is made from the URL as parsed by the fixed rules
//! RFC 3986 calls safe, while the request is still fetched at its own URL.
//! The Frontier's visited set keeps each identity it admitted, with the
//! crawl request it was admitted for, in the run file, so its duplicate
//! check holds nothing in memory (STYLE.md); a redirect that reaches an
//! identity admitted for another request ends that request `skipped` with
//! `duplicate`, and a redirect's final URL joins the set under its key.

use data_encoding::HEXLOWER;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};
use url::{Position, Url};

use crate::rules;
use crate::scope::{Narrowing, Scope};

/// The Frontier's visited set: each identity admitted, with the ID of the
/// crawl request it was admitted for, or that reached it by a redirect;
/// and, beside it, each identity a visited one has once the parameters
/// learned to have no effect are taken from its URL, with the rule that
/// took the last one away and the crawl request.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-visited",
    statements: &[
        "CREATE TABLE IF NOT EXISTS visited (
            identity TEXT NOT NULL PRIMARY KEY,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS learned_identities (
            identity TEXT NOT NULL PRIMARY KEY,
            rule INTEGER NOT NULL,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID",
    ],
};

/// A request's identities: the one its fixed rules make, and the one it
/// has once the parameters learned to have no effect on its host are taken
/// from its URL, with the rules that took them, when any did.
#[derive(Clone, Debug)]
pub(crate) struct Identities {
    plain: String,
    learned: Option<(String, Vec<i64>)>,
}

/// Whether the Frontier saw a request's identities before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// Never.
    New,
    /// Its own identity, for the crawl request named.
    Visited(String),
    /// Its identity under learned rules, for the crawl request named: the
    /// rules merged it.
    Merged {
        /// The rules that merged it.
        rules: Vec<i64>,
        /// The crawl request it merged into.
        crawl_request: String,
    },
}

/// The identities of a request to `url` declared so: a declared key has
/// no learned one, since a program's own canonicalization always wins.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn identities(
    connection: &Connection,
    url: &Url,
    declared: Declared<'_>,
) -> rusqlite::Result<Identities> {
    let plain = identity(url, declared);
    if declared.key.is_some() {
        return Ok(Identities {
            plain,
            learned: None,
        });
    }
    let (stripped, rules) = rules::without_learned(connection, &keyed(url))?;
    let learned = (!rules.is_empty()).then(|| (identity(&stripped, declared), rules));
    Ok(Identities { plain, learned })
}

/// Whether the visited set holds `identities`: its own identity, or its
/// learned one as visited or as a visited one's learned identity.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn seen(connection: &Connection, identities: &Identities) -> rusqlite::Result<Seen> {
    if let Some(visitor) = visitor(connection, &identities.plain)? {
        return Ok(Seen::Visited(visitor));
    }
    let (probe, rules) = match &identities.learned {
        Some((learned, rules)) => {
            if let Some(visitor) = visitor(connection, learned)? {
                return Ok(Seen::Merged {
                    rules: rules.clone(),
                    crawl_request: visitor,
                });
            }
            (learned.as_str(), Some(rules))
        }
        None => (identities.plain.as_str(), None),
    };
    let learned = connection
        .prepare_cached("SELECT rule, crawl_request FROM learned_identities WHERE identity = ?1")?
        .query_row([probe], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))
        .optional()?;
    Ok(match learned {
        Some((rule, crawl_request)) => Seen::Merged {
            rules: rules.cloned().unwrap_or_else(|| vec![rule]),
            crawl_request,
        },
        None => Seen::New,
    })
}

/// Adds `identities` to the visited set for `crawl_request`.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn visit_all(
    connection: &Connection,
    identities: &Identities,
    crawl_request: &str,
) -> rusqlite::Result<()> {
    visit(connection, &identities.plain, crawl_request)?;
    if let Some((learned, rules)) = &identities.learned {
        learn_identity(connection, learned, rules[rules.len() - 1], crawl_request)?;
    }
    Ok(())
}

fn learn_identity(
    connection: &Connection,
    identity: &str,
    rule: i64,
    crawl_request: &str,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO learned_identities (identity, rule, crawl_request) VALUES (?1, ?2, ?3)
             ON CONFLICT (identity) DO NOTHING",
        )?
        .execute((identity, rule, crawl_request))?;
    Ok(())
}

/// Gives each visited identity of `no_effect`'s host whose URL carries its
/// parameter its learned identity, so a URL that differs from it only by
/// learned parameters is a `duplicate` from then on, whichever of the two
/// carried them.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn merge_learned(
    connection: &Connection,
    no_effect: &rules::NoEffect,
) -> rusqlite::Result<()> {
    let mut statement = connection
        .prepare("SELECT identity, crawl_request FROM visited WHERE instr(identity, ?1) > 0")?;
    let candidates = statement
        .query_map([format!("://{}", no_effect.host)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (visited, crawl_request) in candidates {
        let Some((method, rest)) = visited.split_once(' ') else {
            continue;
        };
        let (key, hash) = rest
            .split_once(' ')
            .map_or((rest, None), |(key, hash)| (key, Some(hash)));
        let Ok(url) = Url::parse(key) else {
            continue;
        };
        if url.host_str() != Some(no_effect.host.as_str()) {
            continue;
        }
        let (stripped, rules) = rules::without_learned(connection, &url)?;
        if !rules.contains(&no_effect.rule) {
            continue;
        }
        let mut learned = format!("{method} {}", url_key(&stripped));
        if let Some(hash) = hash {
            learned.push(' ');
            learned.push_str(hash);
        }
        learn_identity(connection, &learned, no_effect.rule, &crawl_request)?;
    }
    Ok(())
}

/// `url`'s key, as a URL.
pub(crate) fn keyed(url: &Url) -> Url {
    Url::parse(&url_key(url)).expect("invariant: a URL's key is a URL")
}

/// How a crawl request is sent beside its URL, and the key a program may
/// declare in place of its identity.
#[derive(Clone, Copy, Debug)]
pub struct Declared<'a> {
    /// The method it is sent with.
    pub method: &'a HttpMethod,
    /// The body it is sent with, if any.
    pub body: Option<&'a [u8]>,
    /// The identity the program declared, if any, which replaces its
    /// method, its URL's key and its body's hash.
    pub key: Option<&'a str>,
}

/// Where a redirect hop leads a crawl request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hop {
    /// It leaves the crawl's scope: the request ends `skipped` with
    /// `out_of_scope`, and its handler does not run.
    OutOfScope,
    /// It is followed: its key is the request's own, or no other request's.
    Follow,
    /// Its key was admitted for another request: the request ends
    /// `skipped` with `duplicate`, and its handler does not run.
    Duplicate,
}

/// The identity of a request to `url` declared so: the declared key, kept
/// apart from every identity the Frontier makes (no method has a `:`), or
/// its method, its URL's key and, for a body that is not empty, its
/// SHA-256 in hexadecimal, separated by spaces, which neither a method nor
/// a serialized URL holds.
pub(crate) fn identity(url: &Url, declared: Declared<'_>) -> String {
    if let Some(key) = declared.key {
        return format!("key:{key}");
    }
    let mut identity = format!("{} {}", declared.method, url_key(url));
    if let Some(body) = declared.body.filter(|body| !body.is_empty()) {
        identity.push(' ');
        identity.push_str(&HEXLOWER.encode(&Sha256::digest(body)));
    }
    identity
}

/// The crawl request `identity` was admitted for, or reached by a
/// redirect; `None` while the visited set lacks it.
pub(crate) fn visitor(connection: &Connection, identity: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT crawl_request FROM visited WHERE identity = ?1",
            [identity],
            |row| row.get(0),
        )
        .optional()
}

/// Adds `identity` to the visited set for `crawl_request`, unless it is
/// there already.
pub(crate) fn visit(
    connection: &Connection,
    identity: &str,
    crawl_request: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO visited (identity, crawl_request) VALUES (?1, ?2)
         ON CONFLICT (identity) DO NOTHING",
        (identity, crawl_request),
    )?;
    Ok(())
}

/// Where a redirect hop of the crawl request `crawl_request` to `url`,
/// sent with `method` and `body`, leads it within `scope` (ADRs 0093 and
/// 0120): `OutOfScope` when it leaves the scope, `Duplicate` when that
/// identity was admitted for another request, else `Follow`, a hop back to
/// the request's own identity included.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn redirect_hop(
    connection: &Connection,
    scope: &Scope,
    crawl_request: &str,
    url: &Url,
    method: &HttpMethod,
    body: Option<&[u8]>,
) -> rusqlite::Result<Hop> {
    if !scope.admits(connection, url, Narrowing::none())? {
        return Ok(Hop::OutOfScope);
    }
    let hop = identities(
        connection,
        url,
        Declared {
            method,
            body,
            key: None,
        },
    )?;
    Ok(match seen(connection, &hop)? {
        Seen::Visited(visitor)
        | Seen::Merged {
            crawl_request: visitor,
            ..
        } if visitor != crawl_request => Hop::Duplicate,
        _ => Hop::Follow,
    })
}

/// Records the final URL `url` a redirect led the crawl request
/// `crawl_request` to, sent with `method` and `body`, under its key as
/// visited, so a later enqueue of it is a `duplicate` (ADR 0120).
///
/// # Errors
///
/// When the run file cannot be written.
pub fn redirect_reached(
    connection: &Connection,
    crawl_request: &str,
    url: &Url,
    method: &HttpMethod,
    body: Option<&[u8]>,
) -> rusqlite::Result<()> {
    let reached = identities(
        connection,
        url,
        Declared {
            method,
            body,
            key: None,
        },
    )?;
    visit_all(connection, &reached, crawl_request)
}

/// The query parameters a key drops by name: `Crawl4AI`'s click IDs.
const TRACKING: [&str; 2] = ["gclid", "fbclid"];

/// The query parameters a key drops by their name's prefix: Crawlee's.
const TRACKING_PREFIX: &str = "utm_";

/// The Servlet session path parameter's name (Jakarta Servlet 6.0 §7.1.3),
/// matched in any case.
const SESSION_PARAMETER: &str = "jsessionid";

/// The key of `url`, as parsed: the URL's part of a crawl request's
/// identity, by ADR 0120's fixed rules, in order. Hexadecimal digits of
/// the path's and the query's percent-encodings are uppercased and
/// percent-encoded unreserved characters decoded (RFC 3986 §6.2.2),
/// unless decoding would make a new percent-encoding with a `%` written
/// as is; the
/// fragment is dropped; a `;jsessionid=` path parameter is dropped, unless
/// that leaves a dot segment; and
/// the query's `utm_*`, `gclid` and `fbclid` parameters and empty pairs
/// are dropped and the rest sorted by name, keeping the order of one
/// name's values, an empty query dropped. Nothing else changes: the
/// path's case, `%2F` against `/` and `+` against `%20` stay apart.
pub fn url_key(url: &Url) -> String {
    let mut key = url[..Position::BeforePath].to_owned();
    let path = normalize_encoding(&url[Position::BeforePath..Position::AfterPath]);
    let mut segments = path.split('/').map(without_session);
    if let Some(first) = segments.next() {
        key.push_str(&first);
    }
    for segment in segments {
        key.push('/');
        key.push_str(&segment);
    }
    if let Some(query) = url.query() {
        let query = normalize_encoding(query);
        let mut pairs: Vec<&str> = query
            .split('&')
            .filter(|pair| !pair.is_empty() && !is_tracking(name(pair)))
            .collect();
        pairs.sort_by_key(|pair| name(pair));
        if !pairs.is_empty() {
            key.push('?');
            key.push_str(&pairs.join("&"));
        }
    }
    key
}

/// The name of a query pair: what precedes its first `=`.
fn name(pair: &str) -> &str {
    pair.split_once('=').map_or(pair, |(name, _)| name)
}

fn is_tracking(name: &str) -> bool {
    name.starts_with(TRACKING_PREFIX) || TRACKING.contains(&name)
}

/// A path segment without its `;jsessionid=` parameters, unless dropping
/// them would leave a dot segment, which the parser would then resolve:
/// such a segment stays as written, so the key of a key is itself and
/// `a/.;jsessionid=1/b` keeps apart from `a/b`.
fn without_session(segment: &str) -> String {
    let mut parameters = segment.split(';');
    let mut kept = parameters.next().unwrap_or_default().to_owned();
    for parameter in parameters {
        let is_session = parameter
            .split_once('=')
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case(SESSION_PARAMETER));
        if !is_session {
            kept.push(';');
            kept.push_str(parameter);
        }
    }
    if matches!(kept.as_str(), "." | "..") {
        return segment.to_owned();
    }
    kept
}

/// `text` with each percent-encoded unreserved character decoded and every
/// other percent-encoding's hexadecimal digits uppercased (RFC 3986
/// §6.2.2.1 and §6.2.2.2); a `%` not followed by two hexadecimal digits
/// stays as written. Decoding never makes a percent-encoding the text did
/// not hold: an encoded hexadecimal digit right after such a `%`, or after
/// it and one digit, stays encoded, so `%3%30` keeps apart from `%30` as
/// the WHATWG URL Standard's percent-decoding keeps them, and the key of a
/// key is itself.
fn normalize_encoding(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut normalized = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let encoded = (bytes[at] == b'%')
            .then(|| Some((hex(*bytes.get(at + 1)?)?, hex(*bytes.get(at + 2)?)?)))
            .flatten();
        if let Some((high, low)) = encoded {
            let byte = high * 16 + low;
            let is_unreserved =
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
            if is_unreserved && !(byte.is_ascii_hexdigit() && ends_in_stray_percent(&normalized)) {
                normalized.push(char::from(byte));
            } else {
                normalized.push('%');
                normalized.push(char::from(bytes[at + 1].to_ascii_uppercase()));
                normalized.push(char::from(bytes[at + 2].to_ascii_uppercase()));
            }
            at += 3;
        } else {
            let next = text[at..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(offset, _)| at + offset);
            normalized.push_str(&text[at..next]);
            at = next;
        }
    }
    normalized
}

/// Whether `normalized` ends in a `%` not followed by two hexadecimal
/// digits, alone or with one digit, so a hexadecimal digit written next
/// would start or complete a percent-encoding. Every percent-encoding
/// `normalize_encoding` writes is whole, so such a `%` is one the text
/// held as written.
fn ends_in_stray_percent(normalized: &str) -> bool {
    match normalized.as_bytes() {
        [.., b'%'] => true,
        [.., b'%', digit] => digit.is_ascii_hexdigit(),
        _ => false,
    }
}

/// The fuzz target's body for the URL key (ADR 0067): reads `data` as an
/// `href` on a page, resolved as the Frontier resolves an enqueue; the key
/// of a URL that parses must be a URL whose key is itself. It panics only
/// on a bug.
pub fn fuzz_url_key(data: &[u8]) {
    let href = String::from_utf8_lossy(data);
    let base = Url::parse("http://shop.test/shelf/page;v=1?b=2&a=1#top")
        .expect("invariant: the base URL parses");
    let Ok(url) = crate::url::parse(&href, Some(&base)) else {
        return;
    };
    let key = url_key(&url);
    let reparsed = Url::parse(&key).expect("invariant: a URL's key is a URL");
    assert_eq!(url_key(&reparsed), key, "a key's key is the key itself");
}

/// The value of a hexadecimal digit.
fn hex(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

#[cfg(test)]
mod tests;
