//! The `encodings` family on `encodings.test` (ADRs 0073 and 0119, CO-24):
//! one page per case of the encodings ADR 0073 lists and of the labels and
//! declarations ADR 0119 reads, each written in its encoding, and the
//! expected text of each held in UTF-8 with the encoding ADR 0119's order
//! reads it with and where that encoding comes from, as Provenance records
//! it. The text is drawn from the seed out of hand-written phrases.

use std::fmt::Write as _;

use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};
use encoding_rs::{
    BIG5, EUC_KR, Encoding, GB18030, ISO_8859_2, ISO_8859_15, SHIFT_JIS, UTF_8, UTF_16LE,
    WINDOWS_1251, WINDOWS_1252,
};

/// The host name the family is served on.
pub(crate) const HOST: &str = "encodings.test";

const FAMILY: &str = "encodings";
const FAMILY_PLACE: u64 = 22;

/// The phrases a page's text is drawn from, in one language.
#[derive(Clone, Copy)]
enum Language {
    Japanese,
    Korean,
    SimplifiedChinese,
    TraditionalChinese,
    Russian,
    Polish,
    French,
    /// English text with the euro sign, which windows-1252 writes as byte
    /// 0x80.
    EnglishWithEuro,
}

impl Language {
    fn tag(self) -> &'static str {
        match self {
            Self::Japanese => "ja",
            Self::Korean => "ko",
            Self::SimplifiedChinese => "zh-Hans",
            Self::TraditionalChinese => "zh-Hant",
            Self::Russian => "ru",
            Self::Polish => "pl",
            Self::French => "fr",
            Self::EnglishWithEuro => "en",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Japanese => "日本語",
            Self::Korean => "한국어",
            Self::SimplifiedChinese => "简体中文",
            Self::TraditionalChinese => "繁體中文",
            Self::Russian => "Русский язык",
            Self::Polish => "Język polski",
            Self::French => "Le français",
            Self::EnglishWithEuro => "Prices in €",
        }
    }

    fn phrases(self) -> [&'static str; 6] {
        match self {
            Self::Japanese => [
                "東京は日本の首都です。",
                "今日はいい天気ですね。",
                "私は毎朝コーヒーを飲みます。",
                "この本はとても面白いです。",
                "駅まで歩いて十分かかります。",
                "日本語の文には空白がありません。",
            ],
            Self::Korean => [
                "서울은 한국의 수도입니다.",
                "오늘은 날씨가 좋습니다.",
                "저는 매일 아침 커피를 마십니다.",
                "이 책은 정말 재미있습니다.",
                "역까지 걸어서 십 분 걸립니다.",
                "한글은 아름다운 글자입니다.",
            ],
            Self::SimplifiedChinese => [
                "北京是中国的首都。",
                "今天天气很好。",
                "我每天早上喝茶。",
                "这本书非常有趣。",
                "走到车站要十分钟。",
                "中文句子里没有空格。",
            ],
            Self::TraditionalChinese => [
                "台北是台灣的城市。",
                "今天天氣很好。",
                "我每天早上喝茶。",
                "這本書非常有趣。",
                "走到車站要十分鐘。",
                "中文句子裡沒有空格。",
            ],
            Self::Russian => [
                "Москва — столица России.",
                "Сегодня хорошая погода.",
                "Я пью чай каждое утро.",
                "Эта книга очень интересная.",
                "До вокзала десять минут пешком.",
                "Съешь же ещё этих мягких булок.",
            ],
            Self::Polish => [
                "Zażółć gęślą jaźń.",
                "Dziś jest piękna pogoda.",
                "Kraków leży nad Wisłą.",
                "Łódź to duże miasto.",
                "Pchnąć w tę łódź jeża.",
                "Źrebię śpi na łące.",
            ],
            Self::French => [
                "Le cœur a ses raisons.",
                "Ce gâteau coûte 5 €.",
                "L'œuvre est déjà là.",
                "Où est la bibliothèque ?",
                "Un bœuf mange du foin.",
                "Le café coûte 2 € à Noël.",
            ],
            Self::EnglishWithEuro => [
                "The ticket costs 80 €.",
                "A café au lait is 3 €.",
                "The naïve fee is € 12.",
                "Your résumé costs 0 €.",
                "Delivery adds 5 € per box.",
                "The room is 120 € a night.",
            ],
        }
    }

    /// Japanese and Chinese write sentences with no space between them.
    fn separator(self) -> &'static str {
        match self {
            Self::Japanese | Self::SimplifiedChinese | Self::TraditionalChinese => "",
            _ => " ",
        }
    }
}

/// Where a page's encoding comes from, as Provenance records it (ADR
/// 0119).
#[derive(Clone, Copy)]
enum Source {
    Bom,
    Header,
    Meta,
    Utf8,
    /// The browser's guess, when nothing is declared and the body is not
    /// UTF-8.
    Detected,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Bom => "bom",
            Self::Header => "header",
            Self::Meta => "meta",
            Self::Utf8 => "utf8",
            Self::Detected => "detected",
        }
    }
}

/// The `<meta>` a page declares its encoding with, and where it stands.
#[derive(Clone, Copy)]
enum Meta {
    None,
    /// `<meta charset>` first in the head.
    Charset(&'static str),
    /// `<meta http-equiv="Content-Type">` first in the head.
    HttpEquiv(&'static str),
    /// `<meta charset>` after more than 1,024 bytes of other head tags.
    LateInHead(&'static str),
    /// `<meta charset>` in the body, past its first 1,024 bytes.
    AfterBody(&'static str),
}

/// One page of the family.
struct Case {
    id: &'static str,
    language: Language,
    content_type: &'static str,
    meta: Meta,
    /// The encoding its bytes are written in, which ADR 0119's order also
    /// reads it with; a UTF-16LE page starts with its BOM.
    written: &'static Encoding,
    /// Where ADR 0119's order finds that encoding.
    source: Source,
    /// Whether one byte of its text is one no UTF-8 decoder accepts.
    undecodable: bool,
}

const fn case(
    id: &'static str,
    language: Language,
    content_type: &'static str,
    meta: Meta,
    written: &'static Encoding,
    source: Source,
) -> Case {
    Case {
        id,
        language,
        content_type,
        meta,
        written,
        source,
        undecodable: false,
    }
}

/// Every page, in the index's order.
static CASES: [Case; 18] = [
    case(
        "shift-jis",
        Language::Japanese,
        "text/html; charset=Shift_JIS",
        Meta::None,
        SHIFT_JIS,
        Source::Header,
    ),
    case(
        "euc-kr",
        Language::Korean,
        "text/html; charset=EUC-KR",
        Meta::None,
        EUC_KR,
        Source::Header,
    ),
    case(
        "gb18030",
        Language::SimplifiedChinese,
        "text/html",
        Meta::Charset("GB18030"),
        GB18030,
        Source::Meta,
    ),
    case(
        "big5",
        Language::TraditionalChinese,
        "text/html; charset=Big5",
        Meta::None,
        BIG5,
        Source::Header,
    ),
    case(
        "windows-1251",
        Language::Russian,
        "text/html",
        Meta::HttpEquiv("windows-1251"),
        WINDOWS_1251,
        Source::Meta,
    ),
    case(
        "iso-8859-2",
        Language::Polish,
        "text/html; charset=ISO-8859-2",
        Meta::None,
        ISO_8859_2,
        Source::Header,
    ),
    case(
        "iso-8859-15",
        Language::French,
        "text/html; charset=ISO-8859-15",
        Meta::None,
        ISO_8859_15,
        Source::Header,
    ),
    case(
        "utf-16-bom",
        Language::French,
        "text/html",
        Meta::None,
        UTF_16LE,
        Source::Bom,
    ),
    case(
        "contradicting-meta",
        Language::Russian,
        "text/html; charset=windows-1251",
        Meta::Charset("utf-8"),
        WINDOWS_1251,
        Source::Header,
    ),
    case(
        "undeclared-utf-8",
        Language::Japanese,
        "text/html",
        Meta::None,
        UTF_8,
        Source::Utf8,
    ),
    case(
        "undeclared-legacy",
        Language::Japanese,
        "text/html",
        Meta::None,
        SHIFT_JIS,
        Source::Detected,
    ),
    case(
        "latin1-label",
        Language::EnglishWithEuro,
        "text/html; charset=latin1",
        Meta::None,
        WINDOWS_1252,
        Source::Header,
    ),
    case(
        "ascii-label",
        Language::EnglishWithEuro,
        "text/html",
        Meta::Charset("ascii"),
        WINDOWS_1252,
        Source::Meta,
    ),
    case(
        "meta-utf-16-label",
        Language::French,
        "text/html",
        Meta::Charset("utf-16"),
        UTF_8,
        Source::Meta,
    ),
    case(
        "x-user-defined",
        Language::EnglishWithEuro,
        "text/html",
        Meta::Charset("x-user-defined"),
        WINDOWS_1252,
        Source::Meta,
    ),
    case(
        "late-meta-in-head",
        Language::Russian,
        "text/html",
        Meta::LateInHead("windows-1251"),
        WINDOWS_1251,
        Source::Meta,
    ),
    case(
        "meta-after-body",
        Language::Russian,
        "text/html",
        Meta::AfterBody("windows-1251"),
        UTF_8,
        Source::Utf8,
    ),
    Case {
        undecodable: true,
        ..case(
            "undecodable-byte",
            Language::French,
            "text/html; charset=utf-8",
            Meta::None,
            UTF_8,
            Source::Header,
        )
    },
];

/// A page's title and text, in UTF-8, as a browser shows them.
fn text(seed: u64, number: u64, case: &Case) -> (&'static str, String) {
    let language = case.language;
    let phrases = language.phrases();
    // The legacy page with nothing declared holds every phrase, enough
    // text for a browser's detector to be sure of its encoding.
    let count = match case.source {
        Source::Detected => phrases.len(),
        _ => 2,
    };
    let chosen: Vec<&str> =
        generator::distinct(seed, &[FAMILY_PLACE, number], phrases.len(), count)
            .into_iter()
            .map(|index| phrases[index])
            .collect();
    // A reference number, so no two seeds give a page the same text.
    let reference = 10_000 + generator::below(seed, &[FAMILY_PLACE, number, 0], 90_000);
    let separator = language.separator();
    let mut text = format!("{}{separator}{reference}", chosen.join(separator));
    if case.undecodable {
        // The byte written in its place decodes to U+FFFD.
        text.insert(text.find(' ').unwrap_or(0), '\u{FFFD}');
    }
    (language.title(), text)
}

/// The page `path`, as its bytes and `Content-Type`; `None` when the
/// family has no such page.
pub(crate) fn page(seed: u64, path: &str) -> Option<(&'static str, Vec<u8>)> {
    if path == "/pages" {
        return Some(("text/html; charset=utf-8", index().into_bytes()));
    }
    if path == XHTML_PATH {
        return Some(("application/xhtml+xml", xhtml()));
    }
    let id = path.strip_prefix("/pages/")?;
    let (number, case) = CASES
        .iter()
        .zip(1..)
        .find_map(|(case, number)| (case.id == id).then_some((number, case)))?;
    Some((case.content_type, encoded_page(seed, number, case)))
}

/// The XHTML page no index links and no task reads (ADR 0119; deferred nit
/// of #550): `application/xhtml+xml` in windows-1251, declared only in its
/// XML declaration, so every tier reads it without escalating.
const XHTML_PATH: &str = "/xhtml/declared";

/// The XHTML page's title and text, as a browser shows them.
const XHTML_TITLE: &str = "Русский язык";
const XHTML_TEXT: &str = "Москва — столица России.";

/// The XHTML page's bytes.
fn xhtml() -> Vec<u8> {
    let page = format!(
        "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"ru\">\n<head><title>{XHTML_TITLE}</title></head>\n<body>\n<main>\n<h1 class=\"title\">{XHTML_TITLE}</h1>\n<p class=\"text\">{XHTML_TEXT}</p>\n</main>\n</body>\n</html>\n"
    );
    let (bytes, _, unmappable) = WINDOWS_1251.encode(&page);
    assert!(
        !unmappable,
        "invariant: the XHTML page is written in windows-1251"
    );
    bytes.into_owned()
}

fn index() -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Encodings</title></head>\n<body>\n<main>\n<h1>Encodings</h1>\n<ul class=\"pages\">\n",
    );
    for case in &CASES {
        writeln!(
            html,
            "<li><a href=\"/pages/{id}\">{id}</a></li>",
            id = case.id
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n</main>\n</body>\n</html>\n");
    html
}

/// The page's markup, then its bytes in its encoding.
fn encoded_page(seed: u64, number: u64, case: &Case) -> Vec<u8> {
    let (title, text) = text(seed, number, case);
    let declared = |meta| match meta {
        Meta::Charset(label) => format!("<meta charset=\"{label}\">\n"),
        Meta::HttpEquiv(label) => {
            format!("<meta http-equiv=\"Content-Type\" content=\"text/html; charset={label}\">\n")
        }
        Meta::None | Meta::LateInHead(_) | Meta::AfterBody(_) => String::new(),
    };
    let mut head = declared(case.meta);
    if let Meta::LateInHead(label) = case.meta {
        head.push_str(&filler(
            "<meta name=\"keywords\" content=\"proving ground\">\n",
        ));
        writeln!(head, "<meta charset=\"{label}\">")
            .expect("invariant: writing to a String succeeds");
    }
    let mut body = String::new();
    if let Meta::AfterBody(label) = case.meta {
        body.push_str(&filler("<p class=\"filler\">Proving ground.</p>\n"));
        writeln!(body, "<meta charset=\"{label}\">")
            .expect("invariant: writing to a String succeeds");
    }
    let html = format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n{head}<title>{title}</title>\n</head>\n<body>\n{body}<main>\n<h1 class=\"title\">{title}</h1>\n<p class=\"text\">{text}</p>\n</main>\n</body>\n</html>\n",
        lang = case.language.tag(),
    );
    encode(&html, case)
}

/// `line` repeated past the first 1,024 bytes of a page.
fn filler(line: &str) -> String {
    line.repeat(1024 / line.len() + 2)
}

fn encode(html: &str, case: &Case) -> Vec<u8> {
    if case.written == UTF_16LE {
        // encoding_rs writes no UTF-16, as the WHATWG Encoding standard has
        // no UTF-16 encoder.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(html.encode_utf16().flat_map(u16::to_le_bytes));
        return bytes;
    }
    let (bytes, _, unmappable) = case.written.encode(html);
    assert!(
        !unmappable || case.undecodable,
        "invariant: every phrase of {} is written in {}",
        case.id,
        case.written.name()
    );
    let mut bytes = bytes.into_owned();
    if case.undecodable {
        let replacement = "\u{FFFD}".as_bytes();
        let at = bytes
            .windows(replacement.len())
            .position(|window| window == replacement)
            .expect("invariant: the undecodable page's text holds U+FFFD");
        bytes.splice(at..at + replacement.len(), [0xFF]);
    }
    bytes
}

/// The host name of the family's task `ordinal`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, on their own host names and `port`.
/// Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let start_url = |ordinal| format!("{}/pages", origin(&task_host(ordinal), port));
    vec![
        Task {
            host: task_host(1),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: start_url(1),
                description: "Crawl every page the index links and return one Record per page, \
                    with the fields `id` (the last segment of its path), `title` and `text`, \
                    the text each page shows, in UTF-8.",
                account: None,
                time_steps: None,
            },
            expected: Expected::records(records(seed, false)),
        },
        Task {
            host: task_host(2),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: start_url(2),
                description: "Crawl every page the index links and return one Record per page, \
                    with the fields `id` (the last segment of its path), `title` and `text`, \
                    the text each page shows, in UTF-8; `encoding`, the WHATWG name of the \
                    encoding it was read with; and `encoding_source`, where that encoding came \
                    from: `bom`, `header`, `meta`, `utf8` or `detected`.",
                account: None,
                time_steps: None,
            },
            expected: Expected::records(records(seed, true)),
        },
    ]
}

/// Every page's expected Record, with its encoding and source when
/// `provenance` asks for them.
fn records(seed: u64, provenance: bool) -> Vec<Record> {
    CASES
        .iter()
        .zip(1..)
        .map(|(case, number)| {
            let (title, text) = text(seed, number, case);
            let mut record = Record::from([
                ("id", case.id.to_owned()),
                ("title", title.to_owned()),
                ("text", text),
            ]);
            if provenance {
                record.insert("encoding", case.written.name().to_owned());
                record.insert("encoding_source", case.source.name().to_owned());
            }
            record
        })
        .collect()
}
