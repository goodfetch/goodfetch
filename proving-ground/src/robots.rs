//! The oracle's own reading of a robots.txt (RFC 9309), never the engine's:
//! whether a site's robots.txt lets a crawler, named by its `User-Agent`'s
//! product token, request a path. A group naming the crawler's token
//! replaces the `*` group, groups naming the same token combine, the longest
//! matching rule wins, `allow` wins a tie, and a rule's `*` matches any
//! octets and a final `$` the path's end. `/robots.txt` itself is always
//! allowed.

/// Whether `robots` lets the crawler whose `User-Agent` is `user_agent`
/// request `target` (a path and its query).
pub(crate) fn allows(robots: &str, user_agent: Option<&str>, target: &str) -> bool {
    if target == "/robots.txt" {
        return true;
    }
    let token = user_agent
        .and_then(|agent| agent.split(['/', ' ']).next())
        .unwrap_or_default();
    let groups = groups(robots);
    let named: Vec<&Group> = groups
        .iter()
        .filter(|group| {
            group
                .agents
                .iter()
                .any(|agent| !token.is_empty() && agent.eq_ignore_ascii_case(token))
        })
        .collect();
    let chosen = if named.is_empty() {
        groups
            .iter()
            .filter(|group| group.agents.iter().any(|agent| agent == "*"))
            .collect()
    } else {
        named
    };
    let mut best: Option<(usize, bool)> = None;
    for (allow, pattern) in chosen.iter().flat_map(|group| &group.rules) {
        if pattern.is_empty() || !matches(pattern.as_bytes(), target.as_bytes()) {
            continue;
        }
        let candidate = (pattern.len(), *allow);
        best = Some(match best {
            Some(current) if current.0 > candidate.0 => current,
            Some(current) if current.0 == candidate.0 => (current.0, current.1 || candidate.1),
            _ => candidate,
        });
    }
    best.is_none_or(|(_, allow)| allow)
}

/// One group: the user agents it names, and its rules (`true` for
/// `allow`) with their patterns.
struct Group {
    agents: Vec<String>,
    rules: Vec<(bool, String)>,
}

fn groups(robots: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut open = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match key.trim().to_ascii_lowercase().as_str() {
            "user-agent" => {
                if !open {
                    groups.push(Group {
                        agents: Vec::new(),
                        rules: Vec::new(),
                    });
                    open = true;
                }
                if let Some(group) = groups.last_mut() {
                    group.agents.push(value);
                }
            }
            rule @ ("allow" | "disallow") => {
                open = false;
                if let Some(group) = groups.last_mut() {
                    group.rules.push((rule == "allow", value));
                }
            }
            _ => {}
        }
    }
    groups
}

/// Whether `pattern` matches a prefix of `path` (all of it, with a final
/// `$`), its `*` matching any octets.
fn matches(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => true,
        Some((b'$', [])) => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len()).any(|at| matches(rest, &path[at..])),
        Some((octet, rest)) => path
            .split_first()
            .is_some_and(|(first, after)| first == octet && matches(rest, after)),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// The lab's robots.txt case (`rnd/lab/tests/identity-policy.test.ts`).
    const LAB: &str = "# comment
User-agent: GoodfetchLab
User-agent: OtherBot
Disallow: /
Allow: /public/

User-agent: *
Disallow: /example/page/disallowed.gif
Allow: /example/page/
Disallow: /*.pdf$
Allow: /tie
Disallow: /tie
Crawl-delay: 2
Content-Signal: search=yes, ai-train=no
";

    #[rstest]
    #[case::named_group_replaces_the_star_group(Some("GoodfetchLab/1.0"), "/catalog", false)]
    #[case::named_group_allows_its_allow(Some("goodfetchlab/2"), "/public/a", true)]
    #[case::longest_match_wins(None, "/example/page/disallowed.gif", false)]
    #[case::a_shorter_allow_applies_elsewhere(None, "/example/page/other", true)]
    #[case::star_and_dollar_are_patterns(None, "/doc.pdf", false)]
    #[case::dollar_anchors_the_end(None, "/doc.pdf?download=1", true)]
    #[case::allow_wins_a_tie(Some("Browser/1"), "/tie", true)]
    #[case::robots_txt_itself_is_allowed(Some("GoodfetchLab/1.0"), "/robots.txt", true)]
    fn the_lab_robots_txt_reads_as_rfc_9309_says(
        #[case] agent: Option<&str>,
        #[case] target: &str,
        #[case] allowed: bool,
    ) {
        assert_eq!(allows(LAB, agent, target), allowed);
    }

    #[test]
    fn no_matching_group_allows_everything() {
        assert!(allows(
            "User-agent: Other\nDisallow: /\n",
            Some("Mine/1"),
            "/a"
        ));
    }
}
