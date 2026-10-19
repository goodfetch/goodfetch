//! The `files` family on `files.test`: ADR 0025's PDFs and downloads, with
//! the responses ADR 0119 judges a crawl by. An index links a PDF, a ZIP, a
//! JSON response, a 404 that is a CSV, a page and a PDF served with no
//! `Content-Type` (which MIME sniffing types), and a page whose
//! `<a download>` link offers a CSV export. Every file is generated from the
//! seed on request, never stored.

use std::fmt::Write as _;

use hyper::StatusCode;
use sha2::{Digest as _, Sha256};

use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "files.test";

const FAMILY: &str = "files";
const FAMILY_PLACE: u64 = 24;

const WORDS: [&str; 16] = [
    "annual",
    "budget",
    "cedar",
    "delta",
    "orchard",
    "field",
    "granite",
    "harvest",
    "inland",
    "juniper",
    "keystone",
    "lighthouse",
    "meridian",
    "northern",
    "orbit",
    "prairie",
];

/// `count` words drawn at `place`, each capitalized when `title` asks.
fn words(seed: u64, place: u64, count: usize, title: bool) -> String {
    generator::distinct(seed, &[FAMILY_PLACE, place], WORDS.len(), count)
        .into_iter()
        .map(|index| {
            let word = WORDS[index];
            if title {
                let (first, rest) = word.split_at(1);
                format!("{}{rest}", first.to_uppercase())
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A response of the family: its status, `Content-Type` (empty for none)
/// and body.
pub(crate) struct File {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    pub(crate) body: Vec<u8>,
}

impl File {
    fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: StatusCode::OK,
            content_type,
            body,
        }
    }
}

/// Each path the index links, in its order.
const LINKED: [&str; 7] = [
    "/files/report.pdf",
    "/files/archive.zip",
    "/files/data.json",
    "/files/missing.csv",
    "/files/untyped-page",
    "/files/untyped-document",
    "/files/download",
];

/// The file at `path`; `None` when the family has no such file.
pub(crate) fn page(seed: u64, path: &str) -> Option<File> {
    Some(match path {
        "/files" => File::ok("text/html; charset=utf-8", index().into_bytes()),
        "/files/report.pdf" => File::ok("application/pdf", report(seed)),
        "/files/archive.zip" => File::ok("application/zip", archive(seed)),
        "/files/data.json" => File::ok("application/json", data(seed).into_bytes()),
        "/files/missing.csv" => File {
            status: StatusCode::NOT_FOUND,
            content_type: "text/csv",
            body: b"error\nnot found\n".to_vec(),
        },
        "/files/untyped-page" => File::ok("", untyped_page(seed).into_bytes()),
        "/files/untyped-document" => File::ok("", pdf(&title(seed, 3), &words(seed, 4, 6, false))),
        "/files/download" => File::ok("text/html; charset=utf-8", download_page(seed).into_bytes()),
        "/files/export.csv" => File::ok("text/csv", export(seed).into_bytes()),
        _ => return None,
    })
}

fn index() -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Files</title></head>\n<body>\n<main>\n<h1>Files</h1>\n<ul>\n",
    );
    for path in LINKED {
        writeln!(html, "<li><a href=\"{path}\">{path}</a></li>")
            .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n</main>\n</body>\n</html>\n");
    html
}

fn title(seed: u64, place: u64) -> String {
    format!("Report on the {}", words(seed, place, 2, true))
}

/// The report's title and text, which `/files/report.pdf` holds.
fn report_text(seed: u64) -> (String, String) {
    (
        title(seed, 1),
        format!(
            "The {} survey found nothing amiss.",
            words(seed, 2, 3, false)
        ),
    )
}

fn report(seed: u64) -> Vec<u8> {
    let (title, text) = report_text(seed);
    pdf(&title, &text)
}

/// A one-page PDF 1.4 whose Info dictionary holds `title` and whose page
/// shows `text` in Helvetica, uncompressed. Both are plain ASCII words, so
/// neither needs escaping.
fn pdf(title: &str, text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        format!("<< /Title ({title}) /Producer (Goodfetch Proving ground) >>"),
    ];
    let mut bytes = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (number, object) in (1..).zip(&objects) {
        offsets.push(bytes.len());
        bytes.extend(format!("{number} 0 obj\n{object}\nendobj\n").into_bytes());
    }
    let xref = bytes.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        writeln!(table, "{offset:010} 00000 n ").expect("invariant: writing to a String succeeds");
    }
    write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .expect("invariant: writing to a String succeeds");
    bytes.extend(table.into_bytes());
    bytes
}

/// A ZIP archive holding one stored file, `readme.txt`.
fn archive(seed: u64) -> Vec<u8> {
    let name = b"readme.txt";
    let content = format!("{}\n", words(seed, 5, 8, false)).into_bytes();
    let crc = crc32fast::hash(&content);
    let size = u32::try_from(content.len()).expect("invariant: the readme is small");
    let name_length = u16::try_from(name.len()).expect("invariant: the name is short");
    // Version 2.0, no flags, stored, at 1980-01-01 00:00.
    let common = |bytes: &mut Vec<u8>| {
        for value in [20_u16, 0, 0, 0, 0x21] {
            bytes.extend(value.to_le_bytes());
        }
        for value in [crc, size, size] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(name_length.to_le_bytes());
    };
    let mut bytes = 0x0403_4b50_u32.to_le_bytes().to_vec();
    common(&mut bytes);
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(name);
    bytes.extend(&content);
    let directory = u32::try_from(bytes.len()).expect("invariant: the archive is small");
    bytes.extend(0x0201_4b50_u32.to_le_bytes());
    bytes.extend(20_u16.to_le_bytes());
    common(&mut bytes);
    // No extra field or comment, disk 0, no attributes, at offset 0.
    for value in [0_u16, 0, 0, 0] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(name);
    let directory_size =
        u32::try_from(bytes.len()).expect("invariant: the archive is small") - directory;
    bytes.extend(0x0605_4b50_u32.to_le_bytes());
    for value in [0_u16, 0, 1, 1] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(directory_size.to_le_bytes());
    bytes.extend(directory.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes
}

fn data(seed: u64) -> String {
    serde_json::json!({ "title": title(seed, 6), "words": words(seed, 7, 4, false) }).to_string()
}

fn untyped_page(seed: u64) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main><h1>{title}</h1><p>{text}</p></main>\n</body>\n</html>\n",
        title = title(seed, 8),
        text = words(seed, 20, 6, false)
    )
}

/// The name the download link suggests for the export.
fn export_name(seed: u64) -> String {
    format!("{}-export.csv", words(seed, 9, 3, false).replace(' ', "-"))
}

fn download_page(seed: u64) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Export</title></head>\n<body>\n<main>\n<h1>Export</h1>\n<a download=\"{name}\" href=\"/files/export.csv\">Download the export</a>\n</main>\n</body>\n</html>\n",
        name = export_name(seed)
    )
}

fn export(seed: u64) -> String {
    let mut csv = String::from("id,word\n");
    for row in 1..=5 {
        writeln!(csv, "{row},{}", words(seed, 10 + row, 1, false))
            .expect("invariant: writing to a String succeeds");
    }
    csv
}

/// The lowercase hex SHA-256 of a body.
fn sha256(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("invariant: writing to a String succeeds");
            hex
        })
}

/// The MIME type a navigation gives each linked response: its
/// `Content-Type`'s essence, or what MIME sniffing makes of a response with
/// none (an HTML document's signature, a PDF's `%PDF-`).
const SNIFFED: [&str; 7] = [
    "application/pdf",
    "application/zip",
    "application/json",
    "text/csv",
    "text/html",
    "application/pdf",
    "text/html",
];

/// The host name of the family's task `ordinal`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, on their own host names and `port`.
/// Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal: u32, kind, path: &str, description, records| Task {
        host: task_host(ordinal),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind,
            start_url: format!("{}{path}", origin(&task_host(ordinal), port)),
            description,
            account: None,
            time_steps: None,
        },
        expected: Expected::records(records),
    };
    let responses = LINKED
        .iter()
        .zip(SNIFFED)
        .map(|(path, content_type)| {
            let file = page(seed, path).expect("invariant: the index links the family's files");
            Record::from([
                ("id", (*path).to_owned()),
                ("status", file.status.as_u16().to_string()),
                ("content_type", content_type.to_owned()),
                ("sha256", sha256(&file.body)),
            ])
        })
        .collect();
    let (title, text) = report_text(seed);
    vec![
        task(
            1,
            Kind::Crawl,
            "/files",
            "Request each link of the index once and return one Record per response with the \
                fields `id` (the link's path), `status`, `content_type` (the MIME type's essence \
                as a browser's navigation computes it, sniffing a response with no \
                `Content-Type`) and `sha256` (the lowercase hex SHA-256 of its body).",
            responses,
        ),
        task(
            2,
            Kind::Extract,
            "/files/report.pdf",
            "Extract this PDF as one Record with the fields `id` (`report`), `title` (its \
                document title) and `text` (the text its page shows).",
            vec![Record::from([
                ("id", "report".to_owned()),
                ("title", title),
                ("text", text),
            ])],
        ),
        task(
            3,
            Kind::Extract,
            "/files/download",
            "Download the file this page's download link offers and return one Record with the \
                fields `id` (`export`), `file_name` (the name the link suggests) and `sha256` \
                (the lowercase hex SHA-256 of the file).",
            vec![Record::from([
                ("id", "export".to_owned()),
                ("file_name", export_name(seed)),
                ("sha256", sha256(export(seed).as_bytes())),
            ])],
        ),
    ]
}
