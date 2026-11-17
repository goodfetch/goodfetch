use goodfetch_test_support::{ArticleAnswer, ArticleAnswers, ArticleLink};
use serde_json::{Value, json};

use super::{Answers, Differing, Extracted, Fielded, Page, Written, check, check_fields, read};

const HEADER: &str = "Tidewater Supply masthead";
const NAV: &str = "Catalogue index navigation";
const ASIDE: &str = "Related reading sidebar";
const FOOTER: &str = "Tidewater Supply imprint and terms";
const AD: &str = "Sponsored offer from Harbourline Freight";

/// The right answers of a site of three pages, as the Proving ground
/// serves them: read here from its own format, so a field renamed there
/// fails this file's tests.
fn answers() -> Answers {
    let page = |number: u32, title: &str, paragraphs: &[&str]| ArticleAnswer {
        path: format!("/sites/3/{number}"),
        title: title.to_owned(),
        paragraphs: paragraphs.iter().map(|&text| text.to_owned()).collect(),
        name: NAMES[number as usize - 1].to_owned(),
        children: if number == 1 {
            vec![link(2), link(3)]
        } else {
            Vec::new()
        },
    };
    let served = ArticleAnswers {
        pages: vec![
            page(
                1,
                "Brass lantern",
                &["Brass lantern is piece number 1.", "It ships in a crate."],
            ),
            page(2, "Oak barrel & Co.", &["Oak barrel is piece number 2."]),
            page(
                3,
                "Rope coil \u{2014} new",
                &["Rope coil is piece number 3."],
            ),
        ],
        boilerplate: [HEADER, NAV, ASIDE, FOOTER].map(str::to_owned).to_vec(),
        ad: AD.to_owned(),
    };
    serde_json::from_value(serde_json::to_value(served).unwrap()).unwrap()
}

/// The three pages' names, as their headings hold them.
const NAMES: [&str; 3] = ["Brass lantern", "Oak barrel", "Rope coil"];

fn link(number: u32) -> ArticleLink {
    ArticleLink {
        path: format!("/sites/3/{number}"),
        name: NAMES[number as usize - 1].to_owned(),
    }
}

fn page(number: u32, title: &str, text: &str) -> Page {
    Page {
        url: format!("http://article.localhost:4000/sites/3/{number}"),
        title: Some(title.to_owned()),
        text: Some(text.to_owned()),
    }
}

/// The three pages as a tool that reads each one right writes them.
fn right() -> Vec<Page> {
    vec![
        page(
            1,
            "Brass lantern",
            "Brass lantern is piece number 1. It ships in a crate.",
        ),
        page(2, "Oak barrel & Co.", "Oak barrel is piece number 2."),
        page(3, "Rope coil \u{2014} new", "Rope coil is piece number 3."),
    ]
}

fn differing(pages: u64, first: &str) -> Differing {
    Differing {
        pages,
        first: Some(first.to_owned()),
    }
}

/// A page reads right when its title equals the right one once white
/// space, Unicode's included, is collapsed on it, and its text holds every
/// paragraph, however the tool laid the text out around them: its heading
/// before them, line breaks between and inside them.
#[test]
fn a_title_equal_once_collapsed_and_a_text_holding_every_paragraph_read_right() {
    let written = vec![
        page(
            1,
            "  Brass \n lantern ",
            "Brass lantern\n\nBrass lantern is piece\nnumber 1.\n\n  It ships in a crate.\n",
        ),
        page(2, "Oak barrel & Co.", "Oak barrel is piece number 2."),
        page(
            3,
            "Rope coil\u{a0}\u{2014} new",
            "Rope coil is piece number 3.",
        ),
    ];

    assert_eq!(check(&answers(), &written), Extracted::default());
}

/// The checker decodes nothing: a title whose character reference the tool
/// left as written differs, and the first difference names the page, what
/// was written and the right title.
#[test]
fn a_title_left_undecoded_differs() {
    let mut written = right();
    written[1] = page(2, "Oak barrel &amp; Co.", "Oak barrel is piece number 2.");

    let first = "/sites/3/2: title \"Oak barrel &amp; Co.\", not \"Oak barrel & Co.\"";
    assert_eq!(
        check(&answers(), &written),
        Extracted {
            whole: differing(1, first),
            ad_allowed: differing(1, first),
        }
    );
}

/// A text that lacks a paragraph, one that holds a marker of the page's
/// boilerplate, a page written with no title or no text and a page never
/// written each differ; the first difference is the first page's, in the
/// site's order, whatever order the tool wrote its pages in.
#[test]
fn a_missing_paragraph_boilerplate_and_a_page_not_written_differ() {
    let lacking = vec![
        page(3, "Rope coil \u{2014} new", "Rope coil is piece number 3."),
        page(2, "Oak barrel & Co.", "Oak barrel is piece number 2."),
        page(1, "Brass lantern", "Brass lantern is piece number 1."),
    ];
    let boilerplate = vec![
        page(
            1,
            "Brass lantern",
            "Brass lantern is piece number 1. It ships in a crate.",
        ),
        page(
            2,
            "Oak barrel & Co.",
            &format!("Oak barrel is piece number 2. {FOOTER}"),
        ),
    ];
    let empty = vec![
        Page {
            url: "http://article.localhost:4000/sites/3/1".to_owned(),
            title: None,
            text: Some("Brass lantern is piece number 1. It ships in a crate.".to_owned()),
        },
        Page {
            url: "http://article.localhost:4000/sites/3/2".to_owned(),
            title: Some("Oak barrel & Co.".to_owned()),
            text: None,
        },
        page(3, "Rope coil \u{2014} new", "Rope coil is piece number 3."),
    ];

    let firsts = [lacking, boilerplate, empty].map(|written| check(&answers(), &written).whole);

    assert_eq!(
        firsts,
        [
            differing(1, "/sites/3/1: the text lacks paragraph 2"),
            differing(
                2,
                "/sites/3/2: the text holds \"Tidewater Supply imprint and terms\""
            ),
            differing(2, "/sites/3/1: no title, not \"Brass lantern\""),
        ]
    );
    assert_eq!(
        check(&answers(), &right()[..2]).whole,
        differing(1, "/sites/3/3: no page written")
    );
}

/// The ad's marker in a text differs under the whole check, and not once
/// the ad is allowed: what a tool that removes no ad is held to. A page
/// that differs otherwise still differs there.
#[test]
fn the_ads_marker_differs_unless_the_ad_is_allowed() {
    let mut written = right();
    written[0] = page(
        1,
        "Brass lantern",
        &format!("Brass lantern is piece number 1. {AD}: half price. It ships in a crate."),
    );
    written[2] = page(
        3,
        "Rope coil",
        &format!("Rope coil is piece number 3. {AD}"),
    );

    assert_eq!(
        check(&answers(), &written),
        Extracted {
            whole: differing(
                2,
                "/sites/3/1: the text holds the ad's \"Sponsored offer from Harbourline Freight\""
            ),
            ad_allowed: differing(
                1,
                "/sites/3/3: title \"Rope coil\", not \"Rope coil \u{2014} new\""
            ),
        }
    );
}

/// A tool's pages are read from what it left in its data folder: a file of
/// JSON lines, or the files of Crawlee's default Dataset, each page's
/// `url`, `title` and `text`, a null or absent one read as none.
#[test]
fn a_tools_pages_are_read_from_its_lines_or_its_dataset() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("pages.jsonl"),
        "{\"url\":\"http://h/sites/3/1\",\"title\":null,\"text\":\"one\"}\n\
         {\"url\":\"http://h/sites/3/2\",\"title\":\"Two\",\"text\":\"two\"}\n",
    )
    .unwrap();
    let dataset = home.path().join("storage/datasets/default");
    std::fs::create_dir_all(&dataset).unwrap();
    std::fs::write(
        dataset.join("000000002.json"),
        "{\n  \"url\": \"http://h/sites/3/2\",\n  \"title\": \"Two\",\n  \"text\": \"two\"\n}",
    )
    .unwrap();
    std::fs::write(
        dataset.join("000000001.json"),
        "{ \"url\": \"http://h/sites/3/1\", \"text\": \"one\" }",
    )
    .unwrap();
    std::fs::write(dataset.join("__metadata__.json"), "{ \"id\": \"default\" }").unwrap();

    let expected = vec![
        Page {
            url: "http://h/sites/3/1".to_owned(),
            title: None,
            text: Some("one".to_owned()),
        },
        Page {
            url: "http://h/sites/3/2".to_owned(),
            title: Some("Two".to_owned()),
            text: Some("two".to_owned()),
        },
    ];
    assert_eq!(
        (
            read(Written::Lines("pages.jsonl"), home.path()).unwrap(),
            read(Written::Dataset, home.path()).unwrap(),
        ),
        (expected.clone(), expected)
    );
}

/// A tool that left no pages where it writes them fails the read, naming
/// the file: a crawl that wrote nothing is no page that differs.
#[test]
fn a_tool_that_left_no_file_fails_the_read() {
    let home = tempfile::tempdir().unwrap();

    let error = read::<Page>(Written::Lines("items.jsonl"), home.path()).unwrap_err();

    assert!(format!("{error:#}").contains("items.jsonl"), "{error:#}");
}

/// Page `number`'s twenty fields as a handler that reads each one right
/// writes them: page 1 links to pages 2 and 3, which link to none.
fn fielded(number: u32) -> Fielded {
    let answers = answers();
    let answer = &answers.pages[number as usize - 1];
    let child = |index: usize, value: fn(&super::Link) -> &str| {
        answer
            .children
            .get(index)
            .map_or(Value::Null, |link| json!(value(link)))
    };
    let mut fields = json!({
        "title": answer.title,
        "name": answer.name,
        "id": format!("S-{number:07}"),
        "home": "/sites/3/1",
        "home_label": "Home",
        "header": HEADER,
        "nav": NAV,
        "footer": FOOTER,
        "first_child": child(0, |link| &link.path),
        "first_child_name": child(0, |link| &link.name),
        "second_child": child(1, |link| &link.path),
        "second_child_name": child(1, |link| &link.name),
    });
    for k in 1..=8 {
        fields[format!("paragraph_{k}")] = answer
            .paragraphs
            .get(k - 1)
            .map_or(Value::Null, |paragraph| json!(paragraph));
    }
    Fielded {
        url: format!("http://article.localhost:4000/sites/3/{number}"),
        fields: fields.as_object().unwrap().clone(),
    }
}

/// A handled page reads right when each of its twenty fields is its
/// answer: text once white space is collapsed on it, a link by its path
/// whatever its origin, and a field the page has no element for null or
/// absent.
#[test]
fn a_pages_twenty_fields_read_right_when_each_is_its_answer() {
    let mut written: Vec<Fielded> = (1..=3).map(fielded).collect();
    written[0]
        .fields
        .insert("name".to_owned(), json!("  Brass\n lantern "));
    written[0].fields.insert(
        "first_child".to_owned(),
        json!("http://article.localhost:4000/sites/3/2"),
    );
    written[1].fields.remove("paragraph_8");
    written[1].fields.remove("second_child");

    assert_eq!(check_fields(&answers(), &written), Extracted::default());
}

/// A field that is not its answer, one the page has an element for but
/// the tool left out, and one the page has none for that the tool wrote,
/// each differ, as does a page never written; the first difference is the
/// first page's in the site's order and names the field.
#[test]
fn a_field_that_is_not_its_answer_differs() {
    let mut written: Vec<Fielded> = (1..=3).map(fielded).collect();
    written[0]
        .fields
        .insert("paragraph_3".to_owned(), json!("A third paragraph."));
    written[1].fields.insert("name".to_owned(), json!("Oak"));
    written[2].fields.remove("paragraph_1");
    let differs = check_fields(&answers(), &written);
    let missing = check_fields(&answers(), &written[1..]);

    let first = "/sites/3/1: paragraph_3 \"A third paragraph.\", not none";
    assert_eq!(
        (differs, missing.whole),
        (
            Extracted {
                whole: differing(3, first),
                ad_allowed: differing(3, first),
            },
            differing(3, "/sites/3/1: no page written"),
        )
    );
}
