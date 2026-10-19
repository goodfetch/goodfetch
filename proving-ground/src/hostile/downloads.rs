//! The `downloads` family on `downloads.test`: a download whose page
//! suggests a name, through `Content-Disposition` (RFC 6266), that tries to
//! leave the folder it is saved to, with each OS's cases (ADR 0067): `..`,
//! absolute paths, Windows drives, `\\?\` and UNC paths, reserved names, a
//! trailing dot or space, an alternate data stream, a name percent-encoded in
//! `filename*` and one that `filename*` overrides, control characters, a
//! name decomposed as APFS may store it, and one too long for a file system.
//! A correct tool keeps each file's bytes under one safe name; a plain name
//! and a name in ISO-8859-1 are kept exactly.

use hyper::Method;
use hyper::StatusCode;

use super::word;
use crate::digest;
use crate::exchange::{Reply, Request};
use crate::task::{self, Entry, Expected, File, Kind, Naming, Task};

pub(super) const HOST: &str = "downloads.test";
pub(super) const FAMILY: &str = "downloads";

/// The family's number among the hostile families' generator places.
const FAMILY_NUMBER: u64 = 3;

/// The path of every task's download.
const PATH: &str = "/download";

/// One download: its `Content-Disposition`, the name a correct tool saves it
/// under (the suggested one, decoded) and how that name is judged.
struct Download {
    disposition: &'static str,
    name: &'static str,
    naming: Naming,
}

const DOWNLOADS: [Download; 17] = [
    Download {
        disposition: "attachment; filename=\"export.csv\"",
        name: "export.csv",
        naming: Naming::Exact,
    },
    Download {
        disposition: "attachment; filename=\"../../../../tmp/goodfetch-traversal.csv\"",
        name: "../../../../tmp/goodfetch-traversal.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"/tmp/goodfetch-absolute.csv\"",
        name: "/tmp/goodfetch-absolute.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"..\\\\..\\\\goodfetch-traversal.csv\"",
        name: "..\\..\\goodfetch-traversal.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"C:\\\\Windows\\\\goodfetch-drive.csv\"",
        name: "C:\\Windows\\goodfetch-drive.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"\\\\\\\\?\\\\C:\\\\goodfetch-long-path.csv\"",
        name: "\\\\?\\C:\\goodfetch-long-path.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"\\\\\\\\server\\\\share\\\\goodfetch-unc.csv\"",
        name: "\\\\server\\share\\goodfetch-unc.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"CON.csv\"",
        name: "CON.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"LPT1\"",
        name: "LPT1",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"report.csv.\"",
        name: "report.csv.",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"report.csv \"",
        name: "report.csv ",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"report.csv:hidden\"",
        name: "report.csv:hidden",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename*=UTF-8''..%2F..%2Fgoodfetch-encoded.csv",
        name: "../../goodfetch-encoded.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename=\"safe.csv\"; filename*=UTF-8''..%2Fgoodfetch-star.csv",
        name: "../goodfetch-star.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename*=UTF-8''report%07%0A.csv",
        name: "report\u{7}\n.csv",
        naming: Naming::Safe,
    },
    Download {
        disposition: "attachment; filename*=ISO-8859-1''r%E9sum%E9.csv",
        name: "r\u{e9}sum\u{e9}.csv",
        naming: Naming::Exact,
    },
    Download {
        disposition: "attachment; filename*=UTF-8''cafe%CC%81.csv",
        name: "cafe\u{301}.csv",
        naming: Naming::Safe,
    },
];

/// The last task's download suggests a name of this many bytes, more than
/// the 255 a file system's name holds.
const OVERLONG_BYTES: usize = 300;

pub(super) const TASKS: u32 = 18;

/// Task `ordinal`'s `Content-Disposition` and the name it suggests, decoded,
/// with how that name is judged.
fn download(ordinal: u32) -> (String, String, Naming) {
    match DOWNLOADS.get(usize::try_from(ordinal - 1).expect("invariant: a small ordinal")) {
        Some(download) => (
            download.disposition.to_owned(),
            download.name.to_owned(),
            download.naming,
        ),
        None => {
            let name = format!("{}.csv", "a".repeat(OVERLONG_BYTES - ".csv".len()));
            (
                format!("attachment; filename=\"{name}\""),
                name,
                Naming::Safe,
            )
        }
    }
}

/// The bytes of task `ordinal`'s download: a CSV of one seeded row.
fn bytes(seed: u64, ordinal: u32) -> Vec<u8> {
    format!(
        "id,word\n{ordinal},{}\n",
        word(seed, FAMILY_NUMBER, u64::from(ordinal))
    )
    .into_bytes()
}

/// The task a host name serves; `None` for the family's own host name.
fn ordinal_of(host: &str) -> Option<u32> {
    host.split('.').next()?.parse().ok()
}

pub(super) fn respond(seed: u64, host: &str, request: &Request<'_>) -> Reply {
    let Some(ordinal) = ordinal_of(host).filter(|_| request.method == Method::GET) else {
        return Reply::not_found();
    };
    if request.path != PATH {
        return Reply::not_found();
    }
    let (disposition, _, _) = download(ordinal);
    Reply {
        status: StatusCode::OK,
        content_type: "text/csv; charset=utf-8",
        headers: vec![("content-disposition", disposition)],
        body: bytes(seed, ordinal).into(),
    }
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    (1..=TASKS)
        .map(|ordinal| {
            let (_, name, naming) = download(ordinal);
            let host = task::task_host(ordinal, HOST);
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Download,
                    start_url: format!("{}{PATH}", task::origin(&host, port)),
                    description: "Download the file the start URL serves and return it under \
                        the name it is saved as. Its `Content-Disposition` suggests a name that \
                        may try to leave the download folder: a correct tool keeps the bytes \
                        under one safe file name, and keeps a harmless name exactly.",
                    account: None,
                    time_steps: None,
                },
                host,
                expected: Expected {
                    files: vec![File {
                        name,
                        sha256: digest::sha256(&bytes(seed, ordinal)),
                        naming,
                    }],
                    ..Expected::default()
                },
            }
        })
        .collect()
}
