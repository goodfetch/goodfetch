//! The egress policy (ADRs 0014, 0017, 0100 and 0120): whether a request
//! of the run may leave, judged before any lookup or connection. Its
//! interface is [`judge`], which answers [`Judgment::Allowed`] with the URL
//! as the WHATWG parser leaves it, or [`Judgment::Refused`] with the rule
//! and the setting that would allow it. It fails closed: a URL that does
//! not parse, a scheme no request may use and a kind of request the caller
//! cannot see are refused.
//!
//! The domain policy is `egress.allowed_domains` and
//! `egress.blocked_domains`. An entry matches its host and their
//! subdomains, as `agent.domains` does (ADR 0100); `*` matches every host,
//! only when written; a blocked entry wins. Hosts and entries alike are
//! read as the WHATWG URL parser reads a host, so `2130706433`,
//! `0x7f000001` and `127.0.0.1` are one address, and `bücher.test` and
//! `xn--bcher-kva.test` one name.
//!
//! The resolved-address check is [`judge_addresses`], on the addresses a
//! connection may try once a request is allowed: every answer must be one
//! a page may reach ([`classify`]), or lie in a range the Session lists
//! (`egress.private_ranges`), or be an answer of a host it lists by name,
//! a cloud-metadata address only when listed by its exact address. With
//! `security.resolved_ip_check` off, the Run's switch (ADR 0016), it
//! judges nothing and the domain policy alone applies. A URL whose host
//! carries a zone ID is refused as a private address by [`judge`] itself,
//! since the URL parser reads no host from it.
//!
//! Operator-configured endpoints are the second egress class (ADR 0017),
//! judged by [`judge_endpoint`] and [`judge_endpoint_addresses`] under an
//! [`EndpointPolicy`]: no run's domain policy applies, and an endpoint
//! reaches private addresses, so a model on localhost works, unless
//! `egress.no_private_endpoints`, set in the Operator policy alone, holds
//! it to globally reachable ones.

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use goodfetch_protocol::egress::{
    EGRESS_ALLOWED_DOMAINS, EGRESS_BLOCKED_DOMAINS, EGRESS_NO_PRIVATE_ENDPOINTS,
    EGRESS_PRIVATE_RANGES, EGRESS_REFUSED, EVERY_HOST, EgressRefusedDetails, EgressRule,
    PrivateRange,
};
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::security::SECURITY_RESOLVED_IP_CHECK;
use serde_json::{Map, Value};
use url::{Host, Url};

use crate::call_error::CallError;
use crate::settings::{HostEntry, Resolved, without_root};

mod address;
mod cdp_url;
mod proxy_script;

pub use address::{AddressClass, classify, classify_socket};
pub use cdp_url::{
    BrowserLocation, CdpUrlOrigin, CdpUrlRefused, cdp_url_address, judge_cdp_url, judge_unresolved,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod address_classification {
    goodfetch_test_support::fuzz_replay!(
        crate::egress::fuzz_address_classification,
        "fuzz/address_classification"
    );
}

/// A Session's egress policy: its domain policy, which [`judge`] applies,
/// and the private addresses [`judge_addresses`] lets it reach.
#[derive(Clone, Debug)]
pub struct Policy {
    allowed: Entries,
    blocked: Entries,
    private_ranges: Vec<Block>,
    private_hosts: HashSet<Box<str>, FixedKeys>,
    resolved_ip_check: bool,
}

/// A range of private addresses a page may reach.
#[derive(Clone, Copy, Debug)]
struct Block {
    first: IpAddr,
    prefix: u8,
}

/// One domain list, each entry as the URL parser reads a host.
#[derive(Clone, Debug, Default)]
struct Entries {
    every_host: bool,
    domains: HashSet<Box<str>, FixedKeys>,
    addresses: HashSet<IpAddr, FixedKeys>,
}

/// The hasher of a domain list's sets, with the same keys in every process.
/// The default one takes keys from the OS in each process, so the work of a
/// lookup changed from one process to the next (randomness outside the
/// Entropy handle, ADR 0059). Its entries come from the Session's settings
/// and a lookup inserts nothing, so known keys let no page lengthen a probe.
type FixedKeys = BuildHasherDefault<DefaultHasher>;

/// An entry of a domain list that is no host.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{entry}` in `{setting}` is no host: write a host name, an IP address or `*`")]
pub struct InvalidEntry {
    /// The setting that holds it.
    pub setting: &'static str,
    /// The entry, as written.
    pub entry: String,
}

impl Policy {
    /// The policy allowing `allowed` and blocking `blocked`, the values of
    /// `egress.allowed_domains` and `egress.blocked_domains`.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] for the first entry that is no host, such as one
    /// with a path, a port or a `*` label.
    pub fn new(
        allowed: &[impl AsRef<str>],
        blocked: &[impl AsRef<str>],
    ) -> Result<Self, InvalidEntry> {
        Ok(Self {
            allowed: Entries::read(allowed, EGRESS_ALLOWED_DOMAINS.name)?,
            blocked: Entries::read(blocked, EGRESS_BLOCKED_DOMAINS.name)?,
            private_ranges: Vec::new(),
            private_hosts: HashSet::default(),
            resolved_ip_check: true,
        })
    }

    /// The policy a Run's resolved settings give: its domain policy, its
    /// `egress.private_ranges` and its `security.resolved_ip_check`.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] as [`Policy::new`] and
    /// [`Policy::with_private_ranges`].
    pub fn of(settings: &Resolved) -> Result<Self, InvalidEntry> {
        let list = |key: &str| -> Vec<String> {
            settings
                .value(key)
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        };
        let mut policy = Self::new(
            &list(EGRESS_ALLOWED_DOMAINS.name),
            &list(EGRESS_BLOCKED_DOMAINS.name),
        )?
        .with_private_ranges(&list(EGRESS_PRIVATE_RANGES.name))?;
        policy.resolved_ip_check = settings
            .boolean(SECURITY_RESOLVED_IP_CHECK.name)
            .unwrap_or(true);
        Ok(policy)
    }
}

impl Entries {
    fn read(entries: &[impl AsRef<str>], setting: &'static str) -> Result<Self, InvalidEntry> {
        let mut read = Self::default();
        for entry in entries {
            let entry = entry.as_ref();
            match HostEntry::read(entry).ok_or_else(|| InvalidEntry {
                setting,
                entry: entry.to_owned(),
            })? {
                HostEntry::Every => read.every_host = true,
                HostEntry::Domain(domain) => {
                    read.domains.insert(domain.into());
                }
                HostEntry::Address(address) => {
                    read.addresses.insert(address);
                }
            }
        }
        Ok(read)
    }

    /// Whether an entry matches `host`: the host itself, or a domain it is
    /// a subdomain of.
    fn matches(&self, host: &Host<&str>) -> bool {
        if self.every_host {
            return true;
        }
        match host {
            Host::Domain(domain) => {
                let mut name = without_root(domain);
                loop {
                    if self.domains.contains(name) {
                        return true;
                    }
                    match name.split_once('.') {
                        Some((_, parent)) => name = parent,
                        None => return false,
                    }
                }
            }
            Host::Ipv4(address) => self.addresses.contains(&IpAddr::V4(*address)),
            Host::Ipv6(address) => self.addresses.contains(&IpAddr::V6(*address)),
        }
    }
}

/// The kind of a request, as the code that sends it sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// A request the engine's own HTTP client sends for the run.
    Fetch,
    /// A request of a page that the page's interception holds, a route's
    /// overrides applied (spec #577's step 3).
    Page,
    /// A hop of a page's main-frame navigation that a Client asked for,
    /// while that call goes on: [`Navigator::reads_files`] decides whether
    /// it may load a `file:` URL.
    Navigation(Navigator),
    /// A request whose kind the interception that holds it cannot see.
    Unseen,
}

/// Who asked for a page's navigation, as the `file:` rule reads it (ADRs
/// 0056 and 0089; spec #564): a `file:` URL, or any other scheme but
/// HTTP's, is loaded only for the program's own code or the CLI on an
/// engine without accounts, so a model never reads the machine's files, or
/// the browser's own pages, through a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Navigator {
    /// A Client acting for a model asked.
    pub acting_for_model: bool,
    /// The page is a Recipe's Run's.
    pub recipe_run: bool,
    /// The engine serves accounts, so people other than the program's own.
    pub accounts: bool,
}

impl Navigator {
    /// Whether its navigation may load a `file:` URL, or any other
    /// document of the machine or the browser that reaches no network.
    pub fn reads_files(self) -> bool {
        !(self.acting_for_model || self.recipe_run || self.accounts)
    }
}

/// Where a navigation a Client asked for goes, once its scheme is judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    /// To the network, whose every hop the page's interception judges.
    Network,
    /// To a document of the machine or the browser that reaches no
    /// network, a `file:` URL among them, which its [`Navigator`] may load.
    Local,
    /// A `javascript:` URL, which runs in the frame if
    /// [`judge_javascript`] allows it.
    JavaScript,
    /// `about:blank`, which loads nothing.
    Blank,
}

/// Judges the scheme of `url`, a navigation `navigator` asked for, before
/// any route sees it.
///
/// # Errors
///
/// [`Refused`] with [`EgressRule::Scheme`]; with
/// [`EgressRule::PrivateAddress`] for an IPv6 address with a zone ID, and
/// [`EgressRule::DomainPolicy`] for any other URL that does not parse.
pub fn navigation(url: &str, navigator: Navigator) -> Result<Destination, Refused> {
    let refused = |rule| Refused {
        url: url.to_owned(),
        rule,
        address: None,
        setting: None,
    };
    let Ok(parsed) = Url::parse(url) else {
        // A zone names a link, as `judge` has it (ADR 0067).
        if let Some(address) = zone_id_address(url) {
            return Err(Refused {
                address: Some(IpAddr::V6(address)),
                ..refused(EgressRule::PrivateAddress)
            });
        }
        return Err(refused(EgressRule::DomainPolicy));
    };
    match parsed.scheme() {
        scheme if SCHEMES.contains(&scheme) => Ok(Destination::Network),
        JAVASCRIPT => Ok(Destination::JavaScript),
        _ if parsed.as_str() == BLANK => Ok(Destination::Blank),
        _ if navigator.reads_files() => Ok(Destination::Local),
        _ => Err(refused(EgressRule::Scheme)),
    }
}

/// The scheme of a URL to a file of the engine's machine.
const FILE: &str = "file";

/// The scheme of a URL that runs a script in a frame.
const JAVASCRIPT: &str = "javascript";

/// The page that loads nothing.
const BLANK: &str = "about:blank";

/// Judges whether the `javascript:` URL `url` may run in a frame whose
/// document is at `frame_url`.
///
/// # Errors
///
/// [`Refused`] naming `url`.
pub fn judge_javascript(url: &str, frame_url: &str, session: &Policy) -> Result<(), Refused> {
    let frame = Request {
        url: frame_url,
        kind: RequestKind::Page,
    };
    match judge(&frame, session) {
        Judgment::Allowed(_) => Ok(()),
        Judgment::Refused(refused) => Err(Refused {
            url: url.to_owned(),
            ..refused
        }),
    }
}

/// A request to judge.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Its URL, as the run gave it.
    pub url: &'a str,
    /// Its kind.
    pub kind: RequestKind,
}

/// What [`judge`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Judgment {
    /// The request may leave, to this URL as the parser leaves it.
    Allowed(Url),
    /// The request may not.
    Refused(Refused),
}

/// A refusal: the request, the rule and what would allow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The request's URL, as the run gave it.
    pub url: String,
    /// The rule that refused it.
    pub rule: EgressRule,
    /// The address the rule judged, when it judged one.
    pub address: Option<IpAddr>,
    /// The setting whose value would allow it, when one would.
    pub setting: Option<&'static str>,
}

impl Refused {
    /// What `egress_refused` details.
    pub fn details(&self) -> EgressRefusedDetails {
        EgressRefusedDetails {
            url: self.url.clone(),
            rule: self.rule,
            address: self.address.map(|address| address.to_string()),
            setting: self.setting.map(str::to_owned),
        }
    }

    /// The failure the call that waited for the request answers with.
    pub fn error(&self) -> CallError {
        CallError::new(&EGRESS_REFUSED, self.to_string(), &self.details())
    }

    /// The failure as an event carries it, for a request no call waited
    /// for: `egress_refused` and its details.
    pub fn failure(&self) -> ErrorData {
        let details = match serde_json::to_value(self.details()) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        ErrorData {
            code: EGRESS_REFUSED.name.to_owned(),
            retryable: EGRESS_REFUSED.retryable,
            details,
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the egress policy refused a request to {} ({})",
            self.url,
            self.rule.as_str()
        )?;
        if let Some(setting) = self.setting {
            write!(formatter, "; `{setting}` would allow it")?;
        }
        Ok(())
    }
}

impl Policy {
    /// The policy that also lets a page reach `ranges`, the value of
    /// `egress.private_ranges`: each a CIDR block or one address, IPv4 or
    /// IPv6, or a host name whose connections may use any address that is
    /// not globally reachable. A cloud-metadata address is reached only
    /// through an entry that is that address alone, never through a range
    /// or a host name that covers it.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] for the first entry that is none of them.
    pub fn with_private_ranges(mut self, ranges: &[impl AsRef<str>]) -> Result<Self, InvalidEntry> {
        for entry in ranges {
            let entry = entry.as_ref();
            match PrivateRange::parse(entry) {
                Ok(PrivateRange::Block { first, prefix }) => {
                    self.private_ranges.push(Block { first, prefix });
                }
                Ok(PrivateRange::Host(host)) => {
                    self.private_hosts.insert(host.into());
                }
                Err(_) => {
                    return Err(InvalidEntry {
                        setting: PRIVATE_RANGES,
                        entry: entry.to_owned(),
                    });
                }
            }
        }
        Ok(self)
    }

    /// Whether an entry lets a page reach `address`, whose class is
    /// `class`, on a connection for `url`.
    fn lists(&self, url: &str, address: IpAddr, class: AddressClass) -> bool {
        let in_a_block = self.private_ranges.iter().any(|range| {
            range.contains(address) && (class != AddressClass::Metadata || range.is_one(address))
        });
        in_a_block || (class != AddressClass::Metadata && self.lists_host_of(url))
    }

    /// Whether `url`'s host is one `egress.private_ranges` names.
    fn lists_host_of(&self, url: &str) -> bool {
        if self.private_hosts.is_empty() {
            return false;
        }
        match Url::parse(url).ok().as_ref().and_then(Url::host) {
            Some(Host::Domain(domain)) => self
                .private_hosts
                .contains(without_root(&domain.to_ascii_lowercase())),
            _ => false,
        }
    }
}

impl Block {
    fn contains(self, address: IpAddr) -> bool {
        match (self.first, address) {
            (IpAddr::V4(first), IpAddr::V4(address)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                first.to_bits() & mask == address.to_bits() & mask
            }
            (IpAddr::V6(first), IpAddr::V6(address)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                first.to_bits() & mask == address.to_bits() & mask
            }
            _ => false,
        }
    }

    /// Whether the range is `address` alone.
    fn is_one(self, address: IpAddr) -> bool {
        self.first == address && self.prefix == if address.is_ipv4() { 32 } else { 128 }
    }
}

/// The setting that lists the private ranges a page may reach.
pub const PRIVATE_RANGES: &str = EGRESS_PRIVATE_RANGES.name;

/// Judges the addresses a connection for `url` may try, every answer its
/// host has, before it tries any (ADR 0017): one a page may not reach
/// refuses the connection, so a name with a public and a private answer is
/// reached through neither (ADR 0067). A connection judged here tries only
/// these addresses, so an answer that changes after the judgment
/// (rebinding) reaches nothing.
///
/// # Errors
///
/// [`Refused`] with [`EgressRule::PrivateAddress`] and the first address a
/// page may not reach, naming `egress.private_ranges` unless no range could
/// allow it (an address with a zone ID). None while the Session's
/// `security.resolved_ip_check` is off.
pub fn judge_addresses(
    url: &str,
    addresses: &[SocketAddr],
    session: &Policy,
) -> Result<(), Refused> {
    if !session.resolved_ip_check {
        return Ok(());
    }
    judge_reachable(url, addresses, session)
}

/// [`judge_addresses`] whatever the switch says, for the rules that are
/// not the run's resolved-address check, such as ADR 0056's for a
/// `cdp_url`.
fn judge_reachable(url: &str, addresses: &[SocketAddr], session: &Policy) -> Result<(), Refused> {
    for address in addresses {
        let class = classify_socket(*address);
        let listed = class != AddressClass::ZoneId && session.lists(url, address.ip(), class);
        if class.is_global() || listed {
            continue;
        }
        return Err(Refused {
            url: url.to_owned(),
            rule: EgressRule::PrivateAddress,
            address: Some(address.ip()),
            setting: (class != AddressClass::ZoneId).then_some(PRIVATE_RANGES),
        });
    }
    Ok(())
}

/// What the Operator policy lets an Operator-configured endpoint reach
/// (ADR 0017), the second egress class: no run's domain policy or private
/// ranges apply to it, and it reaches private addresses, a model on
/// localhost among them, unless `egress.no_private_endpoints` holds it to
/// globally reachable ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndpointPolicy {
    no_private_endpoints: bool,
}

impl EndpointPolicy {
    /// The policy that refuses every address that is not globally
    /// reachable when `no_private_endpoints`.
    pub fn new(no_private_endpoints: bool) -> Self {
        Self {
            no_private_endpoints,
        }
    }

    /// Whether it refuses every address that is not globally reachable:
    /// `egress.no_private_endpoints` is on.
    pub fn no_private_endpoints(self) -> bool {
        self.no_private_endpoints
    }

    /// The policy the resolved settings give: `egress.no_private_endpoints`,
    /// which only the Operator policy sets.
    pub fn of(settings: &Resolved) -> Self {
        Self::new(
            settings
                .boolean(EGRESS_NO_PRIVATE_ENDPOINTS.name)
                .unwrap_or(false),
        )
    }
}

/// Judges the URL of an Operator-configured endpoint before any lookup:
/// its scheme alone, one of HTTP's, since no run's domain policy applies
/// to it; [`judge_endpoint_addresses`] then judges where it resolves.
pub fn judge_endpoint(url: &str) -> Judgment {
    let refused = |rule| {
        Judgment::Refused(Refused {
            url: url.to_owned(),
            rule,
            address: None,
            setting: None,
        })
    };
    match Url::parse(url) {
        Ok(parsed) if SCHEMES.contains(&parsed.scheme()) => Judgment::Allowed(parsed),
        Ok(_) => refused(EgressRule::Scheme),
        Err(_) => refused(EgressRule::DomainPolicy),
    }
}

/// Judges the addresses a connection to the Operator-configured endpoint
/// at `url` may try, before it tries any.
///
/// # Errors
///
/// [`Refused`] with [`EgressRule::PrivateAddress`], the first address that
/// is not globally reachable and `egress.no_private_endpoints`, when it
/// is on.
pub fn judge_endpoint_addresses(
    url: &str,
    addresses: &[SocketAddr],
    policy: EndpointPolicy,
) -> Result<(), Refused> {
    if !policy.no_private_endpoints {
        return Ok(());
    }
    match addresses
        .iter()
        .find(|address| !classify_socket(**address).is_global())
    {
        Some(address) => Err(Refused {
            url: url.to_owned(),
            rule: EgressRule::PrivateAddress,
            address: Some(address.ip()),
            setting: Some(EGRESS_NO_PRIVATE_ENDPOINTS.name),
        }),
        None => Ok(()),
    }
}

/// The schemes a request may use; every other is refused (ADR 0017).
const SCHEMES: [&str; 4] = ["http", "https", "ws", "wss"];

/// Judges `request` under `session`'s policy, before any lookup.
pub fn judge(request: &Request<'_>, session: &Policy) -> Judgment {
    let refused = |rule, setting| {
        Judgment::Refused(Refused {
            url: request.url.to_owned(),
            rule,
            address: None,
            setting,
        })
    };
    if request.kind == RequestKind::Unseen {
        return refused(EgressRule::DomainPolicy, None);
    }
    let Ok(url) = Url::parse(request.url) else {
        if let Some(address) = zone_id_address(request.url) {
            return Judgment::Refused(Refused {
                url: request.url.to_owned(),
                rule: EgressRule::PrivateAddress,
                address: Some(IpAddr::V6(address)),
                setting: None,
            });
        }
        return refused(EgressRule::DomainPolicy, None);
    };
    if url.scheme() == FILE
        && let RequestKind::Navigation(navigator) = request.kind
        && navigator.reads_files()
    {
        return Judgment::Allowed(url);
    }
    if !SCHEMES.contains(&url.scheme()) {
        return refused(EgressRule::Scheme, None);
    }
    let Some(host) = url.host() else {
        return refused(EgressRule::DomainPolicy, None);
    };
    if session.blocked.matches(&host) {
        return refused(EgressRule::DomainPolicy, Some(EGRESS_BLOCKED_DOMAINS.name));
    }
    if !session.allowed.matches(&host) {
        return refused(EgressRule::DomainPolicy, Some(EGRESS_ALLOWED_DOMAINS.name));
    }
    Judgment::Allowed(url)
}

/// The address of a URL whose host is an IPv6 address with a zone ID, such
/// as `http://[fe80::1%25eth0]/`, which the WHATWG parser refuses: a zone
/// names a link, so the request is to a private address (ADR 0067).
fn zone_id_address(url: &str) -> Option<Ipv6Addr> {
    let (_, rest) = url.split_once("://")?;
    let host = rest.strip_prefix('[')?.split_once(']')?.0;
    let (address, zone) = host.split_once('%')?;
    if zone.is_empty() {
        return None;
    }
    address.parse().ok()
}

/// The fuzz target of the resolved-address classification (ADR 0067):
/// `data` read as an IPv4 address (its first 4 bytes), an IPv6 address (its
/// first 16) with a zone ID (the next 4), and as text, an address or a host
/// in a URL. The classification never panics; an IPv4-mapped address
/// classifies as the address it embeds; a zone ID is refused; and judging
/// a URL with the text as its host, then the address it names, never
/// panics.
pub fn fuzz_address_classification(data: &[u8]) {
    if let Some(octets) = data.first_chunk::<4>() {
        let address = std::net::Ipv4Addr::from(*octets);
        assert_eq!(
            classify(IpAddr::V6(address.to_ipv6_mapped())),
            classify(IpAddr::V4(address)),
            "an IPv4-mapped address classifies as another than the address it embeds"
        );
    }
    if let Some(octets) = data.first_chunk::<16>() {
        let address = Ipv6Addr::from(*octets);
        let zone = data
            .get(16..20)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map_or(0, u32::from_be_bytes);
        let socket = std::net::SocketAddrV6::new(address, 0, 0, zone);
        let class = classify_socket(SocketAddr::V6(socket));
        if zone == 0 {
            assert_eq!(
                class,
                classify(IpAddr::V6(address)),
                "a socket address with no zone classifies as another than its address"
            );
        } else {
            assert_eq!(class, AddressClass::ZoneId, "a zone ID is not refused");
        }
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(address) = text.parse::<IpAddr>() {
        let _ = classify(address);
    }
    let every_host = Policy::new(&[EVERY_HOST], &[] as &[&str])
        .expect("invariant: `*` is an entry of a domain list");
    for url in [format!("http://{text}/"), format!("http://[{text}]/")] {
        let request = Request {
            url: &url,
            kind: RequestKind::Fetch,
        };
        let Judgment::Allowed(allowed) = judge(&request, &every_host) else {
            continue;
        };
        let address = match allowed.host() {
            Some(Host::Ipv4(address)) => IpAddr::V4(address),
            Some(Host::Ipv6(address)) => IpAddr::V6(address),
            _ => continue,
        };
        let _ = judge_addresses(&url, &[SocketAddr::new(address, 80)], &every_host);
    }
}
