//! A view of a Run another program started (ADR 0102), such as a detached
//! one. A Connection's `open_run` opens it by the Run's ID with no round
//! trip. It takes no writer role, reads `progress()`, `report()`, `trace()`,
//! `save_recording()` and the Key-value store's reads (`get`, `get_with`,
//! `get_bytes`, `keys`), and closing it frees only the view: the Run goes on.
//! Python's `run_view.py` and TypeScript's `run-view.ts` are its twins.

use std::future::IntoFuture as _;
use std::path::Path;
use std::pin::Pin;
use std::sync::PoisonError;
use std::task::{Context, Poll, ready};

use goodfetch_protocol::key_value_store::{
    KEY_VALUE_STORE_GET, KEY_VALUE_STORE_GET_BYTES, KEY_VALUE_STORE_KEYS, KeyValueStoreBytesValue,
    KeyValueStoreKeyParams, KeyValueStoreKeysParams, KeyValueStoreValue, Refused, VALUE_INVALID,
    ValueInvalidDetails, ValueInvalidReason,
};
use goodfetch_protocol::run::{
    RECORDING_SAVE_AS, RUN_PROGRESS_READ, RUN_REPORT, RecordingSaveAsParams, RecordingSaved, RunId,
    RunParams, RunProgress, RunProgressParams, RunReport,
};
use goodfetch_protocol::run_trace::{RUN_TRACE, RunTraceResult};
use goodfetch_protocol::schema_validator::{SCHEMA_VALIDATOR_FAILED, SchemaValidatorFailedDetails};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::answer::Answer;
use crate::call::{Call, Reply};
use crate::connection::{BlockingRuntime, ClientConnection, Core, SaveFile};
use crate::error::Error;
use crate::event_stream::EventStream;
use crate::markers::warn_operation;
use crate::run_copies::RunTrace;
use crate::schema::{self, Schema, SchemaError, ValidateError};
use crate::sync::Mutex;

/// The notification carrying one key from `key_value_store.keys`.
pub(crate) const KEY_NOTIFICATION: &str = "$/key";

/// One `$/key` notification payload.
#[derive(Deserialize)]
pub(crate) struct KeyNotification {
    pub(crate) key: String,
}

/// Maps `$/key` notification payloads to their key strings.
pub(crate) struct KeyStream(pub(crate) EventStream<KeyNotification>);

impl futures_core::Stream for KeyStream {
    type Item = Result<String, Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0)
            .poll_next(context)
            .map(|item| item.map(|result| result.map(|notice| notice.key)))
    }
}

/// Bytes a Key-value store holds for a key, with their content type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredBytes {
    /// The bytes.
    pub content: Vec<u8>,
    /// Their content type, as `set_bytes` gave it, such as `image/png`.
    pub content_type: String,
}

/// A Run named by its Run ID, opened as a view: one another program
/// started, such as a detached Run. It takes no writer role.
#[derive(Debug, Clone)]
#[must_use = "a view is the Run's ID; closing it frees only the view"]
pub struct RunView {
    core: Core,
    run_id: RunId,
}

impl RunView {
    /// The narrow runtime handle its blocking twin uses to wait on calls.
    pub(crate) fn runtime(&self) -> BlockingRuntime {
        self.core.runtime()
    }

    /// The Run's ID, which names it for good. Read without a round trip.
    pub fn id(&self) -> &RunId {
        &self.run_id
    }

    fn call<T, P>(&self, method: &'static str, params: P) -> Call<T, P>
    where
        T: DeserializeOwned + Send + 'static,
        P: Serialize + Send + 'static,
    {
        let params = if self.core.is_closed() {
            Err(Error::closed())
        } else {
            Ok(params)
        };
        Call::new(self.core.sender(), method, params)
    }

    /// Reads a Run's progress: what it does and why it waits. Without `after`,
    /// it answers at once; given `after`, the position of the value the reader
    /// last read, it waits for a change and answers the latest value, so values
    /// in between are coalesced. The value whose `last` is true ends the
    /// sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
    /// or no engine works it (`writer: none`) until a start with its Run ID
    /// resumes it. A Run another engine works is read through that engine, the
    /// owner the Run index names, and one no engine works from its committed
    /// state; a read never takes the Run's writer lock. A Run ID the Run index
    /// does not list fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn progress(&self) -> EventStream<Answer<RunProgress>> {
        warn_operation(RUN_PROGRESS_READ.name);
        EventStream::from_items(ViewProgress {
            core: self.core.clone(),
            run_id: self.run_id.clone(),
            after: None,
            read: None,
            ended: false,
        })
    }

    /// Reads a Run's report: at once for a Run that ended, and at its end for
    /// a live one, which the read never ends. A Run another engine works is
    /// read through that engine, and one no engine works from its committed
    /// state, never taking its writer lock. A Run that has no live writer and
    /// has not ended fails with `run_interrupted`, whose `fix` names a start
    /// with its Run ID, and a Run ID the Run index does not list with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn report(&self) -> Call<Answer<RunReport>, RunParams> {
        warn_operation(RUN_REPORT.name);
        self.call(
            RUN_REPORT.name,
            RunParams {
                run_id: self.run_id.clone(),
            },
        )
    }

    /// Reads a Run's trace copy as recorded so far, or null when the Run keeps
    /// no trace (ADRs 0102 and 0116). Returns only once the copy is whole,
    /// with its size and SHA-256 digest; a write the engine cannot make fails
    /// with `file_write_failed`, side `engine`, and leaves no unfinished file.
    /// Takes no writer lock, so a view reads a Run no program holds without
    /// resuming it. A Run ID the Run index does not list fails with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn trace(&self) -> Call<Option<RunTrace>, RunParams> {
        warn_operation(RUN_TRACE.name);
        let core = self.core.clone();
        self.call(
            RUN_TRACE.name,
            RunParams {
                run_id: self.run_id.clone(),
            },
        )
        .map(move |result: RunTraceResult| {
            result
                .trace
                .map(|descriptor| RunTrace::new(core, descriptor))
        })
    }

    /// Read-only view of the Run's Key-value store (ADRs 0102 and 0103):
    /// `get`, `get_with`, `get_bytes` and `keys`, taking no writer lock.
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStoreView {
        KeyValueStoreView {
            core: self.core.clone(),
            run_id: self.run_id.clone(),
        }
    }

    /// Saves a Run's recorded Response recording as a WARC 1.1 file, whole or
    /// not at all (ADRs 0105 and 0116). An Attached engine given the program's
    /// path writes it through a hidden temporary file beside it, synced and
    /// renamed over it, creating missing folders and replacing a file there;
    /// any other engine writes nothing on its own machine and sends the file
    /// in `$/chunk` notifications of at most 64 KiB, one per credit granted
    /// with `$/credit`, for the SDK to write. A Run that recorded no responses
    /// raises `recording_invalid`. A path that cannot be written raises
    /// `path_not_writable`, and full or failing storage `file_write_failed`,
    /// leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn save_recording(&self, path: impl AsRef<Path>) -> SaveFile<RecordingSaved> {
        warn_operation(RECORDING_SAVE_AS.name);
        let path = path.as_ref();
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.core.save_file(
            RECORDING_SAVE_AS.name,
            &RecordingSaveAsParams {
                run_id: self.run_id.clone(),
                path: None,
                filename,
            },
            path,
            "save_recording",
        )
    }

    /// Closes the view. Frees only the view: the Run goes on.
    pub fn close(self) {}
}

struct ViewProgress {
    core: Core,
    run_id: RunId,
    after: Option<u64>,
    read: Option<Mutex<Reply<Answer<RunProgress>>>>,
    ended: bool,
}

impl futures_core::Stream for ViewProgress {
    type Item = Result<Answer<RunProgress>, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let following = self.get_mut();
        if following.ended {
            return Poll::Ready(None);
        }
        let read = following.read.get_or_insert_with(|| {
            let params = if following.core.is_closed() {
                Err(Error::closed())
            } else {
                Ok(RunProgressParams {
                    run_id: following.run_id.clone(),
                    after: following.after,
                })
            };
            Mutex::new(
                Call::new(following.core.sender(), RUN_PROGRESS_READ.name, params).into_future(),
            )
        });
        let answer = ready!(
            read.get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
                .poll(context)
        );
        following.read = None;
        match &answer {
            Ok(value) if !value.last => following.after = Some(value.position),
            _ => following.ended = true,
        }
        Poll::Ready(Some(answer))
    }
}

/// A read-only view of a Run's Key-value store, reached through
/// [`RunView::key_value_store`] (ADRs 0102 and 0103): reads `get`, `get_with`,
/// `get_bytes` and `keys` without taking a writer lock.
#[derive(Clone, Debug)]
pub struct KeyValueStoreView {
    core: Core,
    run_id: RunId,
}

impl KeyValueStoreView {
    /// The narrow runtime handle its blocking twin uses to wait on calls.
    pub(crate) fn runtime(&self) -> BlockingRuntime {
        self.core.runtime()
    }

    fn call<T, P>(&self, method: &'static str, params: P) -> Call<T, P>
    where
        T: DeserializeOwned + Send + 'static,
        P: Serialize + Send + 'static,
    {
        let params = if self.core.is_closed() {
            Err(Error::closed())
        } else {
            Ok(params)
        };
        Call::new(self.core.sender(), method, params)
    }

    fn key_params(&self, key: String) -> KeyValueStoreKeyParams {
        KeyValueStoreKeyParams {
            run_id: self.run_id.clone(),
            key,
            attempt: None,
        }
    }

    /// Answers the JSON value a Run's Key-value store holds for a key, as
    /// committed, or null when the key is absent. A key holding bytes fails
    /// with `value_invalid`, naming `getBytes`. Takes no writer lock, so it
    /// reads a Run no program holds without resuming it. Through a crawl
    /// attempt (`attempt`), it reads what the attempt staged first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<T>, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_GET.name);
        let key = key.into();
        self.call(KEY_VALUE_STORE_GET.name, self.key_params(key.clone()))
            .try_map(move |held: KeyValueStoreValue| read_back(&key, held.value))
    }

    /// Answers the JSON value a Run's Key-value store holds for a key, as
    /// committed, or null when the key is absent, validated against `schema`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn get_with<T: Serialize + DeserializeOwned + Send + 'static>(
        &self,
        key: impl Into<String>,
        schema: &Schema<T>,
    ) -> Call<Option<T>, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_GET.name);
        let key = key.into();
        let schema = schema.clone();
        self.call(KEY_VALUE_STORE_GET.name, self.key_params(key.clone()))
            .try_map(move |held: KeyValueStoreValue| {
                if held.value.is_null() {
                    return Ok(None);
                }
                validate_stored(&key, &held.value, &schema)?;
                read_back(&key, held.value)
            })
    }

    /// Answers the bytes and content type a Run's Key-value store holds for a
    /// key, as committed, or null when the key is absent. A key holding JSON
    /// fails with `value_invalid`, naming `get`. Takes no writer lock. Through
    /// a crawl attempt (`attempt`), it reads what the attempt staged first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn get_bytes(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<StoredBytes>, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_GET_BYTES.name);
        self.call(KEY_VALUE_STORE_GET_BYTES.name, self.key_params(key.into()))
            .map(|held: KeyValueStoreBytesValue| {
                held.bytes.map(|bytes| StoredBytes {
                    content: bytes.content.into_bytes(),
                    content_type: bytes.content_type,
                })
            })
    }

    /// Sends the committed keys of a Run's Key-value store that start with
    /// `prefix`, in the order of their Unicode scalar values, in `$/key`
    /// notifications, one per credit granted with `$/credit`, then answers how
    /// many it sent. Takes no writer lock.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn keys(&self, prefix: Option<&str>) -> EventStream<String> {
        warn_operation(KEY_VALUE_STORE_KEYS.name);
        let params = KeyValueStoreKeysParams {
            run_id: self.run_id.clone(),
            prefix: prefix.map(str::to_owned),
        };
        let stream: EventStream<KeyNotification> =
            self.core
                .stream(KEY_VALUE_STORE_KEYS.name, &params, KEY_NOTIFICATION);
        EventStream::from_items(KeyStream(stream))
    }
}

/// Validates a stored Key-value store value against `schema`, distinguishing
/// an invalid schema from a value violation.
pub(crate) fn validate_stored<T: Serialize + DeserializeOwned>(
    key: &str,
    value: &Value,
    schema: &Schema<T>,
) -> Result<(), Error> {
    match schema.validate(value) {
        Ok(()) => Ok(()),
        Err(ValidateError::InvalidSchema(message)) => Err(invalid_schema(key, message)),
        Err(ValidateError::Violation { path, message }) => Err(invalid(
            key,
            &format!("The schema rejected the value stored for {key:?}: {message}."),
            "Read it with the schema it was stored under, or as `serde_json::Value`, \
             and store it again in the new shape.",
            ValueInvalidReason::ValueSchema,
            path,
        )),
    }
}

/// Validates a value being written to the Key-value store against `schema`,
/// distinguishing an invalid schema from a value violation.
pub(crate) fn validate_given<T: Serialize + DeserializeOwned>(
    key: &str,
    value: &Value,
    schema: &Schema<T>,
) -> Result<(), Error> {
    match schema.validate(value) {
        Ok(()) => Ok(()),
        Err(ValidateError::InvalidSchema(message)) => Err(invalid_schema(key, message)),
        Err(ValidateError::Violation { path, message }) => Err(invalid(
            key,
            &format!("The schema rejected the value given for {key:?}: {message}."),
            "Give a value the schema accepts.",
            ValueInvalidReason::ValueSchema,
            path,
        )),
    }
}

/// Reports a malformed JSON Schema distinctly from a value-schema violation
/// (`schema_validator_failed`, never `value_invalid` / `value_schema`).
fn invalid_schema(key: &str, message: String) -> Error {
    Error::of_sdk(
        &SCHEMA_VALIDATOR_FAILED,
        &format!(
            "The schema for Key-value store key {key:?} is not a valid JSON Schema: {message}."
        ),
        "Fix the JSON Schema passed to `Schema::json` so it compiles as a valid JSON Schema.",
        &SchemaValidatorFailedDetails {
            schema: None,
            path: None,
            exception_type: "InvalidSchema".to_owned(),
            exception_message: message,
        },
    )
}

/// What `get` answers of `held`, the value the engine answered for `key`:
/// null is an absent key, which the store never holds.
pub(crate) fn read_back<T: DeserializeOwned>(key: &str, held: Value) -> Result<Option<T>, Error> {
    if held.is_null() {
        return Ok(None);
    }
    schema::decode(held).map(Some).map_err(|error| {
        invalid(
            key,
            &format!("The type read does not accept the value stored for {key:?}: {error}."),
            "Read it as the type it was stored from, or as `serde_json::Value`, and store it \
             again in the new shape.",
            ValueInvalidReason::ValueSchema,
            String::new(),
        )
    })
}

/// The `value_invalid` of the value given for `key`, which `error` refused
/// before anything was sent.
pub(crate) fn refused(key: &str, error: &SchemaError) -> Error {
    let (reason, path) = match error {
        SchemaError::NotJson { path, .. } => (ValueInvalidReason::ValueNotJson, path.clone()),
        SchemaError::ChangesOnReadBack { path, .. } => {
            (ValueInvalidReason::ValueSchema, path.clone())
        }
        _ => (ValueInvalidReason::ValueSchema, String::new()),
    };
    invalid(
        key,
        &format!("The Key-value store refused the value of {key:?}: {error}."),
        "Give a value that is JSON and that its type's `Deserialize` reads back as it was given.",
        reason,
        path,
    )
}

/// The `value_invalid` of `key`'s value, refused for `reason` at `path`.
pub(crate) fn invalid(
    key: &str,
    what_failed: &str,
    fix: &str,
    reason: ValueInvalidReason,
    path: String,
) -> Error {
    Error::of_sdk(
        &VALUE_INVALID,
        what_failed,
        fix,
        &ValueInvalidDetails {
            key: key.to_owned(),
            refused: Refused::Value,
            reason,
            path: Some(path),
            fix: fix.to_owned(),
        },
    )
}

impl ClientConnection {
    /// Opens a view of the Run `run_id` names, one another program started
    /// such as a detached Run, with no round trip (ADR 0102). The view takes
    /// no writer role, and closing it frees only the view.
    ///
    /// # Errors
    ///
    /// `closed` once the Connection is closed, and `connection_forked` in
    /// another process than the one that opened it.
    pub fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        self.ensure_opener()?;
        if self.is_closed() {
            return Err(Error::closed());
        }
        Ok(RunView {
            core: self.core().clone(),
            run_id: run_id.clone(),
        })
    }
}

/// C-SEND-SYNC: a view moves between threads, is shared between them and
/// outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<RunView>();
    send_sync_static::<KeyValueStoreView>();
    send_sync_static::<StoredBytes>();
};
