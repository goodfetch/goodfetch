use rstest::rstest;

use super::moves;

/// A move goes from where the mouse is to its point in evenly spaced
/// steps, the last on the point, as Playwright's `mouse.move` does.
#[rstest]
#[case::one_step(1, vec![(30.0, 50.0)])]
#[case::no_step_as_one(0, vec![(30.0, 50.0)])]
#[case::four_steps(4, vec![(15.0, 20.0), (20.0, 30.0), (25.0, 40.0), (30.0, 50.0)])]
fn a_move_reaches_its_point_in_even_steps(#[case] steps: u32, #[case] expected: Vec<(f64, f64)>) {
    assert_eq!(
        moves((10.0, 10.0), (30.0, 50.0), steps).collect::<Vec<_>>(),
        expected
    );
}
