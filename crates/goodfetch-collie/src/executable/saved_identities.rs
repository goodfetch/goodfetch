//! `identity list|import|delete|export` (ADRs 0015, 0035, 0056, 0069, 0090 and
//! 0095; ticket #736): manages Saved identities in the per-major encrypted
//! store shared by both Collie and Foxhound.
//!
//! `identity export` writes ADR 0015's `age`-encrypted `session-export` bundle
//! by default (`--format bundle`); its clear-text Playwright `storageState`
//! form (`--format storage-state`) is person-only, asked at an interactive
//! terminal through [`Prompt`] and refused with `person_required` under
//! `--json` or when stdin is not a terminal.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand, ValueEnum};
use goodfetch_engine::{CallError, Dependencies, Engine};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::identity::{
    IDENTITY_DELETE, IDENTITY_EXPORT, IDENTITY_EXPORT_STORAGE_STATE, IDENTITY_IMPORT,
    IDENTITY_LIST, IdentityDeleteParams, IdentityDeleted, IdentityExportParams,
    IdentityExportStorageStateParams, IdentityExported, IdentityImportParams, IdentityList,
    IdentityStorageStateExported, SavedIdentityInfo,
};
use goodfetch_protocol::page::proxy::ProxyKind;
use goodfetch_protocol::settings::session_source::{SessionSource, StateFileFormat};
use serde_json::json;

use super::catalog_commands::{Joined, Joining};
use super::daemon::Machine;
use super::engine_choice::{EngineChoice, EngineFlags};
use super::exit_codes;
use super::operation::run_operation;
use super::output::{self, Human};
use super::prompts::Prompt;

/// The person-only prompt for clear-text Playwright `storageState` export.
const EXPORT_STORAGE_STATE: Prompt = Prompt {
    operation: &IDENTITY_EXPORT_STORAGE_STATE,
    elsewhere: None,
};

/// Output format for `identity export`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(super) enum ExportFormat {
    /// Encrypted `age` `session-export` bundle (default, safe for scripts and CI).
    #[default]
    Bundle,
    /// Clear-text Playwright 1.64 `storageState` JSON (person-only at a terminal).
    #[value(name = "storage-state")]
    StorageState,
}

#[derive(Subcommand)]
pub(super) enum IdentityCommand {
    #[command(flatten)]
    Generated(Joining<OwnIdentityCommand>),
    #[command(flatten)]
    Own(OwnIdentityCommand),
}

#[derive(Subcommand)]
pub(super) enum OwnIdentityCommand {
    /// Lists every non-expired Saved identity in this Product major's store,
    /// with its metadata and domains (never secret values).
    List {
        /// Prints the list as JSON.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineFlags,
    },
    /// Imports cookies and storage from a file, Cookie header, or JSON source
    /// into this Product major's encrypted Saved identity store.
    Import(Box<ImportArgs>),
    /// Deletes the named Saved identity from this Product major's store,
    /// wiping its encrypted secret fields and recording the deletion in the
    /// Audit log.
    Delete {
        /// The Saved identity's name to delete.
        name: String,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineFlags,
    },
    /// Exports the named Saved identity as an `age`-encrypted `session-export`
    /// bundle (default), or as clear-text Playwright `storageState` when
    /// `--format storage-state` is confirmed by a person at a terminal.
    Export {
        /// The Saved identity's name to export.
        name: String,
        /// Export format: `bundle` (`age`-encrypted, default) or
        /// `storage-state` (clear text, person-only at a terminal).
        #[arg(long, value_enum, default_value_t = ExportFormat::Bundle)]
        format: ExportFormat,
        /// `age` public key (`age1…`) to encrypt a `bundle` export to.
        #[arg(long, value_name = "AGE_PUBLIC_KEY")]
        to_public_key: Option<String>,
        /// Secret reference whose value is the passphrase for a `bundle`
        /// export.
        #[arg(long, value_name = "SECRET_REF", conflicts_with = "to_public_key")]
        passphrase: Option<String>,
        /// Destination file path (mode `0600` on Unix); prints to stdout when
        /// omitted.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Prints the result as JSON (refused for `--format storage-state`).
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineFlags,
    },
}

#[derive(Args)]
pub(super) struct ImportArgs {
    /// The name under which to keep the Saved identity.
    name: String,
    /// Path to a Playwright `storageState`, Netscape `cookies.txt`, CDP
    /// cookie JSON array, or `age`-encrypted `session-export` bundle.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// Overrides automatic format detection for `--file`.
    #[arg(long, value_parser = ["storage_state", "cookies_txt", "cdp_cookies", "session_export"])]
    format: Option<String>,
    /// Secret reference of the `age` identity or passphrase opening a
    /// `session_export` bundle.
    #[arg(long)]
    key: Option<String>,
    /// Raw `Cookie:` header value to import for `--domain`.
    #[arg(long, conflicts_with = "file")]
    cookie_header: Option<String>,
    /// Domain filter (or target domain for `--cookie-header`). Repeat to
    /// include several domains.
    #[arg(long = "domain")]
    domains: Vec<String>,
    /// Raw `session.source` JSON object, in place of `--file` /
    /// `--cookie-header`.
    #[arg(long, conflicts_with_all = ["file", "cookie_header"])]
    source: Option<String>,
    /// Access mode to bind (`standard` by default, or `stealth`).
    #[arg(long, value_parser = ["standard", "stealth"])]
    access_mode: Option<String>,
    /// BCP 47 locale (language) of the imported identity.
    #[arg(long)]
    locale: Option<String>,
    /// IANA time zone ID of the imported identity.
    #[arg(long)]
    timezone_id: Option<String>,
    /// Proxy server (`scheme://host:port`, without credentials) to bind.
    #[arg(long)]
    proxy_server: Option<String>,
    /// ISO 3166-1 alpha-2 country code of the proxy binding.
    #[arg(long)]
    proxy_country: Option<String>,
    /// Proxy kind of the proxy binding (`datacenter`, `isp`, `residential`, `mobile`).
    #[arg(long, value_parser = ["datacenter", "isp", "residential", "mobile"])]
    proxy_kind: Option<String>,
    /// Prints the saved metadata as JSON.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    engine: EngineFlags,
}

impl Joined for OwnIdentityCommand {
    const NAMESPACE: &'static str = "identity";
}

/// Runs the `identity` command `command`.
pub(super) fn run(
    command: IdentityCommand,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    human: &Human,
) -> ExitCode {
    match command {
        IdentityCommand::Generated(joining) => {
            joining.call().run(engine, dependencies, machine, human)
        }
        IdentityCommand::Own(OwnIdentityCommand::List {
            json,
            engine: flags,
        }) => {
            let warn = |text: &str| human.warning(text);
            let reach = EngineChoice::passing(flags.choice(), machine, &warn);
            list(engine, dependencies, &reach, json, human)
        }
        IdentityCommand::Own(OwnIdentityCommand::Import(args)) => {
            let ImportArgs {
                name,
                file,
                format,
                key,
                cookie_header,
                domains,
                source,
                access_mode,
                locale,
                timezone_id,
                proxy_server,
                proxy_country,
                proxy_kind,
                json,
                engine: flags,
            } = *args;
            let warn = |text: &str| human.warning(text);
            let reach = EngineChoice::passing(flags.choice(), machine, &warn);
            let resolved_source = match build_import_source(
                file,
                format.as_deref(),
                key,
                cookie_header,
                domains,
                source.as_deref(),
            ) {
                Ok(src) => src,
                Err(message) => {
                    let error = CallError::protocol(ProtocolFailure::InvalidParams, message)
                        .to_error_object();
                    return output::fail(error, Some(IDENTITY_IMPORT.name), json, human);
                }
            };
            let params = IdentityImportParams {
                name,
                source: resolved_source,
                access_mode,
                locale,
                timezone_id,
                proxy_server,
                proxy_country,
                proxy_kind: proxy_kind.as_deref().and_then(ProxyKind::named),
            };
            import_cmd(engine, dependencies, &reach, &params, json, human)
        }
        IdentityCommand::Own(OwnIdentityCommand::Delete {
            name,
            json,
            engine: flags,
        }) => {
            let warn = |text: &str| human.warning(text);
            let reach = EngineChoice::passing(flags.choice(), machine, &warn);
            delete_cmd(engine, dependencies, &reach, name, json, human)
        }
        IdentityCommand::Own(OwnIdentityCommand::Export {
            name,
            format,
            to_public_key,
            passphrase,
            out,
            json,
            engine: flags,
        }) => {
            let warn = |text: &str| human.warning(text);
            let reach = EngineChoice::passing(flags.choice(), machine, &warn);
            match format {
                ExportFormat::Bundle => export_bundle(
                    engine,
                    dependencies,
                    &reach,
                    name,
                    to_public_key,
                    passphrase,
                    out,
                    json,
                    human,
                ),
                ExportFormat::StorageState => {
                    export_storage_state(engine, dependencies, &reach, name, out, json, human)
                }
            }
        }
    }
}

fn build_import_source(
    file: Option<PathBuf>,
    format: Option<&str>,
    key: Option<String>,
    cookie_header: Option<String>,
    domains: Vec<String>,
    source_json: Option<&str>,
) -> Result<SessionSource, String> {
    if let Some(raw) = source_json {
        return serde_json::from_str::<SessionSource>(raw)
            .map_err(|error| format!("invalid `--source` JSON: {error}"));
    }
    if let Some(header) = cookie_header {
        let domain = domains
            .into_iter()
            .next()
            .ok_or_else(|| "`--cookie-header` requires `--domain <DOMAIN>`".to_owned())?;
        return Ok(SessionSource::CookieHeader { header, domain });
    }
    if let Some(path) = file {
        let parsed_format = match format {
            None => None,
            Some("storage_state") => Some(StateFileFormat::StorageState),
            Some("cookies_txt") => Some(StateFileFormat::CookiesTxt),
            Some("cdp_cookies") => Some(StateFileFormat::CdpCookies),
            Some("session_export") => Some(StateFileFormat::SessionExport),
            Some(other) => return Err(format!("unknown `--format` `{other}`")),
        };
        return Ok(SessionSource::StateFile {
            path: path.to_string_lossy().into_owned(),
            format: parsed_format,
            key,
            domains: (!domains.is_empty()).then_some(domains),
        });
    }
    Err(
        "give `--file <PATH>`, `--cookie-header <HEADER> --domain <DOMAIN>`, or `--source <JSON>`"
            .to_owned(),
    )
}

fn list(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let answered = match run_operation(
        engine,
        dependencies,
        reach,
        &IDENTITY_LIST,
        json!({}),
        json,
        human,
    ) {
        Ok(value) => value,
        Err(exit) => return exit,
    };
    if json {
        output::json(&answered);
        return ExitCode::from(exit_codes::SUCCESS);
    }
    match serde_json::from_value::<IdentityList>(answered) {
        Ok(listed) if listed.identities.is_empty() => {
            output::line("No saved identities.");
            ExitCode::from(exit_codes::SUCCESS)
        }
        Ok(listed) => {
            for info in &listed.identities {
                print_info(info, human);
            }
            ExitCode::from(exit_codes::SUCCESS)
        }
        Err(problem) => output::fail(
            exit_codes::internal(format!(
                "The engine answered `{}` with an unexpected list: {problem}.",
                IDENTITY_LIST.name
            )),
            Some(IDENTITY_LIST.name),
            json,
            human,
        ),
    }
}

fn import_cmd(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    params: &IdentityImportParams,
    json: bool,
    human: &Human,
) -> ExitCode {
    let body = serde_json::to_value(params).expect("invariant: IdentityImportParams encodes");
    let answered = match run_operation(
        engine,
        dependencies,
        reach,
        &IDENTITY_IMPORT,
        body,
        json,
        human,
    ) {
        Ok(value) => value,
        Err(exit) => return exit,
    };
    if json {
        output::json(&answered);
        return ExitCode::from(exit_codes::SUCCESS);
    }
    match serde_json::from_value::<SavedIdentityInfo>(answered) {
        Ok(info) => {
            print_info(&info, human);
            ExitCode::from(exit_codes::SUCCESS)
        }
        Err(problem) => output::fail(
            exit_codes::internal(format!(
                "The engine answered `{}` with unexpected metadata: {problem}.",
                IDENTITY_IMPORT.name
            )),
            Some(IDENTITY_IMPORT.name),
            json,
            human,
        ),
    }
}

fn delete_cmd(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    name: String,
    json: bool,
    human: &Human,
) -> ExitCode {
    let body = serde_json::to_value(&IdentityDeleteParams { name })
        .expect("invariant: IdentityDeleteParams encodes");
    let answered = match run_operation(
        engine,
        dependencies,
        reach,
        &IDENTITY_DELETE,
        body,
        json,
        human,
    ) {
        Ok(value) => value,
        Err(exit) => return exit,
    };
    if json {
        output::json(&answered);
        return ExitCode::from(exit_codes::SUCCESS);
    }
    match serde_json::from_value::<IdentityDeleted>(answered) {
        Ok(deleted) => {
            if deleted.deleted {
                output::line(&format!("Deleted saved identity `{}`.", deleted.name));
            } else {
                output::line(&format!("No saved identity named `{}`.", deleted.name));
            }
            ExitCode::from(exit_codes::SUCCESS)
        }
        Err(problem) => output::fail(
            exit_codes::internal(format!(
                "The engine answered `{}` with an unexpected response: {problem}.",
                IDENTITY_DELETE.name
            )),
            Some(IDENTITY_DELETE.name),
            json,
            human,
        ),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "maps the CLI flags of `identity export --format bundle` directly"
)]
fn export_bundle(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    name: String,
    public_key: Option<String>,
    passphrase: Option<String>,
    out: Option<PathBuf>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let body = serde_json::to_value(&IdentityExportParams {
        name,
        public_key,
        passphrase,
    })
    .expect("invariant: IdentityExportParams encodes");
    let answered = match run_operation(
        engine,
        dependencies,
        reach,
        &IDENTITY_EXPORT,
        body,
        json,
        human,
    ) {
        Ok(value) => value,
        Err(exit) => return exit,
    };
    let exported: IdentityExported = match serde_json::from_value(answered.clone()) {
        Ok(exported) => exported,
        Err(problem) => {
            return output::fail(
                exit_codes::internal(format!(
                    "The engine answered `{}` with an unexpected bundle: {problem}.",
                    IDENTITY_EXPORT.name
                )),
                Some(IDENTITY_EXPORT.name),
                json,
                human,
            );
        }
    };
    if let Some(path) = out {
        if let Err(error) = write_private_file(&path, exported.bundle.as_bytes()) {
            let failed = CallError::protocol(
                ProtocolFailure::InternalError,
                format!("could not write `{}`: {error}", path.display()),
            )
            .to_error_object();
            return output::fail(failed, Some(IDENTITY_EXPORT.name), json, human);
        }
        if json {
            output::json(&json!({
                "name": exported.name,
                "path": path.to_string_lossy(),
                "digest": exported.digest,
                "cookie_count": exported.cookie_count,
                "origin_count": exported.origin_count,
            }));
        } else {
            human.field("name", &exported.name);
            human.field("path", &path.display().to_string());
            human.field("digest", &exported.digest);
            human.field("cookies", &exported.cookie_count.to_string());
            human.field("origins", &exported.origin_count.to_string());
        }
        return ExitCode::from(exit_codes::SUCCESS);
    }
    if json {
        output::json(&answered);
    } else {
        output::line(exported.bundle.trim_end_matches('\n'));
    }
    ExitCode::from(exit_codes::SUCCESS)
}

fn export_storage_state(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    name: String,
    out: Option<PathBuf>,
    json: bool,
    human: &Human,
) -> ExitCode {
    if let Err(exit) = EXPORT_STORAGE_STATE.person_present(json, human) {
        return exit;
    }
    let question = format!("Export saved identity `{name}` as clear-text Playwright storageState?");
    let warning = "Clear-text storageState contains readable cookies and login tokens (ADR 0056).";
    let yes = match EXPORT_STORAGE_STATE.ask(&question, Some(warning), human) {
        Ok(yes) => yes,
        Err(exit) => return exit,
    };
    if !yes {
        output::line("Export cancelled.");
        return ExitCode::from(exit_codes::SUCCESS);
    }
    let body = serde_json::to_value(&IdentityExportStorageStateParams { name })
        .expect("invariant: IdentityExportStorageStateParams encodes");
    let answered = match run_operation(
        engine,
        dependencies,
        reach,
        &IDENTITY_EXPORT_STORAGE_STATE,
        body,
        false,
        human,
    ) {
        Ok(value) => value,
        Err(exit) => return exit,
    };
    let exported: IdentityStorageStateExported = match serde_json::from_value(answered) {
        Ok(exported) => exported,
        Err(problem) => {
            return output::fail(
                exit_codes::internal(format!(
                    "The engine answered `{}` with an unexpected document: {problem}.",
                    IDENTITY_EXPORT_STORAGE_STATE.name
                )),
                Some(IDENTITY_EXPORT_STORAGE_STATE.name),
                false,
                human,
            );
        }
    };
    if let Some(path) = out {
        if let Err(error) = write_private_file(&path, exported.storage_state.as_bytes()) {
            let failed = CallError::protocol(
                ProtocolFailure::InternalError,
                format!("could not write `{}`: {error}", path.display()),
            )
            .to_error_object();
            return output::fail(
                failed,
                Some(IDENTITY_EXPORT_STORAGE_STATE.name),
                false,
                human,
            );
        }
        human.field("name", &exported.name);
        human.field("path", &path.display().to_string());
        human.field("cookies", &exported.cookie_count.to_string());
        human.field("origins", &exported.origin_count.to_string());
    } else {
        output::line(&exported.storage_state);
    }
    ExitCode::from(exit_codes::SUCCESS)
}

fn print_info(info: &SavedIdentityInfo, human: &Human) {
    human.field("name", &info.name);
    human.field("access_mode", &info.access_mode);
    human.field("browser", &info.browser_family);
    human.field("cookies", &info.cookie_count.to_string());
    human.field("origins", &info.origin_count.to_string());
    if !info.domains.is_empty() {
        human.field("domains", &info.domains.join(", "));
    }
    if info.was_lent {
        human.field("was_lent", "true");
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI command writes its `--out` file synchronously before exiting"
)]
fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
