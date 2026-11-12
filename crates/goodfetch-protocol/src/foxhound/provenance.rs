//! A Record's Provenance (ADRs 0045, 0047, 0052, 0103, 0119 and 0123):
//! what each committed Record carries of how it was obtained, so each value
//! can be traced to its page. It names the crawl request that produced it
//! (there is no push outside one, ADR 0103), the tier that fetched its
//! document and how that tier was chosen (ADR 0093), the encoding the
//! document was read in and where it came from (ADR 0119), a Response cache
//! or Response recording that answered in place of the network, the
//! Clean-up rules the Run holds to (ADR 0047), a Wall read past (ADR 0052),
//! a payment (ADR 0045), the invalid-certificate exception its document's
//! host was reached through (ADR 0042, pitfall H03), a robots.txt its fetch
//! ignored (ADR 0086), and each Value transform with its text, locale and
//! zone (ADR 0123). The Run trace keeps the rest of each page's Provenance,
//! which the request's ID reaches.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::foxhound::fetch::FetchTier;
use crate::run::PointInTime;

/// How one committed Record was obtained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordProvenance {
    /// The ID of the crawl request whose attempt produced it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// The tier that fetched that request's document, and how it was
    /// chosen; none where the handler was given its request alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tier: Option<TierProvenance>,
    /// The encoding the document was read in, and where it came from;
    /// none for a request with no document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub encoding: Option<EncodingProvenance>,
    /// The Response cache or Response recording that answered in place of
    /// the network; none when the network did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Option<ResponseOrigin>,
    /// The digest, `sha256:<hex>`, of the Clean-up rules the Run holds to,
    /// whose Run trace names their date and the release that applied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub clean_up_rules: Option<String>,
    /// The Wall its page was read past; none where no Wall was met.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wall: Option<WallMarker>,
    /// The payment of a Paid fetch of its page; none where nothing was
    /// paid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payment: Option<PaymentMark>,
    /// The exception of `tls.accept_invalid_certs` its document's host was
    /// reached through; none where its certificate verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificate_exception: Option<CertificateExceptionMark>,
    /// `ignored` when its page was fetched while
    /// `site_preferences.respect_robots_txt` was off and
    /// `site_preferences.mark_robots_txt_ignored` on, from a URL its
    /// origin's robots.txt disallows, so a result taken against a site's
    /// stated preferences can be found and removed; none otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub robots_txt: Option<RobotsTxtMark>,
    /// Each field whose value an engine Value transform gave, in the
    /// order read; empty where the program parsed its own values.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value_transforms: Vec<TransformProvenance>,
    /// Each field of a code-free crawl the page did not give, null in the
    /// Record, with why; absent where every field was read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unread_fields: Vec<UnreadField>,
}

/// A field of a code-free crawl's Record that its page did not give, so
/// the Record holds null (ADR 0123).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnreadField {
    /// The field, a JSON Pointer into the Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// Why.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: UnreadReason,
    /// The text read, where one was: the text its `match` did not match, or
    /// that its transform does not read as a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: Option<String>,
}

/// Why a field of a code-free crawl's Record is null.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnreadReason {
    /// Its locator found no element.
    NoElement,
    /// Its element has no such attribute.
    NoAttribute,
    /// Its `match` did not match the text, or its group took no part.
    NoMatch,
    /// Its transform does not read the text as a value.
    NotParsed,
    /// A `date_time` whose text states no offset, with no `time_zone`.
    NoTimeZone,
}

/// The tier that fetched a Record's document and how it was chosen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TierProvenance {
    /// The tier that sent the document's request, whoever chose it:
    /// `browser_fetch` when it left from inside the Session's browser, as
    /// under `auto`, and `http` when the Session's own HTTP transport sent
    /// it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tier: FetchTier,
    /// How it was chosen.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub chosen: TierChosen,
    /// What forced it, when `chosen` is `forced`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub forced_by: Option<ForcedBy>,
    /// The call a cheaper tier could not serve, which moved the request
    /// here, for an escalation or a sample's browser run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call: Option<String>,
}

/// How the tier of a Record's document was chosen (ADRs 0023 and 0093).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TierChosen {
    /// `fetch.tier` forced it.
    Forced,
    /// Under `auto`, the crawl's sample comparison chose it.
    Compared,
    /// It was the browser run of a sample of that comparison.
    Sample,
    /// A cheaper tier could not serve a call, and the request moved here.
    Escalation,
    /// It won a race of tiers (`fetch.tier_race`, ADR 0040).
    Race,
}

/// Which scope's `fetch.tier` forced a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForcedBy {
    /// The Run's setting.
    Run,
    /// The setting of the request's Label.
    Label,
}

/// The encoding a Record's document was read in, and where it came from
/// (ADR 0119).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EncodingProvenance {
    /// Its WHATWG name, such as `Shift_JIS`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub encoding: String,
    /// Where it came from; none where the browser read the document by a
    /// rule the engine did not see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: Option<EncodingSource>,
}

/// Where a document's encoding came from, in the order the tiers look
/// (ADR 0119).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EncodingSource {
    /// Its byte order mark.
    Bom,
    /// `fetch.encoding`.
    Setting,
    /// The `Content-Type` charset.
    Header,
    /// A declaration in its own bytes: a `<meta>` or an XML declaration.
    Meta,
    /// Nothing declared, and the body is valid UTF-8.
    Utf8,
    /// The browser's guess, after the request escalated to it.
    Detected,
}

/// What answered a Record's request in place of the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseOrigin {
    /// The Response cache (ADR 0046).
    ResponseCache {
        /// When the cached response was fetched.
        #[schemars(extend("arrived_in" = "0.1"))]
        fetched_at: PointInTime,
    },
    /// A Response recording (ADR 0105).
    ResponseRecording {
        /// When the recorded response was received.
        #[schemars(extend("arrived_in" = "0.1"))]
        recorded_at: PointInTime,
    },
}

/// The `wall` marker of a Record whose page was read past a Wall (ADR
/// 0052), which Dataset queries and every Export format filter on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WallMarker {
    /// The Wall's kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: WallKind,
    /// The vendor detected, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub vendor: Option<String>,
}

/// A Wall's kind (ADR 0052).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WallKind {
    /// A paywall.
    Paywall,
    /// A consent wall.
    ConsentWall,
    /// An ad-blocker wall.
    AdBlockerWall,
}

/// What a Record's Provenance says of the robots.txt of its page's origin
/// (ADR 0086).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RobotsTxtMark {
    /// It disallowed the page's URL, and the fetch ignored it, since
    /// `site_preferences.respect_robots_txt` was off; marked only while
    /// `site_preferences.mark_robots_txt_ignored` is on.
    Ignored,
}

/// The payment of a Paid fetch, as a Record's Provenance marks it (ADR
/// 0045).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaymentMark {
    /// The payment scheme the site asked through.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scheme: String,
    /// The amount charged, an exact decimal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: String,
    /// Its currency, an ISO 4217 code.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: String,
    /// The domain paid.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payee_domain: String,
}

/// The invalid-certificate exception a Record's document was fetched
/// under (ADR 0042, pitfall H03).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertificateExceptionMark {
    /// The kind of exception.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: CertificateExceptionKind,
    /// The SHA-256 of the public key of the certificate it accepted, as
    /// `sha256/<base64>`, the known-certificates store's spelling.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub spki_sha256: String,
}

/// The kind of an invalid-certificate exception (ADR 0042).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CertificateExceptionKind {
    /// Its domain was listed without a pin: the key pinned on first use.
    TrustOnFirstUse,
    /// Its domain was listed with `any`, and the host has no pin.
    Any,
}

/// How a Value transform gave one field of a Record (ADR 0123).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TransformProvenance {
    /// The field, a JSON Pointer into the Record.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// The page text exactly as read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The transform.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub transform: ValueTransform,
    /// Its resolved locale, a BCP 47 tag, for a locale-dependent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locale: Option<String>,
    /// Its resolved time zone, an IANA name, for a date-time or a
    /// relative time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: Option<String>,
    /// The stated CLDR pattern, such as `dd/MM/yyyy`, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: Option<String>,
    /// A relative time's anchor: the fetch time of the page's document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub anchor: Option<PointInTime>,
}

/// One of the engine's closed list of Value transforms (ADR 0123).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueTransform {
    /// The element's text, or one attribute.
    Text,
    /// A capture group of a regular expression over that text.
    Match,
    /// A number or an amount under its locale.
    Number,
    /// A currency symbol or code under its locale.
    Currency,
    /// A date under its locale or a stated pattern.
    Date,
    /// A date and time with its time zone.
    DateTime,
    /// A relative time under its locale, anchored at the fetch time.
    RelativeTime,
    /// An exact text looked up in the transform's table.
    Map,
}
