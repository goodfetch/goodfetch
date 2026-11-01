use rstest::rstest;

use super::{SeedError, Suite, SuiteError, Variables};

#[test]
fn goodfetch_test_seed_overrides_the_seed() {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SEED", "4242")]);

    let seed = variables.seed("env::tests::any_test");

    assert_eq!(seed, Ok(4242));
}

#[test]
fn two_tests_get_different_seeds() {
    let variables = Variables::default();

    let first = variables.seed("env::tests::first");
    let second = variables.seed("env::tests::second");

    assert_ne!(first.unwrap(), second.unwrap());
}

#[test]
fn one_test_gets_the_same_seed_on_every_run() {
    let variables = Variables::default();

    let seed = variables.seed("env::tests::first");

    // 64-bit FNV-1a of the name, shifted right by 11 bits.
    assert_eq!(seed, Ok(7_452_444_360_195_669));
}

#[rstest]
#[case::not_a_number("forty", SeedError::NotANumber)]
#[case::above_what_javascript_reads_exactly("9007199254740992", SeedError::TooLarge)]
fn an_unreadable_seed_override_is_refused(#[case] value: &str, #[case] expected: SeedError) {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SEED", value)]);

    let seed = variables.seed("env::tests::any_test");

    assert_eq!(seed, Err(expected));
}

#[rstest]
#[case::quick("quick", Suite::Quick)]
#[case::pre_merge("pre-merge", Suite::PreMerge)]
#[case::release("release", Suite::Release)]
fn suite_follows_goodfetch_test_suite(#[case] value: &str, #[case] expected: Suite) {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SUITE", value)]);

    let suite = variables.suite();

    assert_eq!(suite, Ok(expected));
}

#[test]
fn suite_is_quick_when_goodfetch_test_suite_is_absent() {
    let variables = Variables::default();

    let suite = variables.suite();

    assert_eq!(suite, Ok(Suite::Quick));
}

#[test]
fn an_unknown_suite_is_refused() {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SUITE", "nightly")]);

    let suite = variables.suite();

    assert_eq!(
        suite,
        Err(SuiteError {
            value: "nightly".to_owned()
        })
    );
}
