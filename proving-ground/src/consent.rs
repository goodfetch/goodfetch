//! The consent pages on `consent.test` and `consent-frame.test` (ADR 0048;
//! spec #577's Clean-up conformance cases): Consent prompts modelled on
//! OneTrust, Cookiebot, Didomi, Usercentrics (in shadow DOM), Sourcepoint
//! (in a cross-origin iframe), Quantcast, TrustArc, Google's consent page
//! and Axeptio, on TikTok's banner (whose rule clicks with
//! `element.click()`) and on Moove's (whose rule runs a snippet), each
//! written from the selectors autoconsent's rules read, never copied from a
//! vendor's page; a prompt no rule knows in twelve languages, in the page
//! and in a cross-origin iframe; one that appears late; one that appears
//! on a route change; and a notice with no refusal. No task: every page
//! loads `/consent/common.js`, which reports each answer, whether its
//! click was trusted, every click the page's own `HTMLElement.prototype.click`
//! hook saw and every `autoconsent-*` element put in the page as a request
//! the Proving ground records (`GET /observations`), each named by its
//! `vendor` parameter.
//!
//! - `/vendor/<vendor>`: a vendor's prompt over an article; `onetrust`,
//!   `cookiebot` (refused through its own API, `Cookiebot.withdraw()`,
//!   reported as the answer `withdraw`, then closed by its `hide()`,
//!   reported to `/consent/closed`), `didomi`, `usercentrics`,
//!   `sourcepoint` (its message in `consent-frame.test`'s
//!   `/index.html?message_id=1`), `quantcast`, `trustarc`, `google`,
//!   `axeptio`, `tiktok` and `moove` (its settings saved, reported as the
//!   answer `save` with `performance`, whether its performance cookies
//!   stayed on, and `hooks`, how many times the page's own `checked` setter
//!   ran).
//! - `/heuristic/<language>`: a banner no rule knows, in `en`, `fr`, `de`,
//!   `nl`, `es`, `it`, `pl`, `ru`, `ja`, `ca`, `eu` or `gl`, with an accept
//!   and a refuse button; `/framed/<language>` shows
//!   `consent-frame.test`'s `/heuristic/<language>` in an iframe.
//! - `/late`: the English banner, shown 3 s after the page loaded.
//! - `/spa`: a single-page app that moves to `/spa/next` with
//!   `history.pushState` 1 s after it loaded, and shows Didomi's prompt
//!   there.
//! - `/notice`: a notice with only an acknowledgement, which reports
//!   `/consent/notice?visible=false` once it is no longer shown.
//! - `/wall`: a consent wall, a prompt with only an acceptance that covers
//!   the whole viewport, the article withheld behind it.
//! - `/pay-or-accept`: an "accept or pay" wall, a banner at the bottom of
//!   the viewport that leaves the article readable but offers a
//!   subscription instead of a refusal.

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name of the pages.
pub(crate) const HOST: &str = "consent.test";

/// The host name of the framed prompts, another site than [`HOST`], so
/// their frames are out of process.
pub(crate) const FRAME_HOST: &str = "consent-frame.test";

/// The script every page loads first.
const COMMON: &str = include_str!("../pages/consent/common.js");

/// The heuristic banner's text, accept button and refuse button in each
/// language, each matching autoconsent's detection and button patterns.
const LANGUAGES: [(&str, &str, &str, &str); 12] = [
    (
        "en",
        "We use cookies to give you the best experience on our website.",
        "Accept all",
        "Reject all",
    ),
    (
        "fr",
        "Nous utilisons des cookies pour améliorer votre expérience.",
        "Tout accepter",
        "Tout refuser",
    ),
    (
        "de",
        "Wir verwenden Cookies, um Ihnen das beste Erlebnis zu bieten.",
        "Alle akzeptieren",
        "Alle ablehnen",
    ),
    (
        "nl",
        "Wij gebruiken cookies om onze website te verbeteren.",
        "Alles accepteren",
        "Alles weigeren",
    ),
    (
        "es",
        "Utilizamos cookies propias y de terceros para mejorar su experiencia.",
        "Aceptar todo",
        "Rechazar todo",
    ),
    (
        "it",
        "Usiamo i cookie per migliorare la tua esperienza.",
        "Accetta tutto",
        "Rifiuta tutto",
    ),
    (
        "pl",
        "Używamy plików cookie, aby ulepszyć działanie strony.",
        "Akceptuj wszystkie",
        "Odrzuć wszystkie",
    ),
    (
        "ru",
        "Мы используем файлы cookie для улучшения работы сайта.",
        "Принять все",
        "Отклонить все",
    ),
    (
        "ja",
        "当サイトはCookieを使用しています。",
        "すべて同意",
        "すべて拒否",
    ),
    (
        "ca",
        "Utilitzem galetes per millorar la vostra experiència.",
        "Acceptar-les totes",
        "Rebutjar-les totes",
    ),
    (
        "eu",
        "Web orri honek cookieak erabiltzen ditu.",
        "Onartu guztiak",
        "Baztertu guztiak",
    ),
    (
        "gl",
        "Este portal emprega cookies propias ou de terceiros con fins analíticos.",
        "Aceptar todas",
        "Rexeitar todas",
    ),
];

/// The article every prompt covers.
const ARTICLE: &str = "<main><h1>An article behind a Consent prompt</h1>\
<p>The article's text, which a reader came for and a Consent prompt covers until it is answered.</p>\
<p>A second paragraph, so the page has content a prompt can leave readable.</p></main>";

/// The style of a banner at the bottom of the viewport.
const BANNER: &str = "position:fixed;left:0;right:0;bottom:0;padding:16px;background:#fff;border-top:1px solid #333;z-index:1000";

/// A page of `vendor` titled `title`, with `head` in its head after the
/// common script, and `body` after its article.
fn page(vendor: &str, title: &str, head: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n\
<script src=\"/consent/common.js\" data-vendor=\"{vendor}\"></script>\n{head}\n</head>\n<body>\n{ARTICLE}\n{body}\n</body>\n</html>\n"
    )
}

/// The page of `vendor`'s prompt, or `None` for a vendor the pages do not
/// model.
fn vendor(vendor: &str) -> Option<String> {
    let (head, body) = match vendor {
        "onetrust" => (
            "<script>window.OnetrustActiveGroups = \",C0001,\";</script>".to_owned(),
            format!(
                "<div id=\"onetrust-consent-sdk\"><div id=\"onetrust-banner-sdk\" style=\"{BANNER}\">\
<p id=\"onetrust-policy-text\">We use cookies to improve your experience and for marketing.</p>\
<button id=\"onetrust-accept-btn-handler\" data-answer=\"accept\" data-closes=\"#onetrust-consent-sdk\">Accept All Cookies</button>\
<button id=\"onetrust-reject-all-handler\" data-answer=\"reject\" data-closes=\"#onetrust-consent-sdk\">Reject All</button>\
</div></div>"
            ),
        ),
        "cookiebot" => (
            "<script>\
window.Cookiebot = { hasResponse: false, declined: false, dialog: { visible: true },\
 withdraw() { this.hasResponse = true; this.declined = true; window.consentBeacon('/consent/answered', { answer: 'withdraw', world: 'main' }); },\
 hide() { this.dialog.visible = false; document.getElementById('CybotCookiebotDialog')?.remove(); window.consentBeacon('/consent/closed', {}); } };\
</script>"
                .to_owned(),
            format!(
                "<div id=\"CybotCookiebotDialog\" style=\"{BANNER}\">\
<p>This website uses cookies to personalise content and ads.</p>\
<button id=\"CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll\" data-answer=\"accept\" data-closes=\"#CybotCookiebotDialog\">Allow all</button>\
</div>"
            ),
        ),
        "didomi" => (String::new(), didomi()),
        "quantcast" => (
            String::new(),
            format!(
                "<div id=\"qc-cmp2-container\"><div id=\"qc-cmp2-ui\" class=\"qc-cmp2-main\" style=\"{BANNER}\">\
<p>We value your privacy. We and our partners store and/or access information on a device, such as cookies.</p>\
<div class=\"qc-cmp2-summary-buttons\">\
<button id=\"disagree-btn\" mode=\"secondary\" data-answer=\"reject\" data-closes=\"#qc-cmp2-container\">DISAGREE</button>\
<button mode=\"primary\" data-answer=\"accept\" data-closes=\"#qc-cmp2-container\">AGREE</button>\
</div></div></div>"
            ),
        ),
        "trustarc" => (
            String::new(),
            format!(
                "<div id=\"truste-consent-track\" style=\"{BANNER}\"><div id=\"truste-consent-content\">\
<p>This site uses cookies and related technologies.</p>\
<button id=\"truste-consent-button\" data-answer=\"accept\" data-closes=\"#truste-consent-track\">Agree and Proceed</button>\
<button id=\"truste-consent-required\" data-answer=\"reject\" data-closes=\"#truste-consent-track\">Required Only</button>\
</div></div>"
            ),
        ),
        "google" => (
            String::new(),
            format!(
                "<div class=\"HTjtHe\" id=\"xe7COe\" style=\"{BANNER}\">\
<p>We use <a href=\"https://policies.google.com/technologies/cookies\">cookies and data</a> to deliver and maintain our services.</p>\
<button id=\"W0wltc\" data-answer=\"reject\" data-closes=\"#xe7COe\">Reject all</button>\
<button id=\"L2AGLb\" data-answer=\"accept\" data-closes=\"#xe7COe\">Accept all</button>\
</div>"
            ),
        ),
        "axeptio" => (
            String::new(),
            format!(
                "<div class=\"axeptio_mount\"><div class=\"axeptio_widget axeptio-widget--open\" style=\"{BANNER}\">\
<p>This website uses cookies.</p>\
<button id=\"axeptio_btn_acceptAll\" data-answer=\"accept\" data-closes=\".axeptio_mount\">OK for me</button>\
<button id=\"axeptio_btn_dismiss\" data-answer=\"reject\" data-closes=\".axeptio_mount\">No thanks</button>\
</div></div>"
            ),
        ),
        _ => scripted(vendor)?,
    };
    Some(page(
        vendor,
        &format!("A page with {vendor}'s prompt"),
        &head,
        &body,
    ))
}

/// The prompt of `vendor` whose page builds it or answers it with a script
/// of its own: in a shadow root, in a frame, or through a snippet.
fn scripted(vendor: &str) -> Option<(String, String)> {
    Some(match vendor {
        "usercentrics" => (
            "<script>window.UC_UI = { closeCMP() { return true; }, denyAllConsents() { return true; }, acceptAllConsents() { return true; }, areAllConsentsAccepted() { return false; } };</script>"
                .to_owned(),
            format!(
                "<div id=\"usercentrics-root\"></div>\
<script>\
const root = document.getElementById('usercentrics-root').attachShadow({{ mode: 'open' }});\
root.innerHTML = `<div data-testid=\"uc-container\" style=\"{BANNER}\"><p>We use cookies and other technologies.</p>\
<button data-testid=\"uc-accept-all-button\" data-answer=\"accept\" data-closes=\"#usercentrics-root\">Accept All</button>\
<button data-testid=\"uc-deny-all-button\" data-answer=\"reject\" data-closes=\"#usercentrics-root\">Deny</button></div>`;\
</script>"
            ),
        ),
        "sourcepoint" => (
            String::new(),
            format!(
                "<div id=\"sp_message_container_1\" style=\"{BANNER};height:160px\"><iframe title=\"Consent\" style=\"border:0;width:100%;height:150px\"></iframe></div>\
<script>\
document.querySelector('#sp_message_container_1 iframe').src = `http://{FRAME_HOST}:${{location.port}}/index.html?message_id=1`;\
document.documentElement.classList.add('sp-message-open');\
window.addEventListener('message', (event) => {{ if (event.data?.consent === 'closed') {{ document.getElementById('sp_message_container_1')?.remove(); document.documentElement.classList.remove('sp-message-open'); }} }});\
</script>"
            ),
        ),
        "tiktok" => (
            String::new(),
            format!(
                "<tiktok-cookie-banner></tiktok-cookie-banner>\
<script>\
const root = document.querySelector('tiktok-cookie-banner').attachShadow({{ mode: 'open' }});\
root.innerHTML = `<div class=\"tiktok-cookie-banner\" style=\"{BANNER}\"><p>We use cookies to enhance your experience.</p>\
<div class=\"button-wrapper\"><button data-answer=\"reject\" data-closes=\"tiktok-cookie-banner\">Decline optional cookies</button>\
<button data-answer=\"accept\" data-closes=\"tiktok-cookie-banner\">Accept all</button></div></div>`;\
</script>"
            ),
        ),
        "moove" => (
            "<script>\
window.checkedHooks = 0;\
const checked = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'checked');\
Object.defineProperty(HTMLInputElement.prototype, 'checked', { ...checked, set(value) { window.checkedHooks += 1; checked.set.call(this, value); } });\
</script>"
                .to_owned(),
            format!(
                "<div id=\"moove_gdpr_cookie_info_bar\" style=\"{BANNER}\"><p>We are using cookies to give you the best experience.</p>\
<button class=\"change-settings-button\" onclick=\"document.getElementById('moove_gdpr_cookie_modal').hidden = false\">Settings</button>\
<button class=\"moove-gdpr-infobar-allow-all\" data-answer=\"accept\" data-closes=\"#moove_gdpr_cookie_info_bar\">Accept</button></div>\
<div id=\"moove_gdpr_cookie_modal\" hidden style=\"position:fixed;top:20%;left:20%;width:60%;padding:16px;background:#fff;border:1px solid #333;z-index:1001\">\
<label><input type=\"checkbox\" name=\"moove_gdpr_strict_cookies\" checked> Strictly necessary</label>\
<label><input type=\"checkbox\" name=\"moove_gdpr_performance_cookies\" checked> Performance</label>\
<button class=\"moove-gdpr-modal-save-settings\">Save Changes</button></div>\
<script>\
document.querySelector('.moove-gdpr-modal-save-settings').addEventListener('click', (event) => {{\
 const performance = document.querySelector('[name=moove_gdpr_performance_cookies]').checked;\
 window.consentBeacon('/consent/answered', {{ answer: 'save', trusted: String(event.isTrusted), performance: String(performance), hooks: String(window.checkedHooks) }});\
 document.getElementById('moove_gdpr_cookie_info_bar').remove(); document.getElementById('moove_gdpr_cookie_modal').remove();\
}});\
</script>"
            ),
        ),
        _ => return None,
    })
}

/// Didomi's prompt.
fn didomi() -> String {
    format!(
        "<div id=\"didomi-host\"><div id=\"didomi-notice\" class=\"didomi-notice-banner\" style=\"{BANNER}\">\
<p>We and our partners use cookies and similar technologies.</p>\
<button id=\"didomi-notice-agree-button\" data-answer=\"accept\" data-closes=\"#didomi-host\">Agree and close</button>\
<button id=\"didomi-notice-disagree-button\" data-answer=\"reject\" data-closes=\"#didomi-host\">Disagree</button>\
</div></div>"
    )
}

/// The heuristic banner in `language`, named `vendor`; framed, it fills its
/// frame's body.
fn heuristic(vendor: &str, language: &str, framed: bool) -> Option<String> {
    let (_, text, accept, reject) = LANGUAGES.iter().find(|(code, ..)| *code == language)?;
    let banner = format!(
        "<div id=\"consent-banner\" style=\"{}\"><p>{text}</p>\
<button data-answer=\"accept\" data-closes=\"#consent-banner\">{accept}</button>\
<button data-answer=\"reject\" data-closes=\"#consent-banner\">{reject}</button></div>",
        if framed { "padding:8px" } else { BANNER }
    );
    if framed {
        return Some(format!(
            "<!DOCTYPE html>\n<html lang=\"{language}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{language}</title>\n\
<script src=\"/consent/common.js\" data-vendor=\"{vendor}\"></script>\n</head>\n<body>{banner}</body>\n</html>\n"
        ));
    }
    Some(page(
        vendor,
        &format!("A prompt no rule knows ({language})"),
        "",
        &banner,
    ))
}

/// A page showing the framed heuristic banner in `language`.
fn framed(language: &str) -> Option<String> {
    LANGUAGES.iter().find(|(code, ..)| *code == language)?;
    Some(page(
        &format!("framed-{language}"),
        &format!("A framed prompt no rule knows ({language})"),
        "",
        &format!(
            "<div id=\"consent-frame\" style=\"{BANNER};height:120px\"><iframe title=\"Consent\" style=\"border:0;width:100%;height:110px\"></iframe></div>\
<script>\
document.querySelector('#consent-frame iframe').src = `http://{FRAME_HOST}:${{location.port}}/heuristic/{language}`;\
window.addEventListener('message', (event) => {{ if (event.data?.consent === 'closed') document.getElementById('consent-frame')?.remove(); }});\
</script>"
        ),
    ))
}

/// Sourcepoint's message, in its frame.
fn sourcepoint_message() -> String {
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Privacy</title>\n\
<script src=\"/consent/common.js\" data-vendor=\"sourcepoint\"></script>\n</head>\n<body>\
<div class=\"message type-notice\"><p>We and our partners store and access information on your device.</p>\
<button class=\"message-component sp_choice_type_11\" data-answer=\"accept\">Accept</button>\
<button class=\"message-component sp_choice_type_13\" data-answer=\"reject\">Reject all</button>\
</div></body>\n</html>\n"
        .to_owned()
}

/// The late banner: the English one, shown 3 s after the page loaded.
fn late() -> String {
    let (_, text, accept, reject) = LANGUAGES[0];
    page(
        "late",
        "A prompt that appears late",
        "",
        &format!(
            "<script>\
setTimeout(() => {{ document.body.insertAdjacentHTML('beforeend', `<div id=\"consent-banner\" style=\"{BANNER}\"><p>{text}</p>\
<button data-answer=\"accept\" data-closes=\"#consent-banner\">{accept}</button>\
<button data-answer=\"reject\" data-closes=\"#consent-banner\">{reject}</button></div>`); }}, 3000);\
</script>"
        ),
    )
}

/// The single-page app, which shows Didomi's prompt on its next route.
fn spa() -> String {
    page(
        "spa",
        "A single-page app",
        "",
        &format!(
            "<script>\
setTimeout(() => {{ history.pushState({{}}, '', '/spa/next'); document.querySelector('h1').textContent = 'The next route'; document.body.insertAdjacentHTML('beforeend', `{}`); }}, 1000);\
</script>",
            didomi()
        ),
    )
}

/// The notice with no refusal.
fn notice() -> String {
    page(
        "notice",
        "A notice with no refusal",
        "",
        &format!(
            "<div id=\"cookie-notice\" style=\"{BANNER}\"><p>This website uses cookies to ensure you get the best experience. By continuing to browse you agree to our use of cookies.</p>\
<button>Got it</button></div>\
<script>\
const shown = setInterval(() => {{ const notice = document.getElementById('cookie-notice'); if (!notice.checkVisibility()) {{ clearInterval(shown); window.consentBeacon('/consent/notice', {{ visible: 'false', sheets: String(document.styleSheets.length) }}); }} }}, 200);\
</script>"
        ),
    )
}

/// The consent wall: a prompt with only an acceptance, over the whole
/// viewport.
fn wall() -> String {
    page(
        "wall",
        "A consent wall",
        "",
        "<div id=\"consent-wall\" style=\"position:fixed;inset:0;background:#fff;z-index:1000;padding:48px\" role=\"dialog\" aria-modal=\"true\">\
<p>We use cookies and similar technologies to personalise content and ads. To read this site, accept our use of cookies.</p>\
<button data-answer=\"accept\" data-closes=\"#consent-wall\">Accept all</button></div>",
    )
}

/// A consent wall a Filter list's cosmetic rule would hide: the same wall
/// as `/wall`, under Fanboy Annoyance's `###grow-consent-modal`.
fn hidden_wall() -> String {
    page(
        "hidden-wall",
        "A consent wall a Filter list would hide",
        "",
        "<div id=\"grow-consent-modal\" style=\"position:fixed;inset:0;background:#fff;z-index:1000;padding:48px\" role=\"dialog\" aria-modal=\"true\">\
<p>We use cookies and similar technologies to personalise content and ads. To read this site, accept our use of cookies.</p>\
<button data-answer=\"accept\" data-closes=\"#grow-consent-modal\">Accept all</button></div>\
<main><h1>The article behind the wall</h1><p>The article's text, which no one reads before the wall is answered.</p></main>",
    )
}

/// The "accept or pay" wall: a banner offering a subscription instead of a
/// refusal.
fn pay_or_accept() -> String {
    page(
        "pay-or-accept",
        "An accept or pay wall",
        "",
        &format!(
            "<div id=\"consent-pay\" style=\"{BANNER}\"><p>We use cookies to fund our journalism. Accept cookies, or subscribe for 2,99 € a month to read without ads and tracking.</p>\
<button data-answer=\"accept\" data-closes=\"#consent-pay\">Accept cookies</button>\
<button data-answer=\"subscribe\">Subscribe for 2,99 € a month</button></div>"
        ),
    )
}

/// Answers a request to the pages' hosts; `None` for any other host.
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if name != HOST && name != FRAME_HOST {
        return None;
    }
    if request.method != Method::GET {
        return Some(Reply::not_found());
    }
    let path = request.path.split('?').next().unwrap_or_default();
    let html = match (name == FRAME_HOST, path) {
        (_, "/consent/common.js") => {
            return Some(Reply {
                status: StatusCode::OK,
                content_type: "application/javascript",
                headers: Vec::new(),
                body: COMMON.as_bytes().into(),
            });
        }
        // The beacons: their record is the answer.
        (_, path) if path.starts_with("/consent/") => {
            return Some(Reply {
                status: StatusCode::NO_CONTENT,
                content_type: "text/plain",
                headers: Vec::new(),
                body: Vec::new().into(),
            });
        }
        (true, "/index.html") => Some(sourcepoint_message()),
        (framed, path) if path.starts_with("/heuristic/") => {
            let language = &path["/heuristic/".len()..];
            let kind = if framed { "framed" } else { "heuristic" };
            heuristic(&format!("{kind}-{language}"), language, framed)
        }
        (false, path) if path.starts_with("/vendor/") => vendor(&path["/vendor/".len()..]),
        (false, path) if path.starts_with("/framed/") => framed(&path["/framed/".len()..]),
        (false, "/late") => Some(late()),
        (false, "/spa" | "/spa/next") => Some(spa()),
        (false, "/notice") => Some(notice()),
        (false, "/wall") => Some(wall()),
        (false, "/hidden-wall") => Some(hidden_wall()),
        (false, "/pay-or-accept") => Some(pay_or_accept()),
        _ => None,
    };
    Some(html.map_or_else(Reply::not_found, Reply::html))
}
