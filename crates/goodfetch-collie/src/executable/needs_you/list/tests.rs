//! The list a session holds, as the engine's events change it.

use std::time::SystemTime;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::needs_you::{NeedsYouChange, NeedsYouItem};
use serde_json::Map;

use super::{Kinds, List};

const COLLIES: Kinds = Kinds {
    product: Product::Collie,
    own: &[],
};

fn item(id: &str, summary: &str) -> NeedsYouItem {
    NeedsYouItem {
        id: id.to_owned(),
        kind: "toy".to_owned(),
        since: SystemTime::UNIX_EPOCH,
        run: None,
        schedule: None,
        summary: summary.to_owned(),
        actions: Vec::new(),
        details: Map::new(),
    }
}

/// A subscription that fell behind gives the whole list again: what left it
/// and what came is said, what stayed keeps its number, and no number is
/// given twice.
#[test]
fn a_resync_says_what_left_and_what_came_and_keeps_the_numbers() {
    let mut list = List::default();
    list.resync(vec![item("a", "A."), item("b", "B.")], COLLIES);
    list.answered("b");

    let news = list.resync(vec![item("b", "B."), item("c", "C.")], COLLIES);

    let shown: Vec<(usize, &str, bool)> = list
        .rows()
        .iter()
        .map(|row| (row.number, row.item.id.as_str(), row.answered))
        .collect();
    assert_eq!(
        (news, shown),
        (
            vec![
                "Left the list: 1. A.".to_owned(),
                "Now on the list: 3. C.".to_owned(),
                "   Nothing here answers it: its summary says what does.".to_owned(),
            ],
            vec![(2, "b", true), (3, "c", false)]
        )
    );
}

/// An item that changed is said to have, waits for an answer again, and is
/// no longer the item a prompt was asked for.
#[test]
fn an_item_that_changed_waits_for_an_answer_again() {
    let mut list = List::default();
    let asked = item("a", "A.");
    list.resync(vec![asked.clone()], COLLIES);
    list.answered("a");

    let news = list.change(
        NeedsYouChange::Listed {
            item: item("a", "A, worse."),
        },
        COLLIES,
    );

    assert_eq!(
        (
            news.first().map(String::as_str),
            list.holds(&asked),
            list.rows()[0].answered
        ),
        (Some("Changed: 1. A, worse."), false, false)
    );
}
