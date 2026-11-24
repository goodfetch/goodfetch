//! The Scripted model: answers call *n* with reply *n*, records each
//! request, and fails the test when the calls and the script part ways.
#![expect(
    clippy::disallowed_types,
    reason = "a test adapter's script: one lock guards it for one call's step or one read, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use futures_util::stream::{self, BoxStream, StreamExt as _};
use goodfetch_model_provider::{CallContext, ModelProvider};
use goodfetch_protocol::model::{ModelRequest, ModelResponsePart};

use crate::reply::Reply;

/// A Model provider that answers call *n* with reply *n* of its script.
/// A call beyond the script ends with [`ScriptExhausted`]; that call, or a
/// reply left unused, fails the test at [`ScriptedModel::assert_done`] or,
/// when the test does not call it, as the model is dropped.
#[derive(Debug)]
pub struct ScriptedModel {
    script: Mutex<Script>,
}

#[derive(Debug)]
struct Script {
    /// The replies not given yet, the next first.
    replies: VecDeque<Reply>,
    /// How many replies the script held.
    length: usize,
    /// Every request received, in order.
    requests: Vec<ModelRequest>,
}

impl Script {
    /// How many calls came after the last reply.
    fn beyond(&self) -> usize {
        self.requests.len().saturating_sub(self.length)
    }

    /// Why the calls and the script parted ways, if they did.
    fn mismatch(&self) -> Option<String> {
        let beyond = self.beyond();
        if beyond > 0 {
            return Some(format!(
                "ScriptedModel: {beyond} {} beyond its script of {} {}",
                plural(beyond, "call", "calls"),
                self.length,
                plural(self.length, "reply", "replies"),
            ));
        }
        let unused = self.replies.len();
        (unused > 0).then(|| {
            format!(
                "ScriptedModel: {unused} of its {} {} unused",
                self.length,
                plural(self.length, "reply", "replies"),
            )
        })
    }
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// A call came after the script's last reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptExhausted {
    /// The call's place, from 1.
    pub call: usize,
    /// How many replies the script holds.
    pub replies: usize,
}

impl fmt::Display for ScriptExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "call {} came beyond the Scripted model's script of {} {}",
            self.call,
            self.replies,
            plural(self.replies, "reply", "replies"),
        )
    }
}

impl Error for ScriptExhausted {}

impl ScriptedModel {
    /// A model that answers its calls with `replies`, in order.
    pub fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        let replies: VecDeque<Reply> = replies.into_iter().collect();
        Self {
            script: Mutex::new(Script {
                length: replies.len(),
                replies,
                requests: Vec::new(),
            }),
        }
    }

    /// Every request received, in the order it came, those beyond the
    /// script included.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.script().requests.clone()
    }

    /// Fails the test when a call came beyond the script or a reply is
    /// unused.
    ///
    /// # Panics
    ///
    /// When the calls and the script parted ways.
    pub fn assert_done(&self) {
        let mismatch = self.script().mismatch();
        if let Some(mismatch) = mismatch {
            panic!("{mismatch}");
        }
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        // A test that panicked holding the lock left the script whole: each
        // step under it is one push and one pop.
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ModelProvider for ScriptedModel {
    fn call<'a>(
        &'a self,
        request: &'a ModelRequest,
        _context: CallContext<'a>,
    ) -> BoxStream<'a, Result<ModelResponsePart, Box<dyn Error + Send + Sync>>> {
        let parts = {
            let mut script = self.script();
            script.requests.push(request.clone());
            match script.replies.pop_front() {
                Some(reply) => reply.into_parts(),
                None => vec![Err(Box::new(ScriptExhausted {
                    call: script.requests.len(),
                    replies: script.length,
                }) as Box<dyn Error + Send + Sync>)],
            }
        };
        stream::iter(parts).boxed()
    }
}

impl Drop for ScriptedModel {
    /// Fails the test as [`ScriptedModel::assert_done`] does, unless it is
    /// failing already.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let mismatch = self.script().mismatch();
        if let Some(mismatch) = mismatch {
            panic!("{mismatch}");
        }
    }
}

#[cfg(test)]
mod tests;
