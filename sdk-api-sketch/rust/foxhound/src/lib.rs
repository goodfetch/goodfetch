//! `goodfetch-foxhound` (ADRs 0090, 0095): Foxhound's Rust SDK. It re-exports the Collie client
//! crate, so a handler's [`Page`] is the Page a Collie program gets, and adds Foxhound's connection,
//! [`Foxhound`], with Foxhound's engine linked in (ADR 0024), and the values only Foxhound hands out:
//! crawls and their contexts, Datasets, Records, Exports, Hooks and Politeness. Foxhound's own Error
//! codes are the free constants of [`error_code`]; its settings keys, with Collie's, are in
//! [`settings`].
//!
//! SDK API sketch: signatures only, no implementation. In the SDK, Foxhound's contract types are
//! `goodfetch-protocol`'s `foxhound` module, which only this crate re-exports (ADR 0099); the
//! sketch defines them here.

pub use goodfetch_collie_client::*;
pub mod blocking;
pub mod connection;
pub mod crawl;
pub mod data;
pub mod error_code;
pub mod hooks;
pub mod outcomes;
pub mod output;
pub mod run;
pub mod settings;
#[cfg(feature = "test-util")]
pub mod testing;
pub mod values;

pub use connection::{ConnectOptions, Foxhound};
pub use crawl::{
    Crawl, CrawlCall, CrawlContext, CrawlRequest, CrawlRun, DatasetDeclaration, Enqueue,
    EnqueueLinks, Enqueued, EnqueuedLinks, FetchTier, LabelDeclaration, LinkInfo, NotAddedReason,
    Seed, StartCrawl, UrlPattern,
};
pub use data::{Dataset, Label, PayloadInvalid, PayloadSchemaChanged};
pub use hooks::{
    AfterResponseContext, BackendRequest, BackendResponse, BeforeRequestContext, FetchCall,
    HeaderForbidden, HookAnswer, HookCall, HookDuplicate, HookFailed, HookIneffective, HookOn,
    PageStageContext, RecordCommitContext, RecordHookOn, RequestChange, ResponseChange, SinkCall,
    UntypedRecord,
};
pub use outcomes::{
    CrawlRequestOutcome, CrawlRequestOutcomeKind, CrawlRequestRef, HandlerFailed, SkipReason,
};
pub use output::{
    Compression, DatasetDuplicate, DatasetKeyInvalid, DatasetRecord, DatasetUnknown, Export,
    ExportCall, ExportFormat, RecordCounts, RecordInvalid, RejectedRecord, SinkConfig,
    SinkWriteMode,
};
pub use run::{
    CrawlConcurrency, CrawlRequestCounts, CrawlRequestTotals, CrawlRunProgress, CrawlRunReport,
    CrawlRunView, CrawlUncertainEffect, DatasetTotals, EndedRequestBreakdown, GovernorState,
    InFlightRequest, LearnedRule, OutcomeKind, OutcomesFilter, RequestPhase, RetryFailed,
    SinkDeliveryStatus, SitemapTotals, UnreadSitemap, WaitGroup, WaitReason, WaitingDomain,
};
pub use settings::Settings;
pub use values::{hook_stage, limit_scope, usage_kind};
