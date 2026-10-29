//! A Run's kept files through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0001, 0002, 0085, 0116 and
//! 0122): a Download's file, which an area keeps with its Run as the
//! browser gives it (a contribution of the tests' here, until the page's
//! downloads keep theirs), is saved whole at the program's path after its
//! Run ended, and after a resume of its Run on another machine; a path the
//! program cannot write raises `path_not_writable` and a full filesystem
//! `file_write_failed`, side `program`, the path keeping what it held and
//! no temporary file left beside it; a Daemon writes no path of the
//! program's and sends the file in `$/chunk` notifications, one per
//! credit; a file kept while the Run folder's filesystem is at `critical`
//! fails with `file_write_failed`, side `engine`; and a discarded Run's
//! files are gone from the Goodfetch home in either layout.

use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::download::DownloadId;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::checkpoint::two_machines_on;
use super::disk_view::DiskView;
use super::events::credit;
use super::layout::{per_install_on, per_run_on};
use super::run::call;
use super::storage::configure;
use super::{attached_on, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::engine::Engine;
use crate::run_files::{self, FileKind};
use crate::run_files_operations::keep_error;

/// Keeps a Download's file in a live Run, as the page's downloads will.
const KEEP_DOWNLOAD: Operation = Operation {
    name: "test.keep_download",
    description: "Keeps `text`, `times` over, as a Download's file of a live Run, `wait` milliseconds after it found the Run.",
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

async fn keep_download(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let times = usize::try_from(params["times"].as_u64().unwrap_or(1)).unwrap();
    let bytes = params["text"].as_str().unwrap().repeat(times).into_bytes();
    let run = context.runs().live(run_id.clone()).await.unwrap();
    if let Some(wait) = params["wait"].as_u64() {
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
    let kept = run
        .keep_file(
            FileKind::Download,
            "report.csv".to_owned(),
            Cursor::new(bytes),
        )
        .await
        .map_err(|error| keep_error(&run_id, run.folder(), error))?;
    Ok(json!({
        "download_id": DownloadId::of(&run_id, kept.position),
        "sha256": kept.sha256,
        "size": kept.size,
    }))
}

/// Keeps the tests' file in a live Run beside a `forget` of the same bytes
/// ordered before its record, as a Key-value store key's replaced file is
/// forgotten: the writer runs the `forget` once the keep has written its
/// bytes, then the keep's record.
const KEEP_BESIDE_FORGET: Operation = Operation {
    name: "test.keep_beside_forget",
    description: "Keeps the tests' file twice, the first record dropped and its file forgotten in the writer just before the second's record.",
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

/// A source of `bytes` that says, as it drops, that the keep reading it
/// has written them.
struct Signalling {
    bytes: Cursor<Vec<u8>>,
    written: Option<std::sync::mpsc::Sender<()>>,
}

impl std::io::Read for Signalling {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.bytes.read(buffer)
    }
}

impl Drop for Signalling {
    fn drop(&mut self) {
        if let Some(written) = self.written.take() {
            let _ = written.send(());
        }
    }
}

async fn keep_beside_forget(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = context.runs().live(run_id).await.unwrap();
    let first = run
        .keep_file(
            FileKind::Download,
            "first.csv".to_owned(),
            Cursor::new(bytes(1)),
        )
        .await
        .unwrap();
    let (written, wait) = std::sync::mpsc::channel::<()>();
    let held = run.state().write(move |_| {
        let _ = wait.recv();
        Ok(())
    });
    let (folder, sha256, position) = (run.folder().to_owned(), first.sha256, first.position);
    let forgotten = run.state().write(move |connection| {
        run_files::unrecord(connection, position)?;
        run_files::forget(connection, &folder, &sha256)
    });
    let source = Signalling {
        bytes: Cursor::new(bytes(1)),
        written: Some(written),
    };
    let (held, forgotten, kept) = tokio::join!(
        held,
        forgotten,
        run.keep_file(FileKind::Download, "second.csv".to_owned(), source)
    );
    held.unwrap();
    forgotten.unwrap();
    let kept = kept.unwrap();
    let file = run_files::file(run.folder(), &kept.sha256).unwrap();
    Ok(json!({ "kept": file.exists() }))
}

fn keeping() -> Contribution {
    Contribution::new("test-run-files")
        .with_handler(Handler::new(&KEEP_DOWNLOAD, keep_download))
        .with_handler(Handler::new(&KEEP_BESIDE_FORGET, keep_beside_forget))
}

fn engine() -> Engine {
    engine_with(Product::Collie, Some(keeping()))
}

/// The lines of the file the tests keep: 26 bytes.
const TEXT: &str = "id,name\n1,Ada\n2,Grace\n3,Al";

/// What the tests' file is when kept `times` over.
fn bytes(times: usize) -> Vec<u8> {
    TEXT.repeat(times).into_bytes()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The names in `folder` that start with a dot: temporary files left there.
fn hidden_in(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default()
}

/// Starts the Run `run_id`, keeps the tests' file `times` over in it and
/// finishes the Run, answering the Download's ID.
async fn kept_then_finished(client: &mut super::Client, run_id: &str, times: u64) -> String {
    client.send(&[hello("collie", "0.1")]).await;
    client.next().await;
    call(client, 1, "run.start", json!({ "run_id": run_id })).await;
    let kept = call(
        client,
        2,
        "test.keep_download",
        json!({ "run_id": run_id, "text": TEXT, "times": times }),
    )
    .await;
    call(client, 3, "run.finish", json!({ "run_id": run_id })).await;
    kept["result"]["download_id"].as_str().unwrap().to_owned()
}

/// A Download's file the Run kept is saved at the program's path once the
/// Run ended, its missing folders made, whole, with its size and SHA-256,
/// and no temporary file left beside it.
///
/// Covers: story:6-17, story:6-18, operation:download.save_as
#[test]
fn a_download_kept_with_its_run_is_saved_whole_after_the_run_ended() {
    let env = TestEnv::new();
    let path = env.folder().join("saved files").join("réport.csv");
    let asked = path.display().to_string();

    let saved = attached_on(&engine(), &env, |mut client| async move {
        let download_id = kept_then_finished(&mut client, "kept", 1).await;
        call(
            &mut client,
            4,
            "download.save_as",
            json!({ "download_id": download_id, "path": asked }),
        )
        .await
    });

    assert_eq!(
        (
            &saved["result"],
            std::fs::read(&path).ok(),
            hidden_in(path.parent().unwrap()),
        ),
        (
            &json!({ "size": 26, "sha256": sha256(&bytes(1)), "written": true }),
            Some(bytes(1)),
            Vec::<String>::new(),
        ),
        "{saved}"
    );
}

/// A file kept while a `forget` of the same bytes, ordered before its
/// record, deletes the file those bytes had is kept all the same: the
/// record never names a file that is gone (deep review 580-8).
///
/// Covers: story:6-17
#[test]
fn a_file_kept_beside_a_forget_of_the_same_bytes_is_kept() {
    let env = TestEnv::new();

    let kept = attached_on(&engine(), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "run.start", json!({ "run_id": "kept" })).await;
        let kept = call(
            &mut client,
            2,
            "test.keep_beside_forget",
            json!({ "run_id": "kept" }),
        )
        .await;
        call(&mut client, 3, "run.finish", json!({ "run_id": "kept" })).await;
        kept
    });

    assert_eq!(kept["result"], json!({ "kept": true }), "{kept}");
}

/// A save over a file the path held replaces it.
///
/// Covers: story:6-18, operation:download.save_as
#[test]
fn a_save_replaces_the_file_the_path_held() {
    let env = TestEnv::new();
    let path = env.folder().join("report.csv");
    std::fs::write(&path, b"an earlier export, longer than the download is").unwrap();
    let asked = path.display().to_string();

    attached_on(&engine(), &env, |mut client| async move {
        let download_id = kept_then_finished(&mut client, "replaces", 1).await;
        call(
            &mut client,
            4,
            "download.save_as",
            json!({ "download_id": download_id, "path": asked }),
        )
        .await
    });

    assert_eq!(std::fs::read(&path).unwrap(), bytes(1));
}

/// Sets up a path the program cannot write, in `folder`, and answers it.
#[cfg(unix)]
type Unwritable = fn(&Path) -> PathBuf;

#[cfg(unix)]
fn a_folder_at_the_path(folder: &Path) -> PathBuf {
    let path = folder.join("report.csv");
    std::fs::create_dir_all(path.join("inside")).unwrap();
    path
}

#[cfg(unix)]
fn a_file_where_a_folder_should_be(folder: &Path) -> PathBuf {
    std::fs::write(folder.join("exports"), b"a file").unwrap();
    folder.join("exports").join("report.csv")
}

#[cfg(unix)]
fn a_folder_the_user_cannot_write(folder: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let locked = folder.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("report.csv"), b"what it held").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    locked.join("report.csv")
}

/// A path the program cannot write raises `path_not_writable` with its
/// reason, the OS's name of the error and the path as given, and the path
/// keeps what it held, with no temporary file beside it.
#[cfg(unix)]
#[rstest]
/// Covers: story:6-18, error:path_not_writable
#[case::a_folder_at_the_path(a_folder_at_the_path, "is_a_directory", "EISDIR")]
/// Covers: story:6-18, error:path_not_writable
#[case::a_file_where_a_folder_should_be(
    a_file_where_a_folder_should_be,
    "not_a_directory",
    "ENOTDIR"
)]
/// Covers: story:6-18, error:path_not_writable
#[case::a_folder_the_user_cannot_write(
    a_folder_the_user_cannot_write,
    "permission_denied",
    "EACCES"
)]
fn a_path_the_program_cannot_write_raises_path_not_writable_and_keeps_what_it_held(
    #[case] unwritable: Unwritable,
    #[case] reason: &str,
    #[case] os_code: &str,
) {
    let env = TestEnv::new();
    let path = unwritable(env.folder());
    let folder = path.parent().unwrap().to_owned();
    let before = (std::fs::read(&path).ok(), hidden_in(&folder));
    let asked = path.display().to_string();
    let given = asked.clone();

    let failed = attached_on(&engine(), &env, |mut client| async move {
        let download_id = kept_then_finished(&mut client, "refused", 1).await;
        call(
            &mut client,
            4,
            "download.save_as",
            json!({ "download_id": download_id, "path": given }),
        )
        .await
    });

    let data = &failed["error"]["data"];
    let after = (std::fs::read(&path).ok(), hidden_in(&folder));
    // The test's folder is removed once the test passes: give its user back
    // the right to write the folder a case locked.
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(
        (
            (&data["code"], &data["retryable"], &data["reason"]),
            (&data["os_code"], &data["path"]),
            after,
        ),
        (
            (&json!("path_not_writable"), &json!(false), &json!(reason)),
            (&json!(os_code), &json!(asked)),
            before,
        ),
        "{failed}"
    );
}

/// A filesystem full under the save fails it with `file_write_failed`,
/// side `program`, reason `storage_full`, and the path keeps the file it
/// held byte for byte, with no temporary file beside it. The filesystem is
/// a tmpfs of 64 KiB in a user and mount namespace of a process of the
/// test's (`ci/os-services.sh setup`'s `unshare`), reached through its
/// `/proc/<pid>/root`.
///
/// Covers: story:6-18, error:file_write_failed
#[cfg(target_os = "linux")]
#[test]
fn a_full_filesystem_fails_the_save_with_file_write_failed_and_keeps_the_earlier_file() {
    let env = TestEnv::new();
    let full = SmallFilesystem::mount(&env, 64 * 1024);
    let path = full.inside("report.csv");
    std::fs::write(&path, b"what it held").unwrap();
    let asked = path.display().to_string();
    let given = asked.clone();

    let failed = attached_on(&engine(), &env, |mut client| async move {
        let download_id = kept_then_finished(&mut client, "full", 10_000).await;
        call(
            &mut client,
            4,
            "download.save_as",
            json!({ "download_id": download_id, "path": given }),
        )
        .await
    });

    let data = &failed["error"]["data"];
    let held = (std::fs::read(&path).ok(), hidden_in(path.parent().unwrap()));
    drop(full);
    assert_eq!(
        (
            (&data["code"], &data["retryable"], &data["reason"]),
            (&data["os_code"], &data["side"], &data["path"]),
            held,
        ),
        (
            (
                &json!("file_write_failed"),
                &json!(false),
                &json!("storage_full")
            ),
            (&json!("ENOSPC"), &json!("program"), &json!(asked)),
            (Some(b"what it held".to_vec()), Vec::<String>::new()),
        ),
        "{failed}"
    );
}

/// A tmpfs of a fixed size over a folder of the test's, mounted in a user
/// and mount namespace of a `sleep` the test starts and kills.
#[cfg(target_os = "linux")]
struct SmallFilesystem {
    sleeper: std::process::Child,
    folder: PathBuf,
}

#[cfg(target_os = "linux")]
impl SmallFilesystem {
    /// The test-only `unshare` that `ci/os-services.sh setup` installs.
    const TEST_UNSHARE: &str = "/usr/local/libexec/goodfetch-test-tools/unshare";

    #[expect(
        clippy::disallowed_methods,
        reason = "a test waits for the mount it started, outside any async worker"
    )]
    fn mount(env: &TestEnv, size: u64) -> Self {
        let folder = env.folder().join("small");
        std::fs::create_dir_all(&folder).unwrap();
        let unshare = if Path::new(Self::TEST_UNSHARE).exists() {
            Self::TEST_UNSHARE
        } else {
            "unshare"
        };
        let sleeper = std::process::Command::new(unshare)
            .args(["--user", "--map-root-user", "--mount", "sh", "-c"])
            .arg("mount -t tmpfs -o size=\"$2\" tmpfs \"$1\" && touch \"$1/.mounted\" && exec sleep 600")
            .arg("sh")
            .arg(&folder)
            .arg(size.to_string())
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mounted = Self { sleeper, folder };
        let marker = mounted.inside(".mounted");
        for _ in 0..500 {
            if marker.exists() {
                std::fs::remove_file(&marker).unwrap();
                return mounted;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "`{unshare}` mounted no tmpfs over {}: run `ci/os-services.sh setup`",
            mounted.folder.display()
        );
    }

    /// The file `name` of the tmpfs, as the test's process reaches it.
    fn inside(&self, name: &str) -> PathBuf {
        let relative = self.folder.strip_prefix("/").unwrap_or(&self.folder);
        Path::new("/proc")
            .join(self.sleeper.id().to_string())
            .join("root")
            .join(relative)
            .join(name)
    }
}

#[cfg(target_os = "linux")]
impl Drop for SmallFilesystem {
    fn drop(&mut self) {
        let _ = self.sleeper.kill();
        let _ = self.sleeper.wait();
    }
}

/// A Daemon given the program's path writes nothing there, and sends the
/// file in `$/chunk` notifications of at most 64 KiB, none before a
/// credit and one per credit, then answers it was not written; the chunks
/// are the file.
///
/// Covers: story:6-20, operation:download.save_as
#[test]
fn a_daemon_writes_no_path_and_sends_the_file_one_chunk_per_credit() {
    let env = TestEnv::new();
    let path = env.folder().join("report.csv");
    let asked = path.display().to_string();
    // 6,000 copies of the 26 bytes: two chunks of 64 KiB and a short one.
    let times = 6_000;

    let (before_credit, after_one, rest, answer) =
        per_install_on(&engine(), &env, |mut talk| async move {
            talk.ok(1, "run.start", json!({ "run_id": "remote" })).await;
            let kept = talk
                .ok(
                    2,
                    "test.keep_download",
                    json!({ "run_id": "remote", "text": TEXT, "times": times }),
                )
                .await;
            talk.ok(3, "run.finish", json!({ "run_id": "remote" }))
                .await;
            let client = talk.client();
            client
                .send(&[json!({
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "download.save_as",
                    "params": { "download_id": kept["download_id"], "path": asked },
                })])
                .await;
            let before_credit = tokio::time::timeout(Duration::from_secs(5), client.next())
                .await
                .is_err();
            client.send(&[credit(4, 1)]).await;
            let after_one = within(client).await;
            let quiet = tokio::time::timeout(Duration::from_secs(5), client.next())
                .await
                .is_err();
            client.send(&[credit(4, 10)]).await;
            let mut rest = Vec::new();
            let answer = loop {
                let message = within(client).await;
                if message["id"] == json!(4) {
                    break message;
                }
                rest.push(message);
            };
            (before_credit && quiet, after_one, rest, answer)
        });

    let chunks: Vec<&Value> = std::iter::once(&after_one).chain(&rest).collect();
    let received: Vec<u8> = chunks
        .iter()
        .flat_map(|chunk| {
            data_encoding::BASE64
                .decode(chunk["params"]["data"].as_str().unwrap().as_bytes())
                .unwrap()
        })
        .collect();
    let offsets: Vec<&Value> = chunks
        .iter()
        .map(|chunk| &chunk["params"]["offset"])
        .collect();
    assert_eq!(
        (
            before_credit,
            offsets,
            received == bytes(times),
            &answer["result"],
            path.exists(),
        ),
        (
            true,
            vec![&json!(0), &json!(65_536), &json!(131_072)],
            true,
            &json!({ "size": 156_000, "sha256": sha256(&bytes(times)), "written": false }),
            false,
        ),
        "{answer}"
    );
}

/// The next message of `client`, which must come within a minute on the
/// paused clock.
async fn within(client: &mut super::Client) -> Value {
    tokio::time::timeout(Duration::from_secs(60), client.next())
        .await
        .expect("the engine sent nothing for a minute")
        .expect("the engine closed the Connection")
}

/// A Download's file kept by a Run is saved whole after the Run was
/// checkpointed on one machine and resumed from that Checkpoint on
/// another, which never saw the Run's folder.
///
/// Covers: story:6-17, operation:download.save_as, operation:run.resume
#[test]
fn a_download_is_saved_whole_after_its_run_resumed_on_another_machine() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let path = second.folder().join("report.csv");
    let asked = path.display().to_string();

    let saved = two_machines_on(
        &engine(),
        &first,
        &second,
        |mut here, mut there| async move {
            here.ok(2, "run.start", json!({ "run_id": "moved" })).await;
            let kept = here
                .ok(
                    3,
                    "test.keep_download",
                    json!({ "run_id": "moved", "text": TEXT, "times": 3 }),
                )
                .await;
            let paused = here.ok(4, "run.pause", json!({ "run_id": "moved" })).await;
            there
                .ok(
                    2,
                    "run.resume",
                    json!({ "run_id": "moved", "checkpoint": paused["checkpoint"]["location"] }),
                )
                .await;
            let saved = there
                .call(
                    3,
                    "download.save_as",
                    json!({ "download_id": kept["download_id"], "path": asked }),
                )
                .await;
            there
                .ok(4, "run.finish", json!({ "run_id": "moved" }))
                .await;
            saved
        },
    );

    assert_eq!(
        (&saved["result"]["written"], std::fs::read(&path).ok()),
        (&json!(true), Some(bytes(3))),
        "{saved}"
    );
}

/// An empty Download's file, kept by a Run, is carried by its Checkpoint
/// like any other: after a resume from it on another machine, the resumed
/// Run checkpoints again and the file is saved, empty.
///
/// Covers: story:6-17, operation:download.save_as, operation:run.checkpoint
#[test]
fn an_empty_download_survives_a_checkpoint_and_a_resume_on_another_machine() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let path = second.folder().join("report.csv");
    let asked = path.display().to_string();

    let (checkpointed, saved) = two_machines_on(
        &engine(),
        &first,
        &second,
        |mut here, mut there| async move {
            here.ok(2, "run.start", json!({ "run_id": "empty" })).await;
            let kept = here
                .ok(
                    3,
                    "test.keep_download",
                    json!({ "run_id": "empty", "text": TEXT, "times": 0 }),
                )
                .await;
            let paused = here.ok(4, "run.pause", json!({ "run_id": "empty" })).await;
            there
                .ok(
                    2,
                    "run.resume",
                    json!({ "run_id": "empty", "checkpoint": paused["checkpoint"]["location"] }),
                )
                .await;
            let checkpointed = there
                .call(3, "run.checkpoint", json!({ "run_id": "empty" }))
                .await;
            let saved = there
                .call(
                    4,
                    "download.save_as",
                    json!({ "download_id": kept["download_id"], "path": asked }),
                )
                .await;
            there
                .ok(5, "run.finish", json!({ "run_id": "empty" }))
                .await;
            (checkpointed, saved)
        },
    );

    assert_eq!(
        (
            checkpointed.get("error"),
            &saved["result"]["written"],
            std::fs::read(&path).ok()
        ),
        (None, &json!(true), Some(Vec::new())),
        "{checkpointed} {saved}"
    );
}

/// While the Run folder's filesystem is at the `critical` storage level, a
/// file the Run keeps fails at once with `file_write_failed`, side
/// `engine`, reason `storage_full`, naming the folder, and nothing of it
/// is written there (ADR 0122). The filesystem crosses `critical` while
/// the keeping holds the live Run, which the crossing then stops.
///
/// Covers: story:6-17, error:file_write_failed, setting:storage.critical_free
#[test]
fn a_file_kept_while_the_run_folder_is_at_critical_fails_and_writes_nothing() {
    let env = TestEnv::new();
    configure(
        &env,
        "storage.low_free = 2000000\nstorage.critical_free = 1000000\n",
    );
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100_000_000, 50_000_000);
    let filled = view.clone();
    let engine = engine().with_governor(view.system());
    let folder = env.home().join("1").join("runs").join("full");

    let failed = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "run.start", json!({ "run_id": "full" })).await;
        client
            .send(&[json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "test.keep_download",
                "params": { "run_id": "full", "text": TEXT, "wait": 1_000 },
            })])
            .await;
        filled.set_free(disk, 500_000);
        loop {
            let message = client.next().await.unwrap();
            if message["id"] == json!(2) {
                break message;
            }
        }
    });

    let data = &failed["error"]["data"];
    assert_eq!(
        (
            (&data["code"], &data["side"], &data["reason"]),
            &data["path"],
            folder.join("files").exists(),
        ),
        (
            (
                &json!("file_write_failed"),
                &json!("engine"),
                &json!("storage_full")
            ),
            &json!(folder.display().to_string()),
            false,
        ),
        "{failed}"
    );
}

/// Which layout of run state a discard runs in.
type Layout = fn(&Engine, &TestEnv, String, String) -> (Value, Value);

fn discarded_per_run(engine: &Engine, env: &TestEnv, text: String, path: String) -> (Value, Value) {
    per_run_on(engine, env, |talk| discard_after_keeping(talk, text, path))
}

fn discarded_per_install(
    engine: &Engine,
    env: &TestEnv,
    text: String,
    path: String,
) -> (Value, Value) {
    per_install_on(engine, env, |talk| discard_after_keeping(talk, text, path))
}

/// Keeps `text` as a Download of the Run `discarded`, ends and discards
/// the Run, then asks to save the Download at `path`; answers the discard's
/// and the save's replies.
async fn discard_after_keeping(
    mut talk: super::layout::Talk,
    text: String,
    path: String,
) -> (Value, Value) {
    talk.ok(1, "run.start", json!({ "run_id": "discarded" }))
        .await;
    let kept = talk
        .ok(
            2,
            "test.keep_download",
            json!({ "run_id": "discarded", "text": text }),
        )
        .await;
    talk.ok(3, "run.finish", json!({ "run_id": "discarded" }))
        .await;
    let discarded = talk
        .reply(4, "run.discard", json!({ "run_id": "discarded" }))
        .await;
    let saved = talk
        .reply(
            5,
            "download.save_as",
            json!({ "download_id": kept["download_id"], "path": path }),
        )
        .await;
    (discarded, saved)
}

/// A discarded Run's Download is gone with it in either layout: its bytes,
/// a Secret canary, are found nowhere in the Goodfetch home, and saving it
/// raises `run_not_found`, writing nothing.
#[rstest]
/// Covers: story:6-22, operation:run.discard
#[case::per_run(discarded_per_run)]
/// Covers: story:6-22, operation:run.discard, setting:run_state.layout
#[case::per_install(discarded_per_install)]
fn a_discarded_runs_downloads_are_gone_from_the_home(#[case] layout: Layout) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let path = env.folder().join("report.csv");

    let (discarded, saved) = layout(&engine(), &env, token, path.display().to_string());

    let found: BTreeSet<&str> = canaries
        .search(&[Output::Path(env.home())])
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect();
    assert_eq!(
        (
            &discarded["result"],
            found,
            &saved["error"]["data"]["code"],
            path.exists(),
        ),
        (
            &json!({ "run_id": "discarded" }),
            BTreeSet::new(),
            &json!("run_not_found"),
            false,
        ),
        "{saved}"
    );
}
