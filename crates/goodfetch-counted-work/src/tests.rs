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

/// Each row's name, in `Row::ALL`'s order.
const NAMES: [&str; 101] = [
    "F1.parses",
    "F1.selector_compilations",
    "F1.readability_runs",
    "F1.file_opens",
    "F1.locale_form_builds",
    "F1.plugin_serializations",
    "F1.plugin_answer_parses",
    "F1.tree_copies",
    "F1.share_readability_runs",
    "F1.block_classifications",
    "E1.deep_copies",
    "E1.encodes",
    "F5.messages_no_hook",
    "F5.calls",
    "F5.builds",
    "F6.opens",
    "F6.transactions",
    "F6.writer_reads",
    "F4.declaration_reads",
    "F4.validator_builds",
    "F4.validations",
    "F4.serializations",
    "F4.deep_copies",
    "F4.constant_lookups",
    "F4.transactions",
    "F3.invariant_reads",
    "F3.compilations",
    "F3.transactions",
    "F3.invariant_reads.hop",
    "F3.compilations.hop",
    "F3.transactions.hop",
    "F3.invariant_reads.sitemap",
    "F3.compilations.sitemap",
    "F3.transactions.sitemap",
    "F3.compilations.payload",
    "F3.payload_serializations",
    "F3.validations",
    "C1.encodes",
    "C1.sends",
    "C1.file_opens",
    "C1.log_sends_off",
    "C3.script_sends",
    "C3.check_commands",
    "C3.commands.click",
    "C3.commands.fill",
    "C3.commands.press",
    "F2.transactions",
    "F2.writer_reads",
    "F2.configuration_reads",
    "F2.prunes",
    "F2.effect_syncs",
    "E3.statements",
    "E3.slice_transactions",
    "E3.opens",
    "C4.detection_reads",
    "C4.compilations",
    "C4.script_compilations",
    "C4.provenance_entries",
    "E7.merge_round_trips",
    "E7.document_loads",
    "E7.document_encodes",
    "E7.opens",
    "E7.prepares",
    "E7.transactions",
    "E7.syncs",
    "E7.encodes",
    "E2.configuration_reads",
    "E2.opens",
    "E2.prunes",
    "E2.transactions",
    "E2.syncs",
    "C6.file_opens",
    "C6.compilations",
    "C6.spawns",
    "C6.acknowledgements",
    "C6.seals",
    "E5.file_opens",
    "E5.spawns",
    "E5.formatting_off",
    "E6.transactions",
    "E6.opens",
    "E6.script_sends",
    "E6.robots_parses",
    "E6.robots_reads",
    "C2.commands",
    "C2.world_creations",
    "C2.document_worlds",
    "C2.serialization_commands",
    "E4.configuration_reads",
    "E4.meminfo_opens",
    "E4.static_opens",
    "E4.cpu_stat_opens",
    "E4.process_reads",
    "E4.pool_walks",
    "C5.fetch_messages_no_rule",
    "C5.spawns",
    "C5.file_opens",
    "C5.matches",
    "C5.answers",
    "C5.compilations",
    "C5.entries",
];

/// `Row::ALL` sizes the counts, so a row left out of it would index past them.
#[test]
fn each_row_counts_on_its_own() {
    let work = CountedWork::default();

    work.count(Row::SinkWriterReads);

    assert_eq!(Row::ALL.map(Row::name), NAMES);
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

/// The cache holds at most [`super::CAPACITY`] compilations: one over it
/// evicts one kept before rather than grows.
#[test]
fn a_compilation_over_the_bound_evicts_rather_than_grows() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    let over = super::CAPACITY + 64;
    for class in 0..over {
        let _ = work.selector(row, &format!(".c{class}"));
    }

    assert_eq!(
        (work.kept() <= super::CAPACITY, work.read(row)),
        (true, over)
    );
}
