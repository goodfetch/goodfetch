//! The SSRF case table (ADR 0067): one table of cases, committed as data
//! among test support's shared fixtures, each with its input and the typed
//! refusal every layer must give, which each layer's thin runner applies.

use std::borrow::Cow;
use std::fmt;
use std::net::IpAddr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::resolver::Lookup;

/// The SSRF case table: every case, in the committed file's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SsrfTable {
    /// The cases.
    pub cases: Vec<SsrfCase>,
}

/// One SSRF case: what a run asks for and the refusal it must meet, with no
/// connection reaching its target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SsrfCase {
    /// The case's name, unique in the table, in snake case.
    pub name: String,
    /// The family the case belongs to.
    pub family: Family,
    /// The URL the run requests.
    pub url: String,
    /// The way the request leaves the run.
    #[serde(default)]
    pub route: Route,
    /// The DNS answers the case's names get, in the order they are asked
    /// for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lookups: Vec<CaseLookup>,
    /// The redirect the public stand-in answers `url` with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect: Option<Redirect>,
    /// How many times the run requests `url`; a rebinding case needs two.
    #[serde(default = "one")]
    #[schemars(range(min = 1))]
    pub fetches: u32,
    /// The address no connection may reach, when the case has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<IpAddr>,
    /// The block of IANA's special-purpose registries the target lies in,
    /// in CIDR notation as the registry writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// The typed refusal every layer gives.
    pub refusal: Refusal,
}

impl SsrfCase {
    /// The case's lookups as a [`ScriptedResolver`](crate::ScriptedResolver)
    /// script, with `public` for the layer's public stand-in.
    pub fn script(&self, public: IpAddr) -> Vec<Lookup> {
        self.lookups
            .iter()
            .map(|lookup| {
                let addresses = lookup.answers.iter().map(|answer| match answer {
                    Answer::Public => public,
                    Answer::Address(address) => *address,
                });
                Lookup::new(&lookup.name, addresses)
            })
            .collect()
    }
}

fn one() -> u32 {
    1
}

/// The families of SSRF cases (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Spellings of an address a URL parser accepts: integers, octal and
    /// hexadecimal parts, short forms, percent-encoding, full-width digits.
    Encoding,
    /// IPv6 forms that embed or stand for a private address: IPv4-mapped,
    /// NAT64, 6to4, Teredo, zone IDs.
    Ipv6Form,
    /// One block of IANA's special-purpose registries that is not globally
    /// reachable.
    SpecialPurposeBlock,
    /// A name whose answers mix public and private addresses.
    MixedAnswers,
    /// A redirect of each status and kind, and to schemes other than HTTP.
    Redirect,
    /// A name whose answer changes between lookups.
    Rebinding,
    /// A request through the run's upstream proxy, which would resolve the
    /// name itself.
    UpstreamProxy,
    /// A Plugin's own request.
    PluginTraffic,
    /// A Lent page's client trying a browser context with its own proxy, or
    /// an extension (ADR 0089).
    LentPageClient,
}

/// The way a case's request leaves the run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum Route {
    /// The run's own request.
    #[default]
    Direct,
    /// Through the run's upstream proxy.
    UpstreamProxy,
    /// A Plugin's request.
    Plugin,
    /// A browser context a Lent page's client creates with its own proxy.
    BrowserContext {
        /// The context's proxy server.
        proxy_server: String,
    },
    /// An extension a Lent page's client loads, which requests the URL.
    Extension,
}

/// A case's scripted DNS answer to one lookup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseLookup {
    /// The name looked up.
    pub name: String,
    /// The addresses it resolves to.
    pub answers: Vec<Answer>,
}

/// One address of a lookup's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Answer {
    /// The layer's public stand-in, written `public`: the address a runner
    /// treats as public, such as the Proving ground's.
    Public,
    /// This address.
    Address(IpAddr),
}

/// What an answer is written as for the layer's public stand-in.
const PUBLIC: &str = "public";

impl TryFrom<String> for Answer {
    type Error = std::net::AddrParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text == PUBLIC {
            return Ok(Self::Public);
        }
        text.parse().map(Self::Address)
    }
}

impl From<Answer> for String {
    fn from(answer: Answer) -> Self {
        match answer {
            Answer::Public => PUBLIC.to_owned(),
            Answer::Address(address) => address.to_string(),
        }
    }
}

impl JsonSchema for Answer {
    fn schema_name() -> Cow<'static, str> {
        "Answer".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An address, or `public` for the layer's public stand-in.",
            "anyOf": [
                { "const": PUBLIC },
                { "type": "string", "format": "ipv4" },
                { "type": "string", "format": "ipv6" }
            ]
        })
    }
}

/// The redirect a case's public stand-in answers with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Redirect {
    /// A 3xx status with a `Location` header.
    Status {
        /// The status.
        #[schemars(range(min = 300, max = 399))]
        status: u16,
        /// The `Location` header.
        location: String,
    },
    /// A `Refresh` header.
    RefreshHeader {
        /// The URL it names.
        location: String,
    },
    /// A `<meta http-equiv="refresh">` element.
    MetaRefresh {
        /// The URL it names.
        location: String,
    },
    /// A script that assigns `location`.
    Script {
        /// The URL it assigns.
        location: String,
    },
}

/// The typed refusal a case expects, by its Error code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Refusal {
    /// `egress_refused`, with the rule that refused.
    EgressRefused {
        /// The rule.
        rule: EgressRule,
    },
    /// `cdp_command_refused`: a Lent page's endpoint refused the command
    /// (ADR 0089).
    CdpCommandRefused,
}

/// The rule an `egress_refused` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EgressRule {
    /// An address that is not globally reachable.
    PrivateAddress,
    /// A scheme other than HTTP and HTTPS.
    Scheme,
}

/// Why a table cannot be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SsrfTableError {
    /// The text is not a table of the committed format.
    #[error("the SSRF case table is malformed: {0}")]
    Malformed(#[from] toml::de::Error),
}

impl SsrfTable {
    /// The table written in `text`.
    ///
    /// # Errors
    ///
    /// [`SsrfTableError::Malformed`] when the text is not TOML of the
    /// table's format, such as a case naming an unknown family.
    pub fn parse(text: &str) -> Result<Self, SsrfTableError> {
        Ok(toml::from_str(text)?)
    }
}

/// What a layer did with one case, as its runner observed it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The typed refusal the layer gave, if any.
    pub refusal: Option<Refusal>,
    /// The addresses connections reached while the layer handled the case,
    /// from the Proving ground's connection record end to end.
    pub connections: Vec<IpAddr>,
}

impl Outcome {
    /// A refusal with no connection.
    pub fn refused(refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            connections: Vec::new(),
        }
    }

    /// No refusal, and connections to `addresses`.
    pub fn connected(addresses: impl IntoIterator<Item = IpAddr>) -> Self {
        Self {
            refusal: None,
            connections: addresses.into_iter().collect(),
        }
    }
}

/// A case a layer let through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Miss {
    /// The case's name.
    pub case: String,
    /// The refusal the case expects.
    pub expected: Refusal,
    /// The refusal the layer gave, if any.
    pub refusal: Option<Refusal>,
    /// Whether a connection reached the case's target.
    pub reached_target: bool,
}

/// Every case a layer let through, in the table's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LetThrough {
    /// The cases.
    pub misses: Vec<Miss>,
}

impl fmt::Display for LetThrough {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the layer let {} SSRF cases through:",
            self.misses.len()
        )?;
        for miss in &self.misses {
            write!(formatter, "\n  {miss}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LetThrough {}

/// Runs each case through `layer`, a layer's thin runner, and judges it:
/// a case passes when the layer gave its expected refusal and no connection
/// reached its target.
///
/// # Errors
///
/// [`LetThrough`] listing every case that did not pass, not only the
/// first.
pub fn run_cases<'case>(
    cases: impl IntoIterator<Item = &'case SsrfCase>,
    mut layer: impl FnMut(&SsrfCase) -> Outcome,
) -> Result<(), LetThrough> {
    let misses: Vec<Miss> = cases
        .into_iter()
        .filter_map(|case| {
            let outcome = layer(case);
            // An IPv4-mapped target and its IPv4 address are one destination.
            let reached_target = case.target.is_some_and(|target| {
                outcome
                    .connections
                    .iter()
                    .any(|connection| connection.to_canonical() == target.to_canonical())
            });
            let refused = outcome.refusal.as_ref() == Some(&case.refusal);
            (!refused || reached_target).then(|| Miss {
                case: case.name.clone(),
                expected: case.refusal.clone(),
                refusal: outcome.refusal,
                reached_target,
            })
        })
        .collect();
    if misses.is_empty() {
        Ok(())
    } else {
        Err(LetThrough { misses })
    }
}

impl fmt::Display for Miss {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let refusal = self
            .refusal
            .as_ref()
            .map_or_else(|| "no refusal".to_owned(), |refusal| format!("{refusal:?}"));
        write!(
            formatter,
            "{}: expected {:?}, got {refusal}",
            self.case, self.expected
        )?;
        if self.reached_target {
            formatter.write_str(", and a connection reached the target")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
