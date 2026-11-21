//! The Politeness file's wake endpoints (ADR 0053): where an engine waiting
//! for a Permit is woken by the engine that gave one back, with no polling.
//! A Unix socket, readable and writable by its owner alone, on macOS and
//! Linux, and a named pipe on Windows. Each wake is one short message on a
//! connection of its own.

use std::io;
use std::path::Path;

/// The most bytes one wake carries.
const LONGEST_WAKE: u64 = 4096;

/// A wake endpoint's listener.
pub struct WakeListener(Listening);

#[cfg(unix)]
#[expect(
    clippy::disallowed_types,
    reason = "the transport module is the one place that opens sockets (ADR 0060)"
)]
struct Listening(tokio::net::UnixListener);

#[cfg(windows)]
struct Listening {
    name: String,
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens pipes (ADR 0060)"
    )]
    server: tokio::net::windows::named_pipe::NamedPipeServer,
}

/// Listens for wakes: on a Unix socket at `place` on macOS and Linux, and
/// on a named pipe named after `place`'s folder and file names on Windows.
/// Answers the endpoint a waiter's row names.
///
/// It blocks on the file system, so it runs as its owner's blocking work
/// ([`crate::tasks::TaskOwner::blocking`]), in the runtime's context, which
/// registers the listener: inline under
/// [`crate::tasks::BlockingPolicy::Inline`], so a run replays from its seed
/// whatever the host's load, where the runtime's own blocking pool let the
/// other tasks run for as long as its thread took.
///
/// # Errors
///
/// When the OS refuses the socket or the pipe.
#[cfg(unix)]
pub fn listen_wakes(place: &Path) -> io::Result<(WakeListener, String)> {
    use std::os::unix::fs::PermissionsExt as _;
    #[expect(
        clippy::disallowed_methods,
        reason = "it runs on the tasks module's blocking policy"
    )]
    let removed = std::fs::remove_file(place);
    match removed {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::UnixListener::bind(place)?;
    std::fs::set_permissions(place, std::fs::Permissions::from_mode(0o600))?;
    let endpoint = place.to_str().map(str::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a wake endpoint's path is not Unicode",
        )
    })?;
    Ok((WakeListener(Listening(listener)), endpoint))
}

/// Listens for wakes on a named pipe named after `place`'s folder and file
/// names, as blocking work as on Unix. A pipe's name is global to the host,
/// so the folder, made for this endpoint alone, keeps it apart from every
/// other engine's: with the file name alone, a second engine's pipe was
/// refused, the first holding its name as its first instance.
///
/// # Errors
///
/// When the OS refuses the pipe.
#[cfg(windows)]
pub fn listen_wakes(place: &Path) -> io::Result<(WakeListener, String)> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let folder = place
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("goodfetch");
    let file = place
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wake");
    let name = format!(r"\\.\pipe\{folder}-{file}");
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    Ok((
        WakeListener(Listening {
            name: name.clone(),
            server,
        }),
        name,
    ))
}

impl WakeListener {
    /// The next wake's message.
    ///
    /// # Errors
    ///
    /// When the OS fails to accept a connection or read it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses at most the wake it was reading; its
    /// waiter is woken again by the next Permit given back, or by its own
    /// deadline.
    pub async fn accept(&mut self) -> io::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt as _;
        let mut message = Vec::new();
        #[cfg(unix)]
        {
            let (stream, _) = self.0.0.accept().await?;
            stream.take(LONGEST_WAKE).read_to_end(&mut message).await?;
        }
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            self.0.server.connect().await?;
            let next = ServerOptions::new().create(&self.0.name)?;
            let connected = std::mem::replace(&mut self.0.server, next);
            connected
                .take(LONGEST_WAKE)
                .read_to_end(&mut message)
                .await?;
        }
        Ok(message)
    }
}

/// Wakes the waiter whose wake endpoint is `endpoint` with `message`.
///
/// # Errors
///
/// `NotFound` or `ConnectionRefused` when nothing listens there any more,
/// such as the endpoint of an engine that ended; another error when the OS
/// failed otherwise.
///
/// # Cancel safety
///
/// Dropping the future sends at most a partial message, which the listener
/// reads as no wake.
pub async fn wake(endpoint: &str, message: &[u8]) -> io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    #[cfg(unix)]
    {
        #[expect(
            clippy::disallowed_types,
            reason = "the transport module is the one place that opens sockets (ADR 0060)"
        )]
        let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
        stream.write_all(message).await?;
        stream.shutdown().await
    }
    #[cfg(windows)]
    {
        let mut pipe = tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)?;
        pipe.write_all(message).await?;
        pipe.shutdown().await
    }
}

/// Whether a wake's failure says nothing listens at its endpoint any more.
pub fn is_gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

#[cfg(test)]
mod tests;
