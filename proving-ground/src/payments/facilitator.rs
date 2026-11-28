//! x402 facilitator HTTP stand-in (#1537, ADRs 0045 and 0076):
//! verifies and settles `exact`, `upto`, and `batch-settlement` x402 payment
//! payloads against the Proving ground's local chain on loopback, with no
//! external network calls.
#![expect(
    clippy::disallowed_types,
    reason = "the facilitator stand-in opens its own loopback listener outside the engine transport (ADR 0060)"
)]

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use anyhow::{Context as _, Result};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::chain::{Chain, NETWORK, NonceState, SigningInput};

const BODY_LIMIT: usize = 256 * 1024;

/// Supported x402 schemes.
pub(crate) const SCHEMES: [&str; 3] = ["exact", "upto", "batch-settlement"];

/// Decoded x402 payment payload carried in `payment-signature` (base64 JSON)
/// or submitted to the facilitator's `/verify` and `/settle` endpoints.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaymentPayload {
    pub(crate) x402_version: u8,
    pub(crate) scheme: String,
    pub(crate) network: String,
    #[serde(default)]
    pub(crate) payment_id: Option<String>,
    pub(crate) payload: Authorization,
}

/// Payment authorization fields inside [`PaymentPayload`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Authorization {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) value: String,
    pub(crate) valid_before: u64,
    pub(crate) nonce: String,
    pub(crate) signature: String,
}

/// Requirements against which the facilitator verifies or settles a payment.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaymentRequirements {
    pub(crate) scheme: String,
    pub(crate) network: String,
    pub(crate) max_amount_required: String,
    pub(crate) asset: String,
    pub(crate) pay_to: String,
    /// Optional actual amount to settle for `upto` (defaults to `max_amount_required`).
    #[serde(default)]
    pub(crate) settle_amount: Option<String>,
}

/// Result of a facilitator settlement.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settlement {
    pub(crate) success: bool,
    pub(crate) transaction: String,
    pub(crate) network: String,
    pub(crate) payer: String,
    pub(crate) settled_amount: u128,
}

/// Why a payment failed verification or settlement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    UnsupportedScheme,
    WrongNetwork,
    WrongPayee,
    WrongAmount,
    Unsigned,
    Expired,
    Replay,
    InsufficientFunds,
    Malformed,
}

impl Rejection {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedScheme => "unsupported_scheme",
            Self::WrongNetwork => "wrong_network",
            Self::WrongPayee => "wrong_payee",
            Self::WrongAmount => "wrong_amount",
            Self::Unsigned => "unsigned",
            Self::Expired => "expired",
            Self::Replay => "replay",
            Self::InsufficientFunds => "insufficient_funds",
            Self::Malformed => "malformed",
        }
    }
}

/// Shared facilitator logic backed by the local [`Chain`].
#[derive(Clone)]
pub(crate) struct Facilitator {
    chain: Chain,
}

/// Bound facilitator HTTP server.
pub(crate) struct FacilitatorServer {
    listener: TcpListener,
    facilitator: Facilitator,
}

impl Facilitator {
    pub(crate) fn new(chain: Chain) -> Self {
        Self { chain }
    }

    pub(crate) fn chain(&self) -> &Chain {
        &self.chain
    }

    /// Verifies `payment` against `requirements` and returns the atomic amount
    /// that will be settled if valid.
    pub(crate) fn verify(
        &self,
        payment: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<u128, Rejection> {
        if payment.x402_version != 2 || !SCHEMES.contains(&payment.scheme.as_str()) {
            return Err(Rejection::UnsupportedScheme);
        }
        if payment.scheme != requirements.scheme {
            return Err(Rejection::UnsupportedScheme);
        }
        if payment.network != NETWORK || payment.network != requirements.network {
            return Err(Rejection::WrongNetwork);
        }
        if !payment
            .payload
            .to
            .eq_ignore_ascii_case(&requirements.pay_to)
        {
            return Err(Rejection::WrongPayee);
        }
        let offered: u128 = payment
            .payload
            .value
            .parse()
            .map_err(|_| Rejection::Malformed)?;
        let required: u128 = requirements
            .max_amount_required
            .parse()
            .map_err(|_| Rejection::Malformed)?;
        let settle_amount = match &requirements.settle_amount {
            Some(amount) => amount.parse().map_err(|_| Rejection::Malformed)?,
            None => required,
        };
        match payment.scheme.as_str() {
            "exact" | "batch-settlement" => {
                if offered != required {
                    return Err(Rejection::WrongAmount);
                }
            }
            "upto" => {
                if offered < settle_amount || offered < required {
                    return Err(Rejection::WrongAmount);
                }
            }
            _ => return Err(Rejection::UnsupportedScheme),
        }
        let signing_input = SigningInput {
            scheme: &payment.scheme,
            network: &payment.network,
            from: &payment.payload.from,
            to: &payment.payload.to,
            value: offered,
            valid_before: payment.payload.valid_before,
            nonce: &payment.payload.nonce,
        };
        if payment.payload.signature.is_empty()
            || !self
                .chain
                .verify_signature(&signing_input, &payment.payload.signature)
        {
            return Err(Rejection::Unsigned);
        }
        if let Some(existing) = self.chain.nonce_record(&payment.payload.nonce) {
            return match existing.state {
                NonceState::Settled => Err(Rejection::Replay),
                NonceState::Expired => Err(Rejection::Expired),
                NonceState::Pending => Ok(settle_amount),
            };
        }
        if self.chain.block_timestamp() >= payment.payload.valid_before {
            return Err(Rejection::Expired);
        }
        Ok(settle_amount)
    }

    /// Verifies and settles `payment` on the local chain.
    pub(crate) fn settle(
        &self,
        payment: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<Settlement, Rejection> {
        let settled_amount = self.verify(payment, requirements)?;
        let tx = self
            .chain
            .settle(
                &payment.scheme,
                &payment.payload.from,
                &payment.payload.to,
                settled_amount,
                payment.payload.valid_before,
                &payment.payload.nonce,
            )
            .map_err(|err| match err.as_str() {
                "replay" => Rejection::Replay,
                "expired" => Rejection::Expired,
                "insufficient_funds" => Rejection::InsufficientFunds,
                _ => Rejection::Malformed,
            })?;
        Ok(Settlement {
            success: true,
            transaction: tx,
            network: NETWORK.to_owned(),
            payer: payment.payload.from.to_ascii_lowercase(),
            settled_amount,
        })
    }
}

impl FacilitatorServer {
    pub(crate) async fn bind(host: IpAddr, facilitator: Facilitator) -> Result<Self> {
        let listener = TcpListener::bind((host, 0))
            .await
            .context("binding the x402 facilitator stand-in")?;
        Ok(Self {
            listener,
            facilitator,
        })
    }

    pub(crate) fn address(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .context("reading the facilitator's address")
    }

    pub(crate) async fn run(self, tracker: TaskTracker, stop: CancellationToken) {
        loop {
            let accepted = tokio::select! {
                () = stop.cancelled() => return,
                accepted = self.listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else {
                continue;
            };
            let facilitator = self.facilitator.clone();
            tracker.spawn(async move {
                let service = service_fn(move |request| {
                    let facilitator = facilitator.clone();
                    async move { Ok::<_, Infallible>(handle_http(facilitator, request).await) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FacilitatorRequest {
    payment_payload: PaymentPayload,
    payment_requirements: PaymentRequirements,
}

async fn handle_http(
    facilitator: Facilitator,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    match (request.method(), request.uri().path()) {
        (&Method::GET, "/supported") => {
            let accounts = facilitator.chain().accounts();
            let kinds: Vec<Value> = SCHEMES
                .iter()
                .map(|scheme| {
                    json!({
                        "x402Version": 2,
                        "scheme": scheme,
                        "network": accounts.network,
                        "asset": accounts.token,
                    })
                })
                .collect();
            json_response(StatusCode::OK, &json!({ "kinds": kinds }))
        }
        (&Method::POST, "/verify") => {
            let Some(req) = read_body(request).await else {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &json!({"isValid": false, "invalidReason": "malformed"}),
                );
            };
            match facilitator.verify(&req.payment_payload, &req.payment_requirements) {
                Ok(amount) => json_response(
                    StatusCode::OK,
                    &json!({
                        "isValid": true,
                        "payer": req.payment_payload.payload.from,
                        "settledAmount": amount.to_string(),
                    }),
                ),
                Err(reason) => json_response(
                    StatusCode::OK,
                    &json!({
                        "isValid": false,
                        "invalidReason": reason.code(),
                    }),
                ),
            }
        }
        (&Method::POST, "/settle") => {
            let Some(req) = read_body(request).await else {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &json!({"success": false, "errorReason": "malformed"}),
                );
            };
            match facilitator.settle(&req.payment_payload, &req.payment_requirements) {
                Ok(settlement) => json_response(
                    StatusCode::OK,
                    &serde_json::to_value(settlement)
                        .expect("invariant: settlement serializes to JSON"),
                ),
                Err(reason) => json_response(
                    StatusCode::OK,
                    &json!({
                        "success": false,
                        "errorReason": reason.code(),
                    }),
                ),
            }
        }
        _ => json_response(StatusCode::NOT_FOUND, &json!({"error": "not_found"})),
    }
}

async fn read_body(request: Request<Incoming>) -> Option<FacilitatorRequest> {
    let collected = Limited::new(request.into_body(), BODY_LIMIT)
        .collect()
        .await
        .ok()?;
    serde_json::from_slice(&collected.to_bytes()).ok()
}

fn json_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(value).expect("invariant: JSON value serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("invariant: JSON response builds")
}
