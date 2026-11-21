use std::fmt::Write as _;
use std::time::Duration;

use proptest::prelude::*;
use rstest::rstest;

use super::*;

/// RFC 9309 §5.1's simple example.
const SIMPLE: &[u8] = include_bytes!("fixtures/rfc-9309-simple.txt");
/// RFC 9309 §5.2's longest match example.
const LONGEST_MATCH: &[u8] = include_bytes!("fixtures/rfc-9309-longest-match.txt");
/// A file with a byte order mark, CRLF and CR line ends, comments, keys in
/// any case, spaces around the colon, a Crawl-delay and a Sitemap line.
const WINDOWS: &[u8] = include_bytes!("fixtures/windows-and-comments.txt");

/// The group of `file` whose `User-agent` lines name `agent` exactly, in
/// any case: the test's own reading, not the engine's group selection.
fn group<'a>(file: &'a RobotsTxt, agent: &str) -> &'a Group {
    file.groups()
        .iter()
        .find(|group| {
            group
                .user_agents()
                .iter()
                .any(|named| named.eq_ignore_ascii_case(agent))
        })
        .unwrap_or_else(|| panic!("no group names {agent}"))
}

/// Whether `rules` let a crawler request `target`.
fn allows(rules: &[Rule], target: &str) -> bool {
    matches!(judge(rules, target), Judgment::Allowed(_))
}

#[rstest]
#[case::star_disallows_a_gif("*", "/images/logo.gif", false)]
#[case::star_dollar_anchors_the_gif("*", "/images/logo.gif?size=2", true)]
#[case::star_disallows_example("*", "/example/index.html", false)]
#[case::star_allows_publications("*", "/publications/2024", true)]
#[case::star_allows_the_rest("*", "/about", true)]
#[case::foobot_is_disallowed_everywhere("foobot", "/about", false)]
#[case::foobot_allows_its_page("foobot", "/example/page.html", true)]
#[case::foobot_allows_its_gif_over_nothing_longer("foobot", "/example/allowed.gif", true)]
#[case::barbot_disallows_the_page("barbot", "/example/page.html", false)]
#[case::bazbot_shares_barbots_group("bazbot", "/example/page.html", false)]
#[case::bazbot_allows_the_rest("bazbot", "/example/other.html", true)]
#[case::quxbot_has_no_rule("quxbot", "/example/page.html", true)]
fn rfc_9309s_simple_example_reads_as_the_rfc_says(
    #[case] agent: &str,
    #[case] target: &str,
    #[case] allowed: bool,
) {
    let file = parse(SIMPLE);

    assert_eq!(allows(group(&file, agent).rules(), target), allowed);
}

#[test]
fn rfc_9309s_simple_example_has_four_groups_in_file_order() {
    let file = parse(SIMPLE);

    let agents: Vec<&[String]> = file.groups().iter().map(Group::user_agents).collect();
    assert_eq!(
        agents,
        [
            &["*".to_owned()][..],
            &["foobot".to_owned()],
            &["barbot".to_owned(), "bazbot".to_owned()],
            &["quxbot".to_owned()],
        ]
    );
}

#[rstest]
#[case::the_longer_disallow_wins("/example/page/disallowed.gif", false)]
#[case::the_allow_applies_elsewhere("/example/page/other.gif", true)]
#[case::no_rule_matches("/elsewhere", true)]
fn rfc_9309s_longest_match_example_reads_as_the_rfc_says(
    #[case] target: &str,
    #[case] allowed: bool,
) {
    let file = parse(LONGEST_MATCH);

    assert_eq!(allows(group(&file, "foobot").rules(), target), allowed);
}

#[test]
fn a_disallow_names_its_rule_its_line_and_its_group() {
    let file = parse(SIMPLE);

    let Judgment::Disallowed(rule) = judge(group(&file, "barbot").rules(), "/example/page.html")
    else {
        panic!("the page is allowed");
    };
    assert_eq!(
        (rule.to_string(), rule.line(), rule.group()),
        ("Disallow: /example/page.html".to_owned(), 13, 2)
    );
}

#[rstest]
#[case::cr_lf_bom_and_comments("ExampleBot", "/private/secret", false)]
#[case::the_second_user_agent_line_joins_the_group("otherbot", "/private/secret", false)]
#[case::a_tab_before_the_value("ExampleBot", "/private/open/door", true)]
#[case::cr_alone_ends_a_line("*", "/files/report.pdf", false)]
#[case::a_question_mark_is_a_literal("*", "/cart?item=1", false)]
#[case::the_path_without_a_query("*", "/cart", true)]
fn a_file_with_any_line_ends_and_spellings_reads(
    #[case] agent: &str,
    #[case] target: &str,
    #[case] allowed: bool,
) {
    let file = parse(WINDOWS);

    assert_eq!(allows(group(&file, agent).rules(), target), allowed);
}

#[test]
fn crawl_delay_and_sitemap_lines_are_kept() {
    let file = parse(WINDOWS);

    assert_eq!(
        (
            group(&file, "ExampleBot").crawl_delay(),
            group(&file, "*").crawl_delay(),
            file.sitemaps(),
        ),
        (
            Some(Duration::from_millis(2500)),
            None,
            &["https://example.com/sitemap.xml".to_owned()][..],
        )
    );
}

/// A `Content-Signal` line belongs to its group, as Cloudflare's Content
/// Signals Policy places it, each `name=yes` or `name=no` read in order,
/// any other value saying nothing.
#[test]
fn content_signal_lines_are_kept_with_their_group() {
    let file = parse(
        b"User-agent: ExampleBot\nContent-Signal: search=yes, AI-Train=no, ai-input=maybe\nDisallow: /x\n\nUser-agent: *\nAllow: /\n",
    );

    assert_eq!(
        (
            group(&file, "ExampleBot").content_signals(),
            group(&file, "*").content_signals(),
        ),
        (
            &[("search".to_owned(), true), ("ai-train".to_owned(), false)][..],
            &[][..],
        )
    );
}

#[rstest]
#[case::star_matches_any_octets("Disallow: /fish*.php", "/fish/salmon.php", false)]
#[case::star_matches_nothing_too("Disallow: /fish*.php", "/fish.php", false)]
#[case::star_needs_the_rest("Disallow: /fish*.php", "/fish/salmon.html", true)]
#[case::dollar_ends_the_path("Disallow: /*.php$", "/index.php", false)]
#[case::dollar_refuses_more("Disallow: /*.php$", "/index.php?x=1", true)]
#[case::a_dollar_inside_is_a_literal("Disallow: /a$b", "/a$b", false)]
#[case::a_rule_is_a_prefix("Disallow: /fish", "/fishheads/yummy.html", false)]
#[case::rules_match_case_sensitively("Disallow: /fish", "/Fish.asp", true)]
#[case::an_empty_disallow_disallows_nothing("Disallow:", "/anything", true)]
#[case::allow_wins_a_tie("Disallow: /page\nAllow: /page", "/page", true)]
#[case::the_longest_rule_wins("Allow: /p\nDisallow: /p*", "/page", false)]
#[case::robots_txt_is_always_allowed("Disallow: /", "/robots.txt", true)]
#[case::an_encoded_star_is_a_literal(
    "Disallow: /file-with-a-%2A.html",
    "/file-with-a-*.html",
    false
)]
#[case::an_encoded_dollar_is_a_literal("Disallow: /path/foo-%24", "/path/foo-$", false)]
#[case::an_encoded_unreserved_octet_is_decoded(
    "Disallow: /foo/bar/%62%61%7A",
    "/foo/bar/baz",
    false
)]
#[case::a_url_encoded_unreserved_octet_is_decoded(
    "Disallow: /foo/bar/baz",
    "/foo/bar/%62az",
    false
)]
#[case::non_ascii_is_compared_encoded("Disallow: /foo/bar/ツ", "/foo/bar/%E3%83%84", false)]
#[case::encoded_hex_in_any_case("Disallow: /foo/bar/%e3%83%84", "/foo/bar/%E3%83%84", false)]
#[case::an_encoded_slash_stays_encoded("Disallow: /a%2Fb", "/a/b", true)]
fn rules_match_paths_as_rfc_9309_says(
    #[case] rules: &str,
    #[case] target: &str,
    #[case] allowed: bool,
) {
    let file = parse(format!("User-agent: *\n{rules}\n").as_bytes());

    assert_eq!(allows(group(&file, "*").rules(), target), allowed);
}

#[rstest]
#[case::rules_before_any_user_agent_belong_to_no_group(
    "Disallow: /early\nUser-agent: a\nDisallow: /late\n",
    &[&["a"][..]],
    &[&["Disallow: /late"][..]]
)]
#[case::a_user_agent_after_rules_starts_a_group(
    "User-agent: a\nDisallow: /a\nUser-agent: b\nDisallow: /b\n",
    &[&["a"][..], &["b"]],
    &[&["Disallow: /a"][..], &["Disallow: /b"]]
)]
#[case::a_sitemap_line_ends_no_run_of_user_agents(
    "User-agent: a\nSitemap: https://example.com/s.xml\nUser-agent: b\nDisallow: /ab\n",
    &[&["a", "b"][..]],
    &[&["Disallow: /ab"][..]]
)]
#[case::an_unknown_line_is_ignored(
    "User-agent: a\nNoindex: /x\nDisallow: /a\n",
    &[&["a"][..]],
    &[&["Disallow: /a"][..]]
)]
#[case::a_line_without_a_colon_is_ignored(
    "User-agent: a\nDisallow /x\nDisallow: /a\n",
    &[&["a"][..]],
    &[&["Disallow: /a"][..]]
)]
fn groups_follow_rfc_9309s_grammar(
    #[case] text: &str,
    #[case] agents: &[&[&str]],
    #[case] rules: &[&[&str]],
) {
    let file = parse(text.as_bytes());

    let read: (Vec<Vec<&str>>, Vec<Vec<String>>) = (
        file.groups()
            .iter()
            .map(|group| group.user_agents().iter().map(String::as_str).collect())
            .collect(),
        file.groups()
            .iter()
            .map(|group| group.rules().iter().map(ToString::to_string).collect())
            .collect(),
    );
    assert_eq!(
        read,
        (
            agents.iter().map(|agents| agents.to_vec()).collect(),
            rules
                .iter()
                .map(|rules| rules.iter().map(|&rule| rule.to_owned()).collect())
                .collect()
        )
    );
}

#[test]
fn only_the_first_500_kibibytes_are_read() {
    let mut text = b"User-agent: *\nDisallow: /kept\n".to_vec();
    text.resize(MAX_SIZE, b'\n');
    text.extend_from_slice(b"Disallow: /beyond\n");

    let file = parse(&text);

    assert_eq!(
        (
            allows(group(&file, "*").rules(), "/kept"),
            allows(group(&file, "*").rules(), "/beyond"),
        ),
        (false, true)
    );
}

#[rstest]
#[case::path_and_query("https://example.com/a/b?c=d#e", "/a/b?c=d")]
#[case::root("https://example.com", "/")]
#[case::non_ascii_encoded("https://example.com/caf\u{e9}", "/caf%C3%A9")]
fn a_urls_target_is_its_path_and_query(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(target(&url::Url::parse(url).unwrap()), expected);
}

#[rstest]
#[case::default_port("https://example.com/a?b", Some("https://example.com/robots.txt"))]
#[case::own_port(
    "http://example.com:8080/a",
    Some("http://example.com:8080/robots.txt")
)]
#[case::punycode(
    "http://b\u{fc}cher.example/",
    Some("http://xn--bcher-kva.example/robots.txt")
)]
#[case::no_http("about:blank", None)]
#[case::a_file("file:///tmp/a.html", None)]
fn a_urls_robots_txt_is_its_origins(#[case] url: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        location(&url::Url::parse(url).unwrap()).map(|location| location.to_string()),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::found(
    200,
    b"User-agent: *\nDisallow: /a\n",
    Fetched::File(parse(b"User-agent: *\nDisallow: /a\n"))
)]
#[case::not_found(404, b"", Fetched::NotFound(404))]
#[case::gone(410, b"", Fetched::NotFound(410))]
#[case::too_many_requests(429, b"", Fetched::Unreachable("answered 429".to_owned()))]
#[case::server_error(503, b"", Fetched::Unreachable("answered 503".to_owned()))]
fn a_response_reads_as_rfc_9309_says(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] expected: Fetched,
) {
    assert_eq!(Fetched::of_response(status, body), expected);
}

/// Runs `body` on a Run's own copies over a fresh run file in `env`'s
/// folder, then closes the file.
fn with_copies<F, Fut>(env: &goodfetch_test_support::TestEnv, body: F)
where
    F: FnOnce(RunCopies, crate::run_state::RunState) -> Fut,
    Fut: Future<Output = ()>,
{
    use crate::run_state::{RunState, RunStateOptions};
    use crate::tasks::{BlockingPolicy, TaskOwner};

    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            env.folder().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![TABLES],
            1,
            &owner,
        )
        .await
        .unwrap();
        body(RunCopies::default(), state).await;
        owner.wait().await;
    });
}

/// The robots.txt of the origin `https://site-<n>.test`.
fn location_of(origin: usize) -> url::Url {
    url::Url::parse(&format!("https://site-{origin}.test/robots.txt")).unwrap()
}

#[test]
fn a_runs_copies_fetch_each_origin_once() {
    let env = goodfetch_test_support::TestEnv::new();
    with_copies(&env, |copies, state| async move {
        let fetches = std::cell::Cell::new(0);
        let fetch = || async {
            fetches.set(fetches.get() + 1);
            tokio::task::yield_now().await;
            Ok(Answer::Response {
                status: 503,
                body: Vec::new(),
            })
        };
        let first = location_of(1);
        let second = location_of(2);

        let shared = copies.clone();
        let (one, two) = tokio::join!(
            copies.copy(&state, &first, fetch()),
            shared.copy(&state, &first, fetch())
        );
        let three = copies.copy(&state, &first, fetch()).await;
        let other = copies.copy(&state, &second, fetch()).await;

        assert_eq!(
            (fetches.get(), one == two && two == three, other),
            (
                2,
                true,
                Ok(Arc::new(Fetched::Unreachable("answered 503".to_owned())))
            )
        );
    });
}

/// Covers: story:6-62
#[test]
fn a_reading_that_never_reached_the_site_is_kept_nowhere_and_the_next_reader_fetches() {
    let env = goodfetch_test_support::TestEnv::new();
    with_copies(&env, |copies, state| async move {
        let location = location_of(1);
        let closed = NotRead("the Session `run.session-1` is closed".to_owned());
        let file = b"User-agent: *\nDisallow: /private/\n".to_vec();

        let first = copies
            .copy(&state, &location, async { Err(closed.clone()) })
            .await;
        let second = copies
            .copy(&state, &location, async {
                Ok(Answer::Response {
                    status: 200,
                    body: file.clone(),
                })
            })
            .await;
        let third = copies
            .copy(&state, &location, async {
                panic!("a third reader of the origin fetched it again")
            })
            .await;

        assert_eq!(
            (first, second, third.as_deref()),
            (
                Err(closed),
                Ok(Arc::new(Fetched::File(parse(&file)))),
                Ok(&Fetched::File(parse(&file)))
            )
        );
    });
}

/// Covers: story:6-62
#[test]
fn a_run_across_many_origins_holds_at_most_its_cache_in_memory_and_fetches_each_origin_once() {
    let env = goodfetch_test_support::TestEnv::new();
    with_copies(&env, |copies, state| async move {
        let origins = CACHED * 4;
        let fetches = std::cell::Cell::new(0);
        let fetch = |origin: usize| {
            let fetches = &fetches;
            async move {
                fetches.set(fetches.get() + 1);
                Ok(Answer::Response {
                    status: 200,
                    body: format!("User-agent: *\nDisallow: /{origin}/\n").into_bytes(),
                })
            }
        };
        let mut most_held = 0;

        // Every origin twice, the second time after every other origin was
        // read, so its copy left memory meanwhile.
        for _ in 0..2 {
            for origin in 0..origins {
                let copy = copies
                    .copy(&state, &location_of(origin), fetch(origin))
                    .await;
                assert_eq!(
                    copy,
                    Ok(Arc::new(Fetched::File(parse(
                        format!("User-agent: *\nDisallow: /{origin}/\n").as_bytes()
                    ))))
                );
                let in_memory = lock(&copies.memory);
                most_held = most_held.max(in_memory.cached.len() + in_memory.in_flight.len());
            }
        }

        assert_eq!((most_held, fetches.get()), (CACHED, origins));
    });
}

/// A pattern the generator writes: path segments of letters, a `*` or a
/// final `$` among them.
fn pattern() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(
            prop_oneof![Just("*".to_owned()), "[a-c]{1,3}", Just("/".to_owned())],
            1..6,
        ),
        any::<bool>(),
    )
        .prop_map(|(pieces, anchored)| {
            let mut pattern = format!("/{}", pieces.concat());
            if anchored {
                pattern.push('$');
            }
            pattern
        })
}

/// A target the generator writes over the same letters.
fn path() -> impl Strategy<Value = String> {
    "/[a-c/]{0,10}"
}

/// A reference matcher, by recursion over the pattern: slow, and written
/// apart from the engine's.
fn reference(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => true,
        Some((b'$', [])) => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len()).any(|at| reference(rest, &path[at..])),
        Some((octet, rest)) => path
            .split_first()
            .is_some_and(|(first, after)| first == octet && reference(rest, after)),
    }
}

proptest! {
    /// A file written from groups reads back as those groups.
    #[test]
    fn a_written_file_reads_back(
        groups in prop::collection::vec(
            (
                prop::collection::vec("[A-Za-z_-]{1,8}", 1..3),
                prop::collection::vec((any::<bool>(), pattern()), 0..4),
            ),
            1..4,
        )
    ) {
        let mut text = String::new();
        for (agents, rules) in &groups {
            for agent in agents {
                writeln!(text, "User-agent: {agent}").unwrap();
            }
            for (allow, pattern) in rules {
                writeln!(text, "{}: {pattern}", if *allow { "Allow" } else { "Disallow" }).unwrap();
            }
            // A group with no rule takes the next group's agents.
            if rules.is_empty() {
                text.push_str("Crawl-delay: 1\n");
            }
        }

        let file = parse(text.as_bytes());

        let read: Vec<(Vec<String>, Vec<String>)> = file
            .groups()
            .iter()
            .map(|group| (
                group.user_agents().to_vec(),
                group.rules().iter().map(ToString::to_string).collect(),
            ))
            .collect();
        let written: Vec<(Vec<String>, Vec<String>)> = groups
            .iter()
            .map(|(agents, rules)| (
                agents.clone(),
                rules
                    .iter()
                    .map(|(allow, pattern)| format!("{}: {pattern}", if *allow { "Allow" } else { "Disallow" }))
                    .collect(),
            ))
            .collect();
        prop_assert_eq!(read, written);
    }

    /// One rule matches a path exactly when the reference matcher says its
    /// pattern matches a prefix of the path (all of it, with a final `$`).
    #[test]
    fn one_rule_matches_as_the_reference_says(pattern in pattern(), path in path()) {
        let file = parse(format!("User-agent: *\nDisallow: {pattern}\n").as_bytes());
        let anchored = pattern.ends_with('$');
        let open = if anchored { pattern.clone() } else { format!("{pattern}*") };

        prop_assert_eq!(
            !allows(group(&file, "*").rules(), &path),
            path != "/robots.txt" && reference(open.as_bytes(), path.as_bytes())
        );
    }

    /// An Allow and a Disallow of one pattern allow every path.
    #[test]
    fn allow_wins_every_tie(pattern in pattern(), path in path(), allow_first in any::<bool>()) {
        let rules = if allow_first {
            format!("Allow: {pattern}\nDisallow: {pattern}")
        } else {
            format!("Disallow: {pattern}\nAllow: {pattern}")
        };
        let file = parse(format!("User-agent: *\n{rules}\n").as_bytes());

        prop_assert!(allows(group(&file, "*").rules(), &path));
    }

    /// A longer pattern wins over every shorter one that matches too,
    /// whatever their order.
    #[test]
    fn the_longest_match_wins(
        stem in "/[a-c]{1,4}",
        rest in "[a-c]{1,4}",
        tail in "[a-c/]{0,4}",
        longer_allows in any::<bool>(),
        longer_first in any::<bool>(),
    ) {
        let longer = format!("{stem}{rest}");
        let (short_kind, long_kind) = if longer_allows { ("Disallow", "Allow") } else { ("Allow", "Disallow") };
        let lines = [format!("{short_kind}: {stem}"), format!("{long_kind}: {longer}")];
        let rules = if longer_first { format!("{}\n{}", lines[1], lines[0]) } else { lines.join("\n") };
        let file = parse(format!("User-agent: *\n{rules}\n").as_bytes());

        prop_assert_eq!(allows(group(&file, "*").rules(), &format!("{longer}{tail}")), longer_allows);
    }
}

mod robots_txt_parser {
    goodfetch_test_support::fuzz_replay!(crate::robots_txt::fuzz_robots_txt, "fuzz/robots_txt");
}
