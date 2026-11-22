use std::num::NonZeroUsize;

use rstest::rstest;

use super::{Bound, most_at_once};

#[rstest]
#[case::half_the_cores(4, None, 2)]
#[case::at_most_eight(32, None, 8)]
#[case::at_least_one(1, None, 1)]
#[case::a_cap_lowers_it(16, Some(2), 2)]
#[case::a_cap_of_one(16, Some(1), 1)]
#[case::a_cap_never_raises_it(4, Some(6), 2)]
fn the_most_at_once_is_half_the_cores_within_one_and_eight_and_never_over_the_cap(
    #[case] cores: usize,
    #[case] cap: Option<usize>,
    #[case] expected: usize,
) {
    let cap = cap.and_then(NonZeroUsize::new);

    assert_eq!(most_at_once(cores, cap), expected);
}

/// However many threads start a Callgrind child at once, a bound with a most
/// of 2 never has more than 2 running, each one's result answered as it was.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "plain threads stand for a benchmark's cells, each starting its child"
)]
#[expect(
    clippy::disallowed_types,
    reason = "the test reads how many run at once from the threads it starts"
)]
fn a_bound_runs_at_most_its_most_at_once_and_answers_each() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let bound = Bound::new();
    let (running, highest) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let children = 24;

    let answers: Vec<usize> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..children)
            .map(|child| {
                let (bound, running, highest) = (&bound, &running, &highest);
                scope.spawn(move || {
                    bound.run(2, || {
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

    assert_eq!(answers, (0..children).collect::<Vec<_>>());
    let highest = highest.load(Ordering::SeqCst);
    assert!(
        (1..=2).contains(&highest),
        "{highest} ran at once, at most 2"
    );
}
