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
//! Its named `fields` are read from the first element each one's locator
//! finds and given by its Value transform (ADR 0123, the `fields`
//! module): the Record holds each, null where the page did not give it,
//! and its Provenance names each transform and why each null field is
//! null.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_frontier::Url;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::{
    CODE_FREE_DATASET, CODE_FREE_LABEL, CrawlExtraction, CrawlRequestHandleParams,
    CrawlRequestHandled, CrawlStartParams, ExtractionFollow, FieldLocator, LabelDeclaration,
    PushedRecord, Seed, SitemapRule, SitemapSeed,
};
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::frontier::CrawlRequestEnqueueLinksParams;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{FormatsRequest, Locator, PageFormats, PageId, page_formats_schema};
use goodfetch_protocol::run::PointInTime;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::enqueue::links::defer_links;
use crate::serving::Crawls;
use crate::work_items::FieldsRead;

/// The field of a Record that holds when its page was read: each run of
/// an attempt reads at its own time, so a sample's comparison leaves it
/// out (ADR 0020).
pub(crate) const FETCHED_AT: &str = "fetched_at";

/// The locator whose links `follow: scope` admits from each page.
const LINKS: &str = "a[href]";

/// The key of the Dataset `pages`.
const KEY: &str = "url";

/// The pattern of a sitemap seed's rule that every entry matches.
const EVERY_ENTRY: &str = "**";

/// What a code-free crawl's attempts handle their requests with: its
/// extraction, what each of its fields reads with, built once as the crawl
/// starts, and the live crawls and tiers whose pages they read.
#[derive(Clone)]
pub(crate) struct CodeFree {
    pub(crate) extraction: CrawlExtraction,
    /// Each field's reader, in the order of `extraction.fields`; none when
    /// the crawl keeps no Record.
    readers: Arc<[fields::Reader]>,
    /// Whether the crawl keeps Records, its `dataset.records`: off, a page's
    /// formats and fields are not read for one (ADR 0138).
    keeps_records: bool,
    pub(crate) crawls: Crawls,
    pub(crate) tiers: FetchTiers,
}

/// The table that keeps a code-free crawl's extraction in every run file;
/// a crawl whose program handles its requests keeps no row.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-code-free",
    statements: &["CREATE TABLE IF NOT EXISTS code_free_extraction (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            extraction TEXT NOT NULL
        ) STRICT"],
};

/// Records `extraction`, the code-free crawl's start's; nothing for a
/// crawl whose program handles its requests.
///
/// # Errors
///
/// The run file's, when the row cannot be written.
pub(crate) fn record(
    connection: &Connection,
    extraction: Option<&CrawlExtraction>,
) -> rusqlite::Result<()> {
    let Some(extraction) = extraction else {
        return Ok(());
    };
    let extraction =
        serde_json::to_string(extraction).expect("invariant: an extraction encodes as JSON");
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO code_free_extraction (id, extraction) VALUES (1, ?1)",
        )
        .and_then(|mut cached| cached.execute(params![extraction]))?;
    Ok(())
}

/// Records `extraction` again, a start's under the ID of the Run that
/// resumes, in place of the one the run file holds: none takes it out, for
/// a crawl whose program handles its requests from then on.
///
/// # Errors
///
/// The run file's, when the row cannot be written.
pub(crate) fn record_again(
    connection: &Connection,
    extraction: Option<&CrawlExtraction>,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached("DELETE FROM code_free_extraction")
        .and_then(|mut cached| cached.execute([]))?;
    record(connection, extraction)
}

/// The extraction [`record`] kept; none for a crawl whose program handles
/// its requests, or one whose row no longer reads.
///
/// # Errors
///
/// The run file's, when the row cannot be read.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Option<CrawlExtraction>> {
    let kept: Option<String> = connection
        .prepare_cached("SELECT extraction FROM code_free_extraction WHERE id = 1")
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))
        .optional()?;
    Ok(kept.and_then(|kept| match serde_json::from_str(&kept) {
        Ok(extraction) => Some(extraction),
        Err(error) => {
            tracing::warn!(%error, "the code-free crawl's extraction in the run file no longer reads");
            None
        }
    }))
}

/// Declares in `params` what a code-free crawl commits into, when it
/// takes an extraction: the Label `page`, beside any the start declares,
/// and the Dataset `pages`, the start's only one; `invalid_params` when the
/// start declares a Dataset of its own.
pub(crate) fn declare(params: &mut CrawlStartParams) -> Result<(), CallError> {
    let Some(extraction) = &params.extraction else {
        return Ok(());
    };
    declare_into(extraction, &mut params.labels, &mut params.datasets)
}

fn declare_into(
    extraction: &CrawlExtraction,
    labels: &mut BTreeMap<String, LabelDeclaration>,
    datasets: &mut Vec<DatasetDeclaration>,
) -> Result<(), CallError> {
    if let Some(declared) = datasets.first() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "A crawl with `extraction` declares its own Dataset, `{CODE_FREE_DATASET}`, so it takes none of the program's, such as `{}`: leave `datasets` out, or handle the requests with a handler.",
                declared.name
            ),
        ));
    }
    let every_record = page_schema(&extraction.formats);
    for (name, field) in &extraction.fields {
        let refused = if every_record["properties"].get(name).is_some() {
            Err(format!(
                "The field `{name}` takes the name of a property every Record of `{CODE_FREE_DATASET}` holds: name it otherwise."
            ))
        } else {
            fields::check(name, field)
        };
        refused.map_err(|message| CallError::protocol(ProtocolFailure::InvalidParams, message))?;
    }
    labels.entry(CODE_FREE_LABEL.to_owned()).or_default();
    datasets.push(dataset(extraction));
    Ok(())
}

/// The Dataset `pages` of a crawl reading `extraction`: keyed by `url`,
/// its schema [`page_schema`]'s with each of its fields, its transform's
/// type or null, required beside them.
pub(crate) fn dataset(extraction: &CrawlExtraction) -> DatasetDeclaration {
    let mut schema = page_schema(&extraction.formats);
    if let Some(properties) = schema["properties"].as_object_mut() {
        for (name, field) in &extraction.fields {
            properties.insert(name.clone(), fields::schema(field));
        }
    }
    if let Some(required) = schema["required"].as_array_mut() {
        required.extend(extraction.fields.keys().cloned().map(Value::String));
    }
    DatasetDeclaration {
        name: CODE_FREE_DATASET.to_owned(),
        schema: Some(schema),
        key: vec![KEY.to_owned()],
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

/// The seeds a crawl admits: its own `seeds`, and for a code-free crawl
/// reading `extraction`, its [`sitemap_seeds`] after them.
pub(crate) fn admitted(extraction: Option<&CrawlExtraction>, seeds: &[Seed]) -> Vec<Seed> {
    let mut admitted = seeds.to_vec();
    if let Some(extraction) = extraction {
        admitted.extend(sitemap_seeds(extraction, seeds));
    }
    admitted
}

/// The seeds a code-free crawl reading `extraction` admits beside its own
/// `seeds`: under `follow: sitemap`, one sitemap seed of each request
/// seed's site's robots.txt, each entry given the Label `page`; none
/// otherwise.
fn sitemap_seeds(extraction: &CrawlExtraction, seeds: &[Seed]) -> Vec<Seed> {
    if extraction.follow != ExtractionFollow::Sitemap {
        return Vec::new();
    }
    let sites: BTreeSet<String> = seeds
        .iter()
        .filter_map(|seed| match seed {
            Seed::Request(request) => Url::parse(&request.url).ok(),
            Seed::Sitemap(_) => None,
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
        .map(|robots_txt| {
            Seed::Sitemap(SitemapSeed {
                sitemap: robots_txt,
                rules: vec![SitemapRule {
                    pattern: EVERY_ENTRY.to_owned(),
                    label: CODE_FREE_LABEL.to_owned(),
                }],
            })
        })
        .collect()
}

/// The Record of `pages` for a page read as `formats`, its response's
/// `status`, read at `fetched_at`, with its `fields`' values.
pub(crate) fn record_of(
    formats: &PageFormats,
    status: Option<u16>,
    fetched_at: &PointInTime,
    fields: Map<String, Value>,
) -> Value {
    let mut record = serde_json::to_value(formats).expect("invariant: formats encode as JSON");
    if let Some(properties) = record.as_object_mut() {
        properties.insert("status".to_owned(), json!(status));
        properties.insert(FETCHED_AT.to_owned(), json!(fetched_at));
        properties.extend(fields);
    }
    record
}

impl CodeFree {
    /// What the crawl `run` reading `extraction` handles its requests with,
    /// on the live `crawls` and the pages of `tiers`, each field's `match`
    /// pattern and locale's forms learned here, once, unless the crawl
    /// keeps no Record; none for a crawl whose program handles them.
    pub(crate) fn of(
        extraction: Option<CrawlExtraction>,
        crawls: &Crawls,
        tiers: &FetchTiers,
        run: &Run,
    ) -> Option<Self> {
        let keeps_records = crate::serving::keeps_records(run);
        let work = run.metrics().counted_work();
        extraction.map(|extraction| Self {
            readers: if keeps_records {
                extraction
                    .fields
                    .values()
                    .map(|field| fields::Reader::of(field, work))
                    .collect()
            } else {
                Arc::from([])
            },
            keeps_records,
            extraction,
            crawls: crawls.clone(),
            tiers: tiers.clone(),
        })
    }

    /// What the readers of each page's untouched tree read for it, once:
    /// under `follow: scope`, the links [`LINKS`] matches, and its fields'
    /// locators, in order (ADR 0138).
    pub(crate) fn untouched(&self) -> (Vec<String>, Vec<FieldLocator>) {
        let links = if self.extraction.follow == ExtractionFollow::Scope {
            vec![LINKS.to_owned()]
        } else {
            Vec::new()
        };
        let locators = self
            .extraction
            .fields
            .values()
            .map(|field| field.locator.clone())
            .collect();
        (links, locators)
    }

    /// Handles the attempt `params` gives, as a handler would: reads its
    /// page's formats and fields, under `follow: scope` stages every link
    /// of its page with the attempt, for its commit to admit in its
    /// outcome's one job (ADR 0138), then answers its page's
    /// Record, read at `fetched_at`, and what its fields' Provenance names.
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
        fetched_at: PointInTime,
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
        let read = if self.keeps_records {
            Some(self.read(page_id, &params.crawl_request.id).await?)
        } else {
            None
        };
        if self.extraction.follow == ExtractionFollow::Scope {
            defer_links(
                &CrawlRequestEnqueueLinksParams {
                    run_id: params.run_id.clone(),
                    crawl_request_id: params.crawl_request.id.clone(),
                    lease_epoch: params.lease_epoch,
                    locator: Locator {
                        page_id: page_id.clone(),
                        selector: LINKS.to_owned(),
                    },
                    label: CODE_FREE_LABEL.to_owned(),
                    payload: None,
                    payloads: None,
                    include: Vec::new(),
                    exclude: Vec::new(),
                    limit: None,
                    priority: 0,
                },
                &self.crawls,
                &self.tiers,
            )
            .await?;
        }
        let status = params.response.as_ref().map(|response| response.status);
        let Some((formats, values, read)) = read else {
            return Ok((
                CrawlRequestHandled {
                    records: Vec::new(),
                    skipped: None,
                },
                FieldsRead::default(),
            ));
        };
        Ok((
            CrawlRequestHandled {
                records: vec![PushedRecord {
                    dataset: CODE_FREE_DATASET.to_owned(),
                    data: record_of(&formats, status, &fetched_at, values),
                }],
                skipped: None,
            },
            read,
        ))
    }

    /// The formats and fields of the page `page_id` of the crawl request
    /// `request_id` that its Record holds, and what its fields' Provenance
    /// names.
    async fn read(
        &self,
        page_id: &PageId,
        request_id: &str,
    ) -> Result<(PageFormats, Map<String, Value>, FieldsRead), CallError> {
        let formats = self
            .tiers
            .formats(page_id, self.extraction.formats)
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
        let (values, read) = self.fields(page_id, request_id).await?;
        Ok((formats, values, read))
    }

    /// The values of the extraction's fields on the page `page_id` of the
    /// crawl request `request_id`, by name, and what their Provenance
    /// names; none for an extraction with no field.
    async fn fields(
        &self,
        page_id: &PageId,
        request_id: &str,
    ) -> Result<(Map<String, Value>, FieldsRead), CallError> {
        let declared = &self.extraction.fields;
        if declared.is_empty() {
            return Ok((Map::new(), FieldsRead::default()));
        }
        let locators: Vec<FieldLocator> = declared
            .values()
            .map(|field| field.locator.clone())
            .collect();
        let found = self
            .tiers
            .field_elements(page_id, &locators)
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
        let mut values = Map::new();
        let mut read = FieldsRead::default();
        for (((name, field), reader), element) in
            declared.iter().zip(self.readers.iter()).zip(found.elements)
        {
            let one = fields::read(name, (field, reader), element.as_ref(), &found.base_url);
            values.insert(name.clone(), one.value);
            read.value_transforms.extend(one.transforms);
            read.unread.extend(one.unread);
        }
        Ok((values, read))
    }
}

mod fields;

#[cfg(test)]
mod tests;
