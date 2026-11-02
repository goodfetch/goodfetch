use super::{Calls, calls_beneath_in, calls_in};

/// A piece of what Callgrind writes: two callers of the symbol, a call of
/// another symbol between them, and the symbol's own cost line.
const OUTPUT: &str = "\
fn=rusqlite::Connection::prepare
16 4
cfn=sqlite3_prepare_v3
calls=135 0
16 90000
cfn=sqlite3_step
calls=500 0
17 1000
fn=rusqlite::Connection::execute_batch
cfn=sqlite3_prepare_v3
calls=7 0
20 1000
fn=sqlite3_prepare_v3
8 12
cfn=sqlite3LockAndPrepare
calls=142 0
8 900
";

#[test]
fn every_callers_calls_of_the_symbol_are_added_and_no_other_symbols() {
    assert_eq!(calls_in(OUTPUT, "sqlite3_prepare_v3"), 142);
}

#[test]
fn a_symbol_the_program_never_calls_counts_none() {
    assert_eq!(calls_in(OUTPUT, "sqlite3_exec"), 0);
}

/// A piece of what Callgrind writes with the symbol's callers separated: a
/// file's opening, two of its owner's statements by two of SQLite's wrapper's
/// functions, and another owner's.
const SEPARATED: &str = "\
fn=rusqlite::Connection::prepare'goodfetch_engine::politeness::create_tables'goodfetch_engine::politeness::open_file
cfn=sqlite3_prepare_v3'rusqlite::Connection::prepare'goodfetch_engine::politeness::create_tables'goodfetch_engine::politeness::open_file
calls=5 0
16 900
fn=rusqlite::Connection::prepare'goodfetch_engine::politeness::take_in
cfn=sqlite3_prepare_v3'rusqlite::Connection::prepare'goodfetch_engine::politeness::take_in
calls=40 0
16 900
fn=rusqlite::Connection::execute'goodfetch_frontier::robots_txt::stored'goodfetch_engine::politeness::Politeness::transact::{{closure}}
cfn=sqlite3_prepare_v3'rusqlite::Connection::execute'goodfetch_frontier::robots_txt::stored'goodfetch_engine::politeness::Politeness::transact::{{closure}}
calls=14 0
16 900
fn=rusqlite::Connection::prepare'goodfetch_engine::run_state::commit_group
cfn=sqlite3_prepare_v3'rusqlite::Connection::prepare'goodfetch_engine::run_state::commit_group
calls=100 0
16 900
cfn=sqlite3_prepare_v3_helper'goodfetch_engine::politeness::take_in
calls=1000 0
16 900
";

#[test]
fn the_calls_beneath_an_owner_are_its_own_and_its_left_out_callers_are_not() {
    assert_eq!(
        calls_beneath_in(
            SEPARATED,
            "sqlite3_prepare_v3",
            "goodfetch_engine::politeness::",
            &["politeness::open_file", "politeness::create_tables"],
        ),
        Calls {
            all: 159,
            beneath: 54
        }
    );
}
