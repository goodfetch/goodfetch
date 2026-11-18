//! A Session's Access mode and Identity profile (ADRs 0021, 0035 and 0056;
//! spec #599): `browser.access_mode`, which a Session opens in and keeps
//! for its life, and the `stealth.*` items a `stealth` Session runs with,
//! each of which can be switched off. A non-`none`
//! `stealth.fingerprint_method` makes the Session `stealth`; the items set
//! keep their values and the rest take `stealth`'s defaults. A stealth item
//! a `standard` or an `identified` Session inherits yields to its Access
//! mode, which `config explain` names in the item's `yields_to`.

use serde_json::{Value, json};

use super::ARRIVED_IN;
use crate::catalog::{Authority, Entries, Overruling, Setting, ValueKind, ValueShape, Yielding};

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
    description: "The Access mode a Session opens in and keeps for its life: `standard` (the default), the browser's own identity; `identified`, which declares who the program is, signing each request it sends with `identified.signing_key` (Web Bot Auth), and masks nothing; or `stealth`, which turns on the whole stealth catalog, each `stealth.*` item of which can be switched off. A non-`none` `stealth.fingerprint_method` makes the Session `stealth`; an explicit `standard` or `identified` beside one at the same scope is `setting_invalid` naming both keys. A stealth item set explicitly where a `standard` Session opens is `setting_not_applicable`, and one set explicitly on an `identified` Session is `setting_invalid` naming both keys; an inherited one yields, which `config explain` names in the item's `yields_to`. `stealth` on WebKit is `capability_unavailable`; a Borrowed browser stays `standard` with its own identity, and `stealth` set where one opens is `setting_not_applicable`, as is `identified` at an import or on a Profile directory used in place. Changing the mode, the Identity profile or the proxy means opening a new Session. A Label refuses it with `setting_not_applicable`.",
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

fn none() -> Value {
    Value::Null
}

/// `identified.signing_key` (ADRs 0021 and 0045; #1551).
pub const IDENTIFIED_SIGNING_KEY: Setting = Setting {
    name: "identified.signing_key",
    description: "The Secret reference of the Ed25519 key an `identified` Session signs each of its requests with (Web Bot Auth), such as `env:BOT_KEY`: never a value. It resolves to the key as a private JSON Web Key (`kty` `OKP`, `crv` `Ed25519`, with `d` and `x`), resolved once when the Session opens and never written to a log, a trace, a process's arguments or a file; a rotated key is read by the next Session. None by default; an `identified` Session needs it, with `identified.signature_agent`. Set where a Session that is not `identified` opens, `setting_not_applicable`; inherited, it yields to the Access mode. Set only by the program's own code, configuration and an Operator policy: a Recipe may expose it but never sets it, and a Client acting for a model is refused with `setting_locked`. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The longest `identified.signature_agent`, which an inline key directory
/// of a few keys fits.
const SIGNATURE_AGENT_LIMIT: usize = 4096;

/// What `identified.signature_agent` holds.
const SIGNATURE_AGENT_FORMS: &str = "an `https://` or `http://` URL of the key directory's origin, or an inline `data:application/http-message-signatures-directory;base64,` directory";

/// `identified.signature_agent`'s values: where a site reads the key
/// directory, sent as `Signature-Agent`'s string, so printable ASCII
/// without `"` or `\`.
pub const SIGNATURE_AGENT_SHAPE: ValueShape = ValueShape {
    name: "SignatureAgent",
    check: |value| {
        let Some(text) = value.as_str() else {
            return Err(format!("{SIGNATURE_AGENT_FORMS}, found `{value}`"));
        };
        let form = [
            "https://",
            "http://",
            "data:application/http-message-signatures-directory",
        ]
        .iter()
        .any(|prefix| text.len() > prefix.len() && text.starts_with(prefix));
        if !form {
            return Err(format!("{SIGNATURE_AGENT_FORMS}, found `{text}`"));
        }
        if text.len() > SIGNATURE_AGENT_LIMIT {
            return Err(format!(
                "at most {SIGNATURE_AGENT_LIMIT} bytes, found {}",
                text.len()
            ));
        }
        match text
            .chars()
            .find(|character| !matches!(character, ' '..='~') || matches!(character, '"' | '\\'))
        {
            Some(character) => Err(format!(
                "printable ASCII without `\"` or `\\`, found {character:?}"
            )),
            None => Ok(()),
        }
    },
    schema: || {
        json!({
            "type": "string",
            "maxLength": SIGNATURE_AGENT_LIMIT,
            "pattern": "^(https?://|data:application/http-message-signatures-directory)[ !#-\\[\\]-~]+$",
            "description": "Where a site reads the key directory: an `https://` or `http://` URL of the origin that serves it at `/.well-known/http-message-signatures-directory`, or an inline `data:application/http-message-signatures-directory;base64,` directory.",
        })
    },
    examples: || vec![json!("https://bot.example")],
};

/// `identified.signature_agent` (ADRs 0021 and 0045; #1551).
pub const IDENTIFIED_SIGNATURE_AGENT: Setting = Setting {
    name: "identified.signature_agent",
    description: "Where a site reads the key directory of an `identified` Session's key, sent with each signed request as `Signature-Agent`, in the string form Cloudflare accepts: the `https://` origin that serves `goodfetch signing-key directory`'s output at `/.well-known/http-message-signatures-directory`, such as `https://bot.example`, or an inline `data:application/http-message-signatures-directory;base64,` directory. None by default; an `identified` Session needs it, with `identified.signing_key`. Set where a Session that is not `identified` opens, `setting_not_applicable`; inherited, it yields to the Access mode. A Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Shape(&SIGNATURE_AGENT_SHAPE),
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// What an `identified` Session signs its requests with, which a Session in
/// another mode never does.
pub const IDENTIFIED_ITEMS: [&Setting; 2] = [&IDENTIFIED_SIGNING_KEY, &IDENTIFIED_SIGNATURE_AGENT];

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

/// Whether an Access mode is one an item of [`IDENTIFIED_ITEMS`] yields to.
fn not_identified(mode: &Value) -> bool {
    mode.as_str().is_some_and(|mode| mode != "identified")
}

fn set(value: &Value) -> bool {
    !value.is_null()
}

/// An item of [`IDENTIFIED_ITEMS`] set for the Run yields to an Access mode
/// that is not `identified`.
const fn yields_to_identified(item: &'static Setting) -> Yielding {
    Yielding {
        key: item,
        when: set,
        to: &BROWSER_ACCESS_MODE,
        to_values: not_identified,
        unless: None,
        reason: "only an `identified` Session signs its requests",
    }
}

/// Each item of [`IDENTIFIED_ITEMS`] yields to an Access mode that is not
/// `identified`.
pub const SIGNING_YIELDS_TO_THE_ACCESS_MODE: [Yielding; 2] = [
    yields_to_identified(&IDENTIFIED_SIGNING_KEY),
    yields_to_identified(&IDENTIFIED_SIGNATURE_AGENT),
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
        IDENTIFIED_SIGNING_KEY,
        IDENTIFIED_SIGNATURE_AGENT,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
