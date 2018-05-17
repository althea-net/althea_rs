use actix::prelude::*;

use reqwest;

use althea_types::{ExitClientIdentity, Identity};

use settings::{RitaClientSettings, RitaCommonSettings};
use SETTING;

use rita_client::rita_loop::Tick;
use rita_client::traffic_watcher::{TrafficWatcher, Watch};

use althea_types::{ExitServerReply, ExitState};
use failure::Error;
use settings::ExitClientDetails;
use std::net::SocketAddr;
use KI;

fn linux_setup_exit_tunnel() -> Result<(), Error> {
    let exit_client = SETTING.get_exit_client();
    let current_exit = exit_client.get_current_exit().unwrap();
    let details = current_exit.details.as_ref().unwrap();

    KI.setup_wg_if_named("wg_exit")?;
    KI.set_client_exit_tunnel_config(
        SocketAddr::new(current_exit.ip, details.wg_exit_port),
        details.wg_public_key.clone(),
        SETTING.get_network().wg_private_key_path.clone(),
        SETTING.get_exit_client().wg_listen_port,
        details.own_internal_ip,
        details.netmask,
    )?;
    KI.set_route_to_tunnel(&details.server_internal_ip)?;

    let lan_nics = &SETTING.get_exit_tunnel_settings().lan_nics;
    for nic in lan_nics {
        KI.add_client_nat_rules(&nic)?;
    }

    Ok(())
}

fn exit_setup_request(exit: &String) -> Result<(), Error> {
    let exit_response: ExitServerReply = {
        let exits = SETTING.get_exits();
        let current_exit = &exits[exit];
        let exit_server = current_exit.ip;
        let ident = ExitClientIdentity {
            global: SETTING.get_identity(),
            wg_port: SETTING.get_exit_client().wg_listen_port.clone(),
            reg_details: SETTING.get_exit_client().reg_details.clone().unwrap(),
        };

        let endpoint = format!(
            "http://[{}]:{}/setup",
            exit_server, current_exit.registration_port
        );

        trace!("Sending exit setup request to {:?}", endpoint);
        let client = reqwest::Client::new();
        let response = client.post(&endpoint).json(&ident).send();

        response?.json()?
    };

    let mut exits = SETTING.set_exits();

    let current_exit = exits.get_mut(exit).unwrap();

    current_exit.message = exit_response.message.clone();
    current_exit.state = exit_response.state.clone();

    let exit_id = exit_response.identity.clone().unwrap();

    trace!("Got exit setup response {:?}", exit_response.clone());

    current_exit.details = Some(ExitClientDetails {
        own_internal_ip: exit_id.own_local_ip,
        eth_address: exit_id.global.eth_address,
        wg_public_key: exit_id.global.wg_public_key,
        wg_exit_port: exit_id.wg_port,
        server_internal_ip: exit_id.server_local_ip,
        exit_price: exit_id.price,
        netmask: exit_id.netmask,
    });

    Ok(())
}

/// An actor which pays the exit
#[derive(Default)]
pub struct ExitManager;

impl Actor for ExitManager {
    type Context = Context<Self>;
}

impl Supervised for ExitManager {}
impl SystemService for ExitManager {
    fn service_started(&mut self, _ctx: &mut Context<Self>) {
        info!("Exit Manager started");
    }
}

impl Handler<Tick> for ExitManager {
    type Result = Result<(), Error>;

    fn handle(&mut self, _: Tick, _ctx: &mut Context<Self>) -> Self::Result {
        let exit_server = {
            SETTING
                .get_exit_client()
                .get_current_exit()
                .map(|c| c.clone())
        };

        // code that connects to the current exit server
        if let Some(exit) = exit_server {
            if let Some(ref details) = exit.details {
                linux_setup_exit_tunnel().expect("failure setting up exit tunnel");
                TrafficWatcher::from_registry().do_send(Watch(
                    Identity {
                        mesh_ip: exit.ip,
                        wg_public_key: details.wg_public_key.clone(),
                        eth_address: details.eth_address,
                    },
                    details.exit_price,
                ));
            }
        }

        // code that manages requesting details to exits
        let servers = { SETTING.get_exits().clone() };

        for (k, s) in servers {
            match s.state {
                ExitState::Denied | ExitState::Disabled | ExitState::Registered => {}
                ExitState::New | ExitState::Pending => match exit_setup_request(&k) {
                    Ok(_) => {
                        info!("exit request to {} was successful", k);
                    }
                    Err(e) => {
                        info!("exit request to {} failed with {:?}", k, e);
                    }
                },
            }
        }

        Ok(())
    }
}
