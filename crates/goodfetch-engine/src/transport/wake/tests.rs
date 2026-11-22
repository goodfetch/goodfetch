//! The Politeness file's wake endpoints, two engines' on one host.

use super::{listen_wakes, wake};
use crate::transport::socket_folder;

/// Two engines on one host, each with its wake endpoint in a folder of its
/// own, both listen and are each woken through their own: on Windows too,
/// where a named pipe's name is global to the host.
#[test]
fn each_of_two_wake_endpoints_on_one_host_receives_the_wakes_sent_to_it() {
    let received = goodfetch_test_support::real_time_runtime().block_on(async {
        let first_folder = socket_folder("goodfetch-wake-", "wake.sock").unwrap();
        let second_folder = socket_folder("goodfetch-wake-", "wake.sock").unwrap();
        let (mut first, first_endpoint) =
            listen_wakes(&first_folder.path().join("wake.sock")).unwrap();
        let (mut second, second_endpoint) =
            listen_wakes(&second_folder.path().join("wake.sock")).unwrap();

        let (to_first, to_second, ()) = tokio::join!(first.accept(), second.accept(), async {
            wake(&second_endpoint, b"to the second").await.unwrap();
            wake(&first_endpoint, b"to the first").await.unwrap();
        });
        (to_first.unwrap(), to_second.unwrap())
    });

    assert_eq!(
        received,
        (b"to the first".to_vec(), b"to the second".to_vec())
    );
}
