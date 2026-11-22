use super::*;

/// The cases come from the shared table alone, so a case it gains runs in
/// the suite too; the test only checks that each character the suite's
/// refusals name is among each source's cases.
#[test]
fn every_source_the_suite_sends_has_a_case_of_each_character() {
    for source in ["page_request", "route"] {
        let values = hostile_values(source);

        for character in ["crlf", "lf", "cr", "nul"] {
            let case = format!("{source}_header_with_{character}");
            assert!(
                values.iter().any(|(name, _)| *name == case),
                "the table has no {case}: {values:?}"
            );
        }
    }
}
