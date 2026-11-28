//! The version of the Clean-up rules each Run holds to (ADR 0047), which
//! the composition's Clean-up rules choose: before a new Run starts, the
//! digest it records in its run state and keeps for its whole life, a
//! resume included; once it is live, started or resumed, what its
//! Provenance and its report say of that version.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::RunKind;

use super::RunParts;
use crate::CallError;
use crate::clock::Clock;
use crate::home::Home;
use crate::settings::RunSettings;
use crate::tasks::TaskOwner;

/// What the engine tells the Clean-up rules of the Run they serve.
#[derive(Clone, Copy)]
pub struct RulesContext<'a> {
    /// The Run's resolved settings, when its start resolved them.
    pub settings: Option<&'a RunSettings>,
    /// The Run's kind.
    pub kind: RunKind,
    /// The Goodfetch home.
    pub home: &'a Home,
    /// The engine's clock.
    pub clock: Clock,
    /// The Product the engine runs as.
    pub product: Product,
    /// Its Product version: the release whose code applies the rules.
    pub product_version: &'a ProductVersion,
    /// The owner of the blocking work the rules do, such as reading their
    /// files and compiling them (ADR 0060).
    pub owner: &'a TaskOwner,
}

/// The version a new Run holds to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    /// Its digest, which the Run's state records.
    pub digest: String,
    /// Why it is not the version the Run's settings ask for, when it is not:
    /// what [`CleanUpVersions::held`] records in Provenance and warns of.
    pub fallback: Option<Fallback>,
}

/// Why a Run holds to another version than the one its settings ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    /// The stable name of the reason, which Provenance records.
    pub reason: &'static str,
    /// What happened, in words.
    pub message: String,
}

/// How a live Run came to hold its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Holding {
    /// It started with the version [`CleanUpVersions::choose`] chose, and
    /// the fallback that choice gave, if any.
    Started {
        /// Why it is not the version its settings ask for, when it is not.
        fallback: Option<Fallback>,
    },
    /// It resumed, keeping the version its run state records.
    Resumed,
}

/// The future one of [`CleanUpVersions`]' calls answers.
pub type Answering<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The composition's Clean-up rules, as the Runs ask them for the version
/// each Run holds to (ADR 0047).
pub trait CleanUpVersions: Send + Sync {
    /// The version a new Run holds to, chosen under its settings before it
    /// starts: a version `clean_up.update` asks for that cannot be had
    /// leaves the newest one the engine has, and says why.
    ///
    /// # Errors
    ///
    /// The refusal of a start whose settings ask for what the engine cannot
    /// give, such as a `clean_up.version` it cannot have, which the start
    /// answers.
    fn choose<'a>(&'a self, context: RulesContext<'a>) -> Answering<'a, Result<Chosen, CallError>>;

    /// Records, once `run` started or resumed, the version it holds to, as
    /// its run state names it: in its Provenance, and in its report's
    /// warnings when it is old or not the one asked for. `holding` says how it
    /// came to hold it.
    fn held<'a>(
        &'a self,
        run: &'a RunParts,
        holding: Holding,
        context: RulesContext<'a>,
    ) -> Answering<'a, ()>;

    /// Checks, at a Safe point of `run` under `clean_up.follow_latest`,
    /// whether a newly published Clean-up rules version is available; when
    /// one is, adopts it for `run`, drops its `filter_lists.host_cache`
    /// entries, records the switch in run state and Provenance, and returns
    /// what switched (ADRs 0047 and 0139).
    fn refresh<'a>(
        &'a self,
        _run: &'a RunParts,
        _context: RulesContext<'a>,
    ) -> Answering<'a, Option<Switched>> {
        Box::pin(async { None })
    }
}

/// One Clean-up rules version switch adopted at a Safe point under
/// `clean_up.follow_latest` (ADRs 0047 and 0139).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switched {
    /// The digest the Run held before the switch.
    pub from_version: String,
    /// The digest the Run adopted.
    pub to_version: String,
    /// When the adopted version was published.
    pub published_at: SystemTime,
    /// When the Run adopted it at its Safe point.
    pub switched_at: SystemTime,
}

/// A live Run's handle to check for and adopt a newer Clean-up rules version
/// at a Safe point under `clean_up.follow_latest` (ADRs 0047 and 0139).
#[derive(Clone)]
pub struct CleanUpRefresher {
    pub(super) versions: Arc<dyn CleanUpVersions>,
    pub(super) run: RunParts,
    pub(super) kind: RunKind,
    pub(super) home: Home,
    pub(super) clock: Clock,
    pub(super) product: Product,
    pub(super) product_version: ProductVersion,
    pub(super) owner: Arc<TaskOwner>,
}

impl CleanUpRefresher {
    /// Checks for a newly published Clean-up rules version and adopts it at
    /// this Safe point when one is available.
    pub async fn refresh(&self) -> Option<Switched> {
        let context = RulesContext {
            settings: self.run.identity.settings(),
            kind: self.kind,
            home: &self.home,
            clock: self.clock,
            product: self.product,
            product_version: &self.product_version,
            owner: &self.owner,
        };
        self.versions.refresh(&self.run, context).await
    }
}
