//! The program's side of file saving (ADR 0116): a path written whole or
//! not at all, through a hidden temporary file beside it renamed over it,
//! with the path's failures told from the storage's. The writer is tested
//! against the real filesystem, a full one included; the Connection's
//! saver against a Daemon the test scripts over a Unix socket, which either
//! writes the path itself or streams the bytes under credits. The stories
//! are the Download, Checkpoint, Plan and recording tickets', which use the
//! writer, so these tests declare none of their own.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and set up the files outside any async worker"
)]

use std::path::{Path, PathBuf};

use goodfetch_collie_client::Error;
use goodfetch_collie_client::file_saving::FileSaving;
use goodfetch_protocol::checkpoint::{FileWriteFailedDetails, PathNotWritableDetails, WriteSide};

/// The call a failure's fix names.
const CALL: &str = "download.saveAs";

/// The names in `folder`, sorted.
fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// What a file failure details, either code's, read as one shape.
struct Failure {
    reason: String,
    os_code: Option<String>,
    side: Option<&'static str>,
    path: PathBuf,
    fix: String,
}

impl Failure {
    fn reason(&self) -> &str {
        &self.reason
    }

    fn os_code(&self) -> Option<&str> {
        self.os_code.as_deref()
    }

    fn side(&self) -> Option<&str> {
        self.side
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn fix(&self) -> &str {
        &self.fix
    }
}

/// The failure `error` carries, and whether it is the path's.
fn failure(error: &Error) -> (Failure, bool) {
    if let Some(details) = error.details::<PathNotWritableDetails>() {
        let failure = Failure {
            reason: details.reason,
            os_code: details.os_code,
            side: None,
            path: details.path.into(),
            fix: details.fix,
        };
        return (failure, true);
    }
    let Some(details) = error.details::<FileWriteFailedDetails>() else {
        panic!("not a file failure: {error:?}");
    };
    let failure = Failure {
        reason: details.reason,
        os_code: details.os_code,
        side: Some(match details.side {
            WriteSide::Engine => "engine",
            WriteSide::Program => "program",
        }),
        path: details.path.unwrap_or_default().into(),
        fix: error.fix().unwrap_or_default().to_owned(),
    };
    (failure, false)
}

#[test]
fn a_file_is_written_whole_into_folders_it_makes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("reports/2026/report.csv");

    let mut saving = FileSaving::create(&path, CALL).unwrap();
    saving.write(b"a,b\n").unwrap();
    saving.write(b"1,2\n").unwrap();
    saving.finish().unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"a,b\n1,2\n");
    assert_eq!(names(path.parent().unwrap()), ["report.csv"]);
}

#[test]
fn a_relative_path_resolves_against_the_working_directory() {
    let folder = tempfile::tempdir().unwrap();
    std::env::set_current_dir(folder.path()).unwrap();

    let mut saving = FileSaving::create(Path::new("out/report.csv"), CALL).unwrap();
    saving.write(b"bytes").unwrap();
    saving.finish().unwrap();

    assert_eq!(
        std::fs::read(folder.path().join("out/report.csv")).unwrap(),
        b"bytes"
    );
}

#[test]
fn a_failure_names_the_path_as_the_program_gave_it() {
    let folder = tempfile::tempdir().unwrap();
    std::env::set_current_dir(folder.path()).unwrap();
    std::fs::write(folder.path().join("out"), b"a file").unwrap();

    let error = FileSaving::create(Path::new("out/report.csv"), CALL).unwrap_err();

    let (failure, path_side) = failure(&error);
    assert!(path_side, "{error:?}");
    assert_eq!(failure.reason(), "not_a_directory");
    assert_eq!(failure.path(), Path::new("out/report.csv"));
}

#[test]
fn a_repeat_write_replaces_the_file_whole() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("report.csv");
    std::fs::write(&path, b"a longer file that held something else").unwrap();

    let mut saving = FileSaving::create(&path, CALL).unwrap();
    saving.write(b"new").unwrap();
    saving.finish().unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(names(folder.path()), ["report.csv"]);
}

#[test]
fn a_saving_dropped_before_it_finishes_removes_its_partial_file() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("report.csv");
    std::fs::write(&path, b"before").unwrap();

    let mut saving = FileSaving::create(&path, CALL).unwrap();
    saving.write(b"half of it").unwrap();
    let partial = names(folder.path());
    drop(saving);

    assert!(
        partial
            .iter()
            .any(|name| name.starts_with(".report.csv.") && name.ends_with(".partial")),
        "{partial:?}"
    );
    assert_eq!(names(folder.path()), ["report.csv"]);
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
}

#[cfg(unix)]
#[test]
fn a_folder_the_user_cannot_write_is_the_path_s_failure_with_the_os_error_as_its_source() {
    use std::os::unix::fs::PermissionsExt as _;
    if rustix_is_root() {
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    let locked = folder.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();

    let error = FileSaving::create(&locked.join("report.csv"), CALL)
        .err()
        .unwrap();

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (failure, of_path) = failure(&error);
    assert!(of_path, "{error:?}");
    assert_eq!(
        (failure.reason(), failure.os_code(), failure.side()),
        ("permission_denied", Some("EACCES"), None)
    );
    assert_eq!(failure.path(), locked.join("report.csv"));
    assert!(failure.fix().contains(CALL), "{}", failure.fix());
    let source = std::error::Error::source(&error).unwrap();
    assert_eq!(
        source.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(names(&locked), Vec::<String>::new());
}

/// Whether the test runs as root, whom no permission stops.
#[cfg(unix)]
fn rustix_is_root() -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let folder = tempfile::tempdir().unwrap();
    std::fs::metadata(folder.path()).unwrap().uid() == 0
}

#[test]
fn a_path_that_cannot_hold_a_file_is_the_path_s_failure() {
    let folder = tempfile::tempdir().unwrap();
    let a_file = folder.path().join("a-file");
    std::fs::write(&a_file, b"").unwrap();
    let mut trailing = folder.path().join("named").into_os_string();
    trailing.push(std::path::MAIN_SEPARATOR_STR);

    let cases: [(PathBuf, &str); 3] = [
        (folder.path().to_owned(), "is_a_directory"),
        (a_file.join("report.csv"), "not_a_directory"),
        (PathBuf::from(trailing), "invalid_filename"),
    ];
    for (path, reason) in cases {
        let error = FileSaving::create(&path, CALL).err().unwrap();
        let (failure, of_path) = failure(&error);
        assert!(of_path, "{path:?}: {error:?}");
        assert_eq!(failure.reason(), reason, "{path:?}");
    }
    assert_eq!(names(folder.path()), ["a-file"]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_full_storage_fails_the_write_on_the_program_s_side_and_leaves_no_partial_file() {
    let small = SmallFilesystem::mount(64 * 1024);
    let path = small.inside("report.csv");

    let error = (|| {
        let mut saving = FileSaving::create(&path, CALL)?;
        saving.write(&vec![7_u8; 256 * 1024])?;
        saving.finish()
    })()
    .err()
    .unwrap();

    let (failure, of_path) = failure(&error);
    assert!(!of_path, "{error:?}");
    assert_eq!(
        (failure.reason(), failure.os_code(), failure.side()),
        ("storage_full", Some("ENOSPC"), Some("program"))
    );
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(names(path.parent().unwrap()), Vec::<String>::new());
}

/// A tmpfs of a fixed size over a folder of the test's, mounted in a user
/// and mount namespace of a `sleep` the test starts and kills.
#[cfg(target_os = "linux")]
struct SmallFilesystem {
    sleeper: std::process::Child,
    folder: PathBuf,
    _parent: tempfile::TempDir,
}

#[cfg(target_os = "linux")]
impl SmallFilesystem {
    /// The test-only `unshare` that `ci/os-services.sh setup` installs.
    const TEST_UNSHARE: &str = "/usr/local/libexec/goodfetch-test-tools/unshare";

    #[expect(
        clippy::disallowed_methods,
        reason = "a test waits for the mount it started, outside any async worker"
    )]
    fn mount(size: u64) -> Self {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("small");
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
        let mounted = Self {
            sleeper,
            folder,
            _parent: parent,
        };
        let marker = mounted.inside(".mounted");
        for _ in 0..500 {
            if marker.exists() {
                std::fs::remove_file(&marker).unwrap();
                return mounted;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
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

/// The saver's tests: a Daemon the test scripts over a Unix socket.
#[cfg(unix)]
mod saver {
    use std::path::Path;
    use std::time::Duration;

    use goodfetch_collie_client::transport::{Endpoint, Network};
    use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams, ProductVersion, Surface};
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
    use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

    use super::{CALL, names};

    /// The Daemon's end of the Connection: the lines it reads and where
    /// it writes.
    pub(super) struct Daemon {
        lines: Lines<BufReader<OwnedReadHalf>>,
        writer: OwnedWriteHalf,
    }

    impl Daemon {
        /// The next message the client sent.
        pub(super) async fn read(&mut self) -> Value {
            serde_json::from_str(&self.lines.next_line().await.unwrap().unwrap()).unwrap()
        }

        /// Sends `message`.
        pub(super) async fn send(&mut self, message: &Value) {
            let mut line = serde_json::to_vec(message).unwrap();
            line.push(b'\n');
            self.writer.write_all(&line).await.unwrap();
        }

        /// Answers the call `request` with `result`.
        pub(super) async fn answer(&mut self, request: &Value, result: Value) {
            self.send(&json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
                .await;
        }

        /// Sends the `$/chunk` of the call `request` at `offset`.
        pub(super) async fn chunk(&mut self, request: &Value, offset: u64, data: &str) {
            self.send(&json!({
                "jsonrpc": "2.0",
                "method": "$/chunk",
                "params": {"id": request["id"], "offset": offset, "data": data},
            }))
            .await;
        }

        /// Reads the client's close.
        pub(super) async fn closed(mut self) {
            assert!(
                self.lines.next_line().await.unwrap().is_none(),
                "closing ends the input"
            );
            self.writer.shutdown().await.unwrap();
        }
    }

    /// Runs `script` as a Daemon on a Unix socket, after `hello`, and
    /// `program` with a Connection to it, on the test helper's runtime.
    pub(super) fn with_daemon<D, P, S, T>(
        script: impl FnOnce(Daemon) -> D,
        program: impl FnOnce(ClientConnection) -> P,
    ) -> (S, T)
    where
        D: Future<Output = S>,
        P: Future<Output = T>,
    {
        let folder = tempfile::tempdir().unwrap();
        let socket = folder.path().join("daemon.sock");
        let runtime = goodfetch_test_support::runtime();
        runtime.block_on(async {
            #[expect(
                clippy::disallowed_types,
                reason = "the test stands in for the Daemon, which listens on its socket"
            )]
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let daemon = async {
                let (stream, _) = listener.accept().await.unwrap();
                let (reader, writer) = stream.into_split();
                let mut daemon = Daemon {
                    lines: BufReader::new(reader).lines(),
                    writer,
                };
                let hello = daemon.read().await;
                daemon
                    .answer(
                        &hello,
                        json!({
                            "protocol_version": "0.1",
                            "product": "collie",
                            "product_version": "0.0.0",
                            "engine_instance": "01J0000000000000000000000",
                        }),
                    )
                    .await;
                script(daemon).await
            };
            let client = async {
                let hello = HelloParams {
                    needs: Some(Product::Collie),
                    token: Some("token".to_owned()),
                    ..HelloParams::new(Surface::Rust, ProductVersion::parse("0.0.0").unwrap())
                };
                let (connection, _) = ClientConnection::open(
                    ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                    &Network::Os,
                    &Endpoint::Daemon(DaemonEndpoint::Unix(socket.clone())),
                    hello,
                )
                .await
                .unwrap();
                let out = program(connection.clone()).await;
                connection.close().await.unwrap();
                out
            };
            tokio::join!(daemon, client)
        })
    }

    #[test]
    fn the_daemon_on_the_program_s_machine_is_given_the_absolute_path_and_writes_it() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.csv");
        let (given, saved): (Value, Result<Value, Error>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                // The credits come with every save, whoever writes the file.
                let _granted = daemon.read().await;
                daemon.answer(&call, json!({"written": true})).await;
                daemon.closed().await;
                call["params"].clone()
            },
            |connection| {
                let path = path.clone();
                async move {
                    connection
                        .save_file("download.save_as", &json!({"download": "d1"}), &path, CALL)
                        .await
                }
            },
        );

        assert_eq!(saved.unwrap(), json!({"written": true}));
        assert_eq!(
            given,
            json!({"download": "d1", "path": path.to_str().unwrap()})
        );
        assert_eq!(names(folder.path()), Vec::<String>::new());
    }

    #[test]
    fn a_daemon_that_streams_the_bytes_under_credits_has_them_written_whole() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("nested/report.csv");
        let ((call_id, credits), saved): ((Value, Vec<Value>), Result<Value, Error>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                let mut credits = vec![daemon.read().await];
                daemon.chunk(&call, 0, "YSxiCg==").await;
                daemon.chunk(&call, 4, "MSwyCg==").await;
                credits.push(daemon.read().await);
                credits.push(daemon.read().await);
                daemon.answer(&call, json!({"written": false})).await;
                daemon.closed().await;
                (call["id"].clone(), credits)
            },
            |connection| {
                let path = path.clone();
                async move {
                    connection
                        .save_file("download.save_as", &json!({}), &path, CALL)
                        .await
                }
            },
        );

        assert_eq!(saved.unwrap(), json!({"written": false}));
        assert_eq!(std::fs::read(&path).unwrap(), b"a,b\n1,2\n");
        assert_eq!(names(path.parent().unwrap()), ["report.csv"]);
        let granted = |credits: u32| json!({"jsonrpc": "2.0", "method": "$/credit", "params": {"id": call_id, "credits": credits}});
        assert_eq!(credits, [granted(16), granted(1), granted(1)]);
    }

    #[test]
    fn a_daemon_that_streams_nothing_leaves_an_empty_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("empty.csv");
        let ((), saved): ((), Result<Value, Error>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                let _granted = daemon.read().await;
                daemon.answer(&call, json!({"written": false})).await;
                daemon.closed().await;
            },
            |connection| {
                let path = path.clone();
                async move {
                    connection
                        .save_file("download.save_as", &json!({}), &path, CALL)
                        .await
                }
            },
        );

        saved.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn a_failure_of_the_engine_mid_stream_leaves_the_path_as_it_was() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.csv");
        std::fs::write(&path, b"before").unwrap();
        let ((), saved): ((), Result<Value, Error>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                let _granted = daemon.read().await;
                daemon.chunk(&call, 0, "YSxiCg==").await;
                let _granted = daemon.read().await;
                daemon
                    .send(&json!({
                        "jsonrpc": "2.0",
                        "id": call["id"],
                        "error": {
                            "code": -32000,
                            "message": "The page crashed.",
                            "data": {
                                "code": "page_crashed",
                                "group": "unavailable",
                                "retryable": false,
                                "fix": "Open a new page.",
                            },
                        },
                    }))
                    .await;
                daemon.closed().await;
            },
            |connection| {
                let path = path.clone();
                async move {
                    connection
                        .save_file("download.save_as", &json!({}), &path, CALL)
                        .await
                }
            },
        );

        assert!(
            matches!(&saved, Err(error) if error.code() == ErrorCode::PAGE_CRASHED),
            "{saved:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        assert_eq!(names(folder.path()), ["report.csv"]);
    }

    #[test]
    fn a_save_cancelled_mid_stream_removes_its_partial_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.csv");
        let (written, chunk_written) = tokio::sync::oneshot::channel::<()>();
        let mut written = Some(written);
        let ((call_id, cancel), left): ((Value, Value), Vec<String>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                let _granted = daemon.read().await;
                daemon.chunk(&call, 0, "YSxiCg==").await;
                let _granted = daemon.read().await;
                written.take().unwrap().send(()).unwrap();
                let cancel = daemon.read().await;
                daemon.closed().await;
                (call["id"].clone(), cancel)
            },
            |connection| {
                let path = path.clone();
                let folder = folder.path().to_owned();
                async move {
                    let saving =
                        connection.save_file::<Value>("download.save_as", &json!({}), &path, CALL);
                    tokio::select! {
                        saved = saving => panic!("the save ended: {saved:?}"),
                        _ = chunk_written => {}
                    }
                    names(&folder)
                }
            },
        );

        assert_eq!(left, Vec::<String>::new());
        assert!(!Path::new(&path).exists());
        assert_eq!(
            cancel,
            json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": call_id}})
        );
    }

    #[test]
    fn a_save_timed_out_mid_stream_removes_its_partial_file_and_stops_the_call() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("report.csv");
        let ((call_id, cancel), saved): ((Value, Value), Result<Value, Error>) = with_daemon(
            |mut daemon| async move {
                let call = daemon.read().await;
                let _granted = daemon.read().await;
                daemon.chunk(&call, 0, "YSxiCg==").await;
                let _granted = daemon.read().await;
                let cancel = daemon.read().await;
                daemon.closed().await;
                (call["id"].clone(), cancel)
            },
            |connection| {
                let path = path.clone();
                async move {
                    connection
                        .save_file("download.save_as", &json!({}), &path, CALL)
                        .timeout(Duration::from_millis(200))
                        .await
                }
            },
        );

        assert!(
            matches!(&saved, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
            "{saved:?}"
        );
        assert_eq!(names(folder.path()), Vec::<String>::new());
        assert_eq!(
            cancel,
            json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": call_id}})
        );
    }
}
