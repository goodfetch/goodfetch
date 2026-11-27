//! `plugin.add` (ADR 0033, issue 889): a Plugin installed from its OCI
//! reference and pinned in the Plugin lockfile, against a stand-in registry
//! on loopback over HTTPS.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write and read the lockfile outside any async worker"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use goodfetch_engine::CallError;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::plugin_add;
use goodfetch_engine::plugin_lockfile;
use goodfetch_engine::plugin_oci::TAR_GZIP_MEDIA_TYPE;
use goodfetch_engine::plugin_store::PluginStore;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::add::PluginAdded;
use goodfetch_protocol::plugin::lockfile::{LOCKFILE_NAME, LockedPlugin, PluginLockfile};
use goodfetch_protocol::plugin::{NetworkLevel, PluginKindName};
use goodfetch_test_support::TestEnv;

use crate::downloads::{Received, StandIn};
use crate::plugin_install::{Artifact, bundle, gzip, store};

/// The Plugin `kettle`'s artifact, its bundle unpacked into the home of
/// `env`.
fn kettle(env: &TestEnv) -> Artifact {
    Artifact::of(gzip(&bundle(&env.home().join("ran"))), TAR_GZIP_MEDIA_TYPE)
}

/// A lockfile path in a project folder of `env` that does not exist yet.
fn lockfile(env: &TestEnv, project: &str) -> PathBuf {
    env.home().join(project).join(LOCKFILE_NAME)
}

/// Adds `artifact` from a stand-in registry that serves it, once for each
/// lockfile of `lockfiles` in turn, into the store of `env`; answers each
/// outcome with the requests the registry received for it.
fn add(
    env: &TestEnv,
    artifact: &Artifact,
    lockfiles: &[&Path],
) -> Vec<(Result<PluginAdded, CallError>, Vec<Received>)> {
    let store = PluginStore::new(store(env));
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    goodfetch_test_support::runtime().block_on(async {
        let mut stand_in = StandIn::serve_with(artifact.answer(artifact.clone())).await;
        let reference = artifact.reference(&stand_in);
        let fetching = stand_in.fetching();
        let mut outcomes = Vec::new();
        for lockfile in lockfiles {
            let added = plugin_add::add(&reference, &fetching, &blocking, &store, lockfile).await;
            outcomes.push((added, stand_in.requests()));
        }
        outcomes
    })
}

/// The lockfile at `path`, read as the engine reads it.
fn pins(path: &Path) -> PluginLockfile {
    plugin_lockfile::read(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `plugin add` installs the Plugin and pins its index's digest in a
/// lockfile it makes, its folder with it; adding it again answers the same
/// without a pull.
///
/// Covers: story:7-5
#[test]
fn a_plugin_added_is_installed_and_pinned_and_a_repeat_pulls_nothing() {
    let env = TestEnv::new();
    let artifact = kettle(&env);
    let lockfile = lockfile(&env, "project");

    let [(first, pulled), (again, repeated)]: [_; 2] =
        add(&env, &artifact, &[&lockfile, &lockfile])
            .try_into()
            .unwrap();

    let digest = Digest::of(&artifact.index).to_string();
    let added = first.unwrap();
    assert_eq!(
        added,
        PluginAdded {
            name: "kettle".to_owned(),
            version: "1.0.0".to_owned(),
            digest: digest.clone(),
            lockfile: lockfile.display().to_string(),
            kinds: BTreeSet::from([PluginKindName::EXTRACT]),
            network_level: NetworkLevel::None,
            env: BTreeMap::new(),
            files: vec![],
            processes: vec![],
            browsers: vec![],
            spend: BTreeMap::new(),
            permissions: vec![],
            signer: None,
            approval: None,
        }
    );
    assert_eq!(pulled.len(), 3);
    assert_eq!(
        pins(&lockfile).locked("kettle"),
        Some(&LockedPlugin {
            name: "kettle".to_owned(),
            version: "1.0.0".to_owned(),
            digest,
            signer: None,
            approvals: None,
        })
    );
    assert_eq!(again.unwrap(), added);
    assert_eq!(repeated, []);
}

/// A lockfile that pins the Plugin to another digest refuses the add with
/// `plugin_digest_mismatch` before anything is pulled, and the lockfile
/// and the store stay as they were.
///
/// Covers: story:7-81
#[test]
fn a_lockfile_pinning_another_digest_refuses_the_add_before_any_pull() {
    let env = TestEnv::new();
    let artifact = kettle(&env);
    let lockfile = lockfile(&env, "project");
    let other = Digest::of(b"another index").to_string();
    let mut pinned = PluginLockfile::empty();
    pinned
        .lock(LockedPlugin {
            name: "kettle".to_owned(),
            version: "0.9.0".to_owned(),
            digest: other.clone(),
            signer: None,
            approvals: None,
        })
        .unwrap();
    let text = plugin_lockfile::text(&pinned);
    std::fs::create_dir_all(lockfile.parent().unwrap()).unwrap();
    std::fs::write(&lockfile, &text).unwrap();

    let [(refused, requests)]: [_; 1] = add(&env, &artifact, &[&lockfile]).try_into().unwrap();

    let refused = refused.unwrap_err();
    assert_eq!(refused.code(), "plugin_digest_mismatch");
    let details = refused.details().unwrap();
    assert_eq!(details["locked"], other.as_str());
    assert_eq!(
        details["found"],
        Digest::of(&artifact.index).to_string().as_str()
    );
    assert_eq!(requests, []);
    assert_eq!(std::fs::read_to_string(&lockfile).unwrap(), text);
    assert!(!store(&env).exists());
}

/// A Plugin the store holds that another lockfile does not pin is refused
/// with `plugin_installed`, without a pull, and that lockfile is not
/// written.
///
/// Covers: story:7-5
#[test]
fn a_plugin_installed_that_the_lockfile_does_not_pin_is_refused() {
    let env = TestEnv::new();
    let artifact = kettle(&env);
    let (first, second) = (lockfile(&env, "first"), lockfile(&env, "second"));

    let [(added, _), (refused, requests)]: [_; 2] =
        add(&env, &artifact, &[&first, &second]).try_into().unwrap();

    added.unwrap();
    assert_eq!(refused.unwrap_err().code(), "plugin_installed");
    assert_eq!(requests, []);
    assert!(!second.exists());
}
