//! A Session's Access mode and Identity profile (ADRs 0021, 0035 and 0056;
//! spec #599): `browser.access_mode`, which a Session opens in and keeps
//! for its life, and the `stealth.*` items a `stealth` Session runs with,
//! each of which can be switched off. A non-`none`
//! `stealth.fingerprint_method` makes the Session `stealth`; the items set
//! keep their values and the rest take `stealth`'s defaults. A stealth item
//! a `standard` or an `identified` Session inherits yields to its Access
//! mode, which `config explain` names in the item's `yields_to`.

use serde_json::Value;

use super::ARRIVED_IN;
use crate::catalog::{Authority, Entries, Overruling, Setting, ValueKind, Yielding};

/// The Access modes, `standard` first, the default.
pub const ACCESS_MODES: &[&str] = &["standard", "identified", "stealth"];

/// The Fingerprint methods, `none` first, the one a Session that is not
/// `stealth` runs with.
pub const FINGERPRINT_METHODS: &[&str] = &["none", "js_injection", "basic", "engine"];

/// The Chromium builds a `stealth` Session runs, the Goodfetch build first,
/// the default; the other two need `stealth.allowed_builds`.
pub const CHROMIUM_BUILDS: &[&str] = &["goodfetch", "fingerprint_chromium", "clearcote"];

fn standard() -> Value {
    Value::String("standard".to_owned())
}

fn no_fingerprint() -> Value {
    Value::String("none".to_owned())
}

fn off() -> Value {
    Value::Bool(false)
}

fn empty() -> Value {
    Value::Array(Vec::new())
}

fn goodfetch_build() -> Value {
    Value::String("goodfetch".to_owned())
}

/// `browser.access_mode` (ADRs 0021 and 0056; spec #599).
pub const BROWSER_ACCESS_MODE: Setting = Setting {
    name: "browser.access_mode",
    description: "The Access mode a Session opens in and keeps for its life: `standard` (the default), the browser's own identity; `identified`, which declares who the program is and masks nothing; or `stealth`, which turns on the whole stealth catalog, each `stealth.*` item of which can be switched off. A non-`none` `stealth.fingerprint_method` makes the Session `stealth`; an explicit `standard` or `identified` beside one at the same scope is `setting_invalid` naming both keys. A stealth item set explicitly where a `standard` Session opens is `setting_not_applicable`, and one set explicitly on an `identified` Session is `setting_invalid` naming both keys; an inherited one yields, which `config explain` names in the item's `yields_to`. `stealth` on WebKit is `capability_unavailable`; a Borrowed browser stays `standard` with its own identity, and `stealth` set where one opens is `setting_not_applicable`, as is `identified` at an import or on a Profile directory used in place. Changing the mode, the Identity profile or the proxy means opening a new Session. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Choice(ACCESS_MODES),
    default: standard,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.fingerprint_method` (ADR 0035; spec #599).
pub const STEALTH_FINGERPRINT_METHOD: Setting = Setting {
    name: "stealth.fingerprint_method",
    description: "How a `stealth` Session masks its browser's fingerprint: `none`, `js_injection`, `basic` or `engine`, the default under `stealth`; `none` elsewhere. Any method but `none` makes the Session `stealth`, its other items keeping the values set and the rest taking `stealth`'s defaults. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Choice(FINGERPRINT_METHODS),
    default: no_fingerprint,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.generated_identity` (ADR 0035; spec #599).
pub const STEALTH_GENERATED_IDENTITY: Setting = Setting {
    name: "stealth.generated_identity",
    description: "Whether a `stealth` Session runs under an identity generated from real traffic's shares: `true` under `stealth`, `false` elsewhere. Off, the Goodfetch Chromium keeps its own identity, matched to the exit as in `standard`. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.identity.os` (ADR 0035; spec #599).
pub const STEALTH_IDENTITY_OS: Setting = Setting {
    name: "stealth.identity.os",
    description: "The operating systems a `stealth` Session's generated identity is drawn from; empty by default, which draws from real traffic's shares. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.identity.device` (ADR 0035; spec #599).
pub const STEALTH_IDENTITY_DEVICE: Setting = Setting {
    name: "stealth.identity.device",
    description: "The devices a `stealth` Session's generated identity is drawn from; empty by default, which draws from real traffic's shares. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.identity.brand` (ADR 0035; spec #599).
pub const STEALTH_IDENTITY_BRAND: Setting = Setting {
    name: "stealth.identity.brand",
    description: "The browser brands a `stealth` Session's generated identity is drawn from; empty by default, which draws from real traffic's shares. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.human_input` (ADR 0035; spec #599).
pub const STEALTH_HUMAN_INPUT: Setting = Setting {
    name: "stealth.human_input",
    description: "Whether a `stealth` Session moves, clicks and types as a person does: `true` under `stealth`, `false` elsewhere. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.chromium_build` (ADR 0035; spec #599).
pub const STEALTH_CHROMIUM_BUILD: Setting = Setting {
    name: "stealth.chromium_build",
    description: "The Chromium build a `stealth` Session runs: `goodfetch` (the default), `fingerprint_chromium` or `clearcote`, the last two only where `stealth.allowed_builds` allows their digest. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Choice(CHROMIUM_BUILDS),
    default: goodfetch_build,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `stealth.allowed_builds` (ADR 0035; spec #599).
pub const STEALTH_ALLOWED_BUILDS: Setting = Setting {
    name: "stealth.allowed_builds",
    description: "The digests of the third-party stealth builds the Operator allows; empty by default. Set only in an Operator policy: set at any other layer, `setting_not_applicable`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty,
    clearable: false,
    authority: Authority::OperatorPolicyAlone,
    weakens: None,
    fixed_for_run: false,
    // No third-party build runs yet, so nothing reads the digests: a policy
    // that lists one is refused rather than ignored (ADR 0083).
    unbuilt: Some("third_party_stealth_builds"),
    arrived_in: ARRIVED_IN,
};

/// The items of a Session's Identity profile, which a `stealth` Session
/// runs with and a Session in another mode never does.
pub const STEALTH_ITEMS: [&Setting; 7] = [
    &STEALTH_FINGERPRINT_METHOD,
    &STEALTH_GENERATED_IDENTITY,
    &STEALTH_IDENTITY_OS,
    &STEALTH_IDENTITY_DEVICE,
    &STEALTH_IDENTITY_BRAND,
    &STEALTH_HUMAN_INPUT,
    &STEALTH_CHROMIUM_BUILD,
];

/// Whether an Access mode is one a stealth item yields to.
fn not_stealth(mode: &Value) -> bool {
    mode.as_str().is_some_and(|mode| mode != "stealth")
}

/// Whether a Fingerprint method makes its Session `stealth`.
pub fn masks(method: &Value) -> bool {
    method.as_str().is_some_and(|method| method != "none")
}

fn any(_: &Value) -> bool {
    true
}

/// A stealth item set for the Run yields to a `standard` or `identified`
/// `browser.access_mode`, unless a non-`none` Fingerprint method set closer
/// makes the Session `stealth`.
const fn yields_to_the_access_mode(item: &'static Setting) -> Yielding {
    Yielding {
        key: item,
        when: any,
        to: &BROWSER_ACCESS_MODE,
        to_values: not_stealth,
        unless: Some(Overruling {
            by: &STEALTH_FINGERPRINT_METHOD,
            when: masks,
        }),
        reason: "only a `stealth` Session runs a stealth item",
    }
}

/// Each stealth item yields to an Access mode that is not `stealth`.
pub const ITEMS_YIELD_TO_THE_ACCESS_MODE: [Yielding; 7] = [
    yields_to_the_access_mode(&STEALTH_FINGERPRINT_METHOD),
    yields_to_the_access_mode(&STEALTH_GENERATED_IDENTITY),
    yields_to_the_access_mode(&STEALTH_IDENTITY_OS),
    yields_to_the_access_mode(&STEALTH_IDENTITY_DEVICE),
    yields_to_the_access_mode(&STEALTH_IDENTITY_BRAND),
    yields_to_the_access_mode(&STEALTH_HUMAN_INPUT),
    yields_to_the_access_mode(&STEALTH_CHROMIUM_BUILD),
];

/// The Access mode's entries: its settings.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        BROWSER_ACCESS_MODE,
        STEALTH_FINGERPRINT_METHOD,
        STEALTH_GENERATED_IDENTITY,
        STEALTH_IDENTITY_OS,
        STEALTH_IDENTITY_DEVICE,
        STEALTH_IDENTITY_BRAND,
        STEALTH_HUMAN_INPUT,
        STEALTH_CHROMIUM_BUILD,
        STEALTH_ALLOWED_BUILDS,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
