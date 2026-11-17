use super::{
    Calls, MOST_AT_ONCE, bounded, calls_beneath_in, calls_in, entered_in, most_at_once, total_in,
};

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

/// However many threads start a Callgrind child at once, [`bounded`] runs at
/// most half the machine's cores of them together, and never more than
/// [`MOST_AT_ONCE`], each one's result answered as it was.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "plain threads stand for a benchmark's cells, each starting its child"
)]
#[expect(
    clippy::disallowed_types,
    reason = "the test reads how many run at once from the threads it starts"
)]
fn bounded_runs_at_most_half_the_cores_at_once_and_answers_each() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let most = most_at_once();
    let (running, highest) = (AtomicUsize::new(0), AtomicUsize::new(0));

    let answers: Vec<usize> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..3 * MOST_AT_ONCE)
            .map(|child| {
                let (running, highest) = (&running, &highest);
                scope.spawn(move || {
                    bounded(|| {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        highest.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        running.fetch_sub(1, Ordering::SeqCst);
                        child
                    })
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect()
    });

    assert_eq!(answers, (0..3 * MOST_AT_ONCE).collect::<Vec<_>>());
    let highest = highest.load(Ordering::SeqCst);
    assert!(
        (1..=most).contains(&highest),
        "{highest} ran at once, at most {most}"
    );
}

/// The end of what Callgrind writes: the instructions it collected, in all.
const COLLECTED: &str = "\
events: Ir
fn=goodfetch_clean_up_rules::removal::remove
16 4
cfn=dom_query::node::NodeRef::is_match
calls=135 0
16 90000

totals: 90004
";

#[test]
fn the_instructions_collected_are_the_outputs_total() {
    assert_eq!(total_in(COLLECTED), Some(90_004));
}

#[test]
fn an_output_with_no_total_holds_no_instruction_count() {
    assert_eq!(total_in(OUTPUT), None);
}

#[test]
fn a_function_is_entered_when_the_output_holds_its_costs() {
    assert!(entered_in(OUTPUT, "rusqlite::Connection::prepare"));
    assert!(!entered_in(OUTPUT, "rusqlite::Connection::prepare_cached"));
}

#[test]
fn a_called_function_whose_costs_were_not_collected_was_not_entered() {
    // A `cfn=` line names a callee whose own costs another `fn=` holds.
    assert!(!entered_in(OUTPUT, "sqlite3_step"));
}

#[test]
fn a_star_stands_for_any_text_in_a_function_name() {
    assert!(entered_in(OUTPUT, "rusqlite::Connection::*"));
    assert!(entered_in(OUTPUT, "*::execute_batch"));
    assert!(entered_in(OUTPUT, "sqlite3_*_v3"));
    assert!(!entered_in(OUTPUT, "rusqlite::*::step"));
    assert!(!entered_in(OUTPUT, "rusqlite::Connection::prepare*x"));
}
