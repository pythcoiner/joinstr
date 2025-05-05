use backoff::Backoff;
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
    thread::{self},
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

pub fn short_hash(s: &ScriptBuf) -> String {
    let s = ScriptHash::new(s).to_string();
    short_string(s)
}

pub fn short_string(s: String) -> String {
    let head = 4;
    let tail = 4;
    if s.len() <= head + tail + 2 {
        // No need to truncate if string is short
        return s.to_string();
    }
    format!("{}..{}", &s[..head], &s[s.len() - tail..])
}

#[derive(Clone)]
pub enum CoinRequest {
    Subscribe(Vec<ScriptBuf>),
    History(Vec<ScriptBuf>),
    Txs(Vec<Txid>),
    Stop,
}

impl Debug for CoinRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Subscribe(vec) => {
                let hashes: Vec<_> = vec.iter().map(short_hash).collect();
                f.debug_tuple("Subscribe").field(&hashes).finish()
            }
            Self::History(vec) => {
                let hashes: Vec<_> = vec.iter().map(short_hash).collect();
                f.debug_tuple("History").field(&hashes).finish()
            }
            Self::Txs(arg0) => f.debug_tuple("Txs").field(arg0).finish(),
            Self::Stop => write!(f, "Stop"),
        }
    }
}

#[derive(Clone)]
pub enum CoinResponse {
    Status(BTreeMap<ScriptBuf, Option<String>>),
    History(BTreeMap<ScriptBuf, Vec<(Txid, Option<u64> /* height */)>>),
    Txs(Vec<Transaction>),
    Stopped,
    Error(String),
}

impl Debug for CoinResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Txs(vec) => {
                let txids: Vec<_> = vec.iter().map(|tx| tx.compute_txid()).collect();
                f.debug_tuple("Txs").field(&txids).finish()
            }
            Self::Status(map) => {
                let statuses: Vec<_> = map
                    .iter()
                    .map(|(spk, status)| {
                        format!(
                            "{} => {:?}",
                            short_hash(spk),
                            status.as_ref().map(|st| short_string(st.to_string()))
                        )
                    })
                    .collect();
                f.debug_tuple("Status").field(&statuses).finish()
            }
            Self::History(map) => {
                let map: Vec<_> = map
                    .iter()
                    .map(|(spk, v)| {
                        let conf: Vec<_> =
                            v.iter().filter(|(_, height)| height.is_some()).collect();
                        format!(
                            "{} => conf: {}, total: {}",
                            short_hash(spk),
                            conf.len(),
                            v.len()
                        )
                    })
                    .collect();
                f.debug_tuple("History").field(&map).finish()
            }
            Self::Stopped => write!(f, "Stopped"),
            Self::Error(e) => write!(f, "Error({})", e),
        }
    }
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
        let address = address.to_string().replace("ssl://", "");
        let mut inner = RawClient::new_ssl_maybe(&address, port, ssl);
        inner.try_connect()?;
        Ok(Client {
            inner,
            index: HashMap::new(),
            last_id: 0,
            url: address,
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
        let address = address.to_string().replace("ssl://", "");
        let mut inner = RawClient::new_ssl_maybe(&address, port, ssl).verif_certificate(false);
        inner.try_connect()?;
        Ok(Client {
            inner,
            index: HashMap::new(),
            last_id: 0,
            url: address,
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
        let mut reqid_spk_map = BTreeMap::new();
        let mut watched_spks_sh = BTreeMap::<usize /* request_id */, ScriptHash>::new();
        let mut sh_sbf_map = BTreeMap::<ScriptHash, ScriptBuf>::new();

        let mut last_request = None;

        fn responses_matches_requests(req: &[Request], resp: &[Response]) -> bool {
            req.iter()
                .all(|rq| resp.iter().any(|response| response.id() == Some(rq.id)))
        }

        let mut backoff = Backoff::new_ms(50);

        loop {
            let mut received = false;
            // Handle requests from consumer
            // NOTE: some server implementation (electrs for instance) will answer by an empty
            // response if it receive a request while it has not yes sent its previous response
            // so we need to make sure to not send a request before receiving the previous response
            if last_request.is_none() {
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
                                    log::debug!("Client::listen_txs() subscribe request: {sub:?}");
                                    let sh = ScriptHash::new(&spk);
                                    watched_spks_sh.insert(id, sh);
                                    sh_sbf_map.insert(sh, spk);
                                    batch.push(sub);
                                }
                                if !batch.is_empty() {
                                    log::debug!(
                                        "Client::listen_txs() last_request = {:?}",
                                        batch.len()
                                    );
                                    last_request = Some(batch.clone());

                                    let mut retry = 0usize;
                                    while let Err(e) =
                                        self.inner.try_send_batch(batch.iter().collect())
                                    {
                                        retry += 1;
                                        if retry > 10 {
                                            send.send(CoinResponse::Error(format!("electrum::Client::listen_txs() Fail to send bacth request: {:?}", e)).into()).expect("caller dropped");
                                        }
                                        thread::sleep(Duration::from_millis(50));
                                    }
                                }
                            }
                            CoinRequest::History(sbfs) => {
                                let mut batch = vec![];
                                for spk in sbfs {
                                    let mut history = Request::sh_get_history(&spk);
                                    let id = self.register(&mut history);
                                    log::debug!(
                                        "Client::listen_txs() history request: {history:?}"
                                    );
                                    reqid_spk_map.insert(id, spk);
                                    batch.push(history);
                                }
                                if !batch.is_empty() {
                                    log::debug!(
                                        "Client::listen_txs() last_request = {:?}",
                                        batch.len()
                                    );
                                    last_request = Some(batch.clone());

                                    let mut retry = 0usize;
                                    while let Err(e) =
                                        self.inner.try_send_batch(batch.iter().collect())
                                    {
                                        retry += 1;
                                        if retry > 10 {
                                            send.send(CoinResponse::Error(format!("electrum::Client::listen_txs() Fail to send bacth request: {:?}", e)).into()).expect("caller dropped");
                                        }
                                        thread::sleep(Duration::from_millis(50));
                                    }
                                }
                            }
                            CoinRequest::Txs(txids) => {
                                let mut batch = vec![];
                                for txid in txids {
                                    let mut tx = Request::tx_get(txid);
                                    self.register(&mut tx);
                                    log::debug!("Client::listen_txs() txs request: {tx:?}");
                                    batch.push(tx);
                                }
                                if !batch.is_empty() {
                                    log::debug!(
                                        "Client::listen_txs() last_request = {:?}",
                                        batch.len()
                                    );
                                    last_request = Some(batch.clone());

                                    let mut retry = 0usize;
                                    while let Err(e) =
                                        self.inner.try_send_batch(batch.iter().collect())
                                    {
                                        retry += 1;
                                        if retry > 10 && send.send(CoinResponse::Error(format!("electrum::Client::listen_txs() Fail to send bacth request: {:?}", e)).into()).is_err() {
                                        // NOTE: caller has dropped the channel
                                        // == Close request
                                        return;
                                                    }
                                        thread::sleep(Duration::from_millis(50));
                                    }
                                }
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
            }
            // Handle responses from electrum server
            match self.inner.try_recv(&self.index) {
                Ok(Some(r)) => {
                    log::debug!("Client::listen_txs() from electrum: {r:#?}");
                    let r_match = if let Some(req) = &last_request {
                        responses_matches_requests(req, &r)
                    } else {
                        false
                    };
                    if r_match {
                        last_request = None;
                    } else if let Some(last_req) = &last_request {
                        log::debug!("Client::listen_txs() request not match resend last request");
                        thread::sleep(Duration::from_millis(100));
                        self.inner
                            .try_send_batch(last_req.iter().collect())
                            .unwrap();
                    }

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
                                reqid_spk_map.remove(&id);
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
                                if send
                                    .send(CoinResponse::Error(e.to_string()).into())
                                    .is_err()
                                {
                                    // NOTE: caller has dropped the channel
                                    // == Close request
                                    return;
                                }
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
                    if send
                        .send(CoinResponse::Error(e.to_string()).into())
                        .is_err()
                    {
                        // NOTE: caller has dropped the channel
                        // == Close request
                        return;
                    }
                }
            }
            if received {
                continue;
            }
            backoff.snooze();
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

    /// Returns the URL of the electrum client.
    ///
    /// # Returns
    /// A `String` containing the URL of the electrum server.
    pub fn url(&self) -> String {
        self.url.clone()
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
