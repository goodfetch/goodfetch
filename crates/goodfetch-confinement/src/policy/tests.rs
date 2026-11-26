#![expect(
    clippy::disallowed_methods,
    reason = "these tests make and read files outside any async worker"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use goodfetch_protocol::plugin::{Limits, NetworkLevel, PluginManifest};
use rstest::rstest;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{Instance, Policy, PolicyError};
use crate::deny_list::Places;

/// A user's home with a key in `~/.ssh` and a project holding a `.env` file
/// and a Git checkout, and a clean folder of data.
struct Host {
    root: TempDir,
}

impl Host {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for folder in [
            "home/.ssh",
            "home/project/.git/hooks",
            "home/env-project",
            "data/nested",
            "goodfetch/plugins/probe/1.0.0",
        ] {
            fs::create_dir_all(root.path().join(folder)).unwrap();
        }
        for file in [
            "home/.ssh/id_ed25519",
            "home/project/.git/hooks/pre-commit",
            "home/env-project/.env.local",
            "data/nested/words.txt",
        ] {
            fs::write(root.path().join(file), "probe").unwrap();
        }
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        fs::canonicalize(self.root.path()).unwrap().join(relative)
    }

    fn places(&self) -> Places {
        Places {
            home: self.path("home"),
            goodfetch_home: self.path("goodfetch"),
            app_data: None,
            local_app_data: None,
        }
    }

    fn instance(&self) -> Instance {
        Instance {
            bundle: self.path("goodfetch/plugins/probe/1.0.0"),
            runtime: Some(self.path("goodfetch/runtimes/node-22")),
            folders: vec![self.path("goodfetch/instances/probe-1")],
            env: BTreeMap::from([
                ("API_KEY".to_owned(), "k".to_owned()),
                ("UNDECLARED".to_owned(), "u".to_owned()),
            ]),
        }
    }

    fn policy(&self, extra: Value) -> Result<Policy, PolicyError> {
        let mut manifest = json!({
            "manifest_version": "0.1",
            "name": "probe",
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "runtime": { "kind": "node", "version": "22.11.0" },
            "entrypoint": "index.js",
            "env": { "API_KEY": "api_key" },
            "limits": { "memory": 1_048_576 },
        });
        let Value::Object(extra) = extra else {
            panic!("the manifest's extra fields are an object");
        };
        for (field, value) in extra {
            manifest[field] = value;
        }
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        Policy::of(&manifest, &self.instance(), &self.places())
    }
}

#[test]
fn a_policy_grants_the_bundle_runtime_and_instance_folders_and_the_declared_variables_alone() {
    let host = Host::new();

    let policy = host.policy(json!({})).unwrap();

    assert_eq!(
        policy,
        Policy {
            network_level: NetworkLevel::None,
            reads: vec![
                host.path("goodfetch/plugins/probe/1.0.0"),
                host.path("goodfetch/runtimes/node-22"),
            ],
            writes: vec![host.path("goodfetch/instances/probe-1")],
            executables: vec![
                host.path("goodfetch/plugins/probe/1.0.0/index.js"),
                host.path("goodfetch/runtimes/node-22"),
            ],
            env: BTreeMap::from([("API_KEY".to_owned(), "k".to_owned())]),
            limits: Limits {
                memory: 1_048_576,
                ..Limits::default()
            },
        },
    );
}

#[test]
fn a_declared_folder_with_nothing_denied_is_read_at_its_real_path() {
    let host = Host::new();

    let policy = host
        .policy(json!({ "files": [host.path("data/nested/../nested")] }))
        .unwrap();

    assert_eq!(policy.reads.last(), Some(&host.path("data/nested")));
}

#[rstest]
#[case::a_key_in_the_ssh_folder("home/.ssh/id_ed25519", "deny.ssh")]
#[case::the_ssh_folder("home/.ssh", "deny.ssh")]
#[case::the_home_that_holds_it("home", "deny.ssh")]
#[case::a_dot_env_file("home/env-project/.env.local", "deny.env_file")]
#[case::a_folder_holding_a_dot_env_file("home/env-project", "deny.env_file")]
#[case::a_folder_holding_git_hooks("home/project", "deny.git_hooks")]
#[case::the_goodfetch_home("goodfetch", "deny.goodfetch_home")]
fn a_declared_file_the_deny_list_covers_refuses_the_policy(
    #[case] declared: &str,
    #[case] rule: &str,
) {
    let host = Host::new();

    let refused = host.policy(json!({ "files": [host.path(declared)] }));

    assert!(
        matches!(&refused, Err(PolicyError::Denied { rule: found, .. }) if *found == rule),
        "{refused:?}",
    );
}

#[cfg(unix)]
#[test]
fn a_declared_link_into_a_denied_folder_refuses_the_policy() {
    let host = Host::new();
    std::os::unix::fs::symlink(host.path("home/.ssh"), host.path("data/keys")).unwrap();

    let refused = host.policy(json!({ "files": [host.path("data/keys")] }));

    assert!(
        matches!(
            &refused,
            Err(PolicyError::Denied {
                rule: "deny.ssh",
                ..
            })
        ),
        "{refused:?}",
    );
}

#[test]
fn a_declared_process_the_deny_list_covers_refuses_the_policy() {
    let host = Host::new();

    let refused =
        host.policy(json!({ "processes": [host.path("home/project/.git/hooks/pre-commit")] }));

    assert!(
        matches!(
            &refused,
            Err(PolicyError::Denied {
                rule: "deny.git_hooks",
                ..
            })
        ),
        "{refused:?}",
    );
}

#[rstest]
#[case::relative(json!({ "files": ["data/nested"] }))]
#[case::missing(json!({ "files": ["/nonexistent-goodfetch-probe/file"] }))]
fn a_declared_file_that_names_no_readable_path_refuses_the_policy(#[case] extra: Value) {
    let host = Host::new();

    let refused = host.policy(extra);

    assert!(
        matches!(
            &refused,
            Err(PolicyError::Relative { .. } | PolicyError::Unreadable { .. })
        ),
        "{refused:?}",
    );
}

/// Every key of `value` at any depth, and every value of a `name` field.
fn names(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields {
                found.push(key.clone());
                if key == "name" {
                    found.extend(field.as_str().map(str::to_owned));
                }
                names(field, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| names(item, found)),
        _ => {}
    }
}

/// No setting, operation parameter (a run's options) or manifest field
/// turns confinement off, and the policy has no field that would: the
/// catalog's settings and operations, which every Client and Recipe reads,
/// the manifest's schema and the policy's own shape hold no such name
/// (ADRs 0016, 0033). The catalog's error codes are what the engine
/// answers, not what anyone sets, so `confinement_unavailable` is no way
/// out.
///
/// Covers: story:7-22
#[test]
fn nothing_a_user_or_a_plugin_sets_names_a_way_out_of_confinement() {
    let generated = Path::new(env!("CARGO_MANIFEST_DIR")).join("../goodfetch-protocol/generated");
    let read = |file: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(generated.join(file)).unwrap()).unwrap()
    };
    let mut found = Vec::new();
    let catalog = read("catalog.json");
    names(&catalog["settings"], &mut found);
    names(&catalog["operations"], &mut found);
    assert!(found.len() > 100, "the catalog's settings and operations");
    names(&read("plugin-manifest.schema.json"), &mut found);
    names(&read("plugin-protocol.schema.json"), &mut found);
    let policy = Host::new().policy(json!({})).unwrap();
    names(&serde_json::to_value(&policy).unwrap(), &mut found);

    let ways_out: Vec<&String> = found
        .iter()
        .filter(|name| {
            let name = name.to_lowercase();
            [
                "unconfined",
                "confinement",
                "confine",
                "seccomp",
                "landlock",
            ]
            .iter()
            .any(|word| name.contains(word))
                || (name.contains("plugin") && name.contains("sandbox"))
        })
        .collect();
    assert_eq!(ways_out, Vec::<&String>::new());
}
