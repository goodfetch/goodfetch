use rstest::rstest;

use super::ExportId;
use crate::run::RunId;

const SHA256: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

#[test]
fn an_export_id_splits_into_its_run_its_run_epoch_and_sha256() {
    let run_id = RunId::parse("nightly.export-7").unwrap();

    let id = ExportId::of(&run_id, 12, SHA256);

    assert_eq!(
        (id.0.as_str(), id.split()),
        (
            "nightly.export-7.export-12-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            Some((run_id, 12, SHA256))
        )
    );
}

#[rstest]
#[case::no_mark("nightly-1-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")]
#[case::no_run_epoch(
    "nightly.export-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
)]
#[case::a_signed_run_epoch(
    "nightly.export-+1-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
)]
#[case::a_run_epoch_with_a_leading_zero(
    "nightly.export-01-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
)]
#[case::a_short_digest("nightly.export-1-9f86d0")]
#[case::an_uppercase_digest(
    "nightly.export-1-9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08"
)]
#[case::a_digest_naming_a_parent(
    "nightly.export-1-../../../../../../../../../../../../../../../../etc/pass"
)]
#[case::not_a_run_id(
    "Nightly.export-1-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
)]
fn a_text_no_engine_names_an_export_with_splits_into_nothing(#[case] text: &str) {
    assert_eq!(ExportId(text.to_owned()).split(), None);
}
