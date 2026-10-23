//! The version of the Clean-up rules each Run holds to (ADR 0047): chosen
//! before a new Run starts, by `clean_up.update` and `clean_up.version`,
//! from the embedded part, the daily channel ([`daily`]) or the Filter
//! lists' own URLs ([`upstream`]); its digest recorded in run state, which
//! a resume keeps, fetching a version this machine lacks where the daily
//! channel is configured or `clean_up.version` names it; named in the Run
//! trace's Provenance with its date and the release that applied it; and
//! warned of in the Run report when it is older than 14 days, or is not
//! the one the settings asked for. A version this release does not embed is
//! kept in the Browser cache as an artifact of kind `clean_up_rules`, its
//! build the digest of its files, and compiled on this machine when a page
//! first needs it. Each loaded version is shared by the Runs that hold to
//! it, and goes when none does.
//!
//! This release pins no full set (#207 pins the first), so `release` holds
//! to the embedded part.
#![expect(
    clippy::disallowed_methods,
    reason = "a version's files are read on the tasks module's blocking policy, never on an async worker"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use goodfetch_clean_up_rules::rules::{EMBEDDED_DIGEST, EMBEDDED_MANIFEST, FilterLists, ListTexts};
use goodfetch_engine::browser_cache::{BrowserCache, CacheLocation};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::http_client::Resolve;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::runs::{Answering, Chosen, CleanUpVersions, Holding, RulesContext, Run};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::tls::read_certificates;
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::clean_up_rules::{CleanUpRulesManifest, MANIFEST_FILE, RulesSource};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::formats::{CLEAN_UP_RULES, Compatibility};
use goodfetch_protocol::run::warning::{CLEAN_UP_RULES_FALLBACK, CLEAN_UP_RULES_STALE};
use goodfetch_protocol::run::{RunId, RunWarning};
use goodfetch_protocol::settings::{CLEAN_UP_UPDATE, CLEAN_UP_VERSION, NETWORK_RESOLVE};
use goodfetch_protocol::tls::TLS_TRUSTED_CAS;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::trace;

mod daily;
mod upstream;

/// How old a version may be before its Run's report warns (ADR 0047).
const STALE_AFTER: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// The most bytes one file of a version may hold.
const FILE_LIMIT: usize = 64 * 1024 * 1024;

/// The file a version's sources keep their state in, in the per-major
/// folder of Goodfetch home: when the daily channel was last checked and
/// what it gave, and the newest version fetched from the lists' authors.
const STATE_FILE: &str = "clean-up-rules.json";

/// The engine's versions of the Clean-up rules: those loaded, by digest,
/// and the Runs that hold to another version than their state records,
/// since this machine could not get theirs.
#[derive(Default)]
pub(crate) struct Versions {
    home: OnceLock<Home>,
    loaded: Mutex<BTreeMap<String, Weak<FilterLists>>>,
    loading: AsyncMutex<()>,
    instead: Mutex<BTreeMap<RunId, String>>,
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One version of the Clean-up rules: its digest and its manifest.
#[derive(Clone, Debug)]
pub(crate) struct Version {
    pub(crate) digest: String,
    pub(crate) manifest: CleanUpRulesManifest,
}

impl Version {
    /// The embedded part.
    fn embedded() -> Self {
        Self {
            digest: EMBEDDED_DIGEST.to_owned(),
            manifest: serde_json::from_str(EMBEDDED_MANIFEST)
                .expect("invariant: the clean-up crate carries a manifest of this format"),
        }
    }

    /// Whether it is past its expiry at `now`.
    fn expired(&self, now: jiff::Timestamp) -> bool {
        self.manifest
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
    }
}

/// What a Run's settings ask of the Clean-up rules.
struct Asked {
    /// `clean_up.update`: `release`, `daily` or `upstream`.
    update: String,
    /// `clean_up.version`, when set.
    version: Option<String>,
}

impl Asked {
    fn of(settings: Option<&Resolved>) -> Self {
        let text = |key: &str| {
            settings
                .and_then(|settings| settings.value(key).as_str())
                .map(str::to_owned)
        };
        Self {
            update: text(CLEAN_UP_UPDATE.name).unwrap_or_else(|| "release".to_owned()),
            version: text(CLEAN_UP_VERSION.name),
        }
    }
}

/// Where the versions this release does not embed are kept: the Browser
/// cache, read and written through the owner of their blocking work.
pub(crate) struct Kept {
    cache: BrowserCache,
    owner: TaskOwner,
}

impl Kept {
    /// The versions kept in the Browser cache of `home`.
    fn in_home(home: &Home, owner: TaskOwner) -> Self {
        Self {
            cache: BrowserCache::new(&CacheLocation::new(
                home.root(),
                home.browsers_path().map(Path::to_path_buf),
            )),
            owner,
        }
    }

    /// The version `files` hold, kept as an artifact of `source`'s name.
    pub(crate) async fn keep(
        &self,
        source: RulesSource,
        files: &[(String, Vec<u8>)],
        now: jiff::Timestamp,
    ) -> Result<Version, String> {
        let manifest = readable_manifest(files)?;
        let (digest, _held) = self
            .cache
            .keep(ArtifactKind::CleanUpRules, source.as_str(), files, now)
            .await
            .map_err(|error| format!("cannot be kept in the Browser cache: {error}"))?;
        Ok(Version {
            digest: digest.to_string(),
            manifest,
        })
    }

    /// The version of `digest` the cache holds, whatever its source, and
    /// the folder of its files.
    pub(crate) async fn cached(
        &self,
        digest: &str,
        now: jiff::Timestamp,
    ) -> Option<(Version, PathBuf)> {
        let parsed: Digest = digest.parse().ok()?;
        for source in [
            RulesSource::Daily,
            RulesSource::Upstream,
            RulesSource::Release,
        ] {
            let id = ArtifactId {
                kind: ArtifactKind::CleanUpRules,
                name: source.as_str().to_owned(),
                build: parsed.hex(),
            };
            let Ok(Some(held)) = self.cache.find(&id, parsed, now).await else {
                continue;
            };
            let files = held.files();
            let read = files.clone();
            let manifest = self
                .owner
                .blocking(move || std::fs::read(read.join(MANIFEST_FILE)))
                .await
                .ok()
                .and_then(|bytes| manifest_of(&bytes).ok())?;
            return Some((
                Version {
                    digest: digest.to_owned(),
                    manifest,
                },
                files,
            ));
        }
        None
    }
}

/// Where versions come from and are kept, under one Run's settings.
pub(crate) struct Sources {
    /// What a fetch may do: `egress.offline`, `network.resolve`, and the
    /// certificate authorities `tls.trusted_cas` lists.
    pub(crate) fetching: Fetching,
    /// Where the versions are kept.
    pub(crate) kept: Kept,
    /// The sources' state file.
    state: PathBuf,
    /// The engine's Product version, which the daily channel's tag names.
    pub(crate) product_version: ProductVersion,
}

impl Sources {
    /// The sources under `context`'s settings.
    async fn of(context: &RulesContext<'_>) -> Self {
        let settings = context.settings.map(|settings| settings.resolved());
        let owner = context.owner.child();
        let mut fetching = Fetching::new(context.clock);
        if let Some(settings) = settings {
            fetching.offline = settings
                .get(EGRESS_OFFLINE.name)
                .filter(|offline| offline.value == Value::Bool(true))
                .map(|offline| offline.layer);
            fetching.resolve = settings
                .hosts(NETWORK_RESOLVE.name)
                .into_iter()
                .fold(Resolve::new(), |mapping, (host, addresses)| {
                    mapping.with(host, addresses)
                });
            fetching.delays = Delays::of(settings);
            let files = authority_files(settings.value(TLS_TRUSTED_CAS.name));
            fetching.trusted_certificates = owner
                .blocking(move || {
                    files
                        .iter()
                        .filter_map(|file| match std::fs::read(file) {
                            Ok(bytes) => read_certificates(&bytes).ok(),
                            Err(error) => {
                                tracing::warn!(%error, file = %file.display(), "a certificate authority of `tls.trusted_cas` cannot be read");
                                None
                            }
                        })
                        .flatten()
                        .collect()
                })
                .await;
        }
        Self {
            fetching,
            kept: Kept::in_home(context.home, owner),
            state: context
                .home
                .per_major(context.product_version)
                .join(STATE_FILE),
            product_version: context.product_version.clone(),
        }
    }

    /// The sources' state, as last written.
    pub(crate) async fn read_state(&self) -> Value {
        let path = self.state.clone();
        self.kept
            .owner
            .blocking(move || std::fs::read(path))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({}))
    }

    /// Writes the sources' state; a state that cannot be written is logged,
    /// and the next start checks again.
    pub(crate) async fn write_state(&self, state: Value) {
        let path = self.state.clone();
        let written = self
            .kept
            .owner
            .blocking(move || {
                if let Some(folder) = path.parent() {
                    std::fs::create_dir_all(folder)?;
                }
                std::fs::write(&path, state.to_string())
            })
            .await;
        if let Err(error) = written {
            tracing::warn!(%error, "the Clean-up rules' sources cannot record their state");
        }
    }
}

/// The certificate files `tls.trusted_cas`, `value`, lists for any domain:
/// the authorities a source's endpoint is trusted under.
fn authority_files(value: &Value) -> Vec<PathBuf> {
    value
        .as_object()
        .into_iter()
        .flat_map(|domains| domains.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect()
}

/// The `manifest.json` of a version of the Clean-up rules, as the engine
/// writes it for the lists it fetched itself (ADR 0047): the format's
/// writer, which the Compatibility corpus's `clean-up-rules` set runs.
pub fn manifest_file(manifest: &CleanUpRulesManifest) -> Vec<u8> {
    let mut file = serde_json::to_vec_pretty(manifest)
        .expect("invariant: a manifest, of strings and times, always encodes");
    file.push(b'\n');
    file
}

/// `bytes` read as a manifest this engine reads (ADR 0030's
/// newest-readable rule): one of another format version is skipped.
fn manifest_of(bytes: &[u8]) -> Result<CleanUpRulesManifest, String> {
    let header: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("has no manifest: {error}"))?;
    let found = header["format_version"]
        .as_str()
        .and_then(|version| version.parse().ok())
        .ok_or("has a manifest without its format version")?;
    if CLEAN_UP_RULES
        .rule
        .compatibility(CLEAN_UP_RULES.version, found)
        != Compatibility::Compatible
    {
        return Err(format!(
            "has a manifest of format {found}, which this engine, reading {}, skips",
            CLEAN_UP_RULES.version
        ));
    }
    serde_json::from_value(header)
        .map_err(|error| format!("has a manifest that does not read: {error}"))
}

/// The manifest of the version `files` hold, which must name a file of
/// each list it holds.
fn readable_manifest(files: &[(String, Vec<u8>)]) -> Result<CleanUpRulesManifest, String> {
    let (_, bytes) = files
        .iter()
        .find(|(path, _)| path == MANIFEST_FILE)
        .ok_or("has no manifest")?;
    let manifest = manifest_of(bytes)?;
    for (list, file) in manifest.lists.files() {
        if !files.iter().any(|(path, _)| path == file) {
            return Err(format!(
                "names `{file}` for its `{list}` list, which it lacks"
            ));
        }
    }
    Ok(manifest)
}

/// Wall time now on `clock`.
fn now_of(clock: Clock) -> jiff::Timestamp {
    jiff::Timestamp::try_from(clock.wall()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

impl Versions {
    /// The Goodfetch home, kept from the first start or resume.
    fn remember(&self, home: &Home) {
        let _ = self.home.set(home.clone());
    }

    /// The version a new Run starting under `context` holds to.
    async fn choose_version(&self, context: RulesContext<'_>) -> Chosen {
        self.remember(context.home);
        let asked = Asked::of(context.settings.map(|settings| settings.resolved()));
        let now = now_of(context.clock);
        let embedded = Version::embedded();
        let chosen = if let Some(digest) = &asked.version {
            let sources = Sources::of(&context).await;
            obtain(digest, &asked, true, &sources, now)
                .await
                .map_err(|why| format!("`{}` names {digest}, which {why}", CLEAN_UP_VERSION.name))
        } else {
            match asked.update.as_str() {
                "daily" => {
                    let sources = Sources::of(&context).await;
                    daily::newest(&sources, now).await.and_then(|version| {
                        if version.expired(now) {
                            Err(format!(
                                "the daily channel's newest version, {}, is past its expiry",
                                version.digest
                            ))
                        } else {
                            Ok(version)
                        }
                    })
                }
                "upstream" => upstream::newest(&Sources::of(&context).await, now).await,
                _ => Ok(embedded.clone()),
            }
        };
        match chosen {
            Ok(version) => Chosen {
                digest: version.digest,
                fallback: None,
            },
            Err(why) => Chosen {
                digest: embedded.digest.clone(),
                fallback: Some(format!(
                    "{why}: the Run holds to the embedded part, {}",
                    embedded.digest
                )),
            },
        }
    }

    /// Records the version `run` holds to once it started or resumed.
    async fn record(&self, run: &Run, holding: Holding, context: RulesContext<'_>) {
        self.remember(context.home);
        let Some(recorded) = recorded_digest(run).await else {
            return;
        };
        let asked = Asked::of(context.settings.map(|settings| settings.resolved()));
        let now = now_of(context.clock);
        let resumed = holding == Holding::Resumed;
        let (version, fallback) = match holding {
            Holding::Started { fallback } => match known(&recorded, &context, now).await {
                Some(version) => (version, fallback),
                None => self.instead_of(run, &recorded, "is gone from this machine"),
            },
            Holding::Resumed => {
                let sources = Sources::of(&context).await;
                let explicit = asked.version.as_deref() == Some(recorded.as_str());
                match obtain(&recorded, &asked, explicit, &sources, now).await {
                    Ok(version) => (version, None),
                    Err(why) => self.instead_of(run, &recorded, &why),
                }
            }
        };
        let mut entry = json!({
            "action": "clean_up_rules",
            "digest": version.digest,
            "source": version.manifest.source,
            "published_at": version.manifest.published_at,
            "applied_by": { "product": context.product, "product_version": context.product_version },
            "update": asked.update,
            "resumed": resumed,
        });
        if let Some(fallback) = &fallback {
            entry["fallback"] = json!(fallback);
            entry["recorded"] = json!(recorded);
        }
        trace::provenance(run, &entry).await;
        if let Some(fallback) = fallback {
            let warning = RunWarning {
                code: CLEAN_UP_RULES_FALLBACK.to_owned(),
                message: format!(
                    "The Run's Clean-up rules are not the version asked for: {fallback}."
                ),
                sessions: Vec::new(),
                folder: None,
            };
            if let Err(error) = run.warn(warning).await {
                tracing::warn!(%error, "the warning of the Run's Clean-up rules was not kept");
            }
        }
        let published = version.manifest.published_at;
        if published
            .checked_add(STALE_AFTER)
            .is_ok_and(|stale_from| stale_from <= now)
        {
            let warning = RunWarning {
                code: CLEAN_UP_RULES_STALE.to_owned(),
                message: format!(
                    "The Run holds to Clean-up rules {} of {published}, older than 14 days.",
                    version.digest
                ),
                sessions: Vec::new(),
                folder: None,
            };
            if let Err(error) = run.warn_once(warning).await {
                tracing::warn!(%error, "the warning of the Run's Clean-up rules was not kept");
            }
        }
    }

    /// The embedded part, which `run` holds to instead of its recorded
    /// version, `recorded`, that `why` keeps from it, and the fallback that
    /// Provenance records.
    fn instead_of(&self, run: &Run, recorded: &str, why: &str) -> (Version, Option<String>) {
        let embedded = Version::embedded();
        lock(&self.instead).insert(run.id().clone(), embedded.digest.clone());
        let fallback = format!(
            "the Run's recorded Clean-up rules, {recorded}, {why}: it holds to the embedded part, {}, instead",
            embedded.digest
        );
        (embedded, Some(fallback))
    }

    /// The Filter lists `run`'s pages match: the version it holds to,
    /// loaded once for every Run that holds to it, through `owner`'s
    /// blocking tasks.
    pub(crate) async fn lists(&self, run: &Run, owner: &TaskOwner) -> Arc<FilterLists> {
        let instead = lock(&self.instead).get(run.id()).cloned();
        let digest = match instead {
            Some(digest) => digest,
            None => recorded_digest(run)
                .await
                .unwrap_or_else(|| EMBEDDED_DIGEST.to_owned()),
        };
        if let Some(lists) = self.loaded(&digest) {
            return lists;
        }
        let _loading = self.loading.lock().await;
        if let Some(lists) = self.loaded(&digest) {
            return lists;
        }
        let lists = Arc::new(self.load(&digest, run, owner).await);
        lock(&self.loaded).insert(digest, Arc::downgrade(&lists));
        lists
    }

    /// The loaded version of `digest`, if a Run still holds it.
    fn loaded(&self, digest: &str) -> Option<Arc<FilterLists>> {
        let mut loaded = lock(&self.loaded);
        loaded.retain(|_, lists| lists.strong_count() > 0);
        loaded.get(digest).and_then(Weak::upgrade)
    }

    /// Loads the version of `digest`: the embedded part from its compiled
    /// form, any other from its texts in the cache, compiled here; the
    /// embedded part when the cache has lost it.
    async fn load(&self, digest: &str, run: &Run, owner: &TaskOwner) -> FilterLists {
        let cached = match (digest == EMBEDDED_DIGEST, self.home.get()) {
            (false, Some(home)) => {
                Kept::in_home(home, owner.child())
                    .cached(digest, now_of(run.clock()))
                    .await
            }
            _ => None,
        };
        if let Some((version, files)) = cached {
            let lists = version.manifest.lists;
            let compiled = owner
                .blocking(move || -> std::io::Result<FilterLists> {
                    let read = |file: &str| std::fs::read_to_string(files.join(file));
                    let (ads, trackers, annoyances, unbreak) = (
                        read(&lists.ads)?,
                        read(&lists.trackers)?,
                        read(&lists.annoyances)?,
                        read(&lists.unbreak)?,
                    );
                    Ok(FilterLists::compile(&ListTexts {
                        ads: &ads,
                        trackers: &trackers,
                        annoyances: &annoyances,
                        unbreak: &unbreak,
                    }))
                })
                .await;
            match compiled {
                Ok(lists) => return lists,
                Err(error) => {
                    tracing::warn!(%error, %digest, "the Run's Clean-up rules cannot be read: its pages match the embedded part");
                }
            }
        } else if digest != EMBEDDED_DIGEST {
            tracing::warn!(%digest, "the Run's Clean-up rules are gone from the Browser cache: its pages match the embedded part");
        }
        owner
            .blocking(FilterLists::embedded)
            .await
            .expect("invariant: the build compiled the embedded Filter lists with this binary's own adblock")
    }
}

/// The version of `digest`: the embedded part, one the cache holds, or one
/// fetched from the daily channel where it is configured or `explicit`,
/// named by `clean_up.version`.
async fn obtain(
    digest: &str,
    asked: &Asked,
    explicit: bool,
    sources: &Sources,
    now: jiff::Timestamp,
) -> Result<Version, String> {
    if digest == EMBEDDED_DIGEST {
        return Ok(Version::embedded());
    }
    if digest.parse::<Digest>().is_err() {
        return Err("is no digest `sha256:<hex>`".to_owned());
    }
    if let Some((version, _)) = sources.kept.cached(digest, now).await {
        return Ok(version);
    }
    if explicit || asked.update == "daily" {
        return daily::by_digest(sources, digest, now).await;
    }
    Err("is not on this machine, and the daily channel is not configured to fetch it".to_owned())
}

/// The version of `digest` this machine holds: the embedded part or one the
/// cache keeps.
async fn known(digest: &str, context: &RulesContext<'_>, now: jiff::Timestamp) -> Option<Version> {
    if digest == EMBEDDED_DIGEST {
        return Some(Version::embedded());
    }
    Kept::in_home(context.home, context.owner.child())
        .cached(digest, now)
        .await
        .map(|(version, _)| version)
}

/// The Clean-up rules digest `run`'s state records.
async fn recorded_digest(run: &Run) -> Option<String> {
    run.state()
        .read(|connection| {
            connection.query_row("SELECT clean_up_digest FROM run", [], |row| {
                row.get::<_, Option<String>>(0)
            })
        })
        .await
        .ok()
        .flatten()
}

impl CleanUpVersions for Versions {
    fn choose<'a>(&'a self, context: RulesContext<'a>) -> Answering<'a, Chosen> {
        Box::pin(self.choose_version(context))
    }

    fn held<'a>(
        &'a self,
        run: &'a Run,
        holding: Holding,
        context: RulesContext<'a>,
    ) -> Answering<'a, ()> {
        Box::pin(self.record(run, holding, context))
    }
}
