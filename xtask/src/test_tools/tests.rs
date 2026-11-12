use super::shipped_target_of;

#[test]
fn linux_ships_the_static_musl_build_of_its_architecture() {
    assert_eq!(
        shipped_target_of("linux", "x86_64").as_deref(),
        Some("x86_64-unknown-linux-musl")
    );
    assert_eq!(
        shipped_target_of("linux", "aarch64").as_deref(),
        Some("aarch64-unknown-linux-musl")
    );
}

#[test]
fn macos_and_windows_ship_the_hosts_own_build() {
    assert_eq!(shipped_target_of("macos", "aarch64"), None);
    assert_eq!(shipped_target_of("windows", "x86_64"), None);
}
