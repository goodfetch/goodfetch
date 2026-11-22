use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{Form, ProductVersion};
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::translated;
use crate::product::{Channel, Identity};
use crate::settings::SettingsError;

/// A refused key reads in its Client's form where the message names what
/// the Client set, and by its canonical name where a file set it, as does
/// the key a `fix` names in a file to edit (ADR 0099).
#[rstest]
#[case::set_by_the_client(SettingsLayer::Run, "`timeout.firstByte` cannot be set in run: ")]
#[case::read_from_a_file(
    SettingsLayer::UserConfiguration,
    "`timeout.first_byte` cannot be set in user_configuration: "
)]
fn a_refused_key_reads_in_the_form_of_who_set_it(#[case] layer: SettingsLayer, #[case] head: &str) {
    let fix = "set `timeout.first_byte` in the project file";
    let error = SettingsError::NotApplicable {
        key: "timeout.first_byte".to_owned(),
        layer,
        fix: fix.to_owned(),
    };

    let object = translated(&error, &identity(), false).to_error_object_in(Form::Typescript);

    assert_eq!(
        (
            object.message,
            object.data.details["fix"].clone(),
            object.data.details["key"].clone(),
        ),
        (
            format!("{head}{fix}"),
            fix.into(),
            "timeout.first_byte".into(),
        )
    );
}

fn identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    }
}
