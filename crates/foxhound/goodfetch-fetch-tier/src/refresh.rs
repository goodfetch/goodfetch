//! A document's refresh stub (ADR 0118; pitfall U13): its first refresh
//! directive as the HTML standard reads it, a `Refresh` header, else the
//! first `<meta http-equiv="refresh">` outside `<noscript>`, each parsed by
//! the standard's shared declarative refresh steps into a delay in whole
//! seconds and a URL resolved against the document's. A refresh with no URL
//! names the document's own.

use std::time::Duration;

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use url::Url;

/// A document's refresh directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refresh {
    /// How long the document asks to be shown first.
    pub(crate) delay: Duration,
    /// The URL it refreshes to: the document's own when it names none.
    pub(crate) url: Url,
}

impl Refresh {
    /// Whether it refreshes to `document`'s own URL, a reload, its fragment
    /// aside.
    pub(crate) fn reloads(&self, document: &Url) -> bool {
        let mut url = self.url.clone();
        let mut document = document.clone();
        url.set_fragment(None);
        document.set_fragment(None);
        url == document
    }
}

/// The refresh directive of the document at `url` with `headers` (names
/// lower-cased) and `body`, whose `<meta>` is read only when it is HTML;
/// `None` when it has none that parses.
pub(crate) fn directive(
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    html: bool,
    work: &CountedWork,
) -> Option<Refresh> {
    let header = in_header(url, headers);
    if header.is_some() || !html {
        return header;
    }
    in_markup(url, body, work)
}

/// The first `Refresh` header among `headers` (names lower-cased) of the
/// document at `url` that parses.
pub(crate) fn in_header(url: &Url, headers: &[(String, String)]) -> Option<Refresh> {
    headers
        .iter()
        .filter(|(name, _)| name == "refresh")
        .find_map(|(_, value)| parse(value, url))
}

/// The first `<meta http-equiv="refresh">` outside `<noscript>` of the
/// HTML document `body` at `url` that parses: a whole parse of the
/// document, which blocks, for a hop no untouched tree reads (a sitemap's,
/// or a body whose type sniffs as no document).
pub(crate) fn in_markup(url: &Url, body: &[u8], work: &CountedWork) -> Option<Refresh> {
    let document = work.parse_html(Row::PageParses, String::from_utf8_lossy(body).as_ref());
    in_tree(
        url,
        &document,
        &work.literal_selector(Row::PageSelectorCompilations, META),
    )
}

/// What names a document's refresh `<meta>` elements, among others.
pub(crate) const META: &str = "meta[http-equiv][content]";

/// The first `<meta http-equiv="refresh">` outside `<noscript>` of
/// `document`, the untouched tree of the HTML document at `url`, that
/// parses, `meta` being [`META`]'s matcher.
pub(crate) fn in_tree(
    url: &Url,
    document: &ParsedHtml,
    meta: &dom_query::Matcher,
) -> Option<Refresh> {
    let metas = document.select_matcher(meta);
    let metas = metas.nodes().iter().filter(|meta| {
        !meta.ancestors(None).iter().any(|ancestor| {
            ancestor
                .node_name()
                .is_some_and(|name| &*name == "noscript")
        })
    });
    first_of(
        url,
        metas.map(|meta| (meta.attr("http-equiv"), meta.attr("content"))),
    )
}

/// The first refresh directive that parses among `metas`, each
/// `<meta>`'s `http-equiv` and `content`, in document order, of the HTML
/// document at `url`.
pub(crate) fn first_of<T: AsRef<str>>(
    url: &Url,
    metas: impl IntoIterator<Item = (Option<T>, Option<T>)>,
) -> Option<Refresh> {
    metas
        .into_iter()
        .filter(|(equiv, _)| {
            equiv
                .as_ref()
                .is_some_and(|equiv| equiv.as_ref().trim().eq_ignore_ascii_case("refresh"))
        })
        .find_map(|(_, content)| content.and_then(|content| parse(content.as_ref(), url)))
}

/// HTML's ASCII whitespace.
fn whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

/// The shared declarative refresh steps over `input`, a `Refresh` header's
/// value or a `<meta>`'s `content`, against the document at `url`.
pub(crate) fn parse(input: &str, url: &Url) -> Option<Refresh> {
    let rest = input.trim_start_matches(whitespace);
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let (time, rest) = rest.split_at(digits);
    if time.is_empty() && !rest.starts_with('.') {
        return None;
    }
    let seconds = if time.is_empty() {
        0
    } else {
        time.parse::<u64>().unwrap_or(u64::MAX)
    };
    let mut rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    if let Some(first) = rest.chars().next() {
        if !(first == ';' || first == ',' || whitespace(first)) {
            return None;
        }
        rest = rest.trim_start_matches(whitespace);
        rest = rest
            .strip_prefix(';')
            .or_else(|| rest.strip_prefix(','))
            .unwrap_or(rest);
        rest = rest.trim_start_matches(whitespace);
    }
    let target = if rest.is_empty() {
        url.clone()
    } else {
        url.join(url_string(rest)).ok()?
    };
    Some(Refresh {
        delay: Duration::from_secs(seconds),
        url: target,
    })
}

/// The URL string of the refresh steps' remainder `rest`: after an
/// optional `url=` in any case, its quotes removed. A remainder that starts
/// with a `u` but no `url=` is the URL string whole.
fn url_string(rest: &str) -> &str {
    let after = if rest.starts_with(['u', 'U']) {
        let named = rest
            .get(..3)
            .filter(|word| word.eq_ignore_ascii_case("url"))
            .map(|_| rest[3..].trim_start_matches(whitespace))
            .and_then(|after| after.strip_prefix('='))
            .map(|after| after.trim_start_matches(whitespace));
        let Some(after) = named else {
            return rest;
        };
        after
    } else {
        rest
    };
    match after.chars().next() {
        Some(quote @ ('\'' | '"')) => {
            let quoted = &after[1..];
            quoted.split(quote).next().unwrap_or(quoted)
        }
        _ => after,
    }
}

/// The refresh-stub parser over arbitrary bytes, as headers and documents
/// a crawl reads hold them: the fuzz target's body (ADR 0067).
pub fn fuzz_refresh(data: &[u8]) {
    let url = Url::parse("http://fuzz.test/a/b?c").expect("invariant: the URL parses");
    let text = String::from_utf8_lossy(data);
    let _ = parse(&text, &url);
    let _ = directive(&url, &[], data, true, &CountedWork::default());
}

#[cfg(test)]
mod tests;
