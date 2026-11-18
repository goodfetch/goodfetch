use rstest::rstest;

use std::num::NonZeroUsize;

use super::{CallgrindAtOnceError, SeedError, Suite, SuiteError, Variables};

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

#[rstest]
#[case::one("1", 1)]
#[case::two("2", 2)]
fn the_callgrind_cap_follows_its_variable(#[case] value: &str, #[case] expected: usize) {
    let variables = Variables::from_iter([("GOODFETCH_COUNTS_CALLGRIND_AT_ONCE", value)]);

    let cap = variables.callgrind_at_once();

    assert_eq!(cap, Ok(NonZeroUsize::new(expected)));
}

#[test]
fn there_is_no_callgrind_cap_when_its_variable_is_absent() {
    let variables = Variables::default();

    let cap = variables.callgrind_at_once();

    assert_eq!(cap, Ok(None));
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::a_word("two")]
#[case::a_fraction("1.5")]
#[case::empty("")]
fn a_callgrind_cap_that_is_no_whole_number_of_one_or_more_is_refused(#[case] value: &str) {
    let variables = Variables::from_iter([("GOODFETCH_COUNTS_CALLGRIND_AT_ONCE", value)]);

    let cap = variables.callgrind_at_once();

    assert_eq!(
        cap,
        Err(CallgrindAtOnceError {
            value: value.to_owned()
        })
    );
}
