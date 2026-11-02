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

#[test]
fn a_row_is_named_as_the_register_names_it() {
    assert_eq!(Row::PageParses.name(), "F1.parses");
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
