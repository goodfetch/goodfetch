//! A process Plugin started as the engine's Plugin host starts it (ADR
//! 0033): through Collie's executable in its hidden `plugin-shim` mode,
//! confined, with `goodfetch-scripted-plugin` as the Plugin, alone or as
//! an instance of a Run or of a task.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out a Plugin's folders and start processes outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_engine::plugin_host::instance::{self, CallFailure, PluginInstance, Scope};
use goodfetch_engine::plugin_host::process::{Pipes, PluginProcess};
use goodfetch_engine::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE, PluginManifest};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{Tool, capturing, tools};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::ChildStdout;
use tracing::Level;

use crate::COLLIE;

/// A scripted Plugin's bundle and private temporary folder under a fresh
/// root, its approved manifest and the launch that starts it.
struct Plugin {
    _root: TempDir,
    layout: Layout,
    manifest: PluginManifest,
    launch: Launch,
}

impl Plugin {
    /// A Plugin whose manifest declares `API_URL`, given with a variable it
    /// does not declare, and whose `hello` answers nothing.
    fn new() -> Self {
        Self::scripted(&json!({ "hello": {} }))
    }

    /// A Plugin that answers its approved manifest's `hello`, the Proxy
    /// kind's.
    fn approved() -> Self {
        Self::scripted(
            &json!({ "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])) }),
        )
    }

    /// The Plugin as [`new`](Self::new) lays it out, which plays `script`.
    fn scripted(script: &Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
        for folder in [&layout.bundle, &layout.instance] {
            fs::create_dir_all(folder).unwrap();
        }
        let scripted = tools()
            .unwrap()
            .path(Tool::ScriptedPlugin)
            .unwrap()
            .to_owned();
        fs::copy(scripted, layout.bundle.join("plugin")).unwrap();
        let played = layout.bundle.join("script.json");
        fs::write(&played, script.to_string()).unwrap();
        let manifest = json!({
            "manifest_version": "0.1",
            "name": "scripted",
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
            "env": { "API_URL": "api_url" },
        });
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        let instance = Instance {
            bundle: layout.bundle.clone(),
            runtime: None,
            folders: vec![layout.instance.clone()],
            env: BTreeMap::from([
                ("API_URL".to_owned(), "https://api.test".to_owned()),
                ("GITHUB_TOKEN".to_owned(), "canary".to_owned()),
            ]),
        };
        let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
        let launch = Launch {
            policy,
            program: layout.bundle.join("plugin"),
            args: vec![played.to_string_lossy().into_owned()],
            folder: layout.instance.clone(),
        };
        Self {
            _root: root,
            layout,
            manifest,
            launch,
        }
    }

    /// What starts its instance in Collie.
    fn instance(&self) -> instance::Plugin {
        instance::Plugin {
            launcher: COLLIE.into(),
            launch: self.launch.clone(),
            manifest: self.manifest.clone(),
            product: Product::Collie,
        }
    }
}

/// The scripted Plugin's answer to `hello` with `kinds`, its approved
/// manifest's otherwise.
fn hello(kinds: &Value) -> Value {
    json!({
        "protocol_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": kinds,
        "network_level": "none",
    })
}

/// The JSON value `instance` answers `method` with.
async fn answer(instance: &PluginInstance, method: &str) -> Value {
    instance
        .call(method, &json!({}))
        .await
        .unwrap()
        .read()
        .unwrap()
}

/// The Error code of a call that the instance did not carry out.
fn unavailable(failure: Result<impl std::fmt::Debug, CallFailure>) -> &'static str {
    match failure {
        Err(CallFailure::Unavailable(error)) => error.code(),
        other => panic!("{other:?} is no call the instance did not carry out"),
    }
}

/// The Run the tests' instances serve.
fn checkout() -> RunId {
    RunId::parse("checkout").unwrap()
}

/// The started Plugin's side of the plugin protocol, one line a message.
struct Connection {
    stdin: tokio::process::ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next: u64,
}

impl Connection {
    fn new(pipes: Pipes) -> Self {
        Self {
            stdin: pipes.stdin,
            lines: BufReader::new(pipes.stdout).lines(),
            next: 1,
        }
    }

    /// The Plugin's answer to `method` with `params`.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line = serde_json::to_vec(&request).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line).await.unwrap();
        self.stdin.flush().await.unwrap();
        let answer = self.lines.next_line().await.unwrap().unwrap();
        let answer: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["id"], id, "{answer}");
        answer
    }
}

/// The Plugin sees the variables its manifest declares and the cookie,
/// nothing the engine holds and no loader variable, works from its
/// private temporary folder and holds no handle beyond stdio.
///
/// Covers: story:7-30
#[tokio::test]
async fn a_started_plugin_holds_its_declared_variables_alone() {
    let plugin = Plugin::new();
    let (process, pipes) = PluginProcess::start(Path::new(COLLIE), &plugin.launch).unwrap();
    let mut connection = Connection::new(pipes);

    let mut environment = connection.call("scripted.environment", json!({})).await;

    let mut environment = environment["result"].take();
    let id = environment.as_object_mut().unwrap().remove("process");
    assert_eq!(
        (environment, id.is_some_and(|id| id.is_u64())),
        (
            json!({
                "variables": { "API_URL": "https://api.test", COOKIE_VARIABLE: COOKIE },
                "folder": plugin.layout.instance,
                "handles": [],
            }),
            true
        )
    );
    process
        .stop(&TaskOwner::root(BlockingPolicy::Threads))
        .await
        .unwrap();
}

/// What the Plugin writes in its private temporary folder is gone once it
/// stops.
///
/// Covers: story:7-30
#[tokio::test]
async fn a_stopped_plugins_temporary_folder_is_gone() {
    let plugin = Plugin::new();
    let (process, pipes) = PluginProcess::start(Path::new(COLLIE), &plugin.launch).unwrap();
    let mut connection = Connection::new(pipes);
    let written = plugin.layout.instance.join("written");
    let answer = connection
        .call(
            "scripted.write",
            json!({ "path": written, "text": "canary" }),
        )
        .await;
    assert_eq!(answer["result"], json!({}), "{answer}");
    assert!(written.exists());

    process
        .stop(&TaskOwner::root(BlockingPolicy::Threads))
        .await
        .unwrap();

    assert!(!plugin.layout.instance.exists());
}

/// A Plugin started without the plugin protocol's cookie, as a program
/// run by hand, refuses to run and says nothing on the protocol's pipe.
///
/// Covers: story:7-31
#[test]
fn a_plugin_started_without_the_cookie_refuses_to_run() {
    let plugin = Plugin::new();

    let output = Command::new(&plugin.launch.program)
        .args(&plugin.launch.args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!((output.status.code(), output.stdout), (Some(1), Vec::new()));
}

/// An instance started on a Run's owner says `hello` for that Run before
/// the call made while it started, and is gone once the Run's owner stops,
/// as every end of a Run stops it: its process, its private temporary
/// folder and the calls made after.
///
/// Covers: story:7-28
#[tokio::test]
async fn a_runs_instance_is_gone_once_its_run_ends() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let run = root.child_for(OwnerKind::Run);
    let plugin = Plugin::approved();
    let instance = PluginInstance::start(&run, plugin.instance(), Scope::Run(checkout()));

    let calls = answer(&instance, "scripted.calls").await;
    run.stop().await;

    assert_eq!(
        (
            calls.as_array().map(Vec::len),
            calls[0]["method"].clone(),
            calls[0]["params"]["run_id"].clone(),
            plugin.layout.instance.exists(),
            unavailable(instance.call("scripted.calls", &json!({})).await),
        ),
        (
            Some(1),
            json!("hello"),
            json!("checkout"),
            false,
            "call_cancelled"
        )
    );
}

/// An instance started on a task's own owner, outside the Run's life,
/// runs a process of its own, says `hello` for no Run, and is killed when
/// its task ends while the Run's instance still answers.
///
/// Covers: story:7-29
#[tokio::test]
async fn a_tasks_instance_shares_no_process_with_a_run_and_ends_with_its_task() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (run, task) = (root.child_for(OwnerKind::Run), root.child());
    let (for_run, for_task) = (Plugin::approved(), Plugin::approved());
    let run_instance = PluginInstance::start(&run, for_run.instance(), Scope::Run(checkout()));
    let task_instance = PluginInstance::start(&task, for_task.instance(), Scope::Task(None));
    let run_process = answer(&run_instance, "scripted.environment").await["process"].take();
    let task_process = answer(&task_instance, "scripted.environment").await["process"].take();
    let task_hello = answer(&task_instance, "scripted.calls").await[0]["params"].take();

    task.stop().await;

    assert_eq!(
        (
            run_process == task_process,
            task_hello.get("run_id"),
            for_task.layout.instance.exists(),
            unavailable(task_instance.call("scripted.calls", &json!({})).await),
            answer(&run_instance, "scripted.calls").await[0]["method"].clone(),
        ),
        (false, None, false, "call_cancelled", json!("hello"))
    );
    run.stop().await;
}

/// An instance whose `hello` declares another kind than its approved
/// manifest is stopped with `plugin_manifest_mismatch`, which the call made
/// while it started fails with.
///
/// Covers: story:7-31, error:plugin_manifest_mismatch
#[tokio::test]
async fn an_instance_whose_hello_differs_from_its_manifest_is_stopped() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let plugin = Plugin::scripted(
        &json!({ "hello": hello(&json!([{ "kind": "sink", "capabilities": [] }])) }),
    );
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));

    let failure = instance.call("scripted.calls", &json!({})).await;

    assert_eq!(
        (
            unavailable(failure),
            instance.ended().map(|error| error.code()),
            plugin.layout.instance.exists(),
        ),
        (
            "plugin_manifest_mismatch",
            Some("plugin_manifest_mismatch"),
            false
        )
    );
    root.stop().await;
}

/// A second answer to `hello` and an answer to no call are logged at
/// `warn` as malformed answers and change nothing: the instance still
/// answers, and no transition is refused.
///
/// Covers: story:7-28
#[tokio::test]
async fn a_plugins_stray_answers_are_malformed_answers_not_events() {
    let (subscriber, log) = capturing(&[("goodfetch_engine::plugin_host", Level::WARN)]);
    let _logging = tracing::subscriber::set_default(subscriber);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let mut script = json!({ "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])) });
    script["after_hello"] = json!([
        { "jsonrpc": "2.0", "id": 0, "result": script["hello"] },
        { "jsonrpc": "2.0", "id": 99, "result": {} },
    ]);
    let plugin = Plugin::scripted(&script);
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));

    let calls = answer(&instance, "scripted.calls").await;

    let written = String::from_utf8(log.written()).unwrap();
    let levels: Vec<&str> = written
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    assert_eq!(
        (
            calls[0]["method"].clone(),
            levels,
            instance.ended().is_none()
        ),
        (json!("hello"), vec!["WARN", "WARN"], true),
        "{written}"
    );
    instance.stop().await;
}
