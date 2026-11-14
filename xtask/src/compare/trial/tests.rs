use std::path::Path;

use super::output::traced;
use super::{Mode, Plan, cell_of, instructions_of, invocation, system_calls_of};
use crate::compare::matrix::{self, Config, Scenario, SpiderBuild};
use crate::rss::crawl::Site;

fn spider(capacity: usize) -> Config {
    Config::Spider {
        build: SpiderBuild::Http,
        capacity,
    }
}

/// Foxhound and Spider run as their executables; Crawlee's script runs on
/// Node.js and Scrapy's on the Python environment beside it, so the tool's
/// process is the interpreter running the side, with no launcher between.
#[test]
fn each_tool_runs_as_its_executable_or_its_sides_interpreter() {
    let spider = invocation(spider(64), Path::new("/s/spider"));
    let crawlee = invocation(Config::Crawlee, Path::new("/c/crawl.mjs"));
    let scrapy = invocation(Config::Scrapy, Path::new("/p/crawl.py"));

    assert_eq!(
        (
            spider.program.as_path(),
            crawlee.program.file_name().and_then(|name| name.to_str()),
            crawlee.arguments,
            scrapy.program.as_path(),
            scrapy.arguments,
        ),
        (
            Path::new("/s/spider"),
            Some("node"),
            vec!["/c/crawl.mjs".into()],
            Path::new("/p/.venv/bin/python"),
            vec!["/p/crawl.py".into()],
        )
    );
}

/// crawlee-rs's side runs as its executable, with nothing but the
/// arguments every side takes.
#[test]
fn crawlee_rs_runs_as_its_executable() {
    let crawlee_rs = invocation(Config::CrawleeRs, Path::new("/r/crawlee-rs"));

    assert_eq!(
        (crawlee_rs.program.as_path(), crawlee_rs.arguments),
        (Path::new("/r/crawlee-rs"), Vec::new())
    );
}

/// pagesift's side runs as its executable, with nothing but the arguments
/// every side takes.
#[test]
fn pagesift_runs_as_its_executable() {
    let pagesift = invocation(Config::Pagesift, Path::new("/r/pagesift"));

    assert_eq!(
        (pagesift.program.as_path(), pagesift.arguments),
        (Path::new("/r/pagesift"), Vec::new())
    );
}

/// OpenSpider's side runs as its executable, with nothing but the
/// arguments every side takes.
#[test]
fn openspider_runs_as_its_executable() {
    let openspider = invocation(Config::OpenSpider, Path::new("/r/openspider"));

    assert_eq!(
        (openspider.program.as_path(), openspider.arguments),
        (Path::new("/r/openspider"), Vec::new())
    );
}

/// A Spider row's capacity reaches Spider's side: the row's own, so a
/// sensitivity row at another capacity runs the same executable with it.
#[test]
fn a_spider_row_gives_spiders_side_its_page_channels_capacity() {
    let arguments = |capacity| invocation(spider(capacity), Path::new("/s/spider")).arguments;

    assert_eq!(
        (arguments(64), arguments(4096)),
        (
            vec!["--capacity".into(), "64".into()],
            vec!["--capacity".into(), "4096".into()],
        )
    );
}

/// On the article site Spider's side reads each page in a task of its own,
/// and strace and Callgrind slow those tasks unevenly: at the row's
/// capacity, 5 of 20 crawls of the 1,000-page site lost pages under strace
/// (#2136). A counted run there gives the channel the whole site, where no
/// page can be lost, and reads no memory; a timed trial keeps the row's
/// capacity, and so does a counted run of a scenario that reads no page.
#[test]
fn a_counted_run_on_the_article_site_gives_spiders_channel_the_whole_site() {
    let cell = |scenario| matrix::cell(scenario, "spider").unwrap();
    let capacity = |scenario, capacity, mode: Mode| {
        let Config::Spider { capacity, .. } = traced(cell(scenario), spider(capacity), &mode, 1000)
        else {
            panic!("Spider's configuration stays Spider's");
        };
        capacity
    };

    assert_eq!(
        [
            capacity(Scenario::Extraction, 64, Mode::Timed),
            capacity(Scenario::Extraction, 64, Mode::Syscalls("counts".into())),
            capacity(Scenario::Extraction, 64, Mode::Callgrind("counts".into())),
            capacity(Scenario::Extraction, 4096, Mode::Syscalls("counts".into())),
            capacity(Scenario::Raw, 64, Mode::Syscalls("counts".into())),
        ],
        [64, 1000, 1000, 4096, 64]
    );
}

/// A crawl-like trial of `cell` (#2284) trusting the authority at
/// `authority`.
fn crawl_like<'a>(cell: &str, authority: &'a Path) -> Plan<'a> {
    Plan {
        certificate_authority: Some(authority),
        ..trial_of(Scenario::CrawlLike, cell, &[])
    }
}

/// A trial of `scenario`'s `cell` whose Foxhound runs with `settings`.
fn trial_of<'a>(scenario: Scenario, cell: &str, settings: &'a [String]) -> Plan<'a> {
    let (cell, config) = cell_of(scenario, cell).unwrap();
    Plan {
        cell,
        config,
        executable: Path::new("/f/goodfetch"),
        site: Site {
            address: "127.0.0.1:4000",
            pages: 1000,
            concurrency: 16,
        },
        mode: Mode::Timed,
        goodfetch_variables: &[],
        browser: None,
        executable_pages: false,
        certificate_authority: None,
        foxhound_settings: settings,
    }
}

/// The user configuration a trial wrote in `home`, if any.
fn configuration_in(home: &Path) -> Option<toml::Table> {
    let version =
        goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let file = goodfetch_protocol::home::per_major(home, &version)
        .join(goodfetch_protocol::home::USER_CONFIGURATION_FILE);
    let text = std::fs::read_to_string(file).ok()?;
    Some(toml::from_str(&text).unwrap())
}

/// The settings a run gives Foxhound (`--foxhound-setting`, #2200) go into
/// its trials' user configuration, served over TLS or not; another tool's
/// home holds none, and a trial without them writes no configuration.
#[test]
fn a_trials_foxhound_runs_with_the_settings_the_run_gives_it() {
    let settings = [
        "governor.worker_threads = \"per_core\"".to_owned(),
        "governor.blocking_threads = 512".to_owned(),
    ];
    let homes: Vec<tempfile::TempDir> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let authority = Path::new("/tmp/authority.pem");

    trial_of(Scenario::Raw, "foxhound", &settings)
        .configure(homes[0].path())
        .unwrap();
    Plan {
        certificate_authority: Some(authority),
        ..trial_of(Scenario::CrawlLike, "foxhound", &settings)
    }
    .configure(homes[1].path())
    .unwrap();
    trial_of(Scenario::Raw, "scrapy", &settings)
        .configure(homes[2].path())
        .unwrap();
    trial_of(Scenario::Raw, "foxhound", &[])
        .configure(homes[3].path())
        .unwrap();

    let governor = |home: &tempfile::TempDir| {
        configuration_in(home.path()).map(|configuration| {
            (
                configuration["settings"]["governor"].clone(),
                configuration["settings"].get("tls").is_some(),
            )
        })
    };
    let expected: toml::Value =
        toml::from_str::<toml::Table>("worker_threads = \"per_core\"\nblocking_threads = 512")
            .unwrap()
            .into();
    assert_eq!(
        homes.iter().map(governor).collect::<Vec<_>>(),
        [
            Some((expected.clone(), false)),
            Some((expected, true)),
            None,
            None
        ]
    );
}

/// A crawl-like trial's Foxhound trusts the Proving ground's authority for
/// the article site's host from its user configuration, the one place
/// `tls.trusted_cas` is taken from; another tool's home holds none.
#[test]
fn a_crawl_like_trials_foxhound_trusts_the_authority_from_its_user_configuration() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let authority = Path::new("/tmp/proving ground/\"authority\".pem");

    crawl_like("foxhound", authority)
        .configure(home.path())
        .unwrap();
    crawl_like("scrapy", authority)
        .configure(other.path())
        .unwrap();

    let version =
        goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let file = goodfetch_protocol::home::per_major(home.path(), &version)
        .join(goodfetch_protocol::home::USER_CONFIGURATION_FILE);
    let configuration: toml::Table =
        toml::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(
        (
            configuration["version"].as_str(),
            configuration["settings"]["tls"]["trusted_cas"]
                .as_table()
                .map(|trusted| trusted.keys().cloned().collect::<Vec<_>>()),
            configuration["settings"]["tls"]["trusted_cas"]["article.localhost"][0].as_str(),
            std::fs::read_dir(other.path()).unwrap().count(),
        ),
        (
            Some("0.1"),
            Some(vec!["article.localhost".to_owned()]),
            authority.to_str(),
            0,
        )
    );
}

/// The shape Callgrind 3.26 writes: a header, each function's costs, then the
/// total of the one event it counted.
const OUTPUT: &str = "# callgrind format\n\
    version: 1\n\
    creator: callgrind-3.26.0\n\
    pid: 7\n\
    cmd:  ./tool\n\
    positions: line\n\
    events: Ir\n\
    summary: 1234567\n\
    \n\
    fn=(1) main\n\
    0 100\n\
    \n\
    totals: 1234567\n";

#[test]
fn the_total_is_read_from_callgrinds_totals_line() {
    assert_eq!(instructions_of(OUTPUT).unwrap(), 1_234_567);
}

#[test]
fn an_output_with_no_total_is_an_error() {
    assert!(instructions_of("events: Ir\nfn=(1) main\n").is_err());
}

/// The shape `strace -f -c -U calls,name` writes: a header, one row a
/// system call, then the total.
const COUNTS: &str = "    calls syscall\n\
    --------- ----------------\n\
          7 fsync\n\
          2 fdatasync\n\
        300 write\n\
          1 sync_file_range\n\
    --------- ----------------\n\
        310 total\n";

#[test]
fn strace_counts_give_the_total_and_the_syncs() {
    assert_eq!(system_calls_of(COUNTS).unwrap(), (310, 10));
}

#[test]
fn counts_with_no_total_are_an_error() {
    assert!(system_calls_of("    calls syscall\n          7 fsync\n").is_err());
}

/// A counting tool is started in its tool's own folder (a tool other than
/// Foxhound runs in its data folder), so the file it writes its count to is
/// named from the root: `just compare` names Callgrind's outputs under
/// `./target/compare/callgrind/`, and Callgrind could not open that from
/// another folder (#2281).
#[test]
fn a_counted_trials_output_file_is_named_from_the_root() {
    let output = super::counting_output(Some(std::path::Path::new(
        "./target/compare/callgrind/raw-spider-1.out",
    )))
    .unwrap();

    assert!(output.is_absolute(), "{}", output.display());
    assert!(
        output.ends_with("target/compare/callgrind/raw-spider-1.out"),
        "{}",
        output.display()
    );
    assert!(super::counting_output(None).is_err());
}
