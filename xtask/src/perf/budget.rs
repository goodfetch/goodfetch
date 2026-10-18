//! The Performance budget's numbers as constants (ADR 0071, as amended by
//! ADR 0087). They change only by changing the ADR that sets them; only the
//! owner moves a ceiling.

/// ADR 0071: a benchmark's figure trips at +5% over the base's.
pub(crate) const INCREASE_PERCENT: u64 = 5;

/// ADR 0071: a size-limit entry trips at +5 kB brotli over the base's (the
/// ADRs' kB is 1,000 bytes).
pub(crate) const BUNDLE_INCREASE_BYTES: u64 = 5_000;

/// ADRs 0071 and 0087: each Product's whole embedded Console build stays
/// under 1 MB brotli, whatever the pull request says.
pub(crate) const CONSOLE_CEILING_BYTES: u64 = 1_000_000;

/// The size-limit entries that are each Product's whole embedded Console
/// build (ADR 0087), by the `name` the size-limit configuration gives them.
pub(crate) const CONSOLE_ENTRIES: [&str; 2] = ["collie-console", "foxhound-console"];

/// ADR 0071: a new or changed benchmark runs this many times on one build
/// for its admission.
pub(crate) const ADMISSION_RUNS: usize = 20;

/// ADR 0071: across the admission runs, every gated figure spreads less
/// than 0.5%, a tenth of the threshold, in hundredths of a percent.
pub(crate) const ADMISSION_SPREAD_BASIS_POINTS: u64 = 50;

/// ADR 0071: the drift check flags a figure up more than 10% since the last
/// release.
pub(crate) const DRIFT_PERCENT: u64 = 10;

/// ADR 0071: the drift check flags a resident-memory peak within 10% of its
/// ceiling.
pub(crate) const CEILING_MARGIN_PERCENT: u64 = 10;
