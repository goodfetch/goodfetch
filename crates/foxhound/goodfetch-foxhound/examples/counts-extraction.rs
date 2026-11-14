//! B4's counts cells (ADR 0140's note of 2026-11-14, spec #2164): the
//! extraction path's work per page. Each cell runs a code-free crawl with
//! `formats: { text: true }` and the field `title`, on the engine and
//! through the driver `benches/scenarios.rs` gives B1, over the Proving
//! ground's article site served from memory, and prints what it counted as
//! one JSON line, which `xtask counts` divides and holds to the Quality
//! register's limits.
//!
//! The site is the fixture `benches/fixtures/article-site-16.html`: the 16
//! documents of the article site of 16 pages, which the Proving ground's
//! own test holds equal to what it serves (#2136's page: a header, a nav,
//! an article of a few paragraphs with one ad slot a default Filter list's
//! cosmetic rule matches, an aside and a footer). No benchmark links the
//! Proving ground, so the pages are served from here.
//!
//! The cells:
//!
//! - `default`: the crawl at its defaults, from the site's first page,
//!   every link followed, on one host;
//! - `filter_lists_off`: `filter_lists.ads`, `.trackers` and `.annoyances`
//!   off, so no rule list is built and no rule tested, and the ad stays;
//! - `host_cache_off`: `filter_lists.host_cache: 0`, so each page builds
//!   its own rule list;
//! - `single_attempt`: `extraction.readability: single_attempt`, so the
//!   main-content step tries each page once; it runs the step as often as
//!   `default` does, which the example checks;
//! - `many_hosts`: the same pages, one on each of [`HOSTS`] hosts, each a
//!   seed and no link followed, since a host enters the host cache only
//!   through a page: more hosts than the cache's default bound.
//!
//! The rows are those B4 holds: `F1.rule_list_builds`, `F1.rule_tests`,
//! `F1.analysis_handoffs` and `F1.parses` per page, and
//! `F1.host_cache_entries` and `F1.host_cache_bytes` per crawl, read as the
//! crawl ended. A code-free crawl that asks for a format reads its formats
//! ahead in its page's one hand-off, on a copy of the tree its untouched
//! readers parsed (#2189): one parse a page. It prints no
//! `F1.readability_runs`, 2 a page here, the formats' main-content step on
//! the cleaned tree beside the Page signature's on the tree as served (ADR
//! 0138), which stays B1's row; and no `F1.tree_copies`, 1 a page here, the
//! read ahead's copy, as the landmarks fallback reads the tree its built-in
//! kept (#2639), but 2 in `single_attempt`, whose step reads the tree in
//! place, so the fallback reads a copy taken before it: a figure the
//! register's row does not hold on B4 yet.
//!
//! `F1.clean_up_instructions` is the instructions Callgrind counts in the
//! clean-up's removal and whatever it calls, on the `default` cell, for
//! which this example runs again as `--callgrind default` under Callgrind:
//! the base of spec #2164's 40 % criterion. That run crawls the site twice,
//! on engines sharing one cache of compilations, and Callgrind counts the
//! second crawl alone, so the rules' selectors, compiled once per engine,
//! are no page's instructions. It is the figure of this
//! example's build, the `dev` profile, whose parsing, selector matching and
//! Filter-list engine are optimized (`Cargo.toml`); `just compare`'s
//! Callgrind cell reads the release build's whole page. Without Valgrind
//! it goes unprinted, which `xtask counts` fails as a row not counted.
//!
//! `F1.selector_compilation_instructions` is what that first crawl left
//! out: the instructions Callgrind counts in the compilation of each
//! selector, the one counted function every compilation goes through
//! (`CountedWork::compile_selector`), over one crawl of the `default` cell
//! on an engine whose cache of compilations starts empty, for which this
//! example runs again as `--callgrind-compilations default`. A crawl on
//! an engine of its own runs first and goes uncounted: what the process
//! sets up once, such as the table of HTML names a first compilation
//! would otherwise fill, is no crawl's compilation. It is paid
//! once per engine and rules version whatever the pages that follow (ADR
//! 0138), so it is read per crawl: the Clean-up rules' selectors and the
//! engine's own, compiled as the crawl starts. The cache's insert of each
//! is not in it, its cost varying from run to run with its hasher's seed.
//!
//! `F1.generic_set_instructions` is the other work that crawl leaves out:
//! the instructions Callgrind counts in the build of the version's generic
//! set (`Generic::of`), which the Filter lists' load does once per version
//! (ADR 0138, issue 2191), not a page's removal. A third run of this example
//! under Callgrind reads it, as `--callgrind default` again, over the same
//! counted crawl, whose engine loads its own Filter lists: per crawl.
//!
//! `--plant <row>` does one extra unit of that row's work per page served,
//! in every cell: a count more, for `F1.clean_up_instructions` one more
//! removal of the page served, for `F1.selector_compilation_instructions`
//! [`PLANTED_SELECTORS`] selectors more compiled, and for
//! `F1.generic_set_instructions` one more load of the embedded Filter lists
//! in the crawl. The crawl fails here if it read a page's text wrong, if
//! the work it counts did not happen, or if the Filter lists off did not
//! take their work away.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stderr,
    reason = "the counts harness (ADR 0140): why a row goes unprinted, which `xtask counts` fails, beside its finding"
)]
#![expect(
    clippy::print_stdout,
    reason = "the counts harness (ADR 0140): the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/scenarios.rs"]
#[expect(
    dead_code,
    reason = "B1's site and crawl, which its own benchmarks run and this one does not"
)]
mod scenarios;

use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use goodfetch_clean_up_rules::matching::Choices;
use goodfetch_clean_up_rules::removal::{self, HostCache, Rules};
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use goodfetch_fetch_tier::{Body, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_test_support::callgrind;
use scenarios::{AfterStart, Driven, Plant, drive};
use serde_json::{Map, Value, json};

/// The article site of 16 pages, its documents in page order.
const SITE: &str = include_str!("../benches/fixtures/article-site-16.html");

/// The end of each of [`SITE`]'s documents.
const DOCUMENT_END: &str = "</html>\n";

/// The host of the one-host cells.
const HOST: &str = "article.test";

/// The hosts of the `many_hosts` cell, one page each: twice the host
/// cache's default bound of 1,024 (spec #2164), so the cell reaches it.
/// Their names are of one length, so whichever the cache keeps, it holds
/// the same bytes.
const HOSTS: usize = 2_048;

/// The ad's marker, which the right answer of a page cleaned at the
/// defaults does not hold (`proving-ground`'s article site).
const AD: &str = "Sponsored offer from Harbourline Freight";

/// The rows B4 holds per page.
const PER_PAGE: [Row; 4] = [
    Row::PageRuleListBuilds,
    Row::PageRuleTests,
    Row::PageAnalysisHandoffs,
    Row::PageParses,
];

/// The rows B4 holds per crawl, read as the crawl ended.
const PER_CRAWL: [Row; 2] = [Row::PageHostCacheEntries, Row::PageHostCacheBytes];

/// The clean-up's instructions, and the function Callgrind counts them
/// beneath, named as Callgrind names it.
const INSTRUCTIONS: &str = "F1.clean_up_instructions";
const REMOVAL: &str = "goodfetch_clean_up_rules::removal::remove";

/// The selectors' compilations' instructions, and the function Callgrind
/// counts them beneath, named as Callgrind names it.
const COMPILATIONS: &str = "F1.selector_compilation_instructions";
const COMPILE: &str = "<goodfetch_counted_work::CountedWork>::compile_selector";

/// The generic set's build's instructions, and the function Callgrind
/// counts them beneath, named as Callgrind names it.
const GENERIC_SET: &str = "F1.generic_set_instructions";
const GENERIC_BUILD: &str = "<goodfetch_clean_up_rules::generic::Generic>::of";

/// How many selectors the plant of [`COMPILATIONS`] compiles per page:
/// enough for its instructions to clear the row's tolerance.
const PLANTED_SELECTORS: usize = 64;

/// The cell whose crawl runs again under Callgrind.
const DEFAULT: &str = "default";

/// The cell whose main-content step tries each page once.
const SINGLE_ATTEMPT: &str = "single_attempt";

/// The pages of [`SITE`].
fn pages() -> Vec<&'static str> {
    SITE.split_inclusive(DOCUMENT_END).collect()
}

/// The site, served from memory: the page `/sites/16/<n>` on any host, and
/// nothing else, so each host's robots.txt is answered 404.
struct Site {
    pages: Vec<&'static str>,
    /// Run once per page it serves: the benchmark's own extra unit of work.
    plant: Option<Plant>,
}

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let page = hop
            .url
            .path()
            .strip_prefix("/sites/16/")
            .and_then(|number| number.parse::<usize>().ok())
            .and_then(|number| self.pages.get(number.checked_sub(1)?))
            .copied();
        if let (Some(plant), Some(page)) = (&self.plant, page) {
            plant(page);
        }
        let (status, content_type) = match page {
            Some(_) => (200, "text/html; charset=utf-8"),
            None => (404, "text/plain"),
        };
        let body = page.unwrap_or_default();
        Box::pin(std::future::ready(Ok(Sent {
            status,
            headers: vec![
                ("content-type".to_owned(), content_type.to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
            ],
            body: Box::new(Whole(Some(Bytes::from_static(body.as_bytes())))),
            reused: false,
            decoded: false,
            certificate_exception: None,
        })))
    }
}

/// A page's body, arriving in one chunk borrowed from the fixture.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let chunk = self.0.take().filter(|chunk| !chunk.is_empty());
        Box::pin(std::future::ready(Ok(chunk)))
    }
}

/// One cell: its name, the settings it runs with beside the crawl's own,
/// whether its pages are spread one a host, whether its Filter lists are
/// off, so it builds no rule list and keeps the ad, and whether its host
/// cache is, so each page builds its own.
struct Cell {
    name: &'static str,
    settings: Value,
    many_hosts: bool,
    filter_lists_off: bool,
    host_cache_off: bool,
}

fn cells() -> [Cell; 5] {
    let cell = |name, settings| Cell {
        name,
        settings,
        many_hosts: false,
        filter_lists_off: false,
        host_cache_off: false,
    };
    [
        cell(DEFAULT, json!({})),
        Cell {
            filter_lists_off: true,
            ..cell(
                "filter_lists_off",
                json!({ "filter_lists": { "ads": false, "trackers": false, "annoyances": false } }),
            )
        },
        Cell {
            host_cache_off: true,
            ..cell(
                "host_cache_off",
                json!({ "filter_lists": { "host_cache": 0 } }),
            )
        },
        cell(
            SINGLE_ATTEMPT,
            json!({ "extraction": { "readability": "single_attempt" } }),
        ),
        Cell {
            many_hosts: true,
            ..cell("many_hosts", json!({}))
        },
    ]
}

impl Cell {
    /// The URL of each page its crawl starts from: the site's first page,
    /// or, spread over [`HOSTS`] hosts, each host's one page.
    fn seeds(&self) -> Vec<String> {
        if self.many_hosts {
            (0..HOSTS)
                .map(|host| format!("http://h{host:04}.{HOST}/sites/16/{}", host % 16 + 1))
                .collect()
        } else {
            vec![format!("http://{HOST}/sites/16/1")]
        }
    }

    /// The pages its crawl reaches.
    fn pages(&self) -> usize {
        if self.many_hosts { HOSTS } else { 16 }
    }

    /// Its `crawl.start`: each page's text and its `title`, every link
    /// followed from the first page, or the seeds alone where each is a
    /// host's one page, on the `http` tier with no Politeness delay, each
    /// host mapped to an address the egress policy lets through, which no
    /// connection reaches. Its own settings replace a key of these whole.
    fn crawl_start(&self) -> Value {
        let seeds = self.seeds();
        let hosts: Vec<String> = seeds
            .iter()
            .filter_map(|seed| Some(url::Url::parse(seed).ok()?.host_str()?.to_owned()))
            .collect();
        let resolve: Map<String, Value> = hosts
            .iter()
            .map(|host| (host.clone(), json!(["127.0.0.1"])))
            .collect();
        let mut settings = Map::from_iter([
            ("fetch".to_owned(), json!({ "tier": "http" })),
            ("politeness".to_owned(), json!({ "domain": { "delay": 0 } })),
            ("network".to_owned(), json!({ "resolve": resolve })),
            ("egress".to_owned(), json!({ "private_ranges": hosts })),
        ]);
        if let Value::Object(own) = &self.settings {
            settings.extend(own.clone());
        }
        let seeds: Vec<Value> = seeds
            .iter()
            .map(|url| json!({ "url": url, "label": "page" }))
            .collect();
        json!({
            "jsonrpc": "2.0",
            "id": "start",
            "method": "crawl.start",
            "params": {
                "run_id": "pages",
                "seeds": seeds,
                "labels": {},
                "datasets": [],
                "extraction": {
                    "formats": { "text": true },
                    "follow": if self.many_hosts { "none" } else { "scope" },
                    "fields": {
                        "title": { "locator": { "kind": "css", "selector": "title" } },
                    },
                },
                "settings": settings,
            },
        })
    }
}

/// The embedded Filter lists, for the plant of [`INSTRUCTIONS`].
static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("invariant: the embedded Filter lists load"));

/// The plant of the row named `name`: one more count on `work`, the
/// crawl's, for [`INSTRUCTIONS`] one more removal of the page served, and
/// for [`COMPILATIONS`] [`PLANTED_SELECTORS`] selectors more compiled,
/// and for [`GENERIC_SET`] one more load of the Filter lists in the crawl,
/// counted nowhere but by Callgrind.
fn plant(name: &str, work: &CountedWork) -> Plant {
    if name == GENERIC_SET {
        let loaded = Arc::new(std::sync::Once::new());
        return Arc::new(move |_: &str| {
            loaded.call_once(|| {
                std::hint::black_box(
                    FilterLists::embedded().expect("invariant: the embedded Filter lists load"),
                );
            });
        });
    }
    if name == COMPILATIONS {
        return Arc::new(|_: &str| {
            let apart = CountedWork::default();
            for at in 0..PLANTED_SELECTORS {
                let selector = format!("div.planted-{at} > p:nth-child(2n+1)");
                std::hint::black_box(
                    apart.compile_selector(Row::PageSelectorCompilations, &selector),
                );
            }
        });
    }
    if name == INSTRUCTIONS {
        return Arc::new(|page: &str| {
            let apart = CountedWork::default();
            let document = apart.parse_html(Row::PageParses, page);
            let choices = Choices {
                ads: true,
                trackers: true,
                annoyances: true,
                ..Choices::default()
            };
            let rules = Rules {
                lists: &LISTS,
                version: "planted",
                choices: &choices,
                hosts: &HostCache::new(0),
            };
            let url = format!("http://{HOST}/planted");
            std::hint::black_box(removal::remove(&document, &url, rules, &apart));
        });
    }
    let Some(row) = PER_PAGE
        .into_iter()
        .chain(PER_CRAWL)
        .find(|row| row.name() == name)
    else {
        panic!("no plant for the row {name}");
    };
    let work = work.clone();
    Arc::new(move |_: &str| work.count(row))
}

/// The crawl of `cell`, on an engine and in a home of its own, with the
/// row `planted` planted: what it counted, having read each page right.
fn crawl(cell: &Cell, planted: Option<&str>) -> CountedWork {
    crawl_on(cell, planted, Metrics::none())
}

/// Where the run under Callgrind starts to count: Callgrind zeroes its
/// costs as this is entered, named as Callgrind names it.
const COUNTED_FROM: &str = "counts_extraction::counted_from_here";

/// Entered once the engine's cache holds the Clean-up rules' selectors
/// compiled, or, before the compilations are counted, once a first crawl
/// has set up what the process sets up once ([`COUNTED_FROM`]).
#[inline(never)]
fn counted_from_here() {
    std::hint::black_box(());
}

/// The crawl of `cell` as [`crawl`] runs it, on an engine that counts on
/// `metrics` and compiles into its Counted work's cache.
fn crawl_on(cell: &Cell, planted: Option<&str>, metrics: Metrics) -> CountedWork {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let work = metrics.counted_work().clone();
    // An earlier crawl on the same metrics counted its own builds.
    let built_before = work.read(Row::PageRuleListBuilds);
    let site = Arc::new(Site {
        pages: pages(),
        plant: planted.map(|name| plant(name, &work)),
    });
    let (records, _) = drive(
        site,
        &cell.crawl_start(),
        Driven {
            home: home.path(),
            start: std::time::UNIX_EPOCH,
            seed: 0,
            metrics,
            run_state_vfs: None,
            handler: false,
            credits: u64::try_from(cell.pages()).expect("invariant: a few thousand pages"),
            after_start: AfterStart::Finish,
        },
    );
    assert_eq!(
        records.len(),
        cell.pages(),
        "the cell {} extracts each page once",
        cell.name
    );
    for record in &records {
        let text = record["text"].as_str().unwrap_or_default();
        assert!(
            text.contains("of the catalogue.") && record["title"].is_string(),
            "the cell {} read no article and title: {record}",
            cell.name
        );
        assert_eq!(
            text.contains(AD),
            cell.filter_lists_off,
            "the cell {} kept the ad, or removed it with its Filter lists off: {record}",
            cell.name
        );
    }
    let builds = work.read(Row::PageRuleListBuilds) - built_before;
    if planted.is_none() && !cell.filter_lists_off {
        // One rule list per host the host cache misses: each page's at
        // `host_cache_off` and on `many_hosts`, the crawl's one otherwise.
        let hosts = if cell.many_hosts || cell.host_cache_off {
            cell.pages()
        } else {
            1
        };
        assert_eq!(
            builds,
            u64::try_from(hosts).expect("invariant: a few thousand pages"),
            "the cell {} built a rule list for a host its host cache holds",
            cell.name
        );
    }
    let cleaned = builds > 0 && work.read(Row::PageRuleTests) > 0;
    assert_eq!(
        cleaned || planted.is_some(),
        !cell.filter_lists_off || planted.is_some(),
        "the cell {} built no rule list, or built one with its Filter lists off",
        cell.name
    );
    work
}

/// The instructions the `default` cell's counted crawl runs beneath
/// `function`, the row `row`'s, with the row `planted` planted, from a run
/// of this example under Callgrind with `--{run} default`; none without
/// Valgrind, said on stderr.
fn instructions_beneath(
    (row, function): (&str, &str),
    run: &str,
    planted: Option<&str>,
) -> Option<u64> {
    let program = std::env::current_exe().expect("invariant: this executable has a path");
    let flag = format!("--{run}");
    let mut arguments = vec![flag.as_str(), DEFAULT];
    if let Some(planted) = planted {
        arguments.extend(["--plant", planted]);
    }
    match callgrind::instructions_beneath(&[function], Some(COUNTED_FROM), &program, &arguments) {
        Ok(Some(instructions)) => Some(instructions),
        // A function renamed would otherwise read as free, and pass.
        Ok(None) => panic!("Callgrind never entered {function}: the work {row} counts has moved"),
        Err(error) => {
            eprintln!("{row} is not read: {error}");
            None
        }
    }
}

/// The cell named `name`, which a parent names for its run under Callgrind.
fn cell_named(name: &str) -> Cell {
    cells()
        .into_iter()
        .find(|cell| cell.name == name)
        .expect("invariant: its parent named a cell")
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let value_of = |flag: &str| {
        arguments
            .iter()
            .position(|argument| argument == flag)
            .and_then(|at| arguments.get(at + 1))
            .map(String::as_str)
    };
    let planted = value_of("--plant");
    if let Some(name) = value_of("--callgrind-compilations") {
        // A first crawl on an engine of its own sets up what the process
        // sets up once whatever its engines, the table of HTML names among
        // them, which no crawl's compilation pays; the counted crawl's
        // engine starts with its cache of compilations empty, so each
        // selector it meets is compiled once.
        let cell = cell_named(name);
        crawl_on(&cell, None, Metrics::none());
        counted_from_here();
        crawl_on(&cell, planted, Metrics::none());
        return;
    }
    if let Some(name) = value_of("--callgrind") {
        let cell = cell_named(name);
        // The crawl runs twice on engines that share one cache of
        // compilations, as an engine's later Runs do, and only the second
        // is counted. The first compiles the rules' selectors, once for the
        // engine and their version whatever the pages that follow (ADR
        // 0138): no page's work, and what the cache's table costs to fill
        // differs from run to run with its hasher's seed, by more than the
        // row's tolerance on a site this short.
        let metrics = Metrics::none();
        crawl_on(&cell, None, metrics.clone());
        counted_from_here();
        crawl_on(&cell, planted, metrics);
        return;
    }
    // Side by side, each on its own paused runtime, the run under Callgrind
    // beside them: no cell waits on another, so the family's rows stay
    // within their minute. A plant of another row has no use for the
    // instructions and would wait for them.
    let reads = |row: &str| planted.is_none_or(|planted| planted == row);
    let (counted, read_under_callgrind) = std::thread::scope(|scope| {
        let runs = [
            ((INSTRUCTIONS, REMOVAL), "callgrind"),
            ((COMPILATIONS, COMPILE), "callgrind-compilations"),
            ((GENERIC_SET, GENERIC_BUILD), "callgrind"),
        ]
        .map(|(row, run)| {
            (
                row.0,
                reads(row.0).then(|| scope.spawn(move || instructions_beneath(row, run, planted))),
            )
        });
        let cells: Vec<_> = cells()
            .into_iter()
            .map(|cell| scope.spawn(move || (crawl(&cell, planted), cell)))
            .collect();
        let counted: Vec<(CountedWork, Cell)> = cells
            .into_iter()
            .map(|cell| cell.join().expect("invariant: a cell's checks passed"))
            .collect();
        let read_under_callgrind = runs.map(|(row, run)| {
            let figure = run.and_then(|run| {
                run.join()
                    .expect("invariant: the run under Callgrind passed")
            });
            (row, figure)
        });
        (counted, read_under_callgrind)
    });
    if planted.is_none() {
        // The number of attempts is inside one run of the step: a single
        // attempt runs it as often as the retries do.
        let runs = |name: &str| {
            counted
                .iter()
                .find(|(_, cell)| cell.name == name)
                .map(|(work, _)| work.read(Row::PageReadabilityRuns))
        };
        assert_eq!(
            runs(SINGLE_ATTEMPT),
            runs(DEFAULT),
            "the cell {SINGLE_ATTEMPT} ran the main-content step another number of times than {DEFAULT}"
        );
    }
    for (work, cell) in &counted {
        let mut counts: Map<String, Value> = PER_PAGE
            .iter()
            .chain(&PER_CRAWL)
            .map(|row| (row.name().to_owned(), json!(work.read(*row))))
            .collect();
        // The other cells run under no Callgrind: they print the row with
        // none of its unit, which `xtask counts` reads as no figure.
        let read = cell.name == DEFAULT;
        for (row, figure) in read_under_callgrind {
            if let Some(figure) = figure.filter(|_| read) {
                counts.insert(row.to_owned(), json!(figure));
            } else if !read {
                counts.insert(row.to_owned(), json!(0));
            }
        }
        println!(
            "{}",
            json!({
                "cell": cell.name,
                "units": {
                    "pages": cell.pages(),
                    "crawls": 1,
                    "callgrind_pages": if read { cell.pages() } else { 0 },
                    "callgrind_crawls": usize::from(read),
                },
                "counts": counts,
            })
        );
    }
}
