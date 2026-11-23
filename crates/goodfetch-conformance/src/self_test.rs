//! The self-test suite (spec #152): it proves the Proving ground's own
//! oracle against known-right and known-wrong results. The known-right
//! ones are reference results read from the pages the way any tool would,
//! never copied from the oracle's expectations; the known-wrong ones are
//! made from them as each case of `conformance/self-test/cases.toml`
//! says. Its target is the Proving ground itself.
//!
//! A reference result is an input, so a wrong one fails the suite and the
//! document names the case and the oracle's answer; a task with no
//! reference result leaves its cases unreached and their capabilities
//! `unproved`.

use std::collections::BTreeMap;

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::journey::{FeatureId, TaskId};
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, OracleOutcome, Suite, Target,
    TargetIdentity,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::case_list::{CaseList, DeclaredCapability, DeclaredCase};
use crate::host::{RUNTIME, platform};
use crate::proving_ground::{ProvingGround, ProvingGroundError, Submission, TaskList};
use crate::writer::{WriteError, write_document};

/// The suite's case list, embedded so the suite runs wherever the harness
/// does.
const CASES: &str = include_str!("../../../conformance/self-test/cases.toml");

/// The page family whose tasks the suite judges.
const CATALOG: &str = "catalog";

/// The target's name in the document.
const TARGET: &str = "goodfetch-proving-ground";

/// The price the `wrong-price` case submits, or the other one when the
/// page shows it.
const WRONG_PRICES: [&str; 2] = ["0.01", "0.02"];

/// The reference result of each task: the Records its pages show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    /// Each task's Records; a task absent here is not reached.
    pub records: BTreeMap<TaskId, Vec<Record>>,
}

/// A Record of the `catalog` family: one product.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Its ID, such as `P-0042`.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its price, as the page shows it, with two decimals.
    pub price: String,
}

impl References {
    /// Reads the reference result of every `crawl` and `extract` task of the
    /// `catalog` family, the one the suite's cases judge, from the pages: a
    /// crawl from its start page through each listing's next link, an
    /// extract of its one page. A task of another kind or family gets none.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot serve the task list or a page, or a
    /// page lacks what its family shows.
    pub fn read(proving_ground: &ProvingGround) -> Result<Self, SelfTestError> {
        let reader = Reader::new();
        let mut records = BTreeMap::new();
        for task in proving_ground.task_list()?.tasks {
            // The reader knows only the `catalog` family's pages; another
            // family's pages show their Records in their own shape.
            if task.family != CATALOG {
                continue;
            }
            let read = match task.kind.as_str() {
                "crawl" => reader.crawl(proving_ground, &task.start_url)?,
                "extract" => vec![reader.product(proving_ground, &task.start_url)?],
                // No case of the suite reads another kind yet, such as a
                // `submit` task's Effects: its task gets no reference result.
                _ => continue,
            };
            records.insert(task.id, read);
        }
        Ok(Self { records })
    }
}

/// Runs the self-test suite: each declared case's result goes to the
/// oracle, and the case log of its answers becomes the document.
///
/// # Errors
///
/// When the Proving ground cannot serve the task list or answer a
/// submission.
pub fn run_self_test(
    proving_ground: &ProvingGround,
    references: &References,
) -> Result<ConformanceDocument, SelfTestError> {
    let cases: SelfTestCases =
        toml::from_str(CASES).expect("invariant: the embedded case list is valid");
    let task_list = proving_ground.task_list()?;
    let mut log = Vec::new();
    for capability in &cases.capabilities {
        for case in &capability.cases {
            let Some(reference) = references.records.get(&case.task) else {
                continue;
            };
            let records = case.submit.apply(reference);
            let answer = proving_ground.judge(&Submission {
                task: &case.task,
                status: &case.status,
                records: &records,
            })?;
            log.push(CaseLogEntry {
                case: case.id.clone(),
                capability: capability.name.clone(),
                mode: None,
                blocking: None,
                platform: platform(),
                runtime: RUNTIME.to_owned(),
                checks: vec![
                    CaseCheck {
                        name: format!("the oracle answers `{}`", case.expect.as_str()),
                        passed: answer.outcome == case.expect,
                    },
                    CaseCheck {
                        name: "the oracle names the task it judged".to_owned(),
                        passed: answer.task == case.task,
                    },
                ],
                evidence: Evidence {
                    oracle_answers: vec![answer],
                    ..Evidence::default()
                },
            });
        }
    }
    let header = header(&cases.suite, task_list, proving_ground.is_online());
    Ok(write_document(header, &cases.case_list(), &log)?)
}

fn header(suite: &Suite, task_list: TaskList, online: bool) -> ConformanceHeader {
    ConformanceHeader {
        suite: suite.clone(),
        target: Target {
            name: TARGET.to_owned(),
            identity: TargetIdentity::Version(task_list.proving_ground.version.clone()),
            product: None,
        },
        platform: platform(),
        runtime: RUNTIME.to_owned(),
        proving_ground: task_list.proving_ground,
        online,
        gate: false,
    }
}

/// Why the self-test could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SelfTestError {
    /// The Proving ground gave no usable answer.
    #[error(transparent)]
    ProvingGround(#[from] ProvingGroundError),
    /// A page lacks what its family shows.
    #[error("the page {url} shows no {what}")]
    Page {
        /// The page.
        url: String,
        /// What it lacks.
        what: &'static str,
    },
    /// The case log did not become a document.
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// `conformance/self-test/cases.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelfTestCases {
    suite: Suite,
    #[serde(rename = "capability")]
    capabilities: Vec<SelfTestCapability>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelfTestCapability {
    name: String,
    #[serde(rename = "case")]
    cases: Vec<SelfTestCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelfTestCase {
    id: String,
    covers: Vec<FeatureId>,
    task: TaskId,
    submit: Submit,
    status: String,
    expect: OracleOutcome,
}

impl SelfTestCases {
    fn case_list(&self) -> CaseList {
        CaseList {
            suite: self.suite.clone(),
            capabilities: self
                .capabilities
                .iter()
                .map(|capability| DeclaredCapability {
                    name: capability.name.clone(),
                    cases: capability
                        .cases
                        .iter()
                        .map(|case| DeclaredCase {
                            id: case.id.clone(),
                            covers: case.covers.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// How a case makes its result from the reference result.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Submit {
    Reference,
    OneRecordMissing,
    WrongPrice,
    ExtraRecord,
    RecordTwice,
}

impl Submit {
    fn apply(self, reference: &[Record]) -> Vec<Record> {
        let mut records = reference.to_vec();
        match self {
            Self::Reference => {}
            Self::OneRecordMissing => {
                records.pop();
            }
            Self::WrongPrice => {
                if let Some(record) = records.first_mut() {
                    let wrong = WRONG_PRICES
                        .iter()
                        .find(|price| **price != record.price)
                        .expect("invariant: two prices differ from any one");
                    (*wrong).clone_into(&mut record.price);
                }
            }
            Self::ExtraRecord => records.push(Record {
                id: "P-9999".to_owned(),
                name: "Unlisted".to_owned(),
                price: "1.00".to_owned(),
            }),
            Self::RecordTwice => {
                if let Some(record) = records.first().cloned() {
                    records.push(record);
                }
            }
        }
        records
    }
}

/// Reads the `catalog` family's pages as any tool would.
struct Reader {
    id: Regex,
    name: Regex,
    price: Regex,
    product_link: Regex,
    next_link: Regex,
}

impl Reader {
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    fn new() -> Self {
        let pattern = |pattern: &str| Regex::new(pattern).expect("invariant: a valid pattern");
        Self {
            id: pattern(r#"<article class="product" data-id="([^"]+)""#),
            name: pattern(r#"<h1 class="name">([^<]+)</h1>"#),
            price: pattern(r#"data-price="([^"]+)""#),
            product_link: pattern(r#"href="(/products/[^"]+)""#),
            next_link: pattern(r#"rel="next" href="([^"]+)""#),
        }
    }

    /// The Record a product page shows.
    fn product(&self, proving_ground: &ProvingGround, url: &str) -> Result<Record, SelfTestError> {
        let page = proving_ground.page(url)?;
        let field = |pattern: &Regex, what: &'static str| {
            pattern
                .captures(&page)
                .map(|found| found[1].to_owned())
                .ok_or_else(|| SelfTestError::Page {
                    url: url.to_owned(),
                    what,
                })
        };
        Ok(Record {
            id: field(&self.id, "product ID")?,
            name: field(&self.name, "product name")?,
            price: field(&self.price, "price")?,
        })
    }

    /// The Records of every product page the listing pages link to, from
    /// the start page through each next link.
    fn crawl(
        &self,
        proving_ground: &ProvingGround,
        start_url: &str,
    ) -> Result<Vec<Record>, SelfTestError> {
        let origin = origin(start_url).ok_or_else(|| SelfTestError::Page {
            url: start_url.to_owned(),
            what: "origin",
        })?;
        let mut records = Vec::new();
        let mut listing = Some(start_url.to_owned());
        while let Some(url) = listing {
            let page = proving_ground.page(&url)?;
            for link in self.product_link.captures_iter(&page) {
                records.push(self.product(proving_ground, &format!("{origin}{}", &link[1]))?);
            }
            listing = self
                .next_link
                .captures(&page)
                .map(|link| format!("{origin}{}", &link[1]));
        }
        Ok(records)
    }
}

/// `http://host:port` of an `http` URL.
fn origin(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("http://")?;
    Some(&url[..url.len() - rest.len() + rest.find('/').unwrap_or(rest.len())])
}
