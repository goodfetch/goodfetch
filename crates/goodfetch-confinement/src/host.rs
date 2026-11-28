//! What this host allows, which the engine probes once at startup (ADRs
//! 0033 and 0049): whether a process may create and map a user namespace,
//! whether it runs under a seccomp filter, whether AppArmor restricts
//! unprivileged user namespaces, the kernel's Landlock ABI, and whether it
//! is WSL1. The engine's `sandbox_probe` judges the same reading for
//! Chromium's sandbox and for a Plugin's confinement ([`HostFacts::confines`]),
//! so one probe explains both refusals. Linux reads it in
//! `linux::host`; other OSes need no user namespace.

use goodfetch_protocol::plugin::NetworkLevel;

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
#[expect(
    clippy::struct_excessive_bools,
    reason = "HostFacts holds independent host capability facts read once at startup (ADR 0049)"
)]
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
    /// Whether the installed `goodfetch-sandbox` helper is available and can
    /// create and map a user namespace on this host (Ubuntu 23.10+ after
    /// `plugin setup`, ADRs 0033 and 0039).
    pub sandbox_helper: bool,
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
        sandbox_helper: false,
    };

    /// Whether a Plugin can be confined here, by Landlock from ABI 6, by a
    /// user namespace of its own, or through the installed `goodfetch-sandbox`
    /// helper: never on WSL1, which has no seccomp, nor where none is given
    /// (ADR 0033's hosts that cannot confine).
    pub fn confines(&self) -> bool {
        !self.wsl1
            && (self.landlock_abi >= SIGNAL_SCOPE_ABI
                || self.user_namespace == UserNamespace::Created
                || self.sandbox_helper)
    }

    /// Whether starting a process Plugin at `network_level` on this host
    /// should use the installed `goodfetch-sandbox` helper instead of the
    /// Product executable's `plugin-shim` (ADR 0033, issue 884): true when
    /// the engine itself cannot map a user namespace, `network_level` or the
    /// kernel's Landlock ABI needs one, and the installed helper provides it.
    #[must_use]
    pub fn needs_sandbox_helper(&self, network_level: NetworkLevel) -> bool {
        self.sandbox_helper
            && !self.wsl1
            && self.user_namespace != UserNamespace::Created
            && (network_level == NetworkLevel::ProxyTcp || self.landlock_abi < SIGNAL_SCOPE_ABI)
    }

    /// The parts of process-Plugin confinement this host cannot provide for
    /// `network_level`: empty when the host can fully confine at that level
    /// (ADR 0033).
    #[must_use]
    pub fn missing_confinement(&self, network_level: NetworkLevel) -> Vec<&'static str> {
        if self.wsl1 {
            let mut missing = vec!["seccomp", "filesystem", "signal_scope", "user_namespace"];
            if network_level == NetworkLevel::ProxyTcp {
                missing.push("network_namespace");
            }
            return missing;
        }
        let has_userns = self.user_namespace == UserNamespace::Created || self.sandbox_helper;
        let has_fs = self.landlock_abi > 0 || has_userns;
        let has_signal_scope = self.landlock_abi >= SIGNAL_SCOPE_ABI || has_userns;
        let has_netns = network_level != NetworkLevel::ProxyTcp || has_userns;
        if has_fs && has_signal_scope && has_netns {
            return Vec::new();
        }
        let mut missing = Vec::new();
        if !has_fs {
            missing.push("filesystem");
        }
        if !has_signal_scope {
            missing.push("signal_scope");
        }
        if !has_netns {
            missing.push("network_namespace");
        }
        if !has_userns {
            missing.push("user_namespace");
        }
        missing
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
