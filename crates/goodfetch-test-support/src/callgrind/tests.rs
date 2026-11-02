use super::calls_in;

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
