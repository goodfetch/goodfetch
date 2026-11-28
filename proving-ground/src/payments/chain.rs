//! Local chain stand-in for Paid fetches (#1537, ADRs 0045 and 0076):
//! starts an in-process Ethereum-compatible JSON-RPC HTTP server on loopback
//! with deterministic seeded accounts, tracks EIP-3009 authorization nonces,
//! Permit2 nonce bitmaps, token balances and block timestamps, and never
//! touches any external network.
#![expect(
    clippy::disallowed_types,
    reason = "the local chain stand-in opens its own loopback listener outside the engine transport (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use data_encoding::HEXLOWER;
use hmac::{Hmac, Mac as _};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::interface::ChainAccounts;
use crate::sync::Mutex;

/// Standard local EVM chain ID (matches Anvil).
pub(crate) const CHAIN_ID: u64 = 31_337;

/// CAIP-2 network identifier for the local chain.
pub(crate) const NETWORK: &str = "eip155:31337";

/// Initial funded USDC balance for the seeded payer (1,000.000000 USDC).
const INITIAL_PAYER_BALANCE: u128 = 1_000_000_000;

/// Initial deterministic block timestamp (2026-01-01T00:00:00Z).
pub(crate) const INITIAL_TIMESTAMP: u64 = 1_767_225_600;

/// Maximum JSON-RPC request body size.
const BODY_LIMIT: usize = 256 * 1024;

type HmacSha256 = Hmac<Sha256>;

/// The settlement status of a payment nonce on the local chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NonceState {
    Settled,
    Pending,
    Expired,
}

/// Recorded settlement or pending payment on the local chain.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct NonceRecord {
    pub(crate) scheme: String,
    pub(crate) payer: String,
    pub(crate) payee: String,
    pub(crate) amount: u128,
    pub(crate) valid_before: u64,
    pub(crate) state: NonceState,
    pub(crate) tx_hash: Option<String>,
}

/// Shared mutable state of the local chain.
struct State {
    block_number: u64,
    block_timestamp: u64,
    balances: BTreeMap<String, u128>,
    nonces: BTreeMap<String, NonceRecord>,
    receipts: BTreeMap<String, Value>,
    tx_counter: u64,
}

/// Handle to the local chain, shared by the JSON-RPC server, the facilitator
/// stand-in, and the paid pages / oracle.
#[derive(Clone)]
pub(crate) struct Chain {
    accounts: ChainAccounts,
    state: Arc<Mutex<State>>,
}

/// Bound local chain HTTP JSON-RPC server.
pub(crate) struct ChainServer {
    listener: TcpListener,
    chain: Chain,
}

/// Parameters covered by a local-chain x402 payment signature.
pub(crate) struct SigningInput<'a> {
    pub(crate) scheme: &'a str,
    pub(crate) network: &'a str,
    pub(crate) from: &'a str,
    pub(crate) to: &'a str,
    pub(crate) value: u128,
    pub(crate) valid_before: u64,
    pub(crate) nonce: &'a str,
}

impl Chain {
    /// Creates a new local chain with accounts deterministically derived from `seed`.
    pub(crate) fn new(seed: u64) -> Self {
        let accounts = derive_accounts(seed);
        let mut balances = BTreeMap::new();
        balances.insert(accounts.payer_address.clone(), INITIAL_PAYER_BALANCE);
        balances.insert(accounts.payee_address.clone(), 0);
        Self {
            accounts,
            state: Arc::new(Mutex::new(State {
                block_number: 1,
                block_timestamp: INITIAL_TIMESTAMP,
                balances,
                nonces: BTreeMap::new(),
                receipts: BTreeMap::new(),
                tx_counter: 0,
            })),
        }
    }

    pub(crate) fn accounts(&self) -> &ChainAccounts {
        &self.accounts
    }

    pub(crate) fn block_timestamp(&self) -> u64 {
        self.state.lock().block_timestamp
    }

    /// Computes the canonical signature for an x402 payment payload using `key_hex`.
    pub(crate) fn sign_payment(key_hex: &str, input: &SigningInput<'_>) -> Result<String, String> {
        let key = decode_hex_key(key_hex)?;
        let mut mac =
            HmacSha256::new_from_slice(&key).map_err(|_| "invalid signing key".to_owned())?;
        let message = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            input.scheme,
            input.network,
            input.from.to_ascii_lowercase(),
            input.to.to_ascii_lowercase(),
            input.value,
            input.valid_before,
            input.nonce,
        );
        mac.update(message.as_bytes());
        Ok(format!(
            "0x{}",
            HEXLOWER.encode(&mac.finalize().into_bytes())
        ))
    }

    /// Verifies that `signature` was signed by the seeded payer account's key.
    pub(crate) fn verify_signature(&self, input: &SigningInput<'_>, signature: &str) -> bool {
        if !input
            .from
            .eq_ignore_ascii_case(&self.accounts.payer_address)
        {
            return false;
        }
        let Ok(expected) = Self::sign_payment(&self.accounts.payer_key, input) else {
            return false;
        };
        expected.eq_ignore_ascii_case(signature)
    }

    /// Checks whether `nonce` has already been settled or expired.
    pub(crate) fn nonce_record(&self, nonce: &str) -> Option<NonceRecord> {
        self.state
            .lock()
            .nonces
            .get(&nonce.to_ascii_lowercase())
            .cloned()
    }

    /// Records a payment as pending (not yet settled, still able to settle).
    pub(crate) fn record_pending(
        &self,
        scheme: &str,
        sender: &str,
        recipient: &str,
        amount: u128,
        valid_before: u64,
        nonce: &str,
    ) {
        let key = nonce.to_ascii_lowercase();
        let mut state = self.state.lock();
        state.nonces.insert(
            key,
            NonceRecord {
                scheme: scheme.to_owned(),
                payer: sender.to_ascii_lowercase(),
                payee: recipient.to_ascii_lowercase(),
                amount,
                valid_before,
                state: NonceState::Pending,
                tx_hash: None,
            },
        );
    }

    /// Marks a payment nonce as expired and advances the chain's block timestamp
    /// past `valid_before` so on-chain expiry checks prove it cannot settle.
    pub(crate) fn expire_payment(
        &self,
        scheme: &str,
        sender: &str,
        recipient: &str,
        amount: u128,
        valid_before: u64,
        nonce: &str,
    ) {
        let key = nonce.to_ascii_lowercase();
        let mut state = self.state.lock();
        if state.block_timestamp <= valid_before {
            state.block_timestamp = valid_before.saturating_add(1);
        }
        state.block_number = state.block_number.saturating_add(1);
        state.nonces.insert(
            key,
            NonceRecord {
                scheme: scheme.to_owned(),
                payer: sender.to_ascii_lowercase(),
                payee: recipient.to_ascii_lowercase(),
                amount,
                valid_before,
                state: NonceState::Expired,
                tx_hash: None,
            },
        );
    }

    /// Settles a verified payment on the local chain, updating balances,
    /// marking `nonce` settled, and mining a new block.
    pub(crate) fn settle(
        &self,
        scheme: &str,
        sender: &str,
        recipient: &str,
        amount: u128,
        valid_before: u64,
        nonce: &str,
    ) -> Result<String, String> {
        let sender_key = sender.to_ascii_lowercase();
        let recipient_key = recipient.to_ascii_lowercase();
        let nonce_key = nonce.to_ascii_lowercase();
        let mut state = self.state.lock();

        if let Some(existing) = state.nonces.get(&nonce_key) {
            match existing.state {
                NonceState::Settled => return Err("replay".to_owned()),
                NonceState::Expired => return Err("expired".to_owned()),
                NonceState::Pending => {}
            }
        }
        if state.block_timestamp >= valid_before {
            return Err("expired".to_owned());
        }
        let balance = state.balances.get(&sender_key).copied().unwrap_or(0);
        if balance < amount {
            return Err("insufficient_funds".to_owned());
        }
        state.balances.insert(sender_key.clone(), balance - amount);
        let recipient_balance = state.balances.get(&recipient_key).copied().unwrap_or(0);
        state.balances.insert(
            recipient_key.clone(),
            recipient_balance.saturating_add(amount),
        );

        state.block_number = state.block_number.saturating_add(1);
        state.tx_counter = state.tx_counter.saturating_add(1);
        let tx_hash = format!(
            "0x{}",
            HEXLOWER.encode(&Sha256::digest(format!(
                "tx:{}:{nonce_key}:{amount}",
                state.tx_counter
            )))
        );
        let receipt = json!({
            "transactionHash": tx_hash,
            "blockNumber": format!("0x{:x}", state.block_number),
            "status": "0x1",
            "from": sender_key,
            "to": self.accounts.token,
            "amount": amount.to_string(),
            "scheme": scheme,
        });
        state.receipts.insert(tx_hash.clone(), receipt);
        state.nonces.insert(
            nonce_key,
            NonceRecord {
                scheme: scheme.to_owned(),
                payer: sender_key,
                payee: recipient_key,
                amount,
                valid_before,
                state: NonceState::Settled,
                tx_hash: Some(tx_hash.clone()),
            },
        );
        Ok(tx_hash)
    }

    /// Handles one Ethereum JSON-RPC 2.0 request.
    fn rpc(&self, method: &str, params: &[Value]) -> Result<Value, (i64, String)> {
        match method {
            "eth_chainId" => Ok(json!(format!("0x{CHAIN_ID:x}"))),
            "eth_blockNumber" => {
                let block = self.state.lock().block_number;
                Ok(json!(format!("0x{block:x}")))
            }
            "eth_getBlockByNumber" => {
                let state = self.state.lock();
                Ok(json!({
                    "number": format!("0x{:x}", state.block_number),
                    "timestamp": format!("0x{:x}", state.block_timestamp),
                }))
            }
            "eth_getBalance" => {
                let address = params
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let balance = self
                    .state
                    .lock()
                    .balances
                    .get(&address)
                    .copied()
                    .unwrap_or(0);
                Ok(json!(format!("0x{balance:x}")))
            }
            "eth_getTransactionReceipt" => {
                let hash = params.first().and_then(Value::as_str).unwrap_or_default();
                let receipt = self.state.lock().receipts.get(hash).cloned();
                Ok(receipt.unwrap_or(Value::Null))
            }
            "eth_call" => Ok(self.eth_call(params)),
            "evm_mine" => {
                let mut state = self.state.lock();
                state.block_number = state.block_number.saturating_add(1);
                state.block_timestamp = state.block_timestamp.saturating_add(1);
                Ok(json!(format!("0x{:x}", state.block_number)))
            }
            "evm_increaseTime" => {
                let delta = params.first().and_then(Value::as_u64).unwrap_or(1);
                let mut state = self.state.lock();
                state.block_timestamp = state.block_timestamp.saturating_add(delta);
                Ok(json!(format!("0x{:x}", state.block_timestamp)))
            }
            "proving_ground_nonceState" => {
                let nonce = params
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let state = self.state.lock();
                let record = state.nonces.get(&nonce);
                Ok(json!({
                    "block_timestamp": state.block_timestamp,
                    "record": record,
                }))
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        }
    }

    /// Minimal ABI decoder for EIP-3009 `authorizationState(address,bytes32)`,
    /// Permit2 `nonceBitmap(address,uint256)`, and ERC-20 `balanceOf(address)`.
    fn eth_call(&self, params: &[Value]) -> Value {
        let data = params
            .first()
            .and_then(|call| call.get("data").or_else(|| call.get("input")))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let hex = data.strip_prefix("0x").unwrap_or(data).to_ascii_lowercase();
        let state = self.state.lock();
        // authorizationState(address,bytes32): selector e94a0102
        // nonceBitmap(address,uint256): selector 4fe02b44
        if (hex.starts_with("e94a0102") || hex.starts_with("4fe02b44")) && hex.len() >= 8 + 128 {
            let nonce_hex = format!("0x{}", &hex[8 + 64..8 + 128]);
            let settled = state
                .nonces
                .get(&nonce_hex)
                .is_some_and(|record| record.state == NonceState::Settled);
            let word = u8::from(settled);
            return json!(format!("0x{:064x}", word));
        }
        // balanceOf(address): selector 70a08231
        if hex.starts_with("70a08231") && hex.len() >= 8 + 64 {
            let address = format!("0x{}", &hex[8 + 24..8 + 64]);
            let balance = state.balances.get(&address).copied().unwrap_or(0);
            return json!(format!("0x{balance:064x}"));
        }
        json!(format!("0x{:064x}", 0u8))
    }
}

impl ChainServer {
    /// Binds the local chain JSON-RPC server on `(host, 0)`.
    pub(crate) async fn bind(host: IpAddr, chain: Chain) -> Result<Self> {
        let listener = TcpListener::bind((host, 0))
            .await
            .context("binding the local chain JSON-RPC server")?;
        Ok(Self { listener, chain })
    }

    pub(crate) fn address(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .context("reading the local chain's address")
    }

    /// Serves JSON-RPC requests until `stop` is cancelled.
    pub(crate) async fn run(self, tracker: TaskTracker, stop: CancellationToken) {
        loop {
            let accepted = tokio::select! {
                () = stop.cancelled() => return,
                accepted = self.listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else {
                continue;
            };
            let chain = self.chain.clone();
            tracker.spawn(async move {
                let service = service_fn(move |request| {
                    let chain = chain.clone();
                    async move { Ok::<_, Infallible>(handle_http(chain, request).await) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }
}

#[derive(Deserialize)]
struct RpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Vec<Value>,
}

async fn handle_http(chain: Chain, request: Request<Incoming>) -> Response<Full<Bytes>> {
    if request.method() != Method::POST {
        return json_response(
            StatusCode::METHOD_NOT_ALLOWED,
            &json!({"error": "method_not_allowed"}),
        );
    }
    let Ok(collected) = Limited::new(request.into_body(), BODY_LIMIT)
        .collect()
        .await
    else {
        return json_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &json!({"error": "body_too_large"}),
        );
    };
    let bytes = collected.to_bytes();
    let Ok(rpc) = serde_json::from_slice::<RpcRequest>(&bytes) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}),
        );
    };
    let body = match chain.rpc(&rpc.method, &rpc.params) {
        Ok(result) => json!({
            "jsonrpc": "2.0",
            "id": rpc.id,
            "result": result,
        }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": rpc.id,
            "error": {"code": code, "message": message},
        }),
    };
    json_response(StatusCode::OK, &body)
}

fn json_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(value).expect("invariant: JSON value serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("invariant: JSON response builds")
}

fn derive_accounts(seed: u64) -> ChainAccounts {
    let hash = |label: &str| -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"goodfetch-proving-ground-chain:");
        hasher.update(seed.to_le_bytes());
        hasher.update(b":");
        hasher.update(label.as_bytes());
        hasher.finalize().into()
    };
    let addr = |label: &str| -> String {
        let digest = hash(label);
        format!("0x{}", HEXLOWER.encode(&digest[..20]))
    };
    let payer_key = format!("0x{}", HEXLOWER.encode(&hash("payer-key")));
    ChainAccounts {
        chain_id: CHAIN_ID,
        network: NETWORK.to_owned(),
        token: addr("usdc-token"),
        permit2: addr("permit2"),
        payer_address: addr("payer"),
        payer_key,
        payee_address: addr("payee"),
    }
}

fn decode_hex_key(key_hex: &str) -> Result<Vec<u8>, String> {
    let trimmed = key_hex
        .strip_prefix("0x")
        .or_else(|| key_hex.strip_prefix("0X"))
        .unwrap_or(key_hex);
    HEXLOWER
        .decode(trimmed.to_ascii_lowercase().as_bytes())
        .map_err(|_| "invalid hex key".to_owned())
}
