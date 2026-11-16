#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use rstest::rstest;
use rusqlite::Connection;

use super::write_export;
use crate::encoding::ExportOptions;

/// A run file in memory holding `pages`' Records 1 through `count`, each
/// its number and a title with accents.
fn run_file(count: u32) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in crate::TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    for number in 1..=count {
        connection
            .execute(
                "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', ?2, NULL, '{}')",
                rusqlite::params![
                    number,
                    format!("{{\"n\":{number},\"title\":\"Le Café, tome {number}\"}}")
                ],
            )
            .unwrap();
    }
    connection
}

/// An Export of more Records than a page holds, compressed page after
/// page, gives back the bytes of the same Export uncompressed.
///
/// Covers: story:6-276
#[rstest]
#[case::gzip(ExportCompression::Gzip)]
#[case::zstd(ExportCompression::Zstd)]
fn a_compressed_export_of_several_pages_gives_back_its_uncompressed_bytes(
    #[case] compression: ExportCompression,
) {
    let connection = run_file(600);
    let export = |compression| {
        write_export(
            &connection,
            "pages",
            ExportFormat::Json,
            ExportOptions::of("pages"),
            compression,
        )
        .unwrap()
    };

    let bytes = export(compression);

    let mut plain = Vec::new();
    if compression == ExportCompression::Gzip {
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut plain)
            .unwrap();
    } else {
        plain = zstd::decode_all(&bytes[..]).unwrap();
    }
    assert_eq!(plain, export(ExportCompression::None));
}
