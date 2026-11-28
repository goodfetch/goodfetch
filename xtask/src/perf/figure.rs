//! The counted figures `xtask perf` gates (ADR 0071), and what each one
//! measures: a benchmark's instructions and heap metrics, or a size-limit
//! entry's brotli bytes.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Figure {
    /// Callgrind's `Ir`.
    Instructions,
    /// DHAT's total heap blocks allocated.
    TotalAllocations,
    /// DHAT's total heap bytes allocated.
    TotalBytes,
    /// DHAT's bytes alive at the heap's global maximum.
    GlobalMaximumBytes,
    /// A size-limit entry's size, brotli-compressed.
    Brotli,
    /// A protocol payload's wire byte count (MCP `tools/list` and tool results,
    /// ADRs 0065 and 0071; issue 1360).
    WireBytes,
    /// A resident-memory peak, which only the drift check reads.
    ResidentMemory,
}

impl Figure {
    /// The figures of gungraun's summary, by the key of its metric.
    pub(crate) const BENCHMARK: [(&'static str, Self); 4] = [
        ("Ir", Self::Instructions),
        ("TotalBlocks", Self::TotalAllocations),
        ("TotalBytes", Self::TotalBytes),
        ("AtTGmaxBytes", Self::GlobalMaximumBytes),
    ];

    /// Whether a benchmark measures this figure, rather than size-limit or a
    /// resident-memory check.
    pub(crate) fn is_benchmark(self) -> bool {
        Self::BENCHMARK.iter().any(|(_, figure)| *figure == self)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Instructions => "instructions",
            Self::TotalAllocations => "total-allocations",
            Self::TotalBytes => "total-bytes",
            Self::GlobalMaximumBytes => "global-maximum-bytes",
            Self::Brotli => "brotli",
            Self::WireBytes => "wire-bytes",
            Self::ResidentMemory => "resident-memory",
        }
    }
}

impl fmt::Display for Figure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// One figure of one benchmark or entry, with the base's value when the
/// base has one.
#[derive(Clone, Debug)]
pub(crate) struct Measured {
    /// The benchmark's ID, `<crate>/<module path>[/<id>]`, or the entry's
    /// `size-limit/<name>`.
    pub(crate) subject: String,
    pub(crate) figure: Figure,
    pub(crate) base: Option<u64>,
    pub(crate) head: u64,
}

impl Measured {
    /// The identifier a finding and a "Performance change" line name.
    pub(crate) fn id(&self) -> String {
        id(&self.subject, self.figure)
    }
}

/// A figure's identifier, `<subject> <figure>`, which findings, "Performance
/// change" lines, the drift check's issues and the admission name it by.
pub(crate) fn id(subject: &str, figure: impl fmt::Display) -> String {
    format!("{subject} {figure}")
}

/// Whether a value in hundredths is written with its sign when positive.
#[derive(Clone, Copy)]
pub(crate) enum Sign {
    /// `+1.25`, as a change.
    Always,
    /// `1.25`, as a value.
    WhenNegative,
}

/// A value in hundredths with two decimals: `125` is `1.25`.
pub(crate) fn hundredths(value: i128, sign: Sign) -> String {
    let prefix = match (sign, value < 0) {
        (_, true) => "-",
        (Sign::Always, false) => "+",
        (Sign::WhenNegative, false) => "",
    };
    let value = value.unsigned_abs();
    format!("{prefix}{}.{:02}", value / 100, value % 100)
}

/// `numerator / denominator`, rounded away from zero, so a change copied
/// from it is never under the measured one.
pub(crate) fn away_from_zero(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator
        .unsigned_abs()
        .div_ceil(denominator.unsigned_abs());
    let quotient = i128::try_from(quotient).unwrap_or(i128::MAX);
    if (numerator < 0) == (denominator < 0) {
        quotient
    } else {
        -quotient
    }
}
