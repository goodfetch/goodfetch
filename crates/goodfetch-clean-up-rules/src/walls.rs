//! Generic Wall detection rules and the "withholds the content" test
//! (ADR 0052; spec #577, ticket #757).
//!
//! All detection is generic and rule-data driven: paywall overlays,
//! `isAccessibleForFree: false` JSON-LD declarations, and ad-blocker-wall
//! overlays. Goodfetch's own code never names a paywall vendor, crawler, or
//! archive as a way past a Wall (ADR 0052).
//!
//! Rules are compiled once when the rules version loads and passed into
//! `recovery`'s one composed detection read (`C4.detection_reads`,
//! `C4.compilations`); [`withholds_content`] operates on the main content
//! already produced, adding no parse or readability run (`F1.parses`,
//! `F1.readability_runs`).

/// Minimum extracted main-content word count below which a matched paywall
/// counts as withholding the page's content (ADR 0052).
pub const WITHHOLDS_CONTENT_MIN_WORDS: usize = 150;

/// Generic CSS selectors for paywall overlays, compiled once per rules version.
pub const PAYWALL_OVERLAY_SELECTORS: &[&str] = &[
    "[class*=\"paywall\"][class*=\"overlay\"]",
    "[class*=\"paywall\"][class*=\"modal\"]",
    "[id*=\"paywall\"][id*=\"overlay\"]",
    "[id*=\"paywall\"][id*=\"modal\"]",
    ".paywall-overlay",
    ".paywall-modal",
    ".paywall-wall",
    "#paywall-overlay",
    "#paywall-modal",
    "[data-paywall-overlay]",
    "[data-testid*=\"paywall\"]",
];

/// Generic CSS selectors for ad-blocker-wall overlays, compiled once per rules
/// version.
pub const AD_BLOCKER_WALL_SELECTORS: &[&str] = &[
    "[class*=\"adblock\"][class*=\"wall\"]",
    "[class*=\"adblock\"][class*=\"overlay\"]",
    "[class*=\"ad-blocker\"][class*=\"wall\"]",
    "[id*=\"adblock\"][id*=\"wall\"]",
    ".adblock-wall",
    ".ad-blocker-wall",
    "#adblock-wall",
    "[data-adblock-wall]",
];

/// Compiled Wall detection rules, built once when a Clean-up rules version
/// loads and shared across every Session and page of a Run.
#[derive(Clone, Debug)]
pub struct WallRules {
    paywall_selectors: Vec<String>,
    ad_blocker_wall_selectors: Vec<String>,
}

impl WallRules {
    /// Builds the embedded generic Wall detection rules once.
    #[must_use]
    pub fn embedded() -> Self {
        Self {
            paywall_selectors: PAYWALL_OVERLAY_SELECTORS
                .iter()
                .map(|selector| (*selector).to_owned())
                .collect(),
            ad_blocker_wall_selectors: AD_BLOCKER_WALL_SELECTORS
                .iter()
                .map(|selector| (*selector).to_owned())
                .collect(),
        }
    }

    /// Generic paywall overlay selectors.
    #[must_use]
    pub fn paywall_selectors(&self) -> &[String] {
        &self.paywall_selectors
    }

    /// Generic ad-blocker-wall overlay selectors.
    #[must_use]
    pub fn ad_blocker_wall_selectors(&self) -> &[String] {
        &self.ad_blocker_wall_selectors
    }
}

impl Default for WallRules {
    fn default() -> Self {
        Self::embedded()
    }
}

/// Counts whitespace-delimited words in already-extracted main content text,
/// adding no HTML parse or readability run (`F1.parses`, `F1.readability_runs`).
#[must_use]
pub fn word_count(extracted_main_text: &str) -> usize {
    extracted_main_text.split_whitespace().count()
}

/// ADR 0052's "withholds the content" test: a page withholds its content when
/// the paywall detector matched AND either:
/// - a section the page declares `isAccessibleForFree: false` has none of its
///   text in the served HTML (`declared_paid_section_empty`), OR
/// - the extracted main content of the served page holds fewer than
///   [`WITHHOLDS_CONTENT_MIN_WORDS`] (150) words.
#[must_use]
pub const fn withholds_content(
    paywall_matched: bool,
    declared_paid_section_empty: bool,
    extracted_main_word_count: usize,
) -> bool {
    paywall_matched
        && (declared_paid_section_empty || extracted_main_word_count < WITHHOLDS_CONTENT_MIN_WORDS)
}

#[cfg(test)]
mod tests {
    use super::{WITHHOLDS_CONTENT_MIN_WORDS, withholds_content, word_count};

    #[test]
    fn withholds_content_requires_paywall_match_and_empty_paid_section_or_short_text() {
        assert!(!withholds_content(false, true, 0));
        assert!(!withholds_content(true, false, WITHHOLDS_CONTENT_MIN_WORDS));
        assert!(withholds_content(true, true, 500));
        assert!(withholds_content(true, false, 149));
        assert_eq!(word_count("  one   two\nthree\tfour "), 4);
    }
}
