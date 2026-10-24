//! A URL's equivalent forms as pure functions of the URL (ADR 0120): the
//! form two URLs differ by, alone, with the host its rule holds on, and
//! the spelling a learned form keeps.

use rstest::rstest;
use url::Url;

use super::{differ_by, kept, name};
use goodfetch_protocol::foxhound::report::EquivalentForm;

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

#[rstest]
#[case::www("http://www.site.test/a", "http://site.test/a", Some(("www", "site.test")))]
#[case::bare_first("http://site.test/a?b=1", "http://www.site.test/a?b=1", Some(("www", "site.test")))]
#[case::scheme("http://site.test/a", "https://site.test/a", Some(("scheme", "site.test")))]
#[case::slash("http://site.test/a", "http://site.test/a/", Some(("trailing_slash", "site.test")))]
#[case::index_html("http://site.test/docs/index.html", "http://site.test/docs/", Some(("index_html", "site.test")))]
#[case::index_php("http://site.test/index.php", "http://site.test/", Some(("index_php", "site.test")))]
#[case::two_forms("http://www.site.test/a", "https://site.test/a", None)]
#[case::another_path("http://site.test/a", "http://site.test/", None)]
#[case::another_domain("http://site.test/a", "http://www.other.test/a", None)]
#[case::the_root_has_no_slash_form("http://site.test/", "http://site.test//", None)]
#[case::a_public_suffix_is_no_bare_host("http://www.co.uk/a", "http://co.uk/a", None)]
#[case::an_address_has_other_forms("http://192.0.2.1/a", "http://192.0.2.1/a/", Some(("trailing_slash", "192.0.2.1")))]
#[case::an_index_file_in_a_name("http://site.test/myindex.html", "http://site.test/my", None)]
#[case::the_same_url("http://site.test/a", "http://site.test/a", None)]
fn two_urls_differ_by_one_form_alone(
    #[case] one: &str,
    #[case] other: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let found = differ_by(&url(one), &url(other));
    assert_eq!(
        found.map(|(form, host)| (name(form), host)),
        expected.map(|(form, host)| (form, host.to_owned()))
    );
    let back = differ_by(&url(other), &url(one));
    assert_eq!(
        back.map(|(form, host)| (name(form), host)),
        expected.map(|(form, host)| (form, host.to_owned())),
        "the difference is the same either way"
    );
}

#[rstest]
#[case::www(
    EquivalentForm::Www,
    "http://www.site.test/a",
    Some("http://site.test/a")
)]
#[case::bare_stays(EquivalentForm::Www, "http://site.test/a", None)]
#[case::scheme(
    EquivalentForm::Scheme,
    "http://site.test/a?b=1",
    Some("https://site.test/a?b=1")
)]
#[case::https_stays(EquivalentForm::Scheme, "https://site.test/a", None)]
#[case::slash(
    EquivalentForm::TrailingSlash,
    "http://site.test/a/",
    Some("http://site.test/a")
)]
#[case::the_root_stays(EquivalentForm::TrailingSlash, "http://site.test/", None)]
#[case::index(
    EquivalentForm::IndexHtml,
    "http://site.test/d/index.html",
    Some("http://site.test/d/")
)]
#[case::the_folder_stays(EquivalentForm::IndexHtml, "http://site.test/d/", None)]
fn a_learned_form_keeps_one_spelling(
    #[case] form: EquivalentForm,
    #[case] given: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(kept(form, &url(given)), expected.map(url));
}
