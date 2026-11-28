//! Paid fetches family (`paid.test`, #1537, ADRs 0045 and 0076):
//! Cloudflare pay-per-crawl (`crawler-price` / `crawler-exact-price` /
//! `crawler-max-price` inside Web Bot Auth), x402 (`exact`, `upto`,
//! `batch-settlement`) settled through a local facilitator stand-in onto a
//! local chain with seeded accounts, plus refusal and interrupted-payment
//! tasks (`settled`, `still_able_to_settle`, `unable_to_settle`).
//! No external network is ever touched.

pub(crate) mod chain;
pub(crate) mod facilitator;
mod http;

use std::collections::{BTreeMap, BTreeSet};

use data_encoding::BASE64;
use hyper::header::HeaderMap;
use hyper::{Method, StatusCode};
use serde::Serialize;
use serde_json::json;

use crate::directory;
use crate::exchange::Reply;
use crate::generator;
use crate::oracle;
use crate::sync::Mutex;
use crate::task::{self, Entry, Expected, ExpectedError, Kind, Record, Task};

use self::chain::Chain;
use self::facilitator::{Facilitator, PaymentPayload, PaymentRequirements};

/// The family's own host name; each task is served on `<ordinal>.paid.test`.
pub(crate) const HOST: &str = "paid.test";

/// The family's name in the task list.
pub(crate) const FAMILY: &str = "paid";

const PLACE: u64 = 1537;

/// Standard price in atomic USDC / micro-USD (`10_000` = $0.01).
const PRICE_ATOMIC: u128 = 10_000;
const PRICE_USD_HEADER: &str = "USD 0.01";

/// Maximum authorized price for `upto` (`20_000` = $0.02; settles at `PRICE_ATOMIC` = $0.01).
const UPTO_MAX_ATOMIC: u128 = 20_000;

/// High price for the refusal task (`500_000` = $0.50).
const REFUSAL_PRICE_USD_HEADER: &str = "USD 0.50";

/// Outcome of an interrupted payment, as declared by the task and reported by
/// the oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InterruptedState {
    Settled,
    StillAbleToSettle,
    UnableToSettle,
}

impl InterruptedState {
    fn as_header(self) -> &'static str {
        match self {
            Self::Settled => "settled",
            Self::StillAbleToSettle => "still_able_to_settle",
            Self::UnableToSettle => "unable_to_settle",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "settled" => Some(Self::Settled),
            "still_able_to_settle" => Some(Self::StillAbleToSettle),
            "unable_to_settle" => Some(Self::UnableToSettle),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskKind {
    PayPerCrawl,
    X402Exact,
    X402Upto,
    X402BatchSettlement,
    Refusal,
    Interrupted(InterruptedState),
}

struct TaskSpec {
    ordinal: u64,
    kind: TaskKind,
    description: &'static str,
}

const TASKS: [TaskSpec; 8] = [
    TaskSpec {
        ordinal: 1,
        kind: TaskKind::PayPerCrawl,
        description: "Pay for this page using Cloudflare pay-per-crawl headers signed with Web Bot Auth and return its Record.",
    },
    TaskSpec {
        ordinal: 2,
        kind: TaskKind::X402Exact,
        description: "Pay for this page using x402 `exact` settled on the local chain and return its Record.",
    },
    TaskSpec {
        ordinal: 3,
        kind: TaskKind::X402Upto,
        description: "Pay for this page using x402 `upto` settled on the local chain and return its Record.",
    },
    TaskSpec {
        ordinal: 4,
        kind: TaskKind::X402BatchSettlement,
        description: "Pay for this page using x402 `batch-settlement` signed with Web Bot Auth and return its Record.",
    },
    TaskSpec {
        ordinal: 5,
        kind: TaskKind::Refusal,
        description: "Refuse to pay this page and report `payment_required` without paying.",
    },
    TaskSpec {
        ordinal: 6,
        kind: TaskKind::Interrupted(InterruptedState::Settled),
        description: "Attempt an x402 `exact` payment whose response is interrupted after on-chain settlement.",
    },
    TaskSpec {
        ordinal: 7,
        kind: TaskKind::Interrupted(InterruptedState::StillAbleToSettle),
        description: "Attempt an x402 `exact` payment whose response is interrupted while the payment can still settle.",
    },
    TaskSpec {
        ordinal: 8,
        kind: TaskKind::Interrupted(InterruptedState::UnableToSettle),
        description: "Attempt an x402 `exact` payment whose response is interrupted after expiry makes it unable to settle.",
    },
];

/// Per-task payment accounting observed by the Proving ground.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct PaymentSummary {
    pub(crate) paid_count: u64,
    pub(crate) total_paid_atomic: u128,
    pub(crate) fetched_count: u64,
    pub(crate) refused_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interrupted: Option<InterruptedState>,
}

/// Oracle answer for a `paid/*` task: the standard verdict plus what was paid,
/// fetched, refused, and any interrupted payment state.
#[derive(Serialize)]
pub(crate) struct PaidAnswer {
    #[serde(flatten)]
    answer: oracle::Answer,
    payments: PaymentSummary,
}

#[derive(Default)]
struct Ledger {
    /// Per-task counters.
    summaries: BTreeMap<String, PaymentSummary>,
    /// Used Web Bot Auth replay tokens (pay-per-crawl and batch-settlement).
    used_tokens: BTreeSet<String>,
    /// Tasks whose one-shot interrupted payment fault has already fired.
    interrupted_fired: BTreeSet<String>,
    /// Facilitator HTTP URL once bound.
    facilitator_url: String,
}

/// Shared state for the `paid` family, linking the HTTP pages, the local
/// chain, the facilitator stand-in, and the oracle.
pub(crate) struct Payments {
    seed: u64,
    chain: Chain,
    facilitator: Facilitator,
    ledger: Mutex<Ledger>,
}

impl Payments {
    pub(crate) fn new(seed: u64) -> Self {
        let chain = Chain::new(seed);
        let facilitator = Facilitator::new(chain.clone());
        Self {
            seed,
            chain,
            facilitator,
            ledger: Mutex::default(),
        }
    }

    pub(crate) fn chain(&self) -> Chain {
        self.chain.clone()
    }

    pub(crate) fn facilitator(&self) -> Facilitator {
        self.facilitator.clone()
    }

    pub(crate) fn set_facilitator_url(&self, url: String) {
        self.ledger.lock().facilitator_url = url;
    }

    /// Returns whether `host` belongs to the `paid.test` family.
    pub(crate) fn serves(host: &str) -> bool {
        host == HOST
            || host
                .strip_suffix(HOST)
                .is_some_and(|prefix| prefix.ends_with('.'))
    }

    /// Handles a request to `paid.test` or `<ordinal>.paid.test`.
    pub(crate) async fn respond(
        &self,
        method: &Method,
        host: &str,
        authority: &str,
        target: &str,
        headers: &HeaderMap,
    ) -> Reply {
        if method != Method::GET {
            return Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed");
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != "/page" {
            return Reply::error(StatusCode::NOT_FOUND, "not_found");
        }
        let ordinal = ordinal(host).unwrap_or(1);
        let Some(spec) = TASKS.iter().find(|task| task.ordinal == ordinal) else {
            return Reply::error(StatusCode::NOT_FOUND, "unknown_task");
        };
        let task_id = format!("{FAMILY}/{ordinal}");
        let fault_override = query_param(query, "interrupt")
            .or_else(|| header_str(headers, "x-proving-ground-payment-fault").map(str::to_owned));
        if fault_override.as_deref() == Some("refuse") {
            self.record_refused(&task_id);
            return pay_per_crawl_402(PRICE_USD_HEADER, "refused");
        }
        let override_interrupt = fault_override.as_deref().and_then(InterruptedState::parse);

        match spec.kind {
            TaskKind::PayPerCrawl => {
                self.handle_pay_per_crawl(&task_id, ordinal, authority, target, headers)
                    .await
            }
            TaskKind::Refusal => {
                self.record_refused(&task_id);
                pay_per_crawl_402(REFUSAL_PRICE_USD_HEADER, "refused")
            }
            TaskKind::X402Exact => {
                self.handle_x402(
                    &task_id,
                    ordinal,
                    "exact",
                    PRICE_ATOMIC,
                    PRICE_ATOMIC,
                    override_interrupt,
                    false,
                    authority,
                    target,
                    headers,
                )
                .await
            }
            TaskKind::X402Upto => {
                self.handle_x402(
                    &task_id,
                    ordinal,
                    "upto",
                    UPTO_MAX_ATOMIC,
                    PRICE_ATOMIC,
                    override_interrupt,
                    false,
                    authority,
                    target,
                    headers,
                )
                .await
            }
            TaskKind::X402BatchSettlement => {
                self.handle_x402(
                    &task_id,
                    ordinal,
                    "batch-settlement",
                    PRICE_ATOMIC,
                    PRICE_ATOMIC,
                    override_interrupt,
                    true,
                    authority,
                    target,
                    headers,
                )
                .await
            }
            TaskKind::Interrupted(state) => {
                let effective = override_interrupt.unwrap_or(state);
                self.handle_x402(
                    &task_id,
                    ordinal,
                    "exact",
                    PRICE_ATOMIC,
                    PRICE_ATOMIC,
                    Some(effective),
                    false,
                    authority,
                    target,
                    headers,
                )
                .await
            }
        }
    }

    async fn handle_pay_per_crawl(
        &self,
        task_id: &str,
        ordinal: u64,
        authority: &str,
        target: &str,
        headers: &HeaderMap,
    ) -> Reply {
        let exact = header_str(headers, "crawler-exact-price");
        let max = header_str(headers, "crawler-max-price");
        if exact.is_none() && max.is_none() {
            self.record_refused(task_id);
            return pay_per_crawl_402(PRICE_USD_HEADER, "payment_required");
        }
        let header_pairs = collect_headers(headers);
        let Ok(verified) = directory::check(authority, "GET", target, &header_pairs).await else {
            self.record_refused(task_id);
            return pay_per_crawl_402(PRICE_USD_HEADER, "unsigned");
        };
        let covers_price = (exact.is_some()
            && verified
                .covered_headers
                .iter()
                .any(|h| h == "crawler-exact-price"))
            || (max.is_some()
                && verified
                    .covered_headers
                    .iter()
                    .any(|h| h == "crawler-max-price"));
        if !covers_price {
            self.record_refused(task_id);
            return pay_per_crawl_402(PRICE_USD_HEADER, "unsigned");
        }
        let price_ok = match (exact, max) {
            (Some(value), _) => parse_usd_micro(value) == Some(PRICE_ATOMIC),
            (None, Some(value)) => parse_usd_micro(value).is_some_and(|amt| amt >= PRICE_ATOMIC),
            (None, None) => false,
        };
        if !price_ok {
            self.record_refused(task_id);
            return pay_per_crawl_402(PRICE_USD_HEADER, "wrong_amount");
        }
        {
            let mut ledger = self.ledger.lock();
            if !ledger.used_tokens.insert(verified.replay_token) {
                ledger
                    .summaries
                    .entry(task_id.to_owned())
                    .or_default()
                    .refused_count += 1;
                return pay_per_crawl_402(PRICE_USD_HEADER, "replay");
            }
            let summary = ledger.summaries.entry(task_id.to_owned()).or_default();
            summary.paid_count += 1;
            summary.total_paid_atomic += PRICE_ATOMIC;
            summary.fetched_count += 1;
        }
        let mut reply = Reply::html(render_page(self.seed, ordinal));
        reply
            .headers
            .push(("crawler-charged", PRICE_USD_HEADER.to_owned()));
        reply
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "handles all x402 schemes and optional interrupt injection in one shared flow"
    )]
    async fn handle_x402(
        &self,
        task_id: &str,
        ordinal: u64,
        scheme: &str,
        max_amount: u128,
        settle_amount: u128,
        interrupt: Option<InterruptedState>,
        require_web_bot_auth: bool,
        authority: &str,
        target: &str,
        headers: &HeaderMap,
    ) -> Reply {
        let payment_header =
            header_str(headers, "payment-signature").or_else(|| header_str(headers, "x-payment"));
        let Some(encoded) = payment_header else {
            self.record_refused(task_id);
            return self.x402_402(scheme, max_amount, target, "payment_required");
        };
        if require_web_bot_auth {
            let header_pairs = collect_headers(headers);
            let Ok(verified) = directory::check(authority, "GET", target, &header_pairs).await
            else {
                self.record_refused(task_id);
                return self.x402_402(scheme, max_amount, target, "unsigned");
            };
            let covers_payment = verified
                .covered_headers
                .iter()
                .any(|h| h == "payment-signature" || h == "x-payment");
            if !covers_payment {
                self.record_refused(task_id);
                return self.x402_402(scheme, max_amount, target, "unsigned");
            }
            let is_replay = {
                let mut ledger = self.ledger.lock();
                if ledger.used_tokens.insert(verified.replay_token) {
                    false
                } else {
                    ledger
                        .summaries
                        .entry(task_id.to_owned())
                        .or_default()
                        .refused_count += 1;
                    true
                }
            };
            if is_replay {
                return self.x402_402(scheme, max_amount, target, "replay");
            }
        }
        let Ok(decoded) = BASE64.decode(encoded.as_bytes()) else {
            self.record_refused(task_id);
            return self.x402_402(scheme, max_amount, target, "malformed");
        };
        let Ok(payment) = serde_json::from_slice::<PaymentPayload>(&decoded) else {
            self.record_refused(task_id);
            return self.x402_402(scheme, max_amount, target, "malformed");
        };
        let (network, asset, pay_to) = self.chain.x402_terms();
        let requirements = PaymentRequirements {
            scheme: scheme.to_owned(),
            network: network.to_owned(),
            max_amount_required: max_amount.to_string(),
            asset: asset.to_owned(),
            pay_to: pay_to.to_owned(),
            settle_amount: Some(settle_amount.to_string()),
        };

        if let Some(state) = interrupt
            && let Some(reply) = self.try_handle_interrupted(
                task_id,
                state,
                &payment,
                &requirements,
                scheme,
                max_amount,
                target,
            )
        {
            return reply;
        }

        match self.facilitator.settle(&payment, &requirements) {
            Ok(settlement) => {
                {
                    let mut ledger = self.ledger.lock();
                    let summary = ledger.summaries.entry(task_id.to_owned()).or_default();
                    summary.paid_count += 1;
                    summary.total_paid_atomic += settlement.settled_amount;
                    summary.fetched_count += 1;
                }
                let encoded_response = BASE64.encode(
                    &serde_json::to_vec(&settlement)
                        .expect("invariant: settlement serializes to JSON"),
                );
                let mut reply = Reply::html(render_page(self.seed, ordinal));
                reply.headers.push(("payment-response", encoded_response));
                reply
            }
            Err(rejection) => {
                self.record_refused(task_id);
                self.x402_402(scheme, max_amount, target, rejection.code())
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "applies the requested interruption outcome to a verified x402 payment"
    )]
    fn try_handle_interrupted(
        &self,
        task_id: &str,
        state: InterruptedState,
        payment: &PaymentPayload,
        requirements: &PaymentRequirements,
        scheme: &str,
        max_amount: u128,
        target: &str,
    ) -> Option<Reply> {
        if self.ledger.lock().interrupted_fired.contains(task_id) {
            return None;
        }
        let verified_amount = match self.facilitator.verify(payment, requirements) {
            Ok(amount) => amount,
            Err(rejection) => {
                self.record_refused(task_id);
                return Some(self.x402_402(scheme, max_amount, target, rejection.code()));
            }
        };
        if !self
            .ledger
            .lock()
            .interrupted_fired
            .insert(task_id.to_owned())
        {
            return None;
        }
        match state {
            InterruptedState::Settled => {
                if let Err(rejection) = self.facilitator.settle(payment, requirements) {
                    self.record_refused(task_id);
                    return Some(self.x402_402(scheme, max_amount, target, rejection.code()));
                }
                let mut ledger = self.ledger.lock();
                let summary = ledger.summaries.entry(task_id.to_owned()).or_default();
                summary.paid_count += 1;
                summary.total_paid_atomic += verified_amount;
                summary.interrupted = Some(InterruptedState::Settled);
            }
            InterruptedState::StillAbleToSettle => {
                self.chain.record_pending(
                    &payment.scheme,
                    &payment.payload.from,
                    &payment.payload.to,
                    verified_amount,
                    payment.payload.valid_before,
                    &payment.payload.nonce,
                );
                let mut ledger = self.ledger.lock();
                let summary = ledger.summaries.entry(task_id.to_owned()).or_default();
                summary.interrupted = Some(InterruptedState::StillAbleToSettle);
            }
            InterruptedState::UnableToSettle => {
                self.chain.expire_payment(
                    &payment.scheme,
                    &payment.payload.from,
                    &payment.payload.to,
                    verified_amount,
                    payment.payload.valid_before,
                    &payment.payload.nonce,
                );
                let mut ledger = self.ledger.lock();
                let summary = ledger.summaries.entry(task_id.to_owned()).or_default();
                summary.interrupted = Some(InterruptedState::UnableToSettle);
            }
        }
        let mut reply = Reply::json(
            StatusCode::SERVICE_UNAVAILABLE,
            &json!({
                "error": "payment_interrupted",
                "interrupted": state,
            }),
        );
        reply
            .headers
            .push(("x-payment-interrupted", state.as_header().to_owned()));
        Some(reply)
    }

    fn x402_402(&self, scheme: &str, max_amount: u128, resource: &str, error_code: &str) -> Reply {
        let (network, asset, pay_to) = self.chain.x402_terms();
        let facilitator_url = self.ledger.lock().facilitator_url.clone();
        let body = json!({
            "x402Version": 2,
            "error": error_code,
            "accepts": [{
                "scheme": scheme,
                "network": network,
                "maxAmountRequired": max_amount.to_string(),
                "asset": asset,
                "payTo": pay_to,
                "resource": resource,
                "maxTimeoutSeconds": 60,
                "facilitator": facilitator_url,
                "extra": {
                    "paymentIdentifierSupported": true,
                },
            }],
        });
        let encoded =
            BASE64.encode(&serde_json::to_vec(&body).expect("invariant: 402 body serializes"));
        let mut reply = Reply::json(StatusCode::PAYMENT_REQUIRED, &body);
        reply.headers.push(("payment-required", encoded));
        reply
            .headers
            .push(("x-payment-error", error_code.to_owned()));
        reply
    }

    fn record_refused(&self, task_id: &str) {
        self.ledger
            .lock()
            .summaries
            .entry(task_id.to_owned())
            .or_default()
            .refused_count += 1;
    }

    /// Returns the oracle's extended verdict for a `paid/*` task, including
    /// counts of what was paid and fetched and any interrupted payment state.
    pub(crate) fn judge(&self, task_id: &str, answer: oracle::Answer) -> PaidAnswer {
        let mut summary = self
            .ledger
            .lock()
            .summaries
            .get(task_id)
            .cloned()
            .unwrap_or_default();
        if summary.interrupted.is_none()
            && let Some(ordinal) = task_id
                .strip_prefix(FAMILY)
                .and_then(|rest| rest.strip_prefix('/'))
                .and_then(|num| num.parse::<u64>().ok())
            && let Some(TaskSpec {
                kind: TaskKind::Interrupted(declared),
                ..
            }) = TASKS.iter().find(|spec| spec.ordinal == ordinal)
        {
            summary.interrupted = Some(*declared);
        }
        PaidAnswer {
            answer,
            payments: summary,
        }
    }
}

/// The `paid` family's tasks.
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    TASKS
        .iter()
        .map(|spec| {
            let host = format!("{}.{HOST}", spec.ordinal);
            let expected = match spec.kind {
                TaskKind::Refusal => Expected::refused(Some(ExpectedError {
                    code: "payment_required",
                    ..ExpectedError::default()
                })),
                TaskKind::Interrupted(_) => Expected::default(),
                _ => Expected::records(vec![record_for(seed, spec.ordinal)]),
            };
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{}", spec.ordinal),
                    family: FAMILY,
                    kind: Kind::Extract,
                    start_url: format!("{}/page", task::origin(&host, port)),
                    description: spec.description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected,
            }
        })
        .collect()
}

fn record_for(seed: u64, ordinal: u64) -> Record {
    let code = 1000 + generator::below(seed, &[PLACE, ordinal], 9000);
    let title = format!("Paid article {ordinal}-{code}");
    BTreeMap::from([
        ("id", format!("PAID-{ordinal}-{code}")),
        ("title", title),
        ("price_atomic", PRICE_ATOMIC.to_string()),
    ])
}

fn render_page(seed: u64, ordinal: u64) -> String {
    let record = record_for(seed, ordinal);
    let id = record.get("id").map(String::as_str).unwrap_or_default();
    let title = record.get("title").map(String::as_str).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body><article data-id=\"{id}\"><h1>{title}</h1><p data-price=\"{PRICE_ATOMIC}\">Paid content delivered.</p></article></body></html>"
    )
}

fn pay_per_crawl_402(price: &str, error_code: &str) -> Reply {
    let mut reply = Reply::json(
        StatusCode::PAYMENT_REQUIRED,
        &json!({
            "error": error_code,
            "price": price,
        }),
    );
    reply.headers.push(("crawler-price", price.to_owned()));
    reply
        .headers
        .push(("x-payment-error", error_code.to_owned()));
    reply
}

fn ordinal(host: &str) -> Option<u64> {
    host.split_once('.')?.0.parse().ok()
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn collect_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|val| (name.as_str().to_ascii_lowercase(), val.to_owned()))
        })
        .collect()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Parses a `USD <decimal>` header value into micro-USD (1 USD = `1_000_000`).
fn parse_usd_micro(raw: &str) -> Option<u128> {
    let amount = raw.trim().strip_prefix("USD ")?.trim();
    if amount.is_empty() {
        return None;
    }
    let (whole, frac) = match amount.split_once('.') {
        Some((w, f)) => (w, f),
        None => (amount, ""),
    };
    if frac.len() > 6
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole_val: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut frac_val: u128 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    for _ in frac.len()..6 {
        frac_val = frac_val.checked_mul(10)?;
    }
    whole_val.checked_mul(1_000_000)?.checked_add(frac_val)
}

#[cfg(test)]
mod tests {
    use super::facilitator::Rejection;
    use super::*;

    #[test]
    fn parses_usd_prices_into_micro_usd() {
        assert_eq!(parse_usd_micro("USD 0.01"), Some(10_000));
        assert_eq!(parse_usd_micro("USD 0.0100"), Some(10_000));
        assert_eq!(parse_usd_micro("USD 1.5"), Some(1_500_000));
        assert_eq!(parse_usd_micro("EUR 0.01"), None);
        assert_eq!(parse_usd_micro("USD -0.01"), None);
    }

    #[test]
    fn rejection_codes_are_stable() {
        assert_eq!(Rejection::WrongAmount.code(), "wrong_amount");
        assert_eq!(Rejection::Replay.code(), "replay");
        assert_eq!(Rejection::Unsigned.code(), "unsigned");
    }
}
