//! The Performance budget's numbers as constants (ADR 0071, as amended by
//! ADRs 0087 and 0134). They change only by changing the ADR that sets them;
//! only the owner moves a ceiling.

use super::figure::Figure;

/// Whether the budget's findings fail a change: a figure past its threshold
/// that the pull request does not accept, and a figure over its ceiling.
/// A broken measurement (a benchmark that fails its admission, a ceiling
/// whose figure the head lost) fails either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gating {
    /// Each is a note in the gate's log, and fails nothing.
    Informational,
    /// Each fails the change.
    #[expect(
        dead_code,
        reason = "M1's optimization pass sets `GATING` to it (ADR 0134, as amended on 2026-10-22)"
    )]
    Enforced,
}

impl Gating {
    /// What a finding of the budget says after its figure: `enforced`, the
    /// remedy, while the budget gates.
    pub(crate) fn remedy(self, enforced: &'static str) -> &'static str {
        match self {
            Self::Informational => {
                "noted only: until M1's optimization pass, no figure past its threshold or over its ceiling fails a change (ADR 0134, as amended on 2026-10-22)"
            }
            Self::Enforced => enforced,
        }
    }
}

/// ADR 0134, as amended on 2026-10-22 (owner decision): informational until
/// M1's optimization pass (#1577). The pull request of that pass, which sets
/// the tuned ceilings, sets this to `Gating::Enforced`, and the per-change
/// budget and the ceilings gate again.
pub(crate) const GATING: Gating = Gating::Informational;

/// ADR 0071: a benchmark's figure trips at +5% over the base's.
pub(crate) const INCREASE_PERCENT: u64 = 5;

/// ADR 0071: a size-limit entry trips at +5 kB brotli over the base's (the
/// ADRs' kB is 1,000 bytes).
pub(crate) const BUNDLE_INCREASE_BYTES: u64 = 5_000;

/// ADR 0134: at most 8 heap bytes per URL in the Frontier.
const FRONTIER_BYTES_PER_URL: u64 = 8;

/// ADR 0134: the Frontier's bytes per URL are measured at 100,000 URLs.
const FRONTIER_URLS: u64 = 100_000;

/// ADR 0134: a crawled page allocates at most twice its body's bytes.
const PAGE_BYTES_PER_BODY_BYTE: u64 = 2;

/// One absolute ceiling, keyed by the benchmarks or entries whose figures it
/// reads. A "Performance change" section never passes one (ADR 0134).
pub(crate) enum Ceiling {
    /// One figure of one benchmark or size-limit entry, at most `max`.
    #[expect(
        dead_code,
        reason = "ADR 0134's tuned ceilings are of this kind, and none is approved yet"
    )]
    Figure {
        subject: &'static str,
        figure: Figure,
        max: u64,
    },
    /// Heap bytes per URL: the bytes at the global maximum of `subject`,
    /// which holds `urls` URLs, less those of `empty`, the same scenario with
    /// no URL, over `urls`, at most `max_bytes`.
    PerUrl {
        subject: &'static str,
        empty: &'static str,
        urls: u64,
        max_bytes: u64,
    },
    /// Bytes allocated per page as a multiple of body bytes: the total bytes
    /// of `subject` over the total body bytes its fixture records as
    /// `body-bytes` in the TOML file `fixture`, relative to the workspace,
    /// at most `max_multiple`.
    PerBodyByte {
        subject: &'static str,
        fixture: &'static str,
        max_multiple: u64,
    },
}

impl Ceiling {
    /// The benchmarks or entries whose figures it reads.
    pub(crate) fn subjects(&self) -> Vec<&'static str> {
        match self {
            Self::Figure { subject, .. } | Self::PerBodyByte { subject, .. } => vec![subject],
            Self::PerUrl { subject, empty, .. } => vec![subject, empty],
        }
    }

    /// The benchmark or entry it is named after, and its number.
    pub(crate) fn limit(&self) -> (&'static str, u64) {
        match self {
            Self::Figure { subject, max, .. } => (subject, *max),
            Self::PerUrl {
                subject, max_bytes, ..
            } => (subject, *max_bytes),
            Self::PerBodyByte {
                subject,
                max_multiple,
                ..
            } => (subject, *max_multiple),
        }
    }
}

/// The fixed ceilings of ADR 0134. Each starts when its benchmark first
/// runs.
pub(crate) const FIXED_CEILINGS: [Ceiling; 2] = [
    Ceiling::PerUrl {
        subject: "goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls",
        empty: "goodfetch-frontier/heap::frontier::enqueue/no_urls",
        urls: FRONTIER_URLS,
        max_bytes: FRONTIER_BYTES_PER_URL,
    },
    Ceiling::PerBodyByte {
        subject: "goodfetch-foxhound/heap::crawl::in_memory_pages",
        fixture: concat!(
            crate::products::foxhound_folder!(),
            "/benches/pages.source.toml"
        ),
        max_multiple: PAGE_BYTES_PER_BODY_BYTE,
    },
];

/// ADR 0134's tuned ceilings: instructions, allocations and bytes allocated
/// per dispatched JSON-RPC message, each 1.10 times its figure after M1's
/// optimization pass, rounded up, once the owner approves them.
pub(crate) const TUNED_CEILINGS: [Ceiling; 0] = [];

/// Every absolute ceiling.
pub(crate) fn ceilings() -> impl Iterator<Item = &'static Ceiling> {
    FIXED_CEILINGS.iter().chain(TUNED_CEILINGS.iter())
}

/// The ceiling on one figure of one benchmark or entry, when it has one.
pub(crate) fn figure_ceiling(of: &str, of_figure: Figure) -> Option<u64> {
    ceilings().find_map(|ceiling| match ceiling {
        Ceiling::Figure {
            subject,
            figure,
            max,
        } if *subject == of && *figure == of_figure => Some(*max),
        _ => None,
    })
}

/// ADR 0071: a new or changed benchmark runs this many times on one build
/// for its admission.
pub(crate) const ADMISSION_RUNS: usize = 20;

/// ADR 0071: across the admission runs, every gated figure spreads less
/// than 0.5%, a tenth of the threshold, in hundredths of a percent.
pub(crate) const ADMISSION_SPREAD_BASIS_POINTS: u64 = 50;

/// ADR 0071: the drift check flags a figure up more than 10% since the last
/// release. ADR 0140: no figure passes 1.10 times its pinned reference build.
pub(crate) const DRIFT_PERCENT: u64 = 10;

/// Whether `value` is past `DRIFT_PERCENT` over `earlier`.
pub(crate) fn drifted(earlier: u64, value: u64) -> bool {
    u128::from(value) * 100 > u128::from(earlier) * u128::from(100 + DRIFT_PERCENT)
}

/// ADR 0071: the drift check flags a resident-memory peak within 10% of its
/// ceiling.
pub(crate) const CEILING_MARGIN_PERCENT: u64 = 10;
