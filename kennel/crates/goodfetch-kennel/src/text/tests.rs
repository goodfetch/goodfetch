//! The Text function, table-driven: each case gives the text as it came
//! from the Daemon or a crawled site, and what a person is shown.

use rstest::rstest;

use super::shown;

#[rstest]
#[case::plain_text_is_kept("Fetching page 3 of 40", "Fetching page 3 of 40")]
#[case::a_line_feed_becomes_a_space("first\nsecond", "first second")]
#[case::runs_of_white_space_become_one_space("a\t\r\n  b", "a b")]
#[case::spaces_at_the_ends_are_trimmed("  padded  ", "padded")]
#[case::c0_controls_are_stripped("bell\u{7}null\u{0}escape\u{1b}[31m", "bellnullescape[31m")]
#[case::delete_and_c1_controls_are_stripped("a\u{7f}b\u{80}c\u{9b}d", "abcd")]
#[case::bidirectional_overrides_are_stripped("invoice\u{202e}fdp.exe", "invoicefdp.exe")]
#[case::bidirectional_isolates_and_marks_are_stripped(
    "\u{2066}a\u{2069}\u{2067}b\u{2068}c\u{200e}d\u{200f}e\u{61c}f",
    "abcdef"
)]
#[case::zero_width_characters_are_stripped("pay\u{200b}pal\u{200d}\u{feff}", "paypal")]
#[case::line_and_paragraph_separators_become_spaces("one\u{2028}two\u{2029}three", "one two three")]
#[case::html_is_shown_as_written(
    "<b>bold</b> <script>alert(1)</script> &amp;",
    "<b>bold</b> <script>alert(1)</script> &amp;"
)]
#[case::other_scripts_are_kept("Récolte 東京 Ünïcode", "Récolte 東京 Ünïcode")]
fn text_is_shown_as_plain_text_without_control_or_bidirectional_characters(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    assert_eq!(shown(raw, 200), expected);
}

#[rstest]
#[case::a_short_text_is_kept("twelve chars", 12, "twelve chars")]
#[case::a_long_text_ends_with_an_ellipsis("thirteen char", 12, "thirteen ch…")]
#[case::truncation_counts_characters_not_bytes("ééééé", 4, "ééé…")]
#[case::truncation_comes_after_stripping("a\u{202e}b\u{202e}c\u{202e}d", 4, "abcd")]
#[case::a_very_long_text_is_cut(&"x".repeat(100_000), 5, "xxxx…")]
fn text_is_truncated_to_the_characters_it_may_take(
    #[case] raw: &str,
    #[case] most: usize,
    #[case] expected: &str,
) {
    assert_eq!(shown(raw, most), expected);
}

#[rstest]
#[case::a_url_password(
    "fetching https://ann:hunter2@example.com/a",
    "fetching https://ann:••••@example.com/a"
)]
#[case::a_url_user_alone_is_kept("ftp://ann@example.com/", "ftp://ann@example.com/")]
#[case::a_token_query_parameter(
    "https://example.com/feed?token=abc123&page=2",
    "https://example.com/feed?token=••••&page=2"
)]
#[case::query_parameter_names_ignore_case(
    "https://example.com/?API_KEY=k1&Access_Token=t2&x=1",
    "https://example.com/?API_KEY=••••&Access_Token=••••&x=1"
)]
#[case::a_signed_url(
    "https://bucket.s3.amazonaws.com/o?X-Amz-Credential=AK%2F1&X-Amz-Signature=f00d",
    "https://bucket.s3.amazonaws.com/o?X-Amz-Credential=••••&X-Amz-Signature=••••"
)]
#[case::a_password_in_prose(
    "login failed with password=s3cret!",
    "login failed with password=••••"
)]
#[case::a_bearer_token(
    "sent Authorization: Bearer eyJhbGciOi.payload.sig",
    "sent Authorization: Bearer ••••"
)]
#[case::basic_credentials("authorization: basic YW5uOmh1bnRlcjI=", "authorization: basic ••••")]
#[case::a_word_that_only_looks_like_a_name_is_kept(
    "tokenizer=on keyboard=fr monkey=1",
    "tokenizer=on keyboard=fr monkey=1"
)]
fn secret_values_are_masked(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(shown(raw, 200), expected);
}

#[test]
fn a_secret_cut_by_the_truncation_never_shows_its_start() {
    let shown = shown("see https://example.com/?token=0123456789abcdef", 32);

    assert_eq!(shown, "see https://example.com/?token=…");
}
