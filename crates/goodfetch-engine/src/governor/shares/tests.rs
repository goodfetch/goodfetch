//! Fair shares through their interface (spec #213's Seam 4): Runs take and
//! give back admissions of each kind under the governor's targets.

use goodfetch_protocol::governor::WorkKind::{BrowserPage, Http, ModelCall};

use super::Shares;

#[test]
fn a_run_takes_admissions_up_to_its_kinds_target() {
    let shares = Shares::new([2, 1, 1]);
    let run = shares.join(100);

    let taken: Vec<bool> = (0..3)
        .map(|_| run.try_admit(Http).map(std::mem::forget).is_some())
        .collect();

    assert_eq!(taken, [true, true, false]);
}

#[test]
fn each_kind_is_admitted_on_its_own() {
    let shares = Shares::new([1, 1, 1]);
    let run = shares.join(100);
    let _http = run.try_admit(Http).unwrap();

    let others = (
        run.try_admit(Http).is_some(),
        run.try_admit(BrowserPage).is_some(),
        run.try_admit(ModelCall).is_some(),
    );

    assert_eq!(others, (false, true, true));
}

#[test]
fn an_admission_given_back_goes_to_the_run_that_waited_not_the_one_that_held_it() {
    let shares = Shares::new([2, 1, 1]);
    let (first, second) = (shares.join(100), shares.join(100));
    let held = [
        first.try_admit(Http).unwrap(),
        first.try_admit(Http).unwrap(),
    ];
    assert!(second.try_admit(Http).is_none());
    assert!(first.try_admit(Http).is_none());

    drop(held);
    let after = (
        second.try_admit(Http).is_some(),
        first.try_admit(Http).is_some(),
        second.try_admit(Http).is_some(),
    );

    // The two given back went, in turn, to the run that waited first, then
    // to the other one; the third is past the target.
    assert_eq!(after, (true, true, false));
}

#[test]
fn runs_that_keep_asking_take_turns() {
    let shares = Shares::new([1, 1, 1]);
    let runs = [shares.join(100), shares.join(100), shares.join(100)];
    let mut held = runs[0].try_admit(Http);
    let mut order = vec![0];

    for _ in 0..6 {
        for run in &runs {
            let _ = run.try_admit(Http).map(std::mem::forget);
        }
        drop(held.take());
        let next = runs
            .iter()
            .position(|run| {
                run.try_admit(Http)
                    .map(|permit| held = Some(permit))
                    .is_some()
            })
            .unwrap();
        order.push(next);
    }

    assert_eq!(order, [0, 1, 2, 0, 1, 2, 0]);
}

#[test]
fn a_runs_share_caps_what_it_holds_though_the_target_has_room() {
    let shares = Shares::new([4, 1, 1]);
    let (capped, other) = (shares.join(50), shares.join(100));

    let capped_took = (0..3)
        .filter_map(|_| capped.try_admit(Http).map(std::mem::forget))
        .count();
    let other_took = (0..3)
        .filter_map(|_| other.try_admit(Http).map(std::mem::forget))
        .count();

    assert_eq!((capped_took, other_took), (2, 2));
}

#[test]
fn a_run_always_holds_at_least_one_whatever_its_share() {
    let shares = Shares::new([3, 1, 1]);
    let run = shares.join(0);

    let took = (0..3)
        .filter_map(|_| run.try_admit(Http).map(std::mem::forget))
        .count();

    assert_eq!(took, 1);
}

#[test]
fn a_lowered_target_admits_nothing_until_enough_were_given_back() {
    let shares = Shares::new([3, 1, 1]);
    let run = shares.join(100);
    let mut held: Vec<_> = (0..3).map(|_| run.try_admit(Http).unwrap()).collect();
    shares.set_targets([1, 1, 1]);

    let mut admitted = Vec::new();
    for _ in 0..3 {
        drop(held.pop());
        admitted.push(
            run.try_admit(Http)
                .map(|permit| held.push(permit))
                .is_some(),
        );
    }

    assert_eq!(admitted, [false, false, true]);
}

#[test]
fn a_raised_target_admits_the_runs_that_waited() {
    let shares = Shares::new([1, 1, 1]);
    let (first, second) = (shares.join(100), shares.join(100));
    let _held = first.try_admit(Http).unwrap();
    assert!(second.try_admit(Http).is_none());

    shares.set_targets([2, 1, 1]);

    assert!(second.try_admit(Http).is_some());
}

#[test]
fn a_run_that_left_gives_back_what_it_held_and_what_waited_for_it() {
    let shares = Shares::new([1, 1, 1]);
    let (first, second, third) = (shares.join(100), shares.join(100), shares.join(100));
    let held = first.try_admit(Http).unwrap();
    assert!(second.try_admit(Http).is_none());
    assert!(third.try_admit(Http).is_none());
    drop(held);

    drop(second);

    assert!(third.try_admit(Http).is_some());
}

#[test]
fn in_use_counts_every_runs_admissions_of_each_kind() {
    let shares = Shares::new([4, 4, 4]);
    let (first, second) = (shares.join(100), shares.join(100));
    let _held = [
        first.try_admit(Http).unwrap(),
        second.try_admit(Http).unwrap(),
        second.try_admit(ModelCall).unwrap(),
    ];

    assert_eq!(shares.in_use(), [2, 0, 1]);
}

#[test]
fn admit_waits_for_the_runs_turn() {
    goodfetch_test_support::runtime().block_on(admit_waits());
}

async fn admit_waits() {
    let shares = Shares::new([1, 1, 1]);
    let (first, second) = (shares.join(100), shares.join(100));
    let held = first.try_admit(Http).unwrap();
    let admit = second.admit(Http);
    tokio::pin!(admit);

    let waited = tokio::select! {
        biased;
        _ = &mut admit => false,
        () = tokio::task::yield_now() => true,
    };
    drop(held);
    let admitted = admit.await;

    assert_eq!((waited, shares.in_use()), (true, [1, 0, 0]));
    drop(admitted);
}

#[test]
fn a_run_waits_from_a_refused_try_until_its_turn_comes() {
    let shares = Shares::new([1, 1, 1]);
    let (first, second) = (shares.join(100), shares.join(100));
    let held = first.try_admit(Http).unwrap();
    let before = second.waits(Http);
    assert!(second.try_admit(Http).is_none());
    let refused = second.waits(Http);

    drop(held);

    assert_eq!((before, refused, second.waits(Http)), (false, true, false));
}
