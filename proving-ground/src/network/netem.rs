//! `tc netem` on Linux's loopback interface, for published figures: a `prio`
//! root with a fourth band, which the default priority map never fills, holds
//! a `netem` of half the round trip and the declared loss, and `u32` filters
//! send it only the packets to or from one port, IPv4 and IPv6. Loopback
//! carries each direction once, so the two halves add the whole round trip,
//! and no other loopback traffic is shaped. Changing `lo`'s queueing
//! discipline needs `CAP_NET_ADMIN`, and it takes `lo`'s root: one shaped
//! port per network namespace at a time.

use anyhow::{Context as _, Result, bail};

use super::Profile;

/// The interface every loopback connection crosses.
const DEVICE: &str = "lo";

/// The queueing discipline on `lo`'s root, while it is shaped.
pub(crate) struct Netem(());

impl Netem {
    /// The `tc` commands that shape `port` with `profile`, each its
    /// arguments.
    pub(crate) fn plan(profile: &Profile, port: u16) -> Vec<Vec<String>> {
        let loss = if profile.loss.is_some() {
            format!(" loss {}%", profile.loss)
        } else {
            String::new()
        };
        let netem = format!(
            "qdisc add dev {DEVICE} parent 1:4 handle 40: netem delay {}us{loss}",
            (profile.round_trip / 2).as_micros()
        );
        let filter = |protocol: &str, matched: &str, priority: u8, field: &str| {
            format!(
                "filter add dev {DEVICE} parent 1: protocol {protocol} prio {priority} u32 match {matched} {field} {port} 0xffff flowid 1:4"
            )
        };
        [
            format!("qdisc add dev {DEVICE} root handle 1: prio bands 4"),
            netem,
            filter("ip", "ip", 1, "sport"),
            filter("ip", "ip", 1, "dport"),
            filter("ipv6", "ip6", 2, "sport"),
            filter("ipv6", "ip6", 2, "dport"),
        ]
        .iter()
        .map(|command| command.split(' ').map(str::to_owned).collect())
        .collect()
    }

    /// Shapes `port`; a command that fails undoes what came before it.
    pub(crate) async fn apply(profile: &Profile, port: u16) -> Result<Self> {
        for arguments in Self::plan(profile, port) {
            if let Err(error) = tc(&arguments).await {
                let _ = tc(&removal()).await;
                return Err(error);
            }
        }
        Ok(Self(()))
    }

    /// Removes the shaping.
    pub(crate) async fn remove(self) -> Result<()> {
        tc(&removal()).await
    }
}

/// The `tc` command that removes `lo`'s root queueing discipline.
fn removal() -> Vec<String> {
    ["qdisc", "del", "dev", DEVICE, "root"]
        .map(str::to_owned)
        .to_vec()
}

/// Runs `tc` and fails with what it printed when it fails.
async fn tc(arguments: &[String]) -> Result<()> {
    let output = tokio::process::Command::new("tc")
        .args(arguments)
        .kill_on_drop(true)
        .output()
        .await
        .context("running `tc`, which `tc netem` needs (Linux's iproute2)")?;
    if !output.status.success() {
        bail!(
            "`tc {}` failed ({}): {}",
            arguments.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Netem;
    use crate::network::{Loss, Profile};

    fn commands(plan: &[Vec<String>]) -> Vec<String> {
        plan.iter().map(|arguments| arguments.join(" ")).collect()
    }

    #[test]
    fn netem_holds_half_the_round_trip_and_the_loss_for_one_port() {
        let profile = Profile::new(
            "rtt-20",
            Duration::from_millis(20),
            "0.5".parse::<Loss>().unwrap(),
        );

        let plan = Netem::plan(&profile, 8443);

        assert_eq!(
            commands(&plan),
            [
                "qdisc add dev lo root handle 1: prio bands 4",
                "qdisc add dev lo parent 1:4 handle 40: netem delay 10000us loss 0.5%",
                "filter add dev lo parent 1: protocol ip prio 1 u32 match ip sport 8443 0xffff flowid 1:4",
                "filter add dev lo parent 1: protocol ip prio 1 u32 match ip dport 8443 0xffff flowid 1:4",
                "filter add dev lo parent 1: protocol ipv6 prio 2 u32 match ip6 sport 8443 0xffff flowid 1:4",
                "filter add dev lo parent 1: protocol ipv6 prio 2 u32 match ip6 dport 8443 0xffff flowid 1:4",
            ]
        );
    }

    #[test]
    fn netem_without_loss_sets_only_the_delay() {
        let profile = Profile::new("rtt-300", Duration::from_millis(300), Loss::default());

        let plan = Netem::plan(&profile, 1);

        assert_eq!(
            commands(&plan)[1],
            "qdisc add dev lo parent 1:4 handle 40: netem delay 150000us"
        );
    }
}
