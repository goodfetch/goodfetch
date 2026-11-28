//! Embedded Product docs resources (`goodfetch://docs/<page>` and
//! `goodfetch://docs/llms.txt`) and rendered agent skills (`skills/list`,
//! `skills/get`, and `resources/read`) served over MCP (ADRs 0070, 0097;
//! ticket #1371).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use goodfetch_protocol::catalog::Product;
use rmcp::model::{
    MetaObject, ReadResourceResult, Resource, ResourceContents, Skill, SkillFrontmatter,
    SkillResource, SkillResources, sha256_digest,
};
use serde_json::Value;

/// URI of the generated Product documentation index resource.
pub const LLMS_TXT_URI: &str = "goodfetch://docs/llms.txt";

/// URI prefix of embedded Product documentation pages.
pub const DOCS_URI_PREFIX: &str = "goodfetch://docs/";

/// URI prefix of embedded Product skill files.
pub const SKILLS_URI_PREFIX: &str = "goodfetch://skills/";

/// `_meta` key on each [`Skill`] carrying the `skill install` tree digest
/// (`sha256:<hex>` over sorted `<rel_path>\0<file_sha256_hex>\n`).
pub const SKILL_TREE_DIGEST_META_KEY: &str = "goodfetch_digest";

/// One embedded documentation page belonging to a Product.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedDoc {
    /// Relative page name, e.g. `"runs.md"`.
    pub page: String,
    /// Human-readable page title (e.g. from its H1 heading).
    pub title: String,
    /// Rendered markdown contents.
    pub markdown: String,
}

/// One file of a rendered skill embedded in an executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedSkillFile {
    /// `/`-separated path starting with the skill's folder name, e.g.
    /// `"goodfetch-collie/SKILL.md"`.
    pub path: String,
    /// Exact rendered file bytes.
    pub contents: Vec<u8>,
}

/// A rendered skill prepared for `skills/list`, `skills/get` and `resources/read`.
#[derive(Clone, Debug)]
struct ServedSkill {
    skill: Skill,
    files: BTreeMap<String, Vec<u8>>,
}

/// Embedded Product documentation and rendered skills served by [`crate::server::McpServer`].
#[derive(Clone, Debug, Default)]
pub struct ResourceStore {
    docs: BTreeMap<String, EmbeddedDoc>,
    llms_txt: Option<String>,
    skills: BTreeMap<String, ServedSkill>,
}

impl ResourceStore {
    /// Builds a [`ResourceStore`] from a Product's embedded docs pages and
    /// rendered skill files.
    #[must_use]
    pub fn from_embedded(
        product: Product,
        version: &str,
        docs: Vec<EmbeddedDoc>,
        skill_files: Vec<EmbeddedSkillFile>,
    ) -> Self {
        let mut doc_map = BTreeMap::new();
        for doc in docs {
            let uri = format!("{DOCS_URI_PREFIX}{}", doc.page.trim_start_matches('/'));
            doc_map.insert(uri, doc);
        }

        let llms_txt = Some(render_llms_txt(product, version, &doc_map));

        let mut grouped: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
        for file in skill_files {
            if let Some((skill_name, rel_path)) = file.path.split_once('/') {
                grouped
                    .entry(skill_name.to_owned())
                    .or_default()
                    .insert(rel_path.to_owned(), file.contents);
            }
        }

        let mut skills = BTreeMap::new();
        for (skill_name, rel_files) in grouped {
            let Some(skill_md_bytes) = rel_files.get("SKILL.md") else {
                continue;
            };
            let skill_uri = format!("{SKILLS_URI_PREFIX}{skill_name}/SKILL.md");
            let frontmatter = parse_frontmatter(&skill_name, skill_md_bytes);

            let tree_digest = skill_tree_digest(
                rel_files
                    .iter()
                    .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
            );

            let mut manifest_files = Vec::with_capacity(rel_files.len());
            let mut uri_files = BTreeMap::new();
            for (rel_path, bytes) in rel_files {
                let file_uri = format!("{SKILLS_URI_PREFIX}{skill_name}/{rel_path}");
                manifest_files.push(SkillResource::from_bytes(&file_uri, &bytes));
                uri_files.insert(file_uri, bytes);
            }

            let mut meta = MetaObject::new();
            meta.insert(
                SKILL_TREE_DIGEST_META_KEY.to_owned(),
                Value::String(tree_digest),
            );
            let skill = Skill::new(
                skill_uri.clone(),
                frontmatter,
                SkillResources::Files(manifest_files),
            )
            .with_meta(meta);

            skills.insert(
                skill_uri,
                ServedSkill {
                    skill,
                    files: uri_files,
                },
            );
        }

        Self {
            docs: doc_map,
            llms_txt,
            skills,
        }
    }

    /// Whether this store has any docs or skills to serve.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty() && self.llms_txt.is_none() && self.skills.is_empty()
    }

    /// Lists every documentation resource (`goodfetch://docs/llms.txt` and each
    /// `goodfetch://docs/<page>`) plus every skill file resource.
    #[must_use]
    pub fn list_resources(&self) -> Vec<Resource> {
        let mut resources = Vec::new();
        if let Some(llms) = &self.llms_txt {
            resources.push(
                Resource::new(LLMS_TXT_URI, "llms.txt")
                    .with_title("Product documentation index")
                    .with_description("Index of embedded Goodfetch product documentation.")
                    .with_mime_type("text/markdown")
                    .with_size(llms.len() as u64),
            );
        }
        for (uri, doc) in &self.docs {
            resources.push(
                Resource::new(uri.clone(), doc.page.clone())
                    .with_title(doc.title.clone())
                    .with_mime_type("text/markdown")
                    .with_size(doc.markdown.len() as u64),
            );
        }
        for served in self.skills.values() {
            for (uri, bytes) in &served.files {
                let is_skill_md = uri == &served.skill.uri;
                let name = if is_skill_md {
                    served.skill.frontmatter.name.clone()
                } else {
                    uri.strip_prefix(SKILLS_URI_PREFIX)
                        .unwrap_or(uri.as_str())
                        .to_owned()
                };
                let mut resource = Resource::new(uri.clone(), name)
                    .with_mime_type(mime_type_for_path(uri))
                    .with_size(bytes.len() as u64);
                if is_skill_md && !served.skill.frontmatter.description.is_empty() {
                    resource =
                        resource.with_description(served.skill.frontmatter.description.clone());
                }
                resources.push(resource);
            }
        }
        resources
    }

    /// Reads one embedded documentation page, `llms.txt`, or skill file by URI.
    #[must_use]
    pub fn read_resource(&self, uri: &str) -> Option<ReadResourceResult> {
        if uri == LLMS_TXT_URI
            && let Some(llms) = &self.llms_txt
        {
            return Some(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri: LLMS_TXT_URI.to_owned(),
                    mime_type: Some("text/markdown".to_owned()),
                    text: llms.clone(),
                    meta: None,
                },
            ]));
        }
        if let Some(doc) = self.docs.get(uri) {
            return Some(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri: uri.to_owned(),
                    mime_type: Some("text/markdown".to_owned()),
                    text: doc.markdown.clone(),
                    meta: None,
                },
            ]));
        }
        for served in self.skills.values() {
            if let Some(bytes) = served.files.get(uri) {
                let text = String::from_utf8_lossy(bytes).into_owned();
                return Some(ReadResourceResult::new(vec![
                    ResourceContents::TextResourceContents {
                        uri: uri.to_owned(),
                        mime_type: Some(mime_type_for_path(uri).to_owned()),
                        text,
                        meta: None,
                    },
                ]));
            }
        }
        None
    }

    /// Lists all rendered skills served by `skills/list`.
    #[must_use]
    pub fn list_skills(&self) -> Vec<Skill> {
        self.skills
            .values()
            .map(|served| served.skill.clone())
            .collect()
    }

    /// Looks up one rendered skill by its `SKILL.md` resource URI for `skills/get`.
    #[must_use]
    pub fn get_skill(&self, uri: &str) -> Option<Skill> {
        self.skills.get(uri).map(|served| served.skill.clone())
    }
}

/// Computes the canonical `skill install` tree digest (`sha256:<hex>`) over
/// sorted `(relative_path, bytes)` entries: `<path>\0<sha256_hex>\n`.
#[must_use]
pub fn skill_tree_digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut sorted: Vec<_> = files.collect();
    sorted.sort_by_key(|(path, _)| *path);
    let mut tree = String::new();
    for (path, bytes) in sorted {
        let digest = sha256_digest(bytes);
        let hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
        let _ = writeln!(tree, "{path}\0{hex}");
    }
    sha256_digest(tree.as_bytes())
}

fn render_llms_txt(
    product: Product,
    version: &str,
    docs: &BTreeMap<String, EmbeddedDoc>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Goodfetch {} {version}", product.name());
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Embedded product documentation for `{}` {version}.",
        product.executable_name()
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Pages");
    let _ = writeln!(out);
    for (uri, doc) in docs {
        let _ = writeln!(out, "- [{}]({uri})", doc.title);
    }
    out
}

fn parse_frontmatter(default_name: &str, bytes: &[u8]) -> SkillFrontmatter {
    let text = String::from_utf8_lossy(bytes);
    let mut name = default_name.to_owned();
    let mut description = String::new();
    if let Some(rest) = text.strip_prefix("---\n")
        && let Some((header, _)) = rest.split_once("\n---")
    {
        for line in header.lines() {
            if let Some(val) = line.strip_prefix("name:") {
                let cleaned = unquote_yaml(val);
                if !cleaned.is_empty() {
                    name = cleaned;
                }
            } else if let Some(val) = line.strip_prefix("description:") {
                description = unquote_yaml(val);
            }
        }
    }
    SkillFrontmatter::new(name, description)
}

fn unquote_yaml(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn mime_type_for_path(uri: &str) -> &'static str {
    let ext = std::path::Path::new(uri).extension();
    if ext.is_some_and(|e| e.eq_ignore_ascii_case("md")) {
        "text/markdown"
    } else if ext.is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        "application/json"
    } else {
        "text/plain"
    }
}
