//! The redirect stand-ins (ADR 0047): DuckDuckGo's tracker surrogates and
//! the plain stand-ins, each under the names Filter lists use for it, from
//! the alias table the build reads.

use adblock::resources::{MimeType, Resource, ResourceType};
use data_encoding::BASE64;

/// One stand-in.
pub(crate) struct StandIn {
    /// The names a redirect rule gives it, its main one first.
    pub(crate) names: &'static [&'static str],
    /// Its media type.
    pub(crate) mime: &'static str,
    /// Its body.
    pub(crate) body: Body,
}

/// A stand-in's body.
pub(crate) enum Body {
    Bytes(&'static [u8]),
    Base64(&'static str),
}

include!(concat!(env!("OUT_DIR"), "/stand_ins.rs"));

/// The stand-ins as `adblock` resources.
pub(crate) fn resources() -> Vec<Resource> {
    STAND_INS
        .iter()
        .filter_map(|stand_in| {
            let (name, aliases) = stand_in.names.split_first()?;
            let content = match stand_in.body {
                Body::Bytes(bytes) => BASE64.encode(bytes),
                Body::Base64(base64) => base64.to_owned(),
            };
            Some(Resource {
                name: (*name).to_owned(),
                aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                kind: ResourceType::Mime(MimeType::from(stand_in.mime)),
                content,
                dependencies: Vec::new(),
                permission: adblock::resources::PermissionMask::default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
