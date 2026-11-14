//! A document read as its decoded bytes arrive gives what today's order
//! gives on the whole body, however the bytes are cut (spec #2164, issue
//! 2187).

use bytes::Bytes;
use encoding_rs::{Encoding, UTF_8, WINDOWS_1251};
use proptest::prelude::*;
use rstest::rstest;

use crate::charset::{Kind, Read, Source, Stream, Streamed, Text, decode, read};

/// A label of every WHATWG encoding, then labels no encoding has.
const LABELS: &[&str] = &[
    "UTF-8",
    "IBM866",
    "ISO-8859-2",
    "ISO-8859-3",
    "ISO-8859-4",
    "ISO-8859-5",
    "ISO-8859-6",
    "ISO-8859-7",
    "ISO-8859-8",
    "ISO-8859-8-I",
    "ISO-8859-10",
    "ISO-8859-13",
    "ISO-8859-14",
    "ISO-8859-15",
    "ISO-8859-16",
    "KOI8-R",
    "KOI8-U",
    "macintosh",
    "windows-874",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "x-mac-cyrillic",
    "GBK",
    "gb18030",
    "Big5",
    "EUC-JP",
    "ISO-2022-JP",
    "Shift_JIS",
    "EUC-KR",
    "replacement",
    "UTF-16BE",
    "UTF-16LE",
    "x-user-defined",
    "latin1",
    "no-such-encoding",
];

/// No BOM, then each BOM.
const BOMS: &[&[u8]] = &[b"", b"\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF"];

/// Characters of several scripts, which each encoding writes as far as it
/// can.
const SAMPLE: &str = "<p>日本 Привет café 한국 Ελλάδα שלום ไทย</p>";

/// Where a document declares its encoding in its own bytes.
#[derive(Clone, Copy, Debug)]
enum Declared {
    Nowhere,
    MetaCharset,
    MetaPragma,
    XmlDeclaration,
}

/// What comes before a `<meta>`, which pushes it past the bytes the
/// prescan reads at first.
#[derive(Clone, Copy, Debug)]
enum Before {
    Nothing,
    Script(usize),
    HeadTags(usize),
    BodyText(usize),
}

impl Before {
    fn markup(self) -> String {
        match self {
            Self::Nothing => String::new(),
            Self::Script(length) => format!("<script>{}</script>", "x".repeat(length)),
            Self::HeadTags(count) => "<link rel=x>".repeat(count),
            Self::BodyText(length) => format!("<p>{}</p>", "y".repeat(length)),
        }
    }
}

/// A document's parts, as the property generates them.
#[derive(Clone, Debug)]
struct Document {
    bom: &'static [u8],
    declared: Declared,
    label: &'static str,
    before: Before,
    written_in: &'static str,
    tail: Vec<u8>,
}

impl Document {
    fn bytes(&self) -> Vec<u8> {
        let declaration = match self.declared {
            Declared::Nowhere => String::new(),
            Declared::MetaCharset => format!("<meta charset={}>", self.label),
            Declared::MetaPragma => format!(
                "<meta http-equiv=\"Content-Type\" content=\"text/html; charset={}\">",
                self.label
            ),
            Declared::XmlDeclaration => {
                format!("<?xml version=\"1.0\" encoding=\"{}\"?>", self.label)
            }
        };
        let markup = match self.declared {
            Declared::XmlDeclaration => format!("{declaration}{}", self.before.markup()),
            Declared::Nowhere | Declared::MetaCharset | Declared::MetaPragma => {
                format!("<!doctype html><head>{}{declaration}", self.before.markup())
            }
        };
        let encoding = Encoding::for_label(self.written_in.as_bytes()).unwrap_or(UTF_8);
        let (sample, _, _) = encoding.encode(SAMPLE);
        [self.bom, markup.as_bytes(), &sample, &self.tail].concat()
    }
}

fn label() -> impl Strategy<Value = &'static str> {
    prop::sample::select(LABELS)
}

fn document() -> impl Strategy<Value = Document> {
    (
        prop::sample::select(BOMS),
        prop::sample::select(
            &[
                Declared::Nowhere,
                Declared::MetaCharset,
                Declared::MetaPragma,
                Declared::XmlDeclaration,
            ][..],
        ),
        label(),
        prop_oneof![
            Just(Before::Nothing),
            (0_usize..9000).prop_map(Before::Script),
            (0_usize..400).prop_map(Before::HeadTags),
            (0_usize..3000).prop_map(Before::BodyText),
        ],
        label(),
        prop::collection::vec(any::<u8>(), 0..64),
    )
        .prop_map(
            |(bom, declared, label, before, written_in, tail)| Document {
                bom,
                declared,
                label,
                before,
                written_in,
                tail,
            },
        )
}

fn kind() -> impl Strategy<Value = Kind> {
    prop::sample::select(&[Kind::Html, Kind::Xml, Kind::Other][..])
}

/// `body` read through a [`Stream`] as its bytes arrive in pieces of
/// `cuts`' sizes, taken in turn, and the characters it handed out.
fn handing(
    body: &Bytes,
    (kind, charset, forced): (Kind, Option<&str>, Option<&'static Encoding>),
    cuts: &[usize],
) -> (Option<Streamed>, Vec<u8>) {
    let mut stream = Stream::new(kind, charset, forced);
    let mut handed = Vec::new();
    let mut to = |text: &[u8]| handed.extend_from_slice(text);
    let mut arrived = 0;
    for cut in cuts.iter().cycle() {
        if arrived == body.len() {
            break;
        }
        arrived = (arrived + cut).min(body.len());
        stream.advance(&body[..arrived], &mut to);
    }
    let streamed = stream.finish(body, &mut to);
    (streamed, handed)
}

/// [`handing`]'s stream alone.
fn streamed(
    body: &Bytes,
    read: (Kind, Option<&str>, Option<&'static Encoding>),
    cuts: &[usize],
) -> Option<Streamed> {
    handing(body, read, cuts).0
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..goodfetch_test_support::proptest_config()
    })]

    /// However its bytes are cut, a document read as they arrive has the
    /// characters and the encoding with its source that today's order,
    /// [`read`] then [`decode`], finds on the whole body, for every WHATWG
    /// encoding, invalid bytes included: a body only a guess reads stays
    /// one, which escalates.
    ///
    /// Covers: story:5-129, setting:fetch.encoding
    #[test]
    fn a_document_read_as_it_arrives_reads_as_the_whole_body_does(
        document in document(),
        kind in kind(),
        charset in prop::option::of(label()),
        forced in prop::option::of(label()),
        cuts in prop::collection::vec(1_usize..5000, 1..8),
    ) {
        let body = Bytes::from(document.bytes());
        let forced = forced.and_then(|label| Encoding::for_label(label.as_bytes()));
        let whole = read(&body, kind, charset, forced);
        let expected = Text {
            read: whole,
            html: match whole {
                Some(found) => decode(&body, found),
                None => String::from_utf8_lossy(&body).into_owned(),
            }
            .into(),
        };

        let (streamed, handed) = handing(&body, (kind, charset, forced), &cuts);

        prop_assert_eq!(Text::of(&body, kind, charset, forced), expected.clone());
        let streamed = streamed.expect("a body of 64 KiB at most is read as it arrives");
        prop_assert!(streamed.read_under(forced));
        let (text, as_handed) = streamed.text(&body);
        prop_assert_eq!(&text, &expected);
        // What it handed out as the bytes arrived is its text, unless a
        // byte did not decode in place, and it says which.
        prop_assert_eq!(as_handed, handed == text.html.as_bytes()[..]);
        prop_assert!(as_handed || String::from_utf8(handed).is_err());
    }
}

/// `filler` repeated to `length` bytes and more.
fn filled(filler: &str, length: usize) -> String {
    filler.repeat(length / filler.len() + 1)
}

/// Привет in windows-1251.
const PRIVET: &[u8] = &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];

/// A `<meta>` after an 8 KB inline script is read as one after more than
/// 1,024 bytes of head tags is, however the bytes arrive; an undeclared
/// body that is no UTF-8 is only a guess.
///
/// Covers: story:5-129
#[rstest]
#[case::after_an_8_kb_inline_script(
    format!("<!doctype html><head><script>{}</script><meta charset=windows-1251>", filled("var x = 1;", 8 * 1024)),
    Some(Read { encoding: WINDOWS_1251, source: Source::Meta }),
)]
#[case::undeclared_bytes_that_are_no_utf_8(
    "<!doctype html><p>".to_owned(),
    None,
)]
fn a_document_read_as_it_arrives_finds_its_encoding(
    #[values(1, 1000, 64 * 1024)] cut: usize,
    #[case] markup: String,
    #[case] expected: Option<Read>,
) {
    let body = Bytes::from([markup.as_bytes(), PRIVET].concat());

    let text =
        streamed(&body, (Kind::Html, None, None), &[cut]).map(|streamed| streamed.text(&body).0);

    assert_eq!(text.map(|text| text.read), Some(expected));
}

/// A document longer than the bytes read as they arrive, its prescan
/// never ended, is read whole once it ended.
#[test]
fn a_long_document_whose_prescan_never_ended_is_read_whole() {
    let body = Bytes::from(format!(
        "<!doctype html><head>{}",
        filled("<link rel=x>", 70 * 1024)
    ));

    let streamed = streamed(&body, (Kind::Html, None, None), &[4096]);

    assert_eq!(streamed, None);
}
