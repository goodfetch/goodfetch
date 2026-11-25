use std::path::Path;

use rstest::rstest;

use super::named;
use crate::variables::Variables;

#[rstest]
#[case::a_released_minor(&[("GOODFETCH_TEST_OLDER_ENGINE", "/released/goodfetch-collie")], "/released/goodfetch-collie")]
#[case::no_release(&[], "/built/goodfetch-collie")]
fn the_variable_names_the_older_engine_in_place_of_the_built_one(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let variables: Variables = variables.iter().copied().collect();

    let engine = named(&variables, Path::new("/built/goodfetch-collie"));

    assert_eq!(engine, Path::new(expected));
}
