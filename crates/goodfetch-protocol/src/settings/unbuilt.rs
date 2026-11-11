//! The settings whose feature the engine does not build yet (ADR 0083:
//! "A value that cannot take effect where it is set is refused when the
//! scope opens, never ignored, clamped or dropped"). No product code reads
//! one of these keys, so a scope that sets one away from its default is
//! refused as it opens, with `capability_unavailable` naming the key,
//! where it would otherwise run as if the value applied.
//!
//! This is the one list. A key leaves it in the change that builds its
//! reader, and the engine's settings tests keep it honest: every catalog
//! setting is read by product code, listed here, or locked at its default
//! for every layer.

use crate::catalog::Setting;
use crate::foxhound::CRAWL_SAMPLE_PAGES;
use crate::foxhound::settings::{
    PAID_FETCH_CHAIN_RPC_ENDPOINTS, PAID_FETCH_DOMAINS, PAID_FETCH_EXCHANGE_RATES,
    PAID_FETCH_MAX_PRICE_PER_PAGE,
};
use crate::sample::SAMPLE_ECHO_REPEATS;
use crate::security::{SECURITY_FIELD_ENCRYPTION, SECURITY_SECURE_DELETE};
use crate::settings::WALL_DOMAINS;

/// A setting whose feature is not built, and the feature.
#[derive(Clone, Copy)]
pub struct Unbuilt {
    /// The setting no product code reads.
    pub setting: &'static Setting,
    /// What a value away from its default asks for, as
    /// `capability_unavailable` names it in `kind`.
    pub feature: &'static str,
}

/// Every setting whose feature is not built, in the catalog's order.
///
/// `security.field_encryption` and `security.secure_delete` are here for
/// their `false`: the engine seals and secure-deletes whatever they say,
/// so until their switch is built, off is refused where it would be
/// reported as a weakened Run that was not one.
pub const UNBUILT: [Unbuilt; 9] = [
    Unbuilt {
        setting: &SAMPLE_ECHO_REPEATS,
        feature: "sample_echo_repeats",
    },
    Unbuilt {
        setting: &WALL_DOMAINS,
        feature: "wall_reading",
    },
    Unbuilt {
        setting: &SECURITY_FIELD_ENCRYPTION,
        feature: "unsealed_fields",
    },
    Unbuilt {
        setting: &SECURITY_SECURE_DELETE,
        feature: "plain_delete",
    },
    Unbuilt {
        setting: &CRAWL_SAMPLE_PAGES,
        feature: "sample_crawl_pages",
    },
    Unbuilt {
        setting: &PAID_FETCH_DOMAINS,
        feature: "paid_fetch",
    },
    Unbuilt {
        setting: &PAID_FETCH_MAX_PRICE_PER_PAGE,
        feature: "paid_fetch",
    },
    Unbuilt {
        setting: &PAID_FETCH_EXCHANGE_RATES,
        feature: "paid_fetch",
    },
    Unbuilt {
        setting: &PAID_FETCH_CHAIN_RPC_ENDPOINTS,
        feature: "paid_fetch",
    },
];
