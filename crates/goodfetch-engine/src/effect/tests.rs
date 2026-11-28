//! The mark a Run's end puts on the bodies it judges, so it can await the
//! Effect journal's write with the book unlocked (STYLE.md's rule on locks
//! across `.await`): a late `effect.close` leaves a body being judged
//! alone, and the end takes back exactly the bodies it judged.

use super::{Body, Book, Owner};
use crate::client::Client;

/// A body of `entry` declared outside any attempt, nothing sent for it.
fn body(entry: i64) -> Body {
    let (client, _) = Client::new();
    Body {
        entry,
        request: None,
        key: format!("key-{entry}"),
        repeatable: false,
        sent: None,
        owner: Owner {
            client,
            attempt: None,
        },
        judging: false,
    }
}

fn entries(bodies: &[Body]) -> Vec<i64> {
    bodies.iter().map(|body| body.entry).collect()
}

/// A Run's end marks its running bodies and unlocks the book for the
/// journal's write: an `effect.close` arriving then finds no running body
/// to judge, so it leaves the row to the end's judgement, and the end then
/// takes only the bodies it marked, not one declared meanwhile.
#[test]
fn a_late_close_leaves_a_body_being_judged_alone() {
    use crate::runs::ActiveEffects;

    let active = ActiveEffects::default();
    let mut book = Book::new(active.clone());
    book.push_body(body(1));
    assert_eq!(active.count(), 1);

    let judging = book.judging();
    let late_close = book.running(1);
    book.push_body(body(2));
    assert_eq!(active.count(), 2);
    let cut = book.judged(&judging);
    assert_eq!(active.count(), 1);

    assert_eq!(
        (judging, late_close, entries(&cut), entries(&book.bodies)),
        (vec![1], None, vec![1], vec![2])
    );
}
