use goodfetch_protocol::connection::Form;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use serde_json::json;

use super::{CallError, named};

/// A failure that marks a setting reads canonically to every reader but
/// its Connection, a log and a command of the executable included, and in
/// its Connection's form on the wire, the head of its call and a text of
/// its details included; its structured fields stay canonical.
#[test]
fn a_marked_failure_reads_canonically_but_on_its_connection() {
    let setting = named::setting("timeout.first_byte");
    let error = CallError::new(
        &SETTING_INVALID,
        format!("`{setting}` is 0"),
        &SettingInvalidDetails {
            key: "timeout.first_byte".to_owned(),
            layer: Some(SettingsLayer::Run),
            reason: "0 ends the call at once".to_owned(),
            limit: None,
            fix: Some(format!("raise `{setting}`")),
        },
    )
    .in_call("page.wait_for_url", Form::Typescript);

    let on_wire = error.to_error_object_in(Form::Typescript);

    assert_eq!(
        (
            error.to_string(),
            error.details().map(|details| details["fix"].clone()),
        ),
        (
            "setting_invalid: page.waitForURL: `timeout.first_byte` is 0".to_owned(),
            Some(json!("raise `timeout.first_byte`")),
        )
    );
    assert_eq!(
        (
            on_wire.message.as_str(),
            &on_wire.data.details["fix"],
            &on_wire.data.details["key"],
            &on_wire.data.details["operation"],
        ),
        (
            "page.waitForURL: `timeout.firstByte` is 0",
            &json!("raise `timeout.firstByte`"),
            &json!("timeout.first_byte"),
            &json!("page.wait_for_url"),
        )
    );
}
