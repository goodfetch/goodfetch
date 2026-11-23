use std::path::PathBuf;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{Form, ProductVersion};
use goodfetch_protocol::run::{RUN_STATE_LOCATION, RunId, RunStateFilesystem};
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::{start_error, translated};
use crate::product::{Channel, Identity};
use crate::runs::{StartError, UnsupportedFolder};
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

/// A start whose run folder is on a filesystem the engine keeps no run
/// state on answers with the filesystem, the folder and a fix that names
/// the setting to move it with (ADR 0049).
///
/// Covers: story:3-33, error:run_state_filesystem_unsupported
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn a_start_refused_for_its_folders_filesystem_names_the_setting_that_moves_it() {
    let refused = UnsupportedFolder {
        folder: PathBuf::from("/mnt/bucket/runs/nightly"),
        filesystem: RunStateFilesystem::FuseObjectStorage,
        found: "fuse.s3fs".to_owned(),
    };

    let error = start_error(StartError::FilesystemUnsupported(
        RunId::parse("nightly").unwrap(),
        refused,
    ));

    let details = error.details().unwrap();
    assert_eq!(
        (
            error.code(),
            details["filesystem"].clone(),
            details["path"].clone()
        ),
        (
            "run_state_filesystem_unsupported",
            "fuse_object_storage".into(),
            "/mnt/bucket/runs/nightly".into(),
        )
    );
    assert!(
        details["fix"]
            .as_str()
            .unwrap()
            .contains(&format!("`{}`", RUN_STATE_LOCATION.name))
    );
}

fn identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    }
}
