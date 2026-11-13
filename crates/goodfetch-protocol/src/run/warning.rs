//! The warning table (ADR 0102): the codes the areas hand over to a Run's
//! report, each a value of a type both Products share, to which Foxhound
//! adds its own. A reader keeps a code it does not know, a newer engine's,
//! as it is (G3).

use crate::catalog::{ProtocolVersion, SharedType, SharedTypeValue};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Session's fingerprint is off (area 8).
pub const FINGERPRINT_NONE: &str = "fingerprint_none";
/// A Session's fingerprint is injected by JavaScript (area 8).
pub const FINGERPRINT_JS_INJECTION: &str = "fingerprint_js_injection";
/// A Session runs the basic fingerprint (area 8).
pub const FINGERPRINT_BASIC: &str = "fingerprint_basic";
/// A Session fell back to the basic fingerprint (area 8).
pub const FINGERPRINT_BASIC_FALLBACK: &str = "fingerprint_basic_fallback";
/// A Session's browser build is outdated (area 8).
pub const BROWSER_BUILD_OUTDATED: &str = "browser_build_outdated";
/// A Session runs a third party's browser build (area 8).
pub const BROWSER_BUILD_THIRD_PARTY: &str = "browser_build_third_party";
/// A Session runs Firefox (area 8).
pub const BROWSER_BUILD_FIREFOX: &str = "browser_build_firefox";
/// A Session runs a browser build a Plugin gave (area 8).
pub const BROWSER_BUILD_PLUGIN: &str = "browser_build_plugin";
/// A Session's human input fell back to plain input (area 8).
pub const HUMAN_INPUT_FALLBACK: &str = "human_input_fallback";
/// A Session's proxy exit may change between requests (area 8).
pub const PROXY_EXIT_MAY_CHANGE: &str = "proxy_exit_may_change";
/// A filter list flags a vendor the Run used (area 8).
pub const FILTER_VENDOR_FLAGGED: &str = "filter_vendor_flagged";
/// The host throttled the engine's CPU (ADR 0049).
pub const CPU_THROTTLED: &str = "cpu_throttled";
/// The Run met the `low` storage level (ADR 0122).
pub const STORAGE_LOW: &str = "storage_low";
/// The Politeness store was degraded (ADR 0053).
pub const POLITENESS_STORE_DEGRADED: &str = "politeness_store_degraded";
/// The Run's Clean-up rules are older than 14 days (ADR 0047).
pub const CLEAN_UP_RULES_STALE: &str = "clean_up_rules_stale";
/// The Run holds to other Clean-up rules than its settings ask for (ADR
/// 0047).
pub const CLEAN_UP_RULES_FALLBACK: &str = "clean_up_rules_fallback";
/// A Session blocks resource kinds a site can see missing (ADRs 0047, 0117).
pub const BLOCK_RESOURCES_VISIBLE: &str = "block_resources_visible";
/// A file the Run writes at its end could not be written at its path (ADR
/// 0116).
pub const PATH_NOT_WRITABLE: &str = "path_not_writable";
/// A file the Run writes could not be written to its storage (ADR 0116).
pub const FILE_WRITE_FAILED: &str = "file_write_failed";
/// The baseline a comparison needs was unavailable.
pub const BASELINE_UNAVAILABLE: &str = "baseline_unavailable";
/// A Plugin reported a quantity above its declared maximum (ADR 0033).
pub const PLUGIN_QUANTITY_ABOVE_MAXIMUM: &str = "plugin_quantity_above_maximum";
/// Usage figures come from a Plugin (ADR 0033).
pub const PLUGIN_FIGURES: &str = "plugin_figures";
/// The engine stopped the Run while it stopped on a panic in a part every
/// Run shares (ADR 0148).
pub const ENGINE_PANICKED: &str = "engine_panicked";

const fn warning(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::RunWarningCode,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// The warning codes both Products hand over to a Run's report (ADR
/// 0102); Foxhound adds its own.
pub const WARNING_CODES: [SharedTypeValue; 23] = [
    warning(FINGERPRINT_NONE, "A Session ran with no fingerprint."),
    warning(
        FINGERPRINT_JS_INJECTION,
        "A Session's fingerprint was injected by JavaScript, which a site can detect.",
    ),
    warning(FINGERPRINT_BASIC, "A Session ran the basic fingerprint."),
    warning(
        FINGERPRINT_BASIC_FALLBACK,
        "A Session fell back to the basic fingerprint.",
    ),
    warning(
        BROWSER_BUILD_OUTDATED,
        "A Session's browser build is outdated.",
    ),
    warning(
        BROWSER_BUILD_THIRD_PARTY,
        "A Session ran a third party's browser build.",
    ),
    warning(BROWSER_BUILD_FIREFOX, "A Session ran Firefox."),
    warning(
        BROWSER_BUILD_PLUGIN,
        "A Session ran a browser build a Plugin gave.",
    ),
    warning(
        HUMAN_INPUT_FALLBACK,
        "A Session's human input fell back to plain input.",
    ),
    warning(
        PROXY_EXIT_MAY_CHANGE,
        "A Session's proxy exit may change between its requests.",
    ),
    warning(
        FILTER_VENDOR_FLAGGED,
        "A filter list flags a vendor the Run used.",
    ),
    warning(CPU_THROTTLED, "The host throttled the engine's CPU."),
    warning(
        STORAGE_LOW,
        "The Run met the `low` storage level: the folder, its free space and what was shed.",
    ),
    warning(
        POLITENESS_STORE_DEGRADED,
        "The Politeness store was unreachable, and the Run took a degraded share.",
    ),
    warning(
        CLEAN_UP_RULES_STALE,
        "The Run's Clean-up rules are older than 14 days.",
    ),
    warning(
        CLEAN_UP_RULES_FALLBACK,
        "The Run holds to other Clean-up rules than `clean_up.update` or `clean_up.version` ask for, such as the embedded part when the daily channel cannot be reached; Provenance names both.",
    ),
    warning(
        BLOCK_RESOURCES_VISIBLE,
        "A Session's `browser.block_resources` blocks loads a site can see missing, which few people block.",
    ),
    warning(
        PATH_NOT_WRITABLE,
        "A file the Run writes at its end could not be written at its path.",
    ),
    warning(
        FILE_WRITE_FAILED,
        "A file the Run writes, such as a periodic Checkpoint, could not be written to its storage.",
    ),
    warning(
        BASELINE_UNAVAILABLE,
        "The baseline a comparison needs was unavailable.",
    ),
    warning(
        PLUGIN_QUANTITY_ABOVE_MAXIMUM,
        "A Plugin reported a quantity above its declared maximum, recorded in full.",
    ),
    warning(PLUGIN_FIGURES, "Usage figures come from a Plugin."),
    warning(
        ENGINE_PANICKED,
        "The engine stopped the Run as it stopped on a bug in a part every Run shares; a start with its Run ID resumes it.",
    ),
];
