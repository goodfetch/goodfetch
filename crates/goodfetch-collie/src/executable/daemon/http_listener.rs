//! Opens the Daemon's HTTP JSON-RPC listener (`http_listener.enabled`,
//! `http_listener.address`, `http_listener.port`) for both a machine's Daemon
//! (`daemon launched`) and the foreground Daemon (`daemon run`), reusing the
//! Exposed Daemon's TLS configuration (`daemon.tls.*`, ADRs 0027 and 0049).

use std::net::{IpAddr, SocketAddr};

use goodfetch_engine::daemon::files::DaemonFiles;
use goodfetch_engine::endpoint::{Authorities, Exposure, Termination};
use goodfetch_engine::http_listener::HttpListenerOptions;
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{Resolved, SettingsError};
use goodfetch_engine::transport::Network;
use goodfetch_protocol::daemon::{DaemonEndpoint, TLS_CERT_FILE, TLS_KEY_FILE, TLS_UPSTREAM};
use goodfetch_protocol::http_listener::{ADDRESS, ENABLED, PORT};
use goodfetch_protocol::settings::SettingsLayer;
use secrecy::SecretString;

use super::tls::open_termination;
use super::{Failure, Machine, unavailable};

/// Opens the HTTP listener when `http_listener.enabled` is true; returns
/// `Ok(None)` when off. If `termination` is `Some`, reuses the TLS termination
/// already opened by `daemon run`; otherwise reads `daemon.tls.*` from
/// `settings`.
pub(super) async fn open(
    machine: &Machine<'_>,
    settings: &Resolved,
    files: &DaemonFiles,
    termination: Option<Termination>,
) -> Result<Option<HttpListenerOptions>, Failure> {
    if !settings.get::<bool>(&ENABLED) {
        return Ok(None);
    }
    let address_text = settings.get::<&str>(&ADDRESS);
    let address_layer = settings
        .effective(&ADDRESS)
        .map_or(SettingsLayer::Default, |effective| effective.layer);
    let ip = address_text.parse::<IpAddr>().map_err(|_| {
        settings_failure(
            machine,
            &SettingsError::Invalid {
                key: ADDRESS.name.to_owned(),
                layer: Some(address_layer),
                reason: format!(
                    "`{address_text}` is not an IP address; set an address such as `127.0.0.1`"
                ),
            },
        )
    })?;
    let port_u64 = settings.get::<u64>(&PORT);
    let port = u16::try_from(port_u64).map_err(|_| {
        let port_layer = settings
            .effective(&PORT)
            .map_or(SettingsLayer::Default, |effective| effective.layer);
        settings_failure(
            machine,
            &SettingsError::Invalid {
                key: PORT.name.to_owned(),
                layer: Some(port_layer),
                reason: format!("`{port_u64}` is not a valid TCP port (0..=65535)"),
            },
        )
    })?;
    let termination = match termination {
        Some(termination) => termination,
        None => open_termination(settings).map_err(|error| settings_failure(machine, &error))?,
    };
    if !ip.is_loopback() && matches!(termination, Termination::Plain) {
        return Err(settings_failure(
            machine,
            &SettingsError::Invalid {
                key: ADDRESS.name.to_owned(),
                layer: Some(address_layer),
                reason: format!(
                    "`{address_text}` is not a loopback address: the HTTP listener binds elsewhere only when TLS is terminated; set `{}` and `{}`, or `{}` behind an ingress that terminates it, or bind loopback",
                    TLS_CERT_FILE.name, TLS_KEY_FILE.name, TLS_UPSTREAM.name
                ),
            },
        ));
    }
    let token = files.http_listener_token().map_err(unavailable(
        "the HTTP listener's token cannot be read or made",
    ))?;
    let requested = SocketAddr::new(ip, port);
    let (listener, endpoint) = Network::Os.listen_at(requested).await.map_err(unavailable(
        "the Daemon cannot listen at `http_listener.address`",
    ))?;
    let DaemonEndpoint::Tcp(bound) = endpoint else {
        return Err(Failure::Unavailable(
            "the HTTP listener did not bind a TCP address".to_owned(),
        ));
    };
    tracing::info!(
        address = %bound,
        token_file = %files.http_listener_token_file().display(),
        "the HTTP listener serves"
    );
    Ok(Some(HttpListenerOptions {
        listener,
        bound,
        token: SecretString::from(token),
        exposure: Exposure {
            authorities: Authorities::loopback(bound),
            termination,
        },
    }))
}

fn settings_failure(machine: &Machine<'_>, error: &SettingsError) -> Failure {
    Failure::Refused {
        error: local_settings_error(error, machine.identity).to_error_object(),
        operation: None,
    }
}
