//! OpenTelemetry's trace and span IDs, drawn from an [`Entropy`] of their
//! own (ADRs 0059 and 0060), so a test's seed fixes the IDs it exports.
#![expect(
    clippy::disallowed_types,
    reason = "OpenTelemetry's ID generator is synchronous and called from any thread: a draw holds the lock for one generator step, with nothing else under it, which no loom model needs (ADR 0060)"
)]

use std::sync::{Mutex, PoisonError};

use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::IdGenerator;

use crate::entropy::Entropy;

/// Trace and span IDs drawn from an [`Entropy`]: never all zeros, which
/// OpenTelemetry reads as no ID.
#[derive(Debug)]
pub struct EntropyIds(Mutex<Entropy>);

impl EntropyIds {
    /// IDs drawn from `entropy`, a fork no other owner draws from.
    pub fn new(entropy: Entropy) -> Self {
        Self(Mutex::new(entropy))
    }

    /// `N` draws of 64 bits, in one hold of the lock.
    fn draw<const N: usize>(&self) -> [u64; N] {
        let mut entropy = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::array::from_fn(|_| entropy.next_u64())
    }
}

impl IdGenerator for EntropyIds {
    fn new_trace_id(&self) -> TraceId {
        loop {
            let [high, low] = self.draw();
            let id = (u128::from(high) << 64) | u128::from(low);
            if id != 0 {
                return TraceId::from(id);
            }
        }
    }

    fn new_span_id(&self) -> SpanId {
        loop {
            let [id] = self.draw();
            if id != 0 {
                return SpanId::from(id);
            }
        }
    }
}
