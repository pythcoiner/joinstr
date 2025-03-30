use bitcoin::{consensus, Address, Amount, ScriptBuf};
use hex_conservative::FromHex;
use miniscript::bitcoin::{consensus::Decodable, OutPoint, Script, Transaction, TxOut, Txid};
use simple_electrum_client::{
    electrum::{
        request::Request,
        response::{
            ErrorResponse, HistoryResult, Response, SHGetHistoryResponse, SHNotification,
            SHSubscribeResponse, TxBroadcastResponse, TxGetResponse, TxGetResult,
        },
        types::ScriptHash,
    },
    raw_client::{self, Client as RawClient},
};
use simple_nostr_client::nostr::bitcoin::consensus::encode::serialize_hex;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::{Debug, Display},
    sync::mpsc,
    thread::{self, sleep},
    time::Duration,
};

use crate::coinjoin::BitcoinBackend;

#[derive(Debug, Clone)]
pub enum Error {
    Electrum(String),
    TxParsing,
    WrongResponse,
    WrongOutPoint,
    TxDoesNotExists,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Electrum(e) => write!(f, "{e:?}"),
            Error::TxParsing => write!(f, "Fail to parse the transaction"),
            Error::WrongResponse => write!(f, "Wrong response from electrum server"),
            Error::WrongOutPoint => write!(f, "Requested outpoint did not exists"),
            Error::TxDoesNotExists => write!(f, "Requested transaction did not exists"),
        }
    }
}

impl From<raw_client::Error> for Error {
    fn from(value: raw_client::Error) -> Self {
        Error::Electrum(format!("{value:?}"))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CoinStatus {
    Unconfirmed,
    Confirmed,
    Spend,
}

#[derive(Debug, Clone)]
pub enum CoinRequest {
    Subscribe(Vec<ScriptBuf>),
    History(Vec<ScriptBuf>),
    Txs(Vec<Txid>),
    Stop,
}

#[derive(Debug, Clone)]
pub enum CoinResponse {
    Status(BTreeMap<ScriptBuf, Option<String>>),
    History(BTreeMap<ScriptBuf, Vec<(Txid, Option<u64> /* height */)>>),
    Txs(Vec<Transaction>),
    Stopped,
    Error(String),
}

#[derive(Debug)]
pub struct Client {
    inner: RawClient,
    index: HashMap<usize, Request>,
    last_id: usize,
    url: String,
    port: u16,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Client::new(&self.url, self.port).unwrap()
    }
}

impl Client {
    /// Create a new electrum client.
    ///
    /// # Arguments
    /// * `address` - url/ip of the electrum server as String
    /// * `port` - port of the electrum server
    pub fn new(address: &str, port: u16) -> Result<Self, Error> {
        let ssl = address.starts_with("ssl://");
        let mut inner = RawClient::new_ssl_maybe(address, port, ssl);
        inner.try_connect()?;
        Ok(Client {
            inner,
            index: HashMap::new(),
            last_id: 0,
            url: address.into(),
            port,
        })
    }

    /// Create a new local electrum client: SSL certificate validation id disabled in
    ///   order to be used with self-signed certificates.
    ///
    /// # Arguments
    /// * `address` - url/ip of the electrum server as String
    /// * `port` - port of the electrum server
    pub fn new_local(address: &str, port: u16) -> Result<Self, Error> {
        let ssl = address.starts_with("ssl://");
        let mut inner = RawClient::new_ssl_maybe(address, port, ssl).verif_certificate(false);
        inner.try_connect()?;
        Ok(Client {
            inner,
            index: HashMap::new(),
            last_id: 0,
            url: address.into(),
            port,
        })
    }

    /// Generate a new request id
    fn id(&mut self) -> usize {
        self.last_id = self.last_id.wrapping_add(1);
        self.last_id
    }

    fn register(&mut self, req: &mut Request) -> usize {
        let id = self.id();
        req.id = id;
        self.index.insert(req.id, req.clone());
        id
    }

    pub fn listen<RQ, RS>(self) -> (mpsc::Sender<RQ>, mpsc::Receiver<RS>)
    where
        RQ: Into<CoinRequest> + Debug + Send + 'static,
        RS: From<CoinResponse> + Debug + Send + 'static,
    {
        let (sender, request) = mpsc::channel();
        let (response, receiver) = mpsc::channel();
        thread::spawn(move || self.listen_txs(response, request));

        (sender, receiver)
    }

    fn listen_txs<RQ, RS>(mut self, send: mpsc::Sender<RS>, recv: mpsc::Receiver<RQ>)
    where
        RQ: Into<CoinRequest> + Debug + Send + 'static,
        RS: From<CoinResponse> + Debug + Send + 'static,
    {
        log::debug!("Client::listen_txs()");
        // TODO: this need to be cleanup to drop request that have been responded
        let mut reqid_spk_map = BTreeMap::new();
        let mut watched_spks_sh = BTreeMap::<usize /* request_id */, ScriptHash>::new();
        let mut sh_sbf_map = BTreeMap::<ScriptHash, ScriptBuf>::new();

        loop {
            let mut received = false;
            // Handle requests from consumer
            match recv.try_recv() {
                Ok(rq) => {
                    log::debug!("Client::listen_txs() recv request: {rq:#?}");
                    received = true;
                    let rq: CoinRequest = rq.into();
                    match rq {
                        CoinRequest::Subscribe(spks) => {
                            let mut batch = vec![];
                            for spk in spks {
                                let mut sub = Request::subscribe_sh(&spk);
                                let id = self.register(&mut sub);
                                log::debug!("Client::listen_txs() subscribe id: {sub:?}");
                                let sh = ScriptHash::new(&spk);
                                watched_spks_sh.insert(id, sh);
                                sh_sbf_map.insert(sh, spk);
                                batch.push(sub);
                            }
                            // TODO: do not unwrap
                            self.inner.try_send_batch(batch.iter().collect()).unwrap();
                        }
                        CoinRequest::History(sbfs) => {
                            let mut batch = vec![];
                            for spk in sbfs {
                                let mut history = Request::sh_get_history(&spk);
                                let id = self.register(&mut history);
                                reqid_spk_map.insert(id, spk);
                                batch.push(history);
                            }
                            // TODO: do not unwrap
                            self.inner.try_send_batch(batch.iter().collect()).unwrap();
                        }
                        CoinRequest::Txs(txids) => {
                            let mut batch = vec![];
                            for txid in txids {
                                let mut tx = Request::tx_get(txid);
                                self.register(&mut tx);
                                batch.push(tx);
                            }
                            // TODO: do not unwrap
                            self.inner.try_send_batch(batch.iter().collect()).unwrap();
                        }
                        CoinRequest::Stop => {
                            send.send(CoinResponse::Stopped.into()).unwrap();
                            return;
                        }
                    };
                }
                Err(e) => {
                    match e {
                        mpsc::TryRecvError::Empty => {}
                        mpsc::TryRecvError::Disconnected => {
                            // NOTE: caller has dropped the channel
                            // == Close request
                            return;
                        }
                    }
                }
            }
            // Handle responses from electrum server
            match self.inner.try_recv(&self.index) {
                Ok(Some(r)) => {
                    log::debug!("Client::listen_txs() from electrum: {r:#?}");
                    received = true;
                    let mut statuses = BTreeMap::new();
                    let mut txs = Vec::new();
                    // let mut txid_to_get = Vec::new();
                    let mut histories = BTreeMap::new();
                    for r in r {
                        match r {
                            Response::SHSubscribe(SHSubscribeResponse { result: status, id }) => {
                                let sh = watched_spks_sh.get(&id).expect("already inserted");
                                let sbf = sh_sbf_map.get(sh).expect("already inserted");
                                statuses.insert(sbf.clone(), status);
                            }
                            Response::SHNotification(SHNotification {
                                status: (sh, status),
                                ..
                            }) => {
                                let sbf = sh_sbf_map.get(&sh).expect("already inserted");
                                statuses.insert(sbf.clone(), status);
                            }
                            Response::SHGetHistory(SHGetHistoryResponse { history, id }) => {
                                let spk = reqid_spk_map.get(&id).expect("already inserted").clone();
                                let mut spk_hist = vec![];
                                for tx in history {
                                    let HistoryResult { txid, height, .. } = tx;
                                    let height = if height < 1 {
                                        None
                                    } else {
                                        Some(height as u64)
                                    };
                                    spk_hist.push((txid, height));
                                }
                                histories.insert(spk, spk_hist);
                            }
                            Response::TxGet(TxGetResponse {
                                result: TxGetResult::Raw(raw_tx),
                                ..
                            }) => {
                                let tx: Transaction =
                            // TODO: do not unwrap
                                    consensus::encode::deserialize_hex(&raw_tx).unwrap();
                                txs.push(tx);
                            }
                            Response::Error(e) => {
                                // TODO: do not unwrap
                                send.send(CoinResponse::Error(e.to_string()).into())
                                    .unwrap();
                            }
                            _ => {}
                        }
                    }
                    if !histories.is_empty() {
                        let rsp = CoinResponse::History(histories);
                        log::debug!("Client::listen_txs() send response: {rsp:#?}");
                        send.send(rsp.into()).unwrap();
                    }
                    if !statuses.is_empty() {
                        let rsp = CoinResponse::Status(statuses);
                        log::debug!("Client::listen_txs() send response: {rsp:#?}");
                        send.send(rsp.into()).unwrap();
                    }
                    // let mut txs = Vec::new();
                    if !txs.is_empty() {
                        let rsp = CoinResponse::Txs(txs);
                        log::debug!("Client::listen_txs() send response: {rsp:#?}");
                        send.send(rsp.into()).unwrap();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    // TODO: do not unwrap
                    send.send(CoinResponse::Error(e.to_string()).into())
                        .unwrap();
                }
            }
            if received {
                continue;
            }
            // FIXME: maybe 50ms is WAY too much
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Try to get a transaction by its txid
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - fail to send the request
    ///   - parsing response fails
    ///   - the response is not of expected type
    ///   - the transaction does not exists
    pub fn get_tx(&mut self, txid: Txid) -> Result<Transaction, Error> {
        let request = Request::tx_get(txid).id(self.id());
        self.inner.try_send(&request)?;
        let req_id = request.id;
        self.index.insert(request.id, request);
        let resp = match self.inner.recv(&self.index) {
            Ok(r) => r,
            Err(e) => {
                self.index.remove(&req_id);
                return Err(e.into());
            }
        };
        for r in resp {
            if let Response::TxGet(TxGetResponse {
                id,
                result: TxGetResult::Raw(res),
            }) = r
            {
                if req_id == id {
                    self.index.remove(&req_id);
                    let raw_tx = match Vec::<u8>::from_hex(&res) {
                        Ok(raw) => raw,
                        Err(_) => {
                            return Err(Error::TxParsing);
                        }
                    };
                    let tx: Result<Transaction, _> =
                        Decodable::consensus_decode(&mut raw_tx.as_slice());
                    return tx.map_err(|_| Error::TxParsing);
                }
            } else if let Response::Error(ErrorResponse { id, .. }) = r {
                if req_id == id {
                    self.index.remove(&req_id);
                    // NOTE: it's very likely if we receive an error response from the server
                    // it's because the txid does not match any Transaction, but maybe we can
                    // do a better handling of the error case (for this we need check if responses
                    // from all electrum server implementations are consistant).
                    return Err(Error::TxDoesNotExists);
                }
            }
        }
        self.index.remove(&req_id);
        Err(Error::WrongResponse)
    }

    /// Get coins that pay to the given spk and their related transaction.
    /// This method will make several calls to the electrum server:
    ///   - it will first request a list of all transactions txid that have
    ///     an output paying to the spk.
    ///   - it will then fetch all txs, store them and extract all the coins
    ///     that pay to the given spk.
    ///   - it will return a list of (TxOut, OutPoint) and a map of transactions.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - a call to the electrum server fail
    #[allow(clippy::type_complexity)]
    pub fn get_coins_at(
        &mut self,
        script: &Script,
    ) -> Result<(Vec<(TxOut, OutPoint)>, HashMap<Txid, Transaction>), Error> {
        let mut txouts = Vec::new();
        let mut transactions = HashMap::new();
        let txs = self.get_coins_tx_at(script)?;
        for txid in txs {
            let tx = self.get_tx(txid)?;
            for (i, txout) in tx.output.iter().enumerate() {
                if *txout.script_pubkey == *script {
                    let outpoint = OutPoint {
                        txid,
                        vout: i as u32,
                    };
                    txouts.push((txout.clone(), outpoint));
                }
            }
            transactions.insert(txid, tx);
        }
        Ok((txouts, transactions))
    }

    /// Get a list of txid of all transaction that have an output paying to the
    ///   given spk
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - fail sending the request
    ///   - receive a wrong response
    pub fn get_coins_tx_at(&mut self, script: &Script) -> Result<Vec<Txid>, Error> {
        let request = Request::sh_get_history(script).id(self.id());
        self.inner.try_send(&request)?;
        let req_id = request.id;
        self.index.insert(request.id, request);
        let resp = match self.inner.recv(&self.index) {
            Ok(r) => r,
            Err(e) => {
                self.index.remove(&req_id);
                return Err(e.into());
            }
        };
        for r in resp {
            if let Response::SHGetHistory(SHGetHistoryResponse { id, history }) = r {
                if req_id == id {
                    self.index.remove(&req_id);
                    let history: Vec<_> = history.into_iter().map(|r| r.txid).collect();
                    return Ok(history);
                }
            }
        }
        self.index.remove(&req_id);
        Err(Error::WrongResponse)
    }

    /// Broadcast the given transaction.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    ///   - fail to send the request
    ///   - get a wrong response
    pub fn broadcast(&mut self, tx: &Transaction) -> Result<(), Error> {
        let raw_tx = serialize_hex(tx);
        log::debug!("electrum::Client().broadcast(): {:?}", raw_tx);
        let request = Request::tx_broadcast(raw_tx);
        self.inner.try_send(&request)?;
        sleep(Duration::from_secs(2));
        let req_id = request.id;
        self.index.insert(request.id, request);
        let resp = match self.inner.recv(&self.index) {
            Ok(r) => r,
            Err(e) => {
                self.index.remove(&req_id);
                return Err(e.into());
            }
        };
        log::debug!(
            "electrum::Client().broadcast(): receive response: {:?}",
            resp
        );
        for r in resp {
            if let Response::TxBroadcast(TxBroadcastResponse { id, .. }) = r {
                if req_id == id {
                    self.index.remove(&req_id);
                    return Ok(());
                }
            }
        }
        self.index.remove(&req_id);
        Err(Error::WrongResponse)
    }
}

impl BitcoinBackend for Client {
    type Error = Error;
    fn address_already_used(&mut self, addr: &Address) -> Result<bool, Error> {
        let spk = addr.script_pubkey();
        let txs = self.get_coins_tx_at(&spk)?;
        Ok(!txs.is_empty())
    }

    fn get_outpoint_value(&mut self, outpoint: OutPoint) -> Result<Option<Amount>, Error> {
        let tx = match self.get_tx(outpoint.txid) {
            Ok(tx) => tx,
            Err(e) => match e {
                // NOTE: it's very likely if we receive an error response from the server
                // it's because the txid does not match any Transaction, but maybe we can
                // do a better handling of the error case (for this we need check if responses
                // from all electrum server implementations are consistant).
                Error::TxDoesNotExists => return Ok(None),
                e => return Err(e),
            },
        };
        Ok(Some(
            tx.output
                .get(outpoint.vout as usize)
                .ok_or(Error::WrongOutPoint)?
                .value,
        ))
    }
}
