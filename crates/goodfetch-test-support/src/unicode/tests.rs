use super::{UnicodeField, unicode_record};

fn field(name: &str, text: &str) -> UnicodeField {
    UnicodeField {
        name: name.to_owned(),
        text: text.to_owned(),
    }
}

#[test]
fn the_unicode_record_holds_adr_0073s_hard_text() {
    let record = unicode_record();

    assert_eq!(
        record.fields,
        [
            field(
                "bidi",
                "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{200f}world\u{200e} \u{645}\u{631}\u{62d}\u{628}\u{627} \u{2067}\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}\u{2069} end",
            ),
            field("nfc", "caf\u{e9}"),
            field("nfd", "cafe\u{301}"),
            field("zwj_emoji", "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}"),
            field("astral", "\u{1d11e}"),
            field("zero_width_joiner", "a\u{200d}b"),
            field("bom", "\u{feff}leads with a byte order mark"),
            field("tab_and_newline", "one\ttwo\nthree"),
        ]
    );
}
