//! The session export bundle's writer for the Compatibility corpus (ADR
//! 0066): the document a bundle encrypts, every field of it set, written
//! into the folder given as the only argument as `session.json`. The age
//! file around it is age's own format, and no two encryptions of a document
//! are the same bytes. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes a file (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::session_bundle::document;
use goodfetch_protocol::formats::SESSION_EXPORT;
use goodfetch_protocol::session_export::{
    ExportedCookie, ExportedOrigin, SameSite, SessionExport, SessionValue, StorageItem,
};

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let export = SessionExport {
        format_version: SESSION_EXPORT.version,
        cookies: vec![
            ExportedCookie {
                name: "sid".to_owned(),
                value: SessionValue::new("synthetic session"),
                domain: ".shop.goodfetch.test".to_owned(),
                path: "/".to_owned(),
                expires: Some(1_798_761_600),
                http_only: true,
                secure: true,
                same_site: Some(SameSite::Lax),
            },
            ExportedCookie {
                name: "locale".to_owned(),
                value: SessionValue::new("fr-FR"),
                domain: "shop.goodfetch.test".to_owned(),
                path: "/account".to_owned(),
                expires: None,
                http_only: false,
                secure: false,
                same_site: None,
            },
        ],
        origins: vec![ExportedOrigin {
            origin: "https://shop.goodfetch.test".to_owned(),
            local_storage: vec![StorageItem {
                name: "cart".to_owned(),
                value: SessionValue::new("3 items"),
            }],
            session_storage: vec![StorageItem {
                name: "step".to_owned(),
                value: SessionValue::new("payment"),
            }],
        }],
    };
    std::fs::write(output.join("session.json"), document(&export))
        .expect("invariant: the output folder is writable");
}
