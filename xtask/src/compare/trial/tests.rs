use std::path::Path;

use super::{instructions_of, invocation, system_calls_of};
use crate::compare::matrix::Config;

/// Foxhound and Spider run as their executables; Crawlee's script runs on
/// Node.js and Scrapy's on the Python environment beside it, so the tool's
/// process is the interpreter running the side, with no launcher between.
#[test]
fn each_tool_runs_as_its_executable_or_its_sides_interpreter() {
    let spider = invocation(Config::Spider, Path::new("/s/spider"));
    let crawlee = invocation(
        Config::Crawlee { durable: false },
        Path::new("/c/crawl.mjs"),
    );
    let scrapy = invocation(Config::Scrapy, Path::new("/p/crawl.py"));

    assert_eq!(
        (
            (spider.program.as_path(), spider.arguments.len()),
            crawlee.program.file_name().and_then(|name| name.to_str()),
            crawlee.arguments,
            scrapy.program.as_path(),
            scrapy.arguments,
        ),
        (
            (Path::new("/s/spider"), 0),
            Some("node"),
            vec!["/c/crawl.mjs".into()],
            Path::new("/p/.venv/bin/python"),
            vec!["/p/crawl.py".into()],
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
