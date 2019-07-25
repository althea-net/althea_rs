use crate::rita_common::oracle::update_nonce;
use crate::rita_common::rita_loop::get_web3_server;
use crate::rita_common::token_bridge::TokenBridge;
use crate::rita_common::token_bridge::Withdraw;
use crate::SETTING;
use ::actix::SystemService;
use ::actix_web::http::StatusCode;
use ::actix_web::HttpResponse;
use ::actix_web::Path;
use ::settings::RitaCommonSettings;
use althea_types::SystemChain;
use clarity::{Address, Transaction};
use failure::Error;
use futures::{future, Future};
use std::boxed::Box;
use std::time::Duration;
use web30::client::Web3;

pub const WITHDRAW_TIMEOUT: Duration = Duration::from_secs(10);

pub fn withdraw(path: Path<(Address, u64)>) -> Box<dyn Future<Item = HttpResponse, Error = Error>> {
    let address = path.0;
    let amount = path.1;
    debug!("/withdraw/{:#x}/{} hit", address, amount);
    let payment_settings = SETTING.get_payment();
    let system_chain = payment_settings.system_chain;
    let withdraw_chain = payment_settings.withdraw_chain;
    drop(payment_settings);

    match (system_chain, withdraw_chain) {
        (SystemChain::Ethereum, SystemChain::Ethereum) => eth_compatable_withdraw(address, amount),
        (SystemChain::Rinkeby, SystemChain::Rinkeby) => eth_compatable_withdraw(address, amount),
        (SystemChain::Xdai, SystemChain::Xdai) => eth_compatable_withdraw(address, amount),
        (SystemChain::Xdai, SystemChain::Ethereum) => xdai_to_eth_withdraw(address, amount),
        (_, _) => Box::new(future::ok(
            HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                .into_builder()
                .json(format!(
                    "System chain is {} but withdraw chain is {}, withdraw impossible!",
                    system_chain, withdraw_chain
                )),
        )),
    }
}

/// Withdraw for eth compatible chains
fn eth_compatable_withdraw(
    address: Address,
    amount: u64,
) -> Box<dyn Future<Item = HttpResponse, Error = Error>> {
    let full_node = get_web3_server();
    let web3 = Web3::new(&full_node, WITHDRAW_TIMEOUT);
    let payment_settings = SETTING.get_payment();
    let our_address = match payment_settings.eth_address {
        Some(address) => address,
        None => {
            return Box::new(future::ok(
                HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                    .into_builder()
                    .json("No Address configured, withdraw impossible!"),
            ))
        }
    };

    let tx = Transaction {
        nonce: payment_settings.nonce.clone(),
        gas_price: payment_settings.gas_price.clone(),
        gas_limit: "21000".parse().unwrap(),
        to: address,
        value: amount.into(),
        data: Vec::new(),
        signature: None,
    };
    let transaction_signed = tx.sign(
        &payment_settings
            .eth_private_key
            .expect("No private key configured!"),
        payment_settings.net_version,
    );

    let transaction_bytes = match transaction_signed.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => {
            return Box::new(future::ok(
                HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                    .into_builder()
                    .json(format!("Transaction to bytes failed! {:?}", e)),
            ));
        }
    };

    let transaction_status = web3.eth_send_raw_transaction(transaction_bytes);

    Box::new(transaction_status.then(move |result| match result {
        Ok(tx_id) => Box::new(future::ok({
            SETTING.get_payment_mut().nonce += 1u64.into();
            HttpResponse::Ok().json(format!("txid:{:#066x}", tx_id))
        })),
        Err(e) => {
            update_nonce(our_address, &web3, full_node);
            if e.to_string().contains("nonce") {
                Box::new(future::ok(
                    HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                        .into_builder()
                        .json(format!("The nonce was not updated, try again {:?}", e)),
                ))
            } else {
                Box::new(future::ok(
                    HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                        .into_builder()
                        .json(format!("Full node failed to send transaction! {:?}", e)),
                ))
            }
        }
    }))
}

/// Cross chain bridge withdraw from Xdai -> ETH
fn xdai_to_eth_withdraw(
    address: Address,
    amount: u64,
) -> Box<dyn Future<Item = HttpResponse, Error = Error>> {
    Box::new(
        TokenBridge::from_registry()
            .send(Withdraw {
                to: address,
                amount: amount.into(),
            })
            .then(|val| match val {
                Ok(_) => Box::new(future::ok(
                    HttpResponse::Ok().json("View endpoints for progress"),
                )),
                Err(e) => Box::new(future::ok(
                    HttpResponse::new(StatusCode::from_u16(504u16).unwrap())
                        .into_builder()
                        .json(format!("{:?}", e)),
                )),
            }),
    )
}
