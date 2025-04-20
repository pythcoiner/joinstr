use std::{collections::HashMap, fmt::Display, str::FromStr};

use super::{method::Method, params::VersionKind, request::Request, types::ScriptHash, Error};
use miniscript::bitcoin::Txid;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, PartialEq, Clone)]
pub enum Response {
    HeaderNotif(HeaderNotification),
    BatchHeaderNotif(BatchHeaderNotif),
    SHNotification(SHNotification),
    Ping(PingResponse),
    Banner(BannerResponse),
    Header(HeaderResponse),
    Headers(HeadersResponse),
    Version(VersionResponse),
    TxGet(TxGetResponse),
    SHSubscribe(SHSubscribeResponse),
    SHUnsubscribe(SHUnsubscribeResponse),
    SHGetBalance(SHGetBalanceResponse),
    SHGetHistory(SHGetHistoryResponse),
    SHGetMempool(SHGetMempoolResponse),
    SHListUnspent(SHListUnspentResponse),
    Error(ErrorResponse),
    Features(FeaturesResponse),
    TxBroadcast(TxBroadcastResponse),
    Donation(DonationResponse),
    EstimateFee(EstimateFeeResponse),
    FeeHistogram(FeeHistogramResponse),
    RelayFee(RelayFeeResponse),
    TxGetMerkle(TxGetMerkleResponse),
    TxFromposition(TxFromPositionResponse),
    ListPeers(ListPeersResponse),
}

impl From<Response> for Vec<Response> {
    fn from(val: Response) -> Self {
        vec![val]
    }
}

impl Response {
    pub fn id(&self) -> Option<usize> {
        match self {
            Response::Ping(PingResponse { id, .. }) => Some(*id),
            Response::Banner(BannerResponse { id, .. }) => Some(*id),
            Response::Header(HeaderResponse { id, .. }) => Some(*id),
            Response::Headers(HeadersResponse { id, .. }) => Some(*id),
            Response::Version(VersionResponse { id, .. }) => Some(*id),
            Response::TxGet(TxGetResponse { id, .. }) => Some(*id),
            Response::SHSubscribe(SHSubscribeResponse { id, .. }) => Some(*id),
            Response::SHUnsubscribe(SHUnsubscribeResponse { id, .. }) => Some(*id),
            Response::SHGetBalance(SHGetBalanceResponse { id, .. }) => Some(*id),
            Response::SHGetHistory(SHGetHistoryResponse { id, .. }) => Some(*id),
            Response::SHGetMempool(SHGetMempoolResponse { id, .. }) => Some(*id),
            Response::SHListUnspent(SHListUnspentResponse { id, .. }) => Some(*id),
            Response::Error(ErrorResponse { id, .. }) => Some(*id),
            Response::Features(FeaturesResponse { id, .. }) => Some(*id),
            Response::TxBroadcast(TxBroadcastResponse { id, .. }) => Some(*id),
            Response::Donation(DonationResponse { id, .. }) => Some(*id),
            Response::EstimateFee(EstimateFeeResponse { id, .. }) => Some(*id),
            Response::FeeHistogram(FeeHistogramResponse { id, .. }) => Some(*id),
            Response::RelayFee(RelayFeeResponse { id, .. }) => Some(*id),
            Response::TxGetMerkle(TxGetMerkleResponse { id, .. }) => Some(*id),
            Response::TxFromposition(TxFromPositionResponse { id, .. }) => Some(*id),
            Response::ListPeers(ListPeersResponse { id, .. }) => Some(*id),
            _ => None,
        }
    }
}

pub struct ResponseBatch {
    pub batch: Vec<Response>,
}

pub fn parse_str_response(
    raw: &str,
    index: &HashMap<usize, Request>,
) -> Result<Vec<Response>, Error> {
    // first we check if it's a batch
    let batch = ResponseBatch::from_str(raw, index)?;
    if let Some(b) = batch {
        return Ok(b.batch);
    }
    // then try to parse a single Response
    Ok(Response::try_parse(raw, index)?.into())
}

impl ResponseBatch {
    pub fn from_str(s: &str, index: &HashMap<usize, Request>) -> Result<Option<Self>, Error> {
        let parsed: Result<Vec<Value>, _> = serde_json::from_str(s);
        if let Ok(parsed) = parsed {
            let mut batch = Vec::<Response>::new();
            for response in parsed {
                let raw =
                    serde_json::to_string(&response).expect("parsing Value to string do not fail!");
                batch.push(Response::try_parse(&raw, index)?);
            }
            Ok(Some(ResponseBatch { batch }))
        } else {
            Ok(None)
        }
    }
}

macro_rules! parse {
    ($method:ident, $response_type:ty, $raw:expr) => {{
        let r: $response_type =
            serde_json::from_str($raw).map_err(|_| Error::ResponseParsing($raw.into()))?;
        Ok(Self::$method(r))
    }};
}

impl Response {
    pub fn parse(raw: &str, index: &HashMap<usize, Request>) -> Response {
        Self::try_parse(raw, index).unwrap()
    }

    pub fn try_parse(raw: &str, index: &HashMap<usize, Request>) -> Result<Response, Error> {
        log::debug!("Response::try_parse() {raw}");
        // first we handle the case of a single error
        let error: Result<ErrorResponse, _> = serde_json::from_str(raw);
        if let Ok(e) = error {
            return Ok(Response::Error(e));
        }

        // then we handle the Batch Header Notification case
        let header_notif: Result<BatchHeaderNotif, _> = serde_json::from_str(raw);
        if let Ok(n) = header_notif {
            return Ok(Response::BatchHeaderNotif(n));
        }

        // then we handle the ScriptHash Notification case
        let sh_notif: Result<SHNotification, _> = serde_json::from_str(raw);
        if let Ok(n) = sh_notif {
            return Ok(Response::SHNotification(n));
        }

        // the we handle the case we need to match request/response id
        let rr: RawResponse = serde_json::from_str(raw)
            .map_err(|e| Error::RawResponseParsing(format!("Fail to parse `{}`: {:?}", raw, e)))?;
        let request = index.get(&rr.id).ok_or(Error::ResponseId(rr.id))?;
        match request.method {
            Method::Ping => parse!(Ping, PingResponse, raw),
            Method::Banner => parse!(Banner, BannerResponse, raw),
            Method::HeadersSubscribe => parse!(HeaderNotif, HeaderNotification, raw),
            Method::BlockHeader => parse!(Header, HeaderResponse, raw),
            Method::BlockHeaders => parse!(Headers, HeadersResponse, raw),
            Method::Version => parse!(Version, VersionResponse, raw),
            Method::TransactionGet => parse!(TxGet, TxGetResponse, raw),
            Method::ScriptHashSubscribe => parse!(SHSubscribe, SHSubscribeResponse, raw),
            Method::ScriptHashUnsubscribe => parse!(SHUnsubscribe, SHUnsubscribeResponse, raw),
            Method::ScriptHashGetBalance => parse!(SHGetBalance, SHGetBalanceResponse, raw),
            Method::ScriptHashGetHistory => parse!(SHGetHistory, SHGetHistoryResponse, raw),
            Method::ScriptHashListUnspent => parse!(SHListUnspent, SHListUnspentResponse, raw),
            // NOTE: not supported by electrs
            // Method::ScriptHashGetMempool => parse!(SHGetMempool, SHGetMempoolResponse, raw),
            Method::Features => parse!(Features, FeaturesResponse, raw),
            Method::Donation => parse!(Donation, DonationResponse, raw),
            Method::EstimateFee => parse!(EstimateFee, EstimateFeeResponse, raw),
            Method::FeeHistogram => parse!(FeeHistogram, FeeHistogramResponse, raw),
            Method::RelayFee => parse!(RelayFee, RelayFeeResponse, raw),
            Method::TransactionGetMerkle => parse!(TxGetMerkle, TxGetMerkleResponse, raw),
            Method::TransactionFromPosition => parse!(TxFromposition, TxFromPositionResponse, raw),
            Method::TransactionBroadcast => parse!(TxBroadcast, TxBroadcastResponse, raw),
            Method::ListPeers => todo!(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ErrorResult {
    pub code: usize,
    pub message: String,
}

impl Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Electrum error {}: {}",
            self.error.code, self.error.message
        )
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ErrorResponse {
    pub id: usize,
    pub error: ErrorResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHNotification {
    pub method: Method,
    #[serde(rename = "params")]
    pub status: (ScriptHash, Option<String>),
}

impl FromStr for SHNotification {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let notif: Self =
            serde_json::from_str(value).map_err(|_| Error::ResponseParsing(value.into()))?;
        if let Method::ScriptHashSubscribe = notif.method {
            Ok(notif)
        } else {
            Err(Error::WrongMethod)
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RawResponse {
    jsonrpc: String,
    pub id: usize,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BannerResponse {
    pub id: usize,
    pub result: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct Header {
    pub height: usize,
    #[serde(rename = "hex")]
    pub raw_header: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SingleHeaderNotif {
    pub id: usize,
    #[serde(rename = "result")]
    pub header: Header,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BatchHeaderNotif {
    pub method: Method,
    #[serde(rename = "params")]
    pub headers: Vec<Header>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum HeaderNotification {
    Single(SingleHeaderNotif),
    Batch(BatchHeaderNotif),
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct HeaderResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub raw_header: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Headers {
    pub count: usize,
    #[serde(rename = "hex")]
    pub raw_headers: String,
    pub max: usize,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct HeadersResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub headers: Headers,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TxBroadcastResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub txid: Txid,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct DonationResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub address: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct EstimateFeeResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub fee: OptionalFee,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum Port {
    String(String),
    U16(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Host {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_port: Option<Port>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssl_port: Option<Port>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Hosts {
    Single(Host),
    Map(HashMap<String, Host>),
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FeaturesResult {
    #[serde(rename = "genesis_hash")]
    pub genesis: String,
    pub hosts: Hosts,
    pub protocol_max: String,
    pub protocol_min: String,
    pub pruning: Option<usize>,
    pub server_version: String,
    pub hash_function: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FeaturesResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub features: FeaturesResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct FeeHistogramResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub histogram: Vec<(usize, usize)>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct PingResponse {
    pub id: usize,
    // result should always be `null`
    pub result: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum OptionalFee {
    Fee(f64),
    None(i64),
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct RelayFeeResponse {
    pub id: usize,
    #[serde(rename = "result")]
    // TODO: handle
    pub fee: OptionalFee,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHSubscribeResponse {
    pub id: usize,
    pub result: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHUnsubscribeResponse {
    pub id: usize,
    pub result: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BalanceResult {
    pub confirmed: i64,
    pub unconfirmed: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHGetBalanceResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub balance: BalanceResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct HistoryResult {
    // NOTE: as mempool txs are appended at the end of the response:
    //  height for mempool txs => 0 if all inputs are confirmed, and -1 otherwise
    //  height for confirmed txs => block height
    pub height: i128,
    #[serde(rename = "tx_hash")]
    pub txid: Txid,
    pub fee: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHGetHistoryResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub history: Vec<HistoryResult>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHGetMempoolResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub mempool: Vec<HistoryResult>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct UtxoResult {
    pub height: usize,
    #[serde(rename = "tx_hash")]
    pub txid: Txid,
    #[serde(rename = "tx_pos")]
    pub vout: usize,
    pub value: usize,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SHListUnspentResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub unspent: Vec<UtxoResult>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct VerboseTx {
    pub blockhash: String,
    pub blocktime: usize,
    pub confirmations: usize,
    pub locktime: usize,
    pub size: usize,
    pub time: usize,
    pub version: usize,
    pub txid: String,
    #[serde(rename = "hex")]
    pub raw_tx: String,
    // TODO: better parsing of vin/vout
    pub vin: Value,
    pub vout: Value,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum TxGetResult {
    Raw(String),
    Verbose(Box<VerboseTx>),
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TxGetResponse {
    pub id: usize,
    pub result: TxGetResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct GetMerkleResult {
    merkle: Vec<String>,
    block_height: usize,
    #[serde(rename = "pos")]
    tx_pos: usize,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TxGetMerkleResponse {
    pub id: usize,
    pub result: GetMerkleResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum TxfromPosResult {
    Simple(Txid),
    WithMerkle {
        #[serde(rename = "tx_hash")]
        txid: Txid,
        merkle: Vec<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TxFromPositionResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub tx: TxfromPosResult,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Peer(
    (
        String,      /* ip address */
        String,      /* domain */
        Vec<String>, /* features */
    ),
);

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ListPeersResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub peers: Vec<Peer>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ResultVersion((String, VersionKind));

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct VersionResponse {
    pub id: usize,
    #[serde(rename = "result")]
    pub version: ResultVersion,
}

#[cfg(test)]
mod tests {
    use miniscript::bitcoin::{OutPoint, Script};

    use super::*;

    #[test]
    fn parse_block_header_subscribe_response_a() {
        let response = r#"{"id":3,"jsonrpc":"2.0","result":{"height":119367,"hex":"00000020835fdbdeeadd23463fad98b4e21aaa8519afde89eecd0eb224001317421cbb5f5e636df02303e51280b586bc596ee9326bc849bbb5993e121a8cab7e6b60e8ab593fe166ffff7f2000000000"}}"#;

        let parsed: HeaderNotification = serde_json::from_str(response).unwrap();
        let expected = HeaderNotification::Single(SingleHeaderNotif { id: 3, header: Header { height: 119367, raw_header: "00000020835fdbdeeadd23463fad98b4e21aaa8519afde89eecd0eb224001317421cbb5f5e636df02303e51280b586bc596ee9326bc849bbb5993e121a8cab7e6b60e8ab593fe166ffff7f2000000000".into() }});
        assert_eq!(parsed, expected)
    }

    #[test]
    fn parse_header_response() {
        let response = r#"{"id":0,"jsonrpc":"2.0","result":"000000206e59d4b0d8d5b9daa4d3ad3093975b0f2a18a6909533350cbfb4b7a04adc6f5f380884ecf7425e488e7f2b249de516e839a5b2d48bcc9b65d45387ce5081c1e8563fe166ffff7f2001000000"}"#;

        let parsed: HeaderResponse = serde_json::from_str(response).unwrap();
        assert_eq!(
            parsed,
            HeaderResponse {
                id: 0,
                raw_header: "000000206e59d4b0d8d5b9daa4d3ad3093975b0f2a18a6909533350cbfb4b7a04adc6f5f380884ecf7425e488e7f2b249de516e839a5b2d48bcc9b65d45387ce5081c1e8563fe166ffff7f2001000000".into()
            }
        )
    }

    #[test]
    fn parse_headers_response() {
        let response = r#"{"id":0,"jsonrpc":"2.0","result":{"count":5,"hex":"000000206e59d4b0d8d5b9daa4d3ad3093975b0f2a18a6909533350cbfb4b7a04adc6f5f380884ecf7425e488e7f2b249de516e839a5b2d48bcc9b65d45387ce5081c1e8563fe166ffff7f200100000000000020e4a9efb184a77e3b3d75c374823a808f437c5d04fc322f6585c1682ea859a379874002727ca2397cbf8b45bffbd0463c1a8e4f52c23af48b3d8e30c0c4556bd1563fe166ffff7f200100000000000020d02dd6842a2be3611748c75b423d0199f86599a7f565de283ee09ffe3527cf49d2e107eae3f796827fb71fc950ee32f5c45c58704cd0f6de8c5125dfe18d0005573fe166ffff7f20000000000000002007e28823c56f2b29644eaa8060f1e62e622733fbb796a429119963f6318e4d012833a1ec146ca836cbd22f3be596ee73f00134c1edafaeb1178623cf480e554c573fe166ffff7f200600000000000020a7cc866c5522c258d4d08cf78aaf6dec40df9cba90c51b4fb63577dab6000b4805c639b49ecb0ddb0d6e922047310faefc6d69316e137084386a24238d1152ba573fe166ffff7f2000000000","max":2016}}"#;

        let parsed: HeadersResponse = serde_json::from_str(response).unwrap();
        assert_eq!(
            parsed,
            HeadersResponse {
                id: 0,
                headers: Headers {
                    count: 5,
                    raw_headers: "000000206e59d4b0d8d5b9daa4d3ad3093975b0f2a18a6909533350cbfb4b7a04adc6f5f380884ecf7425e488e7f2b249de516e839a5b2d48bcc9b65d45387ce5081c1e8563fe166ffff7f200100000000000020e4a9efb184a77e3b3d75c374823a808f437c5d04fc322f6585c1682ea859a379874002727ca2397cbf8b45bffbd0463c1a8e4f52c23af48b3d8e30c0c4556bd1563fe166ffff7f200100000000000020d02dd6842a2be3611748c75b423d0199f86599a7f565de283ee09ffe3527cf49d2e107eae3f796827fb71fc950ee32f5c45c58704cd0f6de8c5125dfe18d0005573fe166ffff7f20000000000000002007e28823c56f2b29644eaa8060f1e62e622733fbb796a429119963f6318e4d012833a1ec146ca836cbd22f3be596ee73f00134c1edafaeb1178623cf480e554c573fe166ffff7f200600000000000020a7cc866c5522c258d4d08cf78aaf6dec40df9cba90c51b4fb63577dab6000b4805c639b49ecb0ddb0d6e922047310faefc6d69316e137084386a24238d1152ba573fe166ffff7f2000000000".into(),
                    max: 2016
                }
            }
        )
    }

    #[test]
    fn version() {
        let response = r#"{"id":0,"jsonrpc":"2.0","result":["electrs/0.10.5","1.4"]}"#;
        let response: VersionResponse = serde_json::from_str(response).unwrap();
        if let VersionResponse {
            id,
            version: ResultVersion((server_name, VersionKind::Single(version))),
        } = response
        {
            assert_eq!(id, 0);
            assert_eq!(server_name, "electrs/0.10.5");
            assert_eq!(version, "1.4");
        } else {
            panic!("wrong response")
        }

        let response = r#"{"id":0,"jsonrpc":"2.0","result":["electrs/0.10.5",["1.1","1.4"]]}"#;
        let response: VersionResponse = serde_json::from_str(response).unwrap();
        if let VersionResponse {
            id,
            version: ResultVersion((server_name, VersionKind::MinMax(min, max))),
        } = response
        {
            assert_eq!(id, 0);
            assert_eq!(server_name, "electrs/0.10.5");
            assert_eq!(min, "1.1");
            assert_eq!(max, "1.4");
        } else {
            panic!("wrong response")
        }
    }

    #[test]
    fn hash_subscribe_response() {
        let response = r#"{"id":14,"jsonrpc":"2.0","result":"1c8606707de065bef7474d719b76fb41cdff0090fffb78ca6b640c66ba9a9542"}"#;

        let response: SHSubscribeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 14);
        assert_eq!(
            response.result,
            Some("1c8606707de065bef7474d719b76fb41cdff0090fffb78ca6b640c66ba9a9542".to_string())
        )
    }

    #[test]
    fn batch_sh_subscribe_response() {
        let script = Script::from_bytes(&[0x00]);
        let req = Request::subscribe_sh(script);

        // populate index w/ requests
        let mut index = HashMap::new();
        for i in 14..35usize {
            let mut r = req.clone();
            r.id = i;
            index.insert(i, r);
        }

        let response = r#"[{"id":14,"jsonrpc":"2.0","result":"1c8606707de065bef7474d719b76fb41cdff0090fffb78ca6b640c66ba9a9542"},{"id":15,"jsonrpc":"2.0","result":null},{"id":16,"jsonrpc":"2.0","result":null},{"id":17,"jsonrpc":"2.0","result":null},{"id":18,"jsonrpc":"2.0","result":null},{"id":19,"jsonrpc":"2.0","result":null},{"id":20,"jsonrpc":"2.0","result":null},{"id":21,"jsonrpc":"2.0","result":null},{"id":22,"jsonrpc":"2.0","result":null},{"id":23,"jsonrpc":"2.0","result":null},{"id":24,"jsonrpc":"2.0","result":null},{"id":25,"jsonrpc":"2.0","result":null},{"id":26,"jsonrpc":"2.0","result":null},{"id":27,"jsonrpc":"2.0","result":null},{"id":28,"jsonrpc":"2.0","result":null},{"id":29,"jsonrpc":"2.0","result":null},{"id":30,"jsonrpc":"2.0","result":null},{"id":31,"jsonrpc":"2.0","result":null},{"id":32,"jsonrpc":"2.0","result":null},{"id":33,"jsonrpc":"2.0","result":null},{"id":34,"jsonrpc":"2.0","result":null}]"#;

        let batch = ResponseBatch::from_str(response, &index).unwrap().unwrap();
        assert_eq!(batch.batch.len(), 21);
        let resp = &batch.batch[5];
        if let Response::SHSubscribe(SHSubscribeResponse { id, result }) = resp {
            assert_eq!(*id, 19);
            assert_eq!(*result, None);
        } else {
            panic!("wrong response");
        }
    }

    #[test]
    fn error_response() {
        let response = r#"{"error":{"code":1,"message":"unsupported request Single(\"0.4\") by smart"},"id":0,"jsonrpc":"2.0"}"#;

        let response: ErrorResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.error.code, 1);
        assert_eq!(response.id, 0);
        assert_eq!(
            response.error.message,
            r#"unsupported request Single("0.4") by smart"#
        );
    }

    #[test]
    fn sh_unsubscribe_response() {
        let response = r#"{"id":0,"jsonrpc":"2.0","result":false}"#;
        let response: SHUnsubscribeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert!(!response.result);
    }

    #[test]
    fn sh_subscribe_response() {
        let response = r#"{"id":1,"jsonrpc":"2.0","result":null}"#;
        let response: SHSubscribeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 1);
        assert_eq!(response.result, None);

        let response = r#"{"id":1,"jsonrpc":"2.0","result":"some_garbage_string"}"#;
        let response: SHSubscribeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 1);
        assert_eq!(response.result, Some("some_garbage_string".into()));
    }

    #[test]
    fn sh_notification() {
        let response = r#" {"jsonrpc":"2.0","method":"blockchain.scripthash.subscribe","params":["95ebd95e7c0763b785d12b1d20d9f548fa5bb809f120afb0dd11276fa1ce8352","9bf1d98ff899eafd048290199144aed63e3d7ccbc8925e8351a4c1e8af2137f4"]}"#;

        let _: SHNotification = serde_json::from_str(response).unwrap();
        let response = SHNotification::from_str(response).unwrap();

        assert_eq!(response.method, Method::ScriptHashSubscribe);
        assert!(response.status.1.is_some());
        assert_eq!(
            response.status.1,
            Some("9bf1d98ff899eafd048290199144aed63e3d7ccbc8925e8351a4c1e8af2137f4".into())
        );
    }

    #[test]
    fn sh_list_unspent() {
        let response = r#"{"jsonrpc": "2.0", "result": [{"tx_hash": "b14edd61d6902890932be0d4386c79ca64a8dea345e9b9c95b2e8a825316cfc0", "tx_pos": 1, "height": 861250, "value": 566888}], "id": 0}"#;
        let response: SHListUnspentResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.unspent.len(), 1);
        assert_eq!(
            response.unspent[0].txid,
            Txid::from_str("b14edd61d6902890932be0d4386c79ca64a8dea345e9b9c95b2e8a825316cfc0")
                .unwrap()
        );
        assert_eq!(response.unspent[0].vout, 1);
        assert_eq!(response.unspent[0].height, 861250);
        assert_eq!(response.unspent[0].value, 566888);
    }

    #[test]
    fn sh_get_balance() {
        let response =
            r#"{"jsonrpc": "2.0", "result": {"confirmed": 566888, "unconfirmed": 0}, "id": 0}"#;
        let response: SHGetBalanceResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.balance.confirmed, 566888);
        assert_eq!(response.balance.unconfirmed, 0);
    }

    #[test]
    fn sh_get_history() {
        let response = r#"{"jsonrpc": "2.0", "result": [{"tx_hash": "b14edd61d6902890932be0d4386c79ca64a8dea345e9b9c95b2e8a825316cfc0", "height": 861250}], "id": 0}"#;
        let response: SHGetHistoryResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.history.len(), 1);
        assert_eq!(
            response.history[0].txid,
            Txid::from_str("b14edd61d6902890932be0d4386c79ca64a8dea345e9b9c95b2e8a825316cfc0")
                .unwrap()
        );
        assert_eq!(response.history[0].height, 861250);
    }

    #[test]
    fn features() {
        let response = r#"{"jsonrpc": "2.0", "result": {"hosts": {}, "pruning": null, "server_version": "ElectrumX 1.15.0", "protocol_min": "1.4", "protocol_max": "1.4.2", "genesis_hash": "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f", "hash_function": "sha256", "services": []}, "id": 0}"#;

        let response: FeaturesResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert!(response.features.pruning.is_none());
        assert_eq!(response.features.server_version, "ElectrumX 1.15.0");
        assert_eq!(response.features.protocol_min, "1.4");
        assert_eq!(response.features.protocol_max, "1.4.2");
        assert_eq!(
            response.features.genesis,
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        );
        assert_eq!(response.features.hash_function, "sha256");
        assert!(response.features.services.is_some());
        assert!(response.features.services.unwrap().is_empty());

        let response = r#"{
              "id": 0,
              "jsonrpc": "2.0",
              "result": {
                "genesis_hash": "abc",
                "hash_function": "sha256",
                "hosts": {
                  "tcp_port": 46771
                },
                "protocol_max": "1.4",
                "protocol_min": "1.4",
                "pruning": null,
                "server_version": "toto"
              }
            }"#;

        let response: FeaturesResponse = serde_json::from_str(response).unwrap();

        let expected = FeaturesResponse {
            id: 0,
            features: FeaturesResult {
                genesis: "abc".into(),
                hosts: Hosts::Single(Host {
                    tcp_port: Some(Port::U16(46771)),
                    ssl_port: None,
                }),
                protocol_max: "1.4".into(),
                protocol_min: "1.4".into(),
                pruning: None,
                server_version: "toto".into(),
                hash_function: "sha256".into(),
                services: None,
            },
        };
        assert_eq!(response, expected);
    }

    #[test]
    fn donation() {
        let response = r#"{"jsonrpc": "2.0", "result": "make_me_rich", "id": 0}"#;

        let response: DonationResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.address, Some("make_me_rich".into()));
    }

    #[test]
    fn estimate_fee() {
        let response = r#"{"jsonrpc": "2.0", "result": 3.006e-05, "id": 0}"#;

        let response: EstimateFeeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.fee, OptionalFee::Fee(0.00003006));
    }

    #[test]
    fn get_fee_histogram() {
        let response = r#"{"jsonrpc": "2.0", "result": [[5, 103673], [3, 238053], [2, 12058673], [1, 34188435]], "id": 0}"#;

        let response: FeeHistogramResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.histogram.len(), 4);
        assert_eq!(response.histogram[1].0, 3);
        assert_eq!(response.histogram[2].1, 12058673);
        let expected = FeeHistogramResponse {
            id: 0,
            histogram: vec![(5, 103673), (3, 238053), (2, 12058673), (1, 34188435)],
        };
        assert_eq!(response, expected);
    }

    #[test]
    fn relay_fee() {
        let response = r#"{"jsonrpc": "2.0", "result": 1e-05, "id": 0}"#;

        let response: RelayFeeResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        assert_eq!(response.fee, OptionalFee::Fee(0.00001));
    }

    #[test]
    fn tx_get_merkle() {
        let response = r#"{"jsonrpc": "2.0", "result": {"block_height": 200000, "merkle": ["ffa0267c8f2af736858894d6f3e5081a05e2ec16dc98f78a80f376ce35077491", "d0039b6be844e631698f57fa02bbfbfb5e8b680f3ebb17646631e6ec9f91f6e6", "bbe3063ce3d04c2e3f18e494a287867f81ad1182b62a1ecb3e1ea2686edcea20", "1d15a2423f52d4aa281a2ac389c0a5a601ed08bdf814494ddf7697196860b801", "b63e58ec9f5ee2e268f1540af8bb0e5b8fd0ce7cd6877a174e6178c676d6b574", "7407724b98c77cdbf070f3fe297839de2bef50fead98b452883f0f3a4643cde2", "d029f17725e71e3c025bd7d0505006dc859af5450d0b6dd092ee88c0d98f9a25", "e4df974d81ab4fdf35f635024a01f20aa88af9f520215708b339dbc5bceddf63", "20f4202f18666483306f175e1c9c521741845afcf2710f0b0d42602ac72c5fd6"], "pos": 2}, "id": 0}"#;

        let response: TxGetMerkleResponse = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, 0);
        let expected = TxGetMerkleResponse {
            id: 0,
            result: GetMerkleResult {
                merkle: vec![
                    "ffa0267c8f2af736858894d6f3e5081a05e2ec16dc98f78a80f376ce35077491".into(),
                    "d0039b6be844e631698f57fa02bbfbfb5e8b680f3ebb17646631e6ec9f91f6e6".into(),
                    "bbe3063ce3d04c2e3f18e494a287867f81ad1182b62a1ecb3e1ea2686edcea20".into(),
                    "1d15a2423f52d4aa281a2ac389c0a5a601ed08bdf814494ddf7697196860b801".into(),
                    "b63e58ec9f5ee2e268f1540af8bb0e5b8fd0ce7cd6877a174e6178c676d6b574".into(),
                    "7407724b98c77cdbf070f3fe297839de2bef50fead98b452883f0f3a4643cde2".into(),
                    "d029f17725e71e3c025bd7d0505006dc859af5450d0b6dd092ee88c0d98f9a25".into(),
                    "e4df974d81ab4fdf35f635024a01f20aa88af9f520215708b339dbc5bceddf63".into(),
                    "20f4202f18666483306f175e1c9c521741845afcf2710f0b0d42602ac72c5fd6".into(),
                ],
                block_height: 200_000,
                tx_pos: 2,
            },
        };
        assert_eq!(expected, response);
    }

    #[test]
    fn tx_from_pos() {
        let response = r#"{"jsonrpc": "2.0", "result": "ffa0267c8f2af736858894d6f3e5081a05e2ec16dc98f78a80f376ce35077491", "id": 0}"#;

        let outpoint = OutPoint::from_str(
            "ffa0267c8f2af736858894d6f3e5081a05e2ec16dc98f78a80f376ce35077491:0",
        )
        .unwrap();

        let response: TxFromPositionResponse = serde_json::from_str(response).unwrap();
        let expected = TxFromPositionResponse {
            id: 0,
            tx: TxfromPosResult::Simple(outpoint.txid),
        };
        assert_eq!(response, expected);

        let response = r#"{"jsonrpc": "2.0", "result": {"tx_hash": "9cc064bbce74a2c56ce12b0b59fc7267a2618a35e1d8c66f642efd6d033a9681", "merkle": ["e48b08df0afa01a7339335fb6b6964100d11985765cbc6afcde990fd65856a9b", "12a6c68b6c033d6704bda3437370b3e7d65bec81b2e3f4eafb17632197f0b6c7", "c0dbecba7c7990f3bfbe727dd9a7371225852600dc0a0f07e68b3ec7c4fd629e", "8e351c5bac49e6dbf08bc67cc1f57fb4dbea0383336d0ee2c38fefc8736b18eb", "aa7171ca4f639d14050101ac602f3f526abec753414b3b4648071b252434e38e", "583b92abff3481905c686d3ff594c4a1d6a00bab25deb3397369b9e49adf11ae", "6a4d797a4d3e162a951ccd142fe6ca86e12006145f1670c5d1aa5e7bfcc05fa3", "c6dd553f393d1b7694ae168e8f5efeba8db4c3b000c2d9bf5205dd19f96c08a8"]}, "id": 1}"#;

        let response: TxFromPositionResponse = serde_json::from_str(response).unwrap();
        let outpoint = OutPoint::from_str(
            "9cc064bbce74a2c56ce12b0b59fc7267a2618a35e1d8c66f642efd6d033a9681:0",
        )
        .unwrap();

        let expected = TxFromPositionResponse {
            id: 1,
            tx: TxfromPosResult::WithMerkle {
                txid: outpoint.txid,
                merkle: vec![
                    "e48b08df0afa01a7339335fb6b6964100d11985765cbc6afcde990fd65856a9b".into(),
                    "12a6c68b6c033d6704bda3437370b3e7d65bec81b2e3f4eafb17632197f0b6c7".into(),
                    "c0dbecba7c7990f3bfbe727dd9a7371225852600dc0a0f07e68b3ec7c4fd629e".into(),
                    "8e351c5bac49e6dbf08bc67cc1f57fb4dbea0383336d0ee2c38fefc8736b18eb".into(),
                    "aa7171ca4f639d14050101ac602f3f526abec753414b3b4648071b252434e38e".into(),
                    "583b92abff3481905c686d3ff594c4a1d6a00bab25deb3397369b9e49adf11ae".into(),
                    "6a4d797a4d3e162a951ccd142fe6ca86e12006145f1670c5d1aa5e7bfcc05fa3".into(),
                    "c6dd553f393d1b7694ae168e8f5efeba8db4c3b000c2d9bf5205dd19f96c08a8".into(),
                ],
            },
        };
        assert_eq!(response, expected);
    }
}
