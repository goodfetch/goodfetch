//! The contacts a page shows (spec #580, ticket #787): e-mail addresses
//! from `mailto:` links and text, telephone numbers from `tel:` links and
//! text with E.164 only when the number carries its country code or the
//! document's language tag carries a region (G1), and social profiles from
//! links to the extractor's list of networks; each with the text it was
//! read from, once, where it first appears.

use std::fmt::Write as _;

use goodfetch_protocol::page::{Contacts, Email, Phone, SocialNetwork, SocialProfile};
use rstest::rstest;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::read;
use crate::social::{NETWORKS, SOCIAL_NETWORKS_VERSION};

fn contacts(html: &str) -> Contacts {
    read(html, json!({ "contacts": true })).contacts.unwrap()
}

fn email(value: &str, text: &str) -> Email {
    Email {
        value: value.to_owned(),
        text: text.to_owned(),
    }
}

fn phone(value: &str, text: &str, e164: Option<&str>) -> Phone {
    Phone {
        value: value.to_owned(),
        text: text.to_owned(),
        e164: e164.map(ToOwned::to_owned),
    }
}

fn profile(value: &str, text: &str, network: SocialNetwork) -> SocialProfile {
    SocialProfile {
        value: value.to_owned(),
        text: text.to_owned(),
        network,
    }
}

/// Covers: story:6-7
#[test]
fn e_mail_addresses_come_from_mailto_links_and_text_each_once() {
    let html = r#"<html lang="en"><body>
        <p>Write to <a href="mailto:shop@northwind.example?subject=Kites">the shop</a>
        or <a href="MAILTO:desk@kites.test,%20repairs@kites.test">both desks</a>.</p>
        <p>Press: press@kites.test. Again, press@kites.test, and
        <a href="mailto:shop@northwind.example">shop@northwind.example</a>.</p>
        <p>None of these: @kites, a@, a@-b.test, <a href="mailto:">nobody</a>.</p>
        <script>const hidden = "hidden@kites.test";</script>
        </body></html>"#;

    assert_eq!(
        contacts(html).emails,
        [
            email("shop@northwind.example", "the shop"),
            email("desk@kites.test", "both desks"),
            email("repairs@kites.test", "both desks"),
            email("press@kites.test", "press@kites.test"),
        ]
    );
}

/// Covers: story:6-7
#[test]
fn a_number_in_a_document_whose_language_tag_has_a_region_gets_e164() {
    let html = r#"<html lang="fr-FR"><body>
        <p><a href="tel:01%2042%2068%2053%2000">Appelez-nous</a>,
        fax : 01 42 68 53 01, Londres : +44 20 7031 3000.</p>
        <p>Commande 2026-10-26, 12 345 pièces.</p>
        </body></html>"#;

    assert_eq!(
        contacts(html).phones,
        [
            phone("01 42 68 53 00", "Appelez-nous", Some("+33142685300")),
            phone("01 42 68 53 01", "01 42 68 53 01", Some("+33142685301")),
            phone(
                "+44 20 7031 3000",
                "+44 20 7031 3000",
                Some("+442070313000")
            ),
        ]
    );
}

/// No country is guessed (G1): with no region in the language tag, a
/// number without its country code, from a `tel:` link or from text, has
/// no E.164; a date written with digits alone is no number.
///
/// Covers: story:6-7
#[test]
fn a_number_without_its_country_code_and_no_region_gets_no_e164() {
    let html = r#"<html lang="en"><body>
        <p><a href="tel:0142685300">Call us</a> or 01 42 68 53 01;
        in the US, +1 650-253-0000;
        <a href="tel:+44-20-7031-3000;ext=12">London</a>.</p>
        <p>Ordered 2026-10-26, shipped 28.10.2026.</p>
        </body></html>"#;

    assert_eq!(
        contacts(html).phones,
        [
            phone("0142685300", "Call us", None),
            phone("01 42 68 53 01", "01 42 68 53 01", None),
            phone("+1 650-253-0000", "+1 650-253-0000", Some("+16502530000")),
            phone("+44-20-7031-3000", "London", Some("+442070313000")),
        ]
    );
}

/// Covers: story:6-7
#[rstest]
#[case::language_and_region("fr-FR", Some("+33142685300"))]
#[case::another_language_in_the_region("en-FR", Some("+33142685300"))]
#[case::a_script_before_the_region("fr-Latn-FR", Some("+33142685300"))]
#[case::a_region_in_lower_case("fr-fr", Some("+33142685300"))]
#[case::language_alone("fr", None)]
#[case::a_numeric_region("fr-419", None)]
#[case::no_language_tag("", None)]
fn only_a_region_in_the_language_tag_gives_a_national_number_its_e164(
    #[case] lang: &str,
    #[case] e164: Option<&str>,
) {
    let html =
        format!(r#"<html lang="{lang}"><body><a href="tel:01 42 68 53 00">Call</a></body></html>"#);

    assert_eq!(
        contacts(&html).phones,
        [phone("01 42 68 53 00", "Call", e164)]
    );
}

/// Covers: story:6-7
#[test]
fn social_profiles_come_from_links_to_the_listed_networks_and_mastodon_by_rel_me() {
    let html = r#"<html lang="en"><body><footer>
        <a href="https://x.com/northwind">On X</a>
        <a href="https://twitter.com/northwind_kites">Old account</a>
        <a href="https://www.linkedin.com/company/northwind/">LinkedIn</a>
        <a href="https://www.facebook.com/sharer/sharer.php?u=https://kites.test/">Share</a>
        <a href="https://m.facebook.com/northwind">Facebook</a>
        <a href="https://instagram.com/northwind">Instagram</a>
        <a href="https://www.youtube.com/@northwind">YouTube</a>
        <a href="https://www.tiktok.com/@northwind">TikTok</a>
        <a href="//github.com/northwind">  GitHub </a>
        <a rel="me" href="https://mastodon.social/@northwind">Mastodon</a>
        <a href="https://mastodon.social/@someone">Not ours</a>
        <a rel="me" href="https://kites.test/about">About</a>
        <a href="https://x.com/">X home</a>
        <a href="https://x.com/intent/tweet?text=Kites">Tweet this</a>
        <a href="https://github.com.kites.test/northwind">Look-alike</a>
        <a href="https://github.com/northwind">GitHub again</a>
        </footer></body></html>"#;

    assert_eq!(
        contacts(html).social,
        [
            profile("https://x.com/northwind", "On X", SocialNetwork::X),
            profile(
                "https://twitter.com/northwind_kites",
                "Old account",
                SocialNetwork::X
            ),
            profile(
                "https://www.linkedin.com/company/northwind/",
                "LinkedIn",
                SocialNetwork::Linkedin
            ),
            profile(
                "https://m.facebook.com/northwind",
                "Facebook",
                SocialNetwork::Facebook
            ),
            profile(
                "https://instagram.com/northwind",
                "Instagram",
                SocialNetwork::Instagram
            ),
            profile(
                "https://www.youtube.com/@northwind",
                "YouTube",
                SocialNetwork::Youtube
            ),
            profile(
                "https://www.tiktok.com/@northwind",
                "TikTok",
                SocialNetwork::Tiktok
            ),
            profile(
                "https://github.com/northwind",
                "GitHub",
                SocialNetwork::Github
            ),
            profile(
                "https://mastodon.social/@northwind",
                "Mastodon",
                SocialNetwork::Mastodon
            ),
        ]
    );
}

/// The list of networks is versioned with the extractor: each version
/// pins the digest of its list, so a list that changes fails here until
/// its version changes with it.
///
/// Covers: story:6-7
#[test]
fn the_social_networks_list_changes_only_with_its_version() {
    let mut listed = String::new();
    for network in NETWORKS {
        writeln!(
            listed,
            "{:?} {:?} {:?}",
            network.network, network.hosts, network.not_profiles
        )
        .unwrap();
    }
    let digest = format!("{:x}", Sha256::digest(listed.as_bytes()));

    let pinned = match SOCIAL_NETWORKS_VERSION {
        1 => "f118742610ad04ca55c784e7b7b69f16dcbcae00fe02e2284f7159c0baf2b016",
        version => panic!("version {version} pins no digest here; pin its list's"),
    };
    assert_eq!(
        digest, pinned,
        "the social networks list changed: give it a new SOCIAL_NETWORKS_VERSION and pin its digest"
    );
}

/// Covers: story:6-7
#[test]
fn a_page_with_no_contacts_answers_empty_lists() {
    assert_eq!(
        contacts("<p>Kites fly.</p>"),
        Contacts {
            emails: Vec::new(),
            phones: Vec::new(),
            social: Vec::new(),
        }
    );
}
