//! What this host allows, which the engine probes once at startup (ADRs
//! 0033 and 0049): whether a process may create and map a user namespace,
//! whether it runs under a seccomp filter, whether AppArmor restricts
//! unprivileged user namespaces, the kernel's Landlock ABI, and whether it
//! is WSL1. The engine's `sandbox_probe` judges the same reading for
//! Chromium's sandbox and for a Plugin's confinement ([`HostFacts::confines`]),
//! so one probe explains both refusals. Linux reads it in
//! `linux::host`; other OSes need no user namespace.

/// The Landlock ABI that scopes signals (kernel 6.12), so that a Plugin is
/// confined without a namespace of its own.
pub const SIGNAL_SCOPE_ABI: u8 = 6;

/// What the probe found of the user namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserNamespace {
    /// Created, and this user mapped into it.
    Created,
    /// `unshare(CLONE_NEWUSER)` refused, with its `errno`.
    Refused(i32),
    /// Created, but this user could not be mapped into it, with the
    /// `errno` of the write to `uid_map`: AppArmor's restriction leaves the
    /// namespace without the capabilities to map itself.
    Unmapped(i32),
}

/// What this process's host allows, alike for every engine in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostFacts {
    /// The user namespace a child of this process tried to make.
    pub user_namespace: UserNamespace,
    /// Whether this process runs under a seccomp filter (`Seccomp: 2`).
    pub seccomp_filtered: bool,
    /// Whether AppArmor restricts unprivileged user namespaces
    /// (`kernel.apparmor_restrict_unprivileged_userns=1`).
    pub apparmor_restricts: bool,
    /// The Landlock ABI the kernel gives, 0 without Landlock.
    pub landlock_abi: u8,
    /// Whether the kernel is WSL1's, which offers neither seccomp nor user
    /// namespaces.
    pub wsl1: bool,
}

impl HostFacts {
    /// What an OS whose confinement needs no user namespace allows: every
    /// OS but Linux.
    pub const NO_NAMESPACE_NEEDED: Self = Self {
        user_namespace: UserNamespace::Created,
        seccomp_filtered: false,
        apparmor_restricts: false,
        landlock_abi: 0,
        wsl1: false,
    };

    /// Whether a Plugin can be confined here, by Landlock from ABI 6 or by
    /// a user namespace of its own, as `linux::MechanismKind::of_kernel`
    /// picks: never on WSL1, which has no seccomp, nor where neither is
    /// given (ADR 0033's hosts that cannot confine).
    pub fn confines(&self) -> bool {
        !self.wsl1
            && (self.landlock_abi >= SIGNAL_SCOPE_ABI
                || self.user_namespace == UserNamespace::Created)
    }
}

/// Whether the kernel release `release`, as `/proc/sys/kernel/osrelease`
/// reads, is WSL1's: it ends in `-Microsoft`, where WSL 2's real Linux
/// kernel names itself `microsoft-standard-WSL2`.
pub fn is_wsl1(release: &str) -> bool {
    let release = release.trim();
    release.contains("Microsoft") && !release.contains("WSL2")
}

#[cfg(test)]
mod tests;
