//! A page's formats read once extraction-side removal ran (ADR 0047): the
//! elements its Session's cosmetic rules match go from its serialized
//! document, shadow roots included, before the extractor reads it; what
//! went, and the share of the main content's text it took, is a Provenance
//! entry, flagged above a fifth.

use goodfetch_clean_up_rules::removal::{self, Removed, Share};
use goodfetch_extractor::{Context, Document};
use goodfetch_protocol::page::{FormatsRequest, PageFormats, PageId, SessionId};
use serde_json::{Value, json};

use crate::driver::SerializedDocument;
use crate::interception::Cosmetic;

/// What removal took from one page's document, as Provenance records it.
pub(super) struct Removal {
    url: String,
    removed: Vec<Removed>,
    removed_text: usize,
    share: Share,
}

impl Removal {
    /// Its Provenance entry, for the page `page_id` of the Session
    /// `session_id`.
    pub(super) fn entry(&self, page_id: &PageId, session_id: &SessionId) -> Value {
        json!({
            "action": "content_removed",
            "page_id": page_id,
            "session_id": session_id,
            "url": self.url,
            "removed": self.removed.iter().map(|element| json!({
                "rule": element.rule,
                "list": element.list.name(),
                "in_shadow_root": element.in_shadow_root,
                "text": element.text,
                "links": element.links,
            })).collect::<Vec<_>>(),
            "removed_text": self.removed_text,
            "main_content": { "before": self.share.before, "after": self.share.after },
            "share": self.share.taken(),
            "flagged": self.share.flagged(),
        })
    }
}

/// The formats `request` names, read from `document` once the cosmetic
/// rules `cosmetic` hold removed what they match, and what removal took;
/// none when it took nothing.
pub(super) fn read(
    document: &SerializedDocument,
    request: FormatsRequest,
    cosmetic: Option<&Cosmetic>,
) -> (PageFormats, Option<Removal>) {
    let of = |html| Document {
        html,
        url: &document.url,
        base_url: &document.base_url,
    };
    let Some(cosmetic) = cosmetic else {
        return (
            goodfetch_extractor::formats(&of(&document.html), &request, &Context::default()),
            None,
        );
    };
    let removal = removal::remove(
        &document.html,
        &document.url,
        &cosmetic.lists,
        &cosmetic.choices,
    );
    let formats = goodfetch_extractor::formats(&of(&removal.html), &request, &Context::default());
    if removal.removed.is_empty() {
        return (formats, None);
    }
    let share = Share::of(
        goodfetch_extractor::main_content_length(&of(&document.html)),
        goodfetch_extractor::main_content_length(&of(&removal.html)),
    );
    (
        formats,
        Some(Removal {
            url: document.url.clone(),
            removed: removal.removed,
            removed_text: removal.removed_text,
            share,
        }),
    )
}
