use std::path::{Component, Path};

use rstest::rstest;
use serde_json::json;

use super::{
    ExpectedError, HeaderSource, HeaderValue, HostileTable, HostileTableError, HostileTables,
    LimitSetting, Os, PathHazard, PathName, ResourceLimit, Schedule, SchemaRef, SecretRedirect,
    ServedBy, TableGap, TableGaps,
};

fn table<I: serde::de::DeserializeOwned>(text: &str) -> HostileTable<I> {
    HostileTable::parse("test", text).unwrap()
}

/// Tables of one case each but the headers', whose cases `headers` gives.
fn tables(headers: &str) -> HostileTables {
    HostileTables {
        header_values: table(headers),
        secret_redirects: table(
            r#"
            [[cases]]
            name = "header_across_a_found_redirect"
            input = { placement = "header", bound_to = ["login.test"], url = "https://login.test/api", status = 302, location = "https://other.test/api" }
            expected = { code = "secret_domain_not_allowed" }
            served_by = { missing_page = "a login page that redirects to another domain" }
            "#,
        ),
        paths: table(
            r#"
            [[cases]]
            name = "parent_traversal"
            input = { targets = ["export_name"], name = "../../escape.csv", os = "every", hazard = "parent_traversal" }
            expected = { code = "path_unsafe" }
            served_by = "program_input"
            "#,
        ),
        schema_refs: table(
            r#"
            [[cases]]
            name = "url"
            input = { uses = ["extraction_schema"], keyword = "$ref", reference = "https://schemas.test/record.json" }
            expected = { code = "schema_ref_not_allowed" }
            served_by = "program_input"
            "#,
        ),
        limits: table(
            r#"
            [[cases]]
            name = "gzip_bomb"
            input = { kind = "decompression_bomb", content_encodings = ["gzip"] }
            expected = { code = "response_limit_exceeded", setting = "network.max_decompression_ratio" }
            served_by = { waiting_on_owner = "a gzip decompression bomb page" }
            "#,
        ),
    }
}

const NUL_HEAD: &str = r#"
    [[cases]]
    name = "response_head_with_nul"
    input = { source = "response", name = "X-Note", value = "one\u0000two" }
    expected = { code = "navigation_failed", network_error = "net::ERR_INVALID_HTTP_RESPONSE" }
    served_by = { page = "headers/5" }
"#;

#[test]
fn a_case_reads_its_input_expected_error_and_page() {
    let table: HostileTable<HeaderValue> = table(NUL_HEAD);

    let case = &table.cases[0];

    assert_eq!(
        (
            case.name.as_str(),
            &case.input,
            &case.expected,
            &case.served_by
        ),
        (
            "response_head_with_nul",
            &HeaderValue {
                source: HeaderSource::Response,
                name: "X-Note".to_owned(),
                value: "one\0two".to_owned(),
            },
            &ExpectedError::NavigationFailed {
                network_error: Some("net::ERR_INVALID_HTTP_RESPONSE".to_owned())
            },
            &ServedBy::Page("headers/5".to_owned())
        )
    );
}

#[test]
fn a_case_with_no_expected_error_fails_the_table_check() {
    let parsed = HostileTable::<HeaderValue>::parse(
        "header_values",
        r#"
        [[cases]]
        name = "recipe_header_with_crlf"
        input = { source = "recipe", name = "X-Note", value = "a\r\nX-Injected: yes" }
        served_by = "program_input"
        "#,
    );

    assert!(
        matches!(
            parsed,
            Err(HostileTableError::Malformed {
                family: "header_values",
                ..
            })
        ),
        "{parsed:?}"
    );
}

#[test]
fn a_case_of_an_unknown_error_code_fails_the_table_check() {
    let parsed = HostileTable::<HeaderValue>::parse(
        "header_values",
        r#"
        [[cases]]
        name = "recipe_header_with_crlf"
        input = { source = "recipe", name = "X-Note", value = "a\r\nX-Injected: yes" }
        expected = { code = "something_failed" }
        served_by = "program_input"
        "#,
    );

    assert!(
        matches!(parsed, Err(HostileTableError::Malformed { .. })),
        "{parsed:?}"
    );
}

#[test]
fn a_limit_case_names_the_setting_its_error_names() {
    let table: HostileTable<ResourceLimit> = table(
        r#"
        [[cases]]
        name = "deep_response_json"
        input = { kind = "deep_nesting", channel = "response_json", depth = 129 }
        expected = { code = "response_limit_exceeded", setting = "network.max_json_depth" }
        served_by = { waiting_on_owner = "a page whose JSON nests 129 deep" }
        "#,
    );

    assert_eq!(
        table.cases[0].expected,
        ExpectedError::ResponseLimitExceeded {
            setting: LimitSetting::MaxJsonDepth
        }
    );
}

#[test]
fn check_passes_tables_whose_pages_are_proving_ground_tasks() {
    let tables = tables(NUL_HEAD);

    let checked = tables.check(&["headers/4", "headers/5"]);

    assert_eq!(checked, Ok(()));
}

#[test]
fn check_reports_every_duplicate_name_and_unknown_page() {
    let tables = tables(
        r#"
        [[cases]]
        name = "response_head_with_nul"
        input = { source = "response", name = "X-Note", value = "one\u0000two" }
        expected = { code = "navigation_failed" }
        served_by = { page = "headers/50" }

        [[cases]]
        name = "response_head_with_nul"
        input = { source = "response", name = "X-Note", value = "one\rtwo" }
        expected = { code = "navigation_failed" }
        served_by = { page = "headers/6" }
        "#,
    );

    let checked = tables.check(&["headers/5", "headers/6"]);

    assert_eq!(
        checked,
        Err(TableGaps {
            gaps: vec![
                TableGap::UnknownPage {
                    family: "header_values",
                    case: "response_head_with_nul".to_owned(),
                    page: "headers/50".to_owned(),
                },
                TableGap::DuplicateName {
                    family: "header_values",
                    case: "response_head_with_nul".to_owned(),
                },
            ]
        })
    );
}

#[test]
fn pages_lists_every_case_that_names_a_page() {
    let tables = tables(NUL_HEAD);

    let pages = tables.pages();

    assert_eq!(
        pages,
        vec![("header_values", "response_head_with_nul", "headers/5")]
    );
}

#[rstest]
#[case::every_on_linux(Schedule::Every, Os::Linux, true)]
#[case::every_on_macos(Schedule::Every, Os::Macos, true)]
#[case::every_on_windows(Schedule::Every, Os::Windows, true)]
#[case::unix_on_linux(Schedule::Unix, Os::Linux, true)]
#[case::unix_on_macos(Schedule::Unix, Os::Macos, true)]
#[case::unix_not_on_windows(Schedule::Unix, Os::Windows, false)]
#[case::linux_not_on_macos(Schedule::Linux, Os::Macos, false)]
#[case::macos_on_macos(Schedule::Macos, Os::Macos, true)]
#[case::macos_not_on_linux(Schedule::Macos, Os::Linux, false)]
#[case::windows_on_windows(Schedule::Windows, Os::Windows, true)]
#[case::windows_not_on_linux(Schedule::Windows, Os::Linux, false)]
fn a_schedule_runs_on_its_oses(#[case] schedule: Schedule, #[case] os: Os, #[case] runs: bool) {
    assert_eq!(schedule.includes(os), runs);
}

#[test]
fn a_path_table_keeps_the_cases_of_the_os_asked_for() {
    let table: HostileTable<PathName> = table(
        r#"
        [[cases]]
        name = "parent_traversal"
        input = { targets = ["export_name"], name = "../escape.csv", os = "every", hazard = "parent_traversal" }
        expected = { code = "path_unsafe" }
        served_by = "program_input"

        [[cases]]
        name = "unc"
        input = { targets = ["export_name"], name = '\\server\share\escape.csv', os = "windows", hazard = "unc" }
        expected = { code = "path_unsafe" }
        served_by = "program_input"
        "#,
    );

    let on = |os| {
        table
            .cases_on(os)
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        (on(Os::Linux), on(Os::Windows)),
        (vec!["parent_traversal"], vec!["parent_traversal", "unc"])
    );
}

#[cfg(target_os = "linux")]
#[test]
fn the_current_os_is_linux_on_linux() {
    assert_eq!(Os::current(), Os::Linux);
}

#[cfg(target_os = "macos")]
#[test]
fn the_current_os_is_macos_on_macos() {
    assert_eq!(Os::current(), Os::Macos);
}

#[cfg(windows)]
#[test]
fn the_current_os_is_windows_on_windows() {
    assert_eq!(Os::current(), Os::Windows);
}

#[test]
fn a_schema_ref_case_gives_a_schema_holding_its_reference() {
    let reference = SchemaRef {
        uses: Vec::new(),
        keyword: super::RefKeyword::DynamicRef,
        reference: "file:///etc/passwd".to_owned(),
    };

    let schema = reference.schema();

    assert_eq!(
        schema,
        json!({
            "type": "object",
            "properties": { "value": { "$dynamicRef": "file:///etc/passwd" } }
        })
    );
}

#[test]
fn the_committed_tables_parse_with_unique_names() {
    let tables = HostileTables::committed().unwrap();

    let duplicates = (
        tables.header_values.duplicate_names(),
        tables.secret_redirects.duplicate_names(),
        tables.paths.duplicate_names(),
        tables.schema_refs.duplicate_names(),
        tables.limits.duplicate_names(),
    );

    assert_eq!(
        duplicates,
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
    );
}

#[test]
fn the_committed_tables_name_the_proving_grounds_hostile_pages() {
    let tables = HostileTables::committed().unwrap();

    let pages = tables.pages();

    assert_eq!(
        pages,
        vec![
            ("header_values", "response_head_with_nul", "headers/5"),
            ("header_values", "response_head_with_a_bare_cr", "headers/6"),
            ("limits", "gzip_bomb", "bombs/1"),
            ("limits", "deflate_bomb", "bombs/2"),
            ("limits", "brotli_bomb", "bombs/3"),
            ("limits", "zstd_bomb", "bombs/4"),
            ("limits", "stacked_codings_bomb", "bombs/5"),
            ("limits", "body_past_64_mib", "body-limits/1"),
            ("limits", "coded_body_past_256_mib", "body-limits/6"),
            ("limits", "endless_chunked_body", "body-limits/2"),
            ("limits", "endless_close_delimited_body", "body-limits/3"),
            ("limits", "response_json_past_128_deep", "body-limits/4"),
            ("limits", "response_head_past_256_kib", "headers/1"),
        ]
    );
}

#[test]
fn every_committed_family_has_cases() {
    let tables = HostileTables::committed().unwrap();

    let empty: Vec<&str> = [
        ("header_values", tables.header_values.cases.is_empty()),
        ("secret_redirects", tables.secret_redirects.cases.is_empty()),
        ("paths", tables.paths.cases.is_empty()),
        ("schema_refs", tables.schema_refs.cases.is_empty()),
        ("limits", tables.limits.cases.is_empty()),
    ]
    .into_iter()
    .filter_map(|(family, empty)| empty.then_some(family))
    .collect();

    assert_eq!(empty, Vec::<&str>::new());
}

/// The hazards of the committed path cases that run on `os`.
fn hazards_on(os: Os) -> Vec<PathHazard> {
    let tables = HostileTables::committed().unwrap();
    let mut hazards: Vec<PathHazard> = Vec::new();
    for case in tables.paths.cases_on(os) {
        if !hazards.contains(&case.input.hazard) {
            hazards.push(case.input.hazard);
        }
    }
    hazards
}

#[test]
fn the_committed_path_cases_on_windows_cover_its_separators_and_reserved_names() {
    let hazards = hazards_on(Os::Windows);

    let missing: Vec<PathHazard> = [
        PathHazard::ParentTraversal,
        PathHazard::Absolute,
        PathHazard::Drive,
        PathHazard::Verbatim,
        PathHazard::DeviceNamespace,
        PathHazard::Unc,
        PathHazard::ReservedName,
        PathHazard::TrailingDotOrSpace,
        PathHazard::AlternateDataStream,
        PathHazard::ForbiddenCharacter,
        PathHazard::ControlCharacter,
        PathHazard::Overlong,
        PathHazard::DotName,
        PathHazard::ArchiveLink,
    ]
    .into_iter()
    .filter(|hazard| !hazards.contains(hazard))
    .collect();

    assert_eq!(missing, Vec::new());
}

#[rstest]
#[case::linux(Os::Linux)]
#[case::macos(Os::Macos)]
fn the_committed_path_cases_on_unix_cover_its_separator_and_names(#[case] os: Os) {
    let hazards = hazards_on(os);

    let missing: Vec<PathHazard> = [
        PathHazard::ParentTraversal,
        PathHazard::Absolute,
        PathHazard::ControlCharacter,
        PathHazard::Overlong,
        PathHazard::DotName,
        PathHazard::ArchiveLink,
    ]
    .into_iter()
    .filter(|hazard| !hazards.contains(hazard))
    .collect();

    assert_eq!(missing, Vec::new());
}

#[test]
fn the_committed_path_cases_on_macos_cover_apfs_normalisation() {
    let hazards = hazards_on(Os::Macos);

    assert!(hazards.contains(&PathHazard::Normalisation), "{hazards:?}");
}

/// Whether `name`, joined to a folder as a naive writer joins it with this
/// OS's path rules, names a place outside that folder: a root, a drive or
/// a UNC prefix replaces the folder, and `..` climbs above it.
fn escapes(name: &str) -> bool {
    let mut depth = 0_usize;
    for component in Path::new(name).components() {
        match component {
            Component::RootDir | Component::Prefix(_) => return true,
            Component::ParentDir if depth == 0 => return true,
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
        }
    }
    false
}

#[rstest]
#[case::plain_name("report.csv")]
#[case::name_in_a_subfolder("shots/front.png")]
#[case::climb_back_inside("shots/../front.png")]
fn a_name_that_stays_inside_does_not_escape_a_naive_join(#[case] name: &str) {
    assert!(!escapes(name));
}

#[test]
fn the_escaping_path_cases_scheduled_here_leave_the_folder_a_naive_join_writes_under() {
    let tables = HostileTables::committed().unwrap();

    let kept_in: Vec<&str> = tables
        .paths
        .cases_here()
        .filter(|case| {
            matches!(
                case.input.hazard,
                PathHazard::ParentTraversal
                    | PathHazard::Absolute
                    | PathHazard::Drive
                    | PathHazard::Verbatim
                    | PathHazard::DeviceNamespace
                    | PathHazard::Unc
            )
        })
        .filter(|case| !escapes(&case.input.name))
        .map(|case| case.name.as_str())
        .collect();

    assert_eq!(kept_in, Vec::<&str>::new());
}

#[test]
fn the_committed_secret_redirects_leave_their_binding() {
    let tables = HostileTables::committed().unwrap();

    let inside: Vec<&str> = tables
        .secret_redirects
        .cases
        .iter()
        .filter(|case| {
            let SecretRedirect {
                bound_to, location, ..
            } = &case.input;
            bound_to
                .iter()
                .any(|domain| host(location).ends_with(domain.as_str()))
        })
        .map(|case| case.name.as_str())
        .collect();

    assert_eq!(inside, Vec::<&str>::new());
}

/// The host of an absolute `http` or `https` URL, after any userinfo.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    host_port.split(':').next().unwrap_or(host_port)
}
