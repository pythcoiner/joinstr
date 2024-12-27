use bitcoin::{Address, Amount};
use hex_conservative::FromHex;
use miniscript::bitcoin::{consensus::Decodable, OutPoint, Script, Transaction, TxOut, Txid};
use nostr_sdk::bitcoin::consensus::encode::serialize_hex;
use simple_electrum_client::{
    electrum::{
        request::Request,
        response::{
            ErrorResponse, Response, SHGetHistoryResponse, TxBroadcastResponse, TxGetResponse,
            TxGetResult,
        },
    },
    raw_client::{self, Client as RawClient},
};
use std::{collections::HashMap, fmt::Display, thread::sleep, time::Duration};

use crate::coinjoin::BitcoinBackend;

#[derive(Debug)]
pub enum Error {
    Electrum(raw_client::Error),
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
        Error::Electrum(value)
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
    pub fn new(url: &str, port: u16) -> Result<Self, Error> {
        let mut inner = RawClient::new_tcp(url, port);
        inner.try_connect()?;
        Ok(Client {
            inner,
            index: HashMap::new(),
            last_id: 0,
            url: url.into(),
            port,
        })
    }

    fn id(&mut self) -> usize {
        let id = self.last_id;
        self.last_id = self.last_id.wrapping_add(1);
        id
    }

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
