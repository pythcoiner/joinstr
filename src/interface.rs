use std::{fmt::Display, thread::sleep, time::Duration};

use bitcoin::{address::NetworkUnchecked, Address, Network};
use simple_nostr_client::nostr::Keys;

use crate::{
    electrum::Client,
    joinstr::Joinstr,
    nostr::{sync::NostrClient, Pool},
    signer::{Coin, CoinPath, WpkhHotSigner},
    utils::now,
};

pub enum Error {
    Unknown,
    NostrClient(crate::nostr::error::Error),
    SerdeJson(serde_json::Error),
    Joinstr(crate::joinstr::Error),
    Signer(crate::signer::Error),
    Electrum(crate::electrum::Error),
}

impl From<crate::nostr::error::Error> for Error {
    fn from(value: crate::nostr::error::Error) -> Self {
        Self::NostrClient(value)
    }
}

impl From<crate::joinstr::Error> for Error {
    fn from(value: crate::joinstr::Error) -> Self {
        Self::Joinstr(value)
    }
}

impl From<crate::signer::Error> for Error {
    fn from(value: crate::signer::Error) -> Self {
        Self::Signer(value)
    }
}

impl From<crate::electrum::Error> for Error {
    fn from(value: crate::electrum::Error) -> Self {
        Self::Electrum(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::SerdeJson(value)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unknown => write!(f, "Unknown error!"),
            Error::NostrClient(e) => write!(f, "NostrClient error: {:?}", e),
            Error::SerdeJson(e) => write!(f, "serde_json error: {:?}", e),
            Error::Joinstr(e) => write!(f, "Joinstr error: {:?}", e),
            Error::Signer(e) => write!(f, "Signer error: {:?}", e),
            Error::Electrum(e) => write!(f, "Electrum error: {:?}", e),
        }
    }
}

pub struct PoolConfig {
    pub denomination: f64,
    pub fee: u32,
    pub max_duration: u64,
    pub peers: usize,
    pub network: Network,
}

pub struct PeerConfig {
    pub mnemonics: String,
    pub electrum_address: String,
    pub electrum_port: u16,
    pub input: String,
    pub output: String,
    pub relay: String,
}

/// List available coins
// FIXME: this function is a ugly+ineficient hack, we should use
// the electrum notification mechanism and let consumer poll our
// static/cached state
pub fn list_coins(
    mnemonics: String,
    electrum_address: String,
    electrum_port: u16,
    range: (u32, u32),
    network: Network,
) -> Result<Vec<Coin>, Error> {
    let mut signer = WpkhHotSigner::new_from_mnemonics(network, &mnemonics)?;
    let client = Client::new(&electrum_address, electrum_port)?;
    signer.set_client(client);

    for i in range.0..range.1 {
        let recv = CoinPath::new(0, i);
        let change = CoinPath::new(1, i);
        let _ = signer.get_coins_at(recv);
        let _ = signer.get_coins_at(change);
    }

    let coins = signer.list_coins().into_iter().map(|c| c.1).collect();

    Ok(coins)
}

/// Initiate and participate to a coinjoin
///
/// # Arguments
/// * `config` - configuration of the pool to initiate
/// * `peer` - information about the peer
///
pub fn initiate_coinjoin(config: PoolConfig, peer: PeerConfig) -> Result<String /* Txid */, Error> {
    let (url, port) = (peer.electrum_address, peer.electrum_port);
    let mut initiator = Joinstr::new_initiator(
        Keys::generate(),
        peer.relay.clone(),
        (&url, port),
        config.network,
        "initiator",
    )?
    .denomination(config.denomination)?
    .fee(config.fee)?
    .simple_timeout(now() + config.max_duration)?
    .min_peers(config.peers)?;

    let mut signer = WpkhHotSigner::new_from_mnemonics(config.network, &peer.mnemonics)?;
    let client = Client::new(&url, port)?;
    signer.set_client(client);

    let addr: Address<NetworkUnchecked> = serde_json::from_str(&peer.output)?;
    let coin: Coin = serde_json::from_str(&peer.input)?;

    initiator.set_coin(coin)?;
    initiator.set_address(addr)?;

    initiator.start_coinjoin(None, Some(&signer))?;

    let txid = initiator
        .final_tx()
        .expect("coinjoin success")
        .compute_txid()
        .to_string();

    Ok(txid)
}

/// List available pools
///
/// # Arguments
/// * `back` - how many second back look in the past
/// * `timeout` - how many microseconds we will wait before fetching relay notifications
/// * `relay` - the relay url, must start w/ `wss://` or `ws://`
///
/// # Returns a [`Vec`]  of [`String`] containing a json serialization of a [`Pool`]
pub fn list_pools(back: u64, timeout: u64, relay: String) -> Result<Vec<String /* Pool */>, Error> {
    let mut pools = Vec::new();
    let mut pool_listener = NostrClient::new("pool_listener")
        .relay(relay)?
        .keys(Keys::generate())?;
    pool_listener.connect_nostr().unwrap();
    // subscribe to 2020 event up to 1 day back in time
    pool_listener.subscribe_pools(back).unwrap();

    sleep(Duration::from_micros(timeout));

    while let Some(pool) = pool_listener.receive_pool_notification()? {
        let str = serde_json::to_string(&pool)?;
        pools.push(str)
    }

    Ok(pools)
}

/// Try to join an already initiated coinjoin
///
/// # Arguments
/// * `pool` - [`String`] containing a json serialization of a [`Pool`]
/// * `peer` - information about the peer
///
pub fn join_coinjoin(
    pool: String, /* Pool */
    peer: PeerConfig,
) -> Result<String /* Txid */, Error> {
    let pool: Pool = serde_json::from_str(&pool)?;
    let (url, port) = (peer.electrum_address, peer.electrum_port);
    let addr: Address<NetworkUnchecked> = serde_json::from_str(&peer.output)?;
    let coin: Coin = serde_json::from_str(&peer.input)?;
    let mut joinstr_peer = Joinstr::new_peer_with_electrum(
        peer.relay.clone(),
        &pool,
        (&url, port),
        coin,
        addr,
        pool.network,
        "peer",
    )?;

    let mut signer = WpkhHotSigner::new_from_mnemonics(pool.network, &peer.mnemonics)?;
    let client = Client::new(&url, port)?;
    signer.set_client(client);

    joinstr_peer.start_coinjoin(None, Some(&signer))?;

    let txid = joinstr_peer
        .final_tx()
        .expect("coinjoin success")
        .compute_txid()
        .to_string();

    Ok(txid)
}
