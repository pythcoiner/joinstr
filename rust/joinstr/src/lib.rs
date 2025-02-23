#![allow(dead_code)]
pub mod coinjoin;
pub mod electrum;
pub mod interface;
pub mod joinstr;
pub mod nostr;
pub mod signer;
pub mod utils;
pub use bip39;
pub use log;
pub use miniscript;

#[cfg(feature = "async")]
pub use nostr_sdk;

use serde::Serialize;
use std::{
    ffi::{c_char, CStr, CString},
    ptr::null,
};

fn serialize_to_cstring<T>(value: T) -> Result<CString, Error>
where
    T: Serialize,
{
    match serde_json::to_string(&value) {
        Ok(v) => match CString::new(v) {
            Ok(s) => Ok(s),
            Err(_) => Err(Error::Json),
        },
        Err(_) => Err(Error::CString),
    }
}

macro_rules! to_string {
    ($value:expr, $t:ty) => {{
        let cstr = unsafe { CStr::from_ptr($value) };
        match cstr.to_str() {
            Ok(str_slice) => str_slice.to_owned(),
            Err(_) => return <$t>::error(Error::CString),
        }
    }};

    ($value:expr) => {{
        let cstr = unsafe { CStr::from_ptr($value) };
        match cstr.to_str() {
            Ok(str_slice) => str_slice.to_owned(),
            Err(_) => return Err(Error::CString),
        }
    }};
}

macro_rules! result {
    ($t:ty, $inner:ident) => {
        impl $t {
            pub fn ok($inner: CString) -> Self {
                Self {
                    $inner: $inner.into_raw(),
                    error: Error::None,
                }
            }
            pub fn error(e: Error) -> Self {
                Self {
                    $inner: null(),
                    error: e,
                }
            }
        }
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum Network {
    /// Mainnet Bitcoin.
    Bitcoin,
    /// Bitcoin's testnet network.
    Testnet,
    /// Bitcoin's signet network.
    Signet,
    /// Bitcoin's regtest network.
    Regtest,
}

impl Network {
    pub fn to_rust_bitcoin(self) -> bitcoin::Network {
        match self {
            Network::Bitcoin => bitcoin::Network::Bitcoin,
            Network::Testnet => bitcoin::Network::Testnet,
            Network::Signet => bitcoin::Network::Signet,
            Network::Regtest => bitcoin::Network::Regtest,
        }
    }
}

#[repr(C)]
pub struct PoolConfig {
    pub denomination: f64,
    pub fee: u32,
    pub max_duration: u64,
    pub peers: u8,
    pub network: Network,
}

impl TryFrom<PoolConfig> for interface::PoolConfig {
    type Error = Error;

    fn try_from(value: PoolConfig) -> Result<Self, Self::Error> {
        Ok(interface::PoolConfig {
            denomination: value.denomination,
            fee: value.fee,
            max_duration: value.max_duration,
            peers: value.peers.into(),
            network: value.network.to_rust_bitcoin(),
        })
    }
}

#[repr(C)]
pub struct PeerConfig {
    pub electrum_address: *const c_char,
    pub electrum_port: u16,
    pub mnemonics: *const c_char,
    pub input: *const c_char,
    pub output: *const c_char,
    pub relay: *const c_char,
}

impl TryFrom<PeerConfig> for interface::PeerConfig {
    type Error = Error;

    fn try_from(value: PeerConfig) -> Result<Self, Self::Error> {
        Ok(interface::PeerConfig {
            mnemonics: to_string!(value.mnemonics),
            electrum_address: to_string!(value.electrum_address),
            electrum_port: value.electrum_port,
            input: to_string!(value.input),
            output: to_string!(value.output),
            relay: to_string!(value.relay),
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum Error {
    None = 0,
    Tokio,
    CastString,
    Json,
    CString,
    ListPools,
    ListCoins,
    InitiateConjoin,
    SerdeJson,
    PoolConfig,
    PeerConfig,
}

#[repr(C)]
pub struct Pools {
    pools: *const c_char,
    error: Error,
}

result!(Pools, pools);

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn list_pools(back: u64, timeout: u64, relay: *const c_char) -> Pools {
    let relay = to_string!(relay, Pools);
    match interface::list_pools(back, timeout, relay) {
        Ok(p) => match serialize_to_cstring(p) {
            Ok(r) => Pools::ok(r),
            Err(_) => Pools::error(Error::ListPools),
        },
        Err(_) => Pools::error(Error::ListPools),
    }
}

#[repr(C)]
pub struct Coins {
    coins: *const c_char,
    error: Error,
}
result!(Coins, coins);

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn list_coins(
    mnemonics: *const c_char,
    addr: *const c_char,
    port: u16,
    network: Network,
    index_min: u32,
    index_max: u32,
) -> Coins {
    let mnemonics = to_string!(mnemonics, Coins);
    let addr = to_string!(addr, Coins);

    match interface::list_coins(
        mnemonics,
        addr,
        port,
        (index_min, index_max),
        network.to_rust_bitcoin(),
    ) {
        Ok(c) => match serialize_to_cstring(c) {
            Ok(r) => Coins::ok(r),
            Err(_) => Coins::error(Error::ListCoins),
        },
        Err(_) => Coins::error(Error::ListCoins),
    }
}

#[repr(C)]
pub struct Txid {
    txid: *const c_char,
    error: Error,
}

result!(Txid, txid);

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn initiate_coinjoin(config: PoolConfig, peer: PeerConfig) -> Txid {
    let pool: interface::PoolConfig = match config.try_into() {
        Ok(c) => c,
        Err(_) => {
            return Txid::error(Error::PoolConfig);
        }
    };
    let peer: interface::PeerConfig = match peer.try_into() {
        Ok(c) => c,
        Err(_) => {
            return Txid::error(Error::PeerConfig);
        }
    };
    match interface::initiate_coinjoin(pool, peer) {
        Ok(c) => match serialize_to_cstring(c) {
            Ok(r) => Txid::ok(r),
            Err(_) => Txid::error(Error::InitiateConjoin),
        },
        Err(_) => Txid::error(Error::InitiateConjoin),
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn join_coinjoin(pool: *const c_char, peer: PeerConfig) -> Txid {
    let pool = to_string!(pool, Txid);
    let peer: interface::PeerConfig = match peer.try_into() {
        Ok(c) => c,
        Err(_) => {
            return Txid::error(Error::PeerConfig);
        }
    };
    match interface::join_coinjoin(pool, peer) {
        Ok(c) => match serialize_to_cstring(c) {
            Ok(r) => Txid::ok(r),
            Err(_) => Txid::error(Error::InitiateConjoin),
        },
        Err(_) => Txid::error(Error::InitiateConjoin),
    }
}
