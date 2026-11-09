use std::time::Duration;

use rstest::rstest;

use super::{Limits, Opens, Placement, Recycle, Seats, Stage, Use, leaves, place, recycle};

const MIB: u64 = 1024 * 1024;

/// A page that a move could not race.
const ENGINE_PAGE: Opens = Opens::EnginePage { races: false };

/// A page that opens while a move could race a program's navigation.
const RACING_ENGINE_PAGE: Opens = Opens::EnginePage { races: true };

#[rstest]
#[case::a_serving_browser_keeps_a_program_page(Opens::ProgramPage, Stage::Serving, false)]
#[case::a_serving_browser_keeps_an_engine_page(ENGINE_PAGE, Stage::Serving, false)]
#[case::a_recycled_browser_opens_no_program_page(Opens::ProgramPage, Stage::Retiring, true)]
#[case::a_recycled_browser_opens_no_engine_page(ENGINE_PAGE, Stage::Retiring, true)]
#[case::a_recycled_browser_opens_one_a_move_would_race(RACING_ENGINE_PAGE, Stage::Retiring, false)]
#[case::a_closing_browser_opens_no_program_page(Opens::ProgramPage, Stage::Closing, true)]
#[case::a_closing_browser_opens_no_engine_page(ENGINE_PAGE, Stage::Closing, true)]
#[case::a_closing_browser_opens_none_a_move_would_race(RACING_ENGINE_PAGE, Stage::Closing, true)]
fn a_session_leaves_a_browser_that_opens_it_no_page(
    #[case] opens: Opens,
    #[case] stage: Stage,
    #[case] left: bool,
) {
    assert_eq!(leaves(opens, stage), left);
}

/// ADR 0040's defaults: 8 contexts per process, 100 pages, 5 idle
/// minutes, a cap of twice the warm-up size.
fn defaults() -> Limits {
    Limits {
        contexts_per_process: 8,
        pages: 100,
        idle: Duration::from_secs(300),
        memory_cap: None,
    }
}

fn used(served: u64, footprint: Option<u64>, warm_up: Option<u64>, held: bool) -> Use {
    Use {
        served,
        footprint,
        warm_up,
        held,
    }
}

#[rstest]
#[case::a_fresh_browser(used(0, None, None, false), defaults(), None)]
#[case::one_page_short_of_its_pages(used(99, Some(300 * MIB), Some(200 * MIB), false), defaults(), None)]
#[case::at_its_pages(used(100, Some(300 * MIB), Some(200 * MIB), false), defaults(), Some(Recycle::Pages))]
#[case::at_twice_its_warm_up(used(3, Some(400 * MIB), Some(200 * MIB), false), defaults(), None)]
#[case::past_twice_its_warm_up(used(3, Some(400 * MIB + 1), Some(200 * MIB), false), defaults(), Some(Recycle::Memory))]
#[case::no_warm_up_measured_yet(used(1, Some(900 * MIB), None, false), defaults(), None)]
#[case::past_an_absolute_cap(
    used(1, Some(600 * MIB), None, false),
    Limits { memory_cap: Some(500 * MIB), ..defaults() },
    Some(Recycle::Memory)
)]
#[case::past_an_absolute_cap_before_its_first_page(
    used(0, Some(600 * MIB), None, false),
    Limits { memory_cap: Some(500 * MIB), ..defaults() },
    None
)]
#[case::an_absolute_cap_in_place_of_twice_its_warm_up(
    used(3, Some(900 * MIB), Some(200 * MIB), false),
    Limits { memory_cap: Some(1000 * MIB), ..defaults() },
    None
)]
#[case::held_at_its_pages(used(100, Some(300 * MIB), Some(200 * MIB), true), defaults(), None)]
#[case::held_past_its_cap(used(3, Some(900 * MIB), Some(200 * MIB), true), defaults(), None)]
fn a_browser_is_recycled_at_its_pages_or_past_its_memory_cap_unless_it_is_held(
    #[case] browser: Use,
    #[case] limits: Limits,
    #[case] expected: Option<Recycle>,
) {
    assert_eq!(recycle(&browser, &limits), expected);
}

fn seats(contexts: u64, retiring: bool, own: bool) -> Seats {
    Seats {
        contexts,
        retiring,
        own,
    }
}

#[rstest]
#[case::no_browser_yet(vec![], Placement::Shared, None)]
#[case::the_first_with_room(vec![(1, seats(8, false, false)), (2, seats(3, false, false)), (3, seats(0, false, false))], Placement::Shared, Some(2))]
#[case::every_browser_full(vec![(1, seats(8, false, false)), (2, seats(8, false, false))], Placement::Shared, None)]
#[case::a_recycled_browser_takes_no_session(vec![(1, seats(1, true, false))], Placement::Shared, None)]
#[case::a_process_of_its_own_is_never_shared(vec![(1, seats(1, false, true))], Placement::Shared, None)]
#[case::a_per_process_session_never_shares(vec![(1, seats(0, false, false))], Placement::OwnProcess, None)]
fn a_session_shares_the_first_browser_of_its_run_with_room_or_gets_a_new_one(
    #[case] browsers: Vec<(u64, Seats)>,
    #[case] placement: Placement,
    #[case] expected: Option<u64>,
) {
    assert_eq!(place(browsers, placement, &defaults()), expected);
}

#[test]
fn a_contexts_per_process_of_zero_still_seats_one_session_a_browser() {
    let limits = Limits {
        contexts_per_process: 0,
        ..defaults()
    };

    assert_eq!(
        (
            place(
                vec![(1, seats(0, false, false))],
                Placement::Shared,
                &limits
            ),
            place(
                vec![(1, seats(1, false, false))],
                Placement::Shared,
                &limits
            ),
        ),
        (Some(1), None)
    );
}
