//! The `page-load` pages on `page-load.test` (issue 2319): the page whose
//! load `xtask compare`'s Collie scenario times, in two forms. No task: the
//! comparison reads what the Proving ground received (`/observations`).
//!
//! - `/plain`: 150 subresources on the page's own host, 40 scripts, 40
//!   style sheets, 60 images and 10 fonts, under `/assets/`, none of which
//!   the embedded Filter lists block.
//! - `/ads`: the same page with 30 of them, 12 scripts and 18 images, on
//!   six ad hosts ([`AD_HOSTS`]), two scripts and three images each, which
//!   EasyList blocks by host (`||adnxs.com^` and the like). Its own
//!   subresources are the other 120.
//!
//! Both pages hold the same `#content`. Every subresource answers a small
//! body of its type; any other path is `404`.

use std::fmt::Write as _;

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};
use crate::task::origin;

/// The host name the pages are served on.
pub(crate) const HOST: &str = "page-load.test";

/// The hosts of `/ads`' ad subresources, each one EasyList blocks by its
/// domain. None is HSTS-preloaded, so a browser loads them over plain HTTP
/// as the Proving ground serves them.
pub(crate) const AD_HOSTS: [&str; 6] = [
    "ib.adnxs.com",
    "ads.pubmatic.com",
    "fastlane.rubiconproject.com",
    "u.openx.net",
    "js-sec.casalemedia.com",
    "z.moatads.com",
];

const SCRIPTS: u32 = 40;
const STYLES: u32 = 40;
const IMAGES: u32 = 60;
const FONTS: u32 = 10;
/// The scripts and images `/ads` loads from each ad host.
const AD_SCRIPTS: u32 = 2;
const AD_IMAGES: u32 = 3;

/// A transparent 1×1 GIF, every image's body.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x01, 0x44, 0x00, 0x3b,
];

/// The text both pages hold in `#content`.
const CONTENT: &str = "<main id=\"content\"><h1>A page of many parts</h1>\
<p>This article loads its scripts, style sheets, images and fonts before its load event.</p>\
<p>Its text is the same on the page with ads and on the page without.</p></main>";

/// One page: `/ads` when `ads`, else `/plain`, its ad subresources on
/// `port` of each ad host.
fn page(ads: bool, port: u16) -> String {
    let (ad_scripts, ad_images) = if ads {
        let hosts = u32::try_from(AD_HOSTS.len()).expect("invariant: a handful of ad hosts");
        (AD_SCRIPTS * hosts, AD_IMAGES * hosts)
    } else {
        (0, 0)
    };
    let mut head = String::new();
    let mut body = String::new();
    for number in 1..=STYLES {
        writeln!(
            head,
            "<link rel=\"stylesheet\" href=\"/assets/style-{number}.css\">"
        )
        .expect("invariant: writing to a String succeeds");
    }
    head.push_str("<style>\n");
    for number in 1..=FONTS {
        writeln!(head, "@font-face {{ font-family: \"face-{number}\"; src: url(\"/assets/font-{number}.woff2\") format(\"woff2\"); }}").expect("invariant: writing to a String succeeds");
    }
    head.push_str("</style>\n");
    for number in 1..=FONTS {
        writeln!(
            body,
            "<p style=\"font-family: 'face-{number}'\">Face {number}</p>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    for number in 1..=SCRIPTS - ad_scripts {
        writeln!(body, "<script src=\"/assets/script-{number}.js\"></script>")
            .expect("invariant: writing to a String succeeds");
    }
    for number in 1..=IMAGES - ad_images {
        writeln!(
            body,
            "<img src=\"/assets/image-{number}.gif\" alt=\"\" width=\"1\" height=\"1\">"
        )
        .expect("invariant: writing to a String succeeds");
    }
    if ads {
        for host in AD_HOSTS {
            let origin = origin(host, Some(port));
            for number in 1..=AD_SCRIPTS {
                writeln!(body, "<script src=\"{origin}/ad/{number}.js\"></script>")
                    .expect("invariant: writing to a String succeeds");
            }
            for number in 1..=AD_IMAGES {
                writeln!(
                    body,
                    "<img src=\"{origin}/ad/{number}.gif\" alt=\"\" width=\"1\" height=\"1\">"
                )
                .expect("invariant: writing to a String succeeds");
            }
        }
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>A page of many parts</title>\n{head}</head>\n<body>\n{CONTENT}\n{body}</body>\n</html>\n"
    )
}

/// The reply to `request` on the host `name` of a Proving ground on
/// `port`, or none when `name` is none of the family's.
pub(crate) fn respond(port: u16, name: &str, request: &Request<'_>) -> Option<Reply> {
    let ad = AD_HOSTS.contains(&name);
    if name != HOST && !ad {
        return None;
    }
    if *request.method != Method::GET {
        return Some(Reply::not_found());
    }
    let path = request.path;
    if ad {
        return Some(asset_of(path.strip_prefix("/ad/")).unwrap_or_else(Reply::not_found));
    }
    Some(match path {
        "/plain" => Reply::html(page(false, port)),
        "/ads" => Reply::html(page(true, port)),
        _ => asset_of(path.strip_prefix("/assets/")).unwrap_or_else(Reply::not_found),
    })
}

/// The subresource named `name`, by its extension.
fn asset_of(name: Option<&str>) -> Option<Reply> {
    let name = name?;
    let (content_type, body): (&'static str, &'static [u8]) = match name.rsplit_once('.')?.1 {
        "js" => ("application/javascript", b"void 0;\n"),
        "css" => ("text/css", b"main { margin: 1em; }\n"),
        "gif" => ("image/gif", GIF),
        "woff2" => ("font/woff2", b"wOF2"),
        _ => return None,
    };
    Some(Reply {
        status: StatusCode::OK,
        content_type,
        headers: Vec::new(),
        body: body.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each subresource URL of `page`: every `src` and `href`, and each
    /// font's `url(…)`.
    fn subresources(page: &str) -> Vec<String> {
        let mut found = Vec::new();
        for marker in ["src=\"", "href=\"", "url(\""] {
            for (at, _) in page.match_indices(marker) {
                let rest = &page[at + marker.len()..];
                if let Some(end) = rest.find('"') {
                    found.push(rest[..end].to_owned());
                }
            }
        }
        found
    }

    #[test]
    fn the_plain_page_loads_150_subresources_from_its_own_host() {
        let page = page(false, 4000);
        let found = subresources(&page);
        assert_eq!(found.len(), 150);
        assert!(
            found.iter().all(|url| url.starts_with("/assets/")),
            "{found:?}"
        );
        let kind = |extension: &str| found.iter().filter(|url| url.ends_with(extension)).count();
        assert_eq!(
            (kind(".js"), kind(".css"), kind(".gif"), kind(".woff2")),
            (40, 40, 60, 10)
        );
        assert!(page.contains(CONTENT));
    }

    #[test]
    fn the_page_with_ads_moves_30_of_them_to_six_ad_hosts_on_its_port() {
        let page = page(true, 4000);
        let found = subresources(&page);
        assert_eq!(found.len(), 150);
        let own = found
            .iter()
            .filter(|url| url.starts_with("/assets/"))
            .count();
        assert_eq!(own, 120);
        for host in AD_HOSTS {
            let on_host = found
                .iter()
                .filter(|url| url.starts_with(&format!("http://{host}:4000/ad/")))
                .count();
            assert_eq!(on_host, 5, "{host}");
        }
        assert!(page.contains(CONTENT));
    }

    #[test]
    fn every_subresource_answers_and_any_other_path_is_not_found() {
        static EMPTY: hyper::body::Bytes = hyper::body::Bytes::new();
        let get = |name: &str, path: &str| {
            let request = Request {
                method: &Method::GET,
                path,
                query: "",
                cookies: "",
                authorization: "",
                headers: &hyper::HeaderMap::new(),
                body: &EMPTY,
                peer: std::net::IpAddr::from([127, 0, 0, 1]),
                tls: None,
            };
            respond(4000, name, &request).map(|reply| (reply.status, reply.content_type))
        };
        assert_eq!(
            get(HOST, "/assets/font-3.woff2"),
            Some((StatusCode::OK, "font/woff2"))
        );
        assert_eq!(
            get("u.openx.net", "/ad/2.gif"),
            Some((StatusCode::OK, "image/gif"))
        );
        assert_eq!(
            get(HOST, "/other").map(|(status, _)| status),
            Some(StatusCode::NOT_FOUND)
        );
        assert_eq!(get("other.test", "/plain"), None);
    }
}
