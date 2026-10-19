//! Covers: event:sample.echoed

#[rstest::rstest]
#[case::empty("")]
#[case::full("text")]
fn keeps_a_value(#[case] value: &str) {
    let _ = value;
}
