//! The `languages` family on `languages.test` (ADR 0073): a page per script
//! a tool must keep intact (Arabic and Hebrew with their marks, Japanese and
//! Chinese without spaces, Devanagari, Thai, combining marks served
//! decomposed, emoji sequences and astral characters), the Unicode record,
//! a page at a non-ASCII path and query, and one on the IDN host name
//! `bücher.languages.test`. Every page is UTF-8, and its expected text is
//! held in UTF-8 in the normalization form served. The text is drawn from
//! the seed out of hand-written phrases; the Unicode record is fixed.

use std::fmt::Write as _;

use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task};

/// The host name the family is served on.
pub(crate) const HOST: &str = "languages.test";

/// `bücher.languages.test`, in the ASCII form a URL and a `Host` header
/// carry (RFC 5891): the host name of the family's task 2.
pub(crate) const IDN_HOST: &str = "xn--bcher-kva.languages.test";

const FAMILY: &str = "languages";
const FAMILY_PLACE: u64 = 23;

/// The non-ASCII path and query of the page `path-and-query`, as written in
/// its link and as a browser sends them, percent-encoded in UTF-8.
const UNICODE_PATH: &str = "/wörter/日本語";
const UNICODE_QUERY: &str = "q=café&ort=Zürich";

/// The Unicode record (ADR 0073): bidirectional text with marks, the NFC
/// and NFD forms of one word, a ZWJ emoji sequence, an astral character, a
/// zero-width joiner, a BOM, and a tab and a newline inside the field.
const UNICODE_RECORD: &str = "\u{5E9}\u{5B8}\u{5C1}\u{5DC}\u{5D5}\u{5B9}\u{5DD}\u{200F} abc caf\u{E9} / cafe\u{301}\t\u{1F469}\u{200D}\u{1F4BB} \u{1D518}\na\u{200D}b \u{FEFF}end";

/// One page of the family, by its ID.
struct Script {
    id: &'static str,
    lang: &'static str,
    /// Written right to left.
    rtl: bool,
    title: &'static str,
    phrases: [&'static str; 4],
    /// Thai and Chinese and Japanese write no space between sentences.
    separator: &'static str,
}

/// The pages whose text is drawn from phrases, in the index's order.
static SCRIPTS: [Script; 9] = [
    Script {
        id: "arabic",
        lang: "ar",
        rtl: true,
        title: "العربية",
        phrases: [
            "مَرْحَبًا بِكُمْ",
            "اللُّغَةُ الْعَرَبِيَّةُ جَمِيلَةٌ",
            "السَّلَامُ عَلَيْكُمْ",
            "شُكْرًا جَزِيلًا",
        ],
        separator: " ",
    },
    Script {
        id: "hebrew",
        lang: "he",
        rtl: true,
        title: "עברית",
        phrases: ["שָׁלוֹם עוֹלָם", "תּוֹדָה רַבָּה", "בֹּקֶר טוֹב", "עִבְרִית הִיא שָׂפָה יָפָה"],
        separator: " ",
    },
    Script {
        id: "japanese",
        lang: "ja",
        rtl: false,
        title: "日本語",
        phrases: [
            "東京は日本の首都です。",
            "今日はいい天気ですね。",
            "駅まで歩いて十分かかります。",
            "この本はとても面白いです。",
        ],
        separator: "",
    },
    Script {
        id: "chinese",
        lang: "zh",
        rtl: false,
        title: "中文",
        phrases: [
            "北京是中国的首都。",
            "今天天气很好。",
            "我每天早上喝茶。",
            "这本书非常有趣。",
        ],
        separator: "",
    },
    Script {
        id: "hindi",
        lang: "hi",
        rtl: false,
        title: "हिन्दी",
        phrases: [
            "नमस्ते दुनिया",
            "हिन्दी एक सुंदर भाषा है",
            "मैं हर सुबह चाय पीता हूँ",
            "क्षत्रिय और ज्ञान",
        ],
        separator: " । ",
    },
    Script {
        id: "thai",
        lang: "th",
        rtl: false,
        title: "ภาษาไทย",
        phrases: [
            "สวัสดีครับ",
            "วันนี้อากาศดีมาก",
            "ฉันชอบกินข้าวผัด",
            "ภาษาไทยไม่มีช่องว่างระหว่างคำ",
        ],
        separator: "",
    },
    Script {
        id: "combining-marks",
        lang: "vi",
        rtl: false,
        title: "Combining marks",
        // Each letter with a diacritic is written decomposed (NFD), and the
        // page serves it so.
        phrases: [
            "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t",
            "cafe\u{301} cre\u{300}me bru\u{302}le\u{301}e",
            "man\u{303}ana in Zu\u{308}rich",
            "A\u{30A}ngstro\u{308}m is nai\u{308}ve",
        ],
        separator: " ",
    },
    Script {
        id: "emoji",
        lang: "en",
        rtl: false,
        title: "Emoji sequences",
        phrases: [
            "family \u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}",
            "flag \u{1F1EF}\u{1F1F5}",
            "thumbs up \u{1F44D}\u{1F3FD}",
            "keycap 1\u{FE0F}\u{20E3} and rainbow \u{1F3F3}\u{FE0F}\u{200D}\u{1F308}",
        ],
        separator: " ",
    },
    Script {
        id: "astral",
        lang: "en",
        rtl: false,
        title: "Astral characters",
        phrases: [
            "\u{1D518}\u{1D52B}\u{1D526}\u{1D520}\u{1D52C}\u{1D521}\u{1D522}",
            "\u{2000B}\u{20B9F}",
            "\u{13080} eye",
            "grin \u{1F600}",
        ],
        separator: " ",
    },
];

/// The German phrases of the page on the IDN host name.
const IDN_PHRASES: [&str; 4] = [
    "Willkommen in der Buchhandlung.",
    "Bücher für alle.",
    "Größe zählt nicht.",
    "Schöne Grüße aus Zürich.",
];

/// Two of `phrases`, drawn at `number`, then a reference number, so no two
/// seeds give a page the same text, joined by `separator`.
fn drawn(seed: u64, number: u64, phrases: &[&str], separator: &str) -> String {
    let reference = 10_000 + generator::below(seed, &[FAMILY_PLACE, number, 0], 90_000);
    let mut text: Vec<String> =
        generator::distinct(seed, &[FAMILY_PLACE, number], phrases.len(), 2)
            .into_iter()
            .map(|index| phrases[index].to_owned())
            .collect();
    text.push(reference.to_string());
    text.join(separator)
}

/// One page's Record: its ID, title and text.
struct Shown {
    id: &'static str,
    lang: &'static str,
    rtl: bool,
    title: &'static str,
    text: String,
    /// Whether its text keeps its tabs and newlines in a `<pre>`.
    preformatted: bool,
}

/// Every page the index links, in its order, with what each shows.
fn shown(seed: u64) -> Vec<Shown> {
    let mut pages: Vec<Shown> = SCRIPTS
        .iter()
        .zip(1..)
        .map(|(script, number)| Shown {
            id: script.id,
            lang: script.lang,
            rtl: script.rtl,
            title: script.title,
            text: drawn(seed, number, &script.phrases, script.separator),
            preformatted: false,
        })
        .collect();
    pages.push(Shown {
        id: "unicode-record",
        lang: "en",
        rtl: false,
        title: "The Unicode record",
        text: UNICODE_RECORD.to_owned(),
        preformatted: true,
    });
    pages.push(path_and_query(seed));
    pages
}

fn path_and_query(seed: u64) -> Shown {
    Shown {
        id: "path-and-query",
        lang: "de",
        rtl: false,
        title: "Wörter",
        text: format!(
            "Gesucht: café in Zürich. {}",
            drawn(seed, 100, &IDN_PHRASES, " ")
        ),
        preformatted: false,
    }
}

fn idn(seed: u64) -> Shown {
    Shown {
        id: "idn",
        lang: "de",
        rtl: false,
        title: "Bücher",
        text: format!(
            "{} bücher.languages.test",
            drawn(seed, 101, &IDN_PHRASES, " ")
        ),
        preformatted: false,
    }
}

/// The page at `path` and `query` (both as sent, percent-encoded) on host
/// `name`, generated on request; `None` when the family has no such page.
pub(crate) fn page(seed: u64, name: &str, path: &str, query: &str) -> Option<String> {
    if name == IDN_HOST {
        return (path == "/" && query.is_empty()).then(|| html(&idn(seed)));
    }
    if path == "/pages" && query.is_empty() {
        return Some(index(seed));
    }
    if percent_decoded(path)? == UNICODE_PATH {
        return (percent_decoded(query)? == UNICODE_QUERY).then(|| html(&path_and_query(seed)));
    }
    let id = path.strip_prefix("/pages/")?;
    if !query.is_empty() {
        return None;
    }
    shown(seed)
        .iter()
        .find(|page| page.id == id && page.id != "path-and-query")
        .map(html)
}

/// Text with each `%XX` turned into its byte, read as UTF-8; `None` when it
/// is not.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn index(seed: u64) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Languages</title></head>\n<body>\n<main>\n<h1>Languages</h1>\n<ul class=\"pages\">\n",
    );
    for page in shown(seed) {
        let href = if page.id == "path-and-query" {
            format!("{UNICODE_PATH}?{}", UNICODE_QUERY.replace('&', "&amp;"))
        } else {
            format!("/pages/{}", page.id)
        };
        writeln!(
            html,
            "<li><a data-id=\"{id}\" href=\"{href}\">{title}</a></li>",
            id = page.id,
            title = page.title
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n</main>\n</body>\n</html>\n");
    html
}

fn html(page: &Shown) -> String {
    let Shown {
        lang, title, text, ..
    } = page;
    let dir = if page.rtl { "rtl" } else { "ltr" };
    let element = if page.preformatted { "pre" } else { "p" };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\" dir=\"{dir}\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n<h1 class=\"title\">{title}</h1>\n<{element} class=\"text\">{text}</{element}>\n</main>\n</body>\n</html>\n"
    )
}

/// The host name of the family's task 1.
fn task_host() -> String {
    format!("1.{HOST}")
}

/// The family's tasks for `seed`, on their own host names and `port`.
/// Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    let record = |page: &Shown| {
        Record::from([
            ("id", page.id.to_owned()),
            ("title", page.title.to_owned()),
            ("text", page.text.clone()),
        ])
    };
    vec![
        Task {
            host: task_host(),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: format!("http://{}:{port}/pages", task_host()),
                description: "Crawl every page the index links, each link as written (its path \
                    and query in Unicode), and return one Record per page with the fields `id` \
                    (the link's `data-id`), `title` and `text`, each text exactly as the page \
                    holds it: in UTF-8, in the normalization form served, its tabs and newlines \
                    kept.",
            },
            expected: Expected {
                records: shown(seed).iter().map(record).collect(),
                effects: Vec::new(),
            },
        },
        Task {
            host: IDN_HOST.to_owned(),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("http://{IDN_HOST}:{port}/"),
                description: "Extract the page on the IDN host name `bücher.languages.test` as \
                    one Record with the fields `id` (`idn`), `title` and `text`.",
            },
            expected: Expected {
                records: vec![record(&idn(seed))],
                effects: Vec::new(),
            },
        },
    ]
}
