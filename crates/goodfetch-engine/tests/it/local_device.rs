//! Linux network-namespace proof of interface device binding (`SO_BINDTODEVICE`,
//! ADR 0044, ticket 1616): when `egress.local_addresses` names an interface,
//! both `LocalBinding::connect` (used by the egress relay) and
//! `http_client::connect` bind each outbound socket to that device; omitting
//! the interface leaves the socket unbound to a device, and naming a missing
//! device fails the connection even when the local IP is valid.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::SystemTime;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{Dns, HostMapping, ResolverConfiguration};
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{self, Connecting, Egress, LocalBinding};
use goodfetch_engine::local_addresses::{BlockSizes, LocalAddresses, Rotation, System};
use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv};
use serde_json::json;

const SEED: u64 = 1616;
const CHILD: &str = "local_device::in_the_namespace_interface_binding_sets_so_bindtodevice";

/// Covers: story:4-44, story:4-45, setting:egress.local_addresses
#[test]
fn on_linux_an_interface_local_address_binds_engine_client_sockets_to_its_device() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
    let namespace = DnsNamespace::enter(&env, fixture).unwrap();

    let output = namespace
        .output(
            &env,
            std::env::current_exe().unwrap(),
            [CHILD, "--exact", "--include-ignored"],
        )
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{output:?}"
    );
}

/// The child, running inside the isolated Linux network namespace where `lo`
/// is up with `127.0.0.1` and `127.0.0.2`.
#[test]
#[ignore = "run as a child by local_device::on_linux_an_interface_local_address_binds_engine_client_sockets_to_its_device"]
#[expect(
    clippy::disallowed_methods,
    reason = "runs on the test runtime inside the isolated network namespace (ADR 0062)"
)]
fn in_the_namespace_interface_binding_sets_so_bindtodevice() {
    let rotation = Rotation::new(
        LocalAddresses::parse(&json!("lo")).unwrap(),
        BlockSizes::default(),
    );
    rotation.check(&System).unwrap();
    let interface = rotation.interface();
    let addresses = rotation.session(&System, 0).unwrap();

    let (bound_relay, unbound_relay, missing_relay, bound_client, unbound_client) =
        goodfetch_test_support::runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let target: SocketAddr = listener.local_addr().unwrap();
            let accept = tokio::spawn(async move {
                let mut accepted = Vec::new();
                for _ in 0..4 {
                    if let Ok((stream, _)) = listener.accept().await {
                        accepted.push(stream);
                    }
                }
                accepted
            });

            let clock = Clock::paused(SystemTime::UNIX_EPOCH);
            let dns = Dns::new(
                HostMapping::default(),
                ResolverConfiguration::system().unwrap(),
                clock,
            );

            let device = goodfetch_engine::local_addresses::Device::of(interface);
            let none = goodfetch_engine::local_addresses::Device::default();
            let missing = goodfetch_engine::local_addresses::Device::of(Some("missing_dev0"));

            let bound_relay = bound_device(
                &LocalBinding::new(addresses, device.clone(), &dns)
                    .connect(target)
                    .await
                    .unwrap(),
            );
            let unbound_relay = bound_device(
                &LocalBinding::new(addresses, none.clone(), &dns)
                    .connect(target)
                    .await
                    .unwrap(),
            );
            let missing_relay = LocalBinding::new(addresses, missing, &dns)
                .connect(target)
                .await
                .is_err();

            let connecting = Connecting {
                dns: dns.clone(),
                clock,
                delays: Delays::default(),
            };
            let egress = |local_device| Egress {
                local_ipv4: addresses.ipv4,
                local_ipv6: addresses.ipv6,
                local_device,
                proxy: None,
            };
            let bound_client = bound_device(
                &http_client::connect("127.0.0.1", target.port(), &egress(device), &connecting)
                    .await
                    .unwrap()
                    .stream,
            );
            let unbound_client = bound_device(
                &http_client::connect("127.0.0.1", target.port(), &egress(none), &connecting)
                    .await
                    .unwrap()
                    .stream,
            );

            let _ = accept.await;
            (
                bound_relay,
                unbound_relay,
                missing_relay,
                bound_client,
                unbound_client,
            )
        });

    assert_eq!(
        (
            interface,
            bound_relay.as_deref(),
            unbound_relay.as_deref(),
            missing_relay,
            bound_client.as_deref(),
            unbound_client.as_deref(),
        ),
        (Some("lo"), Some("lo"), None, true, Some("lo"), None)
    );
}

fn bound_device(stream: &tokio::net::TcpStream) -> Option<String> {
    socket2::SockRef::from(stream)
        .device()
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}
