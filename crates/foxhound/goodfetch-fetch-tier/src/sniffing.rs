//! The type of a response that carries no `Content-Type`, as Chromium's
//! own MIME sniffer types a navigation's (`net/base/mime_sniffer.cc`'s
//! `SniffMimeType` with no type hint, read at Chromium 156.0.8078.4), over
//! at most its first 1,024 bytes, `kMaxBytesToSniff` (ADR 0119): Chromium's
//! sniffer, not the WHATWG MIME Sniffing standard's 1,445-byte header, is
//! the reference, so every tier types such a response as the browser does.
//! Pure: no I/O and no task.

/// How many bytes Chromium sniffs at most, `kMaxBytesToSniff`.
pub(crate) const MAX_BYTES_TO_SNIFF: usize = 1024;

/// How many bytes the HTML step reads.
const HTML_BYTES: usize = 512;

/// How many bytes the magic numbers read, `kBytesRequiredForMagic`.
const MAGIC_BYTES: usize = 42;

/// How many bytes the Office magic numbers read.
const OFFICE_BYTES: usize = 8;

/// A pattern a body's start is matched against.
enum Magic {
    /// Bytes, `.` matching any byte.
    Bytes(&'static [u8]),
    /// An ASCII string in any case.
    Text(&'static [u8]),
}

impl Magic {
    fn matches(&self, content: &[u8]) -> bool {
        match self {
            Self::Bytes(magic) => {
                content.len() >= magic.len()
                    && magic
                        .iter()
                        .zip(content)
                        .all(|(magic, byte)| *magic == b'.' || magic == byte)
            }
            Self::Text(magic) => {
                content.len() >= magic.len() && content[..magic.len()].eq_ignore_ascii_case(magic)
            }
        }
    }
}

/// The tags Chromium sniffs as HTML, `kSniffableTags`, in its order.
const SNIFFABLE_TAGS: [(&str, Magic); 18] = [
    ("text/xml", Magic::Bytes(b"<?xml")),
    ("text/html", Magic::Text(b"<!DOCTYPE html")),
    ("text/html", Magic::Text(b"<script")),
    ("text/html", Magic::Text(b"<html")),
    ("text/html", Magic::Text(b"<!--")),
    ("text/html", Magic::Text(b"<head")),
    ("text/html", Magic::Text(b"<iframe")),
    ("text/html", Magic::Text(b"<h1")),
    ("text/html", Magic::Text(b"<div")),
    ("text/html", Magic::Text(b"<font")),
    ("text/html", Magic::Text(b"<table")),
    ("text/html", Magic::Text(b"<a")),
    ("text/html", Magic::Text(b"<style")),
    ("text/html", Magic::Text(b"<title")),
    ("text/html", Magic::Text(b"<b")),
    ("text/html", Magic::Text(b"<body")),
    ("text/html", Magic::Text(b"<br")),
    ("text/html", Magic::Text(b"<p")),
];

/// Chromium's magic numbers, `kMagicNumbers`, in its order.
const MAGIC_NUMBERS: [(&str, Magic); 24] = [
    ("application/pdf", Magic::Bytes(b"%PDF-")),
    ("application/postscript", Magic::Bytes(b"%!PS-Adobe-")),
    ("image/gif", Magic::Bytes(b"GIF87a")),
    ("image/gif", Magic::Bytes(b"GIF89a")),
    ("image/png", Magic::Bytes(b"\x89PNG\x0D\x0A\x1A\x0A")),
    ("image/jpeg", Magic::Bytes(b"\xFF\xD8\xFF")),
    ("image/bmp", Magic::Bytes(b"BM")),
    ("text/plain", Magic::Bytes(b"#!")),
    ("text/plain", Magic::Bytes(b"%!")),
    ("text/plain", Magic::Bytes(b"From")),
    ("text/plain", Magic::Bytes(b">From")),
    ("application/x-gzip", Magic::Bytes(b"\x1F\x8B\x08")),
    ("audio/x-pn-realaudio", Magic::Bytes(b"\x2E\x52\x4D\x46")),
    (
        "video/x-ms-asf",
        Magic::Bytes(b"\x30\x26\xB2\x75\x8E\x66\xCF\x11\xA6\xD9\x00\xAA\x00\x62\xCE\x6C"),
    ),
    ("image/tiff", Magic::Bytes(b"I I")),
    ("image/tiff", Magic::Bytes(b"II*")),
    ("image/tiff", Magic::Bytes(b"MM\x00*")),
    ("audio/mpeg", Magic::Bytes(b"ID3")),
    ("image/webp", Magic::Bytes(b"RIFF....WEBPVP")),
    ("video/webm", Magic::Bytes(b"\x1A\x45\xDF\xA3")),
    ("application/zip", Magic::Bytes(b"PK\x03\x04")),
    (
        "application/x-rar-compressed",
        Magic::Bytes(b"Rar!\x1A\x07\x00"),
    ),
    (
        "application/x-msmetafile",
        Magic::Bytes(b"\xD7\xCD\xC6\x9A"),
    ),
    ("application/octet-stream", Magic::Bytes(b"MZ")),
];

/// Chromium's Office magic numbers, `kOfficeMagicNumbers`.
const CFB: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const OOXML: &[u8] = b"PK\x03\x04";

/// The type Chromium gives `body`, a response with no `Content-Type` at a
/// URL whose path is `path`: the path names an Office document's or a
/// Chrome extension's type, as in Chromium.
pub fn sniff(body: &[u8], path: &str) -> &'static str {
    let content = &body[..body.len().min(MAX_BYTES_TO_SNIFF)];
    if let Some(html) = sniff_html(content) {
        return html;
    }
    let binary = !has_bom(content) && looks_like_binary(content);
    let fallback = if binary {
        "application/octet-stream"
    } else {
        "text/plain"
    };
    if let Some(crx) = sniff_crx(content, path) {
        return crx;
    }
    if let Some(office) = sniff_office(content, path) {
        return office;
    }
    let magic = &content[..content.len().min(MAGIC_BYTES)];
    MAGIC_NUMBERS
        .iter()
        .find(|(_, number)| number.matches(magic))
        .map_or(fallback, |(mime_type, _)| mime_type)
}

/// HTML or XML by its first tag after leading ASCII spaces in its first
/// 512 bytes (`SniffForHTML`).
fn sniff_html(content: &[u8]) -> Option<&'static str> {
    let content = &content[..content.len().min(HTML_BYTES)];
    let start = content
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r'))
        .unwrap_or(content.len());
    let trimmed = &content[start..];
    SNIFFABLE_TAGS
        .iter()
        .find(|(_, tag)| tag.matches(trimmed))
        .map(|(mime_type, _)| *mime_type)
}

/// Whether `content` opens with a UTF-16 or UTF-8 byte order mark.
fn has_bom(content: &[u8]) -> bool {
    content.starts_with(b"\xFE\xFF")
        || content.starts_with(b"\xFF\xFE")
        || content.starts_with(b"\xEF\xBB\xBF")
}

/// Whether a control byte other than tab, line feed, carriage return,
/// form feed and escape appears (`LooksLikeBinary`).
fn looks_like_binary(content: &[u8]) -> bool {
    content
        .iter()
        .any(|byte| *byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r' | b'\x0C' | b'\x1B'))
}

/// A Chrome extension at a path ending `.crx` (`SniffCRX`).
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Chromium compares the extension case-sensitively, and the sniffer is its copy"
)]
fn sniff_crx(content: &[u8], path: &str) -> Option<&'static str> {
    let magic = &content[..content.len().min(MAGIC_BYTES)];
    (path.ends_with(".crx")
        && (magic.starts_with(b"Cr24\x02\x00\x00\x00")
            || magic.starts_with(b"Cr24\x03\x00\x00\x00")))
    .then_some("application/x-chrome-extension")
}

/// An Office document by its magic number and its path's extension
/// (`SniffForOfficeDocs`).
fn sniff_office(content: &[u8], path: &str) -> Option<&'static str> {
    let magic = &content[..content.len().min(OFFICE_BYTES)];
    let cfb = magic.starts_with(CFB);
    if !cfb && !magic.starts_with(OOXML) {
        return None;
    }
    let ends = |extension: &str| {
        path.len() >= extension.len()
            && path[path.len() - extension.len()..].eq_ignore_ascii_case(extension)
    };
    let kind = [
        (".doc", 0),
        (".xls", 1),
        (".ppt", 2),
        (".docx", 0),
        (".xlsx", 1),
        (".pptx", 2),
    ]
    .into_iter()
    .find(|(extension, _)| ends(extension))
    .map(|(_, kind)| kind)?;
    Some(match (cfb, kind) {
        (true, 0) => "application/msword",
        (true, 1) => "application/vnd.ms-excel",
        (true, _) => "application/vnd.ms-powerpoint",
        (false, 0) => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        (false, 1) => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        (false, _) => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    })
}
