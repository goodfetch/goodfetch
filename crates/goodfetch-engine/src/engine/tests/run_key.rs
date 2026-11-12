//! Run state's field encryption through the engine's public interface over
//! the in-memory network (spec #564's `run_key`; ADRs 0002 and 0015): a
//! value a Run seals into a secret table is found in neither its run file,
//! its WAL, its free pages nor its Checkpoint, and opens back in the Run;
//! the Run's data key is made when it first seals, wrapped by the
//! key-encryption key `run_state.key_reference` names, whose default item,
//! in the encrypted local file on a host with no OS keychain, the engine
//! makes then. A Checkpoint moved to another machine resumes only when the
//! caller names that key over JSON-RPC: without it the resume fails naming
//! `run_state.key_reference`, and the Checkpoint holds no key.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the run files and the encrypted local file the engine wrote"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::{Arc, Weak};

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_protocol::secrets::RUN_STATE_KEY_REFERENCE;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use serde_json::{Value, json};

use super::checkpoint::Talk;
use super::{Client, attached_with, dependencies, engine_with};
use crate::audit::AuditLog;
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::{Dependencies, Engine};
use crate::home::Home;
use crate::run_key::{self, sealing};
use crate::run_state::RunStateTables;
use crate::runs::{EndedBy, Run, Start, Started};

/// Starts a Collie Run its program drives, which the tests seal values in;
/// with `opens_settings`, its settings opened as a start opens them.
const KEEP: Operation = Operation {
    name: "test.keep",
    description: "Starts a Run the tests seal values in.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Seals `value` into the secret table's row `row` of a live Run, and
/// answers its data key in hexadecimal, which the tests search files for.
const SEAL: Operation = Operation {
    name: "test.seal",
    description: "Seals a value into a Run's secret table.",
    ..KEEP
};

/// Opens the secret table's row `row` of a live Run.
const OPEN: Operation = Operation {
    name: "test.open",
    description: "Opens a value of a Run's secret table.",
    ..KEEP
};

/// Stores a data key for a live Run that has made none, wrapped by the
/// key-encryption key `kek`, and seals `value` into row `row` with it, as
/// a making whose caller was dropped once its write was queued leaves it.
const STORED_KEY: Operation = Operation {
    name: "test.stored_key",
    description: "Stores a Run's wrapped data key as a dropped making leaves it.",
    ..KEEP
};

/// The secret table the tests seal values in.
const SECRETS: RunStateTables = RunStateTables {
    name: "test-secrets",
    statements: &[
        "CREATE TABLE IF NOT EXISTS test_secrets (row INTEGER NOT NULL PRIMARY KEY, value BLOB NOT NULL) STRICT",
    ],
};

/// The Runs `test.keep` started, each held by a task of its own, which its
/// stop ends.
type Kept = Arc<tokio::sync::Mutex<BTreeMap<String, Weak<Run>>>>;

/// A Collie Run's settings, opened as a start opens them, when `params`
/// ask for them.
async fn opened(params: &Value, context: &CallContext) -> Option<crate::settings::RunSettings> {
    if params["opens_settings"] != true {
        return None;
    }
    let opening = crate::settings::Opening {
        run: Some(Product::Collie),
        ..crate::settings::Opening::default()
    };
    Some(context.engine().settings().open(opening).await.unwrap().0)
}

fn sealing() -> Contribution {
    let kept = Kept::default();
    Contribution::new("test-run-key")
        .with_run_state_tables(SECRETS)
        .with_handler(Handler::new(
            &KEEP,
            move |params: Value, context: CallContext| {
                let kept = Arc::clone(&kept);
                async move {
                    let settings = opened(&params, &context).await;
                    let started = context
                        .runs()
                        .start(Start {
                            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
                            tags: Tags::default(),
                            kind: RunKind::Collie,
                            needs: Product::Collie,
                            inputs: BTreeMap::new(),
                            ended_by: EndedBy::Finish,
                            initialize: None,
                            policy_version: None,
                            weakened: Vec::new(),
                            binding: None,
                            clean_up_digest: None,
                            checkpoint_interval: None,
                            timeout: None,
                            settings,
                        })
                        .await
                        .unwrap();
                    let Started::New(run) = started else {
                        panic!("the Run began before");
                    };
                    let held = Arc::new(run.clone());
                    kept.lock()
                        .await
                        .insert(run.id().to_string(), Arc::downgrade(&held));
                    run.spawn(async move {
                        held.cancellation().cancelled().await;
                    });
                    Ok::<Value, CallError>(json!({ "run_id": run.id().to_string() }))
                }
            },
        ))
        .with_handler(Handler::new(
            &SEAL,
            |params: Value, context: CallContext| async move {
                let run = live(&context, &params).await;
                let row = params["row"].as_i64().unwrap();
                let key = run.key().await.map_err(|error| no_key(&error))?;
                let sealed = key
                    .seal(
                        "test_secrets",
                        "value",
                        &row.to_string(),
                        params["value"].as_str().unwrap().as_bytes(),
                    )
                    .unwrap();
                let data_key = data_encoding::HEXLOWER.encode(data_key(&run).await.as_slice());
                run.state()
                    .write(move |connection| {
                        connection
                            .execute(
                                "INSERT INTO test_secrets (row, value) VALUES (?1, ?2)
                                 ON CONFLICT (row) DO UPDATE SET value = excluded.value",
                                (row, sealed),
                            )
                            .map(drop)
                    })
                    .await
                    .unwrap();
                Ok::<Value, CallError>(json!({ "data_key": data_key }))
            },
        ))
        .with_handler(stored_key())
        .with_handler(Handler::new(
            &OPEN,
            |params: Value, context: CallContext| async move {
                let run = live(&context, &params).await;
                let row = params["row"].as_i64().unwrap();
                let sealed: Vec<u8> = run
                    .state()
                    .read(move |connection| {
                        connection.query_row(
                            "SELECT value FROM test_secrets WHERE row = ?1",
                            [row],
                            |found| found.get(0),
                        )
                    })
                    .await
                    .unwrap();
                let key = run.key().await.map_err(|error| no_key(&error))?;
                let opened = key
                    .open("test_secrets", "value", &row.to_string(), &sealed)
                    .unwrap();
                Ok::<Value, CallError>(json!({
                    "value": String::from_utf8(opened.to_vec()).unwrap()
                }))
            },
        ))
}

/// The handler of `test.stored_key`.
fn stored_key() -> Handler {
    Handler::new(
        &STORED_KEY,
        |params: Value, context: CallContext| async move {
            let run = live(&context, &params).await;
            let row = params["row"].as_i64().unwrap();
            let kek = secrecy::SecretString::from(params["kek"].as_str().unwrap().to_owned());
            let key = run_key::RunKey::generate(run.id().clone()).unwrap();
            let stored = run_key::StoredKey {
                wrapped: key.wrap(&kek).unwrap(),
                key_reference: "secretfile:run-state-key".to_owned(),
            };
            let sealed = key
                .seal(
                    "test_secrets",
                    "value",
                    &row.to_string(),
                    params["value"].as_str().unwrap().as_bytes(),
                )
                .unwrap();
            run.state()
                .write(move |connection| {
                    run_key::store(connection, &stored)?;
                    connection
                        .execute(
                            "INSERT INTO test_secrets (row, value) VALUES (?1, ?2)",
                            (row, sealed),
                        )
                        .map(drop)
                })
                .await
                .unwrap();
            Ok::<Value, CallError>(json!({}))
        },
    )
}

/// The failure a call answers when its Run has no data key, so a test
/// fails on its reply rather than waiting for one.
fn no_key(error: &crate::runs::RunKeyError) -> CallError {
    CallError::protocol(
        goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
        error.to_string(),
    )
}

/// The live Run the call's `run_id` names.
/// The data key of `run`, unwrapped from what its run state stores by the
/// key-encryption key the Run holds, as the Run itself unwraps it.
async fn data_key(run: &Run) -> secrecy::zeroize::Zeroizing<[u8; sealing::KEY_LENGTH]> {
    let stored = run
        .state()
        .read(run_key::stored)
        .await
        .unwrap()
        .expect("the Run stored its key");
    let kek = run
        .secrets()
        .held_for(RUN_STATE_KEY_REFERENCE.name)
        .await
        .expect("the Run holds its key-encryption key");
    sealing::unwrap(run.id().as_str(), stored.wrapped.as_bytes(), &kek).unwrap()
}

async fn live(context: &CallContext, params: &Value) -> Run {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    context.runs().live(run_id).await.unwrap()
}

fn engine() -> Engine {
    engine_with(Product::Collie, Some(sealing()))
}

/// Talks to the engine on `dependencies` while `talk` runs.
fn on<D, F, Fut, T>(dependencies: D, talk: F) -> T
where
    D: FnOnce() -> Dependencies,
    F: FnOnce(Talk) -> Fut,
    Fut: Future<Output = T>,
{
    attached_with(&engine(), dependencies, |client: Client| async move {
        talk(Talk::open(client).await).await
    })
}

/// The dependencies of the engine of another machine, `env`'s home, whose
/// environment holds `variables`.
fn another_machine(env: &TestEnv, variables: &[(&str, &str)]) -> Dependencies {
    Dependencies {
        environment: variables
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect(),
        entropy: crate::entropy::Entropy::seeded(env.seed() ^ 1),
        ..dependencies(env)
    }
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

/// The key-encryption key the engine made in `env`'s encrypted local file
/// `run-state-key`, read as `age -d -i` reads it.
fn made_key(env: &TestEnv) -> String {
    let folder = env.home().join("secrets");
    let identity = std::fs::read_to_string(folder.join("identity.txt")).unwrap();
    let identity = identity
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-"))
        .map(|line| age::x25519::Identity::from_str(line).unwrap())
        .unwrap();
    let file = std::fs::read(folder.join("run-state-key.age")).unwrap();
    let decryptor = age::Decryptor::new_buffered(&file[..]).unwrap();
    let mut key = String::new();
    decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap()
        .read_to_string(&mut key)
        .unwrap();
    key.trim_end().to_owned()
}

/// Every place of `files` that holds one of `secrets`, each a name and its
/// bytes.
fn found_in(files: &[&Path], secrets: &[(&str, &[u8])]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        let bytes = std::fs::read(file).unwrap_or_default();
        for (name, secret) in secrets {
            if bytes.windows(secret.len()).any(|window| window == *secret) {
                found.push(format!("{name} in {}", file.display()));
            }
        }
    }
    found
}

/// The owner-only bits of the identity file the engine made.
#[cfg(unix)]
fn shared_bits(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o077
}

/// A value sealed and overwritten leaves its plaintext in no byte of the
/// run file, its WAL or its Checkpoint, free pages included, and the data
/// key and the key-encryption key appear in none of them; the value opens
/// back in the Run. The engine made the default key-encryption key in the
/// encrypted local file, with an identity only its owner may read.
/// Covers: story:4-64, setting:run_state.key_reference
#[test]
fn a_sealed_value_is_in_no_byte_of_the_run_file_or_its_checkpoint_and_opens_back() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let first = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let second = canaries.plant(SecretKind::SessionCookie).value().to_owned();

    let (sealed_first, sealed_second) = (first.clone(), second.clone());
    let (data_key, opened) = on(
        || dependencies(&env),
        |mut talk| async move {
            let (first, second) = (sealed_first, sealed_second);
            talk.ok(2, "test.keep", json!({ "run_id": "sealed" })).await;
            let sealed = talk
                .ok(
                    3,
                    "test.seal",
                    json!({ "run_id": "sealed", "row": 1, "value": first }),
                )
                .await;
            talk.ok(
                4,
                "test.seal",
                json!({ "run_id": "sealed", "row": 1, "value": second }),
            )
            .await;
            talk.ok(5, "run.checkpoint", json!({ "run_id": "sealed" }))
                .await;
            let opened = talk
                .ok(6, "test.open", json!({ "run_id": "sealed", "row": 1 }))
                .await;
            talk.ok(7, "run.finish", json!({ "run_id": "sealed" }))
                .await;
            (sealed["data_key"].as_str().unwrap().to_owned(), opened)
        },
    );

    let folder = run_folder(&env, "sealed");
    let data_key = data_encoding::HEXLOWER.decode(data_key.as_bytes()).unwrap();
    let kek = made_key(&env);
    let files = [
        folder.join("run.sqlite"),
        folder.join("run.sqlite-wal"),
        folder.join("checkpoint.sqlite"),
    ];
    let files: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    #[cfg(unix)]
    assert_eq!(
        shared_bits(&env.home().join("secrets").join("identity.txt")),
        0
    );
    assert_eq!(
        (
            opened,
            canaries
                .leaks(&[Output::Path(&folder)])
                .unwrap()
                .into_iter()
                .map(|hit| format!("{} in {}", hit.canary, hit.output))
                .collect::<Vec<_>>(),
            found_in(
                &files,
                &[
                    ("data key", &data_key),
                    ("key-encryption key", kek.as_bytes())
                ]
            ),
        ),
        (json!({ "value": second }), Vec::new(), Vec::<String>::new())
    );
}

/// A Run whose data key's making was dropped after its wrapped key was
/// written, so its run state holds a key its Run never took, opens with
/// that key at its next use instead of failing to store a second one.
/// Covers: story:4-64
#[test]
fn a_key_a_dropped_making_stored_is_the_one_the_run_opens_with_next() {
    let env = TestEnv::new();
    let made = &env;
    let opened = on(
        || dependencies(&env),
        |mut talk| async move {
            talk.ok(2, "test.keep", json!({ "run_id": "first" })).await;
            talk.ok(
                3,
                "test.seal",
                json!({ "run_id": "first", "row": 1, "value": "made the engine's key" }),
            )
            .await;
            talk.ok(4, "test.keep", json!({ "run_id": "dropped" }))
                .await;
            talk.ok(
                5,
                "test.stored_key",
                json!({
                    "run_id": "dropped",
                    "row": 1,
                    "value": "sealed by the dropped making's key",
                    "kek": made_key(made),
                }),
            )
            .await;
            let opened = talk
                .call(6, "test.open", json!({ "run_id": "dropped", "row": 1 }))
                .await;
            talk.ok(7, "run.finish", json!({ "run_id": "dropped" }))
                .await;
            talk.ok(8, "run.finish", json!({ "run_id": "first" })).await;
            opened
        },
    );

    assert_eq!(
        opened.get("result"),
        Some(&json!({ "value": "sealed by the dropped making's key" })),
        "{opened}"
    );
}

/// Seals `cookie` in the Run `moved` on `env`'s engine, then migrates it,
/// and answers its data key and its Checkpoint's location.
fn sealed_and_migrated(env: &TestEnv, cookie: String) -> (Vec<u8>, PathBuf) {
    let (data_key, checkpoint) = on(
        || dependencies(env),
        |mut talk| async move {
            talk.ok(2, "test.keep", json!({ "run_id": "moved" })).await;
            let sealed = talk
                .ok(
                    3,
                    "test.seal",
                    json!({ "run_id": "moved", "row": 1, "value": cookie }),
                )
                .await;
            let migrated = talk
                .ok(4, "run.migrate", json!({ "run_id": "moved" }))
                .await;
            (
                sealed["data_key"].as_str().unwrap().to_owned(),
                migrated["checkpoint"]["location"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        },
    );
    (
        data_encoding::HEXLOWER.decode(data_key.as_bytes()).unwrap(),
        PathBuf::from(checkpoint),
    )
}

/// What `run.resume` takes to resume `moved` from the Checkpoint `copied`,
/// with the project configuration `project`.
fn resume_from(copied: &Path, project: Option<&Value>) -> Value {
    let mut params = json!({ "run_id": "moved", "checkpoint": copied.display().to_string() });
    if let Some(project) = project {
        params["project"] = project.clone();
    }
    params
}

/// A refusal's details, without its `fix`, and whether it had one.
fn refusal(reply: &Value) -> (Value, bool) {
    let mut data = reply["error"]["data"].clone();
    let fix = data.as_object_mut().and_then(|data| data.remove("fix"));
    (data, fix.is_some_and(|fix| fix.is_string()))
}

/// A Run that sealed a value, migrated and moved to another machine,
/// resumes there only when the caller names the key-encryption key over
/// JSON-RPC, as project configuration: with no key there, not even the
/// encrypted local file's identity, which a resume never makes, the resume
/// fails with `secret_unavailable` naming `run_state.key_reference`, and
/// with that machine's own key, which is another, with `run_key_mismatch`;
/// the Checkpoint holds neither key. Named, the key opens the Run's value.
/// Covers: story:4-66, operation:run.resume, error:secret_unavailable, error:run_key_mismatch
#[test]
fn a_checkpoint_resumes_on_another_machine_only_with_the_key_the_caller_names() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let mut canaries = Canaries::new(first.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();

    let (data_key, checkpoint) = sealed_and_migrated(&first, cookie.clone());
    let kek = made_key(&first);
    let copied = second.folder().join("moved.sqlite");
    std::fs::copy(&checkpoint, &copied).unwrap();
    let named = json!({
        "path": second.folder().join("goodfetch.toml").display().to_string(),
        "settings": { "run_state": { "key_reference": "env:GF_TEST_RUN_KEY" } },
    });

    let without_key = on(
        || another_machine(&second, &[]),
        |mut talk| {
            let request = resume_from(&copied, None);
            async move { talk.call(2, "run.resume", request).await }
        },
    );
    let (with_its_own_key, with_the_key, opened) = on(
        || another_machine(&second, &[("GF_TEST_RUN_KEY", &kek)]),
        |mut talk| {
            let (own, named) = (
                resume_from(&copied, None),
                resume_from(&copied, Some(&named)),
            );
            async move {
                talk.ok(2, "test.keep", json!({ "run_id": "own" })).await;
                talk.ok(
                    5,
                    "test.seal",
                    json!({ "run_id": "own", "row": 1, "value": "own" }),
                )
                .await;
                let own = talk.call(3, "run.resume", own).await;
                let named = talk.call(4, "run.resume", named).await;
                let opened = talk
                    .ok(6, "test.open", json!({ "run_id": "moved", "row": 1 }))
                    .await;
                talk.ok(7, "run.finish", json!({ "run_id": "moved" })).await;
                talk.ok(8, "run.finish", json!({ "run_id": "own" })).await;
                (own, named, opened)
            }
        },
    );

    assert_eq!(
        (
            refusal(&without_key),
            refusal(&with_its_own_key),
            with_the_key["result"]["started_as"].clone(),
            opened,
            found_in(
                &[&checkpoint, &copied],
                &[
                    ("data key", &data_key),
                    ("key-encryption key", kek.as_bytes())
                ]
            ),
        ),
        (
            (
                json!({
                    "code": "secret_unavailable",
                    "placeholder": "run_state.key_reference",
                    "reference": "secretfile:run-state-key",
                    "provider": "secretfile",
                    "failure": "not_configured",
                    "retryable": false,
                }),
                true
            ),
            (
                json!({
                    "code": "run_key_mismatch",
                    "run_id": "moved",
                    "setting": "run_state.key_reference",
                    "reference": "secretfile:run-state-key",
                    "wrapped_by": "secretfile:run-state-key",
                    "retryable": false,
                }),
                true
            ),
            json!("resumed"),
            json!({ "value": cookie }),
            Vec::<String>::new(),
        )
    );
}

/// Where the host has an OS keychain, a Run's first seal makes the
/// engine's key there, never in an encrypted local file, and a paused Run
/// resumed by the engine started again unwraps its data key with it and
/// opens its value. keyring-core's mock store stands in for the keychain.
/// Covers: story:4-64, story:4-66, setting:run_state.key_reference
#[test]
fn with_an_os_keychain_the_key_comes_from_it_and_a_paused_run_resumes_with_it() {
    let env = TestEnv::new();
    let store: Arc<keyring_core::CredentialStore> = keyring_core::mock::Store::new().unwrap();
    let with_keychain = || Dependencies {
        keychain: crate::secrets::Keychain::of(Arc::clone(&store)),
        ..dependencies(&env)
    };

    on(with_keychain, |mut talk| async move {
        talk.ok(2, "test.keep", json!({ "run_id": "keyed" })).await;
        talk.ok(
            3,
            "test.seal",
            json!({ "run_id": "keyed", "row": 1, "value": "in the keychain" }),
        )
        .await;
        talk.ok(4, "run.pause", json!({ "run_id": "keyed" })).await;
    });
    let opened = on(with_keychain, |mut talk| async move {
        talk.ok(2, "run.resume", json!({ "run_id": "keyed" })).await;
        let opened = talk
            .ok(3, "test.open", json!({ "run_id": "keyed", "row": 1 }))
            .await;
        talk.ok(4, "run.finish", json!({ "run_id": "keyed" })).await;
        opened
    });

    let wrapped_by: String =
        rusqlite::Connection::open(run_folder(&env, "keyed").join("run.sqlite"))
            .unwrap()
            .query_row("SELECT key_reference FROM run_key", [], |row| row.get(0))
            .unwrap();
    assert_eq!(
        (opened, wrapped_by, env.home().join("secrets").exists()),
        (
            json!({ "value": "in the keychain" }),
            "keychain:goodfetch/run-state-key".to_owned(),
            false
        )
    );
}

/// A Run that opens its settings, as `crawl.start` and `run.start` do, and
/// makes no data key opens no OS keychain, on Linux a Secret Service
/// session over D-Bus: the keychain opens once, when the Run first makes
/// its key, here on a host where it does not open, so the key-encryption
/// key is the encrypted local file's, as before (#2204).
/// Covers: setting:run_state.key_reference
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "a test's counter, which no loom model runs (ADR 0060)"
)]
fn a_run_opens_the_os_keychain_only_once_it_first_makes_its_key() {
    let env = TestEnv::new();
    let opened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&opened);
    let keychain = crate::secrets::Keychain::opened_by(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    });
    let count = || opened.load(std::sync::atomic::Ordering::SeqCst);
    let folder = run_folder(&env, "lazy");

    let (started, sealed, wrapped_by) = on(
        || Dependencies {
            keychain,
            ..dependencies(&env)
        },
        |mut talk| async move {
            talk.ok(
                2,
                "test.keep",
                json!({ "run_id": "lazy", "opens_settings": true }),
            )
            .await;
            let started = count();
            talk.ok(
                3,
                "test.seal",
                json!({ "run_id": "lazy", "row": 1, "value": "sealed" }),
            )
            .await;
            let sealed = count();
            talk.ok(4, "run.finish", json!({ "run_id": "lazy" })).await;
            let wrapped_by: String = rusqlite::Connection::open(folder.join("run.sqlite"))
                .unwrap()
                .query_row("SELECT key_reference FROM run_key", [], |row| row.get(0))
                .unwrap();
            (started, sealed, wrapped_by)
        },
    );

    assert_eq!(
        (started, sealed, wrapped_by),
        (0, 1, "secretfile:run-state-key".to_owned())
    );
}

/// A Run resumed with `security.audit_chain` off writes its entries
/// unchained from the resume on, its key-encryption key's use included
/// (ADR 0016), and the log still verifies; the entries it wrote before,
/// while its chain was on, stay chained.
/// Covers: story:4-74, setting:security.audit_chain, operation:run.resume
#[test]
fn a_run_resumed_with_the_audit_chain_off_writes_its_entries_unchained() {
    let env = TestEnv::new();
    let store: Arc<keyring_core::CredentialStore> = keyring_core::mock::Store::new().unwrap();
    let with_keychain = || Dependencies {
        keychain: crate::secrets::Keychain::of(Arc::clone(&store)),
        ..dependencies(&env)
    };
    let used = || {
        let log = AuditLog::of(&Home::new(env.home().to_owned()), None);
        assert!(log.verify().unwrap().intact, "the Audit log verifies");
        log.export()
            .unwrap()
            .into_iter()
            .filter(|exported| exported.entry.kind == "secret_reference_used")
            .map(|exported| exported.entry.chained)
            .collect::<Vec<_>>()
    };
    on(with_keychain, |mut talk| async move {
        talk.ok(2, "test.keep", json!({ "run_id": "keyed" })).await;
        talk.ok(
            3,
            "test.seal",
            json!({ "run_id": "keyed", "row": 1, "value": "kept" }),
        )
        .await;
        talk.ok(4, "run.pause", json!({ "run_id": "keyed" })).await;
    });
    let before = used();

    on(with_keychain, |mut talk| async move {
        talk.ok(
            2,
            "run.resume",
            json!({ "run_id": "keyed", "settings": { "security": { "audit_chain": false } } }),
        )
        .await;
        talk.ok(3, "run.finish", json!({ "run_id": "keyed" })).await;
    });
    let after = used();

    assert!(
        !before.is_empty() && before.iter().all(|chained| *chained),
        "{before:?}"
    );
    assert_eq!(&after[before.len()..], &[false], "{after:?}");
}

/// Discarding a Run overwrites its wrapped data key in its run file before
/// the run folder goes (ADR 0015): a hard link to the run file, which keeps
/// its bytes as a disk would keep a removed file's, holds the wrapped key
/// before the discard and neither its row nor its bytes after, so what the
/// Run sealed there no longer opens under any key-encryption key.
/// Covers: story:3-39, operation:run.discard
#[test]
fn a_discarded_runs_wrapped_data_key_is_overwritten_in_its_run_file() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let linked = env.folder().join("linked run file.sqlite");
    let link_to = linked.clone();
    let folder = run_folder(&env, "sealed");
    let run_file = folder.join("run.sqlite");

    let (wrapped, discarded) = on(
        || dependencies(&env),
        |mut talk| async move {
            talk.ok(2, "test.keep", json!({ "run_id": "sealed" })).await;
            talk.ok(
                3,
                "test.seal",
                json!({ "run_id": "sealed", "row": 1, "value": cookie }),
            )
            .await;
            talk.ok(4, "run.finish", json!({ "run_id": "sealed" }))
                .await;
            std::fs::hard_link(&run_file, &link_to).unwrap();
            // Read through the run file's own path, whose WAL its writer left
            // beside it (#2117): the connection's close copies the WAL, the
            // key with it, into the file the link names too.
            let wrapped = wrapped_key(&run_file);
            let discarded = talk
                .ok(5, "run.discard", json!({ "run_id": "sealed" }))
                .await;
            (wrapped, discarded)
        },
    );

    let kept = std::fs::read(&linked).unwrap();
    assert_eq!(
        (
            discarded,
            wrapped.is_some(),
            rusqlite::Connection::open(&linked)
                .unwrap()
                .query_row("SELECT count(*) FROM run_key", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            wrapped
                .is_some_and(|wrapped| kept.windows(wrapped.len()).any(|window| window == wrapped)),
            folder.exists(),
        ),
        (json!({ "run_id": "sealed" }), true, 0, false, false)
    );
}

/// The wrapped data key the run file at `path` holds.
fn wrapped_key(path: &Path) -> Option<Vec<u8>> {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT wrapped FROM run_key", [], |row| row.get(0))
        .ok()
}
