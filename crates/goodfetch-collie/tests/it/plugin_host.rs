//! A process Plugin started as the engine's Plugin host starts it (ADR
//! 0033): through Collie's executable in its hidden `plugin-shim` mode,
//! confined, with `goodfetch-scripted-plugin` as the Plugin.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out a Plugin's folders and start processes outside any async worker"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_engine::plugin_host::process::{Pipes, PluginProcess};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE, PluginManifest};
use goodfetch_test_support::{Tool, tools};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::ChildStdout;

use crate::COLLIE;

/// A scripted Plugin's bundle and private temporary folder under a fresh
/// root, and the launch that starts it.
struct Plugin {
    _root: TempDir,
    layout: Layout,
    launch: Launch,
}

impl Plugin {
    /// A Plugin whose manifest declares `API_URL`, given with a variable it
    /// does not declare.
    fn new() -> Self {
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
        let script = layout.bundle.join("script.json");
        fs::write(&script, json!({ "hello": {} }).to_string()).unwrap();
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
            args: vec![script.to_string_lossy().into_owned()],
            folder: layout.instance.clone(),
        };
        Self {
            _root: root,
            layout,
            launch,
        }
    }
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

    let environment = connection.call("scripted.environment", json!({})).await;

    assert_eq!(
        environment["result"],
        json!({
            "variables": { "API_URL": "https://api.test", COOKIE_VARIABLE: COOKIE },
            "folder": plugin.layout.instance,
            "handles": [],
        })
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
