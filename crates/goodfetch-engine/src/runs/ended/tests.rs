use goodfetch_protocol::run::{Checkpoint, PointInTime, RunId};

use super::{EndedHere, EndedRuns};

fn id(name: &str) -> RunId {
    RunId::parse(name).unwrap()
}

fn checkpoint(name: &str) -> Checkpoint {
    Checkpoint {
        run_id: id(name),
        location: format!("/checkpoints/{name}.sqlite"),
        size: 4096,
        digest: "0".repeat(64),
        run_epoch: 1,
        journal_position: 3,
        time: PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned()),
    }
}

/// The Runs, of `names`, whose Checkpoint `ended` keeps.
fn checkpointed(ended: &EndedRuns, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|name| {
            matches!(ended.get(&id(name)), Some(EndedHere::Checkpointed(kept)) if kept.run_id == id(name))
        })
        .map(|name| (*name).to_owned())
        .collect()
}

/// A Run whose report its run file holds and that wrote no Checkpoint
/// leaves nothing in the registry (#1768).
#[test]
fn a_run_ended_in_its_run_file_without_a_checkpoint_leaves_nothing() {
    let mut ended = EndedRuns::new();

    for number in 0..100 {
        ended.in_run_file(id(&format!("run-{number}")), None);
    }

    assert_eq!(ended.kept.len(), 0);
}

/// The Checkpoints of the latest Runs are kept up to the bound, and the
/// oldest is forgotten past it.
#[test]
fn the_latest_runs_checkpoints_are_kept_up_to_the_bound() {
    let mut ended = EndedRuns::keeping(2);

    for name in ["one", "two", "three"] {
        ended.in_run_file(id(name), Some(checkpoint(name)));
    }

    assert_eq!(
        (
            checkpointed(&ended, &["one", "two", "three"]),
            ended.checkpointed.len()
        ),
        (vec!["two".to_owned(), "three".to_owned()], 2)
    );
}

/// A Run that resumed and ended again counts once, as its latest end: the
/// one it ended before is not what the bound forgets next.
#[test]
fn a_run_that_ended_again_counts_once_as_its_latest_end() {
    let mut ended = EndedRuns::keeping(2);

    for name in ["one", "two", "one", "three"] {
        ended.in_run_file(id(name), Some(checkpoint(name)));
    }

    assert_eq!(
        checkpointed(&ended, &["one", "two", "three"]),
        vec!["one".to_owned(), "three".to_owned()]
    );
}

/// A Run forgotten on its resume or discard leaves no place in the bound.
#[test]
fn a_forgotten_run_leaves_no_place_in_the_bound() {
    let mut ended = EndedRuns::keeping(2);

    ended.in_run_file(id("one"), Some(checkpoint("one")));
    ended.in_run_file(id("two"), Some(checkpoint("two")));
    ended.forget(&id("one"));
    ended.in_run_file(id("three"), Some(checkpoint("three")));

    assert_eq!(
        (
            checkpointed(&ended, &["one", "two", "three"]),
            ended.contains(&id("one"))
        ),
        (vec!["two".to_owned(), "three".to_owned()], false)
    );
}
