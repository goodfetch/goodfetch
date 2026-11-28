//! `goodfetch-filter-broker` executable entrypoint (ADRs 0033 and 0039,
//! issue 879): runs as a Windows service (`--service`), in foreground test
//! mode (`--run-foreground`), registers the Windows service
//! (`--install-service`), or uninstalls the service and removes all persistent
//! WFP filters (`--uninstall`).

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(windows)]
    {
        run_windows()
    }
    #[cfg(not(windows))]
    {
        ExitCode::from(goodfetch_confinement::launch::LaunchError::Unsupported.status())
    }
}

#[cfg(windows)]
#[expect(
    clippy::disallowed_types,
    reason = "Windows foreground runner passes a stop flag to the blocking named-pipe loop (ADR 0060)"
)]
fn run_windows() -> ExitCode {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use goodfetch_confinement::attestation::AttestationTrustRoot;
    use goodfetch_confinement::windows::broker::DEFAULT_PIPE_NAME;
    use goodfetch_filter_broker::win32::{
        current_user_sid, install_windows_service, run_windows_service, serve_pipe_loop,
        uninstall_broker,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--uninstall") {
        return match uninstall_broker() {
            Ok(_) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    if args.iter().any(|arg| arg == "--install-service") {
        let binary = flag_value(&args, "--binary")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok());
        let user_sid = flag_value(&args, "--user-sid").or_else(|| current_user_sid().ok());
        return match (binary, user_sid) {
            (Some(bin), Some(sid)) => match install_windows_service(&bin, &sid) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            },
            _ => ExitCode::FAILURE,
        };
    }
    if args.iter().any(|arg| arg == "--run-foreground") {
        let pipe = flag_value(&args, "--pipe").unwrap_or_else(|| DEFAULT_PIPE_NAME.to_owned());
        let Some(user_sid) = flag_value(&args, "--user-sid").or_else(|| current_user_sid().ok())
        else {
            return ExitCode::FAILURE;
        };
        let trust = flag_value(&args, "--trust-root-json")
            .and_then(|path| {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "foreground test harness reads an injected test trust root file synchronously"
                )]
                std::fs::read_to_string(path).ok()
            })
            .and_then(|text| {
                serde_json::from_str::<std::collections::BTreeMap<String, [u8; 32]>>(&text).ok()
            })
            .map_or_else(AttestationTrustRoot::bundled, AttestationTrustRoot::new);
        return match serve_pipe_loop(&pipe, &user_sid, trust, &Arc::new(AtomicBool::new(false))) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    let Some(user_sid) = flag_value(&args, "--user-sid").or_else(|| current_user_sid().ok()) else {
        return ExitCode::FAILURE;
    };
    match run_windows_service(user_sid) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(windows)]
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|pair| (pair[0] == flag).then(|| pair[1].clone()))
}
