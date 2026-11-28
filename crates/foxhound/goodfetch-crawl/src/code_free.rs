//! Code-free crawls (spec #582; ADR 0103): a `crawl.start` with no handler
//! takes `extraction`, and the engine handles each of its requests itself.
//! The start declares the Label `page` and the Dataset `pages`, keyed by
//! `url`, whose JSON Schema is `page.formats`' schema of the formats
//! request (Collie's `page_formats_schema`) with the Record's `status` and
//! `fetched_at`; a start that also declares a Dataset is refused with
//! `invalid_params`. Each attempt reads the formats of the page its tier
//! served, as `page.formats` answers them, and answers one Record of
//! `pages`, committed under its lease as a handler's answer is; under
//! `follow: scope` it first admits every `a[href]` link of the page as
//! `enqueueLinks` does, staged with the attempt, and under `follow:
//! sitemap` the start seeds the sitemaps each seed's site names in its
//! robots.txt, as a sitemap seed of that robots.txt. The extraction is
//! kept in the run file, so a resumed crawl handles its requests alike; a
//! start under the Run's ID replaces it with its own once the Run resumes,
//! as it replaces the Dataset `pages` generated from it.
//! Its named `fields`, value fields, lists and rows, are found in the page
//! as `page.read` finds them and read by the extractor's `fields` module
//! (ADRs 0123 and 0147): the Record holds each, null where the page did
//! not give it, and its Provenance names each transform and why each null
//! value is null.
//! A start may take a `declaration` in place of `extraction`, its crawl as
//! data (spec #611's #1514): its Labels, Datasets, settings and seeds join
//! the start's, and each page of a declared Label follows and extracts as
//! that Label declares (the `declared` module). It is kept in the run file
//! as an extraction is.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_counted_work::CountedWork;
use goodfetch_dataset::Datasets;
use goodfetch_engine::run::settings_error;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_extractor::fields::{self, Fields};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_frontier::Url;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::{
    CODE_FREE_DATASET, CODE_FREE_LABEL, CrawlExtraction, CrawlRequestHandleParams,
    CrawlRequestHandled, CrawlStartParams, ExtractionFields, ExtractionFollow, LabelDeclaration,
    PushedRecord, Seed, SitemapRule, SitemapSeed,
};
use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
use goodfetch_protocol::foxhound::declaration::{CrawlDeclaration, FollowRule};
use goodfetch_protocol::foxhound::frontier::CrawlRequestEnqueueLinksParams;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::read::{FieldLocator, PageField};
use goodfetch_protocol::page::{FormatsRequest, Locator, PageFormats, PageId, page_formats_schema};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::enqueue::links::defer_links;
use crate::serving::Crawls;
use crate::work_items::{CheckedSeed, FieldsRead};

/// The field of a Record that holds when its page was read: each run of
/// an attempt reads at its own time, so a sample's comparison leaves it
/// out (ADR 0020).
pub(crate) const FETCHED_AT: &str = "fetched_at";

/// The fields of a code-free crawl's Record that two runs of one attempt
/// never hold alike, which a sample's comparison leaves out: `fetched_at`.
pub(crate) static NOT_COMPARED: std::sync::LazyLock<std::collections::BTreeSet<String>> =
    std::sync::LazyLock::new(|| std::collections::BTreeSet::from([FETCHED_AT.to_owned()]));

/// The locator whose links `follow: scope` admits from each page.
const LINKS: &str = "a[href]";

/// The key of the Dataset `pages`.
const KEY: &str = "url";

/// The pattern of a sitemap seed's rule that every entry matches.
const EVERY_ENTRY: &str = "**";

mod declared;

/// What a code-free crawl's start gave the engine to handle its requests
/// with: an extraction, or a crawl declared as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CodeFreeCrawl {
    Extraction(CrawlExtraction),
    Declaration(CrawlDeclaration),
}

impl CodeFreeCrawl {
    /// What a start naming `extraction` and `declaration` handles its
    /// requests with; none for a crawl whose program handles them. A start
    /// naming both was refused before.
    pub(crate) fn of(
        extraction: Option<CrawlExtraction>,
        declaration: Option<CrawlDeclaration>,
    ) -> Option<Self> {
        extraction
            .map(Self::Extraction)
            .or_else(|| declaration.map(Self::Declaration))
    }
}

/// What a code-free crawl's attempts handle their requests with, built
/// once as the crawl starts, and the live crawls and tiers whose pages
/// they read.
#[derive(Clone)]
pub(crate) struct CodeFree {
    handles: Handles,
    /// Whether the crawl keeps Records, its `dataset.records`: off, a page's
    /// formats and fields are not read for one (ADR 0138).
    keeps_records: bool,
    pub(crate) crawls: Crawls,
    pub(crate) tiers: FetchTiers,
}

/// How a code-free crawl handles each request: by its extraction, or as
/// its declaration handles the request's Label.
#[derive(Clone)]
enum Handles {
    Extraction(Arc<ExtractionHandling>),
    Declaration(Arc<declared::LabelHandlings>),
}

/// What an extraction's pages are handled with, built once as the crawl
/// starts or resumes.
struct ExtractionHandling {
    extraction: CrawlExtraction,
    /// What its fields read with; none when the crawl keeps no Record.
    reader: Option<Fields>,
    /// The rule that follows every link of its pages under `follow:
    /// scope`; none otherwise.
    scope: Option<FollowRule>,
}

/// The tables that keep a code-free crawl's extraction, or its
/// declaration, in every run file; a crawl whose program handles its
/// requests keeps no row.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-code-free",
    statements: &[
        "CREATE TABLE IF NOT EXISTS code_free_extraction (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            extraction TEXT NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS code_free_declaration (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            declaration TEXT NOT NULL
        ) STRICT",
    ],
};

/// Records `crawl`, the code-free crawl's start's; nothing for a crawl
/// whose program handles its requests.
///
/// # Errors
///
/// The run file's, when the row cannot be written.
pub(crate) fn record(
    connection: &Connection,
    crawl: Option<&CodeFreeCrawl>,
) -> rusqlite::Result<()> {
    let (statement, kept) = match crawl {
        None => return Ok(()),
        Some(CodeFreeCrawl::Extraction(extraction)) => (
            "INSERT OR REPLACE INTO code_free_extraction (id, extraction) VALUES (1, ?1)",
            serde_json::to_string(extraction).expect("invariant: an extraction encodes as JSON"),
        ),
        Some(CodeFreeCrawl::Declaration(declaration)) => (
            "INSERT OR REPLACE INTO code_free_declaration (id, declaration) VALUES (1, ?1)",
            serde_json::to_string(declaration).expect("invariant: a declaration encodes as JSON"),
        ),
    };
    connection
        .prepare_cached(statement)
        .and_then(|mut cached| cached.execute(params![kept]))?;
    Ok(())
}

/// Records `crawl` again, a start's under the ID of the Run that resumes,
/// in place of the one the run file holds: none takes it out, for a crawl
/// whose program handles its requests from then on.
///
/// # Errors
///
/// The run file's, when the row cannot be written.
pub(crate) fn record_again(
    connection: &Connection,
    crawl: Option<&CodeFreeCrawl>,
) -> rusqlite::Result<()> {
    for statement in [
        "DELETE FROM code_free_extraction",
        "DELETE FROM code_free_declaration",
    ] {
        connection
            .prepare_cached(statement)
            .and_then(|mut cached| cached.execute([]))?;
    }
    record(connection, crawl)
}

/// The crawl [`record`] kept; none for a crawl whose program handles its
/// requests, or one whose row no longer reads.
///
/// # Errors
///
/// The run file's, when the row cannot be read.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Option<CodeFreeCrawl>> {
    if let Some(extraction) = kept(
        connection,
        "SELECT extraction FROM code_free_extraction WHERE id = 1",
    )? {
        return Ok(Some(CodeFreeCrawl::Extraction(extraction)));
    }
    Ok(kept(
        connection,
        "SELECT declaration FROM code_free_declaration WHERE id = 1",
    )?
    .map(CodeFreeCrawl::Declaration))
}

/// The value the row `query` selects holds; none when there is no row, or
/// when it no longer reads.
fn kept<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    query: &str,
) -> rusqlite::Result<Option<T>> {
    let kept: Option<String> = connection
        .prepare_cached(query)
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))
        .optional()?;
    Ok(kept.and_then(|kept| match serde_json::from_str(&kept) {
        Ok(read) => Some(read),
        Err(error) => {
            tracing::warn!(%error, "the code-free crawl kept in the run file no longer reads");
            None
        }
    }))
}

/// Declares in `params` what a code-free crawl commits into: for a
/// declaration, its Labels, Datasets, settings and seeds beside the
/// start's, checked on `work`, the engine's Counted work (the `declared`
/// module); for an extraction, the Label `page`, beside any the start
/// declares, and the Dataset `pages`, the start's only one, which it
/// answers built, as `pages` keeps it for the extraction, its fields
/// checked on `work`; none for a crawl whose program handles its requests.
/// `invalid_params` when an extraction's start declares a Dataset of its
/// own. A declaration is checked for the engine `context` calls, which
/// answers a setting it may not set.
pub(crate) async fn declare(
    params: &mut CrawlStartParams,
    pages: &Pages,
    work: &CountedWork,
    context: &CallContext,
) -> Result<Option<Datasets>, CallError> {
    let refused = |error: &_| settings_error(error, context);
    declared::merge(params, context.identity().product, work, &refused)?;
    let Some(extraction) = &params.extraction else {
        return Ok(None);
    };
    declare_into(
        extraction,
        &mut params.labels,
        &mut params.datasets,
        pages,
        work,
    )
    .await
    .map(Some)
}

async fn declare_into(
    extraction: &CrawlExtraction,
    labels: &mut BTreeMap<String, LabelDeclaration>,
    datasets: &mut Vec<DatasetDeclaration>,
    pages: &Pages,
    work: &CountedWork,
) -> Result<Datasets, CallError> {
    if let Some(declared) = datasets.first() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "A crawl with `extraction` declares its own Dataset, `{CODE_FREE_DATASET}`, so it takes none of the program's, such as `{}`: leave `datasets` out, or handle the requests with a handler.",
                declared.name
            ),
        ));
    }
    let built = pages.of(extraction, work).await?;
    labels.entry(CODE_FREE_LABEL.to_owned()).or_default();
    datasets.push(built.declaration.clone());
    Ok(built.datasets.clone())
}

/// The Dataset `pages` of a crawl reading `extraction`: keyed by `url`,
/// its schema [`page_schema`]'s, generated once, with each of its fields,
/// its transform's type or null, required beside them; its fields checked
/// on `work`, the engine's Counted work.
///
/// # Errors
///
/// `invalid_params` for a field whose name a property of every Record
/// takes, or one the engine could never read.
pub(crate) fn dataset(
    extraction: &CrawlExtraction,
    work: &CountedWork,
) -> Result<DatasetDeclaration, CallError> {
    let mut schema = page_schema(&extraction.formats);
    let refused = |message| CallError::protocol(ProtocolFailure::InvalidParams, message);
    if let Some(name) = extraction
        .fields
        .by_name()
        .keys()
        .find(|name| schema["properties"].get(name.as_str()).is_some())
    {
        return Err(refused(format!(
            "The field `{name}` takes the name of a property every Record of `{CODE_FREE_DATASET}` holds: name it otherwise."
        )));
    }
    goodfetch_fetch_tier::check_fields(extraction.fields.by_name(), work).map_err(refused)?;
    if let Some(properties) = schema["properties"].as_object_mut() {
        for (name, field) in extraction.fields.by_name() {
            properties.insert(name.clone(), fields::schema(field));
        }
    }
    if let Some(required) = schema["required"].as_array_mut() {
        required.extend(
            extraction
                .fields
                .by_name()
                .keys()
                .cloned()
                .map(Value::String),
        );
    }
    Ok(DatasetDeclaration {
        name: CODE_FREE_DATASET.to_owned(),
        schema: Some(schema),
        key: vec![KEY.to_owned()],
    })
}

/// The Dataset `pages` built for one extraction: its declaration, and the
/// Datasets it is the only one of, its schema's validator compiled.
pub(crate) struct Built {
    declaration: DatasetDeclaration,
    datasets: Datasets,
}

/// What an extraction's Dataset `pages` follows from: its formats request
/// and its fields.
type Read = (FormatsRequest, ExtractionFields);

/// How many extractions' Datasets `pages` an engine keeps.
const KEPT: usize = 8;

/// The Datasets `pages` kept, each with the extraction it was built for.
type Kept = VecDeque<(Read, Arc<Built>)>;

/// The Datasets `pages` this engine built for the extractions its latest
/// code-free starts read, up to [`KEPT`], the latest last: a start reading
/// one of them again shares it, so its schema is generated and its
/// validator compiled once (issue 2204). A clone shares them.
#[derive(Clone, Default)]
pub(crate) struct Pages(Arc<Mutex<Kept>>);

impl Pages {
    /// The Dataset `pages` of a crawl reading `extraction`, built as
    /// [`dataset`] declares it on `work` unless kept.
    ///
    /// # Errors
    ///
    /// Those of [`dataset`] and of [`Datasets::declare`].
    pub(crate) async fn of(
        &self,
        extraction: &CrawlExtraction,
        work: &CountedWork,
    ) -> Result<Arc<Built>, CallError> {
        let mut kept = self.0.lock().await;
        let read = |(formats, fields): &Read| {
            *formats == extraction.formats && *fields == extraction.fields
        };
        if let Some(at) = kept.iter().position(|(key, _)| read(key)) {
            let found = kept
                .remove(at)
                .expect("invariant: the position was just found");
            let built = Arc::clone(&found.1);
            kept.push_back(found);
            return Ok(built);
        }
        let declaration = dataset(extraction, work)?;
        let datasets = Datasets::declare(std::slice::from_ref(&declaration))?;
        let built = Arc::new(Built {
            declaration,
            datasets,
        });
        kept.push_back((
            (extraction.formats, extraction.fields.clone()),
            Arc::clone(&built),
        ));
        while kept.len() > KEPT {
            kept.pop_front();
        }
        Ok(built)
    }
}

/// What every Record of a crawl reading `formats` holds: `page.formats`'
/// schema of the request with `status` and `fetched_at` required beside it.
fn page_schema(formats: &FormatsRequest) -> Value {
    let mut schema = page_formats_schema(formats);
    if let Some(properties) = schema["properties"].as_object_mut() {
        properties.insert(
            "status".to_owned(),
            json!({
                "description": "The status of the page's response; null when the browser's navigation answered none.",
                "type": ["integer", "null"],
                "minimum": 100,
                "maximum": 999,
            }),
        );
        properties.insert(
            FETCHED_AT.to_owned(),
            json!({
                "description": "When the engine read the page, in UTC (RFC 3339).",
                "type": "string",
                "format": "date-time",
            }),
        );
    }
    if let Some(required) = schema["required"].as_array_mut() {
        required.extend([json!("status"), json!(FETCHED_AT)]);
    }
    schema
}

/// The seeds a crawl admits: its own, `checked`, and for a code-free crawl
/// reading `extraction`, its [`sitemap_seeds`] after them.
pub(crate) fn admitted(
    extraction: Option<&CrawlExtraction>,
    mut checked: Vec<CheckedSeed>,
) -> Vec<CheckedSeed> {
    if let Some(extraction) = extraction {
        let added = sitemap_seeds(extraction, checked.iter().map(CheckedSeed::seed));
        checked.extend(added.into_iter().map(CheckedSeed::sitemap));
    }
    checked
}

/// The seeds a code-free crawl reading `extraction` admits beside its own
/// `seeds`: under `follow: sitemap`, one sitemap seed of each request
/// seed's site's robots.txt, each entry given the Label `page`; none
/// otherwise.
fn sitemap_seeds<'a>(
    extraction: &CrawlExtraction,
    seeds: impl IntoIterator<Item = &'a Seed>,
) -> Vec<SitemapSeed> {
    if extraction.follow != ExtractionFollow::Sitemap {
        return Vec::new();
    }
    let sites: BTreeSet<String> = seeds
        .into_iter()
        .filter_map(|seed| match seed {
            Seed::Request(request) => Url::parse(&request.url).ok(),
            Seed::Sitemap(_) | Seed::Syndication(_) => None,
        })
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|mut url| {
            url.set_query(None);
            url.set_fragment(None);
            url.set_path("/robots.txt");
            url.to_string()
        })
        .collect();
    sites
        .into_iter()
        .map(|robots_txt| SitemapSeed {
            sitemap: robots_txt,
            rules: vec![SitemapRule {
                pattern: EVERY_ENTRY.to_owned(),
                label: CODE_FREE_LABEL.to_owned(),
            }],
        })
        .collect()
}

/// The Record of `pages` for a page read as `formats`, its response's
/// `status`, read at `fetched_at`, with the `values` of its fields written
/// in the order `declared` gives (issue 2394). The page's formats, Markdown,
/// HTML and text included, are encoded straight into the Record's text,
/// which its Dataset's check and its commit then read.
pub(crate) fn record_of(
    formats: &PageFormats,
    status: Option<u16>,
    fetched_at: SystemTime,
    declared: &[String],
    values: &Map<String, Value>,
) -> RecordData {
    RecordData::of(&PageRecord {
        formats,
        status,
        fetched_at: goodfetch_engine::clock::rfc3339(fetched_at),
        fields: DeclaredValues { declared, values },
    })
    .expect("invariant: a page's Record encodes as JSON")
}

/// A page's Record as [`record_of`] writes it: its formats, its status,
/// when it was read, then its fields.
#[derive(Serialize)]
struct PageRecord<'a> {
    #[serde(flatten)]
    formats: &'a PageFormats,
    status: Option<u16>,
    // Named as `FETCHED_AT`, which the Dataset's schema requires.
    fetched_at: String,
    #[serde(flatten)]
    fields: DeclaredValues<'a>,
}

/// The values of a page's fields, written in the order they were declared.
struct DeclaredValues<'a> {
    declared: &'a [String],
    values: &'a Map<String, Value>,
}

impl Serialize for DeclaredValues<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(
            self.declared
                .iter()
                .filter_map(|name| Some((name, self.values.get(name)?))),
        )
    }
}

/// What the readers of a page's untouched tree read for it, once (ADR
/// 0138): the selectors whose links it follows, and its fields' locators,
/// in order, when every one is a value field.
pub(crate) type Demand = (Vec<String>, Vec<FieldLocator>);

/// What the readers of each page's untouched tree read for a code-free
/// crawl: what every Label's pages demand, and what each Label that a
/// declaration handles demands in its place.
#[derive(Default)]
pub(crate) struct Demands {
    pub(crate) run: Demand,
    pub(crate) labels: BTreeMap<String, Demand>,
}

impl CodeFree {
    /// What the crawl `run` started as `crawl` handles its requests with,
    /// on the live `crawls` and the pages of `tiers`, each field's `match`
    /// pattern and locale's forms learned here, once, unless the crawl
    /// keeps no Record; none for a crawl whose program handles them.
    pub(crate) fn of(
        crawl: Option<CodeFreeCrawl>,
        crawls: &Crawls,
        tiers: &FetchTiers,
        run: &RunIdentity,
    ) -> Option<Self> {
        let keeps_records = crate::serving::keeps_records(run);
        let work = run.metrics().counted_work();
        let handles = match crawl? {
            CodeFreeCrawl::Extraction(extraction) => {
                Handles::Extraction(Arc::new(ExtractionHandling {
                    reader: keeps_records.then(|| Fields::of(extraction.fields.by_name(), work)),
                    scope: (extraction.follow == ExtractionFollow::Scope).then(|| FollowRule {
                        links: LINKS.to_owned(),
                        label: CODE_FREE_LABEL.to_owned(),
                        include: Vec::new(),
                        exclude: Vec::new(),
                        limit: None,
                    }),
                    extraction,
                }))
            }
            CodeFreeCrawl::Declaration(declaration) => Handles::Declaration(Arc::new(
                declared::handlings(declaration, keeps_records, work),
            )),
        };
        Some(Self {
            handles,
            keeps_records,
            crawls: crawls.clone(),
            tiers: tiers.clone(),
        })
    }

    /// What the readers of each page's untouched tree read for it, once:
    /// for an extraction, under `follow: scope`, the links [`LINKS`]
    /// matches, and its fields' locators; for a declaration, each declared
    /// Label's own (ADR 0138).
    pub(crate) fn untouched(&self) -> Demands {
        match &self.handles {
            Handles::Extraction(handling) => Demands {
                run: (
                    handling
                        .scope
                        .iter()
                        .map(|rule| rule.links.clone())
                        .collect(),
                    locators(handling.extraction.fields.by_name()),
                ),
                labels: BTreeMap::new(),
            },
            Handles::Declaration(handlings) => Demands {
                run: Demand::default(),
                labels: handlings
                    .iter()
                    .map(|(label, handling)| (label.clone(), handling.demand()))
                    .collect(),
            },
        }
    }

    /// The formats it reads of each page, as `page.formats` answers them:
    /// none for a declaration.
    pub(crate) fn formats(&self) -> FormatsRequest {
        match &self.handles {
            Handles::Extraction(handling) => handling.extraction.formats,
            Handles::Declaration(_) => FormatsRequest::default(),
        }
    }

    /// Whether it reads any format of its pages.
    pub(crate) fn reads_formats(&self) -> bool {
        self.formats() != FormatsRequest::default()
    }

    /// Handles the attempt `params` gives, as a handler would, and answers
    /// its page's Records, read at `fetched_at`, and what their fields'
    /// Provenance names: for an extraction, reads its page's formats and
    /// fields, under `follow: scope` stages every link of its page with
    /// the attempt, for its commit to admit in its outcome's one job (ADR
    /// 0138), and answers one Record of `pages`; for a declaration, as it
    /// handles its request's Label.
    ///
    /// # Errors
    ///
    /// `closed` when the attempt has no page, and those of `page.formats`,
    /// of [`FetchTiers::field_elements`] and of [`defer_links`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it read; what it staged is the
    /// attempt's, committed or discarded with it.
    pub(crate) async fn handled(
        &self,
        params: &CrawlRequestHandleParams,
        fetched_at: SystemTime,
    ) -> Result<(CrawlRequestHandled, FieldsRead), CallError> {
        let Some(page_id) = &params.page_id else {
            return Err(CallError::new(
                &CLOSED,
                format!(
                    "Crawl request `{}` was given no page, so a code-free crawl has nothing to read.",
                    params.crawl_request.id
                ),
                &(),
            ));
        };
        match &self.handles {
            Handles::Extraction(handling) => {
                self.extracted(handling, params, page_id, fetched_at).await
            }
            Handles::Declaration(handlings) => match handlings.get(&params.crawl_request.label) {
                Some(handling) => self.as_declared(handling, params, page_id).await,
                None => Ok(nothing()),
            },
        }
    }

    /// The attempt `params` of the page `page_id`, handled as `handling`
    /// says.
    async fn extracted(
        &self,
        handling: &ExtractionHandling,
        params: &CrawlRequestHandleParams,
        page_id: &PageId,
        fetched_at: SystemTime,
    ) -> Result<(CrawlRequestHandled, FieldsRead), CallError> {
        let extraction = &handling.extraction;
        let request_id = &params.crawl_request.id;
        let read = if self.keeps_records {
            let formats = self
                .tiers
                .formats(page_id, extraction.formats)
                .await
                .ok_or_else(|| {
                    CallError::new(
                        &CLOSED,
                        format!(
                            "No tier serves the page of crawl request `{request_id}`, so its formats were not read."
                        ),
                        &(),
                    )
                })??;
            let (values, read) = self
                .fields(
                    page_id,
                    request_id,
                    extraction.fields.by_name(),
                    handling.reader.as_ref(),
                )
                .await?;
            Some((formats, values, read))
        } else {
            None
        };
        if let Some(scope) = &handling.scope {
            self.follow(params, page_id, scope).await?;
        }
        let status = params.response.as_ref().map(|response| response.status);
        let Some((formats, values, read)) = read else {
            return Ok(nothing());
        };
        Ok((
            CrawlRequestHandled {
                records: vec![PushedRecord {
                    dataset: CODE_FREE_DATASET.to_owned(),
                    data: record_of(
                        &formats,
                        status,
                        fetched_at,
                        extraction.fields.declared(),
                        &values,
                    ),
                }],
                skipped: None,
            },
            read,
        ))
    }

    /// The attempt `params` of the page `page_id`, handled as `handling`,
    /// its Label's, declares: its fields read once, each of its follow
    /// rules' links staged with the attempt, then one Record per extract,
    /// unless the crawl keeps no Record.
    async fn as_declared(
        &self,
        handling: &declared::LabelHandling,
        params: &CrawlRequestHandleParams,
        page_id: &PageId,
    ) -> Result<(CrawlRequestHandled, FieldsRead), CallError> {
        let read = if self.keeps_records {
            Some(
                self.fields(
                    page_id,
                    &params.crawl_request.id,
                    handling.fields.by_name(),
                    handling.reader.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };
        for rule in &handling.follow {
            self.follow(params, page_id, rule).await?;
        }
        let Some((values, read)) = read else {
            return Ok(nothing());
        };
        let records = handling
            .extracts
            .iter()
            .map(|(dataset, declared)| PushedRecord {
                dataset: dataset.clone(),
                data: RecordData::of(&DeclaredValues {
                    declared,
                    values: &values,
                })
                .expect("invariant: a page's fields encode as JSON"),
            })
            .collect();
        Ok((
            CrawlRequestHandled {
                records,
                skipped: None,
            },
            read,
        ))
    }

    /// Stages with the attempt `params` the links of its page `page_id`
    /// that `rule` follows, as `enqueueLinks` admits them.
    async fn follow(
        &self,
        params: &CrawlRequestHandleParams,
        page_id: &PageId,
        rule: &FollowRule,
    ) -> Result<(), CallError> {
        defer_links(
            &CrawlRequestEnqueueLinksParams {
                run_id: params.run_id.clone(),
                crawl_request_id: params.crawl_request.id.clone(),
                lease_epoch: params.lease_epoch,
                locator: Locator {
                    page_id: page_id.clone(),
                    selector: rule.links.clone(),
                },
                label: rule.label.clone(),
                payload: None,
                payloads: None,
                include: rule.include.clone(),
                exclude: rule.exclude.clone(),
                limit: rule.limit,
                priority: 0,
            },
            &self.crawls,
            &self.tiers,
        )
        .await
    }

    /// The values of the fields `declared` on the page `page_id` of the
    /// crawl request `request_id`, by name, read with `reader`, and what
    /// their Provenance names; none when there is no field or no reader.
    async fn fields(
        &self,
        page_id: &PageId,
        request_id: &str,
        declared: &BTreeMap<String, PageField>,
        reader: Option<&Fields>,
    ) -> Result<(Map<String, Value>, FieldsRead), CallError> {
        let Some(reader) = reader.filter(|_| !declared.is_empty()) else {
            return Ok((Map::new(), FieldsRead::default()));
        };
        let found = self
            .tiers
            .field_elements(page_id, declared)
            .await
            .ok_or_else(|| {
                CallError::new(
                    &CLOSED,
                    format!(
                        "No tier serves the page of crawl request `{request_id}`, so its fields were not read."
                    ),
                    &(),
                )
            })??;
        let read = reader.read(found.found, &found.base_url);
        Ok((
            read.values,
            FieldsRead {
                value_transforms: read.transforms,
                unread: read.unread,
            },
        ))
    }
}

/// An attempt's answer pushing no Record and reading no field.
fn nothing() -> (CrawlRequestHandled, FieldsRead) {
    (
        CrawlRequestHandled {
            records: Vec::new(),
            skipped: None,
        },
        FieldsRead::default(),
    )
}

/// The locators of `fields`, in order, when every one is a value field;
/// none otherwise.
fn locators(fields: &BTreeMap<String, PageField>) -> Vec<FieldLocator> {
    goodfetch_fetch_tier::value_locators(fields).unwrap_or_default()
}

#[cfg(test)]
mod tests;
