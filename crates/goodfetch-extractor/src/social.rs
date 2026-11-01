//! The closed list of social networks whose profile links `contacts`
//! reads (spec #580, ticket #787), versioned with the extractor: a link
//! is a profile when its host is one of a network's hosts, or a subdomain
//! a person's browser reaches it on (`www.`, `m.`, `mobile.`), and its
//! path names something other than the network's home or one of its
//! sharing endpoints; and Mastodon, whose instances no list holds, by a
//! `rel="me"` link to a `/@name` path.

use goodfetch_protocol::page::SocialNetwork;
use url::Url;

/// The version of [`NETWORKS`]: it changes whenever the list does, which a
/// test holds by the list's digest.
pub const SOCIAL_NETWORKS_VERSION: u32 = 1;

/// One listed network.
#[derive(Debug)]
pub(crate) struct Listed {
    /// The network.
    pub(crate) network: SocialNetwork,
    /// Its hosts.
    pub(crate) hosts: &'static [&'static str],
    /// The first path segments of its pages that are no profile: sharing,
    /// posting and login endpoints.
    pub(crate) not_profiles: &'static [&'static str],
}

/// The listed networks.
pub(crate) const NETWORKS: &[Listed] = &[
    Listed {
        network: SocialNetwork::X,
        hosts: &["x.com", "twitter.com"],
        not_profiles: &["intent", "share", "home", "search", "hashtag", "i", "login"],
    },
    Listed {
        network: SocialNetwork::Linkedin,
        hosts: &["linkedin.com"],
        not_profiles: &["shareArticle", "sharing", "feed", "login", "jobs"],
    },
    Listed {
        network: SocialNetwork::Facebook,
        hosts: &["facebook.com", "fb.com"],
        not_profiles: &[
            "sharer",
            "sharer.php",
            "share.php",
            "dialog",
            "login",
            "plugins",
        ],
    },
    Listed {
        network: SocialNetwork::Instagram,
        hosts: &["instagram.com"],
        not_profiles: &["p", "reel", "explore", "accounts"],
    },
    Listed {
        network: SocialNetwork::Youtube,
        hosts: &["youtube.com"],
        not_profiles: &["watch", "embed", "shorts", "results", "playlist"],
    },
    Listed {
        network: SocialNetwork::Tiktok,
        hosts: &["tiktok.com"],
        not_profiles: &["share", "embed", "tag", "login"],
    },
    Listed {
        network: SocialNetwork::Github,
        hosts: &["github.com"],
        not_profiles: &["login", "search", "features", "pricing", "sponsors"],
    },
];

/// The subdomains a listed host is also reached on.
const SUBDOMAINS: &[&str] = &["www.", "m.", "mobile."];

/// The network whose profile `url` is, a link whose `rel` holds `me` when
/// `rel_me`; `None` when it is none.
pub(crate) fn profile(url: &Url, rel_me: bool) -> Option<SocialNetwork> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    let listed = NETWORKS.iter().find(|listed| {
        listed.hosts.iter().any(|&listed_host| {
            host == listed_host
                || SUBDOMAINS
                    .iter()
                    .any(|subdomain| host.strip_prefix(subdomain) == Some(listed_host))
        })
    });
    match listed {
        Some(listed) => (!listed.not_profiles.contains(&first)).then_some(listed.network),
        None => (rel_me && first.len() > 1 && first.starts_with('@') && segments.next().is_none())
            .then_some(SocialNetwork::Mastodon),
    }
}
