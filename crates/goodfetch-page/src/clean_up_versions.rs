//! The version of the Clean-up rules each Run holds to (ADR 0047): chosen
//! before a new Run starts, by `clean_up.update` and `clean_up.version`,
//! from the embedded part or the Filter lists' own URLs ([`upstream`]), or,
//! for a version `clean_up.version` pins by its digest, the daily channel
//! ([`daily`]); its digest recorded in run state, which a resume keeps,
//! fetching a version this machine lacks where `clean_up.version` names it
//! or the daily channel is configured; named in the Run trace's Provenance
//! with its date and the release that applied it; and warned of in the Run
//! report when it is older than 14 days, or is not the one the settings
//! asked for. A version this release does not embed is kept in the Browser
//! cache as an artifact of kind `clean_up_rules`, its build the digest of
//! its files, held there while a Run holds to it, and compiled on this
//! machine when a page first needs it. Each loaded version is shared by the
//! Runs that hold to it, and the engine keeps the [`KEPT_VERSIONS`] it
//! used last once none does, so the next Run on one of them compiles
//! nothing again; an older one goes. Every fetch at a start or
//! a resume ends within `clean_up.fetch_timeout`.
//!
//! What the engine cannot do yet is refused when the Run starts, never
//! ignored (ADR 0083): `clean_up.update = daily`, whose versions are used
//! only once the channel's signature is checked (ADR 0033's verifier),
//! `filter_lists.page_scriptlets` and `filter_lists.extra`.
//!
//! This release pins no full set (#207 pins the first), so `release` holds
//! to the embedded part.
#![expect(
    clippy::disallowed_methods,
    reason = "a version's files are read on the tasks module's blocking policy, never on an async worker"
)]
#![expect(
    clippy::disallowed_types,
    reason = "each lock guards a table for one operation, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use goodfetch_clean_up_rules::rules::{EMBEDDED_DIGEST, EMBEDDED_MANIFEST, FilterLists, ListTexts};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::{BrowserCache, CacheLocation, HeldArtifact};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::run_state::run_row;
use goodfetch_engine::runs::{
    Answering, Chosen, CleanUpVersions, Fallback, Holding, RulesContext, Run,
};
use goodfetch_engine::settings::{Resolved, RunSettings, catalog_value, get_or_default};
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::tls::read_certificates;
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::clean_up_rules::{CleanUpRulesManifest, MANIFEST_FILE, RulesSource};
use goodfetch_protocol::formats::{CLEAN_UP_RULES, Compatibility};
use goodfetch_protocol::run::warning::{CLEAN_UP_RULES_FALLBACK, CLEAN_UP_RULES_STALE};
use goodfetch_protocol::run::{RunId, RunWarning};
use goodfetch_protocol::settings::{
    CLEAN_UP_CHANNEL, CLEAN_UP_FETCH_TIMEOUT, CLEAN_UP_LISTS, CLEAN_UP_UPDATE, CLEAN_UP_VERSION,
    SettingsLayer, UPSTREAM_LISTS,
};
use goodfetch_protocol::tls::TLS_TRUSTED_CAS;
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use crate::trace;

mod daily;
mod failures;
mod unbuilt;
mod upstream;

pub(crate) use self::failures::Unobtained;
use self::failures::{fallback, refusal};
use self::unbuilt::refuse_unbuilt;

/// How old a version may be before its Run's report warns (ADR 0047).
const STALE_AFTER: Duration = Duration::from_hours(14 * 24);

/// The most bytes one file of a version may hold.
const FILE_LIMIT: usize = 64 * 1024 * 1024;

/// How many loaded versions the engine keeps once no Run holds them, the
/// most recently used: the next Run on one of them reuses its Filter lists
/// rather than compiling them again. Two covers a switch of version, Runs
/// on the old one ending while Runs on the new one start, and the embedded
/// part a fallback holds beside a fetched version.
const KEPT_VERSIONS: usize = 2;

/// The file a version's sources keep their state in, in the per-major
/// folder of Goodfetch home: the newest version fetched from the lists'
/// authors, and the URLs it came from.
const STATE_FILE: &str = "clean-up-rules.json";

/// The engine's versions of the Clean-up rules: those loaded, by digest,
/// and the version each live Run holds to.
#[derive(Default)]
pub(crate) struct Versions {
    home: OnceLock<Home>,
    /// Each version loaded or loading, by digest: the loads of one digest
    /// wait on its cell, never on a lock held while it compiles; a load that
    /// failed keeps its failure there for the loads that waited on it.
    loaded: Mutex<BTreeMap<String, Arc<Loaded>>>,
    /// The versions used last, newest first, at most [`KEPT_VERSIONS`]:
    /// each stays loaded once no Run holds it, for the next Run.
    kept: Mutex<VecDeque<(String, Arc<FilterLists>)>>,
    holding: Mutex<BTreeMap<RunId, Held>>,
    /// The fetches of the Filter lists from their authors' URLs.
    upstream: upstream::Fetches,
}

/// One version's load: the version once loaded, while a Run holds it or
/// the engine keeps it, or why it did not load.
type Loaded = OnceCell<Result<Weak<FilterLists>, Unobtained>>;

/// The version one live Run holds to: its digest, and its files, held in
/// the Browser cache so no clean-up removes them while the Run lives.
struct Held {
    /// Cancelled once the Run ends.
    live: CancellationToken,
    digest: String,
    artifact: Option<Arc<HeldArtifact>>,
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The Filter lists of the version a Run holds to, loaded, and its digest,
/// which names that version wherever a Run records what its rules did.
#[derive(Clone)]
pub(crate) struct HeldLists {
    pub(crate) digest: String,
    pub(crate) lists: Arc<FilterLists>,
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

    /// Whether it is older than [`STALE_AFTER`] at `now`, which its Run's
    /// report warns of.
    fn stale(&self, now: jiff::Timestamp) -> bool {
        self.manifest
            .published_at
            .checked_add(STALE_AFTER)
            .is_ok_and(|stale_from| stale_from <= now)
    }
}

/// Where a new Run's Clean-up rules come from, as `clean_up.update` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    Release,
    Daily,
    Upstream,
}

/// What a Run's settings ask of the Clean-up rules.
struct Asked {
    /// `clean_up.update`.
    update: Update,
    /// `clean_up.version`, when set, and the layer that set it.
    version: Option<(String, SettingsLayer)>,
    /// `clean_up.fetch_timeout`.
    timeout: Duration,
}

impl Asked {
    fn of(settings: Option<&Resolved>) -> Self {
        let update = match get_or_default::<&str>(settings, &CLEAN_UP_UPDATE) {
            "daily" => Update::Daily,
            "upstream" => Update::Upstream,
            // The setting's own validation lets only its three choices in.
            _ => Update::Release,
        };
        let version = settings
            .and_then(|settings| settings.effective(&CLEAN_UP_VERSION))
            .and_then(|effective| Some((effective.value.as_str()?.to_owned(), effective.layer)));
        Self {
            update,
            version,
            timeout: get_or_default(settings, &CLEAN_UP_FETCH_TIMEOUT),
        }
    }

    /// The digest `clean_up.version` names, when set.
    fn digest(&self) -> Option<&str> {
        self.version.as_ref().map(|(digest, _)| digest.as_str())
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

    /// The version `files` hold, kept as an artifact of `source`'s name,
    /// and held.
    pub(crate) async fn keep(
        &self,
        source: RulesSource,
        files: Vec<(String, Vec<u8>)>,
        now: jiff::Timestamp,
    ) -> Result<(Version, HeldArtifact), Unobtained> {
        let manifest = readable_manifest(&files)?;
        let cache = self.cache.clone();
        let (digest, held) = self
            .owner
            .blocking(move || cache.keep(ArtifactKind::CleanUpRules, source.as_str(), &files, now))
            .await
            .map_err(|error| Unobtained::NotKept(error.to_string()))?;
        Ok((
            Version {
                digest: digest.to_string(),
                manifest,
            },
            held,
        ))
    }

    /// The version of `digest` the cache holds, whatever its source, held.
    pub(crate) async fn cached(
        &self,
        digest: &str,
        now: jiff::Timestamp,
    ) -> Option<(Version, HeldArtifact)> {
        let parsed: Digest = digest.parse().ok()?;
        let cache = self.cache.clone();
        let found = self
            .owner
            .blocking(move || {
                [
                    RulesSource::Daily,
                    RulesSource::Upstream,
                    RulesSource::Release,
                ]
                .into_iter()
                .find_map(|source| {
                    let id = ArtifactId {
                        kind: ArtifactKind::CleanUpRules,
                        name: source.as_str().to_owned(),
                        build: parsed.hex(),
                    };
                    let held = cache.find(&id, parsed, now).ok()??;
                    let bytes = std::fs::read(held.files().join(MANIFEST_FILE)).ok()?;
                    Some((manifest_of(&bytes).ok()?, held))
                })
            })
            .await;
        found.map(|(manifest, held)| {
            (
                Version {
                    digest: digest.to_owned(),
                    manifest,
                },
                held,
            )
        })
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
    /// The daily channel's repository, `host[:port]/path`.
    pub(crate) channel: String,
    /// The URL of each Filter list `upstream` fetches, by list.
    pub(crate) lists: BTreeMap<String, String>,
}

impl Sources {
    /// The sources under `context`'s settings.
    async fn of(context: &RulesContext<'_>) -> Self {
        let settings = context.settings.map(RunSettings::resolved);
        let owner = context.owner.child();
        let channel = get_or_default::<&str>(settings, &CLEAN_UP_CHANNEL).to_owned();
        let mut lists: BTreeMap<String, String> = UPSTREAM_LISTS
            .iter()
            .map(|(list, url)| ((*list).to_owned(), (*url).to_owned()))
            .collect();
        if let Some(named) =
            settings.and_then(|settings| settings.value(&CLEAN_UP_LISTS).as_object())
        {
            for (list, url) in named {
                if let Some(url) = url.as_str() {
                    lists.insert(list.clone(), url.to_owned());
                }
            }
        }
        let mut fetching = Fetching::new(context.clock);
        if let Some(settings) = settings {
            fetching = Fetching::of(settings, context.clock);
            let files = authority_files(settings.value(&TLS_TRUSTED_CAS));
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
            channel,
            lists,
        }
    }

    /// The sources' state, as last written; an empty one when it does not
    /// read, so the sources fetch again.
    pub(crate) async fn read_state(&self) -> Value {
        let path = self.state.clone();
        let read = self.kept.owner.blocking(move || std::fs::read(path)).await;
        match read.map(|bytes| serde_json::from_slice::<Value>(&bytes)) {
            Ok(Ok(state)) => state,
            Ok(Err(error)) => {
                tracing::warn!(%error, "the Clean-up rules' sources' state does not read: they fetch again");
                json!({})
            }
            Err(_) => json!({}),
        }
    }

    /// Writes the sources' state through a partial file, synced, then
    /// renamed over the last one, so a write cut short leaves the last
    /// state whole; a state that cannot be written is logged, and the next
    /// start fetches again.
    pub(crate) async fn write_state(&self, state: Value) {
        let path = self.state.clone();
        let written = self
            .kept
            .owner
            .blocking(move || replace(&path, state.to_string().as_bytes()))
            .await;
        if let Err(error) = written {
            tracing::warn!(%error, "the Clean-up rules' sources cannot record their state");
        }
    }
}

/// Replaces `path`'s content with `bytes`: written to a hidden partial file
/// beside it, synced, renamed over it, and its folder synced.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(folder)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = folder.join(format!(".{name}.partial"));
    let mut file = std::fs::File::create(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&partial, path)?;
    if let Ok(folder) = std::fs::File::open(folder) {
        let _ = folder.sync_all();
    }
    Ok(())
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
fn manifest_of(bytes: &[u8]) -> Result<CleanUpRulesManifest, Unobtained> {
    let header: Value = serde_json::from_slice(bytes)
        .map_err(|error| Unobtained::Unreadable(format!("has no manifest: {error}")))?;
    let found = header["format_version"]
        .as_str()
        .and_then(|version| version.parse().ok())
        .ok_or_else(|| {
            Unobtained::Unreadable("has a manifest without its format version".to_owned())
        })?;
    if CLEAN_UP_RULES
        .rule
        .compatibility(CLEAN_UP_RULES.version, found)
        != Compatibility::Compatible
    {
        return Err(Unobtained::Unreadable(format!(
            "has a manifest of format {found}, which this engine, reading {}, skips",
            CLEAN_UP_RULES.version
        )));
    }
    serde_json::from_value(header).map_err(|error| {
        Unobtained::Unreadable(format!("has a manifest that does not read: {error}"))
    })
}

/// The manifest of the version `files` hold, which must name a file of
/// each list it holds.
fn readable_manifest(files: &[(String, Vec<u8>)]) -> Result<CleanUpRulesManifest, Unobtained> {
    let (_, bytes) = files
        .iter()
        .find(|(path, _)| path == MANIFEST_FILE)
        .ok_or_else(|| Unobtained::Unreadable("has no manifest".to_owned()))?;
    let manifest = manifest_of(bytes)?;
    for (list, file) in manifest.lists.files() {
        if !files.iter().any(|(path, _)| path == file) {
            return Err(Unobtained::Unreadable(format!(
                "names `{file}` for its `{list}` list, which it lacks"
            )));
        }
    }
    Ok(manifest)
}

/// Wall time now on `clock`.
fn now_of(clock: Clock) -> jiff::Timestamp {
    clock.timestamp()
}

/// `fetching`, ended as [`Unobtained::TimedOut`] once `limit` passed on
/// `clock`.
async fn within<T>(
    clock: Clock,
    limit: Duration,
    fetching: impl Future<Output = Result<T, Unobtained>>,
) -> Result<T, Unobtained> {
    tokio::select! {
        fetched = fetching => fetched,
        () = clock.sleep(limit) => Err(Unobtained::TimedOut(limit)),
    }
}

impl Versions {
    /// The Goodfetch home, kept from the first start or resume.
    fn remember(&self, home: &Home) {
        let _ = self.home.set(home.clone());
    }

    /// The version a new Run starting under `context` holds to.
    async fn choose_version(&self, context: RulesContext<'_>) -> Result<Chosen, CallError> {
        self.remember(context.home);
        let resolved = context.settings.map(RunSettings::resolved);
        refuse_unbuilt(resolved)?;
        let asked = Asked::of(resolved);
        let now = now_of(context.clock);
        if let Some((digest, layer)) = &asked.version {
            let sources = Sources::of(&context).await;
            let obtained = within(
                context.clock,
                asked.timeout,
                obtain(digest, &asked, true, &sources, now),
            )
            .await;
            return match obtained {
                Ok((version, _)) => Ok(Chosen {
                    digest: version.digest,
                    fallback: None,
                }),
                Err(failure) => Err(refusal((digest, *layer), &failure, &context)),
            };
        }
        let embedded = Chosen {
            digest: EMBEDDED_DIGEST.to_owned(),
            fallback: None,
        };
        match asked.update {
            Update::Upstream => {
                let sources = Sources::of(&context).await;
                let newest = self
                    .upstream
                    .newest(&sources, now, context.clock, asked.timeout)
                    .await;
                Ok(match newest {
                    Ok(version) => Chosen {
                        digest: version.digest,
                        fallback: None,
                    },
                    Err(failure) => Chosen {
                        fallback: Some(fallback("the Filter lists from their authors", &failure)),
                        ..embedded
                    },
                })
            }
            // Refused above until the channel's signature is checked.
            Update::Daily | Update::Release => Ok(embedded),
        }
    }

    /// Records the version `run` holds to once it started or resumed, and
    /// holds its files in the Browser cache while the Run lives.
    async fn record(&self, run: &Run, holding: Holding, context: RulesContext<'_>) {
        self.remember(context.home);
        let Some(recorded) = recorded_digest(run).await else {
            return;
        };
        let asked = Asked::of(context.settings.map(RunSettings::resolved));
        let now = now_of(context.clock);
        let resumed = holding == Holding::Resumed;
        let obtained = match holding {
            Holding::Started { fallback } => known(&recorded, &context, now)
                .await
                .map(|(version, artifact)| (version, artifact, fallback))
                .ok_or(Unobtained::NotHere),
            Holding::Resumed => {
                let sources = Sources::of(&context).await;
                let explicit = asked.digest() == Some(recorded.as_str());
                within(
                    context.clock,
                    asked.timeout,
                    obtain(&recorded, &asked, explicit, &sources, now),
                )
                .await
                .map(|(version, artifact)| (version, artifact, None))
            }
        };
        let (version, artifact, fallback) = match obtained {
            Ok(obtained) => obtained,
            Err(failure) => {
                let fallback = fallback(
                    &format!("the Run's recorded Clean-up rules, {recorded},"),
                    &failure,
                );
                (Version::embedded(), None, Some(fallback))
            }
        };
        self.hold(run, &version.digest, artifact);
        let mut entry = json!({
            "action": "clean_up_rules",
            "digest": version.digest,
            "source": version.manifest.source,
            "published_at": version.manifest.published_at,
            "applied_by": { "product": context.product, "product_version": context.product_version },
            "update": context
                .settings
                .map(RunSettings::resolved)
                .map_or_else(|| catalog_value(&CLEAN_UP_UPDATE), |settings| settings.value(&CLEAN_UP_UPDATE))
                .clone(),
            "resumed": resumed,
        });
        if let Some(fallback) = &fallback {
            entry["fallback"] = json!({ "reason": fallback.reason, "message": fallback.message });
            entry["recorded"] = json!(recorded);
        }
        trace::provenance(run, entry).await;
        if let Some(fallback) = fallback {
            warn_of(run, &fallback).await;
        }
        if version.stale(now) {
            let published = version.manifest.published_at;
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

    /// `run` holds to the version of `digest`, whose files `artifact` holds
    /// in the Browser cache, until it ends; the Runs that ended let theirs
    /// go.
    fn hold(&self, run: &Run, digest: &str, artifact: Option<HeldArtifact>) {
        held_by(&mut lock(&self.holding), run, digest, artifact);
    }

    /// `run`, whose version of `digest` did not load, holds to the embedded
    /// part from now on; whether this call moved it there, which only one of
    /// the Run's pages that asked at once does.
    fn fall_back(&self, run: &Run, digest: &str) -> bool {
        let mut holding = lock(&self.holding);
        if holding
            .get(run.id())
            .is_some_and(|held| held.digest != digest)
        {
            return false;
        }
        held_by(&mut holding, run, EMBEDDED_DIGEST, None);
        true
    }

    /// The Filter lists `run`'s pages match: the version it holds to,
    /// loaded once for every Run that holds to it, through `owner`'s
    /// blocking tasks; the embedded part, recorded in Provenance and warned
    /// of once, however many of its pages asked at once, when its files no
    /// longer read. Each comes with its digest.
    pub(crate) async fn lists(&self, run: &Run, owner: &TaskOwner) -> HeldLists {
        let held = lock(&self.holding)
            .get(run.id())
            .map(|held| (held.digest.clone(), held.artifact.clone()));
        let (digest, artifact) = match held {
            Some(held) => held,
            None => (
                recorded_digest(run)
                    .await
                    .unwrap_or_else(|| EMBEDDED_DIGEST.to_owned()),
                None,
            ),
        };
        if digest != EMBEDDED_DIGEST {
            let loading = || self.load(&digest, artifact.clone(), run, owner);
            match self.loaded_once(&digest, loading).await {
                Ok(lists) => return HeldLists { digest, lists },
                Err(failure) if self.fall_back(run, &digest) => {
                    let fallback =
                        fallback(&format!("the Run's Clean-up rules, {digest},"), &failure);
                    trace::provenance(
                        run,
                        json!({
                            "action": "clean_up_rules",
                            "digest": EMBEDDED_DIGEST,
                            "source": RulesSource::Embedded,
                            "recorded": digest,
                            "fallback": { "reason": fallback.reason, "message": fallback.message },
                        }),
                    )
                    .await;
                    warn_of(run, &fallback).await;
                }
                // Another of the Run's pages fell back and recorded it.
                Err(_) => {}
            }
        }
        let embedded = || async {
            Ok(owner.blocking(FilterLists::embedded).await.expect(
                "invariant: the build compiled the embedded Filter lists with this binary's own adblock",
            ))
        };
        let lists = self
            .loaded_once(EMBEDDED_DIGEST, embedded)
            .await
            .expect("invariant: the embedded part's loads never fail");
        HeldLists {
            digest: EMBEDDED_DIGEST.to_owned(),
            lists,
        }
    }

    /// The version of `digest`, loaded by `load` unless a Run still holds
    /// it loaded or the engine kept it: the loads of one digest run one at a
    /// time on its cell, so each version is compiled once for every Run that
    /// holds to it, and once for the Runs after while it is among the
    /// [`KEPT_VERSIONS`] used last; a load that failed fails the loads that
    /// waited on it with it, and leaves the next to try again.
    async fn loaded_once<Load, Loading>(
        &self,
        digest: &str,
        load: Load,
    ) -> Result<Arc<FilterLists>, Unobtained>
    where
        Load: Fn() -> Loading,
        Loading: Future<Output = Result<FilterLists, Unobtained>>,
    {
        loop {
            let cell = {
                let mut loaded = lock(&self.loaded);
                loaded.retain(|_, cell| {
                    cell.get().is_none_or(|lists| {
                        lists.as_ref().is_ok_and(|lists| lists.strong_count() > 0)
                    })
                });
                Arc::clone(loaded.entry(digest.to_owned()).or_default())
            };
            let mut compiled = None;
            let weak = cell
                .get_or_init(|| async {
                    let lists = Arc::new(load().await?);
                    let weak = Arc::downgrade(&lists);
                    compiled = Some(lists);
                    Ok(weak)
                })
                .await
                .as_ref()
                .map_err(Clone::clone)?;
            // A version another load compiled whose Runs all ended since,
            // and which the engine no longer keeps, is loaded again.
            if let Some(lists) = compiled.or_else(|| weak.upgrade()) {
                self.keep(digest, &lists);
                return Ok(lists);
            }
        }
    }

    /// Keeps `lists`, the version of `digest`, as the one used last, and
    /// lets go of the versions past [`KEPT_VERSIONS`], outside the lock.
    fn keep(&self, digest: &str, lists: &Arc<FilterLists>) {
        let gone = {
            let mut kept = lock(&self.kept);
            if kept.front().is_some_and(|(newest, _)| newest == digest) {
                return;
            }
            kept.retain(|(kept_digest, _)| kept_digest != digest);
            kept.push_front((digest.to_owned(), Arc::clone(lists)));
            let past = KEPT_VERSIONS.min(kept.len());
            kept.split_off(past)
        };
        drop(gone);
    }

    /// Loads the version of `digest` from its texts in the cache, which
    /// `artifact` holds when its Run's start or resume found them, compiled
    /// here.
    async fn load(
        &self,
        digest: &str,
        artifact: Option<Arc<HeldArtifact>>,
        run: &Run,
        owner: &TaskOwner,
    ) -> Result<FilterLists, Unobtained> {
        let found = match (artifact, self.home.get()) {
            (Some(artifact), _) => Some(artifact.files()),
            (None, Some(home)) => Kept::in_home(home, owner.child())
                .cached(digest, now_of(run.clock()))
                .await
                .map(|(_, held)| held.files()),
            (None, None) => None,
        };
        let files = found.ok_or(Unobtained::NotLoaded("it is gone".to_owned()))?;
        owner
            .blocking(move || -> std::io::Result<FilterLists> {
                let manifest = manifest_of(&std::fs::read(files.join(MANIFEST_FILE))?)
                    .map_err(|failure| std::io::Error::other(failure.to_string()))?;
                let lists = manifest.lists;
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
            .await
            .map_err(|error| Unobtained::NotLoaded(error.to_string()))
    }
}

/// `run` holds to the version of `digest` in `holding`, whose files
/// `artifact` holds in the Browser cache, until it ends; the Runs that
/// ended let theirs go.
fn held_by(
    holding: &mut BTreeMap<RunId, Held>,
    run: &Run,
    digest: &str,
    artifact: Option<HeldArtifact>,
) {
    holding.retain(|_, held| !held.live.is_cancelled());
    holding.insert(
        run.id().clone(),
        Held {
            live: run.cancellation().clone(),
            digest: digest.to_owned(),
            artifact: artifact.map(Arc::new),
        },
    );
}

/// Warns in `run`'s report, once, that its Clean-up rules are not the
/// version asked for, for `fallback`.
async fn warn_of(run: &Run, fallback: &Fallback) {
    let warning = RunWarning {
        code: CLEAN_UP_RULES_FALLBACK.to_owned(),
        message: format!(
            "The Run's Clean-up rules are not the version asked for: {}.",
            fallback.message
        ),
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warn_once(warning).await {
        tracing::warn!(%error, "the warning of the Run's Clean-up rules was not kept");
    }
}

/// The version of `digest`, held unless it is the embedded part: the
/// embedded part, one the cache holds, or one fetched from the daily
/// channel where it is configured or `explicit`, named by
/// `clean_up.version`.
async fn obtain(
    digest: &str,
    asked: &Asked,
    explicit: bool,
    sources: &Sources,
    now: jiff::Timestamp,
) -> Result<(Version, Option<HeldArtifact>), Unobtained> {
    if digest == EMBEDDED_DIGEST {
        return Ok((Version::embedded(), None));
    }
    if digest.parse::<Digest>().is_err() {
        return Err(Unobtained::NotADigest);
    }
    if let Some((version, held)) = sources.kept.cached(digest, now).await {
        return Ok((version, Some(held)));
    }
    if explicit || asked.update == Update::Daily {
        let (version, held) = daily::by_digest(sources, digest, now).await?;
        return Ok((version, Some(held)));
    }
    Err(Unobtained::NotHere)
}

/// The version of `digest` this machine holds, held unless it is the
/// embedded part: the embedded part or one the cache keeps.
async fn known(
    digest: &str,
    context: &RulesContext<'_>,
    now: jiff::Timestamp,
) -> Option<(Version, Option<HeldArtifact>)> {
    if digest == EMBEDDED_DIGEST {
        return Some((Version::embedded(), None));
    }
    Kept::in_home(context.home, context.owner.child())
        .cached(digest, now)
        .await
        .map(|(version, held)| (version, Some(held)))
}

/// The Clean-up rules digest `run`'s state records.
async fn recorded_digest(run: &Run) -> Option<String> {
    run.state()
        .read(run_row::clean_up_digest)
        .await
        .ok()
        .flatten()
}

impl CleanUpVersions for Versions {
    fn choose<'a>(&'a self, context: RulesContext<'a>) -> Answering<'a, Result<Chosen, CallError>> {
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

#[cfg(test)]
mod tests;
