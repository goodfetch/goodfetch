//! The `clean-up` pages on `clean-up.test` (ADR 0047; spec #577's Clean-up
//! conformance cases): an article with ads and trackers, a challenge page
//! whose anti-bot sensors a tracker list blocks, a page that relies on a
//! redirect stand-in, and the pages of live hiding and extraction-side
//! removal. Each subresource is served from the site's own host
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
//!   keeps loading, Cloudflare's challenge platform script and logo, Stripe's
//!   fraud-signals script (`/6` of [`PAYMENT_HOST`], another site's, which
//!   EasyPrivacy's `||m.stripe.com^$third-party` blocks and the never-block
//!   list keeps loading as a payment script), and the same tracker as
//!   `/ads`, which stays blocked.
//! - `/stand-in`: loads Chartbeat's `https://static.chartbeat.com/js/chartbeat.js`,
//!   which EasyPrivacy redirects to its stand-in, and calls the
//!   `pSUPERFLY.virtualPage` it defines; then, before its `load`, shows
//!   the image `/stand-in/worked.gif`, or `/stand-in/broken.gif` when the
//!   call threw.
//! - `/hiding`: an article whose ad (EasyList's `##.ad-slot`) holds a link and a
//!   button, with its own style sheet and a frame, `/hiding/frame`, with an
//!   ad of its own; each page's script reports, once its ad is hidden or
//!   2 s after `load`, what the site sees: `/hiding/seen` (the frame's
//!   `/hiding/frame-seen`) with its `document.styleSheets` count and the
//!   ad's computed `display`. The button's click requests
//!   `/hiding/clicked`.
//! - `/shadow`: ads in its light DOM, in an open shadow root it declares
//!   and in a closed one its script attaches.
//! - `/removal`: an article most of whose text sits in an `.ad-slot`
//!   element.
//! - `/needs-script`: an article whose text its own script
//!   (`/js/ads.bundle.js`, EasyList's `/ads.bundle.js$script`) renders, so
//!   a blocker that blocks the script leaves a page without its
//!   `#content`.
//! - `/listed-challenge`: a challenge, answered `403`, whose check script
//!   (`/js/fingerprint2.min.js`, EasyPrivacy's `/fingerprint2.min.js`) no
//!   never-block entry keeps: once it runs, it clears the challenge in
//!   place, putting the page behind it, with its `#content`, in the
//!   challenge's stead.
//! - `/cross-site`: an article that frames `/framed` of another site,
//!   `clean-up-frame.test`, which the browser loads in a process of its own:
//!   the frame's own script and photo, which load, and the same ad and
//!   tracker as `/ads`, which a blocker blocks.

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name the pages are served on.
pub(crate) const HOST: &str = "clean-up.test";

/// The host name of `/cross-site`'s frame, another site.
pub(crate) const FRAME_HOST: &str = "clean-up-frame.test";

/// The host of Stripe's fraud-signals script, a payment script `/challenge`
/// loads from it over HTTPS as a checkout page would: its own name, so the
/// Filter lists and the never-block list judge the request as they judge
/// Stripe's. Browsers reach `stripe.com`'s hosts only over HTTPS (it is
/// HSTS-preloaded), so the server's leaf of its seeded authority answers it.
pub(crate) const PAYMENT_HOST: &str = "m.stripe.com";

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
<script>
const payment = document.createElement(\"script\");
payment.src = `https://m.stripe.com:${location.port}/6`;
document.head.append(payment);
</script>
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

/// `/hiding`: an article whose ad (EasyList's `##.ad-slot`) holds a link and a button,
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
<div class=\"ad-slot\" id=\"ad\">An ad's text no extraction keeps, for <a href=\"/hiding/offer\">the offer</a>. <button id=\"inside\" type=\"button\" onclick=\"fetch('/hiding/clicked')\">Continue to the offer</button></div>
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

/// `/needs-script`: an article whose text its own script renders.
const NEEDS_SCRIPT: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article its own ad script renders</title>
</head>
<body>
<main>
<h1>An article its own ad script renders</h1>
<script src=\"/js/ads.bundle.js\"></script>
</main>
</body>
</html>
";

/// `/js/ads.bundle.js`: renders `/needs-script`'s text after itself.
const RENDERING_SCRIPT: &[u8] = b"document.currentScript.insertAdjacentHTML(\"afterend\", \"<p id='content'>The article's text, which the site's own script renders.</p>\");\n";

/// `/listed-challenge`: an interstitial challenge whose check script a
/// Filter list blocks.
const LISTED_CHALLENGE: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>Just a moment...</title>
</head>
<body>
<main id=\"challenge\">
<h1>Checking your browser</h1>
<p>Enable JavaScript and cookies to continue.</p>
</main>
<script src=\"/js/fingerprint2.min.js\"></script>
</body>
</html>
";

/// `/js/fingerprint2.min.js`: clears `/listed-challenge` in place.
const CLEARING_SCRIPT: &[u8] = b"document.title = \"The page behind the challenge\";\ndocument.getElementById(\"challenge\").outerHTML = \"<main id='content'><h1>The page behind the challenge</h1><p>Its text, shown once the challenge cleared.</p></main>\";\n";

/// `/cross-site`: an article framing `/framed` of [`FRAME_HOST`], on the
/// port it was served from.
const CROSS_SITE: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>An article with another site's frame</title>
</head>
<body>
<main>
<h1>An article with another site's frame</h1>
<p>The article's text, above another site's frame.</p>
<iframe title=\"Another site's frame\" width=\"300\" height=\"100\"></iframe>
<script>
document.querySelector(\"iframe\").src = `http://clean-up-frame.test:${location.port}/framed`;
</script>
</main>
</body>
</html>
";

/// `/framed` of [`FRAME_HOST`]: its own script and photo, an ad and a
/// tracker.
const FRAMED: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>Another site's frame</title>
<script src=\"/js/site.js\"></script>
<script src=\"/js/adobe-analytics.js\"></script>
</head>
<body>
<img src=\"/img/photo.gif\" alt=\"The frame's photo\" width=\"1\" height=\"1\">
<img src=\"/img/top-ad-sidebar.gif\" alt=\"\" width=\"1\" height=\"1\">
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
    if name != HOST && name != FRAME_HOST && name != PAYMENT_HOST {
        return None;
    }
    if request.method != Method::GET {
        return Some(Reply::not_found());
    }
    let path = request.path.split('?').next().unwrap_or_default();
    if name == PAYMENT_HOST {
        return Some(match path {
            "/6" => asset("application/javascript", b"void 0;\n"),
            _ => Reply::not_found(),
        });
    }
    if name == FRAME_HOST && path == "/framed" {
        return Some(Reply::html(FRAMED.to_owned()));
    }
    Some(match path {
        "/cross-site" => Reply::html(CROSS_SITE.to_owned()),
        "/ads" => Reply::html(ADS.to_owned()),
        "/challenge" => Reply::html(CHALLENGE.to_owned()),
        "/stand-in" => Reply::html(STAND_IN.to_owned()),
        "/hiding" => Reply::html(reporting(HIDING, "/hiding/seen")),
        "/hiding/frame" => Reply::html(reporting(HIDING_FRAME, "/hiding/frame-seen")),
        "/hiding/seen" | "/hiding/frame-seen" | "/hiding/clicked" => asset("text/plain", b"seen\n"),
        "/removal" => Reply::html(REMOVAL.to_owned()),
        "/shadow" => Reply::html(SHADOW.to_owned()),
        "/needs-script" => Reply::html(NEEDS_SCRIPT.to_owned()),
        "/js/ads.bundle.js" => asset("application/javascript", RENDERING_SCRIPT),
        "/listed-challenge" => {
            Reply::html_with_status(StatusCode::FORBIDDEN, LISTED_CHALLENGE.to_owned())
        }
        "/js/fingerprint2.min.js" => asset("application/javascript", CLEARING_SCRIPT),
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
