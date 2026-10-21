//! robots.txt's group selection over a table of files' groups (RFC 9309
//! §2.2.1; ADRs 0086 and 0090): which rules a crawl obeys, for Foxhound's
//! own product token and the operator's.

use goodfetch_page::robots_txt::{Group, ProductToken, applicable_rules};
use rstest::rstest;

/// A group of `user_agents` whose rules are `rules`.
fn group(user_agents: &[&str], rules: &[&'static str]) -> Group<&'static str> {
    Group {
        user_agents: user_agents.iter().map(|&agent| agent.to_owned()).collect(),
        rules: rules.to_vec(),
    }
}

#[rstest]
#[case::bare_foxhound_names_no_group(
    vec![group(&["Foxhound"], &["Disallow: /foxhound"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /all"]
)]
#[case::token_in_capitals(
    vec![group(&["GOODFETCH-FOXHOUND"], &["Disallow: /ours"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /ours"]
)]
#[case::token_with_a_version(
    vec![group(&["goodfetch-foxhound/1.2"], &["Disallow: /ours"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /ours"]
)]
#[case::longer_token_is_another_crawler(
    vec![group(&["goodfetch-foxhound-beta"], &["Disallow: /beta"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /all"]
)]
#[case::token_inside_a_user_agent_string(
    vec![group(&["Mozilla/5.0 (compatible; goodfetch-foxhound)"], &["Disallow: /ua"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /all"]
)]
#[case::operator_token(
    vec![group(&["AcmeBot"], &["Disallow: /acme"]), group(&["*"], &["Disallow: /all"])],
    Some("acmebot"),
    vec!["Disallow: /acme"]
)]
#[case::operator_token_unset(
    vec![group(&["AcmeBot"], &["Disallow: /acme"]), group(&["*"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /all"]
)]
#[case::both_tokens_merge_in_file_order(
    vec![
        group(&["AcmeBot"], &["Disallow: /acme"]),
        group(&["*"], &["Disallow: /all"]),
        group(&["goodfetch-foxhound"], &["Disallow: /ours", "Allow: /ours/open"]),
    ],
    Some("AcmeBot"),
    vec!["Disallow: /acme", "Disallow: /ours", "Allow: /ours/open"]
)]
#[case::two_groups_of_one_token_merge(
    vec![
        group(&["goodfetch-foxhound"], &["Disallow: /a"]),
        group(&["*"], &["Disallow: /all"]),
        group(&["goodfetch-foxhound"], &["Disallow: /b"]),
    ],
    None,
    vec!["Disallow: /a", "Disallow: /b"]
)]
#[case::group_naming_both_tokens_counts_once(
    vec![group(&["AcmeBot", "goodfetch-foxhound"], &["Disallow: /both"])],
    Some("AcmeBot"),
    vec!["Disallow: /both"]
)]
#[case::star_groups_merge_when_none_matches(
    vec![
        group(&["*"], &["Disallow: /a"]),
        group(&["OtherBot"], &["Disallow: /other"]),
        group(&["*"], &["Disallow: /b"]),
    ],
    None,
    vec!["Disallow: /a", "Disallow: /b"]
)]
#[case::star_ignored_when_a_token_matches(
    vec![group(&["*"], &["Disallow: /all"]), group(&["goodfetch-foxhound"], &[])],
    None,
    vec![]
)]
#[case::no_group_applies(
    vec![group(&["OtherBot"], &["Disallow: /other"])],
    None,
    vec![]
)]
#[case::star_with_trailing_text(
    vec![group(&["* everyone"], &["Disallow: /all"])],
    None,
    vec!["Disallow: /all"]
)]
#[case::star_inside_a_token_is_no_wildcard(
    vec![group(&["goodfetch-*"], &["Disallow: /star"])],
    None,
    vec![]
)]
fn a_crawl_obeys_the_groups_its_product_tokens_name_else_star(
    #[case] groups: Vec<Group<&'static str>>,
    #[case] operator: Option<&str>,
    #[case] expected: Vec<&'static str>,
) {
    let operator = operator.map(|token| ProductToken::new(token).unwrap());

    let rules: Vec<&str> = applicable_rules(&groups, operator.as_ref())
        .into_iter()
        .copied()
        .collect();

    assert_eq!(rules, expected);
}

#[rstest]
#[case::empty("")]
#[case::space("acme bot")]
#[case::slash("acme/1.0")]
#[case::digit("acme2")]
#[case::star("*")]
fn a_product_token_holds_only_letters_underscores_and_hyphens(#[case] token: &str) {
    let refused = ProductToken::new(token);

    assert!(refused.is_err());
}

#[test]
fn foxhounds_product_token_is_qualified() {
    let token = ProductToken::foxhound();

    assert_eq!(token.as_str(), "goodfetch-foxhound");
}
