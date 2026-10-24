//! ADR 0056's address rules for a `cdp_url` Session source (ADR 0115),
//! judged on every address it resolves to before any connection: one set
//! in Operator policy or user or project configuration is an
//! Operator-configured endpoint, which reaches private addresses unless
//! `egress.no_private_endpoints` refuses them; one from the program's own
//! code or the CLI may also use loopback; any other, a Client acting for a
//! model's or a Recipe's, is an address a page can reach. A refusal raises
//! `cdp_url_not_allowed`. ADR 0056's Exposed Daemon row, where even the
//! program's own `cdp_url` may not use loopback, comes with the Exposed
//! Daemon (A04-56).

use std::net::{IpAddr, SocketAddr};

use goodfetch_protocol::egress::{
    CDP_URL_NOT_ALLOWED, CdpUrlNotAllowedDetails, CdpUrlRule, ConfiguredKind, EgressClass,
};
use goodfetch_protocol::settings::SettingsLayer;
use url::Url;

use super::{EndpointPolicy, Policy, judge_addresses, judge_endpoint_addresses};
use crate::composition::CallError;

/// What `cdp_url_not_allowed` asks for.
const FIX: &str = "set the address in Operator policy or in user or project configuration, or give a public address";

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
        judge_addresses(url, std::slice::from_ref(address), session).map_err(|page| {
            refused(
                CdpUrlRule::PrivateAddress,
                page.address.unwrap_or(address.ip()),
                page.setting,
            )
        })?;
    }
    Ok(())
}

/// Whether `address` is a loopback address, an IPv4-mapped one included.
fn is_loopback(address: IpAddr) -> bool {
    address.to_canonical().is_loopback()
}
