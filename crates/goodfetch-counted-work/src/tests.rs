#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]
use std::collections::{BTreeMap, BTreeSet};

use super::compiled::CAPACITY;
use super::{CountedWork, Row};

#[test]
fn each_parse_counts_once_on_every_clone_and_no_other_value() {
    let (work, other) = (CountedWork::default(), CountedWork::default());
    let clone = work.clone();

    let parsed = work.parse_html(Row::PageParses, "<p>one</p>");
    let _ = clone.parse_html(Row::PageParses, "<p>two</p>");

    let paragraph = work.literal_selector(Row::PageSelectorCompilations, "p");
    assert_eq!(parsed.select_matcher(&paragraph).text().as_ref(), "one");
    assert_eq!(
        (work.read(Row::PageParses), other.read(Row::PageParses)),
        (2, 0)
    );
}

/// A parsed tree is `Send` (`dom_query`'s `atomic` tendrils), so a page's one
/// tree can move to the task that keeps it and be read there (#2109, #1918).
#[test]
fn a_parsed_tree_is_send_so_one_task_can_keep_it_and_read_it() {
    fn moved<T: Send>(value: T) -> T {
        value
    }
    let work = CountedWork::default();
    let parsed = moved(work.parse_html(Row::PageParses, "<p class=one>one</p>"));

    let paragraph = work.literal_selector(Row::PageSelectorCompilations, "p.one");
    assert_eq!(parsed.select_matcher(&paragraph).text().as_ref(), "one");
}

/// Each counted function counts one unit on its row whether or not its work
/// succeeds, and answers what the raw call answers.
#[test]
fn a_compilation_or_an_opening_counts_one_on_its_row_and_answers_as_the_raw_call() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    let compiled = (
        work.compile_selector(row, "p").is_some(),
        work.compile_selector(row, "p[").is_some(),
        work.compile_regex(row, "a+").is_ok(),
        work.compile_regex(row, "a(").is_ok(),
        work.compile_glob(row, "*.rs").is_ok(),
        work.compile_glob(row, "***").is_ok(),
    );
    let folder = tempfile::tempdir().unwrap();
    let inner = folder.path().join("a/b");
    let file = inner.join("c");
    let files = Row::PageFileOpens;
    work.create_dir_all(files, &inner).unwrap();
    work.open_file(
        files,
        std::fs::OpenOptions::new().create(true).append(true),
        &file,
    )
    .unwrap();
    let missing = work.open_file(
        files,
        std::fs::OpenOptions::new().read(true),
        &inner.join("d"),
    );

    assert_eq!(
        (compiled, work.read(row), work.read(files), missing.is_err()),
        ((true, false, true, false, true, false), 6, 3, true)
    );
}

/// Each row is named as the register names it, `<entry>.<work>` with an
/// entry such as `F1` and a work of one or two words, and no two rows share
/// a name or a metric's labels, Shared path and work; each Shared path
/// belongs to one register entry, the part of its rows' names before the
/// dot.
#[test]
fn each_row_has_its_own_register_name_and_labels_and_its_path_one_entry() {
    let mut names = BTreeSet::new();
    let mut labels = BTreeSet::new();
    let mut entries: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut misnamed = Vec::new();
    for row in Row::ALL {
        let mut parts = row.name().split('.');
        let entry = parts.next().unwrap_or_default();
        let work: Vec<&str> = parts.collect();
        let (letter, number) = entry.split_at(entry.len().min(1));
        let shaped = letter.chars().all(|letter| letter.is_ascii_uppercase())
            && !letter.is_empty()
            && !number.is_empty()
            && number.chars().all(|digit| digit.is_ascii_digit())
            && (1..=2).contains(&work.len())
            && work.iter().all(|word| {
                !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            });
        if !shaped || !names.insert(row.name()) || !labels.insert((row.path(), row.work())) {
            misnamed.push(row.name());
        }
        entries.entry(row.path()).or_default().insert(entry);
    }
    let shared: Vec<&str> = entries
        .iter()
        .filter(|(_, of)| of.len() > 1)
        .map(|(path, _)| *path)
        .collect();

    assert_eq!((misnamed, shared), (Vec::new(), Vec::new()));
}

/// `Row::ALL` sizes the counts, so a row left out of it would index past them.
#[test]
fn each_row_counts_on_its_own() {
    let work = CountedWork::default();

    work.count(Row::SinkWriterReads);

    assert_eq!(
        Row::ALL.map(|row| work.read(row)),
        Row::ALL.map(|row| u64::from(row.name() == "F6.writer_reads"))
    );
}

/// The engine's cache compiles each text once, counted on its miss, and
/// answers every later lookup from what it kept, a selector that does not
/// parse included; a selector of the Clean-up rules is kept apart for each
/// version of them.
#[test]
fn a_kept_compilation_counts_once_for_its_text_and_its_rules_version() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    let answers = (0..3)
        .map(|_| {
            (
                work.selector(row, "p").is_some(),
                work.selector(row, "p[").is_some(),
                work.parse_html(Row::PageParses, "<h1>a</h1>")
                    .select_matcher(&work.literal_selector(row, "h1"))
                    .nodes()[0]
                    .is_match(&work.literal_selector(row, "h1, h2")),
                work.rules_selector(row, "version-a", "p").is_some(),
                work.rules_selector(row, "version-b", "p").is_some(),
                work.regex(row, "a+").is_ok(),
                work.regex(row, "a(").is_ok(),
                work.glob(row, "*.rs").is_ok(),
                work.glob(row, "***").is_ok(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        (answers[0], answers[0] == answers[2], work.read(row)),
        (
            (true, false, true, true, true, true, false, true, false),
            true,
            10
        )
    );
}

/// The cache holds at most [`CAPACITY`] compilations: one over it
/// evicts one kept before rather than grows.
#[test]
fn a_compilation_over_the_bound_evicts_rather_than_grows() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    let over = CAPACITY + 64;
    for class in 0..over {
        let _ = work.selector(row, &format!(".c{class}"));
    }

    assert_eq!((work.kept() <= CAPACITY, work.read(row)), (true, over));
}
