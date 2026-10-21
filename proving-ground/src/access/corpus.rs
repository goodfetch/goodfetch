//! The block-detection corpus (ADR 0035): hand-written pages that answer as a
//! site's bot protection does when it refuses a page, one per Block cause,
//! and one per block-detection rule seeded from an outside source, beside
//! pages a rule must not take for a block, such as one that only loads a
//! vendor's script. Each page's Block cause is written by hand. The
//! pages are written from the signals rules key on, never copied from a
//! vendor's own pages.

use std::fmt::Write as _;

use hyper::StatusCode;

use crate::exchange::Reply;

/// One corpus page: its path, its Block cause (`none` when it is no block),
/// its status, the headers it adds and its body.
struct Page {
    path: &'static str,
    cause: &'static str,
    status: StatusCode,
    headers: &'static [(&'static str, &'static str)],
    body: &'static str,
}

/// The corpus, in the index's order.
const PAGES: [Page; 13] = [
    Page {
        path: "/corpus/interstitial",
        cause: "challenge",
        status: StatusCode::FORBIDDEN,
        headers: &[("cf-mitigated", "challenge"), ("cache-control", "no-store")],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Just a moment...</title><meta name=\"robots\" content=\"noindex,nofollow\"></head>\n<body>\n<main id=\"challenge-body\">\n<h1>Checking if the site connection is secure</h1>\n<p>This page needs JavaScript and cookies to continue.</p>\n</main>\n<script>window._cf_chl_opt = { cType: \"managed\", cRay: \"0000000000000000\" };</script>\n<script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\"></script>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/captcha",
        cause: "captcha",
        status: StatusCode::OK,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Verify you are a human</title></head>\n<body>\n<main>\n<h1>Please verify you are a human to continue</h1>\n<form method=\"post\" action=\"/corpus/captcha\">\n<div class=\"g-recaptcha\" data-sitekey=\"6Lc0000000000000000000000000000000000000\"></div>\n<button type=\"submit\">Continue</button>\n</form>\n</main>\n<script src=\"https://www.google.com/recaptcha/api.js\" async defer></script>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/rate-limit",
        cause: "rate_limit",
        status: StatusCode::TOO_MANY_REQUESTS,
        headers: &[("retry-after", "120")],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Too many requests</title></head>\n<body>\n<h1>Too many requests</h1>\n<p>You sent too many requests. Try again in two minutes.</p>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/ip-block",
        cause: "ip_block",
        status: StatusCode::FORBIDDEN,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Access denied</title></head>\n<body>\n<h1>Access denied</h1>\n<p>Your IP address has been blocked from this site. Reference: 0000-0000.</p>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/press-and-hold",
        cause: "captcha",
        status: StatusCode::FORBIDDEN,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Access to this page has been denied</title></head>\n<body>\n<main>\n<h1>Access to this page has been denied</h1>\n<p>Press and hold the button to confirm you are a person.</p>\n<div id=\"px-captcha\"></div>\n</main>\n<script src=\"/corpus/captcha/captcha.js?a=c&amp;m=0\"></script>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/cloudflare-block",
        cause: "ip_block",
        status: StatusCode::FORBIDDEN,
        headers: &[("cache-control", "no-store")],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Attention Required! | Cloudflare</title></head>\n<body>\n<main id=\"cf-wrapper\">\n<h1>Sorry, you have been blocked</h1>\n<p>You are unable to access this site. The site's security rules refused the request.</p>\n<p>Ray ID: 0000000000000000</p>\n</main>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/incapsula-block",
        cause: "ip_block",
        status: StatusCode::FORBIDDEN,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Request unsuccessful</title></head>\n<body>\n<p>Request unsuccessful. Incapsula incident ID: 000000000000000000-0000000000000000</p>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/sucuri-block",
        cause: "ip_block",
        status: StatusCode::FORBIDDEN,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Sucuri WebSite Firewall - Access Denied</title></head>\n<body>\n<main>\n<h1>Access Denied - Sucuri Website Firewall</h1>\n<p>Your request was blocked by the site's firewall.</p>\n</main>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/labyrinth",
        cause: "decoy",
        status: StatusCode::OK,
        headers: &[("x-robots-tag", "noindex")],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><meta name=\"robots\" content=\"noindex, nofollow\"><title>Further reading</title></head>\n<body>\n<main>\n<h1>Further reading</h1>\n<p>An article of generated text that leads to more of the same.</p>\n<a href=\"/corpus/labyrinth\" rel=\"nofollow\" style=\"display:none\">Further reading</a>\n</main>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/payment",
        cause: "payment_required",
        status: StatusCode::PAYMENT_REQUIRED,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Payment required</title></head>\n<body>\n<h1>Payment required</h1>\n<p>This page is available to crawlers for a fee.</p>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/vendor-script",
        cause: "none",
        status: StatusCode::OK,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Opening hours</title><script src=\"/cdn-cgi/challenge-platform/scripts/jsd/main.js\" async></script></head>\n<body>\n<main>\n<h1>Opening hours</h1>\n<p>Open every day from nine to five. This page loads a bot protection's script and is no block.</p>\n</main>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/not-found",
        cause: "none",
        status: StatusCode::NOT_FOUND,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Not found</title></head>\n<body>\n<h1>Not found</h1>\n<p>No page has this address.</p>\n</body>\n</html>\n",
    },
    Page {
        path: "/corpus/article",
        cause: "none",
        status: StatusCode::OK,
        headers: &[],
        body: "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>A walk by the river</title></head>\n<body>\n<main>\n<h1>A walk by the river</h1>\n<p>An ordinary article, with nothing between the reader and the text.</p>\n</main>\n</body>\n</html>\n",
    },
];

/// The corpus's index, which links every page.
pub(crate) const INDEX: &str = "/corpus";

/// Each corpus page's path and Block cause, in the index's order.
pub(crate) fn causes() -> impl Iterator<Item = (&'static str, &'static str)> {
    PAGES.iter().map(|page| (page.path, page.cause))
}

/// The corpus page at `path`, or its index; `None` for another path.
pub(crate) fn page(path: &str) -> Option<Reply> {
    if path == INDEX {
        let links = PAGES.iter().fold(String::new(), |mut links, page| {
            writeln!(links, "<li><a href=\"{0}\">{0}</a></li>", page.path)
                .expect("invariant: writing to a String succeeds");
            links
        });
        return Some(Reply::html(format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Block-detection corpus</title></head>\n<body>\n<main>\n<h1>Block-detection corpus</h1>\n<ul>\n{links}</ul>\n</main>\n</body>\n</html>\n"
        )));
    }
    let page = PAGES.iter().find(|page| page.path == path)?;
    let mut reply = Reply::html_with_status(page.status, page.body.to_owned());
    reply.headers.extend(
        page.headers
            .iter()
            .map(|(name, value)| (*name, (*value).to_owned())),
    );
    Some(reply)
}
