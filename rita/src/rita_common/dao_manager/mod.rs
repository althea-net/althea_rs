// Manages subnet DAO membership, DAOManager mantains a cache of subnet DAO entries.
// The workflow goes as follows, an actor message DAOCheck is sent to DAOManager
// if the identity is not on the DAO it will run a callback to tunnel manager to remove
// that tunnel from operation. If the identity is on the DAO it will do nothing.
// Entires from the DAO are cached for a configurable amount of time, this may of course
// have the effect of adding someone to the DAO taking time to kick in.

use actix::prelude::*;
use actix_web::client::Connection;
use actix_web::error::JsonPayloadError;
use actix_web::*;
use failure::Error;
use futures::future;
use futures::future::join_all;
use futures::Future;
use std::collections::HashMap;
use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::Instant;
use tokio::net::TcpStream as TokioTcpStream;

use althea_types::EthAddress;
use althea_types::Identity;
use num256::Uint256;
use settings::RitaCommonSettings;

use SETTING;

// A json object specifcally for the web3 function
// call response we expect from the SubnetDAO contract
#[derive(Deserialize)]
struct web3_response {
    jsonrpc: String,
    id: u32,
    result: Uint256,
}

pub struct DAOManager {
    entries: HashMap<Identity, Vec<DAOEntry>>,
}

impl Actor for DAOManager {
    type Context = Context<Self>;
}
impl Supervised for DAOManager {}
impl SystemService for DAOManager {
    fn service_started(&mut self, _ctx: &mut Context<Self>) {
        info!("Tunnel manager started");
    }
}

impl Default for DAOManager {
    fn default() -> DAOManager {
        DAOManager::new()
    }
}

impl DAOManager {
    fn new() -> DAOManager {
        DAOManager {
            entries: HashMap::<Identity, Vec<DAOEntry>>::new(),
        }
    }
}

pub struct DAOEntry {
    on_list: bool,
    dao_address: EthAddress,
    id: Identity,
    last_updated: Instant,
}

pub struct DAOCheck(Identity);
impl Message for DAOCheck {
    type Result = ();
}

impl Handler<DAOCheck> for DAOManager {
    type Result = ();

    fn handle(&mut self, msg: DAOCheck, _: &mut Context<Self>) -> Self::Result {
        let their_id = msg.0;
        check_cache(their_id, &self.entries);
    }
}

/// Called by returning DAO requests, sends a message to TunnelManager
pub struct CacheCallback((Identity, EthAddress, web3_response));
impl Message for CacheCallback {
    type Result = ();
}

impl Handler<CacheCallback> for DAOManager {
    type Result = ();

    fn handle(&mut self, msg: CacheCallback, _: &mut Context<Self>) -> Self::Result {
        let their_id = (msg.0).0;
        let dao_address = (msg.0).1;
        let response = (msg.0).2;

        let entry = DAOEntry {
            on_list: false,
            dao_address: dao_address,
            id: their_id,
            last_updated: Instant::now(),
        };

        if response.result != Uint256::zero() {
            // send on dao message
            match self.entries.get(&their_id) {}
        } else {
            // send not on dao message
        }
    }
}

/// True if timestamp does not need to be updated
fn timer_check(timestamp: Instant) -> bool {
    if Instant::now() - timestamp < SETTING.get_dao().cache_timeout {
        true
    } else {
        false
    }
}

/// Checks is an identity is in at least one of the set of DAO's we are a member of.
/// will check the cache first before going out and updating via web3
fn check_cache(their_id: Identity, entries: &HashMap<Identity, Vec<DAOEntry>>) -> () {
    // we don't care about subnet DAO's, short circuit.
    if !SETTING.get_dao().dao_enforcement {
        trace!("DAO enforcement disabled DAOMAnager doing nothing!");
        return ();
    }
    // TODO when we start enforcing dao state more strictly we will need
    // to detect when we are bootstrapping and explicitly allow everyone

    match entries.get(&their_id) {
        // Cache hit
        Some(membership_list) => {
            for entry in membership_list.iter() {
                if entry.on_list && timer_check(entry.last_updated) {
                    trace!(
                        "{:?} is on the SubnetDAO {:?}",
                        their_id.clone(),
                        entry.dao_address
                    );
                // send approval
                } else if !timer_check(entry.last_updated) {
                    get_membership(&entry.dao_address, entry.id.clone());
                }
            }
            trace!("{:?} is not on any SubnetDAO", their_id);
            // send rejection
        }
        // Cache miss, do a lookup for all DAO's
        None => {
            for dao in SETTING.get_dao().dao_addresses.iter() {
                get_membership(dao, their_id.clone());
            }
        }
    }
}

fn get_membership(dao_address: &EthAddress, target: Identity) -> () {
    let (ip, port) = get_web3_server();
    let endpoint = format!("http://[{}]:{}/", ip, port);
    let socket: SocketAddr = endpoint.parse().expect("Invalid DAO fullnode!");

    let ip_bytes = match target.mesh_ip {
        IpAddr::V6(ip) => ip.octets(),
        _ => panic!("MeshIP must be ipv6 and is not!"),
    };
    let mut full_bytes: [u8; 32] = [0; 32];
    let mut i = 0;
    for byte in ip_bytes.iter() {
        full_bytes[i] = byte.clone();
        i = i + 1;
    }

    let call_args: Uint256 = full_bytes.into();
    let func_call = format!("{{'jsonrpc':'2.0','method':'eth_call','params':[{{'to': '{:x}', 'data': '{:x}'}}, 'latest'],'id':1}}", dao_address, call_args);

    let stream = TokioTcpStream::connect(&socket);

    let res = stream.then(move |stream| {
        let stream = stream.expect("Error opening connection to DAO node!");
        client::post(&endpoint)
            .timeout(Duration::from_secs(8))
            .with_connection(Connection::from_stream(stream))
            .json(func_call)
            .unwrap()
            .send()
            .then(|response| {
                if response.is_err() {
                    trace!("Got {:?} from full node DAO request", response);
                    return Ok(());
                }
                response
                    .unwrap()
                    .json()
                    .from_err()
                    .and_then(|val: web3_response| {
                        DAOManager::from_registry().do_send(CacheCallback((
                            target,
                            dao_address.clone(),
                            val,
                        )));
                        Ok(())
                    });
                Ok(())
            })
    });
    Arbiter::spawn(res);
}

/// Checks the list of full nodes, panics if none exist, if there exists
/// one or more it rotates the entires such that requests are load balanced
/// evenly. TODO sort before writing in settings to reduce flash wear
fn get_web3_server() -> (IpAddr, u16) {
    if SETTING.get_dao().node_list.len() == 0 {
        panic!("DAO enforcement enabled but not DAO's configured!");
    }

    let node_list = &mut SETTING.get_dao_mut().node_list;
    let ret = node_list.pop().unwrap();
    node_list.push(ret.clone());

    ret
}
