//! Where a JSON placeholder's string goes in its body, and how a header's
//! value is built from a placeholder's.

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{Form, set, settable};

#[rstest]
#[case::a_new_member("/password", true)]
#[case::a_member_replaced("/user", true)]
#[case::a_nested_member("/auth/token", true)]
#[case::an_escaped_member("/a~1b", true)]
#[case::an_element_held("/codes/1", true)]
#[case::an_element_past_the_end("/codes/2", false)]
#[case::an_index_with_a_leading_zero("/codes/01", false)]
#[case::the_end_of_an_array("/codes/-", false)]
#[case::a_member_of_a_missing_object("/missing/token", false)]
#[case::a_member_of_a_string("/user/token", false)]
#[case::the_whole_document("", false)]
fn a_pointer_is_settable_where_its_parent_holds_it(#[case] pointer: &str, #[case] expected: bool) {
    let body = json!({ "user": "ann", "auth": {}, "codes": ["a", "b"] });

    assert_eq!(settable(&body, pointer), expected);
}

#[test]
fn a_string_set_at_each_pointer_lands_there_and_nowhere_else() {
    let mut body = json!({ "user": "ann", "auth": {}, "codes": ["a", "b"] });

    for pointer in ["/password", "/auth/token", "/a~1b", "/codes/1"] {
        set(&mut body, pointer, Value::String("v".to_owned()));
    }

    assert_eq!(
        body,
        json!({
            "user": "ann",
            "password": "v",
            "auth": { "token": "v" },
            "a/b": "v",
            "codes": ["a", "v"],
        })
    );
}

#[test]
fn a_header_value_is_its_prefix_and_the_value_or_basic_credentials() {
    let value = SecretString::from("open sesame");
    let built = [
        Form::Value {
            prefix: "Bearer ".to_owned(),
        },
        Form::Value {
            prefix: String::new(),
        },
        Form::Basic {
            username: "Aladdin".to_owned(),
        },
    ]
    .map(|form| form.build(&value).expose_secret().to_owned());

    assert_eq!(
        built,
        [
            "Bearer open sesame".to_owned(),
            "open sesame".to_owned(),
            // RFC 7617's own example.
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==".to_owned(),
        ]
    );
}

#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
/// Covers: story:5-84, error:secret_input_refused
#[test]
fn other_code_guard_toggles_other_code_may_read_and_builds_refusal() {
    use goodfetch_protocol::secrets::{SECRET_INPUT_REFUSED, SecretInputRefusedReason};
    use goodfetch_test_support::runtime;

    use super::secret_input_refused;
    use crate::live_run::played_page;

    runtime().block_on(async {
        let (driver, _played, _served) = played_page();

        assert!(!driver.other_code_may_read());
        let guard = driver.attach_other_code();
        assert!(driver.other_code_may_read());
        let nested = driver.attach_other_code();
        drop(guard);
        assert!(driver.other_code_may_read());
        drop(nested);
        assert!(!driver.other_code_may_read());

        let refused = secret_input_refused(
            "pw".to_owned(),
            "env:FAKE_PW".to_owned(),
            "example.test".to_owned(),
            SecretInputRefusedReason::OtherCodeMayRead,
        );
        assert_eq!(refused.code(), SECRET_INPUT_REFUSED.name);
        assert_eq!(
            refused.details().unwrap()["reason"],
            serde_json::to_value(SecretInputRefusedReason::OtherCodeMayRead).unwrap()
        );
    });
}
