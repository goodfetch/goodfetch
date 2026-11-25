//! The Proving ground's codec and font samples for one Identity profile
//! (ADR 0074): the Goodfetch Chromium's codec test holds that every codec
//! the Identity profile claims decodes its sample on `codecs.test`, and its
//! font test that every font the Identity profile lists renders on
//! `fonts.test` and no host font outside it does.
//! [`ProvingGround::codec_samples`] and [`ProvingGround::font_samples`]
//! list what those pages serve for the Identity profile's codecs and
//! families, read from the pages themselves, so a test names what it
//! expects without copying the Proving ground's lists.

use serde::Deserialize;
use serde_json::Value;

use crate::proving_ground::ProvingGround;

/// The host name of the codec samples.
pub const CODECS_HOST: &str = "codecs.test";

/// The host name of the font pages.
pub const FONTS_HOST: &str = "fonts.test";

/// One codec's sample on [`CODECS_HOST`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CodecSample {
    /// The codec, such as `h264`.
    pub name: String,
    /// Its file's name, served at [`path`](Self::path).
    pub file: String,
    /// The file's media type.
    pub media_type: String,
    /// The type with its `codecs` parameter, as a page asks `canPlayType`
    /// and Media Capabilities about it.
    #[serde(rename = "type")]
    pub asked_type: String,
    /// Whether it holds video or audio.
    pub kind: MediaKind,
    /// What decoding it gives: a video's `width`, `height`, `duration` and
    /// first frame's `color`, an audio's `channels`, `duration` and
    /// `frequency`.
    pub expected: Value,
}

impl CodecSample {
    /// The path the sample is served at.
    pub fn path(&self) -> String {
        format!("/samples/{}", self.file)
    }
}

/// What a [`CodecSample`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    /// A video track.
    Video,
    /// An audio track.
    Audio,
}

/// The font page for one Identity profile's families on [`FONTS_HOST`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSamples {
    /// The page's path and query, which names the families.
    pub path: String,
    /// Each font the page measures: the listed families, then each host
    /// font the Identity profile does not list.
    pub fonts: Vec<FontSample>,
}

/// One font the font page measures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSample {
    /// Its family.
    pub family: String,
    /// Whether the Identity profile lists it.
    pub listed: bool,
    /// Whether it must render.
    pub expected: FontAvailability,
}

/// Whether a font must render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontAvailability {
    /// Listed by the Identity profile, so it must render.
    Available,
    /// A host font the Identity profile does not list, so it must not.
    Unavailable,
}

/// Why the samples could not be listed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SamplesError {
    /// The Proving ground's page gave no answer, or not a `200`.
    #[error("{host}{path} gave no listing: {reason}")]
    Unanswered {
        /// The page's host.
        host: &'static str,
        /// Its path.
        path: String,
        /// What reading it gave.
        reason: String,
    },
    /// The page's answer is not what it describes.
    #[error("{host}{path} answered what this helper cannot read: {reason}")]
    Unreadable {
        /// The page's host.
        host: &'static str,
        /// Its path.
        path: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The Identity profile claims a codec the Proving ground has no sample of, which
    /// the codec test could then never hold.
    #[error("the Proving ground has no sample of the codec {0:?}")]
    NoSample(String),
}

/// `/samples.json`'s listing.
#[derive(Deserialize)]
struct Listing {
    samples: Vec<CodecSample>,
}

impl ProvingGround {
    /// The samples of each codec of `codecs`, the codecs an Identity
    /// profile claims, in that order.
    ///
    /// # Errors
    ///
    /// `Unanswered` or `Unreadable` when `codecs.test`'s listing cannot be
    /// read, and `NoSample` when a codec has no sample.
    pub fn codec_samples(&self, codecs: &[&str]) -> Result<Vec<CodecSample>, SamplesError> {
        let path = "/samples.json";
        let listing: Listing =
            serde_json::from_str(&self.page(CODECS_HOST, path)?).map_err(|error| {
                SamplesError::Unreadable {
                    host: CODECS_HOST,
                    path: path.to_owned(),
                    reason: error.to_string(),
                }
            })?;
        codecs
            .iter()
            .map(|codec| {
                listing
                    .samples
                    .iter()
                    .find(|sample| sample.name == *codec)
                    .cloned()
                    .ok_or_else(|| SamplesError::NoSample((*codec).to_owned()))
            })
            .collect()
    }

    /// The font page for `families`, the families an Identity profile
    /// lists, and each font it measures.
    ///
    /// # Errors
    ///
    /// `Unanswered` when `fonts.test` refuses a family, as it does one with
    /// a character outside ASCII letters, digits, spaces, `-`, `_` and `.`,
    /// and `Unreadable` when its page lists no font.
    pub fn font_samples(&self, families: &[&str]) -> Result<FontSamples, SamplesError> {
        let query: Vec<String> = families
            .iter()
            .map(|family| format!("family={}", percent_encoded(family)))
            .collect();
        let path = format!("/?{}", query.join("&"));
        let page = self.page(FONTS_HOST, &path)?;
        let fonts: Vec<FontSample> = page
            .split("<p class=\"font\" ")
            .skip(1)
            .map(|element| {
                let attribute = |name: &str| {
                    let start = element.find(&format!("{name}=\""))? + name.len() + 2;
                    let length = element[start..].find('"')?;
                    Some(&element[start..start + length])
                };
                let expected = match attribute("data-expected")? {
                    "available" => FontAvailability::Available,
                    "unavailable" => FontAvailability::Unavailable,
                    _ => return None,
                };
                Some(FontSample {
                    family: attribute("data-family")?.to_owned(),
                    listed: attribute("data-listed")?.parse().ok()?,
                    expected,
                })
            })
            .collect::<Option<_>>()
            .filter(|fonts: &Vec<FontSample>| !fonts.is_empty())
            .ok_or_else(|| SamplesError::Unreadable {
                host: FONTS_HOST,
                path: path.clone(),
                reason:
                    "its fonts lack a data-family, data-listed or data-expected, or it lists none"
                        .to_owned(),
            })?;
        Ok(FontSamples { path, fonts })
    }

    /// The body of `path` on the host `host`, over plain HTTP to the
    /// Proving ground's own address.
    fn page(&self, host: &'static str, path: &str) -> Result<String, SamplesError> {
        ureq::get(format!("http://{}{path}", self.address()))
            .header("Host", host)
            .call()
            .and_then(|mut response| response.body_mut().read_to_string())
            .map_err(|error| SamplesError::Unanswered {
                host,
                path: path.to_owned(),
                reason: error.to_string(),
            })
    }
}

/// `text` with every byte but an unreserved one percent-encoded, as a query
/// value.
fn percent_encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
