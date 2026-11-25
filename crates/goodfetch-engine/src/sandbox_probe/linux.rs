//! The probe's AppArmor reads on Linux; what the host allows is read by
//! `goodfetch_confinement::linux::host`.

/// The AppArmor label of the process `pid`.
#[expect(
    clippy::disallowed_methods,
    reason = "a launch reads it right after its browser started, a read of `/proc` that does not block"
)]
pub(super) fn label_of(pid: u32) -> Option<String> {
    [
        format!("/proc/{pid}/attr/apparmor/current"),
        format!("/proc/{pid}/attr/current"),
    ]
    .iter()
    .find_map(|path| std::fs::read_to_string(path).ok())
    .filter(|label| !label.trim().is_empty())
}
