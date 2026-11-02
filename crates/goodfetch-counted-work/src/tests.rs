use super::{CountedWork, Row};

#[test]
fn each_parse_counts_once_on_every_clone_and_no_other_value() {
    let (work, other) = (CountedWork::default(), CountedWork::default());
    let clone = work.clone();

    let parsed = work.parse_html(Row::PageParses, "<p>one</p>");
    let _ = clone.parse_html(Row::PageParses, "<p>two</p>");

    assert_eq!(parsed.select("p").text().as_ref(), "one");
    assert_eq!(
        (work.read(Row::PageParses), other.read(Row::PageParses)),
        (2, 0)
    );
}

#[test]
fn a_row_is_named_as_the_register_names_it() {
    assert_eq!(Row::PageParses.name(), "F1.parses");
}

/// `Row::ALL` sizes the counts, so a row left out of it would index past them.
#[test]
fn each_row_counts_on_its_own() {
    let work = CountedWork::default();

    work.count(Row::SinkWriterReads);

    assert_eq!(
        Row::ALL.map(|row| (row.name(), work.read(row))),
        [
            ("F1.parses", 0),
            ("F5.messages_no_hook", 0),
            ("F5.calls", 0),
            ("F5.builds", 0),
            ("F6.opens", 0),
            ("F6.transactions", 0),
            ("F6.writer_reads", 1),
        ]
    );
}
