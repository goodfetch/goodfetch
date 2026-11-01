//! The page formats (spec #580; ADR 0007): what a program reads from a
//! page's document, such as its main content as Markdown, through
//! [`formats`] over the document serialized with its URL and base URL. The
//! main content is the [`Extractor`] `content.extractor` chose, dom-smoothie
//! by default or rs-trafilatura, and the Markdown htmd's, with ADR 0007's
//! two fixes. Its own crate because it is the Extractor Plugin kind's seam
//! (STYLE.md, reason 4; ADR 0033) and pulls heavy parsers nothing else
//! needs. Pure over its input: no I/O, no clock, and no Foxhound type, so a
//! Collie page and a Foxhound handler's page read alike (ADRs 0087, 0088).

mod blocks;
mod character_references;
mod contacts;
mod extractor;
mod formats;
mod light;
mod main_content;
mod markdown;
mod references;
mod shadow;
mod signature;
mod social;
mod structured_data;
mod tables;

pub use extractor::{BuiltIn, Extractor};
pub use formats::{
    CleanUp, Context, Document, Measured, Serialization, formats, fuzz_formats,
    main_content_length, measured_formats,
};
pub use signature::page_signature;
pub use social::SOCIAL_NETWORKS_VERSION;
