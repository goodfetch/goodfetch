use rstest::rstest;

use super::RefusalCase;

/// Each generated case, every registered format's among them, against the
/// compatibility function of the rule it names: a function that disagrees
/// with its rule's table fails its cases (ADR 0066).
#[rstest]
fn each_generated_case_gets_its_rules_verdict(
    #[files("generated/refusals/*/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();

    let verdict = match case.lowest_minor {
        Some(lowest_minor) => {
            case.rule
                .compatibility_of_store(case.reader, case.found, lowest_minor)
        }
        None => case.rule.compatibility(case.reader, case.found),
    };

    assert_eq!(verdict, case.compatibility, "{case:#?}");
}
