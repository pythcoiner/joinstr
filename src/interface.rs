use bitcoin::Network;

pub enum Error {
    //
}

pub struct PoolConfig {
    pub denomination: f64,
    pub fee: u32,
    pub max_duration: u32,
    pub peers: u8,
    pub network: Network,
}

pub struct PeerConfig {
    pub outpoint: String,
    pub electrum: String,
    pub mnemonics: String,
    pub address: String,
    pub relay: String,
}

/// Initiate and participate to a coinjoin
///
/// # Arguments
/// * `config` - configuration of the pool to initiate
/// * `peer` - information about the peer
///
pub fn initiate_coinjoin(
    _config: PoolConfig,
    _peer: PeerConfig,
) -> Result<String /* Txid */, Error> {
    // TODO:
    Ok(String::new())
}

/// List available pools
///
/// # Arguments
/// * `back` - how many second back look in the past
/// * `timeout` - how many second we will wait before fetching relay notifications
/// * `relay` - the relay url, must start w/ `wss://` or `ws://`
///
/// # Returns a [`Vec`]  of [`String`] containing a json serialization of a [`Pool`]
pub fn list_pool(
    _back: u64,
    _timeout: u64,
    _relay: String,
) -> Result<Vec<String /* Pool */>, Error> {
    // TODO:
    Ok(Vec::new())
}

/// Try to join an already initiated coinjoin
///
/// # Arguments
/// * `pool` - [`String`] containing a json serialization of a [`Pool`]
/// * `peer` - information about the peer
///
pub fn join_coinjoin(
    _pool: String, /* Pool */
    _peer: PeerConfig,
) -> Result<String /* Txid */, Error> {
    // TODO:
    Ok(String::new())
}
