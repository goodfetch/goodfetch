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

/// A redo measures with the run's own arguments, read from its rerun
/// command, the redo's tools, Spider builds and trials file kept (#2521).
#[test]
fn a_redo_measures_with_the_runs_own_arguments() {
    let redo = recipe_arguments(&["--redo", "--out", "elsewhere/trials.json"]);
    let command = "just compare release --scenario browser --spider-pages 500 \
                   --foxhound-setting 'governor.worker_threads = \"per_core\"' \
                   --browser /opt/chrome --out target/compare/moved.json";

    let arguments = redone_arguments(command, &redo).expect("the command parses");

    assert_eq!(
        (
            arguments.scenarios,
            arguments.spider_pages,
            arguments.foxhound_settings,
            arguments.browser,
            arguments.out,
            arguments.tools,
            arguments.spider_builds,
        ),
        (
            vec![Asked::Browser],
            Some(500),
            vec!["governor.worker_threads = \"per_core\"".to_owned()],
            Some(PathBuf::from("/opt/chrome")),
            PathBuf::from("elsewhere/trials.json"),
            redo.tools,
            redo.spider_builds,
        )
    );
}

/// A redo builds its tools with the profile it is given, so one given
/// another profile than the run's is refused rather than measured on other
/// builds.
#[test]
fn a_redo_at_another_profile_than_the_runs_is_refused() {
    let redo = recipe_arguments(&["--redo"]);

    let error = redone_arguments("just compare dev --trials 3", &redo)
        .map(|_| ())
        .expect_err("refused");

    assert_eq!(
        error.to_string(),
        "the trials file's run built its tools with the `dev` profile: redo it with `just compare dev --redo`"
    );
}
