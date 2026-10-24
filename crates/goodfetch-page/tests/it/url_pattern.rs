//! A regular expression URL pattern: matched anywhere in a URL in the
//! `regex` crate's syntax, and refused when it is compiled, naming its
//! construct, when it uses one outside that syntax (ADR 0081).
//!
//! Covers: story:5-43

use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::page::PatternConstruct;
use rstest::rstest;

#[rstest]
#[case::anywhere_in_the_url(r"/api/v\d+/", "https://site.test/api/v2/items", true)]
#[case::anchored_to_the_start("^https://site\\.test/$", "https://site.test/", true)]
#[case::anchored_elsewhere("^/api", "https://site.test/api", false)]
#[case::case_insensitive_flag("(?i)ITEMS", "https://site.test/items", true)]
#[case::unicode_class(r"\w+\.png$", "https://site.test/caf\u{e9}.png", true)]
fn a_regular_expression_matches_anywhere_in_the_url(
    #[case] source: &str,
    #[case] url: &str,
    #[case] matches: bool,
) {
    let pattern = UrlPattern::regex(source).unwrap();

    assert_eq!(pattern.matches(url), matches);
}

#[rstest]
#[case::look_ahead(r"/items(?=\?)", PatternConstruct::LookAround)]
#[case::negative_look_ahead(r"/items(?!/new)", PatternConstruct::LookAround)]
#[case::look_behind(r"(?<=/api)/items", PatternConstruct::LookAround)]
#[case::negative_look_behind(r"(?<!/admin)/items", PatternConstruct::LookAround)]
#[case::numbered_back_reference(r"/(\w+)/\1", PatternConstruct::Backreference)]
#[case::named_back_reference(r"/(?<part>\w+)/\k<part>", PatternConstruct::Backreference)]
#[case::pythons_named_back_reference(r"/(?P<part>\w+)/(?P=part)", PatternConstruct::Backreference)]
#[case::javascripts_null_character(r"/items\0", PatternConstruct::Other)]
#[case::unclosed_group(r"/items(", PatternConstruct::Other)]
#[case::javascripts_any_character_class(r"/items/[^]", PatternConstruct::Other)]
fn a_construct_outside_the_regex_syntax_is_refused_by_name(
    #[case] source: &str,
    #[case] construct: PatternConstruct,
) {
    let refused = UrlPattern::regex(source).unwrap_err();

    assert_eq!(refused.construct(), construct);
}
