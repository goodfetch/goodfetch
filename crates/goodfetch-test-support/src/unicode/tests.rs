#![expect(
    clippy::disallowed_methods,
    reason = "the round trip writes and reads plain files in a test, outside any async worker (ADR 0062)"
)]

use std::fs;

use super::{UnicodeField, unicode_record};
use crate::env::TestEnv;

fn field(name: &str, text: &str) -> UnicodeField {
    UnicodeField {
        name: name.to_owned(),
        text: text.to_owned(),
    }
}

/// Writes each field to its own file in `env`'s folder through `write`, and
/// reads the files back as they are on disk.
fn round_trip(env: &TestEnv, write: impl Fn(&str) -> Vec<u8>) -> Vec<UnicodeField> {
    unicode_record()
        .fields
        .iter()
        .map(|field| {
            let path = env.folder().join(format!("{}.txt", field.name));
            fs::write(&path, write(&field.text)).unwrap();
            UnicodeField {
                name: field.name.clone(),
                text: String::from_utf8(fs::read(&path).unwrap()).unwrap(),
            }
        })
        .collect()
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

#[test]
fn the_unicode_record_round_trips_byte_for_byte_through_a_file() {
    let env = TestEnv::new();

    let read = round_trip(&env, |text| text.as_bytes().to_vec());

    assert_eq!(read, unicode_record().fields);
}

#[test]
fn a_write_that_translates_newlines_fails_the_unicode_round_trip() {
    let env = TestEnv::new();

    let read = round_trip(&env, |text| text.replace('\n', "\r\n").into_bytes());

    assert_ne!(read, unicode_record().fields);
}
