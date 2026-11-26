//! The population: each user agent of fingerprint-suite's fingerprint
//! network, weighted by its share of real traffic, with the values of each
//! property real browsers of that user agent presented and the consistency
//! rules keep. Read once from the embedded data; a user agent whose
//! property keeps no value is left out, so drawing from a candidate never
//! fails.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::archive::only_entry;
use crate::population::{Brand, Device, EngineFamily, Kind, Os, kind};
use crate::profile::{BrowserVersion, Screen, VideoCard};

static FINGERPRINT_NETWORK: &[u8] = include_bytes!("../data/fingerprint-network-definition.zip");
static HEADER_NETWORK: &[u8] = include_bytes!("../data/header-network-definition.zip");
static HEADER_ORDER: &str = include_str!("../data/headers-order.json");

/// The values a property takes, with their probabilities.
pub(crate) type Leaf<T> = Vec<(T, f64)>;

/// One user agent of the population and what may go with it.
pub(crate) struct Candidate {
    pub(crate) kind: Kind,
    pub(crate) user_agent: String,
    pub(crate) weight: f64,
    /// `None` on Firefox, which has none.
    pub(crate) client_hints: Leaf<Option<Hints>>,
    pub(crate) platform: Leaf<String>,
    pub(crate) vendor: Leaf<String>,
    pub(crate) product_sub: Leaf<String>,
    pub(crate) app_version: Leaf<String>,
    pub(crate) oscpu: Leaf<Option<String>>,
    pub(crate) hardware_concurrency: Leaf<u32>,
    pub(crate) device_memory: Leaf<Option<f64>>,
    pub(crate) max_touch_points: Leaf<u32>,
    pub(crate) screen: Leaf<Screen>,
    pub(crate) video_card: Leaf<VideoCard>,
    pub(crate) accept: Leaf<String>,
    pub(crate) accept_encoding: Leaf<String>,
    pub(crate) upgrade_insecure_requests: Leaf<bool>,
}

/// The client hints a Chromium user agent's browsers presented, but for
/// the brand lists, which the version writer derives.
#[derive(Clone)]
pub(crate) struct Hints {
    pub(crate) mobile: bool,
    pub(crate) platform: String,
    pub(crate) platform_version: String,
    pub(crate) architecture: String,
    pub(crate) bitness: String,
    pub(crate) model: String,
    pub(crate) full_version: BrowserVersion,
}

/// Every candidate of the population.
pub(crate) fn all() -> &'static [Candidate] {
    static ALL: OnceLock<Vec<Candidate>> = OnceLock::new();
    ALL.get_or_init(read)
}

/// The order a browser of `brand` sends its headers in over HTTP/2.
pub(crate) fn header_order(brand: Brand) -> &'static [String] {
    static ORDERS: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    let orders = ORDERS.get_or_init(|| {
        serde_json::from_str(HEADER_ORDER).expect("invariant: the embedded header order parses")
    });
    let browser = match brand {
        Brand::Chrome | Brand::AndroidChrome => "chrome",
        Brand::Edge => "edge",
        Brand::Firefox => "firefox",
    };
    let order = orders
        .get(browser)
        .expect("invariant: the embedded header order has each brand's");
    let after_pseudo_headers = order
        .iter()
        .rposition(|name| name.starts_with(':'))
        .map_or(0, |last| last + 1);
    &order[after_pseudo_headers..]
}

#[derive(Deserialize)]
struct Network {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    name: String,
    #[serde(rename = "conditionalProbabilities")]
    probabilities: Value,
}

/// One network's nodes, each with the probability of its values given its
/// parent's value.
type Nodes = BTreeMap<String, Value>;

fn nodes(archive: &[u8]) -> Nodes {
    let network: Network = serde_json::from_slice(&only_entry(archive))
        .expect("invariant: an embedded population network parses");
    network
        .nodes
        .into_iter()
        .map(|node| (node.name, node.probabilities))
        .collect()
}

fn read() -> Vec<Candidate> {
    let fingerprint = nodes(FINGERPRINT_NETWORK);
    let header = nodes(HEADER_NETWORK);
    fingerprint
        .get("userAgent")
        .and_then(Value::as_object)
        .expect("invariant: the fingerprint network starts from the user agent")
        .iter()
        .filter_map(|(user_agent, weight)| {
            candidate(&fingerprint, &header, user_agent, weight.as_f64()?)
        })
        .collect()
}

fn candidate(
    fingerprint: &Nodes,
    header: &Nodes,
    user_agent: &str,
    weight: f64,
) -> Option<Candidate> {
    let kind = kind(user_agent)?;
    let chromium = kind.brand.family() == EngineFamily::Chromium;
    let of = |nodes, node| given(nodes, node, user_agent);
    let candidate = Candidate {
        client_hints: keep(of(fingerprint, "userAgentData"), |raw| {
            client_hints(raw, kind)
        }),
        platform: keep(of(fingerprint, "platform"), |raw| {
            text(raw).filter(|platform| platforms(kind.os).contains(&platform.as_str()))
        }),
        vendor: keep(of(fingerprint, "vendor"), |raw| match (raw, chromium) {
            (Raw::Text(vendor @ "Google Inc."), true) => Some(vendor.to_owned()),
            (Raw::Missing, false) => Some(String::new()),
            _ => None,
        }),
        product_sub: keep(of(fingerprint, "productSub"), |raw| {
            text(raw).filter(|sub| sub == if chromium { "20030107" } else { "20100101" })
        }),
        app_version: keep(of(fingerprint, "appVersion"), text),
        oscpu: keep(of(fingerprint, "oscpu"), |raw| match (raw, chromium) {
            (Raw::Missing, true) => Some(None),
            (Raw::Text(oscpu), false) => Some(Some(oscpu.to_owned())),
            _ => None,
        }),
        hardware_concurrency: keep(of(fingerprint, "hardwareConcurrency"), |raw| {
            count(raw).filter(|&cores| cores > 0)
        }),
        device_memory: keep(of(fingerprint, "deviceMemory"), |raw| {
            match (raw, chromium) {
                (Raw::Json(memory), true) => memory.as_f64().filter(|&gb| gb > 0.0).map(Some),
                (Raw::Missing, false) => Some(None),
                _ => None,
            }
        }),
        max_touch_points: keep(of(fingerprint, "maxTouchPoints"), |raw| {
            count(raw).filter(|&points| match (kind.device, kind.os) {
                (Device::Mobile, _) => points > 0,
                (Device::Desktop, Os::Macos) => points == 0,
                (Device::Desktop, _) => true,
            })
        }),
        screen: keep(of(fingerprint, "screen"), screen),
        video_card: keep(of(fingerprint, "videoCard"), |raw| video_card(raw, kind)),
        accept: keep(of(header, "accept"), text),
        accept_encoding: keep(of(header, "accept-encoding"), text),
        upgrade_insecure_requests: keep(of(header, "upgrade-insecure-requests"), |raw| match raw {
            Raw::Text("1") => Some(true),
            Raw::Missing => Some(false),
            _ => None,
        }),
        kind,
        user_agent: user_agent.to_owned(),
        weight,
    };
    candidate.complete().then_some(candidate)
}

impl Candidate {
    fn complete(&self) -> bool {
        ![
            self.client_hints.is_empty(),
            self.platform.is_empty(),
            self.vendor.is_empty(),
            self.product_sub.is_empty(),
            self.app_version.is_empty(),
            self.oscpu.is_empty(),
            self.hardware_concurrency.is_empty(),
            self.device_memory.is_empty(),
            self.max_touch_points.is_empty(),
            self.screen.is_empty(),
            self.video_card.is_empty(),
            self.accept.is_empty(),
            self.accept_encoding.is_empty(),
            self.upgrade_insecure_requests.is_empty(),
        ]
        .contains(&true)
    }
}

/// A network value: absent, a JSON value, or plain text.
enum Raw<'a> {
    Missing,
    Json(Value),
    Text(&'a str),
}

/// The values `node` takes given its parent's `parent`, with their
/// probabilities.
fn given<'a>(nodes: &'a Nodes, node: &str, parent: &str) -> Vec<(Raw<'a>, f64)> {
    nodes
        .get(node)
        .and_then(|probabilities| probabilities.get("deeper")?.get(parent)?.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(value, probability)| {
            let raw = match value.as_str() {
                "*MISSING_VALUE*" => Raw::Missing,
                value => match value.strip_prefix("*STRINGIFIED*") {
                    Some(json) => Raw::Json(serde_json::from_str(json).ok()?),
                    None => Raw::Text(value),
                },
            };
            Some((raw, probability.as_f64()?))
        })
        .collect()
}

/// The values the rule `allowed` keeps, as what it makes of them.
fn keep<T>(values: Vec<(Raw<'_>, f64)>, allowed: impl Fn(Raw<'_>) -> Option<T>) -> Leaf<T> {
    values
        .into_iter()
        .filter_map(|(raw, probability)| Some((allowed(raw)?, probability)))
        .collect()
}

fn text(raw: Raw<'_>) -> Option<String> {
    match raw {
        Raw::Text(text) => Some(text.to_owned()),
        Raw::Missing | Raw::Json(_) => None,
    }
}

fn count(raw: Raw<'_>) -> Option<u32> {
    match raw {
        Raw::Json(number) => u32::try_from(number.as_u64()?).ok(),
        Raw::Missing | Raw::Text(_) => None,
    }
}

/// The `navigator.platform` values a browser on `os` shows.
fn platforms(os: Os) -> &'static [&'static str] {
    match os {
        Os::Windows => &["Win32"],
        Os::Macos => &["MacIntel"],
        Os::Linux => &[
            "Linux x86_64",
            "Linux aarch64",
            "Linux armv81",
            "Linux armv8l",
        ],
        Os::Android => &["Linux armv81", "Linux armv8l", "Linux aarch64"],
    }
}

/// The client hints' platform of a browser on `os`.
pub(crate) fn hint_platform(os: Os) -> &'static str {
    match os {
        Os::Windows => "Windows",
        Os::Macos => "macOS",
        Os::Linux => "Linux",
        Os::Android => "Android",
    }
}

/// Firefox's missing client hints, or Chromium's when they agree with the
/// user agent: its platform, whether it is mobile, whether Edge is among
/// the brands, and its major version.
fn client_hints(raw: Raw<'_>, kind: Kind) -> Option<Option<Hints>> {
    let data = match (raw, kind.brand.family()) {
        (Raw::Missing, EngineFamily::Firefox) => return Some(None),
        (Raw::Json(data), EngineFamily::Chromium) => data,
        _ => return None,
    };
    let field = |name: &str| Some(data.get(name)?.as_str()?.to_owned());
    let edge = data
        .get("brands")?
        .as_array()?
        .iter()
        .any(|brand| brand.get("brand").and_then(Value::as_str) == Some("Microsoft Edge"));
    let hints = Hints {
        mobile: data.get("mobile")?.as_bool()?,
        platform: field("platform")?,
        platform_version: field("platformVersion")?,
        architecture: field("architecture")?,
        bitness: field("bitness")?,
        model: field("model")?,
        full_version: BrowserVersion::parse(&field("uaFullVersion")?)?,
    };
    let agrees = hints.platform == hint_platform(kind.os)
        && hints.mobile == (kind.device == Device::Mobile)
        && edge == (kind.brand == Brand::Edge)
        && hints.full_version.major() == kind.major;
    agrees.then_some(Some(hints))
}

fn screen(raw: Raw<'_>) -> Option<Screen> {
    let Raw::Json(data) = raw else {
        return None;
    };
    let size = |name: &str| {
        u32::try_from(data.get(name)?.as_u64()?)
            .ok()
            .filter(|&size| size > 0)
    };
    let screen = Screen {
        width: size("width")?,
        height: size("height")?,
        avail_width: size("availWidth")?,
        avail_height: size("availHeight")?,
        color_depth: size("colorDepth")?,
        pixel_depth: size("pixelDepth")?,
        device_pixel_ratio: data
            .get("devicePixelRatio")?
            .as_f64()
            .filter(|&ratio| ratio > 0.0)?,
    };
    (screen.avail_width <= screen.width && screen.avail_height <= screen.height).then_some(screen)
}

/// A graphics card a browser on `kind`'s system reports: Direct3D on
/// Windows and only there, and Apple's under Chromium on macOS.
fn video_card(raw: Raw<'_>, kind: Kind) -> Option<VideoCard> {
    let Raw::Json(data) = raw else {
        return None;
    };
    let card = VideoCard {
        vendor: data.get("vendor")?.as_str()?.to_owned(),
        renderer: data.get("renderer")?.as_str()?.to_owned(),
    };
    let direct3d = card.renderer.contains("Direct3D");
    let apple = card.renderer.contains("Apple") || card.renderer.contains("Metal");
    let chromium_on_macos = kind.os == Os::Macos && kind.brand.family() == EngineFamily::Chromium;
    (direct3d == (kind.os == Os::Windows) && (apple || !chromium_on_macos)).then_some(card)
}
