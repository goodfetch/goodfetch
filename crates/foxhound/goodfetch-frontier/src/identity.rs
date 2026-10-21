//! A crawl request's identity (ADRs 0005, 0093 and 0120): its method, its
//! URL's key and its body's hash, or the `key` the program declared, which
//! replaces them. The key is made from the URL as parsed by the fixed rules
//! RFC 3986 calls safe, while the request is still fetched at its own URL.

use url::{Position, Url};

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
/// percent-encoded unreserved characters decoded (RFC 3986 §6.2.2); the
/// fragment is dropped; a `;jsessionid=` path parameter is dropped; and
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

/// A path segment without its `;jsessionid=` parameters.
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
    kept
}

/// `text` with each percent-encoded unreserved character decoded and every
/// other percent-encoding's hexadecimal digits uppercased (RFC 3986
/// §6.2.2.1 and §6.2.2.2); a `%` not followed by two hexadecimal digits
/// stays as written.
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
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
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
