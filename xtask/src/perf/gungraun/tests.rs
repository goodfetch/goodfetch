use rstest::rstest;

use super::source;

/// A benchmark's ID reads back as its package, its bench file and its
/// function; a subject that is no benchmark's reads as none.
#[rstest]
#[case::a_benchmark(
    "goodfetch-frontier/instructions::frontier::push",
    Some(("goodfetch-frontier", "instructions", "push"))
)]
#[case::a_benchmark_with_an_id(
    "goodfetch-page/parse::html::parse/large",
    Some(("goodfetch-page", "parse", "parse"))
)]
#[case::no_group("goodfetch-page/parse::parse", None)]
#[case::no_package("instructions::frontier::push", None)]
#[case::a_size("goodfetch-sdk.js", None)]
fn a_benchmark_id_reads_back_as_where_it_is_written(
    #[case] subject: &str,
    #[case] written: Option<(&str, &str, &str)>,
) {
    assert_eq!(
        source(subject).map(|source| (source.package, source.file, source.function)),
        written
    );
}
