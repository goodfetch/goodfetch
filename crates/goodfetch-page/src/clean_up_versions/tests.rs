#![expect(
    clippy::disallowed_methods,
    reason = "the test reads and writes a temporary folder outside any engine"
)]

use super::replace;

/// Covers: story:5-97
#[test]
fn the_sources_state_is_replaced_whole_through_a_partial_file_that_does_not_stay() {
    let env = goodfetch_test_support::TestEnv::new();
    let state = env.folder().join("0.0").join("clean-up-rules.json");

    replace(&state, b"{\"upstream\":1}").unwrap();
    replace(&state, b"{\"upstream\":2}").unwrap();

    let mut left: Vec<String> = std::fs::read_dir(state.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        (std::fs::read_to_string(&state).unwrap(), left),
        (
            "{\"upstream\":2}".to_owned(),
            vec!["clean-up-rules.json".to_owned()]
        )
    );
}
