use std::path::Path;

use super::{instructions_of, invocation, system_calls_of};
use crate::compare::matrix::{Config, SpiderBuild};

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
