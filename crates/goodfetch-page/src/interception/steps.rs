//! Steps 3 to 8 of the pipeline for what the last route sends on, as one
//! function of the request: the egress check and the domain policy, the
//! never-block list, the Filter lists and `browser.block_resources`, the
//! places of the steps later tickets fill, and the command that sends the
//! request on or answers it with a stand-in. Pure, so the per-request
//! benchmark runs it as the pipeline does (ADR 0071): the values it builds
//! are the CDP module's `requests`, which its parent imports, the
//! interception in the crate and the scenarios in the benchmark.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{ContinueRequest, Fulfill, Header, PausedId, RequestHeaders};
use goodfetch_clean_up_rules::matching::{self, Choices, ResourceKind, Verdict};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use goodfetch_engine::request_signing::{RequestSigner, SignedHeaders};
use goodfetch_protocol::page::request::SecretHeader;
use url::Url;

/// What to send instead of what a request carries; each part replaces the
/// request's whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Overrides {
    pub(crate) url: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    /// Headers whose values come from Secret placeholders, resolved at step 7
    /// only on their bound domains (ADR 0092, ticket #733).
    pub(crate) secret_headers: Option<BTreeMap<String, SecretHeader>>,
    /// In Base64.
    pub(crate) post_data: Option<String>,
}

impl Overrides {
    /// These overrides, each part `later` names taken over its own.
    pub(crate) fn then(self, later: Self) -> Self {
        Self {
            url: later.url.or(self.url),
            method: later.method.or(self.method),
            headers: later.headers.or(self.headers),
            secret_headers: later.secret_headers.or(self.secret_headers),
            post_data: later.post_data.or(self.post_data),
        }
    }
}

/// Step 4 as a page's Session chose it (ADRs 0047 and 0117): the
/// never-block list, the embedded Filter lists when any is on, which every
/// Session shares, and its choices; each match against the lists is
/// counted on `work`, its Run's.
#[derive(Clone)]
pub(crate) struct Blocking {
    pub(crate) never_block: Arc<NeverBlock>,
    pub(crate) lists: Option<Arc<FilterLists>>,
    pub(crate) choices: Choices,
    pub(crate) work: CountedWork,
}

impl Blocking {
    /// Step 4 with every Filter list off, as a page loaded once more with
    /// blocking off, or on a domain blocking was learned off for, takes it
    /// (ADR 0047): the never-block list and `browser.block_resources` stay.
    pub(crate) fn without_lists(&self) -> Self {
        Self {
            never_block: Arc::clone(&self.never_block),
            lists: None,
            choices: Choices {
                ads: false,
                trackers: false,
                annoyances: false,
                extra: false,
                ..self.choices.clone()
            },
            work: self.work.clone(),
        }
    }
}

/// The request paused at `id`, as the last route sends it on.
pub(crate) struct Paused<'a> {
    pub(crate) id: &'a PausedId,
    pub(crate) url: &'a str,
    pub(crate) method: &'a str,
    pub(crate) overrides: &'a Overrides,
    /// The headers it carries, which `overrides` may replace.
    pub(crate) headers: RequestHeaders<'a>,
    /// What it loads.
    pub(crate) kind: ResourceKind,
    /// The URL of the document that sent it.
    pub(crate) source_url: &'a str,
    /// How the egress policy judges it: as the page's own request, or as a
    /// hop of a navigation a call asked for.
    pub(crate) egress: RequestKind,
}

/// What steps 3 to 8 do with a request that goes on.
#[derive(Debug)]
pub(crate) enum Sent {
    /// Step 8's command: on to the network.
    Continue(ContinueRequest),
    /// A redirect rule's stand-in answers it, as the browser takes it.
    StandIn(Fulfill),
}

/// Why a request goes no further.
#[derive(Debug)]
pub(crate) enum Stopped {
    /// The domain policy refused it (step 3).
    Refused(Refused),
    /// A Filter list, when `by_list`, or `browser.block_resources` blocked
    /// it (step 4): it fails as `BlockedByClient`.
    Blocked { by_list: bool },
}

/// Step 7's engine-owned additions, applied after every route ran: a Borrowed
/// browser's `Sec-GPC: 1` header when `consent.gpc: on` (ADR 0048) and an
/// `identified` Session's signature (ADR 0045), signed last over what leaves.
#[derive(Clone, Copy, Default)]
pub(crate) struct Additions<'a> {
    pub(crate) signing: Option<&'a RequestSigner>,
    pub(crate) send_gpc_header: bool,
}

/// The Global Privacy Control request header name (ADR 0048).
const SEC_GPC_NAME: &str = "Sec-GPC";

/// The Global Privacy Control request header value (ADR 0048).
const SEC_GPC_VALUE: &str = "1";

/// Steps 3 to 7 for `paused` under `policy` and `blocking`, with step 7's
/// `additions`, and step 8's command; `fetch` pauses its response, so a
/// stand-in, which has none, blocks a fetched request instead.
///
/// # Errors
///
/// The domain policy's refusal, or a block: the request goes no further.
pub(crate) fn send_on(
    paused: &Paused<'_>,
    policy: &Policy,
    blocking: &Blocking,
    additions: Additions<'_>,
    fetch: bool,
) -> Result<Sent, Stopped> {
    // Step 3: the egress check and the domain policy.
    let request = egress::Request {
        url: paused.url,
        kind: paused.egress,
    };
    if let Judgment::Refused(refused) = egress::judge(&request, policy) {
        return Err(Stopped::Refused(refused));
    }
    // Step 4: the never-block list, the Filter lists and
    // `browser.block_resources`.
    if blocking.lists.is_some() {
        blocking.work.count(Row::InterceptedMatches);
    }
    let verdict = matching::judge(
        &matching::Request {
            url: paused.url,
            source_url: paused.source_url,
            kind: paused.kind,
            method: paused.method,
        },
        &blocking.never_block,
        blocking.lists.as_deref(),
        &blocking.choices,
    );
    match verdict {
        Verdict::StandIn { mime, body, .. } if !fetch => {
            return Ok(Sent::StandIn(Fulfill {
                id: paused.id.clone(),
                status: 200,
                headers: vec![Header {
                    name: "Content-Type".to_owned(),
                    value: mime,
                }],
                body,
            }));
        }
        verdict if !verdict.sends_on() => {
            return Err(Stopped::Blocked {
                by_list: !matches!(verdict, Verdict::ResourceBlocked),
            });
        }
        _ => {}
    }
    // Step 5: the Response cache and a replaying Run's Response recording
    // (area 6).
    // Step 6: the page's navigation gate, which its caller asks once this
    // answers, since it waits; the lease check is asked before the command
    // goes.
    // Step 7: the engine's own additions, which no route read. The
    // Session's cookies are the browser's own: it puts them on the request
    // as it sends it on, whatever headers a route set, and the routes' view
    // leaves them out. Secret placeholders and the Session's credentials
    // (#654, #733) are resolved here. On a Borrowed browser with
    // `consent.gpc: on`, `Sec-GPC: 1` goes here (ADR 0048, issue 758). An
    // `identified` Session signs the request last, over what it sends
    // (ADR 0045; #1551).
    let signed = additions.signing.and_then(|signer| {
        Url::parse(paused.url)
            .ok()
            .and_then(|url| signer.sign(&url))
    });
    let headers = if signed.is_some() || additions.send_gpc_header {
        Some(added_entries(paused, additions.send_gpc_header, signed))
    } else {
        paused.overrides.headers.as_ref().map(entries)
    };
    // Step 8: the network.
    Ok(Sent::Continue(ContinueRequest {
        id: paused.id.clone(),
        url: paused.overrides.url.clone(),
        method: paused.overrides.method.clone(),
        post_data: paused.overrides.post_data.clone(),
        headers,
        intercept_response: fetch,
    }))
}

/// The headers `paused` is sent on with, its overrides' or its own, plus
/// `Sec-GPC: 1` when `send_gpc_header` is set and `signed` when present,
/// replacing any carried headers of the same names.
fn added_entries(
    paused: &Paused<'_>,
    send_gpc_header: bool,
    signed: Option<SignedHeaders>,
) -> Vec<Header> {
    let carried: Vec<(&str, &str)> = match paused.overrides.headers.as_ref() {
        Some(headers) => headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect(),
        None => paused.headers.iter().collect(),
    };
    let mut out: Vec<Header> = carried
        .into_iter()
        .filter(|(name, _)| {
            let replaced_by_gpc = send_gpc_header && SEC_GPC_NAME.eq_ignore_ascii_case(name);
            let replaced_by_sig = signed.is_some()
                && SignedHeaders::NAMES
                    .iter()
                    .any(|signed| signed.eq_ignore_ascii_case(name));
            !replaced_by_gpc && !replaced_by_sig
        })
        .map(|(name, value)| Header {
            name: name.to_owned(),
            value: value.to_owned(),
        })
        .collect();
    if send_gpc_header {
        out.push(Header {
            name: SEC_GPC_NAME.to_owned(),
            value: SEC_GPC_VALUE.to_owned(),
        });
    }
    if let Some(signed) = signed {
        out.extend(
            signed
                .into_entries()
                .into_iter()
                .map(|(name, value)| Header {
                    name: name.to_owned(),
                    value,
                }),
        );
    }
    out
}

/// Headers as the browser takes them.
pub(crate) fn entries(headers: &BTreeMap<String, String>) -> Vec<Header> {
    headers
        .iter()
        .map(|(name, value)| Header {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}
