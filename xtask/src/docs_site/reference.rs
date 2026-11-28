//! Generated per-Product reference pages (ADRs 0050, 0095 and 0097; issue 150).
//!
//! Reads the authoritative `goodfetch_protocol::entries::MODULES` catalog and
//! emits, for each Product (`collie` and `foxhound`):
//! - `<product>/reference/index.md`: overview linking `TypeDoc`, Griffe, docs.rs,
//!   and `clap-markdown` API references;
//! - `<product>/reference/operations/<slug>.md`: one page per Operation carried
//!   by that Product, with TypeScript, Python, Rust, CLI, and MCP signatures
//!   where `Reach` applies and "Added in" / "Deprecated in" badges;
//! - `<product>/reference/settings/<slug>.md`: one page per Setting honoured by
//!   that Product (Foxhound-only settings appear only in Foxhound's section);
//! - `<product>/reference/metrics/<slug>.md`: one page per Metric (ADR 0050)
//!   with instrument, UCUM unit, and bounded labels.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use goodfetch_protocol::catalog::{
    Metric, Operation, Product as ProtocolProduct, Reach, Setting, Stability,
};
use goodfetch_protocol::entries::MODULES;
use serde_json::{Value, json};

use crate::products::Product;

/// Generated reference files and sidebar entries for one Product.
pub(crate) struct ProductReference {
    /// `(output_path_under_docs, markdown_text)` pairs.
    pub(crate) files: Vec<(String, String)>,
    /// Sidebar category JSON for this Product's `Reference` section.
    pub(crate) sidebar: Value,
}

/// Generates all reference pages and sidebar entries for `product`.
pub(crate) fn generate(product: Product) -> ProductReference {
    let key = product.key();
    let mut operations: BTreeMap<String, (ProtocolProduct, &'static Operation)> = BTreeMap::new();
    let mut settings: BTreeMap<String, (ProtocolProduct, &'static Setting)> = BTreeMap::new();
    let mut metrics: BTreeMap<String, (ProtocolProduct, &'static Metric)> = BTreeMap::new();

    for module in MODULES {
        let entry_product = ProtocolProduct::of_module(module.defined_in);
        if !carried_by(entry_product, product) {
            continue;
        }
        for op in module.operations {
            operations.insert(op.name.to_owned(), (entry_product, op));
        }
        for setting in module.settings {
            settings.insert(setting.name.to_owned(), (entry_product, setting));
        }
        for metric in module.metrics {
            metrics.insert(metric.name.to_owned(), (entry_product, metric));
        }
    }

    let mut files = Vec::with_capacity(1 + operations.len() + settings.len() + metrics.len());
    files.push((
        format!("{key}/reference/index.md"),
        render_index(product, operations.len(), settings.len(), metrics.len()),
    ));

    let mut op_ids = Vec::with_capacity(operations.len());
    for (owner, op) in operations.values() {
        let slug = slug_of(op.name);
        let id = format!("{key}/reference/operations/{slug}");
        files.push((format!("{id}.md"), render_operation(product, *owner, op)));
        op_ids.push(Value::String(id));
    }

    let mut setting_ids = Vec::with_capacity(settings.len());
    for (owner, setting) in settings.values() {
        let slug = slug_of(setting.name);
        let id = format!("{key}/reference/settings/{slug}");
        files.push((format!("{id}.md"), render_setting(*owner, setting)));
        setting_ids.push(Value::String(id));
    }

    let mut metric_ids = Vec::with_capacity(metrics.len());
    for (owner, metric) in metrics.values() {
        let slug = slug_of(metric.name);
        let id = format!("{key}/reference/metrics/{slug}");
        files.push((format!("{id}.md"), render_metric(*owner, metric)));
        metric_ids.push(Value::String(id));
    }

    let sidebar = json!({
        "type": "category",
        "label": "Reference",
        "link": { "type": "doc", "id": format!("{key}/reference/index") },
        "items": [
            {
                "type": "category",
                "label": "Operations",
                "items": op_ids,
            },
            {
                "type": "category",
                "label": "Settings",
                "items": setting_ids,
            },
            {
                "type": "category",
                "label": "Metrics",
                "items": metric_ids,
            },
        ],
    });

    ProductReference { files, sidebar }
}

fn carried_by(entry_product: ProtocolProduct, section_product: Product) -> bool {
    match section_product {
        Product::Collie => entry_product == ProtocolProduct::Collie,
        Product::Foxhound => true,
    }
}

fn slug_of(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn product_title(product: Product) -> &'static str {
    match product {
        Product::Collie => "Goodfetch Collie",
        Product::Foxhound => "Goodfetch Foxhound",
    }
}

fn owner_title(owner: ProtocolProduct) -> &'static str {
    match owner {
        ProtocolProduct::Collie => "Goodfetch Collie",
        ProtocolProduct::Foxhound => "Goodfetch Foxhound",
    }
}

fn stability_label(stability: Stability) -> &'static str {
    match stability {
        Stability::Stable => "Stable",
        Stability::Experimental => "Experimental",
    }
}

fn camel_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn render_index(product: Product, ops: usize, settings: usize, metrics: usize) -> String {
    let title = product_title(product);
    let key = product.key();
    let crate_name = match product {
        Product::Collie => "goodfetch-collie",
        Product::Foxhound => "goodfetch-foxhound",
    };
    let npm_pkg = match product {
        Product::Collie => "@goodfetch/collie",
        Product::Foxhound => "@goodfetch/foxhound",
    };
    let py_pkg = crate_name;
    format!(
        "---\ntitle: \"{title} Reference\"\n---\n\n\
         # {title} Reference\n\n\
         Generated from the committed Operation catalog and settings schema for **{title}**.\n\n\
         - **Operations:** {ops}\n\
         - **Settings:** {settings}\n\
         - **Metrics:** {metrics}\n\n\
         ## Language and CLI API References\n\n\
         - **TypeScript (TypeDoc):** `{npm_pkg}` (`https://goodfetch.dev/docs/{key}/api/typescript/`)\n\
         - **Python (Griffe):** `{py_pkg}` (`https://goodfetch.dev/docs/{key}/api/python/`)\n\
         - **Rust (docs.rs):** `{crate_name}` (`https://docs.rs/{crate_name}`)\n\
         - **CLI (`clap-markdown`):** `{crate_name}` (`https://goodfetch.dev/docs/{key}/api/cli/`)\n"
    )
}

fn render_operation(section: Product, owner: ProtocolProduct, op: &Operation) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "title: \"{}\"", op.name);
    let _ = writeln!(out, "---\n");
    let _ = writeln!(out, "# `{}`\n", op.name);
    let _ = writeln!(
        out,
        "**Product:** {} · **Stability:** {} · **Added in:** `{}`",
        owner_title(owner),
        stability_label(op.stability),
        op.arrived_in,
    );
    if let Some(dep) = op.deprecated {
        let note = dep
            .replacement
            .map_or_else(String::new, |rep| format!("Replacement: `{rep}`.\n"));
        let _ = writeln!(out, "\n:::warning[Deprecated in {}]\n{note}:::", dep.since);
    }
    let _ = writeln!(out, "\n{}\n", op.description);

    let (ns, member) = op.name.split_once('.').unwrap_or(("", op.name));
    let ts_method = camel_case(member);
    let cli_bin = match section {
        Product::Collie => "goodfetch-collie",
        Product::Foxhound => "goodfetch-foxhound",
    };
    let cli_sub = member.replace('_', "-");
    let mcp_tool = op.name.replace('.', "_");

    let _ = writeln!(out, "## Surfaces\n");
    let _ = writeln!(out, "### TypeScript\n");
    let _ = writeln!(out, "```text\n{ns}.{ts_method}(params)\n```\n");
    let _ = writeln!(out, "### Python\n");
    let _ = writeln!(out, "```text\n{ns}.{member}(...)\n```\n");
    let _ = writeln!(out, "### Rust\n");
    let _ = writeln!(out, "```text\n{ns}.{member}(...)\n```\n");

    if op.reach == Reach::AllSurfaces {
        let _ = writeln!(out, "### CLI\n");
        let _ = writeln!(out, "```text\n{cli_bin} {ns} {cli_sub}\n```\n");
        let _ = writeln!(out, "### MCP\n");
        let _ = writeln!(out, "```text\n{mcp_tool}\n```\n");
    }

    out
}

fn render_setting(owner: ProtocolProduct, setting: &Setting) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "title: \"{}\"", setting.name);
    let _ = writeln!(out, "---\n");
    let _ = writeln!(out, "# `{}`\n", setting.name);
    let _ = writeln!(
        out,
        "**Product:** {} · **Added in:** `{}` · **Clearable:** `{}` · **Fixed for run:** `{}`",
        owner_title(owner),
        setting.arrived_in,
        setting.clearable,
        setting.fixed_for_run,
    );
    if let Some(dep) = setting.deprecated {
        let note = dep
            .replacement
            .map_or_else(String::new, |rep| format!("Replacement: `{rep}`.\n"));
        let _ = writeln!(out, "\n:::warning[Deprecated in {}]\n{note}:::", dep.since);
    }
    let _ = writeln!(out, "\n{}\n", setting.description);
    let default_json = serde_json::to_string(&(setting.default)()).unwrap_or_default();
    let _ = writeln!(out, "## Default\n");
    let _ = writeln!(out, "```json\n{default_json}\n```\n");
    out
}

fn render_metric(owner: ProtocolProduct, metric: &Metric) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "title: \"goodfetch.{}\"", metric.name);
    let _ = writeln!(out, "---\n");
    let _ = writeln!(out, "# `goodfetch.{}`\n", metric.name);
    let _ = writeln!(
        out,
        "**Product:** {} · **Stability:** {} · **Added in:** `{}` · **Instrument:** `{:?}` · **Unit:** `{}`",
        owner_title(owner),
        stability_label(metric.stability),
        metric.arrived_in,
        metric.instrument,
        metric.unit,
    );
    let _ = writeln!(out, "\n{}\n", metric.description);
    if !metric.labels.is_empty() {
        let _ = writeln!(out, "## Labels\n");
        for label in metric.labels {
            let values = (label.values)().join("`, `");
            let _ = writeln!(out, "- `{}`: `{values}`", label.name);
        }
    }
    out
}
