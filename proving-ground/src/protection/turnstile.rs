//! The Turnstile page: an article behind a widget marked as Cloudflare
//! Turnstile's is, which takes only a token minted for its own site key and
//! page. No Cloudflare host answers for it: the solver-service stand-ins
//! (`solvers`) mint the tokens, as the real services answer real ones.

use sha2::{Digest as _, Sha256};

use super::{FAMILY_PLACE, SITE_KEY_PLACE, TURNSTILE_COOKIE_PLACE};
use crate::generator;

/// The page's path, which also takes its form.
pub(super) const PATH: &str = "/turnstile";

/// The form field the widget fills with its token.
pub(super) const FIELD: &str = "cf-turnstile-response";

/// The cookie that carries an accepted token.
pub(super) const COOKIE: &str = "turnstile";

/// The widget's script: it holds the token's field, and submits the form
/// once the widget's callback is called with a token.
const SCRIPT: &str = include_str!("../../pages/protection/turnstile.js");

const SITE_KEY_CHARACTERS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// The length of a site key past its `0x4AAAAAAA`, as Cloudflare's are.
const SITE_KEY_LENGTH: u64 = 14;

/// The widget's site key for `seed`.
pub(super) fn site_key(seed: u64) -> String {
    let drawn = (0..SITE_KEY_LENGTH).map(|place| {
        let at = generator::below(
            seed,
            &[FAMILY_PLACE, 0, SITE_KEY_PLACE, place],
            SITE_KEY_CHARACTERS.len() as u64,
        );
        char::from(SITE_KEY_CHARACTERS[super::index(at)])
    });
    "0x4AAAAAAA".chars().chain(drawn).collect()
}

/// The cookie value an accepted token sets for `seed`.
pub(super) fn cookie(seed: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, 0, TURNSTILE_COOKIE_PLACE])
    )
}

/// The token `number` for the widget of `site_key` on the page at `path` of
/// `host`, a host name without its port: the number, then a digest only the
/// Proving ground's seed gives.
pub(crate) fn token(seed: u64, site_key: &str, host: &str, path: &str, number: u64) -> String {
    let mut digest = Sha256::new();
    for part in [
        seed.to_string().as_str(),
        site_key,
        host,
        path,
        &number.to_string(),
    ] {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    let mac = data_encoding::HEXLOWER.encode(&digest.finalize());
    format!("0.{number:016x}.{mac}")
}

/// Whether the page on `host` accepts `given`: a token of its own widget
/// and page.
pub(super) fn accepts(seed: u64, host: &str, given: &str) -> bool {
    given
        .strip_prefix("0.")
        .and_then(|rest| rest.split_once('.'))
        .and_then(|(number, _)| u64::from_str_radix(number, 16).ok())
        .is_some_and(|number| given == token(seed, &site_key(seed), host, PATH, number))
}

/// The page of a client whose token was not accepted yet.
pub(super) fn page(seed: u64) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>One more step</title></head>\n<body>\n<main id=\"turnstile\">\n<h1>Verify you are human</h1>\n<form method=\"post\" action=\"{PATH}\">\n<div class=\"cf-turnstile\" data-sitekey=\"{}\" data-response-field-name=\"{FIELD}\" data-callback=\"solved\"></div>\n<button type=\"submit\">Continue</button>\n</form>\n</main>\n<script>\n{SCRIPT}</script>\n</body>\n</html>\n",
        site_key(seed)
    )
}
