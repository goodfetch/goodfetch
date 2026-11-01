use rstest::rstest;

use super::ExportId;
use crate::run::RunId;

const SHA256: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

#[test]
fn an_export_id_splits_into_its_run_and_sha256() {
    let run_id = RunId::parse("nightly.export-7").unwrap();

    let id = ExportId::of(&run_id, SHA256);

    assert_eq!(id.split(), Some((run_id, SHA256)));
}

#[rstest]
#[case::no_mark("nightly-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08")]
#[case::a_short_digest("nightly.export-9f86d0")]
#[case::an_uppercase_digest(
    "nightly.export-9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08"
)]
#[case::a_digest_naming_a_parent(
    "nightly.export-../../../../../../../../../../../../../../../../etc/pass"
)]
#[case::not_a_run_id(
    "Nightly.export-9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
)]
fn a_text_no_engine_names_an_export_with_splits_into_nothing(#[case] text: &str) {
    assert_eq!(ExportId(text.to_owned()).split(), None);
}
