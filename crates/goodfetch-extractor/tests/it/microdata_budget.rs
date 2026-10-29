//! One call's microdata does a bounded amount of work, whatever the page's
//! items share (spec #580, finding 580-1): items that reach one container
//! through `itemref` each read it again, so a page of a few kilobytes holds
//! exponentially many items, and past the call's budget each value not read
//! yet is written `"ERROR"`. A page past the budget reads the whole budget,
//! some tenths of a second in an unoptimized build, so these tests run in
//! the Pre-merge suite: the property test reads 32 such pages, seconds in
//! all, where a Quick-suite test is marked slow after one (#1744).

use goodfetch_extractor::{Context, Document, Serialization, formats};
use goodfetch_protocol::page::{MicrodataItem, MicrodataValue};
use serde_json::json;

/// The document's URL.
const URL: &str = "https://kites.test/journal/bridle";

/// The microdata items of `html` at [`URL`].
fn microdata(html: &str) -> Vec<MicrodataItem> {
    formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        &serde_json::from_value(json!({ "metadata": true })).unwrap(),
        &Context::default(),
    )
    .metadata
    .unwrap()
    .microdata
}

/// A page whose top-level item reaches, through `itemref`, a chain of
/// `levels` containers, each holding `width` items whose `itemprop` has
/// `names` tokens and whose `itemref` names the next container: each item
/// reaches every item of the next container, so the items multiply by
/// `width` a level, and their values by `names`, though each container is
/// written once.
fn shared_containers(levels: usize, width: usize, names: usize) -> String {
    let names = (0..names)
        .map(|name| format!("n{name}"))
        .collect::<Vec<_>>()
        .join(" ");
    let item = |level: usize| {
        format!(
            r#"<div itemprop="{names}" itemscope itemref="c{}"></div>"#,
            level + 1
        )
    };
    let containers = (0..levels)
        .map(|level| format!(r#"<div id="c{level}">{}</div>"#, item(level).repeat(width)))
        .collect::<Vec<_>>()
        .concat();
    format!(r#"<div itemscope itemref="c0"></div>{containers}"#)
}

/// What a reading wrote: its values other than `"ERROR"`, nested items
/// included, the bytes of their text, and its `"ERROR"` values.
#[derive(Debug, Default)]
struct Written {
    values: usize,
    text: usize,
    errors: usize,
}

impl Written {
    fn of(items: &[MicrodataItem]) -> Self {
        let mut written = Self::default();
        for item in items {
            written.add(item);
        }
        written
    }

    fn add(&mut self, item: &MicrodataItem) {
        for value in item.properties.values().flatten() {
            match value {
                MicrodataValue::Text(text) if text == "ERROR" => self.errors += 1,
                MicrodataValue::Text(text) => {
                    self.values += 1;
                    self.text += text.len();
                }
                MicrodataValue::Item(item) => {
                    self.values += 1;
                    self.add(item);
                }
            }
        }
    }
}

/// Covers: story:6-6
#[test]
fn items_that_share_their_containers_are_read_whole_within_the_budget() {
    let html = shared_containers(3, 2, 1);

    let written = Written::of(&microdata(&html));

    assert_eq!((written.values, written.errors), (14, 0), "{written:?}");
}

/// Covers: story:6-6
#[test]
fn a_page_whose_items_share_their_containers_writes_at_most_the_budget_and_error_past_it() {
    // 2.2 KB of HTML whose items, read whole, would be 2^19 - 1.
    let html = shared_containers(18, 2, 1);

    let written = Written::of(&microdata(&html));

    assert!(
        written.values <= 100_000 && written.text <= 4 << 20 && written.errors > 0,
        "{written:?}"
    );
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        // Each case past the budget reads up to the whole budget.
        cases: 32,
        ..goodfetch_test_support::proptest_config()
    })]

    /// Whatever the page's items share, and however many names each
    /// property has, one call writes at most the budget's values and text.
    /// Covers: story:6-6
    #[test]
    fn microdata_stays_within_the_budget_whatever_the_items_share(
        levels in 1_usize..=24,
        width in 1_usize..=3,
        names in 1_usize..=3,
    ) {
        let html = shared_containers(levels, width, names);

        let written = Written::of(&microdata(&html));

        proptest::prop_assert!(
            written.values <= 100_000 && written.text <= 4 << 20,
            "{:?}",
            written
        );
    }
}
