//! The block-detection rules, as data: each with the Block cause it gives,
//! its source and license (for the notice inventory), the statuses it reads
//! and the markers its document must hold. The rules seeded from outside
//! sources join here with theirs (stories 60 and 61).

use goodfetch_protocol::block::BlockCause;

/// One rule: it fires on a response whose status is one of `statuses` and
/// whose document holds, for each group of `markers`, one of its markers.
pub(super) struct Rule {
    pub(super) name: &'static str,
    pub(super) cause: BlockCause,
    pub(super) source: &'static str,
    pub(super) license: &'static str,
    pub(super) statuses: &'static [u16],
    /// Lowercase text, matched in the lowercased markup.
    pub(super) markers: &'static [&'static [&'static str]],
}

/// Goodfetch's own rules' source and license.
const GOODFETCH_CORPUS: &str = "Goodfetch, from the Proving ground's block-detection corpus";
const GOODFETCH_LICENSE: &str = "MIT OR Apache-2.0";

/// Every rule, in the order they are tried.
pub(super) const RULES: &[Rule] = &[
    // A vendor-neutral interstitial challenge: a refusal whose page says it
    // checks the browser and needs JavaScript to let it through. A vendor's
    // script on an ordinary page holds neither.
    Rule {
        name: "interstitial_challenge",
        cause: BlockCause::Challenge,
        source: GOODFETCH_CORPUS,
        license: GOODFETCH_LICENSE,
        statuses: &[403, 429, 503],
        markers: &[
            &[
                "just a moment",
                "checking your browser",
                "checking if the site connection is secure",
            ],
            &[
                "javascript and cookies",
                "needs javascript",
                "requires javascript",
                "enable javascript",
            ],
        ],
    },
];
