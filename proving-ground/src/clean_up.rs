//! The `clean-up` pages on `clean-up.test` (ADR 0047; spec #577's Clean-up
//! conformance cases): an article with ads and trackers, a challenge page
//! whose anti-bot sensors a tracker list blocks, and a page that relies on
//! a redirect stand-in. Each subresource is served from the site's own host
//! under a path the embedded Filter lists judge, so the record of the
//! requests the Proving ground received is the set a client loaded: what a
//! blocker blocks never arrives. No task: a client's conformance case reads
//! that record (`/observations`).
//!
//! - `/ads`: its own style sheet, script and photo, which load; an ad image
//!   (`/img/top-ad-sidebar.gif`, EasyList's `-ad-sidebar.$image`) and a
//!   tracker (`/js/adobe-analytics.js`, EasyPrivacy's `/adobe-analytics.js`)
//!   and an annoyance (`/js/copy_text.js`, Fanboy Annoyance's
//!   `/copy_text.js`), which a blocker blocks.
//! - `/challenge`: Akamai's (`/akam/13/…`) and HUMAN's (`/perimeterx/…`)
//!   first-party sensors, which EasyPrivacy blocks and the never-block list
//!   keeps loading, Cloudflare's challenge platform script and logo, and the
//!   same tracker as `/ads`, which stays blocked.
//! - `/stand-in`: loads Chartbeat's `https://static.chartbeat.com/js/chartbeat.js`,
//!   which EasyPrivacy redirects to its stand-in, and calls the
//!   `pSUPERFLY.virtualPage` it defines; then, before its `load`, shows
//!   the image `/stand-in/worked.gif`, or `/stand-in/broken.gif` when the
//!   call threw.

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name the pages are served on.
pub(crate) const HOST: &str = "clean-up.test";

/// A transparent 1×1 GIF, every image's body.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x01, 0x44, 0x00, 0x3b,
];

const ADS: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article with ads and trackers</title>
<link rel=\"stylesheet\" href=\"/css/site.css\">
<script src=\"/js/site.js\"></script>
<script src=\"/js/adobe-analytics.js\"></script>
<script src=\"/js/copy_text.js\"></script>
</head>
<body>
<main>
<h1>An article with ads and trackers</h1>
<img src=\"/img/photo.gif\" alt=\"The article's photo\" width=\"1\" height=\"1\">
<p>The article's text, between an ad and a tracker.</p>
<aside><img src=\"/img/top-ad-sidebar.gif\" alt=\"\" width=\"1\" height=\"1\"></aside>
</main>
</body>
</html>
";

const CHALLENGE: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>A challenge</title>
<script src=\"/akam/13/sensor.js\"></script>
<script src=\"/perimeterx/px.js\"></script>
<script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\"></script>
<script src=\"/js/adobe-analytics.js\"></script>
</head>
<body>
<main>
<h1>Checking your browser</h1>
<img src=\"/cdn-cgi/challenge-platform/h/b/img/logo.gif\" alt=\"The challenge's logo\" width=\"1\" height=\"1\">
</main>
</body>
</html>
";

const STAND_IN: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>A page that calls a blocked tracker</title>
<script src=\"https://static.chartbeat.com/js/chartbeat.js\"></script>
</head>
<body>
<main>
<h1>A page that calls a blocked tracker</h1>
<script>
let outcome = \"worked\";
try {
  pSUPERFLY.virtualPage(\"/stand-in\");
} catch {
  outcome = \"broken\";
}
document.write(`<img src=\"/stand-in/${outcome}.gif\" alt=\"\" width=\"1\" height=\"1\">`);
</script>
</main>
</body>
</html>
";

/// Answers a request to the pages' host; `None` for any other host.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the pages name their own subresources, all in lowercase"
)]
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if name != HOST {
        return None;
    }
    if request.method != Method::GET {
        return Some(Reply::not_found());
    }
    let path = request.path.split('?').next().unwrap_or_default();
    Some(match path {
        "/ads" => Reply::html(ADS.to_owned()),
        "/challenge" => Reply::html(CHALLENGE.to_owned()),
        "/stand-in" => Reply::html(STAND_IN.to_owned()),
        path if path.ends_with(".gif") => asset("image/gif", GIF),
        path if path.ends_with(".css") => asset("text/css", b"main { margin: 1em; }\n"),
        path if path.ends_with(".js") || path.starts_with("/cdn-cgi/") => {
            asset("application/javascript", b"void 0;\n")
        }
        _ => Reply::not_found(),
    })
}

/// A subresource: `body`, of type `content_type`.
fn asset(content_type: &'static str, body: &'static [u8]) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type,
        headers: Vec::new(),
        body: body.into(),
    }
}
