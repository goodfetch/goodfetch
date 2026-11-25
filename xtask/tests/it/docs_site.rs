//! `xtask docs-site render`: the docs site's source tree under
//! `target/docs-site/source/`, which Docusaurus builds (ADR 0097). Each
//! published page of `docs/` is rendered into the section of each Product
//! that carries it, with that Product's names; `docs/root/` holds the root's
//! pages, copied as written; Foxhound's section groups Collie's pages under
//! a heading that names Collie, and each such page says it is Collie's.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const SOURCE: &str = "target/docs-site/source";

fn rendered(workspace: &Workspace, relative: &str) -> String {
    workspace.read_text(&format!("{SOURCE}/{relative}"))
}

fn rendered_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("docs-site");
    let outcome = workspace.xtask(&["docs-site", "render"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    workspace
}

#[test]
fn render_writes_each_products_section_with_its_names() {
    let workspace = rendered_workspace();

    assert_eq!(
        rendered(&workspace, "docs/collie/index.md"),
        "---\ntitle: Home\ndescription: Where each Product's docs start.\n---\n\n# Home\n\n\
         Goodfetch Collie drives a browser.\n\nRun `goodfetch-collie --help`.\n"
    );
    assert_eq!(
        rendered(&workspace, "docs/foxhound/index.md"),
        "---\ntitle: Home\ndescription: Where each Product's docs start.\n---\n\n# Home\n\n\
         Goodfetch Foxhound crawls sites.\n\nRun `goodfetch-foxhound --help`.\n"
    );
    assert_eq!(
        rendered(&workspace, "docs/collie/install.md"),
        "---\ntitle: Install\n---\n\n# Install\n\nInstall `@goodfetch/collie`.\n"
    );
    assert_eq!(
        workspace.read(&format!("{SOURCE}/docs/foxhound/install.md")),
        None
    );
    assert_eq!(
        workspace.read(&format!("{SOURCE}/docs/collie/throughput.md")),
        None
    );
}

#[test]
fn render_marks_each_of_collies_pages_in_foxhounds_section_as_collies() {
    let workspace = rendered_workspace();

    assert_eq!(
        rendered(&workspace, "docs/foxhound/runs.md"),
        "---\ntitle: Runs\n---\n\n# Runs\n\n\
         :::note[Goodfetch Collie's page]\n\n\
         This page is Goodfetch Collie's. Foxhound contains Collie, so it is part of Foxhound's docs, \
         written with Foxhound's names.\n\n:::\n\n\
         Start with `goodfetch-foxhound run`. See [Throughput](throughput.md#steps) and \
         [the chooser](../index.md).\n"
    );
}

#[test]
fn render_points_a_link_to_a_page_the_section_lacks_at_the_section_that_has_it() {
    let workspace = rendered_workspace();

    assert_eq!(
        rendered(&workspace, "docs/collie/runs.md"),
        "---\ntitle: Runs\n---\n\n# Runs\n\n\
         Start with `goodfetch-collie run`. See [Throughput](../foxhound/throughput.md#steps) and \
         [the chooser](../index.md).\n"
    );
    assert_eq!(
        rendered(&workspace, "docs/foxhound/throughput.md"),
        "---\ntitle: Throughput\n---\n\n# Throughput\n\n## Steps\n\n\
         Read [Install](../collie/install.md) first.\n"
    );
}

#[test]
fn render_points_a_link_into_a_sections_folder_at_that_sections_rendering() {
    let workspace = Workspace::from_fixture("docs-site");
    workspace.append(
        "docs/index.md",
        "\nCompare [Foxhound's runs](foxhound/runs.md).\n",
    );

    let outcome = workspace.xtask(&["docs-site", "render"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        rendered(&workspace, "docs/collie/index.md")
            .ends_with("Compare [Foxhound's runs](../foxhound/runs.md).\n")
    );
    assert!(
        rendered(&workspace, "docs/foxhound/index.md")
            .ends_with("Compare [Foxhound's runs](runs.md).\n")
    );
}

#[test]
fn render_copies_the_root_pages_as_written_and_leaves_out_the_unpublished_folders() {
    let workspace = rendered_workspace();

    assert_eq!(
        rendered(&workspace, "docs/index.md"),
        workspace.read_text("docs/root/index.md")
    );
    let mut written: Vec<String> = walk(&workspace.path().join(SOURCE).join("docs"));
    written.sort();
    assert_eq!(
        written,
        [
            "collie/index.md",
            "collie/install.md",
            "collie/runs.md",
            "foxhound/index.md",
            "foxhound/runs.md",
            "foxhound/throughput.md",
            "index.md",
        ]
    );
}

#[test]
fn render_lists_each_sections_sidebar_with_collies_pages_under_collies_heading_in_foxhounds() {
    let workspace = rendered_workspace();

    let sidebars: Value = serde_json::from_str(&rendered(&workspace, "sidebars.json")).unwrap();

    assert_eq!(
        sidebars,
        json!({
            "root": ["index"],
            "collie": ["collie/index", "collie/install", "collie/runs"],
            "foxhound": [
                "foxhound/index",
                "foxhound/throughput",
                {"type": "category", "label": "Goodfetch Collie", "collapsible": false, "items": ["foxhound/runs"]},
            ],
        })
    );
}

#[test]
fn render_removes_what_an_earlier_render_wrote() {
    let workspace = rendered_workspace();
    workspace.remove("docs/install.md");
    workspace.edit(
        "docs/throughput.md",
        "Read [Install](./install.md) first.",
        "Read on.",
    );

    let outcome = workspace.xtask(&["docs-site", "render"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read(&format!("{SOURCE}/docs/collie/install.md")),
        None
    );
}

#[test]
fn render_fails_on_a_page_the_rendering_cannot_read_and_writes_nothing() {
    let workspace = Workspace::from_fixture("docs-site");
    workspace.append(
        "docs/runs.md",
        "{/* products: foxhound */}\nNever closed.\n",
    );

    let outcome = workspace.xtask(&["docs-site", "render"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("docs-samples", "docs/runs.md:12")]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(&format!("{SOURCE}/sidebars.json")), None);
}

/// Every file under `folder`, relative to it, with `/` between its parts.
fn walk(folder: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if entry.file_type().unwrap().is_dir() {
            files.extend(
                walk(&entry.path())
                    .into_iter()
                    .map(|file| format!("{name}/{file}")),
            );
        } else {
            files.push(name);
        }
    }
    files
}
