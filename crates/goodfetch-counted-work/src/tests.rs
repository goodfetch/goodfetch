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

/// `Row::ALL` sizes the counts, so a row left out of it would index past them.
#[test]
fn each_row_counts_on_its_own() {
    let work = CountedWork::default();

    work.count(Row::SinkWriterReads);

    assert_eq!(
        Row::ALL.map(|row| (row.name(), work.read(row))),
        [
            ("F1.parses", 0),
            ("F1.selector_compilations", 0),
            ("F1.readability_runs", 0),
            ("F1.file_opens", 0),
            ("F1.locale_form_builds", 0),
            ("F1.plugin_serializations", 0),
            ("F1.plugin_answer_parses", 0),
            ("E1.deep_copies", 0),
            ("E1.encodes", 0),
            ("F5.messages_no_hook", 0),
            ("F5.calls", 0),
            ("F5.builds", 0),
            ("F6.opens", 0),
            ("F6.transactions", 0),
            ("F6.writer_reads", 1),
            ("F4.declaration_reads", 0),
            ("F4.validator_builds", 0),
            ("F4.validations", 0),
            ("F4.serializations", 0),
            ("F4.deep_copies", 0),
            ("F4.constant_lookups", 0),
            ("F4.transactions", 0),
            ("F3.invariant_reads", 0),
            ("F3.compilations", 0),
            ("F3.transactions", 0),
            ("F3.invariant_reads.hop", 0),
            ("F3.compilations.hop", 0),
            ("F3.transactions.hop", 0),
            ("F3.invariant_reads.sitemap", 0),
            ("F3.compilations.sitemap", 0),
            ("F3.transactions.sitemap", 0),
            ("F3.compilations.payload", 0),
            ("F3.payload_serializations", 0),
            ("F3.validations", 0),
            ("C1.encodes", 0),
            ("C1.sends", 0),
            ("C1.file_opens", 0),
            ("C1.log_sends_off", 0),
            ("F2.transactions", 0),
            ("F2.writer_reads", 0),
            ("F2.configuration_reads", 0),
            ("F2.prunes", 0),
            ("F2.effect_syncs", 0),
            ("E3.statements", 0),
            ("E3.slice_transactions", 0),
            ("E3.opens", 0),
            ("E5.file_opens", 0),
            ("E5.spawns", 0),
            ("E5.formatting_off", 0),
            ("E7.merge_round_trips", 0),
            ("E7.document_loads", 0),
            ("E7.document_encodes", 0),
            ("E7.opens", 0),
            ("E7.prepares", 0),
            ("E7.transactions", 0),
            ("E7.syncs", 0),
            ("E7.encodes", 0),
            ("E2.configuration_reads", 0),
            ("E2.opens", 0),
            ("E2.prunes", 0),
            ("E2.transactions", 0),
            ("E2.syncs", 0),
            ("C6.file_opens", 0),
            ("C6.compilations", 0),
            ("C6.spawns", 0),
            ("C6.acknowledgements", 0),
            ("C6.seals", 0),
        ]
    );
}
