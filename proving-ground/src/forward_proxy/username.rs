//! The usernames an authenticated forward proxy accepts: the one `serve`
//! reports, or a sticky username built on it the way each built-in Proxy
//! provider adapter builds its provider's, which names the Session's sticky
//! session and, where the provider's format carries them, its country and
//! lifetime:
//!
//! - `bright_data`: `brd-customer-<username>-zone-<zone>`, then
//!   `-<key>-<value>` pairs among which `session` and `country`;
//! - `oxylabs`: `customer-<username>`, then `-<key>-<value>` pairs among
//!   which `sessid`, `cc` and `sesstime` (minutes);
//! - `apify`: `auto`, or `<key>-<value>` pairs joined by `,` among which
//!   `session` and `country`; Apify's username names no account, so only
//!   the password is checked.

use crate::connection_record::Sticky;

/// A username an authenticated proxy takes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Username {
    /// The one `serve` reports, which asks for no sticky session.
    Reported,
    /// A provider's sticky username.
    Sticky(Sticky),
}

/// Reads `username` against the reported `base`; `None` when the proxy
/// refuses it.
pub(crate) fn read(username: &str, base: &str) -> Option<Username> {
    if username == base {
        return Some(Username::Reported);
    }
    if let Some(rest) = username.strip_prefix("brd-customer-") {
        let rest = rest.strip_prefix(base)?.strip_prefix("-zone-")?;
        let (_zone, pairs) = rest.split_once('-').unwrap_or((rest, ""));
        return dashed_pairs(pairs, "bright_data", "session", "country", None)
            .map(Username::Sticky);
    }
    if let Some(rest) = username.strip_prefix("customer-") {
        let pairs = rest.strip_prefix(base)?;
        let pairs = if pairs.is_empty() {
            pairs
        } else {
            pairs.strip_prefix('-')?
        };
        return dashed_pairs(pairs, "oxylabs", "sessid", "cc", Some("sesstime"))
            .map(Username::Sticky);
    }
    apify(username).map(Username::Sticky)
}

/// Reads `key-value-key-value…` pairs.
fn dashed_pairs(
    pairs: &str,
    provider: &'static str,
    session_key: &str,
    country_key: &str,
    lifetime_key: Option<&str>,
) -> Option<Sticky> {
    let mut sticky = Sticky {
        provider,
        session: None,
        country: None,
        lifetime_minutes: None,
    };
    let mut parts = pairs.split('-').filter(|part| !part.is_empty());
    while let Some(key) = parts.next() {
        let value = parts.next()?;
        if key == session_key {
            sticky.session = Some(value.to_owned());
        } else if key == country_key {
            sticky.country = Some(value.to_owned());
        } else if Some(key) == lifetime_key {
            sticky.lifetime_minutes = Some(value.parse().ok()?);
        }
    }
    Some(sticky)
}

/// Reads Apify's `auto` or its `key-value,key-value…` parameters.
fn apify(username: &str) -> Option<Sticky> {
    let mut sticky = Sticky {
        provider: "apify",
        session: None,
        country: None,
        lifetime_minutes: None,
    };
    if username == "auto" {
        return Some(sticky);
    }
    for parameter in username.split(',') {
        let (key, value) = parameter.split_once('-')?;
        match key {
            "session" => sticky.session = Some(value.to_owned()),
            "country" => sticky.country = Some(value.to_owned()),
            "groups" => {}
            _ => return None,
        }
    }
    Some(sticky)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const BASE: &str = "pg0123abcd";

    #[rstest]
    #[case::another_account("brd-customer-other-zone-residential-session-s1")]
    #[case::a_pair_without_its_value("customer-pg0123abcd-sessid")]
    #[case::a_lifetime_that_is_no_number("customer-pg0123abcd-sesstime-long")]
    #[case::an_unknown_apify_parameter("colour-blue")]
    #[case::another_name("someone")]
    fn a_username_built_on_no_known_format_is_refused(#[case] username: &str) {
        assert_eq!(read(username, BASE), None);
    }

    #[test]
    fn the_reported_username_asks_for_no_sticky_session() {
        assert_eq!(read(BASE, BASE), Some(Username::Reported));
    }

    #[test]
    fn a_bright_data_username_names_its_session_and_country() {
        assert_eq!(
            read(
                "brd-customer-pg0123abcd-zone-isp-country-de-session-s1",
                BASE
            ),
            Some(Username::Sticky(Sticky {
                provider: "bright_data",
                session: Some("s1".to_owned()),
                country: Some("de".to_owned()),
                lifetime_minutes: None,
            }))
        );
    }
}
