mod error;
use backoff::Backoff;
pub use error::Error;

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use miniscript::bitcoin::{Amount, Network};
use simple_nostr_client::nostr::{
    bitcoin::{address::NetworkUnchecked, Address},
    hashes::{sha256, Hash, HashEngine},
    Keys, PublicKey,
};

use crate::{
    coinjoin::CoinJoin,
    nostr::{
        default_version, sync::NostrClient, Credentials, Fee, InputDataSigned, Pool, PoolMessage,
        PoolPayload, PoolType, Timeline, Tor, Vpn,
    },
    signer::{Coin, JoinstrSigner},
    utils::{now, rand_delay},
};

// delay we wait between (non-blocking) polls of a channel
pub const WAIT: u64 = 50;

#[derive(Debug)]
pub struct Joinstr<'a> {
    inner: Arc<Mutex<JoinstrInner<'a>>>,
}

#[derive(Debug)]
pub struct JoinstrInner<'a> {
    pub initiator: bool,
    pub client: NostrClient,
    pub pool: Option<Pool>,
    pub denomination: Option<Amount>,
    pub peers: Option<usize>,
    pub timeout: Option<Timeline>,
    pub relays: Vec<String>,
    pub fee: Option<Fee>,
    pub network: Network,
    pub coinjoin: Option<CoinJoin<'a, crate::electrum::Client>>,
    pub electrum_client: Option<crate::electrum::Client>,
    input: Option<Coin>,
    output: Option<Address>,
    final_tx: Option<miniscript::bitcoin::Transaction>,
}

impl Default for JoinstrInner<'_> {
    fn default() -> Self {
        Self {
            initiator: false,
            client: Default::default(),
            pool: Default::default(),
            denomination: Default::default(),
            peers: Default::default(),
            timeout: Default::default(),
            relays: Default::default(),
            fee: Default::default(),
            network: Network::Bitcoin,
            coinjoin: None,
            electrum_client: None,
            input: None,
            output: None,
            final_tx: None,
        }
    }
}

impl Joinstr<'_> {
    /// Create a new [`Joinstr`] instance
    ///
    /// # Arguments
    /// * `keys` - Nostr keys that will be used for auth to the nostr relay
    /// * `relays` - A list of relays address to connect to
    /// * `name` - Name of the [`Joinstr`] instance (use for debug logs), can
    ///   be an empty &str.
    ///
    /// Note: this instance do not have a bitcoin backend, it then cannot verify
    ///   that coins registered by other peers exists, and that an output is willing to
    ///   do address reuse.
    fn new(keys: Keys, relay: String, name: &str) -> Result<Self, Error> {
        let mut client = NostrClient::new(name).relay(relay.clone())?.keys(keys)?;
        client.connect_nostr()?;
        let relays = vec![relay];
        let inner = Arc::new(Mutex::new(JoinstrInner {
            client,
            relays,
            ..Default::default()
        }));
        Ok(Joinstr { inner })
    }

    /// Create a new [`Joinstr`] instance with a bitcoin backend
    ///
    /// # Arguments
    /// * `keys` - Nostr keys that will be used for auth to the nostr relay
    /// * `relays` - A list of relays address to connect to
    /// * `electrum_server` - A tuple (<address>, <port>)
    /// * `name` - Name of the [`Joinstr`] instance (use for debug logs), can
    ///   be an empty &str.
    fn new_with_electrum(
        keys: Keys,
        relay: String,
        electrum_server: (&str, u16),
        name: &str,
    ) -> Result<Self, Error> {
        let electrum = crate::electrum::Client::new(electrum_server.0, electrum_server.1)?;
        let j = Self::new(keys, relay, name)?;
        j.inner.lock().expect("poisoned").electrum_client = Some(electrum);
        Ok(j)
    }

    /// Create a new [`Joinstr`] instance that have a `Peer` role, this role means
    ///   the pool have already been initited by another peer.
    ///
    /// # Arguments
    /// * `relays` - A list of relays address to connect to
    /// * `pool` - The [`Pool`] struct representing the pool we want to join
    /// * `input` - The transaction input to include in the coinjoin
    /// * `output` - The address we want to receive the coin to
    /// * `network` - The bitcoin network (bitcoin/testnet/signet/regtest)
    /// * `name` - Name of the [`Joinstr`] instance (use for debug logs), can
    ///   be an empty &str.
    ///
    /// Note: this instance do not have a bitcoin backend, it then cannot verify
    ///   that coins registered by other peers exists, and that an output is willing to
    ///   do address reuse.
    pub fn new_peer(
        relay: String,
        pool: &Pool,
        input: Coin,
        output: Address<NetworkUnchecked>,
        network: Network,
        name: &str,
    ) -> Result<Self, Error> {
        let (denomination, fee, timeout, peers) = match &pool.payload {
            None => return Err(Error::PoolPayloadMissing),
            Some(PoolPayload {
                denomination,
                peers,
                timeout,
                fee,
                ..
            }) => {
                let fee = match &fee {
                    Fee::Fixed(f) => *f,
                    Fee::Provider(_) => return Err(Error::FeeProviderNotImplemented),
                };
                let timeout = match timeout {
                    Timeline::Simple(t) => *t,
                    _ => return Err(Error::TimelineNotImplemented),
                };
                (denomination.to_btc(), fee, timeout, *peers)
            }
        };
        let address = match output.is_valid_for_network(network) {
            true => output.assume_checked(),
            false => return Err(Error::WrongAddressNetwork),
        };
        // NOTE: we create a randow key to process pool auth
        // FIXME: is the entropy of the key good enough?
        let peer = Self::new(Keys::generate(), relay, name)?
            .network(network)
            .denomination(denomination)?
            .fee(fee)?
            .simple_timeout(timeout)?
            .min_peers(peers)?;
        let mut inner = peer.inner.lock().expect("poisoned");
        inner.input = Some(input);
        inner.output = Some(address);
        inner.initiator = false;
        drop(inner);
        Ok(peer)
    }

    /// Create a new [`Joinstr`] instance that have a `Peer` role, this role means
    ///   the pool have already been initited by another peer.
    ///
    /// # Arguments
    /// * `relays` - A list of relays address to connect to
    /// * `pool` - The [`Pool`] struct representing the pool we want to join
    /// * `electrum_server` - A tuple (<address>, <port>)
    /// * `input` - The transaction input to include in the coinjoin
    /// * `output` - The address we want to receive the coin to
    /// * `network` - The bitcoin network (bitcoin/testnet/signet/regtest)
    /// * `name` - Name of the [`Joinstr`] instance (use for debug logs), can
    ///   be an empty &str.
    #[allow(clippy::too_many_arguments)]
    pub fn new_peer_with_electrum(
        relay: String,
        pool: &Pool,
        electrum_server: (&str, u16),
        input: Coin,
        output: Address<NetworkUnchecked>,
        network: Network,
        name: &str,
    ) -> Result<Self, Error> {
        let electrum = crate::electrum::Client::new(electrum_server.0, electrum_server.1)?;
        let peer = Self::new_peer(relay, pool, input, output, network, name)?;
        let mut inner = peer.inner.lock().expect("poisoned");
        inner.initiator = false;
        inner.electrum_client = Some(electrum);
        drop(inner);
        Ok(peer)
    }

    /// Create a new [`Joinstr`] instance that have a `Coordinator` role, this role means
    ///   this instance will only initiate & monitor the coinjoin but will not add input
    ///   nor output.
    ///
    /// # Arguments
    /// * `keys` - Nostr keys that will be used for auth to the nostr relay
    /// * `relays` - A list of relays address to connect to
    /// * `electrum_server` - A tuple (<address>, <port>)
    /// * `network` - The bitcoin network (bitcoin/testnet/signet/regtest)
    /// * `name` - Name of the [`Joinstr`] instance (use for debug logs), can
    ///   be an empty &str.
    ///
    /// Note: the parameters of the pool should be passed with builder pattern
    pub fn new_initiator(
        keys: Keys,
        relay: String,
        electrum_server: (&str, u16),
        network: Network,
        name: &str,
    ) -> Result<Self, Error> {
        let j = Self::new_with_electrum(keys, relay, electrum_server, name)?.network(network);
        j.inner.lock().expect("poisoned").initiator = true;
        Ok(j)
    }

    /// Set the bitcoin network to mainnet
    pub fn mainnet(self) -> Self {
        self.inner.lock().expect("poisoned").network = Network::Bitcoin;
        self
    }

    /// Set the bitcoin network to signet
    pub fn signet(self) -> Self {
        self.inner.lock().expect("poisoned").network = Network::Signet;
        self
    }

    /// Set the bitcoin network to testnet
    pub fn testnet(self) -> Self {
        self.inner.lock().expect("poisoned").network = Network::Testnet;
        self
    }

    /// Set the bitcoin network to regtest
    pub fn regtest(self) -> Self {
        self.inner.lock().expect("poisoned").network = Network::Regtest;
        self
    }

    /// Set the bitcoin network to network
    pub fn network(self, network: Network) -> Self {
        self.inner.lock().expect("poisoned").network = network;
        self
    }

    /// Set the denomination of the pool in Bitcoin.
    pub fn denomination(self, denomination: f64) -> Result<Self, Error> {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_not_exists()?;
        if inner.denomination.is_none() {
            inner.denomination =
                Some(Amount::from_btc(denomination).map_err(|_| Error::WrongDenomination)?);
            drop(inner);
            Ok(self)
        } else {
            Err(Error::DenominationAlreadySet)
        }
    }

    /// Set the min number of peers of the pool
    pub fn min_peers(self, peers: usize) -> Result<Self, Error> {
        if peers < 2 {
            return Err(Error::Min2Peers);
        }
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_not_exists()?;
        if inner.peers.is_none() {
            inner.peers = Some(peers);
            drop(inner);
            Ok(self)
        } else {
            Err(Error::PeersAlreadySet)
        }
    }

    /// Set the timestamp at which the pool will be considered canceled if
    ///   not enough peer have join.
    pub fn simple_timeout(self, timestamp: u64) -> Result<Self, Error> {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_not_exists()?;
        if inner.timeout.is_none() {
            inner.timeout = Some(Timeline::Simple(timestamp));
            drop(inner);
            Ok(self)
        } else {
            Err(Error::TimeoutAlreadySet)
        }
    }

    /// Add a relay address to [`Joinstr::relays`]
    pub fn relay<T: Into<String>>(self, url: T) -> Result<Self, Error> {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_not_exists()?;
        // TODO: check the address is valid
        let url: String = url.into();
        inner.relays.push(url);
        drop(inner);
        Ok(self)
    }

    /// Set the minimum fee rate that the final transaction should spend to
    /// be considered valid (sats/vb)
    pub fn fee(self, fee: u32) -> Result<Self, Error> {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_not_exists()?;
        if inner.fee.is_none() {
            inner.fee = Some(Fee::Fixed(fee));
            drop(inner);
            Ok(self)
        } else {
            Err(Error::FeeAlreadySet)
        }
    }

    /// Set the coin to coinjoin
    ///
    /// # Errors
    ///
    /// This function will return an error if the coin is already set
    pub fn set_coin(&mut self, coin: Coin) -> Result<(), Error> {
        self.inner.lock().expect("poisoned").set_coin(coin)
    }

    /// Set the address the coin must be sent to
    ///
    /// # Errors
    ///
    /// This function will return an error if the address is already set
    /// or if address is for wrong network
    pub fn set_address(&mut self, addr: Address<NetworkUnchecked>) -> Result<(), Error> {
        self.inner.lock().expect("poisoned").set_address(addr)
    }

    /// Returns the finalized transaction
    pub fn final_tx(&self) -> Option<miniscript::bitcoin::Transaction> {
        self.inner
            .lock()
            .expect("poisoned")
            .final_tx
            .as_ref()
            .cloned()
    }

    /// Try to join the pool.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the pool does not exists
    ///   - the nostr client does not have keys
    ///   - the nostr client fail to connect relays
    ///   - sending a message to the pool fails
    ///   - receiving credentials fails
    ///   - pool connexion timed out
    fn join_pool(&mut self) -> Result<(), Error> {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.pool_exists()?;
        let pool_npub = inner.pool_as_ref()?.public_key;
        // TODO: receive the response on a derived npub;
        let my_npub = inner.client.get_keys()?.public_key();

        inner
            .client
            .send_pool_message(&pool_npub, PoolMessage::Join(Some(my_npub)))?;
        let (timeout, _) = inner.start_timeline()?;
        drop(inner);

        let mut backoff = Backoff::new_us(WAIT);

        let mut connected = false;
        while now() < timeout {
            let mut inner = self.inner.lock().expect("poisoned");
            if let Some(PoolMessage::Credentials(Credentials { id, key })) =
                inner.client.try_receive_pool_msg()?
            {
                log::warn!(
                    "Coordinator({}).connect_to_pool(): receive credentials.",
                    inner.client.name
                );
                if id == inner.pool_as_ref()?.id {
                    // we create a new nostr client using pool keys and replace the actual one
                    let keys = Keys::new(key);
                    let fg = &inner.client.name;
                    let name = format!("prev_{fg}");
                    let mut new_client = NostrClient::new(&name)
                        .relay(inner.client.get_relay().unwrap())?
                        .keys(keys)?;
                    new_client.connect_nostr()?;
                    inner.client = new_client;
                    connected = true;
                    break;
                } else {
                    log::error!(
                        "Coordinator({}).connect_to_pool(): pool id not match!",
                        inner.client.name
                    );
                }
            }
            drop(inner);
            backoff.snooze();
        }
        if !connected {
            return Err(Error::PoolConnectionTimeout);
        }
        Ok(())
    }

    /// Start the round of output registration, will block until enough output
    ///   registered or if some error occur.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the inner pool not exists
    ///   - the payload of the pool is missing
    ///   - the fee are not of type [`Fee::Fixed`]
    ///   - the nostr client do not have private keys
    ///   - timeout elapsed
    ///   - peer count do not match
    fn register_outputs(&mut self, initiator: bool) -> Result<(), Error> {
        let inner = self.inner.lock().expect("poisoned");
        inner.pool_exists()?;
        let (expired, start_early) = inner.start_timeline()?;
        let payload = inner.payload_as_ref()?.clone();
        let fee = if let Fee::Fixed(fee) = payload.fee {
            fee
        } else {
            return Err(Error::NotYetImplemented);
        };
        drop(inner);

        let mut peers = HashSet::<PublicKey>::new();
        let mut coinjoin = CoinJoin::<crate::electrum::Client>::new(payload.denomination, None)
            .min_peer(payload.peers)
            .fee(fee as usize);

        let mut backoff = Backoff::new_us(WAIT);

        // register peers
        while (now() < expired) && !(start_early && peers.len() >= payload.peers) {
            let mut inner = self.inner.lock().expect("poisoned");
            if let Ok(Some(msg)) = inner.client.try_receive_pool_msg() {
                match (msg, initiator) {
                    (PoolMessage::Join(Some(npub)), send_response) => {
                        if !peers.contains(&npub) {
                            if send_response {
                                let response = PoolMessage::Credentials(Credentials {
                                    id: inner.pool_as_ref()?.id.clone(),
                                    key: inner.client.get_keys()?.secret_key().clone(),
                                });
                                inner.client.send_pool_message(&npub, response)?;
                            }
                            peers.insert(npub);
                            log::debug!(
                                "Coordinator({}).register_outputs(): receive Join({}) request. \n      peers: {}",
                                inner.client.name,
                                npub,
                                peers.len()
                            );
                        }
                    }
                    (PoolMessage::Join(None), _) => panic!("cannot answer if npub is None!"),
                    (PoolMessage::Output(o), _) => {
                        log::error!(
                            "Coordinator({}).register_outputs(): receive Output({:?}) request before output registartion step!",
                            inner.client.name,
                            o
                        );
                        // NOTE: should we accept output registration at this step?
                        // Should we store the output and reuse at next step?
                    }
                    r => {
                        // NOTE: simply drop other kind of messages
                        log::debug!(
                            "Coordinator({}).register_outputs(): request not handled at peer registration step: {:?}!",
                            inner.client.name,
                            r
                        );
                    }
                }
            } else {
                drop(inner);
                backoff.snooze();
            }
        }

        // NOTE: at this point should we wait for every peer ACK the output template prior to
        // signing inputs?

        rand_delay();

        let mut inner = self.inner.lock().expect("poisoned");
        if let Some(output) = inner.output.as_ref() {
            coinjoin.add_output(output.clone());
            inner.register_output()?;
        }
        drop(inner);

        let mut backoff = Backoff::new_us(WAIT);

        // register ouputs
        let expired = self.inner.lock().expect("poisoned").end_timeline()?;
        while (now() < expired) && (coinjoin.outputs_len() < peers.len()) {
            let mut inner = self.inner.lock().expect("poisoned");
            if let Ok(Some(msg)) = inner.client.try_receive_pool_msg() {
                match msg {
                    PoolMessage::Join(_) => {
                        log::error!(
                            "Coordinator({}).register_outputs(): receive Join request at output registration step!",
                            inner.client.name,
                        );
                    }
                    PoolMessage::Output(o) => {
                        log::debug!(
                            "Coordinator({}).register_outputs(): receive Output({:?}) request.",
                            inner.client.name,
                            o
                        );
                        let outputs = vec![o];
                        inner.receive_outputs(outputs, &mut coinjoin)?;
                    }
                    // FIXME: here it can be some cases where, because network timing, we can
                    // receive a signed input before the output registration round ended, we should
                    // store those inputs in order to use them later.
                    PoolMessage::Input(_) => todo!("store input"),
                    r => {
                        // NOTE: simply drop other kind of messages
                        log::debug!(
                            "Coordinator({}).register_outputs(): request not handled at output registration step: {:?}!",
                            inner.client.name,
                            r
                        );
                    }
                }
            } else {
                drop(inner);
                backoff.snooze();
            }
        }

        if now() > expired {
            return Err(Error::Timeout);
        } else if peers.len() < payload.peers {
            return Err(Error::NotEnoughPeers(peers.len(), payload.peers));
        } else if coinjoin.outputs_len() != peers.len() {
            // NOTE: do not allow registered peer that not commit an output as it can be some
            // lurkers trying deanonimyze peers

            return Err(Error::PeerCountNotMatch(
                coinjoin.outputs_len(),
                peers.len(),
            ));
        }
        self.inner.lock().expect("poisoined").coinjoin = Some(coinjoin);
        Ok(())
    }

    /// Start the round of input registration, will block until enough input
    ///   registered or if some error occur.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the inner pool does not exists
    ///   - the pool payload is missing
    ///   - [`Joinstr::coinjoin`] is None
    ///   - timeout expired
    ///   - trying register an input error
    ///   - trying finalize coinjoin error
    fn register_inputs(&mut self) -> Result<(), Error> {
        let inner = self.inner.lock().expect("poisoned");
        inner.pool_exists()?;
        inner.coinjoin_exists()?;
        let payload = inner.payload_as_ref()?;
        let expired = match payload.timeout {
            Timeline::Simple(timestamp) => timestamp,
            Timeline::Fixed {
                start,
                max_duration,
            } => start + max_duration,
            Timeline::Timeout { max_duration, .. } => now() + max_duration,
        };
        drop(inner);
        if now() > expired {
            return Err(Error::Timeout);
        }

        let mut backoff = Backoff::new_us(WAIT);

        while now() < expired
            && self
                .inner
                .lock()
                .expect("poisoned")
                .coinjoin_as_ref()?
                .tx
                .is_none()
        {
            let mut inner = self.inner.lock().expect("poisoned");
            let msg = inner.client.try_receive_pool_msg();
            if let Ok(Some(msg)) = msg {
                match msg {
                    PoolMessage::Psbt(psbt) => {
                        let input: InputDataSigned =
                            psbt.try_into().map_err(|_| Error::PsbtToInput)?;
                        inner.try_register_input(input)?;
                        if inner.try_finalize_coinjoin()? {
                            break;
                        }
                    }
                    PoolMessage::Input(input) => {
                        inner.try_register_input(input)?;
                        if inner.try_finalize_coinjoin()? {
                            break;
                        }
                    }
                    m => {
                        // NOTE: simply drop other kind of messages
                        log::error!(
                            "Coordinator({}).register_input(): drop message {:?}",
                            inner.client.name,
                            m
                        );
                    }
                }
            } else {
                drop(inner);
                backoff.snooze();
            }
        }
        if now() > expired {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }

    /// Start a coinjoin process, followings steps will be processed:
    ///   - if no `pool` arg is passed, a new pool will be initiated.
    ///   - if a `pool` arg is passed, it will join the pool
    ///   - run the outputs registration round
    ///   - if a `signer` arg is passed, it will signed the input it owns.
    ///   - run the inputs registration round
    ///   - finalize the transaction
    ///   - broadcast the transaction
    ///
    /// # Arguments
    /// * `pool` - The pool we want join (optional)
    /// * `signer` - The signer to sign our input with (optional)
    ///
    /// # Errors
    ///
    /// This function will return an error if any step return an error.
    pub fn start_coinjoin<S>(&mut self, pool: Option<Pool>, signer: Option<&S>) -> Result<(), Error>
    where
        S: JoinstrSigner,
    {
        let initiator = pool.is_none();
        if let Some(pool) = pool {
            let mut inner = self.inner.lock().expect("poisoned");
            inner.pool_not_exists()?;
            inner.pool = Some(pool);
            drop(inner);
            self.join_pool()?;
        } else {
            // broadcast the pool event
            self.inner.lock().expect("poisoned").post()?;
        }
        // register peers & outputs
        self.register_outputs(initiator)?;

        self.inner
            .lock()
            .expect("poisoned")
            .generate_unsigned_tx()?;

        rand_delay();

        let mut inner = self.inner.lock().expect("poisoned");
        if inner.input.is_some() {
            if let Some(s) = signer {
                inner.register_input(s)?;
            } else {
                return Err(Error::SignerMissing);
            }
        }
        drop(inner);

        self.register_inputs()?;

        self.inner.lock().expect("poisoned").broadcast_tx()?;

        Ok(())
    }
}

impl<'a> JoinstrInner<'a> {
    /// Utility function that will error if [`Joinstr::pool`] is Some()
    fn pool_not_exists(&self) -> Result<(), Error> {
        if self.pool.is_some() {
            Err(Error::PoolAlreadyExists)
        } else {
            Ok(())
        }
    }

    /// Utility function that will error if [`Joinstr::pool`] is None
    fn pool_exists(&self) -> Result<(), Error> {
        if let Some(Pool {
            payload: Some(_), ..
        }) = self.pool.as_ref()
        {
            Ok(())
        } else {
            Err(Error::PoolNotExists)
        }
    }

    /// Returns inner pool as ref.
    ///
    /// # Errors
    ///
    /// This function will return an error if the pool is None
    fn pool_as_ref(&self) -> Result<&Pool, Error> {
        self.pool.as_ref().ok_or(Error::PoolNotExists)
    }

    /// Returns the inner pool payload as ref.
    ///
    /// # Errors
    ///
    /// This function will return an error if the pool is None or
    ///   the payload is None.
    fn payload_as_ref(&self) -> Result<&PoolPayload, Error> {
        self.pool
            .as_ref()
            .ok_or(Error::PoolNotExists)
            .and_then(|p| p.payload.as_ref().ok_or(Error::PoolPayloadMissing))
    }

    /// utility funtion, will error if the inner [`CoinJoin`] is None
    fn coinjoin_exists(&self) -> Result<(), Error> {
        self.coinjoin
            .as_ref()
            .ok_or(Error::CoinjoinMissing)
            .map(|_| ())
    }

    /// Returns the coinjoin as ref.
    ///
    /// # Errors
    ///
    /// This function will return an error if the inner [`CoinJoin`] is None.
    fn coinjoin_as_ref(&self) -> Result<&CoinJoin<'a, crate::electrum::Client>, Error> {
        self.coinjoin.as_ref().ok_or(Error::CoinjoinMissing)
    }

    /// Returns the coinjoin as mut.
    ///
    /// # Errors
    ///
    /// This function will return an error if the inner [`CoinJoin`] is None.
    fn coinjoin_as_mut(&mut self) -> Result<&mut CoinJoin<'a, crate::electrum::Client>, Error> {
        self.coinjoin.as_mut().ok_or(Error::CoinjoinMissing)
    }

    /// Utility function, will error if some fields of the [`Pool`] are None.
    fn is_ready(&self) -> Result<(), Error> {
        if self.pool.is_none()
            && self.denomination.is_some()
            && self.peers.is_some()
            && self.timeout.is_some()
            && !self.relays.is_empty()
            && self.fee.is_some()
        {
            Ok(())
        } else {
            if self.pool.is_some() {
                log::error!("Coordinator.is_ready(): pool is not None!")
            }
            if self.denomination.is_none() {
                log::error!("Coordinator.is_ready(): denomination is missing!")
            }
            if self.peers.is_none() {
                log::error!("Coordinator.is_ready(): peers is missing!")
            }
            if self.timeout.is_none() {
                log::error!("Coordinator.is_ready(): timeout is missing!")
            }
            if self.relays.is_empty() {
                log::error!("Coordinator.is_ready(): no relay specified!")
            }
            if self.fee.is_none() {
                log::error!("Coordinator.is_ready(): fee is missing!")
            }
            Err(Error::ParamMissing)
        }
    }

    /// Initiate a new pool by sending a pool creation event (Kind 2022)
    ///   to nostr relays.
    ///
    /// # Errors
    ///
    /// This function will return an error if a pool already exists, if
    ///   some fields of the pool are missing or if posting the event fail.
    fn post(&mut self) -> Result<(), Error> {
        self.is_ready()?;
        self.pool_not_exists()?;
        let public_key = self.client.get_keys()?.public_key();
        let transport = crate::nostr::Transport {
            vpn: Some(Vpn {
                enable: false,
                gateway: None,
            }),
            tor: Some(Tor { enable: false }),
        };
        if self.relays.is_empty() {
            return Err(Error::RelaysMissing);
        };
        let payload = PoolPayload {
            denomination: self.denomination.ok_or(Error::DenominationMissing)?,
            peers: self.peers.ok_or(Error::PeerMissing)?,
            timeout: self.timeout.ok_or(Error::TimeoutMissing)?,
            relays: self.relays.clone(),
            fee: self.fee.clone().ok_or(Error::FeeMissing)?,
            transport,
        };
        let mut engine = sha256::Hash::engine();
        engine.input(&public_key.clone().to_bytes());
        engine.input(
            &SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unix timestamp must not fail")
                .as_micros()
                .to_be_bytes(),
        );
        let id = sha256::Hash::from_engine(engine).to_string();

        let pool = Pool {
            versions: default_version(),
            id,
            pool_type: PoolType::Create,
            public_key,
            payload: Some(payload),
            network: self.network,
        };
        self.client.post_event(pool.clone().try_into()?)?;
        self.pool = Some(pool);
        Ok(())
    }

    /// Returns informations about the start timeline of this pool:
    ///   - expiration timestamp
    ///   - wether if the coinjoin can start early if enough peer join
    ///
    /// # Errors
    ///
    /// This function will return an error if the pool not exists or the
    ///   pool payload is missing.
    fn start_timeline(&self) -> Result<(u64 /* expiration */, bool /* start_early */), Error> {
        self.pool_exists()?;
        let payload = self.payload_as_ref()?;
        Ok(match &payload.timeout {
            Timeline::Simple(timestamp) => (*timestamp, true),
            Timeline::Fixed { start, .. } => (*start, false),
            Timeline::Timeout { timeout, .. } => (*timeout, true),
        })
    }

    /// Returns timestamp of the timeline end of this pool
    ///
    /// # Errors
    ///
    /// This function will return an error if the pool not exists ,the
    ///   pool payload is missing or there is an error in the timeline
    ///   duration calculation.
    fn end_timeline(&self) -> Result<u64, Error> {
        self.pool_exists()?;
        let payload = self.payload_as_ref()?;
        Ok(match &payload.timeout {
            Timeline::Simple(timestamp) => *timestamp,
            Timeline::Fixed {
                start,
                max_duration,
            } => start
                .checked_add(*max_duration)
                .ok_or(Error::TimelineDuration)?,
            Timeline::Timeout {
                timeout,
                max_duration,
            } => timeout
                .checked_add(*max_duration)
                .ok_or(Error::TimelineDuration)?,
        })
    }

    /// Register [`Joinstr::output`] address to the pool
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the pool not exists
    ///   - [`Joinstr::output`] is missing
    ///   - fails to send the nostr message
    fn register_output(&mut self) -> Result<(), Error> {
        if let Some(address) = &self.output {
            // let msg = PoolMessage::Outputs(Outputs::single(address.as_unchecked().clone()));
            let msg = PoolMessage::Output(address.as_unchecked().clone());
            self.pool_exists()?;
            let npub = self.pool_as_ref()?.public_key;
            self.client.send_pool_message(&npub, msg)?;
            // TODO: handle re-send if fails
            Ok(())
        } else {
            Err(Error::OutputMissing)
        }
    }

    /// Try to register a received output address to the inner [`CoinJoin`]
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the inner pool is None
    ///   - the address is not valid for the network
    ///
    /// Note: `outputs` is a Vec in order to allow a future compatibility
    /// for several "coordinator" instances operating on differents nostr relays.
    fn receive_outputs<T>(
        &mut self,
        outputs: Vec<Address<NetworkUnchecked>>,
        coinjoin: &mut CoinJoin<'_, T>,
    ) -> Result<(), Error>
    where
        T: crate::coinjoin::BitcoinBackend,
    {
        for addr in outputs {
            if addr.is_valid_for_network(self.pool_as_ref()?.network) {
                let addr = addr.assume_checked();
                // FIXME: should we check if the output have been added?
                coinjoin.add_output(addr);
            } else {
                log::debug!(
                    "Coordinator({}).register_outputs(): address {:?} is not valid for network {}.",
                    self.client.name,
                    addr,
                    self.network
                );
            }
        }
        Ok(())
    }

    /// Try to sign / register / send our input.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the inner coinjoin is missing
    ///   - the unsigned transaction has not been processed
    ///   - signing the input fails
    ///   - the inner pool dont exists
    ///   - [`Joinstr::input`] is None
    ///   - sending the input fails
    fn register_input<S>(&mut self, signer: &S) -> Result<(), Error>
    where
        S: JoinstrSigner,
    {
        let unsigned = match self.coinjoin_as_ref()?.unsigned_tx() {
            Some(u) => u,
            None => return Err(Error::UnsignedTxNotExists),
        };
        if let Some(input) = self.input.take() {
            let signed_input = signer
                .sign_input(&unsigned, input)
                .map_err(Error::SigningFail)?;
            let msg = PoolMessage::Input(signed_input);
            self.pool_exists()?;
            let npub = self.pool_as_ref()?.public_key;
            self.client.send_pool_message(&npub, msg)?;
            // TODO: handle re-send if fails
            Ok(())
        } else {
            Err(Error::InputMissing)
        }
    }

    // Try to register a received signed input to the inner [`CoinJoin`]
    ///
    /// # Errors
    ///
    /// This function will return an error if [`Joinstr::coinjoin`] is None
    fn try_register_input(&mut self, input: InputDataSigned) -> Result<(), Error> {
        self.coinjoin_exists()?;
        log::debug!(
            "Coordinator({}).register_input(): receive Inputs({:?}) request.",
            self.client.name,
            input
        );
        // Register inputs
        if let Some(coinjoin) = self.coinjoin.as_mut() {
            if let Err(e) = coinjoin.add_input(input) {
                log::error!(
                    "Coordinator({}).register_input(): fail to add input: {:?}",
                    self.client.name,
                    e
                );
            }
        }
        Ok(())
    }

    /// Return wether the coinjoin can be finalyzed.
    ///
    /// # Errors
    ///
    /// This function will return an error if [`Joinstr::coinjoin`] is None.
    fn try_finalize_coinjoin(&mut self) -> Result<bool, Error> {
        let coinjoin = self.coinjoin_as_mut()?;
        if coinjoin.inputs_len() >= coinjoin.outputs_len() && coinjoin.generate_tx(false).is_ok() {
            log::info!(
                "Coordinator({}).register_input(): coinjoin finalyzed!",
                self.client.name,
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Generate the unsignex transaction
    ///
    /// # Errors
    ///
    /// This function will return an error if [`Joinstr::coinjoin`] is
    ///   None or generating the psbt fails.
    fn generate_unsigned_tx(&mut self) -> Result<(), Error> {
        let coinjoin = self.coinjoin_as_mut()?;
        // process unsigned tx
        coinjoin.generate_psbt()?;

        Ok(())
    }

    /// Broadcast the signed + finalized transaction.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - The inner pool does not exists
    ///   - [`Joinstr::coinjoin`] is None
    ///   - The transaction has not been finalized
    ///   - brodcasting transaction to the backend fails
    ///
    /// Note: if no backend, the transaction will not been broadcasted
    ///   but no error will be emited.
    fn broadcast_tx(&mut self) -> Result<(), Error> {
        self.pool_exists()?;
        let tx = self.coinjoin_as_ref()?.tx().ok_or(Error::MissingFinalTx)?;
        if let Some(client) = self.electrum_client.as_mut() {
            client.broadcast(&tx)?;
        }
        self.final_tx = Some(tx);
        Ok(())
    }

    /// Returns the finalized transaction
    pub fn final_tx(&self) -> Option<&miniscript::bitcoin::Transaction> {
        self.final_tx.as_ref()
    }

    /// Set the coin to coinjoin
    ///
    /// # Errors
    ///
    /// This function will return an error if the coin is already set
    pub fn set_coin(&mut self, coin: Coin) -> Result<(), Error> {
        if self.input.is_none() {
            self.input = Some(coin);
            Ok(())
        } else {
            Err(Error::AlreadyHaveInput)
        }
    }

    /// Set the address the coin must be sent to
    ///
    /// # Errors
    ///
    /// This function will return an error if the address is already set
    /// or if address is for wrong network
    pub fn set_address(&mut self, addr: Address<NetworkUnchecked>) -> Result<(), Error> {
        let addr = if addr.is_valid_for_network(self.network) {
            addr.assume_checked()
        } else {
            return Err(Error::WrongAddressNetwork);
        };
        if self.output.is_none() {
            self.output = Some(addr);
            Ok(())
        } else {
            Err(Error::AlreadyHaveOutput)
        }
    }
}
