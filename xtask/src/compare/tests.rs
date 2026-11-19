use super::*;

/// `xtask compare run`'s arguments as `just compare` gives them: the
/// recipe's own, then `given`, as its caller gives them after the profile.
pub(crate) fn recipe_arguments(given: &[&str]) -> RunArguments {
    parsed(
        "target/compare/test-tools.json",
        "target/compare-spider",
        "release",
        given,
    )
    .expect("the test's arguments parse")
}

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

/// A run given `--cell` measures only the cells it names, as a fresh run
/// of the full trials, Foxhound's settings allowed with it (#2200).
#[test]
fn a_run_measures_only_the_cells_it_names() {
    let arguments = recipe_arguments(&[
        "--cell",
        "browser/foxhound",
        "--cell",
        "raw/foxhound",
        "--foxhound-setting",
        "governor.worker_threads = \"per_core\"",
    ]);

    let options = options(&arguments, &Environment::of(&[])).expect("the cells are named");

    assert_eq!(
        (options.only, options.trials),
        (
            vec![(Scenario::Browser, "foxhound"), (Scenario::Raw, "foxhound")],
            trials::TRIALS
        )
    );
}

/// A run's `--cell` names a cell of a scenario the run asks for.
#[test]
fn a_run_refuses_a_cell_of_a_scenario_it_does_not_ask_for() {
    let arguments = recipe_arguments(&["--scenario", "raw", "--cell", "browser/foxhound"]);

    let error = options(&arguments, &Environment::of(&[]))
        .map(|_| ())
        .expect_err("refused");

    assert_eq!(error.to_string(), "the run asked for no scenario `browser`");
}
