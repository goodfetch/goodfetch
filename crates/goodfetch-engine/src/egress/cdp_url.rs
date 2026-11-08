//! ADR 0056's address rules for a `cdp_url` Session source (ADR 0115),
//! judged on every address it resolves to before any connection: one set
//! in Operator policy or user configuration, or in project configuration
//! a Client acting for no model sends, is an Operator-configured endpoint,
//! which reaches private addresses unless `egress.no_private_endpoints`
//! refuses them; one from the program's own code or the CLI may also use
//! loopback; any other, a Client acting for a model's (its project
//! configuration's included, O8 of the deep review) or a Recipe's, is an
//! address a page can reach. A refusal raises `cdp_url_not_allowed`. ADR 0056's Exposed Daemon row, where even the
//! program's own `cdp_url` may not use loopback, comes with the Exposed
//! Daemon (A04-56).

use std::net::{IpAddr, SocketAddr};

use goodfetch_protocol::egress::{
    CDP_URL_NOT_ALLOWED, CdpUrlNotAllowedDetails, CdpUrlRule, EgressRule,
};
use goodfetch_protocol::settings::{NETWORK_RESOLVE, SettingsLayer};
use url::Url;

use super::{
    ConfiguredKind, EgressClass, EndpointPolicy, Policy, judge_endpoint_addresses, judge_reachable,
};
use crate::call_error::CallError;

/// What `cdp_url_not_allowed` asks for: the layers whose value is
/// Operator-configured for every Client, a model's included.
const FIX: &str =
    "set the address in Operator policy or in user configuration, or give a public address";

/// Who gave a `cdp_url`, as ADR 0056's address rules read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdpUrlOrigin {
    /// The settings layer that gave it.
    pub layer: SettingsLayer,
    /// Whether a Client acting for a model gave it, or opens the Session
    /// it is the source of.
    pub acting_for_model: bool,
}

/// How the address rules treat a `cdp_url`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Treated {
    /// As an Operator-configured endpoint.
    Configured,
    /// As an address a page can reach, loopback aside: the program's own.
    ProgramsOwn,
    /// As an address a page can reach.
    PageReachable,
}

impl CdpUrlOrigin {
    fn treated(self) -> Treated {
        match ConfiguredKind::CdpUrl.classed().class(self.layer) {
            // A Client acting for a model sends the project configuration
            // itself, so its value there is the model's (O8).
            EgressClass::OperatorConfigured
                if self.layer == SettingsLayer::ProjectConfiguration && self.acting_for_model =>
            {
                Treated::PageReachable
            }
            EgressClass::OperatorConfigured => Treated::Configured,
            EgressClass::PageReachable
                if self.layer == SettingsLayer::Run && !self.acting_for_model =>
            {
                Treated::ProgramsOwn
            }
            EgressClass::PageReachable => Treated::PageReachable,
        }
    }
}

/// A `cdp_url` the address rules refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdpUrlRefused {
    /// The `cdp_url`, as [`cdp_url_address`] names it.
    pub url: String,
    /// The layer that gave it.
    pub layer: SettingsLayer,
    /// The rule that refused it.
    pub rule: CdpUrlRule,
    /// The address it resolved to that the rule refused.
    pub address: IpAddr,
    /// The setting that refused it, or whose value would allow it.
    pub setting: Option<&'static str>,
}

impl CdpUrlRefused {
    /// What `cdp_url_not_allowed` details.
    pub fn details(&self) -> CdpUrlNotAllowedDetails {
        CdpUrlNotAllowedDetails {
            url: self.url.clone(),
            layer: self.layer,
            rule: self.rule,
            address: self.address.to_string(),
            setting: self.setting.map(str::to_owned),
            fix: FIX.to_owned(),
        }
    }

    /// `cdp_url_not_allowed`, which the call that opens the Session raises.
    pub fn error(&self) -> CallError {
        let rule = match self.rule {
            CdpUrlRule::Loopback => "loopback from a Client acting for a model or a Recipe",
            CdpUrlRule::PrivateAddress => "an address that is not globally reachable",
        };
        CallError::new(
            &CDP_URL_NOT_ALLOWED,
            format!(
                "the `cdp_url` {} from {} resolves to {}, {rule}, which ADR 0056's rules refuse: {FIX}",
                self.url,
                self.layer.as_str(),
                self.address
            ),
            &self.details(),
        )
    }
}

/// `url` as a `cdp_url`'s details and messages name it: without its query
/// string, fragment or credentials, which may hold a token (ADR 0115).
pub fn cdp_url_address(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            // A URL that cannot hold credentials holds none to remove.
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.into()
        }
        Err(_) => url.split(['?', '#']).next().unwrap_or_default().to_owned(),
    }
}

/// Judges `addresses`, every answer the host of `url` has, which `origin`
/// gave as a `cdp_url`, before any connection to it: under `endpoints`
/// when it is Operator-configured, else under the Session's `session`
/// policy as an address a page can reach, loopback allowed to the
/// program's own code.
///
/// # Errors
///
/// [`CdpUrlRefused`] naming the first address the rules refuse.
pub fn judge_cdp_url(
    url: &str,
    addresses: &[SocketAddr],
    origin: CdpUrlOrigin,
    session: &Policy,
    endpoints: EndpointPolicy,
) -> Result<(), CdpUrlRefused> {
    let refused = |rule, address, setting| CdpUrlRefused {
        url: cdp_url_address(url),
        layer: origin.layer,
        rule,
        address,
        setting,
    };
    let treated = origin.treated();
    let name = super::name_of(url);
    if treated == Treated::Configured {
        return judge_endpoint_addresses(url, addresses, endpoints).map_err(|endpoint| {
            refused(
                CdpUrlRule::PrivateAddress,
                endpoint.address.unwrap_or(IpAddr::from([0, 0, 0, 0])),
                endpoint.setting,
            )
        });
    }
    for address in addresses {
        if is_loopback(address.ip()) {
            if treated == Treated::ProgramsOwn {
                continue;
            }
            return Err(refused(CdpUrlRule::Loopback, address.ip(), None));
        }
        judge_reachable(url, name.as_deref(), std::slice::from_ref(address), session).map_err(
            |page| {
                refused(
                    CdpUrlRule::PrivateAddress,
                    page.address.unwrap_or(address.ip()),
                    page.setting,
                )
            },
        )?;
    }
    Ok(())
}

/// Where a Borrowed browser is, as ADR 0056's rule for a name the engine
/// cannot resolve reads it (O9 of the deep review).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserLocation {
    /// On the engine's machine: its `cdp_url` resolves to loopback alone,
    /// so it resolves names on the network the engine sees.
    EnginesMachine,
    /// Elsewhere, whose network may resolve names the engine's cannot,
    /// such as an intranet's or a cloud's metadata service's.
    Elsewhere,
}

impl BrowserLocation {
    /// Where the browser whose `cdp_url` resolves to `addresses` is: on
    /// the engine's machine only when every address is loopback.
    pub fn of(addresses: &[SocketAddr]) -> Self {
        if !addresses.is_empty() && addresses.iter().all(|address| is_loopback(address.ip())) {
            Self::EnginesMachine
        } else {
            Self::Elsewhere
        }
    }
}

/// Judges a request of a Borrowed browser at `browser` to `url`, whose
/// host the engine cannot resolve, so it judges no address of it: a
/// browser elsewhere may resolve the name on its own network, to an
/// intranet host or a cloud's metadata service, so the request is refused
/// there; one on the engine's machine resolves it as the engine would, and
/// gets it (ADR 0056's #633 note, amended by O9). With
/// `security.resolved_ip_check` off, the Run judges no address, and none
/// is refused.
///
/// # Errors
///
/// [`Refused`](super::Refused) with [`EgressRule::DomainPolicy`], a request
/// that could not be judged, naming `network.resolve`, which maps the name
/// to addresses the engine judges.
pub fn judge_unresolved(
    url: &str,
    session: &Policy,
    browser: BrowserLocation,
) -> Result<(), super::Refused> {
    if !session.resolved_ip_check || browser == BrowserLocation::EnginesMachine {
        return Ok(());
    }
    Err(super::Refused {
        url: url.to_owned(),
        rule: EgressRule::DomainPolicy,
        address: None,
        setting: Some(NETWORK_RESOLVE.name),
    })
}

/// Whether `address` is a loopback address, an IPv4-mapped one included.
fn is_loopback(address: IpAddr) -> bool {
    address.to_canonical().is_loopback()
}
