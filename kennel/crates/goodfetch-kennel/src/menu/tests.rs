//! The menu's edits through their interface: the rows shown and the next
//! rows in, the edits out, checked against what a toolkit's menu shows once
//! it applies them in order.

use rstest::rstest;

use super::{Edit, edits};
use crate::state::{Choice, Row};

fn text(line: &str) -> Row {
    Row::Text(line.to_owned())
}

fn detail(line: &str) -> Row {
    Row::Detail(line.to_owned())
}

fn choice(label: &str, choice: Choice) -> Row {
    Row::Choice {
        label: label.to_owned(),
        choice,
    }
}

fn submenu(label: &str, rows: Vec<Row>) -> Row {
    Row::Submenu {
        label: label.to_owned(),
        rows,
    }
}

/// A menu as the Daemon's state, two Runs with their controls, and Quit.
fn two_runs() -> Vec<Row> {
    vec![
        text("Foxhound Daemon 0.4.1"),
        detail("Serving"),
        Row::Separator,
        submenu(
            "nightly — Running",
            vec![
                text("3 succeeded, 0 failed, 9 pending"),
                Row::Separator,
                choice("Quit", Choice::Quit),
            ],
        ),
        detail("Fetching page 3"),
        submenu("weekly — Running", vec![text("Fetching")]),
        Row::Separator,
        choice("Quit", Choice::Quit),
    ]
}

/// What a toolkit's menu shows once it applies `edits` to a menu showing
/// `shown`, one after the other.
fn applied(shown: &[Row], edits: &[Edit<'_>]) -> Vec<Row> {
    let mut menu = shown.to_vec();
    for edit in edits {
        match edit {
            Edit::Redraw { at, row } => {
                let item = item(&mut menu, at);
                *item = match (&*item, row) {
                    (Row::Submenu { rows, .. }, Row::Submenu { label, .. }) => Row::Submenu {
                        label: label.clone(),
                        rows: rows.clone(),
                    },
                    (Row::Submenu { .. } | Row::Separator, _)
                    | (_, Row::Submenu { .. } | Row::Separator) => {
                        panic!("a redraw changes no item's kind: {item:?} as {row:?}")
                    }
                    (_, line) => (*line).clone(),
                };
            }
            Edit::Insert { at, row } => {
                let (index, rows) = parent(&mut menu, at);
                assert!(index <= rows.len(), "an insertion past the end at {at:?}");
                rows.insert(index, (*row).clone());
            }
            Edit::Remove { at } => {
                let (index, rows) = parent(&mut menu, at);
                assert!(index < rows.len(), "a removal past the end at {at:?}");
                rows.remove(index);
            }
        }
    }
    menu
}

fn parent<'a>(menu: &'a mut Vec<Row>, at: &[usize]) -> (usize, &'a mut Vec<Row>) {
    let (index, above) = at.split_last().expect("an edit names a place");
    let mut rows = menu;
    for step in above {
        rows = match &mut rows[*step] {
            Row::Submenu { rows, .. } => rows,
            other => panic!("{other:?} holds no rows"),
        };
    }
    (*index, rows)
}

fn item<'a>(menu: &'a mut Vec<Row>, at: &[usize]) -> &'a mut Row {
    let (index, rows) = parent(menu, at);
    &mut rows[index]
}

#[test]
fn a_menu_that_shows_the_next_rows_takes_no_edit() {
    assert_eq!(edits(&two_runs(), &two_runs()), Vec::new());
}

#[test]
fn a_changed_line_is_redrawn_in_place() {
    let mut next = two_runs();
    next[4] = detail("Fetching page 4");

    assert_eq!(
        edits(&two_runs(), &next),
        vec![Edit::Redraw {
            at: vec![4],
            row: &detail("Fetching page 4"),
        }]
    );
}

#[test]
fn a_line_that_becomes_a_choice_keeps_its_item() {
    let shown = vec![text("No Daemon"), text("No Product is installed")];
    let next = vec![
        text("No Daemon"),
        choice("Start Daemon", Choice::StartDaemon),
    ];

    assert_eq!(
        edits(&shown, &next),
        vec![Edit::Redraw {
            at: vec![1],
            row: &choice("Start Daemon", Choice::StartDaemon),
        }]
    );
}

#[test]
fn a_changed_row_of_a_submenu_is_redrawn_without_its_submenu() {
    let mut next = two_runs();
    next[3] = submenu(
        "nightly — Running",
        vec![
            text("4 succeeded, 0 failed, 8 pending"),
            Row::Separator,
            choice("Quit", Choice::Quit),
        ],
    );

    assert_eq!(
        edits(&two_runs(), &next),
        vec![Edit::Redraw {
            at: vec![3, 0],
            row: &text("4 succeeded, 0 failed, 8 pending"),
        }]
    );
}

#[test]
fn a_submenu_whose_label_changed_is_redrawn_with_its_rows_kept() {
    let mut next = two_runs();
    next[5] = submenu("weekly — Paused", vec![text("Fetching")]);

    assert_eq!(
        edits(&two_runs(), &next),
        vec![Edit::Redraw {
            at: vec![5],
            row: &submenu("weekly — Paused", vec![text("Fetching")]),
        }]
    );
}

#[test]
fn rows_past_the_shown_ones_are_inserted_in_order() {
    let shown = vec![text("No Daemon")];
    let next = vec![
        text("No Daemon"),
        Row::Separator,
        choice("Quit", Choice::Quit),
    ];

    assert_eq!(
        edits(&shown, &next),
        vec![
            Edit::Insert {
                at: vec![1],
                row: &Row::Separator,
            },
            Edit::Insert {
                at: vec![2],
                row: &choice("Quit", Choice::Quit),
            },
        ]
    );
}

#[test]
fn rows_the_next_view_lacks_are_removed_from_the_last() {
    let shown = vec![
        text("No Daemon"),
        Row::Separator,
        choice("Quit", Choice::Quit),
    ];
    let next = vec![text("No Daemon")];

    assert_eq!(
        edits(&shown, &next),
        vec![Edit::Remove { at: vec![2] }, Edit::Remove { at: vec![1] }]
    );
}

#[test]
fn a_row_of_another_kind_replaces_the_item() {
    let shown = vec![text("No Daemon"), Row::Separator];
    let next = vec![
        text("No Daemon"),
        submenu("nightly — Running", vec![text("Fetching")]),
    ];

    assert_eq!(
        edits(&shown, &next),
        vec![
            Edit::Remove { at: vec![1] },
            Edit::Insert {
                at: vec![1],
                row: &submenu("nightly — Running", vec![text("Fetching")]),
            },
        ]
    );
}

#[rstest]
#[case::from_an_empty_menu(Vec::new(), two_runs())]
#[case::to_an_empty_menu(two_runs(), Vec::new())]
#[case::a_run_ends(two_runs(), {
    let mut next = two_runs();
    next.drain(5..6);
    next
})]
#[case::a_run_starts_first(two_runs(), {
    let mut next = two_runs();
    next.insert(3, submenu("hourly — Running", vec![text("Starting"), choice("Quit", Choice::Quit)]));
    next.insert(4, detail("Starting"));
    next
})]
#[case::a_submenu_grows_and_shrinks(
    vec![submenu("a", vec![text("1")]), submenu("b", vec![text("1"), text("2"), Row::Separator])],
    vec![submenu("a", vec![text("1"), Row::Separator, choice("Quit", Choice::Quit)]), submenu("b", vec![detail("1")])],
)]
#[case::every_kind_changes(
    vec![text("a"), Row::Separator, submenu("b", vec![text("1")]), choice("Quit", Choice::Quit)],
    vec![Row::Separator, submenu("a", Vec::new()), text("b"), Row::Separator, text("c")],
)]
fn the_edits_turn_the_shown_menu_into_the_next(#[case] shown: Vec<Row>, #[case] next: Vec<Row>) {
    assert_eq!(applied(&shown, &edits(&shown, &next)), next);
}
