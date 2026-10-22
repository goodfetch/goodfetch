//! The block-detection rules, as data: each with the Block cause it gives,
//! its source and license (for the notice inventory), the statuses it reads
//! and the headers and markers its response must hold. A rule seeded from an
//! outside source names it, and that source is an input of
//! `third-party.toml` beside this file with the same license, so `xtask
//! notices` lists it (story 60). The vendors the rules cover are listed, by
//! rule, in `vendors.toml` beside this file, for the watchlist job (story
//! 61).

use goodfetch_protocol::block::BlockCause;

/// One rule: it fires on a response whose status is one of `statuses`,
/// whose headers hold, for each group of `headers`, one of its markers, and
/// whose document holds, for each group of `markers`, one of its markers and
/// none of `unless`.
pub(super) struct Rule {
    pub(super) name: &'static str,
    pub(super) cause: BlockCause,
    pub(super) source: Source,
    pub(super) statuses: &'static [u16],
    pub(super) headers: &'static [&'static [HeaderMarker]],
    /// Lowercase text, matched in the lowercased markup.
    pub(super) markers: &'static [&'static [&'static str]],
    /// Lowercase text whose presence in the lowercased markup shows the page
    /// is no refusal, though it holds the markers.
    pub(super) unless: &'static [&'static str],
}

/// Where a rule comes from: Goodfetch itself, or an outside source it was
/// seeded from.
#[derive(Clone, Copy)]
pub(super) enum Source {
    /// Goodfetch's own rules, written from the Proving ground's
    /// block-detection corpus and the HTTP specifications.
    Goodfetch,
    /// An outside source, declared in `third-party.toml` for the notice
    /// inventory.
    Seeded(Outside),
}

/// An outside source a rule was seeded from.
#[derive(Clone, Copy)]
pub(super) struct Outside {
    /// Its input's name in `third-party.toml`.
    pub(super) name: &'static str,
    /// The SPDX license expression it is used under.
    pub(super) license: &'static str,
}

impl Source {
    /// Its name.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Goodfetch => "Goodfetch, from the Proving ground's block-detection corpus",
            Self::Seeded(outside) => outside.name,
        }
    }

    /// The SPDX license expression it is used under.
    pub(super) fn license(self) -> &'static str {
        match self {
            Self::Goodfetch => "MIT OR Apache-2.0",
            Self::Seeded(outside) => outside.license,
        }
    }
}

/// A header a rule reads: its lowercase name, and lowercase text its value
/// holds (empty for any value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeaderMarker {
    pub(crate) name: &'static str,
    pub(crate) value: &'static str,
}

/// Goodfetch's own rules.
const GOODFETCH: Source = Source::Goodfetch;

/// Spider's anti-bot markers (`spider/src/utils/mod.rs`).
const SPIDER: Source = Source::Seeded(Outside {
    name: "Spider",
    license: "MIT",
});

/// wafw00f's plugins (`wafw00f/plugins/`).
const WAFW00F: Source = Source::Seeded(Outside {
    name: "wafw00f",
    license: "BSD-3-Clause",
});

/// identYwaf's blocking-page expressions (`data.json`).
const IDENTYWAF: Source = Source::Seeded(Outside {
    name: "identYwaf",
    license: "MIT",
});

/// Nuclei's `waf-detect` template.
const NUCLEI_WAF_DETECT: Source = Source::Seeded(Outside {
    name: "Nuclei's waf-detect template",
    license: "MIT",
});

/// Every rule, in the order they are tried: challenges first, so a challenge
/// served with a refusal's status is a challenge, then captchas, refusals of
/// the address and the rate limit.
pub(super) const RULES: &[Rule] = &[
    // Cloudflare marks a response it answered with a challenge with
    // `cf-mitigated: challenge` (Spider reads the header); its mere `cf-ray`
    // on every response is no evidence.
    Rule {
        name: "cloudflare_challenge",
        cause: BlockCause::Challenge,
        source: SPIDER,
        statuses: &[403, 429, 503],
        headers: &[&[HeaderMarker {
            name: "cf-mitigated",
            value: "challenge",
        }]],
        markers: &[],
        unless: &[],
    },
    // A vendor-neutral interstitial challenge: a refusal whose page says it
    // checks the browser and needs JavaScript to let it through. A vendor's
    // script on an ordinary page holds neither.
    Rule {
        name: "interstitial_challenge",
        cause: BlockCause::Challenge,
        source: GOODFETCH,
        statuses: &[403, 429, 503],
        headers: &[],
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
        unless: &[],
    },
    // A press-and-hold human check on a refusal (Spider's markers: the
    // denial's title and the widget's element or script).
    Rule {
        name: "press_and_hold",
        cause: BlockCause::Captcha,
        source: SPIDER,
        statuses: &[403],
        headers: &[],
        markers: &[
            &["access to this page has been denied"],
            &["px-captcha", "/captcha/captcha.js"],
        ],
        unless: &[],
    },
    // A page that asks the visitor to prove they are human and holds a
    // captcha widget (Spider's generic phrases and its widget markers). A
    // widget alone, on a contact form, is no block, and neither is a login
    // or sign-up form whose widget says the same: the page is served, and
    // the check guards only the form's submission, which the program meets
    // when it submits.
    Rule {
        name: "captcha_widget",
        cause: BlockCause::Captcha,
        source: SPIDER,
        statuses: &[200, 403, 405, 429, 503],
        headers: &[],
        markers: &[
            &[
                "verify you are human",
                "verify you are a human",
                "prove you're not a robot",
            ],
            &["g-recaptcha", "h-captcha", "cf-turnstile", "funcaptcha"],
        ],
        unless: &["type=\"password\"", "type='password'", "type=password"],
    },
    // The three firewall refusals below (Cloudflare's, Imperva's and
    // Sucuri's) are `ip_block`, though a firewall refuses by its rules (the
    // user agent, the path, the country) as well as by address: ADR 0035's
    // closed list has no cause for a refusal by rule, and a firewall's
    // refusal page names no rule, so the evidence is a refusal of this
    // client where it stands, whose remedy is the IP block's (another
    // address or identity, or a pause), never the challenge's or the rate
    // limit's. Whether a firewall's refusal earns a cause of its own is the
    // owner's to decide (review of specs #598 and #606, L9).
    //
    // Cloudflare's block page (identYwaf's expression for its title), with
    // the words of a block rather than of its older captcha page.
    Rule {
        name: "cloudflare_block",
        cause: BlockCause::IpBlock,
        source: IDENTYWAF,
        statuses: &[403],
        headers: &[],
        markers: &[
            &["attention required! | cloudflare"],
            &["you have been blocked"],
        ],
        unless: &[],
    },
    // Imperva's refusal (wafw00f's `incapsula incident id`).
    Rule {
        name: "incapsula_block",
        cause: BlockCause::IpBlock,
        source: WAFW00F,
        statuses: &[403],
        headers: &[],
        markers: &[&["request unsuccessful"], &["incapsula incident id"]],
        unless: &[],
    },
    // Sucuri's refusal (the template's expressions for its title).
    Rule {
        name: "sucuri_block",
        cause: BlockCause::IpBlock,
        source: NUCLEI_WAF_DETECT,
        statuses: &[403],
        headers: &[],
        markers: &[&[
            "access denied - sucuri website firewall",
            "sucuri website firewall - cloudproxy - access denied",
            "sucuri website firewall - access denied",
        ]],
        unless: &[],
    },
    // A refusal that says the visitor's address is blocked.
    Rule {
        name: "ip_block_notice",
        cause: BlockCause::IpBlock,
        source: GOODFETCH,
        statuses: &[403],
        headers: &[],
        markers: &[&[
            "your ip address has been blocked",
            "your ip has been blocked",
            "your ip address is blocked",
        ]],
        unless: &[],
    },
    // 429 Too Many Requests is the site's statement of a rate limit (RFC
    // 6585), whether or not it says when to come back.
    Rule {
        name: "too_many_requests",
        cause: BlockCause::RateLimit,
        source: GOODFETCH,
        statuses: &[429],
        headers: &[],
        markers: &[],
        unless: &[],
    },
];
