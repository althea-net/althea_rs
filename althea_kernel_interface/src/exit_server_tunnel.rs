use super::{KernelInterface, KernelInterfaceError};
use althea_types::WgKey;
use failure::Error;
use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ExitClient {
    pub internal_ip: Ipv4Addr,
    pub internal_ipv6: Ipv6Addr,
    pub public_key: WgKey,
    pub mesh_ip: Ipv6Addr,
    pub port: u16,
}

impl dyn KernelInterface {
    pub fn set_exit_wg_config(
        &self,
        clients: &HashSet<ExitClient>,
        client_netmaskv6: u8,
        listen_port: u16,
        private_key_path: &str,
    ) -> Result<(), Error> {
        let command = "wg".to_string();

        let mut args = Vec::new();
        args.push("set".into());
        args.push("wg_exit".into());
        args.push("listen-port".into());
        args.push(format!("{}", listen_port));
        args.push("private-key".into());
        args.push(private_key_path.to_string());

        let mut client_pubkeys = HashSet::new();

        for c in clients.iter() {
            args.push("peer".into());
            args.push(format!("{}", c.public_key));
            args.push("endpoint".into());
            args.push(format!("[{}]:{}", c.mesh_ip, c.port));
            args.push("allowed-ips".into());
            args.push(format!("{},", c.internal_ip));
            args.push(format!("{}/{}", c.internal_ipv6, client_netmaskv6));
            args.push("persistent-keepalive".into());
            args.push("5".into());

            client_pubkeys.insert(c.public_key.clone());
        }

        let arg_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        self.run_command(&command, &arg_str[..])?;

        // Assign routes to all the client subnets
        // we don't clean these up when a client is removed becuase they
        // won't be routable anyways and if a new client is created and assigned
        // this address it will route correctly to the new client
        for c in clients.iter() {
            let command = "ip";
            let route_args = [
                "route",
                "add",
                &format!("{}/{}", c.internal_ipv6, client_netmaskv6),
                "via",
                &c.internal_ipv6.to_string(),
                "dev",
                "wg_exit",
            ];
            let _res = self.run_command(command, &route_args);
        }

        let wg_peers = self.get_peers("wg_exit")?;
        info!("wg_exit has {} peers", wg_peers.len());
        for i in wg_peers {
            if !client_pubkeys.contains(&i) {
                warn!("Removing no longer authorized peer {}", i);
                self.run_command(
                    "wg",
                    &["set", "wg_exit", "peer", &format!("{}", i), "remove"],
                )?;
            }
        }

        // setup traffic classes for enforcement with flow id's derived from the ip
        // only get the flows list once
        let flows = self.get_flows("wg_exit")?;
        for c in clients.iter() {
            let addr = c.internal_ip;
            if !self.has_flow_bulk(&addr, &flows) {
                self.create_flow_by_ip("wg_exit", &addr)?
            }
        }

        Ok(())
    }

    /// Performs the one time startup tasks for the rita_exit clients loop
    pub fn one_time_exit_setup(
        &self,
        local_ip: &Ipv4Addr,
        local_ipv6: &Ipv6Addr,
        netmask: u8,
        client_netmask_v6: u8,
    ) -> Result<(), Error> {
        let _output = self.run_command(
            "ip",
            &[
                "address",
                "add",
                &format!("{}/{}", local_ip, netmask),
                "dev",
                "wg_exit",
            ],
        )?;
        let _output = self.run_command(
            "ip",
            &[
                "address",
                "add",
                &format!("{}/{}", local_ipv6, client_netmask_v6),
                "dev",
                "wg_exit",
            ],
        )?;

        let output = self.run_command("ip", &["link", "set", "dev", "wg_exit", "mtu", "1340"])?;
        if !output.stderr.is_empty() {
            return Err(KernelInterfaceError::RuntimeError(format!(
                "received error adding wg link: {}",
                String::from_utf8(output.stderr)?
            ))
            .into());
        }

        let output = self.run_command("ip", &["link", "set", "dev", "wg_exit", "up"])?;
        if !output.stderr.is_empty() {
            return Err(KernelInterfaceError::RuntimeError(format!(
                "received error setting wg interface up: {}",
                String::from_utf8(output.stderr)?
            ))
            .into());
        }

        // this creates the root classful htb limit for which we will make
        // subclasses to enforce payment
        if !self.has_limit("wg_exit")? {
            info!("Setting up root HTB qdisc, this should only run once");
            self.create_root_classful_limit("wg_exit")
                .expect("Failed to setup root HTB qdisc!");
        }

        Ok(())
    }

    pub fn setup_nat(&self, external_interface: &str) -> Result<(), Error> {
        self.add_iptables_rule(
            "iptables",
            &[
                "-w",
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-o",
                external_interface,
                "-j",
                "MASQUERADE",
            ],
        )?;

        self.add_iptables_rule(
            "iptables",
            &[
                "-w",
                "-t",
                "filter",
                "-A",
                "FORWARD",
                "-o",
                external_interface,
                "-i",
                "wg_exit",
                "-j",
                "ACCEPT",
            ],
        )?;

        self.add_iptables_rule(
            "iptables",
            &[
                "-w",
                "-t",
                "filter",
                "-A",
                "FORWARD",
                "-o",
                "wg_exit",
                "-i",
                external_interface,
                "-m",
                "state",
                "--state",
                "RELATED,ESTABLISHED",
                "-j",
                "ACCEPT",
            ],
        )?;

        Ok(())
    }
}
