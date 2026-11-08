//! The `files` family (ADRs 0025 and 0119): a PDF, a ZIP, a JSON response,
//! a 404 that is a CSV, responses with no `Content-Type`, and a download
//! link, each judged from the bytes the Proving ground served.
//! Covers: story:2-33

use std::fmt::Write as _;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::proving_ground::{ProvingGround, RawAnswer, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 165;

fn hex(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing to a String succeeds");
            hex
        })
}

/// The MIME type a navigation computes: the `Content-Type`'s essence, or,
/// with none, the WHATWG MIME Sniffing standard's HTML and PDF signatures.
fn navigation_type(answer: &RawAnswer) -> String {
    match &answer.content_type {
        Some(declared) => essence(declared),
        None if answer.body.starts_with(b"%PDF-") => "application/pdf".to_owned(),
        None if answer
            .body
            .to_ascii_uppercase()
            .starts_with(b"<!DOCTYPE HTML") =>
        {
            "text/html".to_owned()
        }
        None => "application/octet-stream".to_owned(),
    }
}

/// A media type without its parameters.
fn essence(declared: &str) -> String {
    declared
        .split(';')
        .next()
        .expect("a media type")
        .trim()
        .to_owned()
}

/// One Record per response the index links, its type computed by `typed`.
fn crawl(proving_ground: &ProvingGround, typed: impl Fn(&RawAnswer) -> String) -> Vec<Value> {
    let task = task(proving_ground, "files/1");
    let index = proving_ground.get(start_url(&task)).body;
    every(r#"<a href="(/files/[^"]+)""#, &index)
        .iter()
        .map(|path| {
            let answer = proving_ground.get_raw(&format!("{}{path}", origin(start_url(&task))));
            json!({
                "id": path,
                "status": answer.status,
                "content_type": typed(&answer),
                "sha256": hex(&answer.body),
            })
        })
        .collect()
}

fn file(proving_ground: &ProvingGround, path: &str) -> RawAnswer {
    proving_ground.get_raw(&format!(
        "http://files.test:{}{path}",
        proving_ground.port()
    ))
}

#[test]
fn every_response_typed_as_a_navigation_types_it_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground, navigation_type);

    let answer = proving_ground.judge(&submission("files/1", "success", &records));

    assert_eq!(records.len(), 7);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_the_zip_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = crawl(&proving_ground, navigation_type);
    records.retain(|record| record["id"] != "/files/archive.zip");

    let answer = proving_ground.judge(&submission("files/1", "failure", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!(["/files/archive.zip"]));
}

#[test]
fn a_success_that_never_sniffs_is_a_false_success_naming_the_untyped_responses() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground, |answer| {
        answer
            .content_type
            .as_deref()
            .map_or_else(|| "application/octet-stream".to_owned(), essence)
    });

    let answer = proving_ground.judge(&submission("files/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let wrong: Vec<Value> = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .map(|wrong| json!([wrong["id"], wrong["field"]]))
        .collect();
    assert_eq!(
        wrong,
        [
            json!(["/files/untyped-page", "content_type"]),
            json!(["/files/untyped-document", "content_type"]),
        ]
    );
}

#[test]
fn the_untyped_responses_have_no_content_type() {
    let proving_ground = ProvingGround::start(SEED);

    let types = (
        file(&proving_ground, "/files/untyped-page").content_type,
        file(&proving_ground, "/files/untyped-document").content_type,
    );

    assert_eq!(types, (None, None));
}

#[test]
fn the_missing_csv_is_a_404_typed_as_csv() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = file(&proving_ground, "/files/missing.csv");

    assert_eq!(
        (answer.status, answer.content_type),
        (404, Some("text/csv".to_owned()))
    );
}

/// The PDF's document title and the text its page shows.
fn read_pdf(body: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(body);
    (
        first(r"/Title \(([^)]*)\)", &text),
        every(r"\(([^)]*)\) Tj", &text).join(" "),
    )
}

#[test]
fn the_pdfs_title_and_text_are_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "files/2");
    let (title, text) = read_pdf(&proving_ground.get_raw(start_url(&task)).body);

    let answer = proving_ground.judge(&submission(
        "files/2",
        "success",
        &[json!({ "id": "report", "title": title, "text": text })],
    ));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_success_that_reads_the_pdfs_bytes_as_text_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "files/2");
    let bytes = proving_ground.get_raw(start_url(&task)).body;
    let (title, _) = read_pdf(&bytes);
    let body = String::from_utf8_lossy(&bytes);

    let answer = proving_ground.judge(&submission(
        "files/2",
        "success",
        &[json!({ "id": "report", "title": title, "text": body })],
    ));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "text", "{answer}");
}

/// Every object the PDF's cross-reference table names starts where it says.
#[test]
fn the_pdfs_cross_reference_table_points_at_its_objects() {
    let proving_ground = ProvingGround::start(SEED);

    let body = file(&proving_ground, "/files/report.pdf").body;

    let text = String::from_utf8_lossy(&body).into_owned();
    let start: usize = first(r"startxref\n([0-9]+)\n%%EOF\n$", &text)
        .parse()
        .expect("an offset");
    assert!(body[start..].starts_with(b"xref\n"));
    let offsets = every(r"([0-9]{10}) 00000 n ", &text);
    assert_eq!(offsets.len(), 6);
    for (number, offset) in (1..).zip(&offsets) {
        let offset: usize = offset.parse().expect("an offset");
        assert!(
            body[offset..].starts_with(format!("{number} 0 obj\n").as_bytes()),
            "object {number}"
        );
    }
}

#[test]
fn the_zips_end_record_points_at_its_central_directory() {
    let proving_ground = ProvingGround::start(SEED);

    let body = file(&proving_ground, "/files/archive.zip").body;

    assert!(body.starts_with(b"PK\x03\x04"));
    let end = body.len() - 22;
    assert_eq!(body[end..end + 4], *b"PK\x05\x06");
    let directory = u32::from_le_bytes(body[end + 16..end + 20].try_into().expect("four bytes"));
    let directory = usize::try_from(directory).expect("an offset");
    assert_eq!(body[directory..directory + 4], *b"PK\x01\x02");
}

/// The download link's suggested name and the file it offers.
fn download(proving_ground: &ProvingGround, name_from_url: bool) -> Value {
    let task = task(proving_ground, "files/3");
    let page = proving_ground.get(start_url(&task)).body;
    let href = first(r#"<a download="[^"]+" href="([^"]+)""#, &page);
    let suggested = first(r#"<a download="([^"]+)""#, &page);
    let body = proving_ground
        .get_raw(&format!("{}{href}", origin(start_url(&task))))
        .body;
    let file_name = if name_from_url {
        href.rsplit('/').next().expect("a path").to_owned()
    } else {
        suggested
    };
    json!({ "id": "export", "file_name": file_name, "sha256": hex(&body) })
}

#[test]
fn the_file_the_download_link_offers_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let record = download(&proving_ground, false);

    let answer = proving_ground.judge(&submission("files/3", "success", &[record]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_success_that_names_the_file_after_its_url_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let record = download(&proving_ground, true);

    let answer = proving_ground.judge(&submission("files/3", "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "file_name", "{answer}");
}
