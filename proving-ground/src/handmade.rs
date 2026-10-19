//! The hand-made families (spec #152, story 29): pages and expected results
//! written by hand in data files, for situations a generator can't express,
//! here the lab's actionability, primitive, form, navigation, frames and
//! files fixtures (`pages/actionability.toml`, `pages/navigation.toml`).
//! Each task is served on its own host name; a page's script posts the event
//! that reached one of its controls to `/effects`, which the site records as
//! the task's Effect. A download's bytes come from the seed.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use hyper::body::Bytes;
use hyper::{Method, StatusCode};
use serde::Deserialize;

use crate::digest;
use crate::exchange::{Reply, Request};
use crate::task::{Effect, Entry, Expected, File, Kind, Reported, Task};

/// One hand-made family: its name, its host name, the host name of its
/// cross-origin frames, if it has any, and its data file.
struct Family {
    name: &'static str,
    host: &'static str,
    frame_host: Option<&'static str>,
    data: &'static LazyLock<Data>,
}

static ACTIONABILITY: LazyLock<Data> =
    LazyLock::new(|| parse(include_str!("../pages/actionability.toml")));
static NAVIGATION: LazyLock<Data> =
    LazyLock::new(|| parse(include_str!("../pages/navigation.toml")));

const FAMILIES: [Family; 2] = [
    Family {
        name: "actionability",
        host: "actionability.test",
        frame_host: None,
        data: &ACTIONABILITY,
    },
    Family {
        name: "navigation",
        host: "navigation.test",
        frame_host: Some("navigation-frame.test"),
        data: &NAVIGATION,
    },
];

/// A family's data file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    /// Files every task of the family serves, by path.
    shared: BTreeMap<String, String>,
    /// The tasks, in ordinal order from 1.
    tasks: Vec<HandMade>,
}

/// One hand-made task.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandMade {
    /// The task's name in its data file, for its readers.
    #[expect(dead_code, reason = "the name is for the data file's readers")]
    name: String,
    kind: Kind,
    description: String,
    /// The events its page's script reports when the task is done right.
    #[serde(default)]
    effects: Vec<Reported>,
    #[serde(default)]
    refusal: bool,
    /// The task's pages and files by path, `/` its start page.
    pages: BTreeMap<String, String>,
    download: Option<Download>,
}

/// A file a task's page offers to download, whose bytes come from the seed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Download {
    path: String,
    name: String,
}

fn parse(text: &str) -> Data {
    toml::from_str(text).expect("invariant: a hand-made family's data file is valid")
}

/// The bytes of a download for `seed`: the lab's seeded line, with a CRLF
/// and bytes that no text decoding keeps.
fn download_bytes(seed: u64) -> Vec<u8> {
    let mut bytes = format!("goodfetch-fixture:{seed}\r\n").into_bytes();
    bytes.extend_from_slice(&[0x00, 0xff, b'\n']);
    bytes
}

fn ordinals(family: &Family) -> impl Iterator<Item = (u32, &'static HandMade)> {
    let data: &'static Data = family.data;
    (1..).zip(data.tasks.iter())
}

/// Every host name of the hand-made families, each with its task: each
/// task's own, and its frame host's.
pub(crate) fn hosts() -> Vec<(String, String)> {
    let mut hosts = Vec::new();
    for family in &FAMILIES {
        for (ordinal, _) in ordinals(family) {
            let task = format!("{}/{ordinal}", family.name);
            hosts.push((format!("{ordinal}.{}", family.host), task.clone()));
            if let Some(frame_host) = family.frame_host {
                hosts.push((format!("{ordinal}.{frame_host}"), task));
            }
        }
    }
    hosts
}

/// The hand-made families' tasks, their start URLs with `port`.
pub(crate) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    let mut tasks = Vec::new();
    for family in &FAMILIES {
        for (ordinal, task) in ordinals(family) {
            let host = format!("{ordinal}.{}", family.host);
            tasks.push(Task {
                entry: Entry {
                    id: format!("{}/{ordinal}", family.name),
                    family: family.name,
                    kind: task.kind,
                    start_url: format!("http://{host}:{port}/"),
                    description: &task.description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected: Expected {
                    effects: task.effects.iter().cloned().map(Effect::from).collect(),
                    files: task
                        .download
                        .iter()
                        .map(|download| File {
                            name: download.name.clone(),
                            sha256: digest::sha256(&download_bytes(seed)),
                        })
                        .collect(),
                    refusal: task.refusal,
                    ..Expected::default()
                },
            });
        }
    }
    tasks
}

/// The family, ordinal and task of a host name, without its port, and
/// whether it is the task's frame host.
fn find(name: &str) -> Option<(&'static Family, u32, &'static HandMade)> {
    FAMILIES.iter().find_map(|family| {
        let ordinal = name
            .strip_suffix(family.host)
            .or_else(|| family.frame_host.and_then(|frame| name.strip_suffix(frame)))?
            .strip_suffix('.')?;
        let number = ordinal
            .parse::<u32>()
            .ok()
            .filter(|n| n.to_string() == ordinal)?;
        let task = family
            .data
            .tasks
            .get(usize::try_from(number).ok()?.checked_sub(1)?)?;
        Some((family, number, task))
    })
}

/// Answers a request to a hand-made task's host name, `name` without its
/// port, if it is one: the reply, and the Effect its page reported.
pub(crate) fn respond(
    seed: u64,
    port: u16,
    name: &str,
    request: &Request<'_>,
) -> Option<(Reply, Option<Effect>)> {
    let (family, ordinal, task) = find(name)?;
    Some(match *request.method {
        Method::POST if request.path == "/effects" => {
            match serde_json::from_slice::<Reported>(request.body) {
                Ok(reported) => (Reply::no_content(), Some(reported.into())),
                Err(_) => (
                    Reply::error(StatusCode::BAD_REQUEST, "malformed_effect"),
                    None,
                ),
            }
        }
        Method::GET => (get(seed, port, family, ordinal, task, request.path), None),
        _ => (Reply::not_found(), None),
    })
}

fn get(seed: u64, port: u16, family: &Family, ordinal: u32, task: &HandMade, path: &str) -> Reply {
    if let Some(download) = task
        .download
        .as_ref()
        .filter(|download| download.path == path)
    {
        return Reply {
            status: StatusCode::OK,
            content_type: "application/octet-stream",
            headers: vec![(
                "content-disposition",
                format!("attachment; filename=\"{}\"", download.name),
            )],
            body: Bytes::from(download_bytes(seed)),
        };
    }
    let Some(page) = task
        .pages
        .get(path)
        .or_else(|| family.data.shared.get(path))
    else {
        return Reply::not_found();
    };
    let page = match family.frame_host {
        Some(frame_host) => page.replace(
            "{frame_origin}",
            &format!("//{ordinal}.{frame_host}:{port}"),
        ),
        None => page.clone(),
    };
    Reply {
        status: StatusCode::OK,
        content_type: content_type(path),
        headers: Vec::new(),
        body: page.into(),
    }
}

/// A served file's media type, from its extension; a path without one is a
/// page.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "text/html; charset=utf-8",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hand_made_family_parses_and_each_task_has_a_start_page() {
        for family in &FAMILIES {
            for (ordinal, task) in ordinals(family) {
                assert!(
                    task.pages.contains_key("/"),
                    "{}/{ordinal} has no start page",
                    family.name
                );
            }
        }
    }
}
