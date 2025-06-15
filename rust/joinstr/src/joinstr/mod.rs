mod error;
use backoff::Backoff;
pub use error::Error;
use serde::{Deserialize, Serialize};

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use miniscript::bitcoin::{Amount, Network};
use simple_nostr_client::nostr::{
    self,
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

#[derive(Debug, Clone)]
pub struct Joinstr<'a> {
    pub inner: Arc<Mutex<JoinstrInner<'a>>>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role {
    #[default]
    Unknown,
    Initiator,
    Peer,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Step {
    #[default]
    Unconfigured,
    Configured,
    Posting,
    Connecting,
    OutputRegistration,
    InputRegistration,
    Broadcast,
    Mined,
    Failed,
}

pub struct Status {
    role: Role,
    step: Step,
    registered_peers: usize,
    registered_outputs: usize,
    registered_inputs: usize,
    confirmations: usize,
    error: Option<String>,
}

#[derive(Debug)]
pub struct JoinstrInner<'a> {
    role: Role,
    step: Step,
    confirmations: usize,
    error: Option<String>,
    pub client: NostrClient,
    pub pool: Option<Pool>,
    pub denomination: Option<Amount>,
    pub peers_count: Option<usize>,
    pub timeout: Option<Timeline>,
    pub relay: Option<String>,
    pub fee: Option<Fee>,
    pub network: Network,
    pub coinjoin: Option<CoinJoin<'a, crate::electrum::Client>>,
    pub electrum_client: Option<crate::electrum::Client>,
    input: Option<Coin>,
    output: Option<Address>,
    final_tx: Option<miniscript::bitcoin::Transaction>,
    // requests history
    peers: Vec<nostr::PublicKey>,
    outputs: Vec<miniscript::bitcoin::Address>,
    inputs: Vec<InputDataSigned>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub role: Role,
    pub step: Step,
    pub pool_secret_key: String, /* nostr::Keys*/
    pub relay: String,
    pub electrum: Option<(String, u16)>,
    pub pool: Pool,
    pub input: Option<Coin>,
    pub output: Option<Address<NetworkUnchecked>>,
    pub network: bitcoin::Network,
    pub final_tx: Option<bitcoin::Transaction>,
    // requests history
    pub peers: Vec<nostr::PublicKey>,
    pub outputs: Vec<bitcoin::Address<NetworkUnchecked>>,
    pub inputs: Vec<serde_json::Value /* InputDataSigned*/>,
}

impl Default for JoinstrInner<'_> {
    fn default() -> Self {
        Self {
            role: Default::default(),
            step: Default::default(),
            confirmations: 0,
            error: None,
            client: Default::default(),
            pool: Default::default(),
            denomination: Default::default(),
            peers_count: Default::default(),
            timeout: Default::default(),
            relay: Default::default(),
            fee: Default::default(),
            network: Network::Bitcoin,
            coinjoin: None,
            electrum_client: None,
            input: None,
            output: None,
            final_tx: None,
            peers: Default::default(),
            outputs: Default::default(),
            inputs: Default::default(),
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
        let relay = Some(relay);
        let inner = Arc::new(Mutex::new(JoinstrInner {
            client,
            relay,
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
        inner.role = Role::Peer;
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
        inner.role = Role::Peer;
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
        j.inner.lock().expect("poisoned").role = Role::Initiator;
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
        if inner.peers_count.is_none() {
            inner.peers_count = Some(peers);
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
        inner.relay = Some(url);
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
        inner.step = Step::Connecting;
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
                log::debug!(
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
                    inner.step = Step::OutputRegistration;
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
    /// # Arguments
    /// * `notif` - A callback function called every time the pool state is updated.
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
    fn register_outputs<N>(&mut self, notif: N) -> Result<(), Error>
    where
        N: Fn(),
    {
        let inner = self.inner.lock().expect("poisoned");
        inner.pool_exists()?;
        let (expired, start_early) = inner.start_timeline()?;
        let payload = inner.payload_as_ref()?.clone();
        let fee = if let Fee::Fixed(fee) = payload.fee {
            fee
        } else {
            return Err(Error::NotYetImplemented);
        };
        let pool_pubkey = inner.pool_as_ref()?.public_key;
        let role = inner.role;
        let relay = inner.client.get_relay().ok_or(Error::RelaysMissing)?;
        drop(inner);

        let mut peers = HashSet::<PublicKey>::new();
        let mut coinjoin = CoinJoin::<crate::electrum::Client>::new(payload.denomination, None)
            .min_peer(payload.peers)
            .fee(fee as usize);

        if role == Role::Initiator {
            // send a dummy join request
            let mut dummy_client = NostrClient::new("dummy")
                .keys(Keys::generate())?
                .relay(relay)?;
            dummy_client.connect_nostr()?;

            let dummy_response_key = Keys::generate().public_key();
            dummy_client
                .send_pool_message(&pool_pubkey, PoolMessage::Join(Some(dummy_response_key)))?;
        }

        let mut backoff = Backoff::new_us(WAIT);
        // register peers
        while (now() < expired) && !(start_early && peers.len() >= payload.peers) {
            let mut inner = self.inner.lock().expect("poisoned");
            if let Ok(Some(msg)) = inner.client.try_receive_pool_msg() {
                match (msg, matches!(inner.role, Role::Initiator)) {
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
                            inner.peers.push(npub);
                            notif();
                            log::debug!(
                                "Coordinator({}).register_outputs(): receive Join({}) request. \n      peers: {}",
                                inner.client.name,
                                npub,
                                peers.len()
                            );
                        }
                    }
                    // TODO: do not panic here
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
            inner.register_output(&notif)?;
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
                        // FIXME: we should not log an error here
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
                        let outputs = vec![o.clone()];
                        inner.receive_outputs(outputs, &mut coinjoin)?;
                        // TODO: we must error if outputs > peers
                        // TODO: check address network
                        inner.outputs.push(o.assume_checked());
                        notif();
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
        notif();
        Ok(())
    }

    /// Start the round of input registration, will block until enough input
    ///   registered or if some error occur.
    ///
    /// # Arguments
    /// * `notif` - A callback function called every time the pool state is updated.
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
    fn register_inputs<N>(&mut self, notif: N) -> Result<(), Error>
    where
        N: Fn(),
    {
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
                        inner.try_register_input(input, &notif)?;
                        if inner.try_finalize_coinjoin()? {
                            break;
                        }
                    }
                    PoolMessage::Input(input) => {
                        inner.try_register_input(input, &notif)?;
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

    /// Returns the current status of the [`Joinstr`] instance.
    ///
    /// # Returns
    /// A [`Status`] struct containing the current state information.
    pub fn status(&self) -> Status {
        self.inner.lock().expect("poisoned").status()
    }

    /// Returns the current serializable state of the [`Joinstr`] instance.
    ///
    /// # Returns
    /// A [`State`] struct containing the current state information.
    pub fn state(&self) -> Option<State> {
        self.inner.lock().expect("poisoned").state()
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
    pub fn start_coinjoin<S>(&mut self, pool: Option<Pool>, signer: Option<S>)
    where
        S: JoinstrSigner + Sized + Sync + Clone + Send + 'static,
        Self: Sized + Send + 'static,
    {
        let mut cloned = self.clone();
        let signer = signer.clone();
        thread::spawn(move || {
            if let Err(e) = cloned.start_coinjoin_blocking(pool, signer, || {}) {
                let mut inner = cloned.inner.lock().expect("poisoned");
                inner.error = Some(format!("{:?}", e));
                inner.step = Step::Failed;
            }
        });
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
    /// * `notif` - A callback function called every time the pool state is updated.
    ///
    /// # Errors
    ///
    /// This function will return an error if any step return an error.
    pub fn start_coinjoin_with_notif<S, N>(
        &mut self,
        pool: Option<Pool>,
        signer: Option<S>,
        notif: N,
    ) where
        S: JoinstrSigner + Sized + Sync + Clone + Send + 'static,
        Self: Sized + Send + 'static,
        N: Fn() + Send + 'static,
    {
        let mut cloned = self.clone();
        let signer = signer.clone();
        thread::spawn(move || {
            if let Err(e) = cloned.start_coinjoin_blocking(pool, signer, notif) {
                let mut inner = cloned.inner.lock().expect("poisoned");
                inner.error = Some(format!("{:?}", e));
                inner.step = Step::Failed;
            }
        });
    }

    pub fn start_coinjoin_blocking<S, N>(
        &mut self,
        pool: Option<Pool>,
        signer: Option<S>,
        notif: N,
    ) -> Result<(), Error>
    where
        S: JoinstrSigner + Sync + Clone + Send + 'static,
        N: Fn(),
    {
        let name = self.inner.lock().expect("poisoned").client.name.clone();

        log::debug!("Joinstr::Start_coinjoin_blocking({name})");
        let mut inner = self.inner.lock().expect("poisoned");

        if matches!(inner.role, Role::Unknown) {
            log::error!("Joinstr::start_coinjoin_blocking({name}): wrong role!");
            return Err(Error::WrongRole);
        }

        if let Some(pool) = pool {
            log::debug!("Joinstr::start_coinjoin_blocking({name}) try to join pool...");
            inner.pool_not_exists()?;
            inner.pool = Some(pool);
            drop(inner);
            self.join_pool()?;
            log::debug!("Joinstr::start_coinjoin_blocking({name}) pool joined");
        } else {
            // broadcast the pool event
            log::debug!("Joinstr::start_coinjoin_blocking({name}) try to broadcast pool...");
            inner.post()?;
            log::debug!("Joinstr::start_coinjoin_blocking({name}) pool broadcast!");
            drop(inner);
        }
        notif();

        log::debug!("Joinstr::start_coinjoin_blocking({name}) start register outputs..");
        // register peers & outputs
        self.register_outputs(&notif)?;
        log::debug!("Joinstr::start_coinjoin_blocking({name}) outputs registered!");

        self.inner
            .lock()
            .expect("poisoned")
            .generate_unsigned_tx()?;

        log::debug!("Joinstr::start_coinjoin_blocking({name}) unsigned tx generated!");
        notif();

        rand_delay();

        let mut inner = self.inner.lock().expect("poisoned");
        if inner.input.is_some() {
            if let Some(s) = signer {
                log::debug!("Joinstr::start_coinjoin_blocking({name}) try register input....");
                inner.register_input(&s, &notif)?;
                log::debug!("Joinstr::start_coinjoin_blocking({name}) input registered!");
            } else {
                log::debug!("Joinstr::start_coinjoin_blocking({name}) no input to register!");
                return Err(Error::SignerMissing);
            }
        }
        drop(inner);

        log::debug!(
            "Joinstr::start_coinjoin_blocking({name}) start registering external inputs..."
        );
        self.register_inputs(&notif)?;

        log::debug!("Joinstr::start_coinjoin_blocking({name}) inputs registerd!");

        log::debug!("Joinstr::start_coinjoin_blocking({name}) try broadcast tx...");
        self.inner.lock().expect("poisoned").broadcast_tx()?;
        log::debug!("Joinstr::start_coinjoin_blocking({name}) tx broadcast!");
        notif();

        Ok(())
    }

    pub fn restart<S, N>(state: State, name: &str, signer: S, notif: N) -> Result<Self, Error>
    where
        S: JoinstrSigner + Sized + Sync + Clone + Send + 'static,
        N: Fn() + Send + 'static,
        Self: Sized + Send + 'static,
    {
        let State {
            role,
            step: _,
            pool_secret_key,
            relay,
            electrum,
            pool,
            input,
            output,
            network,
            peers,
            outputs,
            inputs,
            final_tx,
        } = state;
        let secret_key = nostr::SecretKey::from_hex(pool_secret_key).map_err(|_| Error::PoolKey)?;
        let keys = Keys::new(secret_key);
        let j = Joinstr::new(keys, relay, name)?.network(network);
        let mut inner = j.inner.lock().expect("poisoned");
        inner.role = role;
        inner.pool = Some(pool);
        if let Some((url, port)) = electrum {
            inner.electrum_client = Some(crate::electrum::Client::new(&url, port)?)
        }
        inner.input = input;
        if let Some(addr) = output {
            if addr.is_valid_for_network(network) {
                inner.output = Some(addr.assume_checked().clone());
            } else {
                return Err(Error::WrongAddressNetwork);
            }
        }
        inner.network = network;
        inner.peers = peers;
        let mut outs = vec![];
        for o in outputs {
            if !o.is_valid_for_network(network) {
                return Err(Error::WrongAddressNetwork);
            }
            outs.push(o.assume_checked().clone());
        }
        inner.outputs = outs;
        let mut inps = vec![];
        for i in inputs {
            inps.push(serde_json::from_value(i).map_err(|_| Error::InputParsing)?);
        }
        inner.inputs = inps;

        // pool state is already finalized
        if let Some(tx) = final_tx {
            inner.final_tx = Some(tx);
            drop(inner);
            return Ok(j);
        }

        let expected_peers = inner
            .pool
            .as_ref()
            .expect("always have a pool")
            .payload
            .as_ref()
            .expect("have a payload")
            .peers;

        let mut recv_peers = vec![];
        let mut recv_outputs = vec![];
        let mut recv_inputs = vec![];

        // NOTE: here it's tricky to restart if we have a non finalized coinjoin:
        //  - we cannot trust the timestamp of messages
        //  - an external actor can have posted a join request w/ a fake timestamp
        //  - a malicious peer can have posted a message w/ a fake timestamp
        //  - a malicious peer can have posted a fake message
        //  - a message can have been deleted
        //
        // FIXME: it's then easy to be DOS by a malicious actor, we should have a way to sign
        // with the pool key & post a backup state

        // get all already received pool messages
        while let Ok(Some(msg)) = inner.client.try_receive_pool_msg() {
            match msg {
                PoolMessage::Input(input) => {
                    recv_inputs.push(input);
                }
                PoolMessage::Output(address) => {
                    if address.is_valid_for_network(network) {
                        recv_outputs.push(address.assume_checked());
                    }
                }
                PoolMessage::Psbt(psbt) => {
                    if let Ok(input) = psbt.try_into() {
                        recv_inputs.push(input);
                    }
                }
                PoolMessage::Join(Some(public_key)) => {
                    recv_peers.push(public_key);
                }
                _ => {}
            }
        }

        let total_peers = inner.peers.len() + recv_peers.len();
        let total_outputs = inner.outputs.len() + recv_outputs.len();
        let total_inputs = inner.inputs.len() + recv_inputs.len();

        if total_peers > expected_peers {
            return Err(Error::PoolCorrupted);
        }
        if total_outputs > total_peers {
            return Err(Error::PoolCorrupted);
        }
        if total_inputs > total_outputs {
            return Err(Error::PoolCorrupted);
        }
        if total_inputs > total_peers {
            return Err(Error::PoolCorrupted);
        }

        inner.peers.append(&mut recv_peers);
        inner.outputs.append(&mut recv_outputs);
        inner.inputs.append(&mut recv_inputs);

        drop(inner);

        fn restart_blocking<S, N>(
            mut j: Joinstr,
            expected_peers: usize,
            signer: S,
            notif: N,
        ) -> Result<(), Error>
        where
            S: JoinstrSigner + Sync + Clone + Send + 'static,
            N: Fn(),
        {
            let inner = j.inner.lock().expect("poisoned");
            let joined = inner.peers.len() >= expected_peers;
            let output_registered = inner.outputs.len() >= expected_peers;
            let inputs_registered = inner.inputs.len() >= expected_peers;

            drop(inner);

            if !joined || !output_registered {
                j.register_outputs(&notif)?;
            }

            if !inputs_registered {
                j.inner.lock().expect("poisoned").generate_unsigned_tx()?;
                notif();

                rand_delay();

                let mut inner = j.inner.lock().expect("poisoned");
                if inner.input.is_some() {
                    inner.register_input(&signer, &notif)?;
                }
                drop(inner);

                j.register_inputs(&notif)?;

                j.inner.lock().expect("poisoned").broadcast_tx()?;
            }
            Ok(())
        }

        let j2 = j.clone();
        let j3 = j.clone();

        std::thread::spawn(move || {
            if let Err(e) = restart_blocking(j2, expected_peers, signer, &notif) {
                let mut inner = j3.inner.lock().expect("poisoned");
                inner.error = Some(format!("{:?}", e));
                inner.step = Step::Failed;
            }
        });

        Ok(j)
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
            && self.peers_count.is_some()
            && self.timeout.is_some()
            && self.relay.is_some()
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
            if self.peers_count.is_none() {
                log::error!("Coordinator.is_ready(): peers is missing!")
            }
            if self.timeout.is_none() {
                log::error!("Coordinator.is_ready(): timeout is missing!")
            }
            if self.relay.is_none() {
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

        self.step = Step::Posting;
        let public_key = self.client.get_keys()?.public_key();
        let transport = crate::nostr::Transport {
            vpn: Some(Vpn {
                enable: false,
                gateway: None,
            }),
            tor: Some(Tor { enable: false }),
        };
        if self.relay.is_none() {
            return Err(Error::RelaysMissing);
        };
        let payload = PoolPayload {
            denomination: self.denomination.ok_or(Error::DenominationMissing)?,
            peers: self.peers_count.ok_or(Error::PeerMissing)?,
            timeout: self.timeout.ok_or(Error::TimeoutMissing)?,
            relays: self.relay.clone().map(|r| vec![r]).unwrap_or_default(),
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
        self.step = Step::OutputRegistration;
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
    /// # Arguments
    /// * `notif` - A callback function called every time the pool state is updated.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - the pool not exists
    ///   - [`Joinstr::output`] is missing
    ///   - fails to send the nostr message
    fn register_output<N>(&mut self, notif: N) -> Result<(), Error>
    where
        N: Fn(),
    {
        if let Some(address) = &self.output {
            // let msg = PoolMessage::Outputs(Outputs::single(address.as_unchecked().clone()));
            let msg = PoolMessage::Output(address.as_unchecked().clone());
            self.pool_exists()?;
            let npub = self.pool_as_ref()?.public_key;
            self.client.send_pool_message(&npub, msg)?;
            self.outputs.push(address.clone());
            notif();
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
    /// # Arguments
    /// * `notif` - A callback function called every time the pool state is updated.
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
    fn register_input<S, N>(&mut self, signer: &S, notif: N) -> Result<(), Error>
    where
        S: JoinstrSigner,
        N: Fn(),
    {
        let unsigned = match self.coinjoin_as_ref()?.unsigned_tx() {
            Some(u) => u,
            None => return Err(Error::UnsignedTxNotExists),
        };
        if let Some(input) = self.input.take() {
            let signed_input = signer
                .sign_input(&unsigned, input)
                .map_err(Error::SigningFail)?;
            let msg = PoolMessage::Input(signed_input.clone());
            self.pool_exists()?;
            let npub = self.pool_as_ref()?.public_key;
            self.client.send_pool_message(&npub, msg)?;
            self.inputs.push(signed_input);
            notif();
            // TODO: handle re-send if fails
            Ok(())
        } else {
            Err(Error::InputMissing)
        }
    }

    /// Try to register a received signed input to the inner [`CoinJoin`]
    ///
    /// # Arguments
    /// * `notif` - A callback function called every time the pool state is updated.
    ///
    /// # Errors
    ///
    /// This function will return an error if [`Joinstr::coinjoin`] is None
    fn try_register_input<N>(&mut self, input: InputDataSigned, notif: N) -> Result<(), Error>
    where
        N: Fn(),
    {
        self.coinjoin_exists()?;
        log::debug!(
            "Coordinator({}).register_input(): receive Inputs({:?}) request.",
            self.client.name,
            input
        );
        // Register inputs
        if let Some(coinjoin) = self.coinjoin.as_mut() {
            if let Err(e) = coinjoin.add_input(input.clone()) {
                log::error!(
                    "Coordinator({}).register_input(): fail to add input: {:?}",
                    self.client.name,
                    e
                );
            } else {
                self.inputs.push(input);
                notif();
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
        self.step = Step::Broadcast;
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

    /// Returns the current status of the [`JoinstrInner`] instance.
    ///
    /// # Returns
    /// A [`Status`] struct containing the current status information.
    pub fn status(&self) -> Status {
        Status {
            role: self.role,
            step: self.step,
            registered_peers: self.peers.len(),
            registered_outputs: self.outputs.len(),
            registered_inputs: self.inputs.len(),
            confirmations: self.confirmations,
            error: self.error.clone(),
        }
    }

    /// Returns the current serializable state of the [`JoinstrInner`] instance.
    ///
    /// # Returns
    /// A [`State`] struct containing the current state information.
    pub fn state(&self) -> Option<State> {
        let keys = if let Ok(keys) = self.client.get_keys() {
            keys
        } else {
            return None;
        };
        let relay = if let Some(relay) = &self.relay {
            relay.clone()
        } else {
            return None;
        };
        let electrum = self.electrum_client.as_ref().map(|c| (c.url(), c.port()));
        let pool = if let Some(pool) = &self.pool {
            pool.clone()
        } else {
            return None;
        };
        Some(State {
            role: self.role,
            step: self.step,
            pool_secret_key: keys.secret_key().to_secret_hex(),
            relay,
            electrum,
            pool,
            input: self.input.clone(),
            output: self.output.clone().map(|a| a.as_unchecked().clone()),
            network: self.network,
            final_tx: self.final_tx.clone(),
            peers: self.peers.clone(),
            outputs: self
                .outputs
                .iter()
                .map(|a| a.as_unchecked().clone())
                .collect(),
            inputs: self.inputs.iter().map(|i| i.to_json()).collect(),
        })
    }
}
