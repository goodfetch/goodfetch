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

/// What a live-hiding page's own script reports once its `load` fired and
/// its ad was hidden, or 2 s passed: how many style sheets the site sees
/// in `document.styleSheets` and the ad's computed `display`, as the query
/// of `<report>` (`/hiding/seen` or `/hiding/frame-seen`), which the
/// observations list.
const REPORT: &str = "<script>
addEventListener(\"load\", () => {
  const ad = document.getElementById(\"ad\");
  const started = performance.now();
  const look = () => {
    const display = getComputedStyle(ad).display;
    if (display !== \"none\" && performance.now() - started < 2000) {
      setTimeout(look, 50);
      return;
    }
    fetch(`REPORT?sheets=${document.styleSheets.length}&ad=${display}`);
  };
  look();
});
</script>";

/// `/hiding`: an article whose ad (EasyList's `##.ad-slot`) holds a button,
/// in a page with one style sheet of its own and a frame with an ad of its
/// own.
const HIDING: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article with an ad a blocker hides</title>
<link rel=\"stylesheet\" href=\"/css/site.css\">
</head>
<body>
<main>
<h1>An article with an ad a blocker hides</h1>
<p>The article's text begins here, long enough to be the page's main content: a person reads it for these paragraphs, which say what the article has to say about blockers, hidden elements and the pages they leave behind.</p>
<div class=\"ad-slot\" id=\"ad\">An ad's text no extraction keeps. <button id=\"inside\" type=\"button\" onclick=\"fetch('/hiding/clicked')\">Continue to the offer</button></div>
<p>The article goes on after the ad, with more of its own text: a blocker hides what a filter list matches, and a program that clicks inside it still reaches what it clicked.</p>
<iframe src=\"/hiding/frame\" title=\"A frame with an ad\" width=\"300\" height=\"100\"></iframe>
</main>
REPORT_SCRIPT
</body>
</html>
";

/// `/hiding/frame`: the frame of `/hiding`, with an ad of its own.
const HIDING_FRAME: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>A frame with an ad</title>
</head>
<body>
<div class=\"ad-slot\" id=\"ad\">A frame's ad</div>
REPORT_SCRIPT
</body>
</html>
";

/// `/removal`: an article most of whose text sits in an element a cosmetic
/// rule matches (EasyList's `##.ad-slot`), so that removing it takes more
/// than a fifth of the main content.
const REMOVAL: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article a rule takes most of</title>
</head>
<body>
<main>
<h1>An article a rule takes most of</h1>
<p>A short opening paragraph of the article's own.</p>
<div class=\"ad-slot\">
<p>Most of the article's text sits in this element, which a cosmetic rule of a Filter list matches as if it were an ad: the first of its paragraphs says so at some length, so that the removal takes a large share of the main content.</p>
<p>The second paragraph adds more of the same text, about blockers and the rules that remove real content, which the page's Provenance flags so that such a rule shows.</p>
</div>
</main>
</body>
</html>
";

/// `/shadow`: an article with an ad (EasyList's `##.ad-slot`) in its light
/// DOM, one in an open shadow root, declared, and one in a closed shadow
/// root its script attaches.
const SHADOW: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article with ads in shadow roots</title>
</head>
<body>
<main>
<h1>An article with ads in shadow roots</h1>
<p>The article's own text, which a person reads it for, beside ads in its light DOM and in its shadow roots, open and closed.</p>
<div class=\"ad-slot\">A light ad</div>
<div id=\"open-host\"><template shadowrootmode=\"open\"><p>Text in an open root.</p><div class=\"ad-slot\">An open-root ad</div></template></div>
<div id=\"closed-host\"></div>
<script>
document.getElementById(\"closed-host\").attachShadow({ mode: \"closed\" }).innerHTML =
  '<p>Text in a closed root.</p><div class=\"ad-slot\">A closed-root ad</div>';
</script>
</main>
</body>
</html>
";

/// A live-hiding page with its report sent to `report`.
fn reporting(page: &str, report: &str) -> String {
    page.replace("REPORT_SCRIPT", &REPORT.replace("REPORT", report))
}

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
        "/hiding" => Reply::html(reporting(HIDING, "/hiding/seen")),
        "/hiding/frame" => Reply::html(reporting(HIDING_FRAME, "/hiding/frame-seen")),
        "/hiding/seen" | "/hiding/frame-seen" | "/hiding/clicked" => asset("text/plain", b"seen\n"),
        "/removal" => Reply::html(REMOVAL.to_owned()),
        "/shadow" => Reply::html(SHADOW.to_owned()),
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
