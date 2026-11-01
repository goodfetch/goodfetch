//! The fetch edge pages on `fetch-edge.test` and its named hosts (ADR
//! 0118), each served on demand: redirects, refresh stubs, `Retry-After` in
//! each form, cut bodies, and requests a site never processed. A page that
//! fails once does so on its path's first request since `serve` started,
//! and answers after that. The Proving ground's observations record what
//! each case needs: each request's method, connection, protocol and whether
//! it repeats an earlier one, the time to the next request for its host,
//! each answer's body sent against its declared length, and the address
//! family of each connection.
//!
//! - `fetch-edge.test` serves every page below, on HTTP/2 or HTTP/1.1 as ALPN
//!   chose; `h1.fetch-edge.test` the same pages on HTTP/1.1 alone, and
//!   `h2.fetch-edge.test` the same, its certificate also naming
//!   `misdirected.fetch-edge.test`, so a client may coalesce the two.
//! - `misdirected.fetch-edge.test` answers `421` to a request that came on
//!   another host's connection, and its pages on its own.
//! - `goaway.fetch-edge.test` answers a connection's first request; the
//!   next one on that connection gets a `GOAWAY` naming the first's stream
//!   (or, over HTTP/1.1, the connection's close).
//! - `ipv6.fetch-edge.test` serves the pages on IPv4; the Proving ground's
//!   IPv6 loopback address on its port drops every connection attempt.
//! - `/misdirected` answers `421` once on any connection, for a client that
//!   never coalesces hosts on one connection, as the HTTP tier does not.
//! - `/text/shift-jis` and `/json/bom` are the bodies `page.request`'s
//!   decoding cases read (ADR 0119): text in `Shift_JIS`, labelled so, and
//!   JSON after a UTF-8 BOM.
//! - `/sniffed/<case>` serves a body with no `Content-Type`, typed by MIME
//!   sniffing, and `/redirect/301/untyped` a 301 with neither a type nor a
//!   body (ADR 0119).
//! - `/endless/pdf` serves an `application/pdf` head and a chunked body
//!   that never ends, over HTTP/1.1, so a crawl that skips the type before
//!   its body is read ends at once, and one that reads it never does (ADR
//!   0119; pitfall H16).

use std::fmt::Write as _;

use crate::clock;
use crate::observation::{self, Protocol};
use crate::raw::{Answer, Cut, ErrorCode, Framing, Page, Refusal, Request};

/// The host name the pages are served on.
pub(super) const HOST: &str = "fetch-edge.test";
pub(super) const HTTP1_HOST: &str = "h1.fetch-edge.test";
pub(super) const HTTP2_HOST: &str = "h2.fetch-edge.test";
pub(super) const MISDIRECTED_HOST: &str = "misdirected.fetch-edge.test";
const GOAWAY_HOST: &str = "goaway.fetch-edge.test";
const IPV6_HOST: &str = "ipv6.fetch-edge.test";

/// The named hosts besides [`HOST`].
pub(super) const NAMED_HOSTS: [&str; 5] = [
    HTTP1_HOST,
    HTTP2_HOST,
    MISDIRECTED_HOST,
    GOAWAY_HOST,
    IPV6_HOST,
];

/// How far behind the Proving ground's clock a skewed `Date` is.
const SKEW_SECONDS: u64 = 3600;

/// The `Retry-After` the pages ask for.
const RETRY_SECONDS: u64 = 2;

/// The `Retry-After` past any short `politeness.retry_after.max`.
const LONG_RETRY_SECONDS: u64 = 3600;

const ROBOTS: &str = "User-agent: *\nDisallow: /private/\n";

/// The answer to `request` on `host`, asked for its path `earlier` times
/// before.
pub(super) fn answer(host: &str, request: &Request, earlier: u64) -> Answer {
    if host == GOAWAY_HOST && request.earlier_on_connection > 0 {
        return Answer::Refuse(Refusal::GoAway);
    }
    if host == MISDIRECTED_HOST
        && request
            .server_name
            .as_deref()
            .is_some_and(|name| observation::host_name(name) != MISDIRECTED_HOST)
    {
        return Answer::Page(Page::text(421, "misdirected request"));
    }
    let path = request.path();
    let first = earlier == 0;
    if let Some(rest) = path.strip_prefix("/redirect/") {
        return redirect(rest, request);
    }
    if let Some(rest) = path.strip_prefix("/refresh/") {
        return refresh(rest).map_or_else(not_found, Answer::Page);
    }
    if let Some(rest) = path.strip_prefix("/retry-after/") {
        return retry_after(rest, first, path);
    }
    if let Some(rest) = path.strip_prefix("/cut/") {
        return cut(rest, path);
    }
    if let Some(case) = path.strip_prefix("/sniffed/") {
        return SNIFFED
            .iter()
            .find(|(name, _)| *name == case)
            .map_or_else(not_found, |(_, body)| {
                Answer::Page(Page::untyped(200, *body))
            });
    }
    match path {
        "/robots.txt" => Answer::Page(Page::text(200, ROBOTS)),
        // An answer with no document, which a browser's navigation abandons.
        "/no-content" => Answer::Page(Page::text(204, "")),
        "/close-delimited" => Answer::Page(page(path).framed(Framing::Close)),
        // A file no crawl may read before it judged its type (ADR 0119).
        "/endless/pdf" => Answer::Filler {
            page: Page::new(200, "application/pdf", Vec::new()).framed(Framing::Chunked),
            length: None,
        },
        // 日本語 in Shift_JIS, for `page.request`'s decoding cases (ADR 0119).
        "/text/shift-jis" => Answer::Page(Page::new(
            200,
            "text/plain; charset=Shift_JIS",
            [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA],
        )),
        "/json/bom" => Answer::Page(Page::new(
            200,
            "application/json",
            &b"\xEF\xBB\xBF{\"bom\":true}"[..],
        )),
        "/refused" if first => Answer::Refuse(Refusal::Reset(ErrorCode::RefusedStream)),
        "/refused/always" => Answer::Refuse(Refusal::Reset(ErrorCode::RefusedStream)),
        "/http1-required" if request.protocol == Protocol::Http2 => {
            Answer::Refuse(Refusal::Reset(ErrorCode::Http11Required))
        }
        "/misdirected" if first => Answer::Page(Page::text(421, "misdirected request")),
        "/calm" if first => Answer::Refuse(Refusal::Reset(ErrorCode::EnhanceYourCalm)),
        "/reuse/close" if request.earlier_on_connection > 0 => Answer::Refuse(Refusal::Close),
        "/reuse/408" if request.earlier_on_connection > 0 => {
            Answer::Page(Page::text(408, "request timeout").with("connection", "close"))
        }
        _ => Answer::Page(page(path)),
    }
}

fn not_found() -> Answer {
    Answer::Page(Page::text(404, "not found"))
}

/// The bodies `/sniffed/<case>` serves with no `Content-Type`, each typed
/// as Chromium's MIME sniffer types it (ADR 0119): HTML after leading
/// spaces, XML, plain text, binary bytes and a PDF.
const SNIFFED: [(&str, &[u8]); 5] = [
    (
        "html",
        b"\n  <!DOCTYPE html><html><head><title>Sniffed</title></head><body><h1>Sniffed as HTML</h1></body></html>\n",
    ),
    (
        "xml",
        b"<?xml version=\"1.0\"?><note><to>Sniffed as XML</to></note>\n",
    ),
    ("text", b"Sniffed as plain text.\n"),
    ("binary", b"Sniffed\x00as\x01binary\x02bytes"),
    ("pdf", b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n1 0 obj << >> endobj\ntrailer << >>\n%%EOF\n"),
];

/// A plain page naming its path, long enough to be cut.
fn page(path: &str) -> Page {
    let mut html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{path}</title></head><body><main><h1 id=\"path\">{path}</h1>"
    );
    for line in 1..=40 {
        let _ = write!(html, "<p>Line {line} of the fetch edge page at {path}.</p>");
    }
    html.push_str("</main></body></html>");
    Page::html(200, html)
}

/// `/redirect/<rest>`.
fn redirect(rest: &str, request: &Request) -> Answer {
    let to = |status: u16, location: &str| {
        Answer::Page(Page::text(status, "").with("location", location.to_owned()))
    };
    // The request's own port, for a redirect to another host on it.
    let port = request
        .authority
        .rsplit_once(':')
        .map_or(String::new(), |(_, port)| format!(":{port}"));
    match rest {
        "loop" => to(302, "/redirect/loop"),
        "cookie"
            if request
                .header("cookie")
                .is_some_and(|cookie| cookie.contains("hop=1")) =>
        {
            Answer::Page(page("/redirect/cookie"))
        }
        "cookie" => Answer::Page(
            Page::text(302, "")
                .with("location", "/redirect/cookie")
                .with("set-cookie", "hop=1; Path=/; Secure; SameSite=Lax"),
        ),
        // A listed redirect with no `Content-Type` and no body: nothing to
        // type (ADR 0119).
        "301/untyped" => Answer::Page(Page::untyped(301, "").with("location", "/target")),
        "301" | "302" | "303" | "307" | "308" => to(
            rest.parse().expect("invariant: the status is numeric"),
            "/target",
        ),
        "300" => to(300, "/target"),
        "no-location" => Answer::Page(Page::text(302, "")),
        "ftp" => to(302, "ftp://fetch-edge.test/target"),
        "file" => to(302, "file:///etc/passwd"),
        "gopher" => to(302, "gopher://fetch-edge.test/1"),
        "data" => to(302, "data:text/html,hi"),
        "cross-origin" => to(302, &format!("https://{HTTP1_HOST}{port}/target")),
        // The Proving ground's own loopback address, dotted and in hex.
        "address" => to(302, &format!("http://127.0.0.1{port}/address/target")),
        "address/hex" => to(302, &format!("http://0x7f.0.0.1{port}/address/target")),
        // A name every resolver answers with a loopback address.
        "address/name" => to(302, &format!("http://localhost{port}/address/target")),
        // `bücher.test` in punycode.
        "punycode" => to(302, &format!("http://xn--bcher-kva.test{port}/target")),
        "robots" => to(302, "/private/target"),
        hops => match hops
            .strip_prefix("hops/")
            .and_then(|n| n.parse::<u32>().ok())
        {
            Some(0) => Answer::Page(page("/redirect/hops/0")),
            Some(left) => to(302, &format!("/redirect/hops/{}", left - 1)),
            None => not_found(),
        },
    }
}

/// `/refresh/<rest>`: refresh stubs to `/target`, or that reload.
fn refresh(rest: &str) -> Option<Page> {
    let stub = |head: &str, body: &str| {
        Page::html(
            200,
            format!(
                "<!doctype html><html><head><meta charset=\"utf-8\"><title>Refresh</title>{head}</head><body>{body}</body></html>"
            ),
        )
    };
    let meta = |content: &str| format!("<meta http-equiv=\"refresh\" content=\"{content}\">");
    Some(match rest {
        "header" => stub("", "<p>Moved.</p>").with("refresh", "0; url=/target"),
        "meta/0" => stub(&meta("0; url=/target"), "<p>Moved.</p>"),
        "meta/5" => stub(&meta("5; url=/target"), "<p>Moved.</p>"),
        "meta/200" => stub(&meta("200; url=/target"), "<p>Moved.</p>"),
        "noscript" => stub(
            "",
            &format!(
                "<noscript>{}</noscript><p>Moved.</p>",
                meta("0; url=/target")
            ),
        ),
        "no-url" => stub(&meta("0"), "<p>Reloads.</p>"),
        "self" => stub(&meta("0; url=/refresh/self"), "<p>Reloads.</p>"),
        "script" => stub(
            "",
            "<p>Moved.</p><script>location.replace(\"/target\")</script>",
        ),
        _ => return None,
    })
}

/// `/retry-after/<status>/<form>`: on its first request, the status with a
/// `Retry-After` of 2 s, in seconds or as an HTTP-date in one of RFC 9110's
/// three formats (§5.6.7) against a `Date` an hour behind; a page after.
/// `/long` after the form asks for an hour instead, so a client that did
/// not wait is never mistaken for a slow one.
/// `/retry-after/503/long` and `/retry-after/429/long` always answer their
/// status and ask for an hour.
fn retry_after(rest: &str, first: bool, path: &str) -> Answer {
    let Some((status, form)) = rest.split_once('/') else {
        return not_found();
    };
    let Ok(status @ (429 | 503 | 302)) = status.parse::<u16>() else {
        return not_found();
    };
    if form == "long" {
        return Answer::Page(
            Page::text(status, "unavailable").with("retry-after", LONG_RETRY_SECONDS.to_string()),
        );
    }
    let (form, wait) = form
        .strip_suffix("/long")
        .map_or((form, RETRY_SECONDS), |form| (form, LONG_RETRY_SECONDS));
    let now = clock::unix_seconds();
    let skewed = now.saturating_sub(SKEW_SECONDS);
    let (date, value) = match form {
        "seconds" => (http_date(now, Form::ImfFixdate), wait.to_string()),
        "imf-fixdate" | "rfc850" | "asctime" => {
            let form = match form {
                "imf-fixdate" => Form::ImfFixdate,
                "rfc850" => Form::Rfc850,
                _ => Form::Asctime,
            };
            (
                http_date(skewed, Form::ImfFixdate),
                http_date(skewed + wait, form),
            )
        }
        _ => return not_found(),
    };
    if !first {
        return Answer::Page(page(path));
    }
    let mut answer = Page::text(status, "try again later")
        .with("date", date)
        .with("retry-after", value);
    if status == 302 {
        answer = answer.with("location", "/target");
    }
    Answer::Page(answer)
}

/// `/cut/<rest>`: a body broken off halfway; under `/cut/late/`, by a
/// connection closed a while after that half, once a client reading the
/// body as it arrives read it (#1732).
fn cut(rest: &str, path: &str) -> Answer {
    if let Some(late) = rest.strip_prefix("late/") {
        return match cut(late, path) {
            Answer::Cut {
                page,
                sent,
                cut: Cut::Connection,
            } => Answer::Cut {
                page,
                sent,
                cut: Cut::Late,
            },
            _ => not_found(),
        };
    }
    let whole = page(path);
    let half = whole.body.len() / 2;
    match rest {
        // Over HTTP/1.1, a body ending before its `Content-Length`; over
        // HTTP/2, a stream ended by its connection.
        "content-length" | "connection" => Answer::Cut {
            page: whole,
            sent: half,
            cut: Cut::Connection,
        },
        "chunked" => Answer::Cut {
            page: whole.framed(Framing::Chunked),
            sent: half,
            cut: Cut::Connection,
        },
        "reset" => Answer::Cut {
            page: whole,
            sent: half,
            cut: Cut::Stream,
        },
        "gzip" => {
            let mut body = gzip(&whole.body);
            body.truncate(body.len() / 2);
            Answer::Page(Page::html(200, body).with("content-encoding", "gzip"))
        }
        "gzip-content-length" => {
            let body = gzip(&whole.body);
            let sent = body.len() / 2;
            Answer::Cut {
                page: Page::html(200, body).with("content-encoding", "gzip"),
                sent,
                cut: Cut::Connection,
            }
        }
        _ => not_found(),
    }
}

/// `body` compressed whole with gzip.
fn gzip(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = std::io::Write::write_all(&mut encoder, body);
    encoder
        .finish()
        .expect("invariant: writing to a Vec succeeds")
}

/// The three HTTP-date formats of RFC 9110, section 5.6.7.
#[derive(Clone, Copy)]
enum Form {
    ImfFixdate,
    Rfc850,
    Asctime,
}

/// `seconds` since the Unix epoch as an HTTP-date in `form`.
fn http_date(seconds: u64, form: Form) -> String {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const LONG_DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).unwrap_or(0))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let day = usize::from(date.weekday().number_days_from_monday());
    let month = MONTHS[usize::from(u8::from(date.month())) - 1];
    let (hour, minute, second) = (date.hour(), date.minute(), date.second());
    match form {
        Form::ImfFixdate => format!(
            "{}, {:02} {month} {} {hour:02}:{minute:02}:{second:02} GMT",
            DAYS[day],
            date.day(),
            date.year()
        ),
        Form::Rfc850 => format!(
            "{}, {:02}-{month}-{:02} {hour:02}:{minute:02}:{second:02} GMT",
            LONG_DAYS[day],
            date.day(),
            date.year() % 100
        ),
        Form::Asctime => format!(
            "{} {month} {:>2} {hour:02}:{minute:02}:{second:02} {}",
            DAYS[day],
            date.day(),
            date.year()
        ),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// RFC 9110, section 5.6.7's example, `784111777`.
    #[rstest]
    #[case(Form::ImfFixdate, "Sun, 06 Nov 1994 08:49:37 GMT")]
    #[case(Form::Rfc850, "Sunday, 06-Nov-94 08:49:37 GMT")]
    #[case(Form::Asctime, "Sun Nov  6 08:49:37 1994")]
    fn http_dates_read_as_rfc_9110_writes_them(#[case] form: Form, #[case] text: &str) {
        assert_eq!(http_date(784_111_777, form), text);
    }
}
