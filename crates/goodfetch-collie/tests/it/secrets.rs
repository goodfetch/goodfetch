//! Secret references through Collie's executable (spec #564's seam; ADRs
//! 0015, 0016, 0067 and 0092): `run_state.key_reference`, the first
//! credential setting, takes only a Secret reference and resolves it when a
//! Run starts, through the `env:` provider over the engine's environment,
//! the encrypted local file and the OS keychain; the use is recorded in the
//! Audit log, and a reference that gives no value refuses the start with
//! `secret_unavailable`. A Secret canary planted as the value through each
//! provider's real interface is searched in every reply, the logs and every
//! file the test's folder holds, raw and encoded. With no OS keychain on
//! the host, the key reference's default is the encrypted local file.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the encrypted local files of the test's Goodfetch home themselves"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::MESSAGE_DEADLINE;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The variable the tests' key-encryption key is read from.
const VARIABLE: &str = "GF_TEST_RUN_KEY";

/// The user configuration that names the key-encryption key by reference.
fn by_reference(env: &TestEnv) {
    env.user_configuration(&format!("run_state.key_reference = \"env:{VARIABLE}\"\n"));
}

/// A canary planted as the key-encryption key's value, with its ID.
fn planted(env: &TestEnv) -> (Canaries, String, String) {
    let mut canaries = Canaries::new(env.seed());
    let canary = canaries.plant(SecretKind::RunDataKey);
    let (id, value) = (canary.id().to_owned(), canary.value().to_owned());
    (canaries, id, value)
}

/// The replies of an Attached engine started with `variables`, to a `run.start`
/// of `run_id` and, when it started, its `run.finish`.
fn started(env: &TestEnv, variables: &[(&str, &str)], run_id: &str) -> Vec<Value> {
    let mut engine = Attached::start_with(env, COLLIE, MESSAGE_DEADLINE, |command| {
        for (name, value) in variables {
            command.env(name, value);
        }
    });
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let mut replies = vec![engine.ask(1, "run.start", &json!({ "run_id": run_id }))];
    if replies[0].get("result").is_some() {
        replies.push(engine.ask(2, "run.finish", &json!({ "run_id": run_id })));
    }
    let (rest, _) = engine.finish();
    replies.extend(rest);
    replies
}

/// The leaks of `canaries` in `replies` and in every file of `env`'s folder,
/// its logs and Goodfetch home included.
fn leaks(canaries: &Canaries, env: &TestEnv, replies: &[Value]) -> Vec<String> {
    let replies = serde_json::to_vec(replies).unwrap();
    canaries
        .leaks(&[
            Output::Bytes {
                name: "replies",
                bytes: &replies,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{} in {}", hit.canary, hit.output))
        .collect()
}

/// Covers: story:4-49, error:setting_invalid, setting:run_state.key_reference
#[test]
fn a_raw_value_as_the_key_reference_is_refused_with_setting_invalid_naming_no_value() {
    let env = TestEnv::new();
    let (mut canaries, id, value) = planted(&env);
    env.user_configuration(&format!("run_state.key_reference = \"{value}\"\n"));
    let configuration = HomeLayout::of(env.home()).user_configuration();
    canaries.record_planted(&id, &configuration.display().to_string());

    let replies = started(&env, &[], "raw");

    assert_eq!(
        (
            &replies[0]["error"]["data"]["code"],
            &replies[0]["error"]["data"]["key"],
            leaks(&canaries, &env, &replies),
        ),
        (
            &json!("setting_invalid"),
            &json!("run_state.key_reference"),
            Vec::<String>::new()
        )
    );
}

/// Story 4-34: a run whose `tls.client_certificates` gives a client
/// certificate's key as a value, a canary standing for a pasted key, is
/// refused with `setting_invalid` naming the setting, and the value appears
/// in no reply, log or file, the logs at `trace`.
/// Covers: story:4-34, error:setting_invalid, setting:tls.client_certificates
#[test]
fn a_raw_client_certificate_key_is_refused_with_setting_invalid_naming_no_value() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let value = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let mut replies = vec![engine.ask(
        1,
        "run.start",
        &json!({
            "run_id": "mutual",
            "settings": { "tls": { "client_certificates": {
                "mtls.example": { "certificate": "client.pem", "key": value },
            } } },
        }),
    )];
    let (rest, _) = engine.finish();
    replies.extend(rest);

    assert_eq!(
        (
            &replies[0]["error"]["data"]["code"],
            &replies[0]["error"]["data"]["key"],
            leaks(&canaries, &env, &replies),
        ),
        (
            &json!("setting_invalid"),
            &json!("tls.client_certificates"),
            Vec::<String>::new()
        )
    );
}

/// Covers: story:4-49, setting:run_state.key_reference
#[test]
fn config_explain_shows_the_key_reference_never_its_value() {
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    by_reference(&env);

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "run_state.key_reference", "--json"])
        .env(VARIABLE, &value)
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&output.stdout).unwrap();
    let shown: Vec<&Value> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|setting| setting["key"] == "run_state.key_reference")
        .map(|setting| &setting["value"])
        .collect();
    let printed = format!("{}{}", output.stdout, output.stderr);
    assert_eq!(
        (
            shown,
            canaries
                .leaks(&[Output::Bytes {
                    name: "config explain",
                    bytes: printed.as_bytes(),
                }])
                .unwrap()
        ),
        (vec![&json!(format!("env:{VARIABLE}"))], vec![])
    );
}

/// Covers: story:4-49, story:4-62, setting:run_state.key_reference
#[test]
fn a_run_resolves_its_key_reference_at_start_and_records_the_use_without_the_value() {
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    by_reference(&env);

    let replies = started(&env, &[(VARIABLE, &value)], "keyed");

    let recorded: Vec<(String, Value)> = Connection::open(env.home().join("audit-log.sqlite"))
        .unwrap()
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| {
            let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
            (
                entry["kind"].as_str().unwrap().to_owned(),
                entry["details"].clone(),
            )
        })
        .collect();
    assert_eq!(
        (
            &replies[1]["result"]["lifecycle"],
            recorded,
            leaks(&canaries, &env, &replies)
        ),
        (
            &json!("finished"),
            vec![(
                "secret_reference_used".to_owned(),
                json!({
                    "placeholder": "run_state.key_reference",
                    "reference": format!("env:{VARIABLE}"),
                    "domain": null,
                })
            )],
            Vec::<String>::new()
        )
    );
}

/// Covers: story:4-60, error:secret_unavailable, setting:run_state.key_reference
#[test]
fn a_run_whose_key_variable_is_unset_is_refused_with_secret_unavailable_before_it_exists() {
    let env = TestEnv::new();
    by_reference(&env);

    let replies = started(&env, &[], "unkeyed");

    let mut data = replies[0]["error"]["data"].clone();
    let fix = data.as_object_mut().unwrap().remove("fix");
    assert_eq!(
        (
            data,
            fix.is_some_and(|fix| fix.is_string()),
            HomeLayout::of(env.home()).run_folder("unkeyed").exists()
        ),
        (
            json!({
                "code": "secret_unavailable",
                "placeholder": "run_state.key_reference",
                "reference": format!("env:{VARIABLE}"),
                "provider": "env",
                "failure": "missing",
                "retryable": false,
            }),
            true,
            false
        )
    );
}

/// The encrypted local files' folder of `env`'s home.
fn secret_files(env: &TestEnv) -> std::path::PathBuf {
    env.home().join("secrets")
}

/// Makes the identity of `env`'s encrypted local files, as `age-keygen -o`
/// writes it, only its owner's where modes exist, and answers it.
pub(crate) fn identity(env: &TestEnv) -> age::x25519::Identity {
    use age::secrecy::ExposeSecret as _;
    let identity = age::x25519::Identity::generate();
    std::fs::create_dir_all(secret_files(env)).unwrap();
    let path = secret_files(env).join("identity.txt");
    std::fs::write(
        &path,
        format!(
            "# public key: {}\n{}\n",
            identity.to_public(),
            identity.to_string().expose_secret()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    identity
}

/// Plants `value` in the encrypted local file `name`, encrypted to
/// `identity` as `age -r` encrypts it.
pub(crate) fn encrypt(env: &TestEnv, name: &str, value: &str, identity: &age::x25519::Identity) {
    std::fs::write(
        secret_files(env).join(format!("{name}.age")),
        encrypted(value, identity),
    )
    .unwrap();
}

/// `value` encrypted to `identity`, as an encrypted local file holds it.
fn encrypted(value: &str, identity: &age::x25519::Identity) -> Vec<u8> {
    use std::io::Write as _;
    let recipient = identity.to_public();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .unwrap();
    let mut encrypted = Vec::new();
    let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
    writer.write_all(value.as_bytes()).unwrap();
    writer.finish().unwrap();
    encrypted
}

/// A Session opening on a proxy whose password's Secret has not resolved
/// yet holds up no other Run's calls: while one Run's first `run.new_page`
/// waits on the encrypted local file of its `proxy.password`, a named pipe
/// that gives its value only once the other Run's navigation answered,
/// another Run's page goes on. A navigation held up by the wait never
/// answers, and fails past the engine's message deadline.
/// Covers: operation:run.new_page, operation:page.goto, setting:proxy.password
#[cfg(unix)]
#[test]
fn a_session_waiting_on_its_proxy_secret_holds_up_no_other_runs_page() {
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::time::Duration;

    let env = TestEnv::new();
    let proving_ground = goodfetch_test_support::ProvingGround::start(&env, 697).unwrap();
    let cache = goodfetch_test_support::browser_cache(&env).unwrap();
    let mut engine = crate::page::started(&env, &cache, &proving_ground, "going-on");
    let page =
        crate::page::opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "going-on" })));
    let identity = identity(&env);
    let pipe = secret_files(&env).join("proxy-password.age");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    let value = encrypted("proxy-password", &identity);
    let (read_begun, reader_waits) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        // A pipe opens for writing only once its reader opened it: the
        // waiting Run's Session, reading its Secret.
        let mut secret = std::fs::OpenOptions::new().write(true).open(pipe).unwrap();
        read_begun.send(()).unwrap();
        released.recv().unwrap();
        secret.write_all(&value).unwrap();
    });
    let started = engine.ask(
        3,
        "run.start",
        &json!({ "run_id": "waiting", "settings": { "proxy": {
            "server": "http://127.0.0.1:3128",
            "username": "user",
            "password": "secretfile:proxy-password",
        } } }),
    );
    engine.send(&json!({ "jsonrpc": "2.0", "id": 4, "method": "run.new_page", "params": { "run_id": "waiting" } }));
    reader_waits
        .recv_timeout(Duration::from_secs(60))
        .expect("the waiting Run's Session reads its Secret");
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": 5, "method": "page.goto", "params": {
        "page_id": page.page_id,
        "url": proving_ground.url("catalog.test", "/catalog/1"),
    } }),
    );
    let went = loop {
        let message = engine.next().unwrap();
        if message["id"] == json!(5) {
            break message;
        }
    };
    release.send(()).unwrap();
    writer.join().unwrap();
    let _ = engine.finish();

    assert_eq!(
        (
            made.success(),
            started.get("result").is_some(),
            &went["result"]["response"]["status"],
        ),
        (true, true, &json!(200)),
        "{went}"
    );
}

/// The Audit log's entries in `env`'s home, as kind and details.
fn audited(env: &TestEnv) -> Vec<(String, Value)> {
    Connection::open(env.home().join("audit-log.sqlite"))
        .unwrap()
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| {
            let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
            (
                entry["kind"].as_str().unwrap().to_owned(),
                entry["details"].clone(),
            )
        })
        .collect()
}

/// `config explain run_state.key_reference --json`'s value and layer, from
/// the executable itself or, with `daemon`, the major's Daemon, with the
/// environment `variables` adds to the test's.
fn explained_key_reference(
    env: &TestEnv,
    daemon: bool,
    variables: &[(&str, std::ffi::OsString)],
) -> (Value, Value) {
    let mut command =
        cli(env, COLLIE).args(["config", "explain", "run_state.key_reference", "--json"]);
    if daemon {
        command = command.arg("--daemon");
    }
    for (name, value) in variables {
        command = command.env(name, value);
    }
    let output = command.run().unwrap();
    let explained: Value = serde_json::from_str(&output.stdout).unwrap();
    let setting = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["key"] == "run_state.key_reference")
        .unwrap()
        .clone();
    (setting["value"].clone(), setting["layer"].clone())
}

/// Covers: story:4-50, story:4-62, setting:run_state.key_reference
#[test]
fn a_run_resolves_its_key_from_an_encrypted_local_file_and_no_output_holds_it() {
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    let identity = identity(&env);
    encrypt(&env, "run-key", &value, &identity);
    env.user_configuration("run_state.key_reference = \"secretfile:run-key\"\n");

    let replies = started(&env, &[], "filed");

    assert_eq!(
        (
            &replies[1]["result"]["lifecycle"],
            audited(&env),
            leaks(&canaries, &env, &replies)
        ),
        (
            &json!("finished"),
            vec![(
                "secret_reference_used".to_owned(),
                json!({
                    "placeholder": "run_state.key_reference",
                    "reference": "secretfile:run-key",
                    "domain": null,
                })
            )],
            Vec::<String>::new()
        )
    );
}

/// Covers: story:4-50, error:secret_unavailable, setting:run_state.key_reference
#[cfg(unix)]
#[test]
fn an_identity_file_other_users_may_read_refuses_the_run_with_the_chmod_that_closes_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    let identity = identity(&env);
    encrypt(&env, "run-key", &value, &identity);
    let path = secret_files(&env).join("identity.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    env.user_configuration("run_state.key_reference = \"secretfile:run-key\"\n");

    let replies = started(&env, &[], "widened");

    let data = &replies[0]["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["failure"],
            data["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains(&format!("chmod 600 {}", path.display()))),
            leaks(&canaries, &env, &replies)
        ),
        (
            &json!("secret_unavailable"),
            &json!("not_configured"),
            true,
            Vec::<String>::new()
        ),
        "{data}"
    );
}

/// Covers: story:4-50, setting:run_state.key_reference
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[rstest::rstest]
#[case::from_the_executable(false)]
#[case::from_the_daemon(true)]
fn with_no_keychain_on_the_host_the_key_reference_defaults_to_the_encrypted_local_file(
    #[case] daemon: bool,
) {
    let env = TestEnv::new();
    let _stops = crate::daemon::StopsTheDaemon::of(&env, COLLIE);

    assert_eq!(
        explained_key_reference(&env, daemon, &[]),
        (json!("secretfile:run-state-key"), json!("default"))
    );
}

/// The real OS keychain (TESTING.md, Storage, network and OS services):
/// each item under a service named for its test, removed after it; on
/// Linux the child finds the Secret Service through the session bus the
/// test was given, and on macOS the user's keychains, which the test
/// links into its own `HOME`. A host without the service skips this
/// module.
mod keychain {
    use goodfetch_engine::secrets::Keychain;
    use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
    use goodfetch_test_support::TestEnv;
    use serde_json::json;

    use super::{audited, explained_key_reference, leaks, planted, started};

    /// The variables that lead a child to this session's keychain, once
    /// [`reach_the_users_keychain`] gave `env` the user's on macOS.
    #[expect(
        clippy::disallowed_methods,
        reason = "the test forwards its own session's bus to the child it starts (TESTING.md)"
    )]
    fn session(env: &TestEnv) -> Vec<(&'static str, String)> {
        reach_the_users_keychain(env);
        ["DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"]
            .into_iter()
            .filter_map(|name| {
                std::env::var_os(name).map(|value| (name, value.to_string_lossy().into_owned()))
            })
            .collect()
    }

    /// Links the user's keychains and keychain preferences into `env`'s
    /// `HOME`, where macOS's Security framework looks for the user's
    /// default keychain (`Library/Keychains`, `Library/Preferences/
    /// com.apple.security.plist`); without them a child has none. A source
    /// the user does not have is left out.
    #[cfg(target_os = "macos")]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test leads the child it starts to its own user's keychain (TESTING.md)"
    )]
    fn reach_the_users_keychain(env: &TestEnv) {
        let real = std::path::PathBuf::from(std::env::var_os("HOME").expect("the test has a HOME"));
        let user = std::path::Path::new(&env.env()["HOME"]);
        for part in [
            "Library/Keychains",
            "Library/Preferences/com.apple.security.plist",
        ] {
            let source = real.join(part);
            if source.exists() {
                let link = user.join(part);
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                std::os::unix::fs::symlink(&source, &link).unwrap();
            }
        }
    }

    /// Elsewhere the variables [`session`] passes on are enough.
    #[cfg(not(target_os = "macos"))]
    fn reach_the_users_keychain(_env: &TestEnv) {}

    /// Runs `work` against this OS's keychain, on a runtime of real time.
    fn in_keychain<T>(work: impl AsyncFnOnce(&Keychain, &TaskOwner) -> T) -> T {
        let runtime = goodfetch_runtime::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let keychain = Keychain::os();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        runtime.block_on(work(&keychain, &owner))
    }

    /// The item of `service`'s `account` in this OS's keychain, removed when
    /// dropped.
    struct Item {
        service: String,
        account: &'static str,
    }

    impl Item {
        /// On macOS the item lets any program read it (`security -A`): one
        /// made through the keychain API trusts only this test binary, and
        /// the engine's read of it would wait on a confirmation no one sees.
        #[cfg(target_os = "macos")]
        fn stored(service: String, account: &'static str, value: &str) -> Self {
            let added = std::process::Command::new("/usr/bin/security")
                .args(["add-generic-password", "-A", "-U", "-s", &service])
                .args(["-a", account, "-w", value])
                .status()
                .expect("macOS has `security`");
            assert!(
                added.success(),
                "this host has its OS keychain (TESTING.md: `ci/os-services.sh setup`)"
            );
            Self { service, account }
        }

        #[cfg(not(target_os = "macos"))]
        fn stored(service: String, account: &'static str, value: &str) -> Self {
            in_keychain(async |keychain, owner| {
                keychain
                    .store(
                        &service,
                        account,
                        &secrecy::SecretString::from(value.to_owned()),
                        owner,
                    )
                    .await
            })
            .expect("this host has its OS keychain (TESTING.md: `ci/os-services.sh setup`)");
            Self { service, account }
        }
    }

    impl Drop for Item {
        fn drop(&mut self) {
            let _ = in_keychain(async |keychain, owner| {
                keychain.remove(&self.service, self.account, owner).await
            });
        }
    }

    /// Covers: story:4-50, story:4-62, setting:run_state.key_reference
    #[test]
    fn a_run_resolves_its_key_from_the_os_keychain_and_no_output_holds_it() {
        let env = TestEnv::new();
        let (canaries, _, value) = planted(&env);
        let item = Item::stored(
            format!("goodfetch-test-{:016x}", env.seed()),
            "run-key",
            &value,
        );
        let reference = format!("keychain:{}/{}", item.service, item.account);
        env.user_configuration(&format!("run_state.key_reference = \"{reference}\"\n"));
        let session = session(&env);
        let variables: Vec<(&str, &str)> = session
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();

        let replies = started(&env, &variables, "chained");

        assert_eq!(
            (
                &replies[1]["result"]["lifecycle"],
                audited(&env),
                leaks(&canaries, &env, &replies)
            ),
            (
                &json!("finished"),
                vec![(
                    "secret_reference_used".to_owned(),
                    json!({
                        "placeholder": "run_state.key_reference",
                        "reference": reference,
                        "domain": null,
                    })
                )],
                Vec::<String>::new()
            )
        );
    }

    /// Covers: story:4-50, error:secret_unavailable, setting:run_state.key_reference
    #[test]
    fn a_run_whose_keychain_item_is_missing_is_refused_with_secret_unavailable() {
        let env = TestEnv::new();
        let reference = format!("keychain:goodfetch-test-{:016x}/absent", env.seed());
        env.user_configuration(&format!("run_state.key_reference = \"{reference}\"\n"));
        let session = session(&env);
        let variables: Vec<(&str, &str)> = session
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();

        let replies = started(&env, &variables, "unchained");

        let data = &replies[0]["error"]["data"];
        assert_eq!(
            (&data["code"], &data["provider"], &data["failure"]),
            (
                &json!("secret_unavailable"),
                &json!("keychain"),
                &json!("missing")
            ),
            "{data}"
        );
    }

    /// Covers: story:4-50, setting:run_state.key_reference
    #[test]
    fn with_the_os_keychain_the_key_reference_defaults_to_it() {
        let env = TestEnv::new();
        let variables: Vec<(&str, std::ffi::OsString)> = session(&env)
            .into_iter()
            .map(|(name, value)| (name, value.into()))
            .collect();

        assert_eq!(
            explained_key_reference(&env, false, &variables),
            (json!("keychain:goodfetch/run-state-key"), json!("default"))
        );
    }
}

/// The external providers Collie's composition registers for each Run
/// (#645, #647), through Collie's executable: `run_state.key_reference`
/// names a key held by 1Password or Bitwarden, whose CLI,
/// `secret_providers.<scheme>.command` in user configuration, is a
/// shell-script stand-in answering as `op read` and `bw get` do. The value
/// resolves as the Run starts and is in no reply, log or file of the
/// test's folder; with no command, the start fails with
/// `secret_unavailable`, `not_configured`.
#[cfg(unix)]
pub(crate) mod external_providers {
    use std::os::unix::fs::PermissionsExt as _;

    use goodfetch_test_client::HomeLayout;
    use goodfetch_test_support::TestEnv;
    use rstest::rstest;
    use serde_json::json;

    use super::{leaks, planted, started};

    /// The sign-in each stand-in takes; no real token's shape.
    pub(crate) const SIGN_IN: &str = "stand-in-sign-in";

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Vendor {
        OnePassword,
        Bitwarden,
    }

    impl Vendor {
        pub(crate) fn scheme(self) -> &'static str {
            match self {
                Self::OnePassword => "op",
                Self::Bitwarden => "bw",
            }
        }

        pub(crate) fn reference(self) -> &'static str {
            match self {
                Self::OnePassword => "op://Shop/run-key/password",
                Self::Bitwarden => "bw:run-key/password",
            }
        }

        /// The variable the vendor's CLI is signed in by.
        pub(crate) fn sign_in(self) -> &'static str {
            match self {
                Self::OnePassword => "OP_SERVICE_ACCOUNT_TOKEN",
                Self::Bitwarden => "BW_SESSION",
            }
        }

        /// The stand-in, answering the reference with `store`'s content.
        pub(crate) fn script(self, store: &std::path::Path) -> String {
            let (expected, refusal) = match self {
                Self::OnePassword => (
                    "read --no-newline op://Shop/run-key/password",
                    "[ERROR] 2026/10/26 10:00:00 You are not currently signed in.",
                ),
                Self::Bitwarden => ("get password -- run-key", "Vault is locked."),
            };
            format!(
                "#!/bin/sh\nif [ \"${variable}\" != \"{SIGN_IN}\" ]; then printf '%s' '{refusal}' >&2; exit 1; fi\nif [ \"$*\" = \"{expected}\" ]; then cat '{store}'; exit 0; fi\nprintf 'Not found.' >&2\nexit 1\n",
                variable = self.sign_in(),
                store = store.display()
            )
        }
    }

    /// A Run whose key-encryption key the vendor's CLI holds starts, and
    /// the key is in no reply, log or file.
    /// Covers: story:4-50, setting:run_state.key_reference, setting:secret_providers.op.command, setting:secret_providers.bw.command
    #[rstest]
    #[case::one_password(Vendor::OnePassword)]
    #[case::bitwarden(Vendor::Bitwarden)]
    fn a_run_keyed_by_an_external_provider_starts_and_no_output_holds_the_key(
        #[case] vendor: Vendor,
    ) {
        let env = TestEnv::new();
        let (mut canaries, id, value) = planted(&env);
        let folder = env.folder().join("stand-in");
        std::fs::create_dir_all(&folder).unwrap();
        let store = folder.join("value");
        std::fs::write(&store, &value).unwrap();
        canaries.record_planted(&id, &store.display().to_string());
        let command = folder.join(vendor.scheme());
        std::fs::write(&command, vendor.script(&store)).unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
        env.user_configuration(&format!(
            "run_state.key_reference = \"{}\"\nsecret_providers.{}.command = \"{}\"\n",
            vendor.reference(),
            vendor.scheme(),
            command.display()
        ));

        let replies = started(&env, &[(vendor.sign_in(), SIGN_IN)], "keyed");

        assert_eq!(
            (
                &replies[1]["result"]["lifecycle"],
                leaks(&canaries, &env, &replies)
            ),
            (&json!("finished"), Vec::<String>::new()),
            "{replies:?}"
        );
    }

    /// With no `secret_providers.<scheme>.command`, the Run is refused with
    /// `secret_unavailable`, `not_configured`, before it exists.
    /// Covers: story:4-50, error:secret_unavailable, setting:run_state.key_reference
    #[rstest]
    #[case::one_password(Vendor::OnePassword)]
    #[case::bitwarden(Vendor::Bitwarden)]
    fn a_run_keyed_by_an_unconfigured_provider_is_refused_with_not_configured(
        #[case] vendor: Vendor,
    ) {
        let env = TestEnv::new();
        env.user_configuration(&format!(
            "run_state.key_reference = \"{}\"\n",
            vendor.reference()
        ));

        let replies = started(&env, &[], "unconfigured");

        let data = &replies[0]["error"]["data"];
        assert_eq!(
            (
                &data["code"],
                &data["provider"],
                &data["failure"],
                data["fix"]
                    .as_str()
                    .is_some_and(|fix| fix
                        .contains(&format!("secret_providers.{}.command", vendor.scheme()))),
                HomeLayout::of(env.home())
                    .run_folder("unconfigured")
                    .exists()
            ),
            (
                &json!("secret_unavailable"),
                &json!(vendor.scheme()),
                &json!("not_configured"),
                true,
                false
            ),
            "{data}"
        );
    }
}
