//! `xtask compat refusals` (ADR 0066): the refusal cases of every format the
//! bundle registers, generated from each rule's table so that a registered
//! format cannot miss one, and written one file per case under
//! [`FOLDER`]`/<format>/`, which `xtask codegen` writes too and `--check`
//! holds to the registry.
//!
//! - Each rule fabricates a newer minor, an older major (from major 1 on)
//!   and a newer major; during `0.y` previews, where each `0.y` minor counts
//!   as a major (ADR 0030), the next and the previous preview instead,
//!   except in the Usage ledger family. The shared stores' rule also marks
//!   the store with a lowest minor an engine meets and with a newer one
//!   (ADR 0098).
//! - A protocol's cases are fabricated `hello`s, from each who opens it:
//!   - a Client's carry the rule's versions to the smallest Product's
//!     engine, and the Product check of every pair of Products, which comes
//!     before the version check: each Product contains every one the bundle
//!     lists before it (ADRs 0084 and 0095);
//!   - a Browser host's meet no Product check and settle on the lower minor,
//!     only another major refused, and a Session needing more than an older
//!     Browser host's minor is refused with `browser_host_outdated` (ADR
//!     0096);
//!   - a Group member is refused by a group of the other Product (ADR 0095).

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::formats::{Compatibility, HelloFrom, Rule};
use goodfetch_protocol::refusals::{FabricatedHello, RefusalCase, Reply};
use goodfetch_protocol::releases::FormatName;
use serde::Deserialize;
use serde_json::Value;

/// Where the cases are written, one folder per format.
pub(crate) const FOLDER: &str = "crates/goodfetch-protocol/generated/refusals";

/// The refusal of a `hello` of a version the engine does not accept (ADRs
/// 0024 and 0066).
const VERSION_REFUSED: &str = "protocol_version_unsupported";

/// The refusal of a `hello` that needs a Product the engine lacks, or of a
/// Group member of the other Product (ADR 0095).
const PRODUCT_REFUSED: &str = "not_in_product";

/// The refusal of what a Session needs beyond an older Browser host's minor
/// (ADR 0096).
const BROWSER_HOST_OUTDATED: &str = "browser_host_outdated";

#[derive(Deserialize)]
struct Bundle {
    #[serde(default)]
    formats: Vec<Entry>,
    #[serde(rename = "$defs", default)]
    definitions: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Entry {
    name: FormatName,
    version: ProtocolVersion,
    rule: Rule,
    #[serde(default)]
    hello_from: Vec<HelloFrom>,
}

/// A version a rule's table fabricates, with what the table gives.
struct Fabricated {
    name: &'static str,
    found: ProtocolVersion,
    lowest_minor: Option<u16>,
    engine: Option<ProtocolVersion>,
    compatibility: Compatibility,
}

impl Fabricated {
    fn of_format(name: &'static str, found: ProtocolVersion, compatibility: Compatibility) -> Self {
        Self {
            name,
            found,
            lowest_minor: None,
            engine: None,
            compatibility,
        }
    }
}

/// The Product version, major and minor, of the engine that opens a shared
/// store in its cases: a minor of a released major, since during `0.y`
/// previews a line's only minor is 0 (ADR 0030).
const STORE_ENGINE: ProtocolVersion = ProtocolVersion { major: 1, minor: 1 };

/// Every case file of the bundle's registry, by its path from the root.
pub(crate) fn files(bundle: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let bundle: Bundle = serde_json::from_slice(bundle).context("reading the format registry")?;
    let mut files = BTreeMap::new();
    for format in &bundle.formats {
        let cases = cases(format, &bundle.definitions)
            .with_context(|| format!("generating the refusal cases of `{}`", format.name))?;
        for (name, case) in cases {
            let mut text = serde_json::to_string_pretty(&case)?;
            text.push('\n');
            files.insert(
                format!("{FOLDER}/{}/{name}.json", format.name),
                text.into_bytes(),
            );
        }
    }
    Ok(files)
}

fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

/// The versions each rule's table fabricates around `current`. During
/// `0.y` previews each `0.y` minor counts as a major (ADR 0030) for every
/// rule but the Usage ledger family's, so a preview reader's neighbours are
/// other majors. A shared store's cases also hold an engine to the lowest
/// minor the store records (ADR 0098).
fn table(rule: Rule, current: ProtocolVersion) -> Result<Vec<Fabricated>> {
    let next = |number: u16| {
        number
            .checked_add(1)
            .context("the version has no number after it")
    };
    let newer_minor = version(current.major, next(current.minor)?);
    let preview = current.major == 0 && rule != Rule::FrozenMajor;
    let mut table = Vec::new();
    if preview {
        table.push(Fabricated::of_format(
            "newer-preview",
            newer_minor,
            Compatibility::NewerMajor,
        ));
        if let Some(older) = current.minor.checked_sub(1) {
            table.push(Fabricated::of_format(
                "older-preview",
                version(0, older),
                Compatibility::OlderMajor,
            ));
        }
    } else {
        let newer_minor_reads = match rule {
            Rule::Default | Rule::NewestReadable => Compatibility::NewerMinor,
            Rule::FrozenMajor | Rule::LowerMinor | Rule::SharedStore => Compatibility::Compatible,
        };
        table.push(Fabricated::of_format(
            "newer-minor",
            newer_minor,
            newer_minor_reads,
        ));
        if let Some(older) = current.major.checked_sub(1) {
            table.push(Fabricated::of_format(
                "older-major",
                version(older, 0),
                Compatibility::OlderMajor,
            ));
        }
    }
    if rule == Rule::SharedStore {
        for (name, lowest_minor, compatibility) in [
            (
                "lowest-minor-met",
                STORE_ENGINE.minor,
                Compatibility::Compatible,
            ),
            (
                "lowest-minor-newer",
                next(STORE_ENGINE.minor)?,
                Compatibility::NewerMinor,
            ),
        ] {
            table.push(Fabricated {
                name,
                found: current,
                lowest_minor: Some(lowest_minor),
                engine: Some(STORE_ENGINE),
                compatibility,
            });
        }
    }
    table.push(Fabricated::of_format(
        "newer-major",
        version(next(current.major)?, 0),
        Compatibility::NewerMajor,
    ));
    Ok(table)
}

/// The Products the bundle defines, smallest first.
fn products(definitions: &BTreeMap<String, Value>) -> Result<Vec<Product>> {
    let variants = definitions
        .get("Product")
        .and_then(|product| product.get("oneOf"))
        .and_then(Value::as_array)
        .context("a protocol's `hello` names Products, and the bundle defines no `Product`")?;
    variants
        .iter()
        .map(|variant| {
            serde_json::from_value(variant.get("const").cloned().unwrap_or_default())
                .context("a `Product` of the bundle is no Product")
        })
        .collect()
}

/// A format's cases, by name.
fn cases(
    format: &Entry,
    definitions: &BTreeMap<String, Value>,
) -> Result<Vec<(String, RefusalCase)>> {
    if format.hello_from.is_empty() {
        return Ok(table(format.rule, format.version)?
            .into_iter()
            .map(|fabricated| {
                let case = RefusalCase {
                    format: format.name.to_string(),
                    rule: format.rule,
                    reader: format.version,
                    found: fabricated.found,
                    lowest_minor: fabricated.lowest_minor,
                    engine: fabricated.engine,
                    compatibility: fabricated.compatibility,
                    hello: None,
                };
                (fabricated.name.to_owned(), case)
            })
            .collect());
    }
    if format.rule == Rule::SharedStore {
        bail!("a protocol records no lowest minor; `shared_store` is a store's rule");
    }
    let products = products(definitions)?;
    let protocol = Protocol {
        format,
        smallest: *products.first().context("the bundle defines no Product")?,
        products,
    };
    let mut cases = Vec::new();
    for from in &format.hello_from {
        match from {
            HelloFrom::Client => protocol.client(&mut cases)?,
            HelloFrom::BrowserHost => protocol.browser_host(&mut cases)?,
            HelloFrom::GroupMember => protocol.group_member(&mut cases),
        }
    }
    Ok(cases)
}

/// A protocol's `hello`s.
struct Protocol<'format> {
    format: &'format Entry,
    products: Vec<Product>,
    /// The smallest Product, whose engine answers a `hello` whose case
    /// needs no other (ADR 0094).
    smallest: Product,
}

/// What a fabricated `hello` is and gets.
struct Hello {
    rule: Rule,
    found: ProtocolVersion,
    compatibility: Compatibility,
    from: HelloFrom,
    engine: Product,
    needs: Option<Product>,
    session_needs: Option<ProtocolVersion>,
    reply: Reply,
}

fn accepted(protocol_version: ProtocolVersion) -> Reply {
    Reply::Accepted { protocol_version }
}

fn refused(error_code: &str) -> Reply {
    Reply::Refused {
        error_code: error_code.to_owned(),
    }
}

impl Protocol<'_> {
    fn case(&self, hello: Hello) -> RefusalCase {
        RefusalCase {
            format: self.format.name.to_string(),
            rule: hello.rule,
            reader: self.format.version,
            found: hello.found,
            lowest_minor: None,
            engine: None,
            compatibility: hello.compatibility,
            hello: Some(FabricatedHello {
                from: hello.from,
                engine: hello.engine,
                needs: hello.needs,
                session_needs: hello.session_needs,
                reply: hello.reply,
            }),
        }
    }

    /// The reply to a `hello` the version check alone decides.
    fn version_reply(compatibility: Compatibility, settled: ProtocolVersion) -> Reply {
        match compatibility {
            Compatibility::Compatible => accepted(settled),
            Compatibility::NewerMinor | Compatibility::OlderMajor | Compatibility::NewerMajor => {
                refused(VERSION_REFUSED)
            }
        }
    }

    /// A Client's: the rule's versions, then the Product check of every
    /// pair, also before a version the rule refuses.
    fn client(&self, cases: &mut Vec<(String, RefusalCase)>) -> Result<()> {
        let current = self.format.version;
        for fabricated in table(self.format.rule, current)? {
            let case = self.case(Hello {
                rule: self.format.rule,
                found: fabricated.found,
                compatibility: fabricated.compatibility,
                from: HelloFrom::Client,
                engine: self.smallest,
                needs: Some(self.smallest),
                session_needs: None,
                reply: Self::version_reply(fabricated.compatibility, current),
            });
            cases.push((format!("client-{}", fabricated.name), case));
        }
        let newer_major = table(self.format.rule, current)?
            .into_iter()
            .find(|fabricated| fabricated.compatibility == Compatibility::NewerMajor)
            .context("every rule fabricates a newer major")?;
        for (rank, engine) in self.products.iter().enumerate() {
            for (needed_rank, needs) in self.products.iter().enumerate() {
                let name = format!(
                    "client-needing-{}-on-a-{}-engine",
                    needs.as_str(),
                    engine.as_str()
                );
                let contained = needed_rank <= rank;
                let reply = if contained {
                    accepted(current)
                } else {
                    refused(PRODUCT_REFUSED)
                };
                let hello = |found, compatibility, reply| Hello {
                    rule: self.format.rule,
                    found,
                    compatibility,
                    from: HelloFrom::Client,
                    engine: *engine,
                    needs: Some(*needs),
                    session_needs: None,
                    reply,
                };
                cases.push((
                    name.clone(),
                    self.case(hello(current, Compatibility::Compatible, reply)),
                ));
                if !contained {
                    cases.push((
                        format!("{name}-at-newer-major"),
                        self.case(hello(
                            newer_major.found,
                            Compatibility::NewerMajor,
                            refused(PRODUCT_REFUSED),
                        )),
                    ));
                }
            }
        }
        Ok(())
    }

    /// A Browser host's: no Product check, the lower minor settled on, only
    /// another major refused, and a Session needing more than an older
    /// Browser host's minor refused before sending.
    fn browser_host(&self, cases: &mut Vec<(String, RefusalCase)>) -> Result<()> {
        let current = self.format.version;
        let hello = |found, compatibility, session_needs, reply| Hello {
            rule: Rule::LowerMinor,
            found,
            compatibility,
            from: HelloFrom::BrowserHost,
            engine: self.smallest,
            needs: None,
            session_needs,
            reply,
        };
        // During `0.y` previews a line has no older minor (ADR 0030).
        let older_minor = current
            .minor
            .checked_sub(1)
            .filter(|_| current.major > 0)
            .map(|minor| version(current.major, minor));
        if let Some(older_minor) = older_minor {
            cases.push((
                "browser-host-older-minor".to_owned(),
                self.case(hello(
                    older_minor,
                    Compatibility::Compatible,
                    None,
                    accepted(older_minor),
                )),
            ));
        }
        for fabricated in table(Rule::LowerMinor, current)? {
            let reply = Self::version_reply(fabricated.compatibility, current);
            cases.push((
                format!("browser-host-{}", fabricated.name),
                self.case(hello(
                    fabricated.found,
                    fabricated.compatibility,
                    None,
                    reply,
                )),
            ));
        }
        if let Some(older_minor) = older_minor {
            cases.push((
                "browser-host-outdated".to_owned(),
                self.case(hello(
                    older_minor,
                    Compatibility::Compatible,
                    Some(current),
                    refused(BROWSER_HOST_OUTDATED),
                )),
            ));
        }
        Ok(())
    }

    /// A Group member's: refused by a group of the other Product.
    fn group_member(&self, cases: &mut Vec<(String, RefusalCase)>) {
        let current = self.format.version;
        for group in &self.products {
            for member in &self.products {
                let reply = if member == group {
                    accepted(current)
                } else {
                    refused(PRODUCT_REFUSED)
                };
                cases.push((
                    format!(
                        "group-member-of-{}-joining-a-{}-group",
                        member.as_str(),
                        group.as_str()
                    ),
                    self.case(Hello {
                        rule: self.format.rule,
                        found: current,
                        compatibility: Compatibility::Compatible,
                        from: HelloFrom::GroupMember,
                        engine: *group,
                        needs: Some(*member),
                        session_needs: None,
                        reply,
                    }),
                ));
            }
        }
    }
}
