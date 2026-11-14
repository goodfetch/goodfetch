use super::{CELLS, Config, Network, OutputCheck, Row, Scenario, Site, SpiderBuild, Status, Tool};

/// #2316's scenarios: the realistic-page site, the long site and scenario
/// 1 at 64 and 256 in flight. Each gives every tool a main cell with its
/// mapping, and the cell at the catalog's defaults (#2175) stands at each
/// of the sweep's three points, 16, 64 and 256 in flight, pending until
/// #2175 brings it.
#[test]
fn the_realistic_long_and_sweep_scenarios_map_every_tool() {
    let added = [
        Scenario::Realistic,
        Scenario::Long,
        Scenario::Raw64,
        Scenario::Raw256,
    ];
    let mapped = |scenario: Scenario| -> Vec<Tool> {
        let mut tools: Vec<Tool> = CELLS
            .iter()
            .filter(|cell| {
                cell.scenario == scenario
                    && cell.row == Row::Main
                    && matches!(cell.status, Status::Measured(_))
                    && !cell.mapping.is_empty()
            })
            .map(|cell| cell.tool)
            .collect();
        tools.dedup();
        tools
    };
    let defaults: Vec<(Scenario, u32, bool)> = CELLS
        .iter()
        .filter(|cell| cell.name == "foxhound defaults")
        .map(|cell| {
            (
                cell.scenario,
                cell.scenario.concurrency(),
                matches!(cell.status, Status::Pending(reason) if reason.contains("#2175")),
            )
        })
        .collect();

    assert_eq!(
        (
            added.map(mapped),
            added.map(Scenario::site),
            added.map(|scenario| (scenario.concurrency(), scenario.site_pages(1000))),
            defaults,
        ),
        (
            [
                Tool::ALL.to_vec(),
                Tool::ALL.to_vec(),
                Tool::ALL.to_vec(),
                Tool::ALL.to_vec()
            ],
            [Site::Realistic, Site::Scale, Site::Scale, Site::Scale],
            [(16, 1000), (16, 100_000), (64, 1000), (256, 1000)],
            vec![
                (Scenario::Raw, 16, true),
                (Scenario::Raw64, 64, true),
                (Scenario::Raw256, 256, true),
            ],
        )
    );
}

/// Each scenario added to the matrix's own five runs scenario 1's settings,
/// so the sides take it as `raw`, but the hostile pages' pass at defaults,
/// which runs each tool's own; each carries no bar until the owner sets one
/// (#2252 Q11, #2439 Q3); the matrix's own five are unchanged.
#[test]
fn the_added_scenarios_run_scenario_1s_settings_but_the_pass_at_defaults() {
    let read = |scenario: Scenario| (scenario.settings(), scenario.no_bar().is_some());

    assert_eq!(
        Scenario::ALL.map(read),
        [
            (Scenario::Raw, false),
            (Scenario::Durable, false),
            (Scenario::Polite, false),
            (Scenario::Extraction, false),
            (Scenario::Browser, false),
            (Scenario::Raw, true),
            (Scenario::Raw, true),
            (Scenario::Raw, true),
            (Scenario::Raw, true),
            (Scenario::Hostile, true),
            (Scenario::Raw, true),
            (Scenario::Raw, true),
        ]
    );
}

/// The hostile pages' two passes (#2439): each lists every tool's main
/// cell with its mapping, crawls the Proving ground's `traps` family, and
/// the `limits` family's first and third tasks at defaults only, and
/// carries no bar until the owner sets one (#2439, Q3). The pass at
/// defaults runs each tool's own settings, which each side takes as
/// `hostile`; the other, scenario 1's, 16 in flight and no delay.
#[test]
fn the_hostile_pages_passes_map_every_tool_and_carry_no_bar() {
    let passes = [Scenario::Hostile, Scenario::Hostile16];
    let mapped = |scenario: Scenario| -> Vec<Tool> {
        CELLS
            .iter()
            .filter(|cell| {
                cell.scenario == scenario
                    && cell.row == Row::Main
                    && matches!(cell.status, Status::Measured(_))
                    && !cell.mapping.is_empty()
            })
            .map(|cell| cell.tool)
            .collect()
    };
    let tasks = |scenario: Scenario| -> Vec<&str> {
        scenario
            .hostile_tasks()
            .iter()
            .map(|task| task.name)
            .collect()
    };

    assert_eq!(
        passes.map(|scenario| (
            mapped(scenario),
            tasks(scenario),
            scenario.settings(),
            scenario.no_bar().is_some(),
        )),
        [
            (
                Tool::ALL.to_vec(),
                vec!["traps/1", "limits/1", "limits/3"],
                Scenario::Hostile,
                true
            ),
            (Tool::ALL.to_vec(), vec!["traps/1"], Scenario::Raw, true),
        ]
    );
}

/// Spider's measured rows of `scenario`: each row's name, what it is read
/// for, its build and its page channel's capacity.
fn spider_rows(scenario: Scenario) -> Vec<(&'static str, Row, SpiderBuild, usize)> {
    CELLS
        .iter()
        .filter(|cell| cell.scenario == scenario)
        .filter_map(|cell| match cell.status {
            Status::Measured(Config::Spider { build, capacity }) => {
                Some((cell.name, cell.row, build, capacity))
            }
            Status::Measured(_) | Status::NotSupported(_) | Status::Pending(_) => None,
        })
        .collect()
}

/// Spider's main row of each scenario it runs reads its pages at the
/// smallest capacity that lost none there, 64, or 256 where the reader
/// also writes each page; the rows beside it are its sensitivity rows, each
/// one setting apart (#2213): the capacity before, Spider's default
/// features, `bloom` and the allocator its README recommends, jemalloc for
/// the pure crawl and mimalloc where each page is also read. The
/// extraction scenario's rows run the builds that read each page's Main
/// content (#2136). Its `smart` row, in the headless browser scenario, is
/// not supported: it has its own test.
#[test]
fn spiders_main_row_has_its_sensitivity_rows_beside_it() {
    use Row::{Main, Sensitivity};
    use SpiderBuild::{Bloom, Chrome, Defaults, Extraction, ExtractionMimalloc, Http, Jemalloc};

    assert_eq!(
        (
            spider_rows(Scenario::Raw),
            spider_rows(Scenario::Extraction),
            spider_rows(Scenario::Browser),
        ),
        (
            vec![
                ("spider", Main, Http, 64),
                ("spider capacity 4096", Sensitivity, Http, 4096),
                ("spider default features", Sensitivity, Defaults, 64),
                ("spider bloom", Sensitivity, Bloom, 64),
                ("spider jemalloc", Sensitivity, Jemalloc, 64),
            ],
            vec![
                ("spider", Main, Extraction, 64),
                ("spider mimalloc", Sensitivity, ExtractionMimalloc, 64),
            ],
            vec![("spider", Main, Chrome, 64)],
        )
    );
}

/// Spider's `smart` feature does not do the headless browser scenario's
/// work, so its sensitivity row stands in the matrix, labelled, with the
/// reason it has no figures.
#[test]
fn spiders_smart_row_is_a_sensitivity_row_that_is_not_supported() {
    let smart: Vec<(Scenario, Row, bool)> = CELLS
        .iter()
        .filter(|cell| cell.name == "spider smart")
        .map(|cell| {
            (
                cell.scenario,
                cell.row,
                matches!(cell.status, Status::NotSupported(reason) if reason.contains("`crawl_smart()` fetched 1 of the site's 100 pages")),
            )
        })
        .collect();

    assert_eq!(smart, vec![(Scenario::Browser, Row::Sensitivity, true)]);
}

/// The report writes each row's mapping, so a Spider row's names the
/// capacity it ran at: the figure a reader weighs its memory against.
#[test]
fn each_spider_rows_mapping_names_its_capacity() {
    let unnamed: Vec<&str> = CELLS
        .iter()
        .filter_map(|cell| match cell.status {
            Status::Measured(Config::Spider { capacity, .. })
                if !cell.mapping.contains(&format!("`subscribe({capacity})`")) =>
            {
                Some(cell.name)
            }
            Status::Measured(_) | Status::NotSupported(_) | Status::Pending(_) => None,
        })
        .collect();

    assert_eq!(unnamed, Vec::<&str>::new());
}

/// A sensitivity row varies a main row: each sits in a scenario where its
/// tool has one, so its figures always have the row they are read against.
#[test]
fn each_sensitivity_row_has_its_tools_main_row_in_its_scenario() {
    let alone: Vec<(&str, &str)> = CELLS
        .iter()
        .filter(|cell| cell.row == Row::Sensitivity)
        .filter(|cell| {
            !CELLS.iter().any(|main| {
                main.scenario == cell.scenario
                    && main.tool == cell.tool
                    && main.row == Row::Main
                    && matches!(main.status, Status::Measured(_))
            })
        })
        .map(|cell| (cell.scenario.name(), cell.name))
        .collect();

    assert_eq!(alone, Vec::<(&str, &str)>::new());
}

/// `just compare` builds every build a Spider row runs, with its feature,
/// into the folder the run reads it from, and Spider's side declares that
/// feature: a build added here alone would have no executable to run.
#[test]
fn just_compare_builds_each_of_spiders_sides_builds_into_its_folder() {
    let recipes = include_str!("../../../../justfile");
    let manifest: toml::Table = include_str!("../../../compare-spider/Cargo.toml")
        .parse()
        .unwrap();
    let builds = CELLS.iter().filter_map(|cell| match cell.status {
        Status::Measured(Config::Spider { build, .. }) => Some(build),
        Status::Measured(_) | Status::NotSupported(_) | Status::Pending(_) => None,
    });
    let mut missing: Vec<String> = builds
        .filter(|build| {
            let features = build
                .feature()
                .map_or_else(String::new, |feature| format!(" --features {feature}"));
            let built = recipes.contains(&format!(
                "cargo build --quiet --locked --manifest-path xtask/compare-spider/Cargo.toml \
                 --profile {{{{ profile }}}}{features} --target-dir {{{{ compare_spider_builds }}}}/{}\n",
                build.folder()
            ));
            let declared = build.feature().is_none_or(|features| {
                features
                    .split(',')
                    .all(|feature| manifest["features"].get(feature).is_some())
            });
            !(built && declared)
        })
        .map(SpiderBuild::folder)
        .collect();
    missing.sort();
    missing.dedup();

    assert_eq!(missing, Vec::<String>::new());
}

/// The extraction scenario crawls the article site (#2136), which every
/// tool reaches by its name, and each cell's mapping names what reads its
/// Main content: Foxhound's Clean-up rules and `dom_smoothie`, Spider's
/// `llm_readability` at the versions its side pins, pagesift's
/// `onlyMainContent`, and the text of `<main>` for the tools that read no
/// main content of their own.
#[test]
fn the_extraction_scenario_crawls_the_article_site_and_names_each_extractor() {
    let manifest: toml::Table = include_str!("../../../compare-spider/Cargo.toml")
        .parse()
        .unwrap();
    let pinned = |name: &str| {
        manifest["dependencies"][name]["version"]
            .as_str()
            .unwrap()
            .trim_start_matches('=')
            .to_owned()
    };
    let spider = [
        format!("`llm_readability` {}", pinned("llm_readability")),
        format!(
            "`spider_transformations` {}",
            pinned("spider_transformations")
        ),
    ];
    let extractors: [(&str, &[&str]); 8] = [
        ("foxhound", &["Clean-up rules", "dom_smoothie"]),
        (
            "foxhound filter lists off",
            &["`filter_lists.ads`", "dom_smoothie"],
        ),
        ("spider", &[&spider[0], &spider[1]]),
        ("crawlee", &["the text of its `main`"]),
        ("scrapy", &["the text of its `main`"]),
        ("crawlee-rs", &["the text of its `main`"]),
        ("pagesift", &["`onlyMainContent`"]),
        ("spider mimalloc", &["the spider row's settings"]),
    ];
    let unnamed: Vec<(&str, &str)> = extractors
        .iter()
        .flat_map(|(name, named)| {
            let mapping = super::cell(Scenario::Extraction, name).map_or("", |cell| cell.mapping);
            named
                .iter()
                .filter(move |text| !mapping.contains(**text))
                .map(move |text| (*name, *text))
        })
        .collect();
    let cells = CELLS
        .iter()
        .filter(|cell| cell.scenario == Scenario::Extraction)
        .count();

    assert_eq!(
        (
            Scenario::Extraction.site(),
            Site::Article.by_name(),
            Site::Article.hosts(),
            cells,
            unnamed,
        ),
        (
            Site::Article,
            true,
            vec!["article.localhost".to_owned()],
            extractors.len(),
            Vec::new(),
        )
    );
}

/// Foxhound's row with its Filter lists off (#2136) is a main row of the
/// extraction scenario, where nothing removes the page's ad: its pages are
/// held to the right answer with the ad allowed, every other cell of the
/// scenario to the whole answer, and no other scenario's cell to any.
#[test]
fn foxhounds_filter_lists_off_row_is_checked_with_the_ad_allowed() {
    let checks: Vec<(&str, bool, Option<OutputCheck>)> = CELLS
        .iter()
        .filter(|cell| cell.scenario == Scenario::Extraction)
        .map(|cell| (cell.name, cell.filter_lists_off(), cell.output_check()))
        .collect();
    let lists_off = super::cell(Scenario::Extraction, "foxhound filter lists off").unwrap();
    let elsewhere = CELLS
        .iter()
        .filter(|cell| cell.scenario != Scenario::Extraction)
        .filter(|cell| cell.filter_lists_off() || cell.output_check().is_some())
        .count();

    let whole = Some(OutputCheck::Whole);
    assert_eq!(
        (
            checks,
            lists_off.row,
            matches!(lists_off.status, Status::Measured(Config::Foxhound { .. })),
            elsewhere,
        ),
        (
            vec![
                ("foxhound", false, whole),
                (
                    "foxhound filter lists off",
                    true,
                    Some(OutputCheck::AdAllowed)
                ),
                ("spider", false, whole),
                ("crawlee", false, whole),
                ("scrapy", false, whole),
                ("crawlee-rs", false, whole),
                ("pagesift", false, whole),
                ("spider mimalloc", false, whole),
            ],
            Row::Main,
            true,
            0,
        )
    );
}

/// crawlee-rs runs on its side in the four HTTP scenarios and in #2316's
/// four, each its main row, and its headless browser cell is not supported
/// (#2214).
#[test]
fn crawlee_rs_runs_the_http_scenarios_and_not_the_browser() {
    let cells: Vec<(Scenario, &str, Row, Option<Config>)> = CELLS
        .iter()
        .filter(|cell| cell.tool == Tool::CrawleeRs)
        .map(|cell| {
            let config = match cell.status {
                Status::Measured(config) => Some(config),
                Status::NotSupported(_) | Status::Pending(_) => None,
            };
            (cell.scenario, cell.name, cell.row, config)
        })
        .collect();

    let measured = Some(Config::CrawleeRs);
    assert_eq!(
        cells,
        [
            (Scenario::Raw, measured),
            (Scenario::Durable, measured),
            (Scenario::Polite, measured),
            (Scenario::Extraction, measured),
            (Scenario::Browser, None),
            (Scenario::Realistic, measured),
            (Scenario::Long, measured),
            (Scenario::Raw64, measured),
            (Scenario::Raw256, measured),
            (Scenario::Hostile, measured),
            (Scenario::Hostile16, measured),
            (Scenario::CrawlLike, measured),
        ]
        .map(|(scenario, config)| (scenario, "crawlee-rs", Row::Main, config))
    );
}

/// `just compare` builds crawlee-rs's side, locked, at the run's profile,
/// into the folder the run reads its executable from.
#[test]
fn just_compare_builds_crawlee_rss_side_into_its_folder() {
    let recipes = include_str!("../../../../justfile");

    assert!(recipes.contains(&format!(
        "cargo build --quiet --locked --manifest-path xtask/compare-crawlee-rs/Cargo.toml \
         --profile {{{{ profile }}}} --target-dir {{{{ compare_spider_builds }}}}/{}\n",
        super::CRAWLEE_RS_FOLDER
    )));
}

/// pagesift runs on its side in every scenario, each its main row: the
/// four HTTP scenarios, the headless browser scenario on its browser
/// engine, #2316's four and the hostile pages' two (#2215).
#[test]
fn pagesift_runs_every_scenario_on_its_side() {
    let cells: Vec<(Scenario, &str, Row, Option<Config>)> = CELLS
        .iter()
        .filter(|cell| cell.tool == Tool::Pagesift)
        .map(|cell| {
            let config = match cell.status {
                Status::Measured(config) => Some(config),
                Status::NotSupported(_) | Status::Pending(_) => None,
            };
            (cell.scenario, cell.name, cell.row, config)
        })
        .collect();

    assert_eq!(
        cells,
        Scenario::ALL.map(|scenario| (scenario, "pagesift", Row::Main, Some(Config::Pagesift)))
    );
}

/// `just compare` builds pagesift's side, locked, at the run's profile,
/// into the folder the run reads its executable from.
#[test]
fn just_compare_builds_pagesifts_side_into_its_folder() {
    let recipes = include_str!("../../../../justfile");

    assert!(recipes.contains(&format!(
        "cargo build --quiet --locked --manifest-path xtask/compare-pagesift/Cargo.toml \
         --profile {{{{ profile }}}} --target-dir {{{{ compare_spider_builds }}}}/{}\n",
        super::PAGESIFT_FOLDER
    )));
}

/// The polite scenario crawls 10,000 pages on 1,000 domains, 10 on each,
/// where the others crawl the comparison's 1,000 (#2283) and the long one
/// its own 100,000, and the growth cell's 100,000 there too, 100 on each
/// domain; each page on the domain the Proving ground serves it on.
#[test]
fn the_polite_scenario_crawls_ten_pages_on_each_of_a_thousand_domains() {
    let sizes = |pages| Scenario::ALL.map(|scenario| scenario.site_pages(pages));
    let site = Site::MultiDomain;

    assert_eq!(
        (
            sizes(1000),
            sizes(100_000),
            site.hosts().len(),
            [1, 1000, 1001, 10_000].map(|page| site.host_of(page)),
        ),
        (
            [
                1000, 1000, 10_000, 1000, 1000, 1000, 100_000, 1000, 1000, 1000, 1000, 1000
            ],
            [100_000; 12],
            1000,
            [
                "polite-1.localhost",
                "polite-1000.localhost",
                "polite-1.localhost",
                "polite-1000.localhost",
            ]
            .map(str::to_owned),
        )
    );
}

/// The crawl-like scenario (#2284): every tool's main cell, with its
/// mapping, crawls the article site at scenario 1's settings, over TLS and
/// HTTP/2 under the Proving ground's `rtt-20` profile, and carries no bar;
/// no output check holds its pages, and every other scenario stays on
/// loopback.
#[test]
fn the_crawl_like_scenario_maps_every_tool_over_tls_under_rtt_20() {
    let crawl_like = Scenario::CrawlLike;
    let mapped: Vec<(Tool, bool)> = CELLS
        .iter()
        .filter(|cell| {
            cell.scenario == crawl_like
                && cell.row == Row::Main
                && matches!(cell.status, Status::Measured(_))
        })
        .map(|cell| (cell.tool, cell.output_check().is_some()))
        .collect();
    let elsewhere: Vec<Scenario> = Scenario::ALL
        .into_iter()
        .filter(|scenario| scenario.network() != Network::Loopback)
        .collect();
    let network = crawl_like.network();

    assert_eq!(
        (
            mapped,
            crawl_like.name(),
            crawl_like.site(),
            crawl_like.settings(),
            crawl_like.no_bar().is_some(),
            crawl_like.concurrency(),
            elsewhere,
            (network.network_profile(), network.scheme(), network.tls()),
        ),
        (
            Tool::ALL.map(|tool| (tool, false)).to_vec(),
            "crawl-like",
            Site::Article,
            Scenario::Raw,
            true,
            16,
            vec![crawl_like],
            ("rtt-20", "https", true),
        )
    );
}
