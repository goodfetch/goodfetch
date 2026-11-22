use rstest::rstest;
use serde_json::json;

use super::{BROWSER_MIRROR, CHROME_FOR_TESTING_STORAGE};
use crate::catalog::ValueKind;

fn checked(value: &serde_json::Value) -> Result<(), String> {
    let ValueKind::Shape(shape) = BROWSER_MIRROR.kind else {
        panic!("`browser.mirror` holds a shape");
    };
    (shape.check)(value)
}

#[rstest]
#[case::upstream(CHROME_FOR_TESTING_STORAGE)]
#[case::a_port_and_a_path("https://mirror.example:8443/chrome-for-testing")]
#[case::a_bare_host("https://mirror.example")]
fn a_mirror_is_an_https_url_without_query_fragment_or_credentials(#[case] url: &str) {
    assert_eq!(checked(&json!(url)), Ok(()));
}

#[rstest]
#[case::plain_http("http://mirror.example/builds")]
#[case::no_host("https:///builds")]
#[case::a_query("https://mirror.example/builds?token=1")]
#[case::a_fragment("https://mirror.example/builds#top")]
#[case::credentials("https://user:secret@mirror.example/builds")]
#[case::a_space("https://mirror.example/my builds")]
fn any_other_text_is_refused_naming_the_form(#[case] url: &str) {
    assert_eq!(
        checked(&json!(url)),
        Err(format!(
            "an `https://` URL with a host and no query, fragment or credentials, found `{url}`"
        ))
    );
}

#[test]
fn a_value_that_is_no_text_is_refused() {
    assert_eq!(
        checked(&json!(443)),
        Err(
            "an `https://` URL with a host and no query, fragment or credentials, found `443`"
                .to_owned()
        )
    );
}
