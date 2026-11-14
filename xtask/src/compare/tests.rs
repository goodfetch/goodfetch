use super::*;

#[test]
fn each_crawl_scenario_can_be_asked_for_by_its_own_name() {
    for scenario in Scenario::ALL {
        let asked: Vec<Asked> = Asked::value_variants()
            .iter()
            .copied()
            .filter(|asked| asked.crawl() == Some(scenario))
            .collect();
        assert_eq!(asked.len(), 1, "{scenario:?}");
        assert_eq!(
            asked[0]
                .to_possible_value()
                .map(|value| value.get_name().to_owned()),
            scenario
                .to_possible_value()
                .map(|value| value.get_name().to_owned()),
        );
    }
    assert_eq!(Asked::crawls(&[]), Scenario::ALL);
    assert!(Asked::collie(&[]));
    assert!(!Asked::collie(&[Asked::Raw]));
}
