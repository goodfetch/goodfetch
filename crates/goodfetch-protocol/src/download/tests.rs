use rstest::rstest;

use super::DownloadId;
use crate::run::RunId;

#[test]
fn a_download_id_splits_into_its_run_and_position() {
    let run_id = RunId::parse("nightly.download-7").unwrap();

    let split = DownloadId::of(&run_id, 12).split();

    assert_eq!(split, Some((run_id, 12)));
}

#[rstest]
#[case::no_mark("nightly-12")]
#[case::no_position("nightly.download-")]
#[case::a_signed_position("nightly.download-+1")]
#[case::not_a_run_id("Nightly.download-1")]
fn a_text_no_engine_names_a_download_with_splits_into_nothing(#[case] text: &str) {
    assert_eq!(DownloadId(text.to_owned()).split(), None);
}
