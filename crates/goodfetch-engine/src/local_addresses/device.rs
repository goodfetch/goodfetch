//! Binding a Session's outbound socket to the device of the network interface
//! `egress.local_addresses` names (ADR 0044):
//!
//! - **Linux** sets `SO_BINDTODEVICE` to the interface name.
//! - **macOS and illumos** look up the interface's index once and set
//!   `IP_BOUND_IF` / `IPV6_BOUND_IF`.
//! - **Windows** binds by the interface's address alone, which forces the
//!   interface under Windows' strong host model by default (the interface
//!   needs its own route, and an administrator can change this with
//!   `WeakHostSend`).

use std::io;
use std::net::SocketAddr;
#[cfg(target_os = "linux")]
use std::sync::Arc;

/// The network device of the interface a Session's local addresses came from,
/// which each outbound socket binds on Linux (`SO_BINDTODEVICE`), macOS and
/// illumos (`IP_BOUND_IF`, `IPV6_BOUND_IF`).
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device(Option<Arc<str>>);

#[cfg(target_os = "linux")]
impl Device {
    /// The device of `interface`, or none when the Session does not name one.
    #[must_use]
    pub fn of(interface: Option<&str>) -> Self {
        Self(interface.map(Arc::from))
    }

    /// Binds `socket` to the interface's device (`SO_BINDTODEVICE`), if named.
    ///
    /// # Errors
    ///
    /// The OS error if `SO_BINDTODEVICE` is refused (such as a missing
    /// interface).
    pub fn bind(&self, socket: &socket2::SockRef<'_>, _address: SocketAddr) -> io::Result<()> {
        match &self.0 {
            Some(interface) => socket.bind_device(Some(interface.as_bytes())),
            None => Ok(()),
        }
    }
}

/// The interface a Session's local addresses came from, which each outbound
/// socket binds by its index (`IP_BOUND_IF`, `IPV6_BOUND_IF`), on macOS and
/// illumos. The index is read once, when the binding is built, as the
/// Session's addresses are.
#[cfg(any(target_os = "macos", target_os = "illumos"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    /// The Session names no interface.
    #[default]
    Any,
    /// The interface of this index.
    Index(std::num::NonZeroU32),
    /// An interface the host no longer has, which fails each attempt.
    Gone,
}

#[cfg(any(target_os = "macos", target_os = "illumos"))]
impl Device {
    /// The device of `interface`, or [`Device::Any`] when the Session does not
    /// name one.
    #[must_use]
    pub fn of(interface: Option<&str>) -> Self {
        let Some(name) = interface else {
            return Self::Any;
        };
        netdev::get_interfaces()
            .into_iter()
            .find(|interface| interface.name == name)
            .and_then(|interface| std::num::NonZeroU32::new(interface.index))
            .map_or(Self::Gone, Self::Index)
    }

    /// Binds `socket` to the interface's index (`IP_BOUND_IF` or
    /// `IPV6_BOUND_IF`), if named.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::NotFound`] if the interface no longer exists, or the OS
    /// error from setting the socket option.
    pub fn bind(&self, socket: &socket2::SockRef<'_>, address: SocketAddr) -> io::Result<()> {
        match *self {
            Self::Any => Ok(()),
            Self::Gone => Err(io::Error::from(io::ErrorKind::NotFound)),
            Self::Index(index) => match address {
                SocketAddr::V4(_) => socket.bind_device_by_index_v4(Some(index)),
                SocketAddr::V6(_) => socket.bind_device_by_index_v6(Some(index)),
            },
        }
    }
}

/// Nothing: Windows binds the interface's address alone, which forces the
/// interface under Windows' strong host model by default; the interface needs
/// its own route and an administrator can change this with `WeakHostSend`
/// (ADR 0044).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Device;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
impl Device {
    /// No device binding on this OS: address binding forces the interface on
    /// Windows under the strong host model (ADR 0044).
    #[must_use]
    pub fn of(_interface: Option<&str>) -> Self {
        Self
    }

    /// No-op on this OS: address binding alone is used (ADR 0044).
    ///
    /// # Errors
    ///
    /// Never fails on this OS.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "it binds as the other OSes' `Device` does, which reads itself and can fail"
    )]
    pub fn bind(&self, _socket: &socket2::SockRef<'_>, _address: SocketAddr) -> io::Result<()> {
        Ok(())
    }
}
