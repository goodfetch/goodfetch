//! Every module's entries, in one list: what the emitter writes into the
//! catalog, and what each Product's catalog holds, by the Product mark of
//! where each entry is defined (ADRs 0095 and 0099).

use crate::catalog::{Entries, ErrorCodeDefinition, Operation, Product, SharedType, Yielding};
use crate::{
    audit, block, browser_cache, checkpoint, connection, daemon, dns, download, effect, egress,
    engine, events, executables, foxhound, governor, hostile_input, key_value_store, log,
    needs_you, ops, page, politeness, response_limits, run, sample, secrets, security, settings,
    site_preferences, storage, tls, usage, versions,
};

/// Every module that defines entries.
pub const MODULES: [&Entries; 53] = [
    &connection::ENTRIES,
    &log::ENTRIES,
    &daemon::ENTRIES,
    &ops::ENTRIES,
    &governor::ENTRIES,
    &storage::ENTRIES,
    &browser_cache::ENTRIES,
    &dns::ENTRIES,
    &egress::ENTRIES,
    &response_limits::ENTRIES,
    &hostile_input::ENTRIES,
    &tls::ENTRIES,
    &engine::ENTRIES,
    &events::ENTRIES,
    &needs_you::ENTRIES,
    &page::ENTRIES,
    &page::proxy::ENTRIES,
    &page::access_mode::ENTRIES,
    &page::signing_key::ENTRIES,
    &executables::ENTRIES,
    &run::ENTRIES,
    &run::completeness::ENTRIES,
    &checkpoint::ENTRIES,
    &key_value_store::ENTRIES,
    &effect::ENTRIES,
    &download::ENTRIES,
    &audit::ENTRIES,
    &secrets::ENTRIES,
    &security::ENTRIES,
    &sample::ENTRIES,
    &settings::ENTRIES,
    &settings::edit::ENTRIES,
    &usage::ENTRIES,
    &versions::ENTRIES,
    &politeness::ENTRIES,
    &site_preferences::ENTRIES,
    &block::ENTRIES,
    &foxhound::ENTRIES,
    &foxhound::crawl::ENTRIES,
    &foxhound::fetch::ENTRIES,
    &foxhound::frontier::ENTRIES,
    &foxhound::hook::ENTRIES,
    &foxhound::dataset::ENTRIES,
    &foxhound::dataset_view::ENTRIES,
    &foxhound::export::ENTRIES,
    &foxhound::outcomes::ENTRIES,
    &foxhound::rejections::ENTRIES,
    &foxhound::report::ENTRIES,
    &foxhound::session_pool::ENTRIES,
    &foxhound::settings::ENTRIES,
    &foxhound::sql_tab::ENTRIES,
    &foxhound::block::ENTRIES,
    &foxhound::usage::ENTRIES,
];

/// Every setting value that yields to another setting, which
/// `config explain` names in its `yields_to` (#2418).
pub const YIELDING: [&Yielding; 10] = [
    &foxhound::fetch::HTTP_FROM_YIELDS_TO_A_LOGIN,
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[0],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[1],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[2],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[3],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[4],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[5],
    &page::access_mode::ITEMS_YIELD_TO_THE_ACCESS_MODE[6],
    &page::access_mode::SIGNING_YIELDS_TO_THE_ACCESS_MODE[0],
    &page::access_mode::SIGNING_YIELDS_TO_THE_ACCESS_MODE[1],
];

/// The operations of `product`'s catalog: Collie's own for Collie, every
/// operation for Foxhound, which contains Collie.
pub fn operations_of(product: Product) -> impl Iterator<Item = &'static Operation> {
    MODULES
        .into_iter()
        .filter(move |entries| product.contains(Product::of_module(entries.defined_in)))
        .flat_map(|entries| entries.operations)
}

/// The Product that carries the value `name` of `shared_type`, such as the
/// Usage kind `requests`; none for a value neither Product knows, which a
/// newer engine wrote.
pub fn product_of_value(shared_type: SharedType, name: &str) -> Option<Product> {
    MODULES.into_iter().find_map(|entries| {
        entries
            .shared_type_values
            .iter()
            .any(|value| value.shared_type == shared_type && value.name == name)
            .then(|| Product::of_module(entries.defined_in))
    })
}

/// The Error code named `name`, whichever module defines it; none for a
/// code neither Product knows.
pub fn error_code(name: &str) -> Option<&'static ErrorCodeDefinition> {
    MODULES
        .into_iter()
        .flat_map(|entries| entries.error_codes)
        .find(|code| code.name == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use schemars::SchemaGenerator;

    use super::{MODULES, operations_of};
    use crate::catalog::{Product, ValueKind};

    /// Adds to `found` the path of each module under `folder`, the module
    /// `module`, that defines `pub const ENTRIES`, as `module_path!()`
    /// writes it without the crate's name.
    #[expect(
        clippy::disallowed_methods,
        reason = "a synchronous unit test reads the crate's own source; no async worker runs"
    )]
    fn modules_defining_entries(folder: &Path, module: &str, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let inner = if module.is_empty() {
                name.to_owned()
            } else {
                format!("{module}::{name}")
            };
            if path.is_dir() {
                modules_defining_entries(&path, &inner, found);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .any(|line| line.starts_with("pub const ENTRIES: Entries"))
            {
                found.insert(inner);
            }
        }
    }

    /// A module that defines `ENTRIES` and is not in `MODULES` would leave
    /// the catalog, and every SDK, without a word: each is listed, once.
    #[test]
    fn every_module_that_defines_entries_is_listed_once() {
        let mut defining = BTreeSet::new();
        modules_defining_entries(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            "",
            &mut defining,
        );
        let crate_name = concat!(env!("CARGO_CRATE_NAME"), "::");
        let listed: Vec<String> = MODULES
            .iter()
            .map(|entries| entries.defined_in.replacen(crate_name, "", 1))
            .collect();

        assert_eq!(
            (
                listed.iter().cloned().collect::<BTreeSet<_>>(),
                listed.len()
            ),
            (defining.clone(), defining.len())
        );
    }

    /// A call's own `timeout` sets one Duration setting for that call alone,
    /// which its `setting` keyword names: the setting the engine's refusal
    /// of `0` and an SDK's refusal before sending name in their `limit`
    /// (ADR 0080).
    ///
    /// Covers: story:11-42
    #[test]
    fn each_calls_own_timeout_names_the_duration_setting_it_sets() {
        let durations: BTreeSet<&str> = MODULES
            .iter()
            .flat_map(|entries| entries.settings)
            .filter(|setting| setting.kind == ValueKind::Duration)
            .map(|setting| setting.name)
            .collect();
        let mut generator = SchemaGenerator::default();
        let mut named = Vec::new();
        for operation in MODULES.iter().flat_map(|entries| entries.operations) {
            let reference = (operation.params)(&mut generator);
            let params = reference
                .get("$ref")
                .and_then(|reference| reference.as_str())
                .and_then(|reference| reference.rsplit('/').next())
                .and_then(|name| generator.definitions().get(name))
                .cloned()
                .unwrap_or_else(|| reference.to_value());
            if let Some(timeout) = params.pointer("/properties/timeout") {
                named.push((
                    operation.name,
                    timeout["setting"]
                        .as_str()
                        .filter(|setting| durations.contains(setting))
                        .map(str::to_owned),
                ));
            }
        }

        assert_eq!(
            named
                .iter()
                .find(|(name, _)| *name == "page.goto")
                .map(|(_, setting)| setting.as_deref()),
            Some(Some("timeout.navigation"))
        );
        assert_eq!(
            named
                .iter()
                .filter(|(_, setting)| setting.is_none())
                .collect::<Vec<_>>(),
            Vec::<&(&str, Option<String>)>::new()
        );
    }

    /// Collie's operations, in the catalog's order.
    const COLLIE: [&str; 91] = [
        "connection.set_log_level",
        "daemon.status",
        "daemon.stop",
        "daemon.health",
        "certs.list",
        "certs.accept",
        "certs.forget",
        "engine.info",
        "events.subscribe",
        "needs_you.list",
        "needs_you.dismiss",
        "needs_you.allow_consent_domain",
        "needs_you.remove_refetch_domain",
        "needs_you.deny_wall_domain",
        "run.new_page",
        "run.new_session",
        "session.new_page",
        "session.close",
        "page.goto",
        "page.reload",
        "page.go_back",
        "page.go_forward",
        "page.formats",
        "page.read",
        "page.body",
        "locator.click",
        "locator.fill",
        "locator.press",
        "page.route",
        "session.route",
        "route.continue",
        "route.fallback",
        "route.fulfill",
        "route.abort",
        "route.fetch",
        "page.on",
        "response.body",
        "page.wait_for_request_finished",
        "page.request",
        "page.on_dialog",
        "dialog.accept",
        "dialog.dismiss",
        "session.on",
        "session.pages",
        "page.close",
        "page.evaluate",
        "locator.evaluate_all",
        "page.add_init_script",
        "page.wait_for_url",
        "page.wait_for_request",
        "page.wait_for_response",
        "page.wait_for_load_state",
        "page.wait_for_function",
        "locator.wait_for_function",
        "locator.wait_for",
        "locator.wait_for_count",
        "signing_key.create",
        "signing_key.directory",
        "run.start",
        "run.finish",
        "run.stop",
        "run.abort",
        "run.usage",
        "run.set_progress_message",
        "run.discard",
        "run.progress",
        "run.report",
        "run.list",
        "run.checkpoint",
        "run.pause",
        "run.migrate",
        "run.resume",
        "key_value_store.get",
        "key_value_store.set",
        "key_value_store.get_bytes",
        "key_value_store.set_bytes",
        "key_value_store.delete",
        "key_value_store.keys",
        "effect.open",
        "effect.close",
        "download.save_as",
        "audit.export",
        "audit.verify",
        "sample.echo",
        "config.explain",
        "config.policy_read",
        "config.policy_write",
        "config.user_write",
        "usage.list",
        "usage.set",
        "usage.raise",
    ];

    #[test]
    fn foxhounds_catalog_holds_collies_and_its_own_operations() {
        let collie: Vec<&str> = operations_of(Product::Collie).map(|op| op.name).collect();
        let foxhound: Vec<&str> = operations_of(Product::Foxhound).map(|op| op.name).collect();

        assert_eq!(collie, COLLIE);
        assert_eq!(
            foxhound,
            [
                &COLLIE[..],
                &[
                    "sample.crawl",
                    "crawl.start",
                    "crawl.serve",
                    "run.seeds",
                    "run.add_seed",
                    "run.add_seeds",
                    "crawl_request.enqueue",
                    "crawl_request.links",
                    "crawl_request.enqueue_links",
                    "crawl_request.push",
                    "run.records",
                    "dataset.list",
                    "dataset.page",
                    "run.export",
                    "export.save_as",
                    "export.stream",
                    "run.outcomes",
                    "run.failed_requests",
                    "run.retry_failed",
                    "run.rejected_records",
                    "dataset.query",
                ],
            ]
            .concat()
        );
    }
}
