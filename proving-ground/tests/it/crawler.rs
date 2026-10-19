//! A plain breadth-first crawler over one task's host: the known-right crawl
//! of the Crawl families' tests, and, through its knobs, the wrong crawls
//! tools make (following hidden links, fetching every URL of a page, staying
//! in a trap, ignoring `Retry-After`, never following a redirect).
#![expect(
    clippy::disallowed_methods,
    reason = "the crawler waits the `Retry-After` the Proving ground sets, one second at most"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use regex::Regex;

use crate::proving_ground::{Answer, ProvingGround};

/// A page the crawl reached, at the URL its last redirect named.
pub(crate) struct Fetched {
    pub(crate) url: String,
    pub(crate) answer: Answer,
}

/// A crawl's knobs; [`Crawler::new`] gives the known-right crawl.
pub(crate) struct Crawler<'a> {
    proving_ground: &'a ProvingGround,
    /// Follows links hidden from people, as a crawl that hits honeypots does.
    pub(crate) follow_hidden: bool,
    /// The frontier key of a URL: the fixed URL rules (ADR 0120), or the URL
    /// itself for a crawl that fetches every URL of a page.
    pub(crate) key: fn(&str) -> String,
    /// Path prefixes the crawl closes after this many pages under each, as a
    /// frontier that learned the trap does; a prefix not listed is open.
    pub(crate) closes: Vec<(&'static str, usize)>,
    /// Redirects followed for one link; 0 follows none.
    pub(crate) max_redirects: usize,
    /// Waits a `429` or `503`'s `Retry-After` before asking again, up to
    /// [`RETRIES`] times; otherwise asks again at once.
    pub(crate) honour_retry_after: bool,
    /// The most requests the crawl makes.
    pub(crate) limit: usize,
}

/// What fetching one link reached.
enum Reached {
    /// A page answered `200`.
    Page(Fetched),
    /// Anything else: an error, a loop, a rate limit that never lifted.
    Nothing,
    /// The crawl's request limit.
    Limit,
}

/// How many times a link answered `429` or `503` is asked again.
const RETRIES: usize = 5;

impl<'a> Crawler<'a> {
    pub(crate) fn new(proving_ground: &'a ProvingGround) -> Self {
        Self {
            proving_ground,
            follow_hidden: false,
            key: fixed_rules,
            closes: Vec::new(),
            max_redirects: 20,
            honour_retry_after: true,
            limit: 2_000,
        }
    }

    /// Crawls from `start_url` within its origin and returns each page that
    /// answered `200`, in the order reached.
    pub(crate) fn crawl(&self, start_url: &str) -> Vec<Fetched> {
        let origin = origin(start_url);
        let mut seen = BTreeSet::from([(self.key)(start_url)]);
        let mut queue = VecDeque::from([start_url.to_owned()]);
        let mut under: BTreeMap<&str, usize> = BTreeMap::new();
        let mut requests = 0;
        let mut pages = Vec::new();
        while let Some(url) = queue.pop_front() {
            if let Some(prefix) = self.closed_prefix(&url, &origin) {
                let count = under.entry(prefix.0).or_default();
                if *count >= prefix.1 {
                    continue;
                }
                *count += 1;
            }
            let fetched = match self.fetch(&url, &mut requests) {
                Reached::Page(fetched) => fetched,
                Reached::Nothing => continue,
                Reached::Limit => break,
            };
            for link in links(&fetched.answer.body, self.follow_hidden) {
                let target = resolve(&fetched.url, &link);
                if origin_of_same(&target, &origin) && seen.insert((self.key)(&target)) {
                    queue.push_back(target);
                }
            }
            pages.push(fetched);
        }
        pages
    }

    fn closed_prefix(&self, url: &str, origin: &str) -> Option<(&'static str, usize)> {
        let path = url.strip_prefix(origin).unwrap_or(url);
        self.closes
            .iter()
            .copied()
            .find(|(prefix, _)| path.starts_with(prefix))
    }

    /// Fetches one link, following its redirects and waiting out rate
    /// limits.
    fn fetch(&self, url: &str, requests: &mut usize) -> Reached {
        let mut url = url.to_owned();
        let mut chain = BTreeSet::new();
        let mut redirects = 0;
        let mut retries = 0;
        loop {
            if *requests >= self.limit {
                return Reached::Limit;
            }
            *requests += 1;
            let answer = self.proving_ground.get(&url);
            match answer.status {
                200 => return Reached::Page(Fetched { url, answer }),
                301 | 302 | 303 | 307 | 308 => {
                    let Some(location) = answer.header("location") else {
                        return Reached::Nothing;
                    };
                    let next = resolve(&url, location);
                    redirects += 1;
                    if redirects > self.max_redirects || !chain.insert(url.clone()) {
                        return Reached::Nothing;
                    }
                    url = next;
                }
                429 | 503 if retries < RETRIES => {
                    retries += 1;
                    if self.honour_retry_after
                        && let Some(seconds) = answer
                            .header("retry-after")
                            .and_then(|value| value.parse::<u64>().ok())
                    {
                        std::thread::sleep(Duration::from_secs(seconds));
                    }
                }
                _ => return Reached::Nothing,
            }
        }
    }
}

/// The `scheme://host:port` of a URL.
pub(crate) fn origin(url: &str) -> String {
    let after_scheme = url.find("://").map_or(0, |at| at + 3);
    let end = url[after_scheme..]
        .find('/')
        .map_or(url.len(), |at| after_scheme + at);
    url[..end].to_owned()
}

fn origin_of_same(url: &str, origin: &str) -> bool {
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The links of a page: each `<a>`'s `href` (those hidden from people only
/// when `hidden` is set) and each sitemap `<loc>`.
pub(crate) fn links(html: &str, hidden: bool) -> Vec<String> {
    let anchor = Regex::new(r"<a\s([^>]*)>").expect("a valid pattern");
    let href = Regex::new(r#"href="([^"]*)""#).expect("a valid pattern");
    let loc = Regex::new(r"<loc>([^<]*)</loc>").expect("a valid pattern");
    // An element that hides itself hides the links inside it.
    let hiding =
        Regex::new(r#"(?s)<(div|span|p)\s[^>]*(hidden|display:none)[^>]*>.*?</(div|span|p)>"#)
            .expect("a valid pattern");
    let shown = if hidden {
        html.to_owned()
    } else {
        hiding.replace_all(html, "").into_owned()
    };
    let mut links: Vec<String> = anchor
        .captures_iter(&shown)
        .filter(|tag| hidden || !hidden_from_people(&tag[1]))
        .filter_map(|tag| {
            href.captures(&tag[1])
                .map(|link| link[1].replace("&amp;", "&"))
        })
        .collect();
    links.extend(
        loc.captures_iter(html)
            .map(|link| link[1].replace("&amp;", "&")),
    );
    links
}

/// Whether an `<a>`'s attributes hide it from people.
fn hidden_from_people(attributes: &str) -> bool {
    attributes.contains("hidden") || attributes.contains("display:none")
}

/// The absolute URL of `href` on the page at `base`.
pub(crate) fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or_default();
    if href.contains("://") {
        return href.to_owned();
    }
    let origin = origin(base);
    if href.starts_with('/') {
        return format!("{origin}{href}");
    }
    let path = &base[origin.len()..];
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
    format!("{origin}{directory}/{href}")
}

/// A URL's frontier key under the fixed rules every frontier applies
/// (ADR 0120): the fragment and `;jsessionid=` dropped, unreserved
/// characters percent-decoded, `utm_*`, `gclid` and `fbclid` dropped and the
/// other parameters sorted.
pub(crate) fn fixed_rules(url: &str) -> String {
    let url = url.split('#').next().unwrap_or_default();
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let path = match path.find(";jsessionid=") {
        Some(at) => &path[..at],
        None => path,
    };
    let mut parameters: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or_default();
            !pair.is_empty() && !name.starts_with("utm_") && name != "gclid" && name != "fbclid"
        })
        .collect();
    parameters.sort_unstable();
    let path = decode_unreserved(path);
    if parameters.is_empty() {
        path
    } else {
        format!("{path}?{}", parameters.join("&"))
    }
}

/// The URL itself, for a crawl that fetches every URL it finds.
pub(crate) fn every_url(url: &str) -> String {
    url.split('#').next().unwrap_or_default().to_owned()
}

fn decode_unreserved(path: &str) -> String {
    let mut decoded = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(at) = rest.find('%') {
        decoded.push_str(&rest[..at]);
        let byte = rest
            .get(at + 1..at + 3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match byte {
            Some(byte) if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) => {
                decoded.push(char::from(byte));
                rest = &rest[at + 3..];
            }
            _ => {
                decoded.push('%');
                rest = &rest[at + 1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// The first capture of `pattern` in `page`, if it matches.
pub(crate) fn field(page: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(page)
        .map(|captures| captures[1].to_owned())
}
