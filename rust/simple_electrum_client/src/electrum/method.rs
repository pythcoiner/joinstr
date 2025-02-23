use miniscript::serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Method {
    #[serde(rename = "server.banner")]
    Banner,
    #[serde(rename = "blockchain.block.header")]
    BlockHeader,
    #[serde(rename = "blockchain.block.headers")]
    BlockHeaders,
    #[serde(rename = "blockchain.transaction.broadcast")]
    TransactionBroadcast,
    #[serde(rename = "server.donation_address")]
    Donation,
    #[serde(rename = "blockchain.estimatefee")]
    EstimateFee,
    #[serde(rename = "server.features")]
    Features,
    #[serde(rename = "blockchain.headers.subscribe")]
    HeadersSubscribe,
    #[serde(rename = "mempool.get_fee_histogram")]
    FeeHistogram,
    #[serde(rename = "server.peers.subscribe")]
    ListPeers,
    #[serde(rename = "server.ping")]
    Ping,
    #[serde(rename = "blockchain.relayfee")]
    RelayFee,
    #[serde(rename = "blockchain.scripthash.get_balance")]
    ScriptHashGetBalance,
    #[serde(rename = "blockchain.scripthash.get_history")]
    ScriptHashGetHistory,
    // NOTE: not supported by electrs
    // #[serde(rename = "blockchain.scripthash.get_mempool")]
    // ScriptHashGetMempool,
    #[serde(rename = "blockchain.scripthash.listunspent")]
    ScriptHashListUnspent,
    #[serde(rename = "blockchain.scripthash.subscribe")]
    ScriptHashSubscribe,
    #[serde(rename = "blockchain.scripthash.unsubscribe")]
    ScriptHashUnsubscribe,
    #[serde(rename = "blockchain.transaction.get")]
    TransactionGet,
    #[serde(rename = "blockchain.transaction.get_merkle")]
    TransactionGetMerkle,
    #[serde(rename = "blockchain.transaction.id_from_pos")]
    TransactionFromPosition,
    #[serde(rename = "server.version")]
    Version,
}

impl Debug for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Banner => write!(f, "server.banner"),
            Self::BlockHeader => write!(f, "blockchain.block.header"),
            Self::BlockHeaders => write!(f, "blockchain.block.headers"),
            Self::TransactionBroadcast => write!(f, "blockchain.transaction.broadcast"),
            Self::Donation => write!(f, "server.donation_address"),
            Self::EstimateFee => write!(f, "blockchain.estimatefee"),
            Self::Features => write!(f, "server.features"),
            Self::HeadersSubscribe => write!(f, "blockchain.headers.subscribe"),
            Self::FeeHistogram => write!(f, "mempool.get_fee_histogram"),
            Self::ListPeers => write!(f, "server.peers.subscribe"),
            Self::Ping => write!(f, "server.ping"),
            Self::RelayFee => write!(f, "blockchain.relayfee"),
            Self::ScriptHashGetBalance => write!(f, "blockchain.scripthash.get_balance"),
            Self::ScriptHashGetHistory => write!(f, "blockchain.scripthash.get_history"),
            Self::ScriptHashListUnspent => write!(f, "blockchain.scripthash.listunspent"),
            Self::ScriptHashSubscribe => write!(f, "blockchain.scripthash.subscribe"),
            Self::ScriptHashUnsubscribe => write!(f, "blockchain.scripthash.unsubscribe"),
            Self::TransactionGet => write!(f, "blockchain.transaction.get"),
            Self::TransactionGetMerkle => write!(f, "blockchain.transaction.get_merkle"),
            Self::TransactionFromPosition => write!(f, "blockchain.transaction.id_from_pos"),
            Self::Version => write!(f, "server.version"),
            // NOTE: not supported by electrs
            // Self::ScriptHashGetMempool => write!(f, "blockchain.scripthash.get_mempool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Method::*;

    macro_rules! debug_json {
        ($value:expr) => {{
            use serde_json::to_string;

            let json_str = to_string(&$value).unwrap();
            let json_str = json_str.trim_matches('"');

            let debug_str = format!("{:?}", $value);

            assert_eq!(
                json_str, debug_str,
                "Debug and JSON representations do not match"
            );
        }};
    }

    #[test]
    fn debug() {
        debug_json!(Version);
        debug_json!(TransactionFromPosition);
        debug_json!(TransactionGetMerkle);
        debug_json!(TransactionGet);
        debug_json!(ScriptHashUnsubscribe);
        debug_json!(ScriptHashSubscribe);
        debug_json!(ScriptHashListUnspent);
        debug_json!(ScriptHashGetHistory);
        debug_json!(ScriptHashGetBalance);
        debug_json!(RelayFee);
        debug_json!(Ping);
        debug_json!(ListPeers);
        debug_json!(FeeHistogram);
        debug_json!(HeadersSubscribe);
        debug_json!(Features);
        debug_json!(EstimateFee);
        debug_json!(Donation);
        debug_json!(TransactionBroadcast);
        debug_json!(BlockHeaders);
        debug_json!(BlockHeader);
        debug_json!(Banner);
    }
}
