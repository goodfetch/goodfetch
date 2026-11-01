//! What the SQL tab refuses: any statement but one `SELECT` or `WITH`, and
//! any read outside the registered runs, each with `query_invalid`, while
//! a `SELECT` over the same registered run is allowed; a read of a URL
//! never reaches it.

use std::net::TcpListener;
use std::path::Path;

use goodfetch_protocol::foxhound::sql_tab::QueryRefusal;
use goodfetch_sql_tab::Failure;
use rstest::rstest;
use serde_json::json;

use crate::run_files::{LIMITS, book, run_file, sandbox};

/// The statement `case` names, reading `outside` or `url` where it reads.
fn statement(case: &str, outside: &Path, url: &str) -> String {
    let outside = outside.display();
    match case {
        "insert" => "INSERT INTO kept VALUES (1)".to_owned(),
        "create" => "CREATE TABLE kept AS SELECT * FROM records('shop')".to_owned(),
        "attach" => format!("ATTACH '{outside}' AS other"),
        "install" => "INSTALL httpfs".to_owned(),
        "load" => "LOAD httpfs".to_owned(),
        "copy" => format!("COPY (SELECT * FROM records('shop')) TO '{outside}'"),
        "two_statements" => "SELECT 1; DROP TABLE nothing".to_owned(),
        "read_csv_url" => format!("SELECT * FROM read_csv('{url}')"),
        "read_csv_path" => format!("SELECT * FROM read_csv('{outside}')"),
        other => panic!("no case {other}"),
    }
}

/// Covers: story:12-211
#[rstest]
#[case::insert("insert", QueryRefusal::NotARead)]
#[case::create("create", QueryRefusal::NotARead)]
#[case::attach("attach", QueryRefusal::NotARead)]
#[case::install("install", QueryRefusal::NotARead)]
#[case::load("load", QueryRefusal::NotARead)]
#[case::copy("copy", QueryRefusal::NotARead)]
#[case::two_statements("two_statements", QueryRefusal::NotARead)]
#[case::read_csv_url("read_csv_url", QueryRefusal::OutsideRead)]
#[case::read_csv_path("read_csv_path", QueryRefusal::OutsideRead)]
fn a_write_attach_install_or_outside_read_is_refused_beside_an_allowed_select(
    #[case] case: &str,
    #[case] reason: QueryRefusal,
) {
    let folder = tempfile::tempdir().expect("a folder");
    let path = run_file(folder.path(), "shop", &[book(1), book(2)], "none");
    let outside = folder.path().join("outside.csv");
    std::fs::write(&outside, "a,b\n1,2\n").expect("a file outside the runs");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    listener.set_nonblocking(true).expect("non-blocking");
    let url = format!(
        "http://{}/books.csv",
        listener.local_addr().expect("its address")
    );
    let sandbox = sandbox(&[("shop", &path)], LIMITS);

    let refused = sandbox.query(&statement(case, &outside, &url), None);
    let allowed = sandbox.query("SELECT count(*) FROM records('shop')", None);

    assert!(
        matches!(&refused, Err(Failure::Invalid { reason: found, .. }) if *found == reason),
        "{case}: {refused:?}"
    );
    assert_eq!(allowed.expect("the run is read").rows, [vec![json!(2)]]);
    assert!(
        listener.accept().is_err(),
        "{case}: a connection reached the URL"
    );
    assert_eq!(
        std::fs::read_to_string(&outside).expect("the file is kept"),
        "a,b\n1,2\n"
    );
}
