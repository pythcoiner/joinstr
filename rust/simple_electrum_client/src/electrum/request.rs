use super::{
    method::Method,
    params::{Params, TxGetArgs, VersionKind},
    types::ScriptHash,
};
use miniscript::bitcoin::{Script, Txid};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct Request {
    jsonrpc: String,
    pub id: usize,
    pub method: Method,

    #[serde(default)]
    params: Params,
}

impl Request {
    fn new(method: Method, params: Params) -> Self {
        Request {
            jsonrpc: "2.0".into(),
            id: 0,
            method,
            params,
        }
    }

    fn new_with_id(id: usize, method: Method, params: Params) -> Self {
        Request {
            jsonrpc: "2.0".into(),
            id,
            method,
            params,
        }
    }

    pub fn id(mut self, id: usize) -> Self {
        self.id = id;
        self
    }

    pub fn ping() -> Self {
        Self::new(Method::Ping, Params::None)
    }

    pub fn version(client_name: String, version: String) -> Self {
        Self::new(
            Method::Version,
            Params::Version((client_name, VersionKind::Single(version))),
        )
    }

    pub fn version_range(client_name: String, min: String, max: String) -> Self {
        Self::new(
            Method::Version,
            Params::Version((client_name, VersionKind::MinMax(min, max))),
        )
    }

    pub fn banner() -> Self {
        Self::new(Method::Banner, Params::None)
    }

    pub fn donation() -> Self {
        Self::new(Method::Donation, Params::None)
    }

    pub fn features() -> Self {
        Self::new(Method::Features, Params::None)
    }

    pub fn subscribe_peers() -> Self {
        Self::new(Method::ListPeers, Params::None)
    }

    pub fn header(height: usize) -> Self {
        Self::new(Method::BlockHeader, Params::BlockHeader((height,)))
    }

    pub fn headers(start: usize, count: usize) -> Self {
        Self::new(Method::BlockHeaders, Params::BlockHeaders((start, count)))
    }

    pub fn estimate_fee(block_target: u16) -> Self {
        Self::new(Method::EstimateFee, Params::EstimateFee((block_target,)))
    }

    pub fn subscribe_headers() -> Self {
        Self::new(Method::HeadersSubscribe, Params::None)
    }

    pub fn relay_fee() -> Self {
        Self::new(Method::RelayFee, Params::None)
    }

    pub fn sh_get_balance(script: &Script) -> Self {
        let sh = ScriptHash::new(script);
        Self::new(
            Method::ScriptHashGetBalance,
            Params::ScriptHashGetBalance((sh,)),
        )
    }

    pub fn sh_get_history(script: &Script) -> Self {
        let sh = ScriptHash::new(script);
        Self::new(
            Method::ScriptHashGetHistory,
            Params::ScriptHashGetHistory((sh,)),
        )
    }

    // NOTE: not supported by electrs
    // pub fn sh_get_mempool(script: &Script) -> Self {
    //     let sh = ScriptHash::new(script);
    //     Self::new(
    //         Method::ScriptHashGetMempool,
    //         Params::ScriptHashGetMempool((sh,)),
    //     )
    // }

    pub fn sh_list_unspent(script: &Script) -> Self {
        let sh = ScriptHash::new(script);
        Self::new(
            Method::ScriptHashListUnspent,
            Params::ScriptHashListUnspent((sh,)),
        )
    }

    pub fn subscribe_sh(script: &Script) -> Self {
        let sh = ScriptHash::new(script);
        Self::new(
            Method::ScriptHashSubscribe,
            Params::ScriptHashSubscribe((sh,)),
        )
    }

    pub fn unsubscribe_sh(script: &Script) -> Self {
        let sh = ScriptHash::new(script);
        Self::new(
            Method::ScriptHashUnsubscribe,
            Params::ScriptHashUnsubscribe((sh,)),
        )
    }

    pub fn tx_broadcast(tx: String) -> Self {
        Self::new(
            Method::TransactionBroadcast,
            Params::TransactionBroadcast((tx,)),
        )
    }

    pub fn tx_get(txid: Txid) -> Self {
        Self::new(
            Method::TransactionGet,
            Params::TransactionGet(TxGetArgs::Txid((txid,))),
        )
    }

    pub fn tx_get_verbose(txid: Txid) -> Self {
        Self::new(
            Method::TransactionGet,
            Params::TransactionGet(TxGetArgs::TxidVerbose(txid, true)),
        )
    }

    pub fn tx_get_merkle(txid: Txid, height: usize) -> Self {
        Self::new(
            Method::TransactionGetMerkle,
            Params::TransactionGetMerkle((txid, height)),
        )
    }

    pub fn tx_from_pos(height: usize, tx_pos: usize, merlkle: bool) -> Self {
        Self::new(
            Method::TransactionFromPosition,
            Params::TransactionFromPosition((height, tx_pos, merlkle)),
        )
    }

    pub fn get_fee_histogram() -> Self {
        Self::new(Method::FeeHistogram, Params::None)
    }
}

impl From<Request> for String {
    fn from(value: Request) -> Self {
        serde_json::to_string(&value).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use miniscript::bitcoin::OutPoint;

    use super::*;
    #[test]
    fn serialize() {
        assert_eq!(
            &serde_json::to_string(&Request::ping()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.ping","params":[]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::banner()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.banner","params":[]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::donation()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.donation_address","params":[]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::features()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.features","params":[]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::subscribe_peers()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.peers.subscribe","params":[]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::version("smart".into(), "1.4".into())).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.version","params":["smart","1.4"]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::version_range(
                "smart".into(),
                "1.1".into(),
                "1.4".into()
            ))
            .unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"server.version","params":["smart",["1.1","1.4"]]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::header(12345)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.block.header","params":[12345]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::headers(12345, 5)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.block.headers","params":[12345,5]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::estimate_fee(5)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.estimatefee","params":[5]}"#
        );

        assert_eq!(
            &serde_json::to_string(&Request::subscribe_headers()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.headers.subscribe","params":[]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::relay_fee()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.relayfee","params":[]}"#
        );
        let script = Script::from_bytes(&[0x00]);
        assert_eq!(
            &serde_json::to_string(&Request::sh_get_history(script)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.get_history","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        );
        // NOTE: not supported by electrs
        // assert_eq!(
        //     &serde_json::to_string(&Request::sh_get_mempool(script)).unwrap(),
        //     r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.get_mempool","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        // );
        assert_eq!(
            &serde_json::to_string(&Request::sh_list_unspent(script)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.listunspent","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::sh_get_balance(script)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.get_balance","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::subscribe_sh(script)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.subscribe","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::unsubscribe_sh(script)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.scripthash.unsubscribe","params":["1da0af1706a31185763837b33f1d90782c0a78bbe644a59c987ab3ff9c0b346e"]}"#
        );
        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:42",
        )
        .unwrap();
        let txid = outpoint.txid;
        assert_eq!(
            &serde_json::to_string(&Request::tx_get(txid)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.transaction.get","params":["5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456"]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::tx_get_verbose(txid)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.transaction.get","params":["5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456",true]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::tx_get_merkle(txid, 5)).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"blockchain.transaction.get_merkle","params":["5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456",5]}"#
        );
        assert_eq!(
            &serde_json::to_string(&Request::get_fee_histogram()).unwrap(),
            r#"{"jsonrpc":"2.0","id":0,"method":"mempool.get_fee_histogram","params":[]}"#
        );
    }

    #[test]
    fn batch_request() {
        let mut batch = Vec::new();
        let request = Request::ping();
        for i in 1..12usize {
            let mut r = request.clone();
            r.id = i;
            batch.push(r);
        }

        let str_req = serde_json::to_string(&batch).unwrap();
        let expected = r#"[{"jsonrpc":"2.0","id":1,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":2,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":3,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":4,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":5,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":6,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":7,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":8,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":9,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":10,"method":"server.ping","params":[]},{"jsonrpc":"2.0","id":11,"method":"server.ping","params":[]}]"#;

        assert_eq!(&str_req, expected);
    }
}
