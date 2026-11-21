use std::path::{Path, PathBuf};

use super::{
    Executables, Options, counted_output, instruction_cells, order, profile_folder, run, set_aside,
    side_executable, size_points, spider_executable,
};
use crate::compare::matrix::{self, Config, Scenario, SpiderBuild};
use crate::compare::trials::{
    CollieScenario, Comparison, CountedCell, CountedRun, GrowthCell, SizesCell, Tool, Work,
};

/// The executable Cargo builds at `path`, with the OS's suffix (`.exe` on
/// Windows).
fn built(path: &str) -> PathBuf {
    PathBuf::from(format!("{path}{}", std::env::consts::EXE_SUFFIX))
}

/// Each of Spider's side's builds runs the executable Cargo left in the
/// build's own target folder, under the profile's: the main row's from
/// `spider`, a feature's from `spider-<feature>`.
#[test]
fn each_spider_build_runs_the_executable_built_in_its_own_folder() {
    let executable = |build| spider_executable(Path::new("/t/compare-spider"), "release", build);

    assert_eq!(
        (
            executable(SpiderBuild::Http),
            executable(SpiderBuild::Chrome),
            executable(SpiderBuild::Jemalloc),
        ),
        (
            built("/t/compare-spider/spider/release/goodfetch-compare-spider"),
            built("/t/compare-spider/spider-chrome/release/goodfetch-compare-spider"),
            built("/t/compare-spider/spider-jemalloc/release/goodfetch-compare-spider"),
        )
    );
}

/// Each Callgrind run's output is kept in the run's folder under a name of
/// its own, its scenario, cell and number, with no space in it, so a later
/// run of the same cell does not replace it and the reports repository
/// files it (#2281).
#[test]
fn each_callgrind_run_keeps_its_output_under_its_scenario_cell_and_number() {
    let cell = |name| {
        matrix::cell(matrix::Scenario::Raw, name).expect("invariant: scenario 1 has this cell")
    };
    let kept = Path::new("/t/compare/callgrind");

    assert_eq!(
        (
            counted_output(kept, cell("foxhound"), 1),
            counted_output(kept, cell("foxhound"), 3),
            counted_output(kept, cell("spider capacity 4096"), 2),
        ),
        (
            Path::new("/t/compare/callgrind/raw-foxhound-1.out").to_owned(),
            Path::new("/t/compare/callgrind/raw-foxhound-3.out").to_owned(),
            Path::new("/t/compare/callgrind/raw-spider-capacity-4096-2.out").to_owned(),
        )
    );
}

/// Each other Rust side runs the executable Cargo left in the folder of its
/// name among the Rust sides' builds, under the profile's, named after its
/// side's package (#2216, #2217, #2218, issue 2444).
#[test]
fn each_other_rust_side_runs_the_executable_built_in_its_folder() {
    let executable = |folder| side_executable(Path::new("/t/compare-spider"), folder, "release");

    assert_eq!(
        [
            matrix::CRAWLEE_RS_FOLDER,
            matrix::PAGESIFT_FOLDER,
            matrix::OPENSPIDER_FOLDER,
            matrix::FASTCRW_FOLDER,
            matrix::CRAWLBERG_FOLDER,
            matrix::CHROMIUMOXIDE_FOLDER,
        ]
        .map(executable),
        [
            "/t/compare-spider/crawlee-rs/release/goodfetch-compare-crawlee-rs",
            "/t/compare-spider/pagesift/release/goodfetch-compare-pagesift",
            "/t/compare-spider/openspider/release/goodfetch-compare-openspider",
            "/t/compare-spider/fastcrw/release/goodfetch-compare-fastcrw",
            "/t/compare-spider/crawlberg/release/goodfetch-compare-crawlberg",
            "/t/compare-spider/chromiumoxide/release/goodfetch-compare-chromiumoxide",
        ]
        .map(built)
    );
}

/// A profile's builds are read from the folder Cargo writes it to: `debug`
/// for `dev` and `test`, `release` for `bench`, the profile's own name for
/// every other (issue 2099).
#[rstest::rstest]
#[case::dev("dev", "debug")]
#[case::test_profile("test", "debug")]
#[case::release("release", "release")]
#[case::bench("bench", "release")]
#[case::ci("ci", "ci")]
fn each_profiles_builds_are_read_from_the_folder_cargo_writes_it_to(
    #[case] profile: &str,
    #[case] folder: &str,
) {
    assert_eq!(profile_folder(profile), folder);
}

/// Every measured cell's 10 trials run once each, in one order that mixes
/// the cells: no cell's trials all run before another's start. pagesift's
/// polite cell runs one, the owner's decision of 2026-11-14 (#2215).
#[test]
fn each_cells_trials_run_once_each_interleaved_with_the_others() {
    let cells: Vec<_> = matrix::measured().collect();
    let mut state = 7;

    let order = order(&cells, |(cell, _)| cell.runs(10), &mut state);

    let one = |cell: &matrix::Cell| cell.scenario == Scenario::Polite && cell.name == "pagesift";
    let mut ran: Vec<(&str, &str, u32)> = order
        .iter()
        .map(|((cell, _), number)| (cell.scenario.name(), cell.name, *number))
        .collect();
    let first_half: Vec<&str> = ran[..ran.len() / 2]
        .iter()
        .map(|(_, name, _)| *name)
        .collect();
    let mixed = cells
        .iter()
        .filter(|(cell, _)| !one(cell))
        .all(|(cell, _)| first_half.contains(&cell.name));
    ran.sort_unstable();
    let mut expected: Vec<(&str, &str, u32)> = cells
        .iter()
        .flat_map(|(cell, _)| {
            let runs = if one(cell) { 1 } else { 10 };
            (1..=runs).map(move |number| (cell.scenario.name(), cell.name, number))
        })
        .collect();
    expected.sort_unstable();
    assert_eq!((ran, mixed), (expected, true));
}

/// Only a run of the matrix's full trials waits for a quiet machine before
/// each timed trial: a quicker try, which its report flags as short of the
/// 10, never sleeps, so a check that the cells run holds no slot idle.
#[test]
fn only_a_run_of_the_full_trials_waits_for_a_quiet_machine() {
    let waits = |trials| {
        super::waits_for_quiet(&super::Options {
            machine: None,
            command: "just compare release".to_owned(),
            trials,
            growth: false,
            spider_pages: None,
            scenarios: crate::compare::matrix::Scenario::ALL.to_vec(),
            callgrind_outputs: Path::new("/t/compare/callgrind").to_owned(),
            only: Vec::new(),
        })
    };

    assert_eq!((waits(10), waits(3), waits(1)), (true, false, false));
}

/// A counted run goes on while its crawl progresses: it is stopped, with the
/// measured reason, once the Proving ground has received no request for
/// [`super::watch::STALL_SECONDS`], or once it reaches
/// [`super::watch::COUNTED_BOUND_SECONDS`].
#[test]
fn a_counted_run_is_stopped_once_its_crawl_stalls_or_reaches_the_bound() {
    let stall = super::watch::STALL_SECONDS;
    let bound = super::watch::COUNTED_BOUND_SECONDS;

    let going = (
        super::watch::stopped(super::Watch::Counted(1), 300, stall - 1, 240),
        super::watch::stopped(super::Watch::Counted(1), bound - 1, 0, 990),
    );
    let stalled = super::watch::stopped(super::Watch::Counted(1), 400, stall, 240);
    let bounded = super::watch::stopped(super::Watch::Counted(2), bound, 3, 990);

    assert_eq!(
        (going, stalled, bounded),
        (
            (None, None),
            Some(format!(
                "under `strace -f`, run 1 stopped after 400 s: the Proving ground received no request for {stall} s after 240 requests"
            )),
            Some(format!(
                "under `strace -f`, run 2 stopped after {bound} s, the longest a counted run may take, at 990 requests"
            )),
        )
    );
}

/// A timed trial gets the same watch: stopped once the Proving ground has
/// received no request for [`super::watch::STALL_SECONDS`], or once it reaches
/// [`super::watch::TIMED_BOUND_SECONDS`] for the 1,000-page site (scaled to a
/// larger site's pages), with the measured reason.
#[test]
fn a_timed_trial_is_stopped_once_its_crawl_stalls_or_reaches_its_bound() {
    use super::Watch;
    let stall = super::watch::STALL_SECONDS;
    let bound = super::watch::TIMED_BOUND_SECONDS;

    let going = (
        super::watch::stopped(Watch::Timed(3, 1000), 200, stall - 1, 10),
        super::watch::stopped(Watch::Timed(3, 100_000), bound + 1, 0, 990),
    );
    let stalled = super::watch::stopped(Watch::Timed(3, 1000), 130, stall, 0);
    let bounded = super::watch::stopped(Watch::Timed(4, 1000), bound, 2, 990);

    assert_eq!(
        (going, stalled, bounded),
        (
            (None, None),
            Some(format!(
                "timed trial 3 stopped after 130 s: the Proving ground received no request for {stall} s after 0 requests"
            )),
            Some(format!(
                "timed trial 4 stopped after {bound} s, the longest a timed trial may take, at 990 requests"
            )),
        )
    );
}

/// An auto cell's run gets the same watch, so a tool that hangs fails the
/// comparison instead of holding it: stopped once the Proving ground has
/// received no request for [`super::watch::STALL_SECONDS`], or once it
/// reaches [`super::watch::AUTO_BOUND_SECONDS`], with the measured reason.
#[test]
fn an_auto_run_is_stopped_once_its_crawl_stalls_or_reaches_its_bound() {
    use super::Watch;
    let stall = super::watch::STALL_SECONDS;
    let bound = super::watch::AUTO_BOUND_SECONDS;

    let going = (
        super::watch::stopped(Watch::Auto(2), 60, stall - 1, 20),
        super::watch::stopped(Watch::Auto(2), bound - 1, 0, 37),
    );
    let stalled = super::watch::stopped(Watch::Auto(2), 150, stall, 20);
    let bounded = super::watch::stopped(Watch::Auto(3), bound, 1, 37);

    assert_eq!(
        (going, stalled, bounded),
        (
            (None, None),
            Some(format!(
                "auto run 2 stopped after 150 s: the Proving ground received no request for {stall} s after 20 requests"
            )),
            Some(format!(
                "auto run 3 stopped after {bound} s, the longest an auto run may take, at 37 requests"
            )),
        )
    );
}

/// A stopped timed trial runs once more: its retry's figures count, marked
/// with why the first stopped; a second stop leaves the cell with both
/// reasons, and a trial that finishes is never run again.
#[test]
fn a_stopped_timed_trial_is_retried_once() {
    use super::{Outcome, Retried, retried};
    let run = |outcomes: Vec<Outcome<u32>>| {
        let mut outcomes = outcomes.into_iter();
        let mut attempts = 0;
        let result = retried(|| {
            attempts += 1;
            Ok(outcomes.next().expect("no more attempts than outcomes"))
        })
        .unwrap();
        let shown = match result {
            Retried::Done { measured, retried } => format!("done {measured} {retried:?}"),
            Retried::Stopped(reason) => format!("stopped: {reason}"),
        };
        (shown, attempts)
    };

    assert_eq!(
        (
            run(vec![Outcome::Done(7)]),
            run(vec![Outcome::Stopped("stall".to_owned()), Outcome::Done(8)]),
            run(vec![
                Outcome::Stopped("stall".to_owned()),
                Outcome::Stopped("bound".to_owned()),
            ]),
        ),
        (
            ("done 7 None".to_owned(), 1),
            ("done 8 Some(\"stall\")".to_owned(), 2),
            ("stopped: bound; its first attempt: stall".to_owned(), 2),
        )
    );
}

/// Under strace, a cell one of whose runs missed a page of the site will
/// be reported as not measured, so its later runs do not run; another
/// cell's runs, and a cell whose runs fetched the site, go on.
#[test]
fn a_cell_whose_strace_run_missed_pages_runs_no_later_run() {
    let cell = |name| {
        matrix::cell(matrix::Scenario::Browser, name)
            .expect("invariant: the headless browser scenario has this cell")
    };
    let run = |name: &str, pages: u64| CountedRun {
        scenario: matrix::Scenario::Browser,
        cell: name.to_owned(),
        tool: Tool::Spider,
        number: 1,
        load: None,
        count: 9,
        syncs: 0,
        work: Work {
            pages,
            missing: (pages + 1..=1000)
                .map(|page| format!("/sites/1000/{page}"))
                .collect(),
            ..Work::default()
        },
        extracted: None,
    };
    let done = [run("spider", 970), run("crawlee", 1000)];

    assert!(set_aside(&done, cell("spider"), 1000));
    assert!(!set_aside(&done, cell("crawlee"), 1000));
    assert!(!set_aside(&done, cell("scrapy"), 1000));
}

/// A run of `scenarios`, given `--cell` for each of `cells`, whose
/// executables no file holds, so a part over no cell starts nothing and a
/// part that starts a tool fails: how the run ended, and what each
/// hand-over of its trials file said of its Collie scenario and of its
/// sizes, system-call, growth and Callgrind cells, `measured` or why not
/// run.
fn handed_over(
    scenarios: Vec<Scenario>,
    cells: &[&str],
) -> (anyhow::Result<Comparison>, Vec<[String; 5]>) {
    let nowhere = Path::new("/nowhere");
    let executables = Executables {
        proving_ground: nowhere,
        foxhound: nowhere,
        spider_builds: nowhere,
        profile_folder: "release",
        crawlee: nowhere,
        scrapy: nowhere,
        foxhound_program: nowhere,
        browser: nowhere,
        collie: None,
        foxhound_settings: &[],
    };
    // `just compare`'s own arguments, its output paths included (#2521).
    let mut given = vec!["--trials", "1"];
    for cell in cells {
        given.extend(["--cell", cell]);
    }
    let options = Options {
        scenarios,
        ..crate::compare::options(
            &crate::compare::tests::recipe_arguments(&given),
            &crate::environment::Environment::of(&[]),
        )
        .expect("invariant: the test's cells are the matrix's")
    };
    let mut kept = Vec::new();
    let ended = run(
        &executables,
        "0".repeat(40),
        &options,
        &mut |so_far: &Comparison| {
            kept.push([
                collie(&so_far.collie),
                sized(&so_far.sizes),
                reached(&so_far.syscalls),
                grown(&so_far.growth),
                reached(&so_far.instructions),
            ]);
            Ok(())
        },
    );
    (ended, kept)
}

/// A counted cell as a hand-over shows it.
fn reached(cell: &CountedCell) -> String {
    match cell {
        CountedCell::NotRun { reason } => reason.clone(),
        CountedCell::Measured { .. } => "measured".to_owned(),
    }
}

/// The Collie scenario as a hand-over shows it.
fn collie(scenario: &CollieScenario) -> String {
    match scenario {
        CollieScenario::NotRun { reason } => reason.clone(),
        CollieScenario::Measured { .. } => "measured".to_owned(),
    }
}

/// The sizes cell as a hand-over shows it.
fn sized(cell: &SizesCell) -> String {
    match cell {
        SizesCell::NotRun { reason } => reason.clone(),
        SizesCell::Measured { .. } => "measured".to_owned(),
    }
}

/// The growth cell as a hand-over shows it.
fn grown(cell: &GrowthCell) -> String {
    match cell {
        GrowthCell::NotRun { reason } => reason.clone(),
        GrowthCell::Measured { .. } => "measured".to_owned(),
    }
}

/// Why a cell the run has not reached yet is not run.
fn not_reached(cell: &str) -> String {
    format!("the run ended before its {cell} cell")
}

/// A run of hours keeps what it has measured: the trials file's contents
/// are handed over once the timed trials are in, again once the Collie
/// scenario's are, again once the sizes cell is, again once the system-call
/// cell is, again once the growth cell is, again once the Callgrind cell
/// is and again once the hostile pages' passes are, so a failure in a later
/// part, the auto cells' runs (#2411) included, loses none of them (the
/// base run of #2281 lost 220 trials to its first Callgrind run). Before
/// its cell has run, the file says the run did not reach it.
#[test]
fn a_run_hands_over_its_trials_after_each_part_before_the_next() {
    // No scenario, so no cell: every part runs, over nothing.
    let (ended, kept) = handed_over(Vec::new(), &[]);

    let whole = ended.unwrap();
    let no_collie = "the run asked for no Collie cell".to_owned();
    let strace = if crate::compare::trial::strace_missing().is_some() {
        reached(&whole.syscalls)
    } else {
        "measured".to_owned()
    };
    let no_sizes = "the run asked for no scenario 1 main cell".to_owned();
    let no_callgrind =
        "the run asked for no cell of scenario 1 or of the extraction scenario".to_owned();
    assert_eq!(
        kept,
        vec![
            [
                not_reached("Collie"),
                not_reached("sizes"),
                not_reached("system-call"),
                not_reached("growth"),
                not_reached("Callgrind")
            ],
            [
                no_collie.clone(),
                not_reached("sizes"),
                not_reached("system-call"),
                not_reached("growth"),
                not_reached("Callgrind")
            ],
            [
                no_collie.clone(),
                no_sizes.clone(),
                not_reached("system-call"),
                not_reached("growth"),
                not_reached("Callgrind")
            ],
            [
                no_collie.clone(),
                no_sizes.clone(),
                strace.clone(),
                not_reached("growth"),
                not_reached("Callgrind")
            ],
            [
                no_collie.clone(),
                no_sizes.clone(),
                strace.clone(),
                grown(&whole.growth),
                not_reached("Callgrind")
            ],
            [
                no_collie.clone(),
                no_sizes.clone(),
                strace.clone(),
                grown(&whole.growth),
                no_callgrind.clone()
            ],
            [
                no_collie,
                no_sizes,
                strace,
                grown(&whole.growth),
                no_callgrind
            ],
        ]
    );
}

/// A fresh run limited to the cells `--cell` names runs every part a whole
/// run runs, each over those cells alone, and hands its trials over after
/// each (#2200), where a redo runs only the parts its file measured.
#[test]
fn a_run_of_named_cells_runs_every_part_of_a_whole_run() {
    // No scenario, so no cell: every part runs, over nothing.
    let (limited, limited_kept) = handed_over(Vec::new(), &["raw/foxhound"]);
    let (whole, whole_kept) = handed_over(Vec::new(), &[]);

    assert_eq!(
        (limited.map(|_| ()).ok(), limited_kept),
        (whole.map(|_| ()).ok(), whole_kept)
    );
}

/// A fresh run limited to the cells `--cell` names records them in its
/// trials file, so its report counts those cells' trials alone (#2742).
#[test]
fn a_run_of_named_cells_records_them_in_its_trials_file() {
    let (named, _) = handed_over(Vec::new(), &["raw/foxhound", "raw/spider"]);
    let (whole, _) = handed_over(Vec::new(), &[]);

    assert_eq!(
        (named.unwrap().declared.cells, whole.unwrap().declared.cells),
        (
            vec!["raw/foxhound".to_owned(), "raw/spider".to_owned()],
            Vec::new()
        )
    );
}

/// A run whose tool cannot start fails in the pre-flight, before its first
/// timed trial and before it hands over any trials (#2521: a Callgrind run
/// failed hours in, after 220 timed trials and 207 strace runs).
#[test]
fn a_run_whose_tool_cannot_start_fails_in_the_pre_flight() {
    // The hostile pages' first probe cannot start its Proving ground.
    let (ended, kept) = handed_over(vec![Scenario::Hostile], &[]);

    let error = format!("{:#}", ended.expect_err("the pre-flight fails"));
    assert!(
        error.starts_with("the pre-flight's run of hostile "),
        "{error}"
    );
    assert_eq!(kept, Vec::<[String; 5]>::new());
}

/// The sizes cell crawls with every tool's scenario 1 main row at 16 in
/// flight, where #2205 reads its bar against the best competitor, and with
/// Foxhound and Spider alone at 1 and 64, which carry no bar (#2871).
#[test]
fn the_sizes_cell_runs_every_tool_at_16_in_flight_and_two_at_1_and_64() {
    let options = Options {
        machine: None,
        command: "just compare release".to_owned(),
        trials: 1,
        growth: false,
        spider_pages: None,
        scenarios: vec![Scenario::Raw],
        callgrind_outputs: Path::new("/nowhere").to_owned(),
        only: Vec::new(),
    };

    let points = size_points(&options);
    let tools_at = |concurrency: u32, pages: u64| {
        let mut tools: Vec<Tool> = points
            .iter()
            .filter(|(_, _, crawl)| crawl.concurrency == concurrency && crawl.pages == pages)
            .map(|(_, config, _)| config.tool())
            .collect();
        tools.sort_unstable();
        tools
    };

    for pages in [100, 1_000, 10_000] {
        assert_eq!(
            (tools_at(16, pages), tools_at(1, pages), tools_at(64, pages)),
            (
                Tool::ALL.to_vec(),
                vec![Tool::Foxhound],
                vec![Tool::Foxhound, Tool::Spider]
            ),
            "at {pages} pages"
        );
    }
}

/// The sizes cell runs Spider where its crawl holds: not at 1 request in
/// flight, where Spider 2.53.9 stops short of the site, and at 64 in flight
/// on a page channel of 128, where the 64 its main row reads at loses pages;
/// Foxhound runs at every concurrency.
#[test]
fn the_sizes_cell_runs_spider_at_16_and_at_64_on_a_wider_channel() {
    let options = Options {
        machine: None,
        command: "just compare release".to_owned(),
        trials: 1,
        growth: false,
        spider_pages: None,
        scenarios: vec![Scenario::Raw],
        callgrind_outputs: Path::new("/nowhere").to_owned(),
        only: Vec::new(),
    };

    let mut points: Vec<(Tool, u32, Option<usize>)> = size_points(&options)
        .into_iter()
        .filter(|(_, config, crawl)| {
            crawl.pages == 100 && [Tool::Foxhound, Tool::Spider].contains(&config.tool())
        })
        .map(|(_, config, crawl)| {
            let capacity = match config {
                Config::Spider { capacity, .. } => Some(capacity),
                Config::Foxhound { .. }
                | Config::FoxhoundProgram
                | Config::Crawlee
                | Config::Scrapy
                | Config::CrawleeRs
                | Config::Pagesift
                | Config::OpenSpider
                | Config::FastCrw
                | Config::Crawlberg => None,
            };
            (config.tool(), crawl.concurrency, capacity)
        })
        .collect();
    points.sort_unstable_by_key(|&(tool, concurrency, _)| (tool.name(), concurrency));

    assert_eq!(
        points,
        vec![
            (Tool::Foxhound, 1, None),
            (Tool::Foxhound, 16, None),
            (Tool::Foxhound, 64, None),
            (Tool::Spider, 16, Some(64)),
            (Tool::Spider, 64, Some(128)),
        ]
    );
}

/// The Callgrind cell counts the cells its ratios read: in scenario 1
/// Foxhound's first cell, its cell at the catalog's defaults on the row
/// that does not send from the browser and Spider's main row; in the
/// extraction scenario both of Foxhound's rows and Spider's (#2175).
#[test]
fn the_callgrind_cell_counts_the_cells_its_ratios_read() {
    let measured: Vec<_> = matrix::measured().collect();

    let cells: Vec<(Scenario, &str)> = instruction_cells(&measured)
        .into_iter()
        .map(|(cell, _)| (cell.scenario, cell.name))
        .collect();

    assert_eq!(
        cells,
        vec![
            (Scenario::Raw, "foxhound"),
            (Scenario::Raw, "spider"),
            (Scenario::Raw, "foxhound defaults learned"),
            (Scenario::Extraction, "foxhound"),
            (Scenario::Extraction, matrix::FILTER_LISTS_OFF),
            (Scenario::Extraction, "spider"),
        ]
    );
}
