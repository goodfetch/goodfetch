use rstest::rstest;

use super::path_of;

#[rstest]
#[case::a_page("http://catalog.test:4100/catalog/1", Some("/catalog/1"))]
#[case::its_query_and_fragment_left_out("https://catalog.test/b?page=2#top", Some("/b"))]
#[case::an_origin("http://catalog.test:4100", Some("/"))]
#[case::an_origin_with_a_query("http://catalog.test?page=2", Some("/"))]
#[case::not_absolute("/catalog/1", None)]
fn a_urls_path_is_what_follows_its_authority(#[case] url: &str, #[case] path: Option<&str>) {
    assert_eq!(path_of(url), path);
}
