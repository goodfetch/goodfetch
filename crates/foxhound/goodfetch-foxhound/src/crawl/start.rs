//! Starting a crawl's Run (`crawl.start`, ADR 0093): [`CrawlTarget`] names
//! what [`Foxhound::start_crawl`](crate::Foxhound::start_crawl) accepts
//! (`&`[`Crawl`], [`CrawlDeclaration`] or [`CrawlExtraction`]), and
//! [`StartCrawl`] builds and sends `crawl.start`.

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_collie_client::{ClientConnection, Error, Limits};
use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::crawl::{
    CODE_FREE_LABEL, CRAWL_START, CrawlExtraction, CrawlStartParams, CrawlStarted,
};
use goodfetch_protocol::foxhound::declaration::CrawlDeclaration;
use goodfetch_protocol::run::{RunId, Tags};

use super::{Crawl, CrawlId, CrawlRun, code_free_params};
use crate::Seed;
use crate::crawl_requests::CrawlRequests;
use crate::settings::Settings;

/// What [`Foxhound::start_crawl`](crate::Foxhound::start_crawl) starts (ADR 0093): a `&`[`Crawl`] the
/// program serves, or a code-free crawl the engine handles itself, a
/// [`CrawlDeclaration`] or a [`CrawlExtraction`], whose seeds name their
/// Labels by name ([`Seed::named`], [`Seed::named_sitemap`]). Only this
/// crate implements it.
pub trait CrawlTarget: target::Parts {}

/// What a crawl's start takes from its [`CrawlTarget`], which only this
/// crate implements.
pub(crate) mod target {
    use goodfetch_collie_client::Error;
    use goodfetch_protocol::foxhound::crawl::CrawlStartParams;

    use super::CrawlId;
    use crate::Seed;

    /// What a crawl's start sends and keeps.
    #[derive(Debug)]
    pub struct Started {
        /// The crawl's number.
        pub(crate) id: CrawlId,
        /// The Labels the crawl declares.
        pub(crate) declared: Vec<String>,
        /// `crawl.start`'s params from the seeds, or why it cannot start.
        pub(crate) params: Result<CrawlStartParams, Error>,
    }

    /// A crawl a start sends.
    pub trait Parts {
        /// What the start of this crawl from `seeds` sends.
        fn parts(self, seeds: Vec<Seed>) -> Started;
    }
}

impl target::Parts for &Crawl<'_> {
    fn parts(self, seeds: Vec<Seed>) -> target::Started {
        target::Started {
            id: self.id,
            declared: self.declared(),
            params: self.start_params(seeds),
        }
    }
}

impl CrawlTarget for &Crawl<'_> {}

impl target::Parts for CrawlDeclaration {
    fn parts(self, seeds: Vec<Seed>) -> target::Started {
        let declared = self.labels.iter().map(|label| label.name.clone()).collect();
        let params = code_free_params(seeds).map(|params| CrawlStartParams {
            declaration: Some(self),
            ..params
        });
        target::Started {
            id: CrawlId::next(),
            declared,
            params,
        }
    }
}

impl CrawlTarget for CrawlDeclaration {}

impl target::Parts for CrawlExtraction {
    fn parts(self, seeds: Vec<Seed>) -> target::Started {
        let params = code_free_params(seeds).map(|params| CrawlStartParams {
            extraction: Some(self),
            ..params
        });
        target::Started {
            id: CrawlId::next(),
            declared: vec![CODE_FREE_LABEL.to_owned()],
            params,
        }
    }
}

impl CrawlTarget for CrawlExtraction {}

/// The start of a crawl's Run (`crawl.start`), which awaiting sends.
/// Nothing is sent until it is awaited.
#[must_use = "a crawl's start is sent only when it is awaited"]
#[derive(Debug)]
pub struct StartCrawl {
    connection: ClientConnection,
    requests: Arc<CrawlRequests>,
    crawl: CrawlId,
    declared: Arc<[String]>,
    /// The start's params, or why the crawl cannot start.
    params: Result<CrawlStartParams, Error>,
    /// The Run's own settings, which its params carry once checked.
    settings: Settings,
}

impl StartCrawl {
    /// The start of `crawl`'s Run from `seeds` on `connection`, whose
    /// Runs share `requests`. A code-free crawl takes a number of its own,
    /// so no [`Crawl`] serves its Run.
    pub(crate) fn new(
        connection: &ClientConnection,
        requests: &Arc<CrawlRequests>,
        crawl: impl CrawlTarget,
        seeds: impl IntoIterator<Item = Seed>,
    ) -> Self {
        let target::Started {
            id,
            declared,
            params,
        } = target::Parts::parts(crawl, seeds.into_iter().collect());
        Self {
            connection: connection.clone(),
            requests: Arc::clone(requests),
            crawl: id,
            declared: declared.into(),
            params,
            settings: Settings::new(),
        }
    }

    /// Lets the Run outlive the Connection, as
    /// [`HandlerlessStart::detached`](crate::HandlerlessStart::detached)
    /// does: closing or dropping its [`CrawlRun`] then sends no
    /// `run.abort`. Only a code-free crawl started on a Daemon detaches;
    /// any other start fails with `detach_unsupported` (ADR 0102).
    pub fn detached(mut self) -> Self {
        if let Ok(params) = &mut self.params {
            params.detached = true;
        }
        self
    }

    /// Names the Run for good, as [`Start::run_id`](crate::Start::run_id)
    /// does: a start with an existing Run ID resumes the Run if it was
    /// interrupted, or returns it as it ended (ADR 0102).
    pub fn run_id(mut self, run_id: &RunId) -> Self {
        if let Ok(params) = &mut self.params {
            params.run_id = Some(run_id.as_str().to_owned());
        }
        self
    }

    /// Tags the Run, in place of any tags given before, as
    /// [`Start::tags`](crate::Start::tags) does (ADR 0036).
    pub fn tags(mut self, tags: Tags) -> Self {
        if let Ok(params) = &mut self.params {
            params.tags = tags;
        }
        self
    }

    /// Sets the Run's own `limits`, in place of any given before, as
    /// [`Start::limits`](crate::Start::limits) does: each only lowers what
    /// covers the Run, and a start that names the Run again lowers its
    /// Limits and never raises them (ADRs 0080 and 0102).
    pub fn limits(mut self, limits: Limits) -> Self {
        if let Ok(params) = &mut self.params {
            params.limits = limits.into();
        }
        self
    }

    /// Sets the Run's own settings, in place of any given before, as
    /// [`Start::settings`](crate::Start::settings) does: what they leave
    /// out the connection's settings give, and its Labels' own settings
    /// override them for their requests (ADRs 0083 and 0093). A key a
    /// Run's start does not set fails the start with
    /// `setting_not_applicable` before anything is sent.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }
}

impl IntoFuture for StartCrawl {
    type Output = Result<CrawlRun, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<CrawlRun, Error>> + Send + 'static>>;

    /// Opens the connection's stream of Crawl requests before the start
    /// is sent, so the engine's first request finds it. The start carries
    /// the project file the connection carries (ADR 0083).
    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            requests,
            crawl,
            declared,
            params,
            settings,
        } = self;
        let started = params.and_then(|mut params| {
            params.settings = settings.within(SettingScope::Run)?;
            params.project = connection.project().cloned();
            requests.open(&connection)?;
            let detached = params.detached;
            Ok(connection.start_held_with(
                CRAWL_START.name,
                params,
                detached,
                |started: CrawlStarted| (started.started, started.seeds),
            ))
        });
        Box::pin(async move {
            let (run, seeds) = started?.await?;
            Ok(CrawlRun {
                run,
                connection,
                requests,
                crawl,
                declared,
                seeds: seeds.into(),
            })
        })
    }
}
