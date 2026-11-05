//! `formats` through the crate's interface, over committed fixtures and
//! generated documents; no owned code is mocked (ADR 0062).

mod contacts;
mod content_modes;
mod demand_plan;
mod envelope;
mod extractors;
mod live_document;
mod markdown;
mod metadata;
mod properties;
mod references;
mod tables;
mod unicode;

use goodfetch_protocol::page::{FormatsRequest, PageFormats};

use crate::{Context, Document, Serialization, formats};

/// The URL every fixture document is read at.
const URL: &str = "https://kites.test/journal/bridle";

/// The formats `request` (as JSON) names, read from `html` at [`URL`].
fn read(html: &str, request: serde_json::Value) -> PageFormats {
    let request: FormatsRequest = serde_json::from_value(request).unwrap();
    formats(
        &Document {
            html,
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        &request,
        &Context::default(),
    )
}

/// The HTML entry point's fuzz target: its committed seeds and crashers
/// (ADR 0067).
mod formats_target {
    goodfetch_test_support::fuzz_replay!(crate::fuzz_formats, "fuzz/formats");
}
