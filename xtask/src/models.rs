//! The model manifest (ADR 0065): `test-support/fixtures/models.toml` pins
//! one small and one larger model for each format of ADR 0012, by dated or
//! versioned Model reference, from the Vercel AI Gateway's free tier (ADR
//! 0076), each with the retirement date its provider announced and the page
//! that announced it. A format the gateway does not serve says why it waits.
//! `xtask models check`, run with the daily test-health job, opens or
//! updates one `needs-triage` issue for each pinned model from 30 days
//! before its retirement, so its Model recordings are refreshed in time; a
//! new one goes to the spec whose ticket pinned the model (ADR 0136).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use jiff::civil::Date;
use serde::Deserialize;
use toml::value::Datetime;

use crate::environment::Environment;
use crate::github;
use crate::issues::{Issues, Label};
use crate::output;
use crate::owners::{Added, Owners};

/// The manifest, from the repository's root.
pub(crate) const MANIFEST: &str = "test-support/fixtures/models.toml";

/// How many days before a retirement its issue opens.
const NOTICE_DAYS: i64 = 30;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Opens or updates one `needs-triage` issue for each model the manifest
    /// pins that retires within 30 days, or has retired; fails on a manifest
    /// outside its format.
    Check(Arguments),
}

/// `xtask models check`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Today's date in UTC, `YYYY-MM-DD`; the system's when left out.
    #[arg(long)]
    today: Option<Date>,
    /// The repository whose issues the check opens, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let Command::Check(arguments) = command;
    let root = &environment.root;
    let manifest = Manifest::read(&root.join(MANIFEST))?;
    let today = arguments.today.unwrap_or_else(today);
    let due = manifest.due(today);
    if due.is_empty() {
        output::line(&format!(
            "No pinned model retires within {NOTICE_DAYS} days of {today}."
        ));
        return Ok(ExitCode::SUCCESS);
    }
    let api = arguments.github.api(environment.token());
    let repository = &arguments.github.repository;
    let mut issues = Issues::new(&api, repository);
    let owners = Owners::new(&api, repository);
    // A closed issue reopens on the next day's run while the model is
    // pinned: only a new manifest ends it.
    let latest = today
        .to_zoned(jiff::tz::TimeZone::UTC)
        .context("today's start in UTC")?
        .timestamp();
    for model in &due {
        let reference = model.reference;
        issues.keep(
            Label::NeedsTriage,
            &format!("model-retirement {reference}"),
            &format!("Model retirement: {reference} retires on {}", model.retires),
            &body(model, today),
            latest,
            || {
                owners.of_added(
                    root,
                    &Added {
                        text: format!("\"{reference}\""),
                        files: Some(MANIFEST.to_owned()),
                    },
                )
            },
        )?;
    }
    for action in issues.actions() {
        output::line(action);
    }
    Ok(ExitCode::SUCCESS)
}

/// Today's date in UTC.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0059: a retirement is counted to the wall clock's today; tests pass --today"
)]
fn today() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

/// The formats of ADR 0012, in the manifest's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    OpenAiCompatible,
    AnthropicMessages,
    Gemini,
}

impl Format {
    /// Its table in the manifest.
    fn key(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::AnthropicMessages => "anthropic-messages",
            Self::Gemini => "gemini",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI-compatible",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::Gemini => "Gemini",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    Small,
    Larger,
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Larger => "larger",
        }
    }
}

/// A model the manifest pins.
#[derive(Debug, PartialEq, Eq)]
struct Pinned {
    /// Its Model reference, `provider/model`, dated or versioned.
    reference: String,
    /// The day its provider retires it, when one is announced.
    retires: Option<Date>,
    /// The page that announces its retirement, or would.
    source: String,
}

/// What the manifest says of a format.
#[derive(Debug, PartialEq, Eq)]
enum Pins {
    Models {
        small: Pinned,
        larger: Pinned,
    },
    /// No model is pinned, for this reason.
    Waits(String),
}

#[derive(Debug, PartialEq, Eq)]
struct Manifest {
    formats: Vec<(Format, Pins)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(rename = "openai-compatible")]
    openai_compatible: RawFormat,
    #[serde(rename = "anthropic-messages")]
    anthropic_messages: RawFormat,
    gemini: RawFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFormat {
    small: Option<RawModel>,
    larger: Option<RawModel>,
    waits: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModel {
    /// The Model reference.
    model: String,
    retires: Option<Datetime>,
    source: String,
}

/// A model due for its issue.
#[derive(Debug, PartialEq, Eq)]
struct Due<'a> {
    reference: &'a str,
    retires: Date,
    source: &'a str,
    /// Each format and size that pins it.
    pins: Vec<(Format, Size)>,
}

impl Manifest {
    #[expect(
        clippy::disallowed_methods,
        reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
    )]
    fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("reading {}", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let raw: RawManifest = toml::from_str(text)?;
        let formats = [
            (Format::OpenAiCompatible, raw.openai_compatible),
            (Format::AnthropicMessages, raw.anthropic_messages),
            (Format::Gemini, raw.gemini),
        ]
        .into_iter()
        .map(|(format, raw)| Ok((format, pins(format, raw)?)))
        .collect::<Result<_>>()?;
        Ok(Self { formats })
    }

    /// The models that retire within [`NOTICE_DAYS`] of `today`, or before
    /// it, by Model reference.
    fn due(&self, today: Date) -> Vec<Due<'_>> {
        let mut due: BTreeMap<&str, Due<'_>> = BTreeMap::new();
        for (format, pins) in &self.formats {
            let Pins::Models { small, larger } = pins else {
                continue;
            };
            for (size, model) in [(Size::Small, small), (Size::Larger, larger)] {
                let Some(retires) = model.retires else {
                    continue;
                };
                if days(today, retires) > NOTICE_DAYS {
                    continue;
                }
                due.entry(&model.reference)
                    .or_insert_with(|| Due {
                        reference: &model.reference,
                        retires,
                        source: &model.source,
                        pins: Vec::new(),
                    })
                    .pins
                    .push((*format, size));
            }
        }
        due.into_values().collect()
    }
}

/// What the manifest's table of `format` pins: both models, or the reason
/// it waits, never part of either.
fn pins(format: Format, raw: RawFormat) -> Result<Pins> {
    let key = format.key();
    match (raw.small, raw.larger, raw.waits) {
        (Some(small), Some(larger), None) => Ok(Pins::Models {
            small: pinned(small).with_context(|| format!("[{key}]'s small model"))?,
            larger: pinned(larger).with_context(|| format!("[{key}]'s larger model"))?,
        }),
        (None, None, Some(waits)) if !waits.trim().is_empty() => Ok(Pins::Waits(waits)),
        _ => bail!(
            "[{key}] pins a small and a larger model, or says why it waits with `waits`, never part of either"
        ),
    }
}

fn pinned(raw: RawModel) -> Result<Pinned> {
    let reference = raw.model;
    let Some((provider, name)) = reference.split_once('/') else {
        bail!("`{reference}` is no Model reference, `provider/model`");
    };
    if provider.is_empty() || name.is_empty() || name.contains('/') {
        bail!("`{reference}` is no Model reference, `provider/model`");
    }
    // An alias such as `gpt-latest` moves to another model unannounced.
    if !name.bytes().any(|byte| byte.is_ascii_digit()) || name.ends_with("latest") {
        bail!("`{reference}` names no dated or versioned model: pin one by its date or version");
    }
    if !raw.source.starts_with("https://") {
        bail!(
            "`{reference}`'s source, `{}`, is no https:// address of the page that announces its retirement",
            raw.source
        );
    }
    let retires = raw
        .retires
        .map(|retires| date(&retires).with_context(|| format!("`{reference}`'s retirement")))
        .transpose()?;
    Ok(Pinned {
        reference,
        retires,
        source: raw.source,
    })
}

/// A TOML local date, such as `2027-04-01`.
fn date(value: &Datetime) -> Result<Date> {
    let (Some(day), None, None) = (value.date, value.time, value.offset) else {
        bail!("`{value}` is no date: write a local date, `YYYY-MM-DD`");
    };
    Ok(Date::new(
        i16::try_from(day.year).context("the year")?,
        i8::try_from(day.month).context("the month")?,
        i8::try_from(day.day).context("the day")?,
    )?)
}

/// The days from `today` to `day`, negative once it has passed.
fn days(today: Date, day: Date) -> i64 {
    i64::from(day.since(today).map_or(0, |span| span.get_days()))
}

/// The issue's body: what pins the model, when it retires and what to do.
fn body(model: &Due<'_>, today: Date) -> String {
    let pins: Vec<String> = [Size::Small, Size::Larger]
        .into_iter()
        .filter_map(|size| {
            let formats: Vec<String> = model
                .pins
                .iter()
                .filter(|(_, pinned)| *pinned == size)
                .map(|(format, _)| format!("of the {} format", format.name()))
                .collect();
            (!formats.is_empty())
                .then(|| format!("the {} model {}", size.name(), formats.join(" and ")))
        })
        .collect();
    let pins = pins.join(", and as ");
    let when = match days(today, model.retires) {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        days if days > 1 => format!("in {days} days"),
        -1 => "yesterday".to_owned(),
        days => format!("{} days ago", -days),
    };
    format!(
        "`{}`, which `{MANIFEST}` pins as {pins}, retires on {}, {when}, as {} announces.\n\n\
         Refresh its Model recordings before then (ADR 0065): pin a dated or versioned successor \
         from the Vercel AI Gateway's free tier (ADR 0076) beside it, run both side by side for five \
         nights, record the successor with the recorder job, then drop `{}` from the manifest. While \
         the manifest pins it, the daily test-health job keeps this issue current and reopens it.\n",
        model.reference, model.retires, model.source, model.reference
    )
}

#[cfg(test)]
mod tests;
