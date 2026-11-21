//! What a run declares its results are for, its Purpose, and whether the
//! AI-use signals a site sends are applied to it (ADRs 0016, 0045 and
//! 0086): settings of every run of either Product. The signals are read on
//! every run; only with `site_preferences.respect_use_signals` on does a
//! crawl skip a request whose signal refuses the Purpose, with
//! `use_signals`. A Purpose left at `search` is no declaration to judge
//! them against.

use serde_json::Value;

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `site_preferences.purpose`'s values, in the catalog's order (ADR 0016).
pub const PURPOSE_VALUES: &[&str] = &["search", "ai-input", "ai-train", "collect"];

fn search() -> Value {
    Value::String("search".to_owned())
}

fn no() -> Value {
    Value::Bool(false)
}

/// `site_preferences.purpose` (ADRs 0016 and 0086).
pub const SITE_PREFERENCES_PURPOSE: Setting = Setting {
    name: "site_preferences.purpose",
    description: "What the run's results are for, against which the AI-use signals are judged while `site_preferences.respect_use_signals` is on: `search` (the default, which no signal is judged against), `ai-input` (input to an AI model), `ai-train` (training an AI model) or `collect` (plain collection, text and data mining).",
    kind: ValueKind::Choice(PURPOSE_VALUES),
    default: search,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `site_preferences.respect_use_signals` (ADRs 0016, 0045 and 0086).
pub const SITE_PREFERENCES_RESPECT_USE_SIGNALS: Setting = Setting {
    name: "site_preferences.respect_use_signals",
    description: "Whether the AI-use signals a site sends (IETF aipref's `Content-Usage` header, TDMRep's `tdm-reservation` header) are applied to the run's `site_preferences.purpose`: on, a crawl request whose response's signal refuses the Purpose ends `skipped` with `use_signals` and its handler never runs. Off by default; the signals are read either way.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        SITE_PREFERENCES_PURPOSE,
        SITE_PREFERENCES_RESPECT_USE_SIGNALS,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
