//! A crawl request's identity (ADRs 0005, 0093 and 0120): its method, its
//! URL's key and its body's hash, or the `key` the program declared, which
//! replaces them. The key is made from the URL as parsed by the fixed rules
//! RFC 3986 calls safe, while the request is still fetched at its own URL.
//! The Frontier's visited set keeps each identity it admitted, with the
//! crawl request it was admitted for, in the run file, so its duplicate
//! check holds nothing in memory (STYLE.md); a redirect that reaches an
//! identity admitted for another request ends that request `skipped` with
//! `duplicate`, and a redirect's final URL joins the set under its key.
//! Beside its own identity, a request has a learned one, its URL's key
//! without the parameters the learned rules found to have no effect on its
//! host; a request whose learned identity meets a visited one's is a
//! `duplicate` the rules merged.

use std::collections::{BTreeMap, BTreeSet};

use data_encoding::HEXLOWER;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};
use url::{Position, Url};

use crate::rules;
use crate::scope::{Narrowing, Scope};
use crate::staged::Staged;

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

/// A URL's key, made once for everything that reads it: as text, the
/// identity's part, and as a URL, which the learned rules read.
#[derive(Clone, Debug)]
pub(crate) struct Key {
    text: String,
    url: Url,
}

impl Key {
    /// The key of `url`, as parsed, its fragment kept when
    /// `keeps_fragment` is on.
    pub(crate) fn of(url: &Url, keeps_fragment: bool) -> Self {
        let text = key_text(url, keeps_fragment);
        let url = Url::parse(&text).expect("invariant: a URL's key is a URL");
        Self { text, url }
    }

    /// The key, as a URL.
    pub(crate) fn url(&self) -> &Url {
        &self.url
    }
}

/// The identities of a request whose URL's key is `key`, declared so: a
/// declared key has no learned one, since a program's own canonicalization
/// always wins.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn identities(
    connection: &Connection,
    key: &Key,
    declared: Declared<'_>,
) -> rusqlite::Result<Identities> {
    let plain = identity_of_key(&key.text, declared);
    if declared.key.is_some() {
        return Ok(Identities {
            plain,
            learned: None,
        });
    }
    let (stripped, rules) = rules::without_learned(connection, &key.url)?;
    let learned = (!rules.is_empty()).then(|| (identity(&stripped, declared), rules));
    Ok(Identities { plain, learned })
}

impl Identities {
    /// The identity its fixed rules make.
    pub(crate) fn plain(&self) -> &str {
        &self.plain
    }

    /// Its learned identity, with the rule that took the last parameter
    /// away, when the learned rules changed it.
    pub(crate) fn learned(&self) -> Option<(&str, i64)> {
        self.learned
            .as_ref()
            .and_then(|(learned, rules)| Some((learned.as_str(), *rules.last()?)))
    }
}

/// Whether the visited set holds `identities`, or `staged`, what an
/// attempt's earlier staged admissions visited, does: its own identity, or
/// its learned one as visited or as a visited one's learned identity.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn seen(
    connection: &Connection,
    identities: &Identities,
    staged: Option<&Staged>,
) -> rusqlite::Result<Seen> {
    let visitor = |identity: &str| -> rusqlite::Result<Option<String>> {
        Ok(visitor(connection, identity)?.or_else(|| {
            staged
                .and_then(|staged| staged.visitor(identity))
                .map(str::to_owned)
        }))
    };
    if let Some(visitor) = visitor(&identities.plain)? {
        return Ok(Seen::Visited(visitor));
    }
    let (probe, rules) = match &identities.learned {
        Some((learned, rules)) => {
            if let Some(visitor) = visitor(learned)? {
                return Ok(Seen::Merged {
                    rules: rules.clone(),
                    crawl_request: visitor,
                });
            }
            (learned.as_str(), Some(rules))
        }
        None => (identities.plain.as_str(), None),
    };
    let learned = match connection
        .prepare_cached("SELECT rule, crawl_request FROM learned_identities WHERE identity = ?1")?
        .query_row([probe], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))
        .optional()?
    {
        Some(learned) => Some(learned),
        None => staged
            .and_then(|staged| staged.learned(probe))
            .map(|(rule, crawl_request)| (rule, crawl_request.to_owned())),
    };
    Ok(match learned {
        Some((rule, crawl_request)) => {
            // The rules on either side merged it: its own, and the one
            // that gave the visited request its learned identity.
            let mut rules = rules.cloned().unwrap_or_default();
            if !rules.contains(&rule) {
                rules.push(rule);
            }
            Seen::Merged {
                rules,
                crawl_request,
            }
        }
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

/// Gives each visited identity of `merging`'s hosts that its rule changes
/// its learned identity, so a URL that differs from it only by what the
/// learned rules merge is a `duplicate` from then on, whichever of the two
/// was admitted; answers each set of crawl requests the rule just showed to
/// be one page, the requests admitted before it was learned among them.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn merge_learned(
    connection: &Connection,
    merging: &rules::Merging,
) -> rusqlite::Result<Vec<BTreeSet<String>>> {
    let mut pages: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();
    for (learned, rules, crawl_request) in visited_on(connection, &merging.hosts)? {
        let changed = rules.contains(&merging.rule);
        if changed {
            learn_identity(connection, &learned, merging.rule, &crawl_request)?;
        }
        let page = pages.entry(learned).or_default();
        page.0.insert(crawl_request);
        page.1 |= changed;
    }
    let mut groups = Vec::new();
    for (learned, (mut requests, changed)) in pages {
        if !changed {
            continue;
        }
        requests.extend(visitor(connection, &learned)?);
        if requests.len() > 1 {
            groups.push(requests);
        }
    }
    Ok(groups)
}

/// Gives every visited identity of `merging`'s hosts its learned identity
/// again under the rules that still hold, once `merging`'s rule stopped,
/// so nothing it merged merges a URL from then on.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn unmerge(connection: &Connection, merging: &rules::Merging) -> rusqlite::Result<()> {
    for (low, high) in host_ranges(connection, Keyed::Learned, &merging.hosts)? {
        connection
            .prepare_cached(
                "DELETE FROM learned_identities WHERE identity >= ?1 AND identity < ?2",
            )?
            .execute((low, high))?;
    }
    for (learned, rules, crawl_request) in visited_on(connection, &merging.hosts)? {
        if let Some(rule) = rules.last() {
            learn_identity(connection, &learned, *rule, &crawl_request)?;
        }
    }
    Ok(())
}

/// A table of identities, read by ranges of its key.
#[derive(Clone, Copy, Debug)]
enum Keyed {
    Visited,
    Learned,
}

impl Keyed {
    /// The first identity the table holds from `?1` on.
    fn first_from(self) -> &'static str {
        match self {
            Self::Visited => {
                "SELECT identity FROM visited WHERE identity >= ?1 ORDER BY identity LIMIT 1"
            }
            Self::Learned => {
                "SELECT identity FROM learned_identities WHERE identity >= ?1 ORDER BY identity LIMIT 1"
            }
        }
    }
}

/// The ranges of `table`'s key that hold the identities of the URLs on
/// `hosts`, each from its first identity to past its last: for each method
/// the table holds, each scheme a crawl fetches and each host, the URLs
/// with no port and those with one. The methods are found by stepping
/// from one to the next along the key, one lookup each, so no read goes
/// past the rows of the hosts; an identity a program declared, `key:…`,
/// names no URL.
fn host_ranges(
    connection: &Connection,
    table: Keyed,
    hosts: &[String],
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut methods = Vec::new();
    let mut from = String::new();
    while let Some(identity) = connection
        .prepare_cached(table.first_from())?
        .query_row([&from], |row| row.get::<_, String>(0))
        .optional()?
    {
        from = if identity.starts_with(DECLARED) {
            // Past every declared key: `;` follows `:`.
            "key;".to_owned()
        } else if let Some((method, _)) = identity.split_once(' ') {
            // Past every identity of `method`: `!` follows the space.
            let next = format!("{method}!");
            methods.push(method.to_owned());
            next
        } else {
            format!("{identity}\u{1}")
        };
    }
    let mut ranges = Vec::new();
    for method in &methods {
        for scheme in crate::url::FETCHED {
            for host in hosts {
                let prefix = format!("{method} {scheme}://{host}");
                // `0` follows `/`, and `;` follows `:`.
                ranges.push((format!("{prefix}/"), format!("{prefix}0")));
                ranges.push((format!("{prefix}:"), format!("{prefix};")));
            }
        }
    }
    Ok(ranges)
}

/// How a declared key's identity begins.
const DECLARED: &str = "key:";

/// Each visited identity whose URL is on one of `hosts`, under the learned
/// rules that hold, read once per host for the call: its identity under
/// them, the rules that changed it and its crawl request.
fn visited_on(
    connection: &Connection,
    hosts: &[String],
) -> rusqlite::Result<Vec<(String, Vec<i64>, String)>> {
    let mut holding = rules::Holding::of(connection);
    let mut found = Vec::new();
    for (low, high) in host_ranges(connection, Keyed::Visited, hosts)? {
        let mut statement = connection.prepare_cached(
            "SELECT identity, crawl_request FROM visited WHERE identity >= ?1 AND identity < ?2",
        )?;
        let candidates = statement
            .query_map((low, high), |row| {
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
            let (merged, rules) = holding.without_learned(&url)?;
            if rules.is_empty() {
                continue;
            }
            let mut learned = format!("{method} {}", url_key(&merged));
            if let Some(hash) = hash {
                learned.push(' ');
                learned.push_str(hash);
            }
            found.push((learned, rules, crawl_request));
        }
    }
    Ok(found)
}

/// `url`'s key, as a URL, without its fragment.
pub(crate) fn keyed(url: &Url) -> Url {
    Key::of(url, false).url
}

/// `url`'s key as [`url_key`] makes it, followed by its fragment when
/// `keeps_fragment` is on and it has one (ADR 0120): a Label keeping
/// fragments makes one request per fragment.
fn key_text(url: &Url, keeps_fragment: bool) -> String {
    let mut key = url_key(url);
    if keeps_fragment && let Some(fragment) = url.fragment() {
        key.push('#');
        key.push_str(fragment);
    }
    key
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
    /// Whether its URL's key keeps the fragment, as its Label's
    /// `crawl.keep_url_fragment` says (ADR 0120).
    pub keeps_fragment: bool,
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
    /// Its host is one the Run's domain policy refuses: the request ends
    /// `skipped` with `domain_policy`, and its handler does not run.
    DomainPolicy,
}

/// The identity of a request to `url` declared so: the declared key, kept
/// apart from every identity the Frontier makes (no method has a `:`), or
/// its method, its URL's key and, for a body that is not empty, its
/// SHA-256 in hexadecimal, separated by spaces, which neither a method nor
/// a serialized URL holds.
pub(crate) fn identity(url: &Url, declared: Declared<'_>) -> String {
    identity_of_key(&key_text(url, declared.keeps_fragment), declared)
}

/// The identity of a request whose URL's key is `key`, declared so, as
/// [`identity`] makes it.
fn identity_of_key(key: &str, declared: Declared<'_>) -> String {
    if let Some(declared_key) = declared.key {
        return format!("{DECLARED}{declared_key}");
    }
    let mut identity = format!("{} {key}", declared.method);
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
        .prepare_cached("SELECT crawl_request FROM visited WHERE identity = ?1")?
        .query_row([identity], |row| row.get(0))
        .optional()
}

/// Adds `identity` to the visited set for `crawl_request`, unless it is
/// there already.
pub(crate) fn visit(
    connection: &Connection,
    identity: &str,
    crawl_request: &str,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO visited (identity, crawl_request) VALUES (?1, ?2)
             ON CONFLICT (identity) DO NOTHING",
        )?
        .execute((identity, crawl_request))?;
    Ok(())
}

/// The Frontier's answer on one hop, as [`redirect_hop`] answers, on the
/// Scope the run file `connection` holds, its read counted on the hop's own
/// row. A function of its own, never inlined, so that `xtask counts`' Callgrind
/// run can name the hop's work (ADR 0140).
///
/// # Errors
///
/// As [`redirect_hop`] and [`Scope::read`].
#[inline(never)]
pub fn admit_hop(
    connection: &Connection,
    work: &CountedWork,
    policy: &Policy,
    crawl_request: &str,
    hop: (&Url, &HttpMethod, Option<&[u8]>),
) -> rusqlite::Result<Hop> {
    let (url, method, body) = hop;
    let scope = Scope::read_counted(
        connection,
        work,
        Row::HopInvariantReads,
        Row::HopCompilations,
    )?;
    redirect_hop(connection, &scope, policy, crawl_request, url, method, body)
}

/// Where a redirect hop of the crawl request `crawl_request` to `url`,
/// sent with `method` and `body`, leads it within `scope` and under the
/// Run's domain policy `policy` (ADRs 0093, 0118 and 0120), judged in that
/// order: `OutOfScope` when it leaves the scope, `DomainPolicy` when the
/// policy refuses its host, `Duplicate` when that identity was admitted for
/// another request, else `Follow`, a hop back to the request's own
/// identity included.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn redirect_hop(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    crawl_request: &str,
    url: &Url,
    method: &HttpMethod,
    body: Option<&[u8]>,
) -> rusqlite::Result<Hop> {
    match rewrite(connection, scope, policy, url)? {
        Some(SkipReason::OutOfScope) => return Ok(Hop::OutOfScope),
        Some(_) => return Ok(Hop::DomainPolicy),
        None => {}
    }
    let hop = identities(
        connection,
        &Key::of(url, false),
        Declared {
            method,
            body,
            key: None,
            keeps_fragment: false,
        },
    )?;
    Ok(match seen(connection, &hop, None)? {
        Seen::Visited(visitor)
        | Seen::Merged {
            crawl_request: visitor,
            ..
        } if visitor != crawl_request => Hop::Duplicate,
        _ => Hop::Follow,
    })
}

/// Why a crawl request sent on to `url`, by a Hook's rewrite or a redirect
/// hop, ends `skipped`, judged as an enqueue is (ADRs 0093, 0104 and 0120):
/// `out_of_scope` when it leaves `scope`, else `domain_policy` when the
/// Run's domain policy `policy` refuses its host, each read as the WHATWG
/// parser leaves it; `None` when it goes on.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn rewrite(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    url: &Url,
) -> rusqlite::Result<Option<SkipReason>> {
    if !scope.admits(connection, url, Narrowing::none())? {
        return Ok(Some(SkipReason::OutOfScope));
    }
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    Ok(matches!(judge(&request, policy), Judgment::Refused(_)).then_some(SkipReason::DomainPolicy))
}

/// Records the final URL `url` a redirect led the crawl request
/// `crawl_request` to, sent with `method` and `body`, under its key as
/// visited, so a later enqueue of it is a `duplicate` (ADR 0120), and
/// answers whether the request goes on: `Duplicate` when another request
/// was admitted for, or reached, that key meanwhile, between the hop's
/// check ([`redirect_hop`]) and its document's arrival, which leaves the
/// key that request's, else `Follow`. The check and the record are one
/// write, so of two chains reaching one key at once, only the first
/// records it (ADR 0120's #832 note).
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
) -> rusqlite::Result<Hop> {
    let reached = identities(
        connection,
        &Key::of(url, false),
        Declared {
            method,
            body,
            key: None,
            keeps_fragment: false,
        },
    )?;
    if let Seen::Visited(visitor)
    | Seen::Merged {
        crawl_request: visitor,
        ..
    } = seen(connection, &reached, None)?
        && visitor != crawl_request
    {
        return Ok(Hop::Duplicate);
    }
    visit_all(connection, &reached, crawl_request)?;
    Ok(Hop::Follow)
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
