//! The font pages on `fonts.test` (ADR 0074): the Goodfetch Chromium's font
//! test reads, for an Identity profile, whether each font the profile lists
//! renders, and that no host font outside it does. No task, no seed: the
//! profile's families are the query.
//!
//! - `/?family=<family>&family=…`: a known text in each listed family, in
//!   the fallback (`#fallback`), and in each host font of [`HOST_FONTS`] the
//!   profile does not list. Each font is a `.font` element with its
//!   `data-family`, `data-listed` and `data-expected` result: `available`
//!   for a listed family, `unavailable` for any other. The text is set in
//!   the family, then each generic family in turn; a font is available when
//!   its text measures apart from the generic family's alone. Once
//!   measured, `#results` shows the fallback's size and each font's
//!   `listed`, `expected` and `observed` as JSON, and the root element has
//!   `data-fonts="done"`. A family name is ASCII letters, digits, spaces,
//!   `-`, `_` and `.`, at most 64 bytes, at most 64 of them; any other is a
//!   `400`.
//! - `/fonts.js`: the measuring script.

use std::fmt::Write as _;

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name of the pages.
pub(crate) const HOST: &str = "fonts.test";

/// Fonts a host commonly has on Linux, Windows or macOS, which a browser
/// presenting a profile that lists none of them renders none of.
const HOST_FONTS: [&str; 46] = [
    "DejaVu Sans",
    "DejaVu Serif",
    "DejaVu Sans Mono",
    "Liberation Sans",
    "Liberation Serif",
    "Liberation Mono",
    "Noto Sans",
    "Noto Serif",
    "Noto Color Emoji",
    "Ubuntu",
    "Cantarell",
    "FreeSans",
    "FreeSerif",
    "FreeMono",
    "Bitstream Vera Sans",
    "Droid Sans",
    "WenQuanYi Zen Hei",
    "IPAGothic",
    "Arial",
    "Times New Roman",
    "Courier New",
    "Segoe UI",
    "Calibri",
    "Cambria",
    "Consolas",
    "Tahoma",
    "Verdana",
    "Georgia",
    "Trebuchet MS",
    "Comic Sans MS",
    "Impact",
    "Lucida Console",
    "MS Gothic",
    "Microsoft YaHei",
    "Helvetica",
    "Helvetica Neue",
    "Menlo",
    "Monaco",
    "Avenir",
    "Avenir Next",
    "Geneva",
    "Lucida Grande",
    "Optima",
    "Futura",
    "PingFang SC",
    "Hiragino Sans",
];

/// The generic families each text is also set in, the fallbacks a missing
/// font leaves.
const GENERIC: [&str; 3] = ["monospace", "sans-serif", "serif"];

/// The known text.
const TEXT: &str = "mmmmmmmmmmlli 0123456789 WwQq";

/// The longest family name and the most families the page takes.
const NAME_LIMIT: usize = 64;
const FAMILY_LIMIT: usize = 64;

/// The measuring script.
const SCRIPT: &str = include_str!("../pages/fonts/fonts.js");

/// Answers a request to the font pages; `None` on another host.
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if name != HOST {
        return None;
    }
    Some(match (request.method, request.path) {
        (&Method::GET, "/") => match families(request.query) {
            Some(listed) => Reply::html(page(&listed)),
            None => Reply::error(StatusCode::BAD_REQUEST, "unsafe_family"),
        },
        (&Method::GET, "/fonts.js") => Reply {
            status: StatusCode::OK,
            content_type: "application/javascript",
            headers: Vec::new(),
            body: SCRIPT.as_bytes().into(),
        },
        _ => Reply::not_found(),
    })
}

/// The query's `family` values, each once; `None` when one is not a safe
/// family name or there are too many.
fn families(query: &str) -> Option<Vec<String>> {
    let mut listed = Vec::new();
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key != "family" {
            continue;
        }
        let safe = !value.is_empty()
            && value.len() <= NAME_LIMIT
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b" -_.".contains(&byte));
        if !safe {
            return None;
        }
        if !listed.contains(&value) {
            listed.push(value);
        }
    }
    (listed.len() <= FAMILY_LIMIT).then(|| listed.into_iter().map(String::from).collect())
}

/// The page for a profile that lists `listed`.
fn page(listed: &[String]) -> String {
    let mut fonts = String::new();
    let probed = HOST_FONTS
        .iter()
        .filter(|family| !listed.iter().any(|name| name == *family));
    let rows = listed
        .iter()
        .map(|family| (family.as_str(), true))
        .chain(probed.map(|family| (*family, false)));
    for (family, is_listed) in rows {
        let expected = if is_listed {
            "available"
        } else {
            "unavailable"
        };
        write!(
            fonts,
            "<p class=\"font\" data-family=\"{family}\" data-listed=\"{is_listed}\" data-expected=\"{expected}\">"
        )
        .expect("invariant: writing to a String succeeds");
        for generic in GENERIC {
            write!(
                fonts,
                "<span style=\"font-family: '{family}', {generic}\">{TEXT}</span>"
            )
            .expect("invariant: writing to a String succeeds");
        }
        fonts.push_str("</p>\n");
    }
    let mut fallback = String::new();
    for generic in GENERIC {
        write!(
            fallback,
            "<span style=\"font-family: {generic}\">{TEXT}</span>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Fonts</title>\n\
         <style>p span {{ display: inline-block; font-size: 48px; white-space: nowrap; margin-right: 1em; }}</style>\n\
         </head>\n<body>\n<h1>Fonts</h1>\n<p id=\"fallback\">{fallback}</p>\n{fonts}\
         <pre id=\"results\"></pre>\n<script src=\"/fonts.js\"></script>\n</body>\n</html>\n"
    )
}
