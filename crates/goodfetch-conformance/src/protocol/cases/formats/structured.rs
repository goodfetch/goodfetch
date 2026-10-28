//! A page's metadata and contacts (spec #580, ticket #787): on the
//! `extraction` family's structured pages, the title, description,
//! canonical URL, language, Open Graph and Twitter tags, JSON-LD blocks and
//! microdata items, and the e-mail addresses, telephone numbers and social
//! profiles, each judged by the oracle; a number with no country code on a
//! page whose language tag has no region gets `e164: null`, from a `tel:`
//! link or from text, and one on a page whose tag has a region gets it
//! (G1).

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::{PAGE_FORMATS, PageFormats, PageId, Phone};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::json;

use super::super::{Checks, Site, finish_run, new_page, start_run};
use super::{formats, goto, origin_of, task_pages};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::Submission;

/// The task whose pages and oracle judge the metadata and contacts.
const TASK: &str = "extraction/10";

/// One metadata and contacts case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    MetadataAndContactsMatchTheOracle,
    NoCountryIsGuessed,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(in crate::protocol) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "json-ld-open-graph-twitter-microdata-and-contacts-match-the-oracle" => {
                Self::MetadataAndContactsMatchTheOracle
            }
            "a-number-with-no-country-code-and-no-region-gets-no-e164-and-one-with-a-region-gets-it" => {
                Self::NoCountryIsGuessed
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(in crate::protocol) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::MetadataAndContactsMatchTheOracle => {
                metadata_and_contacts_match_the_oracle(connection, site, checks)
            }
            Self::NoCountryIsGuessed => no_country_is_guessed(connection, site, checks),
        }
    }
}

/// The metadata and contacts `page.formats` answers on `page`.
fn read(connection: &mut Connection, page: &PageId) -> Result<Option<PageFormats>, Gone> {
    Ok(formats(
        connection,
        page,
        &json!({ "metadata": true, "contacts": true }),
    )?
    .and_then(|result| serde_json::from_value::<PageFormats>(result).ok()))
}

/// On each structured page, the metadata and contacts `page.formats`
/// gives are what the oracle holds for them.
fn metadata_and_contacts_match_the_oracle(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let run_id = "conformance-formats-structured";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let mut records = Vec::new();
        for path in &paths {
            goto(
                connection,
                &page.page_id,
                &format!("{origin}{path}"),
                checks,
            )?;
            let read = read(connection, &page.page_id)?;
            checks.check(
                format!("`{}` of {path} answers its result", PAGE_FORMATS.name),
                read.is_some(),
            );
            let Some(read) = read else { continue };
            records.push(json!({
                "id": path,
                "metadata": read.metadata,
                "contacts": read.contacts,
            }));
        }
        let task =
            TaskId::try_from(TASK.to_owned()).expect("invariant: the structured task's ID is one");
        let answer = site.proving_ground.judge(&Submission {
            task: &task,
            status: "success",
            records: &records,
        });
        checks.check(
            format!(
                "the oracle verifies the metadata and contacts of every page ({:?})",
                answer.as_ref().map(|answer| &answer.outcome)
            ),
            answer
                .as_ref()
                .is_ok_and(|answer| answer.outcome == OracleOutcome::Verified),
        );
        if let Ok(answer) = answer {
            checks.oracle_answers.push(answer);
        }
    }
    finish_run(connection, run_id)
}

/// The telephone numbers `page.formats` reads at `url`.
fn phones(
    connection: &mut Connection,
    page: &PageId,
    url: &str,
    checks: &mut Checks,
) -> Result<Vec<Phone>, Gone> {
    goto(connection, page, url, checks)?;
    Ok(read(connection, page)?
        .and_then(|read| read.contacts)
        .map(|contacts| contacts.phones)
        .unwrap_or_default())
}

/// On `/structured/no-region` (`lang="en"`), the `tel:` link and the
/// national number in text, without their country code, have
/// `e164: null`; on `/structured/region` (`lang="fr-FR"`), the national number
/// in text gets its E.164.
fn no_country_is_guessed(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, _)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let run_id = "conformance-formats-no-country-guessed";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let no_region = phones(
            connection,
            &page.page_id,
            &format!("{origin}/structured/no-region"),
            checks,
        )?;
        checks.check(
            format!(
                "with no region in the language tag, the `tel:` link and the national number in text, without their country code, have `e164: null` ({no_region:?})"
            ),
            no_region
                .iter()
                .any(|phone| phone.value == "02070313000" && phone.e164.is_none())
                && no_region
                    .iter()
                    .any(|phone| phone.value == "020 7031 3001" && phone.e164.is_none()),
        );
        let region = phones(
            connection,
            &page.page_id,
            &format!("{origin}/structured/region"),
            checks,
        )?;
        checks.check(
            format!(
                "with the region `FR` in the language tag, the national number gets `+33142685300` ({region:?})"
            ),
            region.iter().any(|phone| {
                phone.value == "01 42 68 53 00" && phone.e164.as_deref() == Some("+33142685300")
            }),
        );
    }
    finish_run(connection, run_id)
}
